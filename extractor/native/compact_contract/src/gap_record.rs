//! Canonical typed recording-gap contract shared by producers and consumers.
//!
//! A gap describes trustworthy evidence that some game-state bytes were not
//! observed. It must never be used to excuse malformed, misordered, or otherwise
//! untrustworthy bytes; those remain stream corruption in the consumer.

use crate::{
    CompactDomainId, COMPACT_STATUS_BRIDGE_UNAVAILABLE, COMPACT_STATUS_PARTIAL_DATA,
    COMPACT_STATUS_SOURCE_UNAVAILABLE, COMPACT_STATUS_WORKER_PANIC,
};
use core::fmt;

pub const GAP_RECORD_MAGIC: [u8; 4] = *b"H4GP";
pub const GAP_RECORD_LEGACY_VERSION: u16 = 1;
pub const GAP_RECORD_VERSION: u16 = 2;
pub const MAX_GAP_DOMAIN_COUNT: usize = u16::MAX as usize;
const GAP_RECORD_HEADER_LEN: usize = 64;
const GAP_STATUS_MASK: u32 = COMPACT_STATUS_SOURCE_UNAVAILABLE
    | COMPACT_STATUS_PARTIAL_DATA
    | COMPACT_STATUS_BRIDGE_UNAVAILABLE
    | COMPACT_STATUS_WORKER_PANIC;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GapReason {
    Coalesced,
    SkippedHour,
    SourceUnavailable,
    PartialData,
    BridgeUnavailable,
    WorkerPanic,
    TornSweep,
    CoverageShortfall,
    Backpressure,
}

pub const GAP_REASON_CODE_COALESCED: u8 = 1;
pub const GAP_REASON_CODE_SKIPPED_HOUR: u8 = 2;
pub const GAP_REASON_CODE_SOURCE_UNAVAILABLE: u8 = 3;
pub const GAP_REASON_CODE_PARTIAL_DATA: u8 = 4;
pub const GAP_REASON_CODE_BRIDGE_UNAVAILABLE: u8 = 5;
pub const GAP_REASON_CODE_WORKER_PANIC: u8 = 6;
pub const GAP_REASON_CODE_TORN_SWEEP: u8 = 7;
pub const GAP_REASON_CODE_COVERAGE_SHORTFALL: u8 = 8;
pub const GAP_REASON_CODE_BACKPRESSURE: u8 = 9;

impl GapReason {
    /// Every reason, for consumers that must handle all of them but cannot be
    /// forced to by a `match`: the manifest ledger's name table and its
    /// expander, and the round-trip test that pins the two together.
    ///
    /// `gap_reason_all_covers_every_decodable_reason` rebuilds this set from
    /// `gap_reason_from_code` over the whole `u8` code space, so a variant that
    /// can arrive off the wire and is missing here fails the suite.
    pub const ALL: [GapReason; 9] = [
        Self::Coalesced,
        Self::SkippedHour,
        Self::SourceUnavailable,
        Self::PartialData,
        Self::BridgeUnavailable,
        Self::WorkerPanic,
        Self::TornSweep,
        Self::CoverageShortfall,
        Self::Backpressure,
    ];

    /// The reason's name in the sealed manifest.
    ///
    /// THIS USED TO BE A MATCH INSIDE THE MANIFEST WRITER. The compacted gap
    /// ledger has to expand names back into reasons, which would have made two
    /// transcriptions of the same nine strings -- exactly the drift lane FJ
    /// removed for `DomainGapReason`. One authority, both directions.
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Coalesced => "coalesced",
            Self::SkippedHour => "skipped_hour",
            Self::SourceUnavailable => "source_unavailable",
            Self::PartialData => "partial_data",
            Self::BridgeUnavailable => "bridge_unavailable",
            Self::WorkerPanic => "worker_panic",
            Self::TornSweep => "torn_sweep",
            Self::CoverageShortfall => "coverage_shortfall",
            Self::Backpressure => "backpressure",
        }
    }

    /// The inverse of [`GapReason::wire_name`].
    pub fn from_wire_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reason| reason.wire_name() == name)
    }

    pub const fn code(self) -> u8 {
        match self {
            Self::Coalesced => GAP_REASON_CODE_COALESCED,
            Self::SkippedHour => GAP_REASON_CODE_SKIPPED_HOUR,
            Self::SourceUnavailable => GAP_REASON_CODE_SOURCE_UNAVAILABLE,
            Self::PartialData => GAP_REASON_CODE_PARTIAL_DATA,
            Self::BridgeUnavailable => GAP_REASON_CODE_BRIDGE_UNAVAILABLE,
            Self::WorkerPanic => GAP_REASON_CODE_WORKER_PANIC,
            Self::TornSweep => GAP_REASON_CODE_TORN_SWEEP,
            Self::CoverageShortfall => GAP_REASON_CODE_COVERAGE_SHORTFALL,
            Self::Backpressure => GAP_REASON_CODE_BACKPRESSURE,
        }
    }

    /// Return the deterministic dominant reason for a status-bit observation.
    ///
    /// The witness retains every observed gap-status bit; the reason is only
    /// the canonical index used to classify that complete observation.
    pub const fn canonical_for_status_bits(status_bits: u32) -> Option<Self> {
        if status_bits == 0 || status_bits & !GAP_STATUS_MASK != 0 {
            None
        } else if status_bits & COMPACT_STATUS_WORKER_PANIC != 0 {
            Some(Self::WorkerPanic)
        } else if status_bits & COMPACT_STATUS_BRIDGE_UNAVAILABLE != 0 {
            Some(Self::BridgeUnavailable)
        } else if status_bits & COMPACT_STATUS_SOURCE_UNAVAILABLE != 0 {
            Some(Self::SourceUnavailable)
        } else if status_bits & COMPACT_STATUS_PARTIAL_DATA != 0 {
            Some(Self::PartialData)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GapCounter {
    CoalescedHours,
    QueueOverrunHours,
    SinkBackpressureHours,
}

pub const GAP_COUNTER_CODE_COALESCED_HOURS: u8 = 1;
pub const GAP_COUNTER_CODE_QUEUE_OVERRUN_HOURS: u8 = 2;
pub const GAP_COUNTER_CODE_SINK_BACKPRESSURE_HOURS: u8 = 3;

impl GapCounter {
    /// Every counter, for the manifest name table and its inverse.
    pub const ALL: [GapCounter; 3] = [
        Self::CoalescedHours,
        Self::QueueOverrunHours,
        Self::SinkBackpressureHours,
    ];

    pub const fn code(self) -> u8 {
        match self {
            Self::CoalescedHours => GAP_COUNTER_CODE_COALESCED_HOURS,
            Self::QueueOverrunHours => GAP_COUNTER_CODE_QUEUE_OVERRUN_HOURS,
            Self::SinkBackpressureHours => GAP_COUNTER_CODE_SINK_BACKPRESSURE_HOURS,
        }
    }

    /// The counter's name in the sealed manifest. Moved here from the manifest
    /// writer for the same reason as [`GapReason::wire_name`]: the compacted
    /// ledger has to read it back.
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::CoalescedHours => "coalesced_hours",
            Self::QueueOverrunHours => "queue_overrun_hours",
            Self::SinkBackpressureHours => "sink_backpressure_hours",
        }
    }

    /// The inverse of [`GapCounter::wire_name`].
    pub fn from_wire_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|counter| counter.wire_name() == name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GapWitness {
    Counter {
        counter: GapCounter,
        observed: u64,
    },
    HourDiscontinuity {
        previous_hour: i32,
        next_hour: i32,
    },
    StatusBits {
        status_bits: u32,
    },
    Coverage {
        expected_perf_records: u32,
        observed_perf_records: u32,
        expected_data_records: u32,
        observed_data_records: u32,
    },
    CoverageAndStatus {
        expected_perf_records: u32,
        observed_perf_records: u32,
        expected_data_records: u32,
        observed_data_records: u32,
        status_bits: u32,
    },
    TornSweep {
        tick_start: u64,
        tick_end: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GapRecord {
    /// First missing game hour, inclusive.
    pub hour_start: i32,
    /// Last missing game hour, inclusive. Equal to `hour_start` for one hour.
    pub hour_end: i32,
    pub reason: GapReason,
    /// Affected compact-domain ids. Empty means every domain.
    pub domains: Vec<CompactDomainId>,
    pub witness: GapWitness,
}

impl GapRecord {
    /// Build the single canonical gap for a forward FRAME_END jump.
    ///
    /// The logical skipped interval is half-open
    /// `[previous_hour + 1, next_hour)`. H4GP v2 retains inclusive endpoints,
    /// so the wire stores `hour_end = next_hour - 1`; it must not be expanded
    /// into one record per skipped hour. The `HourDiscontinuity` wire witness is
    /// the v2 spelling of the TickDiscontinuity authority.
    pub fn skipped_tick_discontinuity_half_open(
        previous_hour: i32,
        next_hour: i32,
        domains: Vec<CompactDomainId>,
    ) -> Result<Self, GapRecordValidationError> {
        let hour_start = previous_hour.checked_add(1).ok_or(
            GapRecordValidationError::InvalidInclusiveRange {
                hour_start: previous_hour,
                hour_end: next_hour,
            },
        )?;
        let hour_end =
            next_hour
                .checked_sub(1)
                .ok_or(GapRecordValidationError::InvalidInclusiveRange {
                    hour_start,
                    hour_end: next_hour,
                })?;
        let record = Self {
            hour_start,
            hour_end,
            reason: GapReason::SkippedHour,
            domains,
            witness: GapWitness::HourDiscontinuity {
                previous_hour,
                next_hour,
            },
        };
        record.validate()?;
        Ok(record)
    }

    pub fn inclusive_hour_count(&self) -> Option<u64> {
        let width = i64::from(self.hour_end)
            .checked_sub(i64::from(self.hour_start))?
            .checked_add(1)?;
        u64::try_from(width).ok()
    }

    pub fn affects_domain(&self, domain_id: CompactDomainId) -> bool {
        self.domains.is_empty() || self.domains.contains(&domain_id)
    }

    pub fn covers(&self, hour: i32, domain_id: CompactDomainId) -> bool {
        self.hour_start <= hour && hour <= self.hour_end && self.affects_domain(domain_id)
    }

    pub fn validate(&self) -> Result<(), GapRecordValidationError> {
        let hour_count =
            self.inclusive_hour_count()
                .ok_or(GapRecordValidationError::InvalidInclusiveRange {
                    hour_start: self.hour_start,
                    hour_end: self.hour_end,
                })?;
        // `inclusive_hour_count` only fails on i32/i64 overflow, and a width of
        // exactly zero (`hour_end == hour_start - 1`) survives it as `Some(0)`.
        // A gap that covers no hour is not a gap: `covers()` can never match it,
        // it carries a reason code for an outcome that never happened, and the
        // half-open forward-jump constructor relies on this check to refuse a
        // one-hour step, which is a continuous stream rather than a jump.
        if hour_count == 0 {
            return Err(GapRecordValidationError::InvalidInclusiveRange {
                hour_start: self.hour_start,
                hour_end: self.hour_end,
            });
        }
        if self.domains.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(GapRecordValidationError::DomainsNotStrictlyIncreasing);
        }
        if self.domains.first() == Some(&0) {
            return Err(GapRecordValidationError::InvalidDomainId(0));
        }
        match (self.reason, &self.witness) {
            (
                GapReason::Coalesced,
                GapWitness::Counter {
                    counter: GapCounter::CoalescedHours,
                    observed,
                },
            ) if *observed == hour_count => Ok(()),
            (
                GapReason::Backpressure,
                GapWitness::Counter {
                    counter: GapCounter::QueueOverrunHours | GapCounter::SinkBackpressureHours,
                    observed,
                },
            ) if *observed == hour_count => Ok(()),
            (
                GapReason::SkippedHour,
                GapWitness::HourDiscontinuity {
                    previous_hour,
                    next_hour,
                },
            ) if previous_hour.checked_add(1) == Some(self.hour_start)
                && self.hour_end.checked_add(1) == Some(*next_hour) =>
            {
                Ok(())
            }
            (reason, GapWitness::StatusBits { status_bits })
                if GapReason::canonical_for_status_bits(*status_bits) == Some(reason) =>
            {
                Ok(())
            }
            (
                GapReason::TornSweep,
                GapWitness::TornSweep {
                    tick_start,
                    tick_end,
                },
            ) if tick_start != tick_end => Ok(()),
            (
                GapReason::CoverageShortfall,
                GapWitness::Coverage {
                    expected_perf_records,
                    observed_perf_records,
                    expected_data_records,
                    observed_data_records,
                },
            ) if observed_perf_records <= expected_perf_records
                && observed_data_records <= expected_data_records
                && (observed_perf_records < expected_perf_records
                    || observed_data_records < expected_data_records) =>
            {
                Ok(())
            }
            (
                reason,
                GapWitness::CoverageAndStatus {
                    expected_perf_records,
                    observed_perf_records,
                    expected_data_records,
                    observed_data_records,
                    status_bits,
                },
            ) if observed_perf_records <= expected_perf_records
                && observed_data_records <= expected_data_records
                && (observed_perf_records < expected_perf_records
                    || observed_data_records < expected_data_records)
                && GapReason::canonical_for_status_bits(*status_bits) == Some(reason) =>
            {
                Ok(())
            }
            _ => Err(GapRecordValidationError::WitnessDoesNotProveReason(
                self.reason,
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GapRecordValidationError {
    InvalidObservedRange {
        hour_start: i32,
        hour_end: i32,
    },
    InvalidInclusiveRange {
        hour_start: i32,
        hour_end: i32,
    },
    OutsideObservedRange {
        gap_start: i32,
        gap_end: i32,
        observed_start: i32,
        observed_end: i32,
    },
    DomainsNotStrictlyIncreasing,
    InvalidDomainId(CompactDomainId),
    WitnessDoesNotProveReason(GapReason),
    OverlapForDomain {
        domain: Option<CompactDomainId>,
        first_start: i32,
        first_end: i32,
        second_start: i32,
        second_end: i32,
    },
    /// [`validate_gap_records_hour_ordered`] was handed a record starting
    /// before the one it had already accepted.
    ///
    /// A separate outcome rather than a silent re-sort: the sweep answers the
    /// overlap question by keeping only the records still open at the current
    /// hour, so out-of-order input would let it MISS an overlap. A missed
    /// overlap reads as a valid ledger, which is the one answer this must never
    /// invent.
    NotHourOrdered {
        previous_start: i32,
        next_start: i32,
    },
}

impl fmt::Display for GapRecordValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid gap record: {self:?}")
    }
}

impl std::error::Error for GapRecordValidationError {}

/// Validate a complete gap ledger against the run's inclusive observed range.
///
/// Empty `domains` means all domains, so a global record overlaps every
/// domain-specific record whose hour range intersects it.
pub fn validate_gap_records(
    records: &[GapRecord],
    observed_hour_start: i32,
    observed_hour_end: i32,
) -> Result<(), GapRecordValidationError> {
    if observed_hour_start > observed_hour_end {
        return Err(GapRecordValidationError::InvalidObservedRange {
            hour_start: observed_hour_start,
            hour_end: observed_hour_end,
        });
    }
    for record in records {
        record.validate()?;
        if record.hour_start < observed_hour_start || record.hour_end > observed_hour_end {
            return Err(GapRecordValidationError::OutsideObservedRange {
                gap_start: record.hour_start,
                gap_end: record.hour_end,
                observed_start: observed_hour_start,
                observed_end: observed_hour_end,
            });
        }
    }
    for (index, first) in records.iter().enumerate() {
        for second in &records[index + 1..] {
            if first.hour_end < second.hour_start || second.hour_end < first.hour_start {
                continue;
            }
            let overlap_domain = if first.domains.is_empty() || second.domains.is_empty() {
                Some(None)
            } else {
                first
                    .domains
                    .iter()
                    .find(|domain| second.domains.binary_search(domain).is_ok())
                    .copied()
                    .map(Some)
            };
            if let Some(domain) = overlap_domain {
                return Err(GapRecordValidationError::OverlapForDomain {
                    domain,
                    first_start: first.hour_start,
                    first_end: first.hour_end,
                    second_start: second.hour_start,
                    second_end: second.hour_end,
                });
            }
        }
    }
    Ok(())
}

/// The same rule as [`validate_gap_records`], answered by a sweep over records
/// delivered in non-decreasing `hour_start` order.
///
/// # Why a second entry point
///
/// [`validate_gap_records`] takes a slice and compares every pair, so it costs
/// O(n^2) time and needs every row resident at once. A recorder ledger is one
/// row per (game hour, domain) absence — 1.7 million rows at 68,300 game hours —
/// and neither the time nor the residency survives that. This form holds only
/// the records still OPEN at the hour it is on: sorted by `hour_start`, any
/// earlier record that could still overlap the current one has not ended yet, so
/// the open set is exactly the candidate set and everything before it can be
/// dropped.
///
/// The two answer the same question. They can differ in WHICH error they name
/// when an input has several, because this one interleaves the per-record checks
/// with the sweep while the slice form runs them in two passes; the callers ask
/// only whether the ledger is valid. `hour_ordered_validation_agrees_with_pairwise`
/// holds them to the same verdict.
pub fn validate_gap_records_hour_ordered<I>(
    records: I,
    observed_hour_start: i32,
    observed_hour_end: i32,
) -> Result<(), GapRecordValidationError>
where
    I: IntoIterator<Item = GapRecord>,
{
    if observed_hour_start > observed_hour_end {
        return Err(GapRecordValidationError::InvalidObservedRange {
            hour_start: observed_hour_start,
            hour_end: observed_hour_end,
        });
    }
    // The records that have started and not yet ended. Its size is the number of
    // gaps open at one instant — a handful of domains — not the session length.
    let mut open: Vec<GapRecord> = Vec::new();
    let mut previous_start: Option<i32> = None;
    for record in records {
        if let Some(previous) = previous_start {
            if record.hour_start < previous {
                return Err(GapRecordValidationError::NotHourOrdered {
                    previous_start: previous,
                    next_start: record.hour_start,
                });
            }
        }
        previous_start = Some(record.hour_start);
        record.validate()?;
        if record.hour_start < observed_hour_start || record.hour_end > observed_hour_end {
            return Err(GapRecordValidationError::OutsideObservedRange {
                gap_start: record.hour_start,
                gap_end: record.hour_end,
                observed_start: observed_hour_start,
                observed_end: observed_hour_end,
            });
        }
        open.retain(|candidate| candidate.hour_end >= record.hour_start);
        for candidate in &open {
            // `candidate.hour_start <= record.hour_start <= candidate.hour_end`
            // after the retain above, so the hour ranges intersect and only the
            // domain question is left.
            let overlap_domain = if candidate.domains.is_empty() || record.domains.is_empty() {
                Some(None)
            } else {
                candidate
                    .domains
                    .iter()
                    .find(|domain| record.domains.binary_search(domain).is_ok())
                    .copied()
                    .map(Some)
            };
            if let Some(domain) = overlap_domain {
                return Err(GapRecordValidationError::OverlapForDomain {
                    domain,
                    first_start: candidate.hour_start,
                    first_end: candidate.hour_end,
                    second_start: record.hour_start,
                    second_end: record.hour_end,
                });
            }
        }
        open.push(record);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GapRecordCodecError {
    TooShort,
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidHeaderLength(u16),
    LengthMismatch { declared: usize, actual: usize },
    NonZeroReserved,
    UnknownReason(u8),
    UnknownWitness(u8),
    UnknownCounter(u8),
    DomainCountOverflow,
    DomainCountTooLarge { count: usize, max: usize },
    Validation(GapRecordValidationError),
}

impl fmt::Display for GapRecordCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid encoded gap record: {self:?}")
    }
}

impl std::error::Error for GapRecordCodecError {}

impl From<GapRecordValidationError> for GapRecordCodecError {
    fn from(value: GapRecordValidationError) -> Self {
        Self::Validation(value)
    }
}

fn gap_reason_code(reason: GapReason) -> u8 {
    reason.code()
}

fn gap_reason_from_code(code: u8) -> Result<GapReason, GapRecordCodecError> {
    match code {
        GAP_REASON_CODE_COALESCED => Ok(GapReason::Coalesced),
        GAP_REASON_CODE_SKIPPED_HOUR => Ok(GapReason::SkippedHour),
        GAP_REASON_CODE_SOURCE_UNAVAILABLE => Ok(GapReason::SourceUnavailable),
        GAP_REASON_CODE_PARTIAL_DATA => Ok(GapReason::PartialData),
        GAP_REASON_CODE_BRIDGE_UNAVAILABLE => Ok(GapReason::BridgeUnavailable),
        GAP_REASON_CODE_WORKER_PANIC => Ok(GapReason::WorkerPanic),
        GAP_REASON_CODE_TORN_SWEEP => Ok(GapReason::TornSweep),
        GAP_REASON_CODE_COVERAGE_SHORTFALL => Ok(GapReason::CoverageShortfall),
        GAP_REASON_CODE_BACKPRESSURE => Ok(GapReason::Backpressure),
        other => Err(GapRecordCodecError::UnknownReason(other)),
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

/// Encode one canonical gap record in the dependency-free H4GP v2 envelope.
///
/// This is a ledger encoding, not an H4CR record type. It deliberately does
/// not alter compact-stream framing or capture production.
pub fn encode_gap_record(record: &GapRecord) -> Result<Vec<u8>, GapRecordCodecError> {
    if record.domains.len() > MAX_GAP_DOMAIN_COUNT {
        return Err(GapRecordCodecError::DomainCountTooLarge {
            count: record.domains.len(),
            max: MAX_GAP_DOMAIN_COUNT,
        });
    }
    record.validate()?;
    let domain_bytes = record
        .domains
        .len()
        .checked_mul(core::mem::size_of::<CompactDomainId>())
        .ok_or(GapRecordCodecError::DomainCountOverflow)?;
    let total_len = GAP_RECORD_HEADER_LEN
        .checked_add(domain_bytes)
        .ok_or(GapRecordCodecError::DomainCountOverflow)?;
    let domain_count = u32::try_from(record.domains.len())
        .map_err(|_| GapRecordCodecError::DomainCountOverflow)?;
    let total_len_u32 =
        u32::try_from(total_len).map_err(|_| GapRecordCodecError::DomainCountOverflow)?;
    let mut bytes = vec![0u8; total_len];
    bytes[..4].copy_from_slice(&GAP_RECORD_MAGIC);
    write_u16(&mut bytes, 4, GAP_RECORD_VERSION);
    write_u16(&mut bytes, 6, GAP_RECORD_HEADER_LEN as u16);
    write_u32(&mut bytes, 8, total_len_u32);
    write_i32(&mut bytes, 12, record.hour_start);
    write_i32(&mut bytes, 16, record.hour_end);
    bytes[20] = gap_reason_code(record.reason);
    write_u32(&mut bytes, 24, domain_count);
    match &record.witness {
        GapWitness::Counter { counter, observed } => {
            bytes[21] = 1;
            bytes[28] = counter.code();
            write_u64(&mut bytes, 32, *observed);
        }
        GapWitness::HourDiscontinuity {
            previous_hour,
            next_hour,
        } => {
            bytes[21] = 2;
            write_i32(&mut bytes, 28, *previous_hour);
            write_i32(&mut bytes, 32, *next_hour);
        }
        GapWitness::StatusBits { status_bits } => {
            bytes[21] = 3;
            write_u32(&mut bytes, 28, *status_bits);
        }
        GapWitness::Coverage {
            expected_perf_records,
            observed_perf_records,
            expected_data_records,
            observed_data_records,
        } => {
            bytes[21] = 4;
            write_u32(&mut bytes, 28, *expected_perf_records);
            write_u32(&mut bytes, 32, *observed_perf_records);
            write_u32(&mut bytes, 36, *expected_data_records);
            write_u32(&mut bytes, 40, *observed_data_records);
        }
        GapWitness::TornSweep {
            tick_start,
            tick_end,
        } => {
            bytes[21] = 5;
            write_u64(&mut bytes, 28, *tick_start);
            write_u64(&mut bytes, 36, *tick_end);
        }
        GapWitness::CoverageAndStatus {
            expected_perf_records,
            observed_perf_records,
            expected_data_records,
            observed_data_records,
            status_bits,
        } => {
            bytes[21] = 6;
            write_u32(&mut bytes, 28, *expected_perf_records);
            write_u32(&mut bytes, 32, *observed_perf_records);
            write_u32(&mut bytes, 36, *expected_data_records);
            write_u32(&mut bytes, 40, *observed_data_records);
            write_u32(&mut bytes, 44, *status_bits);
        }
    }
    for (index, domain) in record.domains.iter().enumerate() {
        write_u16(&mut bytes, GAP_RECORD_HEADER_LEN + index * 2, *domain);
    }
    Ok(bytes)
}

/// Decode one canonical H4GP v1/v2 ledger entry and re-run semantic validation.
pub fn decode_gap_record(bytes: &[u8]) -> Result<GapRecord, GapRecordCodecError> {
    if bytes.len() < GAP_RECORD_HEADER_LEN {
        return Err(GapRecordCodecError::TooShort);
    }
    if &bytes[..4] != GAP_RECORD_MAGIC.as_slice() {
        return Err(GapRecordCodecError::InvalidMagic);
    }
    let version = read_u16(bytes, 4);
    if !matches!(version, GAP_RECORD_LEGACY_VERSION | GAP_RECORD_VERSION) {
        return Err(GapRecordCodecError::UnsupportedVersion(version));
    }
    let header_len = read_u16(bytes, 6);
    if usize::from(header_len) != GAP_RECORD_HEADER_LEN {
        return Err(GapRecordCodecError::InvalidHeaderLength(header_len));
    }
    let declared_len =
        usize::try_from(read_u32(bytes, 8)).map_err(|_| GapRecordCodecError::LengthMismatch {
            declared: usize::MAX,
            actual: bytes.len(),
        })?;
    if declared_len != bytes.len() {
        return Err(GapRecordCodecError::LengthMismatch {
            declared: declared_len,
            actual: bytes.len(),
        });
    }
    if bytes[22] != 0 || bytes[23] != 0 {
        return Err(GapRecordCodecError::NonZeroReserved);
    }
    let domain_count = usize::try_from(read_u32(bytes, 24))
        .map_err(|_| GapRecordCodecError::DomainCountOverflow)?;
    if domain_count > MAX_GAP_DOMAIN_COUNT {
        return Err(GapRecordCodecError::DomainCountTooLarge {
            count: domain_count,
            max: MAX_GAP_DOMAIN_COUNT,
        });
    }
    let expected_len = GAP_RECORD_HEADER_LEN
        .checked_add(
            domain_count
                .checked_mul(2)
                .ok_or(GapRecordCodecError::DomainCountOverflow)?,
        )
        .ok_or(GapRecordCodecError::DomainCountOverflow)?;
    if expected_len != bytes.len() {
        return Err(GapRecordCodecError::LengthMismatch {
            declared: expected_len,
            actual: bytes.len(),
        });
    }
    if version == GAP_RECORD_LEGACY_VERSION && bytes[20] == GAP_REASON_CODE_BACKPRESSURE {
        return Err(GapRecordCodecError::UnknownReason(bytes[20]));
    }
    let reason = gap_reason_from_code(bytes[20])?;
    let witness = match bytes[21] {
        1 => {
            if bytes[29..32].iter().any(|byte| *byte != 0)
                || bytes[40..GAP_RECORD_HEADER_LEN]
                    .iter()
                    .any(|byte| *byte != 0)
            {
                return Err(GapRecordCodecError::NonZeroReserved);
            }
            let counter = match bytes[28] {
                GAP_COUNTER_CODE_COALESCED_HOURS => GapCounter::CoalescedHours,
                GAP_COUNTER_CODE_QUEUE_OVERRUN_HOURS if version == GAP_RECORD_VERSION => {
                    GapCounter::QueueOverrunHours
                }
                GAP_COUNTER_CODE_SINK_BACKPRESSURE_HOURS if version == GAP_RECORD_VERSION => {
                    GapCounter::SinkBackpressureHours
                }
                other => return Err(GapRecordCodecError::UnknownCounter(other)),
            };
            GapWitness::Counter {
                counter,
                observed: read_u64(bytes, 32),
            }
        }
        2 => {
            if bytes[36..GAP_RECORD_HEADER_LEN]
                .iter()
                .any(|byte| *byte != 0)
            {
                return Err(GapRecordCodecError::NonZeroReserved);
            }
            GapWitness::HourDiscontinuity {
                previous_hour: read_i32(bytes, 28),
                next_hour: read_i32(bytes, 32),
            }
        }
        3 => {
            if bytes[32..GAP_RECORD_HEADER_LEN]
                .iter()
                .any(|byte| *byte != 0)
            {
                return Err(GapRecordCodecError::NonZeroReserved);
            }
            GapWitness::StatusBits {
                status_bits: read_u32(bytes, 28),
            }
        }
        4 => {
            if bytes[44..GAP_RECORD_HEADER_LEN]
                .iter()
                .any(|byte| *byte != 0)
            {
                return Err(GapRecordCodecError::NonZeroReserved);
            }
            GapWitness::Coverage {
                expected_perf_records: read_u32(bytes, 28),
                observed_perf_records: read_u32(bytes, 32),
                expected_data_records: read_u32(bytes, 36),
                observed_data_records: read_u32(bytes, 40),
            }
        }
        5 => {
            if bytes[44..GAP_RECORD_HEADER_LEN]
                .iter()
                .any(|byte| *byte != 0)
            {
                return Err(GapRecordCodecError::NonZeroReserved);
            }
            GapWitness::TornSweep {
                tick_start: read_u64(bytes, 28),
                tick_end: read_u64(bytes, 36),
            }
        }
        6 => {
            if bytes[48..GAP_RECORD_HEADER_LEN]
                .iter()
                .any(|byte| *byte != 0)
            {
                return Err(GapRecordCodecError::NonZeroReserved);
            }
            GapWitness::CoverageAndStatus {
                expected_perf_records: read_u32(bytes, 28),
                observed_perf_records: read_u32(bytes, 32),
                expected_data_records: read_u32(bytes, 36),
                observed_data_records: read_u32(bytes, 40),
                status_bits: read_u32(bytes, 44),
            }
        }
        other => return Err(GapRecordCodecError::UnknownWitness(other)),
    };
    let mut domains = Vec::with_capacity(domain_count);
    for index in 0..domain_count {
        domains.push(read_u16(bytes, GAP_RECORD_HEADER_LEN + index * 2));
    }
    let record = GapRecord {
        hour_start: read_i32(bytes, 12),
        hour_end: read_i32(bytes, 16),
        reason,
        domains,
        witness,
    };
    record.validate()?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Whichever way a ledger is walked, it is the same ledger.
    ///
    /// The sweep replaces an O(n^2) pairwise loop on the recorder's finalize
    /// path, so what has to hold is that it never says "valid" where the
    /// pairwise form said "invalid" — a missed overlap would publish a ledger
    /// that double-counts an hour. The cases below are built to make the sweep
    /// fail if it purged the open set too eagerly (a long record still open
    /// under several later ones), if it ignored the wildcard rule (an
    /// empty-`domains` record beside a domain one), or if it compared only
    /// adjacent records.
    #[test]
    fn hour_ordered_validation_agrees_with_pairwise() {
        fn shortfall(hour_start: i32, hour_end: i32, domains: Vec<CompactDomainId>) -> GapRecord {
            GapRecord {
                hour_start,
                hour_end,
                reason: GapReason::SourceUnavailable,
                domains,
                witness: GapWitness::StatusBits {
                    status_bits: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                },
            }
        }
        fn skipped(hour_start: i32, hour_end: i32) -> GapRecord {
            GapRecord {
                hour_start,
                hour_end,
                reason: GapReason::SkippedHour,
                domains: Vec::new(),
                witness: GapWitness::HourDiscontinuity {
                    previous_hour: hour_start - 1,
                    next_hour: hour_end + 1,
                },
            }
        }

        // The expected verdict is written down per case, so a sweep that said
        // "valid" for everything would fail here rather than merely agree with
        // a pairwise loop that had been broken the same way.
        let cases: Vec<(bool, Vec<GapRecord>)> = vec![
            // Disjoint by domain at one hour.
            (
                true,
                vec![shortfall(10, 10, vec![4]), shortfall(10, 10, vec![5])],
            ),
            // The same domain twice at one hour.
            (
                false,
                vec![shortfall(10, 10, vec![4]), shortfall(10, 10, vec![4])],
            ),
            // A long record for domain 4 and a later point record for it, with
            // an unrelated domain in between: only a sweep that keeps the long
            // record open finds this.
            (
                false,
                vec![
                    shortfall(10, 40, vec![4]),
                    shortfall(12, 12, vec![9]),
                    shortfall(30, 30, vec![4]),
                ],
            ),
            // The same, but the late record is for another domain.
            (
                true,
                vec![
                    shortfall(10, 40, vec![4]),
                    shortfall(12, 12, vec![9]),
                    shortfall(30, 30, vec![7]),
                ],
            ),
            // A wildcard record overlaps every domain record inside it.
            (false, vec![skipped(10, 20), shortfall(15, 15, vec![4])]),
            // Two wildcards that touch but do not overlap.
            (true, vec![skipped(10, 12), skipped(13, 15)]),
            // Two wildcards that do overlap.
            (false, vec![skipped(10, 14), skipped(12, 15)]),
            // A record outside the observed range.
            (false, vec![shortfall(1, 1, vec![4])]),
            (true, vec![]),
        ];

        for (expected_valid, case) in cases {
            let mut ordered = case.clone();
            ordered.sort_by_key(|record| record.hour_start);
            let pairwise = validate_gap_records(&case, 5, 100).is_ok();
            let swept = validate_gap_records_hour_ordered(ordered, 5, 100).is_ok();
            assert_eq!(pairwise, expected_valid, "pairwise verdict for {case:?}");
            assert_eq!(swept, expected_valid, "swept verdict for {case:?}");
        }
    }

    /// Out-of-order input is refused rather than answered. The sweep can only
    /// see an overlap while both records are open, so a record that arrives
    /// after its own hour has passed could be waved through.
    #[test]
    fn hour_ordered_validation_refuses_records_that_go_backwards() {
        let records = vec![
            GapRecord {
                hour_start: 20,
                hour_end: 20,
                reason: GapReason::SourceUnavailable,
                domains: vec![4],
                witness: GapWitness::StatusBits {
                    status_bits: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                },
            },
            GapRecord {
                hour_start: 10,
                hour_end: 10,
                reason: GapReason::SourceUnavailable,
                domains: vec![4],
                witness: GapWitness::StatusBits {
                    status_bits: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                },
            },
        ];
        assert_eq!(
            validate_gap_records_hour_ordered(records, 5, 100),
            Err(GapRecordValidationError::NotHourOrdered {
                previous_start: 20,
                next_start: 10,
            })
        );
    }

    #[test]
    fn counter_and_backpressure_reason_codes_are_exact_and_round_trip() {
        assert_eq!(GapCounter::CoalescedHours.code(), 1);
        assert_eq!(GapCounter::QueueOverrunHours.code(), 2);
        assert_eq!(GapCounter::SinkBackpressureHours.code(), 3);
        assert_eq!(GapReason::Backpressure.code(), 9);

        for counter in [
            GapCounter::QueueOverrunHours,
            GapCounter::SinkBackpressureHours,
        ] {
            let record = GapRecord {
                hour_start: 20,
                hour_end: 22,
                reason: GapReason::Backpressure,
                domains: vec![81],
                witness: GapWitness::Counter {
                    counter,
                    observed: 3,
                },
            };
            let wire = encode_gap_record(&record).unwrap();
            assert_eq!(wire[20], GAP_REASON_CODE_BACKPRESSURE);
            assert_eq!(wire[28], counter.code());
            assert_eq!(decode_gap_record(&wire).unwrap(), record);
        }
    }

    #[test]
    fn v2_extends_backpressure_while_legacy_v1_remains_decodable_and_closed() {
        let legacy = GapRecord {
            hour_start: 20,
            hour_end: 20,
            reason: GapReason::Coalesced,
            domains: vec![81],
            witness: GapWitness::Counter {
                counter: GapCounter::CoalescedHours,
                observed: 1,
            },
        };
        let mut legacy_wire = encode_gap_record(&legacy).unwrap();
        legacy_wire[4..6].copy_from_slice(&GAP_RECORD_LEGACY_VERSION.to_le_bytes());
        assert_eq!(decode_gap_record(&legacy_wire).unwrap(), legacy);

        let extended = GapRecord {
            reason: GapReason::Backpressure,
            witness: GapWitness::Counter {
                counter: GapCounter::QueueOverrunHours,
                observed: 1,
            },
            ..legacy
        };
        let mut extended_wire = encode_gap_record(&extended).unwrap();
        assert_eq!(
            u16::from_le_bytes(extended_wire[4..6].try_into().unwrap()),
            GAP_RECORD_VERSION
        );
        extended_wire[4..6].copy_from_slice(&GAP_RECORD_LEGACY_VERSION.to_le_bytes());
        assert_eq!(
            decode_gap_record(&extended_wire),
            Err(GapRecordCodecError::UnknownReason(
                GAP_REASON_CODE_BACKPRESSURE
            ))
        );
    }

    fn coverage(hour: i32, domains: Vec<u16>) -> GapRecord {
        GapRecord {
            hour_start: hour,
            hour_end: hour,
            reason: GapReason::CoverageShortfall,
            domains,
            witness: GapWitness::Coverage {
                expected_perf_records: 1,
                observed_perf_records: 0,
                expected_data_records: 1,
                observed_data_records: 0,
            },
        }
    }

    #[test]
    fn canonical_shape_has_inclusive_ranges_and_empty_domains_mean_all() {
        let record = GapRecord {
            hour_start: 41,
            hour_end: 43,
            reason: GapReason::Coalesced,
            domains: Vec::new(),
            witness: GapWitness::Counter {
                counter: GapCounter::CoalescedHours,
                observed: 3,
            },
        };
        assert_eq!(record.inclusive_hour_count(), Some(3));
        assert!(record.covers(42, 17));
        assert!(!record.covers(44, 17));
        assert!(record.validate().is_ok());
    }

    #[test]
    fn reason_witness_mismatch_and_noncanonical_domains_are_rejected() {
        let mut record = coverage(12, vec![10, 11]);
        record.witness = GapWitness::StatusBits {
            status_bits: COMPACT_STATUS_PARTIAL_DATA,
        };
        assert!(matches!(
            record.validate(),
            Err(GapRecordValidationError::WitnessDoesNotProveReason(
                GapReason::CoverageShortfall
            ))
        ));
        let duplicate = coverage(12, vec![10, 10]);
        assert_eq!(
            duplicate.validate(),
            Err(GapRecordValidationError::DomainsNotStrictlyIncreasing)
        );
    }

    #[test]
    fn global_and_domain_specific_overlaps_are_rejected() {
        let global = coverage(12, Vec::new());
        let domain = coverage(12, vec![10]);
        assert!(matches!(
            validate_gap_records(&[global, domain], 12, 12),
            Err(GapRecordValidationError::OverlapForDomain { domain: None, .. })
        ));
    }

    #[test]
    fn disjoint_domains_may_share_an_hour_but_ranges_must_stay_observed() {
        assert!(
            validate_gap_records(&[coverage(12, vec![10]), coverage(12, vec![11])], 12, 12).is_ok()
        );
        assert!(matches!(
            validate_gap_records(&[coverage(11, vec![10])], 12, 13),
            Err(GapRecordValidationError::OutsideObservedRange { .. })
        ));
    }

    #[test]
    fn every_reason_has_a_typed_proving_witness() {
        let cases = [
            GapRecord {
                hour_start: 1,
                hour_end: 1,
                reason: GapReason::SourceUnavailable,
                domains: vec![10],
                witness: GapWitness::StatusBits {
                    status_bits: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                },
            },
            GapRecord {
                hour_start: 1,
                hour_end: 1,
                reason: GapReason::PartialData,
                domains: vec![10],
                witness: GapWitness::StatusBits {
                    status_bits: COMPACT_STATUS_PARTIAL_DATA,
                },
            },
            GapRecord {
                hour_start: 1,
                hour_end: 1,
                reason: GapReason::BridgeUnavailable,
                domains: vec![10],
                witness: GapWitness::StatusBits {
                    status_bits: COMPACT_STATUS_BRIDGE_UNAVAILABLE,
                },
            },
            GapRecord {
                hour_start: 1,
                hour_end: 1,
                reason: GapReason::WorkerPanic,
                domains: vec![10],
                witness: GapWitness::StatusBits {
                    status_bits: COMPACT_STATUS_WORKER_PANIC,
                },
            },
            GapRecord {
                hour_start: 1,
                hour_end: 1,
                reason: GapReason::TornSweep,
                domains: Vec::new(),
                witness: GapWitness::TornSweep {
                    tick_start: 7,
                    tick_end: 8,
                },
            },
        ];
        for record in cases {
            assert!(record.validate().is_ok(), "{record:?}");
        }
    }

    #[test]
    fn multi_bit_status_uses_one_exact_dominant_reason() {
        let source_and_partial = COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA;
        let record = GapRecord {
            hour_start: 3,
            hour_end: 3,
            reason: GapReason::SourceUnavailable,
            domains: vec![20],
            witness: GapWitness::StatusBits {
                status_bits: source_and_partial,
            },
        };
        assert!(record.validate().is_ok());
        let mut noncanonical = record.clone();
        noncanonical.reason = GapReason::PartialData;
        assert!(matches!(
            noncanonical.validate(),
            Err(GapRecordValidationError::WitnessDoesNotProveReason(
                GapReason::PartialData
            ))
        ));

        let panic_and_partial = COMPACT_STATUS_WORKER_PANIC | COMPACT_STATUS_PARTIAL_DATA;
        assert_eq!(
            GapReason::canonical_for_status_bits(panic_and_partial),
            Some(GapReason::WorkerPanic)
        );
    }

    /// A measured-empty hour is not a recording gap.
    ///
    /// A gap is evidence that some game-state bytes were NOT OBSERVED. A domain
    /// that reached its source and found nothing observed all of them; there
    /// are simply no rows. Admitting it here would recreate the ambiguity the
    /// badge exists to remove, one layer further down: the section-8 ledger
    /// would carry an hour of missing data that is not missing.
    ///
    /// Production change that makes this fail: adding SOURCE_EMPTY to
    /// `GAP_STATUS_MASK`, or to the recorder's `GAP_ELIGIBLE_STATUS_MASK`
    /// (which is what feeds this function).
    #[test]
    fn a_measured_empty_status_word_is_never_a_gap_reason() {
        assert_eq!(
            GapReason::canonical_for_status_bits(crate::COMPACT_STATUS_SOURCE_EMPTY),
            None
        );
        assert_eq!(GAP_STATUS_MASK & crate::COMPACT_STATUS_SOURCE_EMPTY, 0);
    }

    #[test]
    fn composite_witness_proves_status_and_coverage_without_overlap() {
        let record = GapRecord {
            hour_start: 4,
            hour_end: 4,
            reason: GapReason::WorkerPanic,
            domains: vec![20],
            witness: GapWitness::CoverageAndStatus {
                expected_perf_records: 1,
                observed_perf_records: 0,
                expected_data_records: 1,
                observed_data_records: 0,
                status_bits: COMPACT_STATUS_WORKER_PANIC | COMPACT_STATUS_PARTIAL_DATA,
            },
        };
        assert!(record.validate().is_ok());
        assert!(validate_gap_records(&[record], 4, 4).is_ok());
    }

    #[test]
    fn versioned_codec_round_trips_every_witness_shape() {
        let records = [
            GapRecord {
                hour_start: 1,
                hour_end: 2,
                reason: GapReason::Coalesced,
                domains: Vec::new(),
                witness: GapWitness::Counter {
                    counter: GapCounter::CoalescedHours,
                    observed: 2,
                },
            },
            GapRecord {
                hour_start: 2,
                hour_end: 2,
                reason: GapReason::SkippedHour,
                domains: vec![10],
                witness: GapWitness::HourDiscontinuity {
                    previous_hour: 1,
                    next_hour: 3,
                },
            },
            GapRecord {
                hour_start: 3,
                hour_end: 3,
                reason: GapReason::SourceUnavailable,
                domains: vec![11],
                witness: GapWitness::StatusBits {
                    status_bits: COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
                },
            },
            coverage(4, vec![12]),
            GapRecord {
                hour_start: 5,
                hour_end: 5,
                reason: GapReason::TornSweep,
                domains: Vec::new(),
                witness: GapWitness::TornSweep {
                    tick_start: 10,
                    tick_end: 11,
                },
            },
            GapRecord {
                hour_start: 6,
                hour_end: 6,
                reason: GapReason::WorkerPanic,
                domains: vec![13],
                witness: GapWitness::CoverageAndStatus {
                    expected_perf_records: 1,
                    observed_perf_records: 0,
                    expected_data_records: 1,
                    observed_data_records: 0,
                    status_bits: COMPACT_STATUS_WORKER_PANIC | COMPACT_STATUS_PARTIAL_DATA,
                },
            },
        ];
        for record in records {
            let encoded = encode_gap_record(&record).expect("encode canonical gap");
            assert_eq!(
                decode_gap_record(&encoded).expect("decode canonical gap"),
                record
            );
        }
    }

    #[test]
    fn forward_jump_is_one_half_open_skipped_range_and_requires_a_real_jump() {
        let gap = GapRecord::skipped_tick_discontinuity_half_open(10, 14, vec![81]).unwrap();
        assert_eq!(gap.hour_start, 11);
        assert_eq!(gap.hour_end, 13);
        assert_eq!(gap.inclusive_hour_count(), Some(3));
        assert_eq!(gap.reason, GapReason::SkippedHour);
        assert_eq!(
            gap.witness,
            GapWitness::HourDiscontinuity {
                previous_hour: 10,
                next_hour: 14,
            }
        );
        assert!(GapRecord::skipped_tick_discontinuity_half_open(10, 11, vec![81]).is_err());
    }

    #[test]
    fn versioned_codec_rejects_unknown_and_malformed_enums() {
        let record = coverage(8, vec![20]);
        let encoded = encode_gap_record(&record).expect("encode canonical gap");

        let mut unknown_reason = encoded.clone();
        unknown_reason[20] = 0xff;
        assert_eq!(
            decode_gap_record(&unknown_reason),
            Err(GapRecordCodecError::UnknownReason(0xff))
        );

        let mut unknown_witness = encoded.clone();
        unknown_witness[21] = 0xff;
        assert_eq!(
            decode_gap_record(&unknown_witness),
            Err(GapRecordCodecError::UnknownWitness(0xff))
        );

        let coalesced = GapRecord {
            hour_start: 8,
            hour_end: 8,
            reason: GapReason::Coalesced,
            domains: Vec::new(),
            witness: GapWitness::Counter {
                counter: GapCounter::CoalescedHours,
                observed: 1,
            },
        };
        let mut unknown_counter =
            encode_gap_record(&coalesced).expect("encode canonical counter gap");
        unknown_counter[28] = 0xff;
        assert_eq!(
            decode_gap_record(&unknown_counter),
            Err(GapRecordCodecError::UnknownCounter(0xff))
        );

        let mut unknown_version = encoded.clone();
        write_u16(&mut unknown_version, 4, GAP_RECORD_VERSION + 1);
        assert_eq!(
            decode_gap_record(&unknown_version),
            Err(GapRecordCodecError::UnsupportedVersion(
                GAP_RECORD_VERSION + 1
            ))
        );

        let mut malformed_reserved = encoded;
        malformed_reserved[22] = 1;
        assert_eq!(
            decode_gap_record(&malformed_reserved),
            Err(GapRecordCodecError::NonZeroReserved)
        );
    }

    #[test]
    fn versioned_codec_rejects_domain_counts_above_the_protocol_bound() {
        let mut too_many = coverage(8, vec![1; MAX_GAP_DOMAIN_COUNT + 1]);
        assert_eq!(
            encode_gap_record(&too_many),
            Err(GapRecordCodecError::DomainCountTooLarge {
                count: MAX_GAP_DOMAIN_COUNT + 1,
                max: MAX_GAP_DOMAIN_COUNT,
            })
        );

        too_many.domains = vec![20];
        let mut encoded = encode_gap_record(&too_many).expect("encode bounded gap");
        write_u32(&mut encoded, 24, (MAX_GAP_DOMAIN_COUNT as u32) + 1);
        assert_eq!(
            decode_gap_record(&encoded),
            Err(GapRecordCodecError::DomainCountTooLarge {
                count: MAX_GAP_DOMAIN_COUNT + 1,
                max: MAX_GAP_DOMAIN_COUNT,
            })
        );
    }

    /// `GapReason::ALL` must hold every reason a record can arrive carrying.
    ///
    /// Rebuilt from the wire decoder rather than from a second list: a variant
    /// given the next free code and never added to `ALL` is invisible to any
    /// test written against `ALL` itself, and the compacted manifest gap ledger
    /// resolves names through `from_wire_name`, which walks `ALL`. A reason the
    /// producer can emit but `ALL` omits would seal a manifest no expander can
    /// read back.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: assigning a tenth gap reason code
    /// without adding the variant to `ALL`.
    #[test]
    fn gap_reason_all_covers_every_decodable_reason() {
        let decodable: BTreeSet<GapReason> = (0u8..=u8::MAX)
            .filter_map(|code| gap_reason_from_code(code).ok())
            .collect();
        let enumerated: BTreeSet<GapReason> = GapReason::ALL.into_iter().collect();
        assert_eq!(
            enumerated, decodable,
            "GapReason::ALL and the wire decoder disagree about the reason set"
        );
        assert_eq!(enumerated.len(), GapReason::ALL.len(), "ALL lists a duplicate");
    }

    /// Names are the manifest's only spelling of a reason, and the ledger
    /// expander turns them back into reasons. A collision would make two
    /// outcomes indistinguishable on the manifest; a broken inverse would make
    /// an honest ledger unreadable.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: renaming one reason on only one
    /// side of the pair, or giving two reasons the same name.
    #[test]
    fn gap_reason_wire_names_are_unique_and_round_trip() {
        let names: BTreeSet<&str> = GapReason::ALL.iter().map(|r| r.wire_name()).collect();
        assert_eq!(names.len(), GapReason::ALL.len(), "two reasons share a name");
        for reason in GapReason::ALL {
            assert_eq!(GapReason::from_wire_name(reason.wire_name()), Some(reason));
        }
        assert_eq!(GapReason::from_wire_name("not_a_reason"), None);
    }
}
