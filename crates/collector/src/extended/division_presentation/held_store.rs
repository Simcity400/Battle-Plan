//! One hourly store loan spans acquisition and FIFO owned settlement.
use crate::acquisition::callback_parks::{self, Site, lock_callback};
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
    loaned: AtomicBool,
    priority: std::sync::OnceLock<Arc<crate::frame_priority::Pipeline>>,
    #[cfg(test)]
    before_wait: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    after_return: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
}
impl Slot {
    pub(crate) fn new(store: Box<ArmyStatStore>) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(Some(store)),
            returned: Condvar::new(),
            daily_refresh: AtomicBool::new(true),
            loaned: AtomicBool::new(false),
            priority: std::sync::OnceLock::new(),
            #[cfg(test)]
            before_wait: std::sync::OnceLock::new(),
            #[cfg(test)]
            after_return: std::sync::OnceLock::new(),
        })
    }
    pub(crate) fn set_priority(&self, priority: Arc<crate::frame_priority::Pipeline>) {
        assert!(self.priority.set(priority).is_ok());
    }
    #[cfg(test)]
    pub(crate) fn set_wait_hook(&self, hook: impl Fn() + Send + Sync + 'static) {
        assert!(self.before_wait.set(Box::new(hook)).is_ok());
    }
    #[cfg(test)]
    fn set_return_hook(&self, hook: impl Fn() + Send + Sync + 'static) {
        assert!(self.after_return.set(Box::new(hook)).is_ok());
    }
    pub(crate) fn prepare_hour(&self, hour: i32, force_full: bool) {
        if self.loaned.load(Ordering::Acquire) {
            if let Some(priority) = self.priority.get() {
                priority.expect_store(&self.loaned);
            }
        }
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
        let mut store = lock_callback(&self.store, Site::StoreMutex);
        let waiting = store
            .is_none()
            .then(|| {
                self.priority
                    .get()
                    .map(|priority| priority.wait_for_store())
            })
            .flatten();
        while store.is_none() {
            if let Some(waiting) = &waiting {
                waiting.retry();
            }
            if crate::allocation::failed() || crate::decode::panic_failure().is_some() {
                return None;
            }
            #[cfg(test)]
            if let Some(hook) = self.before_wait.get() {
                hook();
            }
            let park = callback_parks::Park::new(Site::StoreReturn);
            store = self
                .returned
                .wait_timeout(store, std::time::Duration::from_millis(10))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
            drop(park);
        }
        let mut held = store.take();
        self.loaned.store(true, Ordering::SeqCst);
        held.as_deref_mut()
            .unwrap()
            .daily_progress(self.daily_refresh.swap(false, Ordering::AcqRel));
        drop(store);
        if let Some(priority) = self.priority.get() {
            priority.store_returned();
        }
        drop(waiting);
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
        let mut slot = lock_callback(&self.owner.store, Site::StoreMutex);
        assert!(slot.is_none(), "one presentation store loan");
        *slot = self.store.take();
        self.owner.loaned.store(false, Ordering::SeqCst);
        // Cleared while this loan still owns the slot, so a later loan's hint survives a delayed return.
        let priority = self.owner.priority.get();
        if let Some(priority) = priority {
            priority.clear_store_expected();
        }
        self.owner.returned.notify_one();
        drop(slot);
        #[cfg(test)]
        if let Some(hook) = self.owner.after_return.get() {
            hook();
        }
        if let Some(priority) = priority {
            priority.refresh(crate::frame_priority::Caller::Worker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outstanding_store_promotes_before_dispatch_and_return_clears_the_expectation() {
        let priority = Arc::new(crate::frame_priority::Pipeline::new());
        for index in 0..crate::sampling::BUFFERS {
            priority.publish(index);
        }
        let _callback = priority.callback();
        let slot = Slot::new(held());
        slot.set_priority(Arc::clone(&priority));
        let loan = slot.take().unwrap();
        slot.prepare_hour(12, false);
        let pending = serde_json::to_value(priority.status(1_000_000_000)).unwrap();
        assert_eq!(pending["requested_priority"], 2);
        assert_eq!(pending["store_expected"], true);
        drop(loan);
        let returned = serde_json::to_value(priority.status(1_000_000_000)).unwrap();
        assert_eq!(returned["requested_priority"], 0);
        assert_eq!(returned["store_expected"], false);
        let _next = slot.take().unwrap();
        assert_eq!(
            serde_json::to_value(priority.status(1_000_000_000)).unwrap()["store_waiters"],
            0
        );
    }
    #[test]
    fn a_loan_returned_between_the_check_and_the_hint_leaves_no_expectation() {
        let priority = Arc::new(crate::frame_priority::Pipeline::new());
        for index in 0..crate::sampling::BUFFERS {
            priority.publish(index);
        }
        let _callback = priority.callback();
        let slot = Slot::new(held());
        slot.set_priority(Arc::clone(&priority));
        let loan = slot.take().unwrap();
        assert!(slot.loaned.load(Ordering::SeqCst));
        // prepare_hour saw the loan outstanding; decode returns it before the hint is published.
        drop(loan);
        priority.expect_store(&slot.loaned);
        let status = serde_json::to_value(priority.status(1_000_000_000)).unwrap();
        assert_eq!(status["store_expected"], false);
        assert_eq!(status["requested_priority"], 0);
    }
    #[test]
    fn a_delayed_return_tail_keeps_the_next_loans_hint() {
        let priority = Arc::new(crate::frame_priority::Pipeline::new());
        for index in 0..crate::sampling::BUFFERS {
            priority.publish(index);
        }
        let _callback = priority.callback();
        let slot = Slot::new(held());
        slot.set_priority(Arc::clone(&priority));
        let next = Arc::new(Mutex::new(None));
        let hook_next = Arc::clone(&next);
        let weak = Arc::downgrade(&slot);
        let fired = AtomicBool::new(false);
        // Runs inside the first loan's drop, after it unlocks and before its refresh.
        slot.set_return_hook(move || {
            if fired.swap(true, Ordering::SeqCst) {
                return;
            }
            let slot = weak.upgrade().unwrap();
            let loan = slot.take().unwrap();
            slot.prepare_hour(13, false);
            *hook_next.lock().unwrap() = Some(loan);
        });
        drop(slot.take().unwrap());
        // Taken out before asserting, so a failure still drops it and breaks the hook's cycle.
        let next = next.lock().unwrap().take();
        assert!(next.is_some());
        let status = serde_json::to_value(priority.status(1_000_000_000)).unwrap();
        assert_eq!(status["store_expected"], true);
        assert_eq!(status["requested_priority"], 2);
    }
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
            assert!(
                older
                    .daily_supply(0, 0, &seed, &mut Default::default())
                    .is_some()
            );
            slot.prepare_hour(hour, full);
            // This request is rejected before take. Its older FIFO decode can
            // still return a successful store, but cannot clear the revocation.
            older.complete();
            drop(older);
            slot.prepare_hour(hour + 1, false);
            let next = slot.take().unwrap();
            assert!(
                next.daily_supply(0, 0, &seed, &mut Default::default())
                    .is_none()
            );
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
