//! Allocation-free notifications from the native writers of every air wing's equipment pool
//! ([`engine_mutations::air_wing_equipment_mutations`]). A detour only notifies the shared watch registry
//! ([`WatchRegistry`]) for the pools its arguments name, counts its site and calls the original; it
//! never changes game behaviour. Receivers are the `CEquipmentVariantPool` addresses
//! (`cair_wing::EQUIPMENT_POOL`) a WingEquipment capture registered; a notification never infers a
//! wing. Anything unobserved (disabled, poisoned, a refused registration, an epoch change, a
//! running writer) makes the capture read that wing afresh.
//!
//! aircraft loss writer is patched by the equipment-destruction family
//! (one patch per site); its `air` detour notifies this domain on every path
//! ([`kill_airplanes`]). The domain observes only while both its own hooks
//! and that detour are live ([`observe_kills`]): with the destruction family
//! uninstalled or disabled, losses go unobserved, so nothing is certified.
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
/// Reused and read wings; reused and read rows.
static READS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
/// Wings the latest complete capture walked, for off-thread sizing.
static WINGS: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched pools, each writer site's calls and
/// the losses the equipment-destruction detour reported.
pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    kills: AtomicU64,
    /// This family's hooks are enabled.
    hooks: AtomicBool,
    /// The equipment-destruction family's aircraft loss writer detour is live.
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

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// is about to change `pool`. Dropping the guard notifies its return.
    pub fn notify(&self, pool: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(pool)
    }

    /// Hook path, allocation-free: `aircraft loss writer` is about to change
    /// `wing`'s pool. Dropping the guard notifies its return.
    pub fn kill(&self, wing: usize) -> Notification<'_> {
        self.kills.fetch_add(1, Ordering::Relaxed);
        self.registry
            .notify(wing.wrapping_add(engine_mutations::air_wing_equipment_mutations::POOL))
    }

    /// Observes only while both this family's hooks and the aircraft loss writer
    /// detour are live; enabling starts an epoch, so nothing written while
    /// either was down certifies anything.
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

    /// A domain with a table for `pools` receivers installed, its hooks
    /// enabled and the aircraft loss writer detour live when `kills_observed`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_kills(pools: usize, kills_observed: bool) -> Self {
        let watches = Self::new();
        watches.registry.offer(pools);
        watches.registry.boundary(false);
        watches.set_hooks(true);
        watches.set_kills_observed(kills_observed);
        watches
    }

    /// An enabled domain with a table for `pools` receivers installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(pools: usize) -> Self {
        Self::with_kills(pools, true)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn observe_kills(&self, observed: bool) {
        self.set_kills_observed(observed);
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// `(epoch, sequence)` of one pool, registering it; `None` when it cannot
    /// be observed. Equal values across hours prove that no observed writer
    /// touched it.
    fn sequence(&self, pool: usize) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn sequence(&self, pool: usize) -> Option<(u64, u64)> {
        self.registry.sequence(pool)
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, pool: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(pool)
    }
}

/// A flag receiver is its inline address, distinct from any equipment pool.
/// These notifications never invalidate WingEquipment certificates.
pub fn map_sequence(address: usize) -> Option<(u64, u64)> {
    WATCHES.registry.sequence(address)
}
/// Guards initial reference resolution before its lifetime receiver is registered.
pub fn map_quiet() -> Option<(u64, u64)> {
    WATCHES.registry.quiet()
}

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.registry.active() + crate::wing_preferences::active()
}

/// The equipment-destruction family's `air` detour, on every path before
/// the original `aircraft loss writer(wing, ...)` runs: the guard covers it.
pub fn kill_airplanes(wing: usize) -> Option<Notification<'static>> {
    cfg!(feature = "wing_changes").then(|| WATCHES.kill(wing))
}

/// The equipment-destruction family reports whether its aircraft loss writer
/// detour is live: after enabling its writers with that site installed, and
/// before disabling them.
pub fn observe_kills(observed: bool) {
    crate::crew_changes::observe_kills(observed);
    WATCHES.set_kills_observed(observed);
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    if invalidate {
        crate::wing_preferences::invalidate();
    }
    WATCHES.registry.boundary(invalidate);
}

/// Off the game thread: size the watches for the wings the latest capture
/// walked. Nothing is offered before a capture walked them: a later larger
/// table waits for an invalidating hour, while the first installs at once.
pub fn presize() {
    let wings = WINGS.load(Ordering::Relaxed);
    if wings != 0 {
        // Pool, stop, coverage, raid reference, a distinct live raid lifetime,
        // and at most one distinct incoming destination per wing.
        WATCHES.registry.offer(wings.saturating_mul(6));
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One capture's wings and rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
}

impl Counts {
    /// Adds another part's counts of the same capture.
    pub fn add(&mut self, other: &Counts) {
        self.reused += other.reused;
        self.read += other.read;
        self.reused_rows += other.reused_rows;
        self.read_rows += other.read_rows;
    }
}

/// One capture publishes its counts, every part's together, and, when it
/// walked every wing, their number for sizing.
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

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&WATCHES.calls)
            .map(|((site, _), n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "wing_changes"),
        "poisoned": WATCHES.registry.poisoned(),
        "reused_wings": reads[0],
        "read_wings": reads[1],
        "reused_rows": reads[2],
        "read_rows": reads[3],
        "events": events,
        "replacement_preference": crate::wing_preferences::status(),
        "kill_airplanes": WATCHES.kills.load(Ordering::Relaxed),
        "kills_observed": WATCHES.kills_observed.load(Ordering::SeqCst),
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; a WingEquipment wing's rows are carried when it is the wing held at the same address with the same id and no hooked writer touched its equipment pool since its read; the wing census is read every hour"
    })
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
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        use engine_mutations::{
            air_wing_equipment_mutations::Receiver,
            air_wing_equipment_mutations::SUBTRACT_VARIANT_RVA,
        };
        let mut hooks = Vec::new();
        if cfg!(feature = "wing_changes")
            && MUTATIONS.iter().all(|(site, _)| {
                crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel)
            })
        {
            for site in MAP_MUTATIONS {
                if engine_native::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                    return Err(map_hook_error(site.name, "entry verification failed"));
                }
            }
            SUBTRACT.store(base + SUBTRACT_VARIANT_RVA, Ordering::Release);
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
                let hook = match receiver {
                    Receiver::Pool => unsafe {
                        Hook::new_call(base + site.rva, callbacks[index], site.sentinel)
                    },
                    Receiver::Wing | Receiver::Wings => unsafe {
                        Hook::new(base + site.rva, callbacks[index], site.sentinel)
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
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|reason| map_hook_error(site.name, &reason))?;
                MAP_ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self {
            hooks,
            preferences: unsafe { crate::wing_preferences::Hooks::new(memory, base) }?,
        })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            WATCHES.set_hooks(false);
            crate::wing_preferences::lifetime_enabled(false);
        }
        let preferences = self.preferences.set_enabled(enabled)?;
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
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

/// Variant quantity subtraction, which the replaced CALLs invoke.
static SUBTRACT: AtomicUsize = AtomicUsize::new(0);

/// Hook path: the notifications of a site's pools, covering the original
/// in flight.
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

// Each entry site takes at most four integer or pointer arguments (RCX..R9)
// and returns RAX unchanged from its original; the first two name the pools
// as its [`engine_mutations::air_wing_equipment_mutations::Receiver`] says.
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
// Add airplanes (wing, variant, count, ...).
callback!(add_airplanes, 0);
// Add equipment pool (wing, pool, ...).
callback!(add_equipment_pool, 1);
// Move airplanes (source, destination, count).
callback!(move_airplanes, 2);
// Subtract equipment pool (wing, pool).
callback!(subtract_equipment_pool, 3);
// Destroy wing (wing).
callback!(destroy_wing, 6);

// A replaced CALL of SubtractVariantQuantity(pool, variant, amount): the
// detour calls it itself, which returns to the site's next instruction
// through the relay.
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
// The over-capacity aircraft trim subtraction call.
call_site!(trim_airplanes, 4);
// The daily aircraft return subtraction call.
call_site!(return_airplanes, 5);

fn map_hook_error(site: &str, reason: &str) -> &'static str {
    Box::leak(format!("wing_changes {site}: {reason}").into_boxed_str())
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
    .wrapping_add(engine_native::cair_base::INCOMING_WINGS));
map_writer!(erase_incoming, 5, |base, _| base
    .wrapping_add(engine_native::cair_base::INCOMING_WINGS));

#[cfg(test)]
mod map_flag_tests {
    use super::*;
    #[test]
    fn incoming_writers_have_a_disjoint_receiver_from_equipment_and_flags() {
        let watches = Watches::enabled(4);
        let base = 0x1000usize;
        let pool = base + engine_mutations::air_wing_equipment_mutations::POOL;
        let key = base + engine_native::cair_base::INCOMING_WINGS;
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
            let incoming_key = wing + engine_native::cair_base::INCOMING_WINGS;
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
