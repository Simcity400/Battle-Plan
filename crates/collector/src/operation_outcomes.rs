//! Every successful native operation completion, copied before its caller can
//! auto-repeat or destroy the instance. Native paths/sites live in engine.
use crate::{hook::Hook, shared_lane::SharedLane};
use engine::{Memory, operation_outcome_sites as native};
use std::{
    cell::{Cell, UnsafeCell},
    ops::Range,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

static BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static RUNTIME_ENABLED: AtomicBool = AtomicBool::new(true);
static ENABLED: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicU64 = AtomicU64::new(1);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static LOG: AtomicUsize = AtomicUsize::new(0);
// A larger generation waiting for the lane boundary to seal the current one.
static OFFERED: AtomicUsize = AtomicUsize::new(0);
// Longest description a shared slot could not hold; sizes the next one.
static SHARED_DESCRIPTION_DEMAND: AtomicUsize = AtomicUsize::new(0);
static SERIAL: AtomicU64 = AtomicU64::new(1);
static EVENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
#[allow(clippy::vec_box)] // Published pointers stay stable through vector growth.
static LOGS: Mutex<Vec<Box<Log>>> = Mutex::new(Vec::new());
static UNHOOKED: OnceLock<std::borrow::Cow<'static, str>> = OnceLock::new();
static REQUIRED_LANES: AtomicUsize = AtomicUsize::new(1);
// 11: thread registrations beyond the dedicated lanes, 12: refused shared appends.
static COUNTERS: [AtomicU64; 13] = [const { AtomicU64::new(0) }; 13];
const FAILURE_NAMES: [&str; 36] = [
    "operation completion address overflow",
    "operation completion module overflow",
    "operation completion identity unreadable",
    "operation completion instance vtable mismatch",
    "operation completion null handle",
    "operation completion definition unreadable",
    "operation module address overflow",
    "operation definition identity unreadable",
    "operation definition vtable mismatch",
    "operation completion owner unreadable",
    "operation completion owner identity unreadable",
    "operation completion owner vtable mismatch",
    "operation completion null owner",
    "operation completion target unreadable",
    "operation completion terminal unreadable",
    "operation completion outcomes unreadable",
    "operation completion stored flags invalid",
    "operation completion description exceeds native string maximum",
    "operation completion description header invalid",
    "operation completion description terminator unreadable or invalid",
    "operation completion clock address overflow",
    "operation completion clock unreadable",
    "operation completion hour address overflow",
    "operation completion hour unreadable",
    "operation completion callback incomplete",
    "operation completion description bytes unreadable",
    "operation completion staging absent",
    "operation completion stale handoff",
    "operation completion description exceeds byte budget",
    "operation completion description byte ring short",
    "operation completion description demand overflow",
    "operation completion description capacity overflow",
    "operation completion description allocation failed",
    "operation completion event sequence exhausted",
    "operation completion shared lane exhausted",
    "operation completion sealed staging successor absent",
];
static FAILURES: [AtomicU64; FAILURE_NAMES.len() + 1] =
    [const { AtomicU64::new(0) }; FAILURE_NAMES.len() + 1];
thread_local! {
    static DEPTH: Cell<usize> = const { Cell::new(0) };
    static UNWOUND: Cell<bool> = const { Cell::new(false) };
    static LANE: Cell<(usize, u64, usize)> = const { Cell::new((0, 0, 0)) };
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub owner: u32,
    pub sequence: u64,
    pub id: u64,
    pub target: u32,
    pub definition_token: u32,
    pub completion_hour: i32,
    pub terminal: u8,
    pub bonus: u8,
    pub detected: u8,
    pub description_start: usize,
    pub description_len: usize,
    pub description_error: u8,
    pub description_required: u64,
}
impl Event {
    fn from_native(row: native::Row) -> Self {
        Self {
            owner: row.owner,
            id: row.id,
            target: row.target,
            definition_token: row.definition_token,
            completion_hour: row.completion_hour,
            terminal: row.terminal,
            bonus: row.bonus,
            detected: row.detected,
            ..Self::default()
        }
    }
}
#[derive(Clone, Copy, Default)]
struct Record {
    event: Event,
    order: u64,
    byte_ring: usize,
    byte_start: u64,
    byte_demand: u64,
}
fn boxed<T>(value: T) -> Result<Box<T>, &'static str> {
    let mut values = engine::allocation::vec(1)
        .map_err(|_| "operation completion description allocation failed")?;
    values.push(value);
    let slice = boxed_slice(values)?;
    Ok(unsafe { Box::from_raw(Box::into_raw(slice).cast::<T>()) })
}
fn boxed_slice<T>(values: Vec<T>) -> Result<Box<[T]>, &'static str> {
    engine::allocation::boxed_slice(values)
        .map_err(|_| "operation completion description allocation failed")
}
struct ByteRing {
    bytes: Box<[UnsafeCell<u8>]>,
    write: UnsafeCell<u64>,
    consumed: AtomicU64,
}
unsafe impl Sync for ByteRing {}
impl ByteRing {
    fn try_new(bytes: usize) -> Result<Self, &'static str> {
        let mut storage = Vec::new();
        engine::allocation::reserve_exact(&mut storage, bytes)
            .map_err(|_| "operation completion description allocation failed")?;
        storage.resize_with(bytes, || UnsafeCell::new(0));
        Ok(Self {
            bytes: boxed_slice(storage)?,
            write: UnsafeCell::new(0),
            consumed: AtomicU64::new(0),
        })
    }
    fn pointer(&self, index: usize) -> *mut u8 {
        unsafe { self.bytes.as_ptr().add(index).cast::<u8>().cast_mut() }
    }
}
struct Lane {
    records: Box<[UnsafeCell<Record>]>,
    byte_ring: AtomicUsize,
    // Record pointers keep old rings alive until workers and hooks quiesce.
    #[allow(clippy::vec_box)]
    byte_rings: Mutex<Vec<Box<ByteRing>>>,
    write: UnsafeCell<u64>,
    attempts: UnsafeCell<u64>,
    byte_demand: UnsafeCell<u64>,
    published: AtomicU64,
    published_attempts: AtomicU64,
    consumed: AtomicU64,
    consumed_attempts: AtomicU64,
    consumed_byte_demand: AtomicU64,
    required_entries: AtomicUsize,
    required_bytes: AtomicUsize,
}
unsafe impl Sync for Lane {}
impl Lane {
    fn new(entries: usize, bytes: usize) -> Result<Self, &'static str> {
        let entries = entries.max(1).checked_next_power_of_two().ok_or_else(|| {
            let _ = engine::allocation::oom();
            "operation completion description capacity overflow"
        })?;
        let ring = boxed(ByteRing::try_new(bytes.max(1))?)?;
        let pointer = &*ring as *const ByteRing as usize;
        let mut records = Vec::new();
        engine::allocation::reserve_exact(&mut records, entries)
            .map_err(|_| "operation completion description allocation failed")?;
        records.resize_with(entries, || UnsafeCell::new(Record::default()));
        let mut byte_rings = Vec::new();
        engine::allocation::reserve_exact(&mut byte_rings, 1)
            .map_err(|_| "operation completion description allocation failed")?;
        byte_rings.push(ring);
        Ok(Self {
            records: boxed_slice(records)?,
            byte_ring: AtomicUsize::new(pointer),
            byte_rings: Mutex::new(byte_rings),
            write: UnsafeCell::new(0),
            attempts: UnsafeCell::new(0),
            byte_demand: UnsafeCell::new(0),
            published: AtomicU64::new(0),
            published_attempts: AtomicU64::new(0),
            consumed: AtomicU64::new(0),
            consumed_attempts: AtomicU64::new(0),
            consumed_byte_demand: AtomicU64::new(0),
            required_entries: AtomicUsize::new(0),
            required_bytes: AtomicUsize::new(0),
        })
    }
    fn ring(&self) -> &ByteRing {
        unsafe { &*(self.byte_ring.load(Ordering::Acquire) as *const ByteRing) }
    }
    #[cfg(test)]
    fn grow_bytes(&self) -> Result<(), &'static str> {
        self.grow_bytes_for(0)
    }
    fn grow_bytes_for(&self, floor: usize) -> Result<(), &'static str> {
        let required = self.required_bytes.load(Ordering::Relaxed).max(floor);
        if required > self.ring().bytes.len() {
            let capacity = required.checked_next_power_of_two().ok_or_else(|| {
                let _ = engine::allocation::oom();
                "operation completion description capacity overflow"
            })?;
            let fresh = boxed(ByteRing::try_new(capacity)?)?;
            let pointer = &*fresh as *const ByteRing as usize;
            let mut retained = self
                .byte_rings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            engine::allocation::reserve(&mut retained, 1)
                .map_err(|_| "operation completion description allocation failed")?;
            retained.push(fresh);
            self.byte_ring.store(pointer, Ordering::Release);
        }
        Ok(())
    }
}
struct Log {
    serial: u64,
    lanes: Box<[Lane]>,
    // Threads registering after every lane is taken publish here, so a
    // generation never loses an interval to its writer count.
    shared: SharedLane<Record>,
    registered: AtomicUsize,
    entries: usize,
    sequence: AtomicU64,
    consumed: AtomicU64,
    consumed_epoch: AtomicU64,
    // Sealed at the lane boundary that installs `next`; stale callbacks
    // holding this generation follow it instead of losing a completion.
    publishers: AtomicUsize,
    next: AtomicUsize,
}
impl Log {
    #[cfg(test)]
    fn new(
        entries: usize,
        bytes: usize,
        lanes: usize,
        required: &[(usize, usize)],
    ) -> Result<Self, &'static str> {
        Self::with_shared(entries, bytes, lanes, required, (0, 0))
    }
    /// `shared` is the shared lane's (entries, per-slot description bytes)
    /// demand; the stride starts from the live average description.
    fn with_shared(
        entries: usize,
        bytes: usize,
        lanes: usize,
        required: &[(usize, usize)],
        shared: (usize, usize),
    ) -> Result<Self, &'static str> {
        let entries = entries.max(1).checked_next_power_of_two().ok_or_else(|| {
            let _ = engine::allocation::oom();
            "operation completion description capacity overflow"
        })?;
        // One more thread's lane: its records, and one lane's description
        // bytes spread over them. A longer description keeps its record, is
        // marked short and raises the next generation's slot.
        let slots = entries
            .div_ceil(lanes.max(1))
            .max(shared.0)
            .checked_next_power_of_two()
            .ok_or("operation completion description capacity overflow")?;
        let stride = shared.1.max(bytes.div_ceil(slots));
        let shared = SharedLane::new(slots, stride)
            .map_err(|_| "operation completion description allocation failed")?;
        let mut storage = Vec::new();
        engine::allocation::reserve_exact(&mut storage, lanes)
            .map_err(|_| "operation completion description allocation failed")?;
        for index in 0..lanes {
            storage.push(Lane::new(
                (if index == 0 {
                    entries
                } else {
                    entries.div_ceil(lanes)
                })
                .max(required.get(index).map_or(0, |demand| demand.0)),
                bytes.max(required.get(index).map_or(0, |demand| demand.1)),
            )?);
        }
        Ok(Self {
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
            lanes: boxed_slice(storage)?,
            shared,
            registered: AtomicUsize::new(0),
            entries,
            sequence: AtomicU64::new(0),
            consumed: AtomicU64::new(0),
            consumed_epoch: AtomicU64::new(0),
            publishers: AtomicUsize::new(0),
            next: AtomicUsize::new(0),
        })
    }
}
struct Call {
    returned: bool,
}
impl Call {
    fn enter() -> Self {
        DEPTH.with(|depth| {
            if depth.get() == 0 && !UNWOUND.get() {
                ACTIVE.fetch_add(1, Ordering::SeqCst);
            }
            depth.set(depth.get() + 1);
        });
        Self { returned: false }
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        if !self.returned {
            // Rust cleanup precedes completion of the foreign unwind. Retain
            // this thread's claim; later calls cannot release its return path.
            UNWOUND.set(true);
            count("operation completion callback incomplete");
            COUNTERS[9].fetch_add(1, Ordering::Relaxed);
            invalidate();
        }
        DEPTH.with(|depth| {
            depth.set(depth.get() - 1);
            if depth.get() == 0 && !UNWOUND.get() {
                ACTIVE.fetch_sub(1, Ordering::SeqCst);
            }
        });
    }
}
pub fn invalidate() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
}
pub fn lost_drain() {
    COUNTERS[7].fetch_add(1, Ordering::Relaxed);
    invalidate();
}
fn failed() {
    COUNTERS[4].fetch_add(1, Ordering::Relaxed);
    invalidate();
}
fn count(reason: &'static str) {
    let index = FAILURE_NAMES
        .iter()
        .position(|name| *name == reason)
        .unwrap_or(FAILURE_NAMES.len());
    FAILURES[index].fetch_add(1, Ordering::Relaxed);
}
fn failed_reason(reason: &'static str) {
    count(reason);
    failed();
}
fn overflow() {
    COUNTERS[1].fetch_add(1, Ordering::Relaxed);
    invalidate();
}
pub fn thread_slots() -> usize {
    std::thread::available_parallelism()
        .map_or(1, usize::from)
        .max(REQUIRED_LANES.load(Ordering::Relaxed))
}
fn log() -> Option<&'static Log> {
    let pointer = LOG.load(Ordering::Acquire) as *const Log;
    (!pointer.is_null()).then(|| unsafe { &*pointer })
}
fn offered() -> Option<&'static Log> {
    let pointer = OFFERED.load(Ordering::Acquire) as *const Log;
    (!pointer.is_null()).then(|| unsafe { &*pointer })
}
/// Where this thread publishes: its own lane, or the shared lane once every
/// lane of the generation is registered.
#[derive(Clone, Copy)]
enum Target<'a> {
    Lane(&'a Lane),
    Shared,
}
fn lane(log: &Log) -> Target<'_> {
    LANE.with(|cache| {
        let pointer = log as *const Log as usize;
        let (old, serial, index) = cache.get();
        let index = if old == pointer && serial == log.serial {
            index
        } else {
            let index = log.registered.fetch_add(1, Ordering::Relaxed);
            cache.set((pointer, log.serial, index));
            if index >= log.lanes.len() {
                // Growth offers a dedicated lane to the next generation.
                REQUIRED_LANES.fetch_max(index + 1, Ordering::Relaxed);
                COUNTERS[11].fetch_add(1, Ordering::Relaxed);
            }
            index
        };
        log.lanes.get(index).map_or(Target::Shared, Target::Lane)
    })
}
struct Publishing<'a>(&'a Log);
impl Drop for Publishing<'_> {
    fn drop(&mut self) {
        self.0.publishers.fetch_sub(1, Ordering::Release);
    }
}
/// A publication lease on the generation this callback reached; a sealed
/// one forwards to its prepared successor.
fn publishing(mut log: &Log) -> Option<Publishing<'_>> {
    loop {
        match log
            .publishers
            .fetch_update(Ordering::Acquire, Ordering::Relaxed, |count| {
                count.checked_add(1).filter(|&count| count != usize::MAX)
            }) {
            Ok(_) => return Some(Publishing(log)),
            Err(usize::MAX) => {
                let next = log.next.load(Ordering::Acquire) as *const Log;
                if next.is_null() {
                    failed_reason("operation completion sealed staging successor absent");
                    return None;
                }
                log = unsafe { &*next };
            }
            Err(_) => {
                overflow();
                return None;
            }
        }
    }
}
/// Lane boundary: seal the current generation for this handoff once no
/// callback is publishing into it, and install the offered successor.
/// Returns the generation this handoff drains.
fn seal_offered(log: &'static Log) -> &'static Log {
    let Some(offered) = offered() else {
        return log;
    };
    offered
        .consumed_epoch
        .store(EPOCH.load(Ordering::Acquire), Ordering::Release);
    log.next
        .store(offered as *const Log as usize, Ordering::Release);
    if log
        .publishers
        .compare_exchange(0, usize::MAX, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return log;
    }
    LOG.store(offered as *const Log as usize, Ordering::Release);
    let _ = OFFERED.compare_exchange(
        offered as *const Log as usize,
        0,
        Ordering::AcqRel,
        Ordering::Relaxed,
    );
    log
}
/// Called before installing hooks, using the exact retained recording history.
pub fn restore_sequence(previous: u64) -> Result<(), &'static str> {
    if ENABLED.load(Ordering::Acquire) || ACTIVE.load(Ordering::SeqCst) != 0 {
        return Err("operation completion sequence restoration requires disabled hooks");
    }
    if previous == u64::MAX {
        return Err("operation completion event sequence exhausted");
    }
    EVENT_SEQUENCE.fetch_max(previous, Ordering::Relaxed);
    Ok(())
}

/// Off-thread only. Old published allocations survive every replacement and
/// are freed only after disabled hooks, active originals and workers drain.
pub fn resize(entries: usize) {
    presize(entries, 0);
}
/// Live operation-description demand and rejected-byte pressure prepare every
/// lane off-thread. The completion callback still copies its original
/// retained completion descriptor exactly once into admitted storage.
/// A larger generation is offered to the lane boundary, which seals the
/// current one for its handoff, so growth loses no interval.
pub fn presize(entries: usize, bytes: usize) {
    if !RUNTIME_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let sources = [log(), offered()];
    let generations = || sources.into_iter().flatten();
    let entries = entries.max(generations().map(|log| log.entries).max().unwrap_or(1));
    for log in generations() {
        // A new byte ring is published independently of record storage: old
        // records retain their ring, so byte growth loses no observation epoch.
        for lane in &log.lanes {
            if let Err(reason) = lane.grow_bytes_for(bytes) {
                count(reason);
            }
        }
    }
    let lanes = thread_slots().max(generations().map(|log| log.lanes.len()).max().unwrap_or(0));
    let required = |index: usize| {
        generations()
            .filter_map(|log| log.lanes.get(index))
            .map(|lane| {
                (
                    lane.required_entries
                        .load(Ordering::Relaxed)
                        .max(lane.records.len()),
                    lane.ring().bytes.len(),
                )
            })
            .fold((0, 0), |a, b| (a.0.max(b.0), a.1.max(b.1)))
    };
    let shared = (
        generations()
            .map(|log| log.shared.required_entries().max(log.shared.len()))
            .max()
            .unwrap_or(0),
        generations()
            .map(|log| log.shared.stride())
            .max()
            .unwrap_or(0)
            .max(SHARED_DESCRIPTION_DEMAND.load(Ordering::Relaxed)),
    );
    if generations().any(|log| {
        log.entries >= entries
            && log.lanes.len() >= lanes
            && log.shared.len() >= shared.0
            && log.shared.stride() >= shared.1
            && log
                .lanes
                .iter()
                .enumerate()
                .all(|(index, lane)| lane.records.len() >= required(index).0)
    }) {
        return;
    }
    let fresh = (|| {
        let required = engine::allocation::collect_vec((0..lanes).map(required))
            .map_err(|_| "operation completion description allocation failed")?;
        boxed(Log::with_shared(entries, bytes, lanes, &required, shared)?)
    })();
    let fresh = match fresh {
        Ok(fresh) => fresh,
        Err(reason) => {
            failed_reason(reason);
            return;
        }
    };
    let pointer = &*fresh as *const Log as usize;
    let mut retained = LOGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if engine::allocation::reserve(&mut retained, 1).is_err() {
        failed_reason("operation completion description allocation failed");
        return;
    }
    retained.push(fresh);
    if sources[0].is_none() {
        LOG.store(pointer, Ordering::Release);
    } else {
        OFFERED.store(pointer, Ordering::Release);
    }
}
/// # Safety
/// Hooks are disabled; executable ranges/ACTIVE have quiesced and every worker
/// holding a Handoff or log reference is joined. A premature release retains it.
pub unsafe fn release() {
    if ENABLED.load(Ordering::SeqCst) || ACTIVE.load(Ordering::SeqCst) != 0 {
        COUNTERS[6].fetch_add(1, Ordering::Relaxed);
        return;
    }
    LOG.store(0, Ordering::Release);
    OFFERED.store(0, Ordering::Release);
    LOGS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

fn append_observation(
    log: &Log,
    target: Target<'_>,
    observation: native::Observation,
    memory: &impl Memory,
) {
    match target {
        Target::Lane(lane) => append_to_lane(log, lane, observation, memory),
        Target::Shared => append_shared(log, observation, memory),
    }
}
/// A thread without a dedicated lane in this generation. The record takes
/// its log-wide order exactly as a lane record does.
fn append_shared(log: &Log, observation: native::Observation, memory: &impl Memory) {
    let Some(mut slot) = log.shared.reserve() else {
        // Refused demand is retained for the next generation's shared lane.
        count("operation completion shared lane exhausted");
        COUNTERS[12].fetch_add(1, Ordering::Relaxed);
        overflow();
        return;
    };
    let mut event = Event::from_native(observation.row);
    let length = observation.description_len();
    event.description_required = length as u64;
    let (description_error, description_len) = if length > slot.stride() {
        SHARED_DESCRIPTION_DEMAND.fetch_max(length, Ordering::Relaxed);
        count("operation completion description byte ring short");
        (2, 0)
    } else {
        let output = unsafe { std::slice::from_raw_parts_mut(slot.bytes(), length) };
        if let Err(reason) = observation.copy_description(memory, output) {
            failed_reason(reason);
            return; // The dropped reservation publishes void.
        }
        (0, length)
    };
    (event.description_error, event.description_len) = (description_error, description_len);
    let Ok(sequence) = EVENT_SEQUENCE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
        last.checked_add(1)
    }) else {
        failed_reason("operation completion event sequence exhausted");
        return;
    };
    event.sequence = sequence + 1;
    let order = log.sequence.fetch_add(1, Ordering::Relaxed) + 1;
    // Shared descriptions stay in their slot; no byte ring is involved.
    slot.publish(Record {
        event,
        order,
        byte_ring: 0,
        byte_start: 0,
        byte_demand: 0,
    });
}
fn append_to_lane(log: &Log, lane: &Lane, observation: native::Observation, memory: &impl Memory) {
    let cursor = unsafe { *lane.write.get() };
    let attempts = unsafe { *lane.attempts.get() }.wrapping_add(1);
    unsafe {
        *lane.attempts.get() = attempts;
    }
    lane.published_attempts.store(attempts, Ordering::Release);
    let demand = attempts
        .saturating_sub(lane.consumed_attempts.load(Ordering::Acquire))
        .max(
            cursor
                .saturating_sub(lane.consumed.load(Ordering::Acquire))
                .saturating_add(1),
        );
    let mut event = Event::from_native(observation.row);
    let length = observation.description_len();
    event.description_required = length as u64;
    let ring = lane.ring();
    let byte_start = unsafe { *ring.write.get() };
    let byte_pending = byte_start.saturating_sub(ring.consumed.load(Ordering::Acquire));
    let Some(byte_demand) = (unsafe { *lane.byte_demand.get() }).checked_add(length as u64) else {
        failed_reason("operation completion description demand overflow");
        return;
    };
    let required_bytes =
        byte_demand.saturating_sub(lane.consumed_byte_demand.load(Ordering::Acquire));
    let Ok(required_bytes) = usize::try_from(required_bytes) else {
        failed_reason("operation completion description demand overflow");
        return;
    };
    lane.required_bytes
        .fetch_max(required_bytes, Ordering::Relaxed);
    if demand > lane.records.len() as u64 {
        lane.required_entries
            .fetch_max(demand as usize, Ordering::Relaxed);
        overflow();
        return;
    }
    let (description_error, description_len) = if byte_pending.saturating_add(length as u64)
        > ring.bytes.len() as u64
    {
        count("operation completion description byte ring short");
        (2, 0)
    } else {
        let start = byte_start as usize % ring.bytes.len();
        let first_len = length.min(ring.bytes.len() - start);
        let first = unsafe { std::slice::from_raw_parts_mut(ring.pointer(start), first_len) };
        let second = unsafe { std::slice::from_raw_parts_mut(ring.pointer(0), length - first_len) };
        if let Err(reason) = observation.copy_description_parts(memory, first, second) {
            failed_reason(reason);
            return;
        }
        (0, length)
    };
    (event.description_error, event.description_len) = (description_error, description_len);
    let index = cursor as usize & (lane.records.len() - 1);
    let Ok(sequence) = EVENT_SEQUENCE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
        last.checked_add(1)
    }) else {
        failed_reason("operation completion event sequence exhausted");
        return;
    };
    event.sequence = sequence + 1;
    let order = log.sequence.fetch_add(1, Ordering::Relaxed) + 1;
    unsafe {
        // Only a published record owns demand and bytes for a sealed commit.
        *lane.byte_demand.get() = byte_demand;
        *ring.write.get() = byte_start + description_len as u64;
        *lane.records[index].get() = Record {
            event,
            order,
            byte_ring: ring as *const ByteRing as usize,
            byte_start,
            byte_demand,
        };
        *lane.write.get() = cursor + 1;
    }
    lane.published.store(cursor + 1, Ordering::Release);
}
fn observe_success(
    log: &Log,
    target: Target<'_>,
    memory: &impl Memory,
    base: usize,
    instance: usize,
) {
    match native::observe(memory, base, instance) {
        Ok(observation) => append_observation(log, target, observation, memory),
        Err(reason) => failed_reason(reason),
    }
}
fn observe_return(
    log: &Log,
    target: Target<'_>,
    memory: &impl Memory,
    base: usize,
    instance: usize,
    result: usize,
) {
    if result as u8 != 0 {
        observe_success(log, target, memory, base, instance);
    }
}
type NativeFunction = unsafe extern "system-unwind" fn(usize) -> usize;
unsafe extern "system-unwind" fn completion(instance: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::OperationOutcomes);
    let mut active = Call::enter();
    let original: NativeFunction = unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    let generation = crate::world_lifetime::generation();
    let result = timing.original(|| unsafe { original(instance) });
    // P: native AL is the new-completion predicate; preserve full RAX.
    if result as u8 != 0 && ENABLED.load(Ordering::Acquire) {
        if generation != crate::world_lifetime::generation()
            || crate::world_lifetime::invalidating()
        {
            count("operation completion native world changed during original");
            COUNTERS[10].fetch_add(1, Ordering::Relaxed);
            invalidate();
            active.returned = true;
            return result;
        }
        COUNTERS[0].fetch_add(1, Ordering::Relaxed);
        if let Some(log) = log() {
            if let Some(lease) = publishing(log) {
                let memory = unsafe { engine::memory::LiveMemory::new() };
                observe_return(
                    lease.0,
                    lane(lease.0),
                    &memory,
                    BASE.load(Ordering::Relaxed),
                    instance,
                    result,
                );
            }
        } else {
            count("operation completion staging absent");
            COUNTERS[10].fetch_add(1, Ordering::Relaxed);
            invalidate();
        }
    }
    active.returned = true;
    result
}
#[unsafe(no_mangle)]
pub static BATTLE_PLAN_OPERATION_OUTCOME_CALLBACKS: [NativeFunction; 1] = [completion];

#[derive(Clone, Copy, Default)]
pub struct LaneCursor {
    cursor: u64,
    attempts: u64,
}
#[derive(Clone, Copy)]
pub struct Handoff {
    log: usize,
    epoch: u64,
    sequence: u64,
    lanes: usize,
    shared_rejected: u64,
}
pub fn handoff_growing(cursors: &mut Vec<LaneCursor>) -> Result<Handoff, &'static str> {
    if UNHOOKED.get().is_some() || !ENABLED.load(Ordering::Acquire) {
        return Err("operation completion hooks unavailable");
    }
    let log = log().ok_or("operation completion staging unavailable")?;
    handoff_buffer(seal_offered(log), engine::walk::Buffer::Growing(cursors))
}
#[cfg(test)]
fn handoff_from(log: &Log, cursors: &mut [LaneCursor]) -> Result<Handoff, &'static str> {
    handoff_buffer(log, engine::walk::Buffer::Fixed(cursors))
}
fn handoff_buffer(
    log: &Log,
    mut cursors: engine::walk::Buffer<'_, LaneCursor>,
) -> Result<Handoff, &'static str> {
    let lanes = log.registered.load(Ordering::Acquire).min(log.lanes.len());
    cursors
        .ensure(lanes)
        .map_err(|_| "operation completion handoff allocation failed")?;
    if cursors.len() < lanes {
        COUNTERS[5].fetch_add(1, Ordering::Relaxed);
        invalidate();
        return Err("operation completion thread handoff staging short");
    }
    for (output, lane) in cursors.iter_mut().zip(&log.lanes[..lanes]) {
        output.cursor = lane.published.load(Ordering::Acquire);
        output.attempts = lane.published_attempts.load(Ordering::Acquire);
    }
    let shared_rejected = log.shared.rejected();
    Ok(Handoff {
        log: log as *const Log as usize,
        epoch: EPOCH.load(Ordering::Acquire),
        sequence: log.sequence.load(Ordering::Acquire),
        lanes,
        shared_rejected,
    })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drained {
    pub copied: usize,
    pub bytes: usize,
}
impl Handoff {
    pub fn pending(self) -> usize {
        let log = unsafe { &*(self.log as *const Log) };
        self.sequence
            .saturating_sub(log.consumed.load(Ordering::Acquire)) as usize
    }
    pub fn description_capacity(self, cursors: &[LaneCursor]) -> Result<usize, &'static str> {
        let log = unsafe { &*(self.log as *const Log) };
        if self.sequence < log.consumed.load(Ordering::Acquire) {
            return Ok(0);
        }
        let mut bytes = 0_usize;
        for (lane, end) in log.lanes.iter().zip(cursors).take(self.lanes) {
            // These sealed records cannot be reused until the spool releases them.
            for cursor in lane.consumed.load(Ordering::Acquire)..end.cursor {
                let length = unsafe {
                    (*lane.records[cursor as usize & (lane.records.len() - 1)].get())
                        .event
                        .description_len
                };
                bytes = engine::allocation::add(bytes, length)
                    .map_err(|_| "operation completion description demand overflow")?;
            }
        }
        for (_, index) in self.shared(log)? {
            bytes =
                engine::allocation::add(bytes, log.shared.record(index).0.event.description_len)
                    .map_err(|_| "operation completion description demand overflow")?;
        }
        Ok(bytes)
    }
    /// Published shared records of this interval in log-wide order.
    fn shared(self, log: &Log) -> Result<Vec<(u64, u64)>, &'static str> {
        let first = log.consumed.load(Ordering::Acquire);
        let mut shared = log
            .shared
            .published(|record| record.order)
            .map_err(|_| "operation completion description allocation failed")?;
        shared.retain(|&(order, _)| order > first && order <= self.sequence);
        Ok(shared)
    }
    /// Only after the sealed frame (complete or refused) was written. Failed
    /// conversion/encoding/I/O leaves both lane and byte-ring consumption intact.
    pub fn commit(self, cursors: &[LaneCursor]) {
        let log = unsafe { &*(self.log as *const Log) };
        if self.sequence < log.consumed.load(Ordering::Acquire)
            || cursors.len() < self.lanes
            || log
                .lanes
                .iter()
                .zip(cursors)
                .take(self.lanes)
                .any(|(lane, end)| {
                    end.cursor < lane.consumed.load(Ordering::Acquire)
                        || end.attempts < lane.consumed_attempts.load(Ordering::Acquire)
                })
        {
            return;
        }
        let pending = self.pending();
        for (lane, end) in log.lanes.iter().zip(cursors).take(self.lanes) {
            for cursor in lane.consumed.load(Ordering::Acquire)..end.cursor {
                let record =
                    unsafe { *lane.records[cursor as usize & (lane.records.len() - 1)].get() };
                if record.byte_ring != 0 {
                    let ring = unsafe { &*(record.byte_ring as *const ByteRing) };
                    ring.consumed.store(
                        record.byte_start + record.event.description_len as u64,
                        Ordering::Release,
                    );
                }
                // Demand belongs to this sealed record, never a separately
                // sampled counter from a later unsealed callback.
                lane.consumed_byte_demand
                    .store(record.byte_demand, Ordering::Release);
            }
            lane.consumed.store(end.cursor, Ordering::Release);
            lane.consumed_attempts
                .store(end.attempts, Ordering::Release);
        }
        // Copied shared records, and any of this sealed interval a refused
        // frame withdrew, leave with it; later orders stay pending.
        if let Ok(shared) = log.shared.published(|record| record.order) {
            for (order, index) in shared {
                if order <= self.sequence {
                    log.shared.release(index);
                }
            }
        }
        log.shared.advance(self.shared_rejected);
        log.consumed.store(self.sequence, Ordering::Release);
        log.consumed_epoch.store(self.epoch, Ordering::Release);
        COUNTERS[2].fetch_add(pending as u64, Ordering::Relaxed);
    }
    #[cfg(test)]
    fn drain(
        self,
        cursors: &[LaneCursor],
        output: &mut [Event],
        bytes: &mut [u8],
    ) -> Result<Drained, &'static str> {
        let result = self.prepare(cursors, output, bytes);
        if result != Err("operation completion stale handoff") {
            self.commit(cursors);
        }
        result
    }
    pub fn prepare(
        self,
        cursors: &[LaneCursor],
        output: &mut [Event],
        bytes: &mut [u8],
    ) -> Result<Drained, &'static str> {
        let log = unsafe { &*(self.log as *const Log) };
        let cursors = cursors
            .get(..self.lanes)
            .ok_or("operation completion drain cursors short")?;
        let first = log.consumed.load(Ordering::Acquire);
        if log.lanes.iter().zip(cursors).any(|(lane, end)| {
            end.cursor < lane.consumed.load(Ordering::Acquire)
                || end.attempts < lane.consumed_attempts.load(Ordering::Acquire)
        }) {
            count("operation completion stale handoff");
            return Err("operation completion stale handoff");
        }
        let Some(pending) = self
            .sequence
            .checked_sub(first)
            .map(|pending| pending as usize)
        else {
            // Never rewind consumption or the epoch for an idle reused handoff.
            count("operation completion stale handoff");
            return Err("operation completion stale handoff");
        };
        if log.consumed_epoch.load(Ordering::Acquire) != self.epoch
            || self.epoch != EPOCH.load(Ordering::Acquire)
        {
            return Err("operation completion observation restarted or lost");
        }
        if output.len() < pending || bytes.len() < self.description_capacity(cursors)? {
            lost_drain();
            return Err("operation completion drain staging short");
        }
        let mut tails = engine::allocation::collect_vec(
            log.lanes
                .iter()
                .take(self.lanes)
                .map(|lane| lane.consumed.load(Ordering::Acquire)),
        )
        .map_err(|_| "operation completion description allocation failed")?;
        let mut shared = self.shared(log)?.into_iter().peekable();
        let mut copied = 0;
        let mut used = 0_usize;
        while copied < pending {
            let next = log
                .lanes
                .iter()
                .zip(cursors)
                .enumerate()
                .filter(|(index, (_, end))| tails[*index] < end.cursor)
                .map(|(index, (lane, _))| {
                    let slot = tails[index] as usize & (lane.records.len() - 1);
                    (Some(index), unsafe { *lane.records[slot].get() })
                })
                .chain(
                    shared
                        .peek()
                        .map(|&(_, index)| (None, log.shared.record(index).0)),
                )
                .min_by_key(|(_, record)| record.order);
            let Some((index, record)) = next else {
                break;
            };
            // A reservation not published at the sealed boundary cannot be
            // silently replaced by a later cross-thread observation.
            if record.order != first + copied as u64 + 1 {
                break;
            }
            let mut event = record.event;
            let Some(index) = index else {
                // A shared record keeps its description in its own slot.
                let (_, slot) = shared.next().unwrap();
                let end = used
                    .checked_add(event.description_len)
                    .filter(|end| *end <= bytes.len())
                    .ok_or("operation completion drain staging short")?;
                bytes[used..end]
                    .copy_from_slice(&log.shared.record(slot).1[..event.description_len]);
                event.description_start = used;
                used = end;
                output[copied] = event;
                copied += 1;
                continue;
            };
            let ring = unsafe { &*(record.byte_ring as *const ByteRing) };
            let start = record.byte_start as usize % ring.bytes.len();
            let end = used
                .checked_add(event.description_len)
                .filter(|end| *end <= bytes.len())
                .ok_or("operation completion drain staging short")?;
            if event.description_len > ring.bytes.len() {
                return Err("operation completion description byte ring short");
            }
            let first_len = event.description_len.min(ring.bytes.len() - start);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    ring.pointer(start),
                    bytes.as_mut_ptr().add(used),
                    first_len,
                );
                std::ptr::copy_nonoverlapping(
                    ring.pointer(0),
                    bytes.as_mut_ptr().add(used + first_len),
                    event.description_len - first_len,
                );
            }
            event.description_start = used;
            used = end;
            output[copied] = event;
            copied += 1;
            tails[index] += 1;
        }
        if copied != pending || self.epoch != EPOCH.load(Ordering::Acquire) {
            lost_drain();
            return Err("operation completion publication incomplete or lost");
        }
        Ok(Drained {
            copied,
            bytes: used,
        })
    }
}

pub fn status() -> impl serde::Serialize {
    let counters = COUNTERS
        .each_ref()
        .map(|count| count.load(Ordering::Relaxed));
    let logs = LOGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let allocated_bytes: usize = logs
        .iter()
        .map(|log| {
            size_of::<Log>()
                + log
                    .lanes
                    .iter()
                    .map(|lane| {
                        size_of::<Lane>()
                            + lane.records.len() * size_of::<Record>()
                            + lane
                                .byte_rings
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .iter()
                                .map(|ring| size_of::<ByteRing>() + ring.bytes.len())
                                .sum::<usize>()
                    })
                    .sum::<usize>()
                + log.shared.allocated_bytes()
        })
        .sum();
    crate::status::status!({ "runtime_enabled": RUNTIME_ENABLED.load(Ordering::Relaxed),
        "enabled": ENABLED.load(Ordering::Acquire), "unhooked": UNHOOKED.get(),
        "observed": counters[0], "overflow": counters[1], "drained": counters[2],
        "sentinel_failures": counters[3], "read_failures": counters[4], "short_handoffs": counters[5],
        "release_refusals": counters[6], "lost_intervals": counters[7], "publication_failures":counters[8],
        "callback_incomplete": counters[9], "staging_absent": counters[10],
        "allocated_bytes":allocated_bytes,"retained_logs":logs.len(),
        "required_description_bytes": log().map_or(0, |log| log.lanes.iter()
            .map(|lane| lane.required_bytes.load(Ordering::Relaxed)).max().unwrap_or(0)),
        "required_thread_lanes": REQUIRED_LANES.load(Ordering::Relaxed),
        "shared_lane_threads": counters[11], "shared_lane_refusals": counters[12],
        "shared_lane_entries": log().map_or(0, |log| log.shared.len()),
        "required_shared_lane_entries": log().map_or(0, |log| log.shared.required_entries()),
        "shared_lane_description_stride": log().map_or(0, |log| log.shared.stride()),
        "offered_generation": offered().is_some(),
        "entry_capacity": log().map_or(0, |log| log.entries),
        "description_byte_capacity": log().map_or(0, |log| log.lanes.iter().map(|lane| lane.ring().bytes.len()).sum::<usize>()),
        "refusals": crate::status::sequence(FAILURE_NAMES.iter().enumerate().map(|(index,name)|
            crate::status::status!({"reason":*name,"count":FAILURES[index].load(Ordering::Relaxed)})).chain(std::iter::once(
                crate::status::status!({"reason":"operation completion unrecognized native refusal",
                    "count":FAILURES[FAILURE_NAMES.len()].load(Ordering::Relaxed)})))) })
}
pub struct Hooks {
    entry: Option<Hook>,
}
impl Hooks {
    pub unsafe fn new(memory: &impl Memory, base: usize, runtime_enabled: bool) -> Self {
        RUNTIME_ENABLED.store(runtime_enabled, Ordering::Relaxed);
        if !runtime_enabled {
            let _ = UNHOOKED.set(std::borrow::Cow::Borrowed(
                "operation outcomes disabled by attach config",
            ));
            return Self { entry: None };
        }
        unsafe { Self::install(memory, base) }.unwrap_or_else(|reason| {
            let _ = UNHOOKED.set(std::borrow::Cow::Owned(reason));
            Self { entry: None }
        })
    }
    unsafe fn install(memory: &impl Memory, base: usize) -> Result<Self, String> {
        if engine::read::<[u8; 16]>(memory, base + native::COMPLETION_RVA)
            != Some(native::COMPLETION_SENTINEL)
        {
            COUNTERS[3].fetch_add(1, Ordering::Relaxed);
            return Err(
                engine::allocation::string("operation completion entry sentinel differs")
                    .unwrap_or_default(),
            );
        }
        BASE.store(base, Ordering::Relaxed);
        let hook = unsafe {
            Hook::new(
                base + native::COMPLETION_RVA,
                completion as *const () as usize,
                native::COMPLETION_SENTINEL,
            )
        }?;
        ORIGINAL.store(hook.trampoline(), Ordering::Release);
        Ok(Self { entry: Some(hook) })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        if !crate::hook::shared::defer_preparation(invalidate) {
            invalidate();
        }
        let Some(hook) = self.entry.as_mut() else {
            return Ok(true);
        };
        let mut family = [hook];
        match Hook::set_enabled_batch_attach(&mut family, enabled) {
            Ok(false) => return Ok(false),
            Ok(true) => {}
            Err(reason) => {
                COUNTERS[8].fetch_add(1, Ordering::Relaxed);
                invalidate();
                return Err(reason);
            }
        }
        ENABLED.store(enabled && self.entry.is_some(), Ordering::SeqCst);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        engine::allocation::owned_vec(engine::allocation::collect_vec(
            self.entry.iter().flat_map(Hook::executable_ranges),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{ccountry, cgame_state, coperation, coperation_instance as operation};
    use std::cell::RefCell;

    static TEST: Mutex<()> = Mutex::new(());
    const BASE: usize = 0x1000_0000;
    const INSTANCE: usize = 0x1000;
    const COUNTRY: usize = 0x2000;
    const DEFINITION: usize = 0x3000;
    const STATE: usize = 0x4000;
    const PAYLOAD: usize = 0x5000;
    struct Fixture {
        bytes: RefCell<Vec<u8>>,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == BASE + cgame_state::SINGLETON_RVA && output.len() == 8 {
                output.copy_from_slice(&(STATE as u64).to_le_bytes());
                return true;
            }
            let bytes = self.bytes.borrow();
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            let Some(input) = bytes.get(address..end) else {
                return false;
            };
            output.copy_from_slice(input);
            true
        }
    }
    impl Fixture {
        fn new(description: &[u8]) -> Self {
            let mut bytes = vec![0; (PAYLOAD + description.len() + 1).max(0x6000)];
            let put = |bytes: &mut [u8], address: usize, value: u64| {
                bytes[address..address + 8].copy_from_slice(&value.to_le_bytes())
            };
            put(&mut bytes, INSTANCE, (BASE + operation::VTABLE_RVA) as u64);
            put(
                &mut bytes,
                INSTANCE + operation::HANDLE,
                (1_u64 << 32) | 4713,
            );
            put(
                &mut bytes,
                INSTANCE + operation::OWNER_COUNTRY,
                COUNTRY as u64,
            );
            put(
                &mut bytes,
                INSTANCE + operation::DEFINITION,
                DEFINITION as u64,
            );
            put(&mut bytes, COUNTRY, (BASE + ccountry::VTABLE_RVA) as u64);
            bytes[COUNTRY + ccountry::ID..COUNTRY + ccountry::ID + 4]
                .copy_from_slice(&1_u32.to_le_bytes());
            put(
                &mut bytes,
                DEFINITION,
                (BASE + coperation::VTABLE_RVA) as u64,
            );
            bytes[DEFINITION + coperation::TOKEN_ID..DEFINITION + coperation::TOKEN_ID + 4]
                .copy_from_slice(&0xfabc_1234_u32.to_le_bytes());
            bytes[INSTANCE + operation::TARGET_COUNTRY_ID
                ..INSTANCE + operation::TARGET_COUNTRY_ID + 4]
                .copy_from_slice(&123_u32.to_le_bytes());
            bytes[STATE + cgame_state::CURRENT_HOUR..STATE + cgame_state::CURRENT_HOUR + 4]
                .copy_from_slice(&(-7_i32).to_le_bytes());
            bytes[INSTANCE + operation::TERMINAL] = 1;
            bytes[INSTANCE + operation::BONUS] = 1;
            let header = INSTANCE + operation::EFFECT_DESCRIPTION;
            put(&mut bytes, header + 16, description.len() as u64);
            if description.len() <= 15 {
                bytes[header..header + description.len()].copy_from_slice(description);
                put(&mut bytes, header + 24, 15);
            } else {
                put(&mut bytes, header, PAYLOAD as u64);
                put(&mut bytes, header + 24, description.len() as u64);
                bytes[PAYLOAD..PAYLOAD + description.len()].copy_from_slice(description);
            }
            Self {
                bytes: RefCell::new(bytes),
            }
        }
        fn identify(&self, owner: u32, instance: u32) {
            let mut bytes = self.bytes.borrow_mut();
            bytes[COUNTRY + ccountry::ID..COUNTRY + ccountry::ID + 4]
                .copy_from_slice(&owner.to_le_bytes());
            bytes[INSTANCE + operation::HANDLE..INSTANCE + operation::HANDLE + 8]
                .copy_from_slice(&((u64::from(instance) << 32) | 4713).to_le_bytes());
        }
        fn delete(&self) {
            self.bytes.borrow_mut()[INSTANCE..INSTANCE + 0x188].fill(0);
        }
    }
    fn new_log(entries: usize, stride: usize) -> Log {
        let log = Log::new(entries, entries * stride, 1, &[]).unwrap();
        log.registered.store(1, Ordering::Relaxed);
        log
    }
    fn drain(log: &Log) -> Result<(Vec<Event>, Vec<u8>), &'static str> {
        let mut cursors = [LaneCursor::default()];
        let handoff = handoff_from(log, &mut cursors)?;
        let mut events = vec![Event::default(); handoff.pending()];
        let mut bytes = vec![0; handoff.description_capacity(&cursors).unwrap()];
        let drained = handoff.drain(&cursors, &mut events, &mut bytes)?;
        events.truncate(drained.copied);
        bytes.truncate(drained.bytes);
        Ok((events, bytes))
    }
    fn prime(log: &Log) {
        assert!(drain(log).is_err());
        assert_eq!(drain(log).unwrap().0.len(), 0);
    }
    fn complete(log: &Log, fixture: &Fixture, result: usize) {
        observe_return(
            log,
            Target::Lane(&log.lanes[0]),
            fixture,
            BASE,
            INSTANCE,
            result,
        );
    }

    #[test]
    fn completion_storage_refuses_allocator_failure_before_publication() {
        assert!(matches!(
            crate::test_support::with_allocations_denied(|| ByteRing::try_new(1024)),
            Err("operation completion description allocation failed")
        ));
        assert!(matches!(
            crate::test_support::with_allocations_denied(|| Lane::new(4097, 1024)),
            Err("operation completion description allocation failed")
        ));
        assert!(matches!(
            crate::test_support::with_allocations_denied(|| Log::new(4097, 1024, 2, &[])),
            Err("operation completion description allocation failed")
        ));
    }

    #[test]
    fn unavailable_completion_hook_does_not_block_installation_or_detach() {
        let _serial = TEST.lock().unwrap();
        let mut hooks = Hooks { entry: None };
        ENABLED.store(false, Ordering::Release);
        assert!(hooks.set_enabled(true).unwrap());
        assert!(!ENABLED.load(Ordering::Acquire));
        assert!(hooks.set_enabled(false).unwrap());
        assert!(!ENABLED.load(Ordering::Acquire));
    }

    #[test]
    fn completion_publication_releases_records_only_after_successful_write() {
        use crate::extended::operation_outcomes::{Captured, Ledger};
        use replay::{
            frame::{TableInput, TableView},
            rows::{TableId, TableRows},
        };
        let _serial = TEST.lock().unwrap();
        let log = new_log(4, 32);
        prime(&log);
        let fixture = Fixture::new(b"retained");
        complete(&log, &fixture, 1);
        let mut cursors = vec![LaneCursor::default()];
        let handoff = handoff_from(&log, &mut cursors).unwrap();
        let captured = Captured::sealed(handoff, cursors);
        let mut ledger = Ledger::default();
        let mut tables = vec![
            TableInput::Sampled(TableView::Unavailable {
                reason: "test".into(),
            });
            TableId::ALL.len()
        ];
        let before = log.lanes[0].consumed.load(Ordering::Acquire);
        let bytes_before =
            unsafe { &*(log.lanes[0].byte_ring.load(Ordering::Acquire) as *const ByteRing) }
                .consumed
                .load(Ordering::Acquire);
        // Conversion succeeded, but encoding or I/O can still fail. Dropping
        // that prepared frame is not publication and cannot release its bytes.
        let publication = ledger.assemble_owned(&captured, &mut tables).unwrap();
        assert_eq!(handoff.pending(), 1);
        assert_eq!(log.lanes[0].consumed.load(Ordering::Acquire), before);
        drop(publication);
        assert_eq!(
            unsafe { &*(log.lanes[0].byte_ring.load(Ordering::Acquire) as *const ByteRing) }
                .consumed
                .load(Ordering::Acquire),
            bytes_before
        );
        let publication = ledger.assemble_owned(&captured, &mut tables).unwrap();
        assert!(matches!(&tables[TableId::OperationCompletions.index()],
            TableInput::Sampled(TableView::Complete(TableRows::OperationCompletions(rows)))
                if rows.len() == 1 && rows[0].effect_description == b"retained"));
        publication.commit();
        assert_eq!(handoff.pending(), 0);
        assert!(log.lanes[0].consumed.load(Ordering::Acquire) > before);
        assert!(
            unsafe { &*(log.lanes[0].byte_ring.load(Ordering::Acquire) as *const ByteRing) }
                .consumed
                .load(Ordering::Acquire)
                > bytes_before
        );
        // The next native completion remains independently recordable.
        complete(&log, &fixture, 1);
        assert_eq!(drain(&log).unwrap().1, b"retained");
    }

    #[test]
    fn operation_completion_player_and_ai_are_owned_before_same_update_deletion() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(2, 32);
        prime(&log);
        for (owner, description) in [(1, b"player".as_slice()), (2, b"AI".as_slice())] {
            let fixture = Fixture::new(description);
            fixture.identify(owner, owner);
            complete(&log, &fixture, 0xfeed_0001);
            // Manager deletes both synthetic owners in the same update.
            fixture.delete();
        }
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(
            events.iter().map(|event| event.owner).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(events[0].completion_hour, -7);
        assert_eq!(
            (events[0].terminal, events[0].bonus, events[0].detected),
            (1, 1, 0)
        );
        assert_eq!(bytes, b"playerAI");
    }
    #[test]
    fn operation_completion_auto_repeat_has_distinct_instances_and_invalidation_emits_none() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(4, 32);
        prime(&log);
        let fixture = Fixture::new(&[0xff, 0, b'x']);
        fixture.identify(42, 700);
        complete(&log, &fixture, 1);
        fixture.identify(42, 701); // Manager creates a new auto-repeat instance.
        complete(&log, &fixture, 1);
        fixture.delete();
        // The invalidation path calls no completion. An
        // unsuccessful terminal/completion call returns AL0 and reads nothing.
        complete(&log, &fixture, 0x1234_0000);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events.len(), 2);
        assert_ne!(events[0].id, events[1].id);
        assert!(events[1].sequence > events[0].sequence);
        assert_eq!(bytes, [0xff, 0, b'x', 0xff, 0, b'x']);
    }
    #[test]
    fn table_retry_preserves_admitted_operation_completion() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(1, 32);
        prime(&log);
        let fixture = Fixture::new(b"modded outcome");
        complete(&log, &fixture, 1);
        let mut cursors = [LaneCursor::default()];
        let handoff = handoff_from(&log, &mut cursors).unwrap();
        let shared = crate::sampling::Shared::new(0, 0, 1);
        shared.request_retry_full();
        let recovery = shared.force_next.swap(false, Ordering::AcqRel);
        let refresh = shared.refresh_next.swap(false, Ordering::AcqRel);
        if recovery {
            invalidate();
        }
        let mut events = [Event::default()];
        let mut bytes = [0; 32];
        let drained = handoff.drain(&cursors, &mut events, &mut bytes).unwrap();
        assert_eq!(drained.copied, 1);
        assert_eq!(events[0].definition_token, 0xfabc_1234);
        assert_eq!(&bytes[..drained.bytes], b"modded outcome");
        assert!(refresh);
        assert!(!shared.force_next.load(Ordering::Acquire));
    }

    #[test]
    fn operation_completion_overflow_withholds_whole_interval_and_retains_exact_growth() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(1, 2);
        prime(&log);
        let first = Fixture::new(b"ok");
        complete(&log, &first, 1);
        let long = Fixture::new(&vec![0xfd; 40_000]);
        complete(&log, &long, 1);
        assert_eq!(
            drain(&log),
            Err("operation completion observation restarted or lost")
        );
        assert_eq!(log.lanes[0].required_entries.load(Ordering::Relaxed), 2);
        assert_eq!(log.lanes[0].required_bytes.load(Ordering::Relaxed), 40_002);
        assert!(drain(&log).unwrap().0.is_empty());
        let grown = new_log(2, 40_000);
        prime(&grown);
        complete(&grown, &long, 1);
        assert_eq!(drain(&grown).unwrap().1, vec![0xfd; 40_000]);
    }
    #[test]
    fn operation_completion_overflow_counts_all_dropped_attempts_then_resets_at_seal() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(1, 16);
        prime(&log);
        let fixture = Fixture::new(b"burst");
        for _ in 0..5_000 {
            complete(&log, &fixture, 1);
        }
        assert_eq!(log.lanes[0].required_entries.load(Ordering::Relaxed), 5_000);
        assert!(drain(&log).is_err());
        assert_eq!(
            log.lanes[0].consumed_attempts.load(Ordering::Acquire),
            5_000
        );
        complete(&log, &fixture, 1);
        assert_eq!(drain(&log).unwrap().0.len(), 1);
    }
    #[test]
    fn operation_completion_resize_keeps_keys_monotonic_and_abandons_pending_hour_explicitly() {
        let _serial = TEST.lock().unwrap();
        let old = new_log(2, 16);
        prime(&old);
        let fixture = Fixture::new(b"outcome");
        complete(&old, &fixture, 1);
        let first = drain(&old).unwrap().0[0].sequence;
        complete(&old, &fixture, 1); // Pending when off-thread replacement occurs.
        invalidate();
        let fresh = new_log(4, 32);
        complete(&fresh, &fixture, 1);
        assert_eq!(
            drain(&fresh),
            Err("operation completion observation restarted or lost")
        );
        assert_eq!(
            drain(&old),
            Err("operation completion observation restarted or lost")
        );
        complete(&fresh, &fixture, 1);
        let second = drain(&fresh).unwrap().0[0].sequence;
        assert!(second > first);
    }
    #[test]
    fn operation_completion_every_country_5000_keeps_native_values_without_status_gate() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(5_000, 16);
        prime(&log);
        let fixture = Fixture::new(b"modded outcome");
        for owner in 1..=5_000 {
            fixture.identify(owner, owner * 7);
            fixture.bytes.borrow_mut()[INSTANCE + operation::DETECTED] = (owner % 2) as u8;
            complete(&log, &fixture, 1);
        }
        fixture.delete();
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events.len(), 5_000);
        assert_eq!(events.last().unwrap().owner, 5_000);
        assert_eq!(events.last().unwrap().id, ((35_000_u64) << 32) | 4713);
        assert_eq!(bytes.len(), 5_000 * b"modded outcome".len());
        assert!(
            events
                .iter()
                .all(|event| event.definition_token == 0xfabc_1234)
        );
    }
    #[test]
    fn operation_completion_live_description_floor_prepares_the_first_burst() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(5000, 0);
        prime(&log);
        let description = vec![b'x'; 5001];
        for lane in &log.lanes {
            lane.grow_bytes_for(5000 * description.len()).unwrap();
        }
        let fixture = Fixture::new(&description);
        crate::test_support::with_allocations_denied(|| {
            for owner in 1..=5000 {
                fixture.identify(owner, owner);
                complete(&log, &fixture, 1);
            }
        });
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events.len(), 5000);
        assert!(events.iter().all(|event| event.description_error == 0));
        assert_eq!(bytes.len(), 5000 * description.len());
        for copied in bytes.chunks_exact(description.len()) {
            assert_eq!(copied, description);
        }
        let later = Fixture::new(&vec![b'y'; 10003]);
        for lane in &log.lanes {
            lane.grow_bytes_for(10003).unwrap();
        }
        complete(&log, &later, 1);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events[0].description_error, 0);
        assert_eq!(bytes.len(), 10003);
    }
    #[test]
    fn operation_completion_worker_handoff_grows_original_lane_count_before_sealing() {
        let _serial = TEST.lock().unwrap();
        let mut cursors = Vec::new();
        for count in [5000, 12000] {
            let log = Log::new(1, 0, count, &[]).unwrap();
            log.registered.store(count, Ordering::Release);
            for (index, lane) in log.lanes.iter().enumerate() {
                lane.published.store(index as u64, Ordering::Release);
                lane.published_attempts
                    .store(index as u64 + 7, Ordering::Release);
            }
            let epoch = EPOCH.load(Ordering::Acquire);
            let handoff =
                handoff_buffer(&log, engine::walk::Buffer::Growing(&mut cursors)).unwrap();
            assert_eq!(handoff.lanes, count);
            assert_eq!(handoff.epoch, epoch);
            assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
            assert_eq!(cursors[count - 1].cursor, count as u64 - 1);
            assert_eq!(cursors[count - 1].attempts, count as u64 + 6);
            assert!(handoff_from(&log, &mut []).is_err());
        }
    }
    #[test]
    fn operation_completion_refused_native_form_is_named_counted_and_withholds_interval() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(1, 16);
        prime(&log);
        let fixture = Fixture::new(b"invalid");
        fixture.bytes.borrow_mut()[INSTANCE + operation::DETECTED] = 2;
        let index = FAILURE_NAMES
            .iter()
            .position(|reason| *reason == "operation completion stored flags invalid")
            .unwrap();
        let before = FAILURES[index].load(Ordering::Relaxed);
        complete(&log, &fixture, 1);
        assert_eq!(FAILURES[index].load(Ordering::Relaxed), before + 1);
        assert!(drain(&log).is_err());
    }
    #[test]
    fn operation_completion_off_switch_installs_no_hook_or_sentinel_read() {
        let _serial = TEST.lock().unwrap();
        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("disabled operation outcomes must not read a hook site");
            }
        }
        let before = COUNTERS[3].load(Ordering::Relaxed);
        let hooks = unsafe { Hooks::new(&NoRead, usize::MAX, false) };
        assert!(hooks.entry.is_none());
        assert!(!RUNTIME_ENABLED.load(Ordering::Relaxed));
        assert_eq!(COUNTERS[3].load(Ordering::Relaxed), before);
        let logs = LOGS.lock().unwrap().len();
        resize(usize::MAX);
        assert_eq!(LOGS.lock().unwrap().len(), logs);
        RUNTIME_ENABLED.store(true, Ordering::Relaxed);
    }

    #[test]
    fn operation_completion_idle_stale_handoff_after_later_drain_and_capacity_change() {
        use crate::extended::operation_outcomes::{Capacities, Captured, Ledger};
        use replay::{
            frame::{TableInput, TableView},
            rows::{TableId, TableRows},
        };
        let _serial = TEST.lock().unwrap();
        let log = new_log(4, 32);
        prime(&log);
        let fixture = Fixture::new(b"outcome");
        complete(&log, &fixture, 1);
        let mut cursors = vec![LaneCursor::default()];
        let old = handoff_from(&log, &mut cursors).unwrap();
        let idle = Captured::sealed(old, cursors.clone());
        let mut ledger = Ledger::default();
        let mut tables = vec![
            TableInput::Sampled(TableView::Unavailable {
                reason: "test".into()
            });
            TableId::ALL.len()
        ];
        if let Some(publication) = ledger.assemble_owned(&idle, &mut tables) {
            publication.commit();
        }
        assert!(
            matches!(&tables[TableId::OperationCompletions.index()], TableInput::Sampled(TableView::Complete(TableRows::OperationCompletions(rows))) if rows.len() == 1)
        );
        complete(&log, &fixture, 1);
        assert_eq!(drain(&log).unwrap().0.len(), 1);
        assert_eq!(old.pending(), 0);
        assert_eq!(old.description_capacity(&cursors).unwrap(), 0);
        let consumed = log.consumed.load(Ordering::Acquire);
        let index = FAILURE_NAMES
            .iter()
            .position(|name| *name == "operation completion stale handoff")
            .unwrap();
        let before = FAILURES[index].load(Ordering::Relaxed);
        let reads = COUNTERS[4].load(Ordering::Relaxed);
        let epoch = EPOCH.load(Ordering::Acquire);
        assert_eq!(
            old.drain(&cursors, &mut [], &mut []),
            Err("operation completion stale handoff")
        );
        assert_eq!(FAILURES[index].load(Ordering::Relaxed), before + 1);
        assert_eq!(log.consumed.load(Ordering::Acquire), consumed);
        assert_eq!(COUNTERS[4].load(Ordering::Relaxed), reads);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        // Idle growth uses the retained drain-time demand, not a later cursor.
        assert_eq!(idle.drained_pending(), 1);
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(5_000);
        let expected = capacity;
        capacity.raise_for_operations(idle.drained_pending());
        assert!(capacity == expected);
        if let Some(publication) = ledger.assemble_owned(&idle, &mut tables) {
            publication.commit();
        }
        assert!(
            matches!(&tables[TableId::OperationCompletions.index()], TableInput::Sampled(TableView::Unavailable { reason }) if reason == "operation completion stale handoff")
        );
        assert_eq!(idle.drained_pending(), 0);
        complete(&log, &fixture, 1);
        assert_eq!(drain(&log).unwrap().0.len(), 1);
    }

    #[test]
    fn operation_completion_long_description_wraps_exactly_without_hour_loss() {
        let _serial = TEST.lock().unwrap();
        let log = Log::new(4, 65_536, 1, &[]).unwrap();
        log.registered.store(1, Ordering::Relaxed);
        prime(&log);
        let description: Vec<_> = (0..40_000)
            .map(|index| ((index * 17 + index / 7) % 256) as u8)
            .collect();
        let fixture = Fixture::new(&description);
        for _ in 0..3 {
            complete(&log, &fixture, 1);
            let (events, bytes) = drain(&log).unwrap();
            assert_eq!(events[0].description_error, 0);
            assert_eq!(events[0].description_required, 40_000);
            assert_eq!(bytes, description);
        }
    }

    #[test]
    fn operation_completion_native_demand_above_old_budget_grows_and_records_exact_bytes() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(4, 32);
        prime(&log);
        let description = vec![0xff; (1 << 20) + 1];
        let fixture = Fixture::new(&description);
        fixture.identify(5_000, 91);
        complete(&log, &fixture, 1);
        let epoch = EPOCH.load(Ordering::Acquire);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events[0].description_error, 2);
        assert_eq!(events[0].description_required, description.len() as u64);
        assert!(bytes.is_empty());
        assert_eq!(
            log.lanes[0].required_bytes.load(Ordering::Relaxed),
            description.len()
        );
        log.lanes[0].grow_bytes().unwrap();
        complete(&log, &fixture, 1);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events[0].description_error, 0);
        assert_eq!(bytes, description);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
    }

    #[test]
    fn operation_completion_byte_growth_preserves_pending_records_and_epoch() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(4, 4);
        prime(&log);
        complete(&log, &Fixture::new(b"before"), 1);
        let long = Fixture::new(&vec![0xfc; 40_000]);
        let epoch = EPOCH.load(Ordering::Acquire);
        let reads = COUNTERS[4].load(Ordering::Relaxed);
        let index = FAILURE_NAMES
            .iter()
            .position(|name| *name == "operation completion description byte ring short")
            .unwrap();
        let before = FAILURES[index].load(Ordering::Relaxed);
        complete(&log, &long, 1);
        let old_ring = log.lanes[0].byte_ring.load(Ordering::Acquire);
        log.lanes[0].grow_bytes().unwrap();
        assert_ne!(log.lanes[0].byte_ring.load(Ordering::Acquire), old_ring);
        assert_eq!(log.lanes[0].ring().bytes.len(), 65_536);
        // One sealed batch can include records in both old and grown rings.
        complete(&log, &long, 1);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.description_error)
                .collect::<Vec<_>>(),
            [0, 2, 0]
        );
        assert_eq!(&bytes[..6], b"before");
        assert_eq!(&bytes[6..], vec![0xfc; 40_000]);
        assert_eq!(events[1].description_len, 0);
        assert_eq!(events[1].description_required, 40_000);
        assert_eq!(FAILURES[index].load(Ordering::Relaxed), before + 1);
        assert_eq!(COUNTERS[4].load(Ordering::Relaxed), reads);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        complete(&log, &long, 1);
        assert_eq!(drain(&log).unwrap().1, vec![0xfc; 40_000]);
    }

    #[test]
    fn operation_completion_byte_demand_drain_excludes_later_unsealed_callback() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(4, 4);
        prime(&log);
        complete(&log, &Fixture::new(b"sealed"), 1);
        let mut cursors = [LaneCursor::default()];
        let sealed = handoff_from(&log, &mut cursors).unwrap();
        complete(&log, &Fixture::new(&[0xfb; 10]), 1);
        sealed
            .drain(&cursors, &mut [Event::default()], &mut [0; 6])
            .unwrap();
        assert_eq!(log.lanes[0].consumed_byte_demand.load(Ordering::Acquire), 6);
        complete(&log, &Fixture::new(&[0xfc; 9]), 1);
        assert_eq!(log.lanes[0].required_bytes.load(Ordering::Relaxed), 19);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.description_error)
                .collect::<Vec<_>>(),
            [0, 2]
        );
        assert_eq!(bytes, [0xfb; 10]);
    }

    #[test]
    fn operation_completion_wrapped_inline_bytes_and_second_part_read_failure_are_exact() {
        let _serial = TEST.lock().unwrap();
        let log = Log::new(4, 64, 1, &[]).unwrap();
        log.registered.store(1, Ordering::Relaxed);
        prime(&log);
        complete(&log, &Fixture::new(&[0xfa; 60]), 1);
        assert_eq!(drain(&log).unwrap().1.len(), 60);
        complete(&log, &Fixture::new(b"inline-wrap"), 1);
        assert_eq!(drain(&log).unwrap().1, b"inline-wrap");
        struct FailTail(Fixture);
        impl Memory for FailTail {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                address != PAYLOAD + 57 && self.0.copy(address, output)
            }
        }
        // Byte cursor is 71; a 60-byte heap string splits at offset 57.
        let index = FAILURE_NAMES
            .iter()
            .position(|name| *name == "operation completion description bytes unreadable")
            .unwrap();
        let before = FAILURES[index].load(Ordering::Relaxed);
        observe_return(
            &log,
            Target::Lane(&log.lanes[0]),
            &FailTail(Fixture::new(&[0xfd; 60])),
            BASE,
            INSTANCE,
            1,
        );
        assert_eq!(FAILURES[index].load(Ordering::Relaxed), before + 1);
        assert!(drain(&log).is_err());
    }

    #[test]
    fn operation_completion_failed_payload_demand_does_not_accumulate_after_empty_publication() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(4, 16);
        prime(&log);
        struct FailPayload(Fixture);
        impl Memory for FailPayload {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                address != PAYLOAD && self.0.copy(address, output)
            }
        }
        let fixture = FailPayload(Fixture::new(&[0xfd; 32]));
        let lane = &log.lanes[0];
        let ring = lane.byte_ring.load(Ordering::Acquire);
        for _ in 0..4 {
            observe_return(&log, Target::Lane(lane), &fixture, BASE, INSTANCE, 1);
            assert_eq!(
                drain(&log),
                Err("operation completion observation restarted or lost")
            );
            assert!(drain(&log).unwrap().0.is_empty());
            assert_eq!(unsafe { *lane.byte_demand.get() }, 0);
            assert_eq!(lane.consumed_byte_demand.load(Ordering::Acquire), 0);
            assert_eq!(unsafe { *lane.ring().write.get() }, 0);
            assert_eq!(lane.required_bytes.load(Ordering::Relaxed), 32);
            lane.grow_bytes().unwrap();
            assert_eq!(lane.byte_ring.load(Ordering::Acquire), ring);
        }
        complete(&log, &Fixture::new(b"recorded"), 1);
        assert_eq!(drain(&log).unwrap().1, b"recorded");
        assert_eq!(lane.consumed_byte_demand.load(Ordering::Acquire), 8);
    }

    #[test]
    fn operation_completion_callback_incomplete_retains_its_unwind_claim() {
        let _serial = TEST.lock().unwrap();
        let before = COUNTERS[9].load(Ordering::Relaxed);
        let reads = COUNTERS[4].load(Ordering::Relaxed);
        let active = ACTIVE.load(Ordering::SeqCst);
        let epoch = EPOCH.load(Ordering::Acquire);
        let mut outer = Call::enter();
        drop(Call::enter());
        outer.returned = true;
        drop(outer);
        assert_eq!(COUNTERS[9].load(Ordering::Relaxed), before + 1);
        assert_eq!(COUNTERS[4].load(Ordering::Relaxed), reads);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), active + 1);
        assert_eq!(DEPTH.get(), 0);
        let mut later = Call::enter();
        later.returned = true;
        drop(later);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), active + 1);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch + 1);
        assert_eq!(
            serde_json::to_value(status()).unwrap()["callback_incomplete"],
            before + 1
        );
        UNWOUND.set(false);
        ACTIVE.store(active, Ordering::SeqCst);
    }

    #[test]
    fn operation_completion_unknown_refusal_counts_without_panicking_or_invalidating() {
        let _serial = TEST.lock().unwrap();
        let before = FAILURES[FAILURE_NAMES.len()].load(Ordering::Relaxed);
        let reads = COUNTERS[4].load(Ordering::Relaxed);
        let epoch = EPOCH.load(Ordering::Acquire);
        count("synthetic unrecognized refusal");
        assert_eq!(
            FAILURES[FAILURE_NAMES.len()].load(Ordering::Relaxed),
            before + 1
        );
        assert_eq!(COUNTERS[4].load(Ordering::Relaxed), reads);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
    }
    #[test]
    fn operation_completion_growth_within_installed_capacity_keeps_pending_events() {
        let _serial = TEST.lock().unwrap();
        unsafe {
            release();
        }
        let description = b"pending before country count grew";
        presize(4_097, description.len());
        let old = log().unwrap();
        old.registered.store(1, Ordering::Relaxed);
        prime(old);
        complete(old, &Fixture::new(description), 1);
        let pointer = LOG.load(Ordering::Acquire);
        let epoch = EPOCH.load(Ordering::Acquire);
        resize(4_098);
        assert_eq!(LOG.load(Ordering::Acquire), pointer);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        assert_eq!(drain(old).unwrap().1, description);
        unsafe {
            release();
        }
    }

    #[test]
    fn operation_completion_resumed_attachment_continues_finalized_sequence() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(2, 32);
        prime(&log);
        let saved = EVENT_SEQUENCE.load(Ordering::Relaxed).saturating_add(17);
        restore_sequence(saved).unwrap();
        complete(&log, &Fixture::new(b"after reattach"), 1);
        let events = drain(&log).unwrap().0;
        assert_eq!(events[0].sequence, saved + 1);
        // A second campaign with older events cannot rewind the global key.
        restore_sequence(saved - 1).unwrap();
        complete(&log, &Fixture::new(b"other campaign"), 1);
        assert_eq!(drain(&log).unwrap().0[0].sequence, saved + 2);
    }

    /// The production callback path after the native original returns.
    fn complete_through(log: &Log, fixture: &Fixture) {
        let lease = publishing(log).unwrap();
        observe_return(lease.0, lane(lease.0), fixture, BASE, INSTANCE, 1);
    }

    #[test]
    fn operation_completion_writers_beyond_the_lane_count_lose_nothing_in_one_generation() {
        let _serial = TEST.lock().unwrap();
        let required = REQUIRED_LANES.load(Ordering::Relaxed);
        let shared_threads = COUNTERS[11].load(Ordering::Relaxed);
        // The generation's only lane is already registered.
        let log = new_log(8, 32);
        prime(&log);
        let epoch = EPOCH.load(Ordering::Acquire);
        std::thread::scope(|scope| {
            for owner in 1..=3_u32 {
                let log = &log;
                scope.spawn(move || {
                    let description = format!("thread {owner}");
                    let fixture = Fixture::new(description.as_bytes());
                    fixture.identify(owner, owner);
                    complete_through(log, &fixture);
                });
            }
        });
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        assert!(COUNTERS[11].load(Ordering::Relaxed) >= shared_threads + 3);
        assert!(REQUIRED_LANES.load(Ordering::Relaxed) >= 4);
        let (events, bytes) = drain(&log).unwrap();
        let mut owners = events.iter().map(|event| event.owner).collect::<Vec<_>>();
        owners.sort_unstable();
        assert_eq!(owners, [1, 2, 3]);
        for event in &events {
            assert_eq!(
                &bytes[event.description_start..event.description_start + event.description_len],
                format!("thread {}", event.owner).as_bytes()
            );
        }
        assert!(log.shared.published(|r| r.order).unwrap().is_empty());
        REQUIRED_LANES.store(required, Ordering::Relaxed);
    }

    #[test]
    fn operation_completion_shared_slot_short_keeps_its_record_and_sizes_the_next_slot() {
        let _serial = TEST.lock().unwrap();
        let log = new_log(2, 4);
        prime(&log);
        let long = vec![0xfe; 9];
        let index = FAILURE_NAMES
            .iter()
            .position(|name| *name == "operation completion description byte ring short")
            .unwrap();
        let before = FAILURES[index].load(Ordering::Relaxed);
        observe_return(
            &log,
            Target::Shared,
            &Fixture::new(&long),
            BASE,
            INSTANCE,
            1,
        );
        assert_eq!(FAILURES[index].load(Ordering::Relaxed), before + 1);
        assert!(SHARED_DESCRIPTION_DEMAND.load(Ordering::Relaxed) >= 9);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            (events[0].description_error, events[0].description_required),
            (2, 9)
        );
        assert!(bytes.is_empty());
        // An exhausted shared lane is a named, counted refusal.
        let refused = FAILURE_NAMES
            .iter()
            .position(|name| *name == "operation completion shared lane exhausted")
            .unwrap();
        let refusals = FAILURES[refused].load(Ordering::Relaxed);
        let held: Vec<_> = (0..log.shared.len())
            .map(|_| log.shared.reserve().unwrap())
            .collect();
        let refused_fixture = Fixture::new(b"x");
        crate::test_support::with_allocations_denied(|| {
            observe_return(&log, Target::Shared, &refused_fixture, BASE, INSTANCE, 1);
        });
        assert_eq!(FAILURES[refused].load(Ordering::Relaxed), refusals + 1);
        assert!(log.shared.required_entries() > log.shared.len());
        drop(held);
    }

    #[test]
    fn operation_completion_lane_growth_seals_the_generation_without_losing_its_interval() {
        let _serial = TEST.lock().unwrap();
        unsafe {
            release();
        }
        let required = REQUIRED_LANES.load(Ordering::Relaxed);
        presize(8, 64);
        let old = log().unwrap();
        old.registered.store(1, Ordering::Relaxed);
        prime(old);
        complete(old, &Fixture::new(b"before growth"), 1);
        let epoch = EPOCH.load(Ordering::Acquire);
        // A thread registered beyond every lane asks for a dedicated one.
        REQUIRED_LANES.fetch_max(old.lanes.len() + 1, Ordering::Relaxed);
        presize(8, 64);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        assert!(std::ptr::eq(log().unwrap(), old));
        let fresh = offered().unwrap();
        assert!(fresh.lanes.len() > old.lanes.len());
        // The lane boundary seals the old generation for its own handoff.
        let sealed = crate::test_support::with_allocations_denied(|| seal_offered(old));
        assert!(std::ptr::eq(sealed, old));
        assert!(std::ptr::eq(log().unwrap(), fresh));
        assert!(offered().is_none());
        // A callback still holding the old generation follows its successor.
        complete_through(old, &Fixture::new(b"after seal"));
        let (events, bytes) = drain(old).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(bytes, b"before growth");
        let (events, bytes) = drain(fresh).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(bytes, b"after seal");
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        REQUIRED_LANES.store(required, Ordering::Relaxed);
        unsafe {
            release();
        }
    }
}
