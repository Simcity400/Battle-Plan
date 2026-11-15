use super::*;
use crate::FORMAT_VERSION;
use crate::format::FormatVersion;
use crate::rows::compatibility::{EFFECT_INPUTS_FORMAT, POLITICAL_FORMAT};
use crate::value::{EncodeValue, Value};

fn reverse_inputs() -> ReservedReverseVisibilityInputs {
    ReservedReverseVisibilityInputs {
        sample_hour: PacketHour::Host,
        cells: vec![
            Some(ReverseZeroVisibilityInputs::default()),
            None,
            Some(ReverseZeroVisibilityInputs {
                target: Some(0),
                war: Some([0, 0]),
                subject: Some(ZeroSubjectInputs::default()),
                military_access: Some(0),
            }),
            Some(ReverseZeroVisibilityInputs {
                target: None,
                war: Some([1, 0]),
                subject: None,
                military_access: None,
            }),
            Some(ReverseZeroVisibilityInputs {
                target: Some(u32::MAX),
                war: Some([1, u8::MAX]),
                subject: Some(ZeroSubjectInputs {
                    relation_type: Some(u32::MAX),
                    parties: None,
                }),
                military_access: Some(1),
            }),
            Some(ReverseZeroVisibilityInputs {
                target: Some(i32::MIN as u32),
                war: None,
                subject: Some(ZeroSubjectInputs {
                    relation_type: Some(0x30d1),
                    parties: None,
                }),
                military_access: None,
            }),
            Some(ReverseZeroVisibilityInputs {
                target: Some(17),
                war: Some([0, 0]),
                subject: Some(ZeroSubjectInputs {
                    relation_type: Some(0x30d1),
                    parties: Some([Some(17), None]),
                }),
                military_access: Some(0),
            }),
            Some(ReverseZeroVisibilityInputs {
                target: Some(21),
                war: Some([0, 0]),
                subject: Some(ZeroSubjectInputs {
                    relation_type: Some(0x30d1),
                    parties: Some([None, Some(21)]),
                }),
                military_access: None,
            }),
            Some(ReverseZeroVisibilityInputs {
                target: Some(9),
                war: Some([u8::MAX, u8::MAX]),
                subject: Some(ZeroSubjectInputs {
                    relation_type: Some(0x30d1),
                    parties: Some([Some(u32::MAX), Some(i32::MIN as u32)]),
                }),
                military_access: Some(u8::MAX),
            }),
        ],
    }
}

fn reserved_row() -> ReservedCountryNamingRow {
    ReservedCountryNamingRow {
        tag: vec![0xff, 0, b'R'],
        name_tag: b"NAME".to_vec(),
        cosmetic_tag: b"COSMETIC".to_vec(),
        leader_ideology: b"IDEOLOGY".to_vec(),
        group: Some(vec![0xfe, 0]),
        overlord: i32::MIN,
        autonomy: Some(Vec::new()),
        built: 7,
        original_tag: Some(vec![0xfd, 0]).into(),
        original_tag_reference: Some(i32::MAX).into(),
        original_tag_buckets: Some(vec![vec![0, 9, 0], Vec::new()]).into(),
        country_array: Some(vec![0, 9, 0, 9]).into(),
        owned_state_count: Some(i32::MIN).into(),
        faction: Some(u64::MAX).into(),
        movement: Retired.into(),
        land_theatre_count: Some(0).into(),
        launch_sources: Some(ReservedLaunchSources {
            sample_hour: PacketHour::Host,
            naval_bases: LaunchNavalBases::Bases(Vec::new()),
            owned_states: None,
        })
        .into(),
        reverse_visibility: Some(reverse_inputs()).into(),
        country_intel_sample: Some(ReservedCountryIntelSample {
            sample_hour: i32::MIN,
            vector_count: i32::MAX as u32,
        })
        .into(),
        ..Default::default()
    }
}

fn snapshot(row: &ReservedCountryNamingRow) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_row(row, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        decode_row::<ReservedCountryNamingRow>(0, &mut input).unwrap(),
        *row
    );
    input.finish().unwrap();
    assert_eq!(row.recorded_fields_bytes().unwrap() + 1, bytes.len());
    assert_eq!(RowCodec::try_clone(row).unwrap(), *row);
    bytes
}

fn delta_round_trip(
    before: &ReservedCountryNamingRow,
    after: &ReservedCountryNamingRow,
    mask: u64,
) -> DeltaRow {
    let delta = after.delta(Some(before)).unwrap().unwrap();
    assert_eq!(delta.mask, mask);
    let mut bytes = Vec::new();
    encode_delta(TableId::ReservedCountryNaming, &delta, 0, &mut bytes).unwrap();
    let mask_bytes = ReservedCountryNamingRow::SCHEMA.len().div_ceil(8);
    assert_eq!(mask_bytes, 3);
    assert_eq!(bytes[0], 0);
    assert_eq!(&bytes[1..1 + mask_bytes], &mask.to_le_bytes()[..mask_bytes]);
    assert_eq!(&bytes[1 + mask_bytes..], delta.values);
    let mut input = Decoder::new(&bytes);
    let decoded = decode_delta(TableId::ReservedCountryNaming, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply(Some(before.clone())).unwrap(), *after);
    delta
}

#[test]
fn reserved_reverse_snapshot_preserves_certification_null_skips_and_native_bits() {
    assert_eq!(ReverseZeroVisibilityInputs::STRUCT_FIELDS, 4);
    assert_eq!(ReservedReverseVisibilityInputs::STRUCT_FIELDS, 2);
    assert_eq!(ReservedCountryIntelSample::STRUCT_FIELDS, 2);
    assert!(matches!(ReservedCountryNamingRow::SCHEMA[18], Kind::Opaque));
    assert!(matches!(ReservedCountryNamingRow::SCHEMA[19], Kind::Opaque));
    let before = reserved_row();
    for reverse in [
        None,
        Some(ReservedReverseVisibilityInputs {
            sample_hour: PacketHour::Host,
            cells: Vec::new(),
        }),
        Some(ReservedReverseVisibilityInputs {
            sample_hour: PacketHour::Host,
            cells: vec![None],
        }),
        Some(ReservedReverseVisibilityInputs {
            sample_hour: PacketHour::Host,
            cells: vec![Some(ReverseZeroVisibilityInputs::default())],
        }),
        Some(reverse_inputs()),
    ] {
        for certificate in [
            None,
            Some(ReservedCountryIntelSample {
                sample_hour: i32::MIN,
                vector_count: 0,
            }),
            Some(ReservedCountryIntelSample {
                sample_hour: -1,
                vector_count: 1,
            }),
            Some(ReservedCountryIntelSample {
                sample_hour: i32::MAX,
                vector_count: i32::MAX as u32,
            }),
        ] {
            snapshot(&ReservedCountryNamingRow {
                reverse_visibility: reverse.clone().into(),
                country_intel_sample: certificate.into(),
                ..before.clone()
            });
        }
    }
    let refused = ReservedCountryNamingRow {
        reverse_visibility: None.into(),
        country_intel_sample: None.into(),
        ..before.clone()
    };
    let known_empty = ReservedCountryNamingRow {
        reverse_visibility: Some(ReservedReverseVisibilityInputs::default()).into(),
        country_intel_sample: Some(ReservedCountryIntelSample::default()).into(),
        ..before
    };
    assert_ne!(snapshot(&refused), snapshot(&known_empty));
    assert_eq!(
        refused.reverse_visibility.field().encoded_bytes().unwrap(),
        5
    );
    assert_eq!(
        refused
            .country_intel_sample
            .field()
            .encoded_bytes()
            .unwrap(),
        5
    );
}

#[test]
fn reserved_reverse_delta_changes_only_reached_suffix_and_preserves_other_packet() {
    let before = reserved_row();
    for reverse in [
        None,
        Some(ReservedReverseVisibilityInputs::default()),
        Some(ReservedReverseVisibilityInputs {
            cells: vec![None, Some(ReverseZeroVisibilityInputs::default()), None],
            ..reverse_inputs()
        }),
    ] {
        let after = ReservedCountryNamingRow {
            reverse_visibility: reverse.into(),
            ..before.clone()
        };
        for (old, new) in [(&before, &after), (&after, &before)] {
            delta_round_trip(old, new, 1 << 18);
        }
    }
    for certificate in [
        None,
        Some(ReservedCountryIntelSample::default()),
        Some(ReservedCountryIntelSample {
            sample_hour: i32::MAX,
            vector_count: i32::MAX as u32,
        }),
    ] {
        let after = ReservedCountryNamingRow {
            country_intel_sample: certificate.into(),
            ..before.clone()
        };
        for (old, new) in [(&before, &after), (&after, &before)] {
            delta_round_trip(old, new, 1 << 19);
        }
    }
    let after = ReservedCountryNamingRow {
        reverse_visibility: None.into(),
        country_intel_sample: None.into(),
        ..before.clone()
    };
    delta_round_trip(&before, &after, (1 << 18) | (1 << 19));
    let insert = before.delta(None).unwrap().unwrap();
    assert_eq!(
        insert.mask,
        full_mask(ReservedCountryNamingRow::SCHEMA.len())
    );
    assert_eq!(
        insert.apply::<ReservedCountryNamingRow>(None).unwrap(),
        before
    );
    assert!(before.delta(Some(&before)).unwrap().is_none());
}

#[test]
fn reserved_reverse_cell_change_emits_complete_opaque_payload() {
    let before = reserved_row();
    let mut changed = reverse_inputs();
    changed.cells[1] = Some(ReverseZeroVisibilityInputs::default());
    let after = ReservedCountryNamingRow {
        reverse_visibility: Some(changed).into(),
        ..before.clone()
    };
    let delta = delta_round_trip(&before, &after, 1 << 18);
    let mut expected = Vec::new();
    after
        .reverse_visibility
        .field()
        .encode(&mut expected)
        .unwrap();
    assert_eq!(delta.values, expected);
    assert_eq!(
        delta.values.len(),
        4 + Value::encoded_bytes(&after.reverse_visibility).unwrap()
    );
    assert!(delta.values.len() > size_of::<i32>());
    let mut native = Vec::new();
    let encoded: &dyn EncodeValue = &after.reverse_visibility;
    encoded.encode_value(&mut native).unwrap();
    assert_eq!(encoded.encoded_bytes().unwrap(), native.len());
    assert_eq!(encoded.recorded_bytes().unwrap(), native.len());
    assert_eq!(&delta.values[4..], native);
    assert_eq!(
        u32::from_le_bytes(delta.values[..4].try_into().unwrap()) as usize,
        native.len()
    );

    let after = ReservedCountryNamingRow {
        country_intel_sample: Some(ReservedCountryIntelSample {
            sample_hour: i32::MAX,
            vector_count: i32::MAX as u32,
        })
        .into(),
        ..before.clone()
    };
    let delta = delta_round_trip(&before, &after, 1 << 19);
    let mut expected = Vec::new();
    after
        .country_intel_sample
        .field()
        .encode(&mut expected)
        .unwrap();
    assert_eq!(delta.values, expected);
    assert_eq!(delta.values.len(), 4 + 1 + 4 + 4);
}

#[test]
fn reserved_reverse_identical_matrix_is_unchanged_and_a_change_carries_no_hour() {
    let cell = Some(ReverseZeroVisibilityInputs {
        target: Some(u32::MAX),
        war: Some([1, u8::MAX]),
        subject: Some(ZeroSubjectInputs {
            relation_type: Some(0x30d1),
            parties: Some([Some(0), Some(u32::MAX)]),
        }),
        military_access: Some(1),
    });
    let cell_bytes = Value::encoded_bytes(&cell).unwrap();
    for count in [1, 2, 513] {
        let before = ReservedCountryNamingRow {
            reverse_visibility: Some(ReservedReverseVisibilityInputs {
                sample_hour: PacketHour::Host,
                cells: vec![cell; count],
            })
            .into(),
            ..reserved_row()
        };
        // Equal acquisitions at consecutive hours are one unchanged row.
        assert!(before.clone().delta(Some(&before)).unwrap().is_none());
        let mut cells = vec![cell; count];
        cells[count - 1] = None;
        let after = ReservedCountryNamingRow {
            reverse_visibility: Some(ReservedReverseVisibilityInputs {
                sample_hour: PacketHour::Host,
                cells,
            })
            .into(),
            ..before.clone()
        };
        let delta = delta_round_trip(&before, &after, 1 << 18);
        // Opaque length, packet presence and count precede every cell.
        assert_eq!(delta.values.len(), 4 + 1 + 4 + (count - 1) * cell_bytes + 1);
        let mut input = Decoder::new(&delta.values);
        let changed = Kind::Opaque.decode_change(&mut input).unwrap();
        input.finish().unwrap();
        let value = <Option<ReservedReverseVisibilityInputs> as NativeValue>::from_field(
            changed.apply(Kind::Opaque, Field::Empty).unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(value.sample_hour, PacketHour::Host);
        assert_eq!(value.cells[..count - 1], vec![cell; count - 1]);
        assert_eq!(value.cells[count - 1], None);
    }
}

#[test]
fn reserved_reverse_historical_prefix_keeps_named_gaps_and_original_mask_width() {
    let table = TableId::ReservedCountryNaming;
    for (index, name) in [(18, "reverse_visibility"), (19, "country_intel_sample")] {
        let field = table
            .last_change()
            .appended
            .iter()
            .find(|field| field.name == name)
            .unwrap();
        assert_eq!(field.index, index);
        assert_eq!(field.introduced, POLITICAL_FORMAT);
    }
    let format = FormatVersion::read(EFFECT_INPUTS_FORMAT).unwrap();
    let fields = format.fields(table);
    assert_eq!(fields, 13);
    let row = reserved_row();
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
    assert_eq!(historical.faction, Recorded::NotRecorded);
    assert_eq!(historical.launch_sources, Recorded::NotRecorded);
    assert_eq!(historical.reverse_visibility, Recorded::NotRecorded);
    assert_eq!(historical.country_intel_sample, Recorded::NotRecorded);
    assert_eq!(historical.attack, Recorded::NotRecorded);
    assert_eq!(historical.raw_country_observations, Recorded::NotRecorded);
    assert_eq!(historical.recorded_fields_bytes().unwrap(), prefix.len());
    assert_eq!(
        historical.not_recorded_fields(),
        full_mask(ReservedCountryNamingRow::SCHEMA.len()) ^ full_mask(fields)
    );
    assert!(historical.encode_fields(&mut Vec::new()).is_err());
    assert!(historical.delta(None).is_err());
    for name in ["reverse_visibility", "country_intel_sample"] {
        assert!(
            format
                .field_gaps(table)
                .any(|reason| reason.contains(&format!("{name} not recorded")))
        );
    }
    assert_eq!(
        historical
            .reverse_visibility
            .require("reverse visibility of reserved country naming"),
        Err("reverse visibility of reserved country naming not recorded".to_owned())
    );
    assert_eq!(
        historical
            .country_intel_sample
            .require("country intel sample of reserved country naming"),
        Err("country intel sample of reserved country naming not recorded".to_owned())
    );

    let insert = row.delta(None).unwrap().unwrap();
    let mut input = Decoder::new(&insert.values);
    for kind in &ReservedCountryNamingRow::SCHEMA[..fields] {
        kind.decode_change(&mut input).unwrap();
    }
    let values = &insert.values[..insert.values.len() - input.remaining()];
    let mut bytes = Vec::new();
    table.encode_key(0, 0, &mut bytes).unwrap();
    let mask_bytes = fields.div_ceil(8);
    assert_eq!(mask_bytes, 2);
    bytes.extend_from_slice(&full_mask(fields).to_le_bytes()[..mask_bytes]);
    bytes.extend_from_slice(values);
    let mut input = Decoder::new(&bytes);
    input.format = format.version();
    let delta = decode_delta(table, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(delta.recorded_fields, fields);
    assert_eq!(delta.format, format.version());
    assert_eq!(
        delta.apply::<ReservedCountryNamingRow>(None).unwrap(),
        historical
    );
    // Insert differences use zero numeric baselines. Applying them twice to
    // the extreme-valued prefix must retain the decoder's strict range check.
    assert!(delta.apply(Some(row.clone())).is_err());
    let current_baseline = ReservedCountryNamingRow {
        overlord: 0,
        built: 0,
        original_tag_reference: None.into(),
        owned_state_count: None.into(),
        ..row.clone()
    };
    assert_eq!(delta.apply(Some(current_baseline)).unwrap(), historical);
    assert!(encode_delta(table, &delta, 0, &mut Vec::new()).is_err());

    let current = FormatVersion::read(FORMAT_VERSION).unwrap();
    assert_eq!(
        current.fields(table),
        ReservedCountryNamingRow::SCHEMA.len()
    );
    assert!(!current.field_gaps(table).any(|reason| {
        reason.contains("reverse_visibility") || reason.contains("country_intel_sample")
    }));
    let refused = ReservedCountryNamingRow {
        reverse_visibility: None.into(),
        country_intel_sample: None.into(),
        ..row
    };
    assert_eq!(refused.not_recorded_fields(), 0);
    snapshot(&refused);
}

fn rejected_reverse(bytes: &[u8]) -> bool {
    let mut input = Decoder::new(bytes);
    <Option<ReservedReverseVisibilityInputs> as Value>::decode(&mut input)
        .and_then(|_| input.finish())
        .is_err()
}

#[test]
fn reserved_reverse_codec_rejects_bad_option_tags_counts_and_opaque_framing() {
    let packet = Some(ReservedReverseVisibilityInputs {
        sample_hour: PacketHour::Host,
        cells: vec![Some(ReverseZeroVisibilityInputs {
            target: Some(u32::MAX),
            war: Some([1, u8::MAX]),
            subject: Some(ZeroSubjectInputs {
                relation_type: Some(0x30d1),
                parties: Some([Some(0), Some(u32::MAX)]),
            }),
            military_access: Some(1),
        })],
    });
    let mut bytes = Vec::new();
    Value::encode(&packet, &mut bytes).unwrap();
    assert_eq!(bytes.len(), 33);
    assert!(!rejected_reverse(&bytes));
    for tag in [0, 5, 6, 11, 14, 15, 20, 21, 26, 31] {
        let mut malformed = bytes.clone();
        malformed[tag] = 2;
        assert!(rejected_reverse(&malformed), "option tag at {tag}");
    }
    let mut malformed = bytes.clone();
    malformed[1..5].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(rejected_reverse(&malformed));
    for length in 0..bytes.len() {
        assert!(rejected_reverse(&bytes[..length]), "truncated at {length}");
    }
    let mut malformed = bytes.clone();
    malformed.push(0);
    assert!(rejected_reverse(&malformed));

    let row = ReservedCountryNamingRow {
        reverse_visibility: packet.into(),
        ..reserved_row()
    };
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &ReservedCountryNamingRow::SCHEMA[..18] {
        kind.decode(&mut input).unwrap();
    }
    let start = bytes.len() - input.remaining();
    let body = start + 4;
    for tag in [0, 5, 6, 11, 14, 15, 20, 21, 26, 31] {
        let mut malformed = bytes.clone();
        malformed[body + tag] = 2;
        assert!(ReservedCountryNamingRow::decode_fields(0, &mut Decoder::new(&malformed)).is_err());
    }
    for adjustment in [-1_i32, 1] {
        let mut malformed = bytes.clone();
        let length = u32::from_le_bytes(malformed[start..body].try_into().unwrap());
        let length = u32::try_from(i64::from(length) + i64::from(adjustment)).unwrap();
        malformed[start..body].copy_from_slice(&length.to_le_bytes());
        assert!(ReservedCountryNamingRow::decode_fields(0, &mut Decoder::new(&malformed)).is_err());
    }

    let certificate = Some(ReservedCountryIntelSample {
        sample_hour: i32::MIN,
        vector_count: i32::MAX as u32,
    });
    let mut bytes = Vec::new();
    Value::encode(&certificate, &mut bytes).unwrap();
    assert_eq!(bytes.len(), 9);
    let mut malformed = bytes.clone();
    malformed[0] = 2;
    assert!(
        <Option<ReservedCountryIntelSample> as Value>::decode(&mut Decoder::new(&malformed))
            .is_err()
    );
    for length in 0..bytes.len() {
        assert!(
            <Option<ReservedCountryIntelSample> as Value>::decode(&mut Decoder::new(
                &bytes[..length]
            ))
            .is_err()
        );
    }
    let certificate_bytes = bytes;

    let row = ReservedCountryNamingRow {
        country_intel_sample: certificate.into(),
        ..reserved_row()
    };
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &ReservedCountryNamingRow::SCHEMA[..19] {
        kind.decode(&mut input).unwrap();
    }
    let start = bytes.len() - input.remaining();
    let body = start + 4;
    let packet_bytes = u32::from_le_bytes(bytes[start..body].try_into().unwrap()) as usize;
    assert_eq!(packet_bytes, certificate_bytes.len());
    assert_eq!(&bytes[body..body + packet_bytes], certificate_bytes);
    let mut malformed = bytes.clone();
    malformed[body] = 2;
    assert!(ReservedCountryNamingRow::decode_fields(0, &mut Decoder::new(&malformed)).is_err());
    for length in [8_u32, 10] {
        let mut malformed = bytes.clone();
        malformed[start..body].copy_from_slice(&length.to_le_bytes());
        assert!(ReservedCountryNamingRow::decode_fields(0, &mut Decoder::new(&malformed)).is_err());
    }
    assert!(
        ReservedCountryNamingRow::decode_fields(
            0,
            &mut Decoder::new(&bytes[..body + packet_bytes - 1]),
        )
        .is_err()
    );
    bytes.pop();
    assert!(ReservedCountryNamingRow::decode_fields(0, &mut Decoder::new(&bytes)).is_err());
}

#[test]
fn reserved_country_intel_certificate_rejects_impossible_native_counts_at_codec_boundary() {
    let before = reserved_row();
    for count in [(i32::MAX as u32) + 1, u32::MAX] {
        let invalid_sample = ReservedCountryIntelSample {
            sample_hour: i32::MIN,
            vector_count: count,
        };
        assert!(Value::encoded_bytes(&invalid_sample).is_err());
        assert!(Value::try_clone(&invalid_sample).is_err());
        let mut bytes = vec![0xa5];
        assert!(Value::encode_reserved(&invalid_sample, &mut bytes).is_err());
        assert_eq!(bytes, [0xa5]);
        assert!(Value::encode(&Some(invalid_sample), &mut bytes).is_err());
        assert_eq!(bytes, [0xa5]);
        let packet = Some(invalid_sample);
        let encoded: &dyn EncodeValue = &packet;
        assert!(encoded.encoded_bytes().is_err());
        assert!(encoded.recorded_bytes().is_err());
        assert!(encoded.encode_value(&mut bytes).is_err());
        assert_eq!(bytes, [0xa5]);
        let row = ReservedCountryNamingRow {
            country_intel_sample: packet.into(),
            ..before.clone()
        };
        assert!(encode_row(&row, 0, &mut Vec::new()).is_err());
        assert!(RowCodec::try_clone(&row).is_err());
        assert!(row.delta(None).is_err());
        assert!(row.delta(Some(&before)).is_err());

        let valid_sample = Some(ReservedCountryIntelSample {
            sample_hour: i32::MIN,
            vector_count: i32::MAX as u32,
        });
        let mut bytes = Vec::new();
        Value::encode(&valid_sample, &mut bytes).unwrap();
        bytes[5..9].copy_from_slice(&count.to_le_bytes());
        assert!(
            <Option<ReservedCountryIntelSample> as Value>::decode(&mut Decoder::new(&bytes))
                .is_err()
        );
        assert!(
            <ReservedCountryIntelSample as Value>::decode(&mut Decoder::new(&bytes[1..])).is_err()
        );

        let mut fields = Vec::new();
        before.encode_fields(&mut fields).unwrap();
        let mut input = Decoder::new(&fields);
        for kind in &ReservedCountryNamingRow::SCHEMA[..19] {
            kind.decode(&mut input).unwrap();
        }
        let body = fields.len() - input.remaining() + 4;
        fields[body + 5..body + 9].copy_from_slice(&count.to_le_bytes());
        assert!(ReservedCountryNamingRow::decode_fields(0, &mut Decoder::new(&fields)).is_err());

        let after = ReservedCountryNamingRow {
            country_intel_sample: Some(ReservedCountryIntelSample {
                sample_hour: i32::MAX,
                vector_count: i32::MAX as u32,
            })
            .into(),
            ..before.clone()
        };
        let mut delta = after.delta(Some(&before)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 19);
        delta.values[4 + 5..4 + 9].copy_from_slice(&count.to_le_bytes());
        assert!(delta.apply(Some(before.clone())).is_err());
    }
}
