pub mod durable_fs;
pub mod identity_control;
pub mod rollover_control;
pub mod rollover_fs;

use std::collections::BTreeSet;
use std::fmt;

use crate::control_transport::{
    ControlCancellation, ControlDeadline, ControlFrame, DeadlineControlChannel, DeadlineControlIo,
    MessageKind,
};

const PAYLOAD_SCHEMA: u16 = 1;
const HEARTBEAT_CHALLENGE_LEN: usize = 28;
const HEARTBEAT_PROOF_LEN: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum HeartbeatState {
    WaitingForWorld = 1,
    Producing = 2,
    PausedForControl = 3,
}

impl HeartbeatState {
    fn from_wire(value: u16) -> Result<Self, HeartbeatError> {
        match value {
            1 => Ok(Self::WaitingForWorld),
            2 => Ok(Self::Producing),
            3 => Ok(Self::PausedForControl),
            other => Err(HeartbeatError::UnknownState(other)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeartbeatChallenge {
    pub challenge: [u8; 16],
    pub counter: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeartbeatProof {
    pub challenge: [u8; 16],
    pub last_complete_h4cr_sequence: u64,
    pub state: HeartbeatState,
}

#[derive(Debug, Eq, PartialEq)]
pub enum HeartbeatError {
    Transport(String),
    HeartbeatCapabilityMissing,
    OutstandingChallenge,
    NoOutstandingChallenge,
    ZeroChallenge,
    ReusedChallenge,
    CounterMismatch { expected: u64, actual: u64 },
    CounterExhausted,
    WrongMessageKind(MessageKind),
    InvalidPayloadLength { expected: usize, actual: usize },
    UnsupportedSchema(u16),
    NonZeroReserved,
    ChallengeMismatch,
    SequenceRegressed { previous: u64, actual: u64 },
    UnknownState(u16),
}

impl fmt::Display for HeartbeatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(f, "heartbeat transport failed: {error}"),
            Self::HeartbeatCapabilityMissing => {
                write!(f, "heartbeat capability was not admitted")
            }
            Self::OutstandingChallenge => write!(f, "a heartbeat challenge is already outstanding"),
            Self::NoOutstandingChallenge => write!(f, "no heartbeat challenge is outstanding"),
            Self::ZeroChallenge => write!(f, "all-zero heartbeat challenge is invalid"),
            Self::ReusedChallenge => write!(f, "heartbeat challenge was already used"),
            Self::CounterMismatch { expected, actual } => {
                write!(f, "heartbeat counter mismatch: {actual} != {expected}")
            }
            Self::CounterExhausted => write!(f, "heartbeat counter exhausted"),
            Self::WrongMessageKind(kind) => write!(f, "unexpected heartbeat message {kind:?}"),
            Self::InvalidPayloadLength { expected, actual } => {
                write!(f, "heartbeat payload length {actual} != {expected}")
            }
            Self::UnsupportedSchema(schema) => write!(f, "unsupported heartbeat schema {schema}"),
            Self::NonZeroReserved => write!(f, "heartbeat reserved field is nonzero"),
            Self::ChallengeMismatch => write!(f, "heartbeat proof challenge mismatch"),
            Self::SequenceRegressed { previous, actual } => write!(
                f,
                "heartbeat H4CR sequence regressed: {actual} < {previous}"
            ),
            Self::UnknownState(state) => write!(f, "unknown heartbeat state {state}"),
        }
    }
}

impl std::error::Error for HeartbeatError {}

pub fn encode_heartbeat_challenge(
    challenge: HeartbeatChallenge,
) -> Result<[u8; HEARTBEAT_CHALLENGE_LEN], HeartbeatError> {
    if challenge.challenge == [0; 16] {
        return Err(HeartbeatError::ZeroChallenge);
    }
    let mut payload = [0u8; HEARTBEAT_CHALLENGE_LEN];
    payload[0..2].copy_from_slice(&PAYLOAD_SCHEMA.to_le_bytes());
    payload[4..20].copy_from_slice(&challenge.challenge);
    payload[20..28].copy_from_slice(&challenge.counter.to_le_bytes());
    Ok(payload)
}

pub fn decode_heartbeat_challenge(payload: &[u8]) -> Result<HeartbeatChallenge, HeartbeatError> {
    validate_prefix(payload, HEARTBEAT_CHALLENGE_LEN)?;
    let mut challenge = [0u8; 16];
    challenge.copy_from_slice(&payload[4..20]);
    if challenge == [0; 16] {
        return Err(HeartbeatError::ZeroChallenge);
    }
    Ok(HeartbeatChallenge {
        challenge,
        counter: u64::from_le_bytes(payload[20..28].try_into().unwrap()),
    })
}

pub fn encode_heartbeat_proof(
    proof: HeartbeatProof,
) -> Result<[u8; HEARTBEAT_PROOF_LEN], HeartbeatError> {
    if proof.challenge == [0; 16] {
        return Err(HeartbeatError::ZeroChallenge);
    }
    let mut payload = [0u8; HEARTBEAT_PROOF_LEN];
    payload[0..2].copy_from_slice(&PAYLOAD_SCHEMA.to_le_bytes());
    payload[4..20].copy_from_slice(&proof.challenge);
    payload[20..28].copy_from_slice(&proof.last_complete_h4cr_sequence.to_le_bytes());
    payload[28..30].copy_from_slice(&(proof.state as u16).to_le_bytes());
    Ok(payload)
}

pub fn decode_heartbeat_proof(payload: &[u8]) -> Result<HeartbeatProof, HeartbeatError> {
    validate_prefix(payload, HEARTBEAT_PROOF_LEN)?;
    if payload[30..32] != [0, 0] {
        return Err(HeartbeatError::NonZeroReserved);
    }
    let mut challenge = [0u8; 16];
    challenge.copy_from_slice(&payload[4..20]);
    if challenge == [0; 16] {
        return Err(HeartbeatError::ZeroChallenge);
    }
    Ok(HeartbeatProof {
        challenge,
        last_complete_h4cr_sequence: u64::from_le_bytes(payload[20..28].try_into().unwrap()),
        state: HeartbeatState::from_wire(u16::from_le_bytes(payload[28..30].try_into().unwrap()))?,
    })
}

fn validate_prefix(payload: &[u8], expected_len: usize) -> Result<(), HeartbeatError> {
    if payload.len() != expected_len {
        return Err(HeartbeatError::InvalidPayloadLength {
            expected: expected_len,
            actual: payload.len(),
        });
    }
    let schema = u16::from_le_bytes(payload[0..2].try_into().unwrap());
    if schema != PAYLOAD_SCHEMA {
        return Err(HeartbeatError::UnsupportedSchema(schema));
    }
    if payload[2..4] != [0, 0] {
        return Err(HeartbeatError::NonZeroReserved);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct OutstandingChallenge {
    value: HeartbeatChallenge,
    deadline: ControlDeadline,
}

#[derive(Default)]
pub struct RecorderHeartbeat {
    next_counter: u64,
    used_challenges: BTreeSet<[u8; 16]>,
    outstanding: Option<OutstandingChallenge>,
    last_proven_sequence: Option<u64>,
}

impl RecorderHeartbeat {
    pub fn send_challenge<I: DeadlineControlIo>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        challenge: [u8; 16],
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), HeartbeatError> {
        assert_heartbeat_capability(channel)?;
        if self.outstanding.is_some() {
            return Err(HeartbeatError::OutstandingChallenge);
        }
        if self.next_counter == u64::MAX {
            return Err(HeartbeatError::CounterExhausted);
        }
        if challenge == [0; 16] {
            return Err(HeartbeatError::ZeroChallenge);
        }
        if self.used_challenges.contains(&challenge) {
            return Err(HeartbeatError::ReusedChallenge);
        }
        let value = HeartbeatChallenge {
            challenge,
            counter: self.next_counter,
        };
        let payload = encode_heartbeat_challenge(value)?;
        channel
            .send_until(
                MessageKind::HeartbeatChallenge,
                &payload,
                deadline,
                cancellation,
            )
            .map_err(|error| HeartbeatError::Transport(error.to_string()))?;
        self.used_challenges.insert(challenge);
        self.outstanding = Some(OutstandingChallenge { value, deadline });
        Ok(())
    }

    pub fn receive_proof<I: DeadlineControlIo>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        cancellation: &ControlCancellation,
    ) -> Result<HeartbeatProof, HeartbeatError> {
        assert_heartbeat_capability(channel)?;
        let outstanding = self
            .outstanding
            .ok_or(HeartbeatError::NoOutstandingChallenge)?;
        let frame = channel
            .recv_until(outstanding.deadline, cancellation)
            .map_err(|error| HeartbeatError::Transport(error.to_string()))?;
        self.accept_proof_frame(channel, frame)
    }

    pub fn accept_proof_frame<I: DeadlineControlIo>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        frame: ControlFrame,
    ) -> Result<HeartbeatProof, HeartbeatError> {
        assert_heartbeat_capability(channel)?;
        let outstanding = self
            .outstanding
            .ok_or(HeartbeatError::NoOutstandingChallenge)?;
        if frame.kind != MessageKind::HeartbeatProof {
            channel.poison();
            return Err(HeartbeatError::WrongMessageKind(frame.kind));
        }
        let proof = match decode_heartbeat_proof(&frame.payload) {
            Ok(proof) => proof,
            Err(error) => {
                channel.poison();
                return Err(error);
            }
        };
        if proof.challenge != outstanding.value.challenge {
            channel.poison();
            return Err(HeartbeatError::ChallengeMismatch);
        }
        if let Some(previous) = self.last_proven_sequence {
            if proof.last_complete_h4cr_sequence < previous {
                channel.poison();
                return Err(HeartbeatError::SequenceRegressed {
                    previous,
                    actual: proof.last_complete_h4cr_sequence,
                });
            }
        }
        self.outstanding = None;
        self.next_counter += 1;
        self.last_proven_sequence = Some(proof.last_complete_h4cr_sequence);
        Ok(proof)
    }

    pub fn last_proven_sequence(&self) -> Option<u64> {
        self.last_proven_sequence
    }

    /// protection remain continuous for the lifetime of the H4CT session.
    pub fn begin_logical_stream(&mut self) -> Result<(), HeartbeatError> {
        if self.outstanding.is_some() {
            return Err(HeartbeatError::OutstandingChallenge);
        }
        self.last_proven_sequence = None;
        Ok(())
    }
}

#[derive(Default)]
pub struct CollectorHeartbeat {
    next_counter: u64,
    used_challenges: BTreeSet<[u8; 16]>,
}

impl CollectorHeartbeat {
    pub fn receive_and_respond<I: DeadlineControlIo>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        last_complete_h4cr_sequence: u64,
        state: HeartbeatState,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<HeartbeatChallenge, HeartbeatError> {
        assert_heartbeat_capability(channel)?;
        if self.next_counter == u64::MAX {
            return Err(HeartbeatError::CounterExhausted);
        }
        let frame = channel
            .recv_until(deadline, cancellation)
            .map_err(|error| HeartbeatError::Transport(error.to_string()))?;
        if frame.kind != MessageKind::HeartbeatChallenge {
            channel.poison();
            return Err(HeartbeatError::WrongMessageKind(frame.kind));
        }
        let challenge = match decode_heartbeat_challenge(&frame.payload) {
            Ok(challenge) => challenge,
            Err(error) => {
                channel.poison();
                return Err(error);
            }
        };
        if challenge.counter != self.next_counter {
            channel.poison();
            return Err(HeartbeatError::CounterMismatch {
                expected: self.next_counter,
                actual: challenge.counter,
            });
        }
        if !self.used_challenges.insert(challenge.challenge) {
            channel.poison();
            return Err(HeartbeatError::ReusedChallenge);
        }
        let proof = encode_heartbeat_proof(HeartbeatProof {
            challenge: challenge.challenge,
            last_complete_h4cr_sequence,
            state,
        })?;
        channel
            .send_until(MessageKind::HeartbeatProof, &proof, deadline, cancellation)
            .map_err(|error| HeartbeatError::Transport(error.to_string()))?;
        self.next_counter += 1;
        Ok(challenge)
    }
}

fn assert_heartbeat_capability<I: DeadlineControlIo>(
    channel: &mut DeadlineControlChannel<I>,
) -> Result<(), HeartbeatError> {
    channel
        .assert_capability(MessageKind::HeartbeatChallenge)
        .map_err(|_| HeartbeatError::HeartbeatCapabilityMissing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use crate::control_transport::{
        claim_session_capability, encode_frame, ControlCapabilities, ControlPeerIdentity,
        EndpointRole, ValidatedSessionCapability,
    };

    #[derive(Default)]
    struct ScriptIo {
        inbound: VecDeque<u8>,
        outbound: Vec<u8>,
    }

    impl DeadlineControlIo for ScriptIo {
        fn send_all_until(
            &mut self,
            bytes: &[u8],
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            self.outbound.extend_from_slice(bytes);
            Ok(())
        }

        fn recv_exact_until(
            &mut self,
            bytes: &mut [u8],
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            for byte in bytes {
                *byte = self.inbound.pop_front().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "script exhausted")
                })?;
            }
            Ok(())
        }
    }

    fn channel(role: EndpointRole, inbound: Vec<u8>) -> DeadlineControlChannel<ScriptIo> {
        channel_with_capabilities(role, inbound, ControlCapabilities::HEARTBEAT)
    }

    fn channel_with_capabilities(
        role: EndpointRole,
        inbound: Vec<u8>,
        capability_bits: u32,
    ) -> DeadlineControlChannel<ScriptIo> {
        static NEXT: AtomicU64 = AtomicU64::new(10_000);
        let mut id = [0x91; 16];
        id[8..].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
        let session = claim_session_capability(id).unwrap();
        let peer = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9_001,
        };
        let none = ControlCapabilities::from_bits(0).unwrap();
        let observed = ControlCapabilities::from_bits(capability_bits).unwrap();
        let admission =
            ValidatedSessionCapability::admit(session, peer, peer, none, observed).unwrap();
        DeadlineControlChannel::new(
            ScriptIo {
                inbound: inbound.into(),
                outbound: Vec::new(),
            },
            role,
            admission,
        )
    }

    fn recorder_channel_with_proofs(proofs: &[HeartbeatProof]) -> DeadlineControlChannel<ScriptIo> {
        static NEXT: AtomicU64 = AtomicU64::new(20_000);
        let mut id = [0x92; 16];
        id[8..].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
        let session = claim_session_capability(id).unwrap();
        let session_id = session.session_id();
        let mut inbound = Vec::new();
        for (sequence, proof) in proofs.iter().enumerate() {
            inbound.extend_from_slice(
                &encode_frame(
                    session_id,
                    sequence as u64,
                    MessageKind::HeartbeatProof,
                    &encode_heartbeat_proof(*proof).unwrap(),
                )
                .unwrap(),
            );
        }
        let peer = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9_001,
        };
        let heartbeat = ControlCapabilities::from_bits(ControlCapabilities::HEARTBEAT).unwrap();
        let admission =
            ValidatedSessionCapability::admit(session, peer, peer, heartbeat, heartbeat).unwrap();
        DeadlineControlChannel::new(
            ScriptIo {
                inbound: inbound.into(),
                outbound: Vec::new(),
            },
            EndpointRole::Recorder,
            admission,
        )
    }

    #[test]
    fn heartbeat_codecs_reject_trailing_reserved_and_unknown_state() {
        let challenge = HeartbeatChallenge {
            challenge: [7; 16],
            counter: 4,
        };
        assert_eq!(
            decode_heartbeat_challenge(&encode_heartbeat_challenge(challenge).unwrap()).unwrap(),
            challenge
        );

        let proof = HeartbeatProof {
            challenge: [8; 16],
            last_complete_h4cr_sequence: 99,
            state: HeartbeatState::Producing,
        };
        assert_eq!(
            decode_heartbeat_proof(&encode_heartbeat_proof(proof).unwrap()).unwrap(),
            proof
        );

        let mut trailing = encode_heartbeat_proof(proof).unwrap().to_vec();
        trailing.push(0);
        assert!(matches!(
            decode_heartbeat_proof(&trailing),
            Err(HeartbeatError::InvalidPayloadLength { .. })
        ));
        let mut reserved = encode_heartbeat_proof(proof).unwrap();
        reserved[31] = 1;
        assert!(matches!(
            decode_heartbeat_proof(&reserved),
            Err(HeartbeatError::NonZeroReserved)
        ));
        let mut unknown = encode_heartbeat_proof(proof).unwrap();
        unknown[28..30].copy_from_slice(&77u16.to_le_bytes());
        assert!(matches!(
            decode_heartbeat_proof(&unknown),
            Err(HeartbeatError::UnknownState(77))
        ));
    }

    #[test]
    fn recorder_accepts_multiple_fresh_cycles_and_monotonic_progress() {
        let challenges = [[1; 16], [2; 16]];
        let proofs = [
            HeartbeatProof {
                challenge: challenges[0],
                last_complete_h4cr_sequence: 10,
                state: HeartbeatState::WaitingForWorld,
            },
            HeartbeatProof {
                challenge: challenges[1],
                last_complete_h4cr_sequence: 12,
                state: HeartbeatState::Producing,
            },
        ];
        let mut channel = recorder_channel_with_proofs(&proofs);
        let cancellation = ControlCancellation::new();
        let mut heartbeat = RecorderHeartbeat::default();
        for challenge in challenges {
            let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
            heartbeat
                .send_challenge(&mut channel, challenge, deadline, &cancellation)
                .unwrap();
            heartbeat
                .receive_proof(&mut channel, &cancellation)
                .unwrap();
        }
        assert_eq!(heartbeat.last_proven_sequence(), Some(12));
        assert!(!channel.is_poisoned());
        assert!(!channel.io().outbound.is_empty());
    }

    #[test]
    fn stale_proof_and_sequence_regression_poison_session() {
        let mut channel = recorder_channel_with_proofs(&[HeartbeatProof {
            challenge: [9; 16],
            last_complete_h4cr_sequence: 1,
            state: HeartbeatState::Producing,
        }]);
        let cancellation = ControlCancellation::new();
        let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
        let mut heartbeat = RecorderHeartbeat::default();
        heartbeat
            .send_challenge(&mut channel, [8; 16], deadline, &cancellation)
            .unwrap();
        assert!(matches!(
            heartbeat.receive_proof(&mut channel, &cancellation),
            Err(HeartbeatError::ChallengeMismatch)
        ));
        assert!(channel.is_poisoned());

        let challenges = [[3; 16], [4; 16]];
        let proofs = [
            HeartbeatProof {
                challenge: challenges[0],
                last_complete_h4cr_sequence: 10,
                state: HeartbeatState::Producing,
            },
            HeartbeatProof {
                challenge: challenges[1],
                last_complete_h4cr_sequence: 9,
                state: HeartbeatState::Producing,
            },
        ];
        let mut channel = recorder_channel_with_proofs(&proofs);
        let mut heartbeat = RecorderHeartbeat::default();
        for (index, challenge) in challenges.into_iter().enumerate() {
            let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
            heartbeat
                .send_challenge(&mut channel, challenge, deadline, &cancellation)
                .unwrap();
            let result = heartbeat.receive_proof(&mut channel, &cancellation);
            if index == 0 {
                result.unwrap();
            } else {
                assert!(matches!(
                    result,
                    Err(HeartbeatError::SequenceRegressed { .. })
                ));
            }
        }
        assert!(channel.is_poisoned());
    }

    #[test]
    fn collector_rejects_counter_replay_before_sending_cached_proof() {
        static NEXT: AtomicU64 = AtomicU64::new(30_000);
        let mut id = [0x93; 16];
        id[8..].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
        let session = claim_session_capability(id).unwrap();
        let session_id = session.session_id();
        let challenge = HeartbeatChallenge {
            challenge: [6; 16],
            counter: 0,
        };
        let frame0 = encode_frame(
            session_id,
            0,
            MessageKind::HeartbeatChallenge,
            &encode_heartbeat_challenge(challenge).unwrap(),
        )
        .unwrap();
        let frame1 = encode_frame(
            session_id,
            1,
            MessageKind::HeartbeatChallenge,
            &encode_heartbeat_challenge(HeartbeatChallenge {
                challenge: [7; 16],
                counter: 0,
            })
            .unwrap(),
        )
        .unwrap();
        let mut inbound = frame0;
        inbound.extend_from_slice(&frame1);
        let peer = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9_001,
        };
        let cap = ControlCapabilities::from_bits(ControlCapabilities::HEARTBEAT).unwrap();
        let admission = ValidatedSessionCapability::admit(session, peer, peer, cap, cap).unwrap();
        let mut channel = DeadlineControlChannel::new(
            ScriptIo {
                inbound: inbound.into(),
                outbound: Vec::new(),
            },
            EndpointRole::Collector,
            admission,
        );
        let cancellation = ControlCancellation::new();
        let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
        let mut heartbeat = CollectorHeartbeat::default();
        heartbeat
            .receive_and_respond(
                &mut channel,
                5,
                HeartbeatState::Producing,
                deadline,
                &cancellation,
            )
            .unwrap();
        let result = heartbeat.receive_and_respond(
            &mut channel,
            5,
            HeartbeatState::Producing,
            deadline,
            &cancellation,
        );
        assert!(matches!(
            result,
            Err(HeartbeatError::CounterMismatch {
                expected: 1,
                actual: 0
            })
        ));
        assert!(channel.is_poisoned());
    }

    #[test]
    fn no_proof_can_be_received_without_an_outstanding_challenge() {
        let mut channel = channel(EndpointRole::Recorder, Vec::new());
        let mut heartbeat = RecorderHeartbeat::default();
        assert!(matches!(
            heartbeat.receive_proof(&mut channel, &ControlCancellation::new()),
            Err(HeartbeatError::NoOutstandingChallenge)
        ));
        assert!(!channel.is_poisoned());
    }

    #[test]
    fn logical_stream_reset_clears_only_h4cr_frontier() {
        let challenges = [[0x31; 16], [0x32; 16]];
        let proofs = [
            HeartbeatProof {
                challenge: challenges[0],
                last_complete_h4cr_sequence: 41,
                state: HeartbeatState::PausedForControl,
            },
            HeartbeatProof {
                challenge: challenges[1],
                last_complete_h4cr_sequence: 0,
                state: HeartbeatState::WaitingForWorld,
            },
        ];
        let mut channel = recorder_channel_with_proofs(&proofs);
        let cancellation = ControlCancellation::new();
        let mut heartbeat = RecorderHeartbeat::default();
        let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
        heartbeat
            .send_challenge(&mut channel, challenges[0], deadline, &cancellation)
            .unwrap();
        heartbeat
            .receive_proof(&mut channel, &cancellation)
            .unwrap();
        heartbeat.begin_logical_stream().unwrap();
        heartbeat
            .send_challenge(&mut channel, challenges[1], deadline, &cancellation)
            .unwrap();
        heartbeat
            .receive_proof(&mut channel, &cancellation)
            .unwrap();
        assert_eq!(heartbeat.last_proven_sequence(), Some(0));
    }

    #[test]
    fn heartbeat_states_reject_zero_and_wrong_capability_before_state_or_io() {
        let cancellation = ControlCancellation::new();
        for admitted in [0, ControlCapabilities::ROLLOVER] {
            let mut recorder =
                channel_with_capabilities(EndpointRole::Recorder, Vec::new(), admitted);
            let mut recorder_heartbeat = RecorderHeartbeat::default();
            assert!(matches!(
                recorder_heartbeat.send_challenge(
                    &mut recorder,
                    [5; 16],
                    ControlDeadline::after(Duration::from_secs(1)).unwrap(),
                    &cancellation,
                ),
                Err(HeartbeatError::HeartbeatCapabilityMissing)
            ));
            assert!(recorder.io().outbound.is_empty());
            assert!(recorder.is_poisoned());

            let mut collector =
                channel_with_capabilities(EndpointRole::Collector, Vec::new(), admitted);
            let mut collector_heartbeat = CollectorHeartbeat::default();
            assert!(matches!(
                collector_heartbeat.receive_and_respond(
                    &mut collector,
                    0,
                    HeartbeatState::WaitingForWorld,
                    ControlDeadline::after(Duration::from_secs(1)).unwrap(),
                    &cancellation,
                ),
                Err(HeartbeatError::HeartbeatCapabilityMissing)
            ));
            assert!(collector.io().outbound.is_empty());
            assert!(collector.is_poisoned());
        }
    }
}
