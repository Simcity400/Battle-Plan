use crate::sampling;
use crate::sampling::DivisionsJob;
use crate::sampling::Shared;
use collector_air::extended::{
    air::{AirBasesJob, AirBasesMemo, AirBasesMemoContext, AirStatsJob, AirWingJob},
    fog::{IntelJob, RadarJob},
};
use collector_characters::extended::characters::{
    CharactersJob, PART_MASKS as CHARACTER_MASKS, PARTS as CHARACTER_PARTS,
};
use collector_construction::construction::ConstructionRestJob;
use collector_economy::economy::EconomyRestJob;
use collector_intel::intel::CapitulationJob;
use collector_intel::intel::RelationsJob;
use collector_lexical::lexical::LexicalJob;
use collector_map_capture::map_capture::PlayersJob;
use collector_naval_history::extended::naval_history::{NavalJob, SunkJob, SupremacyJob};
use collector_production::production::ProductionRestJob;
use collector_projects::projects::ProjectsJob;
use collector_session::session::SessionJob;
use collector_weather::weather::WeatherJob;
use collector_workers::thread_cpu::{decode_worker, set_decode_worker};
use replay_data::rows::TableId;

use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering},
};
mod pools;

pub use pools::*;

pub const RELATIONS: [TableId; 2] = [TableId::RelationOpinions, TableId::RelationTuples];

pub(crate) const CHARACTERS: [TableId; collector_characters::extended::characters::N] =
    collector_characters::extended::characters::TABLES;

pub(crate) const PROJECTS: [TableId; 2] = collector_projects::projects::RAW;

pub(crate) const LEXICAL: [TableId; 1] = [TableId::LexicalTokens];
pub const DIVISIONS: [TableId; 6] = [
    TableId::Divisions,
    TableId::DivisionEquipment,
    TableId::ArmyUpgradeGroups,
    TableId::ArmyUpgradeRequests,
    TableId::ArmyUpgradeDeliveries,
    TableId::ArmyUpgradeProduced,
];
pub(crate) const PLAYERS: [TableId; 2] = [TableId::Players, TableId::HumanControllers];

pub(crate) const SESSION: [TableId; 3] = [
    TableId::WorldTension,
    TableId::ThreatSources,
    TableId::ThreatCountryAggregates,
];

pub(crate) const AIR_WING: [TableId; 2] = [TableId::WingEquipment, TableId::WingCumulative];

pub(crate) const AIR_STATS: [TableId; 1] = [TableId::AirStatistics];

pub(crate) const AIR_BASES: [TableId; 3] = [
    TableId::AirBases,
    TableId::AirBaseRegistrations,
    TableId::AirAces,
];

pub(crate) const SUPREMACY: [TableId; 1] = [TableId::NavalSupremacy];

pub const SUNK: [TableId; 1] = [TableId::SunkShips];

pub(crate) const RADAR: [TableId; 1] = [TableId::RadarCoverage];

pub(crate) const INTEL: [TableId; 1] = [TableId::UnitIntel];

pub(crate) const NAVAL_ORDER: [TableId; 9] = [
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

pub(crate) const PRODUCTION_REST: [TableId; 8] = [
    TableId::ProductionLines,
    TableId::Stockpiles,
    TableId::ProductionLicenses,
    TableId::DeploymentConveyors,
    TableId::DeploymentLines,
    TableId::DeploymentManpower,
    TableId::DeploymentEquipment,
    TableId::ProductionLicenseRecords,
];

pub(crate) const CONSTRUCTION_REST: [TableId; 10] = [
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

pub(crate) const ECONOMY_REST: [TableId; collector_economy::economy::N] = [
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

pub(crate) const WEATHER: [TableId; 2] = [TableId::WeatherRegions, TableId::WeatherProvinces];

pub(crate) const CAPITULATION: [TableId; 1] = [TableId::Capitulation];

#[derive(Debug, Default)]
pub struct DecodeJob {
    pub index: usize,
    pub seq: u64,
    pub hour: i32,
    pub(crate) relations_refresh: bool,
    pub(crate) characters_refresh: [bool; CHARACTER_PARTS],
    pub(crate) projects_refresh: bool,
    pub(crate) lexical_refresh: bool,
    pub(crate) divisions_refresh: bool,
    pub(crate) air_wing_refresh: bool,
    pub(crate) air_stats_refresh: bool,
    pub(crate) naval_refresh: bool,
    pub(crate) players_refresh: bool,
    pub(crate) session_refresh: bool,
    pub(crate) production_rest_refresh: bool,
    pub(crate) construction_rest_refresh: bool,
    pub(crate) economy_rest_refresh: bool,
    pub(crate) air_bases_refresh: bool,
    pub(crate) supremacy_refresh: bool,
    pub(crate) sunk_refresh: bool,
    pub(crate) radar_refresh: bool,
    pub(crate) intel_refresh: bool,
    pub(crate) weather_refresh: bool,
    pub(crate) capitulation_refresh: bool,
    pub force_full: bool,
    pub(crate) game_elapsed: i64,
    pub(crate) queued_qpc: i64,
    pub(crate) relations_payload: Option<RelationsJob>,
    pub(crate) characters_payload: [Option<CharactersJob>; CHARACTER_PARTS],
    pub(crate) projects_payload: Option<ProjectsJob>,
    pub(crate) lexical_payload: Option<LexicalJob>,
    pub(crate) divisions_payload: Option<DivisionsJob>,
    pub(crate) air_wing_payload: Option<AirWingJob>,
    pub(crate) air_stats_payload: Option<AirStatsJob>,
    pub(crate) naval_payload: Option<NavalJob>,
    pub(crate) players_payload: Option<PlayersJob>,
    pub(crate) session_payload: Option<SessionJob>,
    pub(crate) production_rest_payload: Option<ProductionRestJob>,
    pub(crate) construction_rest_payload: Option<ConstructionRestJob>,
    pub(crate) economy_rest_payload: Option<EconomyRestJob>,
    pub(crate) air_bases_payload: Option<AirBasesJob>,
    pub(crate) supremacy_payload: Option<SupremacyJob>,
    pub(crate) sunk_payload: Option<SunkJob>,
    pub(crate) radar_payload: Option<RadarJob>,
    pub(crate) intel_payload: Option<IntelJob>,
    pub(crate) weather_payload: Option<WeatherJob>,
    pub(crate) capitulation_payload: Option<CapitulationJob>,
}
static SEQ: AtomicU64 = AtomicU64::new(0);
static STOP: AtomicBool = AtomicBool::new(false);
static WORKER: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

#[cfg(test)]
use collector_workers::panic_failure::HookGuard as PanicHook;
use collector_workers::panic_failure::{self as failure, first as panic_failure};

pub fn worker_finished() -> bool {
    WORKER.lock().map_or(true, |slot| {
        slot.as_ref().is_none_or(|handle| handle.is_finished())
    })
}
pub(crate) fn relations_refresh_hour(hour: i32, force_full: bool) -> bool {
    RELATIONS
        .iter()
        .any(|table| replay_data::cadence::should_walk(*table, hour, force_full))
}

pub(crate) fn characters_refresh_hour(hour: i32, force_full: bool) -> bool {
    CHARACTERS
        .iter()
        .any(|table| replay_data::cadence::should_walk(*table, hour, force_full))
}

pub(crate) fn projects_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay_data::cadence::should_walk(PROJECTS[0], hour, force_full);
    for table in PROJECTS.iter().skip(1) {
        debug_assert_eq!(
            walk,
            replay_data::cadence::should_walk(*table, hour, force_full)
        );
    }
    walk
}

pub(crate) fn lexical_refresh_hour(hour: i32, force_full: bool) -> bool {
    replay_data::cadence::should_walk(LEXICAL[0], hour, force_full)
}

/// # Safety
/// Only an admitted capture may advance the session's decode sequence, once for each job published
/// to decode.
pub(crate) unsafe fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

const PAYLOADS_PER_FRAME: usize = 19 + CHARACTER_PARTS;

#[expect(
    clippy::large_enum_variant,
    reason = "retired payloads sit inline in one reused list so retiring never allocates per payload"
)]
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

    fn take(&self, mut spare: Vec<RetiredPayload>, frames: usize) -> Vec<RetiredPayload> {
        let room = frames.saturating_mul(PAYLOADS_PER_FRAME);
        if spare.capacity() < room {
            let _ =
                engine_allocation::advisory(|| engine_allocation::reserve_exact(&mut spare, room));
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
        unsafe {
            drop(Box::from_raw(queue));
        }
    }
    Ok(())
}

pub struct PayloadPools(());

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
            unsafe {
                drop(Box::from_raw(queue));
            }
        }
    }
}

/// # Safety
/// Only the attaching runtime may prepare the global payload pools, before starting decode or
/// publishing any hook. Retain the returned guard until acquisition and decode are joined and their
/// payloads returned.
pub unsafe fn prepare_payload_pools(game_score: bool) -> std::io::Result<PayloadPools> {
    let pools = PayloadPools(());
    prepare_retirement()?;
    for _ in 0..JOB_POOL_STEADY {
        if collector_memory::terminal::failed() {
            break;
        }
        release_relations_job(RelationsJob::new());
        for part in 0..CHARACTER_PARTS {
            release_characters_job(CharactersJob::new_part(part));
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
    if collector_memory::terminal::failed() {
        return Err(std::io::ErrorKind::OutOfMemory.into());
    }
    Ok(pools)
}

fn defer_payload(payload: RetiredPayload) {
    let queue = RETIRED_PAYLOADS.load(Ordering::Acquire);
    if queue.is_null() && collector_memory::terminal::failed() {
        return;
    }
    assert!(
        !queue.is_null(),
        "payload retirement initialized before hook"
    );
    let queue = unsafe { &*queue };
    queue.push(payload);
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

static SPARE_RETIREMENT: Mutex<Vec<RetiredPayload>> = Mutex::new(Vec::new());

#[cfg(test)]
/// # Safety
/// The caller must own the named frame's unpublished decode job and transfer it to the active
/// session's sole decode worker exactly once, before recycling the frame or releasing the payload
/// pools.
pub(crate) unsafe fn enqueue(shared: &Shared, mut job: DecodeJob) -> Option<DecodeJob> {
    if collector_memory::terminal::failed() {
        return Some(job);
    }
    job.queued_qpc = collector_primitives::clock::qpc();
    // SAFETY: The decode consumer is returning a job for its original slot before releasing the
    // slot.
    unsafe { push(shared, job) };
    enqueued(shared);
    None
}

/// # Safety
/// The caller must own the named frame's unpublished decode job and transfer it to the active
/// session's sole decode worker exactly once, before recycling the frame or releasing the payload
/// pools.
pub(crate) unsafe fn enqueue_reserved(shared: &Shared, mut reserved: Reserved<'_>) -> bool {
    if collector_memory::terminal::failed() {
        recycle_payloads(reserved.job());
        return false;
    }
    reserved.job().queued_qpc = collector_primitives::clock::qpc();
    reserved.publish();
    enqueued(shared);
    true
}

fn enqueued(shared: &Shared) {
    if let Some(worker) = shared.decode_waker.get() {
        worker.unpark();
    }
}

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

fn decode_loop() {
    let _worker = collector_memory::terminal::enter(collector_memory::terminal::Phase::Decode);
    let mut air_bases_memo = AirBasesMemo::default();
    let mut priority =
    // SAFETY: This is the dedicated worker applying its own session's priority policy.
    unsafe { collector_workers::priority::OwnPriority::frame_pipeline() };
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
            // SAFETY: This owned worker updates its priority from its session's own pipeline.
            unsafe {
                priority.follow_pipeline(
                    &shared.frame_priority,
                    collector_workers::frame_priority::Role::Decode,
                )
            };
            registered = true;
        }
        match
        // SAFETY: This consumer owns the queue removal for the named pool.
        unsafe { pop(shared) } {
            Some(job) => {
                // SAFETY: This owned worker updates its priority from its session's own pipeline.
                unsafe { priority.follow_pipeline(
                    &shared.frame_priority,
                    collector_workers::frame_priority::Role::Decode,
                ) };
                collector_workers::tick_gate::wait();
                process_owned_job(shared, job, &mut air_bases_memo);
                collector_workers::private_heap::PRIVATE_HEAP.maybe_flush_thread_cache();
            }
            None => {
                failure::stage("idle retirement", None);
                collector_memory::terminal::attempted_hour(None);
                reclaim_payloads();
                collector_workers::private_heap::PRIVATE_HEAP.maybe_flush_thread_cache();
                std::thread::park();
            }
        }
    }
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Teardown);
    failure::stage("worker retirement", None);
    collector_memory::terminal::attempted_hour(None);
    reclaim_payloads();
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn drain_for_test(shared: &Shared, memo: &mut AirBasesMemo) {
    while let Some(job) =
        // SAFETY: This consumer owns the queue removal for the named pool.
        unsafe { pop(shared) }
    {
        process_owned_job(shared, job, memo);
    }
}

fn process_owned_job(shared: &Shared, job: DecodeJob, air_bases_memo: &mut AirBasesMemo) {
    process_owned_job_with_stage(shared, job, air_bases_memo, failure::stage);
}

fn process_owned_job_with_stage(
    shared: &Shared,
    mut job: DecodeJob,
    air_bases_memo: &mut AirBasesMemo,
    stage: impl Fn(&'static str, Option<usize>),
) {
    // SAFETY: This stage owns the installed frame slot.
    unsafe { shared.frame_stage(job.index, sampling::FrameStage::Decoding) };
    let failure_scope = failure::Scope::frame(failure::Frame {
        index: job.index,
        seq: job.seq,
        hour: job.hour,
        force_full: job.force_full,
        load_signal: None,
        world_generation: None,
    });
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Decode);
    let frame = unsafe { &*shared.frames[job.index].load(Ordering::Acquire) };
    failure::Scope::identity(
        frame
            .clock
            .as_ref()
            .map(|clock| clock.play_session_started_at),
        frame.world_generation,
    );
    if let Some(clock) = frame.clock.as_ref() {
        collector_memory::terminal::set_context(collector_memory::terminal::Context {
            campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
            load_signal: Some(clock.play_session_started_at),
            failed_hour: Some(job.hour),
            ..Default::default()
        });
    } else {
        collector_memory::terminal::attempted_hour(Some(job.hour));
    }
    failure::stage("frame retirement", None);
    reclaim_payloads();
    if !collector_memory::terminal::failed() {
        process_job(shared, &mut job, air_bases_memo, &stage);
    }

    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    // SAFETY: This frame's owner is returning its outstanding naval loan before reuse.
    unsafe { frame.map.naval.discard_owned() };
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
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        shared.set_frame_bytes(job.index, frame.allocated_bytes() as u64);
        frame.pool_bytes = shared.pool_total();
    }
    failure::stage("publish ready", None);
    // SAFETY: This stage owns the installed frame slot.
    unsafe { shared.frame_stage(job.index, sampling::FrameStage::Ready) };
    publish_ready(shared, job.index, job.game_elapsed);
    drop(failure_scope);
    failure::stage("after ready handoff", None);
    collector_memory::terminal::attempted_hour(None);
}

fn publish_ready(shared: &Shared, index: usize, elapsed: i64) -> bool {
    let completed = &shared.frames.slot(index).completed;
    if collector_memory::terminal::failed() {
        completed.store(0, Ordering::Release);
        return false;
    }
    hand_ready(shared, index, elapsed);
    true
}

pub(crate) fn hand_ready(shared: &Shared, index: usize, elapsed: i64) {
    shared.frames.push_ready(index);
    shared
        .frames
        .slot(index)
        .completed
        .store(elapsed.max(1), Ordering::Release);
    shared.wake_spool();
}

#[cfg(any(test, feature = "test-support"))]
fn exercise_projects() -> Option<ProjectsJob> {
    use engine_native_memory::memory::SliceMemory;
    let bytes = [7_u8; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut payload = ProjectsJob::default();
    payload.log = collector_raw::raw::RawLog::new(4, 64);
    payload.active = true;
    if collector_memory::terminal::failed() {
        return None;
    }
    payload.log.take(&memory, 0x1000, 8, 1)?;
    Some(payload)
}

#[cfg(any(test, feature = "test-support"))]
/// # Safety
/// Only an isolated collector test may run this fixture. The test must own the global capture,
/// decode and allocation-failure state and supply the frame claimed by that fixture.
pub unsafe fn allocation_exercise(shared: &Shared, index: usize, hour: i32) -> bool {
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
    !collector_memory::terminal::failed()
        && shared.frames.slot(index).completed.load(Ordering::Acquire) != 0
        && PROJECTS.iter().all(|table| {
            outcomes[table.index()]
                == engine_native_memory::walk::Outcome::Unavailable {
                    reason: ALLOCATION_REASON,
                }
        })
}

#[cfg(all(any(test, feature = "test-support"), feature = "alloc-fault-injection"))]
#[derive(Debug)]
pub struct RefusalHour {
    pub terminal: bool,
    pub published: bool,
    pub projects: [engine_native_memory::walk::Outcome; PROJECTS.len()],
    pub capitulation: engine_native_memory::walk::Outcome,
}

#[cfg(all(any(test, feature = "test-support"), feature = "alloc-fault-injection"))]
/// # Safety
/// Only an isolated collector test may run this fixture. The test must own the global capture,
/// decode and allocation-failure state and supply the frame claimed by that fixture.
pub unsafe fn refusal_exercise(
    shared: &Shared,
    index: usize,
    hour: i32,
    arm: impl FnOnce(),
) -> RefusalHour {
    let payload = {
        let _setup =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Bootstrap);
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
        terminal: collector_memory::terminal::failed(),
        published:
        // SAFETY: The sole spool consumer takes responsibility for the decoded slot until recycle.
        unsafe { shared.frames.pop_ready() } == Some(index),
        projects: PROJECTS.map(|table| outcomes[table.index()]),
        capitulation: outcomes[CAPITULATION[0].index()],
    };
    if !hour.terminal {
        let _setup =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Bootstrap);
        // SAFETY: This owner has finished using the frame and no other stage or helper borrows it.
        unsafe { shared.prepare_frame_for_reuse(index) };
    }
    hour
}

/// # Safety
/// Only the runtime may start decode, once after preparing its payload pools and before the hourly
/// hook can submit work. Join it through detach_drain before releasing that storage.
pub unsafe fn start_worker() -> Result<(), &'static str> {
    let mut slot = WORKER.lock().map_err(|_| "decode worker lock poisoned")?;
    if slot.as_ref().is_some_and(|handle| !handle.is_finished()) {
        return Err("decode worker already running");
    }
    STOP.store(false, Ordering::Release);
    // SAFETY: Startup and joined detach own this session's decode registration.
    unsafe { set_decode_worker(0) };
    *slot = Some(
        std::thread::Builder::new()
            .name("hoi4-decode".into())
            .stack_size(collector_workers::priority::BACKGROUND_STACK_BYTES)
            .spawn(|| {
                collector_workers::private_heap::off_hold_path();
                #[cfg(feature = "worker_sampler")]
                // SAFETY: This runtime-owned worker keeps the guard until it exits.
                let _sampler_exit =
                    unsafe { collector_workers::thread_cpu::StopOnThreadExit::new() };
                #[cfg(feature = "worker_sampler")]
                // SAFETY: This is the session decode thread and its exit path joins the sampler.
                unsafe {
                    collector_workers::thread_cpu::register_decode()
                };
                // SAFETY: Startup and joined detach own this session's decode registration.
                unsafe {
                    set_decode_worker(windows_sys::Win32::System::Threading::GetCurrentThreadId())
                };
                failure::worker(decode_loop);
            })
            .map_err(|_| "cannot start decode worker")?,
    );
    while decode_worker() == 0 {
        std::thread::yield_now();
    }
    Ok(())
}

const MISSING_PAYLOAD_REASON: &str = "missing active decode payload";
const ALLOCATION_REASON: &str = "capture buffer allocation failed";

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
            frame.divisions_result = engine_native_memory::walk::Outcome::Unavailable { reason };
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
        Family::Session => frame.extended.session_pilot().fail_session_capture(reason),
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

fn fail_missing_payloads(frame: &mut sampling::Captured, job: &DecodeJob) {
    let mut missing = |refresh: bool, present: bool, family: Family| {
        if refresh && !present {
            fail_family(frame, family, MISSING_PAYLOAD_REASON);
        }
    };
    missing(
        job.relations_refresh,
        job.relations_payload.is_some(),
        Family::Relations,
    );
    for (part, &refresh) in job.characters_refresh.iter().enumerate() {
        missing(
            refresh,
            job.characters_payload[part].is_some(),
            Family::Characters(part),
        );
    }
    missing(
        job.projects_refresh,
        job.projects_payload.is_some(),
        Family::Projects,
    );
    missing(
        job.lexical_refresh,
        job.lexical_payload.is_some(),
        Family::Lexical,
    );
    missing(
        job.divisions_refresh,
        job.divisions_payload.is_some(),
        Family::Divisions,
    );
    missing(
        job.air_stats_refresh,
        job.air_stats_payload.is_some(),
        Family::AirStats,
    );
    missing(
        job.air_wing_refresh,
        job.air_wing_payload.is_some(),
        Family::AirWing,
    );
    missing(
        job.naval_refresh,
        job.naval_payload.is_some(),
        Family::Naval,
    );
    missing(
        job.players_refresh,
        job.players_payload.is_some(),
        Family::Players,
    );
    missing(
        job.session_refresh,
        job.session_payload.is_some(),
        Family::Session,
    );
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
    missing(
        job.air_bases_refresh,
        job.air_bases_payload.is_some(),
        Family::AirBases,
    );
    missing(
        job.supremacy_refresh,
        job.supremacy_payload.is_some(),
        Family::Supremacy,
    );
    missing(job.sunk_refresh, job.sunk_payload.is_some(), Family::Sunk);
    missing(
        job.radar_refresh,
        job.radar_payload.is_some(),
        Family::Radar,
    );
    missing(
        job.intel_refresh,
        job.intel_payload.is_some(),
        Family::Intel,
    );
    missing(
        job.weather_refresh,
        job.weather_payload.is_some(),
        Family::Weather,
    );
    missing(
        job.capitulation_refresh,
        job.capitulation_payload.is_some(),
        Family::Capitulation,
    );
}

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

fn process_job(
    shared: &Shared,
    job: &mut DecodeJob,
    air_bases_memo: &mut AirBasesMemo,
    stage: impl Fn(&'static str, Option<usize>),
) {
    stage("missing payload verdicts", None);
    if !job.air_bases_refresh || job.air_bases_payload.is_none() {
        air_bases_memo.invalidate();
    }
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    fail_missing_payloads(frame, job);
    decode_presentation(shared, job.index);
    stage("owned capture tails", None);
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    frame.extended.decode_capture_tails();
    stage("owned census publication", None);
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    // SAFETY: The decode path settles this frame's naval loan before reuse.
    unsafe { frame.map.naval.decode_owned() };
    frame.extended.decode_owned_censuses();
    stage("owned mission tails", None);
    frame.extended.decode_mission_tails();
    stage("air copies", None);
    let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
    frame.extended.decode_air_copies();

    let index = job.index;
    if job.relations_refresh
        && let Some(payload) = job.relations_payload.as_ref()
    {
        stage("relations", None);
        let mut decode_ticks = [0; 2];
        decode_contained(shared, index, Family::Relations, |frame| {
            frame
                .extended
                .relations_pilot()
                .decode_relations_payload(payload, &mut decode_ticks);
        });
        account_decode(shared, &RELATIONS, &decode_ticks);
    }
    for (part, (refresh, slot)) in job
        .characters_refresh
        .iter()
        .zip(&mut job.characters_payload)
        .enumerate()
    {
        if !*refresh {
            continue;
        }
        let Some(mut payload) = slot.take() else {
            continue;
        };
        stage("characters", Some(part));
        let mut decode_ticks = [0; CHARACTERS.len()];
        decode_contained(shared, index, Family::Characters(part), |frame| {
            frame.extended.characters_pilot().decode_characters_payload(
                shared.base,
                &mut payload,
                &mut decode_ticks,
            );
        });
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame
            .extended
            .characters_pilot()
            .remember_templates(&mut payload);
        account_decode(shared, &CHARACTERS, &decode_ticks);
        release_characters_job(payload);
    }
    if job.projects_refresh
        && let Some(payload) = job.projects_payload.as_ref()
    {
        stage("projects", None);
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
        stage("lexical", None);
        let mut decode_ticks = [0; 1];
        decode_contained(shared, index, Family::Lexical, |frame| {
            frame.extended.lexical_pilot().decode_lexical_payload(
                shared.base,
                payload,
                &mut decode_ticks,
            );
        });
        account_decode(shared, &LEXICAL, &decode_ticks);
    }

    if job.divisions_refresh
        && let Some(payload) = job.divisions_payload.as_mut()
    {
        stage("divisions", None);
        let mut decode_ticks = [0; 6];
        decode_contained(shared, index, Family::Divisions, |frame| {
            sampling::decode_divisions_payload(frame, shared.base, payload, &mut decode_ticks);
        });
        account_decode(shared, &DIVISIONS, &decode_ticks);
    }
    if job.air_wing_refresh
        && let Some(payload) = job.air_wing_payload.as_mut()
    {
        stage("air_wing", None);
        let mut decode_ticks = [0; 2];
        decode_contained(shared, index, Family::AirWing, |frame| {
            collector_air::extended::air::decode_air_payload(
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
        stage("air_stats", None);
        let mut decode_ticks = [0; 1];
        decode_contained(shared, index, Family::AirStats, |frame| {
            collector_air::extended::air::decode_air_stats_payload(
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
        stage("naval", None);
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
        stage("players", None);
        let mut decode_ticks = [0; 2];
        decode_contained(shared, index, Family::Players, |frame| {
            // SAFETY: Decode is settling the matching job into its originating frame.
            unsafe {
                frame
                    .map
                    .decode_players_payload(shared.base, payload, &mut decode_ticks)
            };
        });
        account_decode(shared, &PLAYERS, &decode_ticks);
    }
    if job.session_refresh
        && let Some(payload) = job.session_payload.as_ref()
    {
        stage("session", None);
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
        stage("production_rest", None);
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
        stage("construction_rest", None);
        let mut decode_ticks = [0; 10];
        decode_contained(shared, index, Family::ConstructionRest, |frame| {
            frame
                .extended
                .construction_rest_pilot()
                .decode_construction_rest_payload(
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
        stage("economy_rest", None);
        let mut decode_ticks = [0; collector_economy::economy::N];
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
        stage("air_bases", None);
        let mut decode_ticks = [0; 3];
        let context = AirBasesMemoContext {
            owner: std::ptr::from_ref::<Shared>(shared) as usize,
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
        stage("supremacy", None);
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
        stage("sunk", None);
        let mut decode_ticks = [0; 1];
        decode_contained(shared, index, Family::Sunk, |frame| {
            frame.extended.sunk_pilot().decode_sunk_payload(
                shared.base,
                payload,
                &mut decode_ticks,
            );
        });
        account_decode(shared, &SUNK, &decode_ticks);
    }
    if job.radar_refresh
        && let Some(payload) = job.radar_payload.as_ref()
    {
        stage("radar", None);
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
        stage("intel", None);
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
        stage("weather", None);
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
            let _ = tracker.observe(job.hour, None);
        }
        account_decode(shared, &WEATHER, &decode_ticks);
    }
    if job.capitulation_refresh
        && let Some(payload) = job.capitulation_payload.as_ref()
    {
        stage("capitulation", None);
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

fn decode_presentation(shared: &Shared, index: usize) {
    failure::stage("division presentation", None);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    #[cfg(feature = "acquisition_profile")]
    let started = collector_primitives::clock::qpc();
    let ticks = frame.extended.decode_division_presentation_inputs();
    account_decode(shared, &[TableId::DivisionMovement], &ticks[..1]);
    #[cfg(feature = "acquisition_profile")]
    account_decode(
        shared,
        &[TableId::DivisionStatus],
        &[(collector_primitives::clock::qpc()
            .saturating_sub(started)
            .max(0) as u64)
            .saturating_sub(ticks[0])],
    );
    if let Some(store) = frame.extended.take_settled_army_store() {
        shared.return_army_stat_store(store);
    }
}

/// # Safety
/// Only the runtime may drain and join its decode worker, after closing hourly admission and
/// quiescing capture. Keep shared state and the payload pools alive until it returns.
pub unsafe fn detach_drain(mut drained: impl FnMut(u64)) -> Result<(), &'static str> {
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
        // SAFETY: This owned sampled worker completes the exit handshake before leaving.
        unsafe { collector_workers::worker_sampler_state::stop_before_thread_exit() };
    }
    if let Some(handle) = worker
        && let Err(payload) = handle.join()
    {
        collector_equipment_destruction::equipment_destruction::lost_drain();
        collector_naval_results::naval_result_sources::lost_drain();
        // SAFETY: Decode panic recovery owns recording lost observer intervals for this session.
        unsafe {
            collector_observers::air_losses::lost_drain();
            collector_observers::building_bombing::lost_drain();
            collector_observers::combat_engagement_refreshes::lost_drain();
            collector_observers::nuclear_strikes::lost_drain();
            collector_land_events::land_events::fail();
        }
        error = Some(failure::joined(
            payload,
            "decode worker panicked; its in-flight frame may be incomplete",
        ));
    }
    // SAFETY: Startup and joined detach own this session's decode registration.
    unsafe { set_decode_worker(0) };
    let shared_ptr = sampling::SHARED.load(Ordering::Acquire);
    if shared_ptr.is_null() {
        STOP.store(false, Ordering::Release);
        return panic_failure().or(error).map_or(Ok(()), Err);
    }
    let shared = unsafe { &*shared_ptr };
    let mut air_bases_memo = AirBasesMemo::default();
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut count = 0;
        while let Some(job) =
            // SAFETY: This consumer owns the queue removal for the named pool.
            unsafe { pop(shared) }
        {
            process_owned_job(shared, job, &mut air_bases_memo);
            count += 1;
            failure::stage("drain progress", None);
            drained(count);
        }
    })) {
        collector_equipment_destruction::equipment_destruction::lost_drain();
        collector_naval_results::naval_result_sources::lost_drain();
        // SAFETY: Decode panic recovery owns recording lost observer intervals for this session.
        unsafe {
            collector_observers::air_losses::lost_drain();
            collector_observers::building_bombing::lost_drain();
            collector_observers::combat_engagement_refreshes::lost_drain();
            collector_observers::nuclear_strikes::lost_drain();
            collector_land_events::land_events::fail();
        }
        let reason = failure::joined(
            payload,
            "decode detach drain panicked; queued frames remain incomplete",
        );
        error.get_or_insert(reason);
    }
    STOP.store(false, Ordering::Release);
    panic_failure().or(error).map_or(Ok(()), Err)
}

/// # Safety
/// The caller must exclusively own an installed frame at index in shared, with no decode job queued
/// for it, and must settle or transfer that claim before recycling the frame.
pub(crate) unsafe fn reserve(shared: &Shared, index: usize) -> Reserved<'_> {
    Reserved { shared, index }
}

/// # Safety
/// Only the sole decode consumer may remove a queued job, and it must hold the corresponding frame
/// until the job is settled.
pub(crate) unsafe fn pop(shared: &Shared) -> Option<DecodeJob> {
    let index =
    // SAFETY: The sole decode consumer owns this queue and takes responsibility for the slot.
    unsafe { shared.frames.pop_decode() }?;
    // SAFETY: popping the index hands its frame, and so its job, to this consumer.
    Some(std::mem::take(unsafe { shared.frames.job(index) }))
}

#[cfg(test)]
/// # Safety
/// Only the sole decode consumer may requeue its own job, while the frame remains installed and
/// exclusively owned by decode.
pub(crate) unsafe fn push(shared: &Shared, job: DecodeJob) {
    let mut reserved =
    // SAFETY: The capture path holds this installed slot exclusively before publishing its decode
    // job.
    unsafe { reserve(shared, job.index) };
    *reserved.job() = job;
    reserved.publish();
}

pub struct Reserved<'a> {
    shared: &'a Shared,
    index: usize,
}

impl Reserved<'_> {
    pub fn job(&mut self) -> &mut DecodeJob {
        // SAFETY: this reservation's caller holds the frame.
        unsafe { self.shared.frames.job(self.index) }
    }

    fn publish(self) {
        // SAFETY: The producer owns this slot and its initialized decode job.
        unsafe { self.shared.frames.push_decode(self.index) };
        std::mem::forget(self);
    }
}

impl Drop for Reserved<'_> {
    fn drop(&mut self) {
        let job = self.job();
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
    std::mem::forget(
        // SAFETY: Attach owns payload storage through worker join. Fixture processes own their test
        // pools.
        unsafe { prepare_payload_pools(false) }.unwrap(),
    );
    SEQ.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use tests::retire_and_reacquire_two_carrier_jobs_for_test;

#[cfg(test)]
mod order_group_loan_tests;
