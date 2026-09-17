//! Owned acquisition requests executed within the hour's live-read lease.
//! Job dependencies and the final shared-country verdict belong to the
//! callback, which schedules any ready job on a worker or runs it itself while
//! waiting;
//! no family owns a dedicated thread.

use crate::extended::{
    air, character_portraits, characters, construction, division_eta, division_presentation,
    economy, fog, governance, intel, lexical, naval_history, production, projects,
    province_unit_order, session, ship_models, supply_trains, weather,
};
use crate::{acquisition, map_capture, sampling};
pub(crate) use acquisition::{Completion, Failure, SubmitError, Ticket};
use engine::memory::LiveMemory;
use serde::Serialize;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub(crate) struct RawRequest<J, C> {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: C,
    pub first: J,
    pub second: Option<J>,
}

pub(crate) struct WorldPair<J, C> {
    pub context: C,
    pub first: J,
    pub second: Option<J>,
}
pub(crate) struct WorldRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub economy: WorldPair<economy::EconomyRestJob, economy::EconomyRestCopyContext>,
    pub construction:
        WorldPair<construction::ConstructionRestJob, construction::ConstructionRestCopyContext>,
    pub production: WorldPair<production::ProductionRestJob, production::ProductionRestCopyContext>,
    pub focus: WorldPair<governance::FocusJob, governance::FocusCopyContext>,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WorldReport {
    pub economy: economy::EconomyRestCopyReport,
    pub construction: construction::ConstructionRestCopyReport,
    pub production: production::ProductionRestCopyReport,
    pub focus: governance::FocusCopyReport,
    pub ticks: u64,
}
pub(crate) struct GovernanceRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: governance::CaptureContext,
    pub captured: governance::Captured,
}
pub(crate) struct IntelResidualRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: intel::CaptureContext,
    pub captured: intel::Captured,
}
pub(crate) struct SupplyTrainsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: supply_trains::CaptureContext,
    pub captured: supply_trains::Captured,
}
pub(crate) struct CharacterPortraitsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: character_portraits::CaptureContext,
    pub captured: character_portraits::Captured,
}
pub(crate) struct ProvinceUnitOrderRequest {
    pub context: province_unit_order::CaptureContext,
    pub captured: province_unit_order::Captured,
}
/// Independent core-map walk moved to the pool so its pointer chase overlaps the
/// other readers. Owns its pre-sized row buffers; the callback restores them.
pub(crate) struct ProvincesRequest {
    pub base: usize,
    pub game_state: usize,
    pub needed: bool,
    pub provinces: Vec<engine::cprovince::Row>,
    pub provinces_second: Vec<engine::cprovince::Row>,
}
#[derive(Clone, Copy)]
pub(crate) struct ProvincesReport {
    pub result: engine::walk::Outcome,
    pub second: engine::walk::Outcome,
    pub mismatches: usize,
}
impl Default for ProvincesReport {
    fn default() -> Self {
        let unavailable = engine::walk::Outcome::Unavailable {
            reason: "provinces acquisition did not complete",
        };
        Self {
            result: unavailable,
            second: unavailable,
            mismatches: 0,
        }
    }
}
pub(crate) struct ShipModelsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: ship_models::CaptureContext,
    pub captured: ship_models::Captured,
}
pub(crate) struct PresentationRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: division_presentation::CaptureContext,
    pub captured: division_presentation::Captured,
    pub first: Option<Arc<crate::division_census::UnitInputs>>,
    pub second: Option<Arc<crate::division_census::UnitInputs>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EtaPhase {
    Acquire { pass: usize },
    Verify { pass: usize },
}
pub(crate) struct EtaRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub base: usize,
    pub state: usize,
    pub payload: division_eta::RangeCaptured,
    pub workspace: Box<division_eta::Workspace>,
    pub first: Option<Arc<crate::division_census::UnitInputs>>,
    pub second: Option<Arc<crate::division_census::UnitInputs>>,
    pub phase: EtaPhase,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EtaReport;
pub(crate) struct IntelRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: fog::IntelCopyContext,
    pub first: fog::IntelJob,
    pub second: Option<fog::IntelJob>,
    pub divisions_first: Option<Arc<crate::division_census::UnitInputs>>,
    pub divisions_second: Option<Arc<crate::division_census::UnitInputs>>,
    pub divisions_matched: bool,
}
pub(crate) type CharactersRequest =
    RawRequest<characters::CharactersJob, characters::CharactersCopyContext>;
pub(crate) type RelationsRequest = RawRequest<intel::RelationsJob, intel::RelationsCopyContext>;
pub(crate) type AirWingRequest = RawRequest<air::AirWingJob, air::AirWingCopyContext>;
pub(crate) type AirStatsRequest = RawRequest<air::AirStatsJob, air::AirStatsCopyContext>;
pub(crate) type AirBasesRequest = RawRequest<air::AirBasesJob, air::AirBasesCopyContext>;
pub(crate) type AirHistoryRequest = RawRequest<air::AirHistoryJob, air::AirHistoryCopyContext>;
pub(crate) type NavalRequest = RawRequest<naval_history::NavalJob, naval_history::NavalCopyContext>;
pub(crate) type SupremacyRequest =
    RawRequest<naval_history::SupremacyJob, naval_history::SupremacyCopyContext>;
pub(crate) type SunkRequest = RawRequest<naval_history::SunkJob, naval_history::SunkCopyContext>;
pub(crate) type CapitulationRequest =
    RawRequest<intel::CapitulationJob, intel::CapitulationCopyContext>;
pub(crate) type PlayersRequest =
    RawRequest<map_capture::PlayersJob, map_capture::PlayersCopyContext>;
pub(crate) type ProjectsRequest = RawRequest<projects::ProjectsJob, projects::ProjectsCopyContext>;
pub(crate) type LexicalRequest = RawRequest<lexical::LexicalJob, lexical::LexicalCopyContext>;
pub(crate) type SessionRequest = RawRequest<session::SessionJob, session::SessionCopyContext>;
pub(crate) type WeatherRequest = RawRequest<weather::WeatherJob, weather::WeatherCopyContext>;
pub(crate) type RadarRequest = RawRequest<fog::RadarJob, fog::RadarCopyContext>;
pub(crate) type DivisionsRequest =
    RawRequest<sampling::DivisionsJob, sampling::DivisionsCopyContext>;

// Jobs move preallocated payloads into fixed executor slots. Boxing here would
// allocate on the hourly callback merely to reduce the enum's stack footprint.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Job {
    Characters(CharactersRequest),
    Relations(RelationsRequest),
    AirWing(AirWingRequest),
    AirStats(AirStatsRequest),
    AirBases(AirBasesRequest),
    AirHistory(AirHistoryRequest),
    Naval(NavalRequest),
    Supremacy(SupremacyRequest),
    Sunk(SunkRequest),
    Capitulation(CapitulationRequest),
    Players(PlayersRequest),
    Projects(ProjectsRequest),
    Lexical(LexicalRequest),
    Session(SessionRequest),
    Weather(WeatherRequest),
    Radar(RadarRequest),
    Divisions(DivisionsRequest),
    World(WorldRequest),
    Governance(GovernanceRequest),
    Eta(EtaRequest),
    Intel(IntelRequest),
    Presentation(PresentationRequest),
    IntelResidual(IntelResidualRequest),
    SupplyTrains(SupplyTrainsRequest),
    CharacterPortraits(CharacterPortraitsRequest),
    ProvinceUnitOrder(ProvinceUnitOrderRequest),
    ShipModels(ShipModelsRequest),
    Provinces(ProvincesRequest),
}
#[derive(Clone, Copy)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Report {
    Characters(characters::CharactersCopyReport),
    Relations(intel::RelationsCopyReport),
    AirWing(air::AirWingCopyReport),
    AirStats(air::AirStatsCopyReport),
    AirBases(air::AirBasesCopyReport),
    AirHistory(air::AirHistoryCopyReport),
    Naval(naval_history::NavalCopyReport),
    Supremacy(naval_history::SupremacyCopyReport),
    Sunk(naval_history::SunkCopyReport),
    Capitulation(intel::CapitulationCopyReport),
    Players(map_capture::PlayersCopyReport),
    Projects(projects::ProjectsCopyReport),
    Lexical(lexical::LexicalCopyReport),
    Session(session::SessionCopyReport),
    Weather(weather::WeatherCopyReport),
    Radar(fog::RadarCopyReport),
    Divisions(sampling::DivisionsCopyReport),
    World(WorldReport),
    Governance(governance::CaptureReport),
    Eta(EtaReport),
    Intel(fog::IntelCopyReport),
    Presentation(division_presentation::CaptureReport),
    IntelResidual(intel::CaptureReport),
    SupplyTrains(supply_trains::CaptureReport),
    CharacterPortraits(character_portraits::CaptureReport),
    ProvinceUnitOrder(province_unit_order::CaptureReport),
    ShipModels(ship_models::CaptureReport),
    Provinces(ProvincesReport),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(usize)]
pub(crate) enum Role {
    Characters,
    Relations,
    AirWing,
    AirStats,
    AirBases,
    AirHistory,
    Naval,
    Supremacy,
    Sunk,
    Capitulation,
    Players,
    Projects,
    Lexical,
    Session,
    Weather,
    Radar,
    Divisions,
    World,
    Governance,
    Eta,
    Intel,
    Presentation,
    IntelResidual,
    SupplyTrains,
    CharacterPortraits,
    ProvinceUnitOrder,
    ShipModels,
    Provinces,
}
const ROLE_COUNT: usize = 28;
const ROLES: [Role; ROLE_COUNT] = [
    Role::Characters,
    Role::Relations,
    Role::AirWing,
    Role::AirStats,
    Role::AirBases,
    Role::AirHistory,
    Role::Naval,
    Role::Supremacy,
    Role::Sunk,
    Role::Capitulation,
    Role::Players,
    Role::Projects,
    Role::Lexical,
    Role::Session,
    Role::Weather,
    Role::Radar,
    Role::Divisions,
    Role::World,
    Role::Governance,
    Role::Eta,
    Role::Intel,
    Role::Presentation,
    Role::IntelResidual,
    Role::SupplyTrains,
    Role::CharacterPortraits,
    Role::ProvinceUnitOrder,
    Role::ShipModels,
    Role::Provinces,
];
impl Job {
    pub(crate) fn role(&self) -> Role {
        match self {
            Self::Characters(..) => Role::Characters,
            Self::Relations(..) => Role::Relations,
            Self::AirWing(..) => Role::AirWing,
            Self::AirStats(..) => Role::AirStats,
            Self::AirBases(..) => Role::AirBases,
            Self::AirHistory(..) => Role::AirHistory,
            Self::Naval(..) => Role::Naval,
            Self::Supremacy(..) => Role::Supremacy,
            Self::Sunk(..) => Role::Sunk,
            Self::Capitulation(..) => Role::Capitulation,
            Self::Players(..) => Role::Players,
            Self::Projects(..) => Role::Projects,
            Self::Lexical(..) => Role::Lexical,
            Self::Session(..) => Role::Session,
            Self::Weather(..) => Role::Weather,
            Self::Radar(..) => Role::Radar,
            Self::Divisions(..) => Role::Divisions,
            Self::World(..) => Role::World,
            Self::Governance(..) => Role::Governance,
            Self::Eta(..) => Role::Eta,
            Self::Intel(..) => Role::Intel,
            Self::Presentation(..) => Role::Presentation,
            Self::IntelResidual(..) => Role::IntelResidual,
            Self::SupplyTrains(..) => Role::SupplyTrains,
            Self::CharacterPortraits(..) => Role::CharacterPortraits,
            Self::ProvinceUnitOrder(..) => Role::ProvinceUnitOrder,
            Self::ShipModels(..) => Role::ShipModels,
            Self::Provinces(..) => Role::Provinces,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct WorkerCalibration {
    pub accepted: u64,
    pub completed: u64,
    pub panicked: u64,
    pub stopped: u64,
    pub rejected: u64,
    pub ticks: u64,
    pub inline_completed: u64,
    pub inline_panicked: u64,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct RoleCalibration {
    pub role: Role,
    #[serde(flatten)]
    pub counters: WorkerCalibration,
}
#[derive(Serialize)]
pub(crate) struct Calibration {
    pub executor: acquisition::Calibration,
    pub roles: [RoleCalibration; ROLE_COUNT],
    pub eta_country_cache: EtaCountryCacheCalibration,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum EtaCountryCacheSeed {
    Ready { entries: usize },
    Capacity { required_cells: usize },
    Unavailable { reason: &'static str },
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct EtaCountryCacheCalibration {
    pub ready: u64,
    pub capacity_fallbacks: u64,
    pub unavailable_fallbacks: u64,
    pub latest: Option<EtaCountryCacheSeed>,
    // Shares the ETA role calibration lock; World leaves this empty.
    // Published separately from country authentication to keep scopes distinct.
    #[serde(skip)]
    pub read_witness: EtaReadCalibration,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct EtaReadCalibration {
    pub captures: u64,
    pub stages: division_eta::StageDiagnostics,
    pub hits: u64,
    pub retained_misses: u64,
    pub direct_reads: u64,
    pub witness_skips: u64,
    pub witness_fallbacks: u64,
    pub witness_entries_peak: usize,
    pub witness_bytes_peak: usize,
    pub witness_entries_required: usize,
    pub witness_bytes_required: usize,
}
impl EtaReadCalibration {
    fn observe(&mut self, next: division_eta::CacheDiagnostics) {
        self.captures = self.captures.saturating_add(1);
        self.stages.absorb(next.stages);
        self.hits = self.hits.saturating_add(next.hits);
        self.retained_misses = self.retained_misses.saturating_add(next.retained_misses);
        self.direct_reads = self.direct_reads.saturating_add(next.direct_reads);
        self.witness_skips = self.witness_skips.saturating_add(next.witness_skips);
        self.witness_fallbacks = self
            .witness_fallbacks
            .saturating_add(next.witness_fallbacks);
        self.witness_entries_peak = self.witness_entries_peak.max(next.witness_entries_peak);
        self.witness_bytes_peak = self.witness_bytes_peak.max(next.witness_bytes_peak);
        self.witness_entries_required = self
            .witness_entries_required
            .max(next.witness_entries_required);
        self.witness_bytes_required = self.witness_bytes_required.max(next.witness_bytes_required);
    }
}
impl EtaCountryCacheCalibration {
    fn observe(&mut self, seed: engine::country_memory::SeedStatus, entries: usize) {
        self.latest = Some(match seed {
            engine::country_memory::SeedStatus::Ready => {
                self.ready = self.ready.saturating_add(1);
                EtaCountryCacheSeed::Ready { entries }
            }
            engine::country_memory::SeedStatus::Capacity { required_cells } => {
                self.capacity_fallbacks = self.capacity_fallbacks.saturating_add(1);
                EtaCountryCacheSeed::Capacity { required_cells }
            }
            engine::country_memory::SeedStatus::Unavailable { reason } => {
                self.unavailable_fallbacks = self.unavailable_fallbacks.saturating_add(1);
                EtaCountryCacheSeed::Unavailable { reason }
            }
        });
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WorldCalibration {
    pub country_cache: EtaCountryCacheCalibration,
    pub acquisition_ticks: u64,
    pub last_error: Option<&'static str>,
}

#[derive(Default)]
struct Counters {
    accepted: AtomicU64,
    completed: AtomicU64,
    panicked: AtomicU64,
    stopped: AtomicU64,
    rejected: AtomicU64,
    ticks: AtomicU64,
    inline_completed: AtomicU64,
    inline_panicked: AtomicU64,
}
impl Counters {
    fn snapshot(&self) -> WorkerCalibration {
        WorkerCalibration {
            accepted: self.accepted.load(Ordering::Relaxed),
            completed: self.completed.load(Ordering::Relaxed),
            panicked: self.panicked.load(Ordering::Relaxed),
            stopped: self.stopped.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            ticks: self.ticks.load(Ordering::Relaxed),
            inline_completed: self.inline_completed.load(Ordering::Relaxed),
            inline_panicked: self.inline_panicked.load(Ordering::Relaxed),
        }
    }
}
fn note_dispatch(counters: &Counters, accepted: bool) {
    if accepted {
        &counters.accepted
    } else {
        &counters.rejected
    }
    .fetch_add(1, Ordering::Relaxed);
}
fn note_completion<R>(counters: &Counters, report: &Result<R, Failure>) {
    counters.completed.fetch_add(1, Ordering::Relaxed);
    match report {
        Err(Failure::Panicked) => {
            counters.panicked.fetch_add(1, Ordering::Relaxed);
        }
        Err(Failure::Stopped) => {
            counters.stopped.fetch_add(1, Ordering::Relaxed);
        }
        Ok(_) => {}
    }
}

fn catch_failure<R>(work: impl FnOnce() -> R) -> Result<R, Failure> {
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(report) => Ok(report),
        Err(payload) => {
            // A panic payload can itself panic when dropped. Preserve the
            // callback's drainage barrier by never running that destructor.
            std::mem::forget(payload);
            Err(Failure::Panicked)
        }
    }
}

struct SharedCalibration {
    roles: [Counters; ROLE_COUNT],
    eta: Mutex<EtaCountryCacheCalibration>,
    world: Mutex<WorldCalibration>,
}
impl Default for SharedCalibration {
    fn default() -> Self {
        Self {
            roles: std::array::from_fn(|_| Counters::default()),
            eta: Mutex::new(EtaCountryCacheCalibration::default()),
            world: Mutex::new(WorldCalibration::default()),
        }
    }
}
pub(crate) fn run_world(
    request: &mut WorldRequest,
    memory: &impl engine::Memory,
    production_memory: &impl engine::Memory,
) -> WorldReport {
    let started = crate::sampling::qpc();
    let production = production::copy_production_rest_refresh(
        production_memory,
        request.production.context,
        &mut request.production.first,
        request.production.second.as_mut(),
    );
    let construction = construction::copy_construction_rest_refresh(
        memory,
        request.construction.context,
        &mut request.construction.first,
        request.construction.second.as_mut(),
    );
    let economy = economy::copy_economy_rest_refresh(
        memory,
        request.economy.context,
        &mut request.economy.first,
        request.economy.second.as_mut(),
    );
    let focus = governance::copy_focus_refresh(
        memory,
        request.focus.context,
        &mut request.focus.first,
        request.focus.second.as_mut(),
    );
    WorldReport {
        economy,
        construction,
        production,
        focus,
        ticks: crate::sampling::qpc().saturating_sub(started).max(0) as u64,
    }
}

fn run_world_shared(
    request: &mut WorldRequest,
    memory: &impl engine::Memory,
    calibration: &Mutex<WorldCalibration>,
) -> WorldReport {
    let started = crate::sampling::qpc();
    let snapshot = Arc::clone(&request.countries);
    let cached = snapshot.view(memory);
    let seed = cached.seed_status();
    let entries = cached.cached_entries();
    let mut report = run_world(request, &cached, memory);
    // The callback's Countries anchor owns preparation and terminal checking.
    // No worker may publish or credit checks before that shared verdict.
    report.ticks = crate::sampling::qpc().saturating_sub(started).max(0) as u64;
    let mut counters = calibration
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    counters.country_cache.observe(seed, entries);
    counters.acquisition_ticks = counters.acquisition_ticks.saturating_add(report.ticks);
    report
}

fn run_eta_with_memory(
    request: &mut EtaRequest,
    memory: &impl engine::Memory,
    calibration: &Mutex<EtaCountryCacheCalibration>,
) -> EtaReport {
    let cached = request.countries.view(memory);
    match request.phase {
        EtaPhase::Acquire { pass } => {
            if pass == 0 {
                calibration
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .observe(cached.seed_status(), cached.cached_entries());
            }
            let inputs = if pass == 0 {
                request.first.as_ref()
            } else {
                request.second.as_ref()
            };
            let seeds = inputs.map_or(&[][..], |inputs| inputs.seeds());
            let outcome = inputs.map_or(
                engine::walk::Outcome::Unavailable {
                    reason: "division inputs unavailable",
                },
                |inputs| inputs.outcome(),
            );
            request.payload.acquire_pass(
                &mut request.workspace,
                &cached,
                request.base,
                request.state,
                seeds,
                outcome,
                pass,
            )
        }
        EtaPhase::Verify { pass } => request.payload.verify_pass(
            &mut request.workspace,
            &cached,
            request.base,
            request.state,
            pass,
        ),
    };
    // The callback records the final range diagnostics once, after its last
    // verification. Neither inputs nor the workspace may be released here.
    EtaReport
}

fn run_with_memory(
    job: &mut Job,
    memory: &impl engine::Memory,
    calibration: &SharedCalibration,
) -> Report {
    let started = sampling::qpc();
    let role = job.role();
    let report = match job {
        Job::Characters(request) => {
            let cached = request.countries.view(memory);
            Report::Characters(characters::copy_characters_refresh(
                &cached,
                request.context,
                &mut request.first,
                request.second.as_mut(),
            ))
        }
        Job::Relations(request) => {
            let cached = request.countries.view(memory);
            Report::Relations(intel::copy_relations_refresh(
                &cached,
                request.context,
                &mut request.first,
                request.second.as_mut(),
            ))
        }
        Job::AirWing(request) => Report::AirWing(air::copy_air_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::AirStats(request) => Report::AirStats(air::copy_air_stats_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::AirBases(request) => Report::AirBases(air::copy_air_bases_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::AirHistory(request) => Report::AirHistory(air::copy_air_history_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::Naval(request) => Report::Naval(naval_history::copy_naval_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::Supremacy(request) => Report::Supremacy(naval_history::copy_supremacy_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::Sunk(request) => Report::Sunk(naval_history::copy_sunk_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::Capitulation(request) => Report::Capitulation(intel::copy_capitulation_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::Players(request) => Report::Players(map_capture::copy_players_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::Projects(request) => Report::Projects(projects::copy_projects_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::Lexical(request) => Report::Lexical(lexical::copy_lexical_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::Session(request) => Report::Session(session::copy_session_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::Weather(request) => Report::Weather(weather::copy_weather_refresh(
            memory,
            request.context,
            &mut request.first,
            request.second.as_mut(),
        )),
        Job::Radar(request) => {
            let cached = request.countries.view(memory);
            Report::Radar(fog::copy_radar_refresh(
                &cached,
                request.context,
                &mut request.first,
                request.second.as_mut(),
            ))
        }
        Job::Divisions(request) => {
            let cached = request.countries.view(memory);
            Report::Divisions(sampling::copy_divisions_core_refresh(
                &cached,
                request.context,
                &mut request.first,
                request.second.as_mut(),
            ))
        }

        Job::World(request) => Report::World(run_world_shared(request, memory, &calibration.world)),
        Job::Governance(request) => {
            let cached = request.countries.view(memory);
            Report::Governance(request.captured.capture_owned(&cached, request.context))
        }
        Job::IntelResidual(request) => {
            let cached = request.countries.view(memory);
            Report::IntelResidual(request.captured.capture_residual(&cached, request.context))
        }
        Job::SupplyTrains(request) => {
            let cached = request.countries.view(memory);
            Report::SupplyTrains(request.captured.capture_owned(&cached, request.context))
        }
        Job::CharacterPortraits(request) => {
            let cached = request.countries.view(memory);
            Report::CharacterPortraits(request.captured.capture_owned(&cached, request.context))
        }
        Job::ProvinceUnitOrder(request) => {
            Report::ProvinceUnitOrder(request.captured.capture_owned(memory, request.context))
        }
        Job::ShipModels(request) => {
            let cached = request.countries.view(memory);
            Report::ShipModels(request.captured.capture_owned(&cached, request.context))
        }
        Job::Provinces(request) => {
            let result = engine::cprovince::walk(
                memory,
                request.base,
                request.game_state,
                &mut request.provinces,
            );
            let (second, mismatches) = if request.needed {
                let second = engine::cprovince::walk(
                    memory,
                    request.base,
                    request.game_state,
                    &mut request.provinces_second,
                );
                let mismatches = crate::sampling::mismatches(
                    result,
                    &request.provinces,
                    second,
                    &request.provinces_second,
                );
                (second, mismatches)
            } else {
                (
                    engine::walk::Outcome::Unavailable {
                        reason: "provinces consistency window closed",
                    },
                    0,
                )
            };
            Report::Provinces(ProvincesReport {
                result,
                second,
                mismatches,
            })
        }
        Job::Presentation(request) => {
            let cached = request.countries.view(memory);
            let first = request.first.as_deref();
            let second = request.second.as_deref();
            let missing = engine::walk::Outcome::Unavailable {
                reason: "acquisition source not supplied",
            };
            Report::Presentation(request.captured.capture_owned(
                &cached,
                request.context,
                production::Sources {
                    first: first.map_or(&[], |inputs| inputs.seeds()),
                    first_outcome: first.map_or(missing, |inputs| inputs.outcome()),
                    second: second.map_or(&[], |inputs| inputs.seeds()),
                    second_outcome: second.map_or(missing, |inputs| inputs.outcome()),
                },
            ))
        }
        Job::Eta(request) => Report::Eta(run_eta_with_memory(request, memory, &calibration.eta)),
        Job::Intel(request) => {
            let cached = request.countries.view(memory);
            let divisions =
                request
                    .divisions_first
                    .as_ref()
                    .map(|first| fog::IntelDivisionSources {
                        first: first.intel_sources(),
                        second: request
                            .divisions_second
                            .as_ref()
                            .map(|second| second.intel_sources()),
                        matched: request.divisions_matched,
                    });
            Report::Intel(fog::copy_intel_refresh_with_divisions(
                &cached,
                request.context,
                &mut request.first,
                request.second.as_mut(),
                divisions,
            ))
        }
    };
    calibration.roles[role as usize].ticks.fetch_add(
        sampling::qpc().saturating_sub(started).max(0) as u64,
        Ordering::Relaxed,
    );
    report
}

pub(crate) struct Pool {
    executor: acquisition::Pool<Job, Report>,
    calibration: Arc<SharedCalibration>,
}
impl Pool {
    pub(crate) fn start(worker_count: usize, capacity: usize) -> Result<Self, String> {
        let calibration = Arc::new(SharedCalibration::default());
        let runner_calibration = Arc::clone(&calibration);
        let executor = acquisition::Pool::start(worker_count, capacity, move |job| {
            // SAFETY: submit's callback lease remains admitted until every
            // accepted request is collected, including on callback unwind.
            let memory = unsafe { LiveMemory::new() };
            run_with_memory(job, &memory, &runner_calibration)
        })?;
        Ok(Self {
            executor,
            calibration,
        })
    }
    /// # Safety
    /// Every accepted request must finish before the foreign-world callback
    /// lease ends. Use Batch to retain that barrier on an unwinding callback.
    pub(crate) unsafe fn submit(
        &self,
        request: &mut Option<Job>,
    ) -> Result<Ticket, acquisition::SubmitError> {
        let role = request.as_ref().map(Job::role);
        // SAFETY: the caller retains the admitted callback lease.
        let result = unsafe { self.executor.submit(request) };
        if let Some(role) = role {
            note_dispatch(&self.calibration.roles[role as usize], result.is_ok());
        }
        result
    }
    pub(crate) fn finish(&self, ticket: Ticket) -> Option<Completion<Job, Report>> {
        let completed = self.executor.finish(ticket)?;
        let role = completed.request.role();
        note_completion(&self.calibration.roles[role as usize], &completed.report);
        if role == Role::World
            && let Err(failure) = completed.report
        {
            self.calibration
                .world
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .last_error = Some(match failure {
                Failure::Panicked => "World acquisition job panicked",
                Failure::Stopped => "World acquisition job stopped before reading",
            });
        }
        Some(completed)
    }
    /// Execute a retained, rejected request on the callback with the same
    /// providers and panic boundary. An accepted request must never retry here.
    ///
    /// # Safety
    /// The admitted callback lease must remain valid throughout this call.
    pub(crate) unsafe fn run_inline(&self, mut request: Job) -> Completion<Job, Report> {
        let role = request.role();
        let report = catch_failure(|| {
            // SAFETY: the caller retains the callback's live-read lease.
            let memory = unsafe { LiveMemory::new() };
            run_with_memory(&mut request, &memory, &self.calibration)
        });
        let counters = &self.calibration.roles[role as usize];
        counters.inline_completed.fetch_add(1, Ordering::Relaxed);
        if report.is_err() {
            counters.inline_panicked.fetch_add(1, Ordering::Relaxed);
        }
        Completion { request, report }
    }
    pub(crate) fn observe_eta_diagnostics(&self, diagnostics: division_eta::CacheDiagnostics) {
        self.calibration
            .eta
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .read_witness
            .observe(diagnostics);
    }
    pub(crate) fn thread_ids(&self) -> Vec<u32> {
        self.executor.thread_ids()
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.executor
            .allocated_bytes()
            .saturating_add(std::mem::size_of::<SharedCalibration>())
            .saturating_add(2 * std::mem::size_of::<usize>())
    }
    pub(crate) fn world_calibration(&self) -> WorldCalibration {
        *self
            .calibration
            .world
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    pub(crate) fn calibration(&self) -> Calibration {
        Calibration {
            executor: self.executor.calibration(),
            roles: std::array::from_fn(|index| RoleCalibration {
                role: ROLES[index],
                counters: self.calibration.roles[index].snapshot(),
            }),
            eta_country_cache: *self
                .calibration
                .eta
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        }
    }
    pub(crate) fn shutdown(&self) -> Result<(), String> {
        self.executor.shutdown()
    }
}

/// Upper bound on simultaneously retained callback requests, not table capacity.
pub(crate) const MAX_JOBS: usize = 64;
pub(crate) struct Batch<'a> {
    pool: &'a Pool,
    tickets: [Option<Ticket>; MAX_JOBS],
}
impl<'a> Batch<'a> {
    pub(crate) fn new(pool: &'a Pool) -> Self {
        Self {
            pool,
            tickets: [None; MAX_JOBS],
        }
    }
    /// # Safety
    /// The callback must own the live-read lease until this guard is
    /// dropped. No lock held across finish/Drop may be needed by a queued job.
    pub(crate) unsafe fn submit(
        &mut self,
        request: &mut Option<Job>,
    ) -> Result<Ticket, SubmitError> {
        let Some(slot) = self.tickets.iter_mut().find(|ticket| ticket.is_none()) else {
            if let Some(request) = request.as_ref() {
                note_dispatch(&self.pool.calibration.roles[request.role() as usize], false);
            }
            return Err(SubmitError::Full);
        };
        // SAFETY: this guard drains every ticket before releasing its lease.
        let ticket = unsafe { self.pool.submit(request) }?;
        *slot = Some(ticket);
        Ok(ticket)
    }
    pub(crate) fn finish(&mut self, ticket: Ticket) -> Option<Completion<Job, Report>> {
        let index = self
            .tickets
            .iter()
            .position(|candidate| *candidate == Some(ticket))?;
        let completed = self.pool.finish(ticket);
        self.tickets[index] = None;
        completed
    }
    /// # Safety
    /// Same live-read lease as submit; only a retained rejected job may run here.
    pub(crate) unsafe fn run_inline(&self, request: Job) -> Completion<Job, Report> {
        // SAFETY: forwarded from this callback's lease.
        unsafe { self.pool.run_inline(request) }
    }
}
impl Drop for Batch<'_> {
    fn drop(&mut self) {
        for ticket in self.tickets.iter_mut().filter_map(Option::take) {
            // User job panics are caught by the executor. Recovering/dropping
            // owned results must not prevent draining any remaining ticket.
            let _ = catch_failure(|| drop(self.pool.finish(ticket)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use characters::CharactersCopyContext;

    fn country_snapshot(
        memory: &impl engine::Memory,
        cells: usize,
    ) -> Arc<engine::country_memory::Cache> {
        let mut storage = Arc::new(engine::country_memory::Cache::with_capacity(cells));
        engine::country_memory::Cache::prepare_shared(&mut storage, memory, 0, 0x1000).unwrap()
    }

    fn empty_country_snapshot() -> Arc<engine::country_memory::Cache> {
        country_snapshot(
            &engine::memory::SliceMemory {
                base: 0,
                bytes: &empty_eta_memory(),
            },
            64,
        )
    }

    fn empty_eta_memory() -> Vec<u8> {
        let mut bytes = vec![0; 0x3000];
        for (offset, pointer) in [
            (engine::cgame_state::COUNTRIES, 0x2000_u64),
            (engine::cgame_state::COUNTRY_LINKS, 0x2100),
            (engine::cgame_state::PROVINCES, 0x2200),
        ] {
            let at = 0x1000 + offset;
            bytes[at..at + 8].copy_from_slice(&pointer.to_le_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&2_u32.to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&2_u32.to_le_bytes());
        }
        bytes
    }

    fn world_request(checked: bool) -> WorldRequest {
        let mut request = WorldRequest {
            countries: empty_country_snapshot(),
            economy: WorldPair {
                context: economy::EconomyRestCopyContext {
                    base: 0,
                    game_state: 0x1000,
                    needed: [checked; 10],
                    resource_rows: [0; 3],
                    resource_second: [0; 3],
                    resource_scratch: 0,
                },
                first: Default::default(),
                second: checked.then(Default::default),
            },
            construction: WorldPair {
                context: construction::ConstructionRestCopyContext {
                    base: 0,
                    game_state: 0x1000,
                    needed: [checked; 10],
                },
                first: Default::default(),
                second: checked.then(Default::default),
            },
            production: WorldPair {
                context: production::ProductionRestCopyContext {
                    base: 0,
                    game_state: 0x1000,
                    needed: [checked; 8],
                },
                first: Default::default(),
                second: checked.then(Default::default),
            },
            focus: WorldPair {
                context: governance::Captured::with_capacities(
                    &Default::default(),
                    &[0; governance::N],
                )
                .focus_copy_context(0, 0x1000, [checked; 2]),
                first: Default::default(),
                second: checked.then(Default::default),
            },
        };
        macro_rules! provision {
            ($pair:expr) => {{
                $pair.first.log = crate::raw::RawLog::new(512, 65536);
                if let Some(second) = &mut $pair.second {
                    second.log = crate::raw::RawLog::new(512, 65536);
                }
            }};
        }
        provision!(request.economy);
        provision!(request.construction);
        provision!(request.production);
        request.focus.first = governance::FocusJob::new();
        request.focus.second = checked.then(governance::FocusJob::new);
        request
    }

    #[test]
    fn world_owned_worker_matches_direct_copy_partitions_and_returns_both_passes() {
        for checked in [false, true] {
            let bytes = empty_eta_memory();
            let mut direct = world_request(checked);
            let direct_memory = engine::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let direct_report = run_world(&mut direct, &direct_memory, &direct_memory);
            let calibration = Arc::new(Mutex::new(WorldCalibration::default()));
            let worker_calibration = Arc::clone(&calibration);
            let worker = acquisition::Pool::start(1, 1, move |request| {
                run_world_shared(
                    request,
                    &engine::memory::SliceMemory {
                        base: 0,
                        bytes: &bytes,
                    },
                    &worker_calibration,
                )
            })
            .unwrap();
            let mut request = Some(world_request(checked));
            let capacity = request.as_ref().unwrap().economy.first.log.spans.capacity();
            let ticket = unsafe { worker.submit(&mut request) }.unwrap();
            assert!(request.is_none());
            let completed = worker.finish(ticket).unwrap();
            let report = completed.report.unwrap();
            assert_eq!(calibration.lock().unwrap().country_cache.ready, 1);
            assert_eq!(
                completed.request.economy.first.log.spans.capacity(),
                capacity
            );
            macro_rules! compare {
                ($family:ident, $n:expr) => {{
                    assert_eq!(report.$family.raw_ok, direct_report.$family.raw_ok);
                    assert_eq!(report.$family.matched, direct_report.$family.matched);
                    assert_eq!(report.$family.raw_bytes, direct_report.$family.raw_bytes);
                    assert_eq!(completed.request.$family.second.is_some(), checked);
                    for slot in 0..$n {
                        assert!(
                            completed
                                .request
                                .$family
                                .first
                                .log
                                .partition_equal(&direct.$family.first.log, slot)
                        );
                    }
                }};
            }
            compare!(economy, 10);
            compare!(construction, 10);
            compare!(production, 8);
            assert_eq!(report.focus.raw_ok, direct_report.focus.raw_ok);
            assert_eq!(report.focus.matched, direct_report.focus.matched);
            assert_eq!(report.focus.mismatches, direct_report.focus.mismatches);
            assert_eq!(report.focus.raw_bytes, direct_report.focus.raw_bytes);
            assert_eq!(completed.request.focus.second.is_some(), checked);
            let decoded_focus = |job: &governance::FocusJob| {
                assert!(job.is_provisioned());
                let mut captured =
                    governance::Captured::with_capacities(&Default::default(), &[0; governance::N]);
                captured.decode_focus_payload(0, job, &mut [0; 2]);
                captured.tables()[..2].to_vec()
            };
            assert_eq!(
                decoded_focus(&completed.request.focus.first),
                decoded_focus(&direct.focus.first),
            );
            if let Some(second) = &completed.request.focus.second {
                assert_eq!(
                    decoded_focus(second),
                    decoded_focus(direct.focus.second.as_ref().unwrap())
                );
            }
            worker.shutdown().unwrap();
            let mut rejected = Some(completed.request);
            assert!(unsafe { worker.submit(&mut rejected) }.is_err());
            let bytes = empty_eta_memory();
            let fallback_memory = engine::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let fallback = run_world(
                rejected.as_mut().unwrap(),
                &fallback_memory,
                &fallback_memory,
            );
            assert_eq!(fallback.economy.raw_bytes, direct_report.economy.raw_bytes);
            assert_eq!(fallback.economy.matched, direct_report.economy.matched);
        }
    }

    #[test]
    fn world_shared_country_fault_is_detected_at_outer_barrier_and_capacity_is_observable() {
        struct Changed {
            bytes: Vec<u8>,
            reads: std::cell::Cell<usize>,
        }
        impl engine::Memory for Changed {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if !engine::Memory::copy(
                    &engine::memory::SliceMemory {
                        base: 0,
                        bytes: &self.bytes,
                    },
                    address,
                    output,
                ) {
                    return false;
                }
                if address == 0x1000 + engine::cgame_state::COUNTRIES {
                    self.reads.set(self.reads.get() + 1);
                    if self.reads.get() > 1 {
                        output[0] ^= 1;
                    }
                }
                true
            }
        }
        let changed = Changed {
            bytes: empty_eta_memory(),
            reads: std::cell::Cell::new(0),
        };
        let calibration = Mutex::new(WorldCalibration::default());
        let mut request = world_request(true);
        request.countries = country_snapshot(&changed, 64);
        assert_eq!(changed.reads.get(), 1, "one shared authentication pass");
        run_world_shared(&mut request, &changed, &calibration);
        assert_eq!(
            changed.reads.get(),
            3,
            "ProductionRest reads two fresh passes; other World families reuse the shared authentication"
        );
        assert!(
            request.countries.view(&changed).verify().is_err(),
            "the outer barrier must reject shared source mutation before publishing rows"
        );
        assert_eq!(calibration.lock().unwrap().last_error, None);
        let bytes = empty_eta_memory();
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut request = world_request(false);
        request.countries = country_snapshot(&memory, 0);
        run_world_shared(&mut request, &memory, &calibration);
        assert_eq!(
            calibration.lock().unwrap().country_cache.capacity_fallbacks,
            1
        );
        let mut overflow = world_request(false);
        overflow.economy.first.log = Default::default();
        let report = run_world(&mut overflow, &memory, &memory);
        assert!(!report.economy.raw_ok);
        assert!(report.focus.raw_ok);
    }

    #[test]
    fn eta_country_cache_calibration_distinguishes_each_seed_and_retains_fallback_counts() {
        use engine::country_memory::SeedStatus;
        let mut calibration = EtaCountryCacheCalibration::default();
        assert!(calibration.latest.is_none());
        calibration.observe(SeedStatus::Ready, 10);
        assert_eq!(
            serde_json::to_value(calibration.latest).unwrap(),
            serde_json::json!({"state": "ready", "entries": 10})
        );
        calibration.observe(
            SeedStatus::Capacity {
                required_cells: 16384,
            },
            0,
        );
        assert_eq!(
            serde_json::to_value(calibration.latest).unwrap(),
            serde_json::json!({"state": "capacity", "required_cells": 16384})
        );
        calibration.observe(
            SeedStatus::Unavailable {
                reason: "synthetic unreadable census",
            },
            0,
        );
        assert_eq!(
            serde_json::to_value(calibration.latest).unwrap(),
            serde_json::json!({"state": "unavailable", "reason": "synthetic unreadable census"})
        );
        calibration.observe(SeedStatus::Ready, 12);
        assert_eq!(calibration.ready, 2);
        assert_eq!(calibration.capacity_fallbacks, 1);
        assert_eq!(calibration.unavailable_fallbacks, 1);
    }

    #[test]
    fn eta_read_calibration_sums_work_and_retains_peak_demand_separately() {
        let mut calibration = EtaCountryCacheCalibration::default();
        calibration
            .read_witness
            .observe(division_eta::CacheDiagnostics {
                hits: 5,
                retained_misses: 2,
                direct_reads: 3,
                witness_skips: 4,
                witness_fallbacks: 1,
                witness_entries_peak: 8,
                witness_bytes_peak: 100,
                witness_entries_required: 10,
                witness_bytes_required: 140,
                stages: division_eta::StageDiagnostics {
                    walk: division_eta::StageCost {
                        calls: 1,
                        ticks: 11,
                    },
                    supply_builds: 2,
                    ..Default::default()
                },
            });
        calibration
            .read_witness
            .observe(division_eta::CacheDiagnostics {
                hits: 7,
                retained_misses: 1,
                direct_reads: 2,
                witness_skips: 3,
                witness_fallbacks: 2,
                witness_entries_peak: 6,
                witness_bytes_peak: 120,
                witness_entries_required: 12,
                witness_bytes_required: 130,
                stages: division_eta::StageDiagnostics {
                    walk: division_eta::StageCost {
                        calls: 2,
                        ticks: 17,
                    },
                    supply_builds: 3,
                    ..Default::default()
                },
            });
        assert_eq!(
            serde_json::to_value(calibration.read_witness).unwrap(),
            serde_json::json!({
                "captures": 2, "hits": 12, "retained_misses": 3,
                "direct_reads": 5, "witness_skips": 7, "witness_fallbacks": 3,
                "witness_entries_peak": 8, "witness_bytes_peak": 120,
                "witness_entries_required": 12, "witness_bytes_required": 140,
                "stages": division_eta::StageDiagnostics {
                    walk: division_eta::StageCost { calls: 3, ticks: 28 },
                    supply_builds: 5,
                    ..Default::default()
                },
            })
        );
        // Worker read diagnostics are not mislabeled as country authentication.
        assert!(
            serde_json::to_value(calibration)
                .unwrap()
                .get("read_witness")
                .is_none()
        );
    }

    fn empty_eta_request(checked: bool) -> (EtaRequest, division_eta::Captured) {
        let first = Arc::new(crate::division_census::UnitInputs::empty_rows());
        let mut captured = division_eta::Captured::empty();
        captured.prepare_ranges(
            &production::Sources {
                first: first.seeds(),
                first_outcome: first.outcome(),
                second: first.seeds(),
                second_outcome: first.outcome(),
            },
            checked,
            1,
        );
        let workspace = Box::new(division_eta::Workspace::with_capacities(
            &captured.workspace_capacities(),
            checked,
        ));
        let request = EtaRequest {
            countries: empty_country_snapshot(),
            base: 0,
            state: 0x1000,
            payload: captured.take_range(0).unwrap(),
            workspace,
            first: Some(Arc::clone(&first)),
            second: checked.then_some(first),
            phase: EtaPhase::Acquire { pass: 0 },
        };
        (request, captured)
    }

    #[test]
    fn eta_owned_worker_returns_both_passes_for_outer_verification_and_can_be_reused() {
        let bytes = empty_eta_memory();
        let calibration = Arc::new(Mutex::new(EtaCountryCacheCalibration::default()));
        let worker_calibration = Arc::clone(&calibration);
        let pool = acquisition::Pool::start(1, 1, move |request| {
            run_eta_with_memory(
                request,
                &engine::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                &worker_calibration,
            )
        })
        .unwrap();
        for checked in [false, true] {
            let (mut request, mut captured) = empty_eta_request(checked);
            let workspace_address = (&*request.workspace) as *const division_eta::Workspace;
            for pass in 0..=usize::from(checked) {
                for phase in [EtaPhase::Acquire { pass }, EtaPhase::Verify { pass }] {
                    request.phase = phase;
                    let mut pending = Some(request);
                    let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
                    assert!(pending.is_none());
                    let completion = pool.finish(ticket).unwrap();
                    assert!(completion.report.is_ok());
                    request = completion.request;
                    assert_eq!(
                        (&*request.workspace) as *const division_eta::Workspace,
                        workspace_address
                    );
                    assert!(request.first.is_some(), "source leases survive every phase");
                    assert_eq!(request.second.is_some(), checked);
                }
            }
            captured.restore_range(0, request.payload);
            assert_eq!(captured.finish_ranges().matched, checked);
            assert_eq!(
                captured.outcomes(),
                [engine::walk::Outcome::Rows { len: 0 }]
            );
        }
        pool.shutdown().unwrap();
        assert_eq!(calibration.lock().unwrap().ready, 2);
        let (request, mut captured) = empty_eta_request(true);
        let mut pending = Some(request);
        assert!(unsafe { pool.submit(&mut pending) }.is_err());
        let mut request = pending.unwrap();
        let bytes = empty_eta_memory();
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        for pass in 0..2 {
            request.phase = EtaPhase::Acquire { pass };
            run_eta_with_memory(&mut request, &memory, &calibration);
            request.phase = EtaPhase::Verify { pass };
            run_eta_with_memory(&mut request, &memory, &calibration);
        }
        captured.restore_range(0, request.payload);
        assert!(captured.finish_ranges().matched);
        assert_eq!(calibration.lock().unwrap().ready, 3);
    }

    #[test]
    fn eta_shared_country_mutation_is_rejected_by_outer_verification() {
        use std::cell::Cell;
        struct Changed {
            bytes: Vec<u8>,
            root_reads: Cell<usize>,
        }
        impl engine::Memory for Changed {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if !engine::Memory::copy(
                    &engine::memory::SliceMemory {
                        base: 0,
                        bytes: &self.bytes,
                    },
                    address,
                    destination,
                ) {
                    return false;
                }
                if address == 0x1000 + engine::cgame_state::COUNTRIES {
                    self.root_reads.set(self.root_reads.get() + 1);
                    if self.root_reads.get() > 1 {
                        destination[0] ^= 1;
                    }
                }
                true
            }
        }
        let memory = Changed {
            bytes: empty_eta_memory(),
            root_reads: Cell::new(0),
        };
        let (mut request, mut captured) = empty_eta_request(true);
        request.countries = country_snapshot(&memory, 64);
        let calibration = Mutex::new(EtaCountryCacheCalibration::default());
        for pass in 0..2 {
            request.phase = EtaPhase::Acquire { pass };
            run_eta_with_memory(&mut request, &memory, &calibration);
            request.phase = EtaPhase::Verify { pass };
            run_eta_with_memory(&mut request, &memory, &calibration);
        }
        captured.restore_range(0, request.payload);
        assert!(
            captured.finish_ranges().matched,
            "table checks await the shared outer verdict"
        );
        assert_eq!(memory.root_reads.get(), 1, "ETA must not reseed countries");
        assert!(request.countries.view(&memory).verify().is_err());
        assert!(request.first.is_some());
        assert!(request.second.is_some());
    }

    #[test]
    fn eta_panic_and_pre_run_cancellation_recover_owned_buffers_without_a_success_report() {
        let pool = acquisition::Pool::start(0, 1, |request: &mut EtaRequest| -> EtaReport {
            request.payload.fail("partial acquisition result", 0, true);
            panic!("synthetic ETA panic");
        })
        .unwrap();
        let (request, mut captured) = empty_eta_request(true);
        let workspace = (&*request.workspace) as *const division_eta::Workspace;
        let mut pending = Some(request);
        let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
        let completion = pool.finish(ticket).unwrap();
        assert!(matches!(completion.report, Err(Failure::Panicked)));
        assert_eq!(
            (&*completion.request.workspace) as *const division_eta::Workspace,
            workspace
        );
        assert!(completion.request.first.is_some());
        assert!(completion.request.second.is_some());
        let mut pending = Some(completion.request);
        assert!(matches!(
            unsafe { pool.submit(&mut pending) },
            Err(SubmitError::Stopped)
        ));
        pool.shutdown().unwrap();
        let canceled = acquisition::Pool::start(0, 1, |_: &mut EtaRequest| -> EtaReport {
            panic!("canceled request must not execute");
        })
        .unwrap();
        let ticket = unsafe { canceled.submit(&mut pending) }.unwrap();
        canceled.shutdown().unwrap();
        let completion = canceled.finish(ticket).unwrap();
        assert!(matches!(completion.report, Err(Failure::Stopped)));
        assert!(completion.request.second.is_some());
        captured.restore_range(0, completion.request.payload);
        assert!(!captured.finish_ranges().matched);
        assert!(matches!(
            captured.outcomes(),
            [engine::walk::Outcome::Unavailable { .. }]
        ));
    }

    #[test]
    fn country_snapshot_leases_return_after_success_panic_and_rejected_dispatch() {
        for panics in [false, true] {
            let bytes = empty_eta_memory();
            let memory = engine::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut owner = Arc::new(engine::country_memory::Cache::with_capacity(64));
            let snapshot =
                engine::country_memory::Cache::prepare_shared(&mut owner, &memory, 0, 0x1000)
                    .unwrap();
            let mut request = world_request(false);
            request.countries = snapshot;
            let pool = acquisition::Pool::start(1, 1, move |_: &mut WorldRequest| {
                assert!(!panics, "synthetic acquisition failure");
            })
            .unwrap();
            let mut pending = Some(request);
            let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
            let completed = pool.finish(ticket).unwrap();
            assert_eq!(completed.report.is_err(), panics);
            assert!(
                engine::country_memory::Cache::prepare_shared(&mut owner, &memory, 0, 0x1000)
                    .is_err()
            );
            pool.shutdown().unwrap();
            let mut rejected = Some(completed.request);
            assert!(unsafe { pool.submit(&mut rejected) }.is_err());
            assert!(Arc::get_mut(&mut owner).is_none());
            assert!(
                rejected
                    .as_ref()
                    .unwrap()
                    .countries
                    .view(&memory)
                    .verify()
                    .is_ok()
            );
            drop(rejected);
            assert!(
                engine::country_memory::Cache::prepare_shared(&mut owner, &memory, 0, 0x1000)
                    .is_ok()
            );
        }
    }

    fn synthetic_pool(run: impl Fn(&mut Job) -> Report + Send + Sync + 'static) -> Pool {
        Pool {
            executor: acquisition::Pool::start(0, MAX_JOBS, run).unwrap(),
            calibration: Arc::new(SharedCalibration::default()),
        }
    }

    fn hourly_family_jobs(
        countries: &Arc<engine::country_memory::Cache>,
        needed: [bool; 4],
    ) -> [Job; 3] {
        let successful = needed.map(|needed| {
            if needed {
                0
            } else {
                sampling::CONSISTENCY_TARGET
            }
        });
        [
            Job::SupplyTrains(SupplyTrainsRequest {
                countries: Arc::clone(countries),
                context: supply_trains::CaptureContext {
                    base: 0,
                    game_state: 0x1000,
                    needed: [needed[0], needed[1]],
                },
                captured: supply_trains::Captured::with_capacities(
                    &Default::default(),
                    &successful[..2],
                ),
            }),
            Job::CharacterPortraits(CharacterPortraitsRequest {
                countries: Arc::clone(countries),
                context: character_portraits::CaptureContext {
                    base: 0,
                    game_state: 0x1000,
                    needed: [needed[2]],
                },
                captured: character_portraits::Captured::with_capacities(
                    &Default::default(),
                    &successful[2..3],
                ),
            }),
            Job::ProvinceUnitOrder(ProvinceUnitOrderRequest {
                context: province_unit_order::CaptureContext {
                    base: 0,
                    game_state: 0x1000,
                    needed: [needed[3]],
                },
                captured: province_unit_order::Captured::with_capacities(
                    &Default::default(),
                    &successful[3..],
                ),
            }),
        ]
    }

    // Empty supply countries and one exact, non-UTF-8 portrait descriptor.
    fn hourly_family_memory() -> Vec<u8> {
        use engine::{
            ccharacter, ccharacter_manager, cgame_state, character_portrait_entry, cprovince,
            csupply_system, msvc_string,
        };
        fn put<const N: usize>(bytes: &mut [u8], at: usize, value: [u8; N]) {
            bytes[at..at + N].copy_from_slice(&value);
        }
        fn array(bytes: &mut [u8], at: usize, data: u64, count: u32) {
            put(bytes, at, data.to_le_bytes());
            put(bytes, at + 8, count.to_le_bytes());
            put(bytes, at + 12, count.to_le_bytes());
        }
        let mut bytes = empty_eta_memory();
        bytes.resize(0x10000, 0);
        put(&mut bytes, 0x2208, 0x6000_u64.to_le_bytes());
        put(
            &mut bytes,
            0x6000,
            (cprovince::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x6000 + cprovince::ID, 1_u32.to_le_bytes());
        array(&mut bytes, 0x6000 + cprovince::OCCUPANTS, 0, 0);
        array(&mut bytes, 0x6000 + cprovince::NAVAL_OCCUPANTS, 0, 0);
        put(
            &mut bytes,
            0x1000 + cgame_state::SUPPLY_SYSTEM,
            0x8000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x8000,
            (csupply_system::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x8008,
            (csupply_system::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            0x8000 + csupply_system::COUNTRY_SYSTEM_DATA,
            0x9000,
            2,
        );
        array(
            &mut bytes,
            0x8000 + csupply_system::COUNTRY_COMPANION_DATA,
            0xa000,
            2,
        );
        put(
            &mut bytes,
            0x1000 + cgame_state::CHARACTER_MANAGER,
            0xc000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xc000,
            (ccharacter_manager::VTABLE_RVA as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            0xc000 + ccharacter_manager::HISTORICAL_CHARACTERS,
            0xd000,
            1,
        );
        put(&mut bytes, 0xd000, 0xe000_u64.to_le_bytes());
        put(
            &mut bytes,
            0xe000,
            (ccharacter::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, 0xe008, ccharacter::REFERENCE_TYPE.to_le_bytes());
        put(&mut bytes, 0xe000 + ccharacter::ID, 73_u32.to_le_bytes());
        array(&mut bytes, 0xe000 + ccharacter::PORTRAIT_DATA, 0xf000, 1);
        put(
            &mut bytes,
            0xf000 + character_portrait_entry::ROLE,
            9_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xf000 + character_portrait_entry::SIZE,
            2_u32.to_le_bytes(),
        );
        bytes[0xf000 + character_portrait_entry::IS_SET] = 255;
        put(
            &mut bytes,
            0xf000 + character_portrait_entry::KIND,
            1_u32.to_le_bytes(),
        );
        let text = 0xf000 + character_portrait_entry::IDENTITY;
        put(&mut bytes, text, [b'g', 0, 0xff]);
        put(&mut bytes, text + msvc_string::LENGTH, 3_u64.to_le_bytes());
        put(
            &mut bytes,
            text + msvc_string::CAPACITY,
            15_u64.to_le_bytes(),
        );
        bytes
    }

    #[test]
    fn hourly_owned_jobs_match_direct_rows_and_keep_independent_cold_checks() {
        use engine::{memory::SliceMemory, walk::Outcome};
        for needed in [
            [false, false, false, false],
            [true, false, true, false],
            [false, true, false, true],
            [true, true, true, true],
        ] {
            let bytes = hourly_family_memory();
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let countries = country_snapshot(&memory, 64);
            let mut direct = hourly_family_jobs(&countries, needed);
            let calibration = SharedCalibration::default();
            let reports = direct
                .each_mut()
                .map(|job| run_with_memory(job, &memory, &calibration));
            let calibration = Arc::new(SharedCalibration::default());
            let runner_calibration = Arc::clone(&calibration);
            let pool = Pool {
                executor: acquisition::Pool::start(0, 3, move |job| {
                    run_with_memory(
                        job,
                        &SliceMemory {
                            base: 0,
                            bytes: &bytes,
                        },
                        &runner_calibration,
                    )
                })
                .unwrap(),
                calibration,
            };
            let [supply, portraits, province_order] = hourly_family_jobs(&countries, needed);
            let mut supply = Some(supply);
            let mut portraits = Some(portraits);
            let mut province_order = Some(province_order);
            let mut batch = Batch::new(&pool);
            let first = unsafe { batch.submit(&mut supply) }.unwrap();
            let second = unsafe { batch.submit(&mut portraits) }.unwrap();
            let third = unsafe { batch.submit(&mut province_order) }.unwrap();
            assert!(supply.is_none() && portraits.is_none() && province_order.is_none());
            let province_order = batch.finish(third).unwrap();
            let portraits = batch.finish(second).unwrap();
            let supply = batch.finish(first).unwrap();
            for ((completed, expected), expected_report) in [supply, portraits, province_order]
                .into_iter()
                .zip(direct)
                .zip(reports)
            {
                match (
                    completed.request,
                    expected,
                    completed.report.unwrap(),
                    expected_report,
                ) {
                    (
                        Job::SupplyTrains(actual),
                        Job::SupplyTrains(expected),
                        Report::SupplyTrains(report),
                        Report::SupplyTrains(reference),
                    ) => {
                        assert_eq!(actual.captured.tables(), expected.captured.tables());
                        assert_eq!(actual.captured.outcomes(), [Outcome::Rows { len: 0 }; 2]);
                        assert_eq!(
                            actual.captured.allocated_bytes(),
                            expected.captured.allocated_bytes()
                        );
                        assert_eq!(report.matched, reference.matched);
                        assert_eq!(report.matched, [needed[0], needed[1]]);
                        assert!(Arc::ptr_eq(&actual.countries, &countries));
                    }
                    (
                        Job::CharacterPortraits(actual),
                        Job::CharacterPortraits(expected),
                        Report::CharacterPortraits(report),
                        Report::CharacterPortraits(reference),
                    ) => {
                        assert_eq!(actual.captured.tables(), expected.captured.tables());
                        assert_eq!(actual.captured.outcomes(), [Outcome::Rows { len: 1 }]);
                        assert_eq!(
                            actual.captured.allocated_bytes(),
                            expected.captured.allocated_bytes()
                        );
                        assert_eq!(report.matched, reference.matched);
                        assert_eq!(report.matched, [needed[2]]);
                        assert!(Arc::ptr_eq(&actual.countries, &countries));
                        let [
                            replay::frame::TableView::Complete(
                                replay::rows::TableRows::CharacterPortraits(rows),
                            ),
                        ] = actual.captured.tables()
                        else {
                            panic!("one complete portrait row");
                        };
                        assert_eq!(rows[0].character, 73);
                        assert_eq!(rows[0].portraits[0].identity, [b'g', 0, 0xff]);
                        assert_eq!(rows[0].portraits[0].is_set, 255);
                    }
                    (
                        Job::ProvinceUnitOrder(actual),
                        Job::ProvinceUnitOrder(expected),
                        Report::ProvinceUnitOrder(report),
                        Report::ProvinceUnitOrder(reference),
                    ) => {
                        assert_eq!(actual.captured.tables(), expected.captured.tables());
                        assert_eq!(actual.captured.outcomes(), [Outcome::Rows { len: 1 }]);
                        assert_eq!(
                            actual.captured.allocated_bytes(),
                            expected.captured.allocated_bytes()
                        );
                        assert_eq!(report.matched, reference.matched);
                        assert_eq!(report.matched, [needed[3]]);
                    }
                    _ => panic!("owned hourly request/report family changed"),
                }
            }
            assert_eq!(Arc::strong_count(&countries), 1);
            for role in [
                Role::SupplyTrains,
                Role::CharacterPortraits,
                Role::ProvinceUnitOrder,
            ] {
                assert_eq!(pool.calibration().roles[role as usize].counters.accepted, 1);
                assert_eq!(
                    pool.calibration().roles[role as usize].counters.completed,
                    1
                );
            }
            assert_eq!(pool.calibration().executor.uncollected, 0);
            pool.shutdown().unwrap();
        }
    }

    #[test]
    fn hourly_owned_rejection_keeps_request_for_one_inline_completion() {
        struct NoReads;
        impl engine::Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("overflowed roots must fail before reading memory");
            }
        }
        for first_index in 0..3 {
            let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
            let mut jobs = hourly_family_jobs(&countries, [false, true, true, true]).map(Some);
            for job in jobs.iter_mut().flatten() {
                // The real inline path constructs LiveMemory, but these roots
                // overflow before any family can ask its provider to read.
                match job {
                    Job::SupplyTrains(request) => request.context.game_state = usize::MAX,
                    Job::CharacterPortraits(request) => request.context.game_state = usize::MAX,
                    Job::ProvinceUnitOrder(request) => request.context.game_state = usize::MAX,
                    _ => unreachable!(),
                }
            }
            let calibration = Arc::new(SharedCalibration::default());
            let runner_calibration = Arc::clone(&calibration);
            let pool = Pool {
                executor: acquisition::Pool::start(0, 1, move |job| {
                    run_with_memory(job, &NoReads, &runner_calibration)
                })
                .unwrap(),
                calibration,
            };
            let roles = [
                Role::SupplyTrains,
                Role::CharacterPortraits,
                Role::ProvinceUnitOrder,
            ];
            let rejected = (first_index + 1) % jobs.len();
            let mut batch = Batch::new(&pool);
            let ticket = unsafe { batch.submit(&mut jobs[first_index]) }.unwrap();
            assert!(matches!(
                unsafe { batch.submit(&mut jobs[rejected]) },
                Err(SubmitError::Full)
            ));
            assert!(jobs[first_index].is_none() && jobs[rejected].is_some());
            let accepted = batch.finish(ticket).unwrap();
            let inline = unsafe { batch.run_inline(jobs[rejected].take().unwrap()) };
            assert!(accepted.report.is_ok() && inline.report.is_ok());
            assert_eq!(accepted.request.role(), roles[first_index]);
            assert_eq!(inline.request.role(), roles[rejected]);
            for (index, job) in jobs.iter().enumerate() {
                assert_eq!(job.is_some(), index != first_index && index != rejected);
            }
            let counters = pool.calibration().roles;
            assert_eq!(counters[roles[first_index] as usize].counters.completed, 1);
            assert_eq!(counters[roles[rejected] as usize].counters.accepted, 0);
            assert_eq!(counters[roles[rejected] as usize].counters.rejected, 1);
            assert_eq!(
                counters[roles[rejected] as usize].counters.inline_completed,
                1
            );
            drop((accepted, inline));
            drop(jobs);
            assert_eq!(Arc::strong_count(&countries), 1);
            pool.shutdown().unwrap();
        }
    }

    #[test]
    fn hourly_owned_panic_and_cancellation_return_storage_and_country_leases() {
        struct PanicMemory(Arc<AtomicU64>);
        impl engine::Memory for PanicMemory {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("synthetic hourly family read failure");
            }
        }
        for family in 0..3 {
            for canceled in [false, true] {
                let reads = Arc::new(AtomicU64::new(0));
                let worker_reads = Arc::clone(&reads);
                let calibration = Arc::new(SharedCalibration::default());
                let runner_calibration = Arc::clone(&calibration);
                let pool = Pool {
                    executor: acquisition::Pool::start(0, 1, move |job| {
                        run_with_memory(
                            job,
                            &PanicMemory(Arc::clone(&worker_reads)),
                            &runner_calibration,
                        )
                    })
                    .unwrap(),
                    calibration,
                };
                let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
                let job = hourly_family_jobs(&countries, [false, true, true, true])
                    .into_iter()
                    .nth(family)
                    .unwrap();
                let allocated = match &job {
                    Job::SupplyTrains(request) => request.captured.allocated_bytes(),
                    Job::CharacterPortraits(request) => request.captured.allocated_bytes(),
                    Job::ProvinceUnitOrder(request) => request.captured.allocated_bytes(),
                    _ => unreachable!(),
                };
                let mut pending = Some(job);
                let mut batch = Batch::new(&pool);
                let ticket = unsafe { batch.submit(&mut pending) }.unwrap();
                if canceled {
                    pool.shutdown().unwrap();
                }
                let completion = batch.finish(ticket).unwrap();
                assert!(
                    matches!(completion.report, Err(failure) if failure == if canceled { Failure::Stopped } else { Failure::Panicked })
                );
                match completion.request {
                    Job::SupplyTrains(mut request) => {
                        assert_eq!(request.captured.allocated_bytes(), allocated);
                        assert!(Arc::ptr_eq(&request.countries, &countries));
                        let report = request
                            .captured
                            .fail_owned_capture(request.context, "terminal supply failure");
                        assert_eq!(report.matched, [false; 2]);
                        assert_eq!(report.ticks, [0; 2]); // Unmeasured failure cost.
                        assert_eq!(request.captured.checked_mismatches(), [0, 1]);
                        assert_eq!(request.captured.allocated_bytes(), allocated);
                    }
                    Job::CharacterPortraits(mut request) => {
                        assert_eq!(request.captured.allocated_bytes(), allocated);
                        assert!(Arc::ptr_eq(&request.countries, &countries));
                        let report = request
                            .captured
                            .fail_owned_capture(request.context, "terminal portraits failure");
                        assert_eq!(report.matched, [false]);
                        assert_eq!(report.ticks, [0]); // Unmeasured failure cost.
                        assert_eq!(request.captured.checked_mismatches(), [1]);
                        assert_eq!(request.captured.allocated_bytes(), allocated);
                    }
                    Job::ProvinceUnitOrder(mut request) => {
                        assert_eq!(request.captured.allocated_bytes(), allocated);
                        let report = request
                            .captured
                            .fail_owned_capture(request.context, "terminal province order failure");
                        assert_eq!(report.matched, [false]);
                        assert_eq!(report.ticks, [0]); // Unmeasured failure cost.
                        assert_eq!(request.captured.checked_mismatches(), [1]);
                        assert_eq!(request.captured.allocated_bytes(), allocated);
                    }
                    _ => unreachable!(),
                }
                assert_eq!(reads.load(Ordering::Relaxed), u64::from(!canceled));
                assert_eq!(Arc::strong_count(&countries), 1);
                assert_eq!(pool.calibration().executor.uncollected, 0);
                pool.shutdown().unwrap();
            }
        }
    }

    #[test]
    fn intel_residual_failure_preserves_owned_buffers_country_lease_and_delegated_slots() {
        struct PanicMemory(Arc<AtomicU64>);
        impl engine::Memory for PanicMemory {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("synthetic residual Intel source failure");
            }
        }
        let reads = Arc::new(AtomicU64::new(0));
        let worker_reads = Arc::clone(&reads);
        let calibration = Arc::new(SharedCalibration::default());
        let runner_calibration = Arc::clone(&calibration);
        let pool = Pool {
            executor: acquisition::Pool::start(0, 1, move |job| {
                run_with_memory(
                    job,
                    &PanicMemory(Arc::clone(&worker_reads)),
                    &runner_calibration,
                )
            })
            .unwrap(),
            calibration,
        };
        // A cache without cells uses the same direct-reader fallback and keeps
        // its immutable lease owned until the entire request has returned.
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let captured = intel::Captured::with_capacities(&Default::default(), &[0; intel::N]);
        let allocated = captured.allocated_bytes();
        let mut pending = Some(Job::IntelResidual(IntelResidualRequest {
            countries: Arc::clone(&countries),
            context: intel::CaptureContext {
                base: 0,
                game_state: 0x1000,
                hour: 0,
                force_full: true,
                needed: [true; intel::N],
            },
            captured,
        }));
        let mut batch = Batch::new(&pool);
        let ticket = unsafe { batch.submit(&mut pending) }.unwrap();
        let completion = batch.finish(ticket).unwrap();
        assert!(matches!(completion.report, Err(Failure::Panicked)));
        let Job::IntelResidual(mut request) = completion.request else {
            panic!("residual Intel request type changed");
        };
        assert_eq!(request.captured.allocated_bytes(), allocated);
        assert!(Arc::ptr_eq(&request.countries, &countries));
        assert_eq!(Arc::strong_count(&countries), 2);
        let before = request.captured.outcomes();
        let failure = request
            .captured
            .fail_residual_capture(request.context, "residual Intel acquisition failed");
        let after = request.captured.outcomes();
        for index in (1..=12).chain(std::iter::once(15)) {
            assert!(matches!(
                after[index],
                engine::walk::Outcome::Unavailable { .. }
            ));
            assert!(!failure.matched[index]);
        }
        for index in [0, 13, 14] {
            assert_eq!(after[index], before[index]);
        }
        assert_eq!(
            reads.load(Ordering::Relaxed),
            1,
            "terminal failure never retries source reads"
        );
        assert_eq!(
            pool.calibration().roles[Role::IntelResidual as usize]
                .counters
                .panicked,
            1
        );
        drop(request);
        assert_eq!(Arc::strong_count(&countries), 1);
    }

    #[test]
    fn presentation_failure_returns_owned_capture_and_both_independent_source_leases() {
        struct PanicMemory;
        impl engine::Memory for PanicMemory {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("synthetic presentation source failure");
            }
        }
        let calibration = Arc::new(SharedCalibration::default());
        let runner_calibration = Arc::clone(&calibration);
        let pool = Pool {
            executor: acquisition::Pool::start(0, 1, move |job| {
                run_with_memory(job, &PanicMemory, &runner_calibration)
            })
            .unwrap(),
            calibration,
        };
        let countries = empty_country_snapshot();
        let first = Arc::new(crate::division_census::UnitInputs::empty_rows());
        let second = Arc::new(crate::division_census::UnitInputs::new(0));
        let captured = division_presentation::Captured::with_capacities(
            &Default::default(),
            &[0; division_presentation::N],
        );
        let allocated = captured.allocated_bytes();
        let mut pending = Some(Job::Presentation(PresentationRequest {
            countries: Arc::clone(&countries),
            context: division_presentation::CaptureContext {
                base: 0,
                game_state: 0x1000,
                needed: [true; 2],
            },
            captured,
            first: Some(Arc::clone(&first)),
            second: Some(Arc::clone(&second)),
        }));
        let mut batch = Batch::new(&pool);
        let ticket = unsafe { batch.submit(&mut pending) }.unwrap();
        let completion = batch.finish(ticket).unwrap();
        assert!(matches!(completion.report, Err(Failure::Panicked)));
        let Job::Presentation(mut request) = completion.request else {
            panic!("presentation request type changed");
        };
        assert_eq!(request.captured.allocated_bytes(), allocated);
        assert!(Arc::ptr_eq(request.first.as_ref().unwrap(), &first));
        assert!(Arc::ptr_eq(request.second.as_ref().unwrap(), &second));
        assert_ne!(first.outcome(), second.outcome());
        request
            .captured
            .fail_owned_capture(request.context, "presentation acquisition failed");
        assert!(
            request
                .captured
                .outcomes()
                .iter()
                .all(|outcome| matches!(outcome, engine::walk::Outcome::Unavailable { .. }))
        );
        assert_eq!(
            pool.calibration().roles[Role::Presentation as usize]
                .counters
                .panicked,
            1
        );
        drop(request);
        assert_eq!(Arc::strong_count(&countries), 1);
        assert_eq!(Arc::strong_count(&first), 1);
        assert_eq!(Arc::strong_count(&second), 1);
    }

    #[test]
    fn characters_and_world_requests_share_executor_without_mixing_reports() {
        let pool = synthetic_pool(|job| match job {
            Job::World(_) => Report::World(WorldReport::default()),
            Job::Characters(_) => Report::Characters(characters::CharactersCopyReport::default()),
            _ => panic!("unexpected test job"),
        });
        let mut batch = Batch::new(&pool);
        let mut world = Some(Job::World(world_request(true)));
        let mut characters = Some(Job::Characters(CharactersRequest {
            countries: empty_country_snapshot(),
            context: CharactersCopyContext {
                base: 0,
                game_state: 0x1000,
                trait_rows: [0; 2],
                trait_bytes: [0; 2],
            },
            first: Default::default(),
            second: None,
        }));
        let a = unsafe { batch.submit(&mut world) }.unwrap();
        let b = unsafe { batch.submit(&mut characters) }.unwrap();
        let first = batch.finish(b).unwrap();
        assert!(matches!(first.request, Job::Characters(_)));
        assert!(matches!(first.report, Ok(Report::Characters(_))));
        let second = batch.finish(a).unwrap();
        assert!(matches!(second.report, Ok(Report::World(_))));
        let Job::World(request) = second.request else {
            panic!("wrong request")
        };
        assert!(request.economy.second.is_some());
        let roles = pool.calibration().roles;
        for role in [Role::World, Role::Characters] {
            assert_eq!(roles[role as usize].counters.accepted, 1);
            assert_eq!(roles[role as usize].counters.completed, 1);
        }
        assert_eq!(roles[Role::Eta as usize].counters.accepted, 0);
    }

    #[test]
    fn world_panic_recovers_owned_bundle_without_retry_and_separate_counters_do_not_mix() {
        let calls = Arc::new(AtomicU64::new(0));
        let worker_calls = Arc::clone(&calls);
        let pool = synthetic_pool(move |_request| {
            worker_calls.fetch_add(1, Ordering::Relaxed);
            panic!("synthetic World acquisition panic");
        });
        let mut request = Some(Job::World(world_request(true)));
        let mut batch = Batch::new(&pool);
        let ticket = unsafe { batch.submit(&mut request) }.unwrap();
        let completion = batch.finish(ticket).unwrap();
        assert!(matches!(completion.report, Err(Failure::Panicked)));
        let Job::World(recovered) = completion.request else {
            panic!("wrong request")
        };
        assert!(recovered.focus.second.is_some());
        pool.shutdown().unwrap();
        let mut rejected = Some(Job::World(recovered));
        assert!(unsafe { batch.submit(&mut rejected) }.is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let roles = pool.calibration().roles;
        assert_eq!(roles[Role::World as usize].counters.accepted, 1);
        assert_eq!(roles[Role::World as usize].counters.completed, 1);
        assert_eq!(roles[Role::World as usize].counters.panicked, 1);
        assert_eq!(roles[Role::World as usize].counters.rejected, 1);
        assert_eq!(roles[Role::Characters as usize].counters.accepted, 0);
    }

    #[test]
    fn callback_unwind_drains_every_accepted_job_and_releases_country_leases() {
        let calls = Arc::new(AtomicU64::new(0));
        let worker_calls = Arc::clone(&calls);
        let pool = synthetic_pool(move |_| {
            let call = worker_calls.fetch_add(1, Ordering::Relaxed);
            assert_ne!(call, 0, "first job fails during callback unwind");
            Report::World(WorldReport::default())
        });
        let countries = empty_country_snapshot();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let mut batch = Batch::new(&pool);
            for _ in 0..3 {
                let mut request = world_request(false);
                request.countries = Arc::clone(&countries);
                let mut request = Some(Job::World(request));
                unsafe { batch.submit(&mut request) }.unwrap();
            }
            for request in hourly_family_jobs(&countries, [false, true, true, true]) {
                let mut request = Some(request);
                unsafe { batch.submit(&mut request) }.unwrap();
            }
            panic!("synthetic callback unwind");
        }));
        assert!(outcome.is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(Arc::strong_count(&countries), 1);
        let calibration = pool.calibration();
        assert_eq!(calibration.executor.active, 0);
        assert_eq!(calibration.executor.uncollected, 0);
        assert_eq!(
            calibration.roles[Role::World as usize].counters.completed,
            3
        );
        assert_eq!(calibration.roles[Role::World as usize].counters.panicked, 1);
        assert_eq!(calibration.roles[Role::World as usize].counters.stopped, 2);
        for role in [
            Role::SupplyTrains,
            Role::CharacterPortraits,
            Role::ProvinceUnitOrder,
        ] {
            assert_eq!(calibration.roles[role as usize].counters.completed, 1);
            assert_eq!(calibration.roles[role as usize].counters.stopped, 1);
        }
    }

    #[test]
    fn caught_payload_destructor_cannot_interrupt_later_drainage() {
        struct PanicOnDrop;
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                panic!("caught panic payload destructor must never run");
            }
        }
        let result: Result<(), Failure> = catch_failure(|| std::panic::panic_any(PanicOnDrop));
        assert_eq!(result, Err(Failure::Panicked));
        assert_eq!(catch_failure(|| 7), Ok(7));
    }
}
