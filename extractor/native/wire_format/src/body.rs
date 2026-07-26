//! `BASELINE` and `DELTA` bodies, per `WIRE_FORMAT.md` §7.
//!
//! The minimality rules in §7.1 are enforced here rather than left to the caller:
//! every one of them exists because the alternative is a second legal encoding of
//! "nothing happened", which both splits byte-identity and wastes bytes.

use crate::bitmap::Bitmap;
use crate::record::Record;
use crate::schema::{Collection, Kind, Schema, Shape};
use crate::value::Value;
use crate::varint::{read_varint, read_zigzag, write_varint, write_zigzag, Result, WireError};
use std::collections::BTreeMap;

/// A key tuple. §7.4 restricts components to kinds `0`, `1` and `5`, the three
/// that encode as a self-delimiting varint or zigzag on their own.
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct Key(pub Vec<i128>);

impl Key {
    pub fn first(&self) -> i128 {
        self.0.first().copied().unwrap_or(0)
    }
}

/// One row of a keyed collection: its key, then a record of the non-key fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: Key,
    pub record: Record,
}

/// The reconstructed content of one keyed collection.
pub type KeyedState = BTreeMap<Key, Record>;

/// §7.4 key components are `varint` or `zigzag`, so each has a representable
/// range. `Key` holds `i128` to carry both without a per-kind enum, which means
/// the range has to be checked rather than assumed — an unchecked `as u64` maps
/// `u64::MAX + 1` onto `0` and emits another key's bytes.
pub fn check_key_component(kind: Kind, value: i128) -> Result<()> {
    let ok = match kind {
        Kind::Signed => value >= i64::MIN as i128 && value <= i64::MAX as i128,
        _ => value >= 0 && value <= u64::MAX as i128,
    };
    if ok {
        Ok(())
    } else {
        Err(WireError::Corrupt("key component is outside its kind's range"))
    }
}

fn write_key_component(out: &mut Vec<u8>, kind: Kind, value: i128) {
    match kind {
        Kind::Signed => write_zigzag(out, value as i64),
        _ => write_varint(out, value as u64),
    }
}

/// Check a whole key against the schema: arity, then every component's range.
pub fn check_key(kinds: &[Kind], key: &Key) -> Result<()> {
    // A keyed set with no key fields is rejected by `Schema::validate`, but the
    // encoders are public and indexed `kinds[0]` before anything checked it.
    if kinds.is_empty() {
        return Err(WireError::Corrupt("keyed collection has no key fields"));
    }
    if key.0.len() != kinds.len() {
        return Err(WireError::Corrupt("key arity does not match schema"));
    }
    for (kind, component) in kinds.iter().zip(&key.0) {
        check_key_component(*kind, *component)?;
    }
    Ok(())
}

fn read_key_component(buf: &[u8], pos: usize, kind: Kind) -> Result<(i128, usize)> {
    Ok(match kind {
        Kind::Signed => {
            let (v, p) = read_zigzag(buf, pos)?;
            (v as i128, p)
        }
        _ => {
            let (v, p) = read_varint(buf, pos)?;
            (v as i128, p)
        }
    })
}

/// §7.4. In a sorted list the first component is non-decreasing, so its
/// difference is never negative and a plain `varint` beats `zigzag` by a bit.
/// Later components jump around under sort and would not compress, so they are
/// absolute.
fn write_sorted_keys(out: &mut Vec<u8>, collection: &Collection, keys: &[Key]) -> Result<()> {
    let kinds: Vec<Kind> = collection.key_fields().iter().map(|f| f.kind).collect();
    let mut previous_first: Option<i128> = None;
    let mut previous_key: Option<&Key> = None;
    for key in keys {
        check_key(&kinds, key)?;
        // §2 sorts by the FULL key tuple, so checking the first component alone
        // would accept (1,5) before (1,3).
        if let Some(prev) = previous_key {
            if key <= prev {
                return Err(WireError::Corrupt("key list is not sorted by the full tuple"));
            }
        }
        previous_key = Some(key);
        match previous_first {
            None => write_key_component(out, kinds[0], key.first()),
            Some(prev) => {
                let diff = key.first() - prev;
                if diff < 0 {
                    return Err(WireError::Corrupt("key list is not sorted ascending"));
                }
                write_varint(out, diff as u64);
            }
        }
        previous_first = Some(key.first());
        for (i, component) in key.0.iter().enumerate().skip(1) {
            write_key_component(out, kinds[i], *component);
        }
    }
    Ok(())
}

fn read_sorted_key(
    buf: &[u8],
    pos: usize,
    collection: &Collection,
    previous_first: Option<i128>,
) -> Result<(Key, usize)> {
    let kinds: Vec<Kind> = collection.key_fields().iter().map(|f| f.kind).collect();
    if kinds.is_empty() {
        return Err(WireError::Corrupt("keyed collection has no key fields"));
    }
    let (first, mut pos) = match previous_first {
        None => read_key_component(buf, pos, kinds[0])?,
        Some(prev) => {
            let (diff, p) = read_varint(buf, pos)?;
            (prev + diff as i128, p)
        }
    };
    let mut components = vec![first];
    for kind in kinds.iter().skip(1) {
        let (value, p) = read_key_component(buf, pos, *kind)?;
        components.push(value);
        pos = p;
    }
    // The first component is `prev + diff`, both off the wire, so the chain can
    // manufacture a value no encoder would accept — first key `u64::MAX` then a
    // difference of 1. The result is still correctly sorted, so the ordering
    // checks do not catch it; the range check has to run on read as well.
    let out = Key(components);
    check_key(&kinds, &out)?;
    Ok((out, pos))
}

/// `BASELINE` body: every collection in schema order, each a row count then its
/// rows in canonical order (§2).
pub fn encode_baseline(schema: &Schema, states: &[KeyedState], out: &mut Vec<u8>) -> Result<()> {
    if states.len() != schema.collections.len() {
        return Err(WireError::Corrupt("baseline arity does not match schema"));
    }
    for (collection, state) in schema.collections.iter().zip(states) {
        // §7.3: ordered shapes carry source order, which a key-sorted map has
        // already destroyed, and an occurrence list has no key to index at all.
        // Encoding one through this path would silently reorder or panic.
        if !matches!(collection.shape, Shape::KeyedSet) {
            return Err(WireError::Corrupt(
                "ordered collections are not representable as a keyed state",
            ));
        }
        let kinds_check: Vec<Kind> = collection.key_fields().iter().map(|f| f.kind).collect();
        if kinds_check.is_empty() {
            return Err(WireError::Corrupt("keyed collection has no key fields"));
        }
        write_varint(out, state.len() as u64);
        let keys: Vec<Key> = state.keys().cloned().collect();
        // Keys and records interleave, so the key delta chain is written inline.
        let mut previous_first: Option<i128> = None;
        let kinds: Vec<Kind> = collection.key_fields().iter().map(|f| f.kind).collect();
        for key in &keys {
            // A `BTreeMap` guarantees ascending unique keys, but not that they
            // have the schema's arity — and the component loop below indexes
            // `kinds` directly, so a long key panics and a short one emits bytes
            // no reader can parse.
            check_key(&kinds, key)?;
            match previous_first {
                None => write_key_component(out, kinds[0], key.first()),
                Some(prev) => write_varint(out, (key.first() - prev) as u64),
            }
            previous_first = Some(key.first());
            for (i, component) in key.0.iter().enumerate().skip(1) {
                write_key_component(out, kinds[i], *component);
            }
            state[key].encode(collection, out)?;
        }
    }
    Ok(())
}

pub fn decode_baseline(schema: &Schema, buf: &[u8], mut pos: usize) -> Result<(Vec<KeyedState>, usize)> {
    let mut out = Vec::new();
    for collection in &schema.collections {
        // The encoder rejects ordered shapes; the decoder must too, or it reads
        // them with keyed-set rules. §7.4's ordered-keyed keys are absolute, so a
        // delta-decoded key list comes back wrong (5, 6 reads as 5, 11), and an
        // occurrence list has no key fields at all, so the key reader indexes an
        // empty list and panics.
        if !matches!(collection.shape, Shape::KeyedSet) {
            return Err(WireError::Corrupt(
                "ordered collections have no keyed-set baseline grammar",
            ));
        }
        let (count, p) = read_varint(buf, pos)?;
        pos = p;
        let mut state = KeyedState::new();
        let mut previous_first: Option<i128> = None;
        // §2 orders a baseline's rows by the full key tuple. Tracking only the
        // first component would accept (1,5) before (1,3) and let the map
        // silently reorder them, and a repeated key would overwrite a row —
        // three byte sequences decoding to two different states.
        let mut previous_key: Option<Key> = None;
        for _ in 0..count {
            let (key, p) = read_sorted_key(buf, pos, collection, previous_first)?;
            if let Some(prev) = &previous_key {
                if key <= *prev {
                    return Err(WireError::Corrupt("baseline rows are not ascending by key"));
                }
            }
            previous_key = Some(key.clone());
            previous_first = Some(key.first());
            let (record, p2) = Record::decode(collection, buf, p)?;
            pos = p2;
            state.insert(key, record);
        }
        out.push(state);
    }
    Ok((out, pos))
}

/// What one collection contributes to a `DELTA` (§7).
#[derive(Clone, Debug, PartialEq)]
pub struct KeyedDelta {
    /// Exactly the keys that are new plus the surviving keys whose record differs.
    pub changed: Vec<Row>,
    /// Exactly the keys present before and absent now.
    pub tombstones: Vec<Key>,
}

impl KeyedDelta {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.tombstones.is_empty()
    }
}

/// §7.1 violations. Each admits a second encoding of the same state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MinimalityError {
    /// The delta or prior-state list does not have one entry per collection.
    ArityMismatch,
    /// A `DELTA` whose change bitmap is all zero — that state is `UNCHANGED`.
    NoCollectionChanged,
    /// A set bit whose keyed contribution carries nothing.
    EmptyContribution { collection_id: u32 },
    /// A key appearing twice in one list.
    DuplicateKey { collection_id: u32 },
    /// A key list not ascending by the full tuple (§2). Distinct from a
    /// duplicate: `(1,5)` before `(1,3)` repeats no key and is still illegal.
    KeysNotSorted { collection_id: u32 },
    /// A key whose component count is not the schema's.
    KeyArityMismatch { collection_id: u32 },
    /// A row that could not be encoded — the schema and the record disagree.
    RowEncodingFailed { collection_id: u32, cause: WireError },
    /// A key both updated and deleted this tick.
    KeyBothChangedAndTombstoned { collection_id: u32 },
    /// A tombstone for a key that was already absent.
    TombstoneForAbsentKey { collection_id: u32 },
    /// A key that vanished without a tombstone.
    MissingTombstone { collection_id: u32 },
    /// A changed row whose record is identical to the reconstructed prior one.
    UnchangedRowEmitted { collection_id: u32 },
    /// A row that changed but was not emitted.
    MissingChangedRow { collection_id: u32 },
    /// §7: only a keyed set has a delta grammar.
    UnsupportedShape { collection_id: u32 },
    /// A changed row for a key that is not in the next state.
    RowNotInNextState { collection_id: u32 },
    /// A changed row whose record is not the next state's record for that key.
    RowDoesNotMatchNextState { collection_id: u32 },
}

/// Build the one minimal delta from prior to next, per §7.1. This is the differ:
/// producing it by rule rather than by hand is what makes minimality checkable.
pub fn diff(collection: &Collection, prior: &KeyedState, next: &KeyedState) -> KeyedDelta {
    let mut changed = Vec::new();
    for (key, record) in next {
        match prior.get(key) {
            // Stored equality, not `==`: a source value that moves within its own
            // quantization step does not change the bytes, so emitting it would
            // be a delta that reconstructs to the state already held.
            Some(old) if old.stored_eq(collection, record) => {}
            _ => changed.push(Row { key: key.clone(), record: record.clone() }),
        }
    }
    let tombstones = prior
        .keys()
        .filter(|k| !next.contains_key(*k))
        .cloned()
        .collect();
    KeyedDelta { changed, tombstones }
}

/// Verify a delta is the minimal one for this transition (§7.1).
pub fn check_minimal(
    collection: &Collection,
    prior: &KeyedState,
    next: &KeyedState,
    delta: &KeyedDelta,
) -> core::result::Result<(), MinimalityError> {
    let collection_id = collection.collection_id;
    let mut seen = Vec::new();
    for row in &delta.changed {
        if seen.contains(&row.key) {
            return Err(MinimalityError::DuplicateKey { collection_id });
        }
        seen.push(row.key.clone());
        if delta.tombstones.contains(&row.key) {
            return Err(MinimalityError::KeyBothChangedAndTombstoned { collection_id });
        }
        match next.get(&row.key) {
            // A row not in the next state has no business being emitted at all.
            None => return Err(MinimalityError::RowNotInNextState { collection_id }),
            // And an emitted row must BE the next state, not merely differ from
            // the prior one — otherwise a writer can emit any value it likes.
            Some(expected) if !expected.stored_eq(collection, &row.record) => {
                return Err(MinimalityError::RowDoesNotMatchNextState { collection_id })
            }
            _ => {}
        }
        if prior.get(&row.key).is_some_and(|p| p.stored_eq(collection, &row.record)) {
            return Err(MinimalityError::UnchangedRowEmitted { collection_id });
        }
    }
    let mut seen_tombs = Vec::new();
    for key in &delta.tombstones {
        if seen_tombs.contains(key) {
            return Err(MinimalityError::DuplicateKey { collection_id });
        }
        seen_tombs.push(key.clone());
        if !prior.contains_key(key) {
            return Err(MinimalityError::TombstoneForAbsentKey { collection_id });
        }
        if next.contains_key(key) {
            return Err(MinimalityError::TombstoneForAbsentKey { collection_id });
        }
    }
    for key in prior.keys() {
        if !next.contains_key(key) && !delta.tombstones.contains(key) {
            return Err(MinimalityError::MissingTombstone { collection_id });
        }
    }
    for (key, record) in next {
        let differs = !prior.get(key).is_some_and(|p| p.stored_eq(collection, record));
        if differs && !delta.changed.iter().any(|r| &r.key == key) {
            return Err(MinimalityError::MissingChangedRow { collection_id });
        }
    }
    Ok(())
}

/// `DELTA` body: a collection change bitmap, then a contribution per set bit.
pub fn encode_delta(
    schema: &Schema,
    priors: &[KeyedState],
    deltas: &[Option<KeyedDelta>],
    out: &mut Vec<u8>,
) -> core::result::Result<(), MinimalityError> {
    // The bitmap is built from every delta, but the contributions are written by
    // a three-way `zip` that stops at the shortest input. A `priors` shorter than
    // the schema would therefore set a bit and then write no bytes for it — the
    // exact "set bit, empty contribution" state §7.1 forbids, produced by an
    // arity mismatch rather than by a bad delta.
    if deltas.len() != schema.collections.len() || priors.len() != schema.collections.len() {
        return Err(MinimalityError::ArityMismatch);
    }
    let mut bitmap = Bitmap::with_len(schema.collections.len());
    for (i, delta) in deltas.iter().enumerate() {
        bitmap.set(i, delta.is_some());
    }
    if bitmap.count_set() == 0 {
        return Err(MinimalityError::NoCollectionChanged);
    }
    bitmap.encode(out);

    for ((collection, delta), prior) in schema.collections.iter().zip(deltas).zip(priors) {
        let Some(delta) = delta else { continue };
        if delta.is_empty() {
            return Err(MinimalityError::EmptyContribution {
                collection_id: collection.collection_id,
            });
        }
        if !matches!(collection.shape, Shape::KeyedSet) {
            // §7: ordered shapes are full replacement and have no delta grammar.
            // Emitting a set bit with no bytes would leave the reader parsing the
            // next collection as this one, so this is a hard error rather than a
            // silent skip.
            return Err(MinimalityError::UnsupportedShape {
                collection_id: collection.collection_id,
            });
        }
        // §7.1: a key cannot be both updated and deleted in the same tick. The
        // decoder rejects it, so an encoder that does not is an encoder that
        // emits a stream its own decoder refuses. `check_minimal` also catches
        // this, but it is a separate call a caller can skip.
        for row in &delta.changed {
            if delta.tombstones.contains(&row.key) {
                return Err(MinimalityError::KeyBothChangedAndTombstoned {
                    collection_id: collection.collection_id,
                });
            }
        }
        // §7.1's other prior-dependent rule, for the same reason: the decoder
        // holds the reconstructed prior state and rejects a tombstone for a key
        // that was already absent, so the encoder must refuse to emit one.
        for key in &delta.tombstones {
            if !prior.contains_key(key) {
                return Err(MinimalityError::TombstoneForAbsentKey {
                    collection_id: collection.collection_id,
                });
            }
        }
        write_varint(out, delta.changed.len() as u64);
        let mut previous_first: Option<i128> = None;
        let kinds: Vec<Kind> = collection.key_fields().iter().map(|f| f.kind).collect();
        let mut previous_key: Option<&Key> = None;
        for row in &delta.changed {
            // The tombstone list gets these two checks from `write_sorted_keys`;
            // the changed list interleaves keys with records so it writes its own
            // key chain, and has to repeat them or it silently emits bytes no
            // reader can parse.
            check_key(&kinds, &row.key).map_err(|_| MinimalityError::KeyArityMismatch {
                collection_id: collection.collection_id,
            })?;
            if let Some(prev) = previous_key {
                if &row.key <= prev {
                    return Err(MinimalityError::KeysNotSorted {
                        collection_id: collection.collection_id,
                    });
                }
            }
            previous_key = Some(&row.key);
            match previous_first {
                None => write_key_component(out, kinds[0], row.key.first()),
                Some(prev) => write_varint(out, (row.key.first() - prev) as u64),
            }
            previous_first = Some(row.key.first());
            for (i, component) in row.key.0.iter().enumerate().skip(1) {
                write_key_component(out, kinds[i], *component);
            }
            // §7.2: key absent in the reconstructed prior state is an insert and
            // carries a full record; key present is an update and carries only
            // what moved. The reader makes the same lookup, so no discriminator
            // byte is needed.
            let encoded = match prior.get(&row.key) {
                None => row.record.encode(collection, out),
                Some(previous) => crate::update::encode(collection, previous, &row.record, out),
            };
            encoded.map_err(|cause| MinimalityError::RowEncodingFailed {
                collection_id: collection.collection_id,
                cause,
            })?;
        }
        write_varint(out, delta.tombstones.len() as u64);
        write_sorted_keys(out, collection, &delta.tombstones).map_err(|cause| match cause {
            WireError::Corrupt("key arity does not match schema") => {
                MinimalityError::KeyArityMismatch { collection_id: collection.collection_id }
            }
            _ => MinimalityError::KeysNotSorted { collection_id: collection.collection_id },
        })?;
    }
    Ok(())
}

pub fn decode_delta(
    schema: &Schema,
    priors: &[KeyedState],
    buf: &[u8],
    pos: usize,
) -> Result<(Vec<Option<KeyedDelta>>, usize)> {
    // Mirrors the encoder's arity check. Looking a prior up only when its bit is
    // set means a short `priors` decodes fine right up until the bit that needs
    // it, which makes the failure depend on the bytes rather than on the caller.
    if priors.len() != schema.collections.len() {
        return Err(WireError::Corrupt("prior state list does not match the schema"));
    }
    let (bitmap, mut pos) = Bitmap::decode(buf, pos, schema.collections.len())?;
    if bitmap.count_set() == 0 {
        return Err(WireError::Corrupt("DELTA change bitmap is all zero"));
    }
    let mut out = Vec::new();
    for (i, collection) in schema.collections.iter().enumerate() {
        if !bitmap.get(i) {
            out.push(None);
            continue;
        }
        if !matches!(collection.shape, Shape::KeyedSet) {
            return Err(WireError::Corrupt("ordered collection has no delta grammar"));
        }
        let prior = priors
            .get(i)
            .ok_or(WireError::Corrupt("prior state missing for a collection"))?;
        let (changed_count, p) = read_varint(buf, pos)?;
        pos = p;
        let mut changed = Vec::new();
        let mut previous_first: Option<i128> = None;
        // §7.1 and §2 constrain both lists to be strictly ascending by the full
        // key tuple. The encoder enforces it; a reader that does not gives a
        // hostile writer a second encoding of every delta — a zero first-component
        // difference repeats a key outright, and later components are absolute so
        // nothing else stops (1,5) preceding (1,3).
        let mut previous_key: Option<Key> = None;
        for _ in 0..changed_count {
            let (key, p) = read_sorted_key(buf, pos, collection, previous_first)?;
            if let Some(prev) = &previous_key {
                if key <= *prev {
                    return Err(WireError::Corrupt("changed rows are not ascending by key"));
                }
            }
            previous_key = Some(key.clone());
            previous_first = Some(key.first());
            let (record, p2) = match prior.get(&key) {
                None => Record::decode(collection, buf, p)?,
                Some(previous) => crate::update::decode(collection, previous, buf, p)?,
            };
            pos = p2;
            changed.push(Row { key, record });
        }
        let (tomb_count, p) = read_varint(buf, pos)?;
        pos = p;
        let mut tombstones = Vec::new();
        let mut previous_first: Option<i128> = None;
        let mut previous_key: Option<Key> = None;
        for _ in 0..tomb_count {
            let (key, p) = read_sorted_key(buf, pos, collection, previous_first)?;
            if let Some(prev) = &previous_key {
                if key <= *prev {
                    return Err(WireError::Corrupt("tombstones are not ascending by key"));
                }
            }
            // §7.1: a key cannot be both updated and deleted in the same tick,
            // and a tombstone for an already-absent key is a second encoding of
            // the same delta. Both are decidable here because the reader holds
            // the same reconstructed prior state the writer did.
            if changed.iter().any(|r: &Row| r.key == key) {
                return Err(WireError::Corrupt("key is both changed and tombstoned"));
            }
            if !prior.contains_key(&key) {
                return Err(WireError::Corrupt("tombstone for a key that was already absent"));
            }
            previous_key = Some(key.clone());
            previous_first = Some(key.first());
            pos = p;
            tombstones.push(key);
        }
        let delta = KeyedDelta { changed, tombstones };
        if delta.is_empty() {
            return Err(WireError::Corrupt("collection bit set with an empty contribution"));
        }
        out.push(Some(delta));
    }
    Ok((out, pos))
}

/// Replay a delta onto reconstructed state.
pub fn apply(state: &mut KeyedState, delta: &KeyedDelta) {
    for row in &delta.changed {
        state.insert(row.key.clone(), row.record.clone());
    }
    for key in &delta.tombstones {
        state.remove(key);
    }
}

/// Convenience for building a key from unsigned components.
pub fn key(components: &[u64]) -> Key {
    Key(components.iter().map(|c| *c as i128).collect())
}

/// A record whose values are all `Absent`/defaults for a collection — used by
/// tests and fixtures to build rows without naming every field.
pub fn blank_record(collection: &Collection) -> Record {
    Record::new(
        collection
            .value_fields()
            .map(|f| {
                if f.optional {
                    Value::Absent
                } else if f.derived.is_some() {
                    // A derived field's stored value is Signed whatever its live
                    // kind, so building from the kind alone produced a record
                    // whose own encode then failed.
                    Value::Signed(0)
                } else {
                    match f.kind {
                        Kind::Boolean => Value::Bool(false),
                        Kind::Enum => Value::Enum(0),
                        Kind::ContentId => Value::ContentId(0),
                        Kind::Continuous => Value::Continuous(0.0),
                        Kind::Signed => Value::Signed(0),
                        _ => Value::Unsigned(0),
                    }
                }
            })
            .collect(),
    )
}
