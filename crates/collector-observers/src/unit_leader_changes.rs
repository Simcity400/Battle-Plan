use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{
    unit_leader_mutations::EXILE_GENERATION, unit_leader_mutations::EXILE_REMOVAL,
    unit_leader_mutations::MUTATIONS,
};
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static PATCHED: AtomicBool = AtomicBool::new(false);
static FORWARDED: AtomicBool = AtomicBool::new(false);
static READS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static LEADERS: AtomicUsize = AtomicUsize::new(0);
static OPERATIVE_EFFECTS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifetime {
    Construct,
    CopyConstruct,
    Destroy,
}
const LIFETIMES: [&str; 3] = ["construct", "copy_construct", "destroy"];

pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    lifetimes: [AtomicU64; LIFETIMES.len()],
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
            lifetimes: [const { AtomicU64::new(0) }; LIFETIMES.len()],
        }
    }

    pub fn notify(&self, leader: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        if site == EXILE_GENERATION || site == EXILE_REMOVAL {
            self.registry.notify_every()
        } else {
            self.registry.notify(leader)
        }
    }

    pub fn lifetime(&self, leader: usize, site: Lifetime) -> Notification<'_> {
        self.lifetimes[site as usize].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(leader)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enable(&self, leaders: usize) {
        self.registry.offer(leaders);
        self.registry.boundary(false);
        self.registry.set_enabled(true);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

pub trait Changes: Sync {
    fn quiet(&self) -> Option<(u64, u64)>;
    fn sequence(&self, leader: usize) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, leader: usize) -> Option<(u64, u64)> {
        self.registry.sequence(leader)
    }
}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.registry.quiet()
    }
    fn sequence(&self, leader: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(leader)
    }
}

pub fn active() -> usize {
    WATCHES.registry.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

pub fn presize() {
    let leaders = LEADERS
        .load(Ordering::Relaxed)
        .saturating_add(OPERATIVE_EFFECTS.load(Ordering::Relaxed));
    if leaders != 0 {
        WATCHES.registry.offer(leaders);
    }
}

pub fn collect() {
    WATCHES.registry.collect();
}

pub fn operative_effects(receivers: usize) {
    OPERATIVE_EFFECTS.fetch_max(receivers, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
}

pub fn record(counts: &Counts, leaders: usize) {
    for (target, value) in READS.iter().zip([counts.reused, counts.read, 1]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    LEADERS.store(leaders, Ordering::Relaxed);
}

fn observe() {
    WATCHES.registry.set_enabled(
        cfg!(feature = "unit_leader_changes")
            && PATCHED.load(Ordering::SeqCst)
            && FORWARDED.load(Ordering::SeqCst),
    );
}

pub fn forwarder(enabled: bool) {
    FORWARDED.store(enabled, Ordering::SeqCst);
    observe();
}

pub fn forwarded(leader: usize, site: Lifetime) -> Notification<'static> {
    WATCHES.lifetime(leader, site)
}

fn hook_failure(site: Option<usize>, action: &str, detail: &str) -> String {
    let sites = site.map_or_else(
        || {
            MUTATIONS
                .iter()
                .map(|site| site.name)
                .collect::<Vec<_>>()
                .join(", ")
        },
        |index| MUTATIONS[index].name.to_owned(),
    );
    format!("unit_leader_changes [{sites}] hook {action} failed: {detail}")
}

fn validate_sites(base: usize) -> Result<(), String> {
    for (index, site) in MUTATIONS.iter().enumerate() {
        crate::verified(base, site.rva)
            .map_err(|reason| hook_failure(Some(index), "sentinel validation", reason))?;
    }
    Ok(())
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel refuses installation with the owning family and site.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        let mut hooks = Vec::new();
        if cfg!(feature = "unit_leader_changes") {
            validate_sites(base)?;
            let callbacks: [usize; SITES] = [
                formation as *const () as usize,
                skill as *const () as usize,
                category as *const () as usize,
                army_attack as *const () as usize,
                army_defense as *const () as usize,
                army_planning as *const () as usize,
                army_logistics as *const () as usize,
                navy_attack as *const () as usize,
                navy_defense as *const () as usize,
                navy_maneuvering as *const () as usize,
                navy_coordination as *const () as usize,
                capture as *const () as usize,
                release as *const () as usize,
                owner as *const () as usize,
                operative_owner as *const () as usize,
                exile_generation as *const () as usize,
                exile_removal as *const () as usize,
                set_name as *const () as usize,
                rebuild_derived as *const () as usize,
                operative_rebuild as *const () as usize,
                link_character as *const () as usize,
                operative_codename as *const () as usize,
                operative_admission as *const () as usize,
                experience_add as *const () as usize,
                experience_add_unchecked as *const () as usize,
                daily_progress as *const () as usize,
                trait_progress_add as *const () as usize,
                trait_progress_remove as *const () as usize,
                trait_progress_prune as *const () as usize,
                trait_progress_complete as *const () as usize,
                trait_factor_set as *const () as usize,
                trait_factor_remove as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|reason| hook_failure(Some(index), "construction", &reason))?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            PATCHED.store(false, Ordering::SeqCst);
            observe();
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|reason| hook_failure(None, "publication", &reason))?
        {
            return Ok(false);
        }
        PATCHED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        observe();
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once these hooks and the forwarding naval tree hooks (dropped
        // before these) were never enabled, or were removed and drained after
        // the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            leader: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::UnitLeaderChanges);
            let _notification = WATCHES.notify(leader, $site);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(leader, second, third, fourth) })
        }
    };
}
callback!(formation, 0);
callback!(skill, 1);
callback!(category, 2);
callback!(army_attack, 3);
callback!(army_defense, 4);
callback!(army_planning, 5);
callback!(army_logistics, 6);
callback!(navy_attack, 7);
callback!(navy_defense, 8);
callback!(navy_maneuvering, 9);
callback!(navy_coordination, 10);
callback!(capture, 11);
callback!(release, 12);
callback!(owner, 13);
callback!(operative_owner, 14);
callback!(exile_generation, EXILE_GENERATION);
callback!(exile_removal, EXILE_REMOVAL);
callback!(set_name, 17);
callback!(rebuild_derived, 18);
callback!(operative_rebuild, 19);
callback!(link_character, 20);
callback!(operative_codename, 21);
callback!(operative_admission, 22);
callback!(experience_add, 23);
callback!(experience_add_unchecked, 24);
callback!(daily_progress, 25);
callback!(trait_progress_add, 26);
callback!(trait_progress_remove, 27);
callback!(trait_progress_prune, 28);
callback!(trait_progress_complete, 29);
callback!(trait_factor_set, 30);
callback!(trait_factor_remove, 31);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writers_move_their_leaders_certificates() {
        let watches = Watches::new();
        watches.enable(16);
        let held = |leader| Changes::sequence(&watches, leader).unwrap();
        let [first, second] = [0x1000, 0x2000].map(held);
        drop(watches.notify(0x2000, 0));
        assert_eq!(held(0x1000), first);
        assert_ne!(held(0x2000), second);
        let second = held(0x2000);
        drop(watches.lifetime(0x1000, Lifetime::Destroy));
        assert_ne!(held(0x1000), first);
        assert_eq!(held(0x2000), second);
        let first = held(0x1000);
        drop(watches.notify(0, EXILE_REMOVAL));
        assert_ne!(held(0x1000), first);
        assert_ne!(held(0x2000), second);
        assert_eq!(watches.calls[0].load(Ordering::Relaxed), 1);
        assert_eq!(watches.calls[EXILE_REMOVAL].load(Ordering::Relaxed), 1);
        assert_eq!(
            watches.lifetimes[Lifetime::Destroy as usize].load(Ordering::Relaxed),
            1
        );
        let running = watches.notify(0x1000, 1);
        assert!(Changes::sequence(&watches, 0x1000).is_none());
        drop(running);
    }
}
