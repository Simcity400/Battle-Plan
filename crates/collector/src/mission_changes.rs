//! Allocation-free notifications from the native writers of every country
//! air object's registered missions ([`engine_mutations::regional_mission_mutations`]).
//! A detour only notifies the shared watch registry ([`WatchRegistry`]) for
//! the `CStrategicAir` its first argument names, counts its site and calls
//! the original; it never changes game behaviour. Both sites also run on the
//! strategic-air TBB worker threads, which the registry's atomics serve.
//! Receivers are the country air objects an AirRegionalMissions walk
//! registered. Anything unobserved (disabled, poisoned, a refused
//! registration, an epoch change, a running writer) makes the walk read that
//! object afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::regional_mission_mutations::MUTATIONS;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused, read, verified and mismatched countries; reused and read rows;
/// quiet walks.
static READS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried AirRegionalMissions registrations differ from a fresh read",
]);
/// Country air objects the latest complete walk visited, for off-thread
/// sizing.
static COUNTRIES: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched country air objects and each writer
/// site's calls.
pub(crate) struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
}

impl Watches {
    pub(crate) const fn new() -> Self {
        Self {
            registry: WatchRegistry::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
        }
    }

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// is about to change `air`'s registrations. Dropping the guard notifies
    /// its return.
    pub(crate) fn notify(&self, air: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(air)
    }

    /// An enabled domain with a table for `objects` receivers installed.
    #[cfg(test)]
    pub(crate) fn enabled(objects: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(objects);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(test)]
    pub(crate) fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
    }

    #[cfg(test)]
    pub(crate) fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

/// What a walk sees of the notifications.
pub(crate) trait Changes: Sync {
    /// `(epoch, revision)` of the whole domain while it is observed and no
    /// writer runs; `None` otherwise. Equal values across hours prove that no
    /// observed writer ran in between, so every certificate taken after the
    /// earlier value still holds without looking it up.
    fn quiet(&self) -> Option<(u64, u64)>;
    /// `(epoch, sequence)` of one country air object, registering it; `None`
    /// when it cannot be observed. Equal values across hours prove that no
    /// observed writer touched it.
    fn sequence(&self, air: usize) -> Option<(u64, u64)>;
    /// A fresh read disagreed with carried registrations: disable reuse for
    /// the attachment.
    fn mismatch(&self);
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, air: usize) -> Option<(u64, u64)> {
        self.registry.sequence(air)
    }
    fn mismatch(&self) {
        self.registry.poison();
    }
}

/// The hooks' domain.
pub(crate) struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, air: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(air)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
}

/// Whether walks read every carryable country afresh and compare, rather
/// than the hour's sampled slice ([`crate::sampled_verify`]).
pub(crate) const VERIFY: bool = cfg!(feature = "mission_verify");

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

/// Off the game thread: size the watches for the country air objects the
/// latest walk visited. Nothing is offered before a walk visited them: a
/// later larger table waits for an invalidating hour, while the first
/// installs at once.
pub fn presize() {
    let countries = COUNTRIES.load(Ordering::Relaxed);
    if countries != 0 {
        WATCHES.registry.offer(countries);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One walk's countries and registrations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
    /// The walk's certificates held by the domain's unchanged quiet token,
    /// without a per-country lookup.
    pub quiet: bool,
}

/// A worker publishes one walk's counts and, when it visited every country
/// air object, their number for sizing.
pub(crate) fn record(counts: &Counts, countries: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        counts.reused_rows,
        counts.read_rows,
        u64::from(counts.quiet),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
    if let Some(countries) = countries {
        COUNTRIES.store(countries, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&WATCHES.calls)
            .map(|(site, n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "mission_changes"),
        "verify": VERIFY,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_countries": reads[0],
        "read_countries": reads[1],
        "verified_countries": reads[2],
        "mismatches": reads[3],
        "reused_rows": reads[4],
        "read_rows": reads[5],
        "quiet_hours": reads[6],
        "events": events,
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; a country's AirRegionalMissions registrations are carried when its country air object has the address, canonical slot and gathered mission-vector header bytes of the held read and no hooked writer touched it since, and each carried registration's wing in this hour's census still names the object's country and the registration's region (otherwise the country is read afresh); every object's headers and the wing census are read every hour; every hour also reads the carryable countries of its slice afresh (one in rotation_hours by canonical slot; every one in verify mode), publishes the fresh rows and disables reuse at the first mismatch (sampled_*); quiet_hours counts walks whose certificates held by the domain's unchanged quiet token alone, without a per-country lookup"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; AirRegionalMissions
    /// then reads every country afresh.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "mission_changes")
            && MUTATIONS.iter().all(|site| {
                engine_native::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                register_mission as *const () as usize,
                unregister_mission as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
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

// Both sites take (CStrategicAir, CAirMission, region) in RCX, RDX and R8
// and return nothing a caller reads; the detour forwards RCX..R9 and RAX.
// The notification covers the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            air: usize,
            mission: usize,
            region: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::MissionChanges);
            let _notification = WATCHES.notify(air, $site);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(air, mission, region, fourth) })
        }
    };
}
// Register mission (air, mission, region).
callback!(register_mission, 0);
// Unregister mission (air, mission, region).
callback!(unregister_mission, 1);

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer moves only the object it names, and every writer moves the
    /// domain's quiet token.
    #[test]
    fn a_writer_moves_only_its_objects_certificate_and_the_quiet_token() {
        let watches = Watches::enabled(16);
        let [first, second] = [0x1000, 0x2000].map(|air| Changes::sequence(&watches, air).unwrap());
        let quiet = Changes::quiet(&watches).unwrap();
        drop(watches.notify(0x2000, 1));
        assert_eq!(Changes::sequence(&watches, 0x1000), Some(first));
        assert_ne!(Changes::sequence(&watches, 0x2000), Some(second));
        assert_ne!(Changes::quiet(&watches), Some(quiet));
        assert_eq!(watches.calls[1].load(Ordering::Relaxed), 1);
        // A running writer certifies nothing.
        let running = watches.notify(0x1000, 0);
        assert!(Changes::sequence(&watches, 0x1000).is_none());
        assert!(Changes::quiet(&watches).is_none());
        drop(running);
    }
}
