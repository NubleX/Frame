//! Compartment lifecycle, owned by the broker.
//!
//! Every operation states a desired end and then converges on it, so running
//! one twice is harmless. Intent is written to the [`Table`] before the VMM is
//! touched, which lets [`Manager::reconcile`] finish whatever a crash
//! interrupted. The VMM's own list is the truth about what is running.

use frame_vmm::{CompartmentId, VmSpec, Vmm, VmmError};
use std::collections::BTreeMap;
use std::fmt;

/// Where a compartment should end up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desired {
    /// It should be running.
    Running,
    /// It should not exist.
    Absent,
}

/// How far a compartment has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Intent recorded; not confirmed.
    Transitioning,
    /// Confirmed at its desired end.
    Settled,
}

/// One row of the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// What the compartment is.
    pub spec: VmSpec,
    /// Where it should end up.
    pub desired: Desired,
    /// How far it has got.
    pub phase: Phase,
}

/// The compartment table. Cloning it stands in for reading it back from
/// durable storage, which comes with the broker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    entries: BTreeMap<CompartmentId, Entry>,
}

impl Table {
    /// Looks up one compartment.
    #[must_use]
    pub fn get(&self, id: CompartmentId) -> Option<&Entry> {
        self.entries.get(&id)
    }

    /// Number of compartments tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A lifecycle operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleError {
    /// The VMM failed. [`Manager::reconcile`] can retry.
    Vmm(VmmError),
    /// A compartment with this id exists with a different spec.
    SpecConflict(CompartmentId),
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vmm(error) => write!(f, "{error}"),
            Self::SpecConflict(id) => write!(f, "compartment {} has another spec", id.0),
        }
    }
}

impl std::error::Error for LifecycleError {}

impl From<VmmError> for LifecycleError {
    fn from(error: VmmError) -> Self {
        Self::Vmm(error)
    }
}

/// Drives compartments towards their desired state.
#[derive(Debug)]
pub struct Manager<V: Vmm> {
    table: Table,
    vmm: V,
}

impl<V: Vmm> Manager<V> {
    /// Resumes from a table. Call [`Manager::reconcile`] next.
    pub fn new(table: Table, vmm: V) -> Self {
        Self { table, vmm }
    }

    /// The current table.
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// The VMM in use.
    pub fn vmm(&self) -> &V {
        &self.vmm
    }

    /// Hands back the VMM, as a crash would leave it behind.
    pub fn into_vmm(self) -> V {
        self.vmm
    }

    /// Makes sure a compartment is running.
    ///
    /// # Errors
    /// [`LifecycleError::SpecConflict`] if the id is taken by a different
    /// spec, or [`LifecycleError::Vmm`] if the VMM failed.
    pub fn ensure_running(&mut self, spec: VmSpec) -> Result<(), LifecycleError> {
        let id = spec.id();
        if let Some(entry) = self.table.entries.get(&id) {
            if entry.spec != spec {
                return Err(LifecycleError::SpecConflict(id));
            }
            if entry.desired == Desired::Running && entry.phase == Phase::Settled {
                let running = self.vmm.running()?;
                if running.contains(&id) {
                    return Ok(());
                }
            }
        }
        self.table.entries.insert(
            id,
            Entry {
                spec,
                desired: Desired::Running,
                phase: Phase::Transitioning,
            },
        );
        self.converge(id)
    }

    /// Makes sure a compartment does not exist. Unknown ids are fine.
    ///
    /// # Errors
    /// [`LifecycleError::Vmm`] if the VMM failed.
    pub fn ensure_absent(&mut self, id: CompartmentId) -> Result<(), LifecycleError> {
        match self.table.entries.get_mut(&id) {
            Some(entry) => {
                entry.desired = Desired::Absent;
                entry.phase = Phase::Transitioning;
                self.converge(id)
            }
            None => {
                if self.vmm.running()?.contains(&id) {
                    self.vmm.stop(id)?;
                }
                Ok(())
            }
        }
    }

    /// Finishes interrupted work and stops any microVM nothing claims. Tries
    /// every compartment even if one fails.
    ///
    /// # Errors
    /// The first [`LifecycleError`] met, after all were attempted.
    pub fn reconcile(&mut self) -> Result<(), LifecycleError> {
        let mut first_error = None;
        let ids: Vec<CompartmentId> = self.table.entries.keys().copied().collect();
        for id in ids {
            if let Err(error) = self.converge(id) {
                first_error.get_or_insert(error);
            }
        }
        for id in self.vmm.running()? {
            if !self.table.entries.contains_key(&id) {
                if let Err(error) = self.vmm.stop(id) {
                    first_error.get_or_insert(error.into());
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn converge(&mut self, id: CompartmentId) -> Result<(), LifecycleError> {
        let running = self.vmm.running()?.contains(&id);
        let Some(entry) = self.table.entries.get_mut(&id) else {
            return Ok(());
        };
        match entry.desired {
            Desired::Running => {
                if !running {
                    entry.phase = Phase::Transitioning;
                    self.vmm.start(&entry.spec)?;
                }
                entry.phase = Phase::Settled;
            }
            Desired::Absent => {
                if running {
                    self.vmm.stop(id)?;
                }
                self.table.entries.remove(&id);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Desired, LifecycleError, Manager, Phase, Table};
    use frame_vmm::fake::FakeVmm;
    use frame_vmm::{CompartmentId, VmSpec, Vmm, VmmError};

    const ID: CompartmentId = CompartmentId(7);

    fn spec() -> VmSpec {
        VmSpec::web(ID, "web-image")
    }

    fn manager() -> Manager<FakeVmm> {
        Manager::new(Table::default(), FakeVmm::default())
    }

    fn is_running(manager: &Manager<FakeVmm>) -> Result<bool, VmmError> {
        Ok(manager.vmm().running()?.contains(&ID))
    }

    #[test]
    fn repeated_create_starts_once() -> Result<(), LifecycleError> {
        let mut manager = manager();
        manager.ensure_running(spec())?;
        manager.ensure_running(spec())?;
        manager.ensure_running(spec())?;
        assert_eq!(manager.vmm().starts, 1);
        assert!(is_running(&manager)?);
        Ok(())
    }

    #[test]
    fn repeated_destroy_converges() -> Result<(), LifecycleError> {
        let mut manager = manager();
        manager.ensure_running(spec())?;
        manager.ensure_absent(ID)?;
        manager.ensure_absent(ID)?;
        manager.ensure_absent(ID)?;
        assert_eq!(manager.vmm().stops, 1);
        assert!(!is_running(&manager)?);
        assert!(manager.table().is_empty());
        Ok(())
    }

    #[test]
    fn destroying_an_unknown_compartment_is_fine() {
        assert_eq!(manager().ensure_absent(ID), Ok(()));
    }

    #[test]
    fn ensure_running_restarts_a_compartment_that_died() -> Result<(), LifecycleError> {
        let mut manager = manager();
        manager.ensure_running(spec())?;
        manager.vmm.stop(ID)?;
        manager.ensure_running(spec())?;
        assert!(is_running(&manager)?);
        assert_eq!(manager.vmm().starts, 2);
        Ok(())
    }

    #[test]
    fn same_id_with_another_spec_is_refused() -> Result<(), LifecycleError> {
        let mut manager = manager();
        manager.ensure_running(spec())?;
        assert_eq!(
            manager.ensure_running(VmSpec::web(ID, "other-image")),
            Err(LifecycleError::SpecConflict(ID))
        );
        Ok(())
    }

    #[test]
    fn crash_after_the_vm_booted_recovers_without_a_second_boot() -> Result<(), LifecycleError> {
        let mut before = manager();
        before.vmm.die_after_next_start = true;
        assert!(before.ensure_running(spec()).is_err());
        let saved = before.table().clone();
        assert_eq!(
            saved.get(ID).map(|entry| entry.phase),
            Some(Phase::Transitioning)
        );

        let mut after = Manager::new(saved, before.into_vmm());
        after.reconcile()?;

        assert_eq!(after.vmm().starts, 1);
        assert_eq!(
            after.table().get(ID).map(|entry| entry.phase),
            Some(Phase::Settled)
        );
        Ok(())
    }

    #[test]
    fn crash_before_the_vm_booted_recovers_by_starting_it() -> Result<(), LifecycleError> {
        let mut before = manager();
        before.vmm.fail_next_start = true;
        assert!(before.ensure_running(spec()).is_err());
        let saved = before.table().clone();

        let mut after = Manager::new(saved, before.into_vmm());
        after.reconcile()?;

        assert!(is_running(&after)?);
        assert_eq!(after.vmm().starts, 2);
        Ok(())
    }

    #[test]
    fn crash_during_destroy_finishes_the_destroy() -> Result<(), LifecycleError> {
        let mut before = manager();
        before.ensure_running(spec())?;
        before.vmm.fail_next_stop = true;
        assert!(before.ensure_absent(ID).is_err());
        let saved = before.table().clone();
        assert_eq!(
            saved.get(ID).map(|entry| entry.desired),
            Some(Desired::Absent)
        );

        let mut after = Manager::new(saved, before.into_vmm());
        after.reconcile()?;

        assert!(!is_running(&after)?);
        assert!(after.table().is_empty());
        Ok(())
    }

    #[test]
    fn reconcile_stops_a_vm_nothing_claims() -> Result<(), LifecycleError> {
        let mut vmm = FakeVmm::default();
        vmm.seed_running(ID);
        let mut manager = Manager::new(Table::default(), vmm);
        manager.reconcile()?;
        assert!(!is_running(&manager)?);
        Ok(())
    }

    #[test]
    fn reconcile_twice_changes_nothing() -> Result<(), LifecycleError> {
        let mut manager = manager();
        manager.ensure_running(spec())?;
        let before = manager.table().clone();
        manager.reconcile()?;
        manager.reconcile()?;
        assert_eq!(manager.table(), &before);
        assert_eq!(manager.vmm().starts, 1);
        Ok(())
    }

    #[test]
    fn reconcile_reports_failure_but_tries_the_rest() -> Result<(), LifecycleError> {
        let other = CompartmentId(8);
        let mut before = manager();
        before.vmm.fail_next_start = true;
        assert!(before.ensure_running(spec()).is_err());
        before.ensure_running(VmSpec::web(other, "web-image"))?;
        before.vmm.stop(other)?;
        before.vmm.fail_next_start = true;

        assert!(before.reconcile().is_err());

        assert!(before.vmm().running()?.contains(&other));
        Ok(())
    }
}
