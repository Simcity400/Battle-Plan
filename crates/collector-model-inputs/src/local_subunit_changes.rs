//! Allocation-free notifications from the native writers of each country's
//! local sub-unit clones and their requirements
//! ([`engine_mutations::local_subunit_mutations`]). A detour only notifies the shared
//! watch registry ([`WatchRegistry`]) for the `CDeploymentStatus` its
//! arguments name, counts its site and calls the original; it never changes
//! game behaviour. Receivers are the deployment statuses a CountryLocalSubunits
//! walk registered. Anything unobserved (disabled, poisoned, a refused
//! registration, an epoch change, a running writer) makes the walk read that
//! country afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{
    local_subunit_mutations::BONUS_LIST, local_subunit_mutations::CLONE_COUNT,
    local_subunit_mutations::MUTATIONS, local_subunit_mutations::Receiver,
};
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused and read countries; reused and read rows; quiet walks.
static READS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// Deployment statuses the latest complete walk visited, for off-thread
/// sizing.
static COUNTRIES: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched deployment statuses and each writer
/// site's calls.
pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
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
        }
    }

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// is about to change `status`'s clones; `None` names a shared dependency
    /// and revokes every certificate. Dropping the guard notifies its return.
    pub fn notify(&self, status: Option<usize>, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        match status {
            Some(status) => self.registry.notify(status),
            None => self.registry.notify_every(),
        }
    }

    /// An enabled domain with a table for `objects` receivers installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(objects: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(objects);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

/// What a walk sees of the notifications.
pub trait Changes: Sync {
    /// `(epoch, revision)` of the whole domain while it is observed and no
    /// writer runs; `None` otherwise. Equal values across hours prove that no
    /// observed writer ran in between, so every certificate taken after the
    /// earlier value still holds without looking it up.
    fn quiet(&self) -> Option<(u64, u64)>;
    /// `(epoch, sequence)` of one deployment status, registering it; `None`
    /// when it cannot be observed. Equal values across hours prove that no
    /// observed writer touched it.
    fn sequence(&self, status: usize) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, status: usize) -> Option<(u64, u64)> {
        self.registry.sequence(status)
    }
}

/// The hooks' domain.
pub struct Observer;
/// A real observer without offered storage, isolated by its caller's test process.
#[cfg(any(test, feature = "test-support"))]
pub fn unsized_observer_for_tests() -> Observer {
    WATCHES.registry.set_enabled(true);
    Observer
}
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, status: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(status)
    }
}

/// Whether CountryLocalSubunits is carried between hours.
pub const CARRY: bool = cfg!(feature = "local_subunit_changes");

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.registry.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

/// Off-thread live sizing for the deployment statuses, before any capture
/// has visited them. The host's existing country census supplies demand.
pub fn prepare(countries: usize) {
    if countries != 0 {
        WATCHES.registry.offer(countries);
    }
}

/// Off-thread growth from capture demand. The existing boundary installs
/// offered storage; unchanged demand allocates nothing.
pub fn presize() {
    prepare(COUNTRIES.load(Ordering::Relaxed));
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One walk's countries and rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
    /// The walk's certificates held by the domain's unchanged quiet token,
    /// without a per-country lookup.
    pub quiet: bool,
}

/// A worker publishes one walk's counts and, when it visited every country,
/// their number for sizing.
pub fn record(counts: &Counts, countries: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.reused_rows,
        counts.read_rows,
        u64::from(counts.quiet),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(countries) = countries {
        COUNTRIES.store(countries, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&WATCHES.calls)
            .map(|((site, _), n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "local_subunit_changes"),
        "poisoned": WATCHES.registry.poisoned(),
        "reused_countries": reads[0],
        "read_countries": reads[1],
        "reused_rows": reads[2],
        "read_rows": reads[3],
        "quiet_hours": reads[4],
        "events": events,
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; CountryLocalSubunits is walked every hour: a country's rows are carried when its country object is that of the held walk and either the domain's quiet token is unchanged since before that walk (quiet_hours; no deployment status is read) or no hooked writer touched its deployment status since the held walk registered it and the status and clone vector header read this hour are the held ones; every other country is read afresh; a walk is held only when no hooked writer ran during it"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; CountryLocalSubunits
    /// then reads every country afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "local_subunit_changes")
            && MUTATIONS.iter().all(|(site, _)| {
                crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                ensure_local_subunit as *const () as usize,
                rebuild_local_subunit as *const () as usize,
                clear_local_subunits as *const () as usize,
                initialize_default_subunits as *const () as usize,
                find_or_create_subunit_definition as *const () as usize,
                reload_equipment_content as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            WATCHES.registry.set_enabled(false);
            collector_certificates::content_changes::enabled(
                collector_certificates::content_changes::Writer::Equipment,
                false,
            );
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        WATCHES
            .registry
            .set_enabled(enabled && !self.hooks.is_empty());
        collector_certificates::content_changes::enabled(
            collector_certificates::content_changes::Writer::Equipment,
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
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

/// The deployment status a site's arguments name; `None` for a shared
/// dependency.
fn receiver(receiver: Receiver, first: usize, third: usize) -> Option<usize> {
    match receiver {
        Receiver::Status => Some(first),
        Receiver::BonusList => Some(third.wrapping_sub(BONUS_LIST)),
        Receiver::Unnamed => None,
    }
}

// Each site takes at most four integer or pointer arguments (RCX..R9) and
// returns RAX unchanged from its original; the detour forwards all four. The
// notification covers the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::LocalSubunitChanges);
            let _notification = WATCHES.notify(receiver(MUTATIONS[$site].1, first, third), $site);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe {
                if $site == 5 {
                    collector_certificates::content_changes::invoke(
                        collector_certificates::content_changes::Writer::Equipment,
                        [first, second, third, fourth],
                        original,
                    )
                } else {
                    original(first, second, third, fourth)
                }
            })
        }
    };
}
/// A live status's clone count; the receiver is the status the native writer
/// is about to read and write.
fn clone_count(status: usize) -> i32 {
    unsafe { std::ptr::read_volatile(status.wrapping_add(CLONE_COUNT) as *const i32) }
}

// Ensure local subunit (status, global, flag).
// AddArmy calls it for every new division, and for a token already cloned
// it writes nothing a row records; so it runs under a guard that only keeps
// captures from certifying, and notifies its status when the clone count
// Moved  (an append, whose rebuild notified it too).
unsafe extern "system" fn ensure_local_subunit(
    status: usize,
    global: usize,
    flag: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::LocalSubunitChanges);
    WATCHES.calls[0].fetch_add(1, Ordering::Relaxed);
    let _running = WATCHES.registry.running();
    let before = clone_count(status);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(status, global, flag, fourth) });
    if clone_count(status) != before {
        drop(WATCHES.registry.notify(status));
    }
    result
}
// Local sub-unit definition rebuild (global, the country's id, the status's bonus list, clone).
callback!(rebuild_local_subunit, 1);
// Clear local subunits (status).
callback!(clear_local_subunits, 2);
// Initialize default subunits (status).
callback!(initialize_default_subunits, 3);
// Find or create subunit definition (database, reader): a global
// definition's requirements.
callback!(find_or_create_subunit_definition, 4);
// The equipment content reload callback (context): the archetypes.
callback!(reload_equipment_content, 5);

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer moves only the status it names (the rebuild names it through its bonus list in the
    /// status), and every writer moves the quiet token; a shared definition writer revokes every
    /// certificate.
    #[test]
    fn a_writer_moves_only_its_statuss_certificate_and_the_quiet_token() {
        let watches = Watches::enabled(16);
        let [first, second] =
            [0x1000, 0x2000].map(|status| Changes::sequence(&watches, status).unwrap());
        let quiet = Changes::quiet(&watches).unwrap();
        let named = receiver(MUTATIONS[1].1, 0x9000, 0x2000 + BONUS_LIST);
        assert_eq!(named, Some(0x2000));
        drop(watches.notify(named, 1));
        assert_eq!(Changes::sequence(&watches, 0x1000), Some(first));
        assert_ne!(Changes::sequence(&watches, 0x2000), Some(second));
        assert_ne!(Changes::quiet(&watches), Some(quiet));
        assert_eq!(watches.calls[1].load(Ordering::Relaxed), 1);
        for site in [0, 2, 3] {
            assert_eq!(receiver(MUTATIONS[site].1, 0x1000, 0), Some(0x1000));
        }
        // A running writer certifies nothing.
        let running = watches.notify(Some(0x1000), 0);
        assert!(Changes::sequence(&watches, 0x1000).is_none());
        assert!(Changes::quiet(&watches).is_none());
        drop(running);
        // A global definition reload names no country.
        let first = Changes::sequence(&watches, 0x1000).unwrap();
        assert_eq!(receiver(MUTATIONS[4].1, 0x1000, 0), None);
        drop(watches.notify(receiver(MUTATIONS[4].1, 0x1000, 0), 4));
        assert_ne!(Changes::sequence(&watches, 0x1000), Some(first));
        assert!(!watches.poisoned());
    }
}
