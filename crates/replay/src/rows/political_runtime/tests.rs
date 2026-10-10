use super::*;
use crate::rows::compatibility::EFFECT_INPUTS_FORMAT;

#[test]
fn historical_weather_keeps_its_prefix_without_inventing_a_stored_clock() {
    let table = TableId::WeatherProvinces;
    let row = WeatherProvinceRow {
        id: 7,
        snow: i64::MIN,
        permanent_snow: i64::MAX,
        display_snow: 255,
        display_mud: 254,
        temperature: Recorded::Value(-1),
        raw_mud: Recorded::Value(253),
        local_hour: Recorded::Value(0),
        night_factor: Recorded::Value(100_000),
        display: Recorded::Value(None),
    };
    // Independently authored published six-field snapshot: fixed-width little
    // endian snow, permanent snow, two display bytes, temperature and raw mud.
    let mut prefix = Vec::new();
    prefix.extend_from_slice(&i64::MIN.to_le_bytes());
    prefix.extend_from_slice(&i64::MAX.to_le_bytes());
    prefix.extend_from_slice(&[255, 254]);
    prefix.extend_from_slice(&(-1_i64).to_le_bytes());
    prefix.push(253);
    let mut current = Vec::new();
    row.encode_fields(&mut current).unwrap();
    assert!(current.starts_with(&prefix));
    let expected = WeatherProvinceRow {
        local_hour: Recorded::NotRecorded,
        night_factor: Recorded::NotRecorded,
        display: Recorded::NotRecorded,
        ..row.clone()
    };
    for version in [93, EFFECT_INPUTS_FORMAT] {
        let old_format = FormatVersion::read(version).unwrap();
        assert_eq!(old_format.fields(table), 6);
        let mut input = Decoder::new(&prefix);
        input.format = version;
        input.recorded_fields = 6;
        let historical = WeatherProvinceRow::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(historical, expected);
        assert_eq!(
            historical.not_recorded_fields(),
            (1 << 6) | (1 << 7) | (1 << 8)
        );
        assert!(historical.encode_fields(&mut Vec::new()).is_err());
        assert!(historical.delta(None).is_err());
    }

    let old_format = FormatVersion::read(EFFECT_INPUTS_FORMAT).unwrap();
    let fields = old_format.fields(table);
    let delta = row.delta(None).unwrap().unwrap();
    let mut scan = Decoder::new(&delta.values);
    for kind in &WeatherProvinceRow::SCHEMA[..fields] {
        kind.decode_change(&mut scan).unwrap();
    }
    let mut bytes = Vec::new();
    table.encode_key(row.key(), 0, &mut bytes).unwrap();
    bytes.extend_from_slice(&full_mask(fields).to_le_bytes()[..fields.div_ceil(8)]);
    bytes.extend_from_slice(&delta.values[..delta.values.len() - scan.remaining()]);
    let mut input = Decoder::new(&bytes);
    input.format = old_format.version();
    input.recorded_fields = fields;
    let historical_delta = decode_delta(table, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(
        historical_delta.apply::<WeatherProvinceRow>(None).unwrap(),
        expected
    );
}

#[test]
fn province_attack_keeps_legacy_bytes_and_distinguishes_exact_zero_from_absence() {
    let table = TableId::Provinces;
    let row = ProvinceRow {
        id: 7,
        state: 9,
        owner: 2,
        controller: 0,
        victory_points: i32::MIN,
        fort_positive: 0,
        move_gate: u8::MAX,
        port_sea: i32::MIN,
        attack: ProvinceAttackInputs {
            sample_hour: -17,
            fort_sum: Some(0),
            land_bunker_effectiveness_factor: None,
        }
        .into(),
        selected_areas: Default::default(),
    };
    // Independently authored seven-field wire prefix from the published row.
    let mut prefix = Vec::new();
    for value in [9_u32, 2, 0] {
        prefix.extend_from_slice(&value.to_le_bytes());
    }
    prefix.extend_from_slice(&i32::MIN.to_le_bytes());
    prefix.extend_from_slice(&[0, u8::MAX]);
    prefix.extend_from_slice(&i32::MIN.to_le_bytes());
    let format = FormatVersion::read(EFFECT_INPUTS_FORMAT).unwrap();
    assert_eq!(format.fields(table), 7);
    let mut input = Decoder::new(&prefix);
    input.format = format.version();
    input.recorded_fields = 7;
    let historical = ProvinceRow::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(
        historical,
        ProvinceRow {
            attack: Recorded::NotRecorded,
            selected_areas: Recorded::NotRecorded,
            ..row.clone()
        }
    );
    assert_eq!(historical.recorded_fields_bytes().unwrap(), prefix.len());
    assert_eq!(historical.not_recorded_fields(), (1 << 7) | (1 << 8));
    assert!(historical.encode_fields(&mut Vec::new()).is_err());

    let mut current = Vec::new();
    row.encode_fields(&mut current).unwrap();
    assert!(current.starts_with(&prefix));
    let mut input = Decoder::new(&current);
    assert_eq!(
        ProvinceRow::decode_fields(row.key(), &mut input).unwrap(),
        row
    );
    input.finish().unwrap();
    let refused = ProvinceRow {
        attack: ProvinceAttackInputs {
            fort_sum: None,
            ..*row.attack.require("attack").unwrap()
        }
        .into(),
        ..row.clone()
    };
    let delta = refused.delta(Some(&row)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 7);
    assert_eq!(delta.apply(Some(row)).unwrap(), refused);
}

#[test]
fn raw_country_observed_zero_survives_current_codec_and_is_missing_in_published_prefix() {
    let row = ReservedCountryNamingRow {
        raw_country_observations: Some(RawCountryObservations {
            sample_hour: -17,
            entries: vec![RawCountryObservation {
                raw: 777,
                mapping: RawCountryMapping::Observed {
                    canonical_slot: 0,
                    country_id: 0,
                },
                tag: RawCountryTag::Bytes(vec![0, 0xff]),
            }],
        })
        .into(),
        ..Default::default()
    };
    let mut current = Vec::new();
    row.encode_fields(&mut current).unwrap();
    let mut input = Decoder::new(&current);
    let decoded = ReservedCountryNamingRow::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded, row);
    let packet = decoded
        .raw_country_observations
        .require("raw country")
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(
        packet.entry(-17, 777).unwrap().mapping,
        RawCountryMapping::Observed {
            canonical_slot: 0,
            country_id: 0
        }
    );
    assert!(packet.entry(-17, 778).is_err());
    assert!(packet.entry(-16, 777).is_err());

    let format = FormatVersion::read(EFFECT_INPUTS_FORMAT).unwrap();
    let fields = format.fields(TableId::ReservedCountryNaming);
    assert_eq!(fields, 13);
    let mut scan = Decoder::new(&current);
    for kind in &ReservedCountryNamingRow::SCHEMA[..fields] {
        kind.decode(&mut scan).unwrap();
    }
    let mut input = Decoder::new(&current[..current.len() - scan.remaining()]);
    input.format = format.version();
    input.recorded_fields = fields;
    let historical = ReservedCountryNamingRow::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(historical.raw_country_observations, Recorded::NotRecorded);
    assert_eq!(historical.attack, Recorded::NotRecorded);
    assert!(historical.encode_fields(&mut Vec::new()).is_err());
}

#[test]
fn selected_routes_published_prefixes_never_invent_current_inputs_in_snapshots_or_deltas() {
    fn historical<T: RowCodec + Fields + std::fmt::Debug>(
        table: TableId,
        fields: usize,
        row: &T,
        expected: &T,
    ) {
        let format = FormatVersion::read(EFFECT_INPUTS_FORMAT).unwrap();
        assert_eq!(format.fields(table), fields);
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut scan = Decoder::new(&bytes);
        for kind in &T::SCHEMA[..fields] {
            kind.decode(&mut scan).unwrap();
        }
        let prefix = &bytes[..bytes.len() - scan.remaining()];
        let mut input = Decoder::new(prefix);
        input.format = format.version();
        input.recorded_fields = fields;
        let decoded = T::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(&decoded, expected);
        assert_eq!(decoded.recorded_fields_bytes().unwrap(), prefix.len());
        assert_eq!(
            decoded.not_recorded_fields(),
            full_mask(T::SCHEMA.len()) ^ full_mask(fields)
        );
        assert!(decoded.encode_fields(&mut Vec::new()).is_err());
        assert!(decoded.delta(None).is_err());

        let current = row.delta(None).unwrap().unwrap();
        let mut scan = Decoder::new(&current.values);
        for kind in &T::SCHEMA[..fields] {
            kind.decode_change(&mut scan).unwrap();
        }
        bytes.clear();
        table.encode_key(row.key(), 0, &mut bytes).unwrap();
        bytes.extend_from_slice(&full_mask(fields).to_le_bytes()[..fields.div_ceil(8)]);
        bytes.extend_from_slice(&current.values[..current.values.len() - scan.remaining()]);
        let mut input = Decoder::new(&bytes);
        input.format = format.version();
        input.recorded_fields = fields;
        let delta = decode_delta(table, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(&delta.apply::<T>(None).unwrap(), expected);
    }

    let province = ProvinceRow {
        id: 17,
        selected_areas: Some(ProvinceAreaInputs {
            sample_hour: i32::MAX,
            controller: 17,
            owner: Some(0),
            node: Some(SelectedAreaNode {
                first_land: Some(17),
                nonsea_index: Some(0),
                nonsea_count: Some(1),
                land_count: Some(1),
                neighbours: Some(vec![29, 29]),
            }),
        })
        .into(),
        ..Default::default()
    };
    historical(
        TableId::Provinces,
        7,
        &province,
        &ProvinceRow {
            attack: Recorded::NotRecorded,
            selected_areas: Recorded::NotRecorded,
            ..province.clone()
        },
    );

    let movement = DivisionMovementRow {
        division: u64::MAX,
        selected: Some(SelectedUnitInputs {
            sample_hour: i32::MIN,
            exiled: Some(u8::MAX),
            commander: Some(false),
            retreat: [Some(0), Some(0)],
        })
        .into(),
        ..Default::default()
    };
    historical(
        TableId::DivisionMovement,
        15,
        &movement,
        &DivisionMovementRow {
            withdrawal: Recorded::NotRecorded,
            movement_priority: Recorded::NotRecorded,
            order_transport: Recorded::NotRecorded,
            fresh: Recorded::NotRecorded,
            combat_references: Recorded::NotRecorded,
            selected: Recorded::NotRecorded,
            transport: Recorded::NotRecorded,
            ..movement.clone()
        },
    );

    let wars = CountryWarRow {
        country: u32::MAX,
        can_occupy_non_war: 0xfe,
        enemies_read: 1,
        enemies: vec![29, 11, 29],
        ordered: Some(OrderedEnemyInputs {
            sample_hour: i32::MAX,
            entries: vec![29, 0, 11, 29],
        })
        .into(),
    };
    // Independent published bytes: the two raw flags, then the native u32 list.
    let mut published = vec![0xfe, 1];
    for word in [3_u32, 29, 11, 29] {
        published.extend_from_slice(&word.to_le_bytes());
    }
    let mut current = Vec::new();
    wars.encode_fields(&mut current).unwrap();
    assert!(current.starts_with(&published));
    historical(
        TableId::CountryWars,
        3,
        &wars,
        &CountryWarRow {
            ordered: Recorded::NotRecorded,
            ..wars.clone()
        },
    );
}
