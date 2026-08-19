use crate::calendar::hour_index_from_raw;
use crate::record_semantics::{
    validate_domain_gap_payload, COMPACT_DOMAIN_GAP_MAGIC, COMPACT_DOMAIN_GAP_PAYLOAD_LEN,
    COMPACT_DOMAIN_GAP_VERSION, DOMAIN_GAP_PART_BOTH, DOMAIN_GAP_PART_EVENT, DOMAIN_GAP_PART_STATE,
    DOMAIN_GAP_REASON_BACKPRESSURE, DOMAIN_GAP_REASON_CLOCK_UNREADABLE,
    DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH, DOMAIN_GAP_REASON_OVERRUN,
    DOMAIN_GAP_REASON_READER_SILENT, DOMAIN_GAP_REASON_SOURCE_UNAVAILABLE, DOMAIN_GAP_REASON_TORN,
    DOMAIN_GAP_REASON_WORKER_PANIC, DOMAIN_GAP_WITNESS_PROVENANCE_BRACKET_OBSERVED,
    DOMAIN_GAP_WITNESS_PROVENANCE_UNOBSERVED, DOMAIN_GAP_WITNESS_PROVENANCE_UNSTATED,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
pub enum DomainGapReason {
    Torn = DOMAIN_GAP_REASON_TORN,
    Overrun = DOMAIN_GAP_REASON_OVERRUN,
    Backpressure = DOMAIN_GAP_REASON_BACKPRESSURE,
    ClockUnreadable = DOMAIN_GAP_REASON_CLOCK_UNREADABLE,
    WorkerPanic = DOMAIN_GAP_REASON_WORKER_PANIC,
    SourceUnavailable = DOMAIN_GAP_REASON_SOURCE_UNAVAILABLE,
    ReaderSilent = DOMAIN_GAP_REASON_READER_SILENT,
    HourLabelMismatch = DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH,
}

impl DomainGapReason {
    pub const ALL: [DomainGapReason; 8] = [
        Self::Torn,
        Self::Overrun,
        Self::Backpressure,
        Self::ClockUnreadable,
        Self::WorkerPanic,
        Self::SourceUnavailable,
        Self::ReaderSilent,
        Self::HourLabelMismatch,
    ];

    const fn position(self) -> usize {
        match self {
            Self::Torn => 0,
            Self::Overrun => 1,
            Self::Backpressure => 2,
            Self::ClockUnreadable => 3,
            Self::WorkerPanic => 4,
            Self::SourceUnavailable => 5,
            Self::ReaderSilent => 6,
            Self::HourLabelMismatch => 7,
        }
    }

    pub const fn code(self) -> u16 {
        self as u16
    }

    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Torn => "torn",
            Self::Overrun => "overrun",
            Self::Backpressure => "backpressure",
            Self::ClockUnreadable => "clock_unreadable",
            Self::WorkerPanic => "worker_panic",
            Self::SourceUnavailable => "source_unavailable",
            Self::ReaderSilent => "reader_silent",
            Self::HourLabelMismatch => "hour_label_mismatch",
        }
    }

    pub fn from_wire_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reason| reason.wire_name() == name)
    }

    fn from_code(code: u16) -> Result<Self, DomainGapError> {
        match code {
            DOMAIN_GAP_REASON_TORN => Ok(Self::Torn),
            DOMAIN_GAP_REASON_OVERRUN => Ok(Self::Overrun),
            DOMAIN_GAP_REASON_BACKPRESSURE => Ok(Self::Backpressure),
            DOMAIN_GAP_REASON_CLOCK_UNREADABLE => Ok(Self::ClockUnreadable),
            DOMAIN_GAP_REASON_WORKER_PANIC => Ok(Self::WorkerPanic),
            DOMAIN_GAP_REASON_SOURCE_UNAVAILABLE => Ok(Self::SourceUnavailable),
            DOMAIN_GAP_REASON_READER_SILENT => Ok(Self::ReaderSilent),
            DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH => Ok(Self::HourLabelMismatch),
            other => Err(DomainGapError::InvalidSemantics(format!(
                "DOMAIN_GAP reason {other} is unknown"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u16)]
pub enum DomainGapWitnessProvenance {
    #[default]
    Unstated = DOMAIN_GAP_WITNESS_PROVENANCE_UNSTATED,
    BracketObserved = DOMAIN_GAP_WITNESS_PROVENANCE_BRACKET_OBSERVED,
    Unobserved = DOMAIN_GAP_WITNESS_PROVENANCE_UNOBSERVED,
}

impl DomainGapWitnessProvenance {
    pub const ALL: [DomainGapWitnessProvenance; 3] =
        [Self::Unstated, Self::BracketObserved, Self::Unobserved];

    pub const fn code(self) -> u16 {
        self as u16
    }

    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Unstated => "unstated",
            Self::BracketObserved => "bracket_observed",
            Self::Unobserved => "unobserved",
        }
    }

    pub const fn is_a_bracket_observation(self) -> bool {
        matches!(self, Self::BracketObserved)
    }

    fn from_code(code: u16) -> Result<Self, DomainGapError> {
        match code {
            DOMAIN_GAP_WITNESS_PROVENANCE_UNSTATED => Ok(Self::Unstated),
            DOMAIN_GAP_WITNESS_PROVENANCE_BRACKET_OBSERVED => Ok(Self::BracketObserved),
            DOMAIN_GAP_WITNESS_PROVENANCE_UNOBSERVED => Ok(Self::Unobserved),
            other => Err(DomainGapError::InvalidSemantics(format!(
                "DOMAIN_GAP witness provenance {other} is unknown"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
pub enum DomainGapPart {
    State = DOMAIN_GAP_PART_STATE,
    Event = DOMAIN_GAP_PART_EVENT,
    Both = DOMAIN_GAP_PART_BOTH,
}

impl DomainGapPart {
    pub const ALL: [DomainGapPart; 3] = [Self::State, Self::Event, Self::Both];

    pub const fn code(self) -> u16 {
        self as u16
    }

    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Event => "event",
            Self::Both => "both",
        }
    }

    pub fn from_wire_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|part| part.wire_name() == name)
    }

    fn from_code(code: u16) -> Result<Self, DomainGapError> {
        match code {
            DOMAIN_GAP_PART_STATE => Ok(Self::State),
            DOMAIN_GAP_PART_EVENT => Ok(Self::Event),
            DOMAIN_GAP_PART_BOTH => Ok(Self::Both),
            other => Err(DomainGapError::InvalidSemantics(format!(
                "DOMAIN_GAP part {other} is unknown"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainGapRecordV1 {
    pub part: DomainGapPart,
    pub reason: DomainGapReason,
    pub target_hour_raw: i32,
    pub range_start_hour_raw: i32,
    pub range_end_hour_raw: i32,
    pub tick_before: u64,
    pub tick_after: u64,
    pub queue_capacity: u32,
    pub queue_depth: u32,
    pub queue_dropped: u64,
    pub outcome_ordinal: u64,
}

impl DomainGapRecordV1 {
    pub fn encode(&self) -> Result<Vec<u8>, DomainGapError> {
        self.encode_with_witness_provenance(DomainGapWitnessProvenance::Unstated)
    }

    pub fn encode_with_witness_provenance(
        &self,
        witness_provenance: DomainGapWitnessProvenance,
    ) -> Result<Vec<u8>, DomainGapError> {
        validate_hours(self)?;
        let mut out = vec![0u8; COMPACT_DOMAIN_GAP_PAYLOAD_LEN];
        out[..4].copy_from_slice(COMPACT_DOMAIN_GAP_MAGIC);
        out[4..6].copy_from_slice(&COMPACT_DOMAIN_GAP_VERSION.to_le_bytes());
        out[6..8].copy_from_slice(&(COMPACT_DOMAIN_GAP_PAYLOAD_LEN as u16).to_le_bytes());
        out[8..10].copy_from_slice(&self.reason.code().to_le_bytes());
        out[10..12].copy_from_slice(&self.part.code().to_le_bytes());
        out[12..14].copy_from_slice(&witness_provenance.code().to_le_bytes());
        out[16..20].copy_from_slice(&self.target_hour_raw.to_le_bytes());
        out[20..24].copy_from_slice(&self.range_start_hour_raw.to_le_bytes());
        out[24..28].copy_from_slice(&self.range_end_hour_raw.to_le_bytes());
        out[32..40].copy_from_slice(&self.tick_before.to_le_bytes());
        out[40..48].copy_from_slice(&self.tick_after.to_le_bytes());
        out[48..52].copy_from_slice(&self.queue_capacity.to_le_bytes());
        out[52..56].copy_from_slice(&self.queue_depth.to_le_bytes());
        out[56..64].copy_from_slice(&self.queue_dropped.to_le_bytes());
        out[64..72].copy_from_slice(&self.outcome_ordinal.to_le_bytes());
        validate_domain_gap_payload(&out).map_err(DomainGapError::InvalidSemantics)?;
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DomainGapError> {
        if bytes.len() != COMPACT_DOMAIN_GAP_PAYLOAD_LEN {
            return Err(DomainGapError::WrongLength {
                expected: COMPACT_DOMAIN_GAP_PAYLOAD_LEN,
                actual: bytes.len(),
            });
        }
        validate_domain_gap_payload(bytes).map_err(DomainGapError::InvalidSemantics)?;
        let value = Self {
            reason: DomainGapReason::from_code(read_u16(bytes, 8))?,
            part: DomainGapPart::from_code(read_u16(bytes, 10))?,
            target_hour_raw: read_i32(bytes, 16),
            range_start_hour_raw: read_i32(bytes, 20),
            range_end_hour_raw: read_i32(bytes, 24),
            tick_before: read_u64(bytes, 32),
            tick_after: read_u64(bytes, 40),
            queue_capacity: read_u32(bytes, 48),
            queue_depth: read_u32(bytes, 52),
            queue_dropped: read_u64(bytes, 56),
            outcome_ordinal: read_u64(bytes, 64),
        };
        validate_hours(&value)?;
        Ok(value)
    }

    pub fn decode_witness_provenance(
        bytes: &[u8],
    ) -> Result<DomainGapWitnessProvenance, DomainGapError> {
        if bytes.len() != COMPACT_DOMAIN_GAP_PAYLOAD_LEN {
            return Err(DomainGapError::WrongLength {
                expected: COMPACT_DOMAIN_GAP_PAYLOAD_LEN,
                actual: bytes.len(),
            });
        }
        validate_domain_gap_payload(bytes).map_err(DomainGapError::InvalidSemantics)?;
        DomainGapWitnessProvenance::from_code(read_u16(bytes, 12))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainGapError {
    WrongLength { expected: usize, actual: usize },
    InvalidGameHour(i32),
    InvalidSemantics(String),
}

impl core::fmt::Display for DomainGapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongLength { expected, actual } => {
                write!(f, "H4DG payload length {actual} != expected {expected}")
            }
            Self::InvalidGameHour(hour) => write!(f, "H4DG game hour {hour} is invalid"),
            Self::InvalidSemantics(error) => write!(f, "invalid H4DG semantics: {error}"),
        }
    }
}

impl std::error::Error for DomainGapError {}

fn validate_hours(value: &DomainGapRecordV1) -> Result<(), DomainGapError> {
    for hour in [
        value.target_hour_raw,
        value.range_start_hour_raw,
        value.range_end_hour_raw,
    ] {
        if hour_index_from_raw(hour).is_none() {
            return Err(DomainGapError::InvalidGameHour(hour));
        }
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("checked H4DG"))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("checked H4DG"))
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("checked H4DG"))
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("checked H4DG"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::DATE_EPOCH_CONST;

    fn hour(offset: i32) -> i32 {
        DATE_EPOCH_CONST + 1944 * 365 * 24 + offset
    }

    fn sample(reason: DomainGapReason) -> DomainGapRecordV1 {
        let (range_start, range_end, tick_before, tick_after, capacity, depth, dropped) =
            match reason {
                DomainGapReason::Torn => (hour(0), hour(0), 100, 101, 0, 0, 0),
                DomainGapReason::Overrun => (hour(-2), hour(0), 100, 103, 0, 0, 0),
                DomainGapReason::Backpressure => (hour(-1), hour(0), 103, 103, 8, 8, 2),
                DomainGapReason::ClockUnreadable => (hour(0), hour(0), 0, 0, 0, 0, 0),
                DomainGapReason::WorkerPanic => (hour(0), hour(0), 103, 103, 0, 0, 0),
                DomainGapReason::SourceUnavailable => (hour(0), hour(0), 103, 103, 0, 0, 0),
                DomainGapReason::ReaderSilent => (hour(0), hour(0), 0, 0, 0, 0, 0),
                DomainGapReason::HourLabelMismatch => (
                    hour(0),
                    hour(0),
                    u64::try_from(hour(1)).expect("sample hour fits u64"),
                    u64::try_from(hour(1)).expect("sample hour fits u64"),
                    0,
                    0,
                    0,
                ),
            };
        DomainGapRecordV1 {
            part: DomainGapPart::State,
            reason,
            target_hour_raw: hour(0),
            range_start_hour_raw: range_start,
            range_end_hour_raw: range_end,
            tick_before,
            tick_after,
            queue_capacity: capacity,
            queue_depth: depth,
            queue_dropped: dropped,
            outcome_ordinal: 17,
        }
    }

    #[test]
    fn all_covers_every_decodable_reason() {
        let decodable: Vec<DomainGapReason> = (0..=u16::MAX)
            .filter_map(|code| DomainGapReason::from_code(code).ok())
            .collect();
        assert_eq!(
            decodable.len(),
            DomainGapReason::ALL.len(),
            "the wire decodes {} reason(s) but ALL lists {}: {decodable:?}",
            decodable.len(),
            DomainGapReason::ALL.len()
        );
        for reason in decodable {
            assert!(
                DomainGapReason::ALL.contains(&reason),
                "{reason:?} can be decoded from the wire but is absent from ALL, so every \
                 consumer that iterates ALL would silently drop it"
            );
        }
    }

    #[test]
    fn all_is_exhaustive() {
        for (slot, reason) in DomainGapReason::ALL.iter().enumerate() {
            assert_eq!(
                reason.position(),
                slot,
                "{reason:?} is not in its declared position in ALL"
            );
        }
    }

    #[test]
    fn every_closed_reason_round_trips_with_exact_layout() {
        for reason in DomainGapReason::ALL {
            let value = sample(reason);
            let wire = value.encode().unwrap();
            assert_eq!(wire.len(), COMPACT_DOMAIN_GAP_PAYLOAD_LEN);
            assert_eq!(&wire[..4], COMPACT_DOMAIN_GAP_MAGIC);
            assert_eq!(read_u16(&wire, 4), COMPACT_DOMAIN_GAP_VERSION);
            assert_eq!(read_u16(&wire, 6) as usize, wire.len());
            assert_eq!(read_u16(&wire, 8), reason.code());
            assert_eq!(DomainGapRecordV1::decode(&wire).unwrap(), value);
        }
    }

    #[test]
    fn the_payload_grammar_admits_exactly_the_codes_from_code_decodes() {
        let base = sample(DomainGapReason::Torn).encode().unwrap();
        for code in 0..=u16::MAX {
            let mut wire = base.clone();
            wire[8..10].copy_from_slice(&code.to_le_bytes());
            let grammar_calls_it_unknown = validate_domain_gap_payload(&wire)
                .err()
                .is_some_and(|error| error == format!("DOMAIN_GAP reason {code} is unknown"));
            assert_eq!(
                grammar_calls_it_unknown,
                DomainGapReason::from_code(code).is_err(),
                "reason {code} is admitted by one vocabulary and refused by the \
                 other"
            );
        }
    }

    #[test]
    fn structural_and_semantic_mutants_fail_closed() {
        let wire = sample(DomainGapReason::Torn).encode().unwrap();
        let mut altered = wire.clone();
        altered[8..10].copy_from_slice(&99u16.to_le_bytes());
        assert!(matches!(
            DomainGapRecordV1::decode(&altered),
            Err(DomainGapError::InvalidSemantics(_))
        ));
    }

    #[test]
    fn reason_specific_witness_mutants_fail_closed() {
        let mutations: [(DomainGapReason, fn(&mut DomainGapRecordV1)); 5] = [
            (DomainGapReason::Torn, |value| {
                value.tick_after = value.tick_before
            }),
            (DomainGapReason::Overrun, |value| {
                value.tick_after = value.tick_before
            }),
            (DomainGapReason::Backpressure, |value| {
                value.queue_depth -= 1
            }),
            (DomainGapReason::ClockUnreadable, |value| {
                value.tick_before = 1
            }),
            (DomainGapReason::WorkerPanic, |value| {
                value.range_start_hour_raw -= 1
            }),
        ];
        for (reason, mutate) in mutations {
            let mut value = sample(reason);
            mutate(&mut value);
            assert!(
                matches!(value.encode(), Err(DomainGapError::InvalidSemantics(_))),
                "{reason:?}"
            );
        }
    }

    #[test]
    fn multi_hour_witnesses_bind_the_exact_inclusive_width() {
        let mut overrun = sample(DomainGapReason::Overrun);
        overrun.tick_after += 1;
        assert!(matches!(
            overrun.encode(),
            Err(DomainGapError::InvalidSemantics(_))
        ));

        let mut backpressure = sample(DomainGapReason::Backpressure);
        backpressure.queue_dropped += 1;
        assert!(matches!(
            backpressure.encode(),
            Err(DomainGapError::InvalidSemantics(_))
        ));
    }

    #[test]
    fn range_queue_and_calendar_mutants_fail_closed() {
        let mut outside = sample(DomainGapReason::Backpressure);
        outside.target_hour_raw = hour(1);
        assert!(matches!(
            outside.encode(),
            Err(DomainGapError::InvalidSemantics(_))
        ));

        let mut queue = sample(DomainGapReason::Backpressure);
        queue.queue_depth = queue.queue_capacity + 1;
        assert!(matches!(
            queue.encode(),
            Err(DomainGapError::InvalidSemantics(_))
        ));

        let mut exhausted = sample(DomainGapReason::WorkerPanic);
        exhausted.outcome_ordinal = u64::MAX;
        assert!(matches!(
            exhausted.encode(),
            Err(DomainGapError::InvalidSemantics(_))
        ));

        let mut calendar = sample(DomainGapReason::WorkerPanic);
        calendar.target_hour_raw = 0;
        calendar.range_start_hour_raw = 0;
        calendar.range_end_hour_raw = 0;
        assert_eq!(calendar.encode(), Err(DomainGapError::InvalidGameHour(0)));
    }

    #[test]
    fn the_provenance_vocabulary_is_complete() {
        let decodable: Vec<DomainGapWitnessProvenance> = (0..=u16::MAX)
            .filter_map(|code| DomainGapWitnessProvenance::from_code(code).ok())
            .collect();
        assert_eq!(
            decodable.len(),
            DomainGapWitnessProvenance::ALL.len(),
            "the wire decodes {decodable:?} but ALL lists {:?}",
            DomainGapWitnessProvenance::ALL
        );
        for provenance in decodable {
            assert!(DomainGapWitnessProvenance::ALL.contains(&provenance));
        }
    }

    #[test]
    fn the_grammar_admits_exactly_the_provenance_codes_from_code_decodes() {
        let base = sample(DomainGapReason::Torn).encode().unwrap();
        for code in 0..=u16::MAX {
            let mut wire = base.clone();
            wire[12..14].copy_from_slice(&code.to_le_bytes());
            let grammar_calls_it_unknown =
                validate_domain_gap_payload(&wire)
                    .err()
                    .is_some_and(|error| {
                        error == format!("DOMAIN_GAP witness provenance {code} is unknown")
                    });
            assert_eq!(
                grammar_calls_it_unknown,
                DomainGapWitnessProvenance::from_code(code).is_err(),
                "provenance {code} is admitted by one vocabulary and refused by \
                 the other"
            );
        }
    }

    #[test]
    fn provenance_round_trips_and_unstated_is_the_legacy_encoding() {
        for reason in DomainGapReason::ALL {
            let value = sample(reason);
            let plain = value.encode().unwrap();
            let unstated = value
                .encode_with_witness_provenance(DomainGapWitnessProvenance::Unstated)
                .unwrap();
            assert_eq!(plain, unstated, "{reason:?}");
            assert_eq!(read_u16(&plain, 12), 0, "{reason:?}");
            assert_eq!(
                DomainGapRecordV1::decode_witness_provenance(&plain).unwrap(),
                DomainGapWitnessProvenance::Unstated,
                "{reason:?}"
            );
            assert!(
                !DomainGapWitnessProvenance::Unstated.is_a_bracket_observation(),
                "an unconverted producer's ticks may never read as an observation"
            );

            let supported = if value.tick_before == 0 && value.tick_after == 0 {
                DomainGapWitnessProvenance::Unobserved
            } else {
                DomainGapWitnessProvenance::BracketObserved
            };
            let stamped = value.encode_with_witness_provenance(supported).unwrap();
            assert_eq!(
                DomainGapRecordV1::decode_witness_provenance(&stamped).unwrap(),
                supported,
                "{reason:?}"
            );
            assert_eq!(DomainGapRecordV1::decode(&stamped).unwrap(), value);
        }
    }

    #[test]
    fn a_provenance_that_contradicts_its_witnesses_fails_closed() {
        let observed_on_zero = sample(DomainGapReason::ReaderSilent)
            .encode_with_witness_provenance(DomainGapWitnessProvenance::BracketObserved);
        assert!(
            matches!(observed_on_zero, Err(DomainGapError::InvalidSemantics(_))),
            "zero is the wire's no-witness value; it cannot be an observation"
        );

        let unobserved_on_reading = sample(DomainGapReason::Torn)
            .encode_with_witness_provenance(DomainGapWitnessProvenance::Unobserved);
        assert!(
            matches!(
                unobserved_on_reading,
                Err(DomainGapError::InvalidSemantics(_))
            ),
            "a record carrying a real reading cannot claim it observed nothing"
        );

        let mut forged = sample(DomainGapReason::ReaderSilent).encode().unwrap();
        forged[12..14].copy_from_slice(
            &DomainGapWitnessProvenance::BracketObserved
                .code()
                .to_le_bytes(),
        );
        assert!(matches!(
            DomainGapRecordV1::decode_witness_provenance(&forged),
            Err(DomainGapError::InvalidSemantics(_))
        ));
        assert!(matches!(
            DomainGapRecordV1::decode(&forged),
            Err(DomainGapError::InvalidSemantics(_))
        ));
    }

    #[test]
    fn the_unspent_half_of_the_reserved_word_is_still_refused() {
        let mut wire = sample(DomainGapReason::Torn).encode().unwrap();
        wire[14..16].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            validate_domain_gap_payload(&wire),
            Err("DOMAIN_GAP reserved field is nonzero".to_string())
        );
    }

    #[test]
    fn queue_witness_is_exactly_zero_outside_backpressure() {
        for reason in DomainGapReason::ALL {
            let carries_queue_witness = match reason {
                DomainGapReason::Backpressure => true,
                DomainGapReason::Torn
                | DomainGapReason::Overrun
                | DomainGapReason::ClockUnreadable
                | DomainGapReason::WorkerPanic
                | DomainGapReason::SourceUnavailable
                | DomainGapReason::ReaderSilent
                | DomainGapReason::HourLabelMismatch => false,
            };
            if carries_queue_witness {
                continue;
            }
            let mut value = sample(reason);
            value.queue_capacity = 1;
            assert!(
                matches!(value.encode(), Err(DomainGapError::InvalidSemantics(_))),
                "{reason:?}"
            );
        }
    }
}
