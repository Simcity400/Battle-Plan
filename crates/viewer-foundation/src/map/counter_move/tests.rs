use super::*;
use crate::map::province_map::WATER_HEIGHT;
use crate::movement_timing::Etas;
use crate::movement_timing::MovementContent;
use installed_content_core::{movement::Neighbour, movement::ProvinceTerrain};

const W: f32 = 5632.0;

/// One place of a publication's own hour, which is what a layout hands for a mover whose hours did
/// not move it inside the interval.
fn at_hour(hour: i32, at: [f32; 3]) -> crate::map::free_icons::Place {
    crate::map::free_icons::Place { hour, at }
}
/// The height [`Motion::new`]'s flat field samples.
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
    // A native zero-length next-anchor sample takes the destination scale.
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
    // If the next anchor is sample zero there is no positional motion, but
    // the initialized route still changes scale to the next province's.
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
    // The skipped final hop leaves three controls. Initialization clears the
    // animator's count even though the unit's ordinary route remains nonempty.
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
    // Main used route length and returned kind 20 here; native admits a
    // docked plate without any ordinary formation.
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
    // An empty animator route falls back to
    // ordinary[0]. With samples, the animator supplies display[0] instead.
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

/// A line of provinces 1-2-3-4-5, where 8 neighbours 2 but has no anchor
/// and 9 has an anchor but no neighbour.
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
        // The next hop is tested against the same province, 2.
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
    // The first hop skipped: no phantom handle before the unit's anchor.
    let walk = walk_route(&map, 1, &[9, 2, 3], &line_anchor);
    assert_eq!(walk.skipped, 1);
    let points = route_control_points(W, &walk.hops);
    assert_eq!(points[0], [10.0, 0.0, 5.0]);
    assert_eq!(points.len(), 4);
    // The last hop skipped: 3's anchor is never appended, and three
    // controls are no movement at all.
    let walk = walk_route(&map, 1, &[2, 3, 9], &line_anchor);
    assert_eq!(walk.skipped, 1);
    assert_eq!(route_control_points(W, &walk.hops).len(), 3);
    assert_eq!(line_motion(&map, &[2, 3, 9]), None);
    // No province of the route kept: nothing to draw.
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
    // Two controls past 2's anchor are fixed by hop 5; the unreachable
    // last hop is never examined and does not move the point.
    let walk = walk_route(&map, 1, &[2, 3, 4, 5, 9], &line_anchor);
    assert_eq!((walk.hops.len(), walk.skipped), (4, 0));
    assert!(walk.hops.iter().all(|hop| !hop.last));
    assert_eq!(
        line_motion(&map, &[2, 3, 4, 5, 9]),
        line_motion(&map, &[2, 3, 4, 5])
    );
    // Until then a skipped hop is examined and counted.
    assert_eq!(walk_route(&map, 1, &[2, 3, 4, 9], &line_anchor).skipped, 1);
}

#[test]
fn route_height_is_sampled_after_flat_control_points() {
    let anchors = [[0.0, 22.0, 1.0], [3.0, 23.0, 1.0]];
    let values = flat(120);
    let motion = Motion::new_with_crossings(W, &anchors, &[Some([1, 1])], &field(&values))
        .expect("valid crossing route");
    let at = motion.position();
    // One world unit along the flat hop, on the terrain rather than the anchors' height.
    assert!((at[0] - 1.0).abs() < 0.05, "{at:?}");
    assert_eq!(at[1], 12.0);
}

#[test]
fn the_settled_counter_sits_one_world_unit_along_its_route() {
    // `g_TickThreshold` clamps the free arc while the unit is still in its
    // province: however far the simulation has moved it, the counter stands
    // 1.0 along the spline from its own anchor.
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
    // The interpolation parameter is not clamped: with t[A] < 1.0 the
    // search stops at A and the engine's lerp carries on past it.
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
    // Heading is the segment's own atan2(dz, dx): almost straight along +x.
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
    // Unwrapped, the eastern anchor is eight units to the *west* of the western one.
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
    // Capped at A = 2 (arc 0.8), the lerp runs on to 1.0 along the last segment.
    close(point_on(&samples, 2, 1.0), [1.0, 0.0, 0.0], 1e-5);
    // A degenerate segment snaps to its end.
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
    // A rebuilt route is moving before its simulation progress advances.
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

/// Only a kind rebuilt from content is marked native: the combat wedge
/// must not take the standstill fallback of a route content cannot place.
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
    // 1 attacks toward 2; 2 defends; 3 attacks with an unplaceable first
    // hop; 4 attacks with no Divisions row, so no location.
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
    // Timing content knows provinces 1 and 9 without an edge between them,
    // so 3's first hop takes the native 100000 distance; the counter map
    // below still cannot place it.
    let timing = MovementContent {
        map: MovementMap::from_parts(&[(1, LAND, Vec::new()), (9, LAND, Vec::new())]),
        defines: replay_derive::eta_derive::Defines {
            outer_scale: 100_000,
            ..Default::default()
        },
        rail: Ok(installed_content_core::movement::RailMovementDefines {
            speed: [0; 4],
            maximum_level: 0,
        }),
        task_force_terrain: Err("unrelated task-force terrain is unavailable".into()),
        land: Ok(crate::movement_timing::LandSpeedContent {
            terrain_movement: vec![0],
            ..Default::default()
        }),
    };
    // Stateless provinces: no infrastructure term.
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
            crate::movement_timing::test_support::neutral_status(3, 1),
        ]));
    crate::movement_timing::test_support::neutral_world(&mut frame, 1);
    let etas = Etas::read(&frame, Some(&timing)).unwrap();
    // Only 3 moves with a base speed: one hour at 1 km/h. 1 and 2 have no
    // route and 4 has no location.
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
    // The attacker's directional kind toward the combat province.
    assert_eq!(first, (1, true));
    assert_eq!(defending, (10, true));
    assert!(!unplaced.1);
    assert!(!unlocated.1);
    // Without movement content only the defending kind is native.
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
    // A province without content terrain is not a stationary land unit.
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
    // 1 is a land (port) province and 6 sea, each with neighbours 2, 3, 4 in native order.
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
    // Standing: docked in a land province, slot 0 at sea.
    assert_eq!(kind(&row, 1), Some(19));
    assert_eq!(kind(&row, 6), Some(0));
    // Moving: 20 out of a land province, the division's land kinds from sea.
    row.path = vec![3];
    assert_eq!(kind(&row, 1), Some(20));
    assert_eq!(kind(&row, 6), Some(2));
    // In combat: 9 attacking, 10 defending, both snapped to their stack anchor.
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
    // 1 is land and 6 sea, each with neighbours 2, 3, 4 in that native order.
    let map = MovementMap::from_parts(&[
        (1, LAND, land(&[2, 3, 4])),
        (2, LAND, land(&[1, 6])),
        (3, LAND, land(&[1, 6])),
        (4, LAND, land(&[1, 6])),
        (6, SEA, land(&[2, 3, 4])),
    ]);
    // Every province but 9 has a `Standstill RG` anchor.
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
    // The alternate route is the displayed one.
    row.alternate_path = vec![4];
    assert_eq!(kind(&row, 1), Some(24));
    // A set movement target with no displayed route stands.
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
    // A gun has no stand-off kind 9, and a division's directional kinds are not a gun's.
    assert!(!snaps_to_stack(gun, 9, 1, 1));
    assert!(!snaps_to_stack(gun, 2, 1, 1));
    assert!(!snaps_to_stack(gun, 30, 1, 1));
    assert!(!snaps_to_stack(gun, 22, 0, 0));
    row.combat_role = 2;
    assert_eq!(kind(&row, 1), Some(10));
    assert!(snaps_to_stack(gun, 10, 2, 1));
    // Only the first hop is checked: 5 is not a neighbour of 1 and 9 has no gun anchor.
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
    // A province without content terrain is not standing still.
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
    // Moving without attacking: the animator's point over the slot-21 anchors, never the
    // directional kind's anchor.
    let position = route.position(1, 22, &world, &anchor).expect("anchored");
    assert_eq!(position.standing, [121.0, 0.0, 1.0]);
    let at = position.at();
    assert!((at[0] - 122.0).abs() < 1e-3, "{at:?}");
    assert_eq!(
        route.standing(1, true, 22, &anchor),
        Some([121.0, 0.0, 1.0])
    );
    // Attacking from land it snaps to its directional stack anchor.
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
    // Defending, it takes the stand-off anchor.
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
    // A defender's stand-off slot snaps to its stack anchor.
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

/// The same route with its stack snapped to its combat anchor.
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
        head: crate::map::anchored::Head { hour, fraction: 0.0 },
        overlay_hour: hour,
        clock_speed: Some(0),
        speeds: [speed; 2],
        steady: false,
    }
}

/// A history whose render delta has settled on 0.1 s frames (the sorted ring takes about 160 to
/// fill), in which `units` have run from their anchors to the settled arc of `old` at hour 100,
/// its last frame at time zero.
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
    // One time among 31 zeros sorts to the top, outside the middle 26.
    assert_eq!(delta.step(0.1), 0.05);
    // The cursor writes over the sorted ring, so a steady rate fills it in about five sweeps.
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

/// What moves is placed at the frame's own head, not at the publication's: between two
/// publications the hour stands still and the counter is still advanced every display frame, and
/// the same frames leave it in the same place whether the publication before it covered 6 game
/// hours or 24. So the simulation's rate never places anything that moves.
#[test]
fn a_gliding_counter_is_placed_at_every_display_frame_between_two_publications() {
    for unit in [Unit::Division(7), Unit::TaskForce(7)] {
        // Four display frames to one publication, each covering `hours` of the clock.
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
    // The counter and the figure build their own copies of the same route.
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
    // A reroute rebuilds at the province's anchor and climbs toward the settled arc.
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
    // Six days after the glide's first advance it gives up and rebuilds at the new anchor.
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
    // One world unit plus one frame along the old route's ten: a fifth of its scale step.
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

/// A mover is placed from the recording's own rows, never from a prediction: its publication
/// carries where this hour's rows put it and where the publication before read it, and the display
/// frame interpolates between the two at its own head. The first publication to place a unit
/// carries no pair, so its counter stands exactly where its rows put it until the next one lands.
///
/// The places a route and its recorded estimate predict stand 14.87 world units from the next
/// publication's own placement at the top speed, 227 pixels at the stress test's camera. So the
/// places are read, not computed.
#[test]
fn a_mover_carries_the_two_publications_own_places_and_no_prediction() {
    let unit = Unit::Division(7);
    let mut history = MovementHistory::default();
    next_frame(&mut history, 0, 100, 2.0);
    assert_eq!(
        history.track(unit, &[at_hour(100, [10.0, 0.0, 0.0])], 100, 0.0),
        None,
        "the first publication to place it has nothing to move it from"
    );
    // The same publication's layout made again (the art it was waiting for arrived) is the same
    // hour's placement, not a second one.
    assert_eq!(history.track(unit, &[at_hour(100, [10.0, 0.0, 0.0])], 100, 0.0), None);
    let track = history
        .track(unit, &[at_hour(148, [58.0, 0.0, 0.0])], 148, 0.0)
        .expect("two publications' places");
    assert_eq!(track.places().len(), 2);
    assert_eq!((track.places()[0].hour, track.places()[0].at), (100, [10.0, 0.0, 0.0]));
    assert_eq!((track.places()[1].hour, track.places()[1].at), (148, [58.0, 0.0, 0.0]));
    // Every display frame of the interval stands between the two, by its own hour.
    let at = |hour, fraction| track.at(crate::map::anchored::Head { hour, fraction })[0];
    assert_eq!([at(100, 0.0), at(124, 0.0), at(148, 0.0)], [10.0, 34.0, 58.0]);
    assert!((at(100, 0.5) - 10.5).abs() < 1e-3, "{}", at(100, 0.5));
    // The layout of the publication before is still asked for while the frame draws it.
    let held = history.track(unit, &[at_hour(100, [10.0, 0.0, 0.0])], 100, 0.0);
    assert_eq!(held, None, "hour 100 has only hour 100's own place before it");
    // A third publication carries all three places, so a head anywhere among them has a pair.
    let longer = history.track(unit, &[at_hour(196, [106.0, 0.0, 0.0])], 196, 0.0).expect("three places");
    assert_eq!(longer.places().len(), 3);
    assert_eq!(longer.at(crate::map::anchored::Head { hour: 124, fraction: 0.0 })[0], 34.0);
}

/// A mover's places are the hours its own rows moved it at, so a display frame stands it where the
/// recording had it at that frame's hour. The publication's own rows give one place for the
/// interval's end: on them alone a unit that crossed a province in the middle of an interval is
/// glided evenly across the whole of it, and a frame at any hour inside it stands the unit
/// somewhere the recording never put it.
#[test]
fn a_movers_places_are_the_hours_it_moved_at_and_not_the_intervals_ends() {
    use crate::map::anchored::Head;
    let unit = Unit::Division(7);
    let (first, second) = ([10.0, 0.0, 0.0], [58.0, 0.0, 0.0]);
    let at = |history: &mut MovementHistory, places: &[crate::map::free_icons::Place], hour| {
        history.track(unit, places, hour, 0.0)
    };
    let mut hourly = MovementHistory::default();
    next_frame(&mut hourly, 0, 100, 2.0);
    at(&mut hourly, &[at_hour(100, first)], 100);
    // The recording moved it into its new province at hour 130, a third of the way through the
    // interval the publication of hour 148 covers, and left it there.
    let track = at(&mut hourly, &[at_hour(130, second), at_hour(148, second)], 148).expect("the hours' places");
    let place = |hour, fraction| track.at(Head { hour, fraction })[0];
    assert_eq!(place(100, 0.0), first[0], "the hour it was read in its old province");
    assert_eq!(place(115, 0.0), 34.0, "half the hours from 100 to 130 is half the way");
    assert_eq!(place(130, 0.0), second[0], "the hour it crossed");
    assert_eq!(place(140, 0.0), second[0], "the hours the recording left it there");
    assert!(track.held_still(Head { hour: 140, fraction: 0.0 }), "standing there is the recording's own");
    assert!(!track.held_still(Head { hour: 115, fraction: 0.0 }), "the hours move it there");

    // The same two publications without the hours: every hour of the interval is a place the
    // recording never read, and the unit is still moving at hour 140, where it had arrived.
    let mut coarse = MovementHistory::default();
    next_frame(&mut coarse, 0, 100, 2.0);
    at(&mut coarse, &[at_hour(100, first)], 100);
    let whole = at(&mut coarse, &[at_hour(148, second)], 148).expect("the publications' places");
    let evenly = |hour| whole.at(Head { hour, fraction: 0.0 })[0];
    assert_eq!(evenly(124), 34.0, "the interval's middle, not the hour it crossed");
    assert!(evenly(140) < second[0], "still gliding at an hour it had arrived at");
}

/// A publication whose steps carried no hours — a seek's reset, a piece's crossing, a catch-up —
/// places its movers at its own hour and nothing is invented for the hours between.
#[test]
fn a_publication_without_hours_places_its_movers_at_its_own_hour_alone() {
    use crate::map::anchored::Head;
    let unit = Unit::Division(7);
    let mut history = MovementHistory::default();
    next_frame(&mut history, 0, 100, 2.0);
    assert_eq!(history.track(unit, &[at_hour(100, [10.0, 0.0, 0.0])], 100, 0.0), None);
    let track = history
        .track(unit, &[at_hour(148, [58.0, 0.0, 0.0])], 148, 0.0)
        .expect("the two publications' own places");
    assert_eq!(track.places().len(), 2, "the hours gave nothing to add");
    assert_eq!(track.at(Head { hour: 100, fraction: 0.0 })[0], 10.0);
    assert_eq!(track.at(Head { hour: 148, fraction: 0.0 })[0], 58.0);
}

/// A seek starts a chain of its own: the hour before the seek and the hour after it are two
/// histories, and pairing them would carry a counter across whatever lies between. The mover
/// stands exactly where the hour sought puts it until the next publication lands.
#[test]
fn a_seek_backwards_starts_the_places_afresh_instead_of_pairing_two_histories() {
    let unit = Unit::Division(7);
    let mut history = MovementHistory::default();
    next_frame(&mut history, 0, 100, 2.0);
    assert_eq!(history.track(unit, &[at_hour(100, [10.0, 0.0, 0.0])], 100, 0.0), None);
    history.track(unit, &[at_hour(148, [58.0, 0.0, 0.0])], 148, 0.0).expect("two places");
    assert_eq!(history.track(unit, &[at_hour(40, [900.0, 0.0, 0.0])], 40, 0.0), None, "the hour sought");
    let track = history.track(unit, &[at_hour(42, [902.0, 0.0, 0.0])], 42, 0.0).expect("the seek's own two places");
    assert_eq!((track.places().len(), track.places()[0].hour, track.places()[1].hour), (2, 40, 42));
}

/// A figure's root is its counter's: both are the place the recording's own rows put the unit at
/// the display frame's own head, read from one chain of places cut at the publication the frame
/// draws. So a figure advances on every display frame of an interval instead of standing until a
/// publication re-slots it (which left 71 to 75% of frames moving no figure's root).
#[test]
fn a_figure_stands_where_its_counters_track_puts_it_at_the_frames_own_head() {
    use crate::map::anchored::Head;
    use crate::map::free_icons::{IconAnchor, IconFamily};
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = settled_on(&moving, &[unit]);
    next_frame(&mut history, 1, 100, 2.0);
    assert_eq!(history.track(unit, &[at_hour(100, [10.0, 0.0, 0.0])], 100, 0.0), None);
    next_frame(&mut history, 2, 148, 2.0);
    let track = history
        .track(unit, &[at_hour(148, [58.0, 0.0, 0.0])], 148, 0.0)
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
            MovementFrame { head, ..movement_frame(frame as f32 / 10.0, 148, 2.0) },
            SEEN,
        );
        let figure = history.at_head(unit, &moving).expect("the hours place it");
        assert_eq!(figure.at, counter.at(Some(&track), head), "head {hour}");
        // Two publications' places say nothing of the facing: it stays the hour's own.
        assert_eq!(figure.heading, None, "head {hour}");
        walked.push(figure.at[0]);
    }
    // The places are a game hour of travel apart, so each head inside them stands further along
    // than the one before, and a head past the newest holds at it rather than running on.
    let wanted = [10.0, 22.5, 34.0, 58.0];
    assert!(
        walked.iter().zip(wanted).all(|(walked, wanted)| (walked - wanted).abs() < 1e-3),
        "{walked:?}"
    );
}

/// Where a figure stands is a function of the places and the head alone: a frame that has just
/// sought to an hour draws every figure where the frames that walked to it draw it.
#[test]
fn a_cold_seek_places_every_figure_where_the_walk_to_the_same_hour_does() {
    use crate::map::anchored::Head;
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let head = Head { hour: 124, fraction: 0.25 };
    let place = |history: &mut MovementHistory, frame: i32| {
        history.begin_frame(
            MovementFrame { head, ..movement_frame(frame as f32 / 10.0, 148, 2.0) },
            SEEN,
        );
        history.at_head(unit, &moving).expect("the hours place it").at
    };
    let walked = {
        let mut history = settled_on(&moving, &[unit]);
        next_frame(&mut history, 1, 100, 2.0);
        history.track(unit, &[at_hour(100, [10.0, 0.0, 0.0])], 100, 0.0);
        next_frame(&mut history, 2, 148, 2.0);
        history.track(unit, &[at_hour(148, [58.0, 0.0, 0.0])], 148, 0.0);
        for frame in 3..8 {
            place(&mut history, frame);
        }
        place(&mut history, 8)
    };
    let sought = {
        let mut history = MovementHistory::default();
        next_frame(&mut history, 1, 148, 2.0);
        history.track(unit, &[at_hour(100, [10.0, 0.0, 0.0])], 100, 0.0);
        history.track(unit, &[at_hour(148, [58.0, 0.0, 0.0])], 148, 0.0);
        place(&mut history, 2)
    };
    assert_eq!(walked, sought);
}

/// A window dump draws every unit settled at one hour, so no figure is carried off the placement
/// its own rows give: a stopped picture is exact.
#[test]
fn a_steady_frame_leaves_every_figure_on_its_hours_placement() {
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = settled_on(&moving, &[unit]);
    next_frame(&mut history, 1, 100, 2.0);
    history.track(unit, &[at_hour(100, [10.0, 0.0, 0.0])], 100, 0.0);
    next_frame(&mut history, 2, 148, 2.0);
    history
        .track(unit, &[at_hour(148, [58.0, 0.0, 0.0])], 148, 0.0)
        .expect("two publications' places");
    history.begin_frame(
        MovementFrame { steady: true, ..movement_frame(0.3, 148, 2.0) },
        SEEN,
    );
    assert_eq!(history.at_head(unit, &moving), None);
}

/// A display frame draws one publication's layout while the maker lays the next one out, so a
/// figure stands on the track that layout was handed for its unit — not on a newer one, and not on
/// a chain of places the counters on screen never saw. A unit that layout left where its rows put
/// it stands there, which is where its counter stands.
#[test]
fn a_figure_stands_on_the_track_of_the_layout_its_frame_draws() {
    use crate::map::anchored::Head;
    let unit = Unit::Division(7);
    let moving = live_position(1, &[2], 0.0, false);
    let mut history = settled_on(&moving, &[unit]);
    next_frame(&mut history, 1, 100, 2.0);
    assert_eq!(history.track(unit, &[at_hour(100, [10.0, 0.0, 0.0])], 100, 0.0), None);
    next_frame(&mut history, 2, 148, 2.0);
    let older = history.track(unit, &[at_hour(148, [58.0, 0.0, 0.0])], 148, 0.0).expect("two places");
    next_frame(&mut history, 3, 196, 2.0);
    let newer = history.track(unit, &[at_hour(196, [130.0, 0.0, 0.0])], 196, 0.0).expect("three places");
    let head = Head { hour: 160, fraction: 0.0 };
    let at = |history: &mut MovementHistory, frame: i32, drawn: i32| {
        history.begin_frame(
            MovementFrame {
                head,
                overlay_hour: drawn,
                ..movement_frame(frame as f32 / 10.0, 196, 2.0)
            },
            SEEN,
        );
        history.at_head(unit, &moving).expect("the hours place it").at
    };
    assert_eq!(at(&mut history, 4, 148), older.at(head));
    assert_eq!(at(&mut history, 5, 196), newer.at(head));
    assert_ne!(older.at(head), newer.at(head), "the two layouts place it apart");
    // A layout no track was handed for leaves the figure where its own animator draws it.
    let standing = at(&mut history, 6, 60);
    let drawn = history.sample(unit, &moving).expect("its animator draws it");
    assert_eq!(standing, drawn.motion.position());
}
