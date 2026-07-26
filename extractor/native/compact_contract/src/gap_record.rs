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
pub const GAP_RECORD_VERSION: u16 = 1;
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
}

impl GapReason {
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
        let hour_count = self
            .inclusive_hour_count()
            .ok_or(GapRecordValidationError::InvalidInclusiveRange {
                hour_start: self.hour_start,
                hour_end: self.hour_end,
            })?;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GapRecordCodecError {
    TooShort,
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidHeaderLength(u16),
    LengthMismatch {
        declared: usize,
        actual: usize,
    },
    NonZeroReserved,
    UnknownReason(u8),
    UnknownWitness(u8),
    UnknownCounter(u8),
    DomainCountOverflow,
    DomainCountTooLarge {
        count: usize,
        max: usize,
    },
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
    match reason {
        GapReason::Coalesced => 1,
        GapReason::SkippedHour => 2,
        GapReason::SourceUnavailable => 3,
        GapReason::PartialData => 4,
        GapReason::BridgeUnavailable => 5,
        GapReason::WorkerPanic => 6,
        GapReason::TornSweep => 7,
        GapReason::CoverageShortfall => 8,
    }
}

fn gap_reason_from_code(code: u8) -> Result<GapReason, GapRecordCodecError> {
    match code {
        1 => Ok(GapReason::Coalesced),
        2 => Ok(GapReason::SkippedHour),
        3 => Ok(GapReason::SourceUnavailable),
        4 => Ok(GapReason::PartialData),
        5 => Ok(GapReason::BridgeUnavailable),
        6 => Ok(GapReason::WorkerPanic),
        7 => Ok(GapReason::TornSweep),
        8 => Ok(GapReason::CoverageShortfall),
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

/// Encode one canonical gap record in the dependency-free H4GP v1 envelope.
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
    let domain_count =
        u32::try_from(record.domains.len()).map_err(|_| GapRecordCodecError::DomainCountOverflow)?;
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
            bytes[28] = match counter {
                GapCounter::CoalescedHours => 1,
            };
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

/// Decode one canonical H4GP v1 ledger entry and re-run semantic validation.
pub fn decode_gap_record(bytes: &[u8]) -> Result<GapRecord, GapRecordCodecError> {
    if bytes.len() < GAP_RECORD_HEADER_LEN {
        return Err(GapRecordCodecError::TooShort);
    }
    if &bytes[..4] != GAP_RECORD_MAGIC.as_slice() {
        return Err(GapRecordCodecError::InvalidMagic);
    }
    let version = read_u16(bytes, 4);
    if version != GAP_RECORD_VERSION {
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
    let domain_count =
        usize::try_from(read_u32(bytes, 24)).map_err(|_| GapRecordCodecError::DomainCountOverflow)?;
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
                1 => GapCounter::CoalescedHours,
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
        assert!(validate_gap_records(&[coverage(12, vec![10]), coverage(12, vec![11])], 12, 12)
            .is_ok());
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
        let source_and_partial =
            COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA;
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
                    status_bits: COMPACT_STATUS_SOURCE_UNAVAILABLE
                        | COMPACT_STATUS_PARTIAL_DATA,
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
                    status_bits: COMPACT_STATUS_WORKER_PANIC
                        | COMPACT_STATUS_PARTIAL_DATA,
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
}
