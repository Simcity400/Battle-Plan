//! Recorder-owned CRP1/CRA1 rollover transaction.
//!
//! The transaction deliberately lives beside the one H4CT receive router.  It
//! enforces the cross-pipe ordering law: receive and authenticate CRP1, drain
//! the old H4CR stream contiguously through its advertised sequence, durably
//! seal it, open the candidate sink, and only then send an accepting CRA1.
//! Acceptance enters a second barrier; the next logical stream is not admitted
//! until its canonical sequence-zero CID1 CFA1 matches the committed id/hash.

use compact_contract::campaign_rollover::{
    decode_rollover_propose, encode_rollover_ack, RolloverAckV1, RolloverDecision,
    RolloverProposeV1,
};
use compact_contract::first_compact_ack::FirstCompactFrameAckV1;
use compact_contract::stream_validator::StreamIdentityMode;

use crate::control_transport::{
    ControlCancellation, ControlDeadline, DeadlineControlChannel, DeadlineControlIo, MessageKind,
};

/// Durable proof returned after the old H4CR source was read contiguously.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainedLogicalStream {
    pub logical_stream_id: u64,
    pub last_h4cr_sequence: u64,
    pub cid1_payload_sha256: [u8; 32],
}

/// Durable proof returned only after the old source is sealed and the new sink
/// exists.  Implementations must not return while either operation is buffered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenedLogicalStream {
    pub sealed_logical_stream_id: u64,
    pub opened_logical_stream_id: u64,
    pub expected_cid1_payload_sha256: [u8; 32],
}

pub trait RecorderRolloverStore {
    fn drain_contiguous_through(
        &mut self,
        proposal: &RolloverProposeV1,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<DrainedLogicalStream, String>;

    /// Fsync/seal the drained old source and create+fsync the candidate new sink.
    /// The returned receipt is checked exactly before CRA1 acceptance is sent.
    fn durable_seal_and_open(
        &mut self,
        proposal: &RolloverProposeV1,
        drained: DrainedLogicalStream,
    ) -> Result<OpenedLogicalStream, String>;

    /// Persist a terminal rejected proposal before the rejection is sent.
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
        cid1_payload_sha256: [u8; 32],
    },
    Failed,
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
    NewHeaderMismatch,
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
            Self::NewHeaderMismatch => write!(f, "new logical-stream CID1/CFA1 mismatch"),
        }
    }
}

impl std::error::Error for RecorderRolloverError {}

pub struct RecorderRolloverCoordinator {
    phase: RecorderRolloverPhase,
    current_logical_stream_id: u64,
    current_cid1_payload_sha256: [u8; 32],
    last_proposal_nonce: u64,
}

impl RecorderRolloverCoordinator {
    pub fn new(initial_cid1_payload_sha256: [u8; 32]) -> Self {
        Self {
            phase: RecorderRolloverPhase::Streaming,
            current_logical_stream_id: 0,
            current_cid1_payload_sha256: initial_cid1_payload_sha256,
            last_proposal_nonce: 0,
        }
    }

    pub fn phase(&self) -> RecorderRolloverPhase {
        self.phase
    }

    /// Receive one CRP1 and execute the complete durable-before-ack boundary.
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
            cid1_payload_sha256: proposal.proposed_cid1_payload_sha256,
        };
        Ok(proposal)
    }

    /// Complete the second barrier after the ordinary durable CFA1 dispatcher
    /// has written and sent its acknowledgement.
    pub fn admit_new_header(
        &mut self,
        acknowledgement: FirstCompactFrameAckV1,
    ) -> Result<(), RecorderRolloverError> {
        let RecorderRolloverPhase::AwaitingNewHeader {
            logical_stream_id,
            cid1_payload_sha256,
        } = self.phase
        else {
            return Err(RecorderRolloverError::IllegalPhase(self.phase));
        };
        if acknowledgement.mode != StreamIdentityMode::IdentityV1
            || acknowledgement.logical_stream_id != logical_stream_id
            || acknowledgement.h4cr_sequence != 0
            || acknowledgement.payload_sha256 != cid1_payload_sha256
        {
            return Err(self.fail(RecorderRolloverError::NewHeaderMismatch));
        }
        self.current_logical_stream_id = logical_stream_id;
        self.current_cid1_payload_sha256 = cid1_payload_sha256;
        self.phase = RecorderRolloverPhase::Streaming;
        Ok(())
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
        store
            .durable_record_rejection(&proposal, error_code, reason)
            .map_err(|error| self.fail(RecorderRolloverError::Store(error)))?;
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
