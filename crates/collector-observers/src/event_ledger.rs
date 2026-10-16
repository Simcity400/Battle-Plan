//! Shared loss semantics for the two air ledgers. Only the game thread touches
//! the store. The spool thread publishes an empty spare through a separate slot.
type Clock = engine::cgame_state::Clock<u64>;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

pub trait Event: Copy {
    fn set_id(&mut self, id: u64);
}

struct Claim {
    token: usize,
    key: (usize, usize),
}
struct Store<T> {
    entries: Vec<T>,
    // Nested/concurrent native callbacks invalidate observation before claiming.
    claim: Option<Claim>,
    next_call: usize,
    next_id: u64,
    clock: Option<Clock>,
    world_generation: u64,
    partial: bool,
    recovery: bool,
    refusal: Option<&'static str>,
}
impl<T> Store<T> {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
            claim: None,
            next_call: 1,
            next_id: 0,
            clock: None,
            world_generation: 0,
            partial: true,
            recovery: false,
            refusal: None,
        }
    }
}

pub struct EventLedger<T> {
    store: Mutex<Store<T>>,
    spare: Mutex<Option<Vec<T>>>,
    retired: Mutex<Option<Vec<T>>>,
    pub poisoned: AtomicBool,
    pub stranded: AtomicBool,
    pub active: AtomicUsize,
    // Published census/overflow demand, never pooled Vec allocation headroom.
    pub capacity: AtomicUsize,
    required: AtomicUsize,
    attempted_calls: AtomicUsize,
    attempted_rows: AtomicUsize,
    pub calls: AtomicU64,
    pub events: AtomicU64,
    pub zero_writes: AtomicU64,
    pub overflow: AtomicU64,
    pub refusals: AtomicU64,
    pub lost_drains: AtomicU64,
    pub lost_events: AtomicU64,
    pub lost_events_unknown: AtomicU64,
    pub partial_intervals: AtomicU64,
    pub partial_events: AtomicU64,
    pub drains: AtomicU64,
    pub handed_over: AtomicU64,
    pub overlaps: AtomicU64,
    pub prepare_failures: AtomicU64,
    prepare_last_refusal: Mutex<Option<&'static str>>,
    pub hourly_peak_events: AtomicUsize,
    pub hourly_peak_capacity: AtomicUsize,
}
impl<T: Event> Default for EventLedger<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T: Event> EventLedger<T> {
    pub const fn new() -> Self {
        Self {
            store: Mutex::new(Store::new()),
            spare: Mutex::new(None),
            retired: Mutex::new(None),
            poisoned: AtomicBool::new(false),
            stranded: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            capacity: AtomicUsize::new(0),
            required: AtomicUsize::new(0),
            attempted_calls: AtomicUsize::new(0),
            attempted_rows: AtomicUsize::new(0),
            calls: AtomicU64::new(0),
            events: AtomicU64::new(0),
            zero_writes: AtomicU64::new(0),
            overflow: AtomicU64::new(0),
            refusals: AtomicU64::new(0),
            lost_drains: AtomicU64::new(0),
            lost_events: AtomicU64::new(0),
            lost_events_unknown: AtomicU64::new(0),
            partial_intervals: AtomicU64::new(0),
            partial_events: AtomicU64::new(0),
            drains: AtomicU64::new(0),
            handed_over: AtomicU64::new(0),
            overlaps: AtomicU64::new(0),
            prepare_failures: AtomicU64::new(0),
            prepare_last_refusal: Mutex::new(None),
            hourly_peak_events: AtomicUsize::new(0),
            hourly_peak_capacity: AtomicUsize::new(0),
        }
    }
    /// Bootstrap only, before hook publication, from the retained replay leases.
    pub fn restore_sequence(&self, previous: Option<u64>) -> Result<(), &'static str> {
        if self.active.load(Ordering::SeqCst) != 0 {
            return Err("event ledger sequence restoration requires quiet callbacks");
        }
        let next = match previous {
            Some(previous) => previous
                .checked_add(1)
                .ok_or("event ledger event identity exhausted")?,
            None => 0,
        };
        let mut store = self
            .store
            .lock()
            .map_err(|_| "event ledger sequence storage poisoned")?;
        store.next_id = store.next_id.max(next);
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn reset(&self) {
        *self.store.lock().unwrap() = Store::new();
        *self.spare.lock().unwrap() = None;
        *self.retired.lock().unwrap() = None;
        *self.prepare_last_refusal.lock().unwrap() = None;
        self.poisoned.store(false, Ordering::SeqCst);
        self.stranded.store(false, Ordering::SeqCst);
        self.active.store(0, Ordering::SeqCst);
        self.capacity.store(0, Ordering::Release);
        self.required.store(0, Ordering::Release);
        self.attempted_calls.store(0, Ordering::Relaxed);
        self.attempted_rows.store(0, Ordering::Relaxed);
        for count in [
            &self.calls,
            &self.events,
            &self.zero_writes,
            &self.overflow,
            &self.refusals,
            &self.lost_drains,
            &self.lost_events,
            &self.lost_events_unknown,
            &self.partial_intervals,
            &self.partial_events,
            &self.drains,
            &self.handed_over,
            &self.overlaps,
            &self.prepare_failures,
        ] {
            count.store(0, Ordering::Relaxed);
        }
        self.hourly_peak_events.store(0, Ordering::Relaxed);
        self.hourly_peak_capacity.store(0, Ordering::Relaxed);
    }
    #[cfg(all(test, feature = "tap-foreign-probe"))]
    pub fn foreign_probe_state(&self) -> (Option<(usize, usize)>, usize, usize) {
        let store = self.store.lock().unwrap();
        (
            store.claim.as_ref().map(|claim| claim.key),
            store.entries.as_ptr() as usize,
            store.entries.capacity(),
        )
    }
    pub fn demand(&self) -> usize {
        self.capacity
            .load(Ordering::Acquire)
            .max(self.required.load(Ordering::Acquire))
    }
    pub fn refuse_preparation(&self, reason: &'static str) {
        *self
            .prepare_last_refusal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
        self.prepare_failures.fetch_add(1, Ordering::Relaxed);
    }
    pub fn prepare_last_refusal(&self) -> Option<&'static str> {
        *self
            .prepare_last_refusal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn prepare(&self, demand: usize) {
        engine::allocation::advisory(|| self.prepare_storage(demand));
    }

    fn prepare_storage(&self, demand: usize) {
        if let Ok(mut retired) = self.retired.lock() {
            drop(retired.take());
        }
        let demand = demand.max(self.required.load(Ordering::Acquire));
        if demand <= self.capacity.load(Ordering::Acquire) {
            return;
        }
        let mut spare = Vec::new();
        if engine::allocation::reserve_exact(&mut spare, demand).is_err() {
            self.refuse_preparation("event ledger preparation allocation failed");
            return;
        }
        // No writer ever locks this slot; its short install runs at drain.
        if let Ok(mut slot) = self.spare.lock() {
            if slot
                .as_ref()
                .is_none_or(|old| old.capacity() < spare.capacity())
            {
                *slot = Some(spare);
            }
            self.capacity.fetch_max(demand, Ordering::Release);
        }
    }
    pub fn enter(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.active.fetch_add(1, Ordering::SeqCst) != 0 {
            self.overlaps.fetch_add(1, Ordering::Relaxed);
            self.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
            self.refuse("event ledger native calls overlap");
        }
    }
    /// Reached native writer, including calls skipped after an interval loss.
    /// This sizes the next prepared store without acquiring refused operands.
    pub fn attempt(&self) {
        let attempted = self
            .attempted_calls
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        crate::output_log::raise(&self.required, attempted, Ordering::Release);
    }
    pub fn leave(&self) {
        if self.active.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.stranded.store(false, Ordering::SeqCst);
        }
    }
    pub fn mark_stranded(&self) {
        // Called only at a proved serial boundary/detach after native work.
        if self.active.load(Ordering::SeqCst) != 0 {
            self.stranded.store(true, Ordering::SeqCst);
        }
    }
    fn discard(&self, store: &mut Store<T>) {
        self.lost_events
            .fetch_add(store.entries.len() as u64, Ordering::Relaxed);
        store.entries.clear();
    }
    pub fn refuse(&self, reason: &'static str) {
        self.invalidate();
        if let Ok(mut store) = self.store.try_lock() {
            store.refusal.get_or_insert(reason);
            self.discard(&mut store);
        }
    }
    // Spool-thread loss paths must never contend with native writers.
    fn invalidate(&self) {
        self.refusals.fetch_add(1, Ordering::Relaxed);
        self.poisoned.store(true, Ordering::SeqCst);
    }
    pub fn lost_drain(&self) {
        self.lost_drains.fetch_add(1, Ordering::Relaxed);
        self.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
        self.invalidate();
    }
    pub fn lost_output(&self, events: usize) {
        self.lost_events.fetch_add(events as u64, Ordering::Relaxed);
        self.lost_drains.fetch_add(1, Ordering::Relaxed);
        self.invalidate();
    }
    pub fn claim(&self, key: (usize, usize)) -> Result<usize, &'static str> {
        let mut store = self.store.try_lock().map_err(|_| {
            self.overlaps.fetch_add(1, Ordering::Relaxed);
            "event ledger concurrent writer"
        })?;
        if let Some(claim) = &store.claim {
            self.overlaps.fetch_add(1, Ordering::Relaxed);
            return Err(if claim.key == key {
                "event ledger writers overlap the same native record"
            } else {
                "event ledger native calls overlap"
            });
        }
        let token = store.next_call;
        store.next_call = token
            .checked_add(1)
            .ok_or("event ledger claim identity overflow")?;
        store.claim = Some(Claim { token, key });
        Ok(token)
    }
    pub fn complete(
        &self,
        token: usize,
        event: Result<Option<T>, &'static str>,
    ) -> Result<(), &'static str> {
        let mut store = self.store.try_lock().map_err(|_| {
            self.overlaps.fetch_add(1, Ordering::Relaxed);
            "event ledger concurrent post writer"
        })?;
        if store
            .claim
            .as_ref()
            .is_none_or(|claim| claim.token != token)
        {
            return Err("event ledger pending claim missing");
        }
        store.claim = None;
        let event = event?;
        if event.is_some() {
            let attempted = self
                .attempted_rows
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1);
            crate::output_log::raise(&self.required, attempted, Ordering::Release);
        }
        if self.poisoned.load(Ordering::SeqCst) {
            return Err("event ledger observation invalidated");
        }
        let Some(mut event) = event else {
            self.zero_writes.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        };
        if store.entries.len() == store.entries.capacity() {
            self.overflow.fetch_add(1, Ordering::Relaxed);
            self.required.fetch_max(
                self.capacity
                    .load(Ordering::Acquire)
                    .saturating_mul(2)
                    .max(store.entries.len().saturating_add(1)),
                Ordering::Release,
            );
            return Err("event ledger event storage exhausted");
        }
        event.set_id(store.next_id);
        store.next_id = store
            .next_id
            .checked_add(1)
            .ok_or("event ledger event identity overflow")?;
        store.entries.push(event);
        self.events.fetch_add(1, Ordering::Relaxed);
        let len = store.entries.len();
        if len > self.hourly_peak_events.load(Ordering::Relaxed) {
            self.hourly_peak_events.store(len, Ordering::Relaxed);
            self.hourly_peak_capacity
                .store(store.entries.capacity(), Ordering::Relaxed);
        }
        Ok(())
    }
    pub fn boundary(&self, clock: Option<Clock>) {
        let Some(clock) = clock else {
            self.lost_drain();
            return;
        };
        if self.active.load(Ordering::SeqCst) != 0 {
            self.mark_stranded();
            self.lost_drain();
            return;
        }
        let generation = self.calls.load(Ordering::SeqCst);
        let Ok(mut store) = self.store.try_lock() else {
            self.lost_drain();
            return;
        };
        self.attempted_calls.store(0, Ordering::Relaxed);
        self.attempted_rows.store(0, Ordering::Relaxed);
        let transition = store.world_generation != crate::world_lifetime::generation()
            || !store.clock.is_some_and(|before| {
                before.hour.checked_add(1) == Some(clock.hour)
                    && before.campaign_id == clock.campaign_id
                    && before.play_session_started_at == clock.play_session_started_at
            });
        store.clock = Some(clock);
        store.world_generation = crate::world_lifetime::generation();
        if transition {
            self.partial_events
                .fetch_add(store.entries.len() as u64, Ordering::Relaxed);
            store.entries.clear();
            store.claim = None;
            // Private clock recovery can occur within one replay branch.
            // Event ordinals remain monotonic for this entire attachment.
            store.partial = true;
            store.recovery = false;
            store.refusal = None;
            self.poisoned.store(false, Ordering::SeqCst);
        } else if self.poisoned.swap(false, Ordering::SeqCst) {
            // Capacity loss ends at this clean boundary. Other refusals can
            // leave native execution unresolved and retain a recovery interval.
            self.discard(&mut store);
            store.claim = None;
            // Refusal may have lost try_lock while a writer held the store.
            // The atomic invalidation still makes this interval unavailable.
            store.refusal.get_or_insert("event ledger lost interval");
            store.recovery = store.refusal != Some("event ledger event storage exhausted");
        } else {
            store.refusal = None;
            store.partial = std::mem::take(&mut store.recovery);
        }
        if self.active.load(Ordering::SeqCst) != 0
            || self.calls.load(Ordering::SeqCst) != generation
        {
            self.overlaps.fetch_add(1, Ordering::Relaxed);
            self.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
            self.refusals.fetch_add(1, Ordering::Relaxed);
            self.poisoned.store(true, Ordering::SeqCst);
            store
                .refusal
                .get_or_insert("event ledger writer overlaps boundary");
            self.discard(&mut store);
        }
        if store.partial {
            self.partial_intervals.fetch_add(1, Ordering::Relaxed);
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn drain(&self, output: &mut Vec<T>) -> Result<usize, &'static str> {
        self.drain_buffer(output, false)
    }
    /// Worker-only: owned events still covered by this interval size the
    /// returned frame before handoff; no native operation is attempted again.
    pub fn drain_growing(&self, output: &mut Vec<T>) -> Result<usize, &'static str> {
        self.drain_buffer(output, true)
    }
    fn drain_buffer(&self, output: &mut Vec<T>, growing: bool) -> Result<usize, &'static str> {
        self.drains.fetch_add(1, Ordering::Relaxed);
        if self.active.load(Ordering::SeqCst) != 0 {
            self.mark_stranded();
            self.lost_drain();
            return Err("event ledger active or unwound native writer at drain");
        }
        let generation = self.calls.load(Ordering::SeqCst);
        let mut store = self
            .store
            .try_lock()
            .map_err(|_| "event ledger writer overlaps drain")?;
        if let Ok(mut slot) = self.spare.try_lock()
            && let Ok(mut retired) = self.retired.try_lock()
            && retired.is_none()
            && slot.as_ref().is_some_and(|spare| {
                spare.capacity() >= store.entries.len()
                    && spare.capacity() > store.entries.capacity()
            })
        {
            let mut spare = slot.take().unwrap();
            spare.extend_from_slice(&store.entries);
            std::mem::swap(&mut store.entries, &mut spare);
            *retired = Some(spare);
        }
        if let Some(reason) = store.refusal {
            self.discard(&mut store);
            return Err(reason);
        }
        if self.poisoned.load(Ordering::SeqCst) {
            self.discard(&mut store);
            return Err("event ledger lost interval");
        }
        if store.partial {
            self.partial_events
                .fetch_add(store.entries.len() as u64, Ordering::Relaxed);
            store.entries.clear();
            return Err("event ledger first/branch/recovery interval is partial");
        }
        output.clear();
        if growing {
            engine::allocation::reserve_exact(output, store.entries.len())
                .map_err(|_| "event ledger drain allocation failed")?;
        }
        if output.capacity() < store.entries.len() {
            self.refusals.fetch_add(1, Ordering::Relaxed);
            self.discard(&mut store);
            return Err("event ledger drain storage exhausted");
        }
        // Keep the writer capacity when an older pooled frame is smaller.
        if output.capacity() >= store.entries.capacity() {
            std::mem::swap(output, &mut store.entries);
        } else {
            output.extend_from_slice(&store.entries);
            store.entries.clear();
        }
        if self.poisoned.load(Ordering::SeqCst)
            || self.active.load(Ordering::SeqCst) != 0
            || self.calls.load(Ordering::SeqCst) != generation
        {
            self.overlaps.fetch_add(1, Ordering::Relaxed);
            let len = output.len();
            output.clear();
            drop(store);
            self.lost_output(len);
            return Err("event ledger writer overlaps hourly drain");
        }
        self.handed_over
            .fetch_add(output.len() as u64, Ordering::Relaxed);
        Ok(output.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone, Copy, Default)]
    struct Row(u64);
    impl Event for Row {
        fn set_id(&mut self, id: u64) {
            self.0 = id;
        }
    }
    fn prepared() -> EventLedger<Row> {
        let ledger = EventLedger::new();
        ledger.prepare(8);
        assert!(ledger.drain(&mut Vec::new()).is_err());
        ledger
    }
    #[test]
    fn callbacks_never_allocate_and_overflow_grows_only_off_thread() {
        let ledger = EventLedger::<Row>::new();
        ledger.prepare(1);
        ledger.boundary(clock(1));
        assert!(ledger.drain(&mut Vec::new()).is_err());
        ledger.boundary(clock(2));
        let pointer = ledger.store.lock().unwrap().entries.as_ptr();
        let result = crate::test_support::with_allocations_denied(|| {
            let token = ledger.claim((1, 1)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            let token = ledger.claim((2, 2)).unwrap();
            ledger.complete(token, Ok(Some(Row(0))))
        });
        assert_eq!(result.unwrap_err(), "event ledger event storage exhausted");
        assert_eq!(ledger.store.lock().unwrap().entries.as_ptr(), pointer);
        assert_eq!(ledger.capacity.load(Ordering::Acquire), 1);
        assert_eq!(ledger.demand(), 2);
        assert_eq!(ledger.overflow.load(Ordering::Relaxed), 1);
        ledger.refuse("event ledger event storage exhausted");
        ledger.prepare(ledger.demand());
        ledger.boundary(clock(3));
        let mut rows = Vec::with_capacity(2);
        assert!(ledger.drain(&mut rows).is_err());
        ledger.boundary(clock(4));
        assert_eq!(ledger.drain(&mut rows).unwrap(), 0);
        ledger.boundary(clock(5));
        for owner in [1, 2] {
            let token = ledger.claim((owner, 0)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
        }
        assert_eq!(ledger.drain(&mut rows).unwrap(), 2);
    }
    fn clock(hour: i32) -> Option<Clock> {
        Some(Clock {
            hour,
            play_session_started_at: 0,
            game_state: 0,
            play_time: 0,
            seed: 0,
            campaign_id: Default::default(),
            speed: 0,
            tick_busy: 0,
        })
    }

    #[test]
    fn recycled_frame_allocation_never_becomes_event_demand() {
        fn exercise<T: Event + Default>() {
            let ledger = EventLedger::<T>::new();
            let mut frames: [crate::table_capture::CaptureRows<T>; 4] =
                std::array::from_fn(|_| crate::table_capture::CaptureRows::new(8));
            ledger.prepare(8);
            ledger.boundary(clock(1));
            assert!(ledger.drain(&mut frames[0].rows).is_err());
            ledger.prepare(0);

            // A one-entry census increase makes the old frames reserve 16.
            ledger.prepare(9);
            let mut capacity = 9;
            for hour in 2..=10 {
                let frame = &mut frames[(hour as usize - 2) % 4];
                crate::test_support::with_allocations_denied(|| {
                    ledger.boundary(clock(hour));
                    let token = ledger.claim((1, 0)).unwrap();
                    ledger.complete(token, Ok(Some(T::default()))).unwrap();
                    assert_eq!(ledger.drain(&mut frame.rows).unwrap(), 1);
                });
                ledger.prepare(0);
                capacity = capacity.max(ledger.demand());
                for frame in &mut frames {
                    assert!(crate::table_capture::grow_vec(&mut frame.rows, capacity));
                }
                assert_eq!(ledger.demand(), 9);
            }
            assert_eq!(ledger.overflow.load(Ordering::Relaxed), 0);
            assert_eq!(ledger.lost_events.load(Ordering::Relaxed), 0);
            assert_eq!(ledger.hourly_peak_events.load(Ordering::Relaxed), 1);
            assert_eq!(ledger.handed_over.load(Ordering::Relaxed), 9);
        }
        exercise::<crate::air_losses::Entry>();
        exercise::<crate::building_bombing::Entry>();
    }

    #[test]
    fn overflow_after_a_larger_frame_swap_uses_attempted_event_demand() {
        let ledger = EventLedger::<crate::air_losses::Entry>::new();
        let mut frame = crate::table_capture::CaptureRows::new(8);
        ledger.prepare(1);
        ledger.boundary(clock(1));
        assert!(ledger.drain(&mut frame.rows).is_err());
        ledger.prepare(0);
        ledger.boundary(clock(2));
        assert_eq!(ledger.drain(&mut frame.rows).unwrap(), 0);
        assert_eq!(ledger.demand(), 1);
        crate::test_support::with_allocations_denied(|| {
            for _ in 0..8 {
                let token = ledger.claim((1, 0)).unwrap();
                ledger
                    .complete(token, Ok(Some(Default::default())))
                    .unwrap();
            }
            let token = ledger.claim((1, 0)).unwrap();
            assert_eq!(
                ledger
                    .complete(token, Ok(Some(Default::default())))
                    .unwrap_err(),
                "event ledger event storage exhausted"
            );
        });
        assert_eq!(ledger.demand(), 9);
        ledger.refuse("event ledger event storage exhausted");
        ledger.prepare(0);
        assert!(crate::table_capture::grow_vec(
            &mut frame.rows,
            ledger.demand()
        ));
        ledger.boundary(clock(3));
        assert!(ledger.drain(&mut frame.rows).is_err());
        ledger.prepare(0);
        ledger.boundary(clock(4));
        for _ in 0..9 {
            let token = ledger.claim((1, 0)).unwrap();
            ledger
                .complete(token, Ok(Some(Default::default())))
                .unwrap();
        }
        assert_eq!(ledger.drain(&mut frame.rows).unwrap(), 9);
        assert_eq!(ledger.overflow.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn short_frame_drains_keep_writer_capacity_and_retire_storage_off_thread() {
        let ledger = prepared();
        assert!(ledger.retired.lock().unwrap().is_some());
        ledger.prepare(0);
        assert!(ledger.retired.lock().unwrap().is_none());
        ledger.boundary(clock(1));
        assert!(ledger.drain(&mut Vec::new()).is_err());
        ledger.boundary(clock(2));
        let pointer = ledger.store.lock().unwrap().entries.as_ptr();
        let mut empty = Vec::new();
        assert_eq!(ledger.drain(&mut empty).unwrap(), 0);
        assert_eq!(ledger.store.lock().unwrap().entries.as_ptr(), pointer);
        let token = ledger.claim((1, 0)).unwrap();
        ledger.complete(token, Ok(Some(Row(0)))).unwrap();
        let mut short = Vec::with_capacity(1);
        assert_eq!(ledger.drain(&mut short).unwrap(), 1);
        assert_eq!(ledger.store.lock().unwrap().entries.as_ptr(), pointer);
        crate::test_support::with_allocations_denied(|| {
            for owner in 1..=8 {
                let token = ledger.claim((owner, 0)).unwrap();
                ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            }
        });
        assert_eq!(ledger.overflow.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn spare_publication_never_locks_store_and_claim_storage_is_fixed() {
        let ledger = prepared();
        let store = ledger.store.lock().unwrap();
        ledger.prepare(5000);
        assert!(store.claim.is_none());
        drop(store);
        ledger.boundary(clock(1));
        assert!(ledger.drain(&mut Vec::new()).is_err());
        ledger.boundary(clock(2));
        for country in 0..5000 {
            let token = ledger.claim((country, 0)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
        }
        let mut rows = Vec::with_capacity(5000);
        assert_eq!(ledger.drain(&mut rows).unwrap(), 5000);
        assert_eq!(rows[4999].0, 4999);
        assert!(ledger.store.lock().unwrap().claim.is_none());
    }
    #[test]
    fn worker_drain_grows_empty_and_recycled_frames_before_handoff() {
        fn exercise<T: Event + Default>() {
            let ledger = EventLedger::<T>::new();
            let mut frame = Vec::new();
            ledger.prepare(5000);
            ledger.boundary(clock(1));
            assert!(ledger.drain_growing(&mut frame).is_err());
            ledger.boundary(clock(2));
            for (hour, count) in [(2, 5000), (3, 10003)] {
                if hour > 2 {
                    ledger.prepare(count);
                    ledger.boundary(clock(hour));
                    assert_eq!(ledger.drain_growing(&mut frame), Ok(0));
                }
                crate::test_support::with_allocations_denied(|| {
                    for country in 0..count {
                        let token = ledger.claim((country, 0)).unwrap();
                        ledger.complete(token, Ok(Some(T::default()))).unwrap();
                    }
                });
                assert_eq!(ledger.drain_growing(&mut frame), Ok(count));
                assert_eq!(frame.len(), count);
                assert_eq!(ledger.overflow.load(Ordering::Relaxed), 0);
                assert_eq!(ledger.lost_events.load(Ordering::Relaxed), 0);
            }
        }
        exercise::<Row>();
        exercise::<crate::air_losses::Entry>();
        exercise::<crate::building_bombing::Entry>();
    }
    #[test]
    fn refused_native_calls_retain_the_full_burst_after_the_first_overflow() {
        let ledger = EventLedger::<Row>::new();
        ledger.prepare(1);
        ledger.boundary(clock(1));
        assert!(ledger.drain_growing(&mut Vec::new()).is_err());
        ledger.boundary(clock(2));
        let token = ledger.claim((1, 0)).unwrap();
        ledger.complete(token, Ok(Some(Row::default()))).unwrap();
        let token = ledger.claim((1, 0)).unwrap();
        let reason = ledger
            .complete(token, Ok(Some(Row::default())))
            .unwrap_err();
        ledger.refuse(reason);
        crate::test_support::with_allocations_denied(|| {
            for _ in 0..10003 {
                ledger.attempt();
            }
        });
        assert_eq!(ledger.demand(), 10003);
        ledger.prepare(0);
        ledger.boundary(clock(3));
        assert!(ledger.drain_growing(&mut Vec::new()).is_err());
        ledger.boundary(clock(4));
        let mut frame = Vec::new();
        assert_eq!(ledger.drain_growing(&mut frame), Ok(0));
        crate::test_support::with_allocations_denied(|| {
            for country in 0..10003 {
                let token = ledger.claim((country, 0)).unwrap();
                ledger.complete(token, Ok(Some(Row::default()))).unwrap();
            }
        });
        assert_eq!(ledger.drain_growing(&mut frame), Ok(10003));
    }
    #[test]
    fn published_ordinals_survive_active_and_contended_valid_clock_boundaries() {
        for active in [true, false] {
            let ledger = prepared();
            ledger.boundary(clock(1));
            assert!(ledger.drain(&mut Vec::new()).is_err());
            ledger.boundary(clock(2));
            let token = ledger.claim((1, 1)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            let mut published = Vec::with_capacity(8);
            assert_eq!(ledger.drain(&mut published).unwrap(), 1);
            let before = published[0].0;

            // Sampling still publishes hour 3 on this same recorder branch.
            // The ledger cannot acquire/update its private clock at that hour.
            let store = if active {
                ledger.enter();
                None
            } else {
                Some(ledger.store.lock().unwrap())
            };
            ledger.boundary(clock(3));
            assert!(ledger.drain(&mut Vec::new()).is_err());
            if active {
                ledger.leave();
            }
            drop(store);
            ledger.boundary(clock(4));
            assert!(
                ledger
                    .drain(&mut Vec::new())
                    .unwrap_err()
                    .contains("partial")
            );
            ledger.boundary(clock(5));
            let token = ledger.claim((1, 1)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            assert_eq!(ledger.drain(&mut published).unwrap(), 1);
            assert!(published[0].0 > before);

            // A true clock transition may also discard data, never reuse IDs.
            let recovered = published[0].0;
            ledger.boundary(clock(-10));
            assert!(
                ledger
                    .drain(&mut Vec::new())
                    .unwrap_err()
                    .contains("partial")
            );
            ledger.boundary(clock(-9));
            let token = ledger.claim((1, 1)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            assert_eq!(ledger.drain(&mut published).unwrap(), 1);
            assert!(published[0].0 > recovered);
        }
    }
    #[test]
    fn one_fixed_claim_preserves_owner_until_matching_completion() {
        let ledger = prepared();
        ledger.boundary(clock(1));
        ledger.boundary(clock(2));
        let token = ledger.claim((1, 1)).unwrap();
        assert_eq!(
            ledger.claim((2, 2)).unwrap_err(),
            "event ledger native calls overlap"
        );
        assert!(ledger.complete(token + 1, Ok(None)).is_err());
        ledger.complete(token, Ok(Some(Row(0)))).unwrap();
        let next = ledger.claim((2, 2)).unwrap();
        assert!(next > token);
        ledger.complete(next, Ok(None)).unwrap();
        let mut published = Vec::with_capacity(8);
        assert_eq!(ledger.drain(&mut published).unwrap(), 1);
    }

    #[test]
    fn overlap_and_loss_refuse_then_recover_through_a_named_partial_interval() {
        let ledger = prepared();
        ledger.boundary(clock(1));
        ledger.boundary(clock(2));
        let token = ledger.claim((1, 1)).unwrap();
        let reason = ledger.claim((1, 1)).unwrap_err();
        ledger.refuse(reason);
        assert!(ledger.complete(token, Ok(Some(Row(0)))).is_err());
        ledger.boundary(clock(3));
        assert_eq!(ledger.drain(&mut Vec::new()).unwrap_err(), reason);
        ledger.boundary(clock(4));
        assert!(
            ledger
                .drain(&mut Vec::new())
                .unwrap_err()
                .contains("partial")
        );
        ledger.boundary(clock(5));
        assert_eq!(ledger.drain(&mut Vec::new()).unwrap(), 0);
        ledger.enter();
        ledger.boundary(clock(6));
        assert!(ledger.stranded.load(Ordering::Relaxed));
        ledger.leave();
        assert!(!ledger.stranded.load(Ordering::Relaxed));
    }
    #[test]
    fn spool_losses_invalidate_atomically_without_touching_writer_store() {
        for output_loss in [false, true] {
            let ledger = prepared();
            ledger.boundary(clock(1));
            ledger.boundary(clock(2));
            let token = ledger.claim((1, 1)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            let store = ledger.store.lock().unwrap();
            if output_loss {
                ledger.lost_output(3);
            } else {
                ledger.lost_drain();
            }
            // The spool path leaves the store entirely to the game thread.
            assert_eq!(store.entries.len(), 1);
            assert_eq!(store.refusal, None);
            assert!(ledger.poisoned.load(Ordering::SeqCst));
            assert_eq!(ledger.refusals.load(Ordering::Relaxed), 1);
            assert_eq!(ledger.overlaps.load(Ordering::Relaxed), 0);
            drop(store);
            ledger.boundary(clock(3));
            assert_eq!(
                ledger.drain(&mut Vec::new()).unwrap_err(),
                "event ledger lost interval"
            );
            assert_eq!(
                ledger.lost_events.load(Ordering::Relaxed),
                if output_loss { 4 } else { 1 }
            );
            assert_eq!(
                ledger.lost_events_unknown.load(Ordering::Relaxed),
                u64::from(!output_loss)
            );
            ledger.boundary(clock(4));
            assert!(
                ledger
                    .drain(&mut Vec::new())
                    .unwrap_err()
                    .contains("partial")
            );
            ledger.boundary(clock(5));
            assert_eq!(ledger.drain(&mut Vec::new()).unwrap(), 0);
        }
        // With the store free, the old spool-side refuse() discarded events
        // itself. Atomic invalidation leaves that work to the game thread.
        let ledger = prepared();
        ledger.boundary(clock(1));
        ledger.boundary(clock(2));
        let token = ledger.claim((1, 1)).unwrap();
        ledger.complete(token, Ok(Some(Row(0)))).unwrap();
        ledger.lost_drain();
        ledger.lost_output(0);
        let store = ledger.store.lock().unwrap();
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.refusal, None);
    }
    #[test]
    fn refusal_without_store_lock_cannot_publish_a_complete_empty_interval() {
        let ledger = prepared();
        ledger.boundary(clock(1));
        ledger.boundary(clock(2));
        let token = ledger.claim((1, 1)).unwrap();
        let store = ledger.store.lock().unwrap();
        ledger.refuse("contended refusal");
        drop(store);
        ledger.boundary(clock(3));
        assert_eq!(
            ledger.drain(&mut Vec::new()).unwrap_err(),
            "event ledger lost interval"
        );
        assert!(ledger.complete(token, Ok(None)).is_err());
        ledger.boundary(clock(4));
        assert!(
            ledger
                .drain(&mut Vec::new())
                .unwrap_err()
                .contains("partial")
        );
        ledger.boundary(clock(5));
        assert_eq!(ledger.drain(&mut Vec::new()).unwrap(), 0);
    }

    #[test]
    fn retained_sequence_keeps_zero_start_partial_startup_and_checked_exhaustion() {
        let first = prepared();
        first.restore_sequence(None).unwrap();
        first.boundary(clock(1));
        first.boundary(clock(2));
        let token = first.claim((1, 1)).unwrap();
        first.complete(token, Ok(Some(Row(0)))).unwrap();
        let mut rows = Vec::with_capacity(8);
        first.drain(&mut rows).unwrap();
        assert_eq!(rows[0].0, 0);

        let resumed = prepared();
        resumed.restore_sequence(Some(rows[0].0)).unwrap();
        resumed.boundary(clock(3));
        assert!(resumed.drain(&mut rows).unwrap_err().contains("partial"));
        resumed.boundary(clock(4));
        for expected in [1, 2] {
            resumed.restore_sequence(Some(0)).unwrap();
            let token = resumed.claim((1, 1)).unwrap();
            resumed.complete(token, Ok(Some(Row(0)))).unwrap();
            resumed.drain(&mut rows).unwrap();
            assert_eq!(rows[0].0, expected);
        }
        assert_eq!(
            resumed.restore_sequence(Some(u64::MAX)).unwrap_err(),
            "event ledger event identity exhausted"
        );
        resumed.enter();
        assert!(
            resumed
                .restore_sequence(None)
                .unwrap_err()
                .contains("quiet")
        );
        resumed.leave();
    }
}
