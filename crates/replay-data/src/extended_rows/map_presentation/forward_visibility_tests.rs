use super::*;
use crate::value::{EncodeValue, Value};

fn complete_inputs() -> ReverseZeroVisibilityInputs {
    ReverseZeroVisibilityInputs {
        target: Some(u32::MAX),
        war: Some([1, u8::MAX]),
        subject: Some(ZeroSubjectInputs {
            relation_type: Some(0x30d1),
            parties: Some([Some(0), Some(i32::MIN as u32)]),
        }),
        military_access: Some(1),
    }
}

fn counter_row() -> CountryCounterInputRow {
    CountryCounterInputRow {
        country: u32::MAX,
        pride_of_the_fleet: u64::MAX,
        owned_state_count: i32::MIN,
        controlled_state_count: i32::MAX,
        first_division: u64::MAX,
        owned_states: Some(vec![u32::MAX, 0, u32::MAX]).into(),
        controlled_states: Some(Vec::new()).into(),
        core_states: None.into(),
        army_leaders: Some(vec![u64::MAX, 0]).into(),
        navy_leaders: None.into(),
        mio_policies: Some(vec![u32::MAX, 0]).into(),
        neighbors: Some(vec![0, u32::MAX, 0]).into(),
        co_belligerents: Some(vec![0, 17]).into(),
        zero_war: Some([0, 0]).into(),
        movement: Recorded::Value(Retired),
        zero_subject: Some(ZeroSubjectInputs::default()).into(),
        pride_name: Some(vec![0xff, 0, b'P']).into(),
        forward_visibility: Some(complete_inputs()).into(),
        air_modifiers: Some([
            Some(i64::MIN),
            None,
            Some(0),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(i64::MAX),
        ])
        .into(),
        ..Default::default()
    }
}

fn snapshot(row: &CountryCounterInputRow) -> Vec<u8> {
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        CountryCounterInputRow::decode_fields(row.key(), &mut input).unwrap(),
        *row
    );
    input.finish().unwrap();
    assert_eq!(row.recorded_fields_bytes().unwrap(), bytes.len());
    assert_eq!(RowCodec::try_clone(row).unwrap(), *row);
    bytes
}

fn delta_round_trip(before: &CountryCounterInputRow, after: &CountryCounterInputRow) {
    let table = TableId::CountryCounterInputs;
    let delta = after.delta(Some(before)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 16);
    assert_eq!(table.mask_width(), 5);
    let mut bytes = Vec::new();
    encode_delta(table, &delta, delta.key, &mut bytes).unwrap();
    assert_eq!(&bytes[..6], &[0, 0, 0, 1, 0, 0]);
    let mut expected = Vec::new();
    after
        .forward_visibility
        .field()
        .encode(&mut expected)
        .unwrap();
    assert_eq!(delta.values, expected);
    assert_eq!(&bytes[6..], expected);
    let mut input = Decoder::new(&bytes);
    let decoded = decode_delta(table, delta.key, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply(Some(before.clone())).unwrap(), *after);
}

#[test]
fn forward_visibility_snapshot_preserves_independent_skips_and_raw_native_bits() {
    assert_eq!(ReverseZeroVisibilityInputs::STRUCT_FIELDS, 4);
    assert_eq!(CountryCounterInputRow::SCHEMA.len(), 35);
    assert!(matches!(CountryCounterInputRow::SCHEMA[16], Kind::Opaque));
    let before = counter_row();
    let mut inputs = vec![None, Some(ReverseZeroVisibilityInputs::default())];
    for military_access in [None, Some(0), Some(1), Some(u8::MAX)] {
        for target in [
            None,
            Some(0),
            Some(17),
            Some(i32::MIN as u32),
            Some(u32::MAX),
        ] {
            inputs.push(Some(ReverseZeroVisibilityInputs {
                target,
                war: None,
                subject: Some(ZeroSubjectInputs {
                    relation_type: Some(0x30d1),
                    parties: Some([Some(17), None]),
                }),
                military_access,
            }));
        }
    }
    inputs.push(Some(complete_inputs()));
    for input in inputs {
        let row = CountryCounterInputRow {
            forward_visibility: input.into(),
            ..before.clone()
        };
        snapshot(&row);
        assert_eq!(row.zero_war, before.zero_war);
        assert_eq!(row.zero_subject, before.zero_subject);
    }
}

#[test]
fn forward_visibility_delta_crosses_mask_boundary_and_clears_and_reintroduces() {
    let before = counter_row();
    for forward in [None, Some(ReverseZeroVisibilityInputs::default())] {
        let after = CountryCounterInputRow {
            forward_visibility: forward.into(),
            ..before.clone()
        };
        delta_round_trip(&before, &after);
        delta_round_trip(&after, &before);
    }
    let insert = before.delta(None).unwrap().unwrap();
    assert_eq!(insert.mask, full_mask(CountryCounterInputRow::SCHEMA.len()));
    assert_eq!(
        insert.apply::<CountryCounterInputRow>(None).unwrap(),
        before
    );
    assert!(before.delta(Some(&before)).unwrap().is_none());
}

fn rejected_inputs(bytes: &[u8]) -> bool {
    let mut input = Decoder::new(bytes);
    <Option<ReverseZeroVisibilityInputs> as Value>::decode(&mut input)
        .and_then(|_| input.finish())
        .is_err()
}

#[test]
fn forward_visibility_codec_rejects_tags_truncation_and_inexact_opaque_framing() {
    let packet = Some(complete_inputs());
    let mut bytes = Vec::new();
    Value::encode(&packet, &mut bytes).unwrap();
    assert_eq!(bytes.len(), 28);
    assert_eq!(bytes[27], 1);
    assert!(!rejected_inputs(&bytes));
    let encoded: &dyn EncodeValue = &packet;
    assert_eq!(encoded.encoded_bytes().unwrap(), bytes.len());
    assert_eq!(encoded.recorded_bytes().unwrap(), bytes.len());
    let mut through_trait = Vec::new();
    encoded.encode_value(&mut through_trait).unwrap();
    assert_eq!(through_trait, bytes);
    for tag in [0, 1, 6, 9, 10, 15, 16, 21, 26] {
        let mut malformed = bytes.clone();
        malformed[tag] = 2;
        assert!(rejected_inputs(&malformed), "option tag at {tag}");
    }
    for length in 0..bytes.len() {
        assert!(rejected_inputs(&bytes[..length]), "truncated at {length}");
    }
    let mut malformed = bytes.clone();
    malformed.push(0);
    assert!(rejected_inputs(&malformed));

    let row = counter_row();
    let bytes = snapshot(&row);
    let mut input = Decoder::new(&bytes);
    for kind in &CountryCounterInputRow::SCHEMA[..16] {
        kind.decode(&mut input).unwrap();
    }
    let start = bytes.len() - input.remaining();
    let body = start + 4;
    let packet_bytes = u32::from_le_bytes(bytes[start..body].try_into().unwrap()) as usize;
    assert_eq!(packet_bytes, through_trait.len());
    assert_eq!(&bytes[body..body + packet_bytes], through_trait);
    for tag in [0, 1, 6, 9, 10, 15, 16, 21, 26] {
        let mut malformed = bytes.clone();
        malformed[body + tag] = 2;
        assert!(
            CountryCounterInputRow::decode_fields(row.key(), &mut Decoder::new(&malformed))
                .is_err()
        );
    }
    for length in [27_u32, 29] {
        let mut malformed = bytes.clone();
        malformed[start..body].copy_from_slice(&length.to_le_bytes());
        assert!(
            CountryCounterInputRow::decode_fields(row.key(), &mut Decoder::new(&malformed))
                .is_err()
        );
    }
    let mut malformed = bytes.clone();
    malformed.push(0);
    malformed[start..body].copy_from_slice(&29_u32.to_le_bytes());
    assert!(
        CountryCounterInputRow::decode_fields(row.key(), &mut Decoder::new(&malformed)).is_err()
    );
    assert!(
        CountryCounterInputRow::decode_fields(
            row.key(),
            &mut Decoder::new(&bytes[..body + packet_bytes - 1]),
        )
        .is_err()
    );
    assert!(
        CountryCounterInputRow::decode_fields(
            row.key(),
            &mut Decoder::new(&bytes[..bytes.len() - 1])
        )
        .is_err()
    );
}

#[test]
fn forward_visibility_and_combat_gate_keep_independent_current_mask_bits() {
    let before = CountryCounterInputRow {
        air_combat_modifiers_known: true.into(),
        air_attack_factor: Some(i64::MIN).into(),
        air_defence_factor: Some(i64::MAX).into(),
        ..counter_row()
    };
    snapshot(&before);
    let after = CountryCounterInputRow {
        forward_visibility: None.into(),
        air_combat_modifiers_known: false.into(),
        ..before.clone()
    };
    let delta = after.delta(Some(&before)).unwrap().unwrap();
    assert_eq!(delta.mask, (1 << 16) | (1 << 33));
    let mut bytes = Vec::new();
    encode_delta(TableId::CountryCounterInputs, &delta, delta.key, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded = decode_delta(TableId::CountryCounterInputs, delta.key, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply(Some(before)).unwrap(), after);
    snapshot(&after);
}
#[test]
fn forward_visibility_distinguishes_historical_missing_refused_and_present_skips() {
    let row = counter_row();
    let historical = CountryCounterInputRow {
        forward_visibility: Recorded::NotRecorded,
        ..row.clone()
    };
    assert_eq!(historical.not_recorded_fields(), 1 << 16);
    assert_eq!(
        historical
            .forward_visibility
            .require("forward visibility of country counter inputs"),
        Err("forward visibility of country counter inputs not recorded".to_owned())
    );
    assert!(historical.encode_fields(&mut Vec::new()).is_err());
    assert!(historical.delta(None).is_err());
    let refused = CountryCounterInputRow {
        forward_visibility: None.into(),
        ..row.clone()
    };
    let skipped = CountryCounterInputRow {
        forward_visibility: Some(ReverseZeroVisibilityInputs::default()).into(),
        ..row
    };
    assert_eq!(refused.not_recorded_fields(), 0);
    assert_eq!(skipped.not_recorded_fields(), 0);
    assert_eq!(
        refused.forward_visibility.field().encoded_bytes().unwrap(),
        5
    );
    assert_eq!(
        skipped.forward_visibility.field().encoded_bytes().unwrap(),
        9
    );
    assert_ne!(snapshot(&refused), snapshot(&skipped));
}

#[test]
fn forward_visibility_wider_delta_mask_keeps_following_table_delta_aligned() {
    let before = counter_row();
    let after = CountryCounterInputRow {
        forward_visibility: None.into(),
        ..before.clone()
    };
    let country_delta = after.delta(Some(&before)).unwrap().unwrap();
    let features_before = SimulationFeatureRow::default();
    let features_after = SimulationFeatureRow {
        human_ai: u8::MAX,
        ..features_before
    };
    let features_delta = features_after
        .delta(Some(&features_before))
        .unwrap()
        .unwrap();
    let mut bytes = Vec::new();
    encode_delta(
        TableId::CountryCounterInputs,
        &country_delta,
        country_delta.key,
        &mut bytes,
    )
    .unwrap();
    encode_delta(TableId::SimulationFeatures, &features_delta, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded =
        decode_delta(TableId::CountryCounterInputs, country_delta.key, &mut input).unwrap();
    assert_eq!(decoded.mask, 1 << 16);
    assert_eq!(decoded.apply(Some(before)).unwrap(), after);
    let decoded = decode_delta(TableId::SimulationFeatures, 0, &mut input).unwrap();
    assert_eq!(
        decoded.apply(Some(features_before)).unwrap(),
        features_after
    );
    input.finish().unwrap();
}
