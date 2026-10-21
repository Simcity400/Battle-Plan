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
        air::{AirBasesJob, AirBasesMemo, AirBasesMemoContext, AirStatsJob, AirWingJob},
        characters::{CharactersJob, PART_MASKS as CHARACTER_MASKS, PARTS as CHARACTER_PARTS},
        construction::ConstructionRestJob,
        economy::EconomyRestJob,
        fog::{IntelJob, RadarJob},
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
use replay_data::rows::TableId;

static CAPTURE_TAIL_DECODE_NS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static CENSUS_TAIL_DECODE_NS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static MISSION_TAIL_DECODE_NS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

use crossbeam_queue::ArrayQueue;
use std::cell::UnsafeCell;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering},
};
mod diagnostics;
mod pools;

pub(crate) use diagnostics::*;
pub use pools::*;

/// Relations family in calibration/report order: [opinions, tuples]. The
/// payload freezes each table's activity selected by replay cadence.
pub const RELATIONS: [TableId; 2] = [TableId::RelationOpinions, TableId::RelationTuples];

/// Characters family in calibration/report order: [characters, leaders,
/// advisors, units, traits]. Cadence selects the active tables.
/// ActiveAbilities is separate.
pub const CHARACTERS: [TableId; crate::extended::characters::N] =
    crate::extended::characters::TABLES;

/// The raw-copied project tables in calibration/report order: [programs,
/// breakthroughs]. Sliced and hourly project tables stay game-side typed.
pub const PROJECTS: [TableId; 2] = crate::extended::projects::RAW;

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
/// purchases, fuel consumers, fuel demand parts]. Supply stays hourly, the
/// three resource tables refresh on slot 10, and the eight remaining tables
/// on slot 17; the demand parts are split from the fuel walk's packs. The supply shared traversal is anchor-charged to the hubs slot; the
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
    TableId::FuelDemandParts,
];
/// [`ECONOMY_REST`]'s names, for the calibration status's positional arrays.
pub const ECONOMY_REST_NAMES: [&str; crate::extended::economy::N] = [
    "CountryFuel",
    "CountryExperience",
    "ProvinceSupply",
    "HubMotorization",
    "DisruptedSupply",
    "ForeignHomebases",
    "ResourceDefinitions",
    "CountryResources",
    "ResourceExchanges",
    "PurchaseContracts",
    "CountryFuelConsumers",
    "FuelDemandParts",
];

/// Hourly weather family in calibration/report order: [regions, provinces].
/// Hourly like every Tier-H table (no refresh gate). The manager header is
/// anchor-charged to the regions slot, matching the read-group convention.
pub const WEATHER: [TableId; 2] = [TableId::WeatherRegions, TableId::WeatherProvinces];

/// Hourly capitulation table in calibration/report order: [capitulation].
/// Hourly like every Tier-H table (no refresh gate).
pub const CAPITULATION: [TableId; 1] = [TableId::Capitulation];

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
    pub air_wing_refresh: bool,
    pub air_stats_refresh: bool,
    pub naval_refresh: bool,
    pub players_refresh: bool,
    pub session_refresh: bool,
    pub production_rest_refresh: bool,
    pub construction_rest_refresh: bool,
    pub economy_rest_refresh: bool,
    pub air_bases_refresh: bool,
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
    pub air_wing_payload: Option<AirWingJob>,
    pub air_stats_payload: Option<AirStatsJob>,
    pub naval_payload: Option<NavalJob>,
    pub players_payload: Option<PlayersJob>,
    pub session_payload: Option<SessionJob>,
    pub production_rest_payload: Option<ProductionRestJob>,
    pub construction_rest_payload: Option<ConstructionRestJob>,
    pub economy_rest_payload: Option<EconomyRestJob>,
    pub air_bases_payload: Option<AirBasesJob>,
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
static CHARACTERS_COPY_NS: [AtomicU64; CHARACTERS.len()] =
    [const { AtomicU64::new(0) }; CHARACTERS.len()];
static CHARACTERS_DECODE_NS: [AtomicU64; CHARACTERS.len()] =
    [const { AtomicU64::new(0) }; CHARACTERS.len()];
static CHARACTERS_RAW_BYTES: [AtomicU64; CHARACTERS.len()] =
    [const { AtomicU64::new(0) }; CHARACTERS.len()];
static PROJECTS_COPY_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static PROJECTS_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static PROJECTS_RAW_BYTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
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

mod failure;
pub(crate) use failure::{HookGuard as PanicHook, first as panic_failure};

pub(crate) fn owned_failed() -> bool {
    crate::allocation::failed() || panic_failure().is_some()
}

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
/// Admit a relations payload whenever any family table is active. The payload
/// freezes each table's activity for decode.
pub fn relations_refresh_hour(hour: i32, force_full: bool) -> bool {
    RELATIONS
        .iter()
        .any(|table| replay_data::cadence::should_walk(*table, hour, force_full))
}

/// Admit a character payload whenever any family table is active.
pub fn characters_refresh_hour(hour: i32, force_full: bool) -> bool {
    CHARACTERS
        .iter()
        .any(|table| replay_data::cadence::should_walk(*table, hour, force_full))
}

/// Projects refresh together on their daily slot (`replay_data::cadence`); full
/// hours (branch start, load, gap or an explicit refresh request) force their
/// complete acquisition.
pub fn projects_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay_data::cadence::should_walk(PROJECTS[0], hour, force_full);
    for table in PROJECTS.iter().skip(1) {
        debug_assert_eq!(
            walk,
            replay_data::cadence::should_walk(*table, hour, force_full)
        );
    }
    walk
}

/// Admit the dictionary according to its authoritative replay cadence.
pub fn lexical_refresh_hour(hour: i32, force_full: bool) -> bool {
    replay_data::cadence::should_walk(LEXICAL[0], hour, force_full)
}

/// Monotonic frame sequence for FIFO order proofs. Gaps in the sequence are
/// dropped frames; order within the sequence is queue order.
pub fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

const PAYLOADS_PER_FRAME: usize = 19 + CHARACTER_PARTS;
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
    AirWing(AirWingJob),
    AirStats(AirStatsJob),
    Naval(NavalJob),
    Players(PlayersJob),
    Session(SessionJob),
    ProductionRest(ProductionRestJob),
    ConstructionRest(ConstructionRestJob),
    EconomyRest(EconomyRestJob),
    AirBases(AirBasesJob),
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

pub fn prepare_payload_pools(game_score: bool) -> PayloadPools {
    prepare_retirement();
    for _ in 0..JOB_POOL_KEEP + 1 {
        if crate::allocation::failed() {
            break;
        }
        release_relations_job(RelationsJob::new());
        for part in 0..CHARACTER_PARTS {
            release_characters_job(CharactersJob::new_part(part));
        }
        release_projects_job(ProjectsJob::new());
        release_lexical_job(LexicalJob::new());
        release_divisions_job(DivisionsJob::new_worker());
        release_air_wing_job(AirWingJob::with_game_score(game_score));
        release_air_stats_job(AirStatsJob::new());
        release_naval_job(NavalJob::new());
        release_players_job(PlayersJob::new());
        release_session_job(SessionJob::new());
        release_production_rest_job(ProductionRestJob::new());
        release_construction_rest_job(ConstructionRestJob::new());
        release_economy_rest_job(EconomyRestJob::new());
        release_air_bases_job(AirBasesJob::new());
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
    if queue.is_null() && crate::allocation::failed() {
        return;
    }
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
            RetiredPayload::AirWing(job) => release_air_wing_job(job),
            RetiredPayload::AirStats(job) => release_air_stats_job(job),
            RetiredPayload::Naval(job) => release_naval_job(job),
            RetiredPayload::Players(job) => release_players_job(job),
            RetiredPayload::Session(job) => release_session_job(job),
            RetiredPayload::ProductionRest(job) => release_production_rest_job(job),
            RetiredPayload::ConstructionRest(job) => release_construction_rest_job(job),
            RetiredPayload::EconomyRest(job) => release_economy_rest_job(job),
            RetiredPayload::AirBases(job) => release_air_bases_job(job),
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
#[cfg(test)]
pub fn enqueue(shared: &Shared, job: DecodeJob) -> Option<DecodeJob> {
    if crate::allocation::failed() {
        return Some(job);
    }
    match shared.decode_queue.push(job) {
        Ok(()) => {
            enqueued(shared);
            None
        }
        Err(job) => {
            refused();
            Some(job)
        }
    }
}

/// `enqueue` for the hour's job, filled in place in its reserved slot. A full
/// queue or a failed allocation retires the job's payloads to their pools for
/// the existing gap path and returns false.
pub fn enqueue_reserved(shared: &Shared, mut reserved: Reserved<'_>) -> bool {
    if crate::allocation::failed() {
        recycle_payloads(reserved.job());
        return false;
    }
    match reserved.publish() {
        Ok(()) => {
            enqueued(shared);
            true
        }
        Err(mut reserved) => {
            refused();
            recycle_payloads(reserved.job());
            false
        }
    }
}

fn enqueued(shared: &Shared) {
    JOBS.fetch_add(1, Ordering::Relaxed);
    // Decode-queue depth high-water for live backlog visibility.
    // Records the deepest observed FIFO occupancy (bounded by BUFFERS).
    let depth = shared.decode_queue.len() as u64;
    QUEUE_HIGH_WATER.fetch_max(depth, Ordering::Relaxed);
}

fn refused() {
    crate::equipment_destruction::lost_drain();
    crate::naval_result_sources::lost_drain();
    crate::air_losses::lost_drain();
    crate::building_bombing::lost_drain();
    crate::nuclear_strikes::lost_drain();
    crate::land_events::fail("land frame decode queue full");
    DROPS.fetch_add(1, Ordering::Relaxed);
}

/// Return an unqueued job's payloads to their pools.
fn recycle_payloads(job: &mut DecodeJob) {
    if let Some(payload) = job.relations_payload.take() {
        recycle_relations_job(payload);
    }
    for payload in job.characters_payload.iter_mut().filter_map(Option::take) {
        recycle_characters_job(payload);
    }
    if let Some(payload) = job.projects_payload.take() {
        recycle_projects_job(payload);
    }
    if let Some(payload) = job.lexical_payload.take() {
        recycle_lexical_job(payload);
    }
    if let Some(payload) = job.divisions_payload.take() {
        recycle_divisions_job(payload);
    }
    if let Some(payload) = job.air_wing_payload.take() {
        recycle_air_wing_job(payload);
    }
    if let Some(payload) = job.air_stats_payload.take() {
        recycle_air_stats_job(payload);
    }
    if let Some(payload) = job.naval_payload.take() {
        recycle_naval_job(payload);
    }
    if let Some(payload) = job.players_payload.take() {
        recycle_players_job(payload);
    }
    if let Some(payload) = job.session_payload.take() {
        recycle_session_job(payload);
    }
    if let Some(payload) = job.production_rest_payload.take() {
        recycle_production_rest_job(payload);
    }
    if let Some(payload) = job.construction_rest_payload.take() {
        recycle_construction_rest_job(payload);
    }
    if let Some(payload) = job.economy_rest_payload.take() {
        recycle_economy_rest_job(payload);
    }
    if let Some(payload) = job.air_bases_payload.take() {
        recycle_air_bases_job(payload);
    }
    if let Some(payload) = job.supremacy_payload.take() {
        recycle_supremacy_job(payload);
    }
    if let Some(payload) = job.sunk_payload.take() {
        recycle_sunk_job(payload);
    }
    if let Some(payload) = job.radar_payload.take() {
        recycle_radar_job(payload);
    }
    if let Some(payload) = job.intel_payload.take() {
        recycle_intel_job(payload);
    }
    if let Some(payload) = job.weather_payload.take() {
        recycle_weather_job(payload);
    }
    if let Some(payload) = job.capitulation_payload.take() {
        recycle_capitulation_job(payload);
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
                failure::stage("idle retirement", None);
                let _phase = crate::allocation::enter(crate::allocation::Phase::Decode);
                crate::allocation::attempted_hour(None);
                reclaim_payloads();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
    let _phase = crate::allocation::enter(crate::allocation::Phase::Teardown);
    failure::stage("worker retirement", None);
    crate::allocation::attempted_hour(None);
    reclaim_payloads();
}

/// The worker owns the raw payloads through decoding, then returns every
/// family to its pool. Dropping this envelope without recycling used to
/// allocate a fresh payload on the game thread on every subsequent hour.
fn process_owned_job(shared: &Shared, mut job: DecodeJob, air_bases_memo: &mut AirBasesMemo) {
    let failure_scope = failure::Scope::frame(failure::Frame {
        index: job.index,
        seq: job.seq,
        hour: job.hour,
        force_full: job.force_full,
        load_signal: None,
        world_generation: None,
    });
    let _phase = crate::allocation::enter(crate::allocation::Phase::Decode);
    let frame = unsafe { &*shared.frames[job.index].load(Ordering::Acquire) };
    failure::Scope::identity(
        frame
            .clock
            .as_ref()
            .map(|clock| clock.play_session_started_at),
        frame.world_generation,
    );
    if let Some(clock) = frame.clock.as_ref() {
        crate::allocation::set_context(crate::allocation::Context {
            campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
            load_signal: Some(clock.play_session_started_at),
            failed_hour: Some(job.hour),
            ..Default::default()
        });
    } else {
        crate::allocation::attempted_hour(Some(job.hour));
    }
    // Enqueue follows the callback's failed-payload retirement. Drain after
    // taking this frame, before publication lets the spool thread return its
    // lease. Draining before the queue pop permits one slot to retire twice.
    failure::stage("frame retirement", None);
    reclaim_payloads();
    if !crate::allocation::failed() {
        process_job(shared, &mut job, air_bases_memo);
    }

    // Return even a terminally skipped frame's loan. Its Drop forgets an
    // incomplete settlement, so later acquisition never carries a partial tree.
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    frame.map.naval.discard_owned();
    frame.extended.discard_owned_censuses();
    if let Some(store) = frame.extended.take_settled_army_store() {
        shared.return_army_stat_store(store);
    }
    failure::stage("release payloads", None);
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
    failure::stage("publish ready", None);
    let published = publish_ready(
        &shared.ready,
        job.index,
        &shared.completed[job.index],
        job.game_elapsed,
        crate::allocation::failed,
    );
    // No frame reference or diagnostic lease survives its ready publication.
    drop(failure_scope);
    failure::stage("after ready handoff", None);
    if published {
        shared.wake_spool();
    }
    crate::allocation::attempted_hour(None);
}

fn publish_ready(
    ready: &ArrayQueue<usize>,
    index: usize,
    completed: &std::sync::atomic::AtomicI64,
    elapsed: i64,
    mut failed: impl FnMut() -> bool,
) -> bool {
    // Failure can stop the spool between admission and a full-queue handoff.
    // It can no longer return a slot; let teardown join this worker.
    loop {
        if failed() {
            completed.store(0, Ordering::Release);
            return false;
        }
        if ready.push(index).is_ok() {
            completed.store(elapsed.max(1), Ordering::Release);
            return true;
        }
        std::thread::yield_now();
    }
}

#[cfg(test)]
pub(crate) fn allocation_exercise(shared: &Shared, index: usize, hour: i32) -> bool {
    use engine_native::memory::SliceMemory;
    let bytes = [7_u8; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut payload = ProjectsJob::default();
    payload.log = crate::raw::RawLog::new(4, 64);
    payload.active = true;
    if crate::allocation::failed() {
        return false;
    }
    if payload.log.take(&memory, 0x1000, 8, 1).is_none() {
        return false;
    }
    let job = DecodeJob {
        index,
        hour,
        game_elapsed: 1,
        projects_refresh: true,
        projects_payload: Some(payload),
        ..Default::default()
    };
    process_owned_job(shared, job, &mut AirBasesMemo::default());
    !crate::allocation::failed()
}

pub fn start_worker() -> Result<(), &'static str> {
    let mut slot = WORKER.lock().map_err(|_| "decode worker lock poisoned")?;
    if slot.as_ref().is_some_and(|handle| !handle.is_finished()) {
        return Err("decode worker already running");
    }
    STOP.store(false, Ordering::Release);
    WORKER_ID.store(0, Ordering::Release);
    *slot = Some(
        std::thread::Builder::new()
            .name("hoi4-decode".into())
            .stack_size(crate::runtime::BACKGROUND_STACK_BYTES)
            .spawn(|| {
                #[cfg(feature = "worker_sampler")]
                let _sampler_exit = crate::thread_cpu::StopOnThreadExit;
                #[cfg(feature = "worker_sampler")]
                crate::thread_cpu::register_decode();
                WORKER_ID.store(
                    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
                    Ordering::Release,
                );
                failure::worker(decode_loop);
            })
            .map_err(|_| "cannot start decode worker")?,
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
        frame.divisions_result = engine_native::walk::Outcome::Unavailable {
            reason: MISSING_PAYLOAD_REASON,
        };
        frame
            .extended
            .install_failed_division_counts(MISSING_PAYLOAD_REASON);
        frame.extended.fail_division_details(MISSING_PAYLOAD_REASON);
        frame.extended.set_divisions_raw_mode(false);
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
            .game_score_pilot()
            .fail_air(MISSING_PAYLOAD_REASON);
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
        frame.extended.install_reserved_land_certificate();
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
    failure::stage("missing payload verdicts", None);
    if !job.air_bases_refresh || job.air_bases_payload.is_none() {
        air_bases_memo.invalidate();
    }
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    fail_missing_payloads(frame, job);
    // Return presentation's cross-frame store before independent publication.
    decode_presentation(shared, job.index);
    failure::stage("owned capture tails", None);
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    let (ticks, bytes) = frame.extended.decode_capture_tails();
    for (total, ticks) in CAPTURE_TAIL_DECODE_NS.iter().zip(ticks) {
        total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
    }
    shared.pool_bytes.fetch_add(bytes, Ordering::Relaxed);
    failure::stage("owned census publication", None);
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    let start = sampling::qpc();
    let bytes = frame.map.naval.decode_owned();
    let ships = sampling::qpc().saturating_sub(start).max(0) as u64;
    let ([map, governance, tasks], extended_bytes) = frame.extended.decode_owned_censuses();
    for (total, ticks) in CENSUS_TAIL_DECODE_NS.iter().zip([ships, map, governance, tasks]) {
        total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
    }
    shared.pool_bytes.fetch_add(bytes.wrapping_add(extended_bytes), Ordering::Relaxed);
    failure::stage("owned mission tails", None);
    let (ticks, bytes) = frame.extended.decode_mission_tails();
    for (total, ticks) in MISSION_TAIL_DECODE_NS.iter().zip(ticks) {
        total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
    }
    shared.pool_bytes.fetch_add(bytes, Ordering::Relaxed);
    failure::stage("air copies", None);
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    frame.extended.decode_air_copies();
    LAST_SEQ.store(job.seq, Ordering::Relaxed);
    LAST_HOUR.store(job.hour as u64, Ordering::Relaxed);
    if job.force_full {
        FULL_REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }

    if job.relations_refresh
        && let Some(payload) = job.relations_payload.as_ref()
    {
        failure::stage("relations", None);
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
    for (part, (refresh, payload)) in job
        .characters_refresh
        .iter()
        .zip(&mut job.characters_payload)
        .enumerate()
    {
        let Some(payload) = payload.as_mut().filter(|_| *refresh) else {
            continue;
        };
        failure::stage("characters", Some(part));
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; CHARACTERS.len()];
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
        failure::stage("projects", None);
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; PROJECTS.len()];
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
        failure::stage("lexical", None);
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
        failure::stage("divisions", None);
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 6];
        let before_bytes = frame.extended.division_decode_allocated_bytes();
        sampling::decode_divisions_payload(frame, shared.base, payload, &mut decode_ticks);
        let after_bytes = frame.extended.division_decode_allocated_bytes();
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
    if job.air_wing_refresh
        && let Some(payload) = job.air_wing_payload.as_mut()
    {
        failure::stage("air_wing", None);
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        crate::extended::air::decode_air_payload(
            frame.extended.air_wing_pilot(),
            payload,
            &mut decode_ticks,
        );
        frame
            .extended
            .game_score_pilot()
            .install_air(&mut payload.game_score);
        payload.update_capacity_charge();
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
        failure::stage("air_stats", None);
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
        failure::stage("naval", None);
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 9];
        frame
            .extended
            .decode_naval_payload(shared.base, payload, &mut decode_ticks);
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
        failure::stage("players", None);
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
        failure::stage("session", None);
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
        failure::stage("production_rest", None);
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
        failure::stage("construction_rest", None);
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
        failure::stage("economy_rest", None);
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
        failure::stage("air_bases", None);
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
    if job.supremacy_refresh
        && let Some(payload) = job.supremacy_payload.as_ref()
    {
        failure::stage("supremacy", None);
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
        failure::stage("sunk", None);
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
        failure::stage("radar", None);
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
        failure::stage("intel", None);
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
        failure::stage("weather", None);
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        let mut tracker = shared
            .weather_temperature
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        frame.extended.weather_pilot().decode_weather_payload(
            shared.base,
            payload,
            job.hour,
            &mut tracker,
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
        failure::stage("capitulation", None);
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
}

/// The FIFO worker owns the frame; this packet has no raw decode dependency.
/// Return its held-store loan before unrelated families can delay the next hour.
fn decode_presentation(shared: &Shared, index: usize) {
    failure::stage("division presentation", None);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let before_bytes = frame.extended.presentation_allocated_bytes();
    let before = frame.extended.presentation_phase_ticks();
    for (total, ticks) in DIVISION_PRESENTATION_DECODE_NS
        .iter()
        .zip(frame.extended.decode_division_presentation_inputs())
    {
        total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
    }
    let after = frame.extended.presentation_phase_ticks();
    shared.record_owned_presentation(std::array::from_fn(|i| after[i].saturating_sub(before[i])));
    let after_bytes = frame.extended.presentation_allocated_bytes();
    shared.pool_bytes.fetch_add(
        (after_bytes as u64).wrapping_sub(before_bytes as u64),
        Ordering::Relaxed,
    );
    if let Some(store) = frame.extended.take_settled_army_store() {
        shared.return_army_stat_store(store);
    }
}

/// Detach drain (runtime only, after callback quiescence): stop the worker,
/// join it, then synchronously decode any remaining FIFO items in order and
/// hand them to the spool ready queue before the existing spool drain runs.
/// The spool thread is still consuming, so handoffs make progress.
/// `drained` receives the count of queued jobs decoded so far, after each.
pub fn detach_drain(mut drained: impl FnMut(u64)) -> Result<(), &'static str> {
    let _failure = failure::Scope::worker("decode detach drain");
    STOP.store(true, Ordering::Release);
    let (worker, mut error) = match WORKER.lock() {
        Ok(mut slot) => (slot.take(), None),
        Err(poisoned) => (
            poisoned.into_inner().take(),
            Some("decode worker lock poisoned"),
        ),
    };
    #[cfg(feature = "worker_sampler")]
    if worker.is_some() {
        crate::worker_sampler::stop_before_thread_exit();
    }
    if let Some(handle) = worker
        && let Err(payload) = handle.join()
    {
        crate::equipment_destruction::lost_drain();
        crate::naval_result_sources::lost_drain();
        crate::air_losses::lost_drain();
        crate::building_bombing::lost_drain();
        crate::nuclear_strikes::lost_drain();
        crate::land_events::fail("land decode worker panicked");
        error = Some(failure::joined(
            payload,
            "decode worker panicked; its in-flight frame may be incomplete",
        ));
    }
    WORKER_ID.store(0, Ordering::Release);
    let shared_ptr = sampling::SHARED.load(Ordering::Acquire);
    if shared_ptr.is_null() {
        STOP.store(false, Ordering::Release);
        return panic_failure().or(error).map_or(Ok(()), Err);
    }
    let shared = unsafe { &*shared_ptr };
    // The joined worker drops its memo. Drain owns one fresh entry and cannot
    // race or retain references into worker/ready-queue storage.
    let mut air_bases_memo = AirBasesMemo::default();
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut count = 0;
        while let Some(job) = shared.decode_queue.pop() {
            process_owned_job(shared, job, &mut air_bases_memo);
            count += 1;
            failure::stage("drain progress", None);
            drained(count);
        }
    })) {
        crate::equipment_destruction::lost_drain();
        crate::naval_result_sources::lost_drain();
        crate::air_losses::lost_drain();
        crate::building_bombing::lost_drain();
        crate::nuclear_strikes::lost_drain();
        crate::land_events::fail("land decode detach drain panicked");
        let reason = failure::joined(
            payload,
            "decode detach drain panicked; queued frames remain incomplete",
        );
        error.get_or_insert(reason);
    }
    STOP.store(false, Ordering::Release);
    panic_failure().or(error).map_or(Ok(()), Err)
}

/// Additive calibration snapshot. Every counter is monotonic since attach.
pub fn calibration() -> Calibration {
    Calibration {
        mission_tail_decode_ns: MISSION_TAIL_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        census_tail_decode_ns: CENSUS_TAIL_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        capture_tail_decode_ns: CAPTURE_TAIL_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
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
    pub mission_tail_decode_ns: [u64; 2],
    pub census_tail_decode_ns: [u64; 4],
    pub capture_tail_decode_ns: [u64; 5],
    pub jobs: u64,
    pub refresh_jobs: u64,
    pub full_refresh_jobs: u64,
    pub drops: u64,
    pub last_seq: u64,
    pub last_hour: i32,
    pub relations_copy_ns: [u64; 2],
    pub relations_decode_ns: [u64; 2],
    pub relations_raw_bytes: [u64; 2],
    pub characters_copy_ns: [u64; CHARACTERS.len()],
    pub characters_decode_ns: [u64; CHARACTERS.len()],
    pub characters_raw_bytes: [u64; CHARACTERS.len()],
    pub projects_copy_ns: [u64; 2],
    pub projects_decode_ns: [u64; 2],
    pub projects_raw_bytes: [u64; 2],
    pub lexical_copy_ns: [u64; 1],
    pub lexical_decode_ns: [u64; 1],
    pub lexical_raw_bytes: [u64; 1],
    pub divisions_copy_ns: [u64; 6],
    pub divisions_decode_ns: [u64; 6],
    pub division_presentation_decode_ns: [u64; 2],
    pub divisions_raw_bytes: [u64; 6],
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
    #[cfg(test)]
    pub fn json(&self) -> serde_json::Value {
        serde_json::to_value((*self).status()).unwrap()
    }

    pub fn status(self) -> crate::status::Object<impl crate::status::Fields> {
        crate::status::status!({
            "scope": "decode worker totals since attach; monotonic counters except live capacity fields",
            "raw_growth": crate::raw::growth_diagnostics(),
            "raw_indexes": crate::raw::index_diagnostics(),
            "read_plans": crate::raw::read_plan_diagnostics(),
            "raw_memory": crate::raw_memory::diagnostics(),

            "carrier_capture": crate::extended::air::carrier_capture_status(),
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
            "capture_tail_decode_ns": self.capture_tail_decode_ns,
            "census_tail_decode_ns": self.census_tail_decode_ns,
            "mission_tail_decode_ns": self.mission_tail_decode_ns,
            "mission_tail_roles": ["OrdinaryNavies", "RailwayGuns"],
            "census_tail_roles": ["ShipModels", "OrdinaryMap", "Governance", "OrdinaryTaskForces"],
            "capture_tail_roles": ["Templates", "ShipModelDetails", "RaidState", "Variants", "OrdinaryCombat"],
            "relations_copy_ns": self.relations_copy_ns,
            "relations_decode_ns": self.relations_decode_ns,
            "relations_raw_bytes": self.relations_raw_bytes,
            "relations_tables": ["RelationOpinions", "RelationTuples"],
            "characters_copy_ns": self.characters_copy_ns,
            "characters_decode_ns": self.characters_decode_ns,
            "characters_raw_bytes": self.characters_raw_bytes,
            "characters_tables": ["Characters", "CountryLeaderRoles", "AdvisorRoles", "UnitLeaders", "UnitLeaderTraits"],
            "projects_copy_ns": self.projects_copy_ns,
            "projects_decode_ns": self.projects_decode_ns,
            "projects_raw_bytes": self.projects_raw_bytes,
            "projects_tables": ["ProjectPrograms", "Projects", "ProjectHistory", "ProjectBreakthroughs"],
            "lexical_copy_ns": self.lexical_copy_ns,
            "lexical_decode_ns": self.lexical_decode_ns,
            "lexical_raw_bytes": self.lexical_raw_bytes,
            "lexical_tables": ["LexicalTokens"],
            "divisions_copy_ns": self.divisions_copy_ns,
            "divisions_decode_ns": self.divisions_decode_ns,
            "division_presentation_decode_ns": self.division_presentation_decode_ns,
            "division_presentation_tables": ["DivisionMovement", "DivisionStatus"],
            "divisions_raw_bytes": self.divisions_raw_bytes,
            "divisions_tables": ["Divisions", "DivisionEquipment", "ArmyUpgradeGroups", "ArmyUpgradeRequests", "ArmyUpgradeDeliveries", "ArmyUpgradeProduced"],
            "air_wing_copy_ns": self.air_wing_copy_ns,
            "air_wing_decode_ns": self.air_wing_decode_ns,
            "air_wing_raw_bytes": self.air_wing_raw_bytes,
            "air_wing_tables": ["WingEquipment", "WingCumulative"],
            "air_stats_copy_ns": self.air_stats_copy_ns,
            "air_stats_decode_ns": self.air_stats_decode_ns,
            "air_stats_raw_bytes": self.air_stats_raw_bytes,
            "air_stats_tables": ["AirStatistics"],
            "naval_copy_ns": self.naval_copy_ns,
            "naval_decode_ns": self.naval_decode_ns,
            "naval_raw_bytes": self.naval_raw_bytes,
            "naval_tables": ["LandTheatres", "LandTheatreGroups", "LandTheatreMembers", "AirTheatres", "AirTheatreGroups", "AirTheatreMembers", "OrdersGroups", "OrdersGroupMembers", "ArmyGroupMembers"],
            "players_copy_ns": self.players_copy_ns,
            "players_decode_ns": self.players_decode_ns,
            "players_raw_bytes": self.players_raw_bytes,
            "players_tables": ["Players", "HumanControllers"],
            "session_copy_ns": self.session_copy_ns,
            "session_decode_ns": self.session_decode_ns,
            "session_raw_bytes": self.session_raw_bytes,
            "session_tables": ["WorldTension", "ThreatSources", "ThreatCountryAggregates"],
            "production_rest_copy_ns": self.production_rest_copy_ns,
            "production_rest_decode_ns": self.production_rest_decode_ns,
            "production_rest_raw_bytes": self.production_rest_raw_bytes,
            "production_rest_tables": ["ProductionLines", "Stockpiles", "ProductionLicenses", "DeploymentConveyors", "DeploymentLines", "DeploymentManpower", "DeploymentEquipment", "ProductionLicenseRecords"],
            "construction_rest_copy_ns": self.construction_rest_copy_ns,
            "construction_rest_decode_ns": self.construction_rest_decode_ns,
            "construction_rest_raw_bytes": self.construction_rest_raw_bytes,
            "construction_rest_tables": ["StateBaseResources", "BuildingConstruction", "RailwayConstruction", "OccupationSettings", "StateOccupationLaws", "StateGarrisons", "StateAssignedManpower", "CountryOccupationReserveManpower", "StateResistance", "CountryCollaboration"],
            "economy_rest_copy_ns": self.economy_rest_copy_ns,
            "economy_rest_decode_ns": self.economy_rest_decode_ns,
            "economy_rest_raw_bytes": self.economy_rest_raw_bytes,
            "economy_rest_tables": ECONOMY_REST_NAMES,
            "air_bases_copy_ns": self.air_bases_copy_ns,
            "air_bases_decode_ns": self.air_bases_decode_ns,
            "air_bases_raw_bytes": self.air_bases_raw_bytes,
            "air_bases_tables": ["AirBases", "AirBaseRegistrations", "AirAces"],
            "supremacy_copy_ns": self.supremacy_copy_ns,
            "supremacy_decode_ns": self.supremacy_decode_ns,
            "supremacy_raw_bytes": self.supremacy_raw_bytes,
            "supremacy_tables": ["NavalSupremacy"],
            "sunk_copy_ns": self.sunk_copy_ns,
            "sunk_decode_ns": self.sunk_decode_ns,
            "sunk_raw_bytes": self.sunk_raw_bytes,
            "sunk_tables": ["SunkShips"],
            "radar_copy_ns": self.radar_copy_ns,
            "radar_decode_ns": self.radar_decode_ns,
            "radar_raw_bytes": self.radar_raw_bytes,
            "radar_tables": ["RadarCoverage"],
            "intel_copy_ns": self.intel_copy_ns,
            "intel_decode_ns": self.intel_decode_ns,
            "intel_raw_bytes": self.intel_raw_bytes,
            "intel_tables": ["UnitIntel"],
            "weather_copy_ns": self.weather_copy_ns,
            "weather_decode_ns": self.weather_decode_ns,
            "weather_raw_bytes": self.weather_raw_bytes,
            "weather_tables": ["WeatherRegions", "WeatherProvinces"],
            "capitulation_copy_ns": self.capitulation_copy_ns,
            "capitulation_decode_ns": self.capitulation_decode_ns,
            "capitulation_raw_bytes": self.capitulation_raw_bytes,
            "capitulation_tables": ["Capitulation"],
            "queue_high_water": self.queue_high_water,
            "queue_capacity": BUFFERS,
        })
    }
}

/// Additive status merge for the runtime statuswriters. Existing keys survive.
pub fn calibration_json() -> impl serde::Serialize {
    calibration().status().join(crate::status::status!({
        "characters_acquisition": crate::extended::characters::acquisition_diagnostics(),
    }))
}

/// The bounded decode FIFO. Each job is filled in place in a preallocated
/// slot and only slot numbers move through the queues, so the game thread
/// never moves a job (tens of KiB) through its stack. A slot belongs to one
/// side at a time: free, reserved by its producer, ready, or held by the
/// consumer while it moves the job out.
pub struct DecodeQueue {
    jobs: Box<[UnsafeCell<DecodeJob>]>,
    free: ArrayQueue<usize>,
    ready: ArrayQueue<usize>,
}

// SAFETY: a slot's job is accessed only by the side holding its number.
unsafe impl Sync for DecodeQueue {}

impl DecodeQueue {
    fn new(capacity: usize) -> Self {
        let free = ArrayQueue::new(capacity);
        for slot in 0..capacity {
            free.push(slot).expect("decode slot fits its free list");
        }
        Self {
            jobs: (0..capacity)
                .map(|_| UnsafeCell::new(DecodeJob::default()))
                .collect(),
            free,
            ready: ArrayQueue::new(capacity),
        }
    }

    /// A vacant slot to fill in place. The hourly body holds one only while it
    /// owns a buffer, and every queued job holds another buffer, so one is free.
    pub fn reserve(&self) -> Option<Reserved<'_>> {
        self.free.pop().map(|slot| Reserved { queue: self, slot })
    }

    /// By value, like the queue it replaced; the hourly body fills a reservation.
    #[cfg(test)]
    #[allow(clippy::result_large_err)]
    pub fn push(&self, job: DecodeJob) -> Result<(), DecodeJob> {
        let Some(mut reserved) = self.reserve() else {
            return Err(job);
        };
        *reserved.job() = job;
        reserved
            .publish()
            .map_err(|mut reserved| std::mem::take(reserved.job()))
    }

    pub fn pop(&self) -> Option<DecodeJob> {
        let slot = self.ready.pop()?;
        // SAFETY: popping the slot's number from `ready` hands it to this consumer.
        let job = std::mem::take(unsafe { &mut *self.jobs[slot].get() });
        self.free
            .push(slot)
            .expect("decode slot returns to its free list");
        Some(job)
    }

    pub fn len(&self) -> usize {
        self.ready.len()
    }

    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.ready.capacity()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.ready.is_empty()
    }
}

/// A producer's slot. Dropped unpublished, it returns vacant: payloads it
/// still holds are dropped, like the locals they replace.
pub struct Reserved<'a> {
    queue: &'a DecodeQueue,
    slot: usize,
}

impl Reserved<'_> {
    pub fn job(&mut self) -> &mut DecodeJob {
        // SAFETY: this reservation holds the slot's number.
        unsafe { &mut *self.queue.jobs[self.slot].get() }
    }

    /// Hand the job to the consumer; a full queue returns the reservation.
    fn publish(self) -> Result<(), Self> {
        if self.queue.ready.push(self.slot).is_err() {
            return Err(self);
        }
        std::mem::forget(self);
        Ok(())
    }
}

impl Drop for Reserved<'_> {
    fn drop(&mut self) {
        let job = self.job();
        // Field by field: no vacant job is built on this stack.
        job.relations_payload = None;
        job.characters_payload = [const { None }; CHARACTER_PARTS];
        job.projects_payload = None;
        job.lexical_payload = None;
        job.divisions_payload = None;
        job.air_wing_payload = None;
        job.air_stats_payload = None;
        job.naval_payload = None;
        job.players_payload = None;
        job.session_payload = None;
        job.production_rest_payload = None;
        job.construction_rest_payload = None;
        job.economy_rest_payload = None;
        job.air_bases_payload = None;
        job.supremacy_payload = None;
        job.sunk_payload = None;
        job.radar_payload = None;
        job.intel_payload = None;
        job.weather_payload = None;
        job.capitulation_payload = None;
        let _ = self.queue.free.push(self.slot);
    }
}

pub fn new_queue() -> DecodeQueue {
    DecodeQueue::new(BUFFERS)
}

#[cfg(test)]
pub(crate) fn reset_calibration_for_test() {
    prepare_retirement();
    reclaim_payloads();
    // Production prepares bounded payload storage before installing the hook.
    // Tests exercise that lifecycle rather than relying on cold tick allocation.
    std::mem::forget(prepare_payload_pools(false));
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
mod tests;

#[cfg(test)]
pub(crate) use tests::{
    presize_and_acquire_carrier_jobs_for_test, retire_and_reacquire_two_carrier_jobs_for_test,
};

#[cfg(test)]
mod order_group_loan_tests;
