use collector_hooks::hook::Hook;
use collector_primitives::shared_lane::SharedLane;
use engine_events::operation_outcome_sites as native;
use engine_native::Memory;
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
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}
static LOG: AtomicUsize = AtomicUsize::new(0);
static OFFERED: AtomicUsize = AtomicUsize::new(0);
static SHARED_DESCRIPTION_DEMAND: AtomicUsize = AtomicUsize::new(0);
static SERIAL: AtomicU64 = AtomicU64::new(1);
static EVENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static SPILL_REFUSED_EPOCH: AtomicU64 = AtomicU64::new(0);
const SPILL_REFUSED: &str = "operation completion spill allocation failed";
#[allow(clippy::vec_box)]
static LOGS: Mutex<Vec<Box<Log>>> = Mutex::new(Vec::new());
static UNHOOKED: OnceLock<std::borrow::Cow<'static, str>> = OnceLock::new();
static REQUIRED_LANES: AtomicUsize = AtomicUsize::new(1);
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
    let mut values = engine_allocation::vec(1)
        .map_err(|_| "operation completion description allocation failed")?;
    values.push(value);
    let slice = boxed_slice(values)?;
    Ok(unsafe { Box::from_raw(Box::into_raw(slice).cast::<T>()) })
}
fn boxed_slice<T>(values: Vec<T>) -> Result<Box<[T]>, &'static str> {
    engine_allocation::boxed_slice(values)
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
        engine_allocation::reserve_exact(&mut storage, bytes)
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
    assembled: AtomicU64,
    assembled_attempts: AtomicU64,
    required_entries: AtomicUsize,
    required_bytes: AtomicUsize,
}
unsafe impl Sync for Lane {}
impl Lane {
    pub fn new(entries: usize, bytes: usize) -> Result<Self, &'static str> {
        let entries = entries.max(1).checked_next_power_of_two().ok_or_else(|| {
            let _ = engine_allocation::oom();
            "operation completion description capacity overflow"
        })?;
        let ring = boxed(ByteRing::try_new(bytes.max(1))?)?;
        let pointer = &*ring as *const ByteRing as usize;
        let mut records = Vec::new();
        engine_allocation::reserve_exact(&mut records, entries)
            .map_err(|_| "operation completion description allocation failed")?;
        records.resize_with(entries, || UnsafeCell::new(Record::default()));
        let mut byte_rings = Vec::new();
        engine_allocation::reserve_exact(&mut byte_rings, 1)
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
            assembled: AtomicU64::new(0),
            assembled_attempts: AtomicU64::new(0),
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
        self.grow_bytes_for_with(floor, || {})
    }
    fn grow_bytes_for_with(
        &self,
        floor: usize,
        before_publish: impl FnOnce(),
    ) -> Result<(), &'static str> {
        let required = self.required_bytes.load(Ordering::Relaxed).max(floor);
        if required > self.ring().bytes.len() {
            let capacity = required.checked_next_power_of_two().ok_or_else(|| {
                let _ = engine_allocation::oom();
                "operation completion description capacity overflow"
            })?;
            let fresh = boxed(ByteRing::try_new(capacity)?)?;
            before_publish();
            let pointer = &*fresh as *const ByteRing as usize;
            let mut retained = self
                .byte_rings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if capacity <= self.ring().bytes.len() {
                return Ok(());
            }
            engine_allocation::reserve(&mut retained, 1)
                .map_err(|_| "operation completion description allocation failed")?;
            retained.push(fresh);
            self.byte_ring.store(pointer, Ordering::Release);
        }
        Ok(())
    }
}
pub struct Log {
    serial: u64,
    lanes: Box<[Lane]>,
    shared: SharedLane<Record>,
    registered: AtomicUsize,
    entries: usize,
    sequence: AtomicU64,
    consumed: AtomicU64,
    consumed_epoch: AtomicU64,
    assembled: AtomicU64,
    assembled_epoch: AtomicU64,
    born_epoch: u64,
    publishers: AtomicUsize,
    next: AtomicUsize,
}
impl Log {
    #[cfg(any(test, feature = "test-support"))]
    pub fn new(
        entries: usize,
        bytes: usize,
        lanes: usize,
        required: &[(usize, usize)],
    ) -> Result<Self, &'static str> {
        Self::with_shared(entries, bytes, lanes, required, (0, 0))
    }
    fn with_shared(
        entries: usize,
        bytes: usize,
        lanes: usize,
        required: &[(usize, usize)],
        shared: (usize, usize),
    ) -> Result<Self, &'static str> {
        let entries = entries.max(1).checked_next_power_of_two().ok_or_else(|| {
            let _ = engine_allocation::oom();
            "operation completion description capacity overflow"
        })?;
        let slots = entries
            .div_ceil(lanes.max(1))
            .max(shared.0)
            .checked_next_power_of_two()
            .ok_or("operation completion description capacity overflow")?;
        let stride = shared.1.max(bytes.div_ceil(slots));
        let shared = SharedLane::new(slots, stride)
            .map_err(|_| "operation completion description allocation failed")?;
        let mut storage = Vec::new();
        engine_allocation::reserve_exact(&mut storage, lanes)
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
            assembled: AtomicU64::new(0),
            assembled_epoch: AtomicU64::new(0),
            born_epoch: EPOCH.load(Ordering::SeqCst),
            publishers: AtomicUsize::new(0),
            next: AtomicUsize::new(0),
        })
    }
    fn reset_admitted(&self) {
        for lane in &self.lanes {
            lane.assembled
                .store(lane.consumed.load(Ordering::Acquire), Ordering::Release);
            lane.assembled_attempts.store(
                lane.consumed_attempts.load(Ordering::Acquire),
                Ordering::Release,
            );
        }
        self.assembled
            .store(self.consumed.load(Ordering::Acquire), Ordering::Release);
        let assembled = self.assembled_epoch.load(Ordering::SeqCst);
        let _ = self.assembled_epoch.compare_exchange(
            assembled,
            self.consumed_epoch.load(Ordering::SeqCst),
            Ordering::SeqCst,
            Ordering::Relaxed,
        );
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
            UNWOUND.set(true);
            restart();
        }
        DEPTH.with(|depth| {
            depth.set(depth.get() - 1);
            if depth.get() == 0 && !UNWOUND.get() {
                ACTIVE.fetch_sub(1, Ordering::SeqCst);
            }
        });
    }
}
fn invalidate_prepared() {
    restart();
}
/// # Safety
/// Caller must own the active session's capture, recovery, or publication failure path (or hold
/// `tests::serial()` in an isolated test) when invalidating live operation-completion observation.
pub unsafe fn invalidate() {
    invalidate_prepared();
}
/// # Safety
/// Caller must own the active session's capture, recovery, or publication failure path (or hold
/// `tests::serial()` in an isolated test) when recording a lost operation-completion drain.
pub unsafe fn lost_drain() {
    // SAFETY: Caller owns the operation-completion recovery or publication failure path.
    unsafe { invalidate() };
}
fn restart() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
}
fn failed() {
    restart();
}
fn overflow() {
    restart();
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
                REQUIRED_LANES.fetch_max(index + 1, Ordering::Relaxed);
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
                    failed();
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
fn seal_offered(log: &'static Log) -> &'static Log {
    let _retained = match LOGS.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return log,
    };
    let Some(offered) = offered() else {
        return log;
    };
    let epoch = EPOCH.load(Ordering::Acquire);
    offered.consumed_epoch.store(epoch, Ordering::SeqCst);
    offered.assembled_epoch.store(epoch, Ordering::SeqCst);
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

/// # Safety
/// Run only while operation-log storage remains live for the active session (or an isolated test
/// holding `tests::serial()`) and [`release`] is excluded; once attached, concurrent growth calls
/// rely on `LOGS` and per-lane `byte_rings` locks.
pub unsafe fn resize(entries: usize) {
    presize(entries, 0);
}
pub(crate) fn presize(entries: usize, bytes: usize) {
    presize_with(entries, bytes, || {});
}
fn same_log(a: Option<&Log>, b: Option<&Log>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        (None, None) => true,
        _ => false,
    }
}
fn sources_contain(sources: [Option<&Log>; 2], target: &Log) -> bool {
    sources
        .into_iter()
        .flatten()
        .any(|source| std::ptr::eq(source, target))
}
fn presize_with(entries: usize, bytes: usize, before_publish: impl FnMut()) {
    presize_with_hooks(entries, bytes, || {}, before_publish);
}
fn presize_with_hooks(
    entries: usize,
    bytes: usize,
    mut before_grow: impl FnMut(),
    mut before_publish: impl FnMut(),
) {
    if !RUNTIME_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    loop {
        let sources = [log(), offered()];
        let generations = || sources.into_iter().flatten();
        let target_entries = entries.max(generations().map(|log| log.entries).max().unwrap_or(1));
        before_grow();
        for log in generations() {
            for (index, lane) in log.lanes.iter().enumerate() {
                let floor = generations()
                    .filter_map(|other| other.lanes.get(index))
                    .map(|other| other.ring().bytes.len())
                    .max()
                    .unwrap_or(0)
                    .max(bytes);
                let _ = lane.grow_bytes_for(floor);
            }
        }
        let lanes =
            thread_slots().max(generations().map(|log| log.lanes.len()).max().unwrap_or(0));
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
        let satisfies = |log: &Log| {
            log.entries >= target_entries
                && log.lanes.len() >= lanes
                && log.shared.len() >= shared.0
                && log.shared.stride() >= shared.1
                && log
                    .lanes
                    .iter()
                    .enumerate()
                    .all(|(index, lane)| lane.records.len() >= required(index).0)
        };
        if generations().any(satisfies) {
            let retained = LOGS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = [log(), offered()];
            let latest = current[1].or(current[0]);
            let changed = !same_log(sources[0], current[0]) || !same_log(sources[1], current[1]);
            let needs_byte_growth = current.into_iter().flatten().any(|existing| {
                !sources_contain(sources, existing)
                    && existing.lanes.iter().enumerate().any(|(index, lane)| {
                        lane.ring().bytes.len() < bytes.max(required(index).1)
                    })
            });
            let satisfied = latest.is_some_and(satisfies) && !needs_byte_growth;
            drop(retained);
            if satisfied || (!changed && latest.is_some_and(satisfies)) {
                return;
            }
            continue;
        }
        let fresh = (|| {
            let required = engine_allocation::collect_vec((0..lanes).map(required))
                .map_err(|_| "operation completion description allocation failed")?;
            boxed(Log::with_shared(
                target_entries,
                bytes,
                lanes,
                &required,
                shared,
            )?)
        })();
        let Ok(fresh) = fresh else {
            failed();
            return;
        };
        before_publish();
        let pointer = &*fresh as *const Log as usize;
        let mut retained = LOGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let current = [log(), offered()];
        let current_generations = || current.into_iter().flatten();
        let latest = current[1].or(current[0]);
        let satisfied_by_latest = latest.is_some_and(|existing| {
            existing.entries >= target_entries
                && existing.lanes.len() >= lanes
                && existing.shared.len() >= shared.0
                && existing.shared.stride() >= shared.1
                && existing.lanes.iter().enumerate().all(|(index, lane)| {
                    lane.records.len() >= required(index).0
                        && lane.ring().bytes.len() >= bytes.max(required(index).1)
                })
        });
        if satisfied_by_latest {
            return;
        }
        let dominates_current = current_generations().all(|existing| {
            fresh.entries >= existing.entries
                && fresh.lanes.len() >= existing.lanes.len()
                && fresh.shared.len() >= existing.shared.len()
                && fresh.shared.stride() >= existing.shared.stride()
                && existing.lanes.iter().enumerate().all(|(index, lane)| {
                    fresh.lanes[index].records.len() >= lane.records.len()
                        && fresh.lanes[index].ring().bytes.len() >= lane.ring().bytes.len()
                })
        });
        if !dominates_current {
            drop(retained);
            drop(fresh);
            continue;
        }
        if engine_allocation::reserve(&mut retained, 1).is_err() {
            failed();
            return;
        }
        retained.push(fresh);
        if current[0].is_none() {
            LOG.store(pointer, Ordering::Release);
        } else {
            OFFERED.store(pointer, Ordering::Release);
        }
        return;
    }
}
pub(crate) fn reset_admitted() {
    let current = LOG.load(Ordering::Acquire);
    for log in LOGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
    {
        if &**log as *const Log as usize != current
            && log.assembled.load(Ordering::Acquire) != log.consumed.load(Ordering::Acquire)
        {
            restart();
        }
        log.reset_admitted();
    }
}
/// # Safety
/// Hooks are disabled; executable ranges/ACTIVE have quiesced and every worker
/// holding a Handoff or log reference is joined. The early return retains the storage only while
/// observation is still enabled or a call is active; it cannot see a worker's reference.
pub unsafe fn release() {
    if ENABLED.load(Ordering::SeqCst) || ACTIVE.load(Ordering::SeqCst) != 0 {
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
        Target::Shared => {
            let length = observation.description_len();
            if length > log.shared.stride() {
                SHARED_DESCRIPTION_DEMAND.fetch_max(length, Ordering::Relaxed);
            }
            append_shared(
                log,
                Event::from_native(observation.row),
                &observation,
                memory,
            );
        }
    }
}
fn append_shared(
    log: &Log,
    mut event: Event,
    observation: &native::Observation,
    memory: &impl Memory,
) {
    let length = observation.description_len();
    event.description_required = length as u64;
    let Some(mut slot) = log.shared.reserve_bytes(length) else {
        spill_refused();
        return;
    };
    let output = unsafe { std::slice::from_raw_parts_mut(slot.bytes(), length) };
    if observation.copy_description(memory, output).is_err() {
        failed();
        return;
    }
    (event.description_error, event.description_len) = (0, length);
    let Ok(sequence) = EVENT_SEQUENCE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
        last.checked_add(1)
    }) else {
        failed();
        return;
    };
    event.sequence = sequence + 1;
    let order = log.sequence.fetch_add(1, Ordering::Relaxed) + 1;
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
        failed();
        return;
    };
    let required_bytes =
        byte_demand.saturating_sub(lane.consumed_byte_demand.load(Ordering::Acquire));
    let Ok(required_bytes) = usize::try_from(required_bytes) else {
        failed();
        return;
    };
    lane.required_bytes
        .fetch_max(required_bytes, Ordering::Relaxed);
    if demand > lane.records.len() as u64 {
        lane.required_entries
            .fetch_max(demand as usize, Ordering::Relaxed);
        append_shared(log, event, &observation, memory);
        return;
    }
    if byte_pending.saturating_add(length as u64) > ring.bytes.len() as u64 {
        append_shared(log, event, &observation, memory);
        return;
    }
    let start = byte_start as usize % ring.bytes.len();
    let first_len = length.min(ring.bytes.len() - start);
    let first = unsafe { std::slice::from_raw_parts_mut(ring.pointer(start), first_len) };
    let second = unsafe { std::slice::from_raw_parts_mut(ring.pointer(0), length - first_len) };
    if observation
        .copy_description_parts(memory, first, second)
        .is_err()
    {
        failed();
        return;
    }
    (event.description_error, event.description_len) = (0, length);
    let index = cursor as usize & (lane.records.len() - 1);
    let Ok(sequence) = EVENT_SEQUENCE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
        last.checked_add(1)
    }) else {
        failed();
        return;
    };
    event.sequence = sequence + 1;
    let order = log.sequence.fetch_add(1, Ordering::Relaxed) + 1;
    unsafe {
        *lane.byte_demand.get() = byte_demand;
        *ring.write.get() = byte_start + length as u64;
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
fn spill_refused() {
    let epoch = EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    SPILL_REFUSED_EPOCH.fetch_max(epoch, Ordering::SeqCst);
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
        Err(_) => failed(),
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
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::OperationOutcomes,
    );
    let mut active = Call::enter();
    let original: NativeFunction = unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    let generation = collector_lifetime::world_lifetime::generation();
    let result = timing.original(|| unsafe { original(instance) });
    if result as u8 != 0 && ENABLED.load(Ordering::Acquire) {
        if generation != collector_lifetime::world_lifetime::generation()
            || collector_lifetime::world_lifetime::invalidating()
        {
            restart();
            active.returned = true;
            return result;
        }
        if let Some(log) = log() {
            if let Some(lease) = publishing(log) {
                let memory = unsafe { engine_native::memory::LiveMemory::new() };
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
            restart();
        }
    }
    active.returned = true;
    result
}
#[unsafe(no_mangle)]
pub static BATTLEPLAN_OPERATION_OUTCOME_CALLBACKS: [NativeFunction; 1] = [completion];

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
pub(crate) fn handoff_growing(cursors: &mut Vec<LaneCursor>) -> Result<Handoff, &'static str> {
    if UNHOOKED.get().is_some() || !ENABLED.load(Ordering::Acquire) {
        return Err("operation completion hooks unavailable");
    }
    let log = log().ok_or("operation completion staging unavailable")?;
    // SAFETY: `log` is a retained generation in `LOGS` kept live until `release()` after workers join.
    unsafe {
        handoff_buffer(
            seal_offered(log),
            engine_native::walk::Buffer::Growing(cursors),
        )
    }
}
/// # Safety
/// `log` must remain alive and pinned at its address for every subsequent call on the returned
/// [`Handoff`], and only the single spool or isolated fixture owning this handoff may consume it.
pub unsafe fn handoff_buffer(
    log: &Log,
    mut cursors: engine_native::walk::Buffer<'_, LaneCursor>,
) -> Result<Handoff, &'static str> {
    let lanes = log.registered.load(Ordering::Acquire).min(log.lanes.len());
    cursors
        .ensure(lanes)
        .map_err(|_| "operation completion handoff allocation failed")?;
    if cursors.len() < lanes {
        restart();
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
    pub(crate) fn pending(self) -> usize {
        let log = unsafe { &*(self.log as *const Log) };
        self.sequence
            .saturating_sub(log.assembled.load(Ordering::Acquire)) as usize
    }
    pub(crate) fn description_capacity(self, cursors: &[LaneCursor]) -> Result<usize, &'static str> {
        let log = unsafe { &*(self.log as *const Log) };
        if self.sequence < log.assembled.load(Ordering::Acquire) {
            return Ok(0);
        }
        let mut bytes = 0_usize;
        for (lane, end) in log.lanes.iter().zip(cursors).take(self.lanes) {
            for cursor in lane.assembled.load(Ordering::Acquire)..end.cursor {
                let length = unsafe {
                    (*lane.records[cursor as usize & (lane.records.len() - 1)].get())
                        .event
                        .description_len
                };
                bytes = engine_allocation::add(bytes, length)
                    .map_err(|_| "operation completion description demand overflow")?;
            }
        }
        for (_, index) in self.shared(log)? {
            bytes = engine_allocation::add(bytes, log.shared.record(index).0.event.description_len)
                .map_err(|_| "operation completion description demand overflow")?;
        }
        Ok(bytes)
    }
    fn shared(self, log: &Log) -> Result<Vec<(u64, u64)>, &'static str> {
        let first = log.assembled.load(Ordering::Acquire);
        let mut shared = log
            .shared
            .published(|record| record.order)
            .map_err(|_| "operation completion description allocation failed")?;
        shared.retain(|&(order, _)| order > first && order <= self.sequence);
        Ok(shared)
    }
    pub(crate) fn admitted(self, cursors: &[LaneCursor]) {
        let log = unsafe { &*(self.log as *const Log) };
        let assembled = log.assembled.load(Ordering::Acquire);
        if self.sequence < assembled
            || (self.sequence == assembled
                && self.epoch < log.assembled_epoch.load(Ordering::Acquire))
            || cursors.len() < self.lanes
        {
            return;
        }
        for (lane, end) in log.lanes.iter().zip(cursors).take(self.lanes) {
            lane.assembled.fetch_max(end.cursor, Ordering::Release);
            lane.assembled_attempts
                .fetch_max(end.attempts, Ordering::Release);
        }
        log.assembled.store(self.sequence, Ordering::Release);
        log.assembled_epoch.store(self.epoch, Ordering::Release);
    }
    pub(crate) fn commit(self, cursors: &[LaneCursor]) {
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
                lane.consumed_byte_demand
                    .store(record.byte_demand, Ordering::Release);
            }
            lane.consumed.store(end.cursor, Ordering::Release);
            lane.consumed_attempts
                .store(end.attempts, Ordering::Release);
        }
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
        self.admitted(cursors);
    }
    /// # Safety
    /// The [`Log`] captured by this [`Handoff`] must still be alive at its original address,
    /// `cursors` must match its sealed lane cursors, and only the single owning consumer may drain it.
    #[cfg(any(test, feature = "test-support"))]
    pub unsafe fn drain(
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
    pub(crate) fn prepare(
        self,
        cursors: &[LaneCursor],
        output: &mut [Event],
        bytes: &mut [u8],
    ) -> Result<Drained, &'static str> {
        let log = unsafe { &*(self.log as *const Log) };
        let cursors = cursors
            .get(..self.lanes)
            .ok_or("operation completion drain cursors short")?;
        let first = log.assembled.load(Ordering::Acquire);
        if log.lanes.iter().zip(cursors).any(|(lane, end)| {
            end.cursor < lane.assembled.load(Ordering::Acquire)
                || end.attempts < lane.assembled_attempts.load(Ordering::Acquire)
        }) {
            return Err("operation completion stale handoff");
        }
        let Some(pending) = self
            .sequence
            .checked_sub(first)
            .map(|pending| pending as usize)
        else {
            return Err("operation completion stale handoff");
        };
        if log.assembled_epoch.load(Ordering::Acquire) != self.epoch
            || self.epoch != EPOCH.load(Ordering::Acquire)
        {
            let refused = SPILL_REFUSED_EPOCH.load(Ordering::Acquire);
            let since = log
                .assembled_epoch
                .load(Ordering::Acquire)
                .max(log.born_epoch);
            return Err(
                if refused > since && refused <= EPOCH.load(Ordering::Acquire) {
                    SPILL_REFUSED
                } else {
                    "operation completions were not followed in the game without a break"
                },
            );
        }
        if output.len() < pending || bytes.len() < self.description_capacity(cursors)? {
            restart();
            return Err("operation completion drain staging short");
        }
        let tails = engine_allocation::collect_vec(
            log.lanes
                .iter()
                .take(self.lanes)
                .map(|lane| lane.assembled.load(Ordering::Acquire)),
        )
        .map_err(|_| "operation completion description allocation failed")?;
        let shared = self.shared(log)?;
        let (copied, used) = self.merge(cursors, tails, &shared, output, bytes)?;
        if copied != pending || self.epoch != EPOCH.load(Ordering::Acquire) {
            restart();
            return Err("operation completion publication incomplete or lost");
        }
        Ok(Drained {
            copied,
            bytes: used,
        })
    }
    fn merge(
        self,
        cursors: &[LaneCursor],
        mut tails: Vec<u64>,
        shared: &[(u64, u64)],
        output: &mut [Event],
        bytes: &mut [u8],
    ) -> Result<(usize, usize), &'static str> {
        let log = unsafe { &*(self.log as *const Log) };
        let first = log.assembled.load(Ordering::Acquire);
        let pending = self.sequence.saturating_sub(first) as usize;
        let mut shared = shared.iter().peekable();
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
                    (Source::Lane(index), unsafe { *lane.records[slot].get() })
                })
                .chain(
                    shared
                        .peek()
                        .map(|&&(_, index)| (Source::Shared, log.shared.record(index).0)),
                )
                .min_by_key(|(_, record)| record.order);
            let Some((source, record)) = next else {
                break;
            };
            if record.order != first + copied as u64 + 1 {
                break;
            }
            let mut event = record.event;
            let end = used
                .checked_add(event.description_len)
                .filter(|end| *end <= bytes.len())
                .ok_or("operation completion drain staging short")?;
            match source {
                Source::Shared => {
                    let &(_, slot) = shared.next().unwrap();
                    bytes[used..end]
                        .copy_from_slice(&log.shared.record(slot).1[..event.description_len]);
                }
                Source::Lane(index) => {
                    let ring = unsafe { &*(record.byte_ring as *const ByteRing) };
                    if event.description_len > ring.bytes.len() {
                        return Err("operation completion description byte ring short");
                    }
                    let start = record.byte_start as usize % ring.bytes.len();
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
                    tails[index] += 1;
                }
            }
            event.description_start = used;
            used = end;
            output[copied] = event;
            copied += 1;
        }
        Ok((copied, used))
    }
}
#[derive(Clone, Copy)]
enum Source {
    Lane(usize),
    Shared,
}

pub struct Hooks {
    entry: Option<Hook>,
}
impl Hooks {
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize, runtime_enabled: bool) -> Self {
        RUNTIME_ENABLED.store(runtime_enabled, Ordering::Relaxed);
        if !runtime_enabled {
            let _ = UNHOOKED.set(std::borrow::Cow::Borrowed(
                "operation outcomes disabled by attach config",
            ));
            return Self { entry: None };
        }
        unsafe { Self::install(memory, base) }.unwrap_or_else(|reason| {
            collector_hooks::hook::installation::refused(
                collector_hooks::hook::installation::Kind::Construction,
                None,
                &reason,
            );
            let _ = UNHOOKED.set(std::borrow::Cow::Owned(reason));
            Self { entry: None }
        })
    }
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    unsafe fn install(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        let (at, original) = collector_hooks::located_hook(base, native::COMPLETION_RVA)
            .map_err(|cause| engine_allocation::string(cause).unwrap_or_default())?;
        BASE.store(base, Ordering::Relaxed);
        let hook = unsafe { Hook::new(at, completion as *const () as usize, original) }?;
        ORIGINAL.store(hook.trampoline(), Ordering::Release);
        Ok(Self { entry: Some(hook) })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        if !collector_hooks::hook::shared::defer_preparation(invalidate_prepared) {
            restart();
        }
        let Some(hook) = self.entry.as_mut() else {
            return Ok(true);
        };
        let mut family = [hook];
        match Hook::set_enabled_batch(&mut family, enabled) {
            Ok(false) => return Ok(false),
            Ok(true) => {}
            Err(reason) => {
                restart();
                return Err(reason);
            }
        }
        ENABLED.store(enabled && self.entry.is_some(), Ordering::SeqCst);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        engine_allocation::owned_vec(engine_allocation::collect_vec(
            self.entry.iter().flat_map(Hook::executable_ranges),
        ))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use engine_native::{ccountry, cgame_state, coperation, coperation_instance as operation};
    use std::cell::RefCell;

    static TEST: Mutex<()> = Mutex::new(());
    thread_local! {
        static HELD: Cell<bool> = const { Cell::new(false) };
    }
    pub(super) struct Exclusive(Option<std::sync::MutexGuard<'static, ()>>);
    impl Drop for Exclusive {
        fn drop(&mut self) {
            if self.0.is_some() {
                HELD.set(false);
            }
        }
    }
    pub(super) fn exclusive() -> Exclusive {
        if HELD.get() {
            return Exclusive(None);
        }
        let turn = TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        HELD.set(true);
        Exclusive(Some(turn))
    }
    pub struct Serial {
        required: usize,
        active: usize,
        _turn: Exclusive,
    }
    /// # Safety
    /// Caller runs in an isolated test/fixture process without live native operation-completion
    /// hooks enabled or active; `Serial` serializes fixture access and restores `ACTIVE` and
    /// `REQUIRED_LANES` on drop.
    pub unsafe fn serial() -> Serial {
        let turn = exclusive();
        RUNTIME_ENABLED.store(true, Ordering::Relaxed);
        UNWOUND.set(false);
        Serial {
            required: REQUIRED_LANES.load(Ordering::Relaxed),
            active: ACTIVE.load(Ordering::SeqCst),
            _turn: turn,
        }
    }
    impl Drop for Serial {
        fn drop(&mut self) {
            REQUIRED_LANES.store(self.required, Ordering::Relaxed);
            ACTIVE.store(self.active, Ordering::SeqCst);
            UNWOUND.set(false);
            RUNTIME_ENABLED.store(true, Ordering::Relaxed);
        }
    }
    const BASE: usize = 0x1000_0000;
    const INSTANCE: usize = 0x1000;
    const COUNTRY: usize = 0x2000;
    const DEFINITION: usize = 0x3000;
    const STATE: usize = 0x4000;
    const PAYLOAD: usize = 0x5000;
    pub struct Fixture {
        bytes: RefCell<Vec<u8>>,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == BASE + cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
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
        pub fn new(description: &[u8]) -> Self {
            let mut bytes = vec![0; (PAYLOAD + description.len() + 1).max(0x6000)];
            let put = |bytes: &mut [u8], address: usize, value: u64| {
                bytes[address..address + 8].copy_from_slice(&value.to_le_bytes())
            };
            put(
                &mut bytes,
                INSTANCE,
                (BASE + operation::VTABLE_RVA.0) as u64,
            );
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
            put(&mut bytes, COUNTRY, (BASE + ccountry::VTABLE_RVA.0) as u64);
            bytes[COUNTRY + ccountry::ID..COUNTRY + ccountry::ID + 4]
                .copy_from_slice(&1_u32.to_le_bytes());
            put(
                &mut bytes,
                DEFINITION,
                (BASE + coperation::VTABLE_RVA.0) as u64,
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
        #[cfg(test)]
        fn identify(&self, owner: u32, instance: u32) {
            let mut bytes = self.bytes.borrow_mut();
            bytes[COUNTRY + ccountry::ID..COUNTRY + ccountry::ID + 4]
                .copy_from_slice(&owner.to_le_bytes());
            bytes[INSTANCE + operation::HANDLE..INSTANCE + operation::HANDLE + 8]
                .copy_from_slice(&((u64::from(instance) << 32) | 4713).to_le_bytes());
        }
        #[cfg(test)]
        fn delete(&self) {
            self.bytes.borrow_mut()[INSTANCE..INSTANCE + 0x188].fill(0);
        }
    }
    pub fn new_log(entries: usize, stride: usize) -> Log {
        let log = Log::new(entries, entries * stride, 1, &[]).unwrap();
        log.registered.store(1, Ordering::Relaxed);
        log
    }
    fn drain(log: &Log) -> Result<(Vec<Event>, Vec<u8>), &'static str> {
        let _exclusive = exclusive();
        let mut cursors = vec![LaneCursor::default()];
        // SAFETY: `_exclusive` serializes fixture access and `cursors` is locally owned.
        let handoff =
            unsafe { handoff_buffer(log, engine_native::walk::Buffer::Growing(&mut cursors)) }?;
        let mut events = vec![Event::default(); handoff.pending()];
        let mut bytes = vec![0; handoff.description_capacity(&cursors).unwrap()];
        // SAFETY: `handoff` was sealed from `log` under `_exclusive` as the sole consumer.
        let drained = unsafe { handoff.drain(&cursors, &mut events, &mut bytes) }?;
        events.truncate(drained.copied);
        bytes.truncate(drained.bytes);
        Ok((events, bytes))
    }
    /// # Safety
    /// Caller holds [`serial()`] and exclusively owns `log` with no live hooks or concurrent
    /// consumers.
    pub unsafe fn prime(log: &Log) {
        assert!(drain(log).is_err());
        assert_eq!(drain(log).unwrap().0.len(), 0);
    }
    /// # Safety
    /// Caller must hold [`serial()`] with no concurrent native or fixture operation completions
    /// (completion advances process-wide `EVENT_SEQUENCE`) and must be the sole writer to
    /// `log.lanes[0]`.
    pub unsafe fn complete(log: &Log, fixture: &Fixture, result: usize) {
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
            collector_test_support::with_allocations_denied(|| ByteRing::try_new(1024)),
            Err("operation completion description allocation failed")
        ));
        assert!(matches!(
            collector_test_support::with_allocations_denied(|| Lane::new(4097, 1024)),
            Err("operation completion description allocation failed")
        ));
        assert!(matches!(
            collector_test_support::with_allocations_denied(|| Log::new(4097, 1024, 2, &[])),
            Err("operation completion description allocation failed")
        ));
    }

    #[test]
    fn unavailable_completion_hook_does_not_block_installation_or_detach() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
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
        use replay_data::{rows::TableId, rows::TableRows};
        use replay_frame::{frame::TableInput, frame::TableView};
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(4, 32);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let fixture = Fixture::new(b"retained");
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        let mut cursors = vec![LaneCursor::default()];
        // SAFETY: test holds `serial()` and is the sole consumer of `log`.
        let handoff =
            unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors)) }
                .unwrap();
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
        // SAFETY: test holds `serial()` as the sole spool consumer of `captured`.
        let publication = unsafe { ledger.assemble_owned(&captured, &mut tables) }.unwrap();
        assert_eq!(handoff.pending(), 1);
        assert_eq!(log.lanes[0].consumed.load(Ordering::Acquire), before);
        drop(publication);
        assert_eq!(
            unsafe { &*(log.lanes[0].byte_ring.load(Ordering::Acquire) as *const ByteRing) }
                .consumed
                .load(Ordering::Acquire),
            bytes_before
        );
        // SAFETY: test holds `serial()` and reassembles the uncommitted interval.
        let publication = unsafe { ledger.assemble_owned(&captured, &mut tables) }.unwrap();
        assert!(matches!(&tables[TableId::OperationCompletions.index()],
            TableInput::Sampled(TableView::Complete(TableRows::OperationCompletions(rows)))
                if rows.len() == 1 && rows[0].effect_description == b"retained"));
        // SAFETY: test holds `serial()` and commits the assembled publication.
        unsafe { publication.commit() };
        assert_eq!(handoff.pending(), 0);
        assert!(log.lanes[0].consumed.load(Ordering::Acquire) > before);
        assert!(
            unsafe { &*(log.lanes[0].byte_ring.load(Ordering::Acquire) as *const ByteRing) }
                .consumed
                .load(Ordering::Acquire)
                > bytes_before
        );
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        assert_eq!(drain(&log).unwrap().1, b"retained");
    }

    #[test]
    fn operation_completion_player_and_ai_are_owned_before_same_update_deletion() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(2, 32);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        for (owner, description) in [(1, b"player".as_slice()), (2, b"AI".as_slice())] {
            let fixture = Fixture::new(description);
            fixture.identify(owner, owner);
            // SAFETY: Test is the sole writer to lane 0 of this log.
            unsafe { complete(&log, &fixture, 0xfeed_0001) };
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
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(4, 32);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let fixture = Fixture::new(&[0xff, 0, b'x']);
        fixture.identify(42, 700);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        fixture.identify(42, 701);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        fixture.delete();
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 0x1234_0000) };
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events.len(), 2);
        assert_ne!(events[0].id, events[1].id);
        assert!(events[1].sequence > events[0].sequence);
        assert_eq!(bytes, [0xff, 0, b'x', 0xff, 0, b'x']);
    }
    #[test]
    fn operation_completion_past_the_lane_spills_whole_and_retains_exact_growth() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(1, 2);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let first = Fixture::new(b"ok");
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &first, 1) };
        let long = Fixture::new(&vec![0xfd; 40_000]);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &long, 1) };
        assert_eq!(log.lanes[0].required_entries.load(Ordering::Relaxed), 2);
        assert_eq!(log.lanes[0].required_bytes.load(Ordering::Relaxed), 40_002);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| (event.description_error, event.description_len))
                .collect::<Vec<_>>(),
            [(0, 2), (0, 40_000)]
        );
        assert_eq!(&bytes[..2], b"ok");
        assert_eq!(&bytes[2..], vec![0xfd; 40_000]);
        assert!(log.shared.is_drained());
        let grown = new_log(2, 40_000);
        // SAFETY: Test holds `serial()` and exclusively owns `grown`.
        unsafe { prime(&grown) };
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&grown, &long, 1) };
        assert_eq!(drain(&grown).unwrap().1, vec![0xfd; 40_000]);
    }
    #[test]
    fn operation_completion_a_burst_past_the_lane_keeps_every_attempt_and_sizes_the_next() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(1, 16);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let fixture = Fixture::new(b"burst");
        for _ in 0..5_000 {
            // SAFETY: Test is the sole writer to lane 0 of this log.
            unsafe { complete(&log, &fixture, 1) };
        }
        assert_eq!(log.lanes[0].required_entries.load(Ordering::Relaxed), 5_000);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events.len(), 5_000);
        assert!(
            events
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        assert_eq!(bytes, b"burst".repeat(5_000));
        assert_eq!(
            log.lanes[0].consumed_attempts.load(Ordering::Acquire),
            5_000
        );
        assert!(log.shared.is_drained());
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        assert_eq!(drain(&log).unwrap().0.len(), 1);
    }
    #[test]
    fn operation_completion_a_burst_past_the_lane_with_hours_in_flight_keeps_every_completion() {
        use crate::extended::operation_outcomes::{Captured, Ledger};
        use replay_data::{rows::TableId, rows::TableRows};
        use replay_frame::{frame::TableInput, frame::TableView};
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(2, 4);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let sealed = || {
            let mut cursors = vec![LaneCursor::default()];
            // SAFETY: test holds `serial()` as the sole consumer of `log`.
            let handoff =
                unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors)) }
                    .unwrap();
            Captured::sealed(handoff, cursors)
        };
        let rows = |tables: &[TableInput]| match &tables[TableId::OperationCompletions.index()] {
            TableInput::Sampled(TableView::Complete(TableRows::OperationCompletions(rows))) => rows
                .iter()
                .map(|row| {
                    (
                        row.owner,
                        row.description_error,
                        row.effect_description.clone(),
                    )
                })
                .collect::<Vec<_>>(),
            _ => panic!("operation completions not complete"),
        };
        let mut ledger = Ledger::default();
        let mut tables = vec![
            TableInput::Sampled(TableView::Unavailable {
                reason: "test".into(),
            });
            TableId::ALL.len()
        ];
        let mut queued = Vec::new();
        let mut owner = 0_u32;
        for hour in 0..3 {
            let mut expected = Vec::new();
            for _ in 0..3 {
                owner += 1;
                let description = vec![owner as u8; owner as usize];
                let fixture = Fixture::new(&description);
                fixture.identify(owner, owner);
                // SAFETY: Test is the sole writer to lane 0 of this log.
                unsafe { complete(&log, &fixture, 1) };
                expected.push((owner, 0, description));
            }
            // SAFETY: test holds `serial()` and assembles intervals in order.
            let publication = unsafe { ledger.assemble_owned(&sealed(), &mut tables) }.unwrap();
            assert_eq!(rows(&tables), expected, "hour {hour}");
            // SAFETY: test holds `serial()` and admits publications in assembly order.
            unsafe { publication.admitted() };
            queued.push(publication);
        }
        assert_eq!(log.consumed.load(Ordering::Acquire), 0);
        assert_eq!(log.shared.pending(), 7);
        for publication in queued {
            // SAFETY: test holds `serial()` and commits publications in assembly order.
            unsafe { publication.commit() };
        }
        assert_eq!(log.consumed.load(Ordering::Acquire), 9);
        assert!(log.shared.is_drained());
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"after"), 1) };
        assert_eq!(drain(&log).unwrap().1, b"after");
    }
    #[test]
    fn operation_completion_resize_keeps_keys_monotonic_and_abandons_pending_hour_explicitly() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        spill_refused();
        let old = new_log(2, 16);
        // SAFETY: Test holds `serial()` and exclusively owns `old`.
        unsafe { prime(&old) };
        let fixture = Fixture::new(b"outcome");
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&old, &fixture, 1) };
        let first = drain(&old).unwrap().0[0].sequence;
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&old, &fixture, 1) };
        unsafe { invalidate() };
        let fresh = new_log(4, 32);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&fresh, &fixture, 1) };
        assert_eq!(
            drain(&fresh),
            Err("operation completions were not followed in the game without a break")
        );
        assert_eq!(
            drain(&old),
            Err("operation completions were not followed in the game without a break")
        );
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&fresh, &fixture, 1) };
        let second = drain(&fresh).unwrap().0[0].sequence;
        assert!(second > first);
    }
    #[test]
    fn operation_completion_every_country_keeps_native_values_without_status_gate() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(40, 16);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let fixture = Fixture::new(b"modded outcome");
        for owner in 1..=40 {
            fixture.identify(owner, owner * 7);
            fixture.bytes.borrow_mut()[INSTANCE + operation::DETECTED] = (owner % 2) as u8;
            // SAFETY: Test is the sole writer to lane 0 of this log.
            unsafe { complete(&log, &fixture, 1) };
        }
        fixture.delete();
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events.len(), 40);
        assert_eq!(events.last().unwrap().owner, 40);
        assert_eq!(events.last().unwrap().id, ((280_u64) << 32) | 4713);
        assert_eq!(bytes.len(), 40 * b"modded outcome".len());
        assert!(
            events
                .iter()
                .all(|event| event.definition_token == 0xfabc_1234)
        );
    }
    #[test]
    fn operation_completion_live_description_floor_prepares_the_first_burst() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(5000, 0);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let description = vec![b'x'; 5001];
        for lane in &log.lanes {
            lane.grow_bytes_for(5000 * description.len()).unwrap();
        }
        let fixture = Fixture::new(&description);
        collector_test_support::with_allocations_denied(|| {
            for owner in 1..=5000 {
                fixture.identify(owner, owner);
                // SAFETY: Test is the sole writer to lane 0 of this log.
                unsafe { complete(&log, &fixture, 1) };
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
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &later, 1) };
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events[0].description_error, 0);
        assert_eq!(bytes.len(), 10003);
    }
    #[test]
    fn operation_completion_worker_handoff_grows_original_lane_count_before_sealing() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
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
            // SAFETY: test holds `serial()` and owns `log` with no concurrent writers.
            let handoff =
                unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors)) }
                    .unwrap();
            assert_eq!(handoff.lanes, count);
            assert_eq!(handoff.epoch, epoch);
            assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
            assert_eq!(cursors[count - 1].cursor, count as u64 - 1);
            assert_eq!(cursors[count - 1].attempts, count as u64 + 6);
            // SAFETY: test holds `serial()` and exercises count-only buffer refusal on `log`.
            assert!(unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Count) }.is_err());
        }
    }
    #[test]
    fn operation_completion_refused_native_form_withholds_interval() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(1, 16);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let fixture = Fixture::new(b"invalid");
        fixture.bytes.borrow_mut()[INSTANCE + operation::DETECTED] = 2;
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        assert!(drain(&log).is_err());
    }
    #[test]
    fn operation_completion_off_switch_installs_no_hook_or_sentinel_read() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("disabled operation outcomes must not read a hook site");
            }
        }
        let hooks = unsafe { Hooks::new(&NoRead, usize::MAX, false) };
        assert!(hooks.entry.is_none());
        assert!(!RUNTIME_ENABLED.load(Ordering::Relaxed));
        let logs = LOGS.lock().unwrap().len();
        // SAFETY: test holds `serial()` with runtime disabled and no live hooks.
        unsafe { resize(usize::MAX) };
        assert_eq!(LOGS.lock().unwrap().len(), logs);
    }

    #[test]
    fn operation_completion_idle_stale_handoff_after_later_drain_and_capacity_change() {
        use crate::extended::operation_outcomes::{Capacities, Captured, Ledger};
        use replay_data::{rows::TableId, rows::TableRows};
        use replay_frame::{frame::TableInput, frame::TableView};
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(4, 32);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let fixture = Fixture::new(b"outcome");
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        let mut cursors = vec![LaneCursor::default()];
        // SAFETY: test holds `serial()` as the sole consumer of `log`.
        let old =
            unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors)) }
                .unwrap();
        let idle = Captured::sealed(old, cursors.clone());
        let mut ledger = Ledger::default();
        let mut tables = vec![
            TableInput::Sampled(TableView::Unavailable {
                reason: "test".into()
            });
            TableId::ALL.len()
        ];
        // SAFETY: test holds `serial()` as the sole spool consumer.
        if let Some(publication) = unsafe { ledger.assemble_owned(&idle, &mut tables) } {
            // SAFETY: test holds `serial()` and commits the assembled publication.
            unsafe { publication.commit() };
        }
        assert!(
            matches!(&tables[TableId::OperationCompletions.index()], TableInput::Sampled(TableView::Complete(TableRows::OperationCompletions(rows))) if rows.len() == 1)
        );
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        assert_eq!(drain(&log).unwrap().0.len(), 1);
        assert_eq!(old.pending(), 0);
        assert_eq!(old.description_capacity(&cursors).unwrap(), 0);
        let consumed = log.consumed.load(Ordering::Acquire);
        let epoch = EPOCH.load(Ordering::Acquire);
        // SAFETY: test holds `serial()` and verifies stale-handoff rejection on `old`.
        assert_eq!(
            unsafe { old.drain(&cursors, &mut [], &mut []) },
            Err("operation completion stale handoff")
        );
        assert_eq!(log.consumed.load(Ordering::Acquire), consumed);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        assert_eq!(idle.drained_pending(), 1);
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(5_000);
        let expected = capacity;
        capacity.raise_for_operations(idle.drained_pending());
        assert!(capacity == expected);
        // SAFETY: test holds `serial()` and verifies stale-handoff assembly refusal.
        if let Some(publication) = unsafe { ledger.assemble_owned(&idle, &mut tables) } {
            // SAFETY: test holds `serial()` and commits any assembled publication.
            unsafe { publication.commit() };
        }
        assert!(
            matches!(&tables[TableId::OperationCompletions.index()], TableInput::Sampled(TableView::Unavailable { reason }) if reason == "operation completion stale handoff")
        );
        assert_eq!(idle.drained_pending(), 0);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        assert_eq!(drain(&log).unwrap().0.len(), 1);
    }

    #[test]
    fn operation_completion_long_description_wraps_exactly_without_hour_loss() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = Log::new(4, 65_536, 1, &[]).unwrap();
        log.registered.store(1, Ordering::Relaxed);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let description: Vec<_> = (0..40_000)
            .map(|index| ((index * 17 + index / 7) % 256) as u8)
            .collect();
        let fixture = Fixture::new(&description);
        for _ in 0..3 {
            // SAFETY: Test is the sole writer to lane 0 of this log.
            unsafe { complete(&log, &fixture, 1) };
            let (events, bytes) = drain(&log).unwrap();
            assert_eq!(events[0].description_error, 0);
            assert_eq!(events[0].description_required, 40_000);
            assert_eq!(bytes, description);
        }
    }

    #[test]
    fn operation_completion_native_demand_above_old_budget_grows_and_records_exact_bytes() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(4, 32);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let description = vec![0xff; (1 << 20) + 1];
        let fixture = Fixture::new(&description);
        fixture.identify(5_000, 91);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        let epoch = EPOCH.load(Ordering::Acquire);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events[0].description_error, 0);
        assert_eq!(events[0].description_required, description.len() as u64);
        assert_eq!(bytes, description);
        assert_eq!(
            log.lanes[0].required_bytes.load(Ordering::Relaxed),
            description.len()
        );
        log.lanes[0].grow_bytes().unwrap();
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events[0].description_error, 0);
        assert_eq!(bytes, description);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
    }

    #[test]
    fn operation_completion_byte_growth_preserves_pending_records_and_epoch() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(4, 4);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"before"), 1) };
        let long = Fixture::new(&vec![0xfc; 40_000]);
        let epoch = EPOCH.load(Ordering::Acquire);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &long, 1) };
        let old_ring = log.lanes[0].byte_ring.load(Ordering::Acquire);
        log.lanes[0].grow_bytes().unwrap();
        assert_ne!(log.lanes[0].byte_ring.load(Ordering::Acquire), old_ring);
        assert_eq!(log.lanes[0].ring().bytes.len(), 65_536);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &long, 1) };
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.description_error)
                .collect::<Vec<_>>(),
            [0, 0, 0]
        );
        assert_eq!(&bytes[..6], b"before");
        assert_eq!(&bytes[6..], vec![0xfc; 80_000]);
        assert_eq!(events[1].description_len, 40_000);
        assert_eq!(events[1].description_required, 40_000);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &long, 1) };
        assert_eq!(drain(&log).unwrap().1, vec![0xfc; 40_000]);
    }

    #[test]
    fn operation_completion_byte_demand_drain_excludes_later_unsealed_callback() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(4, 4);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"sealed"), 1) };
        let mut cursors = vec![LaneCursor::default()];
        // SAFETY: test holds `serial()` as the sole consumer of `log`.
        let sealed =
            unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors)) }
                .unwrap();
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(&[0xfb; 10]), 1) };
        // SAFETY: test holds `serial()` and drains `sealed` as the sole consumer.
        unsafe { sealed.drain(&cursors, &mut [Event::default()], &mut [0; 6]) }.unwrap();
        assert_eq!(log.lanes[0].consumed_byte_demand.load(Ordering::Acquire), 6);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(&[0xfc; 9]), 1) };
        assert_eq!(log.lanes[0].required_bytes.load(Ordering::Relaxed), 19);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.description_error)
                .collect::<Vec<_>>(),
            [0, 0]
        );
        assert_eq!(bytes, [[0xfb; 10].as_slice(), &[0xfc; 9]].concat());
    }

    #[test]
    fn operation_completion_wrapped_inline_bytes_and_second_part_read_failure_are_exact() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = Log::new(4, 64, 1, &[]).unwrap();
        log.registered.store(1, Ordering::Relaxed);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(&[0xfa; 60]), 1) };
        assert_eq!(drain(&log).unwrap().1.len(), 60);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"inline-wrap"), 1) };
        assert_eq!(drain(&log).unwrap().1, b"inline-wrap");
        struct FailTail(Fixture);
        impl Memory for FailTail {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                address != PAYLOAD + 57 && self.0.copy(address, output)
            }
        }
        observe_return(
            &log,
            Target::Lane(&log.lanes[0]),
            &FailTail(Fixture::new(&[0xfd; 60])),
            BASE,
            INSTANCE,
            1,
        );
        assert!(drain(&log).is_err());
    }

    #[test]
    fn operation_completion_failed_payload_demand_does_not_accumulate_after_empty_publication() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(4, 16);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
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
                Err("operation completions were not followed in the game without a break")
            );
            assert!(drain(&log).unwrap().0.is_empty());
            assert_eq!(unsafe { *lane.byte_demand.get() }, 0);
            assert_eq!(lane.consumed_byte_demand.load(Ordering::Acquire), 0);
            assert_eq!(unsafe { *lane.ring().write.get() }, 0);
            assert_eq!(lane.required_bytes.load(Ordering::Relaxed), 32);
            lane.grow_bytes().unwrap();
            assert_eq!(lane.byte_ring.load(Ordering::Acquire), ring);
        }
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"recorded"), 1) };
        assert_eq!(drain(&log).unwrap().1, b"recorded");
        assert_eq!(lane.consumed_byte_demand.load(Ordering::Acquire), 8);
    }

    #[test]
    fn operation_completion_callback_incomplete_retains_its_unwind_claim() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let active = ACTIVE.load(Ordering::SeqCst);
        let epoch = EPOCH.load(Ordering::Acquire);
        let mut outer = Call::enter();
        drop(Call::enter());
        outer.returned = true;
        drop(outer);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), active + 1);
        assert_eq!(DEPTH.get(), 0);
        let mut later = Call::enter();
        later.returned = true;
        drop(later);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), active + 1);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch + 1);
    }

    #[test]
    fn operation_completion_growth_within_installed_capacity_keeps_pending_events() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        unsafe {
            release();
        }
        let description = b"pending before country count grew";
        presize(4_097, description.len());
        let old = log().unwrap();
        old.registered.store(1, Ordering::Relaxed);
        // SAFETY: Test holds `serial()` and exclusively owns `old`.
        unsafe { prime(old) };
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(old, &Fixture::new(description), 1) };
        let pointer = LOG.load(Ordering::Acquire);
        let epoch = EPOCH.load(Ordering::Acquire);
        // SAFETY: test holds `serial()` with no concurrent log sizing.
        unsafe { resize(4_098) };
        assert_eq!(LOG.load(Ordering::Acquire), pointer);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        assert_eq!(drain(old).unwrap().1, description);
        unsafe {
            release();
        }
    }

    #[test]
    fn operation_completion_resumed_attachment_continues_finalized_sequence() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(2, 32);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let saved = EVENT_SEQUENCE.load(Ordering::Relaxed).saturating_add(17);
        restore_sequence(saved).unwrap();
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"after reattach"), 1) };
        let events = drain(&log).unwrap().0;
        assert_eq!(events[0].sequence, saved + 1);
        restore_sequence(saved - 1).unwrap();
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"other campaign"), 1) };
        assert_eq!(drain(&log).unwrap().0[0].sequence, saved + 2);
    }

    #[cfg(test)]
    fn complete_through(log: &Log, fixture: &Fixture) {
        let lease = publishing(log).unwrap();
        observe_return(lease.0, lane(lease.0), fixture, BASE, INSTANCE, 1);
    }

    #[test]
    fn operation_completion_writers_beyond_the_lane_count_lose_nothing_in_one_generation() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(8, 32);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
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
    }

    #[test]
    fn operation_completion_shared_slot_short_spills_whole_and_sizes_the_next_slot() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(2, 4);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let long = vec![0xfe; 9];
        observe_return(
            &log,
            Target::Shared,
            &Fixture::new(&long),
            BASE,
            INSTANCE,
            1,
        );
        assert!(SHARED_DESCRIPTION_DEMAND.load(Ordering::Relaxed) >= 9);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            (events[0].description_error, events[0].description_required),
            (0, 9)
        );
        assert_eq!(bytes, long);
        let held: Vec<_> = (0..log.shared.len())
            .map(|_| log.shared.try_reserve().unwrap())
            .collect();
        observe_return(
            &log,
            Target::Shared,
            &Fixture::new(b"full"),
            BASE,
            INSTANCE,
            1,
        );
        assert!(log.shared.required_entries() > log.shared.len());
        assert_eq!(log.shared.pending(), log.shared.len() + 1);
        let refused_fixture = Fixture::new(b"x");
        collector_test_support::with_allocations_denied(|| {
            observe_return(&log, Target::Shared, &refused_fixture, BASE, INSTANCE, 1);
        });
        drop(held);
        assert_eq!(drain(&log), Err(SPILL_REFUSED));
        assert!(log.shared.is_drained());
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"after"), 1) };
        assert_eq!(drain(&log).unwrap().1, b"after");
    }

    #[test]
    fn operation_completion_spilled_on_the_hook_thread_never_waits_for_the_consumer() {
        use std::sync::mpsc;
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(1, 8);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let held = Fixture::new(b"held");
        for _ in 0..3 {
            // SAFETY: Test is the sole writer to lane 0 of this log.
            unsafe { complete(&log, &held, 1) };
        }
        assert_eq!((log.shared.len(), log.shared.pending()), (1, 2));
        let (holding, consumer_holds) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        std::thread::scope(|scope| {
            let log = &log;
            scope.spawn(move || {
                let _ = log.shared.published(|record| {
                    if record.order == 3 {
                        holding.send(()).unwrap();
                        released.recv().unwrap();
                    }
                    record.order
                });
            });
            consumer_holds.recv().unwrap();
            let hook = scope.spawn(move || {
                let burst = Fixture::new(b"burst");
                for _ in 0..4 {
                    // SAFETY: Test is the sole writer to lane 0 of this log.
                    unsafe { complete(log, &burst, 1) };
                }
            });
            let finished = hook.join();
            release.send(()).unwrap();
            finished.unwrap();
        });
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events.len(), 7);
        assert!(
            events
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        assert_eq!(bytes, [b"held".repeat(3), b"burst".repeat(4)].concat());
        assert_eq!(log.shared.rejected(), 0);
        assert!(log.shared.is_drained());
    }

    #[test]
    fn operation_completion_lane_growth_seals_the_generation_without_losing_its_interval() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        unsafe {
            release();
        }
        presize(8, 64);
        let old = log().unwrap();
        old.registered.store(1, Ordering::Relaxed);
        // SAFETY: Test holds `serial()` and exclusively owns `old`.
        unsafe { prime(old) };
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(old, &Fixture::new(b"before growth"), 1) };
        let epoch = EPOCH.load(Ordering::Acquire);
        REQUIRED_LANES.fetch_max(old.lanes.len() + 1, Ordering::Relaxed);
        presize(8, 64);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        assert!(std::ptr::eq(log().unwrap(), old));
        let fresh = offered().unwrap();
        assert!(fresh.lanes.len() > old.lanes.len());
        let sealed = collector_test_support::with_allocations_denied(|| seal_offered(old));
        assert!(std::ptr::eq(sealed, old));
        assert!(std::ptr::eq(log().unwrap(), fresh));
        assert!(offered().is_none());
        complete_through(old, &Fixture::new(b"after seal"));
        let (events, bytes) = drain(old).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(bytes, b"before growth");
        let (events, bytes) = drain(fresh).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(bytes, b"after seal");
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        unsafe {
            release();
        }
    }

    #[test]
    fn operation_completion_queued_intervals_assemble_disjointly_and_hold_storage_until_written() {
        use crate::extended::operation_outcomes::{Captured, Ledger};
        use replay_data::{rows::TableId, rows::TableRows};
        use replay_frame::{frame::TableInput, frame::TableView};
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(4, 32);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let lane = &log.lanes[0];
        let sealed = || {
            let mut cursors = vec![LaneCursor::default()];
            // SAFETY: test holds `serial()` as the sole consumer of `log`.
            let handoff =
                unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors)) }
                    .unwrap();
            Captured::sealed(handoff, cursors)
        };
        let shared = |description: &[u8]| {
            observe_return(
                &log,
                Target::Shared,
                &Fixture::new(description),
                BASE,
                INSTANCE,
                1,
            )
        };
        let rows = |tables: &[TableInput]| match &tables[TableId::OperationCompletions.index()] {
            TableInput::Sampled(TableView::Complete(TableRows::OperationCompletions(rows))) => rows
                .iter()
                .map(|row| row.effect_description.clone())
                .collect::<Vec<_>>(),
            _ => panic!("operation completions unavailable"),
        };
        let mut ledger = Ledger::default();
        let mut tables = vec![
            TableInput::Sampled(TableView::Unavailable {
                reason: "test".into(),
            });
            TableId::ALL.len()
        ];
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"first"), 1) };
        shared(b"first shared");
        // SAFETY: test holds `serial()` and assembles intervals in order.
        let first = unsafe { ledger.assemble_owned(&sealed(), &mut tables) }.unwrap();
        assert_eq!(rows(&tables), [b"first".to_vec(), b"first shared".to_vec()]);
        // SAFETY: test holds `serial()` and admits publications in assembly order.
        unsafe { first.admitted() };
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"second"), 1) };
        shared(b"second shared");
        // SAFETY: test holds `serial()` and assembles intervals in order.
        let second = unsafe { ledger.assemble_owned(&sealed(), &mut tables) }.unwrap();
        assert_eq!(
            rows(&tables),
            [b"second".to_vec(), b"second shared".to_vec()]
        );
        // SAFETY: test holds `serial()` and admits publications in assembly order.
        unsafe { second.admitted() };
        // SAFETY: test holds `serial()` and assembles intervals in order.
        let quiet = unsafe { ledger.assemble_owned(&sealed(), &mut tables) }.unwrap();
        assert!(rows(&tables).is_empty());
        // SAFETY: test holds `serial()` and admits publications in assembly order.
        unsafe { quiet.admitted() };
        assert_eq!(lane.assembled.load(Ordering::Acquire), 2);
        assert_eq!(lane.assembled_attempts.load(Ordering::Acquire), 2);
        assert_eq!(log.consumed.load(Ordering::Acquire), 0);
        assert_eq!(lane.consumed.load(Ordering::Acquire), 0);
        assert_eq!(lane.consumed_attempts.load(Ordering::Acquire), 0);
        assert_eq!(lane.ring().consumed.load(Ordering::Acquire), 0);
        assert_eq!(
            log.shared.published(|record| record.order).unwrap().len(),
            2
        );
        assert_eq!(
            lane.required_bytes.load(Ordering::Relaxed),
            b"firstsecond".len()
        );
        // SAFETY: test holds `serial()` and commits publications in assembly order.
        unsafe { first.commit() };
        assert_eq!(log.consumed.load(Ordering::Acquire), 2);
        assert_eq!(lane.consumed.load(Ordering::Acquire), 1);
        assert_eq!(lane.consumed_attempts.load(Ordering::Acquire), 1);
        assert_eq!(lane.consumed_byte_demand.load(Ordering::Acquire), 5);
        assert_eq!(lane.ring().consumed.load(Ordering::Acquire), 5);
        assert_eq!(
            log.shared.published(|record| record.order).unwrap().len(),
            1
        );
        assert_eq!(log.assembled.load(Ordering::Acquire), 4);
        // SAFETY: test holds `serial()` and commits publications in assembly order.
        unsafe {
            second.commit();
            quiet.commit();
        }
        assert_eq!(log.consumed.load(Ordering::Acquire), 4);
        assert_eq!(lane.ring().consumed.load(Ordering::Acquire), 11);
        assert!(
            log.shared
                .published(|record| record.order)
                .unwrap()
                .is_empty()
        );
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"third"), 1) };
        assert_eq!(drain(&log).unwrap().1, b"third");
    }

    #[test]
    fn operation_completion_lost_queue_assembles_again_from_the_written_interval() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(4, 32);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"written"), 1) };
        assert_eq!(drain(&log).unwrap().1, b"written");
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"lost"), 1) };
        observe_return(
            &log,
            Target::Shared,
            &Fixture::new(b"lost shared"),
            BASE,
            INSTANCE,
            1,
        );
        let mut first = vec![LaneCursor::default()];
        // SAFETY: test holds `serial()` as the sole consumer of `log`.
        let queued =
            unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut first)) }
                .unwrap();
        let mut events = [Event::default(); 2];
        let mut bytes = [0; 32];
        assert_eq!(
            queued.prepare(&first, &mut events, &mut bytes).unwrap(),
            Drained {
                copied: 2,
                bytes: 15
            }
        );
        queued.admitted(&first);
        queued.admitted(&first);
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"later"), 1) };
        let mut second = vec![LaneCursor::default()];
        // SAFETY: test holds `serial()` as the sole consumer of `log`.
        let later =
            unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut second)) }
                .unwrap();
        assert_eq!(later.pending(), 1);
        later.admitted(&second);
        queued.admitted(&first);
        assert_eq!(log.assembled.load(Ordering::Acquire), 4);
        log.reset_admitted();
        let lane = &log.lanes[0];
        assert_eq!(log.assembled.load(Ordering::Acquire), 1);
        assert_eq!(
            log.assembled_epoch.load(Ordering::Acquire),
            log.consumed_epoch.load(Ordering::Acquire)
        );
        assert_eq!(lane.assembled.load(Ordering::Acquire), 1);
        assert_eq!(lane.assembled_attempts.load(Ordering::Acquire), 1);
        let (events, bytes) = drain(&log).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(bytes, b"lostlost sharedlater");
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"before"), 1) };
        let mut cursors = vec![LaneCursor::default()];
        // SAFETY: test holds `serial()` as the sole consumer of `log`.
        unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors)) }
            .unwrap()
            .admitted(&cursors);
        unsafe { invalidate() };
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"after"), 1) };
        // SAFETY: test holds `serial()` as the sole consumer of `log`.
        unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors)) }
            .unwrap()
            .admitted(&cursors);
        log.reset_admitted();
        assert_eq!(
            drain(&log),
            Err("operation completions were not followed in the game without a break")
        );
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &Fixture::new(b"recovered"), 1) };
        assert_eq!(drain(&log).unwrap().1, b"recovered");
    }

    #[test]
    fn operation_completion_queued_backlog_raises_entry_demand_past_one_interval() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        let log = new_log(2, 32);
        // SAFETY: Test holds `serial()` and exclusively owns `log`.
        unsafe { prime(&log) };
        let fixture = Fixture::new(b"held");
        for _ in 0..2 {
            // SAFETY: Test is the sole writer to lane 0 of this log.
            unsafe { complete(&log, &fixture, 1) };
            let mut cursors = vec![LaneCursor::default()];
            // SAFETY: test holds `serial()` as the sole consumer of `log`.
            unsafe { handoff_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors)) }
                .unwrap()
                .admitted(&cursors);
        }
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(&log, &fixture, 1) };
        assert_eq!(log.lanes[0].required_entries.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn operation_completion_lost_queue_of_a_replaced_generation_reports_the_loss() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        unsafe {
            release();
        }
        presize(8, 64);
        let old = log().unwrap();
        old.registered.store(1, Ordering::Relaxed);
        // SAFETY: Test holds `serial()` and exclusively owns `old`.
        unsafe { prime(old) };
        // SAFETY: Test is the sole writer to lane 0 of this log.
        unsafe { complete(old, &Fixture::new(b"held by the old generation"), 1) };
        REQUIRED_LANES.fetch_max(old.lanes.len() + 1, Ordering::Relaxed);
        presize(8, 64);
        let sealed = seal_offered(old);
        assert!(!std::ptr::eq(log().unwrap(), old));
        let mut cursors = vec![LaneCursor::default()];
        // SAFETY: test holds `serial()` as the sole consumer of `sealed`.
        unsafe { handoff_buffer(sealed, engine_native::walk::Buffer::Growing(&mut cursors)) }
            .unwrap()
            .admitted(&cursors);
        let epoch = EPOCH.load(Ordering::Acquire);
        reset_admitted();
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch + 1);
        assert_eq!(old.assembled.load(Ordering::Acquire), 0);
        assert_eq!(old.lanes[0].consumed.load(Ordering::Acquire), 0);
        reset_admitted();
        unsafe {
            release();
        }
    }

    #[test]
    fn concurrent_smaller_byte_ring_growth_does_not_replace_larger_ring() {
        let log = new_log(4, 16);
        let lane = &log.lanes[0];
        let initial = lane.ring().bytes.len();
        lane.grow_bytes_for_with(initial * 2, || {
            lane.grow_bytes_for(initial * 16).unwrap();
        })
        .unwrap();
        assert_eq!(lane.ring().bytes.len(), initial * 16);
    }

    #[test]
    fn concurrent_smaller_presize_does_not_replace_larger_offer() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        // SAFETY: Unit test holds `serial()` with no live native hooks or workers.
        unsafe { release() };
        presize(8, 64);
        let mut raced = false;
        presize_with(32, 64, || {
            if !raced {
                raced = true;
                presize(256, 64);
            }
        });
        assert!(raced);
        assert_eq!(offered().unwrap().entries, 256);
        // SAFETY: Unit test holds `serial()` with no live native hooks or workers.
        unsafe { release() };
    }

    #[test]
    fn concurrent_mixed_entry_and_byte_presize_preserves_both_dimensions() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        // SAFETY: Unit test holds `serial()` with no live native hooks or workers.
        unsafe { release() };
        presize(8, 64);
        let mut raced_entry = false;
        presize_with(64, 0, || {
            if !raced_entry {
                raced_entry = true;
                presize(0, 4096);
            }
        });
        assert!(raced_entry);
        let offer = offered().unwrap();
        assert_eq!(offer.entries, 64);
        assert!(offer.lanes.iter().all(|lane| lane.ring().bytes.len() >= 4096));
        // SAFETY: Unit test holds `serial()` with no live native hooks or workers.
        unsafe { release() };
        presize(8, 64);
        let mut raced_bytes = false;
        presize_with_hooks(
            0,
            4096,
            || {
                if !raced_bytes {
                    raced_bytes = true;
                    presize(64, 0);
                }
            },
            || {},
        );
        assert!(raced_bytes);
        let offer = offered().unwrap();
        assert_eq!(offer.entries, 64);
        assert!(offer.lanes.iter().all(|lane| lane.ring().bytes.len() >= 4096));
        // SAFETY: Unit test holds `serial()` with no live native hooks or workers.
        unsafe { release() };
    }

    #[test]
    fn promotion_during_presize_revalidation_does_not_install_smaller_offer() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        // SAFETY: Unit test holds `serial()` with no live native hooks or workers.
        unsafe { release() };
        presize(8, 64);
        let old = log().unwrap();
        let mut promoted = false;
        presize_with(32, 64, || {
            if !promoted {
                promoted = true;
                presize(256, 64);
                let _ = seal_offered(old);
            }
        });
        assert!(promoted);
        assert_eq!(log().unwrap().entries, 256);
        assert!(offered().is_none());
        let active = log().unwrap();
        let mut promoted_mixed = false;
        presize_with(512, 4096, || {
            if !promoted_mixed {
                promoted_mixed = true;
                presize(1024, 64);
                let _ = seal_offered(active);
            }
        });
        assert!(promoted_mixed);
        let current = log().unwrap();
        assert_eq!(current.entries, 1024);
        assert!(current.lanes.iter().all(|lane| lane.ring().bytes.len() >= 4096));
        assert!(offered().is_none());
        // SAFETY: Unit test holds `serial()` with no live native hooks or workers.
        unsafe { release() };
    }

    #[test]
    fn seal_offered_defers_while_logs_locked_and_promotes_once_released() {
        // SAFETY: Unit test runs with no live native hooks enabled.
        let _serial = unsafe { serial() };
        // SAFETY: Unit test holds `serial()` with no live native hooks or workers.
        unsafe { release() };
        presize(8, 64);
        let old = log().unwrap();
        presize(64, 64);
        let fresh = offered().unwrap();
        let guard = LOGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let deferred = seal_offered(old);
        assert!(std::ptr::eq(deferred, old));
        assert!(std::ptr::eq(log().unwrap(), old));
        assert!(std::ptr::eq(offered().unwrap(), fresh));
        drop(guard);
        let sealed = seal_offered(old);
        assert!(std::ptr::eq(sealed, old));
        assert!(std::ptr::eq(log().unwrap(), fresh));
        assert!(offered().is_none());
        // SAFETY: Unit test holds `serial()` with no live native hooks or workers.
        unsafe { release() };
    }
}
