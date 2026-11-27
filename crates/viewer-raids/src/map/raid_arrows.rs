
use content_reader_equipment_graphics::equipment_graphics::countries as model_countries;
use std::collections::BTreeMap;
use viewer_unit_reads::aircraft_model;
#[cfg(test)]
use viewer_unit_reads::aircraft_model::designer_entity;
use viewer_gpu::ribbon_content::RaidArrowReport;
use viewer_gpu::ribbon_geometry::smooth_sharp_turns;
use viewer_gpu::unit_pose::{
    Ends,
    UnitPlacement,
    UnitTrack,
    air_height,
    ballistic_height,
    fixed,
    straight_controls,
    unwrap,
};
#[cfg(test)]
use viewer_gpu::unit_pose::{
    elapsed_hours,
    movement_progress,
    next_hour_progress,
    phase_progress,
    remaining_hours,
};

use content_reader_map::{movement::MovementMap, province_graph::{RaidPath, raid_route}};
use content_reader_raids::{
    raids::PathArrowDefines, raids::RaidArrowKind, raids::RaidGraphicsDefines, raids::RaidType,
};
use replay_data::{rows::RaidRow, rows::RaidTargetRow, rows::TaskForceRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use {
    crate::map::raids::RaidAssets, crate::map::raids::TypeToken,
    crate::map::raids::category_admits, crate::map::raids::eligibility::launch_triggers,
    crate::map::raids::script::Eval, crate::map::raids::unit_inputs::*,
    dashboard_text::triggers::World,
};
use viewer_gpu::{province_map::HeightField, ribbon_content::{ArrowType, OrderAssets, OrderField}};

type Point = [i64; 2];

const LAUNCHED: i32 = 4;
const ENDED: i32 = 5;
const PREPARED: i32 = 3;
const CANCELLED: i32 = 5;

const STACK_STANDSTILL: u32 = 0;
const STACK_DOCKED: u32 = 19;

const HANDLE: f32 = 0.25;

pub struct ArrowInputs<'a> {
    pub assets: &'a RaidAssets,
    pub orders: &'a OrderAssets,
    pub heights: &'a HeightField<'a>,
    pub movement: Option<&'a MovementMap>,
    pub anchor: &'a dyn Fn(u32, u32) -> Option<[f32; 3]>,
    pub ours: u32,
    pub declared: &'a dyn Fn(&str) -> bool,
    pub variant_model: &'a dyn Fn(u64, u32) -> Result<Option<String>, String>,
    pub hour_fraction: f32,
}

pub fn build(
    inputs: &ArrowInputs<'_>,
    frame: &DecodedFrame,
    world: &World<'_>,
) -> (OrderField, RaidArrowReport) {
    let mut field = OrderField::default();
    let mut report = RaidArrowReport::default();
    if let Err(reason) = build_into(inputs, frame, world, &mut field, &mut report) {
        report.unavailable = Some(reason);
    }
    (field, report)
}

fn build_into(
    inputs: &ArrowInputs<'_>,
    frame: &DecodedFrame,
    world: &World<'_>,
    field: &mut OrderField,
    report: &mut RaidArrowReport,
) -> Result<(), String> {
    let assets = inputs.assets;
    let complete = |error: std::io::Error| error.to_string();
    let raids = frame.complete::<tables::Raids>().map_err(complete)?;
    let mut ours: Vec<&RaidRow> = raids
        .iter()
        .filter(|raid| raid.parent_slot == inputs.ours && raid.phase != ENDED)
        .collect();
    if ours.is_empty() {
        return Ok(());
    }
    ours.sort_by_key(|raid| raid.ordinal);
    report.raids = ours.len();
    let defines = inputs
        .orders
        .raid_defines
        .as_ref()
        .map_err(|reason| format!("the raid arrow defines cannot be read: {reason}"))?;
    let targets: BTreeMap<(u32, u32), &RaidTargetRow> = frame
        .complete::<tables::RaidTargets>()
        .map_err(complete)?
        .iter()
        .map(|row| ((row.id, row.reference_type), row))
        .collect();
    if let Some(gap) = world.core_gap() {
        return Err(gap.to_string());
    }
    let mut ships: Option<Result<BTreeMap<u64, &TaskForceRow>, String>> = None;
    for raid in ours {
        let Some(target) = targets.get(&(raid.id, raid.reference_type)) else {
            report.refuse("a raid without a recorded target");
            continue;
        };
        let kind = match assets.raid_type(world, target.raid_type) {
            Ok(TypeToken::Type(index)) => &assets.catalog.types[index],
            Ok(TypeToken::Refused(index)) => {
                report.refuse(format!(
                    "raid type {} is refused by the content port",
                    assets.catalog.uncertain[index].name
                ));
                continue;
            }
            Err(reason) => {
                report.refuse(reason);
                continue;
            }
        };
        match category_admits(assets, world, inputs.ours, kind) {
            Eval::True => {}
            Eval::False => {
                report.filtered += 1;
                continue;
            }
            Eval::Unknown(reasons) => {
                for reason in reasons {
                    report.refuse(format!("the raid filter is undecided: {reason}"));
                }
                continue;
            }
        }
        let arrow_kind = kind.arrow;
        let Some(arrow) = inputs.orders.types.get(arrow_kind.arrow_type()) else {
            report.refuse(format!(
                "{} declares no arrowType {}",
                viewer_gpu::ribbon_content::MAPARROWS,
                arrow_kind.arrow_type()
            ));
            continue;
        };
        let (source, source_country) = match source_province(assets, world, raid, &mut ships) {
            Ok(source) => source,
            Err(reason) => {
                report.refuse(reason);
                continue;
            }
        };
        let color = match color(raid.phase, defines) {
            Some(color) => color,
            None => match can_execute(assets, world, raid, kind, target, source, &source_country) {
                Ok(true) => defines.ready_color,
                Ok(false) => defines.preparing_color,
                Err(reason) => {
                    report.refuse(format!(
                        "a prepared raid (phase 3): READY or PREPARING needs {reason}"
                    ));
                    continue;
                }
            },
        };
        let geometry = match controls(inputs, defines, arrow, arrow_kind, source, target, world) {
            Ok(Some(geometry)) => geometry,
            Ok(None) => {
                report.empty += 1;
                continue;
            }
            Err(reason) => {
                report.refuse(reason);
                continue;
            }
        };
        if emit(inputs, defines, field, arrow, arrow_kind, &geometry, color).is_none() {
            report.empty += 1;
            continue;
        }
        report.drawn += 1;
        if unit_shown(raid.phase, raid.outcome, 0.0) {
            let mut held = Vec::new();
            let placed = place_unit(
                inputs, defines, world, raid, kind, arrow_kind, target, &geometry, &mut held,
            );
            field.held.extend(
                held.into_iter()
                    .map(|held| held as std::sync::Arc<dyn std::any::Any + Send + Sync>),
            );
            match placed {
                Ok(unit) if unit.entity.is_some() => field.raid_units.push(unit),
                Ok(_) => {
                    report.units_missing += 1;
                    report.units_reason = Some(
                        "raid unit has no entity: the type names no model and the raid has no wing"
                            .into(),
                    );
                }
                Err(reason) => {
                    report.units_missing += 1;
                    report.units_reason = Some(reason);
                }
            }
        }
    }
    viewer_gpu::ribbon_mesh::sort_draws(&mut field.draws);
    Ok(())
}

pub fn color(phase: i32, defines: &RaidGraphicsDefines) -> Option<[f32; 4]> {
    match phase {
        LAUNCHED | ENDED => Some(defines.active_color),
        PREPARED => None,
        _ => Some(defines.preparing_color),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub controls: Vec<Point>,
}

fn controls(
    inputs: &ArrowInputs<'_>,
    defines: &RaidGraphicsDefines,
    arrow: &ArrowType,
    kind: RaidArrowKind,
    source: u32,
    target: &RaidTargetRow,
    world: &World<'_>,
) -> Result<Option<Geometry>, String> {
    let assets = inputs.assets;
    let width = inputs.heights.width as f32;
    match kind {
        RaidArrowKind::Naval | RaidArrowKind::Land => {
            let (path, spline) = if kind == RaidArrowKind::Naval {
                (RaidPath::Naval, defines.naval)
            } else {
                (RaidPath::Land, defines.land)
            };
            let Some(to) = assets.province(&target.scope) else {
                return Err("a path raid whose target has no province".into());
            };
            let search = assets.paths.as_ref().map_err(Clone::clone)?;
            let Some((_, route)) = raid_route(search.graph(), &assets.geometry, source, to, path)?
            else {
                return Ok(None);
            };
            let Some(movement) = inputs.movement else {
                return Err(
                    "the content movement map (native neighbour order and crossings) could not be derived"
                        .into(),
                );
            };
            let anchor = |province: u32| -> Result<[f32; 3], String> {
                let land = assets
                    .geometry
                    .kind_flags(province)
                    .is_some_and(|flags| flags & 1 != 0);
                let slot = if land { STACK_DOCKED } else { STACK_STANDSTILL };
                (inputs.anchor)(province, slot)
                    .ok_or_else(|| format!("province {province} has no unit-stack anchor {slot}"))
            };
            let points = path_points(&route, width, spline.use_midpoints, &anchor, &|from, to| {
                movement.route_crossing(from, to)
            })?;
            Ok(spline_controls(points, spline, arrow).map(|controls| Geometry { controls }))
        }
        RaidArrowKind::Line | RaidArrowKind::Ballistic | RaidArrowKind::Air => {
            let from = (inputs.anchor)(source, STACK_STANDSTILL)
                .ok_or_else(|| format!("source province {source} has no unit-stack anchor"))?;
            let province = assets
                .anchor_province(world, &target.scope)?
                .ok_or("a raid target without a world position")?;
            let to = (inputs.anchor)(province, STACK_STANDSTILL)
                .ok_or_else(|| format!("target province {province} has no unit-stack anchor"))?;
            let segments = match kind {
                RaidArrowKind::Ballistic => Some((
                    defines.ballistic_max_segment_length,
                    defines.ballistic_max_segments,
                )),
                RaidArrowKind::Air => {
                    Some((defines.air_max_segment_length, defines.air_max_segments))
                }
                _ => None,
            };
            Ok(Some(Geometry {
                controls: straight_controls(from, to, width, segments),
            }))
        }
    }
}

pub fn path_points(
    route: &[u32],
    width: f32,
    midpoints: bool,
    anchor: &dyn Fn(u32) -> Result<[f32; 3], String>,
    crossing: &dyn Fn(u32, u32) -> Option<Option<[i32; 2]>>,
) -> Result<Vec<[f32; 3]>, String> {
    let mut points: Vec<[f32; 3]> = Vec::with_capacity(route.len() * 2 + 3);
    let Some(&first) = route.first() else {
        return Ok(points);
    };
    let push = |points: &mut Vec<[f32; 3]>, point: [f32; 3]| {
        let point = match points.last() {
            Some(last) => [unwrap(point[0], last[0], width), point[1], point[2]],
            None => point,
        };
        points.push(point);
    };
    let last_index = route.len() - 1;
    let mut current = first;
    for (index, &next) in route.iter().enumerate().skip(1) {
        let Some(midpoint) = crossing(current, next) else {
            continue;
        };
        let a = anchor(current)?;
        let b = anchor(next)?;
        let a = [a[0], 0.0, a[2]];
        let b = [unwrap(b[0], a[0], width), 0.0, b[2]];
        if index == 1 {
            push(
                &mut points,
                [
                    (a[0] - b[0]) * HANDLE + a[0],
                    0.0,
                    (a[2] - b[2]) * HANDLE + a[2],
                ],
            );
        }
        push(&mut points, a);
        if midpoints && let Some([x, z]) = midpoint {
            push(&mut points, [x as f32, 0.0, z as f32]);
        }
        if index == last_index {
            push(&mut points, b);
            push(
                &mut points,
                [
                    (b[0] - a[0]) * HANDLE + b[0],
                    0.0,
                    (b[2] - a[2]) * HANDLE + b[2],
                ],
            );
        }
        current = next;
    }
    Ok(points)
}

pub fn spline_controls(
    mut points: Vec<[f32; 3]>,
    spline: PathArrowDefines,
    arrow: &ArrowType,
) -> Option<Vec<Point>> {
    let amount = arrow.body_width[0].max(arrow.body_width[1]) * spline.sharp_turn_smoothness;
    if 0.0 < amount {
        smooth_sharp_turns(&mut points, amount);
    }
    let (&first, &last) = (points.first()?, points.last()?);
    let mut out = vec![fixed(first)];
    let count = points.len();
    if spline.subdivisions > 0 && count >= 4 {
        let n = spline.subdivisions;
        for index in 1..count - 2 {
            let [p0, p1, p2, p3] = [
                points[index - 1],
                points[index],
                points[index + 1],
                points[index + 2],
            ];
            for step in 0..n {
                let s = step as f32 / n as f32;
                let s2 = s * s;
                let s3 = s2 * s;
                let w2 = (s2 * 4.0 - s3 * 3.0) + s;
                let w1 = (s3 * 3.0 - s2 * 5.0) + 2.0;
                let w0 = ((s2 + s2) - s3) - s;
                let w3 = s3 - s2;
                let at = |axis: usize| {
                    (p0[axis] * w0 + p1[axis] * w1 + p2[axis] * w2 + p3[axis] * w3) * 0.5
                };
                out.push(fixed([at(0), 0.0, at(2)]));
            }
        }
    }
    out.push(fixed(last));
    Some(out)
}

fn emit(
    inputs: &ArrowInputs<'_>,
    defines: &RaidGraphicsDefines,
    field: &mut OrderField,
    arrow: &ArrowType,
    kind: RaidArrowKind,
    geometry: &Geometry,
    color: [f32; 4],
) -> Option<()> {
    let controls = &geometry.controls;
    let scale = arrow.global_scale(controls.len());
    let width = i64::from(inputs.heights.width);
    let heights = inputs.heights;
    let ends = Ends::new(controls, width, &|p| {
        heights.fixed_height(p[0], p[1], width)
    });
    let strip_n = controls.len() as i32 + if arrow.head_length > 0.0 { 2 } else { 0 };
    let lift = |i: i32| -> f32 {
        let Some(ends) = ends else { return 0.0 };
        match kind {
            RaidArrowKind::Air => air_height(ends, i, strip_n, defines),
            RaidArrowKind::Ballistic => ballistic_height(ends, i, strip_n, defines),
            _ => 0.0,
        }
    };
    let lifted = |_: f32, _: f32, control: usize| lift(control as i32);
    let terrain = |x: f32, z: f32, _: usize| {
        viewer_gpu::ribbon_mesh::ribbon_height(heights, x, z)
    };
    let height: &dyn Fn(f32, f32, usize) -> f32 = match kind {
        RaidArrowKind::Air | RaidArrowKind::Ballistic => &lifted,
        _ => &terrain,
    };
    let draw =
        viewer_gpu::ribbon_mesh::emit(field, arrow, controls, scale, color, height)?;
    let draw = &mut field.draws[draw];
    draw.culling_offset = f32::from(u8::from(arrow.culling_direction != 0));
    draw.kind = dashboard_core::protocol::PlanKind::Raid;
    draw.secondary_color = [1.0; 4];
    draw.outer_color = [1.0; 4];
    Some(())
}


pub fn unit_shown(phase: i32, outcome: i32, progress: f32) -> bool {
    phase == LAUNCHED
        || (phase == ENDED && progress < f32::from_bits(0x3f7f_be77) && outcome != CANCELLED)
}

#[expect(clippy::too_many_arguments, reason = "the arrow inputs, defines and world, the raid's rows and kinds, the geometry, and the held list")]
fn place_unit(
    inputs: &ArrowInputs<'_>,
    defines: &RaidGraphicsDefines,
    world: &World<'_>,
    raid: &RaidRow,
    kind: &RaidType,
    arrow_kind: RaidArrowKind,
    target: &RaidTargetRow,
    geometry: &Geometry,
    held: &mut Vec<crate::map::raids::paths::Held>,
) -> Result<UnitPlacement, String> {
    let frame = world.frame();
    let (travelled, speed_inputs) = plane_inputs(frame, raid)?;
    let assets = inputs.assets;
    let branch = SpeedBranch::of(kind, &|name| assets.equipment_category(name))?;
    let speed_defines = SpeedDefines::read(defines, &assets.defines)?;
    let speed = raid_unit_speed(
        branch,
        &speed_inputs,
        kind.speed_multiplier_raw,
        &speed_defines,
    );
    let distance = target_distance(inputs.assets, frame, raid, target, held)?;
    let track = UnitTrack::new(
        &geometry.controls,
        arrow_kind,
        inputs.heights,
        defines,
        travelled,
        distance,
        speed,
        kind.unit_model.start_offset_raw,
    )
    .ok_or("a raid arrow with fewer than two controls")?;
    let pose = track
        .place(inputs.heights, inputs.hour_fraction)
        .ok_or("a raid arrow with fewer than two controls")?;
    Ok(UnitPlacement {
        position: pose.position,
        facing: pose.facing,
        scale: kind.unit_model.scale_raw as f32 / 100_000.0 * defines.unit_base_scale,
        progress: pose.progress,
        stages: kind.unit_animations.clone(),
        entity: raid_entity(inputs, world, raid, kind)?,
        seed: raid.id,
        track,
    })
}

fn raid_entity(
    inputs: &ArrowInputs<'_>,
    world: &World<'_>,
    raid: &RaidRow,
    kind: &RaidType,
) -> Result<Option<String>, String> {
    let frame = world.frame();
    let complete = |error: std::io::Error| error.to_string();
    let countries = model_countries::read(frame).map_err(complete)?;
    let country = countries
        .get(&raid.parent_slot)
        .ok_or("the raid owner has no CountryAppearance row")?;
    let culture = country
        .graphical_culture
        .as_deref()
        .ok_or("the raid owner's graphical culture is unreadable")?;
    let slots: BTreeMap<u32, u32> = frame
        .complete::<tables::CountryLinks>()
        .map_err(complete)?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let definitions: BTreeMap<u64, &replay_data::rows::VariantDefinitionRow> = frame
        .complete::<tables::VariantDefinitions>()
        .map_err(complete)?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let mut roster: Vec<(u32, u64)> = frame
        .complete::<tables::AvailableEquipmentVariants>()
        .map_err(complete)?
        .iter()
        .filter(|row| row.roster_kind == 0 && slots.get(&row.country) == Some(&raid.parent_slot))
        .map(|row| (row.ordinal, row.variant))
        .collect();
    roster.sort_unstable();
    let primary = roster
        .iter()
        .map(|&(_, variant)| {
            definitions
                .get(&variant)
                .map(|row| (variant, row.classifier_flags))
                .ok_or_else(|| format!("equipment variant {variant} has no definition"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let wing = (raid.air_wing != 0
        && frame
            .complete::<tables::AirWings>()
            .map_err(complete)?
            .iter()
            .any(|wing| wing.id == raid.air_wing))
    .then_some(raid.air_wing);
    let raw_owner = slots
        .iter()
        .find(|(_, slot)| **slot == raid.parent_slot)
        .map(|(raw, _)| *raw)
        .ok_or("the raid owner has no CountryLinks row")?;
    let designer = |variant: u64| (inputs.variant_model)(variant, raw_owner);
    let wing_model = |wing: u64| -> Result<Option<String>, String> {
        let variant = wing_primary_variant(frame, wing)?
            .ok_or_else(|| format!("the raid's wing {wing} holds no equipment"))?;
        designer(variant)
    };
    match unit_entity(
        &kind.unit_model,
        &country.tag,
        culture,
        &primary,
        wing,
        inputs.declared,
    ) {
        UnitEntity::Entity(name) => Ok(Some(name)),
        UnitEntity::Designer(variant) => match designer(variant)? {
            Some(name) => Ok(Some(name)),
            None => wing.map_or(Ok(None), wing_model),
        },
        UnitEntity::WingEquipment(wing) => wing_model(wing),
        UnitEntity::None => Ok(None),
    }
}

pub fn wing_primary_variant(
    frame: &dyn replay_frame::frame::DecodedSource,
    wing: u64,
) -> Result<Option<u64>, String> {
    aircraft_model::wing_primary(frame, wing)
}


fn can_execute(
    assets: &RaidAssets,
    world: &World<'_>,
    raid: &RaidRow,
    kind: &RaidType,
    target: &RaidTargetRow,
    source: u32,
    source_country: &Result<Option<u32>, String>,
) -> Result<bool, String> {
    let unit = match unit_at_source(world, raid, source) {
        Ok(at) => Eval::from_bool(at),
        Err(reason) => Eval::unknown(reason),
    };
    let result = unit
        .and(|| match raid_pool(world.frame(), raid) {
            Err(reason) => Eval::unknown(reason),
            Ok(pool) => match pool_meets(kind, &pool, &|token| {
                crate::map::raids::spelling(world, token, "equipment archetype").map(str::to_owned)
            }) {
                Ok(met) => Eval::from_bool(met),
                Err(reason) => Eval::unknown(reason),
            },
        })
        .and(|| {
            launch_triggers(
                assets,
                world,
                raid.parent_slot,
                kind,
                &target.scope,
                source_country,
            )
        });
    match result {
        Eval::True => Ok(true),
        Eval::False => Ok(false),
        Eval::Unknown(reasons) => Err(reasons.into_iter().collect::<Vec<_>>().join("; ")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitEntity {
    Entity(String),
    Designer(u64),
    WingEquipment(u64),
    None,
}

pub fn unit_entity(
    model: &content_reader_raids::raids::RaidUnitModel,
    tag: &str,
    culture: &str,
    primary: &[(u64, u64)],
    wing: Option<u64>,
    declared: &dyn Fn(&str) -> bool,
) -> UnitEntity {
    let mask = model.equipment;
    if mask != 0
        && let Some(&(variant, _)) = primary.iter().find(|(_, flags)| flags & mask != 0)
    {
        return UnitEntity::Designer(variant);
    }
    if let Some(entity) = model.entity.as_deref().filter(|name| !name.is_empty()) {
        for name in [
            format!("{tag}_{entity}"),
            format!("{culture}_{entity}"),
            entity.to_owned(),
        ] {
            if declared(&name) {
                return UnitEntity::Entity(name);
            }
        }
    }
    if let Some(entity) = model
        .default_entity
        .as_deref()
        .filter(|name| !name.is_empty())
        && declared(entity)
    {
        return UnitEntity::Entity(entity.to_owned());
    }
    wing.map_or(UnitEntity::None, UnitEntity::WingEquipment)
}

#[cfg(test)]
mod tests;
