
use viewer_foundation::unit_rules::mission_active;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use replay_data::{
    rows::ArmyOrderPlanRow, rows::FleetCounterInputRow, rows::OrdersGroupRow,
    rows::TaskForceCounterInputRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;

use viewer_foundation::tables::rows;

pub const SCALE: i64 = 100_000;
pub const UNDEFINED: i64 = 0xffff_ffff;
pub const MAXIMUM_FLOOR: i64 = 100;
pub const RESERVE_FLEET: u32 = 8;
fn defined(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

pub fn strength_ratio(ships: impl IntoIterator<Item = (i64, i64)>) -> i64 {
    let (mut sum, mut maximum, mut any) = (0i64, 0u64, false);
    for (strength, raw_maximum) in ships {
        any = true;
        sum = sum.wrapping_add(strength);
        maximum = maximum.wrapping_add(raw_maximum.max(MAXIMUM_FLOOR) as u64);
    }
    if !any || (maximum as i64) <= 0 {
        return sum;
    }
    if defined(sum) {
        sum.wrapping_mul(SCALE) / maximum as i64
    } else {
        UNDEFINED
    }
}

pub fn ship_organisation_ratio(organisation: i64, maximum: i64) -> i64 {
    if maximum != 0 && defined(organisation) {
        organisation.wrapping_mul(SCALE) / maximum
    } else {
        UNDEFINED
    }
}

pub fn organisation_ratio(ships: impl IntoIterator<Item = (i64, i64)>) -> i64 {
    let (mut sum, mut count) = (0u64, 0i64);
    for (organisation, maximum) in ships {
        sum = sum.wrapping_add(ship_organisation_ratio(organisation, maximum) as u64);
        count += 1;
    }
    if count == 0 {
        return 0;
    }
    let sum = sum as i64;
    if !defined(sum) {
        return UNDEFINED;
    }
    sum.wrapping_mul(SCALE) / (count * SCALE)
}

pub fn member_mean(values: impl IntoIterator<Item = i64>) -> Option<i64> {
    let (mut sum, mut count) = (0i64, 0i64);
    for value in values {
        sum = sum.wrapping_add(value);
        count += 1;
    }
    if count == 0 {
        return None;
    }
    Some(if defined(sum) {
        sum.wrapping_mul(SCALE) / (count * SCALE)
    } else {
        UNDEFINED
    })
}

pub fn bar_fill(mean: i64) -> f32 {
    mean as f32 / SCALE as f32
}

pub fn bar_step(fill: f32, length: f32) -> i64 {
    let t = length * fill * 0.5f32;
    let positive = if 0.0 <= t { 1.0f32 } else { 0.0 };
    ((positive + t * SCALE as f32) - 0.5f32) as i64 / SCALE
}

pub fn bar_pixels(fill: f32, length: f32) -> f32 {
    (2 * bar_step(fill, length) + 2).clamp(0, length as i64 + 1) as f32
}

pub fn needs_regions(mission: u32) -> bool {
    matches!(mission, 1..=6 | 9)
}

pub fn effective_mission(mission: u32, detached_activity: i32, detached_mission: i32) -> u32 {
    if detached_activity == 0 {
        mission
    } else {
        detached_mission as u32
    }
}

pub fn engagement_refused(
    required_flag: u8,
    engagement_rule: i32,
    mission: u32,
    aggression: &[i64],
) -> Option<bool> {
    let refused = if required_flag != 0 {
        true
    } else {
        *aggression.get(usize::try_from(engagement_rule).ok()?)? < 1
    };
    Some(refused && matches!(mission, 2 | 3))
}

#[derive(Clone, Copy, Debug)]
pub struct NavalMember<'a> {
    pub visible: bool,
    pub own: bool,
    pub mission: u32,
    pub has_fleet: bool,
    pub inputs: Option<&'a TaskForceCounterInputRow>,
    pub fleet_regions: Option<i32>,
    pub pride: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavalStatus {
    pub no_orders: Option<bool>,
    pub invalid_orders: Option<bool>,
    pub pride: Option<bool>,
    pub active: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavalSupply {
    Retain,
    Write(i64),
    Unavailable,
}

pub fn port_supply(members: &[NavalMember<'_>]) -> NavalSupply {
    let Some(first) = members.first() else {
        return NavalSupply::Unavailable;
    };
    if !first.visible {
        return NavalSupply::Retain;
    }
    members
        .iter()
        .try_fold(SCALE, |lowest, member| {
            member
                .inputs
                .map(|row| lowest.min(row.received_supply_ratio))
        })
        .map_or(NavalSupply::Unavailable, NavalSupply::Write)
}

pub fn sea_supply(members: &[NavalMember<'_>]) -> NavalSupply {
    members
        .iter()
        .filter(|member| member.visible)
        .try_fold(SCALE, |_, member| {
            member.inputs.map(|row| row.received_supply_ratio)
        })
        .map_or(NavalSupply::Unavailable, NavalSupply::Write)
}

fn any(terms: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    let mut unknown = false;
    for term in terms {
        match term {
            Some(true) => return Some(true),
            None => unknown = true,
            Some(false) => {}
        }
    }
    (!unknown).then_some(false)
}

fn all(terms: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    any(terms.into_iter().map(|term| term.map(|value| !value))).map(|value| !value)
}

fn order_terms(member: &NavalMember<'_>, aggression: &[i64]) -> (Option<bool>, Option<bool>) {
    let Some(inputs) = member.inputs else {
        return (None, None);
    };
    let refused = engagement_refused(
        inputs.running_away,
        inputs.engagement_rule,
        member.mission,
        aggression,
    );
    let hold = if member.has_fleet {
        member.fleet_regions.map(|regions| {
            regions != 0
                && effective_mission(
                    member.mission,
                    inputs.detached_activity,
                    inputs.repair_last_mission,
                ) == 0
        })
    } else {
        Some(false)
    };
    let invalid = match refused {
        Some(true) => Some(true),
        Some(false) => hold,
        None => hold.filter(|hold| *hold),
    };
    let no_orders = Some(needs_regions(member.mission) && inputs.mission_region_count == 0);
    (invalid, no_orders)
}

fn unlinked(member: &NavalMember<'_>) -> Option<bool> {
    Some(member.own && member.inputs?.repair_parent == 0)
}

pub fn at_sea_status(members: &[NavalMember<'_>], aggression: &[i64]) -> NavalStatus {
    let visible = || members.iter().filter(|member| member.visible);
    let ordered = |member: &&NavalMember<'_>| unlinked(member);
    let mut invalid = Vec::new();
    let mut no_orders = Vec::new();
    for member in visible() {
        match ordered(&member) {
            Some(false) => {}
            None => {
                invalid.push(None);
                no_orders.push(None);
            }
            Some(true) => {
                let (bad, none) = order_terms(member, aggression);
                invalid.push(bad);
                no_orders.push(none);
            }
        }
    }
    NavalStatus {
        no_orders: any(no_orders),
        invalid_orders: any(invalid),
        pride: any(visible().map(|member| member.pride)),
        active: all(members.iter().map(|member| {
            member
                .inputs
                .map(|i| mission_active(i.is_in_regions, member.mission, i.bombardment_region))
        })),
    }
}

pub fn port_status(members: &[NavalMember<'_>], aggression: &[i64]) -> NavalStatus {
    let mut invalid = Vec::new();
    let mut no_orders = Vec::new();
    for member in members {
        match unlinked(member) {
            Some(false) => {}
            None => {
                invalid.push(None);
                no_orders.push(None);
            }
            Some(true) => {
                let (bad, none) = order_terms(member, aggression);
                invalid.push(bad);
                no_orders.push(none);
            }
        }
    }
    NavalStatus {
        no_orders: any(no_orders),
        invalid_orders: any(invalid),
        pride: any(members.iter().map(|member| member.pride)),
        active: None,
    }
}

pub fn pride(
    task_force: u64,
    pride_ship: Option<u64>,
    ship_task_force: impl Fn(u64) -> Option<u64>,
    ships_complete: bool,
) -> Option<bool> {
    let ship = pride_ship?;
    if ship == 0 {
        return Some(false);
    }
    match ship_task_force(ship) {
        Some(owner) => Some(owner == task_force),
        None if ships_complete => Some(false),
        None => None,
    }
}

pub fn reserve_fleet(
    fleet: &FleetCounterInputRow,
    mission_of: impl Fn(u64) -> Option<u32>,
) -> Option<bool> {
    reserve_of(fleet.task_forces.first().copied(), mission_of)
}

pub fn reserve_of(first: Option<u64>, mission_of: impl Fn(u64) -> Option<u32>) -> Option<bool> {
    match first {
        None => Some(false),
        Some(first) => mission_of(first).map(|mission| mission == RESERVE_FLEET),
    }
}

pub fn color(red: u32, green: u32, blue: u32, alpha: u32) -> Option<[f32; 4]> {
    let rgba = [red, green, blue, alpha].map(f32::from_bits);
    rgba.iter().all(|value| value.is_finite()).then_some(rgba)
}

pub fn parent_branch(
    division: u64,
    orders_group: u64,
    orders: &(impl OrderFacts + ?Sized),
) -> Option<bool> {
    if orders_group == 0 {
        return Some(false);
    }
    if orders.group(orders_group)?.parent == 0 {
        return Some(false);
    }
    let Some((group, instance)) = resolved_order(orders, division, orders_group)? else {
        return Some(false);
    };
    if !orders.group(group)?.army_group {
        return Some(false);
    }
    Some(!orders.has_child((group, instance))?)
}

pub fn area_defence(
    division: u64,
    orders_group: u64,
    orders: &(impl OrderFacts + ?Sized),
) -> Option<bool> {
    if orders_group == 0 {
        return Some(false);
    }
    if orders.area_group(orders_group) {
        return Some(true);
    }
    let resolved = resolved_order(orders, division, orders_group)?;
    if let Some(key) = resolved
        && orders.order_type(key) == Some(AREA_DEFENCE_ORDER)
    {
        return Some(true);
    }
    if let Some(key) = resolved {
        orders.order_type(key)?;
        return orders.orders_complete().then_some(false);
    }
    let parent = orders.group(orders_group)?.parent;
    if parent != 0 && orders.area_group(parent) {
        return Some(true);
    }
    orders.orders_complete().then_some(false)
}

pub trait OrderFacts {
    fn group(&self, group: u64) -> Option<GroupFacts>;
    fn has_child(&self, order: (u64, u32)) -> Option<bool>;
    fn order_type(&self, order: (u64, u32)) -> Option<i32>;
    fn area_group(&self, group: u64) -> bool;
    fn orders_complete(&self) -> bool;
    fn memberships(&self, group: u64, set: usize) -> Option<&AssignedOrders>;
}

fn resolved_order(
    orders: &(impl OrderFacts + ?Sized),
    division: u64,
    group: u64,
) -> Option<Option<(u64, u32)>> {
    if group == 0 {
        return Some(None);
    }
    for set in 0..2 {
        if let Some(instance) = orders.memberships(group, set)?.for_division(division)? {
            return Some(Some((group, instance)));
        }
        let parent = orders.group(group)?.parent;
        if parent != 0
            && let Some(instance) = orders.memberships(parent, set)?.for_division(division)?
        {
            return Some(Some((parent, instance)));
        }
    }
    Some(None)
}

pub const AREA_DEFENCE_ORDER: i32 = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroupFacts {
    pub parent: u64,
    pub army_group: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssignedOrders {
    first: BTreeMap<u64, u32>,
    complete: bool,
}

impl AssignedOrders {
    fn read(group: u64, roots: &[u32], plans: &BTreeMap<(u64, u32), &ArmyOrderPlanRow>) -> Self {
        Self::of(roots.iter().map(|&instance| {
            let plan = plans.get(&(group, instance));
            (instance, plan.map(|plan| plan.assigned_divisions.iter().copied()))
        }))
    }

    pub fn of<D: IntoIterator<Item = u64>>(roots: impl IntoIterator<Item = (u32, Option<D>)>) -> Self {
        let mut out = Self {
            complete: true,
            ..Self::default()
        };
        for (instance, plan) in roots {
            let Some(divisions) = plan else {
                out.complete = false;
                break;
            };
            for division in divisions {
                out.first.entry(division).or_insert(instance);
            }
        }
        out
    }

    pub fn bytes(&self) -> usize {
        self.first.len() * 24
    }

    pub fn for_division(&self, division: u64) -> Option<Option<u32>> {
        self.first
            .get(&division)
            .copied()
            .map(Some)
            .or_else(|| self.complete.then_some(None))
    }
}

#[derive(Clone, Debug, Default)]
pub struct OrdersIndex {
    pub groups: BTreeMap<u64, GroupFacts>,
    pub has_child: BTreeMap<(u64, u32), bool>,
    pub colors: BTreeMap<u64, Option<[f32; 4]>>,
    pub order_types: BTreeMap<(u64, u32), i32>,
    pub area_groups: BTreeSet<u64>,
    pub orders_complete: bool,
    memberships: BTreeMap<u64, [AssignedOrders; 2]>,
}

impl OrdersIndex {
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let orders = rows::<tables::ArmyOrders>(frame)?;
        let plans = rows::<tables::ArmyOrderPlans>(frame)?;
        let plans: BTreeMap<_, _> = plans
            .rows()
            .iter()
            .map(|row| ((row.owner_group, row.instance), row))
            .collect();
        let group_plans = rows::<tables::OrdersGroupPlans>(frame)?;
        Ok(Self {
            groups: rows::<tables::OrdersGroups>(frame)?
                .rows()
                .iter()
                .map(|row: &OrdersGroupRow| {
                    (
                        row.handle,
                        GroupFacts {
                            parent: row.parent,
                            army_group: row.is_army_group != 0,
                        },
                    )
                })
                .collect(),
            has_child: plans
                .iter()
                .map(|(&key, row)| (key, row.has_child != 0))
                .collect(),
            colors: group_plans
                .rows()
                .iter()
                .map(|row| (row.group, color(row.red, row.green, row.blue, row.alpha)))
                .collect(),
            order_types: orders
                .rows()
                .iter()
                .map(|row| ((row.owner_group, row.instance), row.order_type))
                .collect(),
            area_groups: orders
                .rows()
                .iter()
                .filter(|row| !row.target_states.is_empty())
                .map(|row| row.owner_group)
                .collect(),
            orders_complete: orders.is_complete(),
            memberships: group_plans
                .rows()
                .iter()
                .map(|row| {
                    (
                        row.group,
                        [
                            AssignedOrders::read(row.group, &row.primary_orders, &plans),
                            AssignedOrders::read(row.group, &row.fallback_orders, &plans),
                        ],
                    )
                })
                .collect(),
        })
    }

    pub fn root_order_type(&self, division: u64, group: u64) -> Option<Option<i32>> {
        if group == 0 {
            return Some(None);
        }
        if let Some(instance) = self.memberships.get(&group)?[0].for_division(division)? {
            return self.order_types.get(&(group, instance)).copied().map(Some);
        }
        let parent = self.groups.get(&group)?.parent;
        if parent == 0 {
            return Some(None);
        }
        match self.memberships.get(&parent)?[0].for_division(division)? {
            Some(instance) => self.order_types.get(&(parent, instance)).copied().map(Some),
            None => Some(None),
        }
    }

    pub fn marker(&self, orders_group: u64, branch: Option<bool>) -> Result<Option<[f32; 4]>, u64> {
        if orders_group == 0 {
            return Ok(None);
        }
        let Some(branch) = branch else {
            return Ok(None);
        };
        let group = if branch {
            let Some(facts) = self.groups.get(&orders_group) else {
                return Ok(None);
            };
            facts.parent
        } else {
            orders_group
        };
        match self.colors.get(&group) {
            None => Ok(None),
            Some(Some(rgba)) => Ok(Some(*rgba)),
            Some(None) => Err(group),
        }
    }
}

impl OrderFacts for OrdersIndex {
    fn group(&self, group: u64) -> Option<GroupFacts> {
        self.groups.get(&group).copied()
    }

    fn has_child(&self, order: (u64, u32)) -> Option<bool> {
        self.has_child.get(&order).copied()
    }

    fn order_type(&self, order: (u64, u32)) -> Option<i32> {
        self.order_types.get(&order).copied()
    }

    fn area_group(&self, group: u64) -> bool {
        self.area_groups.contains(&group)
    }

    fn orders_complete(&self) -> bool {
        self.orders_complete
    }

    fn memberships(&self, group: u64, set: usize) -> Option<&AssignedOrders> {
        self.memberships.get(&group).map(|sets| &sets[set])
    }
}

#[derive(Clone, Default)]
pub struct NativeInputs {
    task_forces: BTreeMap<u64, TaskForceCounterInputRow>,
    fleets: BTreeMap<u64, FleetCounterInputRow>,
    prides: BTreeMap<u32, u64>,
    withdrawing: BTreeMap<u64, u8>,
    leader_tags: BTreeMap<u64, Vec<u8>>,
}

impl NativeInputs {
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        Ok(Self {
            task_forces: rows::<tables::TaskForceCounterInputs>(frame)?
                .rows()
                .iter()
                .map(|row| (row.task_force, row.clone()))
                .collect(),
            fleets: rows::<tables::FleetCounterInputs>(frame)?
                .rows()
                .iter()
                .map(|row| (row.fleet, row.clone()))
                .collect(),
            prides: rows::<tables::CountryCounterInputs>(frame)?
                .rows()
                .iter()
                .map(|row| (row.country, row.pride_of_the_fleet))
                .collect(),
            withdrawing: rows::<tables::OrdersGroupCounterInputs>(frame)?
                .rows()
                .iter()
                .map(|row| (row.group, row.withdrawing))
                .collect(),
            leader_tags: rows::<tables::LeaderPortraitPaths>(frame)?
                .rows()
                .iter()
                .map(|row| (row.leader, row.tag.clone()))
                .collect(),
        })
    }

    pub fn task_force(&self, task_force: u64) -> Option<&TaskForceCounterInputRow> {
        self.task_forces.get(&task_force)
    }

    pub fn fleet(&self, fleet: u64) -> Option<&FleetCounterInputRow> {
        self.fleets.get(&fleet)
    }

    pub fn pride_ship(&self, country: u32) -> Option<u64> {
        self.prides.get(&country).copied()
    }

    pub fn withdrawing(&self, group: u64) -> Option<bool> {
        self.withdrawing.get(&group).map(|byte| *byte != 0)
    }

    pub fn leader_tag(&self, leader: u64) -> Option<&[u8]> {
        self.leader_tags.get(&leader).map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests;
