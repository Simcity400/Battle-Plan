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
//! No backpressure: each frame's job lives in its frame slot and the FIFO links
//! the slots ([`crate::frame_pool`]), so enqueue never fills, never waits and
//! never allocates; a backlog takes more frames. Only a failed allocation
//! refuses an hour, as a whole-frame loss through `LossBatch`. Staging capacity
//! misses instead use each family's admitted fallback and preserve its first
//! failure; accepted-job panics and failed prepared acquisitions cannot retry
//! live reads.
//!
//! Each family's payload pool keeps a payload for every frame a backlog holds and trims back
//! to `JOB_POOL_STEADY` once it clears.
//! Exhaustion or lock contention returns an empty staging object without
//! allocating another payload. Retirement only transfers ownership on the game
//! thread; the decode worker clears, grows and recycles that storage.
//! Raw payload storage is additional to the typed frame pool. Queue depth,
//! process memory and per-family span/byte capacities require live measurement;
//! a sum of byte arenas alone is not a bound on aggregate allocation.
//!
//! Per-frame copy-only ns stays in the `table_ns` slots for the staged tables.

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
    sampling::{self, DivisionsJob, Shared},
};
use replay_data::rows::TableId;

use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering},
};
mod pools;

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
    pub queued_qpc: i64,
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
// hits, misses, ineligible, comparisons, installs, compare/restore/install ns.
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

/// Loader watchdog only. A missing or failed worker cannot return frame leases, so the
/// loader detaches rather than let the frames grow.
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

/// Each family stages one owned payload or retires its failed staging object;
/// each independent character part owns a separate payload.
const PAYLOADS_PER_FRAME: usize = 19 + CHARACTER_PARTS;

// The worker takes each queued frame before draining retirement, so that frame's
// callback has already retired its failures before its lease can be returned.
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
/// Payloads retired on the game thread, freed or recycled by the decode worker. It holds
/// as many as a backlog retires: the worker keeps room for every frame live, so a retirement
/// allocates only when frames grew since its last drain. A retirement never waits: finding the
/// worker taking the held ones, or a refused allocation, frees the payload where it is, which
/// loses only its storage.
struct Retirement {
    held: Mutex<Vec<RetiredPayload>>,
}

impl Retirement {
    fn new() -> Self {
        Self {
            held: Mutex::new(Vec::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<RetiredPayload>> {
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn push(&self, payload: RetiredPayload) {
        let mut held = match self.held.try_lock() {
            Ok(held) => held,
            Err(std::sync::TryLockError::Poisoned(held)) => held.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return,
        };
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut held, 1)).is_ok() {
            held.push(payload);
        }
    }

    /// Everything retired so far, leaving `spare`'s storage, sized for `frames` frames, in its
    /// place.
    fn take(&self, mut spare: Vec<RetiredPayload>, frames: usize) -> Vec<RetiredPayload> {
        let room = frames.saturating_mul(PAYLOADS_PER_FRAME);
        if spare.capacity() < room {
            let _ = engine_allocation::advisory(|| {
                engine_allocation::reserve_exact(&mut spare, room)
            });
        }
        std::mem::replace(&mut *self.lock(), spare)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }

    #[cfg(test)]
    fn pop(&self) -> Option<RetiredPayload> {
        self.lock().pop()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

static RETIRED_PAYLOADS: AtomicPtr<Retirement> = AtomicPtr::new(std::ptr::null_mut());

fn prepare_retirement() -> std::io::Result<()> {
    if !RETIRED_PAYLOADS.load(Ordering::Acquire).is_null() {
        return Ok(());
    }
    let queue = Box::into_raw(engine_allocation::boxed(Retirement::new())?);
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
    Ok(())
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
        if let Ok(mut spare) = SPARE_RETIREMENT.lock() {
            *spare = Vec::new();
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

/// A refused allocation is the collector's allocation failure; the pools prepared so far are
/// released with the returned error.
pub fn prepare_payload_pools(game_score: bool) -> std::io::Result<PayloadPools> {
    let pools = PayloadPools;
    prepare_retirement()?;
    for _ in 0..JOB_POOL_STEADY {
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
    if crate::allocation::failed() {
        return Err(std::io::ErrorKind::OutOfMemory.into());
    }
    Ok(pools)
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
    queue.push(payload);
    // A payload retired without a queued frame still reaches the parked worker.
    let shared = sampling::SHARED.load(Ordering::Acquire);
    if !shared.is_null()
        && let Some(worker) = unsafe { &*shared }.decode_waker.get()
    {
        worker.unpark();
    }
}

fn reclaim_payloads() {
    let queue = RETIRED_PAYLOADS.load(Ordering::Acquire);
    if queue.is_null() {
        return;
    }
    // Only the decode worker or the quiescent loader drains this queue.
    let queue = unsafe { &*queue };
    let shared = sampling::SHARED.load(Ordering::Acquire);
    let frames = if shared.is_null() {
        crate::frame_pool::STEADY
    } else {
        unsafe { &*shared }.frames.live()
    };
    let spare = SPARE_RETIREMENT
        .lock()
        .map(|mut spare| std::mem::take(&mut *spare))
        .unwrap_or_default();
    let mut retired = queue.take(spare, frames);
    for payload in retired.drain(..) {
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
    if let Ok(mut spare) = SPARE_RETIREMENT.lock() {
        *spare = retired;
    }
}

/// The decode worker's empty retirement storage, swapped in at its next drain.
static SPARE_RETIREMENT: Mutex<Vec<RetiredPayload>> = Mutex::new(Vec::new());

/// Enqueue a frame for decode (or ordered forwarding for skip hours) by value, as tests build
/// their jobs. None unless an allocation failed, which returns the job.
#[cfg(test)]
pub fn enqueue(shared: &Shared, mut job: DecodeJob) -> Option<DecodeJob> {
    if crate::allocation::failed() {
        return Some(job);
    }
    job.queued_qpc = sampling::qpc();
    push(shared, job);
    enqueued(shared);
    None
}

/// Hands the hour's job, filled in place in its frame's slot, to the decode worker. Only a
/// failed allocation refuses it: the job's payloads retire to their pools for the existing
/// gap path and it returns false.
pub fn enqueue_reserved(shared: &Shared, mut reserved: Reserved<'_>) -> bool {
    if crate::allocation::failed() {
        recycle_payloads(reserved.job());
        return false;
    }
    reserved.job().queued_qpc = sampling::qpc();
    reserved.publish();
    enqueued(shared);
    true
}

fn enqueued(shared: &Shared) {
    if let Some(worker) = shared.decode_waker.get() {
        worker.unpark();
    }
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
    let _worker = crate::allocation::enter(crate::allocation::Phase::Decode);
    let mut air_bases_memo = AirBasesMemo::default();
    let mut priority = crate::runtime::OwnPriority::frame_pipeline();
    let mut registered = false;
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
        if !registered {
            let _ = shared.decode_waker.set(std::thread::current());
            priority.follow_pipeline(&shared.frame_priority, crate::frame_priority::Role::Decode);
            registered = true;
        }
        match pop(shared) {
            Some(job) => {
                priority
                    .follow_pipeline(&shared.frame_priority, crate::frame_priority::Role::Decode);
                crate::tick_gate::wait();
                process_owned_job(shared, job, &mut air_bases_memo);
            }
            None => {
                failure::stage("idle retirement", None);
                crate::allocation::attempted_hour(None);
                reclaim_payloads();
                std::thread::park();
            }
        }
    }
    let _phase = crate::allocation::enter(crate::allocation::Phase::Teardown);
    failure::stage("worker retirement", None);
    crate::allocation::attempted_hour(None);
    reclaim_payloads();
}

#[cfg(test)]
pub(crate) fn drain_for_test(shared: &Shared, memo: &mut AirBasesMemo) {
    while let Some(job) = pop(shared) {
        process_owned_job(shared, job, memo);
    }
}

/// The worker owns the raw payloads through decoding, then returns every
/// family to its pool. Dropping this envelope without recycling used to
/// allocate a fresh payload on the game thread on every subsequent hour.
fn process_owned_job(shared: &Shared, mut job: DecodeJob, air_bases_memo: &mut AirBasesMemo) {
    shared.frame_stage(job.index, sampling::FrameStage::Decoding);
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
    // This worker holds the frame alone, off the game thread: charge its whole size, capture
    // and decode growth included, before the spool can queue it, and stamp the frame with it.
    shared.measure_frame(job.index);
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        frame.pool_bytes = shared.pool_total();
    }
    failure::stage("publish ready", None);
    shared.frame_stage(job.index, sampling::FrameStage::Ready);
    let published = publish_ready(shared, job.index, job.game_elapsed);
    // No frame reference or diagnostic lease survives its ready publication.
    drop(failure_scope);
    failure::stage("after ready handoff", None);
    if published {
        shared.wake_spool();
    }
    crate::allocation::attempted_hour(None);
}

/// Hands a decoded frame to the spool, unless an allocation failed: the spool may have stopped
/// and can no longer return it, so teardown joins this worker instead.
fn publish_ready(shared: &Shared, index: usize, elapsed: i64) -> bool {
    let completed = &shared.frames.slot(index).completed;
    if crate::allocation::failed() {
        completed.store(0, Ordering::Release);
        return false;
    }
    shared.frames.push_ready(index);
    completed.store(elapsed.max(1), Ordering::Release);
    true
}

#[cfg(test)]
fn exercise_projects() -> Option<ProjectsJob> {
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
        return None;
    }
    payload.log.take(&memory, 0x1000, 8, 1)?;
    Some(payload)
}

/// One projects decode: whether a refused allocation cost its tables by name while the frame
/// was still published.
#[cfg(test)]
pub(crate) fn allocation_exercise(shared: &Shared, index: usize, hour: i32) -> bool {
    let Some(payload) = exercise_projects() else {
        return false;
    };
    let job = DecodeJob {
        index,
        hour,
        game_elapsed: 1,
        projects_refresh: true,
        projects_payload: Some(payload),
        ..Default::default()
    };
    process_owned_job(shared, job, &mut AirBasesMemo::default());
    let frame = unsafe { &*shared.frames[index].load(Ordering::Acquire) };
    let outcomes = frame.extended.outcomes();
    !crate::allocation::failed()
        && shared.frames.slot(index).completed.load(Ordering::Acquire) != 0
        && PROJECTS.iter().all(|table| {
            outcomes[table.index()]
                == engine_native::walk::Outcome::Unavailable {
                    reason: ALLOCATION_REASON,
                }
        })
}

#[cfg(all(test, feature = "alloc-fault-injection"))]
#[derive(Debug)]
pub(crate) struct RefusalHour {
    pub(crate) terminal: bool,
    pub(crate) published: bool,
    pub(crate) projects: [engine_native::walk::Outcome; PROJECTS.len()],
    pub(crate) capitulation: engine_native::walk::Outcome,
}

/// One hour of two payload decodes, projects and capitulation, on the frame `index`, which the
/// caller holds: `arm` runs once the payload is built, then the hour is decoded, taken back from
/// the ready queue when published, and prepared again.
#[cfg(all(test, feature = "alloc-fault-injection"))]
pub(crate) fn refusal_exercise(
    shared: &Shared,
    index: usize,
    hour: i32,
    arm: impl FnOnce(),
) -> RefusalHour {
    let payload = {
        let _setup = crate::allocation::enter(crate::allocation::Phase::Bootstrap);
        exercise_projects().expect("the projects payload records its span")
    };
    arm();
    let job = DecodeJob {
        index,
        hour,
        game_elapsed: 1,
        projects_refresh: true,
        projects_payload: Some(payload),
        capitulation_refresh: true,
        capitulation_payload: Some(CapitulationJob::new()),
        ..Default::default()
    };
    process_owned_job(shared, job, &mut AirBasesMemo::default());
    let frame = unsafe { &*shared.frames[index].load(Ordering::Acquire) };
    let outcomes = frame.extended.outcomes();
    let hour = RefusalHour {
        terminal: crate::allocation::failed(),
        published: shared.frames.pop_ready() == Some(index),
        projects: PROJECTS.map(|table| outcomes[table.index()]),
        capitulation: outcomes[CAPITULATION[0].index()],
    };
    if !hour.terminal {
        let _setup = crate::allocation::enter(crate::allocation::Phase::Bootstrap);
        shared.prepare_frame_for_reuse(index);
    }
    hour
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
                crate::private_heap::off_hold_path();
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
const ALLOCATION_REASON: &str = "capture buffer allocation failed";

/// One payload decode's tables, as one verdict refuses them.
#[derive(Clone, Copy)]
enum Family {
    Relations,
    Characters(usize),
    Projects,
    Lexical,
    Divisions,
    AirStats,
    AirWing,
    Naval,
    Players,
    Session,
    ProductionRest,
    ConstructionRest,
    EconomyRest,
    AirBases,
    Supremacy,
    Sunk,
    Radar,
    Intel,
    Weather,
    Capitulation,
}

fn fail_family(frame: &mut sampling::Captured, family: Family, reason: &'static str) {
    match family {
        Family::Relations => {
            let captured = frame.extended.relations_pilot();
            captured.fail_relations_capture(captured.relation_activity(), reason);
        }
        Family::Characters(part) => {
            let captured = frame.extended.characters_pilot();
            let active = std::array::from_fn(|index| {
                CHARACTER_MASKS[part][index] && captured.character_activity()[index]
            });
            captured.fail_character_tables(active, reason);
        }
        Family::Projects => frame
            .extended
            .projects_pilot()
            .fail_projects_capture(true, reason),
        Family::Lexical => frame.extended.lexical_pilot().fail_lexical_capture(reason),
        Family::Divisions => {
            frame.divisions_result = engine_native::walk::Outcome::Unavailable { reason };
            frame.extended.install_failed_division_counts(reason);
            frame.extended.fail_division_details(reason);
            frame.extended.set_divisions_raw_mode(false);
        }
        Family::AirStats => frame
            .extended
            .air_stats_pilot()
            .fail_air_stats_capture(reason),
        Family::AirWing => {
            frame.extended.game_score_pilot().fail_air(reason);
            frame
                .extended
                .air_wing_pilot()
                .fail_air_wing_capture(reason);
        }
        Family::Naval => {
            frame.extended.naval_pilot().fail_naval_capture(reason);
            frame.extended.install_reserved_land_certificate();
        }
        Family::Players => frame.map.fail_players_capture(reason),
        Family::Session => frame
            .extended
            .session_pilot()
            .fail_session_capture(reason),
        Family::ProductionRest => frame
            .extended
            .production_rest_pilot()
            .fail_production_rest_capture(reason),
        Family::ConstructionRest => frame
            .extended
            .construction_rest_pilot()
            .fail_construction_rest_capture(reason),
        Family::EconomyRest => frame
            .extended
            .economy_rest_pilot()
            .fail_economy_rest_capture(reason),
        Family::AirBases => {
            frame
                .extended
                .air_bases_pilot()
                .fail_air_bases_capture(reason);
            frame
                .extended
                .radar_pilot()
                .fail_registrations_capture(reason);
        }
        Family::Supremacy => frame
            .extended
            .supremacy_pilot()
            .fail_supremacy_capture(reason),
        Family::Sunk => frame.extended.sunk_pilot().fail_sunk_capture(reason),
        Family::Radar => frame.extended.radar_pilot().fail_radar_capture(reason),
        Family::Intel => frame.extended.intel_pilot().fail_intel_capture(reason),
        Family::Weather => frame.extended.weather_pilot().fail_weather_capture(reason),
        Family::Capitulation => frame
            .extended
            .capitulation_pilot()
            .fail_capitulation_capture(reason),
    }
}

/// A declared handoff without its payload is terminal. An undeclared handoff
/// may already contain typed results or skipped outcomes, which remain intact.
fn fail_missing_payloads(frame: &mut sampling::Captured, job: &DecodeJob) {
    let mut missing = |refresh: bool, present: bool, family: Family| {
        if refresh && !present {
            fail_family(frame, family, MISSING_PAYLOAD_REASON);
        }
    };
    missing(job.relations_refresh, job.relations_payload.is_some(), Family::Relations);
    for (part, &refresh) in job.characters_refresh.iter().enumerate() {
        missing(refresh, job.characters_payload[part].is_some(), Family::Characters(part));
    }
    missing(job.projects_refresh, job.projects_payload.is_some(), Family::Projects);
    missing(job.lexical_refresh, job.lexical_payload.is_some(), Family::Lexical);
    missing(job.divisions_refresh, job.divisions_payload.is_some(), Family::Divisions);
    missing(job.air_stats_refresh, job.air_stats_payload.is_some(), Family::AirStats);
    missing(job.air_wing_refresh, job.air_wing_payload.is_some(), Family::AirWing);
    missing(job.naval_refresh, job.naval_payload.is_some(), Family::Naval);
    missing(job.players_refresh, job.players_payload.is_some(), Family::Players);
    missing(job.session_refresh, job.session_payload.is_some(), Family::Session);
    missing(
        job.production_rest_refresh,
        job.production_rest_payload.is_some(),
        Family::ProductionRest,
    );
    missing(
        job.construction_rest_refresh,
        job.construction_rest_payload.is_some(),
        Family::ConstructionRest,
    );
    missing(
        job.economy_rest_refresh,
        job.economy_rest_payload.is_some(),
        Family::EconomyRest,
    );
    missing(job.air_bases_refresh, job.air_bases_payload.is_some(), Family::AirBases);
    missing(job.supremacy_refresh, job.supremacy_payload.is_some(), Family::Supremacy);
    missing(job.sunk_refresh, job.sunk_payload.is_some(), Family::Sunk);
    missing(job.radar_refresh, job.radar_payload.is_some(), Family::Radar);
    missing(job.intel_refresh, job.intel_payload.is_some(), Family::Intel);
    missing(job.weather_refresh, job.weather_payload.is_some(), Family::Weather);
    missing(
        job.capitulation_refresh,
        job.capitulation_payload.is_some(),
        Family::Capitulation,
    );
}

/// A payload decode runs contained: a refused allocation in it costs its family's tables this
/// hour, by name, instead of the capture, and the next hour decodes again.
fn decode_contained(
    shared: &Shared,
    index: usize,
    family: Family,
    decode: impl FnOnce(&mut sampling::Captured),
) -> bool {
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let ((), refused) = engine_allocation::contained(|| decode(frame));
    if refused {
        fail_family(frame, family, ALLOCATION_REASON);
    }
    refused
}

/// Decode one FIFO item and hand its pooled frame to the spool ready queue
/// in FIFO hour order. Skip items forward without touching typed rows.
fn process_job(
    shared: &Shared,
    job: &mut DecodeJob,
    air_bases_memo: &mut AirBasesMemo,
) {
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
    frame.extended.decode_capture_tails();
    failure::stage("owned census publication", None);
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    frame.map.naval.decode_owned();
    frame.extended.decode_owned_censuses();
    failure::stage("owned mission tails", None);
    frame.extended.decode_mission_tails();
    failure::stage("air copies", None);
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    frame.extended.decode_air_copies();

    let index = job.index;
    if job.relations_refresh
        && let Some(payload) = job.relations_payload.as_ref()
    {
        failure::stage("relations", None);
        let mut decode_ticks = [0; 2];
        decode_contained(shared, index, Family::Relations, |frame| {
            frame
                .extended
                .relations_pilot()
                .decode_relations_payload(payload, &mut decode_ticks);
        });
        account_decode(shared, &RELATIONS, &decode_ticks);
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
        let mut decode_ticks = [0; CHARACTERS.len()];
        decode_contained(shared, index, Family::Characters(part), |frame| {
            frame.extended.characters_pilot().decode_characters_payload(
                shared.base,
                payload,
                &mut decode_ticks,
            );
        });
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.extended.characters_pilot().remember_templates(payload);
        account_decode(shared, &CHARACTERS, &decode_ticks);
    }
    if job.projects_refresh
        && let Some(payload) = job.projects_payload.as_ref()
    {
        failure::stage("projects", None);
        let mut decode_ticks = [0; PROJECTS.len()];
        decode_contained(shared, index, Family::Projects, |frame| {
            frame.extended.projects_pilot().decode_projects_payload(
                shared.base,
                payload,
                &mut decode_ticks,
            );
        });
        account_decode(shared, &PROJECTS, &decode_ticks);
    }
    if job.lexical_refresh
        && let Some(payload) = job.lexical_payload.as_ref()
    {
        failure::stage("lexical", None);
        let mut decode_ticks = [0; 1];
        decode_contained(shared, index, Family::Lexical, |frame| {
            frame
                .extended
                .lexical_pilot()
                .decode_lexical_payload(shared.base, payload, &mut decode_ticks);
        });
        account_decode(shared, &LEXICAL, &decode_ticks);
    }

    if job.divisions_refresh
        && let Some(payload) = job.divisions_payload.as_mut()
    {
        failure::stage("divisions", None);
        let mut decode_ticks = [0; 6];
        decode_contained(shared, index, Family::Divisions, |frame| {
            sampling::decode_divisions_payload(frame, shared.base, payload, &mut decode_ticks);
        });
        account_decode(shared, &DIVISIONS, &decode_ticks);
    }
    if job.air_wing_refresh
        && let Some(payload) = job.air_wing_payload.as_mut()
    {
        failure::stage("air_wing", None);
        let mut decode_ticks = [0; 2];
        decode_contained(shared, index, Family::AirWing, |frame| {
            crate::extended::air::decode_air_payload(
                frame.extended.air_wing_pilot(),
                payload,
                &mut decode_ticks,
            );
            frame
                .extended
                .game_score_pilot()
                .install_air(&mut payload.game_score);
        });
        payload.update_capacity_charge();
        account_decode(shared, &AIR_WING, &decode_ticks);
    }
    if job.air_stats_refresh
        && let Some(payload) = job.air_stats_payload.as_ref()
    {
        failure::stage("air_stats", None);
        let mut decode_ticks = [0; 1];
        decode_contained(shared, index, Family::AirStats, |frame| {
            crate::extended::air::decode_air_stats_payload(
                frame.extended.air_stats_pilot(),
                shared.base,
                payload,
                &mut decode_ticks,
            );
        });
        account_decode(shared, &AIR_STATS, &decode_ticks);
    }
    if job.naval_refresh
        && let Some(payload) = job.naval_payload.as_ref()
    {
        failure::stage("naval", None);
        let mut decode_ticks = [0; 9];
        decode_contained(shared, index, Family::Naval, |frame| {
            frame
                .extended
                .decode_naval_payload(shared.base, payload, &mut decode_ticks);
        });
        account_decode(shared, &NAVAL_ORDER, &decode_ticks);
    }
    if job.players_refresh
        && let Some(payload) = job.players_payload.as_ref()
    {
        failure::stage("players", None);
        let mut decode_ticks = [0; 2];
        decode_contained(shared, index, Family::Players, |frame| {
            frame
                .map
                .decode_players_payload(shared.base, payload, &mut decode_ticks);
        });
        account_decode(shared, &PLAYERS, &decode_ticks);
    }
    if job.session_refresh
        && let Some(payload) = job.session_payload.as_ref()
    {
        failure::stage("session", None);
        let mut decode_ticks = [0; 3];
        decode_contained(shared, index, Family::Session, |frame| {
            frame.extended.session_pilot().decode_session_payload(
                shared.base,
                payload,
                &mut decode_ticks,
            );
        });
        account_decode(shared, &SESSION, &decode_ticks);
    }
    if job.production_rest_refresh
        && let Some(payload) = job.production_rest_payload.as_ref()
    {
        failure::stage("production_rest", None);
        let mut decode_ticks = [0; 8];
        decode_contained(shared, index, Family::ProductionRest, |frame| {
            frame
                .extended
                .production_rest_pilot()
                .decode_production_rest_payload(
                    shared.base,
                    payload.game_state,
                    payload,
                    &mut decode_ticks,
                );
        });
        account_decode(shared, &PRODUCTION_REST, &decode_ticks);
    }
    if job.construction_rest_refresh
        && let Some(payload) = job.construction_rest_payload.as_ref()
    {
        failure::stage("construction_rest", None);
        let mut decode_ticks = [0; 10];
        decode_contained(shared, index, Family::ConstructionRest, |frame| {
            frame.extended.construction_rest_pilot().decode_construction_rest_payload(
                shared.base,
                payload.game_state,
                payload,
                &mut decode_ticks,
            );
        });
        account_decode(shared, &CONSTRUCTION_REST, &decode_ticks);
    }
    if job.economy_rest_refresh
        && let Some(payload) = job.economy_rest_payload.as_ref()
    {
        failure::stage("economy_rest", None);
        let mut decode_ticks = [0; crate::extended::economy::N];
        decode_contained(shared, index, Family::EconomyRest, |frame| {
            frame
                .extended
                .economy_rest_pilot()
                .decode_economy_rest_payload(shared.base, payload, &mut decode_ticks);
        });
        account_decode(shared, &ECONOMY_REST, &decode_ticks);
    }
    if job.air_bases_refresh
        && let Some(payload) = job.air_bases_payload.as_ref()
    {
        failure::stage("air_bases", None);
        let mut decode_ticks = [0; 3];
        let context = AirBasesMemoContext {
            owner: shared as *const Shared as usize,
            base: shared.base,
            seq: job.seq,
            hour: job.hour,
            force_full: job.force_full,
        };
        let refused = decode_contained(shared, index, Family::AirBases, |frame| {
            frame.extended.decode_air_bases_payload(
                shared.base,
                payload,
                air_bases_memo,
                context,
                &mut decode_ticks,
            );
        });
        if refused {
            air_bases_memo.invalidate();
        }
        account_decode(shared, &AIR_BASES, &decode_ticks);
    }
    if job.supremacy_refresh
        && let Some(payload) = job.supremacy_payload.as_ref()
    {
        failure::stage("supremacy", None);
        let mut decode_ticks = [0; 1];
        decode_contained(shared, index, Family::Supremacy, |frame| {
            frame.extended.supremacy_pilot().decode_supremacy_payload(
                shared.base,
                payload,
                &mut decode_ticks,
            );
        });
        account_decode(shared, &SUPREMACY, &decode_ticks);
    }
    if job.sunk_refresh
        && let Some(payload) = job.sunk_payload.as_ref()
    {
        failure::stage("sunk", None);
        let mut decode_ticks = [0; 1];
        decode_contained(shared, index, Family::Sunk, |frame| {
            frame
                .extended
                .sunk_pilot()
                .decode_sunk_payload(shared.base, payload, &mut decode_ticks);
        });
        account_decode(shared, &SUNK, &decode_ticks);
    }
    if job.radar_refresh
        && let Some(payload) = job.radar_payload.as_ref()
    {
        failure::stage("radar", None);
        let mut decode_ticks = [0; 1];
        decode_contained(shared, index, Family::Radar, |frame| {
            frame
                .extended
                .radar_pilot()
                .decode_radar_payload(payload, &mut decode_ticks);
        });
        account_decode(shared, &RADAR, &decode_ticks);
    }
    if job.intel_refresh
        && let Some(payload) = job.intel_payload.as_ref()
    {
        failure::stage("intel", None);
        let mut decode_ticks = [0; 1];
        decode_contained(shared, index, Family::Intel, |frame| {
            frame
                .extended
                .intel_pilot()
                .decode_intel_payload(payload, &mut decode_ticks);
        });
        account_decode(shared, &INTEL, &decode_ticks);
    }
    if job.weather_refresh
        && let Some(payload) = job.weather_payload.as_ref()
    {
        failure::stage("weather", None);
        let mut decode_ticks = [0; 2];
        let mut tracker = shared
            .weather_temperature
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let refused = decode_contained(shared, index, Family::Weather, |frame| {
            frame.extended.weather_pilot().decode_weather_payload(
                shared.base,
                payload,
                job.hour,
                &mut tracker,
                &mut decode_ticks,
            );
        });
        if refused {
            // A refused hour's words are no comparison for the next.
            let _ = tracker.observe(job.hour, None);
        }
        account_decode(shared, &WEATHER, &decode_ticks);
    }
    if job.capitulation_refresh
        && let Some(payload) = job.capitulation_payload.as_ref()
    {
        failure::stage("capitulation", None);
        let mut decode_ticks = [0; 1];
        decode_contained(shared, index, Family::Capitulation, |frame| {
            frame
                .extended
                .capitulation_pilot()
                .decode_capitulation_payload(shared.base, payload, &mut decode_ticks);
        });
        account_decode(shared, &CAPITULATION, &decode_ticks);
    }
}

#[inline]
fn account_decode(shared: &Shared, tables: &[TableId], ticks: &[u64]) {
    #[cfg(feature = "acquisition_profile")]
    for (&table, &ticks) in tables.iter().zip(ticks) {
        collector_memory::acquisition_profile::overlapped(table, shared.ns(ticks as i64));
    }
    #[cfg(not(feature = "acquisition_profile"))]
    let _ = (shared, tables, ticks);
}

/// The FIFO worker owns the frame; this packet has no raw decode dependency.
/// Return its held-store loan before unrelated families can delay the next hour.
fn decode_presentation(shared: &Shared, index: usize) {
    failure::stage("division presentation", None);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    #[cfg(feature = "acquisition_profile")]
    let started = sampling::qpc();
    let ticks = frame.extended.decode_division_presentation_inputs();
    account_decode(shared, &[TableId::DivisionMovement], &ticks[..1]);
    #[cfg(feature = "acquisition_profile")]
    account_decode(shared, &[TableId::DivisionStatus], &[
        (sampling::qpc().saturating_sub(started).max(0) as u64).saturating_sub(ticks[0])
    ]);
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
    if let Some(handle) = &worker {
        handle.thread().unpark();
    }
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
        crate::land_events::fail();
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
        while let Some(job) = pop(shared) {
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
        crate::land_events::fail();
        let reason = failure::joined(
            payload,
            "decode detach drain panicked; queued frames remain incomplete",
        );
        error.get_or_insert(reason);
    }
    STOP.store(false, Ordering::Release);
    panic_failure().or(error).map_or(Ok(()), Err)
}

/// Frame `index`'s decode job, to fill in place: the game thread never moves a job (tens of
/// KiB) through its stack. The caller holds the frame.
pub fn reserve(shared: &Shared, index: usize) -> Reserved<'_> {
    Reserved { shared, index }
}

/// The oldest queued job, moved out of its frame's slot.
pub fn pop(shared: &Shared) -> Option<DecodeJob> {
    let index = shared.frames.pop_decode()?;
    // SAFETY: popping the index hands its frame, and so its job, to this consumer.
    Some(std::mem::take(unsafe { shared.frames.job(index) }))
}

/// Queues a job built by value into its frame's slot, as tests build them.
#[cfg(test)]
pub fn push(shared: &Shared, job: DecodeJob) {
    let mut reserved = reserve(shared, job.index);
    *reserved.job() = job;
    reserved.publish();
}

/// A frame's job, until published. Dropped unpublished, it is left vacant: payloads it still
/// holds are dropped, like the locals they replace.
pub struct Reserved<'a> {
    shared: &'a Shared,
    index: usize,
}

impl Reserved<'_> {
    pub fn job(&mut self) -> &mut DecodeJob {
        // SAFETY: this reservation's caller holds the frame.
        unsafe { self.shared.frames.job(self.index) }
    }

    /// Hands the job to the consumer.
    fn publish(self) {
        self.shared.frames.push_decode(self.index);
        std::mem::forget(self);
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
    }
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    prepare_retirement().unwrap();
    reclaim_payloads();
    // Production prepares bounded payload storage before installing the hook.
    // Tests exercise that lifecycle rather than relying on cold tick allocation.
    std::mem::forget(prepare_payload_pools(false).unwrap());
    SEQ.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use tests::retire_and_reacquire_two_carrier_jobs_for_test;

#[cfg(test)]
mod order_group_loan_tests;
