use collector_certificates::watch_registry::WatchRegistry;
use collector_hooks::hook::{Hook, Registers, Tap};
use collector_timing::hook_timing::CallCounters;
use engine_mutations::{
    faction_companion_mutations::Family, faction_companion_mutations::MUTATIONS,
    faction_companion_mutations::Patch,
};
use engine_native_memory::Memory;
use std::{
    mem::ManuallyDrop,
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};
pub const FAMILIES: usize = 9;
static WATCHES: [WatchRegistry; FAMILIES] = [
    WatchRegistry::retaining(),
    WatchRegistry::retaining(),
    WatchRegistry::retaining(),
    WatchRegistry::retaining(),
    WatchRegistry::marking(),
    WatchRegistry::retaining(),
    WatchRegistry::retaining(),
    WatchRegistry::retaining(),
    WatchRegistry::retaining(),
];
static ORIGINAL: [AtomicUsize; MUTATIONS.len()] = [const { AtomicUsize::new(0) }; MUTATIONS.len()];
static CALLS: CallCounters<{ MUTATIONS.len() }> = CallCounters::new();
static BASE: AtomicUsize = AtomicUsize::new(0);
static ACCOUNT_DEMAND: AtomicUsize = AtomicUsize::new(0);
static DEMAND: AtomicUsize = AtomicUsize::new(0);
pub trait Changes {
    fn take_accounts(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    fn quiet(&self, family: usize) -> Option<(u64, u64, u64)>;
    fn sequence(&self, family: usize, receiver: usize) -> Option<(u64, u64, u64)>;
}
pub struct Observer;
impl Changes for Observer {
    fn take_accounts(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES[Family::Accounts as usize].take(visit)
    }
    fn quiet(&self, family: usize) -> Option<(u64, u64, u64)> {
        if family == 6 {
            collector_observers::country_input_changes::sharing_quiet().map(|(e, s)| (e, s, 0))
        } else if family == 5 {
            collector_intel::variable_changes::Observer.embedded_quiet()
        } else {
            WATCHES[if family == 11 {
                8
            } else if family >= 7 {
                family - 2
            } else {
                family
            }]
            .quiet()
            .map(|(epoch, sequence)| (epoch, sequence, 0))
        }
    }
    fn sequence(&self, family: usize, receiver: usize) -> Option<(u64, u64, u64)> {
        if family == 6 {
            collector_observers::country_input_changes::sharing_sequence(receiver)
                .map(|(e, s)| (e, s, 0))
        } else if family == 5 {
            collector_intel::variable_changes::Observer.embedded_sequence(receiver)
        } else {
            WATCHES[if family == 11 {
                8
            } else if family >= 7 {
                family - 2
            } else {
                family
            }]
            .sequence(receiver)
            .map(|(epoch, sequence)| (epoch, sequence, 0))
        }
    }
}
/// # Safety
/// Run only during active session sizing or admitted faction capture (or an isolated test) with
/// measured owner demand.
pub unsafe fn demand(owners: usize) {
    DEMAND.fetch_max(owners, Ordering::Relaxed);
}
/// # Safety
/// Run only during active session sizing or admitted faction capture (or an isolated test) with
/// measured account demand.
pub unsafe fn demand_accounts(count: usize) {
    ACCOUNT_DEMAND.fetch_max(count, Ordering::Relaxed);
}
pub fn active() -> usize {
    crate::faction_pools::active()
        + crate::faction_accounts::mirror::active()
        + WATCHES.iter().map(WatchRegistry::active).sum::<usize>()
}
/// # Safety
/// Only the attaching prime pass or the admitted hour's sole observer boundary (or an isolated
/// test) may advance the faction-companion watch boundary.
pub unsafe fn boundary(invalidate: bool) {
    for watches in &WATCHES {
        watches.boundary(invalidate);
    }
}
/// # Safety
/// Run only during attach or the active session's serial sizing pass (or an isolated test), while
/// the faction-companion watches remain live and teardown is excluded.
pub unsafe fn presize() {
    for (family, watches) in WATCHES.iter().enumerate() {
        let count = if family == Family::Accounts as usize {
            ACCOUNT_DEMAND.load(Ordering::Relaxed)
        } else {
            DEMAND.load(Ordering::Relaxed)
        };
        watches.offer(count.max(watches.registered()));
    }
}
pub fn collect() {
    for watches in &WATCHES {
        watches.collect();
    }
}
/// # Safety
/// Only the attaching runtime or an isolated collector fixture may prime the live faction-companion
/// watches, excluding concurrent observer boundaries, capture and detach.
pub unsafe fn prime() {
    // SAFETY: Caller guarantees exclusive attach/fixture priming with live watches.
    unsafe {
        presize();
        boundary(false);
    }
}
/// # Safety
/// Only the Factions worker under its exclusive native quiescent lease.
pub unsafe fn install_growth() {
    for watches in &WATCHES {
        unsafe {
            watches.install_growth_quiescent();
        }
    }
}
enum OwnedHook {
    Entry(Hook),
    Store(Tap),
}
impl OwnedHook {
    fn hook(&mut self) -> &mut Hook {
        match self {
            Self::Entry(hook) => hook,
            Self::Store(tap) => tap.publication_hook(),
        }
    }
    fn ranges(&self) -> Vec<Range<usize>> {
        match self {
            Self::Entry(hook) => hook.executable_ranges().into_iter().collect(),
            Self::Store(tap) => tap.executable_ranges(),
        }
    }
}
fn site_error(site: &str, operation: &str, cause: impl std::fmt::Display) -> String {
    format!("FactionCompanions/{site} {operation}: {cause}")
}
pub struct Hooks {
    hooks: Vec<(Family, OwnedHook)>,
    available: [bool; FAMILIES],
}
impl Hooks {
    /// # Safety
    /// Attach owns the authenticated image, patch publication and hook lifetime.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        BASE.store(base, Ordering::Release);
        let mut hooks = Vec::new();
        let available = [cfg!(feature = "faction_changes"); FAMILIES];
        let mut located = [(0, [0; 16]); MUTATIONS.len()];
        if cfg!(feature = "faction_changes") {
            for ((site, _, _), slot) in MUTATIONS.iter().zip(&mut located) {
                *slot = engine_sites::hook(base, site.rva)
                    .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
                    .map_err(|cause| site_error(site.name, "sentinel verification", cause))?;
            }
        }
        if available.iter().any(|value| *value) {
            let callbacks = [
                select_rule as *const () as usize,
                remove_rule as *const () as usize,
                reset_rules as *const () as usize,
                add_programme as *const () as usize,
                remove_programme as *const () as usize,
                remove_country as *const () as usize,
                transfer as *const () as usize,
                refund as *const () as usize,
                contribute as *const () as usize,
                clear as *const () as usize,
                0,
                0,
                account_entry_12 as *const () as usize,
                account_entry_13 as *const () as usize,
                account_entry_14 as *const () as usize,
                account_entry_15 as *const () as usize,
                account_entry_16 as *const () as usize,
                account_entry_17 as *const () as usize,
                account_entry_18 as *const () as usize,
                account_entry_19 as *const () as usize,
                account_entry_20 as *const () as usize,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                minimum_upgrade as *const () as usize,
                doctrine_folder as *const () as usize,
                0,
                0,
                0,
                0,
                0,
                account_projection as *const () as usize,
                0,
                0,
                0,
                0,
                extraction_complete as *const () as usize,
                influence_phase as *const () as usize,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                advisor_60 as *const () as usize,
                advisor_61 as *const () as usize,
                advisor_62 as *const () as usize,
                advisor_63 as *const () as usize,
                advisor_64 as *const () as usize,
                advisor_65 as *const () as usize,
                rank_phase as *const () as usize,
                0,
                0,
                projection_factory as *const () as usize,
                0,
                projection_heavy_ship as *const () as usize,
                projection_other_ship as *const () as usize,
                projection_combat_air as *const () as usize,
                projection_other_air as *const () as usize,
                0,
                projection_phase as *const () as usize,
                passive_phase as *const () as usize,
                influence_share_phase as *const () as usize,
            ];
            engine_allocation::reserve(&mut hooks, MUTATIONS.len())
                .map_err(|cause| site_error("all sites", "hook allocation", cause))?;
            for (index, ((site, family, patch), &(at, original))) in
                MUTATIONS.iter().zip(&located).enumerate()
            {
                if !available[*family as usize] {
                    continue;
                }
                let hook = match patch {
                    Patch::Tap => OwnedHook::Store(
                        unsafe {
                            Tap::new(
                                at,
                                if *family as usize == Family::Accounts as usize {
                                    account_store
                                } else if *family as usize == Family::Pools as usize {
                                    pool_store
                                } else {
                                    unlock
                                },
                                index,
                                original,
                            )
                        }
                        .map_err(|cause| site_error(site.name, "construction", cause))?,
                    ),
                    Patch::Call => {
                        let displacement = i32::from_le_bytes(original[1..5].try_into().unwrap());
                        let target = at
                            .checked_add(5)
                            .and_then(|at| at.checked_add_signed(displacement as isize))
                            .ok_or_else(|| {
                                site_error(site.name, "call target", "address overflow")
                            })?;
                        let hook = unsafe { Hook::new_call(at, callbacks[index], original) }
                            .map_err(|cause| site_error(site.name, "construction", cause))?;
                        ORIGINAL[index].store(target, Ordering::Release);
                        OwnedHook::Entry(hook)
                    }
                    _ => {
                        let hook = unsafe { Hook::new(at, callbacks[index], original) }
                            .map_err(|cause| site_error(site.name, "construction", cause))?;
                        ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                        OwnedHook::Entry(hook)
                    }
                };
                hooks.push((*family, hook));
            }
        }
        Ok(Self { hooks, available })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            for watches in &WATCHES {
                watches.set_enabled(false);
            }
        }
        if !Hook::set_enabled_iter_tracking_busy(
            self.hooks.iter_mut().map(|(_, hook)| hook.hook()),
            enabled,
        )
        .map_err(|cause| {
            let sites = MUTATIONS
                .iter()
                .map(|(site, _, _)| site.name)
                .collect::<Vec<_>>()
                .join(", ");
            site_error(&sites, "batch publication", cause)
        })? {
            return Ok(false);
        }
        for (family, watches) in WATCHES.iter().enumerate() {
            watches.set_enabled(enabled && self.available[family]);
        }
        Ok(true)
    }
    pub fn busy_sites(&mut self) -> Vec<&'static str> {
        let available = self.available;
        self.hooks
            .iter_mut()
            .zip(
                MUTATIONS
                    .iter()
                    .filter(|(_, family, _)| available[*family as usize]),
            )
            .filter_map(|((_, hook), (site, _, _))| {
                hook.hook().publication_busy().then_some(site.name)
            })
            .collect()
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(|(_, hook)| hook.ranges())
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        for watches in &WATCHES {
            unsafe {
                watches.release();
            }
        }
    }
}
type Function = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;
type Five = unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) -> usize;
macro_rules! callback {
    ($name:ident,$site:expr) => {
        unsafe extern "system-unwind" fn $name(a: usize, b: usize, c: usize, d: usize) -> usize {
            let timing = collector_timing::hook_timing::enter(
                collector_timing::hook_timing::Family::FactionCompanions,
            );
            CALLS.increment($site);
            let watch = &WATCHES[MUTATIONS[$site].1 as usize];
            if $site == 64 && b as u32 == 4 {
                let original: Function =
                    unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
                return timing.original(|| unsafe { original(a, b, c, d) });
            }
            let selected_slot = if $site == 65 {
                a.checked_sub(engine_stability::cfaction_upgrade_status::SLOTS)
            } else if $site == 64 {
                let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
                engine_native_memory::read::<u64>(
                    &memory,
                    a + engine_stability::cfaction_upgrade_status::SLOTS,
                )
                .and_then(|data| usize::try_from(data).ok())
                .and_then(|data| {
                    (b as u32 as usize)
                        .checked_mul(engine_stability::cfaction_upgrade_status::SLOT_STRIDE)
                        .and_then(|offset| data.checked_add(offset))
                })
            } else {
                Some(a)
            };
            let flight = ManuallyDrop::new(if let Some(selected) = selected_slot {
                watch.notify(if $site == 16 {
                    a.saturating_add(engine_native_parts::cfaction_member_status::UPGRADES)
                } else {
                    selected
                })
            } else {
                watch.notify_every()
            });
            let original: Function =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(a, b, c, d) });
            drop(ManuallyDrop::into_inner(flight));
            result
        }
    };
}
callback!(remove_rule, 1);
callback!(reset_rules, 2);
callback!(add_programme, 3);
callback!(remove_programme, 4);
callback!(remove_country, 5);
callback!(transfer, 6);
callback!(refund, 7);
callback!(contribute, 8);
callback!(clear, 9);
unsafe extern "system-unwind" fn select_rule(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::FactionCompanions,
    );
    CALLS.increment(0);
    let flight = ManuallyDrop::new(WATCHES[Family::Rules as usize].notify(a));
    let original: Five = unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c, d, e) });
    drop(ManuallyDrop::into_inner(flight));
    result
}

unsafe extern "system-unwind" fn unlock(registers: &Registers, site: usize) {
    let _timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::FactionCompanions,
    );
    CALLS.increment(site);
    drop(WATCHES[Family::Unlocks as usize].notify(registers.rax));
}

macro_rules! scalar_callback {
    ($name:ident,$site:expr) => {
        unsafe extern "system-unwind" fn $name(a: usize, b: usize, c: usize, d: usize) -> usize {
            use crate::faction_accounts::mirror::{self, Operation};
            let timing = collector_timing::hook_timing::enter(
                collector_timing::hook_timing::Family::FactionCompanions,
            );
            CALLS.increment($site);
            let flight = ManuallyDrop::new(mirror::Flight::new());
            if $site == 46 {
                mirror::projection_sources(a, b as u32 as usize, c as i64);
            } else if $site == 14 {
                mirror::observe(a, false, mirror::CONTRIBUTION, b as i64, Operation::Add);
            } else {
                mirror::observe(
                    a,
                    true,
                    b as u32 as usize,
                    c as i64,
                    if $site == 12 {
                        Operation::Add
                    } else {
                        Operation::Set
                    },
                );
            }
            let original: Function =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(a, b, c, d) });
            drop(ManuallyDrop::into_inner(flight));
            result
        }
    };
}
scalar_callback!(account_entry_12, 12);
scalar_callback!(account_entry_13, 13);
scalar_callback!(account_entry_14, 14);
callback!(account_entry_15, 15);
callback!(account_entry_16, 16);
callback!(account_entry_18, 18);
callback!(account_entry_19, 19);
callback!(account_entry_20, 20);

unsafe extern "system-unwind" fn account_entry_17(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::FactionCompanions,
    );
    CALLS.increment(17);
    let flight = ManuallyDrop::new(WATCHES[Family::Accounts as usize].notify(a));
    let original: Five = unsafe { std::mem::transmute(ORIGINAL[17].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c, d, e) });
    drop(ManuallyDrop::into_inner(flight));
    result
}
unsafe extern "system-unwind" fn account_store(registers: &Registers, site: usize) {
    let _timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::FactionCompanions,
    );
    CALLS.increment(site);
    use crate::faction_accounts::mirror::{self, Operation};
    if site == 67 {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Acquire);
        let canonical = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
            .and_then(|at| engine_native_memory::read::<u64>(&memory, at))
            .and_then(|state| usize::try_from(state).ok())
            .and_then(|state| engine_native::ccountry::Reader::new(&memory, base, state).ok())
            .and_then(|reader| reader.of_pointer(registers.r13).ok())
            .map(|country| country.canonical_slot);
        mirror::rank_country(canonical);
        return;
    }
    if site == 75 {
        mirror::rank_input(registers.rax as i64, registers.rdi as u32);
        return;
    }
    if site == 50 {
        mirror::rank_commit(registers.rcx, registers.rsi as u32 as i32);
        return;
    }
    if matches!(site, 68 | 70) {
        mirror::projection_operand(site - 68, registers.rbx as i64);
        return;
    }
    let _flight = mirror::Flight::new();
    if matches!(site, 47..=49) {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let denominator = if site == 47 {
            registers
                .r14
                .checked_add(engine_native::cfaction::INFLUENCE_TOTAL)
                .and_then(|at| engine_native_memory::read::<i64>(&memory, at))
        } else {
            let (holder, offset) = if site == 48 {
                (registers.r15, 0x10)
            } else {
                (registers.r14, 0x38)
            };
            holder
                .checked_add(offset)
                .and_then(|at| {
                    engine_native_memory::read::<u64>(&memory, at)
                        .and_then(|value| usize::try_from(value).ok())
                })
                .and_then(|at| engine_native_memory::read::<i64>(&memory, at))
        };
        let receiver = if site == 48 {
            registers.r14
        } else {
            registers.rbp
        };
        if let Some(denominator) = denominator {
            mirror::refresh(
                receiver,
                site != 47,
                if site == 47 {
                    (registers.rax as i64).max(0)
                } else {
                    registers.rax as i64
                },
                denominator,
            );
        } else {
            mirror::refuse(receiver);
        }
        return;
    }
    if site >= 41 {
        let (receiver, influence, field, value) = match site {
            41 => (registers.rbp, false, mirror::GAIN_B, registers.rdx),
            42 => (registers.rsi, false, mirror::GAIN_B, registers.rdx),
            43 => (registers.rbp, false, mirror::PREVIOUS_SCORE, registers.rax),
            44 => (registers.rdi, false, mirror::PREVIOUS_SCORE, registers.rax),
            45 => (registers.rbx, true, 3, registers.rdx),
            47 => (registers.rbp, false, mirror::INFLUENCE_SHARE, registers.rcx),
            48 => (
                registers.r14,
                false,
                mirror::PROJECTION_SHARE,
                registers.rcx,
            ),
            49 => (
                registers.rbp,
                false,
                mirror::PROJECTION_SHARE,
                registers.rcx,
            ),
            50 => (
                registers.rcx,
                false,
                mirror::PROJECTION_RANK,
                registers.rsi as u32 as i32 as i64 as usize,
            ),
            _ => unreachable!(),
        };
        mirror::observe(receiver, influence, field, value as i64, Operation::Set);
        return;
    }
    let index = site - engine_mutations::faction_companion_mutations::ACCOUNT_STORE_FIRST;
    let (receiver, value) = match index {
        0 => (registers.rax, registers.rdx),
        1 => (registers.rax, registers.r8),
        2 => (registers.rdi, registers.r8),
        3 => (registers.rdi, registers.r9),
        4 | 5 => (registers.rax, registers.rcx),
        6 => (registers.rdi, registers.r8),
        7 => (registers.rbx, registers.r8),
        8 => (registers.rax, registers.r8),
        9 | 10 => (registers.rbx, registers.rcx),
        11 => (registers.rax, registers.r8),
        12 => (registers.rbx, registers.r9),
        13 => (registers.rsi, registers.rdx),
        14 => (registers.rbx, registers.r9),
        15 => (registers.rbx, registers.r8),
        16 | 17 => (registers.r8, registers.rcx),
        _ => unreachable!(),
    };
    if matches!(index, 16..=17) {
        mirror::passive(receiver, (value as i64).wrapping_mul(100_000));
    } else if matches!(index, 4 | 5 | 9 | 10) {
        mirror::observe(
            receiver,
            false,
            mirror::GAIN_A,
            value as i64,
            if index == 5 {
                Operation::Subtract
            } else {
                Operation::Add
            },
        );
    } else {
        mirror::observe(
            receiver,
            false,
            mirror::INITIATIVE,
            (value as i64).max(0),
            Operation::Set,
        );
    }
}

callback!(minimum_upgrade, 39);
callback!(doctrine_folder, 40);

scalar_callback!(account_projection, 46);

unsafe extern "system-unwind" fn pool_store(registers: &Registers, site: usize) {
    let _timing = collector_timing::hook_timing::enter(if site == 51 {
        collector_timing::hook_timing::Family::FactionResourceRebuild
    } else {
        collector_timing::hook_timing::Family::FactionCompanions
    });
    CALLS.increment(site);
    if site == 51 {
        crate::faction_pools::rebuild();
        return;
    }
    let (receiver, resource, index, value, add) = match site {
        53 => (registers.rcx, false, 2, registers.rax as i64, false),
        54 => (registers.rcx, false, 0, registers.r9 as i64, false),
        55 => (registers.rsi, false, 1, registers.rbx as i64, false),
        56 => (registers.rsi, false, 1, registers.rbx as i64, false),
        57 => (registers.rsi, false, 0, registers.rcx as i64, false),
        58 | 59 => (registers.rbp, false, 1, registers.rbx as i64, true),
        _ => unreachable!(),
    };
    crate::faction_pools::observe(receiver, resource, index, value, add);
}

#[cfg(test)]
pub static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

callback!(advisor_60, 60);
callback!(advisor_61, 61);
callback!(advisor_62, 62);
callback!(advisor_63, 63);
callback!(advisor_64, 64);
callback!(advisor_65, 65);

macro_rules! phase_callback {
    ($name:ident,$site:expr,$kind:expr) => {
        unsafe extern "system-unwind" fn $name(a: usize, b: usize, c: usize, d: usize) -> usize {
            let timing = collector_timing::hook_timing::enter(
                collector_timing::hook_timing::Family::FactionRefreshPhases,
            );
            CALLS.increment($site);
            let flight = ManuallyDrop::new(crate::faction_accounts::mirror::Flight::new());
            let original: Function =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let call = || timing.original(|| unsafe { original(a, b, c, d) });
            let result = if $kind == 2 {
                crate::faction_pools::boundary_call(a, 2, call)
            } else if $kind == 4 {
                crate::faction_pools::boundary_call(a, 3, call)
            } else if $kind >= 3 {
                call()
            } else {
                crate::faction_pools::phase_call(a, $kind, call)
            };
            drop(ManuallyDrop::into_inner(flight));
            result
        }
    };
}
phase_callback!(influence_phase, 52, 0);
phase_callback!(rank_phase, 66, 1);

phase_callback!(projection_phase, 76, 2);
phase_callback!(passive_phase, 77, 3);
phase_callback!(influence_share_phase, 78, 4);

unsafe extern "system-unwind" fn extraction_complete(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::FactionResourceRebuild,
    );
    CALLS.increment(51);
    let original: Function = unsafe { std::mem::transmute(ORIGINAL[51].load(Ordering::Acquire)) };
    crate::faction_pools::rebuild_call(|| timing.original(|| unsafe { original(a, b, c, d) }))
}
macro_rules! projection_call {
    ($name:ident,$site:expr,$operand:expr) => {
        unsafe extern "system-unwind" fn $name(a: usize, b: usize, c: usize, d: usize) -> usize {
            let timing = collector_timing::hook_timing::enter(
                collector_timing::hook_timing::Family::FactionCompanions,
            );
            CALLS.increment($site);
            let original: Function =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(a, b, c, d) });
            crate::faction_accounts::mirror::projection_operand(
                $operand,
                result as u32 as i32 as i64,
            );
            result
        }
    };
}
projection_call!(projection_heavy_ship, 71, 3);
projection_call!(projection_other_ship, 72, 4);
projection_call!(projection_combat_air, 73, 5);
projection_call!(projection_other_air, 74, 6);
unsafe extern "system-unwind" fn projection_factory(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::FactionCompanions,
    );
    CALLS.increment(69);
    let original: Function = unsafe { std::mem::transmute(ORIGINAL[69].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c, d) });
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    crate::faction_accounts::mirror::projection_factory_total(engine_native_memory::read::<i64>(
        &memory, result,
    ));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    unsafe extern "system" fn busy_original() -> usize {
        7
    }
    unsafe extern "system" fn busy_callback() -> usize {
        19
    }

    #[test]
    fn busy_attach_publication_remains_retryable() {
        static FLIGHT: AtomicUsize = AtomicUsize::new(1);
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let code = collector_hooks::hook::TestCode::new();
        let (site, bytes) = code.stub(0, busy_original as *const () as usize);
        let callback = busy_callback as *const () as usize;
        let hook = unsafe {
            Hook::new_with_safety(
                site,
                callback,
                bytes,
                collector_hooks::hook::TapSafety {
                    in_flight: &FLIGHT,
                    stub: callback..callback + 1,
                },
            )
        }
        .unwrap();
        let family = MUTATIONS[0].1;
        let mut available = [false; FAMILIES];
        available[family as usize] = true;
        let mut hooks = ManuallyDrop::new(Hooks {
            hooks: vec![(family, OwnedHook::Entry(hook))],
            available,
        });
        assert_eq!(hooks.set_enabled(true), Ok(false));
        assert_eq!(hooks.busy_sites(), [MUTATIONS[0].0.name]);
        {
            let mut round =
                collector_hooks::hook::shared::Round::collect_attach().expect("own attach round");
            collector_hooks::hook::publication::select_family(Some("faction_companions"));
            assert_eq!(
                hooks.set_enabled(true),
                Ok(true),
                "collect records it and goes on"
            );
            round.publish();
            assert_eq!(hooks.set_enabled(true), Ok(false), "the round held it back");
        }
        collector_hooks::hook::publication::select_family(None);
        assert!(!hooks.hooks[0].1.hook().is_enabled());
        FLIGHT.store(0, Ordering::SeqCst);
        assert_eq!(hooks.set_enabled(true), Ok(true));
        assert_eq!(hooks.set_enabled(false), Ok(true));
    }
}
