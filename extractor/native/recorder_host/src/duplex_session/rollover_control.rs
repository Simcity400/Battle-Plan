use compact_contract::campaign_rollover::{
    decode_rollover_propose, encode_rollover_ack, RolloverAckV1, RolloverDecision,
    RolloverProposeV1,
};
use compact_contract::stream_validator::StreamIdentityMode;

use crate::control_transport::{
    ControlCancellation, ControlDeadline, ControlFrame, DeadlineControlChannel, DeadlineControlIo,
    MessageKind,
};
use crate::duplex_session::identity_control::{
    AcknowledgedFirstCompactRecord, IdentityDurableCandidateSink,
    RecorderIdentityControlDispatcher, ValidatedFirstCompactRecord,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainedLogicalStream {
    pub logical_stream_id: u64,
    pub last_h4cr_sequence: u64,
    pub campaign_id: [u8; 32],
    pub cid1_payload_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenedLogicalStream {
    pub sealed_logical_stream_id: u64,
    pub opened_logical_stream_id: u64,
    pub expected_campaign_id: [u8; 32],
    pub expected_cid1_payload_sha256: [u8; 32],
}

pub trait RecorderRolloverStore {
    fn drain_contiguous_through(
        &mut self,
        proposal: &RolloverProposeV1,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<DrainedLogicalStream, String>;

    fn durable_seal_and_open(
        &mut self,
        proposal: &RolloverProposeV1,
        drained: DrainedLogicalStream,
    ) -> Result<OpenedLogicalStream, String>;

    fn durable_record_rejection(
        &mut self,
        proposal: &RolloverProposeV1,
        error_code: u16,
        reason: &str,
    ) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecorderRolloverPhase {
    Streaming,
    AwaitingNewHeader {
        logical_stream_id: u64,
        campaign_id: [u8; 32],
        cid1_payload_sha256: [u8; 32],
    },
    Failed,
}

pub(crate) struct AuthenticatedRolloverRebind {
    current_logical_stream_id: u64,
    current_campaign_id: [u8; 32],
    current_cid1_payload_sha256: [u8; 32],
    proposed_logical_stream_id: u64,
    proposed_campaign_id: [u8; 32],
    proposed_cid1_payload_sha256: [u8; 32],
    proposal_nonce: u64,
}

impl AuthenticatedRolloverRebind {
    pub(crate) fn current_logical_stream_id(&self) -> u64 {
        self.current_logical_stream_id
    }

    pub(crate) fn current_campaign_id(&self) -> [u8; 32] {
        self.current_campaign_id
    }

    pub(crate) fn current_cid1_payload_sha256(&self) -> [u8; 32] {
        self.current_cid1_payload_sha256
    }

    pub(crate) fn proposed_logical_stream_id(&self) -> u64 {
        self.proposed_logical_stream_id
    }

    pub(crate) fn proposed_campaign_id(&self) -> [u8; 32] {
        self.proposed_campaign_id
    }

    pub(crate) fn proposed_cid1_payload_sha256(&self) -> [u8; 32] {
        self.proposed_cid1_payload_sha256
    }

    pub(crate) fn proposal_nonce(&self) -> u64 {
        self.proposal_nonce
    }
}

#[derive(Debug)]
pub enum RecorderRolloverError {
    IllegalPhase(RecorderRolloverPhase),
    Transport(String),
    WrongMessage(MessageKind),
    Codec(String),
    Proposal(&'static str),
    Store(String),
    DrainReceiptMismatch,
    OpenReceiptMismatch,
    InitialHeaderMismatch,
    NewHeaderMismatch,
    Identity(String),
}

impl std::fmt::Display for RecorderRolloverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IllegalPhase(phase) => write!(f, "rollover is illegal in phase {phase:?}"),
            Self::Transport(error) => write!(f, "rollover transport failed: {error}"),
            Self::WrongMessage(kind) => write!(f, "expected RolloverPropose, got {kind:?}"),
            Self::Codec(error) => write!(f, "rollover payload invalid: {error}"),
            Self::Proposal(field) => write!(f, "rollover proposal {field} is invalid"),
            Self::Store(error) => write!(f, "rollover durable store failed: {error}"),
            Self::DrainReceiptMismatch => write!(f, "rollover drain receipt does not match CRP1"),
            Self::OpenReceiptMismatch => write!(f, "rollover open receipt does not match CRP1"),
            Self::InitialHeaderMismatch => {
                write!(f, "initial logical-stream CID1/CFA1 authority mismatch")
            }
            Self::NewHeaderMismatch => write!(f, "new logical-stream CID1/CFA1 mismatch"),
            Self::Identity(error) => write!(f, "rollover identity rebind failed: {error}"),
        }
    }
}

impl std::error::Error for RecorderRolloverError {}

pub struct RecorderRolloverCoordinator {
    phase: RecorderRolloverPhase,
    current_logical_stream_id: u64,
    current_campaign_id: [u8; 32],
    current_cid1_payload_sha256: [u8; 32],
    last_proposal_nonce: u64,
}

impl RecorderRolloverCoordinator {
    pub fn new(initial: &AcknowledgedFirstCompactRecord) -> Result<Self, RecorderRolloverError> {
        let acknowledgement = initial.first_record().acknowledgement();
        let Some(cid1) = initial.first_record().validated_cid1() else {
            return Err(RecorderRolloverError::InitialHeaderMismatch);
        };
        if acknowledgement.mode != StreamIdentityMode::IdentityV1
            || acknowledgement.h4cr_sequence != 0
            || acknowledgement.payload_sha256 != cid1.payload_sha256()
            || initial.durable_receipt().acknowledgement() != acknowledgement
        {
            return Err(RecorderRolloverError::InitialHeaderMismatch);
        }
        Ok(Self {
            phase: RecorderRolloverPhase::Streaming,
            current_logical_stream_id: acknowledgement.logical_stream_id,
            current_campaign_id: cid1.campaign_id(),
            current_cid1_payload_sha256: acknowledgement.payload_sha256,
            last_proposal_nonce: 0,
        })
    }

    pub fn phase(&self) -> RecorderRolloverPhase {
        self.phase
    }

    pub fn receive_drain_seal_open_and_ack<I, S>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        store: &mut S,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<RolloverProposeV1, RecorderRolloverError>
    where
        I: DeadlineControlIo,
        S: RecorderRolloverStore,
    {
        if self.phase != RecorderRolloverPhase::Streaming {
            return Err(RecorderRolloverError::IllegalPhase(self.phase));
        }
        let frame = channel
            .recv_until(deadline, cancellation)
            .map_err(|error| self.fail(RecorderRolloverError::Transport(error.to_string())))?;
        self.receive_frame_drain_seal_open_and_ack(channel, store, frame, deadline, cancellation)
    }

    pub fn receive_frame_drain_seal_open_and_ack<I, S>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        store: &mut S,
        frame: ControlFrame,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<RolloverProposeV1, RecorderRolloverError>
    where
        I: DeadlineControlIo,
        S: RecorderRolloverStore,
    {
        if self.phase != RecorderRolloverPhase::Streaming {
            return Err(RecorderRolloverError::IllegalPhase(self.phase));
        }
        if frame.kind != MessageKind::RolloverPropose {
            channel.poison();
            return Err(self.fail(RecorderRolloverError::WrongMessage(frame.kind)));
        }
        let proposal = decode_rollover_propose(&frame.payload).map_err(|error| {
            channel.poison();
            self.fail(RecorderRolloverError::Codec(error.to_string()))
        })?;
        if proposal.proposed_logical_stream_id
            != self
                .current_logical_stream_id
                .checked_add(1)
                .ok_or_else(|| self.fail(RecorderRolloverError::Proposal("logical id exhausted")))?
        {
            return self.reject(
                channel,
                store,
                proposal,
                1,
                "non-contiguous logical stream id",
                deadline,
                cancellation,
            );
        }
        if proposal.current_cid1_payload_sha256 != self.current_cid1_payload_sha256 {
            return self.reject(
                channel,
                store,
                proposal,
                1,
                "current CID1 hash mismatch",
                deadline,
                cancellation,
            );
        }
        if proposal.current_campaign_id != self.current_campaign_id {
            return self.reject(
                channel,
                store,
                proposal,
                1,
                "current campaign-pair digest mismatch",
                deadline,
                cancellation,
            );
        }
        if proposal.proposal_nonce <= self.last_proposal_nonce {
            return self.reject(
                channel,
                store,
                proposal,
                1,
                "stale/reused proposal nonce",
                deadline,
                cancellation,
            );
        }

        let drained = match store.drain_contiguous_through(&proposal, deadline, cancellation) {
            Ok(receipt) => receipt,
            Err(error) => {
                channel.poison();
                return Err(self.fail(RecorderRolloverError::Store(error)));
            }
        };
        if drained.logical_stream_id != self.current_logical_stream_id
            || drained.last_h4cr_sequence != proposal.last_admitted_h4cr_sequence
            || drained.campaign_id != self.current_campaign_id
            || drained.cid1_payload_sha256 != self.current_cid1_payload_sha256
        {
            channel.poison();
            return Err(self.fail(RecorderRolloverError::DrainReceiptMismatch));
        }
        let opened = match store.durable_seal_and_open(&proposal, drained) {
            Ok(receipt) => receipt,
            Err(error) => {
                channel.poison();
                return Err(self.fail(RecorderRolloverError::Store(error)));
            }
        };
        if opened.sealed_logical_stream_id != self.current_logical_stream_id
            || opened.opened_logical_stream_id != proposal.proposed_logical_stream_id
            || opened.expected_campaign_id != proposal.proposed_campaign_id
            || opened.expected_cid1_payload_sha256 != proposal.proposed_cid1_payload_sha256
        {
            channel.poison();
            return Err(self.fail(RecorderRolloverError::OpenReceiptMismatch));
        }
        let ack = RolloverAckV1 {
            proposed_logical_stream_id: proposal.proposed_logical_stream_id,
            proposal_nonce: proposal.proposal_nonce,
            decision: RolloverDecision::Accept,
            error_code: 0,
        };
        let payload = encode_rollover_ack(&ack)
            .map_err(|error| self.fail(RecorderRolloverError::Codec(error.to_string())))?;
        channel
            .send_until(MessageKind::RolloverAck, &payload, deadline, cancellation)
            .map_err(|error| self.fail(RecorderRolloverError::Transport(error.to_string())))?;
        self.last_proposal_nonce = proposal.proposal_nonce;
        self.phase = RecorderRolloverPhase::AwaitingNewHeader {
            logical_stream_id: proposal.proposed_logical_stream_id,
            campaign_id: proposal.proposed_campaign_id,
            cid1_payload_sha256: proposal.proposed_cid1_payload_sha256,
        };
        Ok(proposal)
    }

    pub fn acknowledge_new_header<I, S>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        identity: &mut RecorderIdentityControlDispatcher<S>,
        first_record: ValidatedFirstCompactRecord,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<AcknowledgedFirstCompactRecord, RecorderRolloverError>
    where
        I: DeadlineControlIo,
        S: IdentityDurableCandidateSink,
    {
        let RecorderRolloverPhase::AwaitingNewHeader {
            logical_stream_id,
            campaign_id,
            cid1_payload_sha256,
        } = self.phase
        else {
            return Err(RecorderRolloverError::IllegalPhase(self.phase));
        };
        let authority = AuthenticatedRolloverRebind {
            current_logical_stream_id: self.current_logical_stream_id,
            current_campaign_id: self.current_campaign_id,
            current_cid1_payload_sha256: self.current_cid1_payload_sha256,
            proposed_logical_stream_id: logical_stream_id,
            proposed_campaign_id: campaign_id,
            proposed_cid1_payload_sha256: cid1_payload_sha256,
            proposal_nonce: self.last_proposal_nonce,
        };
        let acknowledged = match identity.acknowledge_rollover_rebind(
            channel,
            first_record,
            authority,
            deadline,
            cancellation,
        ) {
            Ok(acknowledged) => acknowledged,
            Err(error) => {
                channel.poison();
                return Err(self.fail(RecorderRolloverError::Identity(error.to_string())));
            }
        };
        self.current_logical_stream_id = logical_stream_id;
        self.current_campaign_id = campaign_id;
        self.current_cid1_payload_sha256 = cid1_payload_sha256;
        self.phase = RecorderRolloverPhase::Streaming;
        Ok(acknowledged)
    }

    fn reject<I, S>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        store: &mut S,
        proposal: RolloverProposeV1,
        error_code: u16,
        reason: &str,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<RolloverProposeV1, RecorderRolloverError>
    where
        I: DeadlineControlIo,
        S: RecorderRolloverStore,
    {
        if let Err(error) = store.durable_record_rejection(&proposal, error_code, reason) {
            channel.poison();
            return Err(self.fail(RecorderRolloverError::Store(error)));
        }
        let ack = RolloverAckV1 {
            proposed_logical_stream_id: proposal.proposed_logical_stream_id,
            proposal_nonce: proposal.proposal_nonce,
            decision: RolloverDecision::Reject,
            error_code,
        };
        let payload = encode_rollover_ack(&ack)
            .map_err(|error| self.fail(RecorderRolloverError::Codec(error.to_string())))?;
        channel
            .send_until(MessageKind::RolloverAck, &payload, deadline, cancellation)
            .map_err(|error| self.fail(RecorderRolloverError::Transport(error.to_string())))?;
        channel.poison();
        Err(self.fail(RecorderRolloverError::Proposal("rejected")))
    }

    fn fail(&mut self, error: RecorderRolloverError) -> RecorderRolloverError {
        self.phase = RecorderRolloverPhase::Failed;
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io;
    use std::rc::Rc;
    use std::time::Duration;

    use compact_contract::campaign_identity::{
        encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_RECORD_TYPE_STREAM_HEADER,
    };
    use compact_contract::campaign_rollover::{encode_rollover_propose, RolloverReason};

    use crate::control_transport::{
        claim_session_capability, encode_frame, ControlCapabilities, ControlPeerIdentity,
        EndpointRole, SessionId, ValidatedSessionCapability,
    };

    #[derive(Default)]
    struct TestIo {
        events: Rc<RefCell<Vec<&'static str>>>,
        inbound: VecDeque<u8>,
        outbound: Vec<u8>,
    }

    impl DeadlineControlIo for TestIo {
        fn send_all_until(
            &mut self,
            bytes: &[u8],
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            self.events.borrow_mut().push("send");
            self.outbound.extend_from_slice(bytes);
            Ok(())
        }

        fn recv_exact_until(
            &mut self,
            bytes: &mut [u8],
            _deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            if cancellation.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            for byte in bytes {
                *byte = self
                    .inbound
                    .pop_front()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "mock EOF"))?;
            }
            Ok(())
        }
    }

    struct IdentitySink {
        events: Rc<RefCell<Vec<&'static str>>>,
    }

    impl IdentityDurableCandidateSink for IdentitySink {
        type FirstRecordCandidate = ();
        type IdentityFailureCandidate = ();
        type IdentityFailureGapLedgerCandidate = ();

        fn write_first_record_candidate(
            &mut self,
            _first_record: &ValidatedFirstCompactRecord,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<Self::FirstRecordCandidate, String> {
            self.events.borrow_mut().push("write-first");
            Ok(())
        }

        fn flush_first_record_candidate(
            &mut self,
            _candidate: &mut Self::FirstRecordCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            self.events.borrow_mut().push("flush-first");
            Ok(())
        }

        fn sync_first_record_candidate(
            &mut self,
            _candidate: &mut Self::FirstRecordCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            self.events.borrow_mut().push("sync-first");
            Ok(())
        }

        fn write_identity_failure_candidate(
            &mut self,
            _failure: &compact_contract::identity_control::IdentityFailureV1,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<Self::IdentityFailureCandidate, String> {
            Err("identity failure is outside this test".to_string())
        }

        fn flush_identity_failure_candidate(
            &mut self,
            _candidate: &mut Self::IdentityFailureCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            Err("identity failure is outside this test".to_string())
        }

        fn sync_identity_failure_candidate(
            &mut self,
            _candidate: &mut Self::IdentityFailureCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            Err("identity failure is outside this test".to_string())
        }

        fn write_identity_failure_gap_ledger_candidate(
            &mut self,
            _failure: &compact_contract::identity_control::IdentityFailureV1,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<Self::IdentityFailureGapLedgerCandidate, String> {
            Err("identity failure is outside this test".to_string())
        }

        fn flush_identity_failure_gap_ledger_candidate(
            &mut self,
            _candidate: &mut Self::IdentityFailureGapLedgerCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            Err("identity failure is outside this test".to_string())
        }

        fn sync_identity_failure_gap_ledger_candidate(
            &mut self,
            _candidate: &mut Self::IdentityFailureGapLedgerCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            Err("identity failure is outside this test".to_string())
        }
    }

    struct RolloverStore {
        events: Rc<RefCell<Vec<&'static str>>>,
    }

    impl RecorderRolloverStore for RolloverStore {
        fn drain_contiguous_through(
            &mut self,
            proposal: &RolloverProposeV1,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<DrainedLogicalStream, String> {
            self.events.borrow_mut().push("drain");
            Ok(DrainedLogicalStream {
                logical_stream_id: proposal.proposed_logical_stream_id - 1,
                last_h4cr_sequence: proposal.last_admitted_h4cr_sequence,
                campaign_id: proposal.current_campaign_id,
                cid1_payload_sha256: proposal.current_cid1_payload_sha256,
            })
        }

        fn durable_seal_and_open(
            &mut self,
            proposal: &RolloverProposeV1,
            drained: DrainedLogicalStream,
        ) -> Result<OpenedLogicalStream, String> {
            self.events.borrow_mut().push("seal-open");
            Ok(OpenedLogicalStream {
                sealed_logical_stream_id: drained.logical_stream_id,
                opened_logical_stream_id: proposal.proposed_logical_stream_id,
                expected_campaign_id: proposal.proposed_campaign_id,
                expected_cid1_payload_sha256: proposal.proposed_cid1_payload_sha256,
            })
        }

        fn durable_record_rejection(
            &mut self,
            _proposal: &RolloverProposeV1,
            _error_code: u16,
            _reason: &str,
        ) -> Result<(), String> {
            self.events.borrow_mut().push("reject");
            Ok(())
        }
    }

    fn hour() -> i32 {
        compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24
    }

    fn complete_h4cr(game_hour_raw: i32, payload: &[u8]) -> Vec<u8> {
        let mut record = vec![0u8; compact_contract::COMPACT_RECORD_HEADER_LEN];
        record[0..4].copy_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
        record[4..6].copy_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        record[6..8]
            .copy_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        record[8..16].copy_from_slice(&0u64.to_le_bytes());
        record[16..20].copy_from_slice(&game_hour_raw.to_le_bytes());
        record[20..22].copy_from_slice(&0u16.to_le_bytes());
        record[22..24].copy_from_slice(&COMPACT_RECORD_TYPE_STREAM_HEADER.to_le_bytes());
        record[24..28].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        record.extend_from_slice(payload);
        record
    }

    fn validated_identity(
        logical_stream_id: u64,
        seed: i32,
        observation_hour: i32,
    ) -> ValidatedFirstCompactRecord {
        let observation = Cid1Observation {
            game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".to_string(),
            game_unique_seed: seed,
            game_hour_raw: observation_hour,
        };
        let fields = Cid1Fields::from_stable_observations(
            observation.clone(),
            observation,
            "rollover-transaction-test".to_string(),
            Cid1Witnesses::default(),
        )
        .unwrap();
        let payload = encode_cid1(&fields).unwrap();
        ValidatedFirstCompactRecord::identity_v1(
            logical_stream_id,
            &complete_h4cr(observation_hour, &payload),
        )
        .unwrap()
    }

    fn same_pair_proposal(
        initial: &ValidatedFirstCompactRecord,
        candidate: &ValidatedFirstCompactRecord,
    ) -> RolloverProposeV1 {
        let current = initial.validated_cid1().unwrap();
        let proposed = candidate.validated_cid1().unwrap();
        assert_eq!(current.campaign_id(), proposed.campaign_id());
        assert_ne!(current.payload_sha256(), proposed.payload_sha256());
        RolloverProposeV1 {
            proposed_logical_stream_id: 1,
            last_admitted_h4cr_sequence: 41,
            boundary_game_hour_raw: hour() - 1,
            reason: RolloverReason::SamePairTimeRollback,
            current_campaign_id: current.campaign_id(),
            proposed_campaign_id: proposed.campaign_id(),
            current_cid1_payload_sha256: current.payload_sha256(),
            proposed_cid1_payload_sha256: proposed.payload_sha256(),
            proposal_nonce: 1,
        }
    }

    fn channel_with_proposal(
        session_byte: u8,
        proposal: RolloverProposeV1,
        events: Rc<RefCell<Vec<&'static str>>>,
    ) -> DeadlineControlChannel<TestIo> {
        let bytes = [session_byte; 16];
        let session_id = SessionId::from_bytes(bytes).unwrap();
        let capability = claim_session_capability(bytes).unwrap();
        let peer = ControlPeerIdentity {
            pid: 42,
            creation_filetime: u64::from(session_byte),
        };
        let capabilities = ControlCapabilities::auto_attach_duplex_rollover_v1();
        let admission =
            ValidatedSessionCapability::admit(capability, peer, peer, capabilities, capabilities)
                .unwrap();
        let payload = encode_rollover_propose(&proposal).unwrap();
        let inbound = encode_frame(session_id, 0, MessageKind::RolloverPropose, &payload).unwrap();
        DeadlineControlChannel::new(
            TestIo {
                events,
                inbound: inbound.into(),
                outbound: Vec::new(),
            },
            EndpointRole::Recorder,
            admission,
        )
    }

    #[test]
    fn accepted_same_pair_rollover_rebinds_only_after_durable_second_cfa1() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let initial = validated_identity(0, -1234, hour());
        let candidate = validated_identity(1, -1234, hour() - 1);
        let proposal = same_pair_proposal(&initial, &candidate);
        let mut channel = channel_with_proposal(0xD5, proposal, events.clone());
        let mut identity = RecorderIdentityControlDispatcher::new(IdentitySink {
            events: events.clone(),
        });
        let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
        let cancellation = ControlCancellation::new();
        let initial_acknowledged = identity
            .acknowledge_first_record(&mut channel, initial, deadline, &cancellation)
            .unwrap();
        let mut coordinator = RecorderRolloverCoordinator::new(&initial_acknowledged).unwrap();
        let mut store = RolloverStore {
            events: events.clone(),
        };

        coordinator
            .receive_drain_seal_open_and_ack(&mut channel, &mut store, deadline, &cancellation)
            .unwrap();
        assert!(matches!(
            coordinator.phase(),
            RecorderRolloverPhase::AwaitingNewHeader {
                logical_stream_id: 1,
                ..
            }
        ));
        let proposed_campaign_id = candidate.validated_cid1().unwrap().campaign_id();
        let acknowledged = coordinator
            .acknowledge_new_header(
                &mut channel,
                &mut identity,
                candidate,
                deadline,
                &cancellation,
            )
            .unwrap();

        assert_eq!(coordinator.phase(), RecorderRolloverPhase::Streaming);
        assert_eq!(identity.bound_logical_stream_id(), Some(1));
        assert_eq!(identity.bound_campaign_id(), Some(proposed_campaign_id));
        assert_eq!(
            acknowledged
                .first_record()
                .acknowledgement()
                .logical_stream_id,
            1
        );
        assert!(!identity.is_poisoned());
        assert!(!channel.is_poisoned());
        assert_eq!(
            &*events.borrow(),
            &[
                "write-first",
                "flush-first",
                "sync-first",
                "send",
                "drain",
                "seal-open",
                "send",
                "write-first",
                "flush-first",
                "sync-first",
                "send",
            ]
        );
    }

    #[test]
    fn mismatched_second_cid1_poison_fails_before_rebind_or_second_journal() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let initial = validated_identity(0, -1234, hour());
        let candidate = validated_identity(1, -1234, hour() - 1);
        let wrong_candidate = validated_identity(1, -1234, hour() - 2);
        assert_eq!(
            candidate.validated_cid1().unwrap().campaign_id(),
            wrong_candidate.validated_cid1().unwrap().campaign_id()
        );
        assert_ne!(
            candidate.validated_cid1().unwrap().payload_sha256(),
            wrong_candidate.validated_cid1().unwrap().payload_sha256()
        );
        let proposal = same_pair_proposal(&initial, &candidate);
        let initial_campaign_id = initial.validated_cid1().unwrap().campaign_id();
        let mut channel = channel_with_proposal(0xD6, proposal, events.clone());
        let mut identity = RecorderIdentityControlDispatcher::new(IdentitySink {
            events: events.clone(),
        });
        let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
        let cancellation = ControlCancellation::new();
        let initial_acknowledged = identity
            .acknowledge_first_record(&mut channel, initial, deadline, &cancellation)
            .unwrap();
        let mut coordinator = RecorderRolloverCoordinator::new(&initial_acknowledged).unwrap();
        let mut store = RolloverStore {
            events: events.clone(),
        };
        coordinator
            .receive_drain_seal_open_and_ack(&mut channel, &mut store, deadline, &cancellation)
            .unwrap();

        let error = match coordinator.acknowledge_new_header(
            &mut channel,
            &mut identity,
            wrong_candidate,
            deadline,
            &cancellation,
        ) {
            Ok(_) => panic!("mismatched CID1 unexpectedly rebound the dispatcher"),
            Err(error) => error,
        };

        assert!(matches!(error, RecorderRolloverError::Identity(_)));
        assert_eq!(coordinator.phase(), RecorderRolloverPhase::Failed);
        assert!(channel.is_poisoned());
        assert!(identity.is_poisoned());
        assert_eq!(identity.bound_logical_stream_id(), Some(0));
        assert_eq!(identity.bound_campaign_id(), Some(initial_campaign_id));
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|event| **event == "write-first")
                .count(),
            1
        );
    }
}
