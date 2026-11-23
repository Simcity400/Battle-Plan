
use std::cell::Cell;
use viewer_gpu::province_geometry::ProvinceGeometry;
use viewer_gpu::ribbon_content::*;
use viewer_gpu::ribbon_mesh::{emit, ribbon_height, sort_draws, span};
pub(crate) mod allied;
mod chain;
mod execution;
use viewer_gpu::ribbon_fixed as fixed;
mod group_sections;
pub mod invasion;
mod labels;
mod offensive;
mod paradrop_counts;
mod single;
mod submit;
mod transport;
mod transport_counts;

pub use labels::LabelInputs;

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_map::{map::MapData, movement::MovementMap};
use replay_data::{
    rows::ArmyOrderPlanRow, rows::ArmyOrderRow, rows::FrontRow, rows::LandTheatreRow,
    rows::OrdersGroupPlanRow, rows::OrdersGroupRow, rows::TableId, rows::tables,
};
use replay_frame::frame::DecodedFrame;

#[cfg(test)]
use self::fixed::Smoothing;
use self::fixed::{Point, SCALE};
use viewer_runtime::name_draw::{Placements, WorldName};
use viewer_gpu::province_map::HeightField;
use viewer_foundation::tables as table_gate;


pub const TYPE_MOVE: i32 = 1;
pub const TYPE_LINE: i32 = 2;
pub const TYPE_INVASION: i32 = 3;
pub const TYPE_PARADROP: i32 = 4;
pub const TYPE_AREA_DEFENSE: i32 = 5;

const IMPASSABLE: u8 = 4;



#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classified {
    FrontLine,
    OffensiveLine,
    Move,
    Invasion,
    Paradrop,
    GarrisonArea,
    Unknown,
}

impl Classified {
    pub fn arrow_type(self, plan: &ArmyOrderPlanRow, army_group: bool) -> Option<&'static str> {
        let ag = army_group && plan.owner_override == 0;
        Some(match self {
            Self::FrontLine => {
                if army_group {
                    if plan.has_child == 0 {
                        "ag_defence_line_no_child"
                    } else {
                        "ag_defence_line"
                    }
                } else {
                    "defence_line"
                }
            }
            Self::OffensiveLine => {
                if plan.virtual_order != 0 {
                    "virtual_line"
                } else if plan.fallback != 0 {
                    "fallback_line"
                } else if plan.blitz != 0 {
                    "blitz_line"
                } else if plan.withdraw != 0 {
                    "withdraw_line"
                } else if ag {
                    "ag_offence_line"
                } else {
                    "offence_line"
                }
            }
            Self::Move => {
                if plan.blitz != 0 {
                    "blitz"
                } else if plan.withdraw != 0 {
                    "withdraw"
                } else if ag {
                    "ag_move_thick"
                } else {
                    "move_thick"
                }
            }
            Self::Invasion => "invasion",
            Self::Paradrop => "paradrop",
            Self::GarrisonArea | Self::Unknown => return None,
        })
    }
}

fn classify(order: &ArmyOrderRow, plan: Option<&ArmyOrderPlanRow>) -> Classified {
    if plan.is_some_and(|p| p.front_valid != 0) {
        return Classified::FrontLine;
    }
    match order.order_type {
        TYPE_MOVE => Classified::Move,
        TYPE_LINE if plan.is_some() => Classified::OffensiveLine,
        TYPE_LINE => Classified::Unknown,
        TYPE_INVASION => Classified::Invasion,
        TYPE_PARADROP => Classified::Paradrop,
        TYPE_AREA_DEFENSE => Classified::GarrisonArea,
        _ => Classified::Unknown,
    }
}

pub fn chain(order: &ArmyOrderRow, kind: Classified) -> Vec<u32> {
    match kind {
        Classified::Invasion
        | Classified::Paradrop
        | Classified::GarrisonArea
        | Classified::Unknown => Vec::new(),
        Classified::Move if order.path.len() >= 5 => order.path[..2]
            .iter()
            .chain(order.path[3..order.path.len() - 2].iter().step_by(2))
            .chain(&order.path[order.path.len() - 2..])
            .copied()
            .collect(),
        _ => order.path.clone(),
    }
}


type OrderKey = (u64, u32);

fn ordered_group_roots<'a>(
    groups: &BTreeMap<u64, &OrdersGroupRow>,
    theatres: &[LandTheatreRow],
    roots: &'a [OrdersGroupPlanRow],
    links: &BTreeMap<u32, u32>,
    countries: &[u32],
    show_allied: bool,
    refused: &mut BTreeMap<&'static str, usize>,
) -> Vec<&'a OrdersGroupPlanRow> {
    let recorded_theatres: BTreeSet<_> = theatres.iter().map(|theatre| theatre.id).collect();
    let unknown_owners: BTreeSet<_> = theatres
        .iter()
        .filter(|theatre| theatre.owner != 0 && !links.contains_key(&theatre.owner))
        .map(|theatre| theatre.id)
        .collect();
    let theatres: BTreeMap<_, _> = theatres
        .iter()
        .filter_map(|theatre| {
            let country = if theatre.owner == 0 {
                0
            } else {
                *links.get(&theatre.owner)?
            };
            countries
                .contains(&country)
                .then_some((theatre.id, (country, theatre.ordinal)))
        })
        .collect();
    let mut ordered: BTreeMap<u32, BTreeMap<_, _>> = BTreeMap::new();
    for root in roots {
        let Some(group) = groups.get(&root.group) else {
            *refused
                .entry("group plan whose orders group is not recorded")
                .or_default() += 1;
            continue;
        };
        let same_country = if group.country == 0 {
            Some(countries.first() == Some(&0))
        } else {
            links
                .get(&group.country)
                .map(|country| Some(country) == countries.first())
        };
        if !show_allied && same_country == Some(false) {
            continue;
        }
        let Some(&(country, theatre)) = theatres.get(&group.theatre) else {
            if unknown_owners.contains(&group.theatre) {
                *refused
                    .entry("group whose theatre owner has no CountryLinks row")
                    .or_default() += 1;
            } else if !recorded_theatres.contains(&group.theatre) {
                *refused
                    .entry("group whose theatre roster is not recorded")
                    .or_default() += 1;
            }
            continue;
        };
        if !show_allied && same_country.is_none() {
            *refused
                .entry("group country has no CountryLinks row")
                .or_default() += 1;
            continue;
        }
        let Some(ordinal) = group.theatre_ordinal else {
            *refused
                .entry("group without a theatre roster position")
                .or_default() += 1;
            continue;
        };
        let key = (theatre, group.is_army_group == 0, ordinal);
        if ordered
            .entry(country)
            .or_default()
            .insert(key, root)
            .is_some()
        {
            *refused
                .entry("duplicate theatre roster position")
                .or_default() += 1;
        }
    }
    countries
        .iter()
        .filter_map(|country| ordered.get(country))
        .flat_map(|roots| roots.values().copied())
        .collect()
}

fn first_linked<'a>(
    owner: u64,
    ids: &[u32],
    order_type: i32,
    orders: &BTreeMap<OrderKey, &'a ArmyOrderRow>,
) -> Option<&'a ArmyOrderRow> {
    ids.iter()
        .filter_map(|id| orders.get(&(owner, *id)).copied())
        .find(|order| order.order_type == order_type)
}

fn drawing_order<'a>(
    groups: &[&OrdersGroupPlanRow],
    orders: &BTreeMap<OrderKey, &'a ArmyOrderRow>,
    plans: &BTreeMap<OrderKey, &ArmyOrderPlanRow>,
    refused: &mut BTreeMap<&'static str, usize>,
) -> Vec<&'a ArmyOrderRow> {
    let mut output = Vec::new();
    for group in groups {
        let keys: Vec<OrderKey> = orders
            .range((group.group, 0)..=(group.group, u32::MAX))
            .map(|(key, _)| *key)
            .collect();
        if keys.is_empty() {
            continue;
        }
        let complete = keys.iter().all(|key| {
            plans.get(key).is_some_and(|plan| {
                plan.predecessors
                    .iter()
                    .chain(&plan.successors)
                    .all(|id| orders.contains_key(&(key.0, *id)))
            })
        }) && group
            .primary_orders
            .iter()
            .chain(&group.fallback_orders)
            .all(|id| orders.contains_key(&(group.group, *id)));
        if !complete {
            *refused
                .entry("group whose plan graph names an absent order")
                .or_default() += 1;
            continue;
        }
        for &root in group.primary_orders.iter().chain(&group.fallback_orders) {
            let start = output.len();
            let mut active = BTreeSet::new();
            let mut stack = vec![((group.group, root), false)];
            let mut cycled = false;
            while let Some((key, leaving)) = stack.pop() {
                if leaving {
                    active.remove(&key);
                    continue;
                }
                if !active.insert(key) {
                    cycled = true;
                    break;
                }
                stack.push((key, true));
                output.push(orders[&key]);
                let plan = plans[&key];
                if let [child] = plan.successors.as_slice() {
                    let next = plans[&(key.0, *child)];
                    if next.predecessors.len() > 1 && next.predecessors[0] != key.1 {
                        continue;
                    }
                }
                for &child in plan.successors.iter().rev() {
                    stack.push(((key.0, child), false));
                }
            }
            if cycled {
                output.truncate(start);
                *refused
                    .entry("root whose successor graph cycles")
                    .or_default() += 1;
            }
        }
    }
    output
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum FrontSpan {
    Pairs(Vec<[u32; 2]>),
    Destroyed,
}

fn front_pairs(plan: &ArmyOrderPlanRow, front: &FrontRow) -> Option<FrontSpan> {
    let section = front
        .section_ids
        .iter()
        .position(|id| *id == plan.section)?;
    let first = if section == 0 {
        0
    } else {
        front.section_ends[section - 1] as usize
    };
    let words = front
        .province_pairs
        .get(first..front.section_ends[section] as usize)?;
    let (Ok(start), Ok(end)) = (
        usize::try_from(plan.segment_start),
        usize::try_from(plan.segment_end),
    ) else {
        return Some(FrontSpan::Destroyed);
    };
    if end >= words.len() / 2 {
        return Some(FrontSpan::Destroyed);
    }
    if start > end {
        return Some(FrontSpan::Pairs(Vec::new()));
    }
    Some(FrontSpan::Pairs(
        words[start * 2..(end + 1) * 2]
            .chunks_exact(2)
            .map(|pair| [pair[0], pair[1]])
            .collect(),
    ))
}

fn nearest_control(points: &[Point], target: Point, width: i64) -> Option<Point> {
    let mut best = None;
    let mut distance = 0x2540_a5d60_i128;
    for &[mut x, z] in points {
        if width > 0 {
            if x > width {
                x -= width;
            } else if x < 0 {
                x += width;
            }
            if (x - target[0]).abs() > width / 2 {
                x += if target[0] < x { -width } else { width };
            }
        }
        let dx = i128::from((x - target[0]) / 2);
        let dz = i128::from((z - target[1]) / 2);
        let candidate = dx * dx / 100_000 + dz * dz / 100_000;
        if candidate < distance {
            distance = candidate;
            best = Some([x, z]);
        }
    }
    best
}


pub const TABLES: &[TableId] = &[
    TableId::ArmyOrders,
    TableId::OrdersGroups,
    TableId::Divisions,
    TableId::ArmyOrderPlans,
    TableId::Fronts,
    TableId::OrdersGroupPlans,
    TableId::LandTheatres,
    TableId::Provinces,
    TableId::CountryLinks,
];

pub fn memo_tables(show_allied: bool) -> Vec<TableId> {
    TABLES
        .iter()
        .chain(if show_allied { allied::TABLES } else { &[] })
        .copied()
        .collect()
}

pub fn own_order_count(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    ours: u32,
) -> io::Result<usize> {
    let groups = table_gate::rows::<tables::OrdersGroups>(frame)?;
    let groups = groups
        .rows()
        .iter()
        .map(|group| (group.handle, group))
        .collect();
    let theatres = table_gate::rows::<tables::LandTheatres>(frame)?;
    let roots = table_gate::rows::<tables::OrdersGroupPlans>(frame)?;
    let roots = ordered_group_roots(
        &groups,
        theatres.rows(),
        roots.rows(),
        links,
        &[ours],
        false,
        &mut BTreeMap::new(),
    );
    let reached: BTreeSet<_> = roots.iter().map(|root| root.group).collect();
    Ok(table_gate::rows::<tables::ArmyOrders>(frame)?
        .rows()
        .iter()
        .filter(|order| reached.contains(&order.owner_group))
        .count())
}

pub fn requires_full_frame(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    ours: u32,
    show_allied: bool,
    labels: bool,
) -> io::Result<bool> {
    let groups = table_gate::rows::<tables::OrdersGroups>(frame)?;
    let groups = groups
        .rows()
        .iter()
        .map(|group| (group.handle, group))
        .collect();
    let theatres = table_gate::rows::<tables::LandTheatres>(frame)?;
    let roots = table_gate::rows::<tables::OrdersGroupPlans>(frame)?;
    let selection = allied::Selection::read(frame, links, ours, show_allied);
    let roots = ordered_group_roots(
        &groups,
        theatres.rows(),
        roots.rows(),
        links,
        &selection.countries,
        show_allied,
        &mut BTreeMap::new(),
    );
    let reached: BTreeSet<_> = roots.iter().map(|root| root.group).collect();
    Ok(table_gate::rows::<tables::ArmyOrders>(frame)?
        .rows()
        .iter()
        .any(|order| {
            reached.contains(&order.owner_group)
                && (order.active != 0
                    || order.order_type == TYPE_INVASION
                    || labels && order.order_type == TYPE_PARADROP)
        }))
}

pub struct OrderInputs<'a> {
    pub assets: &'a OrderAssets,
    pub transport: Option<viewer_unit_reads::air_transport_content::AirTransportContent<'a>>,
    pub reads: Option<viewer_unit_reads::movement_reads::ReadOperands<'a>>,
    pub geometry: &'a ProvinceGeometry,
    pub movement: Option<&'a MovementMap>,
    pub map: &'a MapData,
    pub heights: &'a HeightField<'a>,
    pub ours: u32,
    pub show_allied: bool,
    pub links: &'a BTreeMap<u32, u32>,
    pub labels: Option<LabelInputs<'a>>,
}

#[derive(Clone, Debug, Default)]
struct Prepared {
    controls: Vec<Point>,
    raw: Vec<Point>,
    side: i32,
    snapped: bool,
    offset: bool,
    lift: Option<transport::Lift>,
    empty_span: bool,
    children: Vec<group_sections::Prepared>,
}

fn group_color(row: &OrdersGroupPlanRow) -> [f32; 4] {
    [
        f32::from_bits(row.red),
        f32::from_bits(row.green),
        f32::from_bits(row.blue),
        f32::from_bits(row.alpha),
    ]
}

fn callback_color(
    defines: &OrderDefines,
    virtual_order: bool,
    owner_is_ours: Option<bool>,
    display: Option<&OrdersGroupPlanRow>,
) -> Option<[f32; 4]> {
    let color = if virtual_order {
        defines.virtual_color
    } else if owner_is_ours? {
        group_color(display?)
    } else {
        defines.allied_color
    };
    color.iter().all(|value| value.is_finite()).then_some(color)
}

fn owner_is_ours(group: &OrdersGroupRow, inputs: &OrderInputs<'_>) -> Option<bool> {
    if group.country == 0 {
        Some(inputs.ours == 0)
    } else {
        inputs
            .links
            .get(&group.country)
            .map(|&country| country == inputs.ours)
    }
}

pub fn build(inputs: &OrderInputs<'_>, frame: &DecodedFrame) -> io::Result<OrderField> {
    let at = stage("plan.tables");
    let transport_context = paradrop_counts::Context::new(frame, inputs.transport);
    let orders = table_gate::rows::<tables::ArmyOrders>(frame)?;
    let groups = table_gate::rows::<tables::OrdersGroups>(frame)?;
    let plans = table_gate::rows::<tables::ArmyOrderPlans>(frame)?;
    let fronts = table_gate::rows::<tables::Fronts>(frame)?;
    let group_plans = table_gate::rows::<tables::OrdersGroupPlans>(frame)?;
    let theatres = table_gate::rows::<tables::LandTheatres>(frame)?;
    let child_division_groups = group_sections::DivisionGroups::read(frame);
    let plans: BTreeMap<_, _> = plans
        .rows()
        .iter()
        .map(|p| ((p.owner_group, p.instance), p))
        .collect();
    let fronts: BTreeMap<_, _> = fronts.rows().iter().map(|p| (p.front, p)).collect();
    let group_plan_rows: BTreeMap<_, _> = group_plans.rows().iter().map(|p| (p.group, p)).collect();
    let groups: BTreeMap<_, _> = groups
        .rows()
        .iter()
        .map(|group| (group.handle, group))
        .collect();
    let mut field = OrderField::default();
    field.report.recorded = orders.rows().len();
    for order in orders.rows() {
        let kind = classify(
            order,
            plans.get(&(order.owner_group, order.instance)).copied(),
        );
        *field
            .report
            .per_kind_recorded
            .entry(kind_name(kind))
            .or_default() += 1;
    }
    drop(at);
    let at = stage("plan.roots");
    let selection = allied::Selection::read(frame, inputs.links, inputs.ours, inputs.show_allied);
    field.report.allied_refused = selection.refused;
    let mut refused = BTreeMap::new();
    let roots = ordered_group_roots(
        &groups,
        theatres.rows(),
        group_plans.rows(),
        inputs.links,
        &selection.countries,
        inputs.show_allied,
        &mut refused,
    );
    let reached_groups: BTreeSet<_> = roots.iter().map(|root| root.group).collect();
    let mine: Vec<_> = orders
        .rows()
        .iter()
        .filter(|order| reached_groups.contains(&order.owner_group))
        .collect();
    field.report.ours = mine.len();
    if mine.is_empty() {
        field.report.refused = refused;
        return Ok(field);
    }
    let Some(movement) = inputs.movement else {
        field.report.unavailable = Some(
            "the content movement map (native neighbour order and crossings) could not be derived"
                .into(),
        );
        return Ok(field);
    };
    drop(at);
    let at = stage("plan.visits");
    let controllers = chain::Controllers::of(table_gate::core::<tables::Provinces>(frame)?);
    let indexed: BTreeMap<OrderKey, &ArmyOrderRow> = mine
        .iter()
        .map(|&order| ((order.owner_group, order.instance), order))
        .collect();
    let execution_context = execution::Context::new(
        frame,
        &inputs.assets.execution,
        &indexed,
        &plans,
        &groups,
        &transport_context,
    );
    let visits = drawing_order(&roots, &indexed, &plans, &mut refused);
    let reached: BTreeSet<OrderKey> = visits
        .iter()
        .map(|order| (order.owner_group, order.instance))
        .collect();
    let unreached = indexed.len() - reached.len();
    if unreached > 0 {
        *refused
            .entry("order not reached from its group's recorded roots")
            .or_default() += unreached;
    }

    drop(at);
    let at = stage("plan.geometry");
    let assets = inputs.assets;
    let defines = &assets.defines;
    let geometry = inputs.geometry;
    let centroid = |province: u32| geometry.map_position(province);
    let neighbours = |province: u32| movement.neighbours(province);
    let adjacent = |a: u32, b: u32| {
        movement.neighbours(a).is_some_and(|list| list.contains(&b))
            && movement
                .crossing_kind(a, b)
                .is_some_and(|kind| kind != IMPASSABLE)
    };
    let border_target = |a, b| movement.front_border_target(a, b);
    let offensive_world = offensive::World {
        centroid: &centroid,
        neighbours: &neighbours,
        adjacent: &adjacent,
    };
    let raster = chain::Raster {
        map: inputs.map,
        controllers,
        links: inputs.links,
    };
    let chain_world = chain::ChainWorld {
        borders: &geometry.borders,
        border_target: &border_target,
        raster: Some(&raster),
    };
    let centroid_controls = |path: &[u32], order_type: i32| -> Option<Vec<Point>> {
        let mut out = Vec::with_capacity(path.len() * 2);
        for (index, &province) in path.iter().enumerate() {
            let c = centroid(province)?;
            let mut point = fixed::from_pixel(c);
            if order_type == TYPE_MOVE
                && index == 0
                && path.len() > 1
                && let Some(crossing) = movement.neighbour_crossing(province, path[1])
            {
                point = fixed::from_pixel(crossing);
            }
            out.push(point);
            if let Some(&next) = path.get(index + 1)
                && let Some(crossing) = movement.neighbour_crossing(province, next)
            {
                out.push(fixed::from_pixel(crossing));
            }
        }
        Some(out)
    };
    const NO_CENTROID: &str = "order path province without a map centroid";

    let mut overlaps: BTreeMap<(u32, u32), u8> = BTreeMap::new();
    let mut prepared: BTreeMap<OrderKey, Prepared> = BTreeMap::new();
    let mut first_visit: Vec<OrderKey> = Vec::new();
    let mut seen: BTreeSet<OrderKey> = BTreeSet::new();
    let mut refused_orders: BTreeSet<(OrderKey, &'static str)> = BTreeSet::new();
    let mut invasion_routes = invasion::Routes::new(frame, inputs.reads);
    let mut refused_invasions = BTreeSet::new();
    for &order in &visits {
        let key = (order.owner_group, order.instance);
        if seen.insert(key) {
            first_visit.push(key);
        }
        let plan = plans[&key];
        let Some(group) = groups.get(&order.owner_group) else {
            continue;
        };
        let kind = classify(order, Some(plan));
        let Some(arrow) = kind
            .arrow_type(plan, group.is_army_group != 0)
            .and_then(|name| assets.types.get(name))
        else {
            continue;
        };
        let mut out = Prepared::default();
        match kind {
            Classified::FrontLine => {
                let pairs = match fronts
                    .get(&plan.front)
                    .and_then(|front| front_pairs(plan, front))
                {
                    Some(FrontSpan::Pairs(pairs)) => pairs,
                    Some(FrontSpan::Destroyed) => Vec::new(),
                    None => {
                        refuse(
                            &mut refused,
                            &mut refused_orders,
                            key,
                            "front binding outside its recorded section",
                        );
                        continue;
                    }
                };
                if pairs.is_empty() {
                    out.empty_span = true;
                    prepared.insert(key, out);
                    continue;
                }
                let reservation = if group.is_army_group != 0 { 2 } else { 1 };
                let pairs: Vec<chain::Pair> = pairs
                    .iter()
                    .map(|&[source, target]| {
                        let count = overlaps.entry((source, target)).or_default();
                        let units = *count;
                        *count = count.saturating_add(reservation).min(defines.max_offsets);
                        chain::Pair {
                            source,
                            target,
                            units,
                        }
                    })
                    .collect();
                if order.path.is_empty() {
                    out.empty_span = true;
                    prepared.insert(key, out);
                    continue;
                }
                let extra_units = match group_sections::extra_units(plan) {
                    Ok(value) => value,
                    Err(reason) => {
                        refuse(&mut refused, &mut refused_orders, key, reason);
                        continue;
                    }
                };
                let params = chain::ChainParams {
                    subdivided: true,
                    smoothing: defines.root_front,
                    extra_units,
                    offset: defines.root_front_offset,
                    max_intersection_tests: defines.root_front_max_intersection_tests_order,
                    probe_teeth: arrow.culling_direction != 0,
                };
                if group.is_army_group != 0 && order.order_type == TYPE_LINE {
                    match group_sections::prepare(
                        plan,
                        &pairs,
                        &chain_world,
                        assets.types.get(GROUP_SECTION_TYPE_NAME),
                        defines,
                    ) {
                        Ok((children, reasons)) => {
                            out.children = children;
                            for reason in reasons {
                                refuse(&mut refused, &mut refused_orders, key, reason);
                            }
                        }
                        Err(reason) => refuse(&mut refused, &mut refused_orders, key, reason),
                    }
                }
                let Some(built) = chain::build(&pairs, &chain_world, params) else {
                    refuse(
                        &mut refused,
                        &mut refused_orders,
                        key,
                        "front whose border chain could not be built",
                    );
                    prepared.insert(key, out);
                    continue;
                };
                out.controls = built.points;
                out.side = built.side.unwrap_or(0);
                out.offset = built.offset;
            }
            Classified::OffensiveLine => {
                let predecessor =
                    first_linked(order.owner_group, &plan.predecessors, TYPE_MOVE, &indexed);
                let approach = predecessor
                    .map(|order| order.path.as_slice())
                    .unwrap_or_default();
                if let Some(snap) = offensive::snap(&order.path, approach, &offensive_world) {
                    let pairs: Vec<chain::Pair> = snap
                        .pairs
                        .iter()
                        .map(|&[source, target]| chain::Pair {
                            source,
                            target,
                            units: 0,
                        })
                        .collect();
                    let params = chain::ChainParams {
                        subdivided: false,
                        smoothing: defines.snapped_off_front,
                        extra_units: 0,
                        offset: defines.root_front_offset,
                        max_intersection_tests: defines.root_front_max_intersection_tests_order,
                        probe_teeth: false,
                    };
                    let Some(built) = chain::build(&pairs, &chain_world, params) else {
                        refuse(
                            &mut refused,
                            &mut refused_orders,
                            key,
                            "snapped offensive line whose border chain could not be built",
                        );
                        continue;
                    };
                    out.controls = built.points;
                    out.side = snap.side;
                    out.snapped = true;
                } else if order.path.len() == 1 {
                    out.controls = single::points(
                        &geometry.centers,
                        inputs.map,
                        order.path[0],
                        approach,
                        centroid,
                    );
                    out.side = arrow.culling_direction;
                } else {
                    let Some(mut controls) = centroid_controls(&order.path, TYPE_LINE) else {
                        refuse(&mut refused, &mut refused_orders, key, NO_CENTROID);
                        continue;
                    };
                    fixed::curve(&mut controls, defines.order_front, true);
                    if arrow.culling_direction != 0
                        && let Some(side) =
                            offensive::unsnapped_side(&controls, approach, &offensive_world)
                    {
                        out.side = side;
                    }
                    out.controls = controls;
                }
            }
            Classified::Move => {
                let Some(raw) = centroid_controls(&chain(order, kind), TYPE_MOVE) else {
                    refuse(&mut refused, &mut refused_orders, key, NO_CENTROID);
                    continue;
                };
                out.raw = raw;
                let mut controls = out.raw.clone();
                fixed::curve(&mut controls, defines.order_move, true);
                out.controls = controls;
            }
            Classified::Paradrop => {
                let source = order.paradrop_target;
                let target = order.path.first().copied().unwrap_or(0);
                if (source as i32) < 1 || (target as i32) < 1 {
                    no_arrow(&mut field.report, &mut refused_orders, key);
                    continue;
                }
                let (Some(from), Some(to)) = (centroid(source), centroid(target)) else {
                    refuse(&mut refused, &mut refused_orders, key, NO_CENTROID);
                    continue;
                };
                out.controls = transport::paradrop_controls(
                    fixed::from_pixel(from),
                    fixed::from_pixel(to),
                    i64::from(geometry.width),
                );
                out.lift = transport::Lift::new(
                    &out.controls,
                    geometry.width as f32,
                    defines.paradrop_height_to_len,
                    defines.paradrop_height_max,
                    &|x, z| inputs.heights.native_height(x, z),
                );
            }
            Classified::Invasion => {
                if order.path.is_empty() {
                    no_arrow(&mut field.report, &mut refused_orders, key);
                    continue;
                }
                let route = match invasion_routes.route(order, plan, group) {
                    Ok(route) if route.is_empty() => {
                        no_arrow(&mut field.report, &mut refused_orders, key);
                        continue;
                    }
                    Ok(route) => route,
                    Err(reason) => {
                        if refused_invasions.insert(key) {
                            *field.report.invasion_refused.entry(reason).or_default() += 1;
                        }
                        continue;
                    }
                };
                let mut path = Vec::with_capacity(route.len() + 2);
                if (order.paradrop_target as i32) > 0 {
                    path.push(order.paradrop_target);
                }
                path.extend(route);
                path.push(order.path[0]);
                let Some(mut controls) = transport::invasion_controls(
                    &path,
                    i64::from(geometry.width),
                    |id| centroid(id).map(fixed::from_pixel),
                    |a, b| movement.neighbour_crossing(a, b).map(fixed::from_pixel),
                ) else {
                    refuse(&mut refused, &mut refused_orders, key, NO_CENTROID);
                    continue;
                };
                fixed::curve(&mut controls, defines.order_move, true);
                out.controls = controls;
            }
            Classified::GarrisonArea | Classified::Unknown => continue,
        }
        prepared.insert(key, out);
    }
    drop(at);
    let at = stage("plan.snap_ends");
    let width = i64::from(geometry.width) * SCALE;
    let snapped_ends: Vec<(OrderKey, Point)> = prepared
        .iter()
        .filter(|(_, p)| p.raw.len() > 1)
        .filter_map(|(key, _)| {
            let order = indexed.get(key)?;
            let plan = plans.get(key)?;
            let successor = first_linked(key.0, &plan.successors, TYPE_LINE, &indexed)?;
            let boundary = prepared.get(&(successor.owner_group, successor.instance))?;
            let target = fixed::from_pixel(centroid(*order.path.last()?)?);
            Some((*key, nearest_control(&boundary.controls, target, width)?))
        })
        .collect();
    for (key, end) in snapped_ends {
        let entry = prepared.get_mut(&key).expect("prepared above");
        if let Some(last) = entry.raw.last_mut() {
            *last = end;
        }
        let mut controls = entry.raw.clone();
        fixed::curve(&mut controls, defines.order_move, true);
        entry.controls = controls;
    }

    drop(at);
    let at = stage("plan.emit");
    let mut gap_total = 0u64;
    let mut gap_count = 0u64;
    let mut placed_labels = Vec::new();
    for key in first_visit {
        let order = indexed[&key];
        let plan = plans[&key];
        let kind = classify(order, Some(plan));
        let Some(group) = groups.get(&order.owner_group) else {
            *refused
                .entry("order whose owner group is not recorded")
                .or_default() += 1;
            continue;
        };
        let Some(name) = kind.arrow_type(plan, group.is_army_group != 0) else {
            if kind == Classified::Unknown {
                *refused
                    .entry("order of a type or binding no arrow is known for")
                    .or_default() += 1;
            } else {
                field.report.no_arrow += 1;
            }
            continue;
        };
        let Some(arrow) = assets.types.get(name) else {
            *refused
                .entry("arrow type the content's maparrows.txt does not declare")
                .or_default() += 1;
            continue;
        };
        let Some(entry) = prepared.get(&key) else {
            continue;
        };
        if entry.empty_span {
            field.report.no_arrow += 1;
            continue;
        }
        let callback = match execution_context.callback(order) {
            Ok(callback) => callback,
            Err(reason) => {
                *field.report.execution_refused.entry(reason).or_default() += 1;
                continue;
            }
        };
        let child_display = group_sections::Display {
            plan,
            group_plans: &group_plan_rows,
            groups: &groups,
            division_groups: &child_division_groups,
            owner_is_ours: owner_is_ours(group, inputs),
            kind: plan_kind(kind),
        };
        let display = if plan.owner_override != 0 {
            plan.owner_override
        } else {
            order.owner_group
        };
        let virtual_order = plan.virtual_order != 0;
        let color = callback_color(
            defines,
            virtual_order,
            owner_is_ours(group, inputs),
            group_plan_rows.get(&display).copied(),
        );
        let Some(color) = color else {
            *refused
                .entry("order whose callback colour is unavailable or nonfinite")
                .or_default() += 1;
            group_sections::emit(
                &mut field,
                &mut placed_labels,
                inputs,
                key,
                &entry.children,
                &child_display,
                callback,
                &mut refused,
            );
            continue;
        };
        let display_group = groups.get(&display);
        let count = match assigned_count(plan, display_group.copied()) {
            Ok(count) => count,
            Err(reason) => {
                *refused.entry(reason).or_default() += 1;
                group_sections::emit(
                    &mut field,
                    &mut placed_labels,
                    inputs,
                    key,
                    &entry.children,
                    &child_display,
                    callback,
                    &mut refused,
                );
                continue;
            }
        };
        let controls = entry.controls.len();
        let scale = arrow.global_scale(controls);
        let terrain = |x: f32, z: f32, _: usize| ribbon_height(inputs.heights, x, z);
        let lifted = |_: f32, _: f32, control: usize| {
            entry.lift.map_or(0.0, |lift| {
                lift.at_control(control, controls, arrow.head_length > 0.0)
            })
        };
        let height: &dyn Fn(f32, f32, usize) -> f32 = if entry.lift.is_some() {
            &lifted
        } else {
            &terrain
        };
        let texture_choice = match texture_choice(arrow, count == 0 && !virtual_order) {
            Ok(choice) => choice,
            Err(reason) => {
                *refused.entry(reason).or_default() += 1;
                group_sections::emit(
                    &mut field,
                    &mut placed_labels,
                    inputs,
                    key,
                    &entry.children,
                    &child_display,
                    callback,
                    &mut refused,
                );
                continue;
            }
        };
        let Some(draw_index) = emit(&mut field, arrow, &entry.controls, scale, color, height)
        else {
            field.report.too_short += 1;
            group_sections::emit(
                &mut field,
                &mut placed_labels,
                inputs,
                key,
                &entry.children,
                &child_display,
                callback,
                &mut refused,
            );
            continue;
        };
        let draw = &mut field.draws[draw_index];
        draw.texture_choice = texture_choice;
        draw.culling_offset = culling_offset(arrow.culling_direction, entry.side);
        draw.execution = callback;
        draw.kind = plan_kind(kind);
        field.report.drawn += 1;
        field.report.snapped += usize::from(entry.snapped);
        field.report.offset_fronts += usize::from(entry.offset);
        *field.report.per_kind.entry(kind_name(kind)).or_default() += 1;
        group_sections::emit(
            &mut field,
            &mut placed_labels,
            inputs,
            key,
            &entry.children,
            &child_display,
            callback,
            &mut refused,
        );
        if matches!(kind, Classified::OffensiveLine | Classified::Move) && !entry.snapped {
            for province in &order.path {
                let Some(c) = geometry.centroid(*province) else {
                    continue;
                };
                let gap = entry
                    .controls
                    .iter()
                    .map(|&control| span(fixed::to_world(control), c))
                    .fold(f32::MAX, f32::min);
                let milli = (gap * 1000.0).round().max(0.0) as u32;
                field.report.control_gap_max_milli = field.report.control_gap_max_milli.max(milli);
                gap_total += u64::from(milli);
                gap_count += 1;
            }
        }
        let Some(label_inputs) = inputs.labels.filter(|_| !virtual_order) else {
            continue;
        };
        let group_name = group_plan_rows
            .get(&display)
            .map_or(&[][..], |row| row.name.as_slice());
        let _labelling = stage("plan.labels");
        let composer = labels::Composer {
            localisation: label_inputs.localisation,
            order_type: order.order_type,
            name: labels::OrderName {
                prefix: &plan.name_prefix,
                name: &plan.name,
                first: &plan.name_first,
                second: &plan.name_second,
                unique: &plan.name_unique,
                postfix: &plan.name_postfix,
            },
            group_name,
            army_group: display_group.is_some_and(|group| group.is_army_group != 0),
            count,
        };
        let transport_failed = Cell::new(false);
        let transports = || {
            let result = if order.order_type == TYPE_INVASION {
                transport_counts::invasion(plan)
            } else {
                transport_context.count(order, groups.get(&order.owner_group).copied())
            };
            if result.is_err() {
                transport_failed.set(true);
            }
            result
        };
        let terrain = |x: f32, z: f32, _: f32| inputs.heights.native_height(x, z);
        let lifted = |_: f32, _: f32, t: f32| entry.lift.map_or(0.0, |lift| lift.at(t));
        let height: &dyn Fn(f32, f32, f32) -> f32 = if entry.lift.is_some() {
            &lifted
        } else {
            &terrain
        };
        let placement = labels::place(
            &|variant| {
                if variant == labels::Variant::Full {
                    if count == 0 && display_group.is_none() {
                        return Err("order display group kind unavailable".into());
                    }
                    if (count > 0 || composer.army_group) && !group_plan_rows.contains_key(&display)
                    {
                        return Err("order display group name unavailable".into());
                    }
                }
                composer.drawn_with_transports(
                    variant,
                    controls,
                    defines.ext_text_points_limit,
                    &transports,
                )
            },
            &entry.controls,
            label_inputs.font,
            arrow.text_size,
            arrow.text_offset,
            height,
        );
        if transport_failed.get() {
            field.report.transports_omitted += 1;
        }
        match placement {
            Ok(Some(placed)) => {
                let glyphs = placed.glyphs.iter().map(|glyph| {
                    (
                        glyph.corners,
                        glyph.uv,
                        glyph.page,
                        glyph.color.unwrap_or([1.0; 4]),
                    )
                });
                match WorldName::from_glyphs(
                    format!("{}:{}", key.0, key.1),
                    placed.text,
                    glyphs,
                    color[3],
                ) {
                    Some(name) => {
                        field.draws[draw_index].label = Some(placed_labels.len());
                        placed_labels.push(name);
                        field.report.labels += 1;
                    }
                    None => field.report.labels_unfitted += 1,
                }
            }
            Ok(None) => field.report.labels_unfitted += 1,
            Err(reason) => *field.report.labels_refused.entry(reason).or_default() += 1,
        }
    }
    drop(at);
    let at = stage("plan.finish");
    if let Some(mean) = gap_total.checked_div(gap_count) {
        field.report.control_gap_mean_milli = mean as u32;
    }
    sort_draws(&mut field.draws);
    field.labels = Placements {
        country: placed_labels,
        warnings: Vec::new(),
        generation: viewer_runtime::name_draw::next_generation(),
        near: None,
    };
    field.report.refused = refused;
    drop(at);
    Ok(field)
}

fn refuse(
    refused: &mut BTreeMap<&'static str, usize>,
    counted: &mut BTreeSet<(OrderKey, &'static str)>,
    key: OrderKey,
    reason: &'static str,
) {
    if counted.insert((key, reason)) {
        *refused.entry(reason).or_default() += 1;
    }
}

fn no_arrow(
    report: &mut OrderReport,
    counted: &mut BTreeSet<(OrderKey, &'static str)>,
    key: OrderKey,
) {
    if counted.insert((key, NO_ARROW)) {
        report.no_arrow += 1;
    }
}

const NO_ARROW: &str = "no arrow";

fn texture_choice(arrow: &ArrowType, unassigned: bool) -> Result<usize, &'static str> {
    let choice = usize::from(unassigned);
    if choice >= arrow.masks.len() || choice >= arrow.patterns.len() {
        return Err("order selected texture mask or pattern absent from recorded content");
    }
    Ok(choice)
}

fn assigned_count(
    plan: &ArmyOrderPlanRow,
    group: Option<&OrdersGroupRow>,
) -> Result<usize, &'static str> {
    if plan.virtual_order != 0 || plan.assigned_divisions.is_empty() {
        return Ok(0);
    }
    let group = group.ok_or("order display group HQ inputs unavailable")?;
    let hq = if group.leader != 0 {
        group.hq_provider
    } else {
        0
    };
    Ok(plan
        .assigned_divisions
        .iter()
        .filter(|&&unit| unit != hq)
        .count())
}

fn culling_offset(direction: i32, side: i32) -> f32 {
    f32::from(u8::from(direction != 0 && side != direction))
}

fn plan_kind(kind: Classified) -> dashboard_core::protocol::PlanKind {
    use dashboard_core::protocol::PlanKind;
    match kind {
        Classified::OffensiveLine => PlanKind::Offensive,
        Classified::Move => PlanKind::Movement,
        Classified::Invasion => PlanKind::Invasion,
        Classified::Paradrop => PlanKind::Paradrop,
        Classified::FrontLine | Classified::GarrisonArea | Classified::Unknown => PlanKind::Front,
    }
}

fn stage(name: &'static str) -> replay_frame::cpu::Scope {
    replay_frame::cpu::scope(name, Some("plan"))
}

fn kind_name(kind: Classified) -> &'static str {
    match kind {
        Classified::FrontLine => "front line",
        Classified::OffensiveLine => "offensive line",
        Classified::Move => "offensive arrow",
        Classified::Invasion => "invasion arrow",
        Classified::Paradrop => "paradrop arrow",
        Classified::GarrisonArea => "garrison area",
        Classified::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests;
