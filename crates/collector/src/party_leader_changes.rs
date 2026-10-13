//! Allocation-free notifications from the native writers of every political
//! party's country-leader vector ([`engine::party_leader_mutations`]). A
//! detour only notifies the shared watch registry ([`WatchRegistry`]) for each
//! party its arguments name, counts its site and calls the original; it never
//! changes game behaviour. A carried PartyLeaders row also needs its leader's
//! character certificate from [`crate::character_changes`], whose sites cover
//! every write of a character's leader map. Anything unobserved (disabled,
//! poisoned, a refused registration, an epoch change, a running writer)
//! makes the capture read afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine::{
    Memory,
    party_leader_mutations::{MUTATIONS, VECTOR},
};
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused, read, verified and mismatched countries; reused and read rows.
static READS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried PartyLeaders rows differ from a fresh read",
]);
/// Parties the latest complete capture walked, for off-thread sizing.
static PARTIES: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched parties and each writer site's calls.
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
    /// is about to change `party`. Dropping the guard notifies its return.
    pub(crate) fn notify(&self, party: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(party)
    }

    /// An enabled domain with a table for `parties` receivers installed.
    #[cfg(test)]
    pub(crate) fn enabled(parties: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(parties);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
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

/// What a capture sees of the notifications: parties and the characters of
/// listed leaders.
pub(crate) trait Changes: Sync {
    /// `(epoch, revision)` of the party domain while it is observed and no
    /// writer runs; `None` otherwise. Equal values across hours prove that no
    /// observed writer touched any party's leader vector in between.
    fn quiet(&self) -> Option<(u64, u64)>;
    /// `(epoch, sequence)` of a party, registering it; `None` when it cannot
    /// be observed.
    fn party(&self, party: usize) -> Option<(u64, u64)>;
    /// `(epoch, sequence)` of a character in the character domain,
    /// registering it; `None` when it cannot be observed.
    fn character(&self, character: usize) -> Option<(u64, u64)>;
    /// A fresh read disagreed with carried rows: disable reuse for the
    /// attachment.
    fn mismatch(&self);
}

/// The hooks' domains.
pub(crate) struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        quiet()
    }
    fn party(&self, party: usize) -> Option<(u64, u64)> {
        WATCHES.registry.sequence(party)
    }
    fn character(&self, character: usize) -> Option<(u64, u64)> {
        crate::character_changes::Changes::sequence(&crate::character_changes::Observer, character)
    }
    fn mismatch(&self) {
        WATCHES.registry.poison();
    }
}

/// A test domain over its own party and character watches.
#[cfg(test)]
pub(crate) struct Pair<'a> {
    pub parties: &'a Watches,
    pub characters: &'a crate::character_changes::Watches,
}
#[cfg(test)]
impl Changes for Pair<'_> {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.parties.registry.quiet()
    }
    fn party(&self, party: usize) -> Option<(u64, u64)> {
        self.parties.registry.sequence(party)
    }
    fn character(&self, character: usize) -> Option<(u64, u64)> {
        crate::character_changes::Changes::sequence(self.characters, character)
    }
    fn mismatch(&self) {
        self.parties.registry.poison();
    }
}

/// The hooks' domain's quiet token ([`Changes::quiet`]).
pub(crate) fn quiet() -> Option<(u64, u64)> {
    WATCHES.registry.quiet()
}

/// Whether captures read every carryable country afresh and compare,
/// rather than the hour's sampled slice ([`crate::sampled_verify`]).
pub(crate) const VERIFY: bool = cfg!(feature = "party_leader_verify");

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

/// Off the game thread: size the watches for the parties the latest capture
/// walked. Nothing is offered before a capture walked them: a later larger
/// table waits for an invalidating hour, while the first installs at once.
pub fn presize() {
    let parties = PARTIES.load(Ordering::Relaxed);
    if parties != 0 {
        WATCHES.registry.offer(parties);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One capture's countries and rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
}

/// A worker publishes one capture's counts and, when it walked every
/// country, the number of parties for sizing.
pub(crate) fn record(counts: &Counts, parties: Option<usize>) {
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
    if let Some(parties) = parties {
        PARTIES.store(parties, Ordering::Relaxed);
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
        "configured": cfg!(feature = "party_leader_changes"),
        "verify": VERIFY,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_countries": reads[0],
        "read_countries": reads[1],
        "verified_countries": reads[2],
        "mismatches": reads[3],
        "reused_rows": reads[4],
        "read_rows": reads[5],
        "events": events,
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; a country's PartyLeaders rows are carried when its political status and party array header are those of the held read, no hooked writer touched any of its parties since and every listed leader's character keeps its certificate; the political status and party array header are read every hour; every hour also reads the carryable countries of its slice afresh (one in rotation_hours by country id; every one in verify mode), publishes the fresh rows and disables reuse at the first mismatch (sampled_*)"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; PartyLeaders then
    /// reads every country afresh.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "party_leader_changes")
            && MUTATIONS.iter().all(|(site, _)| {
                engine::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                insert_leader_front as *const () as usize,
                append_leader as *const () as usize,
                remove_leader as *const () as usize,
                promote_leader as *const () as usize,
                retire_first_leader as *const () as usize,
                kill_first_leader as *const () as usize,
                clear_leaders as *const () as usize,
                transfer_leaders as *const () as usize,
                validate_leaders as *const () as usize,
                read_leaders as *const () as usize,
                destroy_party as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
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
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::PartyLeaders,
                false,
            );
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
        // The stability inputs observe the ruling party's leaders here.
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::PartyLeaders,
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
        unsafe { WATCHES.registry.release() };
    }
}

// Each site takes integer or pointer arguments only (RCX..R9 and, for the
// transfer, two stack slots a Win64 detour forwards) and returns RAX
// unchanged from its original. `$party` names each written party; every
// notification covers the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr, ($($arg:ident),+), [$($party:expr),+]) => {
        unsafe extern "system" fn $name($($arg: usize),+) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::PartyLeaderChanges);
            let _notifications = [$(WATCHES.notify($party, $site)),+];
            let _inputs = [$(crate::country_input_changes::forwarded(
                crate::country_input_changes::Forward::PartyLeaders,
                $party,
                0,
            )),+];
            let original: unsafe extern "system" fn($($arg: usize),+) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($($arg),+) })
        }
    };
}
// Insert leader front (party, leader).
callback!(insert_leader_front, 0, (party, leader), [party]);
// Append leader (party, leader).
callback!(append_leader, 1, (party, leader), [party]);
// Remove leader (party, leader, flag) -> bool.
callback!(remove_leader, 2, (party, leader, flag), [party]);
// Promote leader (party, leader).
callback!(promote_leader, 3, (party, leader), [party]);
// Retire first leader (party).
callback!(retire_first_leader, 4, (party), [party]);
// Kill first leader (party).
callback!(kill_first_leader, 5, (party), [party]);
// Clear leaders (party).
callback!(clear_leaders, 6, (party), [party]);
// Transfer leaders (source, destination, list, out, flag, flag): both parties.
callback!(
    transfer_leaders,
    7,
    (source, destination, list, out, first, second),
    [source, destination]
);
// Validate leaders (party).
callback!(validate_leaders, 8, (party), [party]);
// Read leaders (reader, token, party + 0x70).
callback!(
    read_leaders,
    9,
    (reader, token, vector),
    [vector.wrapping_sub(VECTOR)]
);
// Destroy party (party, flags).
callback!(destroy_party, 10, (party, flags), [party]);
