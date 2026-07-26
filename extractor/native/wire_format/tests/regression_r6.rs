//! Regression tests for round 6. Three of the four findings were the same shape:
//! a rule enforced in the encoder but missing from the shared `validate`, so an
//! update — which touches only its changed fields — carried the violation past.

use wire_format::body::{key, KeyedState};
use wire_format::columnar::encode_baseline_columnar;
use wire_format::record::Record;
use wire_format::schema::{Collection, Field, Schema, Shape};
use wire_format::value::Value;
use wire_format::{Kind, WireError};

fn one(fields: Vec<Field>) -> Schema {
    Schema {
        domain: 1,
        cadence_log2: 0,
        event_part: None,
        collections: vec![Collection { collection_id: 1, shape: Shape::KeyedSet, fields }],
    }
}

/// Every arm must reject the same records. An out-of-range enum sitting on an
/// *unchanged* field slipped through `update::encode`, and `update::decode`
/// then carried it into reconstructed state.
#[test]
fn an_out_of_range_enum_on_an_unchanged_field_is_rejected() {
    let s = one(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Enum).enum_card(3),
        Field::new(3, Kind::Boolean),
    ]);
    let c = &s.collections[0];
    let prior = Record::new(vec![Value::Enum(5), Value::Bool(false)]);
    let next = Record::new(vec![Value::Enum(5), Value::Bool(true)]);

    assert_eq!(
        next.validate(c),
        Err(WireError::Corrupt("enum code >= declared cardinality"))
    );
    let mut out = Vec::new();
    assert_eq!(
        wire_format::update::encode(c, &prior, &next, &mut out),
        Err(WireError::Corrupt("enum code >= declared cardinality")),
        "the enum only changed on a field the update never touches"
    );

    // In range, on an unchanged field, still works.
    let prior = Record::new(vec![Value::Enum(2), Value::Bool(false)]);
    let next = Record::new(vec![Value::Enum(2), Value::Bool(true)]);
    let mut out = Vec::new();
    wire_format::update::encode(c, &prior, &next, &mut out).unwrap();
    let (back, _) = wire_format::update::decode(c, &prior, &out, 0).unwrap();
    assert_eq!(back.values, next.values);
}

/// §4 gives a monotone counter an unsigned absolute form; the zigzag difference
/// is a delta *encoding*, not a value a record holds. `matches_kind` granted
/// `Signed` for `Kind::Monotone`, and once `validate` became its own consumer
/// that grant let one past on an unchanged field.
#[test]
fn a_signed_value_is_not_valid_on_a_monotone_field() {
    let s = one(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Monotone),
        Field::new(3, Kind::Boolean),
    ]);
    let c = &s.collections[0];
    let bad = Record::new(vec![Value::Signed(5), Value::Bool(false)]);
    assert_eq!(bad.validate(c), Err(WireError::Corrupt("value does not match field kind")));

    let next = Record::new(vec![Value::Signed(5), Value::Bool(true)]);
    let mut out = Vec::new();
    assert_eq!(
        wire_format::update::encode(c, &bad, &next, &mut out),
        Err(WireError::Corrupt("value does not match field kind"))
    );

    // Unsigned is the monotone live form and still round-trips through a delta.
    let prior = Record::new(vec![Value::Unsigned(5), Value::Bool(false)]);
    let next = Record::new(vec![Value::Unsigned(9), Value::Bool(true)]);
    let mut out = Vec::new();
    wire_format::update::encode(c, &prior, &next, &mut out).unwrap();
    let (back, _) = wire_format::update::decode(c, &prior, &out, 0).unwrap();
    assert_eq!(back.values, next.values);
}

/// The columnar arm implements the keyed-set grammar only, exactly as the
/// row-major one does — it panicked on an occurrence list and silently used the
/// wrong key grammar for an ordered keyed list.
#[test]
fn the_columnar_arm_rejects_ordered_shapes() {
    for shape in [Shape::OrderedKeyedList, Shape::OrderedOccurrenceList] {
        let fields = match shape {
            Shape::OrderedOccurrenceList => vec![Field::new(2, Kind::Unsigned)],
            _ => vec![Field::new(1, Kind::Unsigned).key(0), Field::new(2, Kind::Unsigned)],
        };
        let s = Schema {
            domain: 1,
            cadence_log2: 0,
            event_part: None,
            collections: vec![Collection { collection_id: 1, shape, fields }],
        };
        let c = &s.collections[0];
        let mut state = KeyedState::new();
        let k = if matches!(shape, Shape::OrderedOccurrenceList) { vec![] } else { vec![1u64] };
        state.insert(key(&k), Record::new(vec![Value::Unsigned(7)]));
        let mut out = Vec::new();
        assert_eq!(
            encode_baseline_columnar(c, &state, &mut out),
            Err(WireError::Corrupt("ordered collections are not representable as a keyed state")),
            "{shape:?} must be refused, not panicked on or mis-encoded"
        );
    }
}

/// The property behind all three: `validate` and `encode` must agree on every
/// record, so no arm can accept what another rejects.
#[test]
fn validate_and_encode_agree_on_every_record() {
    let s = one(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Enum).enum_card(3),
        Field::new(3, Kind::Monotone),
        Field::new(4, Kind::Unsigned).optional().nullable(),
    ]);
    let c = &s.collections[0];

    let variants = [
        Value::Absent,
        Value::Null,
        Value::Unknown,
        Value::Unsigned(1),
        Value::Signed(-1),
        Value::Enum(0),
        Value::Enum(9),
        Value::Bool(true),
        Value::ContentId(3),
        Value::Continuous(1.0),
    ];
    let mut checked = 0;
    for a in variants {
        for b in variants {
            for d in variants {
                let r = Record::new(vec![a, b, d]);
                let valid = r.validate(c).is_ok();
                let mut out = Vec::new();
                let encoded = r.encode(c, &mut out).is_ok();
                assert_eq!(valid, encoded, "disagree on {:?}", r.values);
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 1000);
}
