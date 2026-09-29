//! Single FIFO decode worker for the retained raw-capture families.
//!
//! Each staged payload owns its current copied bytes. The worker fills the
//! corresponding pooled typed rows before the spool ready-queue handoff.
//! Families acquired directly into typed rows need only forwarding; their rows
//! and measured costs remain intact for conversion and encoding.
//!
//! Ordering: every frame transits this one FIFO in game-thread production
//! order, so spool handoff order is inherently hour order and no reorder
//! buffer exists. Off-refresh families carry no payload. Every frame still
//! transits the FIFO so a slow refresh decode cannot let a later hour overtake it.
//!
//! Backpressure: enqueue does not block, but acquisition can wait for a free
//! capture buffer when downstream processing exhausts the shared pool.
//! The FIFO holds one job per pooled buffer (payload byte buffers live on the heap;
//! payload jobs are bounded and recycled, never pool memory), so an owned
//! buffer guarantees a free slot, as in the spool ready queue. A full queue
//! records a whole-frame loss through `LossBatch`. Staging capacity misses
//! instead use each family's admitted fallback and preserve its first failure;
//! accepted-job panics and failed prepared acquisitions cannot retry live reads.
//!
//! Storage is allocated before hook installation. `BUFFERS` bounds queued frames;
//! `JOB_POOL_KEEP` controls reusable payload storage per family or character part.
//! Exhaustion or lock contention returns an empty staging object without
//! allocating another payload. Retirement only transfers ownership on the game
//! thread; the decode worker clears, grows and recycles that storage.
//! Raw payload storage is additional to the typed frame pool. Queue depth,
//! process memory and per-family span/byte capacities require live measurement;
//! a sum of byte arenas alone is not a bound on aggregate allocation.
//!
//! Calibration: per-raw-family-table copy ns, decode ns and raw bytes are monotonic
//! additive counters readable from status (`calibration_json`, merged into the
//! runtime status additively). Per-frame copy-only ns also stays in the
//! existing `table_ns` slots for the staged tables.
//!
//! Counter semantics: `JOBS` counts every FIFO item enqueued (one per frame,
//! including skip-hour ordering placeholders that carry no payload).
//! `REFRESH_JOBS` counts every family-refresh payload decoded (one per moved
//! family per refresh hour, so several increments per frame when families
//! coincide). `FULL_REFRESH_JOBS` counts frames with `force_full` set.
//! `DROPS` counts queue-full whole-frame drops (the `LossBatch` gap path),
//! never staging overflows (those take the typed fallback with no gap).

use crate::{
    extended::{
        air::{
            AirBasesJob, AirBasesMemo, AirBasesMemoContext, AirHistoryJob, AirStatsJob, AirWingJob,
        },
        characters::{CharactersJob, PART_MASKS as CHARACTER_MASKS, PARTS as CHARACTER_PARTS},
        construction::ConstructionRestJob,
        economy::EconomyRestJob,
        fog::{IntelJob, RadarJob},
        governance::FocusJob,
        intel::{CapitulationJob, RelationsJob},
        lexical::LexicalJob,
        naval_history::{NavalJob, SunkJob, SupremacyJob},
        production::ProductionRestJob,
        projects::ProjectsJob,
        session::SessionJob,
        weather::WeatherJob,
    },
    map_capture::PlayersJob,
    sampling::{self, BUFFERS, DivisionsJob, Shared},
};
use crossbeam_queue::ArrayQueue;
use replay::rows::TableId;
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering},
};
/// Focus pair in calibration/report order: [focus, completed]. Both refresh
/// on daily slot 19. The single copy pass is charged to the focus slot.
pub const FOCUS: [TableId; 2] = [TableId::Focus, TableId::CompletedFocuses];

/// Relations family in calibration/report order: [opinions, tuples]. The
/// payload freezes each table's activity selected by replay cadence.
pub const RELATIONS: [TableId; 2] = [TableId::RelationOpinions, TableId::RelationTuples];

/// Characters family in calibration/report order: [characters, leaders,
/// advisors, units, traits, experience]. Cadence selects the active tables.
/// ActiveAbilities is separate.
pub const CHARACTERS: [TableId; 6] = [
    TableId::Characters,
    TableId::CountryLeaderRoles,
    TableId::AdvisorRoles,
    TableId::UnitLeaders,
    TableId::UnitLeaderTraits,
    TableId::UnitLeaderTraitExperience,
];

/// Projects family in calibration/report order: [programs, projects,
/// history, breakthroughs]. Hourly project tables stay game-side typed.
pub const PROJECTS: [TableId; 4] = [
    TableId::ProjectPrograms,
    TableId::Projects,
    TableId::ProjectHistory,
    TableId::ProjectBreakthroughs,
];

/// Lexical family: the global token dictionary.
pub const LEXICAL: [TableId; 1] = [TableId::LexicalTokens];
/// Divisions group in calibration/report order: [divisions, equipment,
/// groups, requests, deliveries, produced]. Divisions stay hourly; the
/// payload's details_active flag gates the five slot-2 detail tables. The
/// upgrades phase cost is charged to the groups slot.
pub const DIVISIONS: [TableId; 6] = [
    TableId::Divisions,
    TableId::DivisionEquipment,
    TableId::ArmyUpgradeGroups,
    TableId::ArmyUpgradeRequests,
    TableId::ArmyUpgradeDeliveries,
    TableId::ArmyUpgradeProduced,
];
/// Players pair in calibration/report order: [players, controllers].
/// Hourly like every Tier-H table (no refresh gate). Each stages its own
/// roster/census copies with no cross-family decoded roots.
pub const PLAYERS: [TableId; 2] = [TableId::Players, TableId::HumanControllers];

/// Session family in calibration/report order: [world, sources, aggregates].
/// All three refresh on daily slot 18; the shared manager header is charged
/// to the world slot.
pub const SESSION: [TableId; 3] = [
    TableId::WorldTension,
    TableId::ThreatSources,
    TableId::ThreatCountryAggregates,
];

/// Wing-fed air pair in calibration/report order: [equipment, cumulative].
/// Equipment stays hourly and cumulative refreshes on daily slot 16. Both
/// consume owned rows from the shared map/extension/naval wing census.
pub const AIR_WING: [TableId; 2] = [TableId::WingEquipment, TableId::WingCumulative];

/// Daily slot-5 air-statistics table in calibration/report order: [statistics].
/// Stages its own strategic-air census copies with no cross-family decoded roots.
pub const AIR_STATS: [TableId; 1] = [TableId::AirStatistics];

/// Shared bases payload in calibration/report order: [bases, registrations,
/// aces]. Bases and registrations stay hourly; aces refresh on daily slot 21.
/// The wing/manager census is taken once into shared raw spans; the bases,
/// registrations and aces decodes all read the shared spans (registrations
/// rows live in the fog pack and decode from this same log).
pub const AIR_BASES: [TableId; 3] = [
    TableId::AirBases,
    TableId::AirBaseRegistrations,
    TableId::AirAces,
];

/// Shared daily slot-7 history payload in calibration/report order: [history,
/// damage]. The single regional walk is charged to the history slot.
pub const AIR_HISTORY: [TableId; 2] = [TableId::AirRegionHistory, TableId::AirRegionDamage];
/// Daily slot-23 supremacy table in calibration/report order: [supremacy].
pub const SUPREMACY: [TableId; 1] = [TableId::NavalSupremacy];

/// Daily slot-19 sunk-ships table in calibration/report order: [sunk ships].
pub const SUNK: [TableId; 1] = [TableId::SunkShips];

/// Daily slot-21 radar table in calibration/report order: [radar].
pub const RADAR: [TableId; 1] = [TableId::RadarCoverage];

/// Hourly intel table in calibration/report order: [intel]. Hourly like
/// every Tier-H table (no refresh gate). The division/fleet census spans
/// are the walk's own takes, shared once inside the payload between the
/// division and fleet paths.
pub const INTEL: [TableId; 1] = [TableId::UnitIntel];

/// Source-fed naval block in calibration/report order: [land theatres, land
/// groups, land members, air theatres, air groups, air members, orders
/// groups, orders members, army members]. The six theatre tables refresh on
/// daily slot 13; the three order/member tables stay hourly. Shared
/// theatre roots are anchor-charged to land theatres; shared OOB roots to
/// orders groups, matching the read-group convention.
pub const NAVAL_ORDER: [TableId; 9] = [
    TableId::LandTheatres,
    TableId::LandTheatreGroups,
    TableId::LandTheatreMembers,
    TableId::AirTheatres,
    TableId::AirTheatreGroups,
    TableId::AirTheatreMembers,
    TableId::OrdersGroups,
    TableId::OrdersGroupMembers,
    TableId::ArmyGroupMembers,
];

/// Production daily slot-8 rest in calibration/report order: [lines, stockpiles,
/// licences, conveyors, lines, manpower, equipment, records]. The deployment
/// shared traversal is charged to the conveyors slot; the licence walk to the licences
/// slot, matching the read-group convention. Empty ProductionLicenseRecords
/// shares the licence walk and moves with it.
pub const PRODUCTION_REST: [TableId; 8] = [
    TableId::ProductionLines,
    TableId::Stockpiles,
    TableId::ProductionLicenses,
    TableId::DeploymentConveyors,
    TableId::DeploymentLines,
    TableId::DeploymentManpower,
    TableId::DeploymentEquipment,
    TableId::ProductionLicenseRecords,
];

/// Construction daily slot-3 rest in calibration/report order: [resources,
/// building, railway, settings, laws, garrisons, assigned, reserve,
/// resistance, collaboration]. The construction queue is charged to the building
/// slot; the occupation shared walk to the settings slot, matching the read-group
/// convention. Empty reserve/collaboration tables share their walks and move
/// with them.
pub const CONSTRUCTION_REST: [TableId; 10] = [
    TableId::StateBaseResources,
    TableId::BuildingConstruction,
    TableId::RailwayConstruction,
    TableId::OccupationSettings,
    TableId::StateOccupationLaws,
    TableId::StateGarrisons,
    TableId::StateAssignedManpower,
    TableId::CountryOccupationReserveManpower,
    TableId::StateResistance,
    TableId::CountryCollaboration,
];

/// Economy mixed rest in calibration/report order: [fuel, experience,
/// supply, hubs, disrupted, foreign, definitions, resources, exchanges,
/// purchases, fuel consumers]. Supply stays hourly, the three resource tables
/// refresh on slot 10, and the seven remaining tables on slot 17. The supply shared traversal is anchor-charged to the hubs slot; the
/// resource dictionary to the definitions slot and the shared fuel walk to the
/// fuel slot, matching the read-group convention.
pub const ECONOMY_REST: [TableId; crate::extended::economy::N] = [
    TableId::CountryFuel,
    TableId::CountryExperience,
    TableId::ProvinceSupply,
    TableId::HubMotorization,
    TableId::DisruptedSupply,
    TableId::ForeignHomebases,
    TableId::ResourceDefinitions,
    TableId::CountryResources,
    TableId::ResourceExchanges,
    TableId::PurchaseContracts,
    TableId::CountryFuelConsumers,
];

/// Hourly weather family in calibration/report order: [regions, provinces].
/// Hourly like every Tier-H table (no refresh gate). The manager header is
/// anchor-charged to the regions slot, matching the read-group convention.
pub const WEATHER: [TableId; 2] = [TableId::WeatherRegions, TableId::WeatherProvinces];

/// Hourly capitulation table in calibration/report order: [capitulation].
/// Hourly like every Tier-H table (no refresh gate).
pub const CAPITULATION: [TableId; 1] = [TableId::Capitulation];

/// Live raw payloads beyond this count are not retained; further refreshes
/// take the existing gap path until the worker catches up (see module docs).
const JOB_POOL_KEEP: usize = 2;

/// One FIFO item. The payload's byte buffers already live on the heap inside
/// its vectors; pooled-frame typed rows are filled by the worker before the
/// ready-queue handoff.
#[derive(Debug, Default)]
pub struct DecodeJob {
    pub index: usize,
    pub seq: u64,
    pub hour: i32,
    // Refresh flags declare an owned decode handoff. False can also mean the
    // callback installed typed outcomes. Per-table activity is frozen in the
    // payload; neither these flags nor timings establish acquisition freshness.
    pub relations_refresh: bool,
    pub characters_refresh: [bool; CHARACTER_PARTS],
    pub projects_refresh: bool,
    pub lexical_refresh: bool,
    pub divisions_refresh: bool,
    pub focus_refresh: bool,
    pub air_wing_refresh: bool,
    pub air_stats_refresh: bool,
    pub naval_refresh: bool,
    pub players_refresh: bool,
    pub session_refresh: bool,
    pub production_rest_refresh: bool,
    pub construction_rest_refresh: bool,
    pub economy_rest_refresh: bool,
    pub air_bases_refresh: bool,
    pub air_history_refresh: bool,
    pub supremacy_refresh: bool,
    pub sunk_refresh: bool,
    pub radar_refresh: bool,
    pub intel_refresh: bool,
    pub weather_refresh: bool,
    pub capitulation_refresh: bool,
    pub force_full: bool,
    pub game_elapsed: i64,
    pub relations_payload: Option<RelationsJob>,
    pub characters_payload: [Option<CharactersJob>; CHARACTER_PARTS],
    pub projects_payload: Option<ProjectsJob>,
    pub lexical_payload: Option<LexicalJob>,
    pub divisions_payload: Option<DivisionsJob>,
    pub focus_payload: Option<FocusJob>,
    pub air_wing_payload: Option<AirWingJob>,
    pub air_stats_payload: Option<AirStatsJob>,
    pub naval_payload: Option<NavalJob>,
    pub players_payload: Option<PlayersJob>,
    pub session_payload: Option<SessionJob>,
    pub production_rest_payload: Option<ProductionRestJob>,
    pub construction_rest_payload: Option<ConstructionRestJob>,
    pub economy_rest_payload: Option<EconomyRestJob>,
    pub air_bases_payload: Option<AirBasesJob>,
    pub air_history_payload: Option<AirHistoryJob>,
    pub supremacy_payload: Option<SupremacyJob>,
    pub sunk_payload: Option<SunkJob>,
    pub radar_payload: Option<RadarJob>,
    pub intel_payload: Option<IntelJob>,
    pub weather_payload: Option<WeatherJob>,
    pub capitulation_payload: Option<CapitulationJob>,
}
static RELATIONS_COPY_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static RELATIONS_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static RELATIONS_RAW_BYTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static CHARACTERS_COPY_NS: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static CHARACTERS_DECODE_NS: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static CHARACTERS_RAW_BYTES: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static PROJECTS_COPY_NS: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static PROJECTS_DECODE_NS: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static PROJECTS_RAW_BYTES: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static LEXICAL_COPY_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static LEXICAL_DECODE_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static LEXICAL_RAW_BYTES: [AtomicU64; 1] = [AtomicU64::new(0)];
static DIVISIONS_COPY_NS: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static DIVISIONS_DECODE_NS: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static DIVISION_PRESENTATION_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static DIVISIONS_RAW_BYTES: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static FOCUS_COPY_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static FOCUS_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static FOCUS_RAW_BYTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static AIR_WING_COPY_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static AIR_WING_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static AIR_WING_RAW_BYTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static AIR_STATS_COPY_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static AIR_STATS_DECODE_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static AIR_STATS_RAW_BYTES: [AtomicU64; 1] = [AtomicU64::new(0)];
static NAVAL_COPY_NS: [AtomicU64; 9] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static NAVAL_DECODE_NS: [AtomicU64; 9] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static NAVAL_RAW_BYTES: [AtomicU64; 9] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static PLAYERS_COPY_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static PLAYERS_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static PLAYERS_RAW_BYTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static SESSION_COPY_NS: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static SESSION_DECODE_NS: [AtomicU64; 3] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static SESSION_RAW_BYTES: [AtomicU64; 3] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static PRODUCTION_REST_COPY_NS: [AtomicU64; 8] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static PRODUCTION_REST_DECODE_NS: [AtomicU64; 8] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static PRODUCTION_REST_RAW_BYTES: [AtomicU64; 8] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static CONSTRUCTION_REST_COPY_NS: [AtomicU64; 10] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static CONSTRUCTION_REST_DECODE_NS: [AtomicU64; 10] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static CONSTRUCTION_REST_RAW_BYTES: [AtomicU64; 10] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static ECONOMY_REST_COPY_NS: [AtomicU64; crate::extended::economy::N] =
    [const { AtomicU64::new(0) }; crate::extended::economy::N];
static ECONOMY_REST_DECODE_NS: [AtomicU64; crate::extended::economy::N] =
    [const { AtomicU64::new(0) }; crate::extended::economy::N];
static ECONOMY_REST_RAW_BYTES: [AtomicU64; crate::extended::economy::N] =
    [const { AtomicU64::new(0) }; crate::extended::economy::N];
static AIR_BASES_COPY_NS: [AtomicU64; 3] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
// hits, misses, ineligible, comparisons, installs, compare/restore/install ns.
static AIR_BASES_MEMO: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
static AIR_BASES_DECODE_NS: [AtomicU64; 3] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static AIR_BASES_RAW_BYTES: [AtomicU64; 3] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static AIR_HISTORY_COPY_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static AIR_HISTORY_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static AIR_HISTORY_RAW_BYTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static SUPREMACY_COPY_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static SUPREMACY_DECODE_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static SUPREMACY_RAW_BYTES: [AtomicU64; 1] = [AtomicU64::new(0)];
static SUNK_COPY_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static SUNK_DECODE_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static SUNK_RAW_BYTES: [AtomicU64; 1] = [AtomicU64::new(0)];
static RADAR_COPY_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static RADAR_DECODE_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static RADAR_RAW_BYTES: [AtomicU64; 1] = [AtomicU64::new(0)];
static INTEL_COPY_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static INTEL_DECODE_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static INTEL_RAW_BYTES: [AtomicU64; 1] = [AtomicU64::new(0)];
static WEATHER_COPY_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static WEATHER_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static WEATHER_RAW_BYTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static CAPITULATION_COPY_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static CAPITULATION_DECODE_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static CAPITULATION_RAW_BYTES: [AtomicU64; 1] = [AtomicU64::new(0)];
static QUEUE_HIGH_WATER: AtomicU64 = AtomicU64::new(0);
static JOBS: AtomicU64 = AtomicU64::new(0);
static REFRESH_JOBS: AtomicU64 = AtomicU64::new(0);
static FULL_REFRESH_JOBS: AtomicU64 = AtomicU64::new(0);
static DROPS: AtomicU64 = AtomicU64::new(0);
static LAST_SEQ: AtomicU64 = AtomicU64::new(0);
static LAST_HOUR: AtomicU64 = AtomicU64::new(0);
static SEQ: AtomicU64 = AtomicU64::new(0);
static STOP: AtomicBool = AtomicBool::new(false);
static WORKER: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static WORKER_ID: AtomicU32 = AtomicU32::new(0);

/// Registered before hook installation, joined before final DLL quiescence.
pub fn worker_id() -> u32 {
    WORKER_ID.load(Ordering::Acquire)
}

/// Loader watchdog only. A missing or failed worker cannot return frame leases
/// and must release any callback waiting for a reusable buffer before detach.
pub fn worker_finished() -> bool {
    WORKER.lock().map_or(true, |slot| {
        slot.as_ref().is_none_or(|handle| handle.is_finished())
    })
}
static RELATIONS_JOB_POOL: Mutex<Vec<RelationsJob>> = Mutex::new(Vec::new());
static CHARACTERS_JOB_POOL: Mutex<[Vec<CharactersJob>; CHARACTER_PARTS]> =
    Mutex::new([const { Vec::new() }; CHARACTER_PARTS]);
static PROJECTS_JOB_POOL: Mutex<Vec<ProjectsJob>> = Mutex::new(Vec::new());
static LEXICAL_JOB_POOL: Mutex<Vec<LexicalJob>> = Mutex::new(Vec::new());
static DIVISIONS_JOB_POOL: Mutex<Vec<DivisionsJob>> = Mutex::new(Vec::new());
static FOCUS_JOB_POOL: Mutex<Vec<FocusJob>> = Mutex::new(Vec::new());
static AIR_WING_JOB_POOL: Mutex<Vec<AirWingJob>> = Mutex::new(Vec::new());
static AIR_STATS_JOB_POOL: Mutex<Vec<AirStatsJob>> = Mutex::new(Vec::new());
static NAVAL_JOB_POOL: Mutex<Vec<NavalJob>> = Mutex::new(Vec::new());
static PLAYERS_JOB_POOL: Mutex<Vec<PlayersJob>> = Mutex::new(Vec::new());
static SESSION_JOB_POOL: Mutex<Vec<SessionJob>> = Mutex::new(Vec::new());
static PRODUCTION_REST_JOB_POOL: Mutex<Vec<ProductionRestJob>> = Mutex::new(Vec::new());
static CONSTRUCTION_REST_JOB_POOL: Mutex<Vec<ConstructionRestJob>> = Mutex::new(Vec::new());
static ECONOMY_REST_JOB_POOL: Mutex<Vec<EconomyRestJob>> = Mutex::new(Vec::new());
static AIR_BASES_JOB_POOL: Mutex<Vec<AirBasesJob>> = Mutex::new(Vec::new());
static AIR_HISTORY_JOB_POOL: Mutex<Vec<AirHistoryJob>> = Mutex::new(Vec::new());
static SUPREMACY_JOB_POOL: Mutex<Vec<SupremacyJob>> = Mutex::new(Vec::new());
static SUNK_JOB_POOL: Mutex<Vec<SunkJob>> = Mutex::new(Vec::new());
static RADAR_JOB_POOL: Mutex<Vec<RadarJob>> = Mutex::new(Vec::new());
static INTEL_JOB_POOL: Mutex<Vec<IntelJob>> = Mutex::new(Vec::new());
static WEATHER_JOB_POOL: Mutex<Vec<WeatherJob>> = Mutex::new(Vec::new());
static CAPITULATION_JOB_POOL: Mutex<Vec<CapitulationJob>> = Mutex::new(Vec::new());

/// Admit a relations payload whenever any family table is active. The payload
/// freezes each table's activity for decode.
pub fn relations_refresh_hour(hour: i32, force_full: bool) -> bool {
    RELATIONS
        .iter()
        .any(|table| replay::cadence::should_walk(*table, hour, force_full))
}

/// Admit a character payload whenever any family table is active.
pub fn characters_refresh_hour(hour: i32, force_full: bool) -> bool {
    CHARACTERS
        .iter()
        .any(|table| replay::cadence::should_walk(*table, hour, force_full))
}

/// Projects refresh together on their daily slot (`replay::cadence`); full
/// hours (branch start, load, gap or an explicit refresh request) force their
/// complete acquisition.
pub fn projects_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay::cadence::should_walk(PROJECTS[0], hour, force_full);
    for table in PROJECTS.iter().skip(1) {
        debug_assert_eq!(walk, replay::cadence::should_walk(*table, hour, force_full));
    }
    walk
}

/// Admit the dictionary according to its authoritative replay cadence.
pub fn lexical_refresh_hour(hour: i32, force_full: bool) -> bool {
    replay::cadence::should_walk(LEXICAL[0], hour, force_full)
}

/// Monotonic frame sequence for FIFO order proofs. Gaps in the sequence are
/// dropped frames; order within the sequence is queue order.
pub fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_relations_job() -> RelationsJob {
    if let Ok(mut pool) = RELATIONS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    RelationsJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_relations_job(job: RelationsJob) {
    defer_payload(RetiredPayload::Relations(job));
}

fn release_relations_job(mut job: RelationsJob) {
    job.return_opinions();
    if !job.is_allocated() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = RELATIONS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_characters_job(part: usize) -> CharactersJob {
    if let Ok(mut pool) = CHARACTERS_JOB_POOL.try_lock()
        && let Some(job) = pool[part].pop()
    {
        return job;
    }
    let mut job = CharactersJob::default();
    job.part = part;
    job
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_characters_job(job: CharactersJob) {
    defer_payload(RetiredPayload::Characters(job));
}

fn release_characters_job(mut job: CharactersJob) {
    if !job.has_storage() {
        return;
    }
    job.note_growth();
    if job.grow_for_next() {
        job.grow();
    }
    job.prepare_read_plans();
    job.clear();
    if let Ok(mut pool) = CHARACTERS_JOB_POOL.lock()
        && pool[job.part].len() < JOB_POOL_KEEP + 1
    {
        pool[job.part].push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_projects_job() -> ProjectsJob {
    if let Ok(mut pool) = PROJECTS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ProjectsJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_projects_job(job: ProjectsJob) {
    defer_payload(RetiredPayload::Projects(job));
}

fn release_projects_job(mut job: ProjectsJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Projects", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.presize_countries();
    job.clear();
    if let Ok(mut pool) = PROJECTS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_lexical_job() -> LexicalJob {
    if let Ok(mut pool) = LEXICAL_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    LexicalJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_lexical_job(job: LexicalJob) {
    defer_payload(RetiredPayload::Lexical(job));
}

fn release_lexical_job(mut job: LexicalJob) {
    if !job.initialized() {
        return;
    }
    job.note_growth();
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = LEXICAL_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_divisions_job() -> DivisionsJob {
    if let Ok(mut pool) = DIVISIONS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    DivisionsJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_divisions_job(job: DivisionsJob) {
    defer_payload(RetiredPayload::Divisions(job));
}

fn release_divisions_job(mut job: DivisionsJob) {
    if !job.initialized() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = DIVISIONS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_focus_job() -> FocusJob {
    if let Ok(mut pool) = FOCUS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    FocusJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_focus_job(job: FocusJob) {
    defer_payload(RetiredPayload::Focus(job));
}

fn release_focus_job(mut job: FocusJob) {
    if !job.is_provisioned() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = FOCUS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_air_wing_job() -> AirWingJob {
    if let Ok(mut pool) = AIR_WING_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    AirWingJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_air_wing_job(job: AirWingJob) {
    defer_payload(RetiredPayload::AirWing(job));
}

fn release_air_wing_job(mut job: AirWingJob) {
    if !job.has_storage() && !job.grow_for_next() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = AIR_WING_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_air_stats_job() -> AirStatsJob {
    if let Ok(mut pool) = AIR_STATS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    AirStatsJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_air_stats_job(job: AirStatsJob) {
    defer_payload(RetiredPayload::AirStats(job));
}

fn release_air_stats_job(mut job: AirStatsJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("AirStats", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = AIR_STATS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_naval_job() -> NavalJob {
    if let Ok(mut pool) = NAVAL_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    NavalJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_naval_job(job: NavalJob) {
    defer_payload(RetiredPayload::Naval(job));
}

fn release_naval_job(mut job: NavalJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Naval", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.presize_countries();
    job.clear();
    if let Ok(mut pool) = NAVAL_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_players_job() -> PlayersJob {
    if let Ok(mut pool) = PLAYERS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    PlayersJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_players_job(job: PlayersJob) {
    defer_payload(RetiredPayload::Players(job));
}

fn release_players_job(mut job: PlayersJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Players", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = PLAYERS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_session_job() -> SessionJob {
    if let Ok(mut pool) = SESSION_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SessionJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_session_job(job: SessionJob) {
    defer_payload(RetiredPayload::Session(job));
}

fn release_session_job(mut job: SessionJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Session", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = SESSION_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_production_rest_job() -> ProductionRestJob {
    if let Ok(mut pool) = PRODUCTION_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ProductionRestJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_production_rest_job(job: ProductionRestJob) {
    defer_payload(RetiredPayload::ProductionRest(job));
}

fn release_production_rest_job(mut job: ProductionRestJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("ProductionRest", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    // Compile sealed source/control metadata at retirement, after any required
    // decode. Production decoding does not modify captured bytes. Reusing a
    // successful plan avoids recompiling it on every returned job.
    job.log.prepare_read_plan();
    job.presize_countries();
    job.clear();
    if let Ok(mut pool) = PRODUCTION_REST_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_construction_rest_job() -> ConstructionRestJob {
    if let Ok(mut pool) = CONSTRUCTION_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ConstructionRestJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_construction_rest_job(job: ConstructionRestJob) {
    defer_payload(RetiredPayload::ConstructionRest(job));
}

fn release_construction_rest_job(mut job: ConstructionRestJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("ConstructionRest", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.presize_countries();
    job.clear();
    if let Ok(mut pool) = CONSTRUCTION_REST_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_economy_rest_job() -> EconomyRestJob {
    if let Ok(mut pool) = ECONOMY_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    EconomyRestJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_economy_rest_job(job: EconomyRestJob) {
    defer_payload(RetiredPayload::EconomyRest(job));
}

fn release_economy_rest_job(mut job: EconomyRestJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("EconomyRest", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = ECONOMY_REST_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_air_bases_job() -> AirBasesJob {
    if let Ok(mut pool) = AIR_BASES_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    AirBasesJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_air_bases_job(job: AirBasesJob) {
    defer_payload(RetiredPayload::AirBases(job));
}

fn release_air_bases_job(mut job: AirBasesJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("AirBases", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.presize_countries();
    job.clear();
    if let Ok(mut pool) = AIR_BASES_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_air_history_job() -> AirHistoryJob {
    if let Ok(mut pool) = AIR_HISTORY_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    AirHistoryJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_air_history_job(job: AirHistoryJob) {
    defer_payload(RetiredPayload::AirHistory(job));
}

fn release_air_history_job(mut job: AirHistoryJob) {
    if !job.has_storage() && !job.grow_for_next() {
        return;
    }
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = AIR_HISTORY_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_supremacy_job() -> SupremacyJob {
    if let Ok(mut pool) = SUPREMACY_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SupremacyJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_supremacy_job(job: SupremacyJob) {
    defer_payload(RetiredPayload::Supremacy(job));
}

fn release_supremacy_job(mut job: SupremacyJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Supremacy", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = SUPREMACY_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_sunk_job() -> SunkJob {
    if let Ok(mut pool) = SUNK_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SunkJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_sunk_job(job: SunkJob) {
    defer_payload(RetiredPayload::Sunk(job));
}

fn release_sunk_job(mut job: SunkJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Sunk", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = SUNK_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_radar_job() -> RadarJob {
    if let Ok(mut pool) = RADAR_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    RadarJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_radar_job(job: RadarJob) {
    defer_payload(RetiredPayload::Radar(job));
}

fn release_radar_job(mut job: RadarJob) {
    if job.allocated_bytes() == 0 {
        return;
    }
    job.note_growth();
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = RADAR_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_intel_job() -> IntelJob {
    if let Ok(mut pool) = INTEL_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    IntelJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_intel_job(job: IntelJob) {
    defer_payload(RetiredPayload::Intel(job));
}

fn release_intel_job(mut job: IntelJob) {
    if job.allocated_bytes() == 0 {
        return;
    }
    job.note_growth();
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = INTEL_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_weather_job() -> WeatherJob {
    if let Ok(mut pool) = WEATHER_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    WeatherJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_weather_job(job: WeatherJob) {
    defer_payload(RetiredPayload::Weather(job));
}

fn release_weather_job(mut job: WeatherJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Weather", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = WEATHER_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Take reusable storage, or return an empty job without allocating on pool miss.
pub fn acquire_capitulation_job() -> CapitulationJob {
    if let Ok(mut pool) = CAPITULATION_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    CapitulationJob::default()
}

/// Defer clearing, growth and recycling to the decode worker.
pub fn recycle_capitulation_job(job: CapitulationJob) {
    defer_payload(RetiredPayload::Capitulation(job));
}

fn release_capitulation_job(mut job: CapitulationJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("Capitulation", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = CAPITULATION_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

const PAYLOADS_PER_FRAME: usize = 21 + CHARACTER_PARTS;
// Each family stages one owned payload or retires its failed staging object;
// each independent character part owns a separate payload.
// The worker takes each queued frame before draining retirement, so that frame's
// callback has already retired its failures before its lease can be returned.
// At most BUFFERS owned frames can contribute between these drains.
const RETIREMENT_CAPACITY: usize = BUFFERS * PAYLOADS_PER_FRAME;

// Queue storage is allocated before the hook is enabled; the game thread
// only moves vector handles here. Growth and allocator traffic stay off it.
#[allow(clippy::large_enum_variant)]
enum RetiredPayload {
    Relations(RelationsJob),
    Characters(CharactersJob),
    Projects(ProjectsJob),
    Lexical(LexicalJob),
    Divisions(DivisionsJob),
    Focus(FocusJob),
    AirWing(AirWingJob),
    AirStats(AirStatsJob),
    Naval(NavalJob),
    Players(PlayersJob),
    Session(SessionJob),
    ProductionRest(ProductionRestJob),
    ConstructionRest(ConstructionRestJob),
    EconomyRest(EconomyRestJob),
    AirBases(AirBasesJob),
    AirHistory(AirHistoryJob),
    Supremacy(SupremacyJob),
    Sunk(SunkJob),
    Radar(RadarJob),
    Intel(IntelJob),
    Weather(WeatherJob),
    Capitulation(CapitulationJob),
}
static RETIRED_PAYLOADS: AtomicPtr<ArrayQueue<RetiredPayload>> =
    AtomicPtr::new(std::ptr::null_mut());

fn prepare_retirement() {
    if !RETIRED_PAYLOADS.load(Ordering::Acquire).is_null() {
        return;
    }
    let queue = Box::into_raw(Box::new(ArrayQueue::new(RETIREMENT_CAPACITY)));
    if RETIRED_PAYLOADS
        .compare_exchange(
            std::ptr::null_mut(),
            queue,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        // Another initializer won; this allocation was never published.
        unsafe {
            drop(Box::from_raw(queue));
        }
    }
}

/// Retain pools while callbacks can still reach the DLL on a failed detach.
/// Normal detach/cancel releases them before self-unload; Rust statics do
/// not run destructors when Windows unloads a DLL.
pub struct PayloadPools;

impl Drop for PayloadPools {
    fn drop(&mut self) {
        if !sampling::SHARED.load(Ordering::Acquire).is_null() {
            return;
        }
        reclaim_payloads();

        if let Ok(mut pool) = RELATIONS_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = CHARACTERS_JOB_POOL.lock() {
            *pool = [const { Vec::new() }; CHARACTER_PARTS];
        }
        if let Ok(mut pool) = PROJECTS_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = LEXICAL_JOB_POOL.lock() {
            *pool = Vec::new();
        }

        if let Ok(mut pool) = DIVISIONS_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = FOCUS_JOB_POOL.lock() {
            *pool = Vec::new();
        }

        if let Ok(mut pool) = AIR_WING_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = AIR_STATS_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = NAVAL_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = PLAYERS_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = SESSION_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = PRODUCTION_REST_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = CONSTRUCTION_REST_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = ECONOMY_REST_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = AIR_BASES_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = AIR_HISTORY_JOB_POOL.lock() {
            *pool = Vec::new();
        }

        if let Ok(mut pool) = SUPREMACY_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = SUNK_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = RADAR_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = INTEL_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = WEATHER_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = CAPITULATION_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        let queue = RETIRED_PAYLOADS.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !queue.is_null() {
            // The loader clears SHARED only after callback quiescence and
            // joining/draining the decode worker. No queue borrow survives.
            unsafe {
                drop(Box::from_raw(queue));
            }
        }
    }
}

/// Spool thread: presize pooled raw-copy jobs whose country census follows the
/// registry, after a larger registry was observed (for example a save loaded
/// after attaching at the main menu). The game thread only `try_lock`s these
/// pools, so a concurrent callback takes a fresh job instead of waiting.
pub fn presize_country_pools() {
    if let Ok(mut pool) = PROJECTS_JOB_POOL.lock() {
        pool.iter_mut().for_each(ProjectsJob::presize_countries);
    }
    if let Ok(mut pool) = PRODUCTION_REST_JOB_POOL.lock() {
        pool.iter_mut()
            .for_each(ProductionRestJob::presize_countries);
    }
    if let Ok(mut pool) = CONSTRUCTION_REST_JOB_POOL.lock() {
        pool.iter_mut()
            .for_each(ConstructionRestJob::presize_countries);
    }
    if let Ok(mut pool) = AIR_BASES_JOB_POOL.lock() {
        pool.iter_mut().for_each(AirBasesJob::presize_countries);
    }
    if let Ok(mut pool) = NAVAL_JOB_POOL.lock() {
        pool.iter_mut().for_each(NavalJob::presize_countries);
    }
}

pub fn prepare_payload_pools() -> PayloadPools {
    prepare_retirement();
    for _ in 0..JOB_POOL_KEEP + 1 {
        release_relations_job(RelationsJob::new());
        for part in 0..CHARACTER_PARTS {
            release_characters_job(CharactersJob::new_part(part));
        }
        release_projects_job(ProjectsJob::new());
        release_lexical_job(LexicalJob::new());
        release_divisions_job(DivisionsJob::new());
        release_focus_job(FocusJob::new());
        release_air_wing_job(AirWingJob::new());
        release_air_stats_job(AirStatsJob::new());
        release_naval_job(NavalJob::new());
        release_players_job(PlayersJob::new());
        release_session_job(SessionJob::new());
        release_production_rest_job(ProductionRestJob::new());
        release_construction_rest_job(ConstructionRestJob::new());
        release_economy_rest_job(EconomyRestJob::new());
        release_air_bases_job(AirBasesJob::new());
        release_air_history_job(AirHistoryJob::new());
        release_supremacy_job(SupremacyJob::new());
        release_sunk_job(SunkJob::new());
        release_radar_job(RadarJob::new());
        release_intel_job(IntelJob::new());
        release_weather_job(WeatherJob::new());
        release_capitulation_job(CapitulationJob::new());
    }
    PayloadPools
}

fn defer_payload(payload: RetiredPayload) {
    let queue = RETIRED_PAYLOADS.load(Ordering::Acquire);
    assert!(
        !queue.is_null(),
        "payload retirement initialized before hook"
    );
    // Preparation precedes enabling the hook; reclamation follows quiescence.
    let queue = unsafe { &*queue };
    assert!(
        queue.push(payload).is_ok(),
        "owned-frame retirement bound exceeded"
    );
}

fn reclaim_payloads() {
    let queue = RETIRED_PAYLOADS.load(Ordering::Acquire);
    if queue.is_null() {
        return;
    }
    // Only the decode worker or the quiescent loader drains this queue.
    let queue = unsafe { &*queue };
    while let Some(payload) = queue.pop() {
        match payload {
            RetiredPayload::Relations(job) => release_relations_job(job),
            RetiredPayload::Characters(job) => release_characters_job(job),
            RetiredPayload::Projects(job) => release_projects_job(job),
            RetiredPayload::Lexical(job) => release_lexical_job(job),
            RetiredPayload::Divisions(job) => release_divisions_job(job),
            RetiredPayload::Focus(job) => release_focus_job(job),
            RetiredPayload::AirWing(job) => release_air_wing_job(job),
            RetiredPayload::AirStats(job) => release_air_stats_job(job),
            RetiredPayload::Naval(job) => release_naval_job(job),
            RetiredPayload::Players(job) => release_players_job(job),
            RetiredPayload::Session(job) => release_session_job(job),
            RetiredPayload::ProductionRest(job) => release_production_rest_job(job),
            RetiredPayload::ConstructionRest(job) => release_construction_rest_job(job),
            RetiredPayload::EconomyRest(job) => release_economy_rest_job(job),
            RetiredPayload::AirBases(job) => release_air_bases_job(job),
            RetiredPayload::AirHistory(job) => release_air_history_job(job),
            RetiredPayload::Supremacy(job) => release_supremacy_job(job),
            RetiredPayload::Sunk(job) => release_sunk_job(job),
            RetiredPayload::Radar(job) => release_radar_job(job),
            RetiredPayload::Intel(job) => release_intel_job(job),
            RetiredPayload::Weather(job) => release_weather_job(job),
            RetiredPayload::Capitulation(job) => release_capitulation_job(job),
        }
    }
}

/// Enqueue a frame for decode (or ordered forwarding for skip hours).
/// The loader starts the worker before enabling the hook. A full queue returns the job for
/// the existing gap path and counts the drop on the existing counters.
pub fn enqueue(shared: &Shared, job: DecodeJob) -> Option<DecodeJob> {
    match shared.decode_queue.push(job) {
        Ok(()) => {
            JOBS.fetch_add(1, Ordering::Relaxed);
            // Decode-queue depth high-water for live backlog visibility.
            // Records the deepest observed FIFO occupancy (bounded by BUFFERS).
            let depth = shared.decode_queue.len() as u64;
            QUEUE_HIGH_WATER.fetch_max(depth, Ordering::Relaxed);
            None
        }
        Err(job) => {
            DROPS.fetch_add(1, Ordering::Relaxed);
            Some(job)
        }
    }
}

/// The single FIFO decode worker. Game frames arrive in hour order and hand
/// off to the spool ready queue in the same order; no reorder buffer exists.
fn decode_loop() {
    let mut air_bases_memo = AirBasesMemo::default();
    let mut priority = crate::runtime::OwnPriority::new();
    loop {
        if STOP.load(Ordering::Acquire) {
            break;
        }
        let shared_ptr = sampling::SHARED.load(Ordering::Acquire);
        if shared_ptr.is_null() {
            std::thread::yield_now();
            continue;
        }
        let shared = unsafe { &*shared_ptr };
        priority.follow(shared.buffers_backlogged());
        match shared.decode_queue.pop() {
            Some(job) => process_owned_job(shared, job, &mut air_bases_memo),
            None => {
                reclaim_payloads();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
    reclaim_payloads();
}

/// The worker owns the raw payloads through decoding, then returns every
/// family to its pool. Dropping this envelope without recycling used to
/// allocate a fresh payload on the game thread on every subsequent hour.
fn process_owned_job(shared: &Shared, mut job: DecodeJob, air_bases_memo: &mut AirBasesMemo) {
    // Enqueue follows the callback's failed-payload retirement. Drain after
    // taking this frame, before publication lets the spool thread return its
    // lease. Draining before the queue pop permits one slot to retire twice.
    reclaim_payloads();
    process_job(shared, &mut job, air_bases_memo);

    if let Some(payload) = job.relations_payload.take() {
        release_relations_job(payload);
    }
    for part in &mut job.characters_payload {
        if let Some(payload) = part.take() {
            release_characters_job(payload);
        }
    }
    if let Some(payload) = job.projects_payload.take() {
        release_projects_job(payload);
    }
    if let Some(payload) = job.lexical_payload.take() {
        release_lexical_job(payload);
    }

    if let Some(payload) = job.divisions_payload.take() {
        release_divisions_job(payload);
    }
    if let Some(payload) = job.focus_payload.take() {
        release_focus_job(payload);
    }

    if let Some(payload) = job.air_wing_payload.take() {
        release_air_wing_job(payload);
    }
    if let Some(payload) = job.air_stats_payload.take() {
        release_air_stats_job(payload);
    }
    if let Some(payload) = job.naval_payload.take() {
        release_naval_job(payload);
    }
    if let Some(payload) = job.players_payload.take() {
        release_players_job(payload);
    }
    if let Some(payload) = job.session_payload.take() {
        release_session_job(payload);
    }
    if let Some(payload) = job.production_rest_payload.take() {
        release_production_rest_job(payload);
    }
    if let Some(payload) = job.construction_rest_payload.take() {
        release_construction_rest_job(payload);
    }
    if let Some(payload) = job.economy_rest_payload.take() {
        release_economy_rest_job(payload);
    }
    if let Some(payload) = job.air_bases_payload.take() {
        release_air_bases_job(payload);
    }
    if let Some(payload) = job.air_history_payload.take() {
        release_air_history_job(payload);
    }

    if let Some(payload) = job.supremacy_payload.take() {
        release_supremacy_job(payload);
    }
    if let Some(payload) = job.sunk_payload.take() {
        release_sunk_job(payload);
    }
    if let Some(payload) = job.radar_payload.take() {
        release_radar_job(payload);
    }
    if let Some(payload) = job.intel_payload.take() {
        release_intel_job(payload);
    }
    if let Some(payload) = job.weather_payload.take() {
        release_weather_job(payload);
    }
    if let Some(payload) = job.capitulation_payload.take() {
        release_capitulation_job(payload);
    }
}

pub fn start_worker() -> Result<(), String> {
    let mut slot = WORKER
        .lock()
        .map_err(|_| "decode worker lock poisoned".to_string())?;
    if slot.as_ref().is_some_and(|handle| !handle.is_finished()) {
        return Err("decode worker already running".into());
    }
    STOP.store(false, Ordering::Release);
    WORKER_ID.store(0, Ordering::Release);
    *slot = Some(
        std::thread::Builder::new()
            .name("hoi4-decode".into())
            .spawn(|| {
                WORKER_ID.store(
                    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
                    Ordering::Release,
                );
                decode_loop();
            })
            .map_err(|error| format!("cannot start decode worker: {error}"))?,
    );
    while worker_id() == 0 {
        std::thread::yield_now();
    }
    Ok(())
}

const MISSING_PAYLOAD_REASON: &str = "missing active decode payload";

/// A declared handoff without its payload is terminal. An undeclared handoff
/// may already contain typed results or skipped outcomes, which remain intact.
fn fail_missing_payloads(frame: &mut sampling::Captured, job: &DecodeJob) {
    if job.relations_refresh && job.relations_payload.is_none() {
        let captured = frame.extended.relations_pilot();
        captured.fail_relations_capture(captured.relation_activity(), MISSING_PAYLOAD_REASON);
    }
    for (part, &refresh) in job.characters_refresh.iter().enumerate() {
        if refresh && job.characters_payload[part].is_none() {
            let captured = frame.extended.characters_pilot();
            let active = std::array::from_fn(|index| {
                CHARACTER_MASKS[part][index] && captured.character_activity()[index]
            });
            captured.fail_character_tables(active, MISSING_PAYLOAD_REASON);
        }
    }
    if job.projects_refresh && job.projects_payload.is_none() {
        frame
            .extended
            .projects_pilot()
            .fail_projects_capture(true, MISSING_PAYLOAD_REASON);
    }
    if job.lexical_refresh && job.lexical_payload.is_none() {
        frame
            .extended
            .lexical_pilot()
            .fail_lexical_capture(MISSING_PAYLOAD_REASON);
    }
    if job.divisions_refresh && job.divisions_payload.is_none() {
        frame.divisions_result = engine::walk::Outcome::Unavailable {
            reason: MISSING_PAYLOAD_REASON,
        };
        frame.extended.fail_division_details(MISSING_PAYLOAD_REASON);
        frame.extended.set_divisions_raw_mode(false);
    }
    if job.focus_refresh && job.focus_payload.is_none() {
        frame
            .extended
            .focus_pilot()
            .fail_focus_capture(true, MISSING_PAYLOAD_REASON);
    }
    if job.air_stats_refresh && job.air_stats_payload.is_none() {
        frame
            .extended
            .air_stats_pilot()
            .fail_air_stats_capture(MISSING_PAYLOAD_REASON);
    }
    if job.air_wing_refresh && job.air_wing_payload.is_none() {
        frame
            .extended
            .air_wing_pilot()
            .fail_air_wing_capture(MISSING_PAYLOAD_REASON);
    }
    if job.naval_refresh && job.naval_payload.is_none() {
        frame
            .extended
            .naval_pilot()
            .fail_naval_capture(MISSING_PAYLOAD_REASON);
    }
    if job.players_refresh && job.players_payload.is_none() {
        frame.map.fail_players_capture(MISSING_PAYLOAD_REASON);
    }
    if job.session_refresh && job.session_payload.is_none() {
        frame
            .extended
            .session_pilot()
            .fail_session_capture(MISSING_PAYLOAD_REASON);
    }
    if job.production_rest_refresh && job.production_rest_payload.is_none() {
        frame
            .extended
            .production_rest_pilot()
            .fail_production_rest_capture(MISSING_PAYLOAD_REASON);
    }
    if job.construction_rest_refresh && job.construction_rest_payload.is_none() {
        frame
            .extended
            .construction_rest_pilot()
            .fail_construction_rest_capture(MISSING_PAYLOAD_REASON);
    }
    if job.economy_rest_refresh && job.economy_rest_payload.is_none() {
        frame
            .extended
            .economy_rest_pilot()
            .fail_economy_rest_capture(MISSING_PAYLOAD_REASON);
    }
    if job.air_bases_refresh && job.air_bases_payload.is_none() {
        frame
            .extended
            .air_bases_pilot()
            .fail_air_bases_capture(MISSING_PAYLOAD_REASON);
        frame
            .extended
            .radar_pilot()
            .fail_registrations_capture(MISSING_PAYLOAD_REASON);
    }
    if job.air_history_refresh && job.air_history_payload.is_none() {
        frame
            .extended
            .air_history_pilot()
            .fail_air_history_capture(MISSING_PAYLOAD_REASON);
    }
    if job.supremacy_refresh && job.supremacy_payload.is_none() {
        frame
            .extended
            .supremacy_pilot()
            .fail_supremacy_capture(MISSING_PAYLOAD_REASON);
    }
    if job.sunk_refresh && job.sunk_payload.is_none() {
        frame
            .extended
            .sunk_pilot()
            .fail_sunk_capture(MISSING_PAYLOAD_REASON);
    }
    if job.radar_refresh && job.radar_payload.is_none() {
        frame
            .extended
            .radar_pilot()
            .fail_radar_capture(MISSING_PAYLOAD_REASON);
    }
    if job.intel_refresh && job.intel_payload.is_none() {
        frame
            .extended
            .intel_pilot()
            .fail_intel_capture(MISSING_PAYLOAD_REASON);
    }
    if job.weather_refresh && job.weather_payload.is_none() {
        frame
            .extended
            .weather_pilot()
            .fail_weather_capture(MISSING_PAYLOAD_REASON);
    }
    if job.capitulation_refresh && job.capitulation_payload.is_none() {
        frame
            .extended
            .capitulation_pilot()
            .fail_capitulation_capture(MISSING_PAYLOAD_REASON);
    }
}

/// Decode one FIFO item and hand its pooled frame to the spool ready queue
/// in FIFO hour order. Skip items forward without touching typed rows.
fn process_job(shared: &Shared, job: &mut DecodeJob, air_bases_memo: &mut AirBasesMemo) {
    if !job.air_bases_refresh || job.air_bases_payload.is_none() {
        air_bases_memo.invalidate();
    }
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    fail_missing_payloads(frame, job);
    frame.extended.decode_air_copies();
    LAST_SEQ.store(job.seq, Ordering::Relaxed);
    LAST_HOUR.store(job.hour as u64, Ordering::Relaxed);
    if job.force_full {
        FULL_REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }

    if job.relations_refresh
        && let Some(payload) = job.relations_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        frame
            .extended
            .relations_pilot()
            .decode_relations_payload(payload, &mut decode_ticks);
        for (total, ticks) in RELATIONS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    for (refresh, payload) in job
        .characters_refresh
        .iter()
        .zip(&mut job.characters_payload)
    {
        let Some(payload) = payload.as_mut().filter(|_| *refresh) else {
            continue;
        };
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 6];
        let characters = frame.extended.characters_pilot();
        let before_bytes = characters.allocated_bytes();
        characters.decode_characters_payload(shared.base, payload, &mut decode_ticks);
        characters.remember_templates(payload);
        shared.pool_bytes.fetch_add(
            characters.allocated_bytes().saturating_sub(before_bytes) as u64,
            Ordering::Relaxed,
        );
        for (total, ticks) in CHARACTERS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.projects_refresh
        && let Some(payload) = job.projects_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 4];
        frame.extended.projects_pilot().decode_projects_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in PROJECTS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.lexical_refresh
        && let Some(payload) = job.lexical_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 1];
        let lexical = frame.extended.lexical_pilot();
        let before_bytes = lexical.allocated_bytes();
        lexical.decode_lexical_payload(shared.base, payload, &mut decode_ticks);
        shared.pool_bytes.fetch_add(
            lexical.allocated_bytes().saturating_sub(before_bytes) as u64,
            Ordering::Relaxed,
        );
        for (total, ticks) in LEXICAL_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }

    if job.divisions_refresh
        && let Some(payload) = job.divisions_payload.as_mut()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 6];
        let before_bytes = frame.extended.allocated_bytes();
        sampling::decode_divisions_payload(frame, shared.base, payload, &mut decode_ticks);
        let after_bytes = frame.extended.allocated_bytes();
        // Payload ownership is charged by decode_divisions_payload. Account
        // for this frame's side of the transfer, including any smaller swap.
        if after_bytes >= before_bytes {
            shared
                .pool_bytes
                .fetch_add((after_bytes - before_bytes) as u64, Ordering::Relaxed);
        } else {
            shared
                .pool_bytes
                .fetch_sub((before_bytes - after_bytes) as u64, Ordering::Relaxed);
        }
        for (total, ticks) in DIVISIONS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.focus_refresh
        && let Some(payload) = job.focus_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        frame
            .extended
            .focus_pilot()
            .decode_focus_payload(shared.base, payload, &mut decode_ticks);
        for (total, ticks) in FOCUS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }

    if job.air_wing_refresh
        && let Some(payload) = job.air_wing_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        crate::extended::air::decode_air_payload(
            frame.extended.air_wing_pilot(),
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in AIR_WING_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.air_stats_refresh
        && let Some(payload) = job.air_stats_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 1];
        crate::extended::air::decode_air_stats_payload(
            frame.extended.air_stats_pilot(),
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in AIR_STATS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.naval_refresh
        && let Some(payload) = job.naval_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 9];
        crate::extended::naval_history::decode_naval_payload(
            frame.extended.naval_pilot(),
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in NAVAL_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.players_refresh
        && let Some(payload) = job.players_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        frame
            .map
            .decode_players_payload(shared.base, payload, &mut decode_ticks);
        for (total, ticks) in PLAYERS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.session_refresh
        && let Some(payload) = job.session_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 3];
        frame.extended.session_pilot().decode_session_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in SESSION_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.production_rest_refresh
        && let Some(payload) = job.production_rest_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 8];
        frame
            .extended
            .production_rest_pilot()
            .decode_production_rest_payload(
                shared.base,
                payload.game_state,
                payload,
                &mut decode_ticks,
            );
        for (total, ticks) in PRODUCTION_REST_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.construction_rest_refresh
        && let Some(payload) = job.construction_rest_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 10];
        let construction = frame.extended.construction_rest_pilot();
        let before_bytes = construction.allocated_bytes();
        construction.decode_construction_rest_payload(
            shared.base,
            payload.game_state,
            payload,
            &mut decode_ticks,
        );
        shared.pool_bytes.fetch_add(
            construction.allocated_bytes().saturating_sub(before_bytes) as u64,
            Ordering::Relaxed,
        );
        for (total, ticks) in CONSTRUCTION_REST_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.economy_rest_refresh
        && let Some(payload) = job.economy_rest_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; crate::extended::economy::N];
        frame
            .extended
            .economy_rest_pilot()
            .decode_economy_rest_payload(shared.base, payload, &mut decode_ticks);
        for (total, ticks) in ECONOMY_REST_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.air_bases_refresh
        && let Some(payload) = job.air_bases_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 3];
        let report = frame.extended.decode_air_bases_payload(
            shared.base,
            payload,
            air_bases_memo,
            AirBasesMemoContext {
                owner: shared as *const Shared as usize,
                base: shared.base,
                seq: job.seq,
                hour: job.hour,
                force_full: job.force_full,
            },
            &mut decode_ticks,
        );
        // Once per worker refresh, never on a game read. These costs are also
        // included in the family's existing decode timers, not claimed as free.
        for (cell, value) in AIR_BASES_MEMO.iter().zip([
            u64::from(report.hit),
            u64::from(!report.hit),
            u64::from(!report.eligible),
            u64::from(report.compared),
            u64::from(report.installed),
            shared.ns(report.compare_ticks as i64),
            shared.ns(report.restore_ticks as i64),
            shared.ns(report.install_ticks as i64),
        ]) {
            cell.fetch_add(value, Ordering::Relaxed);
        }
        for (total, ticks) in AIR_BASES_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.air_history_refresh
        && let Some(payload) = job.air_history_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        frame
            .extended
            .air_history_pilot()
            .decode_air_history_payload(payload, &mut decode_ticks);
        for (total, ticks) in AIR_HISTORY_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }

    if job.supremacy_refresh
        && let Some(payload) = job.supremacy_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 1];
        frame.extended.supremacy_pilot().decode_supremacy_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in SUPREMACY_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.sunk_refresh
        && let Some(payload) = job.sunk_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 1];
        frame
            .extended
            .sunk_pilot()
            .decode_sunk_payload(shared.base, payload, &mut decode_ticks);
        for (total, ticks) in SUNK_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.radar_refresh
        && let Some(payload) = job.radar_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 1];
        frame
            .extended
            .radar_pilot()
            .decode_radar_payload(payload, &mut decode_ticks);
        for (total, ticks) in RADAR_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.intel_refresh
        && let Some(payload) = job.intel_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 1];
        frame
            .extended
            .intel_pilot()
            .decode_intel_payload(payload, &mut decode_ticks);
        for (total, ticks) in INTEL_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.weather_refresh
        && let Some(payload) = job.weather_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        frame.extended.weather_pilot().decode_weather_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in WEATHER_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.capitulation_refresh
        && let Some(payload) = job.capitulation_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 1];
        frame
            .extended
            .capitulation_pilot()
            .decode_capitulation_payload(shared.base, payload, &mut decode_ticks);
        for (total, ticks) in CAPITULATION_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    for (total, ticks) in DIVISION_PRESENTATION_DECODE_NS
        .iter()
        .zip(frame.extended.decode_division_presentation_inputs())
    {
        total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
    }
    // Pool invariant: this frame is owned by the worker, so it occupies no
    // ready slot and the push below always has room: the ready queue has one
    // slot for every reusable frame, including this worker-owned frame.
    while shared.ready.push(job.index).is_err() {
        std::thread::yield_now();
    }
    shared.completed[job.index].store(job.game_elapsed.max(1), Ordering::Release);
}

/// Detach drain (runtime only, after callback quiescence): stop the worker,
/// join it, then synchronously decode any remaining FIFO items in order and
/// hand them to the spool ready queue before the existing spool drain runs.
/// The spool thread is still consuming, so handoffs make progress.
pub fn detach_drain() -> Result<(), String> {
    STOP.store(true, Ordering::Release);
    let (worker, mut error) = match WORKER.lock() {
        Ok(mut slot) => (slot.take(), None),
        Err(poisoned) => (
            poisoned.into_inner().take(),
            Some("decode worker lock poisoned".to_string()),
        ),
    };
    if let Some(handle) = worker
        && handle.join().is_err()
    {
        error = Some("decode worker panicked; its in-flight frame may be incomplete".into());
    }
    WORKER_ID.store(0, Ordering::Release);
    let shared_ptr = sampling::SHARED.load(Ordering::Acquire);
    if shared_ptr.is_null() {
        STOP.store(false, Ordering::Release);
        return error.map_or(Ok(()), Err);
    }
    let shared = unsafe { &*shared_ptr };
    // The joined worker drops its memo. Drain owns one fresh entry and cannot
    // race or retain references into worker/ready-queue storage.
    let mut air_bases_memo = AirBasesMemo::default();
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        while let Some(job) = shared.decode_queue.pop() {
            process_owned_job(shared, job, &mut air_bases_memo);
        }
    }))
    .is_err()
    {
        error = Some("decode detach drain panicked; queued frames remain incomplete".into());
    }
    STOP.store(false, Ordering::Release);
    error.map_or(Ok(()), Err)
}

/// Additive calibration snapshot. Every counter is monotonic since attach.
pub fn calibration() -> Calibration {
    Calibration {
        jobs: JOBS.load(Ordering::Relaxed),
        refresh_jobs: REFRESH_JOBS.load(Ordering::Relaxed),
        full_refresh_jobs: FULL_REFRESH_JOBS.load(Ordering::Relaxed),
        drops: DROPS.load(Ordering::Relaxed),
        last_seq: LAST_SEQ.load(Ordering::Relaxed),
        last_hour: LAST_HOUR.load(Ordering::Relaxed) as i32,
        relations_copy_ns: RELATIONS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        relations_decode_ns: RELATIONS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        relations_raw_bytes: RELATIONS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        characters_copy_ns: CHARACTERS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        characters_decode_ns: CHARACTERS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        characters_raw_bytes: CHARACTERS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        projects_copy_ns: PROJECTS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        projects_decode_ns: PROJECTS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        projects_raw_bytes: PROJECTS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        lexical_copy_ns: LEXICAL_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        lexical_decode_ns: LEXICAL_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        lexical_raw_bytes: LEXICAL_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        divisions_copy_ns: DIVISIONS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        divisions_decode_ns: DIVISIONS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        division_presentation_decode_ns: DIVISION_PRESENTATION_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        divisions_raw_bytes: DIVISIONS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        focus_copy_ns: FOCUS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        focus_decode_ns: FOCUS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        focus_raw_bytes: FOCUS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_wing_copy_ns: AIR_WING_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_wing_decode_ns: AIR_WING_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_wing_raw_bytes: AIR_WING_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_stats_copy_ns: AIR_STATS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_stats_decode_ns: AIR_STATS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_stats_raw_bytes: AIR_STATS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        naval_copy_ns: NAVAL_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        naval_decode_ns: NAVAL_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        naval_raw_bytes: NAVAL_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        players_copy_ns: PLAYERS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        players_decode_ns: PLAYERS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        players_raw_bytes: PLAYERS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        session_copy_ns: SESSION_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        session_decode_ns: SESSION_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        session_raw_bytes: SESSION_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        production_rest_copy_ns: PRODUCTION_REST_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        production_rest_decode_ns: PRODUCTION_REST_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        production_rest_raw_bytes: PRODUCTION_REST_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        construction_rest_copy_ns: CONSTRUCTION_REST_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        construction_rest_decode_ns: CONSTRUCTION_REST_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        construction_rest_raw_bytes: CONSTRUCTION_REST_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        economy_rest_copy_ns: ECONOMY_REST_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        economy_rest_decode_ns: ECONOMY_REST_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        economy_rest_raw_bytes: ECONOMY_REST_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_bases_copy_ns: AIR_BASES_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_bases_decode_ns: AIR_BASES_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_bases_raw_bytes: AIR_BASES_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_history_copy_ns: AIR_HISTORY_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_history_decode_ns: AIR_HISTORY_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        air_history_raw_bytes: AIR_HISTORY_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        supremacy_copy_ns: SUPREMACY_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        supremacy_decode_ns: SUPREMACY_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        supremacy_raw_bytes: SUPREMACY_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        sunk_copy_ns: SUNK_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        sunk_decode_ns: SUNK_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        sunk_raw_bytes: SUNK_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        radar_copy_ns: RADAR_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        radar_decode_ns: RADAR_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        radar_raw_bytes: RADAR_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        intel_copy_ns: INTEL_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        intel_decode_ns: INTEL_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        intel_raw_bytes: INTEL_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        weather_copy_ns: WEATHER_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        weather_decode_ns: WEATHER_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        weather_raw_bytes: WEATHER_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        capitulation_copy_ns: CAPITULATION_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        capitulation_decode_ns: CAPITULATION_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        capitulation_raw_bytes: CAPITULATION_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        queue_high_water: QUEUE_HIGH_WATER.load(Ordering::Relaxed),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Calibration {
    pub jobs: u64,
    pub refresh_jobs: u64,
    pub full_refresh_jobs: u64,
    pub drops: u64,
    pub last_seq: u64,
    pub last_hour: i32,
    pub relations_copy_ns: [u64; 2],
    pub relations_decode_ns: [u64; 2],
    pub relations_raw_bytes: [u64; 2],
    pub characters_copy_ns: [u64; 6],
    pub characters_decode_ns: [u64; 6],
    pub characters_raw_bytes: [u64; 6],
    pub projects_copy_ns: [u64; 4],
    pub projects_decode_ns: [u64; 4],
    pub projects_raw_bytes: [u64; 4],
    pub lexical_copy_ns: [u64; 1],
    pub lexical_decode_ns: [u64; 1],
    pub lexical_raw_bytes: [u64; 1],
    pub divisions_copy_ns: [u64; 6],
    pub divisions_decode_ns: [u64; 6],
    pub division_presentation_decode_ns: [u64; 2],
    pub divisions_raw_bytes: [u64; 6],
    pub focus_copy_ns: [u64; 2],
    pub focus_decode_ns: [u64; 2],
    pub focus_raw_bytes: [u64; 2],
    pub air_wing_copy_ns: [u64; 2],
    pub air_wing_decode_ns: [u64; 2],
    pub air_wing_raw_bytes: [u64; 2],
    pub air_stats_copy_ns: [u64; 1],
    pub air_stats_decode_ns: [u64; 1],
    pub air_stats_raw_bytes: [u64; 1],
    pub naval_copy_ns: [u64; 9],
    pub naval_decode_ns: [u64; 9],
    pub naval_raw_bytes: [u64; 9],
    pub players_copy_ns: [u64; 2],
    pub players_decode_ns: [u64; 2],
    pub players_raw_bytes: [u64; 2],
    pub session_copy_ns: [u64; 3],
    pub session_decode_ns: [u64; 3],
    pub session_raw_bytes: [u64; 3],
    pub production_rest_copy_ns: [u64; 8],
    pub production_rest_decode_ns: [u64; 8],
    pub production_rest_raw_bytes: [u64; 8],
    pub construction_rest_copy_ns: [u64; 10],
    pub construction_rest_decode_ns: [u64; 10],
    pub construction_rest_raw_bytes: [u64; 10],
    pub economy_rest_copy_ns: [u64; crate::extended::economy::N],
    pub economy_rest_decode_ns: [u64; crate::extended::economy::N],
    pub economy_rest_raw_bytes: [u64; crate::extended::economy::N],
    pub air_bases_copy_ns: [u64; 3],
    pub air_bases_decode_ns: [u64; 3],
    pub air_bases_raw_bytes: [u64; 3],
    pub air_history_copy_ns: [u64; 2],
    pub air_history_decode_ns: [u64; 2],
    pub air_history_raw_bytes: [u64; 2],
    pub supremacy_copy_ns: [u64; 1],
    pub supremacy_decode_ns: [u64; 1],
    pub supremacy_raw_bytes: [u64; 1],
    pub sunk_copy_ns: [u64; 1],
    pub sunk_decode_ns: [u64; 1],
    pub sunk_raw_bytes: [u64; 1],
    pub radar_copy_ns: [u64; 1],
    pub radar_decode_ns: [u64; 1],
    pub radar_raw_bytes: [u64; 1],
    pub intel_copy_ns: [u64; 1],
    pub intel_decode_ns: [u64; 1],
    pub intel_raw_bytes: [u64; 1],
    pub weather_copy_ns: [u64; 2],
    pub weather_decode_ns: [u64; 2],
    pub weather_raw_bytes: [u64; 2],
    pub capitulation_copy_ns: [u64; 1],
    pub capitulation_decode_ns: [u64; 1],
    pub capitulation_raw_bytes: [u64; 1],
    pub queue_high_water: u64,
}

impl Calibration {
    pub fn json(&self) -> Value {
        json!({
            "scope": "decode worker totals since attach; monotonic counters except live capacity fields",
            "raw_growth": crate::raw::growth_diagnostics(),
            "raw_indexes": crate::raw::index_diagnostics(),
            "read_plans": crate::raw::read_plan_diagnostics(),
            "raw_memory": crate::raw_memory::diagnostics(),

            "air_bases_memo": {
                "hits": AIR_BASES_MEMO[0].load(Ordering::Relaxed),
                "misses": AIR_BASES_MEMO[1].load(Ordering::Relaxed),
                "ineligible": AIR_BASES_MEMO[2].load(Ordering::Relaxed),
                "comparisons": AIR_BASES_MEMO[3].load(Ordering::Relaxed),
                "installs": AIR_BASES_MEMO[4].load(Ordering::Relaxed),
                "compare_ns": AIR_BASES_MEMO[5].load(Ordering::Relaxed),
                "restore_ns": AIR_BASES_MEMO[6].load(Ordering::Relaxed),
                "install_ns": AIR_BASES_MEMO[7].load(Ordering::Relaxed),
                "memory": crate::extended::air::air_bases_memo_memory(),
            },
            "jobs": self.jobs,
            "refresh_jobs": self.refresh_jobs,
            "full_refresh_jobs": self.full_refresh_jobs,
            "drops": self.drops,
            "last_seq": self.last_seq,
            "last_hour": self.last_hour,
            "relations_copy_ns": self.relations_copy_ns.as_slice(),
            "relations_decode_ns": self.relations_decode_ns.as_slice(),
            "relations_raw_bytes": self.relations_raw_bytes.as_slice(),
            "relations_tables": ["RelationOpinions", "RelationTuples"],
            "characters_copy_ns": self.characters_copy_ns.as_slice(),
            "characters_decode_ns": self.characters_decode_ns.as_slice(),
            "characters_raw_bytes": self.characters_raw_bytes.as_slice(),
            "characters_tables": ["Characters", "CountryLeaderRoles", "AdvisorRoles", "UnitLeaders", "UnitLeaderTraits", "UnitLeaderTraitExperience"],
            "projects_copy_ns": self.projects_copy_ns.as_slice(),
            "projects_decode_ns": self.projects_decode_ns.as_slice(),
            "projects_raw_bytes": self.projects_raw_bytes.as_slice(),
            "projects_tables": ["ProjectPrograms", "Projects", "ProjectHistory", "ProjectBreakthroughs"],
            "lexical_copy_ns": self.lexical_copy_ns.as_slice(),
            "lexical_decode_ns": self.lexical_decode_ns.as_slice(),
            "lexical_raw_bytes": self.lexical_raw_bytes.as_slice(),
            "lexical_tables": ["LexicalTokens"],
            "divisions_copy_ns": self.divisions_copy_ns.as_slice(),
            "divisions_decode_ns": self.divisions_decode_ns.as_slice(),
            "division_presentation_decode_ns": self.division_presentation_decode_ns.as_slice(),
            "division_presentation_tables": ["DivisionMovement", "DivisionStatus"],
            "divisions_raw_bytes": self.divisions_raw_bytes.as_slice(),
            "divisions_tables": ["Divisions", "DivisionEquipment", "ArmyUpgradeGroups", "ArmyUpgradeRequests", "ArmyUpgradeDeliveries", "ArmyUpgradeProduced"],
            "focus_copy_ns": self.focus_copy_ns.as_slice(),
            "focus_decode_ns": self.focus_decode_ns.as_slice(),
            "focus_raw_bytes": self.focus_raw_bytes.as_slice(),
            "focus_tables": ["Focus", "CompletedFocuses"],
            "air_wing_copy_ns": self.air_wing_copy_ns.as_slice(),
            "air_wing_decode_ns": self.air_wing_decode_ns.as_slice(),
            "air_wing_raw_bytes": self.air_wing_raw_bytes.as_slice(),
            "air_wing_tables": ["WingEquipment", "WingCumulative"],
            "air_stats_copy_ns": self.air_stats_copy_ns.as_slice(),
            "air_stats_decode_ns": self.air_stats_decode_ns.as_slice(),
            "air_stats_raw_bytes": self.air_stats_raw_bytes.as_slice(),
            "air_stats_tables": ["AirStatistics"],
            "naval_copy_ns": self.naval_copy_ns.as_slice(),
            "naval_decode_ns": self.naval_decode_ns.as_slice(),
            "naval_raw_bytes": self.naval_raw_bytes.as_slice(),
            "naval_tables": ["LandTheatres", "LandTheatreGroups", "LandTheatreMembers", "AirTheatres", "AirTheatreGroups", "AirTheatreMembers", "OrdersGroups", "OrdersGroupMembers", "ArmyGroupMembers"],
            "players_copy_ns": self.players_copy_ns.as_slice(),
            "players_decode_ns": self.players_decode_ns.as_slice(),
            "players_raw_bytes": self.players_raw_bytes.as_slice(),
            "players_tables": ["Players", "HumanControllers"],
            "session_copy_ns": self.session_copy_ns.as_slice(),
            "session_decode_ns": self.session_decode_ns.as_slice(),
            "session_raw_bytes": self.session_raw_bytes.as_slice(),
            "session_tables": ["WorldTension", "ThreatSources", "ThreatCountryAggregates"],
            "production_rest_copy_ns": self.production_rest_copy_ns.as_slice(),
            "production_rest_decode_ns": self.production_rest_decode_ns.as_slice(),
            "production_rest_raw_bytes": self.production_rest_raw_bytes.as_slice(),
            "production_rest_tables": ["ProductionLines", "Stockpiles", "ProductionLicenses", "DeploymentConveyors", "DeploymentLines", "DeploymentManpower", "DeploymentEquipment", "ProductionLicenseRecords"],
            "construction_rest_copy_ns": self.construction_rest_copy_ns.as_slice(),
            "construction_rest_decode_ns": self.construction_rest_decode_ns.as_slice(),
            "construction_rest_raw_bytes": self.construction_rest_raw_bytes.as_slice(),
            "construction_rest_tables": ["StateBaseResources", "BuildingConstruction", "RailwayConstruction", "OccupationSettings", "StateOccupationLaws", "StateGarrisons", "StateAssignedManpower", "CountryOccupationReserveManpower", "StateResistance", "CountryCollaboration"],
            "economy_rest_copy_ns": self.economy_rest_copy_ns.as_slice(),
            "economy_rest_decode_ns": self.economy_rest_decode_ns.as_slice(),
            "economy_rest_raw_bytes": self.economy_rest_raw_bytes.as_slice(),
            "economy_rest_tables": ["CountryFuel", "CountryExperience", "ProvinceSupply", "HubMotorization", "DisruptedSupply", "ForeignHomebases", "ResourceDefinitions", "CountryResources", "ResourceExchanges", "PurchaseContracts", "CountryFuelConsumers"],
            "air_bases_copy_ns": self.air_bases_copy_ns.as_slice(),
            "air_bases_decode_ns": self.air_bases_decode_ns.as_slice(),
            "air_bases_raw_bytes": self.air_bases_raw_bytes.as_slice(),
            "air_bases_tables": ["AirBases", "AirBaseRegistrations", "AirAces"],
            "air_history_copy_ns": self.air_history_copy_ns.as_slice(),
            "air_history_decode_ns": self.air_history_decode_ns.as_slice(),
            "air_history_raw_bytes": self.air_history_raw_bytes.as_slice(),
            "air_history_tables": ["AirRegionHistory", "AirRegionDamage"],
            "supremacy_copy_ns": self.supremacy_copy_ns.as_slice(),
            "supremacy_decode_ns": self.supremacy_decode_ns.as_slice(),
            "supremacy_raw_bytes": self.supremacy_raw_bytes.as_slice(),
            "supremacy_tables": ["NavalSupremacy"],
            "sunk_copy_ns": self.sunk_copy_ns.as_slice(),
            "sunk_decode_ns": self.sunk_decode_ns.as_slice(),
            "sunk_raw_bytes": self.sunk_raw_bytes.as_slice(),
            "sunk_tables": ["SunkShips"],
            "radar_copy_ns": self.radar_copy_ns.as_slice(),
            "radar_decode_ns": self.radar_decode_ns.as_slice(),
            "radar_raw_bytes": self.radar_raw_bytes.as_slice(),
            "radar_tables": ["RadarCoverage"],
            "intel_copy_ns": self.intel_copy_ns.as_slice(),
            "intel_decode_ns": self.intel_decode_ns.as_slice(),
            "intel_raw_bytes": self.intel_raw_bytes.as_slice(),
            "intel_tables": ["UnitIntel"],
            "weather_copy_ns": self.weather_copy_ns.as_slice(),
            "weather_decode_ns": self.weather_decode_ns.as_slice(),
            "weather_raw_bytes": self.weather_raw_bytes.as_slice(),
            "weather_tables": ["WeatherRegions", "WeatherProvinces"],
            "capitulation_copy_ns": self.capitulation_copy_ns.as_slice(),
            "capitulation_decode_ns": self.capitulation_decode_ns.as_slice(),
            "capitulation_raw_bytes": self.capitulation_raw_bytes.as_slice(),
            "capitulation_tables": ["Capitulation"],
            "queue_high_water": self.queue_high_water,
            "queue_capacity": BUFFERS,
        })
    }
}

/// Additive status merge for the runtime statuswriters. Existing keys survive.
pub fn calibration_json() -> Value {
    let mut status = calibration().json();
    status["characters_acquisition"] = crate::extended::characters::acquisition_diagnostics();
    status
}

pub(crate) fn add_relations_copy_ns(table: usize, ns: u64) {
    RELATIONS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_relations_raw_bytes(table: usize, bytes: u64) {
    RELATIONS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_characters_copy_ns(table: usize, ns: u64) {
    CHARACTERS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_characters_raw_bytes(table: usize, bytes: u64) {
    CHARACTERS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_projects_copy_ns(table: usize, ns: u64) {
    PROJECTS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_projects_raw_bytes(table: usize, bytes: u64) {
    PROJECTS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_lexical_copy_ns(table: usize, ns: u64) {
    LEXICAL_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_lexical_raw_bytes(table: usize, bytes: u64) {
    LEXICAL_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_divisions_copy_ns(table: usize, ns: u64) {
    DIVISIONS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_divisions_raw_bytes(table: usize, bytes: u64) {
    DIVISIONS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_focus_copy_ns(table: usize, ns: u64) {
    FOCUS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_focus_raw_bytes(table: usize, bytes: u64) {
    FOCUS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_air_wing_copy_ns(table: usize, ns: u64) {
    AIR_WING_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_air_wing_raw_bytes(table: usize, bytes: u64) {
    AIR_WING_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_air_stats_copy_ns(table: usize, ns: u64) {
    AIR_STATS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_air_stats_raw_bytes(table: usize, bytes: u64) {
    AIR_STATS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_naval_copy_ns(table: usize, ns: u64) {
    NAVAL_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_naval_raw_bytes(table: usize, bytes: u64) {
    NAVAL_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_players_copy_ns(table: usize, ns: u64) {
    PLAYERS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_players_raw_bytes(table: usize, bytes: u64) {
    PLAYERS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_session_copy_ns(table: usize, ns: u64) {
    SESSION_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_session_raw_bytes(table: usize, bytes: u64) {
    SESSION_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_production_rest_copy_ns(table: usize, ns: u64) {
    PRODUCTION_REST_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_production_rest_raw_bytes(table: usize, bytes: u64) {
    PRODUCTION_REST_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_construction_rest_copy_ns(table: usize, ns: u64) {
    CONSTRUCTION_REST_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_construction_rest_raw_bytes(table: usize, bytes: u64) {
    CONSTRUCTION_REST_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_economy_rest_copy_ns(table: usize, ns: u64) {
    ECONOMY_REST_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_economy_rest_raw_bytes(table: usize, bytes: u64) {
    ECONOMY_REST_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_air_bases_copy_ns(table: usize, ns: u64) {
    AIR_BASES_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_air_bases_raw_bytes(table: usize, bytes: u64) {
    AIR_BASES_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_air_history_copy_ns(table: usize, ns: u64) {
    AIR_HISTORY_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_air_history_raw_bytes(table: usize, bytes: u64) {
    AIR_HISTORY_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_supremacy_copy_ns(table: usize, ns: u64) {
    SUPREMACY_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_supremacy_raw_bytes(table: usize, bytes: u64) {
    SUPREMACY_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_sunk_copy_ns(table: usize, ns: u64) {
    SUNK_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_sunk_raw_bytes(table: usize, bytes: u64) {
    SUNK_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_radar_copy_ns(table: usize, ns: u64) {
    RADAR_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_radar_raw_bytes(table: usize, bytes: u64) {
    RADAR_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_intel_copy_ns(table: usize, ns: u64) {
    INTEL_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_intel_raw_bytes(table: usize, bytes: u64) {
    INTEL_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_weather_copy_ns(table: usize, ns: u64) {
    WEATHER_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_weather_raw_bytes(table: usize, bytes: u64) {
    WEATHER_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_capitulation_copy_ns(table: usize, ns: u64) {
    CAPITULATION_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_capitulation_raw_bytes(table: usize, bytes: u64) {
    CAPITULATION_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

/// Bounded raw FIFO type shared with sampling's pool (indices only).
pub type DecodeQueue = ArrayQueue<DecodeJob>;

pub fn new_queue() -> DecodeQueue {
    ArrayQueue::new(BUFFERS)
}

#[cfg(test)]
pub(crate) fn reset_calibration_for_test() {
    prepare_retirement();
    reclaim_payloads();
    // Production prepares bounded payload storage before installing the hook.
    // Tests exercise that lifecycle rather than relying on cold tick allocation.
    std::mem::forget(prepare_payload_pools());
    for cell in RELATIONS_COPY_NS
        .iter()
        .chain(RELATIONS_DECODE_NS.iter())
        .chain(RELATIONS_RAW_BYTES.iter())
        .chain(CHARACTERS_COPY_NS.iter())
        .chain(CHARACTERS_DECODE_NS.iter())
        .chain(CHARACTERS_RAW_BYTES.iter())
        .chain(PROJECTS_COPY_NS.iter())
        .chain(PROJECTS_DECODE_NS.iter())
        .chain(PROJECTS_RAW_BYTES.iter())
        .chain(LEXICAL_COPY_NS.iter())
        .chain(LEXICAL_DECODE_NS.iter())
        .chain(LEXICAL_RAW_BYTES.iter())
        .chain(DIVISIONS_COPY_NS.iter())
        .chain(DIVISIONS_DECODE_NS.iter())
        .chain(DIVISION_PRESENTATION_DECODE_NS.iter())
        .chain(DIVISIONS_RAW_BYTES.iter())
        .chain(FOCUS_COPY_NS.iter())
        .chain(FOCUS_DECODE_NS.iter())
        .chain(FOCUS_RAW_BYTES.iter())
        .chain(AIR_WING_COPY_NS.iter())
        .chain(AIR_WING_DECODE_NS.iter())
        .chain(AIR_WING_RAW_BYTES.iter())
        .chain(AIR_STATS_COPY_NS.iter())
        .chain(AIR_STATS_DECODE_NS.iter())
        .chain(AIR_STATS_RAW_BYTES.iter())
        .chain(NAVAL_COPY_NS.iter())
        .chain(NAVAL_DECODE_NS.iter())
        .chain(NAVAL_RAW_BYTES.iter())
        .chain(PLAYERS_COPY_NS.iter())
        .chain(PLAYERS_DECODE_NS.iter())
        .chain(PLAYERS_RAW_BYTES.iter())
        .chain(SESSION_COPY_NS.iter())
        .chain(SESSION_DECODE_NS.iter())
        .chain(SESSION_RAW_BYTES.iter())
        .chain(PRODUCTION_REST_COPY_NS.iter())
        .chain(PRODUCTION_REST_DECODE_NS.iter())
        .chain(PRODUCTION_REST_RAW_BYTES.iter())
        .chain(CONSTRUCTION_REST_COPY_NS.iter())
        .chain(CONSTRUCTION_REST_DECODE_NS.iter())
        .chain(CONSTRUCTION_REST_RAW_BYTES.iter())
        .chain(ECONOMY_REST_COPY_NS.iter())
        .chain(ECONOMY_REST_DECODE_NS.iter())
        .chain(ECONOMY_REST_RAW_BYTES.iter())
        .chain(AIR_BASES_COPY_NS.iter())
        .chain(AIR_BASES_DECODE_NS.iter())
        .chain(AIR_BASES_MEMO.iter())
        .chain(AIR_BASES_RAW_BYTES.iter())
        .chain(AIR_HISTORY_COPY_NS.iter())
        .chain(AIR_HISTORY_DECODE_NS.iter())
        .chain(AIR_HISTORY_RAW_BYTES.iter())
        .chain(SUPREMACY_COPY_NS.iter())
        .chain(SUPREMACY_DECODE_NS.iter())
        .chain(SUPREMACY_RAW_BYTES.iter())
        .chain(SUNK_COPY_NS.iter())
        .chain(SUNK_DECODE_NS.iter())
        .chain(SUNK_RAW_BYTES.iter())
        .chain(RADAR_COPY_NS.iter())
        .chain(RADAR_DECODE_NS.iter())
        .chain(RADAR_RAW_BYTES.iter())
        .chain(INTEL_COPY_NS.iter())
        .chain(INTEL_DECODE_NS.iter())
        .chain(INTEL_RAW_BYTES.iter())
        .chain(WEATHER_COPY_NS.iter())
        .chain(WEATHER_DECODE_NS.iter())
        .chain(WEATHER_RAW_BYTES.iter())
        .chain(CAPITULATION_COPY_NS.iter())
        .chain(CAPITULATION_DECODE_NS.iter())
        .chain(CAPITULATION_RAW_BYTES.iter())
    {
        cell.store(0, Ordering::Relaxed);
    }
    QUEUE_HIGH_WATER.store(0, Ordering::Relaxed);
    JOBS.store(0, Ordering::Relaxed);
    REFRESH_JOBS.store(0, Ordering::Relaxed);
    FULL_REFRESH_JOBS.store(0, Ordering::Relaxed);
    DROPS.store(0, Ordering::Relaxed);
    LAST_SEQ.store(0, Ordering::Relaxed);
    LAST_HOUR.store(0, Ordering::Relaxed);
    SEQ.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// Calibration globals are shared across tests in this binary; the tests
    /// below that reset or advance them hold this lock so assertions stay
    /// deterministic under parallel execution.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn advisor_payload_storage_survives_pool_preparation_and_recycling() {
        let _guard = TEST_LOCK.lock().unwrap();
        let saved = std::mem::take(&mut CHARACTERS_JOB_POOL.lock().unwrap()[2]);
        release_characters_job(CharactersJob::new_part(2));
        for _ in 0..2 {
            let job = acquire_characters_job(2);
            assert_eq!(job.part, 2);
            assert!(
                job.has_storage(),
                "typed advisor storage must not be mistaken for an empty raw fallback"
            );
            release_characters_job(job);
        }
        CHARACTERS_JOB_POOL.lock().unwrap()[2] = saved;
    }

    #[test]
    fn retirement_capacity_covers_every_character_part_and_owned_frame() {
        let _guard = TEST_LOCK.lock().unwrap();
        prepare_retirement();
        reclaim_payloads();
        let retired = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
        // Full refreshes can recur on loads or discontinuities. If staging
        // overflows or has no reusable storage, every owned frame can retire
        // one object for each handoff before the decode worker makes progress.
        // Empty payloads exercise the real retirement path without large arenas.
        for _ in 0..BUFFERS {
            recycle_relations_job(RelationsJob::default());
            for part in 0..CHARACTER_PARTS {
                let mut job = CharactersJob::default();
                job.part = part;
                recycle_characters_job(job);
            }
            recycle_projects_job(ProjectsJob::default());
            recycle_lexical_job(LexicalJob::default());
            recycle_divisions_job(DivisionsJob::default());
            recycle_focus_job(FocusJob::default());
            recycle_air_wing_job(AirWingJob::default());
            recycle_air_stats_job(AirStatsJob::default());
            recycle_naval_job(NavalJob::default());
            recycle_players_job(PlayersJob::default());
            recycle_session_job(SessionJob::default());
            recycle_production_rest_job(ProductionRestJob::default());
            recycle_construction_rest_job(ConstructionRestJob::default());
            recycle_economy_rest_job(EconomyRestJob::default());
            recycle_air_bases_job(AirBasesJob::default());
            recycle_air_history_job(AirHistoryJob::default());
            recycle_supremacy_job(SupremacyJob::default());
            recycle_sunk_job(SunkJob::default());
            recycle_radar_job(RadarJob::default());
            recycle_intel_job(IntelJob::default());
            recycle_weather_job(WeatherJob::default());
            recycle_capitulation_job(CapitulationJob::default());
        }
        assert_eq!(retired.len(), RETIREMENT_CAPACITY);
        assert!(retired.is_full());
        reclaim_payloads();
        assert!(retired.is_empty());
    }
    #[test]
    fn exhausted_or_contended_payload_pools_decline_without_allocating() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let mut pool = AIR_STATS_JOB_POOL.lock().unwrap();
        let contended = acquire_air_stats_job();
        assert_eq!(contended.log.spans.capacity(), 0);
        assert_eq!(contended.log.bytes.capacity(), 0);
        let owned = std::mem::take(&mut *pool);
        drop(pool);
        let exhausted = acquire_air_stats_job();
        assert_eq!(exhausted.log.spans.capacity(), 0);
        assert_eq!(exhausted.log.bytes.capacity(), 0);
        assert!(AIR_STATS_JOB_POOL.lock().unwrap().is_empty());
        *AIR_STATS_JOB_POOL.lock().unwrap() = owned;
    }

    #[test]
    fn worker_returns_every_owned_payload_to_its_reuse_pool() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let shared = Shared::new(0, 0, 1);
        let index = shared.free.pop().expect("pooled frame");
        RELATIONS_JOB_POOL.lock().unwrap().clear();
        CHARACTERS_JOB_POOL
            .lock()
            .unwrap()
            .iter_mut()
            .for_each(Vec::clear);
        PROJECTS_JOB_POOL.lock().unwrap().clear();
        LEXICAL_JOB_POOL.lock().unwrap().clear();
        DIVISIONS_JOB_POOL.lock().unwrap().clear();
        FOCUS_JOB_POOL.lock().unwrap().clear();
        AIR_WING_JOB_POOL.lock().unwrap().clear();
        AIR_STATS_JOB_POOL.lock().unwrap().clear();
        NAVAL_JOB_POOL.lock().unwrap().clear();
        PLAYERS_JOB_POOL.lock().unwrap().clear();
        SESSION_JOB_POOL.lock().unwrap().clear();
        PRODUCTION_REST_JOB_POOL.lock().unwrap().clear();
        CONSTRUCTION_REST_JOB_POOL.lock().unwrap().clear();
        ECONOMY_REST_JOB_POOL.lock().unwrap().clear();
        AIR_BASES_JOB_POOL.lock().unwrap().clear();
        AIR_HISTORY_JOB_POOL.lock().unwrap().clear();
        SUPREMACY_JOB_POOL.lock().unwrap().clear();
        SUNK_JOB_POOL.lock().unwrap().clear();
        RADAR_JOB_POOL.lock().unwrap().clear();
        INTEL_JOB_POOL.lock().unwrap().clear();
        WEATHER_JOB_POOL.lock().unwrap().clear();
        CAPITULATION_JOB_POOL.lock().unwrap().clear();
        let air = AirStatsJob::new();
        let air_allocation = air.log.bytes.as_ptr();
        let job = DecodeJob {
            index,
            game_elapsed: 17,
            relations_payload: Some(RelationsJob::new()),
            characters_payload: [Some(CharactersJob::new()), None, None, None],
            projects_payload: Some(ProjectsJob::new()),
            lexical_payload: Some(LexicalJob::new()),
            divisions_payload: Some(DivisionsJob::new()),
            focus_payload: Some(FocusJob::new()),
            air_wing_payload: Some(AirWingJob::new()),
            air_stats_payload: Some(air),
            naval_payload: Some(NavalJob::new()),
            players_payload: Some(PlayersJob::new()),
            session_payload: Some(SessionJob::new()),
            production_rest_payload: Some(ProductionRestJob::new()),
            construction_rest_payload: Some(ConstructionRestJob::new()),
            economy_rest_payload: Some(EconomyRestJob::new()),
            air_bases_payload: Some(AirBasesJob::new()),
            air_history_payload: Some(AirHistoryJob::new()),
            supremacy_payload: Some(SupremacyJob::new()),
            sunk_payload: Some(SunkJob::new()),
            radar_payload: Some(RadarJob::new()),
            intel_payload: Some(IntelJob::new()),
            weather_payload: Some(WeatherJob::new()),
            capitulation_payload: Some(CapitulationJob::new()),
            ..Default::default()
        };
        // A callback can retire a refusal after an idle drain and then enqueue
        // this frame. Its retirement must be consumed before the lease returns.
        recycle_focus_job(FocusJob::default());
        let retirement = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
        assert_eq!(retirement.len(), 1);
        process_owned_job(&shared, job, &mut AirBasesMemo::default());
        assert!(retirement.is_empty());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 17);
        assert_eq!(
            RELATIONS_JOB_POOL.lock().unwrap().len(),
            1,
            "RELATIONS_JOB_POOL"
        );
        assert_eq!(
            CHARACTERS_JOB_POOL.lock().unwrap()[0].len(),
            1,
            "CHARACTERS_JOB_POOL"
        );
        assert_eq!(
            PROJECTS_JOB_POOL.lock().unwrap().len(),
            1,
            "PROJECTS_JOB_POOL"
        );
        assert_eq!(
            LEXICAL_JOB_POOL.lock().unwrap().len(),
            1,
            "LEXICAL_JOB_POOL"
        );
        assert_eq!(
            DIVISIONS_JOB_POOL.lock().unwrap().len(),
            1,
            "DIVISIONS_JOB_POOL"
        );
        assert_eq!(FOCUS_JOB_POOL.lock().unwrap().len(), 1, "FOCUS_JOB_POOL");
        assert_eq!(
            AIR_WING_JOB_POOL.lock().unwrap().len(),
            1,
            "AIR_WING_JOB_POOL"
        );
        assert_eq!(
            AIR_STATS_JOB_POOL.lock().unwrap().len(),
            1,
            "AIR_STATS_JOB_POOL"
        );
        assert_eq!(NAVAL_JOB_POOL.lock().unwrap().len(), 1, "NAVAL_JOB_POOL");
        assert_eq!(
            PLAYERS_JOB_POOL.lock().unwrap().len(),
            1,
            "PLAYERS_JOB_POOL"
        );
        assert_eq!(
            SESSION_JOB_POOL.lock().unwrap().len(),
            1,
            "SESSION_JOB_POOL"
        );
        assert_eq!(
            PRODUCTION_REST_JOB_POOL.lock().unwrap().len(),
            1,
            "PRODUCTION_REST_JOB_POOL"
        );
        assert_eq!(
            CONSTRUCTION_REST_JOB_POOL.lock().unwrap().len(),
            1,
            "CONSTRUCTION_REST_JOB_POOL"
        );
        assert_eq!(
            ECONOMY_REST_JOB_POOL.lock().unwrap().len(),
            1,
            "ECONOMY_REST_JOB_POOL"
        );
        assert_eq!(
            AIR_BASES_JOB_POOL.lock().unwrap().len(),
            1,
            "AIR_BASES_JOB_POOL"
        );
        assert_eq!(
            AIR_HISTORY_JOB_POOL.lock().unwrap().len(),
            1,
            "AIR_HISTORY_JOB_POOL"
        );
        assert_eq!(
            SUPREMACY_JOB_POOL.lock().unwrap().len(),
            1,
            "SUPREMACY_JOB_POOL"
        );
        assert_eq!(SUNK_JOB_POOL.lock().unwrap().len(), 1, "SUNK_JOB_POOL");
        assert_eq!(RADAR_JOB_POOL.lock().unwrap().len(), 1, "RADAR_JOB_POOL");
        assert_eq!(INTEL_JOB_POOL.lock().unwrap().len(), 1, "INTEL_JOB_POOL");
        assert_eq!(
            WEATHER_JOB_POOL.lock().unwrap().len(),
            1,
            "WEATHER_JOB_POOL"
        );
        assert_eq!(
            CAPITULATION_JOB_POOL.lock().unwrap().len(),
            1,
            "CAPITULATION_JOB_POOL"
        );
        let air = acquire_air_stats_job();
        assert_eq!(air.log.bytes.as_ptr(), air_allocation);
        recycle_air_stats_job(air);
    }

    #[test]
    fn overflow_growth_is_deferred_and_keeps_its_flag_until_reclaimed() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        AIR_STATS_JOB_POOL.lock().unwrap().clear();
        let mut job = AirStatsJob::new();
        job.log = crate::raw::RawLog::new(1, 1);
        let bytes = [0u8; 2];
        let memory = engine::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert!(job.log.take(&memory, 0x1000, 1, 1).is_some());
        assert!(job.log.take(&memory, 0x1001, 1, 1).is_none());
        recycle_air_stats_job(job);
        // The game-side retirement neither grows nor publishes a reusable job.
        assert!(AIR_STATS_JOB_POOL.lock().unwrap().is_empty());
        reclaim_payloads();
        let job = acquire_air_stats_job();
        assert!(job.log.spans.capacity() >= 2);
        assert!(job.log.bytes.capacity() >= 2);
        assert!(!job.log.overflowed);
        // Do not leave the intentionally tiny fixture in another test's pool.
        drop(job);
    }

    #[test]
    fn fifo_absent_daily_payloads_preserve_skipped_outcomes_and_idle_pools() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let daily_pool_lengths = || {
            [
                PROJECTS_JOB_POOL.lock().unwrap().len(),
                FOCUS_JOB_POOL.lock().unwrap().len(),
                AIR_STATS_JOB_POOL.lock().unwrap().len(),
                SESSION_JOB_POOL.lock().unwrap().len(),
                PRODUCTION_REST_JOB_POOL.lock().unwrap().len(),
                CONSTRUCTION_REST_JOB_POOL.lock().unwrap().len(),
                AIR_HISTORY_JOB_POOL.lock().unwrap().len(),
                SUPREMACY_JOB_POOL.lock().unwrap().len(),
                SUNK_JOB_POOL.lock().unwrap().len(),
                RADAR_JOB_POOL.lock().unwrap().len(),
            ]
        };
        let idle_pools = daily_pool_lengths();
        assert!(idle_pools.iter().all(|len| *len != 0));
        let shared = Shared::new(0, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame
            .extended
            .air_stats_pilot()
            .fail_air_stats_capture(replay::cadence::SKIPPED_REASON);
        frame
            .extended
            .focus_pilot()
            .fail_focus_capture(false, replay::cadence::SKIPPED_REASON);
        let focus_before = frame.extended.focus_pilot().outcomes();
        let air_before = frame.extended.air_stats_pilot().outcomes();
        let retired = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
        assert!(retired.is_empty());
        for hour in [4, 5, 19] {
            let mut job = bare_job(index, next_seq(), hour, 29);
            // Actual admission remains authoritative even when the envelope
            // alone would request a daily family or every table.
            job.force_full = true;
            assert!(enqueue(&shared, job).is_none());
            process_owned_job(
                &shared,
                shared.decode_queue.pop().expect("queued skipped frame"),
                &mut AirBasesMemo::default(),
            );
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.ready.pop(), None);
            assert_eq!(shared.completed[index].load(Ordering::Acquire), 29);
            assert!(retired.is_empty());
            assert_eq!(daily_pool_lengths(), idle_pools);
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            assert_eq!(frame.extended.focus_pilot().outcomes(), focus_before);
            assert_eq!(frame.extended.air_stats_pilot().outcomes(), air_before);
        }
        assert_eq!(calibration().refresh_jobs, 0);
    }

    #[test]
    fn lazy_world_request_acquires_only_scheduled_parts_and_retires_them_once() {
        use crate::read_accounting;
        use std::sync::Arc;

        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let pool_lengths = || {
            [
                PRODUCTION_REST_JOB_POOL.lock().unwrap().len(),
                CONSTRUCTION_REST_JOB_POOL.lock().unwrap().len(),
                FOCUS_JOB_POOL.lock().unwrap().len(),
                ECONOMY_REST_JOB_POOL.lock().unwrap().len(),
            ]
        };
        let idle = pool_lengths();
        assert!(idle.iter().all(|&len| len > 0));
        let retired = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(16));
        let shared = Shared::new(0, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        for (hour, force_full, parts, economy) in [
            (0, false, [false, false, false], [false, false]),
            (3, false, [false, true, false], [false, false]),
            (8, false, [true, false, false], [false, false]),
            (19, false, [false, false, true], [false, false]),
            (-5, false, [false, false, true], [false, false]),
            (27, false, [false, true, false], [false, false]),
            (32, false, [true, false, false], [false, false]),
            (10, false, [false, false, false], [true, false]),
            (17, false, [false, false, false], [false, true]),
            (0, true, [true, true, true], [true, true]),
        ] {
            assert!(retired.is_empty());
            let reads = read_accounting::Capture::new();
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            let focus = frame
                .extended
                .focus_pilot()
                .focus_copy_context(0, 0, parts[2]);
            let request = sampling::acquire_world_request(
                &countries,
                0,
                0,
                focus,
                frame.extended.economy_rest_pilot(),
                hour,
                force_full,
            );
            assert_eq!(
                [
                    request.production.is_some(),
                    request.construction.is_some(),
                    request.focus.is_some(),
                ],
                parts,
                "hour {hour}, force_full {force_full}"
            );
            assert_eq!(
                [
                    request.economy.context.resources_active,
                    request.economy.context.other_active,
                ],
                economy
            );
            let owned = [parts[0], parts[1], parts[2], true];
            let after_acquire: [usize; 4] =
                std::array::from_fn(|slot| idle[slot] - usize::from(owned[slot]));
            assert_eq!(pool_lengths(), after_acquire);
            assert!(retired.is_empty(), "absent parts must not retire a payload");
            assert_eq!(Arc::strong_count(&countries), 2);
            if let Some(part) = request.production {
                recycle_production_rest_job(part.payload);
            }
            if let Some(part) = request.construction {
                recycle_construction_rest_job(part.payload);
            }
            if let Some(part) = request.focus {
                assert!(part.context.active);
                recycle_focus_job(part.payload);
            }
            recycle_economy_rest_job(request.economy.payload);
            drop(request.countries);
            assert_eq!(Arc::strong_count(&countries), 1);
            assert_eq!(pool_lengths(), after_acquire);
            assert_eq!(
                retired.len(),
                owned.into_iter().filter(|&active| active).count()
            );
            assert!(enqueue(&shared, bare_job(index, next_seq(), hour, 31)).is_none());
            process_owned_job(
                &shared,
                shared
                    .decode_queue
                    .pop()
                    .expect("queued retirement boundary"),
                &mut AirBasesMemo::default(),
            );
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.ready.pop(), None);
            assert_eq!(shared.completed[index].load(Ordering::Acquire), 31);
            assert!(shared.decode_queue.is_empty());
            assert!(retired.is_empty());
            assert_eq!(pool_lengths(), idle);
            assert_eq!(reads.finish(), read_accounting::ZERO);
        }
    }

    #[test]
    fn missing_declared_payloads_fail_only_the_frozen_active_tables() {
        use engine::walk::Outcome;

        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let shared = Shared::new(0, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let characters = [true, false, false, true, false, false];
        let relations = [false, true];
        frame
            .extended
            .characters_pilot()
            .set_character_activity(characters);
        frame
            .extended
            .relations_pilot()
            .set_relation_activity(relations);
        frame.extended.air_bases_pilot().prepare_hour(0, false);
        frame.extended.naval_pilot().prepare_hour(0, false);
        frame.extended.radar_pilot().prepare_hour(0, false);
        let mut job = bare_job(index, next_seq(), 21, 37);
        // Neither this hour nor force_full may replace callback-frozen activity.
        job.force_full = true;
        job.characters_refresh = [true, false, false, false];
        job.relations_refresh = true;
        job.air_wing_refresh = true;
        job.air_bases_refresh = true;
        job.naval_refresh = true;
        job.focus_refresh = true;
        process_owned_job(&shared, job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.ready.pop(), None);
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 37);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let missing = Outcome::Unavailable {
            reason: MISSING_PAYLOAD_REASON,
        };
        let skipped = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        for (active, outcome) in characters
            .into_iter()
            .zip(frame.extended.characters_pilot().outcomes())
        {
            assert_eq!(outcome, if active { missing } else { skipped });
        }
        assert_eq!(
            frame.extended.relations_pilot().outcomes()[13..15],
            [skipped, missing]
        );
        let air = frame.extended.air_bases_pilot().outcomes();
        assert_eq!(
            [air[0], air[1], air[2], air[5]],
            [missing, missing, skipped, skipped]
        );
        assert_eq!(frame.extended.radar_pilot().outcomes()[5], missing);
        let naval = frame.extended.naval_pilot().outcomes();
        // LandTheatres headers are hourly; the theatre details are daily.
        assert_eq!(naval[4], missing);
        assert_eq!(naval[5..10], [skipped; 5]);
        assert_eq!(naval[14..17], [missing; 3]);
        assert_eq!(frame.extended.focus_pilot().outcomes()[..2], [missing; 2]);
        assert_eq!(calibration().refresh_jobs, 0);
    }

    #[test]
    fn relations_gate_admits_every_hour_for_its_owner_slice() {
        for hour in -48..48i32 {
            assert!(relations_refresh_hour(hour, false), "hour {hour}");
            for table in RELATIONS {
                assert!(replay::cadence::is_sliced(table));
                assert!(replay::cadence::should_walk(table, hour, false));
            }
            assert!(relations_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn characters_gate_keeps_the_roster_hourly_and_roles_on_their_daily_slot() {
        for hour in -48..48i32 {
            assert!(characters_refresh_hour(hour, false), "hour {hour}");
            for table in CHARACTERS {
                let expected = matches!(table, TableId::Characters | TableId::UnitLeaders)
                    || hour.rem_euclid(24) == 21;
                assert_eq!(
                    replay::cadence::should_walk(table, hour, false),
                    expected,
                    "{table:?} at hour {hour}"
                );
                assert!(replay::cadence::should_walk(table, hour, true));
            }
            assert!(characters_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn projects_gate_matches_their_daily_slot_and_force_full() {
        let slot = replay::cadence::refresh_slot(PROJECTS[0]).unwrap() as i32;
        for hour in -48..48i32 {
            let refresh = hour.rem_euclid(24) == slot;
            assert_eq!(projects_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(projects_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn lexical_gate_is_static_and_force_full() {
        for hour in -48..48i32 {
            assert!(!lexical_refresh_hour(hour, false), "hour {hour}");
            assert!(lexical_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn fifo_hands_off_every_frame_in_production_order_without_reorder() {
        let shared = Shared::new(0, 0, 1);
        let mut indices = Vec::new();
        for _ in 0..4 {
            indices.push(shared.free.pop().expect("pooled buffer"));
        }
        // Enqueue out-of-seq labels; handoff order must still be FIFO.
        for (order, index) in indices.iter().enumerate() {
            shared.completed[*index].store(0, Ordering::Relaxed);
            let job = DecodeJob {
                index: *index,
                seq: 100 + order as u64,
                hour: 200 + order as i32,
                relations_refresh: false,
                characters_refresh: [false; CHARACTER_PARTS],
                projects_refresh: false,
                lexical_refresh: false,
                divisions_refresh: false,
                focus_refresh: false,
                air_wing_refresh: false,
                air_stats_refresh: false,
                naval_refresh: false,
                players_refresh: false,
                session_refresh: false,
                production_rest_refresh: false,
                construction_rest_refresh: false,
                economy_rest_refresh: false,
                air_bases_refresh: false,
                air_history_refresh: false,
                supremacy_refresh: false,
                sunk_refresh: false,
                radar_refresh: false,
                intel_refresh: false,
                weather_refresh: false,
                capitulation_refresh: false,
                force_full: false,
                game_elapsed: 7,
                relations_payload: None,
                characters_payload: [const { None }; CHARACTER_PARTS],
                projects_payload: None,
                lexical_payload: None,
                divisions_payload: None,
                focus_payload: None,
                air_wing_payload: None,
                air_stats_payload: None,
                naval_payload: None,
                players_payload: None,
                session_payload: None,
                production_rest_payload: None,
                construction_rest_payload: None,
                economy_rest_payload: None,
                air_bases_payload: None,
                air_history_payload: None,
                supremacy_payload: None,
                sunk_payload: None,
                radar_payload: None,
                intel_payload: None,
                weather_payload: None,
                capitulation_payload: None,
            };
            shared.decode_queue.push(job).unwrap();
        }
        let mut handed = Vec::new();
        while let Some(mut job) = shared.decode_queue.pop() {
            process_job(&shared, &mut job, &mut AirBasesMemo::default());
            handed.push(job.index);
        }
        assert_eq!(handed, indices);
        for index in indices {
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.completed[index].load(Ordering::Acquire), 7);
        }
        assert!(shared.ready.pop().is_none());
    }

    #[test]
    fn queue_full_returns_the_job_for_the_existing_gap_path() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let shared = Shared::new(0, 0, 1);
        for seq in 0..BUFFERS {
            let job = DecodeJob {
                index: 0,
                seq: seq as u64,
                hour: 0,
                relations_refresh: false,
                characters_refresh: [false; CHARACTER_PARTS],
                projects_refresh: false,
                lexical_refresh: false,
                divisions_refresh: false,
                focus_refresh: false,
                air_wing_refresh: false,
                air_stats_refresh: false,
                naval_refresh: false,
                players_refresh: false,
                session_refresh: false,
                production_rest_refresh: false,
                construction_rest_refresh: false,
                economy_rest_refresh: false,
                air_bases_refresh: false,
                air_history_refresh: false,
                supremacy_refresh: false,
                sunk_refresh: false,
                radar_refresh: false,
                intel_refresh: false,
                weather_refresh: false,
                capitulation_refresh: false,
                force_full: false,
                game_elapsed: 1,
                relations_payload: None,
                characters_payload: [const { None }; CHARACTER_PARTS],
                projects_payload: None,
                lexical_payload: None,
                divisions_payload: None,
                focus_payload: None,
                air_wing_payload: None,
                air_stats_payload: None,
                naval_payload: None,
                players_payload: None,
                session_payload: None,
                production_rest_payload: None,
                construction_rest_payload: None,
                economy_rest_payload: None,
                air_bases_payload: None,
                air_history_payload: None,
                supremacy_payload: None,
                sunk_payload: None,
                radar_payload: None,
                intel_payload: None,
                weather_payload: None,
                capitulation_payload: None,
            };
            assert!(shared.decode_queue.push(job).is_ok());
        }
        let before = DROPS.load(Ordering::Relaxed);
        let job = DecodeJob {
            index: 0,
            seq: u64::MAX,
            hour: 0,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 1,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        assert!(enqueue(&shared, job).is_some());
        assert_eq!(DROPS.load(Ordering::Relaxed), before + 1);
    }

    #[test]
    fn worker_decodes_a_relations_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::intel::relations_test_kit;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = relations_test_kit::opinionated_fixture();
        let memory = relations_test_kit::memory(&bytes);
        let shared = Shared::new(relations_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        // Reference with the pooled caps so truncation behavior matches.
        let opinion_cap = frame.extended.relations_pilot().opinion_rows_cap();
        let tuple_cap = frame.extended.relations_pilot().tuple_rows_cap();
        let direct = relations_test_kit::direct(&memory, opinion_cap, tuple_cap);
        let (report, first) = relations_test_kit::split(&memory, frame.extended.relations_pilot());
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 1,
            relations_refresh: true,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 17,
            relations_payload: Some(first),
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 17);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        relations_test_kit::assert_matches_direct(frame.extended.relations_pilot(), &direct);
    }

    #[test]
    fn worker_relations_mixed_payload_keeps_hourly_tuples_and_skips_opinions() {
        use crate::extended::intel::{self, RelationsCopyContext, RelationsJob};
        use engine::walk::Outcome;
        use intel::relations_test_kit as kit;

        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::opinionated_fixture();
        let memory = kit::memory(&bytes);
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        for opinions_active in [false, true, false] {
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            let captured = frame.extended.relations_pilot();
            let opinion_cap = captured.opinion_rows_cap();
            let tuple_cap = captured.tuple_rows_cap();
            let mut direct = kit::direct(&memory, opinion_cap, tuple_cap);
            if !opinions_active {
                direct.opinion_rows.clear();
                direct.opinion_outcome = Outcome::Unavailable {
                    reason: replay::cadence::SKIPPED_REASON,
                };
            }
            let mut payload = RelationsJob::new();
            let report = intel::copy_relations_refresh(
                &memory,
                RelationsCopyContext {
                    base: kit::BASE,
                    game_state: kit::GS,
                    active: [opinions_active, true],
                    opinion_cap,
                    tuple_cap,
                    epoch: 0,
                },
                &mut payload,
            );
            assert!(report.raw_ok);
            if !opinions_active {
                assert_eq!(report.copy_ticks[0], 0);
                assert_eq!(report.raw_bytes[0], 0);
            }
            captured.relations_raw_mode = true;
            let mut job = bare_job(index, next_seq(), 23, 35);
            job.force_full = true;
            job.relations_refresh = true;
            job.relations_payload = Some(payload);
            process_owned_job(&shared, job, &mut AirBasesMemo::default());
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.completed[index].load(Ordering::Acquire), 35);
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            kit::assert_matches_direct(frame.extended.relations_pilot(), &direct);
            assert!(!frame.extended.relations_pilot().relations_raw_mode);
        }
    }

    #[test]
    fn worker_decodes_a_characters_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::characters::characters_test_kit;
        use crate::extended::characters::{CharactersCopyContext, CharactersJob};
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = characters_test_kit::mapped_fixture();
        let memory = characters_test_kit::memory(&bytes);
        let shared = Shared::new(characters_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let capacities = crate::extended::characters::Capacities {
            rows: [4096; 6],
            bytes: [131_072; 6],
        };
        let direct = characters_test_kit::direct(&memory, &capacities);
        let mut initial = capacities;
        initial.rows[2] = 0;
        initial.bytes[2] = 0;
        *frame.extended.characters_pilot() =
            crate::extended::characters::Captured::with_capacities(&initial);
        let family_bytes_before = frame.extended.characters_pilot().allocated_bytes();
        let pool_bytes_before = shared.pool_bytes.load(Ordering::Relaxed);
        let (trait_rows, trait_bytes) = direct.trait_capture_capacities();
        let ctx = CharactersCopyContext {
            force_full: false,
            base: characters_test_kit::BASE,
            game_state: characters_test_kit::GAME_STATE,
            active: [true; 6],
            trait_rows,
            trait_bytes,
        };
        let mut first = CharactersJob::new();
        let report = crate::extended::characters::copy_characters_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        frame.extended.characters_pilot().raw_active = [true; 6];
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 0,
            relations_refresh: false,
            characters_refresh: [true, false, false, false],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 19,
            relations_payload: None,
            characters_payload: [Some(first), None, None, None],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 19);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        characters_test_kit::assert_matches_direct(frame.extended.characters_pilot(), &direct);
        let grown = frame.extended.characters_pilot().allocated_bytes() - family_bytes_before;
        assert!(grown > 0);
        assert_eq!(
            shared.pool_bytes.load(Ordering::Relaxed),
            pool_bytes_before + grown as u64
        );
        // The worker clears the raw-mode gate it consumed.
        assert!(
            !frame
                .extended
                .characters_pilot()
                .raw_active
                .iter()
                .any(|&active| active)
        );
    }

    #[test]
    fn independent_character_handoffs_isolate_a_missing_part_before_publication() {
        use crate::extended::characters::{self, CharactersCopyContext};
        use characters::characters_test_kit as kit;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let capacities = characters::Capacities {
            rows: [4096; 6],
            bytes: [131_072; 6],
        };
        for (missing, missing_mask) in CHARACTER_MASKS.into_iter().enumerate() {
            let bytes = kit::mapped_fixture();
            let memory = kit::memory(&bytes);
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            let captured = frame.extended.characters_pilot();
            *captured = characters::Captured::with_capacities(&capacities);
            captured.set_character_activity([true; 6]);
            captured.raw_active = [true; 6];
            captured.capture(&memory, kit::BASE, kit::GAME_STATE, 0, false);
            let (trait_rows, trait_bytes) = captured.trait_capture_capacities();
            let mut job = bare_job(index, next_seq(), 0, 37);
            job.characters_refresh = [true; CHARACTER_PARTS];
            for (part, payload) in job.characters_payload.iter_mut().enumerate() {
                if part == missing {
                    continue;
                }
                let mut owned = CharactersJob::new_part(part);
                let report = characters::copy_characters_refresh(
                    &memory,
                    CharactersCopyContext {
                        force_full: false,
                        base: kit::BASE,
                        game_state: kit::GAME_STATE,
                        active: CHARACTER_MASKS[part],
                        trait_rows,
                        trait_bytes,
                    },
                    &mut owned,
                );
                assert!(report.raw_ok);
                *payload = Some(owned);
            }
            let mut expected = kit::direct(&memory, &capacities);
            expected.fail_character_tables(missing_mask, MISSING_PAYLOAD_REASON);
            drop(bytes);
            process_owned_job(&shared, job, &mut AirBasesMemo::default());
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.ready.pop(), None);
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            kit::assert_matches_direct(frame.extended.characters_pilot(), &expected);
            assert_eq!(frame.extended.characters_pilot().raw_active, [false; 6]);
        }
    }

    #[test]
    fn worker_characters_mixed_payload_matches_native_with_frozen_activity() {
        use crate::extended::characters::{self, CharactersCopyContext, CharactersJob};
        use characters::characters_test_kit as kit;
        use engine::walk::Outcome;

        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::mapped_fixture();
        let memory = kit::memory(&bytes);
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let capacities = characters::Capacities {
            rows: [4096; 6],
            bytes: [131_072; 6],
        };
        for (hour, force_full) in [(0, false), (21, false), (7, true)] {
            let active =
                CHARACTERS.map(|table| replay::cadence::should_walk(table, hour, force_full));
            let mut direct = characters::Captured::with_capacities(&capacities);
            direct.capture(&memory, kit::BASE, kit::GAME_STATE, hour, force_full);
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            *frame.extended.characters_pilot() = characters::Captured::with_capacities(&capacities);
            let (trait_rows, trait_bytes) = direct.trait_capture_capacities();
            let mut payload = CharactersJob::new();
            let report = characters::copy_characters_refresh(
                &memory,
                CharactersCopyContext {
                    force_full: false,
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    active,
                    trait_rows,
                    trait_bytes,
                },
                &mut payload,
            );
            assert!(report.raw_ok);
            for (slot, is_active) in active.into_iter().enumerate() {
                if !is_active {
                    assert_eq!(report.copy_ticks[slot], 0);
                    assert_eq!(report.raw_bytes[slot], 0);
                }
            }
            frame.extended.characters_pilot().raw_active = [true; 6];
            let mut job = bare_job(index, next_seq(), 23, 39);
            job.force_full = !force_full;
            job.characters_refresh = [true, false, false, false];
            job.characters_payload = [Some(payload), None, None, None];
            process_owned_job(&shared, job, &mut AirBasesMemo::default());
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.completed[index].load(Ordering::Acquire), 39);
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            let actual = frame.extended.characters_pilot();
            kit::assert_matches_direct(actual, &direct);
            for (is_active, outcome) in active.into_iter().zip(actual.outcomes()) {
                if !is_active {
                    assert_eq!(
                        outcome,
                        Outcome::Unavailable {
                            reason: replay::cadence::SKIPPED_REASON
                        }
                    );
                }
            }
            assert!(!actual.raw_active.iter().any(|&active| active));
        }
    }

    #[test]
    fn worker_decodes_a_projects_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::projects::projects_test_kit;
        use crate::extended::projects::{ProjectsCopyContext, ProjectsJob};
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = projects_test_kit::fixture();
        let memory = projects_test_kit::memory(&bytes);
        let shared = Shared::new(projects_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let capacities = crate::extended::projects::Capacities {
            rows: [256; crate::extended::projects::N],
            program_names: 8192,
            program_candidates: 1024,
            raid_type_tokens: 256,
            raid_show_for: 256,
            raid_transports: 16,
            raid_equipment_items: 16,
            raid_equipment_archetypes: 16,
            counteroperatives: 16,
        };
        let direct = projects_test_kit::direct(&memory, &capacities);
        let ctx = ProjectsCopyContext {
            base: projects_test_kit::BASE,
            game_state: projects_test_kit::GAME_STATE,
            active: true,
        };
        let mut first = ProjectsJob::new();
        let report = crate::extended::projects::copy_projects_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        frame.extended.projects_pilot().raw_mode = true;
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 3,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: true,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 23,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: Some(first),
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 23);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        projects_test_kit::assert_matches_direct(frame.extended.projects_pilot(), &direct);
        // The worker clears the raw-mode gate it consumed.
        assert!(!frame.extended.projects_pilot().raw_mode);
    }

    #[test]
    fn worker_decodes_a_lexical_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::lexical::lexical_test_kit;
        use crate::extended::lexical::{LexicalCopyContext, LexicalJob};
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let memory = lexical_test_kit::KitMemory::fixture();
        let shared = Shared::new(lexical_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let capacities = crate::extended::lexical::Capacities {
            rows: 256,
            text: 8192,
        };
        let direct = lexical_test_kit::direct(&memory, &capacities);
        let ctx = LexicalCopyContext {
            base: lexical_test_kit::BASE,
        };
        let mut first = LexicalJob::new();
        let report = crate::extended::lexical::copy_lexical_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        frame.extended.lexical_pilot().raw_mode = true;
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 3,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: true,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 29,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: Some(first),
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 29);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        lexical_test_kit::assert_matches_direct(frame.extended.lexical_pilot(), &direct);
        // The worker clears the raw-mode gate it consumed.
        assert!(!frame.extended.lexical_pilot().raw_mode);
    }

    #[test]
    fn worker_preserves_already_typed_wars_rows() {
        use crate::sampling::wars_test_kit;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = wars_test_kit::fixture();
        let memory = wars_test_kit::memory(&bytes);
        let shared = Shared::new(wars_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let direct = wars_test_kit::direct(&memory, 64);
        {
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            frame.wars_result = direct.owned_outcome;
            frame.wars[..direct.owned.len()].copy_from_slice(&direct.owned);
            frame.table_ns[TableId::WarRelations.index()] = 601;
        }
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 41,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 43,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 43);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(frame.wars_result, direct.owned_outcome);
        assert_eq!(&frame.wars[..direct.owned.len()], &direct.owned[..]);
        assert_eq!(frame.table_ns[TableId::WarRelations.index()], 601);
    }

    #[test]
    fn worker_decodes_a_divisions_refresh_payload_into_the_same_typed_rows() {
        use crate::sampling::{
            DivisionsCopyContext, DivisionsJob, copy_divisions_refresh, divisions_test_kit,
        };
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = divisions_test_kit::fixture();
        let memory = divisions_test_kit::memory(&bytes);
        let shared = Shared::new(divisions_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let direct = divisions_test_kit::direct(&memory, [64; 6]);
        let ctx = DivisionsCopyContext {
            base: divisions_test_kit::BASE,
            game_state: divisions_test_kit::GAME_STATE,
            row_capacity: 64,
            source_capacity: 64,
            detail_capacities: [64, 0, 64, 64, 64],
            details_active: true,
        };
        let mut first = DivisionsJob::new();
        let report = copy_divisions_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.extended.test_division_detail().groups.rows = Vec::new();
        let before_bytes = frame.extended.allocated_bytes();
        let pool_before = shared.pool_bytes.load(Ordering::Relaxed);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 42,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: true,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 47,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: Some(first),
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 47);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(
            shared.pool_bytes.load(Ordering::Relaxed),
            pool_before + (frame.extended.allocated_bytes() - before_bytes) as u64
        );
        assert_eq!(frame.divisions_result, direct.divisions_outcome);
        assert_eq!(
            &frame.divisions[..direct.divisions.len()],
            &direct.divisions[..]
        );
        let detail = frame.extended.test_division_detail();
        assert_eq!(detail.division_equipment.outcome, direct.equipment_outcome);
        assert_eq!(
            &detail.division_equipment.rows[..direct.equipment.len()],
            &direct.equipment[..]
        );
        assert_eq!(detail.groups.outcome, direct.groups_outcome);
        assert_eq!(
            &detail.groups.rows[..direct.groups.len()],
            &direct.groups[..]
        );
        assert_eq!(detail.requests.outcome, direct.requests_outcome);
        assert_eq!(
            &detail.requests.rows[..direct.requests.len()],
            &direct.requests[..]
        );
        assert_eq!(detail.deliveries.outcome, direct.deliveries_outcome);
        assert_eq!(
            &detail.deliveries.rows[..direct.deliveries.len()],
            &direct.deliveries[..]
        );
        assert_eq!(detail.produced.outcome, direct.produced_outcome);
        assert_eq!(
            &detail.produced.rows[..direct.produced.len()],
            &direct.produced[..]
        );
        // The worker clears the raw-mode gate it consumed.
        assert!(!detail.divisions_raw_mode);
        // Core decode remains hourly; this fixture explicitly requests details.
        assert_eq!(
            DIVISIONS,
            [
                TableId::Divisions,
                TableId::DivisionEquipment,
                TableId::ArmyUpgradeGroups,
                TableId::ArmyUpgradeRequests,
                TableId::ArmyUpgradeDeliveries,
                TableId::ArmyUpgradeProduced,
            ]
        );
    }

    #[test]
    fn fifo_uses_frozen_division_detail_activity_and_retires_the_owned_core_once() {
        use crate::sampling::{
            DivisionsCopyContext, copy_divisions_refresh, divisions_test_kit as kit,
        };
        use engine::walk::Outcome;

        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let skipped = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        frame
            .extended
            .fail_division_details(replay::cadence::SKIPPED_REASON);
        frame.extended.set_divisions_raw_mode(true);
        let detail_pointer = frame
            .extended
            .test_division_detail()
            .division_equipment
            .rows
            .as_ptr();
        let pool_len = DIVISIONS_JOB_POOL.lock().unwrap().len();
        let mut payload = acquire_divisions_job();
        let context = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            row_capacity: 64,
            source_capacity: 64,
            // Inactive detail capacities do not constrain the hourly core.
            detail_capacities: [usize::MAX; 5],
            details_active: false,
        };
        let report = copy_divisions_refresh(&memory, context, &mut payload);
        assert!(report.raw_ok);
        assert_eq!(report.copy_ticks[1..], [0; 5]);
        assert_eq!(report.raw_bytes[1..], [0; 5]);
        let mut job = bare_job(index, next_seq(), 2, 43);
        // The envelope cannot override the capture's frozen activity, even
        // when it names the detail slot and requests a force-full frame.
        job.force_full = true;
        job.divisions_refresh = true;
        job.divisions_payload = Some(payload);
        assert!(enqueue(&shared, job).is_none());
        process_owned_job(
            &shared,
            shared.decode_queue.pop().expect("queued core payload"),
            &mut AirBasesMemo::default(),
        );
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.ready.pop(), None);
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 43);
        assert_eq!(DIVISIONS_JOB_POOL.lock().unwrap().len(), pool_len);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
        let detail = frame.extended.test_division_detail();
        assert_eq!(detail.division_equipment.rows.as_ptr(), detail_pointer);
        for outcome in [
            detail.division_equipment.outcome,
            detail.groups.outcome,
            detail.requests.outcome,
            detail.deliveries.outcome,
            detail.produced.outcome,
        ] {
            assert_eq!(outcome, skipped);
        }
        assert!(!detail.divisions_raw_mode);
        assert_eq!(calibration().divisions_decode_ns[1..], [0; 5]);
    }

    #[test]
    fn prior_active_division_details_stay_skipped_through_core_failure_and_retirement() {
        use crate::{
            read_accounting,
            sampling::{
                DivisionsCopyContext, copy_divisions_core_refresh, copy_divisions_inputs_refresh,
                copy_divisions_refresh, divisions_test_kit as kit,
            },
        };
        use engine::{
            Memory,
            memory::{CountedMemory, ReadCounts},
            walk::Outcome,
        };
        use std::{
            cell::{Cell, RefCell},
            sync::Arc,
        };

        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("inactive details or completed core must not read again");
            }
        }
        struct CoreFault<'a> {
            bytes: &'a [u8],
            calls: RefCell<Vec<(usize, usize)>>,
            fault_attempts: Cell<usize>,
        }
        impl Memory for CoreFault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, output.len()));
                let fault = kit::GAME_STATE + engine::cgame_state::PROVINCES;
                if address <= fault && address.saturating_add(output.len()) > fault {
                    let attempts = self.fault_attempts.get();
                    self.fault_attempts.set(attempts + 1);
                    if attempts == 0 {
                        return false;
                    }
                }
                kit::memory(self.bytes).copy(address, output)
            }
        }

        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let pool_len = DIVISIONS_JOB_POOL.lock().unwrap().len();
        let retired = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
        for (active_hour, reject_context) in [(2, false), (26, true)] {
            let context = DivisionsCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                row_capacity: 64,
                source_capacity: 64,
                detail_capacities: [64; 5],
                details_active: true,
            };
            let mut payload = acquire_divisions_job();
            assert!(copy_divisions_refresh(&kit::memory(&bytes), context, &mut payload).raw_ok);
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            frame.extended.set_divisions_raw_mode(true);
            assert_eq!(
                frame.extended.production_rest_pilot().acquire(
                    &NoRead,
                    kit::BASE,
                    kit::GAME_STATE,
                    crate::extended::production::Sources {
                        rows: &[],
                        outcome: Outcome::Rows { len: 0 },
                    },
                    active_hour,
                    false,
                ),
                [0; crate::extended::production::N]
            );
            let mut active = bare_job(index, next_seq(), active_hour, 41);
            active.divisions_refresh = true;
            active.divisions_payload = Some(payload);
            assert!(enqueue(&shared, active).is_none());
            process_owned_job(
                &shared,
                shared.decode_queue.pop().expect("queued active details"),
                &mut AirBasesMemo::default(),
            );
            assert_eq!(shared.ready.pop(), Some(index));
            assert!(shared.ready.is_empty());
            assert_eq!(DIVISIONS_JOB_POOL.lock().unwrap().len(), pool_len);
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
            let detail = frame.extended.test_division_detail();
            assert!(detail.division_details_active());
            for outcome in [
                detail.division_equipment.outcome,
                detail.groups.outcome,
                detail.requests.outcome,
                detail.deliveries.outcome,
                detail.produced.outcome,
            ] {
                assert!(matches!(outcome, Outcome::Rows { len } if len > 0));
            }
            let detail_pointer = detail.division_equipment.rows.as_ptr();
            let first_equipment = detail.division_equipment.rows[0];
            let detail_decode_before = calibration().divisions_decode_ns;

            let hour = active_hour + 1;
            let context = DivisionsCopyContext {
                // Both readiness checks must ignore these impossible grants.
                detail_capacities: [usize::MAX; 5],
                details_active: false,
                ..context
            };
            let memory = CountedMemory::new(CoreFault {
                bytes: &bytes,
                calls: RefCell::new(Vec::new()),
                fault_attempts: Cell::new(0),
            });
            let reads = read_accounting::Capture::new();
            let mut payload = acquire_divisions_job();
            assert!(copy_divisions_inputs_refresh(
                &memory,
                context,
                &mut payload
            ));
            assert_eq!(memory.0.fault_attempts.get(), 0);
            let lease = Arc::clone(payload.census.unit_inputs().unwrap());
            let input_pointer = Arc::as_ptr(&lease);
            frame.extended.install_unit_inputs(Some(Arc::clone(&lease)));
            assert_eq!(Arc::strong_count(&lease), 3);
            let report = if reject_context {
                copy_divisions_core_refresh(
                    &NoRead,
                    DivisionsCopyContext {
                        row_capacity: 0,
                        ..context
                    },
                    &mut payload,
                )
            } else {
                copy_divisions_core_refresh(&memory, context, &mut payload)
            };
            assert_eq!(report.raw_ok, !reject_context);
            assert_eq!(report.copy_ticks[1..], [0; 5]);
            assert_eq!(report.raw_bytes[1..], [0; 5]);
            let before_repeat = read_accounting::snapshot();
            let repeated = copy_divisions_core_refresh(&NoRead, context, &mut payload);
            assert_eq!(repeated.raw_ok, report.raw_ok);
            assert_eq!(repeated.copy_ticks, report.copy_ticks);
            assert_eq!(repeated.raw_bytes, report.raw_bytes);
            assert_eq!(read_accounting::snapshot(), before_repeat);
            assert_eq!(memory.0.fault_attempts.get(), usize::from(!reject_context));
            assert!(Arc::ptr_eq(&lease, payload.census.unit_inputs().unwrap()));

            frame.extended.set_divisions_raw_mode(true);
            assert_eq!(
                frame.extended.production_rest_pilot().acquire(
                    &NoRead,
                    kit::BASE,
                    kit::GAME_STATE,
                    crate::extended::production::Sources {
                        rows: &[],
                        outcome: Outcome::Rows { len: 0 },
                    },
                    hour,
                    false,
                ),
                [0; crate::extended::production::N]
            );
            assert!(
                !frame
                    .extended
                    .test_division_detail()
                    .division_details_active()
            );
            let failure = if reject_context {
                "division owned acquisition could not complete"
            } else {
                "province array unreadable or invalid"
            };
            // A failed hourly core cannot mark the five inactive siblings failed.
            frame.extended.fail_division_details(failure);
            let counts = reads.finish();
            let trace = memory.0.calls.borrow();
            let core = counts[TableId::Divisions.index()];
            assert!(core.spans > 0);
            assert_eq!(
                core.spans,
                trace.iter().filter(|(_, len)| *len != 0).count() as u64
            );
            assert_eq!(
                core.bytes,
                trace.iter().map(|(_, len)| *len as u64).sum::<u64>()
            );
            for table in TableId::ALL {
                if table != TableId::Divisions {
                    assert_eq!(counts[table.index()], ReadCounts::ZERO, "{table:?}");
                }
            }
            drop(trace);
            frame.table_reads = counts;
            frame.extended.release_unit_inputs();
            drop(lease);
            assert_eq!(Arc::strong_count(payload.census.unit_inputs().unwrap()), 1);
            assert!(retired.is_empty());

            let mut failed = bare_job(index, next_seq(), hour, 43);
            if reject_context {
                frame.divisions_result = Outcome::Unavailable { reason: failure };
                recycle_divisions_job(payload);
            } else {
                failed.divisions_refresh = true;
                failed.divisions_payload = Some(payload);
            }
            assert_eq!(retired.len(), usize::from(reject_context));
            assert_eq!(DIVISIONS_JOB_POOL.lock().unwrap().len(), pool_len - 1);
            assert!(enqueue(&shared, failed).is_none());
            let decoding = read_accounting::Capture::new();
            process_owned_job(
                &shared,
                shared.decode_queue.pop().expect("queued failed core"),
                &mut AirBasesMemo::default(),
            );
            assert_eq!(decoding.finish(), read_accounting::ZERO);
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.ready.pop(), None);
            assert!(shared.decode_queue.is_empty());
            assert_eq!(shared.completed[index].load(Ordering::Acquire), 43);
            assert!(retired.is_empty());
            assert_eq!(DIVISIONS_JOB_POOL.lock().unwrap().len(), pool_len);
            reclaim_payloads();
            let pool = DIVISIONS_JOB_POOL.lock().unwrap();
            assert_eq!(pool.len(), pool_len);
            assert_eq!(
                Arc::as_ptr(pool.last().unwrap().census.unit_inputs().unwrap()),
                input_pointer
            );
            drop(pool);
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            assert_eq!(
                frame.divisions_result,
                Outcome::Unavailable { reason: failure }
            );
            assert_eq!(frame.table_reads, counts);
            let detail = frame.extended.test_division_detail();
            for outcome in [
                detail.division_equipment.outcome,
                detail.groups.outcome,
                detail.requests.outcome,
                detail.deliveries.outcome,
                detail.produced.outcome,
            ] {
                assert_eq!(
                    outcome,
                    Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON
                    }
                );
            }
            assert_eq!(detail.division_equipment.rows.as_ptr(), detail_pointer);
            assert_eq!(detail.division_equipment.rows[0], first_equipment);
            assert!(!detail.divisions_raw_mode);
            assert_eq!(
                calibration().divisions_decode_ns[1..],
                detail_decode_before[1..]
            );
        }
    }

    #[test]
    fn initial_payload_capacities_cover_documented_seed_shapes() {
        use crate::extended::{
            air::AirStatsJob, characters::CharactersJob, intel::RelationsJob, lexical::LexicalJob,
            projects::ProjectsJob,
        };
        // Analytic worst cases from the 2026-09-11 appendix (late-campaign
        // maxima): relations 8639 opinions / 1512 tuples; characters 13127 rows with
        // 19744 experience rows; projects 21511 rows with 1756 breakthroughs;
        // lexical 70851 tokens; divisions 2843 with 17364 equipment rows and
        // about 4000 upgrade rows. Every cap below must exceed its shape's
        // span/byte cost so staging overflow — which falls back to the typed
        // walk with identical rows (`raw_ok=false`, enqueues nothing, no gap)
        // — never triggers in steady state. This is distinct from a full
        // decode queue, which takes the existing whole-frame drop plus
        // `LossBatch` gap path. A
        // force-full hour stages every family at once. These are seed-shape
        // assertions, not an aggregate memory bound or a growth limit.
        let relations = RelationsJob::new();
        assert!(relations.opinions.len() >= 8639);
        assert!(relations.tuples.len() >= 1512);
        let characters = CharactersJob::new();
        assert!(characters.log.spans.capacity() >= 300_000);
        assert!(characters.log.bytes.capacity() >= 10 << 20);
        let projects = ProjectsJob::new();
        assert!(projects.log.spans.capacity() >= 21_511 * 32);
        assert!(projects.log.bytes.capacity() >= 21_511 * 1024);
        let lexical = LexicalJob::new();
        assert!(lexical.capacities().0 >= 70_851 + 16);
        assert!(lexical.capacities().1 >= 70_851 * 32 * 3);
        let divisions = crate::sampling::DivisionsJob::new();
        assert!(divisions.initialized());
        // Hourly focus worst case is 439 countries with about 3.9k completed
        // rows plus token bytes (the appendix census proves the shape).
        let focus = FocusJob::new();
        assert!(focus.capacities().0[0] >= 439);
        assert!(focus.capacities().0[1] >= 3_900);
        assert!(focus.capacities().1 >= 128 << 10);
        // Hourly air-statistics worst case is 330 rows (appendix census)
        // with per-history channel spans; both pools start above that so
        // steady state never overflows.
        let air_stats = AirStatsJob::new();
        assert!(air_stats.log.spans.capacity() >= 330 * 64);
        assert!(air_stats.log.bytes.capacity() >= 330 * 2048);
        // Queue capacity follows the typed-frame ownership bound. Raw spans,
        // metadata and all three owned jobs must also be counted in memory
        // reporting; byte arenas alone substantially undercount the live set.
        assert_eq!(BUFFERS, 4);
    }

    #[test]
    fn queue_high_water_tracks_the_deepest_observed_fifo_occupancy() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let shared = Shared::new(0, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        for _ in 0..3 {
            let job = DecodeJob {
                index,
                seq: next_seq(),
                hour: 0,
                relations_refresh: false,
                characters_refresh: [false; CHARACTER_PARTS],
                projects_refresh: false,
                lexical_refresh: false,
                divisions_refresh: false,
                focus_refresh: false,
                air_wing_refresh: false,
                air_stats_refresh: false,
                naval_refresh: false,
                players_refresh: false,
                session_refresh: false,
                production_rest_refresh: false,
                construction_rest_refresh: false,
                economy_rest_refresh: false,
                air_bases_refresh: false,
                air_history_refresh: false,
                supremacy_refresh: false,
                sunk_refresh: false,
                radar_refresh: false,
                intel_refresh: false,
                weather_refresh: false,
                capitulation_refresh: false,
                force_full: false,
                game_elapsed: 1,
                relations_payload: None,
                characters_payload: [const { None }; CHARACTER_PARTS],
                projects_payload: None,
                lexical_payload: None,
                divisions_payload: None,
                focus_payload: None,
                air_wing_payload: None,
                air_stats_payload: None,
                naval_payload: None,
                players_payload: None,
                session_payload: None,
                production_rest_payload: None,
                construction_rest_payload: None,
                economy_rest_payload: None,
                air_bases_payload: None,
                air_history_payload: None,
                supremacy_payload: None,
                sunk_payload: None,
                radar_payload: None,
                intel_payload: None,
                weather_payload: None,
                capitulation_payload: None,
            };
            assert!(enqueue(&shared, job).is_none());
        }
        let snapshot = calibration();
        assert_eq!(snapshot.queue_high_water, 3);
        assert_eq!(snapshot.json()["queue_high_water"], 3);
        assert_eq!(snapshot.json()["queue_capacity"], BUFFERS);
        while shared.decode_queue.pop().is_some() {}
    }

    #[test]
    fn worker_decodes_a_focus_payload_into_the_same_typed_rows() {
        use crate::extended::governance::focus_test_kit;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let fixture = focus_test_kit::Fixture::world();
        let memory = fixture.memory();
        let shared = Shared::new(focus_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let mut typed = focus_test_kit::Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
        frame.extended.focus_pilot().focus_raw_mode = true;
        let (report, first) =
            focus_test_kit::Fixture::copy_first(frame.extended.focus_pilot(), &memory);
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 7,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: true,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 23,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: Some(first),
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 23);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let pilot = frame.extended.focus_pilot();
        assert!(!pilot.focus_raw_mode);
        focus_test_kit::Fixture::assert_matches_typed(pilot, &typed);
        let snapshot = calibration();
        assert_eq!(snapshot.focus_copy_ns, [0, 0]);
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(snapshot.json()["focus_tables"][0], "Focus");
        assert_eq!(snapshot.json()["focus_tables"][1], "CompletedFocuses");
    }

    #[test]
    fn worker_decodes_a_wing_pack_payload_into_the_same_typed_rows() {
        use crate::extended::air::{AirWingCopyContext, wing_test_kit};
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = wing_test_kit::fixture();
        let memory = wing_test_kit::memory(&bytes);
        let shared = Shared::new(wing_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let direct = wing_test_kit::direct(&memory);
        let ctx = AirWingCopyContext {
            base: wing_test_kit::BASE,
            game_state: wing_test_kit::GAME_STATE,
            hour: 0,
            force_full: true,
            limits: [1024, 8192, 4096],
            naval_limit: 4096,
        };
        let mut first = acquire_air_wing_job();
        let report = crate::extended::air::copy_air_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 9,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            air_wing_refresh: true,
            air_stats_refresh: false,
            force_full: false,
            game_elapsed: 31,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
            air_wing_payload: Some(first),
            air_stats_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 31);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        wing_test_kit::assert_captured_matches_direct(frame.extended.air_wing_pilot(), &direct);
        let snapshot = calibration();
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(snapshot.json()["air_wing_tables"][0], "WingEquipment");
        recycle_air_wing_job(job.air_wing_payload.unwrap());
    }

    #[test]
    fn worker_decodes_an_air_stats_payload_into_the_same_typed_rows() {
        use crate::extended::air::{AirStatsCopyContext, stats_test_kit};
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = stats_test_kit::fixture();
        let memory = stats_test_kit::memory(&bytes);
        let shared = Shared::new(stats_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let capacities = crate::extended::air::Capacities::default();
        let direct = stats_test_kit::direct(&memory, &capacities);
        let ctx = AirStatsCopyContext {
            base: stats_test_kit::BASE,
            game_state: stats_test_kit::GAME_STATE,
            hour: 0,
            force_full: true,
        };
        let mut first = acquire_air_stats_job();
        let report = crate::extended::air::copy_air_stats_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 9,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: true,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 59,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: Some(first),
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 59);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        stats_test_kit::assert_captured_matches_direct(frame.extended.air_stats_pilot(), &direct);
        assert!(!frame.extended.air_stats_pilot().stats_raw_mode);
        // table_ns stays copy-only for the moved table.
        assert_eq!(frame.table_ns[AIR_STATS[0].index()], 0);
        let snapshot = calibration();
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(snapshot.json()["air_stats_tables"][0], "AirStatistics");
        recycle_air_stats_job(job.air_stats_payload.unwrap());
    }

    #[test]
    fn air_stats_calibration_counters_are_monotonic_and_additive() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        add_air_stats_copy_ns(0, 11);
        add_air_stats_raw_bytes(0, 100);
        let snapshot = calibration();
        assert_eq!(snapshot.air_stats_copy_ns, [11]);
        assert_eq!(snapshot.air_stats_raw_bytes, [100]);
        assert_eq!(snapshot.json()["air_stats_copy_ns"][0], 11);
        assert_eq!(snapshot.json()["air_stats_tables"][0], "AirStatistics");
        let job = acquire_air_stats_job();
        assert!(job.log.spans.capacity() >= 1 << 16);
        recycle_air_stats_job(job);
        let again = acquire_air_stats_job();
        assert!(again.log.spans.capacity() >= 1 << 16);
        recycle_air_stats_job(again);
    }

    #[test]
    fn worker_decodes_a_naval_pack_payload_into_the_same_typed_rows() {
        use crate::extended::naval_history::{NavalCopyContext, naval_test_kit};
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = naval_test_kit::fixture();
        let memory = naval_test_kit::memory(&bytes);
        let shared = Shared::new(naval_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let direct = naval_test_kit::direct(&memory);
        let ctx = NavalCopyContext {
            base: naval_test_kit::BASE,
            game_state: naval_test_kit::GAME_STATE,
            hour: 0,
            force_full: true,
        };
        let mut first = acquire_naval_job();
        naval_test_kit::stage_wings(&memory, &mut first);
        let report = crate::extended::naval_history::copy_naval_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 9,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: true,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 37,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: Some(first),
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 37);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        naval_test_kit::assert_captured_matches_direct(frame.extended.naval_pilot(), &direct);
        let snapshot = calibration();
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(snapshot.json()["naval_tables"][0], "LandTheatres");
        assert_eq!(snapshot.json()["naval_tables"][8], "ArmyGroupMembers");
        recycle_naval_job(job.naval_payload.unwrap());
    }

    #[test]
    fn retained_air_naval_payload_caps_fit_the_force_full_hour_live_set() {
        // Independent fixture worlds exercise each retained consumer pool.
        use crate::extended::air::{AirWingCopyContext, wing_test_kit as air_kit};
        use crate::extended::naval_history::{NavalCopyContext, naval_test_kit as naval_kit};
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let air_bytes = air_kit::fixture();
        let air_memory = air_kit::memory(&air_bytes);
        let mut air_job = acquire_air_wing_job();
        let air_ctx = AirWingCopyContext {
            base: air_kit::BASE,
            game_state: air_kit::GAME_STATE,
            hour: 0,
            force_full: true,
            limits: [1024, 8192, 4096],
            naval_limit: 4096,
        };
        let air_report = crate::extended::air::copy_air_refresh(&air_memory, air_ctx, &mut air_job);
        assert!(
            air_report.raw_ok,
            "wing force-full live set must fit owned storage"
        );
        assert!(!air_job.grow_for_next());
        let naval_bytes = naval_kit::fixture();
        let naval_memory = naval_kit::memory(&naval_bytes);
        let mut naval_job = acquire_naval_job();
        let naval_ctx = NavalCopyContext {
            base: naval_kit::BASE,
            game_state: naval_kit::GAME_STATE,
            hour: 0,
            force_full: true,
        };
        naval_kit::stage_wings(&naval_memory, &mut naval_job);
        let naval_report = crate::extended::naval_history::copy_naval_refresh(
            &naval_memory,
            naval_ctx,
            &mut naval_job,
        );
        assert!(
            naval_report.raw_ok,
            "naval force-full live set must fit spans/bytes"
        );
        assert!(!naval_job.grow_for_next());
        recycle_air_wing_job(air_job);
        recycle_naval_job(naval_job);
    }

    #[test]
    fn focus_calibration_counters_are_monotonic_and_additive_per_table() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        add_focus_copy_ns(0, 11);
        add_focus_copy_ns(0, 4);
        add_focus_copy_ns(1, 0);
        add_focus_raw_bytes(1, 100);
        let snapshot = calibration();
        assert_eq!(snapshot.focus_copy_ns, [15, 0]);
        assert_eq!(snapshot.focus_raw_bytes, [0, 100]);
        assert_eq!(snapshot.json()["focus_copy_ns"][0], 15);
        assert_eq!(snapshot.json()["focus_tables"][0], "Focus");
        // Recycled jobs keep their capacities for the next hour.
        let job = acquire_focus_job();
        assert!(job.is_provisioned());
        assert!(job.capacities().0[0] >= 439);
        assert!(job.capacities().0[1] >= 3_900);
        assert!(job.capacities().1 >= 128 << 10);
        recycle_focus_job(job);
        let again = acquire_focus_job();
        assert!(again.is_provisioned());
        assert!(again.capacities().0[0] >= 439);
        assert!(again.capacities().0[1] >= 3_900);
        assert!(again.capacities().1 >= 128 << 10);
        recycle_focus_job(again);
    }

    #[test]
    fn worker_decodes_a_production_rest_payload_into_the_same_typed_rows() {
        use crate::extended::production::{
            ProductionRestCopyContext, production_rest_test_kit as kit,
        };
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let shared = Shared::new(0, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let direct = kit::direct(&memory, &kit::capacities());
        let ctx = ProductionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
        };
        let mut first = acquire_production_rest_job();
        let report =
            crate::extended::production::copy_production_rest_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        frame.extended.production_rest_pilot().rest_raw_mode = true;
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 11,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: true,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 31,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: Some(first),
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 31);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        kit::assert_matches_direct(frame.extended.production_rest_pilot(), &direct);
        assert!(!frame.extended.production_rest_pilot().rest_raw_mode);
        let snapshot = calibration();
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(
            snapshot.json()["production_rest_tables"][0],
            "ProductionLines"
        );
        assert_eq!(
            snapshot.json()["production_rest_tables"][7],
            "ProductionLicenseRecords"
        );
    }

    #[test]
    fn worker_decodes_a_construction_rest_payload_into_the_same_typed_rows() {
        use crate::extended::construction::{
            ConstructionRestCopyContext, construction_rest_test_kit as kit,
        };
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let shared = Shared::new(0, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let direct = kit::direct(&memory, &kit::capacities());
        let ctx = ConstructionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
        };
        let mut first = acquire_construction_rest_job();
        let report =
            crate::extended::construction::copy_construction_rest_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        frame.extended.construction_rest_pilot().rest_raw_mode = true;
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 12,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: true,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 37,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: Some(first),
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 37);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        kit::assert_matches_direct(frame.extended.construction_rest_pilot(), &direct);
        assert!(!frame.extended.construction_rest_pilot().rest_raw_mode);
        let snapshot = calibration();
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(
            snapshot.json()["construction_rest_tables"][0],
            "StateBaseResources"
        );
        assert_eq!(
            snapshot.json()["construction_rest_tables"][9],
            "CountryCollaboration"
        );
    }

    #[test]
    fn production_rest_calibration_counters_are_monotonic_and_additive_per_table() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        add_production_rest_copy_ns(0, 11);
        add_production_rest_copy_ns(0, 4);
        add_production_rest_copy_ns(7, 9);
        add_production_rest_raw_bytes(7, 100);
        let snapshot = calibration();
        assert_eq!(snapshot.production_rest_copy_ns, [15, 0, 0, 0, 0, 0, 0, 9]);
        assert_eq!(
            snapshot.production_rest_raw_bytes,
            [0, 0, 0, 0, 0, 0, 0, 100]
        );
        assert_eq!(snapshot.json()["production_rest_copy_ns"][0], 15);
        assert_eq!(
            snapshot.json()["production_rest_tables"][0],
            "ProductionLines"
        );
        let job = acquire_production_rest_job();
        assert!(job.log.spans.capacity() >= 1 << 17);
        recycle_production_rest_job(job);
        let again = acquire_production_rest_job();
        assert!(again.log.spans.capacity() >= 1 << 17);
        recycle_production_rest_job(again);
    }

    #[test]
    fn worker_decodes_two_payloads_in_one_job_with_copy_only_table_ns() {
        // S4: two families decode in one process_job on one Shared. Both
        // payloads' rows install identically, both family counters advance
        // (REFRESH_JOBS +2), and per-frame table_ns stays copy-only (the
        // worker never writes it; decode ns goes to the calibration
        // counters). Fixture BASE compatibility: both the wing and naval
        // kits pin BASE=0 with GAME_STATE=0x10000, so one Shared::new(0,..)
        // serves both payloads at full strength with no fixture adaptation
        // and no weakened assertion (each payload is still staged from its
        // own fixture memory and compared against its own typed reference).
        use crate::extended::air::{AirWingCopyContext, wing_test_kit as air_kit};
        use crate::extended::naval_history::{NavalCopyContext, naval_test_kit as naval_kit};
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        // Stage each payload from its own fixture memory (separate address
        // spaces; the kits share BASE/GAME_STATE values but not objects).
        let air_bytes = air_kit::fixture();
        let air_memory = air_kit::memory(&air_bytes);
        let air_direct = air_kit::direct(&air_memory);
        let air_ctx = AirWingCopyContext {
            base: air_kit::BASE,
            game_state: air_kit::GAME_STATE,
            hour: 0,
            force_full: true,
            limits: [1024, 8192, 4096],
            naval_limit: 4096,
        };
        let mut air_job = acquire_air_wing_job();
        let air_report = crate::extended::air::copy_air_refresh(&air_memory, air_ctx, &mut air_job);
        assert!(air_report.raw_ok);
        let naval_bytes = naval_kit::fixture();
        let naval_memory = naval_kit::memory(&naval_bytes);
        let naval_direct = naval_kit::direct(&naval_memory);
        let naval_ctx = NavalCopyContext {
            base: naval_kit::BASE,
            game_state: naval_kit::GAME_STATE,
            hour: 0,
            force_full: true,
        };
        let mut naval_job = acquire_naval_job();
        naval_kit::stage_wings(&naval_memory, &mut naval_job);
        let naval_report = crate::extended::naval_history::copy_naval_refresh(
            &naval_memory,
            naval_ctx,
            &mut naval_job,
        );
        assert!(naval_report.raw_ok);
        // One Shared serves both: BASE coincides (0 == 0), so no mismatch.
        assert_eq!(air_kit::BASE, naval_kit::BASE);
        let shared = Shared::new(air_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        shared.completed[index].store(0, Ordering::Relaxed);
        let refresh_before = REFRESH_JOBS.load(Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 9,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: true,
            air_stats_refresh: false,
            naval_refresh: true,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 53,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: Some(air_job),
            air_stats_payload: None,
            naval_payload: Some(naval_job),
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 53);
        // Both families' rows install identically in the one frame.
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        air_kit::assert_captured_matches_direct(frame.extended.air_wing_pilot(), &air_direct);
        naval_kit::assert_captured_matches_direct(frame.extended.naval_pilot(), &naval_direct);
        // Both counters advance: one REFRESH_JOB per family payload.
        assert_eq!(REFRESH_JOBS.load(Ordering::Relaxed), refresh_before + 2);
        // table_ns stays copy-only: the worker wrote rows + calibration
        // decode ns, never the per-frame copy-only slots.
        for table in AIR_WING.iter().chain(NAVAL_ORDER.iter()) {
            assert_eq!(
                frame.table_ns[table.index()],
                0,
                "table_ns for {table:?} must stay copy-only"
            );
        }
        let snapshot = calibration();
        assert!(snapshot.air_wing_decode_ns.iter().any(|ns| *ns > 0) || snapshot.refresh_jobs >= 2);
        assert!(snapshot.naval_decode_ns.iter().any(|ns| *ns > 0) || snapshot.refresh_jobs >= 2);
        recycle_air_wing_job(job.air_wing_payload.unwrap());
        recycle_naval_job(job.naval_payload.unwrap());
    }

    #[test]
    fn construction_rest_calibration_counters_are_monotonic_and_additive_per_table() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        add_construction_rest_copy_ns(0, 11);
        add_construction_rest_copy_ns(0, 4);
        add_construction_rest_copy_ns(9, 9);
        add_construction_rest_raw_bytes(9, 100);
        let snapshot = calibration();
        assert_eq!(
            snapshot.construction_rest_copy_ns,
            [15, 0, 0, 0, 0, 0, 0, 0, 0, 9]
        );
        assert_eq!(
            snapshot.construction_rest_raw_bytes,
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 100]
        );
        assert_eq!(snapshot.json()["construction_rest_copy_ns"][0], 15);
        assert_eq!(
            snapshot.json()["construction_rest_tables"][0],
            "StateBaseResources"
        );
        let job = acquire_construction_rest_job();
        assert!(job.log.spans.capacity() >= 1 << 17);
        recycle_construction_rest_job(job);
        let again = acquire_construction_rest_job();
        assert!(again.log.spans.capacity() >= 1 << 17);
        recycle_construction_rest_job(again);
    }

    /// Build a FIFO job carrying only the selected family payload (all other
    /// families off). Every worker test below uses this shape with its own
    /// kit-staged payload.
    fn bare_job(index: usize, seq: u64, hour: i32, game_elapsed: i64) -> DecodeJob {
        DecodeJob {
            index,
            seq,
            hour,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        }
    }

    #[test]
    fn worker_air_stats_keeps_current_daily_payloads_and_explicit_missing_results() {
        use crate::extended::air::{AirStatsCopyContext, Capacities, stats_test_kit as kit};
        use engine::walk::Outcome;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let mut bytes = kit::fixture();
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let mut bases_memo = AirBasesMemo::default();
        for (step, hour) in [5, 29, 30, 53, 77, 101, 125, 149].into_iter().enumerate() {
            let mut job = bare_job(index, step as u64 + 1, hour, 41);
            job.air_stats_refresh = replay::cadence::should_walk(AIR_STATS[0], hour, false);
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            let captured = frame.extended.air_stats_pilot();
            captured.prepare_hour(hour, false);
            if job.air_stats_refresh && step != 5 {
                // Every scheduled payload contains a different current value;
                // earlier rows must not survive a skip or missing payload.
                kit::set_first_value(&mut bytes, -(step as i64 + 1));
                let memory = kit::memory(&bytes);
                let mut payload = AirStatsJob::new();
                let report = crate::extended::air::copy_air_stats_refresh(
                    &memory,
                    AirStatsCopyContext {
                        base: kit::BASE,
                        game_state: kit::GAME_STATE,
                        hour,
                        force_full: false,
                    },
                    &mut payload,
                );
                assert!(report.raw_ok);
                captured.stats_raw_mode = true;
                captured.bases_raw_mode = true;
                captured.history_raw_mode = true;
                // The pack reset must preserve the staged-payload reservation.
                captured.capture(
                    &memory,
                    kit::BASE,
                    kit::GAME_STATE,
                    hour,
                    false,
                    &mut Default::default(),
                );
                job.air_stats_payload = Some(payload);
            } else if !job.air_stats_refresh {
                captured.fail_air_stats_capture(replay::cadence::SKIPPED_REASON);
            }
            process_job(&shared, &mut job, &mut bases_memo);
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.completed[index].load(Ordering::Acquire), 41);
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            let captured = frame.extended.air_stats_pilot();
            if step == 2 || step == 5 {
                assert_eq!(
                    captured.outcomes()[6],
                    Outcome::Unavailable {
                        reason: if step == 2 {
                            replay::cadence::SKIPPED_REASON
                        } else {
                            MISSING_PAYLOAD_REASON
                        },
                    }
                );
            } else {
                let direct = kit::direct(&kit::memory(&bytes), &Capacities::default());
                kit::assert_captured_matches_direct(captured, &direct);
            }
            assert!(!captured.stats_raw_mode);
        }
    }

    #[test]
    fn worker_air_bases_memo_invalidates_on_skip_missing_payload_and_ace_mask_changes() {
        use crate::extended::air::{AirBasesCopyContext, air_bases_test_kit as kit};
        use engine::walk::Outcome;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let mut memo = AirBasesMemo::default();
        let expected_hits = [0, 1, 1, 1, 2, 2, 2, 3, 3, 4, 4, 5];
        let expected_installs = [1, 1, 1, 2, 2, 2, 3, 3, 4, 4, 5, 5];
        for step in 1..=12 {
            let mut job = bare_job(index, step, step as i32 + 100, 41);
            job.air_bases_refresh = step != 3;
            if step != 3 && step != 6 {
                let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
                let mut payload = AirBasesJob::new();
                let ctx = AirBasesCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour: if (9..=10).contains(&step) { 21 } else { 0 },
                    force_full: false,
                };
                let report = crate::extended::air::copy_air_bases_refresh(
                    &kit::memory(&bytes),
                    ctx,
                    &mut payload,
                );
                assert!(report.raw_ok);
                if !(9..=10).contains(&step) {
                    assert_eq!(report.copy_ticks[2], 0);
                    assert_eq!(report.raw_bytes[2], 0);
                }
                frame.extended.air_bases_pilot().bases_raw_mode = true;
                frame.extended.radar_pilot().registrations_raw_mode = true;
                job.air_bases_payload = Some(payload);
            }
            process_job(&shared, &mut job, &mut memo);
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.completed[index].load(Ordering::Acquire), 41);
            assert_eq!(
                AIR_BASES_MEMO[0].load(Ordering::Relaxed),
                expected_hits[step as usize - 1]
            );
            assert_eq!(
                AIR_BASES_MEMO[4].load(Ordering::Relaxed),
                expected_installs[step as usize - 1]
            );
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            if !(9..=10).contains(&step) {
                assert_eq!(
                    frame.extended.air_bases_pilot().outcomes()[5],
                    Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON,
                    }
                );
            }
            if step == 6 {
                let missing = Outcome::Unavailable {
                    reason: MISSING_PAYLOAD_REASON,
                };
                assert_eq!(frame.extended.air_bases_pilot().outcomes()[1], missing);
                assert_eq!(frame.extended.radar_pilot().outcomes()[5], missing);
            }
        }
        let diagnostics = calibration().json();
        assert_eq!(diagnostics["air_bases_memo"]["hits"], 5);
        assert_eq!(diagnostics["air_bases_memo"]["misses"], 5);
        assert_eq!(diagnostics["air_bases_memo"]["installs"], 5);
    }

    #[test]
    fn worker_decodes_a_shared_bases_payload_into_the_same_typed_rows() {
        use crate::extended::air::air_bases_test_kit as kit;
        use replay::frame::TableView;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        shared.completed[index].store(0, Ordering::Relaxed);
        // Stage through the real game-thread entry point (cross-pack copy).
        let ctx = crate::extended::air::AirBasesCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            hour: 0,
            force_full: true,
        };
        let mut first = acquire_air_bases_job();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let report = crate::extended::air::copy_air_bases_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        frame.extended.air_bases_pilot().bases_raw_mode = true;
        frame.extended.radar_pilot().registrations_raw_mode = true;
        let mut job = bare_job(index, next_seq(), 7, 41);
        job.air_bases_refresh = true;
        job.air_bases_payload = Some(first);
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 41);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        // Both packs' views install identically: bases and aces in air,
        // registrations in fog, with the worker clearing both raw gates.
        let air = frame.extended.air_bases_pilot().tables();
        let fog = frame.extended.radar_pilot().tables();
        assert!(matches!(air[1], TableView::Complete(_)));
        assert!(matches!(air[5], TableView::Complete(_)));
        assert!(matches!(fog[5], TableView::Complete(_)));
        assert_eq!(
            frame.extended.air_bases_pilot().outcomes()[1],
            direct.bases_outcome
        );
        assert_eq!(
            frame.extended.air_bases_pilot().outcomes()[5],
            direct.aces_outcome
        );
        assert!(!frame.extended.air_bases_pilot().bases_raw_mode);
        let snapshot = calibration();
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(snapshot.json()["air_bases_tables"][0], "AirBases");
        assert_eq!(
            snapshot.json()["air_bases_tables"][1],
            "AirBaseRegistrations"
        );
        assert_eq!(snapshot.json()["air_bases_tables"][2], "AirAces");
        recycle_air_bases_job(job.air_bases_payload.unwrap());
    }

    #[test]
    fn worker_decodes_a_shared_history_payload_into_the_same_typed_rows() {
        use crate::extended::air::air_history_test_kit as kit;
        use replay::frame::TableView;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        shared.completed[index].store(0, Ordering::Relaxed);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let [history_limit, damage_limit] = frame.extended.air_history_pilot().air_history_limits();
        let ctx = crate::extended::air::AirHistoryCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            hour: 0,
            force_full: true,
            history_limit,
            damage_limit,
        };
        let mut first = acquire_air_history_job();
        let report = crate::extended::air::copy_air_history_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        frame.extended.air_history_pilot().history_raw_mode = true;
        let mut job = bare_job(index, next_seq(), 7, 43);
        job.air_history_refresh = true;
        job.air_history_payload = Some(first);
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 43);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let air = frame.extended.air_history_pilot().tables();
        assert!(matches!(air[3], TableView::Complete(_)));
        assert!(matches!(air[4], TableView::Complete(_)));
        assert_eq!(
            frame.extended.air_history_pilot().outcomes()[3],
            direct.history_outcome
        );
        assert_eq!(
            frame.extended.air_history_pilot().outcomes()[4],
            direct.damage_outcome
        );
        assert!(!frame.extended.air_history_pilot().history_raw_mode);
        let snapshot = calibration();
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(snapshot.json()["air_history_tables"][0], "AirRegionHistory");
        assert_eq!(snapshot.json()["air_history_tables"][1], "AirRegionDamage");
        recycle_air_history_job(job.air_history_payload.unwrap());
    }

    #[test]
    fn worker_decodes_a_supremacy_payload_into_the_same_typed_rows() {
        use crate::extended::naval_history::supremacy_test_kit as kit;
        use replay::frame::TableView;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        shared.completed[index].store(0, Ordering::Relaxed);
        let ctx = crate::extended::naval_history::SupremacyCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            hour: 0,
            force_full: true,
        };
        let mut first = acquire_supremacy_job();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let report =
            crate::extended::naval_history::copy_supremacy_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        frame.extended.supremacy_pilot().supremacy_raw_mode = true;
        let mut job = bare_job(index, next_seq(), 7, 53);
        job.supremacy_refresh = true;
        job.supremacy_payload = Some(first);
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 53);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let naval = frame.extended.supremacy_pilot().tables();
        assert!(matches!(naval[0], TableView::Complete(_)));
        assert_eq!(
            frame.extended.supremacy_pilot().outcomes()[0],
            direct.outcome
        );
        assert!(!frame.extended.supremacy_pilot().supremacy_raw_mode);
        let snapshot = calibration();
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(snapshot.json()["supremacy_tables"][0], "NavalSupremacy");
        recycle_supremacy_job(job.supremacy_payload.unwrap());
    }

    #[test]
    fn worker_decodes_a_sunk_payload_into_the_same_typed_rows() {
        use crate::extended::naval_history::sunk_test_kit as kit;
        use replay::frame::TableView;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        shared.completed[index].store(0, Ordering::Relaxed);
        let ctx = crate::extended::naval_history::SunkCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            hour: kit::HOUR,
            force_full: true,
        };
        let mut first = acquire_sunk_job();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let report = crate::extended::naval_history::copy_sunk_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        frame.extended.sunk_pilot().sunk_raw_mode = true;
        let mut job = bare_job(index, next_seq(), 7, 59);
        job.sunk_refresh = true;
        job.sunk_payload = Some(first);
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 59);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        // The sunk table is pack slot 13; the singleton conversion owns the
        // single row, so compare the converted view (one row, two entries).
        // The raw walk holds two records; the singleton owns one row.
        assert_eq!(direct.outcome, engine::walk::Outcome::Rows { len: 2 });
        let naval = frame.extended.sunk_pilot().tables();
        let entries = match &naval[13] {
            TableView::Complete(replay::rows::TableRows::SunkShips(rows)) => {
                assert_eq!(rows.len(), 1);
                rows[0].entries.len()
            }
            other => panic!("expected complete sunk table, found {other:?}"),
        };
        assert_eq!(entries, 2);
        assert!(!frame.extended.sunk_pilot().sunk_raw_mode);
        let snapshot = calibration();
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(snapshot.json()["sunk_tables"][0], "SunkShips");
        recycle_sunk_job(job.sunk_payload.unwrap());
    }

    #[test]
    fn worker_decodes_a_radar_payload_into_the_same_typed_rows() {
        use crate::extended::fog::radar_test_kit as kit;
        use replay::frame::TableView;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut first = acquire_radar_job();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let ctx =
            frame
                .extended
                .radar_pilot()
                .radar_copy_context(kit::BASE, kit::GAME_STATE, 0, true);
        let report = crate::extended::fog::copy_radar_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        frame.extended.radar_pilot().radar_raw_mode = true;
        let mut job = bare_job(index, next_seq(), 7, 61);
        job.radar_refresh = true;
        job.radar_payload = Some(first);
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 61);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let fog = frame.extended.radar_pilot().tables();
        assert!(matches!(fog[0], TableView::Complete(_)));
        assert_eq!(frame.extended.radar_pilot().outcomes()[0], direct.outcome);
        assert!(!frame.extended.radar_pilot().radar_raw_mode);
        let snapshot = calibration();
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(snapshot.json()["radar_tables"][0], "RadarCoverage");
        recycle_radar_job(job.radar_payload.unwrap());
    }

    #[test]
    fn worker_decodes_an_intel_payload_into_the_same_typed_rows() {
        use crate::extended::fog::intel_test_kit as kit;
        use replay::frame::TableView;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let direct = kit::direct(&memory);
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut first = acquire_intel_job();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let ctx = frame
            .extended
            .intel_pilot()
            .intel_copy_context(kit::BASE, kit::GAME_STATE);
        let report =
            crate::extended::fog::copy_intel_refresh_with_divisions(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        frame.extended.intel_pilot().intel_raw_mode = true;
        let mut job = bare_job(index, next_seq(), 7, 67);
        job.intel_refresh = true;
        job.intel_payload = Some(first);
        process_job(&shared, &mut job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 67);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        // UnitIntel is fog pack slot 4.
        let fog = frame.extended.intel_pilot().tables();
        assert!(matches!(fog[4], TableView::Complete(_)));
        assert_eq!(frame.extended.intel_pilot().outcomes()[4], direct.outcome);
        assert!(!frame.extended.intel_pilot().intel_raw_mode);
        let snapshot = calibration();
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(snapshot.json()["intel_tables"][0], "UnitIntel");
        recycle_intel_job(job.intel_payload.unwrap());
    }

    #[test]
    fn new_family_calibration_counters_are_monotonic_and_additive_per_table() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        add_air_bases_copy_ns(0, 11);
        add_air_bases_copy_ns(2, 7);
        add_air_bases_raw_bytes(1, 100);
        add_supremacy_copy_ns(0, 3);
        add_sunk_copy_ns(0, 4);
        add_radar_copy_ns(0, 6);
        add_intel_copy_ns(0, 8);
        add_air_history_copy_ns(1, 2);
        let snapshot = calibration();
        assert_eq!(snapshot.air_bases_copy_ns, [11, 0, 7]);
        assert_eq!(snapshot.air_bases_raw_bytes, [0, 100, 0]);
        assert_eq!(snapshot.supremacy_copy_ns, [3]);
        assert_eq!(snapshot.sunk_copy_ns, [4]);
        assert_eq!(snapshot.radar_copy_ns, [6]);
        assert_eq!(snapshot.intel_copy_ns, [8]);
        assert_eq!(snapshot.air_history_copy_ns, [0, 2]);
        let json = snapshot.json();
        assert_eq!(json["air_bases_copy_ns"][0], 11);
        assert_eq!(json["air_bases_tables"][1], "AirBaseRegistrations");
        assert_eq!(json["air_history_tables"][1], "AirRegionDamage");
        assert_eq!(json["supremacy_tables"][0], "NavalSupremacy");
        assert_eq!(json["sunk_tables"][0], "SunkShips");
        assert_eq!(json["radar_tables"][0], "RadarCoverage");
        assert_eq!(json["intel_tables"][0], "UnitIntel");
        for acquire in [
            acquire_air_bases_job().log.spans.capacity() >= 1 << 17,
            acquire_air_history_job().has_storage(),
            acquire_supremacy_job().log.spans.capacity() >= 1 << 12,
            acquire_sunk_job().log.spans.capacity() >= 1 << 13,
            acquire_radar_job().allocated_bytes() > 0,
            acquire_intel_job().allocated_bytes() > 0,
        ] {
            assert!(acquire);
        }
    }
}
