use super::*;
use crate::value::Value;

#[test]
fn transport_codec_keeps_null_refusal_unreached_and_signed_native_extremes() {
    for transfer in [
        None,
        Some(UnitTransferInputs::Absent),
        Some(UnitTransferInputs::Present {
            transfer: u64::MAX,
            status: Some(i32::MIN),
            source: None,
        }),
        Some(UnitTransferInputs::Present {
            transfer: 1u64 << 32,
            status: None,
            source: None,
        }),
        Some(UnitTransferInputs::Present {
            transfer: u64::MAX,
            status: Some(0),
            source: Some(u32::MAX),
        }),
    ] {
        for (hour, weight, coastal) in [
            (i32::MIN, None, None),
            (i32::MAX, Some(i64::MIN), Some(false)),
            (0, Some(i64::MAX), Some(true)),
        ] {
            let packet = UnitTransportInputs {
                sample_hour: hour,
                weight,
                transfer,
                coastal_template: coastal,
            };
            let mut bytes = Vec::new();
            packet.encode(&mut bytes).unwrap();
            assert_eq!(Value::encoded_bytes(&packet).unwrap(), bytes.len());
            let mut input = Decoder::new(&bytes);
            assert_eq!(UnitTransportInputs::decode(&mut input).unwrap(), packet);
            input.finish().unwrap();
            assert!(
                UnitTransportInputs::decode(&mut Decoder::new(&bytes[..bytes.len() - 1])).is_err()
            );
        }
    }
    assert!(UnitTransferInputs::decode(&mut Decoder::new(&[2])).is_err());
    assert!(UnitTransferInputs::decode(&mut Decoder::new(&[1])).is_err());
}

#[test]
fn only_transport_suffix_changes_and_refusal_cannot_reuse_previous_packet() {
    let old = DivisionMovementRow {
        division: u64::MAX,
        source: 7,
        target: 7,
        selected: Recorded::Value(Some(SelectedUnitInputs {
            sample_hour: 42,
            exiled: Some(0xfe),
            commander: Some(true),
            retreat: [Some(i64::MIN), Some(i64::MAX)],
        })),
        ..Default::default()
    };
    let current = DivisionMovementRow {
        transport: Recorded::Value(Some(UnitTransportInputs {
            sample_hour: 42,
            weight: Some(0),
            transfer: Some(UnitTransferInputs::Absent),
            coastal_template: Some(false),
        })),
        ..old.clone()
    };
    assert_eq!(DivisionMovementRow::SCHEMA.len(), 22);
    for (before, after) in [(&old, &current), (&current, &old)] {
        let delta = after.delta(Some(before)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 21);
        assert_eq!(delta.apply(Some(before.clone())).unwrap(), *after);
    }
    let historical = DivisionMovementRow {
        transport: Recorded::NotRecorded,
        ..current
    };
    assert!(historical.encode_fields(&mut Vec::new()).is_err());
    assert_ne!(historical.transport, old.transport);
}

#[test]
fn selected_and_transport_suffixes_keep_independent_prefixes_and_masks() {
    let row = DivisionMovementRow {
        division: u64::MAX,
        selected: Recorded::Value(Some(SelectedUnitInputs {
            sample_hour: i32::MIN,
            exiled: Some(0xfe),
            commander: Some(true),
            retreat: [Some(i64::MIN), Some(i64::MAX)],
        })),
        transport: Recorded::Value(Some(UnitTransportInputs {
            sample_hour: i32::MAX,
            weight: Some(i64::MAX),
            transfer: Some(UnitTransferInputs::Absent),
            coastal_template: Some(false),
        })),
        ..Default::default()
    };
    assert_eq!(row.try_clone().unwrap(), row);
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        DivisionMovementRow::decode_fields(row.key(), &mut input).unwrap(),
        row
    );
    input.finish().unwrap();

    let changed = DivisionMovementRow {
        selected: Recorded::Value(None),
        ..row.clone()
    };
    let delta = changed.delta(Some(&row)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 20);
    assert_eq!(delta.apply(Some(row.clone())).unwrap(), changed);
    assert_eq!(changed.transport, row.transport);

    for fields in [15, 21] {
        let mut prefix = Decoder::new(&bytes);
        for kind in &DivisionMovementRow::SCHEMA[..fields] {
            kind.decode(&mut prefix).unwrap();
        }
        let length = bytes.len() - prefix.remaining();
        let mut input = Decoder::new(&bytes[..length]);
        input.recorded_fields = fields;
        if fields == 15 {
            input.format = crate::FORMAT_VERSION - 1;
        }
        let historical = DivisionMovementRow::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(historical.transport, Recorded::NotRecorded);
        if fields == 15 {
            assert_eq!(historical.selected, Recorded::NotRecorded);
        } else {
            assert_eq!(historical.selected, row.selected);
            assert_eq!(historical.not_recorded_fields(), 1 << 21);
        }
    }

    // The provisional transport branch assigned its different packet to the
    // routes suffix. A nonempty old packet cannot be decoded as selected data.
    let mut old_packet = Vec::new();
    row.transport.encode(&mut old_packet).unwrap();
    let mut input = Decoder::new(&old_packet);
    assert!(
        Option::<SelectedUnitInputs>::decode(&mut input)
            .and_then(|_| input.finish())
            .is_err()
    );
}
