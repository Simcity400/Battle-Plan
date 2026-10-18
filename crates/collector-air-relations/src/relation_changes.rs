//! Allocation-free notifications from the native writers of the country
//! vectors AirCountryRelations records ([`engine_events::diplomacy_mutations`]). A
//! detour only notifies the shared watch registry ([`WatchRegistry`]) and
//! calls the original; it never changes game behaviour. Receivers are
//! diplomacy status or country object addresses the collector registered; a
//! notification never infers an owner. Anything unobserved (disabled,
//! poisoned, a refused registration, an epoch change) makes the collector
//! read afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_events::diplomacy_mutations::MUTATIONS;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

static WATCHES: WatchRegistry = WatchRegistry::new();
/// Receivers watched per country slot: its country object and diplomacy status.
const RECEIVERS_PER_COUNTRY: usize = 2;
static ORIGINAL: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static EVENTS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static READS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static SAMPLED: crate::sampled_verify::Sampled =
    crate::sampled_verify::Sampled::burned_in(&["held relation vectors differ from a fresh copy"]);

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}

/// Off the game thread: size the watches for `slots` country slots.
pub fn presize(slots: usize) {
    WATCHES.offer(slots.saturating_mul(RECEIVERS_PER_COUNTRY));
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.collect();
}

fn mutation(receiver: usize, site: usize) -> Notification<'static> {
    EVENTS[site].fetch_add(1, Ordering::Relaxed);
    WATCHES.notify(receiver)
}

/// What a capture sees of the notifications.
pub trait Changes {
    /// `(epoch, revision)` when observation is live and no writer is running;
    /// equal values across hours prove no vector changed.
    fn quiet(&self) -> Option<(u64, u64)>;
    /// `(epoch, sequence)` of one receiver, registering it; `None` when it
    /// cannot be observed.
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)>;
}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(receiver)
    }
}

/// Countries reused, re-read for a writer and reached this hour.
pub fn record(reused: u64, read: u64, quiet_hours: u64) {
    for (target, value) in READS.iter().zip([reused, read, quiet_hours]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

/// One hour's sampled comparison ([`crate::sampled_verify`]): `verified`
/// carried countries copied afresh, `mismatches` of them differing from the
/// held rows. A mismatch disables reuse for the attachment.
pub fn sampled(verified: u64, mismatches: u64) {
    if SAMPLED.record(verified, mismatches, 0) {
        WATCHES.poison();
    }
}

pub fn status() -> impl serde::Serialize {
    let event_counts = EVENTS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(event_counts)
            .map(|(site, n)| (site.name, n)),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "relation_changes"),
        "verify": cfg!(feature = "relation_verify"),
        "poisoned": WATCHES.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_countries": reads[0],
        "read_countries": reads[1],
        "quiet_hours": reads[2],
        "events": events,
        "watches": WATCHES.status(),
        "scope": "cumulative since attach; a quiet hour reuses every country's vectors without a read; every hour also copies the carried countries of its slice (one in rotation_hours by index within their part; every carried country in verify mode), compares them with the held rows on the decode thread, publishes the fresh copies and disables reuse at the first mismatch (sampled_*)"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "relation_changes") {
            let callbacks = [
                rebuild as *const () as usize,
                reset as *const () as usize,
                add as *const () as usize,
                remove as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                if engine_native::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                    return Err("relation hook sentinel differs");
                }
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
            WATCHES.set_enabled(false);
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Relations,
                false,
            );
            crate::war_changes::forwarder(crate::war_changes::Forwarder::Relations, false);
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
        WATCHES.set_enabled(enabled && !self.hooks.is_empty());
        // The country inputs observe diplomacy reset and
        // wartime ally rebuild here.
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Relations,
            enabled && !self.hooks.is_empty(),
        );
        // The war domain observes the reset, which ends a country's wars.
        crate::war_changes::forwarder(
            crate::war_changes::Forwarder::Relations,
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
        unsafe { WATCHES.release() };
    }
}

unsafe extern "system" fn rebuild(diplomacy: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RelationChanges);
    let _notification = mutation(diplomacy, 0);
    // The rebuild rewrites the co-belligerent bytes and candidate vectors
    // the country's counter row reads.
    let _inputs = crate::country_input_changes::forwarded(
        crate::country_input_changes::Forward::WartimeAllies,
        diplomacy,
        0,
    );
    let original: unsafe extern "system" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(diplomacy) })
}
unsafe extern "system" fn reset(diplomacy: usize, retain: u8, rebuild: u8, keep: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RelationChanges);
    let _notification = mutation(diplomacy, 1);
    // The reset clears the exile host other countries' war flags read.
    let _inputs = crate::country_input_changes::forwarded(
        crate::country_input_changes::Forward::DiplomacyReset,
        diplomacy,
        0,
    );
    let _wars = crate::war_changes::forwarded(crate::war_changes::Site::DiplomacyReset);
    let original: unsafe extern "system" fn(usize, u8, u8, usize) =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(diplomacy, retain, rebuild, keep) })
}
unsafe extern "system" fn add(country: usize, tag: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RelationChanges);
    let _notification = mutation(country, 2);
    let original: unsafe extern "system" fn(usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(country, tag) })
}
unsafe extern "system" fn remove(country: usize, tag: usize, notify: u8) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RelationChanges);
    let _notification = mutation(country, 3);
    let original: unsafe extern "system" fn(usize, usize, u8) =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(country, tag, notify) })
}
