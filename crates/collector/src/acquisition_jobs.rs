//! Owned acquisition requests executed within the hour's live-read lease.
//! Job dependencies and the shared-country lease belong to the
//! callback, which dispatches jobs and joins their results. A join can execute
//! only its own queued job when caller execution is allowed; no family owns a
//! dedicated thread.

pub(crate) use crate::acquisition_countries::{
    Report as CountriesReport, Request as CountriesRequest,
};
pub(crate) use crate::acquisition_gather::{Report as GatherReport, Request as GatherRequest};
use crate::extended::{
    air, character_portraits, characters, construction, division_eta_inputs, division_presentation,
    economy, fog, governance, intel, lexical, naval_history, production, projects,
    province_unit_order, session, ship_models, supply_trains, weather,
};
use crate::{
    acquisition, acquisition_factions, acquisition_states, acquisition_wars, map_capture, sampling,
};
pub(crate) use acquisition::{Completion, Failure, SubmitError, Ticket};
use engine::memory::{CountedMemory, LiveMemory};
use serde::Serialize;
use std::sync::Arc;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub(crate) struct RawRequest<J, C> {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: C,
    pub payload: J,
}

pub(crate) struct OrderRootsRequest {
    pub base: usize,
    pub orders: crate::map_capture::Orders,
    pub naval: naval_history::NavalJob,
}

pub(crate) struct WorldPart<J, C> {
    pub context: C,
    pub payload: J,
}
/// Supply runs every hour; daily parts own a payload only when scheduled.
pub(crate) struct WorldRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub economy: WorldPart<economy::EconomyRestJob, economy::EconomyRestCopyContext>,
    pub construction: Option<
        WorldPart<construction::ConstructionRestJob, construction::ConstructionRestCopyContext>,
    >,
    pub production:
        Option<WorldPart<production::ProductionRestJob, production::ProductionRestCopyContext>>,
    pub focus: Option<WorldPart<governance::FocusJob, governance::FocusCopyContext>>,
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
    pub buildings: construction::Buildings,
    /// The technologies family's only fingerprint store, loaned for this hour.
    pub technology_fingerprints: Box<crate::slicing::SliceFingerprints>,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GovernanceReport {
    pub governance: governance::CaptureReport,
    pub buildings: u64,
}
impl GovernanceRequest {
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.captured.allocated_bytes() + self.buildings.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, reason: &'static str) -> GovernanceReport {
        self.captured.fail_owned_capture(self.context, reason);
        self.buildings.fail(reason);
        GovernanceReport::default()
    }
}
/// The variant registry and its detail tables, on their daily slot. Their
/// buffers grow from counts read during the walk, so only a worker runs it.
pub(crate) struct VariantsRequest {
    pub context: governance::CaptureContext,
    pub registry: crate::extended::Registry,
    pub variants: production::Variants,
}
impl VariantsRequest {
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.registry.allocated_bytes() + self.variants.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, reason: &'static str) -> [u64; production::N] {
        self.registry.fail(reason);
        self.variants
            .fail(reason, self.context.hour, self.context.force_full);
        [0; production::N]
    }
}
/// The governance family's hourly research tables, walked beside the rest
/// of the family (see `governance::ResearchPart`).
pub(crate) struct ResearchRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: governance::CaptureContext,
    pub part: governance::ResearchPart,
    /// The attachment's only fingerprint store, loaned for this hour.
    pub fingerprints: Box<crate::slicing::SliceFingerprints>,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ResearchReport {
    pub ticks: [u64; 3],
}
pub(crate) struct IntelResidualRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: intel::CaptureContext,
    pub captured: intel::Captured,
}
/// One country-slot share of the supply node census; part 0 also walks the
/// train inventory.
pub(crate) struct SupplyTrainsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: supply_trains::CaptureContext,
    pub part: usize,
    pub nodes: supply_trains::Part,
    pub trains: Option<supply_trains::Trains>,
}
/// One contiguous share of the character roster.
pub(crate) struct CharacterPortraitsRequest {
    pub context: character_portraits::CaptureContext,
    pub part: usize,
    pub captured: character_portraits::Part,
    pub census: Option<Arc<engine::ccharacter_manager::PortraitCensus>>,
}
pub(crate) struct OrdinaryRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub unit_inputs: Option<Arc<crate::division_census::UnitInputs>>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub fresh_modifier_guard: Result<(), &'static str>,
    pub transport_admission: crate::transport_admission::Admission,
    pub transport_modifier_guard: Result<(), &'static str>,
    pub packs: Box<crate::extended::OrdinaryPacks>,
    pub naval_missions: crate::extended::NavalMissionLoan,
}
impl OrdinaryRequest {
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.packs.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, reason: &'static str) -> OrdinaryReport {
        self.packs
            .fail(self.hour, self.naval_missions.force_full, reason);
        OrdinaryReport
    }
}
pub(crate) struct SlowRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub directed_query_admission: crate::transport_admission::Admission,
    pub packs: Box<crate::extended::SlowPacks>,
    /// The Slow job's stability fingerprint store, loaned for this hour.
    pub stores: crate::extended::SlowStores,
}
/// One part of the hour's AirCountryRelations copies
/// (`air_relations::PARTS`).
pub(crate) struct AirRelationsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub part: usize,
    pub staging: crate::extended::air_relations::Staging,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct AirRelationsReport;
/// The registered air missions and the naval-patrol combats, whose regions
/// open the history parts; the Late job joins the registrations it leaves.
pub(crate) struct AirMissionsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    /// The map census the typed visitor supplied the wing identities in.
    pub parent: engine::walk::Outcome,
    pub hour: i32,
    pub transport_guard: Result<(), &'static str>,
    pub transport_admission: crate::transport_admission::Admission,
    pub transport_regions: crate::extended::transport_spotting_context::RegionLoan,
    pub pack: Box<crate::extended::air::Captured>,
    pub histories: Vec<crate::extended::air_histories::Staging>,
}
impl AirMissionsRequest {
    /// A job the pool refused never runs: its growth could allocate on the
    /// game thread. It fails the registrations and combats and leaves the
    /// history parts without regions, as an interrupted job does.
    pub(crate) fn refuse(&mut self) {
        self.pack
            .fail_missions("air mission worker admission failed", &mut self.histories);
    }
}
#[derive(Clone, Copy, Default)]
pub(crate) struct AirMissionsReport;
/// One part of the hour's naval-patrol history copies
/// (`air_histories::PARTS`).
pub(crate) struct AirHistoriesRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub part: usize,
    pub staging: crate::extended::air_histories::Staging,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct AirHistoriesReport;
#[derive(Clone, Copy, Default)]
pub(crate) struct SlowReport;
pub(crate) struct RaidStateRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub packs: Box<crate::extended::RaidStatePacks>,
    /// The state and building-resource families' stores, loaned for this hour.
    pub stores: crate::extended::RaidStateStores,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct RaidStateReport;
pub(crate) struct RaidLaunchRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub packs: Box<crate::extended::RaidLaunchPacks>,
    /// The launch family's store, loaned for this hour.
    pub store: Box<crate::slicing::SliceFingerprints>,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct RaidLaunchReport;
pub(crate) struct RaidTargetsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub packs: Box<crate::extended::RaidTargetPacks>,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct RaidTargetsReport;
#[derive(Clone, Copy, Default)]
pub(crate) struct OrdinaryReport;
pub(crate) struct LateRequest {
    /// The same held country snapshot used by the Orders phase.
    pub countries: Arc<engine::country_memory::Cache>,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub packs: Box<crate::extended::LatePacks>,
    /// Completed plans and execution scratch; a Late failure never fails
    /// their independently acquired prefix or order execution scalars.
    pub plans: crate::extended::plans::Captured,
    /// The attachment's held RaidAirScores table, loaned for this hour.
    pub air_scores: Box<engine::raid_air_scores::Held>,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct LateReport;
/// The army order census and its plan operands. Its optional child operands
/// borrow the completed OOB copy from this same held pass; the Late job reads
/// the roots this job leaves.
pub(crate) struct OrdersRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub base: usize,
    /// This pass's game state, for the invasion preparations' countries and
    /// provinces.
    pub game_state: usize,
    pub orders: crate::map_capture::Orders,
    pub plans: crate::extended::plans::Captured,
    /// Loan of the accepted Naval copy, returned for its ordinary decode.
    /// An absent copy never permits a native fallback for child group inputs.
    pub group_source: Option<naval_history::NavalJob>,
}
impl OrdersRequest {
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.orders.allocated_bytes() + self.plans.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, reason: &'static str) -> OrdersReport {
        self.orders.fail(reason);
        self.plans.fail(reason);
        OrdersReport
    }
}
impl Drop for OrdersRequest {
    fn drop(&mut self) {
        // A drained/cancelled request or a callback unwind may bypass the
        // normal return. Retain the existing pooled payload without copying it.
        if let Some(payload) = self.group_source.take() {
            crate::decode::recycle_naval_job(payload);
        }
    }
}
#[derive(Clone, Copy, Default)]
pub(crate) struct OrdersReport;
/// The raw-gated packs, dispatched once every raw-copy verdict is in.
pub(crate) struct GatedRequest {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub packs: Box<crate::extended::GatedPacks>,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct GatedReport;
pub(crate) struct ProvinceUnitOrderRequest {
    pub context: province_unit_order::CaptureContext,
    pub captured: province_unit_order::Captured,
}
/// Independent core-map walk moved to the pool so its pointer chase overlaps the
/// other readers. Owns its pre-sized row buffers; the callback restores them.
pub(crate) type ProvincesRequest = GatherRequest<engine::cprovince::Capture>;
pub(crate) type ProvincesReport = GatherReport;
pub(crate) struct ShipModelsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: ship_models::CaptureContext,
    pub game_state: usize,
    pub captured: ship_models::Captured,
    /// The job walks only this census; its ships seed the detail parts,
    /// which the callback dispatches once it joins the census.
    pub census: crate::map_capture::ShipCensus,
}
/// One contiguous census range of the ShipModels details (part 0 is the
/// family's own capture).
pub(crate) struct ShipModelDetailsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: ship_models::CaptureContext,
    pub part: usize,
    pub captured: ship_models::Captured,
}
pub(crate) struct PresentationRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: division_presentation::CaptureContext,
    pub captured: division_presentation::Captured,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    /// The `retreat_bans` family's owner store, loaned for the hour.
    pub store: Option<Box<crate::slicing::SliceFingerprints>>,
}
impl PresentationRequest {
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.captured.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, reason: &'static str) -> division_presentation::CaptureReport {
        self.captured.fail_owned_capture(reason)
    }
}
/// One stable handle partition of ETA operands over the shared division
/// census. Every range is dispatched with Presentation, so the route scan runs
/// on several workers at once instead of one.
pub(crate) struct EtaInputsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: division_eta_inputs::CaptureContext,
    pub captured: division_eta_inputs::RangeCaptured,
    pub range: usize,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
}
pub(crate) struct IntelRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: fog::IntelCopyContext,
    pub payload: fog::IntelJob,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    pub input_failure: Option<&'static str>,
}
pub(crate) struct CharactersRequest {
    pub context: characters::CharactersCopyContext,
    pub payload: characters::CharactersJob,
    /// A role job's census loan returns with the request at its join.
    pub roles: characters::Roles<Arc<characters::census::RoleCensus>>,
}
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
    MapCombats(map_capture::CombatRequest),
    ObserverBoundary(crate::extended::observer_boundary::ObserverBoundaryRequest),
    FinalInputs(crate::extended::FinalInputsRequest),
    OrderRoots(OrderRootsRequest),
    DivisionInputs(DivisionsRequest),
    Divisions(DivisionsRequest),
    World(WorldRequest),
    Governance(GovernanceRequest),
    Variants(VariantsRequest),
    Research(ResearchRequest),
    Intel(IntelRequest),
    Presentation(PresentationRequest),
    EtaInputs(EtaInputsRequest),
    IntelResidual(IntelResidualRequest),
    SupplyTrains(SupplyTrainsRequest),
    CharacterPortraits(CharacterPortraitsRequest),
    ProvinceUnitOrder(ProvinceUnitOrderRequest),
    Ordinary(OrdinaryRequest),
    Slow(SlowRequest),
    RaidState(RaidStateRequest),
    RaidLaunch(RaidLaunchRequest),
    RaidTargets(RaidTargetsRequest),
    AirRelations(AirRelationsRequest),
    AirMissions(AirMissionsRequest),
    AirHistories(AirHistoriesRequest),
    Late(LateRequest),
    Orders(OrdersRequest),
    Gated(GatedRequest),
    ShipModels(ShipModelsRequest),
    ShipModelDetails(ShipModelDetailsRequest),
    Provinces(ProvincesRequest),
    Factions(acquisition_factions::Request),
    Countries(CountriesRequest),
    Wars(acquisition_wars::Request),
    States(acquisition_states::Request),
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
    MapCombats(map_capture::CombatReport),
    ObserverBoundary(crate::extended::observer_boundary::ObserverBoundaryReport),
    FinalInputs(crate::extended::FinalInputsReport),
    OrderRoots(u64),
    DivisionInputs(bool),
    Divisions(sampling::DivisionsCopyReport),
    World(WorldReport),
    Governance(GovernanceReport),
    Variants([u64; production::N]),
    Research(ResearchReport),
    Intel(fog::IntelCopyReport),
    Presentation(division_presentation::CaptureReport),
    EtaInputs(division_eta_inputs::CaptureReport),
    IntelResidual(intel::CaptureReport),
    SupplyTrains(supply_trains::CaptureReport),
    CharacterPortraits(character_portraits::CaptureReport),
    ProvinceUnitOrder(province_unit_order::CaptureReport),
    Ordinary(OrdinaryReport),
    Slow(SlowReport),
    RaidState(RaidStateReport),
    RaidLaunch(RaidLaunchReport),
    RaidTargets(RaidTargetsReport),
    AirRelations(AirRelationsReport),
    AirMissions(AirMissionsReport),
    AirHistories(AirHistoriesReport),
    Late(LateReport),
    Orders(OrdersReport),
    Gated(GatedReport),
    /// The census is in the request; the details report per part.
    ShipModels,
    ShipModelDetails(ship_models::CaptureReport),
    Provinces(ProvincesReport),
    Factions(acquisition_factions::Report),
    Countries(CountriesReport),
    Wars(acquisition_wars::Report),
    States(acquisition_states::Report),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(usize)]
pub(crate) enum Role {
    Characters,
    CountryLeaderRoles,
    AdvisorRoles,
    CharacterTraits,
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
    MapCombats,
    ObserverBoundary,
    Divisions,
    World,
    Governance,
    Variants,
    Research,
    Intel,
    Presentation,
    EtaInputs,
    IntelResidual,
    SupplyTrains,
    CharacterPortraits,
    ProvinceUnitOrder,
    Ordinary,
    Slow,
    RaidState,
    RaidLaunch,
    RaidTargets,
    AirRelations,
    AirMissions,
    AirHistories,
    Late,
    Orders,
    Gated,
    ShipModels,
    ShipModelDetails,
    Provinces,
    Factions,
    Countries,
    Wars,
    States,
}

impl Role {
    pub(crate) const fn worker_only(self) -> bool {
        true
    }
    /// Shared native work follows an hourly consumer in mixed families. Detail
    /// acquisition opens nested scopes when it reads its own sources.
    fn read_anchor(self) -> replay::rows::TableId {
        use replay::rows::TableId as T;
        match self {
            Self::Characters => T::Characters,
            Self::CountryLeaderRoles => T::CountryLeaderRoles,
            Self::AdvisorRoles => T::AdvisorRoles,
            Self::CharacterTraits => T::UnitLeaderTraits,
            Self::Relations => T::RelationTuples,
            Self::AirWing => T::AirWings,
            Self::AirStats => T::AirStatistics,
            Self::AirBases => T::AirBases,
            Self::AirHistory => T::AirRegionHistory,
            Self::Naval => T::OrdersGroups,
            Self::Supremacy => T::NavalSupremacy,
            Self::Sunk => T::SunkShips,
            Self::Capitulation => T::Capitulation,
            Self::Players => T::Players,
            Self::Projects => T::ProjectPrograms,
            Self::Lexical => T::LexicalTokens,
            Self::Session => T::WorldTension,
            Self::Weather => T::WeatherRegions,
            Self::Radar => T::RadarCoverage,
            Self::MapCombats => T::LandCombats,
            Self::ObserverBoundary => T::LandEventCoverage,
            Self::Divisions => T::Divisions,
            Self::World => T::ProvinceSupply,
            Self::Governance => T::Politics,
            Self::Variants => T::VariantDefinitions,
            Self::Research => T::ResearchInputs,
            Self::Intel => T::UnitIntel,
            Self::Presentation => T::DivisionMovement,
            Self::EtaInputs => T::DivisionEtaInputs,
            Self::IntelResidual => intel::TABLES[0],
            Self::SupplyTrains => supply_trains::TABLES[0],
            Self::CharacterPortraits => character_portraits::TABLES[0],
            Self::ProvinceUnitOrder => province_unit_order::TABLES[0],
            Self::Ordinary => T::LandCombatDetails,
            Self::Slow => T::ProvinceNames,
            Self::RaidState => T::StateResourceInputs,
            Self::RaidLaunch => T::RaidCountryInputs,
            Self::RaidTargets => T::RaidTargetStatuses,
            Self::AirRelations => T::AirCountryRelations,
            Self::AirMissions => T::AirRegionalMissions,
            Self::AirHistories => T::AirNavalPatrolHistories,
            Self::Late => T::WingEquipment,
            Self::Orders => T::ArmyOrders,
            Self::Gated => T::VolunteerHosts,
            Self::ShipModels => T::Ships,
            Self::ShipModelDetails => T::ShipModels,
            Self::Provinces => T::Provinces,
            Self::Factions => T::Factions,
            Self::Countries => T::Countries,
            Self::Wars => T::WarRelations,
            Self::States => T::States,
        }
    }
}

impl Role {
    /// Claim order when workers are scarce, lower first and FIFO among equals.
    /// World, Governance and Research come first because the block ends when
    /// they do; the rest follow the block segment that joins them. The
    /// ShipModels census is short and joined before the World join, so it
    /// claims first too. Its detail parts come right after the jobs that
    /// wait for no join, so on a small pool a worker-only family (Governance)
    /// never queues behind them.
    pub(crate) fn priority(self) -> u8 {
        match self {
            // The longest hourly jobs start first: the block ends when they do.
            Self::World
            | Self::Governance
            | Self::Research
            | Self::ShipModels
            | Self::ObserverBoundary => 0,
            Self::AirWing | Self::ShipModelDetails => 1,
            Self::Divisions
            | Self::Intel
            | Self::IntelResidual
            | Self::AirBases
            | Self::AirStats
            | Self::AirHistory
            | Self::Supremacy
            | Self::Sunk
            | Self::Radar
            | Self::Weather
            | Self::Capitulation
            | Self::Projects
            | Self::Lexical
            | Self::Session
            | Self::Players => 2,
            // The roster supplies portraits and the role census.
            Self::Characters | Self::Naval => 3,
            // Joined with Orders, before the Late job it feeds.
            Self::Orders
            | Self::MapCombats
            | Self::AirMissions
            | Self::Provinces
            | Self::States
            | Self::Countries
            | Self::Wars
            | Self::Factions => 4,
            // Role jobs borrow the roster's census and are joined after the
            // portrait shares, before the typed fallback.
            Self::SupplyTrains
            | Self::CharacterPortraits
            | Self::CountryLeaderRoles
            | Self::AdvisorRoles
            | Self::CharacterTraits
            | Self::ProvinceUnitOrder
            | Self::Relations
            | Self::Ordinary
            | Self::Slow
            | Self::AirRelations
            | Self::RaidState
            | Self::RaidLaunch
            | Self::RaidTargets
            | Self::Gated
            | Self::Variants => 5,
            Self::Presentation | Self::EtaInputs => 6,
            Self::Late | Self::AirHistories => 7,
        }
    }
}

pub(crate) const ROLE_COUNT: usize = 51;
pub(crate) const ROLES: [Role; ROLE_COUNT] = [
    Role::Characters,
    Role::CountryLeaderRoles,
    Role::AdvisorRoles,
    Role::CharacterTraits,
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
    Role::MapCombats,
    Role::ObserverBoundary,
    Role::Divisions,
    Role::World,
    Role::Governance,
    Role::Variants,
    Role::Research,
    Role::Intel,
    Role::Presentation,
    Role::EtaInputs,
    Role::IntelResidual,
    Role::SupplyTrains,
    Role::CharacterPortraits,
    Role::ProvinceUnitOrder,
    Role::Ordinary,
    Role::Slow,
    Role::RaidState,
    Role::RaidLaunch,
    Role::RaidTargets,
    Role::AirRelations,
    Role::AirMissions,
    Role::AirHistories,
    Role::Late,
    Role::Orders,
    Role::Gated,
    Role::ShipModels,
    Role::ShipModelDetails,
    Role::Provinces,
    Role::Factions,
    Role::Countries,
    Role::Wars,
    Role::States,
];
impl Job {
    pub(crate) fn role(&self) -> Role {
        match self {
            Self::Characters(request) => [
                Role::Characters,
                Role::CountryLeaderRoles,
                Role::AdvisorRoles,
                Role::CharacterTraits,
            ][request.payload.part],
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
            Self::MapCombats(..) => Role::MapCombats,
            Self::ObserverBoundary(..) => Role::ObserverBoundary,
            Self::DivisionInputs(..) | Self::Divisions(..) => Role::Divisions,
            Self::World(..) => Role::World,
            Self::Governance(..) => Role::Governance,
            Self::Variants(..) => Role::Variants,
            Self::Research(..) => Role::Research,
            Self::Intel(..) => Role::Intel,
            Self::FinalInputs(..) | Self::Presentation(..) => Role::Presentation,
            Self::EtaInputs(..) => Role::EtaInputs,
            Self::IntelResidual(..) => Role::IntelResidual,
            Self::SupplyTrains(..) => Role::SupplyTrains,
            Self::CharacterPortraits(..) => Role::CharacterPortraits,
            Self::ProvinceUnitOrder(..) => Role::ProvinceUnitOrder,
            Self::Ordinary(..) => Role::Ordinary,
            Self::Slow(..) => Role::Slow,
            Self::RaidState(..) => Role::RaidState,
            Self::RaidLaunch(..) => Role::RaidLaunch,
            Self::RaidTargets(..) => Role::RaidTargets,
            Self::AirRelations(..) => Role::AirRelations,
            Self::AirMissions(..) => Role::AirMissions,
            Self::AirHistories(..) => Role::AirHistories,
            Self::Late(..) => Role::Late,
            Self::OrderRoots(..) | Self::Orders(..) => Role::Orders,
            Self::Gated(..) => Role::Gated,
            Self::ShipModels(..) => Role::ShipModels,
            Self::ShipModelDetails(..) => Role::ShipModelDetails,
            Self::Provinces(..) => Role::Provinces,
            Self::Factions(..) => Role::Factions,
            Self::Countries(..) => Role::Countries,
            Self::Wars(..) => Role::Wars,
            Self::States(..) => Role::States,
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
    #[serde(serialize_with = "serialize_roles")]
    pub roles: [RoleCalibration; ROLE_COUNT],
}
fn serialize_roles<S: serde::Serializer>(
    roles: &[RoleCalibration; ROLE_COUNT],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    roles.as_slice().serialize(serializer)
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum CountryCacheSeed {
    Ready { entries: usize },
    Capacity { required_cells: usize },
    Unavailable { reason: &'static str },
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct CountryCacheCalibration {
    pub ready: u64,
    pub capacity_fallbacks: u64,
    pub unavailable_fallbacks: u64,
    pub latest: Option<CountryCacheSeed>,
}

impl CountryCacheCalibration {
    fn observe(&mut self, seed: engine::country_memory::SeedStatus, entries: usize) {
        self.latest = Some(match seed {
            engine::country_memory::SeedStatus::Ready => {
                self.ready = self.ready.saturating_add(1);
                CountryCacheSeed::Ready { entries }
            }
            engine::country_memory::SeedStatus::Capacity { required_cells } => {
                self.capacity_fallbacks = self.capacity_fallbacks.saturating_add(1);
                CountryCacheSeed::Capacity { required_cells }
            }
            engine::country_memory::SeedStatus::Unavailable { reason } => {
                self.unavailable_fallbacks = self.unavailable_fallbacks.saturating_add(1);
                CountryCacheSeed::Unavailable { reason }
            }
        });
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WorldCalibration {
    pub country_cache: CountryCacheCalibration,
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
    let _phase = crate::allocation::enter(crate::allocation::Phase::Acquisition);
    if crate::allocation::failed() {
        return Err(Failure::Stopped);
    }
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(report) if !crate::allocation::failed() => Ok(report),
        Ok(_) => Err(Failure::Stopped),
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
    world: Mutex<WorldCalibration>,
}
impl Default for SharedCalibration {
    fn default() -> Self {
        Self {
            roles: std::array::from_fn(|_| Counters::default()),
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
    let production = request
        .production
        .as_mut()
        .map_or_else(Default::default, |part| {
            production::copy_production_rest_refresh_growing(
                production_memory,
                part.context,
                &mut part.payload,
            )
        });
    let construction = request
        .construction
        .as_mut()
        .map_or_else(Default::default, |part| {
            construction::copy_construction_rest_refresh_growing(
                memory,
                part.context,
                &mut part.payload,
            )
        });
    let economy = economy::copy_economy_rest_refresh_growing(
        memory,
        request.economy.context,
        &mut request.economy.payload,
    );
    let focus = request
        .focus
        .as_mut()
        .map_or_else(Default::default, |part| {
            governance::copy_focus_refresh(memory, part.context, &mut part.payload)
        });
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
    // The callback owns preparation and retains the country lease through all jobs.
    report.ticks = crate::sampling::qpc().saturating_sub(started).max(0) as u64;
    let mut counters = calibration
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    counters.country_cache.observe(seed, entries);
    counters.acquisition_ticks = counters.acquisition_ticks.saturating_add(report.ticks);
    report
}

#[cfg(test)]
pub(crate) fn run_for_test(job: &mut Job, memory: &impl engine::Memory) -> Report {
    run_with_memory(job, memory, &SharedCalibration::default())
}

fn run_with_memory(
    job: &mut Job,
    memory: &impl engine::Memory,
    calibration: &SharedCalibration,
) -> Report {
    let started = sampling::qpc();
    let role = job.role();
    let _reads = crate::read_accounting::table(role.read_anchor());
    let report = match job {
        Job::Characters(request) => Report::Characters(characters::copy_characters_growing(
            memory,
            request.context,
            &mut request.payload,
            request.roles.as_deref(),
        )),
        Job::Relations(request) => {
            let cached = request.countries.view(memory);
            Report::Relations(intel::copy_relations_refresh(
                &cached,
                request.context,
                &mut request.payload,
            ))
        }
        Job::AirWing(request) => {
            let cached = request.countries.roster_view(memory);
            Report::AirWing(air::copy_air_refresh_growing(
                &cached,
                request.context,
                &mut request.payload,
            ))
        }
        Job::AirStats(request) => Report::AirStats(air::copy_air_stats_refresh_growing(
            memory,
            request.context,
            &mut request.payload,
        )),
        Job::AirBases(request) => Report::AirBases(air::copy_air_bases_refresh_growing(
            memory,
            request.context,
            &mut request.payload,
        )),
        Job::AirHistory(request) => {
            let cached = request.countries.roster_view(memory);
            Report::AirHistory(air::copy_air_history_refresh_growing(
                &cached,
                request.context,
                &mut request.payload,
            ))
        }
        Job::Naval(request) => {
            // The land theatre copy resolves countries through this
            // callback's authenticated roster; ordinary reads stay native.
            let cached = request.countries.roster_view(memory);
            Report::Naval(naval_history::copy_naval_refresh_growing(
                &cached,
                request.context,
                &mut request.payload,
            ))
        }
        Job::Supremacy(request) => {
            Report::Supremacy(naval_history::copy_supremacy_refresh_growing(
                memory,
                request.context,
                &mut request.payload,
            ))
        }
        Job::Sunk(request) => Report::Sunk(naval_history::copy_sunk_refresh_growing(
            memory,
            request.context,
            &mut request.payload,
        )),
        Job::Capitulation(request) => Report::Capitulation(intel::copy_capitulation_refresh(
            memory,
            request.context,
            &mut request.payload,
        )),
        Job::Players(request) => Report::Players(map_capture::copy_players_refresh(
            memory,
            request.context,
            &mut request.payload,
        )),
        Job::Projects(request) => Report::Projects(projects::copy_projects_refresh(
            memory,
            request.context,
            &mut request.payload,
        )),
        Job::Lexical(request) => Report::Lexical(lexical::copy_lexical_refresh_growing(
            memory,
            request.context,
            &mut request.payload,
        )),
        Job::Session(request) => Report::Session(session::copy_session_refresh_growing(
            memory,
            request.context,
            &mut request.payload,
        )),
        Job::Weather(request) => Report::Weather(weather::copy_weather_refresh_growing(
            memory,
            request.context,
            &mut request.payload,
        )),
        Job::Radar(request) => {
            let cached = request.countries.view(memory);
            Report::Radar(fog::copy_radar_refresh_growing(
                &cached,
                request.context,
                &mut request.payload,
            ))
        }
        Job::MapCombats(request) => Report::MapCombats(request.capture(memory)),
        Job::ObserverBoundary(request) => Report::ObserverBoundary(request.capture()),
        Job::FinalInputs(request) => {
            let countries = Arc::clone(&request.countries);
            let cached = countries.roster_view(memory);
            Report::FinalInputs(request.capture(&cached))
        }
        Job::OrderRoots(request) => {
            request.orders.walk_roots_growing(memory, request.base);
            Report::OrderRoots(
                request
                    .naval
                    .capture_roots_growing(request.orders.roots(), request.orders.root_outcome),
            )
        }
        Job::DivisionInputs(request) => {
            let cached = request.countries.view(memory);
            Report::DivisionInputs(sampling::copy_divisions_inputs_refresh_growing(
                &cached,
                request.context,
                &mut request.payload,
            ))
        }
        Job::Divisions(request) => {
            let cached = request.countries.view(memory);
            Report::Divisions(sampling::copy_divisions_core_refresh(
                &cached,
                request.context,
                &mut request.payload,
            ))
        }

        Job::World(request) => Report::World(run_world_shared(request, memory, &calibration.world)),
        Job::Governance(request) => {
            let cached = request.countries.roster_view(memory);
            let context = request.context;
            let governance = request.captured.capture_owned(
                &cached,
                context,
                &mut request.technology_fingerprints,
            );
            let buildings = request.buildings.capture(
                &cached,
                context.base,
                context.game_state,
                context.hour,
                context.force_full,
            );
            Report::Governance(GovernanceReport {
                governance,
                buildings,
            })
        }
        Job::Variants(request) => {
            let context = request.context;
            // The campaign registry includes variants absent from units,
            // production and stockpiles. Keep all entries and duplicates.
            let begin = sampling::qpc();
            request
                .registry
                .acquire(memory, context.base, context.game_state);
            let registry_ticks = sampling::qpc().saturating_sub(begin).max(0) as u64;
            let mut variants = request.variants.capture(
                memory,
                context.base,
                request.registry.sources(),
                context.hour,
                context.force_full,
            );
            variants[3] = variants[3].saturating_add(registry_ticks);
            Report::Variants(variants)
        }
        Job::Research(request) => {
            let cached = request.countries.roster_view(memory);
            Report::Research(ResearchReport {
                ticks: request
                    .part
                    .capture(&cached, request.context, &mut request.fingerprints),
            })
        }
        Job::IntelResidual(request) => {
            let cached = request.countries.view(memory);
            Report::IntelResidual(request.captured.capture_residual(&cached, request.context))
        }
        Job::SupplyTrains(request) => {
            let cached = request.countries.roster_view(memory);
            let mut ticks = [0; supply_trains::N];
            ticks[0] = request
                .nodes
                .capture(&cached, request.context, request.part);
            if let Some(trains) = request.trains.as_mut() {
                ticks[1] = trains.capture(&cached, request.context);
            }
            Report::SupplyTrains(supply_trains::CaptureReport { ticks })
        }
        Job::CharacterPortraits(request) => {
            Report::CharacterPortraits(character_portraits::CaptureReport {
                ticks: [request.captured.capture(
                    memory,
                    request.context,
                    request.part,
                    request.census.as_deref(),
                )],
            })
        }
        Job::ProvinceUnitOrder(request) => {
            Report::ProvinceUnitOrder(request.captured.capture_owned(memory, request.context))
        }
        Job::Ordinary(request) => {
            let cached = request.countries.roster_view(memory);
            request.packs.capture_with_transport_admission(
                &cached,
                request.base,
                request.game_state,
                request.unit_inputs.as_deref(),
                (request.hour, &mut request.naval_missions),
                request.fresh_modifier_guard,
                request
                    .transport_admission
                    .check(request.base, request.game_state, request.hour),
                request.transport_modifier_guard,
            );
            Report::Ordinary(OrdinaryReport)
        }
        Job::Slow(request) => {
            let cached = request.countries.roster_view(memory);
            request.packs.visibility.set_directed_query_admission(
                request.directed_query_admission.check(
                    request.base,
                    request.game_state,
                    request.hour,
                ),
            );
            request.packs.capture(
                &cached,
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
                &mut request.stores,
            );
            Report::Slow(SlowReport)
        }
        Job::AirRelations(request) => {
            let cached = request.countries.roster_view(memory);
            crate::extended::air_relations::stage(
                &cached,
                request.base,
                request.game_state,
                request.part,
                &crate::relation_changes::Observer,
                &mut crate::extended::air_relations::certificates(request.part),
                &mut request.staging,
            );
            Report::AirRelations(AirRelationsReport)
        }
        Job::AirMissions(request) => {
            let cached = request.countries.roster_view(memory);
            request.pack.capture_missions(
                &cached,
                request.base,
                request.game_state,
                air::MissionSources {
                    parent: request.parent,
                    histories: &mut request.histories,
                    changes: &crate::history_changes::Observer,
                    transport_regions: request.transport_regions.samples(request.hour),
                    transport_guard: request.transport_guard,
                    transport_admission: request.transport_admission.check(
                        request.base,
                        request.game_state,
                        request.hour,
                    ),
                },
            );
            Report::AirMissions(AirMissionsReport)
        }
        Job::AirHistories(request) => {
            let cached = request.countries.roster_view(memory);
            crate::extended::air_histories::stage(
                &cached,
                request.base,
                request.game_state,
                request.part,
                &crate::history_changes::Observer,
                &mut crate::extended::air_histories::certificates(request.part),
                &mut request.staging,
            );
            Report::AirHistories(AirHistoriesReport)
        }
        Job::RaidState(request) => {
            let cached = request.countries.roster_view(memory);
            request.packs.capture(
                &cached,
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
                &mut request.stores,
            );
            Report::RaidState(RaidStateReport)
        }
        Job::RaidLaunch(request) => {
            let cached = request.countries.roster_view(memory);
            request.packs.capture(
                &cached,
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
                &mut request.store,
            );
            Report::RaidLaunch(RaidLaunchReport)
        }
        Job::RaidTargets(request) => {
            let cached = request.countries.roster_view(memory);
            request.packs.capture(
                &cached,
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
            );
            Report::RaidTargets(RaidTargetsReport)
        }
        Job::Late(request) => {
            let cached = request.countries.roster_view(memory);
            request.packs.capture(
                memory,
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
                request.inputs.as_deref(),
                &mut request.air_scores,
            );
            let cost = request.plans.capture_unit_transports(
                &cached,
                request.base,
                request.game_state,
                request.inputs.as_deref(),
                &request.packs.orders,
            );
            request.packs.ticks[replay::rows::TableId::DivisionMovement.index()] += cost;
            Report::Late(LateReport)
        }
        Job::Orders(request) => {
            let cached = request.countries.roster_view(memory);
            request.orders.walk(memory, request.base);
            request.plans.visit(
                &cached,
                request.base,
                request.game_state,
                request.orders.roots(),
                request.orders.visits(),
                request.orders.successor_lists(),
                request.orders.root_outcome,
                request.orders.rows.outcome,
                &request.orders.rows.rows[..request.orders.rows.outcome.len()],
                &request.orders.geometry[..request.orders.geometry_len],
                request.group_source.as_ref(),
            );
            Report::Orders(OrdersReport)
        }
        Job::Gated(request) => {
            request.packs.capture(
                memory,
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
            );
            Report::Gated(GatedReport)
        }
        Job::ShipModels(request) => {
            let cached = request.countries.roster_view(memory);
            request.census.walk(
                &cached,
                request.context.base,
                request.game_state,
                &mut request.captured,
            );
            Report::ShipModels
        }
        Job::ShipModelDetails(request) => {
            let cached = request.countries.roster_view(memory);
            Report::ShipModelDetails(request.captured.capture_owned(&cached, request.context))
        }
        Job::Provinces(request) => {
            let (base, game_state) = (request.base, request.game_state);
            Report::Provinces(request.run(|capture| capture.walk_growing(memory, base, game_state)))
        }
        Job::Factions(request) => {
            Report::Factions(acquisition_factions::run_growing(request, memory))
        }
        Job::Countries(request) => Report::Countries(request.run(memory)),
        Job::Wars(request) => Report::Wars(request.run_growing(memory)),
        Job::States(request) => Report::States(request.run_growing(memory)),
        Job::Presentation(request) => {
            let cached = request.countries.roster_view(memory);
            let inputs = request.inputs.as_deref();
            let missing = engine::walk::Outcome::Unavailable {
                reason: "acquisition source not supplied",
            };
            Report::Presentation(request.captured.capture_owned(
                &cached,
                request.context,
                production::Sources {
                    rows: inputs.map_or(&[], |inputs| inputs.seeds()),
                    outcome: inputs.map_or(missing, |inputs| inputs.outcome()),
                },
                request.store.as_deref_mut(),
            ))
        }
        Job::EtaInputs(request) => {
            let cached = request.countries.view(memory);
            let inputs = request.inputs.as_deref();
            let missing = engine::walk::Outcome::Unavailable {
                reason: "acquisition source not supplied",
            };
            Report::EtaInputs(request.captured.capture_owned(
                memory,
                request.context,
                production::Sources {
                    rows: inputs.map_or(&[], |inputs| inputs.seeds()),
                    outcome: inputs.map_or(missing, |inputs| inputs.outcome()),
                },
                engine::Memory::authenticated_countries(
                    &cached,
                    request.context.base,
                    request.context.game_state,
                ),
                engine::Memory::authenticated_reserved_country_observation(
                    &cached,
                    request.context.base,
                    request.context.game_state,
                ),
            ))
        }
        Job::Intel(request) => {
            let cached = request.countries.roster_view(memory);
            let divisions = Some(if let Some(reason) = request.input_failure {
                engine::unit_viewer_entry::DivisionSources {
                    entries: &[],
                    outcome: engine::walk::Outcome::Unavailable { reason },
                }
            } else {
                request
                    .inputs
                    .as_ref()
                    .map(|inputs| inputs.intel_sources())
                    .unwrap_or(engine::unit_viewer_entry::DivisionSources {
                        entries: &[],
                        outcome: engine::walk::Outcome::Unavailable {
                            reason: "division input acquisition not supplied",
                        },
                    })
            });
            Report::Intel(fog::copy_intel_refresh_with_divisions_growing(
                &cached,
                request.context,
                &mut request.payload,
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

impl Role {
    /// Jobs of this role an hour can hold at once.
    const fn jobs_per_hour(self) -> usize {
        match self {
            Self::AirRelations => crate::extended::air_relations::PARTS,
            Self::EtaInputs => division_eta_inputs::MAX_RANGES,
            Self::AirHistories => crate::extended::air_histories::PARTS,
            _ => 1,
        }
    }
}

/// One reserved slot per worker-only job an hour can hold at once, derived
/// from the roles so a new worker-only role or part count stays covered.
const WORKER_ONLY_JOBS: usize = {
    let mut jobs = 0;
    let mut index = 0;
    while index < ROLE_COUNT {
        if ROLES[index].worker_only() {
            jobs += ROLES[index].jobs_per_hour();
        }
        index += 1;
    }
    jobs
};
const _: () = assert!(WORKER_ONLY_JOBS < MAX_JOBS);

pub(crate) struct Pool {
    executor: acquisition::Pool<Job, Report>,
    calibration: Arc<SharedCalibration>,
}
impl Pool {
    pub(crate) fn start(worker_count: usize, capacity: usize) -> Result<Self, &'static str> {
        let calibration = Arc::new(SharedCalibration::default());
        let runner_calibration = Arc::clone(&calibration);
        let executor = acquisition::Pool::start(worker_count.max(1), capacity, move |job| {
            // SAFETY: submit's callback lease remains admitted until every
            // accepted request is collected, including on callback unwind.
            let memory = CountedMemory::new(unsafe { LiveMemory::new() });
            run_with_memory(job, &memory, &runner_calibration)
        })?;
        executor.reserve_worker_slots(WORKER_ONLY_JOBS)?;
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
        let result = unsafe {
            self.executor.submit_with_priority(
                request,
                role.map_or(0, Role::priority),
                role.is_some_and(Role::worker_only),
            )
        };
        if let Some(role) = role {
            note_dispatch(&self.calibration.roles[role as usize], result.is_ok());
        }
        result
    }
    pub(crate) fn finish(&self, ticket: Ticket) -> Option<Completion<Job, Report>> {
        let completed = self.executor.finish(ticket)?;
        crate::read_accounting::merge(completed.table_reads);
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
    /// Execute an eligible retained, rejected request on the callback with the
    /// same providers and panic boundary. Worker-only roles cannot run here.
    ///
    /// # Safety
    /// The admitted callback lease must remain valid throughout this call.
    pub(crate) unsafe fn run_inline(&self, mut request: Job) -> Completion<Job, Report> {
        assert!(
            !request.role().worker_only(),
            "acquisition requires a worker"
        );
        let role = request.role();
        let reads = crate::read_accounting::Capture::new();
        let report = catch_failure(|| {
            // SAFETY: the caller retains the callback's live-read lease.
            let memory = CountedMemory::new(unsafe { LiveMemory::new() });
            run_with_memory(&mut request, &memory, &self.calibration)
        });
        let table_reads = reads.finish();
        crate::read_accounting::merge(table_reads);
        let counters = &self.calibration.roles[role as usize];
        counters.inline_completed.fetch_add(1, Ordering::Relaxed);
        if report.is_err() {
            counters.inline_panicked.fetch_add(1, Ordering::Relaxed);
        }
        Completion {
            request,
            report,
            table_reads,
            #[cfg(feature = "callback_profile")]
            profile: Default::default(),
        }
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
        }
    }
    pub(crate) fn shutdown(&self) -> Result<(), &'static str> {
        self.executor.shutdown()
    }
}

/// Upper bound on simultaneously retained callback requests, not table capacity.
pub(crate) const MAX_JOBS: usize = {
    let mut count = 1;
    let mut index = 0;
    while index < ROLE_COUNT {
        count += ROLES[index].jobs_per_hour();
        index += 1;
    }
    count
};
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
        let worker_only = request.as_ref().is_some_and(|job| job.role().worker_only());
        let slots = if worker_only {
            &mut self.tickets[..WORKER_ONLY_JOBS]
        } else {
            &mut self.tickets[WORKER_ONLY_JOBS..]
        };
        let Some(slot) = slots.iter_mut().find(|ticket| ticket.is_none()) else {
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
mod order_group_loan_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_accounting::{self, Capture};
    use characters::CharactersCopyContext;
    use engine::memory::ReadCounts;
    use replay::rows::TableId;

    fn accounted_read(owner: TableId, len: usize) {
        read_accounting::with_table(owner, || {
            let source = [7; 8];
            let memory = CountedMemory::new(engine::memory::SliceMemory {
                base: 0x1000,
                bytes: &source,
            });
            assert!(engine::Memory::copy(&memory, 0x1000, &mut [0; 8][..len]));
        });
    }

    fn one_copy(bytes: u64) -> ReadCounts {
        ReadCounts {
            copy_calls: 1,
            spans: 1,
            bytes,
            ..ReadCounts::ZERO
        }
    }

    fn country_snapshot(
        memory: &impl engine::Memory,
        cells: usize,
    ) -> Arc<engine::country_memory::Cache> {
        let mut storage =
            Arc::new(engine::country_memory::Cache::try_with_capacity(cells).unwrap());
        engine::country_memory::Cache::prepare_shared(&mut storage, memory, 0, 0x1000).unwrap()
    }

    fn empty_country_snapshot() -> Arc<engine::country_memory::Cache> {
        country_snapshot(
            &engine::memory::SliceMemory {
                base: 0,
                bytes: &empty_memory(),
            },
            64,
        )
    }

    fn empty_memory() -> Vec<u8> {
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

    fn world_request() -> WorldRequest {
        let mut request = WorldRequest {
            countries: empty_country_snapshot(),
            economy: WorldPart {
                context: economy::EconomyRestCopyContext {
                    base: 0,
                    game_state: 0x1000,
                    resource_rows: [0; 3],
                    resource_scratch: 0,
                    resources_active: true,
                    definitions_slot: true,
                    other_active: true,
                },
                payload: Default::default(),
            },
            construction: Some(WorldPart {
                context: construction::ConstructionRestCopyContext {
                    base: 0,
                    game_state: 0x1000,
                },
                payload: Default::default(),
            }),
            production: Some(WorldPart {
                context: production::ProductionRestCopyContext {
                    base: 0,
                    game_state: 0x1000,
                },
                payload: Default::default(),
            }),
            focus: Some(WorldPart {
                context: governance::Captured::with_capacities(&Default::default())
                    .focus_copy_context(0, 0x1000, true),
                payload: governance::FocusJob::new(),
            }),
        };
        request.economy.payload.log = crate::raw::RawLog::new(512, 65536);
        request.construction.as_mut().unwrap().payload.log = crate::raw::RawLog::new(512, 65536);
        request.production.as_mut().unwrap().payload.log = crate::raw::RawLog::new(512, 65536);
        request
    }

    fn synthetic_pool(run: impl Fn(&mut Job) -> Report + Send + Sync + 'static) -> Pool {
        let executor = acquisition::Pool::start(1, MAX_JOBS, run).unwrap();
        executor.reserve_worker_slots(WORKER_ONLY_JOBS).unwrap();
        Pool {
            executor,
            calibration: Arc::new(SharedCalibration::default()),
        }
    }

    #[test]
    fn every_worker_only_job_of_an_hour_has_a_reserved_slot() {
        // Every single worker-only role and all ETA, air relation and history parts.
        assert_eq!(
            WORKER_ONLY_JOBS,
            ROLES
                .iter()
                .copied()
                .filter(|role| role.worker_only())
                .map(Role::jobs_per_hour)
                .sum::<usize>()
        );
        assert!(
            ROLES
                .iter()
                .copied()
                .filter(|role| *role != Role::Players)
                .all(Role::worker_only)
        );
    }

    #[test]
    fn country_cache_calibration_distinguishes_each_seed_and_retains_fallback_counts() {
        use engine::country_memory::SeedStatus;
        let mut calibration = CountryCacheCalibration::default();
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
    fn intel_roster_view_preserves_aliases_native_reads_and_first_faults() {
        use engine::{Memory, ccountry, cgame_state, cunit, unit_viewer_entry, walk::Outcome};
        use fog::intel_test_kit as kit;
        use std::cell::RefCell;

        struct Traced<'a> {
            source: engine::memory::SliceMemory<'a>,
            reads: &'a RefCell<Vec<(usize, usize)>>,
            faults: &'a [usize],
        }
        impl Memory for Traced<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, destination.len()));
                if self
                    .faults
                    .iter()
                    .any(|&fault| fault >= address && fault - address < destination.len())
                {
                    return false;
                }
                self.source.copy(address, destination)
            }
        }
        fn decoded(job: &fog::IntelJob) -> (Outcome, replay::frame::TableView) {
            let mut captured = fog::Captured::with_capacities(&fog::Capacities {
                rows: [0, 0, 0, 0, 4, 0, 0],
                radar_provinces: 0,
                area_provinces: 0,
                wings: 0,
            });
            captured.decode_intel_payload(job, &mut [0]);
            (captured.outcomes()[4], captured.tables()[4].clone())
        }

        let mut bytes = kit::fixture();
        // The object's raw ID lies beyond the two-slot roster. Its link is
        // authenticated by the census, while UnitIntel only calls Reader::at.
        bytes[kit::C0 + ccountry::ID..kit::C0 + ccountry::ID + 4]
            .copy_from_slice(&2_u32.to_le_bytes());
        let links = engine::read::<u64>(
            &kit::memory(&bytes),
            kit::GAME_STATE + cgame_state::COUNTRY_LINKS,
        )
        .unwrap() as usize;
        bytes[links + 8..links + 12].copy_from_slice(&1_u32.to_le_bytes());
        // A changed, signed ordinary payload value must reach the native source.
        bytes[kit::VIEWERS + unit_viewer_entry::LEVEL] = (-7_i8) as u8;
        let context = fog::IntelCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            row_limit: 4,
        };
        for cells in [0, 64] {
            let reads = RefCell::new(Vec::new());
            let seed = Traced {
                source: kit::memory(&bytes),
                reads: &reads,
                faults: &[],
            };
            let mut storage =
                Arc::new(engine::country_memory::Cache::try_with_capacity(cells).unwrap());
            let countries = engine::country_memory::Cache::prepare_shared(
                &mut storage,
                &seed,
                kit::BASE,
                kit::GAME_STATE,
            )
            .unwrap();
            let authentication = reads.take();
            let cached = countries.view(&seed);
            assert_eq!(
                cached
                    .authenticated_countries(kit::BASE, kit::GAME_STATE)
                    .is_some(),
                cells != 0
            );
            let reader = ccountry::Reader::new(&cached, kit::BASE, kit::GAME_STATE).unwrap();
            assert_eq!(reader.count(), 2);
            assert_eq!(reader.at(1).unwrap().unwrap().id, 2);
            reads.borrow_mut().clear();

            for shared in [false, true] {
                let inputs = shared.then(|| {
                    Arc::new(crate::division_census::UnitInputs::fixture(
                        vec![engine::carmy::DivisionSeed {
                            pointer: kit::DIVISION,
                            roster_owner: Some(1),
                            ..Default::default()
                        }],
                        Outcome::Rows { len: 1 },
                    ))
                });
                for faults in [
                    &[][..],
                    &[kit::VIEWERS][..],
                    &[kit::DIVISION + cunit::HANDLE, kit::VIEWERS][..],
                ] {
                    let source = CountedMemory::new(Traced {
                        source: kit::memory(&bytes),
                        reads: &reads,
                        faults,
                    });
                    let mut previous = fog::IntelJob::new();
                    let capture = Capture::new();
                    let expected = fog::copy_intel_refresh_with_divisions_growing(
                        &countries.view(&source),
                        context,
                        &mut previous,
                        Some(
                            inputs
                                .as_ref()
                                .map(|inputs| inputs.intel_sources())
                                .unwrap_or(unit_viewer_entry::DivisionSources {
                                    entries: &[],
                                    outcome: Outcome::Unavailable {
                                        reason: "division input acquisition not supplied",
                                    },
                                }),
                        ),
                    );
                    let expected_counts = capture.finish();
                    let expected_reads = reads.take();
                    let mut job = Job::Intel(IntelRequest {
                        countries: Arc::clone(&countries),
                        context,
                        payload: fog::IntelJob::new(),
                        inputs: inputs.clone(),
                        input_failure: None,
                    });
                    let capture = Capture::new();
                    let Report::Intel(actual) =
                        run_with_memory(&mut job, &source, &SharedCalibration::default())
                    else {
                        unreachable!();
                    };
                    let counts = capture.finish();
                    let actual_reads = reads.take();
                    let Job::Intel(request) = job else {
                        unreachable!();
                    };
                    assert!(expected.raw_ok && actual.raw_ok);
                    assert_eq!(actual.raw_bytes, expected.raw_bytes);
                    assert_eq!(counts, expected_counts);
                    assert_eq!(actual_reads, expected_reads);
                    assert_eq!(decoded(&request.payload), decoded(&previous));
                    match (&request.inputs, &inputs) {
                        (Some(retained), Some(original)) => {
                            assert!(Arc::ptr_eq(retained, original));
                        }
                        (None, None) => {}
                        _ => panic!("original division inputs must be retained"),
                    }
                    let native = counts[TableId::UnitIntel.index()];
                    assert_eq!(native.spans > 0, shared);
                    assert_eq!(native.record_calls > 0, shared);
                    let mut attributed = read_accounting::ZERO;
                    attributed[TableId::UnitIntel.index()] = native;
                    assert_eq!(counts, attributed);
                    assert_eq!(
                        actual_reads
                            .iter()
                            .any(|span| authentication.contains(span)),
                        shared && cells == 0,
                        "ready rosters must not repeat native country authentication",
                    );
                    let (outcome, table) = decoded(&request.payload);
                    if !shared {
                        let reason = "division input acquisition not supplied";
                        assert_eq!(outcome, Outcome::Unavailable { reason });
                        assert_eq!(
                            table,
                            replay::frame::TableView::Unavailable {
                                reason: reason.to_owned(),
                            }
                        );
                        assert!(actual_reads.is_empty());
                        assert_eq!(counts, read_accounting::ZERO);
                        continue;
                    }
                    match faults.first() {
                        None => {
                            assert_eq!(outcome, Outcome::Rows { len: 2 });
                            let replay::frame::TableView::Complete(rows) = table else {
                                unreachable!();
                            };
                            assert_eq!(
                                rows.rows::<replay::rows::tables::UnitIntel>().unwrap()[0].level,
                                -7
                            );
                            assert!(
                                actual_reads
                                    .contains(&(kit::VIEWERS, 2 * unit_viewer_entry::STRIDE,))
                            );
                        }
                        Some(&fault) => {
                            let reason = if fault == kit::VIEWERS {
                                "unit viewer entry block unreadable"
                            } else {
                                "unit handle or viewer vector header unreadable"
                            };
                            assert_eq!(outcome, Outcome::Unavailable { reason });
                            assert_eq!(actual_reads.last().unwrap().0, fault);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn world_single_payload_matches_direct_copy_and_returns_storage() {
        let bytes = empty_memory();
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut direct = world_request();
        let expected = run_world_shared(
            &mut direct,
            &memory,
            &Mutex::new(WorldCalibration::default()),
        );
        let pool = acquisition::Pool::start(1, 1, move |request: &mut WorldRequest| {
            run_world_shared(
                request,
                &engine::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                &Mutex::new(WorldCalibration::default()),
            )
        })
        .unwrap();
        let request = world_request();
        let pointer = request.economy.payload.log.bytes.as_ptr();
        let mut pending = Some(request);
        // SAFETY: synthetic inputs and all destinations are owned until collection.
        let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
        let completed = pool.finish(ticket).unwrap();
        let report = completed.report.unwrap();
        assert_eq!(
            completed.request.economy.payload.log.bytes.as_ptr(),
            pointer
        );
        assert_eq!(report.economy.raw_ok, expected.economy.raw_ok);
        assert_eq!(report.economy.raw_bytes, expected.economy.raw_bytes);
        assert_eq!(
            completed.request.economy.payload.log.bytes,
            direct.economy.payload.log.bytes
        );
        macro_rules! compare {
            ($family:ident) => {{
                assert_eq!(report.$family.raw_ok, expected.$family.raw_ok);
                assert_eq!(report.$family.raw_bytes, expected.$family.raw_bytes);
                assert_eq!(
                    completed
                        .request
                        .$family
                        .as_ref()
                        .unwrap()
                        .payload
                        .log
                        .bytes,
                    direct.$family.as_ref().unwrap().payload.log.bytes
                );
            }};
        }
        compare!(construction);
        compare!(production);
        assert_eq!(report.focus.raw_ok, expected.focus.raw_ok);
        assert_eq!(report.focus.raw_bytes, expected.focus.raw_bytes);
        assert_eq!(pool.calibration().uncollected, 0);
    }

    #[test]
    fn absent_world_parts_preserve_only_the_economy_attempt_and_owned_storage() {
        for workers in [0, 1] {
            let bytes = empty_memory();
            let make = || {
                let mut request = world_request();
                request.production = None;
                request.construction = None;
                request.focus = None;
                request
            };
            let mut direct = make();
            let memory = CountedMemory::new(engine::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            });
            let reads = Capture::new();
            let expected = economy::copy_economy_rest_refresh(
                &direct.countries.view(&memory),
                direct.economy.context,
                &mut direct.economy.payload,
            );
            let expected_reads = reads.finish();
            let pool = acquisition::Pool::start(workers, 1, move |request: &mut WorldRequest| {
                run_world_shared(
                    request,
                    &CountedMemory::new(engine::memory::SliceMemory {
                        base: 0,
                        bytes: &bytes,
                    }),
                    &Mutex::new(WorldCalibration::default()),
                )
            })
            .unwrap();
            let request = make();
            let pointer = request.economy.payload.log.bytes.as_ptr();
            let mut pending = Some(request);
            // SAFETY: the runner owns its source and collection retains every output.
            let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
            let completed = pool.finish(ticket).unwrap();
            let report = completed.report.unwrap();
            assert!(completed.request.production.is_none());
            assert!(completed.request.construction.is_none());
            assert!(completed.request.focus.is_none());
            assert_eq!(
                completed.request.economy.payload.log.bytes.as_ptr(),
                pointer
            );
            assert_eq!(
                completed.request.economy.payload.log.bytes,
                direct.economy.payload.log.bytes
            );
            assert_eq!(report.economy.raw_ok, expected.raw_ok);
            assert_eq!(report.economy.raw_bytes, expected.raw_bytes);
            assert_eq!(completed.table_reads, expected_reads);
            macro_rules! empty {
                ($part:ident) => {{
                    assert!(!report.$part.raw_ok);
                    assert!(report.$part.copy_ticks.iter().all(|&value| value == 0));
                    assert!(report.$part.raw_bytes.iter().all(|&value| value == 0));
                }};
            }
            empty!(production);
            empty!(construction);
            empty!(focus);
            assert!(!report.production.plan_failed);
            assert_eq!(pool.calibration().uncollected, 0);
        }
    }

    #[test]
    fn single_requests_share_executor_without_mixing_reports() {
        let capture = Capture::new();
        accounted_read(TableId::Clock, 1);
        let pool = synthetic_pool(|job| match job {
            Job::World(_) => {
                accounted_read(TableId::ProductionLines, 3);
                Report::World(WorldReport::default())
            }
            Job::Characters(_) => {
                accounted_read(TableId::Characters, 2);
                Report::Characters(characters::CharactersCopyReport::default())
            }
            _ => panic!("unexpected test job"),
        });
        let mut batch = Batch::new(&pool);
        let mut world = Some(Job::World(world_request()));
        let mut characters = Some(Job::Characters(CharactersRequest {
            context: CharactersCopyContext {
                force_full: false,
                base: 0,
                game_state: 0x1000,
                trait_rows: [0; 2],
                trait_bytes: [0; 2],
                active: [true; 6],
            },
            payload: Default::default(),
            roles: characters::Roles::Independent,
        }));
        // SAFETY: these runners do not access native memory and both jobs are drained.
        let world_ticket = unsafe { batch.submit(&mut world) }.unwrap();
        let characters_ticket = unsafe { batch.submit(&mut characters) }.unwrap();
        let completed = batch.finish(characters_ticket).unwrap();
        assert!(matches!(completed.request, Job::Characters(_)));
        assert!(matches!(completed.report, Ok(Report::Characters(_))));
        let mut expected = read_accounting::ZERO;
        expected[TableId::Characters.index()] = one_copy(2);
        assert_eq!(completed.table_reads, expected);
        expected[TableId::Clock.index()] = one_copy(1);
        assert_eq!(read_accounting::snapshot(), expected);
        assert!(matches!(
            batch.finish(world_ticket).unwrap().report,
            Ok(Report::World(_))
        ));
        let roles = pool.calibration().roles;
        for role in [Role::World, Role::Characters] {
            assert_eq!(roles[role as usize].counters.accepted, 1);
            assert_eq!(roles[role as usize].counters.completed, 1);
        }
        assert_eq!(roles[Role::Intel as usize].counters.accepted, 0);
        expected[TableId::ProductionLines.index()] = one_copy(3);
        assert_eq!(read_accounting::snapshot(), expected);
        assert!(batch.finish(characters_ticket).is_none());
        assert!(batch.finish(world_ticket).is_none());
        drop(batch);
        assert_eq!(capture.finish(), expected);
    }

    #[test]
    fn rejected_provinces_worker_retains_its_storage_without_inline_reads() {
        for workers in [0, 1] {
            // This immutable fixture outlives every accepted read.
            // Its empty header stops the walk before any pointer can be followed.
            let source = vec![0_u8; engine::cgame_state::PROVINCES + 16];
            for (game_state, reason, reads) in [
                (
                    usize::MAX,
                    "province array unreadable or invalid",
                    ReadCounts::ZERO,
                ),
                (
                    source.as_ptr() as usize,
                    "province table has no province slots",
                    one_copy(16),
                ),
            ] {
                let capture = Capture::new();
                accounted_read(TableId::Clock, 3);
                let pool = Pool::start(workers, WORKER_ONLY_JOBS + 1).unwrap();
                let make = || {
                    Job::Provinces(ProvincesRequest {
                        base: 0,
                        game_state,
                        payload: engine::cprovince::Capture::try_with_capacity(2, 0, 0).unwrap(),
                    })
                };
                let mut retained = Some(make());
                let pointer = match retained.as_ref().unwrap() {
                    Job::Provinces(request) => request.payload.rows.as_ptr(),
                    _ => unreachable!(),
                };
                let mut batch = Batch::new(&pool);
                // SAFETY: roots overflow or read the owned header; Batch drains
                // all accepted jobs before the fixture can be dropped.
                let mut tickets = Vec::with_capacity(WORKER_ONLY_JOBS);
                for _ in 0..WORKER_ONLY_JOBS {
                    let mut accepted = Some(make());
                    tickets.push(unsafe { batch.submit(&mut accepted) }.unwrap());
                    assert!(accepted.is_none());
                }
                assert_eq!(
                    unsafe { batch.submit(&mut retained) },
                    Err(SubmitError::Full)
                );
                assert!(retained.is_some());
                let rejected = retained.take().unwrap();
                assert!(rejected.role().worker_only());
                let Job::Provinces(request) = &rejected else {
                    panic!("province request routing changed");
                };
                assert_eq!(request.payload.rows.as_ptr(), pointer);
                let mut job_reads = read_accounting::ZERO;
                job_reads[TableId::Provinces.index()] = reads;
                let mut expected = read_accounting::ZERO;
                expected[TableId::Clock.index()] = one_copy(3);
                // Accepted work stays isolated until its separate collection.
                assert_eq!(read_accounting::snapshot(), expected);
                for ticket in tickets {
                    let completed = batch.finish(ticket).unwrap();
                    assert_eq!(completed.table_reads, job_reads);
                    let Ok(Report::Provinces(report)) = completed.report else {
                        panic!("province report routing changed");
                    };
                    assert_eq!(report.result, engine::walk::Outcome::Unavailable { reason });
                    expected[TableId::Provinces.index()] += reads;
                }
                let calibration = pool.calibration();
                let counts = calibration.roles[Role::Provinces as usize].counters;
                assert_eq!(counts.accepted, WORKER_ONLY_JOBS as u64);
                assert_eq!(counts.completed, WORKER_ONLY_JOBS as u64);
                assert_eq!(counts.rejected, 1);
                assert_eq!(counts.inline_completed, 0);
                assert_eq!(calibration.executor.uncollected, 0);
                drop(batch);
                assert_eq!(capture.finish(), expected);
            }
        }
    }

    /// Exhausting reserved worker slots retains the rejected relations payload.
    /// Its join returns the original slice and fingerprint store and publishes
    /// unavailable tables without callback reads or inline execution.
    #[test]
    fn a_rejected_relations_worker_returns_its_slice_without_callback_reads() {
        use crate::slicing::{SliceFingerprints, SliceSelection};
        for workers in [0, 1] {
            let capture = Capture::new();
            let pool = Pool::start(workers, WORKER_ONLY_JOBS + 1).unwrap();
            let make = || {
                let selection = SliceSelection::for_test(5, false, &[7]).with_census(&[7]);
                Job::Relations(RawRequest {
                    countries: empty_country_snapshot(),
                    // An unreadable game state: the census and the walk fail
                    // before any pointer can be followed.
                    context: intel::RelationsCopyContext {
                        base: 0,
                        game_state: usize::MAX,
                        active: [true; 2],
                        opinion_cap: 4,
                        tuple_cap: 4,
                        epoch: 0,
                    },
                    payload: {
                        let mut job = intel::RelationsJob::new();
                        job.selection = Some(selection);
                        job.fingerprints = Some(Box::new(SliceFingerprints::new()));
                        job
                    },
                })
            };
            let mut retained = Some(make());
            let store = |job: &Job| match job {
                Job::Relations(request) => {
                    request.payload.fingerprints.as_deref().unwrap() as *const SliceFingerprints
                }
                _ => unreachable!(),
            };
            let original_store = store(retained.as_ref().unwrap());
            let Job::Relations(request) = retained.as_ref().unwrap() else {
                unreachable!()
            };
            let original_selection = request.payload.selection.as_ref().unwrap();
            let selected: Vec<_> = (0..32)
                .map(|owner| original_selection.contains(owner))
                .collect();
            let mut batch = Batch::new(&pool);
            // SAFETY: the walks fail on the unreadable game state; Batch
            // drains every accepted job before this frame ends.
            let mut tickets = Vec::new();
            for _ in 0..WORKER_ONLY_JOBS {
                let mut accepted = Some(make());
                tickets.push(unsafe { batch.submit(&mut accepted) }.unwrap());
            }
            assert_eq!(
                unsafe { batch.submit(&mut retained) },
                Err(SubmitError::Full)
            );
            let Job::Relations(request) = retained.take().unwrap() else {
                panic!("relations request routing changed");
            };
            let reason = "acquisition worker stopped before completion";
            assert!(!request.payload.published());
            let mut pack = intel::Captured::vacant();
            let (fingerprints, payload) = pack.join_relations(
                request.payload,
                intel::RelationsCopyReport::default(),
                Some(reason),
            );
            assert_eq!(
                fingerprints
                    .as_deref()
                    .map(|store| store as *const SliceFingerprints),
                Some(original_store)
            );
            assert_eq!(
                (0..32)
                    .map(|owner| pack.relations_selection().contains(owner))
                    .collect::<Vec<_>>(),
                selected
            );
            assert_eq!(
                pack.relations_selection().read_owners().collect::<Vec<_>>(),
                [7]
            );
            let Err(job) = payload else {
                panic!("a rejected worker payload must be recycled");
            };
            assert!(job.selection.is_none() && job.fingerprints.is_none());
            assert!(pack.relations_raw_mode, "no typed fallback read");
            pack.fail_relations_capture([true; 2], reason);
            for index in [13, 14] {
                assert_eq!(
                    pack.outcomes()[index],
                    engine::walk::Outcome::Unavailable { reason }
                );
            }
            assert_eq!(read_accounting::snapshot(), read_accounting::ZERO);
            for ticket in tickets {
                assert!(batch.finish(ticket).is_some());
            }
            let counts = pool.calibration().roles[Role::Relations as usize].counters;
            assert_eq!(counts.accepted, WORKER_ONLY_JOBS as u64);
            assert_eq!(counts.completed, WORKER_ONLY_JOBS as u64);
            assert_eq!(counts.rejected, 1);
            assert_eq!(counts.inline_completed, 0);
            drop(batch);
            assert_eq!(capture.finish(), read_accounting::ZERO);
        }
    }

    #[test]
    fn callback_unwind_drains_jobs_and_releases_single_country_leases() {
        let capture = Capture::new();
        accounted_read(TableId::Clock, 2);
        let calls = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&calls);
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        let pool = synthetic_pool(move |_| {
            assert!(wait.lock().unwrap().recv().is_err());
            observed.fetch_add(1, Ordering::Relaxed);
            accounted_read(TableId::ProductionLines, 3);
            accounted_read(TableId::Focus, 4);
            panic!("synthetic acquisition panic");
        });
        let countries = empty_country_snapshot();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let mut batch = Batch::new(&pool);
            // Drop the sender before Batch drains during unwind. The worker
            // cannot panic and stop admission until all three jobs were queued.
            let _release = release;
            for _ in 0..3 {
                let mut request = world_request();
                request.countries = Arc::clone(&countries);
                let mut pending = Some(Job::World(request));
                // SAFETY: every accepted synthetic request is drained by Batch on unwind.
                unsafe { batch.submit(&mut pending) }.unwrap();
            }
            panic!("synthetic callback unwind");
        }));
        assert!(outcome.is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(Arc::strong_count(&countries), 1);
        let stats = pool.calibration();
        assert_eq!(stats.executor.active, 0);
        assert_eq!(stats.executor.uncollected, 0);
        assert_eq!(stats.roles[Role::World as usize].counters.completed, 3);
        assert_eq!(stats.roles[Role::World as usize].counters.panicked, 1);
        assert_eq!(stats.roles[Role::World as usize].counters.stopped, 2);
        let mut expected = read_accounting::ZERO;
        expected[TableId::Clock.index()] = one_copy(2);
        expected[TableId::ProductionLines.index()] = one_copy(3);
        expected[TableId::Focus.index()] = one_copy(4);
        assert_eq!(capture.finish(), expected);
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
