use super::*;
use viewer_gpu::province_map::WATER_HEIGHT;

use replay_data::{rows::TableId, rows::TableRows, rows::TaskForceMapMotionRow};
use replay_frame::{frame::Metrics, frame::TableView};

use viewer_unit_reads::{counter_move::RouteWorld, task_force_visual::TaskForceMotions};
use viewer_foundation::movement_timing::MovementContent;

fn fixture() -> (DecodedFrame, AirRegistrySources, AirBaseRow) {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 42,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(vec![
        ShipRow {
            id: 19,
            task_force: 29,
            ..Default::default()
        },
        ShipRow {
            id: 20,
            task_force: 30,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(vec![
        TaskForceRow {
            id: 29,
            province: 6,
            ..Default::default()
        },
        TaskForceRow {
            id: 30,
            roster_owner: 7,
            province: 5,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::TaskForceMapMotion.index()] =
        TableView::Complete(TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            task_force: 30,
            target: 6,
            path: vec![6],
            ..Default::default()
        }]));
    let registry = AirRegistrySources {
        host_owners: BTreeMap::from([(10, 7)]),
        ..Default::default()
    };
    let base = AirBaseRow {
        id: 10,
        host: 20,
        ..Default::default()
    };
    (frame, registry, base)
}

fn movement() -> MovementContent {
    use content_reader_map::movement::{MovementMap, Neighbour, ProvinceTerrain};
    let terrain = ProvinceTerrain { flags: 0, index: 1 };
    let neighbour = |to| {
        vec![Neighbour {
            to,
            distance: Some(100_000),
            ..Default::default()
        }]
    };
    MovementContent {
        map: MovementMap::from_parts(&[(5, terrain, neighbour(6)), (6, terrain, neighbour(5))]),
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
        rail: Err("unrelated railway defines are unavailable".into()),
        task_force_terrain: Err("unrelated task-force terrain is unavailable".into()),
        land: Err("unrelated land movement terrain is unavailable".into()),
    }
}

fn anchor(province: u32, slot: u32) -> Option<[f32; 3]> {
    match (province, slot) {
        (5, 0) => Some([20.0, 0.0, 20.0]),
        (6, 0) => Some([30.0, 0.0, 20.0]),
        (5, 19) => Some([13.0, 14.0, 15.0]),
        _ => None,
    }
}

#[test]
fn carrier_takeoff_uses_the_joined_task_forces_settled_route_point() {
    let (frame, registry, base) = fixture();
    let motions = TaskForceMotions::read(&frame).unwrap();
    let movement = movement();
    let world = RouteWorld {
        map: &movement.map,
        heights: HeightField {
            width: 2,
            height: 2,
            values: &[0; 4],
        },
        width: 100.0,
    };
    let origin = |force: &TaskForceRow| {
        motions.settled_origin(force, Some(&movement), Some(&world), &anchor, &|_| false)
    };
    let sources = CarrierSources::read(&frame, &registry, &origin).unwrap();
    assert_eq!(sources.province(&base), Ok(5));
    let actual = sources.start(&base).unwrap();
    let expected = [21.0, WATER_HEIGHT, 20.0];
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
    }
    let visual = motions
        .visual(
            sources.task_force(&base).unwrap(),
            Some(&movement),
            Some(&world),
            &anchor,
            &|_| false,
            &|_, _| 1.0,
        )
        .unwrap();
    assert!(visual.position.motion.is_some());
    assert_eq!(sources.start(&base), Ok(visual.position.at()));
    assert_ne!(sources.start(&base), Ok(visual.position.standing));
}

#[test]
fn a_docked_carrier_uses_the_content_port_stack_anchor() {
    let (mut frame, registry, base) = fixture();
    *frame.tables[TableId::TaskForceMapMotion.index()] =
        TableView::Complete(TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            task_force: 30,
            ..Default::default()
        }]));
    let motions = TaskForceMotions::read(&frame).unwrap();
    let origin =
        |force: &TaskForceRow| motions.settled_origin(force, None, None, &anchor, &|_| true);
    let sources = CarrierSources::read(&frame, &registry, &origin).unwrap();
    assert_eq!(sources.start(&base), Ok([13.0, 14.0, 15.0]));
}

#[test]
fn carrier_origins_require_complete_rosters_and_resolved_host_joins() {
    let (frame, registry, base) = fixture();
    let origin = |_: &TaskForceRow| -> Result<[f32; 3], &'static str> {
        panic!("a refused join must not ask for route placement")
    };
    let cases = [
        (TableId::Ships, None, "the ship census is incomplete"),
        (
            TableId::TaskForces,
            None,
            "the task-force census is incomplete",
        ),
        (
            TableId::Ships,
            Some(TableRows::Ships(Vec::new())),
            "the carrier is outside the recorded ship roster",
        ),
        (
            TableId::TaskForces,
            Some(TableRows::TaskForces(Vec::new())),
            "the carrier's task force is outside the recorded roster",
        ),
    ];
    for (table, rows, reason) in cases {
        let mut missing = frame.clone();
        *missing.tables[table.index()] = match rows {
            Some(rows) => TableView::Complete(rows),
            None => TableView::Unavailable {
                reason: "fixture".into(),
            },
        };
        let sources = CarrierSources::read(&missing, &registry, &origin).unwrap();
        assert_eq!(sources.start(&base), Err(reason));
    }
    for (table, reason) in [
        (TableId::Ships, "the ship census is incomplete"),
        (TableId::TaskForces, "the task-force census is incomplete"),
    ] {
        let TableView::Complete(rows) = &*frame.tables[table.index()] else {
            unreachable!()
        };
        let mut incomplete = frame.clone();
        *incomplete.tables[table.index()] = TableView::Truncated {
            rows: rows.clone(),
            missing: 1,
        };
        assert_eq!(
            CarrierSources::read(&incomplete, &registry, &origin)
                .unwrap()
                .start(&base),
            Err(reason)
        );
    }
    let sources = CarrierSources::read(&frame, &registry, &origin).unwrap();
    assert_eq!(
        sources.start(&AirBaseRow { host: 0, ..base }),
        Err("the stateless base has no host")
    );
}

#[test]
fn conflicting_registry_and_fleet_owners_refuse_before_route_placement() {
    let (frame, mut registry, base) = fixture();
    registry.host_owners.insert(base.id, 8);
    let origin = |_: &TaskForceRow| -> Result<[f32; 3], &'static str> {
        panic!("an owner conflict must not ask for route placement")
    };
    assert_eq!(
        CarrierSources::read(&frame, &registry, &origin)
            .unwrap()
            .start(&base),
        Err("the carrier's registry and fleet owners disagree")
    );
}

#[test]
fn unresolved_or_null_force_registry_witnesses_refuse_before_placement() {
    let (frame, mut registry, base) = fixture();
    let origin = |_: &TaskForceRow| -> Result<[f32; 3], &'static str> {
        panic!("an unresolved registry host must not ask for route placement")
    };
    registry.host_owners.clear();
    assert_eq!(
        CarrierSources::read(&frame, &registry, &origin)
            .unwrap()
            .start(&base),
        Err("the carrier registry host does not resolve")
    );
    registry.host_owners.insert(base.id, 7);
    registry.hosts_without_task_force.insert(base.id);
    let sources = CarrierSources::read(&frame, &registry, &origin).unwrap();
    assert_eq!(sources.start(&base), Err("the carrier has no task force"));
    assert_eq!(
        sources.province(&base),
        Err("the carrier has no task force")
    );
}

#[test]
fn missing_motion_census_or_force_row_is_a_named_carrier_refusal() {
    let (frame, registry, base) = fixture();
    for (view, reason) in [
        (
            TableView::Unavailable {
                reason: "fixture".into(),
            },
            "task-force motion census incomplete",
        ),
        (
            TableView::Complete(TableRows::TaskForceMapMotion(Vec::new())),
            "task-force motion row absent",
        ),
        (
            TableView::Truncated {
                rows: TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                    task_force: 30,
                    ..Default::default()
                }]),
                missing: 1,
            },
            "task-force motion census incomplete",
        ),
    ] {
        let mut missing = frame.clone();
        *missing.tables[TableId::TaskForceMapMotion.index()] = view;
        let motions = TaskForceMotions::read(&missing).unwrap();
        let origin =
            |force: &TaskForceRow| motions.settled_origin(force, None, None, &anchor, &|_| false);
        let sources = CarrierSources::read(&missing, &registry, &origin).unwrap();
        assert_eq!(sources.start(&base), Err(reason));
    }
}

#[test]
fn an_unplaced_carrier_route_never_becomes_a_static_origin() {
    let (frame, registry, base) = fixture();
    let motions = TaskForceMotions::read(&frame).unwrap();
    let movement = movement();
    let origin =
        |force: &TaskForceRow| motions.settled_origin(force, None, None, &anchor, &|_| false);
    assert_eq!(
        CarrierSources::read(&frame, &registry, &origin)
            .unwrap()
            .start(&base),
        Err("the content movement map is not loaded")
    );

    let origin = |force: &TaskForceRow| {
        motions.settled_origin(force, Some(&movement), None, &anchor, &|_| false)
    };
    assert_eq!(
        CarrierSources::read(&frame, &registry, &origin)
            .unwrap()
            .start(&base),
        Err(
            "the content movement map cannot place its route (a province without terrain, neighbours or a stack anchor, or a first step to a province that is not a neighbour)"
        )
    );

    let origin =
        |force: &TaskForceRow| motions.settled_origin(force, None, None, &|_, _| None, &|_| false);
    assert_eq!(
        CarrierSources::read(&frame, &registry, &origin)
            .unwrap()
            .start(&base),
        Err("task-force province has no unit-stack anchor")
    );

    let mut missing_terrain = movement;
    missing_terrain.map = content_reader_map::movement::MovementMap::from_parts::<
        content_reader_map::movement::ProvinceTerrain,
    >(&[]);
    let world = RouteWorld {
        map: &missing_terrain.map,
        heights: HeightField {
            width: 2,
            height: 2,
            values: &[0; 4],
        },
        width: 100.0,
    };
    let origin = |force: &TaskForceRow| {
        motions.settled_origin(
            force,
            Some(&missing_terrain),
            Some(&world),
            &anchor,
            &|_| false,
        )
    };
    assert_eq!(
        CarrierSources::read(&frame, &registry, &origin)
            .unwrap()
            .start(&base),
        Err(
            "the content movement map cannot place its route (a province without terrain, neighbours or a stack anchor, or a first step to a province that is not a neighbour)"
        )
    );
}
