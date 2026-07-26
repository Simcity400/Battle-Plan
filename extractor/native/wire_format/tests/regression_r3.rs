//! Regression tests for the two defects round 3 found.
//!
//! Each is the reviewer's failing input with the assertion inverted, plus the
//! invariant the defect violated — pinning the case alone would leave the
//! property it stands for untested.

use wire_format::record::Record;
use wire_format::schema::{Collection, Field, Quant, Schema, Shape};
use wire_format::value::{dequantize, escapes, quantize, stored_eq, Value};
use wire_format::{Kind, WireError};

/// §7.1 makes the writer compare against the **reconstructed** prior, so a
/// stored integer that does not survive reconstruction breaks the format: an
/// unchanged source reads as changed, the writer emits a delta, and the reader
/// rejects it as a mask bit on a field that did not change.
#[test]
fn a_quantization_that_is_not_a_fixed_point_escapes() {
    let q = Quant::Fixed(11);
    // Scales to exactly 2^53 by rounding up, then dequantizes to a float that
    // scales back to 2^53 - 1.
    let x = 90071.99254740993f64;
    assert!(escapes(x, q), "must escape rather than store an unstable integer");
    assert_eq!(quantize(x, q), None);

    let mut f = Field::new(1, Kind::Continuous);
    f.quant = q;
    // Escaped, so the reader reconstructs the exact f64 and the comparison holds.
    assert!(stored_eq(&f, Value::Continuous(x), Value::Continuous(x)));
}

/// The invariant, not just the one input: on the whole non-escaping domain,
/// `quantize(dequantize(q)) == q`.
#[test]
fn every_non_escaping_value_survives_reconstruction() {
    let mut checked = 0u64;
    for k in [0u8, 1, 2, 5, 11, 17, 22] {
        let q = Quant::Fixed(k);
        let scale = 10f64.powi(k as i32);
        // Ordinary magnitudes, then a sweep across the 2^53 boundary where the
        // property actually breaks down.
        let mut samples: Vec<f64> =
            vec![0.0, 1.0, 0.5, 0.333, -7.5, 1e-8, 12345.6789, 1e15, -1e15];
        for step in 0..64u64 {
            let near = (1u64 << 53) as f64 + step as f64 - 32.0;
            samples.push(near / scale);
            samples.push(-near / scale);
        }
        for v in samples {
            if !v.is_finite() || escapes(v, q) {
                continue;
            }
            let raw = quantize(v, q).expect("non-escaping");
            assert_eq!(
                quantize(dequantize(raw, q), q),
                Some(raw),
                "fixed({k}) is unstable at {v}: {raw} does not survive reconstruction"
            );
            checked += 1;
        }
    }
    for d in [1u8, 8, 16, 32] {
        let q = Quant::Ratio(d);
        for n in 0..=256u32 {
            let v = n as f64 / 256.0;
            if escapes(v, q) {
                continue;
            }
            let raw = quantize(v, q).expect("non-escaping");
            assert_eq!(quantize(dequantize(raw, q), q), Some(raw), "ratio({d}) at {v}");
            checked += 1;
        }
    }
    assert!(checked > 500, "the sweep must actually cover something, got {checked}");
}

/// §4: "a value the writer cannot map into 0..n-1 is a failed sample, never a
/// coerced code." The record path enforced it; the update path truncated to the
/// field's bit width and produced silently wrong data.
#[test]
fn an_out_of_range_enum_is_rejected_on_every_path() {
    let s = Schema {
        domain: 1,
        cadence_log2: 0,
        event_part: None,
        collections: vec![Collection {
            collection_id: 1,
            shape: Shape::KeyedSet,
            fields: vec![
                Field::new(1, Kind::Unsigned).key(0),
                Field::new(2, Kind::Enum).enum_card(3),
            ],
        }],
    };
    let c = &s.collections[0];
    let prior = Record::new(vec![Value::Enum(1)]);
    let bad = Record::new(vec![Value::Enum(4)]);

    let mut out = Vec::new();
    assert_eq!(
        bad.encode(c, &mut out),
        Err(WireError::Corrupt("enum code >= declared cardinality"))
    );
    let mut out = Vec::new();
    assert_eq!(
        wire_format::update::encode(c, &prior, &bad, &mut out),
        Err(WireError::Corrupt("enum code >= declared cardinality")),
        "the update path used to encode this as Enum(0)"
    );

    // An in-range code still works on both paths.
    let good = Record::new(vec![Value::Enum(2)]);
    let mut out = Vec::new();
    wire_format::update::encode(c, &prior, &good, &mut out).unwrap();
    let (back, _) = wire_format::update::decode(c, &prior, &out, 0).unwrap();
    assert_eq!(back.values, vec![Value::Enum(2)]);
}

/// The encoder rejects ordered shapes as out of scope; the decoder read them
/// with keyed-set rules, misdecoding §7.4's absolute keys and panicking on an
/// occurrence list that has no key fields at all.
#[test]
fn the_baseline_decoder_rejects_shapes_the_encoder_will_not_write() {
    let of = |shape| Schema {
        domain: 1,
        cadence_log2: 0,
        event_part: None,
        collections: vec![Collection {
            collection_id: 1,
            shape,
            fields: vec![Field::new(1, Kind::Unsigned).key(0), Field::new(2, Kind::Unsigned)],
        }],
    };
    // Absolute keys 5 and 6 were read as 5 and 11.
    assert_eq!(
        wire_format::body::decode_baseline(&of(Shape::OrderedKeyedList), &[2, 5, 1, 6, 2], 0),
        Err(WireError::Corrupt("ordered collections have no keyed-set baseline grammar"))
    );
    // No key fields, so the key reader indexed an empty list and panicked.
    let occurrence = Schema {
        domain: 1,
        cadence_log2: 0,
        event_part: None,
        collections: vec![Collection {
            collection_id: 1,
            shape: Shape::OrderedOccurrenceList,
            fields: vec![Field::new(2, Kind::Unsigned)],
        }],
    };
    assert_eq!(
        wire_format::body::decode_baseline(&occurrence, &[1, 7], 0),
        Err(WireError::Corrupt("ordered collections have no keyed-set baseline grammar"))
    );
}

/// `domain` and `branch` are varints on the wire and `u32` in the header, so an
/// over-range value must be rejected rather than truncated onto a valid identity.
#[test]
fn an_over_range_block_identity_is_rejected_not_aliased() {
    use wire_format::frame::BlockHeader;
    let mut bytes = Vec::new();
    wire_format::varint::write_varint(&mut bytes, 1u64 << 32); // domain
    wire_format::varint::write_varint(&mut bytes, 0); // branch
    bytes.extend_from_slice(&1u64.to_le_bytes()); // schema_hash
    wire_format::varint::write_varint(&mut bytes, 0); // first_tick
    assert_eq!(
        BlockHeader::decode(&bytes, 0),
        Err(WireError::Corrupt("block domain or branch exceeds u32")),
        "domain 2^32 used to decode as domain 0"
    );

    // The largest legal identity still round-trips.
    let mut ok = Vec::new();
    BlockHeader { domain: u32::MAX, branch: u32::MAX, schema_hash: 7, first_tick: 0 }
        .encode(&mut ok);
    assert_eq!(BlockHeader::decode(&ok, 0).unwrap().0.domain, u32::MAX);
}

/// §6's whole purpose, end to end: a value that falls by one every tick has a
/// constant stored value, so an entire run of ticks collapses to `UNCHANGED`.
/// Before the transform existed, each tick emitted a changed row.
#[test]
fn a_derived_countdown_collapses_to_one_unchanged_run() {
    use wire_format::body::{diff, key, KeyedState};
    use wire_format::derived::{row_from_stored, row_to_stored};
    use wire_format::schema::Derived;

    let mut counter = Field::new(2, Kind::Unsigned);
    counter.derived = Some(Derived { c: 1, u: 0 });
    let s = Schema {
        domain: 1,
        cadence_log2: 0,
        event_part: None,
        collections: vec![Collection {
            collection_id: 1,
            shape: Shape::KeyedSet,
            fields: vec![Field::new(1, Kind::Unsigned).key(0), counter],
        }],
    };
    s.validate(3).expect("a derived unsigned field is legal");
    let c = &s.collections[0];
    let fields: Vec<&Field> = c.value_fields().collect();

    // A siege timer ticking 100, 99, 98 ... down over ten ticks.
    let states: Vec<KeyedState> = (0..10u64)
        .map(|t| {
            let stored = row_to_stored(&fields, &[Value::Unsigned(100 - t)], t).unwrap();
            let mut st = KeyedState::new();
            st.insert(key(&[1]), Record::new(stored));
            st
        })
        .collect();

    for pair in states.windows(2) {
        assert!(
            diff(c, &pair[0], &pair[1]).changed.is_empty(),
            "a clock-linear countdown must not produce a delta"
        );
    }

    // Every tick encodes to the same bytes, which is what makes the run legal.
    let mut first = Vec::new();
    states[0][&key(&[1])].encode(c, &mut first).unwrap();
    for (t, st) in states.iter().enumerate() {
        let mut bytes = Vec::new();
        st[&key(&[1])].encode(c, &mut bytes).unwrap();
        assert_eq!(bytes, first, "tick {t} must encode identically");
    }
    assert_eq!(first, vec![0xC8, 0x01], "zigzag(100) = 200");

    // And the reader recovers the live value for whichever tick it asks about.
    let (decoded, _) = Record::decode(c, &first, 0).unwrap();
    for t in 0..10u64 {
        let live = row_from_stored(&fields, &decoded.values, t).unwrap();
        assert_eq!(live, vec![Value::Unsigned(100 - t)], "live value at tick {t}");
    }
}
