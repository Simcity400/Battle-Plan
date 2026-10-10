//! The viewing country's raid arrows and their in-flight units.
//!
//! Keep one arrow per non-ended raid, in instance order, beside the battle
//! plan. An ended raid moves to a second list while its uncancelled unit
//! progress is below 0.999. Its first update there sets progress to 1, so a
//! still frame never shows it.
//!
//! Per raid:
//!
//! - Hide types rejected by the default filter ([`super::category_admits`]).
//! - Use `NMapMode.ACTIVE_RAID_ARROW_COLOR` in phases 4 and 5. Otherwise use
//!   READY when execution checks hold, else PREPARING. Execution is false
//!   outside phase 3. Phase 3 needs a unit at the source, the raid's own pool
//!   meeting essential and other equipment requirements at level 4 (including
//!   nukes), and [`launch_triggers`]. Offers read neither this pool nor
//!   `launchable`/`launchable_from`, so evaluate them here, taking the pool
//!   from `RaidEquipment` ([`raid_pool`]). Refuse undecided phase-3 checks by name.
//! - Build geometry by the type's arrow kind using its `active_raid_*` content
//!   type through the shared battle-plan manager. Buckets, textures, blend and
//!   pass modes are shared ([`super::super::orders`]).
//!
//! # The unit
//!
//! Phase 4 carries the raid unit's entity ([`raid_entity`]), placed from
//! `RaidRow.travelled` and [`raid_unit_speed`] over `RaidSpeedInputs`.
//!
//! The recording holds each hour's state after its tick. A paused frame at
//! hour H is the start of H with tick progress 0. Samples do not retain
//! sub-hour progress; playback fraction supplies it. Hour builders place at
//! fraction 0 (dumps use `--hour-fraction`) and keep [`UnitTrack`]. Playing
//! windows place again each rendered frame, so units glide and cross stages
//! within the hour. Arrow geometry and colour stay fixed. An ended unit
//! reaches progress 1 on its first phase-5 update, beyond the visibility
//! threshold 0.999, and can show for at most one rendered frame at the target.
//!
//! Create the entity in `idle`; each update selects the type's `unit_animations`
//! stage through [`animation_state`]. Entity clips use wall-clock delta with
//! console scale 1.0, including while paused. State speed advances clip phase,
//! wrapping at clip length; an ended clip enters `next_state`. The viewer uses
//! its own wall clock ([`super::super::entity_animation::named_stage`]). A
//! stage reached under a shown unit plays `intro`, then its `next_state`. A
//! freshly shown mid-stage unit (first placement, seek or dump) shows the stage
//! state. Keep each raid entity's history across hours by raid ID
//! ([`UnitPlacement::seed`]).
//!
//! Camera height does not hide the entity: phase rules and arrow reset own
//! visibility. `NGraphics.DRAW_MAP_OBJECTS_CUTOFF` applies to province objects.
//! The raid entity draws at every height with its mesh's own LOD distances.

use std::collections::BTreeMap;

use installed_content::{
    movement::MovementMap,
    province_geometry::native_fixed_sqrt,
    province_graph::{RaidPath, raid_route},
    raids::{PathArrowDefines, RaidArrowKind, RaidGraphicsDefines, RaidType},
};
use replay::{
    frame::DecodedFrame,
    rows::{RaidRow, RaidTargetRow, TaskForceRow, tables},
};

use super::{
    RaidAssets, TypeToken, category_admits,
    script::{Eval, Scope, Scopes, World},
};
use crate::map::{
    borders::HeightField,
    orders::{self, ArrowType, OrderAssets, OrderField},
};

/// One fixed5 world unit.
const SCALE: i64 = 100_000;
/// A fixed5 `(x, z)` point for an arrow control.
type Point = [i64; 2];

/// Raid phase 4 is launched; phase 5 is ended.
const LAUNCHED: i32 = 4;
const ENDED: i32 = 5;
/// Execution is true only in phase 3.
const PREPARED: i32 = 3;
/// Outcome 5 is cancelled.
const CANCELLED: i32 = 5;

/// Recorded source kinds (`engine::craid_source`).
const SOURCE_NAVAL_BASE: u32 = engine::craid_source::KIND_NAVAL_BASE;
const SOURCE_CARRIER: u32 = engine::craid_source::KIND_CARRIER;
const SOURCE_SUBMARINE: u32 = engine::craid_source::KIND_SUBMARINE;
const SOURCE_BUILDING: u32 = engine::craid_source::KIND_BUILDING;

/// Unit-stack anchor slot: 19 (0x13) on land, else 0.
const STACK_STANDSTILL: u32 = 0;
const STACK_DOCKED: u32 = 19;

/// The phantom handle is a quarter of the end segment.
const HANDLE: f32 = 0.25;

/// Why a phase-4 arrow carries no unit in a recording that predates its inputs.
pub const PLANE_UNRECORDED: &str =
    "the raid's travelled distance (RaidRow.travelled) is not in this recording";

/// The null building-type token used by a source without a building.
const NO_BUILDING_TYPE: u32 = 0x4c17;

/// What [`build`] needs beyond the frame.
pub struct ArrowInputs<'a> {
    pub assets: &'a RaidAssets,
    pub orders: &'a OrderAssets,
    pub heights: &'a HeightField<'a>,
    /// The content movement map: native neighbour order and crossings.
    pub movement: Option<&'a MovementMap>,
    /// The province unit-stack slot for `kind`.
    pub anchor: &'a dyn Fn(u32, u32) -> Option<[f32; 3]>,
    /// The viewing country's canonical slot.
    pub ours: u32,
    /// Whether the content's `gfx` descriptors declare an entity
    /// (`UnitModelAssets`, the land-model registry).
    pub declared: &'a dyn Fn(&str) -> bool,
    /// Complete airborne selector (variant, raw country), shared with
    /// conventional flights through `UnitModelAssets::aircraft_entity`.
    pub variant_model: &'a dyn Fn(u64, u32) -> Result<Option<String>, String>,
    /// The frame's hour fraction: 0 from the window's hour builders, a
    /// dump's `--hour-fraction`. It stands in for the tick's progress
    /// Tick fraction is unrecorded: hourly samples have 1.0, then the hour tick
    /// resets it to zero. Playback uses its own fraction ([`UnitTrack::place`]).
    pub hour_fraction: f32,
}

/// What one hour's raid arrows came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaidArrowReport {
    /// The viewing country's non-ended raids, one arrow each natively.
    pub raids: usize,
    pub drawn: usize,
    /// Arrows the default raid filter hides.
    pub filtered: usize,
    /// Arrows the game draws nothing for: no path, or no controls.
    pub empty: usize,
    /// Launched arrows drawn without their unit, and why.
    pub units_missing: usize,
    pub units_reason: Option<String>,
    /// What was refused rather than guessed, and how often.
    pub refused: BTreeMap<String, usize>,
    /// Why no arrow could be decided at all.
    pub unavailable: Option<String>,
}

impl RaidArrowReport {
    fn refuse(&mut self, reason: impl Into<String>) {
        *self.refused.entry(reason.into()).or_default() += 1;
    }

    /// The warning-area lines; empty when every arrow and unit was drawn.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(why) = &self.unavailable {
            out.push(format!("Raid arrows are not drawn: {why}."));
        }
        for (what, count) in &self.refused {
            out.push(format!("Raid arrows: {count} refused, {what}."));
        }
        if self.units_missing > 0 {
            out.push(format!(
                "Raid arrows: {} launched raids are drawn without their unit: {}.",
                self.units_missing,
                self.units_reason.as_deref().unwrap_or(PLANE_UNRECORDED)
            ));
        }
        out
    }
}

/// The hour's raid arrows, in an [`OrderField`] of their own for
/// [`orders::append`], and the report.
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
    // Fill the cache in instance order.
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
        return Err(gap);
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
                orders::MAPARROWS,
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
        // Show the unit in phase 4. Ended raids carrying a unit never appear in a
        // still frame; every drawn raid is before phase 5, so its update progress
        // does not affect this decision.
        if unit_shown(raid.phase, raid.outcome, 0.0) {
            match place_unit(
                inputs, defines, world, raid, kind, arrow_kind, target, &geometry,
            ) {
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
    orders::sort_draws(&mut field.draws);
    Ok(())
}

/// Raid colour; `None` for phase 3, whose colour depends on [`can_execute`].
pub fn color(phase: i32, defines: &RaidGraphicsDefines) -> Option<[f32; 4]> {
    match phase {
        LAUNCHED | ENDED => Some(defines.active_color),
        PREPARED => None,
        // Execution without text returns false outside phase 3.
        _ => Some(defines.preparing_color),
    }
}

/// Source province and country. Kinds 0 to 2 retain their province; building
/// sources (kind 5) use the building's province. Their country is that province
/// controller. Carrier and submarine sources use the ship's task-force
/// location and roster owner.
pub(super) fn source_province<'a>(
    assets: &RaidAssets,
    world: &World<'a>,
    raid: &RaidRow,
    ships: &mut Option<Result<BTreeMap<u64, &'a TaskForceRow>, String>>,
) -> Result<(u32, Result<Option<u32>, String>), String> {
    let Some(kind) = raid.source.kind.filter(|_| raid.source.kind_present != 0) else {
        return Err("a raid without a source".into());
    };
    let (province, country) = match kind {
        SOURCE_CARRIER | SOURCE_SUBMARINE => {
            let ships = ships.get_or_insert_with(|| {
                let frame = world.frame();
                let forces: BTreeMap<u64, &TaskForceRow> = frame
                    .complete::<tables::TaskForces>()
                    .map_err(|error| error.to_string())?
                    .iter()
                    .map(|row| (row.id, row))
                    .collect();
                Ok(frame
                    .complete::<tables::Ships>()
                    .map_err(|error| error.to_string())?
                    .iter()
                    .filter_map(|ship| Some((ship.id, *forces.get(&ship.task_force)?)))
                    .collect())
            });
            let ships = ships.as_ref().map_err(Clone::clone)?;
            let ship = (u64::from(raid.source.ship_reference_id) << 32)
                | u64::from(raid.source.ship_reference_type);
            let force = ships
                .get(&ship)
                .ok_or("a ship source whose ship or task force is not recorded")?;
            (force.province, Some(force.roster_owner))
        }
        SOURCE_BUILDING => {
            let (building_type, location, country) =
                building_source(raid).map_err(str::to_owned)?;
            let province = building_province(assets, world, building_type, location, country)?
                .ok_or("a building source whose building has no province")?;
            (province, None)
        }
        _ => (raid.source.province, None),
    };
    if province == 0 {
        return Err("a raid source without a province".into());
    }
    let raw = match country {
        Some(raw) => Ok(raw),
        None => world
            .province_row(province)
            .map(|(_, controller)| controller)
            .ok_or_else(|| format!("the raid source province {province} is not recorded")),
    };
    let country = raw.and_then(|raw| {
        if raw == 0 {
            return Ok(None);
        }
        world
            .slot(raw)
            .map(Some)
            .ok_or_else(|| format!("the raid source country {raw} has no CountryLinks row"))
    });
    Ok((province, country))
}

/// The building source: its type token, location (a province or state id by
/// type), and country. A null type token or a location of -1 means no building.
pub fn building_source(raid: &RaidRow) -> Result<(u32, i32, i32), &'static str> {
    let source = &raid.source;
    if source.building_type == NO_BUILDING_TYPE || source.building_location == -1 {
        return Err("a building source without a building");
    }
    Ok((
        source.building_type,
        source.building_location,
        source.building_country,
    ))
}

/// A provincial building source (`level_cap.province_max` positive) stands
/// on its location. A state building uses the first province in its state
/// with a site for its type.
pub fn building_province(
    assets: &RaidAssets,
    world: &World<'_>,
    building_type: u32,
    location: i32,
    country: i32,
) -> Result<Option<u32>, String> {
    let kind = assets.building(world, building_type)?;
    let Ok(location) = u32::try_from(location) else {
        return Ok(None);
    };
    if kind.province_max.is_some_and(|cap| cap > 0) {
        return Ok(Some(location).filter(|&id| id != 0));
    }
    assets.anchor_province(
        world,
        &replay::rows::RaidScope {
            building_type,
            building_location: location,
            building_flags: 2,
            building_country: country,
            ..Default::default()
        },
    )
}

/// An arrow's stored controls and how its strip is lifted.
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub controls: Vec<Point>,
}

/// Arrow controls: `Ok(None)` when no geometry is built.
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
            // Use the source province's slot-zero anchor.
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

/// Straight arrow: wrap both ends within half a map width, then split into
/// equal segments. A line has one segment; other kinds use the ceiling of
/// 3D length over maximum segment length, clamped to 1..maximum count.
/// Use the maximum count alone when the length is not positive.
pub fn straight_controls(
    mut from: [f32; 3],
    mut to: [f32; 3],
    width: f32,
    segments: Option<(f32, i32)>,
) -> Vec<Point> {
    if width * 0.5 < (to[0] - from[0]).abs() {
        if to[0] <= from[0] {
            to[0] += width;
        } else {
            from[0] += width;
        }
    }
    let mut count = 1;
    if let Some((length, maximum)) = segments {
        count = maximum;
        let distance = ((to[1] - from[1]) * (to[1] - from[1])
            + (to[0] - from[0]) * (to[0] - from[0])
            + (to[2] - from[2]) * (to[2] - from[2]))
            .sqrt();
        if 0.0 < length {
            // Take the ceiling before converting to an integer.
            let wanted = (distance / length).ceil() as i32;
            count = if wanted <= maximum {
                wanted.max(1)
            } else {
                maximum
            };
        }
    }
    let mut out = vec![fixed(from)];
    for step in 1..count.max(1) {
        let t = step as f32 / count as f32;
        out.push(fixed([
            (to[0] - from[0]) * t + from[0],
            (to[1] - from[1]) * t + from[1],
            (to[2] - from[2]) * t + from[2],
        ]));
    }
    out.push(fixed(to));
    out
}

/// Convert a float point's x and z to fixed5, rounding half away from zero in f32.
fn fixed(point: [f32; 3]) -> Point {
    let round = |value: f32| ((f32::from(u8::from(0.0 <= value)) + value * 100_000.0) - 0.5) as i64;
    [round(point[0]), round(point[2])]
}

/// Bring `x` within half a map width of `reference` by repeated wrapping.
fn unwrap(x: f32, reference: f32, width: f32) -> f32 {
    let mut x = x;
    while width * 0.5 < (reference - x).abs() {
        x += if reference - x < 0.0 { -width } else { width };
    }
    x
}

/// Start at the route's first province. Each next province that is a content
/// neighbour of the last kept one appends the kept anchor and, when enabled
/// and admitted, the neighbour's crossing midpoint. Precede the first hop and
/// follow the last anchor with phantom handles, each a quarter of its end
/// segment. Wrap each point within half a map width of its predecessor; y is zero.
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

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(v: [f32; 3]) -> f32 {
    (v[1] * v[1] + v[0] * v[0] + v[2] * v[2]).sqrt()
}

/// Leave a zero-length vector unchanged.
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l != 0.0 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        v
    }
}

/// The side the native rotation builds from a direction: `(-z, y, x)`.
fn side(v: [f32; 3]) -> [f32; 3] {
    [-v[2], v[1], v[0]]
}

/// At turns sharper than 60 degrees, insert two points around the corner,
/// each a quarter of its segment in and pushed three `amount` sideways away
/// from the turn. Push later revisits sideways at both visits; a revisit at
/// the second-to-last point alone moves, twice as far.
pub fn smooth_sharp_turns(points: &mut Vec<[f32; 3]>, amount: f32) {
    let mut index = 2usize;
    while (index as i64) < points.len() as i64 - 1 {
        let (previous, current, next) = (points[index - 1], points[index], points[index + 1]);
        let incoming = sub(previous, current);
        let half_in = length(incoming) * 0.5;
        let a = normalize(incoming);
        let outgoing = sub(next, current);
        let half_out = length(outgoing) * 0.5;
        let b = normalize(outgoing);
        if b[0] * a[0] + b[1] * a[1] + b[2] * a[2] <= 0.5 {
            let count = points.len();
            let mut later = index + 1;
            while later < count - 1 {
                if points[later] == current {
                    let push = side(normalize(outgoing));
                    let last = later == count - 2;
                    let reach = if last { amount + amount } else { amount };
                    for (value, push) in points[index].iter_mut().zip(push) {
                        *value += push * reach;
                    }
                    if !last {
                        let away = side(normalize(sub(points[later + 1], current)));
                        for (value, away) in points[later].iter_mut().zip(away) {
                            *value += away * amount;
                        }
                    }
                    break;
                }
                later += 1;
            }
        } else {
            let turned = 0.0 < (b[0] * a[2] - b[2] * a[0]);
            let inset = |direction: [f32; 3], half: f32, towards: [f32; 3], out: bool| {
                let corner = [
                    direction[0] * half * 0.5 + current[0],
                    direction[1] * half * 0.5 + current[1],
                    direction[2] * half * 0.5 + current[2],
                ];
                let mut along = if out {
                    sub(towards, corner)
                } else {
                    sub(corner, towards)
                };
                if turned {
                    along = [-along[0], -along[1], -along[2]];
                }
                let push = side(normalize(along));
                [
                    push[0] * amount * 3.0 + corner[0],
                    push[1] * amount * 3.0 + corner[1],
                    push[2] * amount * 3.0 + corner[2],
                ]
            };
            let before = inset(a, half_in, previous, false);
            let after = inset(b, half_out, next, true);
            points.insert(index, before);
            index += 2;
            points.insert(index, after);
        }
        index += 1;
    }
}

/// Path spline: apply sharp-turn smoothing at type width times
/// `*_SHARP_TURN_SMOOTHNESS` when positive, then emit the first point,
/// `*_SUBDIVISIONS` uniform Catmull-Rom samples per inner segment (none when
/// nonpositive), and the last point, all converted to fixed5. No kept route
/// point returns `None`; attempting to read it would precede the array.
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

/// Clamped interpolation, exact at the ends and monotonic between values of one sign.
pub fn clamped_lerp(a: f32, b: f32, t: f32) -> f32 {
    if !(a.is_finite() && b.is_finite() && t.is_finite()) {
        return a + t * (b - a);
    }
    if (0.0 < a || b < 0.0) && (a < 0.0 || 0.0 < b) {
        if t == 1.0 {
            return b;
        }
        let value = t.mul_add(b - a, a);
        return if (1.0 < t) != (a < b) {
            value.min(b)
        } else {
            value.max(b)
        };
    }
    (1.0 - t) * a + b * t
}

/// Air-arrow rise over `length`: a circle's arc eased by steepness.
pub fn air_slope(x: f32, low: f32, high: f32, steepness: f32, length: f32) -> f32 {
    if length == 0.0 {
        return high;
    }
    let clamped = if x <= length {
        if 0.0 <= x { x } else { 0.0 }
    } else {
        length
    };
    let mut t = clamped / length;
    let u = (1.0 - t) * steepness;
    let base = (1.0 - steepness * steepness).sqrt();
    let denominator = 1.0 - base;
    if denominator != 0.0 {
        t = ((1.0 - u * u).sqrt() - base) / denominator;
    }
    clamped_lerp(low, high, t)
}

/// Shared lift operands: wrapped distance between the rounded first and
/// last controls using `native_fixed_sqrt`, and terrain height at each end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ends {
    pub distance: f32,
    pub first: f32,
    pub last: f32,
}

impl Ends {
    pub fn new(controls: &[Point], width: i64, height: &dyn Fn(Point) -> f32) -> Option<Self> {
        let (&first, &last) = (controls.first()?, controls.last()?);
        let round = |value: i64| {
            if value < 0 {
                ((value - 50_000) / SCALE) as i32
            } else {
                ((value + 50_000) / SCALE) as i32
            }
        };
        let dx = round(first[0]).wrapping_sub(round(last[0]));
        let dz = round(first[1]).wrapping_sub(round(last[1]));
        let width = width as i32;
        let dx = if dx.wrapping_abs() <= width / 2 {
            dx
        } else {
            width.wrapping_sub(dx.wrapping_abs())
        };
        let squared = dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz));
        Some(Self {
            distance: native_fixed_sqrt(squared as u32) as f32 / 100_000.0,
            first: height(first),
            last: height(last),
        })
    }
}

/// Air-arrow height at control `i` of `n`.
pub fn air_height(ends: Ends, i: i32, n: i32, defines: &RaidGraphicsDefines) -> f32 {
    if n == 0 {
        return 0.0;
    }
    let d = ends.distance;
    let reach = d / (defines.air_target_length + defines.air_source_length);
    // Clamp at 1 while preserving reach for an unordered comparison.
    let scale = if 1.0 < reach { 1.0 } else { reach };
    let top = scale * defines.air_height
        + if ends.first < ends.last {
            ends.last
        } else {
            ends.first
        };
    let x = (i as f32 / n as f32) * d;
    let towards = air_slope(
        d - x,
        ends.last,
        top,
        defines.air_target_steepness,
        scale * defines.air_target_length,
    );
    let from = air_slope(
        x,
        ends.first,
        top,
        defines.air_source_steepness,
        scale * defines.air_source_length,
    );
    if towards < from { towards } else { from }
}

/// Ballistic-arrow parabola shaped by `-RAID_ARROW_BALLISTIC_SHAPE`; if its
/// apex would pass `RAID_ARROW_BALLISTIC_MAX_HEIGHT`, refit to reach that height.
pub fn ballistic_height(ends: Ends, i: i32, n: i32, defines: &RaidGraphicsDefines) -> f32 {
    let d = ends.distance;
    if n == 0 || d == 0.0 {
        return 0.0;
    }
    let mut a = -defines.ballistic_shape;
    if a == 0.0 {
        return 0.0;
    }
    let (h0, h1) = (ends.first, ends.last);
    let rise = h1 - h0;
    let mut b = rise / d - a * d;
    let maximum = defines.ballistic_max_height;
    if maximum < -(b * b) / (a * 4.0) + h0 {
        let climb = if rise <= 0.0 { 0.0 } else { rise };
        let mut apex = climb + 0.01;
        if climb + 0.01 <= maximum - h0 {
            apex = maximum - h0;
        }
        if apex == 0.0 {
            return 0.0;
        }
        let k = d * d * apex;
        let root = ((apex - rise) * k).sqrt() + apex * d;
        a = -(root * root) / (k * d * d);
        b = rise / d - a * d;
    }
    let x = (i as f32 / n as f32) * d;
    x * a * x + x * b + h0
}

/// Emits one arrow into `field` with the plan's strip builder.
fn emit(
    inputs: &ArrowInputs<'_>,
    defines: &RaidGraphicsDefines,
    field: &mut OrderField,
    arrow: &ArrowType,
    kind: RaidArrowKind,
    geometry: &Geometry,
    color: [f32; 4],
) -> Option<()> {
    // Store fixed controls in the shared map arrow and build through the
    // battle-plan path, including its smoothing. Its emitter takes the resulting
    // fixed controls as they are.
    let controls = &geometry.controls;
    let scale = arrow.global_scale(controls.len());
    let width = i64::from(inputs.heights.width);
    let heights = inputs.heights;
    let ends = Ends::new(controls, width, &|p| {
        heights.fixed_height(p[0], p[1], width)
    });
    // Pass each stored control's index and the control count, plus two with a
    // head (see the battle-plan lift). Head geometry follows `head_length > 0`,
    // independently of technique.
    let strip_n = controls.len() as i32 + if arrow.head_length > 0.0 { 2 } else { 0 };
    let lift = |i: i32| -> f32 {
        let Some(ends) = ends else { return 0.0 };
        match kind {
            RaidArrowKind::Air => air_height(ends, i, strip_n, defines),
            RaidArrowKind::Ballistic => ballistic_height(ends, i, strip_n, defines),
            _ => 0.0,
        }
    };
    // Ballistic and air arrows set their own height callback. Other kinds
    // retain the shared strip builder's terrain callback.
    let lifted = |_: f32, _: f32, control: usize| lift(control as i32);
    let terrain = |x: f32, z: f32, _: usize| orders::ribbon_height(heights, x, z);
    let height: &dyn Fn(f32, f32, usize) -> f32 = match kind {
        RaidArrowKind::Air | RaidArrowKind::Ballistic => &lifted,
        _ => &terrain,
    };
    let draw = orders::emit(field, arrow, controls, scale, color, height)?;
    let draw = &mut field.draws[draw];
    // The raid arrow's teeth side stays at its initial zero.
    draw.culling_offset = f32::from(u8::from(arrow.culling_direction != 0));
    // Create all three arrow colours as white, then update only the primary colour.
    draw.secondary_color = [1.0; 4];
    draw.outer_color = [1.0; 4];
    Some(())
}

// ------------------------------------------------------------------ the unit

/// `RaidSpeedInputs` stores speed operands for one live raid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpeedInputs {
    /// The recorded maximum speed when the raid's wing resolves.
    pub wing_max_speed: Option<i64>,
    /// The recorded maximum-speed stat when the raid's army resolves.
    pub army_max_speed: Option<i64>,
    /// Stat `0x3E` of each transport variant (category bit 20) with a nonzero
    /// amount, in pool order: the raid's own pool, else the country's.
    pub transport_speeds: Vec<i64>,
    pub transport_from_country: u8,
    /// Set when neither pool held a transport: the native asserts and uses
    /// 100000 without the land scale.
    pub transport_fallback: u8,
    /// The resolved wing ace's alive byte, its own and type
    /// `air_maximum_speed_factor`, its wing's plane count, its owner's
    /// `ace_effectiveness_factor`, and the wing country's
    /// `air_ace_bonuses_factor` (the country terms are in `CountryCommandPower`).
    pub ace_alive: Option<u8>,
    pub ace_speed: i64,
    pub ace_type_speed: i64,
    /// The ace's wing plane count; `None` when the wing does not resolve,
    /// in which case size scale is 100000.
    pub ace_wing_planes: Option<i32>,
    pub ace_effectiveness: i64,
    pub ace_bonuses: i64,
}

/// The recorded inputs of one raid's unit: the travelled distance
/// (`RaidRow.travelled`, fixed5) and speed operands (`RaidSpeedInputs`,
/// keyed as `Raids`, with a living ace's country terms from
/// `CountryCommandPower`). A table the recorder withheld is refused with its
/// reason, including a refused fixed-key contract for raid unit speed key 2
/// or ace country keys 0x72 and 0x25A.
pub fn plane_inputs(frame: &DecodedFrame, raid: &RaidRow) -> Result<(i64, SpeedInputs), String> {
    let travelled = raid.travelled.ok_or(PLANE_UNRECORDED)?;
    let rows = frame
        .complete::<tables::RaidSpeedInputs>()
        .map_err(|error| format!("the raid unit's speed inputs are withheld: {error}"))?;
    let row = rows
        .iter()
        .find(|row| row.id == raid.id && row.reference_type == raid.reference_type)
        .ok_or("the raid has no RaidSpeedInputs row")?;
    Ok((travelled, SpeedInputs::from_row(row, frame)?))
}

impl SpeedInputs {
    /// The recorded row: an ace's terms only when it is present and alive.
    /// Its two country terms are `CountryCommandPower`'s at the same frame,
    /// joined by the row's `ace_owner` and `wing_country` canonical slots
    /// (recorded only for a living ace).
    pub fn from_row(
        row: &replay::rows::RaidSpeedInputRow,
        frame: &DecodedFrame,
    ) -> Result<Self, String> {
        let country = |slot: Option<u32>,
                       name: &str,
                       value: fn(&replay::rows::CountryCommandPowerRow) -> Option<i64>|
         -> Result<i64, String> {
            let Some(slot) = slot else {
                return Ok(0);
            };
            let rows = frame
                .complete::<tables::CountryCommandPower>()
                .map_err(|error| {
                    format!("the raid ace's country modifiers are withheld: {error}")
                })?;
            let row = rows
                .binary_search_by_key(&slot, |row| row.canonical_slot)
                .map(|at| &rows[at])
                .map_err(|_| format!("country slot {slot} has no CountryCommandPower row"))?;
            value(row).ok_or_else(|| format!("{name} is not a registered modifier"))
        };
        Ok(Self {
            wing_max_speed: row.wing_max_speed,
            army_max_speed: row.army_max_speed,
            transport_speeds: row.transport_speeds.clone(),
            transport_from_country: row.transport_from_country,
            transport_fallback: row.transport_fallback,
            ace_alive: (row.ace_present != 0).then_some(row.ace_alive),
            ace_speed: row.ace_speed,
            ace_type_speed: row.ace_type_speed,
            ace_wing_planes: row.ace_wing_planes,
            ace_effectiveness: country(row.ace_owner, "ace_effectiveness_factor", |row| {
                row.ace_effectiveness_factor
            })?,
            ace_bonuses: country(row.wing_country, "air_ace_bonuses_factor", |row| {
                row.air_ace_bonuses_factor
            })?,
        })
    }
}

/// The content and `NDefines` operands of [`raid_unit_speed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeedDefines {
    /// `NMilitary.LAND_SPEED_MODIFIER`.
    pub land_speed_modifier: i64,
    /// `NNavy.NAVAL_SPEED_MODIFIER`.
    pub naval_speed_modifier: i64,
    /// `NNavy.NAVAL_TRANSFER_BASE_SPEED`.
    pub naval_transfer_base_speed: i64,
    /// `NRaids.RAID_UNIT_SPEED_MULTIPLIER`.
    pub raid_unit_speed_multiplier: i64,
    /// `NAir.ACE_WING_SIZE` and `ACE_WING_SIZE_MAX_BONUS`.
    pub ace_wing_size: i64,
    pub ace_wing_size_max_bonus: i64,
}

/// Select the speed branch from essential equipment categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedBranch {
    /// Transport-plane category 0x100000.
    TransportPlanes,
    /// Convoy category bit 0.
    Convoys,
    /// The assigned wing, else army.
    Unit,
}

impl SpeedBranch {
    /// `category` is an essential type's category mask, or why it is
    /// unknown.
    pub fn of(
        kind: &RaidType,
        category: &dyn Fn(&str) -> Result<u64, String>,
    ) -> Result<Self, String> {
        let has = |bit: u64| -> Result<bool, String> {
            let mut found = false;
            for (name, _) in &kind.essential_equipment {
                found |= category(name)? & bit != 0;
            }
            Ok(found)
        };
        Ok(if has(0x10_0000)? {
            Self::TransportPlanes
        } else if has(1)? {
            Self::Convoys
        } else {
            Self::Unit
        })
    }
}

/// `x * y / 100000` with the wrapping product and truncating division.
fn times(x: i64, y: i64) -> i64 {
    x.wrapping_mul(y) / SCALE
}

/// The ace bonus shrinks with its wing's size.
pub fn ace_wing_scale(planes: i64, wing_size: i64, max_bonus: i64) -> i64 {
    if planes == 0 {
        return 100_000;
    }
    let finite = (wing_size as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd;
    if finite {
        (wing_size.wrapping_mul(100_000) / planes.wrapping_mul(100_000)).min(max_bonus)
    } else {
        max_bonus.min(0xffff_ffff)
    }
}

/// Raid-unit speed in fixed5 map units per hour.
///
/// - Transport planes: the best transport speed stat (0x3E, first on ties)
///   times `LAND_SPEED_MODIFIER`; none is 100000 unscaled.
/// - Convoys: naval speed modifier times naval transfer base speed.
/// - A resolved wing: its stored speed times one plus the ace's scaled
///   `air_maximum_speed_factor` (key 2), plus the wing country's
///   `air_ace_bonuses_factor` when the ace type's key 2 is nonzero, then times
///   `LAND_SPEED_MODIFIER`.
/// - Otherwise a resolved army: its speed stat times the same land modifier.
/// - Otherwise zero.
///
/// Then times `RAID_UNIT_SPEED_MULTIPLIER` and the type's `speed_multiplier`.
pub fn raid_unit_speed(
    branch: SpeedBranch,
    inputs: &SpeedInputs,
    speed_multiplier: i64,
    defines: &SpeedDefines,
) -> i64 {
    let unit = match branch {
        SpeedBranch::TransportPlanes if inputs.transport_fallback != 0 => 100_000,
        SpeedBranch::TransportPlanes => {
            let mut best: Option<i64> = None;
            for &speed in &inputs.transport_speeds {
                if best.is_none_or(|best| best < speed) {
                    best = Some(speed);
                }
            }
            match best {
                Some(best) => times(best, defines.land_speed_modifier),
                None => 100_000,
            }
        }
        SpeedBranch::Convoys => times(
            defines.naval_speed_modifier,
            defines.naval_transfer_base_speed,
        ),
        SpeedBranch::Unit => {
            if let Some(wing) = inputs.wing_max_speed {
                let mut factor = 100_000_i64;
                if inputs.ace_alive.is_some_and(|alive| alive != 0) {
                    let scale = inputs.ace_wing_planes.map_or(100_000, |planes| {
                        ace_wing_scale(
                            i64::from(planes),
                            defines.ace_wing_size,
                            defines.ace_wing_size_max_bonus,
                        )
                    });
                    let scaled = times(inputs.ace_speed, scale);
                    let scaled = times(inputs.ace_effectiveness.wrapping_add(100_000), scaled);
                    factor = scaled.wrapping_add(100_000);
                    if inputs.ace_type_speed != 0 {
                        factor = factor.wrapping_add(inputs.ace_bonuses);
                    }
                }
                times(times(factor, wing), defines.land_speed_modifier)
            } else if let Some(army) = inputs.army_max_speed {
                times(army, defines.land_speed_modifier)
            } else {
                0
            }
        }
    };
    times(
        times(unit, defines.raid_unit_speed_multiplier),
        speed_multiplier,
    )
}

/// Checks the signed-64 validity range, excluding the minimum,
/// minimum plus one, and maximum values.
fn finite(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

/// Phase-4 elapsed hours: travelled distance divided by speed, floored.
/// Zero speed or unbounded distance gives the 0xFFFFFFFF sentinel's floor, 42949.
pub fn elapsed_hours(travelled: i64, speed: i64) -> i32 {
    let mut value = if speed == 0 || !finite(travelled) {
        0xffff_ffff
    } else {
        let value = travelled.wrapping_mul(SCALE) / speed;
        if value == value / SCALE * SCALE {
            return (value / SCALE) as i32;
        }
        value
    };
    value = (value / SCALE + (value >> 63)) * SCALE;
    (value / SCALE) as i32
}

/// Phase-4 remaining hours: remaining distance divided by speed, rounded up.
pub fn remaining_hours(distance: i64, travelled: i64, speed: i64) -> i32 {
    let rest = distance.wrapping_sub(travelled);
    let mut value = if speed == 0 || !finite(rest) {
        0xffff_ffff
    } else {
        let value = rest.wrapping_mul(SCALE) / speed;
        if value == value / SCALE * SCALE {
            return (value / SCALE) as i32;
        }
        value
    };
    value = (value / SCALE + i64::from(0 < value)) * SCALE;
    (value / SCALE) as i32
}

/// Phase progress in fixed5.
pub fn phase_progress(elapsed: i32, remaining: i32) -> i64 {
    if remaining.wrapping_add(elapsed) < 1 {
        return 0;
    }
    let total = i64::from(remaining.wrapping_add(elapsed)) * SCALE;
    if total != 0 && -0x7fff_ffff_ffff_ffff < i64::from(elapsed) * SCALE {
        i64::from(elapsed) * 10_000_000_000 / total
    } else {
        0xffff_ffff
    }
}

/// Next-hour phase progress in fixed5.
pub fn next_hour_progress(elapsed: i32, remaining: i32) -> i64 {
    let total = i64::from(elapsed.wrapping_add(remaining)) * SCALE;
    let mut next = (i64::from(elapsed) + 1) * SCALE;
    if total == 0 {
        return 0;
    }
    if total < next {
        next = total;
    }
    if next < -0x7fff_ffff_ffff_fffe {
        0xffff_ffff
    } else {
        next * SCALE / total
    }
}

/// Walk stages in order until at_progress exceeds progress, keeping the last
/// whose state the entity has. Skip missing states. `None` leaves the entity
/// in its current state.
pub fn animation_state<'s>(
    stages: &'s [installed_content::raids::RaidAnimationStage],
    progress: f32,
    has_state: &dyn Fn(&str) -> bool,
) -> Option<&'s installed_content::raids::RaidAnimationStage> {
    let mut chosen = None;
    for stage in stages {
        if progress < stage.at_progress_raw as f32 / 100_000.0 {
            break;
        }
        if has_state(&stage.state) {
            chosen = Some(stage);
        }
    }
    chosen
}

/// Interpolate current and next phase progress at the tick fraction in phase
/// 4; phase 5 gives exactly 1; other phases give 0.
pub fn movement_progress(phase: i32, p0: f32, p1: f32, tick: f32) -> f32 {
    match phase {
        LAUNCHED => clamped_lerp(p0, p1, tick),
        ENDED => clamped_lerp(1.0, 1.0, tick),
        _ => 0.0,
    }
}

/// Show an entity in phase 4, or in phase 5 while progress is below 0.999 and
/// the outcome is not cancelled. Use progress left by the arrow's last update: an
/// ended unit shows for the one frame whose update sets [`movement_progress`] to 1.
pub fn unit_shown(phase: i32, outcome: i32, progress: f32) -> bool {
    phase == LAUNCHED
        || (phase == ENDED && progress < f32::from_bits(0x3f7f_be77) && outcome != CANCELLED)
}

/// Placement of the raid's unit.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitPlacement {
    /// Arrow point at the unit's fraction plus `RAID_UNIT_ENTITY_OFFSET`, with
    /// x brought back inside the map.
    pub position: [f32; 3],
    /// The unit faces back along the arrow's tangent (`^ 0x80000000`).
    pub facing: [f32; 3],
    /// `unit_model.scale * RAID_UNIT_ENTITY_BASE_SCALE`.
    pub scale: f32,
    /// The arrow's progress ([`movement_progress`]) and the type's
    /// `unit_animations`, which [`animation_state`] reads once the entity's
    /// states are known.
    pub progress: f32,
    pub stages: Vec<installed_content::raids::RaidAnimationStage>,
    /// Entity used to draw the unit (`resolve_entity`); `None` when none is created.
    pub entity: Option<String>,
    /// The raid's id: the entity's animation history belongs to its raid,
    /// so the next hour's placement continues the same entity.
    pub seed: u32,
    /// What places the unit again at another tick progress: playback moves
    /// it every rendered frame ([`UnitTrack::place`]).
    pub track: UnitTrack,
}

/// Per-frame placement operands fixed for the hour: stored arrow controls
/// and lift, phase progress for this hour and the next (interpolated by tick
/// fraction), and the type's start offset over the stored length.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitTrack {
    controls: Vec<Point>,
    kind: RaidArrowKind,
    ends: Option<Ends>,
    defines: RaidGraphicsDefines,
    progress: [f32; 2],
    start: f32,
}

/// The unit at one tick progress: [`UnitPlacement`]'s position, facing and
/// arrow progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitPose {
    pub position: [f32; 3],
    pub facing: [f32; 3],
    pub progress: f32,
}

impl UnitTrack {
    /// A launched raid's track over its arrow's stored `controls`, from its
    /// travelled distance, target distance and speed (fixed5), and its
    /// type's start offset. `None` for an arrow of fewer than two controls.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        controls: &[Point],
        kind: RaidArrowKind,
        heights: &HeightField<'_>,
        defines: &RaidGraphicsDefines,
        travelled: i64,
        distance: i64,
        speed: i64,
        start_offset_raw: i64,
    ) -> Option<Self> {
        if controls.len() < 2 {
            return None;
        }
        let elapsed = elapsed_hours(travelled, speed);
        let remaining = remaining_hours(distance, travelled, speed);
        let length = stored_length(controls);
        let offset = start_offset_raw as f32 / 100_000.0;
        let width = i64::from(heights.width);
        Some(Self {
            controls: controls.to_vec(),
            kind,
            ends: Ends::new(controls, width, &|p| {
                heights.fixed_height(p[0], p[1], width)
            }),
            defines: defines.clone(),
            progress: [
                phase_progress(elapsed, remaining) as f32 / 100_000.0,
                next_hour_progress(elapsed, remaining) as f32 / 100_000.0,
            ],
            start: if offset <= length { offset } else { length } / length,
        })
    }

    /// The arrow progress at the tick's progress, which the entity's stage
    /// reads ([`animation_state`]): [`movement_progress`] in phase 4.
    pub fn progress(&self, tick: f32) -> f32 {
        movement_progress(LAUNCHED, self.progress[0], self.progress[1], tick)
    }

    /// The unit's fraction along the arrow: from the start offset's fraction
    /// of the stored length to the end, by the progress.
    pub fn fraction(&self, tick: f32) -> f32 {
        clamped_lerp(self.start, 1.0, self.progress(tick))
    }

    /// At tick progress, take the arrow point at the unit's fraction, add
    /// `RAID_UNIT_ENTITY_OFFSET`, wrap x inside the map once, and face back along
    /// the tangent.
    pub fn place(&self, heights: &HeightField<'_>, tick: f32) -> Option<UnitPose> {
        let progress = self.progress(tick);
        let fraction = self.fraction(tick);
        // Call the lift with the stored control count.
        let count = self.controls.len() as i32;
        let defines = &self.defines;
        let height = |i: usize, x: f32, z: f32| match (self.kind, self.ends) {
            (RaidArrowKind::Air, Some(ends)) => air_height(ends, i as i32, count, defines),
            (RaidArrowKind::Ballistic, Some(ends)) => {
                ballistic_height(ends, i as i32, count, defines)
            }
            _ => heights.native_height(x, z),
        };
        let ([x, y, z], tangent) = arrow_point(&self.controls, fraction, &height)?;
        let map_width = heights.width as f32;
        let x = if map_width < x { x - map_width } else { x };
        let offset = defines.unit_offset;
        Some(UnitPose {
            position: [x + offset[0], y + offset[1], z + offset[2]],
            // Flip the tangent's sign bits.
            facing: [-tangent[0], -tangent[1], -tangent[2]],
            progress,
        })
    }
}

/// The raid's unit in phase 4, from its planned recorded inputs.
#[allow(clippy::too_many_arguments)]
fn place_unit(
    inputs: &ArrowInputs<'_>,
    defines: &RaidGraphicsDefines,
    world: &World<'_>,
    raid: &RaidRow,
    kind: &RaidType,
    arrow_kind: RaidArrowKind,
    target: &RaidTargetRow,
    geometry: &Geometry,
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
    let distance = target_distance(inputs.assets, frame, raid, target)?;
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
    // A paused or exported frame is the hour's own progress; playback
    // places the unit again every rendered frame ([`UnitTrack::place`]).
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

/// Build [`unit_entity`] from the raid owner's tag and graphical culture
/// (`CountryAppearance`), primary equipment roster (`AvailableEquipmentVariants`
/// kind 0 with `VariantDefinitions` flags) and wing, then resolve the entity.
/// A designer model resolving none falls back to the wing.
fn raid_entity(
    inputs: &ArrowInputs<'_>,
    world: &World<'_>,
    raid: &RaidRow,
    kind: &RaidType,
) -> Result<Option<String>, String> {
    let frame = world.frame();
    let complete = |error: std::io::Error| error.to_string();
    let countries = crate::map::model_countries::read(frame).map_err(complete)?;
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
    let definitions: BTreeMap<u64, &replay::rows::VariantDefinitionRow> = frame
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

/// Choose the largest signed `WingEquipment` amount in a nonempty bank,
/// first on ties, as `aircraft.rs` reads it.
pub fn wing_primary_variant(frame: &DecodedFrame, wing: u64) -> Result<Option<u64>, String> {
    crate::map::aircraft_model::wing_primary(frame, wing)
}

/// An equipment type's archetype name and own sprite. The null archetype name
/// is empty; reading an archetype fills an empty sprite. Refuse a type absent
/// from the content by name.
#[cfg(test)]
fn equipment_names(
    content: &installed_content::equipment::Database,
    name: &str,
) -> Result<(String, String), String> {
    let kind = content
        .named(name)
        .ok_or_else(|| format!("equipment {name} is not in the content"))?;
    let text = |bytes: &[u8], what: &str| {
        installed_content::parser::text(bytes)
            .map(str::to_owned)
            .map_err(|refused| format!("equipment {name} {what}: {refused}"))
    };
    let archetype = match kind.archetype {
        Some(archetype) => text(&content.equipment[archetype].name, "archetype name")?,
        None => String::new(),
    };
    Ok((archetype, text(&kind.sprite, "sprite")?))
}

/// Resolve an aircraft designer model for a variant and raw country:
///
/// 1. With DLC feature 0x26 (bit 8 of `SimulationFeatures.dlc_mask`), use
///    `variant_model`: graphics pools when the equipment type has a sub-unit
///    list, otherwise the saved model. Its name wins when declared.
/// 2. Use the variant's saved model when declared.
/// 3. Use equipment-type candidates: country tag then bare prefixes over
///    equipment, archetype and sprite names (`aircraft::candidates`).
///
/// Return `Ok(None)` when nothing is declared.
#[cfg(test)]
pub(crate) fn designer_entity(
    variant: &replay::rows::VariantDefinitionRow,
    equipment: &str,
    content: &installed_content::equipment::Database,
    tag: &str,
    dlc_mask: u32,
    variant_model: &dyn Fn() -> Result<Option<String>, String>,
    declared: &dyn Fn(&str) -> bool,
) -> Result<Option<String>, String> {
    crate::map::aircraft_model::resolve_lazy(
        variant,
        dlc_mask,
        variant_model,
        &|| Ok(None),
        &|| {
            let (archetype, sprite) = equipment_names(content, equipment)?;
            Ok(
                crate::map::aircraft_model::candidates("", tag, equipment, &archetype, &sprite)
                    .into_iter()
                    .find(|name| declared(name)),
            )
        },
        declared,
    )
}

/// A naval base measures the naval path and a building the land path, both
/// through the session memo. Other sources use wrapped centroid distance
/// over three to the resolved target province. A missing distance is 100000.
pub(super) fn target_distance(
    assets: &RaidAssets,
    frame: &DecodedFrame,
    raid: &RaidRow,
    target: &RaidTargetRow,
) -> Result<i64, String> {
    let kind = raid.source.kind.unwrap_or(u32::MAX);
    let from = raid.source.province;
    let found = if kind == SOURCE_NAVAL_BASE || kind == SOURCE_BUILDING {
        super::paths::Distances::new(assets, frame).distance(
            kind as u8,
            from,
            &target.scope,
            super::paths::Asked::Certain,
        )?
    } else {
        assets.province(&target.scope).and_then(|to| {
            assets
                .geometry
                .wrapped_distance(from, to)
                .map(|pixels| pixels.wrapping_mul(100_000) / 300_000)
        })
    };
    Ok(found.unwrap_or(100_000))
}

/// Find the stored control segment containing `distance` and its fraction;
/// past the end, use the last segment's end.
fn point_along(controls: &[Point], distance: f32) -> (usize, f32) {
    let mut walked = 0.0f32;
    for index in 0..controls.len().saturating_sub(1) {
        let dx = controls[index + 1][0] as f32 / 100_000.0 - controls[index][0] as f32 / 100_000.0;
        let dz = controls[index + 1][1] as f32 / 100_000.0 - controls[index][1] as f32 / 100_000.0;
        let run = (dz * dz + dx * dx).sqrt();
        if run != 0.0 && distance <= run + walked {
            return (index, (distance - walked) / run);
        }
        walked += run;
    }
    (controls.len().saturating_sub(2), 1.0)
}

impl SpeedDefines {
    fn read(graphics: &RaidGraphicsDefines, raids: &super::RaidDefines) -> Result<Self, String> {
        let speeds = raids.unit_speeds.ok_or(
            "NMilitary.LAND_SPEED_MODIFIER, NNavy.NAVAL_SPEED_MODIFIER or NNavy.NAVAL_TRANSFER_BASE_SPEED cannot be read",
        )?;
        Ok(Self {
            land_speed_modifier: speeds[0],
            naval_speed_modifier: speeds[1],
            naval_transfer_base_speed: speeds[2],
            raid_unit_speed_multiplier: graphics.speed_multiplier_raw,
            ace_wing_size: raids.ace_wing_size,
            ace_wing_size_max_bonus: raids.ace_wing_size_max_bonus,
        })
    }
}

/// Stored controls' polyline length.
pub fn stored_length(controls: &[Point]) -> f32 {
    controls
        .windows(2)
        .map(|pair| {
            let dx = pair[1][0] as f32 / 100_000.0 - pair[0][0] as f32 / 100_000.0;
            let dz = pair[1][1] as f32 / 100_000.0 - pair[0][1] as f32 / 100_000.0;
            (dz * dz + dx * dx).sqrt()
        })
        .sum()
}

// Execution checks.

/// The raid's own equipment pool: per-archetype fixed5 totals in stored order
/// as archetype tokens, plus its nuke count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaidPool {
    pub archetype_totals: Vec<(u32, i64)>,
    pub nukes: i32,
}

/// `raid`'s pool from its `RaidEquipment` row (keyed as
/// `Raids`): `archetype_tokens` and `archetype_amounts` zipped. A withheld
/// table (the recorder withholds it for a null or foreign archetype), a
/// raid without a row or lists of different lengths are refused by name.
pub fn raid_pool(frame: &DecodedFrame, raid: &RaidRow) -> Result<RaidPool, String> {
    let rows = frame
        .complete::<tables::RaidEquipment>()
        .map_err(|error| format!("the raid's own equipment pool is withheld: {error}"))?;
    let row = rows
        .iter()
        .find(|row| row.id == raid.id && row.reference_type == raid.reference_type)
        .ok_or("the raid has no RaidEquipment row (its own equipment pool)")?;
    if row.archetype_tokens.len() != row.archetype_amounts.len() {
        return Err(format!(
            "the raid's RaidEquipment row has {} archetype tokens and {} archetype amounts",
            row.archetype_tokens.len(),
            row.archetype_amounts.len()
        ));
    }
    Ok(RaidPool {
        archetype_totals: row
            .archetype_tokens
            .iter()
            .copied()
            .zip(row.archetype_amounts.iter().copied())
            .collect(),
        nukes: row.nukes,
    })
}

/// Phase-3 execution checks, in order: unit at source, raid pool meeting both
/// equipment lists at level 4, then [`launch_triggers`]. A proved false decides;
/// refuse an undecided step by name.
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
                super::spelling(world, token, "equipment archetype").map(str::to_owned)
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

/// Launch-trigger order:
///
/// - Type unlocked for the owner (`visible` and category unlock through
///   [`super::offers::category_admits_unlock`]), not used up (`fire_only_once`),
///   and `show_target`.
/// - Off cooldown and `available`.
/// - `launchable`.
/// - `launchable_from`, with the source country as ROOT.
///
/// Every trigger except the last uses owner scopes: owner as ROOT, target
/// country as FROM, and target state, province and leader as variables.
pub fn launch_triggers(
    assets: &RaidAssets,
    world: &World<'_>,
    owner: u32,
    kind: &RaidType,
    scope: &replay::rows::RaidScope,
    source_country: &Result<Option<u32>, String>,
) -> Eval {
    let target_country = match super::target_country(world, scope) {
        Ok(country) => country,
        Err(reason) => return Eval::unknown(reason),
    };
    let scopes = Scopes {
        cursor: None,
        root: Scope::Country(owner),
        this: Scope::Country(owner),
        prev: None,
        from: target_country.map(Scope::Country),
        target_state: super::offers::target_state(world, scope),
        target_province: [
            scope.province,
            if scope.building_type != 0 && scope.building_flags & 1 != 0 {
                scope.building_location
            } else {
                0
            },
            scope.leader_province,
        ]
        .into_iter()
        .find(|&id| id != 0),
        target_leader: (scope.leader_reference != 0).then_some(scope.leader_reference),
        unknown: Default::default(),
        bare: false,
    };
    world
        .evaluate(&kind.visible, Scopes::of(Scope::Country(owner)))
        .and(|| super::offers::category_admits_unlock(assets, world, owner, kind))
        .and(|| used_up(assets, world, owner, kind).negate())
        .and(|| world.evaluate(&kind.show_target, scopes))
        .and(|| on_cooldown(assets, world, owner, kind, scope).negate())
        .and(|| world.evaluate(&kind.available, scopes))
        .and(|| world.evaluate(&kind.launchable, scopes))
        .and(
            || match (world.has_children(&kind.launchable_from), source_country) {
                (Err(reason), _) => Eval::unknown(reason),
                (Ok(false), _) => Eval::True,
                (Ok(true), source_country) => match source_country {
                    Ok(Some(country)) => world.evaluate(
                        &kind.launchable_from,
                        Scopes {
                            root: Scope::Country(*country),
                            this: Scope::Country(*country),
                            ..scopes
                        },
                    ),
                    Ok(None) => Eval::unknown("the raid source has no country for launchable_from"),
                    Err(reason) => Eval::unknown(reason.clone()),
                },
            },
        )
}

/// `fire_only_once` and the type in the owner's recorded used types
/// (`RaidCountries.used_types`).
pub(super) fn used_up(assets: &RaidAssets, world: &World<'_>, owner: u32, kind: &RaidType) -> Eval {
    if !kind.fire_only_once {
        return Eval::False;
    }
    let rows = match world.frame().complete::<tables::RaidCountries>() {
        Ok(rows) => rows,
        Err(error) => return Eval::unknown(error.to_string()),
    };
    let Some(status) = rows.iter().find(|row| row.canonical_slot == owner) else {
        return Eval::False;
    };
    let mut result = Eval::False;
    for &token in &status.used_types {
        result = result.or(|| match assets.raid_type(world, token) {
            Ok(TypeToken::Type(index)) => {
                Eval::from_bool(assets.catalog.types[index].name == kind.name)
            }
            Ok(TypeToken::Refused(_)) => Eval::False,
            Err(reason) => Eval::unknown(format!("a used raid type may be this one: {reason}")),
        });
    }
    result
}

/// Owner cooldown (`RaidCooldowns`) of this type whose building, province,
/// state and leader arms match the target.
fn on_cooldown(
    assets: &RaidAssets,
    world: &World<'_>,
    owner: u32,
    kind: &RaidType,
    scope: &replay::rows::RaidScope,
) -> Eval {
    let rows = match world.frame().complete::<tables::RaidCooldowns>() {
        Ok(rows) => rows,
        Err(error) => return Eval::unknown(error.to_string()),
    };
    let matches = |row: &replay::rows::RaidScope| {
        row.building_type == scope.building_type
            && (scope.building_type == 0
                || (row.building_location == scope.building_location
                    && row.building_flags == scope.building_flags))
            && row.province == scope.province
            && row.state == scope.state
            && row.leader_reference == scope.leader_reference
    };
    let mut result = Eval::False;
    for row in rows
        .iter()
        .filter(|row| row.owner_slot == owner && matches(&row.scope))
    {
        result = result.or(|| match assets.raid_type(world, row.raid_type) {
            Ok(TypeToken::Type(index)) => {
                Eval::from_bool(assets.catalog.types[index].name == kind.name)
            }
            Ok(TypeToken::Refused(at)) => {
                Eval::from_bool(assets.catalog.uncertain[at].name == kind.name)
            }
            Err(reason) => Eval::unknown(reason),
        });
    }
    result
}

/// A resolving army must stand in the source province; otherwise a resolving
/// wing must have its pool's air base there. Otherwise false.
pub(super) fn unit_at_source(
    world: &World<'_>,
    raid: &RaidRow,
    source: u32,
) -> Result<bool, String> {
    let frame = world.frame();
    let complete = |error: std::io::Error| error.to_string();
    if raid.unit != 0
        && let Some(division) = frame
            .complete::<tables::Divisions>()
            .map_err(complete)?
            .iter()
            .find(|division| division.id == raid.unit)
    {
        return Ok(division.province == source);
    }
    if raid.air_wing != 0
        && let Some(wing) = frame
            .complete::<tables::AirWings>()
            .map_err(complete)?
            .iter()
            .find(|wing| wing.id == raid.air_wing)
    {
        let base = frame
            .complete::<tables::AirBases>()
            .map_err(complete)?
            .iter()
            .find(|base| base.id == wing.air_base)
            .and_then(|base| base.province);
        return Ok(base == Some(source));
    }
    Ok(false)
}

/// Equipment checks at level 4 (level 1 passes): essential list, then
/// additional list, against the raid's pool, nukes and convoys -1. Fail when
/// pool nukes are fewer than required or whole archetype units (fixed5 total
/// truncated towards zero) are fewer than required. Convoys use the larger
/// of the pool and passed count, so -1 leaves the pool alone. Archetype
/// quantity is the first matching recorded total, or zero when absent.
/// `archetype_name` names tokens.
pub fn pool_meets(
    kind: &RaidType,
    pool: &RaidPool,
    archetype_name: &dyn Fn(u32) -> Result<String, String>,
) -> Result<bool, String> {
    let mut have: BTreeMap<String, i64> = BTreeMap::new();
    for &(token, total) in &pool.archetype_totals {
        // The first matching entry answers, as the native scan does.
        have.entry(archetype_name(token)?).or_insert(total);
    }
    for (list, nukes) in [
        (&kind.essential_equipment, kind.nukes),
        (&kind.additional_equipment, kind.additional_nukes),
    ] {
        if pool.nukes < nukes {
            return Ok(false);
        }
        for (archetype, amount) in list {
            let units = (have.get(archetype).copied().unwrap_or(0) / 100_000) as i32;
            if units < *amount {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

// ------------------------------------------------------------ the entity

/// Inputs used to choose the raid unit's entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitEntity {
    /// A declared entity, by name.
    Entity(String),
    /// First primary-roster variant of the unit model's equipment category.
    /// Its model is final, even when it resolves none.
    Designer(u64),
    /// Without a type model, use the wing's primary equipment through the same resolver.
    WingEquipment(u64),
    /// Nothing to show.
    None,
}

/// Entity selection and fallback:
///
/// 1. For an `equipment` category (cleared outside 0x1F0037FC00), choose the
///    first country primary-roster variant whose definition flags share it.
/// 2. Try `entity` prefixed by country tag, then graphical culture, then bare,
///    joining each prefix with an underscore.
/// 3. Try `default_entity`.
/// 4. Otherwise use the raid's wing when it resolves.
///
/// `primary` is the roster's (variant, definition flags) in stored order;
/// `declared` reports whether content declares an entity.
pub fn unit_entity(
    model: &installed_content::raids::RaidUnitModel,
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

/// Stored control `i` with height from the arrow callback at `(i, count)`,
/// or sea-clamped terrain when no callback is set.
fn control_point(
    controls: &[Point],
    i: usize,
    height: &dyn Fn(usize, f32, f32) -> f32,
) -> [f32; 3] {
    let x = controls[i][0] as f32 / 100_000.0;
    let z = controls[i][1] as f32 / 100_000.0;
    [x, height(i, x, z), z]
}

/// Direction at `i`: towards the next control, or from the preceding
/// control at the last.
fn control_direction(
    controls: &[Point],
    i: usize,
    height: &dyn Fn(usize, f32, f32) -> f32,
) -> [f32; 3] {
    let here = control_point(controls, i, height);
    if i + 1 == controls.len() {
        normalize(sub(here, control_point(controls, i - 1, height)))
    } else {
        normalize(sub(control_point(controls, i + 1, height), here))
    }
}

/// Point at fraction `u` of stored control length and its tangent, both
/// interpolated between the surrounding controls.
pub fn arrow_point(
    controls: &[Point],
    u: f32,
    height: &dyn Fn(usize, f32, f32) -> f32,
) -> Option<([f32; 3], [f32; 3])> {
    if controls.len() < 2 {
        return None;
    }
    let (index, t) = if (0.0..=1.0).contains(&u) {
        point_along(controls, stored_length(controls) * u)
    } else {
        (0, 0.0)
    };
    let (a, b) = (
        control_point(controls, index, height),
        control_point(controls, index + 1, height),
    );
    let (da, db) = (
        control_direction(controls, index, height),
        control_direction(controls, index + 1, height),
    );
    let lerp = |p: [f32; 3], q: [f32; 3]| {
        [
            (q[0] - p[0]) * t + p[0],
            (q[1] - p[1]) * t + p[1],
            (q[2] - p[2]) * t + p[2],
        ]
    };
    Some((lerp(a, b), normalize(lerp(da, db))))
}

#[cfg(test)]
mod tests;

/// A 50-long line arrow's track two of four hours in, for other modules'
/// tests, and the flat heightfield it is placed on.
#[cfg(test)]
pub(crate) fn test_track() -> (UnitTrack, HeightField<'static>) {
    (tests::line_track(60_000, 1_500_000), tests::flat())
}
