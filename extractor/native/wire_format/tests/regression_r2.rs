//! Regression tests for the six defects the round-2 adversarial review found in
//! the code written to fix round 1.
//!
//! Each is the reviewer's probe with its assertion inverted: the probe passed
//! when the defect was real, so this passes only while the fix holds. They live
//! apart from `golden.rs` because they pin *behaviour*, not bytes — except where
//! the defect was a byte, and then the byte is written out literally.

use wire_format::body::{
    check_minimal, decode_delta, diff, key, KeyedState,
};
use wire_format::record::Record;
use wire_format::schema::{Collection, Derived, Field, Quant, Schema, Shape};
use wire_format::value::Value;
use wire_format::{Kind, WireError};

fn keyed(fields: Vec<Field>) -> Schema {
    Schema {
        domain: 1,
        cadence_log2: 0,
        collections: vec![Collection { collection_id: 1, shape: Shape::KeyedSet, fields }],
        event_part: None,
    }
}

/// §7.2: "A changed field whose new presence bit is 0 has its null, unknown and
/// escape bits 0." Without the check, `became absent` had four encodings.
#[test]
fn state_bits_on_a_field_that_became_absent_are_rejected() {
    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Unsigned).optional().nullable(),
    ]);
    let c = &s.collections[0];
    let prior = Record::new(vec![Value::Unsigned(5)]);

    // mask 0x01, presence 0x00, null 0x01 — a null bit under an absent field.
    assert_eq!(
        wire_format::update::decode(c, &prior, &[0x01, 0x00, 0x01], 0),
        Err(WireError::Corrupt("state bits set on a field that became absent"))
    );

    // The one legal encoding of that transition still decodes.
    let (v, _) = wire_format::update::decode(c, &prior, &[0x01, 0x00, 0x00], 0).unwrap();
    assert_eq!(v.values, vec![Value::Absent]);
}

/// §7.1: "so is a mask bit on a field whose value is re-emitted unchanged."
#[test]
fn a_mask_bit_re_emitting_the_prior_value_is_rejected() {
    let s = keyed(vec![Field::new(1, Kind::Unsigned).key(0), Field::new(2, Kind::Unsigned)]);
    let c = &s.collections[0];
    let prior = Record::new(vec![Value::Unsigned(7)]);

    assert_eq!(
        wire_format::update::decode(c, &prior, &[0x01, 7], 0),
        Err(WireError::Corrupt("change mask bit on a field that did not change"))
    );
    // A bit on a field that genuinely moved is still fine.
    let (v, _) = wire_format::update::decode(c, &prior, &[0x01, 8], 0).unwrap();
    assert_eq!(v.values, vec![Value::Unsigned(8)]);
}

/// A NaN that never changes must not re-encode forever, and the differ's output
/// must satisfy the differ's own checker.
#[test]
fn a_persistent_nan_diffs_as_unchanged() {
    // `exact`, so the NaN reaches the bytes rather than being quantized away.
    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Continuous),
    ]);
    let c = &s.collections[0];
    let mut state = KeyedState::new();
    state.insert(key(&[1]), Record::new(vec![Value::Continuous(f64::NAN)]));

    let d = diff(c, &state, &state.clone());
    assert!(d.changed.is_empty(), "identical states must diff as unchanged");
    assert_eq!(check_minimal(c, &state, &state.clone(), &d), Ok(()));

    // And a NaN with a different payload is the same value, because §0 says the
    // payload is not game state.
    let mut other = KeyedState::new();
    other.insert(
        key(&[1]),
        Record::new(vec![Value::Continuous(f64::from_bits(0xFFF8_0000_DEAD_BEEF))]),
    );
    assert!(diff(c, &state, &other).changed.is_empty(), "one canonical NaN, one value");
}

/// The opposite direction: `-0.0` and `0.0` compare equal under IEEE but encode
/// to different bytes, so the transition is real state the differ must emit.
#[test]
fn a_negative_zero_transition_is_a_change() {
    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Continuous), // exact — the sign of zero survives
    ]);
    let c = &s.collections[0];
    let row = |x: f64| Record::new(vec![Value::Continuous(x)]);

    let mut before = Vec::new();
    row(-0.0).encode(c, &mut before).unwrap();
    let mut after = Vec::new();
    row(0.0).encode(c, &mut after).unwrap();
    assert_ne!(before, after, "the premise: different canonical bytes");

    let mut prior = KeyedState::new();
    prior.insert(key(&[1]), row(-0.0));
    let mut next = KeyedState::new();
    next.insert(key(&[1]), row(0.0));

    let d = diff(c, &prior, &next);
    assert_eq!(d.changed.len(), 1, "different bytes must diff as changed");
    assert_eq!(check_minimal(c, &prior, &next, &d), Ok(()));
}

/// §6: "always encoded as an absolute zigzag, whatever its kind and whatever the
/// frame." The update path had it; baselines and inserts did not.
#[test]
fn a_derived_field_is_absolute_zigzag_in_a_baseline_too() {
    let mut derived = Field::new(2, Kind::Unsigned);
    derived.derived = Some(Derived { c: 1, u: 0 });
    let s = keyed(vec![Field::new(1, Kind::Unsigned).key(0), derived]);
    s.validate(3).expect("a derived unsigned field is legal");
    let c = &s.collections[0];

    // A derived field's STORED value is Signed whatever the live kind, because
    // §6's transform can carry an otherwise unsigned quantity negative.
    let mut bytes = Vec::new();
    Record::new(vec![Value::Signed(1)]).encode(c, &mut bytes).unwrap();
    assert_eq!(bytes, vec![0x02], "zigzag(1) is 2; the plain varint would be 1");

    let (back, pos) = Record::decode(c, &bytes, 0).unwrap();
    assert_eq!(pos, bytes.len());
    assert_eq!(back.values, vec![Value::Signed(1)]);

    // The value that makes the two encodings unambiguously different.
    let mut big = Vec::new();
    Record::new(vec![Value::Signed(64)]).encode(c, &mut big).unwrap();
    assert_eq!(big, vec![0x80, 0x01], "zigzag(64) is 128, a two-byte varint");

    // And the negative stored value the old code could not represent at all.
    let mut neg = Vec::new();
    Record::new(vec![Value::Signed(-5)]).encode(c, &mut neg).unwrap();
    assert_eq!(neg, vec![0x09], "zigzag(-5) is 9");
    assert_eq!(Record::decode(c, &neg, 0).unwrap().0.values, vec![Value::Signed(-5)]);
}

/// §7.1 constrains both delta lists on read as well as on write: the decoder
/// holds the same reconstructed prior state the writer did, so every rule it can
/// decide, it must.
#[test]
fn the_delta_decoder_enforces_the_same_list_rules_as_the_encoder() {
    let s = keyed(vec![Field::new(1, Kind::Unsigned).key(0), Field::new(2, Kind::Unsigned)]);
    let mut prior = KeyedState::new();
    prior.insert(key(&[5]), Record::new(vec![Value::Unsigned(0)]));
    let priors = std::slice::from_ref(&prior);

    // bitmap 0x01, 0 changed, 2 tombstones: absolute 5, then a delta of 0.
    assert_eq!(
        decode_delta(&s, priors, &[0x01, 0x00, 0x02, 0x05, 0x00], 0),
        Err(WireError::Corrupt("tombstones are not ascending by key"))
    );
    // A tombstone for a key that was never there.
    assert_eq!(
        decode_delta(&s, priors, &[0x01, 0x00, 0x01, 0x09], 0),
        Err(WireError::Corrupt("tombstone for a key that was already absent"))
    );
    // The legal delta these were mutations of.
    let (out, _) = decode_delta(&s, priors, &[0x01, 0x00, 0x01, 0x05], 0).unwrap();
    assert_eq!(out[0].as_ref().unwrap().tombstones, vec![key(&[5])]);
}

/// The changed-row list writes its own key chain because keys interleave with
/// records, so it has to repeat the checks `write_sorted_keys` performs.
#[test]
fn the_changed_row_path_checks_key_arity_and_order() {
    use wire_format::body::{encode_delta, KeyedDelta, MinimalityError, Row};

    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Unsigned).key(1),
        Field::new(3, Kind::Unsigned),
    ]);
    let rec = || Record::new(vec![Value::Unsigned(0)]);

    // A one-component key where the schema declares two: silently undecodable.
    let short = KeyedDelta {
        changed: vec![Row { key: key(&[1]), record: rec() }],
        tombstones: vec![],
    };
    let mut out = Vec::new();
    assert_eq!(
        encode_delta(&s, &[KeyedState::new()], &[Some(short)], &mut out),
        Err(MinimalityError::KeyArityMismatch { collection_id: 1 })
    );

    // Sorted on the first component, unsorted on the full tuple.
    let unsorted = KeyedDelta {
        changed: vec![
            Row { key: key(&[1, 5]), record: rec() },
            Row { key: key(&[1, 3]), record: rec() },
        ],
        tombstones: vec![],
    };
    let mut out = Vec::new();
    assert_eq!(
        encode_delta(&s, &[KeyedState::new()], &[Some(unsorted)], &mut out),
        Err(MinimalityError::KeysNotSorted { collection_id: 1 })
    );
}

/// The escape bit is only meaningful on a quantized field, and only when that
/// field carries a value — checked here on the update path, where round 1 fixed
/// it on the record path.
#[test]
fn an_escape_on_a_valueless_update_field_is_rejected() {
    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Continuous).nullable().quant(Quant::Fixed(2)),
    ]);
    let c = &s.collections[0];
    let prior = Record::new(vec![Value::Continuous(1.0)]);

    // mask 0x01, null 0x01, escape 0x01.
    assert_eq!(
        wire_format::update::decode(c, &prior, &[0x01, 0x01, 0x01], 0),
        Err(WireError::Corrupt("escape bit set on a field carrying no value"))
    );
}

// ---- Round 2b: the second reviewer's findings, one regression test each. ----

/// Minimality is a statement about the **stored** value. Under `fixed(2)` a
/// field moving 0.33 → 0.331 does not change the bytes, so no update exists to
/// emit — and emitting one produced a row the decoder rejected as unchanged.
#[test]
fn a_source_change_within_one_quantization_step_is_not_a_change() {
    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Continuous).quant(Quant::Fixed(2)),
    ]);
    let c = &s.collections[0];
    let prior = Record::new(vec![Value::Continuous(0.33)]);
    let next = Record::new(vec![Value::Continuous(0.331)]);

    let mut out = Vec::new();
    assert_eq!(
        wire_format::update::encode(c, &prior, &next, &mut out),
        Err(WireError::Corrupt("update row with an all-zero change mask")),
        "no field changed as stored, so there is no update row"
    );

    // And the differ agrees, which is what keeps encode and check_minimal in step.
    let mut a = KeyedState::new();
    a.insert(key(&[1]), prior.clone());
    let mut b = KeyedState::new();
    b.insert(key(&[1]), next.clone());
    assert!(diff(c, &a, &b).changed.is_empty());
    assert_eq!(check_minimal(c, &a, &b, &wire_format::body::KeyedDelta {
        changed: vec![],
        tombstones: vec![],
    }), Ok(()));

    // A move that crosses the step boundary is still a change.
    let across = Record::new(vec![Value::Continuous(0.336)]);
    let mut out = Vec::new();
    assert!(wire_format::update::encode(c, &prior, &across, &mut out).is_ok());
    let (back, _) = wire_format::update::decode(c, &prior, &out, 0).unwrap();
    assert_eq!(back.values, vec![Value::Continuous(0.34)]);
}

/// The change bitmap is built from every delta, but contributions were written
/// through a `zip` that stops at the shortest input — so a short `priors` set a
/// bit and then wrote nothing for it.
#[test]
fn a_prior_state_list_of_the_wrong_length_is_rejected() {
    use wire_format::body::{encode_delta, KeyedDelta, MinimalityError, Row};

    let two = Schema {
        domain: 1,
        cadence_log2: 0,
        collections: (1..=2)
            .map(|id| Collection {
                collection_id: id,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(1, Kind::Unsigned).key(0),
                    Field::new(2, Kind::Unsigned),
                ],
            })
            .collect(),
        event_part: None,
    };
    let d = || {
        Some(KeyedDelta {
            changed: vec![Row { key: key(&[1]), record: Record::new(vec![Value::Unsigned(1)]) }],
            tombstones: vec![],
        })
    };

    let mut out = Vec::new();
    assert_eq!(
        encode_delta(&two, &[KeyedState::new()], &[d(), d()], &mut out),
        Err(MinimalityError::ArityMismatch),
        "two collections, two deltas, one prior state"
    );
    let mut out = Vec::new();
    assert_eq!(
        encode_delta(&two, &[KeyedState::new(), KeyedState::new()], &[d()], &mut out),
        Err(MinimalityError::ArityMismatch)
    );
}

/// §2 orders baseline rows by the full key tuple, and the baseline path had
/// neither the read-side order check nor the write-side arity check the delta
/// path grew.
#[test]
fn the_baseline_path_checks_key_order_and_arity() {
    use wire_format::body::{decode_baseline, encode_baseline};

    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Unsigned).key(1),
        Field::new(3, Kind::Unsigned),
    ]);

    // Read: count 2, then (1,5) followed by first-diff 0 with component 3 = (1,3).
    assert_eq!(
        decode_baseline(&s, &[0x02, 0x01, 0x05, 0x00, 0x00, 0x03, 0x00], 0),
        Err(WireError::Corrupt("baseline rows are not ascending by key"))
    );
    // The ascending version of the same bytes is fine.
    let (out, _) = decode_baseline(&s, &[0x02, 0x01, 0x03, 0x00, 0x00, 0x05, 0x00], 0).unwrap();
    assert_eq!(out[0].len(), 2);

    // Write: a three-component key where the schema declares two indexed past
    // the end of the kind list and panicked.
    let mut state = KeyedState::new();
    state.insert(key(&[1, 2, 3]), Record::new(vec![Value::Unsigned(0)]));
    let mut bytes = Vec::new();
    assert_eq!(
        encode_baseline(&s, &[state], &mut bytes),
        Err(WireError::Corrupt("key arity does not match schema"))
    );
}

/// A rejected `push` must not leave the writer describing a block that was never
/// written. The sequence below produced a delta with no baseline.
#[test]
fn a_rejected_frame_does_not_advance_the_writer() {
    use wire_format::frame::{BlockHeader, BlockWriter, Frame, Kind as FKind};

    let header = BlockHeader { domain: 20, branch: 0, schema_hash: 1, first_tick: 0 };
    let mut w = BlockWriter::new(header, 1);
    w.push(Frame::events(0, vec![1])).unwrap();
    // Rejected: at tick 0 the state frame must precede the event frame.
    assert!(w.push(Frame::state(FKind::Baseline, 0, vec![2])).is_err());
    // So no baseline has been seen, and a delta cannot follow.
    assert_eq!(
        w.push(Frame::state(FKind::Delta, 1, vec![3])),
        Err(wire_format::frame::BlockError::DeltaBeforeBaseline { tick: 1 })
    );
}

/// §1's invariants are enforced on read as well as on write: a forged block is
/// one `BlockWriter` could never have produced.
#[test]
fn read_block_rejects_a_forged_block() {
    use wire_format::frame::{pack_tag, read_block, BlockHeader, Kind as FKind};

    let mut bytes = Vec::new();
    BlockHeader { domain: 20, branch: 0, schema_hash: 1, first_tick: 0 }.encode(&mut bytes);
    // A DELTA as the block's first state frame — no BASELINE anywhere.
    bytes.push(2); // frame len
    bytes.push(pack_tag(0, FKind::Delta).unwrap() as u8);
    bytes.push(0xAA);
    assert_eq!(
        read_block(&bytes, 1),
        Err(WireError::Corrupt("block violates a §1 invariant"))
    );

    // Off-grid: stride 8, a baseline at tick 0 then a delta at tick 3.
    let mut bytes = Vec::new();
    BlockHeader { domain: 20, branch: 0, schema_hash: 1, first_tick: 0 }.encode(&mut bytes);
    bytes.push(2);
    bytes.push(pack_tag(0, FKind::Baseline).unwrap() as u8);
    bytes.push(0xAA);
    bytes.push(2);
    bytes.push(pack_tag(3, FKind::Delta).unwrap() as u8);
    bytes.push(0xBB);
    assert_eq!(
        read_block(&bytes, 8),
        Err(WireError::Corrupt("block violates a §1 invariant"))
    );
}

/// Timestamps arrive as untrusted varints. Unchecked, the run endpoint panics
/// under this workspace's `overflow-checks` and wraps without them.
#[test]
fn timestamp_arithmetic_does_not_overflow_on_hostile_input() {
    use wire_format::frame::{read_block, BlockHeader, Frame, Kind as FKind};

    // A run length that overflows its endpoint is rejected, not wrapped.
    let f = Frame::unchanged(u64::MAX - 4, u64::MAX);
    assert_eq!(f.last_covered_tick_checked(8), None);
    assert_eq!(Frame::unchanged(0, 3).last_covered_tick_checked(8), Some(16));

    // A tick delta that overflows the chain base is Corrupt, not a panic.
    let mut bytes = Vec::new();
    BlockHeader { domain: 20, branch: 0, schema_hash: 1, first_tick: u64::MAX }
        .encode(&mut bytes);
    let mut frame = Vec::new();
    wire_format::varint::write_varint(&mut frame, (1u64 << 60) << 3 | FKind::Baseline as u64);
    frame.push(0xAA);
    wire_format::varint::write_varint(&mut bytes, frame.len() as u64);
    bytes.extend_from_slice(&frame);
    assert!(read_block(&bytes, 1).is_err(), "must not panic");
}
