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
use std::collections::{BTreeMap, BTreeSet};

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

/// The reconstructed content of one collection, preserving the semantics its
/// §7.3 shape declares. A map is correct only for a keyed set: either ordered
/// variant would lose source order, and an occurrence list would additionally
/// collapse legitimate duplicates.
#[derive(Clone, Debug, PartialEq)]
pub enum CollectionState {
    Keyed(KeyedState),
    OrderedKeyed(Vec<Row>),
    OrderedOccurrence(Vec<Record>),
}

impl CollectionState {
    pub fn len(&self) -> usize {
        match self {
            Self::Keyed(rows) => rows.len(),
            Self::OrderedKeyed(rows) => rows.len(),
            Self::OrderedOccurrence(rows) => rows.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl From<KeyedState> for CollectionState {
    fn from(value: KeyedState) -> Self {
        Self::Keyed(value)
    }
}

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

/// §7.4 ordered-keyed rows carry every component absolute. There is no previous
/// key because source order need not be monotone in any component.
fn write_absolute_key(out: &mut Vec<u8>, collection: &Collection, key: &Key) -> Result<()> {
    let kinds: Vec<Kind> = collection.key_fields().iter().map(|f| f.kind).collect();
    check_key(&kinds, key)?;
    for (kind, component) in kinds.iter().zip(&key.0) {
        write_key_component(out, *kind, *component);
    }
    Ok(())
}

fn read_absolute_key(
    buf: &[u8],
    mut pos: usize,
    collection: &Collection,
) -> Result<(Key, usize)> {
    let kinds: Vec<Kind> = collection.key_fields().iter().map(|f| f.kind).collect();
    if kinds.is_empty() {
        return Err(WireError::Corrupt("keyed collection has no key fields"));
    }
    let mut components = Vec::with_capacity(kinds.len());
    for kind in &kinds {
        let (component, p) = read_key_component(buf, pos, *kind)?;
        components.push(component);
        pos = p;
    }
    let key = Key(components);
    // An absolute component is decoded by its kind, but this check is kept on
    // the read side deliberately: §7.4's range rule is a property of every key
    // path, not an assumption tied to today's varint implementation.
    check_key(&kinds, &key)?;
    Ok((key, pos))
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

/// Encode one collection's complete contribution. This is deliberately shared
/// by `BASELINE` and ordered `DELTA`: §7 requires the replacement bytes to be
/// identical, so a second ordered-row encoder would itself violate §2.
fn encode_complete_collection(
    collection: &Collection,
    state: &CollectionState,
    out: &mut Vec<u8>,
) -> Result<()> {
    match (collection.shape, state) {
        (Shape::KeyedSet, CollectionState::Keyed(rows)) => {
            write_varint(out, rows.len() as u64);
            let kinds: Vec<Kind> = collection.key_fields().iter().map(|f| f.kind).collect();
            if kinds.is_empty() {
                return Err(WireError::Corrupt("keyed collection has no key fields"));
            }
            let mut previous_first: Option<i128> = None;
            for (key, record) in rows {
                check_key(&kinds, key)?;
                match previous_first {
                    None => write_key_component(out, kinds[0], key.first()),
                    Some(prev) => write_varint(out, (key.first() - prev) as u64),
                }
                previous_first = Some(key.first());
                for (i, component) in key.0.iter().enumerate().skip(1) {
                    write_key_component(out, kinds[i], *component);
                }
                record.encode(collection, out)?;
            }
        }
        (Shape::OrderedKeyedList, CollectionState::OrderedKeyed(rows)) => {
            write_varint(out, rows.len() as u64);
            let mut seen = BTreeSet::new();
            for row in rows {
                if !seen.insert(row.key.clone()) {
                    return Err(WireError::Corrupt("ordered-keyed list contains a duplicate key"));
                }
                write_absolute_key(out, collection, &row.key)?;
                row.record.encode(collection, out)?;
            }
        }
        (Shape::OrderedOccurrenceList, CollectionState::OrderedOccurrence(rows)) => {
            write_varint(out, rows.len() as u64);
            for record in rows {
                record.encode(collection, out)?;
            }
        }
        _ => return Err(WireError::Corrupt("collection state shape does not match schema")),
    }
    Ok(())
}

fn decode_complete_collection(
    collection: &Collection,
    buf: &[u8],
    mut pos: usize,
) -> Result<(CollectionState, usize)> {
    let (count, p) = read_varint(buf, pos)?;
    pos = p;
    // `Schema::validate` refuses a fieldless collection, so every legal row
    // consumes at least one byte. Bound the untrusted count by ground truth
    // before allocating or looping. In a multi-collection body `remaining`
    // deliberately includes later collections, so this is a loose upper bound;
    // that is sound because a guard needs an upper bound, not a collection extent
    // the wire does not carry. Keeping this structural bound as well means even a
    // caller that hands this public decoder an invalid future schema cannot turn
    // a wire count into unbounded work.
    let remaining = buf.len() - pos;
    if count > remaining as u64 {
        return Err(WireError::Corrupt(
            "collection row count exceeds remaining body bytes",
        ));
    }
    match collection.shape {
        Shape::KeyedSet => {
            let mut state = KeyedState::new();
            let mut previous_first: Option<i128> = None;
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
            Ok((CollectionState::Keyed(state), pos))
        }
        Shape::OrderedKeyedList => {
            // The count is untrusted. Reserving it up front would let a tiny
            // hostile body request a host-sized allocation before the first row
            // had even been proved present.
            let mut rows = Vec::new();
            let mut seen = BTreeSet::new();
            for _ in 0..count {
                let (key, p) = read_absolute_key(buf, pos, collection)?;
                if !seen.insert(key.clone()) {
                    return Err(WireError::Corrupt(
                        "ordered-keyed list contains a duplicate key",
                    ));
                }
                let (record, p2) = Record::decode(collection, buf, p)?;
                pos = p2;
                rows.push(Row { key, record });
            }
            Ok((CollectionState::OrderedKeyed(rows), pos))
        }
        Shape::OrderedOccurrenceList => {
            let mut rows = Vec::new();
            for _ in 0..count {
                let (record, p) = Record::decode(collection, buf, pos)?;
                pos = p;
                rows.push(record);
            }
            Ok((CollectionState::OrderedOccurrence(rows), pos))
        }
    }
}

fn complete_collection_bytes(
    collection: &Collection,
    state: &CollectionState,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    encode_complete_collection(collection, state, &mut bytes)?;
    Ok(bytes)
}

/// `BASELINE` body for all §7.3 shapes.
pub fn encode_shaped_baseline(
    schema: &Schema,
    states: &[CollectionState],
    out: &mut Vec<u8>,
) -> Result<()> {
    if states.len() != schema.collections.len() {
        return Err(WireError::Corrupt("baseline arity does not match schema"));
    }
    for (collection, state) in schema.collections.iter().zip(states) {
        encode_complete_collection(collection, state, out)?;
    }
    Ok(())
}

pub fn decode_shaped_baseline(
    schema: &Schema,
    buf: &[u8],
    mut pos: usize,
) -> Result<(Vec<CollectionState>, usize)> {
    let mut out = Vec::with_capacity(schema.collections.len());
    for collection in &schema.collections {
        let (state, p) = decode_complete_collection(collection, buf, pos)?;
        pos = p;
        out.push(state);
    }
    Ok((out, pos))
}

/// Keyed-only compatibility entry point. The other domain modules keep their
/// existing call sites; the bytes still come from the single shaped grammar.
pub fn encode_baseline(schema: &Schema, states: &[KeyedState], out: &mut Vec<u8>) -> Result<()> {
    let shaped: Vec<CollectionState> = states.iter().cloned().map(CollectionState::Keyed).collect();
    encode_shaped_baseline(schema, &shaped, out)
}

pub fn decode_baseline(
    schema: &Schema,
    buf: &[u8],
    pos: usize,
) -> Result<(Vec<KeyedState>, usize)> {
    let (states, pos) = decode_shaped_baseline(schema, buf, pos)?;
    let mut keyed = Vec::with_capacity(states.len());
    for state in states {
        match state {
            CollectionState::Keyed(state) => keyed.push(state),
            _ => {
                return Err(WireError::Corrupt(
                    "ordered collection is not representable as a keyed state",
                ))
            }
        }
    }
    Ok((keyed, pos))
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

/// What one collection contributes to a shaped `DELTA`. Ordered variants are
/// complete replacement content; they deliberately have no changed-row or
/// tombstone form (§7.3).
#[derive(Clone, Debug, PartialEq)]
pub enum CollectionDelta {
    Keyed(KeyedDelta),
    OrderedKeyed(Vec<Row>),
    OrderedOccurrence(Vec<Record>),
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
    /// The typed state or contribution does not match the schema's shape.
    ShapeMismatch { collection_id: u32 },
    /// An ordered set bit re-emits the reconstructed prior content (§7.1).
    ReplacementMatchesPrior { collection_id: u32 },
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

fn encode_keyed_contribution(
    collection: &Collection,
    prior: &KeyedState,
    delta: &KeyedDelta,
    out: &mut Vec<u8>,
) -> core::result::Result<(), MinimalityError> {
    let collection_id = collection.collection_id;
    if delta.is_empty() {
        return Err(MinimalityError::EmptyContribution { collection_id });
    }
    for row in &delta.changed {
        if delta.tombstones.contains(&row.key) {
            return Err(MinimalityError::KeyBothChangedAndTombstoned { collection_id });
        }
    }
    for key in &delta.tombstones {
        if !prior.contains_key(key) {
            return Err(MinimalityError::TombstoneForAbsentKey { collection_id });
        }
    }
    write_varint(out, delta.changed.len() as u64);
    let mut previous_first: Option<i128> = None;
    let kinds: Vec<Kind> = collection.key_fields().iter().map(|f| f.kind).collect();
    let mut previous_key: Option<&Key> = None;
    for row in &delta.changed {
        check_key(&kinds, &row.key).map_err(|cause| match cause {
            WireError::Corrupt("key arity does not match schema") => {
                MinimalityError::KeyArityMismatch { collection_id }
            }
            cause => MinimalityError::RowEncodingFailed {
                collection_id,
                cause,
            },
        })?;
        if let Some(prev) = previous_key {
            if &row.key <= prev {
                return Err(MinimalityError::KeysNotSorted { collection_id });
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
        let encoded = match prior.get(&row.key) {
            None => row.record.encode(collection, out),
            Some(previous) => crate::update::encode(collection, previous, &row.record, out),
        };
        encoded.map_err(|cause| MinimalityError::RowEncodingFailed { collection_id, cause })?;
    }
    write_varint(out, delta.tombstones.len() as u64);
    write_sorted_keys(out, collection, &delta.tombstones).map_err(|cause| match cause {
        WireError::Corrupt("key arity does not match schema") => {
            MinimalityError::KeyArityMismatch { collection_id }
        }
        _ => MinimalityError::KeysNotSorted { collection_id },
    })?;
    Ok(())
}

fn decode_keyed_contribution(
    collection: &Collection,
    prior: &KeyedState,
    buf: &[u8],
    mut pos: usize,
) -> Result<(KeyedDelta, usize)> {
    let (changed_count, p) = read_varint(buf, pos)?;
    pos = p;
    let mut changed = Vec::new();
    let mut previous_first: Option<i128> = None;
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
    Ok((delta, pos))
}

/// Shared `DELTA` framing: arity, change bitmap and contribution walk. The
/// compatibility API supplies borrowed keyed states to this same walk, avoiding
/// a deep clone of the reconstructed world without creating a second keyed
/// grammar.
fn encode_delta_frame(
    schema: &Schema,
    prior_len: usize,
    delta_len: usize,
    out: &mut Vec<u8>,
    is_changed: impl Fn(usize) -> bool,
    mut encode_contribution: impl FnMut(
        usize,
        &Collection,
        &mut Vec<u8>,
    ) -> core::result::Result<(), MinimalityError>,
) -> core::result::Result<(), MinimalityError> {
    if delta_len != schema.collections.len() || prior_len != schema.collections.len() {
        return Err(MinimalityError::ArityMismatch);
    }
    let mut bitmap = Bitmap::with_len(schema.collections.len());
    for i in 0..schema.collections.len() {
        bitmap.set(i, is_changed(i));
    }
    if bitmap.count_set() == 0 {
        return Err(MinimalityError::NoCollectionChanged);
    }
    bitmap.encode(out);
    for (i, collection) in schema.collections.iter().enumerate() {
        if bitmap.get(i) {
            encode_contribution(i, collection, out)?;
        }
    }
    Ok(())
}

fn decode_delta_frame<T>(
    schema: &Schema,
    prior_len: usize,
    buf: &[u8],
    pos: usize,
    mut decode_contribution: impl FnMut(
        usize,
        &Collection,
        usize,
    ) -> Result<(T, usize)>,
) -> Result<(Vec<Option<T>>, usize)> {
    if prior_len != schema.collections.len() {
        return Err(WireError::Corrupt("prior state list does not match the schema"));
    }
    let (bitmap, mut pos) = Bitmap::decode(buf, pos, schema.collections.len())?;
    if bitmap.count_set() == 0 {
        return Err(WireError::Corrupt("DELTA change bitmap is all zero"));
    }
    let mut out = Vec::with_capacity(schema.collections.len());
    for (i, collection) in schema.collections.iter().enumerate() {
        if bitmap.get(i) {
            let (delta, p) = decode_contribution(i, collection, pos)?;
            pos = p;
            out.push(Some(delta));
        } else {
            out.push(None);
        }
    }
    Ok((out, pos))
}

/// Stored equality for any collection shape. Comparing the one canonical
/// complete contribution makes record quantization and source order part of the
/// answer without reimplementing either rule here.
pub fn collection_stored_eq(
    collection: &Collection,
    left: &CollectionState,
    right: &CollectionState,
) -> Result<bool> {
    Ok(complete_collection_bytes(collection, left)?
        == complete_collection_bytes(collection, right)?)
}

/// Build the sole legal contribution for a transition. Keyed sets use §7.2;
/// ordered shapes replace their complete content when (and only when) it differs.
pub fn diff_shaped(
    collection: &Collection,
    prior: &CollectionState,
    next: &CollectionState,
) -> Result<Option<CollectionDelta>> {
    match (collection.shape, prior, next) {
        (Shape::KeyedSet, CollectionState::Keyed(prior), CollectionState::Keyed(next)) => {
            let delta = diff(collection, prior, next);
            Ok((!delta.is_empty()).then_some(CollectionDelta::Keyed(delta)))
        }
        (
            Shape::OrderedKeyedList,
            CollectionState::OrderedKeyed(_),
            CollectionState::OrderedKeyed(rows),
        ) => Ok((!collection_stored_eq(collection, prior, next)?)
            .then(|| CollectionDelta::OrderedKeyed(rows.clone()))),
        (
            Shape::OrderedOccurrenceList,
            CollectionState::OrderedOccurrence(_),
            CollectionState::OrderedOccurrence(rows),
        ) => Ok((!collection_stored_eq(collection, prior, next)?)
            .then(|| CollectionDelta::OrderedOccurrence(rows.clone()))),
        _ => Err(WireError::Corrupt("collection state shape does not match schema")),
    }
}

/// `DELTA` body for all §7.3 shapes.
pub fn encode_shaped_delta(
    schema: &Schema,
    priors: &[CollectionState],
    deltas: &[Option<CollectionDelta>],
    out: &mut Vec<u8>,
) -> core::result::Result<(), MinimalityError> {
    encode_delta_frame(
        schema,
        priors.len(),
        deltas.len(),
        out,
        |i| deltas[i].is_some(),
        |i, collection, out| {
            let delta = deltas[i]
                .as_ref()
                .expect("the shared bitmap walk calls only set contributions");
            let prior = &priors[i];
            let collection_id = collection.collection_id;
            match (collection.shape, prior, delta) {
                (
                    Shape::KeyedSet,
                    CollectionState::Keyed(prior),
                    CollectionDelta::Keyed(delta),
                ) => encode_keyed_contribution(collection, prior, delta, out)?,
                (
                    Shape::OrderedKeyedList,
                    CollectionState::OrderedKeyed(_),
                    CollectionDelta::OrderedKeyed(rows),
                ) => {
                    let replacement = CollectionState::OrderedKeyed(rows.clone());
                    if collection_stored_eq(collection, prior, &replacement).map_err(|cause| {
                        MinimalityError::RowEncodingFailed { collection_id, cause }
                    })? {
                        return Err(MinimalityError::ReplacementMatchesPrior { collection_id });
                    }
                    encode_complete_collection(collection, &replacement, out).map_err(|cause| {
                        MinimalityError::RowEncodingFailed { collection_id, cause }
                    })?;
                }
                (
                    Shape::OrderedOccurrenceList,
                    CollectionState::OrderedOccurrence(_),
                    CollectionDelta::OrderedOccurrence(rows),
                ) => {
                    let replacement = CollectionState::OrderedOccurrence(rows.clone());
                    if collection_stored_eq(collection, prior, &replacement).map_err(|cause| {
                        MinimalityError::RowEncodingFailed { collection_id, cause }
                    })? {
                        return Err(MinimalityError::ReplacementMatchesPrior { collection_id });
                    }
                    encode_complete_collection(collection, &replacement, out).map_err(|cause| {
                        MinimalityError::RowEncodingFailed { collection_id, cause }
                    })?;
                }
                _ => return Err(MinimalityError::ShapeMismatch { collection_id }),
            }
            Ok(())
        },
    )
}

pub fn decode_shaped_delta(
    schema: &Schema,
    priors: &[CollectionState],
    buf: &[u8],
    pos: usize,
) -> Result<(Vec<Option<CollectionDelta>>, usize)> {
    decode_delta_frame(schema, priors.len(), buf, pos, |i, collection, pos| {
        let prior = &priors[i];
        match (collection.shape, prior) {
            (Shape::KeyedSet, CollectionState::Keyed(prior)) => {
                let (delta, p) = decode_keyed_contribution(collection, prior, buf, pos)?;
                Ok((CollectionDelta::Keyed(delta), p))
            }
            (Shape::OrderedKeyedList, CollectionState::OrderedKeyed(_))
            | (Shape::OrderedOccurrenceList, CollectionState::OrderedOccurrence(_)) => {
                let (replacement, p) = decode_complete_collection(collection, buf, pos)?;
                if collection_stored_eq(collection, prior, &replacement)? {
                    return Err(WireError::Corrupt(
                        "ordered collection bit set with unchanged replacement",
                    ));
                }
                let delta = match replacement {
                    CollectionState::OrderedKeyed(rows) => CollectionDelta::OrderedKeyed(rows),
                    CollectionState::OrderedOccurrence(rows) => {
                        CollectionDelta::OrderedOccurrence(rows)
                    }
                    CollectionState::Keyed(_) => unreachable!("shape selected the decoder"),
                };
                Ok((delta, p))
            }
            _ => Err(WireError::Corrupt(
                "prior collection state shape does not match schema",
            )),
        }
    })
}

/// Keyed-only compatibility entry point. It uses the shaped path's shared
/// framing walk and sole keyed contribution codec without cloning prior state.
pub fn encode_delta(
    schema: &Schema,
    priors: &[KeyedState],
    deltas: &[Option<KeyedDelta>],
    out: &mut Vec<u8>,
) -> core::result::Result<(), MinimalityError> {
    encode_delta_frame(
        schema,
        priors.len(),
        deltas.len(),
        out,
        |i| deltas[i].is_some(),
        |i, collection, out| {
            if collection.shape != Shape::KeyedSet {
                return Err(MinimalityError::ShapeMismatch {
                    collection_id: collection.collection_id,
                });
            }
            encode_keyed_contribution(
                collection,
                &priors[i],
                deltas[i]
                    .as_ref()
                    .expect("the shared bitmap walk calls only set contributions"),
                out,
            )
        },
    )
}

pub fn decode_delta(
    schema: &Schema,
    priors: &[KeyedState],
    buf: &[u8],
    pos: usize,
) -> Result<(Vec<Option<KeyedDelta>>, usize)> {
    decode_delta_frame(schema, priors.len(), buf, pos, |i, collection, pos| {
        if collection.shape != Shape::KeyedSet {
            return Err(WireError::Corrupt(
                "ordered contribution is not representable as a keyed delta",
            ));
        }
        decode_keyed_contribution(collection, &priors[i], buf, pos)
    })
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

/// Replay one shaped contribution onto reconstructed state.
pub fn apply_shaped(state: &mut CollectionState, delta: &CollectionDelta) -> Result<()> {
    match (state, delta) {
        (CollectionState::Keyed(state), CollectionDelta::Keyed(delta)) => {
            apply(state, delta);
            Ok(())
        }
        (CollectionState::OrderedKeyed(state), CollectionDelta::OrderedKeyed(replacement)) => {
            *state = replacement.clone();
            Ok(())
        }
        (
            CollectionState::OrderedOccurrence(state),
            CollectionDelta::OrderedOccurrence(replacement),
        ) => {
            *state = replacement.clone();
            Ok(())
        }
        _ => Err(WireError::Corrupt(
            "collection delta shape does not match reconstructed state",
        )),
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
