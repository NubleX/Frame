//! Runs the gate. Usage: `frame-gate <socket> [--lifeline]`
//!
//! With `--lifeline` it exits when stdin closes, so it cannot outlive the
//! program that started it.
//!
//! Only this listener is per OS. The rules live in the library.

use frame_gate::{SystemResolver, handle};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Open connections allowed for one compartment.
const MAX_CONNECTIONS: usize = 256;

#[cfg(not(unix))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("no channel listener for this OS yet".into())
}

#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::fs::{Permissions, remove_file, set_permissions};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    let socket = std::env::args()
        .nth(1)
        .ok_or("usage: frame-gate <socket> [--lifeline]")?;
    if std::env::args().nth(2).as_deref() == Some("--lifeline") {
        std::thread::spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            std::process::exit(0);
        });
    }
    let _ = remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    set_permissions(&socket, Permissions::from_mode(0o600))?;
    let open = Arc::new(AtomicUsize::new(0));
    for client in listener.incoming() {
        let client = client?;
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let open = open.clone();
        std::thread::spawn(move || {
            let _ = handle(client, &SystemResolver);
            open.fetch_sub(1, Ordering::SeqCst);
        });
    }
    Ok(())
}
