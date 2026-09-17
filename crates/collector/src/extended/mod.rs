//! Coordinated extended acquisition. Public arrays use global TableId indices;
//! individual packs keep compact local arrays. Allocation is off-thread.

#[cfg(test)]
use crate::sampling::CONSISTENCY_TARGET;
use crate::{
    division_census::UnitInputs,
    map_capture::{MapSources, SourceChecks},
    sampling::qpc,
    table_capture::{CaptureRows, comparison, comparison_rows, grow_capacity, grow_vec},
};
use engine::{
    Memory, cair_wing, carmy, cequipment_variant, cequipment_variant_pool::DivisionSource,
    corders_group, cprovince, cship, walk::Outcome,
};
use replay::{
    frame::{BaselineToken, TABLE_COUNT, TableInput},
    rows::{ScriptVariablesRow, TableId},
};
use std::{mem::size_of, sync::Arc};

const MISSING: Outcome = Outcome::Unavailable {
    reason: "acquisition source not supplied",
};
const VARIANT_TABLES: [TableId; 4] = [
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantStats,
    TableId::VariantUpgrades,
];
const DIVISION_SOURCE_TABLES: [TableId; 10] = [
    TableId::DivisionEquipment,
    TableId::ArmyUpgradeGroups,
    TableId::ArmyUpgradeRequests,
    TableId::ArmyUpgradeDeliveries,
    TableId::ArmyUpgradeProduced,
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::DivisionEta,
    TableId::ArmyIconInputs,
    TableId::DivisionOrderCaches,
];
pub type ReadGroup = (&'static str, TableId, Vec<TableId>);

// Native conversion reuse stays within its pack's existing timing scope.
macro_rules! spool_inputs {
    (lexical, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($lexical)
    };
    (characters, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($characters)
    };
    (intel, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($baseline, $scratch, $intel)
    };
    (governance, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($research)
    };
    (division_presentation, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    (division_eta, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    (production, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    (economy, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    (fog, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
        $captured.table_inputs($projection.as_deref_mut())
    };
    ($pack:ident, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr, $intel:expr, $projection:expr) => {
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
            unit_inputs: [Option<Arc<UnitInputs>>; 2],
            unit_inputs_match: bool,
            division_detail_source_faults: [Option<&'static str>; 5],
            eta_report: Option<division_eta::AcquisitionReport>,
            governance_report: Option<governance::CaptureReport>,
            presentation_report: Option<division_presentation::CaptureReport>,
            intel_residual_report: Option<intel::CaptureReport>,
            supply_trains_report: Option<supply_trains::CaptureReport>,
            character_portraits_report: Option<character_portraits::CaptureReport>,
            province_unit_order_report: Option<province_unit_order::CaptureReport>,
            ship_models_report: Option<ship_models::CaptureReport>,
            registry: Registry,
            air_roots: [Option<Outcome>; 2],
            overhead: [u64; TABLE_COUNT],
        }
        impl Captured {
            #[cfg(test)]
            pub(crate) fn set_intel_shadow_fixture(&mut self, value: i64) {
                self.intel = intel::intel_shadow_fixture(value);
            }

            pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
                assert_eq!(successful.len(), TABLE_COUNT);
                let mut captured = Self {
                    $($pack: $pack::Captured::with_capacities(&capacity.$pack, &local(&$pack::TABLES, successful)),)+
                    divisions: Divisions::new(capacity.division_sources, division_successful(successful)),
                    unit_inputs: [None, None],
                    unit_inputs_match: false,
                    division_detail_source_faults: [None; 5],
                    eta_report: None,
                    governance_report: None,
                    presentation_report: None,
                    intel_residual_report: None,
                    supply_trains_report: None,
                    character_portraits_report: None,
                    province_unit_order_report: None,
                    ship_models_report: None,
                    registry: Registry::new(capacity.variant_registry, variant_successful(successful)),
                    air_roots: [None; 2], overhead: [0; TABLE_COUNT],
                };
                captured.division_presentation.reserve_sources(capacity.division_sources);
                captured.division_eta.reserve_sources(capacity.division_sources);
                captured
            }
            /// Reset before the shared division and map walks, not between
            /// source visitation and capture().
            pub fn reset(&mut self) {
                $(self.$pack.reset();)+
                self.divisions.reset(); self.registry.reset();
                self.release_unit_inputs(); self.unit_inputs_match = false; self.eta_report = None; self.governance_report = None; self.presentation_report = None;
                self.intel_residual_report = None;
                self.supply_trains_report = None; self.character_portraits_report = None; self.province_unit_order_report = None; self.ship_models_report = None;
                self.division_detail_source_faults = [None; 5];
                self.air_roots = [None; 2]; self.overhead = [0; TABLE_COUNT];
            }
            pub fn allocated_bytes(&self) -> usize {
                0 $(+ self.$pack.allocated_bytes())+ + self.divisions.allocated_bytes() + self.registry.allocated_bytes()
            }
            /// Spool-thread metadata for this buffer's native row and scratch
            /// allocations. Excludes core tables, encoded frames and queues.
            pub fn allocated_by_pack(&self) -> Vec<(&'static str, usize)> {
                vec![
                    $((stringify!($pack), self.$pack.allocated_bytes()),)+
                    ("division_sources", self.divisions.allocated_bytes()),
                    ("variant_registry", self.registry.allocated_bytes()),
                ]
            }
            pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
                assert_eq!(successful.len(), TABLE_COUNT);
                $(self.$pack.grow(&mut capacity.$pack, &local(&$pack::TABLES, successful));)+
                self.divisions.grow(&mut capacity.division_sources, division_successful(successful));
                self.division_presentation.reserve_sources(capacity.division_sources);
                self.division_eta.reserve_sources(capacity.division_sources);
                self.registry.grow(&mut capacity.variant_registry, variant_successful(successful));
            }
            /// Spool-thread conversion only. Existing core slots survive.
            #[allow(clippy::too_many_arguments)]
            pub(crate) fn insert_tables(
                &self,
                destination: &mut [TableInput; TABLE_COUNT],
                variables_baseline: Option<(BaselineToken, &[ScriptVariablesRow])>,
                variables_scratch: &mut intel::ScriptVariableComparison,
                research: Option<&mut governance::ResearchConversion<'_>>,
                characters: Option<&mut characters::CharacterConversion<'_>>,
                lexical: Option<&mut lexical::LexicalConversion<'_>>,
                intel: Option<&mut intel::IntelConversion<'_>>,
                mut projection: Option<&mut crate::projection::Projection<'_>>,
            ) -> [i64; PACK_NAMES.len()] {
                let mut costs = [0; PACK_NAMES.len()];
                let mut slots = costs.iter_mut();
                $(let started = qpc();
                for (id, view) in $pack::TABLES.into_iter().zip(spool_inputs!($pack, self.$pack, variables_baseline, variables_scratch, research, characters, lexical, intel, projection)) { destination[id.index()] = view; }
                *slots.next().unwrap() = qpc() - started;)+
                costs
            }
            pub fn outcomes(&self) -> [Outcome; TABLE_COUNT] {
                let mut output = [MISSING; TABLE_COUNT];
                $(scatter(&$pack::TABLES, self.$pack.outcomes(), &mut output);)+
                output
            }
            pub fn checked_mismatches(&self) -> [usize; TABLE_COUNT] {
                let mut output = [0; TABLE_COUNT];
                $(scatter(&$pack::TABLES, self.$pack.checked_mismatches(), &mut output);)+
                for table in VARIANT_TABLES {
                    output[table.index()] = output[table.index()].saturating_add(self.registry.rows.mismatches);
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
            groups.push(("divisions, equipment, upgrades, movement, counter status and ETA", TableId::Divisions,
                std::iter::once(TableId::Divisions).chain(DIVISION_SOURCE_TABLES).collect()));
            groups.push(("country census, appearance and supply trains", TableId::Countries,
                vec![TableId::Countries, TableId::CountryAppearance, TableId::SupplyNodes, TableId::TrainInventory]));
            groups.push(("ship census and model operands", TableId::Ships,
                vec![TableId::Ships, TableId::ShipModels]));
            groups.push(("states and hourly state flags", TableId::States,
                vec![TableId::States, TableId::StateFlags]));
            groups.push(("province identity and stored client visibility", TableId::Provinces,
                vec![TableId::Provinces, TableId::ProvinceVisibility]));
            groups.push(("army order census, plan operands, colors and front sections", TableId::ArmyOrders,
                vec![TableId::ArmyOrders, TableId::ArmyOrderPlans, TableId::OrdersGroupColors, TableId::Fronts]));
            groups
        }
    };
}
packs!(
    air,
    army_icon_inputs,
    division_order_caches,
    weather_passes,
    weather_neighbours,
    counter_state,
    naval_counters,
    country_counters,
    group_counters,
    leader_paths,
    country_appearance,
    ship_models,
    province_unit_order,
    counter_layers,
    character_portraits,
    leader_portraits,
    supply_trains,
    characters,
    combat_progress,
    combat_details,
    naval_details,
    naval_result_details,
    construction,
    division_eta,
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
    province_visibility,
    plans,
    session,
    state_flags,
    weather
);

impl Captured {
    pub(crate) fn capture_province_visibility(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        provinces: &[cprovince::Row],
        source: Outcome,
        needed: bool,
    ) -> (bool, u64) {
        self.province_visibility
            .capture(memory, base, state, provinces, source, needed)
    }

    /// Leases the same immutable native input packet to movement, status and
    /// ETA. The owning census accounts for its allocation exactly once.
    pub(crate) fn install_unit_inputs(
        &mut self,
        first: Option<Arc<UnitInputs>>,
        second: Option<Arc<UnitInputs>>,
        source_checks_match: bool,
    ) {
        self.unit_inputs = [first, second];
        self.unit_inputs_match = source_checks_match;
    }

    /// Call after all consumers join and their union of consumed fields is
    /// verified, before either census packet is cleared or returned to its pool.
    pub(crate) fn release_unit_inputs(&mut self) {
        self.unit_inputs = [None, None];
    }

    /// Movement, status and ETA masks use the same full census indices. A row
    /// excluded by one consumer's capacity remains addressable by the others.
    pub(crate) fn consumed_unit_seed_masks(&self, pass: usize) -> [&[u64]; 3] {
        let [movement, status] = self.division_presentation.consumed_seed_masks(pass != 0);
        [
            movement,
            status,
            self.division_eta.consumed_seed_masks(pass),
        ]
    }

    /// Caller must also AND these verdicts into any already-returned movement,
    /// status and ETA consistency credits before publishing the check windows.
    pub(crate) fn reject_changed_unit_inputs(&mut self, pass: usize, changed: &[u64]) -> [bool; 3] {
        let [movement, status] = self
            .division_presentation
            .reject_changed_seeds(pass != 0, changed);
        let eta = self.division_eta.reject_changed_seeds(pass, changed);
        if let Some(report) = &mut self.eta_report {
            report.matched &= eta;
        }
        [movement, status, eta]
    }

    /// Only owned native operands are projected here. All live reads and the
    /// terminal consumed-field checks have already finished on acquisition.
    pub(crate) fn decode_division_presentation_inputs(&mut self) -> [u64; 2] {
        self.division_presentation.decode_inputs()
    }

    pub(crate) fn take_presentation_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> crate::acquisition_jobs::PresentationRequest {
        self.presentation_report = None;
        let needed: [bool; 2] = local(&division_presentation::TABLES, needed);
        crate::acquisition_jobs::PresentationRequest {
            countries: Arc::clone(countries),
            context: division_presentation::CaptureContext {
                base,
                game_state: state,
                needed,
            },
            first: self.unit_inputs[0].as_ref().map(Arc::clone),
            second: needed
                .iter()
                .any(|&needed| needed)
                .then(|| self.unit_inputs[1].as_ref().map(Arc::clone))
                .flatten(),
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
            Err(failure) => request.captured.fail_owned_capture(
                request.context,
                match failure {
                    crate::acquisition::Failure::Panicked => {
                        "division presentation acquisition panicked"
                    }
                    crate::acquisition::Failure::Stopped => {
                        "division presentation acquisition stopped before reading"
                    }
                },
            ),
        });
        self.division_presentation = request.captured;
    }

    fn acquire_presentation(
        &mut self,
        memory: &impl Memory,
        context: division_presentation::CaptureContext,
    ) -> division_presentation::CaptureReport {
        let mut report = self.presentation_report.take().unwrap_or_else(|| {
            self.division_presentation.capture_owned(
                memory,
                context,
                unit_sources(&self.unit_inputs),
            )
        });
        if !self.unit_inputs_match {
            let source_match = self.division_presentation.reject_source_census();
            for (matched, source_match) in report.matched.iter_mut().zip(source_match) {
                *matched &= source_match;
            }
        }
        report
    }

    /// Loan the complete governance family after the current Focus delegation
    /// has been decided. The callback restores it before any table publication.
    pub(crate) fn take_governance_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        needed: &[bool],
    ) -> crate::acquisition_jobs::GovernanceRequest {
        self.governance_report = None;
        crate::acquisition_jobs::GovernanceRequest {
            countries: Arc::clone(countries),
            context: governance::CaptureContext {
                base,
                game_state: state,
                hour,
                force_full,
                needed: local(&governance::TABLES, needed),
            },
            captured: std::mem::replace(&mut self.governance, governance::Captured::vacant()),
        }
    }

    pub(crate) fn restore_governance_request(
        &mut self,
        mut request: crate::acquisition_jobs::GovernanceRequest,
        report: Result<governance::CaptureReport, crate::acquisition::Failure>,
    ) {
        self.governance_report = Some(match report {
            Ok(report) => report,
            Err(failure) => {
                request.captured.fail_owned_capture(
                    request.context,
                    match failure {
                        crate::acquisition::Failure::Panicked => "governance acquisition panicked",
                        crate::acquisition::Failure::Stopped => {
                            "governance acquisition stopped before reading"
                        }
                    },
                );
                governance::CaptureReport::default()
            }
        });
        self.governance = request.captured;
    }

    /// Loan existing hourly buffers before independent acquisition begins.
    /// Restore them before ordinary capture, table views, growth or publication.
    pub(crate) fn take_supply_trains_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> crate::acquisition_jobs::SupplyTrainsRequest {
        self.supply_trains_report = None;
        crate::acquisition_jobs::SupplyTrainsRequest {
            countries: Arc::clone(countries),
            context: supply_trains::CaptureContext {
                base,
                game_state: state,
                needed: local(&supply_trains::TABLES, needed),
            },
            captured: std::mem::replace(&mut self.supply_trains, supply_trains::Captured::vacant()),
        }
    }

    pub(crate) fn restore_supply_trains_request(
        &mut self,
        mut request: crate::acquisition_jobs::SupplyTrainsRequest,
        report: Result<supply_trains::CaptureReport, crate::acquisition::Failure>,
    ) {
        // A failed accepted job is terminal. Its zero ticks are unmeasured;
        // keeping Some prevents the ordinary stage from retrying native reads.
        self.supply_trains_report = Some(match report {
            Ok(report) => report,
            Err(failure) => request.captured.fail_owned_capture(
                request.context,
                match failure {
                    crate::acquisition::Failure::Panicked => "supply trains acquisition panicked",
                    crate::acquisition::Failure::Stopped => {
                        "supply trains acquisition stopped before reading"
                    }
                },
            ),
        });
        self.supply_trains = request.captured;
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

    pub(crate) fn take_character_portraits_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> crate::acquisition_jobs::CharacterPortraitsRequest {
        self.character_portraits_report = None;
        crate::acquisition_jobs::CharacterPortraitsRequest {
            countries: Arc::clone(countries),
            context: character_portraits::CaptureContext {
                base,
                game_state: state,
                needed: local(&character_portraits::TABLES, needed),
            },
            captured: std::mem::replace(
                &mut self.character_portraits,
                character_portraits::Captured::vacant(),
            ),
        }
    }

    pub(crate) fn restore_character_portraits_request(
        &mut self,
        mut request: crate::acquisition_jobs::CharacterPortraitsRequest,
        report: Result<character_portraits::CaptureReport, crate::acquisition::Failure>,
    ) {
        self.character_portraits_report = Some(match report {
            Ok(report) => report,
            Err(failure) => request.captured.fail_owned_capture(
                request.context,
                match failure {
                    crate::acquisition::Failure::Panicked => {
                        "character portraits acquisition panicked"
                    }
                    crate::acquisition::Failure::Stopped => {
                        "character portraits acquisition stopped before reading"
                    }
                },
            ),
        });
        self.character_portraits = request.captured;
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
    /// acquisition pool. Restore before capture_before_eta publishes this pack.
    pub(crate) fn take_province_unit_order_request(
        &mut self,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> crate::acquisition_jobs::ProvinceUnitOrderRequest {
        self.province_unit_order_report = None;
        crate::acquisition_jobs::ProvinceUnitOrderRequest {
            context: province_unit_order::CaptureContext {
                base,
                game_state: state,
                needed: local(&province_unit_order::TABLES, needed),
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
        // Some, including terminal failure, prevents capture_before_eta from
        // retrying an accepted request. Zero failure ticks are unmeasured.
        self.province_unit_order_report = Some(match report {
            Ok(report) => report,
            Err(failure) => request.captured.fail_owned_capture(
                request.context,
                match failure {
                    crate::acquisition::Failure::Panicked => {
                        "province unit order acquisition panicked"
                    }
                    crate::acquisition::Failure::Stopped => {
                        "province unit order acquisition stopped before reading"
                    }
                },
            ),
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

    /// Loan ShipModels only after the map's native ship census has produced its
    /// ordered first/second seeds. Restore before terminal verification and any
    /// table publication; accepted worker failures are terminal and never retry.
    pub(crate) fn take_ship_models_request(
        &mut self,
        countries: &Arc<engine::country_memory::Cache>,
        base: usize,
        needed: &[bool],
    ) -> crate::acquisition_jobs::ShipModelsRequest {
        self.ship_models_report = None;
        crate::acquisition_jobs::ShipModelsRequest {
            countries: Arc::clone(countries),
            context: ship_models::CaptureContext {
                base,
                needed: local(&ship_models::TABLES, needed),
            },
            captured: std::mem::replace(&mut self.ship_models, ship_models::Captured::vacant()),
        }
    }

    pub(crate) fn restore_ship_models_request(
        &mut self,
        mut request: crate::acquisition_jobs::ShipModelsRequest,
        report: Result<ship_models::CaptureReport, crate::acquisition::Failure>,
    ) {
        self.ship_models_report = Some(match report {
            Ok(report) => report,
            Err(failure) => request.captured.fail_owned_capture(
                request.context,
                match failure {
                    crate::acquisition::Failure::Panicked => "ship models acquisition panicked",
                    crate::acquisition::Failure::Stopped => {
                        "ship models acquisition stopped before reading"
                    }
                },
            ),
        });
        self.ship_models = request.captured;
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
                report.matched[index] = delegated.matched[index];
                report.ticks[index] = delegated.ticks[index];
            }
            report
        } else {
            let (matched, ticks) = self.intel.capture(
                memory,
                context.base,
                context.game_state,
                &context.needed,
                context.hour,
                context.force_full,
            );
            intel::CaptureReport { matched, ticks }
        }
    }

    pub(crate) fn prepare_eta_ranges(&mut self, checked: bool, max_ranges: usize) -> usize {
        self.eta_report = None;
        self.division_eta
            .prepare_ranges(&unit_sources(&self.unit_inputs), checked, max_ranges)
    }
    pub(crate) fn take_eta_range(&mut self, index: usize) -> division_eta::RangeCaptured {
        self.division_eta
            .take_range(index)
            .expect("prepared ETA range remains owned")
    }
    pub(crate) fn restore_eta_range(&mut self, index: usize, range: division_eta::RangeCaptured) {
        self.division_eta.restore_range(index, range);
    }
    pub(crate) fn finish_eta_ranges(&mut self) {
        self.eta_report = Some(self.division_eta.finish_ranges());
    }
    pub(crate) fn eta_diagnostics(&self) -> division_eta::CacheDiagnostics {
        self.division_eta.cache_diagnostics()
    }
    pub(crate) fn fail_eta_capture(&mut self, checked: bool, reason: &'static str) {
        self.division_eta.fail(reason, checked);
        self.eta_report = Some(division_eta::AcquisitionReport::default());
    }
    fn acquire_eta(
        &mut self,
        _memory: &impl Memory,
        _base: usize,
        _state: usize,
        needed: bool,
    ) -> ([bool; 1], [u64; 1]) {
        if let Some(report) = self.eta_report.take() {
            ([report.matched], [report.ticks])
        } else {
            #[cfg(test)]
            {
                self.division_eta.capture(
                    _memory,
                    _base,
                    _state,
                    &[needed],
                    unit_sources(&self.unit_inputs),
                )
            }
            #[cfg(not(test))]
            {
                self.division_eta
                    .fail("ETA acquisition was not completed", needed);
                ([false], [0])
            }
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

    /// Game-thread raw-mode gate for the hourly divisions group. Set every
    /// hour from the divisions copy report before `capture` runs the
    /// production acquire below.
    pub(crate) fn set_divisions_raw_mode(&mut self, raw: bool) {
        self.production.divisions_raw_mode = raw;
    }

    /// The worker installs owned native rows without replaying pointer reads.
    pub(crate) fn install_division_detail(
        &mut self,
        details: &mut crate::division_detail::OwnedDetails,
    ) {
        details.install(&mut self.production);
        self.apply_division_detail_source_faults();
    }

    /// Terminal shared-source verification can finish after typed detail
    /// acquisition. Retain its independent table failures through the owned-row
    /// handoff, including when only some second-pass checks remain active.
    pub(crate) fn reject_division_detail_source(
        &mut self,
        pass: usize,
        needed: &[bool],
        reason: &'static str,
    ) {
        assert!(pass < 2);
        assert_eq!(needed.len(), TABLE_COUNT);
        for (fault, table) in self.division_detail_source_faults.iter_mut().zip([
            TableId::DivisionEquipment,
            TableId::ArmyUpgradeGroups,
            TableId::ArmyUpgradeRequests,
            TableId::ArmyUpgradeDeliveries,
            TableId::ArmyUpgradeProduced,
        ]) {
            if pass == 0 || needed[table.index()] {
                fault.get_or_insert(reason);
            }
        }
        self.apply_division_detail_source_faults();
    }

    fn apply_division_detail_source_faults(&mut self) {
        for (outcome, reason) in [
            &mut self.production.division_equipment.outcome,
            &mut self.production.groups.outcome,
            &mut self.production.requests.outcome,
            &mut self.production.deliveries.outcome,
            &mut self.production.produced.outcome,
        ]
        .into_iter()
        .zip(self.division_detail_source_faults)
        {
            if let Some(reason) = reason {
                *outcome = Outcome::Unavailable { reason };
            }
        }
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

    pub fn source_checks(needed: &[bool]) -> SourceChecks {
        let air = local(&air::TABLES, needed);
        let naval = local(&naval_history::TABLES, needed);
        SourceChecks {
            land_combats: needed[TableId::LandCombatProgress.index()],
            ships: needed[TableId::ShipModels.index()],
            air_wings: air[0] || air[2] || naval_history::needs_wing_sources(&naval),
            order_groups: naval_history::needs_order_sources(&naval)
                || plans::TABLES.iter().any(|t| needed[t.index()]),
        }
    }
    pub fn needs_division_check(needed: &[bool]) -> bool {
        DIVISION_SOURCE_TABLES
            .iter()
            .any(|table| needed[table.index()])
    }
    pub fn needs_state_check(needed: &[bool]) -> bool {
        needed[TableId::StateFlags.index()]
    }
    /// Callback-scoped source copies are charged to the shared States census.
    pub fn visit_state(&mut self, source: engine::cstate::Source, second: bool) {
        self.state_flags.visit(source, second);
    }
    pub fn finish_states(&mut self, outcome: Outcome, second: bool) {
        self.state_flags.finish(outcome, second);
    }

    /// Source copies remain inside the enclosing Divisions measurement. The
    /// shared read group includes equipment and upgrade tables without per-row
    /// timers or repeated source censuses for individual tables.
    pub(crate) fn visit_division_source(&mut self, source: DivisionSource, second: bool) {
        self.divisions.visit_source(source, second);
    }
    #[cfg(test)]
    pub(crate) fn test_division_presentation_tables(&self) -> [replay::frame::TableView; 2] {
        self.division_presentation.tables()
    }
    pub fn finish_divisions(&mut self, outcome: Outcome, second: bool) -> u64 {
        self.divisions.finish(outcome, second);
        0
    }
    pub fn sources<'a, M: Memory>(
        &'a mut self,
        memory: &'a M,
        base: usize,
        needed: &[bool],
        air_raw: bool,
        naval_raw: bool,
    ) -> Sources<'a, M> {
        assert_eq!(needed.len(), TABLE_COUNT);
        let naval = local(&naval_history::TABLES, needed);
        let air_needed = local(&air::TABLES, needed);
        Sources {
            captured: self,
            memory,
            base,
            // Staged payloads, including faulted ones, own these consumers.
            // Static allocation checks above remain conservative for fallback.
            checks: SourceChecks {
                land_combats: needed[TableId::LandCombatProgress.index()],
                ships: needed[TableId::ShipModels.index()],
                air_wings: (!air_raw && (air_needed[0] || air_needed[2]))
                    || (!naval_raw && naval_history::needs_wing_sources(&naval)),
                order_groups: (!naval_raw && naval_history::needs_order_sources(&naval))
                    || plans::TABLES.iter().any(|t| needed[t.index()]),
            },
            plans_needed: local(&plans::TABLES, needed),
            air_needed,
            air_raw,
            naval_raw,
            naval_wing_second: naval_history::needs_wing_sources(&naval),
            naval_order_second: naval_history::needs_order_sources(&naval),
        }
    }
    /// Run after root division acquisition and map.sample. Returned arrays use
    /// global TableId indices; core slots remain zero. The registry census and
    /// variant details run together on their existing cadence.
    #[cfg(test)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        needed: &[bool],
        force_full: bool,
    ) -> ([bool; TABLE_COUNT], [u64; TABLE_COUNT]) {
        let (mut matched, mut ticks) =
            self.capture_before_eta(memory, base, game_state, current_hour, needed, force_full);
        let ships = TableId::ShipModels.index();
        (matched[ships], ticks[ships]) = self.capture_ship_models(
            memory,
            base,
            game_state,
            current_hour,
            needed[ships],
            force_full,
        );
        let eta = TableId::DivisionEta.index();
        (matched[eta], ticks[eta]) = self.capture_eta(memory, base, game_state, needed[eta]);
        self.decode_division_presentation_inputs();
        (matched, ticks)
    }

    /// All frame-owned acquisition independent of the moved ETA output. The
    /// callback can run this while ETA's owned worker request is outstanding.
    /// Caller must recover that request and call capture_eta before publishing.
    pub(crate) fn capture_before_eta(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        needed: &[bool],
        force_full: bool,
    ) -> ([bool; TABLE_COUNT], [u64; TABLE_COUNT]) {
        assert_eq!(needed.len(), TABLE_COUNT);
        let mut matched = [false; TABLE_COUNT];
        let mut ticks = [0; TABLE_COUNT];
        macro_rules! ordinary {
            ($($pack:ident),+ $(,)?) => {$(
                let (pass, cost) = self.$pack.capture(memory, base, game_state, &local(&$pack::TABLES, needed));
                scatter(&$pack::TABLES, pass, &mut matched);
                scatter(&$pack::TABLES, cost, &mut ticks);
            )+};
        }
        macro_rules! staggered {
            ($($pack:ident),+ $(,)?) => {$(
                let (pass, cost) = self.$pack.capture(memory, base, game_state, &local(&$pack::TABLES, needed), current_hour, force_full);
                scatter(&$pack::TABLES, pass, &mut matched);
                scatter(&$pack::TABLES, cost, &mut ticks);
            )+};
        }
        ordinary!(
            economy,
            fog,
            session,
            weather,
            combat_progress,
            combat_details,
            naval_details,
            naval_result_details,
            state_flags,
            country_appearance
        );
        let province_order_report = self.acquire_province_unit_order(
            memory,
            province_unit_order::CaptureContext {
                base,
                game_state,
                needed: local(&province_unit_order::TABLES, needed),
            },
        );
        scatter(
            &province_unit_order::TABLES,
            province_order_report.matched,
            &mut matched,
        );
        scatter(
            &province_unit_order::TABLES,
            province_order_report.ticks,
            &mut ticks,
        );
        ordinary!(counter_layers);
        let portraits_report = self.acquire_character_portraits(
            memory,
            character_portraits::CaptureContext {
                base,
                game_state,
                needed: local(&character_portraits::TABLES, needed),
            },
        );
        scatter(
            &character_portraits::TABLES,
            portraits_report.matched,
            &mut matched,
        );
        scatter(
            &character_portraits::TABLES,
            portraits_report.ticks,
            &mut ticks,
        );
        ordinary!(leader_portraits);
        let supply_report = self.acquire_supply_trains(
            memory,
            supply_trains::CaptureContext {
                base,
                game_state,
                needed: local(&supply_trains::TABLES, needed),
            },
        );
        scatter(&supply_trains::TABLES, supply_report.matched, &mut matched);
        scatter(&supply_trains::TABLES, supply_report.ticks, &mut ticks);
        ordinary!(
            plans,
            weather_passes,
            counter_state,
            naval_counters,
            country_counters,
            group_counters,
            leader_paths
        );
        staggered!(
            characters,
            construction,
            projects,
            map_names,
            division_templates,
            weather_neighbours
        );
        let intel_context = intel::CaptureContext {
            base,
            game_state,
            hour: current_hour,
            force_full,
            needed: local(&intel::TABLES, needed),
        };
        let intel_report = self.acquire_intel(memory, intel_context);
        scatter(&intel::TABLES, intel_report.matched, &mut matched);
        scatter(&intel::TABLES, intel_report.ticks, &mut ticks);
        let governance_report = self.governance_report.take().unwrap_or_else(|| {
            self.governance.capture_owned(
                memory,
                governance::CaptureContext {
                    base,
                    game_state,
                    needed: local(&governance::TABLES, needed),
                    hour: current_hour,
                    force_full,
                },
            )
        });
        scatter(&governance::TABLES, governance_report.matched, &mut matched);
        scatter(&governance::TABLES, governance_report.ticks, &mut ticks);
        let air_needed = local(&air::TABLES, needed);
        self.air.finish_wings(
            self.air_roots[0].unwrap_or(MISSING),
            self.air_roots[1],
            &air_needed,
        );
        let (pass, cost) = self.air.capture(memory, base, game_state, &air_needed);
        scatter(&air::TABLES, pass, &mut matched);
        scatter(&air::TABLES, cost, &mut ticks);
        let (pass, cost) = self.naval_history.capture(
            memory,
            base,
            game_state,
            current_hour,
            &local(&naval_history::TABLES, needed),
        );
        scatter(&naval_history::TABLES, pass, &mut matched);
        scatter(&naval_history::TABLES, cost, &mut ticks);

        let (pass, cost) = self.army_icon_inputs.capture(
            memory,
            base,
            needed[TableId::ArmyIconInputs.index()],
            self.divisions.sources(),
        );
        scatter(&army_icon_inputs::TABLES, pass, &mut matched);
        scatter(&army_icon_inputs::TABLES, cost, &mut ticks);
        let (pass, cost) = self.division_order_caches.capture(
            memory,
            base,
            needed[TableId::DivisionOrderCaches.index()],
            self.divisions.sources(),
        );
        scatter(&division_order_caches::TABLES, pass, &mut matched);
        scatter(&division_order_caches::TABLES, cost, &mut ticks);
        let production_needed = local(&production::TABLES, needed);
        let (pass, cost) = self.production.acquire(
            memory,
            base,
            game_state,
            &production_needed,
            self.divisions.sources(),
            current_hour,
            force_full,
        );
        scatter(&production::TABLES, pass, &mut matched);
        scatter(&production::TABLES, cost, &mut ticks);
        let report = self.acquire_presentation(
            memory,
            division_presentation::CaptureContext {
                base,
                game_state,
                needed: local(&division_presentation::TABLES, needed),
            },
        );
        scatter(&division_presentation::TABLES, report.matched, &mut matched);
        scatter(&division_presentation::TABLES, report.ticks, &mut ticks);
        // The saved campaign registry is authoritative. It includes variants
        // absent from units, current production and stockpiles. Preserve every
        // native entry and duplicate for shared off-thread key rejection.
        let variant_walk =
            replay::cadence::should_walk(TableId::VariantDefinitions, current_hour, force_full);
        if variant_walk {
            let begin = qpc();
            self.registry
                .acquire(memory, base, game_state, variant_needed(needed));
            ticks[TableId::VariantDefinitions.index()] += elapsed(begin);
        } else {
            self.registry.mark_skipped();
        }
        let (pass, cost) = self.production.finish_variants(
            memory,
            base,
            &production_needed,
            self.registry.sources(),
            current_hour,
            force_full,
        );
        for (index, table) in production::TABLES.iter().enumerate() {
            matched[table.index()] |= pass[index];
            ticks[table.index()] = ticks[table.index()].saturating_add(cost[index]);
        }
        staggered!(lexical);
        for table in TABLES {
            if matches!(table, TableId::ShipModels | TableId::DivisionEta) {
                continue;
            }
            ticks[table.index()] =
                ticks[table.index()].saturating_add(self.overhead[table.index()]);
            matched[table.index()] &= needed[table.index()]
                && replay::cadence::should_walk(table, current_hour, force_full);
        }
        (matched, ticks)
    }

    /// Finish the ShipModels family after its owned detail request has had the
    /// whole independent pre-ETA callback region to run. A rejected request is
    /// executed exactly once by the caller before this function. An accepted
    /// panic/stop is terminal: the restored report suppresses any detail retry,
    /// while retained modifier witnesses are still freshly verified here.
    pub(crate) fn capture_ship_models(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        needed: bool,
        force_full: bool,
    ) -> (bool, u64) {
        let context = ship_models::CaptureContext {
            base,
            needed: [needed],
        };
        let report = self.acquire_ship_models(memory, context);
        let (pass, terminal) = self
            .ship_models
            .capture(memory, base, game_state, &context.needed);
        let table = TableId::ShipModels;
        let failed = self.ship_models.has_terminal_failure();
        (
            pass[0] && needed && replay::cadence::should_walk(table, current_hour, force_full),
            if failed {
                // The terminal verifier still ran and cleared every retained
                // witness above, but an interrupted detail job makes the whole
                // table service duration unmeasured.
                0
            } else {
                report.ticks[0]
                    .saturating_add(terminal[0])
                    .saturating_add(self.overhead[table.index()])
            },
        )
    }

    /// Terminal ETA consumption after an accepted request has been joined. A
    /// rejected dispatch still runs the genuine synchronous acquisition here;
    /// terminal worker failures retain their unavailable result without retry.
    pub(crate) fn capture_eta(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: bool,
    ) -> (bool, u64) {
        let (mut pass, cost) = self.acquire_eta(memory, base, game_state, needed);
        if !self.unit_inputs_match {
            pass[0] &= self.division_eta.reject_source_census();
        }
        (
            pass[0] && needed,
            cost[0].saturating_add(self.overhead[TableId::DivisionEta.index()]),
        )
    }
}

pub struct Sources<'a, M: Memory> {
    captured: &'a mut Captured,
    memory: &'a M,
    base: usize,
    checks: SourceChecks,
    plans_needed: [bool; plans::N],
    air_needed: [bool; air::N],
    air_raw: bool,
    naval_raw: bool,
    naval_wing_second: bool,
    naval_order_second: bool,
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
    fn checks(&self) -> SourceChecks {
        self.checks
    }
    fn begin_land(&mut self, second: bool) -> u64 {
        if second && !self.checks.land_combats {
            return 0;
        }
        let begin = qpc();
        self.captured.combat_progress.begin(second);
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
    fn land(&mut self, second: bool, source: engine::cland_combat::Source) -> u64 {
        if second && !self.checks.land_combats {
            return 0;
        }
        let begin = qpc();
        self.captured
            .combat_progress
            .visit(self.memory, self.base, source, second);
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
    fn finish_land(&mut self, second: bool, outcome: Outcome) -> u64 {
        if second && !self.checks.land_combats {
            return 0;
        }
        let begin = qpc();
        self.captured.combat_progress.finish(second, outcome);
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
    fn begin_ships(&mut self, second: bool) -> u64 {
        if second && !self.checks.ships {
            return 0;
        }
        let begin = qpc();
        self.captured.ship_models.begin_sources(second);
        self.attribute_overhead(TableId::ShipModels, begin, 0)
    }
    fn ship(&mut self, second: bool, pointer: usize, row: &cship::Row) -> u64 {
        if second && !self.checks.ships {
            return 0;
        }
        let begin = qpc();
        self.captured.ship_models.visit_source(pointer, row, second);
        self.attribute_overhead(TableId::ShipModels, begin, 0)
    }
    fn finish_ships(&mut self, second: bool, outcome: Outcome) -> u64 {
        if second && !self.checks.ships {
            return 0;
        }
        let begin = qpc();
        self.captured.ship_models.finish_sources(second, outcome);
        self.attribute_overhead(TableId::ShipModels, begin, 0)
    }
    fn begin_air(&mut self, second: bool) -> u64 {
        if self.air_raw {
            return 0;
        }
        let begin = qpc();
        if !second {
            self.captured.air.begin_wings();
        }
        self.attribute_overhead(TableId::WingEquipment, begin, 0)
    }
    fn air(&mut self, second: bool, pointer: usize, _: usize, row: &cair_wing::Row) -> u64 {
        if self.air_raw && (self.naval_raw || (second && !self.naval_wing_second)) {
            return 0;
        }
        let begin = qpc();
        let before = self.wing_ticks();
        if !self.air_raw {
            self.captured.air.visit_wing(
                self.memory,
                self.base,
                air::WingSource {
                    pointer,
                    row,
                    second,
                },
                &self.air_needed,
            );
        }
        if !self.naval_raw && (!second || self.naval_wing_second) {
            self.captured
                .naval_history
                .visit_wing(self.memory, self.base, pointer, row, second);
        }
        let charged = self.wing_ticks().saturating_sub(before);
        self.attribute_overhead(TableId::WingEquipment, begin, charged)
    }
    fn finish_air(&mut self, second: bool, outcome: Outcome) -> u64 {
        if self.air_raw && (self.naval_raw || (second && !self.naval_wing_second)) {
            return 0;
        }
        let begin = qpc();
        let before = self.wing_ticks();
        if !self.air_raw {
            self.captured.air_roots[usize::from(second)] = Some(outcome);
        }
        if !self.naval_raw && (!second || self.naval_wing_second) {
            self.captured.naval_history.finish_wings(outcome, second);
        }
        let charged = self.wing_ticks().saturating_sub(before);
        self.attribute_overhead(TableId::WingEquipment, begin, charged)
    }
    fn plans(
        &mut self,
        second: bool,
        roots: &[corders_group::Root],
        visits: &[engine::corder_instance::Visit],
        root_source: Outcome,
        order_source: Outcome,
    ) -> u64 {
        if second && !self.plans_needed.iter().any(|v| *v) {
            return 0;
        }
        let begin = qpc();
        let charged = self.captured.plans.visit(
            self.memory,
            self.base,
            roots,
            visits,
            root_source,
            order_source,
            second,
            &self.plans_needed,
        );
        self.attribute_overhead(TableId::ArmyOrderPlans, begin, charged)
    }
    fn order_groups(
        &mut self,
        second: bool,
        roots: &[corders_group::Root],
        outcome: Outcome,
    ) -> u64 {
        if self.naval_raw || (second && !self.naval_order_second) {
            return 0;
        }
        let begin = qpc();
        let ticks = self.captured.naval_history.source_ticks();
        let before = ticks[0] + ticks[2];
        self.captured.naval_history.set_order_sources(
            self.memory,
            self.base,
            roots,
            outcome,
            second,
        );
        let ticks = self.captured.naval_history.source_ticks();
        let charged = (ticks[0] + ticks[2]).saturating_sub(before);
        self.attribute_overhead(TableId::LandTheatres, begin, charged)
    }
}

fn unit_sources(
    inputs: &[Option<Arc<UnitInputs>>; 2],
) -> production::Sources<'_, carmy::DivisionSeed> {
    let first = inputs[0].as_deref();
    let second = inputs[1].as_deref();
    production::Sources {
        first: first.map_or(&[], UnitInputs::seeds),
        first_outcome: first.map_or(MISSING, UnitInputs::outcome),
        second: second.map_or(&[], UnitInputs::seeds),
        second_outcome: second.map_or(MISSING, UnitInputs::outcome),
    }
}

fn local<T: Copy, const N: usize>(tables: &[TableId; N], global: &[T]) -> [T; N] {
    tables.map(|table| global[table.index()])
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
fn variant_successful(successful: &[u16]) -> u16 {
    VARIANT_TABLES
        .iter()
        .map(|table| successful[table.index()])
        .min()
        .unwrap()
}
fn division_successful(successful: &[u16]) -> u16 {
    DIVISION_SOURCE_TABLES
        .iter()
        .map(|table| successful[table.index()])
        .min()
        .unwrap()
}
fn variant_needed(needed: &[bool]) -> bool {
    VARIANT_TABLES.iter().any(|table| needed[table.index()])
}
fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

struct Divisions {
    first: Vec<DivisionSource>,
    second: Vec<DivisionSource>,
    seen: [usize; 2],
    outcomes: [Outcome; 2],
    required: usize,
}
impl Divisions {
    fn new(capacity: usize, successful: u16) -> Self {
        Self {
            first: vec![Default::default(); capacity],
            second: comparison_rows(capacity, successful),
            seen: [0; 2],
            outcomes: [MISSING; 2],
            required: 0,
        }
    }
    fn reset(&mut self) {
        self.seen = [0; 2];
        self.outcomes = [MISSING; 2];
        self.required = 0;
    }
    fn visit_source(&mut self, source: DivisionSource, second: bool) {
        let pass = usize::from(second);
        let destination = if second {
            &mut self.second
        } else {
            &mut self.first
        };
        if let Some(slot) = destination.get_mut(self.seen[pass]) {
            *slot = source;
        }
        self.seen[pass] = self.seen[pass].saturating_add(1);
        self.required = self.required.max(self.seen[pass]);
    }
    fn finish(&mut self, outcome: Outcome, second: bool) {
        let pass = usize::from(second);
        let capacity = if second {
            self.second.len()
        } else {
            self.first.len()
        };
        self.outcomes[pass] = match outcome {
            Outcome::Rows { len } if len != self.seen[pass] => Outcome::Unavailable {
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
            first: &self.first,
            first_outcome: self.outcomes[0],
            second: &self.second,
            second_outcome: self.outcomes[1],
        }
    }
    fn grow(&mut self, capacity: &mut usize, successful: u16) {
        grow_capacity(capacity, self.required);
        grow_vec(&mut self.first, *capacity);
        comparison(&mut self.second, *capacity, successful);
    }
    fn allocated_bytes(&self) -> usize {
        (self.first.capacity() + self.second.capacity()) * size_of::<DivisionSource>()
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

struct Registry {
    rows: CaptureRows<cequipment_variant::RegistryRow>,
    outcomes: [Outcome; 2],
}
impl Registry {
    fn new(capacity: usize, successful: u16) -> Self {
        Self {
            rows: CaptureRows::new(capacity, successful),
            outcomes: [MISSING; 2],
        }
    }
    fn reset(&mut self) {
        self.rows.reset();
        self.outcomes = [MISSING; 2];
    }
    fn acquire(&mut self, memory: &impl Memory, base: usize, game_state: usize, needed: bool) {
        self.rows.outcome = bounded_registry(
            cequipment_variant::walk_registry(memory, base, game_state, &mut self.rows.rows),
            self.rows.rows.len(),
        );
        self.outcomes[0] = self.rows.outcome;
        if needed {
            self.rows.checked = true;
            let second = bounded_registry(
                cequipment_variant::walk_registry(memory, base, game_state, &mut self.rows.second),
                self.rows.second.len(),
            );
            self.outcomes[1] = second;
            self.rows.compare(second);
            if self.rows.mismatches != 0 {
                self.outcomes = [Outcome::Unavailable {
                    reason: "variant registry changed during double read",
                }; 2];
            }
        }
    }
    // Off-hours carry-forward hint keeps byte-identical spool output.
    fn mark_skipped(&mut self) {
        self.rows.outcome = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        self.outcomes = [self.rows.outcome; 2];
    }
    fn sources(&self) -> production::Sources<'_, cequipment_variant::RegistryRow> {
        production::Sources {
            first: &self.rows.rows,
            first_outcome: self.outcomes[0],
            second: &self.rows.second,
            second_outcome: self.outcomes[1],
        }
    }
    fn grow(&mut self, capacity: &mut usize, successful: u16) {
        self.rows.grow(capacity, successful);
    }
    fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
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
    fn counter_input_owned_capacity_costs_include_comparison_storage() {
        macro_rules! cost {
            ($pack:ident) => {{
                let capacities = $pack::Capacities::default();
                let cold = $pack::Captured::with_capacities(&capacities, &[0]).allocated_bytes();
                let warm = $pack::Captured::with_capacities(
                    &capacities,
                    &[crate::sampling::CONSISTENCY_TARGET],
                )
                .allocated_bytes();
                assert!(cold >= warm);
                assert!(warm > 0);
                println!(
                    "{} owned bytes per pool slot: cold={} warm={}",
                    stringify!($pack),
                    cold,
                    warm
                );
            }};
        }
        cost!(province_unit_order);
        cost!(counter_layers);
        cost!(character_portraits);
        cost!(leader_portraits);
        cost!(ship_models);
        println!(
            "native ShipModels row={} bytes",
            std::mem::size_of::<engine::cship::ModelRow>()
        );
    }

    #[test]
    fn format24_owned_capacity_costs_include_comparison_storage_until_retirement() {
        macro_rules! cost {
            ($pack:ident) => {{
                let capacities = $pack::Capacities::default();
                let cold =
                    $pack::Captured::with_capacities(&capacities, &[0; $pack::N]).allocated_bytes();
                let warm = $pack::Captured::with_capacities(
                    &capacities,
                    &[crate::sampling::CONSISTENCY_TARGET; $pack::N],
                )
                .allocated_bytes();
                assert!(cold > warm && warm > 0);
                println!(
                    "{} owned bytes per pool slot: cold={} warm={}",
                    stringify!($pack),
                    cold,
                    warm
                );
                (cold, warm)
            }};
        }
        let totals = [
            cost!(weather_passes),
            cost!(weather_neighbours),
            cost!(counter_state),
            cost!(naval_counters),
            cost!(country_counters),
            cost!(group_counters),
            cost!(division_order_caches),
            cost!(leader_paths),
        ]
        .into_iter()
        .fold((0, 0), |sum, (cold, warm)| (sum.0 + cold, sum.1 + warm));
        println!(
            "format24 packs owned bytes per pool slot: cold={} warm={}",
            totals.0, totals.1
        );
    }

    #[test]
    fn independent_capture_leaves_eta_range_owned_until_terminal_reduction() {
        struct FailedReads(std::cell::Cell<usize>);
        impl Memory for FailedReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                false
            }
        }
        struct ProvinceReads(std::cell::Cell<usize>);
        impl Memory for ProvinceReads {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                assert_eq!(
                    (address, output.len()),
                    (0x1000 + engine::cgame_state::PROVINCES, 16)
                );
                self.0.set(self.0.get() + 1);
                output[..8].copy_from_slice(&0x2200_u64.to_le_bytes());
                output[8..12].copy_from_slice(&2_u32.to_le_bytes());
                output[12..16].copy_from_slice(&2_u32.to_le_bytes());
                true
            }
        }
        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("terminal ETA reduction must not retry live acquisition");
            }
        }
        for terminal in [
            None,
            Some("ETA acquisition panicked"),
            Some("ETA acquisition stopped"),
        ] {
            let mut captured = Captured::with_capacities(&Capacities::default(), &[0; TABLE_COUNT]);
            captured.install_unit_inputs(
                Some(Arc::new(UnitInputs::empty_rows())),
                Some(Arc::new(UnitInputs::empty_rows())),
                true,
            );
            let bytes = captured.allocated_bytes();
            assert_eq!(
                captured.prepare_eta_ranges(true, division_eta::MAX_RANGES),
                1
            );
            let mut range = captured.take_eta_range(0);
            assert!(captured.allocated_bytes() < bytes);
            let eta = TableId::DivisionEta.index();
            let overhead = captured.overhead[eta];
            let independent = FailedReads(std::cell::Cell::new(0));
            let (checks, ticks) =
                captured.capture_before_eta(&independent, 0, 0x1000, 0, &[true; TABLE_COUNT], true);
            assert!(
                independent.0.get() > 0,
                "independent capture still runs while ETA is owned elsewhere"
            );
            assert!(!checks[eta]);
            assert_eq!(ticks[eta], 0);
            assert_eq!(captured.overhead[eta], overhead);
            assert!(captured.eta_report.is_none());
            if let Some(reason) = terminal {
                for pass in 0..2 {
                    range.fail(reason, pass, true);
                }
            } else {
                let mut workspace = division_eta::Workspace::with_capacities(
                    &division_eta::Captured::empty().workspace_capacities(),
                    true,
                );
                let memory = ProvinceReads(std::cell::Cell::new(0));
                for pass in 0..2 {
                    range.acquire_pass(
                        &mut workspace,
                        &memory,
                        0,
                        0x1000,
                        &[],
                        Outcome::Rows { len: 0 },
                        pass,
                    );
                    range.verify_pass(&mut workspace, &memory, 0, 0x1000, pass);
                }
                assert_eq!(
                    memory.0.get(),
                    4,
                    "each cold pass independently acquires and verifies its province root"
                );
            }
            captured.restore_eta_range(0, range);
            assert_eq!(captured.allocated_bytes(), bytes);
            captured.finish_eta_ranges();
            let report = captured.eta_report.unwrap();
            assert_eq!(report.matched, terminal.is_none());
            assert_eq!(
                captured.capture_eta(&NoRead, 0, 0x1000, true),
                (report.matched, overhead + report.ticks)
            );
            assert!(
                captured.eta_report.is_none(),
                "the reduced cost is consumed once"
            );
            if terminal.is_some() {
                assert!(matches!(
                    captured.division_eta.outcomes()[0],
                    Outcome::Unavailable { .. }
                ));
            } else {
                assert_eq!(captured.division_eta.outcomes(), [Outcome::Rows { len: 0 }]);
            }
        }
    }

    #[test]
    fn eta_range_handoff_retains_frame_sources_and_restores_storage_without_retry() {
        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("finished ETA range must not acquire live memory again");
            }
        }
        let capacity = Capacities {
            division_sources: 2,
            ..Capacities::default()
        };
        let mut captured = Captured::with_capacities(&capacity, &[0; TABLE_COUNT]);
        let first = Arc::new(UnitInputs::fixture(
            vec![carmy::DivisionSeed {
                pointer: 0x1234,
                handle: Some(7),
                ..Default::default()
            }],
            Outcome::Rows { len: 1 },
        ));
        let second = Arc::new(UnitInputs::fixture(
            vec![carmy::DivisionSeed {
                pointer: 0x5678,
                handle: Some(7),
                ..Default::default()
            }],
            Outcome::Rows { len: 1 },
        ));
        captured.install_unit_inputs(Some(Arc::clone(&first)), Some(Arc::clone(&second)), true);
        let bytes = captured.allocated_bytes();
        for checked in [true, false] {
            assert_eq!(captured.prepare_eta_ranges(checked, 1), 1);
            let mut range = captured.take_eta_range(0);
            assert!(captured.allocated_bytes() < bytes);
            // Range payloads contain owned operands; the job adapter owns any extra
            // source leases. Moving a range does not consume this frame's sources.
            assert_eq!(Arc::strong_count(&first), 2);
            assert_eq!(Arc::strong_count(&second), 2);
            assert_eq!(unit_sources(&captured.unit_inputs).first[0].pointer, 0x1234);
            assert_eq!(
                unit_sources(&captured.unit_inputs).second[0].pointer,
                0x5678
            );
            for pass in 0..=usize::from(checked) {
                range.fail("synthetic terminal ETA", pass, true);
            }
            captured.restore_eta_range(0, range);
            captured.finish_eta_ranges();
            let report = captured.eta_report.unwrap();
            assert!(!report.matched);
            assert_eq!(
                captured.acquire_eta(&NoRead, 11, 22, checked),
                ([false], [report.ticks])
            );
            assert!(matches!(
                captured.division_eta.outcomes()[0],
                Outcome::Unavailable { .. }
            ));
            assert_eq!(captured.allocated_bytes(), bytes);
            assert_eq!(Arc::strong_count(&first), 2);
            assert_eq!(Arc::strong_count(&second), 2);
        }
        captured.release_unit_inputs();
        assert_eq!(Arc::strong_count(&first), 1);
        assert_eq!(Arc::strong_count(&second), 1);
        captured.reset();
        assert!(captured.eta_report.is_none());
    }

    #[test]
    fn presentation_handoff_restores_storage_and_exact_source_leases() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; TABLE_COUNT]);
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let first = Arc::new(UnitInputs::fixture(
            vec![carmy::DivisionSeed {
                pointer: 0x1234,
                handle: Some(7),
                ..Default::default()
            }],
            Outcome::Rows { len: 1 },
        ));
        let second = Arc::new(UnitInputs::fixture(
            vec![carmy::DivisionSeed {
                pointer: 0x5678,
                handle: Some(11),
                ..Default::default()
            }],
            Outcome::Rows { len: 1 },
        ));
        let bytes = captured.allocated_bytes();
        let presentation_bytes = captured.division_presentation.allocated_bytes();
        assert!(presentation_bytes > 0);
        let report = division_presentation::CaptureReport {
            matched: [true, false],
            ticks: [89, 97],
        };
        for checked in [[false, false], [true, false], [false, true], [true, true]] {
            captured.install_unit_inputs(Some(Arc::clone(&first)), Some(Arc::clone(&second)), true);
            captured.presentation_report = Some(report);
            let mut needed = [false; TABLE_COUNT];
            for (table, checked) in division_presentation::TABLES.into_iter().zip(checked) {
                needed[table.index()] = checked;
            }
            let request = captured.take_presentation_request(&countries, 11, 0x1000, &needed);
            assert!(captured.presentation_report.is_none());
            assert_eq!(captured.division_presentation.allocated_bytes(), 0);
            assert_eq!(captured.allocated_bytes() + presentation_bytes, bytes);
            assert_eq!(request.captured.allocated_bytes(), presentation_bytes);
            assert_eq!(
                (request.context.base, request.context.game_state),
                (11, 0x1000)
            );
            assert_eq!(request.context.needed, checked);
            assert!(Arc::ptr_eq(&request.countries, &countries));
            assert!(Arc::ptr_eq(request.first.as_ref().unwrap(), &first));
            let checking = checked.into_iter().any(|checked| checked);
            if checking {
                assert!(Arc::ptr_eq(request.second.as_ref().unwrap(), &second));
                assert_ne!(
                    request.first.as_ref().unwrap().seeds()[0].pointer,
                    request.second.as_ref().unwrap().seeds()[0].pointer
                );
            } else {
                assert!(request.second.is_none());
            }
            assert_eq!(Arc::strong_count(&countries), 2);
            assert_eq!(Arc::strong_count(&first), 3);
            assert_eq!(Arc::strong_count(&second), 2 + usize::from(checking));
            captured.restore_presentation_request(request, Ok(report));
            assert_eq!(captured.allocated_bytes(), bytes);
            assert_eq!(
                captured.division_presentation.allocated_bytes(),
                presentation_bytes
            );
            assert_eq!(Arc::strong_count(&countries), 1);
            assert_eq!(Arc::strong_count(&first), 2);
            assert_eq!(Arc::strong_count(&second), 2);
            assert!(Arc::ptr_eq(
                captured.unit_inputs[0].as_ref().unwrap(),
                &first
            ));
            assert!(Arc::ptr_eq(
                captured.unit_inputs[1].as_ref().unwrap(),
                &second
            ));
            let pending = captured.presentation_report.as_ref().unwrap();
            assert_eq!(pending.matched, report.matched);
            assert_eq!(pending.ticks, report.ticks);
            // Reset must discard a pending report, not merely preserve an already
            // consumed None, and release the frame's independent input leases.
            captured.reset();
            assert!(captured.presentation_report.is_none());
            assert_eq!(captured.allocated_bytes(), bytes);
            assert_eq!(Arc::strong_count(&first), 1);
            assert_eq!(Arc::strong_count(&second), 1);
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
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; TABLE_COUNT]);
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let first = Arc::new(UnitInputs::empty_rows());
        let second = Arc::new(UnitInputs::new(0));
        let mut needed = [false; TABLE_COUNT];
        for table in division_presentation::TABLES {
            needed[table.index()] = true;
        }
        let bytes = captured.allocated_bytes();
        let report = division_presentation::CaptureReport {
            matched: [true, false],
            ticks: [113, 227],
        };
        for failure in [
            None,
            Some(crate::acquisition::Failure::Panicked),
            Some(crate::acquisition::Failure::Stopped),
        ] {
            captured.reset();
            captured.install_unit_inputs(Some(Arc::clone(&first)), Some(Arc::clone(&second)), true);
            let request = captured.take_presentation_request(&countries, 11, 0x1000, &needed);
            let context = request.context;
            captured.restore_presentation_request(request, failure.map_or(Ok(report), Err));
            let expected_outcomes = captured.division_presentation.outcomes();
            if failure.is_some() {
                assert!(
                    expected_outcomes
                        .iter()
                        .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
                );
                assert_eq!(captured.division_presentation.checked_mismatches(), [1; 2]);
            }
            let memory = NoPresentationRead {
                independent: std::cell::Cell::new(0),
                presentation: std::cell::Cell::new(0),
                forbid_presentation: std::cell::Cell::new(true),
            };
            let completed = captured.acquire_presentation(&memory, context);
            assert_eq!(memory.presentation.get(), 0);
            assert_eq!(memory.independent.get(), 0);
            assert!(captured.presentation_report.is_none());
            // Exercise the same completed result through global table routing,
            // where unrelated province consumers may read this native root.
            captured.presentation_report = Some(completed);
            memory.forbid_presentation.set(false);
            let (matched, ticks) =
                captured.capture_before_eta(&memory, 11, 0x1000, 0, &needed, false);
            assert!(memory.independent.get() > 0);
            assert!(
                captured.presentation_report.is_none(),
                "the report is consumed once"
            );
            assert_eq!(captured.division_presentation.outcomes(), expected_outcomes);
            for (slot, table) in division_presentation::TABLES.into_iter().enumerate() {
                assert_eq!(
                    matched[table.index()],
                    failure.is_none() && report.matched[slot]
                );
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
            captured.acquire_presentation(&memory, context);
            assert!(memory.presentation.get() > before);
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
            let mut captured = Captured::with_capacities(&Capacities::default(), &[0; TABLE_COUNT]);
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
                &[true; TABLE_COUNT],
            );
            assert!(captured.governance_report.is_none());
            assert_eq!(captured.governance.allocated_bytes(), 0);
            assert_eq!(captured.allocated_bytes() + governance_bytes, bytes);
            assert_eq!(request.captured.allocated_bytes(), governance_bytes);
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
            assert_eq!(request.context.needed, [true; governance::N]);
            let context = request.context;
            let report = governance::CaptureReport {
                matched: std::array::from_fn(|slot| slot % 2 == 0),
                ticks: std::array::from_fn(|slot| slot as u64 + 101),
            };
            captured.restore_governance_request(request, failure.map_or(Ok(report), Err));
            assert_eq!(Arc::strong_count(&countries), 1);
            assert_eq!(captured.allocated_bytes(), bytes);
            assert!(captured.governance.focus_raw_mode);
            let expected_outcomes = captured.governance.outcomes();
            let memory = Reads {
                total: std::cell::Cell::new(0),
                governance: std::cell::Cell::new(0),
            };
            let (matched, ticks) = captured.capture_before_eta(
                &memory,
                11,
                0x1000,
                1,
                &[true; TABLE_COUNT],
                context.force_full,
            );
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
                    matched[table.index()],
                    failure.is_none() && report.matched[slot]
                );
                assert_eq!(
                    ticks[table.index()],
                    if failure.is_none() {
                        report.ticks[slot]
                    } else {
                        0
                    }
                );
                if failure.is_some() && (2..=4).contains(&slot) {
                    assert_eq!(
                        expected_outcomes[slot],
                        Outcome::Unavailable {
                            reason: replay::cadence::SKIPPED_REASON
                        }
                    );
                }
            }
            // A direct control capture proves that the spy detects Governance work.
            captured.governance.capture_owned(
                &memory,
                governance::CaptureContext {
                    force_full: true,
                    ..context
                },
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
            let mut captured = Captured::with_capacities(&Capacities::default(), &[0; TABLE_COUNT]);
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
                needed: [true; intel::N],
            };
            let request = captured.take_intel_residual_request(&countries, context);
            assert_eq!(captured.intel.allocated_bytes(), 0);
            assert_eq!(captured.allocated_bytes() + family_bytes, bytes);
            assert_eq!(request.captured.allocated_bytes(), family_bytes);
            assert!(Arc::ptr_eq(&request.countries, &countries));
            let original = intel::CaptureReport {
                matched: [true; intel::N],
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
                assert_eq!(report.matched[index], failure.is_none());
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
                assert!(!report.matched[index]);
                assert_eq!(report.ticks[index], 0);
            }
            captured.reset();
            assert!(captured.intel_residual_report.is_none());
        }
    }

    #[test]
    fn ship_models_late_terminal_handoff_is_unavailable_consumed_once_and_keeps_cold_check() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("terminal ShipModels routing must not retry owned detail reads");
            }
        }
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; TABLE_COUNT]);
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let mut needed = [false; TABLE_COUNT];
        let table = TableId::ShipModels.index();
        needed[table] = true;
        for second in [false, true] {
            captured.ship_models.begin_sources(second);
            captured
                .ship_models
                .finish_sources(second, Outcome::Rows { len: 0 });
        }
        let request = captured.take_ship_models_request(&countries, 11, &needed);
        let family_bytes = request.captured.allocated_bytes();
        assert_eq!(captured.ship_models.allocated_bytes(), 0);
        captured.restore_ship_models_request(request, Err(crate::acquisition::Failure::Panicked));
        assert_eq!(captured.ship_models.allocated_bytes(), family_bytes);
        assert_eq!(
            captured.ship_models_report.unwrap().ticks,
            [0],
            "interrupted worker detail cost remains unmeasured"
        );
        let (matched, ticks) = captured.capture_ship_models(&NoReads, 11, 0x1000, 0, true, false);
        assert!(!matched);
        assert_eq!(ticks, 0, "failed whole-table service cost is unmeasured");
        assert!(
            captured.ship_models_report.is_none(),
            "terminal report is consumed once"
        );
        assert!(captured.ship_models.checked_mismatches()[0] > 0);
        assert!(matches!(
            captured.ship_models.outcomes()[0],
            Outcome::Unavailable {
                reason: "ship models acquisition panicked"
            }
        ));
        let (again, again_ticks) =
            captured.capture_ship_models(&NoReads, 11, 0x1000, 0, true, false);
        assert!(!again);
        assert_eq!(again_ticks, 0);
        assert!(captured.ship_models_report.is_none());
        assert!(matches!(
            captured.ship_models.outcomes()[0],
            Outcome::Unavailable {
                reason: "ship models acquisition panicked"
            }
        ));

        captured.reset();
        for second in [false, true] {
            captured.ship_models.begin_sources(second);
            captured
                .ship_models
                .finish_sources(second, Outcome::Rows { len: 0 });
        }
        let (fresh, _fresh_ticks) =
            captured.capture_ship_models(&NoReads, 11, 0x1000, 0, true, true);
        assert!(
            fresh,
            "a new complete source epoch clears the stale terminal failure"
        );
        assert_eq!(captured.ship_models.checked_mismatches(), [0]);
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
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; TABLE_COUNT]);
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let bytes = captured.allocated_bytes();
        let supply_bytes = captured.supply_trains.allocated_bytes();
        let portrait_bytes = captured.character_portraits.allocated_bytes();
        let province_order_bytes = captured.province_unit_order.allocated_bytes();
        for bits in 0_u32..16 {
            let mut needed = [false; TABLE_COUNT];
            for (bit, table) in [
                TableId::SupplyNodes,
                TableId::TrainInventory,
                TableId::CharacterPortraits,
                TableId::ProvinceUnitOrder,
            ]
            .into_iter()
            .enumerate()
            {
                needed[table.index()] = bits & (1 << bit) != 0;
            }
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
                let supply = captured.take_supply_trains_request(&countries, 11, 0x1000, &needed);
                assert!(captured.supply_trains_report.is_none());
                assert_eq!(captured.supply_trains.allocated_bytes(), 0);
                assert_eq!(
                    captured.character_portraits.allocated_bytes(),
                    portrait_bytes
                );
                assert_eq!(supply.captured.allocated_bytes(), supply_bytes);
                assert_eq!(captured.allocated_bytes() + supply_bytes, bytes);
                let portraits =
                    captured.take_character_portraits_request(&countries, 11, 0x1000, &needed);
                assert!(captured.character_portraits_report.is_none());
                assert_eq!(captured.character_portraits.allocated_bytes(), 0);
                assert_eq!(portraits.captured.allocated_bytes(), portrait_bytes);
                assert_eq!(
                    captured.allocated_bytes() + supply_bytes + portrait_bytes,
                    bytes
                );
                let province_order = captured.take_province_unit_order_request(11, 0x1000, &needed);
                assert!(captured.province_unit_order_report.is_none());
                assert_eq!(captured.province_unit_order.allocated_bytes(), 0);
                assert_eq!(
                    province_order.captured.allocated_bytes(),
                    province_order_bytes
                );
                assert_eq!(
                    captured.allocated_bytes()
                        + supply_bytes
                        + portrait_bytes
                        + province_order_bytes,
                    bytes
                );
                assert!(Arc::ptr_eq(&supply.countries, &countries));
                assert!(Arc::ptr_eq(&portraits.countries, &countries));
                assert_eq!(Arc::strong_count(&countries), 3);
                let supply_context = supply.context;
                let portrait_context = portraits.context;
                let province_order_context = province_order.context;
                assert_eq!(
                    supply_context.needed,
                    local(&supply_trains::TABLES, &needed)
                );
                assert_eq!(
                    portrait_context.needed,
                    local(&character_portraits::TABLES, &needed)
                );
                assert_eq!(
                    province_order_context.needed,
                    local(&province_unit_order::TABLES, &needed)
                );
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
                    matched: [supply_context.needed[0], false],
                    ticks: [101, 103],
                };
                let portrait_report = character_portraits::CaptureReport {
                    matched: portrait_context.needed,
                    ticks: [107],
                };
                let province_order_report = province_unit_order::CaptureReport {
                    matched: province_order_context.needed,
                    ticks: [109],
                };
                captured.restore_supply_trains_request(
                    supply,
                    supply_failure.map_or(Ok(supply_report), Err),
                );
                assert_eq!(Arc::strong_count(&countries), 2);
                assert!(captured.supply_trains_report.is_some());
                assert!(captured.character_portraits_report.is_none());
                captured.restore_character_portraits_request(
                    portraits,
                    portrait_failure.map_or(Ok(portrait_report), Err),
                );
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
                assert_eq!(
                    captured.supply_trains.checked_mismatches(),
                    supply_context
                        .needed
                        .map(|needed| usize::from(needed && supply_failure.is_some()))
                );
                assert_eq!(
                    captured.character_portraits.checked_mismatches(),
                    portrait_context
                        .needed
                        .map(|needed| usize::from(needed && portrait_failure.is_some()))
                );
                assert_eq!(
                    captured.province_unit_order.checked_mismatches(),
                    province_order_context
                        .needed
                        .map(|needed| usize::from(needed && province_order_failure.is_some()))
                );
                let supply = captured.acquire_supply_trains(&NoReads, supply_context);
                let portraits = captured.acquire_character_portraits(&NoReads, portrait_context);
                let province_order =
                    captured.acquire_province_unit_order(&NoReads, province_order_context);
                assert_eq!(
                    supply.matched,
                    if supply_failure.is_none() {
                        supply_report.matched
                    } else {
                        [false; 2]
                    }
                );
                assert_eq!(
                    portraits.matched,
                    if portrait_failure.is_none() {
                        portrait_report.matched
                    } else {
                        [false]
                    }
                );
                assert_eq!(
                    province_order.matched,
                    if province_order_failure.is_none() {
                        province_order_report.matched
                    } else {
                        [false]
                    }
                );
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
                let (matched, ticks) =
                    captured.capture_before_eta(&memory, 11, 0x1000, 1, &needed, false);
                assert!(memory.0.get() > 0);
                assert_eq!(local(&supply_trains::TABLES, &matched), supply.matched);
                assert_eq!(local(&supply_trains::TABLES, &ticks), supply.ticks);
                assert_eq!(
                    local(&character_portraits::TABLES, &matched),
                    portraits.matched
                );
                assert_eq!(local(&character_portraits::TABLES, &ticks), portraits.ticks);
                assert_eq!(
                    local(&province_unit_order::TABLES, &matched),
                    province_order.matched
                );
                assert_eq!(
                    local(&province_unit_order::TABLES, &ticks),
                    province_order.ticks
                );
                assert_eq!(
                    local(&supply_trains::TABLES, &captured.outcomes()),
                    local(&supply_trains::TABLES, &outcomes)
                );
                assert_eq!(
                    local(&character_portraits::TABLES, &captured.outcomes()),
                    local(&character_portraits::TABLES, &outcomes)
                );
                assert_eq!(
                    local(&province_unit_order::TABLES, &captured.outcomes()),
                    local(&province_unit_order::TABLES, &outcomes)
                );
                assert!(captured.supply_trains_report.is_none());
                assert!(captured.character_portraits_report.is_none());
                assert!(captured.province_unit_order_report.is_none());
                // With no loan or terminal report, the same helpers still run
                // the direct capture and its requested comparison passes.
                let direct = Reads::default();
                captured.acquire_supply_trains(&direct, supply_context);
                captured.acquire_character_portraits(&direct, portrait_context);
                captured.acquire_province_unit_order(&direct, province_order_context);
                assert_eq!(direct.0.get(), 4 + bits.count_ones() as usize);
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
    }

    #[test]
    fn shared_unit_sources_keep_independent_census_failures_and_release_on_reset() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; TABLE_COUNT]);
        assert_eq!(unit_sources(&captured.unit_inputs).first_outcome, MISSING);
        let first = Arc::new(UnitInputs::fixture(
            vec![carmy::DivisionSeed {
                pointer: 0x1234,
                ..Default::default()
            }],
            Outcome::Truncated {
                len: 1,
                unread_rows: 7,
            },
        ));
        let second = Arc::new(UnitInputs::fixture(
            Vec::new(),
            Outcome::Unavailable {
                reason: "second census failed",
            },
        ));
        captured.install_unit_inputs(Some(Arc::clone(&first)), Some(Arc::clone(&second)), false);
        let sources = unit_sources(&captured.unit_inputs);
        assert_eq!(sources.first.len(), 1);
        assert_eq!(
            sources.first_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 7
            }
        );
        assert_eq!(
            sources.second_outcome,
            Outcome::Unavailable {
                reason: "second census failed"
            }
        );
        captured.reset();
        assert_eq!(Arc::strong_count(&first), 1);
        assert_eq!(Arc::strong_count(&second), 1);
        assert_eq!(unit_sources(&captured.unit_inputs).first_outcome, MISSING);
    }

    #[test]
    fn metadata_partitions_every_extended_table_once_and_preserves_cost_membership() {
        assert_eq!(N, 168);
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
                        TableId::OrdersGroupColors,
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
            } else if anchor == TableId::Provinces {
                assert_eq!(
                    members,
                    vec![TableId::Provinces, TableId::ProvinceVisibility]
                );
            } else {
                assert!(members.iter().all(|table| actual.contains(table)));
            }
        }
        let groups = read_groups();
        for table in DIVISION_SOURCE_TABLES
            .into_iter()
            .chain([TableId::ProvinceVisibility])
        {
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
    fn only_genuine_map_dependencies_retain_sources_and_registry_has_its_own_window() {
        let mut successful = [CONSISTENCY_TARGET; TABLE_COUNT];
        successful[TableId::VariantStats.index()] -= 1;
        let needed = successful.map(|count| count < CONSISTENCY_TARGET);
        assert!(!Captured::needs_division_check(&needed));
        assert_eq!(Captured::source_checks(&needed), SourceChecks::default());
        assert_eq!(variant_successful(&successful), CONSISTENCY_TARGET - 1);
        let mut registry = Registry::new(2, variant_successful(&successful));
        assert_eq!(registry.rows.second.len(), 2);
        assert_eq!(
            registry.allocated_bytes(),
            4 * size_of::<cequipment_variant::RegistryRow>()
        );
        let mut capacity = 2;
        registry.grow(&mut capacity, CONSISTENCY_TARGET);
        assert!(registry.rows.second.is_empty());
        assert_eq!(
            registry.allocated_bytes(),
            2 * size_of::<cequipment_variant::RegistryRow>()
        );
        successful[TableId::AirTheatreMembers.index()] = 0;
        assert!(
            Captured::source_checks(&successful.map(|count| count < CONSISTENCY_TARGET)).air_wings
        );
        successful[TableId::DivisionEquipment.index()] = 0;
        assert!(Captured::needs_division_check(
            &successful.map(|count| count < CONSISTENCY_TARGET)
        ));
    }
    #[test]
    fn division_source_comparison_waits_for_every_equipment_and_upgrade_table() {
        for pending in DIVISION_SOURCE_TABLES {
            let mut successful = [CONSISTENCY_TARGET; TABLE_COUNT];
            successful[pending.index()] -= 1;
            let needed = successful.map(|count| count < CONSISTENCY_TARGET);
            assert!(Captured::needs_division_check(&needed), "{pending:?}");
            assert_eq!(division_successful(&successful), CONSISTENCY_TARGET - 1);
            let mut source = Divisions::new(2, division_successful(&successful));
            let mut capacity = 2;
            source.grow(&mut capacity, division_successful(&successful));
            assert_eq!(source.second.len(), 2, "{pending:?}");

            successful[pending.index()] += 1;
            assert!(!Captured::needs_division_check(
                &successful.map(|count| count < CONSISTENCY_TARGET)
            ));
            source.grow(&mut capacity, division_successful(&successful));
            assert!(source.second.is_empty(), "{pending:?}");
            assert_eq!(source.first.len(), 2);
        }
    }
    #[test]
    fn division_sources_preserve_census_identity_and_reject_an_incomplete_parent() {
        let mut sources = Divisions::new(1, 0);
        let source = DivisionSource {
            pointer: 0x1234,
            division: (1_u64 << 48) | 14,
            country: 17,
            template: u64::MAX,
        };
        sources.visit_source(source, false);
        sources.finish(Outcome::Rows { len: 1 }, false);
        assert_eq!(sources.outcomes[0], Outcome::Rows { len: 1 });
        assert_eq!(
            sources.first[0],
            DivisionSource {
                pointer: 0x1234,
                division: (1_u64 << 48) | 14,
                template: u64::MAX,
                country: 17
            }
        );
        sources.visit_source(
            DivisionSource {
                pointer: 0x5678,
                ..source
            },
            false,
        );
        sources.finish(
            Outcome::Truncated {
                len: 2,
                unread_rows: 7,
            },
            false,
        );
        assert_eq!(
            sources.outcomes[0],
            Outcome::Truncated {
                len: 1,
                unread_rows: 8
            }
        );
        assert_eq!(sources.required, 9);
        sources.reset();
        sources.finish(Outcome::Rows { len: 1 }, false);
        assert!(matches!(sources.outcomes[0], Outcome::Unavailable { .. }));
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
    fn focus_pilot_handoff_reaches_the_governance_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.focus_pilot().focus_raw_mode = true;
        assert!(captured.governance.focus_raw_mode);
        captured.focus_pilot().focus_raw_mode = false;
        assert!(!captured.governance.focus_raw_mode);
    }

    #[test]
    fn relations_pilot_handoff_reaches_the_intel_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.relations_pilot().relations_raw_mode = true;
        assert!(captured.intel.relations_raw_mode);
        captured.relations_pilot().relations_raw_mode = false;
        assert!(!captured.intel.relations_raw_mode);
    }

    #[test]
    fn characters_pilot_handoff_reaches_the_characters_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.characters_pilot().raw_mode = true;
        assert!(captured.characters.raw_mode);
        captured.characters_pilot().raw_mode = false;
        assert!(!captured.characters.raw_mode);
    }

    #[test]
    fn projects_pilot_handoff_reaches_the_projects_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.projects_pilot().raw_mode = true;
        assert!(captured.projects.raw_mode);
        captured.projects_pilot().raw_mode = false;
        assert!(!captured.projects.raw_mode);
    }

    #[test]
    fn lexical_pilot_handoff_reaches_the_lexical_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.lexical_pilot().raw_mode = true;
        assert!(captured.lexical.raw_mode);
        captured.lexical_pilot().raw_mode = false;
        assert!(!captured.lexical.raw_mode);
    }

    #[test]
    fn production_rest_pilot_handoff_reaches_the_production_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.production_rest_pilot().rest_raw_mode = true;
        assert!(captured.production.rest_raw_mode);
        captured.production_rest_pilot().rest_raw_mode = false;
        assert!(!captured.production.rest_raw_mode);
    }

    #[test]
    fn construction_rest_pilot_handoff_reaches_the_construction_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.construction_rest_pilot().rest_raw_mode = true;
        assert!(captured.construction.rest_raw_mode);
        captured.construction_rest_pilot().rest_raw_mode = false;
        assert!(!captured.construction.rest_raw_mode);
    }

    #[test]
    fn economy_rest_pilot_handoff_reaches_the_economy_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
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
        let mut actual = Captured::with_capacities(&capacity, &[0; TABLE_COUNT]);
        let mut expected = Captured::with_capacities(&capacity, &[0; TABLE_COUNT]);
        let row = cair_wing::Row {
            id: 0x1234_5678_0000_0009,
            ..Default::default()
        };
        // Each dependency alone and no checks cover independent second-pass
        // requests in all four raw-admission combinations, with faults and
        // successful nonempty wing rows from the existing typed fixture.
        for readable in [false, true] {
            for pending in [
                None,
                Some(TableId::WingEquipment),
                Some(TableId::WingCumulative),
                Some(TableId::AirTheatreMembers),
                Some(TableId::LandTheatreMembers),
                Some(TableId::OrdersGroups),
                Some(TableId::OrdersGroupMembers),
                Some(TableId::ArmyGroupMembers),
            ] {
                for air_raw in [false, true] {
                    for naval_raw in [false, true] {
                        actual.reset();
                        expected.reset();
                        let mut needed = [false; TABLE_COUNT];
                        if let Some(table) = pending {
                            needed[table.index()] = true;
                        }
                        let static_checks = Captured::source_checks(&needed);
                        let air_needed = local(&air::TABLES, &needed);
                        let naval_needed = local(&naval_history::TABLES, &needed);
                        let wing_second = naval_history::needs_wing_sources(&naval_needed);
                        let order_second = naval_history::needs_order_sources(&naval_needed);
                        let memory = SourceReads {
                            bytes: readable.then(air::wing_test_kit::fixture),
                            ..Default::default()
                        };
                        let reference_memory = SourceReads {
                            bytes: readable.then(air::wing_test_kit::fixture),
                            ..Default::default()
                        };
                        {
                            let mut sources =
                                actual.sources(&memory, 0, &needed, air_raw, naval_raw);
                            assert_eq!(
                                sources.checks(),
                                SourceChecks {
                                    land_combats: needed[TableId::LandCombatProgress.index()],
                                    ships: false,
                                    air_wings: (!air_raw && (air_needed[0] || air_needed[2]))
                                        || (!naval_raw && wing_second),
                                    order_groups: !naval_raw && order_second,
                                }
                            );
                            for second in [false, true] {
                                sources.begin_air(second);
                                sources.air(second, air::wing_test_kit::WING, 0, &row);
                                sources.finish_air(second, Outcome::Rows { len: 1 });
                                sources.order_groups(
                                    second,
                                    &[],
                                    Outcome::Unavailable {
                                        reason: "owned source fault",
                                    },
                                );

                                // Independent direct consumers, not the adapter's
                                // routing helpers, define the old live fallback.
                                if !air_raw {
                                    if !second {
                                        expected.air.begin_wings();
                                    }
                                    expected.air.visit_wing(
                                        &reference_memory,
                                        0,
                                        air::WingSource {
                                            pointer: air::wing_test_kit::WING,
                                            row: &row,
                                            second,
                                        },
                                        &air_needed,
                                    );
                                    expected.air_roots[usize::from(second)] =
                                        Some(Outcome::Rows { len: 1 });
                                }
                                if !naval_raw && (!second || wing_second) {
                                    expected.naval_history.visit_wing(
                                        &reference_memory,
                                        0,
                                        air::wing_test_kit::WING,
                                        &row,
                                        second,
                                    );
                                    expected
                                        .naval_history
                                        .finish_wings(Outcome::Rows { len: 1 }, second);
                                }
                                if !naval_raw && (!second || order_second) {
                                    expected.naval_history.set_order_sources(
                                        &reference_memory,
                                        0,
                                        &[],
                                        Outcome::Unavailable {
                                            reason: "owned source fault",
                                        },
                                        second,
                                    );
                                }
                            }
                        }
                        assert_eq!(*memory.reads.borrow(), *reference_memory.reads.borrow());
                        assert_eq!(actual.air_roots, expected.air_roots);
                        assert_eq!(actual.air.tables(), expected.air.tables());
                        assert_eq!(
                            actual.naval_history.tables(),
                            expected.naval_history.tables()
                        );
                        assert_eq!(Captured::source_checks(&needed), static_checks);
                        if readable && !air_raw {
                            assert_eq!(actual.air.outcomes()[0], Outcome::Rows { len: 2 });
                            assert_eq!(actual.air.outcomes()[2], Outcome::Rows { len: 1 });
                        }
                        if air_raw && naval_raw {
                            assert!(memory.reads.borrow().is_empty());
                            assert_eq!(actual.air_roots, [None; 2]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn dynamic_source_checks_never_release_static_fallback_comparison_storage() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; TABLE_COUNT]);
        let memory = SourceReads::default();
        for index in 0..TABLE_COUNT {
            let mut needed = [false; TABLE_COUNT];
            needed[index] = true;
            let static_checks = Captured::source_checks(&needed);
            assert_eq!(
                captured.sources(&memory, 0, &needed, false, false).checks(),
                static_checks
            );
            assert_eq!(
                captured.sources(&memory, 0, &needed, true, true).checks(),
                SourceChecks {
                    land_combats: needed[TableId::LandCombatProgress.index()],
                    ships: needed[TableId::ShipModels.index()],
                    order_groups: plans::TABLES.iter().any(|t| needed[t.index()]),
                    ..SourceChecks::default()
                }
            );
            assert_eq!(Captured::source_checks(&needed), static_checks);
        }
        let needed = [true; TABLE_COUNT];
        for air_raw in [false, true] {
            for naval_raw in [false, true] {
                let checks = captured
                    .sources(&memory, 0, &needed, air_raw, naval_raw)
                    .checks();
                assert_eq!(checks.air_wings, !air_raw || !naval_raw);
                assert!(checks.order_groups);
            }
        }
    }
}
