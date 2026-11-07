//! The host side of a compartment's jail, and the one part of the launcher
//! that is written per OS. Each host OS implements [`enter`]. An OS without an
//! implementation refuses to start: a compartment never runs unjailed.

use std::path::PathBuf;

/// What the jail must let the VMM reach. Nothing else is visible after entry.
pub struct JailPaths {
    pub guest_root: PathBuf,
    pub krun_dir: PathBuf,
    pub sockets: PathBuf,
}

/// Which side of the jail this process came out on.
pub enum Entered {
    /// Outside, waiting for the VM. Holds the exit code.
    Supervisor(i32),
    /// Inside: start the VM.
    Jailed,
}

#[cfg(target_os = "linux")]
pub use linux::enter;

#[cfg(not(target_os = "linux"))]
pub fn enter(_paths: &JailPaths, _lifeline: bool) -> crate::Outcome<Entered> {
    Err("no jail for this OS yet, refusing to start".into())
}

#[cfg(target_os = "linux")]
mod linux {
    //! No network, no view of the host's files or processes, no privileges.

    use super::{Entered, JailPaths};
    use crate::{Outcome, step};
    use caps::{CapSet, clear};
    use nix::mount::{MntFlags, MsFlags, mount, umount2};
    use nix::poll::{PollFd, PollFlags, poll};
    use nix::sched::{CloneFlags, unshare};
    use nix::sys::prctl::{set_no_new_privs, set_pdeathsig};
    use nix::sys::signal::Signal;
    use nix::sys::statvfs::{FsFlags, statvfs};
    use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
    use nix::unistd::{ForkResult, Gid, Pid, Uid, chdir, fork, pivot_root};
    use std::fs::{self, File};
    use std::os::fd::AsFd;
    use std::path::{Path, PathBuf};

    const JAIL_ROOT: &str = "/mnt";
    const POLL_MS: u16 = 200;

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

    /// Waits for the VM. With a lifeline, also returns once stdin closes, and
    /// this process exiting kills the jailed child through its death signal.
    /// This is one loop because a process that entered a new PID namespace can
    /// no longer start threads.
    fn supervise(child: Pid, lifeline: bool) -> Outcome<i32> {
        let flags = lifeline.then_some(WaitPidFlag::WNOHANG);
        loop {
            match step("wait", waitpid(child, flags))? {
                WaitStatus::StillAlive => {}
                WaitStatus::Exited(_, code) => return Ok(code),
                _ => return Ok(1),
            }
            let stdin = std::io::stdin();
            let mut watched = [PollFd::new(stdin.as_fd(), PollFlags::empty())];
            step("poll", poll(&mut watched, POLL_MS))?;
            if watched[0]
                .revents()
                .is_some_and(|events| !events.is_empty())
            {
                return Ok(0);
            }
        }
    }

    /// Jails this process. Must run before any other thread exists.
    pub fn enter(paths: &JailPaths, lifeline: bool) -> Outcome<Entered> {
        enter_namespaces()?;
        // SAFETY: no other threads exist yet, so the child inherits a sound state.
        match step("fork", unsafe { fork() })? {
            ForkResult::Parent { child } => Ok(Entered::Supervisor(supervise(child, lifeline)?)),
            ForkResult::Child => {
                step("parent watch", set_pdeathsig(Signal::SIGKILL))?;
                build_root(&paths.guest_root, &paths.krun_dir, &paths.sockets)?;
                drop_privileges()?;
                Ok(Entered::Jailed)
            }
        }
    }
}
