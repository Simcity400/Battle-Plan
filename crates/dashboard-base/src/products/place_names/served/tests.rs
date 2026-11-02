use super::super::{NAME_TABLES, PlaceNamesParams};
use super::*;
use replay_data::rows::{
    ProvinceNameRow, ProvinceRow, Recorded, ReservedCountryNamingRow, StateRow, TableRows,
};
use replay_frame::frame::TableView;

struct Requests {
    branches: Vec<Branch>,
    recording: Replay,
    params: Vec<Value>,
}

impl Ask for Requests {
    fn branches(&self) -> &[Branch] {
        &self.branches
    }

    fn recording(&self) -> io::Result<&Replay> {
        Ok(&self.recording)
    }

    fn request_allowance(&mut self, branch: u64, _: usize) -> io::Result<()> {
        assert!(self.branches.iter().any(|held| held.id == branch));
        Ok(())
    }

    fn ask(&mut self, product: &str, params: Value) -> io::Result<Option<Value>> {
        assert_eq!(product, "place-names");
        let parsed: PlaceNamesParams = serde_json::from_value(params.clone()).unwrap();
        assert!(parsed.places.len() <= served::BATCH);
        assert!(parsed.states.len() <= served::KEYS);
        self.params.push(params);
        Ok(None)
    }

    fn countries(&mut self, _: u64) -> io::Result<Vec<String>> {
        panic!("place discovery reads the recording")
    }

    fn holders(&mut self, _: u64) -> io::Result<Vec<served::Holder>> {
        panic!("place discovery reads the recording")
    }
}

fn frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut frame = crate::test_fixtures::source(branch, hour, Some(1));
    for table in NAME_TABLES {
        if table != TableId::Campaign {
            *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
    }
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"mod_reserved".to_vec(),
                original_tag_reference: Recorded::Value(Some(1)),
                original_tag: Recorded::Value(Some(b"mod_original".to_vec())),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        ProvinceRow {
            id: 17,
            state: 64,
            ..Default::default()
        },
        ProvinceRow {
            id: 90_001,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 64,
            custom_name: b"mod_state".to_vec(),
            ..Default::default()
        }]));
    frame
}

fn frames(branch: u64, mut change: impl FnMut(&mut DecodedFrame)) -> Vec<DecodedFrame> {
    (10..=1000)
        .map(|hour| {
            let mut frame = frame(branch, hour);
            change(&mut frame);
            frame
        })
        .collect()
}

fn requests(frames: Vec<Vec<DecodedFrame>>) -> Requests {
    let branches: Vec<Branch> = frames
        .iter()
        .map(|frames| Branch {
            id: frames[0].branch,
            first_hour: frames[0].hour,
            last_hour: frames.last().unwrap().hour,
            windows: vec![(frames[0].hour, frames.last().unwrap().hour)],
            gaps: Vec::new(),
            segments: Vec::new(),
        })
        .collect();
    assert!(branches.iter().all(|branch| {
        [36, 37, 38]
            .iter()
            .all(|hour| !branch.hours().contains(hour))
    }));
    let records: Vec<_> = frames
        .into_iter()
        .map(|frames| crate::test_fixtures::records(frames[0].branch, frames))
        .collect();
    let recording = Replay::from_records(crate::test_fixtures::CAMPAIGN.into(), &records).unwrap();
    let mut requests = Requests {
        branches,
        recording,
        params: Vec::new(),
    };
    (PLAN.queries)(&mut requests).unwrap();
    requests
}

fn assert_provinces(requests: &Requests, branch: &str, ids: &[u32], hours: &[i32]) {
    for &hour in hours {
        for land in [None, Some(false), Some(true)] {
            assert!(
                requests.params.iter().any(|params| {
                    let places = params["places"].as_array().unwrap();
                    params["branchId"] == branch
                        && params.get("land").and_then(Value::as_bool) == land
                        && places.iter().all(|place| place["hour"] == hour)
                        && ids
                            .iter()
                            .all(|&id| places.iter().any(|place| place["provinceId"] == id))
                }),
                "branch {branch}, hour {hour}, provinces {ids:?}, land {land:?}"
            );
        }
    }
}

fn assert_state(requests: &Requests, state: u32, hours: &[i32]) {
    for &hour in hours {
        assert!(
            requests.params.iter().any(|params| {
                params["places"] == json!([])
                    && params["states"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|row| row["stateId"] == state && row["hour"] == hour)
            }),
            "state {state} at {hour}"
        );
    }
}

#[test]
fn renamed_provinces_and_states_select_the_same_subjects_on_both_sides() {
    let requests = requests(vec![frames(7, |frame| {
        if frame.hour == 37 {
            *frame.tables[TableId::ProvinceNames.index()] =
                TableView::Complete(TableRows::ProvinceNames(vec![ProvinceNameRow {
                    province: 90_001,
                    name: b"mod_renamed".to_vec(),
                }]));
            let TableView::Complete(TableRows::States(rows)) =
                &mut *frame.tables[TableId::States.index()]
            else {
                unreachable!()
            };
            rows[0].custom_name = b"mod_renamed_state".to_vec();
        }
    })]);
    assert_provinces(&requests, "7", &[17, 90_001], &[36, 37, 38]);
    assert_state(&requests, 64, &[36, 37, 38]);
}

#[test]
fn disappearing_states_and_provinces_are_queried_on_both_sides() {
    let requests = requests(vec![frames(7, |frame| {
        if frame.hour == 37 {
            *frame.tables[TableId::States.index()] =
                TableView::Complete(TableRows::States(Vec::new()));
            let TableView::Complete(TableRows::Provinces(rows)) =
                &mut *frame.tables[TableId::Provinces.index()]
            else {
                unreachable!()
            };
            rows.retain(|row| row.id != 90_001);
        }
    })]);
    assert_provinces(&requests, "7", &[17, 90_001], &[36, 37, 38]);
    assert_state(&requests, 64, &[36, 37, 38]);
}

#[test]
fn empty_name_tables_losing_availability_and_recovering_select_their_subjects() {
    for table in [TableId::ProvinceNames, TableId::States] {
        let requests = requests(vec![frames(7, |frame| {
            *frame.tables[table.index()] = match frame.hour {
                37 => TableView::Unavailable {
                    reason: "name inputs unread".into(),
                },
                _ => TableView::Complete(TableRows::empty(table)),
            };
        })]);
        assert_provinces(&requests, "7", &[17], &[36, 37, 38]);
        if table == TableId::ProvinceNames {
            assert_provinces(&requests, "7", &[90_001], &[36, 37, 38]);
        } else {
            assert_state(&requests, 64, &[36, 37, 38]);
        }
    }
}

#[test]
fn name_table_missing_reasons_are_changes_even_without_any_rows() {
    let requests = requests(vec![frames(7, |frame| {
        *frame.tables[TableId::ProvinceNames.index()] = TableView::Unavailable {
            reason: if frame.hour == 37 {
                "second failure"
            } else {
                "first failure"
            }
            .into(),
        };
    })]);
    assert_provinces(&requests, "7", &[17, 90_001], &[36, 37, 38]);
}

#[test]
fn reserved_naming_changes_select_stateless_provinces_without_any_states() {
    let requests = requests(vec![frames(7, |frame| {
        *frame.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                id: 90_001,
                ..Default::default()
            }]));
        *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(Vec::new()));
        if frame.hour == 37 {
            let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
                &mut *frame.tables[TableId::ReservedCountryNaming.index()]
            else {
                unreachable!()
            };
            rows[0].original_tag = Recorded::Value(Some(b"mod_changed".to_vec()));
        }
    })]);
    assert_provinces(&requests, "7", &[90_001], &[36, 37, 38]);
    assert!(
        requests
            .params
            .iter()
            .all(|params| params["states"] == json!([]))
    );
}

#[test]
fn sampling_reserves_both_province_kinds_even_when_spread_would_skip_one() {
    for sea_at_eight in [false, true] {
        let requests = requests(vec![frames(7, |frame| {
            *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
                (1..=9)
                    .map(|id| ProvinceRow {
                        id,
                        state: if (id == 8) == sea_at_eight { 0 } else { 64 },
                        ..Default::default()
                    })
                    .collect(),
            ));
        })]);
        assert_provinces(&requests, "7", &[1, 8], &[10, 1000]);
        for params in &requests.params {
            let places = params["places"].as_array().unwrap();
            if !places.is_empty()
                && places
                    .iter()
                    .all(|place| place["hour"] == places[0]["hour"])
            {
                assert!(places.len() <= served::KEYS);
                assert!(places.iter().any(|place| place["provinceId"] == 8));
                assert!(places.iter().any(|place| place["provinceId"] != 8));
            }
        }
    }
}

#[test]
fn discovery_bounds_changes_keeps_branches_separate_and_batches_multiple_hours() {
    let requests = requests(vec![
        frames(1, |frame| {
            *frame.tables[TableId::ProvinceNames.index()] =
                TableView::Complete(TableRows::ProvinceNames(vec![ProvinceNameRow {
                    province: 90_001,
                    name: frame.hour.to_le_bytes().to_vec(),
                }]));
            let TableView::Complete(TableRows::States(rows)) =
                &mut *frame.tables[TableId::States.index()]
            else {
                unreachable!()
            };
            rows[0].custom_name = frame.hour.to_le_bytes().to_vec();
        }),
        frames(2, |frame| {
            *frame.tables[TableId::Provinces.index()] =
                TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                    id: 1_000_001,
                    ..Default::default()
                }]));
            *frame.tables[TableId::States.index()] =
                TableView::Complete(TableRows::States(Vec::new()));
        }),
    ]);
    assert!(
        requests
            .params
            .iter()
            .filter(|params| !params["states"].as_array().unwrap().is_empty())
            .count()
            <= served::HOURS * 2
    );
    for params in &requests.params {
        for place in params["places"].as_array().unwrap() {
            assert!(if params["branchId"] == "1" {
                [json!(17), json!(90_001)].contains(&place["provinceId"])
            } else {
                place["provinceId"] == 1_000_001
            });
        }
    }
    assert_provinces(&requests, "1", &[17, 90_001], &[10, 1000]);
    assert_provinces(&requests, "2", &[1_000_001], &[10, 1000]);
    for land in [None, Some(false), Some(true)] {
        assert!(requests.params.iter().any(|params| {
            let places = params["places"].as_array().unwrap();
            params.get("land").and_then(Value::as_bool) == land
                && places
                    .first()
                    .zip(places.last())
                    .is_some_and(|(first, last)| first["hour"] != last["hour"])
        }));
    }
}
