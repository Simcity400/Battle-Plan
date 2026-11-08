//! Allocation-free notifications from the native writers of the orders-group fields the
//! division census selects through ([`engine_mutations::orders_group_mutations`]). A detour
//! notifies the watch registry ([`WatchRegistry`]) for the group it names, counts the call
//! and calls the original; it never changes game behaviour. Group destruction is the
//! template family's patch, which forwards it ([`forwarded`]), so a reused address never
//! keeps a freed group's certificate. The domain observes only while this family's patches
//! and the forwarder are enabled; anything unobserved (disabled, poisoned, a refused
//! registration, an epoch change, a running writer) makes the census read that group afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, Running, WatchRegistry},
};
use engine_mutations::orders_group_mutations::{DAILY_UPDATE, MUTATIONS, Receiver};
use engine_native::Memory;
use std::{
    mem::ManuallyDrop,
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// This family's patches are enabled.
static PATCHED: AtomicBool = AtomicBool::new(false);
/// The template family's destructor patch is enabled.
static FORWARDED: AtomicBool = AtomicBool::new(false);
/// Carried and read groups, since attach.
static READS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
/// Peak groups the group census or a census part reached, for off-thread sizing.
static GROUPS: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched groups and each site's calls.
pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    destroyed: AtomicU64,
    /// Daily updates of groups not training to their maximum, which write no watched field.
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

    /// Hook path: the writer of `site` ([`MUTATIONS`] index) is about to change `group`.
    /// Dropping the guard notifies its return.
    pub fn notify(&self, group: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(group)
    }

    /// Hook path: the daily update of `group`, which can clear its training only when it
    /// trains to its maximum. Either guard counts as running until it drops.
    pub fn daily(&self, group: usize, trains_to_maximum: bool) -> Guard<'_> {
        if trains_to_maximum {
            return Guard::Notified(self.notify(group, DAILY_UPDATE));
        }
        self.calls[DAILY_UPDATE].fetch_add(1, Ordering::Relaxed);
        self.untouched_daily.fetch_add(1, Ordering::Relaxed);
        Guard::Running(self.registry.running())
    }

    /// Hook path: the forwarded destructor is about to free `group`.
    pub fn destroy(&self, group: usize) -> Notification<'_> {
        self.destroyed.fetch_add(1, Ordering::Relaxed);
        self.registry.notify(group)
    }

    /// An enabled domain with a table for `groups` receivers installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enable(&self, groups: usize) {
        self.registry.offer(groups);
        self.registry.boundary(false);
        self.registry.set_enabled(true);
    }

    /// Stop observing, as a failed or removed patch leaves the domain.
    #[cfg(any(test, feature = "test-support"))]
    pub fn disable(&self) {
        self.registry.set_enabled(false);
    }

    /// Game-thread boundary for tests: `invalidate` as after a load, gap or recovery.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
    }
}

/// A hook's guard over its native call: a notification for the group it writes, or, for
/// a daily update that cannot write it, a running guard that revokes nothing.
pub enum Guard<'a> {
    Notified(Notification<'a>),
    Running(Running<'a>),
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// `(epoch, sequence)` of one group, registering it; `None` when it cannot be observed.
    /// Equal values across hours prove that no observed writer touched it.
    fn sequence(&self, group: usize) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn sequence(&self, group: usize) -> Option<(u64, u64)> {
        self.registry.sequence(group)
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, group: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(group)
    }
}

/// Notifications running now, forwarded ones included.
pub fn active() -> usize {
    WATCHES.registry.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate` after a load, gap
/// or recovery: nothing observed before it certifies anything.
pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

/// Off-thread: the group census counts every registered group before installation.
pub fn prepare(groups: usize) {
    GROUPS.fetch_max(groups, Ordering::Relaxed);
    presize();
}

/// Off the game thread: size the watches for the most groups reached.
pub fn presize() {
    let groups = GROUPS.load(Ordering::Relaxed);
    if groups != 0 {
        WATCHES.registry.offer(groups);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One census part's groups.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub carried: u64,
    pub read: u64,
}

/// A census part publishes its counts; every group it reached sizes the watches.
pub fn record(counts: &Counts) {
    for (target, value) in READS.iter().zip([counts.carried, counts.read]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    GROUPS.fetch_max((counts.carried + counts.read) as usize, Ordering::Relaxed);
}


/// Observe while this family's patches and the forwarder's are enabled. Enabling starts a
/// new epoch, so nothing held from before a gap certifies.
fn observe() {
    WATCHES
        .registry
        .set_enabled(PATCHED.load(Ordering::SeqCst) && FORWARDED.load(Ordering::SeqCst));
}

/// The template family enabled or disabled its destructor patch; outside attach and detach
/// only. It disables before its patches go and enables after they are in place.
pub fn forwarder(enabled: bool) {
    FORWARDED.store(enabled, Ordering::SeqCst);
    observe();
}

/// Hook path, allocation-free: the forwarded destructor is about to free `group`. Hold the
/// result over the native call.
pub fn forwarded(group: usize) -> Notification<'static> {
    WATCHES.destroy(group)
}

// Only a failed attach/detach allocates this diagnostic, retained for the runtime's static
// failure interface.
fn hook_failure(site: Option<usize>, action: &str, detail: &str) -> &'static str {
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
    Box::leak(
        format!("orders_group_changes [{sites}] hook {action} failed: {detail}").into_boxed_str(),
    )
}

fn validate_sites(base: usize) -> Result<(), &'static str> {
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
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
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
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            PATCHED.store(false, Ordering::SeqCst);
            observe();
        }
        // One census and suspension for the whole family, shared with the other families in
        // a runtime round (`hook::shared`).
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

/// Whether `group` trains to its maximum, the only state the daily update clears; an
/// unreadable byte counts as set.
fn trains_to_maximum(group: usize) -> bool {
    // SAFETY: guarded reads on the native calling thread.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    [
        engine_native::corders_group::TRAINING,
        engine_native::corders_group::STOP_AT_MAXIMUM,
    ]
    .into_iter()
    .all(|offset| {
        group
            .checked_add(offset)
            .and_then(|at| engine_native::read::<u8>(&memory, at))
            != Some(0)
    })
}

// Every site takes at most three integer or pointer arguments; the detour forwards RCX..R9
// and returns RAX. Only a normal native return ends the notification: an unwind escaping
// the original keeps it running.
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
            let notification = ManuallyDrop::new(if $site == DAILY_UPDATE {
                WATCHES.daily(group, trains_to_maximum(group))
            } else {
                Guard::Notified(WATCHES.notify(group, $site))
            });
            let original: NativeFunction =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(first, second, third, fourth) });
            drop(ManuallyDrop::into_inner(notification));
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

    /// Each writer moves only the group it names, the forwarded destructor too; a daily
    /// update moves a group only while it trains to its maximum; a running writer
    /// certifies nothing.
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
        // The skipped update still runs native code, so detach must wait for it.
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
        // An invalidating hour certifies nothing held before it.
        let before = held(0x1000);
        watches.boundary(true);
        assert_ne!(held(0x1000), before);
    }

    /// The domain observes only while this family's patches and the forwarder are enabled.
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

    /// A group the daily update cannot write is read as training to its maximum only when
    /// both bytes are set or unreadable.
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
