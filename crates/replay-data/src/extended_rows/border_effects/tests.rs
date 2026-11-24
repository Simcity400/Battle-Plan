use super::*;
use crate::{codec::Decoder, rows::RowCodec};

fn round_trip<T: RowCodec + std::fmt::Debug>(row: &T) {
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        &T::decode_fields(row.key(), &mut Decoder::new(&bytes)).unwrap(),
        row
    );
    bytes.clear();
    assert!(!row.encode_change(Some(row), 0, &mut bytes).unwrap());
    assert!(bytes.is_empty());
}

#[test]
fn border_effect_operands_round_trip_raw_lists_and_three_seed_states() {
    round_trip(&CountryAppearanceRow {
        country: 3,
        subjects: vec![i32::MIN, 17, 17, -1].into(),
        ..Default::default()
    });
    round_trip(&DivisionTemplateRow {
        template: 71,
        name_key: crate::rows::Recorded::Value(b"native_fallback".to_vec()),
        ..Default::default()
    });
    for description_seed in [None, Some(None), Some(Some(i32::MIN)), Some(Some(0))] {
        round_trip(&DecisionRow {
            country: 3,
            container: 3,
            description_seed: crate::rows::Recorded::Value(description_seed),
            ..Default::default()
        });
    }
}

#[test]
fn decision_description_seed_is_rejected_on_non_targeted_containers() {
    for container in 0..3 {
        for description_seed in [Some(None), Some(Some(0)), Some(Some(i32::MIN))] {
            let row = DecisionRow {
                country: 3,
                container,
                description_seed: crate::rows::Recorded::Value(description_seed),
                ..Default::default()
            };
            assert!(row.check_key_fields().is_err());
            assert!(row.encode_change(None, 0, &mut Vec::new()).is_err());
            assert!(
                crate::rows::TableRows::Decisions(vec![row])
                    .validate_keys()
                    .is_err()
            );
        }
        assert!(
            DecisionRow {
                container,
                ..Default::default()
            }
            .check_key_fields()
            .is_ok()
        );
    }
    for container in [3, 4] {
        for description_seed in [None, Some(None), Some(Some(i32::MIN))] {
            round_trip(&DecisionRow {
                country: 3,
                container,
                description_seed: crate::rows::Recorded::Value(description_seed),
                ..Default::default()
            });
        }
    }
}
