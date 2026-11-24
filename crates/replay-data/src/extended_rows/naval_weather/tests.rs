use super::*;
use crate::rows::{decode_row, encode_row};

#[test]
fn regional_weather_map_terms_delta_without_repeating_quiet_siblings() {
    let previous = WeatherRegionRow {
        id: 17,
        map_modifiers_known: true.into(),
        naval_detection: Some(-33_333).into(),
        air_attack: Some(i64::MIN).into(),
        air_defence: Some(i64::MAX).into(),
        custom_sea: Some(vec![1, 0, i16::MIN]).into(),
        ..Default::default()
    };
    for (next, mask) in [
        (
            WeatherRegionRow {
                custom_sea: Some(vec![1, 1, i16::MIN]).into(),
                ..previous.clone()
            },
            1 << 15,
        ),
        (
            WeatherRegionRow {
                custom_sea: Some(Vec::new()).into(),
                ..previous.clone()
            },
            1 << 15,
        ),
        (
            WeatherRegionRow {
                custom_sea: None.into(),
                ..previous.clone()
            },
            1 << 15,
        ),
        (
            WeatherRegionRow {
                naval_detection: Some(-33_334).into(),
                ..previous.clone()
            },
            1 << 11,
        ),
        (
            WeatherRegionRow {
                air_attack: None.into(),
                ..previous.clone()
            },
            1 << 12,
        ),
        (
            WeatherRegionRow {
                air_defence: Some(0).into(),
                ..previous.clone()
            },
            1 << 13,
        ),
        (
            WeatherRegionRow {
                map_modifiers_known: false.into(),
                ..previous.clone()
            },
            1 << 10,
        ),
    ] {
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, mask);
        assert_eq!(
            delta
                .apply::<WeatherRegionRow>(Some(previous.clone()))
                .unwrap(),
            next
        );
    }
    let mut fields = Vec::new();
    previous.encode_fields(&mut fields).unwrap();
    assert_eq!(
        WeatherRegionRow::decode_fields(previous.key(), &mut Decoder::new(&fields)).unwrap(),
        previous
    );
}

#[test]
fn daylight_forecast_direction_preserves_held_state_and_independent_unavailability() {
    let held = WeatherDaylight {
        updated: false,
        held: Some([1, -2, i64::MIN]),
        held_feather: Some([-3, i64::MAX]),
    };
    assert_eq!(held.direction(60_000_000), Ok([1, -2, i64::MIN]));
    assert_eq!(held.direction(60_000_024), held.direction(60_000_000));
    assert!(WeatherDaylight::default().direction(60_000_000).is_err());
    let updated = WeatherDaylight {
        updated: true,
        held: None,
        held_feather: None,
    };
    assert_eq!(
        updated.direction(60_000_000),
        Ok(crate::day_night::seasonal_direction(60_000_000))
    );
    let row = WeatherProvinceRow {
        daylight: Recorded::Value(Some(held)),
        ..Default::default()
    };
    let mut fields = Vec::new();
    row.encode_fields(&mut fields).unwrap();
    assert_eq!(
        WeatherProvinceRow::decode_fields(row.key(), &mut Decoder::new(&fields)).unwrap(),
        row
    );
}

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
    let words = |temperature| crate::temperature::State {
        temperature,
        staged: temperature,
        ..Default::default()
    };
    let before = WeatherProvinceRow {
        id: 13_413,
        temperature: ProvinceTemperature::anchored(words(i64::MIN)),
        ..Default::default()
    };
    let after = WeatherProvinceRow {
        temperature: ProvinceTemperature::anchored(words(i64::MAX)),
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
    let held = |local_hour, night_factor| {
        DayNightClock::held(crate::day_night::Clock {
            local_hour,
            night_factor,
        })
    };
    let before = WeatherProvinceRow {
        id: 13_413,
        clock: held(i32::MIN, i64::MAX),
        ..Default::default()
    };
    for (hour, night) in [(0, 0), (i32::MAX, i64::MIN)] {
        let after = WeatherProvinceRow {
            clock: held(hour, night),
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
        assert_eq!(
            after.clock.state(after.id),
            Ok(ClockState::Held(crate::day_night::Clock {
                local_hour: hour,
                night_factor: night
            }))
        );
    }
}

#[test]
fn updated_clocks_store_nothing_that_changes_with_the_hour() {
    let held = WeatherProvinceRow {
        id: 4,
        clock: DayNightClock::held(crate::day_night::Clock {
            local_hour: 13,
            night_factor: 42_000,
        }),
        ..Default::default()
    };
    let updated = WeatherProvinceRow {
        clock: DayNightClock::UPDATED,
        ..held.clone()
    };
    assert_eq!(updated.clock.state(4), Ok(ClockState::Updated));
    let delta = updated.delta(Some(&held)).unwrap().unwrap();
    assert_eq!(delta.mask, (1 << 6) | (1 << 7) | (1 << 9));
    assert_eq!(
        WeatherProvinceRow::apply_delta(&delta, Some(held)).unwrap(),
        updated
    );
    assert_eq!(updated.delta(Some(&updated)).unwrap(), None);
    let mut bytes = Vec::new();
    updated.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        WeatherProvinceRow::decode_fields(updated.key(), &mut Decoder::new(&bytes)).unwrap(),
        updated
    );
}

#[test]
fn malformed_clocks_and_an_updated_slot_zero_are_refused() {
    let slot_zero = WeatherProvinceRow {
        id: 0,
        clock: DayNightClock::UPDATED,
        ..Default::default()
    };
    assert!(slot_zero.clock.state(0).is_err());
    assert_eq!(slot_zero.clock.state(1), Ok(ClockState::Updated));
    assert_eq!(WeatherProvinceRow::default().clock, DayNightClock::UPDATED);
    for (source, hour, night) in [(2, 0, 0), (0, 1, 0), (0, 0, 1)] {
        let clock = DayNightClock {
            local_hour: Recorded::Value(hour),
            night_factor: Recorded::Value(night),
            source: Recorded::Value(source),
        };
        assert!(clock.state(1).is_err(), "{source} {hour} {night}");
        let row = WeatherProvinceRow {
            id: 1,
            clock,
            ..Default::default()
        };
        assert!(row.delta(None).is_err(), "{source} {hour} {night}");
        assert!(encode_row(&row, 0, &mut Vec::new()).is_err());
    }
    let partial = DayNightClock {
        local_hour: Recorded::Value(1),
        night_factor: Recorded::NotRecorded,
        source: Recorded::NotRecorded,
    };
    assert!(partial.state(1).is_err());
}

#[test]
fn format_101_clocks_decode_as_held_recorded_values() {
    let table = TableId::WeatherProvinces;
    let historical =
        crate::format::FormatVersion::read(crate::rows::compatibility::POLITICAL_FORMAT).unwrap();
    assert_eq!(historical.fields(table), 9);
    assert_eq!(crate::format::FormatVersion::CURRENT.fields(table), 17);
    let row = WeatherProvinceRow {
        id: 9,
        clock: DayNightClock::held(crate::day_night::Clock {
            local_hour: 17,
            night_factor: 61_234,
        }),
        display: Recorded::Value(None),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut scan = Decoder::new(&bytes);
    for kind in &WeatherProvinceRow::SCHEMA[..9] {
        kind.decode(&mut scan).unwrap();
    }
    let mut input = Decoder::new(&bytes[..bytes.len() - scan.remaining()]);
    input.format = crate::rows::compatibility::POLITICAL_FORMAT;
    input.recorded_fields = 9;
    let decoded = WeatherProvinceRow::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.not_recorded_fields(), 0b11111111 << 9);
    assert_eq!(
        decoded.temperature.record(),
        Ok(TemperatureRecord::AtHour(0))
    );
    assert_eq!(
        decoded.clock.state(9),
        Ok(ClockState::Held(crate::day_night::Clock {
            local_hour: 17,
            night_factor: 61_234
        }))
    );
    assert_eq!(
        decoded.clock,
        DayNightClock::historical(crate::day_night::Clock {
            local_hour: 17,
            night_factor: 61_234
        })
    );
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
            ..original
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
        ..original
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
fn naval_result_sources_roundtrip_independent_sources_exactly() {
    let rows: Vec<_> = (0..6_u64)
        .map(|id| NavalResultSourceRow {
            id,
            game_hour: 60_827_795,
            result: (id + 1) << 32 | 0x8123,
            combat: (id + 7) << 32 | 0x8123,
            merged: id.is_multiple_of(3),
            transient: id.is_multiple_of(2) || id.is_multiple_of(3),
            combat_created_hour: Some(60_827_795),
        })
        .collect();
    let expected = TableRows::NavalResultSources(rows);
    let mut bytes = Vec::new();
    expected.encode_rows(&mut bytes).unwrap();
    assert_eq!(bytes.len(), 6 * 28);
    assert_eq!(TableId::NavalResultSources.minimum_row_bytes(), 24);
    let mut input = Decoder::new(&bytes);
    let decoded = TableId::NavalResultSources
        .decode_rows(6, &mut input)
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

#[test]
fn temperature_words_store_staged_offsets_and_refuse_partial_records() {
    let state = crate::temperature::State {
        temperature: i64::MAX,
        drift: -600_000,
        staged: i64::MIN,
        working: -600_000,
    };
    let anchored = ProvinceTemperature::anchored(state);
    assert_eq!(anchored.record(), Ok(TemperatureRecord::Anchored(state)));
    let row = WeatherProvinceRow {
        temperature: anchored,
        ..Default::default()
    };
    let mut fields = Vec::new();
    row.encode_fields(&mut fields).unwrap();
    assert_eq!(
        WeatherProvinceRow::decode_fields(row.key(), &mut Decoder::new(&fields)).unwrap(),
        row
    );
    assert_eq!(
        ProvinceTemperature::historical(7).record(),
        Ok(TemperatureRecord::AtHour(7))
    );
    assert_eq!(
        ProvinceTemperature::NOT_RECORDED.record(),
        Ok(TemperatureRecord::NotRecorded)
    );
    let partial = ProvinceTemperature {
        drift: Recorded::NotRecorded,
        ..anchored
    };
    assert!(partial.record().is_err());
    assert!(
        WeatherProvinceRow {
            temperature: partial,
            ..Default::default()
        }
        .encode_fields(&mut Vec::new())
        .is_err()
    );
}

#[test]
fn the_temperature_pass_rides_slot_zero_with_its_witness_last_before_the_modifiers() {
    let pass = TemperaturePass {
        anchor: 43_808_760,
        cursor: 583,
        anchor_seed: Some(9),
        hours: vec![Some(u32::MAX), None],
        witness: TemperatureWitness::new(0x0123_4567_89ab_cdef),
    };
    let row = WeatherProvinceRow {
        temperature_pass: Recorded::Value(Some(pass.clone())),
        ..Default::default()
    };
    let mut fields = Vec::new();
    row.encode_fields(&mut fields).unwrap();
    let mut tail = Vec::new();
    crate::rows::NativeValue::field(&row.modifiers)
        .encode(&mut tail)
        .unwrap();
    crate::rows::NativeValue::field(&row.daylight)
        .encode(&mut tail)
        .unwrap();
    crate::rows::NativeValue::field(&row.static_modifiers)
        .encode(&mut tail)
        .unwrap();
    let pass_end = fields.len() - tail.len();
    assert!(fields[..pass_end].ends_with(&0x0123_4567_89ab_cdef_u64.to_le_bytes()));
    assert_eq!(&fields[pass_end..], tail.as_slice());
    let decoded = WeatherProvinceRow::decode_fields(row.key(), &mut Decoder::new(&fields)).unwrap();
    assert_eq!(
        decoded.temperature_pass,
        Recorded::Value(Some(pass.clone()))
    );
    let mut older = Decoder::new(&fields);
    older.format = crate::rows::compatibility::PROVINCE_WITNESS_FORMAT - 1;
    let decoded = WeatherProvinceRow::decode_fields(row.key(), &mut older).unwrap();
    assert_eq!(
        decoded.temperature_pass,
        Recorded::Value(Some(TemperaturePass {
            witness: TemperatureWitness::default(),
            ..pass
        }))
    );
    assert_eq!(decoded.modifiers, row.modifiers);
    let unwitnessed = WeatherProvinceRow {
        temperature_pass: Recorded::Value(Some(TemperaturePass::default())),
        ..Default::default()
    };
    assert!(unwitnessed.encode_fields(&mut Vec::new()).is_err());
}

#[test]
fn checkpoints_keep_an_older_pass_without_its_witness() {
    use crate::checkpoint::values::CheckpointValue;
    let witnessed = TemperaturePass {
        anchor: 7,
        cursor: 2,
        anchor_seed: Some(3),
        hours: vec![],
        witness: TemperatureWitness::new(u64::MAX),
    };
    let older = TemperaturePass {
        witness: TemperatureWitness::default(),
        ..witnessed.clone()
    };
    for pass in [witnessed, older.clone()] {
        let mut bytes = Vec::new();
        CheckpointValue::encode(&Some(pass.clone()), &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let restored = <Option<TemperaturePass> as CheckpointValue>::decode(&mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(restored, Some(pass));
    }
    let rows = [WeatherProvinceRow {
        temperature: ProvinceTemperature::anchored(crate::temperature::State::default()),
        temperature_pass: Recorded::Value(Some(older)),
        ..Default::default()
    }];
    let reason = crate::temperature::Memo::default()
        .states(1, 7, &rows, Err("no content"))
        .unwrap_err();
    assert!(reason.contains("predates the province temperature witness"), "{reason}");
}

#[test]
fn a_province_accumulator_keeps_every_entry() {
    let modifiers = ProvinceWeatherModifiers {
        entries: vec![
            ProvinceModifierEntry {
                key: 0x2b,
                value: -40_000,
            },
            ProvinceModifierEntry {
                key: 0x4f,
                value: 0,
            },
            ProvinceModifierEntry {
                key: 0x1a0,
                value: i64::MIN,
            },
        ],
    };
    let row = WeatherProvinceRow {
        id: 7,
        modifiers: Recorded::Value(Some(modifiers.clone())),
        ..Default::default()
    };
    let mut fields = Vec::new();
    row.encode_fields(&mut fields).unwrap();
    let decoded = WeatherProvinceRow::decode_fields(row.key(), &mut Decoder::new(&fields)).unwrap();
    assert_eq!(decoded.modifiers, Recorded::Value(Some(modifiers.clone())));
    assert_eq!(modifiers.get(0x2b), Some(-40_000));
    assert_eq!(modifiers.get(0x4f), Some(0));
    assert_eq!(modifiers.get(0x50), None);
}

#[test]
fn a_province_static_modifier_list_keeps_order_days_and_refusal() {
    let list = ProvinceStaticModifiers {
        entries: vec![
            ProvinceStaticModifier {
                name: b"the_great_wall".to_vec(),
                days: -1,
            },
            ProvinceStaticModifier {
                name: b"flooded".to_vec(),
                days: 30,
            },
            ProvinceStaticModifier {
                name: Vec::new(),
                days: 0,
            },
        ],
    };
    for value in [Some(list), Some(ProvinceStaticModifiers::default()), None] {
        let row = WeatherProvinceRow {
            id: 7,
            static_modifiers: Recorded::Value(value.clone()),
            ..Default::default()
        };
        let mut fields = Vec::new();
        row.encode_fields(&mut fields).unwrap();
        let decoded =
            WeatherProvinceRow::decode_fields(row.key(), &mut Decoder::new(&fields)).unwrap();
        assert_eq!(decoded.static_modifiers, Recorded::Value(value.clone()));
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(delta.apply::<WeatherProvinceRow>(None).unwrap(), decoded);
    }
}

#[test]
fn a_province_accumulator_with_duplicate_unordered_or_zero_keys_is_refused() {
    let entry = |key, value| ProvinceModifierEntry { key, value };
    let row = |entries| WeatherProvinceRow {
        id: 7,
        modifiers: Recorded::Value(Some(ProvinceWeatherModifiers { entries })),
        ..Default::default()
    };
    let valid = row(vec![entry(0x2b, -40_000), entry(0x4f, 0)]);
    assert!(valid.check_key_fields().is_ok());
    for entries in [
        vec![entry(0x2b, 1), entry(0x2b, -40_000)],
        vec![entry(0x4f, 0), entry(0x2b, -40_000)],
        vec![entry(0, 5)],
    ] {
        let malformed = row(entries.clone());
        assert!(malformed.check_key_fields().is_err(), "{entries:?}");
        assert!(
            encode_row(&malformed, 0, &mut Vec::new()).is_err(),
            "{entries:?}"
        );
        assert!(malformed.delta(None).is_err(), "{entries:?}");
        assert!(valid.delta(Some(&malformed)).is_err(), "{entries:?}");
    }
}
