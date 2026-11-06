//! Owned acquisition requests executed within the hour's live-read lease.
//! The callback owns the shared-country lease and ordered result settlement;
//! retained publication continuations may release complete dependency fan-ins.
//! A join can execute
//! its own queued job when caller execution is allowed; optional executor join
//! help can run other eligible queued jobs within learned body-time bounds.
//! No family owns a dedicated thread.

pub(crate) use crate::acquisition_countries::{
    Report as CountriesReport, Request as CountriesRequest, TemplatesReport, TemplatesRequest,
};
pub(crate) use crate::acquisition_gather::{Report as GatherReport, Request as GatherRequest};
use crate::extended::{
    air, character_portraits, characters, construction, division_presentation, economy, fog,
    governance, intel, lexical, naval_history, production, projects, province_unit_order, session,
    ship_models, supply_trains, weather,
};
use crate::{
    acquisition, acquisition_factions, acquisition_states, acquisition_wars, map_capture, sampling,
};
pub(crate) use acquisition::{Completion, Failure, SubmitError, Ticket};
mod presentation;
pub(crate) mod ship_details;
mod variant_workers;
use engine_native::{memory::CountedMemory, memory::LiveMemory};
pub(crate) use presentation::{
    FinalInputsDispatch, PresentationDispatch, PresentationFanIn, PresentationInputs,
};
use serde::Serialize;
use ship_details::ShipDetailsDispatch;
use std::sync::Arc;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub(crate) struct RawRequest<J, C> {
    pub countries: Arc<engine_native::country_memory::Cache>,
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
/// The production and construction rests; each part owns a payload only in
/// its scheduled hour. The economy rest is its own [`EconomyRequest`].
pub(crate) struct WorldRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub construction: Option<
        WorldPart<construction::ConstructionRestJob, construction::ConstructionRestCopyContext>,
    >,
    pub production:
        Option<WorldPart<production::ProductionRestJob, production::ProductionRestCopyContext>>,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WorldReport {
    pub construction: construction::ConstructionRestCopyReport,
    pub production: production::ProductionRestCopyReport,
    pub ticks: u64,
}
/// The economy rest (province supply every hour, the resource census and its
/// consumers hourly, the other tables in their daily slot), its own
/// first-burst job beside World.
pub(crate) type EconomyRequest =
    RawRequest<economy::EconomyRestJob, economy::EconomyRestCopyContext>;
pub(crate) struct GovernanceRequest {
    pub workers: variant_workers::Dispatch,
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub context: governance::CaptureContext,
    pub captured: governance::Captured,
    pub buildings: construction::Buildings,
    /// Held StateBuildings and ProvinceBuildings rows
    /// ([`construction::BuildingStore`]); absent reads every status afresh.
    pub building_store: Option<Box<construction::BuildingStore>>,
    /// Held PartyLeaders rows ([`governance::LeaderStore`]); absent reads
    /// every country's leaders afresh.
    pub leader_store: Option<Box<governance::LeaderStore>>,
    /// Held PoliticalParties and ActiveIdeas rows
    /// ([`governance::PartyStore`]); absent reads every country's parties
    /// and ideas afresh.
    pub party_store: Option<Box<governance::PartyStore>>,
    /// The technologies family's only fingerprint store, loaned for this hour.
    pub technology_fingerprints: Box<crate::slicing::SliceFingerprints>,
    /// The focus family's only fingerprint store, loaned for this hour.
    pub focus_fingerprints: Box<crate::slicing::SliceFingerprints>,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GovernanceReport {
    pub governance: governance::CaptureReport,
    pub buildings: u64,
}
impl GovernanceRequest {
    #[cfg(test)]
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.captured.allocated_bytes() + self.buildings.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, reason: &'static str) -> GovernanceReport {
        self.captured.fail_owned_capture(self.context, reason);
        self.buildings.fail(reason);
        GovernanceReport::default()
    }
}
/// The hourly design changes, then the variant registry and its detail tables
/// when scheduled or recovering. Only a worker drains or grows their storage.
pub(crate) struct VariantsRequest {
    pub context: governance::CaptureContext,
    pub registry: crate::extended::Registry,
    pub variants: production::Variants,
    pub changes: production::design_changes::Captured,
    pub changes_ticks: u64,
    pub changes_finished: bool,
    pub workers: variant_workers::Dispatch,
}
impl VariantsRequest {
    #[cfg(test)]
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.registry.allocated_bytes()
            + self.variants.allocated_bytes()
            + self.changes.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, reason: &'static str) -> [u64; production::N] {
        if !self.changes_finished {
            self.changes.fail(reason);
        }
        self.registry.fail(reason);
        self.variants
            .fail(reason, self.context.hour, self.context.force_full);
        [0; production::N]
    }
}
/// Membership/recovery admission is separate from an explicitly fresh read.
/// Recovery retains only independently certified, untouched deep values.
fn variant_read_plan(hour: i32, force_full: bool, recovery: bool) -> (bool, bool) {
    (
        force_full
            || recovery
            || replay_data::cadence::should_walk(
                replay_data::rows::TableId::VariantDefinitions,
                hour,
                false,
            ),
        force_full,
    )
}
/// The governance family's hourly research tables, walked beside the rest
/// of the family (see `governance::ResearchPart`).
pub(crate) struct ResearchRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
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
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub context: intel::CaptureContext,
    pub captured: intel::Captured,
    /// Held ScriptFlags rows ([`intel::FlagStore`]); absent reads every
    /// manager afresh.
    pub flags: Option<Box<intel::FlagStore>>,
    /// Held ScriptVariables rows ([`intel::VariableStore`]); absent reads
    /// every store afresh.
    pub variables: Option<Box<intel::VariableStore>>,
}
/// One country-slot share of the supply node census; part 0 also walks the
/// train inventory and carries the hour's owner selection with the family's
/// fingerprint store.
pub(crate) struct SupplyTrainsRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub context: supply_trains::CaptureContext,
    pub part: usize,
    pub nodes: supply_trains::Part,
    pub trains: Option<supply_trains::Trains>,
    pub slice: Option<supply_trains::Slice>,
}
/// One contiguous share of the character roster.
pub(crate) struct CharacterPortraitsRequest {
    pub context: character_portraits::CaptureContext,
    pub part: usize,
    pub captured: character_portraits::Part,
    pub census: Option<Arc<engine_native::ccharacter_manager::PortraitCensus>>,
}
/// The Ordinary family's counter part, dispatched once the Countries job has
/// restored the naming buffers its reserved visibility reads.
pub(crate) struct OrdinaryRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub packs: Box<crate::extended::OrdinaryPacks>,
    /// Held CountryCounterInputs rows
    /// ([`crate::extended::country_counters::CounterStore`]); absent reads
    /// every country afresh.
    pub counters: Option<Box<crate::extended::country_counters::CounterStore>>,
}
impl OrdinaryRequest {
    #[cfg(test)]
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.packs.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, reason: &'static str) -> OrdinaryReport {
        self.packs.fail(reason);
        OrdinaryReport
    }
}
/// One independent part of the Ordinary family, dispatched with the first
/// burst (`extended::OrdinaryPart`).
pub(crate) struct OrdinaryPartRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub fresh_modifier_guard: Result<(), &'static str>,
    pub part: crate::extended::OrdinaryPart,
}
impl OrdinaryPartRequest {
    #[cfg(test)]
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.part.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, reason: &'static str) -> OrdinaryReport {
        self.part.fail(reason);
        OrdinaryReport
    }
}
pub(crate) struct SlowRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub packs: Box<crate::extended::SlowPacks>,
    /// The Slow job's advisor and local sub-unit stores, loaned for this hour.
    pub stores: crate::extended::SlowStores,
}
/// One part of the hour's AirCountryRelations copies
/// (`air_relations::PARTS`).
pub(crate) struct AirRelationsRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub part: usize,
    pub staging: crate::extended::air_relations::Staging,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct AirRelationsReport;
/// The registered air missions and the naval-patrol combats, whose regions
/// open the history parts, then the missions' joins, which the Late job
/// keeps.
/// It needs only the wing census, never an Ordinary part's packs; the air
/// statistics and base verdicts are installed in its pack when it returns.
pub(crate) struct AirMissionsRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    /// The map census the typed visitor supplied the wing identities in.
    pub parent: engine_native::walk::Outcome,
    pub pack: Box<crate::extended::air::Captured>,
    pub histories: Vec<crate::extended::air_histories::Staging>,
    /// Held AirRegionalMissions registrations, lent by the game thread for
    /// one walk and taken back at the join; absent reads every country.
    pub mission_store: Option<Box<crate::extended::air::MissionStore>>,
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
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub part: usize,
    pub staging: crate::extended::air_histories::Staging,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct AirHistoriesReport;
#[derive(Clone, Copy, Default)]
pub(crate) struct SlowReport;
pub(crate) struct StabilityRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub packs: Box<crate::extended::StabilityPacks>,
    /// The `stability` family's fingerprint store, loaned for this hour;
    /// absent drops only the family's early reads.
    pub store: Option<Box<crate::slicing::SliceFingerprints>>,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct StabilityReport;
pub(crate) struct ProductionDailyRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub packs: Box<crate::extended::ProductionDailyPacks>,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct ProductionDailyReport;
pub(crate) struct IndustrialOrganisationsRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub captured: crate::extended::industrial_organisations::Captured,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct IndustrialOrganisationsReport {
    pub ticks: [u64; crate::extended::industrial_organisations::N],
}
/// One slot range of a split production modifier capture, beside the
/// ProductionDaily job, which captures the first.
pub(crate) struct ProductionModifierPartRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub part: usize,
    pub slots: std::ops::Range<u32>,
    pub provided: Option<Result<engine_production::production_modifiers::Context, &'static str>>,
    pub captured: Box<crate::extended::production_modifiers::Part>,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct ProductionModifierPartReport {
    pub ticks: u64,
}
pub(crate) struct WarOverviewRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub packs: Box<crate::extended::WarOverviewPacks>,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct WarOverviewReport;
pub(crate) struct RaidStateRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
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
    pub countries: Arc<engine_native::country_memory::Cache>,
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
    pub countries: Arc<engine_native::country_memory::Cache>,
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
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub packs: Box<crate::extended::LatePacks>,
    /// Completed plans and execution scratch; a Late failure never fails
    /// their independently acquired prefix or order execution scalars.
    pub plans: crate::extended::plans::Captured,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct LateReport;
#[derive(Clone, Copy, Default)]
pub(crate) struct ScoresRailsReport;
/// The regional Air supply, read from the game state alone in the first
/// burst; its capture joins the air pack at the missions join, and the Late
/// job merges it into Ground Crews.
pub(crate) struct AirSupplyRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub capture: Box<engine_map_air::map_air_supply::Capture>,
    /// The read completed; set by the job.
    pub ok: bool,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct AirSupplyReport;
/// One OrdersPart job's part of this pass's census and plan walk
/// ([`crate::orders_parts`]), returned to the census at the Orders join.
pub(crate) struct OrdersPartRequest {
    pub base: usize,
    /// The part's index, where the join returns it.
    pub index: usize,
    pub part: Box<crate::orders_parts::Part>,
    /// The pool refused the job: the part did not run.
    pub rejected: bool,
}
impl OrdersPartRequest {
    /// The reason this part fails the census with, as the single Orders job
    /// would: its admission refusal, or a run that `failed` (a panic or a
    /// cancellation); none for a part that ran.
    pub(crate) fn failure(&self, failed: bool) -> Option<&'static str> {
        if self.rejected {
            Some("army order worker admission failed")
        } else if failed {
            Some("army order acquisition failed")
        } else {
            None
        }
    }
}
/// The fronts the Orders job's plans bound, read once that job joins, beside
/// the ChildFronts job; the plans take them back after the Late join.
pub(crate) struct FrontsRequest {
    pub base: usize,
    pub fronts: crate::extended::plans::Fronts,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct FrontsReport;
/// ArmyIconInputs, right after the division sources join: the seeded census
/// carries both icon fields, so the pass reads only alternate templates.
pub(crate) struct ArmyIconsRequest {
    pub base: usize,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    pub divisions: crate::extended::Divisions,
    pub icons: crate::extended::army_icon_inputs::Captured,
    pub ticks: [u64; 1],
}
/// DivisionModelInputs in a Map recording, beside the Divisions job: the
/// seeded census carries the model number and acclimatization header.
pub(crate) struct DivisionModelInputsRequest {
    pub base: usize,
    pub game_state: usize,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    pub captured: crate::extended::division_model_inputs::Captured,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct DivisionModelInputsReport;
/// RaidAirScores and the shared rail tables in their own first-burst job:
/// both walks read only the game state and the hour, the score walk
/// resolving countries from the hour's snapshot bytes. The Late job keeps the
/// tables they leave in the frame's packs.
pub(crate) struct ScoresRailsRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    /// The air pack's held-table capacities.
    pub score_capacity: [usize; 2],
    pub parts: Box<crate::extended::ScoresRailsParts>,
    /// The attachment's held RaidAirScores table, loaned for this hour.
    pub air_scores: Box<engine_raid_inputs::raid_air_scores::Held>,
    /// Held rail outputs ([`naval_history::RailStore`]); absent reads every
    /// rail info afresh.
    pub rail_store: Option<Box<naval_history::RailStore>>,
}
/// The army order census and its plan operands, loaned to the Orders job
/// right after the root census and then, with this same held pass's
/// completed OOB copy, to the ChildFronts job, which completes the child
/// fronts' owner operands. The Late job reads the roots and plans they leave.
pub(crate) struct OrdersRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    /// This pass's game state, for the invasion preparations' countries and
    /// provinces.
    pub game_state: usize,
    pub orders: crate::map_capture::Orders,
    pub plans: crate::extended::plans::Captured,
    /// Loan of the accepted Naval copy to the ChildFronts job, returned for
    /// its ordinary decode. The Orders job never holds it. An absent copy
    /// never permits a native fallback for child group inputs.
    pub group_source: Option<naval_history::NavalJob>,
}
impl OrdersRequest {
    /// The OrdersStitch job: this pass's census and plan parts stitched into
    /// the single census and plans, then every other pass over them as the
    /// single Orders job made them; plans the next pass's split. A census a
    /// failed or refused job failed stays failed.
    fn stitch(&mut self, memory: &impl engine_native::Memory) {
        if self.orders.failed() {
            return;
        }
        let part_ticks = self.orders.stitch();
        if self.orders.failed() {
            if let engine_native::walk::Outcome::Unavailable { reason } = self.orders.rows.outcome {
                self.plans.fail(reason);
            }
            return;
        }
        let count = self.orders.part_count();
        if count != 0 && self.orders.plan_parts().is_none() {
            self.fail("army order part did not run");
            return;
        }
        // No parts: the census refused its failed roots, and so do the plans.
        let parts = self.orders.plan_parts();
        let parts = parts.as_ref().map_or(&[][..], |parts| &parts[..count]);
        let orders = &self.orders;
        self.plans.visit_parts(
            memory,
            self.base,
            self.game_state,
            orders.roots(),
            orders.visits(),
            orders.successor_lists(),
            orders.root_outcome,
            orders.rows.outcome,
            &orders.rows.rows[..orders.rows.outcome.len()],
            &orders.geometry[..orders.geometry_len],
            parts,
            part_ticks,
        );
        self.orders.plan_split();
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
/// The OrdersStitch job's report: whether its body panicked. The stitch
/// contains its own panic, so that the pool does not stop before the
/// stitch's join (`sampling::settle_stitch` applies the stop there).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StitchReport {
    pub contained_panic: bool,
}
/// A test request base that makes the stitch body panic before it reads
/// anything.
#[cfg(test)]
pub(crate) const STITCH_PANIC_BASE: usize = usize::MAX - 0x5717;
/// The raw-gated packs, dispatched once every raw-copy verdict is in.
pub(crate) struct GatedRequest {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub packs: Box<crate::extended::GatedPacks>,
    /// The projects family's only fingerprint store, loaned for this hour.
    pub project_fingerprints: Option<Box<crate::slicing::SliceFingerprints>>,
    /// The hour's authenticated country census, which the projects family's
    /// per-slot census reads instead of each country object.
    pub countries: Option<Arc<engine_native::country_memory::Cache>>,
}
pub(crate) struct GatedProjectsRequest {
    pub workers: variant_workers::Dispatch,
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
    pub captured: projects::Captured,
    pub fingerprints: Box<crate::slicing::SliceFingerprints>,
    pub countries: Option<Arc<engine_native::country_memory::Cache>>,
    pub recorded: bool,
    pub ticks: [u64; projects::N],
}
impl GatedRequest {
    pub(crate) fn take_projects(&mut self) -> GatedProjectsRequest {
        GatedProjectsRequest {
            workers: Default::default(),
            base: self.base,
            game_state: self.game_state,
            hour: self.hour,
            force_full: self.force_full,
            captured: std::mem::replace(&mut self.packs.projects, projects::Captured::vacant()),
            fingerprints: self
                .project_fingerprints
                .take()
                .expect("projects store present"),
            countries: self.countries.take(),
            recorded: self.packs.projects_recorded,
            ticks: [0; projects::N],
        }
    }
    pub(crate) fn restore_projects(&mut self, request: GatedProjectsRequest) {
        self.packs.projects = request.captured;
        self.project_fingerprints = Some(request.fingerprints);
        for (table, ticks) in projects::TABLES.into_iter().zip(request.ticks) {
            self.packs.ticks[table.index()] = ticks;
        }
    }
}
impl GatedProjectsRequest {
    fn capture(
        &mut self,
        memory: &impl engine_native::Memory,
        roster: &impl engine_native::Memory,
        changes: &dyn crate::country_input_changes::Changes,
    ) {
        let _phase = crate::allocation::enter(crate::allocation::Phase::Extended);
        if self.recorded {
            let workers = &self.workers;
            let mut acquire = |parts| workers.projects(parts, roster, self.base, self.game_state);
            self.ticks = self.captured.acquire_selected_partitioned(
                memory,
                roster,
                self.base,
                self.game_state,
                self.hour,
                self.force_full,
                &mut self.fingerprints,
                changes,
                projects::ProjectDispatch {
                    lanes: workers.lanes(),
                    acquire: &mut acquire,
                },
            );
        } else {
            self.captured.fail_gated_capture(
                self.hour,
                self.force_full,
                crate::extended::NOT_RECORDED,
            );
        }
    }
}
#[derive(Clone, Copy, Default)]
pub(crate) struct GatedReport;
pub(crate) struct ProvinceUnitOrderRequest {
    pub context: province_unit_order::CaptureContext,
    pub captured: province_unit_order::Captured,
    /// The hour's division census lease, when its acquisition completed;
    /// released at the join, before the census is reused.
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    /// The held walk ([`crate::province_unit_changes`]), lent for the hour.
    pub store: Option<Box<province_unit_order::ProvinceUnitStore>>,
}
/// Independent core-map walk moved to the pool so its pointer chase overlaps the
/// other readers. Owns its pre-sized row buffers; the callback restores them.
pub(crate) type ProvincesRequest = GatherRequest<engine_native::cprovince::Capture>;
pub(crate) type ProvincesReport = GatherReport;
pub(crate) struct ShipModelsRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub context: ship_models::CaptureContext,
    pub game_state: usize,
    pub captured: ship_models::Captured,
    /// Its publisher releases the detail parts before callback collection.
    pub census: crate::map_capture::ShipCensus,
    pub details: Option<Arc<ShipDetailsDispatch>>,
}
/// One contiguous census range of the ShipModels details (part 0 is the
/// family's own capture).
pub(crate) struct ShipModelDetailsRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub context: ship_models::CaptureContext,
    pub part: usize,
    pub captured: ship_models::Captured,
}
pub(crate) struct PresentationRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub context: division_presentation::CaptureContext,
    pub captured: division_presentation::Captured,
    pub dispatch: PresentationDispatch,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    /// The `army_stat` family's held DivisionStatus attack operands, loaned
    /// for the hour.
    pub army_stats: Option<division_presentation::held_store::Loan>,
}
impl PresentationRequest {
    /// Lend the movement pass to its own job, which runs beside this one.
    pub(crate) fn take_movement(&mut self) -> PresentationMovementRequest {
        PresentationMovementRequest {
            countries: Arc::clone(&self.countries),
            context: self.context,
            inputs: self.inputs.clone(),
            pass: self.captured.take_movement(),
        }
    }

    pub(crate) fn take_speed_terms(&mut self) -> PresentationSpeedTermsRequest {
        PresentationSpeedTermsRequest {
            countries: Arc::clone(&self.countries),
            context: self.context,
            inputs: self.inputs.clone(),
            pass: self.captured.take_speed_terms(),
        }
    }

    /// Return the speed pass before settling failure; its failure precedes
    /// movement and status, as when it ran first inside the movement job.
    pub(crate) fn restore_speed_terms(
        &mut self,
        speed_terms: PresentationSpeedTermsRequest,
        speed_report: Result<division_presentation::CaptureReport, Failure>,
        report: Result<division_presentation::CaptureReport, Failure>,
    ) -> Result<division_presentation::CaptureReport, Failure> {
        self.captured.restore_speed_terms(speed_terms.pass);
        let speed_terms = speed_report?;
        report.map(|mut report| {
            report.merge_speed_terms(speed_terms);
            report
        })
    }

    /// Return the movement pass and merge its ticks into this job's report.
    /// A failed movement job fails the whole presentation, as the single job
    /// did.
    pub(crate) fn restore_movement(
        &mut self,
        movement: PresentationMovementRequest,
        movement_report: Result<division_presentation::CaptureReport, Failure>,
        report: Result<division_presentation::CaptureReport, Failure>,
    ) -> Result<division_presentation::CaptureReport, Failure> {
        self.captured.restore_movement(movement.pass);
        let movement = movement_report?;
        report.map(|mut report| {
            report.merge_movement(movement);
            report
        })
    }
}
/// One final-status range over immutable ordered preparation.
pub(crate) struct PresentationStatusRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub context: division_presentation::CaptureContext,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    pub part: Option<division_presentation::StatusWork>,
    pub preparation: bool,
    pub stitch: Option<(Arc<PresentationFanIn>, usize)>,
}

/// The unchanged whole-census speed pass beside status and movement.
pub(crate) struct PresentationSpeedTermsRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub context: division_presentation::CaptureContext,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    pub pass: division_presentation::SpeedTermsPass,
}
impl PresentationSpeedTermsRequest {
    #[cfg(test)]
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.pass.allocated_bytes()
    }
}

/// Movement, land and dig-in beside the Presentation job.
pub(crate) struct PresentationMovementRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub context: division_presentation::CaptureContext,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    pub pass: division_presentation::MovementPass,
}
pub(crate) struct IntelRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub context: fog::IntelCopyContext,
    pub payload: fog::IntelJob,
    pub inputs: Option<Arc<crate::division_census::UnitInputs>>,
    pub input_failure: Option<&'static str>,
}
pub(crate) struct CharactersRequest {
    pub final_inputs: Option<Arc<FinalInputsDispatch>>,
    pub context: characters::CharactersCopyContext,
    pub payload: characters::CharactersJob,
    /// A role job's census loan returns with the request at its join.
    pub roles: characters::Roles<Arc<characters::census::RoleCensus>>,
}
pub(crate) type RelationsRequest = RawRequest<intel::RelationsJob, intel::RelationsCopyContext>;
pub(crate) type AirWingRequest = RawRequest<air::AirWingJob, air::AirWingCopyContext>;
pub(crate) type AirStatsRequest = RawRequest<air::AirStatsJob, air::AirStatsCopyContext>;
pub(crate) type AirBasesRequest = RawRequest<air::AirBasesJob, air::AirBasesCopyContext>;
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

/// One part of the hour's division inputs: the country slots `slots` of the
/// census the DivisionInputs job walks the first part of
/// ([`crate::division_census::Census::absorb`]).
pub(crate) struct DivisionPartRequest {
    pub countries: Arc<engine_native::country_memory::Cache>,
    pub base: usize,
    pub game_state: usize,
    pub slots: (u32, u32),
    pub census: Box<crate::division_census::Census>,
    pub presentation: Option<(Arc<PresentationInputs>, usize)>,
}

// Jobs move preallocated payloads into fixed executor slots. Boxing here would
// allocate on the hourly callback merely to reduce the enum's stack footprint.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Job {
    Characters(CharactersRequest),
    Relations(RelationsRequest),
    AirWing(AirWingRequest),
    AirStats(AirStatsRequest),
    AirBases(AirBasesRequest),
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
    /// The event feed the observer boundary prepared, sealed after the
    /// Governance job's census.
    EventFeedSeal(crate::extended::event_feed::Captured),
    FinalInputs(crate::extended::FinalInputsRequest),
    OrderRoots(OrderRootsRequest),
    DivisionInputs(DivisionsRequest),
    DivisionInputParts(DivisionPartRequest),
    Divisions(DivisionsRequest),
    World(WorldRequest),
    Economy(EconomyRequest),
    Governance(GovernanceRequest),
    Variants(VariantsRequest),
    VariantPart(variant_workers::Request),
    Research(ResearchRequest),
    Intel(IntelRequest),
    PresentationPrepare(PresentationRequest),
    PresentationSettle(PresentationRequest),
    PresentationStatus(PresentationStatusRequest),
    Presentation(PresentationRequest),
    PresentationMovement(PresentationMovementRequest),
    PresentationSpeedTerms(PresentationSpeedTermsRequest),
    IntelResidual(IntelResidualRequest),
    SupplyTrains(SupplyTrainsRequest),
    CharacterPortraits(CharacterPortraitsRequest),
    ProvinceUnitOrder(ProvinceUnitOrderRequest),
    Ordinary(OrdinaryRequest),
    OrdinaryPart(OrdinaryPartRequest),
    Slow(SlowRequest),
    Stability(StabilityRequest),
    WarOverview(WarOverviewRequest),
    IndustrialOrganisations(IndustrialOrganisationsRequest),
    ProductionDaily(ProductionDailyRequest),
    ProductionModifierPart(ProductionModifierPartRequest),
    RaidState(RaidStateRequest),
    RaidLaunch(RaidLaunchRequest),
    RaidTargets(RaidTargetsRequest),
    AirRelations(AirRelationsRequest),
    AirMissions(AirMissionsRequest),
    AirHistories(AirHistoriesRequest),
    Late(LateRequest),
    ScoresRails(ScoresRailsRequest),
    AirSupply(AirSupplyRequest),
    ArmyIcons(ArmyIconsRequest),
    DivisionModelInputs(DivisionModelInputsRequest),
    /// The Orders job: part 0 of this pass's census and plan walk.
    Orders(OrdersRequest),
    /// Another part of this pass's census and plan walk.
    OrdersPart(OrdersPartRequest),
    /// The parts stitched, and every other pass over the census.
    OrdersStitch(OrdersRequest),
    /// The fronts the Orders job's plans bound.
    Fronts(FrontsRequest),
    /// The Orders job's request with the Naval copy: the child fronts'
    /// owner operands.
    ChildFronts(OrdersRequest),
    Gated(GatedRequest),
    GatedProjects(GatedProjectsRequest),
    CharacterInputs(crate::extended::CharacterInputsRequest),
    ShipModels(ShipModelsRequest),
    ShipModelDetails(ShipModelDetailsRequest),
    Provinces(ProvincesRequest),
    Factions(acquisition_factions::Request),
    Countries(CountriesRequest),
    Wars(acquisition_wars::Request),
    States(acquisition_states::Request),
    Templates(TemplatesRequest),
}
#[derive(Clone, Copy)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Report {
    Characters(characters::CharactersCopyReport),
    Relations(intel::RelationsCopyReport),
    AirWing(air::AirWingCopyReport),
    AirStats(air::AirStatsCopyReport),
    AirBases(air::AirBasesCopyReport),
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
    /// The seal's outcome is in the returned capture.
    EventFeedSeal,
    FinalInputs(crate::extended::FinalInputsReport),
    OrderRoots(u64),
    DivisionInputs(bool),
    DivisionInputParts(u64),
    Divisions(sampling::DivisionsCopyReport),
    World(WorldReport),
    Economy(economy::EconomyRestCopyReport),
    Governance(GovernanceReport),
    Variants([u64; production::N]),
    VariantPart,
    Research(ResearchReport),
    Intel(fog::IntelCopyReport),
    PresentationPrepare(()),
    PresentationStatus(u64),
    Presentation(division_presentation::CaptureReport),
    /// Movement, land and dig-in ticks in presentation table order.
    PresentationMovement(division_presentation::CaptureReport),
    /// Shared speed ticks in presentation table order.
    PresentationSpeedTerms(division_presentation::CaptureReport),
    IntelResidual(intel::CaptureReport),
    SupplyTrains(supply_trains::CaptureReport),
    CharacterPortraits(character_portraits::CaptureReport),
    ProvinceUnitOrder(province_unit_order::CaptureReport),
    Ordinary(OrdinaryReport),
    OrdinaryPart(OrdinaryReport),
    Slow(SlowReport),
    Stability(StabilityReport),
    WarOverview(WarOverviewReport),
    IndustrialOrganisations(IndustrialOrganisationsReport),
    ProductionDaily(ProductionDailyReport),
    ProductionModifierPart(ProductionModifierPartReport),
    RaidState(RaidStateReport),
    RaidLaunch(RaidLaunchReport),
    RaidTargets(RaidTargetsReport),
    AirRelations(AirRelationsReport),
    AirMissions(AirMissionsReport),
    AirHistories(AirHistoriesReport),
    Late(LateReport),
    ScoresRails(ScoresRailsReport),
    AirSupply(AirSupplyReport),
    ArmyIcons(ScoresRailsReport),
    DivisionModelInputs(DivisionModelInputsReport),
    Orders(OrdersReport),
    OrdersPart(OrdersReport),
    OrdersStitch(StitchReport),
    Fronts(FrontsReport),
    ChildFronts(OrdersReport),
    Gated(GatedReport),
    GatedProjects(()),
    /// Per-table worker ticks of the character inputs.
    CharacterInputs([u64; crate::extended::character_inputs::N]),
    /// The census is in the request; the details report per part.
    ShipModels,
    ShipModelDetails(ship_models::CaptureReport),
    Provinces(ProvincesReport),
    Factions(acquisition_factions::Report),
    Countries(CountriesReport),
    Wars(acquisition_wars::Report),
    States(acquisition_states::Report),
    Templates(TemplatesReport),
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
    DivisionInputParts,
    World,
    Economy,
    Governance,
    Variants,
    Research,
    Intel,
    Presentation,
    FinalInputs,
    IntelResidual,
    SupplyTrains,
    CharacterPortraits,
    ProvinceUnitOrder,
    Ordinary,
    OrdinaryCombat,
    OrdinaryNaval,
    OrdinaryMap,
    OrdinaryTaskForces,
    OrdinaryNavies,
    Slow,
    Stability,
    WarOverview,
    IndustrialOrganisations,
    ProductionDaily,
    ProductionModifierParts,
    RaidState,
    RaidLaunch,
    RaidTargets,
    AirRelations,
    AirMissions,
    AirHistories,
    Late,
    ScoresRails,
    ArmyIcons,
    Orders,
    OrdersPart,
    OrdersStitch,
    ChildFronts,
    Fronts,
    Gated,
    ShipModels,
    ShipModelDetails,
    Provinces,
    Factions,
    Countries,
    Wars,
    States,
    Templates,
    CharacterInputs,
    PresentationMovement,
    /// The division input census's first part, apart from the Divisions
    /// core job it leads to, so their joins report apart.
    DivisionInputs,
    /// The orders-group root census, apart from the Orders job it leads to,
    /// so their joins report apart.
    OrderRoots,
    PresentationSpeedTerms,
    PresentationPrepare,
    PresentationStatus,
    PresentationPreparation,
    PresentationSettle,
    DivisionModelInputs,
    GatedProjects,
    AirSupply,
}

impl Role {
    pub(crate) const fn worker_only(self) -> bool {
        true
    }
    // Only admitted leaf captures; coordinators can wait for children or retain loans.
    const fn callback_help(self) -> bool {
        matches!(
            self,
            Role::Characters
                | Role::CountryLeaderRoles
                | Role::AdvisorRoles
                | Role::CharacterTraits
                | Role::Relations
                | Role::AirWing
                | Role::AirStats
                | Role::AirBases
                | Role::Capitulation
                | Role::Players
                | Role::Projects
                | Role::Lexical
                | Role::Session
                | Role::Weather
                | Role::Radar
                | Role::OrderRoots
                | Role::DivisionInputs
                | Role::DivisionInputParts
                | Role::Divisions
                | Role::PresentationPreparation
                | Role::PresentationStatus
                | Role::World
                | Role::Economy
                | Role::Research
                | Role::CharacterPortraits
                | Role::ProvinceUnitOrder
                | Role::AirRelations
                | Role::AirHistories
                | Role::ArmyIcons
                | Role::Orders
                | Role::OrdersPart
                | Role::Fronts
                | Role::ShipModels
                | Role::Countries
                | Role::Wars
                | Role::States
                | Role::AirSupply
        )
    }

    /// Shared native work follows an hourly consumer in mixed families. Detail
    /// acquisition opens nested scopes when it reads its own sources.
    fn read_anchor(self) -> replay_data::rows::TableId {
        use replay_data::rows::TableId as T;
        match self {
            Self::Characters => T::Characters,
            Self::CountryLeaderRoles => T::CountryLeaderRoles,
            Self::AdvisorRoles => T::AdvisorRoles,
            Self::CharacterTraits => T::UnitLeaderTraits,
            Self::Relations => T::RelationTuples,
            Self::AirWing => T::AirWings,
            Self::AirStats => T::AirStatistics,
            Self::AirBases => T::AirBases,
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
            Self::Divisions | Self::DivisionInputs | Self::DivisionInputParts => T::Divisions,
            // Every part opens its own tables' scopes; reads outside them are
            // charged to ProvinceSupply in both jobs.
            Self::World | Self::Economy => T::ProvinceSupply,
            Self::Governance => T::Politics,
            Self::Variants => T::VariantDefinitions,
            Self::Research => T::ResearchInputs,
            Self::Intel => T::UnitIntel,
            Self::Presentation
            | Self::PresentationPrepare
            | Self::PresentationSettle
            | Self::PresentationStatus
            | Self::PresentationPreparation
            | Self::FinalInputs => T::DivisionStatus,
            Self::IntelResidual => intel::TABLES[0],
            Self::SupplyTrains => supply_trains::TABLES[0],
            Self::CharacterPortraits => character_portraits::TABLES[0],
            Self::ProvinceUnitOrder => province_unit_order::TABLES[0],
            // Every Ordinary part keeps the family's anchor.
            Self::Ordinary
            | Self::OrdinaryCombat
            | Self::OrdinaryNaval
            | Self::OrdinaryMap
            | Self::OrdinaryTaskForces
            | Self::OrdinaryNavies => T::LandCombatDetails,
            Self::Slow => T::ProvinceNames,
            Self::Stability => T::CountryStabilityInputs,
            Self::WarOverview => T::WarOwnedRelations,
            Self::IndustrialOrganisations => T::IndustrialOrganisations,
            Self::ProductionDaily | Self::ProductionModifierParts => T::CountryProductionModifiers,
            Self::RaidState => T::StateResourceInputs,
            Self::RaidLaunch => T::RaidCountryInputs,
            Self::RaidTargets => T::RaidTargetStatuses,
            Self::AirRelations => T::AirCountryRelations,
            Self::AirMissions => T::AirRegionalMissions,
            Self::AirHistories => T::AirNavalPatrolHistories,
            Self::Late => T::WingEquipment,
            Self::ScoresRails => T::RaidAirScores,
            Self::AirSupply => T::AirGroundCrews,
            Self::ArmyIcons => T::ArmyIconInputs,
            Self::Orders | Self::OrderRoots => T::ArmyOrders,
            Self::OrdersPart => T::ArmyOrders,
            Self::OrdersStitch => T::ArmyOrderPlans,
            Self::ChildFronts => T::ArmyOrderPlans,
            Self::Fronts => T::Fronts,
            Self::Gated => T::VolunteerHosts,
            Self::GatedProjects => T::Projects,
            Self::ShipModels => T::Ships,
            Self::ShipModelDetails => T::ShipModels,
            Self::Provinces => T::Provinces,
            Self::Factions => T::Factions,
            Self::Countries => T::Countries,
            Self::Wars => T::WarRelations,
            Self::States => T::States,
            Self::Templates => T::DivisionTemplateIcons,
            Self::CharacterInputs => T::CharacterRoleTraits,
            Self::PresentationMovement => T::DivisionMovement,
            Self::PresentationSpeedTerms => T::MovementStateTerms,
            Self::DivisionModelInputs => T::DivisionModelInputs,
        }
    }
}

impl Role {
    /// Claim order when workers are scarce, lower first and FIFO among equals.
    /// The presentation chain, Variants and RaidState claim first, ahead of
    /// the saturated census tier, so their successors and independent bodies
    /// do not become the assembly tail.
    /// Jobs another job waits for claim next, so their dependency chains
    /// open at once: the observer boundaries, Countries (the Ordinary
    /// counter part), the order censuses, wings, the Naval copy, child fronts
    /// and missions (Late), the character roster (portraits, roles and
    /// character inputs), the unit-input copy (Presentation, Divisions,
    /// Intel), the ShipModels census (its detail parts), the raw-copy verdicts
    /// the Gated job reads (radar, intel, registrations, projects) and the
    /// joins before the map sample, which the Late job follows (players,
    /// combats, the air statistics, naval supremacy and sunk ships), and the
    /// World and Economy jobs, which the game thread joins right after the
    /// first segment's joins (1.0/1.3 ms at 1936/1943 as one job, 0.27/0.29
    /// ms of join wait at int_12a1f26; the economy rest, CountryResources
    /// alone 0.62 ms at 1936, runs beside the production and construction
    /// rests). The longest independent jobs follow, so
    /// the workers finish together (the Provinces job among them: 2.2/2.5
    /// ms, which as a medium job waited 0.50 ms at its join at 1936); the
    /// medium and short jobs fill in behind them, the capitulation copy,
    /// joined with the core jobs, before the rest of the short ones. The
    /// first-burst jobs with the most slack (the Ordinary task-force part,
    /// Provinces, States, Weather, Capitulation, Wars and Relations) are
    /// queued later instead, in the slack burst behind the Naval copy, since
    /// priority orders only queued jobs and cannot take a worker back from
    /// one already running.
    pub(crate) fn priority(self) -> u8 {
        match self {
            Self::Variants
            | Self::RaidState
            | Self::PresentationPrepare
            | Self::PresentationPreparation
            | Self::PresentationSettle
            | Self::PresentationStatus
            | Self::Presentation => 0,
            Self::ObserverBoundary
            | Self::Countries
            | Self::Orders
            | Self::OrderRoots
            | Self::OrdersPart
            | Self::OrdersStitch
            | Self::ChildFronts
            | Self::AirWing
            | Self::Naval
            | Self::AirMissions
            | Self::Late
            | Self::Characters
            | Self::Divisions
            | Self::DivisionInputs
            | Self::DivisionInputParts
            | Self::ShipModels
            | Self::AirBases
            | Self::Radar
            | Self::Projects
            | Self::Intel
            | Self::Players
            | Self::MapCombats
            | Self::AirStats
            | Self::Supremacy
            | Self::Sunk
            | Self::World
            | Self::Economy
            | Self::PresentationSpeedTerms
            | Self::FinalInputs
            // Its join gates the Late dispatch, and through it FinalInputs, which ends the
            // hour: at priority 3 it started 9.3 ms in and its join held Late to 10.7 ms.
            | Self::ScoresRails
            // Its join gates the missions join beside ScoresRails.
            | Self::AirSupply
            // FinalInputs' sources: at priority 2 the residual started 7 ms in, after its
            // queueing at 0.8 ms, and ended 9 ms in.
            | Self::IntelResidual
            // The hourly template census is a long independent join tail.
            | Self::Templates
            // The longest first-burst job, 2.55 ms at 1943 with the war
            // overview: at priority 1 it started behind the dependency chains
            // about 2.8 ms into the hour and waited 0.47 ms at its join
            // (int_761c274). The stability tables (0.89/0.45 ms at 1943/1936)
            // and the war overview (0.76 ms at 1943) split from it claim
            // beside it.
            | Self::Slow
            | Self::Stability
            | Self::WarOverview
            | Self::IndustrialOrganisations
            | Self::ProductionDaily
            | Self::ProductionModifierParts => 1,
            Self::Governance
            | Self::Gated
            | Self::GatedProjects
            | Self::CharacterInputs
            // A FinalInputs producer, without delaying queued Presentation work.
            | Self::AdvisorRoles
            | Self::PresentationMovement
            | Self::Ordinary
            | Self::OrdinaryNavies
            | Self::OrdinaryCombat
            // As long as Slow at 1943 (2.55 ms, 0.14 ms of join wait at
            // priority 2, int_761c274).
            | Self::OrdinaryNaval
            | Self::Provinces
            | Self::Research
            | Self::ShipModelDetails
            // Map's model witness, joined with the late joins.
            | Self::DivisionModelInputs => 2,
            Self::States
            | Self::Capitulation
            | Self::Wars
            | Self::OrdinaryTaskForces
            | Self::OrdinaryMap
            | Self::ProvinceUnitOrder
            | Self::CharacterPortraits
            | Self::CountryLeaderRoles
            | Self::CharacterTraits
            | Self::SupplyTrains
            | Self::ArmyIcons
            // Joined after the Late join, which it runs beside.
            | Self::Fronts
            | Self::Weather => 3,
            // FinalInputs' other sources, short, ahead of the rest of the tail.
            Self::Session | Self::Relations => 2,
            Self::Factions
            | Self::Lexical
            | Self::RaidLaunch
            | Self::RaidTargets
            | Self::AirRelations
            | Self::AirHistories => 4,
        }
    }
}

pub(crate) const ROLE_COUNT: usize = 81;
pub(crate) const ROLES: [Role; ROLE_COUNT] = [
    Role::Characters,
    Role::CountryLeaderRoles,
    Role::AdvisorRoles,
    Role::CharacterTraits,
    Role::Relations,
    Role::AirWing,
    Role::AirStats,
    Role::AirBases,
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
    Role::DivisionInputParts,
    Role::World,
    Role::Economy,
    Role::Governance,
    Role::Variants,
    Role::Research,
    Role::Intel,
    Role::Presentation,
    Role::FinalInputs,
    Role::IntelResidual,
    Role::SupplyTrains,
    Role::CharacterPortraits,
    Role::ProvinceUnitOrder,
    Role::Ordinary,
    Role::OrdinaryCombat,
    Role::OrdinaryNaval,
    Role::OrdinaryMap,
    Role::OrdinaryTaskForces,
    Role::OrdinaryNavies,
    Role::Slow,
    Role::Stability,
    Role::WarOverview,
    Role::IndustrialOrganisations,
    Role::ProductionDaily,
    Role::ProductionModifierParts,
    Role::RaidState,
    Role::RaidLaunch,
    Role::RaidTargets,
    Role::AirRelations,
    Role::AirMissions,
    Role::AirHistories,
    Role::Late,
    Role::ScoresRails,
    Role::ArmyIcons,
    Role::Orders,
    Role::OrdersPart,
    Role::OrdersStitch,
    Role::ChildFronts,
    Role::Fronts,
    Role::Gated,
    Role::ShipModels,
    Role::ShipModelDetails,
    Role::Provinces,
    Role::Factions,
    Role::Countries,
    Role::Wars,
    Role::States,
    Role::Templates,
    Role::CharacterInputs,
    Role::PresentationMovement,
    Role::DivisionInputs,
    Role::OrderRoots,
    Role::PresentationSpeedTerms,
    Role::PresentationPrepare,
    Role::PresentationStatus,
    Role::PresentationPreparation,
    Role::PresentationSettle,
    Role::DivisionModelInputs,
    Role::GatedProjects,
    Role::AirSupply,
];
impl Job {
    fn callback_help(&self) -> bool {
        self.role().callback_help()
    }

    /// Whether this job must run on a worker: its role's rule, except a
    /// ChildFronts pass that allocates nothing, which a joining thread may
    /// run while it is still queued.
    pub(crate) fn worker_only(&self) -> bool {
        match self {
            Self::ChildFronts(request) => request.plans.child_fronts_allocate(),
            job => job.role().worker_only(),
        }
    }
    pub(crate) fn diagnostic_identity(&self) -> crate::acquisition_diagnostics::Identity {
        let mut identity = crate::acquisition_diagnostics::Identity {
            role: self.role() as usize,
            ..Default::default()
        };
        identity.part = match self {
            Self::Characters(request) => Some(request.payload.part),
            Self::AirWing(request) => {
                identity.partition = Some((request.payload.slots.start, request.payload.slots.end));
                None
            }
            Self::SupplyTrains(request) => Some(request.part),
            Self::CharacterPortraits(request) => Some(request.part),
            Self::AirRelations(request) => Some(request.part),
            Self::AirHistories(request) => Some(request.part),
            Self::ShipModelDetails(request) => Some(request.part),
            Self::ProductionModifierPart(request) => Some(request.part),
            Self::OrdersPart(request) => Some(request.index),
            Self::Orders(_) | Self::DivisionInputs(_) => Some(0),
            Self::DivisionInputParts(request) => {
                identity.partition = Some(request.slots);
                request.presentation.as_ref().map(|(_, index)| *index)
            }
            Self::PresentationStatus(request) => request.stitch.as_ref().map(|(_, index)| *index),
            Self::VariantPart(request) => Some(request.index),
            _ => None,
        };
        match self {
            Self::VariantPart(request) => identity.kind = request.part.kind(),
            Self::EventFeedSeal(_) => identity.kind = 3,
            Self::ProductionModifierPart(request) => {
                identity.partition = Some((request.slots.start, request.slots.end))
            }
            _ => {}
        }
        identity
    }
    pub(crate) fn priority(&self) -> u8 {
        let cold_tail = match self {
            Self::Governance(request) => request.context.force_full,
            Self::GatedProjects(request) => request.force_full && request.recorded,
            _ => false,
        };
        if cold_tail { 0 } else { self.role().priority() }
    }

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
            // The seal finishes the boundary's event feed and is charged with it.
            Self::ObserverBoundary(..) | Self::EventFeedSeal(..) => Role::ObserverBoundary,
            Self::DivisionInputs(..) => Role::DivisionInputs,
            Self::Divisions(..) => Role::Divisions,
            Self::DivisionInputParts(..) => Role::DivisionInputParts,
            Self::World(..) => Role::World,
            Self::Economy(..) => Role::Economy,
            Self::Governance(..) => Role::Governance,
            Self::Variants(..) => Role::Variants,
            Self::VariantPart(request) => request.part.role(),
            Self::Research(..) => Role::Research,
            Self::Intel(..) => Role::Intel,
            Self::FinalInputs(..) => Role::FinalInputs,
            Self::PresentationPrepare(..) => Role::PresentationPrepare,
            Self::PresentationSettle(..) => Role::PresentationSettle,
            Self::PresentationStatus(request) if request.preparation => {
                Role::PresentationPreparation
            }
            Self::PresentationStatus(..) => Role::PresentationStatus,
            Self::Presentation(..) => Role::Presentation,
            Self::PresentationMovement(..) => Role::PresentationMovement,
            Self::PresentationSpeedTerms(..) => Role::PresentationSpeedTerms,
            Self::IntelResidual(..) => Role::IntelResidual,
            Self::SupplyTrains(..) => Role::SupplyTrains,
            Self::CharacterPortraits(..) => Role::CharacterPortraits,
            Self::ProvinceUnitOrder(..) => Role::ProvinceUnitOrder,
            Self::Ordinary(..) => Role::Ordinary,
            Self::OrdinaryPart(request) => match request.part {
                crate::extended::OrdinaryPart::Combat(_) => Role::OrdinaryCombat,
                crate::extended::OrdinaryPart::Naval(_) => Role::OrdinaryNaval,
                crate::extended::OrdinaryPart::Map(_) => Role::OrdinaryMap,
                crate::extended::OrdinaryPart::TaskForces(_) => Role::OrdinaryTaskForces,
                crate::extended::OrdinaryPart::Navies(_) => Role::OrdinaryNavies,
            },
            Self::Slow(..) => Role::Slow,
            Self::Stability(..) => Role::Stability,
            Self::WarOverview(..) => Role::WarOverview,
            Self::IndustrialOrganisations(..) => Role::IndustrialOrganisations,
            Self::ProductionDaily(..) => Role::ProductionDaily,
            Self::ProductionModifierPart(..) => Role::ProductionModifierParts,
            Self::RaidState(..) => Role::RaidState,
            Self::RaidLaunch(..) => Role::RaidLaunch,
            Self::RaidTargets(..) => Role::RaidTargets,
            Self::AirRelations(..) => Role::AirRelations,
            Self::AirMissions(..) => Role::AirMissions,
            Self::AirHistories(..) => Role::AirHistories,
            Self::Late(..) => Role::Late,
            Self::ScoresRails(..) => Role::ScoresRails,
            Self::AirSupply(..) => Role::AirSupply,
            Self::ArmyIcons(..) => Role::ArmyIcons,
            Self::DivisionModelInputs(..) => Role::DivisionModelInputs,
            Self::OrderRoots(..) => Role::OrderRoots,
            Self::Orders(..) => Role::Orders,
            Self::OrdersPart(..) => Role::OrdersPart,
            Self::OrdersStitch(..) => Role::OrdersStitch,
            Self::ChildFronts(..) => Role::ChildFronts,
            Self::Fronts(..) => Role::Fronts,
            Self::Gated(..) => Role::Gated,
            Self::GatedProjects(..) => Role::GatedProjects,
            Self::CharacterInputs(..) => Role::CharacterInputs,
            Self::ShipModels(..) => Role::ShipModels,
            Self::ShipModelDetails(..) => Role::ShipModelDetails,
            Self::Provinces(..) => Role::Provinces,
            Self::Factions(..) => Role::Factions,
            Self::Countries(..) => Role::Countries,
            Self::Wars(..) => Role::Wars,
            Self::States(..) => Role::States,
            Self::Templates(..) => Role::Templates,
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
    pub presentation_continuations: [presentation::ContinuationCalibration; 4],
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
    fn observe(&mut self, seed: engine_native::country_memory::SeedStatus, entries: usize) {
        self.latest = Some(match seed {
            engine_native::country_memory::SeedStatus::Ready => {
                self.ready = self.ready.saturating_add(1);
                CountryCacheSeed::Ready { entries }
            }
            engine_native::country_memory::SeedStatus::Capacity { required_cells } => {
                self.capacity_fallbacks = self.capacity_fallbacks.saturating_add(1);
                CountryCacheSeed::Capacity { required_cells }
            }
            engine_native::country_memory::SeedStatus::Unavailable { reason } => {
                self.unavailable_fallbacks = self.unavailable_fallbacks.saturating_add(1);
                CountryCacheSeed::Unavailable { reason }
            }
        });
    }
}

/// The World and Economy jobs together: both add their worker time to
/// `acquisition_ticks` and report their failures in `last_error`; the Economy
/// job, which runs every hour, observes the country cache.
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

fn account_completion(calibration: &SharedCalibration, completed: &Completion<Job, Report>) {
    crate::read_accounting::merge(&completed.table_reads);
    crate::attach_diagnostics::completed(completed);
    let role = completed.request.role();
    note_completion(&calibration.roles[role as usize], &completed.report);
    if matches!(role, Role::World | Role::Economy)
        && let Err(failure) = completed.report
    {
        calibration
            .world
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .last_error = Some(match (role, failure) {
            (Role::Economy, Failure::Panicked) => "Economy acquisition job panicked",
            (Role::Economy, Failure::Stopped) => "Economy acquisition job stopped before reading",
            (_, Failure::Panicked) => "World acquisition job panicked",
            (_, Failure::Stopped) => "World acquisition job stopped before reading",
        });
    }
}

/// Contain a teardown step's panic without consulting the terminal
/// allocation state: draining a ticket must run even after a terminal
/// failure, which only stops new work ([`Batch`]'s drop).
fn contain(work: impl FnOnce()) {
    let _phase = crate::allocation::enter(crate::allocation::Phase::Acquisition);
    if let Err(payload) = catch_unwind(AssertUnwindSafe(work)) {
        // A panic payload can itself panic when dropped. Preserve the
        // callback's drainage barrier by never running that destructor.
        std::mem::forget(payload);
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
    cold_parents: std::sync::atomic::AtomicUsize,
    roles: [Counters; ROLE_COUNT],
    world: Mutex<WorldCalibration>,
    presentation_continuations: [presentation::ContinuationCounters; 4],
}
impl Default for SharedCalibration {
    fn default() -> Self {
        Self {
            cold_parents: Default::default(),
            roles: std::array::from_fn(|_| Counters::default()),
            world: Mutex::new(WorldCalibration::default()),
            presentation_continuations: Default::default(),
        }
    }
}
pub(crate) fn run_world(
    request: &mut WorldRequest,
    memory: &impl engine_native::Memory,
    production_memory: &impl engine_native::Memory,
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
    WorldReport {
        construction,
        production,
        ticks: crate::sampling::qpc().saturating_sub(started).max(0) as u64,
    }
}

/// The World job's worker time joins the family's `acquisition_ticks`; the
/// country cache is observed once an hour, by the Economy job.
fn run_world_shared(
    request: &mut WorldRequest,
    memory: &impl engine_native::Memory,
    calibration: &Mutex<WorldCalibration>,
) -> WorldReport {
    let started = crate::sampling::qpc();
    let snapshot = Arc::clone(&request.countries);
    let cached = snapshot.view(memory);
    let mut report = run_world(request, &cached, memory);
    // The callback owns preparation and retains the country lease through all jobs.
    report.ticks = crate::sampling::qpc().saturating_sub(started).max(0) as u64;
    let mut counters = calibration
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    counters.acquisition_ticks = counters.acquisition_ticks.saturating_add(report.ticks);
    report
}

/// The economy rest through the shared country cache, which it observes for
/// the family's calibration: it runs every hour, so the cache is observed
/// once an hour.
fn run_economy_shared(
    request: &mut EconomyRequest,
    memory: &impl engine_native::Memory,
    calibration: &Mutex<WorldCalibration>,
) -> economy::EconomyRestCopyReport {
    let started = crate::sampling::qpc();
    let cached = request.countries.view(memory);
    let seed = cached.seed_status();
    let entries = cached.cached_entries();
    let report =
        economy::copy_economy_rest_refresh_growing(&cached, request.context, &mut request.payload);
    let ticks = crate::sampling::qpc().saturating_sub(started).max(0) as u64;
    let mut counters = calibration
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    counters.country_cache.observe(seed, entries);
    counters.acquisition_ticks = counters.acquisition_ticks.saturating_add(ticks);
    report
}

#[cfg(test)]
pub(crate) fn run_for_test(job: &mut Job, memory: &impl engine_native::Memory) -> Report {
    if let Job::ObserverBoundary(request) = job {
        return Report::ObserverBoundary(request.capture_with_memory(memory));
    }
    run_with_memory(job, memory, &SharedCalibration::default())
}

fn run_with_memory(
    job: &mut Job,
    memory: &impl engine_native::Memory,
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
        Job::AirBases(request) => {
            // The census takes the hour's authenticated roster instead of its
            // spans, and the snapshot serves any country bytes a walk reads.
            // Every other span stays native.
            let cached = request.countries.view(memory);
            Report::AirBases(air::copy_air_bases_refresh_growing(
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
        Job::EventFeedSeal(feed) => {
            feed.seal_growing();
            Report::EventFeedSeal
        }
        Job::FinalInputs(request) => {
            let countries = Arc::clone(&request.countries);
            let cached = countries.roster_view(memory);
            Report::FinalInputs(request.capture(&cached))
        }
        Job::OrderRoots(request) => {
            request.orders.walk_roots_growing(memory, request.base);
            let report = request
                .naval
                .capture_roots_growing(request.orders.roots(), request.orders.root_outcome);
            // This pass's census and plan parts, with the roots they walk.
            request.orders.prepare_parts();
            Report::OrderRoots(report)
        }
        Job::DivisionInputs(request) => {
            let cached = request.countries.view(memory);
            Report::DivisionInputs(sampling::copy_divisions_inputs_refresh_growing(
                &cached,
                request.context,
                &mut request.payload,
            ))
        }
        Job::DivisionInputParts(request) => {
            let cached = request.countries.view(memory);
            let _reads = crate::read_accounting::table(replay_data::rows::TableId::Divisions);
            let begin = sampling::qpc();
            request.census.capture_inputs_part(
                &cached,
                request.base,
                request.game_state,
                request.slots,
            );
            Report::DivisionInputParts(sampling::qpc().saturating_sub(begin).max(0) as u64)
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
        Job::Economy(request) => {
            Report::Economy(run_economy_shared(request, memory, &calibration.world))
        }
        Job::Governance(request) => {
            request
                .captured
                .profile_acquisition(crate::attach_diagnostics::measuring());
            let cached = request.countries.roster_view(memory);
            let context = request.context;
            let workers = &request.workers;
            let mut acquire = |parts| workers.technologies(parts, &cached, context.base);
            let governance = request.captured.acquire_owned_partitioned(
                &cached,
                context,
                &mut request.technology_fingerprints,
                &mut request.focus_fingerprints,
                request.leader_store.as_deref_mut(),
                request.party_store.as_deref_mut(),
                governance::TechnologyDispatch {
                    lanes: workers.lanes(),
                    acquire: &mut acquire,
                },
            );
            let buildings = request.buildings.capture_carried(
                &cached,
                context.base,
                context.game_state,
                context.hour,
                context.force_full,
                request.building_store.as_deref_mut(),
            );
            Report::Governance(GovernanceReport {
                governance,
                buildings,
            })
        }
        Job::Variants(request) => {
            use production::design_changes::profile::{Job as ProfileJob, Phase, Timer};
            let context = request.context;
            let _profile = ProfileJob::begin(context.hour);
            // Repair before deciding recovery: the drain may withdraw the census.
            let begin = sampling::qpc();
            crate::read_accounting::with_table(replay_data::rows::TableId::VariantChanges, || {
                request.changes.capture_partitioned(
                    context.base,
                    context.hour,
                    request.workers.lanes(),
                    |parts| request.workers.context(parts, memory, context.base),
                    |parts| request.workers.snapshots(parts, memory, context.base),
                );
            });
            request.changes_ticks = sampling::qpc().saturating_sub(begin).max(0) as u64;
            request.changes_finished = true;
            // The campaign registry includes variants absent from units,
            // production and stockpiles. Keep all entries and duplicates.
            let begin = sampling::qpc();
            let registry_phase = Timer::new(Phase::Registry);
            let (acquire_roster, fresh) = variant_read_plan(
                context.hour,
                context.force_full,
                production::design_changes::needs_full(),
            );
            if acquire_roster {
                request
                    .registry
                    .acquire(memory, context.base, context.game_state);
            } else {
                request.registry.mark_skipped();
            }
            let registry_ticks = sampling::qpc().saturating_sub(begin).max(0) as u64;
            drop(registry_phase);
            let _variants_phase = Timer::new(Phase::Variants);
            let workers = &request.workers;
            let mut variants = request.variants.capture_deferred_overlapped(
                memory,
                context.base,
                request.registry.sources(),
                context.hour,
                fresh,
                workers.lanes(),
                |parts| workers.acquire(parts, memory, context.base),
                |parts| workers.context(parts, memory, context.base),
                |parts| workers.setup(parts, memory, context.base),
                Some(|part, run: &mut dyn FnMut()| {
                    workers.storage_during(part, memory, context.base, run)
                }),
            );
            variants[3] = variants[3].saturating_add(registry_ticks);
            Report::Variants(variants)
        }
        Job::VariantPart(request) => {
            request.part.capture(memory, request.base);
            Report::VariantPart
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
            Report::IntelResidual(request.captured.capture_residual(
                &cached,
                request.context,
                request.flags.as_deref_mut(),
                request.variables.as_deref_mut(),
            ))
        }
        Job::SupplyTrains(request) => {
            let cached = request.countries.roster_view(memory);
            Report::SupplyTrains(supply_trains::capture_job(
                &cached,
                request.context,
                request.part,
                &mut request.nodes,
                request.trains.as_mut(),
                request.slice.as_mut(),
            ))
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
            Report::ProvinceUnitOrder(request.captured.capture_owned(
                memory,
                request.context,
                request.inputs.as_deref(),
                request.store.as_deref_mut(),
            ))
        }
        Job::Ordinary(request) => {
            let cached = request.countries.roster_view(memory);
            request.packs.capture_at_carried(
                &cached,
                request.base,
                request.game_state,
                request.hour,
                request.counters.as_deref_mut(),
            );
            Report::Ordinary(OrdinaryReport)
        }
        Job::OrdinaryPart(request) => {
            let cached = request.countries.roster_view(memory);
            request.part.capture(
                &cached,
                request.base,
                request.game_state,
                request.hour,
                request.fresh_modifier_guard,
            );
            Report::OrdinaryPart(OrdinaryReport)
        }
        Job::Slow(request) => {
            let cached = request.countries.roster_view(memory);
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
        Job::Stability(request) => {
            let cached = request.countries.roster_view(memory);
            request.packs.capture(
                &cached,
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
                request.store.as_deref_mut(),
            );
            Report::Stability(StabilityReport)
        }
        Job::IndustrialOrganisations(request) => {
            let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
            let cached = request.countries.roster_view(memory);
            let ticks = request.captured.capture(
                &cached,
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
            );
            Report::IndustrialOrganisations(IndustrialOrganisationsReport { ticks })
        }
        Job::ProductionModifierPart(request) => {
            let cached = request.countries.roster_view(memory);
            let ticks = request.captured.capture(
                &cached,
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
                request.slots.clone(),
                request.provided,
            );
            Report::ProductionModifierPart(ProductionModifierPartReport { ticks: ticks[0] })
        }
        Job::ProductionDaily(request) => {
            let cached = request.countries.roster_view(memory);
            request.packs.capture(
                &cached,
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
            );
            Report::ProductionDaily(ProductionDailyReport)
        }
        Job::WarOverview(request) => {
            let cached = request.countries.roster_view(memory);
            request.packs.capture(
                &cached,
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
            );
            Report::WarOverview(WarOverviewReport)
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
                    carry: request
                        .mission_store
                        .as_deref_mut()
                        .map(|store| air::MissionCarry {
                            store,
                            changes: &crate::mission_changes::Observer,
                        }),
                },
            );
            // The joins resolve each country directly, as on the game, from
            // the hour's snapshot bytes; every other read stays native.
            request.pack.join_missions(
                &request.countries.bytes_view(memory),
                request.base,
                request.game_state,
                request.mission_store.as_deref_mut(),
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
                None,
                None,
                None,
            );
            let cost = request.plans.capture_unit_transports(
                &cached,
                request.base,
                request.game_state,
                request.inputs.as_deref(),
                &request.packs.orders,
            );
            request.packs.ticks[replay_data::rows::TableId::DivisionMovement.index()] += cost;
            request
                .packs
                .air
                .merge_ground_supply(&request.countries.bytes_view(memory), request.base);
            Report::Late(LateReport)
        }
        Job::AirSupply(request) => {
            // Countries resolve from the hour's snapshot bytes, as in the AirMissions joins.
            request.ok = request
                .capture
                .read(
                    &request.countries.bytes_view(memory),
                    request.base,
                    request.game_state,
                )
                .is_ok();
            Report::AirSupply(AirSupplyReport)
        }
        Job::ScoresRails(request) => {
            let parts = &mut *request.parts;
            // The score walk resolves each country directly, as on the game,
            // from the hour's snapshot bytes: the roster would name an alias
            // of the reserved country absent, which direct resolution reads.
            // Strategic-air and score reads stay native.
            parts.scores.capture(
                &request.countries.bytes_view(memory),
                request.base,
                request.game_state,
                request.hour,
                request.force_full,
                request.score_capacity,
                &mut request.air_scores,
            );
            parts.rails.capture(
                memory,
                request.base,
                request.game_state,
                request.hour,
                request.rail_store.as_deref_mut(),
            );
            Report::ScoresRails(ScoresRailsReport)
        }
        Job::ArmyIcons(request) => {
            request.ticks = request.icons.capture(
                memory,
                request.base,
                request.divisions.sources(),
                request
                    .inputs
                    .as_deref()
                    .map(crate::division_census::UnitInputs::seeds),
            );
            Report::ArmyIcons(ScoresRailsReport)
        }
        Job::DivisionModelInputs(request) => {
            request.captured.capture(
                memory,
                request.base,
                request.game_state,
                request.inputs.as_deref(),
            );
            Report::DivisionModelInputs(DivisionModelInputsReport)
        }
        Job::Orders(request) => {
            request.orders.walk_first_part(memory, request.base);
            Report::Orders(OrdersReport)
        }
        Job::OrdersPart(request) => {
            request.part.run(memory, request.base);
            Report::OrdersPart(OrdersReport)
        }
        Job::OrdersStitch(request) => {
            // The stitch leaves before work the census does not gate is
            // dispatched, so its own panic must not stop the pool now: it is
            // contained here, reported, and applied at the stitch's join.
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                #[cfg(test)]
                if request.base == STITCH_PANIC_BASE {
                    panic!("synthetic stitch panic");
                }
                // The stitch fills the request it reads its countries from.
                let countries = Arc::clone(&request.countries);
                let cached = countries.roster_view(memory);
                request.stitch(&cached);
            }));
            // As the runner does: a panic payload can itself panic when
            // dropped, so its destructor never runs.
            let contained_panic = outcome.map_err(std::mem::forget).is_err();
            Report::OrdersStitch(StitchReport { contained_panic })
        }
        Job::Fronts(request) => {
            request.fronts.capture(memory, request.base);
            Report::Fronts(FrontsReport)
        }
        Job::ChildFronts(request) => {
            let cached = request.countries.roster_view(memory);
            request.plans.visit_child_fronts(
                &cached,
                request.base,
                request.orders.roots(),
                request.group_source.as_ref(),
            );
            Report::ChildFronts(OrdersReport)
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
        Job::GatedProjects(request) => {
            request
                .captured
                .profile_acquisition(crate::attach_diagnostics::measuring());
            let countries = request.countries.clone();
            match countries.as_deref() {
                Some(countries) => request.capture(
                    memory,
                    &countries.roster_view(memory),
                    &crate::country_input_changes::Observer,
                ),
                None => request.capture(memory, memory, &crate::country_input_changes::Observer),
            }
            Report::GatedProjects(())
        }
        Job::CharacterInputs(request) => Report::CharacterInputs(request.capture(memory)),
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
            Report::ShipModelDetails(request.captured.capture_deferred(&cached, request.context))
        }
        Job::Provinces(request) => {
            let (base, game_state) = (request.base, request.game_state);
            Report::Provinces(
                request.run(|capture| {
                    crate::province_changes::walk(capture, memory, base, game_state)
                }),
            )
        }
        Job::Factions(request) => {
            Report::Factions(acquisition_factions::run_growing(request, memory))
        }
        Job::Countries(request) => Report::Countries(request.run(memory)),
        Job::Templates(request) => Report::Templates(request.run(memory)),
        Job::Wars(request) => Report::Wars(request.run_growing(memory)),
        Job::States(request) => Report::States(request.run_growing(memory)),
        Job::PresentationPrepare(request) | Job::PresentationSettle(request) => {
            let cached = request.countries.roster_view(memory);
            let inputs = request.inputs.as_deref();
            request.captured.prepare_status_holding(
                &cached,
                request.context,
                production::Sources {
                    rows: inputs.map_or(&[], |inputs| inputs.seeds()),
                    outcome: inputs.map_or(
                        engine_native::walk::Outcome::Unavailable {
                            reason: "acquisition source not supplied",
                        },
                        |inputs| inputs.outcome(),
                    ),
                },
                request.army_stats.as_deref_mut(),
            );
            // Armed setup and ordered-settle jobs release their own ranges.
            request.dispatch.dispatch(
                &mut request.captured,
                &request.countries,
                request.context,
                &request.inputs,
            );
            Report::PresentationPrepare(())
        }
        Job::PresentationStatus(request) => {
            let _reads = crate::read_accounting::table(replay_data::rows::TableId::DivisionStatus);
            let begin = sampling::qpc();
            let cached = request.countries.roster_view(memory);
            request
                .part
                .as_mut()
                .expect("presentation range retained")
                .capture(
                    &cached,
                    request.context,
                    request
                        .inputs
                        .as_deref()
                        .map_or(&[], |inputs| inputs.seeds()),
                    &crate::army_stat_changes::Observer,
                );
            Report::PresentationStatus(sampling::qpc().saturating_sub(begin).max(0) as u64)
        }
        Job::Presentation(request) => {
            let cached = request.countries.roster_view(memory);
            let inputs = request.inputs.as_deref();
            let missing = engine_native::walk::Outcome::Unavailable {
                reason: "acquisition source not supplied",
            };
            let sources = || production::Sources {
                rows: inputs.map_or(&[], |inputs| inputs.seeds()),
                outcome: inputs.map_or(missing, |inputs| inputs.outcome()),
            };
            let report = request.captured.capture_status_holding(
                &cached,
                request.context,
                sources(),
                request.army_stats.as_deref_mut(),
            );
            request.captured.capture_attack(
                &cached,
                request.context,
                sources(),
                request.army_stats.as_deref_mut(),
            );
            Report::Presentation(report)
        }
        Job::PresentationSpeedTerms(request) => {
            let cached = request.countries.roster_view(memory);
            let inputs = request.inputs.as_deref();
            let context = request.context;
            Report::PresentationSpeedTerms(request.pass.capture(
                &cached,
                context.base,
                context.game_state,
                production::Sources {
                    rows: inputs.map_or(&[], |inputs| inputs.seeds()),
                    outcome: inputs.map_or(
                        engine_native::walk::Outcome::Unavailable {
                            reason: "acquisition source not supplied",
                        },
                        |inputs| inputs.outcome(),
                    ),
                },
            ))
        }
        Job::PresentationMovement(request) => {
            let cached = request.countries.roster_view(memory);
            let inputs = request.inputs.as_deref();
            let context = request.context;
            Report::PresentationMovement(request.pass.capture(
                &cached,
                context.base,
                context.game_state,
                production::Sources {
                    rows: inputs.map_or(&[], |inputs| inputs.seeds()),
                    outcome: inputs.map_or(
                        engine_native::walk::Outcome::Unavailable {
                            reason: "acquisition source not supplied",
                        },
                        |inputs| inputs.outcome(),
                    ),
                },
                Some(context.hour),
            ))
        }
        Job::Intel(request) => {
            let cached = request.countries.roster_view(memory);
            let divisions = Some(if let Some(reason) = request.input_failure {
                engine_visibility::unit_viewer_entry::DivisionSources {
                    entries: &[],
                    outcome: engine_native::walk::Outcome::Unavailable { reason },
                }
            } else {
                request
                    .inputs
                    .as_ref()
                    .map(|inputs| inputs.intel_sources())
                    .unwrap_or(engine_visibility::unit_viewer_entry::DivisionSources {
                        entries: &[],
                        outcome: engine_native::walk::Outcome::Unavailable {
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
    /// Slots this role contributes to the peak hourly request reservation.
    const fn jobs_per_hour(self) -> usize {
        match self {
            Self::Variants => 2 * acquisition::MAX_WORKERS,
            Self::Governance | Self::GatedProjects => acquisition::MAX_WORKERS,
            Self::AirWing => air::WING_PARTS,
            Self::AirRelations => crate::extended::air_relations::PARTS,
            Self::AirHistories => crate::extended::air_histories::PARTS,
            Self::ShipModelDetails => ship_models::MAX_PARTS,
            Self::CharacterPortraits => character_portraits::PARTS,
            Self::SupplyTrains => supply_trains::PARTS,
            Self::DivisionInputParts => crate::division_census::MAX_PARTS - 1,
            Self::PresentationStatus | Self::PresentationPreparation => {
                division_presentation::MAX_STATUS_PARTS
            }
            // Worker continuations can publish the next phase before the
            // callback collects any ticket from the preceding one.
            Self::PresentationPrepare | Self::PresentationSettle => 1,
            Self::ProductionModifierParts => crate::extended::production_modifiers::MAX_PARTS - 1,
            Self::OrdersPart => crate::orders_parts::MAX_PARTS - 1,
            // Roles whose jobs follow one another (the observer boundary
            // and the event-feed seal) hold one at once. OrderRoots and Orders, and
            // DivisionInputs and Divisions, also follow one another, but are
            // reserved apart.
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
    executor: Arc<acquisition::Pool<Job, Report>>,
    calibration: Arc<SharedCalibration>,
    worker_count: usize,
    presentation_inputs: Arc<PresentationInputs>,
    presentation_preparation: Arc<PresentationFanIn>,
    presentation_stitch: Arc<PresentationFanIn>,
    final_inputs: Arc<FinalInputsDispatch>,
    ship_details: Arc<ShipDetailsDispatch>,
}
impl Pool {
    #[cfg(test)]
    pub(crate) fn with_test_runner(
        workers: usize,
        run: impl Fn(&mut Job) -> Report + Send + Sync + 'static,
    ) -> Self {
        let executor = acquisition::Pool::start(workers, MAX_JOBS, run).unwrap();
        executor.set_on_completed(presentation::completed);
        executor.set_diagnostics(Job::diagnostic_identity);
        executor.reserve_worker_slots(WORKER_ONLY_JOBS).unwrap();
        let executor = Arc::new(executor);
        let calibration = Arc::new(SharedCalibration::default());
        Self {
            presentation_inputs: Arc::new(PresentationInputs::new(&executor, &calibration)),
            final_inputs: Arc::new(FinalInputsDispatch::new(&executor, &calibration)),
            ship_details: Arc::new(ShipDetailsDispatch::new(&executor, &calibration)),
            presentation_preparation: Arc::new(PresentationFanIn::new(&executor, &calibration)),
            presentation_stitch: Arc::new(PresentationFanIn::new(&executor, &calibration)),
            executor,
            calibration,
            worker_count: workers,
        }
    }

    pub(crate) fn start(worker_count: usize, capacity: usize) -> Result<Self, &'static str> {
        let calibration = Arc::new(SharedCalibration::default());
        let runner_calibration = Arc::clone(&calibration);
        let executor = acquisition::Pool::start_with_class(
            worker_count.max(1),
            capacity,
            Some((ROLE_COUNT, |job: &Job| job.role() as usize)),
            move |job: &mut Job| {
                #[cfg(feature = "worker_sampler")]
                let _sampler_job = crate::worker_sampler::job(job.role());
                // SAFETY: submit's callback lease remains admitted until every
                // accepted request is collected, including on callback unwind.
                let memory = CountedMemory::new(unsafe { LiveMemory::new() });
                run_with_memory(job, &memory, &runner_calibration)
            },
        )?;
        executor.set_callback_help(Job::callback_help);
        executor.set_on_completed(presentation::completed);
        executor.set_diagnostics(Job::diagnostic_identity);
        executor.reserve_worker_slots(WORKER_ONLY_JOBS)?;
        let executor = Arc::new(executor);
        Ok(Self {
            presentation_inputs: Arc::new(PresentationInputs::new(&executor, &calibration)),
            final_inputs: Arc::new(FinalInputsDispatch::new(&executor, &calibration)),
            ship_details: Arc::new(ShipDetailsDispatch::new(&executor, &calibration)),
            presentation_preparation: Arc::new(PresentationFanIn::new(&executor, &calibration)),
            presentation_stitch: Arc::new(PresentationFanIn::new(&executor, &calibration)),
            executor,
            calibration,
            worker_count: worker_count.max(1),
        })
    }
    /// # Safety
    /// Every accepted request must finish before the foreign-world callback
    /// lease ends. Use Batch to retain that barrier on an unwinding callback.
    pub(crate) unsafe fn submit(
        &self,
        request: &mut Option<Job>,
        wake: bool,
    ) -> Result<Ticket, acquisition::SubmitError> {
        match request.as_mut() {
            Some(Job::Variants(request)) => request.workers.arm(self),
            Some(Job::Governance(request)) => request.workers.arm(self),
            Some(Job::GatedProjects(request)) => request.workers.arm(self),
            _ => {}
        }
        let role = request.as_ref().map(Job::role);
        let priority = request.as_ref().map_or(0, Job::priority);
        let worker_only = request.as_ref().is_some_and(Job::worker_only);
        // SAFETY: the caller retains the admitted callback lease.
        let result = unsafe {
            self.executor
                .submit_with_priority(request, priority, worker_only, wake)
        };
        if let Some(role) = role {
            note_dispatch(&self.calibration.roles[role as usize], result.is_ok());
        }
        result
    }
    /// Collect into the caller's empty `out` (see `acquisition::Pool::finish_into`).
    pub(crate) fn finish_into(
        &self,
        ticket: Ticket,
        out: &mut Option<Completion<Job, Report>>,
    ) -> bool {
        if !self.executor.finish_into(ticket, out) {
            return false;
        }
        account_completion(
            &self.calibration,
            out.as_ref().expect("collected completion"),
        );
        true
    }
    /// Retain a rejected request in a free executor slot until `unpark`.
    pub(crate) fn park(&self, request: &mut Option<Job>) -> Option<Ticket> {
        let worker_only = request.as_ref().is_some_and(Job::worker_only);
        self.executor.park(request, worker_only)
    }
    pub(crate) fn unpark(&self, ticket: Ticket) -> Option<Job> {
        self.executor.unpark(ticket)
    }
    /// Execute an eligible retained, rejected request on the callback with the
    /// same providers and panic boundary. Worker-only roles cannot run here.
    ///
    /// # Safety
    /// The admitted callback lease must remain valid throughout this call.
    // Keep the request, isolated read counts and completion in their own frame.
    #[inline(never)]
    pub(crate) unsafe fn run_inline(
        &self,
        ticket: Ticket,
        mut request: Job,
    ) -> Completion<Job, Report> {
        assert!(!request.worker_only(), "acquisition requires a worker");
        let role = request.role();
        let identity = request.diagnostic_identity();
        let begin = sampling::qpc();
        let growths_before = engine_allocation::growths();
        let reads = crate::read_accounting::Capture::new();
        let report = catch_failure(|| {
            // SAFETY: the caller retains the callback's live-read lease.
            let memory = CountedMemory::new(unsafe { LiveMemory::new() });
            run_with_memory(&mut request, &memory, &self.calibration)
        });
        let mut table_reads = crate::read_accounting::ZERO;
        reads.finish_into(&mut table_reads);
        crate::read_accounting::merge(&table_reads);
        let counters = &self.calibration.roles[role as usize];
        counters.inline_completed.fetch_add(1, Ordering::Relaxed);
        if report.is_err() {
            counters.inline_panicked.fetch_add(1, Ordering::Relaxed);
        }
        let timeline = acquisition::Timeline {
            body_begin_qpc: begin,
            body_end_qpc: sampling::qpc(),
            growths: engine_allocation::growths().saturating_sub(growths_before),
            caller: true,
            ..Default::default()
        };
        crate::acquisition_diagnostics::update(|hour| {
            hour.jobs.record(crate::acquisition_diagnostics::Job {
                identity,
                ticket,
                timeline,
            })
        });
        Completion {
            request,
            report,
            table_reads,
            timeline,
            #[cfg(feature = "callback_profile")]
            profile: Default::default(),
        }
    }
    pub(crate) fn worker_count(&self) -> usize {
        self.worker_count
    }
    pub(crate) fn thread_ids(&self) -> Vec<u32> {
        self.executor.thread_ids()
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.executor
            .allocated_bytes()
            .saturating_add(std::mem::size_of::<acquisition::Pool<Job, Report>>())
            .saturating_add(2 * std::mem::size_of::<usize>())
            .saturating_add(std::mem::size_of::<SharedCalibration>())
            .saturating_add(2 * std::mem::size_of::<usize>())
            .saturating_add(std::mem::size_of::<PresentationInputs>())
            .saturating_add(std::mem::size_of::<FinalInputsDispatch>())
            .saturating_add(std::mem::size_of::<ShipDetailsDispatch>())
            .saturating_add(2 * std::mem::size_of::<PresentationFanIn>())
            .saturating_add(10 * std::mem::size_of::<usize>())
    }
    pub(crate) fn world_calibration(&self) -> WorldCalibration {
        *self
            .calibration
            .world
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    pub(crate) fn presentation_input_hold_ticks(&self) -> u64 {
        self.presentation_inputs.hold_ticks()
    }
    pub(crate) fn calibration(&self) -> Calibration {
        let mut frequency = 0;
        // SAFETY: valid local output; diagnostics convert retained QPC totals.
        unsafe {
            windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
        };
        Calibration {
            executor: self.executor.calibration(),
            roles: std::array::from_fn(|index| RoleCalibration {
                role: ROLES[index],
                counters: self.calibration.roles[index].snapshot(),
            }),
            presentation_continuations: std::array::from_fn(|index| {
                self.calibration.presentation_continuations[index].snapshot(
                    [
                        "inputs_to_prepare",
                        "preparation_to_settle",
                        "status_to_stitch",
                        "final_inputs",
                    ][index],
                    frequency.max(1) as u64,
                )
            }),
        }
    }
    pub(crate) fn shutdown(&self) -> Result<(), &'static str> {
        self.executor.shutdown()
    }
    /// See [`acquisition::Pool::stopped`].
    pub(crate) fn stopped(&self) -> bool {
        self.executor.stopped()
    }
    /// See [`acquisition::Pool::stop_for_contained_panic`]; the panic is
    /// also counted against `role`, as the runner counts an uncontained one.
    pub(crate) fn stop_for_contained_panic(&self, role: Role) {
        self.calibration.roles[role as usize]
            .panicked
            .fetch_add(1, Ordering::Relaxed);
        self.executor.stop_for_contained_panic();
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
    /// Whether each retained ticket holds a parked rejected request.
    parked: [bool; MAX_JOBS],
    /// Submissions queue without waking a worker until [`Self::release`].
    held: bool,
    /// Jobs queued since the hold began.
    held_jobs: usize,
    presentation_inputs: Option<Arc<PresentationInputs>>,
    presentation_stitch: Arc<PresentationFanIn>,
}
impl<'a> Batch<'a> {
    pub(crate) fn new(pool: &'a Pool) -> Self {
        pool.presentation_preparation.open();
        pool.presentation_stitch.open();
        pool.final_inputs.open();
        Self {
            pool,
            tickets: [None; MAX_JOBS],
            parked: [false; MAX_JOBS],
            held: false,
            held_jobs: 0,
            presentation_inputs: None,
            presentation_stitch: Arc::clone(&pool.presentation_stitch),
        }
    }
    /// Queue the following submissions without waking workers: a burst of
    /// independent jobs is released at once ([`Self::release`]).
    pub(crate) fn hold(&mut self) {
        self.held = true;
    }
    /// Wake one worker and relay the remaining burst wakes on workers; returns
    /// the queued job count. Workers already running claim them sooner.
    pub(crate) fn release(&mut self) -> usize {
        let jobs = std::mem::take(&mut self.held_jobs);
        self.held = false;
        if jobs != 0 {
            self.pool.executor.wake_workers(jobs);
        }
        jobs
    }
    pub(crate) fn arm_ship_details(&self, request: &mut ShipModelsRequest) {
        self.pool.ship_details.arm(request);
        request.details = Some(Arc::clone(&self.pool.ship_details));
    }
    pub(crate) fn arm_presentation(&mut self, request: &mut PresentationRequest) {
        request
            .dispatch
            .arm(self.pool, Arc::clone(&self.presentation_stitch));
    }
    pub(crate) fn final_inputs(&self) -> Arc<FinalInputsDispatch> {
        Arc::clone(&self.pool.final_inputs)
    }
    pub(crate) fn arm_presentation_inputs(
        &mut self,
        request: PresentationRequest,
        parts: usize,
        store: Option<Arc<division_presentation::held_store::Slot>>,
        omit_status: bool,
    ) -> Arc<PresentationInputs> {
        let dispatch = Arc::clone(&self.pool.presentation_inputs);
        dispatch.arm(request, parts, store, omit_status);
        self.presentation_inputs = Some(Arc::clone(&dispatch));
        dispatch
    }
    pub(crate) fn adopt_presentation(
        &mut self,
        request: &mut PresentationRequest,
        index: usize,
    ) -> Option<(Ticket, bool)> {
        let pending = request.dispatch.take(index)?;
        Some(self.adopt_presentation_ticket(pending))
    }
    pub(crate) fn adopt_presentation_ticket(
        &mut self,
        pending: presentation::RangeTicket,
    ) -> (Ticket, bool) {
        // Executor worker-slot admission already bounded parent and children
        // together; adoption cannot need a new native or Batch slot.
        let slot = self.tickets[..WORKER_ONLY_JOBS]
            .iter()
            .position(Option::is_none)
            .expect("admitted continuation child slot");
        self.tickets[slot] = Some(pending.ticket);
        self.parked[slot] = pending.parked;
        (pending.ticket, pending.parked)
    }
    /// Whether a submission now would be refused ([`Pool::stopped`]).
    pub(crate) fn stopped(&self) -> bool {
        self.pool.stopped()
    }
    /// Whether an accepted `ticket` has completed
    /// ([`acquisition::Pool::is_complete`]).
    pub(crate) fn is_complete(&self, ticket: Ticket) -> bool {
        self.pool.executor.is_complete(ticket)
    }
    /// Copy a completed report without collecting its request. This lets a
    /// clean stitch advance while its contained-panic settlement stays at the
    /// existing dispatch barrier. Only the callback collects these tickets.
    pub(crate) fn completed_report(&self, ticket: Ticket) -> Option<Result<Report, Failure>> {
        self.pool.executor.completed_report(ticket)
    }
    pub(crate) fn completed_state(&self, ticket: Ticket) -> Option<(Result<Report, Failure>, i64)> {
        self.pool.executor.completed_state(ticket)
    }
    /// Advance whichever dependency barrier is ready, without joining a
    /// different producer first. Rejected requests are ready at their normal
    /// collection path; accepted producers must have published completely.
    pub(crate) fn wait_ready<T>(&self, mut ready: impl FnMut(&Self) -> Option<T>) -> T {
        self.pool.executor.wait_ready(|| ready(self))
    }
    /// [`Pool::stop_for_contained_panic`].
    pub(crate) fn stop_for_contained_panic(&self, role: Role) {
        self.pool.stop_for_contained_panic(role);
    }
    /// # Safety
    /// The callback must own the live-read lease until this guard is
    /// dropped. No lock held across finish/Drop may be needed by a queued job.
    pub(crate) unsafe fn submit(
        &mut self,
        request: &mut Option<Job>,
    ) -> Result<Ticket, SubmitError> {
        let worker_only = request.as_ref().is_some_and(Job::worker_only);
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
        let ticket = unsafe { self.pool.submit(request, !self.held) }?;
        *slot = Some(ticket);
        self.held_jobs += usize::from(self.held);
        Ok(ticket)
    }
    /// Retain a rejected request in the pool until its join, so the callback
    /// holds only the ticket. Parked and accepted requests share the MAX_JOBS
    /// bound; parking prefers the request's own partition. None leaves the
    /// request with the caller.
    pub(crate) fn park(&mut self, request: &mut Option<Job>) -> Option<Ticket> {
        let worker_only = request.as_ref().is_some_and(Job::worker_only);
        let (own, other) = if worker_only {
            (0..WORKER_ONLY_JOBS, WORKER_ONLY_JOBS..MAX_JOBS)
        } else {
            (WORKER_ONLY_JOBS..MAX_JOBS, 0..WORKER_ONLY_JOBS)
        };
        let index = own
            .chain(other)
            .find(|&index| self.tickets[index].is_none())?;
        let ticket = self.pool.park(request)?;
        self.tickets[index] = Some(ticket);
        self.parked[index] = true;
        Some(ticket)
    }
    /// Take back a parked request; other tickets return None.
    pub(crate) fn unpark(&mut self, ticket: Ticket) -> Option<Job> {
        let index = self.position(ticket, true)?;
        self.tickets[index] = None;
        self.parked[index] = false;
        self.pool.unpark(ticket)
    }
    /// Refuse in the pool's existing heap cell, without an owned request copy.
    pub(crate) fn finish_parked_into(
        &mut self,
        ticket: Ticket,
        out: &mut Option<Completion<Job, Report>>,
        complete: impl FnOnce(&mut Job) -> Option<Result<Report, Failure>>,
    ) -> bool {
        let Some(index) = self.position(ticket, true) else {
            return false;
        };
        if !self.pool.executor.finish_parked_into(ticket, out, complete) {
            return false;
        }
        self.tickets[index] = None;
        self.parked[index] = false;
        true
    }
    #[cfg(test)]
    pub(crate) fn finish(&mut self, ticket: Ticket) -> Option<Completion<Job, Report>> {
        let mut completed = None;
        self.finish_into(ticket, &mut completed);
        completed
    }
    /// Collect an accepted ticket into the caller's empty `out`.
    pub(crate) fn finish_into(
        &mut self,
        ticket: Ticket,
        out: &mut Option<Completion<Job, Report>>,
    ) -> bool {
        let Some(index) = self.position(ticket, false) else {
            return false;
        };
        let collected = self.pool.finish_into(ticket, out);
        self.tickets[index] = None;
        collected
    }
    fn position(&self, ticket: Ticket, parked: bool) -> Option<usize> {
        self.tickets
            .iter()
            .zip(&self.parked)
            .position(|(candidate, &held)| held == parked && *candidate == Some(ticket))
    }
    /// # Safety
    /// Same live-read lease as submit; only a retained rejected job may run here.
    pub(crate) unsafe fn run_inline(
        &self,
        ticket: Ticket,
        request: Job,
    ) -> Completion<Job, Report> {
        // SAFETY: forwarded from this callback's lease.
        unsafe { self.pool.run_inline(ticket, request) }
    }
}
/// The callback's live-read barrier on every exit: the callback returns or
/// unwinds into the game only after this drop, which collects every
/// retained ticket (an accepted job is joined, finished or cancelled before
/// it starts; a parked request is taken back) and then settles the pool, so
/// no job runs or can start afterwards. Collection is unconditional: a
/// terminal allocation failure, before or during the drain, stops new work
/// but never skips a join, and a panic while dropping one completion is
/// contained so the remaining tickets still drain.
impl Drop for Batch<'_> {
    fn drop(&mut self) {
        if let Some(dispatch) = &self.presentation_inputs {
            dispatch.close();
        }
        self.pool.presentation_preparation.close();
        self.presentation_stitch.close();
        self.pool.final_inputs.close();
        self.pool.ship_details.close();
        // A held burst must reach the workers before its tickets drain.
        self.release();
        let pool = self.pool;
        for (ticket, parked) in self.tickets.iter_mut().zip(&mut self.parked) {
            let Some(ticket) = ticket.take() else {
                continue;
            };
            // User job panics are caught by the executor. Recovering/dropping
            // owned results must not prevent draining any remaining ticket.
            if std::mem::take(parked) {
                contain(|| drop_parked(pool, ticket));
            } else {
                contain(|| drop_collected(pool, ticket));
            }
        }
        // Settle publishers before draining their unadopted successors. The
        // closed ledgers admit nothing further; both drains retain the lease.
        pool.executor.settle();
        if let Some(dispatch) = &self.presentation_inputs {
            dispatch.drain();
        }
        pool.presentation_preparation.drain();
        self.presentation_stitch.drain();
        self.pool.final_inputs.drain();
        self.pool.ship_details.drain();
        // SAFETY: all tickets and detached publishers are drained before the next callback.
        let jobs = unsafe { self.pool.executor.take_diagnostics() };
        crate::acquisition_diagnostics::update(|hour| hour.jobs.merge(&jobs));
    }
}

// Separate frames: a request and a completion never share one. Each drops
// where it lies, at the end of its frame.
#[inline(never)]
fn drop_parked(pool: &Pool, ticket: Ticket) {
    let _request = pool.unpark(ticket);
}

#[inline(never)]
fn drop_collected(pool: &Pool, ticket: Ticket) {
    let mut completed = None;
    pool.finish_into(ticket, &mut completed);
    // A stitch contains its own panic until its join settles it
    // (`sampling::settle_stitch`). Drained without that join, it is counted
    // and stops the pool here, unless the pool had already stopped: a
    // stitch the stopped pool would have refused counts no panic.
    if let Some(Completion {
        report:
            Ok(Report::OrdersStitch(StitchReport {
                contained_panic: true,
            })),
        ..
    }) = &completed
        && !pool.stopped()
    {
        pool.stop_for_contained_panic(Role::OrdersStitch);
    }
}

#[cfg(test)]
pub(crate) mod batch_drain_tests;
#[cfg(test)]
mod order_group_loan_tests;
#[cfg(test)]
mod scheduling_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_accounting::{self, Capture};
    use characters::CharactersCopyContext;
    use engine_native::memory::ReadCounts;
    use replay_data::rows::TableId;

    #[test]
    fn variant_schedule_reads_membership_without_forcing_dependency_baselines() {
        for hour in 0..48 {
            let scheduled =
                replay_data::cadence::should_walk(TableId::VariantDefinitions, hour, false);
            assert_eq!(variant_read_plan(hour, false, false), (scheduled, false));
            assert_eq!(variant_read_plan(hour, true, false), (true, true));
            assert_eq!(variant_read_plan(hour, false, true), (true, false));
        }
    }

    fn accounted_read(owner: TableId, len: usize) {
        read_accounting::with_table(owner, || {
            let source = [7; 8];
            let memory = CountedMemory::new(engine_native::memory::SliceMemory {
                base: 0x1000,
                bytes: &source,
            });
            assert!(engine_native::Memory::copy(
                &memory,
                0x1000,
                &mut [0; 8][..len]
            ));
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
        memory: &impl engine_native::Memory,
        cells: usize,
    ) -> Arc<engine_native::country_memory::Cache> {
        let mut storage =
            Arc::new(engine_native::country_memory::Cache::try_with_capacity(cells).unwrap());
        engine_native::country_memory::Cache::prepare_shared(&mut storage, memory, 0, 0x1000)
            .unwrap()
    }

    fn empty_country_snapshot() -> Arc<engine_native::country_memory::Cache> {
        country_snapshot(
            &engine_native::memory::SliceMemory {
                base: 0,
                bytes: &empty_memory(),
            },
            64,
        )
    }

    fn empty_memory() -> Vec<u8> {
        let mut bytes = vec![0; 0x3000];
        for (offset, pointer) in [
            (engine_native::cgame_state::COUNTRIES, 0x2000_u64),
            (engine_native::cgame_state::COUNTRY_LINKS, 0x2100),
            (engine_native::cgame_state::PROVINCES, 0x2200),
        ] {
            let at = 0x1000 + offset;
            bytes[at..at + 8].copy_from_slice(&pointer.to_le_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&2_u32.to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&2_u32.to_le_bytes());
        }
        bytes
    }

    fn economy_request() -> EconomyRequest {
        let mut request = EconomyRequest {
            countries: empty_country_snapshot(),
            context: economy::EconomyRestCopyContext {
                base: 0,
                game_state: 0x1000,
                resource_rows: [0; 3],
                resource_scratch: 0,
                resources_active: true,
                other_active: true,
            },
            payload: Default::default(),
        };
        request.payload.log = crate::raw::RawLog::new(512, 65536);
        request
    }

    fn world_request() -> WorldRequest {
        let mut request = WorldRequest {
            countries: empty_country_snapshot(),
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
        };
        request.construction.as_mut().unwrap().payload.log = crate::raw::RawLog::new(512, 65536);
        request.production.as_mut().unwrap().payload.log = crate::raw::RawLog::new(512, 65536);
        request
    }

    fn synthetic_pool(run: impl Fn(&mut Job) -> Report + Send + Sync + 'static) -> Pool {
        Pool::with_test_runner(1, run)
    }

    #[test]
    fn every_worker_only_job_of_an_hour_has_a_reserved_slot() {
        // Every single worker-only role and all air relation and history parts.
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
        // The final borrowers are queued before the Presentation join.
        assert_eq!(
            Role::PresentationPrepare.jobs_per_hour()
                + Role::PresentationPreparation.jobs_per_hour()
                + Role::PresentationSettle.jobs_per_hour()
                + Role::PresentationStatus.jobs_per_hour()
                + Role::Presentation.jobs_per_hour()
                + Role::FinalInputs.jobs_per_hour(),
            2 * division_presentation::MAX_STATUS_PARTS + 4
        );
    }

    #[test]
    fn presentation_stages_have_distinct_roles_and_complete_chain_slots() {
        let countries =
            Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
        let context = division_presentation::CaptureContext {
            base: 0,
            game_state: 0,
            hour: 0,
        };
        let request = || PresentationRequest {
            countries: Arc::clone(&countries),
            context,
            captured: division_presentation::Captured::vacant(),
            dispatch: Default::default(),
            inputs: None,
            army_stats: None,
        };
        assert_eq!(
            Job::PresentationPrepare(request()).role(),
            Role::PresentationPrepare
        );
        assert_eq!(
            Job::PresentationSettle(request()).role(),
            Role::PresentationSettle
        );
        assert_eq!(Job::Presentation(request()).role(), Role::Presentation);
        let status = Job::PresentationStatus(PresentationStatusRequest {
            countries: Arc::clone(&countries),
            context,
            inputs: None,
            preparation: false,
            stitch: None,
            part: Some(division_presentation::StatusWork::Status {
                prepared: Arc::new(engine_native::carmy::StatusPacket::vacant()),
                part: engine_native::carmy::StatusPart::vacant(),
                attack: Default::default(),
            }),
        });
        assert_eq!(status.role(), Role::PresentationStatus);
        let mut preparation = Job::PresentationStatus(PresentationStatusRequest {
            countries: Arc::clone(&countries),
            context,
            inputs: None,
            preparation: true,
            stitch: None,
            part: Some(division_presentation::StatusWork::Preparation {
                store: None,
                part: Default::default(),
            }),
        });
        assert_eq!(preparation.role(), Role::PresentationPreparation);
        if let Job::PresentationStatus(request) = &mut preparation {
            request.part = None;
        }
        assert_eq!(preparation.role(), Role::PresentationPreparation);
        for role in [
            Role::PresentationPrepare,
            Role::PresentationPreparation,
            Role::PresentationStatus,
            Role::PresentationSettle,
            Role::Presentation,
        ] {
            assert!(role.worker_only());
            assert_eq!(role.priority(), 0);
            assert!(role.priority() < Role::Countries.priority());
            assert_eq!(role.read_anchor(), Role::Presentation.read_anchor());
        }
        assert_eq!(Role::PresentationPrepare.jobs_per_hour(), 1);
        assert_eq!(
            Role::PresentationPreparation.jobs_per_hour(),
            division_presentation::MAX_STATUS_PARTS
        );
        assert_eq!(Role::PresentationSettle.jobs_per_hour(), 1);
        assert_eq!(
            Role::PresentationStatus.jobs_per_hour(),
            division_presentation::MAX_STATUS_PARTS
        );
        assert_eq!(Role::Presentation.jobs_per_hour(), 1);

        let pool = synthetic_pool(|job| {
            assert_eq!(job.role(), Role::PresentationPreparation);
            Report::PresentationStatus(0)
        });
        let mut request = request();
        for _ in 0..2 {
            let mut batch = Batch::new(&pool);
            request.captured = division_presentation::Captured::vacant();
            request.captured.prepare_status_holding(
                &engine_native::memory::SliceMemory {
                    base: 0,
                    bytes: &[],
                },
                context,
                production::Sources {
                    rows: &[],
                    outcome: engine_native::walk::Outcome::Rows { len: 0 },
                },
                None,
            );
            batch.arm_presentation(&mut request);
            assert!(!request.dispatch.dispatched());
            assert_eq!(request.dispatch.ticks, 0);
            request.dispatch.dispatch(
                &mut request.captured,
                &request.countries,
                context,
                &request.inputs,
            );
            assert!(request.dispatch.dispatched());
            let (ticket, parked) = batch.adopt_presentation(&mut request, 0).unwrap();
            assert!(!parked);
            let completed = batch.finish(ticket).unwrap();
            assert!(matches!(
                completed.report,
                Ok(Report::PresentationStatus(0))
            ));
        }
        {
            let mut batch = Batch::new(&pool);
            batch.arm_presentation(&mut request);
            pool.presentation_preparation.close();
            request.dispatch.dispatch(
                &mut request.captured,
                &request.countries,
                context,
                &request.inputs,
            );
            assert!(
                !request.dispatch.dispatched(),
                "closed admission stays closed"
            );
        }
        let stats = pool.calibration();
        assert_eq!(
            stats.roles[Role::PresentationPreparation as usize]
                .counters
                .accepted,
            2
        );
        assert_eq!(
            stats.roles[Role::PresentationStatus as usize]
                .counters
                .accepted,
            0
        );
        assert_eq!(stats.executor.uncollected, 0);
    }

    #[test]
    fn presentation_speed_terms_have_their_own_early_worker_slot() {
        for (index, role) in ROLES.iter().enumerate() {
            assert_eq!(*role as usize, index, "{role:?}");
        }
        assert!(Role::PresentationSpeedTerms.worker_only());
        assert_eq!(Role::PresentationSpeedTerms.jobs_per_hour(), 1);
        assert_eq!(
            Role::PresentationSpeedTerms.priority(),
            Role::Countries.priority()
        );
        assert!(Role::Presentation.priority() < Role::PresentationSpeedTerms.priority());
        assert!(Role::PresentationSpeedTerms.priority() < Role::PresentationMovement.priority());
        assert_eq!(
            Role::PresentationSpeedTerms.read_anchor(),
            TableId::MovementStateTerms
        );
    }

    #[test]
    fn variants_and_raid_state_claim_before_the_saturated_census_tier() {
        assert_eq!(Role::Variants.priority(), Role::RaidState.priority());
        for role in ROLES {
            if matches!(
                role,
                Role::Variants
                    | Role::RaidState
                    | Role::PresentationPrepare
                    | Role::PresentationPreparation
                    | Role::PresentationSettle
                    | Role::PresentationStatus
                    | Role::Presentation
            ) {
                assert_eq!(role.priority(), 0, "{role:?}");
            } else {
                assert!(Role::Variants.priority() < role.priority(), "{role:?}");
            }
        }
    }

    #[test]
    fn the_stability_tables_are_their_own_job_beside_slow() {
        // ROLES lists every role at its index, the new one included.
        for (index, role) in ROLES.iter().enumerate() {
            assert_eq!(*role as usize, index, "{role:?}");
        }
        // It claims no later than Slow, and its reads stay charged to
        // CountryStabilityInputs.
        assert!(Role::Stability.priority() <= Role::Slow.priority());
        assert_eq!(
            Role::Stability.read_anchor(),
            TableId::CountryStabilityInputs
        );
        let mut captured =
            crate::extended::Captured::with_capacities(&crate::extended::Capacities::default());
        let countries = empty_country_snapshot();
        let bytes = empty_memory();
        let memory = CountedMemory::new(engine_native::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        });
        let mut job = Job::Stability(captured.take_stability_request(
            &countries,
            0,
            0x1000,
            1,
            true,
            Some(Box::new(crate::slicing::SliceFingerprints::new())),
        ));
        assert_eq!(job.role(), Role::Stability);
        let reads = Capture::new();
        let report = run_for_test(&mut job, &memory);
        let reads = reads.finish();
        assert!(matches!(report, Report::Stability(StabilityReport)));
        // Only the stability tables' accounting sees the job's reads.
        for (index, counts) in reads.iter().enumerate() {
            if index != TableId::CountryStabilityInputs.index() {
                assert_eq!(*counts, ReadCounts::ZERO, "table {index}");
            }
        }
        let Job::Stability(request) = job else {
            unreachable!()
        };
        assert!(captured.restore_stability_request(request, true).is_some());
    }

    /// The daily production slot's walks are independent jobs, queued exactly
    /// at their slot (hour 8, or a full hour): they claim no later than Slow,
    /// and their reads stay charged to the industrial organisations' and
    /// production modifiers' own tables.
    #[test]
    fn the_daily_production_tables_are_their_own_job_beside_slow() {
        use crate::extended::{
            ProductionDailyPacks, industrial_organisations, production_modifiers,
        };
        assert!(Role::ProductionDaily.priority() <= Role::Slow.priority());
        assert_eq!(
            Role::IndustrialOrganisations.priority(),
            Role::ProductionDaily.priority()
        );
        assert!(Role::IndustrialOrganisations.worker_only());
        assert_eq!(
            Role::IndustrialOrganisations.read_anchor(),
            TableId::IndustrialOrganisations
        );
        assert_eq!(
            Role::ProductionDaily.read_anchor(),
            TableId::CountryProductionModifiers
        );
        for hour in 0..48 {
            assert_eq!(
                ProductionDailyPacks::due(hour, false),
                hour % 24 == 8,
                "hour {hour}"
            );
        }
        assert!(ProductionDailyPacks::due(3, true));
        let mut captured =
            crate::extended::Captured::with_capacities(&crate::extended::Capacities::default());
        let countries = empty_country_snapshot();
        let bytes = empty_memory();
        let memory = CountedMemory::new(engine_native::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        });
        let mut mio = Job::IndustrialOrganisations(
            captured.take_industrial_organisations_request(&countries, 0, 0x1000, 8, false),
        );
        let mut job = Job::ProductionDaily(
            captured.take_production_daily_request(&countries, 0, 0x1000, 8, false),
        );
        assert_eq!(mio.role(), Role::IndustrialOrganisations);
        assert_eq!(job.role(), Role::ProductionDaily);
        let mio_reads = Capture::new();
        let Report::IndustrialOrganisations(mio_report) = run_for_test(&mut mio, &memory) else {
            unreachable!()
        };
        let mio_reads = mio_reads.finish();
        for (index, counts) in mio_reads.iter().enumerate() {
            if index != industrial_organisations::TABLES[0].index() {
                assert_eq!(*counts, ReadCounts::ZERO, "MIO table {index}");
            }
        }
        let reads = Capture::new();
        let report = run_for_test(&mut job, &memory);
        let reads = reads.finish();
        assert!(matches!(
            report,
            Report::ProductionDaily(ProductionDailyReport)
        ));
        for (index, counts) in reads.iter().enumerate() {
            if index != production_modifiers::TABLES[0].index() {
                assert_eq!(*counts, ReadCounts::ZERO, "table {index}");
            }
        }
        let Job::IndustrialOrganisations(mio) = mio else {
            unreachable!()
        };
        let Job::ProductionDaily(mut request) = job else {
            unreachable!()
        };
        captured.restore_industrial_organisations_request(
            mio,
            true,
            mio_report.ticks,
            &mut request.packs,
        );
        captured.restore_production_daily_request(request, true);
    }

    #[test]
    fn the_war_overview_tables_are_their_own_job_beside_slow() {
        // It claims no later than Slow, and its reads stay charged to
        // WarOwnedRelations and WarNameRules, the overview's own tables.
        assert!(Role::WarOverview.priority() <= Role::Slow.priority());
        assert_eq!(Role::WarOverview.read_anchor(), TableId::WarOwnedRelations);
        let mut captured =
            crate::extended::Captured::with_capacities(&crate::extended::Capacities::default());
        let countries = empty_country_snapshot();
        let bytes = empty_memory();
        let memory = CountedMemory::new(engine_native::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        });
        let mut job =
            Job::WarOverview(captured.take_war_overview_request(&countries, 0, 0x1000, 1, true));
        assert_eq!(job.role(), Role::WarOverview);
        let reads = Capture::new();
        let report = run_for_test(&mut job, &memory);
        let reads = reads.finish();
        assert!(matches!(report, Report::WarOverview(WarOverviewReport)));
        for (index, counts) in reads.iter().enumerate() {
            if index != TableId::WarOwnedRelations.index() && index != TableId::WarNameRules.index()
            {
                assert_eq!(*counts, ReadCounts::ZERO, "table {index}");
            }
        }
        let Job::WarOverview(request) = job else {
            unreachable!()
        };
        captured.restore_war_overview_request(request, true);
    }

    /// Runs `job` on `memory`: its report and the reads each table made.
    fn run_counted(
        job: &mut Job,
        memory: &impl engine_native::Memory,
    ) -> (Report, read_accounting::TableCounts) {
        let reads = Capture::new();
        let report = run_for_test(job, memory);
        (report, reads.finish())
    }

    fn snapshot_at(
        memory: &impl engine_native::Memory,
        cells: usize,
        base: usize,
        game_state: usize,
    ) -> Arc<engine_native::country_memory::Cache> {
        let mut storage =
            Arc::new(engine_native::country_memory::Cache::try_with_capacity(cells).unwrap());
        engine_native::country_memory::Cache::prepare_shared(&mut storage, memory, base, game_state)
            .unwrap()
    }

    #[test]
    fn the_air_bases_census_takes_the_hours_country_snapshot() {
        use crate::extended::air::{AirBasesCopyContext, AirBasesJob, air_bases_test_kit as kit};
        let bytes = kit::fixture();
        // An unready snapshot (no cells) serves nothing: every census span is
        // then read from the game, as the job read it before the loan.
        let run = |cells| {
            let countries = snapshot_at(&kit::memory(&bytes), cells, kit::BASE, kit::GAME_STATE);
            let memory = CountedMemory::new(kit::memory(&bytes));
            let mut job = Job::AirBases(RawRequest {
                countries,
                context: AirBasesCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour: 0,
                    force_full: true,
                },
                payload: AirBasesJob::default(),
            });
            let (report, reads) = run_counted(&mut job, &memory);
            assert!(matches!(report, Report::AirBases(report) if report.raw_ok));
            let Job::AirBases(request) = job else {
                unreachable!()
            };
            (request.payload, reads[TableId::AirBases.index()])
        };
        let (native, native_reads) = run(0);
        let (loaned, loaned_reads) = run(1 << 12);
        // A ready snapshot's roster stands in for every census span, and the
        // decoder replays the same rows through it.
        let header = kit::GAME_STATE + engine_native::cgame_state::COUNTRIES;
        assert!(native.roster.roster(kit::BASE, kit::GAME_STATE).is_none());
        assert!(native.log.spans.iter().any(|span| span.addr == header));
        assert!(loaned.roster.roster(kit::BASE, kit::GAME_STATE).is_some());
        assert!(!loaned.log.spans.iter().any(|span| span.addr == header));
        assert!(loaned.log.spans.len() < native.log.spans.len());
        kit::assert_matches_direct(&kit::decode(&loaned), &kit::decode(&native));
        assert!(
            loaned_reads.copy_calls < native_reads.copy_calls,
            "{loaned_reads:?} {native_reads:?}"
        );
    }

    #[test]
    fn the_score_walk_resolves_countries_through_the_hours_snapshot() {
        use engine_native::{
            ccountry, cgame_state, cstrategic_air, cstrategic_air_manager, cstrategic_region,
            memory::SliceMemory,
        };
        use engine_raid_inputs::raid_air_scores::COMBAT_DELAY_RVA;
        const MEMORY: usize = 0x1000_0000;
        const BASE: usize = MEMORY + 0x100 - COMBAT_DELAY_RVA.0;
        const GAME: usize = MEMORY + 0x1000;
        // The combat delay define, one present region and country P in slot
        // 1. Without an air manager the walk stops at the strategic-air
        // roster. With `alias`, P has id zero and also fills slot 0, and the
        // manager's slot 1 owner names raw country 1: the snapshot's roster
        // leaves that alias of the reserved country absent, while the score
        // walk resolves it directly through P's identity.
        let world = |alias: bool| {
            let mut bytes = vec![0_u8; 0x4000];
            let mut put = |at: usize, value: u64, len: usize| {
                bytes[at..at + len].copy_from_slice(&value.to_le_bytes()[..len]);
            };
            put(0x100, 4, 4);
            for (header, data) in [
                (cgame_state::COUNTRIES, 0x3000),
                (cgame_state::COUNTRY_LINKS, 0x3100),
                (cgame_state::STRATEGIC_REGIONS, 0x3400),
            ] {
                put(0x1000 + header, (MEMORY + data) as u64, 8);
                put(0x1000 + header + 8, 2 << 32 | 2, 8);
            }
            put(0x3008, (MEMORY + 0x3200) as u64, 8);
            put(0x3104, 1, 4);
            put(0x3200, (BASE + ccountry::VTABLE_RVA.0) as u64, 8);
            put(0x3200 + ccountry::ID, u64::from(!alias), 4);
            put(0x3408, (MEMORY + 0x3600) as u64, 8);
            put(0x3600, (BASE + cstrategic_region::VTABLE_RVA.0) as u64, 8);
            put(0x3600 + cstrategic_region::PROVINCES_COUNT, 1, 4);
            put(0x3600 + cstrategic_region::ID, 1_001, 4);
            put(0x3600 + cstrategic_region::RUNTIME_SLOT, 1, 4);
            put(0x3600 + cstrategic_region::AIR_PHASE, 1, 4);
            if alias {
                put(0x3000, (MEMORY + 0x3200) as u64, 8);
                put(
                    0x1000 + cgame_state::STRATEGIC_AIR_MANAGER,
                    (MEMORY + 0x3800) as u64,
                    8,
                );
                put(
                    0x3800,
                    (BASE + cstrategic_air_manager::VTABLE_RVA.0) as u64,
                    8,
                );
                let roster = 0x3800 + cstrategic_air_manager::COUNTRY_AIR;
                put(roster, (MEMORY + 0x3900) as u64, 8);
                put(roster + 8, 2 << 32 | 2, 8);
                put(0x3908, (MEMORY + 0x3a00) as u64, 8);
                put(0x3a00, (BASE + cstrategic_air::VTABLE_RVA.0) as u64, 8);
                put(0x3a00 + cstrategic_air::COUNTRY, 1, 4);
            }
            bytes
        };
        for (alias, reason) in [
            (
                false,
                "strategic air manager vtable unreadable or mismatched",
            ),
            (true, "country id is zero"),
        ] {
            let bytes = world(alias);
            let source = || SliceMemory {
                base: MEMORY,
                bytes: &bytes,
            };
            let run = |cells| {
                let mut captured = crate::extended::Captured::with_capacities(
                    &crate::extended::Capacities::default(),
                );
                captured.prepare_hour(1, true);
                let countries = snapshot_at(&source(), cells, BASE, GAME);
                let request = captured.take_scores_rails_request(
                    &countries,
                    BASE,
                    GAME,
                    1,
                    true,
                    Box::default(),
                    None,
                );
                let mut job = Job::ScoresRails(request);
                let (_, reads) = run_counted(&mut job, &CountedMemory::new(source()));
                let Job::ScoresRails(request) = job else {
                    unreachable!()
                };
                let _ = captured.restore_scores_rails_request(request, true);
                (
                    captured.outcomes()[TableId::RaidAirScores.index()],
                    reads[TableId::RaidAirScores.index()],
                )
            };
            // An unready snapshot serves nothing: the walk reads as on the game.
            let (native, native_reads) = run(0);
            let (loaned, loaned_reads) = run(1 << 12);
            assert_eq!(native, engine_native::walk::Outcome::Unavailable { reason });
            assert_eq!(loaned, native, "alias {alias}");
            assert!(
                loaned_reads.copy_calls < native_reads.copy_calls,
                "{loaned_reads:?} {native_reads:?}"
            );
        }
    }

    #[test]
    fn the_economy_rest_is_its_own_job_beside_world() {
        // ROLES lists every role at its index, the new one included.
        for (index, role) in ROLES.iter().enumerate() {
            assert_eq!(*role as usize, index, "{role:?}");
        }
        // It claims with World, and both keep the anchor World had.
        assert_eq!(Role::Economy.priority(), Role::World.priority());
        assert_eq!(Role::Economy.read_anchor(), TableId::ProvinceSupply);
        assert_eq!(Role::World.read_anchor(), TableId::ProvinceSupply);
        let bytes = empty_memory();
        let memory = CountedMemory::new(engine_native::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        });
        let calibration = SharedCalibration::default();
        let mut economy = Job::Economy(economy_request());
        assert_eq!(economy.role(), Role::Economy);
        let reads = Capture::new();
        let report = run_with_memory(&mut economy, &memory, &calibration);
        let economy_reads = reads.finish();
        assert!(matches!(report, Report::Economy(_)));
        // Only the economy tables' accounting sees the job's reads.
        for (index, counts) in economy_reads.iter().enumerate() {
            if !economy::TABLES.iter().any(|table| table.index() == index) {
                assert_eq!(*counts, ReadCounts::ZERO, "table {index}");
            }
        }
        assert_ne!(economy_reads, read_accounting::ZERO);
        // The World job reads none of them.
        let mut world = Job::World(world_request());
        let reads = Capture::new();
        let report = run_with_memory(&mut world, &memory, &calibration);
        let world_reads = reads.finish();
        assert!(matches!(report, Report::World(_)));
        for &table in &economy::TABLES {
            assert_eq!(world_reads[table.index()], ReadCounts::ZERO, "{table:?}");
        }
        // The country cache is observed once for the hour, by the Economy
        // job; both jobs' worker time is the family's.
        let world_calibration = *calibration.world.lock().unwrap();
        let cache = world_calibration.country_cache;
        assert_eq!(
            cache.ready + cache.capacity_fallbacks + cache.unavailable_fallbacks,
            1
        );
        assert!(world_calibration.last_error.is_none());
    }

    #[test]
    fn country_cache_calibration_distinguishes_each_seed_and_retains_fallback_counts() {
        use engine_native::country_memory::SeedStatus;
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
        use engine_native::{Memory, ccountry, cgame_state, cunit, walk::Outcome};
        use engine_visibility::unit_viewer_entry;
        use fog::intel_test_kit as kit;
        use std::cell::RefCell;

        struct Traced<'a> {
            source: engine_native::memory::SliceMemory<'a>,
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
        fn decoded(job: &fog::IntelJob) -> (Outcome, replay_frame::frame::TableView) {
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
        let links = engine_native::read::<u64>(
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
                Arc::new(engine_native::country_memory::Cache::try_with_capacity(cells).unwrap());
            let countries = engine_native::country_memory::Cache::prepare_shared(
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
                        vec![engine_native::carmy::DivisionSeed {
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
                            replay_frame::frame::TableView::Unavailable {
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
                            let replay_frame::frame::TableView::Complete(rows) = table else {
                                unreachable!();
                            };
                            assert_eq!(
                                rows.rows::<replay_data::rows::tables::UnitIntel>().unwrap()[0]
                                    .level,
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
        let memory = engine_native::memory::SliceMemory {
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
                &engine_native::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                &Mutex::new(WorldCalibration::default()),
            )
        })
        .unwrap();
        let request = world_request();
        let pointers = [
            request
                .construction
                .as_ref()
                .unwrap()
                .payload
                .log
                .bytes
                .as_ptr(),
            request
                .production
                .as_ref()
                .unwrap()
                .payload
                .log
                .bytes
                .as_ptr(),
        ];
        let mut pending = Some(request);
        // SAFETY: synthetic inputs and all destinations are owned until collection.
        let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
        let completed = pool.finish(ticket).unwrap();
        let report = completed.report.unwrap();
        macro_rules! compare {
            ($family:ident, $pointer:expr) => {{
                let part = completed.request.$family.as_ref().unwrap();
                assert_eq!(part.payload.log.bytes.as_ptr(), $pointer);
                assert_eq!(report.$family.raw_ok, expected.$family.raw_ok);
                assert_eq!(report.$family.raw_bytes, expected.$family.raw_bytes);
                assert_eq!(
                    part.payload.log.bytes,
                    direct.$family.as_ref().unwrap().payload.log.bytes
                );
            }};
        }
        compare!(construction, pointers[0]);
        compare!(production, pointers[1]);
        assert_eq!(pool.calibration().uncollected, 0);
    }

    #[test]
    fn the_economy_job_matches_the_direct_copy_and_returns_its_storage() {
        for workers in [0, 1] {
            let bytes = empty_memory();
            let mut direct = economy_request();
            let memory = CountedMemory::new(engine_native::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            });
            let reads = Capture::new();
            let expected = economy::copy_economy_rest_refresh(
                &direct.countries.view(&memory),
                direct.context,
                &mut direct.payload,
            );
            let expected_reads = reads.finish();
            let pool = acquisition::Pool::start(workers, 1, move |request: &mut EconomyRequest| {
                run_economy_shared(
                    request,
                    &CountedMemory::new(engine_native::memory::SliceMemory {
                        base: 0,
                        bytes: &bytes,
                    }),
                    &Mutex::new(WorldCalibration::default()),
                )
            })
            .unwrap();
            let request = economy_request();
            let pointer = request.payload.log.bytes.as_ptr();
            let mut pending = Some(request);
            // SAFETY: the runner owns its source and collection retains every output.
            let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
            let completed = pool.finish(ticket).unwrap();
            let report = completed.report.unwrap();
            assert_eq!(completed.request.payload.log.bytes.as_ptr(), pointer);
            assert_eq!(
                completed.request.payload.log.bytes,
                direct.payload.log.bytes
            );
            assert_eq!(report.raw_ok, expected.raw_ok);
            assert_eq!(report.raw_bytes, expected.raw_bytes);
            assert_eq!(completed.table_reads, expected_reads);
            assert_eq!(pool.calibration().uncollected, 0);
        }
    }

    #[test]
    fn absent_world_parts_read_nothing_and_keep_no_economy_attempt() {
        for workers in [0, 1] {
            let bytes = empty_memory();
            let pool = acquisition::Pool::start(workers, 1, move |request: &mut WorldRequest| {
                run_world_shared(
                    request,
                    &CountedMemory::new(engine_native::memory::SliceMemory {
                        base: 0,
                        bytes: &bytes,
                    }),
                    &Mutex::new(WorldCalibration::default()),
                )
            })
            .unwrap();
            let mut request = world_request();
            request.production = None;
            request.construction = None;
            let mut pending = Some(request);
            // SAFETY: the runner owns its source and collection retains every output.
            let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
            let completed = pool.finish(ticket).unwrap();
            let report = completed.report.unwrap();
            assert!(completed.request.production.is_none());
            assert!(completed.request.construction.is_none());
            // The economy rest is the Economy job's: World reads nothing.
            assert_eq!(completed.table_reads, read_accounting::ZERO);
            macro_rules! empty {
                ($part:ident) => {{
                    assert!(!report.$part.raw_ok);
                    assert!(report.$part.copy_ticks.iter().all(|&value| value == 0));
                    assert!(report.$part.raw_bytes.iter().all(|&value| value == 0));
                }};
            }
            empty!(production);
            empty!(construction);
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
            final_inputs: None,
            context: CharactersCopyContext {
                force_full: false,
                base: 0,
                game_state: 0x1000,
                trait_rows: 0,
                trait_bytes: 0,
                active: [true; crate::extended::characters::N],
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
            let source = vec![0_u8; engine_native::cgame_state::PROVINCES + 16];
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
                        payload: engine_native::cprovince::Capture::try_with_capacity(2, 0, 0)
                            .unwrap(),
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
                    assert_eq!(
                        report.result,
                        engine_native::walk::Outcome::Unavailable { reason }
                    );
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
                    engine_native::walk::Outcome::Unavailable { reason }
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

    /// The Map witness runs as its own role over the hour's census; a missing
    /// census or a failed job refuses its table, never an empty one.
    #[test]
    fn the_model_witness_job_reads_the_census_under_its_own_role() {
        use engine_native::walk::Outcome;
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let table = TableId::DivisionModelInputs.index();
        let mut captured =
            crate::extended::Captured::with_capacities(&crate::extended::Capacities::default());
        let empty = Arc::new(crate::division_census::UnitInputs::fixture(
            Vec::new(),
            Outcome::Rows { len: 0 },
        ));
        for (inputs, completed, outcome, training) in [
            (Some(&empty), true, Outcome::Rows { len: 0 }, Some(0)),
            (
                None,
                true,
                Outcome::Unavailable {
                    reason: "division census unavailable",
                },
                None,
            ),
            (
                Some(&empty),
                false,
                Outcome::Unavailable {
                    reason: "division model input acquisition failed",
                },
                None,
            ),
        ] {
            captured.reset();
            let mut job = Job::DivisionModelInputs(captured.take_division_model_inputs_request(
                0,
                0x1000,
                inputs.cloned(),
            ));
            assert_eq!(job.role(), Role::DivisionModelInputs);
            let Report::DivisionModelInputs(_) = run_for_test(&mut job, &memory) else {
                unreachable!("model witness report type");
            };
            let Job::DivisionModelInputs(request) = job else {
                unreachable!("model witness job type");
            };
            assert_eq!(
                captured.restore_division_model_inputs_request(request, completed),
                training
            );
            assert_eq!(captured.outcomes()[table], outcome);
        }
        assert_eq!(Arc::strong_count(&empty), 1);
    }
}
