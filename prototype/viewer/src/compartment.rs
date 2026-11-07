//! Starts the gate and the VM, and owns them. Each child holds this process's
//! end of a pipe on its stdin and exits when the pipe closes. The kernel closes
//! it however this process ends, even on a crash, so nothing can outlive the
//! window.

use std::fs::{self, DirBuilder, File};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const MEMORY_MIB: &str = "1024";
const VCPUS: &str = "2";
const GATE_WAIT: Duration = Duration::from_secs(5);

/// The folder holding one compartment's sockets. Gone when this is dropped.
pub struct RunDir(PathBuf);

impl RunDir {
    pub fn create() -> std::io::Result<Self> {
        let base =
            std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let path = base.join(format!("frame-{}", std::process::id()));
        DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }

    pub fn socket(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct Compartment {
    children: Vec<Child>,
}

impl Compartment {
    pub fn start(
        work: &Path,
        run: &RunDir,
        url: &str,
        flags: &[String],
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let binaries = work.join("target/release");
        let gate_socket = run.socket("gate.sock");
        let gate = Command::new(binaries.join("frame-gate"))
            .arg(&gate_socket)
            .arg("--lifeline")
            .stdin(Stdio::piped())
            .stderr(File::create(work.join("gate.log"))?)
            .spawn()?;
        let mut compartment = Self {
            children: vec![gate],
        };

        let started = Instant::now();
        while !gate_socket.exists() {
            if started.elapsed() > GATE_WAIT {
                return Err("the gate did not start".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        // The jail keeps only the real path, so a symlink must be resolved first.
        let krun = fs::canonicalize(work.join("prefix/krun"))?;
        let log = File::create(work.join("guest.log"))?;
        let vm = Command::new(binaries.join("launcher"))
            .env("LD_LIBRARY_PATH", &krun)
            .arg(work.join("root"))
            .args([MEMORY_MIB, VCPUS])
            .arg(&krun)
            .arg(&run.0)
            .args(["--lifeline", "--", "/bin/start"])
            .arg(format!("--frame-url={url}"))
            .args(flags)
            .stdin(Stdio::piped())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?;
        compartment.children.push(vm);
        Ok(compartment)
    }

    /// True while the VM is still running.
    pub fn is_running(&mut self) -> bool {
        self.children
            .iter_mut()
            .all(|child| matches!(child.try_wait(), Ok(None)))
    }
}

impl Drop for Compartment {
    fn drop(&mut self) {
        for child in &mut self.children {
            drop(child.stdin.take());
        }
        for child in &mut self.children {
            let _ = child.wait();
        }
    }
}
