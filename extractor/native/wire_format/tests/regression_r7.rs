use wire_format::body::{
    decode_baseline, decode_delta, encode_delta, Key, KeyedDelta, KeyedState, Row,
};
use wire_format::record::Record;
use wire_format::schema::{Collection, Field, Schema, Shape};
use wire_format::value::Value;
use wire_format::{Kind, TickGrid, WireError};

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
fn the_key_chain_cannot_decode_an_out_of_range_component() {
    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Unsigned),
    ]);
    let mut bytes = vec![0x02];
    wire_format::varint::write_varint(&mut bytes, u64::MAX);
    bytes.push(0x00);
    wire_format::varint::write_varint(&mut bytes, 1);
    bytes.push(0x00);
    assert_eq!(
        decode_baseline(&s, &bytes, 0),
        Err(WireError::Corrupt(
            "key component is outside its kind's range"
        ))
    );

    let mut ok = vec![0x01];
    wire_format::varint::write_varint(&mut ok, u64::MAX);
    ok.push(0x00);
    let (out, _) = decode_baseline(&s, &ok, 0).unwrap();
    assert_eq!(out[0].keys().next().unwrap(), &Key(vec![u64::MAX as i128]));
}

#[test]
fn update_decode_validates_the_prior_it_is_handed() {
    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Enum).enum_card(3),
        Field::new(3, Kind::Boolean),
    ]);
    let c = &s.collections[0];
    let bad_prior = Record::new(vec![Value::Enum(5), Value::Bool(false)]);
    assert_eq!(
        wire_format::update::decode(c, &bad_prior, &[0x02, 0x01], 0),
        Err(WireError::Corrupt("enum code >= declared cardinality")),
        "used to return Ok, preserving Enum(5) in reconstructed state"
    );
    let good = Record::new(vec![Value::Enum(2), Value::Bool(false)]);
    let (back, _) = wire_format::update::decode(c, &good, &[0x02, 0x01], 0).unwrap();
    assert_eq!(back.values, vec![Value::Enum(2), Value::Bool(true)]);
}

#[test]
fn decode_delta_checks_the_prior_list_against_the_schema() {
    let two = Schema {
        domain: 1,
        cadence_log2: 0,
        event_part: None,
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
    };
    let bytes = [0x01, 0x01, 0x01, 0x01, 0x00];
    assert_eq!(
        decode_delta(&two, &[KeyedState::new()], &bytes, 0),
        Err(WireError::Corrupt(
            "prior state list does not match the schema"
        ))
    );
    assert!(decode_delta(&two, &[KeyedState::new(), KeyedState::new()], &bytes, 0).is_ok());
}

#[test]
fn encode_delta_refuses_a_key_that_is_both_changed_and_tombstoned() {
    use wire_format::body::MinimalityError;
    let s = keyed(vec![
        Field::new(1, Kind::Unsigned).key(0),
        Field::new(2, Kind::Unsigned),
    ]);
    let mut prior = KeyedState::new();
    prior.insert(Key(vec![1]), Record::new(vec![Value::Unsigned(0)]));

    let delta = KeyedDelta {
        changed: vec![Row {
            key: Key(vec![1]),
            record: Record::new(vec![Value::Unsigned(1)]),
        }],
        tombstones: vec![Key(vec![1])],
    };
    let mut out = Vec::new();
    assert_eq!(
        encode_delta(&s, &[prior], &[Some(delta)], &mut out),
        Err(MinimalityError::KeyBothChangedAndTombstoned { collection_id: 1 }),
        "the decoder rejects these bytes, so the encoder must not produce them"
    );
}

#[test]
fn an_unrepresentable_tick_grid_cannot_be_constructed() {
    assert_eq!(
        TickGrid::new(64),
        Err(WireError::Corrupt("ticks_per_hour_log2 exceeds 63"))
    );
    let grid = TickGrid::new(63).unwrap();
    assert_eq!(grid.log2(), 63);
    assert_eq!(grid.ticks_per_hour(), 1u64 << 63);
    assert_eq!(grid.shifted_clock(u64::MAX, 64), 0);
    assert_eq!(grid.shifted_clock(8, 3), 1);
}
