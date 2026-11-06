//! The boundary between Frame and whatever runs its microVMs.
//!
//! A [`VmSpec`] has no field for host paths, host devices or a chosen network.
//! Those are not options: only a gate reaches the host uplink, a web
//! compartment reaches only the gate, and every compartment gets the same
//! virtual hardware so nothing about the host shows through.

use std::fmt;

/// Identifies one compartment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompartmentId(pub u32);

/// What a microVM is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmKind {
    /// Owns routes and the host uplink.
    Gate,
    /// Runs the web engine.
    Web,
}

/// What a microVM's virtual NIC connects to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    /// The host's network. Gate only.
    HostUplink,
    /// The gate, and nothing else.
    GateOnly,
}

/// The hardware a guest sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualHardware {
    /// Virtual CPUs.
    pub vcpus: u8,
    /// Guest memory in MiB.
    pub memory_mib: u32,
    /// Display width in pixels.
    pub display_width: u16,
    /// Display height in pixels.
    pub display_height: u16,
}

/// Identical for every web compartment on every host. Spike 1 showed 1 GiB
/// runs ordinary pages. These numbers must stay the same across editions.
pub const CANONICAL_HARDWARE: VirtualHardware = VirtualHardware {
    vcpus: 2,
    memory_mib: 1024,
    display_width: 1920,
    display_height: 1080,
};

/// Everything needed to start one microVM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmSpec {
    id: CompartmentId,
    kind: VmKind,
    image: String,
}

impl VmSpec {
    /// A gate microVM.
    #[must_use]
    pub fn gate(id: CompartmentId, image: impl Into<String>) -> Self {
        Self {
            id,
            kind: VmKind::Gate,
            image: image.into(),
        }
    }

    /// A web compartment.
    #[must_use]
    pub fn web(id: CompartmentId, image: impl Into<String>) -> Self {
        Self {
            id,
            kind: VmKind::Web,
            image: image.into(),
        }
    }

    /// The compartment this spec describes.
    #[must_use]
    pub fn id(&self) -> CompartmentId {
        self.id
    }

    /// What the microVM is for.
    #[must_use]
    pub fn kind(&self) -> VmKind {
        self.kind
    }

    /// The guest image reference.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Where the virtual NIC connects. Derived from the kind, never configured.
    #[must_use]
    pub fn network(&self) -> Network {
        match self.kind {
            VmKind::Gate => Network::HostUplink,
            VmKind::Web => Network::GateOnly,
        }
    }

    /// The hardware the guest sees.
    #[must_use]
    pub fn hardware(&self) -> VirtualHardware {
        CANONICAL_HARDWARE
    }
}

/// A VMM operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmmError(pub String);

impl fmt::Display for VmmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vmm failure: {}", self.0)
    }
}

impl std::error::Error for VmmError {}

/// Runs microVMs. Implemented for libkrun later; see `docs/adr`.
pub trait Vmm {
    /// Starts a microVM.
    ///
    /// # Errors
    /// Returns [`VmmError`] if it did not start. The microVM may still be
    /// running, so callers must compare against [`Vmm::running`].
    fn start(&mut self, spec: &VmSpec) -> Result<(), VmmError>;

    /// Stops a microVM.
    ///
    /// # Errors
    /// Returns [`VmmError`] if it could not be stopped.
    fn stop(&mut self, id: CompartmentId) -> Result<(), VmmError>;

    /// Lists running microVMs.
    ///
    /// # Errors
    /// Returns [`VmmError`] if the state cannot be read.
    fn running(&self) -> Result<Vec<CompartmentId>, VmmError>;
}

/// An in-memory [`Vmm`] for tests, with failure injection.
#[cfg(feature = "test-double")]
pub mod fake {
    use super::{CompartmentId, VmSpec, Vmm, VmmError};
    use std::collections::BTreeSet;

    /// Records calls and can fail on demand.
    #[derive(Debug, Default)]
    pub struct FakeVmm {
        running: BTreeSet<CompartmentId>,
        /// Number of `start` calls.
        pub starts: u32,
        /// Number of `stop` calls.
        pub stops: u32,
        /// The next `start` boots the VM, then reports failure.
        pub die_after_next_start: bool,
        /// The next `start` fails with nothing running.
        pub fail_next_start: bool,
        /// The next `stop` fails with the VM still running.
        pub fail_next_stop: bool,
    }

    impl FakeVmm {
        /// Marks a VM as running without a `start` call, as a leftover would be.
        pub fn seed_running(&mut self, id: CompartmentId) {
            self.running.insert(id);
        }
    }

    impl Vmm for FakeVmm {
        fn start(&mut self, spec: &VmSpec) -> Result<(), VmmError> {
            self.starts += 1;
            if std::mem::take(&mut self.fail_next_start) {
                return Err(VmmError("start refused".into()));
            }
            self.running.insert(spec.id());
            if std::mem::take(&mut self.die_after_next_start) {
                return Err(VmmError("died after start".into()));
            }
            Ok(())
        }

        fn stop(&mut self, id: CompartmentId) -> Result<(), VmmError> {
            self.stops += 1;
            if std::mem::take(&mut self.fail_next_stop) {
                return Err(VmmError("stop refused".into()));
            }
            self.running.remove(&id);
            Ok(())
        }

        fn running(&self) -> Result<Vec<CompartmentId>, VmmError> {
            Ok(self.running.iter().copied().collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CANONICAL_HARDWARE, CompartmentId, Network, VmSpec};

    #[test]
    fn only_a_gate_reaches_the_host_uplink() {
        assert_eq!(
            VmSpec::gate(CompartmentId(0), "gate").network(),
            Network::HostUplink
        );
        assert_eq!(
            VmSpec::web(CompartmentId(1), "web").network(),
            Network::GateOnly
        );
    }

    #[test]
    fn every_web_compartment_sees_the_same_hardware() {
        let first = VmSpec::web(CompartmentId(1), "web").hardware();
        let second = VmSpec::web(CompartmentId(2), "other").hardware();
        assert_eq!(first, second);
        assert_eq!(first, CANONICAL_HARDWARE);
    }

    #[test]
    fn canonical_display_is_the_persona_resolution() {
        assert_eq!(CANONICAL_HARDWARE.display_width, 1920);
        assert_eq!(CANONICAL_HARDWARE.display_height, 1080);
    }
}
