//! Consumer regressions through the committed prepared reader, not a copied decoder.
use super::*;
use replay_data::rows::{self, TableId, TableRows};
use std::collections::BTreeMap;

fn consumer_replay() -> Replay {
    let mut records = crate::playback::tests::records(1, &(100..=105).collect::<Vec<_>>());
    for record in &mut records {
        let Record::Frame(frame) = record else {
            continue;
        };
        let rows = [
            TableRows::CountryLinks(vec![rows::CountryLinkRow {
                id: 11,
                canonical_slot: 7,
            }]),
            TableRows::Fleets(vec![rows::FleetRow {
                id: 1,
                owner: 11,
                name: Recorded::Value(Some(
                    format!("Synthetic empty fleet {}", frame.hour).into_bytes(),
                )),
                ..Default::default()
            }]),
            TableRows::CountryGameScoreInputs(vec![rows::CountryGameScoreInputRow {
                country: 11,
                army_count: Some(0),
                navy_count: Some(0),
                land_count: Some(0),
                air_state: rows::CountryGameScoreInputRow::AIR_ABSENT,
                fleets: vec![rows::GameScoreFleetCount {
                    fleet: 1,
                    task_forces: Some(0),
                    ..Default::default()
                }],
                ..Default::default()
            }]),
            TableRows::CountryWars(Vec::new()),
            TableRows::WarOwnedRelations(if frame.hour % 2 == 0 {
                vec![rows::WarOwnedRelationRow {
                    owner: 11,
                    retired: 1,
                    ..Default::default()
                }]
            } else {
                Vec::new()
            }),
        ];
        for rows in rows {
            let table = rows.table();
            frame.tables[table.index()] = TableUpdate::Snapshot(rows);
        }
    }
    Replay::from_records(b"campaign".to_vec(), &[records]).unwrap()
}

fn consumers(frame: &DecodedFrame) -> (Vec<(u64, String)>, bool) {
    (
        viewer_counters::map::naval_selection::fleet_choices(frame, 7).unwrap(),
        viewer_foundation::map::division_panel::at_war(frame, &BTreeMap::from([(11, 7)]), 11)
            .unwrap(),
    )
}

#[test]
fn production_selection_keeps_fleet_menu_and_attrition_exact_across_checkpoints_and_steps() {
    let replay = Arc::new(consumer_replay());
    let selected = dashboard_base::prepared_map::selected_tables();
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 2, selected);
    let mut map = fixture.reader(progress, Arc::clone(&replay), selected);
    let mut held = None;
    for hour in 100..=105 {
        let actual = if hour == 100 {
            map.seek(1, hour).unwrap()
        } else {
            map.advance(1, hour, |_| panic!("exact step")).unwrap()
        };
        let Some(Current::Frame(actual)) = actual else {
            panic!("prepared frame")
        };
        let SeekResult::Frame(expected) = replay.seek_selected(1, hour, None).unwrap() else {
            panic!("recorded frame")
        };
        assert_eq!(
            consumers(&actual),
            consumers(&expected),
            "consumer parity at {hour}"
        );
        assert_eq!(
            consumers(&actual),
            (
                vec![(1, format!("Synthetic empty fleet {hour}"))],
                hour % 2 == 0
            )
        );
        if hour == 100 {
            assert_eq!(
                map.source_reads, 0,
                "the consumer reads a prepared checkpoint"
            );
            held = Some(actual);
        }
    }
    for hour in [104, 101, 100] {
        let Some(Current::Frame(actual)) = map.seek(1, hour).unwrap() else {
            panic!("prepared seek")
        };
        let SeekResult::Frame(expected) = replay.seek_selected(1, hour, None).unwrap() else {
            panic!("recorded seek")
        };
        assert_eq!(consumers(&actual), consumers(&expected));
    }
    assert_eq!(
        consumers(&held.unwrap()),
        (vec![(1, "Synthetic empty fleet 100".into())], true),
        "later playback leaves the displayed earlier hour immutable"
    );
}

#[test]
fn omitting_consumer_tables_reproduces_prepared_only_refusals() {
    let replay = Arc::new(consumer_replay());
    let selected: Vec<_> = dashboard_base::prepared_map::selected_tables()
        .iter()
        .copied()
        .filter(|table| {
            !matches!(
                table,
                TableId::CountryGameScoreInputs | TableId::WarOwnedRelations
            )
        })
        .collect();
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 720, &selected);
    let mut map = fixture.reader(progress, Arc::clone(&replay), &selected);
    let Some(Current::Frame(actual)) = map.seek(1, 100).unwrap() else {
        panic!("prepared frame")
    };
    assert_eq!(map.source_reads, 0);
    let SeekResult::Frame(expected) = replay.seek_selected(1, 100, None).unwrap() else {
        panic!("recorded frame")
    };
    assert_eq!(consumers(&expected).0.len(), 1);
    assert!(consumers(&expected).1);
    let fleet = viewer_counters::map::naval_selection::fleet_choices(&actual, 7)
        .unwrap_err()
        .to_string();
    let attrition =
        viewer_foundation::map::division_panel::at_war(&actual, &BTreeMap::from([(11, 7)]), 11)
            .unwrap_err();
    for (table, error) in [
        ("CountryGameScoreInputs", fleet),
        ("WarOwnedRelations", attrition),
    ] {
        assert!(
            error.contains(table) && error.contains("Table not selected for reconstruction"),
            "{error}"
        );
    }
}
