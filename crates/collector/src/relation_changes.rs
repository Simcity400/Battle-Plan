//! Allocation-free notifications from the native writers of the country
//! vectors AirCountryRelations records ([`engine::diplomacy_mutations`]). A
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
use engine::{Memory, diplomacy_mutations::MUTATIONS};
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
static VERIFY: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

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

/// Countries reused, re-read and reached this hour.
pub fn record(reused: u64, read: u64, quiet_hours: u64) {
    for (target, value) in READS.iter().zip([reused, read, quiet_hours]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

/// `relation_verify`: one hour's fresh copy checked against the held rows.
#[cfg(feature = "relation_verify")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Verification {
    pub countries: u64,
    /// Countries whose held rows differ from the fresh copy.
    pub mismatches: u64,
    /// Parts whose fresh copy failed.
    pub unavailable_parts: u64,
    /// Worker ticks of the fresh copies.
    pub ticks: u64,
}

/// A mismatch disables reuse for the attachment.
#[cfg(feature = "relation_verify")]
pub fn verified(verification: Verification) {
    for (target, value) in VERIFY.iter().zip([
        verification.countries,
        verification.mismatches,
        verification.unavailable_parts,
        verification.ticks,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if verification.mismatches != 0 {
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
    let verify = VERIFY.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "relation_changes"),
        "verify": cfg!(feature = "relation_verify"),
        "poisoned": WATCHES.poisoned(),
        "verified_countries": verify[0],
        "mismatches": verify[1],
        "verify_unavailable_parts": verify[2],
        "verify_ticks": verify[3],
        "reused_countries": reads[0],
        "read_countries": reads[1],
        "quiet_hours": reads[2],
        "events": events,
        "watches": WATCHES.status(),
        "scope": "cumulative since attach; a quiet hour reuses every country's vectors without a read; verify mode also copies every country each hour, compares it with the held rows on the decode thread and disables reuse at the first mismatch; verify_ticks are QPC worker ticks"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
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
                if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
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
        }
        for hook in &mut self.hooks {
            if !hook
                .set_enabled(enabled)
                .map_err(|_| "native hook state change failed")?
            {
                return Ok(false);
            }
        }
        WATCHES.set_enabled(enabled && !self.hooks.is_empty());
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
    let _notification = mutation(diplomacy, 0);
    let original: unsafe extern "system" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    unsafe { original(diplomacy) }
}
unsafe extern "system" fn reset(diplomacy: usize, retain: u8, rebuild: u8, keep: usize) {
    let _notification = mutation(diplomacy, 1);
    let original: unsafe extern "system" fn(usize, u8, u8, usize) =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    unsafe { original(diplomacy, retain, rebuild, keep) }
}
unsafe extern "system" fn add(country: usize, tag: usize) {
    let _notification = mutation(country, 2);
    let original: unsafe extern "system" fn(usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    unsafe { original(country, tag) }
}
unsafe extern "system" fn remove(country: usize, tag: usize, notify: u8) {
    let _notification = mutation(country, 3);
    let original: unsafe extern "system" fn(usize, usize, u8) =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    unsafe { original(country, tag, notify) }
}
