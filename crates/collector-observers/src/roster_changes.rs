use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{character_roster_mutations::Domain, character_roster_mutations::MUTATIONS};
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static ROSTER: WatchRegistry = WatchRegistry::new();
static SCIENTISTS: WatchRegistry = WatchRegistry::new();
static CALLS: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
static FORWARDED: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static OWN: AtomicBool = AtomicBool::new(false);
static MANAGER_FORWARD: AtomicBool = AtomicBool::new(false);
static REPAIR_FORWARD: AtomicBool = AtomicBool::new(false);

static READS: [[AtomicU64; 2]; TABLES.len()] =
    [const { [const { AtomicU64::new(0) }; 2] }; TABLES.len()];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    Characters,
    ScriptFlags,
    CharacterRoleTraits,
}
const TABLES: [&str; 3] = ["Characters", "ScriptFlags", "CharacterRoleTraits"];

#[cfg(any(test, feature = "test-support"))]
/// # Safety
/// The fixture must have exclusive use of this roster observer and no installed roster hooks.
pub unsafe fn with_quiet_roster_for_test<R>(check: impl FnOnce() -> R) -> R {
    struct Disable;
    impl Drop for Disable {
        fn drop(&mut self) {
            ROSTER.set_enabled(false);
        }
    }
    ROSTER.set_enabled(true);
    let _disable = Disable;
    check()
}

pub fn roster() -> Option<(u64, u64)> {
    ROSTER.quiet()
}

pub fn scientists() -> Option<(u64, u64)> {
    SCIENTISTS.quiet()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub quiet: bool,
    pub read: bool,
}

pub trait Roster: Sync {
    fn token(&self) -> Option<(u64, u64)>;
    fn record(&self, table: Table, counts: &Counts);
}

#[cfg_attr(any(test, feature = "test-support"), allow(dead_code))]
pub struct Observer;
impl Roster for Observer {
    fn token(&self) -> Option<(u64, u64)> {
        roster()
    }
    fn record(&self, table: Table, counts: &Counts) {
        record(table, counts);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct Isolated {
    pub registry: WatchRegistry,
    counts: [AtomicU64; 2],
}

#[cfg(any(test, feature = "test-support"))]
impl Default for Isolated {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Isolated {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::new(),
            counts: [const { AtomicU64::new(0) }; 2],
        }
    }

    pub fn enable(&self) {
        self.registry.set_enabled(true);
    }

    pub fn write(&self) {
        drop(self.registry.notify(0));
    }

    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }

    pub fn counts(&self) -> [u64; 2] {
        self.counts
            .each_ref()
            .map(|count| count.load(Ordering::Relaxed))
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Roster for Isolated {
    fn token(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn record(&self, _table: Table, counts: &Counts) {
        for (target, value) in self
            .counts
            .iter()
            .zip([u64::from(counts.quiet), u64::from(counts.read)])
        {
            target.fetch_add(value, Ordering::Relaxed);
        }
    }
}

pub fn record(table: Table, counts: &Counts) {
    for (target, value) in READS[table as usize]
        .iter()
        .zip([u64::from(counts.quiet), u64::from(counts.read)])
    {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

pub fn active() -> usize {
    ROSTER.active() + SCIENTISTS.active()
}

pub fn boundary(invalidate: bool) {
    ROSTER.boundary(invalidate);
    SCIENTISTS.boundary(invalidate);
}

fn refresh() {
    let own = OWN.load(Ordering::SeqCst);
    ROSTER.set_enabled(
        own && MANAGER_FORWARD.load(Ordering::SeqCst) && REPAIR_FORWARD.load(Ordering::SeqCst),
    );
    SCIENTISTS.set_enabled(own);
}

pub fn set_manager_forward(enabled: bool) {
    MANAGER_FORWARD.store(enabled, Ordering::SeqCst);
    refresh();
}

pub fn set_repair_forward(enabled: bool) {
    REPAIR_FORWARD.store(enabled, Ordering::SeqCst);
    refresh();
}

pub fn manager_constructed() -> Notification<'static> {
    FORWARDED[0].fetch_add(1, Ordering::Relaxed);
    ROSTER.notify(0)
}

pub fn character_repaired() -> Notification<'static> {
    FORWARDED[1].fetch_add(1, Ordering::Relaxed);
    ROSTER.notify(0)
}

fn notify(site: usize) -> Notification<'static> {
    CALLS[site].fetch_add(1, Ordering::Relaxed);
    match MUTATIONS[site].1 {
        Domain::Roster => ROSTER.notify(0),
        Domain::Scientists => SCIENTISTS.notify(0),
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; every capture then
    /// reads afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "roster_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _)| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; SITES] = [
                roster_push as *const () as usize,
                roster_clear as *const () as usize,
                scientist_build as *const () as usize,
                scientist_attach as *const () as usize,
                scientist_clear as *const () as usize,
                scientist_trait_add as *const () as usize,
                scientist_trait_remove as *const () as usize,
                scientist_trait_reset as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            OWN.store(false, Ordering::SeqCst);
            refresh();
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Roster,
                false,
            );
            crate::census_changes::forwarder(crate::census_changes::Forwarder::Roster, false);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        OWN.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        refresh();
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Roster,
            enabled && !self.hooks.is_empty(),
        );
        crate::census_changes::forwarder(
            crate::census_changes::Forwarder::Roster,
            enabled && !self.hooks.is_empty(),
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the domains hold no table; release
        // only starts an epoch once the hooks were removed and drained.
        unsafe {
            ROSTER.release();
            SCIENTISTS.release();
        }
    }
}

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::RosterChanges);
            let _notification = notify($site);
            let _inputs = crate::country_input_changes::forwarded_roster($site, first, second);
            let _census = match $site {
                sites::SCIENTIST_ATTACH => {
                    crate::census_changes::forwarded(crate::census_changes::Site::ScientistAttach)
                }
                sites::SCIENTIST_CLEAR => {
                    crate::census_changes::forwarded(crate::census_changes::Site::ScientistClear)
                }
                _ => None,
            };
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
use engine_mutations::character_roster_mutations as sites;
callback!(roster_push, sites::PUSH);
callback!(roster_clear, sites::CLEAR);
callback!(scientist_build, sites::SCIENTIST_BUILD);
callback!(scientist_attach, sites::SCIENTIST_ATTACH);
callback!(scientist_clear, sites::SCIENTIST_CLEAR);
callback!(scientist_trait_add, sites::SCIENTIST_TRAIT_ADD);
callback!(scientist_trait_remove, sites::SCIENTIST_TRAIT_REMOVE);
callback!(scientist_trait_reset, sites::SCIENTIST_TRAIT_RESET);
