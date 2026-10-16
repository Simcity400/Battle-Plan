//! Allocation-free notifications from the native writers of the per-country
//! politics fields the Politics walk reads every hour
//! ([`engine::political_status_mutations`]), in two token domains: the
//! structure (capitals, last elections, election frequency and byte, ruling
//! parties, party and idea arrays) and party popularity, which the daily update rewrites every day.
//! A detour only moves its domains' quiet tokens, counts its site and calls
//! the original; it never changes game behaviour. The domains register no
//! receivers: the Politics capture compares the tokens it took before its
//! latest complete walk with this hour's, and equal tokens prove that no
//! observed writer ran in between ([`WatchRegistry::quiet`]).
//!
//! The politics capture family ([`crate::politics_changes`]) patches the
//! country appearance reset and daily politics update and forwards their
//! calls ([`forwarded`]); the domains observe only while those patches are
//! enabled. Appearance reset moves both domains. Daily politics always
//! rewrites popularity through the clamped setter, so a popularity
//! notification covers its call. Its only inline structure store is the
//! election-day last election. The guard reads that date before and after
//! the original, moving structure on return only when it changed, or
//! covering the whole call when the first read fails ([`Guard`]).
//! The capital setter runs for every hourly occupation controller change,
//! whether or not the capital moves, and writes the
//! structure only through its capital `+0x1018` and original capital
//! `+0x101C` stores, so its guard reads both before and after the call the
//! same way. Captures never run while the hourly
//! original runs, so the next capture sees that notification. Anything
//! unobserved (disabled, poisoned, an epoch change, a running writer) yields
//! no token, and the walk reads afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine::{
    Memory, ccountry, cpolitical_status,
    political_status_mutations::{Domain, MUTATIONS, Patch, SET_POPULARITY_RVA},
};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static STRUCTURE: WatchRegistry = WatchRegistry::new();
static POPULARITY: WatchRegistry = WatchRegistry::new();
static CALLS: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
/// Forwarded calls: `country appearance reset`, the daily update, and the daily
/// updates whose last election changed.
static FORWARDED: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
/// `capital setter` calls that changed a capital.
static CAPITAL_CHANGES: AtomicU64 = AtomicU64::new(0);
/// The [`MUTATIONS`] index of `capital setter`.
const SET_CAPITAL: usize = 0;
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// The clamped popularity setter the CALL sites invoke.
static SET_POPULARITY: AtomicUsize = AtomicUsize::new(0);
/// This family's own patches are enabled.
static OWN: AtomicBool = AtomicBool::new(false);
/// The politics capture family forwards its two shared sites.
static FORWARD: AtomicBool = AtomicBool::new(false);

/// Quiet hours (the walk's four tables carried whole), popularity refresh
/// hours (carried with every held party's popularity gathered afresh), read
/// hours, compared countries and mismatches.
static READS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "a quiet politics hour's held Politics, PoliticalParties, ActiveIdeas or PartyLeaders rows differ from a fresh read",
]);

/// Whether the walk reads every hour afresh and compares with what a quiet
/// hour would carry, rather than carrying it (the family's burn-in).
pub const VERIFY: bool = cfg!(feature = "political_status_verify");

/// The shared sites [`crate::politics_changes`] forwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forwarded {
    /// `country appearance reset(country)`.
    ResetCountry,
    /// The daily politics update (status).
    PoliticsDaily,
}

/// The structure domain's token: `(epoch, revision)` while observed and no
/// structure writer runs.
pub fn structure() -> Option<(u64, u64)> {
    STRUCTURE.quiet()
}

/// The popularity domain's token: `(epoch, revision)` while observed and no
/// popularity writer runs.
pub fn popularity() -> Option<(u64, u64)> {
    POPULARITY.quiet()
}

/// A quiet hour's held rows differed from a fresh read: neither domain
/// certifies anything for the rest of the attachment.
pub fn mismatch() {
    STRUCTURE.poison();
    POPULARITY.poison();
}

/// What the Politics walk sees of the domains.
pub trait Changes: Sync {
    /// [`structure`].
    fn structure(&self) -> Option<(u64, u64)>;
    /// [`popularity`].
    fn popularity(&self) -> Option<(u64, u64)>;
    /// [`mismatch`].
    fn mismatch(&self);
}

/// The hooks' domains.
pub struct Observer;
impl Changes for Observer {
    fn structure(&self) -> Option<(u64, u64)> {
        structure()
    }
    fn popularity(&self) -> Option<(u64, u64)> {
        popularity()
    }
    fn mismatch(&self) {
        mismatch();
    }
}

/// The notifications covering one writer's original call; dropping it after
/// the original returns notifies the return.
pub struct Guard<'a> {
    structure_domain: &'a WatchRegistry,
    /// The structure field the call may write, read before it: the structure
    /// moves on return only if it changed.
    watched: Option<Watched>,
    _structure: Option<Notification<'a>>,
    _popularity: Option<Notification<'a>>,
}

#[derive(Clone, Copy)]
enum Watched {
    /// The daily update's status and its last election.
    LastElection(usize, i32),
    /// `capital setter`'s country and its capital and original capital.
    Capitals(usize, u64),
}

impl Watched {
    /// Whether the call changed the field, counting the change.
    fn changed(self) -> bool {
        let (changed, counter) = match self {
            Self::LastElection(status, before) => {
                (last_election(status) != Some(before), &FORWARDED[2])
            }
            Self::Capitals(country, before) => {
                (capitals(country) != Some(before), &CAPITAL_CHANGES)
            }
        };
        if changed {
            counter.fetch_add(1, Ordering::Relaxed);
        }
        changed
    }
}

impl<'a> Guard<'a> {
    fn new(structure: &'a WatchRegistry, popularity: &'a WatchRegistry, domain: Domain) -> Self {
        Self {
            structure_domain: structure,
            watched: None,
            _structure: domain.structure().then(|| structure.notify(0)),
            _popularity: domain.popularity().then(|| popularity.notify(0)),
        }
    }

    /// Set capital on `country`: the structure covers the
    /// call only when the capitals cannot be read first, and otherwise moves
    /// on return if the call changed one.
    fn capital(structure: &'a WatchRegistry, country: usize) -> Self {
        let before = capitals(country);
        Self {
            structure_domain: structure,
            watched: before.map(|before| Watched::Capitals(country, before)),
            _structure: before.is_none().then(|| structure.notify(0)),
            _popularity: None,
        }
    }

    /// The daily politics update on `status`: popularity covers the call;
    /// the structure covers it only when the last election cannot be read
    /// first, and otherwise moves on return if the election-day store changed it.
    fn daily(structure: &'a WatchRegistry, popularity: &'a WatchRegistry, status: usize) -> Self {
        let before = last_election(status);
        Self {
            structure_domain: structure,
            watched: before.map(|before| Watched::LastElection(status, before)),
            _structure: before.is_none().then(|| structure.notify(0)),
            _popularity: Some(popularity.notify(0)),
        }
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        // Runs before the fields' notifications return, so popularity still
        // covers the comparison.
        if self.watched.is_some_and(Watched::changed) {
            drop(self.structure_domain.notify(0));
        }
    }
}

/// The status's last election `CPoliticalStatus+0xA0`, through a guarded read.
fn last_election(status: usize) -> Option<i32> {
    let memory = unsafe { engine::memory::LiveMemory::new() };
    status
        .checked_add(cpolitical_status::LAST_ELECTION)
        .and_then(|address| engine::read::<i32>(&memory, address))
}

/// The country's capital `CCountry+0x1018` and original capital `+0x101C`,
/// through one guarded read.
fn capitals(country: usize) -> Option<u64> {
    let memory = unsafe { engine::memory::LiveMemory::new() };
    country
        .checked_add(ccountry::CAPITAL)
        .and_then(|address| engine::read::<u64>(&memory, address))
}

/// One walk hour's use of the domains.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// The walk's four tables were carried whole, reading no country.
    pub quiet: bool,
    /// They were carried with every held party's popularity read afresh.
    pub popularity: bool,
    /// Every country was read.
    pub read: bool,
    pub verified: u64,
    pub mismatches: u64,
}

/// The walk already called [`mismatch`] at a difference.
pub fn record(counts: &Counts) {
    for (target, value) in READS.iter().zip([
        u64::from(counts.quiet),
        u64::from(counts.popularity),
        u64::from(counts.read),
        counts.verified,
        counts.mismatches,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
}

/// Notifications running now.
pub fn active() -> usize {
    STRUCTURE.active() + POPULARITY.active()
}

/// Game thread, in the hourly callback before any capture.
pub fn boundary(invalidate: bool) {
    STRUCTURE.boundary(invalidate);
    POPULARITY.boundary(invalidate);
}

fn refresh() {
    let enabled = OWN.load(Ordering::SeqCst) && FORWARD.load(Ordering::SeqCst);
    STRUCTURE.set_enabled(enabled);
    POPULARITY.set_enabled(enabled);
}

/// The politics capture family's patches of the shared sites are enabled.
pub fn set_forward(enabled: bool) {
    FORWARD.store(enabled, Ordering::SeqCst);
    refresh();
}

/// Hook path: a shared site is about to run with `receiver` its first
/// argument (the country, or the daily update's status). Dropping the guard
/// after the original returns notifies its return.
pub fn forwarded(site: Forwarded, receiver: usize) -> Guard<'static> {
    FORWARDED[site as usize].fetch_add(1, Ordering::Relaxed);
    match site {
        Forwarded::ResetCountry => Guard::new(&STRUCTURE, &POPULARITY, Domain::Both),
        Forwarded::PoliticsDaily => Guard::daily(&STRUCTURE, &POPULARITY, receiver),
    }
}

/// `first` is the site's first argument: `capital setter`'s country.
fn notify(site: usize, first: usize) -> Guard<'static> {
    CALLS[site].fetch_add(1, Ordering::Relaxed);
    if site == SET_CAPITAL {
        Guard::capital(&STRUCTURE, first)
    } else {
        Guard::new(&STRUCTURE, &POPULARITY, MUTATIONS[site].2)
    }
}

/// Tests: enable the domains as installed hooks would.
#[cfg(any(test, feature = "test-support"))]
pub fn enable_for_test() {
    OWN.store(true, Ordering::SeqCst);
    FORWARD.store(true, Ordering::SeqCst);
    refresh();
}

/// Tests: a writer of `domain` runs and returns.
#[cfg(any(test, feature = "test-support"))]
pub fn write_for_test(domain: Domain) {
    drop(Guard::new(&STRUCTURE, &POPULARITY, domain));
}

/// Tests: domains of their own, so parallel tests never share a token.
#[cfg(any(test, feature = "test-support"))]
pub struct Watches {
    structure: WatchRegistry,
    popularity: WatchRegistry,
}

#[cfg(any(test, feature = "test-support"))]
impl Watches {
    /// Observed, as installed hooks would make them.
    pub fn enabled() -> Self {
        let watches = Self {
            structure: WatchRegistry::new(),
            popularity: WatchRegistry::new(),
        };
        watches.structure.set_enabled(true);
        watches.popularity.set_enabled(true);
        watches
    }

    /// A writer of `domain` runs and returns.
    pub fn write(&self, domain: Domain) {
        drop(Guard::new(&self.structure, &self.popularity, domain));
    }

    /// The daily update's forward on `status`, covering its call until dropped.
    pub fn daily(&self, status: usize) -> Guard<'_> {
        Guard::daily(&self.structure, &self.popularity, status)
    }

    /// `capital setter` on `country`, covering its call until dropped.
    pub fn capital(&self, country: usize) -> Guard<'_> {
        Guard::capital(&self.structure, country)
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    pub fn boundary(&self, invalidate: bool) {
        self.structure.boundary(invalidate);
        self.popularity.boundary(invalidate);
    }

    pub fn poisoned(&self) -> bool {
        self.structure.poisoned() && self.popularity.poisoned()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Changes for Watches {
    fn structure(&self) -> Option<(u64, u64)> {
        self.structure.quiet()
    }
    fn popularity(&self) -> Option<(u64, u64)> {
        self.popularity.quiet()
    }
    fn mismatch(&self) {
        self.structure.poison();
        self.popularity.poison();
    }
}

pub fn status() -> impl serde::Serialize {
    // Each domain's sites, with the shared sites forwarded to it.
    let events = |structure: bool| {
        let forwarded = FORWARDED.each_ref().map(|n| n.load(Ordering::Relaxed));
        crate::status::map(
            MUTATIONS
                .iter()
                .zip(&CALLS)
                .filter(move |((_, _, domain), _)| {
                    if structure {
                        domain.structure()
                    } else {
                        domain.popularity()
                    }
                })
                .map(|((site, _, _), n)| (site.name, n.load(Ordering::Relaxed)))
                .chain(structure.then(|| {
                    (
                        "set_capital_changes",
                        CAPITAL_CHANGES.load(Ordering::Relaxed),
                    )
                }))
                .chain([
                    ("reset_country", forwarded[0]),
                    if structure {
                        ("politics_daily_elections", forwarded[2])
                    } else {
                        ("politics_daily", forwarded[1])
                    },
                ]),
        )
    };
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "political_status_changes"),
        "verify": VERIFY,
        "observed": STRUCTURE.quiet().is_some() || STRUCTURE.active() != 0,
        "poisoned": STRUCTURE.poisoned() || POPULARITY.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "quiet_hours": reads[0],
        "popularity_refresh_hours": reads[1],
        "read_hours": reads[2],
        "verified_countries": reads[3],
        "mismatches": reads[4],
        "structure_events": events(true),
        "popularity_events": events(false),
        "scope": "cumulative since attach; the structure token stands still while no capital, last election, ruling party or party or idea array writer runs, the popularity token while no party popularity writer runs (the daily update moves popularity every day, and the structure only when it changed a last election: politics_daily_elections; the capital setter only when it changed a capital: set_capital_changes); when the structure, party and party-leader tokens, every held leader's character certificate and the country census are those of the latest complete walk, the Politics walk carries Politics, PoliticalParties, ActiveIdeas and PartyLeaders whole without reading any country (quiet_hours), or, when only the popularity token moved, carries them with every held party's popularity gathered afresh in one batch (popularity_refresh_hours); otherwise it reads (read_hours); a carried hour reads its sampled slice of countries afresh (one in rotation_hours by country id) and verify mode reads every hour afresh, comparing each country with what a carried hour would publish (verified_countries, mismatches); the first mismatch disables both domains"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; the Politics walk
    /// then reads every hour.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "political_status_changes")
            && MUTATIONS.iter().all(|(site, _, _)| {
                engine::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            SET_POPULARITY.store(base + SET_POPULARITY_RVA, Ordering::Release);
            let callbacks: [usize; SITES] = [
                set_capital as *const () as usize,
                set_politics as *const () as usize,
                force_ruling_party as *const () as usize,
                set_ruling_party as *const () as usize,
                set_ideas as *const () as usize,
                add_popularity as *const () as usize,
                set_popularity_redistributing as *const () as usize,
                set_political_party_popularity as *const () as usize,
                set_popularities as *const () as usize,
            ];
            for (index, (site, patch, _)) in MUTATIONS.iter().enumerate() {
                let hook = match patch {
                    Patch::Entry => unsafe {
                        Hook::new(base + site.rva, callbacks[index], site.sentinel)
                    },
                    Patch::Call => unsafe {
                        Hook::new_call(base + site.rva, callbacks[index], site.sentinel)
                    },
                }
                .map_err(|_| "native hook construction failed")?;
                if *patch == Patch::Entry {
                    ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                }
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            OWN.store(false, Ordering::SeqCst);
            refresh();
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::PoliticalStatus,
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
        OWN.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        refresh();
        // The stability and research inputs observe these writers too.
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::PoliticalStatus,
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
        // SAFETY: the domains hold no table; release only starts an epoch
        // once the hooks were removed and drained.
        unsafe {
            STRUCTURE.release();
            POPULARITY.release();
        }
    }
}

// Each entry site takes at most five integer or pointer arguments (RCX..R9
// and one stack word; politics setter takes five) and returns RAX unchanged from
// its original. The notifications cover the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
            fifth: usize,
        ) -> usize {
            let timing =
                crate::hook_timing::enter(crate::hook_timing::Family::PoliticalStatusChanges);
            let _notification = notify($site, first);
            let _inputs = crate::country_input_changes::forwarded_status($site, first);
            let original: unsafe extern "system" fn(usize, usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth, fifth) })
        }
    };
}
// Set capital (country, state, preserve): structure,
// when a capital changed.
callback!(set_capital, 0);
// Set politics (status, party, allowed, frequency,
// date): structure.
callback!(set_politics, 1);
// Force ruling party (status, ideology group): the ruling party store and
// popularity through the clamped setter, so both.
callback!(force_ruling_party, 2);
// Set ruling party (status, group):
// structure.
callback!(set_ruling_party, 3);
// Set ideas (status, removed, added, flags): structure.
callback!(set_ideas, 4);
// Add popularity (party, amount): popularity.
callback!(add_popularity, 5);
// Set popularity redistributing (party, popularity): popularity.
callback!(set_popularity_redistributing, 6);

// A replaced CALL of the clamped setter (party, value): the detour
// calls it itself, which returns to the site's next instruction through the
// relay. Popularity.
macro_rules! call_site {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(party: usize, value: usize) -> usize {
            let timing =
                crate::hook_timing::enter(crate::hook_timing::Family::PoliticalStatusChanges);
            let set: unsafe extern "system" fn(usize, usize) -> usize =
                unsafe { std::mem::transmute(SET_POPULARITY.load(Ordering::Acquire)) };
            let _notification = notify($site, party);
            let _inputs = crate::country_input_changes::forwarded_status($site, party);
            timing.original(|| unsafe { set(party, value) })
        }
    };
}
// The party popularity effect's setter call.
call_site!(set_political_party_popularity, 7);
// The popularities effect's setter call.
call_site!(set_popularities, 8);

#[cfg(test)]
mod tests {
    use super::*;

    /// A daily update whose call leaves the last election alone moves only
    /// popularity; one that writes it on election day moves the structure on
    /// return; one whose status cannot be read covers its call in both.
    #[test]
    fn the_daily_forward_moves_the_structure_only_when_the_last_election_changes() {
        let domains = Watches::enabled();
        let mut status = vec![0_u8; 0x100].into_boxed_slice();
        let at = cpolitical_status::LAST_ELECTION;
        status[at..at + 4].copy_from_slice(&1_000_i32.to_le_bytes());
        let address = status.as_ptr() as usize;
        let before = (domains.structure(), domains.popularity());
        let guard = domains.daily(address);
        // Popularity covers the call in flight; the structure stays quiet.
        assert_eq!(domains.popularity(), None);
        assert_eq!(domains.structure(), before.0);
        drop(guard);
        assert_eq!(
            domains.structure(),
            before.0,
            "no election: structure quiet"
        );
        assert!(domains.popularity().is_some());
        assert_ne!(domains.popularity(), before.1);

        let quiet = domains.structure();
        let guard = domains.daily(address);
        // The election-day last-election store.
        status[at..at + 4].copy_from_slice(&1_024_i32.to_le_bytes());
        assert_eq!(domains.structure(), quiet, "the store is seen on return");
        drop(guard);
        assert!(domains.structure().is_some());
        assert_ne!(
            domains.structure(),
            quiet,
            "an election moves the structure"
        );

        let quiet = domains.structure();
        let guard = domains.daily(0x10);
        assert_eq!(
            domains.structure(),
            None,
            "an unreadable status covers the call"
        );
        drop(guard);
        assert_ne!(domains.structure(), quiet);
        drop(status);
    }

    /// A capital setter call that leaves both capitals alone moves nothing;
    /// one that changes either moves the structure on return; one whose
    /// country cannot be read covers its call.
    #[test]
    fn the_capital_setter_moves_the_structure_only_when_a_capital_changes() {
        let domains = Watches::enabled();
        let mut country = vec![0_u8; ccountry::ORIGINAL_CAPITAL + 4].into_boxed_slice();
        let address = country.as_ptr() as usize;
        let quiet = (domains.structure(), domains.popularity());
        // The occupation path re-sets the capital it already has.
        let guard = domains.capital(address);
        assert_eq!(domains.structure(), quiet.0);
        drop(guard);
        assert_eq!((domains.structure(), domains.popularity()), quiet);
        for at in [ccountry::CAPITAL, ccountry::ORIGINAL_CAPITAL] {
            let quiet = domains.structure();
            let guard = domains.capital(address);
            country[at..at + 4].copy_from_slice(&7_i32.to_le_bytes());
            assert_eq!(domains.structure(), quiet, "the store is seen on return");
            drop(guard);
            assert!(domains.structure().is_some());
            assert_ne!(domains.structure(), quiet, "a capital change moves it");
        }
        let quiet = domains.structure();
        let guard = domains.capital(0x10);
        assert_eq!(
            domains.structure(),
            None,
            "an unreadable country covers the call"
        );
        drop(guard);
        assert_ne!(domains.structure(), quiet);
        drop(country);
    }

    /// Each hooked site moves exactly its own domains.
    #[test]
    fn sites_move_their_own_domains() {
        let domains = Watches::enabled();
        for (site, _, domain) in &MUTATIONS {
            let before = (domains.structure(), domains.popularity());
            domains.write(*domain);
            let after = (domains.structure(), domains.popularity());
            assert_eq!(before.0 != after.0, domain.structure(), "{}", site.name);
            assert_eq!(before.1 != after.1, domain.popularity(), "{}", site.name);
        }
    }

    /// The hooks' domains: a writer moves the token. Other tests share them
    /// and may run a writer or poison them meanwhile, which only moves or
    /// withdraws the token, so only a change is asserted.
    #[test]
    fn a_writer_moves_the_hooks_domain() {
        enable_for_test();
        for domain in [Domain::Structure, Domain::Popularity] {
            let token = || match domain {
                Domain::Structure => Observer.structure(),
                _ => Observer.popularity(),
            };
            let before = token();
            write_for_test(domain);
            if before.is_some() {
                assert_ne!(token(), before);
            }
        }
    }
}
