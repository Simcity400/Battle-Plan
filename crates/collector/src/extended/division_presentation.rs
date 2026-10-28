//! Hourly division companions. Reuses the existing authenticated source census;
//! movement and counter reads retain separate costs and outcomes. The
//! commanders' retreat bans (DivisionRetreatBans) are read for every division
//! each hour, carrying each commander's bans while no forwarded writer
//! touched it ([`crate::retreat_ban_changes`]). The land movement speed's
//! shared operands (MovementStateTerms, CoreSupplyAccess,
//! CountryUnitSpeedEntries, RiverRelations, DivisionTransportSpeeds and
//! DivisionStatus.combat_movement) run beside the status pass ([`speed_terms`]).
use super::production::Sources;
use crate::{
    projection::{Projection, project_table, scalar_table},
    sampling::qpc,
    spooling::unavailable,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native::{
    Memory, carmy, carmy::DivisionSeed, ccountry, cname_group_member,
    corders_group_member::MembershipCapacity, cprovince, walk::Outcome,
};
use engine_naval_history::cnaval_unit_transfer::SpeedInputs as TransportInputs;
use replay_data::rows::{
    DivisionCamouflage, DivisionMovementRow, DivisionPanelStatsRow, DivisionRetreatBanRow,
    DivisionStatusRow, NameOperands, Recorded, Retired, TableId, Witnessed, tables,
};
use replay_frame::frame::TableInput;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

mod daily_status;
pub(crate) mod held_store;
mod speed_terms;
mod status_work;
pub(crate) use status_work::StatusWork;

/// One final-status range per available worker, sized from the live census.
pub(crate) const MAX_STATUS_PARTS: usize = crate::acquisition::MAX_WORKERS;
const STATUS_PART_ENTRIES: usize = 512;

pub(crate) fn status_parts_for(divisions: usize, workers: usize) -> usize {
    divisions
        .div_ceil(STATUS_PART_ENTRIES)
        .clamp(1, workers.max(1))
}

pub const N: usize = 7 + speed_terms::N;
/// The passes inside the DivisionStatus acquisition span.
pub(crate) const PHASES: usize = 23;
pub(crate) const PHASE_NAMES: [&str; PHASES] = [
    "status",
    "dig_in",
    "naming",
    "prepare_sources",
    "prepare_certify",
    "prepare_native",
    "sources_ordered",
    "sources_prune",
    "sources_variants",
    "sources_publish",
    "certify_setup",
    "certify_ordered",
    "certify_finish",
    "prepare_attack_plan",
    "decode_sources",
    "decode_attack",
    "decode_stitch",
    "decode_scalars",
    "decode_speed_terms",
    "decode_movement",
    "sources_owned_gate",
    "sources_fallback",
    "certification_native_fallback",
];
// The local-refusal count is a subset of owned admission, not another pass.
static SOURCE_ADMISSIONS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
pub(crate) fn source_admission_data() -> impl serde::Serialize {
    let [owned, local_refusal, ordered, unavailable] =
        SOURCE_ADMISSIONS
            .each_ref()
            .map(|count| count.load(Ordering::Relaxed));
    crate::status::status!({
        "owned": owned,
        "owned_local_province_refusal": local_refusal,
        "ordered": ordered,
        "unavailable": unavailable,
    })
}
pub const TABLES: [TableId; N] = [
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::DivisionRetreatBans,
    TableId::DivisionLandInputs,
    TableId::DivisionHistory,
    TableId::DivisionPanelSources,
    TableId::DivisionPanelStats,
    speed_terms::TABLES[0],
    speed_terms::TABLES[1],
    speed_terms::TABLES[2],
    speed_terms::TABLES[3],
    speed_terms::TABLES[4],
];
/// Every table but DivisionMovement: the status side, the speed terms and the
/// movement pass's land and dig-in inputs, which run only if one is recorded.
pub const STATUS_TABLES: [TableId; N - 1] = [
    TableId::DivisionStatus,
    TableId::DivisionLandInputs,
    TableId::DivisionRetreatBans,
    TableId::DivisionHistory,
    TableId::DivisionPanelSources,
    TableId::DivisionPanelStats,
    speed_terms::TABLES[0],
    speed_terms::TABLES[1],
    speed_terms::TABLES[2],
    speed_terms::TABLES[3],
    speed_terms::TABLES[4],
];
// Both depend on the shared division census; the combined source read group in
// extended::read_groups includes their individually measured costs.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

static WITHDRAWAL_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static MOVEMENT_PRIORITY_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static ORGANISATION_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BASE_SPEED_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TRANSPORT_SPEED_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// The land movement speed operands' cumulative reads, carried rows and
/// refusals per family ([`speed_terms`]).
pub(crate) fn speed_term_data() -> impl serde::Serialize {
    speed_terms::status_data()
}

pub(crate) fn movement_arrow_data() -> impl serde::Serialize {
    crate::status::status!({
        "withdrawal_refused": WITHDRAWAL_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "movement_priority_refused": MOVEMENT_PRIORITY_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "organisation_refused": ORGANISATION_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "base_speed_refused": BASE_SPEED_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "transport_speed_refused": TRANSPORT_SPEED_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "scope": "cumulative field-local division movement scalar refusals (arrow style and the base speed and conditional exact organisation of divisions with a route); established division movement remains available",
    })
}

// One scalar projection for the unchanged-row comparison and the owned row.
// Stepped fields are rounded to their display steps (replay_data::display), as the
// baseline holds them.
macro_rules! status_fields {
    ($row:ident, $apply:ident, [$($args:expr),* $(,)?] $(,)?) => {
        $apply!([$($args),*], {
            division: $row.division,
            maximum_hit_points: DivisionStatusRow::MAXIMUM_STEP.round($row.maximum_hit_points),
            maximum_organisation: DivisionStatusRow::MAXIMUM_STEP.round($row.maximum_organisation),
            organisation_exact: Recorded::Value($row.movement_organisation.flatten().map(|pair| pair[0])),
            maximum_organisation_exact: Recorded::Value($row.movement_organisation.flatten().map(|pair| pair[1])),
            entrenchment: $row.entrenchment,
            entrenchment_cap: $row.entrenchment_cap,
            supply_ratio: DivisionStatusRow::SUPPLY_RATIO_STEP.round($row.supply_ratio),
            attrition_value: $row.attrition_value,
            attrition_threshold: $row.attrition_threshold,
            exercising: $row.exercising,
            has_order: $row.has_order,
            invalid_orders: $row.invalid_orders,
            strength_factor: $row.strength_factor,
            model_number: $row.model_number,
            move_flags: $row.move_flags
                | (u8::from($row.movement_organisation.is_some_and(|pair| pair.is_some())) * DivisionStatusRow::MOVE_LOW_ORGANISATION)
                | (u8::from($row.movement_organisation.is_none()) * DivisionStatusRow::MOVE_ORGANISATION_REFUSED),
        })
    };
}

macro_rules! compare_status_fields {
    ([$previous:expr], {$first:ident: $initial:expr, $($field:ident: $value:expr),* $(,)?}) => {
        $initial == $previous.$first $(&& $value == $previous.$field)*
    };
}

macro_rules! project_status_fields {
    ([$row:expr, $supply_grace:expr, $naming:expr, $supply:expr, $dig_in:expr, $block:expr, $name:expr, $attack_stats:expr, $witness:expr, $movement:expr, $experience:expr, $pending_template:expr, $officer:expr, $career:expr, $source:expr, $combat_movement:expr], {$($field:ident: $value:expr),* $(,)?}) => {{
        let block = $block;
        DivisionStatusRow {
            $($field: $value,)*
            acclimatization_maximum: None,
            acclimatization: Vec::new(),
            camouflage: Recorded::Value(camouflage($row.camouflage)),
            border_combat_stats: Recorded::Value($row.border_combat_stats),
            supply_grace_maximum: Recorded::Value($row.supply_grace_maximum),
            supply_grace_remaining: Recorded::Value(DivisionStatusRow::displayed_grace(
                $row.supply_grace_remaining,
                $supply_grace,
            )),
            name_template: Recorded::Value(name_template($row.naming)),
            naming: Recorded::Value($naming),
            country_ordinal: Recorded::Value($row.country_ordinal),
            invalid_orders_reason: Recorded::Value($row.invalid_orders_reason),
            // Playback computes the glyph byte from the value and threshold.
            attrition: replay_data::rows::RetiredU8::default(),
            supply: Recorded::Value($supply),
            attrition_inputs: Recorded::Value(Retired),
            dig_in_factor: Recorded::Value($dig_in),
            name: Recorded::Value($name),
            attack: Recorded::Value(Witnessed($witness)),
            attack_stats: Recorded::Value($attack_stats),
            movement_stats: Recorded::Value($movement),
            experience: Recorded::Value($experience),
            pending_template: Recorded::Value($pending_template),
            officer_experience: Recorded::Value($officer),
            career_counter: Recorded::Value($career),
            panel_source: Recorded::Value($source),
            weather_factor: Recorded::Value($row.weather_factor),
            out_of_supply_days: Recorded::Value($row.out_of_supply_days),
            commander_modifiers: Recorded::Value(block.map(|entries: [Option<i64>; 6]| [entries[0], entries[1], entries[2], entries[4], entries[5]])),
            commander_width_factor: Recorded::Value(block.and_then(|entries| entries[3])),
            combat_movement: Recorded::Value($combat_movement),
            supply_ratio_exact: Recorded::Value(
                DivisionStatusRow::exact_supply(
                    $row.out_of_supply_days,
                    $row.supply_ratio,
                    $row.supply_grace_remaining,
                )
                .map(|[ratio, _]| ratio),
            ),
            supply_grace_exact: Recorded::Value(
                DivisionStatusRow::exact_supply(
                    $row.out_of_supply_days,
                    $row.supply_ratio,
                    $row.supply_grace_remaining,
                )
                .map(|[_, grace]| grace),
            ),
        }
    }};
}

/// The recorded form of what the camouflage category selector takes from the division; a refused
/// read stays None.
fn camouflage(camouflage: Option<carmy::Camouflage>) -> Option<DivisionCamouflage> {
    camouflage.map(|camouflage| DivisionCamouflage {
        category: camouflage.category,
    })
}

/// Whether this build records the name member's template reference, which only
/// the fresh-save oracle reads (`name_template_witness`).
const NAME_TEMPLATE_WITNESS: bool = cfg!(feature = "name_template_witness");

/// The recorded name member template reference: empty unless this is a
/// `name_template_witness` build and the naming operands were read.
fn name_template(naming: Option<cname_group_member::Inputs>) -> Witnessed<u64> {
    Witnessed(
        naming
            .filter(|_| NAME_TEMPLATE_WITNESS)
            .map(|naming| naming.template),
    )
}

#[derive(Clone, Copy, Default)]
struct NamingRead {
    division: u64,
    country_ordinal: Option<u32>,
    inputs: Option<cname_group_member::Inputs>,
    failure: Option<&'static str>,
    cached_name: Option<cname_group_member::Span>,
    cached_failure: Option<&'static str>,
}

#[derive(Default)]
pub(crate) struct CounterWork {
    captured: u64,
    refused: u64,
    hq_captured: u64,
    hq_refused: u64,
    pub(super) supply: super::supply_inputs::Work,
    dig_in: super::dig_in::Work,
}

impl CounterWork {
    pub(crate) fn observe_hq(&mut self, value: Option<i32>) {
        if value.is_some() {
            self.hq_captured = self.hq_captured.saturating_add(1);
        } else {
            self.hq_refused = self.hq_refused.saturating_add(1);
        }
    }

    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "invalid_orders_reason": {
                "captured": self.captured,
                "refused": self.refused,
                "refusal": "division invalid-order reason unreadable",
            },
            "hq_distance": {
                "captured": self.hq_captured,
                "refused": self.hq_refused,
                "refusal": "orders group HQ distance unreadable",
            },
            "supply": self.supply.status(),
            "dig_in_factor": self.dig_in.status(),
        })
    }
}

/// Off-thread accounting keeps every refusal's native reader reason.
#[derive(Default)]
pub(crate) struct NamingWork {
    captured: u64,
    ordinal_captured: u64,
    ordinal_refused: u64,
    refusals: Vec<(&'static str, u64)>,
    required: usize,
    capacity: usize,
    cached_captured: u64,
    cached_refused: u64,
    cached_required: usize,
    cached_capacity: usize,
}

impl NamingWork {
    fn refuse(&mut self, reason: &'static str, count: u64) -> std::io::Result<()> {
        let index = match self.refusals.binary_search_by_key(&reason, |entry| entry.0) {
            Ok(index) => index,
            Err(index) => {
                engine_allocation::reserve(&mut self.refusals, 1)?;
                self.refusals.insert(index, (reason, 0));
                index
            }
        };
        self.refusals[index].1 = self.refusals[index].1.saturating_add(count);
        Ok(())
    }

    fn observe(&mut self, capture: &Captured) -> std::io::Result<()> {
        for row in &capture.naming.first[..capture.naming.len] {
            if row.country_ordinal.is_some() {
                self.ordinal_captured = self.ordinal_captured.saturating_add(1);
            } else {
                self.ordinal_refused = self.ordinal_refused.saturating_add(1);
            }
            if let Some(reason) = row.failure {
                self.refuse(reason, 1)?;
            } else {
                self.captured = self.captured.saturating_add(1);
            }
            if let Some(reason) = row.cached_failure {
                self.cached_refused = self.cached_refused.saturating_add(1);
                self.refuse(reason, 1)?;
            } else if row.cached_name.is_some() {
                self.cached_captured = self.cached_captured.saturating_add(1);
            }
        }
        let shortfall = capture.naming.required.saturating_sub(capture.naming.len);
        if shortfall != 0 {
            self.ordinal_refused = self.ordinal_refused.saturating_add(shortfall as u64);
            self.refuse("division naming row capacity exceeded", shortfall as u64)?;
            self.cached_refused = self.cached_refused.saturating_add(shortfall as u64);
            self.refuse(
                "division cached name row capacity exceeded",
                shortfall as u64,
            )?;
        }
        self.required = capture.names.required;
        self.capacity = capture.names.first.len();
        self.cached_required = capture.cached_names.required;
        self.cached_capacity = capture.cached_names.first.len();
        Ok(())
    }

    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "country_ordinal": {
                "captured": self.ordinal_captured,
                "refused": self.ordinal_refused,
                "refusal": "division country roster ordinal not captured",
            },
            "captured": self.captured,
            "refused": self.refusals.iter().fold(0_u64, |sum, (_, count)| sum.saturating_add(*count)),
            "refusals": crate::status::map(self.refusals.iter().map(|&(reason, count)| (reason, count))),
            "bytes_required": self.required,
            "bytes_capacity": self.capacity,
            "cached_name": { "captured": self.cached_captured, "refused": self.cached_refused,
                "bytes_required": self.cached_required, "bytes_capacity": self.cached_capacity },
            "scope": "hourly generator operands and exact retained name cache for admitted DivisionStatus rows; each refused field is absent while the status prefix remains available",
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub ticks: [u64; N],
}

impl CaptureReport {
    pub(crate) fn merge_movement(&mut self, movement: Self) {
        self.ticks[0] = movement.ticks[0];
        self.ticks[1] += movement.ticks[1];
        self.ticks[3] = movement.ticks[3];
    }

    pub(crate) fn merge_speed_terms(&mut self, speed_terms: Self) {
        self.ticks[7..].copy_from_slice(&speed_terms.ticks[7..]);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    movement: usize,
    status: usize,
    /// DivisionRetreatBans rows: divisions whose commander bans retreat.
    bans: usize,
    paths: usize,
    name_bytes: usize,
    name_groups: usize,
    cached_name_bytes: usize,
    geometry: cprovince::GeometryCapacities,
    /// Shared DivisionStatus membership storage prepared for pooled packets.
    /// Worker acquisition retains larger original demand in the same pass.
    membership: MembershipCapacity,
    /// Ordered supply-footer origin rows across the census (DivisionStatus).
    supply_origins: usize,
    combat_references: usize,
}

collector_primitives::capacity_max!(Capacities {
    movement,
    status,
    bans,
    paths,
    name_bytes,
    name_groups,
    cached_name_bytes,
    geometry { provinces, neighbour_entries },
    membership { queries, groups, roots, chunks },
    supply_origins,
    combat_references,
});
impl Default for Capacities {
    fn default() -> Self {
        Self {
            movement: 0,
            status: 0,
            bans: 0,
            paths: 0,
            name_bytes: 0,
            name_groups: 0,
            cached_name_bytes: 0,
            geometry: cprovince::GeometryCapacities {
                provinces: 0,
                neighbour_entries: 0,
            },
            membership: MembershipCapacity::default(),
            supply_origins: 0,
            combat_references: 0,
        }
    }
}
impl Capacities {
    #[cfg(test)]
    pub(super) fn status_fixture(status: usize) -> Self {
        Self {
            status,
            membership: MembershipCapacity {
                queries: status,
                ..Default::default()
            },
            ..Self::default()
        }
    }

    /// Each army's name member supplies the override or resolved template
    /// name-group key.
    /// This off-hour hint supplies storage only, never recorded naming values.
    pub(crate) fn presize_names(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let count =
            crate::country_scale::table_demand::rows(carmy::walk(memory, base, state, &mut []))?;
        crate::army_stat_changes::demand_divisions(count);
        let mut rows = engine_allocation::default_vec::<carmy::Row>(count)
            .map_err(|_| "division name sizing allocation failed")?;
        let mut groups = cname_group_member::GroupKeys::try_with_capacity(count, count)
            .map_err(|_| "division name sizing allocation failed")?;
        let mut names = cname_group_member::Names {
            bytes: &mut [],
            len: 0,
            required: 0,
        };
        let mut cached = 0usize;
        let mut failure = None;
        let outcome = carmy::walk_with(memory, base, state, &mut rows, |army, _| {
            let result = army
                .checked_add(engine_native::carmy_combat_inputs::NAME_GROUP_MEMBER)
                .and_then(|address| engine_native::read::<u64>(memory, address))
                .and_then(|member| usize::try_from(member).ok())
                .filter(|member| *member != 0)
                .ok_or("division name sizing member unavailable");
            let result = result.and_then(|member| {
                let input =
                    cname_group_member::inputs(memory, base, member, &mut groups, &mut names);
                match cname_group_member::cached_name_demand(memory, base, member) {
                    Ok(bytes) => {
                        cached = cached
                            .checked_add(bytes)
                            .ok_or("division cached name demand overflow")?
                    }
                    Err(reason) => {
                        failure.get_or_insert(reason);
                    }
                }
                input.map(|_| ())
            });
            if let Err(reason) = result {
                failure.get_or_insert(reason);
            }
        });
        if !matches!(outcome, Outcome::Rows { len } if len == count) {
            return Err("division name sizing census changed or unavailable");
        }
        grow_capacity(&mut self.movement, count);
        grow_capacity(&mut self.status, count);
        grow_capacity(&mut self.bans, count);
        grow_capacity(&mut self.name_groups, count);
        grow_capacity(&mut self.name_bytes, names.required);
        grow_capacity(&mut self.cached_name_bytes, cached);
        failure.map_or(Ok(()), Err)
    }

    /// Every province definition bounds the movement route geometry.
    pub(super) fn raise_for_provinces(&mut self, provinces: usize) {
        self.geometry.provinces = self.geometry.provinces.max(provinces);
    }

    pub(crate) fn raise_for_census(&mut self, demand: carmy::PresentationDemand) {
        grow_capacity(&mut self.status, demand.rows);
        grow_capacity(&mut self.supply_origins, demand.supply_origins);
        grow_capacity(&mut self.cached_name_bytes, demand.cached_name_text);
    }
}
fn status_packet(c: &Capacities) -> carmy::StatusPacket {
    carmy::StatusPacket::with_capacities(carmy::StatusCapacities {
        units: c.status,
        supply_origins: c.supply_origins,
        manpower: 0,
        acclimatization_categories: 0,
        thresholds: 0,
        modifiers: engine_native::modifier_accumulator::SnapshotCapacity {
            nodes: c.status,
            entries: 0,
        },
        membership: c.membership,
    })
    .unwrap_or_else(|_| carmy::StatusPacket::vacant())
}

/// The census rows a pass may read: a count past the rows refuses them all.
fn source_outcome(sources: &Sources<'_, DivisionSeed>) -> Outcome {
    if sources.outcome.len() <= sources.rows.len() {
        sources.outcome
    } else {
        Outcome::Unavailable {
            reason: "division presentation source count invalid",
        }
    }
}

/// Shared speed terms: storage lent to their independent whole-census job.
pub(crate) struct SpeedTermsPass {
    terms: speed_terms::SpeedTerms,
}

impl SpeedTermsPass {
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.terms.allocated_bytes()
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: Sources<'_, DivisionSeed>,
    ) -> CaptureReport {
        let mut report = CaptureReport::default();
        let terms = self
            .terms
            .acquire(memory, base, state, sources.rows, source_outcome(&sources));
        report.ticks[7..].copy_from_slice(&terms);
        report
    }
}

/// DivisionMovement, DivisionLandInputs and DivisionStatus's dig-in factors:
/// storage lent to their job ([`Captured::take_movement`]) beside the status
/// and speed-term passes. They share only the census with them and read the
/// same parked state; the dig-in pass reads every census division and is cut
/// to the status pass's rows when the storage returns.
pub(crate) struct MovementPass {
    land: super::division_land::Captured,
    land_ticks: u64,
    movement: CaptureRows<carmy::MovementRow>,
    paths: Arena<u32>,
    movement_packet: carmy::MovementPacket,
    transport: std::collections::HashMap<(u32, u32), Result<TransportInputs, &'static str>>,
    transport_inputs: Vec<(usize, Option<TransportInputs>)>,
    geometry: cprovince::GeometryPacket,
    dig_in: super::dig_in::Captured,
    dig_in_ticks: u64,
    owned_pending: bool,
    /// False when the capture mode leaves out [`STATUS_TABLES`]: the land and
    /// dig-in walks, which only they read, do not run.
    status: bool,
}

impl MovementPass {
    /// Leave out the land and dig-in walks with the status side.
    pub(crate) fn omit_status(&mut self) {
        self.status = false;
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.land.allocated_bytes()
            + self.movement.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.transport.capacity()
                * (std::mem::size_of::<((u32, u32), Result<TransportInputs, &'static str>)>() + 1)
            + self.transport_inputs.capacity() * size_of::<(usize, Option<TransportInputs>)>()
            + self.movement_packet.allocated_bytes()
            + self.geometry.allocated_bytes()
            + self.dig_in.allocated_bytes()
    }

    /// The movement, land and dig-in terms of the same census.
    /// Only the test fixture omits the sample hour and uses fixed storage.
    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: Sources<'_, DivisionSeed>,
        hour: Option<i32>,
    ) -> CaptureReport {
        let mut report = CaptureReport::default();
        self.land.reset();
        self.land_ticks = 0;
        self.movement.reset();
        self.paths.reset();
        self.dig_in.reset();
        self.dig_in_ticks = 0;
        self.owned_pending = false;
        self.transport_inputs.clear();
        self.movement_packet.defer_route_decode();
        let outcome = source_outcome(&sources);
        let rows = &sources.rows[..outcome.len()];
        if engine_allocation::reserve(&mut self.transport_inputs, rows.len()).is_err() {
            let reason = "division transport observation allocation failed";
            self.movement.outcome = Outcome::Unavailable { reason };
            self.paths.len = 0;
            self.land.fail(reason);
            return report;
        }
        if hour.is_some() {
            self.movement_packet.enable_growth();
            self.geometry.enable_growth();
            if !grow_vec(&mut self.movement.rows, rows.len()) {
                let reason = "presentation row allocation failed";
                self.movement.outcome = Outcome::Unavailable { reason };
                self.paths.len = 0;
                self.land.fail(reason);
                return report;
            }
        }
        if self.status {
            self.land_ticks = self.land.capture(memory, base, rows, outcome);
        } else {
            self.land.fail(super::NOT_RECORDED);
        }
        let _movement_reads = crate::read_accounting::table(TableId::DivisionMovement);
        let start = qpc();
        // A failed geometry root stays the movement rows' own refusal.
        let _ = self.geometry.begin(memory, base, state);
        self.transport.clear();
        let transport = &mut self.transport;
        let transport_inputs = &mut self.transport_inputs;
        let mut ordinal = 0;
        let movement = carmy::capture_movement_inputs_with_extra(
            memory,
            base,
            state,
            rows,
            outcome,
            self.movement.rows.len(),
            self.paths.first.len(),
            &mut self.geometry,
            &mut self.movement_packet,
            hour,
            |seed, row| {
                let index = ordinal;
                ordinal += 1;
                if row.naval_transfer == 0 {
                    return;
                }
                let acquired = (|| {
                    let owner = seed
                        .logical_owner
                        .ok_or("division transport owner unavailable")?;
                    let location = seed
                        .location
                        .ok_or("division transport location unavailable")?;
                    let pointer = location
                        .checked_add(cprovince::STRATEGIC_REGION)
                        .and_then(|at| engine_native::read::<u64>(memory, at))
                        .ok_or("division transport region pointer unavailable")?
                        as usize;
                    let region = engine_native::cstrategic_region::id(memory, base, pointer)?
                        .ok_or("division transport source region absent")?;
                    if !transport.contains_key(&(owner, region)) {
                        engine_allocation::reserve_map(transport, 1)
                            .map_err(|_| "division transport cache allocation failed")?;
                    }
                    *transport.entry((owner, region)).or_insert_with(|| {
                        engine_naval_history::cnaval_unit_transfer::movement_speed_inputs(
                            memory, base, state, owner, pointer,
                        )
                    })
                })();
                if acquired.is_err() {
                    TRANSPORT_SPEED_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                transport_inputs.push((index, acquired.ok()));
            },
        );
        self.movement.outcome = movement.outcome;
        self.owned_pending = true;
        self.paths.len = movement.paths_len;
        self.paths.required = movement.required_paths;
        let movement_ticks = qpc().saturating_sub(start).max(0) as u64;
        drop(_movement_reads);
        if self.status {
            let _status_reads = crate::read_accounting::table(TableId::DivisionStatus);
            let start = qpc();
            self.dig_in
                .capture(memory, base, rows, Outcome::Rows { len: rows.len() });
            self.dig_in_ticks = qpc().saturating_sub(start).max(0) as u64;
        }
        report.ticks[0] = movement_ticks;
        report.ticks[1] = self.dig_in_ticks;
        report.ticks[3] = self.land_ticks;
        report
    }
}

pub struct Captured {
    land: super::division_land::Captured,
    land_ticks: u64,
    movement: CaptureRows<carmy::MovementRow>,
    status: CaptureRows<carmy::StatusRow>,
    bans: CaptureRows<carmy::RetreatBanRow>,
    paths: Arena<u32>,
    naming: Arena<NamingRead>,
    names: Arena<u8>,
    cached_names: Arena<u8>,
    name_groups: cname_group_member::GroupKeys,
    movement_packet: carmy::MovementPacket,
    transport: std::collections::HashMap<(u32, u32), Result<TransportInputs, &'static str>>,
    transport_inputs: Vec<(usize, Option<TransportInputs>)>,
    /// Sparse execution output from the same held pass, installed only after
    /// both Presentation and Late have joined. Never reads native memory.
    order_transport: engine_native::order_execution::UnitPacket,
    geometry: cprovince::GeometryPacket,
    status_packet: carmy::StatusPacket,
    daily_status: daily_status::DailyStatus,
    prepared_status: Option<Arc<carmy::StatusPacket>>,
    status_parts: Vec<carmy::StatusPart>,
    attack_parts: Vec<super::attack_stats::AttackPart>,
    preparation_parts: Vec<super::attack_stats::PreparationPart>,
    preparation_store: Option<Arc<super::attack_stats::ArmyStatStore>>,
    preparation_store_loaned: bool,
    prefetching: bool,
    probes_ready: bool,
    owned_settlement: bool,
    sources_deferred: bool,
    stitch_deferred: bool,
    attack_deferred: bool,
    deferred_store: Option<held_store::Loan>,
    deferred_inputs: Option<Arc<crate::division_census::UnitInputs>>,
    deferred_context: Option<CaptureContext>,
    attack_ready: bool,
    status_part_count: usize,
    pub(crate) status_workers: usize,
    status_prepared: bool,
    dig_in: super::dig_in::Captured,
    /// DivisionStatus.attack_stats and the attack witness
    /// ([`super::attack_stats`]), captured after the status pass.
    attack: super::attack_stats::Captured,
    /// The DivisionStatus span by PHASE_NAMES, including nested ordered
    /// preparation sub-phases. Diagnostics only.
    phase_ticks: [u64; PHASES],
    membership: MembershipCapacity,
    source_capacity: usize,
    source_required: usize,
    decode_pending: bool,
    /// The status side was left out ([`Self::omit_status`]): the decode
    /// projects only the movement pass's rows.
    status_omitted: bool,
    movement_owned_pending: bool,
    /// The live `NMilitary.SUPPLY_GRACE` hours this pass read, which
    /// DivisionStatus.supply_grace_remaining is stored against
    /// ([`DivisionStatusRow::displayed_grace`]); None when unreadable.
    supply_grace: Option<i32>,
    /// The land movement speed's shared operands and each division's combat
    /// movement operand, lent to the independent speed-term pass.
    speed_terms: speed_terms::SpeedTerms,
    #[cfg(feature = "movement_witness")]
    movement_witness: Vec<(u64, Option<i64>)>,
}

impl Captured {
    /// Native queries belong to the held game-thread callback, before either
    /// presentation worker leaves. This entire method and its storage are absent
    /// from ordinary builds.
    #[cfg(feature = "movement_witness")]
    pub(crate) unsafe fn capture_movement_witness(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
    ) {
        self.movement_witness.clear();
        if let Err(error) = engine_allocation::reserve(&mut self.movement_witness, seeds.len()) {
            crate::allocation::observe(&error);
            return;
        }
        for seed in seeds {
            if seed
                .ordinary_route
                .and_then(|h| engine_native::cpdx_array::Array::from_header(h, 4))
                .is_some_and(|route| route.count == 0)
            {
                continue;
            }
            // SAFETY: sampling calls us only on its held game-thread boundary.
            let speed =
                unsafe { carmy::movement_witness::first_edge_speed(memory, base, state, seed) };
            self.movement_witness.push((seed.division, speed.ok()));
        }
        self.movement_witness.sort_unstable_by_key(|entry| entry.0);
    }

    fn speed_witness(&self, division: u64) -> Witnessed<Option<i64>> {
        #[cfg(feature = "movement_witness")]
        {
            Witnessed(
                self.movement_witness
                    .binary_search_by_key(&division, |entry| entry.0)
                    .ok()
                    .map(|at| self.movement_witness[at].1),
            )
        }
        #[cfg(not(feature = "movement_witness"))]
        {
            let _ = division;
            Witnessed(None)
        }
    }

    fn speed_witness_matches(
        &self,
        division: u64,
        previous: &Recorded<Option<replay_data::rows::UnitCombatReferences>>,
    ) -> bool {
        let previous = match previous {
            Recorded::Value(Some(packet)) => packet.speed_witness.clone(),
            _ => Witnessed(None),
        };
        previous == self.speed_witness(division)
    }

    #[cfg(test)]
    pub(super) fn capture_status_fixture(
        &mut self,
        memory: &impl Memory,
        seeds: &[DivisionSeed],
        row_capacity: usize,
    ) {
        self.status.outcome = self.status_packet.capture(
            memory,
            0,
            0,
            seeds,
            Outcome::Rows { len: seeds.len() },
            row_capacity,
        );
        self.capture_naming(memory, 0, &seeds[..self.status.outcome.len()]);
        self.decode_pending = true;
    }

    /// Empty ownership placeholder; constructing it never provisions packets.
    pub(crate) fn vacant() -> Self {
        Self {
            land: super::division_land::Captured::new(0),
            land_ticks: 0,
            movement: CaptureRows::new(0),
            status: CaptureRows::new(0),
            bans: CaptureRows::new(0),
            paths: Arena::new(0),
            naming: Arena::new(0),
            names: Arena::new(0),
            cached_names: Arena::new(0),
            name_groups: cname_group_member::GroupKeys::default(),
            movement_packet: carmy::MovementPacket::vacant(),
            transport: Default::default(),
            transport_inputs: Vec::new(),
            order_transport: engine_native::order_execution::UnitPacket::vacant(),
            geometry: cprovince::GeometryPacket::vacant(),
            status_packet: carmy::StatusPacket::vacant(),
            daily_status: Default::default(),
            prepared_status: None,
            status_parts: Vec::new(),
            attack_parts: Vec::new(),
            preparation_parts: Vec::new(),
            preparation_store: None,
            preparation_store_loaned: false,
            prefetching: false,
            probes_ready: false,
            owned_settlement: false,
            sources_deferred: false,
            stitch_deferred: false,
            attack_deferred: false,
            deferred_store: None,
            deferred_inputs: None,
            deferred_context: None,
            attack_ready: false,
            status_part_count: 0,
            status_workers: 1,
            status_prepared: false,
            dig_in: super::dig_in::Captured::new(0),
            attack: Default::default(),
            phase_ticks: [0; PHASES],
            membership: MembershipCapacity::default(),
            source_capacity: 0,
            source_required: 0,
            decode_pending: false,
            status_omitted: false,
            movement_owned_pending: false,
            supply_grace: None,
            speed_terms: speed_terms::SpeedTerms::default(),
            #[cfg(feature = "movement_witness")]
            movement_witness: Vec::new(),
        }
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        let mut movement_packet = carmy::MovementPacket::try_new(c.movement, c.paths)
            .unwrap_or_else(|_| carmy::MovementPacket::vacant());
        // The checked allocator still signals terminal failure. Keep the
        // already-prepared legacy packet if only this optional arena fails.
        let _ = movement_packet.reserve_combat_references(c.combat_references);
        Self {
            land: super::division_land::Captured::new(c.movement.max(c.status)),
            land_ticks: 0,
            movement: CaptureRows::new(c.movement),
            status: CaptureRows::new(c.status),
            bans: CaptureRows::new(c.bans),
            paths: Arena::new(c.paths),
            naming: Arena::new(c.status),
            names: Arena::new(c.name_bytes),
            cached_names: Arena::new(c.cached_name_bytes),
            name_groups: cname_group_member::GroupKeys::try_with_capacity(
                c.name_groups,
                c.name_groups,
            )
            .map_err(crate::allocation::io)
            .unwrap_or_default(),
            movement_packet,
            transport: Default::default(),
            transport_inputs: Vec::new(),
            order_transport: engine_native::order_execution::UnitPacket::vacant(),
            geometry: cprovince::GeometryPacket::try_with_capacities(c.geometry)
                .unwrap_or_default(),
            status_packet: status_packet(c),
            daily_status: Default::default(),
            prepared_status: None,
            status_parts: Vec::new(),
            attack_parts: Vec::new(),
            preparation_parts: Vec::new(),
            preparation_store: None,
            preparation_store_loaned: false,
            prefetching: false,
            probes_ready: false,
            owned_settlement: false,
            sources_deferred: false,
            stitch_deferred: false,
            attack_deferred: false,
            deferred_store: None,
            deferred_inputs: None,
            deferred_context: None,
            attack_ready: false,
            status_part_count: 0,
            status_workers: 1,
            status_prepared: false,
            dig_in: super::dig_in::Captured::new(c.status),
            attack: Default::default(),
            phase_ticks: [0; PHASES],
            membership: c.membership,
            source_capacity: c.movement.max(c.status),
            source_required: 0,
            decode_pending: false,
            status_omitted: false,
            movement_owned_pending: false,
            supply_grace: None,
            speed_terms: speed_terms::SpeedTerms::default(),
            #[cfg(feature = "movement_witness")]
            movement_witness: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        #[cfg(feature = "movement_witness")]
        self.movement_witness.clear();
        self.land.reset();
        self.land_ticks = 0;
        self.movement.reset();
        self.paths.reset();
        self.dig_in.reset();
        self.speed_terms.reset();
        self.movement_owned_pending = false;
        self.reset_status();
    }

    /// Everything but the movement and speed-term storage, which their jobs
    /// reset ([`MovementPass::capture`], [`SpeedTermsPass::capture`]).
    fn reset_status(&mut self) {
        self.sources_deferred = false;
        self.stitch_deferred = false;
        self.attack_deferred = false;
        self.deferred_store = None;
        self.deferred_inputs = None;
        self.deferred_context = None;
        self.status_prepared = false;
        self.status_part_count = 0;
        self.prefetching = false;
        self.probes_ready = false;
        self.attack_ready = false;
        self.supply_grace = None;
        self.daily_status.reset();
        self.status.reset();
        self.bans.reset();
        self.naming.reset();
        self.names.reset();
        self.cached_names.reset();
        self.attack.reset();
        self.attack.history = None;
        self.attack.sources = None;
        self.attack.selected_sources.clear();
        self.phase_ticks = [0; PHASES];
        self.order_transport.reset();
        // Engine acquisition resets each owned packet in its measured scope.
        self.decode_pending = false;
        self.status_omitted = false;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.land.allocated_bytes()
            + {
                #[cfg(feature = "movement_witness")]
                {
                    self.movement_witness.capacity() * std::mem::size_of::<(u64, Option<i64>)>()
                }
                #[cfg(not(feature = "movement_witness"))]
                {
                    0
                }
            }
            + self.movement.allocated_bytes()
            + self.status.allocated_bytes()
            + self.bans.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.naming.allocated_bytes()
            + self.names.allocated_bytes()
            + self.cached_names.allocated_bytes()
            + self.name_groups.allocated_bytes()
            + self.transport.capacity()
                * (std::mem::size_of::<((u32, u32), Result<TransportInputs, &'static str>)>() + 1)
            + self.transport_inputs.capacity() * size_of::<(usize, Option<TransportInputs>)>()
            + self.movement_packet.allocated_bytes()
            + self.order_transport.allocated_bytes()
            + self.geometry.allocated_bytes()
            + self.status_packet.allocated_bytes()
            + self.daily_status.allocated_bytes()
            + self.prepared_status.as_ref().map_or(0, |packet| {
                packet.allocated_bytes()
                    + std::mem::size_of::<carmy::StatusPacket>()
                    + 2 * std::mem::size_of::<usize>()
            })
            + self.preparation_parts.capacity()
                * std::mem::size_of::<super::attack_stats::PreparationPart>()
            + self
                .preparation_parts
                .iter()
                .map(super::attack_stats::PreparationPart::allocated_bytes)
                .sum::<usize>()
            + self.preparation_store.as_ref().map_or(0, |store| {
                store.allocated_bytes()
                    + std::mem::size_of::<super::attack_stats::ArmyStatStore>()
                    + 2 * std::mem::size_of::<usize>()
            })
            + self.attack_parts.capacity() * std::mem::size_of::<super::attack_stats::AttackPart>()
            + self
                .attack_parts
                .iter()
                .map(super::attack_stats::AttackPart::allocated_bytes)
                .sum::<usize>()
            + self.status_parts.capacity() * std::mem::size_of::<carmy::StatusPart>()
            + self
                .status_parts
                .iter()
                .map(carmy::StatusPart::allocated_bytes)
                .sum::<usize>()
            + self.dig_in.allocated_bytes()
            + self.attack.allocated_bytes()
            + self.speed_terms.allocated_bytes()
    }

    pub fn grow(&mut self, c: &mut Capacities) {
        self.movement.grow(&mut c.movement);
        self.status.grow(&mut c.status);
        self.bans.grow(&mut c.bans);
        self.dig_in.grow(&mut c.status);
        grow_capacity(
            &mut c.supply_origins,
            self.status_packet.required_capacities().supply_origins,
        );
        self.status_packet.raise_supply_origins(c.supply_origins);
        let membership = self.status_packet.membership_capacity();
        c.membership.queries = c.membership.queries.max(membership.queries);
        c.membership.groups = c.membership.groups.max(membership.groups);
        c.membership.roots = c.membership.roots.max(membership.roots);
        c.membership.chunks = c.membership.chunks.max(membership.chunks);
        c.membership = c
            .membership
            .grown(self.status_packet.required_capacities().membership);
        self.membership = c.membership;
        self.paths.grow(&mut c.paths);
        self.names.grow(&mut c.name_bytes);
        self.cached_names.grow(&mut c.cached_name_bytes);
        let geometry_required = self.geometry.required_capacities();
        grow_capacity(&mut c.geometry.provinces, geometry_required.provinces);
        grow_capacity(
            &mut c.geometry.neighbour_entries,
            geometry_required.neighbour_entries,
        );
        let _ = self.geometry.grow(c.geometry);
        let _ = self.movement_packet.reserve(c.movement, c.paths);
        grow_capacity(
            &mut c.combat_references,
            self.movement_packet.combat_references_required(),
        );
        let _ = self
            .movement_packet
            .reserve_combat_references(c.combat_references);
        self.reserve_sources(
            self.source_capacity
                .max(self.source_required)
                .max(c.movement)
                .max(c.status),
        );
    }

    /// Prepare primary packet storage for the complete shared source census.
    pub(crate) fn reserve_sources(&mut self, capacity: usize) {
        self.land.reserve(capacity);
        self.source_capacity = self.source_capacity.max(capacity);
        let _ = self
            .movement_packet
            .reserve(self.source_capacity, self.paths.first.len());
        let _ = self
            .status_packet
            .grow(self.source_capacity, self.membership);
        grow_vec(&mut self.naming.first, self.source_capacity);
        self.dig_in.reserve_rows(self.source_capacity);
    }

    /// Lend the movement pass's storage to its own job; the status pass
    /// runs beside it on this capture. Allocation-free placeholders stand in.
    pub(crate) fn take_movement(&mut self) -> MovementPass {
        MovementPass {
            land: std::mem::replace(&mut self.land, super::division_land::Captured::new(0)),
            land_ticks: std::mem::take(&mut self.land_ticks),
            movement: std::mem::replace(&mut self.movement, CaptureRows::new(0)),
            paths: std::mem::replace(&mut self.paths, Arena::new(0)),
            movement_packet: std::mem::replace(
                &mut self.movement_packet,
                carmy::MovementPacket::vacant(),
            ),
            transport: std::mem::take(&mut self.transport),
            transport_inputs: std::mem::take(&mut self.transport_inputs),
            geometry: std::mem::replace(&mut self.geometry, cprovince::GeometryPacket::vacant()),
            dig_in: std::mem::replace(&mut self.dig_in, super::dig_in::Captured::new(0)),
            dig_in_ticks: 0,
            owned_pending: std::mem::take(&mut self.movement_owned_pending),
            status: true,
        }
    }

    /// Return the lent storage once both passes have run: the dig-in rows
    /// are cut to the status pass's.
    pub(crate) fn restore_movement(&mut self, pass: MovementPass) {
        let MovementPass {
            land,
            land_ticks,
            movement,
            paths,
            movement_packet,
            transport,
            transport_inputs,
            geometry,
            dig_in,
            dig_in_ticks,
            owned_pending,
            status: _,
        } = pass;
        self.land = land;
        self.land_ticks = land_ticks;
        self.movement = movement;
        self.paths = paths;
        self.movement_packet = movement_packet;
        self.transport = transport;
        self.transport_inputs = transport_inputs;
        self.geometry = geometry;
        self.dig_in = dig_in;
        self.movement_owned_pending = owned_pending;
        self.phase_ticks[1] = dig_in_ticks;
    }

    /// Lend the unchanged whole-census speed pass beside status and movement.
    pub(crate) fn take_speed_terms(&mut self) -> SpeedTermsPass {
        SpeedTermsPass {
            terms: std::mem::take(&mut self.speed_terms),
        }
    }

    pub(crate) fn restore_speed_terms(&mut self, pass: SpeedTermsPass) {
        self.speed_terms = pass.terms;
    }

    /// The previous sparse output becomes the execution owner's reusable spare.
    /// Its reset makes a failed/cancelled hour incapable of carrying old values.
    pub(crate) fn install_order_transport(
        &mut self,
        execution: &mut engine_native::order_execution::Prepared,
    ) {
        self.order_transport.reset();
        execution.swap_units(&mut self.order_transport);
    }

    pub(crate) fn order_transport_storage(&self) -> (usize, Outcome) {
        (
            self.order_transport.allocated_bytes(),
            self.order_transport.outcome(),
        )
    }

    fn order_transport(
        &self,
        division: u64,
    ) -> Option<replay_data::rows::DivisionOrderTransportInputs> {
        let rows = self.order_transport.rows();
        if !matches!(self.order_transport.outcome(), Outcome::Rows { len } if len == rows.len()) {
            return None;
        }
        let inputs = rows
            .binary_search_by_key(&division, |row| row.division)
            .ok()
            .and_then(|index| rows[index].inputs)?;
        Some(replay_data::rows::DivisionOrderTransportInputs {
            naval_weight: inputs.naval_weight,
            paradrop_eligible: inputs.paradrop_eligible,
            paradrop_modifiers: inputs.paradrop_modifiers.map(|modifiers| {
                replay_data::rows::ParadropTransportModifiers {
                    country_term: modifiers.country_term,
                    commander_term: modifiers.commander_term,
                }
            }),
        })
    }

    /// Reuse the hourly template icon walk's census; no extra native walk.
    /// A full memo costs fresh reads, so an unreadable census cannot drop names.
    pub(crate) fn grow_name_groups(&mut self, c: &mut Capacities, templates: usize) {
        grow_capacity(&mut c.name_groups, templates);
        let current = self.name_groups.capacities();
        if c.name_groups != 0
            && c.name_groups
                .checked_next_power_of_two()
                .is_some_and(|wanted| current.into_iter().any(|capacity| capacity < wanted))
            && let Ok(groups) =
                cname_group_member::GroupKeys::try_with_capacity(c.name_groups, c.name_groups)
                    .map_err(crate::allocation::io)
        {
            self.name_groups = groups;
        }
    }

    pub(crate) fn observe_naming(&self, work: &mut NamingWork) -> std::io::Result<()> {
        work.observe(self)
    }

    pub(crate) fn observe_counter_inputs(&self, work: &mut CounterWork) -> std::io::Result<()> {
        for row in &self.status.rows[..self.status.outcome.len()] {
            let (inputs, failure, _) = self.supply_sample(row);
            work.supply.observe(inputs, failure);
            if row.invalid_orders != 0 {
                if row.invalid_orders_reason.is_some() {
                    work.captured = work.captured.saturating_add(1);
                } else {
                    work.refused = work.refused.saturating_add(1);
                }
            }
        }
        work.dig_in.observe(&self.dig_in)
    }

    /// [`Self::capture_naming_holding`] without the `army_stat` store: every
    /// member is read.
    #[cfg(test)]
    fn capture_naming(&mut self, memory: &impl Memory, base: usize, seeds: &[DivisionSeed]) {
        self.capture_naming_holding(
            memory,
            base,
            seeds,
            None,
            &crate::army_stat_changes::Observer,
        );
    }

    /// The division's name member pointer leads to the naming generator. Keep its language-neutral
    /// operands and, independently, the exact cached name (`cname_group_member::CACHED_NAME`).
    /// Ordering, binding and rename writers refresh the member, as do template, load and pre-hourly
    /// active-stat updates. Inputs therefore read hourly. One copy of the member span serves both
    /// the generator operands and the cache header; the two arenas keep their refusals independent,
    /// and a span that cannot be copied leaves the cache to its own reads.
    ///
    /// With the `army_stat` store, a member no hooked writer touched since the
    /// Held read  (`engine_mutations::army_stat_mutations`: `cached-name refresh`, and the
    /// country-wide refresh, which moves the marks' epoch) is not read: its
    /// held span and cache text stand in for the reads, and the inputs are
    /// derived from the span as from a read (the template's group and a heap
    /// override are still read). The hour's take ([`super::attack_stats`])
    /// precedes the member's reads, and a member not carried is registered
    /// before them.
    fn capture_naming_holding(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        mut store: Option<&mut super::attack_stats::ArmyStatStore>,
        changes: &dyn crate::army_stat_changes::Changes,
    ) {
        self.naming.reset();
        self.names.reset();
        self.name_groups.reset();
        self.cached_names.reset();
        if let Some(store) = store.as_deref_mut() {
            store.begin_names();
        }
        self.naming.required = seeds.len();
        if !grow_vec(&mut self.naming.first, seeds.len()) {
            return;
        }
        let mut batch = cname_group_member::MemberBatch::default();
        let mut held_indices = [None; 64];
        for (index, (seed, row)) in seeds.iter().zip(&mut self.naming.first).enumerate() {
            if index.is_multiple_of(64) {
                held_indices.fill(None);
                let mut pointers = [0_usize; 64];
                let end = (index + 64).min(seeds.len());
                for (slot, pending) in seeds[index..end].iter().enumerate() {
                    let Some(pointer) = pending.name_member.filter(|&at| at != 0) else {
                        continue;
                    };
                    // Decide eligibility once; the store keeps the member span and text.
                    let held = match store.as_deref_mut() {
                        Some(store) if self.attack.carryable => store
                            .held_name(index + slot, pending.division)
                            .filter(|&at| {
                                store.held_name_record(at).member == pointer
                                    && !self.attack.touched(pointer)
                            }),
                        _ => None,
                    };
                    held_indices[slot] = held;
                    if held.is_none() {
                        pointers[slot] = pointer;
                    }
                }
                batch.capture(memory, &pointers[..end - index]);
            }
            *row = NamingRead {
                division: seed.division,
                country_ordinal: seed.country_ordinal,
                ..NamingRead::default()
            };
            let pointer = seed
                .name_member
                .ok_or("division name member pointer unreadable")
                .and_then(|pointer| {
                    if pointer == 0 {
                        Err("division name member pointer null")
                    } else {
                        Ok(pointer)
                    }
                });
            // The hour's take ([`super::attack_stats::Captured::certify`])
            // certifies a held member registered in the held capture's epoch
            // and untouched since; any other member is registered before its
            // read.
            let held = held_indices[index % 64]
                .and_then(|at| store.as_deref().map(|store| store.held_name_record(at)));
            let registered = match (held, pointer, store.is_some()) {
                (Some(_), _, _) => true,
                (None, Ok(pointer), true) => changes.register(pointer),
                _ => false,
            };
            let member = pointer.map(|pointer| match held {
                Some(held) => (pointer, Ok(held.span)),
                None => (pointer, batch.member(memory, index % 64, pointer)),
            });
            let result = member.and_then(|(pointer, span)| {
                cname_group_member::inputs_growing(
                    memory,
                    base,
                    pointer,
                    &span?,
                    &mut self.name_groups,
                    &mut self.names.first,
                    &mut self.names.len,
                    &mut self.names.required,
                )
            });
            match result {
                Ok(cname_group_member::Naming::Row(inputs)) => {
                    self.names.len += inputs.custom.1;
                    // The reader shows the programmatic name override when
                    // the resolved group is null.
                    row.inputs = Some(inputs);
                }
                Ok(cname_group_member::Naming::DoesNotFit) => {
                    row.failure = Some("division naming bytes do not fit arena");
                }
                Err(reason) => row.failure = Some(reason),
            }
            let cached = match (member, held, store.as_deref()) {
                (_, Some(held), Some(store)) => cname_group_member::held_cached_name_growing(
                    store.held_name_text(held),
                    &mut self.cached_names.first,
                    self.cached_names.len,
                    &mut self.cached_names.required,
                ),
                (Ok((_, Ok(span))), _, _) => cname_group_member::cached_name_from_member_growing(
                    memory,
                    base,
                    &span,
                    &mut self.cached_names.first,
                    &mut self.cached_names.len,
                    &mut self.cached_names.required,
                ),
                _ => seed
                    .name_member
                    .ok_or("division cached name member pointer unreadable")
                    .and_then(|pointer| {
                        cname_group_member::cached_name_growing(
                            memory,
                            base,
                            pointer,
                            &mut self.cached_names.first,
                            &mut self.cached_names.len,
                            &mut self.cached_names.required,
                        )
                    }),
            };
            match cached {
                Ok(Some(span)) => {
                    row.cached_name = Some(span);
                    self.cached_names.len += span.1;
                }
                Ok(None) => {
                    row.cached_failure = Some("division cached name bytes do not fit arena")
                }
                Err(reason) => row.cached_failure = Some(reason),
            }
            let text = row
                .cached_name
                .map(|(start, len)| &self.cached_names.first[start..start + len]);
            if held.is_some() {
                self.attack.status_counts.names_reused += 1;
            }
            if let (Some(store), true, Ok((pointer, Ok(span))), Some(text)) =
                (store.as_deref_mut(), registered, member, text)
            {
                store.keep_name(seed.division, pointer, &span, text);
            }
            self.naming.len += 1;
        }
    }

    /// [`Self::capture_owned_observed`] without an `army_stat` store.
    #[cfg(test)]
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
    ) -> CaptureReport {
        self.capture_owned_observed(
            memory,
            context,
            sources,
            None,
            &crate::army_stat_changes::Observer,
        )
    }

    /// All three passes on this thread, with the same loans as Presentation,
    /// PresentationMovement and PresentationSpeedTerms.
    #[cfg(test)]
    pub(super) fn capture_owned_observed(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        army_stats: Option<&mut super::attack_stats::ArmyStatStore>,
        changes: &dyn crate::army_stat_changes::Changes,
    ) -> CaptureReport {
        let (rows, outcome) = (sources.rows, sources.outcome);
        let mut pass = self.take_movement();
        let mut terms = self.take_speed_terms();
        let mut report =
            self.capture_status_observed(memory, context, sources, army_stats, changes);
        let speed_terms = terms.capture(
            memory,
            context.base,
            context.game_state,
            Sources { rows, outcome },
        );
        let movement = pass.capture(
            memory,
            context.base,
            context.game_state,
            Sources { rows, outcome },
            Some(context.hour),
        );
        self.restore_movement(pass);
        self.restore_speed_terms(terms);
        report.merge_movement(movement);
        report.merge_speed_terms(speed_terms);
        report
    }

    /// Stage independent leaves, then settle certificates and shared maxima
    /// before lending final-status ranges. Naming and attack keep the same
    /// held store and their original ordered publication.
    pub(crate) fn prepare_status_holding(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        store: Option<&mut super::attack_stats::ArmyStatStore>,
    ) {
        self.prepare_status_observed(
            memory,
            context,
            sources,
            store,
            &crate::army_stat_changes::Observer,
        );
    }

    fn prepare_status_observed(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        mut store: Option<&mut super::attack_stats::ArmyStatStore>,
        changes: &dyn crate::army_stat_changes::Changes,
    ) {
        if !self.prefetching
            && let Some(store) = store.as_deref_mut()
        {
            store.sources.capture_boundary();
        }
        if !self.prefetching && self.begin_preparation(&sources, store.as_deref_mut()) {
            return;
        }
        self.restore_preparation_store(store.as_deref_mut());
        self.probes_ready = self.prefetching;
        self.capture_status_at(
            memory,
            context.base,
            context.game_state,
            sources,
            Some(context.hour),
            store,
            changes,
            true,
        );
        self.probes_ready = false;
        self.prefetching = false;
        // A preparation refusal was already attempted; the final job must not retry.
        self.status_prepared = true;
    }

    /// False selects the original ordered preparation without optional staging.
    fn begin_preparation(
        &mut self,
        sources: &Sources<'_, DivisionSeed>,
        store: Option<&mut super::attack_stats::ArmyStatStore>,
    ) -> bool {
        self.reset_status();
        let divisions = source_outcome(sources).len();
        let count = status_parts_for(divisions, self.status_workers);
        if !grow_vec(&mut self.status.rows, divisions) {
            self.fail_owned_capture("presentation row allocation failed");
            self.status_prepared = true;
            return true;
        }
        let additional = count.saturating_sub(self.preparation_parts.len());
        if engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.preparation_parts, additional)
        })
        .is_err()
        {
            return false;
        }
        while self.preparation_parts.len() < count {
            self.preparation_parts.push(Default::default());
        }
        for (index, part) in self.preparation_parts[..count].iter_mut().enumerate() {
            part.range = divisions * index / count..divisions * (index + 1) / count;
        }
        if let Some(store) = store {
            let held = self
                .preparation_store
                .get_or_insert_with(|| Arc::new(super::attack_stats::ArmyStatStore::new()));
            std::mem::swap(
                store,
                Arc::get_mut(held).expect("preparation observations joined"),
            );
            self.preparation_store_loaned = true;
        }
        self.prefetching = true;
        self.status_part_count = count;
        true
    }

    /// All range leases must be dropped before the ordered pass, including failures.
    pub(crate) fn restore_preparation_store(
        &mut self,
        store: Option<&mut super::attack_stats::ArmyStatStore>,
    ) {
        if self.preparation_store_loaned {
            std::mem::swap(
                store.expect("preparation store loan"),
                Arc::get_mut(self.preparation_store.as_mut().expect("preparation store"))
                    .expect("preparation observations joined"),
            );
            self.preparation_store_loaned = false;
        }
    }

    pub(crate) fn preparation_pending(&self) -> bool {
        self.prefetching
    }

    pub(crate) fn status_part_count(&self) -> usize {
        self.status_part_count
    }

    pub(crate) fn take_status_part(&mut self, index: usize) -> StatusWork {
        if self.prefetching {
            StatusWork::Preparation {
                store: self
                    .preparation_store_loaned
                    .then(|| Arc::clone(self.preparation_store.as_ref().unwrap())),
                part: std::mem::take(&mut self.preparation_parts[index]),
            }
        } else {
            StatusWork::Status {
                prepared: Arc::clone(self.prepared_status.as_ref().expect("status prepared")),
                part: std::mem::replace(&mut self.status_parts[index], carmy::StatusPart::vacant()),
                attack: self
                    .attack_parts
                    .get_mut(index)
                    .map(std::mem::take)
                    .unwrap_or_default(),
            }
        }
    }

    pub(crate) fn restore_status_part(&mut self, index: usize, work: StatusWork, ticks: u64) {
        match work {
            StatusWork::Preparation { part, .. } => self.preparation_parts[index] = part,
            StatusWork::Status { part, attack, .. } => {
                self.status_parts[index] = part;
                if let Some(destination) = self.attack_parts.get_mut(index) {
                    *destination = attack;
                    self.attack_ready = true;
                }
            }
        }
        self.phase_ticks[0] += ticks;
    }

    /// The status side on the reserved worker, using the immutable source
    /// lease: DivisionStatus and DivisionRetreatBans. The movement pass runs
    /// in its own job beside it ([`MovementPass`]). Membership storage may
    /// grow at the original demanded root or chunk. `army_stats` is the
    /// store the attack pass will take: it certifies every division before
    /// the status pass reads a stat block, and carries the maxima and name
    /// member of each division it holds ([`super::attack_stats`],
    /// [`Self::capture_naming`]).
    pub(crate) fn capture_status_holding(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        army_stats: Option<&mut super::attack_stats::ArmyStatStore>,
    ) -> CaptureReport {
        self.capture_status_observed(
            memory,
            context,
            sources,
            army_stats,
            &crate::army_stat_changes::Observer,
        )
    }

    /// [`Self::capture_status_holding`] observed through `changes`. The
    /// movement, land, dig-in and speed ticks are the movement pass's.
    pub(super) fn capture_status_observed(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        mut army_stats: Option<&mut super::attack_stats::ArmyStatStore>,
        changes: &dyn crate::army_stat_changes::Changes,
    ) -> CaptureReport {
        let (rows, outcome) = (sources.rows, sources.outcome);
        let status = if self.status_prepared && self.status_part_count == 0 {
            self.status_prepared = false;
            0
        } else if self.status_prepared {
            let _status_reads = crate::read_accounting::table(TableId::DivisionStatus);
            let started = qpc();
            let prepared = self.prepared_status.as_mut().expect("status prepared");
            std::mem::swap(
                &mut self.status_packet,
                Arc::get_mut(prepared).expect("status parts joined"),
            );
            self.status_prepared = false;
            self.stitch_deferred = self.owned_settlement;
            self.status.outcome = if self.stitch_deferred {
                self.status_packet
                    .prepare_status_stitch(&self.status_parts[..self.status_part_count])
            } else {
                self.status_packet
                    .stitch_status_parts(&self.status_parts[..self.status_part_count])
            };
            let refused = self.status_parts[..self.status_part_count]
                .iter()
                .map(|part| part.organisation_refused)
                .sum::<u64>();
            ORGANISATION_REFUSALS.fetch_add(refused, std::sync::atomic::Ordering::Relaxed);
            let named = qpc();
            self.capture_naming_holding(
                memory,
                context.base,
                &rows[..self.status.outcome.len()],
                army_stats.as_deref_mut(),
                changes,
            );
            if !self.owned_settlement
                && let Some(store) = army_stats
            {
                self.daily_status.settle(
                    &self.status_packet,
                    store,
                    context.base,
                    rows,
                    matches!(outcome, Outcome::Rows { .. }) && self.status.outcome == outcome,
                );
            }
            let finished = qpc();
            self.phase_ticks[0] += named.saturating_sub(started).max(0) as u64;
            self.phase_ticks[2] = finished.saturating_sub(named).max(0) as u64;
            self.decode_pending = true;
            self.phase_ticks[0] + self.phase_ticks[2]
        } else {
            self.capture_status_at(
                memory,
                context.base,
                context.game_state,
                sources,
                Some(context.hour),
                army_stats,
                changes,
                false,
            )
        };
        let bans = self.capture_bans(
            memory,
            context,
            Sources { rows, outcome },
            &crate::retreat_ban_changes::Observer,
        );
        let mut ticks = [0; N];
        ticks[1] = status;
        ticks[2] = bans;
        CaptureReport { ticks }
    }

    /// DivisionStatus.attack_stats after [`Self::capture_status_holding`]
    /// with the same store: every division of the same census, carried
    /// through `store` where certified ([`super::attack_stats`]), each held
    /// with the status pass's maxima. It reads only stat blocks and, in a
    /// witnessing hour, the terrain database.
    pub(crate) fn capture_attack(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        mut store: Option<&mut super::attack_stats::ArmyStatStore>,
    ) {
        let _reads = crate::read_accounting::table(TableId::DivisionStatus);
        if self.owned_settlement
            && store.is_some()
            && self.attack_ready
            && self
                .attack_parts
                .get(..self.status_part_count)
                .is_some_and(|parts| parts.iter().all(|part| part.is_prepared()))
        {
            self.attack.prepare_witness(
                memory,
                context.base,
                context.hour,
                sources.rows,
                sources.outcome,
            );
            self.attack_deferred = true;
            return;
        }
        if let Some(store) = store.as_deref_mut() {
            let parts = &self.preparation_parts;
            self.attack.settle_certification(
                context.base,
                sources.rows,
                sources.outcome,
                store,
                |index| {
                    parts
                        .iter()
                        .find(|part| part.range.contains(&index))
                        .and_then(|part| part.get(index))
                },
            );
        }
        let status = &self.status_packet;
        let parts = &self.attack_parts;
        let ready = self.attack_ready;
        self.attack.capture_prepared(
            memory,
            context.base,
            context.hour,
            sources.rows,
            sources.outcome,
            store,
            |index| status.unit_maxima(index),
            |index| {
                ready
                    .then(|| {
                        parts
                            .iter()
                            .find(|part| part.range.contains(&index))
                            .and_then(|part| part.get(index))
                    })
                    .flatten()
            },
        );
        self.attack_ready = false;
    }

    /// DivisionRetreatBans for every division of the census: each group's
    /// commander is read unless the store carries its bans
    /// ([`crate::retreat_ban_changes`]). The store is held only after a
    /// complete read.
    fn capture_bans(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        changes: &dyn crate::retreat_ban_changes::Changes,
    ) -> u64 {
        use crate::retreat_ban_changes as changes_family;
        let _reads = crate::read_accounting::table(TableId::DivisionRetreatBans);
        let start = qpc();
        if !grow_vec(&mut self.bans.rows, sources.outcome.len()) {
            self.bans.fail(Outcome::Unavailable {
                reason: "retreat ban row allocation failed",
            });
            changes_family::failed();
            return 0;
        }
        let mut store = changes_family::take_store();
        let mut leaders = changes_family::Leaders::new(&mut store, changes);
        self.bans.outcome = match ccountry::Reader::new(memory, context.base, context.game_state) {
            Err(reason) => Outcome::Unavailable { reason },
            Ok(countries) => {
                // An unlinked id has no owner; a link fault is unresolved,
                // which withholds the read's rows (`retreat_bans`).
                let owner = |raw: u32| countries.link(raw);
                let outcome = if sources.outcome.len() <= sources.rows.len() {
                    sources.outcome
                } else {
                    Outcome::Unavailable {
                        reason: "division presentation source count invalid",
                    }
                };
                self.status_packet.retreat_bans(
                    memory,
                    context.base,
                    sources.rows,
                    outcome,
                    &owner,
                    &mut self.bans.rows,
                    &mut |leader| leaders.held(leader),
                )
            }
        };
        if matches!(self.bans.outcome, Outcome::Rows { .. }) {
            let counts = leaders.finish(self.status_packet.commander_bans());
            changes_family::record(&counts);
            changes_family::return_store(store);
        } else {
            changes_family::failed();
        }
        qpc().saturating_sub(start).max(0) as u64
    }

    /// Finish an accepted worker failure without retrying the live acquisition.
    pub(crate) fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        if self.status_prepared && self.status_part_count != 0 {
            let prepared = self.prepared_status.as_mut().expect("status prepared");
            std::mem::swap(
                &mut self.status_packet,
                Arc::get_mut(prepared).expect("status parts joined"),
            );
        }
        self.status_prepared = false;
        self.prefetching = false;
        self.probes_ready = false;
        self.attack_ready = false;
        self.attack.history = Some(Err(reason));
        self.attack.sources = Some(Err(reason));
        self.attack.selected_sources.clear();
        self.movement.outcome = Outcome::Unavailable { reason };
        self.status.outcome = Outcome::Unavailable { reason };
        self.bans.outcome = Outcome::Unavailable { reason };
        self.paths.len = 0;
        self.naming.len = 0;
        self.names.len = 0;
        self.cached_names.len = 0;
        self.dig_in.reset();
        self.land.fail(reason);
        self.speed_terms.fail(reason);
        self.decode_pending = false;
        self.status_omitted = false;
        CaptureReport::default()
    }

    /// Leave out the status side (status, retreat bans, naming, attack and
    /// panels) and the speed terms, reading nothing: their jobs do not run.
    /// The movement pass's rows, returned by [`Self::restore_movement`], are
    /// projected as they are when the status side is read.
    pub(crate) fn omit_status(&mut self, reason: &'static str) -> CaptureReport {
        self.reset_status();
        self.attack.history = Some(Err(reason));
        self.attack.sources = Some(Err(reason));
        self.status.outcome = Outcome::Unavailable { reason };
        self.bans.outcome = Outcome::Unavailable { reason };
        self.speed_terms.fail(reason);
        self.status_omitted = true;
        self.decode_pending = true;
        CaptureReport::default()
    }

    #[cfg(test)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: Sources<'_, DivisionSeed>,
    ) -> [u64; 2] {
        let (rows, outcome) = (sources.rows, sources.outcome);
        let mut pass = self.take_movement();
        let mut terms = self.take_speed_terms();
        let status = self.capture_status_at(
            memory,
            base,
            state,
            sources,
            None,
            None,
            &crate::army_stat_changes::Observer,
            false,
        );
        terms.capture(memory, base, state, Sources { rows, outcome });
        let movement = pass.capture(memory, base, state, Sources { rows, outcome }, None);
        self.restore_movement(pass);
        self.restore_speed_terms(terms);
        [movement.ticks[0], status + movement.ticks[1]]
    }

    /// The status and naming passes: the DivisionStatus span but the dig-in
    /// pass, which runs in the movement pass's job.
    #[allow(clippy::too_many_arguments)] // The pass's sources, store and observer.
    fn capture_status_at(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: Sources<'_, DivisionSeed>,
        hour: Option<i32>,
        mut army_stats: Option<&mut super::attack_stats::ArmyStatStore>,
        changes: &dyn crate::army_stat_changes::Changes,
        prepare_only: bool,
    ) -> u64 {
        let probes_ready = self.probes_ready;
        let preparation_count = self.status_part_count;
        let preparation_ticks = if probes_ready { self.phase_ticks[0] } else { 0 };
        self.reset_status();
        self.phase_ticks[0] = preparation_ticks;
        self.status_prepared = prepare_only;
        let outcome = source_outcome(&sources);
        let rows = &sources.rows[..outcome.len()];
        self.supply_grace = engine_native::read::<i32>(
            memory,
            base.wrapping_add(engine_land_inputs::carmy_attrition::SUPPLY_GRACE_RVA),
        );
        // Only the test fixture omits the worker's sample hour and uses fixed storage.
        if hour.is_some() {
            self.status_packet.enable_growth();
            if !grow_vec(&mut self.status.rows, rows.len()) {
                self.fail_owned_capture("presentation row allocation failed");
                return 0;
            }
        }
        self.source_required = self.source_required.max(rows.len());
        let _status_reads = crate::read_accounting::table(TableId::DivisionStatus);
        let start = qpc();
        let probes = &self.preparation_parts;
        let ready = probes_ready;
        let prepared = |index: usize| {
            ready
                .then(|| {
                    probes
                        .iter()
                        .find(|part| part.range.contains(&index))
                        .and_then(|part| part.get(index))
                })
                .flatten()
        };
        self.sources_deferred = self.owned_settlement
            && ready
            && !rows.is_empty()
            && army_stats
                .as_deref()
                .is_some_and(|store| store.sources.can_settle_owned())
            && probes[..preparation_count]
                .iter()
                .all(|part| part.can_settle_sources());
        let sources_ready = qpc();
        self.phase_ticks[20] = sources_ready.saturating_sub(start).max(0) as u64;
        let ordered_sources = !self.sources_deferred && army_stats.is_some();
        if self.sources_deferred {
            SOURCE_ADMISSIONS[0].fetch_add(1, Ordering::Relaxed);
            if probes[..preparation_count]
                .iter()
                .any(|part| part.has_local_source_refusal())
            {
                SOURCE_ADMISSIONS[1].fetch_add(1, Ordering::Relaxed);
            }
        } else {
            SOURCE_ADMISSIONS[if army_stats.is_some() { 2 } else { 3 }]
                .fetch_add(1, Ordering::Relaxed);
        }
        self.attack.sources = if self.sources_deferred {
            None
        } else {
            army_stats.as_deref_mut().map(|store| {
                let result = store.sources.capture_staged(
                    memory,
                    base,
                    state,
                    rows,
                    &mut self.attack.selected_sources,
                    |index| prepared(index).and_then(|row| row.selection),
                    |index| {
                        ready
                            .then(|| {
                                probes
                                    .iter()
                                    .find(|part| part.range.contains(&index))
                                    .and_then(|part| part.source_reads())
                            })
                            .flatten()
                    },
                    |handle, pointer| {
                        ready
                            .then(|| {
                                probes
                                    .iter()
                                    .filter_map(|part| part.source_reads())
                                    .find(|reads| reads.has_variant(handle, pointer))
                            })
                            .flatten()
                    },
                );
                self.phase_ticks[6..10].copy_from_slice(&store.sources.phase_ticks());
                if result.is_err() {
                    store.sources.forget();
                    self.attack.selected_sources.clear();
                }
                result
            })
        };
        self.attack
            .selected_sources
            .sort_unstable_by_key(|row| row.0);
        let sources_finished = qpc();
        self.phase_ticks[3] = sources_finished.saturating_sub(start).max(0) as u64;
        if ordered_sources {
            self.phase_ticks[21] = sources_finished.saturating_sub(sources_ready).max(0) as u64;
        }
        self.attack.certify_staged(
            memory,
            base,
            rows,
            outcome,
            army_stats.as_deref_mut(),
            changes,
            prepared,
            self.owned_settlement && prepare_only && ready,
        );
        self.daily_status.prepare(
            &mut self.status_packet,
            army_stats.as_deref_mut(),
            base,
            rows,
            |index| {
                if !ready {
                    return None;
                }
                probes[..preparation_count]
                    .iter()
                    .find(|part| part.range.contains(&index))
                    .and_then(|part| part.daily_supply(index))
                    .cloned()
            },
        );
        let certified = qpc();
        self.phase_ticks[4] = certified.saturating_sub(sources_finished).max(0) as u64;
        self.phase_ticks[10..13].copy_from_slice(&self.attack.certify_ticks());
        if prepare_only {
            self.status.outcome = self.status_packet.prepare_holding(
                memory,
                base,
                state,
                rows,
                outcome,
                &self.attack.held_maxima,
                &self.attack.exercise_experience,
            );
            let native_finished = qpc();
            self.phase_ticks[5] = native_finished.saturating_sub(certified).max(0) as u64;
            if ordered_sources {
                self.phase_ticks[22] = native_finished.saturating_sub(sources_finished).max(0) as u64;
            }
            let count = status_parts_for(rows.len(), self.status_workers);
            let additional = count.saturating_sub(self.status_parts.len());
            if engine_allocation::reserve(&mut self.status_parts, additional).is_err() {
                self.fail_owned_capture("presentation part allocation failed");
                return 0;
            }
            while self.status_parts.len() < count {
                self.status_parts.push(carmy::StatusPart::vacant());
            }
            let additional = count.saturating_sub(self.attack_parts.len());
            if engine_allocation::advisory(|| {
                engine_allocation::reserve(&mut self.attack_parts, additional)
            })
            .is_ok()
            {
                self.attack_parts
                    .resize_with(self.attack_parts.len().max(count), Default::default);
            }
            // Optional attack staging never changes the original ordered
            // acquisition's admission or failure result.
            for (index, part) in self.attack_parts.iter_mut().take(count).enumerate() {
                part.range = rows.len() * index / count..rows.len() * (index + 1) / count;
                part.prepare(
                    rows,
                    hour.expect("status preparation hour"),
                    &self.attack,
                    army_stats.as_deref(),
                    |index| self.status_packet.unit_maxima(index),
                );
            }
            self.phase_ticks[13] = qpc().saturating_sub(native_finished).max(0) as u64;
            let admitted = self.status_packet.prepared_units();
            for (part, work) in self.status_parts[..count].iter_mut().enumerate() {
                work.range = admitted * part / count..admitted * (part + 1) / count;
            }
            let prepared = self
                .prepared_status
                .get_or_insert_with(|| Arc::new(carmy::StatusPacket::vacant()));
            std::mem::swap(
                &mut self.status_packet,
                Arc::get_mut(prepared).expect("status parts joined"),
            );
            self.status_part_count = count;
            self.phase_ticks[0] += qpc().saturating_sub(start).max(0) as u64;
            return self.phase_ticks[0];
        }
        self.status.outcome = self.status_packet.capture_holding(
            memory,
            base,
            state,
            rows,
            outcome,
            self.status.rows.len(),
            &self.attack.held_maxima,
            &self.attack.exercise_experience,
        );
        self.phase_ticks[5] = qpc().saturating_sub(certified).max(0) as u64;
        if ordered_sources {
            self.phase_ticks[22] = self.phase_ticks[4] + self.phase_ticks[5];
        }
        let refused = self
            .status_packet
            .capture_movement_organisation(memory, base, state, rows);
        ORGANISATION_REFUSALS.fetch_add(refused, std::sync::atomic::Ordering::Relaxed);
        let named = qpc();
        self.capture_naming_holding(
            memory,
            base,
            &rows[..self.status.outcome.len()],
            army_stats.as_deref_mut(),
            changes,
        );
        if !self.owned_settlement
            && let Some(store) = army_stats
        {
            self.daily_status.settle(
                &self.status_packet,
                store,
                base,
                rows,
                matches!(outcome, Outcome::Rows { .. }) && self.status.outcome == outcome,
            );
        }
        let finished = qpc();
        // The dig-in phase is the movement pass job's ([`Self::restore_movement`]).
        self.phase_ticks[0] += named.saturating_sub(start).max(0) as u64;
        self.phase_ticks[1] = 0;
        self.phase_ticks[2] = finished.saturating_sub(named).max(0) as u64;
        drop(_status_reads);
        self.decode_pending = true;
        finished.saturating_sub(start).max(0) as u64
    }

    /// This hour's DivisionStatus span by pass, in [`PHASE_NAMES`] order.
    pub(crate) fn phase_ticks(&self) -> [u64; PHASES] {
        self.phase_ticks
    }

    pub(crate) fn enable_owned_settlement(&mut self) {
        self.owned_settlement = true;
    }

    pub(crate) fn retain_owned_settlement(
        &mut self,
        mut store: held_store::Loan,
        inputs: Option<Arc<crate::division_census::UnitInputs>>,
        context: CaptureContext,
    ) -> Option<held_store::Loan> {
        if !self.sources_deferred
            && !self.attack_deferred
            && !self.attack.has_owned_certification()
            && !self.daily_status.pending()
        {
            store.complete();
            return Some(store);
        }
        self.deferred_store = Some(store);
        self.deferred_inputs = inputs;
        self.deferred_context = Some(context);
        None
    }
    pub(crate) fn take_settled_store(&mut self) -> Option<held_store::Loan> {
        self.speed_terms.discard_owned();
        self.deferred_store.take()
    }
    fn settle_owned_sources(&mut self) {
        if !std::mem::take(&mut self.sources_deferred) {
            return;
        }
        let context = self.deferred_context.expect("owned presentation context");
        let Some(store) = self.deferred_store.as_deref_mut() else {
            return;
        };
        let inputs = self
            .deferred_inputs
            .as_deref()
            .expect("owned presentation sources");
        let seeds = &inputs.seeds()[..inputs.outcome().len()];
        let parts = &self.preparation_parts[..self.status_part_count];
        self.attack.sources = Some(store.sources.settle_owned(
            context.base,
            context.game_state,
            seeds,
            &mut self.attack.selected_sources,
            |index| {
                parts
                    .iter()
                    .find(|part| part.range.contains(&index))
                    .and_then(|part| part.get(index))
                    .and_then(|row| row.selection)
            },
            |index| {
                parts
                    .iter()
                    .find(|part| part.range.contains(&index))
                    .and_then(|part| part.source_reads())
            },
            |handle, pointer| {
                parts
                    .iter()
                    .filter_map(|part| part.source_reads())
                    .find(|reads| reads.has_variant(handle, pointer))
            },
        ));
        self.attack
            .selected_sources
            .sort_unstable_by_key(|row| row.0);
        self.phase_ticks[6..10].copy_from_slice(&store.sources.phase_ticks());
        if self.attack.sources.as_ref().is_some_and(Result::is_err) {
            store.sources.forget();
            self.attack.selected_sources.clear();
        }
    }

    /// Pure decode-worker projection from the current owned packets.
    pub(crate) fn decode_inputs(&mut self) -> [u64; 2] {
        // Independent companions still settle if status itself refused. In
        // particular, return the unit-speed loan before another hour takes marks.
        self.phase_ticks[18] += self.speed_terms.decode();
        if std::mem::take(&mut self.movement_owned_pending) {
            let started = qpc();
            for reason in self.movement_packet.combat_reference_failures() {
                super::unit_combat_references::observe_failure(reason);
            }
            super::unit_combat_references::demand(
                false,
                self.movement_packet.combat_references_required(),
                self.movement_packet.combat_references_capacity(),
            );
            let (withdrawal, priority) = self.movement_packet.movement_style_refusals();
            WITHDRAWAL_REFUSALS.fetch_add(withdrawal as u64, std::sync::atomic::Ordering::Relaxed);
            MOVEMENT_PRIORITY_REFUSALS.fetch_add(priority as u64, std::sync::atomic::Ordering::Relaxed);
            BASE_SPEED_REFUSALS.fetch_add(
                self.movement_packet.base_speed_refusals() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
            self.dig_in.settle(self.status.outcome);
            self.phase_ticks[19] += qpc().saturating_sub(started).max(0) as u64;
        }
        if !std::mem::take(&mut self.decode_pending) {
            return [0; 2];
        }
        if std::mem::take(&mut self.status_omitted) {
            return [self.project_owned_movement(), 0];
        }
        let started = qpc();
        self.settle_owned_sources();
        self.phase_ticks[14] = qpc().saturating_sub(started).max(0) as u64;
        let started = qpc();
        if let Some(store) = self.deferred_store.as_deref_mut() {
            let context = self.deferred_context.expect("owned certification context");
            let inputs = self
                .deferred_inputs
                .as_deref()
                .expect("owned certification census");
            let parts = &self.preparation_parts;
            self.attack.settle_certification(
                context.base,
                inputs.seeds(),
                inputs.outcome(),
                store,
                |index| {
                    parts
                        .iter()
                        .find(|part| part.range.contains(&index))
                        .and_then(|part| part.get(index))
                },
            );
        }
        self.phase_ticks[17] = qpc().saturating_sub(started).max(0) as u64;
        let started = qpc();
        if std::mem::take(&mut self.stitch_deferred) {
            self.status.outcome = self
                .status_packet
                .finish_status_stitch(&self.status_parts[..self.status_part_count]);
        }
        self.phase_ticks[16] = qpc().saturating_sub(started).max(0) as u64;
        let started = qpc();
        if std::mem::take(&mut self.attack_deferred) {
            let context = self.deferred_context.expect("owned presentation context");
            let inputs = self
                .deferred_inputs
                .as_deref()
                .expect("owned attack sources");
            let status = &self.status_packet;
            let parts = &self.attack_parts;
            self.attack.capture_owned(
                context.base,
                context.hour,
                inputs.seeds(),
                inputs.outcome(),
                self.deferred_store.as_deref_mut(),
                |index| status.unit_maxima(index),
                |index| {
                    parts
                        .iter()
                        .find(|part| part.range.contains(&index))
                        .and_then(|part| part.get(index))
                },
            );
            self.attack_ready = false;
        }
        self.phase_ticks[15] = qpc().saturating_sub(started).max(0) as u64;
        if let (Some(store), Some(inputs), Some(context)) = (
            self.deferred_store.as_deref_mut(),
            self.deferred_inputs.as_deref(),
            self.deferred_context,
        ) {
            self.daily_status.settle(
                &self.status_packet,
                store,
                context.base,
                inputs.seeds(),
                matches!(inputs.outcome(), Outcome::Rows { .. })
                    && self.status.outcome == inputs.outcome(),
            );
        }
        self.deferred_inputs = None;
        self.deferred_context = None;
        let movement_ticks = self.project_owned_movement();
        let started = qpc();
        self.status.outcome = self.status_packet.project(&mut self.status.rows);
        let count = self.status.outcome.len();
        for (row, naming) in self.status.rows[..count]
            .iter_mut()
            .zip(&self.naming.first[..self.naming.len])
        {
            if row.division != naming.division {
                self.status.outcome = Outcome::Unavailable {
                    reason: "division naming input identity invalid",
                };
                break;
            }
            row.naming = naming.inputs;
            row.country_ordinal = naming.country_ordinal;
        }
        if let Some(store) = self.deferred_store.as_mut() {
            store.complete();
        }
        [movement_ticks, qpc().saturating_sub(started).max(0) as u64]
    }

    /// The movement pass's rows, projected from its packet; returns the ticks.
    fn project_owned_movement(&mut self) -> u64 {
        let started = qpc();
        // Projection is still worker-owned. Its destination follows the full
        // original route demand, including a failed payload's retained prefix.
        let movement = if grow_vec(
            &mut self.paths.first,
            self.movement_packet.outcome().required_paths,
        ) {
            carmy::project_movement(
                &self.movement_packet,
                &mut self.movement.rows,
                &mut self.paths.first,
            )
        } else {
            carmy::MovementWalk {
                outcome: Outcome::Unavailable {
                    reason: "division movement projection allocation failed",
                },
                ..self.movement_packet.outcome()
            }
        };
        self.movement.outcome = movement.outcome;
        for &(index, inputs) in &self.transport_inputs {
            if let Some(row) = self.movement.rows.get_mut(index) {
                row.transport_speed = inputs.map(TransportInputs::evaluate);
            }
        }
        self.transport.clear();
        self.paths.len = movement.paths_len;
        self.paths.required = self.paths.required.max(movement.required_paths);
        let movement_ticks = qpc().saturating_sub(started).max(0) as u64;
        self.phase_ticks[19] += movement_ticks;
        movement_ticks
    }

    pub(crate) fn capture_border_stats(
        &mut self,
        memory: &impl Memory,
        base: usize,
        demanded: Result<impl IntoIterator<Item = u64>, &'static str>,
    ) -> u64 {
        if !matches!(self.status.outcome, Outcome::Rows { .. }) {
            return 0;
        }
        let _reads = crate::read_accounting::table(TableId::DivisionStatus);
        let start = qpc();
        match demanded {
            Ok(demanded) => {
                self.status.outcome = self
                    .status_packet
                    .capture_border_stats(memory, base, demanded);
            }
            Err(reason) => {
                self.status_packet.fail_border_stats(reason);
                self.status.outcome = Outcome::Unavailable { reason };
            }
        }
        qpc().saturating_sub(start).max(0) as u64
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        let [states, access, entries, rivers, transport] = self.speed_terms.outcomes();
        [
            self.movement.outcome,
            self.status.outcome,
            self.bans.outcome,
            self.land.outcome(),
            match &self.attack.history {
                Some(Ok(rows)) => Outcome::Rows { len: rows.len() },
                Some(Err(reason)) => Outcome::Unavailable { reason },
                None => Outcome::Unavailable {
                    reason: "division history not captured",
                },
            },
            match &self.attack.sources {
                Some(Ok(rows)) => Outcome::Rows { len: rows.len() },
                Some(Err(reason)) => Outcome::Unavailable { reason },
                None => Outcome::Unavailable {
                    reason: "division panel sources not captured",
                },
            },
            self.status.outcome,
            states,
            access,
            entries,
            rivers,
            transport,
        ]
    }

    /// `frame_hour` is the frame's hour; packets read for another hour are
    /// refused. Tests without a frame pass None.
    pub(crate) fn table_inputs(
        &self,
        mut projection: Option<&mut Projection<'_>>,
        frame_hour: Option<i32>,
    ) -> [TableInput; N] {
        if self.decode_pending {
            return std::array::from_fn(|_| {
                unavailable("division presentation projection pending").into()
            });
        }
        let [movement, mut status, bans, ..] = self.outcomes();
        let land = self.land.table_input(projection.as_deref_mut());
        let terms = self.speed_terms.table_inputs(projection.as_deref_mut());
        let [states, access, entries, rivers, transport] = terms;
        let bans = scalar_table::<tables::DivisionRetreatBans, _>(
            projection.as_deref_mut(),
            bans,
            &self.bans.rows,
            |r| u128::from(r.division),
            |r| DivisionRetreatBanRow {
                division: r.division,
                owner: r.owner,
                bans: r.bans,
            },
        );
        // Packets store no hour: only those read for this frame's hour are
        // published; a packet from another request is its refusal.
        let request = self
            .movement_packet
            .combat_references_hour()
            .filter(|sample| frame_hour.is_none_or(|hour| *sample == hour));
        let names = &self.names.first[..self.names.len];
        let name_span = |(start, len): cname_group_member::Span| {
            start.checked_add(len).and_then(|end| names.get(start..end))
        };
        let cached_name = |r: &carmy::StatusRow, index: usize| {
            let row = self.naming.first.get(index)?;
            if row.division != r.division {
                return None;
            }
            let (start, len) = row.cached_name?;
            self.cached_names
                .first
                .get(..self.cached_names.len)?
                .get(start..start.checked_add(len)?)
        };
        if self.status.rows.get(..status.len()).is_none_or(|rows| {
            rows.iter().any(|r| {
                r.naming
                    .is_some_and(|n| name_span(n.custom).is_none() || name_span(n.group).is_none())
            })
        }) {
            status = Outcome::Unavailable {
                reason: "division naming spans invalid",
            };
        }
        let status_outcome = status;
        let status = project_table::<tables::DivisionStatus, _>(
            projection.as_deref_mut(),
            status,
            &self.status.rows,
            |r| u128::from(r.division),
            |r, previous| {
                let (Ok(border), Ok(maximum), Ok(remaining), Ok(naming)) = (
                    previous
                        .border_combat_stats
                        .require("DivisionStatus.border_combat_stats"),
                    previous
                        .supply_grace_maximum
                        .require("DivisionStatus.supply_grace_maximum"),
                    previous
                        .supply_grace_remaining
                        .require("DivisionStatus.supply_grace_remaining"),
                    previous.naming.require("DivisionStatus.naming"),
                ) else {
                    return false;
                };
                if !status_fields!(r, compare_status_fields, [previous])
                    || previous.attack != Recorded::Value(Witnessed(None))
                    || self.attack.has_witness(r.division)
                {
                    return false;
                }
                let attack = self.attack.view(r.division);
                let index = self.status_index(r.division);
                let supply = self.supply_sample_at(r, index);
                let commander = self.commander_modifiers(r.division, index);
                Recorded::Value(attack.experience()) == previous.experience
                    && Recorded::Value(attack.pending_template()) == previous.pending_template
                    && Recorded::Value(attack.officer()) == previous.officer_experience
                    && Recorded::Value(attack.career()) == previous.career_counter
                    && Recorded::Value(attack.panel_source()) == previous.panel_source
                    && attack.same(&previous.attack_stats)
                    && attack.same_movement(&previous.movement_stats)
                    && Recorded::Value(r.weather_factor) == previous.weather_factor
                    && Recorded::Value(r.out_of_supply_days) == previous.out_of_supply_days
                    && DivisionStatusRow::exact_supply(
                        r.out_of_supply_days,
                        r.supply_ratio,
                        r.supply_grace_remaining,
                    ) == previous.supply_exact()
                    && matches!(previous.supply_ratio_exact, Recorded::Value(_))
                    && previous.attrition_inputs == Recorded::Value(Retired)
                    && super::supply_inputs::matches_division(supply.0, supply.2, &previous.supply)
                    && Recorded::Value(r.country_ordinal) == previous.country_ordinal
                    && Recorded::Value(r.invalid_orders_reason) == previous.invalid_orders_reason
                    && Recorded::Value(self.dig_in.factor(r.division, index))
                        == previous.dig_in_factor
                    && Recorded::Value(commander.map(|entries| {
                        [entries[0], entries[1], entries[2], entries[4], entries[5]]
                    })) == previous.commander_modifiers
                    && Recorded::Value(commander.and_then(|entries| entries[3]))
                        == previous.commander_width_factor
                    && Recorded::Value(self.speed_terms.combat_movement(r.division))
                        == previous.combat_movement
                    && match &previous.name {
                        Recorded::Value(name) => cached_name(r, index) == name.as_deref(),
                        Recorded::NotRecorded => false,
                    }
                    && r.border_combat_stats == *border
                    && r.supply_grace_maximum == *maximum
                    && DivisionStatusRow::displayed_grace(
                        r.supply_grace_remaining,
                        self.supply_grace,
                    ) == *remaining
                    && Recorded::Value(camouflage(r.camouflage)) == previous.camouflage
                    && previous.acclimatization_maximum.is_none()
                    && previous.acclimatization.is_empty()
                    && match (r.naming, naming) {
                        (None, None) => true,
                        (Some(n), Some(previous)) => {
                            n.kind == previous.kind
                                && n.order == previous.order
                                && n.ordered == previous.ordered
                                && n.programmatic == previous.programmatic
                                && name_span(n.custom).unwrap() == previous.custom.as_slice()
                                && name_span(n.group).unwrap() == previous.group.as_slice()
                        }
                        _ => false,
                    }
                    && Recorded::Value(name_template(r.naming)) == previous.name_template
            },
            |r| {
                let attack = self.attack.view(r.division);
                let index = self.status_index(r.division);
                let supply = self.supply_sample_at(r, index);
                status_fields!(
                    r,
                    project_status_fields,
                    [
                        r,
                        self.supply_grace,
                        r.naming.map(|n| NameOperands {
                            kind: n.kind,
                            order: n.order,
                            ordered: n.ordered,
                            programmatic: n.programmatic,
                            custom: owned_vec(copy_vec(name_span(n.custom).unwrap())),
                            group: owned_vec(copy_vec(name_span(n.group).unwrap())),
                        }),
                        super::supply_inputs::project_division(supply.0, supply.2),
                        self.dig_in.factor(r.division, index),
                        self.commander_modifiers(r.division, index),
                        cached_name(r, index).map(|name| owned_vec(copy_vec(name))),
                        attack.stats(),
                        attack.witness(r.weather_factor),
                        attack.movement(),
                        attack.experience(),
                        attack.pending_template(),
                        attack.officer(),
                        attack.career(),
                        attack.panel_source(),
                        self.speed_terms.combat_movement(r.division),
                    ],
                )
            },
        );
        let panel = scalar_table::<tables::DivisionPanelStats, _>(
            projection.as_deref_mut(),
            status_outcome,
            &self.status.rows,
            |r| u128::from(r.division),
            |r| {
                let words = self.attack.panel(r.division);
                DivisionPanelStatsRow {
                    division: r.division,
                    panel_stats: std::array::from_fn(|at| words[at]),
                    suppression: words[22].into(),
                }
            },
        );
        let history = self.history_input(projection.as_deref());
        let sources = match &self.attack.sources {
            Some(Ok(rows)) => rows
                .publish(|rows| match projection.as_deref() {
                    Some(projection) => projection.certified::<tables::DivisionPanelSources>(rows),
                    None => match rows.complete::<tables::DivisionPanelSources>() {
                        Ok(rows) => replay_frame::frame::TableView::Complete(rows).into(),
                        Err(reason) => unavailable(reason).into(),
                    },
                })
                .unwrap_or_else(|reason| unavailable(reason).into()),
            Some(Err(reason)) => unavailable(reason).into(),
            None => unavailable("division panel sources not captured").into(),
        };
        let Some(paths) = self.paths.first.get(..self.paths.len) else {
            return [
                unavailable("movement arena invalid").into(),
                status,
                bans,
                land,
                history,
                sources,
                panel,
                states,
                access,
                entries,
                rivers,
                transport,
            ];
        };
        let span =
            |start: usize, len: usize| start.checked_add(len).and_then(|end| paths.get(start..end));
        if self.movement.rows.get(..movement.len()).is_none_or(|rows| {
            rows.iter().any(|r| {
                span(r.path_start, r.path_len).is_none()
                    || span(r.alternate_start, r.alternate_len).is_none()
            })
        }) {
            return [
                unavailable("movement path spans invalid").into(),
                status,
                bans,
                land,
                history,
                sources,
                panel,
                states,
                access,
                entries,
                rivers,
                transport,
            ];
        }
        [
            project_table::<tables::DivisionMovement, _>(
                projection,
                movement,
                &self.movement.rows,
                |r| u128::from(r.division),
                |r, previous| {
                    previous.selected == Recorded::Value(Retired)
                        && r.division == previous.division
                        && super::unit_combat_references::matches(
                            request,
                            self.movement_packet.combat_references(r),
                            r.combat_count,
                            &previous.combat_references,
                        )
                        && self.speed_witness_matches(r.division, &previous.combat_references)
                        && previous.fresh == Recorded::Value(Retired)
                        && Recorded::Value(r.base_speed) == previous.base_speed
                        && Recorded::Value(r.fuel) == previous.fuel
                        && Recorded::Value(r.transport_speed) == previous.transport_speed
                        && r.source == previous.source
                        && r.target == previous.target
                        && r.numerator == previous.numerator
                        && r.retreat == previous.retreat
                        && r.strategic_redeployment == previous.strategic_redeployment
                        && replay_data::rows::Recorded::Value(r.strategic_action)
                            == previous.strategic_action
                        && Recorded::Value(r.withdrawal) == previous.withdrawal
                        && Recorded::Value(r.movement_priority) == previous.movement_priority
                        && Recorded::Value(self.order_transport(r.division))
                            == previous.order_transport
                        && previous.transport == Recorded::Value(Retired)
                        && r.naval_transfer == previous.naval_transfer
                        && r.combat_count == previous.combat_count
                        && r.path_total == previous.path_total
                        && span(r.path_start, r.path_len).unwrap() == previous.path.as_slice()
                        && r.combat_role == previous.combat_role
                        && r.combat_province == previous.combat_province
                        && r.map_entity == previous.map_entity
                        && r.alternate_total == previous.alternate_total
                        && span(r.alternate_start, r.alternate_len).unwrap()
                            == previous.alternate_path.as_slice()
                },
                |r| DivisionMovementRow {
                    selected: Recorded::Value(Retired),
                    combat_references: Recorded::Value(
                        super::unit_combat_references::project(
                            request,
                            self.movement_packet.combat_references(r),
                            r.combat_count,
                        )
                        .map(|mut packet| {
                            packet.speed_witness = self.speed_witness(r.division);
                            packet
                        }),
                    ),
                    division: r.division,
                    fresh: Recorded::Value(Retired),
                    source: r.source,
                    target: r.target,
                    numerator: r.numerator,
                    retreat: r.retreat,
                    strategic_redeployment: r.strategic_redeployment,
                    strategic_action: replay_data::rows::Recorded::Value(r.strategic_action),
                    withdrawal: Recorded::Value(r.withdrawal),
                    movement_priority: Recorded::Value(r.movement_priority),
                    order_transport: Recorded::Value(self.order_transport(r.division)),
                    transport: Recorded::Value(Retired),
                    naval_transfer: r.naval_transfer,
                    combat_count: r.combat_count,
                    path_total: r.path_total,
                    path: owned_vec(copy_vec(span(r.path_start, r.path_len).unwrap())),
                    combat_role: r.combat_role,
                    combat_province: r.combat_province,
                    map_entity: r.map_entity,
                    alternate_total: r.alternate_total,
                    alternate_path: owned_vec(copy_vec(
                        span(r.alternate_start, r.alternate_len).unwrap(),
                    )),
                    base_speed: Recorded::Value(r.base_speed),
                    fuel: Recorded::Value(r.fuel),
                    transport_speed: Recorded::Value(r.transport_speed),
                },
            ),
            status,
            bans,
            land,
            history,
            sources,
            panel,
            states,
            access,
            entries,
            rivers,
            transport,
        ]
    }

    fn history_input(&self, projection: Option<&Projection<'_>>) -> TableInput {
        match &self.attack.history {
            Some(Ok(rows)) if projection.is_some() => projection
                .unwrap()
                .certified::<tables::DivisionHistory>(rows),
            Some(Ok(rows)) => match rows.complete::<tables::DivisionHistory>() {
                Ok(rows) => replay_frame::frame::TableView::Complete(rows).into(),
                Err(reason) => unavailable(reason).into(),
            },
            Some(Err(reason)) => unavailable(reason).into(),
            None => unavailable("division history not captured").into(),
        }
    }
    /// The division's selected commander block's movement entries: the block from the
    /// dig-in factor's selection, its entries from the hour's retreat-ban read of the
    /// same commander ([`carmy::StatusPacket::commander_movement`]).
    fn commander_modifiers(&self, division: u64, index: usize) -> Option<[Option<i64>; 6]> {
        let block = self.dig_in.commander_block(division, index)?;
        self.status_packet.commander_movement(division, block)
    }

    fn status_index(&self, division: u64) -> usize {
        self.status_packet
            .status_index(division)
            .unwrap_or(usize::MAX)
    }
    fn supply_sample(
        &self,
        row: &carmy::StatusRow,
    ) -> (
        Option<engine_native::cunit_supply_inputs::Inputs>,
        Option<engine_native::cunit_supply_inputs::Failure>,
        Option<&[engine_native::cunit_supply_inputs::Origin]>,
    ) {
        self.supply_sample_at(row, self.status_index(row.division))
    }
    fn supply_sample_at(
        &self,
        row: &carmy::StatusRow,
        index: usize,
    ) -> (
        Option<engine_native::cunit_supply_inputs::Inputs>,
        Option<engine_native::cunit_supply_inputs::Failure>,
        Option<&[engine_native::cunit_supply_inputs::Origin]>,
    ) {
        match self.daily_status.sample(index) {
            Some(sample) => (sample.inputs, sample.failure, sample.origins()),
            None => (
                row.supply,
                row.supply_failure,
                self.status_packet.supply_origins(row),
            ),
        }
    }
    #[cfg(test)]
    pub(super) fn tables(&self) -> [replay_frame::frame::TableView; N] {
        self.table_inputs(None, None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection without a baseline always samples"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit_scale_tests::fixture as scale_fixture;
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;

    mod daily_status;
    mod model_witness;
    mod read_ahead;
    mod status_bench;
    mod status_reads;

    fn member(base: usize, name: &[u8], order: i32) -> [u8; cname_group_member::INPUT_SPAN] {
        assert!(name.len() < 16);
        let mut bytes = [0; cname_group_member::INPUT_SPAN];
        bytes[..8].copy_from_slice(&((base + cname_group_member::VTABLE_RVA) as u64).to_le_bytes());
        bytes[cname_group_member::ORDER..cname_group_member::ORDER + 4]
            .copy_from_slice(&order.to_le_bytes());
        let custom = cname_group_member::OVERRIDE_NAME;
        bytes[custom..custom + name.len()].copy_from_slice(name);
        bytes[custom + 16..custom + 24].copy_from_slice(&(name.len() as u64).to_le_bytes());
        bytes[custom + 24..custom + 32].copy_from_slice(&15_u64.to_le_bytes());
        bytes[0x78..0x80].copy_from_slice(&15_u64.to_le_bytes());
        bytes[cname_group_member::FLAG_A] = 255;
        bytes
    }

    fn template_registry(
        world: &mut scale_fixture::World,
    ) -> (Box<[u8; 32]>, Vec<u8>, Box<[u8; 40]>) {
        use engine_native::{creference_registry as registry, reference_bucket as bucket};
        let word = |offset| u64::from_le_bytes(world.bytes[offset..offset + 8].try_into().unwrap());
        let templates: std::collections::BTreeSet<_> = world
            .armies
            .iter()
            .map(|army| word(army - world.base + carmy::STANDARD_TEMPLATE) as usize)
            .collect();
        let count = (templates.len() * 2).next_power_of_two().max(8);
        let mask = count as u32 - 1;
        let overflow = count.trailing_zeros() as usize + 2;
        let mut slots = vec![None; count + overflow];
        for &template in &templates {
            let mut entry = (word(template - world.base + 8), template, 1_usize);
            let hash = entry
                .0
                .rotate_left(32)
                .wrapping_mul(registry::HASH_MULTIPLIER);
            let mut index = (((hash >> 32) as u32 ^ hash as u32) & mask) as usize;
            loop {
                assert!(entry.2 <= overflow + 1);
                match &mut slots[index] {
                    slot @ None => {
                        *slot = Some(entry);
                        break;
                    }
                    Some(resident) if resident.2 < entry.2 => std::mem::swap(resident, &mut entry),
                    _ => {}
                }
                index += 1;
                entry.2 += 1;
            }
        }
        let mut entries = vec![0; (count + overflow + 1) * bucket::STRIDE];
        entries[(count + overflow) * bucket::STRIDE + bucket::DISTANCE] = bucket::TERMINAL_DISTANCE;
        for (index, entry) in slots.into_iter().enumerate() {
            if let Some((handle, pointer, distance)) = entry {
                let at = index * bucket::STRIDE;
                entries[at + bucket::DISTANCE] = distance as u8;
                entries[at + 8..at + 16].copy_from_slice(&handle.to_le_bytes());
                entries[at + 16..at + 24].copy_from_slice(&(pointer as u64).to_le_bytes());
            }
        }
        let mut header = Box::new([0; 32]);
        header[registry::ENTRIES..registry::ENTRIES + 8]
            .copy_from_slice(&(entries.as_ptr() as u64).to_le_bytes());
        header[registry::COUNT..registry::COUNT + 4]
            .copy_from_slice(&(templates.len() as u32).to_le_bytes());
        header[registry::MASK..registry::MASK + 4].copy_from_slice(&mask.to_le_bytes());
        header[registry::OVERFLOW_DISTANCE] = overflow as u8;
        let root = registry::LOW_ROOTS_RVA
            + engine_native::creferenced_division_template::REFERENCE_TYPE as usize * 8;
        world.bytes[root..root + 8].copy_from_slice(&(header.as_ptr() as u64).to_le_bytes());
        let mut group = Box::new([0; 40]);
        group[..8].copy_from_slice(
            &((world.base + cname_group_member::GROUP_VTABLE_RVA) as u64).to_le_bytes(),
        );
        group[8..13].copy_from_slice(b"GROUP");
        group[24..32].copy_from_slice(&5_u64.to_le_bytes());
        group[32..40].copy_from_slice(&15_u64.to_le_bytes());
        for template in templates {
            let offset = template - world.base + cname_group_member::TEMPLATE_NAME_GROUP;
            world.bytes[offset..offset + 8].copy_from_slice(&(group.as_ptr() as u64).to_le_bytes());
        }
        (header, entries, group)
    }

    fn division_name_members(
        world: &mut scale_fixture::World,
    ) -> Vec<[u8; cname_group_member::INPUT_SPAN]> {
        let members: Vec<_> = world
            .armies
            .iter()
            .enumerate()
            .map(|(index, army)| {
                let mut member = member(world.base, b"", index as i32 + 1);
                let offset = army - world.base + carmy::STANDARD_TEMPLATE;
                let template =
                    u64::from_le_bytes(world.bytes[offset..offset + 8].try_into().unwrap())
                        as usize;
                member[cname_group_member::TEMPLATE_REFERENCE
                    ..cname_group_member::TEMPLATE_REFERENCE + 8]
                    .copy_from_slice(
                        &world.bytes[template - world.base + 8..template - world.base + 16],
                    );
                member
            })
            .collect();
        for (&army, member) in world.armies.iter().zip(&members) {
            let offset = army - world.base + engine_native::carmy_combat_inputs::NAME_GROUP_MEMBER;
            world.bytes[offset..offset + 8]
                .copy_from_slice(&(member.as_ptr() as u64).to_le_bytes());
        }
        members
    }

    #[test]
    fn cached_name_growth_and_template_failure_keep_status_and_owned_cache_independent() {
        let mut world = scale_fixture::build(scale_fixture::Shape {
            countries: 2,
            divisions: 6,
            wings: 1,
            planes: 1,
            ships: 0,
        });
        let mut registry = template_registry(&mut world);
        // An unresolved reference (including the invalid sentinel) is native
        // known absence. Refuse an actually malformed template registry while
        // leaving the independent retained name member/cache readable.
        registry.0[engine_native::creference_registry::COUNT
            ..engine_native::creference_registry::COUNT + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        let mut members = division_name_members(&mut world);
        members[0][cname_group_member::FLAG_A] = 0;
        members[0][0x60..0x63].copy_from_slice(&[0xff, 0, b'C']);
        members[0][0x70..0x78].copy_from_slice(&3_u64.to_le_bytes());
        let memory = world.live();
        let mut seeds: Vec<_> = world
            .armies
            .iter()
            .enumerate()
            .map(|(index, &army)| DivisionSeed {
                pointer: army,
                division: engine_native::read(&memory, army + engine_native::cunit::HANDLE)
                    .unwrap(),
                name_member: Some(members[index].as_ptr() as usize),
                ..Default::default()
            })
            .collect();
        carmy::capture_division_seeds(&memory, &mut seeds);
        let capacity = Capacities {
            movement: 0,
            status: seeds.len(),
            bans: 0,
            paths: 0,
            name_bytes: 5,
            name_groups: world.shape.countries,
            cached_name_bytes: 0,
            ..Capacities::default()
        };
        let mut captured = Captured::with_capacities(&capacity);
        let sources = || Sources {
            rows: &seeds,
            outcome: Outcome::Rows { len: seeds.len() },
        };
        captured.capture_owned(
            &memory,
            CaptureContext {
                base: world.base,
                game_state: world.state,
                hour: 0,
            },
            sources(),
        );
        assert_eq!(captured.cached_names.required, 3);
        assert!(captured.naming.first[0].inputs.is_none());
        assert_eq!(
            captured.naming.first[0].failure,
            Some("reference header invalid")
        );
        assert_eq!(captured.naming.first[0].cached_failure, None);
        assert_eq!(captured.cached_names.len, 3);
        // Deferred projection uses only the captured bytes, even if live memory changes.
        members[0][0x60] = b'R';
        captured.decode_inputs();
        let TableView::Complete(TableRows::DivisionStatus(rows)) = captured.tables()[1].clone()
        else {
            panic!("a failed generator preserves the status prefix and exact cache");
        };
        assert_eq!(rows.len(), seeds.len());
        assert_eq!(rows[0].name, Recorded::Value(Some(vec![0xff, 0, b'C'])));
        assert_eq!(rows[0].naming, Recorded::Value(None));
        assert!(
            rows[1..]
                .iter()
                .all(|row| row.name == Recorded::Value(Some(Vec::new())))
        );
        let mut baseline = std::array::from_fn(|_| unavailable("outside cache fixture"));
        baseline[TableId::DivisionStatus.index()] = captured.tables()[1].clone();
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        encoder
            .encode(
                replay_frame::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: Default::default(),
                },
                false,
            )
            .unwrap();
        // Hold the sample hour constant so only the retained cache changes.
        captured.capture_owned(
            &memory,
            CaptureContext {
                base: world.base,
                game_state: world.state,
                hour: 0,
            },
            sources(),
        );
        captured.decode_inputs();
        let scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
        let [_, TableInput::ComparedChanges { indices, rows, .. }, ..] =
            captured.table_inputs(Some(&mut projection), None)
        else {
            panic!("a changed retained cache defeats baseline reuse");
        };
        assert_eq!(indices, [0]);
        let TableRows::DivisionStatus(rows) = rows else {
            panic!("status row changes");
        };
        assert_eq!(rows[0].name, Recorded::Value(Some(vec![b'R', 0, b'C'])));
    }

    /// One copy of each member span serves the generator operands and the
    /// cached-name lookup cache, and a heap cache that fits is one copy of its bytes and
    /// terminator.
    #[test]
    fn naming_copies_each_member_span_once_for_both_passes() {
        struct Reads<'a, M>(&'a M, std::cell::RefCell<Vec<(usize, usize)>>);
        impl<M: Memory> Memory for Reads<'_, M> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.1.borrow_mut().push((address, destination.len()));
                self.0.copy(address, destination)
            }
        }
        let mut world = scale_fixture::build(scale_fixture::Shape {
            countries: 2,
            divisions: 3,
            wings: 1,
            planes: 1,
            ships: 0,
        });
        let _registry = template_registry(&mut world);
        let mut members = division_name_members(&mut world);
        let text = b"Heap cached division name";
        let heap = [&text[..], &[0_u8][..]].concat();
        let cache = cname_group_member::CACHED_NAME;
        members[0][cache..cache + 8].copy_from_slice(&(heap.as_ptr() as u64).to_le_bytes());
        members[0][cache + 16..cache + 24].copy_from_slice(&(text.len() as u64).to_le_bytes());
        members[0][cache + 24..cache + 32].copy_from_slice(&(text.len() as u64).to_le_bytes());
        let live = world.live();
        let memory = Reads(&live, Default::default());
        let seeds: Vec<_> = world
            .armies
            .iter()
            .enumerate()
            .map(|(index, &army)| DivisionSeed {
                pointer: army,
                division: index as u64 + 1,
                name_member: Some(members[index].as_ptr() as usize),
                ..Default::default()
            })
            .collect();
        let mut captured = Captured::with_capacities(&Capacities {
            status: seeds.len(),
            name_groups: world.shape.countries,
            cached_name_bytes: 64,
            ..Capacities::default()
        });
        captured.capture_naming(&memory, world.base, &seeds);
        let reads = memory.1.borrow();
        for member in &members {
            let at = member.as_ptr() as usize;
            assert_eq!(reads.iter().filter(|read| read.0 == at).count(), 1);
            assert!(!reads.iter().any(|read| read.0 == at + cache));
        }
        let heap_reads: Vec<_> = reads
            .iter()
            .filter(|read| {
                (heap.as_ptr() as usize..heap.as_ptr() as usize + heap.len()).contains(&read.0)
            })
            .collect();
        assert_eq!(heap_reads, [&(heap.as_ptr() as usize, heap.len())]);
        let (start, len) = captured.naming.first[0].cached_name.unwrap();
        assert_eq!(&captured.cached_names.first[start..start + len], text);
        assert!(
            captured
                .naming
                .first
                .iter()
                .take(3)
                .all(|row| row.inputs.is_some())
        );
    }

    /// A held name member no hooked writer touched stands in for its reads,
    /// giving the rows a read gives. A member a writer touched is read again.
    #[test]
    fn naming_carries_an_untouched_member_and_rereads_a_touched_one() {
        use crate::army_stat_changes::Watches;
        struct Reads<'a, M>(&'a M, std::cell::RefCell<Vec<usize>>);
        impl<M: Memory> Memory for Reads<'_, M> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.1.borrow_mut().push(address);
                self.0.copy(address, destination)
            }
        }
        let mut world = scale_fixture::build(scale_fixture::Shape {
            countries: 2,
            divisions: 3,
            wings: 1,
            planes: 1,
            ships: 0,
        });
        let _registry = template_registry(&mut world);
        let mut members = division_name_members(&mut world);
        let text = b"Heap cached division name";
        let heap = [&text[..], &[0_u8][..]].concat();
        let cache = cname_group_member::CACHED_NAME;
        members[0][cache..cache + 8].copy_from_slice(&(heap.as_ptr() as u64).to_le_bytes());
        members[0][cache + 16..cache + 24].copy_from_slice(&(text.len() as u64).to_le_bytes());
        members[0][cache + 24..cache + 32].copy_from_slice(&(text.len() as u64).to_le_bytes());
        let live = world.live();
        let seeds: Vec<_> = world
            .armies
            .iter()
            .enumerate()
            .map(|(index, &army)| DivisionSeed {
                pointer: army,
                division: (index as u64 + 1) << 32,
                name_member: Some(members[index].as_ptr() as usize),
                ..Default::default()
            })
            .collect();
        let watches = Watches::enabled(16);
        let mut store = super::super::attack_stats::ArmyStatStore::new();
        let mut captured = Captured::with_capacities(&Capacities {
            status: seeds.len(),
            name_groups: world.shape.countries,
            cached_name_bytes: 64,
            ..Capacities::default()
        });
        let mut hour = |store: &mut super::super::attack_stats::ArmyStatStore| {
            let memory = Reads(&live, Default::default());
            let outcome = Outcome::Rows { len: seeds.len() };
            captured.attack.certify(
                &memory,
                world.base,
                &seeds,
                outcome,
                Some(&mut *store),
                &watches,
            );
            captured.capture_naming_holding(&memory, world.base, &seeds, Some(store), &watches);
            store.hold(captured.attack.epoch());
            let rows: Vec<_> = captured.naming.first[..captured.naming.len]
                .iter()
                .map(|row| {
                    (
                        row.inputs,
                        row.cached_name
                            .map(|(at, len)| captured.cached_names.first[at..at + len].to_vec()),
                    )
                })
                .collect();
            let read: Vec<bool> = members
                .iter()
                .map(|member| memory.1.borrow().contains(&(member.as_ptr() as usize)))
                .collect();
            (rows, read, captured.attack.status_counts)
        };
        let (first, read, counts) = hour(&mut store);
        assert_eq!(read, [true; 3]);
        assert_eq!(counts.names_reused, 0);
        assert_eq!(first[0].1.as_deref(), Some(&text[..]));
        let (second, read, counts) = hour(&mut store);
        assert_eq!(second, first);
        assert_eq!(read, [false; 3]);
        assert_eq!(counts.names_reused, 3);
        drop(watches.notify(members[0].as_ptr() as usize, 1));
        let (third, read, counts) = hour(&mut store);
        assert_eq!(third, first);
        assert_eq!(read, [true, false, false]);
        assert_eq!(counts.names_reused, 2);
        assert!(!watches.poisoned());
    }

    #[test]
    fn division_naming_shared_group_demand_and_memos_belong_to_one_pass() {
        let mut world = scale_fixture::build(scale_fixture::Shape {
            countries: 2,
            divisions: 6,
            wings: 1,
            planes: 1,
            ships: 0,
        });
        let mut registry = template_registry(&mut world);
        let mut members = division_name_members(&mut world);
        let memory = world.live();
        let mut seeds: Vec<_> = world
            .armies
            .iter()
            .enumerate()
            .map(|(index, &army)| DivisionSeed {
                pointer: army,
                division: engine_native::read(&memory, army + engine_native::cunit::HANDLE)
                    .unwrap(),
                name_member: Some(members[index].as_ptr() as usize),
                ..Default::default()
            })
            .collect();
        carmy::capture_division_seeds(&memory, &mut seeds);
        let mut capacity = Capacities {
            movement: 0,
            status: seeds.len(),
            bans: 0,
            paths: 0,
            name_groups: world.shape.countries,
            ..Capacities::default()
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.capture_naming(&memory, world.base, &seeds);
        assert_eq!(capture.names.required, 5, "shared group demand counts once");
        assert_eq!(&capture.names.first[..capture.names.len], b"GROUP");
        assert!(
            capture.naming.first[..seeds.len()]
                .iter()
                .all(|r| r.inputs.is_some_and(|inputs| inputs.group == (0, 5)))
        );
        capture.grow(&mut capacity);
        capture.capture_owned(
            &memory,
            CaptureContext {
                base: world.base,
                game_state: world.state,
                hour: 0,
            },
            Sources {
                rows: &seeds,
                outcome: Outcome::Rows { len: seeds.len() },
            },
        );
        assert_eq!(&capture.names.first[..capture.names.len], b"GROUP");
        assert!(
            capture.naming.first[..seeds.len()]
                .iter()
                .all(|r| r.inputs.unwrap().group == (0, 5))
        );
        registry.2[8] = b'X';
        capture.decode_inputs();
        let TableView::Complete(TableRows::DivisionStatus(rows)) = capture.tables()[1].clone()
        else {
            panic!("owned naming projects with its complete status rows");
        };
        assert!(rows.iter().all(|r| {
            r.naming
                .require("DivisionStatus.naming")
                .unwrap()
                .as_ref()
                .unwrap()
                .group
                == b"GROUP"
        }));
        capture.capture_naming(&memory, world.base, &seeds);
        assert_eq!(
            &capture.names.first[..capture.names.len],
            b"XROUP",
            "a prior pass's keys are never reused"
        );
        members[0][0] ^= 1;
        capture.capture_naming(&memory, world.base, &seeds);
        assert!(capture.naming.first[0].inputs.is_none());
        assert_eq!(
            capture.naming.first[0].failure,
            Some("division name vtable mismatch")
        );
        assert!(
            capture.naming.first[1..seeds.len()]
                .iter()
                .all(|r| r.inputs.is_some())
        );
    }

    #[test]
    fn short_naming_row_arena_grows_before_every_status_name_is_published() {
        let mut world = scale_fixture::build(scale_fixture::Shape {
            countries: 2,
            divisions: 6,
            wings: 1,
            planes: 1,
            ships: 0,
        });
        let _registry = template_registry(&mut world);
        let _members = division_name_members(&mut world);
        let memory = world.live();
        let mut seeds: Vec<_> = world
            .armies
            .iter()
            .map(|&pointer| DivisionSeed {
                pointer,
                division: engine_native::read(&memory, pointer + engine_native::cunit::HANDLE)
                    .unwrap(),
                ..Default::default()
            })
            .collect();
        carmy::capture_division_seeds(&memory, &mut seeds);
        let mut capture = Captured::with_capacities(&Capacities {
            movement: 0,
            status: seeds.len(),
            bans: 0,
            paths: 0,
            name_bytes: 5,
            name_groups: world.shape.countries,
            ..Capacities::default()
        });
        capture.naming.first.truncate(1);
        capture.capture_owned(
            &memory,
            CaptureContext {
                base: world.base,
                game_state: world.state,
                hour: 0,
            },
            Sources {
                rows: &seeds,
                outcome: Outcome::Rows { len: seeds.len() },
            },
        );
        capture.decode_inputs();
        let TableView::Complete(TableRows::DivisionStatus(rows)) = capture.tables()[1].clone()
        else {
            panic!("a short naming arena must not withhold status rows");
        };
        assert_eq!(rows.len(), seeds.len());
        assert!(
            rows[0]
                .naming
                .require("DivisionStatus.naming")
                .unwrap()
                .is_some()
        );
        assert!(rows.iter().all(|row| {
            row.naming
                .require("DivisionStatus.naming")
                .unwrap()
                .is_some()
        }));
        let mut work = NamingWork::default();
        capture.observe_naming(&mut work).unwrap();
        assert_eq!(work.captured, seeds.len() as u64);
        assert!(work.refusals.is_empty());
    }

    #[test]
    fn naming_resolves_the_members_fake_template_reference() {
        let mut world = scale_fixture::build(scale_fixture::Shape {
            countries: 2,
            divisions: 6,
            wings: 1,
            planes: 1,
            ships: 0,
        });
        let _registry = template_registry(&mut world);
        let mut members = division_name_members(&mut world);
        let army = world.armies[0];
        let word = |offset| u64::from_le_bytes(world.bytes[offset..offset + 8].try_into().unwrap());
        let ordinary = word(army - world.base + carmy::STANDARD_TEMPLATE) as usize;
        let fake = world
            .armies
            .iter()
            .map(|army| word(army - world.base + carmy::STANDARD_TEMPLATE) as usize)
            .find(|&template| template != ordinary)
            .unwrap();
        let handle = word(fake - world.base + 8);
        // Naming follows the member's own template reference;
        // it does not consult DivisionRow.template.
        members[0]
            [cname_group_member::TEMPLATE_REFERENCE..cname_group_member::TEMPLATE_REFERENCE + 8]
            .copy_from_slice(&handle.to_le_bytes());
        let offset = army - world.base + carmy::FAKE_INTEL_TEMPLATE;
        world.bytes[offset..offset + 8].copy_from_slice(&(fake as u64).to_le_bytes());
        let offset = ordinary - world.base + cname_group_member::TEMPLATE_NAME_GROUP;
        world.bytes[offset..offset + 8].copy_from_slice(&0_u64.to_le_bytes());
        let mut capture = Captured::with_capacities(&Capacities {
            movement: 0,
            status: 1,
            bans: 0,
            paths: 0,
            name_bytes: 5,
            name_groups: 2,
            ..Capacities::default()
        });
        capture.capture_naming(
            &world.live(),
            world.base,
            &[DivisionSeed {
                division: 7,
                name_member: Some(members[0].as_ptr() as usize),
                ..Default::default()
            }],
        );
        assert_eq!(capture.naming.first[0].inputs.unwrap().group, (0, 5));
        assert_eq!(&capture.names.first[..capture.names.len], b"GROUP");
    }

    #[test]
    fn division_naming_keeps_displayed_programmatic_and_player_overrides() {
        let mut bytes = Vec::new();
        let mut generated = member(0, b"Generated", 7);
        generated[cname_group_member::FLAG_B] = 1;
        bytes.extend_from_slice(&generated);
        bytes.extend_from_slice(&member(0, b"Player", 8));
        let seeds = std::array::from_fn::<_, 2, _>(|index| DivisionSeed {
            division: index as u64 + 1,
            name_member: Some(0x1000 + index * cname_group_member::INPUT_SPAN),
            ..Default::default()
        });
        let mut captured = Captured::with_capacities(&Capacities {
            movement: 0,
            status: seeds.len(),
            bans: 0,
            paths: 0,
            name_bytes: 32,
            name_groups: 0,
            ..Capacities::default()
        });
        captured.capture_naming(
            &engine_native::memory::SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0,
            &seeds,
        );
        for (row, naming) in captured.status.rows.iter_mut().zip(&captured.naming.first) {
            row.division = naming.division;
            row.naming = naming.inputs;
        }
        captured.status.outcome = Outcome::Rows { len: seeds.len() };
        let TableView::Complete(TableRows::DivisionStatus(rows)) = captured.tables()[1].clone()
        else {
            panic!("programmatic naming retains the status rows");
        };
        let generated = rows[0]
            .naming
            .require("DivisionStatus.naming")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(generated.programmatic, 1);
        assert_eq!(generated.order, 7);
        assert_eq!(generated.custom, b"Generated");
        assert_eq!(
            rows[1]
                .naming
                .require("DivisionStatus.naming")
                .unwrap()
                .as_ref()
                .unwrap()
                .custom,
            b"Player"
        );
    }

    #[test]
    fn division_naming_programmatic_overrides_follow_the_resolved_template_group() {
        let mut world = scale_fixture::build(scale_fixture::Shape {
            countries: 2,
            divisions: 2,
            wings: 1,
            planes: 1,
            ships: 0,
        });
        let _registry = template_registry(&mut world);
        let mut members = division_name_members(&mut world);
        for member in &mut members {
            let custom = cname_group_member::OVERRIDE_NAME;
            member[custom..custom + 8].copy_from_slice(b"Scripted");
            member[custom + 16..custom + 24].copy_from_slice(&8_u64.to_le_bytes());
            member[cname_group_member::FLAG_B] = 1;
        }
        // Naming follows the member's nonnull template reference,
        // then reads that template's name-group pointer.
        let template =
            engine_native::read::<u64>(&world.live(), world.armies[0] + carmy::STANDARD_TEMPLATE)
                .unwrap() as usize;
        let offset = template - world.base + cname_group_member::TEMPLATE_NAME_GROUP;
        world.bytes[offset..offset + 8].fill(0);
        let seeds = std::array::from_fn::<_, 2, _>(|index| DivisionSeed {
            division: index as u64 + 1,
            name_member: Some(members[index].as_ptr() as usize),
            ..Default::default()
        });
        let mut captured = Captured::with_capacities(&Capacities {
            movement: 0,
            status: seeds.len(),
            bans: 0,
            paths: 0,
            name_bytes: 13,
            name_groups: 2,
            ..Capacities::default()
        });
        captured.capture_naming(&world.live(), world.base, &seeds);
        for (row, naming) in captured.status.rows.iter_mut().zip(&captured.naming.first) {
            row.division = naming.division;
            row.naming = naming.inputs;
        }
        captured.status.outcome = Outcome::Rows { len: seeds.len() };
        let TableView::Complete(TableRows::DivisionStatus(rows)) = captured.tables()[1].clone()
        else {
            panic!("programmatic naming retains the status rows");
        };
        for (row, custom, group) in [
            (&rows[0], b"Scripted".as_slice(), b"".as_slice()),
            (&rows[1], b"".as_slice(), b"GROUP".as_slice()),
        ] {
            let naming = row
                .naming
                .require("DivisionStatus.naming")
                .unwrap()
                .as_ref()
                .unwrap();
            assert_eq!((naming.kind, naming.programmatic), (0, 1));
            assert_eq!(naming.custom, custom);
            assert_eq!(naming.group, group);
        }
        assert_eq!(captured.names.required, 13);
    }

    #[test]
    fn attach_naming_hint_sizes_native_strings() {
        let mut world = scale_fixture::build(scale_fixture::Shape {
            countries: 24,
            divisions: 24,
            wings: 1,
            planes: 1,
            ships: 0,
        });
        let mut members = division_name_members(&mut world);
        for value in &mut members {
            let name = member(world.base, b"FifteenLetters!", 1);
            value.copy_from_slice(&name);
            value[cname_group_member::CACHED_NAME..cname_group_member::CACHED_NAME + 15]
                .copy_from_slice(b"NativeCachedOne");
            value[cname_group_member::CACHED_NAME + 16..cname_group_member::CACHED_NAME + 24]
                .copy_from_slice(&15u64.to_le_bytes());
        }
        let mut capacity = Capacities {
            status: 0,
            ..Default::default()
        };
        capacity
            .presize_names(&world.live(), world.base, world.state)
            .unwrap();
        assert!(capacity.status >= 24);
        assert!(capacity.name_bytes >= 24 * 15);
        assert!(capacity.cached_name_bytes >= 24 * 15);
        let mut capture = Captured::with_capacities(&capacity);
        let seeds: Vec<_> = members
            .iter()
            .enumerate()
            .map(|(index, member)| DivisionSeed {
                division: index as u64 + 1,
                name_member: Some(member.as_ptr() as usize),
                ..Default::default()
            })
            .collect();
        capture.capture_naming(&world.live(), world.base, &seeds);
        assert!(
            capture.naming.first[..24]
                .iter()
                .all(|row| row.inputs.is_some() && row.cached_failure.is_none())
        );
        assert_eq!(
            (capture.names.len, capture.cached_names.len),
            (24 * 15, 24 * 15)
        );
    }

    #[test]
    fn naming_heap_growth_preserves_the_first_member_and_cached_payload() {
        let mut bytes = member(0, b"", 1).to_vec();
        let source = 0x1000 + bytes.len();
        for offset in [
            cname_group_member::OVERRIDE_NAME,
            cname_group_member::CACHED_NAME,
        ] {
            bytes[offset..offset + 8].copy_from_slice(&(source as u64).to_le_bytes());
            bytes[offset + 16..offset + 24].copy_from_slice(&300u64.to_le_bytes());
            bytes[offset + 24..offset + 32].copy_from_slice(&300u64.to_le_bytes());
        }
        bytes.extend_from_slice(&[b'X'; 300]);
        bytes.push(0);
        struct Once<'a> {
            bytes: &'a [u8],
            members: std::cell::Cell<usize>,
        }
        impl Memory for Once<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if (address, output.len()) == (0x1000, cname_group_member::INPUT_SPAN) {
                    self.members.set(self.members.get() + 1);
                    if self.members.get() != 1 {
                        return false;
                    }
                }
                engine_native::memory::SliceMemory {
                    base: 0x1000,
                    bytes: self.bytes,
                }
                .copy(address, output)
            }
        }
        let memory = Once {
            bytes: &bytes,
            members: std::cell::Cell::new(0),
        };
        let mut capture = Captured::with_capacities(&Capacities {
            status: 1,
            ..Default::default()
        });
        capture.capture_naming(
            &memory,
            0,
            &[DivisionSeed {
                division: 1,
                name_member: Some(0x1000),
                ..Default::default()
            }],
        );
        assert_eq!(memory.members.get(), 1);
        assert_eq!(capture.naming.first[0].failure, None);
        assert_eq!(capture.naming.first[0].cached_failure, None);
        assert_eq!(&capture.names.first[..capture.names.len], &[b'X'; 300]);
        assert_eq!(
            &capture.cached_names.first[..capture.cached_names.len],
            &[b'X'; 300]
        );
    }

    // regression: da93900628 Size live collector storage and preserve native observation intervals
    #[test]
    fn division_naming_first_capture_grows_and_refusals_preserve_status_rows() {
        const COUNTRIES: usize = 6;
        let mut bytes = Vec::new();
        let mut seeds = Vec::new();
        for index in 0..COUNTRIES {
            seeds.push(DivisionSeed {
                division: index as u64 + 1,
                roster_owner: Some(index as u32 + 1),
                name_member: Some(0x1000 + bytes.len()),
                ..Default::default()
            });
            let mut naming = member(0, b"XX", i32::MIN + index as i32);
            naming[cname_group_member::FLAG_B] = (index % 2) as u8;
            bytes.extend_from_slice(&naming);
        }
        let mut capacity = Capacities {
            movement: 0,
            status: COUNTRIES,
            bans: 0,
            paths: 0,
            ..Capacities::default()
        };
        let mut captured = Captured::with_capacities(&capacity);
        let memory = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        captured.capture_naming(&memory, 0, &seeds);
        assert_eq!(captured.names.required, COUNTRIES * 2);
        assert_eq!(captured.naming.len, COUNTRIES);
        let mut work = NamingWork::default();
        captured.observe_naming(&mut work).unwrap();
        assert!(work.refusals.is_empty());
        assert_eq!(work.captured, COUNTRIES as u64);
        captured.grow(&mut capacity);
        captured.capture_naming(&memory, 0, &seeds);
        assert!(
            captured.naming.first[..COUNTRIES]
                .iter()
                .all(|r| r.inputs.is_some())
        );
        assert_eq!(captured.names.len, COUNTRIES * 2);
        assert_eq!(captured.naming.first[0].inputs.unwrap().order, i32::MIN);
        assert_eq!(captured.naming.first[0].inputs.unwrap().ordered, 255);
        for (row, naming) in captured
            .status
            .rows
            .iter_mut()
            .zip(&captured.naming.first)
            .take(COUNTRIES)
        {
            row.division = naming.division;
            row.naming = naming.inputs;
        }
        captured.status.outcome = Outcome::Rows { len: COUNTRIES };
        let TableView::Complete(TableRows::DivisionStatus(rows)) = captured.tables()[1].clone()
        else {
            panic!("naming rows retain status availability");
        };
        assert_eq!(rows.len(), COUNTRIES);
        assert!(rows.iter().all(|r| {
            r.naming
                .require("DivisionStatus.naming")
                .unwrap()
                .as_ref()
                .unwrap()
                .custom
                == b"XX"
        }));
        seeds[0].name_member = None;
        seeds[1].name_member = Some(0);
        seeds[2].name_member = Some(0x1000 + bytes.len());
        captured.capture_naming(&memory, 0, &seeds);
        for (row, naming) in captured
            .status
            .rows
            .iter_mut()
            .zip(&captured.naming.first)
            .take(COUNTRIES)
        {
            row.naming = naming.inputs;
        }
        let mut work = NamingWork::default();
        captured.observe_naming(&mut work).unwrap();
        assert_eq!(work.captured, (COUNTRIES - 3) as u64);
        assert_eq!(work.cached_refused, 3);
        assert_eq!(
            work.refusals,
            [
                (
                    "division cached name member identity unreadable or invalid",
                    2
                ),
                ("division cached name member pointer unreadable", 1),
                ("division name member pointer null", 1),
                ("division name member pointer unreadable", 1),
                ("division name unreadable", 1),
            ]
        );
        let TableView::Complete(TableRows::DivisionStatus(rows)) = captured.tables()[1].clone()
        else {
            panic!("three refused names do not drop their divisions");
        };
        assert_eq!(rows.len(), COUNTRIES);
        assert!(rows[..3].iter().all(|r| r.naming == Recorded::Value(None)));
        assert!(
            rows[3..]
                .iter()
                .all(|r| { r.naming.require("DivisionStatus.naming").unwrap().is_some() })
        );
    }

    #[test]
    fn division_naming_changes_defeat_baseline_reuse() {
        let mut captured = Captured::with_capacities(&Capacities {
            movement: 0,
            status: 1,
            bans: 0,
            paths: 0,
            name_bytes: 11,
            ..Capacities::default()
        });
        captured.names.first.copy_from_slice(b"GROUPplayer");
        captured.names.len = 11;
        let naming = cname_group_member::Inputs {
            kind: 0,
            order: -1,
            ordered: 255,
            programmatic: 1,
            custom: (0, 0),
            group: (0, 5),
            template: 3029 << 32 | 52,
        };
        captured.status.rows[0] = carmy::StatusRow {
            division: 7,
            naming: Some(naming),
            ..Default::default()
        };
        captured.status.outcome = Outcome::Rows { len: 1 };
        let mut baseline = std::array::from_fn(|_| unavailable("outside naming fixture"));
        baseline[TableId::DivisionStatus.index()] = captured.tables()[1].clone();
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        encoder
            .encode(
                replay_frame::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: replay_frame::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        let scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
        assert!(matches!(
            captured.table_inputs(Some(&mut projection), None)[1],
            TableInput::ComparedEqual(_)
        ));
        for changed in [
            None,
            Some(cname_group_member::Inputs { kind: 2, ..naming }),
            Some(cname_group_member::Inputs { order: 0, ..naming }),
            Some(cname_group_member::Inputs {
                ordered: 1,
                ..naming
            }),
            Some(cname_group_member::Inputs {
                programmatic: 255,
                ..naming
            }),
            Some(cname_group_member::Inputs {
                custom: (5, 6),
                ..naming
            }),
            Some(cname_group_member::Inputs {
                group: (0, 0),
                ..naming
            }),
        ] {
            captured.status.rows[0].naming = changed;
            let [_, TableInput::ComparedChanges { indices, rows, .. }, ..] =
                captured.table_inputs(Some(&mut projection), None)
            else {
                panic!("every naming operand must defeat reuse");
            };
            assert_eq!(indices, [0]);
            assert_eq!(TableView::Complete(rows), captured.tables()[1]);
        }
        // The member's template reference is recorded only by a
        // name_template_witness build, where a changed one defeats reuse.
        let moved = 3005 << 32 | 52;
        captured.status.rows[0].naming = Some(cname_group_member::Inputs {
            template: moved,
            ..naming
        });
        let input = captured.table_inputs(Some(&mut projection), None);
        assert_eq!(
            matches!(input[1], TableInput::ComparedChanges { .. }),
            NAME_TEMPLATE_WITNESS
        );
        let TableView::Complete(TableRows::DivisionStatus(rows)) = captured.tables()[1].clone()
        else {
            panic!("the naming fixture's status rows are complete");
        };
        assert_eq!(
            rows[0].name_template,
            Recorded::Value(Witnessed(NAME_TEMPLATE_WITNESS.then_some(moved)))
        );
        captured.status.rows[0].naming = Some(naming);
        captured.names.first[0] ^= 1;
        assert!(matches!(
            captured.table_inputs(Some(&mut projection), None)[1],
            TableInput::ComparedChanges { .. }
        ));
        captured.status.rows[0].naming = Some(cname_group_member::Inputs {
            group: (usize::MAX, 5),
            ..naming
        });
        assert!(matches!(
            captured.tables()[1],
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn invalid_order_reason_projects_and_changes_without_losing_status_or_naming() {
        let mut captured = Captured::with_capacities(&Capacities {
            movement: 0,
            status: 1,
            bans: 0,
            paths: 0,
            name_bytes: 1,
            ..Capacities::default()
        });
        captured.names.first[0] = b'X';
        captured.names.len = 1;
        captured.status.rows[0] = carmy::StatusRow {
            division: 7,
            entrenchment: i64::MIN,
            invalid_orders: 0xff,
            invalid_orders_reason: Some(i32::MIN),
            naming: Some(cname_group_member::Inputs {
                custom: (0, 1),
                ..Default::default()
            }),
            ..Default::default()
        };
        captured.status.outcome = Outcome::Rows { len: 1 };
        let mut work = CounterWork::default();
        captured.observe_counter_inputs(&mut work).unwrap();
        assert_eq!((work.captured, work.refused), (1, 0));
        let mut baseline = std::array::from_fn(|_| unavailable("outside reason fixture"));
        baseline[TableId::DivisionStatus.index()] = captured.tables()[1].clone();
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        encoder
            .encode(
                replay_frame::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: Default::default(),
                },
                false,
            )
            .unwrap();
        let scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
        assert!(matches!(
            captured.table_inputs(Some(&mut projection), None)[1],
            TableInput::ComparedEqual(_)
        ));
        for reason in [None, Some(0), Some(i32::MAX)] {
            captured.status.rows[0].invalid_orders_reason = reason;
            let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
            assert!(!matches!(
                captured.table_inputs(Some(&mut projection), None)[1],
                TableInput::ComparedEqual(_)
            ));
            let TableView::Complete(TableRows::DivisionStatus(rows)) = captured.tables()[1].clone()
            else {
                panic!("an absent reason does not withhold status");
            };
            assert_eq!(rows[0].invalid_orders_reason, Recorded::Value(reason));
            assert_eq!(rows[0].entrenchment, i64::MIN);
            assert_eq!(
                rows[0]
                    .naming
                    .require("naming")
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .custom,
                b"X"
            );
            captured.observe_counter_inputs(&mut work).unwrap();
        }
        assert_eq!((work.captured, work.refused), (3, 1));
        captured.status.rows[0].invalid_orders = 0;
        captured.status.rows[0].invalid_orders_reason = None;
        captured.observe_counter_inputs(&mut work).unwrap();
        assert_eq!((work.captured, work.refused), (3, 1));
    }

    /// The row records what the camouflage category selector takes from the division, never the
    /// weights, and only a different decision or refusal changes it.
    #[test]
    fn camouflage_projects_the_decision_and_changes_only_with_it() {
        let mut captured = Captured::with_capacities(&Capacities {
            status: 1,
            ..Capacities::default()
        });
        let decided = |category| {
            Some(carmy::Camouflage {
                category: Some(category),
            })
        };
        captured.status.rows[0] = carmy::StatusRow {
            division: 7,
            camouflage: decided(20_006),
            ..Default::default()
        };
        captured.status.outcome = Outcome::Rows { len: 1 };
        let rows = |captured: &Captured| {
            let TableView::Complete(TableRows::DivisionStatus(rows)) = captured.tables()[1].clone()
            else {
                panic!("camouflage does not withhold status");
            };
            rows
        };
        let row = &rows(&captured)[0];
        assert_eq!(
            row.camouflage,
            Recorded::Value(Some(DivisionCamouflage {
                category: Some(20_006)
            }))
        );
        assert_eq!(
            (row.acclimatization_maximum, row.acclimatization.len()),
            (None, 0)
        );
        let mut baseline = std::array::from_fn(|_| unavailable("outside camouflage fixture"));
        baseline[TableId::DivisionStatus.index()] = captured.tables()[1].clone();
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        encoder
            .encode(
                replay_frame::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: Default::default(),
                },
                false,
            )
            .unwrap();
        let scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
        assert!(matches!(
            captured.table_inputs(Some(&mut projection), None)[1],
            TableInput::ComparedEqual(_)
        ));
        for (camouflage, recorded) in [
            (decided(36_465), Some(Some(36_465))),
            (Some(carmy::Camouflage { category: None }), Some(None)),
            (None, None),
        ] {
            captured.status.rows[0].camouflage = camouflage;
            let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
            assert!(!matches!(
                captured.table_inputs(Some(&mut projection), None)[1],
                TableInput::ComparedEqual(_)
            ));
            assert_eq!(
                rows(&captured)[0].camouflage,
                Recorded::Value(recorded.map(|category| DivisionCamouflage { category }))
            );
        }
    }

    /// The Presentation job's DivisionStatus passes (status, dig-in,
    /// naming) and the attack pass, for a quiet hour carried through the
    /// army_stat store and for an hour that reads every division.
    #[test]
    #[ignore = "release benchmark: DivisionStatus with and without the army_stat carry"]
    fn presentation_carry_scale_bench() {
        use super::super::attack_stats::ArmyStatStore;
        use crate::army_stat_changes::Watches;
        if cfg!(debug_assertions) {
            panic!("run the carry benchmark in release");
        }
        for divisions in [5_000, 30_000] {
            let shape = scale_fixture::Shape {
                countries: 1_500,
                divisions,
                wings: 1,
                planes: 1,
                ships: 0,
            };
            let mut world = scale_fixture::build(shape);
            let _registry = template_registry(&mut world);
            let _members = division_name_members(&mut world);
            let memory = world.live();
            let mut census = crate::division_census::Census::new();
            loop {
                census.clear();
                census.capture(&memory, world.base, world.state, divisions);
                if !census.overflowed {
                    break;
                }
                census.grow();
            }
            let inputs = census.unit_inputs().unwrap();
            let context = CaptureContext {
                base: world.base,
                game_state: world.state,
                hour: 1,
            };
            let mut capture = Captured::with_capacities(&Capacities {
                status: divisions,
                name_groups: shape.countries,
                ..Capacities::default()
            });
            capture.reserve_sources(divisions);
            let watches = Watches::enabled(2 * divisions);
            let mut store = ArmyStatStore::new();
            let sources = || Sources {
                rows: inputs.seeds(),
                outcome: inputs.outcome(),
            };
            // Per hour: [status, dig-in, naming, attack] in milliseconds, and
            // the published tables.
            let mut hour = |store: Option<&mut ArmyStatStore>| {
                let mut store = store;
                capture.capture_owned_observed(
                    &memory,
                    context,
                    sources(),
                    store.as_deref_mut(),
                    &watches,
                );
                assert_eq!(capture.status.outcome, Outcome::Rows { len: divisions });
                assert_eq!(capture.naming.len, divisions);
                let begin = std::time::Instant::now();
                capture.capture_attack(&memory, context, sources(), store);
                let attack = begin.elapsed().as_secs_f64() * 1e3;
                let mut frequency = 0;
                // SAFETY: writes the frequency into a local.
                unsafe {
                    windows_sys::Win32::System::Performance::QueryPerformanceFrequency(
                        &mut frequency,
                    )
                };
                let [status, dig_in, naming, ..] = capture
                    .phase_ticks()
                    .map(|ticks| ticks as f64 * 1e3 / frequency as f64);
                capture.decode_inputs();
                ([status, dig_in, naming, attack], capture.tables())
            };
            let mut tables = Vec::new();
            for (form, carried) in [("read_every_division", false), ("quiet_carried", true)] {
                let mut samples = Vec::new();
                let mut published = None;
                for _ in 0..25 {
                    let (sample, hour_tables) = hour(carried.then_some(&mut store));
                    samples.push(sample);
                    published = Some(hour_tables);
                }
                // A carried hour publishes what a read hour does.
                tables.push(published.unwrap());
                // The first hours fill the store.
                samples.drain(..3);
                let median = |part: usize| {
                    let mut values: Vec<f64> = samples.iter().map(|hour| hour[part]).collect();
                    values.sort_by(f64::total_cmp);
                    values[values.len() / 2]
                };
                println!(
                    "PRESENTATION_CARRY divisions={divisions} form={form} status_ms={:.3} dig_in_ms={:.3} naming_ms={:.3} attack_ms={:.3}",
                    median(0),
                    median(1),
                    median(2),
                    median(3)
                );
            }
            assert_eq!(tables[0], tables[1]);
            assert!(!watches.poisoned());
        }
    }

    #[test]
    #[ignore = "release callback benchmark: 30,000 divisions with and without naming"]
    fn division_naming_callback_scale_bench() {
        if cfg!(debug_assertions) {
            panic!("run the naming benchmark in release");
        }
        assert!(
            std::env::var_os("BATTLE_PLAN_UNIT_VALIDATE").or_else(|| std::env::var_os("HOI4_UNIT_VALIDATE")).is_none(),
            "unset BATTLE_PLAN_UNIT_VALIDATE to measure callback time"
        );
        for countries in [1_500, 5_000] {
            let shape = scale_fixture::Shape {
                countries,
                divisions: 30_000,
                wings: 1,
                planes: 1,
                ships: 0,
            };
            let mut world = scale_fixture::build(shape);
            let _registry = template_registry(&mut world);
            let mut members = division_name_members(&mut world);
            let memory = world.live();
            let mut census = crate::division_census::Census::new();
            loop {
                census.clear();
                census.capture(&memory, world.base, world.state, shape.divisions);
                if !census.overflowed {
                    break;
                }
                census.grow();
            }
            let inputs = census.unit_inputs().unwrap();
            assert_eq!(
                inputs.outcome(),
                Outcome::Rows {
                    len: shape.divisions
                }
            );
            let capacity = Capacities {
                movement: 0,
                status: shape.divisions,
                bans: 0,
                paths: 0,
                name_bytes: shape.divisions * b"Renamed".len(),
                name_groups: shape.countries,
                ..Capacities::default()
            };
            let mut capture = Captured::with_capacities(&capacity);
            capture.reserve_sources(shape.divisions);
            capture.status_packet.enable_growth();
            // Both timers run the same production status callback. Only the
            // second includes the actual hourly naming reader and arena writes.
            for form in ["generated_shared_group", "inline_player_renames"] {
                if form == "inline_player_renames" {
                    for (index, bytes) in members.iter_mut().enumerate() {
                        *bytes = member(world.base, b"Renamed", index as i32);
                    }
                }
                let mut callback = |naming| {
                    assert_eq!(
                        capture.status_packet.capture(
                            &memory,
                            world.base,
                            world.state,
                            inputs.seeds(),
                            inputs.outcome(),
                            shape.divisions,
                        ),
                        Outcome::Rows {
                            len: shape.divisions
                        }
                    );
                    if naming {
                        capture.capture_naming(&memory, world.base, inputs.seeds());
                    }
                };
                let without = scale_fixture::time(|| callback(false));
                let with = scale_fixture::time(|| callback(true));
                println!(
                    "DIVISION_NAMING countries={countries} divisions={} without_p50_ms={:.6} without_p95_ms={:.6} with_p50_ms={:.6} with_p95_ms={:.6} synthetic_names={form}",
                    shape.divisions, without.0, without.1, with.0, with.1
                );
                assert_eq!(
                    capture.names.len,
                    if form == "generated_shared_group" {
                        5
                    } else {
                        shape.divisions * b"Renamed".len()
                    }
                );
                assert!(
                    capture.naming.first[..shape.divisions]
                        .iter()
                        .all(|r| r.inputs.is_some())
                );
            }
        }
    }

    fn presentation_fixture() -> (DivisionSeed, Vec<u8>) {
        let seed = DivisionSeed {
            pointer: 0x8000,
            division: 7,
            primary_vtable: Some(carmy::VTABLE_RVA as u64),
            handle: Some(7),
            class: Some(0),
            ordinary_route: Some([0; 16]),
            alternate_route: Some([0; 16]),
            combat: Some([0; 16]),
            location: Some(0),
            movement_numerator: Some(13),
            movement_target: Some(0),
            movement_source: Some(0),
            naval_transfer_membership: Some(0),
            map_entity: Some(0),
            retreat: Some(0),
            strategic_redeployment: Some(0),
            ..DivisionSeed::default()
        };
        let mut bytes = vec![0; 0x400];
        // Status authenticates the country/link roots even when its row
        // capacity admits no unit. Supply the required null/sentinel slots.
        for (offset, data, count) in [
            (engine_native::cgame_state::COUNTRIES, 0x1380_u64, 2_u32),
            (engine_native::cgame_state::COUNTRY_LINKS, 0x1390_u64, 1_u32),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&data.to_le_bytes());
            bytes[offset + 8..offset + 12].copy_from_slice(&count.to_le_bytes());
            bytes[offset + 12..offset + 16].copy_from_slice(&count.to_le_bytes());
        }
        (seed, bytes)
    }

    #[test]
    fn vacant_presentation_has_no_owned_allocations_or_rows() {
        let mut capture = Captured::vacant();
        assert_eq!(capture.allocated_bytes(), 0);
        assert_eq!(capture.decode_inputs(), [0; 2]);
        assert!(
            capture
                .tables()
                .iter()
                .all(|view| matches!(view, TableView::Unavailable { .. }))
        );
    }

    #[test]
    fn owned_failure_keeps_stale_packets_unpublished() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        struct PanicAt<'a> {
            memory: engine_native::memory::SliceMemory<'a>,
            address: usize,
        }
        impl Memory for PanicAt<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                assert_ne!(address, self.address, "synthetic acquisition panic");
                self.memory.copy(address, destination)
            }
        }

        let (seed, bytes) = presentation_fixture();
        let seeds = [seed];
        let context = CaptureContext {
            base: 0,
            game_state: 0x1000,
            hour: 0,
        };
        let sources = || Sources {
            rows: &seeds,
            outcome: Outcome::Rows { len: 1 },
        };
        for offset in [
            engine_native::cgame_state::PROVINCES,
            engine_native::cgame_state::COUNTRIES,
        ] {
            let mut capture = Captured::with_capacities(&Capacities {
                movement: 1,
                status: 1,
                bans: 1,
                paths: 0,
                name_bytes: 0,
                name_groups: 0,
                cached_name_bytes: 0,
                geometry: cprovince::GeometryCapacities {
                    provinces: 1,
                    neighbour_entries: 0,
                },
                membership: carmy::StatusCapacities::new(1).membership,
                supply_origins: 0,
                combat_references: 0,
            });
            capture.reserve_sources(1);
            capture.capture_owned(
                &engine_native::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                context,
                sources(),
            );
            let memory = PanicAt {
                memory: engine_native::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                address: 0x1000 + offset,
            };
            assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    capture.capture_owned(&memory, context, sources());
                }))
                .is_err()
            );
            capture
                .attack
                .selected_sources
                .push((seed.division, Some([1, 2, 3])));
            let report = capture.fail_owned_capture("worker failed");
            assert!(capture.attack.selected_sources.is_empty());
            assert_eq!(report.ticks, [0; N]);
            assert_eq!(capture.decode_inputs(), [0; 2]);
            assert_eq!(
                capture.outcomes(),
                [Outcome::Unavailable {
                    reason: "worker failed"
                }; N]
            );
            assert!(
                capture
                    .tables()
                    .iter()
                    .all(|view| matches!(view, TableView::Unavailable { .. }))
            );
            capture.reset();
            capture.fail_owned_capture("worker stopped");
            assert_eq!(capture.decode_inputs(), [0; 2]);
            assert_eq!(
                capture.outcomes(),
                [Outcome::Unavailable {
                    reason: "worker stopped"
                }; N]
            );
        }
    }

    const PRESENTED: usize = 48;

    /// The scale fixture's world with template names and name members and
    /// its census of [`PRESENTED`] divisions, whose status and naming rows
    /// are all read.
    struct Populated {
        world: scale_fixture::World,
        _registry: (Box<[u8; 32]>, Vec<u8>, Box<[u8; 40]>),
        _members: Vec<[u8; cname_group_member::INPUT_SPAN]>,
        census: crate::division_census::Census,
    }

    impl Populated {
        const SHAPE: scale_fixture::Shape = scale_fixture::Shape {
            countries: 4,
            divisions: PRESENTED,
            wings: 1,
            planes: 1,
            ships: 0,
        };

        fn new() -> Self {
            Self::with_shape(Self::SHAPE)
        }

        fn with_shape(shape: scale_fixture::Shape) -> Self {
            let mut world = scale_fixture::build(shape);
            let registry = template_registry(&mut world);
            let members = division_name_members(&mut world);
            let mut census = crate::division_census::Census::new();
            loop {
                census.clear();
                census.capture(&world.live(), world.base, world.state, shape.divisions);
                if !census.overflowed {
                    break;
                }
                census.grow();
            }
            assert_eq!(
                census.unit_inputs().unwrap().outcome(),
                Outcome::Rows {
                    len: shape.divisions
                }
            );
            Self {
                world,
                _registry: registry,
                _members: members,
                census,
            }
        }

        fn context(&self) -> CaptureContext {
            CaptureContext {
                base: self.world.base,
                game_state: self.world.state,
                hour: 1,
            }
        }

        fn capture(&self) -> Captured {
            let divisions = self.census.unit_inputs().unwrap().seeds().len();
            let mut capture = Captured::with_capacities(&Capacities {
                status: divisions,
                name_groups: divisions,
                ..Capacities::default()
            });
            capture.reserve_sources(divisions);
            capture
        }
    }

    #[test]
    fn presentation_status_split_keeps_rows_holds_and_certificates() {
        use super::super::attack_stats::ArmyStatStore;
        use crate::army_stat_changes::Watches;
        let populated = Populated::with_shape(scale_fixture::Shape {
            countries: 500,
            divisions: 1_600,
            wings: 1,
            planes: 1,
            ships: 0,
        });
        let inputs = populated.census.unit_inputs().unwrap();
        let mut faulted = inputs.seeds().to_vec();
        faulted[555].entrenchment = None;
        faulted[1_111].handle = None;
        for (workers, owned) in [1, 2, 3, 7, MAX_STATUS_PARTS]
            .into_iter()
            .flat_map(|workers| [false, true].map(|owned| (workers, owned)))
        {
            let mut serial = populated.capture();
            let mut split = populated.capture();
            split.status_workers = workers;
            if owned {
                split.enable_owned_settlement();
            }
            let serial_changes = Watches::enabled(2 * inputs.seeds().len());
            let split_changes = Watches::enabled(2 * inputs.seeds().len());
            let mut serial_store = ArmyStatStore::new();
            let mut split_store = held_store::Loan::fixture(Box::new(ArmyStatStore::new()));
            let mut warmed = None;
            for hour in [1, 2, 3, 4, 5] {
                // An earlier scalar fault outranks a later preparation fault.
                // The following hour must recover its rows without losing holds.
                let rows = if hour == 2 {
                    &faulted[..]
                } else {
                    inputs.seeds()
                };
                let sources = || Sources {
                    rows,
                    outcome: inputs.outcome(),
                };
                let context = CaptureContext {
                    hour,
                    ..populated.context()
                };
                let memory = populated.world.live();
                serial.capture_status_observed(
                    &memory,
                    context,
                    sources(),
                    Some(&mut serial_store),
                    &serial_changes,
                );
                serial.capture_attack(&memory, context, sources(), Some(&mut serial_store));
                split.prepare_status_observed(
                    &memory,
                    context,
                    sources(),
                    Some(&mut split_store),
                    &split_changes,
                );
                for stage in 0..2 {
                    assert_eq!(split.status_part_count(), status_parts_for(1_600, workers));
                    let mut parts: Vec<_> = (0..split.status_part_count())
                        .map(|index| split.take_status_part(index))
                        .collect();
                    std::thread::scope(|scope| {
                        for part in parts.iter_mut().rev() {
                            let world = &populated.world;
                            let changes = &split_changes;
                            scope
                                .spawn(move || part.capture(&world.live(), context, rows, changes));
                        }
                    });
                    for (index, part) in parts.into_iter().enumerate() {
                        split.restore_status_part(index, part, 0);
                    }
                    if stage == 0 {
                        split.prepare_status_observed(
                            &memory,
                            context,
                            sources(),
                            Some(&mut split_store),
                            &split_changes,
                        );
                    }
                }
                split.capture_status_observed(
                    &memory,
                    context,
                    sources(),
                    Some(&mut split_store),
                    &split_changes,
                );
                split.capture_attack(&memory, context, sources(), Some(&mut split_store));
                serial.decode_inputs();
                let returned = split.retain_owned_settlement(
                    split_store,
                    Some(Arc::new(crate::division_census::UnitInputs::fixture(
                        rows.to_vec(),
                        inputs.outcome(),
                    ))),
                    context,
                );
                split.decode_inputs();
                split_store = returned.or_else(|| split.take_settled_store()).unwrap();
                assert_eq!(
                    split.outcomes(),
                    serial.outcomes(),
                    "workers={workers}, hour={hour}"
                );
                assert_eq!(
                    split.tables(),
                    serial.tables(),
                    "workers={workers}, hour={hour}"
                );
                assert_eq!(split.attack.held_maxima, serial.attack.held_maxima);
                assert_eq!(split.attack.status_counts, serial.attack.status_counts);
                assert_eq!(split.attack.epoch(), serial.attack.epoch());
                assert_eq!(split.attack.carryable, serial.attack.carryable);
                assert_eq!(split.attack.experience, serial.attack.experience);
                if hour > 1 {
                    assert!(split.attack.held_maxima.iter().any(Option::is_some));
                }
                let mut serial_names = NamingWork::default();
                let mut split_names = NamingWork::default();
                serial.observe_naming(&mut serial_names).unwrap();
                split.observe_naming(&mut split_names).unwrap();
                assert_eq!(
                    serde_json::to_value(split_names.status()).unwrap(),
                    serde_json::to_value(serial_names.status()).unwrap()
                );
                assert!(!serial_changes.poisoned());
                assert!(!split_changes.poisoned());
                if hour >= 4 {
                    let bytes = split.allocated_bytes();
                    assert_eq!(
                        *warmed.get_or_insert(bytes),
                        bytes,
                        "steady scratch is reused"
                    );
                }
            }
        }
    }

    #[test]
    fn source_preparation_keeps_country_header_failure_on_the_ordered_path() {
        struct MissingCountries<'a, M>(&'a M, usize);
        impl<M: Memory> Memory for MissingCountries<'_, M> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                address != self.1 && self.0.copy(address, destination)
            }
        }
        let populated = Populated::new();
        let inputs = populated.census.unit_inputs().unwrap();
        let live = populated.world.live();
        let memory = MissingCountries(
            &live,
            populated.world.state + engine_native::cgame_state::COUNTRY_LINKS,
        );
        assert!(ccountry::Reader::new(&memory, populated.world.base, populated.world.state).is_err());
        let store = super::super::attack_stats::ArmyStatStore::new();
        let mut part = super::super::attack_stats::PreparationPart::default();
        part.range = 0..inputs.seeds().len();
        part.capture(
            &memory,
            populated.world.base,
            populated.world.state,
            inputs.seeds(),
            Some(&store),
            &crate::army_stat_changes::Observer,
        );
        assert!(part.source_reads().is_none());
        assert!(!part.can_settle_sources());
        assert!(part.get(0).is_some_and(|row| row.selection.is_none()));
    }

    #[test]
    fn optional_preparation_staging_refusal_keeps_the_ordered_path_available() {
        let populated = Populated::new();
        let inputs = populated.census.unit_inputs().unwrap();
        let mut capture = populated.capture();
        let staged = crate::test_support::with_allocations_denied(|| {
            capture.begin_preparation(
                &Sources {
                    rows: inputs.seeds(),
                    outcome: inputs.outcome(),
                },
                None,
            )
        });
        assert!(!staged);
        assert!(!capture.preparation_pending());
        assert!(!capture.status_prepared);
        assert!(capture.preparation_parts.is_empty());
    }

    #[test]
    fn presentation_status_preparation_allocation_refusal_is_not_retried() {
        let populated = Populated::new();
        let inputs = populated.census.unit_inputs().unwrap();
        let sources = || Sources {
            rows: inputs.seeds(),
            outcome: inputs.outcome(),
        };
        let mut capture = Captured::vacant();
        crate::test_support::with_allocations_denied(|| {
            capture.prepare_status_observed(
                &populated.world.live(),
                populated.context(),
                sources(),
                None,
                &crate::army_stat_changes::Observer,
            );
        });
        assert_eq!(capture.status_part_count(), 0);
        let refused = Outcome::Unavailable {
            reason: "presentation row allocation failed",
        };
        assert_eq!(capture.status.outcome, refused);
        capture.capture_status_observed(
            &populated.world.live(),
            populated.context(),
            sources(),
            None,
            &crate::army_stat_changes::Observer,
        );
        assert_eq!(capture.status.outcome, refused);
        assert!(!capture.decode_pending);
    }

    #[test]
    fn presentation_status_parts_follow_live_divisions_and_pool_workers() {
        for divisions in [0, 1, 511, 512, 513, 1_500, 5_000, 50_000] {
            for workers in 1..=MAX_STATUS_PARTS {
                let count = status_parts_for(divisions, workers);
                assert!((1..=workers).contains(&count));
                assert_eq!(
                    count,
                    divisions.div_ceil(STATUS_PART_ENTRIES).clamp(1, workers)
                );
            }
        }
    }

    /// The movement pass and the status side read nothing of each other:
    /// run side by side on two threads, each through its own memory handle
    /// as their two jobs run, then the attack pass as the Presentation job
    /// runs it, they publish what the single pass publishes.
    #[test]
    fn cold_attach_and_post_gap_keyframes_admit_every_experience_column() {
        use replay::{spool::Record, spool::encode_record};
        use replay_data::{rows::RowCodec, rows::TableRows};
        use replay_frame::{
            branches::Gap, frame::DecodedFrame, frame::FrameEncoder, frame::Metrics,
        };
        let populated = Populated::new();
        let inputs = populated.census.unit_inputs().unwrap();
        let mut captured = populated.capture();
        let mut encoder = FrameEncoder::new();
        for hour in [1, 3] {
            if hour == 3 {
                encode_record(&Record::Gap(Gap {
                    branch: 1,
                    first_hour: 2,
                    last_hour: 2,
                    reason: "synthetic spool gap".into(),
                }))
                .unwrap();
            }
            captured.reset();
            let context = CaptureContext {
                hour,
                ..populated.context()
            };
            let sources = || Sources {
                rows: inputs.seeds(),
                outcome: inputs.outcome(),
            };
            captured.capture_owned(&populated.world.live(), context, sources());
            captured.capture_attack(&populated.world.live(), context, sources(), None);
            captured.decode_inputs();
            let views = captured.tables();
            assert!(
                matches!(&views[1], TableView::Complete(TableRows::DivisionStatus(rows)) if rows.len() == PRESENTED)
            );
            let mut tables = std::array::from_fn(|_| unavailable("outside first-hour audit"));
            for (table, view) in TABLES.into_iter().zip(views) {
                match &view {
                    TableView::Complete(rows) => {
                        for row in rows.iter() {
                            assert_eq!(row.not_recorded_fields(), 0, "{table:?} at {hour}");
                        }
                    }
                    TableView::Unavailable { reason } => assert!(!reason.is_empty(), "{table:?}"),
                    _ => panic!("{table:?} has no admitted cold baseline"),
                }
                tables[table.index()] = view;
            }
            // Commander progression rows have no historically-absent current
            // columns; empty acquisition is distinct from a refused table.
            let leader = replay_data::rows::UnitLeaderRow {
                id: 7,
                experience: -11,
                ..Default::default()
            };
            assert_eq!(leader.not_recorded_fields(), 0);
            tables[TableId::UnitLeaders.index()] =
                TableView::Complete(TableRows::UnitLeaders(vec![leader]));
            tables[TableId::UnitLeaderTraitExperience.index()] =
                TableView::Complete(TableRows::UnitLeaderTraitExperience(vec![
                    replay_data::rows::UnitLeaderTraitExperienceRow {
                        leader_reference: 7,
                        experience: -17,
                        ..Default::default()
                    },
                ]));
            let encoded = encoder
                .encode(
                    DecodedFrame {
                        branch: 1,
                        hour,
                        tables: tables.map(Into::into),
                        metrics: Metrics::default(),
                    },
                    true,
                )
                .unwrap();
            assert!(encoded.keyframe);
            encode_record(&Record::Frame(encoded)).unwrap();
        }
    }

    #[test]
    fn presentation_speed_terms_run_beside_status_and_movement() {
        let populated = Populated::new();
        let world = &populated.world;
        let inputs = populated.census.unit_inputs().unwrap();
        let context = populated.context();
        let sources = || Sources {
            rows: inputs.seeds(),
            outcome: inputs.outcome(),
        };
        let memory = world.live();
        let mut whole = populated.capture();
        whole.capture_owned(&memory, context, sources());
        whole.capture_attack(&memory, context, sources(), None);
        whole.decode_inputs();
        assert_eq!(whole.status.outcome, Outcome::Rows { len: PRESENTED });
        assert_eq!(whole.naming.len, PRESENTED);

        let mut split = populated.capture();
        split.capture_owned(&memory, context, sources());
        let speed_bytes = split.speed_terms.allocated_bytes();
        assert!(speed_bytes > 0);
        let bytes = split.allocated_bytes();
        let mut pass = crate::test_support::with_allocations_denied(|| split.take_movement());
        assert_eq!(split.speed_terms.allocated_bytes(), speed_bytes);
        let mut terms = crate::test_support::with_allocations_denied(|| split.take_speed_terms());
        assert_eq!(split.speed_terms.allocated_bytes(), 0);
        assert_eq!(
            split.allocated_bytes() + pass.allocated_bytes() + terms.allocated_bytes(),
            bytes
        );
        let (movement, speed_terms) = std::thread::scope(|scope| {
            let speed_terms = scope.spawn(|| {
                let counted = world.live();
                let reads = crate::read_accounting::Capture::new();
                let report = terms.capture(&counted, context.base, context.game_state, sources());
                let counts = reads.finish();
                assert!(counts[TableId::MovementStateTerms.index()].copy_calls > 0);
                assert_eq!(counts[TableId::DivisionMovement.index()].copy_calls, 0);
                assert_eq!(report.ticks[..7], [0; 7]);
                report
            });
            let movement = scope.spawn(|| {
                let counted = world.live();
                let reads = crate::read_accounting::Capture::new();
                let report = pass.capture(
                    &counted,
                    context.base,
                    context.game_state,
                    sources(),
                    Some(context.hour),
                );
                let counts = reads.finish();
                for table in speed_terms::TABLES {
                    assert_eq!(counts[table.index()].copy_calls, 0, "{table:?}");
                }
                assert_eq!(report.ticks[7..], [0; speed_terms::N]);
                report
            });
            let counted = world.live();
            let reads = crate::read_accounting::Capture::new();
            let report = split.capture_status_holding(&counted, context, sources(), None);
            split.capture_attack(&counted, context, sources(), None);
            let counts = reads.finish();
            for table in speed_terms::TABLES {
                assert_eq!(counts[table.index()].copy_calls, 0, "{table:?}");
            }
            assert_eq!(report.ticks[7..], [0; speed_terms::N]);
            (movement.join().unwrap(), speed_terms.join().unwrap())
        });
        crate::test_support::with_allocations_denied(|| {
            split.restore_movement(pass);
            split.restore_speed_terms(terms);
        });
        split.decode_inputs();
        assert_eq!(split.outcomes(), whole.outcomes());
        assert_eq!(split.tables(), whole.tables());
        assert_eq!(split.naming.len, PRESENTED);
        assert_eq!(movement.ticks[3], split.land_ticks);
        assert_eq!(split.phase_ticks()[1], movement.ticks[1]);
        let mut report = CaptureReport::default();
        report.merge_movement(movement);
        report.merge_speed_terms(speed_terms);
        assert_eq!(report.ticks[7..], speed_terms.ticks[7..]);
    }

    /// The census's source packet stands in for each pass's own reads of the
    /// same fields, reading no more and publishing every row unchanged.
    #[test]
    fn census_source_packet_publishes_what_each_pass_reads_for_itself() {
        let populated = Populated::new();
        let inputs = populated.census.unit_inputs().unwrap();
        assert!(inputs.seeds().iter().all(|seed| seed.selected.is_some()));
        let own: Vec<_> = inputs
            .seeds()
            .iter()
            .map(|seed| DivisionSeed {
                route_head: None,
                location_id: None,
                selected: None,
                ..*seed
            })
            .collect();
        let context = populated.context();
        let capture = |rows: &[DivisionSeed]| {
            let memory = populated.world.live();
            let sources = || Sources {
                rows,
                outcome: inputs.outcome(),
            };
            let mut captured = populated.capture();
            let reads = crate::read_accounting::Capture::new();
            captured.capture_owned(&memory, context, sources());
            captured.capture_attack(&memory, context, sources(), None);
            let counts = reads.finish();
            captured.decode_inputs();
            let calls = [
                TableId::DivisionStatus,
                TableId::DivisionRetreatBans,
                TableId::MovementStateTerms,
            ]
            .map(|table| counts[table.index()].copy_calls);
            (captured, calls)
        };
        let (shared, shared_calls) = capture(inputs.seeds());
        let (separate, separate_calls) = capture(&own);
        assert_eq!(shared.outcomes(), separate.outcomes());
        assert_eq!(shared.tables(), separate.tables());
        for (shared, separate) in shared_calls.into_iter().zip(separate_calls) {
            assert!(shared <= separate, "{shared} > {separate}");
        }
    }

    /// Map leaves out the status side, the speed terms and the movement
    /// pass's land and dig-in walks: it reads nothing for them and refuses
    /// them, while its DivisionMovement and the reads behind it equal Full's.
    #[test]
    fn map_presentation_reads_only_the_movement_full_reads() {
        let populated = Populated::new();
        let inputs = std::sync::Arc::clone(populated.census.unit_inputs().unwrap());
        let sources = || Sources {
            rows: inputs.seeds(),
            outcome: inputs.outcome(),
        };
        let memory = engine_native::read_counts::CountedMemory::new(populated.world.live());
        let countries = std::sync::Arc::new(
            engine_native::country_memory::Cache::try_with_capacity(0).unwrap(),
        );
        let mut runs = Vec::new();
        for map in [false, true] {
            let mut captured =
                super::super::Captured::with_capacities(&super::super::Capacities::default());
            captured.division_presentation = populated.capture();
            captured.install_unit_inputs(Some(std::sync::Arc::clone(&inputs)));
            let context = populated.context();
            let mut request = captured.take_presentation_request(
                &countries,
                context.base,
                context.game_state,
                context.hour,
                None,
            );
            let mut movement = request.take_movement();
            let reads = crate::read_accounting::Capture::new();
            let report = if map {
                movement.pass.omit_status();
                let ticks = movement.pass.capture(
                    &memory,
                    context.base,
                    context.game_state,
                    sources(),
                    Some(context.hour),
                );
                let status = request.captured.omit_status(super::super::NOT_RECORDED);
                request.restore_movement(movement, Ok(ticks), Ok(status))
            } else {
                let mut terms = request.take_speed_terms();
                let speed_ticks =
                    terms
                        .pass
                        .capture(&memory, context.base, context.game_state, sources());
                let ticks = movement.pass.capture(
                    &memory,
                    context.base,
                    context.game_state,
                    sources(),
                    Some(context.hour),
                );
                let status =
                    request
                        .captured
                        .capture_status_holding(&memory, context, sources(), None);
                let report = request.restore_movement(movement, Ok(ticks), Ok(status));
                request.restore_speed_terms(terms, Ok(speed_ticks), report)
            };
            let counts = reads.finish();
            assert!(report.is_ok());
            captured.restore_presentation_request(request, report);
            captured.decode_division_presentation_inputs();
            runs.push((counts, captured.division_presentation.tables()));
        }
        let [(full_reads, full), (map_reads, map)] = <[_; 2]>::try_from(runs).ok().unwrap();
        assert!(matches!(full[0], TableView::Complete(_)));
        assert_eq!(map[0], full[0]);
        let movement = TableId::DivisionMovement.index();
        assert_eq!(map_reads[movement], full_reads[movement]);
        assert_ne!(
            full_reads[TableId::DivisionStatus.index()],
            engine_native::read_counts::ReadCounts::ZERO
        );
        for table in STATUS_TABLES {
            assert_eq!(
                map_reads[table.index()],
                engine_native::read_counts::ReadCounts::ZERO,
                "{table:?}"
            );
            let view = &map[TABLES.iter().position(|&id| id == table).unwrap()];
            assert!(
                matches!(view, TableView::Unavailable { .. }),
                "{table:?}: {view:?}"
            );
        }
    }

    /// A rejected or failed presentation pass publishes no presentation
    /// table, though the other passes read every division: the merged report
    /// fails, and restoring the request refuses them all once every loan is
    /// home. A rejected job completes Stopped, as a stopped pool's does.
    #[test]
    fn a_failed_presentation_job_publishes_no_presentation_table() {
        use crate::acquisition::Failure;
        let populated = Populated::new();
        let inputs = std::sync::Arc::clone(populated.census.unit_inputs().unwrap());
        let sources = || Sources {
            rows: inputs.seeds(),
            outcome: inputs.outcome(),
        };
        let memory = populated.world.live();
        let countries = std::sync::Arc::new(
            engine_native::country_memory::Cache::try_with_capacity(0).unwrap(),
        );
        for (status_failure, movement_failure, speed_failure) in [
            (Some(Failure::Stopped), None, None),
            (None, Some(Failure::Panicked), None),
            (None, Some(Failure::Stopped), None),
            (None, None, Some(Failure::Panicked)),
            (None, None, Some(Failure::Stopped)),
            (
                Some(Failure::Stopped),
                Some(Failure::Stopped),
                Some(Failure::Stopped),
            ),
        ] {
            let mut captured =
                super::super::Captured::with_capacities(&super::super::Capacities::default());
            captured.division_presentation = populated.capture();
            captured.install_unit_inputs(Some(std::sync::Arc::clone(&inputs)));
            let context = populated.context();
            let mut request = captured.take_presentation_request(
                &countries,
                context.base,
                context.game_state,
                context.hour,
                None,
            );
            let mut movement = request.take_movement();
            let mut terms = request.take_speed_terms();
            let speed_ticks =
                terms
                    .pass
                    .capture(&memory, context.base, context.game_state, sources());
            let speed_bytes = terms.allocated_bytes();
            let ticks = movement.pass.capture(
                &memory,
                context.base,
                context.game_state,
                sources(),
                Some(context.hour),
            );
            let status = request
                .captured
                .capture_status_holding(&memory, context, sources(), None);
            assert_eq!(
                movement.pass.movement.outcome,
                Outcome::Rows { len: PRESENTED }
            );
            assert_eq!(
                request.captured.status.outcome,
                Outcome::Rows { len: PRESENTED }
            );
            let report = request.restore_movement(
                movement,
                movement_failure.map_or(Ok(ticks), Err),
                status_failure.map_or(Ok(status), Err),
            );
            let report = request.restore_speed_terms(
                terms,
                speed_failure.map_or(Ok(speed_ticks), Err),
                report,
            );
            assert_eq!(request.captured.speed_terms.allocated_bytes(), speed_bytes);
            assert!(report.is_err());
            captured.restore_presentation_request(request, report);
            assert_eq!(captured.acquire_presentation().ticks, [0; N]);
            captured.decode_division_presentation_inputs();
            for (id, view) in TABLES
                .into_iter()
                .zip(captured.division_presentation.tables())
            {
                assert!(
                    matches!(view, TableView::Unavailable { .. }),
                    "{id:?} published after {status_failure:?} {movement_failure:?} {speed_failure:?}"
                );
            }
        }
    }

    /// Every loan returns before failure settlement. Speed failure precedes
    /// movement failure, which precedes status failure; completed passes merge
    /// only their own table ticks.
    #[test]
    fn failed_presentation_passes_keep_the_merged_failure_order() {
        use crate::acquisition::Failure;
        let (seed, bytes) = presentation_fixture();
        let seeds = [seed];
        let memory = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let context = CaptureContext {
            base: 0,
            game_state: 0x1000,
            hour: 0,
        };
        let sources = || Sources {
            rows: &seeds,
            outcome: Outcome::Rows { len: 1 },
        };
        for (status_failure, movement_failure, speed_failure) in [
            (None, None, None),
            (Some(Failure::Stopped), None, None),
            (Some(Failure::Panicked), None, None),
            (None, Some(Failure::Panicked), None),
            (None, Some(Failure::Stopped), None),
            (None, None, Some(Failure::Panicked)),
            (None, None, Some(Failure::Stopped)),
            (Some(Failure::Stopped), Some(Failure::Panicked), None),
            (
                Some(Failure::Panicked),
                Some(Failure::Stopped),
                Some(Failure::Panicked),
            ),
        ] {
            let mut captured = Captured::with_capacities(&Capacities {
                movement: 1,
                status: 1,
                bans: 1,
                geometry: cprovince::GeometryCapacities {
                    provinces: 1,
                    neighbour_entries: 0,
                },
                membership: carmy::StatusCapacities::new(1).membership,
                ..Capacities::default()
            });
            captured.reserve_sources(1);
            let mut request = crate::acquisition_jobs::PresentationRequest {
                dispatch: Default::default(),
                countries: std::sync::Arc::new(
                    engine_native::country_memory::Cache::try_with_capacity(0).unwrap(),
                ),
                context,
                captured,
                inputs: None,
                army_stats: None,
            };
            let mut movement = request.take_movement();
            let mut terms = request.take_speed_terms();
            let speed_ticks =
                terms
                    .pass
                    .capture(&memory, context.base, context.game_state, sources());
            let speed_bytes = terms.allocated_bytes();
            let ticks = movement.pass.capture(
                &memory,
                context.base,
                context.game_state,
                sources(),
                Some(context.hour),
            );
            assert_eq!(movement.pass.movement.outcome, Outcome::Rows { len: 1 });
            let status = request
                .captured
                .capture_status_holding(&memory, context, sources(), None);
            let report = request.restore_movement(
                movement,
                movement_failure.map_or(Ok(ticks), Err),
                status_failure.map_or(Ok(status), Err),
            );
            let report = request.restore_speed_terms(
                terms,
                speed_failure.map_or(Ok(speed_ticks), Err),
                report,
            );
            assert_eq!(request.captured.speed_terms.allocated_bytes(), speed_bytes);
            // All loans are home before the outcome is settled.
            assert_eq!(request.captured.outcomes()[0], Outcome::Rows { len: 1 });
            match speed_failure.or(movement_failure).or(status_failure) {
                None => {
                    let report = report.unwrap();
                    assert_eq!(report.ticks[0], ticks.ticks[0]);
                    assert_eq!(report.ticks[1], status.ticks[1] + ticks.ticks[1]);
                    assert_eq!(report.ticks[2], status.ticks[2]);
                    assert_eq!(report.ticks[3], ticks.ticks[3]);
                    assert_eq!(report.ticks[7..], speed_ticks.ticks[7..]);
                }
                failure => assert_eq!(report.err(), failure),
            }
        }
    }

    #[test]
    fn presentation_report_merges_only_the_movement_owned_tables() {
        let mut status = CaptureReport {
            ticks: [0, 2, 3, 0, 5, 7, 11, 0, 0, 0, 0, 0],
        };
        status.merge_movement(CaptureReport {
            ticks: [13, 17, 0, 19, 0, 0, 0, 23, 29, 31, 37, 41],
        });
        assert_eq!(status.ticks, [13, 19, 3, 19, 5, 7, 11, 0, 0, 0, 0, 0]);
        status.merge_speed_terms(CaptureReport {
            ticks: [43, 47, 53, 59, 61, 67, 71, 23, 29, 31, 37, 41],
        });
        assert_eq!(status.ticks, [13, 19, 3, 19, 5, 7, 11, 23, 29, 31, 37, 41]);
    }

    #[test]
    fn presentation_projects_the_primary_packet_once_after_acquisition() {
        let capacity = Capacities {
            movement: 1,
            status: 0,
            bans: 0,
            paths: 0,
            name_bytes: 0,
            name_groups: 0,
            cached_name_bytes: 0,
            geometry: cprovince::GeometryCapacities {
                provinces: 1,
                neighbour_entries: 0,
            },
            membership: carmy::StatusCapacities::new(0).membership,
            supply_origins: 0,
            combat_references: 0,
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.reserve_sources(1);
        let (seed, mut bytes) = presentation_fixture();
        let seeds = [seed];
        capture.movement.rows[0].division = 999;
        capture.capture(
            &engine_native::memory::SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0,
            0x1000,
            Sources {
                rows: &seeds,
                outcome: Outcome::Rows { len: 1 },
            },
        );
        assert_eq!(capture.outcomes()[0], Outcome::Rows { len: 1 });
        assert_eq!(
            capture.outcomes()[1],
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(
            capture.movement.rows[0].division, 999,
            "acquisition never projects into pooled output"
        );
        assert!(
            capture
                .table_inputs(None, None)
                .iter()
                .all(|input| matches!(input, TableInput::Sampled(TableView::Unavailable { .. })))
        );
        bytes.fill(0xff);
        capture.decode_inputs();
        assert_eq!(capture.movement.rows[0].division, 7);
        assert_eq!(capture.movement.rows[0].numerator, 13);
        assert_eq!(capture.outcomes()[0], Outcome::Rows { len: 1 });
        assert_eq!(
            capture.decode_inputs(),
            [0, 0],
            "projection runs once per capture"
        );
        capture.reset();
        assert_eq!(capture.decode_inputs(), [0; 2]);
    }

    #[test]
    fn cold_and_later_route_projection_grows_owned_destination_before_publication() {
        let mut capture = Captured::with_capacities(&Capacities::default());
        for count in [5_000_usize, 12_000] {
            let (mut seed, mut bytes) = presentation_fixture();
            bytes.resize(0x1000 + count * 4, 0);
            let mut route = [0; 16];
            route[..8].copy_from_slice(&0x2000_u64.to_le_bytes());
            route[8..12].copy_from_slice(&(count as u32).to_le_bytes());
            route[12..].copy_from_slice(&(count as u32).to_le_bytes());
            seed.ordinary_route = Some(route);
            for (index, word) in bytes[0x1000..].chunks_exact_mut(4).enumerate() {
                word.copy_from_slice(&(index as u32 + 1).to_le_bytes());
            }
            capture.capture_owned(
                &engine_native::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                CaptureContext {
                    base: 0,
                    game_state: 0x1000,
                    hour: count as i32,
                },
                Sources {
                    rows: &[seed],
                    outcome: Outcome::Rows { len: 1 },
                },
            );
            assert_eq!(capture.movement.outcome, Outcome::Rows { len: 1 });
            bytes.fill(0xff);
            capture.decode_inputs();
            assert_eq!(capture.movement.outcome, Outcome::Rows { len: 1 });
            assert_eq!(capture.paths.len, count);
            assert_eq!(
                capture.paths.first[..count],
                (1..=count as u32).collect::<Vec<_>>()
            );
            let tables = capture.tables();
            let TableView::Complete(TableRows::DivisionMovement(rows)) = &tables[0] else {
                panic!("the same owned first/later route must be complete");
            };
            assert_eq!(rows[0].path.len(), count);
        }
    }

    #[test]
    fn division_presentation_conversion_keeps_capped_counts_and_scalar_bits() {
        let mut capture = Captured::with_capacities(&Capacities {
            movement: 1,
            status: 1,
            paths: 4,
            ..Default::default()
        });
        capture.paths.first[..4].copy_from_slice(&[7, 3, 7, u32::MAX]);
        capture.paths.len = 4;
        capture.movement.rows[0] = carmy::MovementRow {
            division: u64::MAX,
            numerator: i64::MIN,
            combat_province: u32::MAX,
            path_start: 0,
            path_len: 3,
            path_total: 300,
            strategic_redeployment: 1,
            strategic_action: carmy::StrategicAction::Available(
                carmy::StrategicRedeploymentInputs {
                    country_modifier: i64::MIN,
                    state_modifier: None,
                    friendly_state_modifier: None,
                },
            ),
            alternate_start: 3,
            alternate_len: 1,
            alternate_total: 1,
            withdrawal: Some(0xfe),
            movement_priority: Some(i32::MIN),
            ..Default::default()
        };
        capture.movement.outcome = Outcome::Rows { len: 1 };
        capture.status.outcome = Outcome::Rows { len: 1 };
        capture.status.rows[0] = carmy::StatusRow {
            division: u64::MAX,
            entrenchment: i64::MIN,
            maximum_hit_points: i64::MAX,
            has_order: 0,
            strength_factor: 57_038,
            supply_grace_maximum: i64::MAX,
            supply_grace_remaining: i64::MIN,
            supply: Some(engine_native::cunit_supply_inputs::Inputs {
                asked: 123_456,
                disruption: 4,
                ..Default::default()
            }),
            ..Default::default()
        };
        let [
            TableView::Complete(TableRows::DivisionMovement(m)),
            TableView::Complete(TableRows::DivisionStatus(s)),
            _,
            land,
            ..,
            _,
        ] = capture.tables()
        else {
            panic!("complete companions");
        };
        assert_eq!(
            (m[0].division, m[0].numerator, m[0].combat_province),
            (u64::MAX, i64::MIN, u32::MAX)
        );
        assert_eq!(m[0].withdrawal, Recorded::Value(Some(0xfe)));
        assert_eq!(m[0].movement_priority, Recorded::Value(Some(i32::MIN)));
        assert_eq!(
            (&m[0].path[..], m[0].path_total, &m[0].alternate_path[..]),
            (&[7, 3, 7][..], 300, &[u32::MAX][..])
        );
        assert_eq!(
            (
                s[0].entrenchment,
                s[0].maximum_hit_points,
                s[0].has_order,
                s[0].strength_factor
            ),
            (i64::MIN, i64::MAX, 0, 57_038)
        );
        assert_eq!(s[0].supply_grace_maximum, Recorded::Value(i64::MAX));
        assert_eq!(s[0].supply_grace_remaining, Recorded::Value(i64::MIN));
        // Every division records its supply footer at display precision.
        assert_eq!(
            s[0].supply,
            Recorded::Value(Some(replay_data::rows::DivisionSupplyFooter {
                asked: 123_000,
                disruption: 1_000,
                origins: Some(Vec::new()),
                ..Default::default()
            }))
        );
        assert_eq!(s[0].naming, Recorded::Value(None));
        assert!(matches!(land, TableView::Unavailable { .. }));
        let mut baseline = std::array::from_fn(|_| unavailable("outside presentation fixture"));
        for (id, view) in TABLES.into_iter().zip(capture.tables()) {
            baseline[id.index()] = view;
        }
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        encoder
            .encode(
                replay_frame::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: replay_frame::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        let scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
        // The fixture has no country census, so the bans table is unavailable.
        let [movement, status, bans, _, _, _, panel, ..] =
            capture.table_inputs(Some(&mut projection), None);
        assert!(
            [movement, status, panel]
                .iter()
                .all(|input| matches!(input, TableInput::ComparedEqual(_)))
        );
        assert!(matches!(
            bans,
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        // A footer that drifted inside every printed hundredth reuses the
        // baseline; a printed change publishes the row.
        for (asked, equal) in [(123_499, true), (123_500, false), (123_456, true)] {
            capture.status.rows[0].supply.as_mut().unwrap().asked = asked;
            let [_, status, ..] = capture.table_inputs(Some(&mut projection), None);
            assert_eq!(
                matches!(status, TableInput::ComparedEqual(_)),
                equal,
                "{asked}"
            );
        }
        assert_eq!(
            m[0].strategic_action,
            replay_data::rows::Recorded::Value(carmy::StrategicAction::Available(
                carmy::StrategicRedeploymentInputs {
                    country_modifier: i64::MIN,
                    ..Default::default()
                }
            ))
        );
        capture.movement.rows[0].strategic_action =
            carmy::StrategicAction::Available(carmy::StrategicRedeploymentInputs {
                country_modifier: i64::MAX,
                ..Default::default()
            });
        let [movement, status, ..] = capture.table_inputs(Some(&mut projection), None);
        let TableInput::ComparedChanges { indices, rows, .. } = movement else {
            panic!("current modifier-only change must publish movement");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[0]);
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        capture.movement.rows[0].strategic_action =
            carmy::StrategicAction::Withheld(carmy::StrategicActionFailure::CountryModifier);
        let [movement, status, ..] = capture.table_inputs(Some(&mut projection), None);
        let TableInput::ComparedChanges { indices, rows, .. } = movement else {
            panic!("a refused query must publish the movement row");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[0]);
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        capture.movement.rows[0].strategic_action =
            carmy::StrategicAction::Available(carmy::StrategicRedeploymentInputs {
                country_modifier: i64::MIN,
                ..Default::default()
            });
        // A path-only mutation and an independently changing status must both
        // defeat reuse, without changing their keys or arena spans.
        capture.paths.first[3] ^= 1;
        let [movement, status, ..] = capture.table_inputs(Some(&mut projection), None);
        let TableInput::ComparedChanges { indices, rows, .. } = movement else {
            panic!("changed movement must replace its freshly compared row");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[0]);
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        capture.paths.first[3] ^= 1;
        capture.status.rows[0].has_order = 1;
        let [movement, status, ..] = capture.table_inputs(Some(&mut projection), None);
        assert!(matches!(movement, TableInput::ComparedEqual(_)));
        let TableInput::ComparedChanges { indices, rows, .. } = status else {
            panic!("changed status must replace its freshly compared row");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[1]);
        capture.status.rows[0].has_order = 0;
        capture.movement.rows[0].alternate_start = usize::MAX;
        let [movement, status, ..] = capture.table_inputs(Some(&mut projection), None);
        assert!(matches!(
            movement,
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        assert!(matches!(capture.tables()[1], TableView::Complete(_)));
        // Remaining grace is stored at the whole percent of the live
        // define's share, or at 0.01 hour when the define was unreadable.
        capture.status.rows[0].supply_grace_remaining = 3_659_999;
        for (grace, stored) in [(Some(72), 3_600_000), (None, 3_659_000)] {
            capture.supply_grace = grace;
            let TableView::Complete(TableRows::DivisionStatus(rows)) = &capture.tables()[1] else {
                panic!("complete status");
            };
            assert_eq!(rows[0].supply_grace_remaining, Recorded::Value(stored));
        }
    }

    #[test]
    fn division_presentation_preserves_source_failure_and_primary_arena_growth() {
        let mut capacity = Capacities {
            movement: 1,
            status: 1,
            bans: 1,
            paths: 2,
            membership: MembershipCapacity::default(),
            ..Capacities::default()
        };
        let mut capture = Captured::with_capacities(&capacity);
        let unavailable = Outcome::Unavailable {
            reason: "census failed",
        };
        capture.capture(
            &engine_native::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            0,
            Sources {
                rows: &[],
                outcome: unavailable,
            },
        );
        capture.decode_inputs();
        assert_eq!(capture.outcomes()[..2], [unavailable; 2]);
        assert!(
            capture
                .tables()
                .iter()
                .all(|t| matches!(t, TableView::Unavailable { .. }))
        );
        capture.paths.required = 30;
        let membership = capture.status_packet.membership_capacity();
        capture.grow(&mut capacity);
        assert!(capture.paths.first.len() >= 30);
        assert_eq!(capacity.membership, membership);
        capture.reset();
        assert_eq!(capture.decode_inputs(), [0; 2]);
    }

    #[test]
    fn unavailable_current_transport_replaces_the_previous_suffix_on_stationary_rows() {
        let mut capture = Captured::with_capacities(&Capacities {
            movement: 1,
            status: 0,
            bans: 0,
            paths: 0,
            ..Capacities::default()
        });
        capture.movement.rows[0] = carmy::MovementRow {
            division: u64::MAX,
            source: 7,
            target: 7,
            numerator: i64::MIN,
            ..Default::default()
        };
        capture.movement.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::DivisionMovement(mut rows)) =
            capture.tables()[0].clone()
        else {
            panic!("stationary movement row remains available");
        };
        rows[0].order_transport =
            Recorded::Value(Some(replay_data::rows::DivisionOrderTransportInputs {
                naval_weight: Some(i64::MAX),
                paradrop_eligible: Some(9),
                paradrop_modifiers: Some(replay_data::rows::ParadropTransportModifiers {
                    country_term: i64::MIN,
                    commander_term: None,
                }),
            }));
        let mut baseline = std::array::from_fn(|_| unavailable("outside movement fixture"));
        baseline[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(rows));
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        encoder
            .encode(
                replay_frame::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: replay_frame::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        let scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
        let [movement, ..] = capture.table_inputs(Some(&mut projection), None);
        let TableInput::ComparedChanges { indices, rows, .. } = movement else {
            panic!("an unavailable current suffix cannot reuse the prior row");
        };
        assert_eq!(indices, [0]);
        let TableRows::DivisionMovement(rows) = rows else {
            unreachable!();
        };
        assert_eq!(rows[0].order_transport, Recorded::Value(None));
        // The retired selected transport and selection operands store their
        // tombstones.
        assert_eq!(rows[0].transport, Recorded::Value(Retired));
        assert_eq!(rows[0].selected, Recorded::Value(Retired));
        assert_eq!(
            (
                rows[0].division,
                rows[0].source,
                rows[0].target,
                rows[0].numerator
            ),
            (u64::MAX, 7, 7, i64::MIN)
        );
    }

    /// The base speed is published with its row and joins the unchanged-row
    /// comparison; the retired fresh ETA field is the empty marker.
    #[test]
    fn movement_projection_records_base_speed_and_retires_fresh_eta() {
        let mut capture = Captured::with_capacities(&Capacities {
            movement: 2,
            status: 0,
            bans: 0,
            paths: 0,
            ..Capacities::default()
        });
        capture.movement.rows[0] = carmy::MovementRow {
            division: 7,
            source: 17,
            target: 23,
            base_speed: Some(-4_000_000),
            ..Default::default()
        };
        capture.movement.rows[1] = carmy::MovementRow {
            division: 9,
            source: 17,
            target: 17,
            ..Default::default()
        };
        capture.movement.outcome = Outcome::Rows { len: 2 };
        let [
            TableInput::Sampled(TableView::Complete(TableRows::DivisionMovement(rows))),
            ..,
            _,
        ] = capture.table_inputs(None, Some(12))
        else {
            panic!("movement rows unavailable");
        };
        assert_eq!(rows[0].base_speed, Recorded::Value(Some(-4_000_000)));
        assert_eq!(rows[1].base_speed, Recorded::Value(None));
        assert!(rows.iter().all(|row| row.fresh == Recorded::Value(Retired)));
        let mut baseline = std::array::from_fn(|_| unavailable("outside movement fixture"));
        baseline[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(rows));
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        encoder
            .encode(
                replay_frame::frame::DecodedFrame {
                    branch: 1,
                    hour: 12,
                    tables: baseline.map(Into::into),
                    metrics: replay_frame::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        let scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 13, false, &scratch);
        let [movement, ..] = capture.table_inputs(Some(&mut projection), Some(13));
        assert!(
            matches!(movement, TableInput::ComparedEqual(_)),
            "an equal base speed reuses the unchanged rows"
        );
        capture.movement.rows[0].base_speed = Some(-3_000_000);
        let mut projection = Projection::new(&encoder, 1, 13, false, &scratch);
        let [movement, ..] = capture.table_inputs(Some(&mut projection), Some(13));
        let TableInput::ComparedChanges {
            indices,
            rows: TableRows::DivisionMovement(rows),
            ..
        } = movement
        else {
            panic!("a changed base speed is a changed row");
        };
        assert_eq!(indices, [0]);
        assert_eq!(rows[0].base_speed, Recorded::Value(Some(-3_000_000)));
    }
}

#[cfg(test)]
mod combat_reference_capacity_tests {
    use super::*;

    #[test]
    fn configured_reference_capacity_is_prepared_before_packet_publication() {
        let zero = Captured::with_capacities(&Capacities {
            movement: 1,
            status: 0,
            bans: 0,
            paths: 0,
            combat_references: 0,
            ..Capacities::default()
        });
        assert_eq!(zero.movement_packet.combat_references_capacity(), 0);
        let prepared = Captured::with_capacities(&Capacities {
            movement: 1,
            status: 0,
            bans: 0,
            paths: 0,
            combat_references: 73,
            ..Capacities::default()
        });
        assert!(prepared.movement_packet.combat_references_capacity() >= 73);
        assert!(prepared.allocated_bytes() > zero.allocated_bytes());
        assert_eq!(prepared.movement_packet.combat_references_hour(), None);
    }

    #[test]
    fn optional_reference_allocation_failure_retains_prepared_legacy_packet() {
        // This target exceeds the allocation layout for the reference element.
        // It deterministically fails before allocating, without fault injection.
        // The checked allocator's terminal signal remains authoritative; this
        // fixture proves retained buffer ownership, not continued recording.
        let legacy = carmy::MovementPacket::try_new(1, 2).unwrap();
        let capture = Captured::with_capacities(&Capacities {
            movement: 1,
            status: 0,
            bans: 0,
            paths: 2,
            combat_references: usize::MAX,
            ..Capacities::default()
        });
        assert_eq!(capture.movement_packet.combat_references_capacity(), 0);
        assert_eq!(
            capture.movement_packet.allocated_bytes(),
            legacy.allocated_bytes()
        );
        assert!(
            capture.movement_packet.allocated_bytes() > 0,
            "the optional arena failure must not replace the legacy packet with vacant"
        );
    }
}

#[cfg(all(test, feature = "movement_witness"))]
mod movement_witness_tests {
    use super::*;

    #[test]
    fn movement_witness_projection_rejects_changed_missing_and_refused_samples() {
        let mut capture = Captured::vacant();
        capture.movement_witness.push((7, Some(12_785)));
        let previous = Recorded::Value(Some(replay_data::rows::UnitCombatReferences {
            speed_witness: Witnessed(Some(Some(12_785))),
            ..Default::default()
        }));
        assert!(capture.speed_witness_matches(7, &previous));
        capture.movement_witness[0].1 = Some(12_786);
        assert!(!capture.speed_witness_matches(7, &previous));
        capture.movement_witness[0].1 = None;
        assert_eq!(capture.speed_witness(7), Witnessed(Some(None)));
        assert!(!capture.speed_witness_matches(7, &previous));
        capture.reset();
        assert_eq!(capture.speed_witness(7), Witnessed(None));
        assert!(!capture.speed_witness_matches(7, &previous));
    }
}
