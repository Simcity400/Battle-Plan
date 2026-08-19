use compact_contract::campaign_identity::ValidatedCid1;
use compact_contract::campaign_rollover::{
    RolloverAckV1, RolloverDecision, RolloverProposeV1, RolloverReason,
};
use compact_contract::first_compact_ack::FirstCompactFrameAckV1;
use compact_contract::stream_validator::StreamIdentityMode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollectorRolloverPhase {
    Streaming,
    AwaitingRolloverAck,
    AwaitingNewHeaderAck,
    Failed,
}

pub trait CollectorRolloverSink {
    fn pause_and_drain(&mut self, logical_stream_id: u64) -> Result<u64, String>;
    fn start_new_stream(
        &mut self,
        logical_stream_id: u64,
        identity: &ValidatedCid1,
    ) -> Result<(), String>;
    fn resume_after_header_ack(&mut self, logical_stream_id: u64) -> Result<(), String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CollectorRolloverError {
    IllegalPhase(CollectorRolloverPhase),
    NonMonotonicNonce,
    SequenceExhausted,
    Proposal(String),
    AckMismatch,
    Rejected(u16),
    NewHeaderMismatch,
    Sink(String),
}

impl std::fmt::Display for CollectorRolloverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IllegalPhase(phase) => write!(f, "collector rollover is illegal in {phase:?}"),
            Self::NonMonotonicNonce => write!(f, "collector rollover nonce was reused/regressed"),
            Self::SequenceExhausted => write!(f, "collector logical-stream id exhausted"),
            Self::Proposal(error) => write!(f, "collector rollover proposal invalid: {error}"),
            Self::AckMismatch => write!(f, "collector rollover acknowledgement mismatch"),
            Self::Rejected(code) => write!(f, "recorder rejected rollover with code {code}"),
            Self::NewHeaderMismatch => write!(f, "collector new-header CFA1 mismatch"),
            Self::Sink(error) => write!(f, "collector rollover sink failed: {error}"),
        }
    }
}

impl std::error::Error for CollectorRolloverError {}

struct PendingRollover {
    proposal: RolloverProposeV1,
    candidate: ValidatedCid1,
}

pub struct CollectorRolloverCoordinator {
    phase: CollectorRolloverPhase,
    current_logical_stream_id: u64,
    current_campaign_id: [u8; 32],
    current_cid1_payload_sha256: [u8; 32],
    last_proposal_nonce: u64,
    pending: Option<PendingRollover>,
}

impl CollectorRolloverCoordinator {
    pub fn new(initial_identity: &ValidatedCid1) -> Self {
        Self {
            phase: CollectorRolloverPhase::Streaming,
            current_logical_stream_id: 0,
            current_campaign_id: initial_identity.campaign_id(),
            current_cid1_payload_sha256: initial_identity.payload_sha256(),
            last_proposal_nonce: 0,
            pending: None,
        }
    }

    pub fn phase(&self) -> CollectorRolloverPhase {
        self.phase
    }

    pub fn prepare_proposal<S: CollectorRolloverSink>(
        &mut self,
        sink: &mut S,
        candidate: ValidatedCid1,
        boundary_game_hour_raw: i32,
        reason: RolloverReason,
        proposal_nonce: u64,
    ) -> Result<RolloverProposeV1, CollectorRolloverError> {
        if self.phase != CollectorRolloverPhase::Streaming {
            return Err(CollectorRolloverError::IllegalPhase(self.phase));
        }
        if proposal_nonce <= self.last_proposal_nonce || proposal_nonce == u64::MAX {
            return Err(self.fail(CollectorRolloverError::NonMonotonicNonce));
        }
        let proposed_logical_stream_id = self
            .current_logical_stream_id
            .checked_add(1)
            .filter(|value| *value != u64::MAX)
            .ok_or_else(|| self.fail(CollectorRolloverError::SequenceExhausted))?;
        let last_admitted_h4cr_sequence = sink
            .pause_and_drain(self.current_logical_stream_id)
            .map_err(|error| self.fail(CollectorRolloverError::Sink(error)))?;
        let proposal = RolloverProposeV1 {
            proposed_logical_stream_id,
            last_admitted_h4cr_sequence,
            boundary_game_hour_raw,
            reason,
            current_campaign_id: self.current_campaign_id,
            proposed_campaign_id: candidate.campaign_id(),
            current_cid1_payload_sha256: self.current_cid1_payload_sha256,
            proposed_cid1_payload_sha256: candidate.payload_sha256(),
            proposal_nonce,
        };
        proposal
            .validate()
            .map_err(|error| self.fail(CollectorRolloverError::Proposal(error.to_string())))?;
        self.pending = Some(PendingRollover {
            proposal,
            candidate,
        });
        self.phase = CollectorRolloverPhase::AwaitingRolloverAck;
        Ok(proposal)
    }

    pub fn accept_rollover_ack<S: CollectorRolloverSink>(
        &mut self,
        sink: &mut S,
        acknowledgement: RolloverAckV1,
    ) -> Result<(), CollectorRolloverError> {
        if self.phase != CollectorRolloverPhase::AwaitingRolloverAck {
            return Err(CollectorRolloverError::IllegalPhase(self.phase));
        }
        let (proposed_id, proposal_nonce) = match self.pending.as_ref() {
            Some(pending) => (
                pending.proposal.proposed_logical_stream_id,
                pending.proposal.proposal_nonce,
            ),
            None => return Err(self.fail(CollectorRolloverError::AckMismatch)),
        };
        if acknowledgement.proposed_logical_stream_id != proposed_id
            || acknowledgement.proposal_nonce != proposal_nonce
        {
            return Err(self.fail(CollectorRolloverError::AckMismatch));
        }
        match acknowledgement.decision {
            RolloverDecision::Reject => {
                return Err(self.fail(CollectorRolloverError::Rejected(acknowledgement.error_code)))
            }
            RolloverDecision::Accept => {}
        }
        let candidate = self
            .pending
            .as_ref()
            .expect("checked pending")
            .candidate
            .clone();
        sink.start_new_stream(proposed_id, &candidate)
            .map_err(|error| self.fail(CollectorRolloverError::Sink(error)))?;
        self.phase = CollectorRolloverPhase::AwaitingNewHeaderAck;
        Ok(())
    }

    pub fn accept_new_header_ack<S: CollectorRolloverSink>(
        &mut self,
        sink: &mut S,
        acknowledgement: FirstCompactFrameAckV1,
    ) -> Result<(), CollectorRolloverError> {
        if self.phase != CollectorRolloverPhase::AwaitingNewHeaderAck {
            return Err(CollectorRolloverError::IllegalPhase(self.phase));
        }
        let (logical_stream_id, campaign_id, cid1_payload_sha256, proposal_nonce) =
            match self.pending.as_ref() {
                Some(pending) => (
                    pending.proposal.proposed_logical_stream_id,
                    pending.proposal.proposed_campaign_id,
                    pending.proposal.proposed_cid1_payload_sha256,
                    pending.proposal.proposal_nonce,
                ),
                None => return Err(self.fail(CollectorRolloverError::NewHeaderMismatch)),
            };
        if acknowledgement.mode != StreamIdentityMode::IdentityV1
            || acknowledgement.logical_stream_id != logical_stream_id
            || acknowledgement.h4cr_sequence != 0
            || acknowledgement.payload_sha256 != cid1_payload_sha256
        {
            return Err(self.fail(CollectorRolloverError::NewHeaderMismatch));
        }
        sink.resume_after_header_ack(logical_stream_id)
            .map_err(|error| self.fail(CollectorRolloverError::Sink(error)))?;
        self.current_logical_stream_id = logical_stream_id;
        self.current_campaign_id = campaign_id;
        self.current_cid1_payload_sha256 = cid1_payload_sha256;
        self.last_proposal_nonce = proposal_nonce;
        self.pending = None;
        self.phase = CollectorRolloverPhase::Streaming;
        Ok(())
    }

    fn fail(&mut self, error: CollectorRolloverError) -> CollectorRolloverError {
        self.phase = CollectorRolloverPhase::Failed;
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::calendar::DATE_EPOCH_CONST;
    use compact_contract::campaign_identity::{
        encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_DOMAIN_STREAM_HEADER,
        COMPACT_RECORD_TYPE_STREAM_HEADER,
    };
    use compact_contract::record_header::CompactRecordHeader;
    use compact_contract::stream_validator::StatefulStreamValidator;

    fn game_hour() -> i32 {
        DATE_EPOCH_CONST + 1936 * 365 * 24
    }

    fn identity(seed: i32) -> ValidatedCid1 {
        identity_at(seed, game_hour())
    }

    fn identity_at(seed: i32, observation_hour: i32) -> ValidatedCid1 {
        let observation = Cid1Observation {
            game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".to_string(),
            game_unique_seed: seed,
            game_hour_raw: observation_hour,
        };
        let fields = Cid1Fields::from_stable_observations(
            observation.clone(),
            observation,
            "rollover-test".to_string(),
            Cid1Witnesses::default(),
        )
        .unwrap();
        let payload = encode_cid1(&fields).unwrap();
        let header = CompactRecordHeader {
            sequence: 0,
            game_hour_raw: observation_hour,
            domain_id: COMPACT_DOMAIN_STREAM_HEADER,
            record_type: COMPACT_RECORD_TYPE_STREAM_HEADER,
            payload_len: payload.len(),
            record_flags: 0,
        };
        let mut validator = StatefulStreamValidator::identity_v1();
        validator.validate_record(&header, &payload).unwrap();
        validator.identity().unwrap().clone()
    }

    #[derive(Default)]
    struct Sink {
        actions: Vec<String>,
    }

    impl CollectorRolloverSink for Sink {
        fn pause_and_drain(&mut self, logical_stream_id: u64) -> Result<u64, String> {
            self.actions.push(format!("pause:{logical_stream_id}"));
            Ok(41)
        }

        fn start_new_stream(
            &mut self,
            logical_stream_id: u64,
            _identity: &ValidatedCid1,
        ) -> Result<(), String> {
            self.actions.push(format!("start:{logical_stream_id}"));
            Ok(())
        }

        fn resume_after_header_ack(&mut self, logical_stream_id: u64) -> Result<(), String> {
            self.actions.push(format!("resume:{logical_stream_id}"));
            Ok(())
        }
    }

    #[test]
    fn accepted_rollover_stays_paused_until_matching_new_header_ack() {
        let initial = identity(1);
        let candidate = identity(2);
        let mut coordinator = CollectorRolloverCoordinator::new(&initial);
        let mut sink = Sink::default();
        let proposal = coordinator
            .prepare_proposal(
                &mut sink,
                candidate.clone(),
                game_hour(),
                RolloverReason::WholePairChanged,
                1,
            )
            .unwrap();
        assert_eq!(proposal.last_admitted_h4cr_sequence, 41);
        coordinator
            .accept_rollover_ack(
                &mut sink,
                RolloverAckV1 {
                    proposed_logical_stream_id: 1,
                    proposal_nonce: 1,
                    decision: RolloverDecision::Accept,
                    error_code: 0,
                },
            )
            .unwrap();
        assert_eq!(
            coordinator.phase(),
            CollectorRolloverPhase::AwaitingNewHeaderAck
        );
        coordinator
            .accept_new_header_ack(
                &mut sink,
                FirstCompactFrameAckV1 {
                    mode: StreamIdentityMode::IdentityV1,
                    logical_stream_id: 1,
                    h4cr_sequence: 0,
                    game_hour_raw: game_hour(),
                    domain_id: COMPACT_DOMAIN_STREAM_HEADER,
                    record_type: COMPACT_RECORD_TYPE_STREAM_HEADER,
                    complete_record_len: u32::try_from(
                        compact_contract::COMPACT_RECORD_HEADER_LEN + candidate.payload().len(),
                    )
                    .unwrap(),
                    complete_record_sha256: [7; 32],
                    payload_sha256: candidate.payload_sha256(),
                },
            )
            .unwrap();
        assert_eq!(coordinator.phase(), CollectorRolloverPhase::Streaming);
        assert_eq!(
            sink.actions,
            vec![
                "pause:0".to_string(),
                "start:1".to_string(),
                "resume:1".to_string(),
            ]
        );
    }

    #[test]
    fn mismatched_ack_is_terminal_and_never_opens_new_sink() {
        let mut coordinator = CollectorRolloverCoordinator::new(&identity(1));
        let mut sink = Sink::default();
        coordinator
            .prepare_proposal(
                &mut sink,
                identity(2),
                game_hour(),
                RolloverReason::WholePairChanged,
                1,
            )
            .unwrap();
        assert!(coordinator
            .accept_rollover_ack(
                &mut sink,
                RolloverAckV1 {
                    proposed_logical_stream_id: 1,
                    proposal_nonce: 2,
                    decision: RolloverDecision::Accept,
                    error_code: 0,
                },
            )
            .is_err());
        assert_eq!(coordinator.phase(), CollectorRolloverPhase::Failed);
        assert_eq!(sink.actions, vec!["pause:0".to_string()]);
    }

    #[test]
    fn same_pair_time_rollback_uses_pair_digest_not_full_cid1_hash() {
        let initial = identity_at(1, game_hour());
        let candidate = identity_at(1, game_hour() - 1);
        assert_eq!(initial.campaign_id(), candidate.campaign_id());
        assert_ne!(initial.payload_sha256(), candidate.payload_sha256());

        let mut coordinator = CollectorRolloverCoordinator::new(&initial);
        let mut sink = Sink::default();
        let proposal = coordinator
            .prepare_proposal(
                &mut sink,
                candidate,
                game_hour() - 1,
                RolloverReason::SamePairTimeRollback,
                1,
            )
            .unwrap();

        assert_eq!(proposal.current_campaign_id, proposal.proposed_campaign_id);
        assert_ne!(
            proposal.current_cid1_payload_sha256,
            proposal.proposed_cid1_payload_sha256
        );
        assert_eq!(
            coordinator.phase(),
            CollectorRolloverPhase::AwaitingRolloverAck
        );
        assert_eq!(sink.actions, vec!["pause:0".to_string()]);
    }
}
