use super::*;
use installed_content::movement::{Neighbour, ProvinceTerrain};
use replay::{
    frame::{Metrics, TableView},
    rows::{DivisionMovementRow, DivisionRow, Recorded, TableRows},
};

const DIVISION: u64 = 7;
/// A base speed whose expected pair is arithmetic at movement scale 1.0.
const SPEED: i64 = 700_000;
const LAND: ProvinceTerrain = ProvinceTerrain { flags: 1, index: 1 };
const UNRECORDED: &str = "DivisionMovement.base_speed is not recorded in this format";

fn content(map: MovementMap) -> MovementContent {
    MovementContent {
        map,
        defines: Defines {
            outer_scale: 100_000,
            ..Default::default()
        },
        rail: Ok(RailMovementDefines {
            speed: [0; 4],
            maximum_level: 0,
        }),
        task_force_terrain: Err("unrelated task-force terrain is unavailable".into()),
    }
}

/// Land provinces 1, 2 and 3 with the route edges 1-2 (350000) and 2-3 (700000).
fn two_leg_map() -> MovementMap {
    let edge = |to, distance| Neighbour {
        to,
        distance: Some(distance),
        ..Default::default()
    };
    MovementMap::from_parts(&[
        (1, LAND, vec![edge(2, 350_000)]),
        (2, LAND, vec![edge(1, 350_000), edge(3, 700_000)]),
        (3, LAND, vec![edge(2, 700_000)]),
    ])
}

/// Every table unavailable: an estimate reads no weather, state, province or
/// speed-graph row.
fn frame() -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour: 60_832_632,
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed for the estimate".into(),
        })
        .map(Into::into),
        metrics: Metrics::default(),
    }
}

fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let id = rows.table();
    *frame.tables[id.index()] = TableView::Complete(rows);
}

fn moving(base_speed: Recorded<Option<i64>>) -> DivisionMovementRow {
    DivisionMovementRow {
        division: DIVISION,
        path: vec![2, 3],
        path_total: 2,
        numerator: 175_000,
        base_speed,
        ..Default::default()
    }
}

/// One division at province 1 on the two-leg route 2, 3 with `base_speed`.
fn hour(base_speed: Recorded<Option<i64>>) -> DecodedFrame {
    let mut frame = frame();
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            province: 1,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionMovement(vec![moving(base_speed)]),
    );
    frame
}

/// A busy hour: `provinces` land provinces with no authored neighbours, so
/// every hop takes the native 100000 fallback, and `divisions` movers each
/// on a `legs`-hop route at the same base speed.
fn busy_hour(provinces: u32, divisions: u64, legs: usize) -> (DecodedFrame, MovementMap) {
    let mut frame = frame();
    let hop = |division: u64, leg: usize| (division as u32 * 7 + leg as u32) % provinces + 1;
    put(
        &mut frame,
        TableRows::Divisions(
            (0..divisions)
                .map(|id| DivisionRow {
                    id,
                    province: hop(id, 0),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::DivisionMovement(
            (0..divisions)
                .map(|id| DivisionMovementRow {
                    division: id,
                    path: (1..=legs).map(|leg| hop(id, leg)).collect(),
                    path_total: legs as u32,
                    base_speed: Some(SPEED).into(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let map = MovementMap::from_parts(
        &(1..=provinces)
            .map(|id| (id, LAND, Vec::new()))
            .collect::<Vec<_>>(),
    );
    (frame, map)
}

#[test]
fn an_hour_without_movement_has_no_estimate_and_no_refusal() {
    let map = content(two_leg_map());
    let etas = Etas::read(&frame(), Some(&map)).unwrap();
    assert_eq!(etas.report(), &Report::default());
    assert_eq!(etas.get(DIVISION), None);
    assert_eq!(etas.report().line(), None);
    let skipped = Report {
        route_hops_skipped: 2,
        ..Report::default()
    };
    assert!(
        skipped
            .line()
            .is_some_and(|line| line.ends_with("0 routes unplaced, 2 route hops skipped"))
    );
}

/// The estimate is the remaining route distance over the estimated speed,
/// read from the route, the location, content geometry and the base speed
/// alone: this hour holds no weather, state, province or speed-graph rows.
#[test]
fn an_estimate_is_the_remaining_route_distance_over_the_estimated_speed() {
    let map = content(two_leg_map());
    for (base, speed) in [(SPEED, SPEED), (350_000, 350_000), (1, 100_000)] {
        assert_eq!(eta_derive::estimated_speed(base, &map.defines), speed);
        let etas = Etas::read(&hour(Some(base).into()), Some(&map)).unwrap();
        // Legs 350000 and 700000, less the 175000 numerator already moved.
        let leg = |distance: i64| distance * 100_000 / speed;
        let progress = leg(175_000);
        let next = leg(350_000) - progress;
        let destination = leg(350_000) + leg(700_000) - progress;
        assert_eq!(etas.get(DIVISION), Some((next, destination)), "{base}");
        assert_eq!(etas.estimate(DIVISION), Ok((next, destination)));
        // Progress divides the numerator by the first edge's content distance.
        assert_eq!(etas.progress(DIVISION), Ok(50_000));
        assert_eq!(
            etas.report(),
            &Report {
                estimated: 1,
                ..Report::default()
            }
        );
    }
    let etas = Etas::read(&hour(Some(SPEED).into()), Some(&map)).unwrap();
    assert_eq!(etas.get(DIVISION), Some((25_000, 125_000)));
    assert_eq!(super::arrival(&etas, DIVISION, true), Ok(125_000));
    assert_eq!(super::arrival(&etas, DIVISION, false), Ok(25_000));
}

/// A recording older than the base speed keeps progress and refuses only
/// the estimate, by the replay estimate's own name.
#[test]
fn older_recordings_refuse_estimates_by_name_but_keep_progress() {
    let map = content(two_leg_map());
    for (base_speed, reason) in [
        (Recorded::NotRecorded, UNRECORDED),
        (
            Recorded::Value(None),
            "the division's base speed was refused",
        ),
    ] {
        let etas = Etas::read(&hour(base_speed), Some(&map)).unwrap();
        assert_eq!(etas.get(DIVISION), None);
        assert_eq!(etas.estimate(DIVISION), Err(reason));
        assert_eq!(super::arrival(&etas, DIVISION, true), Err(reason.into()));
        assert_eq!(etas.progress(DIVISION), Ok(50_000));
        let report = etas.report();
        assert_eq!((report.estimated, report.refused_total()), (0, 1));
        assert_eq!(report.refused.get(reason).copied(), Some(1));
        assert_eq!(report.progress_refused, 0);
        assert!(
            report
                .line()
                .is_some_and(|line| line.contains(&format!("1 estimates refused (1 {reason})")))
        );
    }
}

#[test]
fn missing_location_content_or_geometry_refuse_by_name() {
    // No census row: neither progress nor estimate.
    let map = content(two_leg_map());
    let mut frame = hour(Some(SPEED).into());
    put(&mut frame, TableRows::Divisions(vec![]));
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(etas.estimate(DIVISION), Err(NO_LOCATION));
    assert_eq!(etas.progress(DIVISION), Err(NO_LOCATION));
    assert_eq!(etas.report().progress_refused, 1);

    // A province absent from the content map is missing geometry, never the
    // null terrain or an ordinary land province.
    let partial = content(MovementMap::from_parts(&[(1, LAND, Vec::new())]));
    let etas = Etas::read(&hour(Some(SPEED).into()), Some(&partial)).unwrap();
    assert_eq!(
        etas.estimate(DIVISION),
        Err("route edge 1->2 has no content geometry")
    );
    assert_eq!(
        etas.progress(DIVISION),
        Err("eta_derive::division_progress refuses its recorded route with the content map")
    );

    // Content that failed to load refuses every moving division, but a
    // division without a route still stands at progress zero.
    let mut frame = hour(Some(SPEED).into());
    put(
        &mut frame,
        TableRows::DivisionMovement(vec![
            moving(Some(SPEED).into()),
            DivisionMovementRow {
                division: DIVISION + 1,
                ..Default::default()
            },
        ]),
    );
    let etas = Etas::read(&frame, None).unwrap();
    assert_eq!(etas.estimate(DIVISION), Err(NO_CONTENT));
    assert_eq!(etas.progress(DIVISION), Err(NO_CONTENT));
    assert_eq!(etas.progress(DIVISION + 1), Ok(0));
    assert_eq!(etas.estimate(DIVISION + 1), Err(NO_ROUTE));
    assert_eq!(
        (
            etas.report().refused_total(),
            etas.report().progress_refused
        ),
        (1, 1)
    );
}

#[test]
fn the_cache_holds_the_hours_in_flight_and_evicts_the_oldest() {
    let cache = EtaCache::default();
    let built = std::cell::Cell::new(0);
    let fill = |hour: replay::Hour| {
        cache
            .get_or_insert_with((1, hour), || {
                built.set(built.get() + 1);
                Ok::<_, io::Error>(Etas::default())
            })
            .expect("the build cannot fail")
    };
    // A worker builds the hours ahead; the window then arrives at them.
    for hour in 1..=3 {
        fill(hour);
    }
    assert_eq!(built.get(), 3);
    fill(1);
    fill(2);
    assert_eq!(built.get(), 3, "the look-ahead must still be cached");
    // The ninth distinct key evicts the first.
    for hour in 4..=9 {
        fill(hour);
    }
    assert_eq!(built.get(), 9);
    fill(9);
    assert_eq!(built.get(), 9);
    fill(1);
    assert_eq!(built.get(), 10, "hour 1 should have been evicted");
}

/// Exercise a large synthetic hour without tying correctness to the speed
/// or load of the machine running the test.
#[test]
fn a_busy_hour_estimates_every_route() {
    let (frame, map) = busy_hour(13_000, 1_000, 30);
    let map = content(map);
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(etas.report().estimated, 1_000);
    assert_eq!(etas.report().refused_total(), 0);
    assert_eq!(etas.report().progress_refused, 0);
    // Thirty known non-neighbour hops at the native 100000 distance, each
    // truncated to 14285 at speed 700000.
    assert_eq!(etas.get(0).map(|pair| pair.1), Some(30 * 14_285));
}
