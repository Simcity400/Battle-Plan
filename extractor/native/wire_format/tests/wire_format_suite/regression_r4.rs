use wire_format::derived::{from_stored, to_stored};
use wire_format::record::Record;
use wire_format::schema::{Collection, Derived, Field, Quant, Schema, Shape};
use wire_format::value::{dequantize, quantize, Value};
use wire_format::{Kind, WireError};

fn one(fields: Vec<Field>) -> Schema {
    Schema {
        domain: 1,
        cadence_log2: 0,
        event_part: None,
        collections: vec![Collection {
            collection_id: 1,
            shape: Shape::KeyedSet,
            fields,
        }],
    }
}

#[test]
fn a_derived_unsigned_field_can_store_a_negative_value() {
    let mut f = Field::new(2, Kind::Unsigned);
    f.derived = Some(Derived { c: -1, u: 0 });
    let s = one(vec![Field::new(1, Kind::Unsigned).key(0), f.clone()]);
    s.validate(3).expect("c = -1 is a legal declaration");

    assert_eq!(to_stored(&f, Value::Unsigned(5), 10), Ok(Value::Signed(-5)));
    assert_eq!(
        from_stored(&f, Value::Signed(-5), 10),
        Ok(Value::Unsigned(5))
    );

    let c = &s.collections[0];
    let mut bytes = Vec::new();
    Record::new(vec![Value::Signed(-5)])
        .encode(c, &mut bytes)
        .unwrap();
    assert_eq!(bytes, vec![0x09], "zigzag(-5)");
    assert_eq!(
        Record::decode(c, &bytes, 0).unwrap().0.values,
        vec![Value::Signed(-5)]
    );

    assert_eq!(
        from_stored(&f, Value::Signed(-11), 10),
        Err(WireError::Corrupt(
            "derived live value is negative on an unsigned field"
        ))
    );
}

#[test]
fn a_derived_stored_value_has_one_representation() {
    let mut f = Field::new(2, Kind::Monotone);
    f.derived = Some(Derived { c: 1, u: 0 });
    let s = one(vec![Field::new(1, Kind::Unsigned).key(0), f.clone()]);
    let c = &s.collections[0];

    assert_eq!(to_stored(&f, Value::Unsigned(10), 0), Ok(Value::Signed(10)));
    assert_eq!(
        to_stored(&f, Value::Signed(10), 0),
        Err(WireError::Corrupt(
            "derived live value does not match the field's kind"
        ))
    );

    let mut bytes = Vec::new();
    Record::new(vec![Value::Signed(10)])
        .encode(c, &mut bytes)
        .unwrap();
    let (decoded, _) = Record::decode(c, &bytes, 0).unwrap();
    assert_eq!(decoded.values, vec![Value::Signed(10)]);

    let next = Record::new(vec![to_stored(&f, Value::Unsigned(10), 0).unwrap()]);
    let mut out = Vec::new();
    assert_eq!(
        wire_format::update::encode(c, &decoded, &next, &mut out),
        Err(WireError::Corrupt(
            "update row with an all-zero change mask"
        )),
        "nothing changed, so there is no update row"
    );

    let mut bad = Vec::new();
    assert_eq!(
        Record::new(vec![Value::Unsigned(10)]).encode(c, &mut bad),
        Err(WireError::Corrupt("value does not match field kind"))
    );
}

#[test]
fn the_reader_rejects_an_unescaped_unstable_integer() {
    let q = Quant::Fixed(11);
    let unstable = 1i64 << 53;
    assert_eq!(
        dequantize(unstable, q),
        dequantize(unstable - 1, q),
        "premise: two integers, one reconstructed value"
    );

    let s = one(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Continuous).quant(q),
    ]);
    let c = &s.collections[0];
    let mut bytes = vec![0x00];
    wire_format::varint::write_zigzag(&mut bytes, unstable);
    assert_eq!(
        Record::decode(c, &bytes, 0),
        Err(WireError::Corrupt(
            "stored integer is not its own reconstruction's quantization"
        ))
    );

    let mut ok = vec![0x00];
    wire_format::varint::write_zigzag(&mut ok, unstable - 1);
    let (back, _) = Record::decode(c, &ok, 0).unwrap();
    assert_eq!(
        back.values,
        vec![Value::Continuous(dequantize(unstable - 1, q))]
    );
    assert_eq!(quantize(dequantize(unstable - 1, q), q), Some(unstable - 1));
}

#[test]
fn dequantize_is_a_division_not_a_reciprocal_multiply() {
    let k = 11u8;
    let q = Quant::Fixed(k);
    let m = 1e11f64;

    let mut divergent = Vec::new();
    for delta in 0..40_000i64 {
        let raw = (1i64 << 53) - 20_000 + delta;
        let by_division = (raw as f64) / m;
        let by_reciprocal = (raw as f64) * (1.0 / m);
        if by_division != by_reciprocal {
            divergent.push(raw);
        }
    }
    assert!(
        !divergent.is_empty(),
        "the two readings must actually differ, or this test proves nothing"
    );

    for raw in divergent.iter().take(64) {
        assert_eq!(
            dequantize(*raw, q),
            (*raw as f64) / m,
            "dequantize must be a single division by the multiplier"
        );
    }
}

#[test]
fn a_frame_tick_too_large_for_i64_is_a_hard_error() {
    let mut f = Field::new(2, Kind::Unsigned);
    f.derived = Some(Derived { c: 1, u: 0 });
    assert_eq!(
        to_stored(&f, Value::Unsigned(1), u64::MAX),
        Err(WireError::Corrupt("derived clock term overflows i64")),
        "u64::MAX used to cast to -1 and return Ok(Signed(0))"
    );
    assert_eq!(
        to_stored(&f, Value::Unsigned(1), i64::MAX as u64),
        Err(WireError::Corrupt("derived stored value overflows i64"))
    );
    assert_eq!(
        to_stored(&f, Value::Unsigned(1), 1 << 40),
        Ok(Value::Signed((1 << 40) + 1))
    );
}

#[test]
fn the_capture_boundary_rejects_a_wrong_live_variant() {
    let mut f = Field::new(2, Kind::Unsigned);
    f.derived = Some(Derived { c: -1, u: 0 });
    assert_eq!(
        to_stored(&f, Value::Signed(-5), 1),
        Err(WireError::Corrupt(
            "derived live value does not match the field's kind"
        )),
        "an unsigned field's live value cannot be Signed"
    );
    assert_eq!(to_stored(&f, Value::Unsigned(5), 1), Ok(Value::Signed(4)));

    let mut signed = Field::new(2, Kind::Signed);
    signed.derived = Some(Derived { c: -1, u: 0 });
    assert_eq!(
        to_stored(&signed, Value::Signed(-5), 1),
        Ok(Value::Signed(-6))
    );
    assert_eq!(
        to_stored(&signed, Value::Unsigned(5), 1),
        Err(WireError::Corrupt(
            "derived live value does not match the field's kind"
        ))
    );
}

#[test]
fn the_columnar_encoder_enforces_the_same_value_rules() {
    use wire_format::body::{key, KeyedState};
    use wire_format::columnar::encode_baseline_columnar;

    let s = one(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Enum).enum_card(3),
    ]);
    let c = &s.collections[0];
    let mut state = KeyedState::new();
    state.insert(key(&[1]), Record::new(vec![Value::Enum(4)]));
    let mut out = Vec::new();
    assert_eq!(
        encode_baseline_columnar(c, &state, &mut out),
        Err(WireError::Corrupt("enum code >= declared cardinality")),
        "used to truncate Enum(4) to Enum(0)"
    );

    let mut d = Field::new(2, Kind::Unsigned);
    d.derived = Some(Derived { c: 1, u: 0 });
    let s = one(vec![Field::new(1, Kind::Unsigned).key(0), d]);
    let c = &s.collections[0];
    let mut state = KeyedState::new();
    state.insert(key(&[1]), Record::new(vec![Value::Signed(-1)]));
    let mut out = Vec::new();
    assert!(encode_baseline_columnar(c, &state, &mut out).is_ok());
}

#[test]
fn blank_record_is_encodable_for_a_derived_field() {
    let mut d = Field::new(2, Kind::Unsigned);
    d.derived = Some(Derived { c: 1, u: 0 });
    let s = one(vec![Field::new(1, Kind::Unsigned).key(0), d]);
    let c = &s.collections[0];
    let blank = wire_format::body::blank_record(c);
    assert_eq!(blank.values, vec![Value::Signed(0)]);
    let mut out = Vec::new();
    blank
        .encode(c, &mut out)
        .expect("a blank record must encode");
}
