use crate::{
    clock::qpc,
    event_ledger::{Event, EventLedger},
    hook::Hook,
};
type Clock = engine_native::cgame_state::Clock<u64>;
use engine_events::building_damage_sites as native;
use engine_native_memory::Memory;
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
pub fn buildings() -> usize {
    BUILDINGS.load(Ordering::Acquire)
}
/// # Safety
/// Run only while building-bombing observer storage remains live for the active session (or an
/// isolated test owning building-bombing observation) and teardown is excluded;
/// `EventLedger::prepare` synchronizes spare and retired buffers across concurrent growth callers.
pub unsafe fn presize(events: usize) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.prepare(events);
}
/// # Safety
/// Caller must own the active session's recovery or publication failure path (or an isolated test
/// owning building-bombing observation) when invalidating building-bombing drains.
pub unsafe fn lost_drain() {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    // SAFETY: Caller owns the building-bombing recovery or publication failure path.
    unsafe { LEDGER.lost_drain() };
}
/// # Safety
/// Caller must own the active session's projection or publication failure path (or an isolated
/// test owning building-bombing observation) when recording lost building-bombing output.
pub unsafe fn lost_output(events: usize) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    // SAFETY: Caller owns the building-bombing projection or publication failure path.
    unsafe { LEDGER.lost_output(events) };
}
fn writer_cost(start: i64) {
    let ticks = qpc().saturating_sub(start).max(0) as u64;
    WRITER_TICKS.fetch_add(ticks, Ordering::Relaxed);
    INTERVAL_TICKS.fetch_add(ticks, Ordering::Relaxed);
}

/// # Safety
/// Run only during attach or the active session's serial sizing pass against the authenticated
/// session image, while building-bombing observer storage remains live and teardown is excluded.
pub unsafe fn prepare(memory: &impl Memory, base: usize, countries: usize) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    LEDGER.prepare(0);
    match collector_extended_support::demand::demand_game_state(memory, base) {
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
    if buildings.checked_add(countries).is_some() {
        LEDGER.prepare(countries.max(256));
    } else {
        LEDGER.refuse_preparation("building bombing preparation population size overflow");
    }
}
/// # Safety
/// Caller must run on the admitted hourly capture boundary as the sole consumer advancing
/// building-bombing observation (or an isolated test owning building-bombing observation).
pub unsafe fn boundary(clock: Option<Clock>) {
    #[cfg(test)]
    if !tests::owns_observation() {
        return;
    }
    let ticks = INTERVAL_TICKS.swap(0, Ordering::Relaxed);
    LAST_HOUR_TICKS.store(ticks, Ordering::Relaxed);
    MAX_HOUR_TICKS.fetch_max(ticks, Ordering::Relaxed);
    // SAFETY: Caller runs on the admitted hourly capture boundary for building-bombing observation.
    unsafe { LEDGER.boundary(clock) };
}
/// # Safety
/// Caller must be the admitted hourly capture consumer draining building-bombing events while
/// observer storage stays live and teardown is excluded.
pub unsafe fn drain(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    drain_admitted()?;
    // SAFETY: Caller holds hourly capture admission as the sole building-bombing consumer.
    unsafe { LEDGER.drain(output) }
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
        && let Some(index) = engine_sites::reference(BASE.load(Ordering::Relaxed), caller)
            .and_then(|key| native::CALLERS.iter().position(|(known, _)| *known == key))
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
                let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
                let game = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
                    .and_then(|a| engine_native_memory::read::<u64>(&memory, a))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|p| *p != 0)
                    .ok_or("building bombing game state unreadable")?;
                let hour = game
                    .checked_add(engine_native::cgame_state::CURRENT_HOUR)
                    .and_then(|a| engine_native_memory::read::<i32>(&memory, a))
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
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            let base = BASE.load(Ordering::Relaxed);
            if engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
                .and_then(|a| engine_native_memory::read::<u64>(&memory, a))
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
            let credited = engine_native_memory::read::<i64>(&memory, returned)
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
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Self {
        BASE.store(base, Ordering::Relaxed);
        let mut frequency = 0;
        if unsafe {
            windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&raw mut frequency)
        } != 0
            && frequency > 0
        {
            FREQUENCY.store(frequency as u64, Ordering::Relaxed);
        }
        let install = || -> Result<Hook, &'static str> {
            crate::located(base, native::APPLY_DAMAGE_RVA)?;
            engine_sites::require(base, &native::CALLER_KEYS)?;
            let code = engine_sites::hook(base, native::APPLY_DAMAGE_RVA)?;
            let hook = unsafe {
                Hook::new(
                    code.address,
                    building_damage_entry as *const () as usize,
                    code.sentinel::<16>()?,
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
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Construction,
                    None,
                    reason,
                );
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

#[cfg(test)]
mod tests;
