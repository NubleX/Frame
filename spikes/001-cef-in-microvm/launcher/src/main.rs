//! Starts one microVM through libkrun. `krun_start_enter` never returns: this
//! process becomes the VMM, and exits with the guest.
//!
//! Usage: launcher <root> <memory-mib> <vcpus> [--frames <socket>] -- <cmd> [args]

use std::ffi::{CString, c_char};
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
    fn krun_add_vsock_port2(ctx: u32, port: u32, path: *const c_char, listen: bool) -> i32;
    fn krun_start_enter(ctx: u32) -> i32;
}

const FRAME_PORT: u32 = 5000;

fn check(step: &str, code: i32) -> Result<(), String> {
    if code < 0 {
        Err(format!("{step} failed: {code}"))
    } else {
        Ok(())
    }
}

fn c(text: &str) -> Result<CString, String> {
    CString::new(text).map_err(|error| error.to_string())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let split = args.iter().position(|a| a == "--").ok_or("missing --")?;
    let (options, rest) = args.split_at(split);
    let command = rest.get(1..).filter(|c| !c.is_empty()).ok_or("no command")?;
    let [root, memory, vcpus, tail @ ..] = options else {
        return Err("usage: launcher <root> <memory-mib> <vcpus> [--frames <socket>] -- <cmd>".into());
    };
    let memory: u32 = memory.parse().map_err(|_| "bad memory")?;
    let vcpus: u8 = vcpus.parse().map_err(|_| "bad vcpus")?;

    let root = c(root)?;
    let exec = c(&command[0])?;
    let argv: Vec<CString> = command[1..].iter().map(|a| c(a)).collect::<Result<_, _>>()?;
    let mut argv_ptrs: Vec<*const c_char> = argv.iter().map(|a| a.as_ptr()).collect();
    argv_ptrs.push(ptr::null());
    let env = [c("HOME=/tmp")?, c("LANG=en_US.UTF-8")?, c("TZ=UTC")?, c("PATH=/bin")?];
    let mut env_ptrs: Vec<*const c_char> = env.iter().map(|e| e.as_ptr()).collect();
    env_ptrs.push(ptr::null());

    // SAFETY: every pointer outlives the calls, and `krun_start_enter` does
    // not return on success, so nothing is freed while the VM runs.
    unsafe {
        check("log level", krun_set_log_level(2))?;
        let ctx = krun_create_ctx();
        check("create ctx", ctx)?;
        let ctx = u32::try_from(ctx).map_err(|_| "bad ctx")?;
        check("vm config", krun_set_vm_config(ctx, vcpus, memory))?;
        check("root", krun_set_root(ctx, root.as_ptr()))?;
        if let [flag, socket] = tail {
            if flag == "--frames" {
                let socket = c(socket)?;
                check(
                    "frame port",
                    krun_add_vsock_port2(ctx, FRAME_PORT, socket.as_ptr(), false),
                )?;
                std::mem::forget(socket);
            }
        }
        check(
            "exec",
            krun_set_exec(ctx, exec.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr()),
        )?;
        check("start", krun_start_enter(ctx))?;
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("launcher: {error}");
        std::process::exit(1);
    }
}
