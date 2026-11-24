use super::*;

#[test]
fn scripted_stockpile_has_its_own_key_and_is_not_a_division_loss() {
    let division = EquipmentDestructionRow {
        country: 4999,
        cause: EquipmentDestructionCause::ScriptedEquipmentFraction,
        variant: 0xf00100000046,
        amount_raw: 11,
        ..Default::default()
    };
    let stockpile = EquipmentDestructionRow {
        cause: game_model::equipment_destruction::Cause::ScriptedStockpileFraction.into(),
        ..division.clone()
    };
    assert_ne!(division.key(), stockpile.key());
    assert_eq!(
        stockpile
            .delta(None)
            .unwrap()
            .unwrap()
            .apply::<EquipmentDestructionRow>(None)
            .unwrap(),
        stockpile
    );
    let deletion = DivisionEquipmentDeletionRow {
        division_handle: 781_u64 << 32 | 0x1234,
        cause: Some(stockpile.cause),
        ..Default::default()
    };
    assert!(!deletion.valid_disposition());
    assert!(deletion.delta(None).is_err());
}

#[test]
fn stable_causes_roundtrip_and_unknown_is_explicit() {
    for value in 0..=26 {
        let cause = EquipmentDestructionCause::try_from(value).unwrap();
        assert_eq!(cause as u8, value);
        let row = EquipmentDestructionRow {
            country: 4999,
            cause,
            variant: 0xf00100000046,
            amount_raw: 7,
            ..Default::default()
        };
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(delta.apply::<EquipmentDestructionRow>(None).unwrap(), row);
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        assert_eq!(
            EquipmentDestructionRow::decode_fields(row.key(), &mut Decoder::new(&bytes)).unwrap(),
            row
        );
    }
    assert_eq!(
        EquipmentDestructionCause::from(game_model::equipment_destruction::Cause::Unknown),
        EquipmentDestructionCause::Unknown
    );
    for value in [27, 255] {
        assert!(EquipmentDestructionCause::try_from(value).is_err());
        let key = (4999_u128 << 72) | ((value as u128) << 64) | 0xf00100000046;
        let row = EquipmentDestructionRow {
            amount_raw: 7,
            variant: 1,
            ..Default::default()
        };
        let mut delta = row.delta(None).unwrap().unwrap();
        delta.key = key;
        assert!(delta.apply::<EquipmentDestructionRow>(None).is_err());
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        assert!(EquipmentDestructionRow::decode_fields(key, &mut Decoder::new(&bytes)).is_err());
    }
}

#[test]
fn signed_equipment_destruction_roundtrips_and_changes_sign_without_losing_precision() {
    let mut previous = None;
    for amount_raw in [7, -30_000, 40_000, i64::MIN, i64::MAX, -1] {
        let row = EquipmentDestructionRow {
            country: 4999,
            cause: EquipmentDestructionCause::LandCombat,
            variant: 0xf00100000046,
            amount_raw,
            ..Default::default()
        };
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        assert_eq!(
            EquipmentDestructionRow::decode_fields(row.key(), &mut Decoder::new(&bytes)).unwrap(),
            row
        );
        let delta = row.delta(previous.as_ref()).unwrap().unwrap();
        let decoded = delta
            .apply::<EquipmentDestructionRow>(previous.clone())
            .unwrap();
        decoded.check_key_fields().unwrap();
        assert_eq!(decoded, row);
        previous = Some(row);
    }
    let zero = EquipmentDestructionRow {
        amount_raw: 0,
        ..previous.unwrap()
    };
    assert!(zero.delta(None).is_err());
    let mut bytes = Vec::new();
    zero.encode_fields(&mut bytes).unwrap();
    assert!(
        EquipmentDestructionRow::decode_fields(zero.key(), &mut Decoder::new(&bytes))
            .and_then(|row| row.check_key_fields())
            .is_err()
    );
}

#[test]
fn historical_destruction_delta_keeps_positive_amounts_and_absent_coverage() {
    let format = crate::rows::compatibility::WIDGET_CAPTURE_FORMAT - 1;
    let recorded_fields = crate::format::FormatVersion::read(format)
        .unwrap()
        .fields(TableId::EquipmentDestruction);
    assert_eq!(recorded_fields, 1);
    let key = (4999_u128 << 72)
        | ((EquipmentDestructionCause::LandCombat as u128) << 64)
        | 0xf00100000046;
    for key in [key, 0] {
        let mut bytes = Vec::new();
        crate::codec::put_varint(&mut bytes, key);
        Field::I64(123_456).encode(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        input.format = format;
        input.recorded_fields = recorded_fields;
        let previous = crate::rows::decode_row::<EquipmentDestructionRow>(0, &mut input).unwrap();
        previous.check_key_fields().unwrap();
        input.finish().unwrap();
        bytes.clear();
        Field::I64(7)
            .change(Field::I64(previous.amount_raw))
            .unwrap()
            .encode(&mut bytes)
            .unwrap();
        let delta = DeltaRow {
            format,
            key,
            mask: 1,
            recorded_fields,
            values: bytes,
        };
        let row = delta.apply(Some(previous)).unwrap();
        row.check_key_fields().unwrap();
        assert_eq!(row.amount_raw, 7);
        assert_eq!(row.key(), key);
        assert_eq!(row.variant, key as u64);
        assert_eq!(row.coverage, Recorded::NotRecorded);
        assert_eq!(row.unavailable_reason, Recorded::NotRecorded);
        assert_eq!(row.not_recorded_fields(), (1 << 1) | (1 << 2));
        assert_eq!(row.try_clone().unwrap(), row);
        assert!(row.delta(None).is_err());
        for amount_raw in [0, -7] {
            assert!(
                EquipmentDestructionRow {
                    amount_raw,
                    ..row.clone()
                }
                .check_key_fields()
                .is_err()
            );
        }
        assert!(
            EquipmentDestructionRow {
                unavailable_reason: String::new().into(),
                ..row.clone()
            }
            .check_key_fields()
            .is_err()
        );
        assert!(
            EquipmentDestructionRow {
                coverage: EquipmentDestructionCoverage::Complete.into(),
                ..row.clone()
            }
            .check_key_fields()
            .is_err()
        );
        let current = EquipmentDestructionRow {
            coverage: EquipmentDestructionCoverage::Complete.into(),
            unavailable_reason: String::new().into(),
            ..row
        };
        assert_eq!(current.check_key_fields().is_ok(), key != 0);
    }
}

#[test]
fn transfers_roundtrip_with_stable_dispositions_and_no_loss_cause() {
    for (disposition, value, name) in [
        (DivisionDeletionDisposition::Returned, 1, "returned"),
        (DivisionDeletionDisposition::Merged, 2, "merged"),
        (
            DivisionDeletionDisposition::HqTransferred,
            3,
            "hq_transferred",
        ),
        (
            DivisionDeletionDisposition::VolunteerTransferred,
            4,
            "volunteer_transferred",
        ),
        (
            DivisionDeletionDisposition::ExileTransferred,
            5,
            "exile_transferred",
        ),
    ] {
        assert!(matches!(disposition.field(), Field::U8(encoded) if encoded == value));
        assert_eq!(disposition.as_str(), name);

        let row = DivisionEquipmentDeletionRow {
            division_handle: 781_u64 << 32 | 0x1234,
            country: 4999,
            expeditionary_owner: 200,
            disposition,
            cause: None,
        };
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        assert_eq!(&bytes[bytes.len() - 2..], &[value, 255]);
        assert_eq!(
            DivisionEquipmentDeletionRow::decode_fields(row.key(), &mut Decoder::new(&bytes))
                .unwrap(),
            row
        );
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(
            delta.apply::<DivisionEquipmentDeletionRow>(None).unwrap(),
            row
        );
    }
}

#[test]
fn disposition_and_loss_cause_are_separate_canonical_fields() {
    let loss = DivisionEquipmentDeletionRow {
        division_handle: 781_u64 << 32 | 0x1234,
        country: 4999,
        cause: Some(EquipmentDestructionCause::DivisionDestroyed),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    loss.encode_fields(&mut bytes).unwrap();
    assert_eq!(&bytes[bytes.len() - 2..], &[0, 13]);
    let returned = DivisionEquipmentDeletionRow {
        disposition: DivisionDeletionDisposition::Returned,
        cause: None,
        ..loss.clone()
    };
    let delta = returned.delta(Some(&loss)).unwrap().unwrap();
    assert_eq!(delta.mask, (1 << 2) | (1 << 3));
    assert_eq!(delta.apply(Some(loss.clone())).unwrap(), returned);
    for bad in [
        DivisionEquipmentDeletionRow {
            cause: None,
            ..loss
        },
        DivisionEquipmentDeletionRow {
            cause: Some(EquipmentDestructionCause::DivisionDestroyed),
            ..returned
        },
    ] {
        assert!(bad.delta(None).is_err());
        let mut bytes = Vec::new();
        bad.encode_fields(&mut bytes).unwrap();
        assert!(
            DivisionEquipmentDeletionRow::decode_fields(bad.key(), &mut Decoder::new(&bytes))
                .and_then(|row| row.check_key_fields())
                .is_err()
        );
    }
}

#[test]
fn old_disposition_cause_values_are_refused_in_the_disposition_field() {
    let row = DivisionEquipmentDeletionRow {
        division_handle: 781_u64 << 32 | 0x1234,
        ..Default::default()
    };
    for value in [22, 23, 24, 255] {
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let disposition_offset = bytes.len() - 2;
        bytes[disposition_offset] = value;
        assert!(
            DivisionEquipmentDeletionRow::decode_fields(row.key(), &mut Decoder::new(&bytes))
                .is_err()
        );
    }
}

#[test]
fn unavailable_coverage_marker_roundtrips_even_with_no_losses() {
    for (coverage, reason) in [
        (
            EquipmentDestructionCoverage::SupplementalUnavailable,
            "division deletion hook sentinel differs: Δ",
        ),
        (
            EquipmentDestructionCoverage::PartialInterval,
            "startup/branch interval partial (reworded)",
        ),
    ] {
        let marker = EquipmentDestructionRow {
            coverage: coverage.into(),
            unavailable_reason: reason.to_owned().into(),
            ..Default::default()
        };
        assert_eq!(marker.key(), 0);
        assert_eq!(marker.try_clone().unwrap(), marker);
        let mut bytes = Vec::new();
        marker.encode_fields(&mut bytes).unwrap();
        assert_eq!(
            EquipmentDestructionRow::decode_fields(0, &mut Decoder::new(&bytes)).unwrap(),
            marker
        );
        let delta = marker.delta(None).unwrap().unwrap();
        assert_eq!(
            delta.apply::<EquipmentDestructionRow>(None).unwrap(),
            marker
        );
        for bad in [
            EquipmentDestructionRow {
                country: 1,
                ..marker.clone()
            },
            EquipmentDestructionRow {
                cause: EquipmentDestructionCause::LandCombat,
                ..marker.clone()
            },
            EquipmentDestructionRow {
                variant: 1,
                ..marker.clone()
            },
            EquipmentDestructionRow {
                amount_raw: 1,
                ..marker.clone()
            },
            EquipmentDestructionRow {
                unavailable_reason: String::new().into(),
                ..marker.clone()
            },
            EquipmentDestructionRow {
                coverage: EquipmentDestructionCoverage::Complete.into(),
                ..marker
            },
        ] {
            assert!(bad.delta(None).is_err());
            let mut bytes = Vec::new();
            bad.encode_fields(&mut bytes).unwrap();
            assert!(
                EquipmentDestructionRow::decode_fields(bad.key(), &mut Decoder::new(&bytes))
                    .and_then(|row| row.check_key_fields())
                    .is_err()
            );
        }
    }
    let mut bytes = Vec::new();
    Field::I64(0).encode(&mut bytes).unwrap();
    Field::U8(3).encode(&mut bytes).unwrap();
    Field::Bytes(b"reason").encode(&mut bytes).unwrap();
    assert!(EquipmentDestructionRow::decode_fields(0, &mut Decoder::new(&bytes)).is_err());
    bytes.clear();
    Field::I64(0).encode(&mut bytes).unwrap();
    Field::U8(255).encode(&mut bytes).unwrap();
    Field::Bytes(b"reason").encode(&mut bytes).unwrap();
    assert!(EquipmentDestructionRow::decode_fields(0, &mut Decoder::new(&bytes)).is_err());
    bytes.clear();
    Field::I64(0).encode(&mut bytes).unwrap();
    Field::U8(1).encode(&mut bytes).unwrap();
    Field::Bytes(&[255]).encode(&mut bytes).unwrap();
    assert_eq!(
        EquipmentDestructionRow::decode_fields(0, &mut Decoder::new(&bytes))
            .unwrap_err()
            .to_string(),
        "invalid UTF-8 string"
    );
}

#[test]
fn deletion_witness_preserves_full_handle_owner_and_stable_cause() {
    let row = DivisionEquipmentDeletionRow {
        division_handle: (781_u64 << 32) | 0x1234,
        country: 4999,
        disposition: DivisionDeletionDisposition::Loss,
        cause: Some(EquipmentDestructionCause::Unknown),
        expeditionary_owner: 79,
    };
    let delta = row.delta(None).unwrap().unwrap();
    assert_eq!(
        delta.apply::<DivisionEquipmentDeletionRow>(None).unwrap(),
        row
    );
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        DivisionEquipmentDeletionRow::decode_fields(row.key(), &mut Decoder::new(&bytes)).unwrap(),
        row
    );
    let invalid = DivisionEquipmentDeletionRow {
        cause: Some(EquipmentDestructionCause::ParadropDisruption),
        ..row.clone()
    };
    assert!(invalid.delta(None).is_err());
    let mut bytes = Vec::new();
    invalid.encode_fields(&mut bytes).unwrap();
    assert!(
        DivisionEquipmentDeletionRow::decode_fields(row.key(), &mut Decoder::new(&bytes))
            .and_then(|row| row.check_key_fields())
            .is_err()
    );
}

#[test]
fn every_deletion_cause_roundtrips_and_damage_causes_are_refused() {
    for value in 0..=26 {
        let row = DivisionEquipmentDeletionRow {
            division_handle: (781_u64 << 32) | 0x1234,
            country: 4999,
            disposition: DivisionDeletionDisposition::Loss,
            cause: Some(EquipmentDestructionCause::try_from(value).unwrap()),
            ..Default::default()
        };
        if row.valid_disposition() {
            let delta = row.delta(None).unwrap().unwrap();
            assert_eq!(
                delta.apply::<DivisionEquipmentDeletionRow>(None).unwrap(),
                row
            );
        } else {
            assert!(row.delta(None).is_err());
        }
    }
}

#[test]
fn deletion_witness_rejects_null_id_and_preserves_zero_kind() {
    for division_handle in [0, 0x1234] {
        let row = DivisionEquipmentDeletionRow {
            division_handle,
            ..Default::default()
        };
        assert!(row.delta(None).is_err());
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        assert!(
            DivisionEquipmentDeletionRow::decode_fields(row.key(), &mut Decoder::new(&bytes))
                .and_then(|row| row.check_key_fields())
                .is_err()
        );
    }
    let row = DivisionEquipmentDeletionRow {
        division_handle: 781_u64 << 32,
        country: 4999,
        disposition: DivisionDeletionDisposition::Loss,
        cause: Some(EquipmentDestructionCause::DivisionDestroyed),
        ..Default::default()
    };
    let delta = row.delta(None).unwrap().unwrap();
    assert_eq!(
        delta.apply::<DivisionEquipmentDeletionRow>(None).unwrap(),
        row
    );
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        DivisionEquipmentDeletionRow::decode_fields(row.key(), &mut Decoder::new(&bytes)).unwrap(),
        row
    );
}
