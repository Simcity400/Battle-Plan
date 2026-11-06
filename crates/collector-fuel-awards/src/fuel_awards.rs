//! Passive applied-fuel observations. Depends on the shared interior Tap;
//! no game function is called and no storage grows on a native callback.
use crate::{
    hook::{Hook, Registers, Tap},
    output_log::{Call, Drained, Log, raise},
};
use engine_fuel::fuel_awards as native;
use engine_native::{Memory, memory::LiveMemory};
use engine_sites::Rva;
use std::{
    cell::{Cell, RefCell, UnsafeCell},
    ops::Range,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

static LOG: Log = Log::new(0);
/// The taps in tag order: begin, end, five land receipts, naval, air, reset.
const SITES: usize = 10;
static BASE: AtomicUsize = AtomicUsize::new(0);
static ROOT: AtomicU64 = AtomicU64::new(1);
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
static REFUSAL: collector_primitives::first_reason::FirstReason =
    collector_primitives::first_reason::FirstReason::new();
static CENSUS_LAST_REFUSAL: Mutex<Option<&'static str>> = Mutex::new(None);
static COUNTERS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
// The live buffer and its sealed predecessor belong to the hourly drain and
// boundary installation. Callbacks publish into unconsumed slots through a
// stable pointer, without taking any lock.
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
const APPEND_NAMES: [&str; 6] = [
    "fuel recipient append lost: entry capacity exceeded",
    "fuel terminal root append lost: entry capacity exceeded",
    "fuel reset append lost: entry capacity exceeded",
    "fuel recipient append lost: live storage absent",
    "fuel terminal root append lost: live storage absent",
    "fuel reset append lost: live storage absent",
];
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
// Sizing hands storage to the boundary through atomics only: the prepared
// successor moves by swap with its shape published beside it, and consumed
// predecessors return through a lock-free release list. The drain and
// boundary installation never share a lock with spool sizing.
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
struct Slot {
    entry: UnsafeCell<Entry>,
    name_required: UnsafeCell<usize>,
    published: AtomicU64,
}
// Each reserved index has one producer. A slot is never overwritten until the
// consumer has copied both its entry and name and released LOG.consumed.
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
    lost: AtomicU64,
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
            lost: AtomicU64::new(0),
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
                // A stale callback follows the same sealed-successor handoff as
                // politics_changes; the active guard retains the old allocation.
                buffer = successor(buffer)?;
            }
            Err(_) => return None,
        }
    }
}
fn successor(buffer: &Buffer) -> Option<&Buffer> {
    let next = unsafe { buffer.next.load(Ordering::Acquire).as_ref() }?;
    // Closing succeeded with no publishers: the old frontier is now immutable.
    // Either the boundary or a stale publisher can initialize the successor.
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
    /// Where the root's start found its country, in the world and hour of
    /// `stamp`: its settle and air recipients take it ([`held`]).
    located: native::Located,
    stamp: Stamp,
    received: [i64; 3],
    refused: bool,
    cause: Option<usize>,
}
thread_local! {
    static PENDING: Cell<Option<Pending>> = const { Cell::new(None) };
    /// The registry headers the pending root's unit recipients share, reset
    /// at each root's start.
    static REGISTRY: RefCell<native::Registry> = RefCell::new(native::Registry::default());
}

/// Stop admitting receipts before testing callback quiescence. Callbacks
/// enter LOG before testing admission, so an admitted callback cannot race
/// past this check while the taps are restored.
fn stop_admission_and_drained() -> bool {
    LOG.enabled.store(false, Ordering::SeqCst);
    LOG.active.load(Ordering::SeqCst) == 0
}

/// The world generation and hourly boundary a located root belongs to.
type Stamp = (u64, u64);
/// Hourly boundaries seen ([`boundary`]).
static HOURS: AtomicU64 = AtomicU64::new(0);
/// Calls that took their root's located country or registry headers: the
/// settle, the air recipients and the unit recipients.
// Diagnostic only: writer epochs, stamps and payload publication retain their
// own atomic ordering. The callback already owns its hook-timing thread slot.
static HELD: crate::hook_timing::CallCounters<3> = crate::hook_timing::CallCounters::new();

/// Game thread, at each hourly boundary: nothing located before it is taken
/// after it.
pub fn boundary() {
    HOURS.fetch_add(1, Ordering::Relaxed);
}

fn stamp() -> Stamp {
    (
        crate::world_lifetime::generation(),
        HOURS.load(Ordering::Relaxed),
    )
}

/// The pending root's located country, for the calls of the same fuel update
/// invocation (`engine_fuel::fuel_awards::start_held`), which writes no country
/// object, id, link or slot and registers no reference. Only a root still
/// standing is taken: one already refused (a reset of its status, a nested
/// award, a lost receipt) or from another log epoch is read afresh, as is
/// one from another world generation or from before an admitted recording
/// callback's boundary (the stamp).
fn held(pending: &Pending) -> Option<native::Located> {
    (!pending.refused
        && pending.epoch == LOG.epoch.load(Ordering::SeqCst)
        && pending.stamp == stamp()
        && !crate::world_lifetime::invalidating())
    .then_some(pending.located)
}

/// Off-thread live census and allocation. Two passes leave room for an hour
/// still waiting in a short drain; these are capacities, never population caps.
pub fn presize(memory: &impl Memory, base: usize) {
    engine_allocation::advisory(|| presize_storage(memory, base));
}

fn presize_storage(memory: &impl Memory, base: usize) {
    // Country reset also runs before strategic air exists.
    // Its reset receipts need only the live country registry, not air rosters.
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
    // Only sizing callers serialise here; the drain never waits on this lock.
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
    // The latest offer is either still pending or already live.
    if fits(&CURRENT_ENTRIES, &CURRENT_NAMES, &CURRENT_STRIDE)
        || fits(&OFFERED_ENTRIES, &OFFERED_NAMES, &OFFERED_STRIDE)
    {
        return;
    }
    // Independent entry/name pressure must preserve both pending offers.
    let mut entry_capacity = CURRENT_ENTRIES
        .load(Ordering::Acquire)
        .max(OFFERED_ENTRIES.load(Ordering::Acquire));
    let mut name_capacity = CURRENT_NAMES
        .load(Ordering::Acquire)
        .max(OFFERED_NAMES.load(Ordering::Acquire));
    crate::table_capture::grow_capacity(&mut entry_capacity, entries);
    crate::table_capture::grow_capacity(&mut name_capacity, names);
    // Allocation and displaced offers are entirely worker-owned.
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
        // Never live: the boundary takes an offer only through the same swap.
        drop(unsafe { Box::from_raw(displaced) });
    }
}
/// Consumed predecessors and superseded offers wait here for the spool. Only
/// the boundary pushes, under its own lock; release takes the whole list.
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
/// Hourly boundary after the drain, on the same worker; replaced storage is
/// freed by collect off-thread.
pub fn install_offered() {
    let Ok(mut domains) = CURRENT.try_lock() else {
        return;
    };
    retire_consumed(&mut domains);
    if domains.retired.is_some()
        || LOG.active.load(Ordering::SeqCst) != 0
        || LOG.cursor.load(Ordering::SeqCst) != LOG.consumed.load(Ordering::Relaxed)
    {
        return;
    }
    install_prepared(&mut domains);
}
/// A sealed predecessor whose receipts are all consumed, with no callback
/// inside the log, is unreachable by every publisher.
fn retire_consumed(domains: &mut Domains) {
    if LOG.active.load(Ordering::SeqCst) == 0
        && domains.retired.as_ref().is_some_and(|buffer| {
            LOG.consumed.load(Ordering::Acquire) >= buffer.cursor.load(Ordering::Acquire)
        })
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
            // Unsealed, so no publisher follows next.
            return_offer(offered);
            return;
        }
        // An old publisher can finish between the quiet/empty check and seal,
        // and a stale pointer can already publish into the successor. Keep the
        // sealed predecessor readable instead of reopening or losing either.
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
/// Sizing grows every offer from the largest prepared shape, so an offer it
/// made while the boundary held this one is at least as large in each part.
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
    // Captured shortfalls are independent of a readable population census.
    offer(0, 0);
}
pub fn active() -> usize {
    LOG.active.load(Ordering::SeqCst)
}

/// Off-thread, after hook/IP quiescence and spool drain. Unflushed samples keep
/// their buffers until then; moving offers never charges or frees on callback.
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
    release_disposed();
    Ok(())
}

fn attempted_since(cursor: u64, consumed: &AtomicU64) -> usize {
    usize::try_from(cursor.wrapping_sub(consumed.load(Ordering::Relaxed))).unwrap_or(usize::MAX)
}
fn append_reason(code: usize) -> Option<&'static str> {
    code.checked_sub(1)
        .and_then(|index| APPEND_NAMES.get(index))
        .copied()
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
    // Losing a terminal record must not leave earlier recipient rows looking
    // like an observed complete interval. Diagnostics use only atomics here.
    LOG.invalidate();
}
/// Within the callback's `_call`, which keeps the log active.
fn append(
    _call: &Call<'_>,
    entry: Entry,
    name: Option<native::StoredName>,
    memory: &impl Memory,
) -> bool {
    // Land fuel receipt writer, naval fuel receipt writer and hourly air fuel update write each native award once.
    // Rejected receipts still belong to the full observed interval demand.
    let attempted = ENTRY_ATTEMPTS
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1);
    raise(
        &ENTRY_DEMAND,
        attempted_since(attempted, &ENTRY_CONSUMED),
        Ordering::Release,
    );
    // A receipt without a name adds no bytes but still refreshes the demand:
    // a named callback may have added its bytes and not yet raised it.
    let attempted = match &name {
        Some(name) => {
            let bytes = name.byte_len() as u64;
            NAME_ATTEMPTS
                .fetch_add(bytes, Ordering::SeqCst)
                .wrapping_add(bytes)
        }
        None => NAME_ATTEMPTS.load(Ordering::SeqCst),
    };
    raise(
        &NAME_DEMAND,
        attempted_since(attempted, &NAME_CONSUMED),
        Ordering::Release,
    );
    if let Some(name) = &name {
        raise(&NAME_STRIDE, name.byte_len(), Ordering::Release);
    }
    let Some(buffer) = (unsafe { LIVE.load(Ordering::SeqCst).as_ref() }).and_then(publishing)
    else {
        refuse_append(entry, true);
        return false;
    };
    let retained = append_buffer(&LOG, buffer.0, entry, name, memory);
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
    if pending > buffer.entries.len() as u64 {
        // Like political lanes, reject shortage without overwriting a reader or
        // another writer. The whole attempted frontier remains growth demand.
        buffer.lost.fetch_max(index + 1, Ordering::Release);
        log.invalidate();
        return false;
    }
    let slot = index as usize % buffer.entries.len();
    let name_required = name.as_ref().map_or(0, native::StoredName::byte_len);
    if let Some(name) = name {
        if name.byte_len() > buffer.stride {
            entry.name_reason = Some("fuel recipient stored name does not fit");
        } else {
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

/// The taps in [`Hooks::new`]'s order: BEGIN, END, the five land receipt
/// writes, the naval and air receipts and the reset.
#[cfg(feature = "callback_profile")]
const TAG_NAMES: [&str; 10] = [
    "begin", "end", "land_0", "land_1", "land_2", "land_3", "land_4", "naval", "air", "reset",
];
/// Diagnostic cost of every [`observe`] call by tag (`callback_profile`):
/// calls and TSC ticks, every call timed rather than one in 64.
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
        // SAFETY: RDTSC has no preconditions on x86-64.
        let start = unsafe { core::arch::x86_64::_rdtsc() };
        Self { tag, start }
    }
}
#[cfg(feature = "callback_profile")]
impl Drop for TagTimer {
    fn drop(&mut self) {
        // SAFETY: as above.
        let ticks = unsafe { core::arch::x86_64::_rdtsc() }.saturating_sub(self.start);
        if let Some([calls, total]) = TAG_COST.get(self.tag) {
            calls.fetch_add(1, Ordering::Relaxed);
            total.fetch_add(ticks, Ordering::Relaxed);
        }
    }
}
/// `{tag: [calls, ns]}` of every [`observe`] call since attach
/// (`callback_profile`); ticks convert with the hook timer's TSC rate.
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
        // The reset body retains its original receiver in RSI.
        let status = registers.rsi;
        if let Some(mut pending) = PENDING.get()
            && pending.started.status == status
        {
            pending.refused = true;
            pending.cause.get_or_insert(1);
            PENDING.set(Some(pending));
        }
        // Initialization may not yet have a resolvable country. The status
        // pointer still invalidates its previously authenticated local history.
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
                "fuel reset receipt storage unavailable or exhausted"
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
        native::air_held(
            &memory,
            base,
            pointer,
            registers.rdx,
            pending.started.country,
            located.as_ref().map(|located| &located.country),
        )
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

#[cfg(any(test, feature = "test-support"))]
pub fn drain(
    entries: &mut [Entry],
    names: &mut [u8],
) -> Result<(Drained, usize, usize), &'static str> {
    drain_buffers(
        engine_native::walk::Buffer::Fixed(entries),
        engine_native::walk::Buffer::Fixed(names),
    )
}
pub fn drain_growing(
    entries: &mut Vec<Entry>,
    names: &mut Vec<u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    drain_buffers(
        engine_native::walk::Buffer::Growing(entries),
        engine_native::walk::Buffer::Growing(names),
    )
}
pub fn lost_drain() {
    LOST_DRAINS.fetch_add(1, Ordering::Relaxed);
    LOG.invalidate();
}
fn drain_buffers(
    entries: engine_native::walk::Buffer<'_, Entry>,
    names: engine_native::walk::Buffer<'_, u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    let result = drain_domains(entries, names);
    if let Err(reason) = result {
        let _ = FIRST_DRAIN_REFUSAL.set(reason);
        DRAIN_REFUSALS.fetch_add(1, Ordering::Relaxed);
    }
    result
}
fn drain_domains(
    entries: engine_native::walk::Buffer<'_, Entry>,
    names: engine_native::walk::Buffer<'_, u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    // Shared only with boundary installation (the same worker) and detach;
    // spool sizing and capacity reads never take it.
    let Ok(domains) = CURRENT.try_lock() else {
        COUNTERS[6].fetch_add(1, Ordering::Relaxed);
        return Err("fuel observation storage busy");
    };
    let Some(buffer) = domains.current.as_deref() else {
        return Err("fuel observation storage absent");
    };
    // Snapshot before the drain: a later rejected callback belongs to the next
    // interval and its pressure cannot be cleared by this handover.
    let name_cursor = NAME_ATTEMPTS.load(Ordering::SeqCst);
    let names_required = attempted_since(name_cursor, &NAME_CONSUMED);
    let result = handover_buffers(
        &LOG,
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
        // A callback can publish while entries are copied. Consume only an
        // attempt frontier whose earlier publishers are settled and whose
        // entries all belong to this drain, including discarded shortages.
        let entry_cursor = ENTRY_ATTEMPTS.load(Ordering::SeqCst);
        let name_cursor = NAME_ATTEMPTS.load(Ordering::SeqCst);
        if LOG.active.load(Ordering::SeqCst) == 0
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
#[cfg(test)]
fn handover(
    log: &Log,
    buffer: &Buffer,
    entries: &mut [Entry],
    names: &mut [u8],
    unavailable: Option<&'static str>,
) -> Result<(Drained, usize, usize), &'static str> {
    handover_domains(log, buffer, None, entries, names, unavailable)
}
#[cfg(test)]
fn handover_domains(
    log: &Log,
    buffer: &Buffer,
    older: Option<&Buffer>,
    entries: &mut [Entry],
    names: &mut [u8],
    unavailable: Option<&'static str>,
) -> Result<(Drained, usize, usize), &'static str> {
    handover_buffers(
        log,
        buffer,
        older,
        engine_native::walk::Buffer::Fixed(entries),
        engine_native::walk::Buffer::Fixed(names),
        unavailable,
    )
}
fn handover_buffers(
    log: &Log,
    buffer: &Buffer,
    older: Option<&Buffer>,
    mut entries: engine_native::walk::Buffer<'_, Entry>,
    mut names: engine_native::walk::Buffer<'_, u8>,
    unavailable: Option<&'static str>,
) -> Result<(Drained, usize, usize), &'static str> {
    COUNTERS[4].fetch_add(1, Ordering::Relaxed);
    let first = log.consumed.load(Ordering::Acquire);
    let cursor = log.cursor.load(Ordering::SeqCst);
    let epoch = log.epoch.load(Ordering::SeqCst);
    if let Some(reason) = unavailable {
        log.consumed.store(cursor, Ordering::Release);
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
    if log.active.load(Ordering::SeqCst) != 0 {
        return Ok((drained, 0, 0));
    }
    let boundary = older.map_or(first, |old| old.cursor.load(Ordering::Acquire));
    let lost = |domain: &Buffer, start: u64, end: u64| {
        end.saturating_sub(start) > domain.entries.len() as u64
            || (start < domain.lost.load(Ordering::Acquire)
                && domain.lost.load(Ordering::Acquire) <= end)
    };
    if older.is_some_and(|old| lost(old, first, boundary.min(cursor)))
        || lost(buffer, first.max(boundary), cursor)
    {
        COUNTERS[10].fetch_add(1, Ordering::Relaxed);
        log.consumed.store(cursor, Ordering::Release);
        drained.first = cursor;
        return Ok((drained, 0, 0));
    }
    let source = |index: u64| older.filter(|_| index < boundary).unwrap_or(buffer);
    // Worker destinations follow the original settled cursor, including a
    // sealed predecessor. Fixed callback fixtures retain short-drain semantics.
    entries.ensure(pending)?;
    let copied = pending.min(entries.len());
    let mut name_bytes = 0usize;
    let mut required = 0usize;
    for index in first..cursor {
        let domain = source(index);
        let slot = &domain.entries[index as usize % domain.entries.len()];
        if slot.published.load(Ordering::Acquire) != index + 1 {
            return Err("fuel receipt publication incomplete");
        }
        required = required.saturating_add(unsafe { *slot.name_required.get() });
        if index - first < copied as u64 {
            name_bytes = name_bytes
                .checked_add(unsafe { *slot.entry.get() }.name.map_or(0, |(_, len)| len))
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
    for (offset, target) in entries[..copied].iter_mut().enumerate() {
        let index = first + offset as u64;
        let domain = source(index);
        let slot = &domain.entries[index as usize % domain.entries.len()];
        let mut entry = unsafe { *slot.entry.get() };
        if let Some((start, name_len)) = entry.name {
            let offset = len;
            let bytes = names
                .get_mut(offset..offset.saturating_add(name_len))
                .ok_or("fuel name handover does not fit")?;
            for (out, byte) in bytes.iter_mut().zip(&domain.names[start..start + name_len]) {
                *out = unsafe { *byte.get() };
            }
            entry.name = Some((offset, name_len));
            len += name_len;
        }
        *target = entry;
    }
    if log.epoch.load(Ordering::SeqCst) != epoch {
        return Err("fuel award observation restarted");
    }
    // Release slots only after their names have also become owned frame bytes.
    log.consumed.store(first + copied as u64, Ordering::Release);
    drained.copied = copied;
    COUNTERS[7].fetch_add(copied as u64, Ordering::Relaxed);
    Ok((drained, len, required))
}

#[cfg(test)]
#[path = "fuel_awards/tests.rs"]
mod tests;
/// Lock-free, so the spool's per-pass size reads never contend with a drain.
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
pub fn status() -> crate::status::Object<impl crate::status::Fields> {
    let append_refusal = append_reason(FIRST_APPEND_REFUSAL.load(Ordering::Acquire));
    crate::status::status!({ "unhooked": UNHOOKED.get(), "enabled": LOG.enabled.load(Ordering::SeqCst),
        "first_refusal": REFUSAL.get().or(append_refusal),
        "first_append_refusal": append_refusal,
        "append_reasons": crate::status::map(APPEND_NAMES.into_iter().zip(APPEND_CAUSES.iter().map(|value| value.load(Ordering::Relaxed)))),
        "settled_reasons": crate::status::map(SETTLED_NAMES.into_iter().zip(SETTLED_CAUSES.iter().map(|value| value.load(Ordering::Relaxed)))),
        "root_read_refusals": COUNTERS[0].load(Ordering::Relaxed),
        "orphan_operands": COUNTERS[1].load(Ordering::Relaxed),
        "settled_refusals": COUNTERS[2].load(Ordering::Relaxed),
        "recipient_refusals": COUNTERS[3].load(Ordering::Relaxed),
        "census_refusals": COUNTERS[5].load(Ordering::Relaxed),
        "census_last_refusal": *CENSUS_LAST_REFUSAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner),
        "name_refusals": COUNTERS[8].load(Ordering::Relaxed),
        "reset_refusals": COUNTERS[9].load(Ordering::Relaxed),
        "recipient_append_refusals": APPEND_REFUSALS[0].load(Ordering::Relaxed),
        "terminal_root_append_refusals": APPEND_REFUSALS[1].load(Ordering::Relaxed),
        "reset_append_refusals": APPEND_REFUSALS[2].load(Ordering::Relaxed),
        "storage_absent_refusals": COUNTERS[11].load(Ordering::Relaxed),
        "overflow_intervals": COUNTERS[10].load(Ordering::Relaxed),
        "lost_worker_drains": LOST_DRAINS.load(Ordering::Relaxed),
        "drain_refusals": DRAIN_REFUSALS.load(Ordering::Relaxed),
        "first_drain_refusal": FIRST_DRAIN_REFUSAL.get(),
        "day_sum_refusals": crate::tables::day_sum_refusals(),
        "busy_refusals": COUNTERS[6].load(Ordering::Relaxed),
        "held_settles": HELD.total(0),
        "held_air_recipients": HELD.total(1),
        "held_unit_recipients": HELD.total(2),
        "held_scope": "calls that left out a read because they took their UpdateFuel invocation's country, located by its start (settles of the same status, which read only the payload and their own fields; air recipients consuming that country), or a registry header an earlier unit recipient of the invocation read; their other reads, values and refusals are a fresh read's" })
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
        // Even a failed batch may have reached native code. Its buffers stay
        // owned until runtime has drained and checked relay/IP quiescence.
        if enabled {
            self.publication_attempted = true;
        }
        if !enabled && !stop_admission_and_drained() {
            // Keep the taps published until admitted callbacks complete.
            return Ok(false);
        }
        let ready = match crate::hook::shared::attach_outcome() {
            Some(outcome) => outcome.map_err(|_| "fuel award tap publication failed")?,
            None => Hook::set_enabled_iter_attach(
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
        // Bootstrap failures before any publication have no native writers.
        if !self.publication_attempted {
            let _ = release();
        }
    }
}
