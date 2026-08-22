use wire_format::body::{
    apply_shaped, collection_stored_eq, decode_shaped_baseline, decode_shaped_delta, diff_shaped,
    encode_baseline, encode_shaped_baseline, encode_shaped_delta, key, CollectionDelta,
    CollectionState, Key, KeyedState, MinimalityError, Row,
};
use wire_format::record::Record;
use wire_format::schema::{Collection, Field, Schema, SchemaError, Shape};
use wire_format::value::Value;
use wire_format::{Kind, WireError};

fn record(value: u64) -> Record {
    Record::new(vec![Value::Unsigned(value)])
}

fn ordered_keyed_schema() -> Schema {
    Schema {
        domain: 1,
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: 1,
            shape: Shape::OrderedKeyedList,
            fields: vec![
                Field::new(1, Kind::Unsigned).key(0),
                Field::new(2, Kind::Unsigned),
            ],
        }],
        event_part: None,
    }
}

fn occurrence_schema() -> Schema {
    Schema {
        domain: 1,
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: 1,
            shape: Shape::OrderedOccurrenceList,
            fields: vec![Field::new(1, Kind::Unsigned)],
        }],
        event_part: None,
    }
}

fn mixed_schema() -> Schema {
    Schema {
        domain: 1,
        cadence_log2: 0,
        collections: vec![
            Collection {
                collection_id: 1,
                shape: Shape::KeyedSet,
                fields: vec![
                    Field::new(1, Kind::Unsigned).key(0),
                    Field::new(2, Kind::Unsigned),
                ],
            },
            Collection {
                collection_id: 2,
                shape: Shape::OrderedOccurrenceList,
                fields: vec![Field::new(1, Kind::Unsigned)],
            },
        ],
        event_part: None,
    }
}

#[test]
fn both_ordered_baselines_round_trip_source_order_without_sorting_or_deduplication() {
    let keyed_schema = ordered_keyed_schema();
    let keyed = CollectionState::OrderedKeyed(vec![
        Row {
            key: key(&[9]),
            record: record(90),
        },
        Row {
            key: key(&[2]),
            record: record(20),
        },
    ]);
    let mut bytes = Vec::new();
    encode_shaped_baseline(&keyed_schema, std::slice::from_ref(&keyed), &mut bytes).unwrap();
    let (decoded, used) = decode_shaped_baseline(&keyed_schema, &bytes, 0).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(
        decoded,
        vec![keyed],
        "non-ascending keys remain in source order"
    );

    let occurrence_schema = occurrence_schema();
    let occurrence = CollectionState::OrderedOccurrence(vec![record(9), record(3), record(9)]);
    let mut bytes = Vec::new();
    encode_shaped_baseline(
        &occurrence_schema,
        std::slice::from_ref(&occurrence),
        &mut bytes,
    )
    .unwrap();
    let (decoded, used) = decode_shaped_baseline(&occurrence_schema, &bytes, 0).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(
        decoded,
        vec![occurrence],
        "a non-ascending run with a byte-identical duplicate is unchanged"
    );
}

#[test]
fn an_ordered_delta_reuses_the_complete_baseline_contribution_byte_for_byte() {
    for (schema, prior, next) in [
        (
            ordered_keyed_schema(),
            CollectionState::OrderedKeyed(vec![Row {
                key: key(&[1]),
                record: record(1),
            }]),
            CollectionState::OrderedKeyed(vec![Row {
                key: key(&[8]),
                record: record(2),
            }]),
        ),
        (
            occurrence_schema(),
            CollectionState::OrderedOccurrence(vec![record(1)]),
            CollectionState::OrderedOccurrence(vec![record(2), record(1)]),
        ),
    ] {
        let mut baseline = Vec::new();
        encode_shaped_baseline(&schema, std::slice::from_ref(&next), &mut baseline).unwrap();
        let delta = diff_shaped(&schema.collections[0], &prior, &next)
            .unwrap()
            .expect("content differs");
        let mut delta_bytes = Vec::new();
        encode_shaped_delta(
            &schema,
            std::slice::from_ref(&prior),
            &[Some(delta)],
            &mut delta_bytes,
        )
        .unwrap();
        assert_eq!(
            &delta_bytes[1..],
            baseline,
            "after the one-byte bitmap the replacement is the baseline contribution"
        );
    }
}

#[test]
fn an_unchanged_ordered_replacement_is_refused_on_write_and_read() {
    let schema = occurrence_schema();
    let prior = CollectionState::OrderedOccurrence(vec![record(7)]);
    let replacement = CollectionDelta::OrderedOccurrence(vec![record(7)]);
    assert_eq!(
        encode_shaped_delta(
            &schema,
            std::slice::from_ref(&prior),
            &[Some(replacement)],
            &mut Vec::new(),
        ),
        Err(MinimalityError::ReplacementMatchesPrior { collection_id: 1 })
    );
    assert_eq!(
        decode_shaped_delta(
            &schema,
            std::slice::from_ref(&prior),
            &[0x01, 0x01, 0x07],
            0
        ),
        Err(WireError::Corrupt(
            "ordered collection bit set with unchanged replacement"
        ))
    );
}

#[test]
fn a_zero_row_replacement_clears_a_nonempty_ordered_collection() {
    let schema = occurrence_schema();
    let prior = CollectionState::OrderedOccurrence(vec![record(1), record(2)]);
    let delta = CollectionDelta::OrderedOccurrence(Vec::new());
    let mut bytes = Vec::new();
    encode_shaped_delta(
        &schema,
        std::slice::from_ref(&prior),
        &[Some(delta)],
        &mut bytes,
    )
    .unwrap();
    assert_eq!(
        bytes,
        vec![0x01, 0x00],
        "bitmap then legal zero-row replacement"
    );
    let (decoded, used) =
        decode_shaped_delta(&schema, std::slice::from_ref(&prior), &bytes, 0).unwrap();
    assert_eq!(used, bytes.len());
    let mut replayed = prior;
    apply_shaped(&mut replayed, decoded[0].as_ref().unwrap()).unwrap();
    assert!(replayed.is_empty());
}

#[test]
fn a_set_bit_cannot_replace_an_empty_ordered_collection_with_empty() {
    let schema = occurrence_schema();
    let prior = CollectionState::OrderedOccurrence(Vec::new());
    let delta = CollectionDelta::OrderedOccurrence(Vec::new());
    assert_eq!(
        encode_shaped_delta(
            &schema,
            std::slice::from_ref(&prior),
            &[Some(delta)],
            &mut Vec::new(),
        ),
        Err(MinimalityError::ReplacementMatchesPrior { collection_id: 1 })
    );
    assert_eq!(
        decode_shaped_delta(&schema, std::slice::from_ref(&prior), &[0x01, 0x00], 0),
        Err(WireError::Corrupt(
            "ordered collection bit set with unchanged replacement"
        ))
    );
}

#[test]
fn ordered_keyed_lists_require_unique_keys_but_not_sorted_keys() {
    let schema = ordered_keyed_schema();
    let duplicate = CollectionState::OrderedKeyed(vec![
        Row {
            key: key(&[5]),
            record: record(1),
        },
        Row {
            key: key(&[5]),
            record: record(2),
        },
    ]);
    assert_eq!(
        encode_shaped_baseline(&schema, &[duplicate], &mut Vec::new()),
        Err(WireError::Corrupt(
            "ordered-keyed list contains a duplicate key"
        )),
        "writer rejects a duplicate for the uniqueness rule"
    );
    assert_eq!(
        decode_shaped_baseline(&schema, &[2, 5, 1, 5, 2], 0),
        Err(WireError::Corrupt(
            "ordered-keyed list contains a duplicate key"
        )),
        "reader rejects a duplicate for the same rule"
    );

    let descending = CollectionState::OrderedKeyed(vec![
        Row {
            key: key(&[5]),
            record: record(1),
        },
        Row {
            key: key(&[2]),
            record: record(2),
        },
    ]);
    let mut bytes = Vec::new();
    encode_shaped_baseline(&schema, std::slice::from_ref(&descending), &mut bytes).unwrap();
    assert_eq!(
        decode_shaped_baseline(&schema, &bytes, 0).unwrap().0[0],
        descending
    );
}

#[test]
fn ordered_key_components_are_range_checked_on_write() {
    let schema = ordered_keyed_schema();
    let out_of_range = CollectionState::OrderedKeyed(vec![Row {
        key: Key(vec![u64::MAX as i128 + 1]),
        record: record(1),
    }]);
    assert_eq!(
        encode_shaped_baseline(&schema, &[out_of_range], &mut Vec::new()),
        Err(WireError::Corrupt(
            "key component is outside its kind's range"
        ))
    );
}

#[test]
fn ordered_absolute_key_varints_reject_values_past_u64() {
    let schema = ordered_keyed_schema();
    let mut hostile = vec![1];
    hostile.extend_from_slice(&[0x80; 9]);
    hostile.push(0x02);
    hostile.push(0x00);
    assert_eq!(
        decode_shaped_baseline(&schema, &hostile, 0),
        Err(WireError::Overlong)
    );
}

#[test]
fn a_mixed_frame_can_change_both_shapes_or_only_the_keyed_set() {
    let schema = mixed_schema();
    let mut keyed_prior = KeyedState::new();
    keyed_prior.insert(key(&[1]), record(10));
    let prior = vec![
        CollectionState::Keyed(keyed_prior.clone()),
        CollectionState::OrderedOccurrence(vec![record(3)]),
    ];
    let mut keyed_next = keyed_prior.clone();
    keyed_next.insert(key(&[1]), record(11));
    let both = vec![
        CollectionState::Keyed(keyed_next.clone()),
        CollectionState::OrderedOccurrence(vec![record(9), record(8)]),
    ];
    let both_deltas: Vec<_> = schema
        .collections
        .iter()
        .enumerate()
        .map(|(i, collection)| diff_shaped(collection, &prior[i], &both[i]).unwrap())
        .collect();
    let mut bytes = Vec::new();
    encode_shaped_delta(&schema, &prior, &both_deltas, &mut bytes).unwrap();
    assert_eq!(bytes[0], 0b0000_0011);
    let (decoded, used) = decode_shaped_delta(&schema, &prior, &bytes, 0).unwrap();
    assert_eq!(used, bytes.len());
    let mut replayed = prior.clone();
    for (state, delta) in replayed.iter_mut().zip(decoded.iter().flatten()) {
        apply_shaped(state, delta).unwrap();
    }
    for (i, collection) in schema.collections.iter().enumerate() {
        assert!(collection_stored_eq(collection, &replayed[i], &both[i]).unwrap());
    }

    let keyed_only = vec![CollectionState::Keyed(keyed_next), prior[1].clone()];
    let keyed_only_deltas: Vec<_> = schema
        .collections
        .iter()
        .enumerate()
        .map(|(i, collection)| diff_shaped(collection, &prior[i], &keyed_only[i]).unwrap())
        .collect();
    let mut keyed_only_bytes = Vec::new();
    encode_shaped_delta(&schema, &prior, &keyed_only_deltas, &mut keyed_only_bytes).unwrap();
    assert_eq!(keyed_only_bytes[0], 0b0000_0001);
    assert_eq!(
        keyed_only_bytes.len(),
        6,
        "the clear ordered bit contributes no row-count byte"
    );
}

#[test]
fn the_keyed_compatibility_entry_point_is_byte_identical_to_the_shaped_path() {
    let schema = mixed_schema();
    let keyed_schema = Schema {
        collections: vec![schema.collections[0].clone()],
        ..schema
    };
    let mut state = KeyedState::new();
    state.insert(key(&[3]), record(7));
    let mut compatibility = Vec::new();
    encode_baseline(
        &keyed_schema,
        std::slice::from_ref(&state),
        &mut compatibility,
    )
    .unwrap();
    let head_bytes = vec![1, 3, 7];
    assert_eq!(
        compatibility, head_bytes,
        "the compatibility entry point retains the HEAD keyed bytes"
    );
    let mut shaped = Vec::new();
    encode_shaped_baseline(
        &keyed_schema,
        &[CollectionState::Keyed(state.clone())],
        &mut shaped,
    )
    .unwrap();
    assert_eq!(
        shaped, head_bytes,
        "the single shaped implementation is pinned to the same HEAD fixture"
    );
}

#[test]
fn the_shaped_decoder_keeps_the_all_zero_delta_bitmap_invalid() {
    let schema = occurrence_schema();
    let prior = CollectionState::OrderedOccurrence(Vec::new());
    assert_eq!(
        decode_shaped_delta(&schema, &[prior], &[0x00], 0),
        Err(WireError::Corrupt("DELTA change bitmap is all zero"))
    );
}

#[test]
fn schema_validation_rejects_a_fieldless_collection_of_every_shape() {
    for shape in [
        Shape::KeyedSet,
        Shape::OrderedKeyedList,
        Shape::OrderedOccurrenceList,
    ] {
        let schema = Schema {
            domain: 1,
            cadence_log2: 0,
            collections: vec![Collection {
                collection_id: 7,
                shape,
                fields: Vec::new(),
            }],
            event_part: None,
        };
        assert_eq!(
            schema.validate(0),
            Err(SchemaError::CollectionHasNoFields { collection_id: 7 }),
            "fieldless rows would make the wire count incapable of bounding work"
        );
    }
}

fn hostile_count(exponent: u32) -> Vec<u8> {
    let mut value = 1u64 << exponent;
    let mut bytes = Vec::new();
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            return bytes;
        }
    }
}

#[test]
fn untrusted_ordered_row_counts_are_bounded_before_allocation_or_looping() {
    for schema in [ordered_keyed_schema(), occurrence_schema()] {
        let prior = match schema.collections[0].shape {
            Shape::OrderedKeyedList => CollectionState::OrderedKeyed(Vec::new()),
            Shape::OrderedOccurrenceList => CollectionState::OrderedOccurrence(Vec::new()),
            Shape::KeyedSet => unreachable!(),
        };
        for exponent in [40, 60] {
            let count = hostile_count(exponent);
            assert_eq!(
                decode_shaped_baseline(&schema, &count, 0),
                Err(WireError::Corrupt(
                    "collection row count exceeds remaining body bytes"
                )),
                "the 2^{exponent} baseline count returns a typed error"
            );
            let mut delta = vec![0x01];
            delta.extend_from_slice(&count);
            assert_eq!(
                decode_shaped_delta(&schema, std::slice::from_ref(&prior), &delta, 0),
                Err(WireError::Corrupt(
                    "collection row count exceeds remaining body bytes"
                )),
                "the 2^{exponent} delta count returns the same typed error"
            );
        }
    }
}

#[test]
fn a_fieldless_occurrence_decoder_still_cannot_run_an_unbounded_loop() {
    let schema = Schema {
        domain: 1,
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: 1,
            shape: Shape::OrderedOccurrenceList,
            fields: Vec::new(),
        }],
        event_part: None,
    };
    assert_eq!(
        decode_shaped_baseline(&schema, &hostile_count(60), 0),
        Err(WireError::Corrupt(
            "collection row count exceeds remaining body bytes"
        ))
    );
}
