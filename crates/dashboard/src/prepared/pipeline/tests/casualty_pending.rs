use super::*;

#[test]
fn war_casualties_unprepared_spans_are_pending_until_retained_totals_are_published() {
    // Only the branch coverage page is published here: the casualties are
    // gated by it as hourly preparation prepares them.
    independent::with_test_hourly(|| {
        use crate::products::{LoadContext, war_casualties::prepared::Prepared};
        use crate::test_fixtures::scale_tag;
        use replay::rows::WarRelationRow;
        use serde_json::{Value, json};

        let sample = |hour| {
            let mut frame = scale_frame(1, hour, 2);
            *frame.tables[TableId::WarRelations.index()] =
                TableView::Complete(TableRows::WarRelations(
                    (hour < 12)
                        .then(|| WarRelationRow {
                            first_country: 100_000,
                            second_country: 100_001,
                            start_hour: 10,
                            first_casualties: i64::from(hour) * 3,
                            second_casualties: i64::from(hour),
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
        let source = fixture.source("casualty-pending");
        let flag = AtomicBool::new(false);
        let context = LoadContext { content: None };
        let (store, key) = crate::prepared::files(&source, None).unwrap();
        let mut prepared = Prepared::new(source.replay_ref(), 1);
        for frame in &prefix {
            prepared
                .observe(frame, source.replay_ref(), &source, &context, &flag)
                .unwrap();
        }
        let mut writer = store.replace(key.clone()).unwrap();
        prepared.finish_block(&mut writer, 1, 10, 11).unwrap();
        prepared.flush(&mut writer, 1, &flag).unwrap();
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
        let tag = String::from_utf8(scale_tag(0)).unwrap();
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
        let state = |hour| {
            json!({"branchId":"1","tags":[tag],"family":{"kind":"war-casualties"},
                "reads":[{"tag":"","hour":hour}]})
        };
        let covered = query("series-state", state(11));
        assert!(!covered["series"].as_array().unwrap().is_empty());
        let ranges = store.range_count(&key, "casualty-values-1-0").unwrap();
        assert_eq!(query("series-state", state(13)), Value::Null);
        assert_eq!(
            query(
                "series-summary",
                json!({"branchId":"1","tags":[tag],"family":{"kind":"war-casualties","measure":"taken"},
                    "from":10,"to":13,"width":4}),
            ),
            Value::Null,
            "an unprepared span is pending, not an empty or zero-valued summary"
        );
        assert_eq!(
            store.range_count(&key, "casualty-values-1-0").unwrap(),
            ranges
        );
        for frame in &tail {
            prepared
                .observe(frame, source.replay_ref(), &source, &context, &flag)
                .unwrap();
        }
        let mut writer = store.extend(&key).unwrap();
        prepared.finish_block(&mut writer, 1, 12, 13).unwrap();
        prepared.flush(&mut writer, 1, &flag).unwrap();
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
        let ready = query("series-state", state(13));
        assert!(!ready["series"].as_array().unwrap().is_empty());
        assert_eq!(
            ready["series"][0]["taken"][0],
            covered["series"][0]["taken"][0]
        );
        assert_eq!(ready["series"][0]["taken"][0], "33");
    });
}
