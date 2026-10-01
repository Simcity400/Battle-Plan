//! Coordinated extended acquisition. Public arrays use global TableId indices;
//! individual packs keep compact local arrays. Allocation is off-thread.

use crate::{
    division_census::UnitInputs,
    map_capture::{MapSources, Orders, ShipCensus},
    sampling::qpc,
    table_capture::{CaptureRows, grow_capacity, grow_vec},
};
use engine::{
    Memory, cair_wing, carmy, cequipment_variant, cequipment_variant_pool::DivisionSource,
    walk::Outcome,
};
use replay::{
    frame::{TABLE_COUNT, TableInput},
    rows::TableId,
};
use std::{mem::size_of, sync::Arc};

#[cfg(test)]
pub(crate) mod bench_world;
pub(crate) mod daily_intel;

mod packs;
pub(crate) use packs::{
    GatedPacks, LatePacks, OrdinaryPacks, RaidLaunchPacks, RaidStatePacks, RaidStateStores,
    RaidTargetPacks, SlowPacks, SlowStores,
};

const MISSING: Outcome = Outcome::Unavailable {
    reason: "acquisition source not supplied",
};
const DIVISION_SOURCE_TABLES: [TableId; 9] = [
    TableId::DivisionEquipment,
    TableId::ArmyUpgradeGroups,
    TableId::ArmyUpgradeRequests,
    TableId::ArmyUpgradeDeliveries,
    TableId::ArmyUpgradeProduced,
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::DivisionEtaInputs,
    TableId::ArmyIconInputs,
];
pub type ReadGroup = (&'static str, TableId, Vec<TableId>);

/// One hour's raid reads that were torn or refused (`status.json`
/// `raid_reads`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RaidReadCounts {
    /// Path memo copies a game writer overlapped (`raid_path_memo`).
    pub(crate) torn_memo_reads: u32,
    /// Raid, cooldown and target rows refused for an unproven leader captor.
    pub(crate) refused_leaders: u32,
}

/// A terminal job failure invalidates scheduled reads, never carried samples.
fn failed_capture(table: TableId, hour: i32, force_full: bool, reason: &'static str) -> Outcome {
    Outcome::Unavailable {
        reason: if replay::cadence::should_walk(table, hour, force_full) {
            reason
        } else {
            replay::cadence::SKIPPED_REASON
        },
    }
}

// Native conversion reuse stays within its pack's existing timing scope.
macro_rules! spool_inputs {
    (lexical, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.lexical.as_deref_mut())
    };
    (characters, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.characters.as_deref_mut())
    };
    (character_portraits, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (leader_portraits, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (ship_models, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (intel, $captured:expr, $conversion:expr) => {
        $captured.table_inputs(
            $conversion.variables_baseline,
            $conversion.variables_scratch,
            $conversion.intel.as_deref_mut(),
        )
    };
    (governance, $captured:expr, $conversion:expr) => {
        $captured.table_inputs()
    };
    (division_presentation, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (production, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (economy, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (fog, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (supply_trains, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    ($pack:ident, $captured:expr, $conversion:expr) => {
        $captured.tables().map(TableInput::Sampled)
    };
}

macro_rules! packs {
    ($($pack:ident),+ $(,)?) => {
        $(pub mod $pack;)+
        pub const PACK_NAMES: &[&str] = &[$(stringify!($pack)),+];
        pub const N: usize = 0 $(+ $pack::N)+;
        pub const TABLES: [TableId; N] = {
            let mut tables = [TableId::LexicalTokens; N];
            let mut next = 0;
            $(let mut index = 0;
            while index < $pack::N {
                tables[next] = $pack::TABLES[index]; next += 1; index += 1;
            })+
            assert!(next == N);
            tables
        };
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct Capacities {
            $(pub $pack: $pack::Capacities,)+
            pub division_sources: usize,
            pub variant_registry: usize,
        }
        impl Default for Capacities {
            fn default() -> Self {
                Self { $($pack: Default::default(),)+ division_sources: 8192, variant_registry: 8192 }
            }
        }
        pub struct Captured {
            $($pack: $pack::Captured,)+
            divisions: Divisions,
            unit_inputs: Option<Arc<UnitInputs>>,
            governance_report: Option<governance::CaptureReport>,
            variants_ticks: Option<[u64; production::N]>,
            presentation_report: Option<division_presentation::CaptureReport>,
            eta_inputs_report: Option<division_eta_inputs::CaptureReport>,
            intel_residual_report: Option<intel::CaptureReport>,
            supply_trains_report: Option<supply_trains::CaptureReport>,
            character_portraits_report: Option<character_portraits::CaptureReport>,
            province_unit_order_report: Option<province_unit_order::CaptureReport>,
            ship_models_report: Option<ship_models::CaptureReport>,
            ordinary_ticks: Option<[u64; TABLE_COUNT]>,
            ordinary_failure: Option<&'static str>,
            ordinary_spare: Option<Box<OrdinaryPacks>>,
            slow_ticks: Option<[u64; TABLE_COUNT]>,
            slow_spare: Option<Box<SlowPacks>>,
            raid_state_ticks: Option<[u64; TABLE_COUNT]>,
            raid_state_spare: Option<Box<RaidStatePacks>>,
            raid_launch_ticks: Option<[u64; TABLE_COUNT]>,
            raid_launch_spare: Option<Box<RaidLaunchPacks>>,
            raid_targets_ticks: Option<[u64; TABLE_COUNT]>,
            raid_targets_spare: Option<Box<RaidTargetPacks>>,
            late_ticks: Option<[u64; TABLE_COUNT]>,
            late_spare: Option<Box<LatePacks>>,
            missions_spare: Option<Box<air::Captured>>,
            gated_ticks: Option<[u64; TABLE_COUNT]>,
            gated_spare: Option<Box<GatedPacks>>,
            registry: Registry,
            air_root: Option<Outcome>,
            overhead: [u64; TABLE_COUNT],
        }
        impl Captured {
            #[cfg(test)]
            pub(crate) fn set_intel_shadow_fixture(&mut self, value: i64) {
                self.intel = intel::intel_shadow_fixture(value);
            }

            pub fn with_capacities(capacity: &Capacities) -> Self {
                let mut captured = Self {
                    $($pack: $pack::Captured::with_capacities(&capacity.$pack),)+
                    divisions: Divisions::new(capacity.division_sources),
                    unit_inputs: None,
                    governance_report: None,
                    variants_ticks: None,
                    presentation_report: None,
                    eta_inputs_report: None,
                    intel_residual_report: None,
                    supply_trains_report: None,
                    character_portraits_report: None,
                    province_unit_order_report: None,
                    ship_models_report: None,
                    ordinary_ticks: None,
                    ordinary_failure: None,
                    ordinary_spare: Some(OrdinaryPacks::spare()),
                    slow_ticks: None,
                    slow_spare: Some(SlowPacks::spare()),
                    raid_state_ticks: None,
                    raid_state_spare: Some(RaidStatePacks::spare()),
                    raid_launch_ticks: None,
                    raid_launch_spare: Some(RaidLaunchPacks::spare()),
                    raid_targets_ticks: None,
                    raid_targets_spare: Some(RaidTargetPacks::spare()),
                    late_ticks: None,
                    late_spare: Some(LatePacks::spare()),
                    missions_spare: Some(Box::new(air::Captured::vacant())),
                    gated_ticks: None,
                    gated_spare: Some(GatedPacks::spare()),
                    registry: Registry::new(capacity.variant_registry),
                    air_root: None, overhead: [0; TABLE_COUNT],
                };
                captured.division_presentation.reserve_sources(capacity.division_sources);
                captured
            }
            /// Reset before the shared division and map walks, not between
            /// source visitation and capture().
            pub fn reset(&mut self) {
                $(self.$pack.reset();)+
                self.divisions.reset(); self.registry.reset();
                self.release_unit_inputs(); self.governance_report = None; self.presentation_report = None; self.eta_inputs_report = None;
                self.variants_ticks = None;
                self.intel_residual_report = None;
                self.supply_trains_report = None; self.character_portraits_report = None; self.province_unit_order_report = None; self.ship_models_report = None;
                self.ordinary_ticks = None;
                self.ordinary_failure = None;
                self.slow_ticks = None;
                self.raid_state_ticks = None;
                self.raid_launch_ticks = None;
                self.raid_targets_ticks = None;
                self.late_ticks = None;
                self.gated_ticks = None;
                self.air_root = None; self.overhead = [0; TABLE_COUNT];
            }
            pub fn allocated_bytes(&self) -> usize {
                0 $(+ self.$pack.allocated_bytes())+ + self.divisions.allocated_bytes() + self.registry.allocated_bytes() + self.spare_bytes()
            }
            /// Spool-thread metadata for this buffer's native row and scratch
            /// allocations. Excludes core tables, encoded frames and queues.
            pub fn allocated_by_pack(&self) -> Vec<(&'static str, usize)> {
                vec![
                    $((stringify!($pack), self.$pack.allocated_bytes()),)+
                    ("division_sources", self.divisions.allocated_bytes()),
                    ("variant_registry", self.registry.allocated_bytes()),
                    ("acquisition_spares", self.spare_bytes()),
                ]
            }
            pub fn grow(&mut self, capacity: &mut Capacities) {
                $(self.$pack.grow(&mut capacity.$pack);)+
                self.divisions.grow(&mut capacity.division_sources);
                self.division_presentation.reserve_sources(capacity.division_sources);
                self.registry.grow(&mut capacity.variant_registry);
            }
            /// Spool-thread conversion only. Existing core slots survive.
            pub(crate) fn insert_tables(
                &self,
                destination: &mut [TableInput; TABLE_COUNT],
                conversion: &mut crate::spooling::SpoolConversion<'_, '_>,
            ) -> [i64; PACK_NAMES.len()] {
                let mut costs = [0; PACK_NAMES.len()];
                let mut slots = costs.iter_mut();
                $(let started = qpc();
                for (id, view) in $pack::TABLES.into_iter().zip(spool_inputs!($pack, self.$pack, conversion)) { destination[id.index()] = view; }
                *slots.next().unwrap() = qpc() - started;)+
                if let Some(reason) = self.ordinary_failure {
                    for &id in OrdinaryPacks::TABLES.iter().flat_map(|tables| tables.iter()) {
                        destination[id.index()] = TableInput::Sampled(replay::frame::TableView::Unavailable { reason: reason.into() });
                    }
                }
                costs
            }
            pub fn outcomes(&self) -> [Outcome; TABLE_COUNT] {
                let mut output = [MISSING; TABLE_COUNT];
                $(scatter(&$pack::TABLES, self.$pack.outcomes(), &mut output);)+
                if let Some(reason) = self.ordinary_failure {
                    for &id in OrdinaryPacks::TABLES.iter().flat_map(|tables| tables.iter()) {
                        output[id.index()] = Outcome::Unavailable { reason };
                    }
                }
                output
            }
        }
        /// Off-thread metadata. Sum members, including physically shared work
        /// charged to a pack's anchor, rather than counting that work twice.
        pub fn read_groups() -> Vec<ReadGroup> {
            let mut groups = Vec::new();
            $(for &(name, anchor, members) in $pack::READ_GROUPS {
                // This graph also consumes the shared division census, so
                // report it once in the combined source group below.
                if $pack::TABLES[anchor] == TableId::ArmyUpgradeGroups { continue; }
                groups.push((name, $pack::TABLES[anchor], members.iter().map(|&index| $pack::TABLES[index]).collect()));
            })+
            groups.push(("divisions, equipment, upgrades, movement, counter status and ETA inputs", TableId::Divisions,
                std::iter::once(TableId::Divisions).chain(DIVISION_SOURCE_TABLES).collect()));
            groups.push(("country census, appearance and supply trains", TableId::Countries,
                vec![TableId::Countries, TableId::CountryAppearance, TableId::SupplyNodes, TableId::TrainInventory]));
            groups.push(("ship census and model operands", TableId::Ships,
                vec![TableId::Ships, TableId::ShipModels]));
            groups.push(("states and hourly state flags", TableId::States,
                vec![TableId::States, TableId::StateFlags]));
            groups.push(("army order census, plan operands, group plans, front sections and invasion preparations", TableId::ArmyOrders,
                vec![TableId::ArmyOrders, TableId::ArmyOrderPlans, TableId::OrdersGroupPlans, TableId::Fronts, TableId::InvasionPreparations]));
            groups
        }
    };
}
pub(crate) mod air_reference_witness;
pub(crate) mod air_sorties;
packs!(
    air,
    air_relations,
    air_histories,
    army_icon_inputs,
    naval_counters,
    railway_guns,
    active_abilities,
    country_counters,
    group_counters,
    saved_event_targets,
    leader_paths,
    country_appearance,
    ship_models,
    province_unit_order,
    naval_bases,
    map_facilities,
    naval_combat_sources,
    character_portraits,
    leader_portraits,
    supply_trains,
    characters,
    combat_progress,
    combat_details,
    naval_details,
    naval_result_details,
    construction,
    division_eta_inputs,
    division_presentation,
    division_templates,
    economy,
    fog,
    governance,
    intel,
    lexical,
    map_names,
    naval_history,
    production,
    projects,
    plans,
    session,
    state_flags,
    visibility,
    raid_target_statuses,
    raid_state_inputs,
    raid_launch_inputs,
    slice_reads,
    faction_theatres,
    model_inputs,
    modifier_definitions,
    hired_advisors,
    character_inputs,
    country_stability,
    new_divisions,
    country_tactic_unlocks,
    weather,
    naval_mission_inputs
);

impl Capacities {
    /// Presize every table whose rows scale with the number of country slots,
    /// so a large mod's first hour is not truncated. Never lowers a capacity.
    pub fn raise_for_countries(&mut self, slots: usize) {
        self.country_appearance.raise_for_countries(slots);
        self.country_counters.raise_for_countries(slots);
        self.combat_details.raise_for_countries(slots);
        self.economy.raise_for_countries(slots);
        self.governance.raise_for_countries(slots);
        self.intel.raise_for_countries(slots);
        self.air.raise_for_countries(slots);
        self.session.raise_for_countries(slots);
        self.projects.raise_for_countries(slots);
        self.supply_trains.raise_for_countries(slots);
        self.construction.raise_for_countries(slots);
        self.visibility.raise_for_countries(slots);
        self.raid_target_statuses.raise_for_countries(slots);
        self.raid_launch_inputs.raise_for_countries(slots);
        self.raid_state_inputs.raise_for_countries(slots);
        self.air_relations.raise_for_countries(slots);
        self.air_histories.raise_for_countries(slots);
        self.model_inputs.raise_for_countries(slots);
        self.hired_advisors.raise_for_countries(slots);
        self.country_stability.raise_for_countries(slots);
        self.country_tactic_unlocks.raise_for_countries(slots);
        self.naval_mission_inputs.raise_for_countries(slots);
    }

    /// Presize CountryIntel from the live dense record total.
    pub fn raise_for_country_intel(&mut self, native_rows: usize) {
        self.intel.raise_for_country_intel(native_rows);
    }

    /// Presize Characters, ScriptFlags and ScriptVariables from the live
    /// roster, flag managers and variable stores
    /// (`country_scale::observe_characters`).
    pub fn raise_for_characters(&mut self, demand: crate::country_scale::CharacterDemand) {
        self.characters.raise_for_characters(demand.characters);
        self.intel.raise_for_script_flags(demand.flags);
        self.intel.raise_for_script_variables(demand.variables);
    }

    /// Presize the raid arenas whose per-country demand content decides.
    pub fn raise_for_raid_arenas(&mut self, demand: crate::country_scale::RaidArenas) {
        self.projects.raise_for_raid_type_tokens(demand.type_tokens);
        self.projects.raise_for_facilities(demand.facility_rows);
        crate::country_scale::raise(&mut self.air.scores.regions, demand.air_regions);
        crate::country_scale::raise(
            &mut self.projects.counteroperatives,
            demand.counteroperatives,
        );
        self.raid_launch_inputs
            .raise_for_reservations(demand.reservation_words);
    }

    /// Presize the political tables from their live demand
    /// (`country_scale::observe_politics`).
    pub fn raise_for_politics(&mut self, demand: crate::country_scale::PoliticsDemand) {
        crate::country_scale::raise(&mut self.intel.agency_names, demand.agency_name_bytes);
        self.hired_advisors
            .raise_for_character_records(demand.active_characters, demand.retired_characters);
        self.character_inputs
            .raise_for_demand(demand.character_inputs);
        self.governance
            .raise_for_party_leaders(demand.party_leaders);
        self.country_counters
            .raise_for_civil_wars(demand.civil_wars);
        self.hired_advisors
            .raise_for_advisor_tokens(demand.advisor_tokens, demand.advisor_token_bytes);
        self.faction_theatres
            .raise_for_faction_advisors(demand.faction_advisors, demand.faction_advisor_bytes);
    }
}

impl Captured {
    /// The CountryIntel buffer this frame uses; `None` only while the intel
    /// pack is out on a residual job.
    pub(crate) fn country_intel_home(&self) -> Option<daily_intel::Home> {
        self.intel.country_intel_home().cloned()
    }

    /// Use the pool's CountryIntel buffer instead of this frame's own.
    pub(crate) fn share_country_intel(&mut self, home: &daily_intel::Home) {
        self.intel.share_country_intel(home);
    }

    /// Freeze this callback's primary activity before any shared source visitor.
    pub(crate) fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        // A frame whose sliced job never runs must not report an older read.
        self.governance.reset_slice_selections(hour, force_full);
        self.air_relations.prepare_hour(hour, force_full);
        self.air_histories.prepare_hour(hour, force_full);
        self.raid_launch_inputs.reset_selection(hour, force_full);
        self.raid_state_inputs.reset_selections(hour, force_full);
        self.division_presentation.reset_selection(hour, force_full);
        self.country_stability.prepare_hour(hour, force_full);
        self.naval_mission_inputs.reset_selection(hour, force_full);
        self.air.prepare_hour(hour, force_full);
        self.fog.prepare_hour(hour, force_full);
        self.naval_history.prepare_hour(hour, force_full);
        self.session.prepare_hour(hour, force_full);
    }

    /// Leases the same immutable native input packet to movement, status and
    /// the ETA inputs. The owning census accounts for its allocation once.
    pub(crate) fn install_unit_inputs(&mut self, inputs: Option<Arc<UnitInputs>>) {
        self.unit_inputs = inputs;
    }

    /// Release after every consumer joins, before the census returns to its pool.
    pub(crate) fn release_unit_inputs(&mut self) {
        self.unit_inputs = None;
    }

    /// Project owned operands after their single live acquisition has finished.
    pub(crate) fn decode_division_presentation_inputs(&mut self) -> [u64; 2] {
        self.division_presentation.decode_inputs()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_presentation_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        store: Option<Box<crate::slicing::SliceFingerprints>>,
    ) -> crate::acquisition_jobs::PresentationRequest {
        self.presentation_report = None;
        crate::acquisition_jobs::PresentationRequest {
            countries: Arc::clone(countries),
            store,
            context: division_presentation::CaptureContext {
                base,
                game_state: state,
                hour,
                force_full,
            },
            inputs: self.unit_inputs.as_ref().map(Arc::clone),
            captured: std::mem::replace(
                &mut self.division_presentation,
                division_presentation::Captured::vacant(),
            ),
        }
    }

    /// Returns the `retreat_bans` store it lent: forgotten if the job failed
    /// (every owner is then new and read at the next hour), and with this
    /// hour's selection invalidated if DivisionRetreatBans was not published
    /// (unavailable or truncated), so its owners are read again at the next
    /// hour.
    pub(crate) fn restore_presentation_request(
        &mut self,
        mut request: crate::acquisition_jobs::PresentationRequest,
        report: Result<division_presentation::CaptureReport, crate::acquisition::Failure>,
    ) -> Option<Box<crate::slicing::SliceFingerprints>> {
        if report.is_err()
            && let Some(store) = request.store.as_deref_mut()
        {
            store.forget();
        }
        let store = request.store.take();
        self.presentation_report = Some(match report {
            Ok(report) => report,
            Err(failure) => request.captured.fail_owned_capture(match failure {
                crate::acquisition::Failure::Panicked => {
                    "division presentation acquisition panicked"
                }
                crate::acquisition::Failure::Stopped => {
                    "division presentation acquisition stopped before reading"
                }
            }),
        });
        self.division_presentation = request.captured;
        let mut store = store;
        let bans = self.division_presentation.outcomes()[division_presentation::BANS];
        if unpublished(&[bans])
            && let Some(store) = store.as_deref_mut()
        {
            self.division_presentation
                .bans_selection()
                .invalidate(store);
        }
        store
    }

    fn acquire_presentation(&mut self) -> division_presentation::CaptureReport {
        self.presentation_report.take().unwrap_or_else(|| {
            self.division_presentation
                .fail_owned_capture("division presentation acquisition not supplied")
        })
    }

    /// Plan stable handle partitions and return how many worker jobs to dispatch.
    pub(crate) fn plan_eta_input_ranges(&mut self) -> usize {
        self.eta_inputs_report = None;
        self.division_eta_inputs
            .plan(&unit_sources(&self.unit_inputs))
    }

    /// Loan one planned range's buffers with the same census seeds Presentation
    /// takes, so the route scan runs across the pool instead of the game thread.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_eta_inputs_request(
        &mut self,
        range: usize,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        cadence: Box<division_eta_inputs::CadenceStore>,
    ) -> crate::acquisition_jobs::EtaInputsRequest {
        let (mut captured, span, partition) = self.division_eta_inputs.take_range(range);
        captured.cadence = Some(cadence);
        crate::acquisition_jobs::EtaInputsRequest {
            countries: Arc::clone(countries),
            context: division_eta_inputs::CaptureContext {
                hour,
                force_full,
                base,
                game_state: state,
                span,
                partition,
            },
            inputs: self.unit_inputs.as_ref().map(Arc::clone),
            captured,
            range,
        }
    }

    /// Every range contributes its outcome and acquisition cost to the table.
    pub(crate) fn restore_eta_inputs_request(
        &mut self,
        mut request: crate::acquisition_jobs::EtaInputsRequest,
        report: Result<division_eta_inputs::CaptureReport, crate::acquisition::Failure>,
    ) {
        let report = match report {
            Ok(report) => report,
            Err(failure) => request.captured.fail_owned_capture(match failure {
                crate::acquisition::Failure::Panicked => "division ETA input acquisition panicked",
                crate::acquisition::Failure::Stopped => {
                    "division ETA input acquisition stopped before reading"
                }
            }),
        };
        let merged = self
            .eta_inputs_report
            .get_or_insert(division_eta_inputs::CaptureReport { ticks: [0] });
        merged.ticks[0] = merged.ticks[0].saturating_add(report.ticks[0]);
        self.division_eta_inputs
            .restore_range(request.range, request.captured);
    }

    /// This hour's template rows whose model inputs failed, published in
    /// status.json.
    pub(crate) fn template_model_failures(&self) -> division_templates::ModelFailures {
        self.division_templates.model_failures()
    }

    /// This hour's ETA input attribution, published in status.json.
    pub(crate) fn eta_input_counters(&self) -> division_eta_inputs::Counters {
        self.division_eta_inputs.counters()
    }

    fn acquire_eta_inputs(&mut self) -> division_eta_inputs::CaptureReport {
        let _reads = crate::read_accounting::table(TableId::DivisionEtaInputs);
        self.eta_inputs_report.take().unwrap_or_else(|| {
            self.division_eta_inputs.fail_missing();
            division_eta_inputs::CaptureReport::default()
        })
    }

    /// Loan the governance family's hourly research tables to their own job.
    /// Take before the family itself; restore after the family is restored.
    pub(crate) fn take_research_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Box<crate::slicing::SliceFingerprints>,
    ) -> crate::acquisition_jobs::ResearchRequest {
        crate::acquisition_jobs::ResearchRequest {
            fingerprints,
            countries: Arc::clone(countries),
            context: governance::CaptureContext {
                base,
                game_state: state,
                hour,
                force_full,
            },
            part: self.governance.take_research(),
        }
    }

    /// Return the research rows and charge their costs to the family report.
    /// A failed job publishes its own failure; it never erases the family.
    /// Returns the fingerprint store: forgotten if the job was interrupted
    /// (every owner is then new and read at the next hour), and with this
    /// hour's selection invalidated if any research table was not read
    /// completely, so its owners are read again at the next hour.
    pub(crate) fn restore_research_request(
        &mut self,
        mut request: crate::acquisition_jobs::ResearchRequest,
        report: Result<crate::acquisition_jobs::ResearchReport, crate::acquisition::Failure>,
    ) -> Box<crate::slicing::SliceFingerprints> {
        let ticks = match report {
            Ok(report) => report.ticks,
            Err(failure) => {
                request.fingerprints.forget();
                request.part.fail(
                    request.context,
                    match failure {
                        crate::acquisition::Failure::Panicked => "research acquisition panicked",
                        crate::acquisition::Failure::Stopped => {
                            "research acquisition stopped before reading"
                        }
                    },
                );
                [0; 3]
            }
        };
        self.governance.restore_research(request.part);
        if let Some(family) = self.governance_report.as_mut() {
            for (index, cost) in governance::RESEARCH.into_iter().zip(ticks) {
                family.ticks[index] = cost;
            }
        }
        let outcomes = self.governance.outcomes();
        if governance::RESEARCH.iter().any(|&index| {
            !matches!(outcomes[index], Outcome::Rows { .. })
                && !matches!(outcomes[index], Outcome::Unavailable { reason } if reason == replay::cadence::SKIPPED_REASON)
        }) {
            self.governance
                .research_selection()
                .invalidate(&mut request.fingerprints);
        }
        request.fingerprints
    }

    /// This hour's torn and refused raid reads: a path memo copy a game
    /// writer overlapped is published unavailable, not retried.
    pub(crate) fn raid_reads(&self) -> RaidReadCounts {
        let projects = self.projects.raid_reads();
        RaidReadCounts {
            torn_memo_reads: projects.torn_memo_reads,
            refused_leaders: projects.refused_leaders + self.raid_target_statuses.refused_leaders(),
        }
    }

    /// One sliced family's read at this frame's hour (`slicing::FAMILIES`).
    pub(crate) fn family_selection(&self, family: usize) -> &crate::slicing::SliceSelection {
        match family {
            crate::slicing::RESEARCH => self.governance.research_selection(),
            crate::slicing::TECHNOLOGIES => self.governance.technology_selection(),
            crate::slicing::RAID_LAUNCH => self.raid_launch_inputs.selection(),
            crate::slicing::RAID_STATE => self.raid_state_inputs.state_selection(),
            crate::slicing::RAID_BUILDINGS => self.raid_state_inputs.country_selection(),
            crate::slicing::RELATIONS => self.intel.relations_selection(),
            crate::slicing::STABILITY => self.country_stability.selection(),
            crate::slicing::RETREAT_BANS => self.division_presentation.bans_selection(),
            crate::slicing::AIR_RELATIONS => self.air_relations.selection(),
            crate::slicing::AIR_HISTORIES => self.air_histories.selection(),
            crate::slicing::NAVAL_MISSIONS => self.naval_mission_inputs.selection(),
            _ => unreachable!("unregistered sliced family {family}"),
        }
    }

    /// The owners whose rows this frame read for a sliced table.
    pub(crate) fn slice_selection(
        &self,
        table: TableId,
    ) -> Option<&crate::slicing::SliceSelection> {
        crate::slicing::family_of(table).map(|family| self.family_selection(family))
    }

    /// Loan the worker-only families after Focus delegation is decided.
    /// Restore after Gated returns construction and before table publication.
    pub(crate) fn take_governance_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        technology_fingerprints: Box<crate::slicing::SliceFingerprints>,
    ) -> crate::acquisition_jobs::GovernanceRequest {
        self.governance_report = None;
        crate::acquisition_jobs::GovernanceRequest {
            technology_fingerprints,
            countries: Arc::clone(countries),
            context: governance::CaptureContext {
                base,
                game_state: state,
                hour,
                force_full,
            },
            captured: std::mem::replace(&mut self.governance, governance::Captured::vacant()),
            buildings: self.construction.take_buildings(),
        }
    }

    /// Loan the variant registry and its detail tables on their slot or a
    /// full hour. Off the slot they are skipped here without reads.
    pub(crate) fn take_variants_request(
        &mut self,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> Option<crate::acquisition_jobs::VariantsRequest> {
        self.variants_ticks = None;
        let context = governance::CaptureContext {
            base,
            game_state: state,
            hour,
            force_full,
        };
        if !replay::cadence::should_walk(TableId::VariantDefinitions, hour, force_full) {
            self.registry.mark_skipped();
            let mut variants = self.production.take_variants();
            variants.fail(replay::cadence::SKIPPED_REASON, hour, force_full);
            self.production.restore_variants(variants);
            self.variants_ticks = Some([0; production::N]);
            return None;
        }
        Some(crate::acquisition_jobs::VariantsRequest {
            context,
            registry: std::mem::replace(&mut self.registry, Registry::new(0)),
            variants: self.production.take_variants(),
        })
    }

    pub(crate) fn restore_variants_request(
        &mut self,
        mut request: crate::acquisition_jobs::VariantsRequest,
        report: Result<[u64; production::N], crate::acquisition::Failure>,
    ) {
        let ticks = report.unwrap_or_else(|failure| {
            request.fail(match failure {
                crate::acquisition::Failure::Panicked => "variant acquisition panicked",
                crate::acquisition::Failure::Stopped => {
                    "variant acquisition stopped before reading"
                }
            })
        });
        self.variants_ticks = Some(ticks);
        self.registry = request.registry;
        self.production.restore_variants(request.variants);
    }

    /// Returns the technology fingerprint store: forgotten if the job was
    /// interrupted, and with this hour's selection invalidated if a technology
    /// table was not read completely, so its owners are read at the next hour.
    pub(crate) fn restore_governance_request(
        &mut self,
        mut request: crate::acquisition_jobs::GovernanceRequest,
        report: Result<crate::acquisition_jobs::GovernanceReport, crate::acquisition::Failure>,
    ) -> Box<crate::slicing::SliceFingerprints> {
        let report = match report {
            Ok(report) => report,
            Err(failure) => {
                request.technology_fingerprints.forget();
                request.fail(match failure {
                    crate::acquisition::Failure::Panicked => "governance acquisition panicked",
                    crate::acquisition::Failure::Stopped => {
                        "governance acquisition stopped before reading"
                    }
                })
            }
        };
        let outcomes = request.captured.outcomes();
        if governance::TECHNOLOGIES.iter().any(|&index| {
            !matches!(outcomes[index], Outcome::Rows { .. })
                && !matches!(outcomes[index], Outcome::Unavailable { reason } if reason == replay::cadence::SKIPPED_REASON)
        }) {
            request
                .captured
                .technology_selection()
                .invalidate(&mut request.technology_fingerprints);
        }
        self.governance_report = Some(report.governance);
        self.governance = request.captured;
        self.construction
            .restore_buildings(request.buildings, Ok(report.buildings));
        request.technology_fingerprints
    }

    /// Loan existing hourly buffers before independent acquisition begins.
    /// Restore them before ordinary capture, table views, growth or publication.
    /// Loan one share of the supply node census; part 0 carries the trains.
    pub(crate) fn take_supply_trains_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        part: usize,
    ) -> crate::acquisition_jobs::SupplyTrainsRequest {
        if part == 0 {
            self.supply_trains_report = None;
        }
        crate::acquisition_jobs::SupplyTrainsRequest {
            countries: Arc::clone(countries),
            context: supply_trains::CaptureContext {
                base,
                game_state: state,
            },
            part,
            nodes: self.supply_trains.take_part(part),
            trains: (part == 0).then(|| self.supply_trains.take_trains()),
        }
    }

    /// Every part adds its ticks; one failed part is terminal for the family.
    /// Keeping the report Some prevents the ordinary stage from retrying reads.
    pub(crate) fn restore_supply_trains_request(
        &mut self,
        mut request: crate::acquisition_jobs::SupplyTrainsRequest,
        report: Result<supply_trains::CaptureReport, crate::acquisition::Failure>,
    ) {
        let report = match report {
            Ok(report) => report,
            Err(failure) => self.supply_trains.fail_owned_capture(match failure {
                crate::acquisition::Failure::Panicked => "supply trains acquisition panicked",
                crate::acquisition::Failure::Stopped => {
                    "supply trains acquisition stopped before reading"
                }
            }),
        };
        let merged = self
            .supply_trains_report
            .get_or_insert(supply_trains::CaptureReport::default());
        for (total, cost) in merged.ticks.iter_mut().zip(report.ticks) {
            *total = total.saturating_add(cost);
        }
        self.supply_trains.restore_part(request.part, request.nodes);
        if let Some(trains) = request.trains.take() {
            self.supply_trains.restore_trains(trains);
        }
    }

    fn acquire_supply_trains(
        &mut self,
        memory: &impl Memory,
        context: supply_trains::CaptureContext,
    ) -> supply_trains::CaptureReport {
        self.supply_trains_report
            .take()
            .unwrap_or_else(|| self.supply_trains.capture_owned(memory, context))
    }

    /// Loan one share of the character roster.
    pub(crate) fn take_character_portraits_request(
        &mut self,
        base: usize,
        state: usize,
        part: usize,
    ) -> crate::acquisition_jobs::CharacterPortraitsRequest {
        if part == 0 {
            self.character_portraits_report = None;
        }
        crate::acquisition_jobs::CharacterPortraitsRequest {
            context: character_portraits::CaptureContext {
                base,
                game_state: state,
            },
            part,
            captured: self.character_portraits.take_part(part),
            census: None,
        }
    }

    /// Every part adds its ticks; one failed part is terminal for the family.
    pub(crate) fn restore_character_portraits_request(
        &mut self,
        request: crate::acquisition_jobs::CharacterPortraitsRequest,
        report: Result<character_portraits::CaptureReport, crate::acquisition::Failure>,
    ) {
        let report = match report {
            Ok(report) => report,
            Err(failure) => self.character_portraits.fail_owned_capture(match failure {
                crate::acquisition::Failure::Panicked => "character portraits acquisition panicked",
                crate::acquisition::Failure::Stopped => {
                    "character portraits acquisition stopped before reading"
                }
            }),
        };
        let merged = self
            .character_portraits_report
            .get_or_insert(character_portraits::CaptureReport::default());
        merged.ticks[0] = merged.ticks[0].saturating_add(report.ticks[0]);
        self.character_portraits
            .restore_part(request.part, request.captured);
    }

    fn acquire_character_portraits(
        &mut self,
        memory: &impl Memory,
        context: character_portraits::CaptureContext,
    ) -> character_portraits::CaptureReport {
        self.character_portraits_report
            .take()
            .unwrap_or_else(|| self.character_portraits.capture_owned(memory, context))
    }

    /// Loan the complete ProvinceUnitOrder rows and handle arena to the flat
    /// acquisition pool. Restore before capture_tables publishes this pack.
    pub(crate) fn take_province_unit_order_request(
        &mut self,
        base: usize,
        state: usize,
    ) -> crate::acquisition_jobs::ProvinceUnitOrderRequest {
        self.province_unit_order_report = None;
        crate::acquisition_jobs::ProvinceUnitOrderRequest {
            context: province_unit_order::CaptureContext {
                base,
                game_state: state,
            },
            captured: std::mem::replace(
                &mut self.province_unit_order,
                province_unit_order::Captured::vacant(),
            ),
        }
    }

    pub(crate) fn restore_province_unit_order_request(
        &mut self,
        mut request: crate::acquisition_jobs::ProvinceUnitOrderRequest,
        report: Result<province_unit_order::CaptureReport, crate::acquisition::Failure>,
    ) {
        // Some, including terminal failure, prevents capture_tables from
        // retrying an accepted request. Zero failure ticks are unmeasured.
        self.province_unit_order_report = Some(match report {
            Ok(report) => report,
            Err(failure) => request.captured.fail_owned_capture(match failure {
                crate::acquisition::Failure::Panicked => "province unit order acquisition panicked",
                crate::acquisition::Failure::Stopped => {
                    "province unit order acquisition stopped before reading"
                }
            }),
        });
        self.province_unit_order = request.captured;
    }

    fn acquire_province_unit_order(
        &mut self,
        memory: &impl Memory,
        context: province_unit_order::CaptureContext,
    ) -> province_unit_order::CaptureReport {
        self.province_unit_order_report
            .take()
            .unwrap_or_else(|| self.province_unit_order.capture_owned(memory, context))
    }

    /// Account for the reusable boxes and any payload they currently own.
    fn spare_bytes(&self) -> usize {
        self.ordinary_spare
            .as_ref()
            .map_or(0, |packs| packs.allocated_bytes())
            + self
                .slow_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
            + self
                .raid_state_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
            + self
                .raid_launch_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
            + self
                .raid_targets_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
            + self
                .late_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
            + self.missions_spare.as_ref().map_or(0, |pack| {
                size_of::<air::Captured>() + pack.allocated_bytes()
            })
            + self
                .gated_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
    }

    /// Loan one part of the AirCountryRelations copies to its job.
    pub(crate) fn take_air_relations_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        part: usize,
    ) -> crate::acquisition_jobs::AirRelationsRequest {
        crate::acquisition_jobs::AirRelationsRequest {
            countries: Arc::clone(countries),
            base,
            game_state: state,
            part,
            staging: self.air_relations.take_part(part),
        }
    }

    /// An interrupted part is failed; the decode thread applies the rest.
    pub(crate) fn restore_air_relations_request(
        &mut self,
        mut request: crate::acquisition_jobs::AirRelationsRequest,
        completed: bool,
    ) {
        if !completed {
            request.staging.fail("air relation acquisition failed");
            air_relations::certificates(request.part).forget();
        }
        self.air_relations
            .restore_part(request.part, request.staging);
    }

    /// Loan the air pack and every history part to the AirMissions job.
    /// Before the map sample it runs on the owned census's identities;
    /// after it, on the typed visitor's, whose census is `air_root`.
    pub(crate) fn take_air_missions_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
    ) -> crate::acquisition_jobs::AirMissionsRequest {
        let mut pack = self.missions_spare.take().expect("missions spare present");
        std::mem::swap(&mut self.air, &mut pack);
        crate::acquisition_jobs::AirMissionsRequest {
            countries: Arc::clone(countries),
            base,
            game_state: state,
            parent: self.air_root.unwrap_or(MISSING),
            pack,
            histories: self.air_histories.take_parts(),
        }
    }

    /// An interrupted job fails the registrations and combats, and leaves
    /// the history parts without regions; nothing is read again.
    pub(crate) fn restore_air_missions_request(
        &mut self,
        mut request: crate::acquisition_jobs::AirMissionsRequest,
        completed: bool,
    ) {
        if !completed {
            request
                .pack
                .fail_missions("air mission acquisition failed", &mut request.histories);
        }
        std::mem::swap(&mut self.air, &mut request.pack);
        self.missions_spare = Some(request.pack);
        self.air_histories.restore_parts(request.histories);
    }

    /// Loan one part of the naval-patrol history copies to its job.
    pub(crate) fn take_air_histories_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        part: usize,
    ) -> crate::acquisition_jobs::AirHistoriesRequest {
        crate::acquisition_jobs::AirHistoriesRequest {
            countries: Arc::clone(countries),
            base,
            game_state: state,
            part,
            staging: self.air_histories.take_part(part),
        }
    }

    pub(crate) fn restore_air_histories_request(
        &mut self,
        mut request: crate::acquisition_jobs::AirHistoriesRequest,
        completed: bool,
    ) {
        if !completed {
            request
                .staging
                .fail("naval-patrol history acquisition failed");
        }
        self.air_histories
            .restore_part(request.part, request.staging);
    }

    /// Game thread, once every history part joined: certify the hour, or
    /// withdraw every part's certificates before the next hour opens.
    pub(crate) fn settle_air_histories(&self) {
        self.air_histories.settle(&mut air_histories::opening());
    }

    /// Decode thread: turn this frame's staged relation and history copies into rows.
    pub(crate) fn decode_air_copies(&mut self) {
        self.air_relations.decode();
        self.air_histories.decode();
    }

    /// The frame is dropped before decoding.
    pub(crate) fn drop_air_copies(&mut self) {
        self.air_relations.dropped();
        self.air_histories.dropped();
    }

    /// Tests reuse a frame without decoding it: drop its copies, as
    /// production does before freeing an undecoded frame, then reset it.
    #[cfg(test)]
    pub(crate) fn drop_and_reset(&mut self) {
        self.drop_air_copies();
        self.reset();
    }

    /// Loan the order census and the plans to the Orders job.
    pub(crate) fn take_orders_request(
        &mut self,
        orders: &mut Orders,
        base: usize,
        game_state: usize,
    ) -> crate::acquisition_jobs::OrdersRequest {
        crate::acquisition_jobs::OrdersRequest {
            base,
            game_state,
            orders: std::mem::replace(orders, Orders::vacant()),
            plans: std::mem::replace(&mut self.plans, plans::Captured::vacant()),
        }
    }

    /// Restore terminal outcomes without retrying either allocating walk.
    pub(crate) fn restore_orders_request(
        &mut self,
        mut request: crate::acquisition_jobs::OrdersRequest,
        completed: bool,
        orders: &mut Orders,
    ) {
        if !completed {
            request.fail("army order acquisition failed");
        }
        *orders = request.orders;
        self.plans = request.plans;
    }

    /// The job walks the ship census into `census` first; the ships seed the
    /// model details.
    pub(crate) fn take_ship_models_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        census: ShipCensus,
    ) -> crate::acquisition_jobs::ShipModelsRequest {
        self.ship_models_report = None;
        crate::acquisition_jobs::ShipModelsRequest {
            countries: Arc::clone(countries),
            context: ship_models::CaptureContext { base },
            game_state: state,
            captured: std::mem::replace(&mut self.ship_models, ship_models::Captured::vacant()),
            census,
        }
    }

    /// Returns the ship census; a failed job leaves it unavailable and the
    /// details terminal. A walked census waits for its detail parts
    /// (`take_ship_model_parts`).
    pub(crate) fn restore_ship_models_request(
        &mut self,
        mut request: crate::acquisition_jobs::ShipModelsRequest,
        report: Result<(), crate::acquisition::Failure>,
    ) -> ShipCensus {
        if let Err(failure) = report {
            let reason = match failure {
                crate::acquisition::Failure::Panicked => "ship models acquisition panicked",
                crate::acquisition::Failure::Stopped => {
                    "ship models acquisition stopped before reading"
                }
            };
            request.census.fail(reason);
            self.ship_models_report = Some(request.captured.fail_owned_capture(reason));
        }
        self.ship_models = request.captured;
        request.census
    }

    /// Split the walked census into detail parts sized from its ship count
    /// and lend them to their jobs; part 0 is the family's own capture. A
    /// failed census lends nothing. Allocation-free.
    pub(crate) fn take_ship_model_parts(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
    ) -> [Option<crate::acquisition_jobs::ShipModelDetailsRequest>; ship_models::MAX_PARTS] {
        let mut requests = [const { None }; ship_models::MAX_PARTS];
        if self.ship_models_report.is_some() || self.ship_models.has_terminal_failure() {
            return requests;
        }
        let parts = self
            .ship_models
            .split_details(ship_models::parts_for(self.ship_models.census_len()));
        let context = ship_models::CaptureContext { base };
        for (part, request) in requests.iter_mut().enumerate().take(parts).skip(1) {
            *request = Some(crate::acquisition_jobs::ShipModelDetailsRequest {
                countries: Arc::clone(countries),
                context,
                part,
                captured: self.ship_models.take_part(part - 1),
            });
        }
        requests[0] = Some(crate::acquisition_jobs::ShipModelDetailsRequest {
            countries: Arc::clone(countries),
            context,
            part: 0,
            captured: std::mem::replace(&mut self.ship_models, ship_models::Captured::vacant()),
        });
        requests
    }

    /// Return one detail part; part 0 first. Ticks add up to the family's
    /// worker time. An interrupted part is terminal for the family.
    pub(crate) fn restore_ship_model_part(
        &mut self,
        mut request: crate::acquisition_jobs::ShipModelDetailsRequest,
        report: Result<ship_models::CaptureReport, crate::acquisition::Failure>,
    ) {
        let report = report.unwrap_or_else(|failure| {
            request.captured.fail_owned_capture(match failure {
                crate::acquisition::Failure::Panicked => "ship models acquisition panicked",
                crate::acquisition::Failure::Stopped => {
                    "ship models acquisition stopped before reading"
                }
            })
        });
        let total = self.ship_models_report.get_or_insert_default();
        total.ticks[0] = total.ticks[0].saturating_add(report.ticks[0]);
        if request.part == 0 {
            self.ship_models = request.captured;
        } else {
            self.ship_models
                .restore_part(request.part - 1, request.captured);
        }
    }

    /// Join the returned parts' outcomes in census order.
    pub(crate) fn finish_ship_model_parts(&mut self) {
        self.ship_models.finish_split();
    }

    fn acquire_ship_models(
        &mut self,
        memory: &impl Memory,
        context: ship_models::CaptureContext,
    ) -> ship_models::CaptureReport {
        if let Some(report) = self.ship_models_report.take() {
            report
        } else if self.ship_models.has_terminal_failure() {
            // A consumed accepted failure remains authoritative until a normal
            // reset/new first source census starts a new epoch. Never reuse its
            // deliberately scrubbed raw seed pointers.
            ship_models::CaptureReport::default()
        } else {
            self.ship_models.capture_owned(memory, context)
        }
    }

    /// Loan Intel's residual tables until the independent raw-job reports are
    /// ready. The callback restores this family before applying those reports.
    pub(crate) fn take_intel_residual_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        context: intel::CaptureContext,
    ) -> crate::acquisition_jobs::IntelResidualRequest {
        self.intel_residual_report = None;
        crate::acquisition_jobs::IntelResidualRequest {
            countries: Arc::clone(countries),
            context,
            captured: std::mem::replace(&mut self.intel, intel::Captured::vacant()),
        }
    }

    pub(crate) fn restore_intel_residual_request(
        &mut self,
        mut request: crate::acquisition_jobs::IntelResidualRequest,
        report: Result<intel::CaptureReport, crate::acquisition::Failure>,
    ) {
        self.intel_residual_report = Some(match report {
            Ok(report) => report,
            Err(failure) => request.captured.fail_residual_capture(
                request.context,
                match failure {
                    crate::acquisition::Failure::Panicked => "residual Intel acquisition panicked",
                    crate::acquisition::Failure::Stopped => {
                        "residual Intel acquisition stopped before reading"
                    }
                },
            ),
        });
        self.intel = request.captured;
    }

    fn acquire_intel(
        &mut self,
        memory: &impl Memory,
        context: intel::CaptureContext,
    ) -> intel::CaptureReport {
        if let Some(mut report) = self.intel_residual_report.take() {
            let delegated = self.intel.capture_delegated(memory, context);
            for index in [0, 13, 14] {
                report.ticks[index] = delegated.ticks[index];
            }
            report
        } else {
            let ticks = self.intel.capture(
                memory,
                context.base,
                context.game_state,
                context.hour,
                context.force_full,
            );
            intel::CaptureReport { ticks }
        }
    }

    /// Decode-worker handoff for the hourly focus pair (copy/decode split).
    /// Sole cross-thread entry to the governance focus buffers: the game
    /// thread stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn focus_pilot(&mut self) -> &mut governance::Captured {
        &mut self.governance
    }

    /// Decode-worker handoff for the relations family (copy/decode split).
    /// Sole cross-thread entry to the intel relations buffers: the game thread
    /// stages raw-copy state through it on refresh hours and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn relations_pilot(&mut self) -> &mut intel::Captured {
        &mut self.intel
    }

    /// Decode-worker handoff for the characters family (copy/decode split).
    /// Sole cross-thread entry to the characters slot-0 buffers: the game
    /// thread stages raw-copy state through it on refresh hours and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff. No other extended state
    /// crosses threads here.
    pub(crate) fn characters_pilot(&mut self) -> &mut characters::Captured {
        &mut self.characters
    }

    /// Decode-worker handoff for the projects family (copy/decode split).
    /// Sole cross-thread entry to the projects slot-3 buffers: the game
    /// thread stages raw-copy state through it on refresh hours and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn projects_pilot(&mut self) -> &mut projects::Captured {
        &mut self.projects
    }

    /// Decode-worker handoff for the lexical family (copy/decode split).
    /// Sole cross-thread entry to the lexical slot-3 buffers: the game
    /// thread stages raw-copy state through it on refresh hours and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn lexical_pilot(&mut self) -> &mut lexical::Captured {
        &mut self.lexical
    }

    /// Decode-worker handoff for the wing-fed air pair (copy/decode split).
    /// Sole cross-thread entry to the air wing-extension buffers: the game
    /// thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn air_wing_pilot(&mut self) -> &mut air::Captured {
        &mut self.air
    }

    /// Decode-worker handoff for the hourly air-statistics table (copy/decode
    /// split). Sole cross-thread entry to the statistics buffers: the game
    /// thread stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff. Shares the air pack buffers with
    /// the wing extensions; each stages its own copies with no cross-family
    /// decoded roots.
    pub(crate) fn air_stats_pilot(&mut self) -> &mut air::Captured {
        &mut self.air
    }

    /// Decode-worker handoff for the shared hourly bases payload
    /// (copy/decode split). Sole cross-thread entry to the air bases/aces
    /// buffers: the game thread stages raw-copy state through it every hour
    /// and the single FIFO decode worker decodes the staged payload into
    /// these pooled typed rows before the spool ready-queue handoff. The fog
    /// registrations half decodes from the same log (see below).
    pub(crate) fn air_bases_pilot(&mut self) -> &mut air::Captured {
        &mut self.air
    }

    /// Decode-worker handoff for the shared hourly history payload
    /// (copy/decode split). Sole cross-thread entry to the air history and
    /// damage buffers: the game thread stages raw-copy state through it
    /// every hour and the single FIFO decode worker decodes the staged
    /// payload into these pooled typed rows before the spool ready-queue
    /// handoff.
    pub(crate) fn air_history_pilot(&mut self) -> &mut air::Captured {
        &mut self.air
    }

    /// Decode-worker handoff for the source-fed naval block (copy/decode
    /// split). Sole cross-thread entry to the naval theatre/OOB buffers: the
    /// game thread stages raw-copy state through it every hour and the
    /// single FIFO decode worker decodes the staged payload into these
    /// pooled typed rows before the spool ready-queue handoff.
    pub(crate) fn naval_pilot(&mut self) -> &mut naval_history::Captured {
        &mut self.naval_history
    }

    /// Decode-worker handoff for the hourly session family (copy/decode
    /// split). Sole cross-thread entry to the session buffers: the game
    /// thread stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn session_pilot(&mut self) -> &mut session::Captured {
        &mut self.session
    }

    /// Decode-worker handoff for the hourly supremacy payload (copy/decode
    /// split). Sole cross-thread entry to the supremacy buffers: the game
    /// thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn supremacy_pilot(&mut self) -> &mut naval_history::Captured {
        &mut self.naval_history
    }

    /// Decode-worker handoff for the hourly sunk-ships payload (copy/decode
    /// split). Sole cross-thread entry to the sunk buffers: the game thread
    /// stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn sunk_pilot(&mut self) -> &mut naval_history::Captured {
        &mut self.naval_history
    }

    /// Decode-worker handoff for the hourly radar payload (copy/decode
    /// split). Sole cross-thread entry to the radar buffers: the game thread
    /// stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn radar_pilot(&mut self) -> &mut fog::Captured {
        &mut self.fog
    }

    /// Decode-worker handoff for the hourly intel payload (copy/decode
    /// split). Sole cross-thread entry to the intel buffers: the game thread
    /// stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn intel_pilot(&mut self) -> &mut fog::Captured {
        &mut self.fog
    }

    /// Decode-worker handoff for the hourly weather family (copy/decode
    /// split). Sole cross-thread entry to the weather buffers: the game
    /// thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn weather_pilot(&mut self) -> &mut weather::Captured {
        &mut self.weather
    }

    /// Decode-worker handoff for the hourly production rest (copy/decode
    /// split). Sole cross-thread entry to the production rest buffers: the
    /// game thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn production_rest_pilot(&mut self) -> &mut production::Captured {
        &mut self.production
    }

    /// Decode-worker handoff for the hourly construction rest (copy/decode
    /// split). Sole cross-thread entry to the construction rest buffers: the
    /// game thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn construction_rest_pilot(&mut self) -> &mut construction::Captured {
        &mut self.construction
    }

    /// Decode-worker handoff for the hourly economy rest (copy/decode split).
    /// Sole cross-thread entry to the economy rest buffers: the game thread
    /// stages raw-copy state through it every hour and the single FIFO decode
    /// worker decodes the staged payload into these pooled typed rows before
    /// the spool ready-queue handoff.
    pub(crate) fn economy_rest_pilot(&mut self) -> &mut economy::Captured {
        &mut self.economy
    }

    /// Decode-worker handoff for the hourly capitulation table (copy/decode
    /// split). Sole cross-thread entry to the capitulation buffers: the game
    /// thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff. Shares the intel pack
    /// buffers with the relations/rest families; each stages its own copies
    /// with no cross-family decoded roots.
    pub(crate) fn capitulation_pilot(&mut self) -> &mut intel::Captured {
        &mut self.intel
    }

    /// Decode-worker interpretation of a raw shared bases payload, in pack
    /// order: bases and aces rows first, then the fog registrations half,
    /// all from the same shared log. Disjoint borrows keep both packs alive
    /// at once.
    pub(crate) fn decode_air_bases_payload(
        &mut self,
        base: usize,
        job: &air::AirBasesJob,
        memo: &mut air::AirBasesMemo,
        context: air::AirBasesMemoContext,
        decode_ticks: &mut [u64; air::AIR_BASES_N],
    ) -> air::AirBasesMemoReport {
        debug_assert_eq!(base, context.base);
        memo.decode(&mut self.air, &mut self.fog, job, context, decode_ticks)
    }

    /// Which divisions this hour's `NewDivisionEquipment` covers.
    /// Whether the modifier definition table header (data pointer, count)
    /// changed since the previous hour; set before the Slow job is taken so
    /// the flag travels with the pack that walks it.
    pub(crate) fn set_modifier_definitions_moved(&mut self, moved: bool) {
        self.modifier_definitions.header_changed = moved;
    }

    pub(crate) fn set_new_division_mark(&mut self, mark: new_divisions::Mark) {
        self.new_divisions.set_mark(mark);
    }

    /// Suppress typed division detail reads while the owned acquisition supplies
    /// this hour's rows or terminal failure.
    pub(crate) fn set_divisions_raw_mode(&mut self, raw: bool) {
        self.production.divisions_raw_mode = raw;
    }

    /// The worker installs owned native rows without replaying pointer reads.
    pub(crate) fn install_division_detail(
        &mut self,
        details: &mut crate::division_detail::OwnedDetails,
    ) {
        details.install(&mut self.production);
    }

    /// Publish an accepted division job's failure after the ordinary stage.
    /// The raw-mode gate keeps its live reads suppressed while the original
    /// buffers remain available for off-thread recycling.
    pub(crate) fn fail_division_details(&mut self, reason: &'static str) {
        let reason = if self.production.division_details_active() {
            reason
        } else {
            replay::cadence::SKIPPED_REASON
        };
        let outcome = Outcome::Unavailable { reason };
        self.production.division_equipment.outcome = outcome;
        self.production.groups.outcome = outcome;
        self.production.requests.outcome = outcome;
        self.production.deliveries.outcome = outcome;
        self.production.produced.outcome = outcome;
    }

    pub(crate) fn division_capture_capacities(&self) -> (usize, [usize; 5]) {
        (
            self.divisions.first.len(),
            [
                self.production.division_equipment.rows.len(),
                self.production.groups.rows.len(),
                self.production.requests.rows.len(),
                self.production.deliveries.rows.len(),
                self.production.produced.rows.len(),
            ],
        )
    }

    /// Test-only buffer resize for the divisions-group decode harness:
    /// shrinks the pooled row buffers to the harness lengths, mirroring a
    /// truncated production frame. Capacities are untouched.
    #[cfg(test)]
    pub(crate) fn test_resize_division_buffers(&mut self, lens: [usize; 6]) {
        use engine::{army_upgrade_request_row, carmy_upgrades_requests, cupgrade_delivery};
        self.production.division_equipment.rows =
            vec![engine::cequipment_variant_pool::DivisionRow::default(); lens[1]];
        self.production.groups.rows = vec![carmy_upgrades_requests::Row::default(); lens[2]];
        self.production.requests.rows = vec![army_upgrade_request_row::Row::default(); lens[3]];
        self.production.deliveries.rows = vec![cupgrade_delivery::Row::default(); lens[4]];
        self.production.produced.rows = vec![cupgrade_delivery::ProducedRow::default(); lens[5]];
    }

    /// Test-only read of the production division-detail buffers for the
    /// divisions-group decode harness.
    #[cfg(test)]
    pub(crate) fn test_division_detail(&mut self) -> &mut production::Captured {
        &mut self.production
    }

    /// Loan the complete visitor until the original States collection point.
    /// Its allocation-free replacement is never used for the later flag read.
    pub(crate) fn take_state_visitor(&mut self) -> state_flags::Captured {
        std::mem::take(&mut self.state_flags)
    }
    pub(crate) fn restore_state_visitor(&mut self, visitor: state_flags::Captured) {
        self.state_flags = visitor;
    }

    /// Source copies remain inside the enclosing Divisions measurement. The
    /// shared read group includes equipment and upgrade tables without per-row
    /// timers or repeated source censuses for individual tables.
    pub(crate) fn visit_division_source(&mut self, source: DivisionSource) {
        self.divisions.visit_source(source);
    }
    #[cfg(test)]
    pub(crate) fn test_division_presentation_tables(
        &self,
    ) -> [replay::frame::TableView; division_presentation::N] {
        self.division_presentation.tables()
    }
    pub fn finish_divisions(&mut self, outcome: Outcome) -> u64 {
        self.divisions.finish(outcome);
        0
    }
    pub fn sources<'a, M: Memory>(
        &'a mut self,
        memory: &'a M,
        base: usize,
        air_raw: bool,
        naval_raw: bool,
    ) -> Sources<'a, M> {
        Sources {
            captured: self,
            memory,
            base,
            air_raw,
            naval_raw,
        }
    }
    /// Run after root division acquisition and map.sample. Returned arrays use
    /// global TableId indices; core slots remain zero. The registry census and
    /// variant details run together on their existing cadence.
    #[cfg(test)]
    pub(crate) fn capture_tables_all(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        force_full: bool,
    ) -> [u64; TABLE_COUNT] {
        // Synthetic direct-walk fixtures supply the same typed request that a
        // worker owns in production. Production assembly never acquires it.
        if self.ordinary_ticks.is_none() && self.ordinary_failure.is_none() {
            let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
            let mut request = self.take_ordinary_request(&countries, base, game_state);
            request.packs.capture(memory, base, game_state);
            self.restore_ordinary_request(request, None);
        }
        if self.presentation_report.is_none() {
            self.presentation_report = Some(self.division_presentation.capture_owned(
                memory,
                division_presentation::CaptureContext {
                    base,
                    game_state,
                    hour: current_hour,
                    force_full,
                },
                unit_sources(&self.unit_inputs),
                None,
            ));
        }
        let mut ticks = self.capture_tables(memory, base, game_state, current_hour, force_full);
        self.capture_tables_late(
            memory,
            base,
            game_state,
            current_hour,
            force_full,
            &mut Orders::vacant(),
            &mut ticks,
        );
        ticks
    }

    #[cfg(test)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        force_full: bool,
    ) -> [u64; TABLE_COUNT] {
        let mut ticks = self.capture_tables_all(memory, base, game_state, current_hour, force_full);
        let ships = TableId::ShipModels.index();
        ticks[ships] = self.capture_ship_models(memory, base);
        self.decode_division_presentation_inputs();
        ticks
    }

    /// All frame-owned acquisition except ShipModels, which the callback joins
    /// at the end of its parallel region.
    pub(crate) fn capture_tables(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        force_full: bool,
    ) -> [u64; TABLE_COUNT] {
        let mut ticks = [0; TABLE_COUNT];
        macro_rules! ordinary {
            ($($pack:ident),+ $(,)?) => {$(
                let cost = self.$pack.capture(memory, base, game_state);
                scatter(&$pack::TABLES, cost, &mut ticks);
            )+};
        }
        macro_rules! staggered {
            ($($pack:ident),+ $(,)?) => {$(
                let cost = self.$pack.capture(memory, base, game_state, current_hour, force_full);
                scatter(&$pack::TABLES, cost, &mut ticks);
            )+};
        }
        // StateFlags needs the States rows supplied first: it stays inline.
        ordinary!(weather, state_flags);
        if let Some(done) = self.ordinary_ticks.take() {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            self.ordinary_failure = Some("ordinary acquisition not supplied");
        }
        // Keep the completion marker after consuming its cost: accepted
        // failure cannot become an inline retry on a second assembly call.
        if let Some(done) = self.slow_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            staggered!(
                map_names,
                division_templates,
                visibility,
                faction_theatres,
                model_inputs,
                modifier_definitions,
                hired_advisors,
                character_inputs,
                country_tactic_unlocks
            );
            // No loaned fingerprint store: only the hour's slice is read.
            let cost = self.country_stability.capture(
                memory,
                base,
                game_state,
                current_hour,
                force_full,
                None,
            );
            scatter(&country_stability::TABLES, cost, &mut ticks);
        }
        if let Some(done) = self.raid_state_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            staggered!(raid_state_inputs);
        }
        if let Some(done) = self.raid_targets_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            staggered!(raid_target_statuses);
        }
        if let Some(done) = self.raid_launch_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            staggered!(raid_launch_inputs);
        }
        // The parts' copies are decoded later, off the game thread.
        let cost = if self.air_relations.staged() {
            self.air_relations.ticks()
        } else {
            self.air_relations.unstaged()
        };
        scatter(&air_relations::TABLES, cost, &mut ticks);
        if let Some(done) = self.gated_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            staggered!(fog, construction, projects);
        }
        ticks[TableId::StateBuildings.index()] =
            self.construction.finish_buildings(current_hour, force_full);
        let province_order_report = self.acquire_province_unit_order(
            memory,
            province_unit_order::CaptureContext { base, game_state },
        );
        scatter(
            &province_unit_order::TABLES,
            province_order_report.ticks,
            &mut ticks,
        );
        let portraits_report = self.acquire_character_portraits(
            memory,
            character_portraits::CaptureContext { base, game_state },
        );
        scatter(
            &character_portraits::TABLES,
            portraits_report.ticks,
            &mut ticks,
        );
        let supply_report =
            self.acquire_supply_trains(memory, supply_trains::CaptureContext { base, game_state });
        scatter(&supply_trains::TABLES, supply_report.ticks, &mut ticks);
        staggered!(economy, session, characters);
        let intel_context = intel::CaptureContext {
            base,
            game_state,
            hour: current_hour,
            force_full,
        };
        let intel_report = self.acquire_intel(memory, intel_context);
        scatter(&intel::TABLES, intel_report.ticks, &mut ticks);
        let governance_report = self.governance_report.take().unwrap_or_else(|| {
            self.governance.fail_owned_capture(
                governance::CaptureContext {
                    base,
                    game_state,
                    hour: current_hour,
                    force_full,
                },
                "governance acquisition not supplied",
            )
        });
        scatter(&governance::TABLES, governance_report.ticks, &mut ticks);
        ticks
    }

    /// The rest of the hour's tables, after the late pool job is back: its
    /// packs' ticks when it ran, the same walks inline when it did not.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_tables_late(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        force_full: bool,
        orders: &mut Orders,
        ticks: &mut [u64; TABLE_COUNT],
    ) {
        if let Some(done) = self.late_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            let mut packs = self.late_spare.take().expect("late spare present");
            self.swap_late(&mut packs, orders);
            // Without a loaned held table, RaidAirScores reads every pair.
            packs.capture(
                memory,
                base,
                game_state,
                current_hour,
                force_full,
                self.unit_inputs.as_deref(),
                &mut engine::raid_air_scores::Held::default(),
            );
            for (total, cost) in ticks.iter_mut().zip(packs.ticks) {
                *total += cost;
            }
            self.swap_late(&mut packs, orders);
            self.late_spare = Some(packs);
        }
        // The parts' copies are decoded later, off the game thread.
        let cost = if self.air_histories.staged() {
            self.air_histories.ticks()
        } else {
            self.air_histories.unstaged()
        };
        scatter(&air_histories::TABLES, cost, ticks);
        scatter(&plans::TABLES, self.plans.capture(), ticks);
        let ticks = &mut *ticks;
        let cost = self.production.acquire(
            memory,
            base,
            game_state,
            self.divisions.sources(),
            current_hour,
            force_full,
        );
        scatter(&production::TABLES, cost, ticks);
        let divisions = self.divisions.sources();
        let cost = self
            .new_divisions
            .capture(memory, base, divisions.rows, divisions.outcome);
        scatter(&new_divisions::TABLES, cost, ticks);
        let report = self.acquire_presentation();
        scatter(&division_presentation::TABLES, report.ticks, ticks);
        let eta_inputs_report = self.acquire_eta_inputs();
        scatter(&division_eta_inputs::TABLES, eta_inputs_report.ticks, ticks);
        let cost = self.variants_ticks.take().unwrap_or_else(|| {
            let mut variants = self.production.take_variants();
            variants.fail("variant acquisition not supplied", current_hour, force_full);
            self.production.restore_variants(variants);
            [0; production::N]
        });
        for (index, table) in production::TABLES.iter().enumerate() {
            ticks[table.index()] = ticks[table.index()].saturating_add(cost[index]);
        }
        let cost = self
            .lexical
            .capture(memory, base, game_state, current_hour, force_full);
        scatter(&lexical::TABLES, cost, ticks);
        for table in TABLES {
            if table == TableId::ShipModels {
                continue;
            }
            ticks[table.index()] =
                ticks[table.index()].saturating_add(self.overhead[table.index()]);
        }
    }

    /// Finish the ShipModels family after its owned detail request has had the
    /// whole independent callback region to run. A rejected request is
    /// executed exactly once by the caller before this function. An accepted
    /// panic/stop is terminal: the restored report suppresses any detail retry.
    pub(crate) fn capture_ship_models(&mut self, memory: &impl Memory, base: usize) -> u64 {
        let context = ship_models::CaptureContext { base };
        let report = self.acquire_ship_models(memory, context);
        let table = TableId::ShipModels;
        if self.ship_models.has_terminal_failure() {
            // Interrupted jobs have no complete acquisition duration.
            0
        } else {
            report.ticks[0].saturating_add(self.overhead[table.index()])
        }
    }
}

pub struct Sources<'a, M: Memory> {
    captured: &'a mut Captured,
    memory: &'a M,
    base: usize,
    air_raw: bool,
    naval_raw: bool,
}
impl<M: Memory> Sources<'_, M> {
    fn wing_ticks(&self) -> u64 {
        self.captured.air.wing_ticks().into_iter().sum::<u64>()
            + self.captured.naval_history.source_ticks()[1]
    }
    fn attribute_overhead(&mut self, table: TableId, begin: i64, separately_measured: u64) -> u64 {
        let total = elapsed(begin);
        self.captured.overhead[table.index()] = self.captured.overhead[table.index()]
            .saturating_add(total.saturating_sub(separately_measured));
        total
    }
}
impl<M: Memory> MapSources for Sources<'_, M> {
    fn begin_land(&mut self) -> u64 {
        let begin = qpc();
        self.captured.combat_progress.begin();
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
    fn land(&mut self, source: engine::cland_combat::Source) -> u64 {
        let begin = qpc();
        self.captured
            .combat_progress
            .visit(self.memory, self.base, source);
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
    fn finish_land(&mut self, outcome: Outcome) -> u64 {
        let begin = qpc();
        self.captured.combat_progress.finish(outcome);
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
    fn begin_air(&mut self) -> u64 {
        if self.air_raw {
            return 0;
        }
        let begin = qpc();
        self.captured.air.begin_wings();
        self.attribute_overhead(TableId::WingEquipment, begin, 0)
    }
    fn air(&mut self, pointer: usize, pool: usize, row: &cair_wing::Row) -> u64 {
        if self.air_raw && self.naval_raw {
            return 0;
        }
        let begin = qpc();
        let before = self.wing_ticks();
        if !self.air_raw {
            self.captured.air.visit_wing(
                self.memory,
                self.base,
                air::WingSource { pointer, pool, row },
            );
        }
        if !self.naval_raw {
            self.captured
                .naval_history
                .visit_wing(self.memory, pointer, row);
        }
        let charged = self.wing_ticks().saturating_sub(before);
        self.attribute_overhead(TableId::WingEquipment, begin, charged)
    }
    fn finish_air(&mut self, outcome: Outcome) -> u64 {
        if self.air_raw && self.naval_raw {
            return 0;
        }
        let begin = qpc();
        let before = self.wing_ticks();
        if !self.air_raw {
            self.captured.air_root = Some(outcome);
        }
        if !self.naval_raw {
            self.captured.naval_history.finish_wings(outcome);
        }
        let charged = self.wing_ticks().saturating_sub(before);
        self.attribute_overhead(TableId::WingEquipment, begin, charged)
    }
}

/// Whether a table was selected but not published (skipped is not a read).
fn unpublished(outcomes: &[Outcome]) -> bool {
    outcomes.iter().any(|outcome| {
        !matches!(outcome, Outcome::Rows { .. })
            && !matches!(outcome, Outcome::Unavailable { reason } if *reason == replay::cadence::SKIPPED_REASON)
    })
}

fn unit_sources(inputs: &Option<Arc<UnitInputs>>) -> production::Sources<'_, carmy::DivisionSeed> {
    let inputs = inputs.as_deref();
    production::Sources {
        rows: inputs.map_or(&[], UnitInputs::seeds),
        outcome: inputs.map_or(MISSING, UnitInputs::outcome),
    }
}

fn scatter<T: Copy, const N: usize>(
    tables: &[TableId; N],
    local: [T; N],
    global: &mut [T; TABLE_COUNT],
) {
    for (table, value) in tables.iter().zip(local) {
        global[table.index()] = value;
    }
}
fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

pub(crate) struct Divisions {
    first: Vec<DivisionSource>,
    seen: usize,
    outcome: Outcome,
    required: usize,
}
impl Divisions {
    fn new(capacity: usize) -> Self {
        Self {
            first: vec![Default::default(); capacity],
            seen: 0,
            outcome: MISSING,
            required: 0,
        }
    }
    fn reset(&mut self) {
        self.seen = 0;
        self.outcome = MISSING;
        self.required = 0;
    }
    fn visit_source(&mut self, source: DivisionSource) {
        if let Some(slot) = self.first.get_mut(self.seen) {
            *slot = source;
        }
        self.seen = self.seen.saturating_add(1);
        self.required = self.required.max(self.seen);
    }
    fn finish(&mut self, outcome: Outcome) {
        let capacity = self.first.len();
        self.outcome = match outcome {
            Outcome::Rows { len } if len != self.seen => Outcome::Unavailable {
                reason: "division visitor and source row count disagree",
            },
            Outcome::Rows { len } if len > capacity => truncated_source(capacity, len - capacity),
            Outcome::Truncated { len, unread_rows } => {
                self.required = self.required.max(len.saturating_add(unread_rows as usize));
                let stored = len.min(capacity);
                truncated_source(
                    stored,
                    len.saturating_sub(stored)
                        .saturating_add(unread_rows as usize),
                )
            }
            _ => outcome,
        };
    }
    fn sources(&self) -> production::Sources<'_, DivisionSource> {
        production::Sources {
            rows: &self.first,
            outcome: self.outcome,
        }
    }
    fn grow(&mut self, capacity: &mut usize) {
        grow_capacity(capacity, self.required);
        grow_vec(&mut self.first, *capacity);
    }
    fn allocated_bytes(&self) -> usize {
        self.first.capacity() * size_of::<DivisionSource>()
    }
}
fn truncated_source(len: usize, omitted: usize) -> Outcome {
    match u32::try_from(omitted) {
        Ok(unread_rows) if unread_rows > 0 => Outcome::Truncated { len, unread_rows },
        _ => Outcome::Unavailable {
            reason: "source omitted row count invalid or exceeds u32",
        },
    }
}

pub(crate) struct Registry {
    rows: CaptureRows<cequipment_variant::RegistryRow>,
}
impl Registry {
    fn new(capacity: usize) -> Self {
        Self {
            rows: CaptureRows::new(capacity),
        }
    }
    fn reset(&mut self) {
        self.rows.reset();
    }
    pub(crate) fn acquire(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        let _reads = crate::read_accounting::table(TableId::VariantDefinitions);
        self.rows.outcome = bounded_registry(
            cequipment_variant::walk_registry(memory, base, game_state, &mut self.rows.rows),
            self.rows.rows.len(),
        );
    }
    // Off-hours carry-forward hint keeps byte-identical spool output.
    pub(crate) fn mark_skipped(&mut self) {
        self.rows.outcome = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
    }
    pub(crate) fn sources(&self) -> production::Sources<'_, cequipment_variant::RegistryRow> {
        production::Sources {
            rows: &self.rows.rows,
            outcome: self.rows.outcome,
        }
    }
    fn grow(&mut self, capacity: &mut usize) {
        *capacity = (*capacity).max(self.rows.rows.capacity());
        self.rows.grow(capacity);
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub(crate) fn fail(&mut self, reason: &'static str) {
        self.rows.outcome = Outcome::Unavailable { reason };
    }
}
fn bounded_registry(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "variant registry count exceeds buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "variant registry truncation has no omitted rows",
        }
    } else {
        outcome
    }
}
#[cfg(test)]
mod tests;
