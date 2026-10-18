use super::*;
use dashboard_base::prepared::series::tests::Fixture;
use replay_data::rows::SunkShipEntry;

fn ship(sink: i32, read: i32, token: u32) -> sinkings::ShipEvent {
    sinkings::ShipEvent {
        entry: SunkShipEntry {
            sink_hour: sink,
            class_token: token,
            class_name: format!("mod class {token}").into_bytes(),
            ..Default::default()
        },
        read_hour: read,
        victim: Some("mod holder".into()),
        attacker: None,
        victim_as_of: None,
        attacker_as_of: None,
    }
}
fn canonical(reply: &ShipsSunkReply, row: &ShipsSunkRow) -> Value {
    let mut row = serde_json::to_value(row).unwrap();
    row["classIndex"] =
        serde_json::to_value(&reply.classes[row["classIndex"].as_u64().unwrap() as usize]).unwrap();
    let recorded = row["recordedNameIndex"]
        .as_u64()
        .map(|at| &reply.recorded_names[at as usize]);
    row["recordedNameIndex"] = serde_json::to_value(recorded).unwrap();
    let place = row["placeIndex"]
        .as_u64()
        .map(|at| &reply.places[at as usize]);
    row["placeIndex"] = serde_json::to_value(place).unwrap();
    row
}
#[test]
fn prepared_sinking_windows_match_oracle_after_two_commits_resume_append_and_late_native_dates() {
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let windows = vec![
        HourSpan {
            first_hour: 100,
            last_hour: 110,
        },
        HourSpan {
            first_hour: 111,
            last_hour: 120,
        },
        HourSpan {
            first_hour: 121,
            last_hour: 130,
        },
    ];
    let mut scan = sinkings::Scanned {
        first_hour: 100,
        last_hour: 110,
        first_ship_read: Some(100),
        ..Default::default()
    };
    for index in 0..5000 {
        scan.country_identities
            .insert(format!("mod country {index}"), Vec::new());
    }
    scan.ships = (0..12000)
        .map(|index| ship(99 + index % 11, 110, 700 + (index % 7) as u32))
        .collect();
    let mut prepared = Prepared::default();
    for ordinal in 0..3 {
        scan.last_hour = windows[ordinal].last_hour;
        if ordinal == 1 {
            scan.ship_unread.push(HourSpan {
                first_hour: 111,
                last_hour: 112,
            });
            scan.ships.push(ship(115, 120, 5));
        }
        if ordinal == 2 {
            // Earlier native date, later supplying read, duplicate occurrences preserved.
            scan.ships.extend([ship(105, 130, 3), ship(105, 130, 3)]);
        }
        let mut writer = if ordinal == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        let indices: Vec<_> = (0..scan.ships.len() as u32).collect();
        prepared
            .observe_with(&scan, &indices, &flag, |page| named(page, None, &flag))
            .unwrap();
        prepared
            .publish_with(&mut writer, 1, ordinal, &scan, &windows, &flag, |page| {
                named(page, None, &flag)
            })
            .unwrap();
        series::write_reply(&mut writer, "continuation", 0, &prepared).unwrap();
        writer.commit().unwrap();
        prepared = series::read_reply(&fixture.store, &fixture.key, "continuation", 0)
            .unwrap()
            .unwrap();
        if ordinal >= 1 {
            let names = named(&scan, None, &flag).unwrap();
            let oracle = reply("1", &scan, &names, &flag).unwrap();
            let mut actual = BTreeMap::new();
            for window in 0..=ordinal {
                let page: ShipsSunkWindowReply =
                    series::read_reply(&fixture.store, &fixture.key, "ships-sunk-1", window as u64)
                        .unwrap()
                        .unwrap();
                for (index, row) in page.indices.iter().zip(&page.reply.rows) {
                    assert!(actual.insert(*index, canonical(&page.reply, row)).is_none());
                }
            }
            assert_eq!(
                actual.into_values().collect::<Vec<_>>(),
                oracle
                    .rows
                    .iter()
                    .map(|row| canonical(&oracle, row))
                    .collect::<Vec<_>>()
            );
        }
    }
    let page: ShipsSunkWindowReply =
        series::read_reply(&fixture.store, &fixture.key, "ships-sunk-1", 0)
            .unwrap()
            .unwrap();
    assert_eq!(&page.indices[page.indices.len() - 2..], &[12001, 12002]);
    assert_eq!(page.reply.rows.last().unwrap().known_at, 130);
    assert!(serde_json::to_vec(&prepared).unwrap().len() < 4096);
    let mut events = series::Reader::new(
        &fixture.store,
        &fixture.key,
        "sinking-events-1".into(),
        &flag,
    )
    .unwrap();
    assert_eq!(events.event_count_through(0, 130).unwrap(), 12003);
}
