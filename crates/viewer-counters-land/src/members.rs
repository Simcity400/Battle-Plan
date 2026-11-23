
use crate::land::{CounterTables, DivisionStatus, LandUnit};
use std::collections::BTreeMap;
use std::sync::Arc;
use viewer_counter_kit::{type_tint, unit_flag_owner};
use viewer_foundation::perspective::{SharingUnknown, Sight};
use viewer_foundation::unit_rules::{SHIP_IN_PORT, STANDSTILL};
use viewer_unit_reads::{counter_action, counter_inputs, counter_move, counter_status};
use viewer_unit_reads::counter_inputs::OrderFacts;
use content_reader_core::movement::MovementMap;

pub type LandStacks = viewer_counter_kit::shards::Shards<(u32, u32), Arc<[(Member, counter_action::Action)]>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DivisionFields {
    pub id: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub template: u64,
    pub hit_points: i64,
    pub organisation: i64,
    pub orders_group: u64,
}

impl DivisionFields {
    pub fn of(row: &replay_data::rows::DivisionRow) -> Self {
        Self {
            id: row.id,
            roster_owner: row.roster_owner,
            logical_owner: row.logical_owner,
            expeditionary_owner: row.expeditionary_owner,
            province: row.province,
            template: row.template,
            hit_points: row.hit_points,
            organisation: row.organisation,
            orders_group: row.orders_group,
        }
    }
}

pub trait MemberJoins {
    fn owner(&self, raw: u32) -> Option<u32>;
    fn ordinal(&self, division: u64, province: u32) -> Option<Option<u32>>;
    fn sight(&self, division: u64, owner: u32, province: u32) -> Sight;
    fn type_tint(&self, roster: u32, logical: u32) -> Option<type_tint::Side>;
    fn shared_vision(&self, owner: u32) -> Result<bool, SharingUnknown>;
    fn ours(&self) -> u32;
    fn icon_input(&self, division: u64) -> Option<(u8, u64)>;
    fn hq_provider(&self, division: u64) -> bool;
    fn stack_kind(&self, division: u64) -> u32;
    fn status(&self, division: u64) -> DivisionStatus;
    fn bars(&self, division: u64) -> bool;
    fn orders(&self) -> &dyn OrderFacts;
    fn withdrawing(&self, group: u64) -> Option<bool>;
    fn leader(&self, group: u64) -> Option<u64>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub id: u64,
    pub ordinal: u32,
    pub alternate_template: u64,
    pub separate: bool,
    pub hq: bool,
    pub owner: u32,
    pub logical: Option<u32>,
    pub flag_owner: u32,
    pub template: u64,
    pub orders_group: u64,
    pub sight: Sight,
    pub shared_vision: Result<bool, SharingUnknown>,
    pub type_tint: Option<type_tint::Side>,
    pub hp: i64,
    pub org: i64,
    pub bars: bool,
    pub status_recorded: bool,
    pub status: counter_status::Member,
    pub group_parent: Option<u64>,
    pub parent_branch: Option<bool>,
    pub withdrawing: Option<bool>,
    pub defending_area: Option<bool>,
    pub commander: Option<Option<u64>>,
}

pub fn division_member(
    division: DivisionFields,
    joins: &impl MemberJoins,
) -> Option<((u32, u32), Member)> {
    let slot = joins.owner(division.roster_owner)?;
    let ordinal = match joins.ordinal(division.id, division.province) {
        Some(ordinal) => ordinal?,
        None => 0,
    };
    let sight = joins.sight(division.id, slot, division.province);
    if sight == Sight::None {
        return None;
    }
    let icon_input = joins.icon_input(division.id);
    let alternate_template = icon_input
        .filter(|&(_, alternate)| alternate != 0)
        .map_or(division.template, |(_, alternate)| alternate);
    let hq_provider = joins.hq_provider(division.id);
    let separate = icon_input.map_or(hq_provider, |(separate, _)| separate != 0);
    let kind = joins.stack_kind(division.id);
    if kind == SHIP_IN_PORT {
        return None;
    }
    let strength = member_strength(&division, slot, joins);
    let orders = member_orders(&division, joins);
    Some((
        (division.province, kind),
        Member {
            id: division.id,
            ordinal,
            alternate_template,
            separate,
            hq: separate && hq_provider,
            owner: slot,
            logical: joins.owner(division.logical_owner),
            flag_owner: unit_flag_owner(slot, division.expeditionary_owner, false, &|raw| {
                joins.owner(raw)
            }),
            template: division.template,
            orders_group: division.orders_group,
            sight,
            shared_vision: joins.shared_vision(slot),
            type_tint: joins.type_tint(division.roster_owner, division.logical_owner),
            hp: strength.hp,
            org: strength.org,
            bars: strength.bars,
            status_recorded: strength.status_recorded,
            status: strength.status,
            group_parent: orders.group_parent,
            parent_branch: orders.parent_branch,
            withdrawing: orders.withdrawing,
            defending_area: orders.defending_area,
            commander: orders.commander,
        },
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strength {
    pub hp: i64,
    pub org: i64,
    pub bars: bool,
    pub status_recorded: bool,
    pub status: counter_status::Member,
}

pub fn member_strength(division: &DivisionFields, owner: u32, joins: &impl MemberJoins) -> Strength {
    let status = joins.status(division.id);
    Strength {
        hp: status.maxima.map_or(0, |[maximum, _]| {
            counter_status::modified_strength(division.hit_points, maximum, status.strength_factor)
        }),
        org: status
            .maxima
            .map_or(0, |[_, maximum]| counter_status::ratio(division.organisation, maximum)),
        bars: joins.bars(division.id),
        status_recorded: status.recorded,
        status: counter_status::Member {
            supply_ratio: status.supply_ratio,
            naval_transfer: status.naval_transfer,
            entrenchment: status.entrenchment,
            entrenchment_cap: status.entrenchment_cap,
            attrition: status.attrition,
            exercising: status.exercising,
            grouped: division.orders_group != 0,
            own: owner == joins.ours(),
            has_order: status.has_order,
            invalid_orders: status.invalid_orders,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Orders {
    pub group_parent: Option<u64>,
    pub parent_branch: Option<bool>,
    pub withdrawing: Option<bool>,
    pub defending_area: Option<bool>,
    pub commander: Option<Option<u64>>,
}

pub fn member_orders(division: &DivisionFields, joins: &impl MemberJoins) -> Orders {
    let group = division.orders_group;
    let orders = joins.orders();
    Orders {
        group_parent: orders.group(group).map(|facts| facts.parent),
        parent_branch: counter_inputs::parent_branch(division.id, group, orders),
        withdrawing: joins.withdrawing(group),
        defending_area: counter_inputs::area_defence(division.id, group, orders),
        commander: if group == 0 {
            Some(None)
        } else {
            joins.leader(group).map(|leader| (leader != 0).then_some(leader))
        },
    }
}

impl Member {
    pub fn set_strength(&mut self, strength: Strength) {
        (self.hp, self.org, self.bars, self.status_recorded, self.status) = (
            strength.hp,
            strength.org,
            strength.bars,
            strength.status_recorded,
            strength.status,
        );
    }

    pub fn set_orders(&mut self, orders: Orders) {
        (self.group_parent, self.parent_branch, self.withdrawing, self.defending_area, self.commander) =
            (orders.group_parent, orders.parent_branch, orders.withdrawing, orders.defending_area, orders.commander);
    }
}

pub fn division_kind(
    route: &counter_move::UnitRoute<'_>,
    province: u32,
    map: Option<&MovementMap>,
    anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
) -> u32 {
    map.and_then(|map| route.stack_kind(province, map, anchor))
        .unwrap_or_else(|| {
            counter_move::stack_kind(counter_move::StackKindInputs {
                combat_role: route.combat_role,
                ..Default::default()
            })
        })
}

impl Member {
    pub fn unit(self, action: counter_action::Action) -> LandUnit<'static> {
        LandUnit {
            ordinal: self.ordinal,
            alternate_template: self.alternate_template,
            shared_vision: self.shared_vision,
            separate: self.separate,
            id: self.id,
            owner: self.owner,
            logical: self.logical,
            flag_owner: self.flag_owner,
            type_tint: self.type_tint,
            template: self.template,
            orders_group: self.orders_group,
            hq: self.hq,
            sight: self.sight,
            hp: self.hp,
            org: self.org,
            bars: self.bars,
            status_recorded: self.status_recorded,
            status: self.status,
            movement: None,
            action,
            group_parent: self.group_parent,
            parent_branch: self.parent_branch,
            withdrawing: self.withdrawing,
            defending_area: self.defending_area,
            commander: self.commander,
            gun: None,
        }
    }
}

impl LandUnit<'_> {
    pub fn member(&self) -> Member {
        Member {
            id: self.id,
            ordinal: self.ordinal,
            alternate_template: self.alternate_template,
            separate: self.separate,
            hq: self.hq,
            owner: self.owner,
            logical: self.logical,
            flag_owner: self.flag_owner,
            template: self.template,
            orders_group: self.orders_group,
            sight: self.sight,
            shared_vision: self.shared_vision,
            type_tint: self.type_tint,
            hp: self.hp,
            org: self.org,
            bars: self.bars,
            status_recorded: self.status_recorded,
            status: self.status,
            group_parent: self.group_parent,
            parent_branch: self.parent_branch,
            withdrawing: self.withdrawing,
            defending_area: self.defending_area,
            commander: self.commander,
        }
    }
}

pub struct TableJoins<'a, 't, O> {
    pub land: &'a CounterTables<'t>,
    pub owner: &'a O,
    pub leaders: &'a BTreeMap<u64, u64>,
}

impl<O: Fn(u32) -> Option<u32>> MemberJoins for TableJoins<'_, '_, O> {
    fn owner(&self, raw: u32) -> Option<u32> {
        (self.owner)(raw)
    }

    fn ordinal(&self, division: u64, province: u32) -> Option<Option<u32>> {
        self.land.order.map(|order| order.mixed(division, province))
    }

    fn sight(&self, division: u64, owner: u32, province: u32) -> Sight {
        self.land.vision.sees_land(division, owner, province)
    }

    fn shared_vision(&self, owner: u32) -> Result<bool, SharingUnknown> {
        self.land.vision.shares_vision_proven(owner)
    }

    fn type_tint(&self, roster: u32, logical: u32) -> Option<type_tint::Side> {
        self.land.type_tints.of(roster, logical)
    }

    fn ours(&self) -> u32 {
        self.land.vision.ours()
    }

    fn icon_input(&self, division: u64) -> Option<(u8, u64)> {
        self.land.icon_inputs.get(division)
    }

    fn hq_provider(&self, division: u64) -> bool {
        self.land.hq_providers.contains(&division)
    }

    fn stack_kind(&self, division: u64) -> u32 {
        self.land.movement.get(division).map_or(STANDSTILL, |movement| movement.stack_kind)
    }

    fn status(&self, division: u64) -> DivisionStatus {
        self.land.status.get(division)
    }

    fn bars(&self, division: u64) -> bool {
        self.land.status.recorded(division)
    }

    fn orders(&self) -> &dyn OrderFacts {
        self.land.orders
    }

    fn withdrawing(&self, group: u64) -> Option<bool> {
        self.land.native.withdrawing(group)
    }

    fn leader(&self, group: u64) -> Option<u64> {
        self.leaders.get(&group).copied()
    }
}
