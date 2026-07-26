//! Regression tests for the two defects round 5 found. Both are round-4 fixes
//! that were applied too narrowly — the lesson each encodes is coverage, so each
//! test sweeps the whole space rather than the one input the reviewer named.

use wire_format::body::{key, KeyedState};
use wire_format::columnar::encode_baseline_columnar;
use wire_format::derived::{from_stored, to_stored};
use wire_format::record::Record;
use wire_format::schema::{Collection, Derived, Field, Schema, Shape};
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

/// The round-4 fix gated the live variant with `matches_kind`, which blesses
/// `Value::Signed` for `Kind::Monotone` — right for a *stored* monotone value,
/// wrong for a live one. Capture accepted `Signed(-5)` on a derived monotone
/// field, encoded it, and replay then refused the crate's own bytes.
///
/// The round-4 test covered only `Unsigned` and `Signed`; this one covers every
/// derived-capable kind, which is the actual property.
#[test]
fn the_capture_boundary_is_a_bijection_for_every_derived_kind() {
    for (kind, live_ok, live_wrong) in [
        (Kind::Unsigned, Value::Unsigned(5), Value::Signed(5)),
        (Kind::Monotone, Value::Unsigned(5), Value::Signed(5)),
        (Kind::Signed, Value::Signed(5), Value::Unsigned(5)),
    ] {
        let mut f = Field::new(2, kind);
        f.derived = Some(Derived { c: 1, u: 0 });

        // The declared live variant round-trips exactly.
        let stored = to_stored(&f, live_ok, 3).unwrap_or_else(|e| panic!("{kind:?}: {e:?}"));
        assert!(matches!(stored, Value::Signed(_)), "{kind:?} stores Signed, got {stored:?}");
        assert_eq!(from_stored(&f, stored, 3), Ok(live_ok), "{kind:?} must invert");

        // Any other variant is refused before it can reach the wire.
        assert_eq!(
            to_stored(&f, live_wrong, 3),
            Err(WireError::Corrupt("derived live value does not match the field's kind")),
            "{kind:?} must reject {live_wrong:?}"
        );
    }
}

/// Round 4 shared the per-value rules with the columnar arm but left §3 record
/// validity and §7.4 key arity unshared, and they had drifted: a `Null` on a
/// non-nullable field was silently dropped, leaving a column with fewer values
/// than rows and nothing marking which; a wrong-arity record or key panicked.
#[test]
fn the_columnar_arm_enforces_record_and_key_validity() {
    let s = one(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Unsigned).key(1),
        Field::new(3, Kind::Unsigned), // required, not nullable, not unknown-capable
    ]);
    let c = &s.collections[0];
    let encode = |k: Vec<u64>, values: Vec<Value>| {
        let mut state = KeyedState::new();
        state.insert(
            key(&k.iter().copied().collect::<Vec<_>>()),
            Record::new(values),
        );
        let mut out = Vec::new();
        encode_baseline_columnar(c, &state, &mut out).map(|_| out)
    };

    // The three §3 states this schema forbids — silently dropped before.
    assert_eq!(
        encode(vec![1, 1], vec![Value::Null]),
        Err(WireError::Corrupt("null on a non-nullable field"))
    );
    assert_eq!(
        encode(vec![1, 1], vec![Value::Unknown]),
        Err(WireError::Corrupt(
            "unknown on a field the schema does not declare unknown-capable"
        ))
    );
    assert_eq!(
        encode(vec![1, 1], vec![Value::Absent]),
        Err(WireError::Corrupt("required field is absent"))
    );
    // Wrong record arity — panicked before.
    assert_eq!(
        encode(vec![1, 1], vec![Value::Unsigned(1), Value::Unsigned(2)]),
        Err(WireError::Corrupt("record arity does not match schema"))
    );
    // Wrong key arity — panicked one way, emitted unparseable bytes the other.
    assert_eq!(
        encode(vec![1], vec![Value::Unsigned(1)]),
        Err(WireError::Corrupt("key arity does not match schema"))
    );
    assert_eq!(
        encode(vec![1, 1, 1], vec![Value::Unsigned(1)]),
        Err(WireError::Corrupt("key arity does not match schema"))
    );

    // A valid row still encodes.
    assert!(encode(vec![1, 1], vec![Value::Unsigned(7)]).is_ok());
}

/// Both arms must reject exactly the same records, or the measurement compares
/// two different formats.
#[test]
fn both_arms_agree_on_what_is_a_valid_record() {
    let s = one(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Unsigned).nullable(),
        Field::new(3, Kind::Enum).enum_card(3),
        Field::new(4, Kind::Unsigned).optional(),
    ]);
    let c = &s.collections[0];

    let cases = vec![
        vec![Value::Unsigned(1), Value::Enum(0), Value::Absent],
        vec![Value::Null, Value::Enum(2), Value::Unsigned(9)],
        vec![Value::Unsigned(1), Value::Enum(3), Value::Absent], // enum out of range
        vec![Value::Unsigned(1), Value::Null, Value::Absent],    // null on non-nullable
        vec![Value::Unknown, Value::Enum(0), Value::Absent],     // unknown, not capable
        vec![Value::Unsigned(1), Value::Enum(0)],                // short arity
    ];
    for values in cases {
        let record = Record::new(values.clone());
        let mut row_bytes = Vec::new();
        let row = record.encode(c, &mut row_bytes);

        let mut state = KeyedState::new();
        state.insert(key(&[1]), record);
        let mut col_bytes = Vec::new();
        let col = encode_baseline_columnar(c, &state, &mut col_bytes);

        assert_eq!(
            row.is_ok(),
            col.is_ok(),
            "arms disagree on {values:?}: row {row:?}, columnar {col:?}"
        );
    }
}

/// An update routes only *changed* fields through the value encoder, so a §3
/// violation sitting on an unchanged field reached the wire while
/// `Record::encode` rejected the identical record. Both records are now
/// validated in full.
#[test]
fn an_update_validates_both_records_not_just_the_changed_fields() {
    let mut d = Field::new(2, Kind::Unsigned);
    d.derived = Some(Derived { c: 0, u: 0 });
    let s = one(vec![
        Field::new(1, Kind::Unsigned).key(0),
        d,
        Field::new(3, Kind::Boolean),
    ]);
    let c = &s.collections[0];

    // Unsigned(10) is the derived field's LIVE variant, not its stored one, and
    // it never changes across this transition.
    let prior = Record::new(vec![Value::Unsigned(10), Value::Bool(false)]);
    let next = Record::new(vec![Value::Unsigned(10), Value::Bool(true)]);
    let mut reject = Vec::new();
    assert!(next.encode(c, &mut reject).is_err(), "the record path rejects it");
    let mut out = Vec::new();
    assert_eq!(
        wire_format::update::encode(c, &prior, &next, &mut out),
        Err(WireError::Corrupt("value does not match field kind")),
        "the update path must reject it too"
    );

    // The valid form of the same transition still encodes and round-trips.
    let prior = Record::new(vec![Value::Signed(10), Value::Bool(false)]);
    let next = Record::new(vec![Value::Signed(10), Value::Bool(true)]);
    let mut out = Vec::new();
    wire_format::update::encode(c, &prior, &next, &mut out).unwrap();
    let (back, _) = wire_format::update::decode(c, &prior, &out, 0).unwrap();
    assert_eq!(back.values, next.values);
}

/// §4's monotone difference is modulo 2^64, because both endpoints are `u64` and
/// their mathematical difference need not fit an `i64`. The rule is now stated;
/// this pins that every pair of valid endpoints round-trips exactly.
#[test]
fn a_monotone_delta_round_trips_across_the_whole_u64_range() {
    let s = one(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Monotone),
    ]);
    let c = &s.collections[0];

    let extremes = [
        (0u64, u64::MAX),
        (u64::MAX, 0u64),
        (0, 1),
        (5, 3),
        (i64::MAX as u64, i64::MAX as u64 + 2),
        (u64::MAX / 2, u64::MAX),
        (1 << 63, 0),
    ];
    for (p, v) in extremes {
        let prior = Record::new(vec![Value::Unsigned(p)]);
        let next = Record::new(vec![Value::Unsigned(v)]);
        let mut out = Vec::new();
        wire_format::update::encode(c, &prior, &next, &mut out)
            .unwrap_or_else(|e| panic!("{p} -> {v}: {e:?}"));
        let (back, pos) = wire_format::update::decode(c, &prior, &out, 0).unwrap();
        assert_eq!(pos, out.len(), "{p} -> {v}: exact byte consumption");
        assert_eq!(back.values, vec![Value::Unsigned(v)], "{p} -> {v} must round-trip");
    }
}
