//! Owned simulation-event history and interval coverage.
//! Operation hours filter history; supplying hours govern its first visibility.
//! The country instance word is native +C, never a writer generation.

use crate::{
    codec::{Decoder, invalid_static as invalid},
    value::{Value, value_struct},
};
use engine::allocation;
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventFeedSource {
    Declaration = 0,
    Join = 1,
    CivilWarWarTransfer = 2,
    Annex = 3,
}

impl Value for EventFeedSource {
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(1)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        (*self as u8).encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match u8::decode(input)? {
            0 => Ok(Self::Declaration),
            1 => Ok(Self::Join),
            2 => Ok(Self::CivilWarWarTransfer),
            3 => Ok(Self::Annex),
            _ => Err(invalid("unknown Event feed source")),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventFeedCoverage {
    #[default]
    Unavailable = 0,
    Partial = 1,
    Complete = 2,
}

impl Value for EventFeedCoverage {
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(1)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        (*self as u8).encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match u8::decode(input)? {
            0 => Ok(Self::Unavailable),
            1 => Ok(Self::Partial),
            2 => Ok(Self::Complete),
            _ => Err(invalid("unknown Event feed coverage")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFeedCountry {
    pub reference: u32,
    pub id: u32,
    pub instance: u32,
}
value_struct!(EventFeedCountry {
    reference: u32,
    id: u32,
    instance: u32
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFeedEvent {
    /// Attachment-wide identity seeded from retained replay history, not a pointer.
    pub sequence: u64,
    /// World occurrence supplied by the collector, not a native object field.
    pub generation: u64,
    pub hour: i32,
    pub source: EventFeedSource,
    pub actor: EventFeedCountry,
    pub target: EventFeedCountry,
}
value_struct!(EventFeedEvent {
    sequence: u64,
    generation: u64,
    hour: i32,
    source: EventFeedSource,
    actor: EventFeedCountry,
    target: EventFeedCountry
});

/// Cumulative attachment accounting survives drains and world replacement.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EventFeedCounts {
    pub acquired: u64,
    pub settled: u64,
    pub failed: u64,
    pub rejected: u64,
}
value_struct!(EventFeedCounts {
    acquired: u64,
    settled: u64,
    failed: u64,
    rejected: u64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EventFeedHistoryRow {
    pub generation: u64,
    pub read_hour: i32,
    pub coverage: EventFeedCoverage,
    /// None after count overflow; never a saturated or wrapped known total.
    pub counts: Option<EventFeedCounts>,
    pub outstanding: u64,
    pub failure: Option<Vec<u8>>,
    /// Known facts also survive partial/unavailable coverage. Missing events
    /// in those intervals cannot be interpreted as proof that nothing happened.
    pub events: Vec<EventFeedEvent>,
}

impl EventFeedHistoryRow {
    pub fn validate(&self) -> io::Result<()> {
        if self.coverage == EventFeedCoverage::Complete
            && (self.outstanding != 0 || self.failure.is_some() || self.counts.is_none())
        {
            return Err(invalid("incomplete Event feed accounting marked complete"));
        }
        if self.coverage == EventFeedCoverage::Unavailable && self.failure.is_none() {
            return Err(invalid("unavailable Event feed coverage has no refusal"));
        }
        if let Some(counts) = self.counts
            && (counts.settled > counts.acquired
                || counts.failed > counts.settled
                || counts.acquired - counts.settled != self.outstanding
                || self.events.len() as u64 > counts.acquired - counts.failed
                || self
                    .events
                    .iter()
                    .filter(|event| event.source != EventFeedSource::Annex)
                    .count() as u64
                    > counts.settled - counts.failed)
        {
            return Err(invalid("inconsistent Event feed accounting"));
        }
        let mut previous = None;
        for event in &self.events {
            if event.hour > self.read_hour
                || event.generation != self.generation
                || previous.is_some_and(|sequence| event.sequence <= sequence)
            {
                return Err(invalid("Event feed history identity or order differs"));
            }
            previous = Some(event.sequence);
        }
        Ok(())
    }
}

impl Value for EventFeedHistoryRow {
    const MINIMUM_ENCODED_BYTES: usize = 27;
    fn encoded_bytes(&self) -> io::Result<usize> {
        self.validate()?;
        let size = allocation::add(21, self.counts.encoded_bytes()?)?;
        let size = allocation::add(size, self.failure.encoded_bytes()?)?;
        allocation::add(size, self.events.encoded_bytes()?)
    }
    fn try_clone(&self) -> io::Result<Self> {
        self.validate()?;
        Ok(Self {
            generation: self.generation,
            read_hour: self.read_hour,
            coverage: self.coverage,
            counts: self.counts,
            outstanding: self.outstanding,
            failure: self.failure.try_clone()?,
            events: self.events.try_clone()?,
        })
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.validate()?;
        self.generation.encode_reserved(output)?;
        self.read_hour.encode_reserved(output)?;
        self.coverage.encode_reserved(output)?;
        self.counts.encode_reserved(output)?;
        self.outstanding.encode_reserved(output)?;
        self.failure.encode_reserved(output)?;
        self.events.encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let row = Self {
            generation: u64::decode(input)?,
            read_hour: i32::decode(input)?,
            coverage: EventFeedCoverage::decode(input)?,
            counts: Option::decode(input)?,
            outstanding: u64::decode(input)?,
            failure: Option::decode(input)?,
            events: Vec::decode(input)?,
        };
        row.validate()?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> EventFeedHistoryRow {
        EventFeedHistoryRow {
            generation: 9,
            read_hour: 12,
            coverage: EventFeedCoverage::Complete,
            counts: Some(EventFeedCounts {
                acquired: 2,
                settled: 2,
                failed: 0,
                rejected: 0,
            }),
            outstanding: 0,
            failure: None,
            events: vec![EventFeedEvent {
                sequence: 4,
                generation: 9,
                hour: -17,
                source: EventFeedSource::Annex,
                actor: EventFeedCountry {
                    reference: 3,
                    id: 2,
                    instance: u32::MAX,
                },
                target: EventFeedCountry {
                    reference: 0,
                    id: 0,
                    instance: 7,
                },
            }],
        }
    }

    #[test]
    fn event_hour_raw_alias_and_zero_holder_survive_later_visibility() {
        let row = row();
        let mut bytes = Vec::new();
        row.encode(&mut bytes).unwrap();
        assert_eq!(bytes.len(), row.encoded_bytes().unwrap());
        let mut input = Decoder::new(&bytes);
        assert_eq!(EventFeedHistoryRow::decode(&mut input).unwrap(), row);
        input.finish().unwrap();
        for end in 0..bytes.len() {
            assert!(EventFeedHistoryRow::decode(&mut Decoder::new(&bytes[..end])).is_err());
        }
    }

    #[test]
    fn complete_empty_partial_empty_and_unavailable_are_distinct() {
        let mut row = row();
        row.events.clear();
        row.validate().unwrap();
        row.coverage = EventFeedCoverage::Partial;
        row.validate().unwrap();
        row.coverage = EventFeedCoverage::Unavailable;
        assert!(row.validate().is_err());
        row.failure = Some(b"missed producer".to_vec());
        row.validate().unwrap();
        row.coverage = EventFeedCoverage::Complete;
        assert!(row.validate().is_err());
    }

    #[test]
    fn annex_before_dispatch_remains_known_with_an_unsettled_native_call() {
        let mut row = row();
        row.coverage = EventFeedCoverage::Unavailable;
        row.failure = Some(b"native dispatcher escaped".to_vec());
        row.counts = Some(EventFeedCounts {
            acquired: 1,
            ..Default::default()
        });
        row.outstanding = 1;
        row.validate().unwrap();
        let mut bytes = Vec::new();
        row.encode(&mut bytes).unwrap();
        assert_eq!(
            EventFeedHistoryRow::decode(&mut Decoder::new(&bytes)).unwrap(),
            row
        );
        row.events[0].source = EventFeedSource::Declaration;
        assert!(row.validate().is_err());
    }

    #[test]
    fn malformed_kinds_counts_generation_and_duplicate_sequences_refuse() {
        assert!(EventFeedSource::decode(&mut Decoder::new(&[4])).is_err());
        assert!(EventFeedCoverage::decode(&mut Decoder::new(&[3])).is_err());
        let mut row = row();
        row.counts.as_mut().unwrap().settled = 3;
        assert!(row.validate().is_err());
        row.counts.as_mut().unwrap().settled = 2;
        row.events[0].generation = 8;
        assert!(row.validate().is_err());
        row.events[0].generation = 9;
        row.events[0].hour = row.read_hour + 1;
        assert!(row.validate().is_err());
        row.events[0].hour = row.read_hour;
        row.events.push(row.events[0]);
        assert!(row.validate().is_err());
    }

    #[test]
    fn registered_history_snapshot_delta_and_older_format_keep_coverage() {
        use crate::{
            frame::{DecodedFrame, FrameDecoder, FrameEncoder, Metrics, TableView},
            rows::{TableId, TableRows, tables},
        };
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        for (hour, mut observed) in [(12, row()), (13, row())] {
            observed.read_hour = hour;
            if hour == 13 {
                observed.coverage = EventFeedCoverage::Unavailable;
                observed.failure = Some(b"world observation unavailable".to_vec());
                observed.events.clear();
            }
            let mut views = std::array::from_fn(|_| TableView::Unavailable {
                reason: "fixture".into(),
            });
            views[TableId::EventFeedHistory.index()] =
                TableView::Complete(TableRows::EventFeedHistory(vec![observed.clone()]));
            let encoded = encoder
                .encode(
                    DecodedFrame {
                        branch: 1,
                        hour,
                        tables: views.map(Into::into),
                        metrics: Metrics::default(),
                    },
                    hour == 12,
                )
                .unwrap();
            let decoded = decoder.apply(&encoded).unwrap();
            assert_eq!(
                decoded.complete::<tables::EventFeedHistory>().unwrap(),
                &[observed]
            );
        }
        let older = crate::format::FormatVersion::read(99).unwrap();
        assert!(older.table_refusal(TableId::EventFeedHistory).is_some());
        assert!(older.table_refusal(TableId::Countries).is_none());
    }
}
