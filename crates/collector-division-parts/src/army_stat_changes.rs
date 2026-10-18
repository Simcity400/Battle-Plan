//! Allocation-free notifications from the native writers of every division's stat block
//! (`cunit::STAT_BLOCK`) whose attack operands and maxima DivisionStatus records, and of its name
//! member (`carmy_combat_inputs::NAME_GROUP_MEMBER`), whose span and cached-name lookup cache it
//! records ([`engine_mutations::army_stat_mutations`]). A detour only notifies the shared marking watch
//! registry ([`WatchRegistry::marking`]) for the `CArmy` or member its first argument names, or
//! starts an epoch for the country-wide name refresh, counts the call and calls the original; it
//! never changes game behaviour. The writers run on the game thread and on the TBB workers of the
//! hourly update, which the registry's atomics serve. Receivers are the armies and members a
//! DivisionStatus capture registered. Anything unobserved (disabled, poisoned, a refused
//! registration, an epoch change, a running writer) makes the capture read that division afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::army_stat_mutations::MUTATIONS;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused, read, verified and mismatched divisions; refused reads; witnessed
/// divisions; divisions whose status maxima, and whose name members, were
/// carried.
static READS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried DivisionStatus attack operands, maxima or name member differ from a fresh read",
]);
/// Divisions the latest complete capture visited, for off-thread sizing.
static DIVISIONS: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched armies and the writer's calls.
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
            registry: WatchRegistry::marking(),
            calls: [const { AtomicU64::new(0) }; SITES],
        }
    }

    /// Hook path, allocation-free: the writer of `site` ([`MUTATIONS`]
    /// index) is about to rewrite `receiver`, an army's stat block or a name
    /// member. Dropping the guard notifies its return.
    pub fn notify(&self, receiver: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(receiver)
    }

    /// Hook path, allocation-free: the writer of `site` rewrites members it
    /// does not name, revoking every certificate on entry and on return.
    pub fn notify_every(&self, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify_every()
    }

    /// An enabled domain with a table for `armies` receivers installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(armies: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(armies);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
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
    /// Before any read of the hour: visits every registered army and member
    /// a hooked writer touched since the previous take, clearing its mark,
    /// and returns the epoch they were marked in ([`WatchRegistry::take`]).
    /// A receiver registered in that epoch and not visited since was touched
    /// by no observed writer. `None` when nothing taken certifies anything.
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    /// Registers a receiver before its read; whether it is now watched in
    /// the epoch of the hour's take.
    fn register(&self, receiver: usize) -> bool;
    /// A fresh read disagreed with carried operands: disable reuse for the
    /// attachment.
    fn mismatch(&self);
}

impl Changes for Watches {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.registry.take(visit)
    }
    fn register(&self, receiver: usize) -> bool {
        self.registry.register(receiver).is_some()
    }
    fn mismatch(&self) {
        self.registry.poison();
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.take(visit)
    }
    fn register(&self, receiver: usize) -> bool {
        WATCHES.register(receiver)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
}

/// Whether captures read every carryable division afresh and compare,
/// rather than the hour's sampled slice ([`crate::sampled_verify`]).
pub const VERIFY: bool = cfg!(feature = "army_stat_verify");

/// Whether the off-by-default diagnostic records the native operands
/// (`DivisionStatus.attack`) of another slice of divisions each hour, which
/// the recorder's derivation check compares with the derived ones.
pub const WITNESS: bool = cfg!(feature = "attack_witness");

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

/// Off the game thread: size the watches for the divisions the latest
/// capture visited. Nothing is offered before a capture visited them: a
/// later larger table waits for an invalidating hour, while the first
/// installs at once.
pub fn presize() {
    let divisions = DIVISIONS.load(Ordering::Relaxed);
    if divisions != 0 {
        // Each division registers its army and its name member.
        WATCHES.registry.offer(divisions.saturating_mul(2));
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One capture's divisions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    pub refused: u64,
    pub witnessed: u64,
    pub maxima_reused: u64,
    pub names_reused: u64,
}

/// A worker publishes one capture's counts and, when it visited every
/// division, their number for sizing.
pub fn record(counts: &Counts, divisions: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        counts.refused,
        counts.witnessed,
        counts.maxima_reused,
        counts.names_reused,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
    if let Some(divisions) = divisions {
        DIVISIONS.store(divisions, Ordering::Relaxed);
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
        "configured": cfg!(feature = "army_stat_changes"),
        "verify": VERIFY,
        "witness": WITNESS,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_divisions": reads[0],
        "read_divisions": reads[1],
        "verified_divisions": reads[2],
        "mismatches": reads[3],
        "refused_divisions": reads[4],
        "witnessed_divisions": reads[5],
        "maxima_reused_divisions": reads[6],
        "names_reused_divisions": reads[7],
        "events": events,
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; a division's DivisionStatus.attack_stats are carried when the hour's take of the marks returned the held capture's epoch, its army has the address and stat block of the held read and no hooked update_active_stats marked it since; every other division's stat block is read; every hour also reads the carryable divisions of its slice afresh (one in rotation_hours by division id; every one in verify mode), publishes the fresh operands and disables reuse at the first mismatch (sampled_*); witnessed_divisions counts the attack_witness diagnostic's native operands; maxima_reused_divisions counts divisions whose DivisionStatus maxima (same block, same writer, same marks) were carried rather than read, and names_reused_divisions those whose name member span and cached-name lookup cache text (marked on the member by cached-name refresh, the country-wide refresh moving the epoch) were; the slice's carryable maxima and members are read and compared too"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; DivisionStatus then
    /// reads every division's stat block afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "army_stat_changes")
            && MUTATIONS.iter().all(|site| {
                engine_native::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                update_active_stats as *const () as usize,
                refresh_cached_name as *const () as usize,
                refresh_country_names as *const () as usize,
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

// `update active stats(army)` takes the primary CArmy in
// RCX and returns nothing a caller reads; the detour forwards RCX..R9 and
// RAX. The notification covers the original in flight.
unsafe extern "system" fn update_active_stats(
    army: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify(army, 0);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(army, second, third, fourth) })
}

// `refresh cached name(member, ...)` takes the
// member in RCX; members of ships, operatives and lines are never registered.
unsafe extern "system" fn refresh_cached_name(
    member: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify(member, 1);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(member, second, third, fourth) })
}

// `refresh country names(group, country, ...)` rewrites the members of a country's
// divisions without naming them: every certificate is revoked.
unsafe extern "system" fn refresh_country_names(
    first: usize,
    country: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify_every(2);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(first, country, third, fourth) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The writer moves only the army it names; a running writer certifies
    /// nothing, and a mismatch poisons the domain.
    #[test]
    fn the_writer_marks_only_its_registered_receiver() {
        let watches = Watches::enabled(16);
        let take = |watches: &Watches| {
            let mut touched = Vec::new();
            let epoch = Changes::take(watches, &mut |receiver| touched.push(receiver));
            (epoch, touched)
        };
        let (epoch, _) = take(&watches);
        assert!([0x1000, 0x2000].map(|army| Changes::register(&watches, army)) == [true; 2]);
        drop(watches.notify(0x2000, 0));
        // An unregistered receiver is not watched.
        drop(watches.notify(0x3000, 1));
        assert_eq!(take(&watches), (epoch, vec![0x2000]));
        assert_eq!(take(&watches), (epoch, vec![]));
        assert_eq!(watches.calls[0].load(Ordering::Relaxed), 1);
        // A running writer certifies nothing.
        let running = watches.notify(0x1000, 0);
        assert_eq!(take(&watches).0, None);
        drop(running);
        assert_eq!(take(&watches), (epoch, vec![0x1000]));
        // The country-wide refresh and an invalidating hour move the epoch.
        drop(watches.notify_every(2));
        let (moved, _) = take(&watches);
        assert!(moved.is_some() && moved != epoch);
        watches.boundary(true);
        assert_ne!(take(&watches).0, moved);
        Changes::mismatch(&watches);
        assert!(watches.poisoned());
        assert_eq!(take(&watches).0, None);
    }
}
