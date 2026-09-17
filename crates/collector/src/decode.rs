//! Single FIFO decode worker for the retained raw-capture families.
//!
//! Each staged payload owns its current copied bytes. The worker fills the
//! corresponding pooled typed rows before the spool ready-queue handoff.
//! Families acquired directly on the callback already have typed rows; forwarding
//! leaves those rows, comparison results and measured costs untouched.
//! Conversion, differencing, encoding and baselines downstream are unchanged.
//!
//! Ordering: every frame transits this one FIFO in game-thread production
//! order, so spool handoff order is inherently hour order and no reorder
//! buffer exists. Skip (off-refresh) hours carry no payload and bypass decode
//! work entirely, but they still transit the FIFO as ordering placeholders so
//! a slow refresh decode can never let a later skip hour overtake it.
//!
//! Backpressure: the game thread never blocks. The FIFO holds one lightweight
//! job per pooled buffer (payload byte buffers already live on the heap;
//! payload jobs are bounded and recycled, never pool memory), so an owned
//! buffer guarantees a free slot exactly as the spool ready queue does today.
//! A full queue takes the existing whole-frame drop plus `LossBatch` gap path
//! with the existing counters and reason strings. This is distinct from
//! staging overflow (pooled span/byte caps exceeded during the copy): that
//! falls back to the typed walk on the game thread with identical rows
//! (`raw_ok=false`, enqueues nothing) and records no gap.
//!
//! Storage: 256 FIFO slots hold complete jobs, not just indices. Before the
//! hook is installed, each family receives three reusable payload buffers.
//! Exhaustion or lock contention returns an empty staging object and invokes
//! the existing typed fallback; it does not allocate another payload. The
//! private worker grows only overflowed storage and returns owned buffers.
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
            AirBasesJob, AirBasesMemo, AirBasesMemoContext, AirHistoryJob, AirStatsJob,
            AirStatsMemo, AirStatsMemoContext, AirWingJob,
        },
        characters::CharactersJob,
        construction::ConstructionRestJob,
        division_eta::{self, Workspace as EtaWorkspace},
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
/// Focus pair in calibration/report order: [focus, completed]. Hourly like
/// every Tier-H table (no refresh gate): the game thread stages a raw payload
/// every hour and the worker replays it before the ready handoff. The single
/// copy pass is anchor-charged to the focus slot, matching the read-group
/// convention.
pub const FOCUS: [TableId; 2] = [TableId::Focus, TableId::CompletedFocuses];

/// Relations family in calibration/report order: [opinions, tuples].
pub const RELATIONS: [TableId; 2] = [TableId::RelationOpinions, TableId::RelationTuples];

/// Characters family in calibration/report order: [characters, leaders,
/// advisors, units, traits, experience]. ActiveAbilities stays hourly.
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

/// Lexical family: the single global dictionary refreshed on slot 3.
pub const LEXICAL: [TableId; 1] = [TableId::LexicalTokens];
/// Divisions group in calibration/report order: [divisions, equipment,
/// groups, requests, deliveries, produced]. Hourly like every Tier-H table
/// (no refresh gate). The upgrades phase cost is anchor-charged to the
/// groups slot, matching the read-group convention.
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
/// Hourly like every Tier-H table (no refresh gate). The shared manager
/// header is anchor-charged to the world slot, matching the read-group
/// convention.
pub const SESSION: [TableId; 3] = [
    TableId::WorldTension,
    TableId::ThreatSources,
    TableId::ThreatCountryAggregates,
];

/// Wing-fed air pair in calibration/report order: [equipment, cumulative].
/// Hourly like every Tier-H table (no refresh gate). Both consume owned rows
/// acquired by the shared map/extension/naval wing census.
pub const AIR_WING: [TableId; 2] = [TableId::WingEquipment, TableId::WingCumulative];

/// Hourly air-statistics table in calibration/report order: [statistics].
/// Hourly like every Tier-H table (no refresh gate). Stages its own
/// strategic-air census copies with no cross-family decoded roots.
pub const AIR_STATS: [TableId; 1] = [TableId::AirStatistics];

/// Shared hourly bases payload in calibration/report order: [bases,
/// registrations, aces]. Hourly like every Tier-H table (no refresh gate).
/// The wing/manager census is taken once into shared raw spans; the bases,
/// registrations and aces decodes all read the shared spans (registrations
/// rows live in the fog pack and decode from this same log).
pub const AIR_BASES: [TableId; 3] = [
    TableId::AirBases,
    TableId::AirBaseRegistrations,
    TableId::AirAces,
];

/// Shared hourly history payload in calibration/report order: [history,
/// damage]. Hourly like every Tier-H table (no refresh gate). The single
/// regional walk is anchor-charged to the history slot, matching the
/// read-group convention.
pub const AIR_HISTORY: [TableId; 2] = [TableId::AirRegionHistory, TableId::AirRegionDamage];
/// Hourly supremacy table in calibration/report order: [supremacy].
/// Hourly like every Tier-H table (no refresh gate).
pub const SUPREMACY: [TableId; 1] = [TableId::NavalSupremacy];

/// Hourly sunk-ships table in calibration/report order: [sunk ships].
/// Hourly like every Tier-H table (no refresh gate).
pub const SUNK: [TableId; 1] = [TableId::SunkShips];

/// Hourly radar table in calibration/report order: [radar]. Hourly like
/// every Tier-H table (no refresh gate).
pub const RADAR: [TableId; 1] = [TableId::RadarCoverage];

/// Hourly intel table in calibration/report order: [intel]. Hourly like
/// every Tier-H table (no refresh gate). The division/fleet census spans
/// are the walk's own takes, shared once inside the payload between the
/// division and fleet paths.
pub const INTEL: [TableId; 1] = [TableId::UnitIntel];

/// Source-fed naval block in calibration/report order: [land theatres, land
/// groups, land members, air theatres, air groups, air members, orders
/// groups, orders members, army members]. Hourly like every Tier-H table (no
/// refresh gate). The four non-source-fed theatres move with the five
/// source-fed tables because they share the theatre/OOB walks. Shared
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

/// Production hourly rest in calibration/report order: [lines, stockpiles,
/// licences, conveyors, lines, manpower, equipment, records]. Hourly like
/// every Tier-H table (no refresh gate). The deployment shared traversal is
/// anchor-charged to the conveyors slot; the licence walk to the licences
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

/// Construction hourly rest in calibration/report order: [resources,
/// building, railway, settings, laws, garrisons, assigned, reserve,
/// resistance, collaboration]. Hourly like every Tier-H table (no refresh
/// gate). The construction queue is anchor-charged to the building slot; the
/// occupation shared walk to the settings slot, matching the read-group
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

/// Economy hourly rest in calibration/report order: [fuel, experience,
/// supply, hubs, disrupted, foreign, definitions, resources, exchanges,
/// purchases]. Hourly like every Tier-H table (no refresh gate). The supply
/// shared traversal is anchor-charged to the hubs slot; the resource
/// dictionary to the definitions slot, matching the read-group convention.
pub const ECONOMY_REST: [TableId; 10] = [
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
];

/// Hourly weather family in calibration/report order: [limits, regions,
/// provinces]. Hourly like every Tier-H table (no refresh gate). The manager
/// header is anchor-charged to the limits slot, matching the read-group
/// convention.
pub const WEATHER: [TableId; 3] = [
    TableId::WeatherLimits,
    TableId::WeatherRegions,
    TableId::WeatherProvinces,
];

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
    pub relations_refresh: bool,
    pub characters_refresh: bool,
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
    pub characters_payload: Option<CharactersJob>,
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
// Worker-only hits, misses, ineligible, comparisons, installs and their costs.
static AIR_STATS_MEMO: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
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
static ECONOMY_REST_COPY_NS: [AtomicU64; 10] = [
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
static ECONOMY_REST_DECODE_NS: [AtomicU64; 10] = [
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
static ECONOMY_REST_RAW_BYTES: [AtomicU64; 10] = [
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
static WEATHER_COPY_NS: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static WEATHER_DECODE_NS: [AtomicU64; 3] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static WEATHER_RAW_BYTES: [AtomicU64; 3] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
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
static RELATIONS_JOB_POOL: Mutex<Vec<RelationsJob>> = Mutex::new(Vec::new());
static CHARACTERS_JOB_POOL: Mutex<Vec<CharactersJob>> = Mutex::new(Vec::new());
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

/// Relations refresh gate: the whole family shares stagger slot 1 (`H%4==1`);
/// branch starts and keyframes force a full refresh like every other table.
pub fn relations_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay::cadence::should_walk(RELATIONS[0], hour, force_full);
    debug_assert_eq!(
        walk,
        replay::cadence::should_walk(RELATIONS[1], hour, force_full)
    );
    walk
}

/// Characters refresh gate: the slot-0 family (tables 0..6 sans hourly
/// ActiveAbilities) shares stagger slot 0 (`H%4==0`); branch starts and
/// keyframes force a full refresh like every other table.
pub fn characters_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay::cadence::should_walk(CHARACTERS[0], hour, force_full);
    for table in CHARACTERS.iter().skip(1) {
        debug_assert_eq!(walk, replay::cadence::should_walk(*table, hour, force_full));
    }
    walk
}

/// Projects refresh gate: the slot-3 project family shares stagger slot 3
/// (`H%4==3`); branch starts and keyframes force a full refresh like every
/// other table.
pub fn projects_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay::cadence::should_walk(PROJECTS[0], hour, force_full);
    for table in PROJECTS.iter().skip(1) {
        debug_assert_eq!(walk, replay::cadence::should_walk(*table, hour, force_full));
    }
    walk
}

/// Lexical refresh gate: the single dictionary shares stagger slot 3
/// (`H%4==3`); branch starts and keyframes force a full refresh like every
/// other table.
pub fn lexical_refresh_hour(hour: i32, force_full: bool) -> bool {
    replay::cadence::should_walk(LEXICAL[0], hour, force_full)
}

/// Monotonic frame sequence for FIFO order proofs. Gaps in the sequence are
/// dropped frames; order within the sequence is queue order.
pub fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Recycled relations raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per refresh.
pub fn acquire_relations_job() -> RelationsJob {
    if let Ok(mut pool) = RELATIONS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    RelationsJob::default()
}

/// Return a relations payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
pub fn recycle_relations_job(job: RelationsJob) {
    defer_payload(RetiredPayload::Relations(job));
}

fn release_relations_job(mut job: RelationsJob) {
    if job.countries.capacity() == 0 {
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

/// Recycled characters raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per refresh.
pub fn acquire_characters_job() -> CharactersJob {
    if let Ok(mut pool) = CHARACTERS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    CharactersJob::default()
}

/// Return a characters payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
pub fn recycle_characters_job(job: CharactersJob) {
    defer_payload(RetiredPayload::Characters(job));
}

fn release_characters_job(mut job: CharactersJob) {
    if job.log.spans.capacity() == 0 {
        return;
    }
    crate::raw::note_growth("characters", &job.log);
    if job.grow_for_next() {
        job.grow();
    }
    job.clear();
    if let Ok(mut pool) = CHARACTERS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled projects raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per refresh.
pub fn acquire_projects_job() -> ProjectsJob {
    if let Ok(mut pool) = PROJECTS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ProjectsJob::default()
}

/// Return a projects payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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
    job.clear();
    if let Ok(mut pool) = PROJECTS_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled lexical raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per refresh.
pub fn acquire_lexical_job() -> LexicalJob {
    if let Ok(mut pool) = LEXICAL_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    LexicalJob::default()
}

/// Return a lexical payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

/// Recycled divisions-group raw-payload job. Cold calls allocate; steady
/// state reuses jobs with their capacities, so neither thread allocates
/// per hour.
pub fn acquire_divisions_job() -> DivisionsJob {
    if let Ok(mut pool) = DIVISIONS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    DivisionsJob::default()
}

/// Return a divisions payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

/// Recycled hourly owned Focus rows and tokens. Pool exhaustion returns an
/// empty refusal; allocation and growth occur only on the decode side.
pub fn acquire_focus_job() -> FocusJob {
    if let Ok(mut pool) = FOCUS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    FocusJob::default()
}

/// Retire Focus storage without clearing, growing or freeing it on the callback.
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

/// Recycled hourly owned wing payload. Pool exhaustion returns an empty job;
/// acquisition never allocates and storage is prepared off-thread.
pub fn acquire_air_wing_job() -> AirWingJob {
    if let Ok(mut pool) = AIR_WING_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    AirWingJob::default()
}

/// Return a wing-pack payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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
    job.retire_witness_if_complete();
    job.clear();
    if let Ok(mut pool) = AIR_WING_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled hourly statistics raw-payload job. Cold calls allocate; steady
/// state reuses jobs with their capacities, so neither thread allocates
/// per hour.
pub fn acquire_air_stats_job() -> AirStatsJob {
    if let Ok(mut pool) = AIR_STATS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    AirStatsJob::default()
}

/// Return a statistics payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

/// Recycled hourly naval-pack raw-payload job. Cold calls allocate; steady
/// state reuses jobs with their capacities, so neither thread allocates
/// per hour.
pub fn acquire_naval_job() -> NavalJob {
    if let Ok(mut pool) = NAVAL_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    NavalJob::default()
}

/// Return a naval-pack payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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
    job.clear();
    if let Ok(mut pool) = NAVAL_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled hourly players-pair raw-payload job. Cold calls allocate; steady
/// state reuses jobs with their capacities, so neither thread allocates
/// per hour.
pub fn acquire_players_job() -> PlayersJob {
    if let Ok(mut pool) = PLAYERS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    PlayersJob::default()
}

/// Return a players-pair payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

/// Recycled hourly session-family raw-payload job. Cold calls allocate; steady
/// state reuses jobs with their capacities, so neither thread allocates
/// per hour.
pub fn acquire_session_job() -> SessionJob {
    if let Ok(mut pool) = SESSION_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SessionJob::default()
}

/// Return a session-family payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

/// Recycled hourly production-rest raw-payload job. Cold calls allocate;
/// steady state reuses jobs with their capacities, so neither thread
/// allocates per hour.
pub fn acquire_production_rest_job() -> ProductionRestJob {
    if let Ok(mut pool) = PRODUCTION_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ProductionRestJob::default()
}

/// Return a production-rest payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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
    job.clear();
    if let Ok(mut pool) = PRODUCTION_REST_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled hourly construction-rest raw-payload job. Cold calls allocate;
/// steady state reuses jobs with their capacities, so neither thread
/// allocates per hour.
pub fn acquire_construction_rest_job() -> ConstructionRestJob {
    if let Ok(mut pool) = CONSTRUCTION_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ConstructionRestJob::default()
}

/// Return a construction-rest payload job to the pool after clearing it.
/// Growth decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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
    job.clear();
    if let Ok(mut pool) = CONSTRUCTION_REST_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled hourly economy-rest raw-payload job. Cold calls allocate; steady
/// state reuses jobs with their capacities, so neither thread allocates
/// per hour.
pub fn acquire_economy_rest_job() -> EconomyRestJob {
    if let Ok(mut pool) = ECONOMY_REST_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    EconomyRestJob::default()
}

/// Return an economy-rest payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

/// Recycled hourly shared-bases raw-payload job. Cold calls allocate; steady
/// state reuses jobs with their capacities, so neither thread allocates
/// per hour.
pub fn acquire_air_bases_job() -> AirBasesJob {
    if let Ok(mut pool) = AIR_BASES_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    AirBasesJob::default()
}

/// Return a shared-bases payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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
    job.clear();
    if let Ok(mut pool) = AIR_BASES_JOB_POOL.lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled hourly owned history/damage rows. Missing pooled storage returns
/// an empty preflight sentinel without allocating on the game thread.
pub fn acquire_air_history_job() -> AirHistoryJob {
    if let Ok(mut pool) = AIR_HISTORY_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    AirHistoryJob::default()
}

/// Retire owned rows/witness without clearing, growing or freeing game-side;
/// the decode worker grows and returns this storage to its pool.
pub fn recycle_air_history_job(job: AirHistoryJob) {
    defer_payload(RetiredPayload::AirHistory(job));
}

fn release_air_history_job(mut job: AirHistoryJob) {
    job.publish_failure();
    if !job.has_storage() && !job.grow_for_next() {
        return;
    }
    let checks_complete = job.checks_complete();
    if job.grow_for_next() {
        job.grow();
    }
    if checks_complete {
        job.retire_witness();
    }
    job.clear();
    if let Ok(mut pool) = AIR_HISTORY_JOB_POOL.lock() {
        if checks_complete {
            for idle in pool.iter_mut() {
                idle.retire_witness();
            }
        }
        if pool.len() < JOB_POOL_KEEP + 1 {
            pool.push(job);
        }
    }
}

/// Recycled hourly supremacy raw-payload job. Cold calls allocate; steady
/// state reuses jobs with their capacities, so neither thread allocates
/// per hour.
pub fn acquire_supremacy_job() -> SupremacyJob {
    if let Ok(mut pool) = SUPREMACY_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SupremacyJob::default()
}

/// Return a supremacy payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

/// Recycled hourly sunk-ships raw-payload job. Cold calls allocate; steady
/// state reuses jobs with their capacities, so neither thread allocates
/// per hour.
pub fn acquire_sunk_job() -> SunkJob {
    if let Ok(mut pool) = SUNK_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    SunkJob::default()
}

/// Return a sunk-ships payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

/// Recycled hourly radar raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per hour.
pub fn acquire_radar_job() -> RadarJob {
    if let Ok(mut pool) = RADAR_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    RadarJob::default()
}

/// Return a radar payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

/// Recycled hourly intel raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per hour.
pub fn acquire_intel_job() -> IntelJob {
    if let Ok(mut pool) = INTEL_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    IntelJob::default()
}

/// Return an intel payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

/// Recycled hourly weather raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per hour.
pub fn acquire_weather_job() -> WeatherJob {
    if let Ok(mut pool) = WEATHER_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    WeatherJob::default()
}

/// Return a weather payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

/// Recycled hourly capitulation raw-payload job. Cold calls allocate; steady
/// state reuses jobs with their capacities, so neither thread allocates per
/// hour.
pub fn acquire_capitulation_job() -> CapitulationJob {
    if let Ok(mut pool) = CAPITULATION_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    CapitulationJob::default()
}

/// Return a capitulation payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
/// Retire raw storage without clearing, growing or freeing it on the game thread.
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

const PAYLOAD_FAMILIES: usize = 22;
// A missing comparison buffer retires the original first job, then the
// failed empty replacement and empty second job: three per family/frame.
// Every frame transits the worker, which drains retirement before forwarding
// another frame. At most BUFFERS owned frames can contribute between drains.
const MAX_RETIRED_PER_FAMILY_PER_FRAME: usize = 3;
const RETIREMENT_CAPACITY: usize = BUFFERS * PAYLOAD_FAMILIES * MAX_RETIRED_PER_FAMILY_PER_FRAME;

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

/// Acquisition scratch is shared across frames and retained through every ETA
/// phase. Only the decode worker grows returned scratch; callback reuse is bounded.
struct EtaWorkspaces {
    ready: ArrayQueue<Box<EtaWorkspace>>,
    retired: ArrayQueue<Box<EtaWorkspace>>,
}
static ETA_WORKSPACES: AtomicPtr<EtaWorkspaces> = AtomicPtr::new(std::ptr::null_mut());
static ETA_WORKSPACE_BYTES: AtomicU64 = AtomicU64::new(0);

fn prepare_eta_workspaces() {
    if !ETA_WORKSPACES.load(Ordering::Acquire).is_null() {
        return;
    }
    let pool = Box::new(EtaWorkspaces {
        ready: ArrayQueue::new(division_eta::MAX_RANGES),
        retired: ArrayQueue::new(division_eta::MAX_RANGES),
    });
    // crossbeam-queue's pinned ArrayQueue slot holds one u64 stamp and one
    // pointer. Include both fixed queue buffers and each workspace's inline data.
    let mut bytes = std::mem::size_of::<EtaWorkspaces>()
        + 2 * division_eta::MAX_RANGES * std::mem::size_of::<(u64, Box<EtaWorkspace>)>();
    for _ in 0..division_eta::MAX_RANGES {
        let workspace = Box::new(EtaWorkspace::with_capacities(
            &division_eta::Capacities::default(),
            true,
        ));
        bytes += std::mem::size_of::<EtaWorkspace>() + workspace.allocated_bytes();
        assert!(pool.ready.push(workspace).is_ok());
    }
    let pool = Box::into_raw(pool);
    if ETA_WORKSPACES
        .compare_exchange(
            std::ptr::null_mut(),
            pool,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
    {
        ETA_WORKSPACE_BYTES.store(bytes as u64, Ordering::Relaxed);
    } else {
        // This unpublished pool has no callback or maintenance borrowers.
        unsafe {
            drop(Box::from_raw(pool));
        }
    }
}

pub(crate) fn eta_workspace_bytes() -> u64 {
    ETA_WORKSPACE_BYTES.load(Ordering::Relaxed)
}

pub(crate) fn acquire_eta_workspace() -> Option<Box<EtaWorkspace>> {
    let pool = ETA_WORKSPACES.load(Ordering::Acquire);
    if pool.is_null() {
        return None;
    }
    // Pool publication precedes hook installation; destruction follows quiescence.
    let pool = unsafe { &*pool };
    pool.ready
        .pop()
        .or_else(|| pool.retired.pop())
        .or_else(|| pool.ready.pop())
}

pub(crate) fn recycle_eta_workspace(workspace: Box<EtaWorkspace>) {
    let pool = ETA_WORKSPACES.load(Ordering::Acquire);
    assert!(
        !pool.is_null(),
        "ETA workspace pool initialized before hook"
    );
    // Exactly MAX_RANGES workspaces exist, including all outstanding leases.
    assert!(
        unsafe { &*pool }.retired.push(workspace).is_ok(),
        "ETA workspace retirement bound exceeded"
    );
}

fn reclaim_eta_workspaces() {
    let pool = ETA_WORKSPACES.load(Ordering::Acquire);
    if pool.is_null() {
        return;
    }
    // Only the decode worker or the quiescent loader performs maintenance.
    let pool = unsafe { &*pool };
    for _ in 0..division_eta::MAX_RANGES {
        let Some(mut workspace) = pool.retired.pop() else {
            break;
        };
        let before = workspace.allocated_bytes() as u64;
        workspace.prepare_for_reuse();
        let after = workspace.allocated_bytes() as u64;
        let shared = sampling::SHARED.load(Ordering::Acquire);
        if after >= before {
            ETA_WORKSPACE_BYTES.fetch_add(after - before, Ordering::Relaxed);
            if !shared.is_null() {
                unsafe { &*shared }
                    .pool_bytes
                    .fetch_add(after - before, Ordering::Relaxed);
            }
        } else {
            ETA_WORKSPACE_BYTES.fetch_sub(before - after, Ordering::Relaxed);
            if !shared.is_null() {
                unsafe { &*shared }
                    .pool_bytes
                    .fetch_sub(before - after, Ordering::Relaxed);
            }
        }
        // Publish this workspace before taking another: maintenance holds at
        // most one, leaving the callback at least three available range slots.
        assert!(
            pool.ready.push(workspace).is_ok(),
            "ETA ready workspace bound exceeded"
        );
    }
}

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

        let workspaces = ETA_WORKSPACES.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !workspaces.is_null() {
            // Callback and acquisition/decode workers have all quiesced.
            unsafe {
                drop(Box::from_raw(workspaces));
            }
        }
        ETA_WORKSPACE_BYTES.store(0, Ordering::Relaxed);

        if let Ok(mut pool) = RELATIONS_JOB_POOL.lock() {
            *pool = Vec::new();
        }
        if let Ok(mut pool) = CHARACTERS_JOB_POOL.lock() {
            *pool = Vec::new();
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

pub fn prepare_payload_pools() -> PayloadPools {
    prepare_retirement();
    prepare_eta_workspaces();
    for _ in 0..JOB_POOL_KEEP + 1 {
        release_relations_job(RelationsJob::new());
        release_characters_job(CharactersJob::new());
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
    reclaim_eta_workspaces();
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
    let mut air_stats_memo = AirStatsMemo::default();
    loop {
        reclaim_payloads();
        if STOP.load(Ordering::Acquire) {
            break;
        }
        let shared_ptr = sampling::SHARED.load(Ordering::Acquire);
        if shared_ptr.is_null() {
            std::thread::yield_now();
            continue;
        }
        let shared = unsafe { &*shared_ptr };
        match shared.decode_queue.pop() {
            Some(job) => process_owned_job(shared, job, &mut air_bases_memo, &mut air_stats_memo),
            None => std::thread::sleep(std::time::Duration::from_millis(1)),
        }
    }
}

/// The worker owns the raw payloads through decoding, then returns every
/// family to its pool. Dropping this envelope without recycling used to
/// allocate a fresh payload on the game thread on every subsequent hour.
fn process_owned_job(
    shared: &Shared,
    mut job: DecodeJob,
    air_bases_memo: &mut AirBasesMemo,
    air_stats_memo: &mut AirStatsMemo,
) {
    process_job(shared, &mut job, air_bases_memo, air_stats_memo);

    if let Some(payload) = job.relations_payload.take() {
        release_relations_job(payload);
    }
    if let Some(payload) = job.characters_payload.take() {
        release_characters_job(payload);
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

/// Decode one FIFO item and hand its pooled frame to the spool ready queue
/// in FIFO hour order. Skip items forward without touching typed rows.
fn process_job(
    shared: &Shared,
    job: &mut DecodeJob,
    air_bases_memo: &mut AirBasesMemo,
    air_stats_memo: &mut AirStatsMemo,
) {
    if !job.air_stats_refresh || job.air_stats_payload.is_none() {
        air_stats_memo.invalidate();
    }
    if !job.air_bases_refresh || job.air_bases_payload.is_none() {
        air_bases_memo.invalidate();
    }
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
        frame.extended.relations_pilot().decode_relations_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in RELATIONS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.characters_refresh
        && let Some(payload) = job.characters_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 6];
        frame.extended.characters_pilot().decode_characters_payload(
            shared.base,
            payload,
            &mut decode_ticks,
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
        frame.extended.lexical_pilot().decode_lexical_payload(
            shared.base,
            payload,
            &mut decode_ticks,
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
        sampling::decode_divisions_payload(frame, shared.base, payload, &mut decode_ticks);
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
            shared.base,
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
        // Local statistics flags reset during extended capture. Use the
        // authoritative current frame's window/verdict for memo eligibility.
        let index = AIR_STATS[0].index();
        let validation_allowed =
            !frame.extended_checked[index] && !frame.extended_raw_mismatches[index];
        let report = air_stats_memo.decode(
            frame.extended.air_stats_pilot(),
            payload,
            AirStatsMemoContext {
                owner: shared as *const Shared as usize,
                base: shared.base,
                seq: job.seq,
                hour: job.hour,
                force_full: job.force_full,
                validation_allowed,
            },
            &mut decode_ticks,
        );
        for (cell, value) in AIR_STATS_MEMO.iter().zip([
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
        frame
            .extended
            .construction_rest_pilot()
            .decode_construction_rest_payload(
                shared.base,
                payload.game_state,
                payload,
                &mut decode_ticks,
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
        let mut decode_ticks = [0; 10];
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
        // The capture packs reset their local check fields after raw staging.
        // Only these per-frame verdicts describe the current validation window.
        let validation_allowed = AIR_BASES.iter().all(|table| {
            !frame.extended_checked[table.index()] && !frame.extended_raw_mismatches[table.index()]
        });
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
                validation_allowed,
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
            .decode_air_history_payload(shared.base, payload, &mut decode_ticks);
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
            .decode_radar_payload(shared.base, payload, &mut decode_ticks);
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
            .decode_intel_payload(shared.base, payload, &mut decode_ticks);
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
        let mut decode_ticks = [0; 3];
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
    // ready slot and the push below always has room while the spool thread
    // lives. The worker may wait (only the game thread must never block).
    while shared.ready.push(job.index).is_err() {
        std::thread::yield_now();
    }
    shared.completed[job.index].store(job.game_elapsed.max(1), Ordering::Release);
}

/// Detach drain (runtime only, after callback quiescence): stop the worker,
/// join it, then synchronously decode any remaining FIFO items in order and
/// hand them to the spool ready queue before the existing spool drain runs.
/// The spool thread is still consuming, so handoffs make progress.
pub fn detach_drain() {
    STOP.store(true, Ordering::Release);
    if let Ok(mut slot) = WORKER.lock()
        && let Some(handle) = slot.take()
    {
        let _ = handle.join();
    }
    WORKER_ID.store(0, Ordering::Release);
    let shared_ptr = sampling::SHARED.load(Ordering::Acquire);
    if shared_ptr.is_null() {
        STOP.store(false, Ordering::Release);
        return;
    }
    let shared = unsafe { &*shared_ptr };
    // The joined worker drops its memo. Drain owns one fresh entry and cannot
    // race or retain references into worker/ready-queue storage.
    let mut air_bases_memo = AirBasesMemo::default();
    let mut air_stats_memo = AirStatsMemo::default();
    while let Some(job) = shared.decode_queue.pop() {
        process_owned_job(shared, job, &mut air_bases_memo, &mut air_stats_memo);
    }
    STOP.store(false, Ordering::Release);
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
    pub economy_rest_copy_ns: [u64; 10],
    pub economy_rest_decode_ns: [u64; 10],
    pub economy_rest_raw_bytes: [u64; 10],
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
    pub weather_copy_ns: [u64; 3],
    pub weather_decode_ns: [u64; 3],
    pub weather_raw_bytes: [u64; 3],
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
            "air_stats_memo": {
                "hits": AIR_STATS_MEMO[0].load(Ordering::Relaxed),
                "misses": AIR_STATS_MEMO[1].load(Ordering::Relaxed),
                "ineligible": AIR_STATS_MEMO[2].load(Ordering::Relaxed),
                "comparisons": AIR_STATS_MEMO[3].load(Ordering::Relaxed),
                "installs": AIR_STATS_MEMO[4].load(Ordering::Relaxed),
                "compare_ns": AIR_STATS_MEMO[5].load(Ordering::Relaxed),
                "restore_ns": AIR_STATS_MEMO[6].load(Ordering::Relaxed),
                "install_ns": AIR_STATS_MEMO[7].load(Ordering::Relaxed),
                "memory": crate::extended::air::air_stats_memo_memory(),
            },
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
            "economy_rest_tables": ["CountryFuel", "CountryExperience", "ProvinceSupply", "HubMotorization", "DisruptedSupply", "ForeignHomebases", "ResourceDefinitions", "CountryResources", "ResourceExchanges", "PurchaseContracts"],
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
            "weather_tables": ["WeatherLimits", "WeatherRegions", "WeatherProvinces"],
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
    calibration().json()
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
        .chain(AIR_STATS_MEMO.iter())
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
    fn retirement_capacity_covers_missing_second_buffers_without_reclamation() {
        // With three seeded buffers and no worker progress, the first
        // activation retires only its second job. Every later activation
        // retires original-first, empty replacement, and empty-second.
        // Even the 18 hourly families alone exceed the old two-per-frame
        // bound before all frame buffers have been consumed.
        let hourly_families = 18;
        let seeded_schedule = hourly_families * (1 + 3 * (BUFFERS - 1));
        assert!(seeded_schedule > BUFFERS * PAYLOAD_FAMILIES * 2);
        assert!(seeded_schedule <= RETIREMENT_CAPACITY);

        // Exercise the exact queue bound with byte-sized event tags, not
        // thousands of large RetiredPayload values or payload allocations.
        // Depleted pools or lock contention permit all three events from
        // every family of every owned frame before any reclamation.
        let queue = ArrayQueue::new(RETIREMENT_CAPACITY);
        for _ in 0..BUFFERS {
            for _ in 0..PAYLOAD_FAMILIES {
                for event in [0_u8, 1, 2] {
                    assert!(queue.push(event).is_ok());
                }
            }
        }
        assert_eq!(queue.len(), RETIREMENT_CAPACITY);
        assert!(queue.push(3).is_err());
        for _ in 0..BUFFERS * PAYLOAD_FAMILIES {
            assert_eq!(queue.pop(), Some(0));
            assert_eq!(queue.pop(), Some(1));
            assert_eq!(queue.pop(), Some(2));
        }
        assert!(queue.is_empty());
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
        CHARACTERS_JOB_POOL.lock().unwrap().clear();
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
            characters_payload: Some(CharactersJob::new()),
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
        process_owned_job(
            &shared,
            job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 17);
        assert_eq!(
            RELATIONS_JOB_POOL.lock().unwrap().len(),
            1,
            "RELATIONS_JOB_POOL"
        );
        assert_eq!(
            CHARACTERS_JOB_POOL.lock().unwrap().len(),
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
    fn relations_gate_matches_stagger_slot_one_and_force_full() {
        // Slot 1 refreshes exactly on H%4==1; branch starts and keyframes
        // force a full refresh regardless of hour.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 1;
            assert_eq!(relations_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(relations_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn characters_gate_matches_stagger_slot_zero_and_force_full() {
        // Slot 0 refreshes exactly on H%4==0; branch starts and keyframes
        // force a full refresh regardless of hour.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 0;
            assert_eq!(characters_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(characters_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn projects_gate_matches_stagger_slot_three_and_force_full() {
        // Slot 3 refreshes exactly on H%4==3; branch starts and keyframes
        // force a full refresh regardless of hour.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 3;
            assert_eq!(projects_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(projects_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn lexical_gate_matches_stagger_slot_three_and_force_full() {
        // The dictionary shares slot 3 like the other slot-3 families.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 3;
            assert_eq!(lexical_refresh_hour(hour, false), refresh, "hour {hour}");
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
                characters_refresh: false,
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
                characters_payload: None,
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
            process_job(
                &shared,
                &mut job,
                &mut AirBasesMemo::default(),
                &mut AirStatsMemo::default(),
            );
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
                characters_refresh: false,
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
                characters_payload: None,
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
            characters_refresh: false,
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
            characters_payload: None,
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
        let (report, first) =
            relations_test_kit::split(&memory, frame.extended.relations_pilot(), true);
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 1,
            relations_refresh: true,
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 17);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        relations_test_kit::assert_matches_direct(frame.extended.relations_pilot(), &direct);
    }

    #[test]
    fn worker_decodes_a_characters_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::characters::characters_test_kit;
        use crate::extended::characters::{CharactersCopyContext, CharactersJob};
        use replay::frame::TABLE_COUNT;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = characters_test_kit::mapped_fixture();
        let memory = characters_test_kit::memory(&bytes);
        let shared = Shared::new(characters_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let capacities = crate::extended::characters::Capacities {
            rows: [4096; 7],
            bytes: [131_072; 7],
        };
        let direct = characters_test_kit::direct(&memory, &capacities);
        let (trait_rows, trait_bytes) = direct.trait_capture_capacities();
        let ctx = CharactersCopyContext {
            base: characters_test_kit::BASE,
            game_state: characters_test_kit::GAME_STATE,
            trait_rows,
            trait_bytes,
        };
        let mut first = CharactersJob::new();
        let mut second = CharactersJob::new();
        let report = crate::extended::characters::copy_characters_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        frame
            .extended
            .characters_pilot()
            .apply_characters_report(&[true; TABLE_COUNT], &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 0,
            relations_refresh: false,
            characters_refresh: true,
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
            characters_payload: Some(first),
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 19);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        characters_test_kit::assert_matches_direct(frame.extended.characters_pilot(), &direct);
        // The worker clears the raw-mode gate it consumed.
        assert!(!frame.extended.characters_pilot().raw_mode);
    }

    #[test]
    fn worker_decodes_a_projects_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::projects::projects_test_kit;
        use crate::extended::projects::{ProjectsCopyContext, ProjectsJob};
        use replay::frame::TABLE_COUNT;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = projects_test_kit::fixture();
        let memory = projects_test_kit::memory(&bytes);
        let shared = Shared::new(projects_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let capacities = crate::extended::projects::Capacities {
            rows: [256; 10],
            program_names: 8192,
            program_candidates: 1024,
        };
        let direct = projects_test_kit::direct(&memory, &capacities);
        let ctx = ProjectsCopyContext {
            base: projects_test_kit::BASE,
            game_state: projects_test_kit::GAME_STATE,
            needed: [true; TABLE_COUNT],
        };
        let mut first = ProjectsJob::new();
        let mut second = ProjectsJob::new();
        let report = crate::extended::projects::copy_projects_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        frame
            .extended
            .projects_pilot()
            .apply_projects_report(ctx, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 4]);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 3,
            relations_refresh: false,
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
        use replay::frame::TABLE_COUNT;
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
            needed: [true; TABLE_COUNT],
        };
        let mut first = LexicalJob::new();
        let mut second = LexicalJob::new();
        let report = crate::extended::lexical::copy_lexical_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        frame
            .extended
            .lexical_pilot()
            .apply_lexical_report(ctx, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true]);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 3,
            relations_refresh: false,
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 29);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        lexical_test_kit::assert_matches_direct(frame.extended.lexical_pilot(), &direct);
        // The worker clears the raw-mode gate it consumed.
        assert!(!frame.extended.lexical_pilot().raw_mode);
    }

    #[test]
    fn worker_preserves_already_typed_wars_rows_and_independent_matrix() {
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
            frame.war_matrix_result = Some(direct.matrix_outcome);
            frame.wars[..direct.owned.len()].copy_from_slice(&direct.owned);
            frame.wars_matrix[..direct.matrix.len()].copy_from_slice(&direct.matrix);
            frame.table_ns[TableId::WarRelations.index()] = 601;
            frame.war_matrix_ns = 602;
        }
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 41,
            relations_refresh: false,
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 43);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(frame.wars_result, direct.owned_outcome);
        assert_eq!(frame.war_matrix_result, Some(direct.matrix_outcome));
        assert_eq!(&frame.wars[..direct.owned.len()], &direct.owned[..]);
        assert_eq!(
            &frame.wars_matrix[..direct.matrix.len()],
            &direct.matrix[..]
        );
        assert_eq!(frame.table_ns[TableId::WarRelations.index()], 601);
        assert_eq!(frame.war_matrix_ns, 602);
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
            detail_capacities: [64; 5],
        };
        let mut first = DivisionsJob::new();
        let mut second = DivisionsJob::new();
        let report = copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        // The pooled frame holds more rows than the fixture needs; the
        // replay truncates identically against the real buffers.
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 42,
            relations_refresh: false,
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 47);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
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
        // Divisions decode is hourly with no refresh gate.
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
        assert!(relations.slot_pool.capacity() >= 8639 * 512);
        assert!(relations.tuples.capacity() >= 1512 * 4);
        assert!(relations.countries.capacity() >= 439);
        assert!(relations.metas.capacity() >= 439);
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
        assert_eq!(BUFFERS, 256);
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
                characters_refresh: false,
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
                characters_payload: None,
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
    fn worker_decodes_an_hourly_focus_payload_into_the_same_typed_rows() {
        use crate::extended::governance::focus_test_kit;
        use replay::frame::TABLE_COUNT;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let fixture = focus_test_kit::Fixture::world();
        let memory = fixture.memory();
        let shared = Shared::new(focus_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let needed = [true; TABLE_COUNT];
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let mut typed = focus_test_kit::Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        frame.extended.focus_pilot().focus_raw_mode = true;
        let mut comparison = FocusJob::new();
        let (report, first) = focus_test_kit::Fixture::copy_first(
            frame.extended.focus_pilot(),
            &memory,
            Some(&mut comparison),
        );
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true, true]);
        assert_eq!(report.mismatches, [0, 0]);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 7,
            relations_refresh: false,
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
            needed: [true; 2],
            limits: [1024, 8192, 4096],
            naval_limits: [4096; 2],
        };
        let mut first = acquire_air_wing_job();
        let report = crate::extended::air::copy_air_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 9,
            relations_refresh: false,
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
        };
        let mut first = acquire_air_stats_job();
        let report = crate::extended::air::copy_air_stats_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 9,
            relations_refresh: false,
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
        };
        let mut first = acquire_naval_job();
        naval_test_kit::stage_wings(&memory, &mut first);
        let report =
            crate::extended::naval_history::copy_naval_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 9,
            relations_refresh: false,
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
            needed: [true; 2],
            limits: [1024, 8192, 4096],
            naval_limits: [4096; 2],
        };
        let air_report =
            crate::extended::air::copy_air_refresh(&air_memory, air_ctx, &mut air_job, None);
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
        };
        naval_kit::stage_wings(&naval_memory, &mut naval_job);
        let naval_report = crate::extended::naval_history::copy_naval_refresh(
            &naval_memory,
            naval_ctx,
            &mut naval_job,
            None,
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
            needed: [true; 8],
        };
        let mut first = acquire_production_rest_job();
        let mut second = acquire_production_rest_job();
        let report = crate::extended::production::copy_production_rest_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        frame
            .extended
            .production_rest_pilot()
            .apply_production_rest_report(ctx.needed, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 8]);
        recycle_production_rest_job(second);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 11,
            relations_refresh: false,
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
            needed: [true; 10],
        };
        let mut first = acquire_construction_rest_job();
        let mut second = acquire_construction_rest_job();
        let report = crate::extended::construction::copy_construction_rest_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        frame
            .extended
            .construction_rest_pilot()
            .apply_construction_rest_report(ctx.needed, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 10]);
        recycle_construction_rest_job(second);
        shared.completed[index].store(0, Ordering::Relaxed);
        let mut job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 12,
            relations_refresh: false,
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
            needed: [true; 2],
            limits: [1024, 8192, 4096],
            naval_limits: [4096; 2],
        };
        let mut air_job = acquire_air_wing_job();
        let air_report =
            crate::extended::air::copy_air_refresh(&air_memory, air_ctx, &mut air_job, None);
        assert!(air_report.raw_ok);
        let naval_bytes = naval_kit::fixture();
        let naval_memory = naval_kit::memory(&naval_bytes);
        let naval_direct = naval_kit::direct(&naval_memory);
        let naval_ctx = NavalCopyContext {
            base: naval_kit::BASE,
            game_state: naval_kit::GAME_STATE,
        };
        let mut naval_job = acquire_naval_job();
        naval_kit::stage_wings(&naval_memory, &mut naval_job);
        let naval_report = crate::extended::naval_history::copy_naval_refresh(
            &naval_memory,
            naval_ctx,
            &mut naval_job,
            None,
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
            characters_refresh: false,
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
            characters_payload: None,
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
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
            characters_refresh: false,
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
            characters_payload: None,
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

    fn stage_stats_memo_frame(frame: &mut sampling::Captured, bytes: &[u8]) -> AirStatsJob {
        use crate::extended::air::{AirStatsCopyContext, stats_test_kit as kit};
        let mut payload = AirStatsJob::new();
        let memory = kit::memory(bytes);
        let report = crate::extended::air::copy_air_stats_refresh(
            &memory,
            AirStatsCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
            },
            &mut payload,
            None,
        );
        assert!(report.raw_ok);
        let captured = frame.extended.air_stats_pilot();
        captured.stats_raw_mode = true;
        captured.bases_raw_mode = true;
        captured.history_raw_mode = true;
        // Execute the intervening real pack reset/raw-mode no-op. Authoritative
        // checked/mismatch flags live on the enclosing frame, not this pack.
        captured.capture(
            &memory,
            kit::BASE,
            kit::GAME_STATE,
            &[false; crate::extended::air::N],
        );
        assert_eq!(captured.checked_mismatches()[6], 0);
        payload
    }

    #[test]
    fn worker_air_stats_memo_obeys_authoritative_flags_after_capture_reset() {
        use crate::extended::air::stats_test_kit as kit;
        let _guard = TEST_LOCK.lock().unwrap();
        let bytes = kit::fixture();
        for mismatch in [false, true] {
            reset_calibration_for_test();
            let shared = Shared::new(kit::BASE, 0, 1);
            let index = shared.free.pop().expect("pooled buffer");
            let mut bases_memo = AirBasesMemo::default();
            let mut stats_memo = AirStatsMemo::default();
            for step in 1..=4 {
                let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
                frame.extended_checked.fill(false);
                frame.extended_raw_mismatches.fill(false);
                if step == 2 {
                    if mismatch {
                        frame.extended_raw_mismatches[AIR_STATS[0].index()] = true;
                    } else {
                        frame.extended_checked[AIR_STATS[0].index()] = true;
                    }
                }
                let checked = frame.extended_checked;
                let mismatches = frame.extended_raw_mismatches;
                let payload = stage_stats_memo_frame(frame, &bytes);
                let mut job = bare_job(index, step, step as i32 + 100, 41);
                job.air_stats_refresh = true;
                job.air_stats_payload = Some(payload);
                process_job(&shared, &mut job, &mut bases_memo, &mut stats_memo);
                assert_eq!(shared.ready.pop(), Some(index));
                let frame = unsafe { &*shared.frames[index].load(Ordering::Acquire) };
                assert_eq!(frame.extended_checked, checked);
                assert_eq!(frame.extended_raw_mismatches, mismatches);
                let diagnostics = calibration().json();
                let memo = &diagnostics["air_stats_memo"];
                assert_eq!(memo["hits"], u64::from(step == 4));
                assert_eq!(memo["ineligible"], u64::from(step >= 2));
                assert_eq!(memo["installs"], if step < 3 { 1 } else { 2 });
            }
        }
    }

    #[test]
    fn worker_air_stats_memo_invalidates_on_skip_and_missing_payload() {
        use crate::extended::air::stats_test_kit as kit;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let mut bases_memo = AirBasesMemo::default();
        let mut stats_memo = AirStatsMemo::default();
        let expected_hits = [0, 1, 1, 1, 2, 2, 2, 3];
        let expected_installs = [1, 1, 1, 2, 2, 2, 3, 3];
        for step in 1..=8 {
            let mut job = bare_job(index, step, step as i32 + 100, 41);
            job.air_stats_refresh = step != 3;
            if step != 3 && step != 6 {
                let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
                job.air_stats_payload = Some(stage_stats_memo_frame(frame, &bytes));
            }
            process_job(&shared, &mut job, &mut bases_memo, &mut stats_memo);
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.completed[index].load(Ordering::Acquire), 41);
            assert_eq!(
                AIR_STATS_MEMO[0].load(Ordering::Relaxed),
                expected_hits[step as usize - 1]
            );
            assert_eq!(
                AIR_STATS_MEMO[4].load(Ordering::Relaxed),
                expected_installs[step as usize - 1]
            );
        }
        let diagnostics = calibration().json();
        assert_eq!(diagnostics["air_stats_memo"]["hits"], 3);
        assert_eq!(diagnostics["air_stats_memo"]["misses"], 3);
        assert_eq!(diagnostics["air_stats_memo"]["installs"], 3);
    }

    #[test]
    fn worker_air_bases_memo_invalidates_on_skip_and_missing_payload() {
        use crate::extended::air::{AirBasesCopyContext, air_bases_test_kit as kit};
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = kit::fixture();
        let shared = Shared::new(kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let mut memo = AirBasesMemo::default();
        let expected_hits = [0, 1, 1, 1, 2, 2, 2, 3];
        let expected_installs = [1, 1, 1, 2, 2, 2, 3, 3];
        for step in 1..=8 {
            let mut job = bare_job(index, step, step as i32 + 100, 41);
            job.air_bases_refresh = step != 3;
            if step != 3 && step != 6 {
                let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
                let mut payload = AirBasesJob::new();
                let ctx = AirBasesCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    needed: [false; 3],
                };
                let report = crate::extended::air::copy_air_bases_refresh(
                    &kit::memory(&bytes),
                    ctx,
                    &mut payload,
                    None,
                );
                frame
                    .extended
                    .air_bases_pilot()
                    .apply_air_bases_report(ctx.needed, &report);
                frame
                    .extended
                    .radar_pilot()
                    .note_registrations_copy(ctx.needed[1], report.matched[1]);
                assert!(report.raw_ok);
                frame.extended.air_bases_pilot().bases_raw_mode = true;
                frame.extended.radar_pilot().registrations_raw_mode = true;
                job.air_bases_payload = Some(payload);
            }
            process_job(&shared, &mut job, &mut memo, &mut AirStatsMemo::default());
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
        }
        let diagnostics = calibration().json();
        assert_eq!(diagnostics["air_bases_memo"]["hits"], 3);
        assert_eq!(diagnostics["air_bases_memo"]["misses"], 3);
        assert_eq!(diagnostics["air_bases_memo"]["installs"], 3);
    }

    #[test]
    fn worker_air_bases_memo_obeys_authoritative_frame_checks() {
        use crate::extended::air::{AirBasesCopyContext, air_bases_test_kit as kit};
        let _guard = TEST_LOCK.lock().unwrap();
        let bytes = kit::fixture();
        for guarded_slot in 0..6 {
            reset_calibration_for_test();
            let shared = Shared::new(kit::BASE, 0, 1);
            let index = shared.free.pop().expect("pooled buffer");
            let mut memo = AirBasesMemo::default();
            for step in 1..=4 {
                let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
                frame.extended_checked.fill(false);
                frame.extended_raw_mismatches.fill(false);
                if step == 2 {
                    let table = AIR_BASES[guarded_slot % 3].index();
                    if guarded_slot < 3 {
                        frame.extended_checked[table] = true;
                    } else {
                        frame.extended_raw_mismatches[table] = true;
                    }
                }
                let checked = frame.extended_checked;
                let mismatches = frame.extended_raw_mismatches;
                let mut payload = AirBasesJob::new();
                let ctx = AirBasesCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    needed: [false; 3],
                };
                let report = crate::extended::air::copy_air_bases_refresh(
                    &kit::memory(&bytes),
                    ctx,
                    &mut payload,
                    None,
                );
                frame
                    .extended
                    .air_bases_pilot()
                    .apply_air_bases_report(ctx.needed, &report);
                frame
                    .extended
                    .radar_pilot()
                    .note_registrations_copy(ctx.needed[1], report.matched[1]);
                assert!(report.raw_ok);
                // Model the raw-mode capture path after local flags reset.
                frame.extended.air_bases_pilot().bases_raw_mode = true;
                frame.extended.radar_pilot().registrations_raw_mode = true;
                let mut job = bare_job(index, step, step as i32 + 100, 41);
                job.air_bases_refresh = true;
                job.air_bases_payload = Some(payload);
                process_job(&shared, &mut job, &mut memo, &mut AirStatsMemo::default());
                assert_eq!(shared.ready.pop(), Some(index));
                let frame = unsafe { &*shared.frames[index].load(Ordering::Acquire) };
                assert_eq!(frame.extended_checked, checked);
                assert_eq!(frame.extended_raw_mismatches, mismatches);
                let diagnostics = calibration().json();
                let memo = &diagnostics["air_bases_memo"];
                assert_eq!(memo["hits"], u64::from(step == 4));
                assert_eq!(memo["ineligible"], u64::from(step >= 2));
                assert_eq!(memo["installs"], if step < 3 { 1 } else { 2 });
            }
        }
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
            needed: [true; 3],
        };
        let mut first = acquire_air_bases_job();
        let mut second = acquire_air_bases_job();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let report = crate::extended::air::copy_air_bases_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        frame
            .extended
            .air_bases_pilot()
            .apply_air_bases_report(ctx.needed, &report);
        frame
            .extended
            .radar_pilot()
            .note_registrations_copy(ctx.needed[1], report.matched[1]);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 3]);
        recycle_air_bases_job(second);
        let mut job = bare_job(index, next_seq(), 7, 41);
        job.air_bases_refresh = true;
        job.air_bases_payload = Some(first);
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
            needed: [true; 2],
            history_limit,
            damage_limit,
        };
        let mut first = acquire_air_history_job();
        let mut second = acquire_air_history_job();
        let report = crate::extended::air::copy_air_history_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        frame
            .extended
            .air_history_pilot()
            .apply_air_history_report(ctx.needed, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 2]);
        recycle_air_history_job(second);
        let mut job = bare_job(index, next_seq(), 7, 43);
        job.air_history_refresh = true;
        job.air_history_payload = Some(first);
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
            needed: [true; 1],
        };
        let mut first = acquire_supremacy_job();
        let mut second = acquire_supremacy_job();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let report = crate::extended::naval_history::copy_supremacy_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        frame
            .extended
            .supremacy_pilot()
            .apply_supremacy_report(ctx.needed, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 1]);
        recycle_supremacy_job(second);
        let mut job = bare_job(index, next_seq(), 7, 53);
        job.supremacy_refresh = true;
        job.supremacy_payload = Some(first);
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
            needed: [true; 1],
        };
        let mut first = acquire_sunk_job();
        let mut second = acquire_sunk_job();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let report = crate::extended::naval_history::copy_sunk_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        frame
            .extended
            .sunk_pilot()
            .apply_sunk_report(ctx.needed, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 1]);
        recycle_sunk_job(second);
        let mut job = bare_job(index, next_seq(), 7, 59);
        job.sunk_refresh = true;
        job.sunk_payload = Some(first);
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
        let mut second = acquire_radar_job();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let ctx =
            frame
                .extended
                .radar_pilot()
                .radar_copy_context(kit::BASE, kit::GAME_STATE, [true; 1]);
        let report =
            crate::extended::fog::copy_radar_refresh(&memory, ctx, &mut first, Some(&mut second));
        frame
            .extended
            .radar_pilot()
            .apply_radar_report(ctx.needed, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 1]);
        recycle_radar_job(second);
        let mut job = bare_job(index, next_seq(), 7, 61);
        job.radar_refresh = true;
        job.radar_payload = Some(first);
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
        let mut second = acquire_intel_job();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let ctx =
            frame
                .extended
                .intel_pilot()
                .intel_copy_context(kit::BASE, kit::GAME_STATE, [true; 1]);
        let report = crate::extended::fog::copy_intel_refresh_with_divisions(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
            None,
        );
        frame
            .extended
            .intel_pilot()
            .apply_intel_report(ctx.needed, &report);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 1]);
        recycle_intel_job(second);
        let mut job = bare_job(index, next_seq(), 7, 67);
        job.intel_refresh = true;
        job.intel_payload = Some(first);
        process_job(
            &shared,
            &mut job,
            &mut AirBasesMemo::default(),
            &mut AirStatsMemo::default(),
        );
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
