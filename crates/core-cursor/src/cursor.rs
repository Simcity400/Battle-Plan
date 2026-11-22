use crate::codec::{Reader, Writer};
use crate::publish::{ErrorMailbox, Published};
use crate::*;
use std::any::Any;
use std::io;
use std::mem::size_of;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::thread::{self, JoinHandle, Thread};
use std::time::Instant;

#[derive(Clone)]
pub struct CursorOptions {
    pub memory: Memory,
    pub working_set: usize,
    pub speed: f64,
    pub refresh_hz: f64,
}
impl Default for CursorOptions {
    fn default() -> Self {
        Self {
            memory: Memory::global(),
            working_set: PLAYBACK_BYTES,
            speed: 0.0,
            refresh_hz: 0.0,
        }
    }
}

pub struct Values<S: ValueStore> {
    state: S,
    slots: Arc<Slots>,
    scope: Hours,
    hour: Hour,
    accepted: u64,
    coverage: Buffer<Coverage>,
}
impl<S: ValueStore> Values<S> {
    pub fn hour(&self) -> Hour {
        self.hour
    }
    pub fn get(&self, address: &Address) -> Lookup<'_> {
        match self.state.get(address) {
            Lookup::Record(record) => Lookup::Record(record.evaluated(self.accepted)),
            other => other,
        }
    }
    pub fn at(&self, slot: usize) -> Option<(&Address, RecordRef<'_>)> {
        let slot = self.slots.internal(slot)?;
        Some((
            self.state.address(slot),
            self.state.record(slot, false).evaluated(self.accepted),
        ))
    }
    pub fn iter(&self) -> impl Iterator<Item = (&Address, RecordRef<'_>)> {
        self.slots().map(|(_, address, record)| (address, record))
    }
    pub fn slots(&self) -> impl Iterator<Item = (usize, &Address, RecordRef<'_>)> {
        (0..self.slots.selection.len()).map(|slot| {
            (
                self.slots.public(slot),
                self.state.address(slot),
                self.state.record(slot, false).evaluated(self.accepted),
            )
        })
    }
    pub fn store(&self) -> &S {
        &self.state
    }
    pub fn coverage(&self) -> &[Coverage] {
        &self.coverage
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeltaStatus {
    Applied,
    NotResident,
    Preparing,
    CatchingUp,
}
pub struct Delta {
    pub from: Hour,
    pub to: Hour,
    pub applied: bool,
    pub status: DeltaStatus,
    pub full_refresh: bool,
    pub scope_switch: bool,
    changes: Buffer<usize>,
    coverage: Buffer<Coverage>,
}

struct Slots {
    selection: Arc<Selection>,
    forward: Buffer<usize>,
    inverse: Buffer<usize>,
    sparse: bool,
    _memory: Reservation,
}
impl Slots {
    fn new(
        selection: Arc<Selection>,
        previous: Option<&Self>,
        memory: &Memory,
    ) -> io::Result<Arc<Self>> {
        let metadata = memory.reserve(Owner::State, size_of::<Self>() + 2 * size_of::<usize>())?;
        let mut forward = Buffer::new(
            memory,
            Owner::State,
            previous.map_or(0, |_| selection.len()),
        )?;
        let mut occupied = Buffer::new(
            &memory.soft(),
            Owner::Transient,
            previous.map_or(0, |_| selection.len()),
        )?;
        if let Some(previous) = previous {
            for _ in 0..selection.len() {
                forward.push(usize::MAX);
            }
            let mut old = 0;
            for slot in 0..selection.len() {
                let id = selection.id(slot);
                while old < previous.selection.len() && previous.selection.id(old) < id {
                    old += 1;
                }
                if old < previous.selection.len() && previous.selection.id(old) == id {
                    let public = previous.public(old);
                    forward[slot] = public;
                    occupied.push(public);
                }
            }
            occupied.as_mut_slice().sort_unstable();
            let mut retained = 0;
            let mut free = 0;
            for slot in 0..selection.len() {
                if forward[slot] == usize::MAX {
                    while occupied.get(retained) == Some(&free) {
                        retained += 1;
                        free += 1;
                    }
                    forward[slot] = free;
                    free += 1;
                }
            }
        }
        let extent = forward.iter().copied().max().map_or(0, |slot| slot + 1);
        let sparse = extent > selection.len().saturating_mul(2);
        let capacity = if sparse {
            selection
                .len()
                .checked_mul(2)
                .and_then(usize::checked_next_power_of_two)
                .ok_or_else(oom)?
        } else {
            extent
        };
        let mut inverse = Buffer::new(memory, Owner::State, capacity)?;
        for _ in 0..capacity {
            inverse.push(usize::MAX);
        }
        for (slot, public) in forward.iter().copied().enumerate() {
            let mut index = if sparse {
                Self::hash(public) & (capacity - 1)
            } else {
                public
            };
            while inverse[index] != usize::MAX {
                index = (index + 1) & (capacity - 1);
            }
            inverse[index] = slot;
        }
        Ok(Arc::new(Self {
            selection,
            forward,
            inverse,
            sparse,
            _memory: metadata,
        }))
    }
    fn hash(slot: usize) -> usize {
        let hash = slot.wrapping_mul(0x9e3779b9);
        hash ^ (hash >> (usize::BITS / 2))
    }
    fn public(&self, slot: usize) -> usize {
        if self.forward.is_empty() {
            slot
        } else {
            self.forward[slot]
        }
    }
    fn internal(&self, slot: usize) -> Option<usize> {
        if self.inverse.is_empty() {
            (self.forward.is_empty() && slot < self.selection.len()).then_some(slot)
        } else if self.sparse {
            let mut index = Self::hash(slot) & (self.inverse.len() - 1);
            loop {
                let internal = self.inverse[index];
                if internal == usize::MAX {
                    return None;
                }
                if self.forward[internal] == slot {
                    return Some(internal);
                }
                index = (index + 1) & (self.inverse.len() - 1);
            }
        } else {
            self.inverse
                .get(slot)
                .copied()
                .filter(|slot| *slot != usize::MAX)
        }
    }
}
impl Delta {
    pub fn changes(&self) -> &[usize] {
        &self.changes
    }
    pub fn coverage(&self) -> &[Coverage] {
        &self.coverage
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CursorStats {
    pub misses: usize,
    pub not_resident: usize,
    pub catch_up_drops: usize,
    pub capacity_misses: usize,
    pub catch_ups: usize,
    pub pages_read: usize,
    pub window_bytes: usize,
    pub directory_bound_bytes: usize,
    pub window_peak: usize,
    pub inverse_bytes: usize,
    pub window_hours: u64,
    pub window_wall_seconds: f64,
    pub shortened_windows: usize,
    pub page_read_wall_ns: u64,
    pub page_read_cycles: u64,
    pub reverse_build_wall_ns: u64,
    pub reverse_build_cycles: u64,
    pub restore_cycles: u64,
    pub decoded_bytes: u64,
    pub worker_cycles: u64,
    pub planning_cycles: u64,
    pub publication_cycles: u64,
    pub preparation_cycles: u64,
    pub publish_retries: usize,
    pub scope_changes: usize,
}
#[derive(Default)]
struct Counters {
    pages: AtomicUsize,
    peak: AtomicUsize,
    shortened: AtomicUsize,
    read_wall: AtomicU64,
    read_cycles: AtomicU64,
    reverse_wall: AtomicU64,
    reverse_cycles: AtomicU64,
    restore_cycles: AtomicU64,
    bytes: AtomicU64,
    worker: AtomicU64,
    planning: AtomicU64,
    publication: AtomicU64,
    preparation: AtomicU64,
    retries: AtomicUsize,
    scopes: AtomicUsize,
}

struct InverseRun {
    hour: Hour,
    start: usize,
}
struct InversePage {
    bytes: Buffer<u8>,
    runs: Buffer<InverseRun>,
    coverage: Buffer<Coverage>,
    _metadata: Reservation,
}
impl InversePage {
    fn decode(
        &self,
        hours: Hours,
        selection: &Selection,
        state: &mut impl ValueStore,
        changed: &mut Buffer<usize>,
    ) -> io::Result<()> {
        let first = self.runs.partition_point(|run| run.hour < hours.first);
        let last = self.runs.partition_point(|run| run.hour <= hours.last);
        for index in (first..last).rev() {
            let run = &self.runs[index];
            let end = self
                .runs
                .get(index + 1)
                .map_or(self.bytes.len(), |run| run.start);
            let mut reader = Reader::new(&self.bytes[run.start..end]);
            while reader.offset < end - run.start {
                let id = usize::try_from(reader.uint()?).map_err(io::Error::other)?;
                let record = reader.record(run.hour)?;
                if let Some(slot) = selection.slot(id) {
                    if state.stage(slot, record)? && !changed.push(slot) {
                        return Err(oom());
                    }
                }
            }
        }
        Ok(())
    }
}
struct ResidentPage<D: PageDecoder> {
    page: Page<D>,
    charged_bytes: usize,
    _metadata: Reservation,
}
impl<D: PageDecoder> std::ops::Deref for ResidentPage<D> {
    type Target = Page<D>;
    fn deref(&self) -> &Self::Target {
        &self.page
    }
}
enum Payload<D: PageDecoder> {
    Forward(Arc<ResidentPage<D>>),
    Reverse(Arc<InversePage>),
}
struct WindowPage<D: PageDecoder> {
    info: PageInfo,
    payload: Payload<D>,
    coverage_runs: usize,
    _metadata: Reservation,
}
impl<D: PageDecoder> WindowPage<D> {
    fn coverage(&self) -> &[Coverage] {
        match &self.payload {
            Payload::Forward(page) => &page.coverage,
            Payload::Reverse(ring) => &ring.coverage,
        }
    }
}
impl<D: PageDecoder> crate::window::Entry for WindowPage<D> {
    fn info(&self) -> PageInfo {
        self.info
    }
    fn bytes(&self) -> usize {
        match &self.payload {
            Payload::Forward(page) => page.charged_bytes,
            Payload::Reverse(ring) => {
                ring.bytes.bytes()
                    + ring.runs.bytes()
                    + ring.coverage.bytes()
                    + ring._metadata.bytes()
            }
        }
    }
    fn coverage(&self) -> &[Coverage] {
        WindowPage::coverage(self)
    }
    fn coverage_runs(&self) -> usize {
        self.coverage_runs
    }
}
struct Snapshot<D: PageDecoder> {
    pages: crate::window::Tree<WindowPage<D>>,
    selection: Arc<Selection>,
    served: Hours,
    reverse: bool,
    _metadata: Reservation,
}
impl<D: PageDecoder> Snapshot<D> {
    fn contains(&self, hours: Hours, reverse: bool) -> bool {
        self.reverse == reverse
            && self.served.first <= hours.first
            && self.served.last >= hours.last
    }
    fn bounds(&self, reverse: bool, hour: Hour) -> Option<Hours> {
        (self.reverse == reverse && self.served.contains(hour)).then_some(self.served)
    }
    fn bytes(&self) -> usize {
        self.pages.bytes()
    }
    fn coverage(
        &self,
        span: Hours,
        mut visit: impl FnMut(Coverage) -> io::Result<()>,
    ) -> io::Result<usize> {
        let mut next = i64::from(span.first);
        let mut count = 0;
        let mut pending: Option<Coverage> = None;
        for page in self.pages.iter(span) {
            for item in page.coverage() {
                if next > i64::from(span.last) {
                    break;
                }
                let Some(item) = crate::reference::clip(
                    item,
                    Hours {
                        first: next as Hour,
                        last: span.last,
                    },
                ) else {
                    continue;
                };
                if i64::from(item.hours.first) != next {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "source omitted coverage inside its window",
                    ));
                }
                next = i64::from(item.hours.last) + 1;
                if let Some(previous) = &mut pending {
                    if crate::window::can_merge(previous, &item) {
                        previous.hours.last = item.hours.last;
                        continue;
                    }
                    visit(pending.take().unwrap())?;
                    count += 1;
                }
                pending = Some(item);
            }
        }
        if let Some(item) = pending {
            visit(item)?;
            count += 1;
        }
        if next <= i64::from(span.last) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "source omitted coverage inside its window",
            ));
        }
        Ok(count)
    }
}

struct Ready<S: ValueStore> {
    state: S,
    slots: Arc<Slots>,
    previous: Arc<Slots>,
    scope: Hours,
    previous_scope: Hours,
    reverse: bool,
    adopted: bool,
    hour: Hour,
    accepted: u64,
    coverage: Buffer<Coverage>,
    rebase: bool,
    _metadata: Reservation,
}
struct PreparedScope<'a, S: LogSource> {
    request: &'a Request<S>,
    previous: Arc<Slots>,
    previous_scope: Hours,
    ready: Option<Box<Ready<S::State>>>,
}
impl<S: LogSource> PreparedScope<'_, S> {
    fn handoff(&mut self, served: Hours, complete: bool) {
        let usable = self.ready.as_ref().is_some_and(|ready| {
            let prime = window(
                ready.hour,
                ready.hour,
                ready.reverse,
                f64::from_bits(self.request.speed.load(Ordering::Relaxed)),
                f64::from_bits(self.request.refresh.load(Ordering::Relaxed)),
                true,
            );
            (served.first <= prime.first && served.last >= prime.last)
                || (complete && served.contains(ready.hour))
        });
        if usable {
            let ready = self.ready.take().unwrap();
            self.request
                .restore_hour
                .store(ready.hour, Ordering::Relaxed);
            self.request
                .restoring
                .store(!ready.rebase, Ordering::Release);
            self.request
                .ready_rates
                .store(ready.state.active_rates(), Ordering::Release);
            self.request
                .ready
                .store(Box::into_raw(ready), Ordering::Release);
        }
    }
}
impl<S: LogSource> Drop for PreparedScope<'_, S> {
    fn drop(&mut self) {
        if self.ready.is_some() {
            *self.request.selection.lock().unwrap() = self.previous.selection.clone();
            *self.request.slots.lock().unwrap() = self.previous.clone();
            *self.request.selection_scope.lock().unwrap() = self.previous_scope;
        }
    }
}
struct Request<S: LogSource> {
    keys: KeyFilter,
    selection: Mutex<Arc<Selection>>,
    slots: Mutex<Arc<Slots>>,
    selection_scope: Mutex<Hours>,
    memory: Memory,
    published: Published<Snapshot<S::Decoder>>,
    filling: Mutex<()>,
    hour: AtomicI32,
    target: AtomicI32,
    reverse: AtomicBool,
    speed: AtomicU64,
    refresh: AtomicU64,
    sequence: AtomicU64,
    catch_up: AtomicBool,
    restoring: AtomicBool,
    restore_hour: AtomicI32,
    stopped: AtomicBool,
    ready: AtomicPtr<Ready<S::State>>,
    retired: AtomicPtr<Ready<S::State>>,
    planned_first: AtomicI32,
    planned_last: AtomicI32,
    planned_reverse: AtomicBool,
    workspace: AtomicPtr<Workspace>,
    workspace_retired: AtomicPtr<Workspace>,
    coverage_capacity: AtomicUsize,
    change_capacity: AtomicUsize,
    prepared_coverage: AtomicUsize,
    prepared_changes: AtomicUsize,
    active_rates: AtomicUsize,
    ready_rates: AtomicUsize,
    completed: Mutex<u64>,
    completion: Condvar,
    error: ErrorMailbox,
    counters: Counters,
    _metadata: Reservation,
}
impl<S: LogSource> Drop for Request<S> {
    fn drop(&mut self) {
        for pointer in [*self.ready.get_mut(), *self.retired.get_mut()] {
            if !pointer.is_null() {
                unsafe {
                    drop(Box::from_raw(pointer));
                }
            }
        }
        for pointer in [*self.workspace.get_mut(), *self.workspace_retired.get_mut()] {
            if !pointer.is_null() {
                unsafe {
                    drop(Box::from_raw(pointer));
                }
            }
        }
    }
}
struct Job<S: LogSource> {
    request: Weak<Request<S>>,
    _metadata: Reservation,
}
fn prune_jobs<S: LogSource>(jobs: &mut Buffer<Job<S>>, memory: &Memory) {
    jobs.retain(|job| {
        job.request
            .upgrade()
            .is_some_and(|request| !request.stopped.load(Ordering::Acquire))
    });
    if jobs.capacity() > jobs.len().saturating_mul(2) {
        let _ = jobs.compact(&memory.soft(), Owner::Metadata);
    }
}
struct CacheEntry<D: PageDecoder> {
    info: PageInfo,
    page: Weak<ResidentPage<D>>,
    _metadata: Reservation,
}
impl<D: PageDecoder> crate::window::Entry for CacheEntry<D> {
    fn info(&self) -> PageInfo {
        self.info
    }
    fn bytes(&self) -> usize {
        0
    }
}
struct Cache<D: PageDecoder> {
    pages: crate::window::Tree<CacheEntry<D>>,
    added: usize,
    clean_after: usize,
}
struct Pool<S: LogSource> {
    log: Arc<S>,
    memory: Memory,
    jobs: Mutex<Buffer<Job<S>>>,
    cache: Mutex<Cache<S::Decoder>>,
    worker: OnceLock<Thread>,
    handle: Mutex<Option<JoinHandle<()>>>,
    stop: AtomicBool,
    _metadata: Reservation,
}
struct RegistryEntry {
    source: usize,
    memory: Memory,
    pool: Weak<dyn Any + Send + Sync>,
}
static POOLS: Mutex<Option<Buffer<RegistryEntry>>> = Mutex::new(None);

pub struct Cursor<S: LogSource> {
    log: Arc<S>,
    values: Values<S::State>,
    delta: Delta,
    request: Arc<Request<S>>,
    pool: Arc<Pool<S>>,
    selection: Arc<Selection>,
    stats: CursorStats,
    _registration: crate::budget::Registration,
    _metadata: Reservation,
}

impl<S: LogSource> Cursor<S> {
    pub fn open(log: &Arc<S>, keys: KeyFilter, hour: Hour) -> io::Result<Self> {
        Self::open_with(log, keys, hour, CursorOptions::default())
    }
    pub fn open_with(
        log: &Arc<S>,
        keys: KeyFilter,
        hour: Hour,
        options: CursorOptions,
    ) -> io::Result<Self> {
        validate_rate(options.speed, options.refresh_hz)?;
        if let KeyFilter::Keys(keys) = &keys {
            if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unordered filter",
                ));
            }
        }
        let memory = Memory::child(
            &options.memory,
            options.working_set.min(options.memory.limit()),
        )?;
        let metadata = memory.reserve(Owner::Metadata, size_of::<Self>())?;
        let scope = window(
            hour,
            hour,
            options.speed < 0.0,
            options.speed,
            options.refresh_hz,
            false,
        );
        let state = log.version_at(&keys, hour, slot_scope(scope, options.speed < 0.0), &memory)?;
        let selection = state.selection();
        let slots = Slots::new(selection.clone(), None, &memory)?;
        let (coverage, accepted) = coverage_at(&**log, hour, &memory)?;
        let changes = Buffer::new(&memory, Owner::Delta, 0)?;
        let request_metadata = memory.reserve(
            Owner::Metadata,
            size_of::<Request<S>>() + 2 * size_of::<usize>(),
        )?;
        let request = Arc::new(Request {
            keys,
            selection: Mutex::new(selection.clone()),
            slots: Mutex::new(slots.clone()),
            selection_scope: Mutex::new(slot_scope(scope, options.speed < 0.0)),
            memory: memory.clone(),
            published: Published::new(),
            filling: Mutex::new(()),
            hour: AtomicI32::new(hour),
            target: AtomicI32::new(hour),
            reverse: AtomicBool::new(options.speed < 0.0),
            speed: AtomicU64::new(options.speed.to_bits()),
            refresh: AtomicU64::new(options.refresh_hz.to_bits()),
            sequence: AtomicU64::new(2),
            catch_up: AtomicBool::new(false),
            restoring: AtomicBool::new(false),
            restore_hour: AtomicI32::new(hour),
            stopped: AtomicBool::new(false),
            ready: AtomicPtr::new(std::ptr::null_mut()),
            retired: AtomicPtr::new(std::ptr::null_mut()),
            planned_first: AtomicI32::new(scope.first),
            planned_last: AtomicI32::new(scope.last),
            planned_reverse: AtomicBool::new(options.speed < 0.0),
            workspace: AtomicPtr::new(std::ptr::null_mut()),
            workspace_retired: AtomicPtr::new(std::ptr::null_mut()),
            coverage_capacity: AtomicUsize::new(0),
            change_capacity: AtomicUsize::new(0),
            prepared_coverage: AtomicUsize::new(0),
            prepared_changes: AtomicUsize::new(0),
            active_rates: AtomicUsize::new(state.active_rates()),
            ready_rates: AtomicUsize::new(0),
            completed: Mutex::new(0),
            completion: Condvar::new(),
            error: ErrorMailbox::new(),
            counters: Counters::default(),
            _metadata: request_metadata,
        });
        let pool = shared_pool(log, &options.memory)?;
        {
            let mut jobs = pool.jobs.lock().unwrap();
            prune_jobs(&mut jobs, &options.memory);
            let next = jobs.len() + 1;
            jobs.grow(&options.memory, Owner::Metadata, next)?;
            jobs.push(Job {
                request: Arc::downgrade(&request),
                _metadata: options
                    .memory
                    .reserve(Owner::Metadata, size_of::<Job<S>>())?,
            });
        }
        let registration = crate::budget::register(pool.clone())?;
        wake(&pool);
        Ok(Self {
            log: log.clone(),
            values: Values {
                state,
                slots,
                scope: slot_scope(scope, options.speed < 0.0),
                hour,
                accepted,
                coverage,
            },
            delta: Delta {
                from: hour,
                to: hour,
                applied: true,
                status: DeltaStatus::Applied,
                full_refresh: false,
                scope_switch: false,
                changes,
                coverage: Buffer::new(&memory, Owner::Delta, 0)?,
            },
            request,
            pool,
            selection,
            stats: CursorStats::default(),
            _registration: registration,
            _metadata: metadata,
        })
    }
    pub fn values(&self) -> &Values<S::State> {
        &self.values
    }
    pub fn working_memory(&self) -> &Memory {
        &self.request.memory
    }
    pub fn playback(&mut self, speed: f64, refresh_hz: f64) -> io::Result<()> {
        validate_rate(speed, refresh_hz)?;
        set_plan(
            &self.request,
            window(
                self.values.hour,
                self.values.hour,
                speed < 0.0,
                speed,
                refresh_hz,
                false,
            ),
            speed < 0.0,
        );
        self.post(self.values.hour, speed < 0.0, Some((speed, refresh_hz)));
        Ok(())
    }
    pub fn wait_read_ahead(&self) -> io::Result<()> {
        let target = self.request.sequence.load(Ordering::Acquire);
        let mut completed = self.request.completed.lock().unwrap();
        while *completed < target && !self.request.stopped.load(Ordering::Acquire) {
            completed = self.request.completion.wait(completed).unwrap();
        }
        if let Some(error) = self.request.error.take() {
            return Err(error);
        }
        Ok(())
    }
    pub fn stats(&self) -> CursorStats {
        let mut stats = self.stats;
        let counters = &self.request.counters;
        stats.pages_read = counters.pages.load(Ordering::Relaxed);
        stats.window_peak = counters.peak.load(Ordering::Relaxed);
        stats.shortened_windows = counters.shortened.load(Ordering::Relaxed);
        stats.page_read_wall_ns = counters.read_wall.load(Ordering::Relaxed);
        stats.page_read_cycles = counters.read_cycles.load(Ordering::Relaxed);
        stats.reverse_build_wall_ns = counters.reverse_wall.load(Ordering::Relaxed);
        stats.reverse_build_cycles = counters.reverse_cycles.load(Ordering::Relaxed);
        stats.restore_cycles = counters.restore_cycles.load(Ordering::Relaxed);
        stats.decoded_bytes = counters.bytes.load(Ordering::Relaxed);
        stats.worker_cycles = counters.worker.load(Ordering::Relaxed);
        stats.planning_cycles = counters.planning.load(Ordering::Relaxed);
        stats.publication_cycles = counters.publication.load(Ordering::Relaxed);
        stats.preparation_cycles = counters.preparation.load(Ordering::Relaxed);
        stats.publish_retries = counters.retries.load(Ordering::Relaxed);
        stats.scope_changes = counters.scopes.load(Ordering::Relaxed);
        let read = self.request.published.read();
        if let Some(snapshot) = read.get() {
            stats.window_bytes = snapshot.bytes();
            stats.directory_bound_bytes = snapshot.pages.directory_bytes();
            stats.inverse_bytes = if snapshot.reverse {
                snapshot.bytes()
            } else {
                0
            };
            let reverse = self.request.reverse.load(Ordering::Relaxed);
            if let Some(bounds) = snapshot.bounds(reverse, self.values.hour) {
                stats.window_hours = if reverse {
                    (i64::from(self.values.hour) - i64::from(bounds.first)) as u64
                } else {
                    (i64::from(bounds.last) - i64::from(self.values.hour)) as u64
                };
            }
            let speed = f64::from_bits(self.request.speed.load(Ordering::Relaxed)).abs();
            stats.window_wall_seconds = if speed > 0.0 {
                stats.window_hours as f64 / speed
            } else {
                0.0
            };
        }
        stats
    }
    pub fn advance(&mut self, hour: Hour) -> io::Result<&Delta> {
        if hour < self.values.hour {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "advance precedes cursor",
            ));
        }
        self.move_to(hour, false)
    }
    pub fn rewind(&mut self, hour: Hour) -> io::Result<&Delta> {
        if hour > self.values.hour {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "rewind follows cursor",
            ));
        }
        self.move_to(hour, true)
    }
    fn post(&self, target: Hour, reverse: bool, rate: Option<(f64, f64)>) {
        self.request.sequence.fetch_add(1, Ordering::AcqRel);
        self.request.hour.store(self.values.hour, Ordering::Relaxed);
        self.request.target.store(target, Ordering::Relaxed);
        self.request.reverse.store(reverse, Ordering::Relaxed);
        if let Some((speed, refresh)) = rate {
            self.request.speed.store(speed.to_bits(), Ordering::Relaxed);
            self.request
                .refresh
                .store(refresh.to_bits(), Ordering::Relaxed);
        }
        self.request.sequence.fetch_add(1, Ordering::Release);
        wake(&self.pool);
    }
    fn accept_ready(&mut self, target: Hour, reverse: bool) -> *mut Ready<S::State> {
        if !self.request.retired.load(Ordering::Acquire).is_null() {
            return std::ptr::null_mut();
        }
        let pointer = self.request.ready.load(Ordering::Acquire);
        if pointer.is_null() {
            return std::ptr::null_mut();
        }
        let ready = unsafe { &mut *pointer };
        let crosses = if reverse {
            ready.hour >= target && ready.hour <= self.values.hour
        } else {
            ready.hour <= target && ready.hour >= self.values.hour
        };
        let ahead = if reverse {
            ready.hour < target
        } else {
            ready.hour > target
        };
        if ready.reverse == reverse && ahead {
            return std::ptr::null_mut();
        }
        if ready.reverse == reverse && crosses {
            let from = if ready.rebase {
                self.values.hour
            } else {
                ready.hour
            };
            let span = Hours {
                first: from.min(target),
                last: from.max(target),
            };
            let read = self.request.published.read();
            let usable = read.get().is_some_and(|snapshot| {
                snapshot.contains(span, reverse)
                    && ready.scope.first <= span.first
                    && ready.scope.last >= span.last
                    && self.delta.changes.capacity()
                        >= frame_capacity(
                            snapshot,
                            span,
                            ready.slots.selection.len(),
                            ready.state.active_rates(),
                        )
                    && snapshot
                        .coverage(span, |_| Ok(()))
                        .is_ok_and(|n| n <= self.delta.coverage.capacity())
            });
            if !usable {
                if ready.scope.first > span.first || ready.scope.last < span.last {
                    self.request
                        .ready
                        .store(std::ptr::null_mut(), Ordering::Release);
                    self.request.catch_up.store(true, Ordering::Release);
                    return pointer;
                }
                return std::ptr::null_mut();
            }
            std::mem::swap(&mut self.values.state, &mut ready.state);
            std::mem::swap(&mut self.values.slots, &mut ready.slots);
            self.values.scope = ready.scope;
            if !ready.rebase {
                std::mem::swap(&mut self.values.coverage, &mut ready.coverage);
                self.values.hour = ready.hour;
                self.values.accepted = ready.accepted;
                self.stats.catch_ups += 1;
            }
            self.delta.scope_switch = ready.rebase;
            self.selection = self.values.state.selection();
            self.request
                .active_rates
                .store(self.values.state.active_rates(), Ordering::Release);
            ready.adopted = true;
        }
        self.request
            .ready
            .store(std::ptr::null_mut(), Ordering::Release);
        if !ready.adopted && (!ready.rebase || ready.reverse == reverse) {
            self.request.catch_up.store(true, Ordering::Release);
        }
        self.delta.full_refresh = ready.adopted && !ready.rebase;
        pointer
    }
    fn move_to(&mut self, hour: Hour, reverse: bool) -> io::Result<&Delta> {
        if let Some(error) = self.request.error.take() {
            return Err(error);
        }
        if self.request.reverse.load(Ordering::Acquire) != reverse {
            let speed = f64::from_bits(self.request.speed.load(Ordering::Relaxed));
            let refresh = f64::from_bits(self.request.refresh.load(Ordering::Relaxed));
            set_plan(
                &self.request,
                window(
                    self.values.hour,
                    self.values.hour,
                    reverse,
                    speed,
                    refresh,
                    false,
                ),
                reverse,
            );
        }
        for slot in self.delta.changes.as_mut_slice() {
            *slot = self
                .values
                .slots
                .internal(*slot)
                .expect("previous delta slot");
        }
        self.values.state.begin(&self.delta.changes);
        self.delta.changes.clear();
        if self
            .request
            .workspace_retired
            .load(Ordering::Acquire)
            .is_null()
        {
            let pointer = self
                .request
                .workspace
                .swap(std::ptr::null_mut(), Ordering::AcqRel);
            if !pointer.is_null() {
                let workspace = unsafe { &mut *pointer };
                std::mem::swap(&mut self.delta.coverage, &mut workspace.coverage);
                std::mem::swap(&mut self.delta.changes, &mut workspace.changes);
                self.request
                    .change_capacity
                    .store(self.delta.changes.capacity(), Ordering::Release);
                self.request
                    .coverage_capacity
                    .store(self.delta.coverage.capacity(), Ordering::Release);
                self.request
                    .workspace_retired
                    .store(pointer, Ordering::Release);
            }
        }
        let original = self.values.hour;
        self.delta.coverage.clear();
        self.delta.from = original;
        self.delta.to = original;
        self.delta.applied = false;
        self.delta.scope_switch = false;
        self.delta.full_refresh = false;
        let pointer = self.accept_ready(hour, reverse);
        let result = self.apply_resident(hour, reverse);
        if !pointer.is_null() {
            let ready = unsafe { &mut *pointer };
            if ready.adopted && ready.rebase && (result.is_err() || !self.delta.applied) {
                std::mem::swap(&mut self.values.state, &mut ready.state);
                std::mem::swap(&mut self.values.slots, &mut ready.slots);
                self.values.scope = ready.previous_scope;
                self.selection = self.values.state.selection();
                self.request
                    .active_rates
                    .store(self.values.state.active_rates(), Ordering::Release);
                self.delta.scope_switch = false;
                self.delta.changes.clear();
                ready.adopted = false;
            }
            self.request.retired.store(pointer, Ordering::Release);
        }
        result?;
        for slot in self.delta.changes.as_mut_slice() {
            *slot = self.values.slots.public(*slot);
        }
        self.post(hour, reverse, None);
        Ok(&self.delta)
    }
    fn apply_resident(&mut self, hour: Hour, reverse: bool) -> io::Result<()> {
        if hour == self.values.hour {
            self.delta.applied = true;
            self.delta.status = DeltaStatus::Applied;
            self.delta.to = hour;
            return Ok(());
        }
        let span = Hours {
            first: self.values.hour.min(hour),
            last: self.values.hour.max(hour),
        };
        let rows = Hours {
            first: span.first + 1,
            last: span.last,
        };
        let read = self.request.published.read();
        let snapshot = read.get();
        let mut served = false;
        let mut capacity_miss = false;
        let mut needs_restore = false;
        if let Some(snapshot) = snapshot.filter(|snapshot| {
            snapshot.contains(span, reverse)
                && (!matches!(self.request.keys, KeyFilter::All)
                    || matches!(self.selection.ordinals(), Selection::All { .. })
                    || (self.values.scope.first <= span.first
                        && self.values.scope.last >= span.last))
        }) {
            let required = frame_capacity(
                snapshot,
                rows,
                self.selection.len(),
                self.values.state.active_rates(),
            );
            let coverage_count = snapshot.coverage(span, |_| Ok(()))?;
            if self.delta.coverage.capacity() >= coverage_count
                && self.delta.changes.capacity() >= required
            {
                snapshot.coverage(span, |item| {
                    if !self.delta.coverage.push(item) {
                        return Err(oom());
                    }
                    Ok(())
                })?;
                let result = if reverse {
                    let mut result = Ok(());
                    for page in snapshot.pages.iter(rows).rev() {
                        let Payload::Reverse(ring) = &page.payload else {
                            unreachable!()
                        };
                        if let Err(error) = ring.decode(
                            rows,
                            &self.selection,
                            &mut self.values.state,
                            &mut self.delta.changes,
                        ) {
                            result = Err(error);
                            break;
                        }
                    }
                    result
                } else {
                    self.log.merged_changes(
                        &self.selection,
                        rows,
                        &mut snapshot
                            .pages
                            .iter(rows)
                            .filter_map(|page| match &page.payload {
                                Payload::Forward(page) => Some(&page.page),
                                _ => None,
                            }),
                        &mut self.values.state,
                        &mut self.delta.changes,
                    )
                };
                match result {
                    Ok(()) => {
                        served = self
                            .values
                            .state
                            .include_active_rates(&mut self.delta.changes);
                    }
                    Err(error) if error.kind() == io::ErrorKind::OutOfMemory => {
                        self.stats.capacity_misses += 1;
                        capacity_miss = true;
                        needs_restore = true;
                    }
                    Err(error) => return Err(error),
                }
                if served {
                    self.delta.changes.as_mut_slice().sort_unstable();
                    self.values.state.commit(&self.delta.changes);
                    self.request
                        .active_rates
                        .store(self.values.state.active_rates(), Ordering::Release);
                    let coverage = self
                        .delta
                        .coverage
                        .iter()
                        .find(|item| item.hours.contains(hour))
                        .unwrap();
                    self.values.accepted = coverage.accepted_through(hour);
                    self.values.hour = hour;
                    self.values.coverage.clear();
                    self.values.coverage.push(
                        crate::reference::clip(
                            coverage,
                            Hours {
                                first: hour,
                                last: hour,
                            },
                        )
                        .unwrap(),
                    );
                }
            } else {
                self.stats.capacity_misses += 1;
                capacity_miss = true;
            }
        }
        let planned = Hours {
            first: self.request.planned_first.load(Ordering::Acquire),
            last: self.request.planned_last.load(Ordering::Acquire),
        };
        let far = !planned.contains(hour)
            || self.request.planned_reverse.load(Ordering::Acquire) != reverse;
        drop(read);
        if served {
            self.delta.to = hour;
            self.delta.applied = true;
            self.delta.status = DeltaStatus::Applied;
        } else {
            self.stats.misses += 1;
            self.values.state.begin(&self.delta.changes);
            self.delta.changes.clear();
            if far || needs_restore {
                self.request.catch_up.store(true, Ordering::Release);
                self.delta.status = DeltaStatus::CatchingUp;
                self.stats.catch_up_drops += 1;
            } else if capacity_miss {
                self.delta.status = DeltaStatus::Preparing;
            } else {
                self.delta.status = DeltaStatus::NotResident;
                self.stats.not_resident += 1;
            }
            self.delta.coverage.clear();
            if self.delta.full_refresh {
                self.delta.changes.clear();
                self.delta.to = self.values.hour;
                self.delta.applied = true;
            }
        }
        Ok(())
    }
}
impl<S: LogSource> Drop for Cursor<S> {
    fn drop(&mut self) {
        self.request.stopped.store(true, Ordering::Release);
        wake(&self.pool);
        let mut jobs = self.pool.jobs.lock().unwrap();
        prune_jobs(&mut jobs, &self.pool.memory);
        let last = jobs.is_empty();
        if last {
            self.pool.stop.store(true, Ordering::Release);
        }
        drop(jobs);
        if last {
            wake(&self.pool);
            if let Some(handle) = self.pool.handle.lock().unwrap().take() {
                let _ = handle.join();
            }
        }
    }
}

fn validate_rate(speed: f64, refresh: f64) -> io::Result<()> {
    if !speed.is_finite()
        || !refresh.is_finite()
        || refresh < 0.0
        || (speed != 0.0 && refresh == 0.0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid playback rate",
        ));
    }
    Ok(())
}
fn wake<S: LogSource>(pool: &Pool<S>) {
    if let Some(worker) = pool.worker.get() {
        worker.unpark();
    }
}
fn coverage_at<S: LogSource>(
    log: &S,
    hour: Hour,
    memory: &Memory,
) -> io::Result<(Buffer<Coverage>, u64)> {
    let mut found = None;
    let mut duplicate = false;
    log.coverage(
        Hours {
            first: hour,
            last: hour,
        },
        &mut |item| {
            if item.hours.contains(hour) {
                if let Some(prior) = &found {
                    if prior != &item {
                        duplicate = true;
                    }
                } else {
                    found = Some(item);
                }
            }
            ControlFlow::Continue(())
        },
    )?;
    let item = found.filter(|_| !duplicate).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "source omitted or conflicted the open coverage",
        )
    })?;
    let accepted = item.accepted_through(hour);
    let mut coverage = Buffer::new(memory, Owner::State, 1)?;
    coverage.push(
        crate::reference::clip(
            &item,
            Hours {
                first: hour,
                last: hour,
            },
        )
        .unwrap(),
    );
    Ok((coverage, accepted))
}

fn shared_pool<S: LogSource>(log: &Arc<S>, memory: &Memory) -> io::Result<Arc<Pool<S>>> {
    let mut registry = POOLS.lock().unwrap();
    if let Some(entries) = registry.as_mut() {
        entries.retain(|entry| entry.pool.strong_count() != 0);
        for entry in entries.iter() {
            if entry.source == Arc::as_ptr(log) as usize && entry.memory.same(memory) {
                if let Some(pool) = entry
                    .pool
                    .upgrade()
                    .and_then(|pool| pool.downcast::<Pool<S>>().ok())
                    .filter(|pool| !pool.stop.load(Ordering::Acquire))
                {
                    return Ok(pool);
                }
            }
        }
    }
    let stack = memory.reserve(Owner::Transient, 256 * 1024)?;
    let pool = Arc::new(Pool {
        log: log.clone(),
        memory: memory.clone(),
        jobs: Mutex::new(Buffer::new(memory, Owner::Metadata, 0)?),
        cache: Mutex::new(Cache {
            pages: crate::window::Tree::default(),
            added: 0,
            clean_after: 1,
        }),
        worker: OnceLock::new(),
        handle: Mutex::new(None),
        stop: AtomicBool::new(false),
        _metadata: memory.reserve(
            Owner::Metadata,
            size_of::<Pool<S>>() + 2 * size_of::<usize>(),
        )?,
    });
    let erased: Arc<dyn Any + Send + Sync> = pool.clone();
    let entries = registry.get_or_insert(Buffer::new(&Memory::global(), Owner::Metadata, 0)?);
    let next = entries.len() + 1;
    entries.grow(&Memory::global(), Owner::Metadata, next)?;
    entries.push(RegistryEntry {
        source: Arc::as_ptr(log) as usize,
        memory: memory.clone(),
        pool: Arc::downgrade(&erased),
    });
    let worker_pool = pool.clone();
    let handle = thread::Builder::new()
        .name("log-read-ahead".into())
        .stack_size(stack.bytes())
        .spawn(move || {
            let _stack = stack;
            let _ = worker_pool.worker.set(thread::current());
            reader_loop(&worker_pool);
        })?;
    *pool.handle.lock().unwrap() = Some(handle);
    Ok(pool)
}
struct Workspace {
    coverage: Buffer<Coverage>,
    changes: Buffer<usize>,
    _metadata: Reservation,
}

fn reader_loop<S: LogSource>(pool: &Arc<Pool<S>>) {
    loop {
        if pool.stop.load(Ordering::Acquire) {
            break;
        }
        let count = {
            let mut jobs = pool.jobs.lock().unwrap();
            prune_jobs(&mut jobs, &pool.memory);
            jobs.len()
        };
        for index in 0..count {
            let request = pool
                .jobs
                .lock()
                .unwrap()
                .get(index)
                .and_then(|job| job.request.upgrade());
            let Some(request) = request.filter(|r| !r.stopped.load(Ordering::Acquire)) else {
                continue;
            };
            let seq = request.sequence.load(Ordering::Acquire);
            if seq & 1 != 0 || seq <= *request.completed.lock().unwrap() {
                continue;
            }
            let hour = request.hour.load(Ordering::Relaxed);
            let target = request.target.load(Ordering::Relaxed);
            let reverse = request.reverse.load(Ordering::Relaxed);
            let speed = f64::from_bits(request.speed.load(Ordering::Relaxed));
            let refresh = f64::from_bits(request.refresh.load(Ordering::Relaxed));
            if request.sequence.load(Ordering::Acquire) != seq {
                continue;
            }
            let worker_cycles = thread_cycles();
            collect_retired(&request);
            let base = if request.catch_up.load(Ordering::Acquire) {
                target
            } else if request.restoring.load(Ordering::Acquire) {
                request.restore_hour.load(Ordering::Acquire)
            } else {
                hour
            };
            clear_obsolete(&request, base, reverse);
            let mut restore_attempt = false;
            let result = (|| -> io::Result<()> {
                if request.ready.load(Ordering::Acquire).is_null()
                    && request.catch_up.swap(false, Ordering::AcqRel)
                {
                    restore_attempt = true;
                    let scope = window(target, target, reverse, speed, refresh, false);
                    set_plan(&request, scope, reverse);
                    let cycles = thread_cycles();
                    let state = pool.log.version_at(
                        &request.keys,
                        target,
                        slot_scope(scope, reverse),
                        &request.memory,
                    )?;
                    let restore = ready(
                        &*pool.log,
                        &request,
                        state,
                        target,
                        slot_scope(scope, reverse),
                        reverse,
                        false,
                    )?;
                    add_cycles(&request.counters.restore_cycles, cycles);
                    fill(
                        pool,
                        &request,
                        target,
                        target,
                        reverse,
                        speed,
                        refresh,
                        Some(restore),
                    )?;
                } else {
                    let base = if request.restoring.load(Ordering::Acquire) {
                        request.restore_hour.load(Ordering::Acquire)
                    } else {
                        hour
                    };
                    fill(pool, &request, base, target, reverse, speed, refresh, None)?;
                }
                let cycles = thread_cycles();
                let prepare_hour = if request.restoring.load(Ordering::Acquire) {
                    request.restore_hour.load(Ordering::Acquire)
                } else {
                    hour
                };
                prepare_frame(&request, prepare_hour, target, reverse, speed, refresh)?;
                add_cycles(&request.counters.preparation, cycles);
                Ok(())
            })();
            if let Err(error) = result {
                if restore_attempt
                    && request.ready.load(Ordering::Acquire).is_null()
                    && request.retired.load(Ordering::Acquire).is_null()
                {
                    request.catch_up.store(true, Ordering::Release);
                }
                if error.kind() == io::ErrorKind::OutOfMemory {
                    request.counters.shortened.fetch_add(1, Ordering::Relaxed);
                } else {
                    request.error.post(error);
                }
            }
            add_cycles(&request.counters.worker, worker_cycles);
            *request.completed.lock().unwrap() = seq;
            request.completion.notify_all();
        }
        if pool.jobs.lock().unwrap().iter().any(|job| {
            job.request.upgrade().is_some_and(|r| {
                !r.stopped.load(Ordering::Acquire)
                    && r.sequence.load(Ordering::Acquire) > *r.completed.lock().unwrap()
            })
        }) {
            continue;
        }
        thread::park();
    }
}
fn set_plan<S: LogSource>(request: &Request<S>, span: Hours, reverse: bool) {
    request.planned_first.store(span.first, Ordering::Release);
    request.planned_last.store(span.last, Ordering::Release);
    request.planned_reverse.store(reverse, Ordering::Release);
}
fn ready<S: LogSource>(
    log: &S,
    request: &Request<S>,
    state: S::State,
    hour: Hour,
    scope: Hours,
    reverse: bool,
    rebase: bool,
) -> io::Result<Box<Ready<S::State>>> {
    let (coverage, accepted) = coverage_at(log, hour, &request.memory)?;
    let metadata = request
        .memory
        .reserve(Owner::Metadata, size_of::<Ready<S::State>>())?;
    let previous = request.slots.lock().unwrap().clone();
    let slots = Slots::new(
        state.selection(),
        rebase.then_some(&*previous),
        &request.memory,
    )?;
    let previous_scope = *request.selection_scope.lock().unwrap();
    Ok(Box::new(Ready {
        state,
        slots,
        previous,
        scope,
        previous_scope,
        reverse,
        adopted: false,
        hour,
        accepted,
        coverage,
        rebase,
        _metadata: metadata,
    }))
}

fn add_cycles(counter: &AtomicU64, start: Option<u64>) {
    if let Some((start, end)) = start.zip(thread_cycles()) {
        counter.fetch_add(end.saturating_sub(start), Ordering::Relaxed);
    }
}
fn oom() -> io::Error {
    io::Error::new(io::ErrorKind::OutOfMemory, "read-ahead shortened")
}
fn slot_scope(span: Hours, reverse: bool) -> Hours {
    let growth = (i64::from(span.last) - i64::from(span.first)).min(i64::from(Hour::MAX)) as Hour;
    if reverse {
        Hours {
            first: span.first.saturating_sub(growth),
            last: span.last,
        }
    } else {
        Hours {
            first: span.first,
            last: span.last.saturating_add(growth),
        }
    }
}
fn window(hour: Hour, target: Hour, reverse: bool, speed: f64, refresh: f64, prime: bool) -> Hours {
    let distance = if prime && refresh > 0.0 {
        (speed.abs() / refresh).ceil()
    } else {
        (speed.abs() * 2.0).ceil()
    };
    let distance = distance.min(f64::from(Hour::MAX)) as Hour;
    if reverse {
        Hours {
            first: hour.saturating_sub(distance).min(target),
            last: hour,
        }
    } else {
        Hours {
            first: hour,
            last: hour.saturating_add(distance).max(target),
        }
    }
}
fn collect_retired<S: LogSource>(request: &Request<S>) {
    drop(request.published.collect());
    let retired = request.retired.swap(std::ptr::null_mut(), Ordering::AcqRel);
    if !retired.is_null() {
        unsafe {
            let ready = Box::from_raw(retired);
            request.restoring.store(false, Ordering::Release);
            request.ready_rates.store(0, Ordering::Release);
            if !ready.adopted {
                *request.selection.lock().unwrap() = ready.previous.selection.clone();
                *request.slots.lock().unwrap() = ready.previous.clone();
                *request.selection_scope.lock().unwrap() = ready.previous_scope;
            }
            drop(ready);
        }
    }
    let retired = request
        .workspace_retired
        .swap(std::ptr::null_mut(), Ordering::AcqRel);
    if !retired.is_null() {
        unsafe {
            drop(Box::from_raw(retired));
        }
    }
}
fn publish<S: LogSource>(request: &Request<S>, mut snapshot: Box<Snapshot<S::Decoder>>) -> bool {
    loop {
        match request.published.publish(snapshot) {
            Ok(previous) => {
                drop(previous);
                return true;
            }
            Err(next) => {
                snapshot = next;
                request.counters.retries.fetch_add(1, Ordering::Relaxed);
                if request.stopped.load(Ordering::Acquire) {
                    return false;
                }
                thread::yield_now();
            }
        }
    }
}
fn snapshot<S: LogSource>(
    request: &Request<S>,
    pages: crate::window::Tree<WindowPage<S::Decoder>>,
    served: Hours,
    reverse: bool,
) -> io::Result<Box<Snapshot<S::Decoder>>> {
    let metadata = request
        .memory
        .soft()
        .reserve(Owner::Metadata, size_of::<Snapshot<S::Decoder>>())?;
    Ok(Box::new(Snapshot {
        pages,
        selection: request.selection.lock().unwrap().clone(),
        served,
        reverse,
        _metadata: metadata,
    }))
}
fn frame_capacity<D: PageDecoder>(
    snapshot: &Snapshot<D>,
    span: Hours,
    selected: usize,
    rates: usize,
) -> usize {
    snapshot
        .pages
        .iter(span)
        .fold(rates, |n, p| n.saturating_add(p.info.records))
        .min(selected)
}
fn prepare_frame<S: LogSource>(
    request: &Request<S>,
    hour: Hour,
    target: Hour,
    reverse: bool,
    speed: f64,
    refresh: f64,
) -> io::Result<()> {
    let read = request.published.read();
    if let Some(snapshot) = read.get() {
        let pending_rates = request.ready_rates.load(Ordering::Acquire);
        prepare_frame_for(
            request,
            snapshot,
            hour,
            target,
            reverse,
            speed,
            refresh,
            pending_rates,
        )?;
    }
    Ok(())
}
fn prepare_frame_for<S: LogSource>(
    request: &Request<S>,
    snapshot: &Snapshot<S::Decoder>,
    hour: Hour,
    target: Hour,
    reverse: bool,
    speed: f64,
    refresh: f64,
    pending_rates: usize,
) -> io::Result<()> {
    let span = window(hour, target, reverse, speed, refresh, true).intersection(snapshot.served);
    if let Some(span) = span {
        let rates = request
            .active_rates
            .load(Ordering::Acquire)
            .max(pending_rates);
        let changes = frame_capacity(snapshot, span, snapshot.selection.len(), rates);
        let coverage = snapshot.pages.coverage_runs_for(snapshot.served);
        prepare_workspace(request, coverage, changes)?;
    }
    Ok(())
}
fn prepare_workspace<S: LogSource>(
    request: &Request<S>,
    requested_coverage: usize,
    requested_changes: usize,
) -> io::Result<()> {
    let prepared_coverage = request.prepared_coverage.load(Ordering::Acquire);
    let prepared_changes = request.prepared_changes.load(Ordering::Acquire);
    let current_coverage = request.coverage_capacity.load(Ordering::Acquire);
    let current_changes = request.change_capacity.load(Ordering::Acquire);
    let mut coverage = requested_coverage
        .max(current_coverage)
        .max(prepared_coverage);
    let mut changes = requested_changes
        .max(current_changes)
        .max(prepared_changes);
    if coverage == prepared_coverage && changes == prepared_changes {
        return Ok(());
    }
    let build = |coverage, changes| -> io::Result<Box<Workspace>> {
        let metadata = request
            .memory
            .reserve(Owner::Metadata, size_of::<Workspace>())?;
        Ok(Box::new(Workspace {
            coverage: Buffer::new(&request.memory, Owner::Delta, coverage)?,
            changes: Buffer::new(&request.memory, Owner::Delta, changes)?,
            _metadata: metadata,
        }))
    };
    let workspace = match build(coverage, changes) {
        Ok(workspace) => workspace,
        Err(error) if error.kind() == io::ErrorKind::OutOfMemory => {
            let old = request
                .workspace
                .swap(std::ptr::null_mut(), Ordering::AcqRel);
            if old.is_null() {
                return Err(error);
            }
            // SAFETY: `old` came from `Box::into_raw` in `prepare_workspace` and was uniquely claimed by `swap`.
            unsafe {
                drop(Box::from_raw(old));
            }
            let current_coverage = request.coverage_capacity.load(Ordering::Acquire);
            let current_changes = request.change_capacity.load(Ordering::Acquire);
            request
                .prepared_coverage
                .store(current_coverage, Ordering::Release);
            request
                .prepared_changes
                .store(current_changes, Ordering::Release);
            coverage = requested_coverage.max(current_coverage);
            changes = requested_changes.max(current_changes);
            if coverage == current_coverage && changes == current_changes {
                return Ok(());
            }
            build(coverage, changes)?
        }
        Err(error) => return Err(error),
    };
    let previous = request
        .workspace
        .swap(Box::into_raw(workspace), Ordering::AcqRel);
    if !previous.is_null() {
        // SAFETY: `previous` came from `Box::into_raw` in `prepare_workspace` and was uniquely claimed by `swap`.
        unsafe {
            drop(Box::from_raw(previous));
        }
    }
    request
        .prepared_coverage
        .store(coverage, Ordering::Release);
    request.prepared_changes.store(changes, Ordering::Release);
    Ok(())
}
fn available_hours<S: LogSource>(
    pool: &Pool<S>,
    request: &Request<S>,
    desired: Hours,
) -> io::Result<Option<Hours>> {
    let mut span: Option<Hours> = None;
    pool.log.pages(&request.keys, desired, &mut |page| {
        if let Some(part) = page.hours.intersection(desired) {
            span = Some(match span {
                Some(held) => Hours {
                    first: held.first.min(part.first),
                    last: held.last.max(part.last),
                },
                None => part,
            });
        }
        ControlFlow::Continue(())
    })?;
    Ok(span)
}
fn plan<S: LogSource>(
    pool: &Pool<S>,
    request: &Request<S>,
    hours: Hours,
) -> io::Result<Buffer<PageInfo>> {
    let cycles = thread_cycles();
    let mut plan = Buffer::new(&request.memory.soft(), Owner::Metadata, 0)?;
    let mut failure = None;
    pool.log.pages(&request.keys, hours, &mut |page| {
        if plan.len() == plan.capacity() {
            let capacity = (plan.len() + 1).max(plan.capacity().saturating_mul(2));
            if let Err(error) = plan.grow(&request.memory.soft(), Owner::Metadata, capacity) {
                failure = Some(error);
                return ControlFlow::Break(());
            }
        }
        if plan.last().is_some_and(|prior: &PageInfo| {
            prior.id >= page.id
                || prior.hours.first > page.hours.first
                || prior.hours.last > page.hours.last
        }) {
            failure = Some(io::Error::new(
                io::ErrorKind::InvalidData,
                "unordered page directory",
            ));
            return ControlFlow::Break(());
        }
        plan.push(page);
        ControlFlow::Continue(())
    })?;
    add_cycles(&request.counters.planning, cycles);
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(plan)
}
fn cached<S: LogSource>(pool: &Pool<S>, info: PageInfo) -> Option<Arc<ResidentPage<S::Decoder>>> {
    pool.cache
        .lock()
        .unwrap()
        .pages
        .get(info.id)
        .filter(|entry| entry.info == info)
        .and_then(|entry| entry.page.upgrade())
}
fn get_page<S: LogSource>(
    pool: &Pool<S>,
    request: &Request<S>,
    info: PageInfo,
    credit: Option<Memory>,
) -> io::Result<Arc<ResidentPage<S::Decoder>>> {
    if let Some(page) = cached(pool, info) {
        return Ok(page);
    }
    let bound = info
        .resident_bytes
        .checked_add(size_of::<ResidentPage<S::Decoder>>() + 2 * size_of::<usize>())
        .ok_or_else(oom)?;
    let memory = match credit {
        Some(memory) => memory,
        None => Memory::prepaid(pool.memory.soft().reserve(Owner::Forward, bound)?)?,
    };
    let wall = Instant::now();
    let cycles = thread_cycles();
    let page = pool.log.read_page(info, &memory)?;
    page.validate()?;
    request
        .counters
        .read_wall
        .fetch_add(wall.elapsed().as_nanos() as u64, Ordering::Relaxed);
    add_cycles(&request.counters.read_cycles, cycles);
    request.counters.pages.fetch_add(1, Ordering::Relaxed);
    request
        .counters
        .bytes
        .fetch_add(page.bytes.len() as u64, Ordering::Relaxed);
    let metadata = memory.reserve(
        Owner::Forward,
        size_of::<ResidentPage<S::Decoder>>() + 2 * size_of::<usize>(),
    )?;
    let page = Arc::new(ResidentPage {
        page,
        charged_bytes: bound - memory.remaining_credit(),
        _metadata: metadata,
    });
    let mut cache = pool.cache.lock().unwrap();
    if cache.added >= cache.clean_after {
        cache
            .pages
            .filter(|entry| entry.page.strong_count() != 0, &pool.memory.soft())?;
        cache.clean_after = cache.pages.len().max(1);
        cache.added = 0;
    }
    let metadata = pool.memory.soft().reserve(
        Owner::Metadata,
        size_of::<CacheEntry<S::Decoder>>() + 2 * size_of::<usize>(),
    )?;
    cache.pages.insert(
        Arc::new(CacheEntry {
            info,
            page: Arc::downgrade(&page),
            _metadata: metadata,
        }),
        &pool.memory.soft(),
    )?;
    cache.added += 1;
    Ok(page)
}
fn window_page<S: LogSource>(
    request: &Request<S>,
    info: PageInfo,
    payload: Payload<S::Decoder>,
) -> io::Result<Arc<WindowPage<S::Decoder>>> {
    let metadata = request.memory.soft().reserve(
        Owner::Metadata,
        size_of::<WindowPage<S::Decoder>>() + 2 * size_of::<usize>(),
    )?;
    let coverage_runs = crate::window::coverage_runs(match &payload {
        Payload::Forward(page) => &page.coverage,
        Payload::Reverse(ring) => &ring.coverage,
    });
    Ok(Arc::new(WindowPage {
        info,
        payload,
        coverage_runs,
        _metadata: metadata,
    }))
}
fn clear_obsolete<S: LogSource>(request: &Request<S>, hour: Hour, reverse: bool) {
    let obsolete = {
        let read = request.published.read();
        read.get()
            .is_some_and(|old| old.reverse != reverse || !old.served.contains(hour))
    };
    if obsolete {
        while !request.published.clear() {
            if request.stopped.load(Ordering::Acquire) {
                return;
            }
            thread::yield_now();
        }
        drop(request.published.collect());
    }
}
fn fill<S: LogSource>(
    pool: &Pool<S>,
    request: &Request<S>,
    hour: Hour,
    target: Hour,
    reverse: bool,
    speed: f64,
    refresh: f64,
    restore: Option<Box<Ready<S::State>>>,
) -> io::Result<()> {
    let desired = window(hour, target, reverse, speed, refresh, false);
    set_plan(request, desired, reverse);
    clear_obsolete(request, hour, reverse);
    let scope = *request.selection_scope.lock().unwrap();
    let previous = request.slots.lock().unwrap().clone();
    let mut pending = restore;
    if pending.is_none()
        && request.ready.load(Ordering::Acquire).is_null()
        && matches!(request.keys, KeyFilter::All)
        && !matches!(previous.selection.ordinals(), Selection::All { .. })
        && (desired.first < scope.first || desired.last > scope.last)
        && let Some(available) = available_hours(pool, request, desired)?
            .filter(|available| available.first < scope.first || available.last > scope.last)
    {
        let preparation = thread_cycles();
        let ahead = {
            let read = request.published.read();
            read.get()
                .filter(|s| s.reverse == reverse && s.served.contains(hour))
                .map_or(available, |s| s.served)
        };
        let ahead = ahead
            .intersection(available)
            .and_then(|span| span.intersection(scope))
            .unwrap_or(Hours {
                first: hour,
                last: hour,
            });
        let frame = window(hour, target, reverse, speed, refresh, true)
            .intersection(available)
            .unwrap_or(Hours {
                first: hour,
                last: hour,
            });
        let cut = if frame.first < scope.first || frame.last > scope.last {
            hour
        } else if reverse {
            hour - ((i64::from(hour) - i64::from(ahead.first)) / 2) as Hour
        } else {
            hour + ((i64::from(ahead.last) - i64::from(hour)) / 2) as Hour
        };
        let scope = slot_scope(available, reverse);
        let state = pool
            .log
            .version_at(&request.keys, cut, scope, &request.memory)?;
        pending = Some(ready(
            &*pool.log, request, state, cut, scope, reverse, true,
        )?);
        request.counters.scopes.fetch_add(1, Ordering::Relaxed);
        add_cycles(&request.counters.preparation, preparation);
    }
    if let Some(ready) = &pending {
        *request.selection.lock().unwrap() = ready.state.selection();
        *request.slots.lock().unwrap() = ready.slots.clone();
        *request.selection_scope.lock().unwrap() = ready.scope;
    }
    let mut pending = PreparedScope {
        request,
        previous,
        previous_scope: scope,
        ready: pending,
    };
    let cycles = thread_cycles();
    let _filling = request.filling.lock().unwrap();
    let (mut pages, old_span) = {
        let read = request.published.read();
        match read.get().filter(|s| s.reverse == reverse) {
            Some(old) => (old.pages.clone(), old.served.intersection(desired)),
            None => (crate::window::Tree::default(), None),
        }
    };
    pages.retain(desired, &request.memory.soft())?;
    let existing = old_span.filter(|span| span.contains(hour));
    if existing.is_none() {
        pages = crate::window::Tree::default();
    }
    if let Some(served) = existing {
        let next = snapshot(request, pages.clone(), served, reverse)?;
        if let Some(ready) = pending.ready.as_ref() {
            prepare_frame_for(
                request,
                &next,
                hour,
                target,
                reverse,
                speed,
                refresh,
                ready.state.active_rates(),
            )?;
        }
        publish(request, next);
        if pending.ready.is_some() {
            pending.handoff(served, false);
        }
    }
    add_cycles(&request.counters.publication, cycles);
    let extension = match existing {
        Some(span) if reverse => Hours {
            first: desired.first,
            last: span.first,
        },
        Some(span) => Hours {
            first: span.last,
            last: desired.last,
        },
        None => desired,
    };
    let plan = plan(pool, request, extension)?;
    let nominal = window(hour, target, reverse, speed, refresh, true);
    let mut credits = Buffer::new(&request.memory.soft(), Owner::Metadata, plan.len())?;
    for info in plan.iter() {
        let credit = if !reverse
            && pages.get(info.id).is_none()
            && info.hours.intersects(nominal)
            && cached(pool, *info).is_none()
        {
            Some(
                pool.memory.reserve(
                    Owner::Forward,
                    info.resident_bytes
                        .checked_add(size_of::<ResidentPage<S::Decoder>>() + 2 * size_of::<usize>())
                        .ok_or_else(oom)?,
                )?,
            )
        } else {
            None
        };
        credits.push(credit);
    }
    let mut served = existing;
    let mut end = if reverse { plan.len() } else { 0 };
    let mut reverse_limit = usize::MAX;
    while if reverse { end > 0 } else { end < plan.len() } {
        if request.stopped.load(Ordering::Acquire)
            || request.reverse.load(Ordering::Acquire) != reverse
            || (request.catch_up.load(Ordering::Acquire)
                && request.ready.load(Ordering::Acquire).is_null())
        {
            break;
        }
        let (first, last) = if reverse {
            let last = end;
            let chunk = window(
                plan[last - 1].hours.last,
                plan[last - 1].hours.last,
                true,
                speed,
                refresh,
                true,
            );
            let run = pool.log.checkpoint_run(plan[last - 1]);
            let mut first = last - 1;
            while first > 0
                && (plan[first - 1].hours.last >= plan[first].hours.first
                    || (last - first < reverse_limit
                        && (plan[first - 1].hours.last >= chunk.first
                            || pool.log.checkpoint_run(plan[first - 1]) == run)))
            {
                first -= 1;
            }
            end = first;
            (first, last)
        } else {
            let first = end;
            end += 1;
            (first, end)
        };
        let mut covered_last = plan[first].hours.last;
        for info in &plan[first + 1..last] {
            if i64::from(info.hours.first) > i64::from(covered_last) + 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "page directory omitted served hours",
                ));
            }
            covered_last = covered_last.max(info.hours.last);
        }
        let mut missing = Buffer::new(&request.memory.soft(), Owner::Metadata, last - first)?;
        for info in &plan[first..last] {
            if pages.get(info.id).is_none() {
                missing.push(*info);
            }
        }
        let result = (|| -> io::Result<()> {
            let mut inverse = if reverse && !missing.is_empty() {
                Some(prepare_inverse(pool, request, &missing, desired)?)
            } else {
                None
            };
            let mut forward = Buffer::new(&request.memory.soft(), Owner::Metadata, missing.len())?;
            for (index, info) in missing.iter().enumerate() {
                if !reverse
                    && pages
                        .bytes()
                        .saturating_add(info.resident_bytes)
                        .saturating_add(request.memory.used())
                        > request.memory.limit()
                {
                    return Err(oom());
                }
                forward.push(get_page(
                    pool,
                    request,
                    *info,
                    match &mut inverse {
                        Some(workspace) => workspace.credits[index].take(),
                        None => credits[plan.binary_search_by_key(&info.id, |p| p.id).unwrap()]
                            .take()
                            .map(Memory::prepaid)
                            .transpose()?,
                    },
                )?);
            }
            let rings = inverse
                .map(|workspace| build_inverse(request, &forward, workspace))
                .transpose()?;
            let publication = thread_cycles();
            let mut next_pages = pages.clone();
            for (index, page) in forward.iter().enumerate() {
                let payload = match &rings {
                    Some(rings) => Payload::Reverse(rings[index].clone()),
                    None => Payload::Forward(page.clone()),
                };
                next_pages.insert(
                    window_page::<S>(request, page.info, payload)?,
                    &request.memory.soft(),
                )?;
            }
            let chunk = Hours {
                first: plan[first].hours.first.max(desired.first),
                last: plan[last - 1].hours.last.min(desired.last),
            };
            if let Some(prior) = served {
                if i64::from(chunk.first) > i64::from(prior.last) + 1
                    || i64::from(prior.first) > i64::from(chunk.last) + 1
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "page directory omitted served hours",
                    ));
                }
            }
            let next_served = served.map_or(chunk, |prior| Hours {
                first: prior.first.min(chunk.first),
                last: prior.last.max(chunk.last),
            });
            let mut span = next_served;
            if !reverse && last < plan.len() && plan[last].hours.first <= span.last {
                span.last = span
                    .last
                    .min(plan[last].hours.first.saturating_sub(1))
                    .max(existing.map_or(Hour::MIN, |span| span.last));
            }
            if reverse && first > 0 && plan[first - 1].hours.last >= span.first {
                span.first = span
                    .first
                    .max(plan[first - 1].hours.last.saturating_add(1))
                    .min(existing.map_or(Hour::MAX, |span| span.first));
            }
            if span.first <= span.last {
                let next = snapshot(request, next_pages.clone(), span, reverse)?;
                let pending_rates = pending.ready.as_ref().map_or_else(
                    || request.ready_rates.load(Ordering::Acquire),
                    |ready| ready.state.active_rates(),
                );
                prepare_frame_for(
                    request,
                    &next,
                    hour,
                    target,
                    reverse,
                    speed,
                    refresh,
                    pending_rates,
                )?;
                request
                    .counters
                    .peak
                    .fetch_max(next.bytes(), Ordering::Relaxed);
                publish(request, next);
                pages = next_pages;
                served = Some(next_served);
                pending.handoff(
                    span,
                    if reverse {
                        first == 0
                    } else {
                        last == plan.len()
                    },
                );
            } else {
                pages = next_pages;
                served = Some(next_served);
            }
            add_cycles(&request.counters.publication, publication);
            Ok(())
        })();
        if let Err(error) = result {
            if error.kind() == io::ErrorKind::OutOfMemory {
                request.counters.shortened.fetch_add(1, Ordering::Relaxed);
                if reverse && last - first > 1 {
                    let mut next = first + (last - first) / 2;
                    while next > first && plan[next - 1].hours.last >= plan[next].hours.first {
                        next -= 1;
                    }
                    if next > first {
                        reverse_limit = last - next;
                        end = last;
                        continue;
                    }
                }
                let read = request.published.read();
                if let Some(snapshot) = read.get().filter(|snapshot| snapshot.reverse == reverse) {
                    set_plan(request, snapshot.served, reverse);
                }
                break;
            }
            return Err(error);
        }
    }
    if pending.ready.as_ref().is_some_and(|ready| !ready.rebase) {
        request.catch_up.store(true, Ordering::Release);
    }
    Ok(())
}
struct InverseBuffers {
    bytes: Buffer<u8>,
    runs: Buffer<InverseRun>,
    coverage: Buffer<Coverage>,
    compact: Memory,
    metadata: Reservation,
}
struct InverseWorkspace<S: ValueStore> {
    state: S,
    changed: Buffer<usize>,
    rings: Buffer<Arc<InversePage>>,
    buffers: Buffer<InverseBuffers>,
    credits: Buffer<Option<Memory>>,
    wall: Instant,
    cycles: Option<u64>,
}
fn prepare_inverse<S: LogSource>(
    pool: &Pool<S>,
    request: &Request<S>,
    pages: &[PageInfo],
    scope: Hours,
) -> io::Result<InverseWorkspace<S::State>> {
    let ring_memory = &request.memory;
    let wall = Instant::now();
    let cycles = thread_cycles();
    let scope = Hours {
        first: scope.first.min(pages[0].hours.first),
        last: scope.last.max(pages[pages.len() - 1].hours.last),
    };
    let state = pool
        .log
        .version_before_page(&request.keys, pages[0], scope, &request.memory)?;
    let selection = state.selection();
    let capacity = pages
        .iter()
        .map(|p| p.records)
        .max()
        .unwrap_or(0)
        .min(selection.len());
    let changed = Buffer::new(&request.memory, Owner::Transient, capacity)?;
    let rings = Buffer::new(ring_memory, Owner::Reverse, pages.len())?;
    let mut buffers = Buffer::new(&request.memory, Owner::Transient, pages.len())?;
    let mut credits = Buffer::new(&request.memory, Owner::Transient, pages.len())?;
    for page in pages {
        let runs = (i64::from(page.hours.last) - i64::from(page.hours.first) + 1) as usize;
        let count = page.records.min(selection.len().saturating_mul(runs));
        let bound = count
            .checked_mul(state.max_record_bytes() + ((usize::BITS + 6) / 7) as usize + 5)
            .ok_or_else(oom)?;
        buffers.push(InverseBuffers {
            bytes: Buffer::new(ring_memory, Owner::Reverse, bound)?,
            runs: Buffer::new(ring_memory, Owner::Reverse, runs.min(count))?,
            coverage: Buffer::new(ring_memory, Owner::Reverse, page.coverage)?,
            compact: Memory::prepaid(ring_memory.reserve(Owner::Transient, bound)?)?,
            metadata: ring_memory.reserve(
                Owner::Reverse,
                size_of::<InversePage>() + 2 * size_of::<usize>(),
            )?,
        });
        credits.push(if cached(pool, *page).is_none() {
            Some(Memory::prepaid(
                pool.memory.reserve(
                    Owner::Forward,
                    page.resident_bytes
                        .checked_add(size_of::<ResidentPage<S::Decoder>>() + 2 * size_of::<usize>())
                        .ok_or_else(oom)?,
                )?,
            )?)
        } else {
            None
        });
    }
    let decode_bytes = pages.iter().try_fold(0usize, |bytes, page| {
        bytes.checked_add(page.resident_bytes).ok_or_else(oom)
    })?;
    if request.memory.used().saturating_add(decode_bytes) > request.memory.limit() {
        return Err(oom());
    }
    buffers.as_mut_slice().reverse();
    Ok(InverseWorkspace {
        state,
        changed,
        rings,
        buffers,
        credits,
        wall,
        cycles,
    })
}
fn build_inverse<S: LogSource>(
    request: &Request<S>,
    pages: &[Arc<ResidentPage<S::Decoder>>],
    workspace: InverseWorkspace<S::State>,
) -> io::Result<Buffer<Arc<InversePage>>> {
    let InverseWorkspace {
        mut state,
        mut changed,
        mut rings,
        mut buffers,
        credits: _,
        wall,
        cycles,
    } = workspace;
    let selection = state.selection();
    let all = Hours {
        first: Hour::MIN,
        last: Hour::MAX,
    };
    for page in pages {
        let InverseBuffers {
            mut bytes,
            runs: mut starts,
            mut coverage,
            compact,
            metadata,
        } = buffers.pop().unwrap();
        let bound = bytes.capacity();
        for _ in 0..bound {
            bytes.push(0);
        }
        let mut offset = 0;
        state.begin(&changed);
        changed.clear();
        page.decoder
            .decode(&page.bytes, all, &selection, &mut |hour, slot, record| {
                if starts
                    .last()
                    .is_none_or(|run: &InverseRun| run.hour != hour)
                {
                    state.commit(&changed);
                    state.begin(&changed);
                    changed.clear();
                    if !starts.push(InverseRun {
                        hour,
                        start: offset,
                    }) {
                        return Err(oom());
                    }
                }
                let prior = state.record(slot, true);
                let mut writer = Writer::new(Some(&mut bytes[offset..]));
                writer.uint(selection.id(slot) as u64)?;
                writer.record(prior, hour)?;
                let len = writer.len;
                if state.stage(slot, record)? {
                    changed.push(slot);
                    offset += len;
                }
                Ok(())
            })?;
        state.commit(&changed);
        state.begin(&changed);
        changed.clear();
        bytes.truncate(offset);
        bytes.compact(&compact, Owner::Reverse)?;
        for item in page.coverage.iter() {
            if !coverage.push(item.clone()) {
                return Err(oom());
            }
        }
        rings.push(Arc::new(InversePage {
            bytes,
            runs: starts,
            coverage,
            _metadata: metadata,
        }));
    }
    request
        .counters
        .reverse_wall
        .fetch_add(wall.elapsed().as_nanos() as u64, Ordering::Relaxed);
    add_cycles(&request.counters.reverse_cycles, cycles);
    Ok(rings)
}
impl<S: LogSource> crate::budget::Reclaim for Pool<S> {
    fn reclaim_one(&self, budget: &Memory) -> bool {
        if !self.memory.is_within(budget) {
            return false;
        }
        let Ok(jobs) = self.jobs.try_lock() else {
            return false;
        };
        for job in jobs.iter() {
            let Some(request) = job.request.upgrade() else {
                continue;
            };
            let Ok(_filling) = request.filling.try_lock() else {
                continue;
            };
            let hour = request.hour.load(Ordering::Relaxed);
            let target = request.target.load(Ordering::Relaxed);
            let critical = Hours {
                first: hour.min(target),
                last: hour.max(target),
            };
            let read = request.published.read();
            let Some(old) = read.get() else {
                continue;
            };
            let edge = if old.reverse {
                old.pages.first()
            } else {
                old.pages.last()
            };
            let Some(edge) = edge.filter(|p| !p.hours.intersects(critical)) else {
                continue;
            };
            let keep = if old.reverse {
                Hours {
                    first: edge.hours.last.saturating_add(1),
                    last: old.served.last,
                }
            } else {
                Hours {
                    first: old.served.first,
                    last: edge.hours.first.saturating_sub(1),
                }
            };
            if keep.first > keep.last {
                continue;
            }
            let used = budget.used();
            let mut tree = old.pages.clone();
            if tree.retain(keep, &request.memory.soft()).is_err() {
                continue;
            }
            let next = snapshot(&request, tree, keep, old.reverse);
            drop(read);
            let Ok(next) = next else {
                continue;
            };
            if let Ok(previous) = request.published.publish(next) {
                drop(previous);
                if budget.used() < used {
                    request.counters.shortened.fetch_add(1, Ordering::Relaxed);
                }
                return true;
            } else {
                request.counters.retries.fetch_add(1, Ordering::Relaxed);
            }
        }
        false
    }
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    #[test]
    fn values_return_absent_for_vacant_and_out_of_range_slots() {
        let memory = Memory::new(1_000_000).unwrap();
        let source = crate::tests::fixture(&memory, 2, 10);
        let previous = Slots::new(Arc::new(Selection::All { len: 10 }), None, &memory).unwrap();
        let address = Address {
            key: 1.into(),
            field: 0.into(),
        };
        let state = source
            .version_at(
                &KeyFilter::sorted(Arc::from([address.clone()])).unwrap(),
                0,
                Hours { first: 0, last: 1 },
                &memory,
            )
            .unwrap();
        let slots = Slots::new(state.selection(), Some(&previous), &memory).unwrap();
        let values = Values {
            state,
            slots,
            scope: Hours { first: 0, last: 1 },
            hour: 0,
            accepted: 1,
            coverage: Buffer::new(&memory, Owner::State, 0).unwrap(),
        };
        assert_eq!(values.at(1).unwrap().0, &address);
        assert!(values.at(0).is_none());
        assert!(values.at(2).is_none());
        assert!(values.at(usize::MAX).is_none());
    }

    #[test]
    fn reclaim_skips_a_filling_cursor_without_shortening_or_losing_pages() {
        for speed in [2.0, -2.0] {
            let memory = Memory::new(4_000_000).unwrap();
            let source = crate::tests::fixture(&memory, 6, 10);
            let cursor = Cursor::open_with(
                &source,
                KeyFilter::All,
                if speed > 0.0 { 0 } else { 4 },
                CursorOptions {
                    memory: memory.clone(),
                    working_set: memory.limit(),
                    speed,
                    refresh_hz: 2.0,
                },
            )
            .unwrap();
            cursor.wait_read_ahead().unwrap();
            stop_reader(&cursor);
            let filling = cursor.request.filling.lock().unwrap();
            let (pages, served) = {
                let read = cursor.request.published.read();
                let snapshot = read.get().unwrap();
                (snapshot.pages.clone(), snapshot.served)
            };
            assert!(pages.len() > 1);
            let shortened = cursor.stats().shortened_windows;
            let used = memory.used();
            assert!(!crate::budget::Reclaim::reclaim_one(&*cursor.pool, &memory));
            assert_eq!(memory.used(), used);
            assert_eq!(cursor.stats().shortened_windows, shortened);
            {
                let read = cursor.request.published.read();
                let snapshot = read.get().unwrap();
                assert_eq!(snapshot.served, served);
                assert_eq!(snapshot.pages.len(), pages.len());
                assert_eq!(snapshot.pages.bytes(), pages.bytes());
                for page in pages.iter(served) {
                    assert!(Arc::ptr_eq(
                        snapshot.pages.get(page.info.id).unwrap(),
                        pages.get(page.info.id).unwrap(),
                    ));
                }
            }
            drop(pages);
            drop(filling);
            let used = memory.used();
            assert!(crate::budget::Reclaim::reclaim_one(&*cursor.pool, &memory));
            assert!(memory.used() < used);
            assert_eq!(cursor.stats().shortened_windows, shortened + 1);
        }
    }

    fn stop_reader<S: LogSource>(cursor: &Cursor<S>) {
        cursor.pool.stop.store(true, Ordering::Release);
        wake(&cursor.pool);
        let handle = cursor.pool.handle.lock().unwrap().take().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn reclaim_counts_a_shortening_only_when_the_trim_releases_bytes() {
        let memory = Memory::new(4_000_000).unwrap();
        let source = crate::tests::fixture(&memory, 6, 10);
        let cursor = Cursor::open_with(
            &source,
            KeyFilter::All,
            0,
            CursorOptions {
                memory: memory.clone(),
                working_set: memory.limit(),
                speed: 2.0,
                refresh_hz: 2.0,
            },
        )
        .unwrap();
        cursor.wait_read_ahead().unwrap();
        stop_reader(&cursor);
        let read = cursor.request.published.read();
        let pages = read.get().unwrap().pages.len();
        let shortened = cursor.stats().shortened_windows;
        let used = memory.used();
        assert!(crate::budget::Reclaim::reclaim_one(&*cursor.pool, &memory));
        assert!(memory.used() >= used);
        assert_eq!(cursor.stats().shortened_windows, shortened);
        assert_eq!(read.get().unwrap().pages.len(), pages);
        assert!(!crate::budget::Reclaim::reclaim_one(&*cursor.pool, &memory));
        assert!(memory.used() >= used);
        assert_eq!(cursor.stats().shortened_windows, shortened);
        drop(read);
        drop(cursor.request.published.collect());
        let used = memory.used();
        assert!(crate::budget::Reclaim::reclaim_one(&*cursor.pool, &memory));
        assert!(memory.used() < used);
        assert_eq!(cursor.stats().shortened_windows, shortened + 1);
    }

    #[test]
    fn a_live_pool_prunes_closed_and_expired_jobs_and_releases_registry_capacity() {
        let memory = Memory::new(4_000_000).unwrap();
        let source = crate::tests::fixture(&memory, 2, 1);
        let config = CursorOptions {
            memory: memory.clone(),
            working_set: memory.limit(),
            speed: 1.0,
            refresh_hz: 1.0,
        };
        let survivor = Cursor::open_with(&source, KeyFilter::All, 0, config.clone()).unwrap();
        survivor.wait_read_ahead().unwrap();
        let mut cursors = Vec::new();
        for _ in 0..24 {
            cursors.push(Cursor::open_with(&source, KeyFilter::All, 0, config.clone()).unwrap());
        }
        assert_eq!(survivor.pool.jobs.lock().unwrap().len(), 25);
        drop(cursors);
        let mut jobs = survivor.pool.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 1);
        assert!(jobs.capacity() <= 2);
        jobs.grow(&memory, Owner::Metadata, 2).unwrap();
        jobs.push(Job {
            request: Weak::new(),
            _metadata: memory
                .reserve(
                    Owner::Metadata,
                    size_of::<Job<crate::reference::InMemory>>(),
                )
                .unwrap(),
        });
        prune_jobs(&mut jobs, &memory);
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn sparse_retained_slots_do_not_keep_the_old_address_space_resident() {
        let memory = Memory::new(1_000_000).unwrap();
        let previous =
            Slots::new(Arc::new(Selection::All { len: 100_000 }), None, &memory).unwrap();
        let mut ids = Buffer::new(&memory, Owner::State, 2).unwrap();
        ids.push(99_999);
        ids.push(100_000);
        let slots = Slots::new(Arc::new(Selection::Ids(ids)), Some(&previous), &memory).unwrap();
        assert!(slots.sparse);
        assert_eq!(slots.public(0), 99_999);
        assert_eq!(slots.public(1), 0);
        assert_eq!(slots.internal(99_999), Some(0));
        assert_eq!(slots.internal(0), Some(1));
        assert_eq!(slots.internal(1), None);
        assert_eq!(slots.internal(usize::MAX), None);
        assert!(slots.inverse.bytes() <= 4 * size_of::<usize>());
        assert_eq!(memory.owner_used(Owner::Transient), 0);
    }
}
