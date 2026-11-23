use collector_certificates::watch_registry::{Notification, WatchRegistry};
use collector_hooks::hook::Hook;
use engine_mutations::regional_mission_mutations::MUTATIONS;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static COUNTRIES: AtomicUsize = AtomicUsize::new(0);

pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
        }
    }

    pub fn notify(&self, air: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(air)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(objects: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(objects);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
    }
}

pub trait Changes: Sync {
    fn quiet(&self) -> Option<(u64, u64)>;
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

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, air: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(air)
    }
}

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

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
    collector_observers::fleet_region_changes::boundary(invalidate);
}

/// # Safety
/// Run only during attach or the active session's serial sizing pass (or an isolated test), while
/// mission and fleet-region watches remain live and teardown is excluded.
pub unsafe fn presize() {
    // SAFETY: Caller guarantees attach or serial sizing pass with live fleet-region watches.
    unsafe { collector_observers::fleet_region_changes::presize() };
    let countries = COUNTRIES.load(Ordering::Relaxed);
    if countries != 0 {
        WATCHES.registry.offer(countries);
    }
}

pub fn collect() {
    WATCHES.registry.collect();
    crate::radar_changes::collect();
    crate::ground_crew_changes::collect();
    crate::naval_map_changes::collect();
    collector_observers::fleet_region_changes::collect();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
    pub quiet: bool,
}

/// # Safety
/// Run only during admitted air-mission capture (or an isolated test) with measured country demand
/// for the active session.
pub unsafe fn record(counts: &Counts, countries: Option<usize>) {
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

pub fn hook_failure(
    family: &str,
    action: &str,
    sites: &str,
    reason: impl std::fmt::Display,
) -> String {
    let message = format!("{family} {action} [{sites}]: {reason}");
    *PUBLICATION_ERROR
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message.clone());
    message
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
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, String> {
        let mut hooks = Vec::new();
        let mut located = [(0, [0; 16]); SITES];
        if cfg!(feature = "mission_changes")
            && MUTATIONS.iter().zip(&mut located).all(|(site, slot)| {
                collector_hooks::located_hook(base, site.rva)
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
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            WATCHES.registry.set_enabled(false);
        }
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

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            air: usize,
            mission: usize,
            region: usize,
            fourth: usize,
        ) -> usize {
            let timing = collector_timing::hook_timing::enter(
                collector_timing::hook_timing::Family::MissionChanges,
            );
            let _notification = WATCHES.notify(air, $site);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(air, mission, region, fourth) })
        }
    };
}
callback!(register_mission, 0);
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
        let running = watches.notify(0x1000, 0);
        assert!(Changes::sequence(&watches, 0x1000).is_none());
        assert!(Changes::quiet(&watches).is_none());
        drop(running);
    }
}
