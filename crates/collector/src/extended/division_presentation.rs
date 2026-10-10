//! Hourly division companions. Reuses the existing authenticated source census;
//! movement and counter reads retain separate costs and outcomes. The
//! commanders' retreat bans (DivisionRetreatBans) are the sliced
//! `retreat_bans` family, read daily per roster owner with a constant print.
use super::production::Sources;
use crate::{
    projection::{Projection, project_table, scalar_table},
    sampling::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    spooling::unavailable,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::allocation::{copy_vec, owned_vec};
use engine::{
    Memory, carmy, carmy::DivisionSeed, ccountry, cname_group_member,
    corders_group_member::MembershipCapacity, cprovince, walk::Outcome,
};
use replay::{
    frame::TableInput,
    rows::{
        DivisionAcclimatization, DivisionMovementRow, DivisionRetreatBanRow, DivisionStatusRow,
        NameOperands, Recorded, TableId, tables,
    },
};

pub const N: usize = 5;
/// DivisionRetreatBans' index in [`TABLES`] and the outcomes.
pub const BANS: usize = 2;
pub const TABLES: [TableId; N] = [
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::DivisionRetreatBans,
    TableId::AttritionModifierBreakdowns,
    TableId::DivisionLandInputs,
];
// Both depend on the shared division census; the combined source read group in
// extended::read_groups includes their individually measured costs.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

static SELECTED_RETREAT_REFUSALS: [std::sync::atomic::AtomicU64; 2] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 2];
static WITHDRAWAL_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static MOVEMENT_PRIORITY_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

pub(crate) fn movement_arrow_data() -> impl serde::Serialize {
    crate::status::status!({
        "selected_defending_retreat_refused": SELECTED_RETREAT_REFUSALS[0].load(std::sync::atomic::Ordering::Relaxed),
        "selected_attacking_retreat_refused": SELECTED_RETREAT_REFUSALS[1].load(std::sync::atomic::Ordering::Relaxed),
        "withdrawal_refused": WITHDRAWAL_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "movement_priority_refused": MOVEMENT_PRIORITY_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "scope": "cumulative field-local movement-arrow scalar refusals; established division movement remains available",
    })
}

// One scalar projection for the unchanged-row comparison and the owned row.
macro_rules! status_fields {
    ($row:ident, $apply:ident, [$($args:expr),* $(,)?] $(,)?) => {
        $apply!([$($args),*], {
            division: $row.division,
            maximum_hit_points: $row.maximum_hit_points,
            maximum_organisation: $row.maximum_organisation,
            entrenchment: $row.entrenchment,
            entrenchment_cap: $row.entrenchment_cap,
            supply_ratio: $row.supply_ratio,
            attrition_value: $row.attrition_value,
            attrition_threshold: $row.attrition_threshold,
            attrition: $row.attrition,
            exercising: $row.exercising,
            has_order: $row.has_order,
            invalid_orders: $row.invalid_orders,
            strength_factor: $row.strength_factor,
            model_number: $row.model_number,
            acclimatization_maximum: $row.acclimatization_maximum,
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
    ([$row:expr, $acclimatization:expr, $naming:expr, $supply:expr, $attrition:expr, $dig_in:expr, $name:expr, $attack:expr], {$($field:ident: $value:expr),* $(,)?}) => {
        DivisionStatusRow {
            $($field: $value,)*
            acclimatization: $acclimatization,
            border_combat_stats: Recorded::Value($row.border_combat_stats),
            supply_grace_maximum: Recorded::Value($row.supply_grace_maximum),
            supply_grace_remaining: Recorded::Value($row.supply_grace_remaining),
            naming: Recorded::Value($naming),
            country_ordinal: Recorded::Value($row.country_ordinal),
            invalid_orders_reason: Recorded::Value($row.invalid_orders_reason),
            supply: Recorded::Value($supply),
            attrition_inputs: Recorded::Value($attrition),
            dig_in_factor: Recorded::Value($dig_in),
            name: Recorded::Value($name),
            attack: Recorded::Value($attack),
        }
    };
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
    attrition: super::attrition_inputs::Work,
    transport: super::selected_transport_inputs::Work,
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
            "attrition": self.attrition.status(),
            "transport": self.transport.status(),
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
                engine::allocation::reserve(&mut self.refusals, 1)?;
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
            "scope": "hourly generator operands and exact GetName cache for admitted DivisionStatus rows; each refused field is absent while the status prefix remains available",
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
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
    attrition: super::attrition_inputs::Capacities,
    supply_origins: usize,
    combat_references: usize,
    attack: super::attack_inputs::UnitCapacities,
    transport: engine::selected_transport_units::Capacity,
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
            attrition: super::attrition_inputs::Capacities::default(),
            supply_origins: 0,
            combat_references: 0,
            attack: Default::default(),
            transport: Default::default(),
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

    /// 140C72F00 supplies each army's +340 member; 1409B5640/1409B7290
    /// select the +88 override or resolved template +1D0 group's key.
    /// This off-hour hint supplies storage only, never recorded naming values.
    pub(crate) fn presize_names(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let count =
            crate::country_scale::table_demand::rows(carmy::walk(memory, base, state, &mut []))?;
        let mut rows = engine::allocation::default_vec::<carmy::Row>(count)
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
                .checked_add(engine::carmy_combat_inputs::NAME_GROUP_MEMBER)
                .and_then(|address| engine::read::<u64>(memory, address))
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

    pub(super) fn raise_attack_domain(&mut self, provinces: usize) {
        self.attack.0.terrains = self.attack.0.terrains.max(provinces);
        self.geometry.provinces = self.geometry.provinces.max(provinces);
    }

    pub(crate) fn raise_for_attrition(
        &mut self,
        demand: engine::attrition_modifier_breakdown::Demand,
    ) {
        self.attrition.raise(demand);
        grow_capacity(&mut self.status, demand.rows);
        grow_capacity(&mut self.supply_origins, demand.supply_origins);
        grow_capacity(&mut self.cached_name_bytes, demand.cached_name_text);
        grow_capacity(&mut self.transport.rows, demand.rows);
        grow_capacity(&mut self.transport.members, demand.transport_members);
    }
}
fn status_packet(c: &Capacities) -> carmy::StatusPacket {
    carmy::StatusPacket::with_capacities(carmy::StatusCapacities {
        units: c.status,
        supply_origins: c.supply_origins,
        manpower: 0,
        acclimatization: 0,
        acclimatization_categories: 0,
        thresholds: 0,
        modifiers: engine::modifier_accumulator::SnapshotCapacity {
            nodes: c.status,
            entries: 0,
        },
        membership: c.membership,
    })
    .unwrap_or_else(|_| carmy::StatusPacket::vacant())
}

pub struct Captured {
    land: super::division_land::Captured,
    land_ticks: u64,
    movement: CaptureRows<carmy::MovementRow>,
    status: CaptureRows<carmy::StatusRow>,
    bans: CaptureRows<carmy::RetreatBanRow>,
    /// The roster owners whose DivisionRetreatBans rows this hour replaced.
    bans_selection: SliceSelection,
    paths: Arena<u32>,
    naming: Arena<NamingRead>,
    names: Arena<u8>,
    cached_names: Arena<u8>,
    name_groups: cname_group_member::GroupKeys,
    movement_packet: carmy::MovementPacket,
    /// Sparse execution output from the same held pass, installed only after
    /// both Presentation and Late have joined. Never reads native memory.
    order_transport: engine::order_execution::UnitPacket,
    geometry: cprovince::GeometryPacket,
    geometry_hour: Option<i32>,
    geometry_result: Result<(), &'static str>,
    status_packet: carmy::StatusPacket,
    attrition: super::attrition_inputs::Captured,
    attrition_ticks: u64,
    pub(super) attack: super::attack_inputs::Units,
    attack_capacity: super::attack_inputs::UnitCapacities,
    transport: engine::selected_transport_units::Packet,
    membership: MembershipCapacity,
    source_capacity: usize,
    source_required: usize,
    decode_pending: bool,
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
            bans_selection: SliceSelection::new(false),
            paths: Arena::new(0),
            naming: Arena::new(0),
            names: Arena::new(0),
            cached_names: Arena::new(0),
            name_groups: cname_group_member::GroupKeys::default(),
            movement_packet: carmy::MovementPacket::vacant(),
            order_transport: engine::order_execution::UnitPacket::vacant(),
            geometry: cprovince::GeometryPacket::vacant(),
            geometry_hour: None,
            geometry_result: Err("presentation geometry not attempted"),
            status_packet: carmy::StatusPacket::vacant(),
            attrition: super::attrition_inputs::Captured::new(Default::default()),
            attrition_ticks: 0,
            attack: super::attack_inputs::Units::vacant(),
            attack_capacity: super::attack_inputs::UnitCapacities(
                engine::selected_attack::UnitCapacity::default(),
            ),
            transport: engine::selected_transport_units::Packet::vacant(),
            membership: MembershipCapacity::default(),
            source_capacity: 0,
            source_required: 0,
            decode_pending: false,
        }
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        let mut attrition = super::attrition_inputs::Captured::new(c.attrition);
        attrition.reserve_rows(c.status);
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
            bans_selection: SliceSelection::new(true),
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
            order_transport: engine::order_execution::UnitPacket::vacant(),
            geometry: cprovince::GeometryPacket::try_with_capacities(c.geometry)
                .unwrap_or_default(),
            status_packet: status_packet(c),
            attrition,
            attrition_ticks: 0,
            geometry_hour: None,
            geometry_result: Err("presentation geometry not attempted"),
            attack: super::attack_inputs::Units::new(c.attack),
            attack_capacity: c.attack,
            transport: engine::selected_transport_units::Packet::try_new(c.transport)
                .map_err(crate::allocation::io)
                .unwrap_or_else(|_| engine::selected_transport_units::Packet::vacant()),
            membership: c.membership,
            source_capacity: c.movement.max(c.status),
            source_required: 0,
            decode_pending: false,
        }
    }

    pub fn reset(&mut self) {
        self.land.reset();
        self.land_ticks = 0;
        self.movement.reset();
        self.status.reset();
        self.bans.reset();
        self.paths.reset();
        self.naming.reset();
        self.names.reset();
        self.cached_names.reset();
        self.attrition.reset();
        self.attack.reset();
        self.transport.clear();
        self.geometry_hour = None;
        self.geometry_result = Err("presentation geometry not attempted");
        self.attrition_ticks = 0;
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
            + self.attrition.allocated_bytes()
            + self.attack.allocated_bytes()
            + self.transport.allocated_bytes()
    }

    pub fn grow(&mut self, c: &mut Capacities) {
        self.movement.grow(&mut c.movement);
        self.status.grow(&mut c.status);
        self.bans.grow(&mut c.bans);
        self.attrition.grow(&mut c.attrition);
        self.attack.grow(&mut c.attack);
        self.attack_capacity = c.attack;
        let required = self.transport.required();
        grow_capacity(&mut c.transport.rows, required.rows);
        grow_capacity(&mut c.transport.members, required.members);
        let _ = self
            .transport
            .reserve(c.transport)
            .map_err(crate::allocation::io);
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
        self.attrition.reserve_rows(self.source_capacity);
        self.attack
            .reserve_sources(self.source_capacity, &mut self.attack_capacity);
        let capacity = engine::selected_transport_units::Capacity {
            rows: self.source_capacity,
            members: self.transport.required().members,
        };
        let _ = self
            .transport
            .reserve(capacity)
            .map_err(crate::allocation::io);
    }

    /// The previous sparse output becomes the execution owner's reusable spare.
    /// Its reset makes a failed/cancelled hour incapable of carrying old values.
    pub(crate) fn install_order_transport(
        &mut self,
        execution: &mut engine::order_execution::Prepared,
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

    fn order_transport(&self, division: u64) -> Option<replay::rows::DivisionOrderTransportInputs> {
        let rows = self.order_transport.rows();
        if !matches!(self.order_transport.outcome(), Outcome::Rows { len } if len == rows.len()) {
            return None;
        }
        let inputs = rows
            .binary_search_by_key(&division, |row| row.division)
            .ok()
            .and_then(|index| rows[index].inputs)?;
        Some(replay::rows::DivisionOrderTransportInputs {
            naval_weight: inputs.naval_weight,
            paradrop_eligible: inputs.paradrop_eligible,
            paradrop_modifiers: inputs.paradrop_modifiers.map(|modifiers| {
                replay::rows::ParadropTransportModifiers {
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
        work.transport.observe(&self.transport)?;
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
        work.attrition.observe(&self.attrition)
    }

    /// CArmy+0x340 is the member pointer (140C72F00). RefreshCachedName
    /// 1409B4C40 calls 1409B5640. Keep the language-neutral generator operands
    /// and independently copy the exact +0x60 GetName cache after that pass.
    /// Order/bind writers are 1409BC230/1409BBC40; rename 1413529F0 reaches
    /// 140C7F800/860/9C0, and template/load/stat paths 140C7EE00/140C7D090/
    /// 1401D8120 -> 140C7DEB0 also refresh it. Inputs therefore read hourly.
    fn capture_naming(&mut self, memory: &impl Memory, base: usize, seeds: &[DivisionSeed]) {
        self.naming.reset();
        self.names.reset();
        self.name_groups.reset();
        self.naming.required = seeds.len();
        if !grow_vec(&mut self.naming.first, seeds.len()) {
            return;
        }
        for (seed, row) in seeds.iter().zip(&mut self.naming.first) {
            *row = NamingRead {
                division: seed.division,
                country_ordinal: seed.country_ordinal,
                ..NamingRead::default()
            };
            let result = seed
                .name_member
                .ok_or("division name member pointer unreadable")
                .and_then(|pointer| {
                    if pointer == 0 {
                        Err("division name member pointer null")
                    } else {
                        cname_group_member::inputs_growing(
                            memory,
                            base,
                            pointer,
                            &mut self.name_groups,
                            &mut self.names.first,
                            &mut self.names.len,
                            &mut self.names.required,
                        )
                    }
                });
            match result {
                Ok(cname_group_member::Naming::Row(inputs)) => {
                    self.names.len += inputs.custom.1;
                    // The reader already applies 1409B5640: a programmatic
                    // +0x88 override is shown when the resolved group is null.
                    row.inputs = Some(inputs);
                }
                Ok(cname_group_member::Naming::DoesNotFit) => {
                    row.failure = Some("division naming bytes do not fit arena");
                }
                Err(reason) => row.failure = Some(reason),
            }
            self.naming.len += 1;
        }
        // Every established generator input finishes before the optional
        // GetName cache. Its separate bytes arena keeps refusals independent.
        self.cached_names.reset();
        for (seed, row) in seeds.iter().zip(&mut self.naming.first[..self.naming.len]) {
            let result = seed
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
                });
            match result {
                Ok(Some(span)) => {
                    row.cached_name = Some(span);
                    self.cached_names.len += span.1;
                }
                Ok(None) => {
                    row.cached_failure = Some("division cached name bytes do not fit arena")
                }
                Err(reason) => row.cached_failure = Some(reason),
            }
        }
    }

    /// Acquire on the reserved worker using the immutable source lease.
    /// Membership storage may grow at the original demanded root or chunk.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        store: Option<&mut SliceFingerprints>,
    ) -> CaptureReport {
        let (rows, outcome) = (sources.rows, sources.outcome);
        let ticks = self.capture_at(
            memory,
            context.base,
            context.game_state,
            sources,
            Some(context.hour),
        );
        let bans = self.capture_bans(memory, context, Sources { rows, outcome }, store);
        CaptureReport {
            ticks: [
                ticks[0],
                ticks[1],
                bans,
                self.attrition_ticks,
                self.land_ticks,
            ],
        }
    }

    /// Start a frame's bans selection at its hour; the capture resets it again.
    pub(crate) fn reset_selection(&mut self, hour: i32, force_full: bool) {
        self.bans_selection.reset(hour, force_full);
    }

    /// The roster owners whose DivisionRetreatBans rows this frame replaced.
    pub(crate) fn bans_selection(&self) -> &SliceSelection {
        &self.bans_selection
    }

    #[cfg(test)]
    pub(crate) fn test_set_bans_outcome(&mut self, outcome: Outcome) {
        self.bans.outcome = outcome;
    }

    /// DivisionRetreatBans for this hour's owner slice: the census scanned
    /// into `store` with a constant print (a new owner, or the owner of an
    /// unpublished read, joins the read), then every division of a read
    /// owner. Without a store every owner is read.
    fn capture_bans(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        store: Option<&mut SliceFingerprints>,
    ) -> u64 {
        let _reads = crate::read_accounting::table(TableId::DivisionRetreatBans);
        let start = qpc();
        self.bans_selection
            .reset(context.hour, context.force_full || store.is_none());
        if !grow_vec(&mut self.bans.rows, sources.outcome.len()) {
            self.bans.fail(Outcome::Unavailable {
                reason: "retreat ban row allocation failed",
            });
            return 0;
        }
        let reader = ccountry::Reader::new(memory, context.base, context.game_state);
        if let Some(store) = store {
            let scan = qpc();
            let census = match &reader {
                Err(_) => false,
                Ok(countries) => {
                    let mut complete = true;
                    for slot in 1..countries.count() {
                        match countries.at(slot) {
                            Ok(Some(_)) => self.bans_selection.scan(store, slot, Some(0)),
                            Ok(None) => {}
                            Err(_) => complete = false,
                        }
                    }
                    complete
                }
            };
            store.finish_scan();
            self.bans_selection.finish_pass(store, census);
            self.bans_selection.scan_ticks = qpc().saturating_sub(scan).max(0) as u64;
        }
        self.bans.outcome = match &reader {
            Err(reason) => Outcome::Unavailable { reason },
            Ok(countries) => {
                let selection = &self.bans_selection;
                // An unlinked id has no owner; a link fault is unresolved,
                // which withholds the read's rows (`retreat_bans`).
                let owner = |raw: u32| {
                    Ok(countries
                        .link(raw)?
                        .filter(|&slot| selection.contains(slot)))
                };
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
                )
            }
        };
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
        self.attrition.fail(reason);
        self.attack.fail(reason);
        self.land.fail(reason);
        self.transport.fail(reason);
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
        self.capture_at(memory, base, state, sources, None)
    }

    fn capture_at(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: Sources<'_, DivisionSeed>,
        hour: Option<i32>,
    ) -> [u64; 2] {
        self.reset();
        let outcome = if sources.outcome.len() <= sources.rows.len() {
            sources.outcome
        } else {
            Outcome::Unavailable {
                reason: "division presentation source count invalid",
            }
        };
        let rows = &sources.rows[..outcome.len()];
        // Only the test fixture omits the worker's sample hour and uses fixed storage.
        if hour.is_some() {
            self.movement_packet.enable_growth();
            self.status_packet.enable_growth();
            self.geometry.enable_growth();
            self.transport.enable_growth();
            if !grow_vec(&mut self.movement.rows, rows.len())
                || !grow_vec(&mut self.status.rows, rows.len())
            {
                self.fail_owned_capture("presentation row allocation failed");
                return [0; 2];
            }
        }
        self.source_required = self.source_required.max(rows.len());
        self.land_ticks = self.land.capture(memory, base, rows, outcome);
        let movement_reads = crate::read_accounting::table(TableId::DivisionMovement);
        let start = qpc();
        self.geometry_hour = hour;
        self.geometry_result = Err("presentation geometry acquisition interrupted");
        self.geometry_result = self.geometry.begin(memory, base, state);
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
        for row in self.movement_packet.units_for_selected() {
            for (index, value) in row.selected_retreat.iter().enumerate() {
                if row.combat_count as i32 > 0 && value.is_none() {
                    SELECTED_RETREAT_REFUSALS[index]
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        self.movement.outcome = movement.outcome;
        let (withdrawal, priority) = self.movement_packet.movement_style_refusals();
        WITHDRAWAL_REFUSALS.fetch_add(withdrawal as u64, std::sync::atomic::Ordering::Relaxed);
        MOVEMENT_PRIORITY_REFUSALS.fetch_add(priority as u64, std::sync::atomic::Ordering::Relaxed);
        self.paths.len = movement.paths_len;
        self.paths.required = movement.required_paths;
        let movement_ticks = qpc().saturating_sub(start).max(0) as u64;
        drop(movement_reads);
        let _status_reads = crate::read_accounting::table(TableId::DivisionStatus);
        let start = qpc();
        self.status.outcome =
            self.status_packet
                .capture(memory, base, state, rows, outcome, self.status.rows.len());
        self.capture_naming(memory, base, &rows[..self.status.outcome.len()]);
        if let Some(hour) = hour {
            self.attack
                .capture(memory, base, hour, rows, outcome, &mut self.geometry);
            self.transport
                .capture(memory, base, hour, rows, outcome, &self.attack.packet);
        } else {
            self.attack.fail("selected attack sample hour unavailable");
            self.transport
                .fail("selected transport sample hour unavailable");
        }
        let status_ticks = qpc().saturating_sub(start).max(0) as u64;
        drop(_status_reads);
        let _attrition_reads = crate::read_accounting::table(TableId::AttritionModifierBreakdowns);
        let start = qpc();
        self.attrition.capture(
            memory,
            base,
            state,
            rows,
            self.status.outcome,
            &self.status_packet,
            Some(&self.attack.packet),
        );
        self.attrition_ticks = qpc().saturating_sub(start).max(0) as u64;
        self.decode_pending = true;
        [movement_ticks, status_ticks]
    }

    /// Every restore path stamps the actual request epoch. A stopped or
    /// panicked request never lends stale geometry to independent groups.
    pub(super) fn stamp_attack_epoch(&mut self, hour: i32, failed: bool) {
        self.attack.stamp_hour(hour);
        if failed {
            self.geometry_hour = Some(hour);
            self.geometry_result =
                Err("presentation acquisition failed; current geometry unavailable");
            self.attack.packet.refuse_domain_at(
                hour,
                "presentation acquisition failed; current geometry unavailable",
            );
        }
    }

    pub(super) fn fail_attack_epoch(&mut self, hour: i32, reason: &'static str) {
        self.attack.stamp_hour(hour);
        self.geometry_hour = Some(hour);
        self.geometry_result = Err(reason);
        self.attack.packet.refuse_domain_at(hour, reason);
        self.attack.fail(reason);
    }

    pub(super) fn capture_group_attack(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        groups: &mut super::group_counters::Captured,
    ) {
        self.attack.stamp_hour(hour);
        let current = if self.geometry_hour == Some(hour) {
            self.geometry_result
        } else {
            Err("current presentation geometry not supplied")
        };
        groups.capture_attack(
            memory,
            base,
            hour,
            &mut self.attack,
            &mut self.geometry,
            current,
        );
    }

    /// Called only after original transport/native borrowers have joined.
    pub(super) fn retire_attack_native(&mut self) {
        self.attack.packet.finish_native();
        self.geometry.retire_attack_terrain();
    }

    fn unit_transport(&self, division: u64) -> Option<replay::rows::UnitTransportInputs> {
        if !matches!(self.transport.outcome(), Outcome::Rows { .. }) {
            return None;
        }
        let rows = self.transport.rows();
        rows.binary_search_by_key(&division, |row| row.unit)
            .ok()
            .and_then(|index| rows.get(index))
            .map(super::selected_transport_inputs::project)
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

    pub(crate) fn guard_selected_retreat(&mut self, layout: Option<&crate::fixed_layout::Layout>) {
        let admitted = layout.is_some_and(|layout| {
            layout.as_ref().is_ok_and(|report| {
                let family = replay::fixed_layout::RETREAT_BANS;
                report.failure(family.keys, family.tokens).is_none()
            })
        });
        if admitted {
            return;
        }
        for (index, count) in self
            .movement_packet
            .refuse_selected_retreat()
            .into_iter()
            .enumerate()
        {
            SELECTED_RETREAT_REFUSALS[index].fetch_add(count, std::sync::atomic::Ordering::Relaxed);
        }
        for row in &mut self.movement.rows {
            if row.selected_commander == Some(true) {
                row.selected_retreat = [None; 2];
            }
        }
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.movement.outcome,
            self.status.outcome,
            self.bans.outcome,
            self.attrition.outcome(),
            self.land.outcome(),
        ]
    }

    pub(crate) fn table_inputs(
        &self,
        mut projection: Option<&mut Projection<'_>>,
        eta: Option<(&super::division_eta_inputs::Captured, i32)>,
    ) -> [TableInput; N] {
        if self.decode_pending {
            return std::array::from_fn(|_| {
                unavailable("division presentation projection pending").into()
            });
        }
        let [movement, mut status, bans, _, _] = self.outcomes();
        let breakdowns = self.attrition.table();
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
        let acclimatization = |r: &carmy::StatusRow| self.status_packet.acclimatization(r);
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
        if self
            .status
            .rows
            .get(..status.len())
            .is_none_or(|rows| rows.iter().any(|r| acclimatization(r).is_none()))
        {
            status = Outcome::Unavailable {
                reason: "division acclimatization spans invalid",
            };
        }
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
                let acclimatization = acclimatization(r).unwrap();
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
                    && self.attack.matches(r.division, &previous.attack)
                    && Recorded::Value(r.country_ordinal) == previous.country_ordinal
                    && Recorded::Value(r.invalid_orders_reason) == previous.invalid_orders_reason
                    && super::supply_inputs::matches(
                        r.supply,
                        self.status_packet.supply_origins(r),
                        &previous.supply,
                    )
                    && self.attrition.matches(
                        r.division,
                        self.status_index(r.division),
                        &previous.attrition_inputs,
                    )
                    && Recorded::Value(
                        self.attrition
                            .dig_in_factor(r.division, self.status_index(r.division)),
                    ) == previous.dig_in_factor
                    && match &previous.name {
                        Recorded::Value(name) => cached_name(r) == name.as_deref(),
                        Recorded::NotRecorded => false,
                    }
                    && r.border_combat_stats == *border
                    && r.supply_grace_maximum == *maximum
                    && r.supply_grace_remaining == *remaining
                    && acclimatization.len() == previous.acclimatization.len()
                    && acclimatization
                        .iter()
                        .zip(&previous.acclimatization)
                        .all(|(a, b)| a.category == b.category && a.weight == b.weight)
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
            },
            |r| {
                status_fields!(
                    r,
                    project_status_fields,
                    [
                        r,
                        owned_vec(engine::allocation::collect_vec(
                            acclimatization(r).unwrap_or_default().iter().map(|entry| {
                                DivisionAcclimatization {
                                    category: entry.category,
                                    weight: entry.weight,
                                }
                            }),
                        )),
                        r.naming.map(|n| NameOperands {
                            kind: n.kind,
                            order: n.order,
                            ordered: n.ordered,
                            programmatic: n.programmatic,
                            custom: owned_vec(copy_vec(name_span(n.custom).unwrap())),
                            group: owned_vec(copy_vec(name_span(n.group).unwrap())),
                        }),
                        super::supply_inputs::project(
                            r.supply,
                            self.status_packet.supply_origins(r)
                        ),
                        self.attrition
                            .project(r.division, self.status_index(r.division)),
                        self.attrition
                            .dig_in_factor(r.division, self.status_index(r.division)),
                        cached_name(r).map(|name| owned_vec(copy_vec(name))),
                        self.attack.division(r.division),
                    ],
                )
            },
        );
        let Some(paths) = self.paths.first.get(..self.paths.len) else {
            return [
                unavailable("movement arena invalid").into(),
                status,
                bans,
                breakdowns.into(),
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
                breakdowns.into(),
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
                    selected_unit(self.movement_packet.combat_references_hour(), r)
                        == previous.selected
                        && r.division == previous.division
                        && super::unit_combat_references::matches(
                            self.movement_packet.combat_references_hour(),
                            self.movement_packet.combat_references(r),
                            r.combat_count,
                            &previous.combat_references,
                        )
                        && match eta.and_then(|(eta, hour)| eta.fresh_profile(r.division, hour)) {
                            Some(profile) => profile.matches(&previous.fresh),
                            None => previous.fresh == Recorded::Value(None),
                        }
                        && r.source == previous.source
                        && r.target == previous.target
                        && r.numerator == previous.numerator
                        && r.retreat == previous.retreat
                        && r.strategic_redeployment == previous.strategic_redeployment
                        && replay::rows::Recorded::Value(r.strategic_action)
                            == previous.strategic_action
                        && Recorded::Value(r.withdrawal) == previous.withdrawal
                        && Recorded::Value(r.movement_priority) == previous.movement_priority
                        && Recorded::Value(self.order_transport(r.division))
                            == previous.order_transport
                        && Recorded::Value(self.unit_transport(r.division)) == previous.transport
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
                    selected: selected_unit(self.movement_packet.combat_references_hour(), r),
                    combat_references: Recorded::Value(super::unit_combat_references::project(
                        self.movement_packet.combat_references_hour(),
                        self.movement_packet.combat_references(r),
                        r.combat_count,
                    )),
                    division: r.division,
                    fresh: eta
                        .and_then(|(eta, hour)| eta.fresh_profile(r.division, hour))
                        .map_or(Recorded::Value(None), |profile| profile.project()),
                    source: r.source,
                    target: r.target,
                    numerator: r.numerator,
                    retreat: r.retreat,
                    strategic_redeployment: r.strategic_redeployment,
                    strategic_action: replay::rows::Recorded::Value(r.strategic_action),
                    withdrawal: Recorded::Value(r.withdrawal),
                    movement_priority: Recorded::Value(r.movement_priority),
                    order_transport: Recorded::Value(self.order_transport(r.division)),
                    transport: Recorded::Value(self.unit_transport(r.division)),
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
                },
            ),
            status,
            bans,
            breakdowns.into(),
            self.land.table().into(),
        ]
    }
    fn status_index(&self, division: u64) -> usize {
        self.status_packet
            .status_index(division)
            .unwrap_or(usize::MAX)
    }
    #[cfg(test)]
    pub(super) fn tables(&self) -> [replay::frame::TableView; N] {
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
    use replay::{frame::TableView, rows::TableRows};

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
        use engine::{creference_registry as registry, reference_bucket as bucket};
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
            + engine::creferenced_division_template::REFERENCE_TYPE as usize * 8;
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
            let offset = army - world.base + engine::carmy_combat_inputs::NAME_GROUP_MEMBER;
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
        // leaving the independent GetName member/cache readable.
        registry.0[engine::creference_registry::COUNT..engine::creference_registry::COUNT + 4]
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
                division: engine::read(&memory, army + engine::cunit::HANDLE).unwrap(),
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
                force_full: true,
            },
            sources(),
            None,
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
        let mut encoder = replay::frame::FrameEncoder::new();
        encoder
            .encode(
                replay::frame::DecodedFrame {
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
                force_full: true,
            },
            sources(),
            None,
        );
        captured.decode_inputs();
        let mut scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &mut scratch);
        let [
            _,
            TableInput::ComparedChanges { indices, rows, .. },
            _,
            _,
            _,
        ] = captured.table_inputs(Some(&mut projection), None)
        else {
            panic!("a changed retained cache defeats baseline reuse");
        };
        assert_eq!(indices, [0]);
        let TableRows::DivisionStatus(rows) = rows else {
            panic!("status row changes");
        };
        assert_eq!(rows[0].name, Recorded::Value(Some(vec![b'R', 0, b'C'])));
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
                division: engine::read(&memory, army + engine::cunit::HANDLE).unwrap(),
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
                force_full: true,
            },
            Sources {
                rows: &seeds,
                outcome: Outcome::Rows { len: seeds.len() },
            },
            None,
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
                division: engine::read(&memory, pointer + engine::cunit::HANDLE).unwrap(),
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
                force_full: true,
            },
            Sources {
                rows: &seeds,
                outcome: Outcome::Rows { len: seeds.len() },
            },
            None,
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
        // SetTemplate@1409BBC40 binds the member's own +0x58 reference;
        // ResolveNameGroup@1409B7290 does not consult DivisionRow.template.
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
            &engine::memory::SliceMemory {
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
        // ResolveNameGroup@1409B7290 follows the member's nonnull +0x58
        // reference, then reads the template's +0x1D0 group pointer.
        let template =
            engine::read::<u64>(&world.live(), world.armies[0] + carmy::STANDARD_TEMPLATE).unwrap()
                as usize;
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
                engine::memory::SliceMemory {
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
        let memory = engine::memory::SliceMemory {
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
        };
        captured.status.rows[0] = carmy::StatusRow {
            division: 7,
            naming: Some(naming),
            ..Default::default()
        };
        captured.status.outcome = Outcome::Rows { len: 1 };
        let mut baseline = std::array::from_fn(|_| unavailable("outside naming fixture"));
        baseline[TableId::DivisionStatus.index()] = captured.tables()[1].clone();
        let mut encoder = replay::frame::FrameEncoder::new();
        encoder
            .encode(
                replay::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: replay::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        let mut scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &mut scratch);
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
            let [
                _,
                TableInput::ComparedChanges { indices, rows, .. },
                _,
                _,
                _,
            ] = captured.table_inputs(Some(&mut projection), None)
            else {
                panic!("every naming operand must defeat reuse");
            };
            assert_eq!(indices, [0]);
            assert_eq!(TableView::Complete(rows), captured.tables()[1]);
        }
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
        let mut encoder = replay::frame::FrameEncoder::new();
        encoder
            .encode(
                replay::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: Default::default(),
                },
                false,
            )
            .unwrap();
        let mut scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &mut scratch);
        assert!(matches!(
            captured.table_inputs(Some(&mut projection), None)[1],
            TableInput::ComparedEqual(_)
        ));
        for reason in [None, Some(0), Some(i32::MAX)] {
            captured.status.rows[0].invalid_orders_reason = reason;
            let mut projection = Projection::new(&encoder, 1, 1, false, &mut scratch);
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
            (engine::cgame_state::COUNTRIES, 0x1380_u64, 2_u32),
            (engine::cgame_state::COUNTRY_LINKS, 0x1390_u64, 1_u32),
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
            memory: engine::memory::SliceMemory<'a>,
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
            force_full: true,
        };
        let sources = || Sources {
            rows: &seeds,
            outcome: Outcome::Rows { len: 1 },
        };
        for offset in [
            engine::cgame_state::PROVINCES,
            engine::cgame_state::COUNTRIES,
        ] {
            let mut capture = Captured::with_capacities(&Capacities {
                movement: 1,
                status: 1,
                bans: 1,
                paths: 0,
                name_bytes: 0,
                name_groups: 0,
                cached_name_bytes: 0,
                attrition: Default::default(),
                supply_origins: 0,
                geometry: cprovince::GeometryCapacities {
                    provinces: 1,
                    neighbour_entries: 0,
                },
                membership: carmy::StatusCapacities::new(1).membership,
                combat_references: 0,
                attack: Default::default(),
                transport: Default::default(),
            });
            capture.reserve_sources(1);
            capture.capture_owned(
                &engine::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                context,
                sources(),
                None,
            );
            let memory = PanicAt {
                memory: engine::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                address: 0x1000 + offset,
            };
            assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    capture.capture_owned(&memory, context, sources(), None);
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
            attrition: Default::default(),
            supply_origins: 0,
            geometry: cprovince::GeometryCapacities {
                provinces: 1,
                neighbour_entries: 0,
            },
            membership: carmy::StatusCapacities::new(0).membership,
            combat_references: 0,
            attack: Default::default(),
            transport: Default::default(),
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.reserve_sources(1);
        let (seed, mut bytes) = presentation_fixture();
        let seeds = [seed];
        capture.movement.rows[0].division = 999;
        capture.capture(
            &engine::memory::SliceMemory {
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
                &engine::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                CaptureContext {
                    base: 0,
                    game_state: 0x1000,
                    hour: count as i32,
                    force_full: true,
                },
                Sources {
                    rows: &[seed],
                    outcome: Outcome::Rows { len: 1 },
                },
                None,
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
            ..Default::default()
        };
        let [
            TableView::Complete(TableRows::DivisionMovement(m)),
            TableView::Complete(TableRows::DivisionStatus(s)),
            _,
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
        assert_eq!(s[0].naming, Recorded::Value(None));
        assert!(matches!(land, TableView::Unavailable { .. }));
        let mut baseline = std::array::from_fn(|_| unavailable("outside presentation fixture"));
        for (id, view) in TABLES.into_iter().zip(capture.tables()) {
            baseline[id.index()] = view;
        }
        let mut encoder = replay::frame::FrameEncoder::new();
        encoder
            .encode(
                replay::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: replay::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        let mut scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &mut scratch);
        // The fixture has no country census, so the bans table is unavailable.
        let [movement, status, bans, _, _] = capture.table_inputs(Some(&mut projection), None);
        assert!(
            [movement, status]
                .iter()
                .all(|input| matches!(input, TableInput::ComparedEqual(_)))
        );
        assert!(matches!(
            bans,
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        assert_eq!(
            m[0].strategic_action,
            replay::rows::Recorded::Value(carmy::StrategicAction::Available(
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
        let [movement, status, _, _, _] = capture.table_inputs(Some(&mut projection), None);
        let TableInput::ComparedChanges { indices, rows, .. } = movement else {
            panic!("current modifier-only change must publish movement");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[0]);
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        capture.movement.rows[0].strategic_action =
            carmy::StrategicAction::Withheld(carmy::StrategicActionFailure::CountryModifier);
        let [movement, status, _, _, _] = capture.table_inputs(Some(&mut projection), None);
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
        let [movement, status, _, _, _] = capture.table_inputs(Some(&mut projection), None);
        let TableInput::ComparedChanges { indices, rows, .. } = movement else {
            panic!("changed movement must replace its freshly compared row");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[0]);
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        capture.paths.first[3] ^= 1;
        capture.status.rows[0].has_order = 1;
        let [movement, status, _, _, _] = capture.table_inputs(Some(&mut projection), None);
        assert!(matches!(movement, TableInput::ComparedEqual(_)));
        let TableInput::ComparedChanges { indices, rows, .. } = status else {
            panic!("changed status must replace its freshly compared row");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[1]);
        capture.status.rows[0].has_order = 0;
        capture.movement.rows[0].alternate_start = usize::MAX;
        let [movement, status, _, _, _] = capture.table_inputs(Some(&mut projection), None);
        assert!(matches!(
            movement,
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        assert!(matches!(capture.tables()[1], TableView::Complete(_)));
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
            &engine::memory::SliceMemory {
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
            Recorded::Value(Some(replay::rows::DivisionOrderTransportInputs {
                naval_weight: Some(i64::MAX),
                paradrop_eligible: Some(9),
                paradrop_modifiers: Some(replay::rows::ParadropTransportModifiers {
                    country_term: i64::MIN,
                    commander_term: None,
                }),
            }));
        rows[0].transport = Recorded::Value(Some(replay::rows::UnitTransportInputs {
            sample_hour: 0,
            weight: Some(i64::MIN),
            transfer: Some(replay::rows::UnitTransferInputs::Absent),
            coastal_template: Some(false),
        }));
        let mut baseline = std::array::from_fn(|_| unavailable("outside movement fixture"));
        baseline[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(rows));
        let mut encoder = replay::frame::FrameEncoder::new();
        encoder
            .encode(
                replay::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: replay::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        let mut scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &mut scratch);
        let [movement, _, _, _, _] = capture.table_inputs(Some(&mut projection), None);
        let TableInput::ComparedChanges { indices, rows, .. } = movement else {
            panic!("an unavailable current suffix cannot reuse the prior row");
        };
        assert_eq!(indices, [0]);
        let TableRows::DivisionMovement(rows) = rows else {
            unreachable!();
        };
        assert_eq!(rows[0].order_transport, Recorded::Value(None));
        assert_eq!(rows[0].transport, Recorded::Value(None));
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

    #[test]
    fn movement_fresh_join_changes_only_the_current_profile_and_expires_at_frame_hour() {
        use replay::rows::RowCodec;
        let mut capture = Captured::with_capacities(&Capacities {
            movement: 1,
            status: 0,
            bans: 0,
            paths: 0,
            ..Capacities::default()
        });
        capture.movement.rows[0] = carmy::MovementRow {
            division: 7,
            source: 17,
            target: 17,
            numerator: i64::MIN,
            ..Default::default()
        };
        capture.movement.outcome = Outcome::Rows { len: 1 };
        let old = super::super::division_eta_inputs::fresh_transfer_profile_fixture(11, true);
        let current = super::super::division_eta_inputs::fresh_transfer_profile_fixture(12, true);
        let [
            TableInput::Sampled(TableView::Complete(TableRows::DivisionMovement(previous))),
            _,
            _,
            _,
            _,
        ] = capture.table_inputs(None, Some((&old, 11)))
        else {
            panic!("stationary movement prefix unavailable");
        };
        let mut baseline = std::array::from_fn(|_| unavailable("outside fresh movement fixture"));
        baseline[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(previous.clone()));
        let mut encoder = replay::frame::FrameEncoder::new();
        encoder
            .encode(
                replay::frame::DecodedFrame {
                    branch: 1,
                    hour: 11,
                    tables: baseline.map(Into::into),
                    metrics: replay::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        let mut scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 12, false, &mut scratch);
        let [movement, _, _, _, _] =
            capture.table_inputs(Some(&mut projection), Some((&current, 12)));
        let TableInput::ComparedChanges {
            indices,
            rows: TableRows::DivisionMovement(rows),
            ..
        } = movement
        else {
            panic!("fresh sample hour change was lost in prefix equality");
        };
        assert_eq!(indices, [0]);
        assert_eq!(rows.len(), 1);
        let delta = rows[0].delta(Some(&previous[0])).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 18);
        assert_eq!(
            (
                rows[0].division,
                rows[0].source,
                rows[0].target,
                rows[0].numerator
            ),
            (7, 17, 17, i64::MIN)
        );
        let Recorded::Value(Some(profile)) = &rows[0].fresh else {
            panic!("idle profile missing");
        };
        assert_eq!(profile.sample_hour, 12);
        assert_eq!(profile.current_speed, None);

        let [
            TableInput::Sampled(TableView::Complete(TableRows::DivisionMovement(expired))),
            _,
            _,
            _,
            _,
        ] = capture.table_inputs(None, Some((&current, 13)))
        else {
            panic!("expired profile erased the genuine movement prefix");
        };
        assert_eq!(expired[0].fresh, Recorded::Value(None));
        assert_eq!(
            expired[0],
            DivisionMovementRow {
                fresh: Recorded::Value(None),
                ..rows[0].clone()
            }
        );
        let mut failed = current;
        failed.fail_missing();
        let [
            TableInput::Sampled(TableView::Complete(TableRows::DivisionMovement(refused))),
            _,
            _,
            _,
            _,
        ] = capture.table_inputs(None, Some((&failed, 12)))
        else {
            panic!("failed range erased the genuine movement prefix");
        };
        assert_eq!(refused, expired);
    }

    #[test]
    fn unavailable_profile_cannot_reuse_a_previous_movement_suffix() {
        let mut capture = Captured::with_capacities(&Capacities {
            movement: 1,
            status: 0,
            bans: 0,
            paths: 0,
            ..Capacities::default()
        });
        capture.movement.rows[0] = carmy::MovementRow {
            division: 7,
            source: 17,
            target: 17,
            ..Default::default()
        };
        capture.movement.outcome = Outcome::Rows { len: 1 };
        let current = super::super::division_eta_inputs::fresh_transfer_profile_fixture(12, true);
        let [
            TableInput::Sampled(TableView::Complete(TableRows::DivisionMovement(rows))),
            _,
            _,
            _,
            _,
        ] = capture.table_inputs(None, Some((&current, 12)))
        else {
            panic!("idle profile unavailable");
        };
        let mut baseline = std::array::from_fn(|_| unavailable("outside fresh movement fixture"));
        baseline[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(rows));
        let mut encoder = replay::frame::FrameEncoder::new();
        encoder
            .encode(
                replay::frame::DecodedFrame {
                    branch: 1,
                    hour: 12,
                    tables: baseline.map(Into::into),
                    metrics: replay::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        let mut scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 13, false, &mut scratch);
        let [movement, _, _, _, _] =
            capture.table_inputs(Some(&mut projection), Some((&current, 13)));
        let TableInput::ComparedChanges {
            indices,
            rows: TableRows::DivisionMovement(rows),
            ..
        } = movement
        else {
            panic!("historical profile cannot be compared equal in a later frame");
        };
        assert_eq!(indices, [0]);
        assert_eq!(rows[0].fresh, Recorded::Value(None));
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

    #[test]
    fn selected_unit_projection_keeps_raw_exile_and_independent_key_refusal() {
        let row = carmy::MovementRow {
            selected_exiled: Some(0xfe),
            selected_commander: Some(true),
            selected_retreat: [None, Some(i64::MIN)],
            ..Default::default()
        };
        assert_eq!(
            selected_unit(Some(47), &row),
            Recorded::Value(Some(replay::rows::SelectedUnitInputs {
                sample_hour: 47,
                exiled: Some(0xfe),
                commander: Some(true),
                retreat: [None, Some(i64::MIN)],
            }))
        );
        assert_eq!(selected_unit(None, &row), Recorded::Value(None));
    }

    #[test]
    fn selected_retreat_layout_refusal_preserves_native_null_commander_and_other_operands() {
        let mut captured = Captured::with_capacities(&Capacities {
            movement: 3,
            ..Capacities::default()
        });
        for (index, commander) in [Some(true), Some(false), None].into_iter().enumerate() {
            captured.movement.rows[index] = carmy::MovementRow {
                division: (index + 1) as u64,
                source: 17,
                selected_exiled: Some(0xfe),
                selected_commander: commander,
                selected_retreat: if commander == Some(true) {
                    [Some(i64::MIN), Some(i64::MAX)]
                } else {
                    [Some(0), Some(0)]
                },
                ..Default::default()
            };
        }
        let previous = captured.movement.rows[..3].to_vec();
        captured.guard_selected_retreat(None);
        assert_eq!(
            captured.movement.rows[0],
            carmy::MovementRow {
                selected_retreat: [None, None],
                ..previous[0]
            }
        );
        assert_eq!(captured.movement.rows[1..3], previous[1..3]);
    }
}

fn selected_unit(
    hour: Option<i32>,
    row: &carmy::MovementRow,
) -> Recorded<Option<replay::rows::SelectedUnitInputs>> {
    Recorded::Value(hour.map(|sample_hour| replay::rows::SelectedUnitInputs {
        sample_hour,
        exiled: row.selected_exiled,
        commander: row.selected_commander,
        retreat: row.selected_retreat,
    }))
}
