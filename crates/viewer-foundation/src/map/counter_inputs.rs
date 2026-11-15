//! Recorded counter inputs and the counter rules that consume them.
//!
//! Rules are pure over recorded rows; the host counter renderer draws their decisions.
//! A missing reached operand returns `None`, preserving the older-format fallback.
//!
//! Naval bars use [`strength_ratio`] and [`organisation_ratio`]. At-sea counters
//! average members whose visibility is at least 250; port counters include every
//! member. [`at_sea_status`] and [`port_status`] retain their separate status rules.
//! [`parent_branch`] decides orders-group inheritance.

use crate::map::unit_rules::mission_active;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use replay_data::{
    rows::ArmyOrderPlanRow, rows::FleetCounterInputRow, rows::OrdersGroupRow,
    rows::TaskForceCounterInputRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;

use crate::map::tables::rows;

/// The native fixed-point scale.
pub const SCALE: i64 = 100_000;
/// The value the ratio getters return for an unusable operand.
pub const UNDEFINED: i64 = 0xffff_ffff;
/// Raw fixed-point floor for a ship's maximum strength.
pub const MAXIMUM_FLOOR: i64 = 100;
/// Reserve-fleet mission kind.
pub const RESERVE_FLEET: u32 = 8;
/// Exclude the three unusable signed fixed-point sentinels: `i64::MAX`,
/// `i64::MIN` and `i64::MIN + 1`.
fn defined(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

/// Task-force strength sum over the sum of floored maxima, on the fixed-point
/// scale. Accumulation wraps. An empty roster returns zero; a nonpositive signed
/// maximum sum retains the undivided strength sum.
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

/// One ship's organisation ratio using its recorded maximum.
/// Zero maximum or an unusable current value returns the undefined sentinel.
pub fn ship_organisation_ratio(organisation: i64, maximum: i64) -> i64 {
    if maximum != 0 && defined(organisation) {
        organisation.wrapping_mul(SCALE) / maximum
    } else {
        UNDEFINED
    }
}

/// The mean of [`ship_organisation_ratio`] over the roster, zero for an empty one.
/// Every value, including sentinels, joins a wrapping `u64` sum before the
/// aggregate's range check.
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

/// Mean fixed-point member value, preserving wrapping multiplication and
/// truncating division. No members returns `None`, hiding both bars.
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

/// The bar fraction the draw computes from a mean: `mean / 100000.0f`.
pub fn bar_fill(mean: i64) -> f32 {
    mean as f32 / SCALE as f32
}

/// Half the drawable length times its fill, rounded in f32 through the fixed-point
/// conversion and truncated to a whole step. Land, naval and port bars share this
/// placement step. Zero leaves the drawable visible.
pub fn bar_step(fill: f32, length: f32) -> i64 {
    let t = length * fill * 0.5f32;
    let positive = if 0.0 <= t { 1.0f32 } else { 0.0 };
    ((positive + t * SCALE as f32) - 0.5f32) as i64 / SCALE
}

/// Drawn bar length is twice [`bar_step`] plus two pixels. The left edge stays
/// fixed while each step advances the right edge by two pixels.
pub fn bar_pixels(fill: f32, length: f32) -> f32 {
    (2 * bar_step(fill, length) + 2).clamp(0, length as i64 + 1) as f32
}

/// The missions that need a mission region.
pub fn needs_regions(mission: u32) -> bool {
    matches!(mission, 1..=6 | 9)
}

/// Use the detached activity's mission while that activity is set; otherwise use the task force's mission.
pub fn effective_mission(mission: u32, detached_activity: i32, detached_mission: i32) -> u32 {
    if detached_activity == 0 {
        mission
    } else {
        detached_mission as u32
    }
}

/// Strike-force or convoy-raiding orders fail when their required flag is set or
/// the content's fixed-point aggression entry is below one.
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

/// One task force as the naval counter's status loops see it.
#[derive(Clone, Copy, Debug)]
pub struct NavalMember<'a> {
    /// Visibility is at least 250.
    pub visible: bool,
    /// Whether the roster owner is the viewing country. Both owners are canonical country slots,
    /// so aliased country links compare equal.
    pub own: bool,
    /// The recorded `TaskForces.mission`.
    pub mission: u32,
    /// Whether the task force belongs to a fleet.
    pub has_fleet: bool,
    /// The format-24 row; `None` when the hour does not record it.
    pub inputs: Option<&'a TaskForceCounterInputRow>,
    /// The fleet's mission-region count; `None` when unrecorded.
    pub fleet_regions: Option<i32>,
    /// [`pride`] for this task force.
    pub pride: Option<bool>,
}

/// What a naval counter's status children show; `None` for a child the hour cannot settle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavalStatus {
    /// `no_orders`.
    pub no_orders: Option<bool>,
    /// `invalid_orders`.
    pub invalid_orders: Option<bool>,
    /// `pride_of_the_fleet_icon`.
    pub pride: Option<bool>,
    /// At sea: every member passes [`mission_active`], which selects `count_bg` one-based frame 2.
    pub active: Option<bool>,
}

/// How a naval item updates its `low_supply` child this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavalSupply {
    /// Visibility below 250 leaves the child as it was.
    Retain,
    /// The updater writes this received-supply ratio; the thresholds pick a frame or hide it.
    Write(i64),
    /// A member's hourly inputs are not recorded.
    Unavailable,
}

/// The game updates the port item's supply child
/// only at full first-member intel, with the minimum over every member, seeded at full supply.
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

/// Write the last fully visible member's ratio at sea,
/// seeded at full supply.
pub fn sea_supply(members: &[NavalMember<'_>]) -> NavalSupply {
    members
        .iter()
        .filter(|member| member.visible)
        .try_fold(SCALE, |_, member| {
            member.inputs.map(|row| row.received_supply_ratio)
        })
        .map_or(NavalSupply::Unavailable, NavalSupply::Write)
}

/// Three-valued "any": true if any term is true, unknown if none is and one is unknown.
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

/// Three-valued "all".
fn all(terms: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    any(terms.into_iter().map(|term| term.map(|value| !value))).map(|value| !value)
}

/// Independent order-icon terms for an own task force without a repair link: its
/// order state, effective hold mission, and availability of accessible fleet regions.
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

/// A task force enters the order loops only when both parts of its packed repair
/// reference are zero.
fn unlinked(member: &NavalMember<'_>) -> Option<bool> {
    Some(member.own && member.inputs?.repair_parent == 0)
}

/// At-sea pride and order glyphs use visible members; order terms additionally
/// require own task forces without repair links. Every member must have an active
/// mission for the plate frame, regardless of visibility.
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

/// Port status uses every member's pride and every own unlinked member's order
/// terms, regardless of visibility. Port items have no active-mission child.
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

/// Whether the roster owner's pride-of-the-fleet ship resolves to this task force.
/// `pride_ship` is `CountryCounterInputs.pride_of_the_fleet` (`None` when unrecorded),
/// `ship_task_force` looks the ship up in `Ships` (`None` when absent), and `ships_complete` says
/// whether an absent ship is a dangling reference rather than an unrecorded row.
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

/// Whether a fleet's ordered roster is non-empty and its first task force's mission is reserve.
/// `mission_of` returns `TaskForces.mission`.
pub fn reserve_fleet(
    fleet: &FleetCounterInputRow,
    mission_of: impl Fn(u64) -> Option<u32>,
) -> Option<bool> {
    reserve_of(fleet.task_forces.first().copied(), mission_of)
}

/// [`reserve_fleet`] from values: the fleet's first task force, if its roster holds one.
pub fn reserve_of(first: Option<u64>, mission_of: impl Fn(u64) -> Option<u32>) -> Option<bool> {
    match first {
        None => Some(false),
        Some(first) => mission_of(first).map(|mission| mission == RESERVE_FLEET),
    }
}

/// A recorded colour's four component bit patterns as floats. Corrupt payload decodes to
/// whatever float the bits name, including non-finite values; there is no validation by design.
pub fn color(red: u32, green: u32, blue: u32, alpha: u32) -> [f32; 4] {
    [red, green, blue, alpha].map(f32::from_bits)
}

/// Use the parent branch when the unit has a group and a resolved member order
/// owned by an army group, with no child sections, and that group has a parent.
/// Order ownership is not compared with the parent. Membership uses ordered
/// roots and assigned members; missing referenced groups or orders stay unknown.
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

/// The `defending_area` term for one member: the unit's orders group exists and
/// either its resolved order (the primary root, else the fallback) is order type **5**,
/// or some order of that group carries area-defence target states, or, only when no
/// order resolved, some order of the group's parent does.
///
/// Missing order memberships remain unknown. A reached area-defence order can
/// prove the positive branch in a truncated census; excluding all such orders
/// requires a complete `ArmyOrders` census.
pub fn area_defence(
    division: u64,
    orders_group: u64,
    orders: &(impl OrderFacts + ?Sized),
) -> Option<bool> {
    if orders_group == 0 {
        return Some(false);
    }
    // This branch is true regardless of which order resolves, so incomplete
    // membership cannot hide a reached own-group area-defence target.
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

/// What [`parent_branch`] and [`area_defence`] read of the hour's orders: [`OrdersIndex`] for a
/// frame of rows, or a reader's own index kept from changes.
pub trait OrderFacts {
    fn group(&self, group: u64) -> Option<GroupFacts>;
    fn has_child(&self, order: (u64, u32)) -> Option<bool>;
    fn order_type(&self, order: (u64, u32)) -> Option<i32>;
    /// Whether some order of `group` carries area-defence target states.
    fn area_group(&self, group: u64) -> bool;
    /// Whether `ArmyOrders` is complete.
    fn orders_complete(&self) -> bool;
    /// `group`'s primary (`set` 0) or fallback (1) memberships; `None` without its plan row.
    fn memberships(&self, group: u64, set: usize) -> Option<&AssignedOrders>;
}

/// Rebuild membership from the group then its immediate parent. Primary matches
/// outrank fallback matches, so a parent primary outranks an own fallback.
/// Never recurse through ancestors.
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

/// The recorded order type for an area-defence order.
pub const AREA_DEFENCE_ORDER: i32 = 5;

/// The parent group and army-group flag [`parent_branch`] reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroupFacts {
    pub parent: u64,
    pub army_group: bool,
}

/// First assigned root per division, stopping at the first unavailable root.
/// A reached plan row carries its entire assigned-member vector, even when
/// other rows in the table were truncated. Later roots cannot change a match.
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

    /// From each root in order, with its plan's assigned divisions or `None` without its plan row.
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

    /// The bytes its index holds.
    pub fn bytes(&self) -> usize {
        self.first.len() * 24
    }

    /// Outer `None` is unknown; inner `None` is a proved absence.
    pub fn for_division(&self, division: u64) -> Option<Option<u32>> {
        self.first
            .get(&division)
            .copied()
            .map(Some)
            .or_else(|| self.complete.then_some(None))
    }
}

/// Every recorded orders group's [`GroupFacts`] and colour, and whether each order has a child.
/// Duplicate keys would be a corrupt hour; collection keeps the last row, matching
/// [`NativeInputs::read`].
#[derive(Clone, Debug, Default)]
pub struct OrdersIndex {
    pub groups: BTreeMap<u64, GroupFacts>,
    pub has_child: BTreeMap<(u64, u32), bool>,
    /// Each group's colour from `OrdersGroupPlans`.
    pub colors: BTreeMap<u64, [f32; 4]>,
    /// The order type by `(owner group, instance)`.
    pub order_types: BTreeMap<(u64, u32), i32>,
    /// Groups holding at least one order with a non-empty target-state vector.
    pub area_groups: BTreeSet<u64>,
    /// Whether `ArmyOrders` is complete; [`area_defence`] states the gap rather than guessing.
    pub orders_complete: bool,
    /// Ordered primary/fallback memberships, indexed once per prepared hour.
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

    /// Primary root getter used by ability communications.
    /// Own primary precedes the immediate parent's primary; fallback orders
    /// and further ancestors do not participate. Outer None is unavailable.
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

    /// Own counter group-marker colour: the representative group's recorded colour,
    /// or its parent's when [`parent_branch`] holds. No palette lookup is involved.
    pub fn marker(&self, orders_group: u64, branch: Option<bool>) -> Option<[f32; 4]> {
        if orders_group == 0 {
            return None;
        }
        let group = if branch? {
            self.groups.get(&orders_group)?.parent
        } else {
            orders_group
        };
        self.colors.get(&group).copied()
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

/// This hour's counter simulation inputs, indexed.
#[derive(Clone, Default)]
pub struct NativeInputs {
    task_forces: BTreeMap<u64, TaskForceCounterInputRow>,
    fleets: BTreeMap<u64, FleetCounterInputRow>,
    /// Raw country link id (`Countries.id`) to its pride-of-the-fleet ship.
    prides: BTreeMap<u32, u64>,
    withdrawing: BTreeMap<u64, u8>,
    leader_tags: BTreeMap<u64, Vec<u8>>,
}

impl NativeInputs {
    /// A valid hour carries unique keys per table, so last-wins collection
    /// is exact; duplicates would be a corrupt hour.
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

    /// The pride-of-the-fleet ship for a raw country link id.
    pub fn pride_ship(&self, country: u32) -> Option<u64> {
        self.prides.get(&country).copied()
    }

    /// Whether the orders group is withdrawing; `None` when unrecorded.
    pub fn withdrawing(&self, group: u64) -> Option<bool> {
        self.withdrawing.get(&group).map(|byte| *byte != 0)
    }

    /// The string for a leader, when recorded.
    pub fn leader_tag(&self, leader: u64) -> Option<&[u8]> {
        self.leader_tags.get(&leader).map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests;
