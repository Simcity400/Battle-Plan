use super::*;
use replay_rows::rows::Field;

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
                DelayedRefused => (18, None),
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

fn changed(words: &Words, previous: &Words) -> u64 {
    (0..FIELDS).fold(0, |mask, column| {
        mask | (u64::from(words[column] != previous[column]) << column)
    })
}

trait Sink {
    fn put(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn varint(&mut self, value: u128) -> io::Result<()>;
    fn change(&mut self, value: Option<u64>, previous: Option<u64>) -> io::Result<()>;
}
struct Count(usize);
impl Sink for Count {
    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0 = allocation::add(self.0, bytes.len())?;
        Ok(())
    }
    fn varint(&mut self, value: u128) -> io::Result<()> {
        self.0 = allocation::add(self.0, replay_rows::rows::varint_bytes(value))?;
        Ok(())
    }
    fn change(&mut self, value: Option<u64>, previous: Option<u64>) -> io::Result<()> {
        let bytes = match value {
            None => 1,
            Some(value) => {
                let delta = i128::from(value) - i128::from(previous.unwrap_or(0));
                1 + replay_rows::rows::varint_bytes(((delta as u128) << 1) ^ ((delta >> 127) as u128))
            }
        };
        self.0 = allocation::add(self.0, bytes)?;
        Ok(())
    }
}
impl Sink for Vec<u8> {
    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.extend_from_slice(bytes);
        Ok(())
    }
    fn varint(&mut self, value: u128) -> io::Result<()> {
        replay_rows::codec::put_varint(self, value);
        Ok(())
    }
    fn change(&mut self, value: Option<u64>, previous: Option<u64>) -> io::Result<()> {
        Field::OptionalU64(value)
            .change(Field::OptionalU64(previous))?
            .encode(self)
    }
}

pub struct Batch<'a>(pub &'a EventFeedHistoryRow);
impl Batch<'_> {
    fn write(&self, output: &mut impl Sink) -> io::Result<()> {
        let count = u32::try_from(self.0.events.len())
            .map_err(|_| invalid("too many Event feed occurrences"))?;
        output.put(&count.to_le_bytes())?;
        let (mut sequence, mut previous, mut previous_mask) = (0, [None; FIELDS], 0);
        for event in &self.0.events {
            let gap = event
                .sequence
                .checked_sub(sequence)
                .ok_or_else(|| invalid("Event feed sequence decreased"))?;
            output.varint(gap.into())?;
            sequence = event.sequence;
            let words = words(event, self.0.read_hour);
            let mask = changed(&words, &previous);
            output.varint(if mask == previous_mask {
                0
            } else {
                u128::from(mask) << 1 | 1
            })?;
            for column in (0..FIELDS).filter(|column| mask & (1 << column) != 0) {
                output.change(words[column], previous[column])?;
            }
            (previous, previous_mask) = (words, mask);
        }
        Ok(())
    }
}
impl replay_rows::value::EncodeValue for Batch<'_> {
    fn encoded_bytes(&self) -> io::Result<usize> {
        let mut count = Count(0);
        self.write(&mut count)?;
        Ok(count.0)
    }
    fn recorded_bytes(&self) -> io::Result<usize> {
        self.encoded_bytes()
    }
    fn encode_value(&self, output: &mut Vec<u8>) -> io::Result<()> {
        allocation::reserve(output, self.encoded_bytes()?)?;
        self.write(output)
    }
}

pub fn decode(
    row: &EventFeedHistoryRow,
    input: &mut Decoder<'_>,
) -> io::Result<Vec<EventFeedEvent>> {
    let count = input.count(2)?;
    let mut events = Vec::new();
    allocation::reserve_exact(&mut events, count)?;
    let (mut sequence, mut words, mut held, mut previous_mask) =
        (0_u64, [None; FIELDS], 0_u64, 0_u64);
    for index in 0..count {
        let gap = wide_read(input)?;
        if index != 0 && gap == 0 {
            return Err(invalid("Event feed sequence repeated"));
        }
        sequence = sequence
            .checked_add(gap)
            .ok_or_else(|| invalid("Event feed sequence overflow"))?;
        let mask = match input.varint()? {
            0 => previous_mask,
            code if code & 1 == 1
                && code >> 1 >> FIELDS == 0
                && code >> 1 != previous_mask.into() =>
            {
                (code >> 1) as u64
            }
            _ => return Err(invalid("Event feed changed-word mask differs")),
        };
        let mut columns = mask;
        while columns != 0 {
            let column = columns.trailing_zeros() as usize;
            columns &= columns - 1;
            let value = changed_word(input, words[column])?;
            if value == words[column] {
                return Err(invalid("Event feed unchanged word stored as a change"));
            }
            words[column] = value;
            held = held & !(1 << column) | u64::from(value.is_some()) << column;
        }
        events.push(event(row, sequence, &words, held)?);
        previous_mask = mask;
    }
    Ok(events)
}

fn changed_word(input: &mut Decoder<'_>, previous: Option<u64>) -> io::Result<Option<u64>> {
    match input.u8()? {
        0 => return Ok(None),
        1 => {}
        _ => return Err(invalid("invalid optional integer tag")),
    }
    let encoded = input.varint()?;
    let difference = ((encoded >> 1) as i128) ^ -((encoded & 1) as i128);
    if difference.unsigned_abs() > u128::from(u64::MAX) {
        return Err(invalid("numeric difference exceeds native field range"));
    }
    let value = i128::from(previous.unwrap_or(0))
        .checked_add(difference)
        .ok_or_else(|| invalid("numeric delta addition overflow"))?;
    u64::try_from(value)
        .map(Some)
        .map_err(|_| invalid("numeric delta exceeds native field range"))
}

struct Read<'w> {
    words: &'w Words,
    read: u64,
}

impl Read<'_> {
    fn optional_word(&mut self, at: usize) -> Option<u64> {
        self.read |= 1 << at;
        self.words[at]
    }
    fn required(&mut self, at: usize) -> io::Result<u64> {
        self.optional_word(at)
            .ok_or_else(|| invalid("Event feed column operand absent"))
    }
    fn word(&mut self, at: usize) -> io::Result<u32> {
        u32::try_from(self.required(at)?).map_err(|_| invalid("Event feed column exceeds u32"))
    }
    fn optional(&mut self, at: usize) -> io::Result<Option<u32>> {
        self.optional_word(at)
            .map(|n| u32::try_from(n).map_err(|_| invalid("Event feed column exceeds u32")))
            .transpose()
    }
    fn country(&mut self, at: usize) -> io::Result<EventFeedCountry> {
        Ok(EventFeedCountry {
            reference: self.word(at)?,
            id: self.word(at + 1)?,
            instance: self.word(at + 2)?,
        })
    }
    fn byte(&mut self, at: usize) -> io::Result<u8> {
        u8::try_from(self.required(at)?).map_err(|_| invalid("Event feed column exceeds u8"))
    }
}

impl ChangeOperands for Read<'_> {
    fn operand(&mut self) -> io::Result<u32> {
        self.word(31)
    }
    fn decision(&mut self) -> io::Result<decisions::DecisionChange> {
        let operation = self.byte(32)?;
        let (container, flags, ordinal) = (self.byte(33)?, self.byte(34)?, self.word(35)?);
        let operation = decisions::Operation::of(operation)?;
        Ok(decisions::DecisionChange {
            operation,
            container,
            flags,
            ordinal,
            political_power: if operation == decisions::Operation::Rebuild {
                self.optional_word(36).map(|n| n as i64)
            } else {
                None
            },
        })
    }
}

fn event(
    row: &EventFeedHistoryRow,
    sequence: u64,
    words: &Words,
    held: u64,
) -> io::Result<EventFeedEvent> {
    fn route(value: u8) -> io::Result<EventFeedRoute> {
        EventFeedRoute::decode(&mut Decoder::new(&[value]))
    }
    let mut read = Read { words, read: 0 };
    let age = read.word(0)?;
    let hour = i32::try_from(i64::from(row.read_hour) - i64::from(age))
        .map_err(|_| invalid("Event feed relative hour exceeds i32"))?;
    let source = EventFeedSource::decode(&mut Decoder::new(&[read.byte(1)?]))?;
    let actor = read.country(2)?;
    let target = read.country(5)?;
    let cause = if words[8].is_some() {
        Recorded::Value(EventFeedCause {
            route: route(read.byte(8)?)?,
            within: route(read.byte(9)?)?,
            conference: read.optional_word(10),
            state: read.optional(11)?,
            ally: if words[12].is_some() {
                Some(read.country(12)?)
            } else {
                None
            },
            reason: read.optional(15)?,
            flags: read.byte(16)?,
        })
    } else {
        Recorded::NotRecorded
    };
    let script = if words[17].is_some() {
        let flags = read.word(17)?;
        let mut script = EventFeedScript {
            token_start: read.word(18)?,
            token_len: read.word(19)?,
            scope_start: read.word(20)?,
            scope_len: read.word(21)?,
            native_event: if words[22].is_some() {
                Some(EventFeedEventIdentity {
                    kind: read.word(22)?,
                    id: read.word(23)?,
                })
            } else {
                None
            },
            instance: read.optional(24)?,
            option: read.optional(25)?.map(signed_value),
            country_target: read.optional(26)?,
            numeric_target: read.optional(27)?.map(signed_value),
            lifecycle: read.optional(28)?.map(signed_value),
            days: read.optional(29)?.map(signed_value),
            scope_unrecorded: flags & (1 << 5) != 0,
            token_unrecorded: flags & (1 << 6) != 0,
            change: None,
        };
        if words[30].is_some() {
            let code = read.byte(30)?;
            script.change = Some(EventFeedScriptChange::of(code, &mut read)?);
        }
        if script.flags() != flags {
            return Err(invalid("Event feed script column flags differ"));
        }
        Some(script)
    } else {
        None
    };
    if held & !read.read != 0 {
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
    use replay_rows::rows::RowCodec;

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
        let rows = vec![row.clone()];
        let mut prepared = Vec::new();
        replay_rows::checkpoint::rows::encode_rows(&rows, &mut prepared).unwrap();
        assert_eq!(
            replay_rows::checkpoint::rows::decode_rows::<EventFeedHistoryRow>(&mut Decoder::new(&prepared))
                .unwrap(),
            rows
        );
    }

    #[test]
    fn event_feed_occurrences_gaps_extreme_relative_hours_refusals_and_empty_batches() {
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
    fn event_feed_occurrences_scripts_operations_and_out_of_order_event_hours() {
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
            DelayedRefused,
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
                    token_len: if identity || change == DelayedRefused {
                        5
                    } else {
                        0
                    },
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
            Operation::Refused,
        ] {
            let refused = operation == Operation::Refused;
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
                    container: if refused { 0 } else { 3 },
                    flags: if refused { 0 } else { u8::MAX },
                    ordinal: if refused { 0 } else { u32::MAX },
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
    fn event_feed_occurrences_truncation_overflow_unused_operands_and_duplicate_sequences_refuse() {
        let row = super::super::tests::row();
        let batch = Batch(&row);
        let mut encoded = Vec::new();
        replay_rows::value::EncodeValue::encode_value(&batch, &mut encoded).unwrap();
        for end in 0..encoded.len() {
            assert!(decode(&row, &mut Decoder::new(&encoded[..end])).is_err());
        }
        let mut malformed = encoded.clone();
        malformed[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&row, &mut Decoder::new(&malformed)).is_err());
        let held = |words: &Words| {
            (0..FIELDS).fold(0, |held, at| held | u64::from(words[at].is_some()) << at)
        };
        let mut entry = words(&row.events[0], row.read_hour);
        entry[36] = Some(1);
        assert!(event(&row, 1, &entry, held(&entry)).is_err());
        let mut entry = words(&row.events[0], row.read_hour);
        entry[0] = Some(u64::MAX);
        assert!(event(&row, 1, &entry, held(&entry)).is_err());
        let mut repeated = row.clone();
        repeated.events.push(repeated.events[0]);
        assert!(repeated.validate().is_err());
    }

    fn roster(row: &mut EventFeedHistoryRow, owner: u32, targets: impl Iterator<Item = u32>) {
        use decisions::{DecisionChange, Operation};
        for target in targets {
            let sequence = row.events.last().map_or(40, |event| event.sequence + 1);
            row.events.push(EventFeedEvent {
                sequence,
                generation: row.generation,
                hour: row.read_hour,
                source: EventFeedSource::ScriptState,
                actor: EventFeedCountry {
                    reference: owner,
                    id: owner,
                    instance: 0,
                },
                target: EventFeedCountry {
                    reference: target,
                    id: target,
                    instance: 0,
                },
                cause: Recorded::Value(Default::default()),
                script: Some(EventFeedScript {
                    token_len: row.script_tokens.len() as u32,
                    country_target: Some(owner),
                    change: Some(EventFeedScriptChange::Decision(DecisionChange {
                        operation: Operation::Candidate,
                        container: 0,
                        flags: 0,
                        ordinal: target,
                        political_power: None,
                    })),
                    ..Default::default()
                }),
            });
        }
    }

    fn batch(row: &EventFeedHistoryRow) -> Vec<u8> {
        let mut encoded = Vec::new();
        replay_rows::value::EncodeValue::encode_value(&Batch(row), &mut encoded).unwrap();
        assert_eq!(
            replay_rows::value::EncodeValue::encoded_bytes(&Batch(row)).unwrap(),
            encoded.len()
        );
        encoded
    }

    #[test]
    fn event_feed_occurrences_store_changed_words_and_repeat_a_run_byte_for_byte() {
        let mut row = super::super::tests::row();
        row.events.clear();
        row.counts = None;
        row.coverage = EventFeedCoverage::Partial;
        row.script_tokens = b"synthetic.targeted".to_vec();
        roster(&mut row, 3, 1..=40);
        roster(&mut row, 4, (1..=40).filter(|target| *target != 4));
        roundtrip(&row);
        let alone = {
            let mut first = row.clone();
            first.events.truncate(40);
            batch(&first)
        };
        let repeated = &alone[alone.len() - 38 * 8..];
        assert!(
            repeated
                .chunks(8)
                .all(|item| item == [1, 0, 1, 2, 1, 2, 1, 2])
        );
        let mut later = row.clone();
        later.read_hour += 24;
        later.events.clear();
        later.events.push(EventFeedEvent {
            sequence: 9_000,
            hour: later.read_hour,
            ..row.events[45]
        });
        roster(&mut later, 3, 1..=40);
        roundtrip(&later);
        assert!(batch(&later).ends_with(repeated));
    }

    fn stream(
        row: &EventFeedHistoryRow,
        mut code: impl FnMut(usize, u64, u64) -> (u128, u64),
    ) -> Vec<u8> {
        let mut output = Vec::new();
        output
            .put(&(row.events.len() as u32).to_le_bytes())
            .unwrap();
        let (mut sequence, mut previous, mut previous_mask) = (0, [None; FIELDS], 0);
        for (index, event) in row.events.iter().enumerate() {
            output.varint((event.sequence - sequence).into()).unwrap();
            sequence = event.sequence;
            let words = words(event, row.read_hour);
            let (value, stored) = code(index, changed(&words, &previous), previous_mask);
            output.varint(value).unwrap();
            for column in (0..FIELDS).filter(|column| stored & (1 << column) != 0) {
                output.change(words[column], previous[column]).unwrap();
            }
            (previous, previous_mask) = (words, stored);
        }
        output
    }

    #[test]
    fn event_feed_occurrences_refuse_noncanonical_masks_and_unchanged_words() {
        let mut row = super::super::tests::row();
        row.events.clear();
        row.counts = None;
        row.coverage = EventFeedCoverage::Partial;
        row.script_tokens = b"synthetic.targeted".to_vec();
        roster(&mut row, 3, 1..=3);
        let canonical = |_, mask, previous| {
            (
                if mask == previous {
                    0
                } else {
                    u128::from(mask) << 1 | 1
                },
                mask,
            )
        };
        let encoded = stream(&row, canonical);
        assert_eq!(encoded, batch(&row));
        assert_eq!(
            decode(&row, &mut Decoder::new(&encoded)).unwrap(),
            row.events
        );
        let explicit = stream(&row, |_, mask, _| (u128::from(mask) << 1 | 1, mask));
        assert!(decode(&row, &mut Decoder::new(&explicit)).is_err());
        let unchanged = stream(&row, |index, mask, previous| {
            if index == 2 {
                let stored = mask | 1 << 3;
                (u128::from(stored) << 1 | 1, stored)
            } else {
                canonical(index, mask, previous)
            }
        });
        assert!(decode(&row, &mut Decoder::new(&unchanged)).is_err());
        for code in [(1_u128 << FIELDS) << 1 | 1, 2] {
            let malformed = stream(&row, |index, mask, previous| {
                if index == 1 {
                    (code, mask)
                } else {
                    canonical(index, mask, previous)
                }
            });
            assert!(decode(&row, &mut Decoder::new(&malformed)).is_err());
        }
    }
}
