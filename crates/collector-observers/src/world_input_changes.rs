//! Allocation-free notifications from the native writers of the tables read
//! every hour and carried between hours ([`engine::world_input_mutations`]):
//! ProvinceNames, NavalRegionAccess, the world tension tables (WorldTension,
//! ThreatSources, ThreatCountryAggregates) and DynamicModifiers. A detour only
//! notifies its domain, counts its site and calls the original; it never
//! changes game behaviour. The whole-table domains register no receivers: a
//! capture compares the token it took before its table's latest complete
//! read with this hour's, and equal tokens prove that no observed writer ran
//! in between ([`WatchRegistry::quiet`]). The dynamic modifier domain
//! notifies the CCountry owning the written container, and a country's
//! certificate joins that sequence to the naval tree family's
//! ([`Changes::country`]).
//!
//! Each table's capture holds its latest complete read in a [`Held`] with the
//! token taken before it. An hour whose token equals the held one publishes
//! the held rows without reading the game; any other hour reads afresh and
//! holds that read when the token stood still across it. The hour of the
//! sampled rotation that a domain's index names reads afresh even when quiet
//! and compares the read with the held rows; a difference is a write no hook
//! saw and disables that domain for the attachment. `world_input_verify`, on
//! by default during the family's burn-in, reads and compares every quiet
//! hour. Anything unobserved (disabled, poisoned, an epoch change, a running
//! writer) yields no token, and the capture reads afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
pub use engine::world_input_mutations::Domain;
use engine::{
    Memory,
    world_input_mutations::{DOMAINS, MUTATIONS},
};
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: [WatchRegistry; DOMAINS] = [const { WatchRegistry::new() }; DOMAINS];
static CALLS: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Per domain: quiet hours, read hours, compared hours and mismatches.
static READS: [[AtomicU64; 4]; DOMAINS] = [const { [const { AtomicU64::new(0) }; 4] }; DOMAINS];
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "a quiet hour's held ProvinceNames rows differ from a fresh read",
    "a quiet hour's held NavalRegionAccess rows differ from a fresh read",
    "a quiet hour's held world tension tables differ from a fresh read",
    "a carried country's DynamicModifiers rows differ from a fresh read",
]);
const NAMES: [&str; DOMAINS] = [
    "province_names",
    "region_access",
    "tension",
    "dynamic_modifiers",
];

/// Read and compare every quiet hour instead of carrying it (burn-in).
pub const VERIFY: bool = cfg!(feature = "world_input_verify");

/// A domain's quiet token: `(epoch, revision)`.
pub type Token = (u64, u64);

/// The domains as a capture consults them; tests substitute their own, so
/// parallel tests never share the hooks' tokens.
pub trait Changes: Sync {
    /// `domain`'s token while observed with no writer running.
    fn token(&self, domain: Domain) -> Option<Token>;
    /// A held table differed from a fresh read: `domain` certifies nothing
    /// for the rest of the attachment.
    fn mismatch(&self, domain: Domain);
    /// A CCountry's certificate of its DynamicModifiers rows, registering it:
    /// the dynamic modifier domain's sequence (add, renew, remove and the
    /// daily update) and the naval tree family's (`modifier recalculation`,
    /// which rewrites every entry's enabled byte and values, and
    /// `country appearance reset`, which clears them). `None` when either cannot
    /// be observed.
    fn country(&self, country: usize) -> Option<[(u64, u64); 2]>;
}

/// The hooks' domains.
pub struct Observer;
impl Changes for Observer {
    fn token(&self, domain: Domain) -> Option<Token> {
        WATCHES[domain as usize].quiet()
    }
    fn mismatch(&self, domain: Domain) {
        WATCHES[domain as usize].poison();
    }
    fn country(&self, country: usize) -> Option<[(u64, u64); 2]> {
        Some([
            WATCHES[Domain::DynamicModifiers as usize].sequence(country)?,
            crate::naval_tree_changes::sequence(country)?,
        ])
    }
}

/// One DynamicModifiers capture's countries: carried, read, compared with
/// their held rows and differing. A difference has already poisoned the
/// domain ([`Changes::mismatch`]).
pub fn record_countries(carried: u64, read: u64, compared: u64, mismatches: u64) {
    let reads = &READS[Domain::DynamicModifiers as usize];
    for (target, value) in reads.iter().zip([carried, read, compared, mismatches]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(compared, mismatches, Domain::DynamicModifiers as usize);
}

/// What a capture does this hour, decided before any read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The token taken before the hour's reads.
    token: Option<Token>,
    /// Publish the held table without reading.
    pub carry: bool,
    /// The read is the domain's sampled comparison of a quiet hour.
    compare: bool,
    /// The token stood still across the read ([`Plan::read`]).
    steady: bool,
}

impl Default for Plan {
    fn default() -> Self {
        Self::READ
    }
}

impl Plan {
    /// Read afresh and hold nothing: no store, or another capture holds it.
    pub const READ: Self = Self {
        token: None,
        carry: false,
        compare: false,
        steady: false,
    };

    /// Right after the hour's reads: whether no writer ran across them, so
    /// that the read may be held under the token taken before it.
    pub fn read(self, domain: Domain, changes: &(impl Changes + ?Sized)) -> Self {
        Self {
            steady: self.token.is_some() && changes.token(domain) == self.token,
            ..self
        }
    }
}

/// A table held whole between hours: its latest complete read and the token
/// taken before it.
pub struct Held<P> {
    token: Option<Token>,
    pub payload: P,
}

impl<P> Held<P> {
    pub const fn new(payload: P) -> Self {
        Self {
            token: None,
            payload,
        }
    }

    /// Before the hour's reads. A full hour (load, gap, recovery or a
    /// requested refresh) always reads.
    pub fn plan(
        &self,
        domain: Domain,
        changes: &(impl Changes + ?Sized),
        force_full: bool,
    ) -> Plan {
        let token = changes.token(domain);
        let quiet = !force_full && token.is_some() && self.token == token;
        let compare =
            quiet && crate::sampled_verify::current_burned_in(VERIFY).covers(domain as u64);
        Plan {
            token,
            carry: quiet && !compare,
            compare,
            steady: false,
        }
    }

    /// Whether a carrying `plan` still finds the table it planned to carry
    /// (a capture whose reads and publication run on different threads).
    pub fn holds(&self, plan: Plan) -> bool {
        plan.carry && self.token == plan.token
    }

    /// The held table a carrying plan publishes, counting the quiet hour.
    pub fn carried(&self, domain: Domain) -> &P {
        READS[domain as usize][0].fetch_add(1, Ordering::Relaxed);
        &self.payload
    }

    /// After a fresh read, with the plan [`Plan::read`] returned: `read` when
    /// it completed. A comparing plan compares it with the held table
    /// (`differs`); the read is then held (`hold`, off the game thread,
    /// false when its storage could not grow) when the token stood still
    /// across it, and nothing is held otherwise.
    pub fn settle(
        &mut self,
        domain: Domain,
        changes: &(impl Changes + ?Sized),
        plan: Plan,
        read: bool,
        differs: impl FnOnce(&P) -> bool,
        hold: impl FnOnce(&mut P) -> bool,
    ) {
        let compared = plan.compare && read;
        let mismatch = compared && differs(&self.payload);
        record(domain, compared, mismatch);
        if mismatch {
            changes.mismatch(domain);
        }
        self.token = None;
        if read && plan.steady && hold(&mut self.payload) {
            self.token = plan.token;
        }
    }
}

fn record(domain: Domain, compared: bool, mismatch: bool) {
    let reads = &READS[domain as usize];
    reads[1].fetch_add(1, Ordering::Relaxed);
    reads[2].fetch_add(u64::from(compared), Ordering::Relaxed);
    reads[3].fetch_add(u64::from(mismatch), Ordering::Relaxed);
    if compared {
        SAMPLED.record(1, u64::from(mismatch), domain as usize);
    }
}

/// Replace `held` with `fresh`, worker-only; false when storage could not grow.
pub fn assign<T: Copy>(held: &mut Vec<T>, fresh: &[T]) -> bool {
    held.clear();
    if engine::allocation::reserve(held, fresh.len()).is_err() {
        return false;
    }
    held.extend_from_slice(fresh);
    true
}

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.iter().map(WatchRegistry::active).sum()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery.
pub fn boundary(invalidate: bool) {
    for watches in &WATCHES {
        watches.boundary(invalidate);
    }
}

/// Off the game thread: size the dynamic modifier watches for `slots`
/// country slots.
pub fn presize(slots: usize) {
    WATCHES[Domain::DynamicModifiers as usize].offer(slots);
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    for watches in &WATCHES {
        watches.collect();
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&CALLS)
            .map(|((site, _), n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let domains = crate::status::map(NAMES.iter().zip(&READS).zip(&WATCHES).map(
        |((&name, reads), watches)| {
            let reads = reads.each_ref().map(|value| value.load(Ordering::Relaxed));
            (
                name,
                crate::status::status!({
                    "observed": watches.quiet().is_some() || watches.active() != 0,
                    "poisoned": watches.poisoned(),
                    "quiet": reads[0],
                    "read": reads[1],
                    "compared": reads[2],
                    "mismatches": reads[3],
                }),
            )
        },
    ));
    crate::status::status!({
        "configured": cfg!(feature = "world_input_changes"),
        "verify": VERIFY,
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "domains": domains,
        "events": events,
        "watches": WATCHES[Domain::DynamicModifiers as usize].status(),
        "scope": "cumulative since attach; a whole-table domain's token stands still while none of its writers runs (province_names: the rename and reset effects; region_access: region access setter and the navy reset; tension: update total, which every source writer ends in); an hour whose token equals the one taken before the table's latest complete read publishes that read (quiet, in hours), any other hour reads afresh (read); the domain's hour of the rotation (its index in rotation_hours) reads a quiet hour afresh and compares it (compared), as verify mode does every quiet hour; dynamic_modifiers counts countries instead: a country whose certificate (the add, renew, remove and daily update sites naming its container, and the naval tree family's modifier recalculation and country appearance reset) is the held read's is carried (quiet), and each country is read afresh and compared on its day of the rotation by id; the first mismatch disables that domain"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; every capture then
    /// reads afresh each hour.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "world_input_changes")
            && MUTATIONS.iter().all(|(site, _)| {
                engine::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                rename_province as *const () as usize,
                reset_province_name as *const () as usize,
                set_region_access_modes as *const () as usize,
                reset_navy as *const () as usize,
                update_threat_total as *const () as usize,
                add_dynamic_modifier as *const () as usize,
                renew_dynamic_modifier as *const () as usize,
                remove_dynamic_modifier as *const () as usize,
                dynamic_modifier_daily_update as *const () as usize,
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
            for watches in &WATCHES {
                watches.set_enabled(false);
            }
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
        for watches in &WATCHES {
            watches.set_enabled(enabled && !self.hooks.is_empty());
        }
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
        for watches in &WATCHES {
            unsafe { watches.release() };
        }
    }
}

/// Hook path: a writer of `site` ([`MUTATIONS`] index) is about to run;
/// `first` is its first argument.
fn notify(site: usize, first: usize) -> Notification<'static> {
    CALLS[site].fetch_add(1, Ordering::Relaxed);
    let domain = MUTATIONS[site].1;
    let receiver = match domain {
        // The container the writer is about to write names its country (a
        // state's names none). Only its constructor writes the link.
        Domain::DynamicModifiers => unsafe {
            std::ptr::read_volatile(
                first.wrapping_add(engine::world_input_mutations::CONTAINER_COUNTRY)
                    as *const usize,
            )
        },
        _ => 0,
    };
    WATCHES[domain as usize].notify(receiver)
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
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
            let _notification = notify($site, first);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
// Rename province (effect).
callback!(rename_province, 0);
// Reset province name (effect, scope).
callback!(reset_province_name, 1);
// Set region access modes (navy, changes).
callback!(set_region_access_modes, 2);
// Strategic navy reset (navy).
callback!(reset_navy, 3);
// Update threat total (manager).
callback!(update_threat_total, 4);
// Add dynamic modifier (container, definition, scope, days).
callback!(add_dynamic_modifier, 5);
// Renew dynamic modifier (container, definition, scope, days).
callback!(renew_dynamic_modifier, 6);
// Remove dynamic modifier (container, definition, scope).
callback!(remove_dynamic_modifier, 7);
// Dynamic modifier daily update (container).
callback!(dynamic_modifier_daily_update, 8);

/// Tests: domains of one test's own.
#[cfg(any(test, feature = "test-support"))]
pub struct Isolated {
    pub watches: [WatchRegistry; DOMAINS],
}

#[cfg(any(test, feature = "test-support"))]
impl Default for Isolated {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Isolated {
    /// Observed, as installed hooks would be.
    pub fn new() -> Self {
        let isolated = Self {
            watches: [const { WatchRegistry::new() }; DOMAINS],
        };
        for watches in &isolated.watches {
            watches.offer(64);
            watches.boundary(false);
            watches.set_enabled(true);
        }
        isolated
    }

    /// A writer of `domain` runs and returns.
    pub fn write(&self, domain: Domain) {
        drop(self.watches[domain as usize].notify(0));
    }

    /// A dynamic modifier writer of `country`'s container runs and returns.
    pub fn write_country(&self, country: usize) {
        drop(self.watches[Domain::DynamicModifiers as usize].notify(country));
    }

    pub fn poisoned(&self, domain: Domain) -> bool {
        self.watches[domain as usize].poisoned()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Changes for Isolated {
    fn token(&self, domain: Domain) -> Option<Token> {
        self.watches[domain as usize].quiet()
    }
    fn mismatch(&self, domain: Domain) {
        self.watches[domain as usize].poison();
    }
    /// The naval tree half is held still.
    fn country(&self, country: usize) -> Option<[(u64, u64); 2]> {
        Some([
            self.watches[Domain::DynamicModifiers as usize].sequence(country)?,
            (1, 0),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callbacks_follow_the_site_order() {
        assert_eq!(
            MUTATIONS.map(|(site, _)| site.name),
            [
                "rename_province",
                "reset_province_name",
                "set_region_access_modes",
                "reset_navy",
                "update_threat_total",
                "add_dynamic_modifier",
                "renew_dynamic_modifier",
                "remove_dynamic_modifier",
                "dynamic_modifier_daily_update",
            ]
        );
    }

    /// A quiet hour carries the held read; a write, a full hour or an
    /// unobserved domain reads afresh, and a read the token did not stand
    /// still across is not held.
    #[test]
    fn a_held_table_is_carried_only_while_its_domain_is_quiet() {
        // Not the domain's comparison hour (its index, 0).
        crate::sampled_verify::begin_hour(5);
        let changes = Isolated::new();
        let domain = Domain::ProvinceNames;
        let mut held = Held::new(Vec::<u32>::new());
        let read = |held: &mut Held<Vec<u32>>, plan: Plan, rows: &[u32]| {
            held.settle(
                domain,
                &changes,
                plan.read(domain, &changes),
                true,
                |held| held != rows,
                |held| assign(held, rows),
            )
        };
        let plan = held.plan(domain, &changes, false);
        assert!(!plan.carry, "nothing is held yet");
        read(&mut held, plan, &[1, 2]);
        assert!(held.plan(domain, &changes, false).carry || VERIFY);
        assert!(!held.plan(domain, &changes, true).carry);
        assert_eq!(held.carried(domain), &[1, 2]);
        changes.write(domain);
        let plan = held.plan(domain, &changes, false);
        assert!(!plan.carry);
        // A writer runs during the read: the read is published, not held.
        changes.write(domain);
        read(&mut held, plan, &[3]);
        assert!(!held.plan(domain, &changes, false).carry);
        let plan = held.plan(domain, &changes, false);
        read(&mut held, plan, &[3]);
        assert!(held.plan(domain, &changes, false).carry || VERIFY);
        // Another domain's writer leaves this one quiet.
        changes.write(Domain::Tension);
        assert!(held.plan(domain, &changes, false).carry || VERIFY);
        // A capture whose read did not complete holds nothing.
        let plan = held.plan(domain, &changes, false);
        held.settle(domain, &changes, plan, false, |_| false, |_| true);
        assert!(!held.plan(domain, &changes, false).carry);
    }

    /// The domain's rotation hour reads a quiet hour afresh and compares it;
    /// a difference poisons the domain, which then never carries.
    #[test]
    fn a_quiet_hour_that_differs_from_the_held_read_poisons_its_domain() {
        let changes = Isolated::new();
        let domain = Domain::RegionAccess;
        let mut held = Held::new(Vec::<u32>::new());
        let plan = held.plan(domain, &changes, false);
        held.settle(
            domain,
            &changes,
            plan.read(domain, &changes),
            true,
            |_| false,
            |held| assign(held, &[7]),
        );
        crate::sampled_verify::begin_hour(domain as i32);
        let plan = held.plan(domain, &changes, false);
        assert!(!plan.carry && plan.compare);
        held.settle(
            domain,
            &changes,
            plan.read(domain, &changes),
            true,
            |held| held != &[8],
            |held| assign(held, &[8]),
        );
        assert!(changes.poisoned(domain));
        assert_eq!(changes.token(domain), None);
        assert!(!held.plan(domain, &changes, false).carry);
    }
}
