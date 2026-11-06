//! Runs inside the guest: CEF renders off-screen and each painted frame goes
//! to the host over vsock. Frame format: width u32 LE, height u32 LE, BGRA.
//!
//! Spike code. The guest runs as root with Chromium's own sandbox off, which
//! a real build must not do.

use cef::args::Args;
use cef::*;
use std::fs::File;
use std::io::Write;
use std::os::fd::FromRawFd;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const WIDTH: i32 = 1920;
const HEIGHT: i32 = 1080;
const HOST_CID: u32 = 2;
const FRAME_PORT: u32 = 5000;

#[derive(Default)]
struct Latest {
    frame: Mutex<Option<(u32, u32, Vec<u8>)>>,
    ready: Condvar,
}

fn connect_host() -> Result<File, String> {
    // SAFETY: plain socket calls on a zeroed, correctly sized address.
    unsafe {
        let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err("vsock socket failed".into());
        }
        let mut address: libc::sockaddr_vm = std::mem::zeroed();
        address.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        address.svm_cid = HOST_CID;
        address.svm_port = FRAME_PORT;
        let length = std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t;
        if libc::connect(fd, (&raw const address).cast(), length) < 0 {
            return Err(format!("vsock connect failed: {}", std::io::Error::last_os_error()));
        }
        Ok(File::from_raw_fd(fd))
    }
}

fn send_frames(latest: Arc<Latest>) {
    let mut host = match connect_host() {
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

wrap_client! {
    struct EngineClient {
        paint: RenderHandler,
        load: LoadHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<cef::RenderHandler> {
            Some(self.paint.clone())
        }

        fn load_handler(&self) -> Option<cef::LoadHandler> {
            Some(self.load.clone())
        }
    }
}

wrap_browser_process_handler! {
    struct Startup {
        url: String,
        latest: Arc<Latest>,
        started: Instant,
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
            let mut client = EngineClient::new(paint, load);
            let created = browser_host_create_browser(
                Some(&window_info),
                Some(&mut client),
                Some(&self.url.as_str().into()),
                Some(&settings),
                None,
                None,
            );
            println!("BROWSER_CREATED {created}");
        }
    }
}

wrap_app! {
    struct EngineApp {
        url: String,
        latest: Arc<Latest>,
        started: Instant,
    }

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(Startup::new(self.url.clone(), self.latest.clone(), self.started))
        }
    }
}

fn main() {
    let started = Instant::now();
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = Args::new();
    let code = execute_process(Some(args.as_main_args()), None::<&mut App>, std::ptr::null_mut());
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

    let mut app = EngineApp::new(url, latest, started);
    let settings = Settings {
        windowless_rendering_enabled: true as _,
        no_sandbox: true as _,
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
