//! Signed building health removals, observed without querying native code.
//! A lost interval is unavailable; unresolved execution retains partial recovery.
use crate::{
    event_ledger::{Event, EventLedger},
    hook::Hook,
    sampling::qpc,
};
type Clock = engine::cgame_state::Clock<u64>;
use engine::{Memory, building_damage_sites as native};
use std::{
    ops::Range,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub game_hour: i32,
    pub attacker: Option<u32>,
    pub victim: u32,
    pub state: u32,
    pub province: Option<u32>,
    pub building_type: u32,
    pub native_cause: u32,
    pub cause: native::Cause,
    pub amount_raw: i64,
    pub credited_raw: i64,
}

impl Event for Entry {
    fn set_id(&mut self, id: u64) {
        self.id = id;
    }
}
static LEDGER: EventLedger<Entry> = EventLedger::new();
static BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
static BUILDINGS: AtomicUsize = AtomicUsize::new(0);
static UNATTRIBUTED: AtomicU64 = AtomicU64::new(0);
static ATTRIBUTION_FAILURES: AtomicU64 = AtomicU64::new(0);
static UNKNOWN_CALLERS: AtomicU64 = AtomicU64::new(0);
static CALLER_CALLS: [AtomicU64; native::CALLERS.len()] =
    [const { AtomicU64::new(0) }; native::CALLERS.len()];
static FREQUENCY: AtomicU64 = AtomicU64::new(1);
static WRITER_TICKS: AtomicU64 = AtomicU64::new(0);
static INTERVAL_TICKS: AtomicU64 = AtomicU64::new(0);
static LAST_HOUR_TICKS: AtomicU64 = AtomicU64::new(0);
static MAX_HOUR_TICKS: AtomicU64 = AtomicU64::new(0);
#[derive(Default)]
#[repr(C)]
struct Observation {
    generation: u64,
    snapshot: native::Snapshot,
    game: usize,
    output: usize,
    valid: bool,
}
const _: () = assert!(std::mem::size_of::<Observation>() <= 0xa0);
pub fn active() -> usize {
    LEDGER.active.load(Ordering::SeqCst)
}
pub fn restore_sequence(previous: Option<u64>) -> Result<(), &'static str> {
    if ENABLED.load(Ordering::Acquire) {
        return Err("building bombing sequence restoration requires disabled hooks");
    }
    LEDGER.restore_sequence(previous)
}
pub fn mark_stranded() {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.mark_stranded();
}
pub fn capacity() -> usize {
    LEDGER.demand()
}
/// The latest building census, which also sizes the nuclear strike ledger.
pub fn buildings() -> usize {
    BUILDINGS.load(Ordering::Acquire)
}
pub fn presize(events: usize) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.prepare(events);
}
pub fn lost_drain() {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.lost_drain();
}
pub fn lost_output(events: usize) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.lost_output(events);
}
fn writer_cost(start: i64) {
    let ticks = qpc().saturating_sub(start).max(0) as u64;
    WRITER_TICKS.fetch_add(ticks, Ordering::Relaxed);
    INTERVAL_TICKS.fetch_add(ticks, Ordering::Relaxed);
}
fn ns(ticks: u64) -> u64 {
    ((u128::from(ticks) * 1_000_000_000) / u128::from(FREQUENCY.load(Ordering::Relaxed)))
        .min(u128::from(u64::MAX)) as u64
}

/// Off-thread census; overflow demand grows even when the census is unavailable.
pub fn prepare(memory: &impl Memory, base: usize, countries: usize) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.prepare(0);
    match crate::country_scale::demand_game_state(memory, base) {
        Ok(Some(_)) => {}
        Ok(None) => return,
        Err(reason) => {
            LEDGER.refuse_preparation(reason);
            return;
        }
    }
    let Some(buildings) = native::building_count(memory, base) else {
        LEDGER.refuse_preparation("building bombing preparation building roster unreadable");
        return;
    };
    BUILDINGS.store(buildings, Ordering::Release);
    if let Some(demand) = buildings.checked_add(countries) {
        LEDGER.prepare(demand);
    } else {
        LEDGER.refuse_preparation("building bombing preparation population size overflow");
    }
}
pub fn boundary(clock: Option<Clock>) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    let ticks = INTERVAL_TICKS.swap(0, Ordering::Relaxed);
    LAST_HOUR_TICKS.store(ticks, Ordering::Relaxed);
    MAX_HOUR_TICKS.fetch_max(ticks, Ordering::Relaxed);
    LEDGER.boundary(clock);
}
#[cfg(test)]
pub fn drain(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    drain_admitted()?;
    LEDGER.drain(output)
}
pub fn drain_growing(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    drain_admitted()?;
    LEDGER.drain_growing(output)
}
fn drain_admitted() -> Result<(), &'static str> {
    #[cfg(test)]
    if !tests::owns_observation() {
        return Err("building bombing hook unavailable (see status unhooked)");
    }
    if UNHOOKED.get().is_some() || !ENABLED.load(Ordering::SeqCst) {
        return Err("building bombing hook unavailable (see status unhooked)");
    }
    Ok(())
}
// Reviewed Win64 frame: native call is outside Rust; .seh_stackalloc covers
// the owned snapshot. Relocation moves only MOV [RSP+20],RBX (five bytes).
core::arch::global_asm!(
    ".text", ".globl building_damage_entry", ".seh_proc building_damage_entry",
    "building_damage_entry:", "sub rsp, 0x138", ".seh_stackalloc 0x138", ".seh_endprologue",
    "mov [rsp + 0x40], rcx", "mov [rsp + 0x48], rdx",
    "mov [rsp + 0x50], r8", "mov [rsp + 0x58], r9",
    "mov rax, [rsp + 0x160]", "mov [rsp + 0x60], rax", "mov [rsp + 0x20], rax",
    "mov rax, [rsp + 0x168]", "mov [rsp + 0x68], rax", "mov [rsp + 0x28], rax",
    "mov rax, [rsp + 0x138]", "mov [rsp + 0x70], rax", "mov [rsp + 0x30], rax",
    "lea rax, [rsp + 0x90]", "mov [rsp + 0x38], rax",
    "call {before}", "mov [rsp + 0x78], rax",
    "mov rcx, [rsp + 0x40]", "mov rdx, [rsp + 0x48]",
    "mov r8, [rsp + 0x50]", "mov r9, [rsp + 0x58]",
    "mov rax, [rsp + 0x60]", "mov [rsp + 0x20], rax",
    "mov rax, [rsp + 0x68]", "mov [rsp + 0x28], rax",
    ".globl building_damage_native_call", "building_damage_native_call:",
    "call qword ptr [rip + {original}]",
    ".globl building_damage_native_return", "building_damage_native_return:",
    "mov [rsp + 0x80], rax",
    "mov rcx, [rsp + 0x78]", "lea rdx, [rsp + 0x90]", "mov r8, [rsp + 0x80]", "call {after}",
    "mov rax, [rsp + 0x80]", "add rsp, 0x138", "ret", ".seh_endproc",
    before = sym before, after = sym after, original = sym ORIGINAL,
);
unsafe extern "system" {
    fn building_damage_entry();
    #[cfg(test)]
    fn building_damage_native_call();
    #[cfg(test)]
    fn building_damage_native_return();
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn before(
    building: usize,
    output: usize,
    request: usize,
    _notify: usize,
    source: usize,
    _extra: usize,
    caller: usize,
    observation: *mut Observation,
) -> usize {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::BuildingBombing);
    let started = qpc();
    LEDGER.enter();
    unsafe {
        observation.write(Observation {
            output,
            ..Default::default()
        });
    }
    if ENABLED.load(Ordering::SeqCst)
        && let Some(index) = caller
            .checked_sub(BASE.load(Ordering::Relaxed))
            .and_then(|rva| native::CALLERS.iter().position(|(known, _)| *known == rva))
    {
        CALLER_CALLS[index].fetch_add(1, Ordering::Relaxed);
    }
    let mut token = 0;
    if ENABLED.load(Ordering::SeqCst) && request as i64 > 0 {
        LEDGER.attempt();
        if LEDGER.poisoned.load(Ordering::SeqCst) || crate::allocation::failed() {
            LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
        } else {
            let result = (|| {
                let generation = crate::world_lifetime::generation();
                if crate::world_lifetime::invalidating() {
                    return Err("building bombing native world awaits hourly boundary");
                }
                token = LEDGER.claim((building, 0))?;
                let base = BASE.load(Ordering::Relaxed);
                let memory = unsafe { engine::memory::LiveMemory::new() };
                let game = base
                    .checked_add(engine::cgame_state::SINGLETON_RVA)
                    .and_then(|a| engine::read::<u64>(&memory, a))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|p| *p != 0)
                    .ok_or("building bombing game state unreadable")?;
                let hour = game
                    .checked_add(engine::cgame_state::CURRENT_HOUR)
                    .and_then(|a| engine::read::<i32>(&memory, a))
                    .ok_or("building bombing clock unreadable")?;
                let snapshot =
                    native::snapshot(&memory, base, game, building, source, caller, hour)?;
                unsafe {
                    (*observation).snapshot = snapshot;
                    (*observation).generation = generation;
                    (*observation).game = game;
                    (*observation).valid = true;
                }
                Ok::<_, &'static str>(())
            })();
            if let Err(reason) = result {
                if token != 0 {
                    let _ = LEDGER.complete(token, Ok(None));
                    token = 0;
                }
                LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
                LEDGER.refuse(reason);
            }
        }
    } else {
        LEDGER.zero_writes.fetch_add(1, Ordering::Relaxed);
    }
    writer_cost(started);
    token
}
unsafe extern "system" fn after(token: usize, observation: *const Observation, returned: usize) {
    let _timing = crate::hook_timing::enter_return(crate::hook_timing::Family::BuildingBombing);
    let started = qpc();
    if token != 0 {
        let observation = unsafe { &*observation };
        let mut applied = None;
        let result = (|| {
            if !observation.valid {
                return Ok(None);
            }
            if observation.generation != crate::world_lifetime::generation()
                || crate::world_lifetime::invalidating()
            {
                return Err("building bombing native world changed during original");
            }
            let memory = unsafe { engine::memory::LiveMemory::new() };
            let base = BASE.load(Ordering::Relaxed);
            if base
                .checked_add(engine::cgame_state::SINGLETON_RVA)
                .and_then(|a| engine::read::<u64>(&memory, a))
                != Some(observation.game as u64)
            {
                return Err("building bombing game root changed during native call");
            }
            let amount = native::applied_damage(&memory, base, &observation.snapshot)?;
            applied = Some(amount);
            if amount == 0 {
                return Ok(None);
            }
            let snapshot = native::attribution(&memory, base, &observation.snapshot)?;
            if returned == 0 || returned != observation.output {
                return Err("building bombing return pointer differs");
            }
            let credited = engine::read::<i64>(&memory, returned)
                .ok_or("building bombing credited damage unreadable")?;
            if snapshot.attacker.is_none() {
                UNATTRIBUTED.fetch_add(1, Ordering::Relaxed);
            }
            if snapshot.attribution_failed {
                ATTRIBUTION_FAILURES.fetch_add(1, Ordering::Relaxed);
            }
            if snapshot.cause == native::Cause::Unknown {
                UNKNOWN_CALLERS.fetch_add(1, Ordering::Relaxed);
            }
            Ok(Some(Entry {
                id: 0,
                game_hour: snapshot.game_hour,
                attacker: snapshot.attacker,
                victim: snapshot.victim,
                state: snapshot.state,
                province: snapshot.province,
                building_type: snapshot.building_type,
                native_cause: snapshot.native_cause,
                cause: snapshot.cause,
                amount_raw: amount,
                credited_raw: credited,
            }))
        })();
        if let Err(reason) = LEDGER.complete(token, result) {
            match applied {
                Some(amount) if amount != 0 => {
                    LEDGER.lost_events.fetch_add(1, Ordering::Relaxed);
                }
                None if observation.valid => {
                    LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
            LEDGER.refuse(reason);
        }
    }
    writer_cost(started);
    LEDGER.leave();
}
pub struct Hooks {
    hook: Option<Hook>,
}
impl Hooks {
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        BASE.store(base, Ordering::Relaxed);
        let mut frequency = 0;
        if unsafe {
            windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
        } != 0
            && frequency > 0
        {
            FREQUENCY.store(frequency as u64, Ordering::Relaxed);
        }
        let install = || -> Result<Hook, &'static str> {
            if engine::read::<[u8; 16]>(memory, base + native::APPLY_DAMAGE_RVA)
                != Some(native::APPLY_DAMAGE_SENTINEL)
            {
                return Err("building bombing sentinel differs or unreadable");
            }
            let hook = unsafe {
                Hook::new(
                    base + native::APPLY_DAMAGE_RVA,
                    building_damage_entry as *const () as usize,
                    native::APPLY_DAMAGE_SENTINEL,
                )
            }
            .map_err(|_| "building bombing hook construction failed")?;
            ORIGINAL.store(hook.trampoline(), Ordering::Release);
            Ok(hook)
        };
        match install() {
            Ok(hook) => {
                INSTALLED.store(true, Ordering::Release);
                Self { hook: Some(hook) }
            }
            Err(reason) => {
                let _ = UNHOOKED.set(reason);
                Self { hook: None }
            }
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        if let Some(hook) = &mut self.hook {
            if !hook
                .set_enabled(enabled)
                .map_err(|_| "building bombing hook state change failed")?
            {
                return Ok(false);
            }
            ENABLED.store(enabled, Ordering::SeqCst);
        }
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hook.iter().flat_map(Hook::executable_ranges)
    }
}
pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "rva": native::APPLY_DAMAGE_RVA, "sentinel": native::APPLY_DAMAGE_SENTINEL,
        "installed": INSTALLED.load(Ordering::Acquire), "enabled": ENABLED.load(Ordering::SeqCst),
        "active": active(), "stranded": LEDGER.stranded.load(Ordering::SeqCst), "unhooked": UNHOOKED.get(), "branch_unavailable": LEDGER.poisoned.load(Ordering::SeqCst),
        "calls": LEDGER.calls.load(Ordering::Relaxed), "events": LEDGER.events.load(Ordering::Relaxed),
        "overflow": LEDGER.overflow.load(Ordering::Relaxed), "refusals": LEDGER.refusals.load(Ordering::Relaxed),
        "lost_events": LEDGER.lost_events.load(Ordering::Relaxed), "lost_drains": LEDGER.lost_drains.load(Ordering::Relaxed),
        "lost_events_unknown": LEDGER.lost_events_unknown.load(Ordering::Relaxed),
        "overlaps": LEDGER.overlaps.load(Ordering::Relaxed), "drains": LEDGER.drains.load(Ordering::Relaxed),
        "handed_over": LEDGER.handed_over.load(Ordering::Relaxed), "event_capacity": LEDGER.capacity.load(Ordering::Acquire), "event_required": capacity(),
        "pending_claim_capacity": 1,
        "partial_intervals": LEDGER.partial_intervals.load(Ordering::Relaxed), "partial_events": LEDGER.partial_events.load(Ordering::Relaxed),
        "building_count": BUILDINGS.load(Ordering::Acquire), "prepare_failures": LEDGER.prepare_failures.load(Ordering::Relaxed),
        "prepare_last_refusal": LEDGER.prepare_last_refusal(),
        "unattributed": UNATTRIBUTED.load(Ordering::Relaxed),
        "attribution_failures": ATTRIBUTION_FAILURES.load(Ordering::Relaxed),
        "unknown_callers": UNKNOWN_CALLERS.load(Ordering::Relaxed),
        "caller_calls": crate::status::sequence(native::CALLERS.iter().zip(&CALLER_CALLS)
            .map(|((rva, _), count)| (*rva, count.load(Ordering::Relaxed)))),
        "hourly_peak_events": LEDGER.hourly_peak_events.load(Ordering::Relaxed), "hourly_peak_capacity": LEDGER.hourly_peak_capacity.load(Ordering::Relaxed),
        "writer_ns": ns(WRITER_TICKS.load(Ordering::Relaxed)), "last_hour_writer_ns": ns(LAST_HOUR_TICKS.load(Ordering::Relaxed)),
        "max_hour_writer_ns": ns(MAX_HOUR_TICKS.load(Ordering::Relaxed)),
        "zero_writes": LEDGER.zero_writes.load(Ordering::Relaxed),
        "scope": "since attach; observer time excludes native work; first/recovery partial intervals withheld"
    })
}

#[cfg(test)]
mod tests;
