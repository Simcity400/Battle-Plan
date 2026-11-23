#[cfg(all(test, feature = "tap-foreign-probe"))]
mod foreign_unwind;
use collector_hooks::hook::Hook;
use collector_observers::opinion_changes::Padded;
use collector_primitives::spill_stack::SpillStack;
use engine_land_inputs::land_events as native;
use engine_native_memory::Memory;
use std::{
    cell::Cell,
    ops::Range,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

struct Storage {
    words: Vec<AtomicU64>,
    _charge: collector_primitives::raw_memory::Charge,
}
impl Storage {
    fn new(words: Vec<AtomicU64>, mut charge: collector_primitives::raw_memory::Charge) -> Self {
        charge.set(collector_primitives::raw_memory::Capacity {
            payload_vectors: collector_primitives::raw_memory::vector_bytes(&words),
            ..Default::default()
        });
        Self {
            words,
            _charge: charge,
        }
    }
}
#[derive(Default)]
struct LogSwap {
    current: Option<Box<Storage>>,
    next: Option<Box<Storage>>,
    retired: Option<Box<Storage>>,
}
static TAP_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static TAP_ESCAPED_UNWINDS: AtomicU64 = AtomicU64::new(0);
const TAP_UNWIND_RETENTION: &str = "land verdict resolver escaped unwind; collector retained";
const CALL_UNWIND_RETENTION: &str = "land callback escaped unwind; collector retained";
static BASE: AtomicUsize = AtomicUsize::new(0);
const STORAGE_FAILED: &str = "land event storage allocation failed";

pub(crate) struct Observer {
    tap_in_flight: &'static AtomicUsize,
    tap_unwinds: &'static AtomicU64,
    log_swap: Mutex<LogSwap>,
    log: AtomicPtr<Storage>,
    log_capacity: AtomicUsize,
    log_required: AtomicUsize,
    observer_words: AtomicUsize,
    base_words: AtomicUsize,
    resizing: AtomicBool,
    spill: SpillStack<(u64, Vec<u64>)>,
    spilled: Mutex<Vec<(u64, u64)>>,
    storage_lost: AtomicU64,
    cursor: AtomicU64,
    consumed: AtomicU64,
    epoch: AtomicU64,
    observed_epoch: AtomicU64,
    last_hour: AtomicU64,
    enabled: AtomicBool,
    call_escaped_unwinds: AtomicU64,
    active: Padded<AtomicUsize>,
    producers: Padded<AtomicUsize>,
    lost: AtomicU64,
}

impl Observer {
    const fn new(tap_in_flight: &'static AtomicUsize, tap_unwinds: &'static AtomicU64) -> Self {
        Self {
            tap_in_flight,
            tap_unwinds,
            log_swap: Mutex::new(LogSwap {
                current: None,
                next: None,
                retired: None,
            }),
            log: AtomicPtr::new(ptr::null_mut()),
            log_capacity: AtomicUsize::new(0),
            log_required: AtomicUsize::new(0),
            observer_words: AtomicUsize::new(0),
            base_words: AtomicUsize::new(0),
            resizing: AtomicBool::new(false),
            spill: SpillStack::new(),
            spilled: Mutex::new(Vec::new()),
            storage_lost: AtomicU64::new(u64::MAX),
            cursor: AtomicU64::new(0),
            consumed: AtomicU64::new(0),
            epoch: AtomicU64::new(1),
            observed_epoch: AtomicU64::new(0),
            last_hour: AtomicU64::new(0),
            enabled: AtomicBool::new(false),
            call_escaped_unwinds: AtomicU64::new(0),
            active: Padded(AtomicUsize::new(0)),
            producers: Padded(AtomicUsize::new(0)),
            lost: AtomicU64::new(0),
        }
    }

    fn spilled(&self) -> std::sync::MutexGuard<'_, Vec<(u64, u64)>> {
        self.spilled.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn take_spill(&self, held: &mut Vec<(u64, u64)>, end: u64) -> bool {
        let mut kept = true;
        for (first, words) in self.spill.take() {
            if engine_allocation::reserve(held, words.len()).is_err() {
                if first < end {
                    kept = false;
                } else {
                    self.storage_lost.fetch_min(first, Ordering::SeqCst);
                    self.fail();
                }
                continue;
            }
            held.extend((first..).zip(words));
        }
        kept
    }

    fn prepare_storage(&self) {
        let Ok(mut swap) = self.log_swap.lock() else {
            return;
        };
        drop(swap.retired.take());
        let required = self.log_required.load(Ordering::Relaxed);
        let mut capacity = self.log_capacity.load(Ordering::Acquire);
        if required <= capacity
            || swap
                .next
                .as_ref()
                .is_some_and(|s| s.words.len() >= required)
        {
            return;
        }
        collector_primitives::table_capture::grow_capacity(&mut capacity, required);
        if let Ok(mut words) = engine_allocation::vec(capacity) {
            words.resize_with(capacity, || AtomicU64::new(0));
            if let Ok(next) = engine_allocation::boxed(Storage::new(words, Default::default())) {
                swap.next = Some(next);
            }
        }
    }

    pub(crate) fn install_offered(&self) {
        let Ok(mut swap) = self.log_swap.try_lock() else {
            return;
        };
        if swap.next.is_none() || swap.retired.is_some() {
            return;
        }
        self.resizing.store(true, Ordering::SeqCst);
        if self.producers.load(Ordering::SeqCst) == 0
            && self.cursor.load(Ordering::SeqCst) == self.consumed.load(Ordering::SeqCst)
        {
            let mut next = swap.next.take().unwrap();
            self.log.store(&mut *next, Ordering::SeqCst);
            self.log_capacity.store(next.words.len(), Ordering::Release);
            swap.retired = swap.current.replace(next);
        }
        self.resizing.store(false, Ordering::SeqCst);
    }

    fn release(&self) -> Result<(), &'static str> {
        if self.enabled.load(Ordering::SeqCst)
            || self.active.load(Ordering::SeqCst) != 0
            || self.producers.load(Ordering::SeqCst) != 0
            || self.tap_in_flight.load(Ordering::SeqCst) != 0
        {
            return Err("land event storage release before callback quiescence");
        }
        let buffers = {
            let mut swap = self.log_swap.lock().unwrap_or_else(|error| error.into_inner());
            self.log.store(ptr::null_mut(), Ordering::SeqCst);
            self.log_capacity.store(0, Ordering::Release);
            self.log_required.store(0, Ordering::Relaxed);
            self.base_words.store(0, Ordering::Relaxed);
            self.observer_words.store(0, Ordering::Relaxed);
            self.consumed.store(self.cursor.load(Ordering::SeqCst), Ordering::SeqCst);
            self.storage_lost.store(u64::MAX, Ordering::SeqCst);
            drop(self.spill.take());
            (std::mem::take(&mut *swap), std::mem::take(&mut *self.spilled()))
        };
        drop(buffers);
        Ok(())
    }

    fn log(&self) -> Option<&Storage> {
        unsafe { self.log.load(Ordering::SeqCst).as_ref() }
    }
    fn escaped_unwind_reason(&self) -> Option<&'static str> {
        if self.tap_unwinds.load(Ordering::SeqCst) != 0 {
            Some(TAP_UNWIND_RETENTION)
        } else if self.call_escaped_unwinds.load(Ordering::SeqCst) != 0 {
            Some(CALL_UNWIND_RETENTION)
        } else {
            None
        }
    }
    fn append_at(
        &self,
        size: usize,
        copy: impl FnOnce(u64, &mut dyn FnMut(u64)) -> Result<(), &'static str>,
    ) {
        if self.escaped_unwind_reason().is_some() {
            return;
        }
        struct Producer<'a> {
            observer: &'a Observer,
            uncommitted: bool,
        }
        impl Drop for Producer<'_> {
            fn drop(&mut self) {
                if self.uncommitted {
                    self.observer.fail();
                }
                self.observer.producers.fetch_sub(1, Ordering::SeqCst);
            }
        }
        self.producers.fetch_add(1, Ordering::SeqCst);
        let mut producer = Producer { observer: self, uncommitted: false };
        let first = self.cursor.fetch_add(size as u64, Ordering::SeqCst);
        let consumed = self.consumed.load(Ordering::SeqCst);
        let required = self.cursor.load(Ordering::SeqCst).wrapping_sub(consumed) as usize;
        self.log_required.fetch_max(required.max(size), Ordering::Relaxed);
        let end = first.wrapping_add(size as u64).wrapping_sub(consumed);
        let ring = (!self.resizing.load(Ordering::SeqCst))
            .then(|| self.log())
            .flatten()
            .filter(|log| end <= log.words.len() as u64);
        let Some(log) = ring else {
            let mut words = Vec::new();
            if engine_allocation::advisory(|| engine_allocation::reserve_exact(&mut words, size.max(1)))
                .is_err()
            {
                self.storage_lost.fetch_min(first, Ordering::SeqCst);
                self.fail();
                return;
            }
            words.push(0);
            producer.uncommitted = true;
            let mut written = 1;
            let result = copy(first, &mut |word| {
                if written < size {
                    words.push(word);
                }
                written += 1;
            });
            producer.uncommitted = false;
            if result.is_err() || written != size {
                self.fail();
                return;
            }
            words[0] = size as u64;
            if self.spill.push((first, words)).is_err() {
                self.storage_lost.fetch_min(first, Ordering::SeqCst);
                self.fail();
            }
            return;
        };
        let capacity = log.words.len() as u64;
        producer.uncommitted = true;
        log.words[(first % capacity) as usize].store(0, Ordering::Relaxed);
        let mut written = 1;
        let result = copy(first, &mut |word| {
            if written < size {
                log.words[(first.wrapping_add(written as u64) % capacity) as usize]
                    .store(word, Ordering::Relaxed);
            }
            written += 1;
        });
        producer.uncommitted = false;
        let lapped = self.consumed.load(Ordering::SeqCst) > first
            && self.cursor.load(Ordering::SeqCst).wrapping_sub(first) > capacity;
        if result.is_err() || written != size || lapped {
            self.fail();
            return;
        }
        log.words[(first % capacity) as usize].store(size as u64, Ordering::Release);
    }

    pub(crate) fn lost_drain(&self, hour: i32, reason: &'static str) -> Drain {
        self.fail();
        let first = self.consumed.load(Ordering::SeqCst);
        Drain {
            epoch: self.epoch.load(Ordering::SeqCst),
            first,
            words: 0,
            required: self.cursor.load(Ordering::SeqCst).wrapping_sub(first) as usize,
            from_hour: self.last_hour.load(Ordering::Relaxed) as i32,
            to_hour: hour,
            complete: false,
            lost: self.lost.load(Ordering::Relaxed),
            reason,
        }
    }
    fn drain_buffer(
        &self,
        output: engine_native_memory::walk::Buffer<'_, u64>,
        hour: i32,
        invalidating: bool,
    ) -> Drain {
        if invalidating {
            self.advance_epoch();
        }
        let mut held = self.spilled();
        let end = self.cursor.load(Ordering::SeqCst);
        let storage_lost = self.storage_lost
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |lost| {
                (lost < end).then_some(u64::MAX)
            })
            .is_ok();
        let result = self.drain_interval(output, hour, invalidating, end, storage_lost, &mut held);
        self.take_spill(&mut held, end);
        held.sort_unstable_by_key(|&(index, _)| index);
        let taken = held.partition_point(|&(index, _)| index < end);
        held.drain(..taken);
        result
    }
    fn drain_interval(
        &self,
        mut output: engine_native_memory::walk::Buffer<'_, u64>,
        hour: i32,
        invalidating: bool,
        end: u64,
        storage_lost: bool,
        spilled: &mut Vec<(u64, u64)>,
    ) -> Drain {
        let first = self.consumed.swap(end, Ordering::SeqCst);
        let epoch = self.epoch.load(Ordering::SeqCst);
        let old_epoch = self.observed_epoch.swap(epoch, Ordering::SeqCst);
        let from = self.last_hour.swap(hour as u64, Ordering::Relaxed) as i32;
        let required = end.wrapping_sub(first) as usize;
        self.log_required.fetch_max(required, Ordering::Relaxed);
        let mut result = Drain {
            epoch,
            first,
            words: 0,
            required,
            from_hour: from,
            to_hour: hour,
            complete: false,
            lost: self.lost.load(Ordering::Relaxed),
            reason: "land event interval not observed",
        };
        if let Some(reason) = self.escaped_unwind_reason() {
            result.reason = reason;
            return result;
        }
        if !self.enabled.load(Ordering::SeqCst) {
            result.reason = "land event hooks unavailable";
            return result;
        }
        if invalidating || old_epoch == 0 {
            result.reason = "land events before first sample/load/gap unknown";
            return result;
        }
        if from.checked_add(1) != Some(hour) {
            result.epoch = self.advance_epoch();
            self.observed_epoch.store(result.epoch, Ordering::SeqCst);
            result.reason = "land event hour unavailable or discontinuous";
            return result;
        }
        if let Err(reason) = output.ensure(required) {
            self.fail();
            result.epoch = self.epoch.load(Ordering::SeqCst);
            result.lost = self.lost.load(Ordering::Relaxed);
            self.observed_epoch.store(result.epoch, Ordering::SeqCst);
            result.reason = reason;
            return result;
        }
        if required > output.len() {
            self.fail();
            result.epoch = self.epoch.load(Ordering::SeqCst);
            result.lost = self.lost.load(Ordering::Relaxed);
            self.observed_epoch.store(result.epoch, Ordering::SeqCst);
            result.reason = "land event drain capacity exceeded";
            return result;
        }
        if storage_lost {
            result.epoch = self.epoch.load(Ordering::SeqCst);
            self.observed_epoch.store(result.epoch, Ordering::SeqCst);
            result.reason = STORAGE_FAILED;
            return result;
        }
        if epoch != old_epoch {
            result.reason = "land event epoch changed after a failed read";
            return result;
        }
        if self.producers.load(Ordering::SeqCst) != 0 {
            self.fail();
            result.epoch = self.epoch.load(Ordering::SeqCst);
            result.lost = self.lost.load(Ordering::Relaxed);
            self.observed_epoch.store(result.epoch, Ordering::SeqCst);
            result.reason = "land event producer active at boundary";
            return result;
        }
        if !self.take_spill(spilled, end) {
            self.fail();
            result.epoch = self.epoch.load(Ordering::SeqCst);
            result.lost = self.lost.load(Ordering::Relaxed);
            self.observed_epoch.store(result.epoch, Ordering::SeqCst);
            result.reason = STORAGE_FAILED;
            return result;
        }
        spilled.sort_unstable_by_key(|&(index, _)| index);
        let log = self.log().filter(|log| !log.words.is_empty());
        let mut held = spilled.partition_point(|&(index, _)| index < first);
        let mut missing = false;
        for (n, word) in output[..required].iter_mut().enumerate() {
            let index = first.wrapping_add(n as u64);
            *word = match spilled.get(held) {
                Some(&(at, value)) if at == index => {
                    held += 1;
                    value
                }
                _ => match log {
                    Some(log) => {
                        log.words[(index % log.words.len() as u64) as usize].load(Ordering::Acquire)
                    }
                    None => {
                        missing = true;
                        0
                    }
                },
            };
        }
        if missing
            || self.producers.load(Ordering::SeqCst) != 0
            || self.epoch.load(Ordering::SeqCst) != epoch
            || self.cursor.load(Ordering::SeqCst) != end
        {
            self.fail();
            result.epoch = self.epoch.load(Ordering::SeqCst);
            result.lost = self.lost.load(Ordering::Relaxed);
            self.observed_epoch.store(result.epoch, Ordering::SeqCst);
            result.reason = "land event log changed during drain";
            return result;
        }
        result.complete = true;
        result.words = required;
        result
    }


    pub(crate) fn prepare(&self) {
        engine_allocation::advisory(|| self.prepare_storage());
    }

    fn presize(&self, words: usize) {
        self.base_words.fetch_max(words, Ordering::Relaxed);
        self.log_required.fetch_max(
            words.saturating_add(self.observer_words.load(Ordering::Relaxed)),
            Ordering::Relaxed,
        );
        self.prepare();
    }

    fn observe_observer_rows(&self, rows: usize) {
        let words = rows.saturating_mul(11);
        self.observer_words.fetch_max(words, Ordering::Relaxed);
        self.log_required.fetch_max(
            self.base_words.load(Ordering::Relaxed).saturating_add(words),
            Ordering::Relaxed,
        );
    }

    pub(crate) fn capacity(&self) -> usize {
        self.log_required.load(Ordering::Relaxed)
    }

    fn observing(&self) -> bool {
        self.enabled.load(Ordering::Relaxed) && self.escaped_unwind_reason().is_none()
    }

    fn stop(&self) {
        self.enabled.store(false, Ordering::SeqCst);
    }

    fn fail(&self) {
        self.lost.fetch_add(1, Ordering::Relaxed);
        self.advance_epoch();
    }

    fn advance_epoch(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub(crate) fn lost_casualty_reads(&self, count: u64) -> u64 {
        self.lost.fetch_add(count, Ordering::Relaxed).saturating_add(count)
    }

    fn append(&self, size: usize, copy: impl FnOnce(&mut dyn FnMut(u64)) -> Result<(), &'static str>) {
        self.append_at(size, |_, put| copy(put));
    }

    pub(crate) fn drain(&self, output: &mut Vec<u64>, hour: i32, invalidating: bool) -> Drain {
        self.drain_buffer(engine_native_memory::walk::Buffer::Growing(output), hour, invalidating)
    }
}
pub(crate) static LIVE: Observer = Observer::new(&TAP_IN_FLIGHT, &TAP_ESCAPED_UNWINDS);


/// # Safety
/// Run only while land-event storage remains live for the active session (or an isolated test
/// owning land-event observation) and teardown is excluded; spare allocation is serialized under
/// `LOG_SWAP`.
pub unsafe fn prepare() {
    LIVE.prepare();
}

/// # Safety
/// Run only while land-event storage remains live for the active session (or an isolated test
/// owning land-event observation) and teardown is excluded; spare allocation is serialized under
/// `LOG_SWAP`.
pub unsafe fn presize(words: usize) {
    LIVE.presize(words);
}

pub(crate) fn observe_observer_rows(rows: usize) {
    LIVE.observe_observer_rows(rows);
}

pub fn capacity() -> usize {
    LIVE.capacity()
}

/// # Safety
/// Run only at attach or the admitted hour's sole observer boundary while no competing swap or
/// teardown runs.
pub unsafe fn install_offered() {
    LIVE.install_offered();
}

/// # Safety
/// Caller must own session teardown/detach after land-event observation is disabled and callbacks
/// have quiesced (or own an isolated test fixture with no live hooks).
pub unsafe fn release() -> Result<(), &'static str> {
    LIVE.release()
}

/// # Safety
/// Caller must own session teardown/detach or isolated test cleanup for land-event observation.
pub unsafe fn stop_observation() {
    LIVE.stop();
}

/// # Safety
/// Caller must own the active session's capture, decode, or publication failure path (or an
/// isolated test fixture) when invalidating land-event observation.
pub unsafe fn fail() {
    LIVE.fail();
}

/// # Safety
/// Caller must own the active session's capture or publication failure path (or an isolated test
/// fixture) when recording a lost land-event drain.
pub unsafe fn lost_drain(hour: i32, reason: &'static str) -> Drain {
    LIVE.lost_drain(hour, reason)
}

#[cfg(test)]
fn drain(output: &mut Vec<u64>, hour: i32, invalidating: bool) -> Drain {
    LIVE.drain(output, hour, invalidating)
}

fn escaped_unwind_reason() -> Option<&'static str> {
    LIVE.escaped_unwind_reason()
}

fn observing() -> bool {
    LIVE.observing()
}

fn append(size: usize, copy: impl FnOnce(&mut dyn FnMut(u64)) -> Result<(), &'static str>) {
    LIVE.append(size, copy);
}

fn append_at(size: usize, copy: impl FnOnce(u64, &mut dyn FnMut(u64)) -> Result<(), &'static str>) {
    LIVE.append_at(size, copy);
}

pub fn active() -> usize {
    LIVE.active
        .load(Ordering::SeqCst)
        .saturating_add(TAP_IN_FLIGHT.load(Ordering::SeqCst))
}
const FINALIZE: usize = 0;
const DESTROYED: usize = 1;
const BORDER_FINALIZE: usize = 2;
const BORDER_TERMINATE: usize = 3;
const DISPATCH_ORIGINAL: usize = 4;
const WIPE_ORIGINAL: usize = 5;
const DESTRUCTION_ORIGINAL: usize = 6;
static ORIGINAL: [AtomicUsize; 7] = [const { AtomicUsize::new(0) }; 7];
#[derive(Clone, Copy)]
struct Border {
    combat: usize,
    handle: u64,
    sides: [usize; 2],
    flags: [u8; 3],
    terminating: [u8; 2],
}
const NO_BORDER: Border = Border {
    combat: 0,
    handle: 0,
    sides: [0; 2],
    flags: [0; 3],
    terminating: [255; 2],
};
thread_local! {static BORDER:Cell<Border>=const{Cell::new(NO_BORDER)};}
#[derive(Clone, Copy)]
struct Dispatch {
    combat: u64,
    sides: [usize; 2],
    leaders: [u64; 2],
    seen: [bool; 2],
    winner: u8,
}
const NO_DISPATCH: Dispatch = Dispatch {
    combat: 0,
    sides: [0; 2],
    leaders: [0; 2],
    seen: [false; 2],
    winner: 0,
};
thread_local! {static DISPATCH:Cell<Dispatch>=const{Cell::new(NO_DISPATCH)};}
thread_local! {static LAST_DISPATCHED:Cell<(usize,u64,u64)>=const{Cell::new((0,0,0))};}
fn take_dispatched(combat: usize, handle: u64) -> bool {
    LAST_DISPATCHED.replace((0, 0, 0)) == (combat, handle, LIVE.epoch.load(Ordering::SeqCst))
}
impl Dispatch {
    fn slot(self, side: usize, caller: usize, base: usize) -> Result<Option<usize>, ()> {
        if self.combat == 0 {
            return Ok(None);
        }
        let returns = native::DISPATCH_LEADER_RETURNS;
        let Some(role) = engine_sites::which(base, caller, &returns)
            .and_then(|key| returns.iter().position(|rva| *rva == key))
        else {
            return Ok(None);
        };
        let winner = usize::from(self.winner == 2);
        let n = if role == 0 { 1 - winner } else { winner };
        if self.sides[n] != side || self.seen[n] {
            return Err(());
        }
        Ok(Some(n))
    }
}
impl Border {
    fn slot(self, side: usize, caller: usize, base: usize) -> Result<Option<usize>, ()> {
        if self.combat == 0 {
            return Ok(None);
        }
        let returns = native::BORDER_TERMINATE_RETURNS;
        let Some(n) = engine_sites::which(base, caller, &returns)
            .and_then(|key| returns.iter().position(|rva| *rva == key))
        else {
            return Ok(None);
        };
        if self.sides[n] != side || self.terminating[n] != 255 {
            return Err(());
        }
        Ok(Some(n))
    }
}
#[derive(Clone, Copy, Default)]
struct Destruction {
    army: usize,
    from: usize,
    to: usize,
    division: u64,
    entry: u64,
    seen: bool,
    failed: bool,
}
thread_local! {static DESTRUCTION:Cell<Destruction>=const{Cell::new(Destruction{army:0,from:0,to:0,division:0,entry:0,seen:false,failed:false})};}

struct Call<'a> {
    observer: &'a Observer,
    completed: Cell<bool>,
    timing: collector_timing::hook_timing::Span,
}
impl Call<'static> {
    fn enter() -> Self {
        Self::enter_for(&LIVE)
    }
}

impl<'a> Call<'a> {
    fn enter_for(observer: &'a Observer) -> Self {
        let timing =
            collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::LandEvents);
        observer.active.fetch_add(1, Ordering::SeqCst);
        Self {
            observer,
            completed: Cell::new(false),
            timing,
        }
    }
    fn finish(&self) {
        self.completed.set(true);
    }
    fn original<T>(&self, original: impl FnOnce() -> T) -> T {
        self.timing.original(original)
    }
}
struct Restore<T: Copy + 'static> {
    cell: &'static std::thread::LocalKey<Cell<T>>,
    previous: T,
}
impl<T: Copy + 'static> Restore<T> {
    fn new(cell: &'static std::thread::LocalKey<Cell<T>>) -> Self {
        Self {
            cell,
            previous: cell.get(),
        }
    }
}
impl<T: Copy + 'static> Drop for Restore<T> {
    fn drop(&mut self) {
        self.cell.set(self.previous);
    }
}
impl Drop for Call<'_> {
    fn drop(&mut self) {
        if !self.completed.get() || std::thread::panicking() {
            self.observer.call_escaped_unwinds.fetch_add(1, Ordering::SeqCst);
            self.observer.enabled.store(false, Ordering::SeqCst);
            self.observer.fail();
            return;
        }
        self.observer.active.fetch_sub(1, Ordering::SeqCst);
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Drain {
    pub epoch: u64,
    pub first: u64,
    pub words: usize,
    pub required: usize,
    pub from_hour: i32,
    pub to_hour: i32,
    pub complete: bool,
    pub lost: u64,
    pub reason: &'static str,
}
fn observe(combat: usize) {
    if !observing() {
        return;
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let border = BORDER.get();
    if border.combat == combat {
        let packed = u64::from(border.flags[0])
            | u64::from(border.flags[1]) << 8
            | u64::from(border.flags[2]) << 16
            | u64::from(border.terminating[0]) << 24
            | u64::from(border.terminating[1]) << 32;
        match native::final_inputs(&memory, base, combat) {
            Ok(inputs) => {
                append(8, |put| {
                    put(2);
                    put(border.handle);
                    put(packed);
                    for word in inputs {
                        put(word);
                    }
                    Ok(())
                });
            }
            Err(_) => LIVE.fail(),
        }
    } else {
        match native::ResultPlan::read(&memory, base, combat, None) {
            Ok(plan) => {
                let plan = if take_dispatched(combat, plan.combat()) {
                    plan.without_candidates()
                } else {
                    plan
                };
                append(plan.words, |put| plan.copy(&memory, base, put));
            }
            Err(_) => LIVE.fail(),
        }
    }
}
unsafe extern "system-unwind" fn finalize(combat: usize) {
    let _call = Call::enter();
    observe(combat);
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[FINALIZE].load(Ordering::Acquire)) };
    _call.original(|| unsafe { original(combat) });
    if observing() {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        match native::LossesPlan::read(&memory, base, combat) {
            Ok(plan) => {
                append(plan.words, |put| plan.copy(&memory, base, put));
                let (handle, sides) = plan.observer_sources();
                for (side, pointer) in sides.into_iter().enumerate() {
                    if engine_land_inputs::combat_log_events::observer(
                        &memory,
                        base,
                        pointer,
                        &mut |fact| {
                            append(11, |put| {
                                put(9);
                                put(handle);
                                put(side as u64);
                                put(fact.kind);
                                put(fact.item);
                                for value in fact.values {
                                    put(value);
                                }
                                Ok(())
                            });
                        },
                    )
                    .is_err()
                    {
                        LIVE.fail();
                    }
                }
            }
            Err(_) => LIVE.fail(),
        }
        match native::HistoryPlan::read(&memory, base, combat) {
            Ok(plan) => append(plan.words, |put| plan.copy(&memory, put)),
            Err(_) => LIVE.fail(),
        }
    }
    _call.finish();
}

core::arch::global_asm!(
    ".text", ".globl land_wipe_entry", ".seh_proc land_wipe_entry",
    "land_wipe_entry:",
    "push rbp", ".seh_pushreg rbp", "lea rsp, [rsp - 0xd0]", ".seh_stackalloc 0xd0",
    "lea rbp, [rsp + 0xd0]", ".seh_setframe rbp, 0xd0", ".seh_endprologue",
    "mov [rsp + 0x20], rax", "mov [rsp + 0x28], rcx", "mov [rsp + 0x30], rdx",
    "mov [rsp + 0x38], r8", "mov [rsp + 0x40], r9", "mov [rsp + 0x48], r10", "mov [rsp + 0x50], r11",
    "movdqu [rsp + 0x60], xmm0", "movdqu [rsp + 0x70], xmm1", "movdqu [rsp + 0x80], xmm2",
    "movdqu [rsp + 0x90], xmm3", "movdqu [rsp + 0xa0], xmm4", "movdqu [rsp + 0xb0], xmm5",
    "pushfq", "pop rax", "mov [rsp + 0xc0], rax",
    "call {body}",
    "movdqu xmm0, [rsp + 0x60]", "movdqu xmm1, [rsp + 0x70]", "movdqu xmm2, [rsp + 0x80]",
    "movdqu xmm3, [rsp + 0x90]", "movdqu xmm4, [rsp + 0xa0]", "movdqu xmm5, [rsp + 0xb0]",
    "mov rax, [rsp + 0x20]", "mov rcx, [rsp + 0x28]", "mov rdx, [rsp + 0x30]",
    "mov r8, [rsp + 0x38]", "mov r9, [rsp + 0x40]", "mov r10, [rsp + 0x48]", "mov r11, [rsp + 0x50]",
    "push qword ptr [rsp + 0xc0]", "popfq",
    "lea rsp, [rbp]", "pop rbp", "jmp qword ptr [rip + {original} + {offset}]",
    ".seh_endproc", body = sym observe_wipe, original = sym ORIGINAL,
    offset = const WIPE_ORIGINAL * size_of::<AtomicUsize>());
unsafe extern "system-unwind" {
    fn land_wipe_entry();
}
unsafe extern "system-unwind" fn observe_wipe(combatant: usize, army: usize) {
    // SAFETY: This is the installed wipe entry with its original native arguments.
    unsafe { observe_wipe_for(&LIVE, combatant, army) };
}

unsafe fn observe_wipe_for(observer: &Observer, combatant: usize, army: usize) {
    let _call = Call::enter_for(observer);
    if !observer.observing() {
        _call.finish();
        return;
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    match engine_land_inputs::land_wipe::WipePlan::read(&memory, base, combatant, army) {
        Ok(Some(plan)) => observer.append(plan.words, |put| plan.copy(&memory, base, put)),
        Ok(None) => {}
        Err(_) => {
            observer.fail();
        }
    }
    #[cfg(test)]
    if WIPE_TEST_CLOBBER_XMM.load(Ordering::Relaxed) {
        unsafe {
            core::arch::asm!(
            "pxor xmm0, xmm0", "pxor xmm1, xmm1", "pxor xmm2, xmm2",
            "pxor xmm3, xmm3", "pxor xmm4, xmm4", "pxor xmm5, xmm5",
            out("xmm0") _, out("xmm1") _, out("xmm2") _,
            out("xmm3") _, out("xmm4") _, out("xmm5") _,
            options(nomem, nostack, preserves_flags));
        }
    }
    _call.finish();
}
#[cfg(test)]
static WIPE_TEST_CLOBBER_XMM: AtomicBool = AtomicBool::new(false);
unsafe extern "system-unwind" fn destroyed(
    from: usize,
    to: usize,
    army: usize,
    pool: usize,
    killer: usize,
) {
    let _call = Call::enter();
    if observing() {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        let mut context = DESTRUCTION.get();
        let matches = context.army == army && context.from == from && context.to == to;
        if !(matches && context.failed) {
            if matches && context.division != 0 && !context.seen {
                match native::killer_handle(&memory, base, killer) {
                    Ok(handle) => append(4, |put| {
                        put(6);
                        put(context.entry);
                        put(handle);
                        Ok(())
                    }),
                    Err(_) => LIVE.fail(),
                }
                context.seen = true;
                DESTRUCTION.set(context);
            } else {
                LIVE.fail();
                if matches {
                    context.failed = true;
                    DESTRUCTION.set(context);
                }
            }
        }
    }
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[DESTROYED].load(Ordering::Acquire)) };
    _call.original(|| unsafe { original(from, to, army, pool, killer) });
    _call.finish();
}
unsafe extern "system-unwind" fn destruction(army: usize, from: usize, to: usize) {
    let _call = Call::enter();
    let _restore = Restore::new(&DESTRUCTION);
    if observing() {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        match native::DestructionPlan::read(&memory, base, from, to, army, 0) {
            Ok(plan) => {
                DESTRUCTION.set(Destruction {
                    army,
                    from,
                    to,
                    division: plan.division(),
                    entry: 0,
                    seen: false,
                    failed: false,
                });
                append_at(plan.words, |first, put| {
                    let mut context = DESTRUCTION.get();
                    context.entry = first;
                    DESTRUCTION.set(context);
                    plan.copy(&memory, base, put)
                });
            }
            Err(_) => {
                LIVE.fail();
                DESTRUCTION.set(Destruction {
                    army,
                    from,
                    to,
                    failed: true,
                    ..Default::default()
                });
            }
        }
    }
    let original: unsafe extern "system-unwind" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[DESTRUCTION_ORIGINAL].load(Ordering::Acquire)) };
    _call.original(|| unsafe { original(army, from, to) });
    if observing() && !DESTRUCTION.get().seen && !DESTRUCTION.get().failed {
        LIVE.fail();
    }
    _call.finish();
}
unsafe extern "system-unwind" fn border_finalize(combat: usize) {
    let _call = Call::enter();
    let _restore = Restore::new(&BORDER);
    if observing() {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let fields = engine_native_memory::read::<[u8; 3]>(
            &memory,
            combat + engine_native_parts::cland_border_war_combat::END_FLAGS,
        )
        .zip(engine_native_memory::read::<[u8; 16]>(
            &memory,
            combat + engine_native::cland_combat::ATTACKER,
        ));
        if let Some((flags, sides)) = fields {
            BORDER.set(Border {
                combat,
                handle: engine_native_memory::read::<u64>(
                    &memory,
                    combat + engine_native::cland_combat::HANDLE,
                )
                .unwrap_or(0),
                sides: [
                    u64::from_le_bytes(sides[..8].try_into().unwrap()) as usize,
                    u64::from_le_bytes(sides[8..].try_into().unwrap()) as usize,
                ],
                flags,
                terminating: [255; 2],
            });
        } else {
            LIVE.fail();
            BORDER.set(NO_BORDER);
        }
    }
    if observing() {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        let border = BORDER.get();
        if border.combat == combat && border.handle != 0 {
            match native::ResultPlan::read(&memory, base, combat, Some((border.flags, [255; 2]))) {
                Ok(plan) => {
                    let plan = if take_dispatched(combat, plan.combat()) {
                        plan.without_candidates()
                    } else {
                        plan
                    };
                    append(plan.words, |put| plan.copy(&memory, base, put));
                }
                Err(_) => LIVE.fail(),
            }
        } else {
            LIVE.fail();
        }
    }
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[BORDER_FINALIZE].load(Ordering::Acquire)) };
    _call.original(|| unsafe { original(combat) });
    _call.finish();
}
#[unsafe(naked)]
unsafe extern "system-unwind" fn border_terminate_entry() {
    core::arch::naked_asm!("mov rdx, qword ptr [rsp]", "jmp {body}", body = sym border_terminate);
}
core::arch::global_asm!(
    ".text", ".globl leader_tap_entry", ".seh_proc leader_tap_entry",
    "leader_tap_entry:", "sub rsp, 0x28", ".seh_stackalloc 0x28", ".seh_endprologue",
    "mov r8, qword ptr [rax + {leader_slot}]",
    "lock inc qword ptr [rip + {counter}]", "mov rdx, qword ptr [rsp + 0x28]",
    "lea rdx, [rdx + 1]", "call {body}",
    "lock dec qword ptr [rip + {counter}]", "add rsp, 0x28", "ret",
    ".seh_endproc", ".globl leader_tap_entry_end", "leader_tap_entry_end:",
    counter = sym TAP_IN_FLIGHT, body = sym leader_tap_body,
    leader_slot = const native::DISPATCH_LEADER_VTABLE_SLOT);
unsafe extern "system-unwind" {
    fn leader_tap_entry();
    fn leader_tap_entry_end();
}
fn tap_safety() -> collector_hooks::hook::TapSafety {
    collector_hooks::hook::TapSafety {
        in_flight: &TAP_IN_FLIGHT,
        stub: leader_tap_entry as *const () as usize..leader_tap_entry_end as *const () as usize,
    }
}
unsafe extern "system-unwind" fn leader_tap_body(
    side: usize,
    caller: usize,
    target: usize,
) -> usize {
    struct UnwindTap(bool);
    impl Drop for UnwindTap {
        fn drop(&mut self) {
            if self.0 {
                TAP_ESCAPED_UNWINDS.fetch_add(1, Ordering::SeqCst);
                LIVE.enabled.store(false, Ordering::SeqCst);
                LIVE.fail();
            }
        }
    }
    let mut unwind = UnwindTap(true);
    let result = unsafe { leader_tap(side, caller, target) };
    unwind.0 = false;
    result
}
unsafe extern "system-unwind" fn border_terminate(side: usize, caller: usize) -> u8 {
    let _call = Call::enter();
    let original: unsafe extern "system-unwind" fn(usize) -> u8 =
        unsafe { std::mem::transmute(ORIGINAL[BORDER_TERMINATE].load(Ordering::Acquire)) };
    let result = _call.original(|| unsafe { original(side) });
    let mut border = BORDER.get();
    if observing() {
        match border.slot(side, caller, BASE.load(Ordering::Relaxed)) {
            Ok(Some(n)) => {
                border.terminating[n] = result;
                BORDER.set(border);
            }
            Ok(None) => {}
            Err(()) => LIVE.fail(),
        }
    }
    _call.finish();
    result
}

unsafe extern "system-unwind" fn dispatch(combat: usize, loser: usize, winner: usize) {
    let _call = Call::enter();
    let _restore = Restore::new(&DISPATCH);
    if observing() {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        match native::dispatch_inputs(&memory, BASE.load(Ordering::Relaxed), combat, loser, winner)
        {
            Ok((header, winner)) => {
                DISPATCH.set(Dispatch {
                    combat: header.handle(),
                    sides: header.sides(),
                    winner,
                    ..NO_DISPATCH
                });
                let verdict = if header.border_war() {
                    Some(([0; 3], [255; 2]))
                } else {
                    None
                };
                match native::ResultPlan::read_header(
                    &memory,
                    BASE.load(Ordering::Relaxed),
                    header,
                    verdict,
                ) {
                    Ok(plan) => {
                        let plan = plan.participants();
                        append(plan.words, |put| {
                            plan.copy(&memory, BASE.load(Ordering::Relaxed), put)
                        });
                    }
                    Err(_) => LIVE.fail(),
                }
            }
            Err(_) => {
                LIVE.fail();
                DISPATCH.set(NO_DISPATCH);
            }
        }
    }
    let original: unsafe extern "system-unwind" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[DISPATCH_ORIGINAL].load(Ordering::Acquire)) };
    _call.original(|| unsafe { original(combat, loser, winner) });
    let observed = DISPATCH.get();
    if observing() {
        if observed.combat != 0 && observed.seen == [true; 2] {
            LAST_DISPATCHED.set((combat, observed.combat, LIVE.epoch.load(Ordering::SeqCst)));
            append(6, |put| {
                put(4);
                put(observed.combat);
                put(observed.leaders[0]);
                put(observed.leaders[1]);
                put(u64::from(observed.winner));
                Ok(())
            });
        } else {
            LIVE.fail();
        }
    }
    _call.finish();
}
unsafe extern "system-unwind" fn leader_tap(side: usize, caller: usize, target: usize) -> usize {
    let _call = Call::enter();
    let result = unsafe { leader_observation(&_call, side, caller, target) };
    _call.finish();
    result
}
/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
unsafe fn leader_observation(call: &Call<'_>, side: usize, caller: usize, target: usize) -> usize {
    let original: unsafe extern "system-unwind" fn(usize) -> usize =
        unsafe { std::mem::transmute(target) };
    let result = call.original(|| unsafe { original(side) });
    let mut observed = DISPATCH.get();
    if observing() && observed.combat != 0 {
        match observed.slot(side, caller, BASE.load(Ordering::Relaxed)) {
            Ok(Some(n)) => {
                let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
                match native::leader_reference(&memory, BASE.load(Ordering::Relaxed), result) {
                    Ok(handle) => {
                        observed.leaders[n] = handle;
                        observed.seen[n] = true;
                        DISPATCH.set(observed);
                    }
                    Err(_) => LIVE.fail(),
                }
            }
            Ok(None) => {}
            Err(()) => LIVE.fail(),
        }
    }
    result
}

pub struct Hooks {
    hooks: Vec<Hook>,
    refused: bool,
    publication_attempted: bool,
}
impl Hooks {
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        let hooks = unsafe { Self::install(memory, base) }.unwrap_or_else(|reason| {
            collector_hooks::hook::installation::refused(
                collector_hooks::hook::installation::Kind::Construction,
                None,
                &reason,
            );
            Vec::new()
        });
        Self {
            refused: hooks.is_empty(),
            hooks,
            publication_attempted: false,
        }
    }
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    unsafe fn install(_memory: &impl Memory, base: usize) -> Result<Vec<Hook>, String> {
        let sites = [
            (
                FINALIZE,
                native::FINALIZE_RVA,
                finalize as *const () as usize,
            ),
            (
                DESTROYED,
                native::DESTROY_RVA,
                destroyed as *const () as usize,
            ),
            (
                BORDER_FINALIZE,
                native::BORDER_FINALIZE_RVA,
                border_finalize as *const () as usize,
            ),
            (
                BORDER_TERMINATE,
                native::BORDER_TERMINATE_RVA,
                border_terminate_entry as *const () as usize,
            ),
            (
                DISPATCH_ORIGINAL,
                native::DISPATCH_RVA,
                dispatch as *const () as usize,
            ),
            (
                DESTRUCTION_ORIGINAL,
                native::DESTRUCTION_RVA,
                destruction as *const () as usize,
            ),
            (
                WIPE_ORIGINAL,
                engine_land_inputs::land_wipe::LIFECYCLE_RVA,
                land_wipe_entry as *const () as usize,
            ),
        ];
        engine_sites::require(base, &native::DISPATCH_LEADER_RETURNS)
            .and_then(|()| engine_sites::require(base, &native::BORDER_TERMINATE_RETURNS))
            .map_err(|cause| format!("land event caller withheld: {cause}"))?;
        let mut located = [(0, [0; 16]); 7];
        for ((_, rva, _), slot) in sites.into_iter().zip(&mut located) {
            *slot = collector_hooks::located_hook(base, rva)
                .map_err(|cause| format!("land event site withheld: {cause}"))?;
        }
        let taps = verify_leader_taps(base)?;
        BASE.store(base, Ordering::Relaxed);
        let mut hooks = Vec::new();
        for ((slot, _, callback), (at, original)) in sites.into_iter().zip(located) {
            let hook = if slot == WIPE_ORIGINAL {
                unsafe { Hook::new_push_frame(at, callback, original) }?
            } else {
                unsafe { Hook::new(at, callback, original) }?
            };
            ORIGINAL[slot].store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
        }
        for (at, original) in taps {
            hooks.push(unsafe {
                Hook::new_indirect_call(
                    at,
                    leader_tap_entry as *const () as usize,
                    original,
                    tap_safety(),
                )
            }?);
        }
        Ok(hooks)
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        self.publication_attempted |= enabled;
        LIVE.enabled.store(false, Ordering::SeqCst);
        LIVE.advance_epoch();
        if let Some(reason) = escaped_unwind_reason().filter(|_| enabled) {
            return Err(reason.into());
        }
        if enabled && self.refused {
            return Ok(true);
        }
        match Hook::set_family_enabled(&mut self.hooks, enabled) {
            Ok(false) => return Ok(false),
            Err(reason)
                if enabled && reason == collector_hooks::hook::BYTE_VERIFICATION_FAILURE =>
            {
                self.refused = true;
                collector_hooks::hook::installation::refused(
                    collector_hooks::hook::installation::Kind::Publication,
                    None,
                    &reason,
                );
                return Ok(true);
            }
            Err(reason) => return Err(reason),
            Ok(true) => {}
        }
        #[cfg(test)]
        tests::escape_after_publication();
        LIVE.enabled.store(enabled && self.hooks.len() == 9, Ordering::SeqCst);
        if let Some(reason) = escaped_unwind_reason().filter(|_| enabled) {
            LIVE.enabled.store(false, Ordering::SeqCst);
            return Err(reason.into());
        }
        Ok(true)
    }
    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(std::iter::once(tap_safety().stub))
            .collect()
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        if !self.publication_attempted {
            // SAFETY: Publication was never attempted, so hooks were never enabled and no
            // callbacks are active.
            let _ = unsafe { release() };
        }
    }
}

fn verify_leader_taps(base: usize) -> Result<[(usize, [u8; 16]); 2], String> {
    let mut taps = [(0, [0; 16]); 2];
    for (n, &rva) in native::DISPATCH_LEADER_CALLS.iter().enumerate() {
        let located = engine_sites::hook(base, rva).and_then(|code| {
            collector_hooks::hook::installation::site(code.address);
            if code.sentinel::<6>()? != native::DISPATCH_LEADER_CALL_SENTINEL {
                return Err("land event leader call is not the expected vtable-slot CALL");
            }
            Ok((code.address, code.sentinel::<16>()?))
        });
        taps[n] = located.map_err(|cause| {
            collector_hooks::hook::installation::refused(
                collector_hooks::hook::installation::Kind::Sentinel,
                None,
                cause,
            );
            format!(
                "land event call tap {} withheld: {cause}",
                native::DISPATCH_LEADER_CALL_NAMES[n]
            )
        })?;
    }
    Ok(taps)
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;

    pub fn with_active_observer(check: impl FnOnce()) {
        let call = Call::enter();
        check();
        call.finish();
    }

    pub(super) static EXCLUSIVE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub struct Owned {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    pub fn own() -> Owned {
        Owned { _lock: EXCLUSIVE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) }
    }

    pub fn exclusive() -> (std::sync::MutexGuard<'static, ()>, Owned) {
        let family = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (family, own())
    }
    /// # Safety
    /// Caller holds exclusive fixture ownership (`_owned`) with no live native land-event hooks
    /// installed or active.
    pub unsafe fn reset(owned: &mut Owned) {
        // SAFETY: Caller upholds the reset safety contract with exclusive `Owned` access.
        unsafe { reset_with_words(owned, 1024) };
    }
    /// # Safety
    /// Caller holds exclusive fixture ownership (`_owned`) with no live native land-event hooks
    /// installed or active.
    pub unsafe fn reset_with_words(_owned: &mut Owned, words: usize) {
        let mut swap = LIVE.log_swap.lock().unwrap();
        swap.next = None;
        swap.retired = None;
        let mut storage = Box::new(Storage::new(
            (0..words).map(|_| AtomicU64::new(0)).collect(),
            Default::default(),
        ));
        LIVE.log.store(&mut *storage, Ordering::SeqCst);
        LIVE.log_capacity.store(words, Ordering::SeqCst);
        LIVE.log_required.store(0, Ordering::SeqCst);
        LIVE.base_words.store(0, Ordering::SeqCst);
        LIVE.observer_words.store(0, Ordering::SeqCst);
        LIVE.resizing.store(false, Ordering::SeqCst);
        free_spill();
        LIVE.storage_lost.store(u64::MAX, Ordering::SeqCst);
        swap.current = Some(storage);
        LIVE.cursor.store(0, Ordering::SeqCst);
        LIVE.consumed.store(0, Ordering::SeqCst);
        LIVE.epoch.store(1, Ordering::SeqCst);
        LIVE.observed_epoch.store(0, Ordering::SeqCst);
        LIVE.last_hour.store(0, Ordering::SeqCst);
        LIVE.lost.store(0, Ordering::SeqCst);
        LIVE.active.store(0, Ordering::SeqCst);
        LIVE.call_escaped_unwinds.store(0, Ordering::SeqCst);
        LIVE.producers.store(0, Ordering::SeqCst);
        LAST_DISPATCHED.set((0, 0, 0));
        LIVE.enabled.store(true, Ordering::SeqCst);
    }
    pub fn free_spill() {
        drop(LIVE.spill.take());
        drop(std::mem::take(&mut *LIVE.spilled()));
    }
    #[cfg(test)]
    pub(crate) fn append_record(payload: &[u64]) {
        append(payload.len() + 1, |put| {
            for &word in payload {
                put(word);
            }
            Ok(())
        });
    }
    pub fn disable() {
        LIVE.enabled.store(false, Ordering::SeqCst);
    }



    pub fn lost() -> u64 {
        LIVE.lost.load(Ordering::Relaxed)
    }

    pub fn epoch() -> u64 {
        LIVE.epoch.load(Ordering::SeqCst)
    }

    impl Owned {
        pub fn production_enabled(&self) -> bool {
            LIVE.enabled.load(Ordering::SeqCst)
        }

        pub fn required_words(&self) -> usize {
            LIVE.log_required.load(Ordering::Relaxed)
        }

        pub fn observe_while_disabled(&self) {
            assert!(!self.production_enabled());
            // SAFETY: A disabled observer never follows either native pointer.
            unsafe { observe_wipe(0, 0) };
        }
    }

    pub struct Fixture {
        pub(crate) observer: Observer,
    }

    impl Fixture {
        pub fn new(words: usize) -> Self {
            static NO_TAPS: AtomicUsize = AtomicUsize::new(0);
            static NO_UNWINDS: AtomicU64 = AtomicU64::new(0);
            let observer = Observer::new(&NO_TAPS, &NO_UNWINDS);
            let mut storage = Box::new(Storage::new(
                (0..words).map(|_| AtomicU64::new(0)).collect(),
                Default::default(),
            ));
            observer.log.store(&mut *storage, Ordering::SeqCst);
            observer.log_capacity.store(words, Ordering::SeqCst);
            observer.log_swap.lock().unwrap().current = Some(storage);
            observer.enabled.store(true, Ordering::SeqCst);
            Self { observer }
        }

        pub fn stop(&self) {
            self.observer.stop();
        }

        pub fn production_enabled(&self) -> bool {
            self.observer.enabled.load(Ordering::SeqCst)
        }

        pub fn lost(&self) -> u64 {
            self.observer.lost.load(Ordering::Relaxed)
        }

        pub fn required_words(&self) -> usize {
            self.observer.capacity()
        }

        pub fn observe_while_disabled(&self) {
            assert!(!self.production_enabled());
            // SAFETY: A disabled fixture never follows either native pointer.
            unsafe { observe_wipe_for(&self.observer, 0, 0) }
        }

        #[cfg(test)]
        pub(crate) fn append_record(&self, payload: &[u64]) {
            self.observer.append(payload.len() + 1, |put| {
                for &word in payload {
                    put(word);
                }
                Ok(())
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::{exclusive, reset};

    static ESCAPE_AFTER_PUBLICATION: AtomicBool = AtomicBool::new(false);
    pub(super) fn escape_after_publication() {
        if !ESCAPE_AFTER_PUBLICATION.swap(false, Ordering::SeqCst) {
            return;
        }
        unsafe extern "system-unwind" fn unwind(_: usize) -> usize {
            panic!("direct synthetic enable-race unwind");
        }
        TAP_IN_FLIGHT.store(1, Ordering::SeqCst);
        assert!(
            std::panic::catch_unwind(|| unsafe {
                leader_tap_body(0, 0, unwind as *const () as usize)
            })
            .is_err()
        );
    }
    #[test]
    fn losses_count_on_the_thread_that_records_them() {
        let owner = test_support::Fixture::new(8);
        let other = test_support::Fixture::new(8);
        let before = owner.lost();
        collector_test_support::with_allocations_denied(|| {
            owner.observer.fail();
            owner.observer.fail();
        });
        assert_eq!(owner.lost(), before + 2);
        let before_other = other.lost();
        let after_other = std::thread::spawn(move || {
            other.observer.fail();
            other.lost()
        })
        .join()
        .unwrap();
        assert!(after_other > before_other);
        assert_eq!(owner.lost(), before + 2);
    }

    #[test]
    fn escaped_resolver_racing_final_enable_cannot_reopen_observation() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualFree,
        };
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        let escaped = TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst);
        assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 0);
        let page = unsafe {
            VirtualAlloc(
                ptr::null(),
                4096,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            )
        }
        .cast::<u8>();
        assert!(!page.is_null());
        let sentinel = [0x90; 16];
        let mut hooks = Vec::new();
        for index in 0..9 {
            let site = unsafe { page.add(index * 32) };
            unsafe {
                ptr::copy_nonoverlapping(sentinel.as_ptr(), site, sentinel.len());
            }
            hooks.push(
                unsafe { Hook::new(site as usize, finalize as *const () as usize, sentinel) }
                    .unwrap(),
            );
        }
        let mut group = Hooks {
            hooks,
            refused: false,
            publication_attempted: false,
        };
        ESCAPE_AFTER_PUBLICATION.store(true, Ordering::SeqCst);
        assert_eq!(group.set_enabled(true), Err(TAP_UNWIND_RETENTION.into()));
        assert!(!LIVE.enabled.load(Ordering::SeqCst));
        LIVE.enabled.store(true, Ordering::SeqCst);
        assert!(!observing());
        let cursor = LIVE.cursor.load(Ordering::SeqCst);
        append(2, |_| {
            panic!("retained unwind must refuse before reading payload")
        });
        assert_eq!(LIVE.cursor.load(Ordering::SeqCst), cursor);
        LIVE.enabled.store(false, Ordering::SeqCst);
        assert_eq!(LIVE.active.load(Ordering::SeqCst), 1);
        assert_eq!(LIVE.call_escaped_unwinds.load(Ordering::SeqCst), 1);
        TAP_IN_FLIGHT.store(0, Ordering::SeqCst);
        TAP_ESCAPED_UNWINDS.store(escaped, Ordering::SeqCst);
        LIVE.active.store(0, Ordering::SeqCst);
        LIVE.call_escaped_unwinds.store(0, Ordering::SeqCst);
        assert!(group.set_enabled(false).unwrap());
        drop(group);
        unsafe { release() }.unwrap();
        assert_ne!(unsafe { VirtualFree(page.cast(), 0, MEM_RELEASE) }, 0);
    }
    #[test]
    fn event_storage_charge_follows_offers_and_quiet_release() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        test_support::disable();
        unsafe { release() }.unwrap();
        static TRACKER: collector_primitives::raw_memory::Tracker =
            collector_primitives::raw_memory::Tracker::new();
        let buffer = |size| {
            Box::new(Storage::new(
                (0..size).map(|_| AtomicU64::new(0)).collect(),
                collector_primitives::raw_memory::Charge::empty_on(&TRACKER),
            ))
        };
        let mut swap = LIVE.log_swap.lock().unwrap();
        swap.current = Some(buffer(4));
        swap.next = Some(buffer(8));
        swap.retired = Some(buffer(2));
        LIVE.log.store(swap.current.as_deref_mut().unwrap(), Ordering::SeqCst);
        drop(swap);
        assert_eq!(TRACKER.snapshot().current.payload_vectors, 14 * 8);
        LIVE.active.store(1, Ordering::SeqCst);
        // SAFETY: The isolated fixture tests that release refuses a live observer.
        assert!(unsafe { release() }.is_err());
        assert_eq!(TRACKER.snapshot().current.payload_vectors, 14 * 8);
        LIVE.active.store(0, Ordering::SeqCst);
        // SAFETY: This test holds exclusive fixture control of the land observer.
        unsafe { release() }.unwrap();
        assert!(LIVE.log.load(Ordering::SeqCst).is_null());
        assert_eq!(TRACKER.snapshot().current, Default::default());
        LIVE.log_swap.lock().unwrap().next = Some(buffer(3));
        drop(Hooks {
            hooks: vec![],
            refused: true,
            publication_attempted: false,
        });
        assert_eq!(TRACKER.snapshot().current, Default::default());
    }
    #[test]
    fn land_restore_retries_original_bytes_pending_verification() {
        let _guard = exclusive();
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualFree,
        };
        let page = unsafe {
            VirtualAlloc(
                ptr::null(),
                4096,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            )
        }
        .cast::<u8>();
        assert!(!page.is_null());
        let sentinel = [0x90; 16];
        unsafe {
            ptr::copy_nonoverlapping(sentinel.as_ptr(), page, 16);
        }
        for refused in [false, true] {
            let mut hook =
                unsafe { Hook::new(page as usize, finalize as *const () as usize, sentinel) }
                    .unwrap();
            collector_hooks::hook::fail_flushes(2);
            assert!(hook.set_enabled(true).is_err());
            assert!(hook.is_enabled());
            assert_eq!(
                unsafe { ptr::read_unaligned(page.cast::<[u8; 16]>()) },
                sentinel
            );
            let mut group = Hooks {
                hooks: vec![hook],
                refused,
                publication_attempted: true,
            };
            unsafe {
                *page.add(8) = 0xcc;
            }
            if refused {
                assert_eq!(group.set_enabled(true), Ok(true));
            }
            let refused_restore = group.set_enabled(false).is_err();
            unsafe {
                *page.add(8) = sentinel[8];
            }
            let restored = group.set_enabled(false);
            assert!(refused_restore);
            assert_eq!(restored, Ok(true));
            drop(group);
        }
        unsafe {
            VirtualFree(page.cast(), 0, MEM_RELEASE);
        }
    }
    #[test]
    fn verdict_tap_disable_refuses_while_suspended_thread_is_inside_target() {
        let _guard = exclusive();
        static ENTERED: AtomicBool = AtomicBool::new(false);
        static RELEASE: AtomicBool = AtomicBool::new(false);
        unsafe extern "system-unwind" fn target(side: usize) -> usize {
            ENTERED.store(true, Ordering::Release);
            while !RELEASE.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
            side + 1
        }
        let enabled = LIVE.enabled.swap(false, Ordering::SeqCst);
        ENTERED.store(false, Ordering::SeqCst);
        RELEASE.store(false, Ordering::SeqCst);
        let site = land_leader_tap_test_site as *const () as usize;
        let sentinel = unsafe { ptr::read_unaligned(site as *const [u8; 16]) };
        let mut hook = unsafe {
            Hook::new_indirect_call(
                site,
                leader_tap_entry as *const () as usize,
                sentinel,
                tap_safety(),
            )
        }
        .unwrap();
        assert!(hook.set_enabled(true).unwrap());
        let worker = std::thread::spawn(|| {

            let mut vtable = [0usize; 30];
            vtable[29] = target as *const () as usize;
            let mut preserved = [0usize; 8];
            unsafe {
                land_leader_tap_test_caller(41, vtable.as_ptr() as usize, preserved.as_mut_ptr())
            }
        });
        while !ENTERED.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        let refused = hook.set_enabled(false);
        let still_enabled = hook.is_enabled();
        let in_flight = TAP_IN_FLIGHT.load(Ordering::SeqCst);
        RELEASE.store(true, Ordering::Release);
        let result = worker.join().unwrap();
        assert_eq!(refused, Ok(false));
        assert!(still_enabled);
        assert_eq!(in_flight, 1);
        assert_eq!(result, 42);
        assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 0);
        assert!(hook.set_enabled(false).unwrap());
        assert_eq!(
            unsafe { ptr::read_unaligned(site as *const [u8; 16]) },
            sentinel
        );
        LIVE.enabled.store(enabled, Ordering::SeqCst);
    }
    #[test]
    fn verdict_tap_stub_has_seh_stack_unwind_metadata() {
        use windows_sys::Win32::System::Diagnostics::Debug::RtlLookupFunctionEntry;
        let stub = tap_safety().stub;
        let code = unsafe { std::slice::from_raw_parts(stub.start as *const u8, 12) };
        assert_eq!(
            &code[4..11],
            &[0x4c, 0x8b, 0x80, 0xe8, 0, 0, 0],
            "faultable target load precedes count"
        );
        assert_eq!(code[11], 0xf0, "lock inc follows the load");
        let mut base = 0;
        let entry =
            unsafe { RtlLookupFunctionEntry((stub.start + 4) as u64, &mut base, ptr::null_mut()) };
        assert!(!entry.is_null());
        let entry = unsafe { &*entry };
        assert_eq!(base as usize + entry.BeginAddress as usize, stub.start);
        assert_eq!(base as usize + entry.EndAddress as usize, stub.end);
        let info = base as usize + unsafe { entry.Anonymous.UnwindInfoAddress } as usize;
        let info = unsafe { std::slice::from_raw_parts(info as *const u8, 6) };
        assert_eq!(info[0] & 7, 1);
        assert_eq!(&info[1..3], &[4, 1]);
        assert_eq!(&info[4..6], &[4, 0x42]);
    }
    static IP_ENTERED: AtomicBool = AtomicBool::new(false);
    static IP_RELEASE: AtomicBool = AtomicBool::new(false);
    core::arch::global_asm!(
        ".text", ".globl land_tap_ip_test_stub", ".seh_proc land_tap_ip_test_stub",
        "land_tap_ip_test_stub:", ".seh_endprologue",
        "mov byte ptr [rip + {entered}], 1",
        ".Lland_tap_ip_wait:", "cmp byte ptr [rip + {release}], 0",
        "je .Lland_tap_ip_wait", "ret", ".seh_endproc",
        ".globl land_tap_ip_test_stub_end", "land_tap_ip_test_stub_end:",
        entered = sym IP_ENTERED, release = sym IP_RELEASE);
    unsafe extern "system" {
        fn land_tap_ip_test_stub();
        fn land_tap_ip_test_stub_end();
    }
    #[test]
    fn verdict_tap_disable_refuses_relay_and_stub_ips_with_zero_counter() {
        use windows_sys::Win32::System::Memory::PAGE_READWRITE;
        use windows_sys::Win32::System::{
            Diagnostics::Debug::FlushInstructionCache, Memory::VirtualProtect,
            Threading::GetCurrentProcess,
        };
        let _guard = exclusive();
        let site = land_leader_tap_test_site as *const () as usize;
        let sentinel = unsafe { ptr::read_unaligned(site as *const [u8; 16]) };
        let stub = land_tap_ip_test_stub as *const () as usize;
        let mut hook = unsafe {
            Hook::new_indirect_call(
                site,
                stub,
                sentinel,
                collector_hooks::hook::TapSafety {
                    in_flight: &TAP_IN_FLIGHT,
                    stub: stub..land_tap_ip_test_stub_end as *const () as usize,
                },
            )
        }
        .unwrap();
        let relay_loop = hook.executable_ranges()[0].start + 32;
        let mut bytes = [0u8; 29];
        bytes[..2].copy_from_slice(&[0x48, 0xb8]);
        bytes[2..10].copy_from_slice(&((&IP_ENTERED as *const _ as usize) as u64).to_le_bytes());
        bytes[10..15].copy_from_slice(&[0xc6, 0x00, 1, 0x48, 0xb8]);
        bytes[15..23].copy_from_slice(&((&IP_RELEASE as *const _ as usize) as u64).to_le_bytes());
        bytes[23..].copy_from_slice(&[0x80, 0x38, 0, 0x74, 0xfb, 0xc3]);
        let mut protection = 0;
        assert_ne!(
            unsafe {
                VirtualProtect(
                    relay_loop as *const _,
                    bytes.len(),
                    PAGE_READWRITE,
                    &mut protection,
                )
            },
            0
        );
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), relay_loop as *mut u8, bytes.len());
        }
        let mut ignored = 0;
        assert_ne!(
            unsafe {
                VirtualProtect(
                    relay_loop as *const _,
                    bytes.len(),
                    protection,
                    &mut ignored,
                )
            },
            0
        );
        assert_ne!(
            unsafe {
                FlushInstructionCache(GetCurrentProcess(), relay_loop as *const _, bytes.len())
            },
            0
        );
        for address in [relay_loop, stub] {
            IP_ENTERED.store(false, Ordering::SeqCst);
            IP_RELEASE.store(false, Ordering::SeqCst);
            assert!(hook.set_enabled(true).unwrap());
            let patched = unsafe { ptr::read_unaligned(site as *const [u8; 16]) };
            let worker = std::thread::spawn(move || {

                let wait: unsafe extern "system" fn() = unsafe { std::mem::transmute(address) };
                unsafe {
                    wait();
                }
            });
            while !IP_ENTERED.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            let in_flight = TAP_IN_FLIGHT.load(Ordering::SeqCst);
            let refused = hook.set_enabled(false);
            let still_enabled = hook.is_enabled();
            let unchanged = unsafe { ptr::read_unaligned(site as *const [u8; 16]) } == patched;
            IP_RELEASE.store(true, Ordering::Release);
            worker.join().unwrap();
            assert_eq!(in_flight, 0);
            assert_eq!(refused, Ok(false));
            assert!(still_enabled && unchanged);
            assert!(hook.set_enabled(false).unwrap());
            assert_eq!(
                unsafe { ptr::read_unaligned(site as *const [u8; 16]) },
                sentinel
            );
        }
    }
    #[test]
    fn repeated_division_killer_records_use_the_reserved_destruction_entry() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        unsafe extern "system-unwind" fn writer(_: usize, _: usize, _: usize, _: usize, _: usize) {}
        let original = ORIGINAL[DESTROYED].swap(writer as *const () as usize, Ordering::SeqCst);
        let _restore = Restore::new(&DESTRUCTION);
        let mut output = vec![0; 32];
        drain(&mut output, 10, true);
        let lost = test_support::lost();
        for _ in 0..2 {
            append_at(10, |entry, put| {
                DESTRUCTION.set(Destruction {
                    army: 11,
                    from: 12,
                    to: 13,
                    division: 71,
                    entry,
                    ..Default::default()
                });
                for word in [1, 71, 1, 2, 0, 12, 13, 11, 0] {
                    put(word);
                }
                Ok(())
            });
            unsafe {
                destroyed(12, 13, 11, 0, 0);
            }
        }
        let observed = drain(&mut output, 11, false);
        ORIGINAL[DESTROYED].store(original, Ordering::SeqCst);
        test_support::disable();
        assert!(observed.complete);
        assert_eq!(test_support::lost(), lost);
        assert_eq!(observed.words, 28);
        assert_eq!(&output[10..14], &[4, 6, 0, 0]);
        assert_eq!(&output[24..28], &[4, 6, 14, 0]);
        assert_eq!(LIVE.active.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn failed_destruction_read_is_counted_once_across_nested_writer() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        unsafe extern "system-unwind" fn writer(_: usize, _: usize, _: usize, _: usize, _: usize) {}
        unsafe extern "system-unwind" fn original(army: usize, from: usize, to: usize) {
            unsafe {
                destroyed(from, to, army, 0, 0);
            }
        }
        let base = BASE.swap(0, Ordering::SeqCst);
        let outer =
            ORIGINAL[DESTRUCTION_ORIGINAL].swap(original as *const () as usize, Ordering::SeqCst);
        let inner = ORIGINAL[DESTROYED].swap(writer as *const () as usize, Ordering::SeqCst);
        let lost = test_support::lost();
        unsafe {
            destruction(0, 0, 0);
        }
        assert_eq!(test_support::lost() - lost, 1);
        assert_eq!(LIVE.active.load(Ordering::SeqCst), 0);
        ORIGINAL[DESTRUCTION_ORIGINAL].store(outer, Ordering::SeqCst);
        ORIGINAL[DESTROYED].store(inner, Ordering::SeqCst);
        BASE.store(base, Ordering::SeqCst);
        test_support::disable();
    }
    #[test]
    fn late_enable_verification_failure_withholds_only_land_and_rolls_back_prefix() {
        let _guard = exclusive();
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualFree,
        };
        let page = unsafe {
            VirtualAlloc(
                ptr::null(),
                4096,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            )
        }
        .cast::<u8>();
        assert!(!page.is_null());
        let sentinel = [0x90; 16];
        unsafe {
            ptr::copy_nonoverlapping(sentinel.as_ptr(), page, 16);
            ptr::copy_nonoverlapping(sentinel.as_ptr(), page.add(64), 16);
        }
        let one =
            unsafe { Hook::new(page as usize, finalize as *const () as usize, sentinel) }.unwrap();
        let two = unsafe {
            Hook::new(
                page.add(64) as usize,
                finalize as *const () as usize,
                sentinel,
            )
        }
        .unwrap();
        unsafe {
            *page.add(79) = 0xcc;
        }
        let mut group = Hooks {
            hooks: vec![one, two],
            refused: false,
            publication_attempted: false,
        };
        assert!(
            group.set_enabled(true).unwrap(),
            "collector install may continue"
        );
        assert!(group.refused);
        assert!(!LIVE.enabled.load(Ordering::SeqCst));
        assert!(group.hooks.iter().all(|h| !h.is_enabled()));
        assert_eq!(unsafe { std::slice::from_raw_parts(page, 16) }, sentinel);
        assert_eq!(
            unsafe { *page.add(79) },
            0xcc,
            "mismatching site must not be patched"
        );
        assert!(group.set_enabled(true).unwrap());
        assert!(group.set_enabled(false).unwrap());
        drop(group);
        unsafe {
            VirtualFree(page.cast(), 0, MEM_RELEASE);
        }
    }
    #[test]
    fn terminal_losses_are_read_after_native_pending_damage_flush() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        let world = crate::extended::combat_progress::tests::World::new(1);
        unsafe extern "system-unwind" fn flush(combat: usize) {
            let side = unsafe {
                ptr::read_unaligned(
                    (combat + engine_native::cland_combat::ATTACKER) as *const usize,
                )
            };
            let observer = side + engine_native::cland_combatant::STATS_OBSERVER;
            unsafe {
                ptr::write_unaligned(
                    (observer + engine_land_inputs::cstats_observer::MANPOWER_LOSS) as *mut i32,
                    73,
                );
            }
            let entries = unsafe {
                ptr::read_unaligned(
                    (observer
                        + engine_land_inputs::cstats_observer::EQUIPMENT_LOSSES
                        + engine_native::cequipment_variant::pool::ITEMS)
                        as *const usize,
                )
            };
            unsafe {
                ptr::write_unaligned((entries + 8) as *mut i64, 234_567);
            }
        }
        let old_base = BASE.swap(0, Ordering::SeqCst);
        let old_original = ORIGINAL[FINALIZE].swap(flush as *const () as usize, Ordering::SeqCst);
        let family = collector_timing::hook_timing::Family::LandEvents;
        let _sampled = collector_timing::hook_timing::sample_all();
        let timed = collector_timing::hook_timing::counts(family);
        unsafe {
            finalize(world.combat.as_ptr() as usize);
        }
        let after = collector_timing::hook_timing::counts(family);
        assert!(after.calls > timed.calls && after.original > timed.original);
        let log = LIVE.log().unwrap();
        let words = (0..LIVE.cursor.load(Ordering::SeqCst) as usize)
            .map(|n| log.words[n].load(Ordering::Acquire))
            .collect::<Vec<_>>();
        assert_eq!(words[0], 11);
        assert_eq!(words[1], 8);
        assert_eq!(words[3], 73);
        assert_eq!(words[4], 1);
        assert_eq!(words[5], 1 << 32 | 70);
        assert_eq!(words[6], 234_567);
        assert_eq!(LIVE.active.load(Ordering::SeqCst), 0);
        ORIGINAL[FINALIZE].store(old_original, Ordering::SeqCst);
        BASE.store(old_base, Ordering::SeqCst);
        test_support::disable();
    }
    #[test]
    fn wipe_unsupported_class_refusals_keep_interval_unknown() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        let old_base = BASE.swap(0, Ordering::SeqCst);
        let mut output = vec![0; 8];
        drain(&mut output, 10, true);
        let lost = test_support::lost();
        let unknown_side = 0u64;
        let land_side = engine_native::cland_combatant::VTABLE_RVA.0 as u64;
        let unsupported_unit = [engine_native::carmy::VTABLE_RVA.0 as u64, 1];
        unsafe {
            observe_wipe(&unknown_side as *const _ as usize, 0);
            observe_wipe(
                &land_side as *const _ as usize,
                unsupported_unit.as_ptr() as usize,
            );
        }
        append(2, |_| Err("combat wipe remaining class unsupported"));
        LIVE.fail();
        LIVE.fail();
        unsafe {
            observe_wipe(0, 0);
        }
        let interval = drain(&mut output, 11, false);
        BASE.store(old_base, Ordering::SeqCst);
        test_support::disable();
        assert_eq!(test_support::lost() - lost, 6);
        assert!(interval.lost >= 6);
        assert!(!interval.complete);
    }
    #[test]
    fn win64_observation_entries_preserve_native_argument_and_return() {
        let _guard = exclusive();
        unsafe extern "system" fn commander(side: usize) -> usize {
            side.rotate_left(17)
        }
        unsafe extern "system" fn predicate(side: usize) -> u8 {
            ((side >> 40) ^ side) as u8
        }
        let enabled = LIVE.enabled.swap(false, Ordering::SeqCst);
        let old_border =
            ORIGINAL[BORDER_TERMINATE].swap(predicate as *const () as usize, Ordering::SeqCst);
        let border: unsafe extern "system" fn(usize) -> u8 =
            unsafe { std::mem::transmute(border_terminate_entry as *const ()) };
        let observed = [0, 0x8000_0001_2345_6789, usize::MAX].map(|side| unsafe {
            (
                side,
                leader_tap(side, 0, commander as *const () as usize),
                border(side),
            )
        });
        ORIGINAL[BORDER_TERMINATE].store(old_border, Ordering::SeqCst);
        LIVE.enabled.store(enabled, Ordering::SeqCst);
        assert_eq!(LIVE.active.load(Ordering::SeqCst), 0);
        for (side, actual_leader, actual_border) in observed {
            assert_eq!(actual_leader, side.rotate_left(17));
            assert_eq!(actual_border, ((side >> 40) ^ side) as u8);
        }
    }
    core::arch::global_asm!(
        ".text",
        ".globl land_leader_tap_test_caller",
        ".seh_proc land_leader_tap_test_caller",
        "land_leader_tap_test_caller:",
        "push rbx",
        ".seh_pushreg rbx",
        "push rbp",
        ".seh_pushreg rbp",
        "push rsi",
        ".seh_pushreg rsi",
        "push rdi",
        ".seh_pushreg rdi",
        "push r12",
        ".seh_pushreg r12",
        "push r13",
        ".seh_pushreg r13",
        "push r14",
        ".seh_pushreg r14",
        "push r15",
        ".seh_pushreg r15",
        "sub rsp, 0x38",
        ".seh_stackalloc 0x38",
        ".seh_endprologue",
        "mov [rsp + 0x20], r8",
        "mov rax, rdx",
        "mov rbx, 0x1101",
        "mov rbp, 0x2202",
        "mov rsi, 0x3303",
        "mov rdi, 0x4404",
        "mov r12, 0x5505",
        "mov r13, 0x6606",
        "mov r14, 0x7707",
        "mov r15, 0x8808",
        ".globl land_leader_tap_test_site",
        "land_leader_tap_test_site:",
        "call qword ptr [rax + {leader_slot}]",
        ".rept 10",
        "nop",
        ".endr",
        "mov r10, [rsp + 0x20]",
        "mov [r10], rbx",
        "mov [r10 + 8], rbp",
        "mov [r10 + 16], rsi",
        "mov [r10 + 24], rdi",
        "mov [r10 + 32], r12",
        "mov [r10 + 40], r13",
        "mov [r10 + 48], r14",
        "mov [r10 + 56], r15",
        "add rsp, 0x38",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rdi",
        "pop rsi",
        "pop rbp",
        "pop rbx",
        "ret",
        ".seh_endproc",
        leader_slot = const native::DISPATCH_LEADER_VTABLE_SLOT,
    );
    unsafe extern "system-unwind" {
        pub(super) fn land_leader_tap_test_caller(
            side: usize,
            vtable: usize,
            preserved: *mut usize,
        ) -> usize;
        pub(super) fn land_leader_tap_test_site();
    }
    core::arch::global_asm!(
        ".text", ".globl land_wipe_test_caller", ".seh_proc land_wipe_test_caller",
        "land_wipe_test_caller:", "sub rsp, 0x28", ".seh_stackalloc 0x28", ".seh_endprologue",
        "mov rax, 1", "movq xmm0, rax", "punpcklqdq xmm0, xmm0",
        "mov rax, 2", "movq xmm1, rax", "punpcklqdq xmm1, xmm1",
        "mov rax, 3", "movq xmm2, rax", "punpcklqdq xmm2, xmm2",
        "mov rax, 4", "movq xmm3, rax", "punpcklqdq xmm3, xmm3",
        "mov rax, 5", "movq xmm4, rax", "punpcklqdq xmm4, xmm4",
        "mov rax, 6", "movq xmm5, rax", "punpcklqdq xmm5, xmm5",
        "mov rax, 0x101", "mov r8, 0x808", "mov r9, 0x909", "mov r10, 0xa0a", "mov r11, 0xb0b",
        "stc", "pushfq", "pop qword ptr [rsp + 0x20]", "call {entry}",
        ".globl land_wipe_test_return", "land_wipe_test_return:",
        "mov rax, [rsp + 0x20]", "add rsp, 0x28", "ret", ".seh_endproc",
        ".globl land_wipe_test_native", "land_wipe_test_native:",
        "mov [rcx], rax", "mov [rcx + 8], rdx", "mov [rcx + 16], r8", "mov [rcx + 24], r9",
        "mov [rcx + 32], r10", "mov [rcx + 40], r11", "mov rax, [rsp]", "mov [rcx + 48], rax",
        "pushfq", "pop rax", "mov [rcx + 56], rax",
        "movdqu [rcx + 64], xmm0", "movdqu [rcx + 80], xmm1", "movdqu [rcx + 96], xmm2",
        "movdqu [rcx + 112], xmm3", "movdqu [rcx + 128], xmm4", "movdqu [rcx + 144], xmm5",
        "ret", entry = sym land_wipe_test_site);
    core::arch::global_asm!(
        ".text", ".globl land_wipe_test_site", ".seh_proc land_wipe_test_site",
        "land_wipe_test_site:",
        ".byte 0x40", "push rbx", ".seh_pushreg rbx", "push r15", ".seh_pushreg r15",
        "sub rsp, 0x58", ".seh_stackalloc 0x58", ".seh_endprologue",
        "mov rax, [rdx]", "add rsp, 0x58", "pop r15", "pop rbx", "jmp {native}", ".rept 16", "nop", ".endr", ".seh_endproc",
        native = sym land_wipe_test_native);
    unsafe extern "system-unwind" {
        fn land_wipe_test_caller(output: *mut u64, army: usize) -> u64;
        fn land_wipe_test_native();
        fn land_wipe_test_site();
        fn land_wipe_test_return();
    }
    #[test]
    fn wipe_observation_bridge_preserves_native_caller_registers_and_stack_effects() {
        let _guard = exclusive();
        let enabled = LIVE.enabled.swap(true, Ordering::SeqCst);
        let base = BASE.swap(0, Ordering::SeqCst);
        let clobber = WIPE_TEST_CLOBBER_XMM.swap(true, Ordering::SeqCst);
        let lost = test_support::lost();
        let site = land_wipe_test_site as *const () as usize;
        let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        let mut hook =
            unsafe { Hook::new_push_frame(site, land_wipe_entry as *const () as usize, sentinel) }
                .unwrap();
        let previous = ORIGINAL[WIPE_ORIGINAL].swap(hook.trampoline(), Ordering::SeqCst);
        let mut baseline = None;
        for tapped in [false, true, false] {
            assert!(hook.set_enabled(tapped).unwrap());
            let mut output = [0u64; 20];
            let army = 0x101_u64;
            let army_pointer = &army as *const u64 as usize;
            let _flags = unsafe { land_wipe_test_caller(output.as_mut_ptr(), army_pointer) };
            assert_eq!(
                &output[..6],
                &[0x101, army_pointer as u64, 0x808, 0x909, 0xa0a, 0xb0b]
            );
            assert_eq!(
                output[6],
                land_wipe_test_return as *const () as usize as u64
            );
            if let Some(expected) = baseline {
                assert_eq!(output, expected);
            } else {
                baseline = Some(output);
            }
            for n in 0..6 {
                assert_eq!(&output[8 + n * 2..10 + n * 2], &[n as u64 + 1; 2]);
            }
            assert_eq!(LIVE.active.load(Ordering::SeqCst), 0);
        }
        ORIGINAL[WIPE_ORIGINAL].store(previous, Ordering::SeqCst);
        assert_eq!(
            test_support::lost() - lost,
            1,
            "refusing observer body ran"
        );
        BASE.store(base, Ordering::SeqCst);
        WIPE_TEST_CLOBBER_XMM.store(clobber, Ordering::SeqCst);
        LIVE.enabled.store(enabled, Ordering::SeqCst);
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            sentinel
        );
    }
    #[test]
    fn win64_verdict_tap_preserves_argument_return_and_nonvolatile_integer_registers() {
        let _guard = exclusive();
        let enabled = LIVE.enabled.swap(true, Ordering::SeqCst);
        static HITS: AtomicU64 = AtomicU64::new(0);
        unsafe extern "system-unwind" fn commander(side: usize) -> usize {
            HITS.fetch_add(u64::from(TAP_IN_FLIGHT.load(Ordering::SeqCst) != 0), Ordering::Relaxed);
            side.rotate_left(17)
        }
        unsafe extern "system-unwind" fn other_commander(side: usize) -> usize {
            HITS.fetch_add(u64::from(TAP_IN_FLIGHT.load(Ordering::SeqCst) != 0), Ordering::Relaxed);
            !side
        }
        let mut vtable = [0usize; 30];
        let site = land_leader_tap_test_site as *const () as usize;
        let base = BASE.swap(
            site + 6 - native::DISPATCH_LEADER_RETURNS[0].0,
            Ordering::SeqCst,
        );
        let saved_dispatch = DISPATCH.get();
        let lost = test_support::lost();
        let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        assert_eq!(sentinel[..6], native::DISPATCH_LEADER_CALL_SENTINEL);
        let mut hook = unsafe {
            Hook::new_indirect_call(
                site,
                leader_tap_entry as *const () as usize,
                sentinel,
                tap_safety(),
            )
        }
        .unwrap();
        let hits = HITS.load(Ordering::Relaxed);
        for tapped in [false, true, false] {
            assert!(hook.set_enabled(tapped).unwrap());
            for side in [0, 0x8000_0001_2345_6789, usize::MAX] {
                for target in [
                    commander as *const () as usize,
                    other_commander as *const () as usize,
                ] {
                    DISPATCH.set(Dispatch {
                        combat: 1,
                        sides: [side.wrapping_add(1); 2],
                        ..NO_DISPATCH
                    });
                    vtable[29] = target;
                    let mut preserved = [0; 8];
                    let result = unsafe {
                        land_leader_tap_test_caller(
                            side,
                            vtable.as_ptr() as usize,
                            preserved.as_mut_ptr(),
                        )
                    };
                    assert_eq!(
                        result,
                        if target == commander as *const () as usize {
                            side.rotate_left(17)
                        } else {
                            !side
                        }
                    );
                    assert_eq!(
                        preserved,
                        [
                            0x1101, 0x2202, 0x3303, 0x4404, 0x5505, 0x6606, 0x7707, 0x8808
                        ]
                    );
                }
            }
        }
        assert_eq!(HITS.load(Ordering::Relaxed) - hits, 6);
        assert_eq!(
            test_support::lost() - lost,
            6,
            "all tapped observer bodies ran"
        );
        assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 0);
        assert_eq!(LIVE.active.load(Ordering::SeqCst), 0);
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            sentinel
        );
        LIVE.enabled.store(enabled, Ordering::SeqCst);
        BASE.store(base, Ordering::SeqCst);
        DISPATCH.set(saved_dispatch);
    }
    #[test]
    #[ignore = "print-only counted verdict-tap differential bench; owned test-process hook only"]
    fn land_leader_tap_cost() {
        let _guard = exclusive();
        unsafe extern "system-unwind" fn native_target(side: usize) -> usize {
            side
        }
        const SAMPLES: usize = 201;
        const HITS: usize = 2048;
        let enabled = LIVE.enabled.swap(true, Ordering::SeqCst);
        let site = land_leader_tap_test_site as *const () as usize;
        let base = site + 6 - native::DISPATCH_LEADER_RETURNS[0].0;
        let saved_base = BASE.swap(base, Ordering::SeqCst);
        let saved_dispatch = DISPATCH.get();
        let sentinel = unsafe { ptr::read_unaligned(site as *const [u8; 16]) };
        let mut hook = unsafe {
            Hook::new_indirect_call(
                site,
                leader_tap_entry as *const () as usize,
                sentinel,
                tap_safety(),
            )
        }
        .unwrap();
        let mut vtable = [0usize; 30];
        vtable[29] = native_target as *const () as usize;
        let mut leader = vec![0u8; engine_native::cunit_leader::TYPE + 8];
        let pointer = leader.as_mut_ptr() as usize;
        leader[..8].copy_from_slice(
            &((base + engine_native::carmy_leader::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        leader[engine_native::cunit_leader::TYPE..].copy_from_slice(&17u64.to_le_bytes());
        for with_commander in [false, true] {
            let context = if with_commander {
                Dispatch {
                    combat: 1,
                    sides: [pointer, 0],
                    winner: 2,
                    ..NO_DISPATCH
                }
            } else {
                NO_DISPATCH
            };
            let mut times: [Vec<u128>; 2] = std::array::from_fn(|_| Vec::with_capacity(SAMPLES));
            let mut preserved = [0usize; 8];
            for sample in 0..SAMPLES {
                for step in 0..2 {
                    let mode = (sample + step) % 2;
                    assert!(hook.set_enabled(mode == 1).unwrap());
                    DISPATCH.set(context);
                    assert_eq!(
                        unsafe {
                            land_leader_tap_test_caller(
                                pointer,
                                vtable.as_ptr() as usize,
                                preserved.as_mut_ptr(),
                            )
                        },
                        pointer
                    );
                    let mut result = 0;
                    let start = std::time::Instant::now();
                    for _ in 0..HITS {
                        DISPATCH.set(context);
                        result = std::hint::black_box(unsafe {
                            land_leader_tap_test_caller(
                                pointer,
                                vtable.as_ptr() as usize,
                                preserved.as_mut_ptr(),
                            )
                        });
                    }
                    let elapsed = start.elapsed().as_nanos();
                    assert_eq!(result, pointer);
                    assert_eq!(DISPATCH.get().seen[0], mode == 1 && with_commander);
                    assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 0);
                    assert_eq!(LIVE.active.load(Ordering::SeqCst), 0);
                    times[mode].push(elapsed);
                }
            }
            let mut added = times[1]
                .iter()
                .zip(&times[0])
                .map(|(tap, native)| *tap as i128 - *native as i128)
                .collect::<Vec<_>>();
            let negative_pairs = added.iter().filter(|&&delta| delta < 0).count();
            added.sort_unstable();
            for time in &mut times {
                time.sort_unstable();
            }
            let ns_per_call = |ns: u128| ns as f64 / HITS as f64;
            println!(
                "LAND_LEADER_COST mode={} samples={SAMPLES} hits_per_batch={HITS} native_p50_ns_per_call={:.3} native_p95_ns_per_call={:.3} native_max_ns_per_call={:.3} tap_p50_ns_per_call={:.3} tap_p95_ns_per_call={:.3} tap_max_ns_per_call={:.3} added_p50_ns_per_call={:.3} added_p95_ns_per_call={:.3} added_max_ns_per_call={:.3} added_min_ns_per_call={:.3} negative_pairs={negative_pairs} in_flight_atomic_ops_per_call=2 profile={} scope=synthetic_uncontended_native_vs_installed_tap_identical_caller_and_context_reset",
                if with_commander {
                    "verdict_tap_with_commander"
                } else {
                    "verdict_tap_no_dispatch"
                },
                ns_per_call(times[0][SAMPLES / 2]),
                ns_per_call(times[0][(SAMPLES - 1) * 95 / 100]),
                ns_per_call(times[0][SAMPLES - 1]),
                ns_per_call(times[1][SAMPLES / 2]),
                ns_per_call(times[1][(SAMPLES - 1) * 95 / 100]),
                ns_per_call(times[1][SAMPLES - 1]),
                added[SAMPLES / 2] as f64 / HITS as f64,
                added[(SAMPLES - 1) * 95 / 100] as f64 / HITS as f64,
                added[SAMPLES - 1] as f64 / HITS as f64,
                added[0] as f64 / HITS as f64,
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "optimized"
                }
            );
        }
        assert!(hook.set_enabled(false).unwrap());
        DISPATCH.set(saved_dispatch);
        BASE.store(saved_base, Ordering::SeqCst);
        LIVE.enabled.store(enabled, Ordering::SeqCst);
    }
    #[test]
    fn verdict_resolver_rust_unwind_retains_the_asm_owned_flight() {
        let (_family, mut owned) = exclusive();
        unsafe extern "system-unwind" fn native_unwind(_: usize) -> usize {
            panic!("synthetic resolver unwind");
        }
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 0);
        let escaped = TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst);
        let storage = LIVE.log.load(Ordering::SeqCst);
        TAP_IN_FLIGHT.store(1, Ordering::SeqCst);
        let result = std::panic::catch_unwind(|| {
            unsafe { leader_tap_body(0, 0, native_unwind as *const () as usize) };
        });
        assert!(result.is_err());
        assert_eq!(LIVE.active.load(Ordering::SeqCst), 1);
        assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 1);
        assert_eq!(TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst), escaped + 1);
        assert!(active() != 0);
        assert!(!LIVE.enabled.load(Ordering::SeqCst));
        // SAFETY: The isolated fixture tests that release refuses a live observer.
        assert!(unsafe { release() }.is_err());
        assert_eq!(LIVE.log.load(Ordering::SeqCst), storage);
        let mut out = vec![0; 32];
        for (hour, load) in [(10, false), (11, true), (12, false)] {
            let rejected = drain(&mut out, hour, load);
            assert!(!rejected.complete);
            assert_eq!(rejected.reason, TAP_UNWIND_RETENTION);
        }
        let mut group = Hooks {
            hooks: vec![],
            refused: false,
            publication_attempted: false,
        };
        assert_eq!(group.set_enabled(true), Err(TAP_UNWIND_RETENTION.into()));
        TAP_IN_FLIGHT.store(0, Ordering::SeqCst);
        TAP_ESCAPED_UNWINDS.store(escaped, Ordering::SeqCst);
        LIVE.active.store(0, Ordering::SeqCst);
        LIVE.call_escaped_unwinds.store(0, Ordering::SeqCst);
        // SAFETY: This test holds exclusive fixture control of the land observer.
        unsafe { release() }.unwrap();
    }
    #[test]
    fn original_unwind_retains_active_and_restores_nested_tls() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds exclusive fixture control of an unhooked observer.
        unsafe { reset(&mut owned) };
        let enabled = LIVE.enabled.swap(false, Ordering::SeqCst);
        unsafe extern "system-unwind" fn unwind_three(_: usize, _: usize, _: usize) {
            DISPATCH.set(NO_DISPATCH);
            BORDER.set(NO_BORDER);
            DESTRUCTION.set(Destruction::default());
            panic!("synthetic native unwind");
        }
        unsafe extern "system-unwind" fn unwind_one(_: usize) {
            BORDER.set(NO_BORDER);
            panic!("synthetic native unwind");
        }
        for (slot, callback) in [
            (DISPATCH_ORIGINAL, dispatch as *const () as usize),
            (DESTRUCTION_ORIGINAL, destruction as *const () as usize),
        ] {
            let old = ORIGINAL[slot].swap(unwind_three as *const () as usize, Ordering::SeqCst);
            DISPATCH.set(Dispatch {
                combat: 91,
                ..NO_DISPATCH
            });
            DESTRUCTION.set(Destruction {
                division: 92,
                ..Destruction::default()
            });
            let run: unsafe extern "system-unwind" fn(usize, usize, usize) =
                unsafe { std::mem::transmute(callback) };
            assert!(std::panic::catch_unwind(|| unsafe { run(0, 0, 0) }).is_err());
            assert_ne!(LIVE.active.load(Ordering::SeqCst), 0);
            if slot == DISPATCH_ORIGINAL {
                assert_eq!(DISPATCH.get().combat, 91);
            } else {
                assert_eq!(DESTRUCTION.get().division, 92);
            }
            ORIGINAL[slot].store(old, Ordering::SeqCst);
        }
        for (slot, callback) in [
            (BORDER_FINALIZE, border_finalize as *const () as usize),
            (FINALIZE, finalize as *const () as usize),
        ] {
            let old = ORIGINAL[slot].swap(unwind_one as *const () as usize, Ordering::SeqCst);
            BORDER.set(Border {
                combat: 93,
                ..NO_BORDER
            });
            let run: unsafe extern "system-unwind" fn(usize) =
                unsafe { std::mem::transmute(callback) };
            assert!(std::panic::catch_unwind(|| unsafe { run(0) }).is_err());
            if slot == BORDER_FINALIZE {
                assert_eq!(BORDER.get().combat, 93);
            }
            assert_ne!(LIVE.active.load(Ordering::SeqCst), 0);
            ORIGINAL[slot].store(old, Ordering::SeqCst);
        }
        assert_eq!(LIVE.active.load(Ordering::SeqCst), 4);
        assert!(active() != 0);
        assert!(unsafe { release() }.is_err());
        let mut output = vec![0; 8];
        for (hour, load) in [(10, false), (11, true), (12, false)] {
            let refused = drain(&mut output, hour, load);
            assert!(!refused.complete);
            assert_eq!(refused.reason, CALL_UNWIND_RETENTION);
        }
        let mut group = Hooks {
            hooks: vec![],
            refused: false,
            publication_attempted: false,
        };
        assert_eq!(group.set_enabled(true), Err(CALL_UNWIND_RETENTION.into()));
        LIVE.active.store(0, Ordering::SeqCst);
        LIVE.call_escaped_unwinds.store(0, Ordering::SeqCst);
        DISPATCH.set(NO_DISPATCH);
        BORDER.set(NO_BORDER);
        DESTRUCTION.set(Destruction::default());
        LIVE.enabled.store(enabled, Ordering::SeqCst);
    }
    #[test]
    fn append_unwind_releases_producer_and_refuses_interval() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        let mut output = vec![0; 8];
        drain(&mut output, 10, true);
        let lost = test_support::lost();
        let result = std::panic::catch_unwind(|| {
            append(4, |put| {
                put(6);
                panic!("synthetic writer unwind");
            })
        });
        assert!(result.is_err());
        assert_eq!(LIVE.producers.load(Ordering::SeqCst), 0);
        let refused = drain(&mut output, 11, false);
        assert!(!refused.complete);
        assert_eq!(test_support::lost() - lost, 1);
        assert!(refused.lost >= 1);
        LIVE.enabled.store(false, Ordering::SeqCst);
    }
    #[test]
    fn resolver_observations_require_the_native_verdict_callers() {
        let base = 0x140000000;
        for winner in [1, 2] {
            let dispatch = Dispatch {
                combat: 17,
                sides: [0x1000, 0x2000],
                winner,
                ..NO_DISPATCH
            };
            let win = usize::from(winner == 2);
            for (role, n) in [(0, 1 - win), (1, win)] {
                let caller = base + native::DISPATCH_LEADER_RETURNS[role].0;
                assert_eq!(dispatch.slot(dispatch.sides[n], caller, base), Ok(Some(n)));
                assert!(dispatch.slot(dispatch.sides[1 - n], caller, base).is_err());
                let mut seen = dispatch;
                seen.seen[n] = true;
                assert!(seen.slot(seen.sides[n], caller, base).is_err());
            }
            for side in dispatch.sides {
                assert_eq!(dispatch.slot(side, base + 0xc64e60, base), Ok(None));
            }
        }
        let border = Border {
            combat: 17,
            sides: [0x1000, 0x2000],
            ..NO_BORDER
        };
        for n in 0..2 {
            let caller = base + native::BORDER_TERMINATE_RETURNS[n].0;
            assert_eq!(border.slot(border.sides[n], caller, base), Ok(Some(n)));
            assert!(border.slot(border.sides[1 - n], caller, base).is_err());
            let mut queried = border;
            queried.terminating[n] = 0;
            assert!(queried.slot(queried.sides[n], caller, base).is_err());
            assert_eq!(
                border.slot(border.sides[n], base + 0xa4d400, base),
                Ok(None)
            );
        }
    }
    #[test]
    fn dispatched_match_is_consumed_once() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        LAST_DISPATCHED.set((0x1000, 17, 1));
        assert!(take_dispatched(0x1000, 17));
        assert!(!take_dispatched(0x1000, 17));
        LIVE.enabled.store(false, Ordering::SeqCst);
    }
    #[test]
    fn dispatched_pointer_reuse_with_a_different_handle_keeps_fallback() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        LAST_DISPATCHED.set((0x1000, 17, 1));
        assert!(!take_dispatched(0x1000, 18));
        assert!(!take_dispatched(0x1000, 17));
        LIVE.enabled.store(false, Ordering::SeqCst);
    }
    #[test]
    fn failed_dispatch_append_invalidates_the_skip_witness() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds exclusive fixture control of an unhooked observer.
        unsafe { reset(&mut owned) };
        let epoch = LIVE.epoch.load(Ordering::SeqCst);
        let lost = test_support::lost();
        LAST_DISPATCHED.set((0x1000, 17, epoch));
        append(6, |_| Err("failed dispatch append"));
        assert!(LIVE.epoch.load(Ordering::SeqCst) > epoch);
        assert_eq!(test_support::lost() - lost, 1);
        assert!(!take_dispatched(0x1000, 17));
        LIVE.enabled.store(false, Ordering::SeqCst);
    }
    #[test]
    fn first_gap_read_failure_and_busy_producer_are_unknown() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        let mut out = vec![0; 32];
        assert!(!drain(&mut out, 10, false).complete);
        assert!(drain(&mut out, 11, false).complete);
        append(4, |put| {
            put(2);
            put(19);
            put(0);
            Ok(())
        });
        assert_eq!(drain(&mut out, 12, false).words, 4);
        let lost = test_support::lost();
        append(4, |_| Err("short native read"));
        let bad = drain(&mut out, 13, false);
        assert!(!bad.complete);
        assert_eq!(test_support::lost() - lost, 1);
        assert!(bad.lost >= 1);
        assert!(drain(&mut out, 14, false).complete);
        assert!(!drain(&mut out, 20, true).complete);
        assert!(drain(&mut out, 21, false).complete);
        append(4, |put| {
            put(2);
            put(19);
            put(0);
            Ok(())
        });
        let grown = drain(&mut Vec::new(), 22, false);
        assert!(grown.complete, "{}", grown.reason);
        assert_eq!(grown.words, 4);
        drain(&mut out, 23, true);
        LIVE.producers.store(1, Ordering::SeqCst);
        assert!(!drain(&mut out, 24, false).complete);
        LIVE.producers.store(0, Ordering::SeqCst);
        LIVE.enabled.store(false, Ordering::SeqCst);
    }
    #[test]
    fn a_record_past_the_log_spills_whole_and_the_log_grows_to_it() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { test_support::reset_with_words(&mut owned, 0) };
        let demand = (1 << 20) + 17;
        let mut out = vec![0; demand];
        drain(&mut out, 10, true);
        let lost = test_support::lost();
        let epochs = test_support::epoch();
        for hour in [11, 12] {
            append(demand, |put| {
                for word in 1..demand {
                    put(word as u64);
                }
                Ok(())
            });
            let drained = drain(&mut out, hour, false);
            assert!(drained.complete, "{}", drained.reason);
            assert_eq!(drained.words, demand);
            assert_eq!(drained.required, demand);
            assert_eq!(out[0], demand as u64);
            assert!(out[1..].iter().enumerate().all(|(n, &word)| word == n as u64 + 1));
            assert!(LIVE.spilled().is_empty() && LIVE.spill.is_empty());
            // SAFETY: This test holds exclusive fixture control of the land observer.
            unsafe { prepare() };
            // SAFETY: This test holds exclusive fixture control of the land observer.
            unsafe { install_offered() };
            assert!(LIVE.log_capacity.load(Ordering::SeqCst) >= demand);
        }
        assert_eq!(test_support::lost(), lost);
        assert_eq!(test_support::epoch(), epochs);
        LIVE.enabled.store(false, Ordering::SeqCst);
    }

    #[test]
    fn records_past_a_short_log_keep_their_order_among_ring_records() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { test_support::reset_with_words(&mut owned, 8) };
        let mut out = vec![0; 64];
        drain(&mut out, 10, true);
        for payload in 1..=6 {
            test_support::append_record(&[payload, payload * 10]);
        }
        let drained = drain(&mut out, 11, false);
        assert!(drained.complete, "{}", drained.reason);
        assert_eq!(drained.words, 18);
        let records: Vec<_> = out[..18].chunks(3).map(|record| record.to_vec()).collect();
        assert_eq!(
            records,
            (1..=6).map(|payload| vec![3, payload, payload * 10]).collect::<Vec<_>>()
        );
        assert!(LIVE.spilled().is_empty() && LIVE.spill.is_empty());
        LIVE.enabled.store(false, Ordering::SeqCst);
    }

    #[test]
    fn a_spill_never_waits_for_the_drain_and_a_burst_keeps_every_record_in_order() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { test_support::reset_with_words(&mut owned, 8) };
        let mut out = vec![0; 1024];
        drain(&mut out, 10, true);
        let lost = LIVE.lost.load(Ordering::SeqCst);
        let held = LIVE.spilled();
        let (done, finished) = std::sync::mpsc::channel();
        let producer = std::thread::spawn(move || {

            for payload in 1..=40_u64 {
                test_support::append_record(&[payload, payload * 10, !payload]);
            }
            done.send(()).unwrap();
        });
        finished
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("a land event spill waited for the drain's lock");
        drop(held);
        producer.join().unwrap();
        let drained = drain(&mut out, 11, false);
        assert!(drained.complete, "{}", drained.reason);
        assert_eq!(drained.words, 160);
        let records: Vec<_> = out[..160].chunks(4).map(<[u64]>::to_vec).collect();
        assert_eq!(
            records,
            (1..=40_u64)
                .map(|payload| vec![4, payload, payload * 10, !payload])
                .collect::<Vec<_>>()
        );
        assert_eq!(LIVE.lost.load(Ordering::SeqCst), lost);
        assert!(LIVE.spilled().is_empty() && LIVE.spill.is_empty());
        LIVE.enabled.store(false, Ordering::SeqCst);
    }

    #[test]
    fn live_word_hint_presizes_the_log_and_drains_before_any_event_overflow() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { test_support::reset_with_words(&mut owned, 0) };
        // SAFETY: test holds `exclusive()` with no live land-event hooks.
        unsafe { presize(50_005) };
        let prepared = capacity();
        assert!(prepared >= 50_005);
        let mut out = vec![0; prepared];
        // SAFETY: test holds `exclusive()` with no live land-event hooks.
        unsafe { install_offered() };
        drain(&mut out, 10, true);
        append(50_005, |put| {
            for _ in 1..50_005 {
                put(7);
            }
            Ok(())
        });
        let observed = drain(&mut out, 11, false);
        assert!(observed.complete);
        assert_eq!(observed.words, 50_005);
        LIVE.enabled.store(false, Ordering::SeqCst);
    }

    #[test]
    fn frame_headroom_never_reenters_land_log_demand() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { test_support::reset_with_words(&mut owned, 0) };
        let mut capacities = crate::extended::land_events::Capacities { words: 9 };
        let mut frames: [crate::extended::land_events::Captured; 4] = std::array::from_fn(|_| {
            crate::extended::land_events::Captured::with_capacities(&capacities)
        });
        // SAFETY: test holds `exclusive()` with no live land-event hooks.
        unsafe { presize(9) };
        for hour in 10..18 {
            for frame in &mut frames {
                // SAFETY: test holds `exclusive()` with no live land-event hooks.
                unsafe { frame.grow(&mut capacities) };
            }
            // SAFETY: test holds `exclusive()` with no live land-event hooks.
            unsafe { frames[(hour as usize - 10) % 4].drain(hour, hour == 10) };
            assert_eq!(capacities.words, 9);
            assert_eq!(LIVE.log_required.load(Ordering::Relaxed), 9);
            assert_eq!(LIVE.log_capacity.load(Ordering::Acquire), 16);
        }
        LIVE.enabled.store(false, Ordering::SeqCst);
    }

    #[test]
    fn a_real_producer_overlapping_the_boundary_poisoned_only_that_interval() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        let mut out = vec![0; 32];
        drain(&mut out, 10, true);
        let entered = std::sync::Barrier::new(2);
        let release = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let producer = scope.spawn(|| {

                let _call = Call::enter();
                append(4, |put| {
                    put(6);
                    entered.wait();
                    release.wait();
                    put(19);
                    put(0);
                    Ok(())
                });
                _call.finish();
            });
            entered.wait();
            LIVE.log_required.fetch_max(4096, Ordering::SeqCst);
            // SAFETY: This test holds exclusive fixture control of the land observer.
            unsafe { prepare() };
            // SAFETY: This test holds exclusive fixture control of the land observer.
            unsafe { install_offered() };
            assert_eq!(LIVE.log_capacity.load(Ordering::SeqCst), 1024);
            let bad = drain(&mut out, 11, false);
            assert!(!bad.complete);
            assert!(bad.lost > 0);
            release.wait();
            producer.join().unwrap();
        });
        // SAFETY: This test holds exclusive fixture control of the land observer.
        unsafe { install_offered() };
        assert_eq!(LIVE.log_capacity.load(Ordering::SeqCst), 4096);
        let next = drain(&mut out, 12, false);
        assert!(next.complete);
        assert_eq!(next.words, 0);
        LIVE.enabled.store(false, Ordering::SeqCst);
    }
    #[test]
    fn resolver_lifetime_at_boundary_does_not_poison_an_interval() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds `exclusive()` with no live native land-event hooks.
        unsafe { reset(&mut owned) };
        let mut out = vec![0; 32];
        drain(&mut out, 10, true);
        let _resolver_call = Call::enter();
        assert_ne!(LIVE.active.load(Ordering::SeqCst), 0);
        assert_eq!(LIVE.producers.load(Ordering::SeqCst), 0);
        assert!(drain(&mut out, 11, false).complete);
        LIVE.enabled.store(false, Ordering::SeqCst);
        _resolver_call.finish();
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn live_drains_progress_while_another_observer_is_in_use() {
        let (_family, mut owned) = exclusive();
        // SAFETY: Test holds exclusive fixture control of an unhooked observer.
        unsafe { reset(&mut owned) };
        let fixture = Fixture::new(8);
        let mut out = vec![0; 32];
        fixture.observer.drain(&mut out, 10, true);
        let epoch = fixture.observer.epoch.load(Ordering::SeqCst);
        let capacity = fixture.observer.log_capacity.load(Ordering::SeqCst);
        append_record(&[5]);
        std::thread::spawn(|| {
            let mut words = vec![0; 32];
            drain(&mut words, 9, true);
            append_record(&[5]);
            let recorded = drain(&mut words, 10, false);
            assert!(recorded.complete, "{}", recorded.reason);
            assert_eq!(&words[..recorded.words], &[2, 5]);
            // SAFETY: This test holds exclusive fixture control of the land observer.
            unsafe { fail() };
            assert_eq!(LIVE.lost_casualty_reads(3), 4);
            // SAFETY: This test holds exclusive fixture control of the land observer.
            let drained = unsafe { lost_drain(11, "land observer frame not captured") };
            assert!(!drained.complete);
            assert_eq!(drained.reason, "land observer frame not captured");
            // SAFETY: This test holds exclusive fixture control of the land observer.
            unsafe { presize(1 << 16) };
            // SAFETY: This test holds exclusive fixture control of the land observer.
            unsafe { stop_observation() };
        })
        .join()
        .unwrap();
        assert_eq!(fixture.observer.epoch.load(Ordering::SeqCst), epoch);
        assert_eq!(fixture.observer.log_capacity.load(Ordering::SeqCst), capacity);
        assert!(fixture.production_enabled());
        fixture.append_record(&[9]);
        let owned = fixture.observer.drain(&mut out, 11, false);
        assert!(owned.complete);
        assert_eq!(&out[..owned.words], &[2, 9]);
        std::thread::scope(|scope| scope.spawn(|| fixture.observer.fail()).join().unwrap());
        assert_ne!(fixture.observer.epoch.load(Ordering::SeqCst), epoch);
        assert!(!fixture.observer.drain(&mut out, 12, false).complete);
    }
}
