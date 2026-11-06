//! The window on the host. Pixels come in, typed commands go out, and nothing
//! else crosses. Everything from the guest is hostile, so sizes are checked
//! and observations are decoded strictly.
//!
//! Keys: Ctrl+L address, Enter go, Alt+Left back, Alt+Right forward, F5 reload.
//!
//! Usage: viewer <pixels-socket> <membrane-socket> [--title <text>]

use frame_membrane::{
    Command, DecodeError, Key as WireKey, Modifiers, MouseButton as WireButton, Observation,
    decode_observation,
};
use minifb::{InputCallback, Key, MouseButton, MouseMode, ScaleMode, Window, WindowOptions};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_WIDTH: u32 = 3840;
const MAX_HEIGHT: u32 = 2160;
const DISPLAY: (usize, usize) = (1920, 1080);
const WHEEL_STEP: f32 = 60.0;
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

/// Window position to guest display position, undoing the letterboxing.
fn to_display(window: (usize, usize), mouse: (f32, f32)) -> Option<(u16, u16)> {
    let (window_w, window_h) = (window.0 as f32, window.1 as f32);
    let (display_w, display_h) = (DISPLAY.0 as f32, DISPLAY.1 as f32);
    let scale = (window_w / display_w).min(window_h / display_h);
    let x = (mouse.0 - (window_w - display_w * scale) / 2.0) / scale;
    let y = (mouse.1 - (window_h - display_h * scale) / 2.0) / scale;
    let inside = (0.0..display_w).contains(&x) && (0.0..display_h).contains(&y);
    inside.then_some((x as u16, y as u16))
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

/// No search engine is built in: what you type is where you go.
fn to_url(typed: &str) -> String {
    let typed = typed.trim();
    if typed.contains("://") || typed.starts_with("about:") {
        typed.to_owned()
    } else {
        format!("https://{typed}")
    }
}

/// What the keyboard is doing: typing an address, or talking to the page.
#[derive(Default)]
struct Typing {
    modifiers: Modifiers,
    entry: Option<String>,
}

impl Typing {
    fn on_key(&mut self, key: Key, down: bool, membrane: &Membrane) {
        match key {
            Key::LeftCtrl | Key::RightCtrl => self.modifiers.control = down,
            Key::LeftAlt | Key::RightAlt => self.modifiers.alt = down,
            Key::LeftShift | Key::RightShift => self.modifiers.shift = down,
            _ if down => self.on_press(key, membrane),
            _ => {}
        }
    }

    fn on_press(&mut self, key: Key, membrane: &Membrane) {
        if let Some(text) = self.entry.as_mut() {
            match key {
                Key::Enter | Key::NumPadEnter => {
                    membrane.send(&Command::Navigate { url: to_url(text) });
                    self.entry = None;
                }
                Key::Escape => self.entry = None,
                Key::Backspace => {
                    text.pop();
                }
                _ => {}
            }
            return;
        }
        match key {
            Key::L if self.modifiers.control => self.entry = Some(String::new()),
            Key::Left if self.modifiers.alt => membrane.send(&Command::Back),
            Key::Right if self.modifiers.alt => membrane.send(&Command::Forward),
            Key::F5 => membrane.send(&Command::Reload),
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
            Some(text) => text.push(character),
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

fn accept_in_background<F>(socket: &str, serve: F) -> std::io::Result<()>
where
    F: FnOnce(UnixStream) + Send + 'static,
{
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            serve(stream);
        }
    });
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: viewer <pixels-socket> <membrane-socket>";
    let pixels_socket = args.next().ok_or(usage)?;
    let membrane_socket = args.next().ok_or(usage)?;

    let latest: Shared<Option<Frame>> = Arc::default();
    let writer = latest.clone();
    accept_in_background(&pixels_socket, move |stream| {
        if let Err(error) = read_frames(stream, &writer) {
            eprintln!("viewer: pixels ended: {error}");
        }
    })?;

    let address: Shared<String> = Arc::default();
    let channel: Shared<Option<UnixStream>> = Arc::default();
    let reported = address.clone();
    let connected = channel.clone();
    accept_in_background(&membrane_socket, move |stream| {
        if let (Ok(reader), Ok(mut slot)) = (stream.try_clone(), connected.lock()) {
            *slot = Some(stream);
            drop(slot);
            read_observations(reader, &reported);
        }
    })?;
    let membrane = Membrane(channel);

    let options = WindowOptions {
        resize: true,
        scale_mode: ScaleMode::AspectRatioStretch,
        ..WindowOptions::default()
    };
    let mut window = Window::new("Frame", 1280, 720, options)?;
    window.set_target_fps(30);
    let keyboard: Shared<Vec<Input>> = Arc::default();
    window.set_input_callback(Box::new(Keyboard(keyboard.clone())));

    let mut current: Option<Frame> = None;
    let mut title = String::new();
    let mut typing = Typing::default();
    let mut position = None;
    let mut held = [false; 3];
    let mut clicks = Clicks::default();
    let buttons = [
        (MouseButton::Left, WireButton::Left),
        (MouseButton::Middle, WireButton::Middle),
        (MouseButton::Right, WireButton::Right),
    ];

    while window.is_open() {
        if let Ok(mut slot) = latest.lock() {
            if let Some(frame) = slot.take() {
                current = Some(frame);
            }
        }
        match &current {
            Some(frame) => window.update_with_buffer(&frame.pixels, frame.width, frame.height)?,
            None => window.update(),
        }

        let spot = window
            .get_unscaled_mouse_pos(MouseMode::Pass)
            .and_then(|mouse| to_display(window.get_size(), mouse));
        if let Some((x, y)) = spot {
            if position != spot {
                position = spot;
                membrane.send(&Command::MouseMove { x, y });
            }
            if let Some((wheel_x, wheel_y)) = window.get_scroll_wheel() {
                membrane.send(&Command::Wheel {
                    x,
                    y,
                    delta_x: (wheel_x * WHEEL_STEP) as i16,
                    delta_y: (wheel_y * WHEEL_STEP) as i16,
                });
            }
            for (index, (button, wire)) in buttons.into_iter().enumerate() {
                let pressed = window.get_mouse_down(button);
                if pressed != held[index] {
                    held[index] = pressed;
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
        }

        let events: Vec<Input> = keyboard
            .lock()
            .map(|mut queue| queue.drain(..).collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Input::Key(key, down) => typing.on_key(key, down, &membrane),
                Input::Character(character) => typing.on_character(character, &membrane),
            }
        }

        let shown = match &typing.entry {
            Some(text) => format!("Go to: {text}_"),
            None => address
                .lock()
                .map(|url| format!("Frame - {url}"))
                .unwrap_or_else(|_| "Frame".to_owned()),
        };
        if shown != title {
            window.set_title(&shown);
            title = shown;
        }
    }
    Ok(())
}
