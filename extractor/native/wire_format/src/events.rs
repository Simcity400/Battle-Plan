//! §9 event-body codec and frontier continuity checks.
//!
//! Event records are full §3 records in occurrence order. They deliberately do
//! not use any of §7's state-diff machinery: an event has no key, tombstone,
//! insert/update distinction, change mask, or prior-state minimality check.

use crate::record::Record;
use crate::schema::{Collection, EventPart, Kind, Quant, Shape};
use crate::varint::{read_varint, write_varint, Result, WireError};

/// One complete, non-empty §9 event batch.
#[derive(Clone, Debug, PartialEq)]
pub struct EventBatch {
    /// Full §3 records, in producer occurrence order. Equal records remain
    /// distinct entries.
    pub records: Vec<Record>,
    /// The schema-defined frontier tuple after `records`.
    pub frontier: Vec<u64>,
}

impl EventBatch {
    pub fn new(records: Vec<Record>, frontier: Vec<u64>) -> Self {
        Self { records, frontier }
    }

    /// Derive the sequence identity of record `index` from its position and the
    /// after-batch frontier. No per-event sequence is carried on the wire.
    pub fn sequence_at(&self, part: &EventPart, index: usize) -> Result<u64> {
        if index >= self.records.len() {
            return Err(WireError::Corrupt("event index is outside the batch"));
        }
        validate_frontier_arity(part, &self.frontier)?;
        let count = u64::try_from(self.records.len())
            .map_err(|_| WireError::Corrupt("event count exceeds u64"))?;
        let index =
            u64::try_from(index).map_err(|_| WireError::Corrupt("event index exceeds u64"))?;
        let after = self.frontier[part.sequence_index as usize];
        after
            .checked_sub(count)
            .and_then(|first_minus_one| first_minus_one.checked_add(index))
            .and_then(|value| value.checked_add(1))
            .ok_or(WireError::Corrupt(
                "event frontier cannot derive this batch's sequence range",
            ))
    }
}

/// Encode one §9 body. The returned bytes are exactly the opaque body placed in
/// an `EVENTS` frame by the block layer.
pub fn encode_event_body(part: &EventPart, batch: &EventBatch) -> Result<Vec<u8>> {
    validate_event_fields(part)?;
    if batch.records.is_empty() {
        return Err(WireError::Corrupt("empty event batch"));
    }
    validate_frontier_arity(part, &batch.frontier)?;
    // This also proves the after-frontier can identify every event in the batch.
    batch.sequence_at(part, 0)?;

    let count = u64::try_from(batch.records.len())
        .map_err(|_| WireError::Corrupt("event count exceeds u64"))?;
    let collection = event_collection(part);
    let mut out = Vec::new();
    write_varint(&mut out, count);
    for record in &batch.records {
        record.encode(&collection, &mut out)?;
    }
    for component in &batch.frontier {
        write_varint(&mut out, *component);
    }
    Ok(out)
}

/// Decode one complete §9 body. Trailing bytes are corrupt: the enclosing frame
/// length already supplies the only legal boundary.
pub fn decode_event_body(part: &EventPart, body: &[u8]) -> Result<EventBatch> {
    validate_event_fields(part)?;
    let (count_u64, mut pos) = read_varint(body, 0)?;
    if count_u64 == 0 {
        return Err(WireError::Corrupt("empty event batch"));
    }
    let count = usize::try_from(count_u64)
        .map_err(|_| WireError::Corrupt("event count does not fit usize"))?;
    // A validated event part has at least one field, and every full §3 record
    // therefore consumes at least one byte. Bound work and allocation by the
    // enclosing body before reserving attacker-selected capacity.
    if count > body.len().saturating_sub(pos) {
        return Err(WireError::Truncated);
    }

    let collection = event_collection(part);
    let mut records = Vec::new();
    records
        .try_reserve_exact(count)
        .map_err(|_| WireError::Corrupt("event count exceeds allocation bounds"))?;
    for _ in 0..count {
        let (record, next) = Record::decode(&collection, body, pos)?;
        if next <= pos {
            return Err(WireError::Corrupt("event record consumed no bytes"));
        }
        pos = next;
        records.push(record);
    }

    let mut frontier = Vec::new();
    frontier
        .try_reserve_exact(part.frontier_component_ids.len())
        .map_err(|_| WireError::Corrupt("frontier arity exceeds allocation bounds"))?;
    for _ in &part.frontier_component_ids {
        let (component, next) = read_varint(body, pos)?;
        frontier.push(component);
        pos = next;
    }
    if pos != body.len() {
        return Err(WireError::Corrupt("trailing bytes in event body"));
    }

    let batch = EventBatch { records, frontier };
    // A first/after-gap batch establishes continuity, but its own dense identity
    // range must still be derivable.
    batch.sequence_at(part, 0)?;
    Ok(batch)
}

/// A continuity failure is data, not a codec error. The caller records the
/// corresponding §8 event-part gap and then checks the next batch with no prior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrontierDiscontinuity {
    SchemaChanged,
    EpochChanged {
        component_index: usize,
        previous: u64,
        next: u64,
    },
    SequenceOverflow {
        previous: u64,
        count: u64,
    },
    SequenceMismatch {
        previous: u64,
        count: u64,
        expected: u64,
        next: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrontierOutcome {
    /// First batch of a `(domain, branch)`, or first batch after a recorded
    /// event-part gap.
    Established,
    Continuous,
    Discontinuous(FrontierDiscontinuity),
}

/// Apply §9's continuity rules when the frontier declaration is unchanged.
///
/// Pass `None` for the first batch and for the first batch after the caller has
/// recorded an event-part gap. Across a schema change, use
/// [`check_frontier_transition`] so component identities, rather than tuple
/// positions, govern the join.
pub fn check_frontier(
    part: &EventPart,
    previous: Option<&[u64]>,
    next: &[u64],
    event_count: usize,
) -> Result<FrontierOutcome> {
    check_frontier_transition(
        previous.map(|frontier| (part, frontier)),
        part,
        next,
        event_count,
    )
}

/// Apply §9 continuity across an optional schema change.
///
/// Frontier components are joined by `component_id`; tuple position is only
/// the encoding order within one schema. A pure reordering is safe. Adding,
/// removing, duplicating, or changing the designated sequence component is a
/// typed discontinuity.
pub fn check_frontier_transition(
    previous: Option<(&EventPart, &[u64])>,
    next_part: &EventPart,
    next: &[u64],
    event_count: usize,
) -> Result<FrontierOutcome> {
    if event_count == 0 {
        return Err(WireError::Corrupt("empty event batch"));
    }
    validate_frontier_arity(next_part, next)?;
    validate_frontier_component_ids(next_part)?;
    let count =
        u64::try_from(event_count).map_err(|_| WireError::Corrupt("event count exceeds u64"))?;
    let sequence_index = next_part.sequence_index as usize;
    if next[sequence_index] < count {
        return Err(WireError::Corrupt(
            "event frontier cannot derive this batch's sequence range",
        ));
    }

    let Some((previous_part, previous)) = previous else {
        return Ok(FrontierOutcome::Established);
    };
    validate_frontier_arity(previous_part, previous)?;
    validate_frontier_component_ids(previous_part)?;

    let previous_sequence_index = previous_part.sequence_index as usize;
    let previous_sequence_id =
        previous_part.frontier_component_ids[previous_sequence_index];
    let next_sequence_id = next_part.frontier_component_ids[sequence_index];
    if previous_sequence_id != next_sequence_id
        || previous_part.frontier_component_ids.len()
            != next_part.frontier_component_ids.len()
    {
        return Ok(FrontierOutcome::Discontinuous(
            FrontierDiscontinuity::SchemaChanged,
        ));
    }

    for (index, component_id) in next_part.frontier_component_ids.iter().enumerate() {
        if index == sequence_index {
            continue;
        }
        let Some(previous_index) = previous_part
            .frontier_component_ids
            .iter()
            .position(|candidate| candidate == component_id)
        else {
            return Ok(FrontierOutcome::Discontinuous(
                FrontierDiscontinuity::SchemaChanged,
            ));
        };
        if previous_index == previous_sequence_index {
            return Ok(FrontierOutcome::Discontinuous(
                FrontierDiscontinuity::SchemaChanged,
            ));
        }
        if previous[previous_index] != next[index] {
            return Ok(FrontierOutcome::Discontinuous(
                FrontierDiscontinuity::EpochChanged {
                    component_index: index,
                    previous: previous[previous_index],
                    next: next[index],
                },
            ));
        }
    }

    let previous_sequence = previous[previous_sequence_index];
    let expected = match previous_sequence.checked_add(count) {
        Some(value) => value,
        None => {
            return Ok(FrontierOutcome::Discontinuous(
                FrontierDiscontinuity::SequenceOverflow {
                    previous: previous_sequence,
                    count,
                },
            ))
        }
    };
    if next[sequence_index] != expected {
        return Ok(FrontierOutcome::Discontinuous(
            FrontierDiscontinuity::SequenceMismatch {
                previous: previous_sequence,
                count,
                expected,
                next: next[sequence_index],
            },
        ));
    }
    Ok(FrontierOutcome::Continuous)
}

fn validate_frontier_arity(part: &EventPart, frontier: &[u64]) -> Result<()> {
    if (part.sequence_index as usize) >= part.frontier_component_ids.len() {
        return Err(WireError::Corrupt(
            "event sequence index is outside the frontier",
        ));
    }
    if frontier.len() != part.frontier_component_ids.len() {
        return Err(WireError::Corrupt(
            "event frontier arity does not match schema",
        ));
    }
    Ok(())
}

fn validate_frontier_component_ids(part: &EventPart) -> Result<()> {
    for (index, component_id) in part.frontier_component_ids.iter().enumerate() {
        if part.frontier_component_ids[..index].contains(component_id) {
            return Err(WireError::Corrupt(
                "event frontier component id is duplicated",
            ));
        }
    }
    Ok(())
}

fn validate_event_fields(part: &EventPart) -> Result<()> {
    if part.fields.is_empty() {
        return Err(WireError::Corrupt("event part has no fields"));
    }
    for (index, field) in part.fields.iter().enumerate() {
        if part.fields[..index]
            .iter()
            .any(|candidate| candidate.field_id == field.field_id)
        {
            return Err(WireError::Corrupt("event field id is duplicated"));
        }
    }
    if part.fields.iter().any(|field| field.is_key()) {
        return Err(WireError::Corrupt("event field is marked as a key"));
    }
    if part.fields.iter().any(|field| field.derived.is_some()) {
        return Err(WireError::Corrupt("event field is marked as derived"));
    }
    for field in &part.fields {
        if matches!(field.kind, Kind::Enum) && field.enum_card.map_or(true, |card| card < 2) {
            return Err(WireError::Corrupt(
                "event enum field has no valid cardinality",
            ));
        }
        if matches!(field.kind, Kind::Continuous) && matches!(field.quant, Quant::None) {
            return Err(WireError::Corrupt(
                "event continuous field has no quantization decision",
            ));
        }
        if field.quant.is_quantized() && !matches!(field.kind, Kind::Continuous) {
            return Err(WireError::Corrupt(
                "event non-continuous field has a quantizer",
            ));
        }
        match field.quant {
            Quant::Fixed(k) if k > 22 => {
                return Err(WireError::Corrupt(
                    "event fixed quantizer is outside 0..=22",
                ))
            }
            Quant::Ratio(d) if d == 0 || d > 32 => {
                return Err(WireError::Corrupt(
                    "event ratio quantizer is outside 1..=32",
                ))
            }
            _ => {}
        }
    }
    validate_frontier_component_ids(part)?;
    Ok(())
}

/// `Record` is shape-agnostic. This adapter gives it the event part's fields;
/// the occurrence-list shape has no key, so every event field is encoded.
fn event_collection(part: &EventPart) -> Collection {
    Collection {
        collection_id: part.collection_id,
        shape: Shape::OrderedOccurrenceList,
        fields: part.fields.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Derived, Field, Kind, Quant, Schema, SchemaError, TickGrid, Value};

    fn part() -> EventPart {
        EventPart {
            collection_id: 9,
            fields: vec![
                Field::new(1, Kind::Unsigned).optional(),
                Field::new(2, Kind::Signed).nullable().unknown_capable(),
                Field::new(3, Kind::Enum).enum_card(4),
                Field::new(4, Kind::Boolean),
            ],
            frontier_component_ids: vec![11, 12],
            sequence_index: 1,
        }
    }

    fn every_state_batch() -> EventBatch {
        EventBatch::new(
            vec![
                Record::new(vec![
                    Value::Absent,
                    Value::Null,
                    Value::Enum(0),
                    Value::Bool(false),
                ]),
                Record::new(vec![
                    Value::Unsigned(0),
                    Value::Unknown,
                    Value::Enum(3),
                    Value::Bool(true),
                ]),
                Record::new(vec![
                    Value::Unsigned(u64::MAX),
                    Value::Signed(i64::MIN),
                    Value::Enum(2),
                    Value::Bool(false),
                ]),
            ],
            vec![7, 103],
        )
    }

    #[test]
    fn every_record_state_roundtrips_byte_identically() {
        let part = part();
        let batch = every_state_batch();
        let bytes = encode_event_body(&part, &batch).expect("encodes");
        let decoded = decode_event_body(&part, &bytes).expect("decodes");
        assert_eq!(decoded, batch);
        assert_eq!(
            encode_event_body(&part, &decoded).expect("re-encodes"),
            bytes
        );
    }

    #[test]
    fn equal_records_are_two_events_not_one_state_row() {
        let part = part();
        let record = every_state_batch().records[0].clone();
        let batch = EventBatch::new(vec![record.clone(), record.clone()], vec![1, 2]);
        let bytes = encode_event_body(&part, &batch).unwrap();
        let decoded = decode_event_body(&part, &bytes).unwrap();
        assert_eq!(decoded.records, vec![record.clone(), record]);
        assert_eq!(decoded.sequence_at(&part, 0).unwrap(), 1);
        assert_eq!(decoded.sequence_at(&part, 1).unwrap(), 2);
        assert_eq!(
            encode_event_body(&part, &decoded).unwrap(),
            bytes,
            "duplicate occurrences must re-encode byte-identically"
        );
    }

    #[test]
    fn every_kind_and_schema_capability_has_the_event_record_semantics() {
        let fields = vec![
            Field::new(1, Kind::Unsigned).optional(),
            Field::new(2, Kind::Signed).nullable(),
            Field::new(3, Kind::Monotone).unknown_capable(),
            Field::new(4, Kind::Enum).enum_card(4),
            Field::new(5, Kind::Boolean),
            Field::new(6, Kind::ContentId),
            Field::new(7, Kind::Continuous).quant(Quant::Fixed(2)),
        ];
        let schema = Schema {
            domain: 99,
            cadence_log2: 0,
            collections: Vec::new(),
            event_part: Some(EventPart {
                collection_id: 1,
                fields,
                frontier_component_ids: vec![1],
                sequence_index: 0,
            }),
        };
        schema
            .validate(TickGrid::HOURLY.log2())
            .expect("all seven kinds and §3 field states are event-legal");
        let part = schema.event_part.as_ref().unwrap();
        let batch = EventBatch::new(
            vec![
                Record::new(vec![
                    Value::Absent,
                    Value::Null,
                    Value::Unknown,
                    Value::Enum(0),
                    Value::Bool(false),
                    Value::ContentId(0),
                    Value::Continuous(1.25),
                ]),
                Record::new(vec![
                    Value::Unsigned(u64::MAX),
                    Value::Signed(i64::MIN),
                    Value::Unsigned(u64::MAX),
                    Value::Enum(3),
                    Value::Bool(true),
                    Value::ContentId(u64::MAX),
                    Value::Continuous(-0.0),
                ]),
            ],
            vec![2],
        );
        let bytes = encode_event_body(part, &batch).unwrap();
        let decoded = decode_event_body(part, &bytes).unwrap();
        assert_eq!(decoded, batch);
        assert_eq!(encode_event_body(part, &decoded).unwrap(), bytes);

        for (quant, sample) in [(Quant::Exact, 1.25), (Quant::Ratio(8), 0.5)] {
            let mut quant_schema = schema.clone();
            quant_schema.event_part.as_mut().unwrap().fields[6].quant = quant;
            quant_schema
                .validate(TickGrid::HOURLY.log2())
                .expect("each §6 quantizer keeps its ordinary event-record semantics");
            let quant_part = quant_schema.event_part.as_ref().unwrap();
            let mut quant_batch = batch.clone();
            quant_batch.records[0].values[6] = Value::Continuous(sample);
            quant_batch.records[1].values[6] = Value::Continuous(-sample);
            let quant_bytes = encode_event_body(quant_part, &quant_batch).unwrap();
            let quant_decoded = decode_event_body(quant_part, &quant_bytes).unwrap();
            assert_eq!(quant_decoded.frontier, quant_batch.frontier);
            assert_eq!(quant_decoded.records.len(), quant_batch.records.len());
            let quant_collection = event_collection(quant_part);
            for (decoded_record, source_record) in
                quant_decoded.records.iter().zip(&quant_batch.records)
            {
                assert!(
                    decoded_record.stored_eq(&quant_collection, source_record),
                    "lossy quantizers compare by their canonical stored values"
                );
            }
            assert_eq!(
                encode_event_body(quant_part, &quant_decoded).unwrap(),
                quant_bytes
            );
        }
    }

    #[test]
    fn monotone_is_absolute_and_derived_event_fields_are_rejected() {
        let monotone = EventPart {
            collection_id: 1,
            fields: vec![Field::new(1, Kind::Monotone)],
            frontier_component_ids: vec![1],
            sequence_index: 0,
        };
        let batch = EventBatch::new(vec![Record::new(vec![Value::Unsigned(300)])], vec![1]);
        assert_eq!(
            encode_event_body(&monotone, &batch).unwrap(),
            vec![1, 0xac, 0x02, 1],
            "Monotone is a full-record absolute varint, never a delta"
        );

        let mut derived_field = Field::new(1, Kind::Monotone);
        derived_field.derived = Some(Derived { c: -1, u: 0 });
        let derived = EventPart {
            fields: vec![derived_field],
            ..monotone
        };
        let schema = Schema {
            domain: 99,
            cadence_log2: 0,
            collections: Vec::new(),
            event_part: Some(derived.clone()),
        };
        assert_eq!(
            schema.validate(TickGrid::HOURLY.log2()),
            Err(SchemaError::EventFieldMarkedDerived { field_id: 1 })
        );
        assert_eq!(
            encode_event_body(
                &derived,
                &EventBatch::new(vec![Record::new(vec![Value::Signed(10)])], vec![1])
            ),
            Err(WireError::Corrupt("event field is marked as derived"))
        );
        assert_eq!(
            decode_event_body(&derived, &[1, 20, 1]),
            Err(WireError::Corrupt("event field is marked as derived"))
        );
    }

    #[test]
    fn encoder_and_decoder_both_reject_empty_batches() {
        let part = part();
        assert_eq!(
            encode_event_body(&part, &EventBatch::new(Vec::new(), vec![1, 0])),
            Err(WireError::Corrupt("empty event batch"))
        );
        assert_eq!(
            decode_event_body(&part, &[0, 1, 0]),
            Err(WireError::Corrupt("empty event batch"))
        );
    }

    #[test]
    fn encoder_and_decoder_reject_wrong_frontier_arity_and_key_fields() {
        let part = part();
        let mut batch = every_state_batch();
        batch.frontier.pop();
        assert_eq!(
            encode_event_body(&part, &batch),
            Err(WireError::Corrupt(
                "event frontier arity does not match schema"
            ))
        );

        let good = encode_event_body(&part, &every_state_batch()).unwrap();
        assert_eq!(
            decode_event_body(
                &EventPart {
                    frontier_component_ids: vec![11, 12, 13],
                    ..part.clone()
                },
                &good
            ),
            Err(WireError::Truncated)
        );

        let mut key_marked = part.clone();
        key_marked.fields[0].key_position = Some(0);
        assert_eq!(
            encode_event_body(&key_marked, &every_state_batch()),
            Err(WireError::Corrupt("event field is marked as a key"))
        );
        assert_eq!(
            decode_event_body(&key_marked, &good),
            Err(WireError::Corrupt("event field is marked as a key"))
        );
    }

    #[test]
    fn decoder_rejects_trailing_bytes() {
        let part = part();
        let mut bytes = encode_event_body(&part, &every_state_batch()).unwrap();
        bytes.push(0);
        assert_eq!(
            decode_event_body(&part, &bytes),
            Err(WireError::Corrupt("trailing bytes in event body"))
        );
    }

    #[test]
    fn every_truncation_and_single_byte_corruption_is_total_and_canonical_if_accepted() {
        let part = part();
        let good = encode_event_body(&part, &every_state_batch()).unwrap();

        for len in 0..good.len() {
            assert_eq!(
                decode_event_body(&part, &good[..len]),
                Err(WireError::Truncated),
                "canonical-body truncation at byte {len}"
            );
        }
        for index in 0..good.len() {
            for replacement in [0u8, 1, 0x7f, 0x80, 0xff] {
                let mut changed = good.clone();
                changed[index] = replacement;
                match decode_event_body(&part, &changed) {
                    Ok(batch) => assert_eq!(
                        encode_event_body(&part, &batch).expect("accepted input re-encodes"),
                        changed,
                        "an accepted mutation must already be its canonical byte form"
                    ),
                    Err(_typed_error) => {}
                }
            }
        }
    }

    #[test]
    fn first_batch_and_first_after_gap_establish() {
        let part = part();
        assert_eq!(
            check_frontier(&part, None, &[9, 10], 2).unwrap(),
            FrontierOutcome::Established
        );
        // The caller records a gap by dropping its prior and passing None.
        assert_eq!(
            check_frontier(&part, None, &[77, 500], 3).unwrap(),
            FrontierOutcome::Established
        );
    }

    #[test]
    fn same_epoch_requires_previous_plus_count() {
        let part = part();
        assert_eq!(
            check_frontier(&part, Some(&[9, 10]), &[9, 13], 3).unwrap(),
            FrontierOutcome::Continuous
        );
        assert_eq!(
            check_frontier(&part, Some(&[9, 10]), &[9, 14], 3).unwrap(),
            FrontierOutcome::Discontinuous(FrontierDiscontinuity::SequenceMismatch {
                previous: 10,
                count: 3,
                expected: 13,
                next: 14,
            })
        );
    }

    #[test]
    fn epoch_change_is_a_typed_discontinuity_even_if_sequence_looks_contiguous() {
        let part = part();
        assert_eq!(
            check_frontier(&part, Some(&[9, 10]), &[10, 13], 3).unwrap(),
            FrontierOutcome::Discontinuous(FrontierDiscontinuity::EpochChanged {
                component_index: 0,
                previous: 9,
                next: 10,
            })
        );
    }

    #[test]
    fn schema_change_joins_frontiers_by_component_id_not_position() {
        let previous_part = EventPart {
            collection_id: 9,
            fields: part().fields,
            frontier_component_ids: vec![11, 12],
            sequence_index: 1,
        };
        let reordered = EventPart {
            collection_id: 9,
            fields: previous_part.fields.clone(),
            frontier_component_ids: vec![12, 11],
            sequence_index: 0,
        };
        assert_eq!(
            check_frontier_transition(
                Some((&previous_part, &[5, 10])),
                &reordered,
                &[12, 5],
                2,
            )
            .unwrap(),
            FrontierOutcome::Continuous
        );
        assert_eq!(
            check_frontier_transition(
                Some((&previous_part, &[5, 10])),
                &reordered,
                &[12, 6],
                2,
            )
            .unwrap(),
            FrontierOutcome::Discontinuous(FrontierDiscontinuity::EpochChanged {
                component_index: 1,
                previous: 5,
                next: 6,
            })
        );
    }

    #[test]
    fn schema_change_and_sequence_overflow_have_truthful_typed_outcomes() {
        let part = part();
        let changed_sequence = EventPart {
            frontier_component_ids: vec![11, 99],
            ..part.clone()
        };
        assert_eq!(
            check_frontier_transition(
                Some((&part, &[5, 10])),
                &changed_sequence,
                &[5, 12],
                2,
            )
            .unwrap(),
            FrontierOutcome::Discontinuous(FrontierDiscontinuity::SchemaChanged)
        );
        assert_eq!(
            check_frontier(&part, Some(&[5, u64::MAX]), &[5, u64::MAX], 1).unwrap(),
            FrontierOutcome::Discontinuous(FrontierDiscontinuity::SequenceOverflow {
                previous: u64::MAX,
                count: 1,
            })
        );
    }

    #[test]
    fn invalid_or_fieldless_event_parts_fail_before_record_work() {
        let fieldless = EventPart {
            collection_id: 1,
            fields: Vec::new(),
            frontier_component_ids: vec![1],
            sequence_index: 0,
        };
        assert_eq!(
            decode_event_body(&fieldless, &[1, 1]),
            Err(WireError::Corrupt("event part has no fields"))
        );

        let invalid_ratio = EventPart {
            fields: vec![Field::new(1, Kind::Continuous).quant(Quant::Ratio(64))],
            ..fieldless
        };
        assert_eq!(
            encode_event_body(
                &invalid_ratio,
                &EventBatch::new(
                    vec![Record::new(vec![Value::Continuous(0.5)])],
                    vec![1],
                ),
            ),
            Err(WireError::Corrupt(
                "event ratio quantizer is outside 1..=32",
            ))
        );

        let duplicate_frontier = EventPart {
            collection_id: 1,
            fields: vec![Field::new(1, Kind::Unsigned)],
            frontier_component_ids: vec![1, 1],
            sequence_index: 0,
        };
        assert_eq!(
            decode_event_body(&duplicate_frontier, &[1, 0, 1, 1]),
            Err(WireError::Corrupt(
                "event frontier component id is duplicated",
            ))
        );

        let one_field = EventPart {
            frontier_component_ids: vec![1],
            ..duplicate_frontier
        };
        // count=1,000,000 with no record bytes: the body is tiny and must fail
        // before reserving or iterating that many records.
        assert_eq!(
            decode_event_body(&one_field, &[0xc0, 0x84, 0x3d, 1]),
            Err(WireError::Truncated)
        );
    }
}
