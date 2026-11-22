
use crate::map::movement_routes::catmull_rom;
use crate::map::province_map::HeightField;
use crate::map::tables::rows;
use crate::movement_timing::Etas;
use crate::unit_class::CounterClass;
use crate::unit_handle::Unit;
use content_reader_core::movement::MovementMap;
use std::cell::Cell;
use std::collections::{BTreeMap, btree_map::Entry};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default)]
pub struct StackKindInputs {
    pub class: CounterClass,
    pub combat_role: u8,
    pub combat_count: u32,
    pub ordinary_path_total: u32,
    pub movement_target: u32,
    pub naval_transfer: bool,
    pub terrain_flags: u8,
    pub display_moving: bool,
    pub direction: Option<usize>,
}

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

pub fn snaps_to_stack(class: CounterClass, kind: u32, combat_role: u8, combat_count: u32) -> bool {
    let stand_off = match class {
        CounterClass::Division | CounterClass::TaskForce => matches!(kind, 9 | 10),
        CounterClass::RailwayGun => kind == 10,
    };
    let from_land = class.directional(true);
    stand_off
        || (combat_count != 0 && combat_role == 1 && (from_land..from_land + 8).contains(&kind))
}

pub const SETTLED_ARC: f32 = 1.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct UnitRoute<'a> {
    pub class: CounterClass,
    pub combat_role: u8,
    pub combat_count: u32,
    pub combat_province: u32,
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
        let class = self.class;
        let movement = Movement::build(
            province,
            self.display_path(),
            class,
            self.combat_count != 0,
            snaps_to_stack(class, kind, self.combat_role, self.combat_count),
            &|id| anchor(id, class.standstill(land(id))),
            world,
        );
        Some(Position {
            standing,
            motion: movement.as_ref().and_then(Movement::settled),
            movement: movement.map(Arc::new),
        })
    }
}

#[derive(Clone, Copy)]
pub struct RouteWorld<'a> {
    pub map: &'a MovementMap,
    pub heights: HeightField<'a>,
    pub width: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DivisionRoute {
    pub path: Vec<u32>,
    pub eta: Option<(i64, i64)>,
    pub naval_transfer: bool,
    pub retreat: bool,
    pub combat_role: u8,
    pub combat_count: u32,
    pub stack_kind: u32,
    pub kind_resolved: bool,
    pub idle: bool,
}

impl DivisionRoute {
    pub fn signature(&self) -> Option<[u32; 2]> {
        let next = *self.path.first()?;
        Some([next, self.path.get(1).copied().unwrap_or(0)])
    }

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

    pub fn position(
        &self,
        province: u32,
        world: Option<&RouteWorld<'_>>,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
    ) -> Option<Position> {
        let standing =
            || anchor(province, CounterClass::Division.standstill(false)).map(Position::still);
        let Some(world) = world else {
            return standing();
        };
        self.route()
            .position(province, self.stack_kind, world, anchor)
            .or_else(standing)
    }
}

#[derive(Clone, Debug, Default)]
pub struct DivisionRoutes {
    pub by_division: Arc<BTreeMap<u64, DivisionRoute>>,
    pub unresolved: usize,
    pub skipped_hops: usize,
}

struct Placed {
    province: u32,
    combat_role: u8,
    combat_count: u32,
    combat_province: u32,
    target: u32,
    naval_transfer: u8,
    path_total: u32,
    path: Vec<u32>,
    alternate_total: u32,
    alternate_path: Vec<u32>,
    placement: Placement,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    pub stack_kind: u32,
    pub display: Vec<u32>,
    pub kind_resolved: bool,
    pub skipped: usize,
    pub unresolved: bool,
}

impl UnitRoute<'_> {
    pub fn place(
        &self,
        province: u32,
        map: Option<&MovementMap>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
    ) -> Placement {
        let mut placement = self.resolve(province, map, anchor);
        if let Some(map) = map.filter(|_| !placement.display.is_empty()) {
            placement.skipped = walk_route(map, province, &placement.display, anchor).skipped;
        }
        placement
    }

    pub fn resolve(
        &self,
        province: u32,
        map: Option<&MovementMap>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
    ) -> Placement {
        let resolved = map.and_then(|map| self.stack_kind(province, map, anchor));
        match (map, resolved) {
            (Some(_), Some(kind)) => Placement {
                stack_kind: kind,
                display: self.display_path().to_vec(),
                kind_resolved: true,
                skipped: 0,
                unresolved: false,
            },
            _ => Placement {
                stack_kind: stack_kind(StackKindInputs {
                    combat_role: self.combat_role,
                    ..Default::default()
                }),
                display: Vec::new(),
                kind_resolved: self.combat_role == 2,
                skipped: 0,
                unresolved: !self.display_path().is_empty(),
            },
        }
    }
}

impl DivisionRoute {
    pub fn placed(
        route: &UnitRoute<'_>,
        retreat: bool,
        eta: Option<(i64, i64)>,
        placement: &Placement,
    ) -> Self {
        Self::of_placement(route, retreat, eta, placement.clone())
    }

    pub fn of_placement(
        route: &UnitRoute<'_>,
        retreat: bool,
        eta: Option<(i64, i64)>,
        placement: Placement,
    ) -> Self {
        Self {
            path: placement.display,
            eta,
            naval_transfer: route.naval_transfer,
            retreat,
            combat_role: route.combat_role,
            combat_count: route.combat_count,
            stack_kind: placement.stack_kind,
            kind_resolved: placement.kind_resolved,
            idle: crate::map::unit_rules::idle(route.path_total, route.combat_count),
        }
    }
}

impl Placed {
    fn new(
        row: &replay_data::rows::DivisionMovementRow,
        province: u32,
        map: Option<&MovementMap>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
    ) -> Self {
        let placement = UnitRoute::division(row).place(province, map, anchor);
        Self {
            province,
            combat_role: row.combat_role,
            combat_count: row.combat_count,
            combat_province: row.combat_province,
            target: row.target,
            naval_transfer: row.naval_transfer,
            path_total: row.path_total,
            path: row.path.clone(),
            alternate_total: row.alternate_total,
            alternate_path: row.alternate_path.clone(),
            placement,
        }
    }

    fn reads(&self, row: &replay_data::rows::DivisionMovementRow, province: u32) -> bool {
        self.province == province
            && self.combat_role == row.combat_role
            && self.combat_count == row.combat_count
            && self.combat_province == row.combat_province
            && self.target == row.target
            && self.naval_transfer == row.naval_transfer
            && self.path_total == row.path_total
            && self.alternate_total == row.alternate_total
            && self.path == row.path
            && self.alternate_path == row.alternate_path
    }
}

#[derive(Default)]
pub struct RouteMemo {
    hours: Mutex<Vec<HeldRoutes>>,
    placed: Mutex<std::collections::HashMap<u64, Arc<Placed>>>,
}

struct HeldRoutes {
    at: (replay_data::BranchId, replay_data::Hour),
    by_division: std::sync::Weak<BTreeMap<u64, DivisionRoute>>,
    unresolved: usize,
    skipped_hops: usize,
}

impl std::fmt::Debug for RouteMemo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RouteMemo")
    }
}

impl RouteMemo {
    pub fn sweep(&self, rows: usize, live: impl Fn(&u64) -> bool) {
        let mut placed = self.placed.lock().unwrap_or_else(|error| error.into_inner());
        if placed.len() > rows.saturating_mul(2) {
            placed.retain(|division, _| live(division));
        }
    }
}

impl DivisionRoutes {
    pub fn route_of(
        row: &replay_data::rows::DivisionMovementRow,
        province: u32,
        eta: Option<(i64, i64)>,
        map: Option<&MovementMap>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
        memo: &RouteMemo,
    ) -> DivisionRoute {
        let kept = memo
            .placed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&row.division)
            .filter(|kept| kept.reads(row, province))
            .cloned();
        let route = kept.unwrap_or_else(|| {
            let placed = Arc::new(Placed::new(row, province, map, anchor));
            memo.placed.lock().unwrap_or_else(|error| error.into_inner()).insert(row.division, Arc::clone(&placed));
            placed
        });
        DivisionRoute::placed(&UnitRoute::division(row), row.retreat != 0, eta, &route.placement)
    }

    pub fn read(
        frame: &replay_frame::frame::DecodedFrame,
        etas: &Etas,
        map: Option<&MovementMap>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
    ) -> std::io::Result<Self> {
        Self::read_placing(frame, etas, map, anchor, None)
    }

    pub fn read_kept(
        frame: &replay_frame::frame::DecodedFrame,
        etas: &Etas,
        map: Option<&MovementMap>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
        memo: &RouteMemo,
    ) -> std::io::Result<Self> {
        let key = (frame.branch, frame.hour);
        let held = memo
            .hours
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .find(|held| held.at == key)
            .and_then(|held| {
                Some(Self {
                    by_division: held.by_division.upgrade()?,
                    unresolved: held.unresolved,
                    skipped_hops: held.skipped_hops,
                })
            });
        if let Some(routes) = held {
            return Ok(routes);
        }
        let routes = Self::read_placing(frame, etas, map, anchor, Some(&memo.placed))?;
        let mut hours = memo.hours.lock().unwrap_or_else(|error| error.into_inner());
        hours.retain(|held| held.at != key && held.by_division.strong_count() > 0);
        hours.push(HeldRoutes {
            at: key,
            by_division: Arc::downgrade(&routes.by_division),
            unresolved: routes.unresolved,
            skipped_hops: routes.skipped_hops,
        });
        Ok(routes)
    }

    fn read_placing(
        frame: &replay_frame::frame::DecodedFrame,
        etas: &Etas,
        map: Option<&MovementMap>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
        memo: Option<&Mutex<std::collections::HashMap<u64, Arc<Placed>>>>,
    ) -> std::io::Result<Self> {
        use replay_data::rows::tables;
        let location: BTreeMap<u64, u32> = rows::<tables::Divisions>(frame)?
            .rows()
            .iter()
            .map(|row| (row.id, row.province))
            .collect();
        let movement = rows::<tables::DivisionMovement>(frame)?;
        let kept = memo
            .map(|memo| std::mem::take(&mut *memo.lock().unwrap_or_else(|error| error.into_inner())))
            .unwrap_or_default();
        let mut placed = std::collections::HashMap::with_capacity(if memo.is_some() {
            movement.rows().len()
        } else {
            0
        });
        let mut out = Self::default();
        let mut by_division = BTreeMap::new();
        for row in movement.rows() {
            let province = location.get(&row.division).copied().unwrap_or(0);
            let route = match kept.get(&row.division).filter(|kept| kept.reads(row, province)) {
                Some(kept) => Arc::clone(kept),
                None => Arc::new(Placed::new(row, province, map, anchor)),
            };
            out.skipped_hops += route.placement.skipped;
            out.unresolved += usize::from(route.placement.unresolved);
            by_division.insert(
                row.division,
                DivisionRoute::placed(
                    &UnitRoute::division(row),
                    row.retreat != 0,
                    etas.get(row.division),
                    &route.placement,
                ),
            );
            if memo.is_some() {
                placed.insert(row.division, route);
            }
        }
        if let Some(memo) = memo {
            *memo.lock().unwrap_or_else(|error| error.into_inner()) = placed;
        }
        out.by_division = Arc::new(by_division);
        Ok(out)
    }
}

pub const ENDPOINT_EXTENSION: f32 = 0.25;

pub const SUBDIVISIONS: u32 = 20;

const NEXT_ANCHOR_EPSILON: u32 = 0x3a83_126f;

const DEGENERATE_SEGMENT: f32 = 1e-4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub at: [f32; 3],
    pub arc: f32,
}

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

#[derive(Clone, Copy, Debug, PartialEq)]
struct Hop {
    from: [f32; 3],
    to: [f32; 3],
    crossing: Option<[i32; 2]>,
    first: bool,
    last: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct RouteWalk {
    hops: Vec<Hop>,
    skipped: usize,
}

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

fn segment(samples: &[Sample], a: usize, arc: f32) -> Option<usize> {
    if a == 0 || a >= samples.len() {
        return None;
    }
    Some((1..=a).find(|&index| samples[index].arc > arc).unwrap_or(a))
}

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    at: [f32; 3],
    heading: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RouteSample {
    pub motion: Option<Motion>,
    pub sample_count: usize,
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

    pub fn interpolate_scale(&self, from: f32, to: f32) -> f32 {
        (to - from) * self.scale_fraction + from
    }
}

impl Motion {
    #[cfg(test)]
    fn from_route(
        province: u32,
        path: &[u32],
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
        world: &RouteWorld<'_>,
    ) -> Option<Self> {
        Movement::build(
            province,
            path,
            CounterClass::Division,
            false,
            false,
            anchor,
            world,
        )?
        .settled()
    }

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

    fn from_hops(
        width: f32,
        hops: &[Hop],
        anchors: [[f32; 3]; 2],
        heights: &HeightField<'_>,
    ) -> Option<RouteSample> {
        let _timer = ProfileTimer::start();
        let points = route_control_points(width, hops);
        if points.len() < 4 {
            return Some(RouteSample::default());
        }
        let mut samples = spline(&points, SUBDIVISIONS);
        for sample in &mut samples {
            if !sample.at[0].is_finite() || !sample.at[2].is_finite() {
                return None;
            }
            sample.at[1] = heights.native_height(sample.at[0], sample.at[2]);
        }
        Some(RouteSample::from_samples(width, &samples, anchors))
    }

    fn settled(width: f32, samples: &[Sample], anchors: [[f32; 3]; 2]) -> Option<Self> {
        let next = unwrap_x(width, anchors[1], anchors[0]);
        let a = next_anchor_sample(width, samples, next);
        let index = segment(samples, a, SETTLED_ARC)?;
        Some(Self {
            at: point_on(samples, index, SETTLED_ARC),
            heading: heading_on(samples, index, width, &anchors),
        })
    }

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

    pub fn position_heading(&self) -> ([f32; 3], f32) {
        (self.at, self.heading)
    }
}

fn scale_fraction(next_arc: f32) -> f32 {
    if next_arc == 0.0 {
        1.0
    } else {
        (SETTLED_ARC / next_arc).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Debug, Default)]
pub struct MovedHours {
    pub divisions: BTreeMap<u64, Vec<(i32, u32)>>,
    pub task_forces: BTreeMap<u64, Vec<(i32, u32)>>,
}

pub static NO_MOVED_HOURS: MovedHours = MovedHours { divisions: BTreeMap::new(), task_forces: BTreeMap::new() };

impl MovedHours {
    pub fn of(&self, unit: Unit) -> &[(i32, u32)] {
        let held = match unit {
            Unit::Division(id) | Unit::RailwayGun(id) => self.divisions.get(&id),
            Unit::TaskForce(id) => self.task_forces.get(&id),
        };
        held.map_or(&[][..], Vec::as_slice)
    }

    pub fn note(&mut self, unit: Unit, hour: i32, province: u32) {
        let held = match unit {
            Unit::Division(id) | Unit::RailwayGun(id) => self.divisions.entry(id),
            Unit::TaskForce(id) => self.task_forces.entry(id),
        };
        held.or_default().push((hour, province));
    }

    pub fn extend(&mut self, newer: &Self) {
        for (held, more) in [(&mut self.divisions, &newer.divisions), (&mut self.task_forces, &newer.task_forces)] {
            for (id, hours) in more {
                held.entry(*id).or_default().extend_from_slice(hours);
            }
        }
    }

    pub fn after(&self, hour: i32) -> Self {
        let keep = |held: &BTreeMap<u64, Vec<(i32, u32)>>| -> BTreeMap<u64, Vec<(i32, u32)>> {
            held.iter()
                .filter_map(|(&id, hours)| {
                    let later: Vec<(i32, u32)> = hours.iter().copied().filter(|&(at, _)| at > hour).collect();
                    (!later.is_empty()).then_some((id, later))
                })
                .collect()
        };
        Self { divisions: keep(&self.divisions), task_forces: keep(&self.task_forces) }
    }

    pub fn counts(&self) -> (usize, usize) {
        let hours = self.divisions.values().chain(self.task_forces.values()).map(Vec::len).sum();
        (self.divisions.len() + self.task_forces.len(), hours)
    }
}

#[derive(Clone, Debug)]
pub struct Position {
    pub standing: [f32; 3],
    pub motion: Option<Motion>,
    pub movement: Option<Arc<Movement>>,
}

impl Position {
    pub fn still(standing: [f32; 3]) -> Self {
        Self {
            standing,
            motion: None,
            movement: None,
        }
    }

    pub fn at(&self) -> [f32; 3] {
        self.motion.as_ref().map_or(self.standing, Motion::position)
    }

    pub(crate) fn scale_route(&mut self, scales: [f32; 2]) {
        if let Some(movement) = &mut self.movement {
            Arc::make_mut(movement).scales = scales;
        }
    }
}

#[derive(Clone, Debug)]
pub struct Movement {
    province: u32,
    path: Box<[u32]>,
    class: CounterClass,
    combat: bool,
    snapped: bool,
    width: f32,
    anchors: [[f32; 3]; 2],
    samples: Vec<Sample>,
    next: usize,
    scales: [f32; 2],
}

impl Movement {
    #[allow(clippy::too_many_arguments)]
    fn build(
        province: u32,
        path: &[u32],
        class: CounterClass,
        combat: bool,
        snapped: bool,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
        world: &RouteWorld<'_>,
    ) -> Option<Self> {
        let _timer = ProfileTimer::start();
        let start = anchor(province)?;
        let next = path
            .first()
            .and_then(|&hop| anchor(hop))
            .unwrap_or_default();
        let mut samples = Vec::new();
        if !path.is_empty() {
            let walk = walk_route(world.map, province, path, anchor);
            let points = route_control_points(world.width, &walk.hops);
            if points.len() >= 4 {
                samples = spline(&points, SUBDIVISIONS);
            }
            for sample in &mut samples {
                if !sample.at[0].is_finite() || !sample.at[2].is_finite() {
                    return None;
                }
                sample.at[1] = world.heights.native_height(sample.at[0], sample.at[2]);
            }
        }
        Some(Self {
            next: next_anchor_sample(world.width, &samples, next),
            province,
            path: path.into(),
            class,
            combat,
            snapped,
            width: world.width,
            anchors: [start, next],
            samples,
            scales: [1.0; 2],
        })
    }

    fn settled(&self) -> Option<Motion> {
        (!self.snapped).then(|| self.at(SETTLED_ARC)).flatten()
    }

    pub fn place_in(
        &self,
        province: u32,
        world: &RouteWorld<'_>,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
    ) -> Option<[f32; 3]> {
        let land = |id: u32| world.map.terrain(id).is_some_and(|terrain| terrain.flags & 1 != 0);
        let class = self.class;
        let there = Self::build(
            province,
            &self.path,
            class,
            self.combat,
            self.snapped,
            &|id| anchor(id, class.standstill(land(id))),
            world,
        )?;
        Some(there.settled().map_or(there.anchors[0], |motion| motion.at))
    }

    fn at(&self, arc: f32) -> Option<Motion> {
        let index = segment(&self.samples, self.next, arc)?;
        Some(Motion {
            at: point_on(&self.samples, index, arc),
            heading: heading_on(&self.samples, index, self.width, &self.anchors),
        })
    }

    fn scale(&self, arc: f32) -> f32 {
        let end = self.samples.get(self.next).map_or(0.0, |sample| sample.arc);
        let fraction = if end == 0.0 {
            1.0
        } else {
            (arc / end).clamp(0.0, 1.0)
        };
        (self.scales[1] - self.scales[0]) * fraction + self.scales[0]
    }




    fn drawn(&self, arc: f32) -> Option<Drawn> {
        Some(Drawn {
            motion: self.at(arc)?,
            scale: self.scale(arc),
        })
    }

    fn moves(&self) -> bool {
        !self.samples.is_empty()
    }

    fn same_route(&self, other: &Self) -> bool {
        self.province == other.province && self.path == other.path && self.class == other.class
    }

    fn speed(&self, speeds: [f32; 2]) -> f32 {
        match self.class {
            CounterClass::TaskForce => speeds[1],
            _ if self.combat => 0.0,
            _ => speeds[0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    pub motion: Motion,
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementFrame {
    pub time: f32,
    pub hour: i32,
    pub head: crate::map::anchored::Head,
    pub overlay_hour: i32,
    pub clock_speed: Option<i32>,
    pub speeds: [f32; 2],
    pub steady: bool,
}

impl MovementFrame {
    pub fn clock_speed(frame: &replay_frame::frame::DecodedFrame) -> Option<i32> {
        let clock = rows::<replay_data::rows::tables::Clock>(frame).ok()?;
        clock
            .rows()
            .first()
            .filter(|_| clock.is_complete())
            .map(|clock| clock.speed)
    }
}

#[derive(Debug, Default)]
struct RenderDelta {
    times: [f32; 32],
    next: usize,
}

impl RenderDelta {
    fn step(&mut self, frame_time: f32) -> f32 {
        self.times[self.next] = frame_time;
        self.next = (self.next + 1) % self.times.len();
        self.times.sort_unstable_by(f32::total_cmp);
        let mean = self.times[3..29].iter().sum::<f32>() / 26.0;
        (mean * 0.5 + frame_time * 0.5).clamp(0.0, 0.5)
    }
}

pub type SharedMovement = Arc<Mutex<MovementHistory>>;

#[derive(Debug, Default)]
pub struct MovementHistory {
    frame: Option<MovementFrame>,
    render: RenderDelta,
    delta: f32,
    frames: u64,
    in_view: super::province_schedule::ProvinceSet,
    units: BTreeMap<Unit, Moving>,
    placed: BTreeMap<Unit, Placements>,
    largest_glide: f32,
}

#[derive(Debug, Default)]
struct Placements {
    held: Vec<crate::map::free_icons::Place>,
    handed: [Option<Handed>; 2],
    figured: Option<([f32; 3], u64)>,
}

impl Placements {
    fn note(&mut self, place: crate::map::free_icons::Place) {
        match self.held.binary_search_by_key(&place.hour, |held| held.hour) {
            Ok(at) => self.held[at] = place,
            Err(at) => self.held.insert(at, place),
        }
    }

    fn after_a_seek(&mut self, hour: i32) {
        if self.held.first().is_some_and(|oldest| oldest.hour > hour) {
            self.held.clear();
            self.handed = [None, None];
        }
    }

    fn keep(&mut self, head: i32) {
        let below = self.held.partition_point(|place| place.hour <= head);
        let drop = below.saturating_sub(1).min(self.held.len().saturating_sub(2));
        self.held.drain(..drop);
    }

    fn placed_at(&self, hour: i32) -> bool {
        self.held.iter().any(|place| place.hour == hour)
    }

    fn newest(&self) -> Option<crate::map::free_icons::Place> {
        self.held.last().copied()
    }

    fn track(&self, hour: i32, width: f32) -> Result<crate::map::free_icons::Track, &'static str> {
        let upto = self.held.partition_point(|place| place.hour <= hour);
        if upto < 2 {
            return Err("no publication before this one placed it");
        }
        let track = crate::map::free_icons::Track::new(&self.held[..upto], width);
        if !track.moves() {
            return Err("the recording left it where it was");
        }
        Ok(track)
    }

    fn hand(&mut self, hour: i32, track: Option<crate::map::free_icons::Track>) {
        let handed = Some(Handed { hour, track });
        if self.handed[1].as_ref().is_some_and(|held| held.hour == hour) {
            self.handed[1] = handed;
            return;
        }
        self.handed = [self.handed[1].take(), handed];
    }

    fn handed_at(&self, hour: i32) -> Option<Option<crate::map::free_icons::Track>> {
        self.handed
            .iter()
            .flatten()
            .find(|held| held.hour == hour)
            .map(|held| held.track.clone())
    }
}

#[derive(Clone, Debug)]
struct Handed {
    hour: i32,
    track: Option<crate::map::free_icons::Track>,
}

#[derive(Debug)]
struct Moving {
    shown: Arc<Movement>,
    current: Arc<Movement>,
    arc: f32,
    arriving: bool,
    deadline: Option<i64>,
    advanced: u64,
    hour: i32,
    drawn_at: Option<[f32; 3]>,
    checked: u64,
    rebuilt: bool,
    predicted: Option<(crate::map::free_icons::Track, i32)>,
}

impl Moving {
    fn new(input: &Arc<Movement>, hour: i32) -> Self {
        Self {
            shown: Arc::clone(input),
            current: Arc::clone(input),
            arc: 0.0,
            arriving: false,
            deadline: None,
            advanced: 0,
            hour,
            drawn_at: None,
            checked: 0,
            rebuilt: false,
            predicted: None,
        }
    }

    fn rebuild(&mut self) {
        self.shown = Arc::clone(&self.current);
        self.arc = 0.0;
        self.arriving = false;
        self.deadline = None;
        self.rebuilt = true;
    }

    fn offer(&mut self, input: &Arc<Movement>, day: i64, fastest: bool) {
        if !Arc::ptr_eq(&self.current, input) {
            self.current = Arc::clone(input);
        }
        if self.shown.same_route(input) {
            self.shown = Arc::clone(input);
            self.arriving = false;
            self.deadline = None;
        } else if self.shown.moves()
            && !input.combat
            && !fastest
            && self.shown.path.first() == Some(&input.province)
            && self.shown.path[1..] == *input.path
            && self.deadline.is_none_or(|deadline| day < deadline)
        {
            self.arriving = true;
        } else {
            self.rebuild();
        }
    }

    fn advance(&mut self, frame: MovementFrame, day: i64, fastest: bool, delta: f32) {
        let step = delta * self.current.speed(frame.speeds);
        if self.arriving {
            self.deadline.get_or_insert(day + 6);
            let end = self.shown.samples[self.shown.next].arc;
            self.arc = (self.arc + step).clamp(0.0, end);
            if self.arc == end {
                self.rebuild();
            }
        } else if self.shown.moves() {
            let least = if fastest { SETTLED_ARC } else { 0.0 };
            self.arc = (self.arc + step).clamp(least, SETTLED_ARC);
        }
    }
}

fn note_glide(state: &mut Moving, now: [f32; 3], step: f32, largest: &mut f32) {
    use crate::view_record::{add, set};
    let rebuilt = std::mem::take(&mut state.rebuilt);
    let Some(was) = state.drawn_at.replace(now) else {
        return;
    };
    let moved = (now[0] - was[0]).hypot(now[2] - was[2]);
    if moved > 0.0 {
        add("overlay", "counters_moved", 1);
    }
    *largest = largest.max(moved);
    set("overlay", "largest_glide_milli", (*largest * 1000.0) as u64);
    if rebuilt {
        add("overlay", "counter_rebuilds", 1);
    } else if moved > step + GLIDE_SLACK {
        add("overlay", "jumps", 1);
        add("overlay", "counter_jumps", 1);
    }
}

const GLIDE_SLACK: f32 = 1e-3;

fn note_prediction(state: &mut Moving, head: crate::map::anchored::Head, hour: i32, fresh: &crate::map::free_icons::Track) {
    use crate::view_record::add;
    let Some((held, was)) = state.predicted.clone() else {
        return;
    };
    if was == hour {
        return;
    }
    let from = held.places().last().map_or([0.0; 3], |place| place.at);
    let (there, here) = (held.at(head), fresh.at(head));
    let scope = if state.current.combat { "track combat" } else { "track free" };
    add(scope, "publications", 1);
    add(scope, "hours", u64::from(hour.saturating_sub(was).unsigned_abs()));
    add(scope, "behind_hours", u64::from(hour.saturating_sub(head.hour).max(0).unsigned_abs()));
    add(scope, "said_milli", (distance(from, there) * 1000.0) as u64);
    add(scope, "moved_milli", (distance(from, here) * 1000.0) as u64);
    add(scope, "apart_milli", (distance(there, here) * 1000.0) as u64);
}

const KEPT_HOURS: i32 = 240;

fn game_day(hour: i32) -> i64 {
    (i64::from(hour) - 43_800_000) / 24
}

impl MovementHistory {
    pub fn clear(&mut self) {
        self.units.clear();
        self.placed.clear();
    }

    pub fn begin_frame(&mut self, frame: MovementFrame, in_view: impl IntoIterator<Item = u32>) {
        let previous = self.frame.replace(frame);
        self.frames += 1;
        self.in_view.clear();
        for province in in_view {
            self.in_view.insert(province);
        }
        self.largest_glide = 0.0;
        let frame_time = previous.map_or(0.0, |previous| (frame.time - previous.time).max(0.0));
        self.delta = self.render.step(frame_time);
        if frame.steady
            || frame.clock_speed.is_none()
            || previous.is_some_and(|previous| frame.time < previous.time)
        {
            self.units.clear();
            self.placed.clear();
        } else if let Some(previous) = previous.filter(|previous| previous.hour != frame.hour) {
            self.units.retain(|_, state| state.hour == previous.hour);
            self.placed
                .retain(|_, places| places.newest().is_some_and(|place| (frame.hour - place.hour).abs() <= KEPT_HOURS));
        }
    }

    pub fn sample(&mut self, unit: Unit, position: &Position) -> Option<Drawn> {
        let frame = self.frame.filter(|frame| !frame.steady)?;
        let clock = frame.clock_speed?;
        let input = position.movement.as_ref()?;
        let state = match self.units.entry(unit) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) if input.moves() => entry.insert(Moving::new(input, frame.hour)),
            Entry::Vacant(_) => return None,
        };
        let (day, fastest) = (game_day(frame.hour), clock >= 4);
        state.offer(input, day, fastest);
        state.hour = frame.hour;
        if state.advanced != self.frames && self.in_view.contains(state.current.province) {
            state.advance(frame, day, fastest, self.delta);
            state.advanced = self.frames;
        }
        if !state.shown.moves() {
            self.units.remove(&unit);
            return None;
        }
        if input.snapped {
            if crate::view_record::on() {
                state.drawn_at = None;
                crate::view_record::add("overlay", "counter_snaps", 1);
            }
            return None;
        }
        let drawn = state.shown.drawn(state.arc)?;
        if crate::view_record::on() && state.checked != self.frames {
            state.checked = self.frames;
            let step = self.delta * state.current.speed(frame.speeds);
            note_glide(state, drawn.motion.position(), step, &mut self.largest_glide);
        }
        let settled = position.motion.map(|motion| Drawn {
            motion,
            scale: input.scale(SETTLED_ARC),
        });
        (Some(drawn) != settled).then_some(drawn)
    }

    pub fn track(&mut self, unit: Unit, places: &[crate::map::free_icons::Place], hour: i32, width: f32) -> Option<crate::map::free_icons::Track> {
        let refused = |why: &'static str| {
            if crate::map::overlay_count::counting() {
                crate::view_record::add("track refused", why, 1);
            }
            None
        };
        let Some(frame) = self.frame.filter(|frame| !frame.steady) else {
            return refused("the frame is steady or none");
        };
        let held = self.placed.entry(unit).or_default();
        held.after_a_seek(hour);
        for place in places {
            held.note(*place);
        }
        held.keep(frame.head.hour);
        if crate::view_record::on() {
            crate::view_record::add("places", "noted", places.len() as u64);
            crate::view_record::add("places", "held", held.held.len() as u64);
            crate::view_record::add("places", "movers", 1);
        }
        let made = held.track(hour, width);
        held.hand(hour, made.clone().ok());
        let track = match made {
            Ok(track) => track,
            Err(why) => return refused(why),
        };
        if crate::view_record::on()
            && let Some(state) = self.units.get_mut(&unit)
        {
            note_prediction(state, frame.head, hour, &track);
            state.predicted = Some((track.clone(), hour));
        }
        Some(track)
    }

    pub fn at_head(&mut self, unit: Unit, position: &Position) -> Option<AtHead> {
        let drawn = self.sample(unit, position);
        let standing = || {
            drawn.map(|drawn| {
                let (at, heading) = drawn.motion.position_heading();
                AtHead { at, heading: Some(heading), scale: Some(drawn.scale) }
            })
        };
        let frame = self.frame.filter(|frame| !frame.steady)?;
        frame.clock_speed?;
        let frames = self.frames;
        let on_the_hour = |places: Option<&mut Placements>| {
            if crate::view_record::on() {
                crate::view_record::add("figures", "on_the_hour", 1);
            }
            if let Some(places) = places {
                places.figured = None;
            }
        };
        let Some(places) = self.placed.get_mut(&unit) else {
            on_the_hour(None);
            return standing();
        };
        let Some(track) = places.handed_at(frame.overlay_hour) else {
            if judging() && places.placed_at(frame.overlay_hour) {
                judge_the_layout(unit, frame.overlay_hour, places.handed[1].as_ref().map(|held| held.hour));
            }
            on_the_hour(Some(places));
            return standing();
        };
        let Some(track) = track else {
            on_the_hour(Some(places));
            return standing();
        };
        let at = track.at(frame.head);
        if crate::view_record::on() {
            note_figure(places, frames, at, track.outside(frame.head), track.held_still(frame.head));
        }
        if judging()
            && let Ok(fresh) = places.track(frame.overlay_hour, track.width)
        {
            judge_one_point(unit, fresh.at(frame.head), at);
        }
        Some(AtHead { at, heading: None, scale: drawn.map(|drawn| drawn.scale) })
    }

    pub fn delta(&self) -> Option<f32> {
        self.frame.filter(|frame| !frame.steady).map(|_| self.delta)
    }

    pub fn clock_unavailable(&self) -> bool {
        self.frame
            .is_some_and(|frame| !frame.steady && frame.clock_speed.is_none())
    }

    pub fn running(&self) -> bool {
        let Some(frame) = self.frame.filter(|frame| !frame.steady) else {
            return false;
        };
        self.units.values().any(|state| {
            state.advanced == self.frames
                && state.current.speed(frame.speeds) > 0.0
                && (state.arriving || (state.shown.moves() && state.arc < SETTLED_ARC))
        })
    }
}

fn note_figure(places: &mut Placements, frames: u64, at: [f32; 3], outside: i8, held_still: bool) {
    use crate::view_record::add;
    add("figures", "on_places", 1);
    match outside {
        -1 => add("figures", "before_the_oldest", 1),
        1 => add("figures", "past_the_newest", 1),
        _ => {}
    }
    if let Some((was, frame)) = places.figured.replace((at, frames))
        && frame + 1 == frames
    {
        let moved = distance(was, at);
        add("figures", if moved > 0.0 { "moved" } else { "stood" }, 1);
        if moved == 0.0 {
            add("figures", if held_still { "stood_held_still" } else { "stood_though_moved" }, 1);
        }
        add("figures", "moved_milli", (moved * 1000.0) as u64);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtHead {
    pub at: [f32; 3],
    pub heading: Option<f32>,
    pub scale: Option<f32>,
}

fn judging() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("BATTLEPLAN_VIEWER_CHECK_ACTORS").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_ACTORS"))
            .and_then(|value| value.to_str().map(|value| value.trim().to_owned()))
            .is_some_and(|value| !value.is_empty() && value != "0")
    })
}

const JUDGE_SLACK: f32 = 0.065;

fn judge_one_point(unit: Unit, places: [f32; 3], figure: [f32; 3]) {
    use crate::view_record::{add, add_named};
    add("actors check", "checks", 1);
    add_named("actors check", "checked: one point", 1);
    let apart = distance(places, figure);
    if !(apart <= JUDGE_SLACK) {
        eprintln!(
            "CHECK one point DIFFERS at {unit:?}: a figure and its counter's own places stand {apart:.4} world units apart, where {JUDGE_SLACK} is allowed"
        );
        add("actors check", "differs", 1);
        add_named("actors check", "differs: one point", 1);
    }
}

fn judge_the_layout(unit: Unit, drawn: i32, handed: Option<i32>) {
    use crate::view_record::{add, add_named};
    eprintln!(
        "CHECK one track DIFFERS at {unit:?}: the layout this frame draws stands at hour {drawn}, and the newest track handed for it was cut at {handed:?}"
    );
    add("actors check", "differs", 1);
    add_named("actors check", "differs: one track", 1);
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
