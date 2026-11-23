use crate::{
    hook::{Hook, Registers, Tap},
    output_log::{Call, Drained, Log, raise},
};
use collector_primitives::spill_stack::SpillStack;
use engine_fuel::fuel_awards as native;
use engine_native_memory::{Memory, memory::LiveMemory};
use engine_sites::Rva;
use std::{
    cell::{Cell, RefCell, UnsafeCell},
    ops::Range,
    sync::{
        Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

static LOG: Log = Log::new(0);
static SPILL: Spill = Spill::new();
const SITES: usize = 10;
static BASE: AtomicUsize = AtomicUsize::new(0);
static ROOT: AtomicU64 = AtomicU64::new(1);
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
static REFUSAL: collector_primitives::first_reason::FirstReason =
    collector_primitives::first_reason::FirstReason::new();
static CENSUS_LAST_REFUSAL: Mutex<Option<&'static str>> = Mutex::new(None);
static COUNTERS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
static CURRENT: Mutex<Domains> = Mutex::new(Domains {
    current: None,
    retired: None,
});
static LIVE: AtomicPtr<Buffer> = AtomicPtr::new(std::ptr::null_mut());
static NAME_STRIDE: AtomicUsize = AtomicUsize::new(0);
static CURRENT_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static CURRENT_NAMES: AtomicUsize = AtomicUsize::new(0);
static ENTRY_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static ENTRY_CONSUMED: AtomicU64 = AtomicU64::new(0);
static NAME_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static NAME_CONSUMED: AtomicU64 = AtomicU64::new(0);
static ENTRY_DEMAND: AtomicUsize = AtomicUsize::new(0);
static NAME_DEMAND: AtomicUsize = AtomicUsize::new(0);
static APPEND_REFUSALS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static APPEND_CAUSES: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
const SETTLED_NAMES: [&str; 8] = [
    "fuel root observation epoch changed",
    "fuel root reset during award",
    "fuel root recipient read refused",
    "fuel root recipient receipt lost",
    "fuel receipts do not conserve native consumer totals",
    "fuel nested award replaced an unfinished root",
    "fuel settled native read refused",
    "fuel callback tag unsupported",
];
static SETTLED_CAUSES: [AtomicU64; SETTLED_NAMES.len()] =
    [const { AtomicU64::new(0) }; SETTLED_NAMES.len()];
static FIRST_APPEND_REFUSAL: AtomicUsize = AtomicUsize::new(0);
static LOST_DRAINS: AtomicU64 = AtomicU64::new(0);
static DRAIN_REFUSALS: AtomicU64 = AtomicU64::new(0);
static FIRST_DRAIN_REFUSAL: OnceLock<&'static str> = OnceLock::new();
static SIZING: Mutex<()> = Mutex::new(());
static OFFERED: AtomicPtr<Buffer> = AtomicPtr::new(std::ptr::null_mut());
static OFFERED_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static OFFERED_NAMES: AtomicUsize = AtomicUsize::new(0);
static OFFERED_STRIDE: AtomicUsize = AtomicUsize::new(0);
static CURRENT_STRIDE: AtomicUsize = AtomicUsize::new(0);
static DISPOSED: AtomicPtr<Buffer> = AtomicPtr::new(std::ptr::null_mut());

#[derive(Clone, Copy, Debug, Default)]
pub struct Entry {
    pub root: u64,
    pub country: u32,
    pub recipient: Option<native::Recipient>,
    pub amount: i64,
    pub started: native::Started,
    pub settled: native::Settled,
    pub refused: bool,
    pub reset: bool,
    pub name: Option<(usize, usize)>,
    pub name_reason: Option<&'static str>,
}
struct Spill {
    pushed: SpillStack<Pushed>,
    held: Mutex<Spilled>,
    lost: AtomicU64,
}
struct Pushed {
    index: u64,
    entry: Entry,
    name_required: usize,
    name: Vec<u8>,
}
struct Spilled {
    entries: Vec<(u64, Entry, usize)>,
    names: Vec<u8>,
}
impl Spilled {
    fn discard_before(&mut self, index: u64) {
        self.entries.retain(|&(at, ..)| at >= index);
        if self.entries.is_empty() {
            self.names.clear();
        }
    }
}
impl Spill {
    const fn new() -> Self {
        Self {
            pushed: SpillStack::new(),
            held: Mutex::new(Spilled {
                entries: Vec::new(),
                names: Vec::new(),
            }),
            lost: AtomicU64::new(u64::MAX),
        }
    }
    fn held(&self) -> MutexGuard<'_, Spilled> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let Spilled { entries, names } = &mut *held;
        for pushed in self.pushed.take() {
            if engine_allocation::reserve(entries, 1)
                .and_then(|()| engine_allocation::reserve(names, pushed.name.len()))
                .is_err()
            {
                self.lost.fetch_min(pushed.index, Ordering::AcqRel);
                continue;
            }
            let mut entry = pushed.entry;
            if let Some((_, len)) = entry.name {
                entry.name = Some((names.len(), len));
                names.extend_from_slice(&pushed.name);
            }
            entries.push((pushed.index, entry, pushed.name_required));
        }
        held
    }
    fn push(
        &self,
        index: u64,
        mut entry: Entry,
        name: Option<native::StoredName>,
        memory: &impl Memory,
    ) -> bool {
        let name_required = name.as_ref().map_or(0, native::StoredName::byte_len);
        let mut bytes = Vec::new();
        if engine_allocation::advisory(|| engine_allocation::reserve_exact(&mut bytes, name_required))
            .is_err()
        {
            self.lost.fetch_min(index, Ordering::AcqRel);
            return false;
        }
        if let Some(name) = name {
            bytes.resize(name_required, 0);
            match name.copy(memory, &mut bytes) {
                Ok(()) => entry.name = Some((0, name_required)),
                Err(reason) => {
                    bytes.clear();
                    entry.name_reason = Some(reason);
                }
            }
        }
        let refusal = entry.name_reason;
        let pushed = Pushed {
            index,
            entry,
            name_required,
            name: bytes,
        };
        if self.pushed.push(pushed).is_err() {
            self.lost.fetch_min(index, Ordering::AcqRel);
            return false;
        }
        if let Some(reason) = refusal {
            REFUSAL.record(reason);
            COUNTERS[8].fetch_add(1, Ordering::Relaxed);
        }
        true
    }
    fn release(&self) {
        drop(self.pushed.take());
        *self.held.lock().unwrap_or_else(PoisonError::into_inner) = Spilled {
            entries: Vec::new(),
            names: Vec::new(),
        };
        self.lost.store(u64::MAX, Ordering::Release);
    }
}
const STORAGE_FAILED: &str = "fuel receipt storage allocation failed";

struct Slot {
    entry: UnsafeCell<Entry>,
    name_required: UnsafeCell<usize>,
    published: AtomicU64,
}
unsafe impl Sync for Slot {}
struct Buffer {
    entries: Vec<Slot>,
    names: Vec<UnsafeCell<u8>>,
    stride: usize,
    publishers: AtomicUsize,
    next: AtomicPtr<Buffer>,
    disposed: AtomicPtr<Buffer>,
    base: AtomicU64,
    cursor: AtomicU64,
    names_demand: AtomicUsize,
    entries_required: AtomicUsize,
    _charge: crate::raw_memory::Charge,
}
unsafe impl Sync for Buffer {}
impl Buffer {
    #[cfg(test)]
    fn new(
        entries: usize,
        names: usize,
        charge: crate::raw_memory::Charge,
    ) -> std::io::Result<Self> {
        Self::with_stride(entries, names, names, charge)
    }
    fn with_stride(
        entries: usize,
        names: usize,
        stride: usize,
        mut charge: crate::raw_memory::Charge,
    ) -> std::io::Result<Self> {
        let mut slots = engine_allocation::vec(entries)?;
        slots.resize_with(entries, || Slot {
            entry: UnsafeCell::new(Entry::default()),
            name_required: UnsafeCell::new(0),
            published: AtomicU64::new(0),
        });
        let stride = stride.max(names.div_ceil(entries.max(1)));
        let count = entries
            .checked_mul(stride)
            .ok_or_else(engine_allocation::oom)?;
        let mut names = engine_allocation::vec(count)?;
        names.resize_with(count, || UnsafeCell::new(0));
        let bytes = crate::raw_memory::vector_bytes(&slots)
            .checked_add(crate::raw_memory::vector_bytes(&names))
            .ok_or_else(engine_allocation::oom)?;
        charge.set(crate::raw_memory::Capacity {
            payload_vectors: bytes,
            ..Default::default()
        });
        Ok(Self {
            entries: slots,
            names,
            stride,
            publishers: AtomicUsize::new(0),
            next: AtomicPtr::new(std::ptr::null_mut()),
            disposed: AtomicPtr::new(std::ptr::null_mut()),
            base: AtomicU64::new(0),
            cursor: AtomicU64::new(0),
            names_demand: AtomicUsize::new(0),
            entries_required: AtomicUsize::new(0),
            _charge: charge,
        })
    }
}
struct Publishing<'a>(&'a Buffer);
impl Drop for Publishing<'_> {
    fn drop(&mut self) {
        self.0.publishers.fetch_sub(1, Ordering::Release);
    }
}
fn publishing(mut buffer: &Buffer) -> Option<Publishing<'_>> {
    loop {
        match buffer
            .publishers
            .fetch_update(Ordering::Acquire, Ordering::Relaxed, |n| {
                n.checked_add(1).filter(|&n| n != usize::MAX)
            }) {
            Ok(_) => return Some(Publishing(buffer)),
            Err(usize::MAX) => {
                buffer = successor(buffer)?;
            }
            Err(_) => return None,
        }
    }
}
fn successor(buffer: &Buffer) -> Option<&Buffer> {
    let next = unsafe { buffer.next.load(Ordering::Acquire).as_ref() }?;
    let frontier = buffer.cursor.load(Ordering::Acquire);
    next.base.fetch_max(frontier, Ordering::Release);
    next.cursor.fetch_max(frontier, Ordering::Release);
    Some(next)
}
struct Domains {
    current: Option<Box<Buffer>>,
    retired: Option<Box<Buffer>>,
}
#[derive(Clone, Copy)]
struct Pending {
    root: u64,
    epoch: u64,
    started: native::Started,
    located: native::Located,
    stamp: Stamp,
    received: [i64; 3],
    refused: bool,
    cause: Option<usize>,
}
thread_local! {
    static PENDING: Cell<Option<Pending>> = const { Cell::new(None) };
    static REGISTRY: RefCell<native::Registry> = RefCell::new(native::Registry::default());
}

fn stop_admission_and_drained() -> bool {
    LOG.enabled.store(false, Ordering::SeqCst);
    LOG.active() == 0
}

type Stamp = (u64, u64);
static HOURS: AtomicU64 = AtomicU64::new(0);
static HELD: crate::hook_timing::CallCounters<3> = crate::hook_timing::CallCounters::new();

pub fn boundary() {
    HOURS.fetch_add(1, Ordering::Relaxed);
}

fn stamp() -> Stamp {
    (
        crate::world_lifetime::generation(),
        HOURS.load(Ordering::Relaxed),
    )
}

fn held(pending: &Pending) -> Option<native::Located> {
    (!pending.refused
        && pending.epoch == LOG.epoch.load(Ordering::SeqCst)
        && pending.stamp == stamp()
        && !crate::world_lifetime::invalidating())
    .then_some(pending.located)
}

pub fn presize(memory: &impl Memory, base: usize) {
    engine_allocation::advisory(|| presize_storage(memory, base));
}

fn presize_storage(memory: &impl Memory, base: usize) {
    offer(crate::country_scale::census_rows(0).max(1), 0);
    let observed = collector_extended_support::demand::demand_air_game_state(memory, base)
        .and_then(|state| {
            state
                .map(|state| native::demand(memory, base, state))
                .transpose()
        });
    let demand = match observed {
        Ok(None) => return,
        Ok(Some(demand)) => demand,
        Err(reason) => {
            refuse_census(reason);
            offer(0, 0);
            return;
        }
    };
    NAME_STRIDE.fetch_max(demand.name_stride, Ordering::Release);
    let Some(entries) = demand.entries.checked_mul(2).map(|n| n.max(1)) else {
        refuse_census("fuel census entry size overflow");
        return;
    };
    let Some(names) = demand.names.checked_mul(2) else {
        refuse_census("fuel census name size overflow");
        return;
    };
    offer(entries, names);
}

fn refuse_census(reason: &'static str) {
    REFUSAL.record(reason);
    *CENSUS_LAST_REFUSAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
    COUNTERS[5].fetch_add(1, Ordering::Relaxed);
}

fn offer(entries: usize, names: usize) {
    let Ok(sizing) = SIZING.lock() else { return };
    let entries = entries.max(ENTRY_DEMAND.load(Ordering::Acquire));
    let names = names.max(NAME_DEMAND.load(Ordering::Acquire));
    if entries == 0 && names == 0 {
        return;
    }
    let stride = NAME_STRIDE.load(Ordering::Acquire);
    let fits = |slots: &AtomicUsize, bytes: &AtomicUsize, width: &AtomicUsize| {
        slots.load(Ordering::Acquire) >= entries
            && bytes.load(Ordering::Acquire) >= names
            && width.load(Ordering::Acquire) >= stride
    };
    if fits(&CURRENT_ENTRIES, &CURRENT_NAMES, &CURRENT_STRIDE)
        || fits(&OFFERED_ENTRIES, &OFFERED_NAMES, &OFFERED_STRIDE)
    {
        return;
    }
    let mut entry_capacity = CURRENT_ENTRIES
        .load(Ordering::Acquire)
        .max(OFFERED_ENTRIES.load(Ordering::Acquire));
    let mut name_capacity = CURRENT_NAMES
        .load(Ordering::Acquire)
        .max(OFFERED_NAMES.load(Ordering::Acquire));
    crate::table_capture::grow_capacity(&mut entry_capacity, entries);
    crate::table_capture::grow_capacity(&mut name_capacity, names);
    let Ok(buffer) = Buffer::with_stride(entry_capacity, name_capacity, stride, Default::default())
        .and_then(engine_allocation::boxed)
    else {
        drop(sizing);
        refuse_census("fuel observation allocation failed");
        return;
    };
    let shape = (buffer.entries.len(), buffer.names.len(), buffer.stride);
    let displaced = OFFERED.swap(Box::into_raw(buffer), Ordering::AcqRel);
    OFFERED_ENTRIES.store(shape.0, Ordering::Release);
    OFFERED_NAMES.store(shape.1, Ordering::Release);
    OFFERED_STRIDE.store(shape.2, Ordering::Release);
    drop(sizing);
    if !displaced.is_null() {
        drop(unsafe { Box::from_raw(displaced) });
    }
}
fn dispose(buffer: Box<Buffer>) {
    let node = Box::into_raw(buffer);
    let mut head = DISPOSED.load(Ordering::Acquire);
    loop {
        unsafe { (*node).disposed.store(head, Ordering::Relaxed) };
        match DISPOSED.compare_exchange_weak(head, node, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return,
            Err(current) => head = current,
        }
    }
}
fn release_disposed() {
    let mut node = DISPOSED.swap(std::ptr::null_mut(), Ordering::AcqRel);
    while !node.is_null() {
        let buffer = unsafe { Box::from_raw(node) };
        node = buffer.disposed.load(Ordering::Relaxed);
        drop(buffer);
    }
}
pub fn install_offered() {
    let Ok(mut domains) = CURRENT.try_lock() else {
        return;
    };
    retire_consumed(&mut domains);
    if OFFERED.load(Ordering::Acquire).is_null()
        || domains.retired.is_some()
        || (domains.current.is_some()
            && LOG.cursor.load(Ordering::SeqCst) != LOG.consumed.load(Ordering::Relaxed))
        || LOG.active() != 0
    {
        return;
    }
    install_prepared(&mut domains);
}
fn retire_consumed(domains: &mut Domains) {
    if domains.retired.as_ref().is_some_and(|buffer| {
        LOG.consumed.load(Ordering::Acquire) >= buffer.cursor.load(Ordering::Acquire)
    }) && LOG.active() == 0
        && let Some(retired) = domains.retired.take()
    {
        dispose(retired);
    }
}
fn install_prepared(domains: &mut Domains) {
    let pointer = OFFERED.swap(std::ptr::null_mut(), Ordering::AcqRel);
    if pointer.is_null() {
        return;
    }
    let offered = unsafe { Box::from_raw(pointer) };
    if let Some(old) = domains.current.as_ref() {
        old.next.store(pointer, Ordering::Release);
        if old
            .publishers
            .compare_exchange(0, usize::MAX, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return_offer(offered);
            return;
        }
        let _ = successor(old);
    } else {
        let frontier = LOG.cursor.load(Ordering::SeqCst);
        offered.base.store(frontier, Ordering::Release);
        offered.cursor.store(frontier, Ordering::Release);
    }
    let capacity = offered.entries.len();
    CURRENT_ENTRIES.store(capacity, Ordering::Release);
    CURRENT_NAMES.store(offered.names.len(), Ordering::Release);
    CURRENT_STRIDE.store(offered.stride, Ordering::Release);
    LIVE.store(pointer, Ordering::SeqCst);
    domains.retired = domains.current.replace(offered);
    LOG.capacity.store(capacity, Ordering::Release);
}
fn return_offer(offered: Box<Buffer>) {
    let pointer = Box::into_raw(offered);
    if OFFERED
        .compare_exchange(
            std::ptr::null_mut(),
            pointer,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        dispose(unsafe { Box::from_raw(pointer) });
    }
}
pub fn collect() {
    release_disposed();
    offer(0, 0);
}
pub fn active() -> usize {
    LOG.active()
}

pub fn release() -> Result<(), &'static str> {
    if LOG.enabled.load(Ordering::SeqCst) || active() != 0 {
        return Err("fuel storage release before callback quiescence");
    }
    let (current, retired, offered) = {
        let mut domains = CURRENT.lock().unwrap_or_else(|error| error.into_inner());
        let sizing = SIZING.lock().unwrap_or_else(|error| error.into_inner());
        LIVE.store(std::ptr::null_mut(), Ordering::SeqCst);
        LOG.capacity.store(0, Ordering::Release);
        CURRENT_ENTRIES.store(0, Ordering::Release);
        CURRENT_NAMES.store(0, Ordering::Release);
        CURRENT_STRIDE.store(0, Ordering::Release);
        OFFERED_ENTRIES.store(0, Ordering::Release);
        OFFERED_NAMES.store(0, Ordering::Release);
        OFFERED_STRIDE.store(0, Ordering::Release);
        ENTRY_ATTEMPTS.store(0, Ordering::SeqCst);
        ENTRY_CONSUMED.store(0, Ordering::Relaxed);
        NAME_ATTEMPTS.store(0, Ordering::SeqCst);
        NAME_CONSUMED.store(0, Ordering::Relaxed);
        ENTRY_DEMAND.store(0, Ordering::Release);
        NAME_DEMAND.store(0, Ordering::Release);
        NAME_STRIDE.store(0, Ordering::Release);
        LOG.consumed
            .store(LOG.cursor.load(Ordering::SeqCst), Ordering::Relaxed);
        let offered = OFFERED.swap(std::ptr::null_mut(), Ordering::AcqRel);
        drop(sizing);
        (
            domains.current.take(),
            domains.retired.take(),
            (!offered.is_null()).then(|| unsafe { Box::from_raw(offered) }),
        )
    };
    drop(current);
    drop(retired);
    drop(offered);
    SPILL.release();
    release_disposed();
    Ok(())
}

fn attempted_since(cursor: u64, consumed: &AtomicU64) -> usize {
    usize::try_from(cursor.wrapping_sub(consumed.load(Ordering::Relaxed))).unwrap_or(usize::MAX)
}
fn refuse_append(entry: Entry, absent: bool) {
    let lane = if entry.reset {
        2
    } else if entry.recipient.is_some() {
        0
    } else {
        1
    };
    let code = lane + if absent { 4 } else { 1 };
    let _ = FIRST_APPEND_REFUSAL.compare_exchange(0, code, Ordering::Release, Ordering::Relaxed);
    APPEND_REFUSALS[lane].fetch_add(1, Ordering::Relaxed);
    APPEND_CAUSES[code - 1].fetch_add(1, Ordering::Relaxed);
    if absent {
        COUNTERS[11].fetch_add(1, Ordering::Relaxed);
    }
    LOG.invalidate();
}
fn append(
    _call: &Call<'_>,
    entry: Entry,
    name: Option<native::StoredName>,
    memory: &impl Memory,
) -> bool {
    let attempted = ENTRY_ATTEMPTS
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    raise(
        &ENTRY_DEMAND,
        attempted_since(attempted, &ENTRY_CONSUMED),
        Ordering::Release,
    );
    let attempted = match &name {
        Some(name) => {
            let bytes = name.byte_len() as u64;
            NAME_ATTEMPTS
                .fetch_add(bytes, Ordering::Relaxed)
                .wrapping_add(bytes)
        }
        None => NAME_ATTEMPTS.load(Ordering::Relaxed),
    };
    raise(
        &NAME_DEMAND,
        attempted_since(attempted, &NAME_CONSUMED),
        Ordering::Release,
    );
    if let Some(name) = &name {
        raise(&NAME_STRIDE, name.byte_len(), Ordering::Release);
    }
    let Some(buffer) = (unsafe { LIVE.load(Ordering::Acquire).as_ref() }).and_then(publishing)
    else {
        let index = LOG.cursor.fetch_add(1, Ordering::SeqCst);
        if SPILL.push(index, entry, name, memory) {
            return true;
        }
        refuse_append(entry, true);
        return false;
    };
    let retained = append_buffer(&LOG, &SPILL, buffer.0, entry, name, memory);
    if !retained {
        refuse_append(entry, false);
    }
    retained
}
fn append_root(call: &Call<'_>, pending: Pending, settled: native::Settled, memory: &impl Memory) {
    let retained = append(
        call,
        Entry {
            root: pending.root,
            country: pending.started.country,
            started: pending.started,
            settled,
            refused: pending.refused,
            ..Default::default()
        },
        None,
        memory,
    );
    if pending.refused || !retained {
        if let Some(cause) = pending.cause {
            SETTLED_CAUSES[cause].fetch_add(1, Ordering::Relaxed);
        }
        COUNTERS[2].fetch_add(1, Ordering::Relaxed);
    }
}
fn append_buffer(
    log: &Log,
    spill: &Spill,
    buffer: &Buffer,
    mut entry: Entry,
    name: Option<native::StoredName>,
    memory: &impl Memory,
) -> bool {
    let index = log.cursor.fetch_add(1, Ordering::SeqCst);
    buffer.cursor.fetch_max(index + 1, Ordering::Relaxed);
    let pending = index
        .wrapping_sub(
            log.consumed
                .load(Ordering::Acquire)
                .max(buffer.base.load(Ordering::Acquire)),
        )
        .saturating_add(1);
    raise(
        &buffer.entries_required,
        usize::try_from(pending).unwrap_or(usize::MAX),
        Ordering::Relaxed,
    );
    if pending > buffer.entries.len() as u64
        || name
            .as_ref()
            .is_some_and(|name| name.byte_len() > buffer.stride)
    {
        return spill.push(index, entry, name, memory);
    }
    let slot = index as usize % buffer.entries.len();
    let name_required = name.as_ref().map_or(0, native::StoredName::byte_len);
    if let Some(name) = name {
        let start = slot * buffer.stride;
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(
                buffer.names.as_ptr().add(start).cast::<u8>().cast_mut(),
                name.byte_len(),
            )
        };
        match name.copy(memory, bytes) {
            Ok(()) => {
                entry.name = Some((start, name.byte_len()));
            }
            Err(reason) => entry.name_reason = Some(reason),
        }
    }
    if let Some(reason) = entry.name_reason {
        REFUSAL.record(reason);
        COUNTERS[8].fetch_add(1, Ordering::Relaxed);
    }
    unsafe {
        *buffer.entries[slot].entry.get() = entry;
        *buffer.entries[slot].name_required.get() = name_required;
    }
    buffer.entries[slot]
        .published
        .store(index + 1, Ordering::Release);
    true
}

#[cfg(feature = "callback_profile")]
const TAG_NAMES: [&str; 10] = [
    "begin", "end", "land_0", "land_1", "land_2", "land_3", "land_4", "naval", "air", "reset",
];
#[cfg(feature = "callback_profile")]
static TAG_COST: [[AtomicU64; 2]; TAG_NAMES.len()] =
    [const { [const { AtomicU64::new(0) }; 2] }; TAG_NAMES.len()];
#[cfg(feature = "callback_profile")]
struct TagTimer {
    tag: usize,
    start: u64,
}
#[cfg(feature = "callback_profile")]
impl TagTimer {
    fn new(tag: usize) -> Self {
        let start = unsafe { core::arch::x86_64::_rdtsc() };
        Self { tag, start }
    }
}
#[cfg(feature = "callback_profile")]
impl Drop for TagTimer {
    fn drop(&mut self) {
        let ticks = unsafe { core::arch::x86_64::_rdtsc() }.saturating_sub(self.start);
        if let Some([calls, total]) = TAG_COST.get(self.tag) {
            calls.fetch_add(1, Ordering::Relaxed);
            total.fetch_add(ticks, Ordering::Relaxed);
        }
    }
}
#[cfg(feature = "callback_profile")]
pub fn tag_profile() -> impl serde::Serialize {
    let hz = crate::hook_timing::tsc_hz().unwrap_or(0);
    crate::status::map(
        TAG_NAMES
            .into_iter()
            .zip(&TAG_COST)
            .map(move |(name, [calls, ticks])| {
                let ticks = u128::from(ticks.load(Ordering::Relaxed));
                let ns = ticks
                    .saturating_mul(1_000_000_000)
                    .checked_div(u128::from(hz))
                    .map_or(u64::MAX, |ns| u64::try_from(ns).unwrap_or(u64::MAX));
                (name, [calls.load(Ordering::Relaxed), ns])
            }),
    )
}

unsafe extern "system-unwind" fn observe(registers: &Registers, tag: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::FuelAwards);
    #[cfg(feature = "callback_profile")]
    let _tag = TagTimer::new(tag);
    let call = LOG.enter();
    if !LOG.enabled.load(Ordering::SeqCst) {
        return;
    }
    let memory = unsafe { LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    if tag == 9 {
        let status = registers.rsi;
        if let Some(mut pending) = PENDING.get()
            && pending.started.status == status
        {
            pending.refused = true;
            pending.cause.get_or_insert(1);
            PENDING.set(Some(pending));
        }
        if status == 0
            || !append(
                &call,
                Entry {
                    reset: true,
                    started: native::Started {
                        status,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                None,
                &memory,
            )
        {
            LOG.invalidate();
            REFUSAL.record(if status == 0 {
                "fuel reset status is null"
            } else {
                STORAGE_FAILED
            });
            COUNTERS[9].fetch_add(1, Ordering::Relaxed);
        }
        return;
    }
    if tag == 0 {
        match native::start_held(&memory, base, registers.rsi, None) {
            Ok((started, located, _)) => {
                REGISTRY.with(|registry| *registry.borrow_mut() = native::Registry::default());
                let old = PENDING.replace(Some(Pending {
                    root: ROOT.fetch_add(1, Ordering::Relaxed),
                    epoch: LOG.epoch.load(Ordering::SeqCst),
                    started,
                    located,
                    stamp: stamp(),
                    received: [0; 3],
                    refused: false,
                    cause: None,
                }));
                if old.is_some() {
                    COUNTERS[0].fetch_add(1, Ordering::Relaxed);
                    PENDING.with(|pending| {
                        if let Some(mut p) = pending.get() {
                            p.refused = true;
                            p.cause.get_or_insert(5);
                            pending.set(Some(p));
                        }
                    });
                }
            }
            Err(reason) => {
                REFUSAL.record(reason);
                PENDING.set(None);
                COUNTERS[0].fetch_add(1, Ordering::Relaxed);
            }
        }
        return;
    }
    let Some(mut pending) = PENDING.get() else {
        COUNTERS[1].fetch_add(1, Ordering::Relaxed);
        return;
    };
    if pending.epoch != LOG.epoch.load(Ordering::SeqCst) {
        pending.refused = true;
        pending.cause.get_or_insert(0);
    }
    let located = held(&pending);
    if tag == 1 {
        PENDING.set(None);
        let settled = native::settle_held(
            &memory,
            base,
            pending.started,
            registers.rbx,
            located.as_ref(),
        )
        .map(|(settled, taken)| {
            if taken {
                HELD.increment(0);
            }
            settled
        });
        if let Err(reason) = &settled {
            pending.cause.get_or_insert(6);
            REFUSAL.record(reason);
        }
        if !settled
            .as_ref()
            .is_ok_and(|s| s.received == pending.received)
        {
            pending.refused = true;
            pending.cause.get_or_insert(4);
        }
        if pending.refused {
            REFUSAL.record(SETTLED_NAMES[pending.cause.unwrap_or(4)]);
        }
        append_root(&call, pending, settled.unwrap_or_default(), &memory);
        return;
    }
    let (kind, pointer, amount) = match tag {
        2..=6 => (0, registers.rax, registers.rdx as i64),
        7 => (2, registers.rcx, registers.rdx as i64),
        8 => (1, registers.rdi, registers.rbx as i64),
        _ => {
            pending.refused = true;
            pending.cause.get_or_insert(7);
            PENDING.set(Some(pending));
            return;
        }
    };
    pending.received[kind as usize] = pending.received[kind as usize].wrapping_add(amount);
    let recipient = if kind == 1 {
        if let Some(located) = located.as_ref() {
            REGISTRY.with(|registry| {
                native::air_held_with(
                    &memory,
                    base,
                    pointer,
                    registers.rdx,
                    pending.started.country,
                    Some(&located.country),
                    Some(&mut registry.borrow_mut()),
                )
            })
        } else {
            native::air_held(
                &memory,
                base,
                pointer,
                registers.rdx,
                pending.started.country,
                None,
            )
        }
    } else if located.is_some() {
        REGISTRY.with(|registry| {
            native::unit_with(&memory, base, pointer, kind, &mut registry.borrow_mut())
        })
    } else {
        native::unit(&memory, base, pointer, kind).map(|recipient| (recipient, false))
    }
    .map(|(recipient, taken)| {
        if taken {
            HELD.increment(if kind == 1 { 1 } else { 2 });
        }
        recipient
    });
    match recipient {
        Ok(recipient) => {
            let name = (kind != 1).then(|| native::stored_name(&memory, pointer, kind));
            let (name, name_reason) = match name {
                Some(Ok(n)) => (Some(n), None),
                Some(Err(reason)) => (None, Some(reason)),
                None => (None, None),
            };
            if !append(
                &call,
                Entry {
                    root: pending.root,
                    country: pending.started.country,
                    recipient: Some(recipient),
                    amount,
                    name_reason,
                    ..Default::default()
                },
                name,
                &memory,
            ) {
                pending.refused = true;
                pending.cause.get_or_insert(3);
            }
        }
        Err(reason) => {
            REFUSAL.record(reason);
            pending.refused = true;
            pending.cause.get_or_insert(2);
            COUNTERS[3].fetch_add(1, Ordering::Relaxed);
        }
    }
    PENDING.set(Some(pending));
}

pub fn drain(
    entries: &mut Vec<Entry>,
    names: &mut Vec<u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    drain_buffers(
        engine_native_memory::walk::Buffer::Growing(entries),
        engine_native_memory::walk::Buffer::Growing(names),
    )
}
pub fn lost_drain() {
    LOST_DRAINS.fetch_add(1, Ordering::Relaxed);
    LOG.invalidate();
}
fn drain_buffers(
    entries: engine_native_memory::walk::Buffer<'_, Entry>,
    names: engine_native_memory::walk::Buffer<'_, u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    let result = drain_domains(entries, names);
    if let Err(reason) = result {
        let _ = FIRST_DRAIN_REFUSAL.set(reason);
        DRAIN_REFUSALS.fetch_add(1, Ordering::Relaxed);
    }
    result
}
fn drain_domains(
    entries: engine_native_memory::walk::Buffer<'_, Entry>,
    names: engine_native_memory::walk::Buffer<'_, u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    let Ok(domains) = CURRENT.try_lock() else {
        COUNTERS[6].fetch_add(1, Ordering::Relaxed);
        return Err("fuel observation storage busy");
    };
    let Some(buffer) = domains.current.as_deref() else {
        return Err("fuel observation storage absent");
    };
    let name_cursor = NAME_ATTEMPTS.load(Ordering::SeqCst);
    let names_required = attempted_since(name_cursor, &NAME_CONSUMED);
    let result = handover_buffers(
        &LOG,
        &SPILL,
        buffer,
        domains.retired.as_deref(),
        entries,
        names,
        UNHOOKED.get().copied().or_else(|| {
            (!LOG.enabled.load(Ordering::SeqCst)).then_some("fuel award hooks disabled")
        }),
    );
    ENTRY_DEMAND.fetch_max(
        buffer.entries_required.load(Ordering::Relaxed),
        Ordering::Release,
    );
    NAME_DEMAND.fetch_max(
        buffer.names_demand.load(Ordering::Relaxed),
        Ordering::Release,
    );
    result.map(|(drained, len, required)| {
        let entry_cursor = ENTRY_ATTEMPTS.load(Ordering::SeqCst);
        let name_cursor = NAME_ATTEMPTS.load(Ordering::SeqCst);
        if LOG.active() == 0
            && LOG.cursor.load(Ordering::SeqCst) == drained.cursor
            && LOG.epoch.load(Ordering::SeqCst) == drained.epoch
            && drained.first + drained.copied as u64 == drained.cursor
        {
            ENTRY_CONSUMED.store(entry_cursor, Ordering::Relaxed);
            NAME_CONSUMED.store(name_cursor, Ordering::Relaxed);
        } else {
            ENTRY_CONSUMED.fetch_add(drained.copied as u64, Ordering::Relaxed);
            NAME_CONSUMED.fetch_add(len as u64, Ordering::Relaxed);
        }
        (drained, len, required.max(names_required))
    })
}
fn handover_buffers(
    log: &Log,
    spill: &Spill,
    buffer: &Buffer,
    older: Option<&Buffer>,
    mut entries: engine_native_memory::walk::Buffer<'_, Entry>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
    unavailable: Option<&'static str>,
) -> Result<(Drained, usize, usize), &'static str> {
    COUNTERS[4].fetch_add(1, Ordering::Relaxed);
    let first = log.consumed.load(Ordering::Acquire);
    let cursor = log.cursor.load(Ordering::SeqCst);
    let epoch = log.epoch.load(Ordering::SeqCst);
    if let Some(reason) = unavailable {
        log.consumed.store(cursor, Ordering::Release);
        spill.held().discard_before(cursor);
        return Err(reason);
    }
    let pending = usize::try_from(cursor.wrapping_sub(first))
        .map_err(|_| "fuel observation interval size exceeds address space")?;
    let mut drained = Drained {
        epoch,
        first,
        cursor,
        copied: 0,
        pending,
    };
    if log.active() != 0 {
        return Ok((drained, 0, 0));
    }
    let mut held = spill.held();
    if spill.lost.load(Ordering::Acquire) < cursor {
        let _ = spill
            .lost
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |lost| {
                (lost < cursor).then_some(u64::MAX)
            });
        COUNTERS[10].fetch_add(1, Ordering::Relaxed);
        log.consumed.store(cursor, Ordering::Release);
        held.discard_before(cursor);
        drained.first = cursor;
        return Ok((drained, 0, 0));
    }
    let boundary = older.map_or(first, |old| old.cursor.load(Ordering::Acquire));
    let source = |index: u64| older.filter(|_| index < boundary).unwrap_or(buffer);
    let slot = |index: u64| {
        let domain = source(index);
        let slot = (index as usize)
            .checked_rem(domain.entries.len())
            .map(|slot| &domain.entries[slot])
            .filter(|slot| slot.published.load(Ordering::Acquire) == index + 1)
            .ok_or("fuel receipt publication incomplete")?;
        Ok::<_, &'static str>((domain, slot))
    };
    entries.ensure(pending)?;
    let copied = pending.min(entries.len());
    held.entries.sort_unstable_by_key(|&(index, ..)| index);
    let stale = held.entries.partition_point(|&(index, ..)| index < first);
    let spilled = &held.entries[stale..];
    let spilled_at = |at: &mut usize, index: u64| match spilled.get(*at) {
        Some(&(held, entry, required)) if held == index => {
            *at += 1;
            Some((entry, required))
        }
        _ => None,
    };
    let mut name_bytes = 0usize;
    let mut required = 0usize;
    let mut at = 0;
    for index in first..cursor {
        let (entry, name_required) = match spilled_at(&mut at, index) {
            Some(held) => held,
            None => {
                let (_, slot) = slot(index)?;
                unsafe { (*slot.entry.get(), *slot.name_required.get()) }
            }
        };
        required = required.saturating_add(name_required);
        if index - first < copied as u64 {
            name_bytes = name_bytes
                .checked_add(entry.name.map_or(0, |(_, len)| len))
                .ok_or("fuel name handover size overflow")?;
        }
    }
    buffer.names_demand.fetch_max(required, Ordering::Relaxed);
    if !names.ensure(if names.is_growing() {
        required
    } else {
        name_bytes
    })? {
        return Err("fuel name handover does not fit");
    }
    let mut len = 0usize;
    let mut at = 0;
    for (offset, target) in entries[..copied].iter_mut().enumerate() {
        let index = first + offset as u64;
        let entry = match spilled_at(&mut at, index) {
            Some((mut entry, _)) => {
                if let Some((start, name_len)) = entry.name {
                    names
                        .get_mut(len..len.saturating_add(name_len))
                        .ok_or("fuel name handover does not fit")?
                        .copy_from_slice(&held.names[start..start + name_len]);
                    entry.name = Some((len, name_len));
                    len += name_len;
                }
                entry
            }
            None => {
                let (domain, slot) = slot(index)?;
                let mut entry = unsafe { *slot.entry.get() };
                if let Some((start, name_len)) = entry.name {
                    let bytes = names
                        .get_mut(len..len.saturating_add(name_len))
                        .ok_or("fuel name handover does not fit")?;
                    for (out, byte) in bytes.iter_mut().zip(&domain.names[start..start + name_len])
                    {
                        *out = unsafe { *byte.get() };
                    }
                    entry.name = Some((len, name_len));
                    len += name_len;
                }
                entry
            }
        };
        *target = entry;
    }
    if log.epoch.load(Ordering::SeqCst) != epoch {
        return Err("fuel award observation restarted");
    }
    log.consumed.store(first + copied as u64, Ordering::Release);
    held.discard_before(first + copied as u64);
    drained.copied = copied;
    COUNTERS[7].fetch_add(copied as u64, Ordering::Relaxed);
    Ok((drained, len, required))
}

#[cfg(test)]
#[path = "fuel_awards/tests.rs"]
mod tests;
pub fn capacities() -> (usize, usize) {
    (
        CURRENT_ENTRIES
            .load(Ordering::Acquire)
            .max(OFFERED_ENTRIES.load(Ordering::Acquire)),
        CURRENT_NAMES
            .load(Ordering::Acquire)
            .max(OFFERED_NAMES.load(Ordering::Acquire)),
    )
}

pub struct Hooks {
    taps: Vec<Tap>,
    publication_attempted: bool,
}
impl Hooks {
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Self {
        let sites: [Rva; SITES] = [
            native::BEGIN,
            native::END,
            native::LAND[0],
            native::LAND[1],
            native::LAND[2],
            native::LAND[3],
            native::LAND[4],
            native::NAVAL,
            native::AIR,
            native::RESET,
        ];
        let mut taps = Vec::new();
        let result = (|| -> Result<(), &'static str> {
            let mut located = [(0, [0; 16]); SITES];
            for (slot, &key) in located.iter_mut().zip(&sites) {
                *slot = engine_sites::hook(base, key)
                    .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
                    .inspect_err(|cause| {
                        crate::hook::installation::refused(
                            crate::hook::installation::Kind::Sentinel,
                            None,
                            cause,
                        )
                    })?;
            }
            for (tag, (address, original)) in located.into_iter().enumerate() {
                taps.push(
                    unsafe { Tap::new(address, observe, tag, original) }
                        .map_err(|_| "fuel award tap construction failed")?,
                );
            }
            Ok(())
        })();
        if let Err(reason) = result {
            let _ = UNHOOKED.set(reason);
            taps.clear();
        }
        BASE.store(base, Ordering::Relaxed);
        Self {
            taps,
            publication_attempted: false,
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if enabled {
            self.publication_attempted = true;
        }
        if !enabled && !stop_admission_and_drained() {
            return Ok(false);
        }
        let ready = match crate::hook::shared::attach_outcome() {
            Some(outcome) => outcome.map_err(|_| "fuel award tap publication failed")?,
            None => Hook::set_enabled_iter(
                self.taps.iter_mut().map(Tap::publication_hook),
                enabled,
            )
            .map_err(|_| "fuel award tap publication failed")?,
        };
        if ready {
            LOG.invalidate();
            LOG.enabled
                .store(enabled && !self.taps.is_empty(), Ordering::SeqCst);
        }
        Ok(ready)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.taps.iter().flat_map(Tap::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        if !self.publication_attempted {
            let _ = release();
        }
    }
}
