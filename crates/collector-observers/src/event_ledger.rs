type Clock = engine_native::cgame_state::Clock<u64>;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

pub const STORAGE_FAILED: &str = "event ledger event storage allocation failed";

pub trait Event: Copy {
    fn set_id(&mut self, id: u64);
}

struct Claim {
    token: usize,
    key: (usize, usize),
}
struct Store<T> {
    entries: Vec<T>,
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
    native_recovery: AtomicBool,
    pub stranded: AtomicBool,
    pub active: AtomicUsize,
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
            native_recovery: AtomicBool::new(false),
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
        self.native_recovery.store(false, Ordering::SeqCst);
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
        engine_allocation::advisory(|| self.prepare_storage(demand));
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
        if engine_allocation::reserve_exact(&mut spare, demand).is_err() {
            self.refuse_preparation("event ledger preparation allocation failed");
            return;
        }
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
        if self.active.load(Ordering::SeqCst) != 0 {
            self.stranded.store(true, Ordering::SeqCst);
            self.native_recovery.store(true, Ordering::SeqCst);
        }
    }
    fn discard(&self, store: &mut Store<T>) {
        self.lost_events
            .fetch_add(store.entries.len() as u64, Ordering::Relaxed);
        store.entries.clear();
    }
    pub fn refuse(&self, reason: &'static str) {
        if !matches!(
            reason,
            STORAGE_FAILED | "event ledger observation invalidated"
        ) {
            self.native_recovery.store(true, Ordering::SeqCst);
        }
        self.invalidate();
        if let Ok(mut store) = self.store.try_lock() {
            store.refusal.get_or_insert(reason);
            self.discard(&mut store);
        }
    }
    fn invalidate(&self) {
        self.refusals.fetch_add(1, Ordering::Relaxed);
        self.poisoned.store(true, Ordering::SeqCst);
    }
    fn record_lost_drain(&self) {
        self.lost_drains.fetch_add(1, Ordering::Relaxed);
        self.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
        self.invalidate();
    }
    /// # Safety
    /// Caller must own the active session's recovery or publication failure path (or an isolated
    /// test ledger) when invalidating live event-ledger observation.
    pub unsafe fn lost_drain(&self) {
        self.record_lost_drain();
    }
    fn record_lost_output(&self, events: usize) {
        self.lost_events.fetch_add(events as u64, Ordering::Relaxed);
        self.lost_drains.fetch_add(1, Ordering::Relaxed);
        self.invalidate();
    }
    /// # Safety
    /// Caller must own the active session's projection or publication failure path (or an isolated
    /// test ledger) when invalidating live event-ledger output.
    pub unsafe fn lost_output(&self, events: usize) {
        self.record_lost_output(events);
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
            engine_allocation::advisory(|| engine_allocation::reserve(&mut store.entries, 1))
                .map_err(|_| STORAGE_FAILED)?;
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
    /// # Safety
    /// Caller must run on the admitted hourly capture boundary as the sole consumer advancing this
    /// ledger's clock and interval (or own an isolated test ledger).
    pub unsafe fn boundary(&self, clock: Option<Clock>) {
        self.advance_boundary(clock);
    }
    fn advance_boundary(&self, clock: Option<Clock>) {
        let Some(clock) = clock else {
            self.record_lost_drain();
            return;
        };
        if self.active.load(Ordering::SeqCst) != 0 {
            self.mark_stranded();
            self.record_lost_drain();
            return;
        }
        let generation = self.calls.load(Ordering::SeqCst);
        let Ok(mut store) = self.store.try_lock() else {
            self.native_recovery.store(true, Ordering::SeqCst);
            self.record_lost_drain();
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
            store.partial = true;
            store.recovery = false;
            store.refusal = None;
            self.poisoned.store(false, Ordering::SeqCst);
            self.native_recovery.store(false, Ordering::SeqCst);
        } else if self.poisoned.swap(false, Ordering::SeqCst) {
            self.discard(&mut store);
            store.claim = None;
            store.refusal.get_or_insert("event ledger lost interval");
            store.recovery = self.native_recovery.swap(false, Ordering::SeqCst);
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
            self.native_recovery.store(true, Ordering::SeqCst);
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
    pub fn interval_incomplete(&self) -> bool {
        self.poisoned.load(Ordering::SeqCst)
            || self.active.load(Ordering::SeqCst) != 0
            || self
                .store
                .try_lock()
                .map_or(true, |store| store.partial || store.refusal.is_some())
    }
    /// # Safety
    /// Caller must be the admitted sole consumer draining this ledger while its storage stays live
    /// and teardown is excluded (or own an isolated test ledger).
    pub unsafe fn drain(&self, output: &mut Vec<T>) -> Result<usize, &'static str> {
        self.drain_buffer(output)
    }
    fn drain_buffer(&self, output: &mut Vec<T>) -> Result<usize, &'static str> {
        self.drain_buffer_with(output, || {})
    }
    fn drain_buffer_with(
        &self,
        output: &mut Vec<T>,
        after_transfer: impl FnOnce(),
    ) -> Result<usize, &'static str> {
        self.drains.fetch_add(1, Ordering::Relaxed);
        if self.active.load(Ordering::SeqCst) != 0 {
            self.mark_stranded();
            self.record_lost_drain();
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
        engine_allocation::reserve_exact(output, store.entries.len())
            .map_err(|_| "event ledger drain allocation failed")?;
        
        if output.capacity() < store.entries.len() {
            self.refusals.fetch_add(1, Ordering::Relaxed);
            self.discard(&mut store);
            return Err("event ledger drain storage exhausted");
        }
        if output.capacity() >= store.entries.capacity() {
            std::mem::swap(output, &mut store.entries);
        } else {
            output.extend_from_slice(&store.entries);
            store.entries.clear();
        }
        after_transfer();
        let poisoned = self.poisoned.load(Ordering::SeqCst);
        let native_overlap = self.active.load(Ordering::SeqCst) != 0
            || self.calls.load(Ordering::SeqCst) != generation;
        if poisoned || native_overlap {
            if native_overlap {
                self.overlaps.fetch_add(1, Ordering::Relaxed);
                self.native_recovery.store(true, Ordering::SeqCst);
            }
            let len = output.len();
            output.clear();
            drop(store);
            self.record_lost_output(len);
            return Err(if native_overlap {
                "event ledger writer overlaps hourly drain"
            } else {
                "event ledger lost interval"
            });
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
        assert!(ledger.drain_buffer(&mut Vec::new()).is_err());
        ledger
    }
    #[test]
    fn incomplete_boundary_hint_preserves_first_loss_and_recovery_refusals() {
        let ledger = EventLedger::<Row>::new();
        ledger.prepare(8);
        let mut rows = Vec::new();
        ledger.advance_boundary(clock(1));
        assert!(ledger.interval_incomplete());
        assert!(ledger.drain_buffer(&mut rows).is_err());
        ledger.advance_boundary(clock(2));
        assert!(!ledger.interval_incomplete());
        assert_eq!(ledger.drain_buffer(&mut rows), Ok(0));
        ledger.refuse("synthetic lost interval");
        ledger.advance_boundary(clock(3));
        assert!(ledger.interval_incomplete());
        assert_eq!(
            ledger.drain_buffer(&mut rows),
            Err("synthetic lost interval")
        );
        ledger.advance_boundary(clock(4));
        assert!(ledger.interval_incomplete());
        assert!(ledger.drain_buffer(&mut rows).is_err());
        ledger.advance_boundary(clock(5));
        assert!(!ledger.interval_incomplete());
        assert_eq!(ledger.drain_buffer(&mut rows), Ok(0));
    }

    #[test]
    fn a_burst_past_the_prepared_store_is_kept_whole() {
        let ledger = EventLedger::<Row>::new();
        ledger.prepare(1);
        ledger.advance_boundary(clock(1));
        assert!(ledger.drain_buffer(&mut Vec::new()).is_err());
        ledger.advance_boundary(clock(2));
        for owner in 0..5000 {
            let token = ledger.claim((owner, 0)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
        }
        let mut rows = Vec::new();
        assert_eq!(ledger.drain_buffer(&mut rows), Ok(5000));
        assert_eq!(rows[4999].0, 4999);
        assert_eq!(ledger.lost_events.load(Ordering::Relaxed), 0);
        assert!(ledger.demand() >= 5000);
    }

    #[test]
    fn a_full_store_grows_in_the_callback_and_only_a_failed_allocation_refuses() {
        let ledger = EventLedger::<Row>::new();
        ledger.prepare(1);
        ledger.advance_boundary(clock(1));
        assert!(ledger.drain_buffer(&mut Vec::new()).is_err());
        ledger.advance_boundary(clock(2));
        let pointer = ledger.store.lock().unwrap().entries.as_ptr();
        let result = crate::test_support::with_allocations_denied(|| {
            let token = ledger.claim((1, 1)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            let token = ledger.claim((2, 2)).unwrap();
            ledger.complete(token, Ok(Some(Row(0))))
        });
        assert_eq!(result.unwrap_err(), STORAGE_FAILED);
        assert_eq!(ledger.store.lock().unwrap().entries.as_ptr(), pointer);
        assert_eq!(ledger.capacity.load(Ordering::Acquire), 1);
        assert_eq!(ledger.demand(), 2);
        assert_eq!(ledger.overflow.load(Ordering::Relaxed), 1);
        ledger.refuse(STORAGE_FAILED);
        ledger.prepare(ledger.demand());
        ledger.advance_boundary(clock(3));
        let mut rows = Vec::with_capacity(2);
        assert!(ledger.drain_buffer(&mut rows).is_err());
        ledger.advance_boundary(clock(4));
        assert_eq!(ledger.drain_buffer(&mut rows).unwrap(), 0);
        ledger.advance_boundary(clock(5));
        for owner in [1, 2] {
            let token = ledger.claim((owner, 0)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
        }
        assert_eq!(ledger.drain_buffer(&mut rows).unwrap(), 2);
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
            ledger.advance_boundary(clock(1));
            assert!(ledger.drain_buffer(&mut frames[0].rows).is_err());
            ledger.prepare(0);

            ledger.prepare(9);
            let mut capacity = 9;
            for hour in 2..=10 {
                let frame = &mut frames[(hour as usize - 2) % 4];
                crate::test_support::with_allocations_denied(|| {
                    ledger.advance_boundary(clock(hour));
                    let token = ledger.claim((1, 0)).unwrap();
                    ledger.complete(token, Ok(Some(T::default()))).unwrap();
                    assert_eq!(ledger.drain_buffer(&mut frame.rows).unwrap(), 1);
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
        ledger.advance_boundary(clock(1));
        assert!(ledger.drain_buffer(&mut frame.rows).is_err());
        ledger.prepare(0);
        ledger.advance_boundary(clock(2));
        assert_eq!(ledger.drain_buffer(&mut frame.rows).unwrap(), 0);
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
                STORAGE_FAILED
            );
        });
        assert_eq!(ledger.demand(), 9);
        ledger.refuse(STORAGE_FAILED);
        ledger.prepare(0);
        assert!(crate::table_capture::grow_vec(
            &mut frame.rows,
            ledger.demand()
        ));
        ledger.advance_boundary(clock(3));
        assert!(ledger.drain_buffer(&mut frame.rows).is_err());
        ledger.prepare(0);
        ledger.advance_boundary(clock(4));
        for _ in 0..9 {
            let token = ledger.claim((1, 0)).unwrap();
            ledger
                .complete(token, Ok(Some(Default::default())))
                .unwrap();
        }
        assert_eq!(ledger.drain_buffer(&mut frame.rows).unwrap(), 9);
        assert_eq!(ledger.overflow.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn short_frame_drains_keep_writer_capacity_and_retire_storage_off_thread() {
        let ledger = prepared();
        assert!(ledger.retired.lock().unwrap().is_some());
        ledger.prepare(0);
        assert!(ledger.retired.lock().unwrap().is_none());
        ledger.advance_boundary(clock(1));
        assert!(ledger.drain_buffer(&mut Vec::new()).is_err());
        ledger.advance_boundary(clock(2));
        let pointer = ledger.store.lock().unwrap().entries.as_ptr();
        let mut empty = Vec::new();
        assert_eq!(ledger.drain_buffer(&mut empty).unwrap(), 0);
        assert_eq!(ledger.store.lock().unwrap().entries.as_ptr(), pointer);
        let token = ledger.claim((1, 0)).unwrap();
        ledger.complete(token, Ok(Some(Row(0)))).unwrap();
        let mut short = Vec::with_capacity(1);
        assert_eq!(ledger.drain_buffer(&mut short).unwrap(), 1);
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
        ledger.advance_boundary(clock(1));
        assert!(ledger.drain_buffer(&mut Vec::new()).is_err());
        ledger.advance_boundary(clock(2));
        for country in 0..5000 {
            let token = ledger.claim((country, 0)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
        }
        let mut rows = Vec::with_capacity(5000);
        assert_eq!(ledger.drain_buffer(&mut rows).unwrap(), 5000);
        assert_eq!(rows[4999].0, 4999);
        assert!(ledger.store.lock().unwrap().claim.is_none());
    }
    #[test]
    fn worker_drain_grows_empty_and_recycled_frames_before_handoff() {
        fn exercise<T: Event + Default>() {
            let ledger = EventLedger::<T>::new();
            let mut frame = Vec::new();
            ledger.prepare(5000);
            ledger.advance_boundary(clock(1));
            assert!(ledger.drain_buffer(&mut frame).is_err());
            ledger.advance_boundary(clock(2));
            for (hour, count) in [(2, 5000), (3, 10003)] {
                if hour > 2 {
                    ledger.prepare(count);
                    ledger.advance_boundary(clock(hour));
                    assert_eq!(ledger.drain_buffer(&mut frame), Ok(0));
                }
                crate::test_support::with_allocations_denied(|| {
                    for country in 0..count {
                        let token = ledger.claim((country, 0)).unwrap();
                        ledger.complete(token, Ok(Some(T::default()))).unwrap();
                    }
                });
                assert_eq!(ledger.drain_buffer(&mut frame), Ok(count));
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
    fn refused_native_calls_retain_the_full_burst_after_a_failed_growth() {
        let ledger = EventLedger::<Row>::new();
        ledger.prepare(1);
        ledger.advance_boundary(clock(1));
        assert!(ledger.drain_buffer(&mut Vec::new()).is_err());
        ledger.advance_boundary(clock(2));
        let token = ledger.claim((1, 0)).unwrap();
        ledger.complete(token, Ok(Some(Row::default()))).unwrap();
        let token = ledger.claim((1, 0)).unwrap();
        let reason = crate::test_support::with_allocations_denied(|| {
            ledger.complete(token, Ok(Some(Row::default())))
        })
        .unwrap_err();
        assert_eq!(reason, STORAGE_FAILED);
        ledger.refuse(reason);
        crate::test_support::with_allocations_denied(|| {
            for _ in 0..10003 {
                ledger.attempt();
            }
        });
        assert_eq!(ledger.demand(), 10003);
        ledger.prepare(0);
        ledger.advance_boundary(clock(3));
        assert!(ledger.drain_buffer(&mut Vec::new()).is_err());
        ledger.advance_boundary(clock(4));
        let mut frame = Vec::new();
        assert_eq!(ledger.drain_buffer(&mut frame), Ok(0));
        crate::test_support::with_allocations_denied(|| {
            for country in 0..10003 {
                let token = ledger.claim((country, 0)).unwrap();
                ledger.complete(token, Ok(Some(Row::default()))).unwrap();
            }
        });
        assert_eq!(ledger.drain_buffer(&mut frame), Ok(10003));
    }
    #[test]
    fn published_ordinals_survive_active_and_contended_valid_clock_boundaries() {
        for active in [true, false] {
            let ledger = prepared();
            ledger.advance_boundary(clock(1));
            assert!(ledger.drain_buffer(&mut Vec::new()).is_err());
            ledger.advance_boundary(clock(2));
            let token = ledger.claim((1, 1)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            let mut published = Vec::with_capacity(8);
            assert_eq!(ledger.drain_buffer(&mut published).unwrap(), 1);
            let before = published[0].0;

            let store = if active {
                ledger.enter();
                None
            } else {
                Some(ledger.store.lock().unwrap())
            };
            ledger.advance_boundary(clock(3));
            assert!(ledger.drain_buffer(&mut Vec::new()).is_err());
            if active {
                ledger.leave();
            }
            drop(store);
            ledger.advance_boundary(clock(4));
            assert!(
                ledger
                    .drain_buffer(&mut Vec::new())
                    .unwrap_err()
                    .contains("partial")
            );
            ledger.advance_boundary(clock(5));
            let token = ledger.claim((1, 1)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            assert_eq!(ledger.drain_buffer(&mut published).unwrap(), 1);
            assert!(published[0].0 > before);

            let recovered = published[0].0;
            ledger.advance_boundary(clock(-10));
            assert!(
                ledger
                    .drain_buffer(&mut Vec::new())
                    .unwrap_err()
                    .contains("partial")
            );
            ledger.advance_boundary(clock(-9));
            let token = ledger.claim((1, 1)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            assert_eq!(ledger.drain_buffer(&mut published).unwrap(), 1);
            assert!(published[0].0 > recovered);
        }
    }
    #[test]
    fn one_fixed_claim_preserves_owner_until_matching_completion() {
        let ledger = prepared();
        ledger.advance_boundary(clock(1));
        ledger.advance_boundary(clock(2));
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
        assert_eq!(ledger.drain_buffer(&mut published).unwrap(), 1);
    }

    #[test]
    fn overlap_and_loss_refuse_then_recover_through_a_named_partial_interval() {
        let ledger = prepared();
        ledger.advance_boundary(clock(1));
        ledger.advance_boundary(clock(2));
        let token = ledger.claim((1, 1)).unwrap();
        let reason = ledger.claim((1, 1)).unwrap_err();
        ledger.refuse(reason);
        ledger.record_lost_output(0);
        assert!(ledger.complete(token, Ok(Some(Row(0)))).is_err());
        ledger.advance_boundary(clock(3));
        assert_eq!(ledger.drain_buffer(&mut Vec::new()).unwrap_err(), reason);
        ledger.advance_boundary(clock(4));
        assert!(
            ledger
                .drain_buffer(&mut Vec::new())
                .unwrap_err()
                .contains("partial")
        );
        ledger.advance_boundary(clock(5));
        assert_eq!(ledger.drain_buffer(&mut Vec::new()).unwrap(), 0);
        ledger.enter();
        ledger.advance_boundary(clock(6));
        assert!(ledger.stranded.load(Ordering::Relaxed));
        ledger.leave();
        assert!(!ledger.stranded.load(Ordering::Relaxed));
    }
    #[test]
    fn spool_losses_invalidate_atomically_without_touching_writer_store() {
        for output_loss in [false, true] {
            let ledger = prepared();
            ledger.advance_boundary(clock(1));
            ledger.advance_boundary(clock(2));
            let token = ledger.claim((1, 1)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            let store = ledger.store.lock().unwrap();
            if output_loss {
                ledger.record_lost_output(3);
            } else {
                ledger.record_lost_drain();
            }
            assert_eq!(store.entries.len(), 1);
            assert_eq!(store.refusal, None);
            assert!(ledger.poisoned.load(Ordering::SeqCst));
            assert_eq!(ledger.refusals.load(Ordering::Relaxed), 1);
            assert_eq!(ledger.overlaps.load(Ordering::Relaxed), 0);
            drop(store);
            let token = ledger.claim((2, 2)).unwrap();
            let reason = ledger.complete(token, Ok(Some(Row(0)))).unwrap_err();
            assert_eq!(reason, "event ledger observation invalidated");
            ledger.refuse(reason);
            ledger.advance_boundary(clock(3));
            assert_eq!(ledger.drain_buffer(&mut Vec::new()).unwrap_err(), reason);
            assert_eq!(
                ledger.lost_events.load(Ordering::Relaxed),
                if output_loss { 4 } else { 1 }
            );
            assert_eq!(
                ledger.lost_events_unknown.load(Ordering::Relaxed),
                u64::from(!output_loss)
            );
            let token = ledger.claim((2, 2)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            ledger.advance_boundary(clock(4));
            assert!(!ledger.interval_incomplete());
            let mut intact = Vec::with_capacity(8);
            assert_eq!(ledger.drain_buffer(&mut intact).unwrap(), 1);
            assert_eq!(intact[0].0, 1);
            ledger.advance_boundary(clock(5));
            assert_eq!(ledger.drain_buffer(&mut Vec::new()).unwrap(), 0);
        }
        let ledger = prepared();
        ledger.advance_boundary(clock(1));
        ledger.advance_boundary(clock(2));
        let token = ledger.claim((1, 1)).unwrap();
        ledger.complete(token, Ok(Some(Row(0)))).unwrap();
        ledger.record_lost_drain();
        ledger.record_lost_output(0);
        let store = ledger.store.lock().unwrap();
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.refusal, None);
    }
    #[test]
    fn spool_loss_during_drain_preserves_the_next_intact_interval_without_a_census() {
        for output_loss in [false, true] {
            let ledger = prepared();
            ledger.advance_boundary(clock(100));
            ledger.advance_boundary(clock(101));
            let token = ledger.claim((1, 1)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            let mut rows = Vec::with_capacity(8);
            assert_eq!(
                ledger.drain_buffer_with(&mut rows, || {
                    assert!(!ledger.poisoned.load(Ordering::SeqCst));
                    assert!(ledger.store.try_lock().is_err());
                    if output_loss {
                        ledger.record_lost_output(3);
                    } else {
                        ledger.record_lost_drain();
                    }
                }),
                Err("event ledger lost interval")
            );
            assert!(rows.is_empty());
            assert_eq!(ledger.handed_over.load(Ordering::Relaxed), 0);
            assert_eq!(ledger.overlaps.load(Ordering::Relaxed), 0);
            assert!(!ledger.native_recovery.load(Ordering::SeqCst));
            ledger.advance_boundary(clock(102));
            assert!(ledger.interval_incomplete());
            assert_eq!(ledger.drain_buffer(&mut rows), Err("event ledger lost interval"));
            let token = ledger.claim((2, 2)).unwrap();
            ledger.complete(token, Ok(Some(Row(0)))).unwrap();
            ledger.advance_boundary(clock(103));
            assert!(!ledger.interval_incomplete());
            assert_eq!(ledger.drain_buffer(&mut rows), Ok(1));
            assert_eq!(rows[0].0, 1);
            ledger.advance_boundary(clock(104));
            assert!(!ledger.interval_incomplete());
            assert_eq!(ledger.drain_buffer(&mut rows), Ok(0));
        }
    }
    #[test]
    fn native_overlap_during_drain_still_requires_partial_recovery() {
        for active in [false, true] {
            let ledger = prepared();
            ledger.advance_boundary(clock(1));
            ledger.advance_boundary(clock(2));
            let mut rows = Vec::with_capacity(8);
            assert_eq!(
                ledger.drain_buffer_with(&mut rows, || {
                    ledger.record_lost_drain();
                    ledger.enter();
                    if !active {
                        ledger.leave();
                    }
                }),
                Err("event ledger writer overlaps hourly drain")
            );
            if active {
                ledger.leave();
            }
            assert!(ledger.native_recovery.load(Ordering::SeqCst));
            ledger.advance_boundary(clock(3));
            assert!(ledger.drain_buffer(&mut rows).is_err());
            ledger.advance_boundary(clock(4));
            assert!(ledger.drain_buffer(&mut rows).unwrap_err().contains("partial"));
            ledger.advance_boundary(clock(5));
            assert_eq!(ledger.drain_buffer(&mut rows), Ok(0));
        }
    }
    #[test]
    fn spool_loss_during_drain_does_not_clear_an_existing_native_recovery() {
        let ledger = prepared();
        ledger.advance_boundary(clock(1));
        ledger.advance_boundary(clock(2));
        let mut rows = Vec::with_capacity(8);
        assert_eq!(
            ledger.drain_buffer_with(&mut rows, || {
                ledger.refuse("synthetic native observation failure");
                ledger.record_lost_drain();
            }),
            Err("event ledger lost interval")
        );
        assert!(ledger.native_recovery.load(Ordering::SeqCst));
        ledger.advance_boundary(clock(3));
        assert!(ledger.drain_buffer(&mut rows).is_err());
        ledger.advance_boundary(clock(4));
        assert!(ledger.drain_buffer(&mut rows).unwrap_err().contains("partial"));
        ledger.advance_boundary(clock(5));
        assert_eq!(ledger.drain_buffer(&mut rows), Ok(0));
    }
    #[test]
    fn refusal_without_store_lock_cannot_publish_a_complete_empty_interval() {
        let ledger = prepared();
        ledger.advance_boundary(clock(1));
        ledger.advance_boundary(clock(2));
        let token = ledger.claim((1, 1)).unwrap();
        let store = ledger.store.lock().unwrap();
        ledger.refuse("contended refusal");
        drop(store);
        ledger.advance_boundary(clock(3));
        assert_eq!(
            ledger.drain_buffer(&mut Vec::new()).unwrap_err(),
            "event ledger lost interval"
        );
        assert!(ledger.complete(token, Ok(None)).is_err());
        ledger.advance_boundary(clock(4));
        assert!(
            ledger
                .drain_buffer(&mut Vec::new())
                .unwrap_err()
                .contains("partial")
        );
        ledger.advance_boundary(clock(5));
        assert_eq!(ledger.drain_buffer(&mut Vec::new()).unwrap(), 0);
    }

    #[test]
    fn retained_sequence_keeps_zero_start_partial_startup_and_checked_exhaustion() {
        let first = prepared();
        first.restore_sequence(None).unwrap();
        first.advance_boundary(clock(1));
        first.advance_boundary(clock(2));
        let token = first.claim((1, 1)).unwrap();
        first.complete(token, Ok(Some(Row(0)))).unwrap();
        let mut rows = Vec::with_capacity(8);
        first.drain_buffer(&mut rows).unwrap();
        assert_eq!(rows[0].0, 0);

        let resumed = prepared();
        resumed.restore_sequence(Some(rows[0].0)).unwrap();
        resumed.advance_boundary(clock(3));
        assert!(resumed.drain_buffer(&mut rows).unwrap_err().contains("partial"));
        resumed.advance_boundary(clock(4));
        for expected in [1, 2] {
            resumed.restore_sequence(Some(0)).unwrap();
            let token = resumed.claim((1, 1)).unwrap();
            resumed.complete(token, Ok(Some(Row(0)))).unwrap();
            resumed.drain_buffer(&mut rows).unwrap();
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
