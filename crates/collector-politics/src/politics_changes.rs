use engine_sites::Rva;
use crate::{
    hook::{Hook, Registers, Tap},
    shared_lane::SharedLane,
};
use collector_primitives::spill_stack::SpillStack;
use engine_allocation as allocation;
use engine_events::politics_sites as native;
use engine_native_memory::Memory;
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
const ELECTION_ON_ACTION_RETURN: Rva = Rva(0xbfb8ce);
#[doc = " The daily relation and party boost cost calls return here with RDX the cost output."]
const DAILY_RELATION_COST_RETURN: Rva = Rva(0xbfcf03);
const MIO_RESEARCH_COST_RETURN: Rva = Rva(0xed4fb9);
const MIO_PRODUCTION_COST_RETURN: Rva = Rva(0xed4fd2);
const ABILITY_ACTIVATION_COST_RETURN: Rva = Rva(0x7320f8);
const NESTED_ABILITY_RETURNS: [Rva; 4] =
    [Rva(0x71b405), Rva(0x71b9a9), Rva(0x732467), Rva(0x732c4f)];
const LEADER_DEPLOYMENT_RETURN: Rva = Rva(0xc71212);
const DAILY_COST_RETURN: Rva = Rva(0xbfb952);
#[doc = " The scripted diplomacy action's handler, compared with the action's vtable slot."]
const SCRIPTED_DIPLOMACY_HANDLER: Rva = Rva(0xb03bc0);
const CLASSIFIED: [Rva; 16] = [
    native::DAILY_PP_CALLER,
    native::DAILY_CP_CALLER,
    native::TIMED_PP_CALLER,
    engine_decisions::cdecision::MISSION_COMPLETION_RETURN_RVA,
    ELECTION_ON_ACTION_RETURN,
    DAILY_RELATION_COST_RETURN,
    MIO_RESEARCH_COST_RETURN,
    MIO_PRODUCTION_COST_RETURN,
    ABILITY_ACTIVATION_COST_RETURN,
    NESTED_ABILITY_RETURNS[0],
    NESTED_ABILITY_RETURNS[1],
    NESTED_ABILITY_RETURNS[2],
    NESTED_ABILITY_RETURNS[3],
    LEADER_DEPLOYMENT_RETURN,
    DAILY_COST_RETURN,
    SCRIPTED_DIPLOMACY_HANDLER,
];
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
static ACTIVE_GATE: collector_primitives::asymmetric::FlightGate =
    collector_primitives::asymmetric::FlightGate::new(0);
pub fn active() -> usize {
    ACTIVE_GATE.active()
}
static LOG_SERIAL: AtomicU64 = AtomicU64::new(1);
static LOG: AtomicUsize = AtomicUsize::new(0);
static OFFERED: AtomicUsize = AtomicUsize::new(0);
#[allow(clippy::vec_box)]
static LOGS: Mutex<Vec<Box<[Log]>>> = Mutex::new(Vec::new());
#[allow(
    clippy::vec_box,
    reason = "superseded logs stay boxed until dropped off the capture path"
)]
static RETIRED_LOGS: Mutex<Vec<Box<[Log]>>> = Mutex::new(Vec::new());
static UNHOOKED: OnceLock<String> = OnceLock::new();
static COUNTERS: [AtomicU64; 13] = [const { AtomicU64::new(0) }; 13];
const FAILURE_NAMES: [&str; 41] = [
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
    SET_POWER_NESTED,
    SET_POWER_NO_FRAME,
];
static FAILURES: [AtomicU64; FAILURE_NAMES.len()] =
    [const { AtomicU64::new(0) }; FAILURE_NAMES.len()];
static UNLISTED_FAILURE: collector_primitives::first_reason::FirstReason =
    collector_primitives::first_reason::FirstReason::new();
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

struct Lane {
    slots: Box<[UnsafeCell<Record>]>,
    bytes: Box<[UnsafeCell<u8>]>,
    write: UnsafeCell<u64>,
    published: AtomicU64,
    consumed: AtomicU64,
    spilled: AtomicU64,
    consumed_spilled: AtomicU64,
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
            spilled: AtomicU64::new(0),
            consumed_spilled: AtomicU64::new(0),
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
    shared: SharedLane<Record>,
    pushed: SpillStack<(Record, Vec<u8>)>,
    spill: Mutex<Vec<(Record, Vec<u8>)>>,
    registered: AtomicUsize,
    entries: usize,
    key_stride: usize,
    publishers: AtomicUsize,
    next: AtomicUsize,
    base_cursor: AtomicU64,
    cursor: Padded<AtomicU64>,
    consumed: AtomicU64,
    consumed_epoch: AtomicU64,
    drained: AtomicBool,
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
        let secondary = entries.div_ceil(lanes.max(1)).min(8);
        let shared_initial = entries.div_ceil(lanes.max(1)).min(32);
        let shared = SharedLane::new(shared_initial.max(shared), keys)?;
        let mut storage = allocation::vec(lanes)?;
        for n in 0..lanes {
            storage.push(Lane::new(
                (if n == 0 { entries } else { secondary })
                    .max(required.get(n).copied().unwrap_or(0)),
                keys,
            )?);
        }
        Ok(Self {
            serial: LOG_SERIAL.fetch_add(1, Ordering::Relaxed),
            lanes: allocation::boxed_slice(storage)?,
            shared,
            pushed: SpillStack::new(),
            spill: Mutex::new(Vec::new()),
            registered: AtomicUsize::new(0),
            entries,
            key_stride: keys,
            publishers: AtomicUsize::new(0),
            next: AtomicUsize::new(0),
            base_cursor: AtomicU64::new(0),
            cursor: Padded(AtomicU64::new(0)),
            consumed: AtomicU64::new(0),
            consumed_epoch: AtomicU64::new(EPOCH.load(Ordering::Acquire)),
            drained: AtomicBool::new(false),
        })
    }
    fn frontier(&self) -> u64 {
        self.base_cursor.load(Ordering::Acquire) + self.cursor.load(Ordering::Acquire)
    }
    fn consumed_frontier(&self) -> u64 {
        self.base_cursor.load(Ordering::Acquire) + self.consumed.load(Ordering::Acquire)
    }
    #[cfg(test)]
    fn spill_pending(&self) -> usize {
        let spill = self
            .spill
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        spill.len().saturating_add(self.pushed.len())
    }
    fn held(&self) -> std::sync::MutexGuard<'_, Vec<(Record, Vec<u8>)>> {
        let mut spill = self
            .spill
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.pushed.is_empty() {
            match self.pushed.take_into(&mut spill) {
                Ok(()) => spill.sort_unstable_by_key(|(record, _)| record.sequence),
                Err(_) => {
                    COUNTERS[10].fetch_add(1, Ordering::Relaxed);
                    invalidate_epoch();
                }
            }
        }
        spill
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
            return Some(unsafe { &*(lane as *const Lane) });
        }
        lane_index(log).map(|index| &log.lanes[index])
    })
}

struct Call(Option<collector_primitives::asymmetric::FlightGuard<'static>>);
impl Call {
    #[inline(always)]
    fn enter() -> Self {
        let guard = DEPTH.with(|depth| {
            let current = depth.get();
            depth.set(current + 1);
            (current == 0).then(|| ACTIVE_GATE.enter(false))
        });
        Self(guard)
    }
}
impl Drop for Call {
    #[inline(always)]
    fn drop(&mut self) {
        DEPTH.with(|depth| {
            depth.set(depth.get() - 1);
        });
        drop(self.0.take());
    }
}
/// # Safety
/// Caller must own session attach before enabling politics hooks (or hold `staging_test()` in an
/// isolated test) when restoring the process-wide politics observation epoch.
pub unsafe fn restore_epoch(previous: u64) -> Result<(), &'static str> {
    let epoch = previous
        .checked_add(1)
        .ok_or("politics observation epoch exhausted")?;
    EPOCH.store(epoch, Ordering::Release);
    Ok(())
}

fn invalidate_epoch() {
    #[cfg(any(test, feature = "test-support"))]
    if !STAGING_OWNER.get() {
        return;
    }
    crate::event_feed_observer::decisions::political_balance_lost();
    EPOCH.fetch_add(1, Ordering::SeqCst);
}
/// # Safety
/// Caller must own the active session's capture, recovery, or publication failure path (or hold
/// `staging_test()` in an isolated test) when invalidating live politics observation.
pub unsafe fn invalidate() {
    invalidate_epoch();
}
fn failed(reason: &'static str) {
    if let Some(index) = FAILURE_NAMES.iter().position(|name| *name == reason) {
        FAILURES[index].fetch_add(1, Ordering::Relaxed);
    } else {
        UNLISTED_FAILURE.record(reason);
        UNLISTED_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    COUNTERS[3].fetch_add(1, Ordering::Relaxed);
    invalidate_epoch();
}
fn memory() -> engine_native_memory::memory::LiveMemory {
    unsafe { engine_native_memory::memory::LiveMemory::new() }
}
fn base() -> usize {
    BASE.load(Ordering::Relaxed)
}

fn daily_cost_caller(caller: Option<u64>) -> bool {
    caller.is_some_and(|caller| engine_sites::at(base(), DAILY_COST_RETURN) == Some(caller as usize))
}
fn read<T: engine_native_memory::memory::Raw>(address: usize) -> Option<T> {
    engine_native_memory::read::<T>(&memory(), address)
}
fn operand_memory() -> native::NativeOperands {
    // SAFETY: callers below read only the supported synchronous native operands.
    unsafe { native::NativeOperands::new() }
}
#[inline(always)]
fn operand<T: engine_native_memory::memory::Raw>(address: usize) -> Option<T> {
    engine_native_memory::read::<T>(&operand_memory(), address)
}
fn operand_hour() -> Option<i32> {
    let state =
        operand::<u64>(engine_sites::at(base(), engine_native::cgame_state::SINGLETON_RVA)?)? as usize;
    operand::<i32>(state.checked_add(engine_native::cgame_state::CURRENT_HOUR)?)
}
fn event_hour() -> Option<i32> {
    let state =
        read::<u64>(engine_sites::at(base(), engine_native::cgame_state::SINGLETON_RVA)?)? as usize;
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
/// # Safety
/// Run only while politics staging remains live for the active session (or an isolated test holding
/// `staging_test()`) and [`release`] is excluded; concurrent growth calls are serialized by `LOGS`.
pub unsafe fn resize(entries: usize, keys: usize) {
    engine_allocation::advisory(|| resize_storage(entries, keys));
}

fn resize_storage(entries: usize, keys: usize) {
    if resize_checked(entries, keys).is_err() {
        COUNTERS[10].fetch_add(1, Ordering::Relaxed);
        invalidate_epoch();
    }
}
fn resize_checked(entries: usize, keys: usize) -> io::Result<()> {
    let retired = {
        let mut retired = RETIRED_LOGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *retired)
    };
    drop(retired);
    let mut logs = LOGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let current_ptr = LOG.load(Ordering::Acquire);
    let offered_ptr = OFFERED.load(Ordering::Acquire);
    if active() == 0 {
        logs.retain(|entry| {
            let ptr = entry.as_ptr() as usize;
            if ptr == current_ptr || ptr == offered_ptr {
                return true;
            }
            let log = &entry[0];
            !(log.publishers.load(Ordering::Acquire) == usize::MAX
                && log.drained.load(Ordering::Acquire))
        });
    }
    let offered = offered_ptr as *const Log;
    let offered = (!offered.is_null()).then(|| unsafe { &*offered });
    let current = (current_ptr != 0).then(|| unsafe { &*(current_ptr as *const Log) });
    let sources = [current, offered];
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

/// # Safety
/// Run only while politics staging remains live for the active session (or an isolated test holding
/// `staging_test()`) and [`release`] is excluded; concurrent calls are serialized by `LOGS`.
pub unsafe fn prepare() {
    if let Some(log) = log() {
        // SAFETY: Caller guarantees live politics staging with `release` excluded.
        unsafe { resize(log.entries, log.key_stride) };
    }
}
/// Release staging after disabled hooks and all callback/capture workers are
/// quiescent. No published log reference may survive this call.
///
/// # Safety
/// The caller has disabled hooks, checked their executable ranges and ACTIVE,
/// and joined all workers that can call drain/resize/status.
pub unsafe fn release() {
    if ENABLED.load(Ordering::SeqCst) || active() != 0 {
        COUNTERS[8].fetch_add(1, Ordering::Relaxed);
        return;
    }
    LOG.store(0, Ordering::Release);
    OFFERED.store(0, Ordering::Release);
    LOGS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    RETIRED_LOGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    CONSERVATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .examples
        .fill(None);
}
#[inline(always)]
#[allow(clippy::too_many_arguments)]
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
    POWER_RECORDS.set(POWER_RECORDS.get().wrapping_add(1));
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
#[allow(clippy::too_many_arguments)]
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
fn append_raw_shared(
    log: &Log,
    mut record: Record,
    mut keys: [native::Literal; 3],
    memory: &impl Memory,
) {
    if key_bytes(&keys) > log.key_stride {
        REQUIRED_KEYS.fetch_max(key_bytes(&keys), Ordering::Relaxed);
        spill(log, record, keys, memory);
        return;
    }
    let Some(mut slot) = log.shared.try_reserve() else {
        COUNTERS[12].fetch_add(1, Ordering::Relaxed);
        spill(log, record, keys, memory);
        return;
    };
    if copy_keys(&mut keys, slot.stride(), slot.bytes(), memory).is_err() {
        return;
    }
    record.sequence =
        log.base_cursor.load(Ordering::Acquire) + log.cursor.fetch_add(1, Ordering::Relaxed) + 1;
    record.keys = keys.map(|key| [key.address as u32, key.len as u32]);
    slot.publish(record);
}
fn key_bytes(keys: &[native::Literal; 3]) -> usize {
    keys.iter()
        .map(|key| key.len)
        .fold(0, usize::saturating_add)
}

#[cold]
fn spill(log: &Log, record: Record, mut keys: [native::Literal; 3], memory: &impl Memory) {
    COUNTERS[1].fetch_add(1, Ordering::Relaxed);
    let needed = key_bytes(&keys);
    let mut bytes = Vec::new();
    let reserved = allocation::advisory(|| allocation::reserve_exact(&mut bytes, needed))
        .and_then(|()| {
            bytes.resize(needed, 0);
            log.pushed.reserve((record, bytes))
        });
    let Ok(mut spilled) = reserved else {
        COUNTERS[10].fetch_add(1, Ordering::Relaxed);
        invalidate_epoch();
        return;
    };
    let (record, bytes) = &mut *spilled;
    if copy_keys(&mut keys, needed, bytes.as_mut_ptr(), memory).is_err() {
        return;
    }
    record.sequence =
        log.base_cursor.load(Ordering::Acquire) + log.cursor.fetch_add(1, Ordering::Relaxed) + 1;
    record.keys = keys.map(|key| [key.address as u32, key.len as u32]);
    spilled.push();
}

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
#[cfg(test)]
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
    let cursor = unsafe { *lane.write.get() };
    let demand = cursor
        .wrapping_sub(lane.consumed.load(Ordering::Acquire))
        .saturating_add(
            lane.spilled
                .load(Ordering::Relaxed)
                .wrapping_sub(lane.consumed_spilled.load(Ordering::Acquire)),
        )
        .saturating_add(1);
    if demand > lane.slots.len() as u64 {
        lane.spilled.fetch_add(1, Ordering::Release);
        lane.required_entries.fetch_max(
            (demand as usize).max(lane.slots.len().saturating_mul(2)),
            Ordering::Relaxed,
        );
        spill(log, record, keys, memory);
        return;
    }
    if key_bytes(&keys) > log.key_stride {
        REQUIRED_KEYS.fetch_max(key_bytes(&keys), Ordering::Relaxed);
        spill(log, record, keys, memory);
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
    spilled: u64,
}

#[derive(Clone, Copy)]
pub struct Handoff {
    log: usize,
    epoch: u64,
    cursor: u64,
    lanes: usize,
    shared_rejected: u64,
    final_for_log: bool,
}
pub(crate) fn handoff_growing(cursors: &mut Vec<LaneCursor>) -> Result<Handoff, &'static str> {
    handoff_buffer(engine_native_memory::walk::Buffer::Growing(cursors))
}
fn handoff_buffer(
    mut cursors: engine_native_memory::walk::Buffer<'_, LaneCursor>,
) -> Result<Handoff, &'static str> {
    if UNHOOKED.get().is_some() || !ENABLED.load(Ordering::Acquire) {
        return Err("politics hooks unavailable");
    }
    let log = log().ok_or("politics staging unavailable")?;
    if active() != 0 {
        return handoff_from_buffer(log, cursors, false);
    }
    retire_drained_logs();
    if !cursors.ensure(log.lanes.len())? {
        return handoff_from_buffer(log, cursors, false);
    }
    let offered = OFFERED.load(Ordering::Acquire) as *const Log;
    if offered.is_null() {
        return handoff_from_buffer(log, cursors, false);
    }
    let offered = unsafe { &*offered };
    if !cursors.ensure(offered.lanes.len())? {
        return handoff_from_buffer(log, cursors, false);
    }
    if !close_for_handoff(log, offered) {
        return handoff_from_buffer(log, cursors, false);
    }
    offered.base_cursor.store(log.frontier(), Ordering::Release);
    let sealed = handoff_from_buffer(log, cursors, true);
    LOG.store(offered as *const Log as usize, Ordering::Release);
    let _ = OFFERED.compare_exchange(
        offered as *const Log as usize,
        0,
        Ordering::AcqRel,
        Ordering::Relaxed,
    );
    sealed
}
fn retire_drained_logs() {
    let Ok(mut logs) = LOGS.try_lock() else {
        return;
    };
    if logs.len() <= 1 {
        return;
    }
    let Ok(mut retired) = RETIRED_LOGS.try_lock() else {
        return;
    };
    let current_ptr = LOG.load(Ordering::Acquire);
    let offered_ptr = OFFERED.load(Ordering::Acquire);
    let mut index = 0;
    while index < logs.len() {
        let ptr = logs[index].as_ptr() as usize;
        let done = ptr != current_ptr
            && ptr != offered_ptr
            && logs[index][0].publishers.load(Ordering::Acquire) == usize::MAX
            && logs[index][0].drained.load(Ordering::Acquire);
        if done && retired.try_reserve(1).is_ok() {
            let entry = logs.swap_remove(index);
            retired.push(entry);
        } else {
            index += 1;
        }
    }
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
fn handoff_from_buffer(
    log: &Log,
    mut cursors: engine_native_memory::walk::Buffer<'_, LaneCursor>,
    final_for_log: bool,
) -> Result<Handoff, &'static str> {
    let count = log.registered.load(Ordering::Acquire).min(log.lanes.len());
    if !cursors.ensure(count)? {
        short_handoff();
        return Err("politics thread handoff staging short");
    }
    for (out, lane) in cursors.iter_mut().zip(&log.lanes[..count]) {
        out.cursor = lane.published.load(Ordering::Acquire);
        out.spilled = lane.spilled.load(Ordering::Acquire);
    }
    let shared_rejected = log.shared.rejected();
    Ok(Handoff {
        log: log as *const Log as usize,
        epoch: EPOCH.load(Ordering::Acquire),
        cursor: log.frontier(),
        lanes: count,
        shared_rejected,
        final_for_log,
    })
}
impl Handoff {
    pub(crate) fn pending(self) -> usize {
        let log = unsafe { &*(self.log as *const Log) };
        self.cursor.wrapping_sub(log.consumed_frontier()) as usize
    }
    pub(crate) fn key_capacity(self) -> usize {
        let log = unsafe { &*(self.log as *const Log) };
        let spilled = log
            .held()
            .iter()
            .map(|(_, bytes)| bytes.len())
            .fold(0, usize::saturating_add);
        self.pending()
            .saturating_mul(log.key_stride)
            .saturating_add(spilled)
    }
    /// # Safety
    /// The log captured by this [`Handoff`] must remain live at its original address (`release`
    /// excluded), `cursors` must be the exact unmodified slice populated when this [`Handoff`] was
    /// sealed (covering at least `self.lanes`), and only the single spool consumer (or isolated
    /// fixture) may drain handoffs in strict seal order without concurrent consumption.
    pub unsafe fn drain(
        self,
        cursors: &[LaneCursor],
        output: &mut [Event],
        bytes: &mut [u8],
    ) -> Result<Drained, &'static str> {
        let log = unsafe { &*(self.log as *const Log) };
        let drained = drain_until(
            log,
            Bound {
                epoch: self.epoch,
                cursor: self.cursor,
                shared_rejected: self.shared_rejected,
            },
            &cursors[..self.lanes],
            output,
            bytes,
        );
        if self.final_for_log {
            log.drained.store(true, Ordering::Release);
        }
        drained
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
            spilled: l.spilled.load(Ordering::Acquire),
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
#[derive(Clone, Copy)]
enum Source {
    Lane(usize),
    Shared,
    Spill,
}

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
    let mut spilled = log.held();
    if log.consumed_epoch.swap(epoch, Ordering::AcqRel) != epoch
        || epoch != EPOCH.load(Ordering::Acquire)
    {
        for (lane, end) in log.lanes.iter().zip(cursors) {
            lane.consumed.store(end.cursor, Ordering::Release);
            lane.consumed_spilled
                .store(end.spilled, Ordering::Release);
        }
        log.consumed.store(
            cursor - log.base_cursor.load(Ordering::Acquire),
            Ordering::Release,
        );
        release_through(cursor);
        log.shared.advance(bound.shared_rejected);
        spilled.retain(|(record, _)| record.sequence > cursor);
        return Err("politics observation restarted");
    }
    release_through(first);
    spilled.retain(|(record, _)| record.sequence > first);
    let mut spill_at = 0;
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
    while copied < output.len() {
        let next = log
            .lanes
            .iter()
            .zip(cursors)
            .enumerate()
            .filter(|(n, (_, end))| tails[*n] < end.cursor)
            .map(|(n, (lane, _))| {
                let slot = tails[n] as usize & (lane.slots.len() - 1);
                (n, unsafe { (*lane.slots[slot].get()).sequence })
            })
            .map(|(n, sequence)| (Source::Lane(n), sequence))
            .chain(shared.peek().map(|&(sequence, _)| (Source::Shared, sequence)))
            .chain(
                spilled
                    .get(spill_at)
                    .filter(|(record, _)| record.sequence <= cursor)
                    .map(|(record, _)| (Source::Spill, record.sequence)),
            )
            .min_by_key(|(_, seq)| *seq);
        let Some((n, sequence)) = next else {
            break;
        };
        if sequence != first + copied as u64 + 1 {
            break;
        }
        let (record, source) = match n {
            Source::Lane(n) => {
                let lane = &log.lanes[n];
                let index = tails[n] as usize & (lane.slots.len() - 1);
                (unsafe { *lane.slots[index].get() }, unsafe {
                    std::slice::from_raw_parts(
                        lane.byte_pointer(index, log.key_stride).cast_const(),
                        log.key_stride,
                    )
                })
            }
            Source::Shared => log.shared.record(shared.peek().unwrap().1),
            Source::Spill => {
                let (record, bytes) = &spilled[spill_at];
                (*record, bytes.as_slice())
            }
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
            Source::Lane(n) => tails[n] += 1,
            Source::Shared => log.shared.release(shared.next().unwrap().1),
            Source::Spill => spill_at += 1,
        }
    }
    spilled.drain(..spill_at);
    drop(spilled);
    log.shared.advance(bound.shared_rejected);
    for ((lane, tail), end) in log.lanes.iter().zip(tails).zip(cursors) {
        lane.consumed.store(tail, Ordering::Release);
        if tail == end.cursor {
            lane.consumed_spilled
                .store(end.spilled, Ordering::Release);
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
            invalidate_epoch();
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
                pointer.checked_add(engine_native_parts::cpolitical_status_layout::POLITICAL_POWER)
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
    match address.and_then(|address| engine_native_memory::read(memory, address)) {
        Some(value) => Some(value),
        None => {
            failed(reason);
            None
        }
    }
}

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
        && pending.owner.same_owner(owner)
        && pending.currency == currency
    {
        pending.before = after;
        PENDING.set(Some(pending));
    }
}
fn caller_cause(caller: Option<Rva>) -> PowerCause {
    if caller == Some(native::DAILY_PP_CALLER) || caller == Some(native::DAILY_CP_CALLER) {
        return PowerCause::DailyGain;
    }
    if caller == Some(native::TIMED_PP_CALLER) {
        return PowerCause::TimedActivity;
    }
    let name = caller
        .and_then(|caller| {
            native::CHARGING_CALLERS
                .binary_search_by_key(&caller, |(key, _)| *key)
                .ok()
        })
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

const ENTRY_COUNT: usize = 39;
const TARGETED_DECISION_ENTRY: usize = 21;
const SET_POWER_ENTRY: usize = 38;
const SET_POWER_NESTED: &str = "set political power nested another power writer";
const SET_POWER_NO_FRAME: &str = "set political power commit outside its effect";

#[derive(Clone, Copy)]
struct SetPower {
    owner: native::Owner,
    before: i64,
    raw: i64,
    identity: Identity,
}
std::thread_local! {
    static SET_POWER_FRAME: Cell<*mut Option<SetPower>> = const { Cell::new(std::ptr::null_mut()) };
    static POWER_RECORDS: Cell<u64> = const { Cell::new(0) };
}
fn set_power_published(witness: SetPower, stored: i64) -> (i64, i64, i64) {
    (
        witness.before,
        stored,
        witness.raw.wrapping_sub(witness.before),
    )
}
/// # Safety
/// Only the power entry's detour calls this, on the native thread running the hooked call, with
/// that call's five arguments.
unsafe fn set_power(a: usize, b: usize, c: usize, d: usize, fifth: usize) -> usize {
    struct Frame(*mut Option<SetPower>);
    impl Drop for Frame {
        fn drop(&mut self) {
            SET_POWER_FRAME.set(self.0);
        }
    }
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PoliticsCapture);
    let _active = Call::enter();
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[SET_POWER_ENTRY].load(Ordering::Acquire)) };
    let mut flight = crate::event_feed_observer::WriterFlight::enter();
    let mut staged: Option<SetPower> = None;
    let _frame = Frame(SET_POWER_FRAME.replace(&mut staged as *mut _));
    let records = POWER_RECORDS.get();
    let result = timing.original(|| unsafe { original(a, b, c, d, fifth) });
    drop(_frame);
    if let Some(witness) = staged
        && ENABLED.load(Ordering::Acquire)
    {
        if POWER_RECORDS.get() != records {
            failed(SET_POWER_NESTED);
        } else if let Some(stored) = balance(witness.owner, PowerCurrency::Political) {
            let (before, after, requested) = set_power_published(witness, stored);
            power(
                witness.owner,
                PowerCurrency::Political,
                PowerOperation::Set,
                PowerCause::ScriptEffect,
                PowerSite::SetPoliticalPower,
                before,
                after,
                Some(requested),
                witness.identity,
            );
            resume_pending(witness.owner, PowerCurrency::Political, after);
        }
    }
    flight.returned();
    result
}
const RESET_COUNTRY_ENTRY: usize = 3;
const POLITICS_DAILY_ENTRY: usize = 10;
const FOCUS_ENTRY: usize = 36;
const FOCUS_REFUND_ENTRY: usize = 37;

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
    .chain([native::SET_POLITICAL_POWER_EFFECT])
    .collect()
}

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
entry!(entry_38, 38);
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
        entry_38 as *const () as usize,
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
fn item_read<T: engine_native_memory::memory::Raw>(address: usize) -> Option<T> {
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
    let value = engine_native_memory::read::<u64>(memory, at(pointer, offset)?).or_else(|| {
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
        Scope::AddIdea | Scope::RemoveIdea | Scope::ReplaceIdea => {
            let offset = if scope == Scope::ReplaceIdea {
                0x38
            } else {
                0x30
            };
            if let Some(idea) = at(a, offset).and_then(item_read::<u64>) {
                identity.keys[0] = key(idea as usize, engine_native_parts::cidea::TOKEN);
            }
            identity.action = match scope {
                Scope::AddIdea => PowerAction::Add,
                Scope::RemoveIdea => PowerAction::Remove,
                _ => PowerAction::Replace,
            };
        }
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
                    0x30
                } else {
                    0x38
                },
            );
            identity.action = if scope == Scope::HireAdvisor {
                PowerAction::Add
            } else {
                PowerAction::Replace
            };
        }
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
        Scope::Diplomacy | Scope::ScriptedDiplomacy => {
            identity.set(0, at(a, 8).and_then(item_read::<u32>));
            identity.action = PowerAction::Execute;
            identity.set(
                2,
                at(a, 0x1c).and_then(item_read::<u32>).filter(|id| *id != 0),
            );
            let scripted = scope == Scope::ScriptedDiplomacy
                || read::<u64>(a)
                    .and_then(|vtable| at(vtable as usize, 0x150))
                    .and_then(item_read::<u64>)
                    .is_some_and(|handler| {
                        engine_sites::at(base(), SCRIPTED_DIPLOMACY_HANDLER)
                            == Some(handler as usize)
                    });
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
                        engine_native_parts::cunit_leader_trait::TOKEN_ID,
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
fn government_context(scope: Scope, caller: Option<Rva>, mut context: Context) -> Context {
    let cause = match scope {
        Scope::CoupCompletion => GovernmentCause::CoupCompletion,
        Scope::CivilWar => GovernmentCause::CivilWar,
        Scope::SetPoliticsEffect => GovernmentCause::SetPoliticsEffect,
        Scope::PeaceForceGovernment => GovernmentCause::PeaceForceGovernment,
        Scope::SubjectGovernment => GovernmentCause::SubjectGovernment,
        Scope::ConsoleGovernment | Scope::ConsoleSubject => GovernmentCause::Console,
        Scope::OnAction => {
            context.government = if caller == Some(ELECTION_ON_ACTION_RETURN) {
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
        at(status, engine_native_parts::cpolitical_status_layout::RULING_PARTY).and_then(read::<u64>)
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

#[allow(clippy::too_many_arguments)]
unsafe extern "system-unwind" fn entry_body(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    fifth: usize,
    index: usize,
    caller: usize,
    native_rbp: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    if index == SET_POWER_ENTRY {
        return unsafe { set_power(a, b, c, d, fifth) };
    }
    if index == TARGETED_DECISION_ENTRY {
        let _provenance = crate::war_provenance::politics(
            crate::war_provenance::PoliticsScope::TargetedDecision,
            caller,
        );
        return crate::script_event_observer::targeted_selection(|| unsafe {
            entry_forward(a, b, c, d, fifth, index, caller, native_rbp)
        });
    }
    let mut reset =
        (index == RESET_COUNTRY_ENTRY).then(crate::event_feed_observer::WriterFlight::enter);
    let result = unsafe { entry_forward(a, b, c, d, fifth, index, caller, native_rbp) };
    if let Some(reset) = &mut reset {
        reset.returned();
    }
    result
}
/// # Safety
/// Only the entry stubs call this, on the native thread running the hooked call, with that call's
/// arguments, its site index and the caller's return address.
#[allow(clippy::too_many_arguments)]
unsafe fn entry_forward(
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
    let _status = forwarded_status(index, a);
    if index == RESET_COUNTRY_ENTRY {
        crate::event_feed_observer::reset_delayed_country(a);
    }
    let _inputs = {
        use crate::country_input_changes::{Forward, forwarded};
        match index {
            RESET_COUNTRY_ENTRY => Some(forwarded(Forward::ResetCountry, a, b)),
            FOCUS_ENTRY | FOCUS_REFUND_ENTRY => Some(forwarded(Forward::Focus, a, 0)),
            _ => None,
        }
    };
    let _naval = (index == RESET_COUNTRY_ENTRY).then(|| crate::naval_tree_changes::forwarded(1, a));
    let scope = index.checked_sub(10).map(|scope| SCOPE_ORDER[scope]);
    let _provenance = scope
        .filter(|_| index != TARGETED_DECISION_ENTRY)
        .and_then(Scope::provenance)
        .and_then(|scope| crate::war_provenance::politics(scope, caller));
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[index].load(Ordering::Acquire)) };
    if !ENABLED.load(Ordering::Acquire) {
        return timing.original(|| unsafe { original(a, b, c, d, fifth) });
    }
    let caller = engine_sites::reference(base(), caller);
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
    let previous = restore.context;
    let mut context = context_identity(scope, a, b, c, previous);
    context = government_context(scope, caller, context);
    if scope == Scope::PoliticsDaily {
        DAILY_HOUR.set(operand_hour());
    }
    CONTEXT.set(context);
    match scope {
        Scope::Decision
            if caller != Some(engine_decisions::cdecision::MISSION_COMPLETION_RETURN_RVA) =>
        {
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
        match scope {
            Scope::ImproveRelationDaily | Scope::BoostPartyDaily
                if caller == Some(DAILY_RELATION_COST_RETURN) =>
            {
                if let Some(cost) = operand::<i64>(b) {
                    let identity = relation_identity(a);
                    component(owner, PowerDailyCostKind::Relation, cost, identity);
                } else {
                    failed("politics daily relation cost output unreadable");
                }
            }
            Scope::MioResearchDaily if caller == Some(MIO_RESEARCH_COST_RETURN) => {
                mio_component(owner, a, operand::<i64>(b), PowerDailyCostKind::MioResearch)
            }
            Scope::MioProductionDaily if caller == Some(MIO_PRODUCTION_COST_RETURN) => mio_component(
                owner,
                a,
                operand::<i64>(b),
                PowerDailyCostKind::MioProduction,
            ),
            _ => {}
        }
    }
    if scope == Scope::AbilityCost
        && caller == Some(ABILITY_ACTIVATION_COST_RETURN)
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
            .and_then(|p| engine_native_memory::read::<u32>(memory, p)),
    );
    identity.set(
        2,
        at(relation, engine_native::crelation::SECOND_COUNTRY)
            .and_then(|p| engine_native_memory::read::<u32>(memory, p))
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
        .and_then(|p| engine_native_memory::read::<u32>(memory, p)),
    );
    identity
}
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
    .and_then(|p| engine_native_memory::read::<u64>(memory, p))
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
        .and_then(|p| engine_native_memory::read::<u64>(memory, p))
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
#[cfg(test)]
#[inline(always)]
pub(crate) fn bench_identity(kind: PowerDailyCostKind, receiver: usize) -> Identity {
    if kind == PowerDailyCostKind::Relation {
        relation_identity(receiver)
    } else {
        mio_identity(receiver)
    }
}
#[cfg(test)]
#[inline(always)]
pub(crate) fn bench_upkeep_identity(record: usize) -> Identity {
    upkeep_identity(record)
}

/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
unsafe fn writer(
    timing: &crate::hook_timing::Span,
    original: NativeFunction,
    args: [usize; 5],
    index: usize,
    caller: Option<Rva>,
    context: Context,
) -> usize {
    let [a, b, c, d, fifth] = args;
    flush(false);
    let mut previous = PENDING.get();
    let native_daily =
        caller == Some(native::DAILY_PP_CALLER) || caller == Some(native::DAILY_CP_CALLER);
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
    if caller.is_some_and(|caller| NESTED_ABILITY_RETURNS.contains(&caller))
        && let Some(parent) =
            previous.filter(|parent| owner.is_some_and(|owner| parent.owner.same_owner(owner)))
    {
        identity = parent.identity;
    }
    let mut cause = caller_cause(caller);
    let requested = match index {
        0 | 1 => Some(b as i64),
        2 => before.map(i64::wrapping_neg),
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
    if caller == Some(LEADER_DEPLOYMENT_RETURN)
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
            at(b, engine_native_parts::cstrategic_region::ID).and_then(read::<u32>),
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
                let witness = SetPower {
                    owner,
                    before,
                    raw: registers.rcx as i64,
                    identity: CONTEXT.get().identity,
                };
                let frame = SET_POWER_FRAME.get();
                if frame.is_null() {
                    failed(SET_POWER_NO_FRAME);
                } else {
                    unsafe { *frame = Some(witness) };
                }
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
            if registers.flags & (1 << 6) != 0 {
                if let Some(party) = at(
                    registers.r14,
                    engine_native_parts::cpolitical_status_layout::RULING_PARTY,
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
            if !daily_cost_caller(at(registers.rbp, 0x358).and_then(operand::<u64>)) {
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
                if let Some(define) = engine_sites::at(base(), native::BLOCKADE_COST_DEFINE_RVA)
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
            if !daily_cost_caller(at(registers.rbp, 0x358).and_then(operand::<u64>)) {
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

pub fn refusals() -> impl serde::Serialize {
    let staging_failures = COUNTERS[10].load(Ordering::Relaxed);
    crate::status::status!({
        "staging_allocation_failures": staging_failures,
        "staging_error": (staging_failures != 0).then_some("politics staging allocation failed"),
        "native_read_reasons": crate::status::map(FAILURE_NAMES.into_iter().zip(FAILURES.iter().map(|count| count.load(Ordering::Relaxed))).filter(|(_, count)| *count != 0)),
        "first_unlisted_native_read_reason": UNLISTED_FAILURE.get(),
    })
}
#[cfg(test)]
struct ActiveStatusMeta {
    events: u64,
    spill_pending: usize,
    staging_entries: usize,
    staging_threads: usize,
    shared_lane_entries: usize,
    required_shared_lane_entries: usize,
    staging_key_stride: usize,
}

#[cfg(test)]
struct StatusSnapshot {
    counters: [u64; 13],
    epoch: u64,
    failures: [u64; FAILURE_NAMES.len()],
    unlisted_failures: u64,
    required_keys: usize,
    active: Option<ActiveStatusMeta>,
}

#[cfg(test)]
fn snapshot_status(
    required_entries: &mut Vec<usize>,
    staging_entries: &mut Vec<usize>,
) -> StatusSnapshot {
    required_entries.clear();
    staging_entries.clear();
    let retained = LOGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let counters = COUNTERS
        .each_ref()
        .map(|value| value.load(Ordering::Relaxed));
    let epoch = EPOCH.load(Ordering::SeqCst);
    let failures = FAILURES
        .each_ref()
        .map(|count| count.load(Ordering::Relaxed));
    let unlisted_failures = UNLISTED_FAILURES.load(Ordering::Relaxed);
    let required_keys = REQUIRED_KEYS.load(Ordering::Relaxed);
    let pointer = LOG.load(Ordering::Acquire) as *const Log;
    let active = retained
        .iter()
        .flat_map(|batch| batch.iter())
        .find(|log| std::ptr::eq(*log, pointer))
        .map(|log| {
            required_entries.extend(
                log.lanes
                    .iter()
                    .map(|lane| lane.required_entries.load(Ordering::Relaxed)),
            );
            staging_entries.extend(log.lanes.iter().map(|lane| lane.slots.len()));
            ActiveStatusMeta {
                events: log.frontier(),
                spill_pending: log.spill_pending(),
                staging_entries: log.entries,
                staging_threads: log.lanes.len(),
                shared_lane_entries: log.shared.len(),
                required_shared_lane_entries: log.shared.required_entries(),
                staging_key_stride: log.key_stride,
            }
        });
    StatusSnapshot {
        counters,
        epoch,
        failures,
        unlisted_failures,
        required_keys,
        active,
    }
}

#[cfg(test)]
fn format_status<L: serde::Serialize>(
    snapshot: StatusSnapshot,
    required_entries: L,
    staging_entries: L,
) -> impl serde::Serialize {
    let StatusSnapshot {
        counters,
        epoch,
        failures,
        unlisted_failures,
        required_keys,
        active,
    } = snapshot;
    let (
        events,
        spill_pending,
        required_entries_per_lane,
        staging_entries_per_lane,
        staging_entries_count,
        staging_threads,
        shared_lane_entries,
        required_shared_lane_entries,
        staging_key_stride,
    ) = match active {
        Some(active) => (
            Some(active.events),
            Some(active.spill_pending),
            Some(required_entries),
            Some(staging_entries),
            Some(active.staging_entries),
            Some(active.staging_threads),
            Some(active.shared_lane_entries),
            Some(active.required_shared_lane_entries),
            Some(active.staging_key_stride),
        ),
        None => (None, None, None, None, None, None, None, None, None),
    };
    crate::status::status!({
        "unhooked": UNHOOKED.get(), "enabled": ENABLED.load(Ordering::Acquire),
        "feature_enabled": cfg!(feature = "politics_capture"), "runtime_enabled": RUNTIME_ENABLED.load(Ordering::Relaxed), "release_refusals": counters[8], "unwound_observers": counters[9],
        "epoch": epoch, "events": events,
        "spilled_records": counters[1], "sentinel_failures": counters[2],
        "spill_pending": spill_pending,
        "native_read_reasons": crate::status::map(FAILURE_NAMES.into_iter().zip(failures).filter(|(_, count)| *count != 0)),
        "first_unlisted_native_read_reason": UNLISTED_FAILURE.get(),
        "unlisted_native_read_reasons": unlisted_failures,
        "failed_native_reads": counters[3], "missing_item_identities": counters[4],
        "required_entries_per_lane": required_entries_per_lane,
        "staging_entries_per_lane": staging_entries_per_lane,
        "required_key_bytes": required_keys,
        "staging_entries": staging_entries_count,
        "staging_threads": staging_threads,
        "shared_lane_threads": counters[11], "shared_lane_full": counters[12],
        "shared_lane_entries": shared_lane_entries,
        "required_shared_lane_entries": required_shared_lane_entries,
        "staging_key_stride": staging_key_stride,
        "observer_timing": "offline and matched live run; no per-detour clock instrumentation",
        "staging_allocation_failures": counters[10],
        "staging_error": (counters[10] != 0).then_some("politics staging allocation failed"),
        "short_handoffs": counters[6], "daily_decomposition_failures": counters[7],
        "conservation": conservation_status(), "government_endpoint_mismatches": counters[5],
    })
}

#[cfg(test)]
fn status_into<'a>(
    required_entries: &'a mut Vec<usize>,
    staging_entries: &'a mut Vec<usize>,
) -> impl serde::Serialize + use<'a> {
    let snapshot = snapshot_status(required_entries, staging_entries);
    format_status(snapshot, required_entries.as_slice(), staging_entries.as_slice())
}

#[cfg(test)]
fn status() -> impl serde::Serialize {
    let mut required_entries = Vec::new();
    let mut staging_entries = Vec::new();
    let snapshot = snapshot_status(&mut required_entries, &mut staging_entries);
    format_status(snapshot, required_entries, staging_entries)
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
    examples: [Option<ConservationExample>; 16],
}
static CONSERVATION: Mutex<Conservation> = Mutex::new(Conservation {
    counts: [0; 2],
    amounts: [0; 2],
    absolute_amounts: [0; 2],
    examples: [None; 16],
});
pub(crate) fn mismatch(country: u32, hour: i32, currency: PowerCurrency, amount: i128) {
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
pub fn conservation_status() -> impl serde::Serialize {
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
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    unsafe fn install(memory: &impl Memory, base: usize) -> Result<Self, String> {
        let sites = entry_sites();
        let located = sites
            .iter()
            .chain(TAPS.iter())
            .map(|site| {
                engine_sites::code(base, site.rva)
                    .and_then(|code| Ok((code.address, code.original::<16>()?)))
                    .map_err(|withheld| {
                        COUNTERS[2].fetch_add(1, Ordering::Relaxed);
                        let cause = format!("politics {}: {withheld}", site.name);
                        crate::hook::installation::refused(
                            crate::hook::installation::Kind::Sentinel,
                            None,
                            &cause,
                        );
                        cause
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let charging = native::CHARGING_CALLERS.iter().map(|&(key, _)| key);
        for key in CLASSIFIED.into_iter().chain(charging) {
            engine_sites::resolve(base, key).map_err(|withheld| {
                COUNTERS[2].fetch_add(1, Ordering::Relaxed);
                format!("politics caller {withheld}")
            })?;
        }
        let (entry_sites, tap_sites) = located.split_at(sites.len());
        BASE.store(base, Ordering::Relaxed);
        let mut entries = Vec::new();
        for (index, (&(address, original), callback)) in
            entry_sites.iter().zip(callbacks()).enumerate()
        {
            let hook = unsafe { Hook::new(address, callback, original) }?;
            ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
            entries.push(hook);
        }
        let mut taps = Vec::new();
        for (index, &(address, original)) in tap_sites.iter().enumerate() {
            taps.push(unsafe { Tap::new(address, tap, index, original) }?);
        }
        taps.push(
            unsafe { crate::script_event_observer::targeted_selection_tap(memory, base) }?,
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
        if !crate::hook::shared::defer_preparation(invalidate_epoch) {
            invalidate_epoch();
        }
        let outcome = match crate::hook::shared::attach_outcome() {
            Some(outcome) => outcome,
            None => Hook::set_enabled_iter(
                self.entries
                    .iter_mut()
                    .chain(self.taps.iter_mut().map(Tap::publication_hook)),
                enabled,
            ),
        };
        if !outcome.map_err(|reason| {
            let _ = UNHOOKED.set(reason);
            "politics hook state change failed"
        })? {
            return Ok(false);
        }
        ENABLED.store(enabled && !self.entries.is_empty(), Ordering::SeqCst);
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
    /// # Safety
    /// Keep `self` alive and unmoved at its current address for every subsequent call on the
    /// returned [`Handoff`].
    pub unsafe fn handoff(&self, cursors: &mut Vec<LaneCursor>) -> Result<Handoff, &'static str> {
        handoff_from_buffer(
            &self.0,
            engine_native_memory::walk::Buffer::Growing(cursors),
            false,
        )
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
#[cfg(test)]
pub(crate) unsafe fn bench_begin(base: usize, entries: usize, keys: usize) {
    BASE.store(base, Ordering::Relaxed);
    // SAFETY: Caller holds `staging_test()` and owns the global politics staging log.
    unsafe { resize(entries, keys) };
    ENABLED.store(true, Ordering::SeqCst);
    let mut cursors = vec![LaneCursor::default(); thread_slots()];
    let _ = handoff_buffer(engine_native_memory::walk::Buffer::Growing(&mut cursors));
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

#[cfg(test)]
#[inline(always)]
pub(crate) fn bench_scope(f: impl FnOnce()) {
    let _active = Call::enter();
    DAILY_HOUR.set(event_hour());
    f();
    DAILY_HOUR.set(None);
}
#[cfg(test)]
#[inline(always)]
pub(crate) fn bench_observe(event: Event) {
    let _active = Call::enter();
    append_record(fixture_record(&event), event.identity.keys);
}

#[cfg(test)]
pub(crate) fn bench_daily_country(country: u32, receivers: &[usize]) {
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

#[cfg(test)]
pub(crate) fn bench_daily_taps(status: usize, modifier_frame: usize, rate_frame: usize) {
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

#[cfg(test)]
pub(crate) unsafe fn bench_drain(events: &mut [Event], bytes: &mut [u8]) -> Drained {
    drain_from(log().unwrap(), events, bytes).unwrap()
}

#[cfg(test)]
pub(crate) unsafe fn bench_staging_parts(events: &[Event]) {
    use std::{hint::black_box, time::Instant};
    let packed: Vec<_> = events
        .iter()
        .map(|e| (fixture_record(e), e.identity.keys))
        .collect();
    for mode in ["guard", "direct_lane", "append_record", "expanded_fixture"] {
        let mut times = Vec::new();
        for _ in 0..100 {
            // SAFETY: Caller holds `staging_test()` for the duration of this benchmark.
            unsafe { bench_begin(base(), events.len(), 32) };
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
#[cfg(test)]
pub(crate) fn bench_end() {
    ENABLED.store(false, Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) unsafe fn bench_midnight_parts(receivers: &[usize]) {
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
    let stack = [black_box(base() + native::DAILY_PP_CALLER.0); 128];
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
            // SAFETY: Caller holds `staging_test()` for the duration of this benchmark.
            unsafe { bench_begin(base(), events.len(), 32) };
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
            &engine_native_memory::memory::SliceMemory {
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
        // SAFETY: test holds `staging_test()` and is the sole consumer of `sealed`.
        assert_eq!(
            unsafe { sealed.drain(&cursors, &mut events, &mut []) }
                .unwrap()
                .copied,
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
        let sealed = crate::test_support::with_allocations_denied(|| {
            handoff_buffer(engine_native_memory::walk::Buffer::Growing(&mut cursors))
        });
        ENABLED.store(enabled, Ordering::SeqCst);
        (sealed.unwrap(), cursors)
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
        let memory = engine_native_memory::memory::SliceMemory {
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
        unsafe { restore_epoch(prior) }.unwrap();
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
        assert!(unsafe { restore_epoch(u64::MAX) }.is_err());
        assert_eq!(EPOCH.load(Ordering::Acquire), epoch);
    }

    #[test]
    fn a_burst_past_one_lane_spills_whole_and_sizes_only_that_lane_for_the_next_generation() {
        let _guard = staging_test();
        let log = Log::with_lanes(16, 1, 4);
        let memory = engine_native_memory::memory::SliceMemory {
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
        assert_eq!(drain_from(&log, &mut output, &mut []).unwrap().copied, 33);
        assert!(log.held().is_empty());
        append_lane(&log, 2, Event::default(), &memory);
        assert_eq!(drain_from(&log, &mut output, &mut []).unwrap().copied, 1);
        for _ in 0..33 {
            append_lane(&grown, 2, Event::default(), &memory);
        }
        assert_eq!(drain_from(&grown, &mut output, &mut []).unwrap().copied, 33);
    }

    #[test]
    fn lane_pressure_retries_growth_without_more_countries_or_keys() {
        let _guard = staging_test();
        clear_staging();
        resize_checked(16, 1).unwrap();
        let before = log().unwrap();
        let capacity = before.lanes[0].slots.len();
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        for _ in 0..=capacity {
            append_lane(before, 0, Event::default(), &memory);
        }
        let pointer = LOG.load(Ordering::Acquire);
        // SAFETY: test holds `staging_test()` as the sole politics-log sizer.
        unsafe { prepare() };
        assert_eq!(LOG.load(Ordering::Acquire), pointer);
        assert_ne!(OFFERED.load(Ordering::Acquire), 0);
        seal_staging();
        assert_ne!(LOG.load(Ordering::Acquire), pointer);
        let after = log().unwrap();
        assert!(after.lanes[0].slots.len() >= capacity * 2);
        assert!(after.entries >= before.entries);
        assert!(after.key_stride >= before.key_stride);
        let pointer = LOG.load(Ordering::Acquire);
        // SAFETY: test holds `staging_test()` as the sole politics-log sizer.
        unsafe { prepare() };
        assert_eq!(LOG.load(Ordering::Acquire), pointer);
    }

    #[test]
    fn staging_growth_preserves_unsealed_and_already_sealed_records() {
        const HALF: u32 = 3;
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
            let memory = engine_native_memory::memory::SliceMemory {
                base: 0x1000,
                bytes: &source,
            };
            for country in 1..=HALF {
                append_to(old, event(country), &memory);
            }
        }
        let (first, first_cursors) = seal_staging();
        {
            let memory = engine_native_memory::memory::SliceMemory {
                base: 0x1000,
                bytes: &source,
            };
            for country in HALF + 1..=2 * HALF {
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
                event(2 * HALF + 1),
                &engine_native_memory::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &source,
                },
            );
        });
        let (third, third_cursors) = seal_staging();
        source.fill(0);
        let mut output = vec![Event::default(); HALF as usize];
        let mut bytes = vec![0; HALF as usize * 9];
        for (start, sealed, cursors) in [
            (1, first, first_cursors),
            (HALF + 1, second, second_cursors),
        ] {
            // SAFETY: test holds `staging_test()` and drains each sealed interval in order.
            let drained = unsafe { sealed.drain(&cursors, &mut output, &mut bytes) }.unwrap();
            assert_eq!(
                (
                    drained.first,
                    drained.cursor,
                    drained.copied,
                    drained.pending
                ),
                (
                    (start - 1) as u64,
                    (start + HALF - 1) as u64,
                    HALF as usize,
                    HALF as usize
                )
            );
            for (index, event) in output.iter().enumerate() {
                assert_eq!(event.country, start + index as u32);
                assert_eq!(event.sequence, (start + index as u32) as u64);
                assert_eq!(event.epoch, epoch);
                let key = event.identity.keys[0];
                assert_eq!(&bytes[key.address..key.address + key.len], b"mod_itemx");
            }
        }
        // SAFETY: test holds `staging_test()` and drains `third` in order.
        let drained = unsafe { third.drain(&third_cursors, &mut output, &mut bytes) }.unwrap();
        assert_eq!(
            (
                drained.first,
                drained.cursor,
                drained.copied,
                drained.pending
            ),
            (2 * HALF as u64, 2 * HALF as u64 + 1, 1, 1)
        );
        assert_eq!(
            (output[0].country, output[0].sequence),
            (2 * HALF + 1, 2 * HALF as u64 + 1)
        );
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
        let memory = engine_native_memory::memory::SliceMemory {
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
        // SAFETY: test holds `staging_test()` as the sole politics-log sizer.
        unsafe { prepare() };
        assert_eq!(OFFERED.load(Ordering::Acquire), offered);
        seal_staging();
        assert_eq!(LOG.load(Ordering::Acquire), offered);
        clear_staging();
    }

    #[test]
    fn refused_cursor_growth_for_a_larger_thread_domain_keeps_the_log_and_its_offer() {
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
        let refused = crate::test_support::with_allocations_denied(|| {
            handoff_buffer(engine_native_memory::walk::Buffer::Growing(&mut small)).err()
        });
        ENABLED.store(false, Ordering::SeqCst);
        assert_eq!(refused, Some("capture buffer allocation failed"));
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
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
        );
        let (sealed, cursors) = seal_staging();
        resize_checked(old.entries + 1, 0).unwrap();
        invalidate_epoch();
        let mut output = [Event::default()];
        // SAFETY: test holds `staging_test()` as the sole consumer of `sealed`.
        assert_eq!(
            unsafe { sealed.drain(&cursors, &mut output, &mut []) },
            Err("politics observation restarted")
        );
        seal_staging();
        let fresh = log().unwrap();
        append_to(
            fresh,
            Event::default(),
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
        );
        let (sealed, cursors) = seal_staging();
        // SAFETY: test holds `staging_test()` as the sole consumer of `sealed`.
        assert_eq!(
            unsafe { sealed.drain(&cursors, &mut output, &mut []) }
                .unwrap()
                .copied,
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
        let before = active();
        assert_eq!(unsafe { callback(1, 2, 3, 4, 0xabcdef00) }, 0xabcdef0a);
        assert_eq!(CONTEXT.get().government, previous.government);
        let epoch = EPOCH.load(Ordering::SeqCst);
        let result = std::panic::catch_unwind(|| unsafe { callback(usize::MAX, 0, 0, 0, 0) });
        assert!(result.is_err());
        assert_eq!(active(), before);
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
        let memory = engine_native_memory::memory::SliceMemory {
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
        for country in 6001..=6003 {
            append_to(
                &log,
                Event {
                    country,
                    ..Default::default()
                },
                &memory,
            );
        }
        for (country, pending) in [(6001, 3), (6002, 2), (6003, 1)] {
            let drain = drain_from(&log, &mut output, &mut bytes).unwrap();
            assert_eq!((drain.copied, drain.pending), (1, pending));
            assert_eq!(output[0].country, country);
        }
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
        let memory = engine_native_memory::memory::SliceMemory {
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
                    let memory = engine_native_memory::memory::SliceMemory {
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
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        append_lane(&log, 0, Event::default(), &memory);
        append_lane(&log, 1, Event::default(), &memory);
        let partial = [
            LaneCursor {
                cursor: 0,
                spilled: 0,
            },
            LaneCursor {
                cursor: 1,
                spilled: 0,
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
        for caller in [0x7320f7, 0x7320f9, 0] {
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
                base().wrapping_add(0x7320f8),
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

    #[test]
    fn set_power_publishes_its_stored_balance_and_raw_request_after_the_effect() {
        let witness = SetPower {
            owner: native::Owner {
                pointer: 0x1000,
                country: 7,
                instance: 1,
                politics: Some(0x2000),
            },
            before: 100 * 100_000,
            raw: -500 * 100_000,
            identity: Identity::default(),
        };
        assert_eq!(
            set_power_published(witness, 0),
            (100 * 100_000, 0, -600 * 100_000)
        );
        let witness = SetPower {
            raw: 150 * 100_000,
            ..witness
        };
        assert_eq!(
            set_power_published(witness, 150 * 100_000),
            (100 * 100_000, 150 * 100_000, 50 * 100_000)
        );
    }

    #[test]
    fn targeted_selection_publishes_inside_its_targeted_decision_scope() {
        let _guard = staging_test();
        crate::war_provenance::base_for_test(1);
        ORIGINAL[TARGETED_DECISION_ENTRY]
            .store(shim_fixture as *const () as usize, Ordering::Release);
        let callback: NativeFunction =
            unsafe { std::mem::transmute(callbacks()[TARGETED_DECISION_ENTRY]) };
        let _ = crate::script_event_observer::take_selection_cause();
        assert_eq!(unsafe { callback(1, 2, 3, 4, 0) }, 10);
        let cause = crate::script_event_observer::take_selection_cause().unwrap();
        assert_eq!(
            cause.route,
            replay_data::event_feed_rows::EventFeedRoute::TargetedDecision
        );
        assert_ne!(
            crate::war_provenance::inside().route,
            replay_data::event_feed_rows::EventFeedRoute::TargetedDecision
        );
        crate::war_provenance::base_for_test(0);
    }

    #[test]
    fn the_event_feed_detours_own_their_politics_entries_once() {
        let sites = entry_sites();
        assert_eq!(sites.len(), ENTRY_COUNT);
        assert_eq!(
            sites[SET_POWER_ENTRY].name,
            native::SET_POLITICAL_POWER_EFFECT.name
        );
        assert_eq!(
            SCOPE_ORDER[TARGETED_DECISION_ENTRY - 10],
            Scope::TargetedDecision
        );
        for (index, site) in sites.iter().enumerate() {
            assert!(sites[..index].iter().all(|other| other.rva != site.rva));
        }
        assert!(
            TAPS.iter()
                .any(|tap| tap.rva == native::SET_POLITICAL_POWER.rva)
        );
    }

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
        let key = |address| engine_sites::reference(0, address);
        assert_eq!(caller_cause(key(0x76fe2e)), PowerCause::Decision);
        assert_eq!(caller_cause(key(0x7631ef)), PowerCause::NotIdentified);
        assert_eq!(caller_cause(Some(native::DAILY_CP_CALLER)), PowerCause::DailyGain);
        let console = government_context(Scope::ConsoleSubject, None, Context::default());
        assert_eq!(
            government_context(Scope::SubjectGovernment, None, console).government,
            GovernmentCause::Console
        );
        let context = government_context(Scope::OnAction, key(0xbfb8ce), Context::default());
        assert_eq!(context.government, GovernmentCause::ElectionOnAction);
        assert_eq!(
            government_context(Scope::OnAction, key(0xbfb8cf), context).government,
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
                    let memory = engine_native_memory::memory::SliceMemory {
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
        assert_eq!(log.registered.load(Ordering::Acquire), 5);
        assert!(COUNTERS[11].load(Ordering::Relaxed) >= shared_threads + 3);
        assert!(REQUIRED_LANES.load(Ordering::Relaxed) >= 5);
        let mut cursors = vec![LaneCursor::default(); 2];
        let handoff = handoff_from_buffer(
            &log,
            engine_native_memory::walk::Buffer::Growing(&mut cursors),
            false,
        )
        .unwrap();
        assert_eq!(handoff.pending(), 20);
        let mut output = [Event::default(); 20];
        let mut bytes = [0; 20 * 16];
        // SAFETY: all producer threads have joined and test holds `staging_test()`.
        let drained = unsafe { handoff.drain(&cursors, &mut output, &mut bytes) }.unwrap();
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
        assert!(log.shared.published(|r| r.sequence).unwrap().is_empty());
        assert!(log.shared.try_reserve().is_some());
        REQUIRED_LANES.store(required, Ordering::Relaxed);
    }

    #[test]
    fn shared_records_merge_by_sequence_and_wait_for_an_unpublished_predecessor() {
        let _guard = staging_test();
        let log = Log::with_lanes(8, 1, 1);
        let memory = engine_native_memory::memory::SliceMemory {
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
        let early = log.shared.try_reserve().unwrap();
        let late = log.shared.try_reserve().unwrap();
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
            let slot = log.shared.try_reserve().unwrap();
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
        invalidate_epoch();
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
        let pending = log.shared.published(|r| r.sequence).unwrap();
        assert_eq!(pending.iter().map(|&(s, _)| s).collect::<Vec<_>>(), [2]);
        let drain = drain_from(&log, &mut output, &mut []).unwrap();
        assert_eq!((drain.copied, output[0].country), (1, 2));
    }

    #[test]
    fn a_full_shared_lane_spills_and_only_a_refused_allocation_is_named() {
        let _guard = staging_test();
        clear_staging();
        resize_checked(4, 1).unwrap();
        let log = log().unwrap();
        let capacity = log.shared.len();
        let full = COUNTERS[12].load(Ordering::Relaxed);
        let failures = COUNTERS[10].load(Ordering::Relaxed);
        let held: Vec<_> = (0..capacity)
            .map(|_| log.shared.try_reserve().unwrap())
            .collect();
        let mut source = [0u8; 32];
        source[..4].copy_from_slice(b"name");
        source[16..24].copy_from_slice(&4u64.to_le_bytes());
        source[24..32].copy_from_slice(&15u64.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &source,
        };
        let named = [
            native::Literal {
                address: 0x1000,
                len: 4,
            },
            native::Literal::default(),
            native::Literal::default(),
        ];
        append_raw_shared(
            log,
            Record::default(),
            [native::Literal::default(); 3],
            &memory,
        );
        assert_eq!(COUNTERS[12].load(Ordering::Relaxed), full + 1);
        assert_eq!(log.held().len(), 1);
        assert_eq!(COUNTERS[10].load(Ordering::Relaxed), failures);
        crate::test_support::with_allocations_denied(|| {
            append_raw_shared(log, Record::default(), named, &memory);
        });
        assert_eq!(log.held().len(), 1);
        assert_eq!(COUNTERS[10].load(Ordering::Relaxed), failures + 1);
        assert!(log.shared.required_entries() > capacity);
        let status = serde_json::to_value(status()).unwrap();
        assert!(status["shared_lane_full"].as_u64().unwrap() > full);
        assert_eq!(
            status["staging_error"],
            serde_json::json!("politics staging allocation failed")
        );
        drop(held);
        // SAFETY: test holds `staging_test()` as the sole politics-log sizer.
        unsafe { prepare() };
        assert_ne!(OFFERED.load(Ordering::Acquire), 0);
        seal_staging();
        assert!(super::log().unwrap().shared.len() > capacity);
        clear_staging();
    }

    #[test]
    fn a_burst_past_the_ring_keeps_every_event_in_order_with_its_keys() {
        let _guard = staging_test();
        let log = Log::with_lanes(2, 1, 1);
        let name = *b"Germany";
        let mut source = [0u8; 32];
        source[..7].copy_from_slice(&name);
        source[16..24].copy_from_slice(&7u64.to_le_bytes());
        source[24..32].copy_from_slice(&15u64.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &source,
        };
        for country in 1..=6_u32 {
            let mut event = Event {
                country,
                ..Default::default()
            };
            if country == 4 {
                event.identity.keys[0] = native::Literal {
                    address: 0x1000,
                    len: name.len(),
                };
            }
            append_lane(&log, 0, event, &memory);
        }
        let mut output = [Event::default(); 6];
        let mut bytes = [0; 6 + 7];
        let drain = drain_from(&log, &mut output, &mut bytes).unwrap();
        assert_eq!(drain.copied, 6);
        assert_eq!(
            output.iter().map(|event| event.country).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
        assert_eq!(
            output.iter().map(|event| event.sequence).collect::<Vec<_>>(),
            (1..=6).collect::<Vec<_>>()
        );
        let key = output[3].identity.keys[0];
        assert_eq!(&bytes[key.address..key.address + key.len], &name);
        assert!(log.held().is_empty());
    }

    #[test]
    fn a_spill_never_waits_for_the_drain_and_a_burst_keeps_every_event_in_order() {
        let _guard = staging_test();
        let log = Log::with_lanes(2, 1, 1);
        let name = *b"Germany";
        let mut source = [0u8; 32];
        source[..7].copy_from_slice(&name);
        source[16..24].copy_from_slice(&7u64.to_le_bytes());
        source[24..32].copy_from_slice(&15u64.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &source,
        };
        let consumer = log.spill.lock().unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let (log, memory) = (&log, &memory);
            scope.spawn(move || {
                for country in 1..=40_u32 {
                    let mut event = Event {
                        country,
                        ..Default::default()
                    };
                    if country % 2 == 0 {
                        event.identity.keys[0] = native::Literal {
                            address: 0x1000,
                            len: name.len(),
                        };
                    }
                    append_lane(log, 0, event, memory);
                }
                done.send(()).unwrap();
            });
            finished
                .recv_timeout(std::time::Duration::from_secs(30))
                .expect("a politics spill waited for the drain's lock");
            drop(consumer);
        });
        let mut output = [Event::default(); 40];
        let mut bytes = [0; 20 * 7];
        let drain = drain_from(&log, &mut output, &mut bytes).unwrap();
        assert_eq!(drain.copied, 40);
        assert_eq!(
            output.iter().map(|event| event.country).collect::<Vec<_>>(),
            (1..=40).collect::<Vec<_>>()
        );
        assert_eq!(
            output.iter().map(|event| event.sequence).collect::<Vec<_>>(),
            (1..=40).collect::<Vec<_>>()
        );
        for event in output.iter().filter(|event| event.country % 2 == 0) {
            let key = event.identity.keys[0];
            assert_eq!(&bytes[key.address..key.address + key.len], &name);
        }
        assert!(log.held().is_empty());
    }

    #[test]
    fn status_reads_active_log_without_allocating_or_popping_spilled_records() {
        let _guard = staging_test();
        clear_staging();
        struct Cleanup(usize);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                REQUIRED_LANES.store(self.0, Ordering::Relaxed);
                clear_staging();
            }
        }
        let _cleanup = Cleanup(REQUIRED_LANES.load(Ordering::Relaxed));
        let target_lanes = thread_slots().max(257);
        REQUIRED_LANES.store(target_lanes, Ordering::Relaxed);
        // SAFETY: Unit test owns politics staging with no concurrent teardown.
        unsafe { resize(8, 16) };
        let active = log().unwrap();
        assert!(active.lanes.len() >= 257);
        active
            .pushed
            .push((Record::default(), Vec::new()))
            .unwrap();
        let epoch_before = EPOCH.load(Ordering::SeqCst);
        let failures_before = COUNTERS[10].load(Ordering::Relaxed);
        struct LockCheckingWriter<'a> {
            cursor: std::io::Cursor<&'a mut [u8]>,
            writes: usize,
        }
        impl std::io::Write for LockCheckingWriter<'_> {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                assert!(LOGS.try_lock().is_ok());
                self.writes += 1;
                self.cursor.write(buf)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                assert!(LOGS.try_lock().is_ok());
                self.cursor.flush()
            }
        }
        let mut required_scratch = Vec::with_capacity(active.lanes.len());
        let mut staging_scratch = Vec::with_capacity(active.lanes.len());
        let mut scratch = vec![0u8; 16384];
        let (written, writes) = collector_test_support::with_allocations_denied(|| {
            let snapshot = status_into(&mut required_scratch, &mut staging_scratch);
            assert!(LOGS.try_lock().is_ok());
            let mut writer = LockCheckingWriter {
                cursor: std::io::Cursor::new(scratch.as_mut_slice()),
                writes: 0,
            };
            serde_json::to_writer(&mut writer, &snapshot).unwrap();
            (writer.cursor.position() as usize, writer.writes)
        });
        assert!(writes > 0);
        assert_eq!(active.pushed.len(), 1);
        assert_eq!(EPOCH.load(Ordering::SeqCst), epoch_before);
        assert_eq!(COUNTERS[10].load(Ordering::Relaxed), failures_before);
        let parsed: serde_json::Value = serde_json::from_slice(&scratch[..written]).unwrap();
        assert_eq!(parsed["spill_pending"], 1);
        assert_eq!(
            parsed["staging_entries_per_lane"].as_array().unwrap().len(),
            active.lanes.len()
        );
        assert_eq!(
            parsed["required_entries_per_lane"].as_array().unwrap().len(),
            active.lanes.len()
        );
    }
}
