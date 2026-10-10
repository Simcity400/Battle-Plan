use super::*;
use crate::rows::{decode_row, encode_row};

#[test]
fn weather_display_boundary_deltas_keep_zero_bytes_and_refresh_identity() {
    let before = WeatherProvinceRow {
        display: Recorded::Value(None),
        ..Default::default()
    };
    let after = WeatherProvinceRow {
        display: Recorded::Value(Some(WeatherDisplay {
            width: 1,
            height: 1,
            refresh_interval: i64::MIN,
            refresh_identity: u64::MAX,
            edges: vec![0, 0],
        })),
        ..before.clone()
    };
    let delta = after.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 8);
    assert_eq!(
        WeatherProvinceRow::apply_delta(&delta, Some(before)).unwrap(),
        after
    );
    let mut bytes = vec![];
    after.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        WeatherProvinceRow::decode_fields(after.key(), &mut Decoder::new(&bytes)).unwrap(),
        after
    );
    assert_ne!(after.display, Recorded::NotRecorded);
    assert_ne!(
        Recorded::<Option<WeatherDisplay>>::Value(None),
        Recorded::NotRecorded
    );
}

#[test]
fn temperature_and_raw_mud_deltas_keep_extreme_native_words() {
    let before = WeatherProvinceRow {
        id: 13_413,
        temperature: crate::rows::Recorded::Value(i64::MIN),
        ..Default::default()
    };
    let after = WeatherProvinceRow {
        temperature: crate::rows::Recorded::Value(i64::MAX),
        raw_mud: crate::rows::Recorded::Value(255),
        ..before.clone()
    };
    let delta = after.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(delta.mask, (1 << 4) | (1 << 5));
    assert_eq!(
        WeatherProvinceRow::apply_delta(&delta, Some(before)).unwrap(),
        after
    );
    let mut bytes = Vec::new();
    after.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        WeatherProvinceRow::decode_fields(after.key(), &mut Decoder::new(&bytes)).unwrap(),
        after
    );
}

#[test]
fn held_clock_deltas_preserve_zero_hour_and_full_signed_native_words() {
    let before = WeatherProvinceRow {
        id: 13_413,
        local_hour: i32::MIN.into(),
        night_factor: i64::MAX.into(),
        ..Default::default()
    };
    for (hour, night) in [(0, 0), (i32::MAX, i64::MIN)] {
        let after = WeatherProvinceRow {
            local_hour: hour.into(),
            night_factor: night.into(),
            ..before.clone()
        };
        let delta = after.delta(Some(&before)).unwrap().unwrap();
        assert_eq!(delta.mask, (1 << 6) | (1 << 7));
        assert_eq!(
            WeatherProvinceRow::apply_delta(&delta, Some(before.clone())).unwrap(),
            after
        );
        let mut bytes = Vec::new();
        after.encode_fields(&mut bytes).unwrap();
        assert_eq!(
            WeatherProvinceRow::decode_fields(after.key(), &mut Decoder::new(&bytes)).unwrap(),
            after
        );
    }
}

#[test]
fn naval_result_source_codec_keeps_full_references_hours_and_transient_witnesses() {
    let original = NavalResultSourceRow {
        id: u64::MAX,
        game_hour: i32::MIN,
        result: u64::MAX,
        combat: 0x8000_0041_0000_0035,
        combat_created_hour: None,
        ..Default::default()
    };
    for row in [
        original.clone(),
        NavalResultSourceRow {
            merged: true,
            ..original.clone()
        },
        NavalResultSourceRow {
            transient: true,
            combat_created_hour: Some(i32::MIN),
            ..original.clone()
        },
    ] {
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            decode_row::<NavalResultSourceRow>(0, &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(delta.mask, TableId::NavalResultSources.full_mask());
        assert_eq!(delta.apply::<NavalResultSourceRow>(None).unwrap(), row);
    }
    let merged = NavalResultSourceRow {
        merged: true,
        ..original.clone()
    };
    assert_eq!(merged.delta(Some(&original)).unwrap().unwrap().mask, 1 << 3);
    for combat_created_hour in [None, Some(i32::MIN + 1)] {
        let invalid = NavalResultSourceRow {
            transient: true,
            combat_created_hour,
            ..original.clone()
        };
        assert!(encode_row(&invalid, 0, &mut Vec::new()).is_err());
        assert!(invalid.delta(None).is_err());
    }
}

#[test]
fn naval_result_sources_roundtrip_five_thousand_independent_sources_exactly() {
    let rows: Vec<_> = (0..5_000_u64)
        .map(|id| NavalResultSourceRow {
            id,
            game_hour: 60_827_795,
            result: (id + 1) << 32 | 0x8123,
            combat: (id + 5_001) << 32 | 0x8123,
            merged: id.is_multiple_of(3),
            transient: id.is_multiple_of(2) || id.is_multiple_of(3),
            combat_created_hour: Some(60_827_795),
        })
        .collect();
    let expected = TableRows::NavalResultSources(rows);
    let mut bytes = Vec::new();
    expected.encode_rows(&mut bytes).unwrap();
    assert_eq!(bytes.len(), 5_000 * 28);
    assert_eq!(TableId::NavalResultSources.minimum_row_bytes(), 24);
    let mut input = Decoder::new(&bytes);
    let decoded = TableId::NavalResultSources
        .decode_rows(5_000, &mut input)
        .unwrap();
    decoded.validate_keys().unwrap();
    input.finish().unwrap();
    assert_eq!(decoded, expected);
}

#[test]
fn historical_naval_prefixes_remain_readable_without_result_sources() {
    let old = crate::format::FormatVersion::read(99).unwrap();
    assert!(old.table_refusal(TableId::NavalResultSources).is_some());
    for table in [
        TableId::NavalResults,
        TableId::NavalResultMembers,
        TableId::NavalResultCountries,
        TableId::NavalCombatResultDetails,
        TableId::SunkShips,
        TableId::SunkConvoys,
    ] {
        assert!(old.table_refusal(table).is_none(), "{table:?}");
    }
    assert!(
        crate::format::FormatVersion::CURRENT
            .table_refusal(TableId::NavalResultSources)
            .is_none()
    );
}
