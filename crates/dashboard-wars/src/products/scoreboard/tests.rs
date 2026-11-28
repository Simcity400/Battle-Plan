use super::*;

#[test]
fn every_column_but_the_combined_casualties_reads_a_declared_value_of_a_country_and_casualties_name_their_partial_mark() {
    let columns = registry(|_| None);
    assert_eq!(columns.len(), DEFS.len());
    for column in &columns {
        if column.key == ScoreboardColumnKey::Casualties {
            assert!(!column.available && column.family.is_none() && column.reason.is_some());
            continue;
        }
        assert!(column.available, "{:?}: {:?}", column.key, column.reason);
        let family = column.family.as_ref().unwrap();
        let (_, measure) = declared((family.kind.as_str(), family.measure.as_str())).unwrap();
        assert_eq!(column.decimals, measure.decimals);
    }
    for key in [ScoreboardColumnKey::CasualtiesDealt, ScoreboardColumnKey::CasualtiesTaken] {
        let column = columns.iter().find(|column| column.key == key).unwrap();
        assert_eq!(column.partial, Some(ScoreboardFamily { kind: "casualty-totals".into(), measure: "partial".into() }));
        assert!(column.none_at_zero, "{key:?}: a country with no casualty row has dealt or taken none");
    }
    assert!(columns.iter().filter(|column| !matches!(column.key, ScoreboardColumnKey::CasualtiesDealt | ScoreboardColumnKey::CasualtiesTaken)).all(|column| !column.none_at_zero),
        "a missing row of any other measure stays missing");
    assert!(DEFAULT_COLUMNS.iter().all(|key| columns.iter().any(|column| column.key == *key && column.available)));
    assert!(columns.iter().any(|column| column.key == DEFAULT_SORT && column.available));
}

#[test]
fn scoreboard_panel_reads_rank_and_values_together_and_reuses_the_readers() {
    use dashboard_base::{SourceKey, test_fixtures::{CAMPAIGN, records, source}};
    use replay::campaign::Replay;
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    use replay_recording::cells::{ReadCounts, count_reads};
    use serde_json::json;

    let frames = [100, 101, 102, 103, 110, 111].map(|hour| {
        let mut frame = source(1, hour, Some(if hour == 100 || hour == 102 { 40_000 } else { 10_000 }));
        if let TableView::Complete(TableRows::Countries(rows)) = &mut *frame.tables[TableId::Countries.index()] {
            rows.push(CountryRow {
                id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), political_power_raw: Some(20_000),
                command_power_raw: if hour < 110 { 210_000 } else { 310_000 }, ..Default::default()
            });
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) = &mut *frame.tables[TableId::CountryLinks.index()] {
            rows.push(CountryLinkRow { id: 9002, canonical_slot: 8 });
        }
        let factories = [9001, 9002].map(|country| CountryFactoriesRow { country, military_levels: hour + country as i32 - 9000, ..Default::default() });
        *frame.tables[TableId::CountryFactories.index()] = if hour == 102 {
            TableView::Unavailable { reason: "fixture factories unreadable".into() }
        } else {
            TableView::Complete(TableRows::CountryFactories(factories.into()))
        };
        let casualties = [7, 8].map(|slot| CasualtyTotalRow {
            slot, id: 8994 + slot, dealt: Recorded::Value(i64::from(hour)), taken: Recorded::Value(i64::from(slot)), partial: u8::from(slot == 8),
            ..Default::default()
        });
        *frame.tables[TableId::CasualtyTotals.index()] = TableView::Complete(TableRows::CasualtyTotals(casualties.into()));
        frame
    });
    let current = replay_recording::schema::columns_at(replay_data::FORMAT_VERSION).unwrap();
    let mut inputs = current.clone();
    inputs[TableId::CasualtyTotals.index()] = 0;
    let writers = recorder_derive::writers_within(&inputs, &current);
    let replay = Replay::from_records(writers, CAMPAIGN.into(), &[records(1, frames.into())])
        .unwrap().into_timelines(&Default::default()).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = Arc::new(Source::from_replay(SourceKey { source_id: "scoreboard-panel".into(), revision: "1".into() },
        "synthetic.battleplan".into(), Arc::new(replay), &cancelled).unwrap());
    let load = LoadContext { content: None };
    let request = json!({"panel": "scoreboard", "branchId": "1",
        "columns": ["politicalPower", "factoriesBuilt", "casualtiesTaken", "casualtiesDealt"],
        "top": {"count": 1, "sort": "politicalPower", "descending": true}});
    let followed = panel(&source, &load, &request).unwrap();
    let exact = followed.exact().unwrap();
    let opens = |counts: &ReadCounts| [TableId::Countries, TableId::CountryFactories, TableId::CasualtyTotals]
        .iter().filter_map(|table| counts.get(table)).map(|stats| stats.opened).sum::<u64>();
    for (hour, seek, expected_opens) in [(100, true, 3), (101, false, 0), (102, false, 0), (101, true, 0),
        (106, false, 0), (110, false, 3), (111, false, 0)] {
        let (oracle, cold) = count_reads(|| (exact.cold)(hour, &cancelled));
        let oracle = oracle.unwrap();
        let (step, warm) = count_reads(|| (exact.advance)(hour, seek, &cancelled, None));
        let reply = step.value.as_ref().as_ref().unwrap();
        assert_eq!(serde_json::to_vec(reply).unwrap(), serde_json::to_vec(&oracle).unwrap(), "scoreboard at {hour}");
        assert_eq!(opens(&warm), expected_opens, "table opens at {hour}");
        if hour == 101 {
            assert_eq!(opens(&cold), 4, "the cold rank and three table groups read independently");
            assert_eq!(reply["rows"][0]["tag"], "ITA", "the new leader has all its details in the same reply");
            assert_eq!(reply["rows"][0]["cells"][2]["partial"], true);
            assert_eq!(reply["rows"][0]["cells"][3]["valueRaw"], "101");
        }
        if hour == 102 {
            assert_eq!(reply["rows"][0]["tag"], "GER");
            assert!(reply["rows"][0]["cells"][1]["missing"].as_str().unwrap().contains("fixture factories unreadable"));
        }
        if hour == 106 { assert!(reply["missing"].as_str().is_some()); }
    }
    let mut wider_request = request.clone();
    wider_request["columns"].as_array_mut().unwrap().push("commandPower".into());
    let wider = panel(&source, &load, &wider_request).unwrap();
    let expanded = wider.exact().unwrap();
    for (reader, hour, expected_opens) in [(expanded, 111, 1), (exact, 110, 0), (expanded, 111, 0)] {
        let (step, warm) = count_reads(|| (reader.advance)(hour, true, &cancelled, None));
        assert_eq!(step.value.as_ref(), &(reader.cold)(hour, &cancelled));
        assert_eq!(opens(&warm), expected_opens, "a second scoreboard adds a field without discarding the first projection");
    }

    let replay = source.replay_ref();
    let page = replay.recording().table_entry(1, 0, TableId::Countries.index()).unwrap();
    let column = dashboard_declare::field(TableId::Countries, "command_power_raw").unwrap();
    let stream = replay_columnar::unit::restore::column_stream(column);
    let restore = page.unit.first.as_ref().unwrap();
    let chunk = &restore.streams.iter().find(|(slot, _)| *slot == stream).unwrap().1[0].frame;
    let mut bytes = replay.bytes().to_vec();
    bytes[(chunk.offset + u64::from(chunk.stored) / 2) as usize] ^= 0x40;
    let damaged_replay = Replay::open(bytes).unwrap().into_timelines(&Default::default()).unwrap();
    let damaged = Arc::new(Source::from_replay(SourceKey { source_id: "scoreboard-panel-damaged".into(), revision: "1".into() },
        "synthetic.battleplan".into(), Arc::new(damaged_replay), &cancelled).unwrap());
    let narrow = panel(&damaged, &load, &request).unwrap();
    let narrow = narrow.exact().unwrap();
    let wide = panel(&damaged, &load, &wider_request).unwrap();
    let wide = wide.exact().unwrap();
    assert!((wide.advance)(101, true, &cancelled, None).value.is_ok());
    assert!((wide.cold)(110, &cancelled).is_err(), "the extra command power column is damaged");
    let oracle = (narrow.cold)(110, &cancelled).expect("the narrow reply can read the undamaged columns");
    let step = (narrow.advance)(110, false, &cancelled, None);
    assert_eq!(step.value.as_ref().as_ref().unwrap(), &oracle);
    let (step, counts) = count_reads(|| (narrow.advance)(111, false, &cancelled, None));
    assert_eq!(step.value.as_ref(), &(narrow.cold)(111, &cancelled));
    assert_eq!(opens(&counts), 3, "the failed widened reader was released before the next healthy step");
}

