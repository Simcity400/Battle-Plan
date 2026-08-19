//! Regression test for a one-sided delta-minimality rule. The decoder rejects a
//! tombstone for a key absent from the prior state, so the encoder must do the same.

use wire_format::body::{
    decode_delta, encode_delta, KeyedDelta, KeyedState, MinimalityError, Row,
};
use wire_format::body::key;
use wire_format::record::Record;
use wire_format::schema::{Collection, Field, Schema, Shape};
use wire_format::value::Value;
use wire_format::Kind;

fn keyed() -> Schema {
    Schema {
        domain: 1,
        cadence_log2: 0,
        event_part: None,
        collections: vec![Collection {
            collection_id: 1,
            shape: Shape::KeyedSet,
            fields: vec![Field::new(1, Kind::Unsigned).key(0), Field::new(2, Kind::Unsigned)],
        }],
    }
}

fn prior_with_key_1() -> KeyedState {
    let mut prior = KeyedState::new();
    prior.insert(key(&[1]), Record::new(vec![Value::Unsigned(0)]));
    prior
}

/// Regression input: prior {1 -> 0}, delta tombstoning key 2.
/// Used to encode to [01, 00, 01, 02], which `decode_delta` rejects.
#[test]
fn encode_delta_refuses_a_tombstone_for_a_prior_absent_key() {
    let s = keyed();
    let delta = KeyedDelta { changed: vec![], tombstones: vec![key(&[2])] };
    let mut out = Vec::new();
    assert_eq!(
        encode_delta(&s, &[prior_with_key_1()], &[Some(delta)], &mut out),
        Err(MinimalityError::TombstoneForAbsentKey { collection_id: 1 }),
        "the decoder rejects these bytes, so the encoder must not produce them"
    );
}

/// The same defect alongside a legal changed row, so the fix cannot be an
/// empty-contribution special case.
#[test]
fn the_refusal_holds_with_a_legal_changed_row_alongside() {
    let s = keyed();
    let delta = KeyedDelta {
        changed: vec![Row { key: key(&[1]), record: Record::new(vec![Value::Unsigned(7)]) }],
        tombstones: vec![key(&[2])],
    };
    let mut out = Vec::new();
    assert_eq!(
        encode_delta(&s, &[prior_with_key_1()], &[Some(delta)], &mut out),
        Err(MinimalityError::TombstoneForAbsentKey { collection_id: 1 })
    );
}

/// The rule must not over-reach: a tombstone for a key the prior state does
/// hold is the legal case, and it must still round-trip.
#[test]
fn a_legal_tombstone_still_encodes_and_round_trips() {
    let s = keyed();
    let prior = prior_with_key_1();
    let delta = KeyedDelta { changed: vec![], tombstones: vec![key(&[1])] };
    let mut out = Vec::new();
    encode_delta(&s, &[prior.clone()], &[Some(delta.clone())], &mut out).unwrap();
    let (back, consumed) = decode_delta(&s, &[prior], &out, 0).unwrap();
    assert_eq!(consumed, out.len());
    assert_eq!(back, vec![Some(delta)]);
}
