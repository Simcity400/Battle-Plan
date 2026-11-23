
use content_reader_core::map::MapData;
use crate::naval;
use crate::naval::{NavalContext, NavalLinks};
use std::collections::{BTreeMap, BTreeSet};
use viewer_counter_kit::{CounterMap, type_tint, unit_flag_owner};
use viewer_foundation::mode_visibility::UnitKinds;
use viewer_foundation::movement_timing::MovementContent;
use viewer_foundation::perspective::{Sight, Vision};
use viewer_foundation::unit_rules::SHIP_IN_PORT;
use viewer_unit_reads::counter_move;
#[cfg(test)]
use viewer_unit_reads::counter_order;
use viewer_unit_reads::task_force_visual::Placement;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForceFields {
    pub id: u64,
    pub fleet: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub mission: u32,
    pub combat_references: i32,
}

impl ForceFields {
    pub fn of(row: &replay_data::rows::TaskForceRow) -> Self {
        Self {
            id: row.id,
            fleet: row.fleet,
            roster_owner: row.roster_owner,
            logical_owner: row.logical_owner,
            expeditionary_owner: row.expeditionary_owner,
            province: row.province,
            mission: row.mission,
            combat_references: row.combat_references,
        }
    }
}

#[derive(Clone, Copy)]
pub struct SeaGround<'a> {
    pub content: &'a CounterMap,
    pub map: &'a MapData,
    pub movement: Option<&'a MovementContent>,
    pub world: Option<counter_move::RouteWorld<'a>>,
}

impl SeaGround<'_> {
    pub fn land(&self, province: u32) -> bool {
        viewer_counter_kit::province_is_land(self.map, self.movement, province)
    }

    pub fn place(&self, province: u32, route: &counter_move::UnitRoute<'_>) -> Result<Placement, &'static str> {
        viewer_unit_reads::task_force_visual::visual_of(
            province,
            route,
            self.movement,
            self.world.as_ref(),
            &|id, slot| self.content.anchor(id, slot),
            &|id| self.land(id),
            &|id, kind| self.content.stack_scale(id, kind),
        )
    }
}

pub trait NavalJoins {
    fn owner(&self, raw: u32) -> Option<u32>;
    fn mixed(&self, force: u64, province: u32) -> Option<Option<u32>>;
    fn naval(&self, force: u64, province: u32) -> Option<Option<u32>>;
    fn land(&self, province: u32) -> bool;
    fn region(&self, province: u32) -> Option<u32>;
    fn placement(&self, force: &ForceFields) -> Result<Placement, &'static str>;
    fn skip(&self, force: &ForceFields, kind: u32, region: Option<u32>) -> Option<bool>;
    fn carries(&self, force: u64) -> Option<bool>;
    fn ships(&self, force: u64) -> Option<(usize, i64)>;
    fn ships_complete(&self) -> bool;
    fn ratio(&self, force: u64) -> Option<(i64, i64)>;
    fn vision(&self) -> &Vision;
    fn type_tint(&self, roster: u32, logical: u32) -> Option<type_tint::Side>;
    fn docked(&self, force: u64) -> Result<bool, &'static str>;
    fn base(&self, province: u32) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavalOperands {
    pub slot: u32,
    pub logical: Option<u32>,
    pub flag_owner: u32,
    pub sight: Sight,
    pub byte: u16,
    pub ships: Option<usize>,
    pub strength: i64,
}

impl NavalOperands {
    pub fn of(force: &ForceFields, ships: Option<(usize, i64)>, ships_complete: bool, vision: &Vision, owner: &impl Fn(u32) -> Option<u32>) -> Option<Self> {
        let slot = owner(force.roster_owner)?;
        let byte = vision.navy_byte(force.id, slot);
        let sight = vision.sees_navy(force.id, slot);
        let (count, strength) = ships.unwrap_or_default();
        (sight != Sight::None).then(|| Self {
            slot,
            logical: owner(force.logical_owner),
            flag_owner: unit_flag_owner(slot, force.expeditionary_owner, false, owner),
            sight,
            byte,
            ships: ships_complete.then_some(count),
            strength,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sea {
    Source { kind: u32, ordinal: u32, moving: bool },
    Out(&'static str),
    Refused(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Port {
    In(u32),
    Out,
    Undecided(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalMember {
    pub fields: ForceFields,
    pub sea: Sea,
    pub unplaced: Option<&'static str>,
    pub port: Port,
    pub unit: Option<NavalOperands>,
    pub type_tint: Option<type_tint::Side>,
    pub ratio: Option<(i64, i64)>,
}

pub const NOT_WALKED: &str = "not in its province's mixed occupant array";
pub const DOCKED: &str = "docked: the naval base shows it";
pub const MODE: &str = "the map mode keeps no such task force";
pub const IN_COMBAT: &str = "in combat";
pub const ON_MISSION: &str = "on its mission here: the mission icon shows it";
pub const UNSEEN: &str = "unseen, or its roster owner is not a linked country";
pub const SKIP_UNRECORDED: &str = "its mission skip's counter inputs or current region are not recorded";

pub fn sea_place(force: &ForceFields, kinds: UnitKinds, joins: &impl NavalJoins) -> (Sea, Option<Placement>) {
    let ordinal = match joins.mixed(force.id, force.province) {
        Some(Some(ordinal)) => ordinal,
        Some(None) => return (Sea::Out(NOT_WALKED), None),
        None => 0,
    };
    let placement = match joins.placement(force) {
        Ok(placement) => placement,
        Err(reason) => return (Sea::Refused(reason), None),
    };
    let kind = placement.kind;
    let sea = if kind == SHIP_IN_PORT {
        Sea::Out(DOCKED)
    } else {
        match naval::mode_keeps_task_force(kinds, || joins.carries(force.id)) {
            None => Sea::Refused(naval::CARRIER_TEST_UNRECORDED),
            Some(false) => Sea::Out(MODE),
            Some(true) if force.combat_references > 0 => Sea::Out(IN_COMBAT),
            Some(true) => match joins.skip(force, kind, joins.region(force.province)) {
                None => Sea::Refused(SKIP_UNRECORDED),
                Some(true) => Sea::Out(ON_MISSION),
                Some(false) => Sea::Source { kind, ordinal, moving: placement.position.motion.is_some() },
            },
        }
    };
    (sea, Some(placement))
}

pub fn port_place(force: &ForceFields, joins: &impl NavalJoins) -> Port {
    if force.province == 0 || force.fleet == 0 || !joins.land(force.province) || !joins.base(force.province) {
        return Port::Out;
    }
    let Some(naval) = joins.naval(force.id, force.province) else { return Port::Out };
    match joins.ships(force.id) {
        Some((count, _)) if count > 0 => {}
        Some(_) => return Port::Out,
        None if joins.ships_complete() => return Port::Out,
        None => return Port::Undecided(naval::PORT_SHIPS_UNRECORDED),
    }
    let Some(ordinal) = naval else { return Port::Out };
    match joins.docked(force.id) {
        Ok(true) => {}
        Ok(false) => return Port::Out,
        Err(reason) => return Port::Undecided(reason),
    }
    let owner = |raw| joins.owner(raw);
    if owner(force.roster_owner).is_none() {
        return Port::Undecided(naval::PORT_OWNER_UNLINKED);
    }
    match NavalOperands::of(force, joins.ships(force.id), joins.ships_complete(), joins.vision(), &owner) {
        Some(_) => Port::In(ordinal),
        None => Port::Out,
    }
}

pub fn naval_member(force: ForceFields, kinds: UnitKinds, joins: &impl NavalJoins) -> NavalMember {
    let (sea, placement) = sea_place(&force, kinds, joins);
    let unit = NavalOperands::of(&force, joins.ships(force.id), joins.ships_complete(), joins.vision(), &|raw| joins.owner(raw));
    let sea = match sea {
        Sea::Source { .. } if unit.is_none() => Sea::Out(UNSEEN),
        sea => sea,
    };
    NavalMember {
        fields: force,
        sea,
        unplaced: placement.and_then(|placement| placement.unplaced_route),
        port: port_place(&force, joins),
        unit,
        type_tint: joins.type_tint(force.roster_owner, force.logical_owner),
        ratio: joins.ratio(force.id),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FleetMarker {
    pub reserve: Option<bool>,
    pub colour: [u32; 4],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortSummary {
    pub anchors: Vec<(u32, [u32; 3])>,
    pub unanchored: usize,
    pub undecided: BTreeMap<u32, String>,
}

pub fn port_undecided_reason(reason: &str) -> String {
    naval::port_undecided(reason).into_owned()
}

pub fn port_force_unrecorded() -> String {
    port_undecided_reason(naval::PORT_FORCE_UNRECORDED)
}

pub struct HourJoins<'a, 't, O> {
    pub land: &'a NavalLinks<'t>,
    pub context: &'a NavalContext<'t>,
    pub owner: &'a O,
    pub ground: SeaGround<'a>,
    pub row: &'t replay_data::rows::TaskForceRow,
    pub bases: &'a BTreeSet<u32>,
}

impl<O: Fn(u32) -> Option<u32>> NavalJoins for HourJoins<'_, '_, O> {
    fn owner(&self, raw: u32) -> Option<u32> {
        (self.owner)(raw)
    }

    fn mixed(&self, force: u64, province: u32) -> Option<Option<u32>> {
        self.land.order.map(|order| order.mixed(force, province))
    }

    fn naval(&self, force: u64, province: u32) -> Option<Option<u32>> {
        self.land.order.map(|order| order.naval(force, province))
    }

    fn land(&self, province: u32) -> bool {
        self.ground.land(province)
    }

    fn region(&self, province: u32) -> Option<u32> {
        self.ground.content.region(province)
    }

    fn placement(&self, force: &ForceFields) -> Result<Placement, &'static str> {
        debug_assert_eq!(force.id, self.row.id);
        let ground = self.ground;
        self.land.task_forces.visual(self.row, ground.movement, ground.world.as_ref(), &|id, slot| ground.content.anchor(id, slot), &|id| ground.land(id), &|id, kind| ground.content.stack_scale(id, kind))
    }

    fn skip(&self, force: &ForceFields, kind: u32, region: Option<u32>) -> Option<bool> {
        debug_assert_eq!(force.id, self.row.id);
        self.context.mission_skips.skip(self.row, kind, region)
    }

    fn carries(&self, force: u64) -> Option<bool> {
        self.context.ship_inputs.as_ref().and_then(|ships| ships.carries_air_wings(force))
    }

    fn ships(&self, force: u64) -> Option<(usize, i64)> {
        self.context.ships.get(&force).copied()
    }

    fn ships_complete(&self) -> bool {
        self.context.ships_complete
    }

    fn ratio(&self, force: u64) -> Option<(i64, i64)> {
        self.context.ship_inputs.as_ref().map(|inputs| inputs.ratio(force))
    }

    fn vision(&self) -> &Vision {
        self.context.vision
    }

    fn type_tint(&self, roster: u32, logical: u32) -> Option<type_tint::Side> {
        self.context.type_tints.of(roster, logical)
    }

    fn docked(&self, force: u64) -> Result<bool, &'static str> {
        self.land.task_forces.docked(force)
    }

    fn base(&self, province: u32) -> bool {
        self.bases.contains(&province)
    }
}

pub struct TodayNaval {
    pub members: Vec<NavalMember>,
    pub fleets: BTreeMap<u64, FleetMarker>,
    pub ports: PortSummary,
    pub peace_conference: bool,
    pub forces_complete: bool,
}

#[cfg(test)]
pub struct Fake<'a> {
    pub row: &'a replay_data::rows::TaskForceRow,
    pub kind: u32,
    pub region: Option<u32>,
    pub skips: viewer_unit_reads::task_force_visual::MissionSkips,
    pub order: Option<counter_order::RecordedOrder>,
    pub ships: BTreeMap<u64, (usize, i64)>,
    pub ships_complete: bool,
    pub docked: Result<bool, &'static str>,
    pub linked: bool,
    pub vision: Vision,
}

#[cfg(test)]
impl<'a> Fake<'a> {
    pub fn new(row: &'a replay_data::rows::TaskForceRow) -> Self {
        Self {
            row,
            kind: 0,
            region: None,
            skips: Default::default(),
            order: None,
            ships: BTreeMap::new(),
            ships_complete: true,
            docked: Ok(true),
            linked: true,
            vision: Vision::default(),
        }
    }

    pub fn sea(&self) -> Sea {
        sea_place(&ForceFields::of(self.row), UnitKinds::All, self).0
    }

    pub fn port(&self) -> Port {
        port_place(&ForceFields::of(self.row), self)
    }
}

#[cfg(test)]
impl NavalJoins for Fake<'_> {
    fn owner(&self, raw: u32) -> Option<u32> {
        self.linked.then_some(raw)
    }

    fn mixed(&self, force: u64, province: u32) -> Option<Option<u32>> {
        self.order.as_ref().map(|order| order.mixed(force, province))
    }

    fn naval(&self, force: u64, province: u32) -> Option<Option<u32>> {
        self.order.as_ref().map(|order| order.naval(force, province))
    }

    fn land(&self, _province: u32) -> bool {
        true
    }

    fn region(&self, _province: u32) -> Option<u32> {
        self.region
    }

    fn placement(&self, _force: &ForceFields) -> Result<Placement, &'static str> {
        Ok(Placement { kind: self.kind, position: counter_move::Position::still([0.0; 3]), animator: None, unplaced_route: None })
    }

    fn skip(&self, _force: &ForceFields, kind: u32, region: Option<u32>) -> Option<bool> {
        self.skips.skip(self.row, kind, region)
    }

    fn carries(&self, _force: u64) -> Option<bool> {
        None
    }

    fn ships(&self, force: u64) -> Option<(usize, i64)> {
        self.ships.get(&force).copied()
    }

    fn ships_complete(&self) -> bool {
        self.ships_complete
    }

    fn ratio(&self, _force: u64) -> Option<(i64, i64)> {
        None
    }

    fn vision(&self) -> &Vision {
        &self.vision
    }

    fn type_tint(&self, _roster: u32, _logical: u32) -> Option<type_tint::Side> {
        None
    }

    fn docked(&self, _force: u64) -> Result<bool, &'static str> {
        self.docked
    }

    fn base(&self, _province: u32) -> bool {
        true
    }
}
