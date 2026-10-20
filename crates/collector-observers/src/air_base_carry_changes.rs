//! Allocation-free notifications from the native writers of everything a state air base's
//! AirBaseRegistrations subtree reads ([`engine_mutations::air_base_mutations`]). A detour only notifies the
//! shared watch registry ([`WatchRegistry`]) for the base handle its first argument names (a base's
//! handle, a pool's base handle), counts its site and calls the original; it never changes game
//! behaviour. A receiver whose handle does not read revokes every certificate. A pool site names an
//! unlinked pool's zero or invalid reference, which no registered base holds: a carried base's
//! pools all name it ([`crate::extended::air::RegistrationStore`]). Two writers are other families'
//! patches, whose detours forward here: the country air pool capacity setter
//! ([`crate::raid_changes`], only a store that changes the held value, [`capacity_written`]) and
//! air base destruction ([`crate::ship_model_changes`], [`base_destroyed`]). The domain observes
//! only while its own hooks and both forwarders are enabled. Anything unobserved (disabled,
//! poisoned, a refused registration, an epoch change, a running writer) makes the capture read that
//! base afresh ([`crate::extended::air::RegistrationStore`]).
pub mod fuel;
pub mod registration;

use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{air_base_mutations::MUTATIONS, air_base_mutations::Receiver};
use engine_native::Memory;
use registration::Difference;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Carried, read, verified and mismatched bases.
static READS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "a carried air base's registration subtree differs from a fresh read",
]);
/// State bases the latest complete pass visited, for off-thread sizing.
static BASES: AtomicUsize = AtomicUsize::new(0);
/// State bases whose decoded rows the decode worker appended from a walk of
/// the same segment, and those it walked and kept.
static DECODED: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
/// The first compared base that differed from its held subtree.
static FIRST_MISMATCH: std::sync::OnceLock<(u64, i32, Difference)> = std::sync::OnceLock::new();
/// [`Work`] summed over every pass: bases and QPC ticks by kind, then the
/// spliced spans and bytes.
static WORK: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];

/// Where one pass's AirBaseRegistrations acquisition time goes, by base:
/// [read without a carry (hosts, and every base without a store), carried
/// state bases (certificate, links and splice), state bases read afresh
/// (read and hold)], and what the carried ones spliced. Status only.
#[derive(Clone, Copy, Debug, Default)]
pub struct Work {
    bases: [u64; 3],
    ticks: [u64; 3],
    spliced: [u64; 2],
}

impl Work {
    pub const UNCARRIED: usize = 0;
    pub const CARRIED: usize = 1;
    pub const FRESH: usize = 2;

    /// One base of `kind`, whose registrations began at QPC `started`.
    pub fn note(&mut self, kind: usize, started: i64) {
        self.bases[kind] += 1;
        self.ticks[kind] += crate::clock::qpc().saturating_sub(started).max(0) as u64;
    }

    pub fn splice(&mut self, spans: usize, bytes: usize) {
        self.spliced[0] += spans as u64;
        self.spliced[1] += bytes as u64;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn bases(&self) -> [u64; 3] {
        self.bases
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn spliced(&self) -> [u64; 2] {
        self.spliced
    }
}

/// A worker publishes one pass's [`Work`].
#[cfg_attr(any(test, feature = "test-support"), allow(dead_code))]
pub fn record_work(work: &Work) {
    let values = work.bases.into_iter().chain(work.ticks).chain(work.spliced);
    for (target, value) in WORK.iter().zip(values) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

fn work_status() -> impl serde::Serialize {
    let work = WORK.each_ref().map(|value| value.load(Ordering::Relaxed));
    let mut frequency = 0;
    // SAFETY: an out pointer to a live local.
    let known = unsafe {
        windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
    } != 0;
    let ns = |ticks: u64| {
        (known && frequency > 0)
            .then(|| (u128::from(ticks) * 1_000_000_000 / frequency as u128) as u64)
    };
    crate::status::status!({
        "uncarried_bases": work[0],
        "uncarried_ns": ns(work[3]),
        "carried_bases": work[1],
        "carried_ns": ns(work[4]),
        "fresh_state_bases": work[2],
        "fresh_state_ns": ns(work[5]),
        "spliced_spans": work[6],
        "spliced_bytes": work[7],
        "scope": "cumulative since attach: AirBaseRegistrations acquisition by base, each base's time from the start of its registrations to its log: uncarried are host bases and every base without a store, read afresh with their carrier observer work; carried are state bases spliced from the store, with their certificate, link checks and splice (spliced_spans, spliced_bytes); fresh_state are state bases read afresh and held, sampled comparisons included",
    })
}

/// One observation domain: the watched base handles and each writer site's
/// calls.
pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    /// Receivers whose base handle did not read.
    unnamed: AtomicU64,
    /// Forwarded capacity stores that changed a container's capacity.
    capacities: AtomicU64,
    /// This family's hooks, and each [`Forwarder`], enabled.
    hooks: AtomicBool,
    forwarders: [AtomicBool; 2],
}

/// A family whose patch forwards a writer here.
#[derive(Clone, Copy)]
pub enum Forwarder {
    /// The capacity setter.
    Raid = 0,
    /// The base destructor.
    ShipModel = 1,
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
            unnamed: AtomicU64::new(0),
            capacities: AtomicU64::new(0),
            hooks: AtomicBool::new(false),
            forwarders: [const { AtomicBool::new(false) }; 2],
        }
    }

    fn apply(&self) {
        self.registry.set_enabled(
            self.hooks.load(Ordering::SeqCst)
                && self
                    .forwarders
                    .iter()
                    .all(|forwarder| forwarder.load(Ordering::SeqCst)),
        );
    }

    /// Hook path, allocation-free: a writer is about to change the base with
    /// `handle`; `None` names no base and revokes every certificate.
    /// Dropping the guard notifies its return.
    #[inline]
    pub fn notify(&self, handle: Option<u64>) -> Notification<'_> {
        match handle.and_then(|handle| usize::try_from(handle).ok()) {
            Some(handle) => self.registry.notify(handle),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.registry.notify_every()
            }
        }
    }

    /// Hook path: the notification of a call of `site` whose first argument
    /// is `first`, naming its base by the handle the site's [`Receiver`]
    /// holds.
    #[inline]
    pub fn notify_site(&self, site: usize, first: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.notify(handle(MUTATIONS[site].1, first))
    }

    /// An enabled domain with a table for `bases` receivers installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(bases: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(bases);
        watches.registry.boundary(false);
        watches.hooks.store(true, Ordering::SeqCst);
        for forwarder in &watches.forwarders {
            forwarder.store(true, Ordering::SeqCst);
        }
        watches.apply();
        watches
    }

    /// An enabled domain before any table is installed, as at attach.
    #[cfg(any(test, feature = "test-support"))]
    pub fn without_table() -> Self {
        let watches = Self::new();
        watches.hooks.store(true, Ordering::SeqCst);
        for forwarder in &watches.forwarders {
            forwarder.store(true, Ordering::SeqCst);
        }
        watches.apply();
        watches
    }

    /// Off the game thread: a table for `bases` receivers, installed at the
    /// next boundary.
    pub fn offer(&self, bases: usize) {
        self.registry.offer(bases);
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

/// The base handle `receiver` holds in RCX's object `first`; `None` when it
/// does not read.
#[inline]
fn handle(receiver: Receiver, first: usize) -> Option<u64> {
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    first
        .checked_add(receiver.handle())
        .and_then(|address| engine_native::read::<u64>(&memory, address))
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// `(epoch, sequence)` of one base handle, registering it; `None` when it
    /// cannot be observed. Equal values across hours prove that no observed
    /// writer touched the base.
    fn sequence(&self, handle: u64) -> Option<(u64, u64)>;
    /// A fresh read disagreed with a carried subtree: disable reuse for the
    /// attachment.
    fn mismatch(&self);
}

impl Changes for Watches {
    fn sequence(&self, handle: u64) -> Option<(u64, u64)> {
        self.registry.sequence(usize::try_from(handle).ok()?)
    }
    fn mismatch(&self) {
        self.registry.poison();
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, handle: u64) -> Option<(u64, u64)> {
        WATCHES.sequence(handle)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
}

/// Whether captures read every carryable base afresh and compare, rather
/// than the hour's sampled slice ([`crate::sampled_verify`]).
pub const VERIFY: bool = cfg!(feature = "air_base_carry_verify");

/// Hook path, allocation-free: `country air pool capacity setter(base, ..)`
/// is about to change a container capacity of `base` (or its held value did
/// not read). Hold the result over the native call.
pub fn capacity_written(base: usize) -> Notification<'static> {
    WATCHES.capacities.fetch_add(1, Ordering::Relaxed);
    WATCHES.notify(handle(Receiver::Base, base))
}

/// Hook path, allocation-free: `air base destruction(base)` is about to
/// retire `base`'s vectors. Hold the result over the native call.
pub fn base_destroyed(base: usize) -> Notification<'static> {
    WATCHES.notify(handle(Receiver::Base, base))
}

/// `forwarder`'s patch is enabled.
pub fn forwarder(forwarder: Forwarder, enabled: bool) {
    WATCHES.forwarders[forwarder as usize].store(enabled, Ordering::SeqCst);
    WATCHES.apply();
}

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.registry.active().saturating_add(fuel::active())
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
    fuel::boundary(invalidate);
}

/// Off the game thread: size the watches for the bases the latest pass
/// visited. Nothing is offered before a pass visited them: a later larger
/// table waits for an invalidating hour, while the first installs at once.
pub fn presize() {
    fuel::presize();
    let bases = BASES.load(Ordering::Relaxed);
    if bases != 0 {
        WATCHES.offer(bases);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
    fuel::collect();
}

/// A compared base `handle` differed from its held subtree at `hour`; the
/// first one is kept for the status.
pub fn note_mismatch(handle: u64, hour: i32, difference: Difference) {
    let _ = FIRST_MISMATCH.set((handle, hour, difference));
}

/// A worker publishes one pass's counts and, when it completed, the state
/// bases it visited for sizing.
#[cfg_attr(any(test, feature = "test-support"), allow(dead_code))]
pub fn record(counts: &registration::Counts, bases: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.carried,
        counts.read,
        counts.verified,
        counts.mismatches,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
    if let Some(bases) = bases {
        BASES.store(bases, Ordering::Relaxed);
    }
}

/// The decode worker publishes one pass's appended and kept bases.
pub fn record_decoded(reused: u64, walked: u64) {
    DECODED[0].fetch_add(reused, Ordering::Relaxed);
    DECODED[1].fetch_add(walked, Ordering::Relaxed);
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
        "configured": cfg!(feature = "air_base_carry_changes"),
        "verify": VERIFY,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "carried_bases": reads[0],
        "read_bases": reads[1],
        "verified_bases": reads[2],
        "mismatches": reads[3],
        "decoded_reused_bases": DECODED[0].load(Ordering::Relaxed),
        "decoded_walked_bases": DECODED[1].load(Ordering::Relaxed),
        "registration_work": work_status(),
        "first_mismatch": FIRST_MISMATCH.get().map(|(handle, hour, difference)| {
            crate::status::status!({
                "base": handle,
                "hour": hour,
                "what": difference.what,
                "address": difference.address,
                "length": difference.len,
                "byte": difference.byte,
            })
        }),
        "events": events,
        "forwarded_capacities": WATCHES.capacities.load(Ordering::Relaxed),
        "unnamed_receivers": WATCHES.unnamed.load(Ordering::Relaxed),
        "watches": WATCHES.registry.status(),
        "fuel": fuel::status(),
        "scope": "cumulative since attach; a state air base's AirBaseRegistrations subtree (slot vector, dense list, containers, pools, wings) is carried (carried_bases) by splicing the log spans its latest fresh read took into the hour's air bases log, when the base is at the same address, no hooked writer touched its handle since that read registered it and the hour takes its slots as that read did; its container countries' links are resolved afresh and must still name the held slots; host bases and every other base are read afresh (read_bases); every hour also reads the carryable bases of its slice afresh (one in rotation_hours by base handle; every one in verify mode) and compares them with the held subtree, disabling reuse at the first mismatch (sampled_*); the decode worker appends a base's rows from its walk of the same held segment in an earlier hour (decoded_reused_bases) and keeps the rows of each reported base it walks (decoded_walked_bases)"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
    fuel: fuel::Hooks,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; AirBaseRegistrations
    /// then reads every base afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "air_base_carry_changes")
            && MUTATIONS.iter().all(|(site, _)| {
                crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                create_country_container as *const () as usize,
                remove_country_access as *const () as usize,
                register_pool as *const () as usize,
                erase_pool as *const () as usize,
                add_wing as *const () as usize,
                remove_wing as *const () as usize,
                destroy_all_wings as *const () as usize,
                split_wing as *const () as usize,
                destroy_pool as *const () as usize,
                rebind_pool_definition as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self {
            hooks,
            fuel: unsafe { fuel::Hooks::new(memory, base) }?,
        })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !self.fuel.set_enabled(enabled)? {
            return Ok(false);
        }
        if !enabled {
            crate::wing_preferences::publication_enabled(false);
        }
        if !enabled {
            WATCHES.hooks.store(false, Ordering::SeqCst);
            WATCHES.apply();
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
            .hooks
            .store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        WATCHES.apply();
        crate::wing_preferences::publication_enabled(enabled && self.hooks.len() == SITES);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.fuel.executable_ranges())
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        crate::wing_preferences::publication_enabled(false);
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

// Each site takes integer or pointer arguments only (the asm prologues use
// RCX..R9) and returns RAX unchanged from its original; the first names the
// base as its [`Receiver`] says. The notification covers the original in
// flight.
macro_rules! callback {
    ($name:ident, $site:expr, ($first:ident $(, $arg:ident)*)) => {
        unsafe extern "system-unwind" fn $name($first: usize $(, $arg: usize)*) -> usize {
            let timing =
                crate::hook_timing::enter(crate::hook_timing::Family::AirBaseCarryChanges);
            let _notification = WATCHES.notify_site($site, $first);
            let preference = crate::wing_preferences::base_site($site, &[$first $(, $arg)*]);
            let original: unsafe extern "system-unwind" fn($first: usize $(, $arg: usize)*) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original($first $(, $arg)*) });
            if let Some(preference) = preference { preference.finish(result); }
            result
        }
    };
}
// Create country container (base, tag).
callback!(create_country_container, 0, (base, tag));
// Remove country access (base, tag).
callback!(remove_country_access, 1, (base, tag));
// Register pool (base, pool, dedupe) -> bool.
callback!(register_pool, 2, (base, pool, dedupe));
// Erase pool (base, pool) -> bool.
callback!(erase_pool, 3, (base, pool));
// Add wing (pool, wing, dedupe).
callback!(add_wing, 4, (pool, wing, dedupe));
// Remove wing (pool, wing, destroy) -> bool.
callback!(remove_wing, 5, (pool, wing, destroy));
// Destroy all wings (pool).
callback!(destroy_all_wings, 6, (pool));
// Split wing (pool, wing, size, force) -> wing.
callback!(split_wing, 7, (pool, wing, size, force));
// Wing pool deleting destructor(pool, flags) -> pool.
callback!(destroy_pool, 8, (pool, flags));
callback!(rebind_pool_definition, 9, (pool));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "per-call air-base callback microbenchmark; run only without a live game benchmark"]
    fn hook_cost_air_base_carry_dispatch() {
        const CALLS: usize = 100_000;
        unsafe extern "system-unwind" fn original(pool: usize) -> usize {
            std::hint::black_box(pool)
        }
        let handle = (7_u64 << 32) | 0x43;
        let mut pool = vec![0_u8; Receiver::Pool.handle() + 8];
        pool[Receiver::Pool.handle()..].copy_from_slice(&handle.to_ne_bytes());
        let pointer = pool.as_ptr() as usize;
        WATCHES.registry.offer(1);
        WATCHES.registry.boundary(false);
        WATCHES.registry.set_enabled(true);
        WATCHES.sequence(handle).unwrap();
        ORIGINAL[9].store(original as *const () as usize, Ordering::Release);
        let callback: unsafe extern "system-unwind" fn(usize) -> usize = rebind_pool_definition;
        let nanos = collector_test_support::per_call_ns(CALLS, || {
            for _ in 0..CALLS {
                std::hint::black_box(unsafe { std::hint::black_box(callback)(pointer) });
            }
        });
        let registry = collector_test_support::per_call_ns(CALLS, || {
            for _ in 0..CALLS {
                drop(std::hint::black_box(
                    WATCHES.registry.notify(handle as usize),
                ));
            }
        });
        println!(
            "hook_cost air_base pool rebind: callback={nanos:.1} ns/call, registry={registry:.1} ns/call"
        );
    }

    /// A base site names its base by its handle and a pool site by the pool's base handle; a
    /// receiver that does not read names none, so it revokes every certificate.
    #[test]
    fn a_site_names_its_base_by_handle_and_an_unread_receiver_names_none() {
        let site = |name: &str| {
            MUTATIONS
                .iter()
                .position(|(site, _)| site.name == name)
                .unwrap()
        };
        let handle: u64 = (7 << 32) | 0x43;
        let mut base = vec![0_u8; 0x20];
        base[8..16].copy_from_slice(&handle.to_le_bytes());
        let mut pool = vec![0_u8; 0x20];
        pool[0x18..0x20].copy_from_slice(&handle.to_le_bytes());
        let watches = Watches::enabled(16);
        for (name, object) in [("register_pool", &base), ("add_wing", &pool)] {
            let certified = watches.sequence(handle).unwrap();
            drop(watches.notify_site(site(name), object.as_ptr() as usize));
            assert_ne!(watches.sequence(handle), Some(certified), "{name}");
        }
        let other = watches.sequence(handle + 1).unwrap();
        drop(watches.notify(None));
        assert_ne!(watches.sequence(handle + 1), Some(other));
        assert_eq!(watches.unnamed.load(Ordering::Relaxed), 1);
    }
}
