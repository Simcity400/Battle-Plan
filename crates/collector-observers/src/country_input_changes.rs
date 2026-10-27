//! Allocation-free notifications from the native writers of every per-country
//! field the stability, research, focus, project, appearance, counter and
//! visibility tables record ([`engine_mutations::country_input_mutations`]). A detour only marks
//! the countries its receiver names in the [`DirtySet`] of each table family
//! its writes reach, counts its site and calls the original; it never
//! changes game behaviour.
//!
//! Nine sets of native country ids (`ccountry::ID`, the census key), each
//! with one consumer, the family that reads it:
//! - [`Table::Stability`]: CountryStabilityInputs and CountryStabilityLines,
//!   read in the Stability job ([`crate::extended::country_stability`]).
//! - [`Table::Research`]: ResearchInputs, ResearchAppliedBonuses and
//!   ResearchModifiers, read in the Research job
//!   ([`crate::extended::governance`]).
//! - [`Table::Technologies`]: ResearchTechnologies (and the
//!   TechnologyDefinitions its rosters reach), read in the Governance job.
//!   Only the writer's own country is marked ([`Receiver::own`]): countries
//!   sharing research read a level for their modifiers, not as a row.
//! - [`Table::Focus`]: Focus and CompletedFocuses, read in the Governance
//!   job, which reads every country after the daily tick
//!   ([`crate::slicing::SliceSelection::reset_daily`]).
//! - [`Table::Projects`]: Projects and ProjectHistory, read in the Gated job
//!   the same way.
//! - [`Table::Appearance`]: CountryAppearance, read in the Countries job.
//!   Its writers: the cosmetic tag, colour copy, graphical culture and
//!   localized name sites, the subject rebuild (the status's country and its
//!   overlord, named again on return), and the forwarded country and
//!   diplomacy resets, ruling party, party leader and character role
//!   writers.
//! - [`Table::Counters`]: CountryCounterInputs, read in the Ordinary counter
//!   part. Its writers: `modifier recalculation`, the war body, the division
//!   roster, unit leader, neighbour and MIO policy sites, and the forwarded
//!   country and diplomacy resets, wartime ally rebuild
//!   ([`Forward::WartimeAllies`]), relation status reset and slot
//!   replacements ([`Forward::RelationReset`], [`Forward::RelationSlot`]),
//!   state owner and controller changes (the old and the new country) and
//!   the state reset, core inserts and erases ([`Forward::Cores`]) and the
//!   `CCountryCharacters` constructor and destructor.
//!
//! - [`Table::Visibility`]: CountryVisibility, read in the Slow job. Its
//!   raw accumulator entry is rebuilt by country modifier recalculation
//!   (site 0) and cleared by the forwarded country appearance reset.
//!   Effective definition and eligibility metadata are still read every hour.
//! - [`Table::Designs`]: demanded equipment-design country modifiers. It
//!   subscribes independently to those same accumulator recalculation/reset
//!   writers; taking these marks never consumes the visibility marks.
//! - [`Table::UnitSpeeds`]: CountryUnitSpeedEntries, read with the division
//!   presentation ([`crate::extended::division_presentation`]). Every writer
//!   that marks [`Table::Counters`] marks it too, so it certifies exactly what
//!   the counter set does (country modifier recalculation rebuilds the
//!   accumulator its entries are read from); a set has one consumer, so the
//!   two families never take each other's marks.
//!
//! A marked country is read within the hour; an unmarked one keeps the rows
//! of its last read until its daily slice. Writers whose address another family patches are forwarded by that
//! family ([`forwarded`], [`Forward`]), and a set observes only while the
//! domain's own hooks and the patches of every family forwarding a writer
//! of that set are enabled ([`forwarder`]). A receiver that names no
//! country, an id beyond the bitmap, a load or a gap moves the set's epoch,
//! so the next hour reads every country. A writer naming several countries
//! (a war's two sides, a level every country sharing research reads, a
//! state's old and new owner) marks each on entry and again on return, and
//! the subject rebuild names its overlord again on return
//! ([`Receiver::renames`]).
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
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

/// Attach/detach registration while native callbacks are quiescent.
/// Modifier recalculation is patched once; source dependants wrap its original.
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
/// The module base receivers resolve country link ids against
/// ([`sites::of_link`]); stored before any hook is enabled.
static BASE: AtomicUsize = AtomicUsize::new(0);
/// Per [`Table`] and [`Counts`] field, cumulative.
static READS: [[AtomicU64; 3]; SETS] = [const { [const { AtomicU64::new(0) }; 3] }; SETS];
/// Countries carried by a per-country input store.
static CARRIED: [AtomicU64; SETS] = [const { AtomicU64::new(0) }; SETS];
/// Country id bound of the latest complete census, for off-thread sizing.
static IDS: AtomicUsize = AtomicUsize::new(0);

/// A table family the domain observes.
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

/// One set per [`Table`].
const SETS: usize = 10;

/// Per [`Table`], whether its marks select countries at all
/// (`technology_changes`, `focus_changes`, `projects_changes`,
/// `appearance_changes`, `counter_changes`, `visibility_changes`); a set
/// without them certifies nothing, so its family reads every country every hour.
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

/// A writer another family patches and forwards ([`forwarded`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forward {
    /// Country appearance reset (country), through
    /// [`crate::politics_changes`]: every country field, the tree and the
    /// bonus rosters. country dynamic reuse calls it
    /// before the original-tag setter, the only
    /// runtime writer of the original tag (`ccountry::ORIGINAL_TAG`) the appearance row
    /// records, in the same game-thread step, and every read of an hour
    /// finishes inside the hourly callback, so the reset's mark covers it.
    ResetCountry,
    /// Set politics, force ruling party and
    /// Set ruling party  (status), through
    /// [`crate::political_status_changes`]: the ruling party and its leader,
    /// whose ideology and character the appearance row records.
    RulingParty,
    /// The idea writer `set ideas(status)`: the active ideas, and the
    /// technologies' idea contributions through both technology contribution writers.
    Ideas,
    /// Add popularity, set popularity redistributing and the
    /// clamped-setter CALLs party popularity effect call, popularities effect call (party).
    Popularity,
    /// Every party leader vector writer (party), through
    /// [`crate::party_leader_changes`].
    PartyLeaders,
    /// The country leader and advisor role writers, through
    /// [`crate::advisor_changes`], with the character they write.
    Roles,
    /// Scientist role build, attach and clear, through
    /// [`crate::roster_changes`], with the character.
    ScientistRoles,
    /// Scientist trait writers (scientist), through [`crate::roster_changes`].
    ScientistTraits,
    /// Diplomacy reset, which clears the exile host
    /// other countries' war flags read, through [`crate::relation_changes`].
    DiplomacyReset,
    /// National focus selection and cancellation
    /// (progress), through [`crate::politics_changes`].
    Focus,
    /// The character name setter and identity repair on the character, through
    /// [`crate::advisor_changes`]: a scientist's saved name and id, which a technology's scientist
    /// source rows carry.
    CharacterIdentity,
    /// Wartime ally rebuild (diplomacy), through
    /// [`crate::relation_changes`]: the co-belligerent bytes
    /// The co-belligerent byte (`crelation_status::CO_BELLIGERENT`) of the status's relation matrix and its
    /// candidate vectors (enemies and wartime allies).
    WartimeAllies,
    /// State owner setter (state, tag, _, _), through
    /// [`crate::territory_changes`]: the owned states of the old and
    /// the new owner.
    StateOwner,
    /// State controller setter (state, tag, _, _), through
    /// [`crate::territory_changes`]: the controlled states of the
    /// old and the new controller.
    StateController,
    /// The state reset `reset state(state)`, through
    /// [`crate::territory_changes`]: clears both tags without naming their
    /// countries.
    StateReset,
    /// `add core(state, &tag)` and remove core, through [`crate::raid_changes`]: the tag's core
    /// states.
    Cores,
    /// Relation status reset (status), through
    /// [`crate::opinion_changes`]: the owner's co-belligerent byte, war and
    /// relation slots.
    RelationReset,
    /// The fifteen typed relation slot replacements (status, relation,
    /// slot), through [`crate::opinion_changes`]: the owner's slot-zero
    /// subject and access relations.
    RelationSlot,
    /// Country character pool construction and destruction, through
    /// [`crate::advisor_pool_changes`]: a country's The country characters and the unit leader
    /// rosters the counter rows read.
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

    fn name(self) -> &'static str {
        match self {
            Self::ResetCountry => "reset_country",
            Self::RulingParty => "ruling_party",
            Self::Ideas => "ideas",
            Self::Popularity => "popularity",
            Self::PartyLeaders => "party_leaders",
            Self::Roles => "character_roles",
            Self::ScientistRoles => "scientist_roles",
            Self::ScientistTraits => "scientist_traits",
            Self::DiplomacyReset => "diplomacy_reset",
            Self::Focus => "focus",
            Self::CharacterIdentity => "character_identity",
            Self::WartimeAllies => "wartime_allies",
            Self::StateOwner => "state_owner",
            Self::StateController => "state_controller",
            Self::StateReset => "state_reset",
            Self::Cores => "cores",
            Self::RelationReset => "relation_reset",
            Self::RelationSlot => "relation_slot",
            Self::CountryCharacters => "country_characters",
        }
    }

    /// How the forwarded first argument names the country.
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
            // The ruling party's first leader: its ideology and character.
            Self::RulingParty | Self::PartyLeaders | Self::Roles => {
                Tables::STABILITY.with(Tables::APPEARANCE)
            }
            // The exile host, the subject and wartime ally relations.
            Self::DiplomacyReset => Tables::STABILITY
                .with(Tables::APPEARANCE)
                .with(Tables::COUNTERS),
            Self::Focus => Tables::FOCUS,
            // The old and the new owner's owned-state count, which gates their daily research,
            // focus and project steps (daily country update) and so whether those families read
            // them at their slice or after the daily tick ([`drifts`]): a country that owned a
            // state only across a daily tick, between two of those checks, is read when its
            // ownership changes.
            Self::StateOwner | Self::StateReset => Tables::COUNTERS.with(DRIFTING),
            Self::WartimeAllies
            | Self::StateController
            | Self::Cores
            | Self::RelationReset
            | Self::RelationSlot
            | Self::CountryCharacters => Tables::COUNTERS,
        }
    }

    /// The family that patches the writer and forwards it.
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

/// The families whose rows drift with the daily tick only for a country
/// owning a state ([`drifts`]): ResearchTechnologies (with the research
/// inputs [`Tables::TECHNOLOGY`] carries), Focus and Projects.
const DRIFTING: Tables = Tables::TECHNOLOGY
    .with(Tables::FOCUS)
    .with(Tables::PROJECTS);

/// Per set ([`Table`] index), whether a write reaching `tables` reaches it.
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

/// A family that forwards writers it patches ([`forwarder`]).
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

    fn name(self) -> &'static str {
        match self {
            Self::Politics => "politics_changes",
            Self::PoliticalStatus => "political_status_changes",
            Self::PartyLeaders => "party_leader_changes",
            Self::Roles => "advisor_changes",
            Self::Roster => "roster_changes",
            Self::Relations => "relation_changes",
            Self::Territory => "territory_changes",
            Self::Raid => "raid_changes",
            Self::Opinions => "opinion_changes",
            Self::AdvisorPool => "advisor_pool_changes",
        }
    }

    /// Whether a writer this family forwards reaches set `set`: a set
    /// observes only while each such family's patches are enabled.
    fn reaches(self, set: usize) -> bool {
        Forward::ALL
            .iter()
            .any(|forward| forward.forwarder() == self && table_flags(forward.tables())[set])
    }
}

/// One observation domain: a set per [`Table`] and each site's calls.
pub struct Domain {
    sharing: WatchRegistry,
    accumulator: WatchRegistry,
    sets: [DirtySet; SETS],
    calls: [AtomicU64; SITES],
    /// Forwarded calls per [`Forward`].
    forwarded: [AtomicU64; Forward::ALL.len()],
    /// Receivers that named no country.
    unnamed: AtomicU64,
    /// `research slot synchronization` calls that changed the slot roster.
    resynced: AtomicU64,
    /// Faction sharing base setter calls that changed the base.
    rebased: AtomicU64,
    /// `RemoveUnitLeader` calls that changed a leader roster.
    removed_leaders: AtomicU64,
    /// Writes of IntelNetworks outside the daily step: forwarded
    /// `country appearance reset` calls and the console's grow_intel_network,
    /// each counted on entry and on return ([`intel_network_writes`]).
    network_writes: AtomicU64,
    /// Calls of the province weather writers outside the native window
    /// ([`sites::WEATHER_WRITERS`]), each counted on entry and on return
    /// ([`weather_writes`]).
    weather_writes: AtomicU64,
    /// This family's hooks enabled; with [`Self::forwarders`], each set's
    /// observation ([`Self::apply`]).
    hooks: AtomicBool,
    forwarders: [AtomicBool; Forwarder::ALL.len()],
    /// Per [`Table`], the hours its family recorded ([`record`]).
    hours: [AtomicU64; SETS],
}

type Sets<'a> = [Option<&'a DirtySet>; SETS];

/// A running writer's marks; dropping them marks its return.
#[must_use]
pub struct Marks<'a> {
    /// The sets the receiver names countries for, and the technology set
    /// when it names only the writer's own country ([`Receiver::own`]).
    _parts: [Part<'a>; 2],
    _occupation: Option<crate::watch_registry::Notification<'static>>,
    _wargoals: Option<crate::wargoal_changes::Guard>,
    _project_reset: Option<crate::project_variable_changes::Reset>,
    _accumulator: Option<crate::watch_registry::Notification<'a>>,
    _market: Option<crate::market_changes::CountryReset>,
}

#[derive(Default)]
struct Part<'a> {
    #[allow(dead_code)]
    held: [Option<Dirty<'a>>; SETS],
    /// A writer that named several countries: each is marked again on
    /// return, before the held guards.
    again: Option<Again<'a>>,
}

struct Again<'a> {
    sets: Sets<'a>,
    receiver: Receiver,
    first: usize,
    second: usize,
}

impl<'a> Marks<'a> {
    /// Nothing recorded is written.
    pub fn none() -> Self {
        Self {
            _parts: [Part::default(), Part::default()],
            _occupation: None,
            _wargoals: None,
            _project_reset: None,
            _accumulator: None,
            _market: None,
        }
    }
}

impl<'a> Part<'a> {
    /// Every country of `sets`: their epochs move on entry and on return.
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

/// Hook path, allocation-free: the countries `receiver` names for a call
/// with `first` and `second` ([`sites::countries`]), through guarded reads of
/// live memory.
fn name(receiver: Receiver, first: usize, second: usize, visit: &mut dyn FnMut(u32)) -> bool {
    // SAFETY: guarded reads of live memory on the calling thread.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
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

    /// The sets of `tables` whose rows `receiver` names; with `own`, the
    /// technology set too ([`Receiver::own`]).
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

    /// Marks every country `receiver` names for a call with `first` and
    /// `second` in the sets of `tables`, or moves their epochs when one
    /// cannot be named. The marks cover the call until dropped. The
    /// technology set is marked for the writer's own country only.
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

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// is about to run with `first` and `second` its first two arguments.
    pub fn notify(&self, site: usize, first: usize, second: usize) -> Marks<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        let mut marks = self.mark(MUTATIONS[site].2, MUTATIONS[site].1, first, second);
        marks._accumulator = (site == 0).then(|| self.accumulator.notify(first));
        marks
    }

    /// `sync research slots(status)` around `original`: the daily
    /// tick runs it for every country, but it changes the slot roster only
    /// when the country's slot count moved, so the country is marked only
    /// when the roster header (`roster`, [`sites::slot_roster`]) differs after
    /// the original, or over the whole call when it cannot be read first.
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

    /// The faction sharing base setter on `group` with `count` around
    /// `original`: the daily faction refresh calls it for every faction, so
    /// the members are marked only when the base (`base`,
    /// [`sites::faction_sharing_base`]) differs after the original, or over
    /// the whole call when it cannot be read first.
    fn rebase(
        &self,
        group: usize,
        base: &dyn Fn(usize) -> Option<[u8; 16]>,
        original: impl FnOnce() -> usize,
    ) -> usize {
        self.when_changed(FACTION_SHARING_BASE, group, base, &self.rebased, original)
    }

    /// `remove unit leader(characters, leader,
    /// flag)` around `original`: the deferred hourly queue calls it for
    /// leaders no roster lists, so the owner is marked only when a roster
    /// count (`counts`, [`sites::unit_leader_counts`]) differs after the
    /// original, or over the whole call when it cannot be read first.
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

    /// A writer at `site` whose receiver is `first` around `original`, marked
    /// only when `state` of `first` differs after the original (counted in
    /// `changed`), or over the whole call when it cannot be read first.
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

    /// Hook path: a writer another family patches, with `first` and `second`
    /// its first two arguments.
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
        marks
    }

    /// The counted intel network writes, while the hooks and the politics
    /// family that forwards the country reset are enabled.
    fn intel_network_writes(&self) -> Option<u64> {
        let observed = self.hooks.load(Ordering::SeqCst)
            && self.forwarders[Forwarder::Politics as usize].load(Ordering::SeqCst);
        observed.then(|| self.network_writes.load(Ordering::SeqCst))
    }

    fn intel_network_resets(&self) -> Option<u64> {
        self.intel_network_writes()
            .map(|_| self.forwarded[Forward::ResetCountry as usize].load(Ordering::Acquire))
    }

    /// The counted province weather writes, while the hooks are enabled and
    /// the weather read is configured (`weather_changes`).
    fn weather_writes(&self) -> Option<u64> {
        (cfg!(feature = "weather_changes") && self.hooks.load(Ordering::SeqCst))
            .then(|| self.weather_writes.load(Ordering::SeqCst))
    }

    /// `technology research progress adder(technology, _, source, _)`
    /// around `original`: an effect's progress (`source` 2) marks the
    /// technology's country; every other source is the daily drift, runs
    /// inside a hooked write, or (the tech tree's GUI handler) is followed
    /// in the same call by hooked writes ([`sites::EFFECT_PROGRESS_SOURCE`]).
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

    /// A province weather writer `site` around `original`, counted on entry
    /// and on return, so an hour whose read overlaps it reads every record.
    fn weather_write(&self, site: usize, original: impl FnOnce() -> usize) -> usize {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.weather_writes.fetch_add(1, Ordering::SeqCst);
        let result = original();
        self.weather_writes.fetch_add(1, Ordering::SeqCst);
        result
    }

    /// The console's grow_intel_network around `original`, counted on entry
    /// and on return so a read while it runs is repeated.
    fn grow_intel_network(&self, original: impl FnOnce() -> usize) -> usize {
        self.calls[GROW_INTEL_NETWORK].fetch_add(1, Ordering::Relaxed);
        self.network_writes.fetch_add(1, Ordering::SeqCst);
        let result = original();
        self.network_writes.fetch_add(1, Ordering::SeqCst);
        result
    }

    /// Observe a set only while the hooks and every forwarder of a writer
    /// reaching it ([`Forwarder::reaches`]) are enabled; enabling starts an
    /// epoch.
    fn apply(&self) {
        self.sharing.set_enabled(self.hooks.load(Ordering::SeqCst));
        self.accumulator.set_enabled(
            self.hooks.load(Ordering::SeqCst)
                && self.forwarders[Forwarder::Politics as usize].load(Ordering::SeqCst),
        );
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

    /// An enabled domain with bitmaps for `ids` countries installed.
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

    /// A writer of `table` changes country `id` and returns.
    #[cfg(any(test, feature = "test-support"))]
    pub fn write(&self, table: Table, id: u32) {
        drop(self.set(table).notify(id as usize));
    }

    /// A writer of `table` that names no country runs and returns.
    #[cfg(any(test, feature = "test-support"))]
    pub fn write_every(&self, table: Table) {
        drop(self.set(table).notify_every());
    }

    /// The hourly boundary; `invalidate` starts an epoch.
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

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// The single consumer's hourly take of the countries marked for `table`
    /// ([`DirtySet::take`]); `None` certifies nothing.
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

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn take(&self, table: Table, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        DOMAIN.take(table, visit)
    }
}

/// Whether a country's rows drift with the daily tick. Research, focus, program ticks and the
/// project pool pass run only for a country owning a State  (an owned-state count of at least 1); a
/// stateless country returns after its units and war checks. An unreadable count drifts.
pub fn drifts(memory: &impl Memory, country: usize) -> bool {
    country
        .checked_add(engine_native::ccountry::OWNED_STATE_COUNT)
        .and_then(|address| engine_native::read::<i32>(memory, address))
        .is_none_or(|count| count >= 1)
}

/// The MIO policy reload is installed and enabled with this existing family.
pub fn mio_policies_observed() -> bool {
    DOMAIN.hooks.load(Ordering::SeqCst)
}

/// A count moving with every IntelNetworks write outside the daily step,
/// or `None` while unobserved ([`crate::extended::intel`]). Country dynamic
/// reuse runs the forwarded appearance reset before the strategic operative
/// reset destroys that country's networks; the console's intel network
/// growth command sets their local strengths.
pub fn intel_network_writes() -> Option<u64> {
    DOMAIN.intel_network_writes()
}

/// Existing forwarded reset occurrences distinguish destroyed network
/// lifetimes from console growth, which retains the national caches.
/// Reading this certificate adds no callback work.
pub fn intel_network_resets() -> Option<u64> {
    DOMAIN.intel_network_resets()
}

/// A count moving with every province weather writer outside the hourly
/// window: reseed, reset, snow/mud/region console commands, load reading,
/// post-load initial passes, reload and the console's hourly game-state
/// command. `None` while unobserved ([`crate::extended::weather`]).
pub fn weather_writes() -> Option<u64> {
    DOMAIN.weather_writes()
}

/// Notifications running now.
pub fn active() -> usize {
    DOMAIN.accumulator.active()
        + DOMAIN.sharing.active()
        + DOMAIN.sets.iter().map(|set| set.active()).sum::<usize>()
        + crate::crew_changes::active()
        + crate::wargoal_changes::active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
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

/// A forwarding family's patches enabled or disabled; outside attach and
/// detach only. The domain observes only while every forwarder is enabled.
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

/// Hook path, allocation-free: the forwarded writer `site` is about to run
/// with `first` and `second` its first two arguments (the role's character
/// for [`Forward::Roles`] and [`Forward::ScientistRoles`]). Hold the result
/// over the native call.
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

// Exile operands have diplomacy certificates, separate from modifier-source certificates.
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

/// A generic variable writer of a store the selected project census registered.
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
        _market: None,
    }
}

/// The writer a political status site forwards
/// ([`engine_mutations::political_status_mutations::MUTATIONS`] index): the capital
/// setter writes nothing recorded here.
fn status_forward(site: usize) -> Option<Forward> {
    match site {
        // set_politics, force_ruling_party, set_ruling_party.
        1..=3 => Some(Forward::RulingParty),
        // set_ideas.
        4 => Some(Forward::Ideas),
        // add_popularity, set_popularity_redistributing and the two
        // clamped-setter CALLs.
        5..=8 => Some(Forward::Popularity),
        _ => None,
    }
}

/// Hook path: the political status family's site `site` with `first` its
/// first argument (the status, or the party for popularity).
pub fn forwarded_status(site: usize, first: usize) -> Marks<'static> {
    match status_forward(site) {
        Some(forward) => forwarded(forward, first, 0),
        None => Marks::none(),
    }
}

/// What a character role site ([`engine_mutations::character_role_mutations`])
/// writes: a stability source (the country leader and advisor modifiers,
/// role creation and destruction and the leader ideology), or, for the name
/// and repair sites, a scientist source's name and id the research
/// modifiers read.
fn role_forward(site: usize) -> Forward {
    use engine_mutations::character_role_mutations as roles;
    match site {
        roles::CHARACTER_NAME | roles::CHARACTER_REPAIR => Forward::CharacterIdentity,
        _ => Forward::Roles,
    }
}

/// Hook path: the advisor family's character role site `site`, with the
/// character it resolved (`None`: it names none, so every country).
pub fn forwarded_role(site: usize, character: Option<usize>) -> Marks<'static> {
    forwarded(role_forward(site), character.unwrap_or(0), 0)
}

/// The writer and receiver a roster site forwards
/// ([`engine_mutations::character_roster_mutations`]): the scientist build names its
/// character in RDX, attach and clear in RCX and the trait writers their
/// scientist in RCX; the roster array sites write nothing read here.
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

/// Hook path: the roster family's site `site` with its first two arguments.
pub fn forwarded_roster(site: usize, first: usize, second: usize) -> Marks<'static> {
    match roster_forward(site, first, second) {
        Some((forward, receiver)) => forwarded(forward, receiver, 0),
        None => Marks::none(),
    }
}

/// The writer a territory site forwards
/// ([`engine_mutations::territory_mutations::MUTATIONS`] index): the state owner and
/// controller setters and the state reset write the counter rows' state
/// lists.
fn territory_forward(site: usize) -> Option<Forward> {
    match site {
        // State owner setter, state controller setter.
        7 => Some(Forward::StateOwner),
        8 => Some(Forward::StateController),
        // The state reset.
        9 => Some(Forward::StateReset),
        _ => None,
    }
}

/// Hook path: the territory family's site `site` with its first two
/// arguments (the state and, for the owner and controller, the new tag in
/// EDX).
pub fn forwarded_territory(site: usize, first: usize, second: usize) -> Marks<'static> {
    match territory_forward(site) {
        Some(forward) => forwarded(forward, first, second),
        None => Marks::none(),
    }
}

/// Whether a raid site ([`engine_mutations::raid_mutations::MUTATIONS`] index) is a
/// core insert or erase, `(state, &tag)`.
fn raid_forward(site: usize) -> Option<Forward> {
    match site {
        // Add core, remove core.
        6 | 7 => Some(Forward::Cores),
        _ => None,
    }
}

/// Hook path: the raid family's site `site` with its first two arguments.
pub fn forwarded_raid(site: usize, first: usize, second: usize) -> Marks<'static> {
    match raid_forward(site) {
        Some(forward) => forwarded(forward, first, second),
        None => Marks::none(),
    }
}

/// Whether an advisor pool site
/// ([`engine_mutations::advisor_pool_mutations::MUTATIONS`] index) constructs or
/// destroys a `CCountryCharacters`.
fn pool_forward(site: usize) -> Option<Forward> {
    match site {
        // Country character pool construction and destruction.
        6 | 7 => Some(Forward::CountryCharacters),
        _ => None,
    }
}

/// Hook path: the advisor pool family's site `site`.
pub fn forwarded_pool(site: usize) -> Marks<'static> {
    match pool_forward(site) {
        Some(forward) => forwarded(forward, 0, 0),
        None => Marks::none(),
    }
}

/// Off the game thread: size both sets for the country ids the latest
/// census saw. Nothing is offered before a capture walked them: a later
/// larger bitmap waits for an invalidating hour, while the first installs
/// at once.
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
/// Off-thread registry demand, available before the first hourly census.
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

/// Off the game thread: free replaced bitmaps.
pub fn collect() {
    DOMAIN.accumulator.collect();
    DOMAIN.sharing.collect();
    crate::crew_changes::collect();
    crate::wargoal_changes::collect();
    for set in &DOMAIN.sets {
        set.collect();
    }
}

/// One hour's read of a table family.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Countries the take marked.
    pub marked: u64,
    /// Countries read.
    pub read: u64,
    /// The take certified nothing: every country was read.
    pub full: bool,
}

/// A worker publishes one hour's counts and, after a complete census, the
/// census's country id bound for sizing.
pub fn record(table: Table, counts: &Counts, ids: Option<usize>) {
    for (total, value) in READS[table as usize].iter().zip([
        counts.marked,
        counts.read,
        u64::from(counts.full),
    ]) {
        total.fetch_add(value, Ordering::Relaxed);
    }
    DOMAIN.hours[table as usize].fetch_add(1, Ordering::Relaxed);
    if let Some(ids) = ids {
        IDS.fetch_max(ids, Ordering::Relaxed);
    }
}

/// A worker publishes the countries whose raw inputs it carried this hour.
pub fn record_carried(table: Table, carried: u64) {
    CARRIED[table as usize].fetch_add(carried, Ordering::Relaxed);
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&DOMAIN.calls)
            .map(|((site, _, _), n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let forwarded = crate::status::map(
        Forward::ALL
            .iter()
            .zip(&DOMAIN.forwarded)
            .map(|(site, n)| (site.name(), n.load(Ordering::Relaxed))),
    );
    let forwarders = crate::status::map(
        Forwarder::ALL
            .iter()
            .zip(&DOMAIN.forwarders)
            .map(|(family, enabled)| (family.name(), enabled.load(Ordering::Relaxed))),
    );
    let table = |table: Table| {
        let reads = READS[table as usize]
            .each_ref()
            .map(|value| value.load(Ordering::Relaxed));
        crate::status::status!({
            "carried_countries": CARRIED[table as usize].load(Ordering::Relaxed),
            "marked_countries": reads[0],
            "read_countries": reads[1],
            "full_reads": reads[2],
            "hours": DOMAIN.hours[table as usize].load(Ordering::Relaxed),
            "set": DOMAIN.set(table).status(),
        })
    };
    crate::status::status!({
        "configured": cfg!(feature = "country_input_changes"),
        "sets_configured": CONFIGURED,
        "observed": DOMAIN.hooks.load(Ordering::Relaxed),
        "poisoned": DOMAIN.sets.iter().any(|set| set.poisoned()),
        "stability": table(Table::Stability),
        "research": table(Table::Research),
        "technologies": table(Table::Technologies),
        "focus": table(Table::Focus),
        "projects": table(Table::Projects),
        "appearance": table(Table::Appearance),
        "counters": table(Table::Counters),
        "visibility": table(Table::Visibility),
        "acquired_crews": crate::crew_changes::status(),
        "wargoals": crate::wargoal_changes::status(),
        "designs": table(Table::Designs),
        "unit_speeds": table(Table::UnitSpeeds),
        "events": events,
        "forwarded": forwarded,
        "forwarders": forwarders,
        "unnamed_receivers": DOMAIN.unnamed.load(Ordering::Relaxed),
        "resynced_slot_rosters": DOMAIN.resynced.load(Ordering::Relaxed),
        "weather_writes": DOMAIN.weather_writes.load(Ordering::Relaxed),
        "weather": weather_carry::status(),
        "changed_faction_sharing_bases": DOMAIN.rebased.load(Ordering::Relaxed),
        "changed_leader_rosters": DOMAIN.removed_leaders.load(Ordering::Relaxed),
        "scope": "cumulative since attach; the stability, research, technology, appearance, counter and visibility families read, beside their daily slice (the focus and projects families: every country after the daily tick), every country a hooked or forwarded writer marked since the previous hour (marked_countries); an unmarked country keeps the rows of its last read only while the hour's take returned the held epoch and the per-read globals are unchanged (visibility holds raw accumulator presence/value and recomputes the effective value hourly), and otherwise every country is read (full_reads); the research and technology families carry an unmarked drifting country at its slice, reading only its daily drift afresh; a family's set observes only while the hooks and every family forwarding one of its writers are enabled (forwarders)"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
    wargoals: crate::wargoal_changes::Hooks,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
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
            && MUTATIONS.iter().all(|(site, _, _)| {
                crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel)
            })
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
                let hook = if index == sites::WEATHER_EXPIRY_REBUILD {
                    let hook =
                        unsafe { Hook::new_call(base + site.rva, callbacks[index], site.sentinel) }
                            .map_err(|cause| {
                                format!(
                                    "country_input_changes hook {} construction failed: {cause}",
                                    site.name
                                )
                            })?;
                    // The detour calls the CALL's target itself.
                    ORIGINAL[index].store(
                        base + sites::PROVINCE_WEATHER_REBUILD_RVA,
                        Ordering::Release,
                    );
                    hook
                } else {
                    let hook =
                        unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
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
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
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
        // The relation tuples observe the war body here.
        crate::opinion_changes::set_war_forward(enabled && !self.hooks.is_empty());
        // The naval tree family observes modifier recalculation, and the advisor
        // pool family the unappointment helper, through this family's patches
        // ([`crate::naval_tree_changes::forwarded`],
        // [`crate::advisor_pool_changes::forwarded_unappoint`]).
        crate::naval_tree_changes::forwarder(0, enabled && !self.hooks.is_empty());
        crate::advisor_pool_changes::unappoint_forwarder(enabled && !self.hooks.is_empty());
        // The war domain observes the war body and the instigator setters
        // through this family's patches ([`crate::war_changes`]).
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

// Each site takes at most four integer or pointer arguments (RCX..R9) and
// returns RAX unchanged from its original; the first two name the countries
// as its [`Receiver`] says. The marks cover the original in flight.
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
            // Ordinary manpower operations do not write exile resources.
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
            // Modifier recalculation also rebuilds the country's naval hover
            // tree, whose family this patch forwards ([`crate::naval_tree_changes`]).
            let _naval = ($site == 0).then(|| crate::naval_tree_changes::forwarded(0, first));
            // The unappointment helper also changes the pool's appointed
            // advisors, whose family this patch forwards
            // ([`crate::advisor_pool_changes`]).
            let _advisors =
                ($site == 13).then(|| crate::advisor_pool_changes::forwarded_unappoint(first));
            // SetInstigator and SetDefender write a war's recorded instigator
            // byte, which the war domain forwards ([`crate::war_changes`]).
            let _wars = match $site {
                8 => crate::war_changes::forwarded(crate::war_changes::Site::SetInstigator),
                9 => crate::war_changes::forwarded(crate::war_changes::Site::SetDefender),
                _ => None,
            };
            // add_war_support applies a strike's war support loss, which the
            // nuclear strike family observes ([`crate::nuclear_strikes`]).
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
// Country modifier recalculation (country).
callback!(recalculate_modifiers, 0);
// Add stability (country, delta).
callback!(add_stability, 1);
// Add war support (country, delta).
callback!(add_war_support, 2);
// Set stability (country, value).
callback!(set_stability, 3);
// Set war support (country, value).
callback!(set_war_support, 4);
// Weekly update (country).
callback!(weekly_update, 5);
// Add air statistic (country, column, value).
callback!(add_air_statistic, 6);
// Replace relation (status, new, slot), the war
// body of the relation slot replacements: the relation tuples observe it too
// (`opinion_changes::relation_replaced`).
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
// Set instigator (war, tag, _).
callback!(set_instigator, 8);
// Set defender (war, tag, _).
callback!(set_defender, 9);
// Become exiled (diplomacy, host, _).
callback!(become_exiled, 10);
// Clear exile links (diplomacy).
callback!(clear_exile_links, 11);
// Return from exile (diplomacy).
callback!(return_from_exile, 12);
// Unappoint advisor (characters, advisor, flag).
callback!(unappoint_advisor, 13);
// Set technology (slot, technology, _).
callback!(set_technology, 14);
// Set level (technology, level).
callback!(set_level, 16);
// Claim limited use bonuses (technology).
callback!(claim_limited_use_bonuses, 17);
// Release limited use bonuses (technology).
callback!(release_limited_use_bonuses, 18);
// Complete research slot (status, slot, _).
callback!(complete_research_slot, 19);
// Reset research (technology).
callback!(reset_research, 20);
// Set design team (technology, organisation).
callback!(set_design_team, 21);
// Set boosted by xp (technology, flag).
callback!(set_boosted_by_xp, 22);
// Research completed (status, technology, _).
callback!(research_completed, 23);
// Rebuild or transfer b (destination, source).
callback!(rebuild_or_transfer_b, 24);
// Rebuild or transfer a (status).
callback!(rebuild_or_transfer_a, 25);
// Add limited use bonus (status).
callback!(add_limited_use_bonus, 26);
// Add cost reduction (status).
callback!(add_cost_reduction, 27);
// Sharing roster add (status, group).
callback!(sharing_roster_add, 28);
// Sharing roster remove (status, group).
callback!(sharing_roster_remove, 29);
// Sharing add member (group, tag).
callback!(sharing_add_member, 30);
// Sharing remove member (group, tag).
callback!(sharing_remove_member, 31);
// Sharing bonus (group, _, bonus).
callback!(sharing_bonus, 32);
// Register license (status).
callback!(register_license, 33);
// The licence removal (licence).
callback!(license_removal, 55);
// The licence extension (licence, variant).
callback!(license_extension, 56);
// Faction set leader (faction, country).
callback!(faction_set_leader, 57);
// Scientist categories (program status).
callback!(scientist_categories, 34);
// Research console (out, arguments).
callback!(research_console, 35);
// Technology status initialization(status).
callback!(fresh_init, 36);
// Progress reset (progress).
callback!(focus_reset, 37);
// The focus database clear (database).
callback!(focus_database_clear, 38);
// Continuous focus select/clear (progress, focus).
callback!(continuous_focus, 39);
// Focus completed (progress, focus, origin, _).
callback!(focus_completed, 40);
// Focus state copy (destination, source).
callback!(focus_copy, 41);
// Completed focus removal (progress, focus).
callback!(focus_remove_completed, 42);
// Set focus tree (country, tree, keep).
callback!(set_focus_tree, 43);
// Add completed focus (progress, focus, origin).
callback!(add_completed_focus, 44);
// Reload technologies (out, arguments).
callback!(reload_technologies, 45);
// Start project (project, program).
callback!(start_project, 47);
// Stop project (project, cancel).
callback!(stop_project, 48);
// Cancel project (project).
callback!(cancel_project, 49);
// Complete project (project, scientist, province).
callback!(complete_project, 50);
// Add project progress (project, ratio).
callback!(add_project_progress, 51);
// The forced updater (project).
callback!(forced_project_update, 53);
// Set cosmetic tag (country, name).
callback!(set_cosmetic_tag, 58);
// Copy country colors (destination, source).
callback!(copy_country_colors, 59);
// Resolve graphical culture (country, name).
callback!(resolve_graphical_culture, 60);
// The 2D graphical culture resolver (country, name).
callback!(resolve_graphical_culture_2d, 61);
// Rebuild localized name (country).
callback!(rebuild_localized_name, 62);
// The subject rebuild (diplomacy): its country and overlord, the
// overlord named again on return.
callback!(rebuild_subjects, 63);
// Add army (country, army, _, _): one guarded read of the
// country's id per new division.
callback!(add_army, 64);
// Remove army (country, army) -> removed.
callback!(remove_army, 65);
// The unit leader insert (characters, &leader).
callback!(insert_unit_leader, 66);
// Remove unit leader (characters, character,
// flag) -> removed.
/// `remove unit leader(characters, leader,
/// flag)`: notifies only a changed leader roster ([`Domain::remove_leader`]).
unsafe extern "system" fn remove_unit_leader(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
    // SAFETY: guarded reads of live memory on the calling thread.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[REMOVE_UNIT_LEADER].load(Ordering::Acquire)) };
    DOMAIN.remove_leader(
        first,
        &|characters: usize| sites::unit_leader_counts(&memory, characters),
        || timing.original(|| unsafe { original(first, second, third, fourth) }),
    )
}
// The neighbour rebuild (country).
callback!(refresh_neighbors, 68);
// The MIO policy reload (game state): every country.
callback!(reload_mio_policies, 69);

// A site whose fifth argument is on the caller's stack: the detour takes it
// at the same position and passes it on, so the original reads its own.
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
            // Ordinary manpower operations do not write exile resources.
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
// Project reward option (project, option, reward, option token,
// context flag).
callback5!(project_reward_option, PROJECT_REWARD_OPTION);
// The pool database resync (definitions, owner tag, scope, counter,
// header).
callback5!(project_database_resync, PROJECT_DATABASE_RESYNC);

/// The console's intel network growth command: counts an intel network
/// write ([`Domain::grow_intel_network`]).
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

// A province weather writer outside the native window: counted on entry and
// on return ([`Domain::weather_write`]); its arguments name no country.
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
// Weather reinitialize (manager, seed).
weather_callback!(weather_reinitialize, sites::WEATHER_WRITERS.start);
// Weather reset (out, arguments).
weather_callback!(weather_reset, sites::WEATHER_WRITERS.start + 1);
// Weather add snow (out, arguments).
weather_callback!(weather_add_snow, sites::WEATHER_WRITERS.start + 2);
// The console mud command(out).
weather_callback!(weather_set_mud, sites::WEATHER_WRITERS.start + 3);
// The console region-weather command(out, arguments).
weather_callback!(weather_set_region, sites::WEATHER_WRITERS.start + 4);
// The manager's load reader (manager, reader).
weather_callback!(weather_read, sites::WEATHER_WRITERS.start + 5);
// The post-load initial passes (manager).
weather_callback!(weather_initial_passes, sites::WEATHER_WRITERS.start + 6);
// Weather reload (out, arguments).
weather_callback!(weather_reload, sites::WEATHER_WRITERS.start + 7);
// The console command(out, arguments) that runs hourly game state.
weather_callback!(weather_console_hours, sites::WEATHER_WRITERS.start + 8);
// Province modifier removal (manager, province, modifier, rebuild).
weather_callback!(
    weather_remove_province_modifier,
    sites::WEATHER_WRITERS.start + 10
);
// The daily expiry's CALL of the rebuild (record, manager, force); the detour calls the
// rebuild, which returns to the site's next instruction through the relay.
weather_callback!(weather_expiry_rebuild, sites::WEATHER_EXPIRY_REBUILD);

/// Province modifier addition (manager, province, modifier, days, rebuild), whose
/// fifth argument is on the caller's stack: counted as [`weather_callback`] counts,
/// passing the fifth argument on.
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

/// `technology research progress adder(technology, amount, source,
/// notify)`: marks only an effect's progress ([`Domain::research_progress`]).
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

/// `sync research slots(status)`: notifies only a
/// changed slot roster ([`Domain::sync`]).
unsafe extern "system" fn sync_research_slots(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
    // SAFETY: guarded reads of live memory on the calling thread.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[SYNC_RESEARCH_SLOTS].load(Ordering::Acquire)) };
    DOMAIN.sync(
        first,
        &|status: usize| sites::slot_roster(&memory, status),
        || timing.original(|| unsafe { original(first, second, third, fourth) }),
    )
}

/// Faction sharing base setter (group,
/// count)`: notifies only a changed base ([`Domain::rebase`]).
unsafe extern "system" fn faction_sharing_base(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
    // SAFETY: guarded reads of live memory on the calling thread.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
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
