use super::*;

#[test]
fn equipment_losses_unprepared_spans_are_pending_until_drain_totals_are_published() {
    independent::with_test_hourly(
        equipment_losses_unprepared_spans_are_pending_until_drain_totals_are_published_hourly,
    );
}

/// The loss pages are written as the hourly walk publishes them.
fn equipment_losses_unprepared_spans_are_pending_until_drain_totals_are_published_hourly() {
    use crate::products::{country_history::HistoryScan, equipment_losses::prepared::Prepared};
    use crate::test_fixtures::scale_tag;
    use replay::rows::{EquipmentDestructionCause, EquipmentDestructionRow};
    use serde_json::{Value, json};

    let sample = |hour| {
        let mut frame = scale_frame(1, hour, 2);
        *frame.tables[TableId::EquipmentDestruction.index()] =
            TableView::Complete(TableRows::EquipmentDestruction(
                (hour < 12)
                    .then(|| EquipmentDestructionRow {
                        country: 100_000,
                        cause: EquipmentDestructionCause::LandCombat,
                        variant: 17_u64 << 32
                            | u64::from(engine::cequipment_variant::REFERENCE_TYPE),
                        amount_raw: 100_001,
                        ..Default::default()
                    })
                    .into_iter()
                    .collect(),
            ));
        frame
    };
    let prefix = vec![sample(10), sample(11)];
    let tail = vec![sample(12), sample(13)];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, prefix.clone())]).unwrap();
    let replay = replay
        .append(&[continuation_records(&replay, 1, tail.clone())])
        .unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("equipment-loss-pending");
    let flag = AtomicBool::new(false);
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let mut prepared = Prepared::new(source.replay_ref(), 1, 10);
    let mut identities = HistoryScan::new(false, &[]);
    for frame in &prefix {
        prepared.observe(frame, source.replay_ref(), &flag).unwrap();
        identities.observe(frame);
    }
    let mut writer = store.replace(key.clone()).unwrap();
    prepared.finish_block(11);
    prepared.flush(&mut writer, 1, &flag).unwrap();
    json(&mut writer, "country-identities-1", &identities).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 10,
            last_hour: 11,
            complete: false,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let selection = json!({"branchId":"1","tag":String::from_utf8(scale_tag(0)).unwrap(),
        "landSource":"destruction","country":{"slot":0,"countryId":100_000,"originHour":10}});
    let query = |product: &str, params: Value| {
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params,
        };
        serde_json::from_str::<Value>(
            &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
        )
        .unwrap()["data"]
            .clone()
    };
    let params = json!({"selection":selection,"from":10,"to":13,"width":4,
        "reads":[10,13],"headHour":13,"detailHour":13});
    let ranges = store.range_count(&key, "equipment-loss-values-1").unwrap();
    assert!(ranges.is_some(), "real prefix series are already published");
    let partial = query("equipment-losses", params.clone());
    assert_eq!(partial["preparedHeadHour"], 11);
    assert_eq!(partial["pending"], json!([{"firstHour":12,"lastHour":13}]));
    assert_eq!(
        query(
            "series-state",
            json!({"branchId":"1","family":{"kind":"equipment-losses","selection":selection},
                "reads":[{"hour":13}]}),
        ),
        Value::Null,
        "the real series route must keep the unprepared hour pending"
    );
    assert_eq!(
        store.range_count(&key, "equipment-loss-values-1").unwrap(),
        ranges
    );
    for frame in &tail {
        prepared.observe(frame, source.replay_ref(), &flag).unwrap();
        identities.observe(frame);
    }
    let mut writer = store.extend(&key).unwrap();
    prepared.finish_block(13);
    prepared.flush(&mut writer, 1, &flag).unwrap();
    json(&mut writer, "country-identities-1", &identities).unwrap();
    json(
        &mut writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: 10,
            last_hour: 13,
            complete: true,
        },
    )
    .unwrap();
    writer.commit().unwrap();
    let ready = query("equipment-losses", params);
    assert_eq!(ready["preparedHeadHour"], 13);
    // The root `coverage` is the preparation coverage a client follows, so a
    // finished reply is final; the land coverage spans are `landCoverage`.
    assert_eq!(ready["coverage"]["complete"], true);
    assert_eq!(ready["pending"], json!([]));
    assert!(ready["landCoverage"].is_array());
    assert!(
        ready["causes"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|cause| cause["leaves"].as_array().unwrap())
            .flat_map(|leaf| leaf["points"].as_array().unwrap())
            .any(|point| point["hour"] == 11 && point["totalRaw"] == "200002"),
        "empty tail drains preserve both earlier signed drains, not zero losses"
    );
}

/// With the identity history prepared, losses answer before their own
/// coverage completes: each reached hour after the prepared prefix reads its
/// last hour, the reply names the later hours pending, and it waits for the
/// losses family alone.
#[test]
fn equipment_losses_answer_their_prepared_prefix_once_the_identities_are_prepared() {
    use crate::products::{country_history::HistoryScan, equipment_losses::prepared::Prepared};
    use crate::test_fixtures::scale_tag;
    use replay::rows::{EquipmentDestructionCause, EquipmentDestructionRow};
    use serde_json::{Value, json};

    let sample = |hour| {
        let mut frame = scale_frame(1, hour, 2);
        *frame.tables[TableId::EquipmentDestruction.index()] =
            TableView::Complete(TableRows::EquipmentDestruction(vec![
                EquipmentDestructionRow {
                    country: 100_000,
                    cause: EquipmentDestructionCause::LandCombat,
                    variant: 17_u64 << 32 | u64::from(engine::cequipment_variant::REFERENCE_TYPE),
                    amount_raw: 100_001,
                    ..Default::default()
                },
            ]));
        frame
    };
    let prefix = vec![sample(10), sample(11)];
    let mut tail = vec![sample(12), sample(13)];
    for frame in &mut tail {
        *frame.tables[TableId::SunkShips.index()] =
            TableView::Complete(TableRows::SunkShips(vec![replay::rows::SunkShipsRow {
                entries: vec![replay::rows::SunkShipEntry {
                    ship_name: b"synthetic ship".to_vec(),
                    victim_country: 100_000,
                    sink_hour: 10,
                    victim_equipment_variant: 17_u64 << 32
                        | u64::from(engine::cequipment_variant::REFERENCE_TYPE),
                    ..Default::default()
                }],
            }]));
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, prefix.clone())]).unwrap();
    let replay = replay
        .append(&[continuation_records(&replay, 1, tail.clone())])
        .unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("equipment-loss-prefix");
    let flag = AtomicBool::new(false);
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let coverage = |last_hour, complete| Coverage {
        schema: SCHEMA,
        first_hour: 10,
        last_hour,
        complete,
    };
    // Losses committed through hour 11; the identity history through the end.
    let mut prepared = Prepared::new(source.replay_ref(), 1, 10);
    for frame in &prefix {
        prepared.observe(frame, source.replay_ref(), &flag).unwrap();
    }
    let mut identities = HistoryScan::new(false, &[]);
    for frame in prefix.iter().chain(&tail) {
        identities.observe(frame);
    }
    let mut writer = store.replace(key.clone()).unwrap();
    prepared.finish_block(11);
    prepared.flush(&mut writer, 1, &flag).unwrap();
    json(&mut writer, "country-identities-1", &identities).unwrap();
    json(
        &mut writer,
        &independent::coverage_section("identities", 1),
        &coverage(13, true),
    )
    .unwrap();
    json(
        &mut writer,
        &independent::coverage_section("equipment_losses", 1),
        &coverage(11, false),
    )
    .unwrap();
    writer.commit().unwrap();
    let selection = json!({"branchId":"1","tag":String::from_utf8(scale_tag(0)).unwrap(),
        "landSource":"destruction","country":{"slot":0,"countryId":100_000,"originHour":10}});
    let params = json!({"selection":selection,"from":10,"to":13,"width":4,
        "reads":[10,12,13],"headHour":13,"detailHour":12});
    let ask = |params| {
        independent::with_test_only(&["equipment_losses", "identities"], || {
            let request = crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: "equipment-losses".into(),
                params,
            };
            serde_json::from_str::<Value>(
                &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
            )
            .unwrap()
        })
    };
    let reply = ask(params.clone());
    // The old reply waited for the losses family's whole coverage.
    let data = &reply["data"];
    assert!(!data.is_null(), "a prefix reply before completion");
    assert_eq!(data["coverage"], json!(coverage(11, false)));
    assert_eq!(
        data["approximate"],
        json!([{"firstHour": 10, "lastHour": 11}])
    );
    assert_eq!(data["pending"], json!([{"firstHour": 12, "lastHour": 13}]));
    assert_eq!(data["preparedReads"], json!([10, 11]));
    assert_eq!(data["preparedHeadHour"], 11);
    assert_eq!(data["preparedDetailHour"], 11);
    assert_eq!(data["preparedTo"], 11.0);
    assert!(data["landCoverage"].is_array());
    assert!(
        data["causes"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|cause| cause["leaves"].as_array().unwrap())
            .flat_map(|leaf| leaf["points"].as_array().unwrap())
            .any(|point| point["hour"] == 11 && point["totalRaw"] == "200002"),
        "the prefix's signed total at its last hour"
    );
    assert_eq!(reply["gate"], json!(["equipment_losses"]));
    for frame in &tail {
        prepared.observe(frame, source.replay_ref(), &flag).unwrap();
    }
    let mut writer = store.extend(&key).unwrap();
    prepared.finish_block(13);
    prepared.flush(&mut writer, 1, &flag).unwrap();
    json(
        &mut writer,
        &independent::coverage_section("equipment_losses", 1),
        &coverage(13, true),
    )
    .unwrap();
    writer.commit().unwrap();
    let complete = ask(json!({"selection": selection, "from":10,"to":11,"width":4,
        "reads":[10,11],"headHour":11,"detailHour":11}));
    assert_eq!(complete["data"]["approximate"], json!([]));
    assert_eq!(complete["data"]["pending"], json!([]));
    assert_ne!(
        data["causes"], complete["data"]["causes"],
        "the later observation backdates a sinking into the prefix"
    );
    // A later sinking can be dated at the branch's first hour. Every hour
    // this prefix presents is explicitly approximate until the scan completes.
    for hour in 10..=11 {
        assert!(
            data["approximate"]
                .as_array()
                .unwrap()
                .iter()
                .any(|span| span["firstHour"].as_i64().unwrap() <= hour
                    && hour <= span["lastHour"].as_i64().unwrap())
        );
    }
}
