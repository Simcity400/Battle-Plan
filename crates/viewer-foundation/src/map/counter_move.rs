//! Settled unit movement positions for counters and figures.
//!
//! The movement animator builds a polyline through successive provinces' standstill
//! anchors: slot zero for divisions, the railway-gun slot for guns and port building
//! anchors for docked task forces. Neighbour hops are retained; rejected hops leave
//! the preceding province as the next comparison source. End handles extend a quarter
//! of the first and last segments. The flat controls are sampled as a Catmull-Rom
//! spline before terrain height is sampled at each result.
//!
//! The free arc starts at the current province's anchor; the next anchor selects the
//! sample that limits segment search. The free arc advances by the content's movement
//! speed and frame time, with zero speed in combat. While the unit remains in the
//! first province, it is clamped to one world unit. Interpolation itself is unclamped.
//! Counters and figures share this point. Simulation progress affects selected-unit
//! arrows rather than the counter position.
//!
//! Arrival glides along the old spline before rebuilding. Reroutes, teleports, jumps
//! of two or more hops, game speed five, combat and six game days force a rebuild.
//! Recorded hours use the settled arc ([`SETTLED_ARC`]); they do not carry the arrival
//! glide clock. The settled prefix is walked only until its necessary controls are
//! known, and rejected hops are counted. Allocation occurs during hour preparation;
//! frames read the stored point. Naval admission shares a complete route sample
//! between task-force kind, position and scale.

use crate::map::movement_routes::catmull_rom;
use crate::map::province_map::HeightField;
use crate::map::tables::rows;
use crate::movement_timing::Etas;
use crate::unit_class::CounterClass;
use installed_content_core::movement::MovementMap;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::time::Instant;

/// Land-counter display inputs. Animator operands belong to this view rather
/// than the recording client's map entity.
#[derive(Clone, Copy, Debug, Default)]
pub struct StackKindInputs {
    pub class: CounterClass,
    pub combat_role: u8,
    pub combat_count: u32,
    pub ordinary_path_total: u32,
    pub movement_target: u32,
    pub naval_transfer: bool,
    /// Province-definition kind flags of the unit's province.
    pub terrain_flags: u8,
    pub display_moving: bool,
    /// Position of the direction province in the native neighbour order.
    pub direction: Option<usize>,
}

/// The division, class-1 and class-13 branches: the unit-stack slot a counter joins.
///
/// * Defending in its first combat: stand-off kind 10; a task force attacking in it: 9.
/// * Standing (no displayed route, no combat, and an empty ordinary route or a set movement
///   target): 19 for a task force or a naval transfer on land, else the class's
///   [`CounterClass::standstill`] slot.
/// * Otherwise 20 for a task force or a naval transfer on land, else the class's directional
///   kinds toward the direction province: from land 1..=8 (division, task force) or 22..=29
///   (gun), from sea 11..=18 or 30..=37.
pub fn stack_kind(input: StackKindInputs) -> u32 {
    if input.combat_role == 2 {
        return 10;
    }
    let task_force = input.class == CounterClass::TaskForce;
    if input.combat_role == 1 && task_force {
        return 9;
    }
    let land = input.terrain_flags & 1 != 0;
    let naval = land && (task_force || input.naval_transfer);
    // The ordinary-route clause is the native test. For a cold rebuild ([`UnitRoute::stack_kind`])
    // it always holds when the display route is empty: that route is the alternate route when
    // nonempty, else the ordinary one, so an empty display route means an empty ordinary route.
    if (input.ordinary_path_total == 0 || input.movement_target != 0)
        && !input.display_moving
        && input.combat_count == 0
    {
        return if naval {
            19
        } else {
            input.class.standstill(land)
        };
    }
    if naval {
        return 20;
    }
    input.class.directional(input.terrain_flags & 3 != 0)
        + input.direction.filter(|&index| index < 8).unwrap_or(0) as u32
}

/// Use stack anchors for the combat stand-off kinds (9 and 10 for a division
/// or a task force, 10 for a railway gun) and for an attacking unit's directional kinds from
/// land; every other kind uses its animator.
pub fn snaps_to_stack(class: CounterClass, kind: u32, combat_role: u8, combat_count: u32) -> bool {
    let stand_off = match class {
        CounterClass::Division | CounterClass::TaskForce => matches!(kind, 9 | 10),
        CounterClass::RailwayGun => kind == 10,
    };
    let from_land = class.directional(true);
    stand_off
        || (combat_count != 0 && combat_role == 1 && (from_land..from_land + 8).contains(&kind))
}

/// `g_TickThreshold` : the free arc's clamp while the unit is still in its
/// province, in world units.
pub const SETTLED_ARC: f32 = 1.0;

/// A unit's simulation route: a division's `DivisionMovement` row, a railway gun's
/// `RailwayGunCounterInputs` row or a task force's `TaskForceMapMotion` row.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnitRoute<'a> {
    pub class: CounterClass,
    pub combat_role: u8,
    pub combat_count: u32,
    /// Province of the first referenced combat, zero without one.
    pub combat_province: u32,
    /// The movement target province, zero when null.
    pub target: u32,
    pub naval_transfer: bool,
    pub path_total: u32,
    pub path: &'a [u32],
    pub alternate_total: u32,
    pub alternate_path: &'a [u32],
}

impl<'a> UnitRoute<'a> {
    pub fn division(row: &'a replay_data::rows::DivisionMovementRow) -> Self {
        Self {
            class: CounterClass::Division,
            combat_role: row.combat_role,
            combat_count: row.combat_count,
            combat_province: row.combat_province,
            target: row.target,
            naval_transfer: row.naval_transfer != 0,
            path_total: row.path_total,
            path: &row.path,
            alternate_total: row.alternate_total,
            alternate_path: &row.alternate_path,
        }
    }

    /// A class-13 railway gun's route. Its routes are recorded whole, so each total is its
    /// route's length.
    pub fn railway_gun(row: &'a replay_data::rows::RailwayGunCounterInputRow) -> Self {
        Self {
            class: CounterClass::RailwayGun,
            combat_role: row.combat_role,
            combat_count: row.combat_count,
            combat_province: row.combat_province,
            target: row.target,
            naval_transfer: row.naval_transfer != 0,
            path_total: row.ordinary_path.len() as u32,
            path: &row.ordinary_path,
            alternate_total: row.alternate_path.len() as u32,
            alternate_path: &row.alternate_path,
        }
    }

    /// A class-1 task force's route. Its routes are recorded whole, so each total is its
    /// route's length; its first combat's role is all of its combat a task force's stack kind
    /// reads.
    pub fn task_force(row: &'a replay_data::rows::TaskForceMapMotionRow) -> Self {
        Self {
            class: CounterClass::TaskForce,
            combat_role: row.combat_role,
            combat_count: u32::from(row.combat_role != 0),
            combat_province: 0,
            target: row.target,
            naval_transfer: false,
            path_total: row.path.len() as u32,
            path: &row.path,
            alternate_total: row.alternate_path.len() as u32,
            alternate_path: &row.alternate_path,
        }
    }

    /// Build the local route from the alternate route
    /// whenever that array is nonempty, otherwise from the ordinary route.
    pub fn display_path(&self) -> &'a [u32] {
        if self.alternate_total != 0 {
            self.alternate_path
        } else {
            self.path
        }
    }

    fn display_total(&self) -> u32 {
        if self.alternate_total != 0 {
            self.alternate_total
        } else {
            self.path_total
        }
    }

    /// Cold animator reconstruction. A whole route with an admitted first hop and
    /// anchors at both ends establishes movement; unavailable geometry returns `None`.
    /// Only the first hop decides the stack kind. `anchor` supplies the class's
    /// standstill slot in each province.
    pub fn stack_kind(
        &self,
        province: u32,
        map: &MovementMap,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
    ) -> Option<u32> {
        if self.combat_role == 2 {
            return Some(10);
        }
        let terrain = map.terrain(province)?;
        let neighbours = map.neighbours(province)?;
        let path = self.display_path();
        if path.len() != self.display_total() as usize {
            return None;
        }
        anchor(province)?;
        if let Some(&next) = path.first() {
            anchor(next)?;
            if !neighbours.contains(&next) {
                return None;
            }
        }
        // Use the first display province, else the first combat's province,
        // independently of the separate movement target.
        let direction_target = path.first().copied().unwrap_or(if self.combat_count != 0 {
            self.combat_province
        } else {
            0
        });
        Some(stack_kind(StackKindInputs {
            class: self.class,
            combat_role: self.combat_role,
            combat_count: self.combat_count,
            ordinary_path_total: self.path_total,
            movement_target: self.target,
            naval_transfer: self.naval_transfer,
            terrain_flags: terrain.flags,
            display_moving: !path.is_empty(),
            direction: neighbours
                .iter()
                .position(|&neighbour| neighbour == direction_target),
        }))
    }

    /// Stack kind uses the animator sample count,
    /// not the simulation route's length. the game can clear a nonempty
    /// route when skipped hops leave fewer than four control points.
    pub fn stack_kind_from_animator(
        &self,
        province: u32,
        map: &MovementMap,
        sample_count: usize,
    ) -> Option<u32> {
        let terrain = map.terrain(province)?;
        let neighbours = map.neighbours(province)?;
        let path = self.display_path();
        if path.len() != self.display_total() as usize {
            return None;
        }
        // Use the animator route[1]. When the game clears
        // its samples/route, the game falls back to the combat province
        // or the ORDINARY route's first hop, not the alternate.
        let direction_target = if sample_count != 0 {
            path.first().copied().unwrap_or(0)
        } else if self.combat_count != 0 {
            self.combat_province
        } else {
            self.path.first().copied().unwrap_or(0)
        };
        Some(stack_kind(StackKindInputs {
            class: self.class,
            combat_role: self.combat_role,
            combat_count: self.combat_count,
            ordinary_path_total: self.path_total,
            movement_target: self.target,
            naval_transfer: self.naval_transfer,
            terrain_flags: terrain.flags,
            display_moving: sample_count != 0,
            direction: neighbours
                .iter()
                .position(|&neighbour| neighbour == direction_target),
        }))
    }

    /// The anchor the counter stands on in `province`, a land province when
    /// `land`: its kind's stack anchor when that kind snaps,
    /// else the animator's point on the class's [`CounterClass::standstill`]
    /// slot, never a directional kind's anchor. A snapped kind without its own
    /// anchor takes the standing slot.
    pub fn standing(
        &self,
        province: u32,
        land: bool,
        kind: u32,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
    ) -> Option<[f32; 3]> {
        let standstill = self.class.standstill(land);
        if snaps_to_stack(self.class, kind, self.combat_role, self.combat_count) {
            anchor(province, kind).or_else(|| anchor(province, standstill))
        } else {
            anchor(province, standstill)
        }
    }

    /// The counter's position this hour: [`Self::standing`] for a snapped kind
    /// or a unit without a usable route, else the settled point on its route
    /// ([`Motion`]).
    pub fn position(
        &self,
        province: u32,
        kind: u32,
        world: &RouteWorld<'_>,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
    ) -> Option<Position> {
        let land = |id| {
            world
                .map
                .terrain(id)
                .is_some_and(|terrain| terrain.flags & 1 != 0)
        };
        let standing = self.standing(province, land(province), kind, anchor)?;
        let snapped = snaps_to_stack(self.class, kind, self.combat_role, self.combat_count);
        let path = self.display_path();
        if snapped || path.is_empty() {
            return Some(Position {
                standing,
                motion: None,
            });
        }
        let class = self.class;
        let motion = Motion::from_route(
            province,
            path,
            &|id| anchor(id, class.standstill(land(id))),
            world,
        );
        Some(Position { standing, motion })
    }
}

/// Content the route sampler reads: movement geometry, terrain height and the
/// map width the route wraps at.
#[derive(Clone, Copy)]
pub struct RouteWorld<'a> {
    pub map: &'a MovementMap,
    pub heights: HeightField<'a>,
    pub width: f32,
}

/// One division's movement this hour, as every map layer that places it reads it: the counter
/// (the host counter renderer) and the close-zoom figure (the host unit figures) take the same stack
/// kind and route so a counter and its figure never disagree.
#[derive(Clone, Debug, Default)]
pub struct DivisionRoute {
    /// The animator's display route (`DivisionMovement.alternate_path` when nonempty, else
    /// `path`); `path[0]` is the next hop. Empty when content cannot place the route.
    pub path: Vec<u32>,
    /// Estimated fixed5 `(next_remaining, destination_remaining)`; refused estimates stay absent.
    pub eta: Option<(i64, i64)>,
    pub naval_transfer: bool,
    pub retreat: bool,
    pub combat_role: u8,
    pub combat_count: u32,
    /// The result, rebuilt from the route and content
    /// ([`UnitRoute::stack_kind`]). `9` and `10` are the combat stand-off slots.
    pub stack_kind: u32,
    /// Whether `stack_kind` is that native kind. A route the content map cannot
    /// place stands on the standstill fallback instead, which the counter may
    /// draw but the combat wedge must not take as the unit's kind.
    pub kind_resolved: bool,
    /// Return 1 for a land unit iff both arrays are empty.
    pub idle: bool,
}

impl DivisionRoute {
    /// The next hop and the one after it (zero without one); `None` for a
    /// division that is not moving.
    pub fn signature(&self) -> Option<[u32; 2]> {
        let next = *self.path.first()?;
        Some([next, self.path.get(1).copied().unwrap_or(0)])
    }

    /// The route the counter follows, on its display path.
    pub fn route(&self) -> UnitRoute<'_> {
        UnitRoute {
            combat_role: self.combat_role,
            combat_count: self.combat_count,
            naval_transfer: self.naval_transfer,
            path_total: self.path.len() as u32,
            path: &self.path,
            ..Default::default()
        }
    }

    /// Where this division's stack hangs in `province`; `None` without an anchor.
    pub fn position(
        &self,
        province: u32,
        world: Option<&RouteWorld<'_>>,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
    ) -> Option<Position> {
        let standing = || {
            anchor(province, CounterClass::Division.standstill(false)).map(|standing| Position {
                standing,
                motion: None,
            })
        };
        let Some(world) = world else {
            return standing();
        };
        self.route()
            .position(province, self.stack_kind, world, anchor)
            .or_else(standing)
    }
}

/// This hour's division routes and how many the content map could not place.
#[derive(Clone, Debug, Default)]
pub struct DivisionRoutes {
    pub by_division: BTreeMap<u64, DivisionRoute>,
    /// Moving divisions whose route the content map cannot place; they stand
    /// on their province and are counted, never dropped.
    pub unresolved: usize,
    /// Later route hops the placed routes' polylines skip ([`walk_route`]):
    /// no content neighbour of the province before them, or no anchor.
    pub skipped_hops: usize,
}

impl DivisionRoutes {
    /// `anchor` is a province's `Standstill` anchor: a route through a province
    /// without one is not placed.
    pub fn read(
        frame: &replay_frame::frame::DecodedFrame,
        etas: &Etas,
        map: Option<&MovementMap>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
    ) -> std::io::Result<Self> {
        use replay_data::rows::tables;
        let location: BTreeMap<u64, u32> = rows::<tables::Divisions>(frame)?
            .rows()
            .iter()
            .map(|row| (row.id, row.province))
            .collect();
        let mut out = Self::default();
        for row in rows::<tables::DivisionMovement>(frame)?.rows() {
            let route = UnitRoute::division(row);
            let province = location.get(&row.division).copied().unwrap_or(0);
            let resolved = map.and_then(|map| route.stack_kind(province, map, anchor));
            let (stack_kind, path, kind_resolved) = match (map, resolved) {
                (Some(map), Some(kind)) => {
                    let path = route.display_path();
                    out.skipped_hops += walk_route(map, province, path, anchor).skipped;
                    (kind, path.to_vec(), true)
                }
                _ => {
                    // An unplaceable route stands still on its province; the
                    // stand-off kinds need no geometry.
                    if !route.display_path().is_empty() {
                        out.unresolved += 1;
                    }
                    let kind = stack_kind(StackKindInputs {
                        combat_role: row.combat_role,
                        ..Default::default()
                    });
                    // Defending in its first combat is kind 10 whatever the
                    // route, so only that fallback is native.
                    (kind, Vec::new(), row.combat_role == 2)
                }
            };
            out.by_division.insert(
                row.division,
                DivisionRoute {
                    path,
                    eta: etas.get(row.division),
                    naval_transfer: row.naval_transfer != 0,
                    retreat: row.retreat != 0,
                    combat_role: row.combat_role,
                    combat_count: row.combat_count,
                    stack_kind,
                    kind_resolved,
                    idle: crate::map::unit_rules::idle(row.path_total, row.combat_count),
                },
            );
        }
        Ok(out)
    }
}

/// how far past the first and last control point the phantom handles reach.
pub const ENDPOINT_EXTENSION: f32 = 0.25;

/// Samples per control interval.
pub const SUBDIVISIONS: u32 = 20;

/// Squared horizontal distance threshold for matching the next anchor and
/// for taking heading from a spline segment.
const NEXT_ANCHOR_EPSILON: u32 = 0x3a83_126f;

/// a segment shorter than this snaps to its end rather than dividing.
const DEGENERATE_SEGMENT: f32 = 1e-4;

/// One resampled point of the spline and its cumulative arc length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub at: [f32; 3],
    pub arc: f32,
}

/// Brings `point` within half a map width of `near`, the way the game does.
///
/// The polyline is built in world x, which wraps at the map `width`; a route that crosses the seam
/// would otherwise be drawn as a dash back across the whole map.
pub fn unwrap_x(width: f32, point: [f32; 3], near: [f32; 3]) -> [f32; 3] {
    let mut out = point;
    let half = width * 0.5;
    while out[0] - near[0] > half {
        out[0] -= width;
    }
    while near[0] - out[0] > half {
        out[0] += width;
    }
    out
}

/// One hop `movement path construction` keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hop {
    /// The last kept province's anchor, and this hop's.
    from: [f32; 3],
    to: [f32; 3],
    /// The admitted crossing ([`MovementMap::route_crossing`]).
    crossing: Option<[i32; 2]>,
    /// The route's first hop: the polyline starts with a phantom handle.
    first: bool,
    /// The route's last hop: the polyline ends with its anchor and a phantom handle.
    last: bool,
}

/// The hops of a route the settled point reads, and the hops skipped among them.
#[derive(Clone, Debug, Default, PartialEq)]
struct RouteWalk {
    hops: Vec<Hop>,
    skipped: usize,
}

/// The walk from `province` along `path`. A hop is kept when it is a
/// content neighbour of the last kept province
/// and has an anchor, the viewer's counterpart of the native province lookup;
/// otherwise it is skipped and the next hop is tested against the same
/// province. A province without an anchor keeps nothing, as the native builder
/// returns on a null province. The walk stops once the controls
/// [`Motion::settled`] reads are fixed, two past the next province's anchor
/// (see the module comment), or at the end of the route.
fn walk_route(
    map: &MovementMap,
    province: u32,
    path: &[u32],
    anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
) -> RouteWalk {
    walk_route_until(map, province, path, anchor, true)
}

fn walk_route_until(
    map: &MovementMap,
    province: u32,
    path: &[u32],
    anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
    prefix: bool,
) -> RouteWalk {
    let mut walk = RouteWalk::default();
    let Some(mut from_anchor) = anchor(province) else {
        return walk;
    };
    let mut from = province;
    // Control points the kept hops append, and the position of the next
    // province's anchor among them once it is appended.
    let mut controls = 0;
    let mut next_anchor = None;
    let mut from_is_next = false;
    for (index, &hop) in path.iter().enumerate() {
        if prefix && next_anchor.is_some_and(|at| controls >= at + 3) {
            break;
        }
        let (Some(crossing), Some(to)) = (map.route_crossing(from, hop), anchor(hop)) else {
            walk.skipped += 1;
            continue;
        };
        let (first, last) = (index == 0, index + 1 == path.len());
        controls += usize::from(first);
        if from_is_next {
            next_anchor = Some(controls);
        }
        controls += 1 + usize::from(crossing.is_some());
        if last {
            if first {
                next_anchor = Some(controls);
            }
            controls += 2;
        }
        walk.hops.push(Hop {
            from: from_anchor,
            to,
            crossing,
            first,
            last,
        });
        (from, from_anchor, from_is_next) = (hop, to, first);
    }
    walk
}

/// A route whose every hop is kept: consecutive anchors and the crossings
/// between them.
#[cfg(any(test, feature = "test-support"))]
fn chain(anchors: &[[f32; 3]], crossings: &[Option<[i32; 2]>]) -> Option<Vec<Hop>> {
    if anchors.len() < 2 || crossings.len() != anchors.len() - 1 {
        return None;
    }
    Some(
        anchors
            .windows(2)
            .zip(crossings)
            .enumerate()
            .map(|(index, (pair, &crossing))| Hop {
                from: pair[0],
                to: pair[1],
                crossing,
                first: index == 0,
                last: index + 2 == anchors.len(),
            })
            .collect(),
    )
}

/// The control points for the kept hops: each hop appends its source
/// anchor and its admitted crossing, the first hop a phantom handle before them
/// and the last hop its own anchor and a phantom handle after. Each point is
/// brought within half a map width of the one before. Every control point has
/// y = 0; terrain height is sampled after the spline.
fn route_control_points(width: f32, hops: &[Hop]) -> Vec<[f32; 3]> {
    let mut points = Vec::with_capacity(hops.len() * 2 + 3);
    for hop in hops {
        let from = [hop.from[0], 0.0, hop.from[2]];
        let to = unwrap_x(width, [hop.to[0], 0.0, hop.to[2]], from);
        if hop.first {
            points.push([
                from[0] + (from[0] - to[0]) * ENDPOINT_EXTENSION,
                0.0,
                from[2] + (from[2] - to[2]) * ENDPOINT_EXTENSION,
            ]);
        }
        let previous = points.last().copied().unwrap_or(from);
        points.push(unwrap_x(width, from, previous));
        if let Some([x, z]) = hop.crossing {
            let previous = *points.last().expect("source anchor appended above");
            points.push(unwrap_x(width, [x as f32, 0.0, z as f32], previous));
        }
        if hop.last {
            let previous = *points.last().expect("source anchor appended above");
            let end = unwrap_x(width, to, previous);
            points.push(end);
            points.push([
                end[0] + (to[0] - from[0]) * ENDPOINT_EXTENSION,
                0.0,
                end[2] + (to[2] - from[2]) * ENDPOINT_EXTENSION,
            ]);
        }
    }
    points
}

/// The complete native animator controls, shared with its selected-unit ribbon.
pub struct RouteControls {
    pub controls: Vec<[f32; 3]>,
    pub skipped: usize,
    pub next_arc: f32,
    pub total_arc: f32,
}

pub fn route_controls(
    province: u32,
    path: &[u32],
    anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
    world: &RouteWorld<'_>,
) -> RouteControls {
    let walk = walk_route_until(world.map, province, path, anchor, false);
    let controls = route_control_points(world.width, &walk.hops);
    let samples = spline(&controls, SUBDIVISIONS);
    let next = path
        .first()
        .and_then(|&hop| anchor(hop))
        .unwrap_or_default();
    let a = next_anchor_sample(world.width, &samples, next);
    RouteControls {
        next_arc: samples.get(a).map_or(0.0, |sample| sample.arc),
        total_arc: samples.last().map_or(0.0, |sample| sample.arc),
        controls,
        skipped: walk.skipped,
    }
}

/// Resamples a control polyline into spline samples carrying cumulative arc length,
/// using uniform Catmull-Rom in the native coefficient and summation order.
pub fn spline(control: &[[f32; 3]], subdivisions: u32) -> Vec<Sample> {
    let subdivisions = subdivisions.max(1);
    let mut out: Vec<Sample> = Vec::new();
    if control.len() < 4 {
        for point in control {
            let arc = out
                .last()
                .map_or(0.0, |last: &Sample| last.arc + distance(last.at, *point));
            out.push(Sample { at: *point, arc });
        }
        return out;
    }
    // The game emits at i/n but charges the chord to i/n + 1/n, whose f32
    // rounding can differ from (i+1)/n. The terminal backward chord is charged
    // once more before emitting its endpoint, as in the ribbon resampler.
    let increment = 1.0 / subdivisions as f32;
    let mut arc = 0.0;
    for (segment, window) in control.windows(4).enumerate() {
        let terminal = segment + 4 == control.len();
        for step in 0..subdivisions + u32::from(terminal) {
            let t = step as f32 / subdivisions as f32;
            let last = step == subdivisions;
            let point = |t| catmull_rom(window[0], window[1], window[2], window[3], t);
            let (a, b) = if last {
                (point(t - increment), point(t))
            } else {
                (point(t), point(t + increment))
            };
            let chord = distance(a, b);
            if last {
                arc += chord;
            }
            out.push(Sample {
                at: if last { b } else { a },
                arc,
            });
            if !last {
                arc += chord;
            }
        }
    }
    out
}

/// The first sample within the native horizontal squared-distance threshold,
/// falling back to the last sample. The locator's x is unwrapped against each
/// sample before the comparison.
fn next_anchor_sample(width: f32, samples: &[Sample], point: [f32; 3]) -> usize {
    let epsilon = f32::from_bits(NEXT_ANCHOR_EPSILON);
    for (index, sample) in samples.iter().enumerate() {
        let locator = unwrap_x(width, point, sample.at);
        let (dx, dz) = (locator[0] - sample.at[0], locator[2] - sample.at[2]);
        if dx * dx + dz * dz < epsilon {
            return index;
        }
    }
    samples.len().saturating_sub(1)
}

/// The segment for `arc`: the first index in `1..=a` whose sample
/// lies past the arc, else `a`; `None` when `a` is zero, where the engine keeps
/// the point the game placed on the province's anchor.
fn segment(samples: &[Sample], a: usize, arc: f32) -> Option<usize> {
    if a == 0 || a >= samples.len() {
        return None;
    }
    Some((1..=a).find(|&index| samples[index].arc > arc).unwrap_or(a))
}

/// The drawn point on segment `index` at `arc`: the linear
/// interpolation of its two samples, the parameter not clamped, so an arc past
/// a short `t[A]` runs on past it as the engine's does.
fn point_on(samples: &[Sample], index: usize, arc: f32) -> [f32; 3] {
    let (before, after) = (samples[index - 1], samples[index]);
    let span = after.arc - before.arc;
    if span <= DEGENERATE_SEGMENT {
        return after.at;
    }
    let u = (arc - before.arc) / span;
    [
        before.at[0] + (after.at[0] - before.at[0]) * u,
        before.at[1] + (after.at[1] - before.at[1]) * u,
        before.at[2] + (after.at[2] - before.at[2]) * u,
    ]
}

/// The heading on segment `index`: `atan2f(dz, dx)` of the segment
/// when its squared length passes the threshold, else the straight line to the
/// next province's anchor (the heading a fresh rebuild assigns outright).
fn heading_on(samples: &[Sample], index: usize, width: f32, anchors: &[[f32; 3]; 2]) -> f32 {
    let (before, after) = (samples[index - 1].at, samples[index].at);
    let (dx, dz) = (after[0] - before[0], after[2] - before[2]);
    if dx * dx + dz * dz > f32::from_bits(NEXT_ANCHOR_EPSILON) {
        return dz.atan2(dx);
    }
    let next = unwrap_x(width, anchors[1], anchors[0]);
    let (dx, dz) = (next[0] - anchors[0][0], next[2] - anchors[0][2]);
    if dx == 0.0 && dz == 0.0 {
        0.0
    } else {
        dz.atan2(dx)
    }
}

/// A moving division's settled point and heading this hour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    at: [f32; 3],
    heading: f32,
}

/// Viewer-owned animator state from recorded routes and content geometry.
/// A nonempty route may have no samples; sample zero can be the next anchor
/// without positional motion.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RouteSample {
    pub motion: Option<Motion>,
    /// `(controls - 3) * SUBDIVISIONS + 1`.
    pub sample_count: usize,
    /// The current route sample's parameter, retained for arrival glide.
    pub next_arc: f32,
    pub scale_fraction: f32,
}

impl RouteSample {
    fn from_samples(width: f32, samples: &[Sample], anchors: [[f32; 3]; 2]) -> Self {
        let a = next_anchor_sample(width, samples, anchors[1]);
        Self {
            motion: Motion::settled(width, samples, anchors),
            sample_count: samples.len(),
            next_arc: samples[a].arc,
            scale_fraction: scale_fraction(samples[a].arc),
        }
    }

    /// The segment index at the settled arc, including sample A=0.
    pub fn interpolate_scale(&self, from: f32, to: f32) -> f32 {
        (to - from) * self.scale_fraction + from
    }
}

impl Motion {
    /// Builds the game's route from `province` along the display `path` with
    /// the matching map content ([`walk_route`]). The neighbour crossing and the
    /// height callback are presentation geometry; neither is recorded from the
    /// client's map entities. `A` is searched against the next province's
    /// anchor whatever the walk skipped, the origin for a
    /// province without one, as the native reads a null province's.
    pub fn from_route(
        province: u32,
        path: &[u32],
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
        world: &RouteWorld<'_>,
    ) -> Option<Self> {
        let start = anchor(province)?;
        let next = path
            .first()
            .and_then(|&hop| anchor(hop))
            .unwrap_or_default();
        let walk = walk_route(world.map, province, path, anchor);
        Self::from_hops(world.width, &walk.hops, [start, next], &world.heights)
            .and_then(|sample| sample.motion)
    }

    /// Sample once for the stack selector, route point and scale. The complete
    /// control walk supplies the actual control count.
    pub fn route_sample(
        province: u32,
        path: &[u32],
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
        world: &RouteWorld<'_>,
    ) -> Option<RouteSample> {
        let start = anchor(province)?;
        let next = path
            .first()
            .and_then(|&hop| anchor(hop))
            .unwrap_or_default();
        let walk = walk_route_until(world.map, province, path, anchor, false);
        Self::from_hops(world.width, &walk.hops, [start, next], &world.heights)
    }

    /// A route whose every hop is kept, for the tests.
    #[cfg(any(test, feature = "test-support"))]
    fn new_with_crossings(
        width: f32,
        anchors: &[[f32; 3]],
        crossings: &[Option<[i32; 2]>],
        heights: &HeightField<'_>,
    ) -> Option<Self> {
        let hops = chain(anchors, crossings)?;
        Self::from_hops(width, &hops, [anchors[0], anchors[1]], heights)
            .and_then(|sample| sample.motion)
    }

    /// `anchors` are the unit's province's and the next province's.
    fn from_hops(
        width: f32,
        hops: &[Hop],
        anchors: [[f32; 3]; 2],
        heights: &HeightField<'_>,
    ) -> Option<RouteSample> {
        let _timer = ProfileTimer::start();
        let points = route_control_points(width, hops);
        // Clear a route of fewer than four controls: not moving.
        if points.len() < 4 {
            return Some(RouteSample::default());
        }
        // The game accumulates arc length on the flat controls before its
        // terrain callback overwrites each sample's y. Its terminal sample
        // takes one extra last-chord step, unlike interior samples.
        let mut samples = spline(&points, SUBDIVISIONS);
        for sample in &mut samples {
            if !sample.at[0].is_finite() || !sample.at[2].is_finite() {
                return None;
            }
            sample.at[1] = heights.native_height(sample.at[0], sample.at[2]);
        }
        Some(RouteSample::from_samples(width, &samples, anchors))
    }

    /// The point and heading at [`SETTLED_ARC`], the segment search capped at
    /// the next province's sample; `None` when that sample is the first, where
    /// the unit stands on its anchor.
    fn settled(width: f32, samples: &[Sample], anchors: [[f32; 3]; 2]) -> Option<Self> {
        let next = unwrap_x(width, anchors[1], anchors[0]);
        let a = next_anchor_sample(width, samples, next);
        let index = segment(samples, a, SETTLED_ARC)?;
        Some(Self {
            at: point_on(samples, index, SETTLED_ARC),
            heading: heading_on(samples, index, width, &anchors),
        })
    }

    /// Anchor-only motion for the tests: the production route with every hop
    /// kept, no crossings and a flat field at the water floor
    /// ([`crate::map::province_map::WATER_HEIGHT`]).
    #[cfg(any(test, feature = "test-support"))]
    pub fn new(width: f32, anchors: &[[f32; 3]]) -> Option<Self> {
        let crossings = vec![None; anchors.len().saturating_sub(1)];
        let flat = HeightField {
            width: 2,
            height: 2,
            values: &[0; 4],
        };
        Self::new_with_crossings(width, anchors, &crossings, &flat)
    }

    pub fn position(&self) -> [f32; 3] {
        self.at
    }

    /// The point and the heading, `atan2f(tangent.z, tangent.x)` about +Y.
    pub fn position_heading(&self) -> ([f32; 3], f32) {
        (self.at, self.heading)
    }
}

/// interpolate scale over t[A], capped to [0,1]; zero arc takes 1.
fn scale_fraction(next_arc: f32) -> f32 {
    if next_arc == 0.0 {
        1.0
    } else {
        (SETTLED_ARC / next_arc).clamp(0.0, 1.0)
    }
}

/// An anchored stack, with its optional settled route point.
#[derive(Clone, Debug)]
pub struct Position {
    pub standing: [f32; 3],
    pub motion: Option<Motion>,
}
impl Position {
    pub fn at(&self) -> [f32; 3] {
        self.motion.as_ref().map_or(self.standing, Motion::position)
    }
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    (dy * dy + dx * dx + dz * dz).sqrt()
}

use crate::profile_writer::profiling;

thread_local! { static SPLINE_MS: Cell<f64> = const { Cell::new(0.0) }; }
pub fn take_profile() -> f64 {
    SPLINE_MS.with(|value| value.replace(0.0))
}
pub struct ProfileTimer(Option<Instant>);
impl ProfileTimer {
    pub fn start() -> Self {
        Self(profiling().then(Instant::now))
    }
}
impl Drop for ProfileTimer {
    fn drop(&mut self) {
        if let Some(start) = self.0 {
            SPLINE_MS.with(|value| value.set(value.get() + start.elapsed().as_secs_f64() * 1000.0));
        }
    }
}

#[cfg(test)]
mod tests;
