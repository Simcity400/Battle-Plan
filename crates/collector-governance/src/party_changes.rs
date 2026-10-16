//! Allocation-free notifications from the native writers of the political
//! party fields PoliticalParties carries
//! ([`engine::political_party_mutations`]). A detour only notifies the shared
//! watch registry ([`WatchRegistry`]) for the party its first argument names,
//! counts its site and calls the original; it never changes game behaviour.
//! Receivers are the `CPoliticalParty` addresses a PoliticalParties capture
//! registered, and the `CPoliticalStatus` addresses an ActiveIdeas capture
//! registered, which no writer notifies, so their certificates change only
//! with the epoch. Anything unobserved (disabled, poisoned, a refused
//! registration, an epoch change, a running writer) makes the capture read
//! that country afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine::{Memory, political_party_mutations::MUTATIONS};
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Per table (PoliticalParties, ActiveIdeas): reused, read, verified and
/// mismatched countries; reused and read rows.
static READS: [[AtomicU64; 6]; 2] = [const { [const { AtomicU64::new(0) }; 6] }; 2];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried PoliticalParties rows differ from a fresh read",
    "carried ActiveIdeas rows differ from a fresh read",
]);
/// Parties and political statuses the latest complete capture registered,
/// for off-thread sizing.
static RECEIVERS: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched receivers and each writer site's calls.
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
            registry: WatchRegistry::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
        }
    }

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// is about to change `party`. Dropping the guard notifies its return.
    pub fn notify(&self, party: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(party)
    }

    /// An enabled domain with a table for `receivers` installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(receivers: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(receivers);
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
    /// `(epoch, revision)` of the whole domain while it is observed and no
    /// writer runs; `None` otherwise. Equal values across hours prove that no
    /// observed writer touched any party in between.
    fn quiet(&self) -> Option<(u64, u64)>;
    /// `(epoch, sequence)` of one party or political status, registering it;
    /// `None` when it cannot be observed. Equal values across hours prove
    /// that no observed writer touched it.
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)>;
    /// A fresh read disagreed with carried rows: disable reuse for the
    /// attachment.
    fn mismatch(&self);
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.registry.sequence(receiver)
    }
    fn mismatch(&self) {
        self.registry.poison();
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        quiet()
    }
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(receiver)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
}

/// The hooks' domain's quiet token ([`Changes::quiet`]).
pub fn quiet() -> Option<(u64, u64)> {
    WATCHES.registry.quiet()
}

/// Whether captures read every carryable country afresh and compare, rather
/// than the hour's sampled slice ([`crate::sampled_verify`]).
pub const VERIFY: bool = cfg!(feature = "party_verify");

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

/// Off the game thread: size the watches for the receivers the latest
/// capture registered. Nothing is offered before a capture registered them:
/// a later larger table waits for an invalidating hour, while the first
/// installs at once.
pub fn presize() {
    let receivers = RECEIVERS.load(Ordering::Relaxed);
    if receivers != 0 {
        WATCHES.registry.offer(receivers);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One table's countries and rows in one capture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
}

/// A worker publishes one capture's counts per table (PoliticalParties,
/// ActiveIdeas) and, when it walked every country, the receivers it
/// registered, for sizing.
pub fn record(counts: &[Counts; 2], receivers: Option<usize>) {
    for (table, counts) in counts.iter().enumerate() {
        for (target, value) in READS[table].iter().zip([
            counts.reused,
            counts.read,
            counts.verified,
            counts.mismatches,
            counts.reused_rows,
            counts.read_rows,
        ]) {
            target.fetch_add(value, Ordering::Relaxed);
        }
        SAMPLED.record(counts.verified, counts.mismatches, table);
    }
    if let Some(receivers) = receivers {
        RECEIVERS.store(receivers, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&WATCHES.calls)
            .map(|(site, n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let table = |index: usize| {
        let reads = READS[index]
            .each_ref()
            .map(|value| value.load(Ordering::Relaxed));
        crate::status::status!({
            "reused_countries": reads[0],
            "read_countries": reads[1],
            "verified_countries": reads[2],
            "mismatches": reads[3],
            "reused_rows": reads[4],
            "read_rows": reads[5],
        })
    };
    crate::status::status!({
        "configured": cfg!(feature = "party_changes"),
        "verify": VERIFY,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "political_parties": table(0),
        "active_ideas": table(1),
        "events": events,
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; a country's PoliticalParties rows are carried when its political status, party array header and every party pointer are those of the held read and no hooked writer touched a listed party since that read, with each party's popularity and the ruling flag read every hour; its ActiveIdeas rows are carried when its status, idea array header and every idea pointer are those of the held read in the same epoch; every hour also reads the carryable countries of its slice afresh (one in rotation_hours by country id; every one in verify mode), publishes the fresh rows and disables reuse at the first mismatch (sampled_*)"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; PoliticalParties
    /// then reads every party afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "party_changes")
            && MUTATIONS.iter().all(|site| {
                engine::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                party_names as *const () as usize,
                construct_party as *const () as usize,
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

// Each site takes three integer or pointer arguments (RCX..R8) and returns
// RAX unchanged from its original; the first is the written party. The
// notification covers the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr, ($party:ident $(, $arg:ident)*)) => {
        unsafe extern "system" fn $name($party: usize $(, $arg: usize)*) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::PartyChanges);
            let _notification = WATCHES.notify($party, $site);
            let original: unsafe extern "system" fn($party: usize $(, $arg: usize)*) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($party $(, $arg)*) })
        }
    };
}
// Party names (party, long name, name).
callback!(party_names, 0, (party, long_name, name));
// Construct party (party, country link, ideology).
callback!(construct_party, 1, (party, link, ideology));
