use super::*;
use viewer_gpu::province_map::WATER_HEIGHT;
use viewer_foundation::movement_timing::Etas;
use viewer_foundation::movement_timing::MovementContent;
use content_reader_map::movement::{Neighbour, ProvinceTerrain};

const W: f32 = 5632.0;

fn at_hour(hour: i32, at: [f32; 3]) -> viewer_gpu::icon_tracks::Place {
    viewer_gpu::icon_tracks::Place { hour, at }
}
const FLOOR: f32 = WATER_HEIGHT;

#[test]
fn scale_interpolates_over_the_next_anchor_arc_not_the_current_segment() {
    let samples = [
        Sample {
            at: [0.0; 3],
            arc: 0.0,
        },
        Sample {
            at: [0.5, 0.0, 0.0],
            arc: 0.5,
        },
        Sample {
            at: [10.0, 0.0, 0.0],
            arc: 10.0,
        },
    ];
    let sample = RouteSample::from_samples(W, &samples, [[0.0; 3], [10.0, 0.0, 0.0]]);
    assert_eq!(sample.next_arc, 10.0);
    assert_eq!(
        sample.interpolate_scale(0.22, 0.44),
        (0.44 - 0.22) * 0.1 + 0.22
    );
    let zero = [
        Sample {
            at: [0.0; 3],
            arc: 0.0,
        },
        Sample {
            at: [1.0, 0.0, 0.0],
            arc: 0.0,
        },
    ];
    let sample = RouteSample::from_samples(W, &zero, [[0.0; 3], [1.0, 0.0, 0.0]]);
    assert_eq!(sample.interpolate_scale(0.22, 0.44), 0.44);
    let map = MovementMap::from_parts(&[(1, LAND, land(&[2])), (2, LAND, land(&[1]))]);
    let values = flat(0);
    let world = RouteWorld {
        map: &map,
        heights: field(&values),
        width: W,
    };
    let anchor = |_: u32| Some([10.0, 0.0, 10.0]);
    assert!(Motion::from_route(1, &[2], &anchor, &world).is_none());
    let sample = Motion::route_sample(1, &[2], &anchor, &world).unwrap();
    assert!(sample.sample_count != 0 && sample.motion.is_none());
    assert_eq!(sample.scale_fraction, 1.0);
    assert_eq!(sample.interpolate_scale(0.22, 0.44), 0.44);
}

#[test]
fn a_nonempty_route_can_initialize_no_animator_samples() {
    let map = line_map();
    let values = flat(100);
    let world = RouteWorld {
        map: &map,
        heights: field(&values),
        width: W,
    };
    let sample = Motion::route_sample(1, &[2, 3, 9], &line_anchor, &world).unwrap();
    assert_eq!(sample.sample_count, 0);
    assert!(sample.motion.is_none());
    let route = UnitRoute {
        class: CounterClass::TaskForce,
        target: 9,
        path_total: 3,
        path: &[2, 3, 9],
        ..Default::default()
    };
    assert_eq!(
        route.stack_kind_from_animator(1, &map, sample.sample_count),
        Some(19)
    );
}

#[test]
fn cleared_animator_uses_the_ordinary_route_for_direction() {
    let sea = ProvinceTerrain { flags: 0, index: 1 };
    let map = MovementMap::from_parts(&[(1, sea, land(&[2, 3]))]);
    let route = UnitRoute {
        class: CounterClass::TaskForce,
        path_total: 1,
        path: &[2],
        alternate_total: 1,
        alternate_path: &[3],
        ..Default::default()
    };
    assert_eq!(route.stack_kind_from_animator(1, &map, 0), Some(1));
    assert_eq!(route.stack_kind_from_animator(1, &map, 8), Some(2));
}

fn flat(value: u8) -> Vec<u8> {
    vec![value; 16]
}

fn field(values: &[u8]) -> HeightField<'_> {
    HeightField {
        width: 4,
        height: 4,
        values,
    }
}

fn land(to: &[u32]) -> Vec<Neighbour> {
    to.iter()
        .map(|&to| Neighbour {
            to,
            distance: Some(100_000),
            ..Default::default()
        })
        .collect()
}

const LAND: ProvinceTerrain = ProvinceTerrain { flags: 1, index: 1 };

fn close(a: [f32; 3], b: [f32; 3], tolerance: f32) {
    for axis in 0..3 {
        assert!(
            (a[axis] - b[axis]).abs() < tolerance,
            "{a:?} against {b:?} on axis {axis}"
        );
    }
}

fn settled(anchors: &[[f32; 3]]) -> Option<[f32; 3]> {
    Motion::new(W, anchors).map(|motion| motion.position())
}

fn controls(anchors: &[[f32; 3]], crossings: &[Option<[i32; 2]>]) -> Option<Vec<[f32; 3]>> {
    chain(anchors, crossings).map(|hops| route_control_points(W, &hops))
}

#[test]
fn native_route_inserts_only_admitted_crossings_before_spline_sampling() {
    let anchors = [[W - 5.0, 8.0, 20.0], [5.0, 9.0, 40.0]];
    assert_eq!(
        controls(&anchors, &[Some([0, 30])]),
        Some(vec![
            [W - 7.5, 0.0, 15.0],
            [W - 5.0, 0.0, 20.0],
            [W, 0.0, 30.0],
            [W + 5.0, 0.0, 40.0],
            [W + 7.5, 0.0, 45.0],
        ])
    );
    assert_eq!(
        controls(&anchors, &[None]),
        Some(vec![
            [W - 7.5, 0.0, 15.0],
            [W - 5.0, 0.0, 20.0],
            [W + 5.0, 0.0, 40.0],
            [W + 7.5, 0.0, 45.0],
        ])
    );
    assert_eq!(controls(&anchors, &[]), None);
}

fn line_map() -> MovementMap {
    MovementMap::from_parts(&[
        (1, LAND, land(&[2])),
        (2, LAND, land(&[1, 3, 8])),
        (3, LAND, land(&[2, 4])),
        (4, LAND, land(&[3, 5])),
        (5, LAND, land(&[4])),
        (8, LAND, land(&[2])),
        (9, LAND, Vec::new()),
    ])
}

fn line_anchor(id: u32) -> Option<[f32; 3]> {
    (id != 8).then_some([id as f32 * 10.0, 0.0, (id % 2) as f32 * 5.0])
}

fn line_motion(map: &MovementMap, path: &[u32]) -> Option<Motion> {
    let values = flat(100);
    let world = RouteWorld {
        map,
        heights: field(&values),
        width: W,
    };
    Motion::from_route(1, path, &line_anchor, &world)
}

#[test]
fn a_later_hop_without_a_neighbour_or_an_anchor_is_skipped() {
    let map = line_map();
    let whole = walk_route(&map, 1, &[2, 3], &line_anchor);
    assert_eq!(whole.skipped, 0);
    for gap in [9, 8] {
        let walk = walk_route(&map, 1, &[2, gap, 3], &line_anchor);
        assert_eq!(walk.skipped, 1, "{gap}");
        assert_eq!(walk.hops, whole.hops, "{gap}");
        assert_eq!(line_motion(&map, &[2, gap, 3]), line_motion(&map, &[2, 3]));
    }
    assert!(line_motion(&map, &[2, 3]).is_some());
}

#[test]
fn a_skipped_end_hop_leaves_its_end_of_the_polyline_without_a_handle() {
    let map = line_map();
    let walk = walk_route(&map, 1, &[9, 2, 3], &line_anchor);
    assert_eq!(walk.skipped, 1);
    let points = route_control_points(W, &walk.hops);
    assert_eq!(points[0], [10.0, 0.0, 5.0]);
    assert_eq!(points.len(), 4);
    let walk = walk_route(&map, 1, &[2, 3, 9], &line_anchor);
    assert_eq!(walk.skipped, 1);
    assert_eq!(route_control_points(W, &walk.hops).len(), 3);
    assert_eq!(line_motion(&map, &[2, 3, 9]), None);
    assert_eq!(walk_route(&map, 1, &[9, 8], &line_anchor).hops, []);
    assert_eq!(line_motion(&map, &[9, 8]), None);
    assert_eq!(
        walk_route(&map, 8, &[2], &line_anchor),
        RouteWalk::default()
    );
}

#[test]
fn the_walk_stops_once_the_settled_controls_are_fixed() {
    let map = line_map();
    let walk = walk_route(&map, 1, &[2, 3, 4, 5, 9], &line_anchor);
    assert_eq!((walk.hops.len(), walk.skipped), (4, 0));
    assert!(walk.hops.iter().all(|hop| !hop.last));
    assert_eq!(
        line_motion(&map, &[2, 3, 4, 5, 9]),
        line_motion(&map, &[2, 3, 4, 5])
    );
    assert_eq!(walk_route(&map, 1, &[2, 3, 4, 9], &line_anchor).skipped, 1);
}

#[test]
fn route_height_is_sampled_after_flat_control_points() {
    let anchors = [[0.0, 22.0, 1.0], [3.0, 23.0, 1.0]];
    let values = flat(120);
    let motion = Motion::new_with_crossings(W, &anchors, &[Some([1, 1])], &field(&values))
        .expect("valid crossing route");
    let at = motion.position();
    assert!((at[0] - 1.0).abs() < 0.05, "{at:?}");
    assert_eq!(at[1], 12.0);
}

#[test]
fn the_settled_counter_sits_one_world_unit_along_its_route() {
    close(
        settled(&[[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]]).expect("moves"),
        [1.0, FLOOR, 0.0],
        1e-3,
    );
    close(
        settled(&[[10.0, 0.0, 10.0], [10.0, 0.0, -90.0]]).expect("moves"),
        [10.0, FLOOR, 9.0],
        1e-3,
    );
}

#[test]
fn a_hop_shorter_than_the_clamp_runs_past_its_next_anchor() {
    let at = settled(&[[0.0, 0.0, 0.0], [0.5, 0.0, 0.0]]).expect("moves");
    assert!(at[0] > 0.5, "{at:?}");
    close(at, [1.0, FLOOR, 0.0], 0.05);
}

#[test]
fn the_turn_ahead_bends_the_first_world_unit_and_sets_the_heading() {
    let anchors = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 0.0, 100.0]];
    let motion = Motion::new(W, &anchors).expect("moves");
    let (at, heading) = motion.position_heading();
    assert!((at[0] - 1.0).abs() < 0.05, "{at:?}");
    assert!(
        at[2] <= 0.0,
        "the corner ahead bows the start away from it: {at:?}"
    );
    assert!(heading.abs() < 0.1, "{heading}");
}

#[test]
fn a_one_province_route_leaves_the_counter_where_it_stands() {
    assert_eq!(settled(&[[1.0, 2.0, 3.0]]), None);
    assert_eq!(settled(&[]), None);
    assert_eq!(Position::still([4.0, 5.0, 6.0]).at(), [4.0, 5.0, 6.0]);
}

#[test]
fn a_route_across_the_map_seam_takes_the_short_way() {
    let west = [4.0, 0.0, 500.0];
    let east = [W - 4.0, 0.0, 500.0];
    let brought = unwrap_x(W, east, west);
    assert!((brought[0] - -4.0).abs() < 1e-3, "{brought:?}");
    close(
        settled(&[west, east]).expect("moves"),
        [3.0, FLOOR, 500.0],
        1e-3,
    );
}

#[test]
fn two_hops_draw_exactly_what_the_whole_route_draws() {
    let values = flat(100);
    let anchors = [
        [0.0, 0.0, 0.0],
        [4.0, 0.0, 1.0],
        [7.0, 0.0, 6.0],
        [6.0, 0.0, 12.0],
        [2.0, 0.0, 15.0],
    ];
    let crossings = [Some([2, 0]), None, Some([6, 9]), None];
    let whole = Motion::new_with_crossings(W, &anchors, &crossings, &field(&values)).unwrap();
    let cut =
        Motion::new_with_crossings(W, &anchors[..3], &crossings[..2], &field(&values)).unwrap();
    assert_eq!(whole, cut);
}

#[test]
fn next_anchor_uses_first_near_sample_and_wraps_at_the_map_seam() {
    let sample = |x| Sample {
        at: [x, 10.0, 1.0],
        arc: x,
    };
    let samples = [sample(0.0), sample(0.01), sample(0.02)];
    assert_eq!(next_anchor_sample(W, &samples, [0.02, 999.0, 1.0]), 0);
    assert_eq!(next_anchor_sample(W, &samples, [5.0, 999.0, 1.0]), 2);
    assert_eq!(
        next_anchor_sample(W, &[sample(W - 0.01), sample(100.0)], [0.01, 999.0, 1.0]),
        0
    );
}

#[test]
fn the_segment_search_stops_at_the_next_anchor_and_does_not_clamp() {
    let sample = |x: f32, arc| Sample {
        at: [x, 0.0, 0.0],
        arc,
    };
    let samples = [
        sample(0.0, 0.0),
        sample(0.4, 0.4),
        sample(0.8, 0.8),
        sample(2.0, 2.0),
    ];
    assert_eq!(segment(&samples, 3, 1.0), Some(3));
    assert_eq!(segment(&samples, 2, 1.0), Some(2));
    assert_eq!(segment(&samples, 0, 1.0), None);
    close(point_on(&samples, 3, 1.0), [1.0, 0.0, 0.0], 1e-5);
    close(point_on(&samples, 2, 1.0), [1.0, 0.0, 0.0], 1e-5);
    let flat = [sample(0.0, 0.0), sample(0.5, 0.00001)];
    assert_eq!(point_on(&flat, 1, 1.0), [0.5, 0.0, 0.0]);
}

#[test]
fn native_stack_kind_uses_viewer_animation_and_native_neighbor_ordinal() {
    let stationary = StackKindInputs {
        terrain_flags: 1,
        ..Default::default()
    };
    assert_eq!(stack_kind(stationary), 0);
    assert_eq!(
        stack_kind(StackKindInputs {
            naval_transfer: true,
            ..stationary
        }),
        19
    );
    let moving = StackKindInputs {
        display_moving: true,
        direction: Some(3),
        ..stationary
    };
    assert_eq!(stack_kind(moving), 4);
    assert_eq!(
        stack_kind(StackKindInputs {
            naval_transfer: true,
            ..moving
        }),
        20
    );
    assert_eq!(
        stack_kind(StackKindInputs {
            terrain_flags: 0,
            ..moving
        }),
        14
    );
    assert_eq!(
        stack_kind(StackKindInputs {
            direction: Some(8),
            ..moving
        }),
        1
    );
    assert_eq!(
        stack_kind(StackKindInputs {
            combat_role: 2,
            ..moving
        }),
        10
    );
}

#[test]
fn routes_mark_which_stack_kinds_are_native() {
    use replay_data::{
        rows::DivisionMovementRow, rows::DivisionRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 0,
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed for routes".into(),
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    let located = [(1, 1), (2, 2), (3, 1)];
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
        located
            .iter()
            .map(|&(id, province)| DivisionRow {
                id,
                province,
                roster_owner: 1,
                logical_owner: 1,
                ..Default::default()
            })
            .collect(),
    ));
    let attacking = |division, path: Vec<u32>| DivisionMovementRow {
        division,
        combat_role: 1,
        combat_count: 1,
        combat_province: 2,
        withdrawal: replay_data::rows::Recorded::Value(Some(0)),
        path_total: path.len() as u32,
        path,
        ..Default::default()
    };
    *frame.tables[TableId::DivisionMovement.index()] =
        TableView::Complete(TableRows::DivisionMovement(vec![
            attacking(1, Vec::new()),
            DivisionMovementRow {
                division: 2,
                combat_role: 2,
                combat_count: 1,
                combat_province: 2,
                ..Default::default()
            },
            attacking(3, vec![9]),
            attacking(4, Vec::new()),
        ]));
    let mut panel = replay_data::rows::DivisionPanelStatsRow {
        division: 3,
        ..Default::default()
    };
    panel.panel_stats[replay_data::rows::DivisionPanelStatsRow::MAXIMUM_SPEED] = Some(100_000);
    *frame.tables[TableId::DivisionPanelStats.index()] =
        TableView::Complete(TableRows::DivisionPanelStats(vec![panel]));
    let map = MovementMap::from_parts(&[(1, LAND, land(&[2])), (2, LAND, land(&[1]))]);
    let anchor = |id: u32| Some([id as f32, 0.0, 0.0]);
    let timing = MovementContent {
        map: MovementMap::from_parts(&[(1, LAND, Vec::new()), (9, LAND, Vec::new())]),
        defines: replay_derive::eta_derive::Defines {
            outer_scale: 100_000,
            ..Default::default()
        },
        rail: Ok(content_reader_map::movement::RailMovementDefines {
            speed: [0; 4],
            maximum_level: 0,
        }),
        task_force_terrain: Err("unrelated task-force terrain is unavailable".into()),
        land: Ok(viewer_foundation::movement_timing::LandSpeedContent {
            terrain_movement: vec![0],
            ..Default::default()
        }),
    };
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        [1, 9]
            .map(|id| replay_data::rows::ProvinceRow {
                id,
                ..Default::default()
            })
            .into(),
    ));
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![
            viewer_foundation::movement_timing::test_support::neutral_status(3, 1),
        ]));
    viewer_foundation::movement_timing::test_support::neutral_world(&mut frame, 1);
    let etas = Etas::read(&frame, Some(&timing)).unwrap();
    assert_eq!(etas.get(3), Some((100_000, 100_000)));
    assert_eq!(etas.get(1), None);
    assert_eq!(etas.get(4), None);
    let kinds = |map: Option<&MovementMap>| {
        let routes = DivisionRoutes::read(&frame, &etas, map, &anchor).unwrap();
        [1, 2, 3, 4].map(|id| {
            let route = &routes.by_division[&id];
            assert_eq!(route.eta, etas.get(id));
            (route.stack_kind, route.kind_resolved)
        })
    };
    let [first, defending, unplaced, unlocated] = kinds(Some(&map));
    assert_eq!(first, (1, true));
    assert_eq!(defending, (10, true));
    assert!(!unplaced.1);
    assert!(!unlocated.1);
    let [first, defending, unplaced, unlocated] = kinds(None);
    assert_eq!(defending, (10, true));
    assert!([first, unplaced, unlocated].iter().all(|kind| !kind.1));
}

#[test]
fn cold_stack_uses_alternate_route_and_requires_complete_geometry() {
    let map = MovementMap::from_parts(&[
        (1, LAND, land(&[2, 3])),
        (2, LAND, land(&[1])),
        (3, LAND, land(&[1])),
    ]);
    let anchor = |id| Some([id as f32, 0.0, 0.0]);
    let mut row = replay_data::rows::DivisionMovementRow {
        path: vec![2],
        path_total: 1,
        alternate_path: vec![3],
        alternate_total: 1,
        ..Default::default()
    };
    let kind = |row: &replay_data::rows::DivisionMovementRow,
                anchor: &dyn Fn(u32) -> Option<[f32; 3]>| {
        UnitRoute::division(row).stack_kind(1, &map, anchor)
    };
    assert_eq!(UnitRoute::division(&row).display_path(), &[3]);
    assert_eq!(kind(&row, &anchor), Some(2));
    row.alternate_total = 2;
    assert_eq!(kind(&row, &anchor), None);
    row.alternate_total = 0;
    assert_eq!(kind(&row, &anchor), Some(1));
    assert_eq!(kind(&row, &|_| None), None);
    row.path.clear();
    row.path_total = 0;
    row.combat_role = 1;
    row.combat_count = 1;
    row.combat_province = 3;
    assert_eq!(kind(&row, &anchor), Some(2));
    row.combat_role = 0;
    row.combat_count = 0;
    assert_eq!(kind(&row, &anchor), Some(0));
    assert_eq!(UnitRoute::division(&row).stack_kind(9, &map, &anchor), None);
    assert!(snaps_to_stack(CounterClass::Division, 2, 1, 1));
    assert!(!snaps_to_stack(CounterClass::Division, 12, 1, 1));
    assert!(!snaps_to_stack(CounterClass::Division, 2, 0, 0));
    assert!(snaps_to_stack(CounterClass::Division, 9, 0, 0));
}

const SEA: ProvinceTerrain = ProvinceTerrain { flags: 0, index: 1 };

#[test]
fn a_task_force_takes_the_class1_kinds_and_its_port_slot() {
    use replay_data::rows::TaskForceMapMotionRow as Row;
    let map = MovementMap::from_parts(&[
        (1, LAND, land(&[2, 3, 4])),
        (2, SEA, land(&[1, 6])),
        (3, SEA, land(&[1, 6])),
        (4, SEA, land(&[1, 6])),
        (6, SEA, land(&[2, 3, 4])),
    ]);
    let anchor = |id: u32| Some([id as f32, 0.0, 0.0]);
    let kind = |row: &Row, province| UnitRoute::task_force(row).stack_kind(province, &map, &anchor);
    let navy = CounterClass::TaskForce;
    assert_eq!((navy.standstill(true), navy.standstill(false)), (19, 0));
    let mut row = Row::default();
    assert_eq!(kind(&row, 1), Some(19));
    assert_eq!(kind(&row, 6), Some(0));
    row.path = vec![3];
    assert_eq!(kind(&row, 1), Some(20));
    assert_eq!(kind(&row, 6), Some(2));
    row.combat_role = 1;
    assert_eq!(kind(&row, 6), Some(9));
    row.combat_role = 2;
    assert_eq!(kind(&row, 6), Some(10));
    assert!(snaps_to_stack(navy, 9, 1, 1) && snaps_to_stack(navy, 10, 2, 1));
    assert!(!snaps_to_stack(navy, 0, 0, 0));
}

#[test]
fn a_railway_gun_takes_the_class13_kinds() {
    use replay_data::rows::RailwayGunCounterInputRow as Row;
    let map = MovementMap::from_parts(&[
        (1, LAND, land(&[2, 3, 4])),
        (2, LAND, land(&[1, 6])),
        (3, LAND, land(&[1, 6])),
        (4, LAND, land(&[1, 6])),
        (6, SEA, land(&[2, 3, 4])),
    ]);
    let anchor = |id: u32| (id != 9).then_some([id as f32, 0.0, 0.0]);
    let kind =
        |row: &Row, province| UnitRoute::railway_gun(row).stack_kind(province, &map, &anchor);
    let gun = CounterClass::RailwayGun;
    let mut row = Row::default();
    assert_eq!(kind(&row, 1), Some(21));
    row.naval_transfer = 1;
    assert_eq!(kind(&row, 1), Some(19));
    assert_eq!(kind(&row, 6), Some(21));
    row.ordinary_path = vec![3];
    assert_eq!(kind(&row, 1), Some(20));
    row.naval_transfer = 0;
    assert_eq!(kind(&row, 1), Some(23));
    assert_eq!(kind(&row, 6), Some(31));
    row.alternate_path = vec![4];
    assert_eq!(kind(&row, 1), Some(24));
    row.alternate_path.clear();
    row.ordinary_path.clear();
    row.target = 3;
    assert_eq!(kind(&row, 1), Some(21));
    row.target = 0;
    row.combat_role = 1;
    row.combat_province = 2;
    row.combat_count = 1;
    assert_eq!(kind(&row, 1), Some(22));
    assert!(snaps_to_stack(gun, 22, 1, 1));
    assert!(!snaps_to_stack(gun, 9, 1, 1));
    assert!(!snaps_to_stack(gun, 2, 1, 1));
    assert!(!snaps_to_stack(gun, 30, 1, 1));
    assert!(!snaps_to_stack(gun, 22, 0, 0));
    row.combat_role = 2;
    assert_eq!(kind(&row, 1), Some(10));
    assert!(snaps_to_stack(gun, 10, 2, 1));
    for (path, expected) in [
        (vec![5], None),
        (vec![9], None),
        (vec![2, 9], Some(22)),
        (vec![2, 3], Some(22)),
    ] {
        let row = Row {
            ordinary_path: path,
            ..Default::default()
        };
        assert_eq!(kind(&row, 1), expected);
    }
    assert_eq!(kind(&Row::default(), 12), None);
}

#[test]
fn a_railway_gun_stands_and_moves_on_its_own_slot_unless_it_snaps() {
    use replay_data::rows::RailwayGunCounterInputRow as Row;
    let map = MovementMap::from_parts(&[(1, LAND, land(&[2])), (2, LAND, land(&[1]))]);
    let values = flat(100);
    let world = RouteWorld {
        map: &map,
        heights: field(&values),
        width: W,
    };
    let anchor = |id: u32, kind: u32| Some([id as f32 * 100.0 + kind as f32, 0.0, 1.0]);
    let moving = Row {
        ordinary_path: vec![2],
        ..Default::default()
    };
    let route = UnitRoute::railway_gun(&moving);
    assert_eq!(route.stack_kind(1, &map, &|id| anchor(id, 21)), Some(22));
    let position = route.position(1, 22, &world, &anchor).expect("anchored");
    assert_eq!(position.standing, [121.0, 0.0, 1.0]);
    let at = position.at();
    assert!((at[0] - 122.0).abs() < 1e-3, "{at:?}");
    assert_eq!(
        route.standing(1, true, 22, &anchor),
        Some([121.0, 0.0, 1.0])
    );
    let attacking = Row {
        combat_role: 1,
        combat_count: 1,
        combat_province: 2,
        ..moving.clone()
    };
    let snapped = UnitRoute::railway_gun(&attacking)
        .position(1, 22, &world, &anchor)
        .expect("anchored");
    assert!(snapped.motion.is_none());
    assert_eq!(snapped.at(), [122.0, 0.0, 1.0]);
    let defending = Row {
        combat_role: 2,
        ..attacking.clone()
    };
    assert_eq!(
        UnitRoute::railway_gun(&defending).standing(1, true, 10, &anchor),
        Some([110.0, 0.0, 1.0])
    );
}

#[test]
fn a_division_moves_to_its_settled_point_and_a_snapped_one_stands() {
    let map = MovementMap::from_parts(&[(1, LAND, land(&[2])), (2, LAND, land(&[1]))]);
    let values = flat(100);
    let world = RouteWorld {
        map: &map,
        heights: field(&values),
        width: W,
    };
    let anchor = |id: u32, kind: u32| Some([id as f32 * 100.0 + kind as f32, 0.0, 1.0]);
    let path = [2];
    let route = UnitRoute {
        path_total: 1,
        path: &path,
        ..Default::default()
    };
    let moving = route.position(1, 1, &world, &anchor).expect("anchored");
    assert_eq!(moving.standing, [100.0, 0.0, 1.0]);
    let at = moving.at();
    assert!((at[0] - 101.0).abs() < 1e-3, "{at:?}");
    let defending = UnitRoute {
        combat_role: 2,
        ..route
    };
    let snapped = defending
        .position(1, 10, &world, &anchor)
        .expect("anchored");
    assert!(snapped.motion.is_none());
    assert_eq!(snapped.at(), [110.0, 0.0, 1.0]);
}

#[test]
fn counter_action_route_idle_uses_native_signed_counts() {
    use replay_data::{rows::DivisionMovementRow, rows::TableId, rows::TableRows};
    use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 0,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not needed".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::DivisionMovement.index()] =
        TableView::Complete(TableRows::DivisionMovement(vec![DivisionMovementRow {
            division: 7,
            path_total: u32::MAX,
            combat_count: u32::MAX,
            ..Default::default()
        }]));
    let etas = Etas::default();
    let routes = DivisionRoutes::read(&frame, &etas, None, &|_| None).unwrap();
    assert!(routes.by_division[&7].idle);
}

fn live_position(province: u32, path: &[u32], start: f32, combat: bool) -> Position {
    let movement = Movement {
        province,
        path: path.into(),
        class: CounterClass::Division,
        combat,
        snapped: false,
        width: 1000.0,
        anchors: [[start, 0.0, 0.0], [start + 10.0, 0.0, 0.0]],
        samples: if path.is_empty() {
            Vec::new()
        } else {
            vec![
                Sample {
                    at: [start, 0.0, 0.0],
                    arc: 0.0,
                },
                Sample {
                    at: [start + 10.0, 0.0, 0.0],
                    arc: 10.0,
                },
            ]
        },
        next: usize::from(!path.is_empty()),
        scales: [1.0; 2],
    };
    Position {
        standing: [start, 0.0, 0.0],
        motion: movement.settled(),
        movement: Some(Arc::new(movement)),
    }
}

fn snapped(position: &Position, combat: bool) -> Position {
    let mut movement = Movement::clone(position.movement.as_ref().unwrap());
    movement.snapped = true;
    movement.combat = combat;
    Position {
        standing: [position.standing[0], 5.0, 0.0],
        motion: None,
        movement: Some(Arc::new(movement)),
    }
}

const SEEN: [u32; 4] = [1, 2, 5, 9];

fn movement_frame(time: f32, hour: i32, speed: f32) -> MovementFrame {
    MovementFrame {
        time,
        hour,
        head: viewer_gpu::icon_tracks::Head { hour, fraction: 0.0 },
        overlay_layout: None,
        clock_speed: Some(0),
        speeds: [speed; 2],
        steady: false,
    }
}

fn settled_on(old: &Position, units: &[Unit]) -> MovementHistory {
    let mut history = MovementHistory::default();
    for frame in -256..=0 {
        let hour = if frame < -250 { 99 } else { 100 };
        history.begin_frame(movement_frame(frame as f32 / 10.0, hour, 2.0), SEEN);
        if hour == 100 {
            for &unit in units {
                history.sample(unit, old);
            }
        }
    }
    history
}

fn next_frame(history: &mut MovementHistory, frame: i32, hour: i32, speed: f32) {
    history.begin_frame(movement_frame(frame as f32 / 10.0, hour, speed), SEEN);
}

fn drawn(history: &mut MovementHistory, unit: Unit, position: &Position) -> [f32; 3] {
    history
        .sample(unit, position)
        .map_or_else(|| position.at(), |drawn| drawn.motion.position())
}

#[test]
fn the_render_delta_averages_the_middle_of_its_sorted_frame_times_and_caps_at_half_a_second() {
    let mut delta = RenderDelta::default();
    assert_eq!(delta.step(0.1), 0.05);
    for _ in 0..32 {
        delta.step(0.1);
    }
    assert!(delta.step(0.1) < 0.08);
    for _ in 0..256 {
        delta.step(0.1);
    }
    assert!((delta.step(0.1) - 0.1).abs() < 1e-6);
    assert_eq!(delta.step(2.0), 0.5);
    assert_eq!(delta.step(-1.0), 0.0);
}

#[test]
fn arrivals_keep_the_old_route_and_move_each_frame_without_advancing_twice() {
    for unit in [Unit::Division(7), Unit::RailwayGun(7), Unit::TaskForce(7)] {
        let old = live_position(1, &[2, 3], 0.0, false);
        let next = live_position(2, &[3], 10.0, false);
        let mut history = settled_on(&old, &[unit]);
        assert_eq!(history.sample(unit, &old), None);
        let mut previous = 1.0;
        for frame in 1..=60 {
            next_frame(&mut history, frame, 101, 2.0);
            let x = drawn(&mut history, unit, &next)[0];
            assert_eq!(drawn(&mut history, unit, &next)[0], x);
            assert!(
                x >= previous && x - previous <= 0.201,
                "{frame}: {previous} -> {x}"
            );
            previous = x;
        }
        assert!((previous - 11.0).abs() < 1e-4);
        assert_eq!(history.sample(unit, &next), None);
        assert!(!history.running());
    }
}

#[test]
fn a_gliding_counter_is_placed_at_every_display_frame_between_two_publications() {
    for unit in [Unit::Division(7), Unit::TaskForce(7)] {
        let placed = |hours: i32| {
            let old = live_position(1, &[2, 3], 0.0, false);
            let next = live_position(2, &[3], 10.0, false);
            let mut history = settled_on(&old, &[unit]);
            (1..=12)
                .map(|frame| {
                    next_frame(&mut history, frame, 100 + hours * ((frame + 3) / 4), 2.0);
                    drawn(&mut history, unit, &next)[0]
                })
                .collect::<Vec<_>>()
        };
        let six = placed(6);
        for (at, pair) in six.windows(2).enumerate() {
            assert!(pair[1] > pair[0], "{unit:?} stood still over frame {}: {six:?}", at + 2);
        }
        assert_eq!(placed(24), six, "{unit:?}: the hours a publication covers do not place it");
    }
}

#[test]
fn layers_placing_one_unit_share_its_advance() {
    let unit = Unit::Division(7);
    let old = live_position(1, &[2, 3], 0.0, false);
    let counter = live_position(2, &[3], 10.0, false);
    let figure = live_position(2, &[3], 10.0, false);
    let mut history = settled_on(&old, &[unit]);
    for frame in 1..=10 {
        next_frame(&mut history, frame, 101, 2.0);
        let shown = drawn(&mut history, unit, &counter);
        assert_eq!(drawn(&mut history, unit, &figure), shown);
        assert!((shown[0] - (1.0 + 0.2 * frame as f32)).abs() < 1e-4);
    }
}

#[test]
fn units_off_screen_keep_their_arc_until_their_province_updates_in_view() {
    let unit = Unit::Division(7);
    let old = live_position(1, &[2, 3], 0.0, false);
    let next = live_position(2, &[3], 10.0, false);
    let mut history = settled_on(&old, &[unit]);
    next_frame(&mut history, 1, 101, 2.0);
    drawn(&mut history, unit, &next);
    next_frame(&mut history, 2, 101, 2.0);
    let before = drawn(&mut history, unit, &next);
    for frame in 3..=40 {
        history.begin_frame(movement_frame(frame as f32 / 10.0, 101, 2.0), [1, 9]);
        assert_eq!(drawn(&mut history, unit, &next), before);
        assert!(!history.running());
    }
    next_frame(&mut history, 41, 101, 2.0);
    assert!((drawn(&mut history, unit, &next)[0] - before[0] - 0.2).abs() < 1e-4);
}

#[test]
fn a_rendering_hitch_advances_an_arrival_at_most_half_a_second() {
    let unit = Unit::Division(7);
    let old = live_position(1, &[2, 3], 0.0, false);
    let next = live_position(2, &[3], 10.0, false);
    let mut history = settled_on(&old, &[unit]);
    next_frame(&mut history, 1, 101, 12.0);
    let before = drawn(&mut history, unit, &next)[0];
    history.begin_frame(movement_frame(2.1, 101, 12.0), SEEN);
    let after = drawn(&mut history, unit, &next)[0];
    assert!((after - before - 6.0).abs() < 1e-4, "{before} -> {after}");
}

#[test]
fn entering_combat_on_the_same_route_keeps_the_arc_and_stops_it() {
    let unit = Unit::Division(7);
    let old = live_position(1, &[2, 3], 0.0, false);
    let route = live_position(5, &[6], 50.0, false);
    let fighting = live_position(5, &[6], 50.0, true);
    let mut history = settled_on(&old, &[unit]);
    next_frame(&mut history, 1, 101, 2.0);
    assert!((drawn(&mut history, unit, &route)[0] - 50.2).abs() < 1e-4);
    next_frame(&mut history, 2, 101, 2.0);
    assert!((drawn(&mut history, unit, &route)[0] - 50.4).abs() < 1e-4);
    for frame in 3..=8 {
        next_frame(&mut history, frame, 102, 2.0);
        assert!((drawn(&mut history, unit, &fighting)[0] - 50.4).abs() < 1e-4);
        assert!(!history.running());
    }
}

#[test]
fn movement_never_glides_across_reroutes_combat_gaps_or_multiple_hops() {
    let unit = Unit::Division(7);
    let old = live_position(1, &[2, 3], 0.0, false);
    for next in [
        live_position(9, &[3], 90.0, false),
        live_position(3, &[], 20.0, false),
        live_position(2, &[3], 10.0, true),
    ] {
        let mut history = settled_on(&old, &[unit]);
        next_frame(&mut history, 1, 101, 2.0);
        let at = drawn(&mut history, unit, &next)[0];
        assert!(at >= next.standing[0] && at <= next.at()[0], "{at}");
    }
    let next = live_position(2, &[], 10.0, false);
    let mut history = settled_on(&old, &[unit]);
    history.clear();
    next_frame(&mut history, 1, 101, 2.0);
    assert_eq!(history.sample(unit, &next), None);
    history.begin_frame(
        MovementFrame {
            steady: true,
            ..movement_frame(0.2, 101, 2.0)
        },
        SEEN,
    );
    assert_eq!(history.sample(unit, &old), None);
}

#[test]
fn movement_arrival_deadlines_and_unit_classes_remain_independent() {
    let old = live_position(1, &[2, 3], 0.0, false);
    let next = live_position(2, &[3], 10.0, false);
    let units = [Unit::Division(7), Unit::RailwayGun(7), Unit::TaskForce(7)];
    let mut history = settled_on(&old, &units);
    for frame in 1..=2 {
        next_frame(&mut history, frame, 101, 0.1);
        for unit in units {
            history.sample(unit, &next);
        }
    }
    assert_eq!(history.units.len(), 3);
    assert!(history.running());
    next_frame(&mut history, 3, 101 + 6 * 24, 0.1);
    let at = drawn(&mut history, Unit::Division(7), &next);
    assert!((at[0] - 10.01).abs() < 1e-5, "{at:?}");
}

#[test]
fn recorded_maximum_speed_rebuilds_arrivals_and_clamps_new_routes() {
    let unit = Unit::Division(7);
    let old = live_position(1, &[2, 3], 0.0, false);
    let next = live_position(2, &[3], 10.0, false);
    let mut history = settled_on(&old, &[unit]);
    next_frame(&mut history, 1, 101, 2.0);
    next_frame(&mut history, 2, 101, 2.0);
    assert!(drawn(&mut history, unit, &next)[0] < 2.0);
    let fastest = MovementFrame {
        clock_speed: Some(4),
        ..movement_frame(0.3, 101, 2.0)
    };
    history.begin_frame(fastest, SEEN);
    assert_eq!(history.sample(unit, &next), None);
    assert_eq!(history.sample(unit, &next), None);
    assert!(!history.running());
}

#[test]
fn settling_and_abandoned_entries_do_not_keep_a_view_animated() {
    let unit = Unit::Division(7);
    let old = live_position(1, &[2, 3], 0.0, false);
    let next = live_position(2, &[3], 10.0, false);
    let mut history = settled_on(&old, &[unit]);
    next_frame(&mut history, 1, 101, 2.0);
    history.sample(unit, &next);
    assert!(history.running());
    next_frame(&mut history, 2, 101, 2.0);
    assert!(!history.running());
    history.begin_frame(
        MovementFrame {
            steady: true,
            ..movement_frame(0.3, 101, 2.0)
        },
        SEEN,
    );
    assert_eq!(history.sample(unit, &next), None);
    assert!(!history.running() && history.units.is_empty());
    history.begin_frame(
        MovementFrame {
            clock_speed: None,
            ..movement_frame(0.4, 102, 2.0)
        },
        SEEN,
    );
    assert_eq!(history.sample(unit, &old), None);
    assert!(!history.running() && history.units.is_empty());
}

#[test]
fn units_no_layer_drew_last_hour_leave_the_history() {
    let old = live_position(1, &[2, 3], 0.0, false);
    let mut history = settled_on(&old, &[Unit::Division(1), Unit::Division(2)]);
    next_frame(&mut history, 1, 101, 2.0);
    history.sample(Unit::Division(1), &old);
    next_frame(&mut history, 2, 102, 2.0);
    assert_eq!(
        history.units.keys().copied().collect::<Vec<_>>(),
        [Unit::Division(1)]
    );
}

#[test]
fn a_glide_interpolates_the_drawn_routes_stack_scale_over_its_next_anchor() {
    let unit = Unit::TaskForce(7);
    let mut old = live_position(1, &[2, 3], 0.0, false);
    old.scale_route([1.0, 2.0]);
    let mut next = live_position(2, &[3], 10.0, false);
    next.scale_route([2.0, 3.0]);
    let mut history = settled_on(&old, &[unit]);
    next_frame(&mut history, 1, 101, 10.0);
    next_frame(&mut history, 2, 101, 10.0);
    let glide = history.sample(unit, &next).unwrap();
    assert!((glide.motion.position()[0] - 2.0).abs() < 1e-4);
    assert!((glide.scale - 1.2).abs() < 1e-4, "{}", glide.scale);
}

#[test]
fn a_snapped_combat_keeps_the_route_so_the_later_arrival_still_glides() {
    let unit = Unit::TaskForce(7);
    let old = live_position(1, &[2, 3], 0.0, false);
    let fighting = snapped(&old, true);
    let next = live_position(2, &[3], 10.0, false);
    let mut history = settled_on(&old, &[unit]);
    for frame in 1..=3 {
        next_frame(&mut history, frame, 101, 2.0);
        assert_eq!(history.sample(unit, &fighting), None);
    }
    for frame in 4..=6 {
        next_frame(&mut history, frame, 102, 2.0);
        history.sample(unit, &fighting);
    }
    next_frame(&mut history, 7, 103, 2.0);
    let at = drawn(&mut history, unit, &next)[0];
    assert!((at - 1.2).abs() < 1e-4, "{at}");
}

#[test]
fn a_new_animator_starts_at_its_anchor_as_the_naval_one_does() {
    let unit = Unit::Division(7);
    let old = live_position(1, &[2, 3], 0.0, false);
    let mut history = settled_on(&old, &[]);
    for frame in 1..=5 {
        next_frame(&mut history, frame, 100, 2.0);
        let x = drawn(&mut history, unit, &old)[0];
        assert!((x - 0.2 * frame as f32).abs() < 1e-4, "{frame}: {x}");
    }
    next_frame(&mut history, 6, 100, 2.0);
    assert_eq!(history.sample(unit, &old), None);
}

#[test]
fn a_mover_carries_the_two_publications_own_places_and_no_prediction() {
    let unit = Unit::Division(7);
    let mut history = MovementHistory::default();
    next_frame(&mut history, 0, 100, 2.0);
    let first = DrawnLayout::new(100);
    assert_eq!(
        history.track(&first, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement),
        "the first publication to place it has nothing to move it from"
    );
    assert_eq!(
        history.track(&first, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement)
    );
    let second = DrawnLayout::new(148);
    let track = history
        .track(&second, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("two publications' places");
    assert_eq!(track.places().len(), 2);
    assert_eq!((track.places()[0].hour, track.places()[0].at), (100, [10.0, 0.0, 0.0]));
    assert_eq!((track.places()[1].hour, track.places()[1].at), (148, [58.0, 0.0, 0.0]));
    let at = |hour, fraction| track.at(viewer_gpu::icon_tracks::Head { hour, fraction })[0];
    assert_eq!([at(100, 0.0), at(124, 0.0), at(148, 0.0)], [10.0, 34.0, 58.0]);
    assert!((at(100, 0.5) - 10.5).abs() < 1e-3, "{}", at(100, 0.5));
    let held = history.track(&first, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
    assert_eq!(
        held,
        Err(TrackRefusal::NoPriorPlacement),
        "hour 100 has only hour 100's own place before it"
    );
    let third = DrawnLayout::new(196);
    let longer = history
        .track(&third, unit, &[at_hour(196, [106.0, 0.0, 0.0])], 0.0)
        .expect("three places");
    assert_eq!(longer.places().len(), 3);
    assert_eq!(longer.at(viewer_gpu::icon_tracks::Head { hour: 124, fraction: 0.0 })[0], 34.0);
}

#[test]
fn a_movers_places_are_the_hours_it_moved_at_and_not_the_intervals_ends() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let (first, second) = ([10.0, 0.0, 0.0], [58.0, 0.0, 0.0]);
    let at = |history: &mut MovementHistory, places: &[viewer_gpu::icon_tracks::Place], hour| {
        let layout = DrawnLayout::new(hour);
        history.track(&layout, unit, places, 0.0)
    };
    let mut hourly = MovementHistory::default();
    next_frame(&mut hourly, 0, 100, 2.0);
    let _ = at(&mut hourly, &[at_hour(100, first)], 100);
    let track = at(&mut hourly, &[at_hour(130, second), at_hour(148, second)], 148).expect("the hours' places");
    let place = |hour, fraction| track.at(Head { hour, fraction })[0];
    assert_eq!(place(100, 0.0), first[0], "the hour it was read in its old province");
    assert_eq!(place(115, 0.0), 34.0, "half the hours from 100 to 130 is half the way");
    assert_eq!(place(130, 0.0), second[0], "the hour it crossed");
    assert_eq!(place(140, 0.0), second[0], "the hours the recording left it there");
    assert!(track.held_still(Head { hour: 140, fraction: 0.0 }), "standing there is the recording's own");
    assert!(!track.held_still(Head { hour: 115, fraction: 0.0 }), "the hours move it there");

    let mut coarse = MovementHistory::default();
    next_frame(&mut coarse, 0, 100, 2.0);
    let _ = at(&mut coarse, &[at_hour(100, first)], 100);
    let whole = at(&mut coarse, &[at_hour(148, second)], 148).expect("the publications' places");
    let evenly = |hour| whole.at(Head { hour, fraction: 0.0 })[0];
    assert_eq!(evenly(124), 34.0, "the interval's middle, not the hour it crossed");
    assert!(evenly(140) < second[0], "still gliding at an hour it had arrived at");
}

#[test]
fn a_publication_without_hours_places_its_movers_at_its_own_hour_alone() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let mut history = MovementHistory::default();
    next_frame(&mut history, 0, 100, 2.0);
    let first = DrawnLayout::new(100);
    assert_eq!(
        history.track(&first, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement)
    );
    let second = DrawnLayout::new(148);
    let track = history
        .track(&second, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("the two publications' own places");
    assert_eq!(track.places().len(), 2, "the hours gave nothing to add");
    assert_eq!(track.at(Head { hour: 100, fraction: 0.0 })[0], 10.0);
    assert_eq!(track.at(Head { hour: 148, fraction: 0.0 })[0], 58.0);
}

#[test]
fn a_seek_backwards_starts_the_places_afresh_instead_of_pairing_two_histories() {
    let unit = Unit::Division(7);
    let mut history = MovementHistory::default();
    next_frame(&mut history, 0, 100, 2.0);
    let first = DrawnLayout::new(100);
    assert_eq!(
        history.track(&first, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement)
    );
    let second = DrawnLayout::new(148);
    history.track(&second, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0).expect("two places");
    let sought = DrawnLayout::new(40);
    assert_eq!(
        history.track(&sought, unit, &[at_hour(40, [900.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement),
        "the hour sought"
    );
    let after = DrawnLayout::new(42);
    let track = history
        .track(&after, unit, &[at_hour(42, [902.0, 0.0, 0.0])], 0.0)
        .expect("the seek's own two places");
    assert_eq!((track.places().len(), track.places()[0].hour, track.places()[1].hour), (2, 40, 42));
}

#[test]
fn a_figure_stands_where_its_counters_track_puts_it_at_the_frames_own_head() {
    use viewer_gpu::icon_tracks::Head;
    use viewer_gpu::icon_tracks::{IconAnchor, IconFamily};
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = settled_on(&moving, &[unit]);
    next_frame(&mut history, 1, 100, 2.0);
    let first = DrawnLayout::new(100);
    assert_eq!(
        history.track(&first, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement)
    );
    next_frame(&mut history, 2, 148, 2.0);
    let second = DrawnLayout::new(148);
    let track = history
        .track(&second, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("two publications' places");
    let counter = IconAnchor {
        world: moving.at(),
        cutoff: 0.0,
        family: IconFamily::Columns,
        moves: Some(1),
    };
    let mut walked = Vec::new();
    for (frame, hour, fraction) in [(3, 100, 0.0), (4, 112, 0.5), (5, 124, 0.0), (6, 160, 0.0)] {
        let head = Head { hour, fraction };
        history.begin_frame(
            MovementFrame {
                head,
                overlay_layout: Some(second.clone()),
                ..movement_frame(frame as f32 / 10.0, 148, 2.0)
            },
            SEEN,
        );
        let figure = history.at_head(unit, &moving).expect("the hours place it");
        assert_eq!(figure.at, counter.at(Some(&track), head), "head {hour}");
        assert_eq!(figure.heading, None, "head {hour}");
        walked.push(figure.at[0]);
    }
    let wanted = [10.0, 22.5, 34.0, 58.0];
    assert!(
        walked.iter().zip(wanted).all(|(walked, wanted)| (walked - wanted).abs() < 1e-3),
        "{walked:?}"
    );
}

#[test]
fn a_cold_seek_places_every_figure_where_the_walk_to_the_same_hour_does() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let head = Head { hour: 124, fraction: 0.25 };
    let place = |history: &mut MovementHistory, frame: i32, layout: &DrawnLayout| {
        history.begin_frame(
            MovementFrame {
                head,
                overlay_layout: Some(layout.clone()),
                ..movement_frame(frame as f32 / 10.0, 148, 2.0)
            },
            SEEN,
        );
        history.at_head(unit, &moving).expect("the hours place it").at
    };
    let walked = {
        let mut history = settled_on(&moving, &[unit]);
        next_frame(&mut history, 1, 100, 2.0);
        let first = DrawnLayout::new(100);
        let _ = history.track(&first, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
        next_frame(&mut history, 2, 148, 2.0);
        let second = DrawnLayout::new(148);
        let _ = history.track(&second, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0);
        for frame in 3..8 {
            place(&mut history, frame, &second);
        }
        place(&mut history, 8, &second)
    };
    let sought = {
        let mut history = MovementHistory::default();
        next_frame(&mut history, 1, 148, 2.0);
        let first = DrawnLayout::new(100);
        let _ = history.track(&first, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
        let second = DrawnLayout::new(148);
        let _ = history.track(&second, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0);
        place(&mut history, 2, &second)
    };
    assert_eq!(walked, sought);
}

#[test]
fn a_steady_frame_leaves_every_figure_on_its_hours_placement() {
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = settled_on(&moving, &[unit]);
    next_frame(&mut history, 1, 100, 2.0);
    let first = DrawnLayout::new(100);
    let _ = history.track(&first, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
    next_frame(&mut history, 2, 148, 2.0);
    let second = DrawnLayout::new(148);
    history
        .track(&second, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("two publications' places");
    history.begin_frame(
        MovementFrame {
            steady: true,
            overlay_layout: Some(second),
            ..movement_frame(0.3, 148, 2.0)
        },
        SEEN,
    );
    assert_eq!(history.at_head(unit, &moving), None);
}

#[test]
fn a_figure_stands_on_the_track_of_the_layout_its_frame_draws() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = settled_on(&moving, &[unit]);
    next_frame(&mut history, 1, 100, 2.0);
    let initial = DrawnLayout::new(100);
    assert_eq!(
        history.track(&initial, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement)
    );
    next_frame(&mut history, 2, 148, 2.0);
    let older_layout = DrawnLayout::new(148);
    let older = history
        .track(&older_layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("two places");
    next_frame(&mut history, 3, 196, 2.0);
    let newer_layout = DrawnLayout::new(196);
    let newer = history
        .track(&newer_layout, unit, &[at_hour(196, [130.0, 0.0, 0.0])], 0.0)
        .expect("three places");
    let unplaced_layout = DrawnLayout::new(60);
    let head = Head { hour: 160, fraction: 0.0 };
    let at = |history: &mut MovementHistory, frame: i32, layout: &DrawnLayout| {
        history.begin_frame(
            MovementFrame {
                head,
                overlay_layout: Some(layout.clone()),
                ..movement_frame(frame as f32 / 10.0, 196, 2.0)
            },
            SEEN,
        );
        history.at_head(unit, &moving).expect("the hours place it").at
    };
    assert_eq!(at(&mut history, 4, &older_layout), older.at(head));
    assert_eq!(at(&mut history, 5, &newer_layout), newer.at(head));
    assert_ne!(older.at(head), newer.at(head), "the two layouts place it apart");
    let standing = at(&mut history, 6, &unplaced_layout);
    let drawn = history.sample(unit, &moving).expect("its animator draws it");
    assert_eq!(standing, drawn.motion.position());
}

#[test]
fn a_held_layout_keeps_its_track_while_later_layouts_prepare() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let unplaced_in_held = Unit::Division(8);
    let never_placed = Unit::Division(9);
    let moving = live_position(1, &[2], 0.0, false);
    let other_moving = live_position(1, &[2], 20.0, false);
    let never_moving = live_position(1, &[2], 40.0, false);
    let mut history = settled_on(&moving, &[unit]);
    next_frame(&mut history, 1, 100, 2.0);
    let initial = DrawnLayout::new(100);
    assert_eq!(
        history.track(&initial, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement)
    );
    next_frame(&mut history, 2, 148, 2.0);
    let held_layout = DrawnLayout::new(148);
    let held_track = history
        .track(&held_layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("two places");
    next_frame(&mut history, 3, 196, 2.0);
    let later_one = DrawnLayout::new(196);
    let later_one_track = history
        .track(&later_one, unit, &[at_hour(196, [130.0, 0.0, 0.0])], 0.0)
        .expect("three places");
    assert_eq!(
        history.track(&later_one, unplaced_in_held, &[at_hour(196, [300.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement)
    );
    next_frame(&mut history, 4, 244, 2.0);
    let later_two = DrawnLayout::new(244);
    let later_two_track = history
        .track(&later_two, unit, &[at_hour(244, [210.0, 0.0, 0.0])], 0.0)
        .expect("four places");
    let _later_other_track = history
        .track(&later_two, unplaced_in_held, &[at_hour(244, [350.0, 0.0, 0.0])], 0.0)
        .expect("two places for unit 8");
    next_frame(&mut history, 5, 292, 2.0);
    let later_three = DrawnLayout::new(292);
    let later_three_track = history
        .track(&later_three, unit, &[at_hour(292, [310.0, 0.0, 0.0])], 0.0)
        .expect("five places");
    let head = Head { hour: 124, fraction: 0.0 };
    history.begin_frame(
        MovementFrame {
            head,
            overlay_layout: Some(held_layout.clone()),
            ..movement_frame(0.6, 292 + KEPT_HOURS + 48, 2.0)
        },
        SEEN,
    );
    let figure = history.at_head(unit, &moving).expect("held layout still places it");
    assert_eq!(figure.at, held_track.at(head));
    assert_eq!(figure.heading, None);
    let unplaced_figure = history
        .at_head(unplaced_in_held, &other_moving)
        .expect("unplaced unit uses its sampled placement");
    let unplaced_sampled = history.sample(unplaced_in_held, &other_moving).expect("sampled");
    assert_eq!(unplaced_figure.at, unplaced_sampled.motion.position());
    assert_eq!(unplaced_figure.heading, Some(unplaced_sampled.motion.position_heading().1));
    let never_figure = history
        .at_head(never_placed, &never_moving)
        .expect("never-placed unit uses its sampled placement");
    let never_sampled = history.sample(never_placed, &never_moving).expect("sampled");
    assert_eq!(never_figure.at, never_sampled.motion.position());
    assert_eq!(never_figure.heading, Some(never_sampled.motion.position_heading().1));
    let later_head = Head { hour: 220, fraction: 0.0 };
    for (frame, layout, expected) in [
        (7, &later_one, &later_one_track),
        (8, &later_two, &later_two_track),
        (9, &later_three, &later_three_track),
    ] {
        history.begin_frame(
            MovementFrame {
                head: later_head,
                overlay_layout: Some(layout.clone()),
                ..movement_frame(frame as f32 / 10.0, 292 + KEPT_HOURS + 48, 2.0)
            },
            SEEN,
        );
        let at = history.at_head(unit, &moving).expect("later layout places it");
        assert_eq!(at.at, expected.at(later_head));
        assert_eq!(at.heading, None);
    }
    history.begin_frame(
        MovementFrame {
            head: Head { hour: 130, fraction: 0.0 },
            overlay_layout: Some(held_layout.clone()),
            clock_speed: None,
            ..movement_frame(1.0, 292 + KEPT_HOURS + 48, 2.0)
        },
        SEEN,
    );
    assert_eq!(
        history.track(&held_layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::ClockSpeedUnavailable)
    );
    assert_eq!(history.at_head(unit, &moving), None);
    history.begin_frame(
        MovementFrame {
            head: Head { hour: 132, fraction: 0.0 },
            overlay_layout: Some(held_layout.clone()),
            clock_speed: Some(1),
            ..movement_frame(1.1, 292 + KEPT_HOURS + 48, 2.0)
        },
        SEEN,
    );
    let recovered = history
        .track(&held_layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("clock recovery restores held layout's saved track after placement eviction");
    assert_eq!(recovered, held_track);
    let next_head = Head { hour: 136, fraction: 0.0 };
    history.begin_frame(
        MovementFrame {
            head: next_head,
            overlay_layout: Some(held_layout.clone()),
            clock_speed: Some(1),
            ..movement_frame(1.2, 292 + KEPT_HOURS + 48, 2.0)
        },
        SEEN,
    );
    let recovered_figure = history
        .at_head(unit, &moving)
        .expect("recovered held layout keeps figure on counter track");
    assert_eq!(recovered_figure.at, recovered.at(next_head));
    assert_eq!(recovered_figure.at, held_track.at(next_head));
    assert_eq!(recovered_figure.heading, None);
}

#[test]
fn retiring_old_layouts_prunes_their_retained_tracks() {
    let unit = Unit::Division(7);
    let mut history = MovementHistory::default();
    next_frame(&mut history, 1, 100, 2.0);
    let first = DrawnLayout::new(100);
    let _ = history.track(&first, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
    let kept = DrawnLayout::new(148);
    let kept_track = history
        .track(&kept, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("moving track");
    assert_eq!(history.layouts.len(), 2);
    drop(first);
    for step in 0..8 {
        let hour = 196 + step * 48;
        let transient = DrawnLayout::new(hour);
        let _ = history.track(
            &transient,
            unit,
            &[at_hour(hour, [100.0 + step as f32 * 40.0, 0.0, 0.0])],
            0.0,
        );
        assert_eq!(history.layouts.len(), 2, "retired transient layouts do not accumulate");
    }
    next_frame(&mut history, 2, 600, 2.0);
    assert_eq!(history.layouts.len(), 1);
    assert_eq!(history.layout_track(&kept, unit), Some(Ok(kept_track)));
    drop(kept);
    next_frame(&mut history, 3, 601, 2.0);
    assert!(history.layouts.is_empty());
}

#[test]
fn named_track_refusals_stay_keyed_to_a_held_layout() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = MovementHistory::default();
    let no_frame_layout = DrawnLayout::new(90);
    let steady_refusal = history
        .track(&no_frame_layout, unit, &[at_hour(90, [5.0, 0.0, 0.0])], 0.0)
        .expect_err("no frame refuses track");
    assert_eq!(steady_refusal, TrackRefusal::SteadyOrNoFrame);
    assert_eq!(steady_refusal.reason(), "the frame is steady or none");
    assert_eq!(steady_refusal.to_string(), "the frame is steady or none");

    next_frame(&mut history, 1, 100, 2.0);
    let first_layout = DrawnLayout::new(100);
    let first_refusal = history
        .track(&first_layout, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0)
        .expect_err("single place refuses track");
    assert_eq!(first_refusal, TrackRefusal::NoPriorPlacement);
    assert_eq!(first_refusal.reason(), "no publication before this one placed it");

    next_frame(&mut history, 2, 148, 2.0);
    let still_layout = DrawnLayout::new(148);
    let still_refusal = history
        .track(&still_layout, unit, &[at_hour(148, [10.0, 0.0, 0.0])], 0.0)
        .expect_err("unchanged place refuses track");
    assert_eq!(still_refusal, TrackRefusal::UnchangedPlacement);
    assert_eq!(still_refusal.reason(), "the recording left it where it was");

    next_frame(&mut history, 3, 196, 2.0);
    let moving_one = DrawnLayout::new(196);
    assert!(history.track(&moving_one, unit, &[at_hour(196, [58.0, 0.0, 0.0])], 0.0).is_ok());
    next_frame(&mut history, 4, 244, 2.0);
    let moving_two = DrawnLayout::new(244);
    assert!(history.track(&moving_two, unit, &[at_hour(244, [130.0, 0.0, 0.0])], 0.0).is_ok());

    assert_eq!(
        history.layout_track(&no_frame_layout, unit),
        Some(Err(TrackRefusal::SteadyOrNoFrame))
    );
    assert_eq!(
        history.layout_track(&first_layout, unit),
        Some(Err(TrackRefusal::NoPriorPlacement))
    );
    assert_eq!(
        history.layout_track(&still_layout, unit),
        Some(Err(TrackRefusal::UnchangedPlacement))
    );

    history.begin_frame(
        MovementFrame {
            head: Head { hour: 124, fraction: 0.0 },
            overlay_layout: Some(still_layout.clone()),
            ..movement_frame(0.5, 244, 2.0)
        },
        SEEN,
    );
    let figure = history
        .at_head(unit, &moving)
        .expect("refused track falls back to sampled placement");
    let sampled = history.sample(unit, &moving).expect("sampled");
    assert_eq!(figure.at, sampled.motion.position());
    assert_eq!(figure.heading, Some(sampled.motion.position_heading().1));
}

#[test]
fn same_hour_rebuild_publishes_tracks_before_figures_read_on_first_frame() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = settled_on(&moving, &[unit]);
    next_frame(&mut history, 1, 100, 2.0);
    let initial = DrawnLayout::new(100);
    let _ = history.track(&initial, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
    let first_layout = DrawnLayout::new(148);
    let head = Head { hour: 124, fraction: 0.0 };
    history.begin_frame(
        MovementFrame {
            head,
            overlay_layout: Some(first_layout.clone()),
            ..movement_frame(0.2, 148, 2.0)
        },
        SEEN,
    );
    let first_track = history
        .track(&first_layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("first layout track");
    let rebuilt_layout = DrawnLayout::new(148);
    let rebuilt_track = history
        .track(&rebuilt_layout, unit, &[at_hour(148, [82.0, 0.0, 0.0])], 0.0)
        .expect("rebuilt layout track");
    assert_ne!(first_track.at(head), rebuilt_track.at(head));
    drop(first_layout);
    history.set_overlay_layout(Some(rebuilt_layout.clone()));
    let figure = history.at_head(unit, &moving).expect("figure placed");
    assert_eq!(figure.at, rebuilt_track.at(head));
    assert_eq!(figure.heading, None);
    let reprojected_layout = DrawnLayout::new(148);
    history.begin_frame(
        MovementFrame {
            head,
            overlay_layout: Some(reprojected_layout.clone()),
            ..movement_frame(0.3, 148, 2.0)
        },
        SEEN,
    );
    let reprojected_track = history
        .track(&reprojected_layout, unit, &[at_hour(148, [94.0, 0.0, 0.0])], 0.0)
        .expect("reprojected layout track");
    history.set_overlay_layout(Some(reprojected_layout));
    let figure = history.at_head(unit, &moving).expect("reprojected first frame places figure");
    assert_eq!(figure.at, reprojected_track.at(head));
    assert_eq!(figure.heading, None);
}

#[test]
fn continuity_reset_keeps_live_layout_tracks_without_leaking_to_fresh_same_hour_layout() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = settled_on(&moving, &[unit]);
    next_frame(&mut history, 1, 100, 2.0);
    let initial = DrawnLayout::new(100);
    let _ = history.track(&initial, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
    next_frame(&mut history, 2, 148, 2.0);
    let held_layout = DrawnLayout::new(148);
    let untracked_pre_reset = DrawnLayout::new(148);
    let held_track = history
        .track(&held_layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("two places");
    drop(initial);
    history.clear();
    assert_eq!(history.layouts.len(), 1);
    let head = Head { hour: 124, fraction: 0.0 };
    history.begin_frame(
        MovementFrame {
            head,
            overlay_layout: Some(held_layout.clone()),
            ..movement_frame(0.3, 148, 2.0)
        },
        SEEN,
    );
    let figure = history
        .at_head(unit, &moving)
        .expect("still-held layout survives continuity reset");
    assert_eq!(figure.at, held_track.at(head));
    let repainted = history
        .track(&held_layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("repainted pre-reset layout keeps its pre-reset track");
    assert_eq!(repainted, held_track);
    assert_eq!(
        history.track(&untracked_pre_reset, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::ContinuityReset),
        "untracked pre-reset layout is refused without seeding post-reset places"
    );
    let post_gap = DrawnLayout::at_continuity(196, history.continuity());
    assert_eq!(
        history.track(&post_gap, unit, &[at_hour(196, [130.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement),
        "pre-reset repaint must not seed a track across the continuity gap"
    );
    let fresh_same_hour = DrawnLayout::at_continuity(148, history.continuity());
    assert_eq!(history.layout_track(&fresh_same_hour, unit), None);
    drop(held_layout);
    drop(untracked_pre_reset);
    history.set_overlay_layout(Some(post_gap));
    assert_eq!(history.layouts.len(), 1);
}

#[test]
fn missing_clock_speed_refuses_track_so_counter_and_figure_agree() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = MovementHistory::default();
    let layout = DrawnLayout::new(148);
    history.begin_frame(
        MovementFrame {
            head: Head { hour: 124, fraction: 0.0 },
            overlay_layout: Some(layout.clone()),
            clock_speed: None,
            ..movement_frame(0.1, 148, 2.0)
        },
        SEEN,
    );
    let outcome = history.track(
        &layout,
        unit,
        &[at_hour(100, [10.0, 0.0, 0.0]), at_hour(148, [58.0, 0.0, 0.0])],
        0.0,
    );
    assert_eq!(outcome, Err(TrackRefusal::ClockSpeedUnavailable));
    assert_eq!(
        TrackRefusal::ClockSpeedUnavailable.reason(),
        "clock speed is unavailable"
    );
    assert_eq!(
        history.layout_track(&layout, unit),
        Some(Err(TrackRefusal::ClockSpeedUnavailable))
    );
    assert_eq!(history.at_head(unit, &moving), None);
}

#[test]
fn reused_painted_layout_keeps_figure_on_track_when_clock_speed_drops() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = settled_on(&moving, &[unit]);
    next_frame(&mut history, 1, 100, 2.0);
    let initial = DrawnLayout::new(100);
    let _ = history.track(&initial, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
    let painted = DrawnLayout::new(148);
    let head = Head { hour: 124, fraction: 0.0 };
    history.begin_frame(
        MovementFrame {
            head,
            overlay_layout: Some(painted.clone()),
            clock_speed: Some(1),
            ..movement_frame(0.2, 148, 2.0)
        },
        SEEN,
    );
    let track = history
        .track(&painted, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("painted track");
    let clockless_head = Head { hour: 136, fraction: 0.0 };
    history.begin_frame(
        MovementFrame {
            head: clockless_head,
            overlay_layout: Some(painted.clone()),
            clock_speed: None,
            ..movement_frame(0.3, 148, 2.0)
        },
        SEEN,
    );
    let figure = history
        .at_head(unit, &moving)
        .expect("reused painted batch still carries the figure along its track");
    assert_eq!(figure.at, track.at(clockless_head));
    assert_eq!(figure.heading, None);
    assert_eq!(
        history.track(&painted, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::ClockSpeedUnavailable)
    );
    assert_eq!(history.at_head(unit, &moving), None);
}

#[test]
fn destination_layout_created_before_continuity_reset_is_not_rejected() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let mut history = MovementHistory::default();
    next_frame(&mut history, 1, 148, 2.0);
    let destination = DrawnLayout::new(196);
    let epoch = history.clear();
    destination.set_continuity(epoch);
    history.begin_frame(
        MovementFrame {
            head: Head { hour: 190, fraction: 0.0 },
            overlay_layout: Some(destination.clone()),
            clock_speed: Some(1),
            ..movement_frame(0.2, 196, 2.0)
        },
        SEEN,
    );
    let track = history
        .track(
            &destination,
            unit,
            &[at_hour(180, [100.0, 0.0, 0.0]), at_hour(196, [130.0, 0.0, 0.0])],
            0.0,
        )
        .expect("destination layout must not be rejected as pre-reset");
    assert_eq!(track.places().len(), 2);
}

#[test]
fn reprojected_pre_reset_layout_cannot_seed_post_gap_track() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let mut history = MovementHistory::default();
    next_frame(&mut history, 1, 100, 2.0);
    let initial = DrawnLayout::new(100);
    let _ = history.track(&initial, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
    next_frame(&mut history, 2, 148, 2.0);
    let old_layout = DrawnLayout::new(148);
    let _ = history
        .track(&old_layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("pre-reset track");
    let epoch = history.clear();
    let reprojected = old_layout.reproject();
    history.begin_frame(
        MovementFrame {
            head: Head { hour: 196, fraction: 0.0 },
            overlay_layout: Some(reprojected.clone()),
            clock_speed: Some(1),
            ..movement_frame(0.3, 196, 2.0)
        },
        SEEN,
    );
    assert_eq!(
        history.track(&reprojected, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::ContinuityReset)
    );
    let post_gap = DrawnLayout::at_continuity(196, epoch);
    assert_eq!(
        history.track(&post_gap, unit, &[at_hour(196, [130.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement),
        "reprojected stale layout must not seed a track across the gap"
    );
}

#[test]
fn pre_reset_repaint_sample_does_not_continue_old_route_after_reset() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let old_route = live_position(1, &[2, 3], 0.0, false);
    let post_gap_route = live_position(2, &[3], 10.0, false);
    let mut history = settled_on(&old_route, &[unit]);
    next_frame(&mut history, 1, 148, 2.0);
    let old_layout = DrawnLayout::new(148);
    let _ = history.track(&old_layout, unit, &[at_hour(148, [1.0, 0.0, 0.0])], 0.0);
    let epoch = history.clear();
    history.begin_frame(
        MovementFrame {
            head: Head { hour: 196, fraction: 0.0 },
            overlay_layout: Some(old_layout.clone()),
            clock_speed: Some(1),
            ..movement_frame(0.2, 196, 2.0)
        },
        SEEN,
    );
    assert_eq!(history.sample_for(&old_layout, unit, &old_route), None);
    let _ = history.track(&old_layout, unit, &[at_hour(148, [1.0, 0.0, 0.0])], 0.0);
    let post_gap = DrawnLayout::at_continuity(196, epoch);
    history.begin_frame(
        MovementFrame {
            head: Head { hour: 196, fraction: 0.0 },
            overlay_layout: Some(post_gap.clone()),
            clock_speed: Some(1),
            ..movement_frame(0.3, 196, 2.0)
        },
        SEEN,
    );
    let figure = history.at_head(unit, &post_gap_route).expect("post-gap sampled");
    assert!(
        figure.at[0] >= post_gap_route.standing[0],
        "pre-reset sample leaked old arrival route across reset: {:?}",
        figure.at
    );
    assert!(history.sample_for(&post_gap, unit, &post_gap_route).is_some());
}

#[test]
fn clock_recovery_does_not_diverge_from_clockless_painted_layout() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = settled_on(&moving, &[unit]);
    next_frame(&mut history, 1, 100, 2.0);
    let initial = DrawnLayout::new(100);
    let _ = history.track(&initial, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
    let layout = DrawnLayout::new(148);
    history.begin_frame(
        MovementFrame {
            head: Head { hour: 124, fraction: 0.0 },
            overlay_layout: Some(layout.clone()),
            clock_speed: Some(1),
            ..movement_frame(0.2, 148, 2.0)
        },
        SEEN,
    );
    assert!(history.track(&layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0).is_ok());
    history.begin_frame(
        MovementFrame {
            head: Head { hour: 130, fraction: 0.0 },
            overlay_layout: Some(layout.clone()),
            clock_speed: None,
            ..movement_frame(0.3, 148, 2.0)
        },
        SEEN,
    );
    assert_eq!(
        history.track(&layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::ClockSpeedUnavailable)
    );
    assert_eq!(history.at_head(unit, &moving), None);
    history.begin_frame(
        MovementFrame {
            head: Head { hour: 136, fraction: 0.0 },
            overlay_layout: Some(layout.clone()),
            clock_speed: Some(1),
            ..movement_frame(0.4, 148, 2.0)
        },
        SEEN,
    );
    assert_eq!(
        history.at_head(unit, &moving),
        None,
        "reused clockless painted layout must keep figure settled until repainted"
    );
    let recovered_track = history
        .track(&layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("clock recovery restores the moving track instead of downgrading to NoPriorPlacement");
    let reused_head = Head { hour: 140, fraction: 0.0 };
    history.begin_frame(
        MovementFrame {
            head: reused_head,
            overlay_layout: Some(layout.clone()),
            clock_speed: Some(1),
            ..movement_frame(0.5, 148, 2.0)
        },
        SEEN,
    );
    let reused_figure = history
        .at_head(unit, &moving)
        .expect("reused recovered layout keeps figure on counter track");
    assert_eq!(reused_figure.at, recovered_track.at(reused_head));
    assert_eq!(reused_figure.heading, None);

    let post_reset_epoch = history.clear();
    history.begin_frame(
        MovementFrame {
            head: Head { hour: 130, fraction: 0.0 },
            overlay_layout: Some(layout.clone()),
            clock_speed: None,
            ..movement_frame(0.6, 148, 2.0)
        },
        SEEN,
    );
    assert_eq!(
        history.track(&layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::ClockSpeedUnavailable)
    );
    assert_eq!(history.at_head(unit, &moving), None);
    let recovered_pre_reset_head = Head { hour: 142, fraction: 0.0 };
    history.begin_frame(
        MovementFrame {
            head: recovered_pre_reset_head,
            overlay_layout: Some(layout.clone()),
            clock_speed: Some(1),
            ..movement_frame(0.7, 148, 2.0)
        },
        SEEN,
    );
    let restored_pre_reset = history
        .track(&layout, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("clock recovery restores a retained pre-reset layout's saved track");
    assert_eq!(restored_pre_reset, recovered_track);
    let figure_after_pre_reset_recovery = history
        .at_head(unit, &moving)
        .expect("figure moves with restored pre-reset track");
    assert_eq!(
        figure_after_pre_reset_recovery.at,
        restored_pre_reset.at(recovered_pre_reset_head)
    );
    let post_gap = DrawnLayout::at_continuity(196, post_reset_epoch);
    assert_eq!(
        history.track(&post_gap, unit, &[at_hour(196, [130.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement)
    );
}

#[test]
fn same_hour_layouts_with_different_endpoints_do_not_false_trigger_actor_judge() {
    use viewer_gpu::icon_tracks::Head;
    let unit = Unit::Division(7);
    let refused_unit = Unit::Division(8);
    let absent_unit = Unit::Division(9);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = settled_on(&moving, &[unit]);
    history.force_judge = true;
    next_frame(&mut history, 1, 100, 2.0);
    let initial = DrawnLayout::new(100);
    let _ = history.track(&initial, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
    next_frame(&mut history, 2, 148, 2.0);
    let older_same_hour = DrawnLayout::new(148);
    let older_track = history
        .track(&older_same_hour, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0)
        .expect("older track");
    assert_eq!(
        history.track(&older_same_hour, refused_unit, &[at_hour(148, [12.0, 0.0, 0.0])], 0.0),
        Err(TrackRefusal::NoPriorPlacement)
    );
    let newer_same_hour = DrawnLayout::new(148);
    let newer_track = history
        .track(&newer_same_hour, unit, &[at_hour(148, [78.0, 0.0, 0.0])], 0.0)
        .expect("newer track at same hour");
    assert!(history.layouts[&older_same_hour.id()].tracks[&unit].reference.is_some());
    assert!(history.layouts[&newer_same_hour.id()].tracks[&unit].reference.is_some());
    let head = Head { hour: 124, fraction: 0.0 };
    history.begin_frame(
        MovementFrame {
            head,
            overlay_layout: Some(older_same_hour.clone()),
            ..movement_frame(0.3, 148, 2.0)
        },
        SEEN,
    );
    let older_figure = history.at_head(unit, &moving).expect("older layout track");
    assert_eq!(older_figure.at, older_track.at(head));
    assert_eq!(
        history.judge_unit(&older_same_hour, unit, head, Some(older_figure.at)),
        JudgeOutcome::Matched { apart: 0.0 }
    );
    assert_eq!(
        history.judge_unit(&older_same_hour, refused_unit, head, None),
        JudgeOutcome::Refused(TrackRefusal::NoPriorPlacement)
    );
    assert_eq!(
        history.judge_unit(&older_same_hour, absent_unit, head, None),
        JudgeOutcome::Absent
    );
    history.set_overlay_layout(Some(newer_same_hour.clone()));
    let newer_figure = history.at_head(unit, &moving).expect("newer layout track");
    assert_eq!(newer_figure.at, newer_track.at(head));
    assert_eq!(
        history.judge_unit(&newer_same_hour, unit, head, Some(newer_figure.at)),
        JudgeOutcome::Matched { apart: 0.0 }
    );
    assert_ne!(older_figure.at, newer_figure.at);
    if !judging() {
        let mut unjudged = MovementHistory::default();
        next_frame(&mut unjudged, 1, 100, 2.0);
        let first = DrawnLayout::new(100);
        let _ = unjudged.track(&first, unit, &[at_hour(100, [10.0, 0.0, 0.0])], 0.0);
        let second = DrawnLayout::new(148);
        assert!(unjudged.track(&second, unit, &[at_hour(148, [58.0, 0.0, 0.0])], 0.0).is_ok());
        assert!(unjudged.layouts[&second.id()].tracks[&unit].reference.is_none());
    }
}

#[test]
fn sparse_high_in_view_province_ids_keep_movement_history_bounded() {
    let mut history = MovementHistory::default();
    let unit = Unit::Division(7);
    let moving = live_position(u32::MAX, &[u32::MAX - 1], 0.0, false);
    for frame in 0..4 {
        history.begin_frame(
            movement_frame(frame as f32 / 10.0, 100, 2.0),
            [0, 64, 4_000_000_011, u32::MAX - 1, u32::MAX],
        );
        assert!(history.in_view.slot_capacity() <= 32);
        let _ = history.sample(unit, &moving);
    }
    assert!(history.in_view.contains(u32::MAX));
    assert!(!history.in_view.contains(u32::MAX - 2));
}

#[test]
fn a_division_s_route_made_alone_equals_the_whole_hour_s() {
    use replay_data::rows::{DivisionMovementRow, DivisionRow, TableRows, tables};
    use viewer_foundation::movement_timing::{
        Etas, HourTerms, estimate_one,
        test_support::{DIVISION, SPEED, content, hour, put, two_leg_map},
    };
    use viewer_foundation::tables::rows;

    let map = content(two_leg_map());
    let anchor = |province: u32| Some([province as f32 * 10.0, 0.0, 1.0]);
    let (whole_memo, alone_memo) = (RouteMemo::default(), RouteMemo::default());
    let standing = DivisionMovementRow { division: DIVISION + 1, ..Default::default() };
    for (at, speed) in [Some(SPEED), Some(SPEED), None].into_iter().enumerate() {
        let mut frame = hour(speed);
        frame.hour += at as i32;
        let mut census = rows::<tables::Divisions>(&frame).unwrap().rows().to_vec();
        census.push(DivisionRow { id: DIVISION + 1, province: 2, ..Default::default() });
        put(&mut frame, TableRows::Divisions(census));
        let mut movement = rows::<tables::DivisionMovement>(&frame).unwrap().rows().to_vec();
        movement.push(standing.clone());
        put(&mut frame, TableRows::DivisionMovement(movement));
        let etas = Etas::read(&frame, Some(&map)).unwrap();
        let whole = DivisionRoutes::read_kept(&frame, &etas, Some(&map.map), &anchor, &whole_memo).unwrap();
        let terms = HourTerms::new(&frame);
        let census = rows::<tables::Divisions>(&frame).unwrap().rows();
        for row in rows::<tables::DivisionMovement>(&frame).unwrap().rows() {
            let division = census.iter().find(|division| division.id == row.division);
            let eta = estimate_one(row, division, &terms, Some(&map));
            assert_eq!(eta, etas.get(row.division), "hour {at}, division {}", row.division);
            let alone = DivisionRoutes::route_of(row, division.map_or(0, |division| division.province), eta, Some(&map.map), &anchor, &alone_memo);
            assert_eq!(Some(&alone), whole.by_division.get(&row.division), "hour {at}, division {}", row.division);
        }
    }
}
