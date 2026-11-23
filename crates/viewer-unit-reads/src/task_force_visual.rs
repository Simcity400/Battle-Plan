
#[cfg(test)]
use viewer_gpu::province_map::HeightField;
use std::{collections::BTreeMap, io};

use replay_data::{
    rows::TaskForceCounterInputRow, rows::TaskForceMapMotionRow, rows::TaskForceRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;

use crate::counter_move::{
    self,
    Motion,
    Position,
    RouteSample,
    RouteWorld,
    StackKindInputs,
    UnitRoute,
};
use viewer_foundation::{
    tables::rows,
    unit_rules::{ATTACKING, DEFENDING, PATROL, mission_active},
    movement_timing::MovementContent,
};
use viewer_runtime::unit_class::CounterClass;

#[derive(Clone, Debug, Default)]
pub struct TaskForceMotions {
    rows: Option<BTreeMap<u64, TaskForceMapMotionRow>>,
}

#[derive(Clone, Debug)]
pub struct Placement {
    pub kind: u32,
    pub position: Position,
    pub animator: Option<RouteSample>,
    pub unplaced_route: Option<&'static str>,
}

impl TaskForceMotions {
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let table = rows::<tables::TaskForceMapMotion>(frame)?;
        if !table.is_complete() {
            return Ok(Self::default());
        }
        Ok(Self {
            rows: Some(
                table
                    .rows()
                    .iter()
                    .map(|row| (row.task_force, row.clone()))
                    .collect(),
            ),
        })
    }

    pub fn row(&self, force: u64) -> Result<&TaskForceMapMotionRow, &'static str> {
        self.rows
            .as_ref()
            .ok_or("task-force motion census incomplete")?
            .get(&force)
            .ok_or("task-force motion row absent")
    }

    pub fn visual(
        &self,
        force: &TaskForceRow,
        movement: Option<&MovementContent>,
        world: Option<&RouteWorld<'_>>,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
        land: &dyn Fn(u32) -> bool,
        scale: &dyn Fn(u32, u32) -> f32,
    ) -> Result<Placement, &'static str> {
        let row = self.row(force.id)?;
        visual_of(force.province, &UnitRoute::task_force(row), movement, world, anchor, land, scale)
    }

    pub fn settled_origin(
        &self,
        force: &TaskForceRow,
        movement: Option<&MovementContent>,
        world: Option<&RouteWorld<'_>>,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
        land: &dyn Fn(u32) -> bool,
    ) -> Result<[f32; 3], &'static str> {
        let placed = self.visual(force, movement, world, anchor, land, &|_, _| 1.0)?;
        if let Some(reason) = placed.unplaced_route {
            return Err(reason);
        }
        Ok(placed.position.at())
    }

    pub fn docked(&self, force: u64) -> Result<bool, &'static str> {
        self.row(force).map(|row| row.path.is_empty())
    }
}

pub fn visual_of(
    province: u32,
    route: &UnitRoute<'_>,
    movement: Option<&MovementContent>,
    world: Option<&RouteWorld<'_>>,
    anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
    land: &dyn Fn(u32) -> bool,
    scale: &dyn Fn(u32, u32) -> f32,
) -> Result<Placement, &'static str> {
    let class = CounterClass::TaskForce;
    let animator = if route.display_path().is_empty() {
        Some(RouteSample::default())
    } else {
        world.and_then(|world| {
            Motion::route_sample(
                province,
                route.display_path(),
                &|id| anchor(id, class.standstill(land(id))),
                world,
            )
        })
    };
    let placed = movement.and_then(|movement| {
        animator.and_then(|animator| {
            route.stack_kind_from_animator(province, &movement.map, animator.sample_count)
        })
    });
    let here = land(province);
    let kind = placed.unwrap_or_else(|| {
        counter_move::stack_kind(StackKindInputs {
            class,
            combat_role: route.combat_role,
            combat_count: route.combat_count,
            terrain_flags: u8::from(here),
            ..Default::default()
        })
    });
    let mut position = world
        .and_then(|world| route.position(province, kind, world, anchor))
        .or_else(|| {
            route
                .standing(province, here, kind, anchor)
                .map(Position::still)
        })
        .ok_or("task-force province has no unit-stack anchor")?;
    let standing_scale = |province| scale(province, class.standstill(land(province)));
    position.scale_route([
        standing_scale(province),
        route
            .display_path()
            .first()
            .map_or(1.0, |&next| standing_scale(next)),
    ]);
    Ok(Placement {
        kind,
        position,
        animator,
        unplaced_route: (placed.is_none() && !route.display_path().is_empty()).then_some(
            if movement.is_none() {
                "the content movement map is not loaded"
            } else {
                "the content movement map cannot place its route (a province without terrain, neighbours or a stack anchor, or a first step to a province that is not a neighbour)"
            },
        ),
    })
}

#[derive(Clone, Debug, Default)]
pub struct MissionSkips {
    inputs: BTreeMap<u64, (u8, u32)>,
    current_regions: Option<BTreeMap<u64, u32>>,
}

impl MissionSkips {
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let missions = rows::<tables::TaskForceMissions>(frame)?;
        Ok(Self {
            inputs: rows::<tables::TaskForceCounterInputs>(frame)?
                .rows()
                .iter()
                .map(|row: &TaskForceCounterInputRow| {
                    (row.task_force, (row.is_in_regions, row.bombardment_region))
                })
                .collect(),
            current_regions: missions.is_complete().then(|| {
                missions
                    .rows()
                    .iter()
                    .map(|row| (row.task_force, row.current_region))
                    .collect()
            }),
        })
    }

    pub fn skip(&self, force: &TaskForceRow, kind: u32, region: Option<u32>) -> Option<bool> {
        let current = self.current_regions.as_ref().map(|regions| regions.get(&force.id).copied().unwrap_or(0));
        skip_of(kind, self.inputs.get(&force.id).copied(), force.mission, current, region)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture(
        inputs: impl IntoIterator<Item = (u64, (u8, u32))>,
        current_regions: impl IntoIterator<Item = (u64, u32)>,
    ) -> Self {
        Self {
            inputs: inputs.into_iter().collect(),
            current_regions: Some(current_regions.into_iter().collect()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mission_skip_follows_the_native_predicate() {
        let force = |id, mission| TaskForceRow {
            id,
            mission,
            ..Default::default()
        };
        let skips = MissionSkips::fixture(
            [
                (1, (1, 0)),
                (2, (0, 0)),
                (3, (1, 12)),
                (4, (1, 0)),
                (5, (1, 0)),
            ],
            [(1, 5)],
        );
        assert_eq!(skips.skip(&force(1, 2), 0, None), Some(false));
        assert_eq!(skips.skip(&force(1, 4), 0, None), Some(true));
        assert_eq!(skips.skip(&force(2, 4), 0, None), Some(false));
        assert_eq!(skips.skip(&force(4, 9), 0, None), Some(false));
        assert_eq!(skips.skip(&force(3, 9), 0, None), Some(true));
        assert_eq!(skips.skip(&force(1, PATROL), 0, Some(5)), Some(true));
        assert_eq!(skips.skip(&force(1, PATROL), 0, Some(6)), Some(false));
        assert_eq!(skips.skip(&force(5, PATROL), 0, None), Some(false));
        for kind in [ATTACKING, DEFENDING] {
            assert_eq!(skips.skip(&force(1, PATROL), kind, Some(5)), Some(false));
        }
        assert_eq!(skips.skip(&force(9, 4), 0, None), None);
        let incomplete = MissionSkips {
            current_regions: None,
            ..skips
        };
        assert_eq!(incomplete.skip(&force(1, PATROL), 0, Some(5)), None);
    }

    fn motions(row: TaskForceMapMotionRow) -> TaskForceMotions {
        TaskForceMotions {
            rows: Some(BTreeMap::from([(row.task_force, row)])),
        }
    }

    #[test]
    fn partial_invalid_route_uses_the_sample_count_for_port_admission() {
        use content_reader_map::movement::{MovementMap, Neighbour, ProvinceTerrain};
        let terrain = ProvinceTerrain { flags: 1, index: 1 };
        let neighbours = |to: u32| {
            vec![Neighbour {
                to,
                distance: Some(100_000),
                ..Default::default()
            }]
        };
        let map = MovementMap::from_parts(&[
            (1, terrain, neighbours(2)),
            (2, terrain, neighbours(1)),
            (9, terrain, Vec::new()),
        ]);
        let movement = MovementContent {
            map,
            defines: replay_derive::eta_derive::Defines {
                embark_distance: 0,
                transport_scale: 0,
                invasion_speed: 0,
                outer_scale: 0,
                river_crossing_two: 0,
                river_crossing_three: 0,
                organisation: None,
                rail: None,
                noncore_supply: [None; 2],
                combat_movement: None,
            },
            rail: Ok(content_reader_map::movement::RailMovementDefines {
                speed: [0; 4],
                maximum_level: 0,
            }),
            task_force_terrain: Err("unrelated task-force terrain is unavailable".into()),
            land: Err("unrelated land movement terrain is unavailable".into()),
        };
        let world = RouteWorld {
            map: &movement.map,
            heights: HeightField {
                width: 2,
                height: 2,
                values: &[0; 4],
            },
            width: 5632.0,
        };
        let force = TaskForceRow {
            id: 7,
            province: 1,
            ..Default::default()
        };
        let anchor = |id: u32, _: u32| Some([id as f32 * 10.0, 0.0, 0.0]);
        let partial = motions(TaskForceMapMotionRow {
            task_force: 7,
            target: 9,
            path: vec![2, 9],
            ..Default::default()
        });
        let placed = partial
            .visual(&force, Some(&movement), Some(&world), &anchor, &|_| true, &|_, _| 1.0)
            .unwrap();
        assert_eq!(placed.kind, 19);
        assert_eq!(placed.animator.unwrap().sample_count, 0);
        assert!(placed.position.motion.is_none());
        assert!(placed.unplaced_route.is_none());

        let valid = motions(TaskForceMapMotionRow {
            task_force: 7,
            target: 2,
            path: vec![2],
            ..Default::default()
        });
        let placed = valid
            .visual(&force, Some(&movement), Some(&world), &anchor, &|_| true, &|_, _| 1.0)
            .unwrap();
        assert_eq!(placed.kind, 20);
        assert!(placed.animator.unwrap().sample_count != 0);
        assert!(placed.position.motion.is_some());
    }

    #[test]
    fn the_route_is_class_one_and_prefers_the_alternate_display_path() {
        let mut row = TaskForceMapMotionRow {
            path: vec![2, 3],
            ..Default::default()
        };
        let unit = UnitRoute::task_force(&row);
        assert_eq!(unit.class, CounterClass::TaskForce);
        assert_eq!((unit.combat_role, unit.combat_count), (0, 0));
        assert_eq!(unit.display_path(), &[2, 3]);
        row.alternate_path = vec![4];
        assert_eq!(UnitRoute::task_force(&row).display_path(), &[4]);
        assert_eq!(UnitRoute::task_force(&row).path_total, 2);
    }

    #[test]
    fn standing_task_forces_dock_on_land_and_fight_at_their_stand_off_kind() {
        let force = TaskForceRow {
            id: 7,
            province: 5,
            ..Default::default()
        };
        let anchor = |province: u32, slot: u32| Some([province as f32, 0.0, slot as f32]);
        let (sea, land) = (|_| false, |_| true);
        let standing = motions(TaskForceMapMotionRow {
            task_force: 7,
            ..Default::default()
        });
        let at_sea = standing.visual(&force, None, None, &anchor, &sea, &|_, _| 1.0).unwrap();
        assert_eq!(
            (at_sea.kind, at_sea.position.standing),
            (0, [5.0, 0.0, 0.0])
        );
        assert!(at_sea.position.motion.is_none() && at_sea.unplaced_route.is_none());
        assert_eq!(
            standing
                .visual(&force, None, None, &anchor, &land, &|_, _| 1.0)
                .unwrap()
                .kind,
            19
        );
        assert_eq!(standing.docked(7), Ok(true));
        for (role, kind) in [(1, 9), (2, 10)] {
            let fighting = motions(TaskForceMapMotionRow {
                task_force: 7,
                combat_role: role,
                path: vec![6],
                ..Default::default()
            });
            let placed = fighting.visual(&force, None, None, &anchor, &sea, &|_, _| 1.0).unwrap();
            assert_eq!(
                (placed.kind, placed.position.standing),
                (kind, [5.0, 0.0, kind as f32])
            );
            assert_eq!(
                placed.unplaced_route,
                Some("the content movement map is not loaded")
            );
            assert_eq!(fighting.docked(7), Ok(false));
        }
        let other = TaskForceRow {
            id: 8,
            ..force.clone()
        };
        assert_eq!(
            standing
                .visual(&other, None, None, &anchor, &sea, &|_, _| 1.0)
                .unwrap_err(),
            "task-force motion row absent"
        );
        assert!(TaskForceMotions::default().docked(7).is_err());
    }
}

pub fn skip_of(kind: u32, inputs: Option<(u8, u32)>, mission: u32, current: Option<u32>, region: Option<u32>) -> Option<bool> {
    if kind == ATTACKING || kind == DEFENDING {
        return Some(false);
    }
    let (is_in_regions, bombardment_region) = inputs?;
    if !mission_active(is_in_regions, mission, bombardment_region) {
        return Some(false);
    }
    if mission != PATROL {
        return Some(true);
    }
    let current = current?;
    Some(current != 0 && Some(current) == region)
}
