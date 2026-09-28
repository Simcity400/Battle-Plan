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
    frame::{BaselineToken, TABLE_COUNT, TableInput},
    rows::{ScriptVariablesRow, TableId},
};
use std::{mem::size_of, sync::Arc};

#[cfg(test)]
pub(crate) mod bench_world;
pub(crate) mod daily_intel;

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
    (lexical, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($lexical)
    };
    (characters, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($characters)
    };
    (character_portraits, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    (leader_portraits, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    (ship_models, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    (intel, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($baseline, $scratch, $intel)
    };
    (governance, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs()
    };
    (division_presentation, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    (production, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    (economy, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    (fog, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    (supply_trains, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    ($pack:ident, $captured:expr, $baseline:expr, $scratch:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
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
            late_ticks: Option<[u64; TABLE_COUNT]>,
            late_spare: Option<Box<LatePacks>>,
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
                    late_ticks: None,
                    late_spare: Some(LatePacks::spare()),
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
            #[allow(clippy::too_many_arguments)]
            pub(crate) fn insert_tables(
                &self,
                destination: &mut [TableInput; TABLE_COUNT],
                variables_baseline: Option<(BaselineToken, &[ScriptVariablesRow])>,
                variables_scratch: &mut intel::ScriptVariableComparison,
                characters: Option<&mut characters::CharacterConversion<'_>>,
                lexical: Option<&mut lexical::LexicalConversion<'_>>,
                intel: Option<&mut intel::IntelConversion<'_>>,
                mut projection: Option<&mut crate::projection::Projection<'_>>,
            ) -> [i64; PACK_NAMES.len()] {
                let mut costs = [0; PACK_NAMES.len()];
                let mut slots = costs.iter_mut();
                $(let started = qpc();
                for (id, view) in $pack::TABLES.into_iter().zip(spool_inputs!($pack, self.$pack, variables_baseline, variables_scratch, characters, lexical, intel, projection)) { destination[id.index()] = view; }
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
            groups.push(("army order census, plan operands, group plans and front sections", TableId::ArmyOrders,
                vec![TableId::ArmyOrders, TableId::ArmyOrderPlans, TableId::OrdersGroupPlans, TableId::Fronts]));
            groups
        }
    };
}
packs!(
    air,
    army_icon_inputs,
    naval_counters,
    railway_guns,
    active_abilities,
    country_counters,
    group_counters,
    leader_paths,
    country_appearance,
    ship_models,
    province_unit_order,
    naval_bases,
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
    building_detection,
    slice_reads,
    faction_theatres,
    model_inputs,
    modifier_definitions,
    hired_advisors,
    country_stability,
    new_divisions,
    weather
);

impl Capacities {
    /// Presize every table whose rows scale with the number of country slots,
    /// so a large mod's first hour is not truncated. Never lowers a capacity.
    pub fn raise_for_countries(&mut self, slots: usize) {
        self.country_appearance.raise_for_countries(slots);
        self.country_counters.raise_for_countries(slots);
        self.economy.raise_for_countries(slots);
        self.governance.raise_for_countries(slots);
        self.intel.raise_for_countries(slots);
        self.session.raise_for_countries(slots);
        self.projects.raise_for_countries(slots);
        self.supply_trains.raise_for_countries(slots);
        self.construction.raise_for_countries(slots);
        self.visibility.raise_for_countries(slots);
        self.building_detection.raise_for_countries(slots);
        self.model_inputs.raise_for_countries(slots);
        self.hired_advisors.raise_for_countries(slots);
        self.country_stability.raise_for_countries(slots);
    }

    /// Presize CountryIntel from the live dense record total.
    pub fn raise_for_country_intel(&mut self, native_rows: usize) {
        self.intel.raise_for_country_intel(native_rows);
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
        self.country_stability.prepare_hour(hour, force_full);
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

    pub(crate) fn take_presentation_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
    ) -> crate::acquisition_jobs::PresentationRequest {
        self.presentation_report = None;
        crate::acquisition_jobs::PresentationRequest {
            countries: Arc::clone(countries),
            context: division_presentation::CaptureContext {
                base,
                game_state: state,
            },
            inputs: self.unit_inputs.as_ref().map(Arc::clone),
            captured: std::mem::replace(
                &mut self.division_presentation,
                division_presentation::Captured::vacant(),
            ),
        }
    }

    pub(crate) fn restore_presentation_request(
        &mut self,
        mut request: crate::acquisition_jobs::PresentationRequest,
        report: Result<division_presentation::CaptureReport, crate::acquisition::Failure>,
    ) {
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

    /// One sliced family's read at this frame's hour (`slicing::FAMILIES`).
    pub(crate) fn family_selection(&self, family: usize) -> &crate::slicing::SliceSelection {
        match family {
            crate::slicing::RESEARCH => self.governance.research_selection(),
            crate::slicing::TECHNOLOGIES => self.governance.technology_selection(),
            crate::slicing::RELATIONS => self.intel.relations_selection(),
            crate::slicing::STABILITY => self.country_stability.selection(),
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
                .late_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
            + self
                .gated_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
    }

    /// Lend the ordinary packs to the pool: swap them with the spare set so
    /// nothing allocates on the game thread.
    pub(crate) fn take_ordinary_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
    ) -> crate::acquisition_jobs::OrdinaryRequest {
        self.ordinary_ticks = None;
        self.ordinary_failure = None;
        let mut packs = self.ordinary_spare.take().expect("ordinary spare present");
        self.swap_ordinary(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        packs.failure = None;
        crate::acquisition_jobs::OrdinaryRequest {
            countries: Arc::clone(countries),
            base,
            game_state: state,
            packs,
        }
    }

    /// Every terminal job installs a completion marker, including failure, so
    /// table assembly cannot retry an allocating walk on the callback.
    pub(crate) fn restore_ordinary_request(
        &mut self,
        request: crate::acquisition_jobs::OrdinaryRequest,
        failure: Option<&'static str>,
    ) {
        let mut packs = request.packs;
        if let Some(reason) = failure {
            packs.fail(reason);
        }
        self.ordinary_ticks = Some(packs.ticks);
        self.ordinary_failure = packs.failure;
        self.swap_ordinary(&mut packs);
        self.ordinary_spare = Some(packs);
    }

    fn swap_ordinary(&mut self, packs: &mut OrdinaryPacks) {
        std::mem::swap(&mut self.combat_details, &mut packs.combat_details);
        std::mem::swap(&mut self.naval_details, &mut packs.naval_details);
        std::mem::swap(
            &mut self.naval_result_details,
            &mut packs.naval_result_details,
        );
        std::mem::swap(&mut self.country_appearance, &mut packs.country_appearance);
        std::mem::swap(&mut self.naval_bases, &mut packs.naval_bases);
        std::mem::swap(&mut self.naval_counters, &mut packs.naval_counters);
        std::mem::swap(&mut self.railway_guns, &mut packs.railway_guns);
        std::mem::swap(&mut self.active_abilities, &mut packs.active_abilities);
        std::mem::swap(&mut self.country_counters, &mut packs.country_counters);
        std::mem::swap(&mut self.group_counters, &mut packs.group_counters);
    }

    pub(crate) fn take_slow_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        stability_fingerprints: Option<Box<crate::slicing::SliceFingerprints>>,
    ) -> crate::acquisition_jobs::SlowRequest {
        self.slow_ticks = None;
        let mut packs = self.slow_spare.take().expect("slow spare present");
        self.swap_slow(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::SlowRequest {
            countries: Arc::clone(countries),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
            stability_fingerprints,
        }
    }

    /// Returns the stability family's fingerprint store: forgotten if the job
    /// failed, and with this hour's selection invalidated if a stability
    /// table was not read completely, so its countries are read again at the
    /// next hour.
    pub(crate) fn restore_slow_request(
        &mut self,
        request: crate::acquisition_jobs::SlowRequest,
        completed: bool,
    ) -> Option<Box<crate::slicing::SliceFingerprints>> {
        let mut packs = request.packs;
        let mut fingerprints = request.stability_fingerprints;
        if !completed {
            packs.fail(request.hour, request.force_full, "slow acquisition failed");
            if let Some(fingerprints) = fingerprints.as_mut() {
                fingerprints.forget();
            }
        }
        if let Some(fingerprints) = fingerprints.as_mut()
            && packs.country_stability.outcomes().iter().any(|outcome| {
                !matches!(outcome, Outcome::Rows { .. })
                    && !matches!(outcome, Outcome::Unavailable { reason } if *reason == replay::cadence::SKIPPED_REASON)
            })
        {
            packs.country_stability.selection().invalidate(fingerprints);
        }
        self.slow_ticks = Some(packs.ticks);
        self.swap_slow(&mut packs);
        self.slow_spare = Some(packs);
        fingerprints
    }

    fn swap_slow(&mut self, packs: &mut SlowPacks) {
        std::mem::swap(&mut self.map_names, &mut packs.map_names);
        std::mem::swap(&mut self.division_templates, &mut packs.division_templates);
        std::mem::swap(&mut self.visibility, &mut packs.visibility);
        std::mem::swap(&mut self.building_detection, &mut packs.building_detection);
        std::mem::swap(&mut self.faction_theatres, &mut packs.faction_theatres);
        std::mem::swap(&mut self.model_inputs, &mut packs.model_inputs);
        std::mem::swap(
            &mut self.modifier_definitions,
            &mut packs.modifier_definitions,
        );
        std::mem::swap(&mut self.hired_advisors, &mut packs.hired_advisors);
        std::mem::swap(&mut self.country_stability, &mut packs.country_stability);
    }

    /// Loan the order census and the plans to the Orders job.
    pub(crate) fn take_orders_request(
        &mut self,
        orders: &mut Orders,
        base: usize,
    ) -> crate::acquisition_jobs::OrdersRequest {
        crate::acquisition_jobs::OrdersRequest {
            base,
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

    /// Loan the late packs and the walked order census to the job.
    /// `naval_raw`: the Naval raw job staged the order-of-battle sources.
    pub(crate) fn take_late_request(
        &mut self,
        orders: &mut Orders,
        naval_raw: bool,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> crate::acquisition_jobs::LateRequest {
        self.late_ticks = None;
        let mut packs = self.late_spare.take().expect("late spare present");
        self.swap_late(&mut packs, orders);
        packs.naval_raw = naval_raw;
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::LateRequest {
            inputs: self.unit_inputs.as_ref().map(Arc::clone),
            base,
            game_state: state,
            hour,
            force_full,
            packs,
        }
    }

    pub(crate) fn restore_late_request(
        &mut self,
        request: crate::acquisition_jobs::LateRequest,
        completed: bool,
        orders: &mut Orders,
    ) {
        let mut packs = request.packs;
        if !completed {
            packs.fail(request.hour, request.force_full, "late acquisition failed");
        }
        self.late_ticks = Some(packs.ticks);
        self.swap_late(&mut packs, orders);
        self.late_spare = Some(packs);
    }

    /// Loan the raw-gated packs once every raw-copy verdict is in.
    pub(crate) fn take_gated_request(
        &mut self,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> crate::acquisition_jobs::GatedRequest {
        self.gated_ticks = None;
        let mut packs = self.gated_spare.take().expect("gated spare present");
        self.swap_gated(&mut packs);
        packs.ticks = [0; TABLE_COUNT];
        crate::acquisition_jobs::GatedRequest {
            base,
            game_state: state,
            hour,
            force_full,
            packs,
        }
    }

    pub(crate) fn restore_gated_request(
        &mut self,
        request: crate::acquisition_jobs::GatedRequest,
        completed: bool,
    ) {
        let mut packs = request.packs;
        if !completed {
            packs.fail(request.hour, request.force_full, "gated acquisition failed");
        }
        self.gated_ticks = Some(packs.ticks);
        self.swap_gated(&mut packs);
        self.gated_spare = Some(packs);
    }

    fn swap_gated(&mut self, packs: &mut GatedPacks) {
        std::mem::swap(&mut self.fog, &mut packs.fog);
        std::mem::swap(&mut self.projects, &mut packs.projects);
        std::mem::swap(&mut self.construction, &mut packs.construction);
    }

    fn swap_late(&mut self, packs: &mut LatePacks, orders: &mut Orders) {
        std::mem::swap(&mut self.leader_portraits, &mut packs.leader_portraits);
        std::mem::swap(&mut self.leader_paths, &mut packs.leader_paths);
        std::mem::swap(&mut self.air, &mut packs.air);
        std::mem::swap(&mut self.naval_history, &mut packs.naval_history);
        std::mem::swap(&mut self.army_icon_inputs, &mut packs.army_icon_inputs);
        std::mem::swap(&mut self.divisions, &mut packs.divisions);
        std::mem::swap(&mut self.air_root, &mut packs.air_root);
        std::mem::swap(orders, &mut packs.orders);
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
    pub(crate) fn test_division_presentation_tables(&self) -> [replay::frame::TableView; 2] {
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
                division_presentation::CaptureContext { base, game_state },
                unit_sources(&self.unit_inputs),
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
                building_detection,
                faction_theatres,
                model_inputs,
                modifier_definitions,
                hired_advisors
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
            packs.capture(
                memory,
                base,
                game_state,
                current_hour,
                force_full,
                self.unit_inputs.as_deref(),
            );
            for (total, cost) in ticks.iter_mut().zip(packs.ticks) {
                *total += cost;
            }
            self.swap_late(&mut packs, orders);
            self.late_spare = Some(packs);
        }
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
    fn air(&mut self, pointer: usize, _: usize, row: &cair_wing::Row) -> u64 {
        if self.air_raw && self.naval_raw {
            return 0;
        }
        let begin = qpc();
        let before = self.wing_ticks();
        if !self.air_raw {
            self.captured
                .air
                .visit_wing(self.memory, self.base, air::WingSource { pointer, row });
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

/// The map counter inputs that need only live memory: one pool job from the
/// first dispatch burst.
pub(crate) struct OrdinaryPacks {
    pub(crate) combat_details: combat_details::Captured,
    pub(crate) naval_details: naval_details::Captured,
    pub(crate) naval_result_details: naval_result_details::Captured,
    pub(crate) country_appearance: country_appearance::Captured,
    pub(crate) naval_bases: naval_bases::Captured,
    pub(crate) naval_counters: naval_counters::Captured,
    pub(crate) railway_guns: railway_guns::Captured,
    pub(crate) active_abilities: active_abilities::Captured,
    pub(crate) country_counters: country_counters::Captured,
    pub(crate) group_counters: group_counters::Captured,
    pub(crate) ticks: [u64; TABLE_COUNT],
    failure: Option<&'static str>,
}

impl OrdinaryPacks {
    const TABLES: &'static [&'static [TableId]] = &[
        &combat_details::TABLES,
        &naval_details::TABLES,
        &naval_result_details::TABLES,
        &country_appearance::TABLES,
        &naval_bases::TABLES,
        &naval_counters::TABLES,
        &railway_guns::TABLES,
        &active_abilities::TABLES,
        &country_counters::TABLES,
        &group_counters::TABLES,
    ];

    pub(crate) fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.combat_details.allocated_bytes()
            + self.naval_details.allocated_bytes()
            + self.naval_result_details.allocated_bytes()
            + self.country_appearance.allocated_bytes()
            + self.naval_bases.allocated_bytes()
            + self.naval_counters.allocated_bytes()
            + self.railway_guns.allocated_bytes()
            + self.active_abilities.allocated_bytes()
            + self.country_counters.allocated_bytes()
            + self.group_counters.allocated_bytes()
    }

    pub(crate) fn fail(&mut self, reason: &'static str) {
        self.combat_details.fail(reason);
        self.naval_details.reset();
        self.naval_result_details.reset();
        self.country_appearance.reset();
        self.naval_bases.reset();
        self.naval_counters.reset();
        self.railway_guns.reset();
        self.active_abilities.reset();
        self.country_counters.reset();
        self.group_counters.reset();
        self.ticks = [0; TABLE_COUNT];
        self.failure = Some(reason);
    }
    /// Placeholders the extended capture holds while the pool owns the real
    /// packs. Only the reusable box allocates; the hourly exchange is a swap.
    fn spare() -> Box<Self> {
        Box::new(Self {
            combat_details: combat_details::Captured::vacant(),
            naval_details: naval_details::Captured::vacant(),
            naval_result_details: naval_result_details::Captured::vacant(),
            country_appearance: country_appearance::Captured::vacant(),
            naval_bases: naval_bases::Captured::vacant(),
            naval_counters: naval_counters::Captured::vacant(),
            railway_guns: railway_guns::Captured::vacant(),
            active_abilities: active_abilities::Captured::vacant(),
            country_counters: country_counters::Captured::vacant(),
            group_counters: group_counters::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
            failure: None,
        })
    }

    pub(crate) fn capture(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        let mut ticks = [0; TABLE_COUNT];
        macro_rules! pack {
            ($($pack:ident),+ $(,)?) => {$(
                scatter(&$pack::TABLES, self.$pack.capture(memory, base, game_state), &mut ticks);
            )+};
        }
        pack!(
            combat_details,
            naval_details,
            naval_result_details,
            country_appearance,
            naval_bases,
            naval_counters,
            railway_guns,
            active_abilities,
            country_counters,
            group_counters
        );
        self.ticks = ticks;
    }
}

/// The live-memory packs run as a second pool job from the first burst
/// (map names, templates, visibility, building detection,
/// faction theatres, model inputs, modifier definitions and hired advisors). No defines are recorded: the viewer reads the movement
/// defines from the content's `NDefines`. Each table keeps its own [`replay::cadence`]: the
/// template dictionary is daily, its counter inputs hourly.
pub(crate) struct SlowPacks {
    pub(crate) map_names: map_names::Captured,
    pub(crate) division_templates: division_templates::Captured,
    pub(crate) visibility: visibility::Captured,
    pub(crate) building_detection: building_detection::Captured,
    pub(crate) faction_theatres: faction_theatres::Captured,
    pub(crate) model_inputs: model_inputs::Captured,
    pub(crate) modifier_definitions: modifier_definitions::Captured,
    pub(crate) hired_advisors: hired_advisors::Captured,
    pub(crate) country_stability: country_stability::Captured,
    pub(crate) ticks: [u64; TABLE_COUNT],
}

impl SlowPacks {
    fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.map_names.fail_capture(hour, force_full, reason);
        self.division_templates
            .fail_capture(hour, force_full, reason);
        self.visibility.fail_capture(hour, force_full, reason);
        self.building_detection
            .fail_capture(hour, force_full, reason);
        self.faction_theatres.fail_capture(hour, force_full, reason);
        self.model_inputs.fail_capture(hour, force_full, reason);
        self.modifier_definitions
            .fail_capture(hour, force_full, reason);
        self.hired_advisors.fail_capture(hour, force_full, reason);
        self.country_stability
            .fail_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.map_names.allocated_bytes()
            + self.division_templates.allocated_bytes()
            + self.visibility.allocated_bytes()
            + self.building_detection.allocated_bytes()
            + self.faction_theatres.allocated_bytes()
            + self.model_inputs.allocated_bytes()
            + self.modifier_definitions.allocated_bytes()
            + self.hired_advisors.allocated_bytes()
            + self.country_stability.allocated_bytes()
    }

    fn spare() -> Box<Self> {
        Box::new(Self {
            map_names: map_names::Captured::vacant(),
            division_templates: division_templates::Captured::vacant(),
            visibility: visibility::Captured::vacant(),
            building_detection: building_detection::Captured::vacant(),
            faction_theatres: faction_theatres::Captured::vacant(),
            model_inputs: model_inputs::Captured::vacant(),
            modifier_definitions: modifier_definitions::Captured::vacant(),
            hired_advisors: hired_advisors::Captured::vacant(),
            country_stability: country_stability::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        stability_fingerprints: Option<&mut crate::slicing::SliceFingerprints>,
    ) {
        let mut ticks = [0; TABLE_COUNT];
        macro_rules! staggered {
            ($($pack:ident),+ $(,)?) => {$(
                scatter(
                    &$pack::TABLES,
                    self.$pack.capture(memory, base, game_state, hour, force_full),
                    &mut ticks,
                );
            )+};
        }
        staggered!(
            map_names,
            division_templates,
            visibility,
            building_detection,
            faction_theatres,
            model_inputs,
            modifier_definitions,
            hired_advisors
        );
        let cost = self.country_stability.capture(
            memory,
            base,
            game_state,
            hour,
            force_full,
            stability_fingerprints,
        );
        scatter(&country_stability::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

/// The typed walks seeded by the map sample (wings, order sources, HQ portraits) and the
/// division census, moved off the game thread as a second pool job dispatched
/// once the map sample has run.
pub(crate) struct LatePacks {
    pub(crate) leader_portraits: leader_portraits::Captured,
    pub(crate) leader_paths: leader_paths::Captured,
    pub(crate) air: air::Captured,
    pub(crate) naval_history: naval_history::Captured,
    pub(crate) army_icon_inputs: army_icon_inputs::Captured,
    pub(crate) divisions: Divisions,
    pub(crate) air_root: Option<Outcome>,
    /// The walked order census; the naval order sources read its roots.
    pub(crate) orders: Orders,
    /// The Naval raw job staged the order-of-battle sources itself.
    pub(crate) naval_raw: bool,
    pub(crate) ticks: [u64; TABLE_COUNT],
}

impl LatePacks {
    fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.leader_portraits.fail_capture(hour, force_full, reason);
        self.leader_paths.fail_capture(hour, force_full, reason);
        self.air.fail_late_capture(hour, force_full, reason);
        self.naval_history
            .fail_late_capture(hour, force_full, reason);
        self.army_icon_inputs.fail_capture(reason);
        self.ticks = [0; TABLE_COUNT];
    }

    fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.leader_portraits.allocated_bytes()
            + self.leader_paths.allocated_bytes()
            + self.air.allocated_bytes()
            + self.naval_history.allocated_bytes()
            + self.army_icon_inputs.allocated_bytes()
            + self.divisions.allocated_bytes()
            + self.orders.allocated_bytes()
    }

    fn spare() -> Box<Self> {
        Box::new(Self {
            leader_portraits: leader_portraits::Captured::vacant(),
            leader_paths: leader_paths::Captured::vacant(),
            air: air::Captured::vacant(),
            naval_history: naval_history::Captured::vacant(),
            army_icon_inputs: army_icon_inputs::Captured::vacant(),
            divisions: Divisions::new(0),
            air_root: None,
            orders: Orders::vacant(),
            naval_raw: false,
            ticks: [0; TABLE_COUNT],
        })
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        inputs: Option<&UnitInputs>,
    ) {
        let mut ticks = [0; TABLE_COUNT];
        let cost = self.leader_portraits.capture(
            memory,
            base,
            self.orders.roots(),
            self.orders.root_outcome,
        );
        scatter(&leader_portraits::TABLES, cost, &mut ticks);
        let cost =
            self.leader_paths
                .capture(memory, base, game_state, self.leader_portraits.sources());
        scatter(&leader_paths::TABLES, cost, &mut ticks);
        if !self.naval_raw {
            self.naval_history.set_order_sources(
                memory,
                base,
                self.orders.roots(),
                self.orders.root_outcome,
            );
        }
        self.air.finish_wings(self.air_root.unwrap_or(MISSING));
        let cost = self.air.capture(memory, base, game_state, hour, force_full);
        scatter(&air::TABLES, cost, &mut ticks);
        let cost = self
            .naval_history
            .capture(memory, base, game_state, hour, force_full);
        scatter(&naval_history::TABLES, cost, &mut ticks);
        let seeds = inputs.map(UnitInputs::seeds);
        let cost = self
            .army_icon_inputs
            .capture(memory, base, self.divisions.sources(), seeds);
        scatter(&army_icon_inputs::TABLES, cost, &mut ticks);
        self.ticks = ticks;
    }
}

/// The packs whose typed walks wait for the raw-copy verdicts (radar, intel,
/// registrations, projects, construction rest): one pool job dispatched after
/// the mid-block joins and joined before capture_tables.
pub(crate) struct GatedPacks {
    pub(crate) fog: fog::Captured,
    pub(crate) projects: projects::Captured,
    pub(crate) construction: construction::Captured,
    pub(crate) ticks: [u64; TABLE_COUNT],
}

impl GatedPacks {
    fn fail(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.fog.fail_gated_capture(hour, force_full, reason);
        self.projects.fail_gated_capture(hour, force_full, reason);
        self.construction
            .fail_gated_capture(hour, force_full, reason);
        self.ticks = [0; TABLE_COUNT];
    }

    fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.fog.allocated_bytes()
            + self.projects.allocated_bytes()
            + self.construction.allocated_bytes()
    }

    fn spare() -> Box<Self> {
        Box::new(Self {
            fog: fog::Captured::vacant(),
            projects: projects::Captured::vacant(),
            construction: construction::Captured::vacant(),
            ticks: [0; TABLE_COUNT],
        })
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) {
        let mut ticks = [0; TABLE_COUNT];
        scatter(
            &fog::TABLES,
            self.fog.capture(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        scatter(
            &construction::TABLES,
            self.construction
                .capture(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        scatter(
            &projects::TABLES,
            self.projects
                .capture(memory, base, game_state, hour, force_full),
            &mut ticks,
        );
        self.ticks = ticks;
    }
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
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn vacant_spares_restore_storage_and_verdicts_after_success_and_failure() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let mut orders = Orders::vacant();
        orders.geometry = vec![11, 13, 17];
        orders.geometry_len = 3;
        orders.root_outcome = Outcome::Rows { len: 3 };
        let order_storage = orders.geometry.as_ptr();
        let division_storage = captured.divisions.first.as_ptr();
        let box_bytes = [
            size_of::<OrdinaryPacks>(),
            size_of::<SlowPacks>(),
            size_of::<LatePacks>(),
            size_of::<GatedPacks>(),
        ];
        let assert_vacant = |captured: &Captured| {
            // The boxes remain reusable; none of their parked packs owns a
            // second row, arena, hash or source allocation.
            assert_eq!(
                [
                    captured.ordinary_spare.as_ref().unwrap().allocated_bytes(),
                    captured.slow_spare.as_ref().unwrap().allocated_bytes(),
                    captured.late_spare.as_ref().unwrap().allocated_bytes(),
                    captured.gated_spare.as_ref().unwrap().allocated_bytes(),
                ],
                box_bytes
            );
            assert_eq!(captured.spare_bytes(), box_bytes.iter().sum::<usize>());
            assert_eq!(
                captured
                    .allocated_by_pack()
                    .iter()
                    .map(|(_, bytes)| bytes)
                    .sum::<usize>(),
                captured.allocated_bytes()
            );
        };
        assert_vacant(&captured);
        let bytes = captured.allocated_bytes();
        let mut removed = [0; 4];
        for completed in [true, false, true] {
            captured.reset();
            let ordinary_box = std::ptr::from_ref(&**captured.ordinary_spare.as_ref().unwrap());
            let request = captured.take_ordinary_request(&countries, 11, 0x1000);
            assert_eq!(std::ptr::from_ref(&*request.packs), ordinary_box);
            assert_eq!(
                captured.allocated_bytes() + request.allocated_bytes(),
                bytes
            );
            removed[0] = request.allocated_bytes() - box_bytes[0];
            captured.restore_ordinary_request(request, (!completed).then_some("fixture failure"));
            assert_eq!(
                std::ptr::from_ref(&**captured.ordinary_spare.as_ref().unwrap()),
                ordinary_box
            );
            assert_eq!(captured.allocated_bytes(), bytes);
            assert_vacant(&captured);

            let slow_box = std::ptr::from_ref(&**captured.slow_spare.as_ref().unwrap());
            let request = captured.take_slow_request(&countries, 11, 0x1000, 9, false, None);
            assert_eq!(std::ptr::from_ref(&*request.packs), slow_box);
            assert_eq!(
                captured.allocated_bytes() + request.packs.allocated_bytes(),
                bytes
            );
            removed[1] = request.packs.allocated_bytes() - box_bytes[1];
            captured.restore_slow_request(request, completed);
            assert_eq!(
                std::ptr::from_ref(&**captured.slow_spare.as_ref().unwrap()),
                slow_box
            );
            assert_eq!(captured.allocated_bytes(), bytes);
            assert_vacant(&captured);

            captured.fog.radar_raw_mode = true;
            captured.fog.intel_raw_mode = true;
            captured.fog.registrations_raw_mode = true;
            captured.projects.raw_mode = true;
            captured.construction.rest_raw_mode = true;
            let gated_box = std::ptr::from_ref(&**captured.gated_spare.as_ref().unwrap());
            let request = captured.take_gated_request(11, 0x1000, 9, false);
            assert_eq!(std::ptr::from_ref(&*request.packs), gated_box);
            assert_eq!(
                captured.allocated_bytes() + request.packs.allocated_bytes(),
                bytes
            );
            removed[3] = request.packs.allocated_bytes() - box_bytes[3];
            assert!(request.packs.fog.radar_raw_mode);
            assert!(request.packs.fog.intel_raw_mode);
            assert!(request.packs.fog.registrations_raw_mode);
            assert!(request.packs.projects.raw_mode);
            assert!(request.packs.construction.rest_raw_mode);
            assert!(!captured.fog.radar_raw_mode);
            assert!(!captured.projects.raw_mode);
            assert!(!captured.construction.rest_raw_mode);
            captured.restore_gated_request(request, completed);
            assert_eq!(
                std::ptr::from_ref(&**captured.gated_spare.as_ref().unwrap()),
                gated_box
            );
            assert!(captured.fog.radar_raw_mode);
            assert!(captured.fog.intel_raw_mode);
            assert!(captured.fog.registrations_raw_mode);
            assert!(captured.projects.raw_mode);
            assert!(captured.construction.rest_raw_mode);
            assert_eq!(captured.allocated_bytes(), bytes);
            assert_vacant(&captured);

            captured.air.stats_raw_mode = true;
            captured.naval_history.raw_mode = true;
            captured.air_root = Some(Outcome::Rows { len: 7 });
            let order_bytes = orders.allocated_bytes();
            let late_box = std::ptr::from_ref(&**captured.late_spare.as_ref().unwrap());
            let request = captured.take_late_request(&mut orders, true, 11, 0x1000, 9, false);
            assert_eq!(std::ptr::from_ref(&*request.packs), late_box);
            assert_eq!(
                captured.allocated_bytes()
                    + orders.allocated_bytes()
                    + request.packs.allocated_bytes(),
                bytes + order_bytes
            );
            // These two auxiliary placeholders were already vacant before
            // this change; only the four late table packs lose duplicates.
            removed[2] = request.packs.allocated_bytes()
                - box_bytes[2]
                - request.packs.divisions.allocated_bytes()
                - request.packs.orders.allocated_bytes();
            assert!(request.packs.air.stats_raw_mode);
            assert!(request.packs.naval_history.raw_mode);
            assert_eq!(request.packs.air_root, Some(Outcome::Rows { len: 7 }));
            assert_eq!(request.packs.orders.geometry.as_ptr(), order_storage);
            assert_eq!(request.packs.divisions.first.as_ptr(), division_storage);
            assert!(!captured.air.stats_raw_mode);
            assert!(!captured.naval_history.raw_mode);
            assert_eq!(captured.air_root, None);
            captured.restore_late_request(request, completed, &mut orders);
            assert_eq!(
                std::ptr::from_ref(&**captured.late_spare.as_ref().unwrap()),
                late_box
            );
            assert!(captured.air.stats_raw_mode);
            assert!(captured.naval_history.raw_mode);
            assert_eq!(captured.air_root, Some(Outcome::Rows { len: 7 }));
            assert_eq!(orders.geometry.as_ptr(), order_storage);
            assert_eq!(orders.geometry, [11, 13, 17]);
            assert_eq!(orders.root_outcome, Outcome::Rows { len: 3 });
            assert_eq!(captured.divisions.first.as_ptr(), division_storage);
            assert_eq!(captured.allocated_bytes(), bytes);
            assert_vacant(&captured);
        }
        let total: usize = removed.iter().sum();
        println!(
            "Removed default spare payload bytes (Ordinary/Slow/Late/Gated): {removed:?}; \
             per frame {total}; four frames {}; retained box bytes per frame {}",
            total * 4,
            box_bytes.iter().sum::<usize>()
        );
    }

    #[test]
    fn late_handoff_retains_only_the_current_input_lease() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut orders = Orders::vacant();
        for pointer in [0x1234, 0x5678] {
            let inputs = Arc::new(UnitInputs::fixture(
                vec![carmy::DivisionSeed {
                    pointer,
                    ..Default::default()
                }],
                Outcome::Rows { len: 1 },
            ));
            captured.install_unit_inputs(Some(Arc::clone(&inputs)));
            let request = captured.take_late_request(&mut orders, true, 11, 0x1000, 9, false);
            assert!(Arc::ptr_eq(request.inputs.as_ref().unwrap(), &inputs));
            assert_eq!(request.inputs.as_ref().unwrap().seeds()[0].pointer, pointer);
            assert_eq!(Arc::strong_count(&inputs), 3);
            captured.restore_late_request(request, true, &mut orders);
            assert_eq!(Arc::strong_count(&inputs), 2);
            captured.release_unit_inputs();
            assert_eq!(Arc::strong_count(&inputs), 1);
        }
    }

    #[test]
    fn presentation_handoff_restores_storage_and_exact_source_leases() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let inputs = Arc::new(UnitInputs::fixture(
            vec![carmy::DivisionSeed {
                pointer: 0x1234,
                handle: Some(7),
                ..Default::default()
            }],
            Outcome::Rows { len: 1 },
        ));
        let bytes = captured.allocated_bytes();
        let presentation_bytes = captured.division_presentation.allocated_bytes();
        assert!(presentation_bytes > 0);
        let report = division_presentation::CaptureReport { ticks: [89, 97] };
        captured.install_unit_inputs(Some(Arc::clone(&inputs)));
        captured.presentation_report = Some(report);
        let request = captured.take_presentation_request(&countries, 11, 0x1000);
        assert!(captured.presentation_report.is_none());
        assert_eq!(captured.division_presentation.allocated_bytes(), 0);
        assert_eq!(captured.allocated_bytes() + presentation_bytes, bytes);
        assert_eq!(request.captured.allocated_bytes(), presentation_bytes);
        assert_eq!(
            (request.context.base, request.context.game_state),
            (11, 0x1000)
        );
        assert!(Arc::ptr_eq(&request.countries, &countries));
        assert!(Arc::ptr_eq(request.inputs.as_ref().unwrap(), &inputs));
        assert_eq!(request.inputs.as_ref().unwrap().seeds(), inputs.seeds());
        assert_eq!(Arc::strong_count(&countries), 2);
        assert_eq!(Arc::strong_count(&inputs), 3);
        captured.restore_presentation_request(request, Ok(report));
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_eq!(
            captured.division_presentation.allocated_bytes(),
            presentation_bytes
        );
        assert_eq!(Arc::strong_count(&countries), 1);
        assert_eq!(Arc::strong_count(&inputs), 2);
        assert!(Arc::ptr_eq(captured.unit_inputs.as_ref().unwrap(), &inputs));
        let pending = captured.presentation_report.as_ref().unwrap();
        assert_eq!(pending.ticks, report.ticks);
        // Reset discards a pending report and releases the frame's source lease.
        captured.reset();
        assert!(captured.presentation_report.is_none());
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_eq!(Arc::strong_count(&inputs), 1);
    }

    #[test]
    fn eta_input_handoff_restores_storage_leases_and_a_terminal_failure() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let inputs = Arc::new(UnitInputs::fixture(
            vec![carmy::DivisionSeed {
                pointer: 0x1234,
                handle: Some(7),
                ..Default::default()
            }],
            Outcome::Rows { len: 1 },
        ));
        let bytes = captured.allocated_bytes();
        let owned = captured.division_eta_inputs.allocated_bytes();
        // The placeholder keeps the packet's fixed route and transport arenas.
        let vacant = division_eta_inputs::RangeCaptured::vacant().allocated_bytes();
        assert!(owned > vacant);
        for failure in [
            crate::acquisition::Failure::Panicked,
            crate::acquisition::Failure::Stopped,
        ] {
            captured.install_unit_inputs(Some(Arc::clone(&inputs)));
            // Stable worker ownership does not depend on census size.
            assert!(captured.plan_eta_input_ranges() >= 1);
            let request = captured.take_eta_inputs_request(
                0,
                &countries,
                11,
                0x1000,
                7,
                false,
                Box::default(),
            );
            let lent = captured.division_eta_inputs.allocated_bytes();
            assert!(lent < owned);
            assert_eq!(captured.allocated_bytes() + owned, bytes + lent);
            assert_eq!(request.context.span, (0, 1));
            assert_eq!(
                (request.context.base, request.context.game_state),
                (11, 0x1000)
            );
            assert!(Arc::ptr_eq(&request.countries, &countries));
            assert!(Arc::ptr_eq(request.inputs.as_ref().unwrap(), &inputs));
            assert_eq!(Arc::strong_count(&countries), 2);
            assert_eq!(Arc::strong_count(&inputs), 3);
            captured.restore_eta_inputs_request(request, Err(failure));
            assert_eq!(captured.allocated_bytes(), bytes);
            assert_eq!(captured.division_eta_inputs.allocated_bytes(), owned);
            assert_eq!(Arc::strong_count(&countries), 1);
            assert_eq!(Arc::strong_count(&inputs), 2);
            // The terminal failure is consumed once, without a live retry.
            let report = captured.acquire_eta_inputs();
            assert_eq!(report.ticks, [0]);
            assert!(captured.eta_inputs_report.is_none());
            assert_eq!(
                captured.division_eta_inputs.outcomes(),
                [Outcome::Unavailable {
                    reason: match failure {
                        crate::acquisition::Failure::Panicked =>
                            "division ETA input acquisition panicked",
                        crate::acquisition::Failure::Stopped =>
                            "division ETA input acquisition stopped before reading",
                    },
                }]
            );
            captured.reset();
            assert!(captured.eta_inputs_report.is_none());
            assert_eq!(Arc::strong_count(&inputs), 1);
        }
    }

    #[test]
    fn presentation_terminal_reports_scatter_once_without_live_retry() {
        struct NoPresentationRead {
            independent: std::cell::Cell<usize>,
            presentation: std::cell::Cell<usize>,
            forbid_presentation: std::cell::Cell<bool>,
        }
        impl Memory for NoPresentationRead {
            fn copy(&self, address: usize, _: &mut [u8]) -> bool {
                // Presentation begins with this geometry root even for an empty
                // census. New province tables legitimately read the same root,
                // so the strict probe applies to the family handoff itself.
                if address == 0x1000 + engine::cgame_state::PROVINCES {
                    assert!(
                        !self.forbid_presentation.get(),
                        "terminal Presentation report must not retry live acquisition"
                    );
                    self.presentation.set(self.presentation.get() + 1);
                } else {
                    self.independent.set(self.independent.get() + 1);
                }
                false
            }
        }
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let inputs = Arc::new(UnitInputs::empty_rows());
        let bytes = captured.allocated_bytes();
        let report = division_presentation::CaptureReport { ticks: [113, 227] };
        for failure in [
            None,
            Some(crate::acquisition::Failure::Panicked),
            Some(crate::acquisition::Failure::Stopped),
        ] {
            captured.reset();
            captured.install_unit_inputs(Some(Arc::clone(&inputs)));
            let request = captured.take_presentation_request(&countries, 11, 0x1000);
            let context = request.context;
            captured.restore_presentation_request(request, failure.map_or(Ok(report), Err));
            let expected_outcomes = captured.division_presentation.outcomes();
            if failure.is_some() {
                assert!(
                    expected_outcomes
                        .iter()
                        .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
                );
            }
            let memory = NoPresentationRead {
                independent: std::cell::Cell::new(0),
                presentation: std::cell::Cell::new(0),
                forbid_presentation: std::cell::Cell::new(true),
            };
            let completed = captured.acquire_presentation();
            assert_eq!(memory.presentation.get(), 0);
            assert_eq!(memory.independent.get(), 0);
            assert!(captured.presentation_report.is_none());
            // Exercise the same completed result through global table routing,
            // where unrelated province consumers may read this native root.
            captured.presentation_report = Some(completed);
            memory.forbid_presentation.set(false);
            let ticks = captured.capture_tables_all(&memory, 11, 0x1000, 0, false);
            assert!(memory.independent.get() > 0);
            assert!(
                captured.presentation_report.is_none(),
                "the report is consumed once"
            );
            assert_eq!(captured.division_presentation.outcomes(), expected_outcomes);
            for (slot, table) in division_presentation::TABLES.into_iter().enumerate() {
                assert_eq!(
                    ticks[table.index()],
                    if failure.is_none() {
                        report.ticks[slot]
                    } else {
                        0
                    }
                );
            }
            assert_eq!(captured.allocated_bytes(), bytes);
            assert_eq!(captured.decode_division_presentation_inputs(), [0; 2]);
            assert_eq!(captured.division_presentation.outcomes(), expected_outcomes);
            // The same probe must detect an actual acquisition, making absence
            // above meaningful even when every unrelated read fails immediately.
            let before = memory.presentation.get();
            memory.forbid_presentation.set(true);
            captured.acquire_presentation();
            assert_eq!(memory.presentation.get(), before);
            assert_eq!(
                captured.division_presentation.outcomes(),
                [Outcome::Unavailable {
                    reason: "division presentation acquisition not supplied",
                }; 2]
            );
            memory.forbid_presentation.set(false);
            captured.division_presentation.capture_owned(
                &memory,
                context,
                unit_sources(&captured.unit_inputs),
            );
            assert!(memory.presentation.get() > before);
        }
    }

    #[test]
    fn failed_pack_jobs_are_terminal_without_live_retry_or_partial_publication() {
        use engine::memory::{CountedMemory, SliceMemory};
        use replay::frame::TableView;

        let slow = [
            map_names::TABLES.as_slice(),
            division_templates::TABLES.as_slice(),
            visibility::TABLES.as_slice(),
            building_detection::TABLES.as_slice(),
            faction_theatres::TABLES.as_slice(),
        ]
        .concat();
        let gated = [
            fog::TABLES.as_slice(),
            &construction::TABLES[2..],
            projects::TABLES.as_slice(),
        ]
        .concat();
        let late = [
            leader_portraits::TABLES.as_slice(),
            leader_paths::TABLES.as_slice(),
            air::TABLES.as_slice(),
            naval_history::TABLES.as_slice(),
            army_icon_inputs::TABLES.as_slice(),
        ]
        .concat();
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let memory = CountedMemory::new(SliceMemory {
            base: 0,
            bytes: &[],
        });
        for force_full in [false, true] {
            let hour = 1;
            let mut captured = Captured::with_capacities(&Capacities::default());
            captured.prepare_hour(hour, force_full);
            let bytes = captured.allocated_bytes();
            let mut orders = Orders::vacant();
            let mut request =
                captured.take_slow_request(&countries, 11, 0x1000, hour, force_full, None);
            request.packs.ticks.fill(17);
            captured.restore_slow_request(request, false);
            let mut request = captured.take_gated_request(11, 0x1000, hour, force_full);
            request.packs.ticks.fill(23);
            captured.restore_gated_request(request, false);
            let mut request =
                captured.take_late_request(&mut orders, false, 11, 0x1000, hour, force_full);
            request.packs.ticks.fill(31);
            captured.restore_late_request(request, false, &mut orders);
            assert_eq!(captured.allocated_bytes(), bytes);

            // A second assembly must not turn a consumed failure marker into
            // an unsubmitted job and retry its reads on the callback.
            for _ in 0..2 {
                let counted = crate::read_accounting::Capture::new();
                let mut ticks = captured.capture_tables(&memory, 11, 0x1000, hour, force_full);
                captured.capture_tables_late(
                    &memory,
                    11,
                    0x1000,
                    hour,
                    force_full,
                    &mut orders,
                    &mut ticks,
                );
                let reads = counted.finish();
                let outcomes = captured.outcomes();
                let mut tables = std::array::from_fn(|_| {
                    TableInput::Sampled(crate::spooling::unavailable("fixture"))
                });
                captured.insert_tables(
                    &mut tables,
                    None,
                    &mut Default::default(),
                    None,
                    None,
                    None,
                    None,
                );
                for (ids, reason) in [
                    (&slow, "slow acquisition failed"),
                    (&gated, "gated acquisition failed"),
                    (&late, "late acquisition failed"),
                ] {
                    for &id in ids {
                        let expected = failed_capture(id, hour, force_full, reason);
                        assert_eq!(outcomes[id.index()], expected, "{id:?}");
                        assert_eq!(
                            reads[id.index()],
                            engine::memory::ReadCounts::ZERO,
                            "retried {id:?}"
                        );
                        assert_eq!(ticks[id.index()], 0, "partial cost for {id:?}");
                        let Outcome::Unavailable { reason } = expected else {
                            unreachable!()
                        };
                        assert_eq!(
                            tables[id.index()],
                            TableInput::Sampled(TableView::Unavailable {
                                reason: reason.into()
                            }),
                            "{id:?}"
                        );
                    }
                }
            }
            // Reset starts a new observation. The normal path must still read.
            captured.reset();
            let counted = crate::read_accounting::Capture::new();
            let mut ticks = captured.capture_tables(&memory, 11, 0x1000, hour, true);
            captured.capture_tables_late(&memory, 11, 0x1000, hour, true, &mut orders, &mut ticks);
            let reads = counted.finish();
            for id in [
                TableId::ProvinceNames,
                TableId::VolunteerHosts,
                TableId::RailCooldowns,
            ] {
                assert!(
                    reads[id.index()].copy_calls > 0,
                    "control did not read {id:?}"
                );
            }
        }
    }

    #[test]
    fn failed_pack_jobs_preserve_separately_acquired_tables_and_raw_gates() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.prepare_hour(1, true);
        let mut orders = Orders::vacant();
        let raw_failure = "independent raw result";

        captured.fog.fail_radar_capture(raw_failure);
        captured.fog.fail_intel_capture(raw_failure);
        captured.fog.fail_registrations_capture(raw_failure);
        captured.fog.radar_raw_mode = true;
        captured.fog.intel_raw_mode = true;
        captured.fog.registrations_raw_mode = true;
        captured.projects.fail_projects_capture(true, raw_failure);
        captured.projects.raw_mode = true;
        captured
            .construction
            .fail_construction_rest_capture(raw_failure);
        captured.construction.rest_raw_mode = true;
        let buildings = captured.construction.take_buildings();
        captured
            .construction
            .restore_buildings(buildings, Err("independent building result"));
        let before = captured.outcomes();
        let request = captured.take_gated_request(11, 0x1000, 1, true);
        captured.restore_gated_request(request, false);
        let after = captured.outcomes();
        for id in [
            TableId::RadarCoverage,
            TableId::UnitIntel,
            TableId::AirBaseRegistrations,
        ]
        .into_iter()
        .chain(projects::TABLES[..4].iter().copied())
        .chain(construction::TABLES)
        {
            assert_eq!(after[id.index()], before[id.index()], "{id:?}");
        }
        assert!(
            captured.fog.radar_raw_mode
                && captured.fog.intel_raw_mode
                && captured.fog.registrations_raw_mode
        );
        assert!(captured.projects.raw_mode && captured.construction.rest_raw_mode);

        captured.air.fail_air_bases_capture(raw_failure);
        captured.air.fail_air_history_capture(raw_failure);
        captured.air.fail_air_stats_capture(raw_failure);
        captured.air.bases_raw_mode = true;
        captured.air.history_raw_mode = true;
        captured.air.stats_raw_mode = true;
        captured.naval_history.fail_naval_capture(raw_failure);
        captured.naval_history.fail_supremacy_capture(raw_failure);
        captured.naval_history.fail_sunk_capture(raw_failure);
        captured.naval_history.raw_mode = true;
        captured.naval_history.supremacy_raw_mode = true;
        captured.naval_history.sunk_raw_mode = true;
        let before = captured.outcomes();
        let request = captured.take_late_request(&mut orders, true, 11, 0x1000, 1, true);
        captured.restore_late_request(request, false, &mut orders);
        let after = captured.outcomes();
        for id in [
            TableId::AirBases,
            TableId::AirRegionHistory,
            TableId::AirRegionDamage,
            TableId::AirAces,
            TableId::AirStatistics,
            TableId::NavalSupremacy,
            TableId::SunkShips,
        ]
        .into_iter()
        .chain(naval_history::TABLES[4..10].iter().copied())
        .chain(naval_history::TABLES[14..].iter().copied())
        {
            assert_eq!(after[id.index()], before[id.index()], "{id:?}");
        }
        assert!(
            captured.air.bases_raw_mode
                && captured.air.history_raw_mode
                && captured.air.stats_raw_mode
        );
        assert!(
            captured.naval_history.raw_mode
                && captured.naval_history.supremacy_raw_mode
                && captured.naval_history.sunk_raw_mode
        );
    }

    #[test]
    fn ordinary_terminal_results_cannot_retry_or_publish_partial_tables() {
        #[derive(Default)]
        struct Reads {
            combat: std::cell::Cell<usize>,
        }
        impl Memory for Reads {
            fn copy(&self, address: usize, _: &mut [u8]) -> bool {
                if address == 0x1000 + engine::cgame_state::COMBATS + 8 {
                    self.combat.set(self.combat.get() + 1);
                }
                false
            }
        }
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        for reason in [
            "ordinary worker admission failed",
            "acquisition worker panicked",
            "acquisition worker stopped before completion",
        ] {
            let mut captured = Captured::with_capacities(&Capacities::default());
            let before = captured.allocated_bytes();
            let mut request = captured.take_ordinary_request(&countries, 11, 0x1000);
            request.packs.ticks[TableId::LandCombatDetails.index()] = 71;
            let owned = request.allocated_bytes();
            if reason == "ordinary worker admission failed" {
                request.fail(reason);
                captured.restore_ordinary_request(request, None);
            } else {
                captured.restore_ordinary_request(request, Some(reason));
            }
            assert_eq!(captured.allocated_bytes(), before);
            let memory = Reads::default();
            let ticks = captured.capture_tables(&memory, 11, 0x1000, 1, false);
            assert_eq!(memory.combat.get(), 0, "terminal work must not retry");
            for &id in OrdinaryPacks::TABLES
                .iter()
                .flat_map(|tables| tables.iter())
            {
                assert_eq!(
                    captured.outcomes()[id.index()],
                    Outcome::Unavailable { reason }
                );
                assert_eq!(ticks[id.index()], 0);
            }
            // A missing completion marker also cannot reacquire live data.
            captured.capture_tables(&memory, 11, 0x1000, 1, false);
            assert_eq!(memory.combat.get(), 0);
            assert_eq!(
                captured.ordinary_failure,
                Some("ordinary acquisition not supplied")
            );
            captured.reset();
            assert!(captured.ordinary_failure.is_none());
            let mut request = captured.take_ordinary_request(&countries, 11, 0x1000);
            assert_eq!(request.allocated_bytes(), owned);
            request.packs.capture(&memory, 11, 0x1000);
            assert!(
                memory.combat.get() > 0,
                "control must exercise the native reader"
            );
            captured.restore_ordinary_request(request, None);
            assert_eq!(Arc::strong_count(&countries), 1);
        }
    }

    #[test]
    fn research_handoff_maps_failures_scatters_ticks_and_survives_a_governance_failure() {
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let (hour, force_full) = (7, false);
        for failure in [
            None,
            Some(crate::acquisition::Failure::Panicked),
            Some(crate::acquisition::Failure::Stopped),
        ] {
            let mut captured = Captured::with_capacities(&Capacities::default());
            captured.prepare_hour(hour, force_full);
            let bytes = captured.allocated_bytes();
            // Owner 7 (in this hour's slice) was seen, unchanged, before this hour.
            let mut store = Box::new(crate::slicing::SliceFingerprints::new());
            let mut seen = crate::slicing::SliceSelection::for_test(0, true, &[]);
            seen.scan(&mut store, 7, Some(1));
            store.finish_scan();
            let research =
                captured.take_research_request(&countries, 11, 0x1000, hour, force_full, store);
            // The family keeps no research capacity while the part is loaned.
            assert!(captured.allocated_bytes() < bytes);
            let governance = captured.take_governance_request(
                &countries,
                11,
                0x1000,
                hour,
                force_full,
                Box::new(crate::slicing::SliceFingerprints::new()),
            );
            // Governance fails; its restore must not reach the loaned research rows.
            let _technologies = captured
                .restore_governance_request(governance, Err(crate::acquisition::Failure::Panicked));
            let report = match failure {
                None => Ok(crate::acquisition_jobs::ResearchReport { ticks: [5, 6, 7] }),
                Some(failure) => Err(failure),
            };
            let mut store = captured.restore_research_request(research, report);
            // This hour's research read was not published (a job failure
            // forgets the store; a complete job whose tables are not all
            // complete invalidates its selection), so owner 7 is read again
            // at the next hour although nothing changed.
            let mut next = crate::slicing::SliceSelection::for_test(hour + 1, false, &[]);
            next.scan(&mut store, 7, Some(1));
            assert!(next.contains(7), "{failure:?}");
            assert_eq!(captured.allocated_bytes(), bytes);
            let outcomes = captured.governance.outcomes();
            let family = captured
                .governance_report
                .expect("governance report restored");
            match failure {
                None => {
                    // Costs land on the research tables; the family's failure
                    // did not replace the research outcomes.
                    for (index, cost) in governance::RESEARCH.into_iter().zip([5, 6, 7]) {
                        assert_eq!(family.ticks[index], cost);
                        assert!(!matches!(
                            outcomes[index],
                            Outcome::Unavailable { reason } if reason == "governance acquisition panicked"
                        ));
                    }
                }
                Some(failure) => {
                    let expected = match failure {
                        crate::acquisition::Failure::Panicked => "research acquisition panicked",
                        crate::acquisition::Failure::Stopped => {
                            "research acquisition stopped before reading"
                        }
                    };
                    for index in governance::RESEARCH {
                        assert_eq!(family.ticks[index], 0);
                        assert_eq!(outcomes[index], Outcome::Unavailable { reason: expected });
                    }
                }
            }
            // The family's own tables carry the family failure either way.
            assert_eq!(
                outcomes[5],
                Outcome::Unavailable {
                    reason: "governance acquisition panicked"
                }
            );
        }
    }

    #[test]
    fn variant_handoff_skips_off_slot_without_reads_and_never_retries_a_failed_worker() {
        struct Reads(std::cell::Cell<usize>);
        impl Memory for Reads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                false
            }
        }
        let slot = replay::cadence::refresh_slot(TableId::VariantDefinitions).unwrap() as i32;
        let mut captured = Captured::with_capacities(&Capacities::default());
        let bytes = captured.allocated_bytes();
        let memory = engine::memory::CountedMemory::new(Reads(std::cell::Cell::new(0)));
        assert!(
            captured
                .take_variants_request(11, 0x1000, slot + 1, false)
                .is_none()
        );
        let ticks = captured.capture_tables_all(&memory, 11, 0x1000, slot + 1, false);
        for (index, table) in production::TABLES.into_iter().enumerate().skip(3).take(4) {
            assert_eq!(ticks[table.index()], 0);
            assert_eq!(
                captured.production.outcomes()[index],
                Outcome::Unavailable {
                    reason: replay::cadence::SKIPPED_REASON
                }
            );
        }
        for failure in [
            crate::acquisition::Failure::Panicked,
            crate::acquisition::Failure::Stopped,
        ] {
            let request = captured
                .take_variants_request(11, 0x1000, slot, false)
                .expect("the variant slot loans the family");
            assert_eq!(
                captured.allocated_bytes() + request.allocated_bytes(),
                bytes
            );
            captured.restore_variants_request(request, Err(failure));
            assert_eq!(captured.allocated_bytes(), bytes);
            let reads = crate::read_accounting::Capture::new();
            captured.capture_tables_all(&memory, 11, 0x1000, slot, false);
            let counts = reads.finish();
            for table in &production::TABLES[3..7] {
                assert_eq!(counts[table.index()], engine::memory::ReadCounts::ZERO);
            }
            let expected = match failure {
                crate::acquisition::Failure::Panicked => "variant acquisition panicked",
                crate::acquisition::Failure::Stopped => {
                    "variant acquisition stopped before reading"
                }
            };
            for index in 3..7 {
                assert_eq!(
                    captured.production.outcomes()[index],
                    Outcome::Unavailable { reason: expected }
                );
            }
        }
    }

    #[test]
    fn governance_handoff_restores_owned_storage_and_never_retries_terminal_results() {
        struct Reads {
            total: std::cell::Cell<usize>,
            governance: std::cell::Cell<usize>,
        }
        impl Memory for Reads {
            fn copy(&self, address: usize, _: &mut [u8]) -> bool {
                self.total.set(self.total.get() + 1);
                if address == 0x1000 + engine::cgame_state::TECH_SHARING_GROUPS {
                    self.governance.set(self.governance.get() + 1);
                }
                false
            }
        }
        for failure in [
            None,
            Some(crate::acquisition::Failure::Panicked),
            Some(crate::acquisition::Failure::Stopped),
        ] {
            let mut captured = Captured::with_capacities(&Capacities::default());
            captured.governance.focus_raw_mode = true;
            captured.governance_report = Some(governance::CaptureReport::default());
            let bytes = captured.allocated_bytes();
            let governance_bytes = captured.governance.allocated_bytes();
            let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
            let request = captured.take_governance_request(
                &countries,
                11,
                0x1000,
                1,
                failure.is_none(),
                Box::new(crate::slicing::SliceFingerprints::new()),
            );
            assert!(captured.governance_report.is_none());
            assert_eq!(captured.governance.allocated_bytes(), 0);
            assert_eq!(
                captured.allocated_bytes() + request.allocated_bytes(),
                bytes
            );
            assert_eq!(request.captured.allocated_bytes(), governance_bytes);
            assert!(request.allocated_bytes() > governance_bytes);
            assert!(request.captured.focus_raw_mode);
            assert!(Arc::ptr_eq(&request.countries, &countries));
            assert_eq!(Arc::strong_count(&countries), 2);
            assert_eq!(
                (
                    request.context.base,
                    request.context.game_state,
                    request.context.hour
                ),
                (11, 0x1000, 1)
            );
            assert_eq!(request.context.force_full, failure.is_none());
            let context = request.context;
            let report = crate::acquisition_jobs::GovernanceReport {
                governance: governance::CaptureReport {
                    ticks: std::array::from_fn(|slot| slot as u64 + 101),
                },
                buildings: 307,
            };
            let variant_ticks: [u64; production::N] = std::array::from_fn(|slot| {
                if (3..=6).contains(&slot) {
                    slot as u64 + 211
                } else {
                    0
                }
            });
            // Gated borrows the rest of construction while the allocating
            // building buffers are still on their worker. Restore that pack
            // before restoring the worker-only families.
            let gated = captured.take_gated_request(11, 0x1000, 1, context.force_full);
            captured.restore_gated_request(gated, false);
            let _technologies =
                captured.restore_governance_request(request, failure.map_or(Ok(report), Err));
            // Variants leave only on their slot or a full hour.
            let variants = captured.take_variants_request(11, 0x1000, 1, context.force_full);
            assert_eq!(variants.is_some(), context.force_full);
            if let Some(variants) = variants {
                captured.restore_variants_request(variants, Ok(variant_ticks));
            }
            assert_eq!(Arc::strong_count(&countries), 1);
            assert_eq!(captured.allocated_bytes(), bytes);
            assert!(captured.governance.focus_raw_mode);
            let expected_outcomes = captured.governance.outcomes();
            let memory = Reads {
                total: std::cell::Cell::new(0),
                governance: std::cell::Cell::new(0),
            };
            let ticks = captured.capture_tables_all(&memory, 11, 0x1000, 1, context.force_full);
            assert!(memory.total.get() > 0);
            assert_eq!(
                memory.governance.get(),
                0,
                "completed Governance must not run again during independent capture"
            );
            assert!(captured.governance_report.is_none());
            assert_eq!(captured.governance.outcomes(), expected_outcomes);
            for (slot, table) in governance::TABLES.into_iter().enumerate() {
                assert_eq!(
                    ticks[table.index()],
                    if failure.is_none() {
                        report.governance.ticks[slot]
                    } else {
                        0
                    }
                );
                // ResearchSlots is off its slot; the sliced technology tables
                // are read every hour, so the failure is theirs.
                if failure.is_some() && slot == 3 {
                    assert_eq!(
                        expected_outcomes[slot],
                        Outcome::Unavailable {
                            reason: replay::cadence::SKIPPED_REASON
                        }
                    );
                }
                if failure.is_some() && governance::TECHNOLOGIES.contains(&slot) {
                    assert!(matches!(
                        expected_outcomes[slot],
                        Outcome::Unavailable { reason } if reason != replay::cadence::SKIPPED_REASON
                    ));
                }
            }
            for (slot, table) in production::TABLES.into_iter().enumerate().skip(3).take(4) {
                assert_eq!(
                    ticks[table.index()],
                    if failure.is_none() {
                        variant_ticks[slot]
                    } else {
                        0
                    }
                );
            }
            assert_eq!(
                ticks[TableId::StateBuildings.index()],
                if failure.is_none() {
                    report.buildings
                } else {
                    0
                }
            );
            assert!(captured.variants_ticks.is_none());
            // A missing worker result also cannot trigger an allocating walk
            // on the caller, even on a full-refresh hour.
            captured.capture_tables_all(&memory, 11, 0x1000, 1, true);
            assert_eq!(memory.governance.get(), 0);
            assert_eq!(
                captured.governance.outcomes()[2],
                Outcome::Unavailable {
                    reason: "governance acquisition not supplied"
                }
            );
            // A direct control capture proves that the spy detects Governance work.
            captured.governance.capture_owned(
                &memory,
                governance::CaptureContext {
                    force_full: true,
                    ..context
                },
                &mut crate::slicing::SliceFingerprints::new(),
            );
            assert!(memory.governance.get() > 0);
            captured.reset();
            assert!(captured.governance_report.is_none());
        }
    }

    #[test]
    fn intel_residual_handoff_preserves_terminal_results_without_repeating_live_reads() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("completed residual capture and delegated raw jobs must not read again");
            }
        }
        for failure in [
            None,
            Some(crate::acquisition::Failure::Panicked),
            Some(crate::acquisition::Failure::Stopped),
        ] {
            let mut captured = Captured::with_capacities(&Capacities::default());
            captured.intel.capitulation_raw_mode = true;
            captured.intel.relations_raw_mode = true;
            let bytes = captured.allocated_bytes();
            let family_bytes = captured.intel.allocated_bytes();
            let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
            let context = intel::CaptureContext {
                base: 11,
                game_state: 0x1000,
                hour: 1,
                force_full: true,
            };
            let request = captured.take_intel_residual_request(&countries, context);
            assert_eq!(captured.intel.allocated_bytes(), 0);
            assert_eq!(captured.allocated_bytes() + family_bytes, bytes);
            assert_eq!(request.captured.allocated_bytes(), family_bytes);
            assert!(Arc::ptr_eq(&request.countries, &countries));
            let original = intel::CaptureReport {
                ticks: std::array::from_fn(|index| index as u64 + 101),
            };
            captured.restore_intel_residual_request(request, failure.map_or(Ok(original), Err));
            assert_eq!(Arc::strong_count(&countries), 1);
            assert_eq!(captured.allocated_bytes(), bytes);
            let outcomes = captured.intel.outcomes();
            let report = captured.acquire_intel(&NoReads, context);
            assert!(captured.intel_residual_report.is_none());
            for index in (1..=12).chain(std::iter::once(15)) {
                assert_eq!(captured.intel.outcomes()[index], outcomes[index]);
                assert_eq!(
                    report.ticks[index],
                    if failure.is_none() {
                        original.ticks[index]
                    } else {
                        0
                    }
                );
            }
            for index in [0, 13, 14] {
                assert_eq!(report.ticks[index], 0);
            }
            captured.reset();
            assert!(captured.intel_residual_report.is_none());
        }
    }

    #[test]
    fn ship_model_parts_restore_part_zero_first_sum_ticks_and_match_the_unsplit_walk() {
        struct Faulty;
        impl Memory for Faulty {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let walked = |ships: u64| {
            let mut captured = Captured::with_capacities(&Capacities::default());
            captured.ship_models.begin_sources();
            for id in 1..=ships {
                captured.ship_models.visit_source(
                    0x1000 * id as usize,
                    &engine::cship::Row {
                        id,
                        ..Default::default()
                    },
                );
            }
            captured.ship_models.finish_sources(Outcome::Rows {
                len: ships as usize,
            });
            let request =
                captured.take_ship_models_request(&countries, 11, 0x1000, ShipCensus::vacant());
            let _census = captured.restore_ship_models_request(request, Ok(()));
            captured
        };
        // The unsplit walk's verdict, straight from the family capture.
        let mut reference = walked(3);
        reference
            .ship_models
            .capture_owned(&Faulty, ship_models::CaptureContext { base: 11 });
        let expected = reference.ship_models.outcomes();
        assert!(matches!(expected[0], Outcome::Unavailable { .. }));
        for (ships, parts, stop) in [
            (3, 1, None),
            (1200, 3, None),
            (2049, 4, None),
            (1200, 3, Some(2)),
        ] {
            let mut captured = walked(ships);
            let requests = captured.take_ship_model_parts(&countries, 11);
            assert_eq!(requests.iter().flatten().count(), parts, "{ships} ships");
            // The family lends its own capture as part 0.
            assert_eq!(captured.ship_models.allocated_bytes(), 0);
            for (index, request) in requests.into_iter().flatten().enumerate() {
                assert_eq!(request.part, index);
                let mut request = request;
                let report = if stop == Some(index) {
                    Err(crate::acquisition::Failure::Stopped)
                } else {
                    request.captured.capture_owned(&Faulty, request.context);
                    Ok(ship_models::CaptureReport {
                        ticks: [index as u64 + 1],
                    })
                };
                captured.restore_ship_model_part(request, report);
            }
            captured.finish_ship_model_parts();
            if stop.is_some() {
                assert!(captured.ship_models.has_terminal_failure());
                continue;
            }
            // Every part's cost adds up; the verdict is the unsplit walk's.
            let ticks = (1..=parts as u64).sum::<u64>();
            assert_eq!(captured.ship_models_report.unwrap().ticks, [ticks]);
            assert_eq!(captured.ship_models.outcomes(), expected, "{ships} ships");
        }
        // A failed census lends no part.
        let mut captured = walked(3);
        let request =
            captured.take_ship_models_request(&countries, 11, 0x1000, ShipCensus::vacant());
        let _census = captured
            .restore_ship_models_request(request, Err(crate::acquisition::Failure::Panicked));
        assert!(
            captured
                .take_ship_model_parts(&countries, 11)
                .iter()
                .all(Option::is_none)
        );
    }

    #[test]
    fn ship_models_late_terminal_handoff_is_unavailable_until_new_sources() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("terminal ShipModels routing must not retry owned detail reads");
            }
        }
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        captured.ship_models.begin_sources();
        captured
            .ship_models
            .finish_sources(Outcome::Rows { len: 0 });
        let request =
            captured.take_ship_models_request(&countries, 11, 0x1000, ShipCensus::vacant());
        let family_bytes = request.captured.allocated_bytes();
        assert_eq!(captured.ship_models.allocated_bytes(), 0);
        let census = captured
            .restore_ship_models_request(request, Err(crate::acquisition::Failure::Panicked));
        assert!(matches!(census.ships.outcome, Outcome::Unavailable { .. }));
        assert_eq!(captured.ship_models.allocated_bytes(), family_bytes);
        assert_eq!(
            captured.ship_models_report.unwrap().ticks,
            [0],
            "interrupted worker detail cost remains unmeasured"
        );
        let ticks = captured.capture_ship_models(&NoReads, 11);
        assert_eq!(ticks, 0, "failed whole-table service cost is unmeasured");
        assert!(
            captured.ship_models_report.is_none(),
            "terminal report is consumed once"
        );
        assert!(captured.ship_models.has_terminal_failure());
        assert!(matches!(
            captured.ship_models.outcomes()[0],
            Outcome::Unavailable {
                reason: "ship models acquisition panicked"
            }
        ));
        let again_ticks = captured.capture_ship_models(&NoReads, 11);
        assert_eq!(again_ticks, 0);
        assert!(captured.ship_models_report.is_none());
        assert!(matches!(
            captured.ship_models.outcomes()[0],
            Outcome::Unavailable {
                reason: "ship models acquisition panicked"
            }
        ));

        captured.reset();
        captured.ship_models.begin_sources();
        captured
            .ship_models
            .finish_sources(Outcome::Rows { len: 0 });
        captured.capture_ship_models(&NoReads, 11);
        assert!(
            !captured.ship_models.has_terminal_failure(),
            "a new complete source epoch clears the stale terminal failure"
        );
        assert_eq!(captured.ship_models.outcomes(), [Outcome::Rows { len: 0 }]);
        assert_eq!(Arc::strong_count(&countries), 1);
    }

    #[test]
    fn hourly_family_handoffs_restore_independently_and_consume_terminal_reports_once() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("a terminal hourly family must not read again");
            }
        }
        #[derive(Default)]
        struct Reads(std::cell::Cell<usize>);
        impl Memory for Reads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                false
            }
        }
        use crate::acquisition::Failure::{Panicked, Stopped};
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let bytes = captured.allocated_bytes();
        let supply_bytes = captured.supply_trains.allocated_bytes();
        let portrait_bytes = captured.character_portraits.allocated_bytes();
        let province_order_bytes = captured.province_unit_order.allocated_bytes();
        for (supply_failure, portrait_failure, province_order_failure) in [
            (None, None, None),
            (Some(Panicked), None, None),
            (None, Some(Panicked), None),
            (None, None, Some(Panicked)),
            (Some(Stopped), None, None),
            (None, Some(Stopped), None),
            (None, None, Some(Stopped)),
        ] {
            captured.reset();
            let before = captured.outcomes();
            captured.supply_trains_report = Some(supply_trains::CaptureReport::default());
            captured.character_portraits_report =
                Some(character_portraits::CaptureReport::default());
            captured.province_unit_order_report =
                Some(province_unit_order::CaptureReport::default());
            let supply: Vec<_> = (0..supply_trains::PARTS)
                .map(|part| captured.take_supply_trains_request(&countries, 11, 0x1000, part))
                .collect();
            assert!(captured.supply_trains_report.is_none());
            assert_eq!(captured.supply_trains.allocated_bytes(), 0);
            assert_eq!(
                captured.character_portraits.allocated_bytes(),
                portrait_bytes
            );
            assert_eq!(
                supply
                    .iter()
                    .map(|r| r.nodes.allocated_bytes()
                        + r.trains.as_ref().map_or(0, |t| t.allocated_bytes()))
                    .sum::<usize>(),
                supply_bytes
            );
            assert_eq!(captured.allocated_bytes() + supply_bytes, bytes);
            let portraits: Vec<_> = (0..character_portraits::PARTS)
                .map(|part| captured.take_character_portraits_request(11, 0x1000, part))
                .collect();
            assert!(captured.character_portraits_report.is_none());
            assert_eq!(captured.character_portraits.allocated_bytes(), 0);
            assert_eq!(
                portraits
                    .iter()
                    .map(|r| r.captured.allocated_bytes())
                    .sum::<usize>(),
                portrait_bytes
            );
            assert_eq!(
                captured.allocated_bytes() + supply_bytes + portrait_bytes,
                bytes
            );
            let province_order = captured.take_province_unit_order_request(11, 0x1000);
            assert!(captured.province_unit_order_report.is_none());
            assert_eq!(captured.province_unit_order.allocated_bytes(), 0);
            assert_eq!(
                province_order.captured.allocated_bytes(),
                province_order_bytes
            );
            assert_eq!(
                captured.allocated_bytes() + supply_bytes + portrait_bytes + province_order_bytes,
                bytes
            );
            assert!(supply.iter().all(|r| Arc::ptr_eq(&r.countries, &countries)));
            assert_eq!(Arc::strong_count(&countries), 1 + supply_trains::PARTS);
            let supply_context = supply[0].context;
            let portrait_context = portraits[0].context;
            let province_order_context = province_order.context;
            assert_eq!(
                (supply_context.base, supply_context.game_state),
                (11, 0x1000)
            );
            assert_eq!(
                (portrait_context.base, portrait_context.game_state),
                (11, 0x1000)
            );
            assert_eq!(
                (
                    province_order_context.base,
                    province_order_context.game_state
                ),
                (11, 0x1000)
            );
            let supply_report = supply_trains::CaptureReport {
                ticks: [101 * supply_trains::PARTS as u64, 103],
            };
            let portrait_report = character_portraits::CaptureReport {
                ticks: [107 * character_portraits::PARTS as u64],
            };
            let province_order_report = province_unit_order::CaptureReport { ticks: [109] };
            for request in supply {
                let part = supply_trains::CaptureReport {
                    ticks: [101, if request.part == 0 { 103 } else { 0 }],
                };
                captured
                    .restore_supply_trains_request(request, supply_failure.map_or(Ok(part), Err));
            }
            assert_eq!(Arc::strong_count(&countries), 1);
            assert!(captured.supply_trains_report.is_some());
            assert!(captured.character_portraits_report.is_none());
            for request in portraits {
                captured.restore_character_portraits_request(
                    request,
                    portrait_failure
                        .map_or(Ok(character_portraits::CaptureReport { ticks: [107] }), Err),
                );
            }
            assert_eq!(Arc::strong_count(&countries), 1);
            captured.restore_province_unit_order_request(
                province_order,
                province_order_failure.map_or(Ok(province_order_report), Err),
            );
            assert_eq!(captured.allocated_bytes(), bytes);
            let outcomes = captured.outcomes();
            for (index, outcome) in outcomes.into_iter().enumerate() {
                if ![
                    TableId::SupplyNodes.index(),
                    TableId::TrainInventory.index(),
                    TableId::CharacterPortraits.index(),
                    TableId::ProvinceUnitOrder.index(),
                ]
                .contains(&index)
                {
                    assert_eq!(outcome, before[index], "unowned table {index}");
                }
            }
            for (tables, failure, panicked, stopped) in [
                (
                    supply_trains::TABLES.as_slice(),
                    supply_failure,
                    "supply trains acquisition panicked",
                    "supply trains acquisition stopped before reading",
                ),
                (
                    character_portraits::TABLES.as_slice(),
                    portrait_failure,
                    "character portraits acquisition panicked",
                    "character portraits acquisition stopped before reading",
                ),
                (
                    province_unit_order::TABLES.as_slice(),
                    province_order_failure,
                    "province unit order acquisition panicked",
                    "province unit order acquisition stopped before reading",
                ),
            ] {
                if let Some(failure) = failure {
                    let reason = match failure {
                        Panicked => panicked,
                        Stopped => stopped,
                    };
                    for table in tables {
                        assert_eq!(outcomes[table.index()], Outcome::Unavailable { reason });
                    }
                }
            }
            let supply = captured.acquire_supply_trains(&NoReads, supply_context);
            let portraits = captured.acquire_character_portraits(&NoReads, portrait_context);
            let province_order =
                captured.acquire_province_unit_order(&NoReads, province_order_context);
            assert_eq!(
                supply.ticks,
                if supply_failure.is_none() {
                    supply_report.ticks
                } else {
                    [0; 2]
                }
            );
            assert_eq!(
                portraits.ticks,
                if portrait_failure.is_none() {
                    portrait_report.ticks
                } else {
                    [0]
                }
            );
            assert_eq!(
                province_order.ticks,
                if province_order_failure.is_none() {
                    province_order_report.ticks
                } else {
                    [0]
                }
            );
            assert!(captured.supply_trains_report.is_none());
            assert!(captured.character_portraits_report.is_none());
            assert!(captured.province_unit_order_report.is_none());
            // Route the identical reports through the full ordinary stage.
            // Unrelated families may legitimately read the same native roots.
            captured.supply_trains_report = Some(supply);
            captured.character_portraits_report = Some(portraits);
            captured.province_unit_order_report = Some(province_order);
            let memory = Reads::default();
            let ticks = captured.capture_tables_all(&memory, 11, 0x1000, 1, false);
            assert!(memory.0.get() > 0);
            assert_eq!(
                supply_trains::TABLES.map(|table| ticks[table.index()]),
                supply.ticks
            );
            assert_eq!(
                character_portraits::TABLES.map(|table| ticks[table.index()]),
                portraits.ticks
            );
            assert_eq!(
                province_unit_order::TABLES.map(|table| ticks[table.index()]),
                province_order.ticks
            );
            let routed_outcomes = captured.outcomes();
            assert_eq!(
                supply_trains::TABLES.map(|table| routed_outcomes[table.index()]),
                supply_trains::TABLES.map(|table| outcomes[table.index()])
            );
            assert_eq!(
                character_portraits::TABLES.map(|table| routed_outcomes[table.index()]),
                character_portraits::TABLES.map(|table| outcomes[table.index()])
            );
            assert_eq!(
                province_unit_order::TABLES.map(|table| routed_outcomes[table.index()]),
                province_unit_order::TABLES.map(|table| outcomes[table.index()])
            );
            assert!(captured.supply_trains_report.is_none());
            assert!(captured.character_portraits_report.is_none());
            assert!(captured.province_unit_order_report.is_none());
            // With no pending report, each direct table read fails once at
            // its root: every supply part, the trains, portraits and the order.
            let direct = Reads::default();
            captured.acquire_supply_trains(&direct, supply_context);
            captured.acquire_character_portraits(&direct, portrait_context);
            captured.acquire_province_unit_order(&direct, province_order_context);
            assert_eq!(
                direct.0.get(),
                3 * supply_trains::PARTS + 1 + character_portraits::PARTS + 1
            );
            captured.supply_trains_report = Some(supply);
            captured.character_portraits_report = Some(portraits);
            captured.province_unit_order_report = Some(province_order);
            captured.reset();
            assert!(captured.supply_trains_report.is_none());
            assert!(captured.character_portraits_report.is_none());
            assert!(captured.province_unit_order_report.is_none());
            assert_eq!(captured.allocated_bytes(), bytes);
        }
    }

    #[test]
    fn shared_unit_sources_keep_census_faults_and_release_on_reset() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        assert_eq!(unit_sources(&captured.unit_inputs).outcome, MISSING);
        assert!(unit_sources(&captured.unit_inputs).rows.is_empty());
        let inputs = Arc::new(UnitInputs::fixture(
            vec![carmy::DivisionSeed {
                pointer: 0x1234,
                ..Default::default()
            }],
            Outcome::Truncated {
                len: 1,
                unread_rows: 7,
            },
        ));
        captured.install_unit_inputs(Some(Arc::clone(&inputs)));
        let sources = unit_sources(&captured.unit_inputs);
        assert_eq!(sources.rows, inputs.seeds());
        assert_eq!(sources.rows[0].pointer, 0x1234);
        assert_eq!(
            sources.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 7
            }
        );
        let failed = Arc::new(UnitInputs::fixture(
            Vec::new(),
            Outcome::Unavailable {
                reason: "current census failed",
            },
        ));
        captured.install_unit_inputs(Some(Arc::clone(&failed)));
        assert_eq!(Arc::strong_count(&inputs), 1);
        assert!(unit_sources(&captured.unit_inputs).rows.is_empty());
        assert_eq!(
            unit_sources(&captured.unit_inputs).outcome,
            Outcome::Unavailable {
                reason: "current census failed"
            },
        );
        captured.reset();
        assert_eq!(Arc::strong_count(&failed), 1);
        assert_eq!(unit_sources(&captured.unit_inputs).outcome, MISSING);
        assert!(unit_sources(&captured.unit_inputs).rows.is_empty());
    }

    #[test]
    fn metadata_partitions_every_extended_table_once_and_preserves_cost_membership() {
        assert_eq!(N, replay::frame::TABLE_COUNT - 20);
        let actual: BTreeSet<_> = TABLES.into_iter().collect();
        assert_eq!(actual.len(), N);
        let expected: BTreeSet<_> = TableId::ALL
            .into_iter()
            .filter(|table| table.index() >= 20)
            .collect();
        assert_eq!(actual, expected);
        for (name, anchor, members) in read_groups() {
            assert!(!name.is_empty());
            assert!(members.contains(&anchor));
            if anchor == TableId::Divisions {
                assert_eq!(
                    members,
                    std::iter::once(TableId::Divisions)
                        .chain(DIVISION_SOURCE_TABLES)
                        .collect::<Vec<_>>()
                );
            } else if anchor == TableId::ArmyOrders {
                assert_eq!(
                    members,
                    vec![
                        TableId::ArmyOrders,
                        TableId::ArmyOrderPlans,
                        TableId::OrdersGroupPlans,
                        TableId::Fronts
                    ]
                );
            } else if anchor == TableId::Countries {
                assert_eq!(
                    members,
                    vec![
                        TableId::Countries,
                        TableId::CountryAppearance,
                        TableId::SupplyNodes,
                        TableId::TrainInventory
                    ]
                );
            } else if anchor == TableId::Ships {
                assert_eq!(members, vec![TableId::Ships, TableId::ShipModels]);
            } else if anchor == TableId::States {
                assert_eq!(members, vec![TableId::States, TableId::StateFlags]);
            } else {
                assert!(members.iter().all(|table| actual.contains(table)));
            }
        }
        let groups = read_groups();
        for table in DIVISION_SOURCE_TABLES {
            assert_eq!(
                groups
                    .iter()
                    .filter(|(_, _, members)| members.contains(&table))
                    .count(),
                1,
                "shared division cost counted more than once for {table:?}"
            );
        }
    }
    #[test]
    fn division_sources_preserve_census_identity_and_reject_an_incomplete_parent() {
        let mut sources = Divisions::new(1);
        let source = DivisionSource {
            pointer: 0x1234,
            division: (1_u64 << 48) | 14,
            country: 17,
            template: u64::MAX,
        };
        sources.visit_source(source);
        sources.finish(Outcome::Rows { len: 1 });
        assert_eq!(sources.sources().outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            sources.sources().rows[0],
            DivisionSource {
                pointer: 0x1234,
                division: (1_u64 << 48) | 14,
                template: u64::MAX,
                country: 17
            }
        );
        sources.visit_source(DivisionSource {
            pointer: 0x5678,
            ..source
        });
        sources.finish(Outcome::Truncated {
            len: 2,
            unread_rows: 7,
        });
        assert_eq!(
            sources.sources().outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 8
            }
        );
        assert_eq!(sources.required, 9);
        let mut capacity = 1;
        sources.grow(&mut capacity);
        assert!(capacity >= 9);
        assert_eq!(sources.sources().rows[0], source);
        sources.reset();
        sources.finish(Outcome::Rows { len: 1 });
        assert!(matches!(
            sources.sources().outcome,
            Outcome::Unavailable { .. }
        ));
    }
    #[test]
    fn registry_false_counts_are_rejected_and_incomplete_censuses_remain_incomplete() {
        assert!(matches!(
            bounded_registry(Outcome::Rows { len: 3 }, 2),
            Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            bounded_registry(
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 0
                },
                2
            ),
            Outcome::Unavailable { .. }
        ));
        let empty = bounded_registry(Outcome::Rows { len: 0 }, 0);
        assert_eq!(empty, Outcome::Rows { len: 0 });
        let truncated = Outcome::Truncated {
            len: 1,
            unread_rows: 9,
        };
        assert_eq!(bounded_registry(truncated, 1), truncated);
    }

    #[test]
    fn the_stability_store_travels_with_the_slow_job_and_always_returns() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        for completed in [true, false] {
            let store = Box::new(crate::slicing::SliceFingerprints::new());
            let request = captured.take_slow_request(&countries, 11, 0x1000, 9, false, Some(store));
            assert!(request.stability_fingerprints.is_some());
            // Nothing was read: the unpublished read is invalidated or the
            // failed job's store forgotten, and the store comes back either way.
            assert!(captured.restore_slow_request(request, completed).is_some());
            assert!(!matches!(
                captured.country_stability.outcomes()[0],
                Outcome::Rows { .. }
            ));
        }
        let request = captured.take_slow_request(&countries, 11, 0x1000, 9, false, None);
        assert!(captured.restore_slow_request(request, true).is_none());
    }

    #[test]
    fn modifier_header_verdict_travels_with_the_slow_pack_and_returns() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        captured.set_modifier_definitions_moved(true);
        let request = captured.take_slow_request(&countries, 11, 0x1000, 9, false, None);
        assert!(request.packs.modifier_definitions.header_changed);
        assert!(!captured.modifier_definitions.header_changed);
        captured.restore_slow_request(request, true);
        assert!(captured.modifier_definitions.header_changed);
        captured.set_modifier_definitions_moved(false);
        assert!(!captured.modifier_definitions.header_changed);
    }

    #[test]
    fn focus_pilot_handoff_reaches_the_governance_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.focus_pilot().focus_raw_mode = true;
        assert!(captured.governance.focus_raw_mode);
        captured.focus_pilot().focus_raw_mode = false;
        assert!(!captured.governance.focus_raw_mode);
    }

    #[test]
    fn relations_pilot_handoff_reaches_the_intel_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.relations_pilot().relations_raw_mode = true;
        assert!(captured.intel.relations_raw_mode);
        captured.relations_pilot().relations_raw_mode = false;
        assert!(!captured.intel.relations_raw_mode);
    }

    #[test]
    fn characters_pilot_handoff_reaches_the_characters_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.characters_pilot().raw_active = [true; 6];
        assert!(captured.characters.raw_active.iter().any(|&active| active));
        captured.characters_pilot().raw_active = [false; 6];
        assert!(!captured.characters.raw_active.iter().any(|&active| active));
    }

    #[test]
    fn projects_pilot_handoff_reaches_the_projects_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.projects_pilot().raw_mode = true;
        assert!(captured.projects.raw_mode);
        captured.projects_pilot().raw_mode = false;
        assert!(!captured.projects.raw_mode);
    }

    #[test]
    fn lexical_pilot_handoff_reaches_the_lexical_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.lexical_pilot().raw_mode = true;
        assert!(captured.lexical.raw_mode);
        captured.lexical_pilot().raw_mode = false;
        assert!(!captured.lexical.raw_mode);
    }

    #[test]
    fn production_handoffs_preserve_detail_failures_without_live_retry() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("accepted division detail failure must not retry live reads");
            }
        }
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.production_rest_pilot().rest_raw_mode = true;
        assert!(captured.production.rest_raw_mode);
        captured.production_rest_pilot().rest_raw_mode = false;
        assert!(!captured.production.rest_raw_mode);
        captured.test_resize_division_buffers([1; 6]);
        let tables = [
            TableId::DivisionEquipment,
            TableId::ArmyUpgradeGroups,
            TableId::ArmyUpgradeRequests,
            TableId::ArmyUpgradeDeliveries,
            TableId::ArmyUpgradeProduced,
        ];
        for reason in [
            "accepted division acquisition panicked",
            "owned division payload unavailable",
        ] {
            captured.reset();
            captured.set_divisions_raw_mode(true);
            captured.production_rest_pilot().rest_raw_mode = true;
            for outcome in [
                &mut captured.production.division_equipment.outcome,
                &mut captured.production.groups.outcome,
                &mut captured.production.requests.outcome,
                &mut captured.production.deliveries.outcome,
                &mut captured.production.produced.outcome,
            ] {
                *outcome = Outcome::Rows { len: 1 };
            }
            captured.production.division_equipment.rows[0].division = 17;
            let memory = SourceReads::default();
            captured.capture_tables_all(&memory, 11, 0x1000, 2, false);
            assert!(!memory.reads.borrow().is_empty());
            let before = captured.outcomes();
            assert!(
                tables
                    .iter()
                    .all(|table| before[table.index()] == Outcome::Rows { len: 1 })
            );
            let bytes = captured.allocated_bytes();
            let capacities = captured.division_capture_capacities();

            captured.fail_division_details(reason);
            let outcomes = captured.outcomes();
            for (index, outcome) in outcomes.into_iter().enumerate() {
                if tables.iter().any(|table| table.index() == index) {
                    assert_eq!(outcome, Outcome::Unavailable { reason });
                } else {
                    assert_eq!(outcome, before[index], "unowned table {index}");
                }
            }
            for (table, view) in production::TABLES
                .into_iter()
                .zip(captured.production.tables())
            {
                if tables.contains(&table) {
                    assert_eq!(view, crate::spooling::unavailable(reason));
                }
            }
            assert!(captured.production.divisions_raw_mode);
            let ticks = captured.production.acquire(
                &NoReads,
                11,
                0x1000,
                captured.divisions.sources(),
                2,
                false,
            );
            assert_eq!(ticks, [0; production::N]);
            assert_eq!(captured.outcomes(), outcomes);
            assert_eq!(captured.allocated_bytes(), bytes);
            assert_eq!(captured.division_capture_capacities(), capacities);
            assert_eq!(captured.production.division_equipment.rows[0].division, 17);
        }
    }

    #[test]
    fn construction_rest_pilot_handoff_reaches_the_construction_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.construction_rest_pilot().rest_raw_mode = true;
        assert!(captured.construction.rest_raw_mode);
        captured.construction_rest_pilot().rest_raw_mode = false;
        assert!(!captured.construction.rest_raw_mode);
    }

    #[test]
    fn economy_rest_pilot_handoff_reaches_the_economy_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.economy_rest_pilot().rest_raw_mode = true;
        assert!(captured.economy.rest_raw_mode);
        captured.economy_rest_pilot().rest_raw_mode = false;
        assert!(!captured.economy.rest_raw_mode);
    }

    #[derive(Default)]
    struct SourceReads {
        reads: std::cell::RefCell<Vec<(usize, usize)>>,
        bytes: Option<Vec<u8>>,
    }
    impl Memory for SourceReads {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            self.bytes.as_ref().is_some_and(|bytes| {
                engine::memory::SliceMemory {
                    base: air::wing_test_kit::BASE,
                    bytes,
                }
                .copy(address, destination)
            })
        }
    }

    #[test]
    fn source_adapter_routes_each_staged_consumer_without_replacing_faults() {
        let capacity = Capacities::default();
        let mut actual = Captured::with_capacities(&capacity);
        let mut expected = Captured::with_capacities(&capacity);
        let row = cair_wing::Row {
            id: 0x1234_5678_0000_0009,
            ..Default::default()
        };
        // All four raw-admission combinations retain the direct consumers'
        // read order, faults and nonempty rows from the existing typed fixture.
        for readable in [false, true] {
            for air_raw in [false, true] {
                for naval_raw in [false, true] {
                    actual.reset();
                    expected.reset();
                    let memory = SourceReads {
                        bytes: readable.then(air::wing_test_kit::fixture),
                        ..Default::default()
                    };
                    let reference_memory = SourceReads {
                        bytes: readable.then(air::wing_test_kit::fixture),
                        ..Default::default()
                    };
                    {
                        let mut sources = actual.sources(&memory, 0, air_raw, naval_raw);
                        sources.begin_air();
                        sources.air(air::wing_test_kit::WING, 0, &row);
                        sources.finish_air(Outcome::Rows { len: 1 });
                    }

                    // Independent direct consumers define the reference rows
                    // and ordered native reads for this source admission.
                    if !air_raw {
                        expected.air.begin_wings();
                        expected.air.visit_wing(
                            &reference_memory,
                            0,
                            air::WingSource {
                                pointer: air::wing_test_kit::WING,
                                row: &row,
                            },
                        );
                        expected.air_root = Some(Outcome::Rows { len: 1 });
                    }
                    if !naval_raw {
                        expected.naval_history.visit_wing(
                            &reference_memory,
                            air::wing_test_kit::WING,
                            &row,
                        );
                        expected
                            .naval_history
                            .finish_wings(Outcome::Rows { len: 1 });
                    }
                    assert_eq!(*memory.reads.borrow(), *reference_memory.reads.borrow());
                    assert_eq!(actual.air_root, expected.air_root);
                    assert_eq!(actual.air.tables(), expected.air.tables());
                    assert_eq!(
                        actual.naval_history.tables(),
                        expected.naval_history.tables()
                    );
                    if readable && !air_raw {
                        assert_eq!(actual.air.outcomes()[0], Outcome::Rows { len: 2 });
                        assert_eq!(actual.air.outcomes()[2], Outcome::Rows { len: 1 });
                    }
                    if air_raw && naval_raw {
                        assert!(memory.reads.borrow().is_empty());
                        assert_eq!(actual.air_root, None);
                    }
                }
            }
        }
    }
}
