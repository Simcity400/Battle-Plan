use crate::{
    hook::Hook,
    watch_registry::{Notification, Running, WatchRegistry},
};
use engine_mutations::orders_group_mutations::{DAILY_UPDATE, MUTATIONS, Receiver};
use engine_native_memory::Memory;
use std::{
    mem::ManuallyDrop,
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static PATCHED: AtomicBool = AtomicBool::new(false);
static FORWARDED: AtomicBool = AtomicBool::new(false);
static READS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static GROUPS: AtomicUsize = AtomicUsize::new(0);

pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    destroyed: AtomicU64,
    untouched_daily: AtomicU64,
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
            destroyed: AtomicU64::new(0),
            untouched_daily: AtomicU64::new(0),
        }
    }

    pub fn notify(&self, group: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(group)
    }

    pub fn daily(&self, group: usize, trains_to_maximum: bool) -> Guard<'_> {
        if trains_to_maximum {
            return Guard::Notified(self.notify(group, DAILY_UPDATE));
        }
        self.calls[DAILY_UPDATE].fetch_add(1, Ordering::Relaxed);
        self.untouched_daily.fetch_add(1, Ordering::Relaxed);
        Guard::Running(self.registry.running())
    }

    pub fn destroy(&self, group: usize) -> Notification<'_> {
        self.destroyed.fetch_add(1, Ordering::Relaxed);
        self.registry.notify(group)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enable(&self, groups: usize) {
        self.registry.offer(groups);
        self.registry.boundary(false);
        self.registry.set_enabled(true);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn disable(&self) {
        self.registry.set_enabled(false);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
    }
}

pub enum Guard<'a> {
    Notified(Notification<'a>),
    Running(Running<'a>),
}

pub trait Changes: Sync {
    fn sequence(&self, group: usize) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn sequence(&self, group: usize) -> Option<(u64, u64)> {
        self.registry.sequence(group)
    }
}

pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, group: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(group)
    }
}

pub fn active() -> usize {
    WATCHES.registry.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
    crate::order_membership_changes::boundary(invalidate);
}

pub fn prepare(groups: usize) {
    GROUPS.fetch_max(groups, Ordering::Relaxed);
    presize();
}

pub fn presize() {
    let groups = GROUPS.load(Ordering::Relaxed);
    if groups != 0 {
        WATCHES.registry.offer(groups);
    }
}

pub fn collect() {
    crate::order_membership_changes::collect();
    WATCHES.registry.collect();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub carried: u64,
    pub read: u64,
}

pub fn record(counts: &Counts) {
    for (target, value) in READS.iter().zip([counts.carried, counts.read]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    GROUPS.fetch_max((counts.carried + counts.read) as usize, Ordering::Relaxed);
}


fn observe() {
    let enabled = PATCHED.load(Ordering::SeqCst) && FORWARDED.load(Ordering::SeqCst);
    WATCHES.registry.set_enabled(enabled);
    crate::order_membership_changes::forwarder(enabled);
}

pub fn forwarder(enabled: bool) {
    FORWARDED.store(enabled, Ordering::SeqCst);
    observe();
}

pub fn forwarded(group: usize) -> Notification<'static> {
    WATCHES.destroy(group)
}

fn hook_failure(site: Option<usize>, action: &str, detail: &str) -> String {
    let sites = site.map_or_else(
        || {
            MUTATIONS
                .iter()
                .map(|(site, _)| site.name)
                .collect::<Vec<_>>()
                .join(", ")
        },
        |index| MUTATIONS[index].0.name.to_owned(),
    );
    format!("orders_group_changes [{sites}] hook {action} failed: {detail}")
}

fn validate_sites(base: usize) -> Result<(), String> {
    for (index, (site, _)) in MUTATIONS.iter().enumerate() {
        crate::verified(base, site.rva)
            .map_err(|reason| hook_failure(Some(index), "sentinel validation", reason))?;
    }
    Ok(())
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction. A differing
    /// sentinel refuses installation with the site; the census then reads every group.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in this
    /// process. The caller owns this family exclusively and must restore its hooks and
    /// quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        validate_sites(base)?;
        let callbacks: [usize; SITES] = [
            set_leader as *const () as usize,
            set_training as *const () as usize,
            assign_to_army_group as *const () as usize,
            daily_update as *const () as usize,
            set_deployed_leader as *const () as usize,
            clear_hq_provider as *const () as usize,
        ];
        let mut hooks = Vec::new();
        for (index, (site, _)) in MUTATIONS.iter().enumerate() {
            let code = engine_sites::hook(base, site.rva)?;
            let hook = unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                .map_err(|reason| hook_failure(Some(index), "construction", &reason))?;
            ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
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
        // SAFETY: the watch tables are dropped only once these hooks and the forwarding
        // template hooks were never enabled, or were removed and drained after the last
        // capture.
        unsafe { WATCHES.registry.release() };
    }
}

fn trains_to_maximum(group: usize) -> bool {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    [
        engine_native::corders_group::TRAINING,
        engine_native::corders_group::STOP_AT_MAXIMUM,
    ]
    .into_iter()
    .all(|offset| {
        group
            .checked_add(offset)
            .and_then(|at| engine_native_memory::read::<u8>(&memory, at))
            != Some(0)
    })
}

type NativeFunction = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system-unwind" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::OrdersGroupChanges);
            let group = match MUTATIONS[$site].1 {
                Receiver::First => first,
                Receiver::Second => second,
            };
            let membership = ManuallyDrop::new(
                ($site == 2).then(|| crate::order_membership_changes::group_changing(group)),
            );
            let notification = ManuallyDrop::new(if $site == DAILY_UPDATE {
                WATCHES.daily(group, trains_to_maximum(group))
            } else {
                Guard::Notified(WATCHES.notify(group, $site))
            });
            let original: NativeFunction =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(first, second, third, fourth) });
            drop(ManuallyDrop::into_inner(notification));
            drop(ManuallyDrop::into_inner(membership));
            result
        }
    };
}
callback!(set_leader, 0);
callback!(set_training, 1);
callback!(assign_to_army_group, 2);
callback!(daily_update, DAILY_UPDATE);
callback!(set_deployed_leader, 4);
callback!(clear_hq_provider, 5);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_refusals_identify_the_exact_site_before_any_patch_is_built() {
        let error = validate_sites(0).unwrap_err();
        assert!(error.contains("orders_group_changes"));
        assert!(error.contains(MUTATIONS[0].0.name));
        assert!(error.contains("sentinel validation"));
    }

    #[test]
    fn writers_move_only_their_groups_certificates() {
        let watches = Watches::new();
        watches.enable(16);
        let held = |group| Changes::sequence(&watches, group).unwrap();
        let first = held(0x1000);
        for site in (0..SITES).filter(|&site| site != DAILY_UPDATE) {
            let first = held(0x1000);
            let second = held(0x2000);
            drop(watches.notify(0x2000, site));
            assert_eq!(held(0x1000), first, "{}", MUTATIONS[site].0.name);
            assert_ne!(held(0x2000), second, "{}", MUTATIONS[site].0.name);
            assert_eq!(watches.calls[site].load(Ordering::Relaxed), 1);
        }
        let second = held(0x2000);
        let untouched = watches.daily(0x2000, false);
        assert!(matches!(untouched, Guard::Running(_)));
        assert_eq!(watches.registry.active(), 1);
        drop(untouched);
        assert_eq!(watches.registry.active(), 0);
        assert_eq!(held(0x2000), second);
        drop(watches.daily(0x2000, true));
        assert_ne!(held(0x2000), second);
        assert_eq!(watches.calls[DAILY_UPDATE].load(Ordering::Relaxed), 2);
        assert_eq!(watches.untouched_daily.load(Ordering::Relaxed), 1);
        drop(watches.destroy(0x1000));
        assert_ne!(held(0x1000), first);
        assert_eq!(watches.destroyed.load(Ordering::Relaxed), 1);
        let running = watches.notify(0x1000, 0);
        assert!(Changes::sequence(&watches, 0x1000).is_none());
        drop(running);
        let before = held(0x1000);
        watches.boundary(true);
        assert_ne!(held(0x1000), before);
    }

    #[test]
    fn the_domain_needs_its_patches_and_the_forwarder() {
        WATCHES.enable(16);
        for (patched, forwarded) in [(false, false), (true, false), (false, true)] {
            PATCHED.store(patched, Ordering::SeqCst);
            forwarder(forwarded);
            assert_eq!(Observer.sequence(0x1000), None);
        }
        PATCHED.store(true, Ordering::SeqCst);
        forwarder(true);
        let before = Observer.sequence(0x1000).expect("observed");
        drop(forwarded(0x1000));
        assert_ne!(Observer.sequence(0x1000), Some(before));
        forwarder(false);
        assert_eq!(Observer.sequence(0x1000), None);
    }

    #[test]
    fn only_a_group_training_to_its_maximum_counts_as_written_daily() {
        let mut group = vec![0_u8; engine_native::corders_group::SIZE];
        let at = group.as_mut_ptr() as usize;
        assert!(!trains_to_maximum(at));
        group[engine_native::corders_group::TRAINING] = 1;
        assert!(!trains_to_maximum(at));
        group[engine_native::corders_group::STOP_AT_MAXIMUM] = 1;
        assert!(trains_to_maximum(at));
        group[engine_native::corders_group::TRAINING] = 0;
        assert!(!trains_to_maximum(at));
        assert!(trains_to_maximum(0));
    }
}
