//! Starts one web compartment. Before any VM code runs, this process jails
//! itself: no network, no view of the host's files or processes, no
//! privileges. This is code, not host setup, so every machine gets the same
//! jail, and if any step fails the compartment does not start.
//!
//! The guest has no network device. Its only channels are three vsock ports,
//! each mapped to one Unix socket in `<sockets>`.
//!
//! Usage: launcher <root> <memory-mib> <vcpus> <krun-dir> <sockets> -- <cmd> [args]

use caps::{CapSet, clear};
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, unshare};
use nix::sys::prctl::{set_no_new_privs, set_pdeathsig};
use nix::sys::signal::Signal;
use nix::sys::statvfs::{FsFlags, statvfs};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::{ForkResult, Gid, Uid, chdir, fork, pivot_root};
use std::ffi::{CString, c_char};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
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
const JAIL_ROOT: &str = "/mnt";
const GUEST_ROOT: &str = "/guest";

type Outcome<T = ()> = Result<T, String>;

fn step<T, E: std::fmt::Display>(name: &str, result: Result<T, E>) -> Outcome<T> {
    result.map_err(|error| format!("{name}: {error}"))
}

fn inside(path: &Path) -> PathBuf {
    Path::new(JAIL_ROOT).join(path.strip_prefix("/").unwrap_or(path))
}

fn enter_namespaces() -> Outcome {
    let (uid, gid) = (Uid::current(), Gid::current());
    step(
        "namespaces (unprivileged user namespaces must be allowed)",
        unshare(
            CloneFlags::CLONE_NEWUSER
                | CloneFlags::CLONE_NEWNS
                | CloneFlags::CLONE_NEWNET
                | CloneFlags::CLONE_NEWIPC
                | CloneFlags::CLONE_NEWUTS
                | CloneFlags::CLONE_NEWPID,
        ),
    )?;
    step("setgroups", fs::write("/proc/self/setgroups", "deny"))?;
    step(
        "uid map",
        fs::write("/proc/self/uid_map", format!("{uid} {uid} 1")),
    )?;
    step(
        "gid map",
        fs::write("/proc/self/gid_map", format!("{gid} {gid} 1")),
    )
}

/// Flags the kernel will not let an unprivileged remount drop.
fn locked_flags(target: &Path) -> Outcome<MsFlags> {
    let flags = step("statvfs", statvfs(target))?.flags();
    let mut kept = MsFlags::MS_NOSUID;
    for (from, to) in [
        (FsFlags::ST_NODEV, MsFlags::MS_NODEV),
        (FsFlags::ST_NOEXEC, MsFlags::MS_NOEXEC),
        (FsFlags::ST_NOATIME, MsFlags::MS_NOATIME),
        (FsFlags::ST_NODIRATIME, MsFlags::MS_NODIRATIME),
        (FsFlags::ST_RELATIME, MsFlags::MS_RELATIME),
    ] {
        if flags.contains(from) {
            kept |= to;
        }
    }
    Ok(kept)
}

fn bind(source: &Path, target: &Path, read_only: bool) -> Outcome {
    if source.is_dir() {
        step("bind target", fs::create_dir_all(target))?;
    } else {
        if let Some(parent) = target.parent() {
            step("bind parent", fs::create_dir_all(parent))?;
        }
        step("bind file", File::create(target))?;
    }
    step(
        "bind",
        mount(
            Some(source),
            target,
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        ),
    )?;
    if read_only {
        let flags = MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY;
        step(
            "read-only remount",
            mount(
                None::<&str>,
                target,
                None::<&str>,
                flags | locked_flags(target)?,
                None::<&str>,
            ),
        )?;
    }
    Ok(())
}

/// Leaves the VMM a root holding only what it needs. The rest of the host's
/// files do not exist from here on.
fn build_root(guest_root: &Path, krun_dir: &Path, sockets: &Path) -> Outcome {
    let root = Path::new(JAIL_ROOT);
    step(
        "private mounts",
        mount(
            None::<&str>,
            "/",
            None::<&str>,
            MsFlags::MS_REC | MsFlags::MS_PRIVATE,
            None::<&str>,
        ),
    )?;
    step(
        "jail tmpfs",
        mount(
            Some("tmpfs"),
            root,
            Some("tmpfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
            Some("size=1m,mode=755"),
        ),
    )?;
    bind(guest_root, &root.join("guest"), true)?;
    bind(krun_dir, &inside(krun_dir), true)?;
    bind(sockets, &root.join("sockets"), true)?;
    for device in ["/dev/kvm", "/dev/null", "/dev/urandom"] {
        bind(Path::new(device), &inside(Path::new(device)), false)?;
    }
    let proc = root.join("proc");
    step("proc dir", fs::create_dir_all(&proc))?;
    step(
        "proc",
        mount(
            Some("proc"),
            &proc,
            Some("proc"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
            None::<&str>,
        ),
    )?;
    step("old root dir", fs::create_dir_all(root.join("old")))?;
    step("enter jail", chdir(root))?;
    step("pivot", pivot_root(".", "old"))?;
    step("enter root", chdir("/"))?;
    step("drop host files", umount2("/old", MntFlags::MNT_DETACH))?;
    step("remove old root", fs::remove_dir("/old"))?;
    step(
        "read-only jail",
        mount(
            None::<&str>,
            "/",
            None::<&str>,
            MsFlags::MS_BIND
                | MsFlags::MS_REMOUNT
                | MsFlags::MS_RDONLY
                | locked_flags(Path::new("/"))?,
            None::<&str>,
        ),
    )
}

fn drop_privileges() -> Outcome {
    step("no new privileges", set_no_new_privs())?;
    for set in [CapSet::Inheritable, CapSet::Effective, CapSet::Permitted] {
        step("capabilities", clear(None, set))?;
    }
    Ok(())
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

    enter_namespaces()?;
    // SAFETY: no other threads exist yet, so the child inherits a sound state.
    match step("fork", unsafe { fork() })? {
        ForkResult::Parent { child } => match step("wait", waitpid(child, None))? {
            WaitStatus::Exited(_, code) => Ok(code),
            _ => Ok(1),
        },
        ForkResult::Child => {
            step("parent watch", set_pdeathsig(Signal::SIGKILL))?;
            build_root(&guest_root, &krun_dir, &sockets)?;
            drop_privileges()?;
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
