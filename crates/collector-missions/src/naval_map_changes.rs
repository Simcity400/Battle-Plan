use collector_certificates::watch_registry::WatchRegistry;
use collector_hooks::hook::Hook;
use engine_mutations::navy_ledger_mutations::MUTATIONS as LEDGER_WRITERS;
use engine_native::{Memory, naval_mine_inputs as mines};
use engine_sites::Rva;
use std::{
    ops::Range,
    sync::Mutex,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

static MINES: WatchRegistry = WatchRegistry::retaining();
static ACCIDENTS: WatchRegistry = WatchRegistry::retaining();
static LEDGERS: WatchRegistry = WatchRegistry::retaining();
static METADATA_FAILED: AtomicBool = AtomicBool::new(false);
static METADATA_REFUSALS: AtomicU64 = AtomicU64::new(0);
static LOCAL_ENABLED: AtomicBool = AtomicBool::new(false);
static RESET_ENABLED: AtomicBool = AtomicBool::new(false);
static LEDGER_ENABLED: AtomicBool = AtomicBool::new(false);
static BASE: AtomicUsize = AtomicUsize::new(0);
const SITES: usize =
    mines::WRITERS.len() + engine_naval_history::cnaval_accident_report::WRITERS.len();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
const LEDGER_SITES: usize = LEDGER_WRITERS.len();
static LEDGER_ORIGINAL: [AtomicUsize; LEDGER_SITES] = [const { AtomicUsize::new(0) }; LEDGER_SITES];
static LEDGER_CALLS: [AtomicU64; LEDGER_SITES] = [const { AtomicU64::new(0) }; LEDGER_SITES];
#[derive(Default)]
struct Preparation {
    lifetime: Option<Option<u64>>,
    mine_offer: usize,
    accident_offer: usize,
    ledger_offer: usize,
}
static PREPARATION: Mutex<Preparation> = Mutex::new(Preparation {
    lifetime: None,
    mine_offer: 0,
    accident_offer: 0,
    ledger_offer: 0,
});

fn life_key(navy: usize) -> usize {
    (navy.rotate_left(17) & !1).max(2)
}
fn slice_key(navy: usize, part: usize) -> usize {
    (navy.rotate_left(17) ^ part.wrapping_mul(0x9e37_79b9_7f4a_7c15)) | 1
}
fn lifecycle_receiver(index: usize, receiver: usize) -> usize {
    if index == 6 {
        receiver.saturating_sub(mines::PERSISTENT_RECEIVER)
    } else {
        receiver
    }
}

pub fn prepare(demand: usize) {
    if !LOCAL_ENABLED.load(Ordering::Acquire) || !RESET_ENABLED.load(Ordering::Acquire) {
        return;
    }
    let lifetime = collector_lifetime::world_lifetime::token();
    if lifetime.is_none() {
        return;
    }
    let mut prepared = PREPARATION.lock().unwrap_or_else(|p| p.into_inner());
    let reset = prepared.lifetime != Some(lifetime);
    prepared.lifetime = Some(lifetime);
    let mine_demand = demand
        .saturating_mul(mines::DECAY_SLICES + 1)
        .max(if reset { 0 } else { MINES.registered() });
    let accident_demand = demand.max(if reset { 0 } else { ACCIDENTS.registered() });
    let ledger_demand = demand.max(if reset { 0 } else { LEDGERS.registered() });
    let mine_growth = mine_demand > prepared.mine_offer;
    let accident_growth = accident_demand > prepared.accident_offer;
    let ledger_growth = ledger_demand > prepared.ledger_offer;
    if mine_growth {
        prepared.mine_offer = mine_demand
            .checked_next_power_of_two()
            .unwrap_or(usize::MAX);
    }
    if accident_growth {
        prepared.accident_offer = accident_demand
            .checked_next_power_of_two()
            .unwrap_or(usize::MAX);
    }
    if ledger_growth {
        prepared.ledger_offer = ledger_demand
            .checked_next_power_of_two()
            .unwrap_or(usize::MAX);
    }
    MINES.offer(prepared.mine_offer);
    ACCIDENTS.offer(prepared.accident_offer);
    LEDGERS.offer(prepared.ledger_offer);
    MINES.boundary(reset || mine_growth);
    ACCIDENTS.boundary(reset || accident_growth);
    LEDGERS.boundary(reset || ledger_growth);
}
pub fn mine_quiet() -> Option<(u64, u64)> {
    (!METADATA_FAILED.load(Ordering::Acquire))
        .then(|| MINES.quiet())
        .flatten()
}
pub fn mine_lifetime(navy: usize) -> Option<(u64, u64)> {
    mine_quiet()?;
    MINES.sequence(life_key(navy))
}
pub fn mine_slice(navy: usize, part: usize) -> Option<(u64, u64)> {
    mine_quiet()?;
    (part < mines::DECAY_SLICES)
        .then(|| MINES.sequence(slice_key(navy, part)))
        .flatten()
}
pub fn accident_token(navy: usize) -> Option<(u64, u64)> {
    ACCIDENTS.sequence(navy)
}
#[cfg(not(test))]
pub fn ledger_token(navy: usize) -> Option<(u64, u64)> {
    LEDGERS.sequence(navy)
}

#[cfg(feature = "test-support")]
pub fn ledger_observation_fixture(navies: usize) -> impl Fn(usize) {
    LEDGERS.offer(navies);
    LEDGERS.boundary(false);
    LEDGERS.set_enabled(true);
    |navy| drop(LEDGERS.notify(navy))
}

pub fn active() -> usize {
    MINES
        .active()
        .saturating_add(ACCIDENTS.active())
        .saturating_add(LEDGERS.active())
}
pub fn collect() {
    MINES.collect();
    ACCIDENTS.collect();
    LEDGERS.collect();
}

const REPORT_SITE_NAMES: [&str; engine_naval_history::cnaval_accident_report::WRITERS.len()] = [
    "standing report creation/update",
    "standing report clear",
    "standing report expiry",
    "mine incident",
];
fn report_sites() -> impl Iterator<Item = (&'static str, Rva)> {
    REPORT_SITE_NAMES
        .into_iter()
        .zip(engine_naval_history::cnaval_accident_report::WRITERS)
}

pub struct Hooks {
    hooks: Vec<Hook>,
    ledgers: bool,
}
fn observation_enabled() {
    let enabled = LOCAL_ENABLED.load(Ordering::Acquire) && RESET_ENABLED.load(Ordering::Acquire);
    MINES.set_enabled(enabled);
    ACCIDENTS.set_enabled(enabled);
    LEDGERS.set_enabled(enabled && LEDGER_ENABLED.load(Ordering::Acquire));
}
fn reset_forwarder_enabled(enabled: bool) {
    RESET_ENABLED.store(enabled, Ordering::Release);
    observation_enabled();
}
/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
unsafe fn forwarded_reset(a: usize, b: usize, c: usize, d: usize, original: usize) -> usize {
    let _mine = MINES.notify(life_key(a));
    let _accident = ACCIDENTS.notify(a);
    let _ledger = LEDGERS.notify(a);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(original) };
    unsafe { original(a, b, c, d) }
}
impl Hooks {
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        let callbacks = [
            region_add as *const () as usize,
            index_add as *const () as usize,
            decay as *const () as usize,
            construct as *const () as usize,
            grow as *const () as usize,
            construct as *const () as usize,
            load as *const () as usize,
            accident_update as *const () as usize,
            accident_clear as *const () as usize,
            accident_expire as *const () as usize,
            mine_report as *const () as usize,
        ];
        let mut hooks = Vec::new();
        if cfg!(feature = "mission_changes") {
            let descriptors = mines::WRITERS.into_iter().chain(report_sites());
            for (index, (name, rva)) in descriptors.enumerate() {
                if index == 5 {
                    continue;
                }
                let fail = |action: &str, reason: &str| {
                    crate::mission_changes::hook_failure("naval_map_changes", action, name, reason)
                };
                let (at, prefix) = engine_sites::hook(base, rva)
                    .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
                    .map_err(|reason| fail("verification failed", reason))?;
                let hook = unsafe { Hook::new(at, callbacks[index], prefix) }
                    .map_err(|reason| fail("construction failed", &reason))?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        let mut located = [(0, [0; 16]); LEDGER_SITES];
        let ledgers = cfg!(feature = "navy_ledger_changes")
            && LEDGER_WRITERS.iter().zip(&mut located).all(|(site, slot)| {
                collector_hooks::located_hook(base, site.rva)
                    .map(|code| *slot = code)
                    .is_ok()
            });
        if ledgers {
            let callbacks = [
                register_mission as *const () as usize,
                unregister_mission as *const () as usize,
            ];
            for (index, (site, &(at, original))) in LEDGER_WRITERS.iter().zip(&located).enumerate()
            {
                let hook =
                    unsafe { Hook::new(at, callbacks[index], original) }.map_err(|reason| {
                        crate::mission_changes::hook_failure(
                            "naval_map_changes",
                            "construction failed",
                            site.name,
                            &reason,
                        )
                    })?;
                LEDGER_ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        BASE.store(base, Ordering::Release);
        collector_observers::world_input_changes::navy_reset_forwarder(Some(
            collector_observers::world_input_changes::NavyResetForwarder {
                invoke: forwarded_reset,
                enabled: reset_forwarder_enabled,
            },
        ));
        Ok(Self { hooks, ledgers })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            LOCAL_ENABLED.store(false, Ordering::Release);
            LEDGER_ENABLED.store(false, Ordering::Release);
            observation_enabled();
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled).map_err(|reason| {
            crate::mission_changes::hook_failure(
                "naval_map_changes",
                "publication failed",
                "mine and report writers",
                reason,
            )
        })? {
            return Ok(false);
        }
        LOCAL_ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::Release);
        LEDGER_ENABLED.store(enabled && self.ledgers, Ordering::Release);
        observation_enabled();
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        collector_observers::world_input_changes::navy_reset_forwarder(None);
        LOCAL_ENABLED.store(false, Ordering::Release);
        LEDGER_ENABLED.store(false, Ordering::Release);
        unsafe {
            MINES.release();
            ACCIDENTS.release();
            LEDGERS.release();
        }
        crate::naval_mines::release();
        *PREPARATION.lock().unwrap_or_else(|p| p.into_inner()) = Preparation::default();
        METADATA_FAILED.store(false, Ordering::Release);
    }
}

#[cfg(test)]
pub fn patch_sites() -> Vec<(&'static str, Rva)> {
    mines::WRITERS
        .into_iter()
        .enumerate()
        .filter(|(index, _)| *index != 5)
        .map(|(_, site)| site)
        .chain(report_sites())
        .chain(LEDGER_WRITERS.iter().map(|site| (site.name, site.rva)))
        .collect()
}

/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn original(index: usize, a: usize, b: usize, c: usize, d: usize) -> usize {
    let native: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[index].load(Ordering::Acquire)) };
    unsafe { native(a, b, c, d) }
}

fn event_slice(index: usize, navy: usize, second: usize) -> Option<usize> {
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    {
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let count = mines::event_count(&memory, navy)?;
        if index == 2 {
            mines::event_decay_slice(&memory, BASE.load(Ordering::Acquire))
        } else {
            let region = if index == 0 {
                engine_native::read::<u32>(
                    &memory,
                    second.checked_add(engine_native::cstrategic_region::ID)?,
                )?
            } else {
                u32::try_from(second).ok()?
            };
            mines::region_slice(count, region)
        }
    }
    #[cfg(not(all(windows, target_env = "msvc", target_arch = "x86_64")))]
    {
        let _ = (index, navy, second);
        None
    }
}

/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn changed(index: usize, a: usize, b: usize, c: usize, d: usize) -> usize {
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::MissionChanges);
    let _running = MINES.running();
    let part = event_slice(index, a, b);
    if part.is_none() {
        METADATA_REFUSALS.fetch_add(1, Ordering::Relaxed);
        METADATA_FAILED.store(true, Ordering::Release);
    }
    let _mark = part.map(|part| MINES.notify(slice_key(a, part)));
    timing.original(|| unsafe { original(index, a, b, c, d) })
}
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn lifecycle(index: usize, a: usize, b: usize, c: usize, d: usize) -> usize {
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::MissionChanges);
    let navy = lifecycle_receiver(index, a);
    let _mine = MINES.notify(life_key(navy));
    let _accident = ACCIDENTS.notify(navy);
    let _ledger = LEDGERS.notify(navy);
    timing.original(|| unsafe { original(index, a, b, c, d) })
}
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn ledger(index: usize, a: usize, b: usize, c: usize, d: usize) -> usize {
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::MissionChanges);
    LEDGER_CALLS[index].fetch_add(1, Ordering::Relaxed);
    let native: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(LEDGER_ORIGINAL[index].load(Ordering::Acquire)) };
    ledger_write(&LEDGERS, a, || {
        timing.original(|| unsafe { native(a, b, c, d) })
    })
}
fn ledger_write(ledgers: &WatchRegistry, navy: usize, write: impl FnOnce() -> usize) -> usize {
    let _mark = ledgers.notify(navy);
    write()
}
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn reports(index: usize, a: usize, b: usize, c: usize, d: usize) -> usize {
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::MissionChanges);
    let _mark = ACCIDENTS.notify(a);
    timing.original(|| unsafe { original(index, a, b, c, d) })
}
macro_rules! detour {
    ($name:ident,$body:ident,$index:expr) => {
        unsafe extern "system" fn $name(a: usize, b: usize, c: usize, d: usize) -> usize {
            unsafe { $body($index, a, b, c, d) }
        }
    };
}
detour!(region_add, changed, 0);
detour!(index_add, changed, 1);
detour!(decay, changed, 2);
detour!(construct, lifecycle, 3);
detour!(grow, lifecycle, 4);
detour!(load, lifecycle, 6);
detour!(accident_update, reports, 7);
detour!(accident_clear, reports, 8);
detour!(accident_expire, reports, 9);
detour!(mine_report, reports, 10);
detour!(register_mission, ledger, 0);
detour!(unregister_mission, ledger, 1);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_report_hook_error_and_registered_patch_keeps_its_purpose_name() {
        let sites: Vec<_> = report_sites().collect();
        assert_eq!(sites.len(), 4);
        for ((name, rva), expected) in sites.into_iter().zip([
            "standing report creation/update",
            "standing report clear",
            "standing report expiry",
            "mine incident",
        ]) {
            assert_eq!(name, expected);
            assert!(patch_sites().contains(&(name, rva)));
            let error = crate::mission_changes::hook_failure(
                "naval_map_changes",
                "verification failed",
                name,
                "synthetic refusal",
            );
            assert!(error.contains(expected));
            assert!(error.contains("naval_map_changes"));
            assert!(error.contains("synthetic refusal"));
        }
    }
    #[test]
    fn mine_slices_and_accidents_have_independent_lifetimes() {
        let mines = WatchRegistry::new();
        let reports = WatchRegistry::new();
        for registry in [&mines, &reports] {
            registry.offer(100);
            registry.boundary(false);
            registry.set_enabled(true);
        }
        let navy = 0x12000;
        let life = mines.sequence(life_key(navy));
        let slices = [0, 1].map(|part| mines.sequence(slice_key(navy, part)));
        let accident = reports.sequence(navy);
        drop(mines.notify(slice_key(navy, 0)));
        assert_ne!(mines.sequence(slice_key(navy, 0)), slices[0]);
        assert_eq!(mines.sequence(slice_key(navy, 1)), slices[1]);
        assert_eq!(mines.sequence(life_key(navy)), life);
        assert_eq!(reports.sequence(navy), accident);
        let active = mines.notify(life_key(navy));
        assert!(mines.quiet().is_none());
        drop(active);
        assert_ne!(mines.sequence(life_key(navy)), life);
    }
    #[test]
    fn a_ledger_write_moves_only_its_navys_certificate_across_the_write() {
        let ledgers = WatchRegistry::retaining();
        ledgers.offer(64);
        ledgers.boundary(false);
        ledgers.set_enabled(true);
        let (navy, other) = (0x7_1000, 0x7_2000);
        let held = ledgers.sequence(navy).unwrap();
        let unrelated = ledgers.sequence(other).unwrap();
        for _ in 0..LEDGER_SITES {
            let before = ledgers.sequence(navy).unwrap();
            let written = ledger_write(&ledgers, navy, || {
                assert!(ledgers.sequence(navy).is_none());
                navy ^ 0x30
            });
            assert_eq!(written, navy ^ 0x30);
            assert_ne!(ledgers.sequence(navy), Some(before));
        }
        assert_ne!(ledgers.sequence(navy), Some(held));
        assert_eq!(ledgers.sequence(other), Some(unrelated));
    }
    #[test]
    fn only_the_persistent_loader_normalizes_its_secondary_receiver() {
        for index in 3..6 {
            assert_eq!(lifecycle_receiver(index, 0x1000), 0x1000);
        }
        assert_eq!(lifecycle_receiver(6, 0x1008), 0x1000);
        assert_eq!(lifecycle_receiver(6, 0), 0);
    }
}
