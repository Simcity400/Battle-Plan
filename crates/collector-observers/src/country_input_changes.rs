pub mod weather_carry;

use crate::{
    dirty_set::{Dirty, DirtySet},
    hook::Hook,
    watch_registry::WatchRegistry,
};
use engine_mutations::{
    country_input_mutations as sites, country_input_mutations::FACTION_SHARING_BASE,
    country_input_mutations::GROW_INTEL_NETWORK, country_input_mutations::MUTATIONS,
    country_input_mutations::PROJECT_DATABASE_RESYNC,
    country_input_mutations::PROJECT_REWARD_OPTION, country_input_mutations::REMOVE_UNIT_LEADER,
    country_input_mutations::Receiver, country_input_mutations::SYNC_RESEARCH_SLOTS,
    country_input_mutations::Tables,
};
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

pub type DesignSourceOriginal = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
pub struct DesignSourceForwarder {
    pub call: unsafe fn([usize; 4], DesignSourceOriginal) -> usize,
    pub enabled: fn(bool),
}
static DESIGN_SOURCES: std::sync::atomic::AtomicPtr<DesignSourceForwarder> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
pub fn design_source_forwarder(receiver: Option<&'static DesignSourceForwarder>) {
    let pointer = receiver.map_or(std::ptr::null_mut(), |receiver| {
        std::ptr::from_ref(receiver).cast_mut()
    });
    DESIGN_SOURCES.store(pointer, Ordering::Release);
    if let Some(receiver) = receiver {
        (receiver.enabled)(DOMAIN.hooks.load(Ordering::Acquire));
    }
}
fn design_source_availability(enabled: bool) {
    if let Some(receiver) = unsafe { DESIGN_SOURCES.load(Ordering::Acquire).as_ref() } {
        (receiver.enabled)(enabled);
    }
}
/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
unsafe fn design_source_invoke(args: [usize; 4], original: DesignSourceOriginal) -> usize {
    let receiver = DESIGN_SOURCES.load(Ordering::Acquire);
    if let Some(receiver) = unsafe { receiver.as_ref() } {
        unsafe { (receiver.call)(args, original) }
    } else {
        unsafe { original(args[0], args[1], args[2], args[3]) }
    }
}

const SITES: usize = MUTATIONS.len();
static DOMAIN: Domain = Domain::new();
static ACCUMULATOR_DEMAND: AtomicUsize = AtomicUsize::new(0);
static SHARING_DEMAND: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static BASE: AtomicUsize = AtomicUsize::new(0);
static READS: [[AtomicU64; 3]; SETS] = [const { [const { AtomicU64::new(0) }; 3] }; SETS];
static CARRIED: [AtomicU64; SETS] = [const { AtomicU64::new(0) }; SETS];
static IDS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    Stability = 0,
    Research = 1,
    Technologies = 2,
    Focus = 3,
    Projects = 4,
    Appearance = 5,
    Counters = 6,
    Visibility = 7,
    Designs = 8,
    UnitSpeeds = 9,
}

const SETS: usize = 10;

const CONFIGURED: [bool; SETS] = [
    true,
    true,
    cfg!(feature = "technology_changes"),
    cfg!(feature = "focus_changes"),
    cfg!(feature = "projects_changes"),
    cfg!(feature = "appearance_changes"),
    cfg!(feature = "counter_changes"),
    cfg!(feature = "visibility_changes"),
    true,
    cfg!(feature = "counter_changes"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forward {
    ResetCountry,
    RulingParty,
    Ideas,
    Popularity,
    PartyLeaders,
    Roles,
    ScientistRoles,
    ScientistTraits,
    DiplomacyReset,
    Focus,
    CharacterIdentity,
    WartimeAllies,
    StateOwner,
    StateController,
    StateReset,
    Cores,
    RelationReset,
    RelationSlot,
    CountryCharacters,
}

impl Forward {
    pub const ALL: [Self; 19] = [
        Self::ResetCountry,
        Self::RulingParty,
        Self::Ideas,
        Self::Popularity,
        Self::PartyLeaders,
        Self::Roles,
        Self::ScientistRoles,
        Self::ScientistTraits,
        Self::DiplomacyReset,
        Self::Focus,
        Self::CharacterIdentity,
        Self::WartimeAllies,
        Self::StateOwner,
        Self::StateController,
        Self::StateReset,
        Self::Cores,
        Self::RelationReset,
        Self::RelationSlot,
        Self::CountryCharacters,
    ];

    fn receiver(self) -> Receiver {
        match self {
            Self::ResetCountry => Receiver::Country,
            Self::RulingParty | Self::Ideas => Receiver::PoliticalStatus,
            Self::Popularity | Self::PartyLeaders => Receiver::Party,
            Self::Roles | Self::ScientistRoles | Self::CharacterIdentity => Receiver::Character,
            Self::ScientistTraits => Receiver::Scientist,
            Self::DiplomacyReset | Self::StateReset | Self::CountryCharacters => Receiver::Every,
            Self::Focus => Receiver::FocusProgress,
            Self::WartimeAllies => Receiver::DiplomacyStatus,
            Self::StateOwner => Receiver::StateOwner,
            Self::StateController => Receiver::StateController,
            Self::Cores => Receiver::TagPointer,
            Self::RelationReset | Self::RelationSlot => Receiver::RelationStatus,
        }
    }

    fn tables(self) -> Tables {
        match self {
            Self::ResetCountry => Tables::ALL,
            Self::Ideas => Tables::BOTH,
            Self::ScientistRoles | Self::ScientistTraits | Self::CharacterIdentity => {
                Tables::RESEARCH
            }
            Self::Popularity => Tables::STABILITY,
            Self::RulingParty | Self::PartyLeaders | Self::Roles => {
                Tables::STABILITY.with(Tables::APPEARANCE)
            }
            Self::DiplomacyReset => Tables::STABILITY
                .with(Tables::APPEARANCE)
                .with(Tables::COUNTERS),
            Self::Focus => Tables::FOCUS,
            Self::StateOwner | Self::StateReset => Tables::COUNTERS.with(DRIFTING),
            Self::WartimeAllies
            | Self::StateController
            | Self::Cores
            | Self::RelationReset
            | Self::RelationSlot
            | Self::CountryCharacters => Tables::COUNTERS,
        }
    }

    fn forwarder(self) -> Forwarder {
        match self {
            Self::ResetCountry | Self::Focus => Forwarder::Politics,
            Self::RulingParty | Self::Ideas | Self::Popularity => Forwarder::PoliticalStatus,
            Self::PartyLeaders => Forwarder::PartyLeaders,
            Self::Roles | Self::CharacterIdentity => Forwarder::Roles,
            Self::ScientistRoles | Self::ScientistTraits => Forwarder::Roster,
            Self::DiplomacyReset | Self::WartimeAllies => Forwarder::Relations,
            Self::StateOwner | Self::StateController | Self::StateReset => Forwarder::Territory,
            Self::Cores => Forwarder::Raid,
            Self::RelationReset | Self::RelationSlot => Forwarder::Opinions,
            Self::CountryCharacters => Forwarder::AdvisorPool,
        }
    }
}

const DRIFTING: Tables = Tables::TECHNOLOGY
    .with(Tables::FOCUS)
    .with(Tables::PROJECTS);

const fn table_flags(tables: Tables) -> [bool; SETS] {
    [
        tables.stability,
        tables.research,
        tables.technologies,
        tables.focus,
        tables.projects,
        tables.appearance,
        tables.counters,
        tables.visibility,
        tables.visibility,
        tables.counters,
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forwarder {
    Politics,
    PoliticalStatus,
    PartyLeaders,
    Roles,
    Roster,
    Relations,
    Territory,
    Raid,
    Opinions,
    AdvisorPool,
}

impl Forwarder {
    pub const ALL: [Self; 10] = [
        Self::Politics,
        Self::PoliticalStatus,
        Self::PartyLeaders,
        Self::Roles,
        Self::Roster,
        Self::Relations,
        Self::Territory,
        Self::Raid,
        Self::Opinions,
        Self::AdvisorPool,
    ];

    fn reaches(self, set: usize) -> bool {
        Forward::ALL
            .iter()
            .any(|forward| forward.forwarder() == self && table_flags(forward.tables())[set])
    }
}

pub struct Domain {
    sharing: WatchRegistry,
    accumulator: WatchRegistry,
    names: WatchRegistry,
    sets: [DirtySet; SETS],
    calls: [AtomicU64; SITES],
    forwarded: [AtomicU64; Forward::ALL.len()],
    unnamed: AtomicU64,
    resynced: AtomicU64,
    rebased: AtomicU64,
    removed_leaders: AtomicU64,
    network_writes: AtomicU64,
    weather_writes: AtomicU64,
    hooks: AtomicBool,
    forwarders: [AtomicBool; Forwarder::ALL.len()],
    hours: [AtomicU64; SETS],
}

type Sets<'a> = [Option<&'a DirtySet>; SETS];

#[must_use]
pub struct Marks<'a> {
    _parts: [Part<'a>; 2],
    _occupation: Option<crate::watch_registry::Notification<'static>>,
    _wargoals: Option<crate::wargoal_changes::Guard>,
    _project_reset: Option<crate::project_variable_changes::Reset>,
    _accumulator: Option<crate::watch_registry::Notification<'a>>,
    _names: Option<crate::watch_registry::Notification<'a>>,
    _market: Option<crate::market_changes::CountryReset>,
}

#[derive(Default)]
struct Part<'a> {
    #[allow(dead_code)]
    held: [Option<Dirty<'a>>; SETS],
    again: Option<Again<'a>>,
}

struct Again<'a> {
    sets: Sets<'a>,
    receiver: Receiver,
    first: usize,
    second: usize,
}

impl<'a> Marks<'a> {
    pub fn none() -> Self {
        Self {
            _parts: [Part::default(), Part::default()],
            _occupation: None,
            _wargoals: None,
            _project_reset: None,
            _accumulator: None,
            _names: None,
            _market: None,
        }
    }
}

impl<'a> Part<'a> {
    fn every(sets: Sets<'a>) -> Self {
        Self {
            held: sets.map(|set| set.map(|set| set.notify_every())),
            again: None,
        }
    }
}

impl Drop for Part<'_> {
    fn drop(&mut self) {
        let Some(again) = self.again.take() else {
            return;
        };
        let sets = again.sets;
        let named = name(again.receiver, again.first, again.second, &mut |id| {
            for set in sets.into_iter().flatten() {
                drop(set.notify(id as usize));
            }
        });
        if !named {
            for set in sets.into_iter().flatten() {
                drop(set.notify_every());
            }
        }
    }
}

fn name(receiver: Receiver, first: usize, second: usize, visit: &mut dyn FnMut(u32)) -> bool {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    sites::countries(
        &memory,
        BASE.load(Ordering::Relaxed),
        receiver,
        first,
        second,
        visit,
    )
}

impl Default for Domain {
    fn default() -> Self {
        Self::new()
    }
}

impl Domain {
    pub const fn new() -> Self {
        Self {
            sharing: WatchRegistry::retaining(),
            accumulator: WatchRegistry::retaining(),
            names: WatchRegistry::new(),
            sets: [const { DirtySet::new() }; SETS],
            calls: [const { AtomicU64::new(0) }; SITES],
            forwarded: [const { AtomicU64::new(0) }; Forward::ALL.len()],
            unnamed: AtomicU64::new(0),
            resynced: AtomicU64::new(0),
            rebased: AtomicU64::new(0),
            removed_leaders: AtomicU64::new(0),
            network_writes: AtomicU64::new(0),
            weather_writes: AtomicU64::new(0),
            hooks: AtomicBool::new(false),
            forwarders: [const { AtomicBool::new(false) }; Forwarder::ALL.len()],
            hours: [const { AtomicU64::new(0) }; SETS],
        }
    }

    fn set(&self, table: Table) -> &DirtySet {
        &self.sets[table as usize]
    }

    fn sets(&self, tables: Tables, own: bool) -> Sets<'_> {
        [
            tables.stability.then_some(self.set(Table::Stability)),
            tables.research.then_some(self.set(Table::Research)),
            (tables.technologies && own).then_some(self.set(Table::Technologies)),
            tables.focus.then_some(self.set(Table::Focus)),
            tables.projects.then_some(self.set(Table::Projects)),
            tables.appearance.then_some(self.set(Table::Appearance)),
            tables.counters.then_some(self.set(Table::Counters)),
            tables.visibility.then_some(self.set(Table::Visibility)),
            tables.visibility.then_some(self.set(Table::Designs)),
            tables.counters.then_some(self.set(Table::UnitSpeeds)),
        ]
    }

    fn mark(&self, tables: Tables, receiver: Receiver, first: usize, second: usize) -> Marks<'_> {
        let own = receiver.own();
        let shared = self.part(self.sets(tables, own == receiver), receiver, first, second);
        let own = if tables.technologies && own != receiver {
            let mut technologies: Sets<'_> = [None; SETS];
            technologies[Table::Technologies as usize] = Some(self.set(Table::Technologies));
            self.part(technologies, own, first, second)
        } else {
            Part::default()
        };
        Marks {
            _parts: [shared, own],
            _occupation: None,
            _wargoals: None,
            _project_reset: None,
            _accumulator: None,
            _names: None,
            _market: None,
        }
    }

    fn part<'a>(
        &self,
        sets: Sets<'a>,
        receiver: Receiver,
        first: usize,
        second: usize,
    ) -> Part<'a> {
        if receiver == Receiver::Every {
            return Part::every(sets);
        }
        let mut held: [Option<Dirty<'a>>; SETS] = [const { None }; SETS];
        let mut seen = false;
        let mut more = false;
        let named = name(receiver, first, second, &mut |id| {
            if seen {
                more = true;
                for set in sets.into_iter().flatten() {
                    drop(set.notify(id as usize));
                }
            } else {
                seen = true;
                held = sets.map(|set| set.map(|set| set.notify(id as usize)));
            }
        });
        if !named {
            self.unnamed.fetch_add(1, Ordering::Relaxed);
            return Part::every(sets);
        }
        Part {
            held,
            again: (more || receiver.renames()).then_some(Again {
                sets,
                receiver,
                first,
                second,
            }),
        }
    }

    pub fn notify(&self, site: usize, first: usize, second: usize) -> Marks<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        let mut marks = self.mark(MUTATIONS[site].2, MUTATIONS[site].1, first, second);
        marks._accumulator = (site == 0).then(|| self.accumulator.notify(first));
        marks._names = (site == sites::REBUILD_LOCALIZED_NAME).then(|| self.names.notify(0));
        marks
    }

    fn sync(
        &self,
        status: usize,
        roster: &dyn Fn(usize) -> Option<[u8; 16]>,
        original: impl FnOnce() -> usize,
    ) -> usize {
        self.when_changed(
            SYNC_RESEARCH_SLOTS,
            status,
            roster,
            &self.resynced,
            original,
        )
    }

    fn rebase(
        &self,
        group: usize,
        base: &dyn Fn(usize) -> Option<[u8; 16]>,
        original: impl FnOnce() -> usize,
    ) -> usize {
        self.when_changed(FACTION_SHARING_BASE, group, base, &self.rebased, original)
    }

    fn remove_leader(
        &self,
        characters: usize,
        counts: &dyn Fn(usize) -> Option<[u8; 16]>,
        original: impl FnOnce() -> usize,
    ) -> usize {
        self.when_changed(
            REMOVE_UNIT_LEADER,
            characters,
            counts,
            &self.removed_leaders,
            original,
        )
    }

    fn when_changed(
        &self,
        site: usize,
        first: usize,
        state: &dyn Fn(usize) -> Option<[u8; 16]>,
        changed: &AtomicU64,
        original: impl FnOnce() -> usize,
    ) -> usize {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        let receiver = MUTATIONS[site].1;
        let tables = MUTATIONS[site].2;
        let before = state(first);
        let sharing_running = (site == FACTION_SHARING_BASE).then(|| self.sharing.running());
        let sharing =
            (site == FACTION_SHARING_BASE && before.is_none()).then(|| self.sharing.notify(first));
        let covered = before
            .is_none()
            .then(|| self.mark(tables, receiver, first, 0));
        let result = original();
        if before.is_some() && state(first) != before {
            if site == FACTION_SHARING_BASE {
                drop(self.sharing.notify(first));
            }
            changed.fetch_add(1, Ordering::Relaxed);
            drop(self.mark(tables, receiver, first, 0));
        }
        drop(covered);
        drop(sharing);
        drop(sharing_running);
        result
    }

    pub fn forward(&self, site: Forward, first: usize, second: usize) -> Marks<'_> {
        self.forwarded[site as usize].fetch_add(1, Ordering::Relaxed);
        if site == Forward::ResetCountry {
            self.network_writes.fetch_add(1, Ordering::SeqCst);
        }
        let mut marks = self.mark(site.tables(), site.receiver(), first, second);
        if site == Forward::ResetCountry {
            marks._wargoals = Some(crate::wargoal_changes::country_reset(first));
        }
        if site == Forward::DiplomacyReset {
            marks._wargoals = Some(crate::wargoal_changes::diplomacy_reset(first));
        }
        marks._accumulator =
            (site == Forward::ResetCountry).then(|| self.accumulator.notify(first));
        marks._names = (site == Forward::ResetCountry).then(|| self.names.notify(0));
        marks
    }

    fn intel_network_writes(&self) -> Option<u64> {
        let observed = self.hooks.load(Ordering::SeqCst)
            && self.forwarders[Forwarder::Politics as usize].load(Ordering::SeqCst);
        observed.then(|| self.network_writes.load(Ordering::SeqCst))
    }

    fn intel_network_resets(&self) -> Option<u64> {
        self.intel_network_writes()
            .map(|_| self.forwarded[Forward::ResetCountry as usize].load(Ordering::Acquire))
    }

    fn weather_writes(&self) -> Option<u64> {
        (cfg!(feature = "weather_changes") && self.hooks.load(Ordering::SeqCst))
            .then(|| self.weather_writes.load(Ordering::SeqCst))
    }

    fn research_progress(
        &self,
        technology: usize,
        source: usize,
        original: impl FnOnce() -> usize,
    ) -> usize {
        let _marks = (source as u32 == sites::EFFECT_PROGRESS_SOURCE)
            .then(|| self.notify(sites::RESEARCH_PROGRESS, technology, 0));
        original()
    }

    fn weather_write(&self, site: usize, original: impl FnOnce() -> usize) -> usize {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.weather_writes.fetch_add(1, Ordering::SeqCst);
        let result = original();
        self.weather_writes.fetch_add(1, Ordering::SeqCst);
        result
    }

    fn grow_intel_network(&self, original: impl FnOnce() -> usize) -> usize {
        self.calls[GROW_INTEL_NETWORK].fetch_add(1, Ordering::Relaxed);
        self.network_writes.fetch_add(1, Ordering::SeqCst);
        let result = original();
        self.network_writes.fetch_add(1, Ordering::SeqCst);
        result
    }

    fn apply(&self) {
        self.sharing.set_enabled(self.hooks.load(Ordering::SeqCst));
        let reset = self.hooks.load(Ordering::SeqCst)
            && self.forwarders[Forwarder::Politics as usize].load(Ordering::SeqCst);
        self.accumulator.set_enabled(reset);
        self.names.set_enabled(reset);
        let hooks = self.hooks.load(Ordering::SeqCst);
        for (index, set) in self.sets.iter().enumerate() {
            let forwarded = Forwarder::ALL.iter().all(|&forwarder| {
                !forwarder.reaches(index)
                    || self.forwarders[forwarder as usize].load(Ordering::SeqCst)
            });
            set.set_enabled(hooks && forwarded);
            if index == Table::Stability as usize {
                crate::occupation_changes::forwarder(
                    crate::occupation_changes::Forwarder::Exile,
                    hooks && forwarded,
                );
            }
        }
    }

    fn offer_countries(&self, ids: usize) {
        engine_allocation::advisory(|| {
            for set in &self.sets {
                set.offer(ids);
            }
        });
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(ids: usize) -> Self {
        let domain = Self::new();
        for set in &domain.sets {
            set.offer(ids);
            set.boundary(false);
        }
        domain.accumulator.offer(ids);
        domain.accumulator.boundary(false);
        domain.sharing.offer(ids);
        domain.sharing.boundary(false);
        domain.hooks.store(true, Ordering::SeqCst);
        for forwarder in &domain.forwarders {
            forwarder.store(true, Ordering::SeqCst);
        }
        domain.apply();
        domain
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn write(&self, table: Table, id: u32) {
        drop(self.set(table).notify(id as usize));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn write_every(&self, table: Table) {
        drop(self.set(table).notify_every());
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.accumulator.boundary(invalidate);
        self.sharing.boundary(invalidate);
        for set in &self.sets {
            set.boundary(invalidate);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self, table: Table) -> bool {
        self.set(table).poisoned()
    }
}

pub trait Changes: Sync {
    fn take(&self, table: Table, visit: &mut dyn FnMut(usize)) -> Option<u64>;
}

impl Changes for Domain {
    fn take(&self, table: Table, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        if !CONFIGURED[table as usize] {
            return None;
        }
        self.set(table).take(visit)
    }
}

pub struct Observer;
impl Changes for Observer {
    fn take(&self, table: Table, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        DOMAIN.take(table, visit)
    }
}

pub fn drifts(memory: &impl Memory, country: usize) -> bool {
    country
        .checked_add(engine_native::ccountry::OWNED_STATE_COUNT)
        .and_then(|address| engine_native_memory::read::<i32>(memory, address))
        .is_none_or(|count| count >= 1)
}

pub fn mio_policies_observed() -> bool {
    DOMAIN.hooks.load(Ordering::SeqCst)
}

pub fn intel_network_writes() -> Option<u64> {
    DOMAIN.intel_network_writes()
}

pub fn intel_network_resets() -> Option<u64> {
    DOMAIN.intel_network_resets()
}

pub fn weather_writes() -> Option<u64> {
    DOMAIN.weather_writes()
}

pub fn active() -> usize {
    DOMAIN.accumulator.active()
        + DOMAIN.sharing.active()
        + DOMAIN.names.active()
        + DOMAIN.sets.iter().map(|set| set.active()).sum::<usize>()
        + crate::crew_changes::active()
        + crate::wargoal_changes::active()
}

pub fn boundary(invalidate: bool) {
    DOMAIN.accumulator.boundary(invalidate);
    DOMAIN.sharing.boundary(invalidate);
    if invalidate {
        crate::project_variable_changes::invalidate();
    }
    crate::crew_changes::boundary(invalidate);
    crate::wargoal_changes::boundary(invalidate);
    for set in &DOMAIN.sets {
        set.boundary(invalidate);
    }
}

pub fn forwarder(family: Forwarder, enabled: bool) {
    if family == Forwarder::Politics {
        crate::wargoal_changes::country_reset_forwarder(enabled);
        crate::market_changes::observe_resets(enabled);
    }
    if family == Forwarder::Relations {
        crate::wargoal_changes::reset_forwarder(enabled);
    }
    DOMAIN.forwarders[family as usize].store(enabled, Ordering::SeqCst);
    DOMAIN.apply();
}

pub fn forwarded(site: Forward, first: usize, second: usize) -> Marks<'static> {
    let mut marks = DOMAIN.forward(site, first, second);
    marks._occupation =
        occupation_forward_receiver(site, first).map(crate::occupation_changes::changed);
    if site == Forward::ResetCountry {
        marks._project_reset = Some(crate::project_variable_changes::resetting());
    }
    if site == Forward::ResetCountry {
        marks._market = Some(crate::market_changes::country_reset(first));
    }
    marks
}

fn occupation_forward_receiver(site: Forward, first: usize) -> Option<usize> {
    matches!(site, Forward::DiplomacyReset).then_some(first)
}
fn occupation_exile_receiver(site: usize, first: usize) -> Option<usize> {
    matches!(
        site,
        10..=12
            | sites::ADD_EXILE_LEGITIMACY
            | sites::SET_EXILE_LEGITIMACY
            | sites::EXILE_EQUIPMENT_LEGITIMACY
    )
    .then_some(first)
}

pub fn project_variables_written(owner: Option<u32>) -> Marks<'static> {
    let mut sets: Sets<'static> = [None; SETS];
    sets[Table::Projects as usize] = Some(DOMAIN.set(Table::Projects));
    let part = match owner {
        Some(owner) => Part {
            held: sets.map(|set| set.map(|set| set.notify(owner as usize))),
            again: None,
        },
        None => Part::every(sets),
    };
    Marks {
        _parts: [part, Part::default()],
        _project_reset: None,
        _occupation: None,
        _wargoals: None,
        _accumulator: None,
        _names: None,
        _market: None,
    }
}

fn status_forward(site: usize) -> Option<Forward> {
    match site {
        1..=3 => Some(Forward::RulingParty),
        4 => Some(Forward::Ideas),
        5..=8 => Some(Forward::Popularity),
        _ => None,
    }
}

pub fn forwarded_status(site: usize, first: usize) -> Marks<'static> {
    match status_forward(site) {
        Some(forward) => forwarded(forward, first, 0),
        None => Marks::none(),
    }
}

fn role_forward(site: usize) -> Forward {
    use engine_mutations::character_role_mutations as roles;
    match site {
        roles::CHARACTER_NAME | roles::CHARACTER_REPAIR => Forward::CharacterIdentity,
        _ => Forward::Roles,
    }
}

pub fn forwarded_role(site: usize, character: Option<usize>) -> Marks<'static> {
    forwarded(role_forward(site), character.unwrap_or(0), 0)
}

fn roster_forward(site: usize, first: usize, second: usize) -> Option<(Forward, usize)> {
    use engine_mutations::character_roster_mutations as roster;
    match site {
        roster::SCIENTIST_BUILD => Some((Forward::ScientistRoles, second)),
        roster::SCIENTIST_ATTACH | roster::SCIENTIST_CLEAR => {
            Some((Forward::ScientistRoles, first))
        }
        roster::SCIENTIST_TRAIT_ADD
        | roster::SCIENTIST_TRAIT_REMOVE
        | roster::SCIENTIST_TRAIT_RESET => Some((Forward::ScientistTraits, first)),
        _ => None,
    }
}

pub fn forwarded_roster(site: usize, first: usize, second: usize) -> Marks<'static> {
    match roster_forward(site, first, second) {
        Some((forward, receiver)) => forwarded(forward, receiver, 0),
        None => Marks::none(),
    }
}

fn territory_forward(site: usize) -> Option<Forward> {
    match site {
        7 => Some(Forward::StateOwner),
        8 => Some(Forward::StateController),
        9 => Some(Forward::StateReset),
        _ => None,
    }
}

pub fn forwarded_territory(site: usize, first: usize, second: usize) -> Marks<'static> {
    match territory_forward(site) {
        Some(forward) => forwarded(forward, first, second),
        None => Marks::none(),
    }
}

fn raid_forward(site: usize) -> Option<Forward> {
    match site {
        6 | 7 => Some(Forward::Cores),
        _ => None,
    }
}

pub fn forwarded_raid(site: usize, first: usize, second: usize) -> Marks<'static> {
    match raid_forward(site) {
        Some(forward) => forwarded(forward, first, second),
        None => Marks::none(),
    }
}

fn pool_forward(site: usize) -> Option<Forward> {
    match site {
        5 | 6 => Some(Forward::CountryCharacters),
        _ => None,
    }
}

pub fn forwarded_pool(site: usize) -> Marks<'static> {
    match pool_forward(site) {
        Some(forward) => forwarded(forward, 0, 0),
        None => Marks::none(),
    }
}

pub fn sharing_demand(count: usize) {
    SHARING_DEMAND.fetch_max(count, Ordering::Relaxed);
}
pub fn sharing_quiet() -> Option<(u64, u64)> {
    DOMAIN.sharing.quiet()
}
pub fn sharing_sequence(group: usize) -> Option<(u64, u64)> {
    DOMAIN.sharing.sequence(group)
}
pub fn accumulator_demand(count: usize) {
    ACCUMULATOR_DEMAND.fetch_max(count, Ordering::Relaxed);
}
pub fn accumulator_quiet() -> Option<(u64, u64)> {
    DOMAIN.accumulator.quiet()
}
pub fn accumulator_sequence(country: usize) -> Option<(u64, u64)> {
    DOMAIN.accumulator.sequence(country)
}
pub fn names_quiet() -> Option<(u64, u64)> {
    DOMAIN.names.quiet()
}
#[cfg(test)]
pub(crate) fn observe_names() {
    DOMAIN.names.set_enabled(true);
}
#[cfg(test)]
pub(crate) fn rebuilding_name() -> crate::watch_registry::Notification<'static> {
    DOMAIN.names.notify(0)
}
pub fn presize_countries(ids: usize) {
    IDS.fetch_max(ids, Ordering::Relaxed);
    if ids != 0 {
        DOMAIN.offer_countries(ids);
    }
}

pub fn presize() {
    crate::wargoal_changes::presize();
    DOMAIN.accumulator.offer(
        ACCUMULATOR_DEMAND
            .load(Ordering::Relaxed)
            .max(DOMAIN.accumulator.registered()),
    );
    DOMAIN.sharing.offer(
        SHARING_DEMAND
            .load(Ordering::Relaxed)
            .max(DOMAIN.sharing.registered()),
    );
    let ids = IDS.load(Ordering::Relaxed);
    if ids != 0 {
        DOMAIN.offer_countries(ids);
    }
}

pub fn collect() {
    DOMAIN.accumulator.collect();
    DOMAIN.sharing.collect();
    crate::crew_changes::collect();
    crate::wargoal_changes::collect();
    for set in &DOMAIN.sets {
        set.collect();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub marked: u64,
    pub read: u64,
    pub full: bool,
}

pub fn record(table: Table, counts: &Counts, ids: Option<usize>) {
    for (total, value) in
        READS[table as usize]
            .iter()
            .zip([counts.marked, counts.read, u64::from(counts.full)])
    {
        total.fetch_add(value, Ordering::Relaxed);
    }
    DOMAIN.hours[table as usize].fetch_add(1, Ordering::Relaxed);
    if let Some(ids) = ids {
        IDS.fetch_max(ids, Ordering::Relaxed);
    }
}

pub fn record_carried(table: Table, carried: u64) {
    CARRIED[table as usize].fetch_add(carried, Ordering::Relaxed);
}


pub struct Hooks {
    hooks: Vec<Hook>,
    wargoals: crate::wargoal_changes::Hooks,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; both families
    /// then read every country every hour.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, String> {
        let mut hooks = Vec::new();
        if cfg!(feature = "country_input_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _, _)| crate::located(base, site.rva).is_ok())
            && let Ok(weather) = engine_sites::hook(base, sites::PROVINCE_WEATHER_REBUILD_RVA)
        {
            BASE.store(base, Ordering::Release);
            let callbacks: [usize; SITES] = [
                recalculate_modifiers as *const () as usize,
                add_stability as *const () as usize,
                add_war_support as *const () as usize,
                set_stability as *const () as usize,
                set_war_support as *const () as usize,
                weekly_update as *const () as usize,
                add_air_statistic as *const () as usize,
                replace_relation as *const () as usize,
                set_instigator as *const () as usize,
                set_defender as *const () as usize,
                become_exiled as *const () as usize,
                clear_exile_links as *const () as usize,
                return_from_exile as *const () as usize,
                unappoint_advisor as *const () as usize,
                set_technology as *const () as usize,
                sync_research_slots as *const () as usize,
                set_level as *const () as usize,
                claim_limited_use_bonuses as *const () as usize,
                release_limited_use_bonuses as *const () as usize,
                complete_research_slot as *const () as usize,
                reset_research as *const () as usize,
                set_design_team as *const () as usize,
                set_boosted_by_xp as *const () as usize,
                research_completed as *const () as usize,
                rebuild_or_transfer_b as *const () as usize,
                rebuild_or_transfer_a as *const () as usize,
                add_limited_use_bonus as *const () as usize,
                add_cost_reduction as *const () as usize,
                sharing_roster_add as *const () as usize,
                sharing_roster_remove as *const () as usize,
                sharing_add_member as *const () as usize,
                sharing_remove_member as *const () as usize,
                sharing_bonus as *const () as usize,
                register_license as *const () as usize,
                scientist_categories as *const () as usize,
                research_console as *const () as usize,
                fresh_init as *const () as usize,
                focus_reset as *const () as usize,
                focus_database_clear as *const () as usize,
                continuous_focus as *const () as usize,
                focus_completed as *const () as usize,
                focus_copy as *const () as usize,
                focus_remove_completed as *const () as usize,
                set_focus_tree as *const () as usize,
                add_completed_focus as *const () as usize,
                reload_technologies as *const () as usize,
                grow_intel_network as *const () as usize,
                start_project as *const () as usize,
                stop_project as *const () as usize,
                cancel_project as *const () as usize,
                complete_project as *const () as usize,
                add_project_progress as *const () as usize,
                project_reward_option as *const () as usize,
                forced_project_update as *const () as usize,
                project_database_resync as *const () as usize,
                license_removal as *const () as usize,
                license_extension as *const () as usize,
                faction_set_leader as *const () as usize,
                set_cosmetic_tag as *const () as usize,
                copy_country_colors as *const () as usize,
                resolve_graphical_culture as *const () as usize,
                resolve_graphical_culture_2d as *const () as usize,
                rebuild_localized_name as *const () as usize,
                rebuild_subjects as *const () as usize,
                add_army as *const () as usize,
                remove_army as *const () as usize,
                insert_unit_leader as *const () as usize,
                remove_unit_leader as *const () as usize,
                refresh_neighbors as *const () as usize,
                reload_mio_policies as *const () as usize,
                faction_sharing_base as *const () as usize,
                weather_reinitialize as *const () as usize,
                weather_reset as *const () as usize,
                weather_add_snow as *const () as usize,
                weather_set_mud as *const () as usize,
                weather_set_region as *const () as usize,
                weather_read as *const () as usize,
                weather_initial_passes as *const () as usize,
                weather_reload as *const () as usize,
                weather_console_hours as *const () as usize,
                weather_add_province_modifier as *const () as usize,
                weather_remove_province_modifier as *const () as usize,
                weather_expiry_rebuild as *const () as usize,
                research_progress as *const () as usize,
                set_recruitment_target as *const () as usize,
                set_recruitment_ratio as *const () as usize,
                add_exile_manpower as *const () as usize,
                restore_exile_manpower as *const () as usize,
                use_exile_manpower as *const () as usize,
                set_exile_manpower_cap as *const () as usize,
                add_exile_legitimacy as *const () as usize,
                set_exile_legitimacy as *const () as usize,
                exile_equipment_legitimacy as *const () as usize,
                refresh_wing_crews as *const () as usize,
                refresh_ship_crews as *const () as usize,
                remove_ship_crews as *const () as usize,
                add_sent_expeditionary as *const () as usize,
                remove_sent_expeditionary as *const () as usize,
            ];
            for (index, (site, _, _)) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let original = code.sentinel::<16>()?;
                let hook = if index == sites::WEATHER_EXPIRY_REBUILD {
                    let hook = unsafe { Hook::new_call(code.address, callbacks[index], original) }
                        .map_err(|cause| {
                            format!(
                                "country_input_changes hook {} construction failed: {cause}",
                                site.name
                            )
                        })?;
                    ORIGINAL[index].store(weather.address, Ordering::Release);
                    hook
                } else {
                    let hook = unsafe { Hook::new(code.address, callbacks[index], original) }
                        .map_err(|cause| {
                            format!(
                                "country_input_changes hook {} construction failed: {cause}",
                                site.name
                            )
                        })?;
                    ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                    hook
                };
                hooks.push(hook);
            }
        }
        let wargoals = unsafe { crate::wargoal_changes::Hooks::new(memory, base) }?;
        Ok(Self { hooks, wargoals })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !self.wargoals.set_enabled(enabled)? {
            return Ok(false);
        }
        if !enabled {
            crate::project_variable_changes::project_hooks_enabled(false);
            crate::crew_changes::enabled(false);
            DOMAIN.hooks.store(false, Ordering::SeqCst);
            design_source_availability(false);
            collector_certificates::content_changes::enabled(
                collector_certificates::content_changes::Writer::Technology,
                false,
            );
            crate::world_input_changes::mio_policy_observation(false);
            DOMAIN.apply();
            crate::opinion_changes::set_war_forward(false);
            crate::naval_tree_changes::forwarder(0, false);
            crate::advisor_pool_changes::unappoint_forwarder(false);
            crate::war_changes::forwarder(crate::war_changes::Forwarder::CountryInputs, false);
            crate::nuclear_strikes::forwarder(
                replay_data::rows::NuclearStrikePart::WarSupport,
                false,
            );
        }
        if !Hook::set_family_enabled_shared_named(
            &mut self.hooks,
            enabled,
            "country_input_changes",
            &MUTATIONS.map(|(site, _, _)| site.name),
        )? {
            return Ok(false);
        }
        DOMAIN
            .hooks
            .store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        DOMAIN.apply();
        crate::project_variable_changes::project_hooks_enabled(enabled && !self.hooks.is_empty());
        crate::crew_changes::enabled(enabled && !self.hooks.is_empty());
        design_source_availability(enabled && !self.hooks.is_empty());
        collector_certificates::content_changes::enabled(
            collector_certificates::content_changes::Writer::Technology,
            enabled && !self.hooks.is_empty(),
        );
        crate::world_input_changes::mio_policy_observation(enabled && !self.hooks.is_empty());
        crate::opinion_changes::set_war_forward(enabled && !self.hooks.is_empty());
        crate::naval_tree_changes::forwarder(0, enabled && !self.hooks.is_empty());
        crate::advisor_pool_changes::unappoint_forwarder(enabled && !self.hooks.is_empty());
        crate::war_changes::forwarder(
            crate::war_changes::Forwarder::CountryInputs,
            enabled && !self.hooks.is_empty(),
        );
        crate::nuclear_strikes::forwarder(
            replay_data::rows::NuclearStrikePart::WarSupport,
            enabled && !self.hooks.is_empty(),
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.wargoals.executable_ranges())
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the bitmaps are dropped only once
        // the hooks were never enabled, or were removed and drained after
        // the last capture; the forwarding families are restored with the
        // other families before any hook is dropped.
        unsafe {
            DOMAIN.accumulator.release();
            DOMAIN.sharing.release()
        };
        unsafe { crate::crew_changes::release() };
        for set in &DOMAIN.sets {
            unsafe { set.release() };
        }
    }
}

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
            let _sharing = matches!($site, 30..=32).then(|| DOMAIN.sharing.notify(first));
            let _project_reset = ($site == 54).then(crate::project_variable_changes::resetting);
            let _project = matches!(MUTATIONS[$site].1, Receiver::Project)
                .then(|| crate::project_variable_changes::writing_project(first));
            let wanted = match $site {
                sites::ADD_EXILE_MANPOWER | sites::USE_EXILE_MANPOWER => third as u8 != 0,
                sites::RESTORE_EXILE_MANPOWER => fourth as u8 != 0,
                _ => true,
            };
            let _crew =
                (sites::CREW_WRITERS.contains(&$site)).then(|| crate::crew_changes::notify(first));
            let _marks = (wanted && !sites::CREW_WRITERS.contains(&$site))
                .then(|| DOMAIN.notify($site, first, second));
            let _occupation =
                occupation_exile_receiver($site, first).map(crate::occupation_changes::changed);
            let _mio_policies = ($site == 69).then(crate::world_input_changes::mio_policy_reload);
            let _naval = ($site == 0).then(|| crate::naval_tree_changes::forwarded(0, first));
            let _advisors =
                ($site == 13).then(|| crate::advisor_pool_changes::forwarded_unappoint(first));
            let _wars = match $site {
                8 => crate::war_changes::forwarded(crate::war_changes::Site::SetInstigator),
                9 => crate::war_changes::forwarded(crate::war_changes::Site::SetDefender),
                _ => None,
            };
            let _nuclear = ($site == 2)
                .then(|| crate::nuclear_strikes::war_support(first))
                .flatten();
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe {
                if $site == 0 {
                    design_source_invoke([first, second, third, fourth], original)
                } else if $site == 45 {
                    collector_certificates::content_changes::invoke(
                        collector_certificates::content_changes::Writer::Technology,
                        [first, second, third, fourth],
                        original,
                    )
                } else {
                    original(first, second, third, fourth)
                }
            })
        }
    };
}
callback!(recalculate_modifiers, 0);
callback!(add_stability, 1);
callback!(add_war_support, 2);
callback!(set_stability, 3);
callback!(set_war_support, 4);
callback!(weekly_update, 5);
callback!(add_air_statistic, 6);
unsafe extern "system" fn replace_relation(
    status: usize,
    relation: usize,
    slot: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
    let _marks = DOMAIN.notify(7, status, relation);
    let _tuples = crate::opinion_changes::relation_replaced(status);
    let _wars = crate::war_changes::forwarded(crate::war_changes::Site::ReplaceRelation);
    let _participation = crate::war_changes::participation_change(
        crate::war_changes::Site::ReplaceRelation,
        status,
        0,
    );
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[7].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(status, relation, slot, fourth) })
}
callback!(set_instigator, 8);
callback!(set_defender, 9);
callback!(become_exiled, 10);
callback!(clear_exile_links, 11);
callback!(return_from_exile, 12);
callback!(unappoint_advisor, 13);
callback!(set_technology, 14);
callback!(set_level, 16);
callback!(claim_limited_use_bonuses, 17);
callback!(release_limited_use_bonuses, 18);
callback!(complete_research_slot, 19);
callback!(reset_research, 20);
callback!(set_design_team, 21);
callback!(set_boosted_by_xp, 22);
callback!(research_completed, 23);
callback!(rebuild_or_transfer_b, 24);
callback!(rebuild_or_transfer_a, 25);
callback!(add_limited_use_bonus, 26);
callback!(add_cost_reduction, 27);
callback!(sharing_roster_add, 28);
callback!(sharing_roster_remove, 29);
callback!(sharing_add_member, 30);
callback!(sharing_remove_member, 31);
callback!(sharing_bonus, 32);
callback!(register_license, 33);
callback!(license_removal, 55);
callback!(license_extension, 56);
callback!(faction_set_leader, 57);
callback!(scientist_categories, 34);
callback!(research_console, 35);
callback!(fresh_init, 36);
callback!(focus_reset, 37);
callback!(focus_database_clear, 38);
callback!(continuous_focus, 39);
callback!(focus_completed, 40);
callback!(focus_copy, 41);
callback!(focus_remove_completed, 42);
callback!(set_focus_tree, 43);
callback!(add_completed_focus, 44);
callback!(reload_technologies, 45);
callback!(start_project, 47);
callback!(stop_project, 48);
callback!(cancel_project, 49);
callback!(complete_project, 50);
callback!(add_project_progress, 51);
callback!(forced_project_update, 53);
callback!(set_cosmetic_tag, 58);
callback!(copy_country_colors, 59);
callback!(resolve_graphical_culture, 60);
callback!(resolve_graphical_culture_2d, 61);
callback!(rebuild_localized_name, 62);
callback!(rebuild_subjects, 63);
callback!(add_army, 64);
callback!(remove_army, 65);
callback!(insert_unit_leader, 66);
unsafe extern "system" fn remove_unit_leader(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[REMOVE_UNIT_LEADER].load(Ordering::Acquire)) };
    DOMAIN.remove_leader(
        first,
        &|characters: usize| sites::unit_leader_counts(&memory, characters),
        || timing.original(|| unsafe { original(first, second, third, fourth) }),
    )
}
callback!(refresh_neighbors, 68);
callback!(reload_mio_policies, 69);

macro_rules! callback5 {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
            fifth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
            let _project_reset = ($site == 54).then(crate::project_variable_changes::resetting);
            let _project = matches!(MUTATIONS[$site].1, Receiver::Project)
                .then(|| crate::project_variable_changes::writing_project(first));
            let wanted = match $site {
                sites::ADD_EXILE_MANPOWER | sites::USE_EXILE_MANPOWER => third as u8 != 0,
                sites::RESTORE_EXILE_MANPOWER => fourth as u8 != 0,
                _ => true,
            };
            let _crew =
                (sites::CREW_WRITERS.contains(&$site)).then(|| crate::crew_changes::notify(first));
            let _marks = (wanted && !sites::CREW_WRITERS.contains(&$site))
                .then(|| DOMAIN.notify($site, first, second));
            let _occupation =
                occupation_exile_receiver($site, first).map(crate::occupation_changes::changed);
            let original: unsafe extern "system" fn(usize, usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth, fifth) })
        }
    };
}
callback5!(project_reward_option, PROJECT_REWARD_OPTION);
callback5!(project_database_resync, PROJECT_DATABASE_RESYNC);

unsafe extern "system" fn grow_intel_network(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[GROW_INTEL_NETWORK].load(Ordering::Acquire)) };
    DOMAIN.grow_intel_network(|| {
        timing.original(|| unsafe { original(first, second, third, fourth) })
    })
}

macro_rules! weather_callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            DOMAIN.weather_write($site, || {
                timing.original(|| unsafe { original(first, second, third, fourth) })
            })
        }
    };
}
weather_callback!(weather_reinitialize, sites::WEATHER_WRITERS.start);
weather_callback!(weather_reset, sites::WEATHER_WRITERS.start + 1);
weather_callback!(weather_add_snow, sites::WEATHER_WRITERS.start + 2);
weather_callback!(weather_set_mud, sites::WEATHER_WRITERS.start + 3);
weather_callback!(weather_set_region, sites::WEATHER_WRITERS.start + 4);
weather_callback!(weather_read, sites::WEATHER_WRITERS.start + 5);
weather_callback!(weather_initial_passes, sites::WEATHER_WRITERS.start + 6);
weather_callback!(weather_reload, sites::WEATHER_WRITERS.start + 7);
weather_callback!(weather_console_hours, sites::WEATHER_WRITERS.start + 8);
weather_callback!(
    weather_remove_province_modifier,
    sites::WEATHER_WRITERS.start + 10
);
weather_callback!(weather_expiry_rebuild, sites::WEATHER_EXPIRY_REBUILD);

unsafe extern "system" fn weather_add_province_modifier(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
    fifth: usize,
) -> usize {
    const SITE: usize = sites::WEATHER_WRITERS.start + 9;
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
    let original: unsafe extern "system" fn(usize, usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[SITE].load(Ordering::Acquire)) };
    DOMAIN.weather_write(SITE, || {
        timing.original(|| unsafe { original(first, second, third, fourth, fifth) })
    })
}

unsafe extern "system" fn research_progress(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[sites::RESEARCH_PROGRESS].load(Ordering::Acquire)) };
    DOMAIN.research_progress(first, third, || {
        timing.original(|| unsafe { original(first, second, third, fourth) })
    })
}

unsafe extern "system" fn sync_research_slots(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[SYNC_RESEARCH_SLOTS].load(Ordering::Acquire)) };
    DOMAIN.sync(
        first,
        &|status: usize| sites::slot_roster(&memory, status),
        || timing.original(|| unsafe { original(first, second, third, fourth) }),
    )
}

unsafe extern "system" fn faction_sharing_base(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[FACTION_SHARING_BASE].load(Ordering::Acquire)) };
    DOMAIN.rebase(
        first,
        &|group: usize| sites::faction_sharing_base(&memory, group),
        || timing.original(|| unsafe { original(first, second, third, fourth) }),
    )
}

#[cfg(test)]
mod tests;

/// # Safety
/// The faction acquisition worker owns the native quiescent lease.
pub unsafe fn accumulator_install_growth() {
    unsafe {
        DOMAIN.accumulator.install_growth_quiescent();
    }
}
callback!(set_recruitment_target, sites::SET_RECRUITMENT_TARGET);
callback!(set_recruitment_ratio, sites::SET_RECRUITMENT_RATIO);
callback!(add_exile_manpower, sites::ADD_EXILE_MANPOWER);
callback!(restore_exile_manpower, sites::RESTORE_EXILE_MANPOWER);
callback!(use_exile_manpower, sites::USE_EXILE_MANPOWER);
callback!(set_exile_manpower_cap, sites::SET_EXILE_MANPOWER_CAP);
callback!(add_exile_legitimacy, sites::ADD_EXILE_LEGITIMACY);
callback!(set_exile_legitimacy, sites::SET_EXILE_LEGITIMACY);
callback!(
    exile_equipment_legitimacy,
    sites::EXILE_EQUIPMENT_LEGITIMACY
);

callback!(refresh_wing_crews, sites::REFRESH_WING_CREWS);
callback!(refresh_ship_crews, sites::REFRESH_SHIP_CREWS);
callback!(remove_ship_crews, sites::REMOVE_SHIP_CREWS);

callback!(add_sent_expeditionary, sites::ADD_SENT_EXPEDITIONARY);
callback!(remove_sent_expeditionary, sites::REMOVE_SENT_EXPEDITIONARY);
