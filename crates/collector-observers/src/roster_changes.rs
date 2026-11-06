//! Allocation-free notifications from the native writers of the character
//! roster and of scientist roles ([`engine_mutations::character_roster_mutations`]).
//! A detour only moves its domain's quiet token, counts its site and calls
//! the original; it never changes game behaviour. Neither domain registers
//! receivers: a capture compares the token it took before its latest complete
//! read with this hour's, and equal tokens prove that no observed writer ran
//! in between ([`WatchRegistry::quiet`]).
//!
//! - The roster domain covers the `CCharacterManager` arrays every roster
//!   walk visits, so the roster's occurrences, their order and their ids
//!   stand still while its token does: its own sites (push, clear), the
//!   manager constructor the advisor-pool family patches and forwards
//!   ([`manager_constructed`]), and `repair_containers`, the only writer of a
//!   rostered character's id, which the advisor family patches and forwards
//!   ([`character_repaired`]). It observes only while both forwarders are
//!   installed and enabled.
//! - The scientist domain covers every scientist's trait array and the
//!   creation, replacement and removal of scientist roles.
//!
//! Anything unobserved (disabled, poisoned, an epoch change, a running
//! writer) yields no token, and the capture reads afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{character_roster_mutations::Domain, character_roster_mutations::MUTATIONS};
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static ROSTER: WatchRegistry = WatchRegistry::new();
static SCIENTISTS: WatchRegistry = WatchRegistry::new();
static CALLS: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
/// Forwarded calls: the manager constructor, then `repair_containers`.
static FORWARDED: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// This family's own patches are enabled.
static OWN: AtomicBool = AtomicBool::new(false);
/// The advisor-pool family forwards the manager constructor.
static MANAGER_FORWARD: AtomicBool = AtomicBool::new(false);
/// The advisor family forwards `repair_containers`.
static REPAIR_FORWARD: AtomicBool = AtomicBool::new(false);

/// Per table: quiet hours (carried without the walk) and full reads.
static READS: [[AtomicU64; 2]; TABLES.len()] =
    [const { [const { AtomicU64::new(0) }; 2] }; TABLES.len()];

/// The captures that rely on these domains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    Characters,
    ScriptFlags,
    CharacterRoleTraits,
}
const TABLES: [&str; 3] = ["Characters", "ScriptFlags", "CharacterRoleTraits"];

/// The roster domain's token: `(epoch, revision)` while observed with no
/// writer running.
pub fn roster() -> Option<(u64, u64)> {
    ROSTER.quiet()
}

/// The scientist domain's token.
pub fn scientists() -> Option<(u64, u64)> {
    SCIENTISTS.quiet()
}

/// One capture's use of a domain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Carried from a quiet domain without the walk.
    pub quiet: bool,
    /// Read afresh.
    pub read: bool,
}

/// The roster domain as a capture consults it; tests substitute a domain of
/// their own, so parallel tests never share the hooks' token.
pub trait Roster: Sync {
    /// [`roster`].
    fn token(&self) -> Option<(u64, u64)>;
    /// [`record`].
    fn record(&self, table: Table, counts: &Counts);
}

/// The hooks' domain. Tests use domains of their own.
#[cfg_attr(any(test, feature = "test-support"), allow(dead_code))]
pub struct Observer;
impl Roster for Observer {
    fn token(&self) -> Option<(u64, u64)> {
        roster()
    }
    fn record(&self, table: Table, counts: &Counts) {
        record(table, counts);
    }
}

/// Tests: a roster domain of one test's own, which counts what its captures
/// record.
#[cfg(any(test, feature = "test-support"))]
pub struct Isolated {
    pub registry: WatchRegistry,
    /// Quiet hours and read hours.
    counts: [AtomicU64; 2],
}

#[cfg(any(test, feature = "test-support"))]
impl Default for Isolated {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Isolated {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::new(),
            counts: [const { AtomicU64::new(0) }; 2],
        }
    }

    /// Observe, as installed hooks would.
    pub fn enable(&self) {
        self.registry.set_enabled(true);
    }

    /// A roster writer runs and returns.
    pub fn write(&self) {
        drop(self.registry.notify(0));
    }

    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }

    /// Quiet hours and read hours recorded.
    pub fn counts(&self) -> [u64; 2] {
        self.counts
            .each_ref()
            .map(|count| count.load(Ordering::Relaxed))
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Roster for Isolated {
    fn token(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn record(&self, _table: Table, counts: &Counts) {
        for (target, value) in self
            .counts
            .iter()
            .zip([u64::from(counts.quiet), u64::from(counts.read)])
        {
            target.fetch_add(value, Ordering::Relaxed);
        }
    }
}

pub fn record(table: Table, counts: &Counts) {
    for (target, value) in READS[table as usize]
        .iter()
        .zip([u64::from(counts.quiet), u64::from(counts.read)])
    {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

/// Notifications running now.
pub fn active() -> usize {
    ROSTER.active() + SCIENTISTS.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery.
pub fn boundary(invalidate: bool) {
    ROSTER.boundary(invalidate);
    SCIENTISTS.boundary(invalidate);
}

fn refresh() {
    let own = OWN.load(Ordering::SeqCst);
    ROSTER.set_enabled(
        own && MANAGER_FORWARD.load(Ordering::SeqCst) && REPAIR_FORWARD.load(Ordering::SeqCst),
    );
    SCIENTISTS.set_enabled(own);
}

/// The advisor-pool family's patch of the manager constructor is enabled.
pub fn set_manager_forward(enabled: bool) {
    MANAGER_FORWARD.store(enabled, Ordering::SeqCst);
    refresh();
}

/// The advisor family's patch of `repair_containers` is enabled.
pub fn set_repair_forward(enabled: bool) {
    REPAIR_FORWARD.store(enabled, Ordering::SeqCst);
    refresh();
}

/// Hook path: the `CCharacterManager` constructor is about to
/// build a manager. Dropping the guard notifies its return.
pub fn manager_constructed() -> Notification<'static> {
    FORWARDED[0].fetch_add(1, Ordering::Relaxed);
    ROSTER.notify(0)
}

/// Hook path: character identity repair may give a rostered
/// character a new id.
pub fn character_repaired() -> Notification<'static> {
    FORWARDED[1].fetch_add(1, Ordering::Relaxed);
    ROSTER.notify(0)
}

/// Hook path: a writer of `site` ([`MUTATIONS`] index) is about to run.
fn notify(site: usize) -> Notification<'static> {
    CALLS[site].fetch_add(1, Ordering::Relaxed);
    match MUTATIONS[site].1 {
        Domain::Roster => ROSTER.notify(0),
        Domain::Scientists => SCIENTISTS.notify(0),
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&CALLS)
            .map(|((site, _), n)| (site.name, n.load(Ordering::Relaxed)))
            .chain(
                ["construct_character_manager", "character_repair"]
                    .into_iter()
                    .zip(&FORWARDED)
                    .map(|(name, n)| (name, n.load(Ordering::Relaxed))),
            ),
    );
    let tables = crate::status::map(TABLES.iter().zip(&READS).map(|(&name, reads)| {
        let reads = reads.each_ref().map(|value| value.load(Ordering::Relaxed));
        (
            name,
            crate::status::status!({
                "quiet_hours": reads[0],
                "read_hours": reads[1],
            }),
        )
    }));
    crate::status::status!({
        "configured": cfg!(feature = "roster_changes"),
        "roster_observed": ROSTER.quiet().is_some() || ROSTER.active() != 0,
        "scientists_observed": SCIENTISTS.quiet().is_some() || SCIENTISTS.active() != 0,
        "poisoned": ROSTER.poisoned() || SCIENTISTS.poisoned(),
        "tables": tables,
        "events": events,
        "scope": "cumulative since attach; the roster domain's token stands still while no character is appended to or cleared from the CCharacterManager arrays, no manager is built and no rostered character's id is repaired, the scientist domain's while no scientist role or trait array is written; Characters carries its rows and the role census's occurrences, and ScriptFlags its whole walk (with the flag domain and the country census), over a quiet roster hour, CharacterRoleTraits its scientist rows over a quiet scientist hour; quiet_hours counts hours carried so, read_hours hours read afresh"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; every capture then
    /// reads afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "roster_changes")
            && MUTATIONS.iter().all(|(site, _)| {
                crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                roster_push as *const () as usize,
                roster_clear as *const () as usize,
                scientist_build as *const () as usize,
                scientist_attach as *const () as usize,
                scientist_clear as *const () as usize,
                scientist_trait_add as *const () as usize,
                scientist_trait_remove as *const () as usize,
                scientist_trait_reset as *const () as usize,
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
            OWN.store(false, Ordering::SeqCst);
            refresh();
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Roster,
                false,
            );
            crate::census_changes::forwarder(crate::census_changes::Forwarder::Roster, false);
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
        // The research inputs observe the scientist roles here.
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Roster,
            enabled && !self.hooks.is_empty(),
        );
        // The role census observes the scientist attach and clear here.
        crate::census_changes::forwarder(
            crate::census_changes::Forwarder::Roster,
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
        // SAFETY: like each Hook's relay, the domains hold no table; release
        // only starts an epoch once the hooks were removed and drained.
        unsafe {
            ROSTER.release();
            SCIENTISTS.release();
        }
    }
}

// Each site takes at most four integer or pointer arguments (RCX..R9) and
// returns RAX unchanged from its original. The notification covers the
// original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::RosterChanges);
            let _notification = notify($site);
            let _inputs = crate::country_input_changes::forwarded_roster($site, first, second);
            let _census = match $site {
                sites::SCIENTIST_ATTACH => {
                    crate::census_changes::forwarded(crate::census_changes::Site::ScientistAttach)
                }
                sites::SCIENTIST_CLEAR => {
                    crate::census_changes::forwarded(crate::census_changes::Site::ScientistClear)
                }
                _ => None,
            };
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
use engine_mutations::character_roster_mutations as sites;
// Roster push (array, unique_ptr).
callback!(roster_push, sites::PUSH);
// Roster clear (array).
callback!(roster_clear, sites::CLEAR);
// Scientist build (out, character, definition, _).
callback!(scientist_build, sites::SCIENTIST_BUILD);
// Scientist attach (character).
callback!(scientist_attach, sites::SCIENTIST_ATTACH);
// Scientist clear (character).
callback!(scientist_clear, sites::SCIENTIST_CLEAR);
// Scientist trait add (scientist, trait).
callback!(scientist_trait_add, sites::SCIENTIST_TRAIT_ADD);
// Scientist trait remove (scientist, trait).
callback!(scientist_trait_remove, sites::SCIENTIST_TRAIT_REMOVE);
// Scientist trait reset (scientist, tokens).
callback!(scientist_trait_reset, sites::SCIENTIST_TRAIT_RESET);
