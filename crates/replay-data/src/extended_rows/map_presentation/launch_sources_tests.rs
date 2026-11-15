use super::*;
use crate::format::FormatVersion;

fn ordered_sources() -> ReservedLaunchSources {
    ReservedLaunchSources {
        sample_hour: PacketHour::Host,
        naval_bases: LaunchNavalBases::Bases(vec![
            LaunchNavalBase {
                base_province: 9,
                country: 0,
            },
            LaunchNavalBase {
                base_province: i32::MIN as u32,
                country: u32::MAX,
            },
            LaunchNavalBase {
                base_province: 0,
                country: 7,
            },
            LaunchNavalBase {
                base_province: 9,
                country: 0,
            },
        ]),
        owned_states: Some(vec![19, 0, u32::MAX, 19, i32::MIN as u32]),
    }
}

#[test]
fn reserved_launch_snapshot_keeps_order_repeats_signed_bits_and_independent_domains() {
    let sources = ordered_sources();
    for packet in [
        None,
        Some(ReservedLaunchSources {
            sample_hour: PacketHour::Host,
            naval_bases: LaunchNavalBases::Unavailable,
            owned_states: None,
        }),
        Some(ReservedLaunchSources {
            naval_bases: LaunchNavalBases::Bases(Vec::new()),
            owned_states: Some(Vec::new()),
            ..sources.clone()
        }),
        Some(ReservedLaunchSources {
            naval_bases: LaunchNavalBases::Unavailable,
            ..sources.clone()
        }),
        Some(ReservedLaunchSources {
            owned_states: None,
            ..sources.clone()
        }),
        Some(ReservedLaunchSources {
            naval_bases: LaunchNavalBases::NativeNull,
            ..sources.clone()
        }),
        Some(sources),
    ] {
        let row = ReservedCountryNamingRow {
            tag: vec![0xff, 0, b'R'],
            owned_state_count: Some(5).into(),
            launch_sources: packet.into(),
            ..Default::default()
        };
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            decode_row::<ReservedCountryNamingRow>(0, &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
        assert_eq!(row.recorded_fields_bytes().unwrap() + 1, bytes.len());
    }
}

#[test]
fn reserved_launch_provider_keeps_previous_list_bytes_and_has_a_strict_null_tag() {
    for legacy in [
        None,
        Some(Vec::new()),
        Some(vec![LaunchNavalBase {
            base_province: u32::MAX,
            country: 7,
        }]),
    ] {
        let provider = match &legacy {
            None => LaunchNavalBases::Unavailable,
            Some(bases) => LaunchNavalBases::Bases(bases.clone()),
        };
        let mut previous = Vec::new();
        legacy.encode(&mut previous).unwrap();
        let mut current = Vec::new();
        provider.encode(&mut current).unwrap();
        assert_eq!(current, previous);
        let mut input = Decoder::new(&previous);
        assert_eq!(LaunchNavalBases::decode(&mut input).unwrap(), provider);
        input.finish().unwrap();
    }
    let mut bytes = Vec::new();
    LaunchNavalBases::NativeNull.encode(&mut bytes).unwrap();
    assert_eq!(bytes, [2]);
    assert_eq!(
        Value::encoded_bytes(&LaunchNavalBases::NativeNull).unwrap(),
        1
    );
    assert_eq!(
        LaunchNavalBases::NativeNull.try_clone().unwrap(),
        LaunchNavalBases::NativeNull
    );
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        LaunchNavalBases::decode(&mut input).unwrap(),
        LaunchNavalBases::NativeNull
    );
    input.finish().unwrap();
    for tag in [3, u8::MAX] {
        assert!(LaunchNavalBases::decode(&mut Decoder::new(&[tag])).is_err());
    }
}

#[test]
fn reserved_launch_delta_clears_and_reintroduces_only_its_suffix_with_three_byte_mask() {
    let before = ReservedCountryNamingRow {
        tag: b"ZERO".to_vec(),
        faction: Some(7).into(),
        owned_state_count: Some(5).into(),
        land_theatre_count: Some(0).into(),
        launch_sources: Some(ordered_sources()).into(),
        ..Default::default()
    };
    for packet in [
        None,
        Some(ReservedLaunchSources {
            owned_states: None,
            ..ordered_sources()
        }),
        Some(ReservedLaunchSources {
            naval_bases: LaunchNavalBases::NativeNull,
            ..ordered_sources()
        }),
        Some(ReservedLaunchSources {
            naval_bases: LaunchNavalBases::Unavailable,
            ..ordered_sources()
        }),
        Some(ReservedLaunchSources {
            naval_bases: LaunchNavalBases::Bases(Vec::new()),
            owned_states: Some(Vec::new()),
            ..ordered_sources()
        }),
    ] {
        let after = ReservedCountryNamingRow {
            launch_sources: packet.into(),
            ..before.clone()
        };
        for (old, new) in [(&before, &after), (&after, &before)] {
            let delta = new.delta(Some(old)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 17);
            let mut bytes = Vec::new();
            encode_delta(TableId::ReservedCountryNaming, &delta, 0, &mut bytes).unwrap();
            assert_eq!(&bytes[..4], &[0, 0, 0, 2]); // Singleton key, then current field mask.
            let mut input = Decoder::new(&bytes);
            let decoded = decode_delta(TableId::ReservedCountryNaming, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(decoded.apply(Some(old.clone())).unwrap(), *new);
        }
        let inserted = after.delta(None).unwrap().unwrap();
        assert_eq!(
            inserted.mask,
            full_mask(ReservedCountryNamingRow::SCHEMA.len())
        );
        assert_eq!(
            inserted.apply::<ReservedCountryNamingRow>(None).unwrap(),
            after
        );
        assert!(after.delta(Some(&after)).unwrap().is_none());
    }
}

#[test]
fn reserved_launch_historical_snapshots_and_insert_deltas_leave_named_gap() {
    let table = TableId::ReservedCountryNaming;
    let field = table
        .last_change()
        .appended
        .iter()
        .find(|field| field.name == "launch_sources")
        .expect("launch sources has a compatibility entry");
    assert_eq!(field.index, 17);
    let format = FormatVersion::read(field.introduced - 1).unwrap();
    let fields = format.fields(table);
    assert!(fields <= field.index);
    let row = ReservedCountryNamingRow {
        tag: vec![0xff, 0, b'R'],
        built: 7,
        country_array: Some(vec![0, 7, 0, 7]).into(),
        owned_state_count: Some(i32::MIN).into(),
        launch_sources: Some(ReservedLaunchSources {
            naval_bases: LaunchNavalBases::NativeNull,
            ..ordered_sources()
        })
        .into(),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &ReservedCountryNamingRow::SCHEMA[..fields] {
        kind.decode(&mut input).unwrap();
    }
    let prefix = &bytes[..bytes.len() - input.remaining()];
    let mut input = Decoder::new(prefix);
    input.format = format.version();
    input.recorded_fields = fields;
    let historical = ReservedCountryNamingRow::decode_fields(0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(historical.tag, row.tag);
    assert_eq!(historical.country_array, row.country_array);
    assert_eq!(historical.owned_state_count, row.owned_state_count);
    assert_eq!(historical.launch_sources, Recorded::NotRecorded);
    assert_eq!(historical.recorded_fields_bytes().unwrap(), prefix.len());
    assert_eq!(
        historical.not_recorded_fields(),
        full_mask(ReservedCountryNamingRow::SCHEMA.len()) ^ full_mask(fields)
    );
    assert!(
        historical
            .launch_sources
            .require("launch sources of reserved country naming")
            .unwrap_err()
            .contains("not recorded")
    );
    assert!(historical.encode_fields(&mut Vec::new()).is_err());
    assert!(historical.delta(None).is_err());
    assert!(
        format
            .field_gaps(table)
            .any(|reason| reason.contains("launch_sources not recorded"))
    );

    let insert = row.delta(None).unwrap().unwrap();
    let mut input = Decoder::new(&insert.values);
    for kind in &ReservedCountryNamingRow::SCHEMA[..fields] {
        kind.decode_change(&mut input).unwrap();
    }
    let prefix = &insert.values[..insert.values.len() - input.remaining()];
    let mut bytes = Vec::new();
    table.encode_key(0, 0, &mut bytes).unwrap();
    bytes.extend_from_slice(&full_mask(fields).to_le_bytes()[..fields.div_ceil(8)]);
    bytes.extend_from_slice(prefix);
    let mut input = Decoder::new(&bytes);
    input.format = format.version();
    let delta = decode_delta(table, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(
        delta.apply::<ReservedCountryNamingRow>(None).unwrap(),
        historical
    );
    assert!(encode_delta(table, &delta, 0, &mut Vec::new()).is_err());
}

#[test]
fn reserved_launch_packet_option_tags_counts_and_framing_are_strict() {
    let row = ReservedCountryNamingRow {
        launch_sources: Some(ReservedLaunchSources {
            sample_hour: PacketHour::Host,
            naval_bases: LaunchNavalBases::Bases(Vec::new()),
            owned_states: Some(Vec::new()),
        })
        .into(),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &ReservedCountryNamingRow::SCHEMA[..17] {
        kind.decode(&mut input).unwrap();
    }
    let start = bytes.len() - input.remaining();
    let body = start + 4; // Opaque byte length precedes packet's Option tag.
    for (offset, tag) in [(body, 2), (body + 1, 3), (body + 6, 2)] {
        // Packet, navy (2 is native null), then states.
        let mut malformed = bytes.clone();
        malformed[offset] = tag;
        assert!(ReservedCountryNamingRow::decode_fields(0, &mut Decoder::new(&malformed)).is_err());
    }
    for count in [body + 2, body + 7] {
        let mut malformed = bytes.clone();
        malformed[count..count + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(ReservedCountryNamingRow::decode_fields(0, &mut Decoder::new(&malformed)).is_err());
    }
    let mut truncated = bytes.clone();
    truncated.pop();
    assert!(ReservedCountryNamingRow::decode_fields(0, &mut Decoder::new(&truncated)).is_err());
    let mut trailing = bytes.clone();
    let length = u32::from_le_bytes(trailing[start..body].try_into().unwrap());
    trailing[start..body].copy_from_slice(&(length + 1).to_le_bytes());
    trailing.push(0);
    assert!(ReservedCountryNamingRow::decode_fields(0, &mut Decoder::new(&trailing)).is_err());
}
