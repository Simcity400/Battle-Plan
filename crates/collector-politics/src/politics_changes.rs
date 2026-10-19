//! Native power mutations and government commits. Writer/caller descriptors
//! live in engine_events::politics_sites. No balance fall is turned into a spend.
//! Hooks never select countries by player/human/AI status.
use crate::{
    hook::{Hook, Registers, Tap},
    shared_lane::SharedLane,
};
use engine_allocation as allocation;
use engine_events::politics_sites as native;
use engine_native::Memory;
use replay_data::{
    rows::GovernmentCause, rows::PowerAction, rows::PowerCause, rows::PowerCurrency,
    rows::PowerDailyCostKind, rows::PowerObjectKind, rows::PowerOperation, rows::PowerOriginKind,
    rows::PowerSite,
};
use std::{
    cell::{Cell, UnsafeCell},
    io,
    ops::{Deref, Range},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

static BASE: AtomicUsize = AtomicUsize::new(0);
static RUNTIME_ENABLED: AtomicBool = AtomicBool::new(true);
static ENABLED: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicU64 = AtomicU64::new(1);
#[repr(align(64))]
pub struct Padded<T>(T);
impl<T> Deref for Padded<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}
pub static ACTIVE: Padded<AtomicUsize> = Padded(AtomicUsize::new(0));
static LOG_SERIAL: AtomicU64 = AtomicU64::new(1);
static LOG: AtomicUsize = AtomicUsize::new(0);
static OFFERED: AtomicUsize = AtomicUsize::new(0);
// Published pointers remain stable when the owning vector grows.
#[allow(clippy::vec_box)]
static LOGS: Mutex<Vec<Box<[Log]>>> = Mutex::new(Vec::new());
static UNHOOKED: OnceLock<String> = OnceLock::new();
// 11: thread registrations beyond the dedicated lanes, 12: refused shared appends.
static COUNTERS: [AtomicU64; 13] = [const { AtomicU64::new(0) }; 13];
const FAILURE_NAMES: [&str; 39] = [
    "ideology identity unreadable",
    "module address overflow",
    "political party address overflow",
    "political party and ideology ids differ",
    "political party identity unreadable",
    "political party ideology pointer unreadable or null",
    "political party ideology vtable mismatch",
    "political party vtable mismatch",
    "politics ability upkeep identity address overflow",
    "politics blockade cost define unreadable or overflowed",
    "politics command power balance unreadable or overflowed",
    "politics country address overflow",
    "politics country backlink mismatch",
    "politics country has null identity",
    "politics country identity unreadable",
    "politics country vtable mismatch",
    "politics daily MIO cost output unreadable",
    "politics daily cost modifier output unreadable or overflowed",
    "politics daily political cost output unreadable or overflowed",
    "politics daily relation cost output unreadable",
    "politics event clock unreadable or overflowed",
    "politics fallback ruling party pointer unreadable or overflowed",
    "politics ideology address overflow",
    "politics literal extent overflow",
    "politics module overflow",
    "politics owner pointer unreadable or null",
    "politics political power balance unreadable, absent or overflowed",
    "politics ruling party pointer unreadable or overflowed",
    "politics sealed staging successor absent",
    "politics staging absent",
    "politics staging publisher count exhausted",
    "politics status owner mismatch",
    "politics status pointer overflow",
    "politics status vtable mismatch",
    "string header invalid",
    "string header unreadable",
    "string heap pointer invalid",
    "string length overflow",
    "string terminator unreadable or invalid",
];
static FAILURES: [AtomicU64; FAILURE_NAMES.len()] =
    [const { AtomicU64::new(0) }; FAILURE_NAMES.len()];
static UNLISTED_FAILURE: OnceLock<&'static str> = OnceLock::new();
static UNLISTED_FAILURES: AtomicU64 = AtomicU64::new(0);
static REQUIRED_KEYS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Power,
    Daily,
    Cost,
    Government,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub action: PowerAction,
    pub keys: [native::Literal; 3],
    pub flags: u16,
    pub words: [u32; 6],
    pub object_kind: PowerObjectKind,
    pub reference: u64,
    pub origin_kind: PowerOriginKind,
}
impl Identity {
    fn set(&mut self, index: usize, value: Option<u32>) {
        self.flags = (self.flags & !(1 << index)) | (u16::from(value.is_some()) << index);
        self.words[index] = value.unwrap_or(0);
    }
    pub fn value(self, index: usize) -> Option<u32> {
        (self.flags & (1 << index) != 0).then_some(self.words[index])
    }
    fn set_reference(&mut self, value: Option<u64>) {
        self.flags = (self.flags & !(1 << 10)) | (u16::from(value.is_some()) << 10);
        self.reference = value.unwrap_or(0);
    }
    pub fn object_reference(self) -> Option<u64> {
        (self.flags & (1 << 10) != 0).then_some(self.reference)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_optional(words: [Option<u32>; 6], reference: Option<u64>) -> Self {
        let mut identity = Self::default();
        for (n, value) in words.into_iter().enumerate() {
            identity.set(n, value);
        }
        identity.set_reference(reference);
        identity
    }
}
/// Expanded worker observation; native detours construct only raw records.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub kind: Kind,
    pub country: u32,
    pub instance: u32,
    pub epoch: u64,
    pub sequence: u64,
    pub hour: i32,
    pub currency: PowerCurrency,
    pub operation: PowerOperation,
    pub cause: PowerCause,
    pub site: PowerSite,
    pub before: i64,
    pub after: i64,
    pub requested: Option<i64>,
    pub identity: Identity,
    pub gross: i64,
    pub cost: i64,
    pub net: i64,
    pub component: PowerDailyCostKind,
    pub government_cause: GovernmentCause,
    pub parties: [Option<u32>; 2],
    pub actor_country: Option<u32>,
}

// Compact native words only. Conversion to replay identities/rows is worker-only.
#[derive(Clone, Copy, Default)]
struct Record {
    country: u32,
    instance: u32,
    epoch: u64,
    sequence: u64,
    hour: i32,
    kind: Kind,
    currency: PowerCurrency,
    operation: PowerOperation,
    cause: PowerCause,
    site: PowerSite,
    component: PowerDailyCostKind,
    government_cause: GovernmentCause,
    action: PowerAction,
    object_kind: PowerObjectKind,
    origin_kind: PowerOriginKind,
    flags: u16,
    amounts: [i64; 3],
    words: [u32; 9],
    reference: u64,
    keys: [[u32; 2]; 3],
}
impl Record {
    #[inline(always)]
    fn base(kind: Kind, country: u32, instance: u32) -> Self {
        Self {
            kind,
            country,
            instance,
            ..Self::default()
        }
    }
    #[inline(always)]
    fn identify(&mut self, i: Identity) {
        self.action = i.action;
        self.object_kind = i.object_kind;
        self.origin_kind = i.origin_kind;
        self.flags |= i.flags;
        self.words[..6].copy_from_slice(&i.words);
        self.reference = i.reference;
    }
    #[cfg(any(test, feature = "test-support"))]
    fn pack(event: Event) -> Self {
        let mut record = Self::base(event.kind, event.country, event.instance);
        record.epoch = event.epoch;
        record.sequence = event.sequence;
        record.hour = event.hour;
        record.currency = event.currency;
        record.operation = event.operation;
        record.cause = event.cause;
        record.site = event.site;
        record.component = event.component;
        record.government_cause = event.government_cause;
        record.identify(event.identity);
        record.flags |= u16::from(event.requested.is_some()) << 9;
        for (n, value) in [event.parties[0], event.parties[1], event.actor_country]
            .into_iter()
            .enumerate()
        {
            record.flags |= u16::from(value.is_some()) << (n + 6);
            record.words[n + 6] = value.unwrap_or(0);
        }
        record.amounts = if matches!(event.kind, Kind::Daily | Kind::Cost) {
            [event.gross, event.cost, event.net]
        } else {
            [event.before, event.after, event.requested.unwrap_or(0)]
        };
        record.keys = event
            .identity
            .keys
            .map(|k| [k.address as u32, k.len as u32]);
        record
    }
    fn unpack(self) -> Event {
        let value = |n: usize| (self.flags & (1 << n) != 0).then_some(self.words[n]);
        let [a, b, c] = self.amounts;
        Event {
            country: self.country,
            instance: self.instance,
            epoch: self.epoch,
            sequence: self.sequence,
            hour: self.hour,
            kind: self.kind,
            currency: self.currency,
            operation: self.operation,
            cause: self.cause,
            site: self.site,
            component: self.component,
            government_cause: self.government_cause,
            before: if self.kind == Kind::Power { a } else { 0 },
            after: if self.kind == Kind::Power { b } else { 0 },
            requested: (self.flags & (1 << 9) != 0).then_some(c),
            gross: if matches!(self.kind, Kind::Daily | Kind::Cost) {
                a
            } else {
                0
            },
            cost: if matches!(self.kind, Kind::Daily | Kind::Cost) {
                b
            } else {
                0
            },
            net: if matches!(self.kind, Kind::Daily | Kind::Cost) {
                c
            } else {
                0
            },
            parties: [value(6), value(7)],
            actor_country: value(8),
            identity: Identity {
                action: self.action,
                object_kind: self.object_kind,
                origin_kind: self.origin_kind,
                flags: self.flags & (0x3f | 1 << 10),
                words: self.words[..6].try_into().unwrap(),
                reference: self.reference,
                keys: self.keys.map(|[address, len]| native::Literal {
                    address: address as usize,
                    len: len as usize,
                }),
            },
        }
    }
}

// Single-producer lanes. Publication releases a complete record; the worker
// releases the consumed index only after copying it. Overflow never overwrites.
struct Lane {
    slots: Box<[UnsafeCell<Record>]>,
    bytes: Box<[UnsafeCell<u8>]>,
    write: UnsafeCell<u64>,
    published: AtomicU64,
    consumed: AtomicU64,
    rejected: AtomicU64,
    consumed_rejected: AtomicU64,
    required_entries: AtomicUsize,
}
unsafe impl Sync for Lane {}
impl Lane {
    fn new(entries: usize, keys: usize) -> io::Result<Self> {
        let entries = entries
            .max(1)
            .checked_next_power_of_two()
            .ok_or_else(allocation::oom)?;
        let bytes = allocation::mul(entries, keys)?;
        Ok(Self {
            slots: allocation::boxed_slice(allocation::default_vec(entries)?)?,
            bytes: allocation::boxed_slice(allocation::default_vec(bytes)?)?,
            write: UnsafeCell::new(0),
            published: AtomicU64::new(0),
            consumed: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            consumed_rejected: AtomicU64::new(0),
            required_entries: AtomicUsize::new(0),
        })
    }
    fn byte_pointer(&self, index: usize, keys: usize) -> *mut u8 {
        unsafe {
            self.bytes
                .as_ptr()
                .add(index * keys)
                .cast::<u8>()
                .cast_mut()
        }
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
    key_stride: usize,
    // Observed power writes are interval facts. A closed
    // generation has no publishers; stale callback pointers follow
    // next. Cursor/consumed stay local, with a stable native sequence offset.
    publishers: AtomicUsize,
    next: AtomicUsize,
    base_cursor: AtomicU64,
    cursor: Padded<AtomicU64>,
    consumed: AtomicU64,
    consumed_epoch: AtomicU64,
}
unsafe impl Sync for Log {}
impl Log {
    #[cfg(any(test, feature = "test-support"))]
    fn new(entries: usize, keys: usize) -> Self {
        Self::with_lanes(entries, keys, 1)
    }
    #[cfg(any(test, feature = "test-support"))]
    fn with_lanes(entries: usize, keys: usize, lanes: usize) -> Self {
        Self::with_lane_requirements(entries, keys, lanes, &[]).unwrap()
    }
    #[cfg(any(test, feature = "test-support"))]
    fn with_lane_requirements(
        entries: usize,
        keys: usize,
        lanes: usize,
        required: &[usize],
    ) -> io::Result<Self> {
        Self::with_shared_requirement(entries, keys, lanes, required, 0)
    }
    fn with_shared_requirement(
        entries: usize,
        keys: usize,
        lanes: usize,
        required: &[usize],
        shared: usize,
    ) -> io::Result<Self> {
        let entries = entries.max(1);
        let keys = keys.max(1);
        // Sized like one more thread's lane, then from observed shared demand.
        let shared = SharedLane::new(entries.div_ceil(lanes.max(1)).max(shared), keys)?;
        let mut storage = allocation::vec(lanes)?;
        for n in 0..lanes {
            storage.push(Lane::new(
                (if n == 0 {
                    entries
                } else {
                    entries.div_ceil(lanes)
                })
                .max(required.get(n).copied().unwrap_or(0)),
                keys,
            )?);
        }
        Ok(Self {
            serial: LOG_SERIAL.fetch_add(1, Ordering::Relaxed),
            lanes: allocation::boxed_slice(storage)?,
            shared,
            registered: AtomicUsize::new(0),
            entries,
            key_stride: keys,
            publishers: AtomicUsize::new(0),
            next: AtomicUsize::new(0),
            base_cursor: AtomicU64::new(0),
            cursor: Padded(AtomicU64::new(0)),
            consumed: AtomicU64::new(0),
            consumed_epoch: AtomicU64::new(EPOCH.load(Ordering::Acquire)),
        })
    }
    fn frontier(&self) -> u64 {
        self.base_cursor.load(Ordering::Acquire) + self.cursor.load(Ordering::Acquire)
    }
    fn consumed_frontier(&self) -> u64 {
        self.base_cursor.load(Ordering::Acquire) + self.consumed.load(Ordering::Acquire)
    }
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
                    failed("politics sealed staging successor absent");
                    return None;
                }
                // Closure follows all old publications. Helpers and the
                // boundary store the same immutable offset, never a cursor.
                let next = unsafe { &*next };
                next.base_cursor.store(log.frontier(), Ordering::Release);
                log = next;
            }
            Err(_) => {
                failed("politics staging publisher count exhausted");
                return None;
            }
        }
    }
}
static REQUIRED_LANES: AtomicUsize = AtomicUsize::new(1);
pub fn thread_slots() -> usize {
    std::thread::available_parallelism()
        .map_or(1, usize::from)
        .max(REQUIRED_LANES.load(Ordering::Relaxed))
}
thread_local! { static LANE: Cell<(usize,u64,usize,usize)> = const {Cell::new((0,0,0,0))}; }
fn lane_index(log: &Log) -> Option<usize> {
    let pointer = log as *const Log as usize;
    LANE.with(|cached| {
        let (old, serial, index, _) = cached.get();
        if old == pointer && serial == log.serial {
            return (index < log.lanes.len()).then_some(index);
        }
        let index = log.registered.fetch_add(1, Ordering::Relaxed);
        cached.set((
            pointer,
            log.serial,
            index,
            log.lanes
                .get(index)
                .map_or(0, |lane| lane as *const Lane as usize),
        ));
        if index >= log.lanes.len() {
            // This thread publishes through the shared lane; growth offers
            // a dedicated lane to the next generation.
            REQUIRED_LANES.fetch_max(index + 1, Ordering::Relaxed);
            COUNTERS[11].fetch_add(1, Ordering::Relaxed);
            None
        } else {
            Some(index)
        }
    })
}
#[inline(always)]
fn cached_lane(log: &Log) -> Option<&Lane> {
    LANE.with(|cached| {
        let (pointer, serial, _, lane) = cached.get();
        if pointer == log as *const Log as usize && serial == log.serial && lane != 0 {
            // Logs/lanes stay allocated until hooks and all observers quiesce.
            return Some(unsafe { &*(lane as *const Lane) });
        }
        lane_index(log).map(|index| &log.lanes[index])
    })
}

struct Call;
impl Call {
    #[inline(always)]
    fn enter() -> Self {
        DEPTH.with(|depth| {
            if depth.get() == 0 {
                ACTIVE.fetch_add(1, Ordering::SeqCst);
            }
            depth.set(depth.get() + 1);
        });
        Self
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        DEPTH.with(|depth| {
            depth.set(depth.get() - 1);
            if depth.get() == 0 {
                ACTIVE.fetch_sub(1, Ordering::SeqCst);
            }
        });
    }
}
/// Before publishing any staging or enabling observers in a new DLL session.
pub fn restore_epoch(previous: u64) -> Result<(), &'static str> {
    let epoch = previous
        .checked_add(1)
        .ok_or("politics observation epoch exhausted")?;
    EPOCH.store(epoch, Ordering::Release);
    Ok(())
}

pub fn invalidate() {
    #[cfg(any(test, feature = "test-support"))]
    if !STAGING_OWNER.get() {
        return;
    }
    crate::event_feed_observer::decisions::political_balance_lost();
    EPOCH.fetch_add(1, Ordering::SeqCst);
}
fn failed(reason: &'static str) {
    if let Some(index) = FAILURE_NAMES.iter().position(|name| *name == reason) {
        FAILURES[index].fetch_add(1, Ordering::Relaxed);
    } else {
        let _ = UNLISTED_FAILURE.set(reason);
        UNLISTED_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    COUNTERS[3].fetch_add(1, Ordering::Relaxed);
    invalidate();
}
fn memory() -> engine_native::memory::LiveMemory {
    unsafe { engine_native::memory::LiveMemory::new() }
}
fn base() -> usize {
    BASE.load(Ordering::Relaxed)
}
fn read<T: engine_native::memory::Raw>(address: usize) -> Option<T> {
    engine_native::read::<T>(&memory(), address)
}
fn operand_memory() -> native::NativeOperands {
    // SAFETY: callers below read only the supported synchronous native operands.
    unsafe { native::NativeOperands::new() }
}
#[inline(always)]
fn operand<T: engine_native::memory::Raw>(address: usize) -> Option<T> {
    engine_native::read::<T>(&operand_memory(), address)
}
fn operand_hour() -> Option<i32> {
    let state =
        operand::<u64>(base().checked_add(engine_native::cgame_state::SINGLETON_RVA)?)? as usize;
    operand::<i32>(state.checked_add(engine_native::cgame_state::CURRENT_HOUR)?)
}
fn event_hour() -> Option<i32> {
    let state =
        read::<u64>(base().checked_add(engine_native::cgame_state::SINGLETON_RVA)?)? as usize;
    read::<i32>(state.checked_add(engine_native::cgame_state::CURRENT_HOUR)?)
}
fn log() -> Option<&'static Log> {
    let pointer = LOG.load(Ordering::Acquire) as *const Log;
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { &*pointer })
    }
}
/// Off-thread only. Larger staging is offered to an hourly handoff; old
/// generations remain alive until callbacks and capture workers quiesce.
pub fn resize(entries: usize, keys: usize) {
    engine_allocation::advisory(|| resize_storage(entries, keys));
}

fn resize_storage(entries: usize, keys: usize) {
    if resize_checked(entries, keys).is_err() {
        // Checked helpers stop admission and write the preopened terminal witness.
        // Keep all previously published storage alive until observer quiescence.
        COUNTERS[10].fetch_add(1, Ordering::Relaxed);
        invalidate();
    }
}
fn resize_checked(entries: usize, keys: usize) -> io::Result<()> {
    let mut logs = LOGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let offered = OFFERED.load(Ordering::Acquire) as *const Log;
    let offered = (!offered.is_null()).then(|| unsafe { &*offered });
    // The boundary clears an offer only after publishing it as current.
    let current = log();
    let sources = [current, offered];
    // Match lane rounding so small roster changes do not retire identical rings.
    let entries = entries
        .max(
            sources
                .into_iter()
                .flatten()
                .map(|log| log.entries)
                .max()
                .unwrap_or(0),
        )
        .max(1)
        .checked_next_power_of_two()
        .ok_or_else(allocation::oom)?;
    let keys = keys.max(REQUIRED_KEYS.load(Ordering::Relaxed)).max(
        sources
            .into_iter()
            .flatten()
            .map(|log| log.key_stride)
            .max()
            .unwrap_or(0),
    );
    let lanes = thread_slots().max(
        sources
            .into_iter()
            .flatten()
            .map(|log| log.lanes.len())
            .max()
            .unwrap_or(0),
    );
    let required = |n: usize| {
        sources
            .into_iter()
            .flatten()
            .filter_map(|log| log.lanes.get(n))
            .map(|lane| {
                lane.required_entries
                    .load(Ordering::Relaxed)
                    .max(lane.slots.len())
            })
            .max()
            .unwrap_or(0)
    };
    let shared = sources
        .into_iter()
        .flatten()
        .map(|log| log.shared.required_entries().max(log.shared.len()))
        .max()
        .unwrap_or(0);
    if sources.into_iter().flatten().any(|log| {
        log.entries >= entries
            && log.key_stride >= keys
            && log.lanes.len() >= lanes
            && log.shared.len() >= shared
            && log
                .lanes
                .iter()
                .enumerate()
                .all(|(n, lane)| lane.slots.len() >= required(n))
    }) {
        return Ok(());
    }
    let required = allocation::collect_vec((0..lanes).map(required))?;
    let fresh = Log::with_shared_requirement(entries, keys, lanes, &required, shared)?;
    let fresh = allocation::boxed_slice(allocation::collect_vec([fresh])?)?;
    let pointer = fresh.as_ptr() as usize;
    allocation::reserve_push_batch(&mut logs, 1)?;
    if allocation::capture_stopped() {
        return Err(allocation::oom());
    }
    logs.push(fresh);
    if current.is_none() {
        LOG.store(pointer, Ordering::Release);
    } else {
        OFFERED.store(pointer, Ordering::Release);
    }
    Ok(())
}

/// Off-thread retry from observed lane/key pressure, independent of registry
/// growth. Existing logs remain alive for admitted readers and callbacks.
pub fn prepare() {
    if let Some(log) = log() {
        resize(log.entries, log.key_stride);
    }
}
/// Release staging after disabled hooks and all callback/capture workers are
/// quiescent. No published log reference may survive this call.
///
/// # Safety
/// The caller has disabled hooks, checked their executable ranges and ACTIVE,
/// and joined all workers that can call drain/resize/status.
pub unsafe fn release() {
    if ENABLED.load(Ordering::SeqCst) || ACTIVE.load(Ordering::SeqCst) != 0 {
        COUNTERS[8].fetch_add(1, Ordering::Relaxed);
        return; // Retain every published log; never panic or free live storage.
    }
    LOG.store(0, Ordering::Release);
    OFFERED.store(0, Ordering::Release);
    LOGS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    CONSERVATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .examples
        .fill(None);
}
#[inline(always)]
#[allow(clippy::too_many_arguments)] // Fixed native writer operands, no replay row.
fn power_record(
    owner: native::Owner,
    currency: PowerCurrency,
    operation: PowerOperation,
    cause: PowerCause,
    site: PowerSite,
    before: i64,
    after: i64,
    requested: Option<i64>,
    identity: Identity,
) -> Record {
    if currency == PowerCurrency::Political && before != after {
        crate::event_feed_observer::decisions::political_balance(
            owner.pointer,
            owner.country,
            owner.instance,
            before,
            after,
        );
    }
    let mut raw = Record::base(Kind::Power, owner.country, owner.instance);
    raw.currency = currency;
    raw.operation = operation;
    raw.cause = cause;
    raw.site = site;
    raw.amounts = [before, after, requested.unwrap_or(0)];
    raw.flags |= u16::from(requested.is_some()) << 9;
    raw.identify(identity);
    raw
}
#[inline(always)]
#[allow(clippy::too_many_arguments)] // Fixed native writer operands.
fn power(
    owner: native::Owner,
    currency: PowerCurrency,
    operation: PowerOperation,
    cause: PowerCause,
    site: PowerSite,
    before: i64,
    after: i64,
    requested: Option<i64>,
    identity: Identity,
) {
    append_record(
        power_record(
            owner, currency, operation, cause, site, before, after, requested, identity,
        ),
        identity.keys,
    );
}
#[inline(always)]
fn cost_record(
    owner: native::Owner,
    kind: PowerDailyCostKind,
    cost: i64,
    cause: PowerCause,
    site: PowerSite,
    identity: Identity,
) -> Record {
    let mut raw = Record::base(Kind::Cost, owner.country, owner.instance);
    raw.amounts[1] = cost;
    raw.component = kind;
    raw.cause = cause;
    raw.site = site;
    raw.identify(identity);
    raw
}
#[inline(always)]
fn daily_record(owner: native::Owner, gross: i64, cost: i64, net: i64) -> Record {
    let mut raw = Record::base(Kind::Daily, owner.country, owner.instance);
    raw.amounts = [gross, cost, net];
    raw
}
#[cfg(any(test, feature = "test-support"))]
fn fixture_record(event: &Event) -> Record {
    let owner = native::Owner {
        country: event.country,
        instance: event.instance,
        pointer: 0,
        politics: None,
    };
    let mut raw = match event.kind {
        Kind::Power => power_record(
            owner,
            event.currency,
            event.operation,
            event.cause,
            event.site,
            event.before,
            event.after,
            event.requested,
            event.identity,
        ),
        Kind::Cost => cost_record(
            owner,
            event.component,
            event.cost,
            event.cause,
            event.site,
            event.identity,
        ),
        Kind::Daily => daily_record(owner, event.gross, event.cost, event.net),
        Kind::Government => Record::pack(*event),
    };
    raw.epoch = event.epoch;
    raw.sequence = event.sequence;
    raw.hour = event.hour;
    raw
}
#[cfg(any(test, feature = "test-support"))]
fn append_to(log: &Log, event: Event, memory: &impl Memory) {
    append_raw(log, fixture_record(&event), event.identity.keys, memory);
}
#[cfg(test)]
fn append_lane(log: &Log, index: usize, event: Event, memory: &impl Memory) {
    append_raw_lane(
        log,
        index,
        fixture_record(&event),
        event.identity.keys,
        memory,
    );
}
#[inline(always)]
fn append_record(record: Record, keys: [native::Literal; 3]) {
    append_record_using(record, keys, &memory(), false);
}
#[inline(always)]
fn append_operands(record: Record, keys: [native::Literal; 3]) {
    append_record_using(record, keys, &operand_memory(), true);
}
#[inline(always)]
fn append_record_using(
    mut record: Record,
    keys: [native::Literal; 3],
    memory: &impl Memory,
    native_clock: bool,
) {
    if !ENABLED.load(Ordering::Acquire) {
        return;
    }
    let Some(log) = log() else {
        failed("politics staging absent");
        return;
    };
    let Some(hour) = DAILY_HOUR.get().or_else(|| {
        if native_clock {
            operand_hour()
        } else {
            event_hour()
        }
    }) else {
        failed("politics event clock unreadable or overflowed");
        return;
    };
    record.hour = hour;
    record.epoch = EPOCH.load(Ordering::Acquire);
    append_raw(log, record, keys, memory);
}
#[inline(always)]
fn append_raw(log: &Log, record: Record, keys: [native::Literal; 3], memory: &impl Memory) {
    let Some(publishing) = publishing(log) else {
        return;
    };
    let log = publishing.0;
    match cached_lane(log) {
        Some(lane) => append_raw_to_lane(log, lane, record, keys, memory),
        None => append_raw_shared(log, record, keys, memory),
    }
}
/// A thread without a dedicated lane in this generation. The record takes
/// its log-wide sequence exactly as a lane record does.
fn append_raw_shared(
    log: &Log,
    mut record: Record,
    mut keys: [native::Literal; 3],
    memory: &impl Memory,
) {
    let Some(mut slot) = log.shared.reserve() else {
        // Refused demand is retained for the next generation's shared lane.
        COUNTERS[1].fetch_add(1, Ordering::Relaxed);
        COUNTERS[12].fetch_add(1, Ordering::Relaxed);
        invalidate();
        return;
    };
    if copy_keys(&mut keys, slot.stride(), slot.bytes(), memory).is_err() {
        return; // The dropped reservation publishes void.
    }
    record.sequence =
        log.base_cursor.load(Ordering::Acquire) + log.cursor.fetch_add(1, Ordering::Relaxed) + 1;
    record.keys = keys.map(|key| [key.address as u32, key.len as u32]);
    slot.publish(record);
}
/// Copies each literal into this record's key span, rebasing it there.
/// A literal beyond the stride is dropped and its demand retained.
#[inline(always)]
fn copy_keys(
    keys: &mut [native::Literal; 3],
    stride: usize,
    destination: *mut u8,
    memory: &impl Memory,
) -> Result<(), ()> {
    let mut used = 0usize;
    for key in keys.iter_mut().filter(|key| key.len != 0) {
        let length = key.len;
        let Some(end) = used.checked_add(length) else {
            failed("politics literal extent overflow");
            return Err(());
        };
        if end > stride || end > u32::MAX as usize {
            REQUIRED_KEYS.fetch_max(end, Ordering::Relaxed);
            COUNTERS[4].fetch_add(1, Ordering::Relaxed);
            *key = native::Literal::default();
            continue;
        }
        if length != 0 {
            let output = unsafe { std::slice::from_raw_parts_mut(destination.add(used), length) };
            if key.copy(memory, output).is_err() {
                COUNTERS[4].fetch_add(1, Ordering::Relaxed);
                *key = native::Literal::default();
                continue;
            }
        }
        *key = native::Literal {
            address: used,
            len: length,
        };
        used = end;
    }
    Ok(())
}
#[cfg(any(test, feature = "test-support"))]
#[inline(always)]
fn append_raw_lane(
    log: &Log,
    lane_index: usize,
    record: Record,
    keys: [native::Literal; 3],
    memory: &impl Memory,
) {
    append_raw_to_lane(log, &log.lanes[lane_index], record, keys, memory);
}
#[inline(always)]
fn append_raw_to_lane(
    log: &Log,
    lane: &Lane,
    mut record: Record,
    mut keys: [native::Literal; 3],
    memory: &impl Memory,
) {
    // Only the thread that registered this lane accesses write.
    let cursor = unsafe { *lane.write.get() };
    let demand = cursor
        .wrapping_sub(lane.consumed.load(Ordering::Acquire))
        .saturating_add(
            lane.rejected
                .load(Ordering::Relaxed)
                .wrapping_sub(lane.consumed_rejected.load(Ordering::Acquire)),
        )
        .saturating_add(1);
    if demand > lane.slots.len() as u64 {
        // Rejected records still contribute to this sealed interval's demand.
        lane.rejected.fetch_add(1, Ordering::Release);
        lane.required_entries.fetch_max(
            (demand as usize).max(lane.slots.len().saturating_mul(2)),
            Ordering::Relaxed,
        );
        COUNTERS[1].fetch_add(1, Ordering::Relaxed);
        invalidate();
        return;
    }
    let index = cursor as usize & (lane.slots.len() - 1);
    if copy_keys(
        &mut keys,
        log.key_stride,
        lane.byte_pointer(index, log.key_stride),
        memory,
    )
    .is_err()
    {
        return;
    }
    record.sequence =
        log.base_cursor.load(Ordering::Acquire) + log.cursor.fetch_add(1, Ordering::Relaxed) + 1;
    record.keys = keys.map(|key| [key.address as u32, key.len as u32]);
    unsafe {
        *lane.slots[index].get() = record;
        *lane.write.get() = cursor + 1;
    }
    lane.published.store(cursor + 1, Ordering::Release);
}
#[derive(Clone, Copy, Default)]
pub struct LaneCursor {
    cursor: u64,
    rejected: u64,
}

#[derive(Clone, Copy)]
pub struct Handoff {
    log: usize,
    epoch: u64,
    cursor: u64,
    lanes: usize,
    shared_rejected: u64,
}
/// Exchange the interval's upper ring index, without copying
/// events, scanning countries, resolving identities or touching replay rows.
#[cfg(any(test, feature = "test-support"))]
pub fn handoff(cursors: &mut [LaneCursor]) -> Result<Handoff, &'static str> {
    handoff_buffer(engine_native::walk::Buffer::Fixed(cursors))
}
pub fn handoff_growing(cursors: &mut Vec<LaneCursor>) -> Result<Handoff, &'static str> {
    handoff_buffer(engine_native::walk::Buffer::Growing(cursors))
}
fn handoff_buffer(
    mut cursors: engine_native::walk::Buffer<'_, LaneCursor>,
) -> Result<Handoff, &'static str> {
    if UNHOOKED.get().is_some() || !ENABLED.load(Ordering::Acquire) {
        return Err("politics hooks unavailable");
    }
    let log = log().ok_or("politics staging unavailable")?;
    // A publisher lease closes the seal/publication race, independently of
    // native calls beginning after ACTIVE's quiet observation. Their emitters
    // follow the prepared successor instead of waiting or losing records.
    if ACTIVE.load(Ordering::SeqCst) != 0 {
        return handoff_from_buffer(log, cursors);
    }
    if !cursors.ensure(log.lanes.len())? {
        return handoff_from_buffer(log, cursors);
    }
    let offered = OFFERED.load(Ordering::Acquire) as *const Log;
    if offered.is_null() {
        return handoff_from_buffer(log, cursors);
    }
    let offered = unsafe { &*offered };
    if !cursors.ensure(offered.lanes.len())? {
        return handoff_from_buffer(log, cursors);
    }
    if !close_for_handoff(log, offered) {
        return handoff_from_buffer(log, cursors);
    }
    offered.base_cursor.store(log.frontier(), Ordering::Release);
    let sealed = handoff_from_buffer(log, cursors);
    LOG.store(offered as *const Log as usize, Ordering::Release);
    let _ = OFFERED.compare_exchange(
        offered as *const Log as usize,
        0,
        Ordering::AcqRel,
        Ordering::Relaxed,
    );
    sealed
}
fn close_for_handoff(log: &Log, offered: &Log) -> bool {
    offered
        .consumed_epoch
        .store(EPOCH.load(Ordering::Acquire), Ordering::Relaxed);
    log.next
        .store(offered as *const Log as usize, Ordering::Release);
    log.publishers
        .compare_exchange(0, usize::MAX, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
}
#[cfg(any(test, feature = "test-support"))]
fn handoff_from(log: &Log, cursors: &mut [LaneCursor]) -> Result<Handoff, &'static str> {
    handoff_from_buffer(log, engine_native::walk::Buffer::Fixed(cursors))
}
fn handoff_from_buffer(
    log: &Log,
    mut cursors: engine_native::walk::Buffer<'_, LaneCursor>,
) -> Result<Handoff, &'static str> {
    let count = log.registered.load(Ordering::Acquire).min(log.lanes.len());
    if !cursors.ensure(count)? {
        short_handoff();
        return Err("politics thread handoff staging short");
    }
    for (out, lane) in cursors.iter_mut().zip(&log.lanes[..count]) {
        out.cursor = lane.published.load(Ordering::Acquire);
        out.rejected = lane.rejected.load(Ordering::Acquire);
    }
    let shared_rejected = log.shared.rejected();
    Ok(Handoff {
        log: log as *const Log as usize,
        epoch: EPOCH.load(Ordering::Acquire),
        cursor: log.frontier(),
        lanes: count,
        shared_rejected,
    })
}
impl Handoff {
    pub fn pending(self) -> usize {
        let log = unsafe { &*(self.log as *const Log) };
        self.cursor.wrapping_sub(log.consumed_frontier()) as usize
    }
    pub fn key_capacity(self) -> usize {
        let log = unsafe { &*(self.log as *const Log) };
        self.pending().saturating_mul(log.key_stride)
    }
    pub fn drain(
        self,
        cursors: &[LaneCursor],
        output: &mut [Event],
        bytes: &mut [u8],
    ) -> Result<Drained, &'static str> {
        let log = unsafe { &*(self.log as *const Log) };
        drain_until(
            log,
            Bound {
                epoch: self.epoch,
                cursor: self.cursor,
                shared_rejected: self.shared_rejected,
            },
            &cursors[..self.lanes],
            output,
            bytes,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drained {
    pub epoch: u64,
    pub first: u64,
    pub cursor: u64,
    pub copied: usize,
    pub bytes: usize,
    pub pending: usize,
    pub keys_required: usize,
}
#[cfg(any(test, feature = "test-support"))]
fn drain_from(log: &Log, output: &mut [Event], bytes: &mut [u8]) -> Result<Drained, &'static str> {
    let cursors: Vec<_> = log
        .lanes
        .iter()
        .map(|l| LaneCursor {
            cursor: l.published.load(Ordering::Acquire),
            rejected: l.rejected.load(Ordering::Acquire),
        })
        .collect();
    drain_until(
        log,
        Bound {
            epoch: EPOCH.load(Ordering::Acquire),
            cursor: log.frontier(),
            shared_rejected: log.shared.rejected(),
        },
        &cursors,
        output,
        bytes,
    )
}
/// The sealed interval: its epoch, log-wide frontier and shared refusals.
#[derive(Clone, Copy)]
struct Bound {
    epoch: u64,
    cursor: u64,
    shared_rejected: u64,
}
fn drain_until(
    log: &Log,
    bound: Bound,
    cursors: &[LaneCursor],
    output: &mut [Event],
    bytes: &mut [u8],
) -> Result<Drained, &'static str> {
    let Bound { epoch, cursor, .. } = bound;
    let first = log.consumed_frontier();
    // Shared records in log-wide order. One at or below the consumed
    // frontier belongs to a withdrawn interval; one past the sealed frontier
    // belongs to the next interval.
    let shared = log
        .shared
        .published(|record| record.sequence)
        .map_err(|_| "politics drain lane storage allocation failed")?;
    let release_through = |through: u64| {
        for &(sequence, index) in &shared {
            if sequence <= through {
                log.shared.release(index);
            }
        }
    };
    if log.consumed_epoch.swap(epoch, Ordering::AcqRel) != epoch
        || epoch != EPOCH.load(Ordering::Acquire)
    {
        for (lane, end) in log.lanes.iter().zip(cursors) {
            lane.consumed.store(end.cursor, Ordering::Release);
            lane.consumed_rejected
                .store(end.rejected, Ordering::Release);
        }
        log.consumed.store(
            cursor - log.base_cursor.load(Ordering::Acquire),
            Ordering::Release,
        );
        release_through(cursor);
        log.shared.advance(bound.shared_rejected);
        return Err("politics observation restarted");
    }
    release_through(first);
    let mut shared = shared
        .iter()
        .copied()
        .filter(|&(sequence, _)| sequence > first && sequence <= cursor)
        .peekable();
    let pending = cursor.wrapping_sub(first) as usize;
    let mut copied = 0;
    let mut used = 0usize;
    let mut keys_required = 0;
    let mut tails =
        allocation::map_slice(&log.lanes[..log.lanes.len().min(cursors.len())], |lane| {
            lane.consumed.load(Ordering::Acquire)
        })
        .map_err(|_| "politics drain lane storage allocation failed")?;
    // Worker-only merge by global sequence preserves native cross-thread order.
    while copied < output.len() {
        let next = log
            .lanes
            .iter()
            .zip(cursors)
            .enumerate()
            .filter(|(n, (_, end))| tails[*n] < end.cursor)
            .map(|(n, (lane, _))| {
                let slot = tails[n] as usize & (lane.slots.len() - 1);
                (Some(n), unsafe { (*lane.slots[slot].get()).sequence })
            })
            .chain(shared.peek().map(|&(sequence, _)| (None, sequence)))
            .min_by_key(|(_, seq)| *seq);
        let Some((n, sequence)) = next else {
            break;
        };
        // A concurrently reserved but not yet published earlier sequence
        // stays pending. Never emit a later record ahead of that hole.
        if sequence != first + copied as u64 + 1 {
            break;
        }
        let (record, source) = match n {
            Some(n) => {
                let lane = &log.lanes[n];
                let index = tails[n] as usize & (lane.slots.len() - 1);
                (unsafe { *lane.slots[index].get() }, unsafe {
                    std::slice::from_raw_parts(
                        lane.byte_pointer(index, log.key_stride).cast_const(),
                        log.key_stride,
                    )
                })
            }
            None => log.shared.record(shared.peek().unwrap().1),
        };
        let mut event = record.unpack();
        let count = event
            .identity
            .keys
            .iter()
            .map(|k| k.address + k.len)
            .max()
            .unwrap_or(0);
        keys_required = used.saturating_add(count);
        if keys_required > bytes.len() {
            break;
        }
        bytes[used..keys_required].copy_from_slice(&source[..count]);
        for key in &mut event.identity.keys {
            key.address += used;
        }
        output[copied] = event;
        used += count;
        copied += 1;
        match n {
            Some(n) => tails[n] += 1,
            None => log.shared.release(shared.next().unwrap().1),
        }
    }
    log.shared.advance(bound.shared_rejected);
    for ((lane, tail), end) in log.lanes.iter().zip(tails).zip(cursors) {
        lane.consumed.store(tail, Ordering::Release);
        if tail == end.cursor {
            lane.consumed_rejected
                .store(end.rejected, Ordering::Release);
        }
    }
    log.consumed.store(
        first + copied as u64 - log.base_cursor.load(Ordering::Acquire),
        Ordering::Release,
    );
    if epoch != EPOCH.load(Ordering::Acquire) {
        return Err("politics observation epoch changed during handoff");
    }
    Ok(Drained {
        epoch,
        first,
        cursor,
        copied,
        bytes: used,
        pending,
        keys_required,
    })
}

#[derive(Clone, Copy)]
struct Pending {
    owner: native::Owner,
    currency: PowerCurrency,
    before: i64,
    requested: Option<i64>,
    cause: PowerCause,
    site: PowerSite,
    operation: PowerOperation,
    identity: Identity,
    emitted: bool,
}
#[derive(Default, Clone, Copy)]
struct Context {
    identity: Identity,
    government: GovernmentCause,
}
// Original native calls can unwind through a shim into a native handler.
// Restore thread-local provenance and withdraw the incomplete interval there.
struct ScopeRestore {
    context: Context,
    daily: Option<native::Owner>,
    hour: Option<i32>,
    completed: bool,
}
impl ScopeRestore {
    fn new() -> Self {
        Self {
            context: CONTEXT.get(),
            daily: DAILY.get(),
            hour: DAILY_HOUR.get(),
            completed: false,
        }
    }
}
impl Drop for ScopeRestore {
    fn drop(&mut self) {
        CONTEXT.set(self.context);
        DAILY.set(self.daily);
        DAILY_HOUR.set(self.hour);
        if !self.completed {
            PENDING.set(None);
            COUNTERS[9].fetch_add(1, Ordering::Relaxed);
            invalidate();
        }
    }
}
thread_local! {
    static DEPTH: Cell<usize> = const { Cell::new(0) };
    static DAILY_HOUR: Cell<Option<i32>> = const { Cell::new(None) };
    static PENDING: Cell<Option<Pending>> = const { Cell::new(None) };
    static DAILY: Cell<Option<native::Owner>> = const { Cell::new(None) };
    static CONTEXT: Cell<Context> = const { Cell::new(Context { identity: Identity { action: PowerAction::None, keys: [native::Literal { address: 0, len: 0 }; 3], flags:0, words:[0;6], object_kind:PowerObjectKind::None, reference:0, origin_kind:PowerOriginKind::None }, government: GovernmentCause::NotIdentified }) };
}
fn observed_owner(value: Result<native::Owner, &'static str>) -> Option<native::Owner> {
    match value {
        Ok(owner) => Some(owner),
        Err(reason) => {
            failed(reason);
            None
        }
    }
}
fn balance(owner: native::Owner, currency: PowerCurrency) -> Option<i64> {
    observed_balance(&memory(), owner, currency)
}
/// Political and command power writers consume these exact stored balances.
#[inline(always)]
fn operand_balance(owner: native::Owner, currency: PowerCurrency) -> Option<i64> {
    observed_balance(&operand_memory(), owner, currency)
}
fn observed_balance(
    memory: &impl Memory,
    owner: native::Owner,
    currency: PowerCurrency,
) -> Option<i64> {
    let (address, reason) = match currency {
        PowerCurrency::Political => (
            owner.politics.and_then(|pointer| {
                pointer.checked_add(engine_native::cpolitical_status::POLITICAL_POWER)
            }),
            "politics political power balance unreadable, absent or overflowed",
        ),
        PowerCurrency::Command => (
            owner
                .pointer
                .checked_add(engine_native::ccountry::COMMAND_POWER),
            "politics command power balance unreadable or overflowed",
        ),
    };
    match address.and_then(|address| engine_native::read(memory, address)) {
        Some(value) => Some(value),
        None => {
            failed(reason);
            None
        }
    }
}

/// Flush a parent's own commits before entering a child writer/signal. After
/// the child, the parent's baseline resumes at the child's resulting balance.
fn flush(force: bool) {
    let Some(mut pending) = PENDING.get() else {
        return;
    };
    let Some(after) = (if pending.cause == PowerCause::DailyGain {
        operand_balance(pending.owner, pending.currency)
    } else {
        balance(pending.owner, pending.currency)
    }) else {
        return;
    };
    if after != pending.before || force && !pending.emitted {
        let record = power_record(
            pending.owner,
            pending.currency,
            pending.operation,
            pending.cause,
            pending.site,
            pending.before,
            after,
            pending.requested,
            pending.identity,
        );
        append_record_using(
            record,
            pending.identity.keys,
            &memory(),
            pending.cause == PowerCause::DailyGain,
        );
        pending.before = after;
        pending.requested = None;
        pending.emitted = true;
        if matches!(
            pending.site,
            PowerSite::AllocateCommandPower | PowerSite::ReleaseCommandPower
        ) {
            pending.cause = PowerCause::CapCorrection;
            pending.operation = PowerOperation::AllocationClamp;
            pending.identity = Identity::default();
        }
        PENDING.set(Some(pending));
    }
}
fn resume_pending(owner: native::Owner, currency: PowerCurrency, after: i64) {
    if let Some(mut pending) = PENDING.get()
        && pending.owner == owner
        && pending.currency == currency
    {
        pending.before = after;
        PENDING.set(Some(pending));
    }
}
fn caller_cause(caller: usize) -> PowerCause {
    if caller == native::DAILY_PP_CALLER || caller == native::DAILY_CP_CALLER {
        return PowerCause::DailyGain;
    }
    if caller == native::TIMED_PP_CALLER {
        return PowerCause::TimedActivity;
    }
    let name = native::CHARGING_CALLERS
        .binary_search_by_key(&caller, |(address, _)| *address)
        .ok()
        .map(|index| native::CHARGING_CALLERS[index].1);
    match name {
        Some("DailyGain") => PowerCause::DailyGain,
        Some("Idea") => PowerCause::Idea,
        Some("Advisor") => PowerCause::Advisor,
        Some("AdvisorRemoval") => PowerCause::AdvisorRemoval,
        Some("Decision") => PowerCause::Decision,
        Some("TargetedDecision") => PowerCause::TargetedDecision,
        Some("DecisionTimeout") => PowerCause::DecisionTimeout,
        Some("Diplomacy") => PowerCause::Diplomacy,
        Some("WarJustification") => PowerCause::WarJustification,
        Some("JoinWar") => PowerCause::JoinWar,
        Some("Focus") => PowerCause::Focus,
        Some("FocusRefund") => PowerCause::FocusRefund,
        Some("ScriptEffect") => PowerCause::ScriptEffect,
        Some("ScaledScriptEffect") => PowerCause::ScaledScriptEffect,
        Some("Autonomy") => PowerCause::Autonomy,
        Some("SpyMaster") => PowerCause::SpyMaster,
        Some("ScientistRecruitment") => PowerCause::ScientistRecruitment,
        Some("ScientistFaction") => PowerCause::ScientistFaction,
        Some("ScorchedEarth") => PowerCause::ScorchedEarth,
        Some("PrideOfFleet") => PowerCause::PrideOfFleet,
        Some("Blockade") => PowerCause::Blockade,
        Some("MioPolicy") => PowerCause::MioPolicy,
        Some("Medal") => PowerCause::Medal,
        Some("Console") => PowerCause::Console,
        Some("ResetSetup") => PowerCause::ResetSetup,
        Some("AbilityActivation") => PowerCause::AbilityActivation,
        Some("AbilityUpkeep") => PowerCause::AbilityUpkeep,
        Some("Allocation") => PowerCause::Allocation,
        Some("AllocationRefund") => PowerCause::AllocationRefund,
        Some("LeaderTrait") => PowerCause::LeaderTrait,
        Some("LeaderPromotion") => PowerCause::LeaderPromotion,
        Some("LeaderAssignment") => PowerCause::LeaderAssignment,
        Some("LeaderRecruitment") => PowerCause::LeaderRecruitment,
        Some("Raid") => PowerCause::Raid,
        Some("AirAction") => PowerCause::AirAction,
        Some("PreferredTactic") => PowerCause::PreferredTactic,
        Some("OfficerUpgrade") => PowerCause::OfficerUpgrade,
        Some("CaptainUpgrade") => PowerCause::CaptainUpgrade,
        Some("TimedActivity") => PowerCause::TimedActivity,
        Some("CapCorrection") => PowerCause::CapCorrection,
        _ => PowerCause::NotIdentified,
    }
}

const ENTRY_COUNT: usize = 38;
/// [`entry_sites`] index of country appearance reset.
const RESET_COUNTRY_ENTRY: usize = 3;
/// [`entry_sites`] index of the daily politics update, the first
/// scope.
const POLITICS_DAILY_ENTRY: usize = 10;
/// [`entry_sites`] indices of national focus selection and
/// cancellation scopes, whose progress argument the country input
/// family's focus rows follow ([`crate::country_input_changes`]).
const FOCUS_ENTRY: usize = 36;
const FOCUS_REFUND_ENTRY: usize = 37;

/// Hook path: the political status family's notifications for an entry this
/// family patches on its behalf ([`engine_mutations::political_status_mutations`]),
/// with `receiver` its first argument: the country for
/// `country appearance reset`, the `CPoliticalStatus` for the daily update,
/// whose guard compares the status's last election after the original.
fn forwarded_status(
    index: usize,
    receiver: usize,
) -> Option<crate::political_status_changes::Guard<'static>> {
    use crate::political_status_changes::{Forwarded, forwarded};
    match index {
        RESET_COUNTRY_ENTRY => Some(forwarded(Forwarded::ResetCountry, receiver)),
        POLITICS_DAILY_ENTRY => Some(forwarded(Forwarded::PoliticsDaily, receiver)),
        _ => None,
    }
}
static ORIGINAL: [AtomicUsize; ENTRY_COUNT] = [const { AtomicUsize::new(0) }; ENTRY_COUNT];
/// The politics scope entries, in [`native::SCOPES`] order from entry 10:
/// the detour dispatches on the entry index, never on the scope's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    PoliticsDaily,
    ImproveRelationDaily,
    BoostPartyDaily,
    MioResearchDaily,
    MioProductionDaily,
    AddIdea,
    RemoveIdea,
    ReplaceIdea,
    HireAdvisor,
    ReplaceAdvisor,
    Decision,
    TargetedDecision,
    DecisionTimeout,
    Diplomacy,
    ScriptedDiplomacy,
    LeaderPromotion,
    LeaderTrait,
    AbilityCost,
    CoupCompletion,
    CivilWar,
    SetPoliticsEffect,
    PeaceForceGovernment,
    SubjectGovernment,
    ConsoleGovernment,
    ConsoleSubject,
    OnAction,
    Focus,
    FocusRefund,
}
const SCOPE_ORDER: [Scope; native::SCOPES.len()] = [
    Scope::PoliticsDaily,
    Scope::ImproveRelationDaily,
    Scope::BoostPartyDaily,
    Scope::MioResearchDaily,
    Scope::MioProductionDaily,
    Scope::AddIdea,
    Scope::RemoveIdea,
    Scope::ReplaceIdea,
    Scope::HireAdvisor,
    Scope::ReplaceAdvisor,
    Scope::Decision,
    Scope::TargetedDecision,
    Scope::DecisionTimeout,
    Scope::Diplomacy,
    Scope::ScriptedDiplomacy,
    Scope::LeaderPromotion,
    Scope::LeaderTrait,
    Scope::AbilityCost,
    Scope::CoupCompletion,
    Scope::CivilWar,
    Scope::SetPoliticsEffect,
    Scope::PeaceForceGovernment,
    Scope::SubjectGovernment,
    Scope::ConsoleGovernment,
    Scope::ConsoleSubject,
    Scope::OnAction,
    Scope::Focus,
    Scope::FocusRefund,
];
impl Scope {
    /// The event feed operation the scope starts, if any.
    fn provenance(self) -> Option<crate::war_provenance::PoliticsScope> {
        use crate::war_provenance::PoliticsScope;
        Some(match self {
            Self::Diplomacy => PoliticsScope::Diplomacy,
            Self::CivilWar => PoliticsScope::CivilWar,
            Self::Decision => PoliticsScope::Decision,
            Self::TargetedDecision => PoliticsScope::TargetedDecision,
            Self::DecisionTimeout => PoliticsScope::DecisionTimeout,
            Self::ScriptedDiplomacy => PoliticsScope::ScriptedDiplomacy,
            Self::OnAction => PoliticsScope::OnAction,
            _ => return None,
        })
    }
}
pub fn entry_sites() -> Vec<native::HookSite> {
    vec![
        native::ADD_POLITICAL_POWER,
        native::ADD_POLITICAL_POWER_NO_DEFICIT,
        native::CLEAR_POLITICS,
        native::RESET_COUNTRY,
        native::ADD_COMMAND_POWER,
        native::SPEND_COMMAND_POWER,
        native::ALLOCATE_COMMAND_POWER,
        native::RELEASE_COMMAND_POWER,
        native::ACTIVATE_ABILITY,
        native::GROUND_CREW,
    ]
    .into_iter()
    .chain(native::SCOPES.iter().copied())
    .collect()
}

// Five native integer/pointer args, plus collector-only site and caller. The
// stack shim retains the real return address and copies arg5 before allocating
// its own shadow/argument area. Full native RAX is returned, including AL.
macro_rules! entry {
    ($name:ident, $index:literal) => {
        core::arch::global_asm!(
            concat!(".text
.globl politics_", stringify!($name), "
.seh_proc politics_", stringify!($name), "
politics_", stringify!($name), ":"),
            "mov r10, rsp", "sub rsp, 0x48", ".seh_stackalloc 0x48", ".seh_endprologue",
            "mov rax, qword ptr [r10 + 0x28]", "mov qword ptr [rsp + 0x20], rax",
            "mov qword ptr [rsp + 0x28], {index}",
            "mov rax, qword ptr [r10]", "mov qword ptr [rsp + 0x30], rax",
            "mov qword ptr [rsp + 0x38], rbp",
            "call {body}", "add rsp, 0x48", "ret", ".seh_endproc",
            index = const $index, body = sym entry_body,
        );
        unsafe extern "system-unwind" {
            #[link_name = concat!("politics_", stringify!($name))]
            fn $name();
        }
    }
}
entry!(entry_0, 0);
entry!(entry_1, 1);
entry!(entry_2, 2);
entry!(entry_3, 3);
entry!(entry_4, 4);
entry!(entry_5, 5);
entry!(entry_6, 6);
entry!(entry_7, 7);
entry!(entry_8, 8);
entry!(entry_9, 9);
entry!(entry_10, 10);
entry!(entry_11, 11);
entry!(entry_12, 12);
entry!(entry_13, 13);
entry!(entry_14, 14);
entry!(entry_15, 15);
entry!(entry_16, 16);
entry!(entry_17, 17);
entry!(entry_18, 18);
entry!(entry_19, 19);
entry!(entry_20, 20);
entry!(entry_21, 21);
entry!(entry_22, 22);
entry!(entry_23, 23);
entry!(entry_24, 24);
entry!(entry_25, 25);
entry!(entry_26, 26);
entry!(entry_27, 27);
entry!(entry_28, 28);
entry!(entry_29, 29);
entry!(entry_30, 30);
entry!(entry_31, 31);
entry!(entry_32, 32);
entry!(entry_33, 33);
entry!(entry_34, 34);
entry!(entry_35, 35);
entry!(entry_36, 36);
entry!(entry_37, 37);
fn callbacks() -> [usize; ENTRY_COUNT] {
    [
        entry_0 as *const () as usize,
        entry_1 as *const () as usize,
        entry_2 as *const () as usize,
        entry_3 as *const () as usize,
        entry_4 as *const () as usize,
        entry_5 as *const () as usize,
        entry_6 as *const () as usize,
        entry_7 as *const () as usize,
        entry_8 as *const () as usize,
        entry_9 as *const () as usize,
        entry_10 as *const () as usize,
        entry_11 as *const () as usize,
        entry_12 as *const () as usize,
        entry_13 as *const () as usize,
        entry_14 as *const () as usize,
        entry_15 as *const () as usize,
        entry_16 as *const () as usize,
        entry_17 as *const () as usize,
        entry_18 as *const () as usize,
        entry_19 as *const () as usize,
        entry_20 as *const () as usize,
        entry_21 as *const () as usize,
        entry_22 as *const () as usize,
        entry_23 as *const () as usize,
        entry_24 as *const () as usize,
        entry_25 as *const () as usize,
        entry_26 as *const () as usize,
        entry_27 as *const () as usize,
        entry_28 as *const () as usize,
        entry_29 as *const () as usize,
        entry_30 as *const () as usize,
        entry_31 as *const () as usize,
        entry_32 as *const () as usize,
        entry_33 as *const () as usize,
        entry_34 as *const () as usize,
        entry_35 as *const () as usize,
        entry_36 as *const () as usize,
        entry_37 as *const () as usize,
    ]
}

type NativeFunction = unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) -> usize;
fn at(pointer: usize, offset: usize) -> Option<usize> {
    pointer.checked_add(offset)
}
#[inline(always)]
fn key(pointer: usize, offset: usize) -> native::Literal {
    at(pointer, offset)
        .and_then(|address| native::literal(&memory(), address).ok())
        .unwrap_or_else(|| {
            COUNTERS[4].fetch_add(1, Ordering::Relaxed);
            native::Literal::default()
        })
}
#[inline(always)]
fn item_read<T: engine_native::memory::Raw>(address: usize) -> Option<T> {
    read(address).or_else(|| {
        COUNTERS[4].fetch_add(1, Ordering::Relaxed);
        None
    })
}
#[inline(always)]
fn reference(pointer: usize, offset: usize, expected_type: u32) -> Option<u64> {
    reference_with(&memory(), pointer, offset, expected_type)
}
#[inline(always)]
fn reference_with(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    expected_type: u32,
) -> Option<u64> {
    let value = engine_native::read::<u64>(memory, at(pointer, offset)?).or_else(|| {
        COUNTERS[4].fetch_add(1, Ordering::Relaxed);
        None
    })?;
    if value >> 32 == 0 {
        return None;
    }
    if value as u32 != expected_type {
        COUNTERS[4].fetch_add(1, Ordering::Relaxed);
        return None;
    }
    Some(value)
}
fn context_identity(scope: Scope, a: usize, b: usize, c: usize, mut context: Context) -> Context {
    let mut identity = Identity {
        origin_kind: context.identity.origin_kind,
        flags: context.identity.flags & (1 << 5),
        words: [0, 0, 0, 0, 0, context.identity.words[5]],
        ..Default::default()
    };
    identity.keys[2] = context.identity.keys[2];
    match scope {
        // P: command idea pointer (replace's new idea after it), definition key inside the idea.
        // Both payment operations of a replace retain Replace provenance.
        Scope::AddIdea | Scope::RemoveIdea | Scope::ReplaceIdea => {
            let offset = if scope == Scope::ReplaceIdea {
                0x38
            } else {
                0x30
            };
            if let Some(idea) = at(a, offset).and_then(item_read::<u64>) {
                identity.keys[0] = key(idea as usize, engine_native::cidea::TOKEN);
            }
            identity.action = match scope {
                Scope::AddIdea => PowerAction::Add,
                Scope::RemoveIdea => PowerAction::Remove,
                _ => PowerAction::Replace,
            };
        }
        // The native reference retains its {type,id} and slot.
        Scope::HireAdvisor | Scope::ReplaceAdvisor => {
            let offset = if scope == Scope::HireAdvisor {
                0x28
            } else {
                0x30
            };
            identity.set(
                1,
                reference(a, offset, engine_native::ccharacter::REFERENCE_TYPE)
                    .map(|r| (r >> 32) as u32),
            );
            identity.keys[1] = key(
                a,
                if scope == Scope::HireAdvisor {
                    0x38
                } else {
                    0x40
                },
            );
            identity.action = if scope == Scope::HireAdvisor {
                PowerAction::Add
            } else {
                PowerAction::Replace
            };
        }
        // Targeted-decision handling receives status, selector, definition; The command passes its
        // selector words and its definition. Both target handlers retain their original
        // definition/selector args.
        Scope::Decision | Scope::TargetedDecision | Scope::DecisionTimeout => {
            let decision = if scope == Scope::TargetedDecision {
                c
            } else {
                a
            };
            identity.keys[0] = key(decision, engine_decisions::cdecision::TOKEN);
            identity.action = if scope == Scope::DecisionTimeout {
                PowerAction::Timeout
            } else {
                PowerAction::Execute
            };
            identity.origin_kind = PowerOriginKind::Decision;
            identity.keys[2] = identity.keys[0];
            if scope != Scope::Decision {
                let selector = if scope == Scope::TargetedDecision {
                    b
                } else {
                    c
                };
                // Target matching compares the selector's two target words against the instance's.
                let target = at(selector, 8).and_then(item_read::<i32>);
                if let Some(target) = target {
                    if target > 0 {
                        identity.set(2, Some(target as u32));
                    } else {
                        identity.set(
                            3,
                            at(selector, 0xc)
                                .and_then(item_read::<u32>)
                                .filter(|id| *id != 0),
                        );
                    }
                }
            }
        }
        // P: base action type; subclass target layouts differ, so no unproved generic target
        // offset. Scripted template and key.
        Scope::Diplomacy | Scope::ScriptedDiplomacy => {
            identity.set(0, at(a, 8).and_then(item_read::<u32>));
            identity.action = PowerAction::Execute;
            // The receiver resolves its own raw target.
            identity.set(
                2,
                at(a, 0x1c).and_then(item_read::<u32>).filter(|id| *id != 0),
            );
            let scripted = scope == Scope::ScriptedDiplomacy
                || read::<u64>(a)
                    .and_then(|vtable| at(vtable as usize, 0x140))
                    .and_then(item_read::<u64>)
                    == Some((base() + 0xaa3660) as u64);
            if scripted && let Some(template) = at(a, 0x78).and_then(item_read::<u64>) {
                identity.keys[0] = key(template as usize, 0x10);
            }
        }
        Scope::Focus | Scope::FocusRefund => {
            let focus = if scope == Scope::Focus {
                Some(b)
            } else {
                at(a, 0x10)
                    .and_then(item_read::<u64>)
                    .map(|pointer| pointer as usize)
            };
            if let Some(focus) = focus.filter(|pointer| *pointer != 0) {
                identity.keys[0] = key(focus, engine_focus::cnational_focus::TOKEN);
                identity.origin_kind = PowerOriginKind::Focus;
                identity.keys[2] = identity.keys[0];
                identity.action = if scope == Scope::Focus {
                    PowerAction::Start
                } else {
                    PowerAction::Stop
                };
            }
        }
        Scope::LeaderPromotion | Scope::LeaderTrait => {
            identity.set_reference(reference(
                a,
                0x28,
                engine_native::cunit_leader::REFERENCE_TYPE,
            ));
            if identity.object_reference().is_some() {
                identity.object_kind = PowerObjectKind::Leader;
            }
            identity.action = if scope == Scope::LeaderPromotion {
                PowerAction::Promote
            } else {
                PowerAction::LearnTrait
            };
            if scope == Scope::LeaderTrait
                && let Some(definition) = at(a, 0x30).and_then(item_read::<u64>)
            {
                identity.set(
                    0,
                    at(
                        definition as usize,
                        engine_native::cunit_leader_trait::TOKEN_ID,
                    )
                    .and_then(item_read::<u32>),
                );
            }
        }
        _ => return context,
    }
    context.identity = identity;
    context
}
fn government_context(scope: Scope, caller: usize, mut context: Context) -> Context {
    let cause = match scope {
        Scope::CoupCompletion => GovernmentCause::CoupCompletion,
        Scope::CivilWar => GovernmentCause::CivilWar,
        Scope::SetPoliticsEffect => GovernmentCause::SetPoliticsEffect,
        Scope::PeaceForceGovernment => GovernmentCause::PeaceForceGovernment,
        Scope::SubjectGovernment => GovernmentCause::SubjectGovernment,
        Scope::ConsoleGovernment | Scope::ConsoleSubject => GovernmentCause::Console,
        // The election update invokes on_election synchronously. Other notifications
        // are new scopes and inherit no election/coup provenance.
        Scope::OnAction => {
            context.government = if caller == 0xb992de {
                GovernmentCause::ElectionOnAction
            } else {
                GovernmentCause::NotIdentified
            };
            return context;
        }
        _ => return context,
    };
    if !matches!(
        context.government,
        GovernmentCause::ElectionOnAction
            | GovernmentCause::CoupCompletion
            | GovernmentCause::CivilWar
            | GovernmentCause::Console
    ) {
        context.government = cause;
    }
    context
}
fn government(
    status: usize,
    new_party: usize,
    site: PowerSite,
    forced_cause: Option<GovernmentCause>,
) {
    government_with_old(status, new_party, site, forced_cause, None);
}
fn government_with_old(
    status: usize,
    new_party: usize,
    site: PowerSite,
    forced_cause: Option<GovernmentCause>,
    observed_old: Option<usize>,
) {
    let owner = match native::status_owner(&memory(), base(), status) {
        Ok(owner) => owner,
        Err(reason) => {
            failed(reason);
            return;
        }
    };
    let Some(old_party) = observed_old.map(|p| p as u64).or_else(|| {
        at(status, engine_native::cpolitical_status::RULING_PARTY).and_then(read::<u64>)
    }) else {
        failed("politics ruling party pointer unreadable or overflowed");
        return;
    };
    if old_party as usize == new_party {
        return;
    }
    let old = native::party_literal(&memory(), base(), old_party as usize);
    let new = native::party_literal(&memory(), base(), new_party);
    let old = match old {
        Ok(value) => value,
        Err(reason) => {
            failed(reason);
            return;
        }
    };
    let new = match new {
        Ok(value) => value,
        Err(reason) => {
            failed(reason);
            return;
        }
    };
    let context = CONTEXT.get();
    let mut identity = context.identity;
    identity.keys[0] = old.map(|party| party.1).unwrap_or_default();
    identity.keys[1] = new.map(|party| party.1).unwrap_or_default();
    let mut raw = Record::base(Kind::Government, owner.country, owner.instance);
    raw.site = site;
    raw.government_cause = if context.government == GovernmentCause::NotIdentified {
        forced_cause.unwrap_or(context.government)
    } else {
        context.government
    };
    raw.identify(identity);
    for (n, value) in [old.map(|p| p.0), new.map(|p| p.0)].into_iter().enumerate() {
        raw.flags |= u16::from(value.is_some()) << (n + 6);
        raw.words[n + 6] = value.unwrap_or(0);
    }
    append_record(raw, identity.keys);
}
#[inline(always)]
fn component(owner: native::Owner, kind: PowerDailyCostKind, cost: i64, identity: Identity) {
    let cause = match kind {
        PowerDailyCostKind::Relation => PowerCause::Diplomacy,
        PowerDailyCostKind::Blockade => PowerCause::Blockade,
        _ => PowerCause::DailyGain,
    };
    append_operands(
        cost_record(
            owner,
            kind,
            cost,
            cause,
            PowerSite::AddPoliticalPowerNoDeficit,
            identity,
        ),
        identity.keys,
    );
}
#[inline(always)]
fn daily(owner: native::Owner, gross: i64, cost: i64, net: i64) {
    append_operands(
        daily_record(owner, gross, cost, net),
        [native::Literal::default(); 3],
    );
}

#[allow(clippy::too_many_arguments)] // Native args plus shim-only provenance.
unsafe extern "system-unwind" fn entry_body(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    fifth: usize,
    index: usize,
    caller: usize,
    _native_rbp: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PoliticsCapture);
    let _active = Call::enter();
    // Covers the original in flight, whether or not this family records;
    // dropped after it returns.
    let _status = forwarded_status(index, a);
    if index == RESET_COUNTRY_ENTRY {
        crate::event_feed_observer::reset_delayed_country(a);
    }
    // Country appearance reset rewrites every country input field of its
    // country, and focus selection and cancellation its focus rows
    // ([`crate::country_input_changes`]).
    let _inputs = {
        use crate::country_input_changes::{Forward, forwarded};
        match index {
            RESET_COUNTRY_ENTRY => Some(forwarded(Forward::ResetCountry, a, b)),
            FOCUS_ENTRY | FOCUS_REFUND_ENTRY => Some(forwarded(Forward::Focus, a, 0)),
            _ => None,
        }
    };
    // It also clears the country's naval hover tree ([`crate::naval_tree_changes`]).
    let _naval = (index == RESET_COUNTRY_ENTRY).then(|| crate::naval_tree_changes::forwarded(1, a));
    // The event feed's operations: how the wars and territory changes inside
    // these scopes began ([`crate::war_provenance::politics`]).
    let scope = index.checked_sub(10).map(|scope| SCOPE_ORDER[scope]);
    let _provenance = scope
        .and_then(Scope::provenance)
        .and_then(|scope| crate::war_provenance::politics(scope, caller));
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[index].load(Ordering::Acquire)) };
    if !ENABLED.load(Ordering::Acquire) {
        return timing.original(|| unsafe { original(a, b, c, d, fifth) });
    }
    let caller = caller.wrapping_sub(base());
    let mut restore = ScopeRestore::new();
    if index < 10 {
        let context = restore.context;
        let result = unsafe {
            writer(
                &timing,
                original,
                [a, b, c, d, fifth],
                index,
                caller,
                context,
            )
        };
        restore.completed = true;
        return result;
    }
    let scope = SCOPE_ORDER[index - 10];
    // The context ScopeRestore took a moment ago: nothing set it since.
    let previous = restore.context;
    let mut context = context_identity(scope, a, b, c, previous);
    context = government_context(scope, caller, context);
    if scope == Scope::PoliticsDaily {
        DAILY_HOUR.set(operand_hour());
    }
    CONTEXT.set(context);
    // These native entries already own their patches. Forward their actual
    // executions independently of whether any currency payment is emitted.
    match scope {
        Scope::Decision if caller != engine_decisions::cdecision::MISSION_COMPLETION_RETURN_RVA => {
            crate::script_event_observer::decision(
                replay_data::event_feed_rows::EventFeedSource::DecisionSelected,
                a,
                b,
                None,
            )
        }
        Scope::DecisionTimeout => crate::script_event_observer::decision(
            replay_data::event_feed_rows::EventFeedSource::DecisionTimeoutEffect,
            a,
            b,
            Some(c),
        ),
        _ => {}
    }
    let result = timing.original(|| unsafe { original(a, b, c, d, fifth) });
    if let Some(owner) = DAILY.get() {
        // Only these exact native returns guarantee RDX is the live cost output.
        // Other nested commands in the daily scope have unrelated second arguments.
        match scope {
            Scope::ImproveRelationDaily | Scope::BoostPartyDaily if caller == 0xb9a913 => {
                if let Some(cost) = operand::<i64>(b) {
                    let identity = relation_identity(a);
                    component(owner, PowerDailyCostKind::Relation, cost, identity);
                } else {
                    failed("politics daily relation cost output unreadable");
                }
            }
            Scope::MioResearchDaily if caller == 0xe4f8c9 => {
                mio_component(owner, a, operand::<i64>(b), PowerDailyCostKind::MioResearch)
            }
            Scope::MioProductionDaily if caller == 0xe4f8e2 => mio_component(
                owner,
                a,
                operand::<i64>(b),
                PowerDailyCostKind::MioProduction,
            ),
            _ => {}
        }
    }
    // The ability activation caller supplies the activation charge.
    // Availability, AI and UI callers of the same getter are unrelated.
    if scope == Scope::AbilityCost
        && caller == 0x6dd978
        && let Some(mut pending) = PENDING.get()
        && pending.site == PowerSite::ActivateAbility
        && let Some(cost) = read::<i64>(b)
    {
        pending.requested = Some(cost.wrapping_neg());
        PENDING.set(Some(pending));
    }
    restore.completed = true;
    result
}
fn mio_component(
    owner: native::Owner,
    organisation: usize,
    cost: Option<i64>,
    kind: PowerDailyCostKind,
) {
    let Some(cost) = cost else {
        failed("politics daily MIO cost output unreadable");
        return;
    };
    component(owner, kind, cost, mio_identity(organisation));
}
#[inline(always)]
fn relation_identity(relation: usize) -> Identity {
    relation_identity_with(&operand_memory(), relation)
}
#[inline(always)]
fn relation_identity_with(memory: &impl Memory, relation: usize) -> Identity {
    let mut identity = Identity::default();
    identity.set(
        0,
        at(relation, engine_native::crelation::TYPE_ID)
            .and_then(|p| engine_native::read::<u32>(memory, p)),
    );
    identity.set(
        2,
        at(relation, engine_native::crelation::SECOND_COUNTRY)
            .and_then(|p| engine_native::read::<u32>(memory, p))
            .filter(|id| *id != 0),
    );
    identity
}
#[inline(always)]
fn mio_identity(organisation: usize) -> Identity {
    mio_identity_with(&operand_memory(), organisation)
}
#[inline(always)]
fn mio_identity_with(memory: &impl Memory, organisation: usize) -> Identity {
    let mut identity = Identity::default();
    let reference = reference_with(
        memory,
        organisation,
        engine_production::nindustrial_organisation_corganisation::REFERENCE,
        engine_production::nindustrial_organisation_corganisation::REFERENCE_TYPE_ID,
    );
    identity.set_reference(reference);
    identity.object_kind = if reference.is_some() {
        PowerObjectKind::Organisation
    } else {
        PowerObjectKind::None
    };
    identity.set(
        0,
        at(
            organisation,
            engine_production::nindustrial_organisation_corganisation::TOKEN,
        )
        .and_then(|p| engine_native::read::<u32>(memory, p)),
    );
    identity
}
// The active record is alive at the native anniversary payment.
#[inline(always)]
fn upkeep_identity(record: usize) -> Identity {
    upkeep_identity_with(&operand_memory(), record)
}
#[inline(always)]
fn upkeep_identity_with(memory: &impl Memory, record: usize) -> Identity {
    let mut identity = Identity {
        action: PowerAction::Upkeep,
        ..Default::default()
    };
    if let Some(definition) = at(
        record,
        engine_land_inputs::active_ability_record::DEFINITION,
    )
    .and_then(|p| engine_native::read::<u64>(memory, p))
    {
        identity.keys[0] = native::literal(
            memory,
            definition as usize + engine_land_inputs::cability::TOKEN,
        )
        .unwrap_or_else(|_| {
            COUNTERS[4].fetch_add(1, Ordering::Relaxed);
            native::Literal::default()
        });
    }
    if let Some(leader) = at(record, engine_land_inputs::active_ability_record::CASTER)
        .and_then(|p| engine_native::read::<u64>(memory, p))
        .filter(|p| *p != 0)
    {
        identity.set_reference(reference_with(
            memory,
            leader as usize,
            engine_native::cunit_leader::TYPE,
            engine_native::cunit_leader::REFERENCE_TYPE,
        ));
        if identity.object_reference().is_some() {
            identity.object_kind = PowerObjectKind::Leader;
        }
    }
    identity
}
#[cfg(any(test, feature = "test-support"))]
#[inline(always)]
pub fn bench_identity(kind: PowerDailyCostKind, receiver: usize) -> Identity {
    if kind == PowerDailyCostKind::Relation {
        relation_identity(receiver)
    } else {
        mio_identity(receiver)
    }
}
#[cfg(any(test, feature = "test-support"))]
#[inline(always)]
pub fn bench_upkeep_identity(record: usize) -> Identity {
    upkeep_identity(record)
}

/// `context` is the thread's current politics context, which nothing in
/// this function sets before its original runs ([`government`] only reads it).
unsafe fn writer(
    timing: &crate::hook_timing::Span,
    original: NativeFunction,
    args: [usize; 5],
    index: usize,
    caller: usize,
    context: Context,
) -> usize {
    let [a, b, c, d, fifth] = args;
    flush(false);
    let mut previous = PENDING.get();
    let native_daily = caller == native::DAILY_PP_CALLER || caller == native::DAILY_CP_CALLER;
    let (owner, currency, site) = match index {
        0..=2 => (
            if native_daily {
                native::status_owner(&operand_memory(), base(), a)
            } else {
                native::status_owner(&memory(), base(), a)
            },
            PowerCurrency::Political,
            [
                PowerSite::AddPoliticalPower,
                PowerSite::AddPoliticalPowerNoDeficit,
                PowerSite::ClearPolitics,
            ][index],
        ),
        _ => (
            if native_daily {
                native::command_owner(&operand_memory(), base(), a)
            } else {
                native::command_owner(&memory(), base(), a)
            },
            PowerCurrency::Command,
            [
                PowerSite::ResetCountry,
                PowerSite::AddCommandPower,
                PowerSite::SpendCommandPower,
                PowerSite::AllocateCommandPower,
                PowerSite::ReleaseCommandPower,
                PowerSite::ActivateAbility,
                PowerSite::GroundCrew,
            ][index - 3],
        ),
    };
    let owner = match owner {
        Ok(owner) => Some(owner),
        Err(reason) => {
            failed(reason);
            None
        }
    };
    let before = owner.and_then(|owner| {
        if native_daily {
            operand_balance(owner, currency)
        } else {
            balance(owner, currency)
        }
    });
    let mut identity = context.identity;
    // The nested ability allocator and release refund
    // belong to their observed parent, not an outer command.
    if matches!(caller, 0x6c6d75 | 0x6c7319 | 0x6ddce7 | 0x6de4cf)
        && let Some(parent) = previous.filter(|parent| Some(parent.owner) == owner)
    {
        identity = parent.identity;
    }
    let mut cause = caller_cause(caller);
    let requested = match index {
        0 | 1 => Some(b as i64),
        2 => before.map(i64::wrapping_neg),
        // Reset assigns the configured starting command power; only the
        // observed effective reset is known here.
        3 => None,
        4 => {
            if native_daily {
                operand::<i64>(b)
            } else {
                read::<i64>(b)
            }
        }
        5 => read::<i64>(b).map(i64::wrapping_neg),
        6 if c as u8 != 0 => at(b, 0x10)
            .and_then(read::<i64>)
            .filter(|value| *value > 0)
            .map(i64::wrapping_neg),
        9 if fifth as u8 != 0 && d as i64 > 0 => Some((d as i64).wrapping_neg()),
        _ => None,
    };
    // The leader's embedded allocator receives the deployment charge.
    if caller == 0xc0bd82
        && let Some(leader) = b.checked_sub(0x1078)
    {
        identity.set_reference(reference(
            leader,
            engine_native::cunit_leader::TYPE,
            engine_native::cunit_leader::REFERENCE_TYPE,
        ));
        if identity.object_reference().is_some() {
            identity.object_kind = PowerObjectKind::Leader;
        }
    }
    if index == 2 || index == 3 {
        cause = PowerCause::ResetSetup;
    }
    if index == 7 {
        cause = PowerCause::CapCorrection;
    }
    if index == 6 && requested.is_none() {
        cause = PowerCause::CapCorrection;
    }
    if cause == PowerCause::CapCorrection {
        identity = Identity::default();
    }
    if index == 8 {
        cause = PowerCause::AbilityActivation;
        identity.keys[0] = key(b, engine_land_inputs::cability::TOKEN);
        identity.action = PowerAction::Activate;
    }
    if index == 9 {
        cause = PowerCause::AirAction;
        identity.set(
            4,
            at(b, engine_native::cstrategic_region::ID).and_then(read::<u32>),
        );
        identity.action = if d as i64 > 0 {
            PowerAction::Start
        } else {
            PowerAction::Stop
        };
    }
    if index == 2 {
        government(
            a,
            0,
            PowerSite::ClearPolitics,
            Some(GovernmentCause::ResetSetup),
        );
    }
    let operation = if matches!(index, 2 | 3) {
        PowerOperation::Reset
    } else if index == 7 || index == 6 && requested.is_none() {
        PowerOperation::AllocationClamp
    } else if matches!(
        cause,
        PowerCause::FocusRefund | PowerCause::AllocationRefund
    ) {
        PowerOperation::Refund
    } else if cause != PowerCause::DailyGain && requested.is_some_and(|value| value > 0) {
        PowerOperation::Grant
    } else if requested.is_some_and(|value| value < 0) {
        PowerOperation::Subtract
    } else {
        PowerOperation::Add
    };
    if let (Some(owner), Some(before)) = (owner, before) {
        PENDING.set(Some(Pending {
            owner,
            currency,
            before,
            requested,
            cause,
            site,
            operation,
            identity,
            emitted: false,
        }));
    } else {
        PENDING.set(None);
    }
    let previous_context = context;
    if index == 3 {
        CONTEXT.set(Context {
            government: GovernmentCause::ResetSetup,
            ..previous_context
        });
    }
    let result = timing.original(|| unsafe { original(a, b, c, d, fifth) });
    CONTEXT.set(previous_context);
    let force = matches!(index, 0..=4) || matches!(index, 5 | 6) && result as u8 != 0;
    flush(
        force
            && PENDING
                .get()
                .is_some_and(|p| p.requested.is_some() || matches!(index, 2 | 3)),
    );
    if let Some(pending) = &mut previous
        && let Some(after) = balance(pending.owner, pending.currency)
    {
        pending.before = after;
    }
    PENDING.set(previous);
    result
}

pub const TAPS: [native::HookSite; 10] = [
    native::SET_POLITICAL_POWER,
    native::ABILITY_UPKEEP,
    native::SET_RULING_PARTY,
    native::FORCE_RULING_PARTY,
    native::SETUP_POLITICS,
    native::FALLBACK_RULING_PARTY,
    native::DAILY_POLITICAL_RATE,
    native::DAILY_PP_MODIFIER,
    native::PP_SIGNAL,
    native::PP_SIGNAL_NO_DEFICIT,
];
unsafe extern "system-unwind" fn tap(registers: &Registers, index: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::PoliticsCaptureTaps);
    let _active = Call::enter();
    if !ENABLED.load(Ordering::Acquire) {
        return;
    }
    match index {
        0 => {
            flush(false);
            let owner = observed_owner(native::status_owner(&memory(), base(), registers.rsi));
            if let Some(owner) = owner
                && let Some(before) = balance(owner, PowerCurrency::Political)
            {
                power(
                    owner,
                    PowerCurrency::Political,
                    PowerOperation::Set,
                    PowerCause::ScriptEffect,
                    PowerSite::SetPoliticalPower,
                    before,
                    (registers.rcx as i64).max(0),
                    Some((registers.rcx as i64).wrapping_sub(before)),
                    CONTEXT.get().identity,
                );
                resume_pending(
                    owner,
                    PowerCurrency::Political,
                    (registers.rcx as i64).max(0),
                );
            }
        }
        1 => {
            flush(false);
            let owner = observed_owner(native::command_owner(
                &operand_memory(),
                base(),
                registers.r13,
            ));
            if let Some(owner) = owner
                && let Some(before) = operand_balance(owner, PowerCurrency::Command)
            {
                let identity = registers
                    .r15
                    .checked_add(registers.rbx)
                    .map(upkeep_identity)
                    .unwrap_or_else(|| {
                        failed("politics ability upkeep identity address overflow");
                        Identity::default()
                    });
                append_operands(
                    power_record(
                        owner,
                        PowerCurrency::Command,
                        PowerOperation::Subtract,
                        PowerCause::AbilityUpkeep,
                        PowerSite::AbilityUpkeep,
                        before,
                        registers.rcx as i64,
                        Some((registers.rdx as i64).wrapping_neg()),
                        identity,
                    ),
                    identity.keys,
                );
                resume_pending(owner, PowerCurrency::Command, registers.rcx as i64);
            }
        }
        2 => government(
            registers.rbx,
            registers.rax,
            PowerSite::SetRulingParty,
            None,
        ),
        3 => government(
            registers.rbp,
            registers.rsi,
            PowerSite::ForceRulingParty,
            None,
        ),
        4 => government(
            registers.rbp,
            registers.rcx,
            PowerSite::SetupPolitics,
            Some(GovernmentCause::ResetSetup),
        ),
        5 => {
            // Bypassing the party loop leaves ZF clear. Exhaustion/empty
            // leaves ZF set and started from a null party.
            if registers.flags & (1 << 6) != 0 {
                if let Some(party) = at(
                    registers.r14,
                    engine_native::cpolitical_status::RULING_PARTY,
                )
                .and_then(read::<u64>)
                {
                    if party != 0 {
                        government_with_old(
                            registers.r14,
                            party as usize,
                            PowerSite::FallbackRulingParty,
                            Some(GovernmentCause::ResetSetup),
                            Some(0),
                        );
                    }
                } else {
                    failed("politics fallback ruling party pointer unreadable or overflowed");
                }
            }
        }
        6 => {
            // Restrict this on-demand getter observation to its daily
            // caller. Tooltip/trigger evaluations are not income applications.
            let caller = at(registers.rbp, 0x358)
                .and_then(operand::<u64>)
                .map(|r| (r as usize).wrapping_sub(base()));
            if caller != Some(0xb99362) {
                return;
            }
            DAILY.set(None);
            let owner = observed_owner(native::status_owner(
                &operand_memory(),
                base(),
                registers.r13,
            ));
            let cost = registers.rbp.checked_sub(0x38).and_then(operand::<i64>);
            if let (Some(owner), Some(cost)) = (owner, cost) {
                daily(owner, registers.rdi as i64, cost, registers.rax as i64);
                if let Some(define) = base()
                    .checked_add(native::BLOCKADE_COST_DEFINE_RVA)
                    .and_then(operand::<i64>)
                {
                    component(
                        owner,
                        PowerDailyCostKind::Blockade,
                        native::blockade_cost(registers.r15, define),
                        Identity::default(),
                    );
                } else {
                    failed("politics blockade cost define unreadable or overflowed");
                }
            } else if owner.is_some() {
                failed("politics daily political cost output unreadable or overflowed");
            }
        }
        7 => {
            // Only the daily caller supplies an applied income observation.
            if at(registers.rbp, 0x358).and_then(operand::<u64>) != Some((base() + 0xb99362) as u64)
            {
                return;
            }
            DAILY.set(None);
            if let Some(owner) = observed_owner(native::status_owner(
                &operand_memory(),
                base(),
                registers.r13,
            )) {
                if let Some(cost) = registers.rbp.checked_sub(0x38).and_then(operand::<i64>) {
                    DAILY.set(Some(owner));
                    component(
                        owner,
                        PowerDailyCostKind::PoliticalPowerCostModifier,
                        cost,
                        Identity::default(),
                    );
                } else {
                    failed("politics daily cost modifier output unreadable or overflowed");
                }
            }
        }
        8 | 9 => flush(
            PENDING
                .get()
                .is_some_and(|p| p.currency == PowerCurrency::Political && p.requested.is_some()),
        ),
        _ => unreachable!(),
    }
}

pub fn status() -> impl serde::Serialize {
    let counters = COUNTERS
        .each_ref()
        .map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "unhooked": UNHOOKED.get(), "enabled": ENABLED.load(Ordering::Acquire),
        "feature_enabled": cfg!(feature = "politics_capture"), "runtime_enabled": RUNTIME_ENABLED.load(Ordering::Relaxed), "release_refusals": counters[8], "unwound_observers": counters[9],
        "epoch": EPOCH.load(Ordering::SeqCst), "events": log().map(Log::frontier),
        "overflows": counters[1], "sentinel_failures": counters[2],
        "native_read_reasons": crate::status::map(FAILURE_NAMES.into_iter().zip(FAILURES.iter().map(|count| count.load(Ordering::Relaxed))).filter(|(_, count)| *count != 0)),
        "first_unlisted_native_read_reason": UNLISTED_FAILURE.get().copied(),
        "unlisted_native_read_reasons": UNLISTED_FAILURES.load(Ordering::Relaxed),
        "failed_native_reads": counters[3], "missing_item_identities": counters[4],
        "required_entries_per_lane": log().map(|log| crate::status::sequence(log.lanes.iter().map(|lane| lane.required_entries.load(Ordering::Relaxed)))),
        "staging_entries_per_lane": log().map(|log| crate::status::sequence(log.lanes.iter().map(|lane| lane.slots.len()))),
        "required_key_bytes": REQUIRED_KEYS.load(Ordering::Relaxed),
        "staging_entries": log().map(|log| log.entries),
        "staging_threads": log().map(|log| log.lanes.len()),
        "shared_lane_threads": counters[11], "shared_lane_refusals": counters[12],
        "shared_lane_entries": log().map(|log| log.shared.len()),
        "required_shared_lane_entries": log().map(|log| log.shared.required_entries()),
        "staging_key_stride": log().map(|log| log.key_stride),
        "observer_timing": "offline and matched live run; no per-detour clock instrumentation",
        "staging_allocation_failures": counters[10],
        "staging_error": (counters[10] != 0).then_some("politics staging allocation failed"),
        "short_handoffs": counters[6], "daily_decomposition_failures":counters[7],
        "conservation": conservation_status(), "government_endpoint_mismatches":counters[5],
    })
}
#[derive(Clone, Copy)]
struct ConservationExample {
    country: u32,
    hour: i32,
    currency: PowerCurrency,
    amount: i128,
}
#[derive(Clone, Copy, Default)]
struct Conservation {
    counts: [u64; 2],
    amounts: [i128; 2],
    absolute_amounts: [u128; 2],
    // The existing diagnostic sample retains the first eight of each currency.
    examples: [Option<ConservationExample>; 16],
}
static CONSERVATION: Mutex<Conservation> = Mutex::new(Conservation {
    counts: [0; 2],
    amounts: [0; 2],
    absolute_amounts: [0; 2],
    examples: [None; 16],
});
pub fn mismatch(country: u32, hour: i32, currency: PowerCurrency, amount: i128) {
    let mut metrics = CONSERVATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let index = currency as usize;
    metrics.counts[index] += 1;
    metrics.amounts[index] += amount;
    metrics.absolute_amounts[index] += amount.unsigned_abs();
    if metrics
        .examples
        .iter()
        .flatten()
        .filter(|example| example.currency == currency)
        .count()
        < 8
        && let Some(example) = metrics
            .examples
            .iter_mut()
            .find(|example| example.is_none())
    {
        *example = Some(ConservationExample {
            country,
            hour,
            currency,
            amount,
        });
    }
}
fn conservation_status() -> impl serde::Serialize {
    use crate::status::{DebugValue, DisplayValue, sequence, status};
    let metrics = *CONSERVATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    status!({
        "pp_mismatches": metrics.counts[0], "cp_mismatches": metrics.counts[1],
        "pp_amount_raw": DisplayValue(metrics.amounts[0]), "cp_amount_raw": DisplayValue(metrics.amounts[1]),
        "pp_absolute_amount_raw": DisplayValue(metrics.absolute_amounts[0]), "cp_absolute_amount_raw": DisplayValue(metrics.absolute_amounts[1]),
        "examples": sequence(metrics.examples.into_iter().flatten().map(|example| status!({
            "country": example.country, "hour": example.hour,
            "currency": DebugValue(example.currency), "amount_raw": DisplayValue(example.amount),
        }))),
    })
}

pub struct Hooks {
    entries: Vec<Hook>,
    taps: Vec<Tap>,
}
impl Hooks {
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize, runtime_enabled: bool) -> Self {
        RUNTIME_ENABLED.store(runtime_enabled, Ordering::Relaxed);
        if !runtime_enabled {
            let _ = UNHOOKED.set("politics_capture disabled by attach config".into());
            return Self {
                entries: Vec::new(),
                taps: Vec::new(),
            };
        }
        if !cfg!(feature = "politics_capture") {
            let _ = UNHOOKED.set("politics_capture feature disabled".into());
            return Self {
                entries: Vec::new(),
                taps: Vec::new(),
            };
        }
        unsafe { Self::install(memory, base) }.unwrap_or_else(|reason| {
            crate::hook::installation::refused(
                crate::hook::installation::Kind::Construction,
                None,
                &reason,
            );
            let _ = UNHOOKED.set(reason);
            Self {
                entries: Vec::new(),
                taps: Vec::new(),
            }
        })
    }
    unsafe fn install(memory: &impl Memory, base: usize) -> Result<Self, String> {
        let sites = entry_sites();
        for site in sites.iter().chain(TAPS.iter()) {
            if !crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel) {
                COUNTERS[2].fetch_add(1, Ordering::Relaxed);
                return Err(format!("politics {} sentinel differs", site.name));
            }
        }
        BASE.store(base, Ordering::Relaxed);
        let mut entries = Vec::new();
        for (index, (site, callback)) in sites.into_iter().zip(callbacks()).enumerate() {
            let hook = unsafe { Hook::new(base + site.rva, callback, site.sentinel) }?;
            ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
            entries.push(hook);
        }
        let mut taps = Vec::new();
        for (index, site) in TAPS.into_iter().enumerate() {
            taps.push(unsafe { Tap::new(base + site.rva, tap, index, site.sentinel) }?);
        }
        taps.push(
            unsafe { crate::script_event_observer::targeted_selection_tap(memory, base) }
                .map_err(str::to_owned)?,
        );
        Ok(Self { entries, taps })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
            crate::political_status_changes::set_forward(false);
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Politics,
                false,
            );
            crate::naval_tree_changes::forwarder(1, false);
            crate::event_feed_observer::forwarder(
                crate::event_feed_observer::Forwarder::Politics,
                false,
            );
        }
        if !crate::hook::shared::defer_preparation(invalidate) {
            invalidate();
        }
        let outcome = match crate::hook::shared::attach_outcome() {
            Some(outcome) => outcome,
            None => {
                let mut family = crate::hook::shared::publication_storage_attach(
                    self.entries.len() + self.taps.len(),
                    enabled,
                )
                .map_err(|_| "politics hook family allocation failed")?;
                family.extend(
                    self.entries
                        .iter_mut()
                        .chain(self.taps.iter_mut().map(Tap::publication_hook)),
                );
                Hook::set_enabled_batch_attach(&mut family, enabled)
            }
        };
        if !outcome.map_err(|reason| {
            let _ = UNHOOKED.set(reason);
            "politics hook state change failed"
        })? {
            return Ok(false);
        }
        ENABLED.store(enabled && !self.entries.is_empty(), Ordering::SeqCst);
        // The political status family observes country appearance reset and the
        // daily politics update through these patches ([`forwarded_status`]).
        crate::political_status_changes::set_forward(enabled && !self.entries.is_empty());
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Politics,
            enabled && !self.entries.is_empty(),
        );
        crate::naval_tree_changes::forwarder(1, enabled && !self.entries.is_empty());
        crate::event_feed_observer::forwarder(
            crate::event_feed_observer::Forwarder::Politics,
            enabled && !self.entries.is_empty(),
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.entries
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.taps.iter().flat_map(Tap::executable_ranges))
            .collect()
    }
}

pub fn short_handoff() {
    COUNTERS[6].fetch_add(1, Ordering::Relaxed);
}
pub fn daily_failure() {
    COUNTERS[7].fetch_add(1, Ordering::Relaxed);
}
pub fn government_mismatch() {
    COUNTERS[5].fetch_add(1, Ordering::Relaxed);
}

#[cfg(any(test, feature = "test-support"))]
static STAGING_TEST: Mutex<()> = Mutex::new(());
#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static STAGING_OWNER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Serialises tests that publish staging or read the observation epoch and
/// makes this thread the epoch's owner. As with the naval, air-loss and
/// bombing observers, another test's hourly path (`lost_observer_interval`)
/// then cannot move the epoch under a politics test. A failing test releases
/// the lock poisoned; later tests set their own staging, so the poison is
/// ignored and one failure stays one failure.
#[cfg(any(test, feature = "test-support"))]
pub struct StagingTest {
    _serial: std::sync::MutexGuard<'static, ()>,
}
#[cfg(any(test, feature = "test-support"))]
pub fn staging_test() -> StagingTest {
    let serial = STAGING_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    STAGING_OWNER.set(true);
    StagingTest { _serial: serial }
}
#[cfg(any(test, feature = "test-support"))]
impl Drop for StagingTest {
    fn drop(&mut self) {
        STAGING_OWNER.set(false);
    }
}
/// Owns offline lanes while exercising production publication and handoff.
#[cfg(any(test, feature = "test-support"))]
pub struct StagingFixture(Log);
#[cfg(any(test, feature = "test-support"))]
impl StagingFixture {
    pub fn new(entries: usize, lanes: usize) -> Self {
        Self(Log::with_lanes(entries, 128, lanes))
    }
    pub fn observe(&self, mut event: Event, memory: &impl Memory) {
        event.epoch = EPOCH.load(Ordering::Acquire);
        append_to(&self.0, event, memory);
    }
    pub fn handoff(&self, cursors: &mut [LaneCursor]) -> Result<Handoff, &'static str> {
        handoff_from(&self.0, cursors)
    }
    pub fn registered_lanes(&self) -> usize {
        self.0.registered.load(Ordering::Acquire)
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn stage_fixture(events: &[Event], memory: &impl Memory) -> (Drained, Vec<Event>, Vec<u8>) {
    let _guard = staging_test();
    let log = Log::new(events.len(), 128);
    for event in events {
        append_to(&log, *event, memory);
    }
    let mut output = vec![Event::default(); events.len()];
    let mut bytes = vec![0; events.len() * 128];
    let drain = drain_from(&log, &mut output, &mut bytes).unwrap();
    bytes.truncate(drain.bytes);
    (drain, output, bytes)
}
#[cfg(any(test, feature = "test-support"))]
pub fn bench_begin(base: usize, entries: usize, keys: usize) {
    BASE.store(base, Ordering::Relaxed);
    resize(entries, keys);
    ENABLED.store(true, Ordering::SeqCst);
    let mut cursors = vec![LaneCursor::default(); thread_slots()];
    let _ = handoff(&mut cursors);
    let log = log().unwrap();
    for lane in &log.lanes {
        lane.consumed
            .store(lane.published.load(Ordering::Acquire), Ordering::Release);
    }
    log.consumed
        .store(log.cursor.load(Ordering::SeqCst), Ordering::Release);
    log.consumed_epoch
        .store(EPOCH.load(Ordering::SeqCst), Ordering::SeqCst);
}

#[cfg(any(test, feature = "test-support"))]
#[inline(always)]
pub fn bench_scope(f: impl FnOnce()) {
    let _active = Call::enter();
    DAILY_HOUR.set(event_hour());
    f();
    DAILY_HOUR.set(None);
}
#[cfg(any(test, feature = "test-support"))]
#[inline(always)]
pub fn bench_observe(event: Event) {
    let _active = Call::enter();
    append_record(fixture_record(&event), event.identity.keys);
}

/// The timed burst uses the same narrow constructors as the detours, rather
/// than decoding/copying the worker's expanded Event on the simulation thread.
#[cfg(any(test, feature = "test-support"))]
pub fn bench_daily_country(country: u32, receivers: &[usize]) {
    let owner = native::Owner {
        country,
        instance: 0,
        pointer: 0,
        politics: None,
    };
    bench_scope(|| {
        let emit_cost = |kind, identity: Identity| {
            let _active = Call::enter();
            append_operands(
                cost_record(
                    owner,
                    kind,
                    1,
                    PowerCause::NotIdentified,
                    PowerSite::NotIdentified,
                    identity,
                ),
                identity.keys,
            );
        };
        emit_cost(
            PowerDailyCostKind::PoliticalPowerCostModifier,
            Identity::default(),
        );
        for (n, receiver) in receivers[..6].iter().enumerate() {
            let kind = if n < 4 {
                PowerDailyCostKind::Relation
            } else if n == 4 {
                PowerDailyCostKind::MioResearch
            } else {
                PowerDailyCostKind::MioProduction
            };
            emit_cost(kind, bench_identity(kind, *receiver));
        }
        emit_cost(PowerDailyCostKind::Blockade, Identity::default());
        {
            let _active = Call::enter();
            daily(owner, 10, 8, 2);
        }
        for currency in [PowerCurrency::Political, PowerCurrency::Command] {
            let _active = Call::enter();
            append_operands(
                power_record(
                    owner,
                    currency,
                    PowerOperation::Add,
                    PowerCause::DailyGain,
                    PowerSite::NotIdentified,
                    0,
                    2,
                    Some(2),
                    Identity::default(),
                ),
                [native::Literal::default(); 3],
            );
        }
        for (n, receiver) in receivers[6..].iter().enumerate() {
            let _active = Call::enter();
            let identity = upkeep_identity(*receiver);
            append_operands(
                power_record(
                    owner,
                    PowerCurrency::Command,
                    PowerOperation::Add,
                    PowerCause::AbilityUpkeep,
                    PowerSite::NotIdentified,
                    2 - n as i64,
                    1 - n as i64,
                    Some(-1),
                    identity,
                ),
                identity.keys,
            );
        }
    });
}

/// Two production daily tap bodies, including exact caller/owner reads. The
/// fixture supplies owned native layouts; no native function is executed.
#[cfg(any(test, feature = "test-support"))]
pub fn bench_daily_taps(status: usize, modifier_frame: usize, rate_frame: usize) {
    let mut registers: Registers = unsafe { std::mem::zeroed() };
    registers.r13 = status;
    registers.rbp = modifier_frame;
    bench_scope(|| unsafe {
        tap(&registers, 7);
        registers.rbp = rate_frame;
        registers.rdi = 10;
        registers.rax = 2;
        registers.r15 = 1;
        tap(&registers, 6);
    });
}

#[cfg(any(test, feature = "test-support"))]
pub fn bench_drain(events: &mut [Event], bytes: &mut [u8]) -> Drained {
    drain_from(log().unwrap(), events, bytes).unwrap()
}

#[cfg(any(test, feature = "test-support"))]
pub fn bench_staging_parts(events: &[Event]) {
    use std::{hint::black_box, time::Instant};
    let packed: Vec<_> = events
        .iter()
        .map(|e| (fixture_record(e), e.identity.keys))
        .collect();
    for mode in ["guard", "direct_lane", "append_record", "expanded_fixture"] {
        let mut times = Vec::new();
        for _ in 0..100 {
            bench_begin(base(), events.len(), 32);
            let log = log().unwrap();
            let index = lane_index(log).unwrap();
            let start = Instant::now();
            bench_scope(|| match mode {
                "guard" => {
                    for _ in &packed {
                        let active = Call::enter();
                        black_box(&active);
                    }
                }
                "direct_lane" => {
                    for (raw, keys) in &packed {
                        append_raw_lane(log, index, *raw, *keys, &memory());
                    }
                }
                "append_record" => {
                    for (raw, keys) in &packed {
                        append_record(*raw, *keys);
                    }
                }
                _ => {
                    for event in events {
                        bench_observe(*event);
                    }
                }
            });
            times.push(start.elapsed().as_nanos());
        }
        times.sort();
        println!(
            "staging_part={mode} events={} median_p95_ns=({}, {})",
            events.len(),
            times[50],
            times[95]
        );
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn bench_end() {
    ENABLED.store(false, Ordering::SeqCst);
}

#[cfg(any(test, feature = "test-support"))]
pub fn bench_midnight_parts(receivers: &[usize]) {
    use std::{hint::black_box, time::Instant};
    let count = receivers.len() / 8;
    let owners: Vec<_> = receivers.chunks_exact(8).map(|r| r[0] - 0x2200).collect();
    let events: Vec<_> = receivers
        .chunks_exact(8)
        .flat_map(|r| {
            let mut v = vec![(Record::default(), [native::Literal::default(); 3]); 13];
            for (n, p) in r.iter().enumerate() {
                let i = if n < 4 {
                    relation_identity(*p)
                } else if n < 6 {
                    mio_identity(*p)
                } else {
                    upkeep_identity(*p)
                };
                v[n].0.identify(i);
                v[n].1 = i.keys;
            }
            v
        })
        .collect();
    let log = log().unwrap();
    let index = lane_index(log).unwrap();
    let stack = [black_box(base() + native::DAILY_PP_CALLER); 128];
    for mode in [
        "owner_status",
        "owner_country",
        "stack_scalar",
        "clock",
        "relation",
        "mio",
        "upkeep_identity",
        "literal_copy",
        "tls_slot",
        "record_empty",
        "record_populated",
        "record_api",
        "country_batch",
        "nested_guard",
    ] {
        let mut times = Vec::new();
        for iteration in 0..110 {
            bench_begin(base(), events.len(), 32);
            let start = Instant::now();
            bench_scope(|| match mode {
                "owner_status" => {
                    for p in &owners {
                        black_box(
                            native::status_owner(&operand_memory(), base(), p + 0x2000).unwrap(),
                        );
                    }
                }
                "owner_country" => {
                    for p in &owners {
                        black_box(native::command_owner(&operand_memory(), base(), *p).unwrap());
                    }
                }
                "stack_scalar" => {
                    for _ in &events {
                        black_box(operand::<u64>(stack.as_ptr() as usize));
                    }
                }
                "clock" => {
                    for _ in &owners {
                        black_box(operand_hour());
                    }
                }
                "relation" => {
                    for r in receivers.chunks_exact(8) {
                        for p in &r[..4] {
                            black_box(relation_identity(*p));
                        }
                    }
                }
                "mio" => {
                    for r in receivers.chunks_exact(8) {
                        for p in &r[4..6] {
                            black_box(mio_identity(*p));
                        }
                    }
                }
                "upkeep_identity" => {
                    for r in receivers.chunks_exact(8) {
                        for p in &r[6..] {
                            black_box(upkeep_identity(*p));
                        }
                    }
                }
                "literal_copy" => {
                    let mut buf = [0; 32];
                    for (_, keys) in &events {
                        for key in keys {
                            if key.len != 0 {
                                key.copy(&operand_memory(), &mut buf).unwrap();
                                black_box(&buf);
                            }
                        }
                    }
                }
                "tls_slot" => {
                    for _ in &events {
                        black_box(cached_lane(log));
                    }
                }
                "record_empty" => {
                    for (raw, _) in &events {
                        append_raw_lane(
                            log,
                            index,
                            *raw,
                            [native::Literal::default(); 3],
                            &memory(),
                        );
                    }
                }
                "record_populated" => {
                    for (raw, keys) in &events {
                        append_raw_lane(log, index, *raw, *keys, &operand_memory());
                    }
                }
                "record_api" => {
                    for (raw, keys) in &events {
                        append_operands(*raw, *keys);
                    }
                }
                "country_batch" => {
                    for (n, r) in receivers.chunks_exact(8).enumerate() {
                        bench_daily_country(n as u32 + 1, r);
                    }
                }
                _ => {
                    for _ in &events {
                        black_box(Call::enter());
                    }
                }
            });
            if iteration >= 10 {
                times.push(start.elapsed().as_nanos());
            }
        }
        times.sort();
        let operations = match mode {
            "owner_status" | "owner_country" | "clock" => count,
            "relation" => count * 4,
            "mio" | "upkeep_identity" | "literal_copy" => count * 2,
            _ => events.len(),
        };
        println!(
            "midnight_part={mode} countries={count} operations={operations} batch_p50_p95_ns=({}, {}) per_op_p50_p95_ns=({:.2}, {:.2})",
            times[50],
            times[95],
            times[50] as f64 / operations as f64,
            times[95] as f64 / operations as f64
        );
    }
    println!(
        "raw_record_bytes={} events_per_country=13 native_operand_backend=SEH_guarded; guarded_spans: status_owner=5 command_owner=1 stack=1 clock=2 relation=2 mio=2 upkeep_identity=4 literal_inline_copy=1",
        std::mem::size_of::<Record>()
    );
}

#[cfg(any(test, feature = "test-support"))]
pub fn fixture_identities(receiver: usize, kind: usize) -> (Identity, Identity) {
    match kind {
        0..=3 => (
            relation_identity_with(&memory(), receiver),
            relation_identity(receiver),
        ),
        4..=5 => (
            mio_identity_with(&memory(), receiver),
            mio_identity(receiver),
        ),
        _ => (
            upkeep_identity_with(&memory(), receiver),
            upkeep_identity(receiver),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The political status family's forwarded writers are exactly the
    /// entries patched at country appearance reset and the daily update.
    #[test]
    fn forwarded_status_entries_name_their_native_writers() {
        let sites = entry_sites();
        assert_eq!(sites[RESET_COUNTRY_ENTRY].rva, 0x6d_61d0);
        assert_eq!(sites[RESET_COUNTRY_ENTRY].name, native::RESET_COUNTRY.name);
        assert_eq!(sites[POLITICS_DAILY_ENTRY].rva, 0xb9_90b0);
        assert_eq!(sites[POLITICS_DAILY_ENTRY].name, "PoliticsDaily");
        let forwarded = (0..ENTRY_COUNT)
            .filter(|&index| forwarded_status(index, 0).is_some())
            .count();
        assert_eq!(forwarded, 2);
        // The focus scopes the country input family's focus rows follow.
        assert_eq!(sites[FOCUS_ENTRY].rva, 0x2d_a9f0);
        assert_eq!(sites[FOCUS_ENTRY].name, "Focus");
        assert_eq!(sites[FOCUS_REFUND_ENTRY].rva, 0x2c_e790);
        assert_eq!(sites[FOCUS_REFUND_ENTRY].name, "FocusRefund");
    }

    #[test]
    fn worker_lane_handoff_grows_original_five_thousand_country_and_mod_sized_domains() {
        let _guard = staging_test();
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut cursors = Vec::new();
        for count in [5000, 12_000] {
            let log = Log::with_lanes(count, 1, count);
            log.registered.store(count, Ordering::Release);
            let epoch = EPOCH.load(Ordering::Acquire);
            for index in 0..count {
                append_lane(
                    &log,
                    index,
                    Event {
                        country: (index % 5000) as u32,
                        epoch,
                        ..Default::default()
                    },
                    &memory,
                );
            }
            assert!(matches!(
                handoff_from(&log, &mut []),
                Err("politics thread handoff staging short")
            ));
            let handoff =
                handoff_from_buffer(&log, engine_native::walk::Buffer::Growing(&mut cursors))
                    .unwrap();
            assert_eq!(handoff.lanes, count);
            assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
            let mut events = vec![Event::default(); count];
            let drain = handoff.drain(&cursors, &mut events, &mut []).unwrap();
            assert_eq!(drain.copied, count);
            assert_eq!(events[count - 1].country, ((count - 1) % 5000) as u32);
            for (index, event) in events.iter().enumerate() {
                assert_eq!(event.sequence, index as u64 + 1);
            }
        }
    }

    #[test]
    fn worker_lane_handoff_sizes_prepared_successor_before_sealing_original_events() {
        let _guard = staging_test();
        clear_staging();
        resize_checked(8, 1).unwrap();
        let old = log().unwrap();
        let previous = REQUIRED_LANES.load(Ordering::Relaxed);
        REQUIRED_LANES.store(old.lanes.len() + 17, Ordering::Relaxed);
        append_to(
            old,
            Event {
                country: 5000,
                ..Default::default()
            },
            &engine_native::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
        );
        resize_checked(old.entries, old.key_stride).unwrap();
        let offered = OFFERED.load(Ordering::Acquire);
        let mut cursors = Vec::new();
        ENABLED.store(true, Ordering::SeqCst);
        let sealed = handoff_growing(&mut cursors).unwrap();
        ENABLED.store(false, Ordering::SeqCst);
        assert_eq!(sealed.log, old as *const Log as usize);
        assert_eq!(LOG.load(Ordering::Acquire), offered);
        assert!(cursors.len() >= log().unwrap().lanes.len());
        let mut events = [Event::default()];
        assert_eq!(
            sealed.drain(&cursors, &mut events, &mut []).unwrap().copied,
            1
        );
        assert_eq!(events[0].country, 5000);
        REQUIRED_LANES.store(previous, Ordering::Relaxed);
        clear_staging();
    }

    fn seal_staging() -> (Handoff, Vec<LaneCursor>) {
        let mut cursors =
            vec![LaneCursor::default(); log().unwrap().lanes.len().max(thread_slots())];
        let enabled = ENABLED.swap(true, Ordering::SeqCst);
        let sealed =
            crate::test_support::with_allocations_denied(|| handoff(&mut cursors).unwrap());
        ENABLED.store(enabled, Ordering::SeqCst);
        (sealed, cursors)
    }

    fn clear_staging() {
        ENABLED.store(false, Ordering::SeqCst);
        unsafe {
            release();
        }
    }

    #[test]
    fn balance_faults_name_each_native_read_and_withdraw_coverage_without_allocation() {
        let _guard = staging_test();
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let owner = native::Owner {
            pointer: 0x1000,
            country: 1,
            instance: 0,
            politics: Some(0x2000),
        };
        let before = COUNTERS[3].load(Ordering::Relaxed);
        let epoch = EPOCH.load(Ordering::SeqCst);
        crate::test_support::with_allocations_denied(|| {
            assert!(observed_balance(&memory, owner, PowerCurrency::Political).is_none());
            assert!(observed_balance(&memory, owner, PowerCurrency::Command).is_none());
            assert!(observed_owner(Err("politics status owner mismatch")).is_none());
        });
        assert_eq!(COUNTERS[3].load(Ordering::Relaxed), before + 3);
        assert_eq!(EPOCH.load(Ordering::SeqCst), epoch + 3);
        let value = serde_json::to_value(status()).unwrap();
        for reason in [
            "politics political power balance unreadable, absent or overflowed",
            "politics command power balance unreadable or overflowed",
            "politics status owner mismatch",
        ] {
            assert!(value["native_read_reasons"][reason].as_u64().unwrap() > 0);
        }
    }

    #[test]
    fn premature_release_retains_storage_and_counts_refusal() {
        let _guard = staging_test();
        let before = LOG.load(Ordering::SeqCst);
        let refusals = COUNTERS[8].load(Ordering::Relaxed);
        ENABLED.store(true, Ordering::SeqCst);
        unsafe {
            release();
        }
        assert_eq!(LOG.load(Ordering::SeqCst), before);
        assert_eq!(COUNTERS[8].load(Ordering::Relaxed), refusals + 1);
        ENABLED.store(false, Ordering::SeqCst);
    }

    #[test]
    fn politics_staging_refuses_unrepresentable_extents_without_panicking() {
        assert!(
            std::panic::catch_unwind(|| {
                let _ = Lane::new(usize::MAX, 1);
            })
            .is_ok()
        );
        assert!(Lane::new(usize::MAX, 1).is_err());
        assert!(Lane::new(2, usize::MAX).is_err());
        assert!(Log::with_lane_requirements(usize::MAX, 1, 1, &[]).is_err());
    }

    #[test]
    fn failed_staging_replacement_preserves_published_log_and_retention() {
        let _guard = staging_test();
        clear_staging();
        resize_checked(16, 1).unwrap();
        let pointer = LOG.load(Ordering::Acquire);
        assert_ne!(pointer, 0);
        let retained = LOGS.lock().unwrap().len();
        assert!(resize_checked(usize::MAX, 1).is_err());
        assert_eq!(LOG.load(Ordering::Acquire), pointer);
        assert_eq!(LOGS.lock().unwrap().len(), retained);
    }

    #[test]
    fn resumed_history_epoch_and_staging_growth_preserve_identity_and_keys() {
        let _guard = staging_test();
        clear_staging();
        let prior = EPOCH.load(Ordering::Acquire).max(99);
        restore_epoch(prior).unwrap();
        resize_checked(16, 257).unwrap();
        let first = log().unwrap();
        let stride = first.key_stride;
        let entries = first.entries;
        let epoch = EPOCH.load(Ordering::Acquire);
        assert!(epoch > prior);
        resize_checked(entries.checked_add(1).unwrap(), 0).unwrap();
        assert_eq!(log().unwrap() as *const Log, first as *const Log);
        seal_staging();
        assert!(log().unwrap().key_stride >= stride);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        let pointer = LOG.load(Ordering::Acquire);
        resize_checked(entries + 2, 0).unwrap();
        assert_eq!(LOG.load(Ordering::Acquire), pointer);
        resize_checked(1, stride + 1).unwrap();
        seal_staging();
        assert!(log().unwrap().entries > entries);
        let epoch = EPOCH.load(Ordering::Acquire);
        assert!(restore_epoch(u64::MAX).is_err());
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
    }

    #[test]
    fn overflowing_one_lane_does_not_multiply_other_lane_capacity() {
        let _guard = staging_test();
        let log = Log::with_lanes(16, 1, 4);
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        for _ in 0..33 {
            append_lane(&log, 2, Event::default(), &memory);
        }
        let required: Vec<_> = log
            .lanes
            .iter()
            .map(|lane| {
                lane.required_entries
                    .load(Ordering::Relaxed)
                    .max(lane.slots.len())
            })
            .collect();
        let grown = Log::with_lane_requirements(16, 1, 4, &required).unwrap();
        assert_eq!(required[2], 33);
        assert_eq!(grown.lanes[0].slots.len(), 16);
        assert_eq!(grown.lanes[1].slots.len(), 4);
        assert!(grown.lanes[2].slots.len() >= 33);
        assert_eq!(grown.lanes[3].slots.len(), 4);
        let mut output = [Event::default(); 33];
        assert_eq!(
            drain_from(&log, &mut output, &mut []),
            Err("politics observation restarted")
        );
        append_lane(&log, 2, Event::default(), &memory);
        assert_eq!(drain_from(&log, &mut output, &mut []).unwrap().copied, 1);
        for _ in 0..33 {
            append_lane(&grown, 2, Event::default(), &memory);
        }
        // Growth admits the whole burst without another refused interval.
        assert_eq!(drain_from(&grown, &mut output, &mut []).unwrap().copied, 33);
    }

    #[test]
    fn lane_pressure_retries_growth_without_more_countries_or_keys() {
        let _guard = staging_test();
        clear_staging();
        resize_checked(16, 1).unwrap();
        let before = log().unwrap();
        let capacity = before.lanes[0].slots.len();
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        for _ in 0..=capacity {
            append_lane(before, 0, Event::default(), &memory);
        }
        let pointer = LOG.load(Ordering::Acquire);
        prepare();
        assert_eq!(LOG.load(Ordering::Acquire), pointer);
        assert_ne!(OFFERED.load(Ordering::Acquire), 0);
        seal_staging();
        assert_ne!(LOG.load(Ordering::Acquire), pointer);
        let after = log().unwrap();
        assert!(after.lanes[0].slots.len() >= capacity * 2);
        assert!(after.entries >= before.entries);
        assert!(after.key_stride >= before.key_stride);
        let pointer = LOG.load(Ordering::Acquire);
        prepare();
        assert_eq!(LOG.load(Ordering::Acquire), pointer);
    }

    #[test]
    fn staging_growth_preserves_5000_country_unsealed_and_already_sealed_records() {
        let _guard = staging_test();
        clear_staging();
        resize_checked(8192, 16).unwrap();
        let old = log().unwrap();
        let epoch = EPOCH.load(Ordering::Acquire);
        let mut source = [0; 32];
        source[..9].copy_from_slice(b"mod_itemx");
        source[16..24].copy_from_slice(&9_u64.to_le_bytes());
        source[24..32].copy_from_slice(&15_u64.to_le_bytes());
        let event = |country| Event {
            country,
            epoch,
            hour: 10,
            identity: Identity {
                keys: [
                    native::Literal {
                        address: 0x1000,
                        len: 9,
                    },
                    native::Literal::default(),
                    native::Literal::default(),
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        {
            let memory = engine_native::memory::SliceMemory {
                base: 0x1000,
                bytes: &source,
            };
            for country in 1..=2500 {
                append_to(old, event(country), &memory);
            }
        }
        let (first, first_cursors) = seal_staging();
        {
            let memory = engine_native::memory::SliceMemory {
                base: 0x1000,
                bytes: &source,
            };
            for country in 2501..=5000 {
                append_to(old, event(country), &memory);
            }
        }
        resize_checked(old.entries + 1, old.key_stride + 1).unwrap();
        let retained_log = LOG.load(Ordering::Acquire);
        let retained_epoch = EPOCH.load(Ordering::Acquire);
        let (second, second_cursors) = seal_staging();
        let fresh = log().unwrap();
        crate::test_support::with_allocations_denied(|| {
            append_to(
                fresh,
                event(5001),
                &engine_native::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &source,
                },
            );
        });
        let (third, third_cursors) = seal_staging();
        source.fill(0);
        let mut output = vec![Event::default(); 2500];
        let mut bytes = vec![0; 2500 * 9];
        for (start, sealed, cursors) in [(1, first, first_cursors), (2501, second, second_cursors)]
        {
            let drained = sealed.drain(&cursors, &mut output, &mut bytes).unwrap();
            assert_eq!(
                (
                    drained.first,
                    drained.cursor,
                    drained.copied,
                    drained.pending
                ),
                ((start - 1) as u64, (start + 2499) as u64, 2500, 2500)
            );
            for (index, event) in output.iter().enumerate() {
                assert_eq!(event.country, start + index as u32);
                assert_eq!(event.sequence, (start + index as u32) as u64);
                assert_eq!(event.epoch, epoch);
                let key = event.identity.keys[0];
                assert_eq!(&bytes[key.address..key.address + key.len], b"mod_itemx");
            }
        }
        let drained = third
            .drain(&third_cursors, &mut output, &mut bytes)
            .unwrap();
        assert_eq!(
            (
                drained.first,
                drained.cursor,
                drained.copied,
                drained.pending
            ),
            (5000, 5001, 1, 1)
        );
        assert_eq!((output[0].country, output[0].sequence), (5001, 5001));
        assert_eq!(retained_log, old as *const Log as usize);
        assert_ne!(fresh as *const Log, old as *const Log);
        assert_eq!(retained_epoch, epoch);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        clear_staging();
    }

    #[test]
    fn stale_publisher_routes_during_closure_before_base_and_current_publication() {
        let _guard = staging_test();
        let old = Log::new(4, 1);
        let fresh = Log::new(8, 1);
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        for country in [1, 2] {
            append_to(
                &old,
                Event {
                    country,
                    ..Default::default()
                },
                &memory,
            );
        }
        assert!(close_for_handoff(&old, &fresh));
        assert_eq!(fresh.base_cursor.load(Ordering::Acquire), 0);
        crate::test_support::with_allocations_denied(|| {
            append_to(
                &old,
                Event {
                    country: 3,
                    ..Default::default()
                },
                &memory,
            );
        });
        assert_eq!(fresh.base_cursor.load(Ordering::Acquire), 2);
        assert_eq!(fresh.cursor.load(Ordering::Acquire), 1);
        // Complete the boundary after the stale emitter has already published.
        fresh.base_cursor.store(old.frontier(), Ordering::Release);
        let mut output = [Event::default(); 3];
        let drained = drain_from(&old, &mut output, &mut []).unwrap();
        assert_eq!((drained.first, drained.cursor, drained.copied), (0, 2, 2));
        assert_eq!((output[0].country, output[1].country), (1, 2));
        let drained = drain_from(&fresh, &mut output, &mut []).unwrap();
        assert_eq!((drained.first, drained.cursor, drained.copied), (2, 3, 1));
        assert_eq!((output[0].country, output[0].sequence), (3, 3));
    }

    #[test]
    fn publishing_overlap_defers_growth_without_losing_the_offer_or_epoch() {
        let _guard = staging_test();
        clear_staging();
        resize_checked(8, 1).unwrap();
        let old = log().unwrap();
        let lease = publishing(old).unwrap();
        resize_checked(old.entries + 1, old.key_stride + 1).unwrap();
        let offered = OFFERED.load(Ordering::Acquire);
        let epoch = EPOCH.load(Ordering::Acquire);
        seal_staging();
        assert_eq!(LOG.load(Ordering::Acquire), old as *const Log as usize);
        assert_eq!(OFFERED.load(Ordering::Acquire), offered);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        drop(lease);
        seal_staging();
        assert_eq!(LOG.load(Ordering::Acquire), offered);
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
        clear_staging();
    }

    #[test]
    fn prepared_growth_preserves_independent_entry_and_key_floors() {
        let _guard = staging_test();
        clear_staging();
        resize_checked(8, 8).unwrap();
        let old = log().unwrap();
        resize_checked(old.entries + 1, 0).unwrap();
        let first = unsafe { &*(OFFERED.load(Ordering::Acquire) as *const Log) };
        resize_checked(0, old.key_stride + 1).unwrap();
        let combined = unsafe { &*(OFFERED.load(Ordering::Acquire) as *const Log) };
        assert!(combined.entries >= first.entries);
        assert!(combined.key_stride > old.key_stride);
        let offered = OFFERED.load(Ordering::Acquire);
        prepare();
        assert_eq!(OFFERED.load(Ordering::Acquire), offered);
        seal_staging();
        assert_eq!(LOG.load(Ordering::Acquire), offered);
        clear_staging();
    }

    #[test]
    fn larger_thread_domain_waits_for_prepared_frame_cursors() {
        let _guard = staging_test();
        clear_staging();
        resize_checked(8, 1).unwrap();
        let old = log().unwrap();
        let previous = REQUIRED_LANES.load(Ordering::Relaxed);
        REQUIRED_LANES.store(old.lanes.len() + 1, Ordering::Relaxed);
        resize_checked(old.entries, old.key_stride).unwrap();
        let offered = OFFERED.load(Ordering::Acquire);
        let mut small = vec![LaneCursor::default(); old.lanes.len()];
        ENABLED.store(true, Ordering::SeqCst);
        crate::test_support::with_allocations_denied(|| handoff(&mut small).unwrap());
        ENABLED.store(false, Ordering::SeqCst);
        assert_eq!(LOG.load(Ordering::Acquire), old as *const Log as usize);
        assert_eq!(OFFERED.load(Ordering::Acquire), offered);
        seal_staging();
        assert_eq!(LOG.load(Ordering::Acquire), offered);
        REQUIRED_LANES.store(previous, Ordering::Relaxed);
        clear_staging();
    }

    #[test]
    fn staging_growth_keeps_real_fault_epoch_refusals() {
        let _guard = staging_test();
        clear_staging();
        resize_checked(8, 1).unwrap();
        let old = log().unwrap();
        append_to(
            old,
            Event::default(),
            &engine_native::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
        );
        let (sealed, cursors) = seal_staging();
        resize_checked(old.entries + 1, 0).unwrap();
        invalidate();
        let mut output = [Event::default()];
        assert_eq!(
            sealed.drain(&cursors, &mut output, &mut []),
            Err("politics observation restarted")
        );
        seal_staging();
        let fresh = log().unwrap();
        append_to(
            fresh,
            Event::default(),
            &engine_native::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
        );
        let (sealed, cursors) = seal_staging();
        assert_eq!(
            sealed.drain(&cursors, &mut output, &mut []).unwrap().copied,
            1
        );
        clear_staging();
    }

    unsafe extern "system-unwind" fn shim_fixture(
        a: usize,
        b: usize,
        c: usize,
        d: usize,
        fifth: usize,
    ) -> usize {
        if a == usize::MAX {
            panic!("native unwind fixture");
        }
        a.wrapping_add(b)
            .wrapping_add(c)
            .wrapping_add(d)
            .wrapping_add(fifth)
    }
    #[test]
    fn entry_shim_preserves_args_result_and_unwinds_active_and_scope() {
        let _guard = staging_test();
        let index = 10
            + native::SCOPES
                .iter()
                .position(|site| site.name == "OnAction")
                .unwrap();
        ORIGINAL[index].store(shim_fixture as *const () as usize, Ordering::Release);
        let callback: NativeFunction = unsafe { std::mem::transmute(callbacks()[index]) };
        ENABLED.store(true, Ordering::SeqCst);
        let previous = Context {
            government: GovernmentCause::CivilWar,
            ..Default::default()
        };
        CONTEXT.set(previous);
        let before = ACTIVE.load(Ordering::SeqCst);
        assert_eq!(unsafe { callback(1, 2, 3, 4, 0xabcdef00) }, 0xabcdef0a);
        assert_eq!(CONTEXT.get().government, previous.government);
        let epoch = EPOCH.load(Ordering::SeqCst);
        let result = std::panic::catch_unwind(|| unsafe { callback(usize::MAX, 0, 0, 0, 0) });
        assert!(result.is_err());
        assert_eq!(ACTIVE.load(Ordering::SeqCst), before);
        assert_eq!(CONTEXT.get().government, previous.government);
        assert_ne!(EPOCH.load(Ordering::SeqCst), epoch);
        ENABLED.store(false, Ordering::SeqCst);
        CONTEXT.set(Context::default());
    }

    #[test]
    #[ignore = "release per-call production shared dispatcher shim profile; no game"]
    fn politics_shared_call_profile() {
        let _guard = staging_test();
        let index = 10
            + native::SCOPES
                .iter()
                .position(|site| site.name == "OnAction")
                .unwrap();
        use windows_sys::Win32::System::{
            Diagnostics::Debug::FlushInstructionCache,
            Memory::{
                MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_READWRITE,
                VirtualAlloc, VirtualFree, VirtualProtect,
            },
            Threading::GetCurrentProcess,
        };
        let code = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                4096,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        assert!(!code.is_null());
        unsafe {
            std::ptr::write_bytes(code, 0x90, 16);
            *code.cast::<u8>().add(16) = 0xc3;
        }
        let mut previous = 0;
        assert_ne!(
            unsafe { VirtualProtect(code, 4096, PAGE_EXECUTE_READ, &mut previous) },
            0
        );
        assert_ne!(
            unsafe { FlushInstructionCache(GetCurrentProcess(), code, 17) },
            0
        );
        let mut hook = unsafe { Hook::new(code as usize, callbacks()[index], [0x90; 16]) }.unwrap();
        ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
        let function: NativeFunction = unsafe { std::mem::transmute(code) };
        for mode in ["unhooked", "shim_disabled", "shim_observed"] {
            ENABLED.store(mode == "shim_observed", Ordering::SeqCst);
            if mode == "shim_disabled" {
                assert!(hook.set_enabled(true).unwrap());
            }
            let mut times = Vec::new();
            for iteration in 0..110 {
                let start = std::time::Instant::now();
                for _ in 0..100_000 {
                    std::hint::black_box(unsafe { function(1, 2, 3, 4, 5) });
                }
                if iteration >= 10 {
                    times.push(start.elapsed().as_nanos());
                }
            }
            times.sort();
            println!(
                "shared_call={mode} function=OnAction callers=70 calls=100000 per_call_p50_p95_ns=({:.2},{:.2})",
                times[50] as f64 / 100_000.,
                times[95] as f64 / 100_000.
            );
        }
        ENABLED.store(false, Ordering::SeqCst);
        assert!(hook.set_enabled(false).unwrap());
        drop(hook);
        unsafe {
            VirtualFree(code, 0, MEM_RELEASE);
        }
    }

    #[test]
    fn staging_shortage_retains_events_and_epoch_loss_recovers_past_holes() {
        let _guard = staging_test();
        let log = Log::new(2, 1);
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        append_to(
            &log,
            Event {
                country: 5001,
                ..Default::default()
            },
            &memory,
        );
        append_to(
            &log,
            Event {
                country: 5002,
                ..Default::default()
            },
            &memory,
        );
        let mut output = [Event::default(); 1];
        let mut bytes = [0; 1];
        let first = drain_from(&log, &mut output, &mut bytes).unwrap();
        assert_eq!(
            (first.copied, first.pending, output[0].country),
            (1, 2, 5001)
        );
        assert_eq!(drain_from(&log, &mut output, &mut bytes).unwrap().copied, 1);
        assert_eq!(output[0].country, 5002);
        for _ in 0..3 {
            append_to(&log, Event::default(), &memory);
        }
        assert_eq!(
            drain_from(&log, &mut output, &mut bytes),
            Err("politics observation restarted")
        );
        append_to(
            &log,
            Event {
                country: 7001,
                ..Default::default()
            },
            &memory,
        );
        assert_eq!(drain_from(&log, &mut output, &mut bytes).unwrap().copied, 1);
        assert_eq!(output[0].country, 7001);
    }
    #[test]
    fn scoped_keys_are_copied_before_native_deletion_and_never_truncated() {
        let _guard = staging_test();
        let mut source = [0u8; 32];
        source[..9].copy_from_slice(b"mod_itemx");
        source[16..24].copy_from_slice(&9u64.to_le_bytes());
        source[24..32].copy_from_slice(&15u64.to_le_bytes());
        let memory = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &source,
        };
        let log = Log::new(2, 32);
        let event = Event {
            identity: Identity {
                keys: [
                    native::Literal {
                        address: 0x1000,
                        len: 9,
                    },
                    native::Literal::default(),
                    native::Literal::default(),
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        append_to(&log, event, &memory);
        source.fill(0);
        let mut output = [Event::default(); 2];
        let mut bytes = [0u8; 32];
        assert_eq!(drain_from(&log, &mut output, &mut bytes).unwrap().copied, 1);
        assert_eq!(&bytes[..9], b"mod_itemx");
    }

    #[test]
    fn thread_lanes_merge_in_native_sequence_and_wait_for_unpublished_predecessors() {
        let _guard = staging_test();
        let log = Log::with_lanes(16, 1, 2);
        std::thread::scope(|scope| {
            for country in [5001, 5002] {
                let log = &log;
                scope.spawn(move || {
                    let memory = engine_native::memory::SliceMemory {
                        base: 0,
                        bytes: &[],
                    };
                    for _ in 0..4 {
                        append_to(
                            log,
                            Event {
                                country,
                                ..Default::default()
                            },
                            &memory,
                        );
                    }
                });
            }
        });
        let mut output = [Event::default(); 8];
        let mut bytes = [];
        let drain = drain_from(&log, &mut output, &mut bytes).unwrap();
        assert_eq!(drain.copied, 8);
        assert_eq!(
            output.iter().map(|e| e.sequence).collect::<Vec<_>>(),
            (1..=8).collect::<Vec<_>>()
        );
        assert_eq!(output.iter().filter(|e| e.country == 5001).count(), 4);
        let log = Log::with_lanes(2, 1, 2);
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        append_lane(&log, 0, Event::default(), &memory);
        append_lane(&log, 1, Event::default(), &memory);
        let partial = [
            LaneCursor {
                cursor: 0,
                rejected: 0,
            },
            LaneCursor {
                cursor: 1,
                rejected: 0,
            },
        ];
        assert_eq!(
            drain_until(
                &log,
                Bound {
                    epoch: EPOCH.load(Ordering::Acquire),
                    cursor: 2,
                    shared_rejected: 0,
                },
                &partial,
                &mut output,
                &mut bytes
            )
            .unwrap()
            .copied,
            0
        );
        assert_eq!(drain_from(&log, &mut output, &mut bytes).unwrap().copied, 2);
    }
    #[test]
    fn targeted_decision_uses_selector_and_definition_as_distinct_operands() {
        let _guard = staging_test();
        let mut definition = vec![0u8; engine_decisions::cdecision::TOKEN + 32];
        definition[engine_decisions::cdecision::TOKEN..engine_decisions::cdecision::TOKEN + 8]
            .copy_from_slice(b"decision");
        definition
            [engine_decisions::cdecision::TOKEN + 16..engine_decisions::cdecision::TOKEN + 24]
            .copy_from_slice(&8u64.to_le_bytes());
        definition
            [engine_decisions::cdecision::TOKEN + 24..engine_decisions::cdecision::TOKEN + 32]
            .copy_from_slice(&15u64.to_le_bytes());
        definition[8..12].copy_from_slice(&42i32.to_le_bytes());
        let mut selector = vec![0u8; definition.len()];
        selector[engine_decisions::cdecision::TOKEN + 24..engine_decisions::cdecision::TOKEN + 32]
            .copy_from_slice(&15u64.to_le_bytes());
        for (country, state) in [(5001i32, 0u32), (0, 312)] {
            selector[8..12].copy_from_slice(&country.to_le_bytes());
            selector[12..16].copy_from_slice(&state.to_le_bytes());
            let identity = context_identity(
                Scope::TargetedDecision,
                0,
                selector.as_ptr() as usize,
                definition.as_ptr() as usize,
                Context::default(),
            )
            .identity;
            assert_eq!(
                identity.keys[0].address,
                definition.as_ptr() as usize + engine_decisions::cdecision::TOKEN
            );
            assert_eq!(identity.keys[0].len, 8);
            assert_eq!(identity.keys[2], identity.keys[0]);
            assert_eq!(identity.value(2), (country > 0).then_some(country as u32));
            assert_eq!(identity.value(3), (country <= 0).then_some(state));
        }
    }

    #[test]
    fn ability_cost_only_updates_the_native_activation_charge_caller() {
        let _guard = staging_test();
        let index = 10
            + native::SCOPES
                .iter()
                .position(|site| site.name == "AbilityCost")
                .unwrap();
        let previous = ORIGINAL[index].swap(shim_fixture as *const () as usize, Ordering::Release);
        PENDING.set(Some(Pending {
            owner: native::Owner {
                pointer: 0,
                country: 5001,
                instance: 1,
                politics: None,
            },
            currency: PowerCurrency::Command,
            before: 40,
            requested: Some(-7),
            cause: PowerCause::AbilityActivation,
            site: PowerSite::ActivateAbility,
            operation: PowerOperation::Subtract,
            identity: Identity::default(),
            emitted: false,
        }));
        ENABLED.store(true, Ordering::SeqCst);
        let family = crate::hook_timing::Family::PoliticsCapture;
        let _sampled = crate::hook_timing::sample_all();
        let timed = crate::hook_timing::counts(family);
        let cost = 11i64;
        for caller in [0x6dd977, 0x6dd979, 0] {
            unsafe {
                entry_body(
                    1,
                    &cost as *const i64 as usize,
                    0,
                    0,
                    0,
                    index,
                    base().wrapping_add(caller),
                    0,
                );
            }
            assert_eq!(PENDING.get().unwrap().requested, Some(-7));
        }
        unsafe {
            entry_body(
                1,
                &cost as *const i64 as usize,
                0,
                0,
                0,
                index,
                base().wrapping_add(0x6dd978),
                0,
            );
        }
        assert_eq!(PENDING.get().unwrap().requested, Some(-11));
        let after = crate::hook_timing::counts(family);
        assert!(after.calls >= timed.calls + 4 && after.sampled >= timed.sampled + 4);
        assert!(after.original > timed.original);
        ENABLED.store(false, Ordering::SeqCst);
        PENDING.set(None);
        ORIGINAL[index].store(previous, Ordering::Release);
    }

    /// Each entry's scope is the native scope of that index, and only the
    /// event feed's seven scopes start an operation.
    #[test]
    fn scope_entries_follow_the_native_scope_order() {
        assert_eq!(SCOPE_ORDER.len(), native::SCOPES.len());
        for (scope, site) in SCOPE_ORDER.iter().zip(native::SCOPES) {
            assert_eq!(format!("{scope:?}"), site.name);
        }
        let started = SCOPE_ORDER
            .iter()
            .filter(|scope| scope.provenance().is_some())
            .count();
        assert_eq!(started, 7);
        assert_eq!(Scope::AddIdea.provenance(), None);
        assert_eq!(
            Scope::OnAction.provenance(),
            Some(crate::war_provenance::PoliticsScope::OnAction)
        );
    }

    #[test]
    fn caller_classification_is_exact_and_does_not_use_time_or_country_control() {
        assert_eq!(caller_cause(0x71b05e), PowerCause::Decision);
        assert_eq!(caller_cause(0x71b05f), PowerCause::NotIdentified);
        assert_eq!(caller_cause(native::DAILY_CP_CALLER), PowerCause::DailyGain);
        let console = government_context(Scope::ConsoleSubject, 0, Context::default());
        assert_eq!(
            government_context(Scope::SubjectGovernment, 0, console).government,
            GovernmentCause::Console
        );
        let context = government_context(Scope::OnAction, 0xb992de, Context::default());
        assert_eq!(context.government, GovernmentCause::ElectionOnAction);
        assert_eq!(
            government_context(Scope::OnAction, 0xb992df, context).government,
            GovernmentCause::NotIdentified
        );
    }

    #[test]
    fn writer_threads_beyond_the_lane_count_lose_nothing_in_one_generation() {
        let _guard = staging_test();
        let required = REQUIRED_LANES.load(Ordering::Relaxed);
        let shared_threads = COUNTERS[11].load(Ordering::Relaxed);
        let log = Log::with_lanes(64, 16, 2);
        let mut source = [0; 32];
        source[..6].copy_from_slice(b"shared");
        source[16..24].copy_from_slice(&6_u64.to_le_bytes());
        source[24..32].copy_from_slice(&15_u64.to_le_bytes());
        std::thread::scope(|scope| {
            for country in 1..=5_u32 {
                let (log, source) = (&log, &source);
                scope.spawn(move || {
                    let memory = engine_native::memory::SliceMemory {
                        base: 0x1000,
                        bytes: source,
                    };
                    for _ in 0..4 {
                        append_to(
                            log,
                            Event {
                                country,
                                identity: Identity {
                                    keys: [
                                        native::Literal {
                                            address: 0x1000,
                                            len: 6,
                                        },
                                        native::Literal::default(),
                                        native::Literal::default(),
                                    ],
                                    ..Default::default()
                                },
                                ..Default::default()
                            },
                            &memory,
                        );
                    }
                });
            }
        });
        // Two threads own the lanes; three publish through the shared lane.
        assert_eq!(log.registered.load(Ordering::Acquire), 5);
        assert!(COUNTERS[11].load(Ordering::Relaxed) >= shared_threads + 3);
        assert!(REQUIRED_LANES.load(Ordering::Relaxed) >= 5);
        let mut cursors = [LaneCursor::default(); 2];
        let handoff = handoff_from(&log, &mut cursors).unwrap();
        assert_eq!(handoff.pending(), 20);
        let mut output = [Event::default(); 20];
        let mut bytes = [0; 20 * 16];
        let drained = handoff.drain(&cursors, &mut output, &mut bytes).unwrap();
        assert_eq!(drained.copied, 20);
        assert_eq!(
            output
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            (1..=20).collect::<Vec<_>>()
        );
        for country in 1..=5 {
            assert_eq!(output.iter().filter(|e| e.country == country).count(), 4);
        }
        for event in output {
            let key = event.identity.keys[0];
            assert_eq!(&bytes[key.address..key.address + key.len], b"shared");
        }
        // Every shared slot is free again for the next interval.
        assert!(log.shared.published(|r| r.sequence).unwrap().is_empty());
        assert!(log.shared.reserve().is_some());
        REQUIRED_LANES.store(required, Ordering::Relaxed);
    }

    #[test]
    fn shared_records_merge_by_sequence_and_wait_for_an_unpublished_predecessor() {
        let _guard = staging_test();
        let log = Log::with_lanes(8, 1, 1);
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let shared = |slot: crate::shared_lane::Reservation<'_, Record>, country| {
            let mut record = fixture_record(&Event {
                country,
                ..Default::default()
            });
            record.sequence = log.cursor.fetch_add(1, Ordering::Relaxed) + 1;
            slot.publish(record);
        };
        // Reserved first, published last: reservation order is not log order.
        let early = log.shared.reserve().unwrap();
        let late = log.shared.reserve().unwrap();
        shared(late, 1);
        append_lane(
            &log,
            0,
            Event {
                country: 2,
                ..Default::default()
            },
            &memory,
        );
        let mut early_record = fixture_record(&Event {
            country: 3,
            ..Default::default()
        });
        early_record.sequence = log.cursor.fetch_add(1, Ordering::Relaxed) + 1;
        append_lane(
            &log,
            0,
            Event {
                country: 4,
                ..Default::default()
            },
            &memory,
        );
        let mut output = [Event::default(); 4];
        // Sequence 3 is taken but unpublished: nothing after it is emitted.
        let drain = drain_from(&log, &mut output, &mut []).unwrap();
        assert_eq!((drain.copied, drain.pending), (2, 4));
        assert_eq!([output[0].country, output[1].country], [1, 2]);
        early.publish(early_record);
        let drain = drain_from(&log, &mut output, &mut []).unwrap();
        assert_eq!(drain.copied, 2);
        assert_eq!([output[0].country, output[1].country], [3, 4]);
        assert!(log.shared.published(|r| r.sequence).unwrap().is_empty());
    }

    #[test]
    fn restarted_interval_withdraws_its_shared_records_and_keeps_later_ones() {
        let _guard = staging_test();
        let log = Log::with_lanes(8, 1, 1);
        let publish = |country| {
            let slot = log.shared.reserve().unwrap();
            let mut record = fixture_record(&Event {
                country,
                ..Default::default()
            });
            record.sequence = log.cursor.fetch_add(1, Ordering::Relaxed) + 1;
            slot.publish(record);
        };
        publish(1);
        let frontier = log.frontier();
        publish(2);
        invalidate();
        let cursors = [LaneCursor::default()];
        let mut output = [Event::default(); 2];
        assert_eq!(
            drain_until(
                &log,
                Bound {
                    epoch: EPOCH.load(Ordering::Acquire),
                    cursor: frontier,
                    shared_rejected: 0,
                },
                &cursors,
                &mut output,
                &mut [],
            ),
            Err("politics observation restarted")
        );
        // Only the record past the withdrawn frontier remains.
        let pending = log.shared.published(|r| r.sequence).unwrap();
        assert_eq!(pending.iter().map(|&(s, _)| s).collect::<Vec<_>>(), [2]);
        let drain = drain_from(&log, &mut output, &mut []).unwrap();
        assert_eq!((drain.copied, output[0].country), (1, 2));
    }

    #[test]
    fn exhausted_shared_lane_is_a_named_refusal_that_sizes_the_next_generation() {
        let _guard = staging_test();
        clear_staging();
        resize_checked(4, 1).unwrap();
        let log = log().unwrap();
        let capacity = log.shared.len();
        let overflows = COUNTERS[1].load(Ordering::Relaxed);
        let refusals = COUNTERS[12].load(Ordering::Relaxed);
        let held: Vec<_> = (0..capacity)
            .map(|_| log.shared.reserve().unwrap())
            .collect();
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        crate::test_support::with_allocations_denied(|| {
            append_raw_shared(
                log,
                Record::default(),
                [native::Literal::default(); 3],
                &memory,
            );
        });
        assert_eq!(COUNTERS[12].load(Ordering::Relaxed), refusals + 1);
        assert!(COUNTERS[1].load(Ordering::Relaxed) > overflows);
        assert!(log.shared.required_entries() > capacity);
        let status = serde_json::to_value(status()).unwrap();
        assert!(status["shared_lane_refusals"].as_u64().unwrap() > refusals);
        drop(held);
        prepare();
        assert_ne!(OFFERED.load(Ordering::Acquire), 0);
        seal_staging();
        assert!(super::log().unwrap().shared.len() > capacity);
        clear_staging();
    }
}
