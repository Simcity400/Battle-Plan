//! Runtime writers of per-country fields recorded by CountryStabilityInputs
//! and CountryStabilityLines (`country_stability`), ResearchInputs and
//! ResearchAppliedBonuses (`research_inputs`), ResearchModifiers
//! (`research_modifiers`), ResearchTechnologies (`ctechnology`),
//! Focus, Projects, CountryAppearance, CountryCounterInputs and CountryVisibility.
//! Observing them selects countries to reread; recording never calls them.
//!
//! Stability formula operands (`country_stability::inputs`):
//! - Bases `+0x10D0`/`+0x10D8` are written by stability and war-support addition
//!   (effects, guarantee breaks, non-aggression war penalties, nukes and console),
//!   their setters, and weekly country updates with their sole helper. The game
//!   runs the weekly update for every country at each week boundary; it alone
//!   also writes the five Q15 penalties `+0x14C0..0x14E0`. Other writers are
//!   construction, forwarded country-appearance reset and the save member reader.
//! - Ruling party popularity (`[[+0xF90]+0xD0]+0x88`) drifts in the daily
//!   politics update for every country and is refreshed by the daily slice.
//!   Any-hour writers belong to [`crate::political_status_mutations`]: five
//!   function sites and two clamped-setter calls, all forwarded.
//! - Only the daily country subsystem update writes coastal protection
//!   (`+0x1600`), once a day at that country's slot hour; the daily slice
//!   refreshes this drift.
//! - Interception is history `+0xFC8` column 1. The air-statistic adder receives
//!   country, column and value; column 1 comes only from interception bookkeeping
//!   for hourly air missions, coordinated strikes and naval air combat. Daily
//!   ring advancement runs for every country on worker threads, is patched by
//!   the history family, and is drift refreshed by the daily slice.
//! - War flags read owned diplomacy relations (`+0x20`), war `+0x158` and its
//!   second country's exile host `+0x1A8`. Relation replacement alone writes a
//!   status's war list, once per endpoint for declaring/joining war, civil war
//!   and war ending. Instigator and defender setters write `+0x158` after
//!   linking; relation creation writes a new relation before replacement.
//!   Becoming exiled writes the host; exile-link clearing clears the country's
//!   and every hosted exile's. Return from exile is daily only below legitimacy
//!   one. Diplomacy reset is patched by the relation family and forwarded.
//! - World tension, defines, active language and tracked keys are per-read
//!   globals (`country_stability::Context`).
//!
//! Stability tree and sources (`walk_country`):
//! - Only country modifier recalculation rebuilds accumulator `+0x5B8` and
//!   children: 77 direct calls, no virtual reference and no other runtime
//!   writer among accumulator clear, filtered/scaled addition and merging.
//!   Appearance reset clears it and the save member reader reads it. Every
//!   dynamic-modifier addition, removal, expiry and enabled-flag/value
//!   recalculation reaches it in the same call. Deferred refresh (`+0x10E0`
//!   bit 1), set by building-level changes, research completion and wartime-ally
//!   rebuilding, recalculates in the next pre-hourly pending-refresh pass and
//!   notifies then.
//! - Recalculation does not follow the ruling party (forwarded political-status
//!   sites), leader vector (all forwarded [`crate::party_leader_mutations`]
//!   writers), first leader modifier/display (modifier rebuilding, two role
//!   creation/insertion sites, destruction and ideology setting; forwarded
//!   character-role sites), active ideas (one forwarded setter), advisor
//!   modifiers (forwarded rebuilding), or appointed advisors (`+0x58`).
//!   Appointment always recalculates; unappointment from retirement, unit-leader
//!   capture, the deferred hourly queue, advisor-status updates and another
//!   advisor helper does not. Character names and portraits are not read.
//!
//! Research (`research_inputs`, `research_modifiers`):
//! - Slots (status `+0xA0`, slot `+0x08..+0x40`) are written by technology
//!   assignment (two research commands, console and completion) and slot
//!   synchronization. The daily tick synchronizes every country, changing the
//!   roster only when country `+0x1348` differs from status `+0xAC`. Banked idle
//!   days `+0x20` drift daily.
//! - Technology fields (`+0x170..+0x1F0`) are written by the level setter
//!   (completion, effects, unlocks, stolen bonuses), research reset (direct
//!   level write), slot completion, research completion (classifier `+0x170`
//!   recursively through propagation), limited-bonus claim/release, design-team
//!   and XP-boost assignment, transfer/inheritance rebuilding (technology
//!   inheritance, civil war and dynamic countries), database-reload rebuilding,
//!   the research console command (through progress, level, completion,
//!   assignment and propagation), and fresh initialization after dynamic-country
//!   reuse and load. Propagation is reached by these sites, load and its observer.
//!   Daily slot progress through the progress adder writes `+0x198` as drift:
//!   only the research panel/tooltips, AI research weight and the random-stolen
//!   bonus selection read it, with no trigger. The adder's site marks progress
//!   added to any technology by technology-setting and random-stolen-bonus
//!   effects (source 2, [`EFFECT_PROGRESS_SOURCE`]). Assignment and console call
//!   it inside hooked writes; the technology-tree GUI handler (source 0) calls
//!   the hooked level setter and research completion later in the same call.
//! - Technology rows (`ctechnology::Row`) read classifier `+0x170`, level
//!   `+0x174`, ahead reduction `+0x1C0`, applied bonus ids `+0x1D0`, researched
//!   array `+0x40` and roster `+0x88`. Only construction, propagation, the
//!   rebuild classification sweep, post-read initialization under load/transfer,
//!   fresh initialization and research completion write classifiers; the
//!   observer propagates only on their classifier changes. Claim/release and
//!   assignment write ahead reduction. Transfer, reset and level setting reach
//!   researched-array insertion/removal. Fresh initialization and rebuilding
//!   write the roster. These sites mark technology rows only for their own
//!   country.
//! - Limited-bonus and doctrine-cost-reduction additions append bonus rosters.
//!   Recorded fields never change after creation. Only appearance reset and
//!   destruction clear them.
//! - Modifier operands include the country accumulator, technology category sum
//!   `+0x190` and script sources `+0x128` (two helpers reached only by country
//!   modifier recalculation); idea contributions `+0x110`/`+0x190` (two helpers
//!   reached only by the forwarded idea setter); licences (registration and
//!   variant extension from variant-creation commands, through a shared insert,
//!   plus removal from two cancellation paths, war cleanup and destruction);
//!   the program scientist-category map (project start, completion, cancel,
//!   scientist assignment and a virtual program-status callback); and scientist
//!   roles (forwarded [`crate::character_roster_mutations`] sites).
//! - Sharing reads the status roster `+0x118` (addition/removal, also reached
//!   from faction addition/removal), virtual group membership addition/removal,
//!   group bonus setting, faction sharing base (supportive scientist counts
//!   from daily refresh and attachment), and faction member order. Faction
//!   leadership moves the leader to the front; faction-mode groups follow that
//!   order. Every sharing country reads a member's levels/classifiers, including
//!   predecessor classifiers in sharing rows. These writers, assignment (which
//!   propagates both old and new technologies) and fresh initialization also
//!   name partners on the writer's groups and faction. Writers naming a
//!   technology (level, reset, completion, assignment) select only partners whose
//!   researched instance here is written or whose predecessor classifiers its
//!   propagation can rewrite ([`sharing_affected`]); rebuilding and fresh
//!   initialization select every member ([sharing]).
//!
//! National focus (`cnational_focus_progress`): the writers below mark
//! the progress object's country (`+0x8`).
//!
//! CountryAppearance (`CountryAppearanceRow`) follows cosmetic tags, colour
//! copies, 3D/2D graphical cultures, localized names and subject rebuilding
//! below, plus forwarded country/diplomacy resets, ruling-party, party-leader
//! and character-role writers. CountryCounterInputs (`CountryCounterInputRow`)
//! follows modifier recalculation, war relations, division and unit-leader
//! rosters, neighbours and industrial-organisation policy below, plus forwarded
//! country/diplomacy resets, wartime-ally rebuilding, relation-status resets and
//! replacements, state ownership/control/reset, core insertion/erasure, and
//! country-character pool construction/destruction. State owner/controller
//! changes name old and new countries; core changes name the core's country;
//! subject rebuilding names its country and overlord before and after the call.
//!
//! CountryVisibility retains only raw presence/value of the combat-intel
//! accumulator entry. Accumulator clearing, filtered/scaled addition and merging
//! reach country `+0x5B8` only through modifier recalculation and appearance
//! reset. Helpers receiving that accumulator run inside recalculation. Load
//! readers and construction cannot certify a held epoch. Definition and
//! eligibility metadata remain per-hour reads.
//!
//! Every site runs on the game thread, takes at most four integer or pointer
//! arguments and opens with whole relocatable instructions covering the five
//! patched bytes.

use crate::{
    Memory, ccharacter, ccountry, ccountry_characters, cdiplomacy_status,
    cequipment_production_license, cfaction, cgame_state, cnational_focus_progress,
    cpdx_array::Array, cpolitical_party, cpolitical_status, crelation, crelation_status,
    cresearch_slot, cstate, ctechnology, ctechnology_sharing_group, ctechnology_status,
    ctechnology_template, nproject_cprogram_status, nproject_cproject, read,
};

pub use crate::advisor_mutations::MutationSite;

/// Which table families a write reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tables {
    /// CountryStabilityInputs and CountryStabilityLines.
    pub stability: bool,
    /// ResearchInputs, ResearchAppliedBonuses and ResearchModifiers.
    pub research: bool,
    /// ResearchTechnologies (`ctechnology::Row`), whose rows only the
    /// writer's own country reads ([`Receiver::own`]).
    pub technologies: bool,
    /// Focus and CompletedFocuses (`cnational_focus_progress`).
    pub focus: bool,
    /// IntelNetworks (`ccountry_intel_network`), read whole again
    /// at the next hour.
    pub intel_networks: bool,
    /// Projects and ProjectHistory (`nproject_cproject`).
    pub projects: bool,
    /// CountryAppearance (the appearance fields `ccountry` reads).
    pub appearance: bool,
    /// CountryCounterInputs (the counter walk `ccountry` runs and
    /// the relation operands of `crelation_status`).
    pub counters: bool,
    /// CountryVisibility: the raw INTEL_FROM_COMBAT_FACTOR accumulator entry.
    pub visibility: bool,
    /// WeatherProvinces outside the hour's native window
    /// (`cweather_manager`): no country set; its writers move a
    /// count the window-following read compares.
    pub weather: bool,
}

impl Tables {
    pub const STABILITY: Self = Self {
        stability: true,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    pub const RESEARCH: Self = Self {
        stability: false,
        research: true,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    pub const BOTH: Self = Self {
        stability: true,
        research: true,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    /// A technology instance writer: the research rows and the technology
    /// rows.
    pub const TECHNOLOGY: Self = Self {
        stability: false,
        research: true,
        technologies: true,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    /// A national focus writer.
    pub const FOCUS: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: true,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    /// The console's intel network writer.
    pub const INTEL_NETWORKS: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: true,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    /// A special project writer.
    pub const PROJECTS: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: true,
        appearance: false,
        counters: false,
        visibility: false,
        weather: false,
    };
    /// A country appearance writer.
    pub const APPEARANCE: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: true,
        counters: false,
        visibility: false,
        weather: false,
    };
    /// A country counter input writer.
    pub const COUNTERS: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: true,
        visibility: false,
        weather: false,
    };
    /// A country visibility accumulator writer: modifier recalculation
    /// and the forwarded appearance reset.
    pub const VISIBILITY: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: true,
        weather: false,
    };
    /// A writer of province weather records outside the native window.
    pub const WEATHER: Self = Self {
        stability: false,
        research: false,
        technologies: false,
        focus: false,
        intel_networks: false,
        projects: false,
        appearance: false,
        counters: false,
        visibility: false,
        weather: true,
    };
    pub const ALL: Self = Self {
        stability: true,
        research: true,
        technologies: true,
        focus: true,
        intel_networks: false,
        projects: true,
        appearance: true,
        counters: true,
        visibility: true,
        weather: false,
    };

    /// Every table either reaches.
    pub const fn with(self, other: Self) -> Self {
        Self {
            stability: self.stability || other.stability,
            research: self.research || other.research,
            technologies: self.technologies || other.technologies,
            focus: self.focus || other.focus,
            intel_networks: self.intel_networks || other.intel_networks,
            projects: self.projects || other.projects,
            appearance: self.appearance || other.appearance,
            counters: self.counters || other.counters,
            visibility: self.visibility || other.visibility,
            weather: self.weather || other.weather,
        }
    }
}

/// How a site's first two arguments (RCX, RDX) name the countries whose rows
/// it writes ([countries]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is the country.
    Country,
    /// RCX is the country and EDX the air statistic column; only column
    /// 1 (interception) is recorded.
    Interception,
    /// RCX is a relation status; its owner is its diplomacy `+0x1F0`'s
    /// country `+0x288`.
    RelationStatus,
    /// RCX is a war relation; both endpoints `+0x10`/`+0x14` (link ids).
    WarRelation,
    /// RCX is a country character pool; its owner `+0x8`.
    CountryCharacters,
    /// RCX is a political status; its owner `+0xC8`.
    PoliticalStatus,
    /// RCX is a political party; its country link id `+0x20`.
    Party,
    /// RCX is a character; its country link id `+0x68`.
    Character,
    /// RCX is a scientist; its character `+0x8`.
    Scientist,
    /// RCX is a research slot; its status `+0x10`.
    ResearchSlot,
    /// A research slot whose assignment reclassifies its country's technologies
    /// through two propagation calls. Sharing countries read a member's
    /// classifiers (`research_modifiers::sharing`), so this names its status's
    /// country and every research partner, as [`Self::SharedTechnologyStatus`].
    SharedResearchSlot,
    /// RCX is a technology status; its country `+0x130`.
    TechnologyStatus,
    /// RCX is a technology; its status `+0x158`.
    Technology,
    /// A technology whose level or classifier other countries read:
    /// its status's country and every country sharing research with it.
    SharedTechnology,
    /// A technology status writing levels or classifiers, as
    /// [`Self::SharedTechnology`].
    SharedTechnologyStatus,
    /// RCX is the technology status and RDX the technology whose research is
    /// completed: its country and the partners whose rows that technology
    /// can change ([`sharing_affected`]).
    CompletedTechnology,
    /// RCX is a program status; its owner link id `+0x10`.
    ProgramStatus,
    /// RCX is a focus progress; its country `+0x8`.
    FocusProgress,
    /// RCX is an project; its owner link id `+0x1C`.
    Project,
    /// RCX is a faction sharing group (faction `+0x860`; construction stores
    /// its faction at `+0x48`): the faction's members and raw ids on the group's
    /// member array (`+0x18`), whose sharing rosters hold it.
    FactionSharing,
    /// RCX is a equipment production licence; its licensee link id `+0x10`,
    /// whose production status holds the licensed variants.
    License,
    /// RCX is a diplomacy status; its country `+0x288`.
    DiplomacyStatus,
    /// RCX is a diplomacy status whose subjects `+0x170` and overlord
    /// `+0x188` the call rebuilds: its country and the overlord the raw link
    /// names while positive, as the rebuild requires,
    /// named again on return ([`Self::renames`]) for the new overlord.
    Subjects,
    /// State owner setting receives the state in RCX and new owner link in
    /// EDX, with two further arguments: old owner `+0xC8` and new owner, each
    /// while positive. The setter resolves the old link only when positive.
    StateOwner,
    /// State controller setting receives the state in RCX and new controller
    /// link in EDX, with two further arguments: old controller `+0xCC` and
    /// new controller, each while positive.
    StateController,
    /// Core addition/removal receives the state and a raw country link pointer
    /// in RDX, doing nothing unless the pointed-to link is positive. This names
    /// that country while positive.
    TagPointer,
    /// The site writes countries it does not name.
    Every,
}

impl Receiver {
    /// The writer's own country only: a technology's level and classifier
    /// are read by the countries sharing research for their modifiers, but a
    /// technology row is its own country's.
    pub const fn own(self) -> Self {
        match self {
            Self::SharedTechnology => Self::Technology,
            Self::SharedTechnologyStatus => Self::TechnologyStatus,
            Self::SharedResearchSlot => Self::ResearchSlot,
            Self::CompletedTechnology => Self::TechnologyStatus,
            other => other,
        }
    }

    /// The call can name a country on return that it did not name on entry
    /// (a rebuilt overlord), so it is named again on return even when it
    /// named one country on entry.
    pub const fn renames(self) -> bool {
        matches!(self, Self::Subjects)
    }
}

/// EDX of air-statistic addition for interception: the only
/// column the stability inputs read (`country_stability::interception`).
pub const INTERCEPTION_COLUMN: u32 = 1;
/// Scientist `+0x8`: its character, copied by copy construction and used to
/// retain scientists in the program category-map rebuild.
pub const SCIENTIST_CHARACTER: usize = 0x08;
/// Technology status `+0x118`: the sharing-group roster, changed by group
/// addition/removal and visited by the sharing member walk.
pub const SHARING_ROSTER: usize = 0x118;
/// Slot synchronization rewrites the slot roster header
/// (technology status `+0xA0`, capacity `+0xA8`, count `+0xAC`) only when it
/// changes the roster.
pub const SLOT_ROSTER: usize = ctechnology_status::RESEARCH_SLOTS_DATA;
/// Raw member ids the sharing member walk reads from a roster entry
/// (`+0x18`, count `+0x24`).
const GROUP_MEMBERS: usize = ctechnology_sharing_group::MEMBERS;
/// The faction sharing roster entry's vtable `research_modifiers` accepts
/// beside the group's.
const FACTION_SHARING_VTABLE_RVA: usize = 0x02a3_7e70;

const UNAPPOINT_ADVISOR_RVA: usize = 0x10d_64e0;
const SET_LEVEL_RVA: usize = 0xec_e360;
const RESOLVE_GRAPHICAL_CULTURE_RVA: usize = 0x70_5600;
const REBUILD_LOCALIZED_NAME_RVA: usize = 0x70_9950;

/// Each hooked site, how its arguments name its countries, and the tables
/// its writes reach.
pub const MUTATIONS: [(MutationSite, Receiver, Tables); 81] = [
    // Country modifier recalculation (country): stores RCX at RSP+8.
    (
        MutationSite {
            name: "recalculate_modifiers",
            rva: 0x6c_e2a0,
            sentinel: [
                0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
                0x56, 0x41,
            ],
        },
        Receiver::Country,
        // Pride of the fleet `+0x250` (its setter recalculates) and counter-row
        // attack modifiers; the visibility entry used by unit intel is rebuilt too.
        Tables::BOTH.with(Tables::COUNTERS).with(Tables::VISIBILITY),
    ),
    // Stability addition (country, delta): loads the base at RCX+0x10D0 into R8.
    (
        MutationSite {
            name: "add_stability",
            rva: ccountry::ADD_STABILITY_RVA,
            sentinel: [
                0x4c, 0x8b, 0x81, 0xd0, 0x10, 0x00, 0x00, 0x4c, 0x03, 0xc2, 0x4c, 0x89, 0x81, 0xd0,
                0x10, 0x00,
            ],
        },
        Receiver::Country,
        Tables::STABILITY,
    ),
    // War-support addition (country, delta): loads the base at RCX+0x10D8 into R8.
    (
        MutationSite {
            name: "add_war_support",
            rva: ccountry::ADD_WAR_SUPPORT_RVA,
            sentinel: [
                0x4c, 0x8b, 0x81, 0xd8, 0x10, 0x00, 0x00, 0x4c, 0x03, 0xc2, 0x4c, 0x89, 0x81, 0xd8,
                0x10, 0x00,
            ],
        },
        Receiver::Country,
        Tables::STABILITY,
    ),
    // Stability setting (country, value): stores RDX at RCX+0x10D0. The
    // instruction-pointer-relative load starts at +7, after the relocated store.
    (
        MutationSite {
            name: "set_stability",
            rva: 0x70_5c70,
            sentinel: [
                0x48, 0x89, 0x91, 0xd0, 0x10, 0x00, 0x00, 0x4c, 0x8b, 0x05, 0x6a, 0x4b, 0xc1, 0x02,
                0x48, 0x8b,
            ],
        },
        Receiver::Country,
        Tables::STABILITY,
    ),
    // War-support setting (country, value): stores RDX at RCX+0x10D8.
    (
        MutationSite {
            name: "set_war_support",
            rva: 0x70_5cc0,
            sentinel: [
                0x48, 0x89, 0x91, 0xd8, 0x10, 0x00, 0x00, 0x4c, 0x8b, 0x05, 0x4a, 0x4c, 0xc1, 0x02,
                0x48, 0x8b,
            ],
        },
        Receiver::Country,
        Tables::STABILITY,
    ),
    // Weekly country update (country): stores RBX at RSP+0x18.
    (
        MutationSite {
            name: "weekly_update",
            rva: ccountry::WEEKLY_UPDATE_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x55, 0x57, 0x41, 0x54,
                0x41, 0x55,
            ],
        },
        Receiver::Country,
        Tables::STABILITY,
    ),
    // Air-statistic addition (country, column, value), also tail-called by
    // building-bombing bookkeeping: stores RBX at RSP+8.
    (
        MutationSite {
            name: "add_air_statistic",
            rva: 0x6c_4e10,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x89, 0xc8,
                0x0f, 0x00,
            ],
        },
        Receiver::Interception,
        Tables::STABILITY,
    ),
    // Relation replacement: RCX is the status, RDX the new relation, R8 the
    // typed slot to replace. The prologue stores RBX at RSP+8.
    (
        MutationSite {
            name: "replace_relation",
            rva: 0xd0_2df0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x49,
            ],
        },
        Receiver::RelationStatus,
        // The war pointer `+0x2E8` of the owner's slot-zero status, a counter
        // operand (zero_war).
        Tables::STABILITY.with(Tables::COUNTERS),
    ),
    // War instigator setting (war, tag, _): stores RBX at RSP+8.
    (
        MutationSite {
            name: "set_instigator",
            rva: 0xd1_87e0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x8b,
            ],
        },
        Receiver::WarRelation,
        Tables::STABILITY,
    ),
    // War defender setting (war, tag, _): shares the instigator prologue
    // bytes, with its own verified site.
    (
        MutationSite {
            name: "set_defender",
            rva: 0xd1_8690,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x8b,
            ],
        },
        Receiver::WarRelation,
        Tables::STABILITY,
    ),
    // Exile entry (exile diplomacy, host, _): other countries read the host
    // for war flags, so every country is marked. Stores RBX at RSP+8.
    (
        MutationSite {
            name: "become_exiled",
            rva: 0xd2_0580,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24,
                0x18, 0x41,
            ],
        },
        Receiver::Every,
        Tables::STABILITY,
    ),
    // Exile-link clearing (diplomacy): clears its own and every hosted
    // exile's host.
    (
        MutationSite {
            name: "clear_exile_links",
            rva: 0xd2_f270,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::Every,
        Tables::STABILITY,
    ),
    // Return from exile (diplomacy): stores RCX at RSP+8.
    (
        MutationSite {
            name: "return_from_exile",
            rva: 0xd2_f320,
            sentinel: [
                0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
                0x56, 0x41,
            ],
        },
        Receiver::Every,
        Tables::STABILITY,
    ),
    // Advisor unappointment (characters, advisor, flag): stores RBX at RSP+0x18.
    (
        MutationSite {
            name: "unappoint_advisor",
            rva: UNAPPOINT_ADVISOR_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::CountryCharacters,
        Tables::STABILITY,
    ),
    // Slot technology assignment (slot, technology, _): saves RBX through RDI.
    (
        MutationSite {
            name: "set_technology",
            rva: 0xec_e4f0,
            sentinel: [
                0x40, 0x53, 0x55, 0x56, 0x57, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec,
                0x50, 0x48,
            ],
        },
        Receiver::SharedResearchSlot,
        Tables::TECHNOLOGY,
    ),
    // Slot synchronization (status) runs daily for every country. The collector
    // notifies only when the slot roster header changed ([`SLOT_ROSTER`]).
    // Stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "sync_research_slots",
            rva: 0xec_f950,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x54,
                0x41, 0x56,
            ],
        },
        Receiver::TechnologyStatus,
        Tables::TECHNOLOGY,
    ),
    // Technology level setting (technology, level): saves RBX and reserves
    // 0x30 stack bytes.
    (
        MutationSite {
            name: "set_level",
            rva: SET_LEVEL_RVA,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0x81, 0x60, 0x01, 0x00, 0x00, 0x48,
                0x8b, 0xd9,
            ],
        },
        Receiver::SharedTechnology,
        Tables::TECHNOLOGY,
    ),
    // Limited-use bonus claim (technology).
    (
        MutationSite {
            name: "claim_limited_use_bonuses",
            rva: 0xec_43a0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x55,
            ],
        },
        Receiver::Technology,
        Tables::TECHNOLOGY,
    ),
    // Limited-use bonus release (technology).
    (
        MutationSite {
            name: "release_limited_use_bonuses",
            rva: 0xec_4890,
            sentinel: [
                0x40, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x40, 0x4c, 0x8b, 0xb1, 0xd0,
                0x01, 0x00,
            ],
        },
        Receiver::Technology,
        Tables::TECHNOLOGY,
    ),
    // Research slot completion (status, slot, _).
    (
        MutationSite {
            name: "complete_research_slot",
            rva: 0xec_c0e0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x56,
                0x48, 0x81,
            ],
        },
        Receiver::TechnologyStatus,
        Tables::TECHNOLOGY,
    ),
    // Research reset (technology): writes the level.
    (
        MutationSite {
            name: "reset_research",
            rva: 0xec_dba0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x56, 0x48, 0x83, 0xec,
                0x20, 0x4c,
            ],
        },
        Receiver::SharedTechnology,
        Tables::TECHNOLOGY,
    ),
    // Design-team assignment (technology, organisation).
    (
        MutationSite {
            name: "set_design_team",
            rva: 0xec_e080,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x48, 0x83, 0xec, 0x30, 0x83, 0xb9,
                0xec, 0x01,
            ],
        },
        Receiver::Technology,
        Tables::RESEARCH,
    ),
    // XP-boost setting (technology, flag): stores RBX at RSP+8; the
    // instruction-pointer-relative comparison starts at +0xA.
    (
        MutationSite {
            name: "set_boosted_by_xp",
            rva: 0xec_fd40,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0xa1, 0x8e,
                0x6f, 0x02,
            ],
        },
        Receiver::Technology,
        Tables::RESEARCH,
    ),
    // Research completion (status, technology, _).
    (
        MutationSite {
            name: "research_completed",
            rva: ctechnology_status::RESEARCH_COMPLETED_RVA,
            sentinel: ctechnology_status::RESEARCH_COMPLETED_SENTINEL,
        },
        Receiver::CompletedTechnology,
        Tables::TECHNOLOGY,
    ),
    // Technology transfer/inheritance rebuilding (destination, source).
    (
        MutationSite {
            name: "rebuild_or_transfer_b",
            rva: ctechnology_status::REBUILD_OR_TRANSFER_B_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x57, 0x48,
                0x83, 0xec,
            ],
        },
        Receiver::SharedTechnologyStatus,
        Tables::TECHNOLOGY,
    ),
    // Technology status rebuilding (status) from the reload console command:
    // every country.
    (
        MutationSite {
            name: "rebuild_or_transfer_a",
            rva: ctechnology_status::REBUILD_OR_TRANSFER_A_RVA,
            sentinel: [
                0x40, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48,
                0x83, 0xec,
            ],
        },
        Receiver::Every,
        Tables::TECHNOLOGY,
    ),
    // Limited-use bonus addition (status).
    (
        MutationSite {
            name: "add_limited_use_bonus",
            rva: 0xec_0360,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf9, 0xb9,
                0x80, 0x00,
            ],
        },
        Receiver::TechnologyStatus,
        Tables::RESEARCH,
    ),
    // Doctrine cost-reduction append (status).
    (
        MutationSite {
            name: "add_cost_reduction",
            rva: 0xec_03d0,
            sentinel: [
                0x40, 0x55, 0x41, 0x56, 0x48, 0x83, 0xec, 0x28, 0x4c, 0x8b, 0xf1, 0xb9, 0x70, 0x00,
                0x00, 0x00,
            ],
        },
        Receiver::TechnologyStatus,
        Tables::RESEARCH,
    ),
    // Sharing roster addition (status, group): members read the new member's
    // levels, so every country is marked.
    (
        MutationSite {
            name: "sharing_roster_add",
            rva: 0xec_0890,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x48, 0x83, 0xec, 0x20, 0x4c, 0x63, 0x89, 0x24,
                0x01, 0x00,
            ],
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    // Sharing roster removal (status, group).
    (
        MutationSite {
            name: "sharing_roster_remove",
            rva: 0xec_dd50,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x4c, 0x8b, 0xc9, 0x33, 0xc0, 0x8b, 0x89, 0x24, 0x01,
                0x00, 0x00,
            ],
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    // Virtual sharing-group member addition (group, tag).
    (
        MutationSite {
            name: "sharing_add_member",
            rva: 0xd6_d770,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x41, 0x54, 0x41, 0x55, 0x41, 0x57, 0x48,
                0x83, 0xec,
            ],
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    // Virtual sharing-group member removal (group, tag).
    (
        MutationSite {
            name: "sharing_remove_member",
            rva: 0xd6_f840,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x41, 0x56, 0x41,
                0x57, 0x48,
            ],
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    // Sharing bonus setting (group, _, bonus), from the bonus-modification effect.
    (
        MutationSite {
            name: "sharing_bonus",
            rva: 0xd6_f390,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x49, 0x8b, 0xd8, 0x48,
                0x8b, 0xf9,
            ],
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    // Licence registration (production status): the status names no country.
    (
        MutationSite {
            name: "register_license",
            rva: crate::clicensed_production_status::REGISTER_LICENSE_RVA,
            sentinel: [
                0x40, 0x55, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x30,
                0x48, 0x89,
            ],
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    // Scientist category-map rebuilding (program status): stores RCX at RSP+8.
    (
        MutationSite {
            name: "scientist_categories",
            rva: 0xe6_1570,
            sentinel: [
                0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
                0x56, 0x41,
            ],
        },
        Receiver::ProgramStatus,
        Tables::RESEARCH,
    ),
    // Research console command (out, arguments): copies the stack pointer to RAX.
    (
        MutationSite {
            name: "research_console",
            rva: 0x27_c930,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x48, 0x08, 0x55, 0x56, 0x57,
                0x41, 0x54,
            ],
        },
        Receiver::Every,
        Tables::TECHNOLOGY,
    ),
    // Fresh technology initialization (status) rebuilds and classifies the
    // roster after load, during runtime-subsystem initialization for dynamic
    // country reuse (civil war, country creation and peace-conference
    // participants), after forwarded appearance reset. Stores RBX at RSP+0x18.
    (
        MutationSite {
            name: "fresh_init",
            rva: ctechnology_status::FRESH_INIT_RVA,
            sentinel: ctechnology_status::FRESH_INIT_SENTINEL,
        },
        Receiver::SharedTechnologyStatus,
        Tables::TECHNOLOGY,
    ),
    // National focus writers (`cnational_focus_progress`) cover the
    // country tree `+0x1370`, current/continuous focus `+0x10`/`+0x18`, progress
    // `+0x38`, paused flag `+0xB0` and completed vector `+0x40..+0x4C`. The politics
    // family patches and forwards selection and cancellation. Daily focus update
    // is unhooked: the game-state daily pass runs it for every country owning a
    // state inside the hour entered at 23; focus reads every country at the next
    // hour. Selection with focus auto-completion is its other caller. Member
    // loading, construction and destruction run only in a load.
    //
    // Progress reset (progress) runs from appearance reset, tree replacement and
    // dynamic-country reuse. It zeroes R8D and sets the paused byte at RCX+0xB0.
    (
        MutationSite {
            name: "focus_reset",
            rva: 0x2c_eed0,
            sentinel: [
                0x45, 0x33, 0xc0, 0xc6, 0x81, 0xb0, 0x00, 0x00, 0x00, 0x01, 0x4c, 0x89, 0x41, 0x10,
                0x4c, 0x89,
            ],
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    // Virtual focus database clearing (database), on reload: every country's
    // tree and selections. Saves RBX and RDI.
    (
        MutationSite {
            name: "focus_database_clear",
            rva: 0x2c_ec50,
            sentinel: [
                0x40, 0x53, 0x57, 0x48, 0x83, 0xec, 0x38, 0x48, 0x89, 0x74, 0x24, 0x60, 0x48, 0x8b,
                0xf9, 0x4c,
            ],
        },
        Receiver::Every,
        Tables::FOCUS,
    ),
    // Continuous-focus selection/clearing (progress, focus): commands, selection,
    // tree restoration and daily invalidation. Stores RBX at RSP+0x18.
    (
        MutationSite {
            name: "continuous_focus",
            rva: 0x2d_a870,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xd9, 0x48,
                0x8b, 0xfa,
            ],
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    // Focus completion (target progress, focus, origin, _): daily completion,
    // completion effects, joint focuses and auto-completion. Stores R9 at RSP+0x20.
    (
        MutationSite {
            name: "focus_completed",
            rva: cnational_focus_progress::COMPLETED_RVA,
            sentinel: cnational_focus_progress::COMPLETED_SENTINEL,
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    // Focus-state copying (destination, source): civil war and dynamic/peace-created
    // countries. Stores RBX at RSP+8.
    (
        MutationSite {
            name: "focus_copy",
            rva: 0x2c_f7b0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x48,
            ],
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    // Completed-focus removal (progress, focus): focus-uncompletion effects.
    // Stores RBX at RSP+0x18.
    (
        MutationSite {
            name: "focus_remove_completed",
            rva: 0x2d_a5a0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24,
                0x10, 0x57,
            ],
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    // Focus-tree assignment (country, tree, keep): tree-loading effects, country
    // reset and dynamic assignment. Copies the stack pointer to RAX.
    (
        MutationSite {
            name: "set_focus_tree",
            rva: 0x70_4e00,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x44, 0x88, 0x40, 0x18, 0x48, 0x89, 0x50, 0x10, 0x55, 0x53, 0x56,
                0x57, 0x41,
            ],
        },
        Receiver::Country,
        Tables::FOCUS,
    ),
    // Completed-focus addition (progress, focus, origin): completion, bypass,
    // unlock, tree effects and restoration. Stores RDX at RSP+0x10.
    (
        MutationSite {
            name: "add_completed_focus",
            rva: 0x2c_d170,
            sentinel: [
                0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x53, 0x41, 0x57, 0x48, 0x8b, 0xec, 0x48, 0x83,
                0xec, 0x60,
            ],
        },
        Receiver::FocusProgress,
        Tables::FOCUS,
    ),
    // Technology reload console command (out, arguments): clears and reparses
    // the template database with postprocessing through its reload helper, then
    // rebuilds every country's status. The whole command covers the templates
    // TechnologyDefinitions records. Stores RBX at RSP+0x18.
    (
        MutationSite {
            name: "reload_technologies",
            rva: 0x27_ad90,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x48, 0x89, 0x4c, 0x24,
                0x08, 0x56,
            ],
        },
        Receiver::Every,
        Tables::TECHNOLOGY,
    ),
    // The console intelligence-network growth handler (out?) is the only writer
    // of IntelNetworks outside the daily step and dynamic-country reuse. Through
    // three helpers it sets each row's local strength in the player's networks.
    // Stores RCX at RSP+8.
    (
        MutationSite {
            name: "grow_intel_network",
            rva: 0x26_0ac0,
            sentinel: [
                0x48, 0x89, 0x4c, 0x24, 0x08, 0x53, 0x55, 0x56, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec,
                0x30, 0x48,
            ],
        },
        Receiver::Every,
        Tables::INTEL_NETWORKS,
    ),
    // Special project writers (`nproject_cproject`) outside the daily
    // pool pass mark the project owner (`+0x1C`). The daily country pass reaches
    // progress advancement through four helpers inside the hour entered at 23;
    // projects reads every country afterwards. Capture loss and forced completion
    // reach cancellation and completion; forwarded appearance reset clears the
    // pool of a dynamically reused country.
    //
    // Project start (project, program): commands, AI, effects and console.
    // Stores RBX at RSP+0x18.
    (
        MutationSite {
            name: "start_project",
            rva: 0xfd_02e0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x40, 0x48,
                0x8b, 0xda,
            ],
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    // Project stopping (project, cancel): stop commands, scientist unassignment
    // and dismantling. Saves RBX and reserves 0x20 stack bytes.
    (
        MutationSite {
            name: "stop_project",
            rva: 0xfd_04f0,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x84, 0xd2, 0x74, 0x0a, 0x48,
                0x83, 0xc4,
            ],
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    // Project cancellation (project): stopping, outer and forced updates, and
    // capture. Stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "cancel_project",
            rva: 0xfc_f450,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x41, 0x56, 0x41,
                0x57, 0x48,
            ],
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    // Project completion (project, scientist, province): progress, ratio,
    // reward and completion effects. Copies the stack pointer to RAX.
    (
        MutationSite {
            name: "complete_project",
            rva: 0xfc_ec50,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x55, 0x41, 0x55, 0x41, 0x56, 0x48, 0x81, 0xec, 0x90, 0x00, 0x00,
                0x00, 0x80,
            ],
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    // Project progress-ratio addition (project, ratio): progress effects, raid
    // reductions, capture bonuses and supportive scientists. Stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "add_project_progress",
            rva: 0xfc_b780,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xf9, 0x48,
                0x8b, 0xda,
            ],
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    // Prototype reward application (project, option, reward, option token,
    // context flag) takes five arguments. It runs from reward commands, effects,
    // console and default rewards. Stores RBX at RSP+0x20.
    (
        MutationSite {
            name: "project_reward_option",
            rva: 0xfc_bb50,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41,
                0x54, 0x41,
            ],
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    // Forced project update (project): dismantling and the debug scheduled
    // program daily pass. Stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "forced_project_update",
            rva: 0xfc_f5e0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x48,
            ],
        },
        Receiver::Project,
        Tables::PROJECTS,
    ),
    // Project database resynchronization (definitions, owner tag, scope, counter,
    // header) takes five arguments. Database reload reaches it through a helper
    // for every country. Stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "project_database_resync",
            rva: 0x146_f470,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24,
                0x18, 0x55,
            ],
        },
        Receiver::Every,
        Tables::PROJECTS,
    ),
    // Licence removal (licence) resolves its licensee through the link at
    // licence+0x10 and removes the licence's variants from production-status
    // licensed variants (`+0x1B0`, count `+0x1BC`), read by technology licence
    // modifiers. Two cancellation paths, war cleanup (creation/joining) and
    // destruction call it. Saves RBP.
    (
        MutationSite {
            name: "license_removal",
            rva: 0x142_7d00,
            sentinel: [
                0x40, 0x55, 0x48, 0x83, 0xec, 0x50, 0x83, 0x79, 0x44, 0x00, 0x48, 0x8b, 0xe9, 0x0f,
                0x84, 0x4d,
            ],
        },
        Receiver::License,
        Tables::RESEARCH,
    ),
    // Licence extension (licence, variant), from variant-creation commands,
    // appends to the licence then tail-calls insertion into the licensee's
    // licensed variants. Stores RDX at RSP+0x10.
    (
        MutationSite {
            name: "license_extension",
            rva: 0x142_5000,
            sentinel: [
                0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48,
                0x8d, 0x54,
            ],
        },
        Receiver::License,
        Tables::RESEARCH,
    ),
    // Faction leadership setting (faction, country) moves the leader to the
    // front of member array `+0x58`; every member's faction-mode sharing group
    // follows that order. Callers are construction, member addition and leader
    // change. Stores RDX at RSP+0x10.
    (
        MutationSite {
            name: "faction_set_leader",
            rva: 0xd7_a150,
            sentinel: [
                0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x41, 0x57, 0x48, 0x83, 0xec, 0x38, 0x48, 0x8b,
                0xea, 0x4c,
            ],
        },
        Receiver::Every,
        Tables::RESEARCH,
    ),
    // CountryAppearance ([`APPEARANCE_SITES`]) reads diplomacy subjects `+0x170`
    // and overlord `+0x188`, country colours `+0x370..+0x3A0` (UI colour `+0x390`),
    // cosmetic tag `+0x1488`, name/adjective/definite name and old-name caches
    // `+0x90/+0xB0/+0xD0`, graphical cultures `+0x1028`/`+0x1050`, and the ruling
    // party's first leader (ideology and character). Other writers are forwarded
    // appearance reset, diplomacy reset, ruling-party and party-leader sites, and
    // character-role writers including ideology setting. Dynamic reuse resets
    // appearance before original-tag assignment (`+0x130C`'s only runtime writer)
    // in the same game-thread step. Modifier recalculation writes none of these
    // fields (its `+0x390` is a static modifier slot); none changes every hour.
    //
    // Cosmetic-tag setting (country, name): tag-set/drop effects and console.
    // Stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "set_cosmetic_tag",
            rva: 0x70_4600,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Country,
        Tables::APPEARANCE,
    ),
    // Country colour copying (destination, source), called only from console
    // colour setting, is a 33-byte leaf without unwind data. Its first two
    // instructions load 16 bytes at RDX+0x20 into XMM0 and store them at RCX+0x370;
    // four and seven bytes relocate whole. They have no instruction-pointer-relative
    // operand or incoming branch, as with the leaf stability setters.
    (
        MutationSite {
            name: "copy_country_colors",
            rva: 0x70_45d0,
            sentinel: [
                0x0f, 0x10, 0x42, 0x20, 0x0f, 0x11, 0x81, 0x70, 0x03, 0x00, 0x00, 0x0f, 0x10, 0x42,
                0x40, 0x0f,
            ],
        },
        Receiver::Country,
        Tables::APPEARANCE,
    ),
    // 3D graphical-culture resolution (country, name) writes the `+0x1028` name
    // and resolved pointer, from the culture-selection caller. Stores RBX at RSP+8.
    (
        MutationSite {
            name: "resolve_graphical_culture",
            rva: RESOLVE_GRAPHICAL_CULTURE_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf9, 0x48,
                0x8b, 0xda,
            ],
        },
        Receiver::Country,
        Tables::APPEARANCE,
    ),
    // 2D graphical-culture resolution (country, name) writes `+0x1050`, from
    // the same culture-selection caller. Prologue bytes match the 3D site but
    // the site is verified independently.
    (
        MutationSite {
            name: "resolve_graphical_culture_2d",
            rva: 0x70_55a0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf9, 0x48,
                0x8b, 0xda,
            ],
        },
        Receiver::Country,
        Tables::APPEARANCE,
    ),
    // Country name rebuilding (country) writes name, adjective and definite
    // name, and alone writes their old-name caches. Fifteen callers cover cosmetic
    // tags, politics/leader-ideology effects, autonomy, subjects and civil war;
    // a thunk also tail-calls it for leader-role and party writers.
    // Stores RBX at RSP+0x18.
    (
        MutationSite {
            name: "rebuild_localized_name",
            rva: REBUILD_LOCALIZED_NAME_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::Country,
        Tables::APPEARANCE,
    ),
    // Subject rebuilding (diplomacy) clears and refills overlord `+0x188` and
    // subjects `+0x170` from relations. Callers are in-game data initialization
    // and two subject-relation callbacks, one virtual. Stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "rebuild_subjects",
            rva: 0xd3_1940,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x55,
            ],
        },
        Receiver::Subjects,
        Tables::APPEARANCE,
    ),
    // CountryCounterInputs ([`COUNTER_SITES`]) also follows modifier recalculation
    // (pride of the fleet, attack modifiers), relation replacement (slot-zero war),
    // forwarded appearance/diplomacy resets and wartime-ally rebuilding (relation
    // `+0x310` co-belligerent matrix bytes and candidate vectors), relation-status
    // reset and fifteen typed replacements (slot-zero subject/access), state owner
    // and controller changes (owned `+0x478` and controlled `+0x460` states of old
    // and new countries), state reset, core changes (core states `+0x4A8`), and
    // character-pool construction/destruction (country `+0xFF0`). Pride-ship names
    // are read every hour.
    //
    // Division addition (country, army, _, _) writes roster `+0x290` (first
    // division), for every new division, load and the fresh-army thunk.
    // Saves RBX/RBP/RDI.
    (
        MutationSite {
            name: "add_army",
            rva: 0x6c_6480,
            sentinel: [
                0x40, 0x53, 0x55, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x50, 0x48, 0x63,
                0x81, 0x9c,
            ],
        },
        Receiver::Country,
        Tables::COUNTERS,
    ),
    // Division removal (country, army) returns the removed value and swap-erases
    // the roster. Callers are two removal helpers and division addition.
    // Stores RBX at RSP+8.
    (
        MutationSite {
            name: "remove_army",
            rva: 0x6f_2e80,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x48,
            ],
        },
        Receiver::Country,
        Tables::COUNTERS,
    ),
    // Unit-leader insertion (characters, leader pointer), from character-status
    // processing, writes army leaders `+0x70` (kinds 0/1) or navy leaders `+0x88`.
    // Saves RBP/RDI/R13.
    (
        MutationSite {
            name: "insert_unit_leader",
            rva: 0x10d_af80,
            sentinel: [
                0x40, 0x55, 0x57, 0x41, 0x55, 0x41, 0x57, 0x48, 0x8b, 0xec, 0x48, 0x83, 0xec, 0x38,
                0x4c, 0x8b,
            ],
        },
        Receiver::CountryCharacters,
        Tables::COUNTERS,
    ),
    // Unit-leader removal (characters, character, flag) returns the removed
    // value. Callers are retirement, capture, removal effects and the deferred
    // queue. Saves RBX/RSI/RDI.
    (
        MutationSite {
            name: "remove_unit_leader",
            rva: 0x10d_90e0,
            sentinel: [
                0x40, 0x53, 0x56, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xda, 0x41, 0x0f, 0xb6,
                0xf0, 0x32,
            ],
        },
        Receiver::CountryCharacters,
        Tables::COUNTERS,
    ),
    // Neighbour rebuilding (country) writes `+0x1088`. Callers are derived-state
    // refresh by flags, province controller changes and another country refresh
    // helper. Saves RBP/RBX.
    (
        MutationSite {
            name: "refresh_neighbors",
            rva: 0x6f_e3c0,
            sentinel: [
                0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48,
                0x8b, 0xec,
            ],
        },
        Receiver::Country,
        Tables::COUNTERS,
    ),
    // Industrial-organisation policy reload (game state) reaches every country's
    // organisation manager (`+0xF68`) through its helper. Only database reload,
    // through its virtual entry and two other callers, reaches it.
    // Stores RSI at RSP+0x10.
    (
        MutationSite {
            name: "reload_mio_policies",
            rva: 0x1e_a360,
            sentinel: [
                0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0xbf, 0x01, 0x00, 0x00,
                0x00, 0x48,
            ],
        },
        Receiver::Every,
        Tables::COUNTERS,
    ),
    // Faction sharing-base setting (group, supportive scientist count) writes
    // the count multiplied by the configured scientist-sharing bonus at `+0x10`,
    // the raw Sharing value for every member. It runs from the daily faction
    // refresh through three helpers, after country program ticks that can prune
    // supportive scientists through two helpers, and from scientist attachment,
    // sharing upgrades and parsing. The detour marks only a changed base.
    // Stores RBX at RSP+0x18.
    (
        MutationSite {
            name: "faction_sharing_base",
            rva: 0x1bd_17f0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x30, 0x48,
                0x63, 0xc2,
            ],
        },
        Receiver::FactionSharing,
        Tables::RESEARCH,
    ),
    // Weather reseeding/reinitialization (manager, seed): weather-randomization
    // effects, reload console commands and reset reinitialize every province
    // record. Saves RBX.
    (
        MutationSite {
            name: "weather_reinitialize",
            rva: 0xf0_a580,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x30, 0x89, 0x91, 0x08, 0x03, 0x00, 0x00, 0x48, 0x8b,
                0xd9, 0x80,
            ],
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    // Weather reset/setup console command (out, arguments): resets and initializes
    // the manager and may reseed. Stores RBX at RSP+0x8.
    (
        MutationSite {
            name: "weather_reset",
            rva: 0x22_dc20,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x56,
                0x48, 0x83,
            ],
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    // Snow console command (out, arguments) directly writes the selected
    // province's snow (`+0x40`). Copies the stack pointer to R11.
    (
        MutationSite {
            name: "weather_add_snow",
            rva: 0x23_c350,
            sentinel: [
                0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x5b, 0x18, 0x49, 0x89, 0x4b, 0x08, 0x55, 0x56, 0x57,
                0x41, 0x56,
            ],
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    // Mud console command (out) forces the selected province's raw mud
    // (`+0x130`). Stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "weather_set_mud",
            rva: 0x28_4ba0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24,
                0x08, 0x57,
            ],
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    // Weather console command (out, arguments) sets the selected province's
    // region flags through their setters and rebuilds its modifiers.
    // Stores RBX at RSP+0x10.
    (
        MutationSite {
            name: "weather_set_region",
            rva: 0x28_7090,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41,
                0x54, 0x41,
            ],
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    // Weather load reading (manager, reader) reads every province/region record
    // and rebuilds region modifiers. Stores RBX at RSP+0x8.
    (
        MutationSite {
            name: "weather_read",
            rva: 0xf0_a5f0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x55,
            ],
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    // Virtual post-load weather initialization (manager) sets initial-mode flag
    // `+0x348` and runs the hourly update, whose initial passes visit every
    // province. Saves RBP.
    (
        MutationSite {
            name: "weather_initial_passes",
            rva: 0xf0_78b0,
            sentinel: [
                0x40, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x30, 0x4c, 0x8b, 0x71, 0x10,
                0x48, 0x8b,
            ],
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    // Weather reload console command (out, arguments) resets and initializes
    // every province/region record before reseeding. Copies the stack pointer to R11.
    (
        MutationSite {
            name: "weather_reload",
            rva: 0x27_b4c0,
            sentinel: [
                0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x5b, 0x18, 0x49, 0x89, 0x4b, 0x08, 0x55, 0x56, 0x57,
                0x41, 0x54,
            ],
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    // The console hourly-step command (out, arguments) calls the hourly game-state
    // pass at three sites, running weather passes outside the hourly tick.
    // Copies the stack pointer to RAX.
    (
        MutationSite {
            name: "weather_console_hours",
            rva: 0x24_37c0,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x50, 0x10, 0x48, 0x89, 0x48,
                0x08, 0x55,
            ],
        },
        Receiver::Every,
        Tables::WEATHER,
    ),
    // Technology progress addition (technology, amount, source, notify) writes
    // `+0x198`. Daily slot progress (source 0) is drift read afresh by a carried
    // slice; assignment (source 1) and console (source 0) run it inside hooked
    // writes. The technology-tree GUI handler (source 0) calls hooked level
    // setting and research completion afterwards in the same call. Technology-set
    // and random-stolen-bonus effects (source 2) can add progress alone to any
    // technology, so only source-2 calls mark ([`EFFECT_PROGRESS_SOURCE`]; the
    // detour reads R8). Saves RBX/RDI/R12.
    (
        MutationSite {
            name: "research_progress",
            rva: 0xec_0500,
            sentinel: [
                0x40, 0x53, 0x57, 0x41, 0x54, 0x41, 0x56, 0x48, 0x83, 0xec, 0x48, 0x48, 0x8b, 0xfa,
                0x41, 0x0f,
            ],
        },
        Receiver::Technology,
        Tables::TECHNOLOGY,
    ),
];

/// Indices in [`MUTATIONS`] of the sites whose writes only CountryAppearance
/// reads.
pub const APPEARANCE_SITES: std::ops::Range<usize> = 58..64;
/// Indices in [`MUTATIONS`] of the sites whose writes only
/// CountryCounterInputs reads.
pub const COUNTER_SITES: std::ops::Range<usize> = 64..70;

/// Index of unit-leader removal in [`MUTATIONS`], whose
/// detour compares the two leader roster counts ([`unit_leader_counts`])
/// before and after the original.
pub const REMOVE_UNIT_LEADER: usize = 67;
/// Index of slot synchronization in [`MUTATIONS`], whose detour compares the
/// slot roster header ([`slot_roster`]) before and after the original.
pub const SYNC_RESEARCH_SLOTS: usize = 15;
/// Index of the faction sharing base setter in [`MUTATIONS`], whose detour
/// compares the base ([`faction_sharing_base`]) before and after the
/// original: the daily refresh sets it for every faction.
pub const FACTION_SHARING_BASE: usize = 70;
/// Indices in [`MUTATIONS`] of the writers of province weather records
/// outside the native hourly window: their detours count each call on entry
/// and on return, and the weather read follows the window only while that
/// count is unchanged.
pub const WEATHER_WRITERS: std::ops::Range<usize> = 71..80;
/// Index of technology progress addition in [`MUTATIONS`], whose
/// detour marks only a call with [`EFFECT_PROGRESS_SOURCE`].
pub const RESEARCH_PROGRESS: usize = 80;
/// Progress source (R8): technology-setting and random-stolen-bonus effects
/// pass 2, daily slot progress and console pass 0, assignment passes 1.
pub const EFFECT_PROGRESS_SOURCE: u32 = 2;
/// Index of the console's grow_intel_network in [`MUTATIONS`], which writes
/// no country row: its detour counts the call on entry and on return.
pub const GROW_INTEL_NETWORK: usize = 46;
/// Indices of the sites in [`MUTATIONS`] that take a fifth (stack) argument,
/// which their detours forward.
pub const PROJECT_REWARD_OPTION: usize = 52;
pub const PROJECT_DATABASE_RESYNC: usize = 54;

/// Visits the native id (country `+0x8`, the census key) of every country
/// whose recorded rows a call with first (RCX) and second (RDX) can
/// change, as receiver names them. False when one cannot be named (a read
/// fails, a pointer is null or a link names no country) and for
/// [`Receiver::Every`]: the caller then counts the call against every
/// country. [`Receiver::Interception`] names nothing for another column.
pub fn countries(
    memory: &impl Memory,
    base: usize,
    receiver: Receiver,
    first: usize,
    second: usize,
    visit: &mut dyn FnMut(u32),
) -> bool {
    let mut named = |id: Option<u32>| match id {
        Some(id) => {
            visit(id);
            true
        }
        None => false,
    };
    match receiver {
        Receiver::Every => false,
        Receiver::Country => named(of_country(memory, first)),
        Receiver::Interception => {
            second as u32 != INTERCEPTION_COLUMN || named(of_country(memory, first))
        }
        Receiver::RelationStatus => named(
            pointer(memory, first, crelation_status::DIPLOMACY_STATUS)
                .and_then(|diplomacy| pointer(memory, diplomacy, cdiplomacy_status::COUNTRY))
                .and_then(|country| of_country(memory, country)),
        ),
        Receiver::WarRelation => [crelation::FIRST_COUNTRY, crelation::SECOND_COUNTRY]
            .into_iter()
            .all(|offset| {
                named(
                    first
                        .checked_add(offset)
                        .and_then(|address| read::<u32>(memory, address))
                        .and_then(|raw| of_link(memory, base, raw)),
                )
            }),
        Receiver::CountryCharacters => named(
            pointer(memory, first, ccountry_characters::OWNER)
                .and_then(|country| of_country(memory, country)),
        ),
        Receiver::PoliticalStatus => named(
            pointer(memory, first, cpolitical_status::COUNTRY)
                .and_then(|country| of_country(memory, country)),
        ),
        Receiver::Party => named(link_at(memory, base, first, cpolitical_party::TAG)),
        Receiver::Character => named(link_at(memory, base, first, ccharacter::COUNTRY)),
        Receiver::Scientist => named(
            pointer(memory, first, SCIENTIST_CHARACTER)
                .and_then(|character| link_at(memory, base, character, ccharacter::COUNTRY)),
        ),
        Receiver::ResearchSlot => named(
            pointer(memory, first, cresearch_slot::STATUS)
                .and_then(|status| status_country(memory, status)),
        ),
        Receiver::SharedResearchSlot => match pointer(memory, first, cresearch_slot::STATUS) {
            // The slot's technology now (the old one on entry, the new one
            // on return) and the one it is given.
            Some(status) => {
                let written = [
                    pointer(memory, first, cresearch_slot::TECHNOLOGY),
                    Some(second),
                ];
                named(status_country(memory, status))
                    && sharing_affected(memory, base, status, &written, visit)
            }
            None => false,
        },
        Receiver::TechnologyStatus => named(status_country(memory, first)),
        Receiver::Technology => named(
            pointer(memory, first, ctechnology::STATUS)
                .and_then(|status| status_country(memory, status)),
        ),
        Receiver::SharedTechnology => match pointer(memory, first, ctechnology::STATUS) {
            Some(status) => {
                named(status_country(memory, status))
                    && sharing_affected(memory, base, status, &[Some(first)], visit)
            }
            None => false,
        },
        Receiver::CompletedTechnology => {
            named(status_country(memory, first))
                && sharing_affected(memory, base, first, &[Some(second)], visit)
        }
        Receiver::SharedTechnologyStatus => {
            named(status_country(memory, first)) && sharing(memory, base, first, visit)
        }
        Receiver::ProgramStatus => named(link_at(
            memory,
            base,
            first,
            nproject_cprogram_status::OWNER,
        )),
        Receiver::FocusProgress => named(
            pointer(memory, first, cnational_focus_progress::COUNTRY)
                .and_then(|country| of_country(memory, country)),
        ),
        Receiver::Project => named(link_at(memory, base, first, nproject_cproject::OWNER)),
        Receiver::FactionSharing => {
            members(memory, base, first, visit)
                && match pointer(memory, first, FACTION_SHARING_FACTION) {
                    Some(faction) => faction_members(memory, faction, visit),
                    None => false,
                }
        }
        Receiver::License => named(link_at(
            memory,
            base,
            first,
            cequipment_production_license::OWNER,
        )),
        Receiver::DiplomacyStatus => named(diplomacy_country(memory, first)),
        Receiver::Subjects => {
            named(diplomacy_country(memory, first))
                && positive_link(
                    memory,
                    base,
                    first
                        .checked_add(cdiplomacy_status::OVERLORD)
                        .and_then(|address| read::<u32>(memory, address)),
                    visit,
                )
        }
        Receiver::StateOwner | Receiver::StateController => {
            let offset = if receiver == Receiver::StateOwner {
                cstate::OWNER
            } else {
                cstate::CONTROLLER
            };
            let old = (first != 0)
                .then(|| first.checked_add(offset))
                .flatten()
                .and_then(|address| read::<u32>(memory, address));
            // EDX: the upper half of RDX is not the argument's.
            positive_link(memory, base, old, visit)
                && positive_link(memory, base, Some(second as u32), visit)
        }
        Receiver::TagPointer => positive_link(
            memory,
            base,
            (second != 0).then(|| read::<u32>(memory, second)).flatten(),
            visit,
        ),
    }
}

/// Visits the country raw link id raw names while it is positive, as the
/// native writers test `0 < tag` before resolving one: a zero or negative id
/// names no country and is no failure. False when raw did not read or a
/// positive id names no country.
fn positive_link(
    memory: &impl Memory,
    base: usize,
    raw: Option<u32>,
    visit: &mut dyn FnMut(u32),
) -> bool {
    match raw {
        None => false,
        Some(raw) if raw as i32 <= 0 => true,
        Some(raw) => match of_link(memory, base, raw) {
            Some(id) => {
                visit(id);
                true
            }
            None => false,
        },
    }
}

/// The country of a diplomacy status (`+0x288`).
fn diplomacy_country(memory: &impl Memory, diplomacy: usize) -> Option<u32> {
    pointer(memory, diplomacy, cdiplomacy_status::COUNTRY)
        .and_then(|country| of_country(memory, country))
}

/// The slot roster header slot synchronization rewrites ([`SLOT_ROSTER`]):
/// equal before and after its call proves it changed no slot.
pub fn slot_roster(memory: &impl Memory, status: usize) -> Option<[u8; 16]> {
    read::<[u8; 16]>(memory, status.checked_add(SLOT_ROSTER)?)
}

/// The army and navy leader roster counts country character pool `+0x7C` and
/// `+0x94`, the only roster words unit-leader removal writes. It erases in
/// place: navy inline, army through a helper copying the tail and decrementing
/// count. It never reallocates, so equal counts before and after prove neither
/// roster changed. Deferred hourly removal of unlisted leaders writes nothing.
pub fn unit_leader_counts(memory: &impl Memory, characters: usize) -> Option<[u8; 16]> {
    let count = |roster: usize| {
        read::<[u8; 4]>(
            memory,
            characters.checked_add(roster)?.checked_add(LEADER_COUNT)?,
        )
    };
    let mut counts = [0; 16];
    counts[..4].copy_from_slice(&count(LEADER_ROSTERS[0])?);
    counts[4..8].copy_from_slice(&count(LEADER_ROSTERS[1])?);
    Some(counts)
}

/// country character pool `+0x70` army and `+0x88` navy leader rosters.
const LEADER_ROSTERS: [usize; 2] = [0x70, 0x88];
/// A roster's count word after its data pointer and capacity.
const LEADER_COUNT: usize = 0xC;

/// The faction sharing group's base `+0x10` and member array pointer `+0x18`
/// the base setter leaves alone: equal before and after its call
/// proves it changed no Sharing row.
pub fn faction_sharing_base(memory: &impl Memory, group: usize) -> Option<[u8; 16]> {
    read::<[u8; 16]>(memory, group.checked_add(FACTION_SHARING_BASE_OFFSET)?)
}

/// sharing group `+0x10`: the group's base bonus.
const FACTION_SHARING_BASE_OFFSET: usize = 0x10;
/// faction sharing group `+0x48`: its faction.
const FACTION_SHARING_FACTION: usize = 0x48;

/// Visits every country sharing research with status's country: the
/// members of each group on its sharing roster ([`SHARING_ROSTER`], raw ids
/// resolved through the links) and every member of its faction, because a
/// faction-mode group (template `+0x78`) on any member's roster takes its
/// members from that member's faction (`research_modifiers::sharing`). False
/// when anything does not read.
pub fn sharing(
    memory: &impl Memory,
    base: usize,
    status: usize,
    visit: &mut dyn FnMut(u32),
) -> bool {
    let (Some(group_vtable), Some(faction_vtable)) = (
        base.checked_add(ctechnology_sharing_group::VTABLE_RVA),
        base.checked_add(FACTION_SHARING_VTABLE_RVA),
    ) else {
        return false;
    };
    let Some(roster) = status
        .checked_add(SHARING_ROSTER)
        .and_then(|address| Array::read(memory, address, 8))
    else {
        return false;
    };
    for index in 0..roster.count {
        let Some(group) = roster
            .element(index, 8)
            .and_then(|address| pointer(memory, address, 0))
        else {
            return false;
        };
        let Some(vtable) = read::<u64>(memory, group) else {
            return false;
        };
        if vtable != group_vtable as u64 && vtable != faction_vtable as u64 {
            return false;
        }
        if !members(memory, base, group, visit) {
            return false;
        }
    }
    // The faction's members (faction `+0x58`, country pointer whose `+0x8` the
    // reader takes), whatever this country's roster lists.
    let Some(diplomacy) = pointer(memory, status, ctechnology_status::COUNTRY)
        .and_then(|country| pointer(memory, country, ccountry::DIPLOMACY_STATUS))
    else {
        return false;
    };
    let Some(faction) = diplomacy
        .checked_add(cdiplomacy_status::FACTION)
        .and_then(|address| read::<u64>(memory, address))
    else {
        return false;
    };
    match usize::try_from(faction) {
        Ok(0) => true,
        Ok(faction) => faction_members(memory, faction, visit),
        Err(_) => false,
    }
}

/// [sharing], visiting each partner's country pointer.
fn sharing_partners(
    memory: &impl Memory,
    base: usize,
    status: usize,
    visit: &mut dyn FnMut(usize),
) -> bool {
    let (Some(group_vtable), Some(faction_vtable)) = (
        base.checked_add(ctechnology_sharing_group::VTABLE_RVA),
        base.checked_add(FACTION_SHARING_VTABLE_RVA),
    ) else {
        return false;
    };
    let Some(roster) = status
        .checked_add(SHARING_ROSTER)
        .and_then(|address| Array::read(memory, address, 8))
    else {
        return false;
    };
    for index in 0..roster.count {
        let Some(group) = roster
            .element(index, 8)
            .and_then(|address| pointer(memory, address, 0))
        else {
            return false;
        };
        let Some(vtable) = read::<u64>(memory, group) else {
            return false;
        };
        if vtable != group_vtable as u64 && vtable != faction_vtable as u64 {
            return false;
        }
        if !member_countries(memory, base, group, visit) {
            return false;
        }
    }
    // The faction's members (faction `+0x58`, country pointer whose `+0x8` the
    // reader takes), whatever this country's roster lists.
    let Some(diplomacy) = pointer(memory, status, ctechnology_status::COUNTRY)
        .and_then(|country| pointer(memory, country, ccountry::DIPLOMACY_STATUS))
    else {
        return false;
    };
    let Some(faction) = diplomacy
        .checked_add(cdiplomacy_status::FACTION)
        .and_then(|address| read::<u64>(memory, address))
    else {
        return false;
    };
    match usize::try_from(faction) {
        Ok(0) => true,
        Ok(faction) => faction_countries(memory, faction, visit),
        Err(_) => false,
    }
}

/// Technology `+0xB0`: the dependents propagation recomputes
/// after a classifier change (count `+0xBC`).
const TECHNOLOGY_DEPENDENTS: usize = 0xb0;
/// technology `+0xC8`: the predecessors whose classifiers
/// technology classification and a sharing partner's predecessor
/// rows read (count `+0xD4`).
const TECHNOLOGY_PREDECESSORS: usize = 0xc8;

/// [sharing], narrowed to the partners whose rows a write of written
/// (technologies of status's country; `None` entries are skipped) can
/// change. A partner reads, for each technology it researches, this
/// country's instance at that template's database index
/// (`research_modifiers::sharing`): its level, and the
/// classifiers of its direct predecessors (`+0xC8`). A write changes the
/// written technology's level and, through propagation, the
/// classifiers of the written technology, of its dependents (`+0xB0`) and
/// of its listeners ([`ctechnology::LISTENERS`]: the technologies whose
/// template names it as a prerequisite or exclusion, which a dependent list
/// need not), transitively. So a partner is visited when one of its
/// researched templates' instances here is written, or has a predecessor
/// that is written or reached from a written one. Anything unreadable, a
/// listener that is not a technology, a pending listener container, or a
/// walk longer than this country's technology roster, visits every partner.
/// False as [sharing].
pub fn sharing_affected(
    memory: &impl Memory,
    base: usize,
    status: usize,
    written: &[Option<usize>],
    visit: &mut dyn FnMut(u32),
) -> bool {
    match affected_partners(memory, base, status, written) {
        Some(Some(affected)) => {
            for id in affected {
                visit(id);
            }
            true
        }
        Some(None) => false,
        None => sharing(memory, base, status, visit),
    }
}

/// The partner ids [`sharing_affected`] visits, ascending; `Some(None)` when
/// [sharing] would return false, `None` when the narrowing cannot complete
/// (an unreadable roster, instance or dependent, or an allocation failure),
/// so every partner is visited.
fn affected_partners(
    memory: &impl Memory,
    base: usize,
    status: usize,
    written: &[Option<usize>],
) -> Option<Option<Vec<u32>>> {
    fn push<T>(values: &mut Vec<T>, value: T) -> Option<()> {
        crate::allocation::reserve_push_batch(values, 1).ok()?;
        values.push(value);
        Some(())
    }
    let roster = Array::read(
        memory,
        status.checked_add(ctechnology_status::TECHNOLOGIES_DATA)?,
        8,
    )?;
    let mut partners: Vec<usize> = Vec::new();
    let mut failed = false;
    if !sharing_partners(memory, base, status, &mut |country| {
        failed |= push(&mut partners, country).is_none();
    }) {
        return Some(None);
    }
    if failed {
        return None;
    }
    let mut affected: Vec<u32> = Vec::new();
    // (predecessor of an instance a partner reads, that partner)
    let mut candidates: Vec<(usize, u32)> = Vec::new();
    for &country in &partners {
        let Some(id) = of_country(memory, country) else {
            return Some(None);
        };
        for instance in partner_reads(memory, country, roster)? {
            if written.contains(&Some(instance)) {
                push(&mut affected, id)?;
                continue;
            }
            let predecessors =
                Array::read(memory, instance.checked_add(TECHNOLOGY_PREDECESSORS)?, 8)?;
            for index in 0..predecessors.count {
                let predecessor = read::<u64>(memory, predecessors.element(index, 8)?)
                    .and_then(|p| usize::try_from(p).ok())?;
                push(&mut candidates, (predecessor, id))?;
            }
        }
    }
    if !candidates.is_empty() {
        // The classifiers propagate can rewrite: the written technologies
        // and, transitively, their dependents and the technologies listening
        // to them, at most the whole roster.
        let listener_vtable = base.checked_add(ctechnology::LISTENER_VTABLE_RVA)? as u64;
        let mut reached: Vec<usize> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        for &technology in written.iter().flatten() {
            if technology != 0 {
                push(&mut stack, technology)?;
            }
        }
        while let Some(technology) = stack.pop() {
            if reached.contains(&technology) {
                continue;
            }
            if reached.len() > roster.count as usize {
                return None;
            }
            push(&mut reached, technology)?;
            let dependents =
                Array::read(memory, technology.checked_add(TECHNOLOGY_DEPENDENTS)?, 8)?;
            for index in 0..dependents.count {
                let dependent = read::<u64>(memory, dependents.element(index, 8)?)
                    .and_then(|p| usize::try_from(p).ok())?;
                if dependent != 0 {
                    push(&mut stack, dependent)?;
                }
            }
            // Propagation notifies each listener through virtual slot 2; the
            // technology observer propagates from the object 0x10 bytes before
            // the listener. Other listener kinds or pending containers visit
            // every partner.
            if read::<u64>(
                memory,
                technology.checked_add(ctechnology::PENDING_LISTENERS)?,
            )? != 0
            {
                return None;
            }
            let listeners =
                Array::read(memory, technology.checked_add(ctechnology::LISTENERS)?, 8)?;
            for index in 0..listeners.count {
                let listener = read::<u64>(memory, listeners.element(index, 8)?)
                    .and_then(|p| usize::try_from(p).ok())?;
                if read::<u64>(memory, listener)? != listener_vtable {
                    return None;
                }
                push(&mut stack, listener.checked_sub(ctechnology::LISTENER)?)?;
            }
        }
        for &(predecessor, id) in &candidates {
            if reached.contains(&predecessor) {
                push(&mut affected, id)?;
            }
        }
    }
    affected.sort_unstable();
    affected.dedup();
    Some(Some(affected))
}

/// The instances of status's technology roster (roster) that country
/// reads as a sharing partner: the one at each researched template's
/// database index (technology status `+0xA0` slots, research slot `+0x18`,
/// technology `+0x160`, technology template `+0x38`). `None` when anything
/// does not read; an idle slot or a missing instance reads nothing.
fn partner_reads(memory: &impl Memory, country: usize, roster: Array) -> Option<Vec<usize>> {
    let status = pointer(memory, country, ccountry::TECHNOLOGY_STATUS)?;
    let slots = Array::read(
        memory,
        status.checked_add(ctechnology_status::RESEARCH_SLOTS_DATA)?,
        8,
    )?;
    let mut instances = Vec::new();
    for index in 0..slots.count {
        let slot = pointer(memory, slots.element(index, 8)?, 0)?;
        let Some(technology) = pointer(memory, slot, cresearch_slot::TECHNOLOGY) else {
            continue;
        };
        let template = pointer(memory, technology, ctechnology::TEMPLATE)?;
        let database = read::<i32>(
            memory,
            template.checked_add(ctechnology_template::DATABASE_INDEX)?,
        )?;
        let position = database
            .checked_sub(1)
            .and_then(|i| u32::try_from(i).ok())?;
        if position >= roster.count {
            continue;
        }
        if let Some(instance) = read::<u64>(memory, roster.element(position, 8)?)
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
        {
            crate::allocation::reserve_push_batch(&mut instances, 1).ok()?;
            instances.push(instance);
        }
    }
    Some(instances)
}

/// Visits each raw id on a sharing group's member array (`+0x18`, count
/// `+0x24`), resolved through the links. False when anything does not read.
fn members(memory: &impl Memory, base: usize, group: usize, visit: &mut dyn FnMut(u32)) -> bool {
    let Some(members) = group
        .checked_add(GROUP_MEMBERS)
        .and_then(|address| Array::read(memory, address, 4))
    else {
        return false;
    };
    for member in 0..members.count {
        let Some(id) = members
            .element(member, 4)
            .and_then(|address| read::<u32>(memory, address))
            .and_then(|raw| of_link(memory, base, raw))
        else {
            return false;
        };
        visit(id);
    }
    true
}

/// Visits each raw id on a sharing group's member array (`+0x18`, count
/// `+0x24`), resolved through the links. False when anything does not read.
fn member_countries(
    memory: &impl Memory,
    base: usize,
    group: usize,
    visit: &mut dyn FnMut(usize),
) -> bool {
    let Some(members) = group
        .checked_add(GROUP_MEMBERS)
        .and_then(|address| Array::read(memory, address, 4))
    else {
        return false;
    };
    for member in 0..members.count {
        let Some(id) = members
            .element(member, 4)
            .and_then(|address| read::<u32>(memory, address))
            .and_then(|raw| country_of_link(memory, base, raw))
        else {
            return false;
        };
        visit(id);
    }
    true
}

/// Visits each member of a faction (`+0x58`, country pointer whose `+0x8` the
/// reader takes). False when anything does not read.
fn faction_members(memory: &impl Memory, faction: usize, visit: &mut dyn FnMut(u32)) -> bool {
    let Some(members) = faction
        .checked_add(cfaction::MEMBERS)
        .and_then(|address| Array::read(memory, address, 8))
    else {
        return false;
    };
    for member in 0..members.count {
        let Some(id) = members
            .element(member, 8)
            .and_then(|address| pointer(memory, address, 0))
            .and_then(|country| of_country(memory, country))
        else {
            return false;
        };
        visit(id);
    }
    true
}

/// Visits each member country pointer of a faction (`+0x58`), reading its
/// country id at `+0x8`. False when anything does not read.
fn faction_countries(memory: &impl Memory, faction: usize, visit: &mut dyn FnMut(usize)) -> bool {
    let Some(members) = faction
        .checked_add(cfaction::MEMBERS)
        .and_then(|address| Array::read(memory, address, 8))
    else {
        return false;
    };
    for member in 0..members.count {
        let Some(id) = members
            .element(member, 8)
            .and_then(|address| pointer(memory, address, 0))
            .filter(|&country| of_country(memory, country).is_some())
        else {
            return false;
        };
        visit(id);
    }
    true
}

/// The native id of the country raw link id raw names, as
/// `ccountry::Reader::resolve` finds it: the game state's `COUNTRY_LINKS[raw]`
/// gives its slot and `COUNTRIES[slot]` its object, whose `+0x8` the census keys.
/// `None` for zero, an unlinked id or anything unreadable.
pub fn of_link(memory: &impl Memory, base: usize, raw: u32) -> Option<u32> {
    if raw == 0 {
        return None;
    }
    let state = pointer(memory, base, cgame_state::SINGLETON_RVA)?;
    let links = Array::read(memory, state.checked_add(cgame_state::COUNTRY_LINKS)?, 4)?;
    let slot = read::<u32>(memory, links.element(raw, 4)?)?;
    if slot == 0 {
        return None;
    }
    let countries = Array::read(memory, state.checked_add(cgame_state::COUNTRIES)?, 8)?;
    of_country(memory, pointer(memory, countries.element(slot, 8)?, 0)?)
}

/// The country pointer a raw link id names.
fn country_of_link(memory: &impl Memory, base: usize, raw: u32) -> Option<usize> {
    if raw == 0 {
        return None;
    }
    let state = pointer(memory, base, cgame_state::SINGLETON_RVA)?;
    let links = Array::read(memory, state.checked_add(cgame_state::COUNTRY_LINKS)?, 4)?;
    let slot = read::<u32>(memory, links.element(raw, 4)?)?;
    if slot == 0 {
        return None;
    }
    let countries = Array::read(memory, state.checked_add(cgame_state::COUNTRIES)?, 8)?;
    pointer(memory, countries.element(slot, 8)?, 0)
}

/// A country pointer's native id.
fn of_country(memory: &impl Memory, country: usize) -> Option<u32> {
    if country == 0 {
        return None;
    }
    read::<u32>(memory, country.checked_add(ccountry::ID)?)
}

/// The country of a technology status (`+0x130`).
fn status_country(memory: &impl Memory, status: usize) -> Option<u32> {
    pointer(memory, status, ctechnology_status::COUNTRY)
        .and_then(|country| of_country(memory, country))
}

/// The country a link id stored at `object + offset` names.
fn link_at(memory: &impl Memory, base: usize, object: usize, offset: usize) -> Option<u32> {
    if object == 0 {
        return None;
    }
    let raw = read::<u32>(memory, object.checked_add(offset)?)?;
    of_link(memory, base, raw)
}

/// The nonnull pointer stored at `object + offset`.
fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Option<usize> {
    if object == 0 {
        return None;
    }
    usize::try_from(read::<u64>(memory, object.checked_add(offset)?)?)
        .ok()
        .filter(|&pointer| pointer != 0)
}

#[cfg(test)]
mod tests;
