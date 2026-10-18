use super::*;
#[cfg(feature = "slow-tests")]
use dashboard_base::test_fixtures::scale_frame;
use dashboard_base::test_fixtures::{stability_frame, stability_inputs};
use replay_data::{rows::CountryStabilityLineRow, rows::TableRows};

fn samples(frame: &DecodedFrame, requested: Option<&[(String, String)]>) -> Samples {
    sample(
        &mut Scan::default(),
        frame,
        requested,
        None,
        |_, _, _, _, _, _, _, _| Ok(()),
    )
    .unwrap()
}

#[cfg(feature = "slow-tests")]
#[test]
fn sampler_and_observer_agree_on_every_key_for_5000_countries() {
    let mut frame = scale_frame(1, 100, 5_000);
    let inputs = (0..5_000)
        .map(|index| {
            let mut input = stability_inputs();
            input.country = 100_000 + index;
            input.stability_raw += i64::from(index);
            input
        })
        .collect();
    *frame.tables[TableId::CountryStabilityInputs.index()] =
        TableView::Complete(TableRows::CountryStabilityInputs(inputs));
    *frame.tables[TableId::CountryStabilityLines.index()] =
        TableView::Complete(TableRows::CountryStabilityLines(vec![]));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(
        (0..5_000)
            .map(|index| replay_data::rows::SliceReadRow {
                owner: 100_000 + index,
                table: TableId::CountryStabilityInputs as u32,
                sample_hour: 100,
            })
            .collect(),
    ));
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![]));
    let expected = samples(&frame, None);
    let mut prepared = Prepared::default();
    prepared.observe(&frame).unwrap();
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let (keys, actors) = dashboard_base::prepared::series::read_catalogue::<
        Vec<dashboard_base::prepared_measure::MeasureActor>,
    >(&fixture.store, &fixture.key, "parts", 1)
    .unwrap()
    .unwrap();
    assert_eq!(actors.len(), expected.len());
    let mut reader = dashboard_base::prepared::series::Reader::new(
        &fixture.store,
        &fixture.key,
        "parts-1".into(),
        &flag,
    )
    .unwrap();
    for actor in &actors {
        let key = keys[&dashboard_base::prepared::series::field_key(&actor.tag, &actor.measure)];
        let (identity, value) = expected[&(actor.tag.clone(), actor.measure.clone())];
        assert_eq!(
            reader.value_at(key, 100).unwrap().map(|run| run.value),
            value
        );
        let (_, holder) = reader.latest_event(key, 100).unwrap().unwrap();
        let holder: dashboard_base::prepared_measure::MeasureHolder =
            serde_json::from_value(holder).unwrap();
        assert_eq!(Some((holder.slot, holder.country_id)), identity);
    }
    // A full 5,000-country selection goes through one actual adapter query,
    // not a per-country or per-key request loop.
    let source = super::super::tests::prepared_source(vec![frame.clone()]);
    let tags: Vec<_> = (0..5_000)
        .map(|index| String::from_utf8(dashboard_base::test_fixtures::scale_tag(index)).unwrap())
        .collect();
    let mut requests = 0;
    let mut query = || {
        requests += 1;
        prepared_query(&source, None, "series-state", serde_json::json!({"branchId":"1","family":{"kind":"stability-parts","measure":"stability"},"reads":tags.iter().map(|tag| serde_json::json!({"tag":tag,"hour":100})).collect::<Vec<_>>()}), &fixture.store, &fixture.key, None, &flag).unwrap()
    };
    let reply = query();
    assert_eq!(requests, 1);
    assert_eq!(reply["parts"].as_object().unwrap().len(), 5_000);
    assert!(reply["states"].as_array().unwrap().len() >= 5_000);
}

#[test]
fn missing_inputs_and_lines_are_unknown_and_only_requested_countries_expand() {
    let frame = stability_frame(100, stability_inputs(), 100, vec![]);
    let wanted = [
        ("GER".into(), "stability".into()),
        ("not recorded".into(), "warSupport".into()),
    ];
    let selected = samples(&frame, Some(&wanted));
    assert_eq!(selected[&wanted[1]], (None, None));
    assert!(
        selected
            .keys()
            .filter(|key| key != &&wanted[1])
            .all(|(tag, field)| {
                serde_json::from_str::<(String, String)>(tag).unwrap().0 == "GER"
                    && field == "stability"
            })
    );
    for table in [
        TableId::CountryStabilityInputs,
        TableId::CountryStabilityLines,
    ] {
        let mut missing = frame.clone();
        *missing.tables[table.index()] = TableView::Unavailable {
            reason: "not recorded".into(),
        };
        let unknown = samples(&missing, Some(&wanted[..1]));
        assert_eq!(
            unknown[&(
                serde_json::to_string(&("GER", "")).unwrap(),
                "stability".into()
            )],
            (Some((7, 9001)), None)
        );
    }
    let mut selected_tables = frame.clone();
    for table in TableId::ALL {
        if !TABLES.contains(&table) {
            *selected_tables.tables[table.index()] = TableView::Unavailable {
                reason: "not selected".into(),
            };
        }
    }
    assert_eq!(samples(&selected_tables, None), samples(&frame, None));
}

#[test]
fn carried_character_parts_never_use_the_current_names_but_formula_parts_stay_known() {
    let line = CountryStabilityLineRow {
        country: 9001,
        key: 0x8d,
        kind: CountryStabilityLineRow::COUNTRY_LEADER,
        identity_id: 7,
        value: 30_000,
        ..Default::default()
    };
    let mut frame = stability_frame(100, stability_inputs(), 100, vec![line]);
    *frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(vec![
        replay_data::rows::CharacterRow {
            id: 7,
            token: b"Old name".to_vec(),
            name_token_id: -1,
            ..Default::default()
        },
    ]));
    let first = samples(&frame, None);
    assert!(first.keys().any(|(tag, _)| tag.contains("Old name")));
    frame.hour = 101;
    if let TableView::Complete(TableRows::Characters(rows)) =
        &mut *frame.tables[TableId::Characters.index()]
    {
        rows[0].token = b"New name".to_vec();
    }
    let held = samples(&frame, None);
    assert!(!held.keys().any(|(tag, _)| tag.contains("countryLeader")));
    let base = (
        serde_json::to_string(&("GER", "term:base")).unwrap(),
        "stability".into(),
    );
    assert_eq!(held[&base], first[&base]);
    let source = super::super::tests::prepared_source(vec![frame]);
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let reply = prepared_query(&source, None, "series-state", serde_json::json!({"branchId":"1","family":{"kind":"stability-parts","measure":"stability"},"reads":[{"tag":"GER","hour":101}]}), &fixture.store, &fixture.key, None, &AtomicBool::new(false)).unwrap();
    assert!(
        reply["states"]
            .as_array()
            .unwrap()
            .iter()
            .any(|state| state["pending"] == true)
    );
    assert!(reply["warnings"].as_array().unwrap().iter().any(|warning| {
        warning
            .as_str()
            .unwrap()
            .contains("earlier character names")
    }));
}
