//! Runs the gate on a Unix socket. Usage: `frame-gate <socket>`

use frame_gate::{SystemResolver, handle};
use std::fs::{Permissions, remove_file, set_permissions};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Open connections allowed for one compartment.
const MAX_CONNECTIONS: usize = 256;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = std::env::args()
        .nth(1)
        .ok_or("usage: frame-gate <socket>")?;
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
