use super::*;

#[test]
fn equipment_losses_unprepared_spans_are_pending_until_drain_totals_are_published() {
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
    assert_eq!(query("equipment-losses", params.clone()), Value::Null);
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
