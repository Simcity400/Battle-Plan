use super::*;
use replay_data::{
    rows::TableId, rows::TableRows, rows::WeatherProvinceRow, rows::WeatherRegionRow,
};
use replay_frame::{frame::Metrics, frame::TableView};

#[test]
fn typed_weather_comparison_keeps_exact_words_and_refuses_unavailable_rows() {
    let mut saved = content_reader_save::parse(
        br#"HOI4txt
date=1936.1.1.1 game_unique_id=campaign game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 version=build
countries={} states={} provinces={}
weather={
provinces={0={province=0 snow=90071992547.40993 temperature=1
    temperature_offset=-0.00001 mud=yes custom_modifiers={mud=-1 mud=7}}}
regions={0={region=0 temperature=-90071992547.40993
    next_weather_change=1936.1.2.1 rain_light=yes}}
}"#,
    )
    .unwrap();
    let hour = saved.date.engine_hour().unwrap();
    let content = replay_data::temperature::Content::default();
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "outside weather comparison".into(),
        })
        .map(Into::into),
    };
    *frame.tables[TableId::WeatherProvinces.index()] =
        TableView::Complete(TableRows::WeatherProvinces(vec![WeatherProvinceRow {
            daylight: replay_data::rows::Recorded::Value(None),
            id: 0,
            snow: 9_007_199_254_740_993,
            permanent_snow: 4_000_000,
            display_snow: 128,
            display_mud: 255,
            temperature: replay_data::rows::ProvinceTemperature::anchored(
                replay_data::temperature::State {
                    temperature: 100_000,
                    ..Default::default()
                },
            ),
            raw_mud: replay_data::rows::Recorded::Value(7),
            clock: replay_data::rows::DayNightClock::held(replay_data::day_night::Clock {
                local_hour: 0,
                night_factor: 100_000,
            }),
            display: replay_data::rows::Recorded::Value(Some(replay_data::rows::WeatherDisplay {
                width: 1,
                height: 1,
                edges: vec![2, 3],
                ..Default::default()
            })),
            temperature_pass: replay_data::rows::Recorded::Value(Some(
                replay_data::rows::TemperaturePass {
                    anchor: hour,
                    witness: replay_data::rows::TemperatureWitness::new(
                        replay_data::temperature::witness([100_000]),
                    ),
                    ..Default::default()
                },
            )),
            modifiers: replay_data::rows::Recorded::Value(Some(Default::default())),
            static_modifiers: static_list(&[("mud", -1), ("mud", 7)]),
        }]));
    *frame.tables[TableId::WeatherRegions.index()] =
        TableView::Complete(TableRows::WeatherRegions(vec![WeatherRegionRow {
            id: 0,
            light_rain: 255,
            custom_sea: Some(Vec::new()).into(),
            ..Default::default()
        }]));
    let mut report = Report::default();
    compare(
        &frame,
        &saved,
        Ok(&content),
        Err("outside this fixture".into()),
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 3);
    assert_eq!(
        report.compared_tables["WeatherProvinces static modifiers"],
        1
    );
    assert_eq!(
        report.failed_comparisons,
        BTreeMap::from([(
            "WeatherProvinces.modifiers".to_owned(),
            "WeatherProvinces.modifiers refused: outside this fixture; 1 recorded accumulators not compared".to_owned(),
        )])
    );
    assert!(report.coverage_line().contains(": incomplete;"));
    assert!(report.outcome().is_err());
    assert_eq!(report.limitations.len(), 7);
    assert!(report.limitations.values().flatten().any(|field| field ==
        "WeatherRegions.map_modifiers_known, naval_detection, air_attack, air_defence: runtime only"));
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|field| field == "WeatherProvinces.daylight: runtime only")
    );
    assert!(report.limitations.contains_key(
        "the weather manager writer omits the stored day/night array; held clocks require a matched native read witness, and updated clocks are the hour's derivation from content (`recorder derived` compares it with recorded clocks)"
    ));
    if let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *frame.tables[TableId::WeatherProvinces.index()]
    {
        rows[0].modifiers = replay_data::rows::Recorded::NotRecorded;
    }

    for (list, differences, failed, not_recorded) in [
        (static_list(&[("mud", -1), ("mud", 6)]), 1, false, false),
        (static_list(&[("mud", 7), ("mud", -1)]), 1, false, false),
        (static_list(&[("mud", -1)]), 1, false, false),
        (replay_data::rows::Recorded::Value(None), 0, true, false),
        (replay_data::rows::Recorded::NotRecorded, 0, false, true),
    ] {
        let original = frame.tables[TableId::WeatherProvinces.index()].clone();
        if let TableView::Complete(TableRows::WeatherProvinces(rows)) =
            &mut *frame.tables[TableId::WeatherProvinces.index()]
        {
            rows[0].static_modifiers = list.clone();
        }
        let mut report = Report::default();
        compare(
            &frame,
            &saved,
            Ok(&content),
            Err("outside this fixture".into()),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), differences, "{list:?}");
        assert!(
            report
                .differences
                .iter()
                .all(|difference| difference.contains("province 0.static_modifiers")),
            "{list:?}"
        );
        assert_eq!(
            report
                .failed_comparisons
                .contains_key("weather province 0.static_modifiers"),
            failed,
            "{list:?}"
        );
        assert_eq!(
            report
                .limitations
                .contains_key("WeatherProvinces.static_modifiers not recorded"),
            not_recorded,
            "{list:?}"
        );
        frame.tables[TableId::WeatherProvinces.index()] = original;
    }

    let original = frame.tables[TableId::WeatherProvinces.index()].clone();
    if let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *frame.tables[TableId::WeatherProvinces.index()]
    {
        rows[0].temperature = replay_data::rows::ProvinceTemperature::NOT_RECORDED;
        rows[0].raw_mud = replay_data::rows::Recorded::NotRecorded;
        rows[0].clock = replay_data::rows::DayNightClock::NOT_RECORDED;
        rows[0].display = replay_data::rows::Recorded::NotRecorded;
    }
    let mut old = Report::default();
    compare(
        &frame,
        &saved,
        Ok(&content),
        Err("outside this fixture".into()),
        &mut old,
    )
    .unwrap();
    assert!(old.differences.is_empty());
    assert_eq!(old.scalar_checks, 2);
    assert_eq!(
        old.limitations["WeatherProvinces.temperature not recorded"].len(),
        1
    );
    assert_eq!(
        old.limitations["WeatherProvinces.raw_mud not recorded"].len(),
        1
    );
    assert_eq!(
        old.limitations["WeatherProvinces.clock not recorded"].len(),
        1
    );
    assert_eq!(
        old.limitations["WeatherProvinces.display not recorded"].len(),
        1
    );
    frame.tables[TableId::WeatherProvinces.index()] = original;

    for (table, name, scalar_checks) in [
        (TableId::WeatherProvinces, "WeatherProvinces", 0),
        (TableId::WeatherRegions, "WeatherRegions", 3),
    ] {
        let original = frame.tables[table.index()].clone();
        *frame.tables[table.index()] = if table == TableId::WeatherProvinces {
            TableView::Truncated {
                rows: TableRows::WeatherProvinces(Vec::new()),
                missing: 1,
            }
        } else {
            TableView::Unavailable {
                reason: "weather read failed".into(),
            }
        };
        let mut report = Report::default();
        assert!(
            compare(
                &frame,
                &saved,
                Ok(&content),
                Err("outside this fixture".into()),
                &mut report
            )
            .is_err()
        );
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, scalar_checks);
        assert_eq!(report.failed_comparisons.len(), 1);
        assert!(report.failed_comparisons.contains_key(name));
        assert_eq!(
            report.compared_tables.len(),
            if table == TableId::WeatherProvinces {
                1
            } else {
                3
            }
        );
        assert!(!report.compared_tables.contains_key(name));
        frame.tables[table.index()] = original;
    }

    let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *frame.tables[TableId::WeatherProvinces.index()]
    else {
        unreachable!();
    };
    rows[0].snow -= 1;
    let mut report = Report::default();
    compare(
        &frame,
        &saved,
        Ok(&content),
        Err("outside this fixture".into()),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1);

    let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *frame.tables[TableId::WeatherProvinces.index()]
    else {
        unreachable!();
    };
    rows[0].temperature =
        replay_data::rows::ProvinceTemperature::anchored(replay_data::temperature::State {
            temperature: 99_999,
            ..Default::default()
        });
    rows[0].temperature_pass =
        replay_data::rows::Recorded::Value(Some(replay_data::rows::TemperaturePass {
            anchor: hour,
            witness: replay_data::rows::TemperatureWitness::new(replay_data::temperature::witness(
                [99_999],
            )),
            ..Default::default()
        }));
    rows[0].raw_mud = replay_data::rows::Recorded::Value(0);
    let mut report = Report::default();
    compare(
        &frame,
        &saved,
        Ok(&content),
        Err("outside this fixture".into()),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 3);
    for field in ["snow", "temperature", "mud"] {
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains(&format!("province 0.{field}")))
        );
    }

    let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *frame.tables[TableId::WeatherProvinces.index()]
    else {
        unreachable!();
    };
    rows[0].snow += 1;
    rows[0].temperature = replay_data::rows::ProvinceTemperature::anchored(
        replay_data::temperature::State::default(),
    );
    rows[0].temperature_pass =
        replay_data::rows::Recorded::Value(Some(replay_data::rows::TemperaturePass {
            anchor: hour,
            witness: replay_data::rows::TemperatureWitness::new(replay_data::temperature::witness(
                [0],
            )),
            ..Default::default()
        }));
    rows[0].raw_mud = replay_data::rows::Recorded::Value(7);
    let expected = &mut saved.weather.provinces.as_mut().unwrap()[0];
    expected.temperature = Some(Decimal::parse(b"0").unwrap());
    let mut report = Report::default();
    compare(
        &frame,
        &saved,
        Ok(&content),
        Err("outside this fixture".into()),
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    saved.weather.provinces.as_mut().unwrap()[0].temperature = None;
    let mut report = Report::default();
    compare(
        &frame,
        &saved,
        Ok(&content),
        Err("outside this fixture".into()),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("province 0.temperature"));

    *frame.tables[TableId::WeatherRegions.index()] = TableView::Unavailable {
        reason: "region read failed".into(),
    };
    assert!(
        compare(
            &frame,
            &saved,
            Ok(&content),
            Err("outside this fixture".into()),
            &mut Report::default()
        )
        .is_err()
    );
}

#[test]
fn region_custom_sea_compares_with_saved_active_modifiers_and_never_matches_unrecorded() {
    use replay_data::rows::Recorded;
    let mut report = Report::default();
    custom_sea(&mut report, "region 3", &Some(vec![1, 0, -1]).into(), Some(&[1, 0, -1]));
    custom_sea(&mut report, "region 4", &Some(Vec::new()).into(), None);
    assert!(report.differences.is_empty());
    for (captured, saved) in [
        (Recorded::Value(Some(vec![1, 1])), Some(&[1, 0][..])),
        (Recorded::Value(Some(vec![1])), None),
        (Recorded::Value(Some(Vec::new())), Some(&[0][..])),
        (Recorded::Value(None), None),
    ] {
        let mut report = Report::default();
        custom_sea(&mut report, "region 3", &captured, saved);
        assert_eq!(report.differences.len(), 1, "{captured:?}");
        assert!(report.differences[0].contains("region 3.active_modifiers"));
    }
    let mut old = Report::default();
    custom_sea(&mut old, "region 3", &Recorded::NotRecorded, Some(&[1][..]));
    assert!(old.differences.is_empty());
    assert_eq!(old.limitations["WeatherRegions.custom_sea not recorded"].len(), 1);
}

#[test]
fn weather_sparse_zero_and_nonzero_flags_follow_serializer_information() {
    let mut report = Report::default();
    sparse_scalar(&mut report, "province 0", "water", 0, None);
    truth(&mut report, "province 0.mud", 0, None);
    truth(&mut report, "region 0.snow", 255, Some(true));
    assert!(report.differences.is_empty());
    sparse_scalar(&mut report, "province 0", "snow", -1, None);
    truth(&mut report, "region 0.snow", 255, None);
    truth(&mut report, "region 1.snow", 0, Some(true));
    assert_eq!(report.differences.len(), 3);
    assert_eq!(report.scalar_checks, 2);
}

#[test]
fn weather_dense_census_includes_zero_and_rejects_duplicates_or_holes() {
    assert!(dense_slot(0, 0, "weather").is_ok());
    assert!(dense_slot(1, 1, "weather").is_ok());
    assert!(dense_slot(0, 1, "weather").is_err());
    assert!(dense_slot(1, 0, "weather").is_err());
    assert!(dense_slot(1, 2, "weather").is_err());
}

#[test]
fn anchored_temperatures_compare_their_derivation_and_refuse_without_one() {
    let saved = content_reader_save::parse(
        br#"HOI4txt
date=1936.1.1.1 game_unique_id=campaign game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 version=build
countries={} states={} provinces={}
weather={
provinces={0={province=0 snow=0 temperature=1}}
regions={0={region=0}}
}"#,
    )
    .unwrap();
    let hour = saved.date.engine_hour().unwrap();
    let state = replay_data::temperature::State {
        temperature: 100_000,
        ..Default::default()
    };
    let pass = |witness| replay_data::rows::TemperaturePass {
        anchor: hour,
        witness: replay_data::rows::TemperatureWitness::new(witness),
        ..Default::default()
    };
    let frame = |pass| {
        let mut frame = DecodedFrame {
            branch: 1,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "outside weather comparison".into(),
            })
            .map(Into::into),
        };
        *frame.tables[TableId::WeatherProvinces.index()] =
            TableView::Complete(TableRows::WeatherProvinces(vec![WeatherProvinceRow {
                temperature: replay_data::rows::ProvinceTemperature::anchored(state),
                raw_mud: replay_data::rows::Recorded::Value(0),
                temperature_pass: replay_data::rows::Recorded::Value(pass),
                modifiers: replay_data::rows::Recorded::NotRecorded,
                static_modifiers: replay_data::rows::Recorded::NotRecorded,
                ..Default::default()
            }]));
        *frame.tables[TableId::WeatherRegions.index()] =
            TableView::Complete(TableRows::WeatherRegions(vec![WeatherRegionRow {
                custom_sea: Some(Vec::new()).into(),
                ..Default::default()
            }]));
        frame
    };
    let content = replay_data::temperature::Content {
        provinces: vec![replay_data::temperature::Province {
            active: false,
            neighbours: Vec::new(),
            region: None,
            terrain: Ok(None),
        }],
        ..Default::default()
    };
    let witness = replay_data::temperature::witness([100_000]);
    let mut report = Report::default();
    compare(
        &frame(Some(pass(witness))),
        &saved,
        Ok(&content),
        Err("outside this fixture".into()),
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report.failed_comparisons.is_empty(),
        "{:?}",
        report.failed_comparisons
    );
    assert_eq!(report.scalar_checks, 2);
    let mut report = Report::default();
    compare(
        &frame(Some(pass(witness ^ 1))),
        &saved,
        Ok(&content),
        Err("outside this fixture".into()),
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 1);
    assert_eq!(
        report.failed_comparisons.len(),
        1,
        "{:?}",
        report.failed_comparisons
    );
    assert!(
        report.failed_comparisons["WeatherProvinces.temperature"].contains("temperature refused")
            && report.failed_comparisons["WeatherProvinces.temperature"]
                .contains("differ from the recorded witness")
    );
    assert!(report.coverage_line().contains(": incomplete;"));
    assert!(report.outcome().is_err());
    let mut report = Report::default();
    compare(
        &frame(None),
        &saved,
        Ok(&content),
        Err("outside this fixture".into()),
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 1);
    assert_eq!(
        report.failed_comparisons.keys().collect::<Vec<_>>(),
        ["WeatherProvinces.temperature"]
    );
    assert!(report.outcome().is_err());
}

fn static_list(
    values: &[(&str, i32)],
) -> replay_data::rows::Recorded<Option<replay_data::rows::ProvinceStaticModifiers>> {
    replay_data::rows::Recorded::Value(Some(replay_data::rows::ProvinceStaticModifiers {
        entries: values
            .iter()
            .map(|&(name, days)| replay_data::rows::ProvinceStaticModifier {
                name: name.as_bytes().to_vec(),
                days,
            })
            .collect(),
    }))
}

fn statics(values: &[(&str, i64)]) -> content_reader_script::modifiers::Modifiers {
    let mut block = content_reader_script::modifiers::Modifiers::default();
    block.values = values
        .iter()
        .map(|&(name, value)| content_reader_script::modifiers::Modifier {
            key: name.to_owned(),
            name: name.to_ascii_lowercase(),
            text: String::new(),
            value,
        })
        .collect();
    block
}

fn entries(values: &[(u32, i64)]) -> replay_data::rows::Recorded<Option<ProvinceWeatherModifiers>> {
    replay_data::rows::Recorded::Value(Some(ProvinceWeatherModifiers {
        entries: values
            .iter()
            .map(|&(key, value)| replay_data::rows::ProvinceModifierEntry { key, value })
            .collect(),
    }))
}

#[test]
fn province_modifier_accumulators_compare_with_their_rebuild_from_the_save() {
    use replay_data::{rows::LexicalTokenRow, rows::ModifierDefinitionRow};

    let saved = content_reader_save::parse(
        br#"HOI4txt
date=1936.1.1.1 game_unique_id=campaign game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 version=build
countries={} states={} provinces={}
weather={
provinces={
    0={province=0 temperature=-25 mud=yes custom_modifiers={flood=-1 flood=30}}
    1={province=1 snow=6 temperature=-10}
    2={province=2 snow=3 temperature=0}
}
regions={}
}"#,
    )
    .unwrap();
    let (speed, attrition, cold, supply) = (1, 2, 3, 4);
    let names: [(u32, &[u8]); 5] = [
        (speed, b"speed".as_slice()),
        (attrition, b"Attrition".as_slice()),
        (cold, b"cold".as_slice()),
        (supply, b"supply".as_slice()),
        (5, b"uncategorised".as_slice()),
    ];
    let mut content = ProvinceModifierContent {
        rules: ProvinceModifierRules {
            temperature: [
                [-10_000_000, -2_000_000],
                [-3_000_000, -500_000],
                [2_500_000, 3_500_000],
                [3_000_000, 10_000_000],
            ],
            ground_snow: [[100_000, 1_000_000], [500_000, 2_000_000]],
            snow_gain_max: 1_200_000,
        },
        statics: [
            (
                "weather_mud",
                statics(&[
                    ("speed", -30_000),
                    ("ATTRITION", 10_000),
                    ("uncategorised", 7),
                    ("unregistered", 9),
                ]),
            ),
            ("weather_extreme_cold", statics(&[("cold", 50_000)])),
            ("weather_very_cold", statics(&[("cold", 20_000)])),
            ("weather_very_hot", statics(&[])),
            ("weather_extreme_hot", statics(&[("supply", -10_000)])),
            ("weather_ground_snow_medium", statics(&[("speed", -10_000)])),
            (
                "weather_ground_snow_high",
                statics(&[("speed", -20_000), ("cold", 5_000)]),
            ),
            ("flood", statics(&[("supply", -25_000), ("speed", -5_000)])),
        ]
        .into_iter()
        .map(|(name, block)| (name.to_owned(), Ok(block)))
        .collect(),
        notes: Vec::new(),
    };
    let row = |id, snow, permanent_snow, mud, modifiers| WeatherProvinceRow {
        id,
        snow,
        permanent_snow,
        raw_mud: replay_data::rows::Recorded::Value(mud),
        temperature: replay_data::rows::ProvinceTemperature::NOT_RECORDED,
        modifiers,
        static_modifiers: replay_data::rows::Recorded::NotRecorded,
        ..Default::default()
    };
    let rows = vec![
        row(
            0,
            0,
            0,
            1,
            entries(&[
                (speed, -40_000),
                (attrition, 10_000),
                (cold, 50_000),
                (supply, -50_000),
            ]),
        ),
        row(
            1,
            600_000,
            100_000,
            0,
            entries(&[(speed, -20_000), (cold, 25_000)]),
        ),
        row(2, 300_000, 0, 0, entries(&[(speed, -10_000)])),
    ];
    let mut frame = DecodedFrame {
        branch: 1,
        hour: saved.date.engine_hour().unwrap(),
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "outside weather comparison".into(),
        })
        .map(Into::into),
    };
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(
            names
                .iter()
                .map(|&(index, _)| ModifierDefinitionRow {
                    index,
                    token: index + 10,
                    category_mask: u32::from(index != 5),
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        names
            .iter()
            .map(|&(index, literal)| LexicalTokenRow {
                token: index + 10,
                literal: literal.to_vec(),
            })
            .collect(),
    ));
    let run = |frame: &mut DecodedFrame,
               rows: &[WeatherProvinceRow],
               content: Result<&ProvinceModifierContent, String>| {
        *frame.tables[TableId::WeatherProvinces.index()] =
            TableView::Complete(TableRows::WeatherProvinces(rows.to_vec()));
        let mut report = Report::default();
        compare_provinces(
            frame,
            &saved,
            Err("outside this fixture".into()),
            content,
            &mut report,
        )
        .unwrap();
        report
    };
    let report = run(&mut frame, &rows, Ok(&content));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["WeatherProvinces modifiers"], 3);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("does not hold permanent snow"))
    );

    let mut held = rows.clone();
    held[2].modifiers = entries(&[(cold, 20_000)]);
    let report = run(&mut frame, &held, Ok(&content));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(report.limitations.values().flatten().any(|subject| subject
        == "WeatherProvinces.modifiers mud, thermal and ground snow part: runtime-only; no text-save equality claim"));
    held[2].modifiers = entries(&[(cold, 20_000), (supply, 1)]);
    let report = run(&mut frame, &held, Ok(&content));
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    held[2].modifiers = entries(&[(cold, 20_000), (supply, -25_000), (speed, -5_000)]);
    let report = run(&mut frame, &held, Ok(&content));
    assert!(!report.differences.is_empty());
    let mut wrong_term = content.clone();
    wrong_term.statics.insert(
        "flood".into(),
        Ok(statics(&[("supply", -25_001), ("speed", -5_000)])),
    );
    let report = run(&mut frame, &rows, Ok(&wrong_term));
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.starts_with("weather province 0.modifiers.supply")),
        "{:?}",
        report.differences
    );

    let mut collided = rows.clone();
    collided[0].modifiers = entries(&[
        (speed, -50_000),
        (attrition, 10_000),
        (cold, 50_000),
        (supply, -50_000),
    ]);
    let report = run(&mut frame, &collided, Ok(&content));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let mut no_snow = content.clone();
    no_snow.rules.ground_snow = [[0, 0], [0, 0]];
    let report = run(&mut frame, &collided, Ok(&no_snow));
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.starts_with("weather province 0.modifiers.speed")),
        "{:?}",
        report.differences
    );
    let mut twin = content.clone();
    twin.statics.insert(
        "weather_ground_snow_high".into(),
        Ok(statics(&[("supply", 0)])),
    );
    let mut twin_rows = rows.clone();
    twin_rows[1].modifiers = entries(&[(cold, 20_000), (supply, 0)]);
    let report = run(&mut frame, &twin_rows, Ok(&twin));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["WeatherProvinces modifiers"], 2);
    assert_eq!(
        report.failed_comparisons,
        BTreeMap::from([(
            "weather province 0.modifiers".to_owned(),
            "weather province 0.modifiers save-backed part ambiguous: 2 distinct mud, thermal and ground snow states the content can select fit".to_owned(),
        )])
    );
    assert!(report.coverage_line().contains(": incomplete;"));
    assert!(report.outcome().is_err());

    let mut huge = content.clone();
    huge.statics.insert(
        "flood".into(),
        Ok(statics(&[("supply", 100_000_000_000_000)])),
    );
    let mut once = saved.clone();
    once.weather.provinces.as_mut().unwrap()[0].custom_modifiers = Some(vec![("flood".into(), 3)]);
    let compare_once = |frame: &mut DecodedFrame, supply_total: i64| {
        let mut wrapped = rows.clone();
        wrapped[0].modifiers = entries(&[
            (speed, -30_000),
            (attrition, 10_000),
            (cold, 50_000),
            (supply, supply_total),
        ]);
        *frame.tables[TableId::WeatherProvinces.index()] =
            TableView::Complete(TableRows::WeatherProvinces(wrapped));
        let mut report = Report::default();
        compare_provinces(
            frame,
            &once,
            Err("outside this fixture".into()),
            Ok(&huge),
            &mut report,
        )
        .unwrap();
        report
    };
    let report = compare_once(&mut frame, -84_467_440_737_095);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report.failed_comparisons.is_empty(),
        "{:?}",
        report.failed_comparisons
    );
    assert_eq!(report.compared_tables["WeatherProvinces modifiers"], 3);
    let report = compare_once(&mut frame, 100_000_000_000_000);
    assert_eq!(
        report.differences,
        [
            "weather province 0.modifiers.supply (definition 4): replay 100000000000000, save -84467440737095"
        ]
    );

    let mut changed = rows.clone();
    changed[0].modifiers = entries(&[
        (speed, -40_000),
        (attrition, 10_001),
        (cold, 50_000),
        (supply, -50_000),
    ]);
    changed[1].modifiers = entries(&[(speed, -20_000)]);
    changed[2].modifiers = entries(&[(speed, -10_000), (supply, 1), (9, 1)]);
    let report = run(&mut frame, &changed, Ok(&content));
    assert_eq!(report.differences.len(), 4, "{:?}", report.differences);
    for expected in [
        "weather province 0.modifiers.attrition (definition 2): replay 10001, save 10000",
        "weather province 1.modifiers.cold (definition 3): replay \"absent\", save 25000",
        "weather province 2.modifiers.supply (definition 4): replay 1, save \"absent\"",
        "weather province 2.modifiers.definition 9: replay 1, save \"absent\"",
    ] {
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference == expected),
            "{expected}: {:?}",
            report.differences
        );
    }

    let mut expired = saved.clone();
    expired.weather.provinces.as_mut().unwrap()[0].custom_modifiers =
        Some(vec![("flood".into(), 3)]);
    *frame.tables[TableId::WeatherProvinces.index()] =
        TableView::Complete(TableRows::WeatherProvinces(rows.clone()));
    let mut report = Report::default();
    compare_provinces(
        &frame,
        &expired,
        Err("outside this fixture".into()),
        Ok(&content),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);

    let mut refused = rows.clone();
    refused[2].modifiers = replay_data::rows::Recorded::Value(None);
    let report = run(&mut frame, &refused, Ok(&content));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["WeatherProvinces modifiers"], 2);
    assert_eq!(
        report.failed_comparisons,
        BTreeMap::from([(
            "weather province 2.modifiers".to_owned(),
            "weather province 2.modifiers: read refused, not compared".to_owned(),
        )])
    );
    assert!(report.coverage_line().contains(": incomplete;"));
    assert!(report.outcome().is_err());
    let report = run(&mut frame, &rows, Err("weather content failed".into()));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["WeatherProvinces modifiers"], 0);
    assert_eq!(
        report.failed_comparisons,
        BTreeMap::from([(
            "WeatherProvinces.modifiers".to_owned(),
            "WeatherProvinces.modifiers refused: weather content failed; 3 recorded accumulators not compared".to_owned(),
        )])
    );
    assert!(report.coverage_line().contains(": incomplete;"));
    assert!(report.outcome().is_err());
    content.statics.insert(
        "flood".into(),
        Err("flood in common/modifiers/a.txt: refused".into()),
    );
    content.statics.insert(
        "weather_ground_snow_medium".into(),
        Ok(statics(&[("unregistered", 1)])),
    );
    let report = run(&mut frame, &rows, Ok(&content));
    assert_eq!(report.compared_tables["WeatherProvinces modifiers"], 2);
    assert_eq!(
        report.failed_comparisons,
        BTreeMap::from([(
            "weather province 0.modifiers".to_owned(),
            "weather province 0.modifiers not rebuilt: flood in common/modifiers/a.txt: refused"
                .to_owned(),
        )])
    );
    assert_eq!(
        report.differences,
        ["weather province 2.modifiers.speed (definition 1): replay -10000, save \"absent\""]
    );

    *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "tokens unread".into(),
    };
    let report = run(&mut frame, &rows, Ok(&content));
    assert_eq!(report.compared_tables["WeatherProvinces modifiers"], 0);
    assert_eq!(
        report.failed_comparisons.keys().collect::<Vec<_>>(),
        ["WeatherProvinces.modifiers"]
    );
    assert!(
        report.failed_comparisons["WeatherProvinces.modifiers"]
            .starts_with("WeatherProvinces.modifiers refused: modifier definitions")
    );
    assert!(report.outcome().is_err());
}

#[test]
fn a_static_modifier_adds_its_own_categorised_entries() {
    use replay_data::{rows::LexicalTokenRow, rows::ModifierDefinitionRow};
    let definitions = [
        ModifierDefinitionRow {
            index: 1,
            token: 11,
            category_mask: 1,
            ..Default::default()
        },
        ModifierDefinitionRow {
            index: 2,
            token: 12,
            category_mask: 1,
            feature_selector: 4,
            eligibility: 1,
            ..Default::default()
        },
        ModifierDefinitionRow {
            index: 3,
            token: 13,
            ..Default::default()
        },
    ];
    let tokens =
        [(11, b"open"), (12, b"gate"), (13, b"none")].map(|(token, literal)| LexicalTokenRow {
            token,
            literal: literal.to_vec(),
        });
    let registry = Registry::new(&definitions, &tokens, Some(1)).unwrap();
    let mut names = BTreeMap::new();
    let mut parent = statics(&[("open", 0), ("gate", 5), ("none", 3)]);
    parent.children.push(statics(&[("open", 7)]));
    assert_eq!(
        ModifierRebuild::added(&registry, "parent", &parent, &mut names),
        Ok(vec![(1, 0), (2, 5)])
    );
    let gated = statics(&[("gate", 5)]);
    assert_eq!(
        ModifierRebuild::added(&registry, "gated", &gated, &mut names),
        Ok(Vec::new())
    );
    let mut with_child = gated.clone();
    with_child.children.push(statics(&[]));
    assert_eq!(
        ModifierRebuild::added(&registry, "gated", &with_child, &mut names),
        Ok(vec![(2, 5)])
    );
}

#[test]
fn a_static_value_scales_with_the_native_wrapping_multiply() {
    assert_eq!(scaled(25_000), 25_000);
    assert_eq!(scaled(-25_000), -25_000);
    assert_eq!(scaled(100_000_000_000_000), -84_467_440_737_095);
}
