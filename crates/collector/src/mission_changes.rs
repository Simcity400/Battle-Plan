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
/// Reused and read countries; reused and read rows; quiet walks.
static READS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
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
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, air: usize) -> Option<(u64, u64)> {
        self.registry.sequence(air)
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
}

/// Notifications running now.
pub fn active() -> usize {
    WATCHES
        .registry
        .active()
        .saturating_add(crate::radar_changes::active())
        .saturating_add(crate::ground_crew_changes::active())
        .saturating_add(crate::air_detection_changes::active())
        .saturating_add(crate::naval_map_changes::active())
        .saturating_add(collector_observers::fleet_region_changes::active())
}

/// In each capture, before its readers. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
    collector_observers::fleet_region_changes::boundary(invalidate);
}

/// Off the game thread: size the watches for the country air objects the
/// latest walk visited. Nothing is offered before a walk visited them: a
/// later larger table waits for an invalidating hour, while the first
/// installs at once.
pub fn presize() {
    collector_observers::fleet_region_changes::presize();
    let countries = COUNTRIES.load(Ordering::Relaxed);
    if countries != 0 {
        WATCHES.registry.offer(countries);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
    crate::radar_changes::collect();
    crate::ground_crew_changes::collect();
    crate::naval_map_changes::collect();
    collector_observers::fleet_region_changes::collect();
}

/// One walk's countries and registrations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub reused: u64,
    pub read: u64,
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

static PUBLICATION_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Attach/detach failures only; retains the family's site and the original reason.
pub(crate) fn hook_failure(
    family: &str,
    action: &str,
    sites: &str,
    reason: impl std::fmt::Display,
) -> &'static str {
    let message = format!("{family} {action} [{sites}]: {reason}");
    *PUBLICATION_ERROR
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message.clone());
    Box::leak(message.into_boxed_str())
}


pub struct Hooks {
    hooks: Vec<Hook>,
    radar: crate::radar_changes::Hooks,
    ground_crews: crate::ground_crew_changes::Hooks,
    detection: crate::air_detection_changes::Hooks,
    naval_map: crate::naval_map_changes::Hooks,
    fleet_regions: collector_observers::fleet_region_changes::Hooks,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A withheld site leaves the family uninstalled; AirRegionalMissions
    /// then reads every country afresh.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        let mut located = [(0, [0; 16]); SITES];
        if cfg!(feature = "mission_changes")
            && MUTATIONS.iter().zip(&mut located).all(|(site, slot)| {
                crate::located_hook(base, site.rva)
                    .map(|code| *slot = code)
                    .is_ok()
            })
        {
            let callbacks: [usize; SITES] = [
                register_mission as *const () as usize,
                unregister_mission as *const () as usize,
            ];
            for (index, &(at, original)) in located.iter().enumerate() {
                let hook = unsafe { Hook::new(at, callbacks[index], original) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self {
            hooks,
            radar: unsafe { crate::radar_changes::Hooks::new(memory, base) }?,
            ground_crews: unsafe { crate::ground_crew_changes::Hooks::new(memory, base) }?,
            detection: unsafe { crate::air_detection_changes::Hooks::new(memory, base) }?,
            naval_map: unsafe { crate::naval_map_changes::Hooks::new(memory, base) }?,
            fleet_regions: if cfg!(feature = "mission_changes") {
                unsafe { collector_observers::fleet_region_changes::Hooks::new(memory, base) }?
            } else {
                collector_observers::fleet_region_changes::Hooks::disabled()
            },
        })
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
        if !self.radar.set_enabled(enabled)? {
            return Ok(false);
        }
        if !self.ground_crews.set_enabled(enabled)? {
            return Ok(false);
        }
        if !self.detection.set_enabled(enabled)? {
            return Ok(false);
        }
        if !self.naval_map.set_enabled(enabled)? {
            return Ok(false);
        }
        self.fleet_regions.set_enabled(enabled)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.radar.executable_ranges())
            .chain(self.ground_crews.executable_ranges())
            .chain(self.detection.executable_ranges())
            .chain(self.naval_map.executable_ranges())
            .chain(self.fleet_regions.executable_ranges())
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

    #[test]
    fn map_hook_errors_name_the_family_site_and_original_reason() {
        let message = hook_failure(
            "ground_crews",
            "construction failed",
            "enable",
            "synthetic verification refusal",
        );
        assert!(message.contains("ground_crews"));
        assert!(message.contains("enable"));
        assert!(message.contains("synthetic verification refusal"));
    }

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
