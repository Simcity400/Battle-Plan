#[cfg(any(test, feature = "test-support"))]
use collector_output::output_continuity;

use collector_divisions::{division_census::UnitInputs, orders::Orders};
use collector_extended_support::outcomes::{MISSING, NOT_RECORDED, elapsed, records_any};
use collector_map_capture::map_capture::MapSources;
use collector_map_capture::map_capture::ShipCensus;
use collector_primitives::clock::qpc;
use collector_primitives::table_capture::grow_capacity;
use collector_primitives::table_capture::grow_vec;
use engine_allocation::{filled, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::cequipment_variant::pool::DivisionSource;
use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
use replay_data::scope::CaptureMode;
use replay_frame::frame::TableInput;
use std::mem::size_of;

use collector_intel::daily_intel;

mod border_inputs;
pub(crate) mod observer_boundary;
mod packs;
use packs::ORDINARY_TABLES;
pub(crate) use packs::{
    CharacterInputRoles, CharacterInputsRequest, FinalInputsReport, FinalInputsRequest, GatedPacks,
    LatePacks, NavalMissionLoan, NaviesBegin, ORDINARY_PARTS, OrdinaryPacks, OrdinaryPart,
    ProductionDailyPacks, RaidLaunchPacks, RaidStatePacks, RaidStateStores, RaidTargetPacks,
    ScoresRailsParts, SlowPacks, SlowStores, StabilityPacks, WarOverviewPacks,
};
pub(crate) use production::registry::Registry;

const FACTION_PROVIDER_TABLES: [TableId; 14] = {
    let mut tables = [TableId::LexicalTokens; 14];
    let mut next = 0;
    let mut index = 0;
    while index < collector_factions::faction_companions::TABLES.len() {
        tables[next] = collector_factions::faction_companions::TABLES[index];
        next += 1;
        index += 1;
    }
    index = 0;
    while index < collector_factions::faction_goals::TABLES.len() {
        tables[next] = collector_factions::faction_goals::TABLES[index];
        next += 1;
        index += 1;
    }
    assert!(next == tables.len());
    tables
};
macro_rules! spool_inputs {
    (combat_progress, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (combat_log, $captured:expr, $conversion:expr) => {
        $captured.table_inputs()
    };
    (group_counters, $captured:expr, $conversion:expr, $owner:expr, $hour:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (country_appearance, $captured:expr, $conversion:expr, $owner:expr, $hour:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut(), Some($hour))
    };
    (country_counters, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (construction, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (naval_details, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (naval_counters, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
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
        $captured.table_inputs(
            $conversion.characters.as_deref_mut(),
            $conversion.projection.as_deref_mut(),
        )
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
    (province_unit_order, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (intel, $captured:expr, $conversion:expr) => {
        $captured.table_inputs_projected(
            $conversion.variables_baseline,
            $conversion.variables_scratch,
            $conversion.intel.as_deref_mut(),
            $conversion.projection.as_deref_mut(),
        )
    };
    (governance, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
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
    (naval_history, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (plans, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (combat_details, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (war_overview, $captured:expr, $conversion:expr) => {
        $captured.table_inputs()
    };
    (air_records, $captured:expr, $conversion:expr) => {
        $captured.table_inputs()
    };
    (air_losses, $captured:expr, $conversion:expr) => {
        // SAFETY: Owning spool publication path converts this frame's air-loss events once.
        unsafe { $captured.table_inputs() }
    };
    (building_bombing, $captured:expr, $conversion:expr) => {
        // SAFETY: Owning spool publication path converts this frame's building-bombing events once.
        unsafe { $captured.table_inputs() }
    };
    (combat_engagement_refreshes, $captured:expr, $conversion:expr) => {
        // SAFETY: Owning spool publication path converts this frame's combat-engagement-refresh events once.
        unsafe { $captured.table_inputs() }
    };
    (nuclear_strikes, $captured:expr, $conversion:expr) => {
        // SAFETY: Owning spool publication path converts this frame's nuclear-strike events once.
        unsafe { $captured.table_inputs() }
    };
    (projects, $captured:expr, $conversion:expr) => {
        $captured.table_inputs($conversion.projection.as_deref_mut())
    };
    (naval_bases, $captured:expr, $conversion:expr) => {
        $captured.table_inputs()
    };
    ($pack:ident, $captured:expr, $conversion:expr) => {
        $captured.tables().map(TableInput::Sampled)
    };
}

macro_rules! spool_pack {
    ($all:expr, $pack:ident; $skip:expr, $destination:ident, $conversion:ident, $hour:ident, $slots:ident) => {
        spool_pack!(@convert $all, $pack; $destination, $conversion, $hour, $slots)
    };
    ($all:expr, $pack:ident, $marker:ident; $skip:expr, $destination:ident, $conversion:ident, $hour:ident, $slots:ident) => {
        if $skip {
            $slots.skip();
        } else {
            spool_pack!(@convert $all, $pack; $destination, $conversion, $hour, $slots)
        }
    };
    (@convert $all:expr, $pack:ident; $destination:ident, $conversion:ident, $hour:ident, $slots:ident) => {{
        collector_workers::tick_gate::wait();
        let started = std::time::Instant::now();
        if collector_memory::terminal::failed() { return; }
        for (id, view) in $pack::TABLES.into_iter().zip(spool_inputs!($pack, $all.$pack, $conversion, $all, $hour)) { $destination[id.index()] = view; }
        $slots.converted(started);
    }};
}

macro_rules! helper_pack {
    ($view:expr, $pack:ident; $($rest:tt)*) => {};
    ($view:expr, $pack:ident, helper; $conversion:ident, $lent:ident, $out:ident, $costs:ident, $index:ident, $hour:ident) => {
        helper_pack!(@convert $out, $costs, $index, $pack, spool_inputs!($pack, HelperView::sync_pack(&$view.0.$pack), $conversion))
    };
    ($view:expr, $pack:ident, helper_appearance; $conversion:ident, $lent:ident, $out:ident, $costs:ident, $index:ident, $hour:ident) => {
        helper_pack!(@convert $out, $costs, $index, $pack, HelperView::sync_pack(&$view.0.$pack)
            .table_inputs($conversion.projection.as_deref_mut(), Some($hour)))
    };
    ($view:expr, $pack:ident, helper_exclusive; $conversion:ident, $lent:ident, $out:ident, $costs:ident, $index:ident, $hour:ident) => {
        helper_pack!(@convert $out, $costs, $index, $pack, spool_inputs!($pack, &$view.0.$pack, $conversion, $view.0, $hour))
    };
    ($view:expr, $pack:ident, helper_cached; $conversion:ident, $lent:ident, $out:ident, $costs:ident, $index:ident, $hour:ident) => {{
        lend!($pack, $lent, $conversion);
        helper_pack!(@convert $out, $costs, $index, $pack, spool_inputs!($pack, &$view.0.$pack, $conversion))
    }};
    (@convert $out:ident, $costs:ident, $index:ident, $pack:ident, $inputs:expr) => {{
        collector_workers::tick_gate::wait();
        let started = std::time::Instant::now();
        if !collector_memory::terminal::failed() {
            for (id, view) in $pack::TABLES.into_iter().zip($inputs) {
                $out[id.index()] = Some(view);
            }
            $costs.converted($index, started);
        }
    }};
}

macro_rules! lend {
    (intel, $lent:ident, $conversion:ident) => {{
        let mut lent = $lent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        $conversion.intel = lent.intel.take();
        $conversion.variables_baseline = lent.variables_baseline;
        if let Some(scratch) = lent.variables_scratch.take() {
            $conversion.variables_scratch = scratch;
        }
    }};
    (characters, $lent:ident, $conversion:ident) => {{
        $conversion.characters = $lent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .characters
            .take();
    }};
    ($pack:ident, $lent:ident, $conversion:ident) => {};
}

macro_rules! helper_marked {
    () => {
        false
    };
    ($marker:ident) => {
        true
    };
}

pub(crate) struct HelperConversion<'a, 'cache> {
    pub projection: Option<&'a mut collector_projection::projection::Projection<'cache>>,
    pub characters: Option<&'a mut characters::CharacterConversion<'cache>>,
    pub intel: Option<&'a mut intel::IntelConversion<'cache>>,
    pub variables_baseline: intel::VariableBaselines<'a>,
    pub variables_scratch: &'a mut intel::ScriptVariableComparison,
}

#[derive(Default)]
pub struct LentCaches<'a, 'cache> {
    pub characters: Option<&'a mut characters::CharacterConversion<'cache>>,
    pub intel: Option<&'a mut intel::IntelConversion<'cache>>,
    pub variables_baseline: intel::VariableBaselines<'a>,
    pub variables_scratch: Option<&'a mut intel::ScriptVariableComparison>,
}

macro_rules! merge_pack_capacity {
    (saved_event_targets, $current:expr, $raised:expr) => {
        $current = $current.max($raised);
    };
    ($pack:ident, $current:expr, $raised:expr) => {
        collector_primitives::capacity::CapacityMax::merge_max(&mut $current, $raised);
    };
}

macro_rules! pack_frame {
    (intel, $capacity:expr) => {
        intel::Captured::without_home($capacity)
    };
    (division_templates, $capacity:expr) => {
        division_templates::Captured::without_home($capacity)
    };
    (naval_combat_sources, $capacity:expr) => {
        naval_combat_sources::Captured::without_home($capacity)
    };
    (naval_counters, $capacity:expr) => {
        // SAFETY: Allocates frame storage while fleet-region watches remain live and teardown is
        // excluded; `WatchRegistry::offer` atomically swaps offered tables.
        unsafe { naval_counters::Captured::with_capacities($capacity) }
    };
    ($pack:ident, $capacity:expr) => {
        $pack::Captured::with_capacities($capacity)
    };
}

macro_rules! grow_pack {
    (operation_outcomes, $pack:expr, $capacity:expr) => {
        // SAFETY: Caller holds exclusive `&mut` access to this frame while operation-log storage
        // stays live and teardown is excluded; shared log growth is synchronized by `LOGS` and
        // per-lane `byte_rings` locks across concurrent spool/sizing growth.
        unsafe { $pack.grow($capacity) }
    };
    (politics, $pack:expr, $capacity:expr) => {
        // SAFETY: Caller holds exclusive `&mut` access to this frame while politics staging stays
        // live and teardown is excluded; shared log growth is serialized by `LOGS` across
        // concurrent spool/sizing growth.
        unsafe { $pack.grow($capacity) }
    };
    (air_losses, $pack:expr, $capacity:expr) => {
        // SAFETY: Caller holds exclusive `&mut` access to this frame while air-loss observer
        // storage stays live and teardown is excluded; `EventLedger::prepare` synchronizes spare
        // and retired buffers across concurrent spool/sizing growth.
        unsafe { $pack.grow($capacity) }
    };
    (building_bombing, $pack:expr, $capacity:expr) => {
        // SAFETY: Caller holds exclusive `&mut` access to this frame while building-bombing
        // observer storage stays live and teardown is excluded; `EventLedger::prepare` synchronizes
        // spare and retired buffers across concurrent spool/sizing growth.
        unsafe { $pack.grow($capacity) }
    };
    (land_events, $pack:expr, $capacity:expr) => {
        // SAFETY: Caller holds exclusive `&mut` access to this frame while land-event storage stays
        // live and teardown is excluded; `land_events::prepare` serializes spare allocation under
        // `LOG_SWAP` across concurrent spool/sizing growth.
        unsafe { $pack.grow($capacity) }
    };
    ($name:ident, $pack:expr, $capacity:expr) => {
        $pack.grow($capacity)
    };
}

macro_rules! packs {
    ($($visibility:vis $pack:ident $(@ $helper:ident)?),+ $(,)?) => {
        pub const PACK_NAMES: &[&str] = &[$(stringify!($pack)),+];
        pub const HELPER_PACKS: [bool; PACK_NAMES.len()] =
            [$(helper_marked!($($helper)?)),+];
        pub(crate) const PACK_N: usize = 0 $(+ $pack::N)+;
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
        pub const PACK_TABLES: [TableId; PACK_N] = {
            let mut tables = [TableId::LexicalTokens; PACK_N];
            let mut index = 0;
            while index < PACK_N { tables[index] = TABLES[index]; index += 1; }
            tables
        };
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct Capacities {
            $($visibility $pack: $pack::Capacities,)+
            pub fuel_awards: fuel_awards::Capacities,
            pub division_sources: usize,
            pub variant_registry: usize,
        }
        impl collector_primitives::capacity::CapacityMax for Capacities {
            fn merge_max(&mut self, other: Self) {
                $(merge_pack_capacity!($pack, self.$pack, other.$pack);)+
                collector_primitives::capacity::CapacityMax::merge_max(&mut self.fuel_awards, other.fuel_awards);
                self.division_sources = self.division_sources.max(other.division_sources);
                self.variant_registry = self.variant_registry.max(other.variant_registry);
            }
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
            unit_inputs: Option<engine_allocation::SharedBox<UnitInputs>>,
            governance_report: Option<governance::CaptureReport>,
            variants_ticks: Option<[u64; production::N]>,
            presentation_report: Option<division_presentation::CaptureReport>,
            intel_residual_report: Option<intel::CaptureReport>,
            script_variables_ticks: Option<u64>,
            supply_trains_report: Option<supply_trains::CaptureReport>,
            character_portraits_report: Option<character_portraits::CaptureReport>,
            province_unit_order_report: Option<province_unit_order::CaptureReport>,
            ship_models_report: Option<ship_models::CaptureReport>,
            ordinary_ticks: Option<[u64; TABLE_COUNT]>,
            ordinary_failure: Option<&'static str>,
            ordinary_spare: Option<Box<OrdinaryPacks>>,
            ordinary_part_spares: [Option<OrdinaryPart>; ORDINARY_PARTS],
            slow_ticks: Option<[u64; TABLE_COUNT]>,
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
            air_supply_spare: Option<Box<engine_map_air::map_air_supply::Capture>>,
            missions_spare: Option<Box<air::Captured>>,
            gated_ticks: Option<[u64; TABLE_COUNT]>,
            gated_spare: Option<Box<GatedPacks>>,
            character_inputs_ticks: Option<[u64; character_inputs::N]>,
            registry: Registry,
            overhead: [u64; TABLE_COUNT],
            capture_mode: CaptureMode,
            parked: bool,
        }
pub(crate) type GovernanceStores = (
    Box<collector_certificates::slicing::SliceFingerprints>,
    Box<collector_certificates::slicing::SliceFingerprints>,
    Option<Box<construction::BuildingStore>>,
    Option<Box<governance::LeaderStore>>,
    Option<Box<governance::PartyStore>>,
);

pub struct ParkedPacks {
    division_presentation: division_presentation::Captured,
    production: production::Captured,
    production_modifiers: production_modifiers::Captured,
    characters: characters::Captured,
    projects: projects::Captured,
    economy: economy::Captured,
    governance: governance::Captured,
    division_templates: division_templates::Captured,
    intel: intel::Captured,
    air_histories: air_histories::Captured,
    ship_models: ship_models::Captured,
    fuel_awards: fuel_awards::Captured,
    character_portraits: character_portraits::Captured,
    naval_history: naval_history::Captured,
    scores_rails_spare: Option<Box<ScoresRailsParts>>,
    ordinary_spare: Option<Box<OrdinaryPacks>>,
    slow_spare: Option<Box<SlowPacks>>,
    late_spare: Option<Box<LatePacks>>,
    gated_spare: Option<Box<GatedPacks>>,
    supply_trains: supply_trains::Captured,
    visibility: visibility::Captured,
    industrial_organisations: industrial_organisations::Captured,
    weather: weather::Captured,
    logistics: logistics::Captured,
    war_overview: war_overview::Captured,
    plans: plans::Captured,
    air: air::Captured,
    new_divisions: new_divisions::Captured,
    event_feed: event_feed::Captured,
    country_appearance: country_appearance::Captured,
    hired_advisors: hired_advisors::Captured,
    peace_settlements: peace_settlements::Captured,
    peace_bids: peace_bids::Captured,
    province_unit_order: province_unit_order::Captured,
    construction: construction::Captured,
    country_counters: country_counters::Captured,
    character_inputs: character_inputs::Captured,
    fog: fog::Captured,
    model_inputs: model_inputs::Captured,
    country_stability: country_stability::Captured,
    country_factories: country_factories::Captured,
    state_building_inputs: state_building_inputs::Captured,
    naval_mission_inputs: naval_mission_inputs::Captured,
    raid_state_inputs: raid_state_inputs::Captured,
    raid_target_statuses: raid_target_statuses::Captured,
    raid_launch_inputs: raid_launch_inputs::Captured,
}

impl ParkedPacks {
    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.division_presentation.allocated_bytes()
            + self.production.allocated_bytes()
            + self.production_modifiers.allocated_bytes()
            + self.characters.allocated_bytes()
            + self.projects.allocated_bytes()
            + self.economy.allocated_bytes()
            + self.governance.allocated_bytes()
            + self.division_templates.allocated_bytes()
            + self.intel.allocated_bytes()
            + self.air_histories.allocated_bytes()
            + self.ship_models.allocated_bytes()
            + self.fuel_awards.allocated_bytes()
            + self.character_portraits.allocated_bytes()
            + self.naval_history.allocated_bytes()
            + self
                .scores_rails_spare
                .as_ref()
                .map_or(0, |spare| spare.allocated_bytes())
            + self
                .ordinary_spare
                .as_ref()
                .map_or(0, |spare| spare.allocated_bytes())
            + self
                .slow_spare
                .as_ref()
                .map_or(0, |spare| spare.allocated_bytes())
            + self
                .late_spare
                .as_ref()
                .map_or(0, |spare| spare.allocated_bytes())
            + self
                .gated_spare
                .as_ref()
                .map_or(0, |spare| spare.allocated_bytes())
            + self.supply_trains.allocated_bytes()
            + self.visibility.allocated_bytes()
            + self.industrial_organisations.allocated_bytes()
            + self.weather.allocated_bytes()
            + self.logistics.allocated_bytes()
            + self.war_overview.allocated_bytes()
            + self.plans.allocated_bytes()
            + self.air.allocated_bytes()
            + self.new_divisions.allocated_bytes()
            + self.event_feed.allocated_bytes()
            + self.country_appearance.allocated_bytes()
            + self.hired_advisors.allocated_bytes()
            + self.peace_settlements.allocated_bytes()
            + self.peace_bids.allocated_bytes()
            + self.province_unit_order.allocated_bytes()
            + self.construction.allocated_bytes()
            + self.country_counters.allocated_bytes()
            + self.character_inputs.allocated_bytes()
            + self.fog.allocated_bytes()
            + self.model_inputs.allocated_bytes()
            + self.country_stability.allocated_bytes()
            + self.country_factories.allocated_bytes()
            + self.state_building_inputs.allocated_bytes()
            + self.naval_mission_inputs.allocated_bytes()
            + self.raid_state_inputs.allocated_bytes()
            + self.raid_target_statuses.allocated_bytes()
            + self.raid_launch_inputs.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        engine_allocation::advisory(|| {
            let _allocation_phase =
                collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
            self.division_presentation
                .grow(&mut capacity.division_presentation);
            self.production.grow(&mut capacity.production);
            self.production_modifiers
                .grow(&mut capacity.production_modifiers);
            self.characters.grow(&mut capacity.characters);
            self.projects.grow(&mut capacity.projects);
            self.economy.grow(&mut capacity.economy);
            self.governance.grow(&mut capacity.governance);
            self.division_templates
                .grow(&mut capacity.division_templates);
            self.intel.grow(&mut capacity.intel);
            self.air_histories.grow(&mut capacity.air_histories);
            self.ship_models.grow(&mut capacity.ship_models);
            self.fuel_awards.grow(&mut capacity.fuel_awards);
            self.character_portraits
                .grow(&mut capacity.character_portraits);
            self.naval_history.grow(&mut capacity.naval_history);
            self.supply_trains.grow(&mut capacity.supply_trains);
            self.visibility.grow(&mut capacity.visibility);
            self.industrial_organisations
                .grow(&mut capacity.industrial_organisations);
            self.weather.grow(&mut capacity.weather);
            self.logistics.grow(&mut capacity.logistics);
            self.war_overview.grow(&mut capacity.war_overview);
            self.plans.grow(&mut capacity.plans);
            self.air.grow(&mut capacity.air);
            self.new_divisions.grow(&mut capacity.new_divisions);
            self.event_feed.grow(&mut capacity.event_feed);
            self.country_appearance
                .grow(&mut capacity.country_appearance);
            self.hired_advisors.grow(&mut capacity.hired_advisors);
            self.peace_settlements.grow(&mut capacity.peace_settlements);
            self.peace_bids.grow(&mut capacity.peace_bids);
            self.province_unit_order
                .grow(&mut capacity.province_unit_order);
            self.construction.grow(&mut capacity.construction);
            self.country_counters.grow(&mut capacity.country_counters);
            self.character_inputs.grow(&mut capacity.character_inputs);
            self.fog.grow(&mut capacity.fog);
            self.model_inputs.grow(&mut capacity.model_inputs);
            self.country_stability.grow(&mut capacity.country_stability);
            self.country_factories.grow(&mut capacity.country_factories);
            self.state_building_inputs
                .grow(&mut capacity.state_building_inputs);
            self.naval_mission_inputs
                .grow(&mut capacity.naval_mission_inputs);
            self.raid_state_inputs.grow(&mut capacity.raid_state_inputs);
            self.raid_target_statuses
                .grow(&mut capacity.raid_target_statuses);
            self.raid_launch_inputs
                .grow(&mut capacity.raid_launch_inputs);
            self.division_presentation
                .reserve_sources(capacity.division_sources);
            self.plans.reserve_units(capacity.division_sources);
            self.division_presentation.grow_name_groups(
                &mut capacity.division_presentation,
                self.division_templates.naming_demand(),
            );
        });
    }
}

impl Captured {
            #[cfg(any(test, feature = "test-support"))]
            pub fn set_intel_shadow_fixture(&mut self, value: i64) {
                self.intel = intel::intel_shadow_fixture(value);
            }

            #[cfg(test)]
            pub fn with_capacities(capacity: &Capacities) -> Self {
                let mut captured =
                // SAFETY: This frame or slot belongs to the current admitted capture or the sole
                // spool consuming it.
                unsafe { Self::frame(capacity) };
                captured.share_homes(&Homes::new(capacity).expect("test homes"));
                captured
            }

            /// # Safety
            /// Only the session's attach, frame-building, sizing or admitted capture sequence may
            /// prepare this frame. Keep observer storage live and exclude conflicting observer
            /// preparation and teardown.
            pub unsafe fn frame(capacity: &Capacities) -> Self {
                let mut captured = Self {
                    $($pack: pack_frame!($pack, &capacity.$pack),)+
                    fuel_awards: fuel_awards::Captured::with_capacities(&capacity.fuel_awards),
                    divisions: Divisions::new(capacity.division_sources),
                    final_inputs_ticks: None,
                    unit_inputs: None,
                    governance_report: None,
                    variants_ticks: None,
                    presentation_report: None,
                    intel_residual_report: None,
                    script_variables_ticks: None,
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
                    air_supply_spare: engine_allocation::boxed(Default::default()).ok(),
                    missions_spare: engine_allocation::boxed(air::Captured::vacant()).ok(),
                    gated_ticks: None,
                    gated_spare: GatedPacks::spare(),
                    character_inputs_ticks: None,
                    registry: Registry::new(capacity.variant_registry),
                    overhead: [0; TABLE_COUNT],
                    capture_mode: CaptureMode::Full,
                    parked: false,
                };
                captured.division_presentation.reserve_sources(capacity.division_sources);
                captured.plans.reserve_units(capacity.division_sources);
                captured
            }
            /// # Safety
            /// Only the session's attach, frame-building, sizing or admitted capture sequence may
            /// prepare this frame. Keep observer storage live and exclude conflicting observer
            /// preparation and teardown.
            pub unsafe fn frame_parked(capacity: &Capacities) -> Self {
                let mut zero = *capacity;
                zero.division_presentation = Default::default();
                zero.production = Default::default();
                zero.production_modifiers = Default::default();
                zero.characters = Default::default();
                zero.projects = Default::default();
                zero.economy = Default::default();
                zero.governance = Default::default();
                zero.division_templates = Default::default();
                zero.intel = Default::default();
                zero.air_histories = Default::default();
                zero.ship_models = Default::default();
                zero.fuel_awards = Default::default();
                zero.character_portraits = Default::default();
                zero.naval_history = Default::default();
                zero.supply_trains = Default::default();
                zero.visibility = Default::default();
                zero.industrial_organisations = Default::default();
                zero.weather = Default::default();
                zero.logistics = Default::default();
                zero.war_overview = Default::default();
                zero.plans = Default::default();
                zero.air = Default::default();
                zero.new_divisions = Default::default();
                zero.event_feed = Default::default();
                zero.country_appearance = Default::default();
                zero.hired_advisors = Default::default();
                zero.peace_settlements = Default::default();
                zero.peace_bids = Default::default();
                zero.province_unit_order = Default::default();
                zero.construction = Default::default();
                zero.country_counters = Default::default();
                zero.character_inputs = Default::default();
                zero.fog = Default::default();
                zero.model_inputs = Default::default();
                zero.country_stability = Default::default();
                zero.country_factories = Default::default();
                zero.state_building_inputs = Default::default();
                zero.naval_mission_inputs = Default::default();
                zero.raid_state_inputs = Default::default();
                zero.raid_target_statuses = Default::default();
                zero.raid_launch_inputs = Default::default();
                zero.division_sources = 0;
                // SAFETY: Caller guarantees live observer storage and exclusive frame preparation.
                let mut captured = unsafe { Self::frame(&zero) };
                captured.divisions = Divisions::new(capacity.division_sources);
                captured.division_presentation = division_presentation::Captured::vacant();
                captured.production = production::Captured::vacant();
                captured.production_modifiers = production_modifiers::Captured::vacant();
                captured.characters = characters::Captured::vacant();
                captured.projects = projects::Captured::vacant();
                captured.economy = economy::Captured::vacant();
                captured.governance = governance::Captured::vacant();
                captured.division_templates = division_templates::Captured::vacant();
                captured.intel = intel::Captured::vacant();
                captured.ship_models = ship_models::Captured::vacant();
                captured.fuel_awards = fuel_awards::Captured::vacant();
                captured.character_portraits = character_portraits::Captured::vacant();
                captured.naval_history = naval_history::Captured::vacant();
                captured.scores_rails_spare = None;
                captured.ordinary_spare = None;
                captured.slow_spare = None;
                captured.late_spare = None;
                captured.gated_spare = None;
                captured.supply_trains = supply_trains::Captured::vacant();
                captured.visibility = visibility::Captured::vacant();
                captured.industrial_organisations = industrial_organisations::Captured::vacant();
                captured.weather = weather::Captured::vacant();
                captured.logistics = logistics::Captured::vacant();
                captured.war_overview = war_overview::Captured::vacant();
                captured.plans = plans::Captured::vacant();
                captured.air = air::Captured::vacant();
                captured.new_divisions = new_divisions::Captured::vacant();
                captured.event_feed = event_feed::Captured::vacant();
                captured.country_appearance = country_appearance::Captured::vacant();
                captured.hired_advisors = hired_advisors::Captured::vacant();
                captured.peace_settlements = peace_settlements::Captured::vacant();
                captured.peace_bids = peace_bids::Captured::vacant();
                captured.province_unit_order = province_unit_order::Captured::vacant();
                captured.construction = construction::Captured::vacant();
                captured.country_counters = country_counters::Captured::vacant();
                captured.character_inputs = character_inputs::Captured::vacant();
                captured.fog = fog::Captured::vacant();
                captured.model_inputs = model_inputs::Captured::vacant();
                captured.country_stability = country_stability::Captured::vacant();
                captured.country_factories = country_factories::Captured::vacant();
                captured.state_building_inputs = state_building_inputs::Captured::vacant();
                captured.naval_mission_inputs = naval_mission_inputs::Captured::vacant();
                captured.raid_state_inputs = raid_state_inputs::Captured::vacant();
                captured.raid_target_statuses = raid_target_statuses::Captured::vacant();
                captured.raid_launch_inputs = raid_launch_inputs::Captured::vacant();
                captured.parked = true;
                captured
            }
            pub fn reset(&mut self) {
                $(self.$pack.reset();)+
                self.fuel_awards.reset();
                self.divisions.reset(); self.registry.reset(); self.final_inputs_ticks = None;
                self.release_unit_inputs(); self.governance_report = None; self.presentation_report = None;
                self.variants_ticks = None;
                self.intel_residual_report = None;
                self.script_variables_ticks = None;
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
                self.overhead = [0; TABLE_COUNT];
            }
            pub fn release_transient(&mut self) {
                self.air_histories.release_transient();
                self.economy.release_daily();
                self.governance.release_transient();
                self.industrial_organisations.release_daily();
                self.intel.release_daily_idle();
                self.lexical.release_static();
                self.logistics.release_daily();
                self.production.release_daily();
                self.production_modifiers.release_daily();
            }
            pub fn is_parked(&self) -> bool {
                self.parked
            }
            pub fn take_parked_packs(&mut self) -> Option<Box<ParkedPacks>> {
                if self.parked {
                    return None;
                }
                let mut slot =
                    engine_allocation::advisory(engine_allocation::boxed_uninit::<ParkedPacks>)
                        .ok()?;
                let ptr = slot.as_mut_ptr();
                macro_rules! move_pack {
                    ($field:ident, $vacant:expr) => {
                        // SAFETY: `ptr` points to uniquely owned writable `ParkedPacks` storage,
                        // and each field is written exactly once before `assume_init`.
                        unsafe {
                            std::ptr::addr_of_mut!((*ptr).$field)
                                .write(std::mem::replace(&mut self.$field, $vacant));
                        }
                    };
                }
                macro_rules! move_opt {
                    ($field:ident) => {
                        // SAFETY: `ptr` points to uniquely owned writable `ParkedPacks` storage,
                        // and each field is written exactly once before `assume_init`.
                        unsafe {
                            std::ptr::addr_of_mut!((*ptr).$field).write(self.$field.take());
                        }
                    };
                }
                move_pack!(division_presentation, division_presentation::Captured::vacant());
                move_pack!(production, production::Captured::vacant());
                move_pack!(production_modifiers, production_modifiers::Captured::vacant());
                move_pack!(characters, characters::Captured::vacant());
                move_pack!(projects, projects::Captured::vacant());
                move_pack!(economy, economy::Captured::vacant());
                move_pack!(governance, governance::Captured::vacant());
                move_pack!(division_templates, division_templates::Captured::vacant());
                move_pack!(intel, intel::Captured::vacant());
                move_pack!(air_histories, air_histories::Captured::vacant());
                move_pack!(ship_models, ship_models::Captured::vacant());
                move_pack!(fuel_awards, fuel_awards::Captured::vacant());
                move_pack!(character_portraits, character_portraits::Captured::vacant());
                move_pack!(naval_history, naval_history::Captured::vacant());
                move_opt!(scores_rails_spare);
                move_opt!(ordinary_spare);
                move_opt!(slow_spare);
                move_opt!(late_spare);
                move_opt!(gated_spare);
                move_pack!(supply_trains, supply_trains::Captured::vacant());
                move_pack!(visibility, visibility::Captured::vacant());
                move_pack!(industrial_organisations, industrial_organisations::Captured::vacant());
                move_pack!(weather, weather::Captured::vacant());
                move_pack!(logistics, logistics::Captured::vacant());
                move_pack!(war_overview, war_overview::Captured::vacant());
                move_pack!(plans, plans::Captured::vacant());
                move_pack!(air, air::Captured::vacant());
                move_pack!(new_divisions, new_divisions::Captured::vacant());
                move_pack!(event_feed, event_feed::Captured::vacant());
                move_pack!(country_appearance, country_appearance::Captured::vacant());
                move_pack!(hired_advisors, hired_advisors::Captured::vacant());
                move_pack!(peace_settlements, peace_settlements::Captured::vacant());
                move_pack!(peace_bids, peace_bids::Captured::vacant());
                move_pack!(province_unit_order, province_unit_order::Captured::vacant());
                move_pack!(construction, construction::Captured::vacant());
                move_pack!(country_counters, country_counters::Captured::vacant());
                move_pack!(character_inputs, character_inputs::Captured::vacant());
                move_pack!(fog, fog::Captured::vacant());
                move_pack!(model_inputs, model_inputs::Captured::vacant());
                move_pack!(country_stability, country_stability::Captured::vacant());
                move_pack!(country_factories, country_factories::Captured::vacant());
                move_pack!(state_building_inputs, state_building_inputs::Captured::vacant());
                move_pack!(naval_mission_inputs, naval_mission_inputs::Captured::vacant());
                move_pack!(raid_state_inputs, raid_state_inputs::Captured::vacant());
                move_pack!(raid_target_statuses, raid_target_statuses::Captured::vacant());
                move_pack!(raid_launch_inputs, raid_launch_inputs::Captured::vacant());
                self.parked = true;
                // SAFETY: every field of `ParkedPacks` was initialized above, and
                // `MaybeUninit<ParkedPacks>` has the exact same layout as `ParkedPacks`.
                Some(unsafe { Box::from_raw(Box::into_raw(slot).cast::<ParkedPacks>()) })
            }
            pub fn discard_parked_packs(&mut self) -> bool {
                if self.parked {
                    return false;
                }
                self.division_presentation = division_presentation::Captured::vacant();
                self.production = production::Captured::vacant();
                self.production_modifiers = production_modifiers::Captured::vacant();
                self.characters = characters::Captured::vacant();
                self.projects = projects::Captured::vacant();
                self.economy = economy::Captured::vacant();
                self.governance = governance::Captured::vacant();
                self.division_templates = division_templates::Captured::vacant();
                self.intel = intel::Captured::vacant();
                self.air_histories = air_histories::Captured::vacant();
                self.ship_models = ship_models::Captured::vacant();
                self.fuel_awards = fuel_awards::Captured::vacant();
                self.character_portraits = character_portraits::Captured::vacant();
                self.naval_history = naval_history::Captured::vacant();
                self.scores_rails_spare = None;
                self.ordinary_spare = None;
                self.slow_spare = None;
                self.late_spare = None;
                self.gated_spare = None;
                self.supply_trains = supply_trains::Captured::vacant();
                self.visibility = visibility::Captured::vacant();
                self.industrial_organisations = industrial_organisations::Captured::vacant();
                self.weather = weather::Captured::vacant();
                self.logistics = logistics::Captured::vacant();
                self.war_overview = war_overview::Captured::vacant();
                self.plans = plans::Captured::vacant();
                self.air = air::Captured::vacant();
                self.new_divisions = new_divisions::Captured::vacant();
                self.event_feed = event_feed::Captured::vacant();
                self.country_appearance = country_appearance::Captured::vacant();
                self.hired_advisors = hired_advisors::Captured::vacant();
                self.peace_settlements = peace_settlements::Captured::vacant();
                self.peace_bids = peace_bids::Captured::vacant();
                self.province_unit_order = province_unit_order::Captured::vacant();
                self.construction = construction::Captured::vacant();
                self.country_counters = country_counters::Captured::vacant();
                self.character_inputs = character_inputs::Captured::vacant();
                self.fog = fog::Captured::vacant();
                self.model_inputs = model_inputs::Captured::vacant();
                self.country_stability = country_stability::Captured::vacant();
                self.country_factories = country_factories::Captured::vacant();
                self.state_building_inputs = state_building_inputs::Captured::vacant();
                self.naval_mission_inputs = naval_mission_inputs::Captured::vacant();
                self.raid_state_inputs = raid_state_inputs::Captured::vacant();
                self.raid_target_statuses = raid_target_statuses::Captured::vacant();
                self.raid_launch_inputs = raid_launch_inputs::Captured::vacant();
                self.parked = true;
                true
            }
            pub fn install_parked_packs(&mut self, mut parked: Box<ParkedPacks>) {
                std::mem::swap(&mut self.division_presentation, &mut parked.division_presentation);
                std::mem::swap(&mut self.production, &mut parked.production);
                std::mem::swap(&mut self.production_modifiers, &mut parked.production_modifiers);
                std::mem::swap(&mut self.characters, &mut parked.characters);
                std::mem::swap(&mut self.projects, &mut parked.projects);
                std::mem::swap(&mut self.economy, &mut parked.economy);
                std::mem::swap(&mut self.governance, &mut parked.governance);
                std::mem::swap(&mut self.division_templates, &mut parked.division_templates);
                std::mem::swap(&mut self.intel, &mut parked.intel);
                std::mem::swap(&mut self.air_histories, &mut parked.air_histories);
                std::mem::swap(&mut self.ship_models, &mut parked.ship_models);
                std::mem::swap(&mut self.fuel_awards, &mut parked.fuel_awards);
                std::mem::swap(&mut self.character_portraits, &mut parked.character_portraits);
                std::mem::swap(&mut self.naval_history, &mut parked.naval_history);
                self.scores_rails_spare = parked
                    .scores_rails_spare
                    .take()
                    .or_else(ScoresRailsParts::spare);
                self.ordinary_spare = parked.ordinary_spare.take().or_else(OrdinaryPacks::spare);
                self.slow_spare = parked.slow_spare.take().or_else(SlowPacks::spare);
                self.late_spare = parked.late_spare.take().or_else(LatePacks::spare);
                self.gated_spare = parked.gated_spare.take().or_else(GatedPacks::spare);
                std::mem::swap(&mut self.supply_trains, &mut parked.supply_trains);
                std::mem::swap(&mut self.visibility, &mut parked.visibility);
                std::mem::swap(
                    &mut self.industrial_organisations,
                    &mut parked.industrial_organisations,
                );
                std::mem::swap(&mut self.weather, &mut parked.weather);
                std::mem::swap(&mut self.logistics, &mut parked.logistics);
                std::mem::swap(&mut self.war_overview, &mut parked.war_overview);
                std::mem::swap(&mut self.plans, &mut parked.plans);
                std::mem::swap(&mut self.air, &mut parked.air);
                std::mem::swap(&mut self.new_divisions, &mut parked.new_divisions);
                std::mem::swap(&mut self.event_feed, &mut parked.event_feed);
                std::mem::swap(&mut self.country_appearance, &mut parked.country_appearance);
                std::mem::swap(&mut self.hired_advisors, &mut parked.hired_advisors);
                std::mem::swap(&mut self.peace_settlements, &mut parked.peace_settlements);
                std::mem::swap(&mut self.peace_bids, &mut parked.peace_bids);
                std::mem::swap(&mut self.province_unit_order, &mut parked.province_unit_order);
                std::mem::swap(&mut self.construction, &mut parked.construction);
                std::mem::swap(&mut self.country_counters, &mut parked.country_counters);
                std::mem::swap(&mut self.character_inputs, &mut parked.character_inputs);
                std::mem::swap(&mut self.fog, &mut parked.fog);
                std::mem::swap(&mut self.model_inputs, &mut parked.model_inputs);
                std::mem::swap(&mut self.country_stability, &mut parked.country_stability);
                std::mem::swap(&mut self.country_factories, &mut parked.country_factories);
                std::mem::swap(
                    &mut self.state_building_inputs,
                    &mut parked.state_building_inputs,
                );
                std::mem::swap(&mut self.naval_mission_inputs, &mut parked.naval_mission_inputs);
                std::mem::swap(&mut self.raid_state_inputs, &mut parked.raid_state_inputs);
                std::mem::swap(&mut self.raid_target_statuses, &mut parked.raid_target_statuses);
                std::mem::swap(&mut self.raid_launch_inputs, &mut parked.raid_launch_inputs);
                self.parked = false;
            }
            pub fn hydrate_parked_packs(&mut self, capacity: &Capacities) {
                self.division_presentation =
                    division_presentation::Captured::with_capacities(&capacity.division_presentation);
                self.production =
                    production::Captured::with_capacities_daily_released(&capacity.production);
                self.production_modifiers = production_modifiers::Captured::vacant();
                self.characters = characters::Captured::with_capacities(&capacity.characters);
                self.projects = projects::Captured::with_capacities(&capacity.projects);
                self.economy = economy::Captured::with_capacities(&capacity.economy);
                self.governance = governance::Captured::with_capacities(&capacity.governance);
                self.division_templates =
                    division_templates::Captured::without_home(&capacity.division_templates);
                self.intel = intel::Captured::without_home(&capacity.intel);
                self.air_histories =
                    air_histories::Captured::with_capacities(&capacity.air_histories);
                self.ship_models = ship_models::Captured::with_capacities(&capacity.ship_models);
                self.fuel_awards = fuel_awards::Captured::with_capacities(&capacity.fuel_awards);
                self.character_portraits =
                    character_portraits::Captured::with_capacities(&capacity.character_portraits);
                self.naval_history =
                    naval_history::Captured::with_capacities(&capacity.naval_history);
                if self.scores_rails_spare.is_none() {
                    self.scores_rails_spare = ScoresRailsParts::spare();
                }
                if self.ordinary_spare.is_none() {
                    self.ordinary_spare = OrdinaryPacks::spare();
                }
                if self.slow_spare.is_none() {
                    self.slow_spare = SlowPacks::spare();
                }
                if self.late_spare.is_none() {
                    self.late_spare = LatePacks::spare();
                }
                if self.gated_spare.is_none() {
                    self.gated_spare = GatedPacks::spare();
                }
                self.supply_trains =
                    supply_trains::Captured::with_capacities(&capacity.supply_trains);
                self.visibility = visibility::Captured::with_capacities(&capacity.visibility);
                self.industrial_organisations = industrial_organisations::Captured::with_capacities(
                    &capacity.industrial_organisations,
                );
                self.weather = weather::Captured::with_capacities(&capacity.weather);
                self.logistics = logistics::Captured::with_capacities(&capacity.logistics);
                self.war_overview = war_overview::Captured::with_capacities(&capacity.war_overview);
                self.plans = plans::Captured::with_capacities(&capacity.plans);
                self.air = air::Captured::with_capacities(&capacity.air);
                self.new_divisions =
                    new_divisions::Captured::with_capacities(&capacity.new_divisions);
                self.event_feed = event_feed::Captured::with_capacities(&capacity.event_feed);
                self.country_appearance =
                    country_appearance::Captured::with_capacities(&capacity.country_appearance);
                self.hired_advisors =
                    hired_advisors::Captured::with_capacities(&capacity.hired_advisors);
                self.peace_settlements =
                    peace_settlements::Captured::with_capacities(&capacity.peace_settlements);
                self.peace_bids = peace_bids::Captured::with_capacities(&capacity.peace_bids);
                self.province_unit_order =
                    province_unit_order::Captured::with_capacities(&capacity.province_unit_order);
                self.construction =
                    construction::Captured::with_capacities(&capacity.construction);
                self.country_counters =
                    country_counters::Captured::with_capacities(&capacity.country_counters);
                self.character_inputs =
                    character_inputs::Captured::with_capacities(&capacity.character_inputs);
                self.fog = fog::Captured::with_capacities(&capacity.fog);
                self.model_inputs =
                    model_inputs::Captured::with_capacities(&capacity.model_inputs);
                self.country_stability =
                    country_stability::Captured::with_capacities(&capacity.country_stability);
                self.country_factories =
                    country_factories::Captured::with_capacities(&capacity.country_factories);
                self.state_building_inputs = state_building_inputs::Captured::with_capacities(
                    &capacity.state_building_inputs,
                );
                self.naval_mission_inputs =
                    naval_mission_inputs::Captured::with_capacities(&capacity.naval_mission_inputs);
                self.raid_state_inputs =
                    raid_state_inputs::Captured::with_capacities(&capacity.raid_state_inputs);
                self.raid_target_statuses = raid_target_statuses::Captured::with_capacities(
                    &capacity.raid_target_statuses,
                );
                self.raid_launch_inputs =
                    raid_launch_inputs::Captured::with_capacities(&capacity.raid_launch_inputs);
                self.division_presentation
                    .reserve_sources(capacity.division_sources);
                self.plans.reserve_units(capacity.division_sources);
                self.release_transient();
                self.parked = false;
            }
            pub fn allocated_bytes(&self) -> usize {
                0 $(+ self.$pack.allocated_bytes())+ + self.fuel_awards.allocated_bytes() + self.divisions.allocated_bytes() + self.registry.allocated_bytes() + self.spare_bytes()
            }
            #[cfg(test)]
            pub fn allocated_by_pack(&self) -> std::io::Result<Vec<(&'static str, usize)>> {
                engine_allocation::copy_vec(&[
                    $((stringify!($pack), self.$pack.allocated_bytes()),)+
                    ("fuel_awards", self.fuel_awards.allocated_bytes()),
                    ("division_sources", self.divisions.allocated_bytes()),
                    ("variant_registry", self.registry.allocated_bytes()),
                    ("acquisition_spares", self.spare_bytes()),
                ])
            }
            /// # Safety
            /// Caller holds exclusive `&mut` access to this frame while observer storage stays
            /// live for the active session (or an isolated test) and teardown/`release` is
            /// excluded. Concurrent frame growth across spool and sizing threads relies on each
            /// observer's internal synchronization.
            pub unsafe fn grow(&mut self, capacity: &mut Capacities) {
                engine_allocation::advisory(|| self.grow_hints(capacity));
            }
            fn grow_hints(&mut self, capacity: &mut Capacities) {
                let _allocation_phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
                if self.parked {
                    self.divisions.grow(&mut capacity.division_sources);
                    self.registry.grow(&mut capacity.variant_registry);
                    return;
                }
                $(grow_pack!($pack, self.$pack, &mut capacity.$pack);)+
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
            /// # Safety
            /// Convert only within the frame's single spool round. If a helper round is active,
            /// skip every pack assigned to a helper and finish reads of those packs only after all
            /// helpers join.
            pub unsafe fn insert_tables(
                &self,
                hour: i32,
                destination: &mut [TableInput; TABLE_COUNT],
                conversion: &mut crate::pack_conversion::SpoolConversion<'_, '_>,
                helper_converts: bool,
                costs: &mut crate::pack_conversion::HourCosts,
            ) {
                let _allocation_phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
                let clock = collector_projection::table_clock::arm();
                let mut slots = costs.packs();
                $(spool_pack!(self, $pack $(, $helper)?; helper_converts, destination, conversion, hour, slots);)+
                drop(clock);
                if !helper_converts {
                    self.refuse_failed_ordinary(destination);
                }
            }
            pub fn refuse_failed_ordinary(&self, destination: &mut [TableInput; TABLE_COUNT]) {
                if collector_memory::terminal::failed() { return; }
                if let Some(reason) = self.ordinary_failure {
                    for &id in ORDINARY_TABLES.iter().flat_map(|tables| tables.iter()) {
                        destination[id.index()] = TableInput::Sampled(replay_frame::frame::TableView::Unavailable { reason: reason.into() });
                    }
                }
            }
            /// # Safety
            /// The spool must join every helper before reusing the frame. Use one queue for this
            /// frame and exclude all other access to each non-Sync pack while its helper converts
            /// it.
            pub unsafe fn helper_view(&self) -> HelperView<'_> {
                HelperView(self)
            }
        }
        pub struct HelperView<'a>(&'a Captured);
        // SAFETY: threads convert through this view only while the spool
        // thread is inside `Helpers::join_some`, which orders each helper's
        // accesses with the spool thread's through its mutex. `@helper` and
        // `@helper_appearance` packs are reached through `sync_pack`, whose bound proves them
        // `Sync`, so any threads may read them at once. The `@helper_cached`
        // packs (characters, intel, character portraits) and the
        // `@helper_exclusive` pack (division presentation) need not be `Sync`:
        // the hour's queue (`pack_conversion::Queue`) hands each to exactly
        // one thread, the spool thread's own packs skip them while the queue
        // runs, and no other pack reads them. The exclusive pack's observers
        // (`observe_*`) run before conversion, and outcomes, slice selections
        // and merging after the join. The thread that takes one of these
        // packs therefore has exclusive use, as if it held `&mut`. No other
        // pack is reachable through the view.
        unsafe impl Sync for HelperView<'_> {}
        unsafe impl Send for HelperView<'_> {}
        impl<'a> HelperView<'a> {
            fn sync_pack<T: Sync>(pack: &'a T) -> &'a T {
                pack
            }
            /// # Safety
            /// Use the spool's sole queue for this frame within its joined conversion round. Each
            /// non-Sync pack must be accessed by only its assigned converter until the round joins.
            pub unsafe fn convert_queued<'cache>(
                &self,
                queue: &crate::pack_conversion::Queue<'_>,
                hour: i32,
                mut projection: Option<&mut collector_projection::projection::Projection<'cache>>,
                lent: &std::sync::Mutex<LentCaches<'_, 'cache>>,
                out: &mut [Option<TableInput>; TABLE_COUNT],
                costs: &mut crate::pack_conversion::HourCosts,
            ) {
                let _allocation_phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
                let _clock = collector_projection::table_clock::arm();
                let mut spare = intel::ScriptVariableComparison::default();
                while let Some(index) = queue.take() {
                    let conversion = &mut HelperConversion {
                        projection: projection.as_deref_mut(),
                        characters: None,
                        intel: None,
                        variables_baseline: intel::VariableBaselines::default(),
                        variables_scratch: &mut spare,
                    };
                    let mut pack = 0;
                    $(
                        if pack == index {
                            helper_pack!(self, $pack $(, $helper)?; conversion, lent, out, costs, index, hour);
                        }
                        pack += 1;
                    )+
                    let _ = pack;
                }
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
    };
}

use collector_advisors::hired_advisors;
use collector_air::extended::air;
use collector_air::extended::fog;
use collector_air::extended::game_score;
use collector_air_histories::air_histories;
use collector_air_records::air_records;
use collector_air_relations::air_relations;
use collector_army_icons::army_icon_inputs;
use collector_characters::extended::active_abilities;
use collector_characters::extended::character_inputs;
use collector_characters::extended::character_portraits;
use collector_characters::extended::characters;
use collector_combat::combat_details;
use collector_combat_log::combat_log;
use collector_construction::construction;
use collector_country_counters::country_appearance;
use collector_country_counters::country_counters;
use collector_country_counters::reserved_visibility;
use collector_division_templates::extended::division_templates;
use collector_division_templates::extended::group_counters;
use collector_divisions::extended::division_counts;
use collector_divisions::extended::division_model_inputs;
use collector_divisions::extended::division_presentation;
use collector_economy::economy;
use collector_equipment_destruction::extended::equipment_destruction;
use collector_experience_gains::tables as experience_gains;
use collector_fuel_awards::tables as fuel_awards;
use collector_governance::governance;
use collector_intel::intel;
use collector_land_events::extended::combat_progress;
use collector_land_events::extended::land_events;
use collector_leaders::leader_paths;
use collector_leaders::leader_portraits;
use collector_ledger_tables::air_losses;
use collector_ledger_tables::building_bombing;
use collector_ledger_tables::combat_engagement_refreshes;
use collector_ledger_tables::event_feed;
use collector_ledger_tables::market;
use collector_ledger_tables::nuclear_strikes;
use collector_lexical::lexical;
use collector_map_tables::battle_sounds;
use collector_map_tables::map_facilities;
use collector_map_tables::naval_bases;
use collector_model_inputs::model_inputs;
use collector_naval_counters::naval_counters;
use collector_naval_details::naval_combat_sources;
use collector_naval_details::naval_details;
use collector_naval_details::naval_result_details;
use collector_naval_history::extended::naval_history;
use collector_naval_missions::extended::naval_mission_inputs;
use collector_naval_movement::country_naval_movement_inputs;
use collector_naval_results::extended::naval_result_sources;
use collector_new_divisions::new_divisions;
use collector_output::construction_tables as construction_ic;
use collector_output::line_tables as line_output;
use collector_production_daily::industrial_organisations;
use collector_session::map_names;
use collector_slow_tables::country_factories;
use collector_slow_tables::country_tactic_unlocks;
use collector_slow_tables::faction_theatres;
use collector_slow_tables::logistics;
use collector_slow_tables::modifier_definitions;
use collector_slow_tables::state_building_inputs;
use collector_stability::country_stability;
pub(crate) use country_naval_movement_inputs::Capacities as PhysicalNavyCapacities;
pub mod map_events;
use collector_divisions::extended::province_unit_order;
use collector_map_tables::saved_event_targets;
use collector_operation_outcomes::extended::operation_outcomes;
use collector_peace::peace_bids;
use collector_peace::tables as peace_settlements;
use collector_plans::extended::plans;
use collector_politics::politics;
use collector_production::production;
use collector_production_daily::production_modifiers;
use collector_projects::projects;
use collector_projects::raid_target_statuses;
use collector_raids::raid_launch_inputs;
pub(crate) use collector_raids::raid_state_inputs;
pub(crate) use collector_raids::resource_map;
use collector_railway_guns::railway_guns;
use collector_session::session;
use collector_ship_models::ship_models;
pub mod slice_reads;
use collector_states::extended::state_flags;
use collector_supply_trains::supply_trains;
use collector_visibility::visibility;
use collector_war_overview::war_overview;
use collector_weather::weather;

packs!(
    pub air @helper,
    pub air_relations,
    pub air_histories,
    pub(crate) army_icon_inputs @helper,
    pub(crate) naval_counters @helper,
    pub railway_guns,
    pub(crate) active_abilities,
    pub country_counters,
    pub game_score,
    pub(crate) group_counters,
    pub saved_event_targets,
    pub(crate) battle_sounds,
    pub equipment_destruction,
    pub naval_result_sources,
    pub experience_gains,
    pub event_feed,
    pub air_losses,
    pub air_records,
    pub building_bombing,
    pub combat_engagement_refreshes,
    pub nuclear_strikes,
    pub market,
    pub leader_paths,
    pub country_appearance @helper_appearance,
    pub ship_models,
    pub province_unit_order @helper,
    pub(crate) naval_bases @helper,
    pub(crate) map_facilities,
    pub naval_combat_sources @helper,
    pub character_portraits @helper_cached,
    pub leader_portraits,
    pub(crate) supply_trains @helper,
    pub characters @helper_cached,
    pub combat_progress @helper,
    pub combat_details @helper,
    pub naval_details @helper,
    pub(crate) naval_result_details @helper,
    pub construction @helper,
    pub division_presentation @helper_exclusive,
    pub(crate) division_model_inputs,
    pub(crate) division_templates,
    pub(crate) economy @helper,
    pub fog @helper,
    pub governance @helper,
    pub intel @helper_cached,
    pub operation_outcomes,
    pub map_events,
    pub lexical,
    pub map_names,
    pub naval_history @helper,
    pub production @helper,
    pub projects,
    pub(crate) plans,
    pub session,
    pub state_flags,
    pub visibility @helper,
    pub(crate) raid_target_statuses @helper,
    pub(crate) raid_state_inputs,
    pub(crate) resource_map,
    pub(crate) raid_launch_inputs,
    pub(crate) slice_reads,
    pub(crate) faction_theatres,
    pub model_inputs,
    pub(crate) modifier_definitions,
    pub hired_advisors @helper,
    pub character_inputs @helper,
    pub logistics,
    pub(crate) combat_log,
    pub(crate) division_counts,
    pub(crate) country_factories,
    pub(crate) state_building_inputs,
    pub war_overview,
    pub(crate) industrial_organisations @helper,
    pub(crate) production_modifiers @helper,
    pub line_output,
    pub politics,
    pub peace_settlements,
    pub peace_bids,
    pub land_events,
    pub construction_ic,
    pub country_stability,
    pub(crate) new_divisions,
    pub(crate) country_tactic_unlocks @helper,
    pub weather @helper,
    pub(crate) naval_mission_inputs @helper
);

impl Capacities {
    /// # Safety
    /// Only the attaching runtime or its designated sizing pass may use this pack to prepare
    /// the live economy observers. Keep their storage live throughout the borrow and its use.
    pub unsafe fn economy_mut(&mut self) -> &mut economy::Capacities {
        &mut self.economy
    }

    /// # Safety
    /// Only the attaching runtime or its designated sizing pass may use this pack to prepare
    /// the live fleet-region observer. Keep its storage live throughout the borrow and its use.
    pub unsafe fn naval_counters_mut(&mut self) -> &mut naval_counters::Capacities {
        &mut self.naval_counters
    }

    pub fn raise_reserved_visibility(&mut self, slots: usize) {
        self.country_appearance.raise_reverse_visibility(slots);
    }
    pub(crate) fn raise_for_states(&mut self, states: usize) {
        self.state_flags.raise_for_states(states);
    }

    pub(crate) fn raise_for_provinces(&mut self, provinces: usize) {
        self.naval_history.rows[2] = self.naval_history.rows[2].max(provinces.saturating_sub(1));
        self.division_presentation.raise_for_provinces(provinces);
    }

    /// # Safety
    /// Run only during attach or the session's sizing pass. Use the active image and exclude
    /// competing presizers and observer teardown until all storage work is complete.
    pub unsafe fn presize_live_inputs(
        &mut self,
        memory: &impl Memory,
        base: usize,
    ) -> Result<(), &'static str> {
        // SAFETY: This is the session's sizing pass, with its observer storage still live.
        unsafe { self.presize_live_inputs_periodic(memory, base, true) }
    }

    /// # Safety
    /// Run only during attach or the session's sizing pass. Use the active image and exclude
    /// competing presizers and observer teardown until all storage work is complete.
    pub unsafe fn presize_live_inputs_periodic(
        &mut self,
        memory: &impl Memory,
        base: usize,
        capacity_census: bool,
    ) -> Result<(), &'static str> {
        let mut failure = None;
        for result in [
            capacity_census.then(|| {
                crate::country_scale::observe_advisor_modifiers(memory, base).map(|demand| {
                    // SAFETY: Runs during attach or the session's serial sizing pass.
                    if let Some(demand) = demand
                        && unsafe { characters::advisors::note_modifier_demand(demand) }
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
            Some(
                crate::country_scale::observe_land_event_words(memory, base).map(|demand| {
                    if let Some(words) = demand {
                        // SAFETY: Runs during attach or the session's serial sizing pass with live storage.
                        unsafe { collector_land_events::land_events::presize(words) };
                        self.land_events.words = self
                            .land_events
                            .words
                            .max(collector_land_events::land_events::capacity());
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

    pub fn raise_for_regions(&mut self, regions: usize) {
        collector_primitives::country_slots::raise(&mut self.air.scores.regions, regions);
    }

    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        self.plans.raise_for_countries(slots);
        collector_primitives::country_slots::raise(
            &mut self.event_feed.events,
            collector_primitives::country_slots::rows(slots, 2),
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

    pub fn raise_for_country_intel(&mut self, native_rows: usize) {
        self.intel.raise_for_country_intel(native_rows);
    }

    pub fn raise_for_characters(&mut self, demand: crate::country_scale::CharacterDemand) {
        self.characters.raise_for_characters(demand.characters);
        collector_primitives::country_slots::raise(
            &mut self.characters.bytes[3],
            collector_primitives::country_slots::rows(demand.unit_leader_bytes, 1),
        );
        self.intel.raise_for_script_flags(demand.flags);
        self.intel.raise_for_script_variables(demand.variables);
    }

    pub fn raise_for_raid_arenas(&mut self, demand: crate::country_scale::RaidArenas) {
        self.projects.raise_for_raid_type_tokens(demand.type_tokens);
        self.projects.raise_for_raid_paths(demand.path_rows);
        self.projects.raise_for_facilities(demand.facility_rows);
        collector_primitives::country_slots::raise(
            &mut self.air.scores.regions,
            demand.air_regions,
        );
        collector_primitives::country_slots::raise(
            &mut self.projects.counteroperatives,
            demand.counteroperatives,
        );
        self.raid_launch_inputs
            .raise_for_reservations(demand.reservation_words);
    }

    /// # Safety
    /// Only the attaching runtime or its designated session sizing pass may publish advisor demand,
    /// using measured demand for the current session. Tests must own the process-wide demand in
    /// isolation.
    pub unsafe fn raise_for_politics(&mut self, demand: crate::country_scale::PoliticsDemand) {
        self.war_overview.raise_for_demand(demand.war_overview);
        // SAFETY: Runs during attach or the session's serial sizing pass with measured demand.
        unsafe { characters::advisors::note_modifier_demand(demand.advisor_modifiers) };
        collector_primitives::country_slots::raise(
            &mut self.intel.agency_names,
            demand.agency_name_bytes,
        );
        collector_primitives::country_slots::raise(
            &mut self.intel.rows[17],
            demand.agency_upgrade_rows,
        );
        collector_primitives::country_slots::raise(
            &mut self.intel.agency_upgrade_definitions,
            demand.agency_upgrade_definitions,
        );
        collector_primitives::country_slots::raise(&mut self.intel.rows[9], demand.operation_rows);
        self.operation_outcomes
            .raise_for_operations(demand.operation_rows);
        self.operation_outcomes
            .raise_for_descriptions(demand.operation_description_bytes);
        collector_primitives::country_slots::raise(
            &mut self.intel.rows[10],
            demand.operation_assignments,
        );
        collector_primitives::country_slots::raise(
            &mut self.intel.operation_descriptions,
            demand.operation_description_bytes,
        );
        collector_primitives::country_slots::raise(
            &mut self.intel.rows[4],
            demand.operative_memberships,
        );
        collector_primitives::country_slots::raise(
            &mut self.intel.operative_names,
            demand.operative_name_bytes,
        );
        collector_primitives::country_slots::raise(
            &mut self.intel.rows[18],
            demand.operative_missions,
        );
        collector_primitives::country_slots::raise(
            &mut self.intel.operative_mission_bytes,
            demand.operative_mission_bytes,
        );
        collector_primitives::country_slots::raise(
            &mut self.intel.operation_parts,
            demand.operation_parts,
        );
        collector_primitives::country_slots::raise(
            &mut self.intel.rows[19],
            demand.strategic_operatives,
        );
        collector_primitives::country_slots::raise(
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_ordinary_failure(&mut self, reason: Option<&'static str>) {
        if reason.is_some() {
            self.naval_counters.forget_censuses();
        }
        self.ordinary_failure = reason;
    }
    pub(crate) fn game_score_pilot(&mut self) -> &mut game_score::Captured {
        &mut self.game_score
    }
}

pub(crate) struct Homes {
    pub country_intel: daily_intel::Home,
    pub(crate) templates: division_templates::Home,
    pub(crate) naval_combat: naval_combat_sources::Home,
    pub(crate) projects: projects::RuntimeHome,
}

impl Homes {
    pub fn new(capacity: &Capacities) -> std::io::Result<Self> {
        Ok(Self {
            country_intel: daily_intel::Home::try_new(
                capacity.intel.rows[15],
                capacity.intel.intel_scratch,
            )?,
            templates: division_templates::new_home()?,
            naval_combat: naval_combat_sources::new_home()?,
            projects: projects::new_runtime_home()?,
        })
    }
}

impl Captured {
    #[cfg(test)]
    pub(crate) fn country_intel_home(&self) -> Option<daily_intel::Home> {
        self.intel.country_intel_home().cloned()
    }

    #[cfg(test)]
    pub fn naval_combat_home(&self) -> Option<naval_combat_sources::Home> {
        self.naval_combat_sources.home()
    }

    pub(crate) fn share_naval_combat_home(&mut self, home: &naval_combat_sources::Home) {
        self.naval_combat_sources.share_home(home);
    }

    #[cfg(test)]
    pub fn template_home(&self) -> Option<division_templates::Home> {
        self.division_templates.home()
    }

    pub(crate) fn share_template_home(&mut self, home: &division_templates::Home) {
        self.division_templates.share_home(home);
    }

    pub(crate) fn share_project_runtime_home(&mut self, home: &projects::RuntimeHome) {
        self.projects.share_runtime_home(home);
    }

    pub(crate) fn share_country_intel(&mut self, home: &daily_intel::Home) {
        self.intel.share_country_intel(home);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn share_homes(&mut self, homes: &Homes) {
        self.share_country_intel(&homes.country_intel);
        self.share_template_home(&homes.templates);
        self.share_naval_combat_home(&homes.naval_combat);
        self.share_project_runtime_home(&homes.projects);
    }

    pub(crate) fn reset_keeping_country_intel(&mut self) {
        let loan = self.intel.take_country_intel_loan();
        self.reset();
        self.intel.restore_country_intel_loan(loan);
    }

    pub(crate) fn release_country_intel(&mut self) {
        self.intel.release_country_intel();
    }

    pub fn country_intel_sample(
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
        // SAFETY: Admitted hourly capture owns attributing lost casualty reads to land events.
        unsafe { self.land_events.lost_casualty_reads(progress.lost_reads()) };
        self.combat_progress = progress;
        self.overhead[TableId::LandCombatProgress.index()] += ticks;
    }

    /// # Safety
    /// Only the spool consumer may borrow this live pack for assembly. Use it for the frame's own
    /// interval and finish all publications before releasing the observer storage.
    pub unsafe fn operation_outcomes(&self) -> &operation_outcomes::Captured {
        &self.operation_outcomes
    }

    /// # Safety
    /// Only the spool consumer may borrow this live pack for assembly. Use it for the frame's own
    /// interval and finish all publications before releasing the observer storage.
    pub unsafe fn map_events(&self) -> &map_events::Captured {
        &self.map_events
    }

    pub(crate) fn seal_map_events(&mut self, hour: i32) -> u64 {
        let ticks = self.map_events.seal(hour);
        self.overhead[TableId::MapEvents.index()] =
            self.overhead[TableId::MapEvents.index()].saturating_add(ticks);
        ticks
    }

    /// # Safety
    /// Only the originating spool may discard this frame's event-feed handoff after conversion and
    /// writer preparation, before returning the slot.
    pub unsafe fn discard_event_feed(&mut self) {
        // SAFETY: Caller is the originating spool discarding this uncommitted event-feed handoff.
        unsafe { self.event_feed.discard() };
    }
    pub(crate) fn take_event_feed_seal(&mut self) -> event_feed::Captured {
        std::mem::replace(&mut self.event_feed, event_feed::Captured::vacant())
    }
    pub(crate) fn restore_event_feed_seal(
        &mut self,
        mut feed: event_feed::Captured,
        failure: Option<&'static str>,
    ) {
        if let Some(reason) = failure {
            // SAFETY: Capture owns the failed event-feed seal recovery path for this hour.
            unsafe { feed.fail_prepared(reason) };
        }
        self.event_feed = feed;
    }
    pub fn event_feed(&self) -> &event_feed::Captured {
        &self.event_feed
    }

    pub fn fuel_awards(&self) -> &fuel_awards::Captured {
        &self.fuel_awards
    }

    pub(crate) fn set_load_signal(&mut self, signal: i64) {
        self.combat_log.set_signal(signal);
    }

    pub fn line_output(&self) -> &line_output::Captured {
        &self.line_output
    }

    pub fn construction_ic(&self) -> &construction_ic::Captured {
        &self.construction_ic
    }

    pub fn apply_combat_hints(
        &mut self,
        land: Option<&combat_details::Capacities>,
        naval: Option<&naval_details::Capacities>,
    ) -> [bool; 2] {
        let mut applied = [false; 2];
        if let Some(capacity) = land {
            applied[0] = self.combat_details.try_grow_hint(capacity).is_ok();
        }
        if let Some(capacity) = naval {
            applied[1] = self.naval_details.try_grow_hint(capacity).is_ok();
        }
        applied
    }

    #[cfg(any(test, feature = "test-support"))]
    /// # Safety
    /// Only an isolated fixture or the admitted hour owning the observer interval may take this
    /// pack. Keep its observer alive and do not drain the interval more than once.
    pub unsafe fn construction_ic_mut(&mut self) -> &mut construction_ic::Captured {
        &mut self.construction_ic
    }

    #[cfg(any(test, feature = "test-support"))]
    /// # Safety
    /// Only an isolated fixture or the admitted hour owning the observer interval may take this
    /// pack. Keep its observer alive and do not drain the interval more than once.
    pub unsafe fn line_output_mut(&mut self) -> &mut line_output::Captured {
        &mut self.line_output
    }

    pub(crate) fn set_capture_mode(&mut self, mode: CaptureMode) {
        self.capture_mode = mode;
    }

    #[cfg(test)]
    pub fn capture_mode(&self) -> CaptureMode {
        self.capture_mode
    }

    pub fn records_any(&self, tables: &[TableId]) -> bool {
        records_any(self.capture_mode, tables)
    }

    pub(crate) fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.group_counters.prepare_hour(force_full);
        self.governance.reset_slice_selections(hour, force_full);
        self.air_relations.prepare_hour(hour, force_full);
        self.air_histories.prepare_hour(hour, force_full);
        self.raid_launch_inputs.reset_selection(hour, force_full);
        self.raid_state_inputs.reset_selections(hour, force_full);
        self.resource_map.reset_selection(hour, force_full);
        self.country_stability.prepare_hour(hour, force_full);
        self.combat_log.prepare_hour(hour, force_full);
        // SAFETY: Runs during admitted hour preparation before combat capture starts.
        unsafe { self.combat_progress.prepare_hour(hour, force_full) };
        self.characters.prepare_hour(hour, force_full);
        self.naval_mission_inputs.reset_selection(hour, force_full);
        self.supply_trains.reset_selection(hour, force_full);
        self.hired_advisors.reset_selection(hour, force_full);
        self.air.prepare_hour(hour, force_full);
        self.air.begin_supply_hour();
        self.fog.prepare_hour(hour, force_full);
        self.naval_history.prepare_hour(hour, force_full);
        self.session.prepare_hour(hour, force_full);
    }

    pub(crate) fn install_unit_inputs(
        &mut self,
        inputs: Option<engine_allocation::SharedBox<UnitInputs>>,
    ) {
        self.unit_inputs = inputs;
    }

    pub(crate) fn release_unit_inputs(&mut self) {
        self.unit_inputs = None;
    }

    pub(crate) fn take_settled_army_store(
        &mut self,
    ) -> Option<division_presentation::held_store::Loan> {
        self.division_presentation.take_settled_store()
    }

    pub(crate) fn decode_owned_censuses(&mut self) -> [u64; 4] {
        let start = collector_primitives::clock::qpc();
        self.naval_bases.decode_owned();
        self.group_counters.decode_owned();
        let map = collector_primitives::clock::qpc()
            .saturating_sub(start)
            .max(0) as u64;
        let start = collector_primitives::clock::qpc();
        self.governance.decode_owned();
        let governance = collector_primitives::clock::qpc()
            .saturating_sub(start)
            .max(0) as u64;
        let start = collector_primitives::clock::qpc();
        self.naval_counters.decode_owned();
        let tasks = collector_primitives::clock::qpc()
            .saturating_sub(start)
            .max(0) as u64;
        let start = collector_primitives::clock::qpc();
        self.projects.decode_owned();
        let projects = collector_primitives::clock::qpc()
            .saturating_sub(start)
            .max(0) as u64;
        [map, governance, tasks, projects]
    }

    pub(crate) fn discard_owned_censuses(&mut self) {
        self.naval_bases.discard_owned();
    }

    pub(crate) fn decode_mission_tails(&mut self) -> [u64; 2] {
        let start = collector_primitives::clock::qpc();
        self.naval_mission_inputs.decode_owned();
        let navies = collector_primitives::clock::qpc()
            .saturating_sub(start)
            .max(0) as u64;
        let start = collector_primitives::clock::qpc();
        self.railway_guns.decode_owned();
        let railway = collector_primitives::clock::qpc()
            .saturating_sub(start)
            .max(0) as u64;
        [navies, railway]
    }

    pub(crate) fn decode_division_presentation_inputs(&mut self) -> [u64; 2] {
        let ticks = self.division_presentation.decode_inputs();
        #[cfg(feature = "acquisition_profile")]
        self.division_presentation.record_profile();
        ticks
    }

    /// # Safety
    /// Only the spool consumer may borrow this live pack for assembly. Use it for the frame's own
    /// interval and finish all publications before releasing the observer storage.
    pub unsafe fn politics(&self) -> &politics::Captured {
        &self.politics
    }

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
        let _reads = collector_memory::read_accounting::table(TableId::Countries);
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

    pub(crate) fn take_presentation_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
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
            countries: engine_allocation::SharedBox::clone(countries),
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

    pub(crate) fn restore_presentation_request(
        &mut self,
        mut request: crate::acquisition_jobs::PresentationRequest,
        report: Result<
            division_presentation::CaptureReport,
            collector_workers::acquisition::Failure,
        >,
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
                collector_workers::acquisition::Failure::Panicked => {
                    "division presentation acquisition panicked"
                }
                collector_workers::acquisition::Failure::Stopped => {
                    "division presentation acquisition stopped before reading"
                }
            }),
        });
        self.division_presentation = request.captured;
        army_stats
    }

    pub(crate) fn acquire_presentation(&mut self) -> division_presentation::CaptureReport {
        self.presentation_report.take().unwrap_or_else(|| {
            self.division_presentation
                .fail_owned_capture("division presentation acquisition not supplied")
        })
    }

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

    pub(crate) fn take_research_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        fingerprints: Box<collector_certificates::slicing::SliceFingerprints>,
    ) -> crate::acquisition_jobs::ResearchRequest {
        crate::acquisition_jobs::ResearchRequest {
            fingerprints,
            countries: engine_allocation::SharedBox::clone(countries),
            context: governance::CaptureContext {
                base,
                game_state: state,
                hour,
                force_full,
            },
            part: self.governance.take_research(),
        }
    }

    pub(crate) fn omit_research(&mut self, base: usize, state: usize, hour: i32, force_full: bool) {
        let mut part = self.governance.take_research();
        part.fail(
            governance::CaptureContext {
                base,
                game_state: state,
                hour,
                force_full,
            },
            NOT_RECORDED,
        );
        self.governance.restore_research(part);
    }

    pub(crate) fn restore_research_request(
        &mut self,
        mut request: crate::acquisition_jobs::ResearchRequest,
        report: Result<
            crate::acquisition_jobs::ResearchReport,
            collector_workers::acquisition::Failure,
        >,
    ) -> Box<collector_certificates::slicing::SliceFingerprints> {
        let ticks = match report {
            Ok(report) => report.ticks,
            Err(failure) => {
                request.fingerprints.forget();
                request.part.fail(
                    request.context,
                    match failure {
                        collector_workers::acquisition::Failure::Panicked => {
                            "research acquisition panicked"
                        }
                        collector_workers::acquisition::Failure::Stopped => {
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

    pub(crate) fn family_selection(
        &self,
        family: usize,
    ) -> &collector_certificates::slicing::SliceSelection {
        match family {
            collector_certificates::slicing::RESEARCH => self.governance.research_selection(),
            collector_certificates::slicing::TECHNOLOGIES => self.governance.technology_selection(),
            collector_certificates::slicing::FOCUS => self.governance.focus_selection(),
            collector_certificates::slicing::PROJECTS => self.projects.project_selection(),
            collector_certificates::slicing::RAID_LAUNCH => self.raid_launch_inputs.selection(),
            collector_certificates::slicing::RAID_STATE => self.raid_state_inputs.state_selection(),
            collector_certificates::slicing::RAID_BUILDINGS => {
                self.raid_state_inputs.country_selection()
            }
            collector_certificates::slicing::TRADE_ROUTES => self.resource_map.route_selection(),
            collector_certificates::slicing::RELATIONS => self.intel.relations_selection(),
            collector_certificates::slicing::STABILITY => self.country_stability.selection(),
            collector_certificates::slicing::COMBAT_LOG => self.combat_log.selection(),
            collector_certificates::slicing::AIR_RELATIONS => self.air_relations.selection(),
            collector_certificates::slicing::AIR_HISTORIES => self.air_histories.selection(),
            collector_certificates::slicing::NAVAL_MISSIONS => {
                self.naval_mission_inputs.selection()
            }
            collector_certificates::slicing::SUPPLY_TRAINS => self.supply_trains.selection(),
            collector_certificates::slicing::AVAILABLE_ADVISORS => {
                self.hired_advisors.available_selection()
            }
            _ => unreachable!("unregistered sliced family {family}"),
        }
    }

    pub fn slice_selection(
        &self,
        table: TableId,
    ) -> Option<&collector_certificates::slicing::SliceSelection> {
        collector_certificates::slicing::family_of(table).map(|family| {
            if family == collector_certificates::slicing::RAID_STATE {
                self.raid_state_inputs.table_selection(table)
            } else {
                self.family_selection(family)
            }
        })
    }

    pub fn research_carried_owners(&self) -> Option<&[u32]> {
        self.governance.research_carried_owners()
    }

    pub fn technology_carried_owners(&self) -> Option<&[u32]> {
        self.governance.technology_carried_owners()
    }

    pub fn stability_lines_carried_owners(&self) -> Option<&[u32]> {
        self.country_stability.lines_carried_owners()
    }

    pub fn project_row_owners(&self) -> std::io::Result<Vec<u32>> {
        self.projects.project_row_owners()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the governance request takes ownership of its fingerprints and each store separately"
    )]
    pub(crate) fn take_governance_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        technology_fingerprints: Box<collector_certificates::slicing::SliceFingerprints>,
        focus_fingerprints: Box<collector_certificates::slicing::SliceFingerprints>,
        building_store: Option<Box<construction::BuildingStore>>,
        leader_store: Option<Box<governance::LeaderStore>>,
        party_store: Option<Box<governance::PartyStore>>,
    ) -> crate::acquisition_jobs::GovernanceRequest {
        self.governance_report = None;
        self.governance.leave_out(
            governance::TABLES.map(|table| !replay_data::scope::records(self.capture_mode, table)),
        );
        crate::acquisition_jobs::GovernanceRequest {
            workers: Default::default(),
            technology_fingerprints,
            focus_fingerprints,
            building_store,
            leader_store,
            party_store,
            countries: engine_allocation::SharedBox::clone(countries),
            context: governance::CaptureContext {
                base,
                game_state: state,
                hour,
                force_full,
            },
            captured: std::mem::replace(&mut self.governance, governance::Captured::vacant()),
            buildings: self.construction.take_buildings(),
            buildings_lent: false,
        }
    }

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
            workers: Default::default(),
        }
    }

    pub(crate) fn restore_variants_request(
        &mut self,
        mut request: crate::acquisition_jobs::VariantsRequest,
        report: Result<[u64; production::N], collector_workers::acquisition::Failure>,
    ) {
        let ticks = report.unwrap_or_else(|failure| {
            request.fail(match failure {
                collector_workers::acquisition::Failure::Panicked => "variant acquisition panicked",
                collector_workers::acquisition::Failure::Stopped => {
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

    pub(crate) fn restore_governance_request(
        &mut self,
        mut request: crate::acquisition_jobs::GovernanceRequest,
        report: Result<
            crate::acquisition_jobs::GovernanceReport,
            collector_workers::acquisition::Failure,
        >,
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
                    collector_workers::acquisition::Failure::Panicked => {
                        "governance acquisition panicked"
                    }
                    collector_workers::acquisition::Failure::Stopped => {
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

    #[expect(
        clippy::too_many_arguments,
        reason = "the supply-train request takes its part and fingerprints beside the hourly operands"
    )]
    pub(crate) fn take_supply_trains_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        part: usize,
        fingerprints: Option<Box<collector_certificates::slicing::SliceFingerprints>>,
    ) -> crate::acquisition_jobs::SupplyTrainsRequest {
        if part == 0 {
            self.supply_trains_report = None;
        }
        crate::acquisition_jobs::SupplyTrainsRequest {
            countries: engine_allocation::SharedBox::clone(countries),
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

    pub(crate) fn restore_supply_trains_request(
        &mut self,
        mut request: crate::acquisition_jobs::SupplyTrainsRequest,
        report: Result<supply_trains::CaptureReport, collector_workers::acquisition::Failure>,
    ) -> Option<Box<collector_certificates::slicing::SliceFingerprints>> {
        let failed = report.is_err();
        let report = match report {
            Ok(report) => report,
            Err(failure) => self.supply_trains.fail_owned_capture(match failure {
                collector_workers::acquisition::Failure::Panicked => {
                    "supply trains acquisition panicked"
                }
                collector_workers::acquisition::Failure::Stopped => {
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

    fn finish_supply_trains(&mut self) -> supply_trains::CaptureReport {
        self.supply_trains_report.take().unwrap_or_else(|| {
            if !records_any(self.capture_mode, &supply_trains::TABLES) {
                return self.supply_trains.fail_owned_capture(NOT_RECORDED);
            }
            self.supply_trains
                .fail_owned_capture("supply_trains acquisition not supplied")
        })
    }

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

    pub(crate) fn restore_character_portraits_request(
        &mut self,
        request: crate::acquisition_jobs::CharacterPortraitsRequest,
        report: Result<character_portraits::CaptureReport, collector_workers::acquisition::Failure>,
    ) {
        let report = match report {
            Ok(report) => report,
            Err(failure) => self.character_portraits.fail_owned_capture(match failure {
                collector_workers::acquisition::Failure::Panicked => {
                    "character portraits acquisition panicked"
                }
                collector_workers::acquisition::Failure::Stopped => {
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

    fn finish_character_portraits(&mut self) -> character_portraits::CaptureReport {
        self.character_portraits_report.take().unwrap_or_else(|| {
            self.character_portraits
                .fail_owned_capture("character_portraits acquisition not supplied")
        })
    }

    pub(crate) fn take_division_model_inputs_request(
        &mut self,
        base: usize,
        state: usize,
        inputs: Option<
            engine_allocation::SharedBox<collector_divisions::division_census::UnitInputs>,
        >,
    ) -> crate::acquisition_jobs::DivisionModelInputsRequest {
        crate::acquisition_jobs::DivisionModelInputsRequest {
            base,
            game_state: state,
            inputs,
            captured: std::mem::replace(
                &mut self.division_model_inputs,
                division_model_inputs::Captured::vacant(),
            ),
        }
    }

    pub(crate) fn restore_division_model_inputs_request(
        &mut self,
        mut request: crate::acquisition_jobs::DivisionModelInputsRequest,
        completed: bool,
    ) -> Option<u32> {
        drop(request.inputs.take());
        let mut captured = request.captured;
        if !completed {
            captured.fail_job("division model input acquisition failed");
        }
        let training = captured.training();
        self.division_model_inputs = captured;
        training
    }

    pub(crate) fn take_province_unit_order_request(
        &mut self,
        base: usize,
        state: usize,
        inputs: Option<
            engine_allocation::SharedBox<collector_divisions::division_census::UnitInputs>,
        >,
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

    pub(crate) fn restore_province_unit_order_request(
        &mut self,
        mut request: crate::acquisition_jobs::ProvinceUnitOrderRequest,
        report: Result<province_unit_order::CaptureReport, collector_workers::acquisition::Failure>,
    ) -> Option<Box<province_unit_order::ProvinceUnitStore>> {
        drop(request.inputs.take());
        if report.is_err()
            && let Some(store) = request.store.as_deref_mut()
        {
            store.forget();
        }
        self.province_unit_order_report = Some(match report {
            Ok(report) => report,
            Err(failure) => request.captured.fail_owned_capture(match failure {
                collector_workers::acquisition::Failure::Panicked => {
                    "province unit order acquisition panicked"
                }
                collector_workers::acquisition::Failure::Stopped => {
                    "province unit order acquisition stopped before reading"
                }
            }),
        });
        self.province_unit_order = request.captured;
        request.store
    }

    fn finish_province_unit_order(&mut self) -> province_unit_order::CaptureReport {
        self.province_unit_order_report.take().unwrap_or_else(|| {
            self.province_unit_order
                .fail_owned_capture("province_unit_order acquisition not supplied")
        })
    }

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
            + self.air_supply_spare.as_ref().map_or(0, |capture| {
                size_of::<engine_map_air::map_air_supply::Capture>() + capture.allocated_bytes()
            })
            + self.missions_spare.as_ref().map_or(0, |pack| {
                size_of::<air::Captured>() + pack.allocated_bytes()
            })
            + self
                .gated_spare
                .as_ref()
                .map_or(0, |packs| packs.allocated_bytes())
    }

    pub(crate) fn take_air_relations_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        part: usize,
    ) -> crate::acquisition_jobs::AirRelationsRequest {
        crate::acquisition_jobs::AirRelationsRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            part,
            staging: self.air_relations.take_part(part),
        }
    }

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

    pub(crate) fn take_air_missions_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        mission_store: Option<Box<air::MissionStore>>,
    ) -> crate::acquisition_jobs::AirMissionsRequest {
        let mut pack = self.missions_spare.take().expect("missions spare present");
        std::mem::swap(&mut self.air, &mut pack);
        crate::acquisition_jobs::AirMissionsRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state: state,
            parent: MISSING,
            pack,
            histories: self.air_histories.take_parts(),
            mission_store,
        }
    }

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

    pub(crate) fn take_air_histories_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        part: usize,
    ) -> crate::acquisition_jobs::AirHistoriesRequest {
        crate::acquisition_jobs::AirHistoriesRequest {
            countries: engine_allocation::SharedBox::clone(countries),
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

    pub(crate) fn settle_air_histories(&self) {
        self.air_histories.settle(&mut air_histories::opening());
    }

    pub(crate) fn decode_air_copies(&mut self) {
        self.air_relations.decode();
        self.air_histories.decode();
    }

    pub(crate) fn decode_capture_tails(&mut self) -> [u64; 5] {
        let mut ticks = [0; 5];
        macro_rules! settle {
            ($index:expr, $capture:ident, $method:ident) => {{
                let start = collector_primitives::clock::qpc();
                self.$capture.$method();
                ticks[$index] = collector_primitives::clock::qpc()
                    .saturating_sub(start)
                    .max(0) as u64;
            }};
        }
        settle!(0, division_templates, settle_owned);
        settle!(1, ship_models, settle_owned);
        settle!(2, resource_map, settle_owned);
        settle!(3, production, settle_variants_owned);
        settle!(4, combat_details, settle_owned);
        let start = collector_primitives::clock::qpc();
        self.division_presentation
            .join_combat_members(&mut self.combat_details);
        ticks[4] += collector_primitives::clock::qpc()
            .saturating_sub(start)
            .max(0) as u64;
        ticks
    }

    pub(crate) fn drop_air_copies(&mut self) {
        self.air_relations.dropped();
        self.air_histories.dropped();
    }

    #[cfg(test)]
    pub(crate) fn drop_and_reset(&mut self) {
        self.drop_air_copies();
        self.reset();
    }

    pub(crate) fn take_orders_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        orders: &mut Orders,
        base: usize,
        game_state: usize,
    ) -> crate::acquisition_jobs::OrdersRequest {
        crate::acquisition_jobs::OrdersRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            base,
            game_state,
            orders: std::mem::replace(orders, Orders::vacant()),
            plans: std::mem::replace(&mut self.plans, plans::Captured::vacant()),
            group_source: None,
        }
    }

    pub(crate) fn install_fronts(&mut self, fronts: plans::Fronts, completed: bool) {
        self.plans.install_fronts(fronts, completed);
    }

    pub(crate) fn restore_orders_request(
        &mut self,
        mut request: crate::acquisition_jobs::OrdersRequest,
        completed: bool,
        orders: &mut Orders,
    ) {
        if !completed {
            request.fail("army order acquisition failed");
        }
        std::mem::swap(orders, &mut request.orders);
        std::mem::swap(&mut self.plans, &mut request.plans);
    }

    pub(crate) fn take_ship_models_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
        state: usize,
        census: ShipCensus,
    ) -> crate::acquisition_jobs::ShipModelsRequest {
        self.ship_models_report = None;
        crate::acquisition_jobs::ShipModelsRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            context: ship_models::CaptureContext::new(base),
            game_state: state,
            captured: std::mem::replace(&mut self.ship_models, ship_models::Captured::vacant()),
            census,
            details: None,
        }
    }

    pub(crate) fn restore_ship_models_request(
        &mut self,
        mut request: crate::acquisition_jobs::ShipModelsRequest,
        report: Result<(), collector_workers::acquisition::Failure>,
    ) -> ShipCensus {
        if let Err(failure) = report {
            let reason = match failure {
                collector_workers::acquisition::Failure::Panicked => {
                    "ship models acquisition panicked"
                }
                collector_workers::acquisition::Failure::Stopped => {
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

    pub(crate) fn take_ship_model_parts(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        base: usize,
    ) -> [Option<crate::acquisition_jobs::ShipModelDetailsRequest>; ship_models::MAX_PARTS] {
        if self.ship_models_report.is_some() || self.ship_models.has_terminal_failure() {
            return [const { None }; ship_models::MAX_PARTS];
        }
        crate::acquisition_jobs::ship_details::take_parts(&mut self.ship_models, countries, base)
    }

    pub(crate) fn restore_ship_model_part(
        &mut self,
        mut request: crate::acquisition_jobs::ShipModelDetailsRequest,
        report: Result<ship_models::CaptureReport, collector_workers::acquisition::Failure>,
    ) {
        let report = report.unwrap_or_else(|failure| {
            request.captured.fail_owned_capture(match failure {
                collector_workers::acquisition::Failure::Panicked => {
                    "ship models acquisition panicked"
                }
                collector_workers::acquisition::Failure::Stopped => {
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
            ship_models::CaptureReport::default()
        } else {
            self.ship_models.capture_owned(memory, context)
        }
    }

    pub(crate) fn take_intel_residual_request(
        &mut self,
        countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
        context: intel::CaptureContext,
        flags: Option<Box<intel::FlagStore>>,
        variables: Option<Box<intel::VariableStore>>,
    ) -> (
        crate::acquisition_jobs::IntelResidualRequest,
        crate::acquisition_jobs::ScriptVariablesRequest,
    ) {
        self.intel_residual_report = None;
        self.script_variables_ticks = None;
        let mut captured = std::mem::replace(&mut self.intel, intel::Captured::vacant());
        captured.leave_out(
            intel::TABLES.map(|table| !replay_data::scope::records(self.capture_mode, table)),
        );
        let part = captured.take_variables();
        (
            crate::acquisition_jobs::IntelResidualRequest {
                countries: engine_allocation::SharedBox::clone(countries),
                context,
                captured,
                flags,
                aux_lent: false,
            },
            crate::acquisition_jobs::ScriptVariablesRequest {
                countries: engine_allocation::SharedBox::clone(countries),
                context,
                part,
                store: variables,
            },
        )
    }

    pub(crate) fn restore_script_variables(
        &mut self,
        mut request: crate::acquisition_jobs::ScriptVariablesRequest,
        report: Result<u64, collector_workers::acquisition::Failure>,
    ) -> Option<Box<intel::VariableStore>> {
        let mut store = request.store.take();
        match report {
            Ok(ticks) => self.script_variables_ticks = Some(ticks),
            Err(failure) => {
                if let Some(store) = store.as_deref_mut() {
                    store.forget();
                }
                request.part.fail(
                    request.context,
                    match failure {
                        collector_workers::acquisition::Failure::Panicked => {
                            "script variable acquisition panicked"
                        }
                        collector_workers::acquisition::Failure::Stopped => {
                            "script variable acquisition stopped before reading"
                        }
                    },
                );
            }
        }
        self.intel.restore_variables(request.part);
        store
    }

    pub(crate) fn restore_intel_residual_request(
        &mut self,
        mut request: crate::acquisition_jobs::IntelResidualRequest,
        report: Result<intel::CaptureReport, collector_workers::acquisition::Failure>,
    ) -> Option<Box<intel::FlagStore>> {
        let mut flags = request.flags.take();
        if report.is_err()
            && let Some(store) = flags.as_deref_mut()
        {
            store.forget();
        }
        self.intel_residual_report = Some(match report {
            Ok(report) => report,
            Err(failure) => request.captured.fail_residual_capture(
                request.context,
                match failure {
                    collector_workers::acquisition::Failure::Panicked => {
                        "residual Intel acquisition panicked"
                    }
                    collector_workers::acquisition::Failure::Stopped => {
                        "residual Intel acquisition stopped before reading"
                    }
                },
            ),
        });
        self.intel = request.captured;
        flags
    }
    pub(crate) fn settle_intel_residual(&mut self, context: intel::CaptureContext) {
        let report = self.finish_intel_residual(context);
        self.intel_residual_report = Some(report);
    }

    fn finish_intel_residual(&mut self, context: intel::CaptureContext) -> intel::CaptureReport {
        self.intel_residual_report.take().unwrap_or_else(|| {
            self.intel
                .fail_residual_capture(context, "residual Intel acquisition not supplied")
        })
    }

    pub(crate) fn relations_pilot(&mut self) -> &mut intel::Captured {
        &mut self.intel
    }

    pub(crate) fn characters_pilot(&mut self) -> &mut characters::Captured {
        &mut self.characters
    }

    pub(crate) fn projects_pilot(&mut self) -> &mut projects::Captured {
        &mut self.projects
    }

    pub(crate) fn lexical_pilot(&mut self) -> &mut lexical::Captured {
        &mut self.lexical
    }

    pub(crate) fn air_wing_pilot(&mut self) -> &mut air::Captured {
        &mut self.air
    }

    pub(crate) fn air_stats_pilot(&mut self) -> &mut air::Captured {
        &mut self.air
    }

    pub(crate) fn air_bases_pilot(&mut self) -> &mut air::Captured {
        &mut self.air
    }

    pub(crate) fn naval_pilot(&mut self) -> &mut naval_history::Captured {
        &mut self.naval_history
    }

    pub(crate) fn install_reserved_land_certificate(&mut self) {
        if !self.country_appearance.reserved_land_certificate_ready() {
            self.naval_history.withhold_reserved_land_certificate();
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

    pub(crate) fn session_pilot(&mut self) -> &mut session::Captured {
        &mut self.session
    }

    pub(crate) fn supremacy_pilot(&mut self) -> &mut naval_history::Captured {
        &mut self.naval_history
    }

    pub(crate) fn sunk_pilot(&mut self) -> &mut naval_history::Captured {
        &mut self.naval_history
    }

    pub(crate) fn radar_pilot(&mut self) -> &mut fog::Captured {
        &mut self.fog
    }

    pub(crate) fn intel_pilot(&mut self) -> &mut fog::Captured {
        &mut self.fog
    }

    pub(crate) fn weather_pilot(&mut self) -> &mut weather::Captured {
        &mut self.weather
    }

    pub(crate) fn production_rest_pilot(&mut self) -> &mut production::Captured {
        &mut self.production
    }

    pub(crate) fn construction_rest_pilot(&mut self) -> &mut construction::Captured {
        &mut self.construction
    }

    pub(crate) fn economy_rest_pilot(&mut self) -> &mut economy::Captured {
        &mut self.economy
    }

    pub(crate) fn capitulation_pilot(&mut self) -> &mut intel::Captured {
        &mut self.intel
    }

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

    pub(crate) fn set_modifier_definitions_moved(&mut self, moved: bool) {
        self.modifier_definitions.header_changed = moved;
    }

    pub(crate) fn set_new_division_mark(&mut self, mark: new_divisions::Mark) {
        self.new_divisions.set_mark(mark);
    }

    pub(crate) fn set_divisions_raw_mode(&mut self, raw: bool) {
        self.production.divisions_raw_mode = raw;
    }

    pub(crate) fn install_division_detail(
        &mut self,
        details: &mut crate::division_detail::OwnedDetails,
    ) {
        details.install(&mut self.production);
    }

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

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn test_resize_division_buffers(&mut self, lens: [usize; 6]) {
        use engine_deployment::{
            army_upgrade_request_row, carmy_upgrades_requests, cupgrade_delivery,
        };
        self.production.division_equipment.rows =
            vec![engine_native::cequipment_variant::pool::DivisionRow::default(); lens[1]];
        self.production.groups.rows = vec![carmy_upgrades_requests::Row::default(); lens[2]];
        self.production.requests.rows = vec![army_upgrade_request_row::Row::default(); lens[3]];
        self.production.deliveries.rows = vec![cupgrade_delivery::Row::default(); lens[4]];
        self.production.produced.rows = vec![cupgrade_delivery::ProducedRow::default(); lens[5]];
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn test_division_detail(&mut self) -> &mut production::Captured {
        &mut self.production
    }

    pub(crate) fn take_state_visitor(&mut self) -> state_flags::Captured {
        std::mem::take(&mut self.state_flags)
    }
    pub fn restore_state_visitor(&mut self, visitor: state_flags::Captured) {
        self.state_flags = visitor;
    }

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
    #[cfg(test)]
    pub(crate) fn test_division_presentation_tables(
        &self,
    ) -> [replay_frame::frame::TableView; division_presentation::N] {
        self.division_presentation.tables()
    }
    pub(crate) fn install_division_counts(
        &mut self,
        census: &collector_divisions::division_census::Census,
    ) {
        self.division_counts.install(census);
    }
    pub(crate) fn install_failed_division_counts(&mut self, reason: &'static str) {
        self.division_counts.fail(reason);
    }

    /// # Safety
    /// Only the admitted capture may create this adapter for its own frame and active image.
    /// Complete or refuse its sole combat observer loan within the same hour before teardown.
    pub(crate) unsafe fn sources<'a, M: Memory>(
        &'a mut self,
        memory: &'a M,
        base: usize,
    ) -> Sources<'a, M> {
        Sources {
            captured: self,
            memory,
            base,
        }
    }
    #[cfg(test)]
    pub(crate) fn capture_tables_all(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        force_full: bool,
    ) -> [u64; TABLE_COUNT] {
        if self.ordinary_ticks.is_none() && self.ordinary_failure.is_none() {
            let countries = engine_allocation::shared(
                engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
            )
            .unwrap();
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
        let countries = engine_allocation::shared(
            engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
        )
        .unwrap();
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
        tests::stage_fixture_acquisitions(
            self,
            memory,
            base,
            game_state,
            current_hour,
            force_full,
        );
        let mut ticks = self.capture_tables(memory, base, game_state, current_hour, force_full);
        scatter(
            &weather::TABLES,
            self.weather.capture(memory, base, game_state),
            &mut ticks,
        );
        let mut orders = Orders::vacant();
        tests::stage_fixture_late(
            self,
            memory,
            base,
            game_state,
            current_hour,
            force_full,
            &mut orders,
        );
        self.capture_tables_late(
            memory,
            base,
            game_state,
            current_hour,
            force_full,
            &mut orders,
            &mut ticks,
        );
        ticks
    }

    #[cfg(test)]
    pub(crate) fn capture(
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

    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands (memory, base, game state, hour, full flag) travel beside the tick counters and inline choice"
    )]
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
        } else if !ProductionDailyPacks::recorded(self.capture_mode) {
            self.fail_missing_production_daily(current_hour, force_full, NOT_RECORDED);
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
            self.fail_missing_production_daily(
                current_hour,
                force_full,
                "slow acquisition not supplied",
            );
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

    #[inline(never)]
    pub(crate) fn capture_tables(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        force_full: bool,
    ) -> [u64; TABLE_COUNT] {
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let mut ticks = [0; TABLE_COUNT];
        macro_rules! ordinary {
            ($($pack:ident),+ $(,)?) => {$(
                let cost = self.$pack.capture(memory, base, game_state);
                scatter(&$pack::TABLES, cost, &mut ticks);
            )+};
        }
        let weather_ticks = self.weather.finish_owned_capture();
        scatter(&weather::TABLES, weather_ticks, &mut ticks);
        ordinary!(state_flags, event_feed);
        if let Some(done) = self.ordinary_ticks.take() {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            self.ordinary_failure = Some("ordinary acquisition not supplied");
            self.naval_counters.forget_censuses();
        }
        if let Some(done) = self.slow_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            self.fail_missing_slow(current_hour, force_full);
        }
        if let Some(done) = self.stability_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else if !self.records_any(&country_stability::TABLES) {
            self.fail_missing_stability(current_hour, force_full, NOT_RECORDED);
        } else {
            self.fail_missing_stability(
                current_hour,
                force_full,
                "stability acquisition not supplied",
            );
        }
        self.assemble_production_daily(
            memory,
            base,
            game_state,
            current_hour,
            force_full,
            &mut ticks,
            false,
        );
        if let Some(done) = self.war_overview_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else if !self.records_any(&war_overview::TABLES) {
            self.fail_missing_war_overview(current_hour, force_full, NOT_RECORDED);
        } else {
            self.fail_missing_war_overview(
                current_hour,
                force_full,
                "war overview acquisition not supplied",
            );
        }
        if let Some(done) = self.template_ticks.replace([0; division_templates::N]) {
            scatter(&division_templates::TABLES, done, &mut ticks);
        } else {
            self.division_templates.fail_capture(
                current_hour,
                force_full,
                "template acquisition not supplied",
            );
        }
        if let Some(done) = self.raid_state_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            self.fail_missing_raid_state(current_hour, force_full);
        }
        if let Some(done) = self.raid_targets_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else if !self.records_any(&raid_target_statuses::TABLES) {
            self.fail_missing_raid_targets(current_hour, force_full, NOT_RECORDED);
        } else {
            self.fail_missing_raid_targets(
                current_hour,
                force_full,
                "raid target acquisition not supplied",
            );
        }
        if let Some(done) = self.raid_launch_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            self.fail_missing_raid_launch(current_hour, force_full);
        }
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
            self.fail_missing_gated(current_hour, force_full);
        }
        if let Some(done) = self
            .character_inputs_ticks
            .replace([0; character_inputs::N])
        {
            scatter(&character_inputs::TABLES, done, &mut ticks);
        } else {
            self.character_inputs.fail_capture(
                current_hour,
                force_full,
                "character inputs acquisition not supplied",
            );
        }
        ticks[TableId::StateBuildings.index()] =
            self.construction.finish_buildings(current_hour, force_full);
        let cost = self.division_model_inputs.finish();
        scatter(&division_model_inputs::TABLES, cost, &mut ticks);
        let province_order_report = self.finish_province_unit_order();
        scatter(
            &province_unit_order::TABLES,
            province_order_report.ticks,
            &mut ticks,
        );
        let portraits_report = self.finish_character_portraits();
        scatter(
            &character_portraits::TABLES,
            portraits_report.ticks,
            &mut ticks,
        );
        let supply_report = self.finish_supply_trains();
        scatter(&supply_trains::TABLES, supply_report.ticks, &mut ticks);
        let intel_context = intel::CaptureContext {
            base,
            game_state,
            hour: current_hour,
            force_full,
        };
        let intel_report = self.finish_intel_residual(intel_context);
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

    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands (memory, base, game state, hour, full flag) travel beside the orders and tick counters"
    )]
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
        let _allocation_phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        if let Some(cost) = self.script_variables_ticks.take() {
            ticks[intel::TABLES[8].index()] += cost;
        }
        if let Some(done) = self.late_ticks.replace([0; TABLE_COUNT]) {
            for (total, cost) in ticks.iter_mut().zip(done) {
                *total += cost;
            }
        } else {
            self.fail_missing_late(current_hour, force_full, orders);
        }
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
            scatter(
                &combat_engagement_refreshes::TABLES,
                report.engagement_refreshes,
                ticks,
            );
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

    pub(crate) fn capture_ship_models(&mut self, memory: &impl Memory, base: usize) -> u64 {
        let context = ship_models::CaptureContext::new(base);
        let report = self.acquire_ship_models(memory, context);
        let table = TableId::ShipModels;
        if self.ship_models.has_terminal_failure() {
            0
        } else {
            report.ticks[0].saturating_add(self.overhead[table.index()])
        }
    }
}

pub(crate) struct Sources<'a, M: Memory> {
    captured: &'a mut Captured,
    memory: &'a M,
    base: usize,
}
impl<M: Memory> Sources<'_, M> {
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
        // SAFETY: Runs in the admitted hour's sole land-combat walk; `finish_land` settles it.
        unsafe { self.captured.combat_progress.begin() };
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
        // SAFETY: Settles the admitted hour's land-combat baseline and reports observer demand.
        unsafe {
            self.captured.combat_progress.finish(outcome);
            self.captured
                .land_events
                .lost_casualty_reads(self.captured.combat_progress.lost_reads());
        }
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
}

fn unpublished(outcomes: &[Outcome]) -> bool {
    outcomes.iter().any(|outcome| {
        !matches!(outcome, Outcome::Rows { .. })
            && !matches!(outcome, Outcome::Unavailable { reason } if *reason == replay_data::cadence::SKIPPED_REASON)
    })
}

#[cfg(test)]
fn unit_sources(
    inputs: &Option<engine_allocation::SharedBox<UnitInputs>>,
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

pub struct Divisions {
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
    pub fn sources(&self) -> production::Sources<'_, DivisionSource> {
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

#[cfg(any(test, feature = "test-support"))]
pub fn mode_gated_walks() -> Vec<(&'static str, Vec<TableId>)> {
    let research = governance::RESEARCH.map(|index| governance::TABLES[index]);
    let character_part = |part: usize| {
        crate::decode::CHARACTERS
            .iter()
            .zip(characters::PART_MASKS[part])
            .filter_map(|(&table, member)| member.then_some(table))
            .collect()
    };
    vec![
        (
            "division status side",
            division_presentation::STATUS_TABLES.to_vec(),
        ),
        ("stability", country_stability::TABLES.to_vec()),
        ("war overview", war_overview::TABLES.to_vec()),
        (
            "raid target statuses",
            raid_target_statuses::TABLES.to_vec(),
        ),
        ("supply trains", supply_trains::TABLES.to_vec()),
        (
            "industrial organisations",
            industrial_organisations::TABLES.to_vec(),
        ),
        (
            "production modifiers",
            production_modifiers::TABLES.to_vec(),
        ),
        ("research", research.to_vec()),
        ("fronts", vec![TableId::Fronts]),
        ("air statistics", crate::decode::AIR_STATS.to_vec()),
        ("sunk ships", crate::decode::SUNK.to_vec()),
        ("project programs", crate::decode::PROJECTS.to_vec()),
        ("world tension", crate::decode::SESSION.to_vec()),
        ("unit intel", crate::decode::INTEL.to_vec()),
        ("country leader roles", character_part(1)),
        ("advisor roles", character_part(2)),
        ("production rest", crate::decode::PRODUCTION_REST.to_vec()),
        (
            "construction rest",
            crate::decode::CONSTRUCTION_REST.to_vec(),
        ),
        ("projects family", projects::TABLES.to_vec()),
        (
            "stationed units",
            vec![TableId::TheatreAreas, TableId::StationedUnits],
        ),
        ("combat progress", combat_progress::TABLES.to_vec()),
        ("peace bids", peace_bids::TABLES.to_vec()),
        ("logistics", logistics::TABLES.to_vec()),
        ("country factories", country_factories::TABLES.to_vec()),
        ("hired advisors", hired_advisors::TABLES.to_vec()),
        (
            "doctrines",
            collector_slow_tables::doctrines::TABLES.to_vec(),
        ),
        ("occupation", vec![TableId::StateResistance]),
        (
            "faction goals",
            collector_factions::faction_goals::TABLES.to_vec(),
        ),
        (
            "faction companions",
            collector_factions::faction_companions::TABLES.to_vec(),
        ),
        ("operation outcomes", operation_outcomes::TABLES.to_vec()),
    ]
}

#[cfg(any(test, feature = "test-support"))]
/// # Safety
/// Run only in an isolated fixture that owns the collector's process-wide observer and allocation
/// state.
pub unsafe fn capture_live_shape(captured: &mut Captured) {
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

#[cfg(any(test, feature = "test-support"))]
/// # Safety
/// Run only in an isolated fixture that owns the collector's process-wide observer and allocation
/// state.
pub unsafe fn output_allocation_failure_exercise(arm: impl FnOnce(), disarm: impl FnOnce()) {
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
    let scope = collector_memory::terminal::enter(collector_memory::terminal::Phase::Spool);
    for hour in 0..3 {
        let copied = usize::from(hour != 0);
        let drained = collector_observers::output_log::Drained {
            epoch: 1,
            first: (hour - 1).max(0) as u64,
            cursor: hour as u64,
            copied,
            pending: copied,
        };
        let construction_entry = collector_output::construction_ic::Entry {
            line: 17,
            amount: 40,
            ..Default::default()
        };
        let production_entry = collector_output::line_output::Entry {
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
        if collector_memory::terminal::failed() {
            break;
        }
        production_totals.assemble(1, hour, &production, &mut tables);
        if collector_memory::terminal::failed() {
            break;
        }
        construction_totals
            .written(construction_totals.receipt(&tables[TableId::ConstructionLineOutput.index()]));
        production_totals
            .written(production_totals.receipt(&tables[TableId::ProductionLineOutput.index()]));
    }
    drop(scope);
    disarm();
}

#[cfg(test)]
mod naval_reserved_land_tests;

#[cfg(test)]
mod plans_late_worker_tests;

#[cfg(test)]
mod presentation_worker_tests;

#[cfg(test)]
mod naval_mission_publication_tests;
