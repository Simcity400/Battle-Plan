//! Allocation-free notifications from the native writers of every air wing's
//! equipment pool ([`engine::air_wing_equipment_mutations`]). A detour only
//! notifies the shared watch registry ([`WatchRegistry`]) for the pools its
//! arguments name, counts its site and calls the original; it never changes
//! game behaviour. Receivers are the `CEquipmentVariantPool` addresses
//! (`CAirWing+0x1D8`) a WingEquipment capture registered; a notification
//! never infers a wing. Anything unobserved (disabled, poisoned, a refused
//! registration, an epoch change, a running writer) makes the capture read
//! that wing afresh.
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
use engine::{
    Memory,
    air_wing_equipment_mutations::{MUTATIONS, pools},
};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused, read, verified and mismatched wings; reused and read rows.
static READS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried WingEquipment rows differ from a fresh read",
]);
/// Wings the latest complete capture walked, for off-thread sizing.
static WINGS: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched pools, each writer site's calls and
/// the losses the equipment-destruction detour reported.
pub(crate) struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    kills: AtomicU64,
    /// This family's hooks are enabled.
    hooks: AtomicBool,
    /// The equipment-destruction family's aircraft loss writer detour is live.
    kills_observed: AtomicBool,
}

impl Watches {
    pub(crate) const fn new() -> Self {
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
    pub(crate) fn notify(&self, pool: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(pool)
    }

    /// Hook path, allocation-free: `aircraft loss writer` is about to change
    /// `wing`'s pool. Dropping the guard notifies its return.
    pub(crate) fn kill(&self, wing: usize) -> Notification<'_> {
        self.kills.fetch_add(1, Ordering::Relaxed);
        self.registry
            .notify(wing.wrapping_add(engine::air_wing_equipment_mutations::POOL))
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
    #[cfg(test)]
    pub(crate) fn with_kills(pools: usize, kills_observed: bool) -> Self {
        let watches = Self::new();
        watches.registry.offer(pools);
        watches.registry.boundary(false);
        watches.set_hooks(true);
        watches.set_kills_observed(kills_observed);
        watches
    }

    /// An enabled domain with a table for `pools` receivers installed.
    #[cfg(test)]
    pub(crate) fn enabled(pools: usize) -> Self {
        Self::with_kills(pools, true)
    }

    #[cfg(test)]
    pub(crate) fn observe_kills(&self, observed: bool) {
        self.set_kills_observed(observed);
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

/// What a capture sees of the notifications.
pub(crate) trait Changes: Sync {
    /// `(epoch, sequence)` of one pool, registering it; `None` when it cannot
    /// be observed. Equal values across hours prove that no observed writer
    /// touched it.
    fn sequence(&self, pool: usize) -> Option<(u64, u64)>;
    /// A fresh read disagreed with carried rows: disable reuse for the
    /// attachment.
    fn mismatch(&self);
}

impl Changes for Watches {
    fn sequence(&self, pool: usize) -> Option<(u64, u64)> {
        self.registry.sequence(pool)
    }
    fn mismatch(&self) {
        self.registry.poison();
    }
}

/// The hooks' domain.
pub(crate) struct Observer;
impl Changes for Observer {
    fn sequence(&self, pool: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(pool)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
}

/// Whether captures read every carryable wing afresh and compare, rather
/// than the hour's sampled slice ([`crate::sampled_verify`]).
pub(crate) const VERIFY: bool = cfg!(feature = "wing_verify");

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.registry.active()
}

/// The equipment-destruction family's `air` detour, on every path before
/// the original `aircraft loss writer(wing, ...)` runs: the guard covers it.
pub(crate) fn kill_airplanes(wing: usize) -> Option<Notification<'static>> {
    cfg!(feature = "wing_changes").then(|| WATCHES.kill(wing))
}

/// The equipment-destruction family reports whether its aircraft loss writer
/// detour is live: after enabling its writers with that site installed, and
/// before disabling them.
pub(crate) fn observe_kills(observed: bool) {
    WATCHES.set_kills_observed(observed);
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

/// Off the game thread: size the watches for the wings the latest capture
/// walked. Nothing is offered before a capture walked them: a later larger
/// table waits for an invalidating hour, while the first installs at once.
pub fn presize() {
    let wings = WINGS.load(Ordering::Relaxed);
    if wings != 0 {
        WATCHES.registry.offer(wings);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One capture's wings and rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
}

impl Counts {
    /// Adds another part's counts of the same capture.
    pub(crate) fn add(&mut self, other: &Counts) {
        self.reused += other.reused;
        self.read += other.read;
        self.verified += other.verified;
        self.mismatches += other.mismatches;
        self.reused_rows += other.reused_rows;
        self.read_rows += other.read_rows;
    }
}

/// One capture publishes its counts, every part's together, and, when it
/// walked every wing, their number for sizing.
pub(crate) fn record(counts: &Counts, wings: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        counts.reused_rows,
        counts.read_rows,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
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
        "verify": VERIFY,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_wings": reads[0],
        "read_wings": reads[1],
        "verified_wings": reads[2],
        "mismatches": reads[3],
        "reused_rows": reads[4],
        "read_rows": reads[5],
        "events": events,
        "kill_airplanes": WATCHES.kills.load(Ordering::Relaxed),
        "kills_observed": WATCHES.kills_observed.load(Ordering::SeqCst),
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; a WingEquipment wing's rows are carried when it is the wing held at the same address with the same id and no hooked writer touched its equipment pool since its read; the wing census is read every hour; every hour also reads the carryable wings of its slice afresh (one in rotation_hours by wing id; every one in verify mode), publishes the fresh rows and disables reuse at the first mismatch (sampled_*)"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; WingEquipment then
    /// reads every wing afresh.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        use engine::air_wing_equipment_mutations::{Receiver, SUBTRACT_VARIANT_RVA};
        let mut hooks = Vec::new();
        if cfg!(feature = "wing_changes")
            && MUTATIONS.iter().all(|(site, _)| {
                engine::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
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
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            WATCHES.set_hooks(false);
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
        WATCHES.set_hooks(enabled && !self.hooks.is_empty());
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

/// Variant quantity subtraction, which the replaced CALLs invoke.
static SUBTRACT: AtomicUsize = AtomicUsize::new(0);

/// Hook path: the notifications of a site's pools, covering the original
/// in flight.
fn notify(site: usize, first: usize, second: usize) -> [Option<Notification<'static>>; 2] {
    let [pool, other] = pools(MUTATIONS[site].1, first, second);
    [
        Some(WATCHES.notify(pool, site)),
        (other != 0).then(|| WATCHES.notify(other, site)),
    ]
}

// Each entry site takes at most four integer or pointer arguments (RCX..R9)
// and returns RAX unchanged from its original; the first two name the pools
// as its [`engine::air_wing_equipment_mutations::Receiver`] says.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::WingChanges);
            let _notifications = notify($site, first, second);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
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
