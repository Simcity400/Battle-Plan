//! One hourly store loan spans acquisition and FIFO owned settlement.
use crate::extended::attack_stats::ArmyStatStore;
use std::{
    ops::{Deref, DerefMut},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) struct Slot {
    store: Mutex<Option<Box<ArmyStatStore>>>,
    returned: Condvar,
    daily_refresh: AtomicBool,
}
impl Slot {
    pub(crate) fn new(store: Box<ArmyStatStore>) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(Some(store)),
            returned: Condvar::new(),
            daily_refresh: AtomicBool::new(true),
        })
    }
    pub(crate) fn prepare_hour(&self, hour: i32, force_full: bool) {
        if force_full
            || replay_data::cadence::Cadence::Daily(
                replay_data::cadence::DIVISION_STATUS_DAILY_SLOT,
            )
            .is_refresh(hour)
        {
            // The preceding FIFO decode may still own the store. Only the
            // next take consumes this revocation, even if dispatch is skipped.
            self.daily_refresh.store(true, Ordering::Release);
        }
    }
    pub(crate) fn take(self: &Arc<Self>) -> Option<Loan> {
        let mut store = self
            .store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while store.is_none() {
            if crate::allocation::failed() || crate::decode::panic_failure().is_some() {
                return None;
            }
            store = self
                .returned
                .wait_timeout(store, std::time::Duration::from_millis(10))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        let mut held = store.take();
        held.as_deref_mut()
            .unwrap()
            .daily_progress(self.daily_refresh.swap(false, Ordering::AcqRel));
        Some(Loan {
            store: held,
            owner: Arc::clone(self),
            complete: false,
        })
    }
}
pub(crate) struct Loan {
    store: Option<Box<ArmyStatStore>>,
    owner: Arc<Slot>,
    complete: bool,
}
impl Loan {
    pub(crate) fn complete(&mut self) {
        self.complete = true;
    }
    pub(crate) fn prepare_return(&mut self) {
        if !self.complete {
            self.forget();
            self.complete = true;
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture(store: Box<ArmyStatStore>) -> Self {
        Slot::new(store).take().unwrap()
    }
}
impl Deref for Loan {
    type Target = ArmyStatStore;
    fn deref(&self) -> &Self::Target {
        self.store.as_deref().unwrap()
    }
}
impl DerefMut for Loan {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.store.as_deref_mut().unwrap()
    }
}
impl Drop for Loan {
    fn drop(&mut self) {
        // Aborted acquisition, dropped frames and decode unwind cannot carry a partial baseline.
        self.prepare_return();
        let mut slot = self
            .owner
            .store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(slot.is_none(), "one presentation store loan");
        *slot = self.store.take();
        self.owner.returned.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn daily_and_full_revocation_survive_an_older_decode_and_skipped_dispatch() {
        use collector_division_parts::supply_inputs::Sample;
        use engine_native::carmy::{self, DivisionSeed};
        let seed = DivisionSeed {
            pointer: 0x1000,
            division: 7,
            handle: Some(7),
            class: Some(0),
            primary_vtable: Some(carmy::VTABLE_RVA as u64),
            secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA as u64),
            ..Default::default()
        };
        for (hour, full) in [(12, false), (13, true)] {
            let slot = Slot::new(Box::new(ArmyStatStore::new()));
            let mut older = slot.take().unwrap();
            let sample = Sample::owned(None, None, None).unwrap();
            older.remember_supply(0, &[seed], &[Some(sample)], true);
            assert!(older.daily_supply(0, 0, &seed, &mut Default::default()).is_some());
            slot.prepare_hour(hour, full);
            // This request is rejected before take. Its older FIFO decode can
            // still return a successful store, but cannot clear the revocation.
            older.complete();
            drop(older);
            slot.prepare_hour(hour + 1, false);
            let next = slot.take().unwrap();
            assert!(next.daily_supply(0, 0, &seed, &mut Default::default()).is_none());
        }
    }
    fn held() -> Box<ArmyStatStore> {
        let mut store = Box::new(ArmyStatStore::new());
        store.begin_names();
        store.keep_name(
            7,
            8,
            &[0; engine_native::cname_group_member::INPUT_SPAN],
            b"held name",
        );
        store.hold(Some(1));
        store
    }
    #[test]
    fn complete_loan_returns_held_state_but_lost_or_unwound_loan_forgets_it() {
        for complete in [false, true] {
            let slot = Slot::new(held());
            let mut loan = slot.take().unwrap();
            assert!(slot.store.lock().unwrap().is_none());
            assert!(loan.held_name(0, 7).is_some());
            if complete {
                loan.complete();
            }
            drop(loan);
            let mut next = slot.take().unwrap();
            assert_eq!(next.held_name(0, 7).is_some(), complete);
        }
        let slot = Slot::new(held());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _loan = slot.take().unwrap();
            panic!("synthetic owned settlement unwind");
        }));
        assert!(result.is_err());
        assert!(slot.take().unwrap().held_name(0, 7).is_none());
    }
}
