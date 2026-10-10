use super::*;
use replay::{
    frame::{Metrics, TableView},
    rows::{TableId, TableRows, WeatherProvinceRow, WeatherRegionRow},
};

#[test]
fn typed_weather_comparison_keeps_exact_words_and_refuses_unavailable_rows() {
    let mut saved = installed_content::save::parse(
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
            id: 0,
            snow: 9_007_199_254_740_993,
            permanent_snow: 4_000_000,
            display_snow: 128,
            display_mud: 255,
            temperature: replay::rows::Recorded::Value(100_000),
            raw_mud: replay::rows::Recorded::Value(7),
            local_hour: replay::rows::Recorded::Value(0),
            night_factor: replay::rows::Recorded::Value(100_000),
            display: replay::rows::Recorded::Value(Some(replay::rows::WeatherDisplay {
                width: 1,
                height: 1,
                edges: vec![2, 3],
                ..Default::default()
            })),
        }]));
    *frame.tables[TableId::WeatherRegions.index()] =
        TableView::Complete(TableRows::WeatherRegions(vec![WeatherRegionRow {
            id: 0,
            light_rain: 255,
            ..Default::default()
        }]));
    let mut report = Report::default();
    compare(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 2);
    assert_eq!(report.limitations.len(), 5);
    assert!(report.limitations.contains_key(
        "the weather manager writer omits the stored day/night array; these simulation values require a matched native read witness"
    ));

    // Historical absence withholds temperature/mud comparisons and identifies
    // the missing stored clock without inventing a save counterpart.
    let original = frame.tables[TableId::WeatherProvinces.index()].clone();
    if let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *frame.tables[TableId::WeatherProvinces.index()]
    {
        rows[0].temperature = replay::rows::Recorded::NotRecorded;
        rows[0].raw_mud = replay::rows::Recorded::NotRecorded;
        rows[0].local_hour = replay::rows::Recorded::NotRecorded;
        rows[0].night_factor = replay::rows::Recorded::NotRecorded;
        rows[0].display = replay::rows::Recorded::NotRecorded;
    }
    let mut old = Report::default();
    compare(&frame, &saved, &mut old).unwrap();
    assert!(old.differences.is_empty());
    assert_eq!(old.scalar_checks, 1); // snow remains compared
    assert_eq!(
        old.limitations["WeatherProvinces.temperature not recorded"].len(),
        1
    );
    assert_eq!(
        old.limitations["WeatherProvinces.raw_mud not recorded"].len(),
        1
    );
    assert_eq!(
        old.limitations["WeatherProvinces.local_hour not recorded"].len(),
        1
    );
    assert_eq!(
        old.limitations["WeatherProvinces.night_factor not recorded"].len(),
        1
    );
    assert_eq!(
        old.limitations["WeatherProvinces.display not recorded"].len(),
        1
    );
    frame.tables[TableId::WeatherProvinces.index()] = original;

    for (table, name, scalar_checks) in [
        (TableId::WeatherProvinces, "WeatherProvinces", 0),
        (TableId::WeatherRegions, "WeatherRegions", 2),
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
        assert!(compare(&frame, &saved, &mut report).is_err());
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, scalar_checks);
        assert_eq!(report.failed_comparisons.len(), 1);
        assert!(report.failed_comparisons.contains_key(name));
        assert_eq!(report.compared_tables.len(), 1);
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
    compare(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);

    let TableView::Complete(TableRows::WeatherProvinces(rows)) =
        &mut *frame.tables[TableId::WeatherProvinces.index()]
    else {
        unreachable!();
    };
    *rows[0]
        .temperature
        .require_mut("WeatherProvinces.temperature")
        .unwrap() -= 1;
    rows[0].raw_mud = replay::rows::Recorded::Value(0);
    let mut report = Report::default();
    compare(&frame, &saved, &mut report).unwrap();
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
    rows[0].temperature = replay::rows::Recorded::Value(0);
    rows[0].raw_mud = replay::rows::Recorded::Value(7);
    let expected = &mut saved.weather.provinces.as_mut().unwrap()[0];
    expected.temperature = Some(Decimal::parse(b"0").unwrap());
    let mut report = Report::default();
    compare(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    saved.weather.provinces.as_mut().unwrap()[0].temperature = None;
    let mut report = Report::default();
    compare(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("province 0.temperature"));

    *frame.tables[TableId::WeatherRegions.index()] = TableView::Unavailable {
        reason: "region read failed".into(),
    };
    assert!(compare(&frame, &saved, &mut Report::default()).is_err());
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
