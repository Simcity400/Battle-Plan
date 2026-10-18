//! Allocation-free notifications from the native writers of the raid state
//! inputs and the building-resource overrides ([`engine_mutations::raid_mutations`]).
//! A detour only marks the id its arguments name in one of the family's
//! [`DirtySet`]s, counts its site and calls the original; it never changes
//! game behaviour.
//!
//! Three sets, each taken once an hour by one job; the first two by the
//! RaidState job ([`crate::extended::raid_state_inputs`]):
//! - `STATES` (state ids, `cstate::ID`): the state writers name their state
//!   directly, the pool method through its pool's offset, the origin
//!   constructor through `r8` and the origin destructor through the origin's own link; an
//!   origin without a state marks nothing. The building status writers,
//!   whose patches [`crate::building_changes`] owns, are forwarded
//!   ([`building_changed`], [`building_unnamed`]) for the infrastructure
//!   level: a state status marks its state.
//! - `COUNTRIES` (raw country links held by the country resources): the override
//!   writers; the consumer resolves each link to its country.
//! - `SITES` (raw country links), taken by the RaidLaunch job
//!   ([`crate::extended::raid_launch_inputs`]): the site list writers name
//!   their `CStrategicAir`'s country link or the link RDX points to; the
//!   initialization and load writers name every country; the capacity setter
//!   names the link R8 points to only when the value it stores differs from
//!   the one there. The daily air base capacity refresh, which calls the
//!   setter for every (base, country) pair, is read once before it runs: when every
//!   store keeps its value and no base is withdrawn, its setter calls skip
//!   their own read, since none would mark.
//!
//! - `ROUTES` (raw country links), taken by the RaidState job's resources map capture
//!   ([`crate::extended::raid_state_inputs`]): the route writers name the country holding
//!   the route, the route update its sender link and the constructors the link RDX points
//!   to (a route's sender, or a new country resources object's country, whose delivery
//!   routes it builds); the save loader names every country.
//!
//! A take that returns the epoch of the family's previous read certifies
//! that every state or country no take visited since then is unchanged; the
//! consumer then reads its daily slice and the marked owners, and anything
//! unobserved (disabled, poisoned, an id beyond the bitmap, an unnamed
//! writer, an epoch change, a running writer) makes it read every owner.
//! Entering nudge mode or an uninstalled building family poisons the state
//! set; it observes only while both its own hooks and that forwarder are
//! enabled.
use crate::{
    dirty_set::{Dirty, DirtySet},
    hook::Hook,
};
use engine_mutations::{
    raid_mutations::DAILY_CAPACITIES, raid_mutations::MUTATIONS, raid_mutations::Receiver,
};
use engine_native::Memory;
use std::{
    cell::Cell,
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
/// The country air pool capacity setter's [`MUTATIONS`] index.
const CAPACITY_SITE: usize = 24;
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// The daily air base capacity refresh's original.
static DAILY_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Daily pass originals in flight; one that never returns stays counted.
static DAILY_FLIGHTS: AtomicUsize = AtomicUsize::new(0);
std::thread_local! {
    /// The calling thread is inside a daily pass whose every capacity store
    /// was read unchanged before it ran ([`daily_refresh_base_capacities`]).
    static CERTIFIED_PASS: Cell<bool> = const { Cell::new(false) };
}
/// The module base the capacity setter's tag resolves against; stored before
/// any hook is enabled.
static BASE: AtomicUsize = AtomicUsize::new(0);
/// Per [`Family`] and [`Counts`] field, cumulative.
static READS: [[AtomicU64; 7]; 4] = [const { [const { AtomicU64::new(0) }; 7] }; 4];
/// The family's sampled comparisons; the reason index is the [`Family`].
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "an unmarked state's raid inputs differ from its previous read",
    "an unmarked country's building resource overrides differ from its previous read",
    "an unmarked country's raid air sites differ from its previous read",
    "an unmarked country's trade routes differ from its previous read",
]);
/// Ids of the latest complete census per [`Family`] (state slots, country
/// link ids), for off-thread sizing.
static IDS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
/// Calls of the strategic-air base and missile list clears, which
/// strategic-air reset makes before zeroing a country's RaidAirScores pairs in place,
/// and every change of the hooks' state ([`air_clear_token`]).
static AIR_CLEARS: AtomicU64 = AtomicU64::new(0);

/// What the two sets mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// The `raid_state` sliced family: StateResourceInputs, StateResources,
    /// StateDynamicModifiers and StateCores (cores, claims and strategic
    /// locations), by state id.
    States = 0,
    /// The `raid_country_buildings` sliced family: CountryBuildingResources,
    /// by country.
    Countries = 1,
    /// RaidAirSites in the `raid_launch` sliced family, by canonical slot.
    Sites = 2,
    /// TradeRoutes in the `trade_routes` sliced family, by the country holding each route.
    Routes = 3,
}

/// One observation domain: the two sets and each writer site's calls.
pub struct Watches {
    states: DirtySet,
    countries: DirtySet,
    sites: DirtySet,
    routes: DirtySet,
    calls: [AtomicU64; SITES],
    /// Receivers whose id did not read.
    unnamed: AtomicU64,
    /// Forwarded building writes of a state status, and those naming no
    /// status.
    buildings: [AtomicU64; 2],
    /// Capacity setter calls that stored the value already there, counted
    /// apart from `calls` so that their common path makes one increment.
    unchanged_capacities: AtomicU64,
    /// Daily passes whose stores were all certified unchanged before they
    /// ran, and those left to the setter's own path.
    daily_passes: [AtomicU64; 2],
    /// This family's hooks, and the building family's forwarder, enabled.
    hooks: AtomicBool,
    forwarder: AtomicBool,
}

/// What a site's arguments name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A state; `None` when its id does not read.
    State(Option<usize>),
    /// A raw country link; `None` when it does not read.
    Country(Option<usize>),
    /// The raw link of a country whose air sites change; `None` when it does
    /// not read.
    Site(Option<usize>),
    /// Every country's air sites.
    EverySite,
    /// The raw link of the country holding a route; `None` when it does not read.
    Route(Option<usize>),
    /// Every country's routes.
    EveryRoute,
    /// Nothing any row reads: an origin without a state, link zero.
    Nothing,
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            states: DirtySet::new(),
            countries: DirtySet::new(),
            sites: DirtySet::new(),
            routes: DirtySet::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
            unnamed: AtomicU64::new(0),
            buildings: [const { AtomicU64::new(0) }; 2],
            unchanged_capacities: AtomicU64::new(0),
            daily_passes: [const { AtomicU64::new(0) }; 2],
            hooks: AtomicBool::new(false),
            forwarder: AtomicBool::new(false),
        }
    }

    fn sets(&self) -> [&DirtySet; 4] {
        [&self.states, &self.countries, &self.sites, &self.routes]
    }

    fn mark<'a>(&'a self, set: &'a DirtySet, id: Option<usize>) -> Dirty<'a> {
        match id {
            Some(id) => set.notify(id),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                set.notify_every()
            }
        }
    }

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// whose arguments name `target` is about to run. Dropping the guard
    /// marks its return.
    pub fn notify(&self, site: usize, target: Target) -> Option<Dirty<'_>> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        match target {
            Target::State(id) => Some(self.mark(&self.states, id)),
            Target::Country(id) => Some(self.mark(&self.countries, id)),
            Target::Site(id) => Some(self.mark(&self.sites, id)),
            Target::EverySite => Some(self.sites.notify_every()),
            Target::Route(id) => Some(self.mark(&self.routes, id)),
            Target::EveryRoute => Some(self.routes.notify_every()),
            Target::Nothing => None,
        }
    }

    /// Hook path, allocation-free: the capacity setter `site` is about to
    /// store a value; `changed` when it differs from the one there (or that
    /// does not read). Only a changing store names and marks its country.
    pub fn capacity(
        &self,
        site: usize,
        changed: bool,
        target: impl FnOnce() -> Target,
    ) -> Option<Dirty<'_>> {
        if changed {
            return self.notify(site, target());
        }
        self.unchanged_capacities.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Calls of writer `site`, the capacity setter's unchanged stores
    /// included.
    fn site_calls(&self, site: usize) -> u64 {
        let calls = self.calls[site].load(Ordering::Relaxed);
        if site == CAPACITY_SITE {
            calls + self.unchanged_capacities.load(Ordering::Relaxed)
        } else {
            calls
        }
    }

    /// A forwarded building write of the status with `flags` and `location`:
    /// a state status marks its state, whose infrastructure building it may
    /// hold; a province status holds none a row reads.
    fn building(&self, flags: u32, location: u32) -> Option<Dirty<'_>> {
        if flags & engine_native::cbuilding_status::STATE_FLAG == 0 {
            return None;
        }
        self.buildings[0].fetch_add(1, Ordering::Relaxed);
        Some(self.mark(&self.states, (location != 0).then_some(location as usize)))
    }

    fn building_unnamed(&self) -> Dirty<'_> {
        self.buildings[1].fetch_add(1, Ordering::Relaxed);
        self.states.notify_every()
    }

    /// The state set observes only while both the hooks and the building
    /// forwarder are enabled, the country and site sets while the hooks are;
    /// enabling starts an epoch.
    fn apply(&self) {
        let hooks = self.hooks.load(Ordering::SeqCst);
        self.states
            .set_enabled(hooks && self.forwarder.load(Ordering::SeqCst));
        self.countries.set_enabled(hooks);
        self.sites.set_enabled(hooks);
        self.routes.set_enabled(hooks);
    }

    /// An enabled domain with bitmaps for `states` and `countries` ids (the
    /// site set takes the country size).
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(states: usize, countries: usize) -> Self {
        let watches = Self::new();
        watches.states.offer(states);
        watches.countries.offer(countries);
        watches.sites.offer(countries);
        watches.routes.offer(countries);
        for set in watches.sets() {
            set.boundary(false);
        }
        watches.hooks.store(true, Ordering::SeqCst);
        watches.forwarder.store(true, Ordering::SeqCst);
        watches.apply();
        watches
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        for set in self.sets() {
            set.boundary(invalidate);
        }
    }

    /// The building family's forwarder turned off or on.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_forwarder(&self, enabled: bool) {
        self.forward(enabled);
    }

    /// Only the state set depends on the building family's forwarder;
    /// enabling a set starts a new epoch, so the country set is left as it is.
    fn forward(&self, enabled: bool) {
        self.forwarder.store(enabled, Ordering::SeqCst);
        self.states
            .set_enabled(enabled && self.hooks.load(Ordering::SeqCst));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self, family: Family) -> bool {
        self.sets()[family as usize].poisoned()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn mark_building(&self, flags: u32, location: u32) -> Option<Dirty<'_>> {
        self.building(flags, location)
    }
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// The single consumer's hourly take of `family`'s marked ids
    /// ([`DirtySet::take`]); `None` certifies nothing.
    fn take(&self, family: Family, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    /// A fresh read disagreed with an unmarked owner's previous read:
    /// `family` certifies nothing for the rest of the attachment.
    fn mismatch(&self, family: Family);
}

impl Changes for Watches {
    fn take(&self, family: Family, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.sets()[family as usize].take(visit)
    }
    fn mismatch(&self, family: Family) {
        self.sets()[family as usize].poison();
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn take(&self, family: Family, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.take(family, visit)
    }
    fn mismatch(&self, family: Family) {
        WATCHES.mismatch(family);
    }
}

/// Whether every hour reads every owner and compares each unmarked one with
/// its previous read, rather than the hour's slice ([`crate::sampled_verify`]).
pub const VERIFY: bool = cfg!(feature = "raid_verify");

/// What a site's arguments name.
fn target(receiver: Receiver, first: usize, second: usize, third: usize) -> Target {
    // SAFETY: guarded reads of live memory on the calling thread.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    match receiver {
        Receiver::CountryResources => Target::Country(
            engine_mutations::raid_mutations::country(&memory, first).map(|raw| raw as usize),
        ),
        Receiver::StrategicAir | Receiver::CountryTag | Receiver::PoolCapacity => {
            match engine_mutations::raid_mutations::site_country(
                &memory, receiver, first, second, third,
            ) {
                Ok(Some(raw)) => Target::Site(Some(raw as usize)),
                Ok(None) => Target::Nothing,
                Err(engine_mutations::raid_mutations::Unreadable) => Target::Site(None),
            }
        }
        Receiver::EveryCountry => Target::EverySite,
        Receiver::Route | Receiver::RouteSender => {
            match engine_mutations::raid_mutations::route_country(&memory, receiver, first, second)
            {
                Ok(Some(raw)) => Target::Route(Some(raw as usize)),
                Ok(None) => Target::Nothing,
                Err(engine_mutations::raid_mutations::Unreadable) => Target::Route(None),
            }
        }
        Receiver::EveryRoute => Target::EveryRoute,
        _ => match engine_mutations::raid_mutations::state(&memory, receiver, first, third) {
            Ok(Some(id)) => Target::State(Some(id as usize)),
            Ok(None) => Target::Nothing,
            Err(engine_mutations::raid_mutations::Unreadable) => Target::State(None),
        },
    }
}

/// Notifications running now.
/// Sites 21 and 22: the strategic-air list clears.
fn note_air_clear(site: usize) {
    if site == 21 || site == 22 {
        AIR_CLEARS.fetch_add(1, Ordering::SeqCst);
    }
}

/// The token RaidAirScores carries its off-phase pairs under
/// ([`engine_raid_inputs::raid_air_scores::Held::certify`]): the count of in-place clears
/// while the hooks observe, `None` while they do not. Enabling or disabling
/// the hooks moves it, so a clear no hook saw cannot leave it unchanged.
pub fn air_clear_token() -> Option<u64> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(token) = TEST_AIR_CLEAR_TOKEN.with(std::cell::Cell::get) {
        return token;
    }
    WATCHES
        .hooks
        .load(Ordering::SeqCst)
        .then(|| AIR_CLEARS.load(Ordering::SeqCst))
}

#[cfg(any(test, feature = "test-support"))]
std::thread_local! {
    /// Tests carry under a fixed token unless they set another.
    pub static TEST_AIR_CLEAR_TOKEN: std::cell::Cell<Option<Option<u64>>> =
        const { std::cell::Cell::new(Some(Some(0))) };
}

pub fn active() -> usize {
    WATCHES.sets().iter().map(|set| set.active()).sum::<usize>()
        + DAILY_FLIGHTS.load(Ordering::SeqCst)
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    for set in WATCHES.sets() {
        set.boundary(invalidate);
    }
}

/// Disable the state set for the attachment: a writer it cannot observe may
/// run (nudge mode, or no building family to forward building writes).
pub fn poison() {
    WATCHES.states.poison();
}

/// The building family's forwarder enabled or disabled; outside attach and
/// detach only.
pub fn forwarder(enabled: bool) {
    WATCHES.forward(enabled);
}

/// Hook path, allocation-free: a building writer of the status with `flags` and `location`
/// (`cbuilding_status::FLAGS` and `ID`) is about to run. Marks the state of a state status, whose
/// infrastructure level the state inputs read. Hold the result over the native call.
pub fn building_changed(flags: u32, location: u32) -> Option<Dirty<'static>> {
    WATCHES.building(flags, location)
}

/// Hook path, allocation-free: a building writer whose status cannot be
/// named is about to run; every state is read.
pub fn building_unnamed() -> Dirty<'static> {
    WATCHES.building_unnamed()
}

/// Off the game thread: size each set for the ids the latest census saw.
/// Nothing is offered before a census: a later larger bitmap waits for an
/// invalidating hour, while the first installs at once.
pub fn presize() {
    for (set, ids) in WATCHES.sets().into_iter().zip(&IDS) {
        let ids = ids.load(Ordering::Relaxed);
        if ids != 0 {
            set.offer(ids);
        }
    }
}

/// Off the game thread: free replaced bitmaps.
pub fn collect() {
    for set in WATCHES.sets() {
        set.collect();
    }
}

/// One hour's use of a family's marks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Owners the take marked.
    pub marked: u64,
    /// Owners read: the slice, the marked, the new and the re-read.
    pub read: u64,
    /// Owners read only because a take marked them.
    pub early: u64,
    /// Every owner read: the take did not certify the previous read's epoch,
    /// or verify mode.
    pub full: bool,
    /// Unmarked owners compared with their previous read, and those that
    /// differed (each poisons the family).
    pub verified: u64,
    pub mismatches: u64,
}

/// A worker publishes one hour's counts and, after a complete census, the
/// ids it saw for sizing.
pub fn record(family: Family, counts: &Counts, ids: Option<usize>) {
    for (target, value) in READS[family as usize].iter().zip([
        1,
        counts.marked,
        counts.read,
        counts.early,
        u64::from(counts.full),
        counts.verified,
        counts.mismatches,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, family as usize);
    if let Some(ids) = ids {
        IDS[family as usize].store(ids, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .enumerate()
            .map(|(index, (site, _))| (site.name, WATCHES.site_calls(index))),
    );
    let family = |family: Family| {
        let reads = READS[family as usize]
            .each_ref()
            .map(|value| value.load(Ordering::Relaxed));
        crate::status::status!({
            "hours": reads[0],
            "marked": reads[1],
            "read_owners": reads[2],
            "early_owners": reads[3],
            "full_reads": reads[4],
            "verified_owners": reads[5],
            "mismatches": reads[6],
            "poisoned": WATCHES.sets()[family as usize].poisoned(),
            "set": WATCHES.sets()[family as usize].status(),
        })
    };
    crate::status::status!({
        "configured": cfg!(feature = "raid_changes"),
        "verify": VERIFY,
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "states": family(Family::States),
        "countries": family(Family::Countries),
        "sites": family(Family::Sites),
        "routes": family(Family::Routes),
        "events": events,
        "unnamed_receivers": WATCHES.unnamed.load(Ordering::Relaxed),
        "building_writes": WATCHES.buildings[0].load(Ordering::Relaxed),
        "unnamed_building_writes": WATCHES.buildings[1].load(Ordering::Relaxed),
        "unchanged_capacities": WATCHES.unchanged_capacities.load(Ordering::Relaxed),
        "certified_daily_passes": WATCHES.daily_passes[0].load(Ordering::Relaxed),
        "per_call_daily_passes": WATCHES.daily_passes[1].load(Ordering::Relaxed),
        "scope": "cumulative since attach; each hour the RaidState job reads its daily slice of states and countries, and the RaidLaunch job its daily slice of RaidAirSites countries, and every owner a hooked writer marked since the previous hour (early_owners; the capacity setter marks only a store that changes the value, unchanged_capacities counts the others; a daily capacity pass whose every store keeps its value and that withdraws no base is read once before it runs and its stores skip the setter's own read (certified_daily_passes), while any other pass leaves each store to the setter (per_call_daily_passes)), so the raid state rows, CountryBuildingResources and RaidAirSites equal the game's at every hour, except StateResourceInputs.supply_route, which follows the hourly supply rebuild and is read with its state; an hour whose take does not return the previous read's epoch (disabled, poisoned, an unnamed writer, an id beyond the bitmap, an invalidating hour) reads every owner (full_reads); every hour compares each unmarked owner it read (its slice; every owner in verify mode) with its previous read and disables the family at the first difference (sampled_*)"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; the raid state
    /// tables and RaidAirSites then read every owner every hour.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "raid_changes")
            && MUTATIONS.iter().all(|(site, _)| {
                engine_native::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            BASE.store(base, Ordering::Release);
            let callbacks: [usize; SITES] = [
                refresh_resource_modifier_gates as *const () as usize,
                add_building_dynamic_modifier as *const () as usize,
                remove_building_dynamic_modifier as *const () as usize,
                add_resource as *const () as usize,
                expire_timed_resource as *const () as usize,
                destroy_state as *const () as usize,
                add_core as *const () as usize,
                remove_core as *const () as usize,
                construct_resource_origin as *const () as usize,
                destroy_resource_origin as *const () as usize,
                add_building_resource_modifier as *const () as usize,
                clear_country_resources as *const () as usize,
                construct_strategic_air as *const () as usize,
                append_air_site as *const () as usize,
                append_missile_site as *const () as usize,
                refresh_air_base_access as *const () as usize,
                populate_air_sites as *const () as usize,
                repair_air_base_containers as *const () as usize,
                withdraw_air_site as *const () as usize,
                withdraw_missile_site as *const () as usize,
                transfer_air_base as *const () as usize,
                clear_air_sites as *const () as usize,
                clear_missile_sites as *const () as usize,
                destroy_strategic_air as *const () as usize,
                set_country_pool_capacity as *const () as usize,
                add_claim as *const () as usize,
                remove_claim as *const () as usize,
                add_state_strategic_location as *const () as usize,
                add_province_strategic_location as *const () as usize,
                update_trade_route as *const () as usize,
                construct_trade_route as *const () as usize,
                construct_country_resources as *const () as usize,
                load_trade_route as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
            // Without the daily pass's patch every store takes the setter's
            // own path.
            let daily = DAILY_CAPACITIES;
            if engine_native::read::<[u8; 16]>(memory, base + daily.rva) == Some(daily.sentinel) {
                let hook = unsafe {
                    Hook::new(
                        base + daily.rva,
                        daily_refresh_base_capacities as *const () as usize,
                        daily.sentinel,
                    )
                }
                .map_err(|_| "native hook construction failed")?;
                DAILY_ORIGINAL.store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            AIR_CLEARS.fetch_add(1, Ordering::SeqCst);
            WATCHES.hooks.store(false, Ordering::SeqCst);
            WATCHES.apply();
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Raid,
                false,
            );
            crate::air_base_carry_changes::forwarder(
                crate::air_base_carry_changes::Forwarder::Raid,
                false,
            );
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        let publication = if enabled {
            Hook::set_family_enabled_shared_prepared(&mut self.hooks, enabled, || {
                AIR_CLEARS.fetch_add(1, Ordering::SeqCst);
            })
        } else {
            Hook::set_family_enabled_shared(&mut self.hooks, enabled)
        };
        if !publication.map_err(|_| "native hook state change failed")? {
            return Ok(false);
        }
        WATCHES
            .hooks
            .store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        WATCHES.apply();
        // The country counter inputs observe the core insert and erase here
        // ([`crate::country_input_changes`]).
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Raid,
            enabled && !self.hooks.is_empty(),
        );
        // The air base registrations observe capacities only through this
        // family's capacity patch.
        crate::air_base_carry_changes::forwarder(
            crate::air_base_carry_changes::Forwarder::Raid,
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
        // SAFETY: like each Hook's relay, the bitmaps are dropped only once
        // the hooks were never enabled, or were removed and drained after
        // the last capture; the building family's forwarders are restored
        // with the other families before any hook is dropped.
        for set in WATCHES.sets() {
            unsafe { set.release() };
        }
    }
}

// Each site takes at most four integer or pointer arguments (RCX..R9) and
// returns RAX unchanged from its original; the first three name the receiver
// as its [`Receiver`] says. The mark covers the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::RaidChanges);
            let _mark = WATCHES.notify($site, target(MUTATIONS[$site].1, first, second, third));
            // The core insert and erase also write the tag's core states,
            // which the country counter inputs observe through this patch.
            let _inputs = crate::country_input_changes::forwarded_raid($site, first, second);
            note_air_clear($site);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
// Refresh resource modifier gates (state).
callback!(refresh_resource_modifier_gates, 0);
// Building dynamic modifier add (state, ...).
callback!(add_building_dynamic_modifier, 1);
// Building dynamic modifier remove (state, ...).
callback!(remove_building_dynamic_modifier, 2);
// Add_resource (state, amount, days).
callback!(add_resource, 3);
// Pool method (the state's resource pool, amount).
callback!(expire_timed_resource, 4);
// Destroy state (state).
callback!(destroy_state, 5);
// Core insert (state, tag).
callback!(add_core, 6);
// Core erase (state, tag).
callback!(remove_core, 7);
// Construct resource origin (origin, tag, state).
callback!(construct_resource_origin, 8);
// Resource origin destruction(origin).
callback!(destroy_resource_origin, 9);
// Add building resource modifier (resources, ...).
callback!(add_building_resource_modifier, 10);
// Country resource clearing (resources).
callback!(clear_country_resources, 11);
// Strategic-air construction(air, tag, manager).
callback!(construct_strategic_air, 12);
// Air base country-container creation  (air, base).
callback!(append_air_site, 13);
// Append missile site (air, base).
callback!(append_missile_site, 14);
// Air base access refresh  (air, countries).
callback!(refresh_air_base_access, 15);
// Initial population (manager).
callback!(populate_air_sites, 16);
// Repair containers  (base).
callback!(repair_air_base_containers, 17);
// Air base withdrawal  (air, base).
callback!(withdraw_air_site, 18);
// Withdraw missile site (air, base).
callback!(withdraw_missile_site, 19);
// Host transfer (manager, from, to, base).
callback!(transfer_air_base, 20);
// Air site list clear (air).
callback!(clear_air_sites, 21);
// Missile site list clear (air).
callback!(clear_missile_sites, 22);
// Strategic-air destruction(air).
callback!(destroy_strategic_air, 23);
// Claim insert (state, tag).
callback!(add_claim, 25);
// Claim erase (state, tag).
callback!(remove_claim, 26);
// State strategic-location append (state, location, anchor province).
callback!(add_state_strategic_location, 27);
// Province strategic-location append (province, location): marks its state.
callback!(add_province_strategic_location, 28);
// Route update (route, from, to, force).
callback!(update_trade_route, 29);
// Route construction (route, &sender, &receiver).
callback!(construct_trade_route, 30);
// Country resources construction (resources, &country): its delivery routes.
callback!(construct_country_resources, 31);
// Route save loading (route, reader, token).
callback!(load_trade_route, 32);

/// `country air pool capacity setter(base, capacity, tag)` stores `capacity` unchanged at the
/// slot's container capacity (the daily pass stores most containers' value again): only a store
/// that differs from the value there, or one whose container does not read, marks the tag's
/// country, and is forwarded to [`crate::air_base_carry_changes`], whose registrations record the
/// capacity.
unsafe extern "system" fn set_country_pool_capacity(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RaidChanges);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[CAPACITY_SITE].load(Ordering::Acquire)) };
    if CERTIFIED_PASS.get() {
        // A store of a certified daily pass keeps its value, as the pass
        // counted.
        return timing.original(|| unsafe { original(first, second, third, fourth) });
    }
    // SAFETY: guarded reads of live memory on the calling thread.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let held = engine_mutations::raid_mutations::pool_capacity(
        &memory,
        BASE.load(Ordering::Relaxed),
        first,
        third,
    );
    let changed = held != Some(second as i64);
    let _mark = WATCHES.capacity(CAPACITY_SITE, changed, || {
        target(MUTATIONS[CAPACITY_SITE].1, first, second, third)
    });
    let _carry = changed.then(|| crate::air_base_carry_changes::capacity_written(first));
    timing.original(|| unsafe { original(first, second, third, fourth) })
}

/// `daily air base capacity refresh(air, pairs)`, the
/// daily pass that calls the capacity setter for every (base, country) pair.
/// When each of its stores keeps the capacity already there and it withdraws
/// no base ([`engine_mutations::raid_mutations::daily_unchanged`]), the pass changes
/// nothing the setter reads, so every one of its setter calls would find its
/// store unchanged and mark nothing: they are counted here and run the
/// original without reading. Any other pass, or one whose words do not read,
/// leaves every store to the setter's own path, which marks a changing store
/// as before.
///
/// Only a pass inside the hourly original is certified: there it runs from
/// daily strategic-air update's serial loop after the
/// manager's TBB stages join, on the thread the hourly detour holds. The
/// Same pass reached from game session initialization  (through
/// daily game update) takes the per-call path. The original
/// is a counted flight ([`active`]) on every path.
unsafe extern "system" fn daily_refresh_base_capacities(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RaidChanges);
    DAILY_FLIGHTS.fetch_add(1, Ordering::SeqCst);
    // SAFETY: guarded reads of live memory on the calling thread.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let unchanged = collector_extended_support::hourly::ticking()
        .then(|| {
            engine_mutations::raid_mutations::daily_unchanged(
                &memory,
                BASE.load(Ordering::Relaxed),
                first,
                second,
            )
        })
        .flatten();
    WATCHES.daily_passes[usize::from(unchanged.is_none())].fetch_add(1, Ordering::Relaxed);
    let _pass = unchanged.map(|stores| {
        WATCHES
            .unchanged_capacities
            .fetch_add(stores, Ordering::Relaxed);
        CertifiedPass::enter()
    });
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(DAILY_ORIGINAL.load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    // Only a normal return retires the flight.
    DAILY_FLIGHTS.fetch_sub(1, Ordering::SeqCst);
    result
}

/// The calling thread inside a certified daily pass, until it returns; the
/// enclosing state is restored.
struct CertifiedPass(bool);
impl CertifiedPass {
    fn enter() -> Self {
        Self(CERTIFIED_PASS.replace(true))
    }
}
impl Drop for CertifiedPass {
    fn drop(&mut self) {
        CERTIFIED_PASS.set(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests and benches driving the global detours take this first.
    static DAILY_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn take(watches: &Watches, family: Family) -> (Option<u64>, Vec<usize>) {
        let mut ids = Vec::new();
        let epoch = watches.take(family, &mut |id| ids.push(id));
        (epoch, ids)
    }

    fn site(name: &str) -> usize {
        MUTATIONS
            .iter()
            .position(|(site, _)| site.name == name)
            .unwrap()
    }

    /// Each receiver names its state or country through live memory: a state's id, the pool's
    /// state, the origin constructor's `r8`, the origin's own link and the resources' country link;
    /// an origin without a state names nothing, and an unreadable receiver names none.
    #[test]
    fn receivers_name_their_state_or_country() {
        let mut state = vec![0_u8; 0x400];
        state[engine_native::cstate::ID..][..4].copy_from_slice(&42_u32.to_le_bytes());
        let state_at = state.as_ptr() as usize;
        let mut origin = vec![0_u8; 0x100];
        origin[engine_resources::cresource_origin::STATE..][..8]
            .copy_from_slice(&(state_at as u64).to_le_bytes());
        let mut resources = vec![0_u8; 0x100];
        resources[engine_resources::ccountry_resources::OWNER..][..4]
            .copy_from_slice(&9_u32.to_le_bytes());
        assert_eq!(
            target(Receiver::State, state_at, 0, 0),
            Target::State(Some(42))
        );
        assert_eq!(
            target(
                Receiver::StatePool,
                state_at + engine_native::cstate::RESOURCE_POOL,
                0,
                0
            ),
            Target::State(Some(42))
        );
        assert_eq!(
            target(Receiver::OriginState, 0, 0, state_at),
            Target::State(Some(42))
        );
        assert_eq!(target(Receiver::OriginState, 0, 0, 0), Target::Nothing);
        assert_eq!(
            target(Receiver::Origin, origin.as_ptr() as usize, 0, 0),
            Target::State(Some(42))
        );
        origin[engine_resources::cresource_origin::STATE..][..8]
            .copy_from_slice(&0_u64.to_le_bytes());
        assert_eq!(
            target(Receiver::Origin, origin.as_ptr() as usize, 0, 0),
            Target::Nothing
        );
        assert_eq!(
            target(
                Receiver::CountryResources,
                resources.as_ptr() as usize,
                0,
                0
            ),
            Target::Country(Some(9))
        );
        assert_eq!(target(Receiver::State, 0, 0, 0), Target::State(None));
        assert_eq!(
            target(Receiver::CountryResources, 0, 0, 0),
            Target::Country(None)
        );
        // A site list writer names its strategic air object's link, the
        // link RDX or R8 points to, or every country; link zero names none.
        let mut air = vec![0_u8; 0x100];
        air[engine_native::cstrategic_air::COUNTRY..][..4].copy_from_slice(&7_u32.to_le_bytes());
        let tag = 11_u32.to_le_bytes();
        let zero = 0_u32.to_le_bytes();
        assert_eq!(
            target(Receiver::StrategicAir, air.as_ptr() as usize, 0, 0),
            Target::Site(Some(7))
        );
        assert_eq!(
            target(Receiver::CountryTag, 0, tag.as_ptr() as usize, 0),
            Target::Site(Some(11))
        );
        assert_eq!(
            target(Receiver::PoolCapacity, 0, 0, tag.as_ptr() as usize),
            Target::Site(Some(11))
        );
        assert_eq!(
            target(Receiver::CountryTag, 0, zero.as_ptr() as usize, 0),
            Target::Nothing
        );
        assert_eq!(target(Receiver::StrategicAir, 0, 0, 0), Target::Site(None));
        assert_eq!(target(Receiver::EveryCountry, 0, 0, 0), Target::EverySite);
        // A route writer names the country holding its route: the route's sender link, or
        // the link RDX points to for the constructors.
        let mut route = [0_u8; 0x40];
        route[engine_resources::resource_map::ROUTE_LINKS[0]..][..4]
            .copy_from_slice(&6_u32.to_le_bytes());
        assert_eq!(
            target(Receiver::Route, route.as_ptr() as usize, 0, 0),
            Target::Route(Some(6))
        );
        assert_eq!(
            target(Receiver::RouteSender, 0, tag.as_ptr() as usize, 0),
            Target::Route(Some(11))
        );
        assert_eq!(
            target(Receiver::RouteSender, 0, zero.as_ptr() as usize, 0),
            Target::Nothing
        );
        assert_eq!(target(Receiver::Route, 0, 0, 0), Target::Route(None));
        assert_eq!(target(Receiver::EveryRoute, 0, 0, 0), Target::EveryRoute);
    }

    /// The family a site's receiver marks.
    fn family_of(receiver: Receiver) -> Family {
        match receiver {
            Receiver::CountryResources => Family::Countries,
            Receiver::StrategicAir
            | Receiver::CountryTag
            | Receiver::EveryCountry
            | Receiver::PoolCapacity => Family::Sites,
            Receiver::Route | Receiver::RouteSender | Receiver::EveryRoute => Family::Routes,
            _ => Family::States,
        }
    }

    /// A state writer marks its state, an override writer its link and a
    /// site list writer its country's link, each in its own set; an unnamed
    /// receiver moves its set's epoch only.
    #[test]
    fn each_site_marks_its_set() {
        const FAMILIES: [Family; 4] = [
            Family::States,
            Family::Countries,
            Family::Sites,
            Family::Routes,
        ];
        for (index, (mutation, receiver)) in MUTATIONS.iter().enumerate() {
            let watches = Watches::enabled(64, 64);
            let epochs = FAMILIES.map(|family| take(&watches, family).0.unwrap());
            let family = family_of(*receiver);
            let named = match family {
                Family::States => Target::State(Some(9)),
                Family::Countries => Target::Country(Some(9)),
                Family::Sites => Target::Site(Some(9)),
                Family::Routes => Target::Route(Some(9)),
            };
            drop(watches.notify(index, named));
            for (other, epoch) in FAMILIES.into_iter().zip(epochs) {
                let marked = if other == family { vec![9] } else { vec![] };
                assert_eq!(
                    take(&watches, other),
                    (Some(epoch), marked),
                    "{}",
                    mutation.name
                );
            }
            let unnamed = match family {
                Family::States => Target::State(None),
                Family::Countries => Target::Country(None),
                Family::Sites => Target::Site(None),
                Family::Routes => Target::Route(None),
            };
            drop(watches.notify(index, unnamed));
            for (other, epoch) in FAMILIES.into_iter().zip(epochs) {
                assert_eq!(
                    take(&watches, other).0 == Some(epoch),
                    other != family,
                    "{}",
                    mutation.name
                );
            }
            assert_eq!(watches.calls[index].load(Ordering::Relaxed), 2);
        }
        // An origin without a state marks nothing and keeps both epochs.
        let watches = Watches::enabled(64, 64);
        let states = take(&watches, Family::States).0;
        assert!(
            watches
                .notify(site("destroy_resource_origin"), Target::Nothing)
                .is_none()
        );
        assert_eq!(take(&watches, Family::States), (states, vec![]));
    }

    /// The state's claim insertion and erasure are observed: each marks its
    /// state in the state set, so StateCores' claims are read within the
    /// hour like its cores.
    #[test]
    fn claim_writers_mark_their_state() {
        for name in ["add_claim", "remove_claim"] {
            let index = site(name);
            assert_eq!(MUTATIONS[index].1, Receiver::State, "{name}");
            let watches = Watches::enabled(64, 64);
            let epoch = take(&watches, Family::States).0.unwrap();
            drop(watches.notify(index, Target::State(Some(12))));
            assert_eq!(
                take(&watches, Family::States),
                (Some(epoch), vec![12]),
                "{name}"
            );
        }
    }

    /// The strategic-location appenders are observed: the state's marks its
    /// state, and the province's marks the province's state, so StateCores'
    /// strategic locations are read within the hour.
    #[test]
    fn strategic_location_writers_mark_their_state() {
        let index = site("add_state_strategic_location");
        assert_eq!(MUTATIONS[index].1, Receiver::State);
        let index = site("add_province_strategic_location");
        assert_eq!(MUTATIONS[index].1, Receiver::Province);
        let watches = Watches::enabled(64, 64);
        let epoch = take(&watches, Family::States).0.unwrap();
        drop(watches.notify(index, Target::State(Some(12))));
        assert_eq!(take(&watches, Family::States), (Some(epoch), vec![12]));
        // The province receiver names its state through the province.
        let mut memory = vec![0_u8; 0x200];
        let state = memory.as_ptr() as usize + 0x100;
        let state_slot = engine_native::cprovince::STATE;
        memory[state_slot..state_slot + 8].copy_from_slice(&(state as u64).to_le_bytes());
        memory[0x100 + engine_native::cstate::ID..0x100 + engine_native::cstate::ID + 4]
            .copy_from_slice(&12_u32.to_le_bytes());
        let province = memory.as_ptr() as usize;
        assert_eq!(
            target(Receiver::Province, province, 0, 0),
            Target::State(Some(12))
        );
        memory[state_slot..state_slot + 8].fill(0);
        assert_eq!(target(Receiver::Province, province, 0, 0), Target::Nothing);
    }

    /// A forwarded write of a state status marks that state; a province
    /// status marks nothing; one without a location, or naming no status,
    /// moves the state set's epoch.
    #[test]
    fn forwarded_building_writes_mark_state_statuses() {
        use engine_native::{cbuilding_status::PROVINCE_FLAG, cbuilding_status::STATE_FLAG};
        let watches = Watches::enabled(64, 64);
        let (Some(epoch), _) = take(&watches, Family::States) else {
            unreachable!("observing")
        };
        drop(watches.mark_building(STATE_FLAG, 12));
        assert!(watches.mark_building(PROVINCE_FLAG, 300).is_none());
        assert_eq!(take(&watches, Family::States), (Some(epoch), vec![12]));
        drop(watches.mark_building(STATE_FLAG, 0));
        assert_ne!(take(&watches, Family::States).0, Some(epoch));
        let (Some(epoch), _) = take(&watches, Family::States) else {
            unreachable!("observing")
        };
        drop(watches.building_unnamed());
        assert_ne!(take(&watches, Family::States).0, Some(epoch));
    }

    /// Without the building family's forwarder the state set certifies
    /// nothing, as an infrastructure write could go unseen; the country set
    /// still observes. A mismatch poisons only its own set.
    #[test]
    fn the_state_set_observes_only_with_the_forwarder() {
        let watches = Watches::enabled(64, 64);
        let (Some(states), _) = take(&watches, Family::States) else {
            unreachable!("observing")
        };
        let (Some(countries), _) = take(&watches, Family::Countries) else {
            unreachable!("observing")
        };
        watches.set_forwarder(false);
        assert_eq!(take(&watches, Family::States).0, None);
        assert_eq!(take(&watches, Family::Countries).0, Some(countries));
        watches.set_forwarder(true);
        assert_ne!(take(&watches, Family::States).0, Some(states));
        watches.mismatch(Family::States);
        assert!(watches.poisoned(Family::States));
        assert!(!watches.poisoned(Family::Countries));
        assert_eq!(take(&watches, Family::States).0, None);
    }

    /// A capacity store of the value already there names no country, marks
    /// nothing and keeps the epoch; a changing one marks its country; a load
    /// writer moves the site set's epoch. Both count as the site's calls.
    #[test]
    fn only_a_changing_capacity_marks_its_country() {
        let site = MUTATIONS
            .iter()
            .position(|(site, _)| site.name == "set_country_pool_capacity")
            .unwrap();
        assert_eq!(site, CAPACITY_SITE);
        let watches = Watches::enabled(64, 64);
        let epoch = take(&watches, Family::Sites).0;
        assert!(
            watches
                .capacity(site, false, || unreachable!(
                    "an unchanged store is not named"
                ))
                .is_none()
        );
        assert_eq!(take(&watches, Family::Sites), (epoch, vec![]));
        drop(watches.capacity(site, true, || Target::Site(Some(5))));
        assert_eq!(take(&watches, Family::Sites), (epoch, vec![5]));
        assert_eq!(watches.site_calls(site), 2);
        assert_eq!(watches.unchanged_capacities.load(Ordering::Relaxed), 1);
        drop(watches.notify(site, Target::EverySite));
        assert_ne!(take(&watches, Family::Sites).0, epoch);
    }

    /// A daily pass in process memory: two bases (a host base, then a state
    /// base whose own container is stocked), each with the pairs (raw 2,
    /// raw 3), and every capacity 100,000.
    struct DailyWorld {
        memory: Vec<u64>,
    }
    impl DailyWorld {
        const GAME: usize = 0x100;
        const LINKS: usize = 0x800;
        const AIR: usize = 0x1000;
        const BASES: usize = 0x1100;
        const PAIRS: usize = 0x5000;
        const VECTORS: usize = 0x5100;
        fn base(index: usize) -> usize {
            0x2000 + index * 0x1000
        }
        fn container(base: usize, slot: usize) -> usize {
            0x8000 + base * 0x400 + slot * 0x100
        }
        fn entries(base: usize) -> usize {
            0x6000 + base * 0x100
        }
        fn at(&self, offset: usize) -> usize {
            self.memory.as_ptr() as usize + offset
        }
        fn word(&mut self, offset: usize, value: u64) {
            self.memory[offset / 8] = value;
        }
        fn half(&mut self, offset: usize, value: u32) {
            let word = &mut self.memory[offset / 8];
            let shift = (offset % 8) * 8;
            *word = (*word & !(0xffff_ffff << shift)) | (u64::from(value) << shift);
        }
        fn array(&mut self, offset: usize, data: usize, count: u32) {
            self.word(offset, self.at(data) as u64);
            self.word(offset + 8, (u64::from(count) << 32) | u64::from(count));
        }
        fn capacity(&mut self, base: usize, slot: usize, value: i64) {
            self.word(
                Self::container(base, slot) + engine_native::ccountry_air_container::CAPACITY_RAW,
                value as u64,
            );
        }
        fn new() -> Self {
            let mut world = Self {
                memory: vec![0; 0xa000 / 8],
            };
            world.word(0, world.at(Self::GAME) as u64);
            world.array(
                Self::GAME + engine_native::cgame_state::COUNTRY_LINKS,
                Self::LINKS,
                4,
            );
            for raw in 1..4 {
                world.half(Self::LINKS + raw * 4, raw as u32);
            }
            world.array(Self::AIR + 0x10, Self::BASES, 2);
            world.half(Self::AIR + engine_native::cstrategic_air::COUNTRY, 1);
            world.word(Self::PAIRS, world.at(Self::VECTORS) as u64);
            for index in 0..2 {
                world.word(Self::BASES + index * 8, world.at(Self::base(index)) as u64);
                let containers = Self::base(index) + 0x100;
                world.array(
                    Self::base(index) + engine_native::cair_base::COUNTRY_CONTAINERS,
                    containers,
                    4,
                );
                for slot in 1..4 {
                    world.word(
                        containers + slot * 8,
                        world.at(Self::container(index, slot)) as u64,
                    );
                    world.capacity(index, slot, 100_000);
                }
                world.array(Self::VECTORS + index * 0x18, Self::entries(index), 2);
                for (entry, raw) in [2, 3].into_iter().enumerate() {
                    world.half(Self::entries(index) + entry * 16, raw);
                    world.word(Self::entries(index) + entry * 16 + 8, 100_000);
                }
            }
            world.word(Self::base(1) + engine_native::cair_base::STATE, 0x77);
            world
        }
        fn pass(&self) {
            BASE.store(
                self.at(0) - engine_native::cgame_state::SINGLETON_RVA,
                Ordering::Release,
            );
            ORIGINAL[CAPACITY_SITE].store(store as *const () as usize, Ordering::Release);
            DAILY_ORIGINAL.store(native_daily as *const () as usize, Ordering::Release);
            unsafe {
                daily_refresh_base_capacities(self.at(Self::AIR), self.at(Self::PAIRS), 0, 0)
            };
        }
    }

    /// The native capacity store: the slot container's capacity.
    unsafe extern "system" fn store(base: usize, capacity: usize, tag: usize, _: usize) -> usize {
        let containers = unsafe {
            *((base + engine_native::cair_base::COUNTRY_CONTAINERS) as *const *const usize)
        };
        let slot = unsafe { *(tag as *const u32) } as usize;
        let container = unsafe { *containers.add(slot) };
        unsafe {
            *((container + engine_native::ccountry_air_container::CAPACITY_RAW) as *mut i64) =
                capacity as i64
        };
        0
    }

    /// [`active`] as the native daily pass saw it.
    static FLIGHTS_SEEN: AtomicUsize = AtomicUsize::new(0);

    /// Daily air base capacity refresh without its withdrawal: every pair through the patched
    /// setter, in base order.
    unsafe extern "system" fn native_daily(air: usize, pairs: usize, _: usize, _: usize) -> usize {
        FLIGHTS_SEEN.store(active(), Ordering::SeqCst);
        unsafe {
            let bases = *((air + 0x10) as *const usize);
            let count = *((air + 0x1c) as *const i32) as usize;
            let vectors = *(pairs as *const usize);
            for index in 0..count {
                let base = *((bases + index * 8) as *const usize);
                let vector = vectors + index * 0x18;
                let data = *(vector as *const usize);
                for entry in 0..*((vector + 0xc) as *const i32) as usize {
                    let tag = data + entry * 16;
                    set_country_pool_capacity(base, *((tag + 8) as *const usize), tag, 0);
                }
            }
        }
        0
    }

    /// Inside the hourly original, a daily pass whose stores all keep their
    /// capacity counts them and marks nothing, its setter calls reading
    /// nothing more; a pass with one changing store, or any pass outside the
    /// hourly original (initialization), leaves every store to the setter,
    /// which marks exactly the changing pair's country, as the setter alone
    /// always did. Every pass's original is an active flight until it
    /// returns.
    #[test]
    fn a_certified_daily_pass_counts_its_stores_and_a_changing_pass_marks_per_call() {
        let _serial = DAILY_TEST.lock().unwrap();
        WATCHES.sites.offer(64);
        WATCHES.sites.boundary(true);
        WATCHES.hooks.store(true, Ordering::SeqCst);
        WATCHES.apply();
        let sites = |visit: &mut Vec<usize>| WATCHES.take(Family::Sites, &mut |id| visit.push(id));
        let mut marked = Vec::new();
        let epoch = sites(&mut marked);
        assert!(epoch.is_some());
        let counts = || {
            (
                WATCHES
                    .daily_passes
                    .each_ref()
                    .map(|n| n.load(Ordering::Relaxed)),
                WATCHES.unchanged_capacities.load(Ordering::Relaxed),
            )
        };
        let ([certified, per_call], unchanged) = counts();
        let world = DailyWorld::new();
        collector_extended_support::hourly::in_tick(|| world.pass());
        // Counted once by the pass: the setter calls added nothing.
        assert_eq!(counts(), ([certified + 1, per_call], unchanged + 4));
        assert_eq!(FLIGHTS_SEEN.load(Ordering::SeqCst), 1);
        assert_eq!(active(), 0);
        marked.clear();
        assert_eq!(sites(&mut marked), epoch);
        assert!(marked.is_empty());
        assert!(!CERTIFIED_PASS.get());

        // Outside the hourly original the setter reads and counts each store.
        world.pass();
        assert_eq!(counts(), ([certified + 1, per_call + 1], unchanged + 8));
        assert_eq!(FLIGHTS_SEEN.load(Ordering::SeqCst), 1);
        assert_eq!(active(), 0);

        let mut world = DailyWorld::new();
        world.word(DailyWorld::entries(1) + 16 + 8, 120_000);
        collector_extended_support::hourly::in_tick(|| world.pass());
        assert_eq!(counts(), ([certified + 1, per_call + 2], unchanged + 8 + 3));
        assert_eq!(FLIGHTS_SEEN.load(Ordering::SeqCst), 1);
        assert_eq!(active(), 0);
        marked.clear();
        assert_eq!(sites(&mut marked), epoch);
        assert_eq!(marked, [3]);
        assert_eq!(
            world.memory[(DailyWorld::container(1, 3)
                + engine_native::ccountry_air_container::CAPACITY_RAW)
                / 8],
            120_000
        );
        WATCHES.hooks.store(false, Ordering::SeqCst);
        WATCHES.apply();
    }

    /// The daily air base capacity refresh stores every base's capacity for every
    /// country through country air pool capacity setter, nearly always the value already there.
    /// Release: one such store through the detour, minus the native store.
    #[test]
    #[ignore = "release per-call detour cost; run with --release --ignored --nocapture --test-threads=1"]
    fn hook_cost_raid_capacity() {
        let _serial = DAILY_TEST.lock().unwrap();
        const COUNTRIES: usize = 1500;
        const GAME: usize = 0x1000;
        const LINKS: usize = 0x2000;
        const AIR_BASE: usize = (LINKS + (COUNTRIES + 1) * 4).next_multiple_of(0x100);
        const CONTAINERS: usize = AIR_BASE + 0x100;
        const OBJECTS: usize = CONTAINERS + (COUNTRIES + 1) * 8;
        const TAGS: usize = OBJECTS + (COUNTRIES + 1) * 0x100;
        let mut memory = vec![0_u64; (TAGS + (COUNTRIES + 1) * 4).div_ceil(8)];
        let at = memory.as_ptr() as usize;
        let mut word = |offset: usize, value: u64| memory[offset / 8] = value;
        word(0, (at + GAME) as u64);
        let array = |word: &mut dyn FnMut(usize, u64), offset: usize, data: usize| {
            word(offset, (at + data) as u64);
            word(
                offset + 8,
                ((COUNTRIES as u64 + 1) << 32) | (COUNTRIES as u64 + 1),
            );
        };
        array(
            &mut word,
            GAME + engine_native::cgame_state::COUNTRY_LINKS,
            LINKS,
        );
        array(
            &mut word,
            AIR_BASE + engine_native::cair_base::COUNTRY_CONTAINERS,
            CONTAINERS,
        );
        for slot in 1..=COUNTRIES {
            let object = OBJECTS + slot * 0x100;
            word(CONTAINERS + slot * 8, (at + object) as u64);
            word(
                object + engine_native::ccountry_air_container::CAPACITY_RAW,
                100_000,
            );
        }
        let words = memory.as_mut_ptr().cast::<u32>();
        for slot in 1..=COUNTRIES {
            // Raw link `slot` names canonical slot `slot`.
            unsafe {
                *words.add((LINKS + slot * 4) / 4) = slot as u32;
                *words.add((TAGS + slot * 4) / 4) = slot as u32;
            }
        }
        unsafe extern "system" fn store(
            base: usize,
            capacity: usize,
            tag: usize,
            _: usize,
        ) -> usize {
            let containers = unsafe {
                *((base + engine_native::cair_base::COUNTRY_CONTAINERS) as *const *const usize)
            };
            let slot = unsafe { *(tag as *const u32) } as usize;
            let container = unsafe { *containers.add(slot) };
            unsafe {
                *((container + engine_native::ccountry_air_container::CAPACITY_RAW) as *mut i64) =
                    capacity as i64
            };
            0
        }
        BASE.store(
            at - engine_native::cgame_state::SINGLETON_RVA,
            Ordering::Release,
        );
        ORIGINAL[CAPACITY_SITE].store(store as *const () as usize, Ordering::Release);
        let unchanged = || {
            for slot in 1..=COUNTRIES {
                std::hint::black_box(unsafe {
                    set_country_pool_capacity(at + AIR_BASE, 100_000, at + TAGS + slot * 4, 0)
                });
            }
        };
        let native = || {
            for slot in 1..=COUNTRIES {
                std::hint::black_box(unsafe {
                    store(at + AIR_BASE, 100_000, at + TAGS + slot * 4, 0)
                });
            }
        };
        let before = WATCHES.unchanged_capacities.load(Ordering::Relaxed);
        let hooked = crate::test_support::per_call_ns(COUNTRIES, unchanged);
        let plain = crate::test_support::per_call_ns(COUNTRIES, native);
        assert_eq!(
            WATCHES.unchanged_capacities.load(Ordering::Relaxed) - before,
            8 * COUNTRIES as u64
        );
        println!(
            "hook_cost raid_changes set_country_pool_capacity unchanged: detour {:.1} ns/call (hooked {hooked:.1}, native {plain:.1})",
            hooked - plain
        );
    }

    /// A claim insertion through the detour: the state read, the mark in the
    /// state set and the forward check, minus the native call. Release, with
    /// the family observing.
    #[test]
    #[ignore = "release per-call detour cost; run with --release --ignored --nocapture --test-threads=1"]
    fn hook_cost_raid_claim() {
        let _serial = DAILY_TEST.lock().unwrap();
        const STATES: usize = 1500;
        let mut memory = vec![0_u64; STATES * 0x100 / 8];
        for state in 0..STATES {
            memory[(state * 0x100 + engine_native::cstate::ID) / 8] = state as u64 + 1;
        }
        let at = memory.as_ptr() as usize;
        let tag = 5_u32;
        let tag = std::ptr::from_ref(&tag) as usize;
        unsafe extern "system" fn insert(state: usize, tag: usize, _: usize, _: usize) -> usize {
            std::hint::black_box((state, tag));
            0
        }
        let site = site("add_claim");
        ORIGINAL[site].store(insert as *const () as usize, Ordering::Release);
        WATCHES.states.offer(STATES + 1);
        WATCHES.states.boundary(true);
        WATCHES.forward(true);
        WATCHES.hooks.store(true, Ordering::SeqCst);
        WATCHES.apply();
        let hooked = crate::test_support::per_call_ns(STATES, || {
            for state in 0..STATES {
                std::hint::black_box(unsafe { add_claim(at + state * 0x100, tag, 0, 0) });
            }
        });
        let plain = crate::test_support::per_call_ns(STATES, || {
            for state in 0..STATES {
                std::hint::black_box(unsafe { insert(at + state * 0x100, tag, 0, 0) });
            }
        });
        let mut marked = Vec::new();
        WATCHES.take(Family::States, &mut |id| marked.push(id));
        assert_eq!(marked.len(), STATES);
        println!(
            "hook_cost raid_changes add_claim: detour {:.1} ns/call (hooked {hooked:.1}, native {plain:.1})",
            hooked - plain
        );
        WATCHES.hooks.store(false, Ordering::SeqCst);
        WATCHES.apply();
    }

    #[test]
    fn an_invalidating_boundary_revokes_both_sets() {
        let watches = Watches::enabled(64, 64);
        let states = take(&watches, Family::States).0;
        let countries = take(&watches, Family::Countries).0;
        watches.boundary(false);
        assert_eq!(take(&watches, Family::States).0, states);
        watches.boundary(true);
        assert_ne!(take(&watches, Family::States).0, states);
        assert_ne!(take(&watches, Family::Countries).0, countries);
    }
}
