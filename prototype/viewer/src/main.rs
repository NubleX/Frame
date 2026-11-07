//! The window on the host, and the owner of its compartment. Pixels come in,
//! typed commands go out, and nothing else crosses. Everything from the guest
//! is hostile, so sizes are checked and observations are decoded strictly.
//!
//! Click the bar or press Ctrl+L for the address, Alt+Left back, Alt+Right
//! forward, F5 reload, Ctrl with plus, minus or 0 to zoom.
//!
//! Usage: viewer <work-dir> <url> [engine flags]

mod bar;
mod compartment;
mod view;

use bar::{BAR_HEIGHT, Hit};
use compartment::{Compartment, RunDir};
use frame_membrane::{
    Command, DecodeError, Key as WireKey, Modifiers, MouseButton as WireButton, Observation, Zoom,
    decode_observation,
};
use minifb::{InputCallback, Key, MouseButton, MouseMode, ScaleMode, Window, WindowOptions};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use view::{Layout, PAGE_HEIGHT, PAGE_WIDTH, draw_page};

const MAX_WIDTH: u32 = 3840;
const MAX_HEIGHT: u32 = 2160;
const WHEEL_STEP: f32 = 60.0;
/// The toolkit only reports the mouse when polled, and a tap-to-click can be
/// over within a few milliseconds, so poll often and redraw only on change.
const POLLS_PER_SECOND: usize = 120;
/// Redrawn this often anyway, so a resized or uncovered window repaints.
const REDRAW_AT_LEAST: Duration = Duration::from_millis(100);
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

struct Frame {
    width: usize,
    height: usize,
    pixels: Vec<u32>,
}

type Shared<T> = Arc<Mutex<T>>;

fn read_u32(stream: &mut impl Read) -> std::io::Result<u32> {
    let mut bytes = [0; 4];
    stream.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_frames(mut stream: impl Read, latest: &Mutex<Option<Frame>>) -> std::io::Result<()> {
    let mut bytes = Vec::new();
    loop {
        let width = read_u32(&mut stream)?;
        let height = read_u32(&mut stream)?;
        if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
            eprintln!("viewer: refused frame size {width}x{height}");
            return Ok(());
        }
        bytes.resize(width as usize * height as usize * 4, 0);
        stream.read_exact(&mut bytes)?;
        let pixels = bytes
            .chunks_exact(4)
            .map(|p| u32::from_le_bytes([p[0], p[1], p[2], 0]))
            .collect();
        if let Ok(mut slot) = latest.lock() {
            *slot = Some(Frame {
                width: width as usize,
                height: height as usize,
                pixels,
            });
        }
    }
}

fn read_observations(mut stream: UnixStream, address: &Mutex<String>) {
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        match decode_observation(&buffer) {
            Ok((observation, used)) => {
                buffer.drain(..used);
                if let (Observation::Address { url }, Ok(mut slot)) = (observation, address.lock())
                {
                    *slot = url;
                }
            }
            Err(DecodeError::Incomplete) => match stream.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(count) => buffer.extend_from_slice(&chunk[..count]),
            },
            Err(error) => {
                eprintln!("viewer: closing membrane: {error}");
                return;
            }
        }
    }
}

/// Keyboard events in the order they happened. Polling once per frame would
/// lose any key pressed and released within one frame.
enum Input {
    Character(char),
    Key(Key, bool),
}

struct Keyboard(Shared<Vec<Input>>);

impl InputCallback for Keyboard {
    fn add_char(&mut self, code: u32) {
        if let (Some(character), Ok(mut queue)) = (char::from_u32(code), self.0.lock()) {
            if !character.is_control() {
                queue.push(Input::Character(character));
            }
        }
    }

    fn set_key_state(&mut self, key: Key, down: bool) {
        if let Ok(mut queue) = self.0.lock() {
            queue.push(Input::Key(key, down));
        }
    }
}

struct Membrane(Shared<Option<UnixStream>>);

impl Membrane {
    fn send(&self, command: &Command) {
        let (Ok(frame), Ok(mut slot)) = (command.encode(), self.0.lock()) else {
            return;
        };
        if let Some(stream) = slot.as_mut() {
            let _ = stream.write_all(&frame);
        }
    }
}

fn to_wire_key(key: Key) -> Option<WireKey> {
    Some(match key {
        Key::Enter | Key::NumPadEnter => WireKey::Enter,
        Key::Tab => WireKey::Tab,
        Key::Backspace => WireKey::Backspace,
        Key::Delete => WireKey::Delete,
        Key::Escape => WireKey::Escape,
        Key::Left => WireKey::Left,
        Key::Right => WireKey::Right,
        Key::Up => WireKey::Up,
        Key::Down => WireKey::Down,
        Key::Home => WireKey::Home,
        Key::End => WireKey::End,
        Key::PageUp => WireKey::PageUp,
        Key::PageDown => WireKey::PageDown,
        _ => return None,
    })
}

/// Schemes that are not followed by `//`, and open as typed.
const BARE_SCHEMES: [&str; 5] = ["about:", "chrome:", "data:", "view-source:", "file:"];

/// No search engine is built in: what you type is where you go.
fn to_url(typed: &str) -> String {
    let typed = typed.trim();
    if typed.contains("://") || BARE_SCHEMES.iter().any(|scheme| typed.starts_with(scheme)) {
        typed.to_owned()
    } else {
        format!("https://{typed}")
    }
}

/// Text being typed into the address bar.
struct Entry {
    text: String,
    /// Still the old address: the first key replaces it, as if selected.
    fresh: bool,
}

/// What the keyboard is doing: typing an address, or talking to the page.
#[derive(Default)]
struct Typing {
    modifiers: Modifiers,
    entry: Option<Entry>,
}

impl Typing {
    fn begin(&mut self, address: String) {
        self.entry = Some(Entry {
            text: address,
            fresh: true,
        });
    }

    fn on_key(&mut self, key: Key, down: bool, address: &str, membrane: &Membrane) {
        match key {
            Key::LeftCtrl | Key::RightCtrl => self.modifiers.control = down,
            Key::LeftAlt | Key::RightAlt => self.modifiers.alt = down,
            Key::LeftShift | Key::RightShift => self.modifiers.shift = down,
            _ if down => self.on_press(key, address, membrane),
            _ => {}
        }
    }

    fn on_press(&mut self, key: Key, address: &str, membrane: &Membrane) {
        if let Some(entry) = self.entry.as_mut() {
            match key {
                Key::Enter | Key::NumPadEnter => {
                    membrane.send(&Command::Navigate {
                        url: to_url(&entry.text),
                    });
                    self.entry = None;
                }
                Key::Escape => self.entry = None,
                Key::Backspace => {
                    entry.fresh = false;
                    entry.text.pop();
                }
                _ => {}
            }
            return;
        }
        match key {
            Key::L if self.modifiers.control => self.begin(address.to_owned()),
            Key::Left if self.modifiers.alt => membrane.send(&Command::Back),
            Key::Right if self.modifiers.alt => membrane.send(&Command::Forward),
            Key::F5 => membrane.send(&Command::Reload),
            Key::Equal | Key::NumPadPlus if self.modifiers.control => {
                membrane.send(&Command::Zoom { step: Zoom::In });
            }
            Key::Minus | Key::NumPadMinus if self.modifiers.control => {
                membrane.send(&Command::Zoom { step: Zoom::Out });
            }
            Key::Key0 | Key::NumPad0 if self.modifiers.control => {
                membrane.send(&Command::Zoom { step: Zoom::Reset });
            }
            _ => {
                if let Some(key) = to_wire_key(key) {
                    let modifiers = self.modifiers;
                    membrane.send(&Command::Key { key, modifiers });
                }
            }
        }
    }

    fn on_character(&mut self, character: char, membrane: &Membrane) {
        if self.modifiers.control || self.modifiers.alt {
            return;
        }
        match self.entry.as_mut() {
            Some(entry) => {
                if entry.fresh {
                    entry.text.clear();
                    entry.fresh = false;
                }
                entry.text.push(character);
            }
            None => membrane.send(&Command::Text { character }),
        }
    }
}

#[derive(Default)]
struct Clicks {
    last: Option<(Instant, (u16, u16))>,
    count: u8,
}

impl Clicks {
    fn press(&mut self, at: (u16, u16)) -> u8 {
        let repeat = self
            .last
            .is_some_and(|(time, spot)| spot == at && time.elapsed() < DOUBLE_CLICK);
        self.count = if repeat { (self.count + 1).min(3) } else { 1 };
        self.last = Some((Instant::now(), at));
        self.count
    }
}

fn accept_in_background<F>(socket: &Path, serve: F) -> std::io::Result<()>
where
    F: FnOnce(UnixStream) + Send + 'static,
{
    let listener = UnixListener::bind(socket)?;
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            serve(stream);
        }
    });
    Ok(())
}

/// Keeps a frame, cut to fit whatever size arrived.
fn store_frame(page: &mut [u32], frame: &Frame) {
    let width = frame.width.min(PAGE_WIDTH);
    for y in 0..frame.height.min(PAGE_HEIGHT) {
        let target = y * PAGE_WIDTH;
        let source = y * frame.width;
        page[target..target + width].copy_from_slice(&frame.pixels[source..source + width]);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: viewer <work-dir> <url> [engine flags]";
    let work = PathBuf::from(args.next().ok_or(usage)?);
    let url = args.next().ok_or(usage)?;
    let flags: Vec<String> = args.collect();

    let run = RunDir::create()?;
    let latest: Shared<Option<Frame>> = Arc::default();
    let writer = latest.clone();
    accept_in_background(&run.socket("pixels.sock"), move |stream| {
        if let Err(error) = read_frames(stream, &writer) {
            eprintln!("viewer: pixels ended: {error}");
        }
    })?;

    let address: Shared<String> = Arc::default();
    let channel: Shared<Option<UnixStream>> = Arc::default();
    let reported = address.clone();
    let connected = channel.clone();
    accept_in_background(&run.socket("membrane.sock"), move |stream| {
        if let (Ok(reader), Ok(mut slot)) = (stream.try_clone(), connected.lock()) {
            *slot = Some(stream);
            drop(slot);
            read_observations(reader, &reported);
        }
    })?;
    let membrane = Membrane(channel);
    let mut compartment = Compartment::start(&work, &run, &url, &flags)?;

    let options = WindowOptions {
        resize: true,
        scale_mode: ScaleMode::UpperLeft,
        ..WindowOptions::default()
    };
    let mut window = Window::new("Frame", 1280, 720 + BAR_HEIGHT, options)?;
    window.set_target_fps(POLLS_PER_SECOND);
    let keyboard: Shared<Vec<Input>> = Arc::default();
    window.set_input_callback(Box::new(Keyboard(keyboard.clone())));

    let mut page = vec![0; PAGE_WIDTH * PAGE_HEIGHT];
    let mut layout = Layout::new(1280, 720 + BAR_HEIGHT, BAR_HEIGHT);
    let mut canvas = vec![0; layout.width * layout.height];
    let mut drawn: Option<(String, bool)> = None;
    let mut page_changed = true;
    let mut last_redraw = Instant::now();
    let mut typing = Typing::default();
    let mut position = None;
    let mut held = [false; 3];
    let mut clicks = Clicks::default();
    let buttons = [
        (MouseButton::Left, WireButton::Left),
        (MouseButton::Middle, WireButton::Middle),
        (MouseButton::Right, WireButton::Right),
    ];

    while window.is_open() && compartment.is_running() {
        let (width, height) = window.get_size();
        if (width, height) != (layout.width, layout.height) {
            layout = Layout::new(width, height, BAR_HEIGHT);
            canvas = vec![0; width * height];
            page_changed = true;
            drawn = None;
        }
        if let Ok(mut slot) = latest.lock() {
            if let Some(frame) = slot.take() {
                store_frame(&mut page, &frame);
                page_changed = true;
            }
        }
        let current = address.lock().map(|url| url.clone()).unwrap_or_default();
        let wanted = match &typing.entry {
            Some(entry) => (entry.text.clone(), true),
            None => (current.clone(), false),
        };
        let mut dirty = page_changed;
        if page_changed {
            draw_page(&page, &mut canvas, &layout);
            page_changed = false;
        }
        if drawn.as_ref() != Some(&wanted) {
            if canvas.len() >= width * BAR_HEIGHT {
                bar::draw(&mut canvas, width, &wanted.0, wanted.1);
            }
            drawn = Some(wanted);
            dirty = true;
        }
        if dirty || last_redraw.elapsed() >= REDRAW_AT_LEAST {
            window.update_with_buffer(&canvas, width, height)?;
            last_redraw = Instant::now();
        } else {
            window.update();
        }

        if let Some((mouse_x, mouse_y)) = window.get_unscaled_mouse_pos(MouseMode::Pass) {
            let on_bar = (0.0..BAR_HEIGHT as f32).contains(&mouse_y)
                && (0.0..width as f32).contains(&mouse_x);
            let spot = layout.to_page(mouse_x, mouse_y);
            if let (Some((x, y)), true) = (spot, position != spot) {
                position = spot;
                membrane.send(&Command::MouseMove { x, y });
            }
            if let (Some((x, y)), Some((wheel_x, wheel_y))) = (spot, window.get_scroll_wheel()) {
                if typing.modifiers.control {
                    let step = if wheel_y > 0.0 { Zoom::In } else { Zoom::Out };
                    membrane.send(&Command::Zoom { step });
                } else {
                    membrane.send(&Command::Wheel {
                        x,
                        y,
                        delta_x: (wheel_x * WHEEL_STEP) as i16,
                        delta_y: (wheel_y * WHEEL_STEP) as i16,
                    });
                }
            }
            for (index, (button, wire)) in buttons.into_iter().enumerate() {
                let pressed = window.get_mouse_down(button);
                if pressed == held[index] {
                    continue;
                }
                held[index] = pressed;
                if on_bar {
                    if pressed && button == MouseButton::Left {
                        match bar::hit(mouse_x as usize) {
                            Hit::Back => membrane.send(&Command::Back),
                            Hit::Forward => membrane.send(&Command::Forward),
                            Hit::Reload => membrane.send(&Command::Reload),
                            Hit::Address => typing.begin(current.clone()),
                        }
                    }
                    continue;
                }
                let Some((x, y)) = spot else {
                    continue;
                };
                if pressed {
                    typing.entry = None;
                }
                let count = if pressed {
                    clicks.press((x, y))
                } else {
                    clicks.count.max(1)
                };
                membrane.send(&Command::MouseButton {
                    x,
                    y,
                    button: wire,
                    pressed,
                    clicks: count,
                });
            }
        }

        let events: Vec<Input> = keyboard
            .lock()
            .map(|mut queue| queue.drain(..).collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Input::Key(key, down) => typing.on_key(key, down, &current, &membrane),
                Input::Character(character) => typing.on_character(character, &membrane),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_pages_open_as_typed() {
        for typed in [
            "about:about",
            "chrome://version",
            "chrome:gpu",
            "data:text/html,x",
        ] {
            assert_eq!(to_url(typed), typed);
        }
    }

    #[test]
    fn a_bare_name_becomes_https() {
        assert_eq!(to_url(" example.com "), "https://example.com");
    }
}
