//! The Rust window on the host. Reads frames from the guest and shows them.
//! Everything from the socket is hostile, so sizes are checked before use.
//!
//! Usage: viewer <socket> [frame-dump.bgra]

use minifb::{Key, ScaleMode, Window, WindowOptions};
use std::io::Read;
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_WIDTH: u32 = 3840;
const MAX_HEIGHT: u32 = 2160;

struct Frame {
    width: usize,
    height: usize,
    pixels: Vec<u32>,
}

fn read_u32(stream: &mut impl Read) -> std::io::Result<u32> {
    let mut bytes = [0; 4];
    stream.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_frames(mut stream: impl Read, latest: &Mutex<Option<Frame>>) -> std::io::Result<()> {
    let mut count = 0u64;
    let mut window_start = Instant::now();
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
        count += 1;
        if window_start.elapsed() >= Duration::from_secs(5) {
            println!("VIEWER {:.1} frames/s", count as f64 / 5.0);
            count = 0;
            window_start = Instant::now();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = std::env::args().nth(1).ok_or("usage: viewer <socket>")?;
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    let latest = Arc::new(Mutex::new(None));
    let writer = latest.clone();
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            println!("VIEWER guest connected");
            if let Err(error) = read_frames(stream, &writer) {
                eprintln!("viewer: stream ended: {error}");
            }
        }
    });

    let options = WindowOptions {
        resize: true,
        scale_mode: ScaleMode::AspectRatioStretch,
        ..WindowOptions::default()
    };
    let mut window = Window::new("Frame spike 1", 1280, 720, options)?;
    window.set_target_fps(30);
    let mut current: Option<Frame> = None;
    while window.is_open() && !window.is_key_down(Key::Escape) {
        if let Ok(mut slot) = latest.lock() {
            if let Some(frame) = slot.take() {
                current = Some(frame);
            }
        }
        match &current {
            Some(frame) => window.update_with_buffer(&frame.pixels, frame.width, frame.height)?,
            None => window.update(),
        }
    }
    Ok(())
}
