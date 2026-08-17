//! Stateful, fail-closed validation for one complete H4CR stream.
//!
//! Identity-bearing and legacy streams are different protocols selected by an
//! explicit constructor.  This module never sniffs the first payload and never
//! falls back between those protocols.

use std::collections::BTreeMap;

use crate::campaign_identity::{parse_cid1_record, Cid1Error, ValidatedCid1};
use crate::domain_gap::DomainGapRecordV1;
use crate::freeze_census_record::{
    validate_freeze_census_placement_policy, validate_record_allowed_after_release_epilogue,
    FreezeCensusPlacement, FreezeCensusRecordV1,
};
use crate::record_header::{parse_compact_record_header, CompactRecordHeader, COMPACT_RECORD_HEADER_LEN};
use crate::record_semantics::{
    validate_compact_record_semantics, KnownRecordType, COMPACT_DOMAIN_FRAME,
    COMPACT_RECORD_TYPE_DOMAIN_GAP, COMPACT_RECORD_TYPE_STREAM_HEADER,
};

/// Validate the H4FC placement slice of one H4CR byte stream's stateful grammar.
///
/// This helper exists so secondary whole-stream scanners cannot accidentally
/// validate only each record's stateless payload while accepting a late,
/// duplicate, misplaced, or hour-bearing H4FC. Every such scanner calls it, so
/// it is the shipped definition of the epilogue grammar for all of them.
///
/// It runs a DEDICATED placement scan rather than the full stateful validator.
/// The full validator stops at the first rule any record breaks, and it used to
/// return `Ok(())` when that rule was not an H4FC rule — on the assumption that
/// the calling scanner independently refuses the stream for the other reason.
/// That assumption does not hold in general: a scanner that tolerates something
/// the stateful validator rejects (reserved domain zero, for instance) would
/// then get a silent pass on the H4FC rules AND accept the stream, which is the
/// loophole the owner ruling was written to avoid. This scan therefore never
/// short-circuits on a semantic rule it does not own.
///
/// It stops only on a FRAMING failure — a header that will not parse, or a
/// declared length that overruns the buffer. Past that point there are no
/// further records to locate, and every caller reports its own framing error
/// and refuses the stream, so stopping there cannot let a stream through.
pub fn validate_freeze_census_stream_placement(raw: &[u8]) -> Result<(), String> {
    if raw.is_empty() {
        return Ok(());
    }
    let identity_present = parse_compact_record_header(raw)
        .map(|first| first.record_type == COMPACT_RECORD_TYPE_STREAM_HEADER)
        .unwrap_or(false);

    let mut offset = 0usize;
    let mut ordinary_record_count = 0u64;
    let mut content_identity_census_seen = false;
    let mut previous_record_type: Option<KnownRecordType> = None;
    let mut release_epilogue_seen = false;
    while offset < raw.len() {
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(_) => return Ok(()),
        };
        let record_len = header.record_len();
        if record_len > raw.len().saturating_sub(offset) {
            return Ok(());
        }
        let payload = &raw[offset + COMPACT_RECORD_HEADER_LEN..offset + record_len];

        // The EOF half of the grammar: once a release epilogue was accepted,
        // no further physical record may follow it, whatever its type.
        validate_record_allowed_after_release_epilogue(release_epilogue_seen)
            .map_err(|error| format!("offset {offset}: {error}"))?;

        let known = KnownRecordType::try_from(header.record_type).ok();
        if known == Some(KnownRecordType::FreezeCensus) {
            // It closes no hour and claims no coverage, so its envelope must
            // carry domain 1000 and the zero no-coverage sentinel.
            if header.domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "offset {offset}: FREEZE_CENSUS domain {} is not {COMPACT_DOMAIN_FRAME}",
                    header.domain_id
                ));
            }
            if header.game_hour_raw != 0 {
                return Err(format!(
                    "offset {offset}: FREEZE_CENSUS H4CR hour {} is not the zero no-coverage sentinel",
                    header.game_hour_raw
                ));
            }
            if header.payload_len != payload.len() {
                return Err(format!(
                    "offset {offset}: FREEZE_CENSUS declared payload length {} does not equal {}",
                    header.payload_len,
                    payload.len()
                ));
            }
            // Decoding enforces the fixed maximum payload size and every
            // self-consistency rule the record makes about its own counters.
            let census = FreezeCensusRecordV1::decode(payload)
                .map_err(|error| format!("offset {offset}: {error}"))?;
            let placement = validate_freeze_census_placement_policy(
                census.phase(),
                header.sequence,
                ordinary_record_count,
                previous_record_type,
                release_epilogue_seen,
            )
            .map_err(|error| format!("offset {offset}: {error}"))?;
            if placement == FreezeCensusPlacement::ReleaseEpilogue {
                release_epilogue_seen = true;
            }
        }

        if known == Some(KnownRecordType::ContentIdentityCensus) {
            if header.domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "offset {offset}: CONTENT_IDENTITY_CENSUS domain {} is not {COMPACT_DOMAIN_FRAME}",
                    header.domain_id
                ));
            }
            if header.game_hour_raw != 0 {
                return Err(format!(
                    "offset {offset}: CONTENT_IDENTITY_CENSUS hour {} is not the zero no-coverage sentinel",
                    header.game_hour_raw
                ));
            }
            crate::content_identity_census::ContentIdentityCensusV1::decode(payload)
                .map_err(|error| format!("offset {offset}: {error}"))?;
            crate::content_identity_census::validate_content_identity_census_placement(
                ordinary_record_count,
                previous_record_type,
                content_identity_census_seen,
            )
            .map_err(|error| format!("offset {offset}: {error}"))?;
            content_identity_census_seen = true;
        }

        // Sequence zero is the identity record and is not an ordinary record;
        // it is also not a legal predecessor for anything but the install
        // preamble, which the placement policy states directly.
        if !(identity_present && offset == 0) {
            ordinary_record_count = ordinary_record_count.saturating_add(1);
            previous_record_type = known;
        }
        offset += record_len;
    }
    Ok(())
}

/// Protocol selected before any bytes from a stream are accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamIdentityMode {
    /// Sequence zero is the mandatory CID1 stream-header record.
    IdentityV1,
    /// No identity record is present. Ordinary records still begin at sequence one.
    Legacy,
}

/// Classification of a successfully accepted record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptedRecordKind {
    CampaignIdentity,
    Ordinary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamValidationError {
    /// The validator previously failed. The original error is available through
    /// [`StatefulStreamValidator::failure`].
    Terminal,
    InvalidCampaignIdentity(Cid1Error),
    UnexpectedSequence {
        expected: u64,
        actual: u64,
    },
    SequenceExhausted(u64),
    PayloadLengthMismatch {
        declared: usize,
        actual: usize,
    },
    ReservedDomainZero,
    InvalidRecordSemantics(String),
}

impl core::fmt::Display for StreamValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Terminal => write!(
                f,
                "compact stream validator is terminal after an earlier failure"
            ),
            Self::InvalidCampaignIdentity(error) => {
                write!(f, "invalid CID1 stream header: {error}")
            }
            Self::UnexpectedSequence { expected, actual } => write!(
                f,
                "compact record sequence {actual} is not the next required sequence {expected}"
            ),
            Self::SequenceExhausted(sequence) => write!(
                f,
                "compact record sequence {sequence} cannot be advanced without overflow"
            ),
            Self::PayloadLengthMismatch { declared, actual } => write!(
                f,
                "compact record payload length {declared} does not equal supplied length {actual}"
            ),
            Self::ReservedDomainZero => write!(
                f,
                "compact domain zero is reserved exclusively for the CID1 sequence-zero record"
            ),
            Self::InvalidRecordSemantics(error) => {
                write!(f, "invalid compact record semantics: {error}")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamPhase {
    AwaitingIdentity,
    AwaitingOrdinary { next_sequence: u64 },
}

enum PendingCommit {
    Identity(ValidatedCid1),
    Ordinary {
        next_sequence: u64,
        gap_outcome: Option<((u16, u16), GapOutcomeState)>,
        record_type: KnownRecordType,
        release_epilogue: bool,
        content_identity_census: bool,
    },
}

#[derive(Clone, Copy, Debug)]
struct GapOutcomeState {
    next_ordinal: u64,
    last_range_end: i32,
}

/// Stateful authority for one stream. Any rejected record makes the instance
/// permanently terminal; callers must construct a new instance for a new stream.
#[derive(Debug)]
pub struct StatefulStreamValidator {
    mode: StreamIdentityMode,
    phase: StreamPhase,
    identity: Option<ValidatedCid1>,
    ordinary_record_count: u64,
    last_ordinary_record_type: Option<KnownRecordType>,
    release_epilogue_seen: bool,
    /// One per LOGICAL STREAM; a second says the session's content identity
    /// changed mid-recording, which the engine's lexer cannot do.
    content_identity_census_seen: bool,
    gap_outcomes: BTreeMap<(u16, u16), GapOutcomeState>,
    failure: Option<StreamValidationError>,
}

impl StatefulStreamValidator {
    /// Construct an identity-v1 validator. The first accepted record must be the
    /// exact CID1 `(sequence, domain, type) = (0, 0, 4)` record.
    pub fn identity_v1() -> Self {
        Self {
            mode: StreamIdentityMode::IdentityV1,
            phase: StreamPhase::AwaitingIdentity,
            identity: None,
            ordinary_record_count: 0,
            last_ordinary_record_type: None,
            release_epilogue_seen: false,
            content_identity_census_seen: false,
            gap_outcomes: BTreeMap::new(),
            failure: None,
        }
    }

    /// Construct a legacy validator. There is no CID1 record and the first
    /// ordinary record must have sequence one.
    pub fn legacy() -> Self {
        Self {
            mode: StreamIdentityMode::Legacy,
            phase: StreamPhase::AwaitingOrdinary { next_sequence: 1 },
            identity: None,
            ordinary_record_count: 0,
            last_ordinary_record_type: None,
            release_epilogue_seen: false,
            content_identity_census_seen: false,
            gap_outcomes: BTreeMap::new(),
            failure: None,
        }
    }

    pub fn mode(&self) -> StreamIdentityMode {
        self.mode
    }

    /// Return placement-authoritative identity only. A merely syntax-parsed
    /// CID1 can never enter this field.
    pub fn identity(&self) -> Option<&ValidatedCid1> {
        self.identity.as_ref()
    }

    /// Number of accepted ordinary records. CID1 is deliberately excluded.
    pub fn ordinary_record_count(&self) -> u64 {
        self.ordinary_record_count
    }

    pub fn next_sequence(&self) -> u64 {
        match self.phase {
            StreamPhase::AwaitingIdentity => 0,
            StreamPhase::AwaitingOrdinary { next_sequence } => next_sequence,
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.failure.is_some()
    }

    /// The first failure is retained unchanged for diagnostics.
    pub fn failure(&self) -> Option<&StreamValidationError> {
        self.failure.as_ref()
    }

    /// Validate and atomically commit one already-framed record.
    ///
    /// Validation failure commits no identity, sequence, or record-count change;
    /// it only records the first error and makes this validator terminal.
    pub fn validate_record(
        &mut self,
        header: &CompactRecordHeader,
        payload: &[u8],
    ) -> Result<AcceptedRecordKind, StreamValidationError> {
        if self.failure.is_some() {
            return Err(StreamValidationError::Terminal);
        }

        let pending = match self.validate_without_commit(header, payload) {
            Ok(pending) => pending,
            Err(error) => {
                self.failure = Some(error.clone());
                return Err(error);
            }
        };

        match pending {
            PendingCommit::Identity(identity) => {
                self.identity = Some(identity);
                self.phase = StreamPhase::AwaitingOrdinary { next_sequence: 1 };
                Ok(AcceptedRecordKind::CampaignIdentity)
            }
            PendingCommit::Ordinary {
                next_sequence,
                gap_outcome,
                record_type,
                release_epilogue,
                content_identity_census,
            } => {
                self.phase = StreamPhase::AwaitingOrdinary { next_sequence };
                self.ordinary_record_count += 1;
                self.last_ordinary_record_type = Some(record_type);
                self.release_epilogue_seen = release_epilogue;
                self.content_identity_census_seen |= content_identity_census;
                if let Some((key, state)) = gap_outcome {
                    self.gap_outcomes.insert(key, state);
                }
                Ok(AcceptedRecordKind::Ordinary)
            }
        }
    }

    fn validate_without_commit(
        &self,
        header: &CompactRecordHeader,
        payload: &[u8],
    ) -> Result<PendingCommit, StreamValidationError> {
        match self.phase {
            StreamPhase::AwaitingIdentity => parse_cid1_record(header, payload)
                .map(PendingCommit::Identity)
                .map_err(StreamValidationError::InvalidCampaignIdentity),
            StreamPhase::AwaitingOrdinary { next_sequence } => {
                validate_record_allowed_after_release_epilogue(self.release_epilogue_seen)
                    .map_err(StreamValidationError::InvalidRecordSemantics)?;
                if header.sequence != next_sequence {
                    return Err(StreamValidationError::UnexpectedSequence {
                        expected: next_sequence,
                        actual: header.sequence,
                    });
                }
                let following_sequence = header
                    .sequence
                    .checked_add(1)
                    .ok_or(StreamValidationError::SequenceExhausted(header.sequence))?;
                if header.payload_len != payload.len() {
                    return Err(StreamValidationError::PayloadLengthMismatch {
                        declared: header.payload_len,
                        actual: payload.len(),
                    });
                }
                if header.domain_id == 0 {
                    return Err(StreamValidationError::ReservedDomainZero);
                }
                if header.record_type == crate::COMPACT_RECORD_TYPE_CAPTURE_TERMINATED
                    && header.game_hour_raw != 0
                {
                    return Err(StreamValidationError::InvalidRecordSemantics(format!(
                        "CAPTURE_TERMINATED H4CR hour {} is not the zero no-coverage sentinel",
                        header.game_hour_raw
                    )));
                }
                validate_compact_record_semantics(
                    header.domain_id,
                    header.record_type,
                    header.record_flags,
                    payload,
                )
                .map_err(StreamValidationError::InvalidRecordSemantics)?;
                let record_type =
                    KnownRecordType::try_from(header.record_type).map_err(|error| {
                        StreamValidationError::InvalidRecordSemantics(error.to_string())
                    })?;
                let release_epilogue = if record_type == KnownRecordType::FreezeCensus {
                    if header.game_hour_raw != 0 {
                        return Err(StreamValidationError::InvalidRecordSemantics(format!(
                            "FREEZE_CENSUS H4CR hour {} is not the zero no-coverage sentinel",
                            header.game_hour_raw
                        )));
                    }
                    let census = FreezeCensusRecordV1::decode(payload)
                        .map_err(StreamValidationError::InvalidRecordSemantics)?;
                    validate_freeze_census_placement_policy(
                        census.phase(),
                        header.sequence,
                        self.ordinary_record_count,
                        self.last_ordinary_record_type,
                        self.release_epilogue_seen,
                    )
                    .map_err(StreamValidationError::InvalidRecordSemantics)?
                        == FreezeCensusPlacement::ReleaseEpilogue
                } else {
                    false
                };
                let content_identity_census =
                    record_type == KnownRecordType::ContentIdentityCensus;
                if content_identity_census {
                    if header.game_hour_raw != 0 {
                        return Err(StreamValidationError::InvalidRecordSemantics(format!(
                            "CONTENT_IDENTITY_CENSUS H4CR hour {} is not the zero no-coverage sentinel",
                            header.game_hour_raw
                        )));
                    }
                    crate::content_identity_census::validate_content_identity_census_placement(
                        self.ordinary_record_count,
                        self.last_ordinary_record_type,
                        self.content_identity_census_seen,
                    )
                    .map_err(|reason| {
                        StreamValidationError::InvalidRecordSemantics(reason.to_string())
                    })?;
                }
                let gap_outcome = if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_GAP {
                    let gap = DomainGapRecordV1::decode(payload).map_err(|error| {
                        StreamValidationError::InvalidRecordSemantics(error.to_string())
                    })?;
                    if header.game_hour_raw != gap.target_hour_raw {
                        return Err(StreamValidationError::InvalidRecordSemantics(format!(
                            "DOMAIN_GAP H4CR hour {} does not equal target witness {}",
                            header.game_hour_raw, gap.target_hour_raw
                        )));
                    }
                    let key = (header.domain_id, gap.part.code());
                    let prior = self.gap_outcomes.get(&key);
                    let expected_ordinal = prior.map_or(0, |state| state.next_ordinal);
                    if gap.outcome_ordinal != expected_ordinal {
                        return Err(StreamValidationError::InvalidRecordSemantics(format!(
                            "DOMAIN_GAP outcome ordinal {} for domain {} part {} does not equal next required {expected_ordinal}",
                            gap.outcome_ordinal,
                            header.domain_id,
                            gap.part.code()
                        )));
                    }
                    if let Some(state) = prior {
                        if gap.range_start_hour_raw <= state.last_range_end {
                            return Err(StreamValidationError::InvalidRecordSemantics(format!(
                                "DOMAIN_GAP range starts at {} at or before prior end {} for domain {} part {}; ranges must be strictly monotonic and nonoverlapping",
                                gap.range_start_hour_raw,
                                state.last_range_end,
                                header.domain_id,
                                gap.part.code()
                            )));
                        }
                    }
                    let next_ordinal = gap.outcome_ordinal.checked_add(1).ok_or_else(|| {
                        StreamValidationError::InvalidRecordSemantics(
                            "DOMAIN_GAP outcome ordinal is exhausted".to_string(),
                        )
                    })?;
                    Some((
                        key,
                        GapOutcomeState {
                            next_ordinal,
                            last_range_end: gap.range_end_hour_raw,
                        },
                    ))
                } else {
                    None
                };
                Ok(PendingCommit::Ordinary {
                    next_sequence: following_sequence,
                    gap_outcome,
                    record_type,
                    release_epilogue,
                    content_identity_census,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::DATE_EPOCH_CONST;
    use crate::campaign_identity::{
        encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_DOMAIN_STREAM_HEADER,
        COMPACT_RECORD_TYPE_STREAM_HEADER,
    };
    use crate::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};
    use crate::freeze_census_record::{
        FreezeCensusCounterSnapshot, FreezeCensusPhase, FreezeCensusRecordV1,
    };
    use crate::record_semantics::{
        COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
        COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END,
        COMPACT_RECORD_TYPE_FREEZE_CENSUS,
    };

    const UUID: &str = "a4b17fed-ed12-4361-9064-fb6576638110";

    fn game_hour() -> i32 {
        DATE_EPOCH_CONST + 1944 * 365 * 24
    }

    fn cid1_payload() -> Vec<u8> {
        let observation = Cid1Observation {
            game_unique_id: UUID.to_owned(),
            game_unique_seed: -947_102,
            game_hour_raw: game_hour(),
        };
        let fields = Cid1Fields::from_stable_observations(
            observation.clone(),
            observation,
            "vanilla_latest_2026_07_15".to_owned(),
            Cid1Witnesses::default(),
        )
        .unwrap();
        encode_cid1(&fields).unwrap()
    }

    fn header(
        sequence: u64,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) -> CompactRecordHeader {
        CompactRecordHeader {
            sequence,
            game_hour_raw: game_hour(),
            domain_id,
            record_type,
            payload_len: payload.len(),
            record_flags: 0,
        }
    }

    fn cid1_header(payload: &[u8]) -> CompactRecordHeader {
        header(
            0,
            COMPACT_DOMAIN_STREAM_HEADER,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            payload,
        )
    }

    fn data_header(sequence: u64, domain_id: u16, payload: &[u8]) -> CompactRecordHeader {
        header(
            sequence,
            domain_id,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            payload,
        )
    }

    fn census_payload(phase: FreezeCensusPhase) -> Vec<u8> {
        FreezeCensusRecordV1::encode_bounded(
            phase,
            FreezeCensusCounterSnapshot::default(),
            FreezeCensusCounterSnapshot::default(),
            &[],
        )
        .unwrap()
    }

    fn census_header(sequence: u64, payload: &[u8]) -> CompactRecordHeader {
        let mut value = header(
            sequence,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            payload,
        );
        value.game_hour_raw = 0;
        value
    }

    #[test]
    fn identity_v1_retains_only_placement_validated_authority_and_excludes_it_from_count() {
        let payload = cid1_payload();
        let mut validator = StatefulStreamValidator::identity_v1();
        assert_eq!(validator.mode(), StreamIdentityMode::IdentityV1);
        assert_eq!(validator.next_sequence(), 0);
        assert_eq!(validator.ordinary_record_count(), 0);
        assert_eq!(
            validator.validate_record(&cid1_header(&payload), &payload),
            Ok(AcceptedRecordKind::CampaignIdentity)
        );
        assert_eq!(validator.next_sequence(), 1);
        assert_eq!(validator.ordinary_record_count(), 0);
        assert_eq!(validator.identity().unwrap().game_unique_id(), UUID);

        let ordinary_payload = b"DATA";
        assert_eq!(
            validator.validate_record(&data_header(1, 18, ordinary_payload), ordinary_payload),
            Ok(AcceptedRecordKind::Ordinary)
        );
        assert_eq!(validator.next_sequence(), 2);
        assert_eq!(validator.ordinary_record_count(), 1);
    }

    /// kills: rejecting the defined CID1 -> install census -> first-frame
    /// preamble, assigning the census an hour, or failing to advance sequence
    /// only after the census is accepted.
    #[test]
    fn cid1_then_census_then_first_frame_is_the_canonical_preamble() {
        let cid1 = cid1_payload();
        let census = census_payload(FreezeCensusPhase::Install);
        let data = b"DATA";
        let mut validator = StatefulStreamValidator::identity_v1();
        validator
            .validate_record(&cid1_header(&cid1), &cid1)
            .unwrap();
        assert_eq!(
            validator.validate_record(&census_header(1, &census), &census),
            Ok(AcceptedRecordKind::Ordinary)
        );
        assert_eq!(
            validator.validate_record(&data_header(2, 18, data), data),
            Ok(AcceptedRecordKind::Ordinary)
        );
    }

    /// kills: making census presence mandatory and thereby rejecting an honest
    /// capture that failed to publish optional telemetry.
    #[test]
    fn missing_census_is_legal() {
        let cid1 = cid1_payload();
        let data = b"DATA";
        let mut validator = StatefulStreamValidator::identity_v1();
        validator
            .validate_record(&cid1_header(&cid1), &cid1)
            .unwrap();
        assert_eq!(
            validator.validate_record(&data_header(1, 18, data), data),
            Ok(AcceptedRecordKind::Ordinary)
        );
    }

    /// kills: accepting install telemetry after world records or accepting a
    /// second install census in one logical stream.
    #[test]
    fn late_and_duplicate_install_census_are_rejected() {
        let census = census_payload(FreezeCensusPhase::Install);
        let data = b"DATA";
        let cid1 = cid1_payload();

        let mut late = StatefulStreamValidator::identity_v1();
        late.validate_record(&cid1_header(&cid1), &cid1).unwrap();
        late.validate_record(&data_header(1, 18, data), data)
            .unwrap();
        assert!(late
            .validate_record(&census_header(2, &census), &census)
            .is_err());

        let mut duplicate = StatefulStreamValidator::identity_v1();
        duplicate
            .validate_record(&cid1_header(&cid1), &cid1)
            .unwrap();
        duplicate
            .validate_record(&census_header(1, &census), &census)
            .unwrap();
        assert!(duplicate
            .validate_record(&census_header(2, &census), &census)
            .is_err());
    }

    /// LEGACY MODE CARRIES AN INSTALL CENSUS, and this test used to assert the
    /// opposite.
    ///
    /// The old rule was "type-11 install telemetry requires the CID1 that
    /// defines the logical stream it must immediately follow". The recorder
    /// refutes it: `capture_identity_mode` in recorder_host selects
    /// `IdentityV1` only for auto-attach captures and `Legacy` -- no CID1 at
    /// all -- for every capture that attaches to a game already running, and
    /// those captures write the install census as their first physical record
    /// at sequence 1. So the rule refused the recorder's own shipped output on
    /// half of production, and the recorder_host fold accepted it anyway by
    /// passing the identity flag as a hardcoded `true`. Identity presence is a
    /// property of the stream MODE; the install census's placement is that it
    /// is the first ORDINARY record of its logical stream, in both modes.
    ///
    /// kills: re-introducing any stream-identity condition on the install slot
    /// (the first assertion fails), and dropping the slot rule itself so an
    /// install census could float anywhere (the second and third fail).
    #[test]
    fn legacy_mode_carries_an_install_census_as_its_first_ordinary_record() {
        let census = census_payload(FreezeCensusPhase::Install);
        let mut validator = StatefulStreamValidator::legacy();
        validator
            .validate_record(&census_header(1, &census), &census)
            .expect("a legacy stream's first ordinary record may be the install census");

        // A second one is still refused, exactly as in identity mode.
        assert!(validator
            .validate_record(&census_header(2, &census), &census)
            .is_err());

        // And a LATE one is still refused: the slot is the first ordinary
        // record, not "anywhere in a legacy stream".
        let data = b"DATA";
        let mut late = StatefulStreamValidator::legacy();
        late.validate_record(&data_header(1, 18, data), data)
            .unwrap();
        assert!(late
            .validate_record(&census_header(2, &census), &census)
            .is_err());
    }

    /// kills: generalising the type-8 sentinel rule incorrectly or accepting a
    /// type-11 census whose own H4CR hour is not the explicit zero sentinel.
    #[test]
    fn freeze_census_has_its_own_zero_hour_rule() {
        let census = census_payload(FreezeCensusPhase::Install);
        let mut bad_header = census_header(1, &census);
        bad_header.game_hour_raw = game_hour();
        let mut validator = StatefulStreamValidator::legacy();
        assert!(validator.validate_record(&bad_header, &census).is_err());
    }

    fn frame_end_header(sequence: u64) -> CompactRecordHeader {
        header(
            sequence,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        )
    }

    /// Validator that has just accepted a FRAME_END as ordinary sequence 1.
    fn validator_at_a_closed_boundary() -> StatefulStreamValidator {
        let boundary = [0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        let mut validator = StatefulStreamValidator::legacy();
        validator
            .validate_record(&frame_end_header(1), &boundary)
            .unwrap();
        validator
    }

    /// kills: refusing the shipped teardown epilogue. If the release branch is
    /// removed, narrowed to install-only, or gated behind a flag again, the
    /// recorder's own teardown record stops being readable and this fails.
    ///
    /// POSITIVE FIXTURE for the stream validator.
    #[test]
    fn one_release_census_after_a_boundary_is_accepted() {
        let release = census_payload(FreezeCensusPhase::Release);
        let mut validator = validator_at_a_closed_boundary();
        assert_eq!(
            validator.validate_record(&census_header(2, &release), &release),
            Ok(AcceptedRecordKind::Ordinary)
        );
    }

    /// kills: dropping the "exactly one, then EOF" half of the grammar. Two
    /// records after the boundary -- a second census, or any other record type
    /// -- must be refused, and the refusal must be the epilogue rule rather
    /// than an incidental sequence error.
    ///
    /// NEGATIVE FIXTURE: two records after the end.
    #[test]
    fn nothing_may_follow_the_release_census() {
        let release = census_payload(FreezeCensusPhase::Release);
        let data = b"DATA";

        let mut second_census = validator_at_a_closed_boundary();
        second_census
            .validate_record(&census_header(2, &release), &release)
            .unwrap();
        assert!(second_census
            .validate_record(&census_header(3, &release), &release)
            .is_err());

        let mut then_data = validator_at_a_closed_boundary();
        then_data
            .validate_record(&census_header(2, &release), &release)
            .unwrap();
        assert!(then_data
            .validate_record(&data_header(3, 18, data), data)
            .is_err());

        let mut then_boundary = validator_at_a_closed_boundary();
        then_boundary
            .validate_record(&census_header(2, &release), &release)
            .unwrap();
        assert!(then_boundary
            .validate_record(
                &frame_end_header(3),
                &[0u8; COMPACT_FRAME_END_PAYLOAD_LEN]
            )
            .is_err());
    }

    /// kills: admitting the epilogue behind a record that closes no hour, which
    /// is what turns "one bounded record after the end" into "anything may
    /// follow anything".
    ///
    /// NEGATIVE FIXTURE: a record after the end without a proper FRAME_END.
    #[test]
    fn a_release_census_needs_a_closed_hour_immediately_before_it() {
        let release = census_payload(FreezeCensusPhase::Release);
        let data = b"DATA";

        let mut after_data = StatefulStreamValidator::legacy();
        after_data
            .validate_record(&data_header(1, 18, data), data)
            .unwrap();
        assert!(after_data
            .validate_record(&census_header(2, &release), &release)
            .is_err());

        // No ordinary record at all before it.
        let mut first_record = StatefulStreamValidator::legacy();
        assert!(first_record
            .validate_record(&census_header(1, &release), &release)
            .is_err());
    }

    /// kills: letting the epilogue open an hour. Its H4CR envelope must keep
    /// the zero no-coverage sentinel exactly as the install record does, or it
    /// would enter an hour's coverage span and swallow real bytes.
    ///
    /// NEGATIVE FIXTURE: the epilogue opens an hour.
    #[test]
    fn a_release_census_may_not_carry_a_game_hour() {
        let release = census_payload(FreezeCensusPhase::Release);
        let mut hour_bearing = census_header(2, &release);
        hour_bearing.game_hour_raw = game_hour();
        let mut validator = validator_at_a_closed_boundary();
        assert!(validator.validate_record(&hour_bearing, &release).is_err());
    }

    /// kills: raising or removing the fixed size ceiling on the epilogue. A
    /// payload past `FREEZE_CENSUS_MAX_PAYLOAD_LEN` is refused before placement
    /// is ever considered.
    ///
    /// NEGATIVE FIXTURE: oversize.
    #[test]
    fn an_oversize_release_census_is_refused() {
        use crate::freeze_census_record::FREEZE_CENSUS_MAX_PAYLOAD_LEN;
        let mut release = census_payload(FreezeCensusPhase::Release);
        release.resize(FREEZE_CENSUS_MAX_PAYLOAD_LEN + 1, 0);
        let mut validator = validator_at_a_closed_boundary();
        assert!(validator
            .validate_record(&census_header(2, &release), &release)
            .is_err());
    }

    /// kills: letting the epilogue ride behind a FRAME_REFUSED without the
    /// whole-stream helper agreeing, and letting that helper go quiet on a
    /// malformed epilogue. `validate_freeze_census_stream_placement` is the one
    /// entry point every secondary scanner uses, so its verdict is the rule
    /// those scanners actually enforce.
    #[test]
    fn the_secondary_scanner_helper_agrees_with_the_stateful_validator() {
        let release = census_payload(FreezeCensusPhase::Release);
        let boundary = [0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        let data = b"DATA".to_vec();

        // Serialize the exact H4CR framing `parse_compact_record_header` reads,
        // so this test drives the helper over real bytes rather than a mock.
        let serialized = |records: &[(CompactRecordHeader, Vec<u8>)]| {
            let mut raw = Vec::new();
            for (header, payload) in records {
                raw.extend_from_slice(crate::record_header::COMPACT_RECORD_MAGIC);
                raw.extend_from_slice(&crate::record_header::COMPACT_RECORD_VERSION.to_le_bytes());
                raw.extend_from_slice(
                    &(crate::record_header::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes(),
                );
                raw.extend_from_slice(&header.sequence.to_le_bytes());
                raw.extend_from_slice(&header.game_hour_raw.to_le_bytes());
                raw.extend_from_slice(&header.domain_id.to_le_bytes());
                raw.extend_from_slice(&header.record_type.to_le_bytes());
                raw.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                raw.extend_from_slice(&header.record_flags.to_le_bytes());
                raw.extend_from_slice(payload);
            }
            raw
        };

        let good = serialized(&[
            (frame_end_header(1), boundary.to_vec()),
            (census_header(2, &release), release.clone()),
        ]);
        assert_eq!(validate_freeze_census_stream_placement(&good), Ok(()));

        let two_after_the_end = serialized(&[
            (frame_end_header(1), boundary.to_vec()),
            (census_header(2, &release), release.clone()),
            (census_header(3, &release), release.clone()),
        ]);
        assert!(validate_freeze_census_stream_placement(&two_after_the_end).is_err());

        let no_boundary = serialized(&[
            (data_header(1, 18, &data), data.clone()),
            (census_header(2, &release), release.clone()),
        ]);
        assert!(validate_freeze_census_stream_placement(&no_boundary).is_err());

        // An epilogue whose envelope carries a game hour. It closes no hour and
        // claims no coverage, so a nonzero hour is a lie about what it observed.
        let mut hour_bearing_header = census_header(2, &release);
        hour_bearing_header.game_hour_raw = game_hour();
        let hour_bearing = serialized(&[
            (frame_end_header(1), boundary.to_vec()),
            (hour_bearing_header, release.clone()),
        ]);
        assert!(validate_freeze_census_stream_placement(&hour_bearing).is_err());

        // THE EARLY-BAIL REGRESSION. A record that breaks some OTHER stateful
        // rule -- here reserved domain zero -- used to make this helper return
        // success without ever looking at the H4FC that followed. A scanner
        // that tolerates domain zero then accepted a malformed epilogue. The
        // dedicated placement scan owns only the H4FC rules and never
        // short-circuits on a rule it does not own.
        //
        // kills: restoring the "stop at the first non-FREEZE_CENSUS error"
        // behaviour, which makes both of these pass silently.
        let mut reserved_domain = data_header(1, 18, &data);
        reserved_domain.domain_id = 0;
        let mut hour_bearing_after_reserved = census_header(3, &release);
        hour_bearing_after_reserved.game_hour_raw = game_hour();
        let masked = serialized(&[
            (reserved_domain, data.clone()),
            (frame_end_header(2), boundary.to_vec()),
            (hour_bearing_after_reserved, release.clone()),
        ]);
        assert!(
            validate_freeze_census_stream_placement(&masked).is_err(),
            "a preceding non-census rule break silenced the placement scan"
        );

        let mut reserved_domain = data_header(1, 18, &data);
        reserved_domain.domain_id = 0;
        let masked_duplicate = serialized(&[
            (reserved_domain, data.clone()),
            (frame_end_header(2), boundary.to_vec()),
            (census_header(3, &release), release.clone()),
            (census_header(4, &release), release.clone()),
        ]);
        assert!(
            validate_freeze_census_stream_placement(&masked_duplicate).is_err(),
            "a preceding non-census rule break silenced the duplicate check"
        );
    }

    #[test]
    fn protocol_mode_is_explicit_and_never_sniffs_or_falls_back() {
        let ordinary_payload = b"DATA";
        let mut identity = StatefulStreamValidator::identity_v1();
        let error = identity
            .validate_record(&data_header(1, 18, ordinary_payload), ordinary_payload)
            .unwrap_err();
        assert!(matches!(
            error,
            StreamValidationError::InvalidCampaignIdentity(Cid1Error::WrongRecordSequence(1))
        ));

        let cid1 = cid1_payload();
        let mut legacy = StatefulStreamValidator::legacy();
        assert_eq!(legacy.mode(), StreamIdentityMode::Legacy);
        assert_eq!(legacy.next_sequence(), 1);
        assert_eq!(
            legacy.validate_record(&cid1_header(&cid1), &cid1),
            Err(StreamValidationError::UnexpectedSequence {
                expected: 1,
                actual: 0,
            })
        );
        assert!(legacy.identity().is_none());
        assert_eq!(legacy.ordinary_record_count(), 0);
    }

    #[test]
    fn identity_placement_flags_type_domain_hour_and_length_are_delegated_to_cid1_authority() {
        let payload = cid1_payload();
        let base = cid1_header(&payload);
        let cases = [
            (
                1,
                base.domain_id,
                base.record_type,
                0,
                base.game_hour_raw,
                payload.len(),
            ),
            (
                0,
                18,
                base.record_type,
                0,
                base.game_hour_raw,
                payload.len(),
            ),
            (
                0,
                base.domain_id,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                base.game_hour_raw,
                payload.len(),
            ),
            (
                0,
                base.domain_id,
                base.record_type,
                1,
                base.game_hour_raw,
                payload.len(),
            ),
            (
                0,
                base.domain_id,
                base.record_type,
                0,
                base.game_hour_raw + 1,
                payload.len(),
            ),
            (
                0,
                base.domain_id,
                base.record_type,
                0,
                base.game_hour_raw,
                payload.len() + 1,
            ),
        ];
        for (sequence, domain_id, record_type, flags, hour, payload_len) in cases {
            let mut validator = StatefulStreamValidator::identity_v1();
            let mut candidate = base;
            candidate.sequence = sequence;
            candidate.domain_id = domain_id;
            candidate.record_type = record_type;
            candidate.record_flags = flags;
            candidate.game_hour_raw = hour;
            candidate.payload_len = payload_len;
            assert!(matches!(
                validator.validate_record(&candidate, &payload),
                Err(StreamValidationError::InvalidCampaignIdentity(_))
            ));
            assert!(validator.identity().is_none());
            assert_eq!(validator.ordinary_record_count(), 0);
            assert!(validator.is_terminal());
        }
    }

    #[test]
    fn sequence_gap_reuse_and_backwards_are_terminal_and_atomic() {
        for actual in [0, 1, 3, u64::MAX] {
            let payload = b"DATA";
            let mut validator = StatefulStreamValidator::legacy();
            validator
                .validate_record(&data_header(1, 18, payload), payload)
                .unwrap();
            let before_count = validator.ordinary_record_count();
            let before_next = validator.next_sequence();
            let error = validator
                .validate_record(&data_header(actual, 18, payload), payload)
                .unwrap_err();
            assert_eq!(
                error,
                StreamValidationError::UnexpectedSequence {
                    expected: 2,
                    actual,
                }
            );
            assert_eq!(validator.ordinary_record_count(), before_count);
            assert_eq!(validator.next_sequence(), before_next);
            assert_eq!(validator.failure(), Some(&error));
            assert_eq!(
                validator.validate_record(&data_header(2, 18, payload), payload),
                Err(StreamValidationError::Terminal)
            );
            assert_eq!(validator.failure(), Some(&error));
        }
    }

    #[test]
    fn sequence_exhaustion_is_rejected_before_state_advances() {
        let payload = b"DATA";
        let mut validator = StatefulStreamValidator {
            mode: StreamIdentityMode::Legacy,
            phase: StreamPhase::AwaitingOrdinary {
                next_sequence: u64::MAX,
            },
            identity: None,
            ordinary_record_count: 7,
            last_ordinary_record_type: Some(KnownRecordType::DomainData),
            release_epilogue_seen: false,
            content_identity_census_seen: false,
            gap_outcomes: BTreeMap::new(),
            failure: None,
        };
        let error = validator
            .validate_record(&data_header(u64::MAX, 18, payload), payload)
            .unwrap_err();
        assert_eq!(error, StreamValidationError::SequenceExhausted(u64::MAX));
        assert_eq!(validator.next_sequence(), u64::MAX);
        assert_eq!(validator.ordinary_record_count(), 7);
        assert_eq!(validator.failure(), Some(&error));
    }

    #[test]
    fn domain_zero_is_never_an_ordinary_record_even_with_an_allowed_type() {
        let payload = b"DATA";
        let mut validator = StatefulStreamValidator::legacy();
        assert_eq!(
            validator.validate_record(&data_header(1, 0, payload), payload),
            Err(StreamValidationError::ReservedDomainZero)
        );
        assert_eq!(validator.next_sequence(), 1);
        assert_eq!(validator.ordinary_record_count(), 0);
    }

    #[test]
    fn domain_gap_is_ordinary_and_binds_header_hour_to_target_witness() {
        let gap_value = DomainGapRecordV1 {
            part: DomainGapPart::State,
            reason: DomainGapReason::Torn,
            target_hour_raw: game_hour(),
            range_start_hour_raw: game_hour(),
            range_end_hour_raw: game_hour(),
            tick_before: 41,
            tick_after: 42,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        };
        let gap = gap_value.encode().unwrap();
        let mut header = header(1, 81, COMPACT_RECORD_TYPE_DOMAIN_GAP, &gap);
        let mut validator = StatefulStreamValidator::legacy();
        assert_eq!(
            validator.validate_record(&header, &gap),
            Ok(AcceptedRecordKind::Ordinary)
        );
        assert_eq!(validator.next_sequence(), 2);
        assert_eq!(validator.ordinary_record_count(), 1);

        header.sequence = 1;
        header.game_hour_raw += 1;
        let mut wrong_hour = StatefulStreamValidator::legacy();
        assert!(matches!(
            wrong_hour.validate_record(&header, &gap),
            Err(StreamValidationError::InvalidRecordSemantics(_))
        ));
        assert_eq!(wrong_hour.next_sequence(), 1);
        assert_eq!(wrong_hour.ordinary_record_count(), 0);
    }

    #[test]
    fn domain_gap_ordinals_and_ranges_are_per_domain_part_contiguous() {
        let make = |target: i32, ordinal: u64, part: DomainGapPart| {
            DomainGapRecordV1 {
                part,
                reason: DomainGapReason::Torn,
                target_hour_raw: target,
                range_start_hour_raw: target,
                range_end_hour_raw: target,
                tick_before: 41 + ordinal,
                tick_after: 42 + ordinal,
                queue_capacity: 0,
                queue_depth: 0,
                queue_dropped: 0,
                outcome_ordinal: ordinal,
            }
            .encode()
            .unwrap()
        };
        let gap_header = |sequence, domain_id, payload: &[u8], target_hour| {
            let mut value = header(sequence, domain_id, COMPACT_RECORD_TYPE_DOMAIN_GAP, payload);
            value.game_hour_raw = target_hour;
            value
        };

        let first = make(game_hour(), 0, DomainGapPart::State);
        let second = make(game_hour() + 1, 1, DomainGapPart::State);
        let mut validator = StatefulStreamValidator::legacy();
        validator
            .validate_record(&gap_header(1, 81, &first, game_hour()), &first)
            .unwrap();
        validator
            .validate_record(&gap_header(2, 81, &second, game_hour() + 1), &second)
            .unwrap();

        let other_domain = make(game_hour(), 0, DomainGapPart::State);
        validator
            .validate_record(
                &gap_header(3, 82, &other_domain, game_hour()),
                &other_domain,
            )
            .unwrap();
        let other_part = make(game_hour(), 0, DomainGapPart::Event);
        validator
            .validate_record(&gap_header(4, 81, &other_part, game_hour()), &other_part)
            .unwrap();

        for bad in [
            make(game_hour() + 1, 0, DomainGapPart::State),
            make(game_hour(), 1, DomainGapPart::State),
            make(game_hour() + 2, 3, DomainGapPart::State),
        ] {
            let mut rejected = StatefulStreamValidator::legacy();
            rejected
                .validate_record(&gap_header(1, 81, &first, game_hour()), &first)
                .unwrap();
            let decoded_bad = DomainGapRecordV1::decode(&bad).unwrap();
            assert!(matches!(
                rejected
                    .validate_record(&gap_header(2, 81, &bad, decoded_bad.target_hour_raw), &bad),
                Err(StreamValidationError::InvalidRecordSemantics(_))
            ));
        }
    }

    #[test]
    fn domain_gap_ranges_may_have_accepted_hours_between_contiguous_ordinals() {
        let make_gap = |target: i32, ordinal: u64| {
            DomainGapRecordV1 {
                part: DomainGapPart::State,
                reason: DomainGapReason::Torn,
                target_hour_raw: target,
                range_start_hour_raw: target,
                range_end_hour_raw: target,
                tick_before: 41 + ordinal,
                tick_after: 42 + ordinal,
                queue_capacity: 0,
                queue_depth: 0,
                queue_dropped: 0,
                outcome_ordinal: ordinal,
            }
            .encode()
            .unwrap()
        };
        let first = make_gap(game_hour(), 0);
        let accepted = vec![0u8; 40];
        let second = make_gap(game_hour() + 2, 1);
        let mut validator = StatefulStreamValidator::legacy();

        validator
            .validate_record(
                &header(1, 81, COMPACT_RECORD_TYPE_DOMAIN_GAP, &first),
                &first,
            )
            .unwrap();
        let mut accepted_header = header(2, 81, COMPACT_RECORD_TYPE_DOMAIN_PERF, &accepted);
        accepted_header.game_hour_raw = game_hour() + 1;
        validator
            .validate_record(&accepted_header, &accepted)
            .unwrap();
        let mut second_header = header(3, 81, COMPACT_RECORD_TYPE_DOMAIN_GAP, &second);
        second_header.game_hour_raw = game_hour() + 2;
        validator.validate_record(&second_header, &second).unwrap();
    }

    #[test]
    fn ordinary_payload_length_flags_and_record_type_semantics_fail_closed() {
        let data = b"DATA";

        let mut wrong_length = StatefulStreamValidator::legacy();
        let mut candidate = data_header(1, 18, data);
        candidate.payload_len += 1;
        assert!(matches!(
            wrong_length.validate_record(&candidate, data),
            Err(StreamValidationError::PayloadLengthMismatch { .. })
        ));

        let mut unknown_flags = StatefulStreamValidator::legacy();
        let mut candidate = data_header(1, 18, data);
        candidate.record_flags = 1;
        assert!(matches!(
            unknown_flags.validate_record(&candidate, data),
            Err(StreamValidationError::InvalidRecordSemantics(_))
        ));

        let mut unknown_type = StatefulStreamValidator::legacy();
        let candidate = header(1, 18, 99, data);
        assert!(matches!(
            unknown_type.validate_record(&candidate, data),
            Err(StreamValidationError::InvalidRecordSemantics(_))
        ));

        let mut malformed_perf = StatefulStreamValidator::legacy();
        let candidate = header(1, 18, COMPACT_RECORD_TYPE_DOMAIN_PERF, &[]);
        assert!(matches!(
            malformed_perf.validate_record(&candidate, &[]),
            Err(StreamValidationError::InvalidRecordSemantics(_))
        ));
    }

    #[test]
    fn later_failure_retains_accepted_identity_but_cannot_mutate_stream_state() {
        let cid1 = cid1_payload();
        let mut validator = StatefulStreamValidator::identity_v1();
        validator
            .validate_record(&cid1_header(&cid1), &cid1)
            .unwrap();
        let identity_hash = validator.identity().unwrap().payload_sha256();

        let data = b"DATA";
        let mut invalid = data_header(1, 18, data);
        invalid.record_flags = 1;
        let first_error = validator.validate_record(&invalid, data).unwrap_err();
        assert_eq!(
            validator.identity().unwrap().payload_sha256(),
            identity_hash
        );
        assert_eq!(validator.next_sequence(), 1);
        assert_eq!(validator.ordinary_record_count(), 0);
        assert_eq!(validator.failure(), Some(&first_error));

        assert_eq!(
            validator.validate_record(&data_header(1, 18, data), data),
            Err(StreamValidationError::Terminal)
        );
        assert_eq!(
            validator.identity().unwrap().payload_sha256(),
            identity_hash
        );
        assert_eq!(validator.next_sequence(), 1);
        assert_eq!(validator.ordinary_record_count(), 0);
    }
}
