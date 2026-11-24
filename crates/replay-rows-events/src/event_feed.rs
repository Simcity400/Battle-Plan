use super::*;
use crate::event_feed_rows::columns::{self, Batch};

#[cfg(any(test, feature = "test-support"))]
use replay_rows::checkpoint::endpoint::sparse::SparseFixture;

impl Fields for EventFeedHistoryRow {
    const SCHEMA: &'static [Kind] = &[
        Kind::U64,
        Kind::I32,
        Kind::Opaque,
        Kind::Opaque,
        Kind::U64,
        Kind::Opaque,
        Kind::Opaque,
        Kind::Bytes,
        Kind::Opaque,
        Kind::U8,
        Kind::Opaque,
    ];
    const STEPS: &'static [Option<Step>] = &[None; 11];
    const NAMES: &'static [&'static str] = &[
        "generation",
        "read_hour",
        "coverage",
        "counts",
        "outstanding",
        "failure",
        "events",
        "script_tokens",
        "script_scopes",
        "script_refusals",
        "windows",
    ];
}
impl EventFeedHistoryRow {
    pub const fn key(&self) -> u128 {
        0
    }
    fn with_fields<T>(&self, f: impl FnOnce([Field<'_>; 11]) -> io::Result<T>) -> io::Result<T> {
        let events = Batch(self);
        f([
            self.generation.field(),
            self.read_hour.field(),
            self.coverage.field(),
            self.counts.field(),
            self.outstanding.field(),
            self.failure.field(),
            Field::Opaque(Opaque::Native(&events)),
            self.script_tokens.field(),
            self.script_scopes.field(),
            self.script_refusals.field(),
            self.windows.field(),
        ])
    }
    fn change_mask(&self, previous: Option<&Self>) -> u64 {
        let Some(old) = previous else {
            return full_mask(11);
        };
        let changed = [
            self.generation != old.generation,
            self.read_hour != old.read_hour,
            self.coverage != old.coverage,
            self.counts != old.counts,
            self.outstanding != old.outstanding,
            self.failure != old.failure,
            self.events != old.events,
            self.script_tokens != old.script_tokens,
            self.script_scopes != old.script_scopes,
            self.script_refusals != old.script_refusals,
            self.windows != old.windows,
        ];
        changed
            .into_iter()
            .enumerate()
            .fold(0, |mask, (index, changed)| {
                mask | (u64::from(changed) << index)
            })
    }
    fn encode_changes(
        &self,
        previous: Option<&Self>,
        mask: u64,
        output: &mut Vec<u8>,
    ) -> io::Result<()> {
        self.with_fields(|fields| {
            let mut encode = |before: [Field<'_>; 11]| {
                for (index, (value, old)) in fields.into_iter().zip(before).enumerate() {
                    if mask & (1 << index) != 0 {
                        value.change(old)?.encode(output)?;
                    }
                }
                Ok(())
            };
            match previous {
                Some(old) => old.with_fields(encode),
                None => encode([Field::Empty; 11]),
            }
        })
    }
    fn read_events(&mut self, input: &mut Decoder<'_>) -> io::Result<()> {
        let Field::Opaque(Opaque::Encoded(bytes, format)) = Kind::Opaque.decode(input)? else {
            unreachable!()
        };
        let mut input = Decoder::new(bytes);
        input.format = format;
        self.events = columns::decode(self, &mut input)?;
        input.finish()
    }
}
impl RowCodec for EventFeedHistoryRow {
    const KEY_BITS: u32 = 0;
    fn key(&self) -> u128 {
        0
    }
    fn heap_bytes(&self) -> usize {
        Value::heap_bytes(self)
    }
    fn has_display_steps(&self) -> bool {
        false
    }
    fn round_to_display(&mut self) {}
    fn check_key_fields(&self) -> io::Result<()> {
        self.validate()
    }
    fn not_recorded_fields(&self) -> u64 {
        u64::from(matches!(self.windows, Recorded::NotRecorded)) << 10
    }
    fn visit_nested_field_gaps(&self, _: &mut dyn FnMut(&'static str)) {}
    fn visit_wire(
        &self,
        visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.with_fields(|fields| {
            for (index, field) in fields.into_iter().enumerate() {
                visit(index, field)?;
            }
            Ok(())
        })
    }
    fn try_clone(&self) -> io::Result<Self> {
        Value::try_clone(self)
    }
    fn try_clone_validated(&self) -> io::Result<Self> {
        Ok(Self {
            failure: Value::try_clone(&self.failure)?,
            events: Value::try_clone(&self.events)?,
            script_tokens: Value::try_clone(&self.script_tokens)?,
            script_scopes: Value::try_clone(&self.script_scopes)?,
            windows: Value::try_clone(&self.windows)?,
            ..*self
        })
    }
    fn recorded_fields_bytes(&self) -> io::Result<usize> {
        self.with_fields(|fields| {
            fields.into_iter().try_fold(0, |size, field| {
                allocation::add(size, field.recorded_bytes()?)
            })
        })
    }
    fn snapshot_bytes(&self) -> io::Result<usize> {
        self.with_fields(|fields| {
            fields.into_iter().try_fold(1, |size, field| {
                allocation::add(size, field.encoded_bytes()?)
            })
        })
    }
    fn encoded_bound(&self) -> io::Result<usize> {
        self.with_fields(|fields| {
            fields.into_iter().try_fold(3, |size, field| {
                allocation::add(size, field.encoded_bound()?)
            })
        })
    }
    fn encode_change_reserved(
        &self,
        previous: Option<&Self>,
        previous_key: u128,
        output: &mut Vec<u8>,
    ) -> io::Result<bool> {
        check_native_key(previous_key, 0)?;
        self.validate()?;
        if let Some(old) = previous {
            old.validate()?;
        }
        let mask = self.change_mask(previous);
        if mask == 0 {
            return Ok(false);
        }
        put_varint(output, 0);
        output.extend_from_slice(&(mask as u16).to_le_bytes());
        self.encode_changes(previous, mask, output)?;
        Ok(true)
    }
    fn delta(&self, previous: Option<&Self>) -> io::Result<Option<DeltaRow>> {
        self.validate()?;
        if let Some(old) = previous {
            old.validate()?;
        }
        let mask = self.change_mask(previous);
        if mask == 0 {
            return Ok(None);
        }
        let mut values = Vec::new();
        self.encode_changes(previous, mask, &mut values)?;
        Ok(Some(DeltaRow {
            format: replay_rows::FORMAT_VERSION,
            key: 0,
            mask,
            values,
            recorded_fields: 11,
        }))
    }
    fn encode_fields_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.with_fields(|fields| {
            for field in fields {
                field.encode(output)?;
            }
            Ok(())
        })
    }
    fn from_wire<'a>(
        key: u128,
        wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
    ) -> io::Result<Self> {
        let mut bytes = Vec::new();
        for index in 0..10 {
            wire(index)?.encode(&mut bytes)?;
        }
        let recorded_fields = match wire(10)? {
            Field::NotRecorded => 10,
            windows => {
                windows.encode(&mut bytes)?;
                11
            }
        };
        let mut input = Decoder::new(&bytes);
        input.recorded_fields = recorded_fields;
        let row = Self::decode_fields(key, &mut input)?;
        input.finish()?;
        Ok(row)
    }
    fn decode_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self> {
        check_native_key(key, 0)?;
        let mut row = Self {
            generation: read_value(Kind::U64, input)?,
            read_hour: read_value(Kind::I32, input)?,
            coverage: read_value(Kind::Opaque, input)?,
            counts: read_value(Kind::Opaque, input)?,
            outstanding: read_value(Kind::U64, input)?,
            failure: read_value(Kind::Opaque, input)?,
            ..Default::default()
        };
        row.read_events(input)?;
        row.script_tokens = read_value(Kind::Bytes, input)?;
        row.script_scopes = read_value(Kind::Opaque, input)?;
        row.script_refusals = read_value(Kind::U8, input)?;
        row.windows = read_field(10, Kind::Opaque, input)?;
        row.validate()?;
        Ok(row)
    }
    fn apply_delta(delta: &DeltaRow, previous: Option<Self>) -> io::Result<Self> {
        check_native_key(delta.key, 0)?;
        let fields = delta.recorded_fields;
        if !(10..=11).contains(&fields) || delta.mask == 0 || delta.mask & !full_mask(fields) != 0 {
            return Err(invalid("Event feed delta fields differ"));
        }
        let mut row = if let Some(row) = previous {
            row.validate()?;
            row
        } else {
            if delta.mask != full_mask(fields) {
                return Err(invalid("inserted row requires every field"));
            }
            Self::default()
        };
        let mut input = Decoder::new(&delta.values);
        input.format = delta.format;
        macro_rules! apply {
            ($index:literal, $field:ident, $kind:ident) => {
                if delta.mask & (1 << $index) != 0 {
                    apply_value(&mut row.$field, Kind::$kind, &mut input)?;
                }
            };
        }
        apply!(0, generation, U64);
        apply!(1, read_hour, I32);
        apply!(2, coverage, Opaque);
        apply!(3, counts, Opaque);
        apply!(4, outstanding, U64);
        apply!(5, failure, Opaque);
        if delta.mask & (1 << 6) != 0 {
            row.read_events(&mut input)?;
        }
        apply!(7, script_tokens, Bytes);
        apply!(8, script_scopes, Opaque);
        apply!(9, script_refusals, U8);
        if fields <= 10 {
            row.windows = Recorded::NotRecorded;
        }
        apply!(10, windows, Opaque);
        input.finish()?;
        row.validate()?;
        Ok(row)
    }
}
impl replay_rows::checkpoint::rows::Row for EventFeedHistoryRow {
    #[inline(never)]
    fn encode_endpoint_fields(&self, mask: u64, output: &mut Vec<u8>) -> io::Result<()> {
        if mask & (1 << 0) != 0 {
            Value::encode(&self.generation, output)?;
        }
        if mask & (1 << 1) != 0 {
            Value::encode(&self.read_hour, output)?;
        }
        if mask & (1 << 2) != 0 {
            Value::encode(&self.coverage, output)?;
        }
        if mask & (1 << 3) != 0 {
            Value::encode(&self.counts, output)?;
        }
        if mask & (1 << 4) != 0 {
            Value::encode(&self.outstanding, output)?;
        }
        if mask & (1 << 5) != 0 {
            Value::encode(&self.failure, output)?;
        }
        if mask & (1 << 6) != 0 {
            Value::encode(&self.events, output)?;
        }
        if mask & (1 << 7) != 0 {
            Value::encode(&self.script_tokens, output)?;
        }
        if mask & (1 << 8) != 0 {
            Value::encode(&self.script_scopes, output)?;
        }
        if mask & (1 << 9) != 0 {
            Value::encode(&self.script_refusals, output)?;
        }
        if mask & (1 << 10) != 0 {
            Value::encode(&self.windows, output)?;
        }
        Ok(())
    }
    #[inline(never)]
    fn decode_endpoint_fields(key: u128, mask: u64, input: &mut Decoder<'_>) -> io::Result<Self> {
        check_native_key(key, 0)?;
        let mut row = Self::default();
        if mask & (1 << 0) != 0 {
            row.generation = Value::decode(input)?;
        }
        if mask & (1 << 1) != 0 {
            row.read_hour = Value::decode(input)?;
        }
        if mask & (1 << 2) != 0 {
            row.coverage = Value::decode(input)?;
        }
        if mask & (1 << 3) != 0 {
            row.counts = Value::decode(input)?;
        }
        if mask & (1 << 4) != 0 {
            row.outstanding = Value::decode(input)?;
        }
        if mask & (1 << 5) != 0 {
            row.failure = Value::decode(input)?;
        }
        if mask & (1 << 6) != 0 {
            row.events = Value::decode(input)?;
        }
        if mask & (1 << 7) != 0 {
            row.script_tokens = Value::decode(input)?;
        }
        if mask & (1 << 8) != 0 {
            row.script_scopes = Value::decode(input)?;
        }
        if mask & (1 << 9) != 0 {
            row.script_refusals = Value::decode(input)?;
        }
        if mask & (1 << 10) != 0 {
            row.windows = Value::decode(input)?;
        }
        Ok(row)
    }
    #[inline(never)]
    fn install_endpoint_fields(&mut self, next: Self, mask: u64) -> io::Result<()> {
        if mask & (1 << 0) != 0 {
            self.generation = next.generation;
        }
        if mask & (1 << 1) != 0 {
            self.read_hour = next.read_hour;
        }
        if mask & (1 << 2) != 0 {
            self.coverage = next.coverage;
        }
        if mask & (1 << 3) != 0 {
            self.counts = next.counts;
        }
        if mask & (1 << 4) != 0 {
            self.outstanding = next.outstanding;
        }
        if mask & (1 << 5) != 0 {
            self.failure = next.failure;
        }
        if mask & (1 << 6) != 0 {
            self.events = next.events;
        }
        if mask & (1 << 7) != 0 {
            self.script_tokens = next.script_tokens;
        }
        if mask & (1 << 8) != 0 {
            self.script_scopes = next.script_scopes;
        }
        if mask & (1 << 9) != 0 {
            self.script_refusals = next.script_refusals;
        }
        if mask & (1 << 10) != 0 {
            self.windows = next.windows;
        }
        Ok(())
    }
    fn encode_checkpoint_fields(&self, output: &mut Vec<u8>) -> io::Result<()> {
        Value::encode(self, output)
    }
    fn decode_checkpoint_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self> {
        check_native_key(key, 0)?;
        Value::decode(input)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl replay_rows::checkpoint::endpoint::sparse::SparseReference for EventFeedHistoryRow {
    fn sparse_fixture(&self, field: usize) -> SparseFixture<Self> {
        use crate::event_feed_rows::*;
        let baseline = Self {
            generation: 7,
            read_hour: 100,
            coverage: EventFeedCoverage::Partial,
            counts: None,
            outstanding: 3,
            failure: Some(b"before".to_vec()),
            script_refusals: 1,
            events: vec![],
            script_tokens: b"old".to_vec(),
            script_scopes: vec![],
            windows: Recorded::NotRecorded,
        };
        let generation = if field == 6 { 7 } else { 9 };
        let source = Self {
            generation,
            read_hour: 200,
            coverage: EventFeedCoverage::Unavailable,
            counts: (field == 3).then_some(EventFeedCounts {
                acquired: 11,
                settled: 8,
                failed: 0,
                rejected: 1,
            }),
            outstanding: if field == 3 { 3 } else { 5 },
            failure: Some(b"after".to_vec()),
            script_refusals: 2,
            events: vec![EventFeedEvent {
                sequence: 11,
                generation,
                hour: 50,
                source: EventFeedSource::Annex,
                actor: EventFeedCountry::default(),
                target: EventFeedCountry::default(),
                cause: Recorded::NotRecorded,
                script: None,
            }],
            script_tokens: b"new".to_vec(),
            script_scopes: vec![EventFeedScope {
                country: 2,
                ..Default::default()
            }],
            windows: Recorded::Value(EventWindows {
                verdicts: Ok(vec![VerdictEntry {
                    sequence: 11,
                    verdict: WindowVerdict::Requested,
                }]),
                presentations: Vec::new(),
                presentations_unrecorded: None,
            }),
        };
        baseline.validate().unwrap();
        source.validate().unwrap();
        let mut expected = baseline.clone();
        let mut payload = Vec::new();
        macro_rules! assign {
            ($member:ident) => {{
                expected.$member = source.$member.clone();
                Value::encode(&source.$member, &mut payload).unwrap();
            }};
        }
        match field {
            0 => assign!(generation),
            1 => assign!(read_hour),
            2 => assign!(coverage),
            3 => assign!(counts),
            4 => assign!(outstanding),
            5 => assign!(failure),
            6 => assign!(events),
            7 => assign!(script_tokens),
            8 => assign!(script_scopes),
            9 => assign!(script_refusals),
            10 => assign!(windows),
            _ => unreachable!(),
        }
        expected.validate().unwrap();
        SparseFixture {
            baseline,
            source,
            expected,
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_feed_rows::tests::row;

    fn read(fields: &[Vec<u8>], present: usize) -> EventFeedHistoryRow {
        let schema = <EventFeedHistoryRow as Fields>::SCHEMA;
        EventFeedHistoryRow::from_wire(0, &mut |index| match fields.get(index).filter(|_| index < present) {
            Some(bytes) => schema[index].decode(&mut Decoder::new(bytes)),
            None => Ok(Field::NotRecorded),
        })
        .unwrap()
    }

    #[test]
    fn a_row_written_before_window_capture_reads_with_its_windows_not_recorded() {
        let written = row();
        let mut fields = Vec::new();
        written
            .visit_wire(&mut |_, field| {
                let mut bytes = Vec::new();
                field.encode(&mut bytes)?;
                fields.push(bytes);
                Ok(())
            })
            .unwrap();
        let older = read(&fields, 10);
        assert_eq!(older.windows, Recorded::NotRecorded);
        assert_eq!(older.events, written.events);
        assert_eq!(RowCodec::not_recorded_fields(&older), 1 << 10);
        let current = read(&fields, 11);
        assert_eq!(current.windows, written.windows);
        assert_eq!(RowCodec::not_recorded_fields(&current), 0);
    }
}
