//! Ordinary H4CR acquisition-gap control record.
//!
//! H4DG consumes the normal H4CR sequence under the affected domain id and is
//! that scheduled domain's terminal outcome, but is neither a DATA nor a PERF
//! record. The enclosing
//! retained H4CR stream provides sequence, campaign, session, affected-domain,
//! and product-digest authentication without retaining a per-attach secret in
//! the rebuilt product.

use crate::calendar::hour_index_from_raw;
use crate::record_semantics::{
    validate_domain_gap_payload, COMPACT_DOMAIN_GAP_MAGIC, COMPACT_DOMAIN_GAP_PAYLOAD_LEN,
    COMPACT_DOMAIN_GAP_VERSION, DOMAIN_GAP_PART_BOTH, DOMAIN_GAP_PART_EVENT, DOMAIN_GAP_PART_STATE,
    DOMAIN_GAP_REASON_BACKPRESSURE, DOMAIN_GAP_REASON_CLOCK_UNREADABLE, DOMAIN_GAP_REASON_OVERRUN,
    DOMAIN_GAP_REASON_TORN, DOMAIN_GAP_REASON_WORKER_PANIC,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum DomainGapReason {
    Torn = DOMAIN_GAP_REASON_TORN,
    Overrun = DOMAIN_GAP_REASON_OVERRUN,
    Backpressure = DOMAIN_GAP_REASON_BACKPRESSURE,
    ClockUnreadable = DOMAIN_GAP_REASON_CLOCK_UNREADABLE,
    WorkerPanic = DOMAIN_GAP_REASON_WORKER_PANIC,
}

impl DomainGapReason {
    pub const fn code(self) -> u16 {
        self as u16
    }

    fn from_code(code: u16) -> Result<Self, DomainGapError> {
        match code {
            DOMAIN_GAP_REASON_TORN => Ok(Self::Torn),
            DOMAIN_GAP_REASON_OVERRUN => Ok(Self::Overrun),
            DOMAIN_GAP_REASON_BACKPRESSURE => Ok(Self::Backpressure),
            DOMAIN_GAP_REASON_CLOCK_UNREADABLE => Ok(Self::ClockUnreadable),
            DOMAIN_GAP_REASON_WORKER_PANIC => Ok(Self::WorkerPanic),
            other => Err(DomainGapError::InvalidSemantics(format!(
                "DOMAIN_GAP reason {other} is unknown"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum DomainGapPart {
    State = DOMAIN_GAP_PART_STATE,
    Event = DOMAIN_GAP_PART_EVENT,
    Both = DOMAIN_GAP_PART_BOTH,
}

impl DomainGapPart {
    pub const fn code(self) -> u16 {
        self as u16
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
    /// Exact requested hour that produced this outcome.
    pub target_hour_raw: i32,
    /// Inclusive affected target range.
    pub range_start_hour_raw: i32,
    pub range_end_hour_raw: i32,
    /// Read-only engine-clock witnesses. Closed reason semantics determine
    /// whether equality, inequality, ordering, or explicit absence is required.
    pub tick_before: u64,
    pub tick_after: u64,
    /// Bounded scheduler queue witness sampled at outcome publication.
    pub queue_capacity: u32,
    pub queue_depth: u32,
    pub queue_dropped: u64,
    /// Zero-based ordinal of this emitted scheduler-outcome record within its
    /// `(domain_id, part)` lane. A coalesced range is one outcome record.
    pub outcome_ordinal: u64,
}

impl DomainGapRecordV1 {
    pub fn encode(&self) -> Result<Vec<u8>, DomainGapError> {
        validate_hours(self)?;
        let mut out = vec![0u8; COMPACT_DOMAIN_GAP_PAYLOAD_LEN];
        out[..4].copy_from_slice(COMPACT_DOMAIN_GAP_MAGIC);
        out[4..6].copy_from_slice(&COMPACT_DOMAIN_GAP_VERSION.to_le_bytes());
        out[6..8].copy_from_slice(&(COMPACT_DOMAIN_GAP_PAYLOAD_LEN as u16).to_le_bytes());
        out[8..10].copy_from_slice(&self.reason.code().to_le_bytes());
        out[10..12].copy_from_slice(&self.part.code().to_le_bytes());
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
    fn every_closed_reason_round_trips_with_exact_layout() {
        for reason in [
            DomainGapReason::Torn,
            DomainGapReason::Overrun,
            DomainGapReason::Backpressure,
            DomainGapReason::ClockUnreadable,
            DomainGapReason::WorkerPanic,
        ] {
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
    fn queue_witness_is_exactly_zero_outside_backpressure() {
        for reason in [
            DomainGapReason::Torn,
            DomainGapReason::Overrun,
            DomainGapReason::ClockUnreadable,
            DomainGapReason::WorkerPanic,
        ] {
            let mut value = sample(reason);
            value.queue_capacity = 1;
            assert!(
                matches!(value.encode(), Err(DomainGapError::InvalidSemantics(_))),
                "{reason:?}"
            );
        }
    }
}
