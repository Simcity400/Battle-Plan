use super::*;
use crate::rows::compatibility::{EFFECT_INPUTS_FORMAT, POLITICAL_FORMAT};

#[test]
fn historical_weather_keeps_its_prefix_without_inventing_a_stored_clock() {
    let table = TableId::WeatherProvinces;
    let row = WeatherProvinceRow {
        daylight: Recorded::Value(None),
        id: 7,
        snow: i64::MIN,
        permanent_snow: i64::MAX,
        display_snow: 255,
        display_mud: 254,
        temperature: ProvinceTemperature::anchored(crate::temperature::State {
            temperature: -1,
            drift: 5,
            staged: -1,
            working: 5,
        }),
        raw_mud: Recorded::Value(253),
        clock: DayNightClock::held(crate::day_night::Clock {
            local_hour: 0,
            night_factor: 100_000,
        }),
        display: Recorded::Value(None),
        temperature_pass: Recorded::Value(None),
        modifiers: Recorded::Value(None),
        static_modifiers: Recorded::Value(None),
    };
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
        temperature: ProvinceTemperature::historical(-1),
        clock: DayNightClock::NOT_RECORDED,
        display: Recorded::NotRecorded,
        temperature_pass: Recorded::NotRecorded,
        modifiers: Recorded::NotRecorded,
        daylight: Recorded::NotRecorded,
        static_modifiers: Recorded::NotRecorded,
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
        assert_eq!(historical.not_recorded_fields(), 0x7ff << 6);
        assert_eq!(
            historical.temperature.record(),
            Ok(TemperatureRecord::AtHour(-1))
        );
        assert_eq!(historical.clock.state(7), Ok(ClockState::NotRecorded));
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

fn with_payloads(row: &[u8], schema: &[Kind], payloads: &[(usize, &[u8])]) -> Vec<u8> {
    let mut input = Decoder::new(row);
    let mut out = Vec::new();
    for (index, kind) in schema.iter().enumerate() {
        let start = row.len() - input.remaining();
        kind.decode(&mut input).unwrap();
        let end = row.len() - input.remaining();
        match payloads.iter().find(|(at, _)| *at == index) {
            Some((_, payload)) => {
                out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                out.extend_from_slice(payload);
            }
            None => out.extend_from_slice(&row[start..end]),
        }
    }
    input.finish().unwrap();
    out
}

#[test]
fn province_attack_operands_of_format_101_decode_and_current_rows_store_none() {
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
        attack: Default::default(),
        selected_areas: Default::default(),
        fort_bunker: Recorded::Value(None),
        controller_area: Default::default(),
        owner_area: Default::default(),
        area_node: Default::default(),
    };
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
            fort_bunker: Recorded::NotRecorded,
            controller_area: Recorded::NotRecorded,
            owner_area: Recorded::NotRecorded,
            area_node: Recorded::NotRecorded,
            ..row
        }
    );
    assert_eq!(historical.not_recorded_fields(), 0b1_1111_1000_0000);

    let mut current = Vec::new();
    row.encode_fields(&mut current).unwrap();
    assert!(current.starts_with(&prefix));
    assert_eq!(
        &current[prefix.len()..prefix.len() + 4],
        &0_u32.to_le_bytes()
    );
    let mut input = Decoder::new(&current);
    assert_eq!(
        ProvinceRow::decode_fields(row.key(), &mut input).unwrap(),
        row
    );
    input.finish().unwrap();

    let mut payload = 1000_i32.to_le_bytes().to_vec();
    payload.extend_from_slice(&[1, 3, 0, 0, 0, 1]);
    payload.extend_from_slice(&(-25_000_i64).to_le_bytes());
    let full = with_payloads(&current, ProvinceRow::SCHEMA, &[(7, &payload)]);
    let format = FormatVersion::read(POLITICAL_FORMAT).unwrap();
    assert_eq!(format.fields(table), 9);
    let mut scan = Decoder::new(&full);
    for kind in &ProvinceRow::SCHEMA[..format.fields(table)] {
        kind.decode(&mut scan).unwrap();
    }
    let legacy = full[..full.len() - scan.remaining()].to_vec();
    let mut input = Decoder::new(&legacy);
    input.format = format.version();
    input.recorded_fields = format.fields(table);
    assert_eq!(
        ProvinceRow::decode_fields(row.key(), &mut input).unwrap(),
        ProvinceRow {
            attack: Recorded::Value(Witnessed(Some(ProvinceAttackInputs {
                sample_hour: PacketHour::Stamped(1000),
                fort_sum: Some(3),
                land_bunker_effectiveness_factor: Some(-25_000),
            }))),
            fort_bunker: Recorded::NotRecorded,
            controller_area: Recorded::NotRecorded,
            owner_area: Recorded::NotRecorded,
            area_node: Recorded::NotRecorded,
            ..row.clone()
        }
    );
    input.finish().unwrap();
    assert!(ProvinceRow::decode_fields(row.key(), &mut Decoder::new(&legacy)).is_err());
}

#[test]
fn retired_reserved_packets_are_discarded_and_later_fields_still_decode() {
    let row = ReservedCountryNamingRow {
        tag: b"RSV".to_vec(),
        faction: Recorded::Value(Some(0)),
        land_theatre_count: Recorded::Value(Some(u32::MAX)),
        ..Default::default()
    };
    let mut current = Vec::new();
    row.encode_fields(&mut current).unwrap();
    let mut input = Decoder::new(&current);
    assert_eq!(
        ReservedCountryNamingRow::decode_fields(row.key(), &mut input).unwrap(),
        row
    );
    input.finish().unwrap();
    let mut attack = vec![1];
    attack.extend_from_slice(&1000_i32.to_le_bytes());
    attack.push(1);
    attack.extend_from_slice(&(-20_000_i64).to_le_bytes());
    attack.push(0);
    let legacy = with_payloads(
        &current,
        ReservedCountryNamingRow::SCHEMA,
        &[
            (15, &[1, 2, 3]),
            (20, &attack),
            (21, &[1, 9]),
            (22, &[1, 0, 0, 0, 0, 1, 7]),
        ],
    );
    let format = FormatVersion::read(POLITICAL_FORMAT).unwrap();
    let mut input = Decoder::new(&legacy);
    input.format = format.version();
    input.recorded_fields = format.fields(TableId::ReservedCountryNaming);
    let decoded = ReservedCountryNamingRow::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.tag, row.tag);
    assert_eq!(decoded.faction, row.faction);
    assert_eq!(decoded.land_theatre_count, row.land_theatre_count);
    for field in [
        &decoded.movement,
        &decoded.raw_country_observations,
        &decoded.transport_mission_inputs,
    ] {
        assert_eq!(*field, Recorded::Value(Retired));
    }
    assert_eq!(
        decoded.attack,
        Recorded::Value(Witnessed(Some(Some(CountryAttackInputs {
            sample_hour: PacketHour::Stamped(1000),
            land_night_attack: Some(-20_000),
            land_bunker_effectiveness_factor: None,
        }))))
    );
}

fn decode_format_101_payloads<T: RowCodec + Fields + std::fmt::Debug>(
    table: TableId,
    row: &T,
    payloads: &[(usize, &[u8])],
) -> T {
    let mut current = Vec::new();
    row.encode_fields(&mut current).unwrap();
    for &(index, _) in payloads {
        assert!(
            compatibility::RETIRED_FIELDS
                .iter()
                .any(|field| field.table == table && field.index == index),
            "{table:?} field {index} is not catalogued as retired"
        );
    }
    let full = with_payloads(&current, T::SCHEMA, payloads);
    assert!(T::decode_fields(row.key(), &mut Decoder::new(&full)).is_err());
    let format = FormatVersion::read(POLITICAL_FORMAT).unwrap();
    let fields = format.fields(table);
    let mut scan = Decoder::new(&full);
    for kind in &T::SCHEMA[..fields] {
        kind.decode(&mut scan).unwrap();
    }
    let legacy = &full[..full.len() - scan.remaining()];
    let mut input = Decoder::new(legacy);
    input.format = format.version();
    input.recorded_fields = fields;
    let decoded = T::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    decoded
}

#[test]
fn retired_previews_discard_supported_payloads_and_current_country_keeps_tombstone() {
    let province = decode_format_101_payloads(
        TableId::Provinces,
        &ProvinceRow {
            id: 17,
            controller: 3,
            ..Default::default()
        },
        &[(8, &[1, 17, 0, 0, 0, 1, 3, 0, 0, 0, 0])],
    );
    assert_eq!(province.controller, 3);
    assert_eq!(province.selected_areas, Recorded::Value(Retired));

    let appearance = decode_format_101_payloads(
        TableId::CountryAppearance,
        &CountryAppearanceRow {
            country: 7,
            red: 1,
            ..Default::default()
        },
        &[(21, &[1, 0, 0, 0, 0, 1, 2, 0, 0, 0])],
    );
    assert_eq!(appearance.red, 1);
    assert_eq!(appearance.transport_convoys, Recorded::Value(Retired));

    let counters = CountryCounterInputRow {
        country: 7,
        first_division: u64::MAX,
        air_combat_modifiers_known: true.into(),
        air_attack_factor: Some(i64::MIN).into(),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    counters.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        CountryCounterInputRow::decode_fields(counters.key(), &mut input).unwrap(),
        counters
    );
    input.finish().unwrap();
    assert_eq!(counters.movement, Recorded::Value(Retired));

    let gun = decode_format_101_payloads(
        TableId::RailwayGunCounterInputs,
        &RailwayGunCounterInputRow {
            gun: u64::MAX,
            province: 7,
            ..Default::default()
        },
        &[(29, &[1, 0, 0, 0, 0, 1, 2])],
    );
    assert_eq!(gun.province, 7);
    assert_eq!(gun.transport, Recorded::Value(Retired));

    let registration = decode_format_101_payloads(
        TableId::AirBaseRegistrations,
        &AirBaseRegistrationRow {
            base: 7,
            slot: 1,
            capacity_raw: -1,
            ..Default::default()
        },
        &[(3, &[1, 1, 0, 0, 0, 1, 3, 0, 0, 0])],
    );
    assert_eq!(registration.capacity_raw, -1);
    assert_eq!(registration.transport_planes, Recorded::Value(Retired));
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
        ..Default::default()
    };
    historical(
        TableId::Provinces,
        7,
        &province,
        &ProvinceRow {
            attack: Recorded::NotRecorded,
            selected_areas: Recorded::NotRecorded,
            fort_bunker: Recorded::NotRecorded,
            controller_area: Recorded::NotRecorded,
            owner_area: Recorded::NotRecorded,
            area_node: Recorded::NotRecorded,
            ..province.clone()
        },
    );

    let wars = CountryWarRow {
        country: u32::MAX,
        can_occupy_non_war: 0xfe,
        enemies_read: 1,
        enemies: vec![29, 11, 29],
        ordered: Some(OrderedEnemyInputs {
            sample_hour: PacketHour::Host,
            entries: vec![29, 0, 11, 29],
        })
        .into(),
    };
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
