//! Stateful, fail-closed validation for one complete H4CR stream.
//!
//! Identity-bearing and legacy streams are different protocols selected by an
//! explicit constructor.  This module never sniffs the first payload and never
//! falls back between those protocols.

use std::collections::BTreeMap;

use crate::campaign_identity::{parse_cid1_record, Cid1Error, ValidatedCid1};
use crate::domain_gap::DomainGapRecordV1;
use crate::record_header::CompactRecordHeader;
use crate::record_semantics::{validate_compact_record_semantics, COMPACT_RECORD_TYPE_DOMAIN_GAP};

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
            } => {
                self.phase = StreamPhase::AwaitingOrdinary { next_sequence };
                self.ordinary_record_count += 1;
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
    use crate::record_semantics::{
        COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
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
