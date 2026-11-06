//! Runs inside the guest. The guest has no network device, so this program
//! has three channels and nothing else, all over vsock:
//!
//! - pixels out: width u32 LE, height u32 LE, BGRA
//! - membrane: typed commands in, observations out
//! - gate: Chromium's SOCKS5 proxy traffic, forwarded from guest loopback
//!
//! The engine gives up root as its first act and runs with Chromium's sandbox
//! on, so a hacked renderer is walled in twice: by the sandbox, then the VM.

use cef::args::Args;
use cef::*;
use frame_membrane::{
    Command, DecodeError, Key, Modifiers, MouseButton, Observation, decode_command,
};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const WIDTH: i32 = 1920;
const HEIGHT: i32 = 1080;
const HOST_CID: u32 = 2;
const PIXELS_PORT: u32 = 5000;
const GATE_PORT: u32 = 5001;
const MEMBRANE_PORT: u32 = 5002;
/// The unprivileged user the browser runs as inside the guest.
const GUEST_USER: libc::uid_t = 1000;
const PROXY_ADDRESS: &str = "127.0.0.1:1080";

const EVENTFLAG_SHIFT_DOWN: u32 = 1 << 1;
const EVENTFLAG_CONTROL_DOWN: u32 = 1 << 2;
const EVENTFLAG_ALT_DOWN: u32 = 1 << 3;
const EVENTFLAG_LEFT_MOUSE_BUTTON: u32 = 1 << 4;
const EVENTFLAG_MIDDLE_MOUSE_BUTTON: u32 = 1 << 5;
const EVENTFLAG_RIGHT_MOUSE_BUTTON: u32 = 1 << 6;

type BrowserSlot = Arc<Mutex<Option<Browser>>>;
type ObservationSink = Arc<Mutex<Option<UnixStream>>>;

#[derive(Default)]
struct Latest {
    frame: Mutex<Option<(u32, u32, Vec<u8>)>>,
    ready: Condvar,
}

/// A `UnixStream` here is only a safe handle on a connected stream socket.
fn connect_host(port: u32) -> Result<UnixStream, String> {
    // SAFETY: plain socket calls on a zeroed, correctly sized address.
    unsafe {
        let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err("vsock socket failed".into());
        }
        let mut address: libc::sockaddr_vm = std::mem::zeroed();
        address.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        address.svm_cid = HOST_CID;
        address.svm_port = port;
        let length = std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t;
        if libc::connect(fd, (&raw const address).cast(), length) < 0 {
            return Err(format!(
                "vsock connect failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(UnixStream::from_raw_fd(fd))
    }
}

fn send_frames(latest: Arc<Latest>) {
    let mut host = match connect_host(PIXELS_PORT) {
        Ok(host) => host,
        Err(error) => {
            eprintln!("engine: {error}");
            return;
        }
    };
    loop {
        let Ok(mut slot) = latest.frame.lock() else {
            return;
        };
        while slot.is_none() {
            let Ok(next) = latest.ready.wait(slot) else {
                return;
            };
            slot = next;
        }
        let Some((width, height, pixels)) = slot.take() else {
            continue;
        };
        drop(slot);
        let sent = host
            .write_all(&width.to_le_bytes())
            .and_then(|()| host.write_all(&height.to_le_bytes()))
            .and_then(|()| host.write_all(&pixels));
        if let Err(error) = sent {
            eprintln!("engine: frame send failed: {error}");
            return;
        }
    }
}

fn pump(mut from: impl Read, mut to: impl Write, close: impl FnOnce()) {
    let _ = std::io::copy(&mut from, &mut to);
    close();
}

/// Chromium reaches the gate through guest loopback. If the gate is gone, the
/// connection fails: there is no other route to fall back to.
fn forward_to_gate(browser_side: TcpStream) {
    let Ok(gate) = connect_host(GATE_PORT) else {
        return;
    };
    let (Ok(browser_read), Ok(gate_read)) = (browser_side.try_clone(), gate.try_clone()) else {
        return;
    };
    let (browser_close, gate_close) = (browser_side, gate);
    let (close_gate, close_browser) = (gate_close.try_clone(), browser_close.try_clone());
    std::thread::spawn(move || {
        pump(browser_read, gate_close, move || {
            if let Ok(gate) = close_gate {
                let _ = gate.shutdown(Shutdown::Write);
            }
        });
    });
    pump(gate_read, browser_close, move || {
        if let Ok(browser) = close_browser {
            let _ = browser.shutdown(Shutdown::Write);
        }
    });
}

fn serve_proxy(listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || forward_to_gate(stream));
    }
}

fn flags(modifiers: Modifiers) -> u32 {
    let mut bits = 0;
    if modifiers.shift {
        bits |= EVENTFLAG_SHIFT_DOWN;
    }
    if modifiers.control {
        bits |= EVENTFLAG_CONTROL_DOWN;
    }
    if modifiers.alt {
        bits |= EVENTFLAG_ALT_DOWN;
    }
    bits
}

fn virtual_key(key: Key) -> i32 {
    match key {
        Key::Backspace => 0x08,
        Key::Tab => 0x09,
        Key::Enter => 0x0d,
        Key::Escape => 0x1b,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        Key::End => 0x23,
        Key::Home => 0x24,
        Key::Left => 0x25,
        Key::Up => 0x26,
        Key::Right => 0x27,
        Key::Down => 0x28,
        Key::Delete => 0x2e,
    }
}

fn key_event(
    host: &BrowserHost,
    kind: sys::cef_key_event_type_t,
    code: i32,
    unit: u16,
    modifiers: u32,
) {
    let event = KeyEvent {
        size: std::mem::size_of::<sys::_cef_key_event_t>(),
        type_: kind.into(),
        modifiers,
        windows_key_code: code,
        character: unit,
        unmodified_character: unit,
        ..Default::default()
    };
    host.send_key_event(Some(&event));
}

fn press_named(host: &BrowserHost, key: Key, modifiers: u32) {
    let code = virtual_key(key);
    key_event(
        host,
        sys::cef_key_event_type_t::KEYEVENT_RAWKEYDOWN,
        code,
        0,
        modifiers,
    );
    if key == Key::Enter {
        key_event(
            host,
            sys::cef_key_event_type_t::KEYEVENT_CHAR,
            code,
            0x0d,
            modifiers,
        );
    }
    key_event(
        host,
        sys::cef_key_event_type_t::KEYEVENT_KEYUP,
        code,
        0,
        modifiers,
    );
}

fn type_character(host: &BrowserHost, character: char) {
    let code = if character.is_ascii_alphanumeric() {
        i32::from(character.to_ascii_uppercase() as u8)
    } else {
        0
    };
    key_event(
        host,
        sys::cef_key_event_type_t::KEYEVENT_RAWKEYDOWN,
        code,
        0,
        0,
    );
    let mut units = [0; 2];
    for unit in character.encode_utf16(&mut units) {
        key_event(
            host,
            sys::cef_key_event_type_t::KEYEVENT_CHAR,
            code,
            *unit,
            0,
        );
    }
    key_event(host, sys::cef_key_event_type_t::KEYEVENT_KEYUP, code, 0, 0);
}

fn mouse(x: u16, y: u16, buttons: u32) -> MouseEvent {
    MouseEvent {
        x: x.into(),
        y: y.into(),
        modifiers: buttons,
    }
}

fn apply(browser: &Browser, command: Command, buttons: &mut u32) {
    let Some(host) = browser.host() else {
        return;
    };
    match command {
        Command::Navigate { url } => {
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&url.as_str().into()));
            }
        }
        Command::Reload => browser.reload(),
        Command::Stop => browser.stop_load(),
        Command::Back => browser.go_back(),
        Command::Forward => browser.go_forward(),
        Command::MouseMove { x, y } => host.send_mouse_move_event(Some(&mouse(x, y, *buttons)), 0),
        Command::MouseButton {
            x,
            y,
            button,
            pressed,
            clicks,
        } => {
            let (kind, flag) = match button {
                MouseButton::Left => (
                    sys::cef_mouse_button_type_t::MBT_LEFT,
                    EVENTFLAG_LEFT_MOUSE_BUTTON,
                ),
                MouseButton::Middle => (
                    sys::cef_mouse_button_type_t::MBT_MIDDLE,
                    EVENTFLAG_MIDDLE_MOUSE_BUTTON,
                ),
                MouseButton::Right => (
                    sys::cef_mouse_button_type_t::MBT_RIGHT,
                    EVENTFLAG_RIGHT_MOUSE_BUTTON,
                ),
            };
            if pressed {
                *buttons |= flag;
                host.set_focus(1);
            } else {
                *buttons &= !flag;
            }
            host.send_mouse_click_event(
                Some(&mouse(x, y, *buttons)),
                kind.into(),
                i32::from(!pressed),
                clicks.into(),
            );
        }
        Command::Wheel {
            x,
            y,
            delta_x,
            delta_y,
        } => host.send_mouse_wheel_event(
            Some(&mouse(x, y, *buttons)),
            delta_x.into(),
            delta_y.into(),
        ),
        Command::Key { key, modifiers } => press_named(&host, key, flags(modifiers)),
        Command::Text { character } => type_character(&host, character),
    }
}

/// Reads commands until the channel ends. Only the closed [`Command`] set can
/// reach the engine, and a bad frame closes the channel.
fn read_commands(mut host: UnixStream, browser: BrowserSlot) {
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    let mut buttons = 0;
    loop {
        match decode_command(&buffer) {
            Ok((command, used)) => {
                buffer.drain(..used);
                let target = browser.lock().ok().and_then(|slot| slot.clone());
                if let Some(target) = target {
                    apply(&target, command, &mut buttons);
                }
            }
            Err(DecodeError::Incomplete) => match host.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(count) => buffer.extend_from_slice(&chunk[..count]),
            },
            Err(error) => {
                eprintln!("engine: closing membrane: {error}");
                return;
            }
        }
    }
}

/// Refuses to continue unless root is gone for good. Chromium's sandbox will
/// not run as root, and nothing here needs it after the guest has booted.
fn drop_root() -> Result<(), String> {
    // SAFETY: plain system calls with no pointers but a null, empty group list.
    unsafe {
        if libc::getuid() != 0 {
            return Ok(());
        }
        if libc::setgroups(0, std::ptr::null()) != 0
            || libc::setgid(GUEST_USER) != 0
            || libc::setuid(GUEST_USER) != 0
        {
            return Err(format!("{}", std::io::Error::last_os_error()));
        }
        if libc::setuid(0) == 0 {
            return Err("root could be regained".into());
        }
        // Changing user marks a process non-dumpable, which makes its own
        // /proc files root-owned. Chromium's sandbox must write to them.
        if libc::prctl(libc::PR_SET_DUMPABLE, 1) != 0 {
            return Err(format!("{}", std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

fn status_field<'a>(status: &'a str, key: &str) -> &'a str {
    status
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .unwrap_or("?")
        .trim()
}

/// Prints each child process's real state, as the kernel reports it, so a
/// test can prove the sandbox is on instead of trusting a setting.
fn report_sandbox() {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(command) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let flag = |name: &[u8]| {
            command
                .split(|byte| *byte == 0)
                .find_map(|arg| arg.strip_prefix(name))
                .map(|value| String::from_utf8_lossy(value).into_owned())
        };
        let kind = flag(b"--type=").map(|kind| {
            format!(
                "{kind}/{}",
                flag(b"--utility-sub-type=").unwrap_or_default()
            )
        });
        let (Some(kind), Ok(status)) = (kind, std::fs::read_to_string(entry.path().join("status")))
        else {
            continue;
        };
        println!(
            "SANDBOX type={kind} uid={} seccomp={} nspid={}",
            status_field(&status, "Uid:")
                .split_whitespace()
                .next()
                .unwrap_or("?"),
            status_field(&status, "Seccomp:"),
            status_field(&status, "NSpid:").split_whitespace().count(),
        );
    }
}

fn field_kib(text: &str, key: &str) -> u64 {
    text.lines()
        .find(|line| line.starts_with(key))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// Prints memory use, so results come from the guest and are not guessed.
fn report_memory(started: Instant) {
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let mut processes = 0;
        let mut rss_kib = 0;
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let status = std::fs::read_to_string(entry.path().join("status"));
                if let Ok(status) = status {
                    let rss = field_kib(&status, "VmRSS:");
                    if rss > 0 {
                        processes += 1;
                        rss_kib += rss;
                    }
                }
            }
        }
        println!(
            "MEM t={}s available={}MiB cached={}MiB shmem={}MiB processes={processes} rss_sum={}MiB",
            started.elapsed().as_secs(),
            field_kib(&meminfo, "MemAvailable:") / 1024,
            field_kib(&meminfo, "Cached:") / 1024,
            field_kib(&meminfo, "Shmem:") / 1024,
            rss_kib / 1024,
        );
        report_sandbox();
    }
}

wrap_render_handler! {
    struct Paint {
        latest: Arc<Latest>,
        started: Instant,
        painted: Arc<Mutex<u64>>,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                rect.width = WIDTH;
                rect.height = HEIGHT;
            }
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            _type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            if buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }
            let length = width as usize * height as usize * 4;
            // SAFETY: CEF guarantees a BGRA buffer of this size for this call.
            let pixels = unsafe { std::slice::from_raw_parts(buffer, length) }.to_vec();
            if let Ok(mut count) = self.painted.lock() {
                *count += 1;
                if *count == 1 {
                    println!("FIRST_PAINT t={}ms", self.started.elapsed().as_millis());
                }
            }
            if let Ok(mut slot) = self.latest.frame.lock() {
                *slot = Some((width as u32, height as u32, pixels));
                self.latest.ready.notify_one();
            }
        }
    }
}

wrap_load_handler! {
    struct Load {
        started: Instant,
    }

    impl LoadHandler {
        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            http_status_code: ::std::os::raw::c_int,
        ) {
            if frame.is_some_and(|frame| frame.is_main() == 1) {
                println!(
                    "LOAD_END t={}ms status={http_status_code}",
                    self.started.elapsed().as_millis()
                );
            }
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let text = error_text.map(ToString::to_string).unwrap_or_default();
            let url = failed_url.map(ToString::to_string).unwrap_or_default();
            println!("LOAD_ERROR {error_code:?} {text} {url}");
        }
    }
}

wrap_display_handler! {
    struct Display {
        sink: ObservationSink,
    }

    impl DisplayHandler {
        fn on_address_change(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            if !frame.is_some_and(|frame| frame.is_main() == 1) {
                return;
            }
            let url = url.map(ToString::to_string).unwrap_or_default();
            let Ok(message) = (Observation::Address { url }).encode() else {
                return;
            };
            if let Ok(mut slot) = self.sink.lock() {
                if let Some(host) = slot.as_mut() {
                    let _ = host.write_all(&message);
                }
            }
        }
    }
}

wrap_life_span_handler! {
    struct Popups;

    impl LifeSpanHandler {
        /// One window, one page: a link that asks for a new window opens here,
        /// and a popup without a user gesture does not open at all.
        fn on_before_popup(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            if user_gesture == 1 {
                if let Some(frame) = browser.and_then(|browser| browser.main_frame()) {
                    frame.load_url(target_url);
                }
            }
            1
        }
    }
}

wrap_client! {
    struct EngineClient {
        paint: RenderHandler,
        load: LoadHandler,
        display: DisplayHandler,
        popups: LifeSpanHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<cef::RenderHandler> {
            Some(self.paint.clone())
        }

        fn load_handler(&self) -> Option<cef::LoadHandler> {
            Some(self.load.clone())
        }

        fn display_handler(&self) -> Option<cef::DisplayHandler> {
            Some(self.display.clone())
        }

        fn life_span_handler(&self) -> Option<cef::LifeSpanHandler> {
            Some(self.popups.clone())
        }
    }
}

wrap_browser_process_handler! {
    struct Startup {
        url: String,
        latest: Arc<Latest>,
        started: Instant,
        browser: BrowserSlot,
        sink: ObservationSink,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            println!("CEF_READY t={}ms", self.started.elapsed().as_millis());
            let window_info = WindowInfo {
                windowless_rendering_enabled: true as _,
                ..Default::default()
            };
            let settings = BrowserSettings {
                windowless_frame_rate: 15,
                ..Default::default()
            };
            let paint = Paint::new(self.latest.clone(), self.started, Arc::new(Mutex::new(0)));
            let load = Load::new(self.started);
            let display = Display::new(self.sink.clone());
            let mut client = EngineClient::new(paint, load, display, Popups::new());
            let created = browser_host_create_browser_sync(
                Some(&window_info),
                Some(&mut client),
                Some(&self.url.as_str().into()),
                Some(&settings),
                None,
                None,
            );
            println!("BROWSER_CREATED {}", created.is_some());
            if let Some(host) = created.as_ref().and_then(|browser| browser.host()) {
                host.set_focus(1);
            }
            if let Ok(mut slot) = self.browser.lock() {
                *slot = created;
            }
        }
    }
}

wrap_app! {
    struct EngineApp {
        url: String,
        latest: Arc<Latest>,
        started: Instant,
        browser: BrowserSlot,
        sink: ObservationSink,
    }

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(Startup::new(
                self.url.clone(),
                self.latest.clone(),
                self.started,
                self.browser.clone(),
                self.sink.clone(),
            ))
        }
    }
}

fn main() {
    if let Err(error) = drop_root() {
        eprintln!("engine: refusing to run: {error}");
        std::process::exit(1);
    }
    let started = Instant::now();
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = Args::new();
    let code = execute_process(
        Some(args.as_main_args()),
        None::<&mut App>,
        std::ptr::null_mut(),
    );
    if code >= 0 {
        std::process::exit(code);
    }

    let url = std::env::args()
        .find_map(|arg| arg.strip_prefix("--frame-url=").map(str::to_owned))
        .unwrap_or_else(|| "about:blank".to_owned());
    let latest = Arc::new(Latest::default());
    let sender = latest.clone();
    std::thread::spawn(move || send_frames(sender));
    std::thread::spawn(move || report_memory(started));

    let proxy = match TcpListener::bind(PROXY_ADDRESS) {
        Ok(proxy) => proxy,
        Err(error) => {
            eprintln!("engine: proxy listener failed: {error}");
            std::process::exit(1);
        }
    };
    std::thread::spawn(move || serve_proxy(proxy));

    let browser: BrowserSlot = Arc::default();
    let sink: ObservationSink = Arc::default();
    match connect_host(MEMBRANE_PORT) {
        Ok(host) => {
            if let (Ok(reader), Ok(mut slot)) = (host.try_clone(), sink.lock()) {
                *slot = Some(host);
                let target = browser.clone();
                std::thread::spawn(move || read_commands(reader, target));
            }
        }
        Err(error) => eprintln!("engine: no membrane: {error}"),
    }

    let mut app = EngineApp::new(url, latest, started, browser, sink);
    let settings = Settings {
        windowless_rendering_enabled: true as _,
        root_cache_path: "/tmp/cef".into(),
        log_severity: LogSeverity::ERROR,
        ..Default::default()
    };
    let ready = initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if ready != 1 {
        eprintln!("engine: CEF failed to start");
        std::process::exit(1);
    }
    run_message_loop();
    shutdown();
}
