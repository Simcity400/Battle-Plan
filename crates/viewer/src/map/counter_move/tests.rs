use super::*;
use installed_content::movement::{Neighbour, ProvinceTerrain};

const W: f32 = 5632.0;
/// The height [`Motion::new`]'s flat field samples.
const FLOOR: f32 = crate::map::borders::WATER_HEIGHT;

#[test]
fn l4e9_scale_interpolates_over_the_next_anchor_arc_not_the_current_segment() {
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
fn l4e9_a_nonempty_route_can_initialize_no_animator_samples() {
    let map = line_map();
    let values = flat(100);
    let world = RouteWorld {
        map: &map,
        heights: field(&values),
        width: W,
    };
    // The skipped final hop leaves three controls. The native initializer
    // clears +0x14 even though CUnit's ordinary route remains nonempty.
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
    // docked plate without any ordinary formation (141279110).
    assert_eq!(
        route.stack_kind_from_animator(1, &map, sample.sample_count),
        Some(19)
    );
}

#[test]
fn l4e9_cleared_animator_uses_the_ordinary_route_for_direction() {
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
    // 141631210 fails after initializer clears the route; 140BEDEC0 reads
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
    // controls are no movement at all (`1416317D0`).
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
    let position = Position {
        standing: [4.0, 5.0, 6.0],
        motion: None,
    };
    assert_eq!(position.at(), [4.0, 5.0, 6.0]);
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
    use replay::{
        frame::{DecodedFrame, Metrics, TableView},
        rows::{DivisionMovementRow, DivisionRow, TableId, TableRows},
    };
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
                ..Default::default()
            })
            .collect(),
    ));
    let attacking = |division, path: Vec<u32>| DivisionMovementRow {
        division,
        combat_role: 1,
        combat_count: 1,
        combat_province: 2,
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
    let map = MovementMap::from_parts(&[(1, LAND, land(&[2])), (2, LAND, land(&[1]))]);
    let anchor = |id: u32| Some([id as f32, 0.0, 0.0]);
    let etas = crate::map::division_eta::Etas::default();
    let kinds = |map: Option<&MovementMap>| {
        let routes = DivisionRoutes::read(&frame, &etas, map, &anchor).unwrap();
        [1, 2, 3, 4].map(|id| {
            let route = &routes.by_division[&id];
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
    let mut row = replay::rows::DivisionMovementRow {
        path: vec![2],
        path_total: 1,
        alternate_path: vec![3],
        alternate_total: 1,
        ..Default::default()
    };
    let kind = |row: &replay::rows::DivisionMovementRow,
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
    use replay::rows::TaskForceMapMotionRow as Row;
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
    use replay::rows::RailwayGunCounterInputRow as Row;
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
    use replay::rows::RailwayGunCounterInputRow as Row;
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
