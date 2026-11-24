use crate::{
    event_ledger::{Event, EventLedger},
    hook::Hook,
};
use engine_mutations::combat_engagement_mutations as native;
use engine_native_memory::Memory;
use engine_native_parts::{cland_border_war_combat, cland_border_war_combatant};
use engine_native::{cland_combat, cland_combatant, cnaval_combat, cnaval_combatant};
use replay_data::rows::CombatEngagementRefreshRow;
use std::{
    cell::Cell,
    ops::Range,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

type Clock = engine_native::cgame_state::Clock<u64>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub kind: u8,
    pub target: u64,
    pub side: u8,
    pub viewer: u32,
    pub triggers: u8,
}

impl Event for Entry {
    fn set_id(&mut self, _id: u64) {}
}

static LEDGER: EventLedger<Entry> = EventLedger::new();
static BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; native::MUTATIONS.len()] =
    [const { AtomicUsize::new(0) }; native::MUTATIONS.len()];
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);
static UNHOOKED: OnceLock<&'static str> = OnceLock::new();
static FLIGHTS: AtomicUsize = AtomicUsize::new(0);

pub fn active() -> usize {
    FLIGHTS
        .load(Ordering::SeqCst)
        .saturating_add(LEDGER.active.load(Ordering::SeqCst))
}

fn foreign_test() -> bool {
    #[cfg(test)]
    if !tests::owns_observation() {
        return true;
    }
    false
}

pub fn mark_stranded() {
    if !foreign_test() {
        LEDGER.mark_stranded();
    }
}

pub fn capacity() -> usize {
    LEDGER.demand()
}

/// # Safety
/// Run only while combat-engagement-refresh observer storage remains live for the active session
/// (or an isolated test owning combat-engagement-refresh observation) and teardown is excluded.
pub unsafe fn presize(demand: usize) {
    if !foreign_test() {
        LEDGER.prepare(demand);
    }
}

/// # Safety
/// Caller must own the active session's recovery or publication failure path (or an isolated test
/// owning combat-engagement-refresh observation) when invalidating refresh drains.
pub unsafe fn lost_drain() {
    if !foreign_test() {
        // SAFETY: Caller owns the combat-engagement-refresh recovery or publication failure path.
        unsafe { LEDGER.lost_drain() };
    }
}

/// # Safety
/// Caller must own the active session's projection or publication failure path (or an isolated
/// test owning combat-engagement-refresh observation) when recording lost refresh output.
pub unsafe fn lost_output(entries: usize) {
    if !foreign_test() {
        // SAFETY: Caller owns the combat-engagement-refresh projection or publication failure path.
        unsafe { LEDGER.lost_output(entries) };
    }
}

/// # Safety
/// Caller must run on the admitted hourly capture boundary as the sole consumer advancing
/// combat-engagement-refresh observation (or an isolated test owning observation).
pub unsafe fn boundary(clock: Option<Clock>) {
    if !foreign_test() {
        // SAFETY: Caller runs on the admitted hourly capture boundary for combat-engagement-refresh observation.
        unsafe { LEDGER.boundary(clock) };
    }
}

/// # Safety
/// Caller must be the admitted hourly capture consumer draining combat-engagement-refresh events
/// while observer storage stays live and teardown is excluded.
pub unsafe fn drain(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    if foreign_test() || UNHOOKED.get().is_some() || !ENABLED.load(Ordering::SeqCst) {
        return Err("combat engagement refresh hooks unavailable (see status unhooked)");
    }
    // SAFETY: Caller holds hourly capture admission as the sole combat-engagement-refresh consumer.
    unsafe { LEDGER.drain(output) }
}

fn read_side_index(
    memory: &impl Memory,
    combat: usize,
    attacker_offset: usize,
    defender_offset: usize,
    combatant: usize,
) -> Result<u8, &'static str> {
    let attacker = combat
        .checked_add(attacker_offset)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .ok_or("combat engagement refresh combat attacker unreadable")?;
    let defender = combat
        .checked_add(defender_offset)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .ok_or("combat engagement refresh combat defender unreadable")?;
    if attacker == defender {
        return Err("combat engagement refresh combat sides identical");
    }
    let combatant = combatant as u64;
    if combatant == attacker {
        Ok(0)
    } else if combatant == defender {
        Ok(1)
    } else {
        Err("combat engagement refresh side backlink mismatch")
    }
}

fn identify_side(
    memory: &impl Memory,
    base: usize,
    combatant: usize,
    triggers: u8,
    naval_only: bool,
) -> Result<Entry, &'static str> {
    if combatant == 0 {
        return Err("combat engagement refresh combatant null");
    }
    let vtable = engine_native_memory::read::<u64>(memory, combatant)
        .ok_or("combat engagement refresh combatant unreadable")?;
    let naval_vtable = engine_sites::at(base, cnaval_combatant::VTABLE_RVA)
        .ok_or("combat engagement refresh naval combatant vtable withheld")?
        as u64;
    let is_naval = if vtable == naval_vtable {
        true
    } else if naval_only {
        return Err("combat engagement refresh naval combatant unauthenticated");
    } else {
        let land_vtable = engine_sites::at(base, cland_combatant::VTABLE_RVA)
            .ok_or("combat engagement refresh land combatant vtable withheld")?
            as u64;
        let border_vtable = engine_sites::at(base, cland_border_war_combatant::VTABLE_RVA)
            .ok_or("combat engagement refresh border war combatant vtable withheld")?
            as u64;
        if vtable != land_vtable && vtable != border_vtable {
            return Err("combat engagement refresh combatant unauthenticated");
        }
        false
    };
    let combat_offset = if is_naval {
        cnaval_combatant::COMBAT
    } else {
        cland_combatant::COMBAT
    };
    let combat = combatant
        .checked_add(combat_offset)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("combat engagement refresh parent combat unreadable")?;
    let combat_vtable = engine_native_memory::read::<u64>(memory, combat)
        .ok_or("combat engagement refresh parent combat vtable unreadable")?;
    if is_naval {
        let expected = engine_sites::at(base, cnaval_combat::VTABLE_RVA)
            .ok_or("combat engagement refresh naval combat vtable withheld")?
            as u64;
        if combat_vtable != expected {
            return Err("combat engagement refresh naval combat unauthenticated");
        }
        let handle = combat
            .checked_add(cnaval_combat::HANDLE)
            .and_then(|at| engine_native_memory::read::<u64>(memory, at))
            .filter(|&handle| handle != 0)
            .ok_or("combat engagement refresh naval combat handle unreadable")?;
        let side = read_side_index(
            memory,
            combat,
            cnaval_combat::ATTACKER,
            cnaval_combat::DEFENDER,
            combatant,
        )?;
        let viewer = read_local_viewing_country(memory, base)?;
        Ok(Entry {
            kind: CombatEngagementRefreshRow::NAVAL_SIDE,
            target: handle,
            side,
            viewer,
            triggers,
        })
    } else {
        let land_vtable = engine_sites::at(base, cland_combat::VTABLE_RVA)
            .ok_or("combat engagement refresh land combat vtable withheld")?
            as u64;
        let border_vtable = engine_sites::at(base, cland_border_war_combat::VTABLE_RVA)
            .ok_or("combat engagement refresh border war combat vtable withheld")?
            as u64;
        let expected_body = if combat_vtable == land_vtable {
            engine_sites::at(base, cland_combat::BODY_VTABLE_RVA)
                .ok_or("combat engagement refresh land combat body vtable withheld")?
                as u64
        } else if combat_vtable == border_vtable {
            engine_sites::at(base, cland_border_war_combat::BODY_VTABLE_RVA)
                .ok_or("combat engagement refresh border war combat body vtable withheld")?
                as u64
        } else {
            return Err("combat engagement refresh land combat unauthenticated");
        };
        let body_vtable = combat
            .checked_add(cland_combat::BODY)
            .and_then(|at| engine_native_memory::read::<u64>(memory, at))
            .ok_or("combat engagement refresh land combat body unreadable")?;
        if body_vtable != expected_body {
            return Err("combat engagement refresh land combat body unauthenticated");
        }
        let handle = combat
            .checked_add(cland_combat::HANDLE)
            .and_then(|at| engine_native_memory::read::<u64>(memory, at))
            .filter(|&handle| handle != 0)
            .ok_or("combat engagement refresh land combat handle unreadable")?;
        let side = read_side_index(
            memory,
            combat,
            cland_combat::ATTACKER,
            cland_combat::DEFENDER,
            combatant,
        )?;
        let viewer = read_local_viewing_country(memory, base)?;
        Ok(Entry {
            kind: CombatEngagementRefreshRow::LAND_SIDE,
            target: handle,
            side,
            viewer,
            triggers,
        })
    }
}

fn identify_controller_target(
    memory: &impl Memory,
    command: usize,
) -> Result<Option<Entry>, &'static str> {
    if command == 0 {
        return Err("combat engagement refresh controller command null");
    }
    let target = command
        .checked_add(native::CHANGE_CONTROLLER_TARGET)
        .and_then(|at| engine_native_memory::read::<u32>(memory, at))
        .ok_or("combat engagement refresh controller target unreadable")?;
    if (target as i32) <= 0 {
        return Ok(None);
    }
    Ok(Some(Entry {
        kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
        target: u64::from(target),
        side: 0,
        viewer: 0,
        triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
    }))
}

fn read_local_viewing_country(memory: &impl Memory, base: usize) -> Result<u32, &'static str> {
    let game = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("combat engagement refresh game state unreadable")?;
    let [country, observer] = engine_native::cgame_state::local_player(memory, game)
        .map_err(|_| "combat engagement refresh local viewer unreadable")?;
    let raw = if (country as i32) > 0 {
        country
    } else if (observer as i32) > 0 {
        observer
    } else {
        0
    };
    Ok(raw)
}

fn identify_all_combats(
    memory: &impl Memory,
    base: usize,
    manager: usize,
) -> Result<Option<Entry>, &'static str> {
    if manager == 0 {
        return Err("combat engagement refresh combat manager null");
    }
    let expected = engine_sites::at(base, native::COMBAT_MANAGER_VTABLE_RVA)
        .ok_or("combat engagement refresh combat manager vtable withheld")?
        as u64;
    let vtable = engine_native_memory::read::<u64>(memory, manager)
        .ok_or("combat engagement refresh combat manager unreadable")?;
    if vtable != expected {
        return Err("combat engagement refresh combat manager unauthenticated");
    }
    let target = u64::from(read_local_viewing_country(memory, base)?);
    Ok(Some(Entry {
        kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
        target,
        side: 0,
        viewer: 0,
        triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
    }))
}

fn read_manager_cached_viewer(
    memory: &impl Memory,
    base: usize,
    manager: usize,
) -> Result<u32, &'static str> {
    if manager == 0 {
        return Err("combat engagement refresh combat manager null");
    }
    let expected = engine_sites::at(base, native::COMBAT_MANAGER_VTABLE_RVA)
        .ok_or("combat engagement refresh combat manager vtable withheld")?
        as u64;
    let vtable = engine_native_memory::read::<u64>(memory, manager)
        .ok_or("combat engagement refresh combat manager unreadable")?;
    if vtable != expected {
        return Err("combat engagement refresh combat manager unauthenticated");
    }
    manager
        .checked_add(native::COMBAT_MANAGER_CACHED_VIEWER)
        .and_then(|at| engine_native_memory::read::<u32>(memory, at))
        .ok_or("combat engagement refresh combat manager viewer unreadable")
}

fn identify_hourly_viewer_change(before: u32, after: u32) -> Option<Entry> {
    (before != after).then_some(Entry {
        kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
        target: if (after as i32) > 0 {
            u64::from(after)
        } else {
            0
        },
        side: 0,
        viewer: 0,
        triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED
            | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE,
    })
}

thread_local! {
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

struct FlightGuard {
    entered_ledger: bool,
    observe: bool,
    completed: bool,
}

impl FlightGuard {
    fn enter(observe: bool) -> Self {
        FLIGHTS.fetch_add(1, Ordering::SeqCst);
        let outer = DEPTH.with(|depth| {
            let current = depth.get();
            depth.set(current + 1);
            current == 0
        });
        if outer {
            LEDGER.enter();
        }
        Self {
            entered_ledger: outer,
            observe,
            completed: false,
        }
    }
}

impl Drop for FlightGuard {
    fn drop(&mut self) {
        DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        if !self.completed {
            if self.observe && ENABLED.load(Ordering::SeqCst) {
                LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
                LEDGER.refuse("combat engagement refresh native call unwound");
            }
            if self.entered_ledger {
                LEDGER.leave();
            }
            return;
        }
        if self.entered_ledger {
            LEDGER.leave();
        }
        FLIGHTS.fetch_sub(1, Ordering::SeqCst);
    }
}

fn commit_observation(
    site_key: usize,
    object: usize,
    generation: u64,
    outcome: Result<Option<Entry>, &'static str>,
) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    if LEDGER.poisoned.load(Ordering::SeqCst) || crate::allocation::failed() {
        LEDGER.attempt();
        LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if generation != crate::world_lifetime::generation() || crate::world_lifetime::invalidating() {
        LEDGER.attempt();
        LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
        LEDGER.refuse("combat engagement refresh native world awaits hourly boundary");
        return;
    }
    match outcome {
        Ok(Some(entry)) => {
            LEDGER.attempt();
            let result = LEDGER
                .claim((site_key, object))
                .and_then(|token| LEDGER.complete(token, Ok(Some(entry))));
            if let Err(reason) = result {
                LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
                LEDGER.refuse(reason);
            }
        }
        Ok(None) => {
            LEDGER.zero_writes.fetch_add(1, Ordering::Relaxed);
        }
        Err(reason) => {
            LEDGER.attempt();
            LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
            LEDGER.refuse(reason);
        }
    }
}

type NativeFunction = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;

fn execute_observed<T>(
    site: usize,
    object: usize,
    prepare: impl FnOnce() -> Result<T, &'static str>,
    original: impl FnOnce() -> usize,
    finish: impl FnOnce(T) -> Result<Option<Entry>, &'static str>,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CombatEngagementRefreshes);
    let observe = ENABLED.load(Ordering::SeqCst);
    let mut flight = FlightGuard::enter(observe);
    let generation = crate::world_lifetime::generation();
    let pre = if observe {
        if crate::world_lifetime::invalidating() {
            Some(Err("combat engagement refresh native world awaits hourly boundary"))
        } else {
            Some(prepare())
        }
    } else {
        None
    };
    let result = timing.original(original);
    flight.completed = true;
    if let Some(pre) = pre {
        let outcome = pre.and_then(finish);
        commit_observation(site, object, generation, outcome);
    }
    result
}

fn run_observed(
    site: usize,
    rcx: usize,
    rdx: usize,
    r8: usize,
    r9: usize,
    read: impl FnOnce(&engine_native_memory::memory::LiveMemory, usize) -> Result<Option<Entry>, &'static str>,
) -> usize {
    execute_observed(
        site,
        rcx,
        || {
            // SAFETY: live read occurs inside the native game callback on the paused object pointer.
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            let base = BASE.load(Ordering::Relaxed);
            read(&memory, base)
        },
        || {
            // SAFETY: trampoline address was stored during hook construction before publication.
            let original: NativeFunction =
                unsafe { std::mem::transmute(ORIGINAL[site].load(Ordering::Acquire)) };
            // SAFETY: forwarding the original native arguments to the hooked function trampoline.
            unsafe { original(rcx, rdx, r8, r9) }
        },
        Ok,
    )
}

unsafe extern "system-unwind" fn add_unit(
    combatant: usize,
    unit: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    run_observed(0, combatant, unit, r8, r9, |memory, base| {
        identify_side(
            memory,
            base,
            combatant,
            CombatEngagementRefreshRow::TRIGGER_RECORDED,
            false,
        )
        .map(Some)
    })
}

unsafe extern "system-unwind" fn remove_unit(
    combatant: usize,
    unit: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    run_observed(1, combatant, unit, r8, r9, |memory, base| {
        identify_side(
            memory,
            base,
            combatant,
            CombatEngagementRefreshRow::TRIGGER_UNITS,
            false,
        )
        .map(Some)
    })
}

unsafe extern "system-unwind" fn naval_add_transfer(
    combatant: usize,
    transfer: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    run_observed(2, combatant, transfer, r8, r9, |memory, base| {
        identify_side(
            memory,
            base,
            combatant,
            CombatEngagementRefreshRow::TRIGGER_RECORDED,
            true,
        )
        .map(Some)
    })
}

unsafe extern "system-unwind" fn change_controller(
    command: usize,
    rdx: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    run_observed(3, command, rdx, r8, r9, |memory, _base| {
        identify_controller_target(memory, command)
    })
}

unsafe extern "system-unwind" fn update_all_combats(
    manager: usize,
    rdx: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    run_observed(4, manager, rdx, r8, r9, |memory, base| {
        identify_all_combats(memory, base, manager)
    })
}

unsafe extern "system-unwind" fn hourly_update(
    manager: usize,
    rdx: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    execute_observed(
        5,
        manager,
        || {
            // SAFETY: live read occurs inside the native game callback on the paused manager pointer.
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            let base = BASE.load(Ordering::Relaxed);
            read_manager_cached_viewer(&memory, base, manager)
        },
        || {
            // SAFETY: trampoline address was stored during hook construction before publication.
            let original: NativeFunction =
                unsafe { std::mem::transmute(ORIGINAL[5].load(Ordering::Acquire)) };
            // SAFETY: forwarding the original native arguments to the hooked function trampoline.
            unsafe { original(manager, rdx, r8, r9) }
        },
        |before| {
            // SAFETY: live read occurs immediately after the native hourly update returns on the live manager.
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            let base = BASE.load(Ordering::Relaxed);
            let after = read_manager_cached_viewer(&memory, base, manager)?;
            Ok(identify_hourly_viewer_change(before, after))
        },
    )
}

const REQUIRED_VTABLES: [engine_sites::Rva; 9] = [
    cland_combatant::VTABLE_RVA,
    cland_border_war_combatant::VTABLE_RVA,
    cnaval_combatant::VTABLE_RVA,
    cland_combat::VTABLE_RVA,
    cland_combat::BODY_VTABLE_RVA,
    cland_border_war_combat::VTABLE_RVA,
    cland_border_war_combat::BODY_VTABLE_RVA,
    cnaval_combat::VTABLE_RVA,
    native::COMBAT_MANAGER_VTABLE_RVA,
];

fn begin_enable_transition(enabled: bool) -> bool {
    let was_enabled = ENABLED.load(Ordering::SeqCst);
    if !enabled {
        ENABLED.store(false, Ordering::SeqCst);
        if was_enabled {
            // SAFETY: Hook disable transition owns invalidating in-flight refresh observation.
            unsafe { LEDGER.lost_drain() };
        }
    }
    was_enabled
}

fn finish_enable_transition(was_enabled: bool, enabled: bool, has_hooks: bool) {
    let now_enabled = enabled && has_hooks;
    if now_enabled && !was_enabled {
        // SAFETY: Hook enable transition owns invalidating any pre-enable partial interval.
        unsafe { LEDGER.lost_drain() };
    }
    ENABLED.store(now_enabled, Ordering::SeqCst);
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_reset(enabled: bool) {
    LEDGER.reset();
    ENABLED.store(enabled, Ordering::SeqCst);
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_record_entry(entry: Entry) {
    let mut flight = FlightGuard::enter(true);
    flight.completed = true;
    commit_observation(0, 1, crate::world_lifetime::generation(), Ok(Some(entry)));
}

pub struct Hooks {
    hooks: Vec<Hook>,
}

impl Hooks {
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Self {
        BASE.store(base, Ordering::Relaxed);
        let callbacks = [
            add_unit as *const () as usize,
            remove_unit as *const () as usize,
            naval_add_transfer as *const () as usize,
            change_controller as *const () as usize,
            update_all_combats as *const () as usize,
            hourly_update as *const () as usize,
        ];
        let install = || -> Result<Vec<Hook>, &'static str> {
            engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
                .ok_or("combat engagement refresh game state singleton withheld on this build")?;
            for rva in REQUIRED_VTABLES {
                engine_sites::at(base, rva)
                    .ok_or("combat engagement refresh vtable withheld on this build")?;
            }
            for site in &native::MUTATIONS {
                crate::located(base, site.rva)?;
            }
            let mut hooks = Vec::with_capacity(native::MUTATIONS.len());
            for (index, site) in native::MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                // SAFETY: code address and sentinel were verified against the live executable image.
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|_| "combat engagement refresh hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
            Ok(hooks)
        };
        match install() {
            Ok(hooks) => {
                INSTALLED.store(true, Ordering::Release);
                Self { hooks }
            }
            Err(reason) => {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Construction,
                    None,
                    reason,
                );
                let _ = UNHOOKED.set(reason);
                Self { hooks: Vec::new() }
            }
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        let was_enabled = begin_enable_transition(enabled);
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "combat engagement refresh hook state change failed")?
        {
            return Ok(false);
        }
        finish_enable_transition(was_enabled, enabled, !self.hooks.is_empty());
        Ok(true)
    }

    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    use std::sync::{Mutex, MutexGuard};

    #[cfg(feature = "tap-foreign-probe")]
    mod foreign_unwind;

    static OWNER: Mutex<()> = Mutex::new(());
    static OWNING_THREAD: AtomicUsize = AtomicUsize::new(0);

    pub(super) fn owns_observation() -> bool {
        let current = current_thread_tag();
        OWNING_THREAD.load(Ordering::Acquire) == current
    }

    fn current_thread_tag() -> usize {
        std::thread_local! {
            static TAG: u8 = const { 0 };
        }
        TAG.with(|tag| std::ptr::from_ref::<u8>(tag) as usize)
    }

    struct TestScope<'a> {
        _guard: MutexGuard<'a, ()>,
    }

    impl TestScope<'_> {
        fn enter() -> Self {
            let guard = OWNER
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            OWNING_THREAD.store(current_thread_tag(), Ordering::Release);
            FLIGHTS.store(0, Ordering::SeqCst);
            LEDGER.reset();
            ENABLED.store(true, Ordering::SeqCst);
            Self { _guard: guard }
        }
    }

    impl Drop for TestScope<'_> {
        fn drop(&mut self) {
            ENABLED.store(false, Ordering::SeqCst);
            FLIGHTS.store(0, Ordering::SeqCst);
            LEDGER.reset();
            OWNING_THREAD.store(0, Ordering::Release);
        }
    }

    fn write_u64(buf: &mut [u8], at: usize, value: u64) {
        buf[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(buf: &mut [u8], at: usize, value: u32) {
        buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn clock(hour: i32) -> Clock {
        Clock {
            hour,
            campaign_id: 1,
            play_session_started_at: 10,
            ..Default::default()
        }
    }

    #[test]
    fn identifies_land_border_naval_controller_and_manager_objects() {
        let base = 0x1000_0000usize;
        let mem_base = 0x2000_0000usize;
        let mut bytes = vec![0u8; 0x1000];
        let addr = |offset: usize| mem_base + offset;

        let land_combat = 0x100;
        let land_att = 0x200;
        let land_def = 0x280;
        write_u64(
            &mut bytes,
            land_combat,
            engine_sites::at(base, cland_combat::VTABLE_RVA).unwrap() as u64,
        );
        write_u64(
            &mut bytes,
            land_combat + cland_combat::BODY,
            engine_sites::at(base, cland_combat::BODY_VTABLE_RVA).unwrap() as u64,
        );
        write_u64(&mut bytes, land_combat + cland_combat::HANDLE, 77);
        write_u64(
            &mut bytes,
            land_combat + cland_combat::ATTACKER,
            addr(land_att) as u64,
        );
        write_u64(
            &mut bytes,
            land_combat + cland_combat::DEFENDER,
            addr(land_def) as u64,
        );
        for side in [land_att, land_def] {
            write_u64(
                &mut bytes,
                side,
                engine_sites::at(base, cland_combatant::VTABLE_RVA).unwrap() as u64,
            );
            write_u64(
                &mut bytes,
                side + cland_combatant::COMBAT,
                addr(land_combat) as u64,
            );
        }

        let border_combat = 0x300;
        let border_def = 0x380;
        write_u64(
            &mut bytes,
            border_combat,
            engine_sites::at(base, cland_border_war_combat::VTABLE_RVA).unwrap() as u64,
        );
        write_u64(
            &mut bytes,
            border_combat + cland_border_war_combat::BODY,
            engine_sites::at(base, cland_border_war_combat::BODY_VTABLE_RVA).unwrap() as u64,
        );
        write_u64(&mut bytes, border_combat + cland_combat::HANDLE, 88);
        write_u64(
            &mut bytes,
            border_combat + cland_combat::ATTACKER,
            addr(0x340) as u64,
        );
        write_u64(
            &mut bytes,
            border_combat + cland_combat::DEFENDER,
            addr(border_def) as u64,
        );
        write_u64(
            &mut bytes,
            border_def,
            engine_sites::at(base, cland_border_war_combatant::VTABLE_RVA).unwrap() as u64,
        );
        write_u64(
            &mut bytes,
            border_def + cland_combatant::COMBAT,
            addr(border_combat) as u64,
        );

        let naval_combat = 0x400;
        let naval_att = 0x480;
        write_u64(
            &mut bytes,
            naval_combat,
            engine_sites::at(base, cnaval_combat::VTABLE_RVA).unwrap() as u64,
        );
        write_u64(&mut bytes, naval_combat + cnaval_combat::HANDLE, 99);
        write_u64(
            &mut bytes,
            naval_combat + cnaval_combat::ATTACKER,
            addr(naval_att) as u64,
        );
        write_u64(
            &mut bytes,
            naval_combat + cnaval_combat::DEFENDER,
            addr(0x4c0) as u64,
        );
        write_u64(
            &mut bytes,
            naval_att,
            engine_sites::at(base, cnaval_combatant::VTABLE_RVA).unwrap() as u64,
        );
        write_u64(
            &mut bytes,
            naval_att + cnaval_combatant::COMBAT,
            addr(naval_combat) as u64,
        );

        let cmd_pos = 0x500;
        write_u32(&mut bytes, cmd_pos + native::CHANGE_CONTROLLER_TARGET, 42);

        let cmd_zero = 0x580;
        write_u32(&mut bytes, cmd_zero + native::CHANGE_CONTROLLER_TARGET, 0);

        let manager = 0x600;
        write_u64(
            &mut bytes,
            manager,
            engine_sites::at(base, native::COMBAT_MANAGER_VTABLE_RVA).unwrap() as u64,
        );
        write_u32(&mut bytes, manager + native::COMBAT_MANAGER_CACHED_VIEWER, 11);

        let game_state = 0x700;
        write_u32(
            &mut bytes,
            game_state + engine_native::cgame_state::LOCAL_PLAYER_COUNTRY,
            0,
        );
        write_u32(
            &mut bytes,
            game_state + engine_native::cgame_state::OBSERVED_COUNTRY,
            17,
        );

        struct CompositeMemory<'a> {
            slice: SliceMemory<'a>,
            singleton_addr: usize,
            game_state_ptr: u64,
        }
        impl Memory for CompositeMemory<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == self.singleton_addr && output.len() == 8 {
                    output.copy_from_slice(&self.game_state_ptr.to_le_bytes());
                    return true;
                }
                self.slice.copy(address, output)
            }
        }

        let memory = CompositeMemory {
            slice: SliceMemory {
                base: mem_base,
                bytes: &bytes,
            },
            singleton_addr: engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
                .unwrap(),
            game_state_ptr: addr(game_state) as u64,
        };

        assert_eq!(
            identify_side(
                &memory,
                base,
                addr(land_att),
                CombatEngagementRefreshRow::TRIGGER_RECORDED,
                false
            ),
            Ok(Entry {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 77,
                side: 0,
                viewer: 17,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            })
        );
        assert_eq!(
            identify_side(
                &memory,
                base,
                addr(border_def),
                CombatEngagementRefreshRow::TRIGGER_UNITS,
                false
            ),
            Ok(Entry {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 88,
                side: 1,
                viewer: 17,
                triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
            })
        );
        assert!(
            identify_side(
                &memory,
                base,
                addr(land_att),
                CombatEngagementRefreshRow::TRIGGER_RECORDED,
                true
            )
            .is_err()
        );
        assert_eq!(
            identify_side(
                &memory,
                base,
                addr(naval_att),
                CombatEngagementRefreshRow::TRIGGER_RECORDED,
                true
            ),
            Ok(Entry {
                kind: CombatEngagementRefreshRow::NAVAL_SIDE,
                target: 99,
                side: 0,
                viewer: 17,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            })
        );
        assert!(
            identify_side(
                &memory.slice,
                base,
                addr(land_att),
                CombatEngagementRefreshRow::TRIGGER_RECORDED,
                false
            )
            .is_err()
        );
        assert_eq!(
            identify_controller_target(&memory, addr(cmd_pos)),
            Ok(Some(Entry {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 42,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            }))
        );
        assert_eq!(identify_controller_target(&memory, addr(cmd_zero)), Ok(None));
        assert_eq!(
            identify_all_combats(&memory, base, addr(manager)),
            Ok(Some(Entry {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 17,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            }))
        );
        assert_eq!(
            read_manager_cached_viewer(&memory, base, addr(manager)),
            Ok(11)
        );
        assert_eq!(identify_hourly_viewer_change(11, 11), None);
        assert_eq!(
            identify_hourly_viewer_change(11, 12),
            Some(Entry {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 12,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED
                    | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE,
            })
        );
        assert_eq!(
            identify_hourly_viewer_change(11, 0),
            Some(Entry {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED
                    | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE,
            })
        );

        assert!(identify_side(&memory, base, 0, 1, false).is_err());
        assert!(identify_controller_target(&memory, 0).is_err());
        assert!(identify_all_combats(&memory, base, 0).is_err());
        assert!(identify_all_combats(&memory, base, addr(cmd_pos)).is_err());
        assert!(read_manager_cached_viewer(&memory, base, 0).is_err());
        assert!(read_manager_cached_viewer(&memory, base, addr(cmd_pos)).is_err());
    }

    #[test]
    fn first_interval_is_partial_and_subsequent_complete_interval_drains_cleanly() {
        let _scope = TestScope::enter();
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { presize(16) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(100))) };
        let mut out = Vec::new();
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert!(unsafe { drain(&mut out) }.is_err());

        let mut committed_during_original = true;
        execute_observed(
            0,
            1,
            || {
                Ok(Some(Entry {
                    kind: CombatEngagementRefreshRow::LAND_SIDE,
                    target: 62,
                    side: 0,
                    viewer: 1,
                    triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
                }))
            },
            || {
                committed_during_original = LEDGER.events.load(Ordering::SeqCst) > 0;
                assert!(active() > 0);
                0
            },
            Ok,
        );
        assert!(!committed_during_original);
        assert_eq!(active(), 0);
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(101))) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        let count = unsafe { drain(&mut out) }.unwrap();
        assert_eq!(count, 1);
        assert_eq!(out[0].target, 62);

        execute_observed(1, 2, || Err::<Option<Entry>, _>("synthetic unreadable"), || 0, Ok);
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(102))) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert!(unsafe { drain(&mut out) }.is_err());

        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(103))) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert!(unsafe { drain(&mut out) }.is_err());

        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(104))) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert_eq!(unsafe { drain(&mut out) }.unwrap(), 0);
    }

    #[test]
    fn unwinding_native_call_retains_flight_and_refuses_interval() {
        let _scope = TestScope::enter();
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { presize(16) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(100))) };
        let mut out = Vec::new();
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert!(unsafe { drain(&mut out) }.is_err());

        let unwound = std::panic::catch_unwind(|| {
            execute_observed(
                0,
                1,
                || {
                    Ok(Some(Entry {
                        kind: CombatEngagementRefreshRow::LAND_SIDE,
                        target: 62,
                        side: 0,
                        viewer: 1,
                        triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
                    }))
                },
                || -> usize {
                    panic!("synthetic native unwind");
                },
                Ok,
            )
        });
        assert!(unwound.is_err());
        assert_eq!(active(), 1);
        assert_eq!(LEDGER.events.load(Ordering::SeqCst), 0);

        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(101))) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert!(unsafe { drain(&mut out) }.is_err());

        execute_observed(0, 2, || Ok(None), || 0, Ok);
        assert_eq!(active(), 1);
    }

    #[test]
    fn nested_same_thread_calls_commit_without_overlap_poisoning() {
        let _scope = TestScope::enter();
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { presize(16) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(100))) };
        let mut out = Vec::new();
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert!(unsafe { drain(&mut out) }.is_err());

        execute_observed(
            5,
            10,
            || Ok(11u32),
            || {
                execute_observed(
                    1,
                    20,
                    || {
                        Ok(Some(Entry {
                            kind: CombatEngagementRefreshRow::LAND_SIDE,
                            target: 62,
                            side: 0,
                            viewer: 1,
                            triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
                        }))
                    },
                    || 0,
                    Ok,
                )
            },
            |before| Ok(identify_hourly_viewer_change(before, 12)),
        );
        assert_eq!(active(), 0);
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(101))) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        let count = unsafe { drain(&mut out) }.unwrap();
        assert_eq!(count, 2);
        assert_eq!(out[0].kind, CombatEngagementRefreshRow::LAND_SIDE);
        assert_eq!(out[1].kind, CombatEngagementRefreshRow::LOCAL_ALL_SIDES);
        assert_eq!(
            out[1].triggers,
            CombatEngagementRefreshRow::TRIGGER_RECORDED
                | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE
        );
    }

    #[test]
    fn disable_then_reenable_poisons_quiet_next_interval() {
        let _scope = TestScope::enter();
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { presize(16) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(100))) };
        let mut out = Vec::new();
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert!(unsafe { drain(&mut out) }.is_err());
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(101))) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert_eq!(unsafe { drain(&mut out) }.unwrap(), 0);

        let was = begin_enable_transition(true);
        finish_enable_transition(was, true, true);
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(102))) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert_eq!(unsafe { drain(&mut out) }.unwrap(), 0);

        let was = begin_enable_transition(false);
        finish_enable_transition(was, false, true);
        let was = begin_enable_transition(true);
        finish_enable_transition(was, true, true);

        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(103))) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert!(unsafe { drain(&mut out) }.is_err());

        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        unsafe { boundary(Some(clock(104))) };
        // SAFETY: Isolated test owns combat-engagement-refresh observation via TestScope.
        assert_eq!(unsafe { drain(&mut out) }.unwrap(), 0);
    }
}
