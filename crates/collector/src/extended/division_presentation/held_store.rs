//! One hourly store loan spans acquisition and FIFO owned settlement. An hour never waits for
//! the store the hour before still settles: it takes a store of its own, which holds nothing
//! and so reads every division, as the attach hour does. Only the store of the latest take
//! carries into the next hour; an older one returns late, forgets what it held and is kept for
//! the next backlog, and once the latest returns the kept ones are freed.
use crate::acquisition::lock_unpoisoned;
use crate::extended::attack_stats::ArmyStatStore;
use std::{
    ops::{Deref, DerefMut},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

struct State {
    /// The latest take's store, once it returned.
    store: Option<Box<ArmyStatStore>>,
    /// The latest take.
    latest: u64,
    /// Stores that returned after a later take, forgotten, for the next take that finds the
    /// latest one still out.
    spares: Vec<Box<ArmyStatStore>>,
}

struct Shared {
    state: Mutex<State>,
    daily_refresh: AtomicBool,
    /// The spares' bytes, which the pool's total counts.
    spare_bytes: AtomicU64,
}

/// A handle on the one store slot; its clones share it.
#[derive(Clone)]
pub(crate) struct Slot(engine_allocation::SharedBox<Shared>);

impl Slot {
    /// A refused allocation is the collector's allocation failure.
    pub(crate) fn new(store: Box<ArmyStatStore>) -> std::io::Result<Self> {
        engine_allocation::shared(Shared {
            state: Mutex::new(State {
                store: Some(store),
                latest: 0,
                spares: Vec::new(),
            }),
            daily_refresh: AtomicBool::new(true),
            spare_bytes: AtomicU64::new(0),
        })
        .map(Self)
    }
    #[cfg(test)]
    pub(crate) fn owners(&self) -> usize {
        engine_allocation::SharedBox::strong_count(&self.0)
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
            self.0.daily_refresh.store(true, Ordering::Release);
        }
    }
    /// The hour's store, at once: the latest take's when it has returned, else one of its
    /// own, which samples the daily operands again. None only when a new store cannot be
    /// allocated, which is the collector's allocation failure.
    pub(crate) fn take(&self) -> Option<Loan> {
        let mut state = lock_unpoisoned(&self.0.state);
        state.latest += 1;
        let id = state.latest;
        let returned = state.store.take();
        let fresh = returned.is_none();
        let spare = if fresh { state.spares.pop() } else { None };
        drop(state);
        if let Some(spare) = &spare {
            self.0.spare_bytes
                .fetch_sub(spare.allocated_bytes() as u64, Ordering::Relaxed);
        }
        let mut held = match returned.or(spare) {
            Some(store) => store,
            None => engine_allocation::boxed(ArmyStatStore::new()).ok()?,
        };
        held.daily_progress(self.0.daily_refresh.swap(false, Ordering::AcqRel) || fresh);
        Some(Loan {
            store: Some(held),
            owner: self.clone(),
            complete: false,
            id,
        })
    }
    pub(crate) fn spare_bytes(&self) -> u64 {
        self.0.spare_bytes.load(Ordering::Relaxed)
    }
    /// Takes back a loan's store: the latest take's carries into the next hour and frees the
    /// spares; an older one forgets what it held and becomes a spare.
    fn give_back(&self, id: u64, mut store: Box<ArmyStatStore>) {
        let mut state = lock_unpoisoned(&self.0.state);
        if id == state.latest {
            debug_assert!(state.store.is_none(), "one latest presentation store");
            state.store = Some(store);
            let spares = std::mem::take(&mut state.spares);
            drop(state);
            self.0.spare_bytes.store(0, Ordering::Relaxed);
            drop(spares);
            return;
        }
        store.forget();
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut state.spares, 1)).is_ok() {
            self.0.spare_bytes
                .fetch_add(store.allocated_bytes() as u64, Ordering::Relaxed);
            state.spares.push(store);
        }
    }
}
pub(crate) struct Loan {
    store: Option<Box<ArmyStatStore>>,
    owner: Slot,
    complete: bool,
    id: u64,
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
        Slot::new(store).unwrap().take().unwrap()
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
        if let Some(store) = self.store.take() {
            self.owner.give_back(self.id, store);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_take_while_the_latest_store_is_out_gets_its_own_and_only_the_latest_carries() {
        let slot = Slot::new(held()).unwrap();
        let first = slot.take().unwrap();
        let first_address = &*first as *const ArmyStatStore;
        // The decode worker still holds the first hour's store: the next take does not wait.
        let mut second = slot.take().unwrap();
        assert_ne!(&*second as *const ArmyStatStore, first_address);
        assert!(second.held_name(0, 7).is_none());
        second.complete();
        let second_address = &*second as *const ArmyStatStore;
        // A third take, while both are out, gets another of its own.
        let third = slot.take().unwrap();
        drop(first);
        assert_eq!(slot.0.state.lock().unwrap().spares.len(), 1);
        drop(second);
        // An older store returning after a later take carries nothing on.
        assert!(slot.0.state.lock().unwrap().store.is_none());
        let third_address = &*third as *const ArmyStatStore;
        drop(third);
        assert_eq!(slot.spare_bytes(), 0);
        let state = slot.0.state.lock().unwrap();
        assert!(state.spares.is_empty());
        assert_eq!(state.store.as_deref().unwrap() as *const ArmyStatStore, third_address);
        drop(state);
        assert_ne!(third_address, second_address);
        // A spare is reused, forgotten, by the next take that finds the latest one out.
        let out = slot.take().unwrap();
        let older = slot.take().unwrap();
        drop(out);
        let mut reused = slot.take().unwrap();
        assert!(reused.held_name(0, 7).is_none());
        drop((older, reused));
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
            primary_vtable: Some(carmy::VTABLE_RVA.0 as u64),
            secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA.0 as u64),
            ..Default::default()
        };
        let daily = replay_data::cadence::DIVISION_STATUS_DAILY_SLOT as i32;
        for (hour, full) in [(daily, false), (daily + 1, true)] {
            let slot = Slot::new(Box::new(ArmyStatStore::new())).unwrap();
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
            let slot = Slot::new(held()).unwrap();
            let mut loan = slot.take().unwrap();
            assert!(slot.0.state.lock().unwrap().store.is_none());
            assert!(loan.held_name(0, 7).is_some());
            if complete {
                loan.complete();
            }
            drop(loan);
            let mut next = slot.take().unwrap();
            assert_eq!(next.held_name(0, 7).is_some(), complete);
        }
        let slot = Slot::new(held()).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _loan = slot.take().unwrap();
            panic!("synthetic owned settlement unwind");
        }));
        assert!(result.is_err());
        assert!(slot.take().unwrap().held_name(0, 7).is_none());
    }
}
