//! Passive applied-fuel observations. Depends on the shared interior Tap;
//! no game function is called and no storage grows on a native callback.
use crate::{
    hook::{Hook, Registers, Tap},
    output_log::{DrainCounters, Drained, Log},
};
use engine::{Memory, fuel_awards as native, memory::LiveMemory};
use std::{
    cell::Cell,
    ops::Range,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

static LOG: Log = Log::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
static ROOT: AtomicU64 = AtomicU64::new(1);
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
static REFUSAL: OnceLock<&'static str> = OnceLock::new();
static CENSUS_LAST_REFUSAL: Mutex<Option<&'static str>> = Mutex::new(None);
static COUNTERS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
static STORAGE: Mutex<Storage> = Mutex::new(Storage {
    current: None,
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
struct Buffer {
    entries: Vec<Entry>,
    names: Vec<u8>,
    names_len: usize,
    names_required: usize,
    names_demand: usize,
    entries_required: usize,
    _charge: crate::raw_memory::Charge,
}
impl Buffer {
    fn new(
        entries: usize,
        names: usize,
        mut charge: crate::raw_memory::Charge,
    ) -> std::io::Result<Self> {
        let entries = engine::allocation::default_vec(entries)?;
        let names = engine::allocation::filled(names, 0)?;
        let bytes = crate::raw_memory::vector_bytes(&entries)
            .checked_add(crate::raw_memory::vector_bytes(&names))
            .ok_or_else(engine::allocation::oom)?;
        charge.set(crate::raw_memory::Capacity {
            payload_vectors: bytes,
            ..Default::default()
        });
        Ok(Self {
            entries,
            names,
            names_len: 0,
            names_required: 0,
            names_demand: 0,
            entries_required: 0,
            _charge: charge,
        })
    }
}
struct Storage {
    current: Option<Box<Buffer>>,
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
}
thread_local! { static PENDING: Cell<Option<Pending>> = const { Cell::new(None) }; }

/// Off-thread live census and allocation. Two passes leave room for an hour
/// still waiting in a short drain; these are capacities, never population caps.
pub fn presize(memory: &impl Memory, base: usize) {
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
        let entries = entries.max(storage.current.as_ref().map_or(0, |b| b.entries_required));
        let names = names.max(
            storage
                .current
                .as_ref()
                .map_or(0, |b| b.names_required.max(b.names_demand)),
        );
        if entries == 0 && names == 0 {
            return;
        }
        let enough = |b: &Buffer| b.entries.len() >= entries && b.names.len() >= names;
        if storage.current.as_deref().is_some_and(enough)
            || storage.offered.as_deref().is_some_and(enough)
        {
            return;
        }
        // Independent entry/name pressure must preserve both pending offers.
        let mut entry_capacity = storage
            .current
            .as_ref()
            .map_or(0, |b| b.entries.len())
            .max(storage.offered.as_ref().map_or(0, |b| b.entries.len()));
        let mut name_capacity = storage
            .current
            .as_ref()
            .map_or(0, |b| b.names.len())
            .max(storage.offered.as_ref().map_or(0, |b| b.names.len()));
        crate::table_capture::grow_capacity(&mut entry_capacity, entries);
        crate::table_capture::grow_capacity(&mut name_capacity, names);
        (entry_capacity, name_capacity)
    };
    // Do not hold the publication mutex across off-thread allocation: native
    // callbacks refuse a busy buffer rather than blocking on this thread.
    let allocated = Buffer::new(entries, names, Default::default());
    match allocated {
        Ok(buffer) => {
            let Ok(mut storage) = STORAGE.lock() else {
                return;
            };
            if storage
                .current
                .as_deref()
                .is_some_and(|b| b.entries.len() >= entries && b.names.len() >= names)
                || storage
                    .offered
                    .as_deref()
                    .is_some_and(|b| b.entries.len() >= entries && b.names.len() >= names)
            {
                return;
            }
            let old = storage.offered.replace(Box::new(buffer));
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
    let Ok(mut storage) = STORAGE.try_lock() else {
        return;
    };
    if storage.retired.is_some()
        || LOG.active.load(Ordering::SeqCst) != 0
        || LOG.cursor.load(Ordering::SeqCst) != LOG.consumed.load(Ordering::Relaxed)
    {
        return;
    }
    let Some(offered) = storage.offered.take() else {
        return;
    };
    let capacity = offered.entries.len();
    storage.retired = storage.current.replace(offered);
    LOG.capacity.store(capacity, Ordering::Release);
}
pub fn collect() {
    let retired = STORAGE
        .lock()
        .ok()
        .and_then(|mut storage| storage.retired.take());
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
    let buffers = {
        let mut storage = STORAGE.lock().unwrap_or_else(|error| error.into_inner());
        LOG.capacity.store(0, Ordering::Release);
        LOG.consumed
            .store(LOG.cursor.load(Ordering::SeqCst), Ordering::Relaxed);
        std::mem::replace(
            &mut *storage,
            Storage {
                current: None,
                offered: None,
                retired: None,
            },
        )
    };
    drop(buffers);
    Ok(())
}

fn append(entry: Entry, name: Option<native::StoredName>, memory: &impl Memory) -> bool {
    let _call = LOG.enter();
    let Ok(mut storage) = STORAGE.try_lock() else {
        COUNTERS[6].fetch_add(1, Ordering::Relaxed);
        return false;
    };
    let Some(buffer) = storage.current.as_mut() else {
        return false;
    };
    append_buffer(&LOG, buffer, entry, name, memory);
    true
}
fn append_buffer(
    log: &Log,
    buffer: &mut Buffer,
    mut entry: Entry,
    name: Option<native::StoredName>,
    memory: &impl Memory,
) {
    if let Some(name) = name {
        buffer.names_required = buffer.names_required.saturating_add(name.byte_len());
        let start = buffer.names_len;
        match start
            .checked_add(name.byte_len())
            .and_then(|end| buffer.names.get_mut(start..end))
        {
            Some(bytes) => match name.copy(memory, bytes) {
                Ok(()) => {
                    entry.name = Some((start, name.byte_len()));
                    buffer.names_len += name.byte_len();
                }
                Err(reason) => entry.name_reason = Some(reason),
            },
            None => entry.name_reason = Some("fuel recipient stored name does not fit"),
        }
    }
    if let Some(reason) = entry.name_reason {
        let _ = REFUSAL.set(reason);
        COUNTERS[8].fetch_add(1, Ordering::Relaxed);
    }
    let (index, slot) = log.reserve();
    buffer.entries[slot] = entry;
    log.finish(index);
}

unsafe extern "system-unwind" fn observe(registers: &Registers, tag: usize) {
    if !LOG.enabled.load(Ordering::SeqCst) {
        return;
    }
    let _call = LOG.enter();
    let memory = unsafe { LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    if tag == 9 {
        let status = registers.rsi;
        if let Some(mut pending) = PENDING.get()
            && pending.started.status == status
        {
            pending.refused = true;
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
            let _ = REFUSAL.set("fuel reset observation lost");
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
                }));
                if old.is_some() {
                    COUNTERS[0].fetch_add(1, Ordering::Relaxed);
                    PENDING.with(|pending| {
                        if let Some(mut p) = pending.get() {
                            p.refused = true;
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
    }
    if tag == 1 {
        PENDING.set(None);
        let settled = native::settle(&memory, base, pending.started, registers.rbx);
        if let Err(reason) = &settled {
            let _ = REFUSAL.set(reason);
        }
        if !settled
            .as_ref()
            .is_ok_and(|s| s.received == pending.received)
        {
            pending.refused = true;
        }
        if pending.refused {
            let _ = REFUSAL
                .set("fuel award root refused or receipts do not conserve native consumer totals");
        }
        if pending.refused {
            COUNTERS[2].fetch_add(1, Ordering::Relaxed);
        }
        append(
            Entry {
                root: pending.root,
                country: pending.started.country,
                started: pending.started,
                settled: settled.unwrap_or_default(),
                refused: pending.refused,
                ..Default::default()
            },
            None,
            &memory,
        );
        return;
    }
    let (kind, pointer, amount) = match tag {
        2..=6 => (0, registers.rax, registers.rdx as i64),
        7 => (2, registers.rcx, registers.rdx as i64),
        8 => (1, registers.rdi, registers.rbx as i64),
        _ => {
            pending.refused = true;
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
            }
        }
        Err(reason) => {
            let _ = REFUSAL.set(reason);
            pending.refused = true;
            COUNTERS[3].fetch_add(1, Ordering::Relaxed);
        }
    }
    PENDING.set(Some(pending));
}

pub fn drain(
    entries: &mut [Entry],
    names: &mut [u8],
) -> Result<(Drained, usize, usize), &'static str> {
    let Ok(mut storage) = STORAGE.try_lock() else {
        return Err("fuel observation storage busy");
    };
    let Some(buffer) = storage.current.as_mut() else {
        return Err("fuel observation storage absent");
    };
    handover(
        &LOG,
        buffer,
        entries,
        names,
        UNHOOKED.get().copied().or_else(|| {
            (!LOG.enabled.load(Ordering::SeqCst)).then_some("fuel award hooks disabled")
        }),
    )
}
fn handover(
    log: &Log,
    buffer: &mut Buffer,
    entries: &mut [Entry],
    names: &mut [u8],
    unavailable: Option<&'static str>,
) -> Result<(Drained, usize, usize), &'static str> {
    if names.len() < buffer.names_len {
        return Err("fuel name handover does not fit");
    }
    let before = log.consumed.load(Ordering::Relaxed);
    let mut drained = log.drain(
        entries,
        |slot| buffer.entries[slot],
        || unavailable,
        "fuel award observation restarted",
        DrainCounters {
            drains: &COUNTERS[4],
            copied: &COUNTERS[7],
            overflow: &COUNTERS[2],
            unobserved: &COUNTERS[1],
        },
    )?;
    let attempted = usize::try_from(drained.cursor.wrapping_sub(before))
        .map_err(|_| "fuel observation interval size exceeds address space")?;
    buffer.entries_required = buffer.entries_required.max(attempted);
    drained.pending = drained.pending.max(attempted);
    names[..buffer.names_len].copy_from_slice(&buffer.names[..buffer.names_len]);
    let len = buffer.names_len;
    let required = buffer.names_required;
    // The interval sum resets, but off-thread growth still needs its pressure.
    buffer.names_demand = buffer.names_demand.max(required);
    if drained.first + drained.copied as u64 == drained.cursor {
        buffer.names_len = 0;
        buffer.names_required = 0;
    }
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
            let current = s
                .current
                .as_ref()
                .map(|b| (b.entries.len(), b.names.len()))
                .unwrap_or_default();
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
    crate::status::status!({ "unhooked": UNHOOKED.get(), "enabled": LOG.enabled.load(Ordering::SeqCst),
        "first_refusal": REFUSAL.get(),
        "root_read_refusals": COUNTERS[0].load(Ordering::Relaxed),
        "orphan_operands": COUNTERS[1].load(Ordering::Relaxed),
        "settled_refusals": COUNTERS[2].load(Ordering::Relaxed),
        "recipient_refusals": COUNTERS[3].load(Ordering::Relaxed),
        "census_refusals": COUNTERS[5].load(Ordering::Relaxed),
        "census_last_refusal": *CENSUS_LAST_REFUSAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner),
        "name_refusals": COUNTERS[8].load(Ordering::Relaxed),
        "reset_refusals": COUNTERS[9].load(Ordering::Relaxed),
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
        let mut hooks: Vec<_> = self.taps.iter_mut().map(Tap::publication_hook).collect();
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
