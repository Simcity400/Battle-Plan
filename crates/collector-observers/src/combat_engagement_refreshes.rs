use crate::{
    event_ledger::{Event, EventLedger},
    hook::Hook,
};
use engine_mutations::combat_engagement_mutations as native;
use engine_native::{
    Memory, cland_border_war_combat, cland_border_war_combatant, cland_combat, cland_combatant,
    cnaval_combat, cnaval_combatant,
};
use replay_data::rows::CombatEngagementRefreshRow;
use std::{
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

pub fn presize(demand: usize) {
    if !foreign_test() {
        LEDGER.prepare(demand);
    }
}

pub fn lost_drain() {
    if !foreign_test() {
        LEDGER.lost_drain();
    }
}

pub fn lost_output(entries: usize) {
    if !foreign_test() {
        LEDGER.lost_output(entries);
    }
}

pub fn boundary(clock: Option<Clock>) {
    if !foreign_test() {
        LEDGER.boundary(clock);
    }
}

pub fn drain(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    if foreign_test() || UNHOOKED.get().is_some() || !ENABLED.load(Ordering::SeqCst) {
        return Err("combat engagement refresh hooks unavailable (see status unhooked)");
    }
    LEDGER.drain(output)
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
        .and_then(|at| engine_native::read::<u64>(memory, at))
        .ok_or("combat engagement refresh combat attacker unreadable")?;
    let defender = combat
        .checked_add(defender_offset)
        .and_then(|at| engine_native::read::<u64>(memory, at))
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
    let vtable = engine_native::read::<u64>(memory, combatant)
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
        .and_then(|at| engine_native::read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("combat engagement refresh parent combat unreadable")?;
    let combat_vtable = engine_native::read::<u64>(memory, combat)
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
            .and_then(|at| engine_native::read::<u64>(memory, at))
            .filter(|&handle| handle != 0)
            .ok_or("combat engagement refresh naval combat handle unreadable")?;
        let side = read_side_index(
            memory,
            combat,
            cnaval_combat::ATTACKER,
            cnaval_combat::DEFENDER,
            combatant,
        )?;
        Ok(Entry {
            kind: CombatEngagementRefreshRow::NAVAL_SIDE,
            target: handle,
            side,
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
            .and_then(|at| engine_native::read::<u64>(memory, at))
            .ok_or("combat engagement refresh land combat body unreadable")?;
        if body_vtable != expected_body {
            return Err("combat engagement refresh land combat body unauthenticated");
        }
        let handle = combat
            .checked_add(cland_combat::HANDLE)
            .and_then(|at| engine_native::read::<u64>(memory, at))
            .filter(|&handle| handle != 0)
            .ok_or("combat engagement refresh land combat handle unreadable")?;
        let side = read_side_index(
            memory,
            combat,
            cland_combat::ATTACKER,
            cland_combat::DEFENDER,
            combatant,
        )?;
        Ok(Entry {
            kind: CombatEngagementRefreshRow::LAND_SIDE,
            target: handle,
            side,
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
        .and_then(|at| engine_native::read::<u32>(memory, at))
        .ok_or("combat engagement refresh controller target unreadable")?;
    if (target as i32) <= 0 {
        return Ok(None);
    }
    Ok(Some(Entry {
        kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
        target: u64::from(target),
        side: 0,
        triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
    }))
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
    let vtable = engine_native::read::<u64>(memory, manager)
        .ok_or("combat engagement refresh combat manager unreadable")?;
    if vtable != expected {
        return Err("combat engagement refresh combat manager unauthenticated");
    }
    Ok(Some(Entry {
        kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
        target: 0,
        side: 0,
        triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
    }))
}

struct ActiveGuard;
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        LEDGER.leave();
    }
}

fn record_observation(
    site_key: usize,
    object: usize,
    read: impl FnOnce(&engine_native::memory::LiveMemory, usize) -> Result<Option<Entry>, &'static str>,
) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    LEDGER.enter();
    let _guard = ActiveGuard;
    if LEDGER.poisoned.load(Ordering::SeqCst) || crate::allocation::failed() {
        LEDGER.attempt();
        LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
        return;
    }
    LEDGER.attempt();
    if crate::world_lifetime::invalidating() {
        LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
        LEDGER.refuse("combat engagement refresh native world awaits hourly boundary");
        return;
    }
    // SAFETY: live read occurs inside the native game callback on the paused object pointer.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    match read(&memory, base) {
        Ok(Some(entry)) => {
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
            LEDGER.lost_events_unknown.fetch_add(1, Ordering::Relaxed);
            LEDGER.refuse(reason);
        }
    }
}

type NativeFunction = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;

fn run(site: usize, rcx: usize, rdx: usize, r8: usize, r9: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CombatEngagementRefreshes);
    FLIGHTS.fetch_add(1, Ordering::SeqCst);
    // SAFETY: trampoline address was stored during hook construction before publication.
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[site].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(rcx, rdx, r8, r9) });
    FLIGHTS.fetch_sub(1, Ordering::SeqCst);
    result
}

unsafe extern "system-unwind" fn add_unit(
    combatant: usize,
    unit: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    record_observation(0, combatant, |memory, base| {
        identify_side(
            memory,
            base,
            combatant,
            CombatEngagementRefreshRow::TRIGGER_RECORDED,
            false,
        )
        .map(Some)
    });
    run(0, combatant, unit, r8, r9)
}

unsafe extern "system-unwind" fn remove_unit(
    combatant: usize,
    unit: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    record_observation(1, combatant, |memory, base| {
        identify_side(
            memory,
            base,
            combatant,
            CombatEngagementRefreshRow::TRIGGER_UNITS,
            false,
        )
        .map(Some)
    });
    run(1, combatant, unit, r8, r9)
}

unsafe extern "system-unwind" fn naval_add_transfer(
    combatant: usize,
    transfer: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    record_observation(2, combatant, |memory, base| {
        identify_side(
            memory,
            base,
            combatant,
            CombatEngagementRefreshRow::TRIGGER_RECORDED,
            true,
        )
        .map(Some)
    });
    run(2, combatant, transfer, r8, r9)
}

unsafe extern "system-unwind" fn change_controller(
    command: usize,
    rdx: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    record_observation(3, command, |memory, _base| {
        identify_controller_target(memory, command)
    });
    run(3, command, rdx, r8, r9)
}

unsafe extern "system-unwind" fn update_all_combats(
    manager: usize,
    rdx: usize,
    r8: usize,
    r9: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    record_observation(4, manager, |memory, base| {
        identify_all_combats(memory, base, manager)
    });
    run(4, manager, rdx, r8, r9)
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
        ];
        let install = || -> Result<Vec<Hook>, &'static str> {
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
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "combat engagement refresh hook state change failed")?
        {
            return Ok(false);
        }
        ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        Ok(true)
    }

    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::memory::SliceMemory;
    use std::sync::{Mutex, MutexGuard};

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
        TAG.with(|tag| tag as *const u8 as usize)
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
            LEDGER.reset();
            ENABLED.store(true, Ordering::SeqCst);
            Self { _guard: guard }
        }
    }

    impl Drop for TestScope<'_> {
        fn drop(&mut self) {
            ENABLED.store(false, Ordering::SeqCst);
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

        let memory = SliceMemory {
            base: mem_base,
            bytes: &bytes,
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
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            })
        );
        assert_eq!(
            identify_controller_target(&memory, addr(cmd_pos)),
            Ok(Some(Entry {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 42,
                side: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            }))
        );
        assert_eq!(identify_controller_target(&memory, addr(cmd_zero)), Ok(None));
        assert_eq!(
            identify_all_combats(&memory, base, addr(manager)),
            Ok(Some(Entry {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 0,
                side: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            }))
        );

        assert!(identify_side(&memory, base, 0, 1, false).is_err());
        assert!(identify_controller_target(&memory, 0).is_err());
        assert!(identify_all_combats(&memory, base, 0).is_err());
        assert!(identify_all_combats(&memory, base, addr(cmd_pos)).is_err());
    }

    #[test]
    fn first_interval_is_partial_and_subsequent_complete_interval_drains_cleanly() {
        let _scope = TestScope::enter();
        presize(16);
        boundary(Some(clock(100)));
        let mut out = Vec::new();
        assert!(drain(&mut out).is_err());

        record_observation(0, 1, |_, _| {
            Ok(Some(Entry {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 62,
                side: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            }))
        });
        boundary(Some(clock(101)));
        let count = drain(&mut out).unwrap();
        assert_eq!(count, 1);
        assert_eq!(out[0].target, 62);

        record_observation(1, 2, |_, _| Err("synthetic unreadable"));
        boundary(Some(clock(102)));
        assert!(drain(&mut out).is_err());

        boundary(Some(clock(103)));
        assert!(drain(&mut out).is_err());

        boundary(Some(clock(104)));
        assert_eq!(drain(&mut out).unwrap(), 0);
    }
}
