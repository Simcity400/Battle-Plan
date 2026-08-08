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
    DOMAIN_GAP_REASON_BACKPRESSURE, DOMAIN_GAP_REASON_CLOCK_UNREADABLE,
    DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH, DOMAIN_GAP_REASON_OVERRUN, DOMAIN_GAP_REASON_READER_SILENT,
    DOMAIN_GAP_REASON_SOURCE_UNAVAILABLE, DOMAIN_GAP_REASON_TORN, DOMAIN_GAP_REASON_WORKER_PANIC,
    DOMAIN_GAP_WITNESS_PROVENANCE_BRACKET_OBSERVED, DOMAIN_GAP_WITNESS_PROVENANCE_UNOBSERVED,
    DOMAIN_GAP_WITNESS_PROVENANCE_UNSTATED,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum DomainGapReason {
    Torn = DOMAIN_GAP_REASON_TORN,
    Overrun = DOMAIN_GAP_REASON_OVERRUN,
    Backpressure = DOMAIN_GAP_REASON_BACKPRESSURE,
    ClockUnreadable = DOMAIN_GAP_REASON_CLOCK_UNREADABLE,
    WorkerPanic = DOMAIN_GAP_REASON_WORKER_PANIC,
    /// The reader completed and could not produce a payload. Distinct from
    /// `ClockUnreadable`: the clock was read, the SOURCE was unavailable.
    SourceUnavailable = DOMAIN_GAP_REASON_SOURCE_UNAVAILABLE,
    /// The domain was scheduled and the frame sealed with neither a DATA
    /// record nor a gap from the domain itself. Minted by the seal so the hour
    /// is named loss instead of an absence nothing can detect.
    ReaderSilent = DOMAIN_GAP_REASON_READER_SILENT,
    /// The reader's bracket did not tear, and did not observe the frame's hour
    /// either. See [`DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH`] for why a bracket
    /// alone cannot catch this and what the witnesses prove.
    HourLabelMismatch = DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH,
}

impl DomainGapReason {
    /// Every reason, for consumers that must handle all of them but cannot be
    /// forced to by a `match` -- report tables keyed by name, mask decoders,
    /// round-trip coverage. Those are hand-maintained lists, and a hand list
    /// checked against another hand list proves nothing.
    ///
    /// What makes this one complete is `all_covers_every_decodable_reason`,
    /// which rebuilds the set from [`DomainGapReason::from_code`] by sweeping
    /// the whole u16 code space. `from_code` is the only way a reason enters the
    /// process from the wire, so a variant that can arrive and is missing here
    /// fails that test -- including the case `position` alone cannot see, where
    /// a new variant takes the next free position and is simply never added to
    /// this array.
    ///
    /// The claim is therefore exactly: complete for every `from_code`-decodable
    /// reason. It is NOT a claim about every declared variant. A variant added
    /// to neither this array nor `from_code` is invisible to the test -- but it
    /// is also unreachable by any consumer, because every one of them reaches a
    /// reason by decoding a record, and `decode` rejects an unknown code at
    /// `from_code`. Such a variant would be an encoder/decoder asymmetry (the
    /// contract encoding a payload its own decoder refuses), which is a
    /// different defect from the one this array exists to prevent.
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

    /// Declared slot in [`DomainGapReason::ALL`]. The match is exhaustive, so a
    /// new variant does not compile until it is given a slot; `all_is_exhaustive`
    /// then pins each entry of `ALL` to its own slot, which is what catches a
    /// variant listed twice or in the wrong order.
    ///
    /// This alone does NOT prove `ALL` is complete -- a new variant given the
    /// next free slot and never added to the array leaves both this match and
    /// that test satisfied. `all_covers_every_decodable_reason` is what closes
    /// that, by rebuilding the set from the wire decoder instead of from a list.
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

    /// The reason's canonical name outside the wire: the string the sealed
    /// manifest writes and every offline consumer matches on.
    ///
    /// IT LIVES HERE BECAUSE IT WAS TWO HAND LISTS (lane FJ, 2026-08-08).
    /// `recorder_host` rendered these names for the manifest, and
    /// `scripts/verify_sink_reconciliation.py` kept its own admission set of
    /// them -- which had gone stale at five entries and reported a manifest
    /// carrying `source_unavailable`, `reader_silent` or `hour_label_mismatch`
    /// as `domain_gap_N_reason_invalid`. A name the producer is entitled to emit
    /// and a verifier calls invalid is a capture failed for being honest.
    ///
    /// One authority, derived by both: this match cannot compile without an arm
    /// for a new variant, and
    /// `verify_sink_reconciliation_admits_exactly_the_contract_gap_reasons`
    /// rebuilds the script's set from [`DomainGapReason::ALL`] mapped through
    /// here.
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

/// Where a record's two tick witnesses came from, as the producer states it.
///
/// See [`DOMAIN_GAP_WITNESS_PROVENANCE_UNSTATED`] for what this field exists to
/// close and why zero is `Unstated`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u16)]
pub enum DomainGapWitnessProvenance {
    /// THE DEFAULT, AND THE ONLY HONEST ONE FOR A PRODUCER THAT HAS NOT BEEN
    /// CONVERTED. A consumer must not read these ticks as an independent
    /// observation on the strength of this value alone.
    #[default]
    Unstated = DOMAIN_GAP_WITNESS_PROVENANCE_UNSTATED,
    /// The ticks are hours a reader's own clock bracket observed.
    BracketObserved = DOMAIN_GAP_WITNESS_PROVENANCE_BRACKET_OBSERVED,
    /// No observation reached the mint site; the ticks are the wire's zero.
    Unobserved = DOMAIN_GAP_WITNESS_PROVENANCE_UNOBSERVED,
}

impl DomainGapWitnessProvenance {
    /// Every provenance, for the same reason [`DomainGapReason::ALL`] exists and
    /// with the same proof: `the_provenance_vocabulary_is_complete` rebuilds
    /// this set from [`DomainGapWitnessProvenance::from_code`] over the whole
    /// `u16` space, so a value that can arrive off the wire and is missing here
    /// fails rather than being silently dropped by every consumer that iterates.
    pub const ALL: [DomainGapWitnessProvenance; 3] =
        [Self::Unstated, Self::BracketObserved, Self::Unobserved];

    pub const fn code(self) -> u16 {
        self as u16
    }

    /// The name offline consumers match on, exactly as [`DomainGapReason::wire_name`].
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Unstated => "unstated",
            Self::BracketObserved => "bracket_observed",
            Self::Unobserved => "unobserved",
        }
    }

    /// Whether these ticks are an ENGINE OBSERVATION this record's producer
    /// vouches for.
    ///
    /// THE ONE PLACE the question is answered, so a consumer cannot decide it
    /// with its own spelling. `Unstated` is false and that is the whole point:
    /// a capture sealed before the field existed may not be read as if it had
    /// made a claim it never made.
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
    /// Encode STATING NO PROVENANCE for the tick witnesses.
    ///
    /// This is the honest encoding for every producer that has not been
    /// converted, and it is byte-identical to what this contract emitted before
    /// the field existed. A producer that KNOWS where its ticks came from must
    /// say so with [`DomainGapRecordV1::encode_with_witness_provenance`];
    /// forgetting to costs an offline consumer the right to believe the
    /// witnesses, which is the safe direction — the reverse would be a claim
    /// nothing made.
    pub fn encode(&self) -> Result<Vec<u8>, DomainGapError> {
        self.encode_with_witness_provenance(DomainGapWitnessProvenance::Unstated)
    }

    /// Encode, stating where the two tick witnesses came from.
    ///
    /// The grammar refuses a provenance that contradicts the witnesses it
    /// describes (see [`validate_domain_gap_payload`]), so a wrong stamp fails
    /// here rather than shipping.
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

    /// The provenance stamped on an encoded payload.
    ///
    /// A SEPARATE READER, and not a field of [`DomainGapRecordV1`], for the
    /// same reason [`crate::record_semantics::domain_gap_target_hour_raw`] is
    /// one: every one of the sixty-odd construction sites in this tree would
    /// otherwise have to state a provenance it does not know, and a value
    /// nobody knows is a value everybody defaults. The producers that DO know
    /// state it at `encode`; the consumers that need it ask for it here.
    ///
    /// Validates the whole payload first, so a provenance can never be read off
    /// bytes the grammar would refuse.
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
                // The reader held a bracket, so it carries a real non-advancing
                // hour witness.
                DomainGapReason::SourceUnavailable => (hour(0), hour(0), 103, 103, 0, 0, 0),
                // No witness of any kind exists; both ticks must be zero.
                DomainGapReason::ReaderSilent => (hour(0), hour(0), 0, 0, 0, 0, 0),
                // The bracket held steady on an hour that is NOT the target.
                // Both witnesses carry the hour actually observed, which is
                // the whole content of the reason.
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

    /// `ALL` is complete, not merely current.
    ///
    /// `position` is an exhaustive match, so a new variant cannot compile
    /// without a position; this pins every position to a distinct slot of
    /// `ALL`, so a variant that has a position but is missing from `ALL` -- or
    /// listed twice -- fails here. Consumers that iterate `ALL` inherit that
    /// guarantee instead of maintaining a second hand list.
    /// `ALL` covers everything that can arrive off the wire.
    ///
    /// Rebuilt from `from_code` over the entire u16 space rather than compared
    /// against a second hand-written list: a list checked against a list is
    /// satisfied by forgetting the variant in both. This is the check that
    /// catches the shape `position` cannot -- an eighth variant that takes slot
    /// 7, decodes fine, and is never added to `ALL`. Every consumer that
    /// iterates `ALL` to name or route a reason depends on this.
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

    /// The payload grammar and the typed decoder admit the SAME reason codes.
    ///
    /// There are two admission lists, not one: `DomainGapReason::from_code`
    /// above, and the `matches!` in `validate_domain_gap_payload`. Every decode
    /// runs both, so a code admitted by one and refused by the other is not a
    /// second opinion -- it is a reason that either cannot be decoded despite
    /// being on the wire, or (the dangerous direction) passes the payload
    /// grammar's reason check and then falls through to whichever witness arm
    /// happens to match. `all_covers_every_decodable_reason` proves `ALL` is
    /// complete against `from_code`; nothing proved `from_code` complete against
    /// the grammar.
    ///
    /// Swept over the entire u16 space rather than over the assigned names, for
    /// the same reason: a list checked against a list is satisfied by forgetting
    /// the code in both.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: adding a reason constant to
    /// `from_code` without adding it to the grammar's `matches!` (that code then
    /// decodes to a typed reason the payload validator calls unknown), or the
    /// reverse (the grammar admits a code no consumer can name).
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

    /// Driven from [`DomainGapReason::ALL`] rather than from a transcribed
    /// list.
    ///
    /// It WAS a transcribed list, and it went stale the moment a reason was
    /// added: `HourLabelMismatch` was missing, so deleting its three queue-zero
    /// predicates from the grammar would have been caught by nothing. `ALL` is
    /// proved complete against the wire decoder by
    /// `all_covers_every_decodable_reason`, so a reason cannot be omitted from
    /// this sweep without first being unreachable.
    ///
    /// The provenance vocabulary is complete against the wire, exactly as
    /// `all_covers_every_decodable_reason` proves `DomainGapReason::ALL` is.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: assigning a fourth provenance
    /// code in `from_code` (or in the grammar's `matches!`) without adding it to
    /// `ALL`, which would leave every consumer that iterates the vocabulary
    /// silently dropping it.
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

    /// The payload grammar and the typed decoder admit the SAME provenance
    /// codes, swept over the whole `u16` space.
    ///
    /// The same defect `the_payload_grammar_admits_exactly_the_codes_from_code_decodes`
    /// exists for this field: a code the grammar admits and `from_code` refuses
    /// is a byte on the wire nothing can name, and the reverse is a claim that
    /// passes the grammar and then falls through to whichever provenance rule
    /// happens to match.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: adding a constant to one
    /// admission list and not the other.
    #[test]
    fn the_grammar_admits_exactly_the_provenance_codes_from_code_decodes() {
        // Torn's sample carries two nonzero, unequal ticks, so neither
        // provenance rule can refuse it and the only thing under test is the
        // code's admission.
        let base = sample(DomainGapReason::Torn).encode().unwrap();
        for code in 0..=u16::MAX {
            let mut wire = base.clone();
            wire[12..14].copy_from_slice(&code.to_le_bytes());
            let grammar_calls_it_unknown = validate_domain_gap_payload(&wire)
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

    /// Every reason round-trips under every provenance its witnesses can
    /// support, and `encode` is byte-identical to `encode_with_witness_provenance(Unstated)`.
    ///
    /// The second half is what keeps a capture sealed before this field existed
    /// readable: `Unstated` IS the zero the old producer wrote and the old
    /// decoder required.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: spending the second half of the
    /// reserved word (bytes 14..16), moving the provenance to another offset, or
    /// giving `encode` a default other than `Unstated`.
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

            // The provenance the witnesses DO support, decided by the witnesses
            // and not by the reason, so a reason whose sample changes shape is
            // still exercised.
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
            // The record itself is unchanged by the stamp: provenance describes
            // the witnesses, it does not alter them.
            assert_eq!(DomainGapRecordV1::decode(&stamped).unwrap(), value);
        }
    }

    /// A provenance that contradicts its own witnesses does not encode.
    ///
    /// This is the whole strength of the field. Without these two rules a
    /// producer could stamp `BracketObserved` on `ReaderSilent`'s required zeros
    /// and `hour_label_audit` would report "the reader observed hour 0", or
    /// stamp `Unobserved` on a real reading and hide it.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: deleting either conjunct of the
    /// provenance match in `validate_domain_gap_payload`.
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
            matches!(unobserved_on_reading, Err(DomainGapError::InvalidSemantics(_))),
            "a record carrying a real reading cannot claim it observed nothing"
        );

        // And the same two rules hold on DECODE, over bytes a hostile or
        // corrupt producer could have written directly.
        let mut forged = sample(DomainGapReason::ReaderSilent).encode().unwrap();
        forged[12..14]
            .copy_from_slice(&DomainGapWitnessProvenance::BracketObserved.code().to_le_bytes());
        assert!(matches!(
            DomainGapRecordV1::decode_witness_provenance(&forged),
            Err(DomainGapError::InvalidSemantics(_))
        ));
        assert!(matches!(
            DomainGapRecordV1::decode(&forged),
            Err(DomainGapError::InvalidSemantics(_))
        ));
    }

    /// The second half of the first reserved word is STILL reserved.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: widening the provenance to the
    /// whole `u32` word, which would silently admit bytes the next field wants.
    #[test]
    fn the_unspent_half_of_the_reserved_word_is_still_refused() {
        let mut wire = sample(DomainGapReason::Torn).encode().unwrap();
        wire[14..16].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            validate_domain_gap_payload(&wire),
            Err("DOMAIN_GAP reserved field is nonzero".to_string())
        );
    }

    /// Backpressure is the single exclusion, stated as a `match` so that a new
    /// reason has to be classified rather than silently inheriting either side.
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
