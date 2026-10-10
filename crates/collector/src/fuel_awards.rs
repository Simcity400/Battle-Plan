//! Passive applied-fuel observations. Depends on the shared interior Tap;
//! no game function is called and no storage grows on a native callback.
use crate::{
    hook::{Hook, Registers, Tap},
    output_log::{Drained, Log},
};
use engine::{Memory, fuel_awards as native, memory::LiveMemory};
use std::{
    cell::{Cell, UnsafeCell},
    ops::Range,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

static LOG: Log = Log::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
static ROOT: AtomicU64 = AtomicU64::new(1);
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
static REFUSAL: OnceLock<&'static str> = OnceLock::new();
static CENSUS_LAST_REFUSAL: Mutex<Option<&'static str>> = Mutex::new(None);
static COUNTERS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
// Ownership locks belong to sizing and the hourly consumer. Callbacks publish
// into unconsumed slots through a stable pointer, without taking either lock.
static CURRENT: Mutex<Option<Box<Buffer>>> = Mutex::new(None);
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
static STORAGE: Mutex<Storage> = Mutex::new(Storage {
    offered: None,
    retired: None,
});

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Entry {
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
        let mut slots = engine::allocation::vec(entries)?;
        slots.resize_with(entries, || Slot {
            entry: UnsafeCell::new(Entry::default()),
            name_required: UnsafeCell::new(0),
            published: AtomicU64::new(0),
        });
        let stride = stride.max(names.div_ceil(entries.max(1)));
        let count = entries
            .checked_mul(stride)
            .ok_or_else(engine::allocation::oom)?;
        let mut names = engine::allocation::vec(count)?;
        names.resize_with(count, || UnsafeCell::new(0));
        let bytes = crate::raw_memory::vector_bytes(&slots)
            .checked_add(crate::raw_memory::vector_bytes(&names))
            .ok_or_else(engine::allocation::oom)?;
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
struct Storage {
    offered: Option<Box<Buffer>>,
    retired: Option<Box<Buffer>>,
}
#[derive(Clone, Copy)]
struct Pending {
    root: u64,
    epoch: u64,
    started: native::Started,
    received: [i64; 3],
    refused: bool,
    cause: Option<usize>,
}
thread_local! { static PENDING: Cell<Option<Pending>> = const { Cell::new(None) }; }

/// Off-thread live census and allocation. Two passes leave room for an hour
/// still waiting in a short drain; these are capacities, never population caps.
pub fn presize(memory: &impl Memory, base: usize) {
    engine::allocation::advisory(|| presize_storage(memory, base));
}

fn presize_storage(memory: &impl Memory, base: usize) {
    // Country reset also runs before strategic air exists.
    // Its reset receipts need only the live country registry, not air rosters.
    offer(crate::country_scale::census_rows(0).max(1), 0);
    let observed = crate::country_scale::demand_air_game_state(memory, base).and_then(|state| {
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
    let _ = REFUSAL.set(reason);
    *CENSUS_LAST_REFUSAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
    COUNTERS[5].fetch_add(1, Ordering::Relaxed);
}

fn offer(entries: usize, names: usize) {
    let (entries, names) = {
        let Ok(storage) = STORAGE.lock() else { return };
        let entries = entries.max(ENTRY_DEMAND.load(Ordering::Acquire));
        let names = names.max(NAME_DEMAND.load(Ordering::Acquire));
        if entries == 0 && names == 0 {
            return;
        }
        let enough = |b: &Buffer| {
            b.entries.len() >= entries
                && b.names.len() >= names
                && b.stride >= NAME_STRIDE.load(Ordering::Acquire)
        };
        if (CURRENT_ENTRIES.load(Ordering::Acquire) >= entries
            && CURRENT_NAMES.load(Ordering::Acquire) >= names
            && unsafe { LIVE.load(Ordering::SeqCst).as_ref() }
                .is_some_and(|b| b.stride >= NAME_STRIDE.load(Ordering::Acquire)))
            || storage.offered.as_deref().is_some_and(enough)
        {
            return;
        }
        // Independent entry/name pressure must preserve both pending offers.
        let mut entry_capacity = CURRENT_ENTRIES
            .load(Ordering::Acquire)
            .max(storage.offered.as_ref().map_or(0, |b| b.entries.len()));
        let mut name_capacity = CURRENT_NAMES
            .load(Ordering::Acquire)
            .max(storage.offered.as_ref().map_or(0, |b| b.names.len()));
        crate::table_capture::grow_capacity(&mut entry_capacity, entries);
        crate::table_capture::grow_capacity(&mut name_capacity, names);
        (entry_capacity, name_capacity)
    };
    // Allocation and displaced offers are entirely worker-owned.
    let allocated = Buffer::with_stride(
        entries,
        names,
        NAME_STRIDE.load(Ordering::Acquire),
        Default::default(),
    );
    match allocated {
        Ok(buffer) => {
            let Ok(mut storage) = STORAGE.lock() else {
                return;
            };
            if (CURRENT_ENTRIES.load(Ordering::Acquire) >= entries
                && CURRENT_NAMES.load(Ordering::Acquire) >= names
                && unsafe { LIVE.load(Ordering::SeqCst).as_ref() }
                    .is_some_and(|b| b.stride >= NAME_STRIDE.load(Ordering::Acquire)))
                || storage.offered.as_deref().is_some_and(|b| {
                    b.entries.len() >= entries
                        && b.names.len() >= names
                        && b.stride >= NAME_STRIDE.load(Ordering::Acquire)
                })
            {
                return;
            }
            let Ok(buffer) = engine::allocation::boxed(buffer) else {
                drop(storage);
                refuse_census("fuel observation allocation failed");
                return;
            };
            let old = storage.offered.replace(buffer);
            drop(storage);
            drop(old);
        }
        Err(_) => {
            refuse_census("fuel observation allocation failed");
        }
    }
}
/// Empty game-thread boundary only; old storage is freed by collect off-thread.
pub fn install_offered() {
    let Ok(mut current) = CURRENT.try_lock() else {
        return;
    };
    let Ok(mut storage) = STORAGE.try_lock() else {
        return;
    };
    if storage.retired.is_some()
        || LOG.active.load(Ordering::SeqCst) != 0
        || LOG.cursor.load(Ordering::SeqCst) != LOG.consumed.load(Ordering::Relaxed)
    {
        return;
    }
    install_prepared(&mut current, &mut storage);
}
fn install_prepared(current: &mut Option<Box<Buffer>>, storage: &mut Storage) {
    let Some(offered) = storage.offered.as_mut() else {
        return;
    };
    let pointer = &mut **offered as *mut Buffer;
    if let Some(old) = current.as_ref() {
        old.next.store(pointer, Ordering::Release);
        if old
            .publishers
            .compare_exchange(0, usize::MAX, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
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
    let offered = storage.offered.take().unwrap();
    let capacity = offered.entries.len();
    CURRENT_ENTRIES.store(capacity, Ordering::Release);
    CURRENT_NAMES.store(offered.names.len(), Ordering::Release);
    LIVE.store(pointer, Ordering::SeqCst);
    storage.retired = current.replace(offered);
    LOG.capacity.store(capacity, Ordering::Release);
}
pub fn collect() {
    let retired = STORAGE.lock().ok().and_then(|mut storage| {
        (LOG.active.load(Ordering::SeqCst) == 0
            && storage.retired.as_ref().is_none_or(|buffer| {
                LOG.consumed.load(Ordering::Acquire) >= buffer.cursor.load(Ordering::Acquire)
            }))
        .then(|| storage.retired.take())
        .flatten()
    });
    drop(retired);
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
    let (current, buffers) = {
        let mut current = CURRENT.lock().unwrap_or_else(|error| error.into_inner());
        let mut storage = STORAGE.lock().unwrap_or_else(|error| error.into_inner());
        LIVE.store(std::ptr::null_mut(), Ordering::SeqCst);
        LOG.capacity.store(0, Ordering::Release);
        CURRENT_ENTRIES.store(0, Ordering::Release);
        CURRENT_NAMES.store(0, Ordering::Release);
        ENTRY_ATTEMPTS.store(0, Ordering::SeqCst);
        ENTRY_CONSUMED.store(0, Ordering::Relaxed);
        NAME_ATTEMPTS.store(0, Ordering::SeqCst);
        NAME_CONSUMED.store(0, Ordering::Relaxed);
        ENTRY_DEMAND.store(0, Ordering::Release);
        NAME_DEMAND.store(0, Ordering::Release);
        NAME_STRIDE.store(0, Ordering::Release);
        LOG.consumed
            .store(LOG.cursor.load(Ordering::SeqCst), Ordering::Relaxed);
        (
            current.take(),
            std::mem::replace(
                &mut *storage,
                Storage {
                    offered: None,
                    retired: None,
                },
            ),
        )
    };
    drop(current);
    drop(buffers);
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
fn append(entry: Entry, name: Option<native::StoredName>, memory: &impl Memory) -> bool {
    let _call = LOG.enter();
    // Each observed award writer emits its native award once.
    // Rejected receipts still belong to the full observed interval demand.
    let attempted = ENTRY_ATTEMPTS
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1);
    ENTRY_DEMAND.fetch_max(
        attempted_since(attempted, &ENTRY_CONSUMED),
        Ordering::Release,
    );
    let bytes = name.as_ref().map_or(0, native::StoredName::byte_len) as u64;
    let attempted = NAME_ATTEMPTS
        .fetch_add(bytes, Ordering::SeqCst)
        .wrapping_add(bytes);
    NAME_DEMAND.fetch_max(
        attempted_since(attempted, &NAME_CONSUMED),
        Ordering::Release,
    );
    if let Some(name) = &name {
        NAME_STRIDE.fetch_max(name.byte_len(), Ordering::Release);
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
fn append_root(pending: Pending, settled: native::Settled, memory: &impl Memory) {
    let retained = append(
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
    buffer.entries_required.fetch_max(
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
        let _ = REFUSAL.set(reason);
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

unsafe extern "system-unwind" fn observe(registers: &Registers, tag: usize) {
    if !LOG.enabled.load(Ordering::SeqCst) {
        return;
    }
    let _call = LOG.enter();
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
            let _ = REFUSAL.set(if status == 0 {
                "fuel reset status is null"
            } else {
                "fuel reset receipt storage unavailable or exhausted"
            });
            COUNTERS[9].fetch_add(1, Ordering::Relaxed);
        }
        return;
    }
    if tag == 0 {
        match native::start(&memory, base, registers.rsi) {
            Ok(started) => {
                let old = PENDING.replace(Some(Pending {
                    root: ROOT.fetch_add(1, Ordering::Relaxed),
                    epoch: LOG.epoch.load(Ordering::SeqCst),
                    started,
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
                let _ = REFUSAL.set(reason);
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
    if tag == 1 {
        PENDING.set(None);
        let settled = native::settle(&memory, base, pending.started, registers.rbx);
        if let Err(reason) = &settled {
            pending.cause.get_or_insert(6);
            let _ = REFUSAL.set(reason);
        }
        if !settled
            .as_ref()
            .is_ok_and(|s| s.received == pending.received)
        {
            pending.refused = true;
            pending.cause.get_or_insert(4);
        }
        if pending.refused {
            let _ = REFUSAL.set(SETTLED_NAMES[pending.cause.unwrap_or(4)]);
        }
        append_root(pending, settled.unwrap_or_default(), &memory);
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
        native::air(
            &memory,
            base,
            pointer,
            registers.rdx,
            pending.started.country,
        )
    } else {
        native::unit(&memory, base, pointer, kind)
    };
    match recipient {
        Ok(recipient) => {
            let name = (kind != 1).then(|| native::stored_name(&memory, pointer, kind));
            let (name, name_reason) = match name {
                Some(Ok(n)) => (Some(n), None),
                Some(Err(reason)) => (None, Some(reason)),
                None => (None, None),
            };
            if !append(
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
            let _ = REFUSAL.set(reason);
            pending.refused = true;
            pending.cause.get_or_insert(2);
            COUNTERS[3].fetch_add(1, Ordering::Relaxed);
        }
    }
    PENDING.set(Some(pending));
}

#[cfg(test)]
pub fn drain(
    entries: &mut [Entry],
    names: &mut [u8],
) -> Result<(Drained, usize, usize), &'static str> {
    drain_buffers(
        engine::walk::Buffer::Fixed(entries),
        engine::walk::Buffer::Fixed(names),
    )
}
pub(crate) fn drain_growing(
    entries: &mut Vec<Entry>,
    names: &mut Vec<u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    drain_buffers(
        engine::walk::Buffer::Growing(entries),
        engine::walk::Buffer::Growing(names),
    )
}
pub(crate) fn lost_drain() {
    LOST_DRAINS.fetch_add(1, Ordering::Relaxed);
    LOG.invalidate();
}
fn drain_buffers(
    entries: engine::walk::Buffer<'_, Entry>,
    names: engine::walk::Buffer<'_, u8>,
) -> Result<(Drained, usize, usize), &'static str> {
    let Ok(current) = CURRENT.try_lock() else {
        return Err("fuel observation storage busy");
    };
    let Some(buffer) = current.as_ref() else {
        return Err("fuel observation storage absent");
    };
    let Ok(storage) = STORAGE.try_lock() else {
        return Err("fuel observation sizing storage busy");
    };
    // Snapshot before the drain: a later rejected callback belongs to the next
    // interval and its pressure cannot be cleared by this handover.
    let name_cursor = NAME_ATTEMPTS.load(Ordering::SeqCst);
    let names_required = attempted_since(name_cursor, &NAME_CONSUMED);
    let result = handover_buffers(
        &LOG,
        buffer,
        storage.retired.as_deref(),
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
        engine::walk::Buffer::Fixed(entries),
        engine::walk::Buffer::Fixed(names),
        unavailable,
    )
}
fn handover_buffers(
    log: &Log,
    buffer: &Buffer,
    older: Option<&Buffer>,
    mut entries: engine::walk::Buffer<'_, Entry>,
    mut names: engine::walk::Buffer<'_, u8>,
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
pub fn capacities() -> (usize, usize) {
    STORAGE
        .lock()
        .ok()
        .map(|s| {
            let current = (
                CURRENT_ENTRIES.load(Ordering::Acquire),
                CURRENT_NAMES.load(Ordering::Acquire),
            );
            let offered = s
                .offered
                .as_ref()
                .map(|b| (b.entries.len(), b.names.len()))
                .unwrap_or_default();
            (current.0.max(offered.0), current.1.max(offered.1))
        })
        .unwrap_or_default()
}
pub fn status() -> crate::status::Object<impl crate::status::Fields> {
    let append_refusal = append_reason(FIRST_APPEND_REFUSAL.load(Ordering::Acquire));
    crate::status::status!({ "unhooked": UNHOOKED.get(), "enabled": LOG.enabled.load(Ordering::SeqCst),
        "first_refusal": REFUSAL.get().copied().or(append_refusal),
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
        "busy_refusals": COUNTERS[6].load(Ordering::Relaxed) })
}

pub struct Hooks {
    taps: Vec<Tap>,
    publication_attempted: bool,
}
impl Hooks {
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        let mut sites = vec![native::BEGIN, native::END];
        sites.extend(native::LAND);
        sites.extend([native::NAVAL, native::AIR]);
        sites.push(native::RESET);
        let mut taps = Vec::new();
        let result = (|| -> Result<(), &'static str> {
            for (rva, sentinel) in &sites {
                if engine::read::<[u8; 16]>(memory, base + rva) != Some(*sentinel) {
                    return Err("fuel award sentinel differs");
                }
            }
            for (tag, (rva, sentinel)) in sites.into_iter().enumerate() {
                taps.push(
                    unsafe { Tap::new(base + rva, observe, tag, sentinel) }
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
        if !enabled {
            LOG.enabled.store(false, Ordering::SeqCst);
        }
        let mut hooks = crate::hook::publication_storage(self.taps.len(), enabled)
            .map_err(|_| "fuel award hook family allocation failed")?;
        hooks.extend(self.taps.iter_mut().map(Tap::publication_hook));
        let ready = Hook::set_enabled_batch(&mut hooks, enabled)
            .map_err(|_| "fuel award tap publication failed")?;
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
