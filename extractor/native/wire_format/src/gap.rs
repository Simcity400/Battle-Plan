//! Container-level gap records and the canonical gap contract normalisation.
//!
//! Ranges are inclusive on the wire but half-open in this API. Keeping the
//! conversion at the codec boundary makes overlap resolution and range queries
//! use one representation.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::varint::{compare_canonical_bytes, read_varint, write_varint};
use crate::{Result, WireError};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum GapReason {
    TornSweep = 0,
    SourceUnavailable = 1,
    PartialData = 2,
    Coalesced = 3,
    Skipped = 4,
    BridgeUnavailable = 5,
    WorkerPanic = 6,
    CoverageShortfall = 7,
    Backpressure = 8,
}

crate::wire_coded::wire_coded_kinds! {
    GapReason: u8,
    code = pub code,
    Ok = from_code(code) -> crate::Result<Self> {
        Err(crate::WireError::Corrupt("unassigned gap reason"))
    };

    TornSweep = 0,
    SourceUnavailable = 1,
    PartialData = 2,
    Coalesced = 3,
    Skipped = 4,
    BridgeUnavailable = 5,
    WorkerPanic = 6,
    CoverageShortfall = 7,
    Backpressure = 8,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum GapPart {
    State = 0,
    Event = 1,
    Both = 2,
}

crate::wire_coded::wire_coded_kinds! {
    GapPart: u8,
    code = pub code,
    Ok = from_code(code) -> crate::Result<Self> {
        Err(crate::WireError::Corrupt("gap part is outside 0..=2"))
    };

    State = 0,
    Event = 1,
    Both = 2,
}

impl GapPart {
    pub const fn affects_state(self) -> bool {
        matches!(self, Self::State | Self::Both)
    }
}

/// The five witness payloads assigned by gap contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GapWitness {
    Counter { counter_id: u64, observed: u64 },
    StatusBits { bit_width: u64, bits: Vec<u8> },
    Coverage { observed: u64, expected: u64 },
    TickBracket { start_tick: u64, end_tick: u64 },
    TickDiscontinuity { previous_tick: u64, next_tick: u64 },
}

fn validate_status_bits(bit_width: u64, bits: &[u8]) -> Result<()> {
    let expected = bit_width / 8 + (bit_width % 8 != 0) as u64;
    let actual = u64::try_from(bits.len())
        .map_err(|_| WireError::Corrupt("status witness is too large"))?;
    if actual != expected {
        return Err(WireError::Corrupt(
            "status witness length does not match its bit width",
        ));
    }

    let used_bits_in_last_byte = (bit_width % 8) as u32;
    if used_bits_in_last_byte != 0 {
        let unused_high_bits = !((1u8 << used_bits_in_last_byte) - 1);
        if bits
            .last()
            .is_some_and(|last| last & unused_high_bits != 0)
        {
            return Err(WireError::Corrupt(
                "status witness has non-zero unused high bits",
            ));
        }
    }
    Ok(())
}

impl GapWitness {
    pub const fn kind(&self) -> u8 {
        match self {
            Self::Counter { .. } => 0,
            Self::StatusBits { .. } => 1,
            Self::Coverage { .. } => 2,
            Self::TickBracket { .. } => 3,
            Self::TickDiscontinuity { .. } => 4,
        }
    }

    fn validate(&self) -> Result<()> {
        if let Self::StatusBits { bit_width, bits } = self {
            validate_status_bits(*bit_width, bits)?;
        }
        Ok(())
    }

    /// Bytes used after `witness_kind`, and therefore the exact bytes used by
    /// gap contract's cause-order comparison.
    pub fn canonical_payload(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut out = Vec::new();
        match self {
            Self::Counter {
                counter_id,
                observed,
            } => {
                write_varint(&mut out, *counter_id);
                write_varint(&mut out, *observed);
            }
            Self::StatusBits { bit_width, bits } => {
                write_varint(&mut out, *bit_width);
                out.extend_from_slice(bits);
            }
            Self::Coverage { observed, expected } => {
                write_varint(&mut out, *observed);
                write_varint(&mut out, *expected);
            }
            Self::TickBracket {
                start_tick,
                end_tick,
            } => {
                write_varint(&mut out, *start_tick);
                write_varint(&mut out, *end_tick);
            }
            Self::TickDiscontinuity {
                previous_tick,
                next_tick,
            } => {
                write_varint(&mut out, *previous_tick);
                write_varint(&mut out, *next_tick);
            }
        }
        Ok(out)
    }
}

/// One gap record. `tick_end` is exclusive in memory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GapRecord {
    pub tick_start: u64,
    pub tick_end: u64,
    pub branch: u32,
    pub reason: GapReason,
    pub part: GapPart,
    pub domains: Vec<u32>,
    pub witness: GapWitness,
}

impl GapRecord {
    pub fn validate(&self) -> Result<()> {
        if self.tick_start >= self.tick_end {
            return Err(WireError::Corrupt("gap range is empty or reversed"));
        }
        if self.domains.is_empty() {
            // The retired H4GP record used empty to mean every domain. gap contract has no
            // such sentinel: accepting it here would silently widen the gap.
            return Err(WireError::Corrupt("gap domain list is empty"));
        }
        if self.domains.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(WireError::Corrupt(
                "gap domains are not strictly increasing",
            ));
        }
        self.witness.validate()
    }

    pub fn range(&self) -> Range<u64> {
        self.tick_start..self.tick_end
    }
}

/// Encode one record in canonical field order.
pub fn encode_record(record: &GapRecord) -> Result<Vec<u8>> {
    record.validate()?;
    let mut out = Vec::new();
    write_varint(&mut out, record.tick_start);
    let inclusive_delta = record
        .tick_end
        .checked_sub(record.tick_start)
        .and_then(|width| width.checked_sub(1))
        .ok_or(WireError::Corrupt("gap range cannot be encoded"))?;
    write_varint(&mut out, inclusive_delta);
    write_varint(&mut out, u64::from(record.branch));
    out.push(record.reason.code());
    out.push(record.part.code());
    write_varint(
        &mut out,
        u64::try_from(record.domains.len())
            .map_err(|_| WireError::Corrupt("gap domain count is too large"))?,
    );
    let mut previous = 0u32;
    for (index, domain) in record.domains.iter().copied().enumerate() {
        let encoded = if index == 0 {
            u64::from(domain)
        } else {
            u64::from(
                domain
                    .checked_sub(previous)
                    .ok_or(WireError::Corrupt("gap domains are not ascending"))?,
            )
        };
        write_varint(&mut out, encoded);
        previous = domain;
    }
    out.push(record.witness.kind());
    out.extend_from_slice(&record.witness.canonical_payload()?);
    Ok(out)
}

fn byte_at(bytes: &[u8], pos: &mut usize) -> Result<u8> {
    let value = bytes.get(*pos).copied().ok_or(WireError::Truncated)?;
    *pos += 1;
    Ok(value)
}

fn two_varints(bytes: &[u8], pos: usize) -> Result<((u64, u64), usize)> {
    let (a, pos) = read_varint(bytes, pos)?;
    let (b, pos) = read_varint(bytes, pos)?;
    Ok(((a, b), pos))
}

fn decode_witness(kind: u8, bytes: &[u8], pos: usize) -> Result<(GapWitness, usize)> {
    match kind {
        0 => {
            let ((counter_id, observed), pos) = two_varints(bytes, pos)?;
            Ok((
                GapWitness::Counter {
                    counter_id,
                    observed,
                },
                pos,
            ))
        }
        1 => {
            let (bit_width, payload_start) = read_varint(bytes, pos)?;
            let expected = bit_width / 8 + (bit_width % 8 != 0) as u64;
            let remaining = bytes.len().saturating_sub(payload_start);
            let remaining_u64 = u64::try_from(remaining)
                .map_err(|_| WireError::Corrupt("status witness is too large"))?;
            if remaining_u64 < expected {
                return Err(WireError::Truncated);
            }
            if remaining_u64 > expected {
                return Err(WireError::Corrupt("trailing bytes after gap witness"));
            }
            let bits = &bytes[payload_start..];
            validate_status_bits(bit_width, bits)?;
            Ok((
                GapWitness::StatusBits {
                    bit_width,
                    bits: bits.to_vec(),
                },
                bytes.len(),
            ))
        }
        2 => {
            let ((observed, expected), pos) = two_varints(bytes, pos)?;
            Ok((GapWitness::Coverage { observed, expected }, pos))
        }
        3 => {
            let ((start_tick, end_tick), pos) = two_varints(bytes, pos)?;
            Ok((
                GapWitness::TickBracket {
                    start_tick,
                    end_tick,
                },
                pos,
            ))
        }
        4 => {
            let ((previous_tick, next_tick), pos) = two_varints(bytes, pos)?;
            Ok((
                GapWitness::TickDiscontinuity {
                    previous_tick,
                    next_tick,
                },
                pos,
            ))
        }
        _ => Err(WireError::Corrupt("unassigned gap witness kind")),
    }
}

/// Decode exactly one record. Trailing bytes are corrupt, including bytes after
/// a fixed-shape witness. The checked conversion maps the encoded inclusive end
/// to the in-memory half-open range.
pub fn decode_record(bytes: &[u8]) -> Result<GapRecord> {
    let (tick_start, pos) = read_varint(bytes, 0)?;
    let (tick_end_delta, pos) = read_varint(bytes, pos)?;
    let tick_end = tick_start
        .checked_add(tick_end_delta)
        .and_then(|inclusive_end| inclusive_end.checked_add(1))
        .ok_or(WireError::Corrupt("gap half-open range overflows u64"))?;
    let (branch, mut pos) = read_varint(bytes, pos)?;
    let branch =
        u32::try_from(branch).map_err(|_| WireError::Corrupt("gap branch exceeds 2^32 - 1"))?;
    let reason = GapReason::from_code(byte_at(bytes, &mut pos)?)?;
    let part = GapPart::from_code(byte_at(bytes, &mut pos)?)?;
    let (domain_count, next) = read_varint(bytes, pos)?;
    pos = next;
    if domain_count == 0 {
        return Err(WireError::Corrupt("gap domain list is empty"));
    }
    let remaining = bytes.len().saturating_sub(pos);
    if domain_count
        > u64::try_from(remaining).map_err(|_| WireError::Corrupt("gap record is too large"))?
    {
        // Every domain needs at least one byte. Refuse before allocating or
        // looping on an attacker-controlled count.
        return Err(WireError::Truncated);
    }
    let mut domains = Vec::new();
    let mut previous = 0u32;
    for index in 0..domain_count {
        let (encoded, next) = read_varint(bytes, pos)?;
        pos = next;
        let domain = if index == 0 {
            u32::try_from(encoded).map_err(|_| WireError::Corrupt("gap domain exceeds 2^32 - 1"))?
        } else {
            if encoded == 0 {
                return Err(WireError::Corrupt("gap domain is repeated"));
            }
            let delta = u32::try_from(encoded)
                .map_err(|_| WireError::Corrupt("gap domain delta exceeds 2^32 - 1"))?;
            previous
                .checked_add(delta)
                .ok_or(WireError::Corrupt("gap domain exceeds 2^32 - 1"))?
        };
        domains.push(domain);
        previous = domain;
    }
    let witness_kind = byte_at(bytes, &mut pos)?;
    let (witness, pos) = decode_witness(witness_kind, bytes, pos)?;
    if pos != bytes.len() {
        return Err(WireError::Corrupt("trailing bytes after gap witness"));
    }
    let record = GapRecord {
        tick_start,
        tick_end,
        branch,
        reason,
        part,
        domains,
        witness,
    };
    record.validate()?;
    Ok(record)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Cause {
    reason: GapReason,
    witness: GapWitness,
    payload: Vec<u8>,
}

impl Cause {
    fn new(reason: GapReason, witness: &GapWitness) -> Result<Self> {
        Ok(Self {
            reason,
            witness: witness.clone(),
            payload: witness.canonical_payload()?,
        })
    }
}

impl Ord for Cause {
    fn cmp(&self, other: &Self) -> Ordering {
        // Canonical cause order is reason, kind, then encoded payload as
        // unsigned bytes. Numeric witness order is wrong.
        self.reason
            .cmp(&other.reason)
            .then_with(|| self.witness.kind().cmp(&other.witness.kind()))
            .then_with(|| compare_canonical_bytes(&self.payload, &other.payload))
    }
}

impl PartialOrd for Cause {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Component {
    State,
    Event,
}

#[derive(Clone, Debug)]
struct Observation {
    start: u64,
    end: u64,
    cause: Cause,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GroupKey {
    branch: u32,
    start: u64,
    end: u64,
    cause: Cause,
}

/// Canonically normalise gap records. The output is invariant under input order
/// and duplicate observations, and can itself be normalised without changing
/// its bytes.
pub fn normalise(records: &[GapRecord]) -> Result<Vec<GapRecord>> {
    let mut by_component: BTreeMap<(u32, u32, Component), Vec<Observation>> = BTreeMap::new();
    for record in records {
        record.validate()?;
        let cause = Cause::new(record.reason, &record.witness)?;
        let components: &[Component] = match record.part {
            GapPart::State => &[Component::State],
            GapPart::Event => &[Component::Event],
            GapPart::Both => &[Component::State, Component::Event],
        };
        for domain in record.domains.iter().copied() {
            for component in components {
                by_component
                    .entry((record.branch, domain, *component))
                    .or_default()
                    .push(Observation {
                        start: record.tick_start,
                        end: record.tick_end,
                        cause: cause.clone(),
                    });
            }
        }
    }

    let mut grouped: BTreeMap<GroupKey, BTreeMap<u32, u8>> = BTreeMap::new();
    for ((branch, domain, component), observations) in by_component {
        let mut endpoints = BTreeSet::new();
        for observation in &observations {
            endpoints.insert(observation.start);
            endpoints.insert(observation.end);
        }
        let endpoints: Vec<u64> = endpoints.into_iter().collect();
        let mut spans: Vec<(u64, u64, Cause)> = Vec::new();
        for pair in endpoints.windows(2) {
            let start = pair[0];
            let end = pair[1];
            let winner = observations
                .iter()
                .filter(|observation| observation.start <= start && end <= observation.end)
                .map(|observation| &observation.cause)
                .min()
                .cloned();
            let Some(winner) = winner else {
                continue;
            };
            if let Some((_, prior_end, prior_cause)) = spans.last_mut() {
                if *prior_end == start && *prior_cause == winner {
                    *prior_end = end;
                    continue;
                }
            }
            spans.push((start, end, winner));
        }
        for (start, end, cause) in spans {
            let component_bit = match component {
                Component::State => 1,
                Component::Event => 2,
            };
            *grouped
                .entry(GroupKey {
                    branch,
                    start,
                    end,
                    cause,
                })
                .or_default()
                .entry(domain)
                .or_default() |= component_bit;
        }
    }

    let mut output = Vec::new();
    for (group, signatures) in grouped {
        for (signature, part) in [
            (1u8, GapPart::State),
            (2u8, GapPart::Event),
            (3u8, GapPart::Both),
        ] {
            let domains: Vec<u32> = signatures
                .iter()
                .filter_map(|(domain, bits)| (*bits == signature).then_some(*domain))
                .collect();
            if !domains.is_empty() {
                output.push(GapRecord {
                    tick_start: group.start,
                    tick_end: group.end,
                    branch: group.branch,
                    reason: group.cause.reason,
                    part,
                    domains,
                    witness: group.cause.witness.clone(),
                });
            }
        }
    }
    output.sort_by_key(|record| {
        (
            record.tick_start,
            record.branch,
            record.reason,
            record.part,
            record.domains.first().copied().unwrap_or(0),
        )
    });
    Ok(output)
}

/// Whether a state-affecting (`part = 0` or `2`) gap intersects `ticks` for one
/// `(branch, domain)`. Event-only gaps deliberately do not force a state
/// baseline.
pub fn state_gap_intersects(
    records: &[GapRecord],
    branch: u32,
    domain: u32,
    ticks: Range<u64>,
) -> bool {
    if ticks.start >= ticks.end {
        return false;
    }
    records.iter().any(|record| {
        record.branch == branch
            && record.part.affects_state()
            && record.domains.contains(&domain)
            && record.tick_start < ticks.end
            && ticks.start < record.tick_end
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    fn counter(
        reason: GapReason,
        part: GapPart,
        domains: Vec<u32>,
        start: u64,
        end: u64,
    ) -> GapRecord {
        GapRecord {
            tick_start: start,
            tick_end: end,
            branch: 0,
            reason,
            part,
            domains,
            witness: GapWitness::Counter {
                counter_id: 1,
                observed: 1,
            },
        }
    }

    fn encoded_stream(records: &[GapRecord]) -> Vec<u8> {
        records
            .iter()
            .flat_map(|record| encode_record(record).expect("normalised record encodes"))
            .collect()
    }

    fn component_coverage(records: &[GapRecord]) -> BTreeSet<(u32, u32, Component, u64)> {
        let mut coverage = BTreeSet::new();
        for record in records {
            let components: &[Component] = match record.part {
                GapPart::State => &[Component::State],
                GapPart::Event => &[Component::Event],
                GapPart::Both => &[Component::State, Component::Event],
            };
            for domain in &record.domains {
                for component in components {
                    for tick in record.tick_start..record.tick_end {
                        coverage.insert((record.branch, *domain, *component, tick));
                    }
                }
            }
        }
        coverage
    }

    #[test]
    fn p1_round_trip_byte_identity_for_every_witness_shape() {
        let witnesses = [
            GapWitness::Counter {
                counter_id: 255,
                observed: 256,
            },
            GapWitness::StatusBits {
                bit_width: 9,
                bits: vec![0x34, 0x01],
            },
            GapWitness::Coverage {
                observed: 2,
                expected: 3,
            },
            GapWitness::TickBracket {
                start_tick: 8,
                end_tick: 13,
            },
            GapWitness::TickDiscontinuity {
                previous_tick: 21,
                next_tick: 34,
            },
        ];
        for (index, witness) in witnesses.into_iter().enumerate() {
            let record = GapRecord {
                tick_start: 7,
                tick_end: 10,
                branch: u32::MAX,
                reason: GapReason::from_code(index as u8).unwrap(),
                part: GapPart::Both,
                domains: vec![0, 127, 128, u32::MAX],
                witness,
            };
            let bytes = encode_record(&record).unwrap();
            let decoded = decode_record(&bytes).unwrap();
            assert_eq!(decoded, record);
            assert_eq!(encode_record(&decoded).unwrap(), bytes);
        }
    }

    #[test]
    fn assigned_reason_codes_are_pinned_by_independent_literal_vectors() {
        let vectors: &[(&[u8], GapReason)] = &[
            (&[1, 0, 0, 0, 0, 1, 7, 0, 1, 2], GapReason::TornSweep),
            (
                &[1, 0, 0, 1, 0, 1, 7, 0, 1, 2],
                GapReason::SourceUnavailable,
            ),
            (&[1, 0, 0, 2, 0, 1, 7, 0, 1, 2], GapReason::PartialData),
            (&[1, 0, 0, 3, 0, 1, 7, 0, 1, 2], GapReason::Coalesced),
            (&[1, 0, 0, 4, 0, 1, 7, 0, 1, 2], GapReason::Skipped),
            (
                &[1, 0, 0, 5, 0, 1, 7, 0, 1, 2],
                GapReason::BridgeUnavailable,
            ),
            (&[1, 0, 0, 6, 0, 1, 7, 0, 1, 2], GapReason::WorkerPanic),
            (
                &[1, 0, 0, 7, 0, 1, 7, 0, 1, 2],
                GapReason::CoverageShortfall,
            ),
            (&[1, 0, 0, 8, 0, 1, 7, 0, 1, 2], GapReason::Backpressure),
        ];
        for (literal, reason) in vectors {
            let expected = GapRecord {
                tick_start: 1,
                tick_end: 2,
                branch: 0,
                reason: *reason,
                part: GapPart::State,
                domains: vec![7],
                witness: GapWitness::Counter {
                    counter_id: 1,
                    observed: 2,
                },
            };
            assert_eq!(decode_record(literal).unwrap(), expected);
            assert_eq!(encode_record(&expected).unwrap(), *literal);
        }
    }

    #[test]
    fn assigned_witness_kinds_and_payload_order_are_pinned_by_literal_vectors() {
        let vectors: &[(&[u8], GapWitness)] = &[
            (
                &[1, 0, 0, 0, 0, 1, 7, 0, 1, 2],
                GapWitness::Counter {
                    counter_id: 1,
                    observed: 2,
                },
            ),
            (
                &[1, 0, 0, 0, 0, 1, 7, 1, 9, 0x34, 0x01],
                GapWitness::StatusBits {
                    bit_width: 9,
                    bits: vec![0x34, 0x01],
                },
            ),
            (
                &[1, 0, 0, 0, 0, 1, 7, 2, 2, 3],
                GapWitness::Coverage {
                    observed: 2,
                    expected: 3,
                },
            ),
            (
                &[1, 0, 0, 0, 0, 1, 7, 3, 8, 13],
                GapWitness::TickBracket {
                    start_tick: 8,
                    end_tick: 13,
                },
            ),
            (
                &[1, 0, 0, 0, 0, 1, 7, 4, 21, 34],
                GapWitness::TickDiscontinuity {
                    previous_tick: 21,
                    next_tick: 34,
                },
            ),
        ];
        for (literal, witness) in vectors {
            let expected = GapRecord {
                tick_start: 1,
                tick_end: 2,
                branch: 0,
                reason: GapReason::TornSweep,
                part: GapPart::State,
                domains: vec![7],
                witness: witness.clone(),
            };
            assert_eq!(decode_record(literal).unwrap(), expected);
            assert_eq!(encode_record(&expected).unwrap(), *literal);
        }
    }

    #[test]
    fn status_bits_zero_unused_high_bits_for_every_partial_byte_width() {
        for used_bits in 1u8..=7 {
            let valid_last_byte = (1u8 << used_bits) - 1;
            let invalid_last_byte = valid_last_byte | (1u8 << used_bits);
            let mut record = GapRecord {
                tick_start: 1,
                tick_end: 2,
                branch: 0,
                reason: GapReason::TornSweep,
                part: GapPart::State,
                domains: vec![7],
                witness: GapWitness::StatusBits {
                    bit_width: u64::from(used_bits),
                    bits: vec![valid_last_byte],
                },
            };

            let encoded = encode_record(&record).unwrap();
            assert_eq!(decode_record(&encoded).unwrap(), record);

            record.witness = GapWitness::StatusBits {
                bit_width: u64::from(used_bits),
                bits: vec![invalid_last_byte],
            };
            assert_eq!(
                encode_record(&record),
                Err(WireError::Corrupt(
                    "status witness has non-zero unused high bits"
                ))
            );
        }
    }

    #[test]
    fn status_bits_literal_malformed_pad_vectors_fail_closed_on_decode() {
        let malformed: &[&[u8]] = &[
            &[1, 0, 0, 0, 0, 1, 7, 1, 1, 0x03],
            &[1, 0, 0, 0, 0, 1, 7, 1, 2, 0x07],
            &[1, 0, 0, 0, 0, 1, 7, 1, 3, 0x0f],
            &[1, 0, 0, 0, 0, 1, 7, 1, 4, 0x1f],
            &[1, 0, 0, 0, 0, 1, 7, 1, 5, 0x3f],
            &[1, 0, 0, 0, 0, 1, 7, 1, 6, 0x7f],
            &[1, 0, 0, 0, 0, 1, 7, 1, 7, 0xff],
            &[1, 0, 0, 0, 0, 1, 7, 1, 9, 0x34, 0x81],
        ];
        for literal in malformed {
            assert_eq!(
                decode_record(literal),
                Err(WireError::Corrupt(
                    "status witness has non-zero unused high bits"
                ))
            );

            let mut valid_neighbour = literal.to_vec();
            let bit_width = valid_neighbour[8];
            let used_bits = bit_width % 8;
            let valid_mask = (1u8 << used_bits) - 1;
            *valid_neighbour.last_mut().unwrap() &= valid_mask;
            assert!(
                decode_record(&valid_neighbour).is_ok(),
                "clearing only the unused high bits must make width {bit_width} valid"
            );
        }
    }

    #[test]
    fn p2_domain_list_is_strictly_ascending_and_delta_encoded() {
        let record = counter(
            GapReason::Skipped,
            GapPart::State,
            vec![127, 128, 384],
            1,
            2,
        );
        let bytes = encode_record(&record).unwrap();
        assert!(bytes
            .windows(5)
            .any(|window| window == [3, 127, 1, 0x80, 2]));
        let mut repeated = bytes;
        let count_pos = 5;
        let second_domain_delta = count_pos + 2;
        repeated[second_domain_delta] = 0;
        assert_eq!(
            decode_record(&repeated),
            Err(WireError::Corrupt("gap domain is repeated"))
        );
        let unsorted = counter(GapReason::Skipped, GapPart::State, vec![2, 1], 1, 2);
        assert!(encode_record(&unsorted).is_err());
    }

    #[test]
    fn p3_empty_domain_list_is_rejected_not_inherited_as_h4gp_all_domains() {
        // compact_contract/src/gap_record.rs:100 defines the retired H4GP empty
        // list as "every domain". gap contract has no sentinel; inheriting it would widen
        // one empty record across the whole telemetry surface.
        let empty = counter(GapReason::Skipped, GapPart::State, vec![], 1, 2);
        assert_eq!(
            encode_record(&empty),
            Err(WireError::Corrupt("gap domain list is empty"))
        );
        assert_eq!(
            decode_record(&[1, 0, 0, 4, 0, 0]),
            Err(WireError::Corrupt("gap domain list is empty"))
        );
    }

    #[test]
    fn p4_every_assigned_bound_and_witness_shape_is_enforced() {
        let good =
            encode_record(&counter(GapReason::Skipped, GapPart::State, vec![1], 1, 2)).unwrap();
        let mut branch = Vec::new();
        write_varint(&mut branch, 1);
        write_varint(&mut branch, 0);
        write_varint(&mut branch, u64::from(u32::MAX) + 1);
        branch.extend_from_slice(&good[3..]);
        assert!(decode_record(&branch).is_err());

        let mut reason = good.clone();
        reason[3] = 9;
        assert!(decode_record(&reason).is_err());
        let mut part = good.clone();
        part[4] = 3;
        assert!(decode_record(&part).is_err());
        let mut witness_kind = good.clone();
        witness_kind[7] = 5;
        assert!(decode_record(&witness_kind).is_err());

        let over_domain = GapRecord {
            domains: vec![u32::MAX],
            ..counter(GapReason::Skipped, GapPart::State, vec![1], 1, 2)
        };
        assert!(decode_record(&encode_record(&over_domain).unwrap()).is_ok());
        let mut domain_too_large = vec![1, 0, 0, 4, 0, 1];
        write_varint(&mut domain_too_large, u64::from(u32::MAX) + 1);
        domain_too_large.extend_from_slice(&[0, 1, 1]);
        assert!(decode_record(&domain_too_large).is_err());

        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(
            decode_record(&trailing),
            Err(WireError::Corrupt("trailing bytes after gap witness"))
        );
        assert!(decode_record(&good[..good.len() - 1]).is_err());
        let bad_status = GapWitness::StatusBits {
            bit_width: 9,
            bits: vec![0],
        };
        assert!(bad_status.canonical_payload().is_err());
    }

    #[test]
    fn p5_public_entry_points_never_panic_on_arbitrary_bytes_or_counts() {
        let huge_count = [
            0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 1,
        ];
        assert!(catch_unwind(|| decode_record(&huge_count)).is_ok());

        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for len in 0..=96usize {
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                seed ^= seed << 7;
                seed ^= seed >> 9;
                seed ^= seed << 8;
                *byte = seed as u8;
            }
            assert!(catch_unwind(AssertUnwindSafe(|| decode_record(&bytes))).is_ok());
            for bit in 0..bytes.len().min(16) * 8 {
                bytes[bit / 8] ^= 1 << (bit % 8);
                assert!(catch_unwind(AssertUnwindSafe(|| decode_record(&bytes))).is_ok());
                bytes[bit / 8] ^= 1 << (bit % 8);
            }
        }

        let mut overflow = Vec::new();
        write_varint(&mut overflow, u64::MAX);
        write_varint(&mut overflow, 0);
        overflow.extend_from_slice(&[0, 0, 0, 1, 1, 0, 1]);
        assert_eq!(
            decode_record(&overflow),
            Err(WireError::Corrupt("gap half-open range overflows u64"))
        );

        let malformed = GapRecord {
            tick_start: u64::MAX,
            tick_end: 0,
            branch: u32::MAX,
            reason: GapReason::Backpressure,
            part: GapPart::Both,
            domains: vec![],
            witness: GapWitness::StatusBits {
                bit_width: u64::MAX,
                bits: vec![],
            },
        };
        assert!(catch_unwind(AssertUnwindSafe(|| encode_record(&malformed))).is_ok());
        assert!(catch_unwind(AssertUnwindSafe(|| normalise(&[malformed.clone()]))).is_ok());
        assert!(catch_unwind(AssertUnwindSafe(|| {
            state_gap_intersects(&[malformed.clone()], u32::MAX, u32::MAX, 9..1)
        }))
        .is_ok());
        assert!(catch_unwind(AssertUnwindSafe(|| malformed.witness.canonical_payload())).is_ok());
    }

    #[test]
    fn p6_wire_ranges_are_inclusive_and_memory_ranges_are_half_open() {
        let one = counter(GapReason::Skipped, GapPart::State, vec![1], 9, 10);
        let one_bytes = encode_record(&one).unwrap();
        assert_eq!(one_bytes[1], 0, "end delta zero is one tick");
        assert_eq!(decode_record(&one_bytes).unwrap().range(), 9..10);

        let maximal = counter(GapReason::Skipped, GapPart::State, vec![1], 0, u64::MAX);
        assert_eq!(
            decode_record(&encode_record(&maximal).unwrap()).unwrap(),
            maximal
        );
    }

    #[test]
    fn p7_coverage_preservation_masks_only_the_overlap_not_the_loser_wholesale() {
        let winner = counter(GapReason::SourceUnavailable, GapPart::State, vec![17], 3, 5);
        let loser = counter(GapReason::CoverageShortfall, GapPart::State, vec![17], 1, 7);
        let input = vec![loser, winner];
        let output = normalise(&input).unwrap();
        assert_eq!(component_coverage(&output), component_coverage(&input));
        assert!(output
            .iter()
            .any(|record| record.tick_start == 1 && record.tick_end == 3));
        assert!(output
            .iter()
            .any(|record| record.tick_start == 5 && record.tick_end == 7));
    }

    #[test]
    fn p8_cause_order_uses_canonical_witness_bytes_for_counter_255_vs_256() {
        let make = |counter_id| GapRecord {
            witness: GapWitness::Counter {
                counter_id,
                observed: 0,
            },
            ..counter(GapReason::Coalesced, GapPart::State, vec![1], 2, 4)
        };
        let output = normalise(&[make(255), make(256)]).unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(
            output[0].witness,
            GapWitness::Counter {
                counter_id: 256,
                observed: 0
            },
            "80 02 sorts before FF 01 even though 256 is numerically larger"
        );

        let low_reason = counter(GapReason::TornSweep, GapPart::State, vec![2], 2, 4);
        let high_reason = counter(GapReason::Backpressure, GapPart::State, vec![2], 2, 4);
        assert_eq!(
            normalise(&[high_reason, low_reason]).unwrap()[0].reason,
            GapReason::TornSweep
        );

        let low_kind = counter(GapReason::Coalesced, GapPart::State, vec![3], 2, 4);
        let high_kind = GapRecord {
            witness: GapWitness::Coverage {
                observed: 0,
                expected: 1,
            },
            ..low_kind.clone()
        };
        assert_eq!(
            normalise(&[high_kind, low_kind]).unwrap()[0].witness.kind(),
            0
        );
    }

    #[test]
    fn p9_equal_causes_are_coalesced_to_maximal_spans() {
        let output = normalise(&[
            counter(GapReason::Skipped, GapPart::State, vec![1], 1, 3),
            counter(GapReason::Skipped, GapPart::State, vec![1], 3, 5),
            counter(GapReason::Skipped, GapPart::State, vec![1], 2, 4),
        ])
        .unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].range(), 1..5);
    }

    #[test]
    fn p10_output_never_overlaps_for_one_branch_domain_component() {
        let output = normalise(&[
            counter(GapReason::Skipped, GapPart::Both, vec![1], 1, 8),
            counter(GapReason::PartialData, GapPart::State, vec![1], 3, 5),
            counter(GapReason::TornSweep, GapPart::Event, vec![1], 4, 6),
        ])
        .unwrap();
        for component in [Component::State, Component::Event] {
            for tick in 1..8 {
                let covering = output
                    .iter()
                    .filter(|record| {
                        record.domains.contains(&1)
                            && record.tick_start <= tick
                            && tick < record.tick_end
                            && match record.part {
                                GapPart::State => component == Component::State,
                                GapPart::Event => component == Component::Event,
                                GapPart::Both => true,
                            }
                    })
                    .count();
                assert!(
                    covering <= 1,
                    "{component:?} tick {tick} covered {covering} times"
                );
            }
        }
    }

    #[test]
    fn p11_grouping_requires_exact_range_and_cause_equality() {
        let same = counter(GapReason::Skipped, GapPart::State, vec![1, 2], 1, 4);
        let different_range = counter(GapReason::Skipped, GapPart::State, vec![3], 1, 5);
        let different_cause = counter(GapReason::CoverageShortfall, GapPart::State, vec![4], 1, 4);
        let output = normalise(&[same, different_range, different_cause]).unwrap();
        assert!(output.iter().any(|record| record.domains == vec![1, 2]));
        assert!(!output
            .iter()
            .any(|record| record.domains.contains(&3) && record.domains.len() > 1));
        assert!(!output
            .iter()
            .any(|record| record.domains.contains(&4) && record.domains.len() > 1));
    }

    #[test]
    fn p12_part_two_splits_into_components_and_recombines_only_under_one_group() {
        let both = counter(GapReason::Skipped, GapPart::Both, vec![1], 1, 4);
        let state = counter(GapReason::Skipped, GapPart::State, vec![2], 1, 4);
        let event_other = counter(GapReason::CoverageShortfall, GapPart::Event, vec![2], 1, 4);
        let output = normalise(&[both, state, event_other]).unwrap();
        assert!(output
            .iter()
            .any(|record| record.part == GapPart::Both && record.domains == vec![1]));
        assert!(!output
            .iter()
            .any(|record| record.part == GapPart::Both && record.domains.contains(&2)));
    }

    #[test]
    fn p13_sort_order_and_multiset_determinism_are_byte_exact() {
        let a = counter(GapReason::Skipped, GapPart::State, vec![2], 5, 7);
        let mut b = counter(GapReason::PartialData, GapPart::Event, vec![1], 1, 3);
        b.branch = 2;
        let canonical = normalise(&[a.clone(), b.clone()]).unwrap();
        let permuted = normalise(&[b.clone(), a.clone(), b, a]).unwrap();
        assert_eq!(encoded_stream(&canonical), encoded_stream(&permuted));
        assert_eq!(
            encoded_stream(&normalise(&canonical).unwrap()),
            encoded_stream(&canonical)
        );
        assert!(canonical.windows(2).all(|pair| {
            (
                pair[0].tick_start,
                pair[0].branch,
                pair[0].reason,
                pair[0].part,
                pair[0].domains[0],
            ) <= (
                pair[1].tick_start,
                pair[1].branch,
                pair[1].reason,
                pair[1].part,
                pair[1].domains[0],
            )
        }));
    }

    #[test]
    fn p14_branches_are_separate_partitions_in_one_sorted_stream() {
        let mut branch_1 = counter(GapReason::Skipped, GapPart::State, vec![1], 1, 4);
        branch_1.branch = 1;
        let mut branch_2 = branch_1.clone();
        branch_2.branch = 2;
        let output = normalise(&[branch_2, branch_1]).unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].branch, 1);
        assert_eq!(output[1].branch, 2);
    }

    #[test]
    fn p15_spec_worked_example_is_two_records_not_three_slices() {
        let a = counter(GapReason::Skipped, GapPart::State, vec![1], 1, 4);
        let b = counter(GapReason::Skipped, GapPart::State, vec![2], 2, 5);
        let output = normalise(&[a, b]).unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].domains, vec![1]);
        assert_eq!(output[0].range(), 1..4);
        assert_eq!(output[1].domains, vec![2]);
        assert_eq!(output[1].range(), 2..5);
    }

    #[test]
    fn p16_state_affecting_query_accepts_parts_zero_and_two_but_not_one() {
        for (part, expected) in [
            (GapPart::State, true),
            (GapPart::Event, false),
            (GapPart::Both, true),
        ] {
            let records = normalise(&[counter(GapReason::Skipped, part, vec![7], 10, 12)]).unwrap();
            assert_eq!(state_gap_intersects(&records, 0, 7, 11..13), expected);
            assert!(!state_gap_intersects(&records, 1, 7, 11..13));
            assert!(!state_gap_intersects(&records, 0, 8, 11..13));
            assert!(!state_gap_intersects(&records, 0, 7, 12..13));
        }
    }

    #[test]
    fn p17_gap_query_matches_the_shipped_baseline_predicate() {
        let measured = [
            0u64,
            1,
            2,
            3,
            5,
            6,
            10,
            11,
            12,
            i32::MAX as u64 - 1,
            i32::MAX as u64,
        ];
        let mut observations = Vec::new();
        for pair in measured.windows(2) {
            if pair[1] != pair[0].saturating_add(1) {
                observations.push(counter(
                    GapReason::Skipped,
                    GapPart::State,
                    vec![50],
                    pair[0].saturating_add(1),
                    pair[1],
                ));
            }
        }
        let gaps = normalise(&observations).unwrap();
        let block_hours = 4usize;
        let mut gap_forced = 0usize;
        for (index, hour) in measured.iter().copied().enumerate() {
            let previous = index.checked_sub(1).map(|i| measured[i]);
            let shipped_gap = previous.is_some_and(|p| hour != p.saturating_add(1));
            let queried_gap = previous
                .is_some_and(|p| state_gap_intersects(&gaps, 0, 50, p.saturating_add(1)..hour));
            assert_eq!(queried_gap, shipped_gap, "hour {hour}");
            let block_start = index % block_hours == 0;
            let shipped_baseline = block_start || shipped_gap || previous.is_none();
            let query_baseline = block_start || queried_gap || previous.is_none();
            assert_eq!(query_baseline, shipped_baseline);
            if query_baseline && !block_start && queried_gap {
                gap_forced += 1;
            }
        }
        assert_eq!(
            gap_forced, 2,
            "a gap on a block start is counted as the block start"
        );
    }

    #[test]
    fn p18_churn_is_invariant_and_keeps_a_nonzero_cross_gap_transition() {
        // bench_real_d50.rs:1866–:1869:
        // the gap rule decides the ENCODING, not what changed. Churn still
        // compares consecutive measured states across an excluded hour.
        let measured_states = [
            (1u64, vec![Some(1u32), Some(2)]),
            (3, vec![None, Some(2)]),
            (4, vec![None, Some(3)]),
        ];
        let count_churn = |gaps: &[GapRecord], wrongly_suppress_cross_gap: bool| {
            let mut counters = (0usize, 0usize, 0usize, 0usize);
            for pair in measured_states.windows(2) {
                let gap_before =
                    state_gap_intersects(gaps, 0, 50, pair[0].0.saturating_add(1)..pair[1].0);
                if wrongly_suppress_cross_gap && gap_before {
                    continue;
                }
                counters.0 += 1;
                let mut changed_this_hour = false;
                for (before, after) in pair[0].1.iter().zip(&pair[1].1) {
                    if before != after {
                        counters.2 += 1;
                        changed_this_hour = true;
                    }
                    if before.is_some() && after.is_none() {
                        counters.3 += 1;
                    }
                }
                counters.1 += usize::from(changed_this_hour);
            }
            counters
        };
        // This is the contract under test: the gap stream reaches the churn
        // function at its signature, but only the encoding path may act on it.
        let churn = |gaps: &[GapRecord]| count_churn(gaps, false);
        let gap_a = counter(GapReason::Skipped, GapPart::State, vec![50], 2, 3);
        let gap_b = counter(GapReason::Skipped, GapPart::State, vec![51], 2, 3);
        let ungrouped = normalise(&[gap_a.clone(), gap_b.clone()]).unwrap();
        let grouped = normalise(&[GapRecord {
            domains: vec![50, 51],
            ..gap_a
        }])
        .unwrap();
        let without_gap_stream = churn(&[]);
        let with_ungrouped_stream = churn(&ungrouped);
        let with_grouped_stream = churn(&grouped);
        assert_eq!(without_gap_stream, (2, 2, 2, 1));
        assert_eq!(with_ungrouped_stream, without_gap_stream);
        assert_eq!(with_grouped_stream, without_gap_stream);
        assert_ne!(
            count_churn(&ungrouped, true),
            without_gap_stream,
            "the fixture must turn red if a gap suppresses churn"
        );
        assert_eq!(component_coverage(&ungrouped), component_coverage(&grouped));
        assert_eq!(
            (with_grouped_stream.2, with_grouped_stream.3),
            (2, 1),
            "the cross-gap changed rows and tombstone remain counted"
        );
    }
}
