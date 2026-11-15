//! Unit movement positions for counters and figures.
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
//! Each hour places units at the settled arc ([`Position`]); a view's
//! [`MovementHistory`] runs each unit's animator from its anchor and carries the free arc and
//! arrival glide across hours on the game's render delta.

use crate::map::movement_routes::catmull_rom;
use crate::map::province_map::HeightField;
use crate::map::tables::rows;
use crate::movement_timing::Etas;
use crate::unit_class::CounterClass;
use crate::unit_handle::Unit;
use installed_content_core::movement::MovementMap;
use std::cell::Cell;
use std::collections::{BTreeMap, btree_map::Entry};
use std::sync::{Arc, Mutex};
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
#[derive(Clone, Debug, Default, PartialEq)]
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

/// This hour's division routes and how many the content map could not place.
#[derive(Clone, Debug, Default)]
pub struct DivisionRoutes {
    pub by_division: Arc<BTreeMap<u64, DivisionRoute>>,
    /// Moving divisions whose route the content map cannot place; they stand
    /// on their province and are counted, never dropped.
    pub unresolved: usize,
    /// Later route hops the placed routes' polylines skip ([`walk_route`]):
    /// no content neighbour of the province before them, or no anchor.
    pub skipped_hops: usize,
}

/// One division's placed route and the movement fields and province it was placed from.
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

/// A division's route placed on the content from `province`: its stack kind, the display route the
/// content can place (empty when it cannot), whether the kind is the native one, the hops the
/// polyline skips, and whether a moving route could not be placed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    pub stack_kind: u32,
    pub display: Vec<u32>,
    pub kind_resolved: bool,
    pub skipped: usize,
    pub unresolved: bool,
}

impl UnitRoute<'_> {
    /// Where this route stands from `province` on `map`; `anchor` is a province's standstill
    /// anchor.
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

    /// [`Self::place`] without walking the route: the hops its polyline skips are not counted.
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
                // An unplaceable route stands still on its province; the
                // stand-off kinds need no geometry.
                stack_kind: stack_kind(StackKindInputs {
                    combat_role: self.combat_role,
                    ..Default::default()
                }),
                display: Vec::new(),
                // Defending in its first combat is kind 10 whatever the
                // route, so only that fallback is native.
                kind_resolved: self.combat_role == 2,
                skipped: 0,
                unresolved: !self.display_path().is_empty(),
            },
        }
    }
}

impl DivisionRoute {
    /// The route a division with movement `route`, retreating when `retreat`, takes once
    /// `placement` placed it.
    pub fn placed(
        route: &UnitRoute<'_>,
        retreat: bool,
        eta: Option<(i64, i64)>,
        placement: &Placement,
    ) -> Self {
        Self::of_placement(route, retreat, eta, placement.clone())
    }

    /// [`Self::placed`], taking the placement's display route.
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

    /// Whether it was placed from exactly what `row` and `province` hold ([`UnitRoute::division`]).
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

/// The content's division routes: each hour's whole while a reader holds its routes, shared
/// by the hour's readers, and each division's placed route, kept while the fields it reads
/// are unchanged.
#[derive(Default)]
pub struct RouteMemo {
    hours: Mutex<Vec<HeldRoutes>>,
    placed: Mutex<std::collections::HashMap<u64, Arc<Placed>>>,
}

/// An hour's routes, held while a reader keeps their division map.
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
    /// Drops the kept placements of divisions `live` no longer names (no movement row this hour),
    /// once they number more than twice the hour's movement rows (`rows`): a sweep by what the
    /// hour holds, never a bound on how many are kept.
    pub fn sweep(&self, rows: usize, live: impl Fn(&u64) -> bool) {
        let mut placed = self.placed.lock().unwrap_or_else(|error| error.into_inner());
        if placed.len() > rows.saturating_mul(2) {
            placed.retain(|division, _| live(division));
        }
    }
}

impl DivisionRoutes {
    /// One division's route as [`Self::read_kept`] places it: its kept placement while
    /// [`Placed::reads`] holds, else placed anew and kept, with its estimate `eta`. For a caller
    /// that reads a few divisions; the memo is locked only to look and to keep.
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

    /// `anchor` is a province's `Standstill` anchor: a route through a province
    /// without one is not placed.
    pub fn read(
        frame: &replay_frame::frame::DecodedFrame,
        etas: &Etas,
        map: Option<&MovementMap>,
        anchor: &dyn Fn(u32) -> Option<[f32; 3]>,
    ) -> std::io::Result<Self> {
        Self::read_placing(frame, etas, map, anchor, None)
    }

    /// [`Self::read`] through `memo`, which must be for this `map` and `anchor`'s content.
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
        // Taken whole, so concurrent hours never wait; the last to finish keeps its own.
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
    /// A moving division's settled point, as [`UnitRoute::position`] builds it.
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

///// An anchored stack: where the hour places it, and the route its presentation glides along.
#[derive(Clone, Debug)]
pub struct Position {
    pub standing: [f32; 3],
    /// The settled point on the route, `None` standing on the anchor.
    pub motion: Option<Motion>,
    pub movement: Option<Arc<Movement>>,
}

impl Position {
    /// A stack without a route.
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

    /// The stack scales at the unit's province and its next province, which the route
    /// interpolates as it glides.
    pub(crate) fn scale_route(&mut self, scales: [f32; 2]) {
        if let Some(movement) = &mut self.movement {
            Arc::make_mut(movement).scales = scales;
        }
    }
}

/// One unit's animator route this hour, which its presentation history compares and glides
/// along. A snapped stack keeps its route but stands on its stack anchor.
#[derive(Clone, Debug)]
pub struct Movement {
    province: u32,
    path: Box<[u32]>,
    class: CounterClass,
    combat: bool,
    snapped: bool,
    width: f32,
    /// The unit's province's anchor and the next province's.
    anchors: [[f32; 3]; 2],
    /// Empty for a route of fewer than four controls.
    samples: Vec<Sample>,
    /// The sample at the next province's anchor.
    next: usize,
    /// The stack scales the route interpolates from and to; one for land units.
    scales: [f32; 2],
}

impl Movement {
    /// The game's route from `province` along the display `path` ([`walk_route`]). The neighbour
    /// crossing and the height callback are presentation geometry, never recorded. `A` is
    /// searched against the next province's anchor whatever the walk skipped, the origin for a
    /// province without one, as the native reads a null province's.
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
            // Fewer than four controls clear the route: not moving.
            if points.len() >= 4 {
                samples = spline(&points, SUBDIVISIONS);
            }
            // Arc length is accumulated on the flat controls before terrain height is sampled.
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

    /// Where the hour places a stack that does not snap.
    fn settled(&self) -> Option<Motion> {
        (!self.snapped).then(|| self.at(SETTLED_ARC)).flatten()
    }

    fn at(&self, arc: f32) -> Option<Motion> {
        let index = segment(&self.samples, self.next, arc)?;
        Some(Motion {
            at: point_on(&self.samples, index, arc),
            heading: heading_on(&self.samples, index, self.width, &self.anchors),
        })
    }

    /// The stack scale at `arc`, interpolated over the next anchor's arc.
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

    /// The same unit on the same route, placed by another layer or a later hour. Combat only
    /// changes the route's speed and whether the stack snaps.
    fn same_route(&self, other: &Self) -> bool {
        self.province == other.province && self.path == other.path && self.class == other.class
    }

    /// The free arc's speed; a land unit in combat has none.
    fn speed(&self, speeds: [f32; 2]) -> f32 {
        match self.class {
            CounterClass::TaskForce => speeds[1],
            _ if self.combat => 0.0,
            _ => speeds[0],
        }
    }
}

/// Where a unit is drawn this frame along the route its history shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    pub motion: Motion,
    /// The stack scale along that route.
    pub scale: f32,
}

/// One view's movement clock, set by its frame before any layer reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementFrame {
    /// Presentation seconds.
    pub time: f32,
    /// The presented replay hour.
    pub hour: i32,
    /// The recorded game-speed index; without it every unit stays settled.
    pub clock_speed: Option<i32>,
    /// `LAND_UNIT_MOVEMENT_SPEED` and `NAVAL_UNIT_MOVEMENT_SPEED`.
    pub speeds: [f32; 2],
    /// Every unit settled, as window dumps draw them.
    pub steady: bool,
}

impl MovementFrame {
    /// The presented frame's recorded game speed.
    pub fn clock_speed(frame: &replay_frame::frame::DecodedFrame) -> Option<i32> {
        let clock = rows::<replay_data::rows::tables::Clock>(frame).ok()?;
        clock
            .rows()
            .first()
            .filter(|_| clock.is_complete())
            .map(|clock| clock.speed)
    }
}

/// The map's render delta: half the mean of the middle 26 of the last 32 frame times, sorted in
/// place, plus half this frame's, within [0, 0.5] seconds.
#[derive(Debug, Default)]
struct RenderDelta {
    times: [f32; 32],
    next: usize,
}

impl RenderDelta {
    fn step(&mut self, frame_time: f32) -> f32 {
        self.times[self.next] = frame_time;
        self.next = (self.next + 1) % self.times.len();
        // The game writes the next time over whatever the sort left at the cursor.
        self.times.sort_unstable_by(f32::total_cmp);
        let mean = self.times[3..29].iter().sum::<f32>() / 26.0;
        (mean * 0.5 + frame_time * 0.5).clamp(0.0, 0.5)
    }
}

pub type SharedMovement = Arc<Mutex<MovementHistory>>;

/// One view's unit animators: each moving unit's free arc and the route it is drawn on, shared
/// by its counter and its figure, and advanced once on each frame its province updates in view.
#[derive(Debug, Default)]
pub struct MovementHistory {
    frame: Option<MovementFrame>,
    render: RenderDelta,
    delta: f32,
    frames: u64,
    /// The provinces this frame updates in view, one bit per province id.
    in_view: Vec<u64>,
    units: BTreeMap<Unit, Moving>,
    /// The largest glide of a unit this frame, while the flight record is kept ([`note_glide`]).
    largest_glide: f32,
}

#[derive(Debug)]
struct Moving {
    /// The route drawn: the previous hour's while the unit glides into its new province.
    shown: Arc<Movement>,
    /// This hour's route.
    current: Arc<Movement>,
    arc: f32,
    arriving: bool,
    /// The game day the arrival glide gives up at, from its first advance.
    deadline: Option<i64>,
    /// The frame that last advanced it.
    advanced: u64,
    /// The hour a layer last placed it.
    hour: i32,
    /// Where it was drawn at the last frame the flight record checked it, and that frame.
    drawn_at: Option<[f32; 3]>,
    checked: u64,
    /// The game rebuilt it onto its route since that frame: a snap of the game's own.
    rebuilt: bool,
}

impl Moving {
    fn new(input: &Arc<Movement>, hour: i32) -> Self {
        Self {
            shown: Arc::clone(input),
            current: Arc::clone(input),
            // A new animator starts at its anchor, as the naval one does.
            arc: 0.0,
            arriving: false,
            deadline: None,
            advanced: 0,
            hour,
            drawn_at: None,
            checked: 0,
            rebuilt: false,
        }
    }

    fn rebuild(&mut self) {
        self.shown = Arc::clone(&self.current);
        self.arc = 0.0;
        self.arriving = false;
        self.deadline = None;
        self.rebuilt = true;
    }

    /// This hour's route: it continues the drawn route, glides along it into the next province,
    /// or rebuilds the animator at zero.
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
            // Reaching the next anchor rebuilds at zero, without a second advance.
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

/// The flight record's check of one drawn unit against the frame before: how far it moved on the
/// map, and whether that is more than its animator's free arc advanced this frame (`step`, its
/// speed times the render delta). A chord is never longer than its arc, so a glide moves at most
/// `step`; a rebuild is the game's own snap onto a new route and is counted apart.
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

/// Float rounding of a drawn point, in world units.
const GLIDE_SLACK: f32 = 1e-3;

/// The game's zero-based day of a replay hour.
fn game_day(hour: i32) -> i64 {
    (i64::from(hour) - 43_800_000) / 24
}

impl MovementHistory {
    pub fn clear(&mut self) {
        self.units.clear();
    }

    /// Starts a frame whose province updates reach `in_view` in view.
    pub fn begin_frame(&mut self, frame: MovementFrame, in_view: impl IntoIterator<Item = u32>) {
        let previous = self.frame.replace(frame);
        self.frames += 1;
        self.in_view.fill(0);
        for province in in_view {
            let word = province as usize / 64;
            if word >= self.in_view.len() {
                self.in_view.resize(word + 1, 0);
            }
            self.in_view[word] |= 1 << (province % 64);
        }
        self.largest_glide = 0.0;
        let frame_time = previous.map_or(0.0, |previous| (frame.time - previous.time).max(0.0));
        self.delta = self.render.step(frame_time);
        if frame.steady
            || frame.clock_speed.is_none()
            || previous.is_some_and(|previous| frame.time < previous.time)
        {
            self.units.clear();
        } else if let Some(previous) = previous.filter(|previous| previous.hour != frame.hour) {
            // A unit no layer placed in the previous hour has left the view.
            self.units.retain(|_, state| state.hour == previous.hour);
        }
    }

    /// Where `unit` is drawn this frame when its animator moves it off the hour's placement
    /// (`position`); `None` draws it there. The first layer to place it this frame offers this
    /// hour's route, then advances it when its province updates in view, as the game installs
    /// routes before its province updates.
    pub fn sample(&mut self, unit: Unit, position: &Position) -> Option<Drawn> {
        let frame = self.frame.filter(|frame| !frame.steady)?;
        let clock = frame.clock_speed?;
        // A layer placing the unit without a route leaves the animator to the layers that route it.
        let input = position.movement.as_ref()?;
        let state = match self.units.entry(unit) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) if input.moves() => entry.insert(Moving::new(input, frame.hour)),
            Entry::Vacant(_) => return None,
        };
        let (day, fastest) = (game_day(frame.hour), clock >= 4);
        state.offer(input, day, fastest);
        state.hour = frame.hour;
        if state.advanced != self.frames
            && self
                .in_view
                .get(state.current.province as usize / 64)
                .is_some_and(|word| word >> (state.current.province % 64) & 1 != 0)
        {
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

    /// This frame's map render delta, which every animator of the view advances by.
    pub fn delta(&self) -> Option<f32> {
        self.frame.filter(|frame| !frame.steady).map(|_| self.delta)
    }

    /// Whether this frame leaves moving units settled for want of the recorded game speed.
    pub fn clock_unavailable(&self) -> bool {
        self.frame
            .is_some_and(|frame| !frame.steady && frame.clock_speed.is_none())
    }

    /// Whether a unit advanced this frame is still moving, so the view draws another.
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
