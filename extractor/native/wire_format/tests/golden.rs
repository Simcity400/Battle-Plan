//! Golden byte vectors.
//!
//! `WIRE_FORMAT.md` §13 lists these as the thing that turns "two implementers
//! produce identical bytes" from a claim the document makes into one it proves.
//! Each vector pins an encoding the specification argues for in prose; if a
//! refactor changes any byte here, either the specification changed or the
//! implementation drifted, and the diff says which.
//!
//! Vectors are written as explicit byte arrays, never as `encode(decode(x))`,
//! because a round-trip test passes just as happily against a wrong encoding.

use wire_format::body::{
    apply, apply_shaped, blank_record, check_minimal, decode_baseline, decode_delta,
    decode_shaped_baseline, decode_shaped_delta, diff, encode_baseline, encode_delta,
    encode_shaped_baseline, encode_shaped_delta, key, CollectionDelta, CollectionState,
    KeyedDelta, KeyedState, MinimalityError, Row,
};
use wire_format::frame::{pack_tag, read_block, BlockHeader, BlockWriter, Frame, Kind};
use wire_format::record::Record;
use wire_format::schema::{Collection, EventPart, Field, Quant, Schema, SchemaError, Shape};
use wire_format::value::Value;
use wire_format::{
    check_frontier, check_frontier_transition, decode_event_body, encode_event_body, EventBatch,
    FrontierDiscontinuity, FrontierOutcome, Kind as FieldKind, TickGrid,
};

/// Pinned d20 schema hash. See `d20_schema_hash_is_pinned`.
const D20_SCHEMA_HASH: u64 = 8669914881139233020;

/// A deliberately small schema, so a vector is readable byte by byte.
fn tiny() -> Schema {
    Schema {
        domain: 1,
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: 1,
            shape: Shape::KeyedSet,
            fields: vec![
                Field::new(1, FieldKind::Unsigned).key(0),
                Field::new(2, FieldKind::Unsigned),               // required
                Field::new(3, FieldKind::Boolean),                // required, bit-packed
                Field::new(4, FieldKind::Unsigned).optional(),    // optional
            ],
        }],
        event_part: None,
    }
}

fn tiny_row(a: u64, flag: bool, opt: Option<u64>) -> Record {
    Record::new(vec![
        Value::Unsigned(a),
        Value::Bool(flag),
        opt.map(Value::Unsigned).unwrap_or(Value::Absent),
    ])
}

#[test]
fn vector_record_presence_and_bit_packing() {
    let schema = tiny();
    let c = &schema.collections[0];

    // One optional field present, boolean true, values 7 and 9.
    let mut bytes = Vec::new();
    tiny_row(7, true, Some(9)).encode(c, &mut bytes).unwrap();
    assert_eq!(
        bytes,
        vec![
            0b0000_0001, // presence: 1 optional field, present
            0b0000_0001, // bit-packed region: boolean true, zero-padded
            7,           // required unsigned
            9,           // optional unsigned
        ],
        "presence bitmap, then bit-packed region, then value bytes"
    );

    // Same row with the optional absent: the presence bit clears and the value
    // disappears, but nothing else moves.
    let mut absent = Vec::new();
    tiny_row(7, true, None).encode(c, &mut absent).unwrap();
    assert_eq!(absent, vec![0b0000_0000, 0b0000_0001, 7]);
}

#[test]
fn vector_absent_is_not_zero() {
    let schema = tiny();
    let c = &schema.collections[0];
    let mut absent = Vec::new();
    tiny_row(1, false, None).encode(c, &mut absent).unwrap();
    let mut zero = Vec::new();
    tiny_row(1, false, Some(0)).encode(c, &mut zero).unwrap();
    assert_ne!(absent, zero, "absent and a captured zero are different bytes");
    assert_eq!(absent, vec![0x00, 0x00, 1]);
    assert_eq!(zero, vec![0x01, 0x00, 1, 0]);
}

#[test]
fn vector_baseline_body() {
    let schema = tiny();
    let c = &schema.collections[0];
    let mut state = KeyedState::new();
    state.insert(key(&[10]), tiny_row(1, false, None));
    state.insert(key(&[12]), tiny_row(2, true, None));

    let mut bytes = Vec::new();
    encode_baseline(&schema, &[state.clone()], &mut bytes).unwrap();
    assert_eq!(
        bytes,
        vec![
            2,    // row count
            10,   // first key, absolute
            0x00, // presence: optional absent
            0x00, // bit-packed: false
            1,    // required unsigned
            2,    // second key: unsigned difference from 10
            0x00, 0x01, // presence, then boolean true
            2,    // required unsigned
        ],
        "the second key is a difference, not an absolute"
    );

    let (back, pos) = decode_baseline(&schema, &bytes, 0).unwrap();
    assert_eq!(pos, bytes.len());
    assert_eq!(back[0], state);
    let _ = c;
}

#[test]
fn vector_delta_body_with_change_bitmap() {
    let schema = tiny();
    let mut prior = KeyedState::new();
    prior.insert(key(&[1]), tiny_row(1, false, None));
    prior.insert(key(&[2]), tiny_row(2, false, None));

    let mut next = prior.clone();
    next.insert(key(&[2]), tiny_row(99, false, None)); // modified
    next.remove(&key(&[1])); // vanished

    let delta = diff(&schema.collections[0], &prior, &next);
    check_minimal(&schema.collections[0], &prior, &next, &delta).unwrap();

    let mut bytes = Vec::new();
    encode_delta(&schema, std::slice::from_ref(&prior), &[Some(delta.clone())], &mut bytes)
        .unwrap();
    // Key 2 exists in the prior state, so this is an UPDATE (§7.2) and carries a
    // change mask plus only the changed field — not a full record.
    assert_eq!(
        bytes,
        vec![
            0b0000_0001, // collection change bitmap: the one collection changed
            1,           // changed row count
            2,           // key 2, absolute (first in its list)
            0b0000_0001, // change mask: only field index 0 moved
            // No bit-packed byte: the changed field is a varint, so that region
            // has zero bits and therefore occupies zero bytes.
            99,          // the new value
            1,           // tombstone count
            1,           // key 1, absolute
        ],
        "an update carries a change mask, not the insert grammar"
    );

    let (back, pos) = decode_delta(&schema, std::slice::from_ref(&prior), &bytes, 0).unwrap();
    assert_eq!(pos, bytes.len());
    assert_eq!(back[0].as_ref().unwrap(), &delta);

    let mut replayed = prior.clone();
    apply(&mut replayed, back[0].as_ref().unwrap());
    assert_eq!(replayed, next, "the delta reconstructs the next state exactly");
}

#[test]
fn vector_ordered_keyed_baseline_and_delta_replacement() {
    let schema = Schema {
        domain: 1,
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: 1,
            shape: Shape::OrderedKeyedList,
            fields: vec![
                Field::new(1, FieldKind::Unsigned).key(0),
                Field::new(2, FieldKind::Unsigned),
            ],
        }],
        event_part: None,
    };
    let next = CollectionState::OrderedKeyed(vec![
        Row { key: key(&[5]), record: Record::new(vec![Value::Unsigned(7)]) },
        Row { key: key(&[2]), record: Record::new(vec![Value::Unsigned(8)]) },
    ]);
    let expected_baseline = vec![
        2, // row count
        5, 7, // absolute key 5, full non-key record
        2, 8, // absolute key 2 (not a difference), full non-key record
    ];
    let mut baseline = Vec::new();
    encode_shaped_baseline(&schema, std::slice::from_ref(&next), &mut baseline).unwrap();
    assert_eq!(baseline, expected_baseline);
    assert_eq!(decode_shaped_baseline(&schema, &expected_baseline, 0).unwrap().0[0], next);

    let prior = CollectionState::OrderedKeyed(vec![Row {
        key: key(&[1]),
        record: Record::new(vec![Value::Unsigned(1)]),
    }]);
    let expected_delta = vec![
        0x01, // collection-change bitmap
        2, 5, 7, 2, 8, // byte-identical baseline contribution
    ];
    let mut delta = Vec::new();
    encode_shaped_delta(
        &schema,
        std::slice::from_ref(&prior),
        &[Some(CollectionDelta::OrderedKeyed(match &next {
            CollectionState::OrderedKeyed(rows) => rows.clone(),
            _ => unreachable!(),
        }))],
        &mut delta,
    )
    .unwrap();
    assert_eq!(delta, expected_delta);
    let expected_replacement = CollectionDelta::OrderedKeyed(match &next {
        CollectionState::OrderedKeyed(rows) => rows.clone(),
        _ => unreachable!(),
    });
    let (decoded, pos) =
        decode_shaped_delta(&schema, std::slice::from_ref(&prior), &expected_delta, 0).unwrap();
    assert_eq!(pos, expected_delta.len());
    assert_eq!(
        decoded,
        vec![Some(expected_replacement)],
        "absolute keys and source order survive decoding"
    );
    let mut replayed = prior;
    apply_shaped(&mut replayed, decoded[0].as_ref().unwrap()).unwrap();
    assert_eq!(replayed, next, "the replacement reconstructs the next list");
}

#[test]
fn vector_ordered_occurrence_baseline_and_delta_replacement() {
    let schema = Schema {
        domain: 1,
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: 1,
            shape: Shape::OrderedOccurrenceList,
            fields: vec![Field::new(1, FieldKind::Unsigned)],
        }],
        event_part: None,
    };
    let rows = vec![
        Record::new(vec![Value::Unsigned(9)]),
        Record::new(vec![Value::Unsigned(3)]),
        Record::new(vec![Value::Unsigned(9)]),
    ];
    let next = CollectionState::OrderedOccurrence(rows.clone());
    let expected_baseline = vec![
        3, // row count
        9, 3, 9, // all fields of each occurrence, source order, duplicate retained
    ];
    let mut baseline = Vec::new();
    encode_shaped_baseline(&schema, std::slice::from_ref(&next), &mut baseline).unwrap();
    assert_eq!(baseline, expected_baseline);
    assert_eq!(decode_shaped_baseline(&schema, &expected_baseline, 0).unwrap().0[0], next);

    let prior = CollectionState::OrderedOccurrence(vec![Record::new(vec![Value::Unsigned(1)])]);
    let expected_delta = vec![
        0x01, // collection-change bitmap
        3, 9, 3, 9, // byte-identical baseline contribution
    ];
    let mut delta = Vec::new();
    encode_shaped_delta(
        &schema,
        std::slice::from_ref(&prior),
        &[Some(CollectionDelta::OrderedOccurrence(rows))],
        &mut delta,
    )
    .unwrap();
    assert_eq!(delta, expected_delta);
    let expected_replacement = CollectionDelta::OrderedOccurrence(match &next {
        CollectionState::OrderedOccurrence(rows) => rows.clone(),
        _ => unreachable!(),
    });
    let (decoded, pos) =
        decode_shaped_delta(&schema, std::slice::from_ref(&prior), &expected_delta, 0).unwrap();
    assert_eq!(pos, expected_delta.len());
    assert_eq!(
        decoded,
        vec![Some(expected_replacement)],
        "all three rows, including the duplicate 9, survive in source order"
    );
    let mut replayed = prior;
    apply_shaped(&mut replayed, decoded[0].as_ref().unwrap()).unwrap();
    assert_eq!(replayed, next, "the replacement reconstructs the next list");
}

#[test]
fn vector_mixed_keyed_delta_and_ordered_replacement() {
    let schema = Schema {
        domain: 1,
        cadence_log2: 0,
        collections: vec![
            Collection {
                collection_id: 1,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(1, FieldKind::Unsigned).key(0),
                    Field::new(2, FieldKind::Unsigned),
                ],
            },
            Collection {
                collection_id: 2,
                shape: Shape::OrderedOccurrenceList,
                fields: vec![Field::new(1, FieldKind::Unsigned)],
            },
        ],
        event_part: None,
    };
    let mut keyed_prior = KeyedState::new();
    keyed_prior.insert(key(&[1]), Record::new(vec![Value::Unsigned(10)]));
    let priors = vec![
        CollectionState::Keyed(keyed_prior.clone()),
        CollectionState::OrderedOccurrence(vec![Record::new(vec![Value::Unsigned(3)])]),
    ];
    let keyed_delta = KeyedDelta {
        changed: vec![Row {
            key: key(&[1]),
            record: Record::new(vec![Value::Unsigned(11)]),
        }],
        tombstones: Vec::new(),
    };
    let deltas = vec![
        Some(CollectionDelta::Keyed(keyed_delta)),
        Some(CollectionDelta::OrderedOccurrence(vec![
            Record::new(vec![Value::Unsigned(9)]),
            Record::new(vec![Value::Unsigned(8)]),
        ])),
    ];
    let expected = vec![
        0b0000_0011, // both collection bits
        1, 1, 0x01, 11, 0, // keyed update: count, key, mask, value, zero tombstones
        2, 9, 8, // ordered replacement: count and complete records
    ];
    let mut bytes = Vec::new();
    encode_shaped_delta(&schema, &priors, &deltas, &mut bytes).unwrap();
    assert_eq!(bytes, expected);
    let (decoded, pos) = decode_shaped_delta(&schema, &priors, &expected, 0).unwrap();
    assert_eq!(pos, expected.len());
    assert_eq!(
        decoded, deltas,
        "both the keyed delta and ordered replacement decode exactly"
    );
    let mut replayed = priors.clone();
    for (state, delta) in replayed.iter_mut().zip(&decoded) {
        apply_shaped(state, delta.as_ref().unwrap()).unwrap();
    }
    let mut keyed_next = keyed_prior;
    keyed_next.insert(key(&[1]), Record::new(vec![Value::Unsigned(11)]));
    assert_eq!(replayed[0], CollectionState::Keyed(keyed_next));
    assert_eq!(
        replayed[1],
        CollectionState::OrderedOccurrence(vec![
            Record::new(vec![Value::Unsigned(9)]),
            Record::new(vec![Value::Unsigned(8)]),
        ])
    );
}

#[test]
fn a_delta_that_changes_nothing_is_rejected() {
    let schema = tiny();
    let mut out = Vec::new();
    assert_eq!(
        encode_delta(&schema, &[KeyedState::new()], &[None], &mut out),
        Err(MinimalityError::NoCollectionChanged),
        "that state is UNCHANGED, and two encodings of one fact is the defect"
    );
}

#[test]
fn a_set_bit_with_an_empty_contribution_is_rejected() {
    let schema = tiny();
    let empty = KeyedDelta { changed: Vec::new(), tombstones: Vec::new() };
    let mut out = Vec::new();
    assert_eq!(
        encode_delta(&schema, &[KeyedState::new()], &[Some(empty)], &mut out),
        Err(MinimalityError::EmptyContribution { collection_id: 1 })
    );
}

#[test]
fn re_emitting_an_unchanged_row_is_rejected() {
    let schema = tiny();
    let mut prior = KeyedState::new();
    prior.insert(key(&[1]), tiny_row(1, false, None));
    let next = prior.clone();
    let bogus = KeyedDelta {
        changed: vec![Row { key: key(&[1]), record: tiny_row(1, false, None) }],
        tombstones: Vec::new(),
    };
    assert_eq!(
        check_minimal(&schema.collections[0], &prior, &next, &bogus),
        Err(MinimalityError::UnchangedRowEmitted { collection_id: 1 }),
        "equal-fidelity size waste and a second encoding at once"
    );
    let _ = schema;
}

#[test]
fn omitting_a_changed_row_is_rejected() {
    let mut prior = KeyedState::new();
    prior.insert(key(&[1]), tiny_row(1, false, None));
    let mut next = KeyedState::new();
    next.insert(key(&[1]), tiny_row(2, false, None));
    let empty = KeyedDelta { changed: Vec::new(), tombstones: Vec::new() };
    let schema = tiny();
    assert_eq!(
        check_minimal(&schema.collections[0], &prior, &next, &empty),
        Err(MinimalityError::MissingChangedRow { collection_id: 1 }),
        "the rule that stops a writer silently dropping a row"
    );
}

#[test]
fn a_vanished_key_must_carry_a_tombstone() {
    let mut prior = KeyedState::new();
    prior.insert(key(&[1]), tiny_row(1, false, None));
    let next = KeyedState::new();
    let empty = KeyedDelta { changed: Vec::new(), tombstones: Vec::new() };
    let schema = tiny();
    assert_eq!(
        check_minimal(&schema.collections[0], &prior, &next, &empty),
        Err(MinimalityError::MissingTombstone { collection_id: 1 })
    );
}

#[test]
fn a_key_cannot_be_both_updated_and_deleted() {
    let mut prior = KeyedState::new();
    prior.insert(key(&[1]), tiny_row(1, false, None));
    let mut next = KeyedState::new();
    next.insert(key(&[1]), tiny_row(2, false, None));
    let bogus = KeyedDelta {
        changed: vec![Row { key: key(&[1]), record: tiny_row(2, false, None) }],
        tombstones: vec![key(&[1])],
    };
    let schema = tiny();
    assert_eq!(
        check_minimal(&schema.collections[0], &prior, &next, &bogus),
        Err(MinimalityError::KeyBothChangedAndTombstoned { collection_id: 1 })
    );
}

#[test]
fn vector_block_framing() {
    let header = BlockHeader { domain: 20, branch: 0, schema_hash: 1, first_tick: 0 };
    let mut w = BlockWriter::new(header, 1);
    w.push(Frame::state(Kind::Baseline, 0, vec![0xAA])).unwrap();
    w.push(Frame::unchanged(1, 3)).unwrap();
    let bytes = w.finish().unwrap();

    assert_eq!(
        bytes,
        vec![
            20, // domain
            0,  // branch
            1, 0, 0, 0, 0, 0, 0, 0, // schema_hash, little-endian u64
            0,  // first_tick
            2,  // frame len: tag + body
            pack_tag(0, Kind::Baseline).unwrap() as u8, // tag: delta 0, kind 0
            0xAA, // body
            2,    // frame len
            pack_tag(1, Kind::Unchanged).unwrap() as u8, // delta 1, kind 2
            3,    // run length in sample ticks
        ]
    );

    let (back_header, frames) = read_block(&bytes, 1).unwrap();
    assert_eq!(back_header.first_tick, 0);
    assert_eq!(frames[1].run_len, 3);
    assert_eq!(frames[1].last_covered_tick_checked(1).unwrap(), 3, "covers ticks 1, 2, 3");
}

#[test]
fn vector_d20_baseline_row_is_stable() {
    // A byte-exact fixture over the real reference-domain schema. Its purpose is
    // to fail loudly if any field's kind, order, nullability or quantization
    // changes without the schema hash being reconsidered.
    let schema = wire_format::d20::schema();
    let c = &schema.collections[0];
    let record = blank_record(c);

    let mut bytes = Vec::new();
    record.encode(c, &mut bytes).unwrap();

    let (back, pos) = Record::decode(c, &bytes, 0).unwrap();
    assert_eq!(pos, bytes.len());
    assert_eq!(back, record);

    // The blank row: every optional absent, every required at its zero value.
    //
    //   3 bytes  presence, over 17 optional fields
    //   0 bytes  null    — every nullable field here is optional and absent
    //   0 bytes  unknown — same for the three unknown-capable extension fields
    //   1 byte   escape  — organisation and strength are quantized AND required,
    //                      so they are always present and always counted
    //   1 byte   bit-packed: four required booleans
    //   3 bytes  location_province, organisation, strength
    //
    // The escape byte is the point: `q` counts **required** qualifying fields too,
    // not only optional ones the presence bitmap marks. Sizing it by present
    // optional fields alone would leave a required quantized field with two
    // encodings.
    assert_eq!(
        bytes,
        vec![
            0x00, 0x00, 0x00, // presence, 17 optional fields, all absent
            0x00, // escape: organisation and strength, neither escaping
            0x00, // bit-packed: four required booleans, all false
            0x00, // location_province content id 0
            0x00, // organisation fixed(2) zigzag 0
            0x00, // strength fixed(2) zigzag 0
        ],
        "a blank d20 row is eight bytes; the null and unknown bitmaps cost zero"
    );
}

#[test]
fn d20_schema_hash_is_pinned() {
    // Not a magic constant to be updated on a whim: if this changes, every block
    // written under the old hash decodes against a definition that no longer
    // matches, so the change must be deliberate and versioned.
    let schema = wire_format::d20::schema();
    schema.validate(TickGrid::new(3).unwrap().log2()).unwrap();
    // The literal below is the pin. If a field's kind, order, nullability,
    // quantization or the cadence changes, this fails — and it must, because
    // every block written under the old hash decodes against a definition that no
    // longer matches. Updating it is a deliberate, versioned act.
    assert_eq!(
        schema.schema_hash(),
        D20_SCHEMA_HASH,
        "d20 schema hash changed; this is a format break, not a test to re-bless"
    );
}

#[test]
fn quantization_and_the_escape_survive_a_real_row() {
    let schema = tiny();
    let mut c = schema.collections[0].clone();
    c.fields.push(Field::new(5, FieldKind::Continuous).optional().quant(Quant::Fixed(2)));

    // An ordinary value quantizes; a NaN escapes to raw f64 bits.
    let ordinary = Record::new(vec![
        Value::Unsigned(0),
        Value::Bool(false),
        Value::Absent,
        Value::Continuous(1.25),
    ]);
    let mut a = Vec::new();
    ordinary.encode(&c, &mut a).unwrap();

    let escaped = Record::new(vec![
        Value::Unsigned(0),
        Value::Bool(false),
        Value::Absent,
        Value::Continuous(f64::NAN),
    ]);
    let mut b = Vec::new();
    escaped.encode(&c, &mut b).unwrap();

    assert!(b.len() > a.len(), "the escape costs eight raw bytes: {} vs {}", b.len(), a.len());
    let (back_a, _) = Record::decode(&c, &a, 0).unwrap();
    assert_eq!(back_a.values[3], Value::Continuous(1.25));
    let (back_b, _) = Record::decode(&c, &b, 0).unwrap();
    match back_b.values[3] {
        Value::Continuous(x) => assert!(x.is_nan(), "NaN survives the escape path"),
        other => panic!("expected a continuous value, got {other:?}"),
    }
}

#[test]
fn vector_null_unknown_and_zero_are_three_encodings() {
    // §3's centrepiece: these must not collapse into each other.
    let c = Collection {
        collection_id: 1,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(1, FieldKind::Unsigned).key(0),
            Field::new(2, FieldKind::Unsigned).optional().nullable().unknown_capable(),
        ],
    };
    let enc = |v: Value| {
        let mut out = Vec::new();
        Record::new(vec![v]).encode(&c, &mut out).unwrap();
        out
    };

    // presence, null, unknown bitmaps are each one bit wide when the field is present.
    // presence, then null, then unknown; the bit-packed region is empty for a
    // varint field, so it costs nothing.
    assert_eq!(enc(Value::Absent), vec![0x00], "presence 0; no qualifying field is present");
    assert_eq!(enc(Value::Null), vec![0x01, 0x01, 0x00], "presence 1, null 1, unknown 0, no value");
    assert_eq!(enc(Value::Unknown), vec![0x01, 0x00, 0x01], "presence 1, null 0, unknown 1, no value");
    assert_eq!(
        enc(Value::Unsigned(0)),
        vec![0x01, 0x00, 0x00, 0x00],
        "zero is present, neither null nor unknown, and carries a value byte"
    );

    for v in [Value::Absent, Value::Null, Value::Unknown, Value::Unsigned(0)] {
        let bytes = enc(v);
        let (back, _) = Record::decode(&c, &bytes, 0).unwrap();
        assert_eq!(back.values[0], v, "{v:?} survives the roundtrip distinctly");
    }
}

#[test]
fn vector_nan_is_normalised_to_the_canonical_pattern() {
    // §0: the payload and sign of a NaN are not game state. Two writers with
    // different NaN payloads must emit identical bytes.
    let c = Collection {
        collection_id: 1,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(1, FieldKind::Unsigned).key(0),
            Field::new(2, FieldKind::Continuous).quant(Quant::Fixed(2)),
        ],
    };
    let enc = |bits: u64| {
        let mut out = Vec::new();
        Record::new(vec![Value::Continuous(f64::from_bits(bits))])
            .encode(&c, &mut out)
            .unwrap();
        out
    };
    let a = enc(0x7FF8_0000_0000_0001); // one NaN payload
    let b = enc(0xFFF8_0000_DEAD_BEEF); // another, with the sign bit set
    assert_eq!(a, b, "different NaN payloads must not produce different bytes");
    // Escape bit set, then the canonical pattern little-endian.
    assert_eq!(a, vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF8, 0x7F]);
}

#[test]
fn a_non_canonical_nan_payload_is_rejected_on_decode() {
    let c = Collection {
        collection_id: 1,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(1, FieldKind::Unsigned).key(0),
            Field::new(2, FieldKind::Continuous).quant(Quant::Fixed(2)),
        ],
    };
    // The field is required, so there is no presence byte: escape bitmap, then
    // the raw f64.
    let mut bytes = vec![0x01];
    bytes.extend_from_slice(&0x7FF8_0000_0000_0001u64.to_le_bytes());
    assert!(Record::decode(&c, &bytes, 0).is_err(), "a second encoding of NaN is corrupt");
}

#[test]
fn an_unnecessary_escape_is_rejected_on_decode() {
    // §6's escape conditions are exhaustive, so escaping an ordinary value is a
    // second encoding of it.
    let c = Collection {
        collection_id: 1,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(1, FieldKind::Unsigned).key(0),
            Field::new(2, FieldKind::Continuous).quant(Quant::Fixed(2)),
        ],
    };
    let mut bytes = vec![0x01];
    bytes.extend_from_slice(&1.25f64.to_bits().to_le_bytes());
    assert!(
        Record::decode(&c, &bytes, 0).is_err(),
        "1.25 quantizes cleanly, so it may not be escaped"
    );
}

#[test]
fn vector_two_time_chains_in_one_block() {
    // §1.4: the state and event chains advance independently, and the event chain
    // measures its first frame from first_tick.
    let header = BlockHeader { domain: 64, branch: 0, schema_hash: 0, first_tick: 0 };
    let mut w = BlockWriter::new(header, 1);
    w.push(Frame::state(Kind::Baseline, 0, vec![])).unwrap();
    w.push(Frame::unchanged(1, 10)).unwrap();
    w.push(Frame::events(5, vec![0x01])).unwrap();
    w.push(Frame::state(Kind::Delta, 11, vec![0x02])).unwrap();
    let bytes = w.finish().unwrap();

    let (_, frames) = read_block(&bytes, 1).unwrap();
    assert_eq!(frames[1].run_len, 10, "the run spans the event tick");
    assert_eq!(frames[2].tick, 5, "event measured from first_tick, not from the run");
    assert_eq!(
        frames[3].tick, 11,
        "the state chain resumes from the run's LAST covered tick, unaffected by the event"
    );
}

#[test]
fn vector_event_body_preserves_occurrence_order_and_derives_identity() {
    let part = EventPart {
        collection_id: 2,
        fields: vec![
            Field::new(1, FieldKind::Unsigned),
            Field::new(2, FieldKind::Boolean),
        ],
        frontier_component_ids: vec![1, 2],
        sequence_index: 1,
    };
    let batch = EventBatch::new(
        vec![
            Record::new(vec![Value::Unsigned(7), Value::Bool(true)]),
            Record::new(vec![Value::Unsigned(0), Value::Bool(false)]),
        ],
        vec![9, 12],
    );
    let bytes = encode_event_body(&part, &batch).unwrap();
    assert_eq!(
        bytes,
        vec![
            2, // event count
            1, 7, // event 0: true bit-packed byte, unsigned 7
            0, 0, // event 1: false bit-packed byte, unsigned zero
            9, 12, // after-frontier: epoch, sequence
        ]
    );
    let decoded = decode_event_body(&part, &bytes).unwrap();
    assert_eq!(decoded, batch);
    assert_eq!(decoded.sequence_at(&part, 0).unwrap(), 11);
    assert_eq!(decoded.sequence_at(&part, 1).unwrap(), 12);
}

#[test]
fn vector_event_body_preserves_byte_identical_duplicate_occurrences() {
    let part = EventPart {
        collection_id: 2,
        fields: vec![
            Field::new(1, FieldKind::Unsigned),
            Field::new(2, FieldKind::Boolean),
        ],
        frontier_component_ids: vec![1, 2],
        sequence_index: 1,
    };
    // Unlike the preceding order vector, this catches an implementation that
    // deduplicates by record content: both occurrences must survive and their
    // positions must derive distinct identities.
    let duplicate = Record::new(vec![Value::Unsigned(7), Value::Bool(true)]);
    let batch = EventBatch::new(
        vec![duplicate.clone(), duplicate.clone()],
        vec![9, 12],
    );
    let bytes = encode_event_body(&part, &batch).unwrap();
    assert_eq!(
        bytes,
        vec![
            2, // event count: duplicate occurrences are not collapsed
            1, 7, // event 0
            1, 7, // event 1, byte-identical content
            9, 12, // after-frontier
        ]
    );
    let decoded = decode_event_body(&part, &bytes).unwrap();
    assert_eq!(decoded.records, vec![duplicate.clone(), duplicate]);
    assert_eq!(decoded.sequence_at(&part, 0).unwrap(), 11);
    assert_eq!(decoded.sequence_at(&part, 1).unwrap(), 12);
    assert_eq!(encode_event_body(&part, &decoded).unwrap(), bytes);
}

#[test]
fn vector_frontier_discontinuities_and_post_gap_establishment() {
    let part = EventPart {
        collection_id: 2,
        fields: vec![Field::new(1, FieldKind::Unsigned)],
        frontier_component_ids: vec![1, 2],
        sequence_index: 1,
    };
    assert_eq!(
        check_frontier(&part, Some(&[4, 10]), &[5, 12], 2).unwrap(),
        FrontierOutcome::Discontinuous(FrontierDiscontinuity::EpochChanged {
            component_index: 0,
            previous: 4,
            next: 5,
        })
    );
    assert_eq!(
        check_frontier(&part, Some(&[4, 10]), &[4, 13], 2).unwrap(),
        FrontierOutcome::Discontinuous(FrontierDiscontinuity::SequenceMismatch {
            previous: 10,
            count: 2,
            expected: 12,
            next: 13,
        })
    );
    assert_eq!(
        check_frontier(&part, None, &[99, 500], 2).unwrap(),
        FrontierOutcome::Established,
        "dropping the prior after recording the gap makes the next batch establish"
    );
}

#[test]
fn vector_frontier_schema_transitions_and_invalid_declarations() {
    let previous = EventPart {
        collection_id: 2,
        fields: vec![Field::new(1, FieldKind::Unsigned)],
        frontier_component_ids: vec![1, 2],
        sequence_index: 1,
    };
    let reordered = EventPart {
        frontier_component_ids: vec![2, 1],
        sequence_index: 0,
        ..previous.clone()
    };
    assert_eq!(
        check_frontier_transition(
            Some((&previous, &[4, 10])),
            &reordered,
            &[12, 4],
            2,
        )
        .unwrap(),
        FrontierOutcome::Continuous,
        "component ids, not tuple positions, join frontiers across schema changes"
    );

    let changed = EventPart {
        frontier_component_ids: vec![2, 3],
        sequence_index: 0,
        ..previous.clone()
    };
    assert_eq!(
        check_frontier_transition(
            Some((&previous, &[4, 10])),
            &changed,
            &[12, 4],
            2,
        )
        .unwrap(),
        FrontierOutcome::Discontinuous(FrontierDiscontinuity::SchemaChanged)
    );
    assert_eq!(
        check_frontier(&previous, Some(&[4, u64::MAX]), &[4, u64::MAX], 1).unwrap(),
        FrontierOutcome::Discontinuous(FrontierDiscontinuity::SequenceOverflow {
            previous: u64::MAX,
            count: 1,
        })
    );

    let mut invalid = Schema {
        domain: 1,
        cadence_log2: 0,
        collections: Vec::new(),
        event_part: Some(EventPart {
            collection_id: 2,
            fields: Vec::new(),
            frontier_component_ids: vec![1],
            sequence_index: 0,
        }),
    };
    assert_eq!(
        invalid.validate(TickGrid::HOURLY.log2()),
        Err(SchemaError::EventHasNoFields { collection_id: 2 })
    );
    invalid.event_part.as_mut().unwrap().fields =
        vec![Field::new(1, FieldKind::Unsigned)];
    invalid.event_part.as_mut().unwrap().frontier_component_ids = vec![1, 1];
    assert_eq!(
        invalid.validate(TickGrid::HOURLY.log2()),
        Err(SchemaError::DuplicateFrontierComponentId { component_id: 1 })
    );
}
