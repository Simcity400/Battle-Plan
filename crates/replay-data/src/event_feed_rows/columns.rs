//! Exact per-field occurrence streams. The interval row supplies world and clock context.
use super::*;
use crate::rows::{Field, Kind};

// Source, country/cause leaves, script spans/flags/operands and operation leaves.
// Sequence has its own unsigned anchor/delta stream before these columns.
const FIELDS: usize = 37;
type Words = [Option<u64>; FIELDS];

fn words(event: &EventFeedEvent, read_hour: i32) -> Words {
    let mut words = [None; FIELDS];
    words[0] = Some((i64::from(read_hour) - i64::from(event.hour)) as u64);
    words[1] = Some(event.source as u64);
    for (at, country) in [(2, event.actor), (5, event.target)] {
        words[at..at + 3].copy_from_slice(&[
            Some(country.reference.into()),
            Some(country.id.into()),
            Some(country.instance.into()),
        ]);
    }
    if let Recorded::Value(cause) = event.cause {
        words[8] = Some(cause.route as u64);
        words[9] = Some(cause.within as u64);
        words[10] = cause.conference;
        words[11] = cause.state.map(u64::from);
        if let Some(ally) = cause.ally {
            words[12..15].copy_from_slice(&[
                Some(ally.reference.into()),
                Some(ally.id.into()),
                Some(ally.instance.into()),
            ]);
        }
        words[15] = cause.reason.map(u64::from);
        words[16] = Some(cause.flags.into());
    }
    if let Some(script) = event.script {
        words[17] = Some(script.flags().into());
        for (at, value) in [
            script.token_start,
            script.token_len,
            script.scope_start,
            script.scope_len,
        ]
        .into_iter()
        .enumerate()
        {
            words[18 + at] = Some(value.into());
        }
        if let Some(identity) = script.native_event {
            words[22] = Some(identity.kind.into());
            words[23] = Some(identity.id.into());
        }
        words[24] = script.instance.map(u64::from);
        words[25] = script.option.map(|n| signed_word(n).into());
        words[26] = script.country_target.map(u64::from);
        words[27] = script.numeric_target.map(|n| signed_word(n).into());
        words[28] = script.lifecycle.map(|n| signed_word(n).into());
        words[29] = script.days.map(|n| signed_word(n).into());
        if let Some(change) = script.change {
            use EventFeedScriptChange::*;
            let (code, argument) = match change {
                PendingBaseline => (0, None),
                PendingStored { due_hour } => (1, Some(signed_word(due_hour))),
                PendingReady => (2, None),
                PendingRemoved => (3, None),
                FiredBaseline => (4, None),
                FiredMember => (5, None),
                FiredReady => (6, None),
                DelayedBaseline => (7, None),
                DelayedStored { remaining } => (8, Some(signed_word(remaining))),
                DelayedReady => (9, None),
                DelayedAdvance { count, finished } => (10 + u64::from(finished), Some(count)),
                DelayedRemoved { iteration, visited } => {
                    (12 + u64::from(iteration) + u64::from(visited), None)
                }
                DelayedEmpty => (15, None),
                FiredRemoved => (16, None),
                Decision(decision) => {
                    words[32] = Some(decision.operation as u64);
                    words[33] = Some(decision.container.into());
                    words[34] = Some(decision.flags.into());
                    words[35] = Some(decision.ordinal.into());
                    words[36] = decision.political_power.map(|n| n as u64);
                    (17, None)
                }
            };
            words[30] = Some(code);
            words[31] = argument.map(u64::from);
        }
    }
    words
}

pub(crate) struct Batch<'a>(pub &'a EventFeedHistoryRow);
impl Batch<'_> {
    /// Every occurrence's words, computed once for all columns.
    fn rows(&self) -> io::Result<Vec<Words>> {
        let mut rows = allocation::vec(self.0.events.len())?;
        rows.extend(
            self.0
                .events
                .iter()
                .map(|event| words(event, self.0.read_hour)),
        );
        Ok(rows)
    }
}
impl crate::value::EncodeValue for Batch<'_> {
    fn encoded_bytes(&self) -> io::Result<usize> {
        u32::try_from(self.0.events.len())
            .map_err(|_| invalid("too many Event feed occurrences"))?;
        let mut bytes = 4;
        let mut sequence = 0;
        for event in &self.0.events {
            let delta = event
                .sequence
                .checked_sub(sequence)
                .ok_or_else(|| invalid("Event feed sequence decreased"))?;
            bytes = allocation::add(bytes, wide_len(delta))?;
            sequence = event.sequence;
        }
        let rows = self.rows()?;
        for column in 0..FIELDS {
            let mut previous = None;
            for row in &rows {
                let value = row[column];
                bytes = allocation::add(
                    bytes,
                    match value {
                        None => 1,
                        Some(value) => {
                            let delta = i128::from(value) - i128::from(previous.unwrap_or(0));
                            let zigzag = ((delta as u128) << 1) ^ ((delta >> 127) as u128);
                            1 + crate::rows::varint_bytes(zigzag)
                        }
                    },
                )?;
                previous = value;
            }
        }
        Ok(bytes)
    }
    fn recorded_bytes(&self) -> io::Result<usize> {
        self.encoded_bytes()
    }
    fn encode_value(&self, output: &mut Vec<u8>) -> io::Result<()> {
        allocation::reserve(output, self.encoded_bytes()?)?;
        (self.0.events.len() as u32).encode_reserved(output)?;
        let mut sequence = 0;
        for event in &self.0.events {
            crate::codec::put_varint(output, u128::from(event.sequence - sequence));
            sequence = event.sequence;
        }
        let rows = self.rows()?;
        for column in 0..FIELDS {
            let mut previous = Field::OptionalU64(None);
            for row in &rows {
                let value = Field::OptionalU64(row[column]);
                // The same exact numeric-difference coder used by ordinary table columns,
                // applied between occurrences rather than to one opaque batch per hour.
                value.change(previous)?.encode(output)?;
                previous = value;
            }
        }
        Ok(())
    }
}

pub(crate) fn decode(
    row: &EventFeedHistoryRow,
    input: &mut Decoder<'_>,
) -> io::Result<Vec<EventFeedEvent>> {
    let count = input.count(FIELDS + 1)?;
    let mut sequences = allocation::vec(count)?;
    let mut sequence = 0_u64;
    for index in 0..count {
        let delta = wide_read(input)?;
        if index != 0 && delta == 0 {
            return Err(invalid("Event feed sequence repeated"));
        }
        sequence = sequence
            .checked_add(delta)
            .ok_or_else(|| invalid("Event feed sequence overflow"))?;
        sequences.push(sequence);
    }
    let mut entries = allocation::vec(count)?;
    entries.resize(count, [None; FIELDS]);
    for column in 0..FIELDS {
        let mut previous = Field::OptionalU64(None);
        for entry in &mut entries {
            let change = Kind::OptionalU64.decode_change(input)?;
            let Field::OptionalU64(value) = change.apply(Kind::OptionalU64, previous)? else {
                unreachable!()
            };
            entry[column] = value;
            previous = Field::OptionalU64(value);
        }
    }
    let mut events = allocation::vec(count)?;
    for (sequence, entry) in sequences.into_iter().zip(entries) {
        events.push(event(row, sequence, entry)?);
    }
    Ok(events)
}

fn event(row: &EventFeedHistoryRow, sequence: u64, mut words: Words) -> io::Result<EventFeedEvent> {
    fn required(words: &mut Words, at: usize) -> io::Result<u64> {
        words[at]
            .take()
            .ok_or_else(|| invalid("Event feed column operand absent"))
    }
    fn word(words: &mut Words, at: usize) -> io::Result<u32> {
        u32::try_from(required(words, at)?).map_err(|_| invalid("Event feed column exceeds u32"))
    }
    fn optional(words: &mut Words, at: usize) -> io::Result<Option<u32>> {
        words[at]
            .take()
            .map(|n| u32::try_from(n).map_err(|_| invalid("Event feed column exceeds u32")))
            .transpose()
    }
    fn country(words: &mut Words, at: usize) -> io::Result<EventFeedCountry> {
        Ok(EventFeedCountry {
            reference: word(words, at)?,
            id: word(words, at + 1)?,
            instance: word(words, at + 2)?,
        })
    }
    fn byte(words: &mut Words, at: usize) -> io::Result<u8> {
        u8::try_from(required(words, at)?).map_err(|_| invalid("Event feed column exceeds u8"))
    }
    fn route(value: u8) -> io::Result<EventFeedRoute> {
        EventFeedRoute::decode(&mut Decoder::new(&[value]))
    }
    let age = word(&mut words, 0)?;
    let hour = i32::try_from(i64::from(row.read_hour) - i64::from(age))
        .map_err(|_| invalid("Event feed relative hour exceeds i32"))?;
    let source = EventFeedSource::decode(&mut Decoder::new(&[byte(&mut words, 1)?]))?;
    let actor = country(&mut words, 2)?;
    let target = country(&mut words, 5)?;
    let cause = if words[8].is_some() {
        Recorded::Value(EventFeedCause {
            route: route(byte(&mut words, 8)?)?,
            within: route(byte(&mut words, 9)?)?,
            conference: words[10].take(),
            state: optional(&mut words, 11)?,
            ally: if words[12].is_some() {
                Some(country(&mut words, 12)?)
            } else {
                None
            },
            reason: optional(&mut words, 15)?,
            flags: byte(&mut words, 16)?,
        })
    } else {
        Recorded::NotRecorded
    };
    let script = if words[17].is_some() {
        let flags = word(&mut words, 17)?;
        let mut script = EventFeedScript {
            token_start: word(&mut words, 18)?,
            token_len: word(&mut words, 19)?,
            scope_start: word(&mut words, 20)?,
            scope_len: word(&mut words, 21)?,
            native_event: if words[22].is_some() {
                Some(EventFeedEventIdentity {
                    kind: word(&mut words, 22)?,
                    id: word(&mut words, 23)?,
                })
            } else {
                None
            },
            instance: optional(&mut words, 24)?,
            option: optional(&mut words, 25)?.map(signed_value),
            country_target: optional(&mut words, 26)?,
            numeric_target: optional(&mut words, 27)?.map(signed_value),
            lifecycle: optional(&mut words, 28)?.map(signed_value),
            days: optional(&mut words, 29)?.map(signed_value),
            scope_unrecorded: flags & (1 << 5) != 0,
            token_unrecorded: flags & (1 << 6) != 0,
            change: None,
        };
        if words[30].is_some() {
            let code = byte(&mut words, 30)?;
            // Reuse the strict operation decoder; only this small fixed-size operand
            // record is assembled, never an occurrence batch or an arena.
            let mut encoded = allocation::vec(32)?;
            encoded.push(code);
            match code {
                1 | 8 | 10 | 11 => compact_write(word(&mut words, 31)?, &mut encoded),
                17 => {
                    encoded.push(byte(&mut words, 32)?);
                    encoded.push(byte(&mut words, 33)?);
                    encoded.push(byte(&mut words, 34)?);
                    compact_write(word(&mut words, 35)?, &mut encoded);
                    if encoded[1] == decisions::Operation::Rebuild as u8 {
                        words[36]
                            .take()
                            .map(|n| n as i64)
                            .encode_reserved(&mut encoded)?;
                    }
                }
                _ => {}
            }
            let mut input = Decoder::new(&encoded);
            script.change = Some(EventFeedScriptChange::decode(&mut input)?);
            input.finish()?;
        }
        if script.flags() != flags {
            return Err(invalid("Event feed script column flags differ"));
        }
        Some(script)
    } else {
        None
    };
    if words.iter().any(Option::is_some) {
        return Err(invalid("Event feed unused column operand"));
    }
    Ok(EventFeedEvent {
        sequence,
        generation: row.generation,
        hour,
        source,
        actor,
        target,
        cause,
        script,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::{RowCodec, TableRows};

    fn roundtrip(row: &EventFeedHistoryRow) {
        let mut encoded = Vec::new();
        Value::encode(row, &mut encoded).unwrap();
        assert_eq!(Value::encoded_bytes(row).unwrap(), encoded.len());
        let mut input = Decoder::new(&encoded);
        assert_eq!(EventFeedHistoryRow::decode(&mut input).unwrap(), *row);
        input.finish().unwrap();
        let mut snapshot = Vec::new();
        row.encode_fields(&mut snapshot).unwrap();
        assert_eq!(row.snapshot_bytes().unwrap(), 1 + snapshot.len());
        let mut input = Decoder::new(&snapshot);
        assert_eq!(
            EventFeedHistoryRow::decode_fields(0, &mut input).unwrap(),
            *row
        );
        input.finish().unwrap();
        assert_eq!(
            row.delta(None)
                .unwrap()
                .unwrap()
                .apply::<EventFeedHistoryRow>(None)
                .unwrap(),
            *row
        );
        let rows = TableRows::EventFeedHistory(vec![row.clone()]);
        let mut prepared = Vec::new();
        rows.encode_checkpoint_rows(&mut prepared).unwrap();
        assert_eq!(
            crate::rows::TableId::EventFeedHistory
                .decode_checkpoint_rows(&mut Decoder::new(&prepared))
                .unwrap(),
            rows
        );
    }

    #[test]
    fn event_feed_columnar_gaps_extreme_relative_hours_refusals_and_empty_batches() {
        let mut row = super::super::tests::row();
        row.read_hour = i32::MAX;
        row.generation = u64::MAX;
        row.events[0].generation = row.generation;
        row.events[0].sequence = 0;
        row.events[0].hour = i32::MIN;
        let mut second = row.events[0];
        second.sequence = u64::MAX;
        second.hour = i32::MAX;
        second.cause = Recorded::NotRecorded;
        row.events.push(second);
        roundtrip(&row);
        row.coverage = EventFeedCoverage::Partial;
        row.failure = Some(b"synthetic interval refusal".to_vec());
        row.counts = None;
        row.outstanding = u64::MAX;
        row.script_refusals = DECISION_FAMILY_REFUSALS;
        roundtrip(&row);
        row.events.clear();
        roundtrip(&row);
        row.coverage = EventFeedCoverage::Unavailable;
        roundtrip(&row);
    }

    #[test]
    fn event_feed_columnar_scripts_operations_and_out_of_order_event_hours() {
        use EventFeedScriptChange::*;
        use decisions::{DecisionChange, Operation};
        let changes = [
            PendingBaseline,
            PendingStored { due_hour: i32::MIN },
            PendingReady,
            PendingRemoved,
            FiredBaseline,
            FiredMember,
            FiredRemoved,
            FiredReady,
            DelayedBaseline,
            DelayedStored {
                remaining: i32::MAX,
            },
            DelayedReady,
            DelayedEmpty,
            DelayedAdvance {
                count: u32::MAX,
                finished: false,
            },
            DelayedAdvance {
                count: 0,
                finished: true,
            },
            DelayedRemoved {
                iteration: false,
                visited: false,
            },
            DelayedRemoved {
                iteration: true,
                visited: false,
            },
            DelayedRemoved {
                iteration: true,
                visited: true,
            },
        ];
        let mut row = super::super::tests::row();
        row.events.clear();
        row.counts = None;
        row.coverage = EventFeedCoverage::Partial;
        row.script_tokens = b"same-token-same-token".to_vec();
        row.script_scopes = vec![EventFeedScope {
            bindings: [0, 0, 0],
            ..Default::default()
        }];
        for change in changes {
            let identity = matches!(
                change,
                PendingStored { .. } | DelayedStored { .. } | FiredMember | FiredRemoved
            );
            let instance = matches!(
                change,
                PendingStored { .. }
                    | DelayedStored { .. }
                    | PendingRemoved
                    | DelayedRemoved { .. }
            );
            let scope = matches!(change, PendingStored { .. } | DelayedStored { .. });
            row.events.push(EventFeedEvent {
                sequence: 99 + 3 * row.events.len() as u64,
                generation: row.generation,
                hour: if row.events.len().is_multiple_of(2) {
                    12
                } else {
                    -100
                },
                source: EventFeedSource::ScriptState,
                actor: Default::default(),
                target: Default::default(),
                cause: Recorded::Value(Default::default()),
                script: Some(EventFeedScript {
                    token_start: 2,
                    token_len: if identity { 5 } else { 0 },
                    scope_start: 0,
                    scope_len: u32::from(scope),
                    native_event: identity.then_some(EventFeedEventIdentity {
                        kind: u32::MAX,
                        id: 0,
                    }),
                    instance: instance.then_some(u32::MAX),
                    change: Some(change),
                    ..Default::default()
                }),
            });
        }
        for operation in [
            Operation::Baseline,
            Operation::Ready,
            Operation::Invalidate,
            Operation::Store,
            Operation::Erase,
            Operation::Update,
            Operation::PassBegin,
            Operation::PassCut,
            Operation::PassEnd,
            Operation::RandomSet,
            Operation::RandomIncrement,
            Operation::Rebuild,
            Operation::Candidate,
            Operation::Ignore,
        ] {
            let member = matches!(
                operation,
                Operation::Store | Operation::Erase | Operation::Update | Operation::PassCut
            );
            let mut event = row.events[0];
            event.sequence = row.events.last().unwrap().sequence + 13;
            event.script = Some(EventFeedScript {
                token_start: 7,
                token_len: 8,
                instance: member.then_some(123),
                country_target: Some(u32::MAX),
                numeric_target: Some(i32::MIN),
                lifecycle: Some(i32::MAX),
                days: Some(i32::MIN),
                change: Some(Decision(DecisionChange {
                    operation,
                    container: 3,
                    flags: u8::MAX,
                    ordinal: u32::MAX,
                    political_power: (operation == Operation::Rebuild).then_some(i64::MIN),
                })),
                ..Default::default()
            });
            row.events.push(event);
        }
        let mut plain = super::super::tests::row().events[0];
        plain.sequence = row.events.last().unwrap().sequence + 100;
        row.events.push(plain);
        roundtrip(&row);
        let mut later = row.clone();
        later.read_hour += 1;
        // A metadata-only delta must keep the absolute event dates, not rebase held occurrences.
        assert_eq!(
            later
                .delta(Some(&row))
                .unwrap()
                .unwrap()
                .apply(Some(row.clone()))
                .unwrap(),
            later
        );
        later.events[0].hour -= 1;
        assert_eq!(
            later
                .delta(Some(&row))
                .unwrap()
                .unwrap()
                .apply(Some(row))
                .unwrap(),
            later
        );
    }

    #[test]
    fn event_feed_columnar_truncation_overflow_unused_operands_and_duplicate_sequences_refuse() {
        let row = super::super::tests::row();
        let batch = Batch(&row);
        let mut encoded = Vec::new();
        crate::value::EncodeValue::encode_value(&batch, &mut encoded).unwrap();
        for end in 0..encoded.len() {
            assert!(decode(&row, &mut Decoder::new(&encoded[..end])).is_err());
        }
        let mut malformed = encoded.clone();
        malformed[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&row, &mut Decoder::new(&malformed)).is_err());
        // A script operand without a script flag is not silently discarded.
        let mut entry = words(&row.events[0], row.read_hour);
        entry[36] = Some(1);
        assert!(event(&row, 1, entry).is_err());
        let mut entry = words(&row.events[0], row.read_hour);
        entry[0] = Some(u64::MAX);
        assert!(event(&row, 1, entry).is_err());
        let mut repeated = row.clone();
        repeated.events.push(repeated.events[0]);
        assert!(repeated.validate().is_err());
    }
}
