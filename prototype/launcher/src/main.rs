//! Starts one web compartment. Before any VM code runs, this process jails
//! itself: see `jail`. This is code, not host setup, and if any step fails the
//! compartment does not start.
//!
//! The guest has no network device. Its only channels are three vsock ports,
//! each mapped to one Unix socket in `<sockets>`.
//!
//! Usage: launcher <root> <memory-mib> <vcpus> <krun-dir> <sockets> -- <cmd> [args]

mod jail;

use jail::{Entered, JailPaths};
use std::ffi::{CString, c_char};
use std::fs;
use std::ptr;

unsafe extern "C" {
    fn krun_set_log_level(level: u32) -> i32;
    fn krun_create_ctx() -> i32;
    fn krun_set_vm_config(ctx: u32, vcpus: u8, ram_mib: u32) -> i32;
    fn krun_set_root(ctx: u32, path: *const c_char) -> i32;
    fn krun_set_exec(
        ctx: u32,
        path: *const c_char,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> i32;
    fn krun_disable_implicit_vsock(ctx: u32) -> i32;
    fn krun_add_vsock(ctx: u32, tsi_features: u32) -> i32;
    fn krun_add_vsock_port2(ctx: u32, port: u32, path: *const c_char, listen: bool) -> i32;
    fn krun_start_enter(ctx: u32) -> i32;
}

/// Every channel a guest has. There is no other way out.
const CHANNELS: [(u32, &str); 3] = [
    (5000, "/sockets/pixels.sock"),
    (5001, "/sockets/gate.sock"),
    (5002, "/sockets/membrane.sock"),
];
/// Off: guest sockets stay in the guest instead of becoming host sockets.
const NO_TSI: u32 = 0;
const GUEST_ROOT: &str = "/guest";

pub type Outcome<T = ()> = Result<T, String>;

pub fn step<T, E: std::fmt::Display>(name: &str, result: Result<T, E>) -> Outcome<T> {
    result.map_err(|error| format!("{name}: {error}"))
}

fn check(name: &str, code: i32) -> Outcome {
    if code < 0 {
        Err(format!("{name} failed: {code}"))
    } else {
        Ok(())
    }
}

fn c(text: &str) -> Outcome<CString> {
    step("text", CString::new(text))
}

fn start_vm(memory: u32, vcpus: u8, command: &[String]) -> Outcome {
    let root = c(GUEST_ROOT)?;
    let exec = c(&command[0])?;
    let argv: Vec<CString> = command[1..].iter().map(|a| c(a)).collect::<Outcome<_>>()?;
    let mut argv_ptrs: Vec<*const c_char> = argv.iter().map(|a| a.as_ptr()).collect();
    argv_ptrs.push(ptr::null());
    let env = [
        c("HOME=/tmp")?,
        c("LANG=en_US.UTF-8")?,
        c("TZ=UTC")?,
        c("PATH=/bin")?,
    ];
    let mut env_ptrs: Vec<*const c_char> = env.iter().map(|e| e.as_ptr()).collect();
    env_ptrs.push(ptr::null());
    let channels: Vec<(u32, CString)> = CHANNELS
        .iter()
        .map(|(port, path)| Ok((*port, c(path)?)))
        .collect::<Outcome<_>>()?;

    // SAFETY: every pointer outlives the calls, and `krun_start_enter` does
    // not return on success, so nothing is freed while the VM runs.
    unsafe {
        check("log level", krun_set_log_level(1))?;
        let ctx = krun_create_ctx();
        check("create ctx", ctx)?;
        let ctx = step("ctx", u32::try_from(ctx))?;
        check("vm config", krun_set_vm_config(ctx, vcpus, memory))?;
        check("root", krun_set_root(ctx, root.as_ptr()))?;
        check("no implicit vsock", krun_disable_implicit_vsock(ctx))?;
        check("vsock without TSI", krun_add_vsock(ctx, NO_TSI))?;
        for (port, path) in &channels {
            check(
                "channel",
                krun_add_vsock_port2(ctx, *port, path.as_ptr(), false),
            )?;
        }
        check(
            "exec",
            krun_set_exec(ctx, exec.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr()),
        )?;
        check("start", krun_start_enter(ctx))?;
    }
    Ok(())
}

fn run() -> Outcome<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let split = args.iter().position(|a| a == "--").ok_or("missing --")?;
    let (options, rest) = args.split_at(split);
    let command = rest
        .get(1..)
        .filter(|c| !c.is_empty())
        .ok_or("no command")?;
    let [guest_root, memory, vcpus, krun_dir, sockets] = options else {
        return Err(
            "usage: launcher <root> <memory-mib> <vcpus> <krun-dir> <sockets> -- <cmd>".into(),
        );
    };
    let memory: u32 = memory.parse().map_err(|_| "bad memory")?;
    let vcpus: u8 = vcpus.parse().map_err(|_| "bad vcpus")?;
    let guest_root = step("root path", fs::canonicalize(guest_root))?;
    let krun_dir = step("krun path", fs::canonicalize(krun_dir))?;
    let sockets = step("socket path", fs::canonicalize(sockets))?;

    let paths = JailPaths {
        guest_root,
        krun_dir,
        sockets,
    };
    match jail::enter(&paths)? {
        Entered::Supervisor(code) => Ok(code),
        Entered::Jailed => {
            start_vm(memory, vcpus, command)?;
            Ok(0)
        }
    }
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("launcher: {error}");
            std::process::exit(1);
        }
    }
}
