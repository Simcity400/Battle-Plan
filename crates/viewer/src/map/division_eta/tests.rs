use super::*;
use installed_content::movement::{Neighbour, ProvinceTerrain};
use replay::{
    frame::{Metrics, TableView},
    rows::{
        DivisionEtaInputsRow, DivisionMovementRow, DivisionRow, ProvinceRow, StateRow, TableId,
        TableRows, WeatherProvinceRow, WeatherRegionRow,
    },
};

const DIVISION: u64 = 7;
/// A normal-speed fixture whose expected pair is arithmetic.
const SPEED: i64 = 700_000;
const LAND: ProvinceTerrain = ProvinceTerrain { flags: 1, index: 1 };

fn content(map: MovementMap) -> MovementContent {
    MovementContent {
        map,
        defines: Defines {
            outer_scale: 100_000,
            ..Default::default()
        },
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

fn frame() -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour: 60_832_632,
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed for the ETA source".into(),
        })
        .map(Into::into),
        metrics: Metrics::default(),
    }
}

fn put(frame: &mut DecodedFrame, id: TableId, rows: TableRows) {
    *frame.tables[id.index()] = TableView::Complete(rows);
}

fn inputs(division: u64) -> DivisionEtaInputsRow {
    DivisionEtaInputsRow {
        division,
        sample_hour: frame().hour,
        speed_modifier: 100_000,
        speed_base: SPEED,
        current_speed: SPEED,
        terrain_speed: vec![replay::rows::TerrainSpeed {
            terrain: 1,
            modifier: 0,
        }],
        ..Default::default()
    }
}

/// A busy hour: `provinces` land provinces with no authored neighbours, so
/// every hop takes the native 100000 fallback, and `divisions` movers each
/// on a `legs`-hop route with complete normal-speed operands.
fn busy_hour(provinces: u32, divisions: u64, legs: usize) -> (DecodedFrame, MovementMap) {
    let mut frame = derivable();
    let hop = |division: u64, leg: usize| (division as u32 * 7 + leg as u32) % provinces + 1;
    let ids = 1..=provinces;
    put(
        &mut frame,
        TableId::WeatherProvinces,
        TableRows::WeatherProvinces(
            ids.clone()
                .map(|id| WeatherProvinceRow {
                    id,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::Provinces,
        TableRows::Provinces(
            ids.clone()
                .map(|id| ProvinceRow {
                    id,
                    state: id % 900 + 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::States,
        TableRows::States(
            (1..=901)
                .map(|id| StateRow {
                    id,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::Divisions,
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
        TableId::DivisionMovement,
        TableRows::DivisionMovement(
            (0..divisions)
                .map(|id| DivisionMovementRow {
                    division: id,
                    path: (1..=legs).map(|leg| hop(id, leg)).collect(),
                    path_total: legs as u32,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::DivisionEtaInputs,
        TableRows::DivisionEtaInputs((0..divisions).map(inputs).collect()),
    );
    let map = MovementMap::from_parts(&ids.map(|id| (id, LAND, Vec::new())).collect::<Vec<_>>());
    (frame, map)
}

/// One division at province 1 on the two-leg route 2, 3, all land, no snow,
/// no fort and no infrastructure, taking the normal-speed branch.
fn derivable() -> DecodedFrame {
    let mut frame = frame();
    put(
        &mut frame,
        TableId::WeatherRegions,
        TableRows::WeatherRegions(vec![WeatherRegionRow {
            max_snow: 20_000,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableId::WeatherProvinces,
        TableRows::WeatherProvinces(
            (1..=3)
                .map(|id| WeatherProvinceRow {
                    id,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::Provinces,
        TableRows::Provinces(
            (1..=3)
                .map(|id| ProvinceRow {
                    id,
                    state: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::States,
        TableRows::States(vec![StateRow {
            id: 1,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableId::Divisions,
        TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            province: 1,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableId::DivisionMovement,
        TableRows::DivisionMovement(vec![DivisionMovementRow {
            division: DIVISION,
            path: vec![2, 3],
            path_total: 2,
            numerator: 175_000,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableId::DivisionEtaInputs,
        TableRows::DivisionEtaInputs(vec![inputs(DIVISION)]),
    );
    frame
}

#[test]
fn an_hour_without_inputs_has_no_eta_and_no_refusal() {
    let map = content(two_leg_map());
    let etas = Etas::read(&frame(), Some(&map)).unwrap();
    assert_eq!(etas.report(), Report::default());
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

#[test]
fn latest_operand_rows_survive_seek_but_future_and_expired_rows_are_refused() {
    let map = content(two_leg_map());
    for age in [-1, 0, 1, 4, 5] {
        let mut frame = derivable();
        let mut row = inputs(DIVISION);
        row.sample_hour = frame.hour - age;
        put(
            &mut frame,
            TableId::DivisionEtaInputs,
            TableRows::DivisionEtaInputs(vec![row]),
        );
        let etas = Etas::read(&frame, Some(&map)).unwrap();
        assert_eq!(etas.get(DIVISION).is_some(), (0..5).contains(&age));
        assert_eq!(etas.report().refused, usize::from(!(0..5).contains(&age)));
    }
}

#[test]
fn an_hour_derives_progress_and_both_remaining_durations() {
    // 350000 and 700000 distance over speed 700000: half an hour and one
    // hour, less the 175000 numerator already moved (a quarter hour).
    let map = content(two_leg_map());
    let etas = Etas::read(&derivable(), Some(&map)).unwrap();
    assert_eq!(etas.get(DIVISION), Some((25_000, 125_000)));
    // Progress divides the numerator by the first edge's content distance.
    assert_eq!(etas.progress(DIVISION), Ok(50_000));
    assert_eq!(
        etas.report(),
        Report {
            derived: 1,
            ..Report::default()
        }
    );
}

#[test]
fn operand_indices_bound_storage_by_rows_and_preserve_sparse_keys() {
    let dense = indexed(&[(0, 3), (2, 7)], |&row| row);
    assert!(matches!(&dense, OperandIndex::Dense(values) if values.len() == 3));
    assert_eq!(dense.get(0), Some(3));
    assert_eq!(dense.get(1), None);
    let sparse = indexed(&[(1, 3), (u32::MAX, 7)], |&row| row);
    assert!(matches!(&sparse, OperandIndex::Sparse(values) if values.len() == 2));
    assert_eq!(sparse.get(1), Some(3));
    assert_eq!(sparse.get(u32::MAX), Some(7));
    assert_eq!(sparse.get(u32::MAX - 1), None);
}

#[test]
fn permanent_snow_and_infrastructure_enter_the_normal_edge() {
    let mut frame = derivable();
    put(
        &mut frame,
        TableId::States,
        TableRows::States(vec![StateRow {
            id: 1,
            movement_infrastructure: 50_000,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableId::DivisionMovement,
        TableRows::DivisionMovement(vec![DivisionMovementRow {
            division: DIVISION,
            path: vec![2],
            path_total: 1,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableId::DivisionEtaInputs,
        TableRows::DivisionEtaInputs(vec![DivisionEtaInputsRow {
            speed_base: 400_000,
            current_speed: 400_000,
            snow_penalty: -100_000,
            ..inputs(DIVISION)
        }]),
    );
    // (1 - 1 snow + 0.5 infrastructure) * 400000 = 200000 speed over the
    // 350000 edge: the destination's permanent snow alone makes it snowy.
    put(
        &mut frame,
        TableId::WeatherProvinces,
        TableRows::WeatherProvinces(
            (1..=3)
                .map(|id| WeatherProvinceRow {
                    id,
                    permanent_snow: i64::from(id == 2),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let map = content(two_leg_map());
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(etas.get(DIVISION), Some((175_000, 175_000)));
    assert_eq!(etas.report().refused, 0);
}

#[test]
fn a_missing_operand_refuses_the_row_instead_of_deriving_without_it() {
    let map = content(two_leg_map());
    // The snow census and its ceiling are part of the edge speed, so
    // losing either refuses the ETA; first-edge progress needs no weather
    // and stands.
    for table in [TableId::WeatherProvinces, TableId::WeatherRegions] {
        let mut frame = derivable();
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "dropped for the test".into(),
        };
        let etas = Etas::read(&frame, Some(&map)).unwrap();
        assert_eq!(etas.get(DIVISION), None);
        assert_eq!(etas.progress(DIVISION), Ok(50_000));
        assert_eq!((etas.report().derived, etas.report().refused), (0, 1));
    }

    // Without the ETA row there is neither progress nor a claimed zero.
    let mut frame = derivable();
    *frame.tables[TableId::DivisionEtaInputs.index()] = TableView::Unavailable {
        reason: "dropped for the test".into(),
    };
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(
        etas.progress(DIVISION),
        Err("it has no DivisionEtaInputs row valid this hour")
    );
    assert_eq!(etas.report().progress_refused, 1);

    // A province absent from the content map cannot silently become the
    // null terrain or an ordinary land province.
    let partial = content(MovementMap::from_parts(&[(1, LAND, Vec::new())]));
    let etas = Etas::read(&derivable(), Some(&partial)).unwrap();
    assert_eq!(etas.get(DIVISION), None);
    assert_eq!(
        etas.progress(DIVISION),
        Err("eta_derive::movement_progress refuses its recorded route with the content map")
    );

    // Content that failed to load refuses every moving division, but a
    // division without a route still stands at progress zero.
    let mut frame = derivable();
    put(
        &mut frame,
        TableId::DivisionMovement,
        TableRows::DivisionMovement(vec![
            DivisionMovementRow {
                division: DIVISION,
                path: vec![2],
                path_total: 1,
                ..Default::default()
            },
            DivisionMovementRow {
                division: DIVISION + 1,
                ..Default::default()
            },
        ]),
    );
    let etas = Etas::read(&frame, None).unwrap();
    assert_eq!(etas.get(DIVISION), None);
    assert_eq!(
        etas.progress(DIVISION),
        Err("the content's movement map (terrain and movement defines) failed to load")
    );
    assert_eq!(etas.progress(DIVISION + 1), Ok(0));
    assert_eq!(
        (etas.report().refused, etas.report().progress_refused),
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
fn a_busy_hour_derives_every_route() {
    let (frame, map) = busy_hour(13_000, 1_000, 30);
    let map = content(map);
    let etas = Etas::read(&frame, Some(&map)).unwrap();
    assert_eq!(etas.report().derived, 1_000);
    assert_eq!(etas.report().refused, 0);
    assert_eq!(etas.report().progress_refused, 0);
    // Thirty known non-neighbour hops at the native 100000 distance, each
    // truncated to 14285 at speed 700000.
    assert_eq!(etas.get(0).map(|pair| pair.1), Some(30 * 14_285));
}
