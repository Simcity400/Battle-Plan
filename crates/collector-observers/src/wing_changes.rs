use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::air_wing_equipment_mutations::{
    ATTACHMENT_COVERAGE, MAP_MUTATIONS, MISSION, MUTATIONS, TRAINING_STOP, pools,
};
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static MAP_ORIGINAL: [AtomicUsize; MAP_MUTATIONS.len()] =
    [const { AtomicUsize::new(0) }; MAP_MUTATIONS.len()];
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static WINGS: AtomicUsize = AtomicUsize::new(0);

pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    kills: AtomicU64,
    hooks: AtomicBool,
    kills_observed: AtomicBool,
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
            kills: AtomicU64::new(0),
            hooks: AtomicBool::new(false),
            kills_observed: AtomicBool::new(false),
        }
    }

    pub fn notify(&self, pool: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(pool)
    }

    pub fn kill(&self, wing: usize) -> Notification<'_> {
        self.kills.fetch_add(1, Ordering::Relaxed);
        self.registry
            .notify(wing.wrapping_add(engine_mutations::air_wing_equipment_mutations::POOL))
    }

    fn refresh(&self) {
        self.registry.set_enabled(
            self.hooks.load(Ordering::SeqCst) && self.kills_observed.load(Ordering::SeqCst),
        );
    }

    fn set_hooks(&self, enabled: bool) {
        self.hooks.store(enabled, Ordering::SeqCst);
        self.refresh();
    }

    fn set_kills_observed(&self, observed: bool) {
        self.kills_observed.store(observed, Ordering::SeqCst);
        self.refresh();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_kills(pools: usize, kills_observed: bool) -> Self {
        let watches = Self::new();
        watches.registry.offer(pools);
        watches.registry.boundary(false);
        watches.set_hooks(true);
        watches.set_kills_observed(kills_observed);
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(pools: usize) -> Self {
        Self::with_kills(pools, true)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn observe_kills(&self, observed: bool) {
        self.set_kills_observed(observed);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

pub trait Changes: Sync {
    fn sequence(&self, pool: usize) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn sequence(&self, pool: usize) -> Option<(u64, u64)> {
        self.registry.sequence(pool)
    }
}

pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, pool: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(pool)
    }
}

pub fn map_sequence(address: usize) -> Option<(u64, u64)> {
    WATCHES.registry.sequence(address)
}
pub fn map_quiet() -> Option<(u64, u64)> {
    WATCHES.registry.quiet()
}

pub fn active() -> usize {
    WATCHES.registry.active() + crate::wing_preferences::active()
}

pub fn kill_airplanes(wing: usize) -> Option<Notification<'static>> {
    cfg!(feature = "wing_changes").then(|| WATCHES.kill(wing))
}

pub fn observe_kills(observed: bool) {
    crate::crew_changes::observe_kills(observed);
    WATCHES.set_kills_observed(observed);
}

pub fn boundary(invalidate: bool) {
    if invalidate {
        crate::wing_preferences::invalidate();
    }
    WATCHES.registry.boundary(invalidate);
}

pub fn presize() {
    let wings = WINGS.load(Ordering::Relaxed);
    if wings != 0 {
        WATCHES.registry.offer(wings.saturating_mul(6));
    }
}

pub fn collect() {
    WATCHES.registry.collect();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
}

impl Counts {
    pub fn add(&mut self, other: &Counts) {
        self.reused += other.reused;
        self.read += other.read;
        self.reused_rows += other.reused_rows;
        self.read_rows += other.read_rows;
    }
}

pub fn record(counts: &Counts, wings: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.reused_rows,
        counts.read_rows,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(wings) = wings {
        WINGS.store(wings, Ordering::Relaxed);
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
    preferences: crate::wing_preferences::Hooks,
}

pub(crate) fn publish_preference_lifetime(enabled: bool, installed: usize) {
    crate::wing_preferences::lifetime_enabled(enabled && installed == SITES + MAP_MUTATIONS.len());
}

impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; WingEquipment then
    /// reads every wing afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        use engine_mutations::{
            air_wing_equipment_mutations::Receiver,
            air_wing_equipment_mutations::SUBTRACT_VARIANT_RVA,
        };
        let mut hooks = Vec::new();
        if cfg!(feature = "wing_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _)| crate::located(base, site.rva).is_ok())
            && let Ok(subtract) = engine_sites::hook(base, SUBTRACT_VARIANT_RVA)
        {
            for site in MAP_MUTATIONS {
                crate::verified(base, site.rva)
                    .map_err(|reason| map_hook_error(site.name, reason))?;
            }
            SUBTRACT.store(subtract.address, Ordering::Release);
            let callbacks: [usize; SITES] = [
                add_airplanes as *const () as usize,
                add_equipment_pool as *const () as usize,
                move_airplanes as *const () as usize,
                subtract_equipment_pool as *const () as usize,
                trim_airplanes as *const () as usize,
                return_airplanes as *const () as usize,
                destroy_wing as *const () as usize,
            ];
            for (index, (site, receiver)) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let original = code.sentinel::<16>()?;
                let hook = match receiver {
                    Receiver::Pool => unsafe {
                        Hook::new_call(code.address, callbacks[index], original)
                    },
                    Receiver::Wing | Receiver::Wings => unsafe {
                        Hook::new(code.address, callbacks[index], original)
                    },
                }
                .map_err(|_| "native hook construction failed")?;
                if *receiver != Receiver::Pool {
                    ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                }
                hooks.push(hook);
            }
        }
        if !hooks.is_empty() {
            let callbacks = [
                training_stop as *const () as usize,
                attachment_coverage as *const () as usize,
                raid_reference as *const () as usize,
                raid_lifetime as *const () as usize,
                append_incoming as *const () as usize,
                erase_incoming as *const () as usize,
            ];
            for (index, site) in MAP_MUTATIONS.into_iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|reason| map_hook_error(site.name, &reason))?;
                MAP_ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self {
            hooks,
            preferences: unsafe { crate::wing_preferences::Hooks::new(base) }?,
        })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            WATCHES.set_hooks(false);
            crate::wing_preferences::lifetime_enabled(false);
        }
        let preferences = self.preferences.set_enabled(enabled)?;
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|reason| map_hook_error("equipment and map flag sites", &reason))?
        {
            return Ok(preferences);
        }
        WATCHES.set_hooks(enabled && !self.hooks.is_empty());
        publish_preference_lifetime(enabled, self.hooks.len());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.preferences.executable_ranges())
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        crate::wing_preferences::lifetime_enabled(false);
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

#[doc = " Variant quantity subtraction, which the replaced CALLs invoke."]
static SUBTRACT: AtomicUsize = AtomicUsize::new(0);

fn notify(site: usize, first: usize, second: usize) -> [Option<Notification<'static>>; 5] {
    let [pool, other] = pools(MUTATIONS[site].1, first, second);
    [
        Some(WATCHES.notify(pool, site)),
        (other != 0).then(|| WATCHES.notify(other, site)),
        (site == 6).then(|| WATCHES.registry.notify(first.wrapping_add(TRAINING_STOP))),
        (site == 6).then(|| {
            WATCHES
                .registry
                .notify(first.wrapping_add(ATTACHMENT_COVERAGE))
        }),
        (site == 6).then(|| {
            WATCHES.registry.notify(
                first.wrapping_add(engine_mutations::air_wing_equipment_mutations::RAID_REFERENCE),
            )
        }),
    ]
}

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system-unwind" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::WingChanges);
            let _notifications = notify($site, first, second);
            let preference = ($site == 6)
                .then(|| crate::wing_preferences::destruction(first))
                .flatten();
            let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(first, second, third, fourth) });
            if let Some(preference) = preference {
                preference.finish(result);
            }
            result
        }
    };
}
callback!(add_airplanes, 0);
callback!(add_equipment_pool, 1);
callback!(move_airplanes, 2);
callback!(subtract_equipment_pool, 3);
callback!(destroy_wing, 6);

macro_rules! call_site {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            pool: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::WingChanges);
            let subtract: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(SUBTRACT.load(Ordering::Acquire)) };
            let _notifications = notify($site, pool, second);
            timing.original(|| unsafe { subtract(pool, second, third, fourth) })
        }
    };
}
call_site!(trim_airplanes, 4);
call_site!(return_airplanes, 5);

fn map_hook_error(site: &str, reason: &str) -> String {
    format!("wing_changes {site}: {reason}")
}

macro_rules! map_writer {
    ($name:ident, $index:expr, $key:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::WingChanges);
            let key: fn(usize, usize) -> usize = $key;
            let _notification = WATCHES.registry.notify(key(first, third));
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(MAP_ORIGINAL[$index].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
map_writer!(training_stop, 0, |mission, _| mission
    .wrapping_sub(MISSION)
    .wrapping_add(TRAINING_STOP));
map_writer!(attachment_coverage, 1, |_, wing| wing
    .wrapping_add(ATTACHMENT_COVERAGE));
map_writer!(raid_reference, 2, |wing, _| wing.wrapping_add(
    engine_mutations::air_wing_equipment_mutations::RAID_REFERENCE
));
map_writer!(raid_lifetime, 3, |raid, _| raid.wrapping_add(
    engine_mutations::air_wing_equipment_mutations::RAID_LIFETIME_KEY
));
map_writer!(append_incoming, 4, |base, _| base
    .wrapping_add(engine_native_air::cair_base::INCOMING_WINGS));
map_writer!(erase_incoming, 5, |base, _| base
    .wrapping_add(engine_native_air::cair_base::INCOMING_WINGS));

#[cfg(test)]
mod map_flag_tests {
    use super::*;
    #[test]
    fn incoming_writers_have_a_disjoint_receiver_from_equipment_and_flags() {
        let watches = Watches::enabled(4);
        let base = 0x1000usize;
        let pool = base + engine_mutations::air_wing_equipment_mutations::POOL;
        let key = base + engine_native_air::cair_base::INCOMING_WINGS;
        let equipment = watches.sequence(pool);
        let incoming = watches.registry.sequence(key);
        drop(watches.registry.notify(key));
        assert_eq!(watches.sequence(pool), equipment);
        assert_ne!(watches.registry.sequence(key), incoming);
    }
    #[test]
    fn flag_writes_preserve_equipment_observations() {
        let watches = Watches::enabled(30_000);
        for wing in (1..=5000).map(|id| id * 0x1000) {
            let pool = wing + engine_mutations::air_wing_equipment_mutations::POOL;
            let equipment = watches.sequence(pool);
            let stop = watches.registry.sequence(wing + TRAINING_STOP);
            let coverage = watches.registry.sequence(wing + ATTACHMENT_COVERAGE);
            let raid_key = wing + engine_mutations::air_wing_equipment_mutations::RAID_REFERENCE;
            let raid = watches.registry.sequence(raid_key);
            let lifetime_key =
                wing + engine_mutations::air_wing_equipment_mutations::RAID_LIFETIME_KEY;
            assert!(watches.registry.sequence(lifetime_key).is_some());
            let incoming_key = wing + engine_native_air::cair_base::INCOMING_WINGS;
            let incoming = watches.registry.sequence(incoming_key);
            drop(watches.registry.notify(wing + TRAINING_STOP));
            assert_eq!(watches.sequence(pool), equipment);
            assert_ne!(watches.registry.sequence(wing + TRAINING_STOP), stop);
            assert_eq!(
                watches.registry.sequence(wing + ATTACHMENT_COVERAGE),
                coverage
            );
            drop(watches.registry.notify(wing + ATTACHMENT_COVERAGE));
            assert_eq!(watches.sequence(pool), equipment);
            assert_eq!(watches.registry.sequence(raid_key), raid);
            drop(watches.registry.notify(raid_key));
            assert_eq!(watches.sequence(pool), equipment);
            assert_ne!(watches.registry.sequence(raid_key), raid);
            drop(watches.registry.notify(incoming_key));
            assert!(incoming.is_some());
            assert_ne!(watches.registry.sequence(incoming_key), incoming);
            assert_eq!(watches.sequence(pool), equipment);
            assert_ne!(
                watches.registry.sequence(wing + ATTACHMENT_COVERAGE),
                coverage
            );
        }
    }
}
