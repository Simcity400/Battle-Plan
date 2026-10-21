//! Coordinated extended acquisition. Public arrays use global TableId indices;
//! individual packs keep compact local arrays. Allocation is off-thread.

#[cfg(test)]
use collector_output::output_continuity;

use crate::{
    division_census::UnitInputs,
    map_capture::{MapSources, Orders, ShipCensus},
    sampling::qpc,
    table_capture::{grow_capacity, grow_vec},
};
pub(crate) use collector_extended_support::outcomes::RaidReadCounts;
use collector_extended_support::outcomes::{MISSING, elapsed, failed_capture, truncated_source};
use engine_allocation::{copy_vec, filled, owned_vec};
use engine_native::{Memory, cair_wing, cequipment_variant_pool::DivisionSource, walk::Outcome};
use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
use replay_frame::frame::TableInput;
use std::mem::size_of;
use std::sync::Arc;

pub(crate) use collector_division_parts::division_land;
#[cfg(test)]
pub(crate) use collector_extended_support::bench_world;
pub(crate) use collector_intel::daily_intel;

pub(crate) use collector_division_parts::attack_stats;
mod border_inputs;
use collector_division_parts::dig_in;
pub(crate) mod observer_boundary;
mod packs;
use collector_division_parts::supply_inputs;
pub(crate) use collector_division_parts::unit_combat_references;
use packs::ORDINARY_TABLES;
pub(crate) use packs::{
    CharacterInputRoles, CharacterInputsRequest, FinalInputsReport, FinalInputsRequest, GatedPacks,
    LatePacks, NavalMissionLoan, ORDINARY_PARTS, OrdinaryPacks, OrdinaryPart, ProductionDailyPacks,
    RaidLaunchPacks, RaidStatePacks, RaidStateStores, RaidTargetPacks, ScoresRailsParts, SlowPacks,
    SlowStores, StabilityPacks, WarOverviewPacks,
};
pub(crate) use production::registry::Registry;

const DIVISION_SOURCE_TABLES: [TableId; 11] = [
    TableId::DivisionLandInputs,
    TableId::DivisionCensus,
    TableId::DivisionEquipment,
    TableId::ArmyUpgradeGroups,
    TableId::ArmyUpgradeRequests,
    TableId::ArmyUpgradeDeliveries,
    TableId::ArmyUpgradeProduced,
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::DivisionPanelStats,
    TableId::ArmyIconInputs,
];
const DIVISION_GROUP_MEMBERS: [TableId; DIVISION_SOURCE_TABLES.len() + 1] = {
    let mut members = [TableId::Divisions; DIVISION_SOURCE_TABLES.len() + 1];
    let mut index = 0;
    while index < DIVISION_SOURCE_TABLES.len() {
        members[index + 1] = DIVISION_SOURCE_TABLES[index];
        index += 1;
    }
    members
};
/// Outputs supplied after pack conversion by the existing faction census.
const FACTION_PROVIDER_TABLES: [TableId; 14] = {
    let mut tables = [TableId::LexicalTokens; 14];
    let mut next = 0;
    let mut index = 0;
    while index < crate::faction_companions::TABLES.len() {
        tables[next] = crate::faction_companions::TABLES[index];
        next += 1;
        index += 1;
    }
    index = 0;
    while index < crate::faction_goals::TABLES.len() {
        tables[next] = crate::faction_goals::TABLES[index];
        next += 1;
        index += 1;
    }
    assert!(next == tables.len());
    tables
};
pub type ReadGroup = (&'static str, TableId, &'static [TableId]);
#[derive(Clone, Copy)]
struct ReadGroupData {
    name: &'static str,
    anchor: TableId,
    members: [TableId; TABLE_COUNT],
    len: usize,
}

// Native conversion reuse stays within its pack's existing timing scope.
macro_rules! spool_inputs {
    (combat_progress, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (combat_log, $captured:expr, $conversion:expr) => {
        $captured.table_inputs()
    };
    (group_counters, $captured:expr, $conversion:expr, $owner:expr, $hour:expr) => {
        $captured.tables().map(TableInput::Sampled)
    };
    (country_appearance, $captured:expr, $conversion:expr, $owner:expr, $hour:expr) => {
        $captured.tables_at(Some($hour)).map(TableInput::Sampled)
    };

    (division_templates, $captured:expr, $conversion:expr) => {
        $captured.projected_inputs($conversion.projection.as_deref_mut())
    };
    (division_presentation, $captured:expr, $conversion:expr, $all:expr, $hour:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut(), Some($hour))
    };
    ($pack:ident, $captured:expr, $conversion:expr, $all:expr, $hour:expr) => {
        spool_inputs!($pack, $captured, $conversion)
    };
    (lexical, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.lexical.as_deref_mut())
    };
    (characters, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.characters.as_deref_mut())
    };
    (character_portraits, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (hired_advisors, $captured:expr, $conversion:expr) => {
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
    (country_tactic_unlocks, $captured:expr, $conversion:expr) => {
        $captured.table_inputs()
    };
    (production, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (economy, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (air, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (character_inputs, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (weather, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (fog, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (supply_trains, $captured:expr, $conversion:expr) => {
        $captured.table_inputs()
    };
    (market, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (war_overview, $captured:expr, $conversion:expr) => {
        $captured.table_inputs()
    };
    (air_records, $captured:expr, $conversion:expr) => {
        $captured.table_inputs()
    };
    (naval_bases, $captured:expr, $conversion:expr) => {
        $captured.table_inputs()
    };
    ($pack:ident, $captured:expr, $conversion:expr) => {
        $captured.tables().map(TableInput::Sampled)
    };
}

/// One pack's conversion on the spool thread. A pack with any helper marker
/// (`@helper`, `@helper_b`, `@helper_appearance`, `@helper_exclusive`,
/// `@helper_cached_a`, `@helper_cached_b`) is skipped there while the
/// conversion helpers convert it.
macro_rules! spool_pack {
    ($all:expr, $pack:ident; $skip:expr, $destination:ident, $conversion:ident, $hour:ident, $slots:ident, $costs:ident) => {
        spool_pack!(@convert $all, $pack; $destination, $conversion, $hour, $slots, $costs)
    };
    ($all:expr, $pack:ident, $marker:ident; $skip:expr, $destination:ident, $conversion:ident, $hour:ident, $slots:ident, $costs:ident) => {
        if $skip {
            $slots.next();
        } else {
            spool_pack!(@convert $all, $pack; $destination, $conversion, $hour, $slots, $costs)
        }
    };
    (@convert $all:expr, $pack:ident; $destination:ident, $conversion:ident, $hour:ident, $slots:ident, $costs:ident) => {{
        let started = qpc();
        let cycles = crate::spool_phases::thread_cycles();
        if crate::allocation::failed() { return $costs; }
        for (id, view) in $pack::TABLES.into_iter().zip(spool_inputs!($pack, $all.$pack, $conversion, $all, $hour)) { $destination[id.index()] = view; }
        let (pack, slot) = $slots.next().unwrap();
        *slot = qpc() - started;
        crate::spool_phases::SPOOL.record_pack(pack, crate::spool_phases::thread_cycles().saturating_sub(cycles));
    }};
}

/// One pack's conversion on a helper lane: lane 0 holds `@helper`,
/// `@helper_exclusive` and `@helper_cached_a`, lane 1 `@helper_b`,
/// `@helper_appearance` and `@helper_cached_b`; a helper converts the lanes set
/// in `$lanes`. `@helper_cached_*` packs (characters, intel, character
/// portraits) need not be `Sync`: characters and intel convert with the spool
/// conversion caches the spool thread lends their lane for the join
/// ([`HelperConversion`]), and like the exclusive pack no other pack reads
/// them, so their lane has sole use while it converts.
/// `@helper` and `@helper_b` packs are statically `Sync` through
/// [`HelperView::sync_pack`] and see only themselves. `@helper_appearance`
/// (country appearance) also reads the air pack through `sync_pack`.
/// `@helper_exclusive` packs need not be `Sync`: while lane 0 converts, it is
/// their only user (see [`HelperView`]). They convert through the owner arm,
/// which supplies the frame's hour.
macro_rules! helper_pack {
    ($view:expr, $pack:ident; $conversion:ident, $out:ident, $slots:ident, $hour:ident, $lanes:ident) => {
        $slots.next();
    };
    ($view:expr, $pack:ident, helper; $conversion:ident, $out:ident, $slots:ident, $hour:ident, $lanes:ident) => {
        helper_pack!(@lane 0, $lanes, $out, $slots, $pack, spool_inputs!($pack, HelperView::sync_pack(&$view.0.$pack), $conversion))
    };
    ($view:expr, $pack:ident, helper_b; $conversion:ident, $out:ident, $slots:ident, $hour:ident, $lanes:ident) => {
        helper_pack!(@lane 1, $lanes, $out, $slots, $pack, spool_inputs!($pack, HelperView::sync_pack(&$view.0.$pack), $conversion))
    };
    ($view:expr, $pack:ident, helper_appearance; $conversion:ident, $out:ident, $slots:ident, $hour:ident, $lanes:ident) => {
        helper_pack!(@lane 1, $lanes, $out, $slots, $pack, HelperView::sync_pack(&$view.0.$pack)
            .tables_at(Some($hour))
            .map(TableInput::Sampled))
    };
    ($view:expr, $pack:ident, helper_exclusive; $conversion:ident, $out:ident, $slots:ident, $hour:ident, $lanes:ident) => {
        helper_pack!(@lane 0, $lanes, $out, $slots, $pack, spool_inputs!($pack, &$view.0.$pack, $conversion, $view.0, $hour))
    };
    ($view:expr, $pack:ident, helper_cached_a; $conversion:ident, $out:ident, $slots:ident, $hour:ident, $lanes:ident) => {
        helper_pack!(@lane 0, $lanes, $out, $slots, $pack, spool_inputs!($pack, &$view.0.$pack, $conversion))
    };
    ($view:expr, $pack:ident, helper_cached_b; $conversion:ident, $out:ident, $slots:ident, $hour:ident, $lanes:ident) => {
        helper_pack!(@lane 1, $lanes, $out, $slots, $pack, spool_inputs!($pack, &$view.0.$pack, $conversion))
    };
    (@lane $lane:literal, $lanes:ident, $out:ident, $slots:ident, $pack:ident, $inputs:expr) => {
        if $lanes & (1 << $lane) == 0 {
            $slots.next();
        } else {
            helper_pack!(@convert $out, $slots, $pack, $inputs)
        }
    };
    (@convert $out:ident, $slots:ident, $pack:ident, $inputs:expr) => {{
        let started = qpc();
        let cycles = crate::spool_phases::thread_cycles();
        let (pack, slot) = $slots.next().unwrap();
        if !crate::allocation::failed() {
            for (id, view) in $pack::TABLES.into_iter().zip($inputs) {
                $out[id.index()] = Some(view);
            }
            *slot = qpc() - started;
            crate::spool_phases::SPOOL.record_pack(
                pack,
                crate::spool_phases::thread_cycles().saturating_sub(cycles),
            );
        }
    }};
}

/// The helper lane a pack marker names.
#[cfg(test)]
macro_rules! helper_lane {
    () => {
        None
    };
    (helper) => {
        Some(0)
    };
    (helper_exclusive) => {
        Some(0)
    };
    (helper_cached_a) => {
        Some(0)
    };
    (helper_b) => {
        Some(1)
    };
    (helper_appearance) => {
        Some(1)
    };
    (helper_cached_b) => {
        Some(1)
    };
}

/// What a helper lane converts with: the projection, and on the lane of the
/// characters or intel pack the spool caches it needs, lent by the spool
/// thread for the join. Packs needing any other spool conversion cache cannot
/// compile as helper packs.
pub(crate) struct HelperConversion<'a, 'cache> {
    pub(crate) projection: Option<&'a mut crate::projection::Projection<'cache>>,
    pub(crate) characters: Option<&'a mut characters::CharacterConversion<'cache>>,
    pub(crate) intel: Option<&'a mut intel::IntelConversion<'cache>>,
    pub(crate) variables_baseline: Option<(
        replay_frame::frame::BaselineToken,
        &'a [replay_data::rows::ScriptVariablesRow],
    )>,
    pub(crate) variables_scratch: &'a mut intel::ScriptVariableComparison,
}

/// The spool conversion caches the characters and intel packs convert with,
/// lent to a helper lane for one join.
#[derive(Default)]
pub(crate) struct LentCaches<'a, 'cache> {
    pub(crate) characters: Option<&'a mut characters::CharacterConversion<'cache>>,
    pub(crate) intel: Option<&'a mut intel::IntelConversion<'cache>>,
    pub(crate) variables_baseline: Option<(
        replay_frame::frame::BaselineToken,
        &'a [replay_data::rows::ScriptVariablesRow],
    )>,
    pub(crate) variables_scratch: Option<&'a mut intel::ScriptVariableComparison>,
}

macro_rules! packs {
    ($($pack:ident $(@ $helper:ident)?),+ $(,)?) => {
        pub const PACK_NAMES: &[&str] = &[$(stringify!($pack)),+];
        /// Each pack's helper lane: `None` for the spool thread's packs.
        #[cfg(test)]
        pub(crate) const HELPER_LANES: [Option<u8>; PACK_NAMES.len()] =
            [$(helper_lane!($($helper)?)),+];
        pub const PACK_N: usize = 0 $(+ $pack::N)+;
        pub const N: usize = PACK_N + FACTION_PROVIDER_TABLES.len() + collector_naval_management::forces::TABLES.len();
        pub const TABLES: [TableId; N] = {
            let mut tables = [TableId::LexicalTokens; N];
            let mut next = 0;
            $(let mut index = 0;
            while index < $pack::N {
                tables[next] = $pack::TABLES[index]; next += 1; index += 1;
            })+
            let mut index = 0;
            while index < FACTION_PROVIDER_TABLES.len() {
                tables[next] = FACTION_PROVIDER_TABLES[index]; next += 1; index += 1;
            }
            let mut index = 0;
            while index < collector_naval_management::forces::TABLES.len() {
                tables[next] = collector_naval_management::forces::TABLES[index];
                next += 1; index += 1;
            }
            assert!(next == N);
            tables
        };
        /// Only pack-owned tables use the pack acquisition tick array.
        pub const PACK_TABLES: [TableId; PACK_N] = {
            let mut tables = [TableId::LexicalTokens; PACK_N];
            let mut index = 0;
            while index < PACK_N { tables[index] = TABLES[index]; index += 1; }
            tables
        };
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct Capacities {
            $(pub $pack: $pack::Capacities,)+
            pub fuel_awards: fuel_awards::Capacities,
            pub division_sources: usize,
            pub variant_registry: usize,
        }
        impl Default for Capacities {
            fn default() -> Self {
                Self { $($pack: Default::default(),)+ fuel_awards: Default::default(), division_sources: 0, variant_registry: 0 }
            }
        }
        pub struct Captured {
            $($pack: $pack::Captured,)+
            fuel_awards: fuel_awards::Captured,
            divisions: Divisions,
            final_inputs_ticks: Option<FinalInputsReport>,
            unit_inputs: Option<Arc<UnitInputs>>,
            governance_report: Option<governance::CaptureReport>,
            variants_ticks: Option<[u64; production::N]>,
            presentation_report: Option<division_presentation::CaptureReport>,
            intel_residual_report: Option<intel::CaptureReport>,
            supply_trains_report: Option<supply_trains::CaptureReport>,
            character_portraits_report: Option<character_portraits::CaptureReport>,
            province_unit_order_report: Option<province_unit_order::CaptureReport>,
            ship_models_report: Option<ship_models::CaptureReport>,
            ordinary_ticks: Option<[u64; TABLE_COUNT]>,
            ordinary_failure: Option<&'static str>,
            ordinary_spare: Option<Box<OrdinaryPacks>>,
            ordinary_part_spares: [Option<OrdinaryPart>; ORDINARY_PARTS],
            slow_ticks: Option<[u64; TABLE_COUNT]>,
            /// Slow's refusal this hour, which also refuses the daily
            /// production tables split from it ([`ProductionDailyPacks`]).
            slow_refusal: Option<&'static str>,
            slow_spare: Option<Box<SlowPacks>>,
            stability_ticks: Option<[u64; TABLE_COUNT]>,
            stability_spare: Option<Box<StabilityPacks>>,
            war_overview_ticks: Option<[u64; TABLE_COUNT]>,
            war_overview_spare: Option<Box<WarOverviewPacks>>,
            production_daily_ticks: Option<[u64; TABLE_COUNT]>,
            production_daily_spare: Option<Box<ProductionDailyPacks>>,
            template_ticks: Option<[u64; division_templates::N]>,
            raid_state_ticks: Option<[u64; TABLE_COUNT]>,
            raid_state_spare: Option<Box<RaidStatePacks>>,
            raid_launch_ticks: Option<[u64; TABLE_COUNT]>,
            raid_launch_spare: Option<Box<RaidLaunchPacks>>,
            raid_targets_ticks: Option<[u64; TABLE_COUNT]>,
            raid_targets_spare: Option<Box<RaidTargetPacks>>,
            late_ticks: Option<[u64; TABLE_COUNT]>,
            late_spare: Option<Box<LatePacks>>,
            scores_rails_spare: Option<Box<ScoresRailsParts>>,
            missions_spare: Option<Box<air::Captured>>,
            gated_ticks: Option<[u64; TABLE_COUNT]>,
            gated_spare: Option<Box<GatedPacks>>,
            character_inputs_ticks: Option<[u64; character_inputs::N]>,
            registry: Registry,
            air_root: Option<Outcome>,
            overhead: [u64; TABLE_COUNT],
        }
        /// The stores a Governance job returns: the technology and focus
/// fingerprint stores and the held building, party-leader and party stores.
pub(crate) type GovernanceStores = (
    Box<crate::slicing::SliceFingerprints>,
    Box<crate::slicing::SliceFingerprints>,
    Option<Box<construction::BuildingStore>>,
    Option<Box<governance::LeaderStore>>,
    Option<Box<governance::PartyStore>>,
);

impl Captured {
            #[cfg(test)]
            pub(crate) fn set_intel_shadow_fixture(&mut self, value: i64) {
                self.intel = intel::intel_shadow_fixture(value);
            }

            pub fn with_capacities(capacity: &Capacities) -> Self {
                let mut captured = Self {
                    $($pack: $pack::Captured::with_capacities(&capacity.$pack),)+
                    fuel_awards: fuel_awards::Captured::with_capacities(&capacity.fuel_awards),
                    divisions: Divisions::new(capacity.division_sources),
                    final_inputs_ticks: None,
                    unit_inputs: None,
                    governance_report: None,
                    variants_ticks: None,
                    presentation_report: None,
                    intel_residual_report: None,
                    supply_trains_report: None,
                    character_portraits_report: None,
                    province_unit_order_report: None,
                    ship_models_report: None,
                    ordinary_ticks: None,
                    ordinary_failure: None,
                    ordinary_spare: OrdinaryPacks::spare(),
                    ordinary_part_spares: OrdinaryPart::spares(),
                    slow_ticks: None,
                    slow_refusal: None,
                    slow_spare: SlowPacks::spare(),
                    stability_ticks: None,
                    stability_spare: StabilityPacks::spare(),
                    war_overview_ticks: None,
                    war_overview_spare: WarOverviewPacks::spare(),
                    production_daily_ticks: None,
                    production_daily_spare: ProductionDailyPacks::spare(),
                    template_ticks: None,
                    raid_state_ticks: None,
                    raid_state_spare: RaidStatePacks::spare(),
                    raid_launch_ticks: None,
                    raid_launch_spare: RaidLaunchPacks::spare(),
                    raid_targets_ticks: None,
                    raid_targets_spare: RaidTargetPacks::spare(),
                    late_ticks: None,
                    late_spare: LatePacks::spare(),
                    scores_rails_spare: ScoresRailsParts::spare(),
                    missions_spare: if crate::allocation::failed() { None } else { Some(Box::new(air::Captured::vacant())) },
                    gated_ticks: None,
                    gated_spare: GatedPacks::spare(),
                    character_inputs_ticks: None,
                    registry: Registry::new(capacity.variant_registry),
                    air_root: None, overhead: [0; TABLE_COUNT],
                };
                captured.division_presentation.reserve_sources(capacity.division_sources);
                captured.plans.reserve_units(capacity.division_sources);
                captured
            }
            /// Reset before the shared division and map walks, not between
            /// source visitation and capture().
            pub fn reset(&mut self) {
                $(self.$pack.reset();)+
                self.fuel_awards.reset();
                self.divisions.reset(); self.registry.reset(); self.final_inputs_ticks = None;
                self.release_unit_inputs(); self.governance_report = None; self.presentation_report = None;
                self.variants_ticks = None;
                self.intel_residual_report = None;
                self.supply_trains_report = None; self.character_portraits_report = None; self.province_unit_order_report = None; self.ship_models_report = None;
                self.ordinary_ticks = None;
                self.ordinary_failure = None;
                self.slow_ticks = None;
                self.slow_refusal = None;
                self.stability_ticks = None;
                self.war_overview_ticks = None;
                self.production_daily_ticks = None;
                self.template_ticks = None;
                self.raid_state_ticks = None;
                self.raid_launch_ticks = None;
                self.raid_targets_ticks = None;
                self.late_ticks = None;
                self.gated_ticks = None;
                self.character_inputs_ticks = None;
                self.air_root = None; self.overhead = [0; TABLE_COUNT];
            }
            pub fn allocated_bytes(&self) -> usize {
                0 $(+ self.$pack.allocated_bytes())+ + self.fuel_awards.allocated_bytes() + self.divisions.allocated_bytes() + self.registry.allocated_bytes() + self.spare_bytes()
            }
            /// Spool-thread metadata for this buffer's native row and scratch
            /// allocations. Excludes core tables, encoded frames and queues.
            pub fn allocated_by_pack(&self) -> std::io::Result<Vec<(&'static str, usize)>> {
                copy_vec(&[
                    $((stringify!($pack), self.$pack.allocated_bytes()),)+
                    ("fuel_awards", self.fuel_awards.allocated_bytes()),
                    ("division_sources", self.divisions.allocated_bytes()),
                    ("variant_registry", self.registry.allocated_bytes()),
                    ("acquisition_spares", self.spare_bytes()),
                ])
            }
            pub fn grow(&mut self, capacity: &mut Capacities) {
                engine_allocation::advisory(|| self.grow_hints(capacity));
            }
            fn grow_hints(&mut self, capacity: &mut Capacities) {
                let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
                $(self.$pack.grow(&mut capacity.$pack);)+
                self.fuel_awards.grow(&mut capacity.fuel_awards);
                self.divisions.grow(&mut capacity.division_sources);
                self.division_presentation.reserve_sources(capacity.division_sources);
                self.plans.reserve_units(capacity.division_sources);
                self.division_presentation.grow_name_groups(
                    &mut capacity.division_presentation,
                    self.division_templates.naming_demand(),
                );
                self.registry.grow(&mut capacity.variant_registry);
            }
            /// Spool-thread conversion only. Existing core slots survive. With
            /// `helper_converts`, packs with a helper marker are left to the
            /// helpers ([`Self::helper_view`]) and the caller applies the ordinary-job
            /// refusal ([`Self::refuse_failed_ordinary`]) after merging them.
            pub(crate) fn insert_tables(
                &self,
                hour: i32,
                destination: &mut [TableInput; TABLE_COUNT],
                conversion: &mut crate::spooling::SpoolConversion<'_, '_>,
                helper_converts: bool,
            ) -> [i64; PACK_NAMES.len()] {
                let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
                let mut costs = [0; PACK_NAMES.len()];
                let mut slots = costs.iter_mut().enumerate();
                $(spool_pack!(self, $pack $(, $helper)?; helper_converts, destination, conversion, hour, slots, costs);)+
                if !helper_converts {
                    self.refuse_failed_ordinary(destination);
                }
                costs
            }
            /// A failed ordinary job refuses every ordinary pack table, after
            /// all of them were converted.
            pub(crate) fn refuse_failed_ordinary(&self, destination: &mut [TableInput; TABLE_COUNT]) {
                if crate::allocation::failed() { return; }
                if let Some(reason) = self.ordinary_failure {
                    for &id in ORDINARY_TABLES.iter().flat_map(|tables| tables.iter()) {
                        destination[id.index()] = TableInput::Sampled(replay_frame::frame::TableView::Unavailable { reason: reason.into() });
                    }
                }
            }
            /// The packs with a helper marker, shareable with the conversion helpers.
            pub(crate) fn helper_view(&self) -> HelperView<'_> {
                HelperView(self)
            }
        }
        /// The conversion helpers' view of one hour's packs.
        pub(crate) struct HelperView<'a>(&'a Captured);
        // SAFETY: helpers convert through this view only while the spool
        // thread is inside `SpoolHelper::join`, which orders each helper's
        // accesses with the spool thread's through its mutex. `@helper`,
        // `@helper_b` and `@helper_appearance` packs, and what the latter
        // reads (the air pack), are reached through `sync_pack`, whose bound
        // proves them `Sync`, so the spool thread and both helpers may read
        // them at once. The `@helper_cached_*` packs (characters, intel,
        // character portraits) need not be `Sync`: one lane converts each, the
        // spool thread skips them while helpers convert, and no other pack
        // reads them. The `@helper_exclusive` pack (division presentation)
        // need not be `Sync`: it is on lane 0, which one helper converts, and
        // during the join no spool-thread code touches it. Its observers
        // (`observe_*`) run before conversion and outcomes, slice selections
        // and merging after the join. That helper therefore has exclusive use,
        // as if it held `&mut`. No other pack is reachable through the view.
        unsafe impl Sync for HelperView<'_> {}
        unsafe impl Send for HelperView<'_> {}
        impl<'a> HelperView<'a> {
            fn sync_pack<T: Sync>(pack: &'a T) -> &'a T {
                pack
            }
            /// Convert the helper packs of the lanes set in `lanes` (bit 0:
            /// lane 0, bit 1: lane 1) into `out`; their per-pack costs land in
            /// `costs` at their `PACK_NAMES` index. One thread at a time
            /// converts lane 0.
            pub(crate) fn tables<'lent, 'cache>(
                &self,
                hour: i32,
                lanes: u8,
                projection: Option<&'lent mut crate::projection::Projection<'cache>>,
                caches: LentCaches<'lent, 'cache>,
                out: &mut [Option<TableInput>; TABLE_COUNT],
                costs: &mut [i64; PACK_NAMES.len()],
            ) {
                let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
                let mut spare = intel::ScriptVariableComparison::default();
                let conversion = &mut HelperConversion {
                    projection,
                    characters: caches.characters,
                    intel: caches.intel,
                    variables_baseline: caches.variables_baseline,
                    variables_scratch: caches.variables_scratch.unwrap_or(&mut spare),
                };
                let mut slots = costs.iter_mut().enumerate();
                $(helper_pack!(self, $pack $(, $helper)?; conversion, out, slots, hour, lanes);)+
            }
        }
        impl Captured {
            pub fn outcomes(&self) -> [Outcome; TABLE_COUNT] {
                let mut output = [MISSING; TABLE_COUNT];
                $(scatter(&$pack::TABLES, self.$pack.outcomes(), &mut output);)+
                if let Some(reason) = self.ordinary_failure {
                    for &id in ORDINARY_TABLES.iter().flat_map(|tables| tables.iter()) {
                        output[id.index()] = Outcome::Unavailable { reason };
                    }
                }
                output
            }
        }
        const READ_GROUP_COUNT: usize = {
            let mut count = 9;
            $(let mut group = 0;
            while group < $pack::READ_GROUPS.len() {
                if $pack::TABLES[$pack::READ_GROUPS[group].1].index() != TableId::ArmyUpgradeGroups.index() { count += 1; }
                group += 1;
            })+
            count
        };
        static READ_GROUP_DATA: [ReadGroupData; READ_GROUP_COUNT] = {
            let empty = ReadGroupData { name: "", anchor: TableId::LexicalTokens,
                members: [TableId::LexicalTokens; TABLE_COUNT], len: 0 };
            let mut groups = [empty; READ_GROUP_COUNT];
            let mut next = 0;
            $(let mut group = 0;
            while group < $pack::READ_GROUPS.len() {
                let (name, anchor, members) = $pack::READ_GROUPS[group];
                // Count the shared division graph only in its combined source group.
                if $pack::TABLES[anchor].index() != TableId::ArmyUpgradeGroups.index() {
                    let mut data = empty;
                    data.name = name;
                    data.anchor = $pack::TABLES[anchor];
                    data.len = members.len();
                    let mut member = 0;
                    while member < members.len() {
                        data.members[member] = $pack::TABLES[members[member]];
                        member += 1;
                    }
                    groups[next] = data;
                    next += 1;
                }
                group += 1;
            })+
            let extra: [(&str, TableId, &[TableId]); 9] = [
                ("faction lists, member accounting and sharing", TableId::FactionRules, &crate::faction_companions::TABLES),
                ("faction goals and held progress", TableId::FactionGoalEntries, &crate::faction_goals::TABLES),
                ("divisions, coverage counts, equipment, upgrades, movement and counter status", TableId::Divisions,
                    &DIVISION_GROUP_MEMBERS),
                ("country census, appearance and supply trains", TableId::Countries,
                    &[TableId::Countries, TableId::CountryAppearance, TableId::SupplyNodes, TableId::TrainInventory]),
                ("navy country and fleet census with changed repair policy", TableId::Fleets,
                    &[TableId::Fleets, TableId::NavalRepairCountries, TableId::NavalRepairFleets]),
                ("task-force census and changed repair and composition policy", TableId::TaskForces,
                    &[TableId::TaskForces, TableId::NavalRepairTaskForces, TableId::NavalTaskForceComposition, TableId::NavalRepairChildren]),
                ("ship census, model operands and changed refit work", TableId::Ships,
                    &[TableId::Ships, TableId::ShipModels, TableId::NavalRefitShips, TableId::NavalRefitProgress]),
                ("states and hourly state flags", TableId::States, &[TableId::States, TableId::StateFlags]),
                ("army order census, plan operands, group plans, front sections and invasion preparations", TableId::ArmyOrders,
                    &[TableId::ArmyOrders, TableId::ArmyOrderPlans, TableId::OrdersGroupPlans, TableId::Fronts, TableId::InvasionPreparations]),
            ];
            let mut group = 0;
            while group < extra.len() {
                let (name, anchor, members) = extra[group];
                let mut data = empty;
                data.name = name;
                data.anchor = anchor;
                data.len = members.len();
                let mut member = 0;
                while member < members.len() { data.members[member] = members[member]; member += 1; }
                groups[next] = data;
                next += 1;
                group += 1;
            }
            assert!(next == READ_GROUP_COUNT);
            groups
        };
        /// Borrow the metadata graph; status never allocates its fixed catalogue.
        pub fn read_groups() -> impl ExactSizeIterator<Item = ReadGroup> + Clone {
            READ_GROUP_DATA.iter().map(|group| (group.name, group.anchor, &group.members[..group.len]))
        }
    };
}
pub(crate) use collector_air_sorties::air_reference_witness;
pub(crate) use collector_air_sorties::air_sorties;

pub mod active_abilities;
pub mod air;
pub use collector_air_histories::air_histories;
pub use collector_air_records::air_records;
pub use collector_air_relations::air_relations;
pub use collector_army_icons::army_icon_inputs;
pub use collector_ledger_tables::air_losses;
pub use collector_ledger_tables::building_bombing;
pub mod character_inputs;
pub mod character_portraits;
pub mod characters;
pub use collector_combat::combat_details;
pub use collector_combat_log::combat_log;
pub mod combat_progress;
pub use collector_construction::construction;
pub use collector_country_counters::country_appearance;
pub use collector_country_counters::country_counters;
pub(crate) use collector_country_counters::reserved_visibility;
pub(crate) use collector_naval_movement::country_naval_movement_inputs;
pub use collector_output::construction_tables as construction_ic;
pub use collector_slow_tables::country_factories;
pub use collector_slow_tables::country_tactic_unlocks;
pub use collector_stability::country_stability;
pub(crate) use country_naval_movement_inputs::{
    Capacities as PhysicalNavyCapacities, Work as PhysicalNavyWork,
};
pub mod division_counts;
pub mod division_presentation;
pub mod division_templates;
pub use collector_economy::economy;
pub mod equipment_destruction;
pub use collector_experience_gains::tables as experience_gains;
pub use collector_ledger_tables::event_feed;
pub use collector_slow_tables::faction_theatres;
pub mod fog;
pub(crate) use collector_fuel_awards::tables as fuel_awards;
pub mod game_score;
pub use collector_governance::governance;
pub mod group_counters;
pub use collector_advisors::hired_advisors;
pub use collector_intel::intel;
pub use collector_production_daily::industrial_organisations;
pub mod land_events;
pub use collector_leaders::leader_paths;
pub use collector_leaders::leader_portraits;
pub use collector_lexical::lexical;
pub use collector_map_tables::battle_sounds;
pub use collector_map_tables::map_facilities;
pub use collector_map_tables::naval_bases;
pub use collector_model_inputs::model_inputs;
pub use collector_naval_counters::naval_counters;
pub use collector_naval_details::naval_combat_sources;
pub use collector_naval_details::naval_details;
pub use collector_output::line_tables as line_output;
pub use collector_session::map_names;
pub use collector_slow_tables::logistics;
pub use collector_slow_tables::modifier_definitions;
pub(crate) mod naval_escort_trees;
pub mod naval_history;
pub mod naval_mission_inputs;
pub use collector_naval_details::naval_result_details;
pub mod naval_result_sources;
pub use collector_ledger_tables::market;
pub use collector_ledger_tables::nuclear_strikes;
pub use collector_new_divisions::new_divisions;
pub mod map_events;
pub mod operation_outcomes;
pub use collector_peace::peace_bids;
pub use collector_peace::tables as peace_settlements;
pub mod plans;
pub use collector_politics::politics;
pub use collector_production::production;
pub use collector_production_daily::production_modifiers;
pub use collector_projects::projects;
pub mod province_unit_order;
pub use collector_map_tables::saved_event_targets;
pub(crate) use collector_projects::raid_intel_carry;
pub use collector_projects::raid_target_statuses;
pub use collector_raids::raid_launch_inputs;
pub use collector_raids::raid_state_inputs;
pub use collector_raids::resource_map;
pub use collector_railway_guns::railway_guns;
pub use collector_session::session;
pub use collector_ship_models::ship_models;
pub mod slice_reads;
pub mod state_flags;
pub use collector_supply_trains::supply_trains;
pub use collector_visibility::visibility;
pub use collector_war_overview::war_overview;
pub use collector_weather::weather;

packs!(
    air @helper,
    air_relations,
    air_histories,
    army_icon_inputs @helper,
    naval_counters @helper,
    railway_guns,
    active_abilities,
    country_counters,
    game_score,
    group_counters,
    saved_event_targets,
    battle_sounds,
    equipment_destruction,
    naval_result_sources,
    experience_gains,
    event_feed,
    air_losses,
    air_records,
    building_bombing,
    nuclear_strikes,
    market,
    leader_paths,
    country_appearance @helper_appearance,
    ship_models,
    province_unit_order @helper,
    naval_bases @helper,
    map_facilities,
    naval_combat_sources @helper,
    character_portraits @helper_cached_b,
    leader_portraits,
    supply_trains @helper,
    characters @helper_cached_b,
    combat_progress @helper,
    combat_details @helper,
    naval_details @helper,
    naval_result_details @helper,
    construction @helper,
    division_presentation @helper_exclusive,
    division_templates,
    economy @helper,
    fog @helper,
    governance @helper_b,
    intel @helper_cached_a,
    operation_outcomes,
    map_events,
    lexical,
    map_names,
    naval_history @helper,
    production @helper,
    projects,
    plans,
    session,
    state_flags,
    visibility @helper,
    raid_target_statuses @helper,
    raid_state_inputs,
    resource_map,
    raid_launch_inputs,
    slice_reads,
    faction_theatres,
    model_inputs,
    modifier_definitions,
    hired_advisors @helper_b,
    character_inputs @helper_b,
    logistics,
    combat_log,
    division_counts,
    country_factories,
    war_overview,
    industrial_organisations @helper,
    production_modifiers @helper,
    line_output,
    politics,
    peace_settlements,
    peace_bids,
    land_events,
    construction_ic,
    country_stability,
    new_divisions,
    country_tactic_unlocks @helper,
    weather @helper,
    naval_mission_inputs @helper
);

impl Capacities {
    pub(crate) fn raise_reserved_visibility(&mut self, slots: usize) {
        self.country_appearance.raise_reverse_visibility(slots);
    }
    /// The States census and its numeric supplement share the same source
    /// visitor, so its pointer arena must hold the final core States capacity.
    pub(crate) fn raise_for_states(&mut self, states: usize) {
        self.state_flags.raise_for_states(states);
    }

    /// Rail manager validation bounds both rail supplements by the same
    /// positive physical province slots; every province definition bounds the
    /// division movement geometry.
    pub(crate) fn raise_for_provinces(&mut self, provinces: usize) {
        self.naval_history.rows[2] = self.naval_history.rows[2].max(provinces.saturating_sub(1));
        self.division_presentation.raise_for_provinces(provinces);
    }

    /// Independent off-thread hints must retry after load even if the registry
    /// did not grow. One failed family cannot withhold another family's demand.
    pub(crate) fn presize_live_inputs(
        &mut self,
        memory: &impl Memory,
        base: usize,
    ) -> Result<(), &'static str> {
        self.presize_live_inputs_periodic(memory, base, true)
    }

    pub(crate) fn presize_live_inputs_periodic(
        &mut self,
        memory: &impl Memory,
        base: usize,
        capacity_census: bool,
    ) -> Result<(), &'static str> {
        let mut failure = None;
        for result in [
            capacity_census.then(|| {
                crate::country_scale::observe_advisor_modifiers(memory, base).map(|demand| {
                    if let Some(demand) = demand
                        && characters::advisors::note_modifier_demand(demand)
                    {
                        crate::decode::presize_advisor_pools();
                    }
                })
            }),
            capacity_census.then(|| {
                crate::country_scale::observe_production_modifiers(memory, base).map(|demand| {
                    if let Some(demand) = demand {
                        self.production_modifiers.raise_for_demand(demand);
                    }
                })
            }),
            // Native land-event writers cannot grow their bounded word log.
            // Unlike the two worker-grown hints above, this stays prompt.
            Some(
                crate::country_scale::observe_land_event_words(memory, base).map(|demand| {
                    if let Some(words) = demand {
                        crate::land_events::presize(words);
                        self.land_events.words =
                            self.land_events.words.max(crate::land_events::capacity());
                    }
                }),
            ),
        ]
        .into_iter()
        .flatten()
        {
            if let Err(reason) = result {
                failure.get_or_insert(reason);
            }
        }
        if let Some(regions) = engine_native::cgame_state::read_strategic_region_count(memory, base)
        {
            self.raise_for_regions(regions as usize);
        }
        failure.map_or(Ok(()), Err)
    }

    pub(crate) fn raise_for_regions(&mut self, regions: usize) {
        crate::country_scale::raise(&mut self.air.scores.regions, regions);
    }

    /// Presize every table whose rows scale with the number of country slots,
    /// so a large mod's first hour is not truncated. Never lowers a capacity.
    pub fn raise_for_countries(&mut self, slots: usize) {
        // country_scale slots is the largest country/link/tag registry count.
        self.plans.raise_for_countries(slots);
        crate::country_scale::raise(
            &mut self.event_feed.events,
            crate::country_scale::rows(slots, 2),
        );
        self.experience_gains = self.experience_gains.max(slots);
        self.country_appearance.raise_for_countries(slots);
        self.country_counters.raise_for_countries(slots);
        self.game_score.raise_for_countries(slots);
        self.combat_details.raise_for_countries(slots);
        self.economy.raise_for_countries(slots);
        self.governance.raise_for_countries(slots);
        self.intel.raise_for_countries(slots);
        self.operation_outcomes.raise_for_countries(slots);
        self.air.raise_for_countries(slots);
        self.air_losses.raise_for_countries(slots);
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

        self.country_factories.raise_for_countries(slots);
        self.politics.raise_for_countries(slots);
        self.war_overview.raise_for_countries(slots);
        self.division_counts.raise_for_countries(slots);
        self.production_modifiers.raise_for_countries(slots);
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
        crate::country_scale::raise(
            &mut self.characters.bytes[3],
            crate::country_scale::rows(demand.unit_leader_bytes, 1),
        );
        self.intel.raise_for_script_flags(demand.flags);
        self.intel.raise_for_script_variables(demand.variables);
    }

    /// Presize the raid arenas whose per-country demand content decides.
    pub fn raise_for_raid_arenas(&mut self, demand: crate::country_scale::RaidArenas) {
        self.projects.raise_for_raid_type_tokens(demand.type_tokens);
        self.projects.raise_for_raid_paths(demand.path_rows);
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
        self.war_overview.raise_for_demand(demand.war_overview);
        characters::advisors::note_modifier_demand(demand.advisor_modifiers);
        crate::country_scale::raise(&mut self.intel.agency_names, demand.agency_name_bytes);
        crate::country_scale::raise(&mut self.intel.rows[17], demand.agency_upgrade_rows);
        crate::country_scale::raise(
            &mut self.intel.agency_upgrade_definitions,
            demand.agency_upgrade_definitions,
        );
        crate::country_scale::raise(&mut self.intel.rows[9], demand.operation_rows);
        self.operation_outcomes
            .raise_for_operations(demand.operation_rows);
        self.operation_outcomes
            .raise_for_descriptions(demand.operation_description_bytes);
        crate::country_scale::raise(&mut self.intel.rows[10], demand.operation_assignments);
        crate::country_scale::raise(
            &mut self.intel.operation_descriptions,
            demand.operation_description_bytes,
        );
        crate::country_scale::raise(&mut self.intel.rows[4], demand.operative_memberships);
        crate::country_scale::raise(&mut self.intel.operative_names, demand.operative_name_bytes);
        crate::country_scale::raise(&mut self.intel.rows[18], demand.operative_missions);
        crate::country_scale::raise(
            &mut self.intel.operative_mission_bytes,
            demand.operative_mission_bytes,
        );
        crate::country_scale::raise(&mut self.intel.operation_parts, demand.operation_parts);
        crate::country_scale::raise(&mut self.intel.rows[19], demand.strategic_operatives);
        crate::country_scale::raise(
            &mut self.intel.strategic_operative_entries,
            demand.strategic_operative_entries,
        );

        self.hired_advisors
            .raise_for_character_records(demand.active_characters, demand.retired_characters);
        self.character_inputs
            .raise_for_demand(demand.character_inputs);
        self.governance
            .raise_for_party_leaders(demand.party_leaders);
        self.governance
            .raise_for_effect_inputs(demand.timed_ideas, demand.decision_random_items);
        self.country_counters
            .raise_for_effect_rosters(demand.effect_rosters);
        self.country_counters
            .raise_for_co_belligerents(demand.co_belligerents);
        self.intel
            .raise_for_captured_operatives(demand.captured_operatives);
        self.construction
            .raise_for_occupied(demand.occupied_countries);
        self.country_appearance
            .raise_reserved_naming(demand.reserved_naming_bytes);
        self.country_appearance
            .raise_original_tag_lists(demand.original_tag_lists);
        self.country_appearance
            .raise_country_array(demand.country_array);
        self.country_counters
            .raise_for_civil_wars(demand.civil_wars);
        self.country_counters
            .raise_for_rosters(demand.country_rosters);
        self.hired_advisors
            .raise_for_advisor_tokens(demand.advisor_tokens, demand.advisor_token_bytes);
        self.hired_advisors.raise_for_available_advisors(
            demand.available_advisors,
            demand.available_advisor_bytes,
        );
        self.faction_theatres
            .raise_for_faction_advisors(demand.faction_advisors, demand.faction_advisor_bytes);
    }
}

impl Captured {
    #[cfg(test)]
    pub(crate) fn set_ordinary_failure(&mut self, reason: Option<&'static str>) {
        if reason.is_some() {
            self.naval_counters.forget_censuses();
        }
        self.ordinary_failure = reason;
    }
    pub(crate) fn game_score_pilot(&mut self) -> &mut game_score::Captured {
        &mut self.game_score
    }
    pub(crate) fn game_score_status(&self) -> &game_score::Captured {
        &self.game_score
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

    /// [`Self::reset`] off the game thread: every verdict is cleared, but the
    /// CountryIntel buffer this frame borrowed stays with it until the next
    /// callback returns it to the frames' shared home
    /// ([`Self::release_country_intel`]), so another frame's walk finds it
    /// held exactly as long as when the callback reset its frame.
    pub(crate) fn reset_keeping_country_intel(&mut self) {
        let loan = self.intel.take_country_intel_loan();
        self.reset();
        self.intel.restore_country_intel_loan(loan);
    }

    pub(crate) fn release_country_intel(&mut self) {
        self.intel.release_country_intel();
    }

    pub(crate) fn country_intel_sample(
        &self,
    ) -> Option<Result<replay_data::rows::ReservedCountryIntelSample, &'static str>> {
        self.intel.country_intel_sample()
    }

    pub(crate) fn take_combat_progress(&mut self) -> combat_progress::Captured {
        std::mem::replace(
            &mut self.combat_progress,
            combat_progress::Captured::with_capacities(&Default::default()),
        )
    }

    pub(crate) fn restore_combat_progress(
        &mut self,
        progress: combat_progress::Captured,
        ticks: u64,
    ) {
        self.land_events.lost_casualty_reads(progress.lost_reads());
        self.combat_progress = progress;
        self.overhead[TableId::LandCombatProgress.index()] += ticks;
    }

    pub(crate) fn operation_outcomes(&self) -> &operation_outcomes::Captured {
        &self.operation_outcomes
    }

    pub(crate) fn map_events(&self) -> &map_events::Captured {
        &self.map_events
    }

    /// Game thread, at the hourly cut; returns its ticks.
    pub(crate) fn seal_map_events(&mut self, hour: i32) -> u64 {
        let ticks = self.map_events.seal(hour);
        self.overhead[TableId::MapEvents.index()] =
            self.overhead[TableId::MapEvents.index()].saturating_add(ticks);
        ticks
    }

    pub(crate) fn discard_event_feed(&mut self) {
        self.event_feed.discard();
    }
    /// The observer boundary prepared the hour's event feed; a worker seals
    /// it once the Governance job's census has joined.
    pub(crate) fn take_event_feed_seal(&mut self) -> event_feed::Captured {
        std::mem::replace(&mut self.event_feed, event_feed::Captured::vacant())
    }
    /// A seal job that did not run refuses its prepared hour, so no hour
    /// stays prepared but unsealed.
    pub(crate) fn restore_event_feed_seal(
        &mut self,
        mut feed: event_feed::Captured,
        failure: Option<&'static str>,
    ) {
        if let Some(reason) = failure {
            feed.fail_prepared(reason);
        }
        self.event_feed = feed;
    }
    pub(crate) fn event_feed(&self) -> &event_feed::Captured {
        &self.event_feed
    }

    pub(crate) fn fuel_awards(&self) -> &fuel_awards::Captured {
        &self.fuel_awards
    }

    /// The sample's load signal, which names the session the Combat Log's
    /// kept rows belong to.
    pub(crate) fn set_load_signal(&mut self, signal: i64) {
        self.combat_log.set_signal(signal);
    }

    /// The production ticks this frame handed over, for the spool's totals.
    pub(crate) fn line_output(&self) -> &line_output::Captured {
        &self.line_output
    }

    /// The construction output this frame handed over, for the spool's totals.
    pub(crate) fn construction_ic(&self) -> &construction_ic::Captured {
        &self.construction_ic
    }

    /// Exact growth delta of the selected packs only, including any storage
    /// retained after a failed advisory reserve. Other heaps are unchanged.
    pub(crate) fn apply_combat_hints(
        &mut self,
        land: Option<&combat_details::Capacities>,
        naval: Option<&naval_details::Capacities>,
    ) -> ([bool; 2], usize) {
        let mut applied = [false; 2];
        let mut added = 0;
        if let Some(capacity) = land {
            let before = self.combat_details.allocated_bytes();
            applied[0] = self.combat_details.try_grow_hint(capacity).is_ok();
            added += self.combat_details.allocated_bytes().saturating_sub(before);
            if !applied[0] {
                crate::country_scale::note_combat_hint_failure(0);
            }
        }
        if let Some(capacity) = naval {
            let before = self.naval_details.allocated_bytes();
            applied[1] = self.naval_details.try_grow_hint(capacity).is_ok();
            added += self.naval_details.allocated_bytes().saturating_sub(before);
            if !applied[1] {
                crate::country_scale::note_combat_hint_failure(1);
            }
        }
        (applied, added)
    }

    #[cfg(test)]
    pub(crate) fn construction_ic_mut(&mut self) -> &mut construction_ic::Captured {
        &mut self.construction_ic
    }

    #[cfg(test)]
    pub(crate) fn line_output_mut(&mut self) -> &mut line_output::Captured {
        &mut self.line_output
    }

    pub(crate) fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        // The hour's sampled verification slice, for every job it dispatches.
        crate::sampled_verify::begin_hour(hour);
        self.group_counters.prepare_hour(force_full);
        // A frame whose sliced job never runs must not report an older read.
        self.governance.reset_slice_selections(hour, force_full);
        self.air_relations.prepare_hour(hour, force_full);
        self.air_histories.prepare_hour(hour, force_full);
        self.raid_launch_inputs.reset_selection(hour, force_full);
        self.raid_state_inputs.reset_selections(hour, force_full);
        self.resource_map.reset_selection(hour, force_full);
        self.country_stability.prepare_hour(hour, force_full);
        self.combat_log.prepare_hour(hour, force_full);
        self.combat_progress.prepare_hour(hour, force_full);
        self.characters.prepare_hour(hour, force_full);
        self.naval_mission_inputs.reset_selection(hour, force_full);
        self.supply_trains.reset_selection(hour, force_full);
        self.hired_advisors.reset_selection(hour, force_full);
        self.air.prepare_hour(hour, force_full);
        self.fog.prepare_hour(hour, force_full);
        self.naval_history.prepare_hour(hour, force_full);
        self.session.prepare_hour(hour, force_full);
    }

    /// Leases the same immutable native input packet to movement, status and
    /// the other division borrowers. The owning census accounts for its
    /// allocation once.
    pub(crate) fn install_unit_inputs(&mut self, inputs: Option<Arc<UnitInputs>>) {
        self.unit_inputs = inputs;
    }

    /// Release after every consumer joins, before the census returns to its pool.
    pub(crate) fn release_unit_inputs(&mut self) {
        self.unit_inputs = None;
    }

    pub(crate) fn take_settled_army_store(&mut self) -> Option<division_presentation::held_store::Loan> {
        self.division_presentation.take_settled_store()
    }

    /// No native-memory capability reaches the independent owned publication tails.
    pub(crate) fn decode_owned_censuses(&mut self) -> ([u64; 3], u64) {
        let bytes = |this: &Self| {
            this.naval_bases.allocated_bytes()
                + this.group_counters.allocated_bytes()
                + this.governance.allocated_bytes()
                + this.naval_counters.allocated_bytes()
        };
        let before = bytes(self);
        let start = crate::sampling::qpc();
        self.naval_bases.decode_owned();
        self.group_counters.decode_owned();
        let map = crate::sampling::qpc().saturating_sub(start).max(0) as u64;
        let start = crate::sampling::qpc();
        self.governance.decode_owned();
        let governance = crate::sampling::qpc().saturating_sub(start).max(0) as u64;
        let start = crate::sampling::qpc();
        self.naval_counters.decode_owned();
        let tasks = crate::sampling::qpc().saturating_sub(start).max(0) as u64;
        (
            [map, governance, tasks],
            (bytes(self) as u64).wrapping_sub(before as u64),
        )
    }

    pub(crate) fn discard_owned_censuses(&mut self) {
        self.naval_bases.discard_owned();
    }

    /// Independent, frame-owned mission tails. Native-demand indexes and
    /// certificate comparisons were completed before either pack returned.
    pub(crate) fn decode_mission_tails(&mut self) -> ([u64; 2], u64) {
        let before = self.naval_mission_inputs.allocated_bytes() + self.railway_guns.allocated_bytes();
        let start = crate::sampling::qpc();
        self.naval_mission_inputs.decode_owned();
        let navies = crate::sampling::qpc().saturating_sub(start).max(0) as u64;
        let start = crate::sampling::qpc();
        self.railway_guns.decode_owned();
        let railway = crate::sampling::qpc().saturating_sub(start).max(0) as u64;
        let after = self.naval_mission_inputs.allocated_bytes() + self.railway_guns.allocated_bytes();
        ([navies, railway], (after as u64).wrapping_sub(before as u64))
    }

    /// Project owned operands after their single live acquisition has finished.
    pub(crate) fn decode_division_presentation_inputs(&mut self) -> [u64; 2] {
        self.division_presentation.decode_inputs()
    }

    pub(crate) fn presentation_allocated_bytes(&self) -> usize {
        self.division_presentation.allocated_bytes()
    }

    /// Only these buffers can change during owned division installation.
    pub(crate) fn division_decode_allocated_bytes(&self) -> usize {
        self.division_counts.allocated_bytes()
            + self.production.division_equipment.allocated_bytes()
            + self.production.groups.allocated_bytes()
            + self.production.requests.allocated_bytes()
            + self.production.deliveries.allocated_bytes()
            + self.production.produced.allocated_bytes()
    }

    pub(crate) fn observe_division_naming(
        &self,
        work: &mut division_presentation::NamingWork,
    ) -> std::io::Result<()> {
        self.division_presentation.observe_naming(work)
    }

    pub(crate) fn observe_physical_navies(
        &self,
        work: &mut PhysicalNavyWork,
    ) -> std::io::Result<()> {
        self.country_counters
            .observe_physical_navies(work, self.country_appearance.launch_sources())
    }

    pub(crate) fn observe_reserved_visibility(
        &self,
        work: &mut reserved_visibility::Work,
    ) -> std::io::Result<()> {
        self.country_appearance.observe_reverse_visibility(work)
    }

    pub(crate) fn observe_battle_plan_child_fronts(&self, work: &mut plans::ChildFrontWork) {
        work.observe(&self.plans)
    }

    pub(crate) fn observe_battle_plan_execution(&self, work: &mut plans::ExecutionWork) {
        let (bytes, outcome) = self.division_presentation.order_transport_storage();
        work.observe(&self.plans, bytes, outcome);
    }

    pub(crate) fn observe_railway_inputs(
        &self,
        work: &mut crate::acquisition_states::InputWork,
    ) -> std::io::Result<()> {
        self.naval_history.observe_railway_inputs(work)
    }

    pub(crate) fn observe_counter_tooltip_inputs(
        &self,
        work: &mut division_presentation::CounterWork,
    ) -> std::io::Result<()> {
        self.division_presentation.observe_counter_inputs(work)?;
        self.group_counters.observe_counter_inputs(work);
        self.railway_guns.observe_supply(&mut work.supply);
        self.naval_counters.observe_supply(&mut work.supply);
        Ok(())
    }

    pub(crate) fn observe_country_rosters(
        &self,
        work: &mut country_counters::RosterWork,
        frequency: u64,
    ) -> std::io::Result<()> {
        self.country_counters.observe_rosters(work, frequency)?;
        self.construction.observe_occupied(work)
    }

    pub(crate) fn observe_character_naming(
        &self,
        work: &mut character_inputs::NamingWork,
    ) -> std::io::Result<()> {
        self.character_inputs.observe_naming(work)?;
        self.characters.observe_naming(work)
    }

    pub(crate) fn observe_supply_vehicles(&self, work: &mut supply_trains::VehicleWork) {
        self.supply_trains.observe_vehicles(work);
    }

    pub(crate) fn observe_variable_seeds(&self, work: &mut intel::SeedWork) {
        self.intel.observe_seeds(work);
    }

    pub(crate) fn politics(&self) -> &politics::Captured {
        &self.politics
    }

    /// Final scoped scalar acquisition, after the existing States, Provinces,
    /// ProvinceUnitOrder and Presentation jobs have all returned their leases.
    pub(crate) fn capture_border_inputs(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        provinces: (&[engine_native::cprovince::Row], Outcome),
        countries: (&mut [engine_native::ccountry::Row], &mut Outcome),
    ) -> [u64; 2] {
        let flags = self.state_flags.rows();
        if flags
            .as_ref()
            .is_ok_and(|rows| !border_inputs::needed(rows))
        {
            return [0; 2];
        }
        let selection_begin = qpc();
        let selection = flags.and_then(|flags| {
            let provinces = border_inputs::provinces(provinces.0, provinces.1, flags)?;
            Ok((flags, provinces))
        });
        let demanded =
            selection
                .as_ref()
                .map_err(|reason| *reason)
                .and_then(|(flags, provinces)| {
                    let land = self.province_unit_order.land()?;
                    border_inputs::land(land, provinces)?;
                    Ok(land.iter().filter_map(move |&(province, handle)| {
                        border_inputs::active(provinces, flags, province).then_some(handle)
                    }))
                });
        let selection_ticks = qpc().saturating_sub(selection_begin).max(0) as u64;
        let stats_ticks = self
            .division_presentation
            .capture_border_stats(memory, base, demanded);
        let _reads = crate::read_accounting::table(TableId::Countries);
        let begin = qpc();
        let outcome = selection.and_then(|(flags, provinces)| {
            border_inputs::doctrine(
                memory,
                base,
                game_state,
                provinces,
                flags,
                countries.0,
                *countries.1,
            )
        });
        if let Err(reason) = outcome
            && matches!(*countries.1, Outcome::Rows { .. })
        {
            *countries.1 = Outcome::Unavailable { reason };
        }
        [
            stats_ticks,
            selection_ticks + qpc().saturating_sub(begin).max(0) as u64,
        ]
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_presentation_request(
        &mut self,
        countries: &Arc<engine_native::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        army_stats: Option<division_presentation::held_store::Loan>,
    ) -> crate::acquisition_jobs::PresentationRequest {
        self.presentation_report = None;
        if army_stats.is_some() {
            self.division_presentation.enable_owned_settlement();
        }
        crate::acquisition_jobs::PresentationRequest {
            dispatch: Default::default(),
            countries: Arc::clone(countries),
            army_stats,
            context: division_presentation::CaptureContext {
                base,
                game_state: state,
                hour,
            },
            inputs: self.unit_inputs.clone(),
            captured: std::mem::replace(
                &mut self.division_presentation,
                division_presentation::Captured::vacant(),
            ),
        }
    }

    /// Returns the `army_stat` store it lent, forgotten if the job failed.
    pub(crate) fn restore_presentation_request(
        &mut self,
        mut request: crate::acquisition_jobs::PresentationRequest,
        report: Result<division_presentation::CaptureReport, crate::acquisition::Failure>,
    ) -> Option<division_presentation::held_store::Loan> {
        if report.is_err()
            && let Some(store) = request.army_stats.as_deref_mut()
        {
            store.forget();
        }
        let army_stats = request.army_stats.take().and_then(|store| {
            if report.is_ok() {
                request.captured.retain_owned_settlement(
                    store,
                    request.inputs.clone(),
                    request.context,
                )
            } else {
                Some(store)
            }
        });
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
        army_stats
    }

    fn acquire_presentation(&mut self) -> division_presentation::CaptureReport {
        self.presentation_report.take().unwrap_or_else(|| {
            self.division_presentation
                .fail_owned_capture("division presentation acquisition not supplied")
        })
    }

    /// Take the direct walk's ticks once the template job has joined;
    /// nothing is read here.
    pub(crate) fn take_preacquired_templates(
        &mut self,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) {
        self.template_ticks = Some(
            self.division_templates
                .take_preacquired(base, state, hour, force_full),
        );
    }

    /// This hour's template rows whose model inputs failed, published in status.json.
    pub(crate) fn template_model_failures(&self) -> division_templates::ModelFailures {
        self.division_templates.model_failures()
    }

    /// This hour's DivisionStatus span by pass, published in status.json.
    pub(crate) fn presentation_phase_ticks(&self) -> [u64; division_presentation::PHASES] {
        self.division_presentation.phase_ticks()
    }

    /// This hour's CountryStabilityInputs span by walk phase, published in
    /// status.json.
    pub(crate) fn stability_phase_ticks(&self) -> [u64; 4] {
        self.country_stability.phase_ticks()
    }

    /// Loan the governance family's hourly research tables to their own job.
    /// Take before the family itself; restore after the family is restored.
    pub(crate) fn take_research_request(
        &mut self,
        countries: &Arc<engine_native::country_memory::Cache>,
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

    pub(crate) fn weather_suspend_failure(&self) -> Option<&'static str> {
        self.visibility.weather_suspend_failure()
    }
    pub(crate) fn weather_display_failure(&self) -> Option<&'static str> {
        self.weather.display_failure()
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
                && !matches!(outcomes[index], Outcome::Unavailable { reason } if reason == replay_data::cadence::SKIPPED_REASON)
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
            crate::slicing::FOCUS => self.governance.focus_selection(),
            crate::slicing::PROJECTS => self.projects.project_selection(),
            crate::slicing::RAID_LAUNCH => self.raid_launch_inputs.selection(),
            crate::slicing::RAID_STATE => self.raid_state_inputs.state_selection(),
            crate::slicing::RAID_BUILDINGS => self.raid_state_inputs.country_selection(),
            crate::slicing::TRADE_ROUTES => self.resource_map.route_selection(),
            crate::slicing::RELATIONS => self.intel.relations_selection(),
            crate::slicing::STABILITY => self.country_stability.selection(),
            crate::slicing::COMBAT_LOG => self.combat_log.selection(),
            crate::slicing::AIR_RELATIONS => self.air_relations.selection(),
            crate::slicing::AIR_HISTORIES => self.air_histories.selection(),
            crate::slicing::NAVAL_MISSIONS => self.naval_mission_inputs.selection(),
            crate::slicing::SUPPLY_TRAINS => self.supply_trains.selection(),
            crate::slicing::AVAILABLE_ADVISORS => self.hired_advisors.available_selection(),
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_governance_request(
        &mut self,
        countries: &Arc<engine_native::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        technology_fingerprints: Box<crate::slicing::SliceFingerprints>,
        focus_fingerprints: Box<crate::slicing::SliceFingerprints>,
        building_store: Option<Box<construction::BuildingStore>>,
        leader_store: Option<Box<governance::LeaderStore>>,
        party_store: Option<Box<governance::PartyStore>>,
    ) -> crate::acquisition_jobs::GovernanceRequest {
        self.governance_report = None;
        crate::acquisition_jobs::GovernanceRequest {
            technology_fingerprints,
            focus_fingerprints,
            building_store,
            leader_store,
            party_store,
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

    /// Loan the hourly design changes with the variant registry and its detail
    /// tables. The worker drains changes before deciding its roster read.
    pub(crate) fn take_variants_request(
        &mut self,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> crate::acquisition_jobs::VariantsRequest {
        self.variants_ticks = None;
        let context = governance::CaptureContext {
            base,
            game_state: state,
            hour,
            force_full,
        };
        crate::acquisition_jobs::VariantsRequest {
            context,
            registry: std::mem::replace(&mut self.registry, Registry::new(0)),
            variants: self.production.take_variants(),
            changes: self.production.take_design_changes(),
            changes_ticks: 0,
            changes_finished: false,
        }
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
        let changes = TableId::VariantChanges.index();
        self.overhead[changes] = self.overhead[changes].saturating_add(request.changes_ticks);
        self.registry = request.registry;
        self.production.restore_variants(request.variants);
        self.production.restore_design_changes(request.changes);
    }

    /// Returns the technology and focus fingerprint stores and the held
    /// building, party-leader and party stores; a failed job forgets each. A
    /// family whose table was not read completely has this hour's selection
    /// invalidated, so its owners are read at the next hour.
    pub(crate) fn restore_governance_request(
        &mut self,
        mut request: crate::acquisition_jobs::GovernanceRequest,
        report: Result<crate::acquisition_jobs::GovernanceReport, crate::acquisition::Failure>,
    ) -> GovernanceStores {
        let report = match report {
            Ok(report) => report,
            Err(failure) => {
                request.technology_fingerprints.forget();
                request.focus_fingerprints.forget();
                if let Some(store) = request.building_store.as_deref_mut() {
                    store.forget();
                }
                if let Some(store) = request.leader_store.as_deref_mut() {
                    store.forget();
                }
                if let Some(store) = request.party_store.as_deref_mut() {
                    store.forget();
                }
                request.fail(match failure {
                    crate::acquisition::Failure::Panicked => "governance acquisition panicked",
                    crate::acquisition::Failure::Stopped => {
                        "governance acquisition stopped before reading"
                    }
                })
            }
        };
        let outcomes = request.captured.outcomes();
        let unread = |family: &[usize]| {
            family.iter().any(|&index| {
                !matches!(outcomes[index], Outcome::Rows { .. })
                    && !matches!(outcomes[index], Outcome::Unavailable { reason } if reason == replay_data::cadence::SKIPPED_REASON)
            })
        };
        if unread(&governance::TECHNOLOGIES) {
            request
                .captured
                .technology_selection()
                .invalidate(&mut request.technology_fingerprints);
        }
        if unread(&governance::FOCUS) {
            request
                .captured
                .focus_selection()
                .invalidate(&mut request.focus_fingerprints);
        }
        self.governance_report = Some(report.governance);
        self.governance = request.captured;
        self.construction
            .restore_buildings(request.buildings, Ok(report.buildings));
        (
            request.technology_fingerprints,
            request.focus_fingerprints,
            request.building_store,
            request.leader_store,
            request.party_store,
        )
    }

    /// Loan existing hourly buffers before independent acquisition begins.
    /// Restore them before ordinary capture, table views, growth or publication.
    /// Loan one share of the supply node census; part 0 carries the trains,
    /// the hour's owner selection and the family's fingerprint store.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take_supply_trains_request(
        &mut self,
        countries: &Arc<engine_native::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        part: usize,
        fingerprints: Option<Box<crate::slicing::SliceFingerprints>>,
    ) -> crate::acquisition_jobs::SupplyTrainsRequest {
        if part == 0 {
            self.supply_trains_report = None;
        }
        crate::acquisition_jobs::SupplyTrainsRequest {
            countries: Arc::clone(countries),
            context: supply_trains::CaptureContext {
                base,
                game_state: state,
                hour,
                force_full,
            },
            part,
            nodes: self.supply_trains.take_part(part),
            trains: (part == 0).then(|| self.supply_trains.take_trains()),
            slice: (part == 0).then(|| supply_trains::Slice {
                selection: self.supply_trains.take_selection(),
                fingerprints,
            }),
        }
    }

    /// Every part adds its ticks; one failed part is terminal for the family.
    /// Keeping the report Some prevents the ordinary stage from retrying reads.
    /// Returns the family's fingerprint store with part 0: forgotten if the
    /// job failed, and with this hour's selection invalidated when a table
    /// was not read whole, so its countries are read again at the next hour.
    pub(crate) fn restore_supply_trains_request(
        &mut self,
        mut request: crate::acquisition_jobs::SupplyTrainsRequest,
        report: Result<supply_trains::CaptureReport, crate::acquisition::Failure>,
    ) -> Option<Box<crate::slicing::SliceFingerprints>> {
        let failed = report.is_err();
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
        let slice = request.slice.take()?;
        self.supply_trains.restore_selection(slice.selection);
        let mut fingerprints = slice.fingerprints;
        if let Some(store) = fingerprints.as_mut() {
            if failed {
                store.forget();
            } else if self
                .supply_trains
                .outcomes()
                .iter()
                .any(|outcome| !matches!(outcome, Outcome::Rows { .. }))
            {
                self.supply_trains.selection().invalidate(store);
            }
        }
        fingerprints
    }

    fn acquire_supply_trains(
        &mut self,
        memory: &impl Memory,
        context: supply_trains::CaptureContext,
    ) -> supply_trains::CaptureReport {
        self.supply_trains_report.take().unwrap_or_else(|| {
            #[cfg(test)]
            {
                self.supply_trains.capture_owned(memory, context)
            }
            #[cfg(not(test))]
            {
                let _ = (memory, context);
                self.supply_trains
                    .fail_owned_capture("supply_trains acquisition not supplied")
            }
        })
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
        self.character_portraits_report.take().unwrap_or_else(|| {
            #[cfg(test)]
            {
                self.character_portraits.capture_owned(memory, context)
            }
            #[cfg(not(test))]
            {
                let _ = (memory, context);
                self.character_portraits
                    .fail_owned_capture("character_portraits acquisition not supplied")
            }
        })
    }

    /// Loan the complete ProvinceUnitOrder rows and handle arena to the flat
    /// acquisition pool, with the hour's division census when it completed
    /// and the shared held walk. Restore before capture_tables publishes this
    /// pack.
    pub(crate) fn take_province_unit_order_request(
        &mut self,
        base: usize,
        state: usize,
        inputs: Option<Arc<crate::division_census::UnitInputs>>,
        store: Option<Box<province_unit_order::ProvinceUnitStore>>,
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
            inputs,
            store,
        }
    }

    /// Takes back the rows and releases the census lease. Returns the held
    /// walk's store it lent, forgotten if the job failed.
    pub(crate) fn restore_province_unit_order_request(
        &mut self,
        mut request: crate::acquisition_jobs::ProvinceUnitOrderRequest,
        report: Result<province_unit_order::CaptureReport, crate::acquisition::Failure>,
    ) -> Option<Box<province_unit_order::ProvinceUnitStore>> {
        drop(request.inputs.take());
        if report.is_err()
            && let Some(store) = request.store.as_deref_mut()
        {
            store.forget();
        }
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
        request.store
    }

    fn acquire_province_unit_order(
        &mut self,
        memory: &impl Memory,
        context: province_unit_order::CaptureContext,
    ) -> province_unit_order::CaptureReport {
        self.province_unit_order_report.take().unwrap_or_else(|| {
            #[cfg(test)]
            {
                self.province_unit_order
                    .capture_owned(memory, context, None, None)
            }
            #[cfg(not(test))]
            {
                let _ = (memory, context);
                self.province_unit_order
                    .fail_owned_capture("province_unit_order acquisition not supplied")
            }
        })
    }

    /// Account for the reusable boxes and any payload they currently own.
    fn spare_bytes(&self) -> usize {
        self.ordinary_spare
            .as_ref()
            .map_or(0, |packs| packs.allocated_bytes())
            + self
                .ordinary_part_spares
                .iter()
                .flatten()
                .map(OrdinaryPart::allocated_bytes)
                .sum::<usize>()
            + self
                .slow_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
            + self
                .stability_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
            + self
                .war_overview_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
            + self
                .production_daily_spare
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
            + self
                .scores_rails_spare
                .as_ref()
                .map_or(0, |parts| parts.allocated_bytes())
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
        countries: &Arc<engine_native::country_memory::Cache>,
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

    /// Loan the air pack, every history part and the held registrations to
    /// the AirMissions job. Before the map sample it runs on the owned
    /// census's identities; after it, on the typed visitor's, whose census
    /// is `air_root`. It borrows nothing an Ordinary part holds.
    pub(crate) fn take_air_missions_request(
        &mut self,
        countries: &Arc<engine_native::country_memory::Cache>,
        base: usize,
        state: usize,
        mission_store: Option<Box<air::MissionStore>>,
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
            mission_store,
        }
    }

    /// An interrupted job fails the registrations and combats, and leaves
    /// the history parts without regions; nothing is read again. Returns
    /// the held registrations, which such a job forgets.
    pub(crate) fn restore_air_missions_request(
        &mut self,
        mut request: crate::acquisition_jobs::AirMissionsRequest,
        completed: bool,
    ) -> Option<Box<air::MissionStore>> {
        let mut mission_store = request.mission_store.take();
        if !completed {
            request
                .pack
                .fail_missions("air mission acquisition failed", &mut request.histories);
            if let Some(store) = mission_store.as_deref_mut() {
                store.forget();
            }
        }
        std::mem::swap(&mut self.air, &mut request.pack);
        self.missions_spare = Some(request.pack);
        self.air_histories.restore_parts(request.histories);
        mission_store
    }

    /// Loan one part of the naval-patrol history copies to its job.
    pub(crate) fn take_air_histories_request(
        &mut self,
        countries: &Arc<engine_native::country_memory::Cache>,
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

    /// FIFO-owned tails of completed native acquisitions. None accepts Memory.
    pub(crate) fn decode_capture_tails(&mut self) -> ([u64; 5], u64) {
        let bytes = |this: &Self| {
            this.division_templates.allocated_bytes()
                + this.ship_models.owned_tail_allocated_bytes()
                + this.combat_details.owned_tail_allocated_bytes()
        };
        let before = bytes(self);
        let mut ticks = [0; 5];
        macro_rules! settle {
            ($index:expr, $capture:ident, $method:ident) => {{
                let start = crate::sampling::qpc();
                self.$capture.$method();
                ticks[$index] = crate::sampling::qpc().saturating_sub(start).max(0) as u64;
            }};
        }
        settle!(0, division_templates, settle_owned);
        settle!(1, ship_models, settle_owned);
        settle!(2, resource_map, settle_owned);
        settle!(3, production, settle_variants_owned);
        settle!(4, combat_details, settle_owned);
        (ticks, (bytes(self) as u64).wrapping_sub(before as u64))
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

    /// Loan the order census and the plans to the Orders job, which hands
    /// them on to the ChildFronts job.
    pub(crate) fn take_orders_request(
        &mut self,
        countries: &Arc<engine_native::country_memory::Cache>,
        orders: &mut Orders,
        base: usize,
        game_state: usize,
    ) -> crate::acquisition_jobs::OrdersRequest {
        crate::acquisition_jobs::OrdersRequest {
            countries: Arc::clone(countries),
            base,
            game_state,
            orders: std::mem::replace(orders, Orders::vacant()),
            plans: std::mem::replace(&mut self.plans, plans::Captured::vacant()),
            group_source: None,
        }
    }

    /// The fronts the Fronts job read, into the plans the Late job returned.
    pub(crate) fn install_fronts(&mut self, fronts: plans::Fronts, completed: bool) {
        self.plans.install_fronts(fronts, completed);
    }

    /// Restore terminal outcomes without retrying either allocating walk;
    /// returned by the ChildFronts join.
    pub(crate) fn restore_orders_request(
        &mut self,
        mut request: crate::acquisition_jobs::OrdersRequest,
        completed: bool,
        orders: &mut Orders,
    ) {
        if !completed {
            request.fail("army order acquisition failed");
        }
        // Request Drop owns an unreturned Naval loan on every unwind path.
        // Swap the capture buffers back instead of moving out of that owner.
        std::mem::swap(orders, &mut request.orders);
        std::mem::swap(&mut self.plans, &mut request.plans);
    }

    /// The job walks the ship census into `census` first; the ships seed the
    /// model details.
    pub(crate) fn take_ship_models_request(
        &mut self,
        countries: &Arc<engine_native::country_memory::Cache>,
        base: usize,
        state: usize,
        census: ShipCensus,
    ) -> crate::acquisition_jobs::ShipModelsRequest {
        self.ship_models_report = None;
        crate::acquisition_jobs::ShipModelsRequest {
            countries: Arc::clone(countries),
            context: ship_models::CaptureContext::new(base),
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
        request.captured.set_context(request.context);
        self.ship_models = request.captured;
        request.census
    }

    /// Split the walked census into detail parts sized from its ship count
    /// and lend them to their jobs; part 0 is the family's own capture. A
    /// failed census lends nothing. Allocation-free.
    pub(crate) fn take_ship_model_parts(
        &mut self,
        countries: &Arc<engine_native::country_memory::Cache>,
        base: usize,
    ) -> [Option<crate::acquisition_jobs::ShipModelDetailsRequest>; ship_models::MAX_PARTS] {
        let mut requests = [const { None }; ship_models::MAX_PARTS];
        if self.ship_models_report.is_some() || self.ship_models.has_terminal_failure() {
            return requests;
        }
        let parts = self
            .ship_models
            .split_details(ship_models::parts_for(self.ship_models.census_len()));
        let context = self.ship_models.capture_context(base);
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
        countries: &Arc<engine_native::country_memory::Cache>,
        context: intel::CaptureContext,
        flags: Option<Box<intel::FlagStore>>,
        variables: Option<Box<intel::VariableStore>>,
    ) -> crate::acquisition_jobs::IntelResidualRequest {
        self.intel_residual_report = None;
        crate::acquisition_jobs::IntelResidualRequest {
            countries: Arc::clone(countries),
            context,
            captured: std::mem::replace(&mut self.intel, intel::Captured::vacant()),
            flags,
            variables,
        }
    }

    /// Returns the held ScriptFlags and ScriptVariables stores; a failed job
    /// forgets each.
    pub(crate) fn restore_intel_residual_request(
        &mut self,
        mut request: crate::acquisition_jobs::IntelResidualRequest,
        report: Result<intel::CaptureReport, crate::acquisition::Failure>,
    ) -> (
        Option<Box<intel::FlagStore>>,
        Option<Box<intel::VariableStore>>,
    ) {
        let mut flags = request.flags.take();
        let mut variables = request.variables.take();
        if report.is_err() {
            if let Some(store) = flags.as_deref_mut() {
                store.forget();
            }
            if let Some(store) = variables.as_deref_mut() {
                store.forget();
            }
        }
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
        (flags, variables)
    }
    /// Resolve the hour's residual Intel report now, exactly as
    /// `capture_tables` would, so the final borrowers can take the intel pack
    /// before it: `capture_tables` then takes the settled report.
    pub(crate) fn settle_intel_residual(
        &mut self,
        memory: &impl Memory,
        context: intel::CaptureContext,
    ) {
        let report = self.acquire_intel(memory, context);
        self.intel_residual_report = Some(report);
    }

    fn acquire_intel(
        &mut self,
        memory: &impl Memory,
        context: intel::CaptureContext,
    ) -> intel::CaptureReport {
        if let Some(report) = self.intel_residual_report.take() {
            report
        } else {
            #[cfg(test)]
            {
                intel::CaptureReport {
                    ticks: self.intel.capture(
                        memory,
                        context.base,
                        context.game_state,
                        context.hour,
                        context.force_full,
                    ),
                }
            }
            #[cfg(not(test))]
            {
                let _ = memory;
                self.intel
                    .fail_residual_capture(context, "residual Intel acquisition not supplied")
            }
        }
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

    /// Decode-worker handoff for the source-fed naval block (copy/decode
    /// split). Sole cross-thread entry to the naval theatre/OOB buffers: the
    /// game thread stages raw-copy state through it every hour and the
    /// single FIFO decode worker decodes the staged payload into these
    /// pooled typed rows before the spool ready-queue handoff.
    pub(crate) fn naval_pilot(&mut self) -> &mut naval_history::Captured {
        &mut self.naval_history
    }

    pub(crate) fn install_reserved_land_certificate(&mut self) {
        if !self.country_appearance.reserved_land_certificate_ready() {
            self.naval_history
                .withhold_reserved_land_certificate("reserved naming singleton prefix unavailable");
        }
        self.country_appearance
            .install_land_theatre_count(self.naval_history.reserved_land_count());
    }

    pub(crate) fn decode_naval_payload(
        &mut self,
        base: usize,
        job: &naval_history::NavalJob,
        decode_ticks: &mut [u64; naval_history::NAVAL_PACK_N],
    ) {
        naval_history::decode_naval_payload(&mut self.naval_history, base, job, decode_ticks);
        self.install_reserved_land_certificate();
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
            replay_data::cadence::SKIPPED_REASON
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
        use engine_deployment::{
            army_upgrade_request_row, carmy_upgrades_requests, cupgrade_delivery,
        };
        self.production.division_equipment.rows =
            vec![engine_native::cequipment_variant_pool::DivisionRow::default(); lens[1]];
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
    pub(crate) fn install_division_sources(
        &mut self,
        rows: &mut Vec<DivisionSource>,
        outcome: Outcome,
    ) {
        std::mem::swap(&mut self.divisions.first, rows);
        self.divisions.seen = outcome.len();
        self.divisions.required = outcome.len();
        self.divisions.outcome = outcome;
    }
    pub(crate) fn visit_division_source(&mut self, source: DivisionSource) {
        self.divisions.visit_source(source);
    }
    #[cfg(test)]
    pub(crate) fn test_division_presentation_tables(
        &self,
    ) -> [replay_frame::frame::TableView; division_presentation::N] {
        self.division_presentation.tables()
    }
    pub(crate) fn install_division_counts(&mut self, census: &crate::division_census::Census) {
        self.division_counts.install(census);
    }
    pub(crate) fn install_failed_division_counts(&mut self, reason: &'static str) {
        self.division_counts.fail(reason);
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
            let countries =
                Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
            self.capture_ordinary_inline(
                &countries,
                memory,
                base,
                game_state,
                current_hour,
                Ok(()),
                NavalMissionLoan {
                    force_full,
                    ..Default::default()
                },
            );
        }
        if self.presentation_report.is_none() {
            self.presentation_report = Some(self.division_presentation.capture_owned(
                memory,
                division_presentation::CaptureContext {
                    base,
                    game_state,
                    hour: current_hour,
                },
                unit_sources(&self.unit_inputs),
            ));
        }
        let countries =
            Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
        if self.intel_residual_report.is_none() {
            self.intel_residual_report = Some(self.intel.capture_residual(
                memory,
                intel::CaptureContext {
                    base,
                    game_state,
                    hour: current_hour,
                    force_full,
                },
                None,
                None,
            ));
        }
        let mut final_request =
            self.take_final_inputs_request(&countries, base, game_state, current_hour, force_full);
        let report = final_request.capture(memory);
        self.restore_final_inputs_request(final_request, report, None);
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
    /// The daily production tables' share of the assembly. The slot's job is
    /// queued exactly when its walks run; its ticks are taken once, and a
    /// second assembly neither reruns nor changes them. A missing job on the
    /// slot refuses them as a missing Slow did, unless `inline` (unit-test
    /// fixtures without jobs) walks them here. Off the slot both captures run
    /// here and skip without a read or an allocation (reset and SKIPPED).
    /// Slow's refusal refuses them too, with its reason and no cost, as when
    /// they were Slow's packs (off the slot, failed_capture keeps them
    /// SKIPPED).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn assemble_production_daily(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        force_full: bool,
        ticks: &mut [u64; TABLE_COUNT],
        inline: bool,
    ) {
        if let Some(done) = self.production_daily_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else if !ProductionDailyPacks::due(current_hour, force_full) || inline {
            if !ProductionDailyPacks::due(current_hour, force_full) {
                self.production_daily_ticks = None;
            }
            let cost = self.industrial_organisations.capture(
                memory,
                base,
                game_state,
                current_hour,
                force_full,
            );
            scatter(&industrial_organisations::TABLES, cost, ticks);
            let cost = self.production_modifiers.capture(
                memory,
                base,
                game_state,
                current_hour,
                force_full,
            );
            scatter(&production_modifiers::TABLES, cost, ticks);
        } else {
            self.fail_missing_production_daily(current_hour, force_full);
        }
        if let Some(reason) = self.slow_refusal {
            self.industrial_organisations
                .fail_capture(current_hour, force_full, reason);
            self.production_modifiers
                .fail_capture(current_hour, force_full, reason);
            for table in industrial_organisations::TABLES
                .into_iter()
                .chain(production_modifiers::TABLES)
            {
                ticks[table.index()] = 0;
            }
        }
    }

    // Keep the assembly's table-sized temporaries off the hourly body's frame.
    #[inline(never)]
    pub(crate) fn capture_tables(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        force_full: bool,
    ) -> [u64; TABLE_COUNT] {
        let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        macro_rules! ordinary {
            ($($pack:ident),+ $(,)?) => {$(
                let cost = self.$pack.capture(memory, base, game_state);
                scatter(&$pack::TABLES, cost, &mut ticks);
            )+};
        }
        #[cfg(test)]
        macro_rules! staggered {
            ($($pack:ident),+ $(,)?) => {$(
                let cost = self.$pack.capture(memory, base, game_state, current_hour, force_full);
                scatter(&$pack::TABLES, cost, &mut ticks);
            )+};
        }
        // StateFlags was read by the States worker; capture consumes its ticks.
        #[cfg(test)]
        let weather_ticks = self.weather.capture(memory, base, game_state);
        #[cfg(not(test))]
        let weather_ticks = self.weather.finish_owned_capture();
        scatter(&weather::TABLES, weather_ticks, &mut ticks);
        ordinary!(state_flags, event_feed);
        if let Some(done) = self.ordinary_ticks.take() {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            self.ordinary_failure = Some("ordinary acquisition not supplied");
            // A family that never completed holds no counter census.
            self.naval_counters.forget_censuses();
        }
        // Keep the completion marker after consuming its cost: accepted
        // failure cannot become an inline retry on a second assembly call.
        if let Some(done) = self.slow_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            #[cfg(test)]
            {
                staggered!(
                    peace_bids,
                    map_names,
                    faction_theatres,
                    model_inputs,
                    modifier_definitions,
                    hired_advisors,
                    country_tactic_unlocks,
                    logistics,
                    combat_log,
                    country_factories
                );
                scatter(
                    &visibility::TABLES,
                    self.visibility.capture_growing(
                        memory,
                        base,
                        game_state,
                        current_hour,
                        force_full,
                    ),
                    &mut ticks,
                );
            }
            #[cfg(not(test))]
            {
                self.fail_missing_slow(current_hour, force_full);
            }
        }
        if let Some(done) = self.stability_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            #[cfg(test)]
            {
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
            #[cfg(not(test))]
            {
                self.fail_missing_stability(current_hour, force_full);
            }
        }
        // A missing job on its slot fails its tables outside unit tests;
        // their direct-walk fixtures read them inline instead.
        self.assemble_production_daily(
            memory,
            base,
            game_state,
            current_hour,
            force_full,
            &mut ticks,
            cfg!(test),
        );
        if let Some(done) = self.war_overview_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            #[cfg(test)]
            {
                staggered!(war_overview);
            }
            #[cfg(not(test))]
            {
                self.fail_missing_war_overview(current_hour, force_full);
            }
        }
        // The Countries job read the template dictionary with its naming.
        if let Some(done) = self.template_ticks.replace([0; division_templates::N]) {
            scatter(&division_templates::TABLES, done, &mut ticks);
        } else {
            #[cfg(test)]
            {
                staggered!(division_templates);
            }
            #[cfg(not(test))]
            {
                self.division_templates.fail_capture(
                    current_hour,
                    force_full,
                    "template acquisition not supplied",
                );
            }
        }
        if let Some(done) = self.raid_state_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            #[cfg(test)]
            {
                staggered!(raid_state_inputs, resource_map);
            }
            #[cfg(not(test))]
            {
                self.fail_missing_raid_state(current_hour, force_full);
            }
        }
        if let Some(done) = self.raid_targets_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            #[cfg(test)]
            {
                staggered!(raid_target_statuses);
            }
            #[cfg(not(test))]
            {
                self.fail_missing_raid_targets(current_hour, force_full);
            }
        }
        if let Some(done) = self.raid_launch_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            #[cfg(test)]
            {
                staggered!(raid_launch_inputs);
            }
            #[cfg(not(test))]
            {
                self.fail_missing_raid_launch(current_hour, force_full);
            }
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
            #[cfg(test)]
            {
                scatter(
                    &fog::TABLES,
                    self.fog
                        .capture_growing(memory, base, game_state, current_hour, force_full),
                    &mut ticks,
                );
                staggered!(construction, projects);
            }
            #[cfg(not(test))]
            {
                self.fail_missing_gated(current_hour, force_full);
            }
        }
        if let Some(done) = self
            .character_inputs_ticks
            .replace([0; character_inputs::N])
        {
            scatter(&character_inputs::TABLES, done, &mut ticks);
        } else {
            #[cfg(test)]
            {
                staggered!(character_inputs);
            }
            #[cfg(not(test))]
            {
                self.character_inputs.fail_capture(
                    current_hour,
                    force_full,
                    "character inputs acquisition not supplied",
                );
            }
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
        let supply_report = self.acquire_supply_trains(
            memory,
            supply_trains::CaptureContext {
                base,
                game_state,
                hour: current_hour,
                force_full,
            },
        );
        scatter(&supply_trains::TABLES, supply_report.ticks, &mut ticks);
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
    #[inline(never)]
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
        let _allocation_phase = crate::allocation::enter(crate::allocation::Phase::Extended);
        if let Some(done) = self.late_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            #[cfg(test)]
            {
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
                    Some(&self.divisions),
                    Some(&mut engine_raid_inputs::raid_air_scores::Held::default()),
                    None,
                );
                let cost = self.plans.capture_unit_transports(
                    memory,
                    base,
                    game_state,
                    self.unit_inputs.as_deref(),
                    &packs.orders,
                );
                packs.ticks[TableId::DivisionMovement.index()] += cost;
                for (total, cost) in ticks.iter_mut().zip(packs.ticks) {
                    *total += cost;
                }
                self.swap_late(&mut packs, orders);
                self.late_spare = Some(packs);
                self.plans
                    .install_order_transport(&mut self.division_presentation);
            }
            #[cfg(not(test))]
            {
                self.fail_missing_late(current_hour, force_full, orders);
            }
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
        if let Some(report) = self.final_inputs_ticks.take() {
            let [divisions, experience] = report.snapshots;
            scatter(&production::TABLES, report.production, ticks);
            scatter(&economy::TABLES, report.economy, ticks);
            scatter(&session::TABLES, report.session, ticks);
            scatter(&characters::TABLES, report.characters, ticks);
            scatter(&equipment_destruction::TABLES, report.destruction, ticks);
            scatter(&naval_result_sources::TABLES, report.naval_sources, ticks);
            scatter(&air_losses::TABLES, report.air_losses, ticks);
            scatter(&air_records::TABLES, report.air_records, ticks);
            scatter(&building_bombing::TABLES, report.bombing, ticks);
            scatter(&nuclear_strikes::TABLES, report.nuclear, ticks);
            scatter(&market::TABLES, report.market, ticks);
            for (table, cost) in intel::TABLES.into_iter().zip(report.intel_delegated) {
                ticks[table.index()] += cost;
            }
            ticks[TableId::NewDivisionEquipment.index()] += divisions;
            ticks[TableId::CountryExperienceGains.index()] += experience;
        }
        let report = self.acquire_presentation();
        scatter(&division_presentation::TABLES, report.ticks, ticks);
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
        let context = ship_models::CaptureContext::new(base);
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
    fn land(&mut self, source: engine_native::cland_combat::Source) -> u64 {
        let begin = qpc();
        self.captured
            .combat_progress
            .visit(self.memory, self.base, source);
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
    fn finish_land(&mut self, outcome: Outcome) -> u64 {
        let begin = qpc();
        self.captured.combat_progress.finish(outcome);
        self.captured
            .land_events
            .lost_casualty_reads(self.captured.combat_progress.lost_reads());
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
    fn begin_air(&mut self) -> u64 {
        if self.air_raw {
            return 0;
        }
        let begin = qpc();
        self.captured.air.begin_wings();
        self.captured.game_score.begin_air();
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
    fn air_score_limit(&self) -> Option<usize> {
        if self.air_raw {
            None
        } else {
            self.captured.game_score.wing_limit()
        }
    }
    fn air_score(&mut self, pointer: usize, row: &cair_wing::ScoreLeaf) -> u64 {
        if self.air_raw {
            return 0;
        }
        let _reads = crate::read_accounting::table(TableId::AirWingGameScoreStats);
        let started = qpc();
        self.captured.game_score.wing(self.memory, pointer, row);
        self.attribute_overhead(TableId::AirWingGameScoreStats, started, 0)
    }
    fn finish_air_score(&mut self, outcome: Outcome) -> u64 {
        if self.air_raw {
            return 0;
        }
        let started = qpc();
        self.captured.game_score.finish_air(outcome);
        self.attribute_overhead(TableId::AirWingGameScoreStats, started, 0)
    }
    fn air_owner(&mut self, slot: u32, entry: Option<usize>) -> u64 {
        if self.air_raw {
            return 0;
        }
        let _reads = crate::read_accounting::table(TableId::CountryGameScoreInputs);
        let begin = qpc();
        self.captured
            .game_score
            .owner(self.memory, self.base, slot, entry);
        self.attribute_overhead(TableId::CountryGameScoreInputs, begin, 0)
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
            && !matches!(outcome, Outcome::Unavailable { reason } if *reason == replay_data::cadence::SKIPPED_REASON)
    })
}

#[cfg(test)]
fn unit_sources(
    inputs: &Option<Arc<UnitInputs>>,
) -> production::Sources<'_, engine_native::carmy::DivisionSeed> {
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

pub(crate) struct Divisions {
    first: Vec<DivisionSource>,
    seen: usize,
    outcome: Outcome,
    required: usize,
}
impl Divisions {
    fn new(capacity: usize) -> Self {
        Self {
            first: owned_vec(filled(capacity, Default::default())),
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
    pub(crate) fn sources(&self) -> production::Sources<'_, DivisionSource> {
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

#[cfg(test)]
mod tests;

/// The naval live shape's hour, captured as production captures it.
#[cfg(test)]
pub(crate) fn capture_live_shape(captured: &mut Captured) {
    use naval_details::fixtures::{IMAGE, STATE, live_shape};
    let memory = live_shape();
    let c = &mut captured.naval_details;
    let mut capacity = naval_details::Capacities::default();
    capacity.raise_for_live_details(
        engine_naval_history::naval_combat_demand::demand(&memory, IMAGE, STATE).unwrap(),
    );
    c.try_grow_hint(&capacity).unwrap();
    c.capture(&memory, IMAGE, STATE);
}

#[cfg(test)]
pub(crate) fn output_allocation_failure_exercise(arm: impl FnOnce(), disarm: impl FnOnce()) {
    use replay_data::{rows::BuildingConstructionRow, rows::ProductionLineRow, rows::TableRows};
    use replay_frame::frame::TableView;
    let capacity = output_continuity::Capacities { entries: 4 };
    let mut construction = construction_ic::Captured::with_capacities(&capacity);
    let mut production = line_output::Captured::with_capacities(&capacity);
    let mut construction_totals = construction_ic::Totals::default();
    let mut production_totals = line_output::Totals::default();
    let mut tables = std::array::from_fn::<_, TABLE_COUNT, _>(|_| {
        TableInput::Sampled(TableView::Unavailable {
            reason: String::new(),
        })
    });
    tables[TableId::BuildingConstruction.index()] =
        TableInput::Sampled(TableView::Complete(TableRows::BuildingConstruction(vec![
            BuildingConstructionRow {
                id: 17,
                ..Default::default()
            },
        ])));
    tables[TableId::RailwayConstruction.index()] = TableInput::Sampled(TableView::Complete(
        TableRows::RailwayConstruction(Vec::new()),
    ));
    tables[TableId::ProductionLines.index()] =
        TableInput::Sampled(TableView::Complete(TableRows::ProductionLines(vec![
            ProductionLineRow {
                handle: 29,
                country: 5001,
                ..Default::default()
            },
        ])));
    arm();
    let scope = crate::allocation::enter(crate::allocation::Phase::Spool);
    for hour in 0..3 {
        let copied = usize::from(hour != 0);
        let drained = crate::output_log::Drained {
            epoch: 1,
            first: (hour - 1).max(0) as u64,
            cursor: hour as u64,
            copied,
            pending: copied,
        };
        let construction_entry = crate::construction_ic::Entry {
            line: 17,
            amount: 40,
            ..Default::default()
        };
        let production_entry = crate::line_output::Entry {
            line: 29,
            country: 5001,
            variant: hour as u64,
            units: 1,
            exact: true,
            ..Default::default()
        };
        construction.set_drained_for_test(
            Ok(drained),
            &std::slice::from_ref(&construction_entry)[..copied],
        );
        production.set_drained_for_test(
            Ok(drained),
            &std::slice::from_ref(&production_entry)[..copied],
        );
        construction_totals.assemble(1, hour, &construction, &mut tables);
        if crate::allocation::failed() {
            break;
        }
        production_totals.assemble(1, hour, &production, &mut tables);
        if crate::allocation::failed() {
            break;
        }
        construction_totals.written(true);
        production_totals.written(true);
    }
    drop(scope);
    disarm();
}
