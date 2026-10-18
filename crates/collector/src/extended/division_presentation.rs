//! Hourly division companions. Reuses the existing authenticated source census;
//! movement and counter reads retain separate costs and outcomes. The
//! commanders' retreat bans (DivisionRetreatBans) are read for every division
//! each hour, carrying each commander's bans while no forwarded writer
//! touched it ([`crate::retreat_ban_changes`]).
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
use replay_data::{
    rows::DivisionCamouflage, rows::DivisionMovementRow, rows::DivisionRetreatBanRow,
    rows::DivisionStatusRow, rows::NameOperands, rows::Recorded, rows::Retired, rows::TableId,
    rows::Witnessed, rows::tables,
};
use replay_frame::frame::TableInput;

pub const N: usize = 4;
/// The passes inside the DivisionStatus acquisition span.
pub(crate) const PHASES: usize = 3;
pub(crate) const PHASE_NAMES: [&str; PHASES] = ["status", "dig_in", "naming"];
pub const TABLES: [TableId; N] = [
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::DivisionRetreatBans,
    TableId::DivisionLandInputs,
];
// Both depend on the shared division census; the combined source read group in
// extended::read_groups includes their individually measured costs.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

static WITHDRAWAL_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static MOVEMENT_PRIORITY_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static BASE_SPEED_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn movement_arrow_data() -> impl serde::Serialize {
    crate::status::status!({
        "withdrawal_refused": WITHDRAWAL_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "movement_priority_refused": MOVEMENT_PRIORITY_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "base_speed_refused": BASE_SPEED_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "scope": "cumulative field-local division movement scalar refusals (arrow style and the base speed of divisions with a route); established division movement remains available",
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
            move_flags: $row.move_flags,
        })
    };
}

macro_rules! compare_status_fields {
    ([$previous:expr], {$first:ident: $initial:expr, $($field:ident: $value:expr),* $(,)?}) => {
        $initial == $previous.$first $(&& $value == $previous.$field)*
    };
}

macro_rules! project_status_fields {
    ([$row:expr, $supply_grace:expr, $naming:expr, $supply:expr, $dig_in:expr, $block:expr, $name:expr, $attack_stats:expr, $witness:expr, $movement:expr], {$($field:ident: $value:expr),* $(,)?}) => {
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
            weather_factor: Recorded::Value($row.weather_factor),
            out_of_supply_days: Recorded::Value($row.out_of_supply_days),
            commander_modifiers: Recorded::Value($block),
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
    };
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

/// DivisionMovement, DivisionLandInputs and DivisionStatus's dig-in factors:
/// the passes' storage, lent to their own job ([`Captured::take_movement`])
/// beside the status pass. They share only the census with it and read the
/// same parked state; the dig-in pass reads every census division and is cut
/// to the status pass's rows when the storage returns.
pub(crate) struct MovementPass {
    land: super::division_land::Captured,
    land_ticks: u64,
    movement: CaptureRows<carmy::MovementRow>,
    paths: Arena<u32>,
    movement_packet: carmy::MovementPacket,
    geometry: cprovince::GeometryPacket,
    dig_in: super::dig_in::Captured,
    dig_in_ticks: u64,
}

impl MovementPass {
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.land.allocated_bytes()
            + self.movement.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.movement_packet.allocated_bytes()
            + self.geometry.allocated_bytes()
            + self.dig_in.allocated_bytes()
    }

    /// The land inputs, the movement inputs and the dig-in factors of every
    /// census division: the DivisionMovement, DivisionLandInputs and dig-in
    /// ticks. Only the test fixture omits the worker's sample hour and uses
    /// fixed storage.
    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: Sources<'_, DivisionSeed>,
        hour: Option<i32>,
    ) -> [u64; 3] {
        self.land.reset();
        self.land_ticks = 0;
        self.movement.reset();
        self.paths.reset();
        self.dig_in.reset();
        self.dig_in_ticks = 0;
        let outcome = source_outcome(&sources);
        let rows = &sources.rows[..outcome.len()];
        if hour.is_some() {
            self.movement_packet.enable_growth();
            self.geometry.enable_growth();
            if !grow_vec(&mut self.movement.rows, rows.len()) {
                let reason = "presentation row allocation failed";
                self.movement.outcome = Outcome::Unavailable { reason };
                self.paths.len = 0;
                self.land.fail(reason);
                return [0; 3];
            }
        }
        self.land_ticks = self.land.capture(memory, base, rows, outcome);
        let _movement_reads = crate::read_accounting::table(TableId::DivisionMovement);
        let start = qpc();
        // A failed geometry root stays the movement rows' own refusal.
        let _ = self.geometry.begin(memory, base, state);
        let movement = carmy::capture_movement_inputs_with_combat_references(
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
        );
        for reason in self.movement_packet.combat_reference_failures() {
            super::unit_combat_references::observe_failure(reason);
        }
        super::unit_combat_references::demand(
            false,
            self.movement_packet.combat_references_required(),
            self.movement_packet.combat_references_capacity(),
        );
        self.movement.outcome = movement.outcome;
        let (withdrawal, priority) = self.movement_packet.movement_style_refusals();
        WITHDRAWAL_REFUSALS.fetch_add(withdrawal as u64, std::sync::atomic::Ordering::Relaxed);
        MOVEMENT_PRIORITY_REFUSALS.fetch_add(priority as u64, std::sync::atomic::Ordering::Relaxed);
        BASE_SPEED_REFUSALS.fetch_add(
            self.movement_packet.base_speed_refusals() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.paths.len = movement.paths_len;
        self.paths.required = movement.required_paths;
        let movement_ticks = qpc().saturating_sub(start).max(0) as u64;
        drop(_movement_reads);
        let _status_reads = crate::read_accounting::table(TableId::DivisionStatus);
        let start = qpc();
        self.dig_in
            .capture(memory, base, rows, Outcome::Rows { len: rows.len() });
        self.dig_in_ticks = qpc().saturating_sub(start).max(0) as u64;
        [movement_ticks, self.land_ticks, self.dig_in_ticks]
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
    /// Sparse execution output from the same held pass, installed only after
    /// both Presentation and Late have joined. Never reads native memory.
    order_transport: engine_native::order_execution::UnitPacket,
    geometry: cprovince::GeometryPacket,
    status_packet: carmy::StatusPacket,
    dig_in: super::dig_in::Captured,
    /// DivisionStatus.attack_stats and the attack witness
    /// ([`super::attack_stats`]), captured after the status pass.
    attack: super::attack_stats::Captured,
    /// The DivisionStatus span split by pass: status and naming, in that
    /// order. Diagnostics only.
    phase_ticks: [u64; PHASES],
    membership: MembershipCapacity,
    source_capacity: usize,
    source_required: usize,
    decode_pending: bool,
    /// The live `NMilitary.SUPPLY_GRACE` hours this pass read, which
    /// DivisionStatus.supply_grace_remaining is stored against
    /// ([`DivisionStatusRow::displayed_grace`]); None when unreadable.
    supply_grace: Option<i32>,
}

impl Captured {
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
            order_transport: engine_native::order_execution::UnitPacket::vacant(),
            geometry: cprovince::GeometryPacket::vacant(),
            status_packet: carmy::StatusPacket::vacant(),
            dig_in: super::dig_in::Captured::new(0),
            attack: Default::default(),
            phase_ticks: [0; PHASES],
            membership: MembershipCapacity::default(),
            source_capacity: 0,
            source_required: 0,
            decode_pending: false,
            supply_grace: None,
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
            order_transport: engine_native::order_execution::UnitPacket::vacant(),
            geometry: cprovince::GeometryPacket::try_with_capacities(c.geometry)
                .unwrap_or_default(),
            status_packet: status_packet(c),
            dig_in: super::dig_in::Captured::new(c.status),
            attack: Default::default(),
            phase_ticks: [0; PHASES],
            membership: c.membership,
            source_capacity: c.movement.max(c.status),
            source_required: 0,
            decode_pending: false,
            supply_grace: None,
        }
    }

    pub fn reset(&mut self) {
        self.land.reset();
        self.land_ticks = 0;
        self.movement.reset();
        self.paths.reset();
        self.dig_in.reset();
        self.reset_status();
    }

    /// Everything but the movement pass's storage, which its own job resets
    /// ([`MovementPass::capture`]).
    fn reset_status(&mut self) {
        self.supply_grace = None;
        self.status.reset();
        self.bans.reset();
        self.naming.reset();
        self.names.reset();
        self.cached_names.reset();
        self.attack.reset();
        self.phase_ticks = [0; PHASES];
        self.order_transport.reset();
        // Engine acquisition resets each owned packet in its measured scope.
        self.decode_pending = false;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.land.allocated_bytes()
            + self.movement.allocated_bytes()
            + self.status.allocated_bytes()
            + self.bans.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.naming.allocated_bytes()
            + self.names.allocated_bytes()
            + self.cached_names.allocated_bytes()
            + self.name_groups.allocated_bytes()
            + self.movement_packet.allocated_bytes()
            + self.order_transport.allocated_bytes()
            + self.geometry.allocated_bytes()
            + self.status_packet.allocated_bytes()
            + self.dig_in.allocated_bytes()
            + self.attack.allocated_bytes()
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
            geometry: std::mem::replace(&mut self.geometry, cprovince::GeometryPacket::vacant()),
            dig_in: std::mem::replace(&mut self.dig_in, super::dig_in::Captured::new(0)),
            dig_in_ticks: 0,
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
            geometry,
            dig_in,
            dig_in_ticks,
        } = pass;
        self.land = land;
        self.land_ticks = land_ticks;
        self.movement = movement;
        self.paths = paths;
        self.movement_packet = movement_packet;
        self.geometry = geometry;
        self.dig_in = dig_in;
        self.dig_in.settle(self.status.outcome);
        self.phase_ticks[1] = dig_in_ticks;
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
            work.supply.observe(row.supply, row.supply_failure);
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
    /// The sampled slice reads its carryable members afresh and compares them
    /// with the held ones; a difference poisons the family.
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
        let check = crate::sampled_verify::current_burned_in(crate::army_stat_changes::VERIFY);
        let mut batch = cname_group_member::MemberBatch::default();
        let mut held_indices = [None; 64];
        let mut carries = [false; 64];
        for (index, (seed, row)) in seeds.iter().zip(&mut self.naming.first).enumerate() {
            if index.is_multiple_of(64) {
                held_indices.fill(None);
                carries.fill(false);
                let mut pointers = [0_usize; 64];
                let end = (index + 64).min(seeds.len());
                for (slot, pending) in seeds[index..end].iter().enumerate() {
                    let Some(pointer) = pending.name_member.filter(|&at| at != 0) else {
                        continue;
                    };
                    // Decide eligibility once, including sampled entries whose
                    // held record is needed later for verification. Retain only
                    // its index; the store keeps the member span and text.
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
                    carries[slot] = held.is_some() && !check.covers(pending.division >> 32);
                    if !carries[slot] {
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
            let carried = held.filter(|_| carries[index % 64]);
            let registered = match (held, pointer, store.is_some()) {
                (Some(_), _, _) => true,
                (None, Ok(pointer), true) => changes.register(pointer),
                _ => false,
            };
            let member = pointer.map(|pointer| match carried {
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
            let cached = match (member, carried, store.as_deref()) {
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
            if let (Some(held), Some(store), Ok((_, Ok(span))), Some(text)) =
                (held, store.as_deref(), member, text)
                && carried.is_none()
            {
                let counts = &mut self.attack.status_counts;
                counts.verified += 1;
                if !cname_group_member::same_recorded_fields(&held.span, &span)
                    || store.held_name_text(held) != text
                {
                    counts.mismatches += 1;
                    changes.mismatch();
                }
            }
            if carried.is_some() {
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

    /// Both sides on this thread, the movement pass lent and returned as
    /// the Presentation and PresentationMovement jobs run them.
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
        let mut report =
            self.capture_status_observed(memory, context, sources, army_stats, changes);
        let [movement, land, dig_in] = pass.capture(
            memory,
            context.base,
            context.game_state,
            Sources { rows, outcome },
            Some(context.hour),
        );
        self.restore_movement(pass);
        report.ticks[0] = movement;
        report.ticks[1] += dig_in;
        report.ticks[3] = land;
        report
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
    /// movement and land ticks are the movement pass's.
    pub(super) fn capture_status_observed(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        army_stats: Option<&mut super::attack_stats::ArmyStatStore>,
        changes: &dyn crate::army_stat_changes::Changes,
    ) -> CaptureReport {
        let (rows, outcome) = (sources.rows, sources.outcome);
        let status = self.capture_status_at(
            memory,
            context.base,
            context.game_state,
            sources,
            Some(context.hour),
            army_stats,
            changes,
        );
        let bans = self.capture_bans(
            memory,
            context,
            Sources { rows, outcome },
            &crate::retreat_ban_changes::Observer,
        );
        CaptureReport {
            ticks: [0, status, bans, 0],
        }
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
        store: Option<&mut super::attack_stats::ArmyStatStore>,
    ) {
        self.capture_attack_observed(
            memory,
            context,
            sources,
            store,
            &crate::army_stat_changes::Observer,
        );
    }

    /// [`Self::capture_attack`] observed through `changes`.
    pub(super) fn capture_attack_observed(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        store: Option<&mut super::attack_stats::ArmyStatStore>,
        changes: &dyn crate::army_stat_changes::Changes,
    ) {
        let _reads = crate::read_accounting::table(TableId::DivisionStatus);
        let status = &self.status_packet;
        self.attack.capture(
            memory,
            context.base,
            context.hour,
            sources.rows,
            sources.outcome,
            store,
            |index| status.unit_maxima(index),
            changes,
        );
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
        let mut leaders = changes_family::Leaders::new(
            &mut store,
            changes,
            crate::sampled_verify::current_burned_in(changes_family::VERIFY),
        );
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
        self.movement.outcome = Outcome::Unavailable { reason };
        self.status.outcome = Outcome::Unavailable { reason };
        self.bans.outcome = Outcome::Unavailable { reason };
        self.paths.len = 0;
        self.naming.len = 0;
        self.names.len = 0;
        self.cached_names.len = 0;
        self.dig_in.reset();
        self.land.fail(reason);
        self.decode_pending = false;
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
        let status = self.capture_status_at(
            memory,
            base,
            state,
            sources,
            None,
            None,
            &crate::army_stat_changes::Observer,
        );
        let [movement, _, dig_in] =
            pass.capture(memory, base, state, Sources { rows, outcome }, None);
        self.restore_movement(pass);
        [movement, status + dig_in]
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
    ) -> u64 {
        self.reset_status();
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
        self.attack
            .certify(rows, outcome, army_stats.as_deref_mut(), changes);
        self.status.outcome = self.status_packet.capture_holding(
            memory,
            base,
            state,
            rows,
            outcome,
            self.status.rows.len(),
            &self.attack.held_maxima,
        );
        let status = &self.status_packet;
        self.attack
            .verify_maxima(|index| status.unit_maxima(index), changes);
        let named = qpc();
        self.capture_naming_holding(
            memory,
            base,
            &rows[..self.status.outcome.len()],
            army_stats,
            changes,
        );
        let finished = qpc();
        // The dig-in phase is the movement pass job's ([`Self::restore_movement`]).
        self.phase_ticks = [
            named.saturating_sub(start).max(0) as u64,
            0,
            finished.saturating_sub(named).max(0) as u64,
        ];
        drop(_status_reads);
        self.decode_pending = true;
        finished.saturating_sub(start).max(0) as u64
    }

    /// This hour's DivisionStatus span by pass, in [`PHASE_NAMES`] order.
    pub(crate) fn phase_ticks(&self) -> [u64; PHASES] {
        self.phase_ticks
    }

    /// Pure decode-worker projection from the current owned packets.
    pub(crate) fn decode_inputs(&mut self) -> [u64; 2] {
        if !std::mem::take(&mut self.decode_pending) {
            return [0; 2];
        }
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
        self.paths.len = movement.paths_len;
        self.paths.required = self.paths.required.max(movement.required_paths);
        let movement_ticks = qpc().saturating_sub(started).max(0) as u64;
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
        [movement_ticks, qpc().saturating_sub(started).max(0) as u64]
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
        [
            self.movement.outcome,
            self.status.outcome,
            self.bans.outcome,
            self.land.outcome(),
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
        let [movement, mut status, bans, _] = self.outcomes();
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
        let cached_name = |r: &carmy::StatusRow| {
            let row = self.naming.first.get(self.status_index(r.division))?;
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
                status_fields!(r, compare_status_fields, [previous])
                    && previous.attack == Recorded::Value(Witnessed(None))
                    && self.attack.witness(r.division, r.weather_factor).is_none()
                    && self.attack.same(r.division, &previous.attack_stats)
                    && self
                        .attack
                        .same_movement(r.division, &previous.movement_stats)
                    && Recorded::Value(r.weather_factor) == previous.weather_factor
                    && Recorded::Value(r.out_of_supply_days) == previous.out_of_supply_days
                    && DivisionStatusRow::exact_supply(
                        r.out_of_supply_days,
                        r.supply_ratio,
                        r.supply_grace_remaining,
                    ) == previous.supply_exact()
                    && matches!(previous.supply_ratio_exact, Recorded::Value(_))
                    && previous.attrition_inputs == Recorded::Value(Retired)
                    && super::supply_inputs::matches_division(
                        r.supply,
                        self.status_packet.supply_origins(r),
                        &previous.supply,
                    )
                    && Recorded::Value(r.country_ordinal) == previous.country_ordinal
                    && Recorded::Value(r.invalid_orders_reason) == previous.invalid_orders_reason
                    && Recorded::Value(
                        self.dig_in
                            .factor(r.division, self.status_index(r.division)),
                    ) == previous.dig_in_factor
                    && Recorded::Value(self.commander_modifiers(r.division))
                        == previous.commander_modifiers
                    && match &previous.name {
                        Recorded::Value(name) => cached_name(r) == name.as_deref(),
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
                        super::supply_inputs::project_division(
                            r.supply,
                            self.status_packet.supply_origins(r),
                        ),
                        self.dig_in
                            .factor(r.division, self.status_index(r.division)),
                        self.commander_modifiers(r.division),
                        cached_name(r).map(|name| owned_vec(copy_vec(name))),
                        self.attack.stats(r.division),
                        self.attack.witness(r.division, r.weather_factor),
                        self.attack.movement(r.division),
                    ],
                )
            },
        );
        let Some(paths) = self.paths.first.get(..self.paths.len) else {
            return [
                unavailable("movement arena invalid").into(),
                status,
                bans,
                self.land.table().into(),
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
                self.land.table().into(),
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
                        && previous.fresh == Recorded::Value(Retired)
                        && Recorded::Value(r.base_speed) == previous.base_speed
                        && Recorded::Value(r.fuel) == previous.fuel
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
                    combat_references: Recorded::Value(super::unit_combat_references::project(
                        request,
                        self.movement_packet.combat_references(r),
                        r.combat_count,
                    )),
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
                },
            ),
            status,
            bans,
            self.land.table().into(),
        ]
    }
    /// The division's selected commander block's movement entries: the block from the
    /// dig-in factor's selection, its entries from the hour's retreat-ban read of the
    /// same commander ([`carmy::StatusPacket::commander_movement`]).
    fn commander_modifiers(&self, division: u64) -> Option<[Option<i64>; 5]> {
        let block = self
            .dig_in
            .commander_block(division, self.status_index(division))?;
        self.status_packet.commander_movement(division, block)
    }

    fn status_index(&self, division: u64) -> usize {
        self.status_packet
            .status_index(division)
            .unwrap_or(usize::MAX)
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

    mod read_ahead;
    mod status_bench;

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
        let [_, TableInput::ComparedChanges { indices, rows, .. }, _, _] =
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
    /// giving the rows a read gives; under `army_stat_verify` every one is
    /// read and compared instead. A member a writer touched is read again.
    #[test]
    fn naming_carries_an_untouched_member_and_rereads_a_touched_one() {
        use crate::army_stat_changes::{VERIFY, Watches};
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
            captured
                .attack
                .certify(&seeds, outcome, Some(&mut *store), &watches);
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
        if VERIFY {
            assert_eq!(read, [true; 3]);
            assert_eq!((counts.verified, counts.mismatches), (3, 0));
        } else {
            assert_eq!(read, [false; 3]);
            assert_eq!(counts.names_reused, 3);
        }
        drop(watches.notify(members[0].as_ptr() as usize, 1));
        let (third, read, counts) = hour(&mut store);
        assert_eq!(third, first);
        assert!(read[0]);
        if VERIFY {
            assert_eq!((counts.verified, counts.mismatches), (2, 0));
        } else {
            assert_eq!(read, [true, false, false]);
            assert_eq!(counts.names_reused, 2);
        }
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
    fn attach_naming_hint_sizes_five_thousand_country_native_strings() {
        let mut world = scale_fixture::build(scale_fixture::Shape {
            countries: 5_000,
            divisions: 5_000,
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
        assert!(capacity.status >= 5_000);
        assert!(capacity.name_bytes >= 5_000 * 15);
        assert!(capacity.cached_name_bytes >= 5_000 * 15);
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
            capture.naming.first[..5_000]
                .iter()
                .all(|row| row.inputs.is_some() && row.cached_failure.is_none())
        );
        assert_eq!(
            (capture.names.len, capture.cached_names.len),
            (5_000 * 15, 5_000 * 15)
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

    #[test]
    fn division_naming_first_capture_grows_and_refusals_preserve_5000_status_rows() {
        const COUNTRIES: usize = 5_000;
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
            let [_, TableInput::ComparedChanges { indices, rows, .. }, _, _] =
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
    /// army_stat store and for an hour that reads every division. Build
    /// without `army_stat_verify`, which reads every carryable division.
    #[test]
    #[ignore = "release benchmark: DivisionStatus with and without the army_stat carry"]
    fn presentation_carry_scale_bench() {
        use super::super::attack_stats::ArmyStatStore;
        use crate::army_stat_changes::{VERIFY, Watches};
        if cfg!(debug_assertions) || VERIFY {
            panic!("run the carry benchmark in release, without army_stat_verify");
        }
        crate::sampled_verify::begin_hour(1);
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
                capture.capture_attack_observed(&memory, context, sources(), store, &watches);
                let attack = begin.elapsed().as_secs_f64() * 1e3;
                let mut frequency = 0;
                // SAFETY: writes the frequency into a local.
                unsafe {
                    windows_sys::Win32::System::Performance::QueryPerformanceFrequency(
                        &mut frequency,
                    )
                };
                let [status, dig_in, naming] = capture
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
            let report = capture.fail_owned_capture("worker failed");
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
            let mut world = scale_fixture::build(Self::SHAPE);
            let registry = template_registry(&mut world);
            let members = division_name_members(&mut world);
            let mut census = crate::division_census::Census::new();
            loop {
                census.clear();
                census.capture(&world.live(), world.base, world.state, PRESENTED);
                if !census.overflowed {
                    break;
                }
                census.grow();
            }
            assert_eq!(
                census.unit_inputs().unwrap().outcome(),
                Outcome::Rows { len: PRESENTED }
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
            let mut capture = Captured::with_capacities(&Capacities {
                status: PRESENTED,
                name_groups: Self::SHAPE.countries,
                ..Capacities::default()
            });
            capture.reserve_sources(PRESENTED);
            capture
        }
    }

    /// The movement pass and the status side read nothing of each other:
    /// run side by side on two threads, each through its own memory handle
    /// as their two jobs run, then the attack pass as the Presentation job
    /// runs it, they publish what the single pass publishes.
    #[test]
    fn presentation_movement_pass_runs_beside_the_status_side() {
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
        let bytes = split.allocated_bytes();
        let mut pass = split.take_movement();
        assert_eq!(split.allocated_bytes() + pass.allocated_bytes(), bytes);
        let movement = std::thread::scope(|scope| {
            let movement = scope.spawn(|| {
                pass.capture(
                    &world.live(),
                    context.base,
                    context.game_state,
                    sources(),
                    Some(context.hour),
                )
            });
            split.capture_status_holding(&memory, context, sources(), None);
            split.capture_attack(&memory, context, sources(), None);
            movement.join().unwrap()
        });
        split.restore_movement(pass);
        split.decode_inputs();
        assert_eq!(split.outcomes(), whole.outcomes());
        assert_eq!(split.tables(), whole.tables());
        assert_eq!(split.naming.len, PRESENTED);
        assert_eq!(movement[1], split.land_ticks);
        assert_eq!(split.phase_ticks()[1], movement[2]);
    }

    /// A rejected or failed job on either side publishes none of the four
    /// tables, though the other side read every division: the merged report
    /// fails, and restoring the request refuses them all once both loans
    /// are home. A rejected job completes Stopped, as a stopped pool's does.
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
        for (status_failure, movement_failure) in [
            (Some(Failure::Stopped), None),
            (None, Some(Failure::Panicked)),
            (None, Some(Failure::Stopped)),
            (Some(Failure::Stopped), Some(Failure::Stopped)),
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
                    "{id:?} published after {status_failure:?} {movement_failure:?}"
                );
            }
        }
    }

    /// A failed status side or a failed movement job refuses the whole
    /// presentation once both loans are home, as the single job's failure
    /// did; with both complete the movement job's ticks join the report.
    #[test]
    fn either_failed_presentation_job_fails_the_merged_report() {
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
        for (status_failure, movement_failure) in [
            (None, None),
            (Some(Failure::Stopped), None),
            (Some(Failure::Panicked), None),
            (None, Some(Failure::Panicked)),
            (None, Some(Failure::Stopped)),
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
                countries: std::sync::Arc::new(
                    engine_native::country_memory::Cache::try_with_capacity(0).unwrap(),
                ),
                context,
                captured,
                inputs: None,
                army_stats: None,
            };
            let mut movement = request.take_movement();
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
            // Both loans are home before the outcome is settled.
            assert_eq!(request.captured.outcomes()[0], Outcome::Rows { len: 1 });
            match status_failure.or(movement_failure) {
                None => {
                    let report = report.unwrap();
                    assert_eq!(report.ticks[0], ticks[0]);
                    assert_eq!(report.ticks[1], status.ticks[1] + ticks[2]);
                    assert_eq!(report.ticks[2], status.ticks[2]);
                    assert_eq!(report.ticks[3], ticks[1]);
                }
                failure => assert_eq!(report.err(), failure),
            }
        }
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
        let [movement, status, bans, _] = capture.table_inputs(Some(&mut projection), None);
        assert!(
            [movement, status]
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
            let [_, status, _, _] = capture.table_inputs(Some(&mut projection), None);
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
        let [movement, status, _, _] = capture.table_inputs(Some(&mut projection), None);
        let TableInput::ComparedChanges { indices, rows, .. } = movement else {
            panic!("current modifier-only change must publish movement");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[0]);
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        capture.movement.rows[0].strategic_action =
            carmy::StrategicAction::Withheld(carmy::StrategicActionFailure::CountryModifier);
        let [movement, status, _, _] = capture.table_inputs(Some(&mut projection), None);
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
        let [movement, status, _, _] = capture.table_inputs(Some(&mut projection), None);
        let TableInput::ComparedChanges { indices, rows, .. } = movement else {
            panic!("changed movement must replace its freshly compared row");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[0]);
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        capture.paths.first[3] ^= 1;
        capture.status.rows[0].has_order = 1;
        let [movement, status, _, _] = capture.table_inputs(Some(&mut projection), None);
        assert!(matches!(movement, TableInput::ComparedEqual(_)));
        let TableInput::ComparedChanges { indices, rows, .. } = status else {
            panic!("changed status must replace its freshly compared row");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[1]);
        capture.status.rows[0].has_order = 0;
        capture.movement.rows[0].alternate_start = usize::MAX;
        let [movement, status, _, _] = capture.table_inputs(Some(&mut projection), None);
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
        let [movement, _, _, _] = capture.table_inputs(Some(&mut projection), None);
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
            _,
            _,
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
        let [movement, _, _, _] = capture.table_inputs(Some(&mut projection), Some(13));
        assert!(
            matches!(movement, TableInput::ComparedEqual(_)),
            "an equal base speed reuses the unchanged rows"
        );
        capture.movement.rows[0].base_speed = Some(-3_000_000);
        let mut projection = Projection::new(&encoder, 1, 13, false, &scratch);
        let [movement, _, _, _] = capture.table_inputs(Some(&mut projection), Some(13));
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
