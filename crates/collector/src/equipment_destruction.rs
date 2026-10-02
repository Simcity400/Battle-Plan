//! Destruction writers only. Every event uses its native owner and exact pool
//! difference. Try-locks never wait on a game thread; overlap, refusal, overflow
//! or a lost drain breaks completeness for the remainder of that branch.
use crate::hook::Hook;
use engine::{Memory, cgame_state::Clock, equipment_destruction as native, equipment_entry};
use std::{
    ops::Range,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub country: u32,
    pub cause: native::Cause,
    pub variant: u64,
    pub amount_raw: i64,
}

struct Store {
    entries: Vec<Entry>,
    before: Vec<equipment_entry::Row>,
    after: Vec<equipment_entry::Row>,
    raw: Vec<u8>,
    transition: bool,
    refusal: Option<&'static str>,
}
impl Store {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            raw: Vec::new(),
            transition: true,
            refusal: None,
        }
    }
    fn append(&mut self, entry: Entry) -> Result<(), &'static str> {
        if entry.amount_raw == 0 {
            return Ok(());
        }
        if entry.amount_raw < 0 {
            return Err("destruction writer increased equipment pool");
        }
        if self.entries.len() == self.entries.capacity() {
            OVERFLOW.fetch_add(1, Ordering::Relaxed);
            return Err("equipment destruction event log overflow");
        }
        self.entries.push(entry); // capacity checked; never allocates
        EVENTS.fetch_add(1, Ordering::Relaxed);
        record_hourly_peak(self.entries.len(), self.entries.capacity());
        Ok(())
    }
    fn refuse(&mut self, reason: &'static str) {
        self.refusal.get_or_insert(reason);
        REFUSALS.fetch_add(1, Ordering::Relaxed);
        poison();
    }
    fn boundary(&mut self, transition: bool) {
        self.transition = transition;
        if transition {
            self.entries.clear();
            self.refusal = None;
            POISONED.store(false, Ordering::SeqCst);
        }
    }
}
static STORE: Mutex<Store> = Mutex::new(Store::new());
// Only the hourly boundary writes this clock. It advances even if a writer
// owns STORE, so an overlapping drain cannot masquerade as a later branch.
static CLOCK: Mutex<Option<Clock>> = Mutex::new(None);
static BASE: AtomicUsize = AtomicUsize::new(0);
static CAPACITY: AtomicUsize = AtomicUsize::new(0);
static DESIGNS: AtomicUsize = AtomicUsize::new(0);
static HOURLY_PEAK_EVENTS: AtomicUsize = AtomicUsize::new(0);
static HOURLY_PEAK_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
static ACTIVE: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static ORIGINAL: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static INSTALLED: [AtomicBool; 4] = [const { AtomicBool::new(false) }; 4];
static SITE_ENABLED: [AtomicBool; 4] = [const { AtomicBool::new(false) }; 4];
static CALLS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static UNHOOKED: OnceLock<String> = OnceLock::new();
static EVENTS: AtomicU64 = AtomicU64::new(0);
static OVERFLOW: AtomicU64 = AtomicU64::new(0);
static REFUSALS: AtomicU64 = AtomicU64::new(0);
static LOST: AtomicU64 = AtomicU64::new(0);
static DRAINS: AtomicU64 = AtomicU64::new(0);
static HANDED_OVER: AtomicU64 = AtomicU64::new(0);
static OVERLAPS: AtomicU64 = AtomicU64::new(0);
static PREPARE_FAILURES: AtomicU64 = AtomicU64::new(0);
static UNKNOWN_CALLERS: AtomicU64 = AtomicU64::new(0);

pub fn active() -> usize {
    ACTIVE.iter().map(|v| v.load(Ordering::SeqCst)).sum()
}
pub fn capacity() -> usize {
    CAPACITY.load(Ordering::Acquire)
}
fn record_hourly_peak(events: usize, capacity: usize) {
    if events > HOURLY_PEAK_EVENTS.load(Ordering::Relaxed) {
        // All producers hold STORE, so the capacity belongs to this peak.
        // Partial and refused intervals remain included as headroom evidence.
        HOURLY_PEAK_CAPACITY.store(capacity, Ordering::Relaxed);
        HOURLY_PEAK_EVENTS.store(events, Ordering::Relaxed);
    }
}
fn count_cause(cause: native::Cause) -> native::Cause {
    if cause == native::Cause::Unknown {
        UNKNOWN_CALLERS.fetch_add(1, Ordering::Relaxed);
    }
    cause
}
fn poison() {
    POISONED.store(true, Ordering::SeqCst);
}
pub fn lost_drain() {
    LOST.fetch_add(1, Ordering::Relaxed);
    poison();
}

/// Allocate only before enabling, or on the spool thread. Grow from live
/// country slots plus saved design census, per native writer site. Capacity is
/// an event bound, not a promise of one event per design: repeat bursts can
/// overflow and then explicitly withhold the table. Build new storage before
/// taking the lock; retain pending events when installing a larger buffer.
/// A failed sizing observation skips growth and counts a prepare failure; it
/// cannot lose a recorded event or invalidate the existing branch observation.
pub fn prepare(memory: &impl Memory, base: usize, countries: usize) {
    let Some(designs) = native::design_count(memory, base) else {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(events) = log_capacity(countries, designs) else {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if events <= capacity() && designs <= DESIGNS.load(Ordering::Acquire) {
        return;
    }
    let events = events.max(capacity());
    let designs = designs.max(DESIGNS.load(Ordering::Acquire));
    let Some(raw_bytes) = designs.checked_mul(equipment_entry::STRIDE) else {
        PREPARE_FAILURES.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let mut entries = Vec::new();
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut raw = Vec::new();
    if entries.try_reserve_exact(events).is_err()
        || before.try_reserve_exact(designs).is_err()
        || after.try_reserve_exact(designs).is_err()
        || raw.try_reserve_exact(raw_bytes).is_err()
    {
        if let Ok(mut store) = STORE.try_lock() {
            store.refuse("equipment destruction buffer allocation failed");
        } else {
            REFUSALS.fetch_add(1, Ordering::Relaxed);
            poison();
        }
        return;
    }
    before.resize(designs, equipment_entry::Row::default());
    after.resize(designs, equipment_entry::Row::default());
    raw.resize(raw_bytes, 0);
    let Ok(mut store) = STORE.try_lock() else {
        return;
    };
    entries.extend_from_slice(&store.entries);
    let old = (
        std::mem::replace(&mut store.entries, entries),
        std::mem::replace(&mut store.before, before),
        std::mem::replace(&mut store.after, after),
        std::mem::replace(&mut store.raw, raw),
    );
    CAPACITY.store(store.entries.capacity(), Ordering::Release);
    DESIGNS.store(designs, Ordering::Release);
    drop(store);
    drop(old);
}

fn log_capacity(countries: usize, designs: usize) -> Option<usize> {
    // An event bound, not the country x global-design cross product. Repeated
    // bursts may exceed it; overflow then withholds the entire branch table.
    countries
        .checked_add(designs)?
        .checked_mul(native::SITES.len())
}

/// Called before acquisition (also when no reusable frame exists). First,
/// load and discontinuity boundaries discard the previous partial interval.
pub fn boundary(clock: Option<Clock>) {
    let Some(clock) = clock else {
        lost_drain();
        return;
    };
    let Ok(mut previous) = CLOCK.try_lock() else {
        lost_drain();
        return;
    };
    let transition = !previous.is_some_and(|before| {
        before.hour.checked_add(1) == Some(clock.hour)
            && before.campaign_id == clock.campaign_id
            && before.play_session_started_at == clock.play_session_started_at
    });
    *previous = Some(clock);
    drop(previous);
    match STORE.try_lock() {
        Ok(mut store) => store.boundary(transition),
        Err(_) => {
            OVERLAPS.fetch_add(1, Ordering::Relaxed);
            poison();
        }
    }
}

pub fn drain(output: &mut [Entry]) -> Result<usize, &'static str> {
    DRAINS.fetch_add(1, Ordering::Relaxed);
    if UNHOOKED.get().is_some() || !ENABLED.load(Ordering::SeqCst) {
        return Err("equipment destruction hooks unavailable (see status unhooked)");
    }
    let Ok(mut store) = STORE.try_lock() else {
        lost_drain();
        return Err("equipment destruction writer overlaps hourly drain");
    };
    if POISONED.load(Ordering::SeqCst) {
        return Err(store
            .refusal
            .unwrap_or("equipment destruction lost observation; branch unavailable"));
    }
    if store.transition {
        return Err("equipment destruction first/branch interval is partial");
    }
    if active() != 0 {
        lost_drain();
        return Err("equipment destruction active writer at drain");
    }
    let len = store.entries.len();
    if len > output.len() {
        lost_drain();
        return Err("equipment destruction drain storage too small");
    }
    output[..len].copy_from_slice(&store.entries);
    // A producer could have failed try_lock while this drain held it.
    if POISONED.load(Ordering::SeqCst) || active() != 0 {
        lost_drain();
        return Err("equipment destruction writer overlaps hourly drain");
    }
    store.entries.clear();
    HANDED_OVER.fetch_add(len as u64, Ordering::Relaxed);
    Ok(len)
}

struct Call(usize);
impl Call {
    fn enter(site: usize) -> Self {
        ACTIVE[site].fetch_add(1, Ordering::SeqCst);
        CALLS[site].fetch_add(1, Ordering::Relaxed);
        Self(site)
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        ACTIVE[self.0].fetch_sub(1, Ordering::SeqCst);
    }
}

/// No observation failure may prevent the original writer from running.
/// The lock spans pre/native/post on these measured simulation-thread writers.
/// An unexpected concurrent/nested writer runs normally, refuses observation
/// and never waits or aliases the scratch. No game function is called to query.
fn observe_pool(
    store: &mut Store,
    site: usize,
    receiver: usize,
    cause: native::Cause,
    memory: &impl Memory,
    base: usize,
    original: impl FnOnce(),
) -> Result<(), &'static str> {
    let pre = (|| {
        let country = match site {
            0 => native::army_owner(memory, receiver),
            2 => native::air_owner(memory, receiver),
            3 => native::garrison_owner(memory, receiver),
            _ => Err("unsupported pool destruction writer"),
        }?;
        let pool = native::pool(receiver, site)?;
        let len = native::snapshot(memory, base, pool, &mut store.before)?;
        Ok::<_, &'static str>((country, pool, len))
    })();
    original();
    let (country, pool, len) = pre?;
    let after = native::snapshot(memory, base, pool, &mut store.after)?;
    let mut failure = None;
    let entries = &mut store.entries;
    native::removals(
        &mut store.before[..len],
        &mut store.after[..after],
        |variant, amount_raw| {
            if entries.len() == entries.capacity() {
                OVERFLOW.fetch_add(1, Ordering::Relaxed);
                failure = Some("equipment destruction event log overflow");
            } else {
                entries.push(Entry {
                    country,
                    cause,
                    variant,
                    amount_raw,
                });
                EVENTS.fetch_add(1, Ordering::Relaxed);
                record_hourly_peak(entries.len(), entries.capacity());
            }
        },
    )?;
    failure.map_or(Ok(()), Err)
}

type ArmyFn = unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize, usize);
// The entry shim copies all four stack arguments and adds the original return
// PC as argument nine. Unlike a naked framed shim, its unwind is described.
core::arch::global_asm!(
    ".text", ".globl destruction_land_entry", ".seh_proc destruction_land_entry",
    "destruction_land_entry:", "sub rsp, 0x78", ".seh_stackalloc 0x78", ".seh_endprologue",
    "mov rax, [rsp + 0xa0]", "mov [rsp + 0x20], rax",
    "mov rax, [rsp + 0xa8]", "mov [rsp + 0x28], rax",
    "mov rax, [rsp + 0xb0]", "mov [rsp + 0x30], rax",
    "mov rax, [rsp + 0xb8]", "mov [rsp + 0x38], rax",
    "mov rax, [rsp + 0x78]", "mov [rsp + 0x40], rax",
    "call {body}", "add rsp, 0x78", "ret", ".seh_endproc", body = sym land);
unsafe extern "system" {
    fn destruction_land_entry();
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn land(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
    caller: usize,
) {
    let _call = Call::enter(0);
    let original: ArmyFn = unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    let run = || unsafe { original(a, b, c, d, e, f, g, h) };
    if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) || b == 0 {
        run();
        return;
    }
    let Ok(mut store) = STORE.try_lock() else {
        OVERLAPS.fetch_add(1, Ordering::Relaxed);
        poison();
        run();
        return;
    };
    let base = BASE.load(Ordering::Relaxed);
    let cause = count_cause(
        caller
            .checked_sub(base)
            .map(native::land_cause)
            .unwrap_or(native::Cause::Unknown),
    );
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let result = observe_pool(&mut store, 0, a, cause, &memory, base, run);
    if let Err(reason) = result {
        store.refuse(reason);
    }
}

/// A tail jump with no frame: the native three arguments remain unchanged.
#[unsafe(naked)]
unsafe extern "system" fn air_entry() {
    core::arch::naked_asm!("mov r9, [rsp]", "jmp {body}", body = sym air);
}
unsafe extern "system" fn air(a: usize, b: usize, c: usize, caller: usize) {
    let _call = Call::enter(2);
    let original: unsafe extern "system" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    let run = || unsafe { original(a, b, c) };
    if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) || b as u32 == 0 {
        run();
        return;
    }
    let Ok(mut store) = STORE.try_lock() else {
        OVERLAPS.fetch_add(1, Ordering::Relaxed);
        poison();
        run();
        return;
    };
    let base = BASE.load(Ordering::Relaxed);
    let cause = count_cause(
        caller
            .checked_sub(base)
            .map(native::air_cause)
            .unwrap_or(native::Cause::Unknown),
    );
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let result = if (b as u32 as i32) > 0 {
        observe_pool(&mut store, 2, a, cause, &memory, base, run)
    } else {
        run();
        Err("unsupported negative KillAirplanes request")
    };
    if let Err(reason) = result {
        store.refuse(reason);
    }
}
unsafe extern "system" fn garrison(a: usize, b: usize, c: usize) {
    let _call = Call::enter(3);
    let original: unsafe extern "system" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    let run = || unsafe { original(a, b, c) };
    if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) {
        run();
        return;
    }
    let Ok(mut store) = STORE.try_lock() else {
        OVERLAPS.fetch_add(1, Ordering::Relaxed);
        poison();
        run();
        return;
    };
    let memory = unsafe { engine::memory::LiveMemory::new() };
    if let Err(reason) = observe_pool(
        &mut store,
        3,
        a,
        native::Cause::Garrison,
        &memory,
        BASE.load(Ordering::Relaxed),
        run,
    ) {
        store.refuse(reason);
    }
}
fn observe_variant(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    pool: usize,
    variant: usize,
    original: impl FnOnce(),
) -> Result<(), &'static str> {
    let pre = (|| {
        let army = pool
            .checked_sub(engine::carmy::EQUIPMENT_POOL)
            .ok_or("attrition army address underflow")?;
        let country = native::army_owner(memory, army)?;
        let design = engine::cequipment_variant::identity(memory, base, variant)?;
        let before = native::variant_amount(memory, base, pool, variant, &mut store.raw)?;
        Ok::<_, &'static str>((country, design, before))
    })();
    original();
    let (country, design, before) = pre?;
    let after = native::variant_amount(memory, base, pool, variant, &mut store.raw)?;
    let amount_raw = before
        .checked_sub(after)
        .ok_or("destruction difference overflow")?;
    store.append(Entry {
        country,
        cause: native::Cause::Attrition,
        variant: design,
        amount_raw,
    })
}

unsafe extern "system" fn attrition(pool: usize, variant: usize, quantity: usize) {
    let _call = Call::enter(1);
    let original: unsafe extern "system" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    let run = || unsafe { original(pool, variant, quantity) };
    if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) {
        run();
        return;
    }
    let Ok(mut store) = STORE.try_lock() else {
        OVERLAPS.fetch_add(1, Ordering::Relaxed);
        poison();
        run();
        return;
    };
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let result = observe_variant(&mut store, &memory, base, pool, variant, run);
    if let Err(reason) = result {
        store.refuse(reason);
    }
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// All sentinels are checked before creating/enabling any family member.
    /// A refusal leaves the table unavailable while ordinary capture continues.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        BASE.store(base, Ordering::Relaxed);
        let callbacks = [
            destruction_land_entry as *const () as usize,
            attrition as *const () as usize,
            air_entry as *const () as usize,
            garrison as *const () as usize,
        ];
        let install = || -> Result<Vec<Hook>, String> {
            for site in native::SITES {
                if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                    return Err(format!(
                        "{} sentinel differs or unreadable at {:#x}",
                        site.name,
                        base + site.rva
                    ));
                }
            }
            let mut hooks = Vec::new();
            for (index, site) in native::SITES.into_iter().enumerate() {
                let hook = if index == 1 {
                    unsafe { Hook::new_call(base + site.rva, callbacks[index], site.sentinel) }
                } else {
                    unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                }?;
                ORIGINAL[index].store(
                    if index == 1 {
                        base + native::ATTRITION_TARGET
                    } else {
                        hook.trampoline()
                    },
                    Ordering::Release,
                );
                hooks.push(hook);
            }
            Ok(hooks)
        };
        match install() {
            Ok(hooks) => {
                for site in &INSTALLED {
                    site.store(true, Ordering::Release);
                }
                Self { hooks }
            }
            Err(reason) => {
                let _ = UNHOOKED.set(reason);
                Self { hooks: Vec::new() }
            }
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        for (index, hook) in self.hooks.iter_mut().enumerate() {
            if !hook.set_enabled(enabled)? {
                return Ok(false);
            }
            SITE_ENABLED[index].store(enabled, Ordering::SeqCst);
        }
        ENABLED.store(
            enabled && self.hooks.len() == native::SITES.len(),
            Ordering::SeqCst,
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .collect()
    }
}
pub fn status() -> serde_json::Value {
    let hooks: Vec<_> = native::SITES.iter().enumerate().map(|(i,site)| serde_json::json!({
        "name":site.name,"rva":site.rva,"sentinel":site.sentinel,"installed":INSTALLED[i].load(Ordering::Acquire),
        "enabled":SITE_ENABLED[i].load(Ordering::SeqCst),"active":ACTIVE[i].load(Ordering::SeqCst),"calls":CALLS[i].load(Ordering::Relaxed)
    })).collect();
    serde_json::json!({"hooks":hooks,"unhooked":UNHOOKED.get(),"branch_unavailable":POISONED.load(Ordering::SeqCst),
        "events":EVENTS.load(Ordering::Relaxed),"overflow":OVERFLOW.load(Ordering::Relaxed),"refusals":REFUSALS.load(Ordering::Relaxed),
        "lost_drains":LOST.load(Ordering::Relaxed),"overlaps":OVERLAPS.load(Ordering::Relaxed),"drains":DRAINS.load(Ordering::Relaxed),
        "handed_over":HANDED_OVER.load(Ordering::Relaxed),"event_capacity":capacity(),"design_capacity":DESIGNS.load(Ordering::Acquire),
        "prepare_failures":PREPARE_FAILURES.load(Ordering::Relaxed),
        "unknown_callers":UNKNOWN_CALLERS.load(Ordering::Relaxed),
        "hourly_peak_events":HOURLY_PEAK_EVENTS.load(Ordering::Relaxed),"hourly_peak_capacity":HOURLY_PEAK_CAPACITY.load(Ordering::Relaxed),
        "scope":"cumulative since attach; stable causes; unknown callers counted; peak includes partial or refused intervals; first and branch intervals are partial"})
}

#[cfg(test)]
mod tests;
