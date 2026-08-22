use wire_format::body::{
    decode_baseline, encode_baseline, encode_delta, Key, KeyedDelta, KeyedState, Row,
};
use wire_format::record::Record;
use wire_format::schema::{Collection, Derived, EventPart, Field, Schema, SchemaError, Shape};
use wire_format::value::Value;
use wire_format::{Kind, WireError};

fn keyed(fields: Vec<Field>) -> Schema {
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
fn an_out_of_range_key_component_is_rejected_everywhere() {
    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Unsigned),
    ]);
    let too_big = Key(vec![(u64::MAX as i128) + 1]);
    let negative = Key(vec![-1]);
    let row = || Record::new(vec![Value::Unsigned(0)]);

    for bad in [too_big.clone(), negative.clone()] {
        let mut state = KeyedState::new();
        state.insert(bad.clone(), row());
        let mut out = Vec::new();
        assert_eq!(
            encode_baseline(&s, &[state.clone()], &mut out),
            Err(WireError::Corrupt(
                "key component is outside its kind's range"
            )),
            "baseline must reject {bad:?}"
        );

        let mut out = Vec::new();
        let delta = KeyedDelta {
            changed: vec![Row {
                key: bad.clone(),
                record: row(),
            }],
            tombstones: vec![],
        };
        assert!(
            encode_delta(&s, &[KeyedState::new()], &[Some(delta)], &mut out).is_err(),
            "delta must reject {bad:?}"
        );

        let mut out = Vec::new();
        assert_eq!(
            wire_format::columnar::encode_baseline_columnar(&s.collections[0], &state, &mut out),
            Err(WireError::Corrupt(
                "key component is outside its kind's range"
            )),
            "columnar must reject {bad:?}"
        );
    }

    let mut state = KeyedState::new();
    state.insert(Key(vec![u64::MAX as i128]), row());
    let mut out = Vec::new();
    encode_baseline(&s, &[state], &mut out).unwrap();
    let back = decode_baseline(&s, &out, 0).unwrap().0;
    assert_eq!(back[0].keys().next().unwrap(), &Key(vec![u64::MAX as i128]));
}

#[test]
fn update_decode_rejects_a_malformed_prior_instead_of_panicking() {
    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Unsigned),
    ]);
    let c = &s.collections[0];
    assert_eq!(
        wire_format::update::decode(c, &Record::new(vec![]), &[0x01, 0x01], 0),
        Err(WireError::Corrupt("record arity does not match schema"))
    );
    assert_eq!(
        wire_format::update::decode(
            c,
            &Record::new(vec![Value::Unsigned(0), Value::Unsigned(0)]),
            &[0x01, 0x01],
            0
        ),
        Err(WireError::Corrupt("record arity does not match schema"))
    );
    let (back, _) =
        wire_format::update::decode(c, &Record::new(vec![Value::Unsigned(0)]), &[0x01, 0x01], 0)
            .unwrap();
    assert_eq!(back.values, vec![Value::Unsigned(1)]);
}

#[test]
fn the_tick_grid_is_bounded_at_a_representable_shift() {
    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Unsigned),
    ]);
    assert_eq!(
        s.validate(64),
        Err(SchemaError::TicksPerHourTooLarge {
            ticks_per_hour_log2: 64
        })
    );
    assert_eq!(
        s.validate(63),
        Ok(()),
        "63 is the largest representable grid"
    );

    let mut f = Field::new(2, Kind::Unsigned);
    f.derived = Some(Derived { c: 1, u: 64 });
    let s = keyed(vec![Field::new(1, Kind::Unsigned).key(0), f]);
    assert!(s.validate(63).is_err(), "u = 64 exceeds a 63 grid");
}

#[test]
fn an_event_sequence_index_must_name_a_frontier_component() {
    let with = |ids: Vec<u32>, sequence_index: u32| Schema {
        domain: 1,
        cadence_log2: 0,
        collections: vec![],
        event_part: Some(EventPart {
            collection_id: 9,
            fields: vec![Field::new(1, Kind::Unsigned)],
            frontier_component_ids: ids,
            sequence_index,
        }),
    };
    assert_eq!(
        with(vec![10], 1).validate(3),
        Err(SchemaError::SequenceIndexOutOfRange {
            sequence_index: 1,
            components: 1
        })
    );
    assert_eq!(
        with(vec![], 0).validate(3),
        Err(SchemaError::SequenceIndexOutOfRange {
            sequence_index: 0,
            components: 0
        })
    );
    assert_eq!(with(vec![10, 11], 1).validate(3), Ok(()));
}
