//! Authored, unrun codec/admission fixtures.
use super::*;
use crate::value::Value;

fn round_trip<T: Value + std::fmt::Debug + PartialEq>(value: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    Value::encode(value, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(T::decode(&mut input).unwrap(), *value);
    input.finish().unwrap();
    bytes
}
fn packet() -> UnitCombatReferences {
    UnitCombatReferences {
        sample_hour: i32::MIN,
        entries: vec![
            UnitCombatReference {
                combat: u64::MAX,
                kind: 0,
            },
            UnitCombatReference {
                combat: 0x1234567887654321,
                kind: 2,
            },
            UnitCombatReference {
                combat: u64::MAX,
                kind: 0,
            },
        ],
    }
}
#[test]
fn ordered_full_handles_and_repeats_round_trip_without_class_word_inference() {
    let packet = packet();
    let bytes = round_trip(&packet);
    assert_eq!(bytes.len(), 8 + 9 * 3);
    assert_eq!(&bytes[8..16], &u64::MAX.to_le_bytes());
    assert_eq!(bytes[16], 0);
    assert_eq!(bytes[25], 2);
    assert_eq!(bytes[34], 0);
}
#[test]
fn historical_absence_current_refusal_and_known_empty_are_distinct() {
    type Field = Recorded<Option<UnitCombatReferences>>;
    let absent = <Field as Value>::not_recorded().unwrap();
    assert_eq!(absent, Recorded::NotRecorded);
    assert_eq!(Value::recorded_bytes(&absent).unwrap(), 0);
    assert!(Value::encode(&absent, &mut Vec::new()).is_err());
    assert_eq!(round_trip::<Field>(&None.into()), [0]);
    let empty = Some(UnitCombatReferences {
        sample_hour: 0,
        entries: Vec::new(),
    })
    .into();
    assert_eq!(round_trip::<Field>(&empty), [1, 0, 0, 0, 0, 0, 0, 0, 0]);
}
#[test]
fn malformed_tags_handles_kinds_counts_and_truncation_refuse() {
    let bytes = round_trip(&packet());
    for end in 0..bytes.len() {
        assert!(UnitCombatReferences::decode(&mut Decoder::new(&bytes[..end])).is_err());
    }
    let mut zero = bytes.clone();
    zero[8..16].fill(0);
    assert!(UnitCombatReferences::decode(&mut Decoder::new(&zero)).is_err());
    let mut kind = bytes.clone();
    kind[16] = 3;
    assert!(UnitCombatReferences::decode(&mut Decoder::new(&kind)).is_err());
    let mut count = bytes;
    count[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(UnitCombatReferences::decode(&mut Decoder::new(&count)).is_err());
    assert!(<Option<UnitCombatReferences> as Value>::decode(&mut Decoder::new(&[2])).is_err());
    for entry in [
        UnitCombatReference { combat: 0, kind: 0 },
        UnitCombatReference { combat: 7, kind: 3 },
    ] {
        assert!(Value::encode(&entry, &mut Vec::new()).is_err());
    }
}
#[test]
fn frame_hour_count_and_kind_admission_are_exact() {
    let packet = packet();
    assert!(packet.valid_at(i32::MIN, 3));
    assert!(!packet.valid_at(i32::MAX, 3));
    assert!(!packet.valid_at(i32::MIN, 2));
    let mut invalid = packet;
    invalid.entries[1].kind = 3;
    assert!(!invalid.valid_at(i32::MIN, 3));
}
#[test]
fn both_existing_rows_reject_a_present_packet_with_wrong_count() {
    let movement = DivisionMovementRow {
        combat_count: 2,
        combat_references: Some(packet()).into(),
        ..Default::default()
    };
    assert!(movement.check_key_fields().is_err());
    let gun = RailwayGunCounterInputRow {
        combat_count: 2,
        combat_references: Some(packet()).into(),
        ..Default::default()
    };
    assert!(gun.check_key_fields().is_err());
    assert!(
        DivisionMovementRow {
            combat_count: 2,
            combat_references: None.into(),
            ..Default::default()
        }
        .check_key_fields()
        .is_ok()
    );
}
fn suffix<T: RowCodec + Fields + std::fmt::Debug>(table: TableId, index: usize, old: &T, row: &T) {
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(&T::decode_fields(row.key(), &mut input).unwrap(), row);
    input.finish().unwrap();
    let delta = row.delta(Some(old)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << index);
    bytes.clear();
    encode_delta(table, &delta, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let delta = decode_delta(table, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(&delta.apply(Some(old.clone())).unwrap(), row);
}
#[test]
fn movement_and_railway_suffix_deltas_keep_legacy_fields_and_clear_refusal() {
    let old = DivisionMovementRow {
        division: u64::MAX,
        combat_count: 3,
        combat_references: None.into(),
        ..Default::default()
    };
    let row = DivisionMovementRow {
        combat_references: Some(packet()).into(),
        ..old.clone()
    };
    suffix(TableId::DivisionMovement, 19, &old, &row);
    suffix(TableId::DivisionMovement, 19, &row, &old);
    let old = RailwayGunCounterInputRow {
        gun: u64::MAX,
        combat_count: 3,
        combat_references: None.into(),
        ..Default::default()
    };
    let row = RailwayGunCounterInputRow {
        combat_references: Some(packet()).into(),
        ..old.clone()
    };
    suffix(TableId::RailwayGunCounterInputs, 28, &old, &row);
    suffix(TableId::RailwayGunCounterInputs, 28, &row, &old);
}
fn historical_prefix<T: RowCodec + Fields + std::fmt::Debug>(
    table: TableId,
    index: usize,
    row: &T,
) -> T {
    let introduced = table
        .last_change()
        .appended
        .iter()
        .find(|field| field.index == index)
        .expect("fresh suffix has a compatibility entry")
        .introduced;
    let format = crate::format::FormatVersion::read(introduced - 1).unwrap();
    let fields = format.fields(table);
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &T::SCHEMA[..fields] {
        kind.decode(&mut input).unwrap();
    }
    let prefix = &bytes[..bytes.len() - input.remaining()];
    let mut input = Decoder::new(prefix);
    input.format = format.version();
    input.recorded_fields = fields;
    let decoded = T::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(
        decoded.not_recorded_fields(),
        full_mask(T::SCHEMA.len()) ^ full_mask(fields)
    );
    assert_eq!(decoded.recorded_fields_bytes().unwrap(), prefix.len());
    assert!(decoded.encode_fields(&mut Vec::new()).is_err());

    let delta = row.delta(None).unwrap().unwrap();
    let mut input = Decoder::new(&delta.values);
    for kind in &T::SCHEMA[..fields] {
        kind.decode_change(&mut input).unwrap();
    }
    bytes.clear();
    table.encode_key(row.key(), 0, &mut bytes).unwrap();
    bytes.extend_from_slice(&full_mask(fields).to_le_bytes()[..fields.div_ceil(8)]);
    bytes.extend_from_slice(&delta.values[..delta.values.len() - input.remaining()]);
    let mut input = Decoder::new(&bytes);
    input.format = format.version();
    input.recorded_fields = fields;
    let delta = decode_delta(table, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(delta.apply::<T>(None).unwrap(), decoded);
    decoded
}

#[test]
fn historical_snapshots_and_deltas_never_become_a_present_empty_vector() {
    let row = DivisionMovementRow {
        division: u64::MAX,
        combat_count: 3,
        combat_references: Some(packet()).into(),
        ..Default::default()
    };
    let decoded = historical_prefix(TableId::DivisionMovement, 19, &row);
    assert_eq!(decoded.combat_count, 3);
    assert_eq!(decoded.combat_references, Recorded::NotRecorded);
    let row = RailwayGunCounterInputRow {
        gun: u64::MAX,
        combat_count: 3,
        combat_references: Some(packet()).into(),
        ..Default::default()
    };
    let decoded = historical_prefix(TableId::RailwayGunCounterInputs, 28, &row);
    assert_eq!(decoded.combat_count, 3);
    assert_eq!(decoded.combat_references, Recorded::NotRecorded);
}
