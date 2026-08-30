#![allow(dead_code)]

pub mod rollover;

use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::io;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use compact_contract::campaign_identity::ValidatedCid1;
use compact_contract::campaign_rollover::{
    decode_rollover_ack, encode_rollover_propose, RolloverAckV1, RolloverCodecError,
    RolloverProposeV1, RolloverReason,
};
use compact_contract::control_transport::payloads::{
    decode_heartbeat_challenge, encode_collector_hello_with_capabilities, encode_heartbeat_proof,
    CollectorHelloV1, HeartbeatProofV1, PayloadError,
};
use compact_contract::control_transport::{
    ControlCapabilities, ControlChannel, ControlError, ControlIo, EndpointRole, MessageKind,
    SessionCapability, CONTROL_VERSION,
};
use compact_contract::early_render_content_manifest::{
    decode_early_render_content_manifest_chunk, decode_early_render_content_manifest_commit,
    EarlyRenderContentManifestCodecError,
};
use compact_contract::early_render_control::{
    canonical_early_diagnostic_liveness_response_binding_bytes,
    decode_early_commit_capture_receipt, decode_early_diagnostic_end,
    decode_early_diagnostic_liveness_request, decode_early_render_launch_bind_v2,
    encode_early_diagnostic_liveness_response, encode_early_first_game_d3d,
    encode_early_hook_ready, EarlyCommitCaptureReceiptV1, EarlyDiagnosticLivenessResponseV1,
    EarlyFirstGameD3dV1, EarlyHookReadyV1, EarlyRenderCodecError,
    EARLY_DIAGNOSTIC_END_RECORDER_CANCEL, EARLY_DIAGNOSTIC_MIN_HOLD_MS,
};
use compact_contract::early_render_runtime_dlc::{
    encode_early_render_runtime_dlc_roster, EarlyRenderRuntimeDlcCodecError,
    EarlyRenderRuntimeDlcRosterV1,
};
use compact_contract::early_render_startup_failure::{
    encode_early_render_startup_failure, EarlyRenderStartupFailureCodecError,
    EarlyRenderStartupFailureV1, EarlyRenderStartupPhase,
};
use compact_contract::early_render_vfs::{
    canonical_early_render_vfs_index_bytes, decode_early_render_vfs_index_chunk,
    decode_early_render_vfs_index_commit, EarlyRenderVfsCodecError, EarlyRenderVfsWinnerRowV1,
};
use compact_contract::early_render_vfs_payload::{
    decode_early_render_vfs_payload_chunk, decode_early_render_vfs_payload_commit,
    encode_early_render_vfs_payload_request, EarlyRenderVfsPayloadCodecError,
    EarlyRenderVfsPayloadRequestV1,
};
use compact_contract::first_compact_ack::{
    decode_first_compact_frame_ack as decode_canonical_first_compact_frame_ack,
    FirstCompactAckCodecError, FirstCompactFrameAckV1,
};
use compact_contract::identity_control::{
    decode_identity_failure_ack, encode_identity_failure, IdentityControlCodecError,
    IdentityFailureAckV1, IdentityFailureV1,
};
use compact_contract::sink_failure::{
    encode_sink_failure, SinkFailureKind, SinkFailureReport, SinkStatusError,
};

pub const DUPLEX_CONTROL_CLIENT_ENABLED: bool = true;

pub const ROLLOVER_LIVE_LIFECYCLE_EVIDENCE_BOUND: bool = true;

pub fn evidence_or_product_identity_enabled() -> bool {
    (DUPLEX_CONTROL_CLIENT_ENABLED
        && compact_contract::build_profile::DOMAIN_CAMPAIGN_ID_LIVE_READER_ENABLED)
        || compact_contract::cid1_lifecycle_evidence::cid1_lifecycle_evidence_permit_enabled()
}

pub fn product_rollover_enabled() -> bool {
    (evidence_or_product_identity_enabled() && ROLLOVER_LIVE_LIFECYCLE_EVIDENCE_BOUND)
        || compact_contract::cid1_lifecycle_evidence::cid1_lifecycle_evidence_permit_enabled()
}

pub struct DuplexClientProductPermit {
    _private: (),
}

pub struct EarlyRenderDuplexProductPermit {
    _private: (),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderControlBinding {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub timeout_ms: u32,
    pub launch_manifest_sha256: [u8; 32],
    pub effective_vfs_manifest_sha256: [u8; 32],
    pub effective_vfs_index_sha256: [u8; 32],
    pub effective_vfs_index_row_count: u32,
    pub effective_vfs_index_chunk_count: u32,
    pub content_manifest_bytes_sha256: [u8; 32],
    pub content_manifest_byte_count: u64,
    pub content_manifest_chunk_count: u32,
    pub session_id: [u8; 16],
    pub launch_bind_sha256: [u8; 32],
    pub executable_sha256: [u8; 32],
    pub collector_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedEarlyRenderDiagnosticHoldProof {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub session_id: [u8; 16],
    pub launch_manifest_sha256: [u8; 32],
    pub hook_epoch_nonce: u64,
    pub staged_hook_generation: u64,
    pub staged_hook_group_mask: u64,
    pub staged_hook_set_sha256: [u8; 32],
    pub observed_hold_ms: u64,
    pub sampled_qpc: u64,
    pub request_sha256: [u8; 32],
    pub response_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiagnosticReadinessSample {
    process_id: u32,
    process_start_nonce: u64,
    hook_epoch_nonce: u64,
    d3d_device_created_qpc: u64,
    first_creation_sequence: u64,
    qpc_frequency_hz: u64,
    staged_hook_generation: u64,
    staged_hook_count: u32,
    diagnostic_stage_probe: bool,
    staged_hook_group_mask: u64,
    staged_hook_set_sha256: [u8; 32],
    game_device_ptr: u64,
    immediate_context_ptr: u64,
    sampled_qpc: u64,
}

fn decode_canonical_lower_sha256(value: &str) -> Result<[u8; 32], ClientError> {
    if value.len() != 64 {
        return Err(ClientError::EarlyRenderBinding);
    }
    let mut out = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        out[index] = digit(pair[0])
            .zip(digit(pair[1]))
            .map(|(high, low)| (high << 4) | low)
            .ok_or(ClientError::EarlyRenderBinding)?;
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedEarlyRenderContentManifest {
    pub bytes_sha256: [u8; 32],
    pub manifest: portable_content_manifest::ContentManifest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedEarlyRenderVfsIndex {
    pub process_start_nonce: u64,
    pub effective_vfs_manifest_sha256: [u8; 32],
    pub ordered_rows_sha256: [u8; 32],
    pub ordered_rows: Vec<EarlyRenderVfsWinnerRowV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedEarlyRenderVfsPayload {
    pub winner: EarlyRenderVfsWinnerRowV1,
    pub payload_offset: u64,
    pub payload_range_sha256: [u8; 32],
    pub bytes: Vec<u8>,
}

pub fn early_render_duplex_product_permit() -> EarlyRenderDuplexProductPermit {
    EarlyRenderDuplexProductPermit { _private: () }
}

pub fn duplex_client_product_permit() -> Result<DuplexClientProductPermit, ClientError> {
    if evidence_or_product_identity_enabled() {
        Ok(DuplexClientProductPermit { _private: () })
    } else {
        Err(ClientError::ActivationDisabled)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_duplex_client_permit() -> DuplexClientProductPermit {
    DuplexClientProductPermit { _private: () }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct MonotonicInstant(pub u128);

pub trait MonotonicClock {
    fn now(&self) -> MonotonicInstant;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MonotonicDeadline(MonotonicInstant);

const MAX_WAIT_NAMED_PIPE_MS: u128 = (u32::MAX as u128) - 1;

impl MonotonicDeadline {
    pub fn from_now<C: MonotonicClock>(clock: &C, budget: Duration) -> Self {
        Self(MonotonicInstant(
            clock.now().0.saturating_add(budget.as_nanos()),
        ))
    }

    pub fn instant(&self) -> MonotonicInstant {
        self.0
    }

    pub fn is_expired(&self, now: MonotonicInstant) -> bool {
        now >= self.0
    }

    pub fn remaining(&self, now: MonotonicInstant) -> Option<Duration> {
        if now >= self.0 {
            None
        } else {
            Some(Duration::from_nanos(
                (self.0 .0 - now.0).min(u64::MAX as u128) as u64,
            ))
        }
    }

    pub fn wait_named_pipe_timeout_ms(&self, now: MonotonicInstant) -> Option<u32> {
        if now >= self.0 {
            return None;
        }
        let remaining_ns = self.0 .0 - now.0;
        let ms = remaining_ns.div_ceil(1_000_000);
        if ms == 0 {
            return None;
        }
        Some(ms.min(MAX_WAIT_NAMED_PIPE_MS) as u32)
    }
}

#[derive(Clone, Copy)]
pub struct SystemClock;

impl SystemClock {
    pub fn new() -> Self {
        SystemClock
    }
}

fn system_clock_origin() -> std::time::Instant {
    static ORIGIN: OnceLock<std::time::Instant> = OnceLock::new();
    *ORIGIN.get_or_init(std::time::Instant::now)
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonotonicClock for SystemClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant(system_clock_origin().elapsed().as_nanos())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DuplexIoError {
    TimedOut,
    PeerClosed,
    Truncated,
    Disconnected,
}

impl DuplexIoError {
    fn into_io(self) -> io::Error {
        let (kind, msg): (io::ErrorKind, &str) = match self {
            Self::TimedOut => (io::ErrorKind::TimedOut, "control deadline expired"),
            Self::PeerClosed => (
                io::ErrorKind::ConnectionAborted,
                "peer closed control pipe at frame boundary",
            ),
            Self::Truncated => (io::ErrorKind::UnexpectedEof, "truncated control frame"),
            Self::Disconnected => (io::ErrorKind::BrokenPipe, "control pipe disconnected"),
        };
        io::Error::new(kind, msg)
    }
}

pub trait DeadlineControlIo {
    fn send_all(&mut self, bytes: &[u8], deadline: MonotonicDeadline) -> Result<(), DuplexIoError>;
    fn recv_exact(
        &mut self,
        bytes: &mut [u8],
        deadline: MonotonicDeadline,
    ) -> Result<(), DuplexIoError>;
}

struct DeadlineBridge<T: DeadlineControlIo> {
    inner: T,
    deadline: Rc<Cell<MonotonicDeadline>>,
}

impl<T: DeadlineControlIo> ControlIo for DeadlineBridge<T> {
    fn send_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.inner
            .send_all(bytes, self.deadline.get())
            .map_err(DuplexIoError::into_io)
    }

    fn recv_exact(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        self.inner
            .recv_exact(bytes, self.deadline.get())
            .map_err(DuplexIoError::into_io)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CollectorIdentity {
    pid: u32,
    creation_filetime: u64,
    h4cr_version: u16,
    dll_sha256: [u8; 32],
    challenge: [u8; 16],
}

impl CollectorIdentity {
    pub fn from_attach(
        pid: u32,
        creation_filetime: u64,
        h4cr_version: u16,
        dll_sha256: [u8; 32],
        challenge: [u8; 16],
    ) -> Self {
        Self {
            pid,
            creation_filetime,
            h4cr_version,
            dll_sha256,
            challenge,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DuplexClientPolicy {
    pub handshake_budget: Duration,
    pub heartbeat_cycle: Duration,
    pub heartbeat_liveness: Duration,
    pub first_frame_budget: Duration,
}

impl DuplexClientPolicy {
    pub const DEFAULT: Self = Self {
        handshake_budget: Duration::from_secs(30),
        heartbeat_cycle: Duration::from_secs(5),
        heartbeat_liveness: Duration::from_secs(15),
        first_frame_budget: Duration::from_secs(300),
    };
}

impl Default for DuplexClientPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub use compact_contract::control_transport::payloads::HeartbeatState as ProductionState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientPhase {
    Handshaking,
    Attached,
    Producing,
    AwaitingRolloverAck,
    AwaitingNewHeaderAck,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientEvent {
    HeartbeatServed {
        host_challenge_counter: u64,
        reported_state: u16,
    },
    FirstFrameAcked(FirstCompactFrameAckV1),
    IdentityFailureAcked(IdentityFailureAckV1),
    GracefulStop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientOutcome {
    StoppedByRequest,
    StoppedByPeer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientError {
    Control(ControlError),
    Payload(PayloadError),
    SinkStatus(SinkStatusError),
    IdentityCodec(IdentityControlCodecError),
    RolloverCodec(RolloverCodecError),
    RolloverState(rollover::CollectorRolloverError),
    FirstCompactAckCodec(FirstCompactAckCodecError),
    EarlyRenderCodec(EarlyRenderCodecError),
    EarlyRenderContentManifestCodec(EarlyRenderContentManifestCodecError),
    EarlyRenderVfsCodec(EarlyRenderVfsCodecError),
    EarlyRenderVfsPayloadCodec(EarlyRenderVfsPayloadCodecError),
    EarlyRenderRuntimeDlcCodec(EarlyRenderRuntimeDlcCodecError),
    EarlyRenderStartupFailureCodec(EarlyRenderStartupFailureCodecError),
    EarlyRenderBinding,
    EarlyRenderLocal(String),
    EarlyRenderDiagnosticHoldTooShort { observed_ms: u64, required_ms: u64 },
    EarlyRenderDiagnosticHoldCompleted(AuthenticatedEarlyRenderDiagnosticHoldProof),
    Unsupported(MessageKind),
    ActivationDisabled,
    NotUsable,
}

impl From<ControlError> for ClientError {
    fn from(err: ControlError) -> Self {
        ClientError::Control(err)
    }
}

impl From<PayloadError> for ClientError {
    fn from(err: PayloadError) -> Self {
        ClientError::Payload(err)
    }
}

impl From<SinkStatusError> for ClientError {
    fn from(err: SinkStatusError) -> Self {
        ClientError::SinkStatus(err)
    }
}

impl From<IdentityControlCodecError> for ClientError {
    fn from(err: IdentityControlCodecError) -> Self {
        ClientError::IdentityCodec(err)
    }
}

impl From<RolloverCodecError> for ClientError {
    fn from(err: RolloverCodecError) -> Self {
        ClientError::RolloverCodec(err)
    }
}

impl From<rollover::CollectorRolloverError> for ClientError {
    fn from(err: rollover::CollectorRolloverError) -> Self {
        ClientError::RolloverState(err)
    }
}

impl From<FirstCompactAckCodecError> for ClientError {
    fn from(err: FirstCompactAckCodecError) -> Self {
        ClientError::FirstCompactAckCodec(err)
    }
}

impl From<EarlyRenderCodecError> for ClientError {
    fn from(err: EarlyRenderCodecError) -> Self {
        ClientError::EarlyRenderCodec(err)
    }
}

impl From<EarlyRenderContentManifestCodecError> for ClientError {
    fn from(err: EarlyRenderContentManifestCodecError) -> Self {
        ClientError::EarlyRenderContentManifestCodec(err)
    }
}

impl From<EarlyRenderVfsCodecError> for ClientError {
    fn from(err: EarlyRenderVfsCodecError) -> Self {
        ClientError::EarlyRenderVfsCodec(err)
    }
}

impl From<EarlyRenderVfsPayloadCodecError> for ClientError {
    fn from(err: EarlyRenderVfsPayloadCodecError) -> Self {
        ClientError::EarlyRenderVfsPayloadCodec(err)
    }
}

impl From<EarlyRenderRuntimeDlcCodecError> for ClientError {
    fn from(err: EarlyRenderRuntimeDlcCodecError) -> Self {
        ClientError::EarlyRenderRuntimeDlcCodec(err)
    }
}

impl From<EarlyRenderStartupFailureCodecError> for ClientError {
    fn from(err: EarlyRenderStartupFailureCodecError) -> Self {
        ClientError::EarlyRenderStartupFailureCodec(err)
    }
}

pub struct CollectorControlClient<T: DeadlineControlIo, C: MonotonicClock> {
    channel: ControlChannel<DeadlineBridge<T>>,
    clock: C,
    deadline: Rc<Cell<MonotonicDeadline>>,
    identity: CollectorIdentity,
    policy: DuplexClientPolicy,
    phase: ClientPhase,
    first_frame_acked: bool,
    last_h4cr_sequence: u64,
    reported_state: ProductionState,
    early_render_binding: Option<EarlyRenderControlBinding>,
    early_render_content_manifest_admitted: bool,
    early_render_vfs_admitted: bool,
    next_early_render_vfs_payload_request_id: u64,
    early_render_first_game_d3d_sent: bool,
    early_render_diagnostic_stage_probe: bool,
    early_render_runtime_dlc_sent: bool,
    early_render_capture_committed: bool,
    ordinary_capabilities: ControlCapabilities,
}

impl<T: DeadlineControlIo, C: MonotonicClock> CollectorControlClient<T, C> {
    pub fn new(
        _permit: &DuplexClientProductPermit,
        io: T,
        clock: C,
        session: SessionCapability,
        identity: CollectorIdentity,
        policy: DuplexClientPolicy,
    ) -> Self {
        Self::new_inner(
            io,
            clock,
            session,
            identity,
            policy,
            None,
            ControlCapabilities::auto_attach_duplex_v1(),
        )
    }

    pub fn new_product(
        permit: &DuplexClientProductPermit,
        io: T,
        clock: C,
        session: SessionCapability,
        identity: CollectorIdentity,
        policy: DuplexClientPolicy,
    ) -> Self {
        if product_rollover_enabled() {
            Self::new_inner(
                io,
                clock,
                session,
                identity,
                policy,
                None,
                ControlCapabilities::auto_attach_duplex_rollover_v1(),
            )
        } else {
            Self::new(permit, io, clock, session, identity, policy)
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn new_rollover_offline(
        _permit: &DuplexClientProductPermit,
        io: T,
        clock: C,
        session: SessionCapability,
        identity: CollectorIdentity,
        policy: DuplexClientPolicy,
    ) -> Self {
        Self::new_inner(
            io,
            clock,
            session,
            identity,
            policy,
            None,
            ControlCapabilities::auto_attach_duplex_rollover_v1(),
        )
    }

    pub fn new_early_render(
        _permit: &EarlyRenderDuplexProductPermit,
        io: T,
        clock: C,
        session: SessionCapability,
        identity: CollectorIdentity,
        policy: DuplexClientPolicy,
    ) -> Self {
        Self::new_inner(
            io,
            clock,
            session,
            identity,
            policy,
            None,
            ControlCapabilities::early_render_duplex_v1(),
        )
    }

    fn new_inner(
        io: T,
        clock: C,
        session: SessionCapability,
        identity: CollectorIdentity,
        policy: DuplexClientPolicy,
        early_render_binding: Option<EarlyRenderControlBinding>,
        ordinary_capabilities: ControlCapabilities,
    ) -> Self {
        let deadline = Rc::new(Cell::new(MonotonicDeadline(clock.now())));
        let bridge = DeadlineBridge {
            inner: io,
            deadline: Rc::clone(&deadline),
        };
        Self {
            channel: ControlChannel::new(bridge, EndpointRole::Collector, session),
            clock,
            deadline,
            identity,
            policy,
            phase: ClientPhase::Handshaking,
            first_frame_acked: false,
            last_h4cr_sequence: 0,
            reported_state: ProductionState::WaitingForWorld,
            early_render_binding,
            early_render_content_manifest_admitted: false,
            early_render_vfs_admitted: false,
            next_early_render_vfs_payload_request_id: 1,
            early_render_first_game_d3d_sent: false,
            early_render_diagnostic_stage_probe: false,
            early_render_runtime_dlc_sent: false,
            early_render_capture_committed: false,
            ordinary_capabilities,
        }
    }

    pub fn phase(&self) -> ClientPhase {
        self.phase
    }

    pub fn first_frame_acked(&self) -> bool {
        self.first_frame_acked
    }

    pub fn set_production_progress(&mut self, last_h4cr_sequence: u64, state: ProductionState) {
        self.last_h4cr_sequence = last_h4cr_sequence;
        self.reported_state = state;
    }

    fn set_deadline(&self, deadline: MonotonicDeadline) {
        self.deadline.set(deadline);
    }

    fn fail<R>(&mut self, error: ClientError) -> Result<R, ClientError> {
        self.phase = ClientPhase::Failed;
        Err(error)
    }

    fn recv_frame(
        &mut self,
        deadline: MonotonicDeadline,
    ) -> Result<Option<compact_contract::control_transport::ControlFrame>, ClientError> {
        self.set_deadline(deadline);
        match self.channel.recv() {
            Ok(frame) => Ok(Some(frame)),
            Err(ControlError::Io(kind)) if kind == io::ErrorKind::ConnectionAborted => Ok(None),
            Err(err) => self.fail(ClientError::Control(err)),
        }
    }

    fn send_frame(
        &mut self,
        kind: MessageKind,
        payload: &[u8],
        deadline: MonotonicDeadline,
    ) -> Result<(), ClientError> {
        self.set_deadline(deadline);
        if let Err(err) = self.channel.send(kind, payload) {
            return self.fail(ClientError::Control(err));
        }
        Ok(())
    }

    pub fn perform_handshake(
        &mut self,
        _permit: &DuplexClientProductPermit,
        handshake_deadline: MonotonicDeadline,
    ) -> Result<(), ClientError> {
        if self.phase != ClientPhase::Handshaking {
            return Err(ClientError::NotUsable);
        }
        let payload = encode_collector_hello_with_capabilities(
            &CollectorHelloV1 {
                pid: self.identity.pid,
                creation_filetime: self.identity.creation_filetime,
                h4cr_version: self.identity.h4cr_version,
                control_version: CONTROL_VERSION,
                dll_sha256: self.identity.dll_sha256,
                challenge: self.identity.challenge,
            },
            self.ordinary_capabilities.bits(),
        );
        self.send_frame(MessageKind::CollectorHello, &payload, handshake_deadline)?;
        self.phase = ClientPhase::Attached;
        Ok(())
    }

    pub fn perform_early_render_handshake(
        &mut self,
        _permit: &EarlyRenderDuplexProductPermit,
        handshake_deadline: MonotonicDeadline,
    ) -> Result<(), ClientError> {
        if self.phase != ClientPhase::Handshaking || self.early_render_binding.is_some() {
            return Err(ClientError::NotUsable);
        }
        let payload = encode_collector_hello_with_capabilities(
            &CollectorHelloV1 {
                pid: self.identity.pid,
                creation_filetime: self.identity.creation_filetime,
                h4cr_version: self.identity.h4cr_version,
                control_version: CONTROL_VERSION,
                dll_sha256: self.identity.dll_sha256,
                challenge: self.identity.challenge,
            },
            ControlCapabilities::early_render_duplex_v1().bits(),
        );
        self.send_frame(MessageKind::CollectorHello, &payload, handshake_deadline)?;
        self.phase = ClientPhase::Attached;
        Ok(())
    }

    pub fn receive_early_render_launch_bind(
        &mut self,
        deadline: MonotonicDeadline,
    ) -> Result<EarlyRenderControlBinding, ClientError> {
        if self.phase != ClientPhase::Attached || self.early_render_binding.is_some() {
            return Err(ClientError::NotUsable);
        }
        let frame = self.recv_frame(deadline)?.ok_or(ClientError::NotUsable)?;
        if frame.kind != MessageKind::EarlyRenderLaunchBind {
            return self.fail(ClientError::Unsupported(frame.kind));
        }
        let wire = decode_early_render_launch_bind_v2(&frame.payload)?;
        if wire.process_id != self.identity.pid
            || wire.process_start_nonce != self.identity.creation_filetime
            || wire.collector_sha256 != self.identity.dll_sha256
        {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        let binding = EarlyRenderControlBinding {
            process_id: wire.process_id,
            process_start_nonce: wire.process_start_nonce,
            timeout_ms: wire.timeout_ms,
            launch_manifest_sha256: wire.launch_manifest_sha256,
            effective_vfs_manifest_sha256: wire.effective_vfs_manifest_sha256,
            effective_vfs_index_sha256: wire.effective_vfs_index_sha256,
            effective_vfs_index_row_count: wire.effective_vfs_index_row_count,
            effective_vfs_index_chunk_count: wire.effective_vfs_index_chunk_count,
            content_manifest_bytes_sha256: wire.content_manifest_bytes_sha256,
            content_manifest_byte_count: wire.content_manifest_byte_count,
            content_manifest_chunk_count: wire.content_manifest_chunk_count,
            session_id: *frame.session_id.as_bytes(),
            launch_bind_sha256: Sha256::digest(&frame.payload).into(),
            executable_sha256: wire.executable_sha256,
            collector_sha256: wire.collector_sha256,
        };
        self.early_render_binding = Some(binding.clone());
        Ok(binding)
    }

    pub fn report_early_render_startup_failure(
        &mut self,
        _permit: &EarlyRenderDuplexProductPermit,
        detail: &str,
        deadline: MonotonicDeadline,
    ) -> Result<(), ClientError> {
        if matches!(self.phase, ClientPhase::Handshaking | ClientPhase::Stopped) {
            return Err(ClientError::NotUsable);
        }
        let (phase, launch_bind_sha256) = match self.early_render_binding.as_ref() {
            None => (EarlyRenderStartupPhase::PostHelloPreBind, None),
            Some(binding) if self.early_render_capture_committed => (
                EarlyRenderStartupPhase::PostCommitBootstrap,
                Some(binding.launch_bind_sha256),
            ),
            Some(binding) if self.early_render_first_game_d3d_sent => (
                EarlyRenderStartupPhase::PostFirstPreCommit,
                Some(binding.launch_bind_sha256),
            ),
            Some(binding) => (
                EarlyRenderStartupPhase::PostBindPreFirst,
                Some(binding.launch_bind_sha256),
            ),
        };
        let report = EarlyRenderStartupFailureV1::new(
            self.identity.pid,
            self.identity.creation_filetime,
            self.identity.challenge,
            phase,
            launch_bind_sha256,
            detail,
        )?;
        let payload = encode_early_render_startup_failure(&report)?;
        let result = self.send_frame(MessageKind::EarlyRenderStartupFailure, &payload, deadline);
        self.phase = ClientPhase::Failed;
        result
    }

    pub fn receive_early_render_content_manifest(
        &mut self,
        _permit: &EarlyRenderDuplexProductPermit,
        binding: &EarlyRenderControlBinding,
        deadline: MonotonicDeadline,
    ) -> Result<AdmittedEarlyRenderContentManifest, ClientError> {
        if self.phase != ClientPhase::Attached
            || self.early_render_binding.as_ref() != Some(binding)
            || self.early_render_content_manifest_admitted
            || self.early_render_vfs_admitted
        {
            return Err(ClientError::NotUsable);
        }
        let capacity = usize::try_from(binding.content_manifest_byte_count)
            .map_err(|_| ClientError::EarlyRenderBinding)?;
        let mut bytes = Vec::with_capacity(capacity);
        for expected_chunk in 0..binding.content_manifest_chunk_count {
            let frame = self.recv_frame(deadline)?.ok_or(ClientError::NotUsable)?;
            if frame.kind != MessageKind::EarlyRenderContentManifestChunk {
                return self.fail(ClientError::Unsupported(frame.kind));
            }
            let chunk = match decode_early_render_content_manifest_chunk(&frame.payload) {
                Ok(value) => value,
                Err(error) => {
                    return self.fail(ClientError::EarlyRenderContentManifestCodec(error));
                }
            };
            if chunk.process_id != binding.process_id
                || chunk.process_start_nonce != binding.process_start_nonce
                || chunk.session_id != binding.session_id
                || chunk.launch_bind_sha256 != binding.launch_bind_sha256
                || chunk.effective_vfs_manifest_sha256 != binding.effective_vfs_manifest_sha256
                || chunk.effective_vfs_index_sha256 != binding.effective_vfs_index_sha256
                || chunk.content_manifest_bytes_sha256 != binding.content_manifest_bytes_sha256
                || chunk.chunk_index != expected_chunk
                || chunk.chunk_count != binding.content_manifest_chunk_count
                || chunk.byte_offset != bytes.len() as u64
                || chunk.total_bytes != binding.content_manifest_byte_count
            {
                return self.fail(ClientError::EarlyRenderBinding);
            }
            bytes.extend_from_slice(&chunk.bytes);
        }
        if bytes.len() != capacity {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        let frame = self.recv_frame(deadline)?.ok_or(ClientError::NotUsable)?;
        if frame.kind != MessageKind::EarlyRenderContentManifestCommit {
            return self.fail(ClientError::Unsupported(frame.kind));
        }
        let commit = match decode_early_render_content_manifest_commit(&frame.payload) {
            Ok(value) => value,
            Err(error) => {
                return self.fail(ClientError::EarlyRenderContentManifestCodec(error));
            }
        };
        let observed_sha256: [u8; 32] = Sha256::digest(&bytes).into();
        if commit.process_id != binding.process_id
            || commit.process_start_nonce != binding.process_start_nonce
            || commit.session_id != binding.session_id
            || commit.launch_bind_sha256 != binding.launch_bind_sha256
            || commit.effective_vfs_manifest_sha256 != binding.effective_vfs_manifest_sha256
            || commit.effective_vfs_index_sha256 != binding.effective_vfs_index_sha256
            || commit.content_manifest_bytes_sha256 != binding.content_manifest_bytes_sha256
            || commit.chunk_count != binding.content_manifest_chunk_count
            || commit.total_bytes != binding.content_manifest_byte_count
            || observed_sha256 != binding.content_manifest_bytes_sha256
        {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        let manifest = match portable_content_manifest::ContentManifest::decode_json(&bytes) {
            Ok(value) => value,
            Err(_) => return self.fail(ClientError::EarlyRenderBinding),
        };
        let vfs_sha256 = binding
            .effective_vfs_manifest_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let executable_sha256 = binding
            .executable_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if manifest.manifest_digest_sha256.is_none()
            || manifest.encode_json().ok().as_deref() != Some(bytes.as_slice())
            || manifest.source_game.executable_sha256 != executable_sha256
            || manifest.effective_vfs_manifest_sha256 != vfs_sha256
            || !manifest.required_content.is_empty()
            || manifest
                .ordered_layers
                .iter()
                .any(|layer| layer.required_for_playback)
            || manifest
                .dlc_selection
                .iter()
                .any(|dlc| dlc.required_for_playback)
        {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        self.early_render_content_manifest_admitted = true;
        Ok(AdmittedEarlyRenderContentManifest {
            bytes_sha256: observed_sha256,
            manifest,
        })
    }

    pub fn receive_early_render_vfs_index(
        &mut self,
        _permit: &EarlyRenderDuplexProductPermit,
        binding: &EarlyRenderControlBinding,
        deadline: MonotonicDeadline,
    ) -> Result<AdmittedEarlyRenderVfsIndex, ClientError> {
        if self.phase != ClientPhase::Attached
            || self.early_render_binding.as_ref() != Some(binding)
            || !self.early_render_content_manifest_admitted
            || self.early_render_vfs_admitted
        {
            return Err(ClientError::NotUsable);
        }
        let mut rows = Vec::with_capacity(binding.effective_vfs_index_row_count as usize);
        for expected_chunk in 0..binding.effective_vfs_index_chunk_count {
            let frame = self.recv_frame(deadline)?.ok_or(ClientError::NotUsable)?;
            if frame.kind != MessageKind::EarlyRenderVfsIndexChunk {
                return self.fail(ClientError::Unsupported(frame.kind));
            }
            let chunk = match decode_early_render_vfs_index_chunk(&frame.payload) {
                Ok(value) => value,
                Err(error) => return self.fail(ClientError::EarlyRenderVfsCodec(error)),
            };
            let expected_first =
                u32::try_from(rows.len()).map_err(|_| ClientError::EarlyRenderBinding)?;
            if chunk.process_id != binding.process_id
                || chunk.process_start_nonce != binding.process_start_nonce
                || chunk.session_id != binding.session_id
                || chunk.launch_bind_sha256 != binding.launch_bind_sha256
                || chunk.index_sha256 != binding.effective_vfs_index_sha256
                || chunk.chunk_index != expected_chunk
                || chunk.chunk_count != binding.effective_vfs_index_chunk_count
                || chunk.first_row_index != expected_first
                || chunk.total_row_count != binding.effective_vfs_index_row_count
            {
                return self.fail(ClientError::EarlyRenderBinding);
            }
            if rows
                .last()
                .is_some_and(|prior: &EarlyRenderVfsWinnerRowV1| {
                    prior.normalized_game_relative_path
                        >= chunk.rows[0].normalized_game_relative_path
                })
            {
                return self.fail(ClientError::EarlyRenderBinding);
            }
            rows.extend(chunk.rows);
        }
        if rows.len() != binding.effective_vfs_index_row_count as usize {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        let frame = self.recv_frame(deadline)?.ok_or(ClientError::NotUsable)?;
        if frame.kind != MessageKind::EarlyRenderVfsIndexCommit {
            return self.fail(ClientError::Unsupported(frame.kind));
        }
        let commit = match decode_early_render_vfs_index_commit(&frame.payload) {
            Ok(value) => value,
            Err(error) => return self.fail(ClientError::EarlyRenderVfsCodec(error)),
        };
        if commit.process_id != binding.process_id
            || commit.process_start_nonce != binding.process_start_nonce
            || commit.session_id != binding.session_id
            || commit.launch_bind_sha256 != binding.launch_bind_sha256
            || commit.index_sha256 != binding.effective_vfs_index_sha256
            || commit.chunk_count != binding.effective_vfs_index_chunk_count
            || commit.total_row_count != binding.effective_vfs_index_row_count
        {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        let canonical = match canonical_early_render_vfs_index_bytes(&rows) {
            Ok(value) => value,
            Err(error) => return self.fail(ClientError::EarlyRenderVfsCodec(error)),
        };
        let observed_sha256: [u8; 32] = Sha256::digest(canonical).into();
        if observed_sha256 != binding.effective_vfs_index_sha256 {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        self.early_render_vfs_admitted = true;
        Ok(AdmittedEarlyRenderVfsIndex {
            process_start_nonce: binding.process_start_nonce,
            effective_vfs_manifest_sha256: binding.effective_vfs_manifest_sha256,
            ordered_rows_sha256: observed_sha256,
            ordered_rows: rows,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn request_early_render_vfs_payload_range(
        &mut self,
        _permit: &EarlyRenderDuplexProductPermit,
        binding: &EarlyRenderControlBinding,
        index: &AdmittedEarlyRenderVfsIndex,
        winner: &EarlyRenderVfsWinnerRowV1,
        payload_offset: u64,
        payload_count: u64,
        expected_payload_range_sha256: [u8; 32],
        deadline: MonotonicDeadline,
    ) -> Result<AdmittedEarlyRenderVfsPayload, ClientError> {
        if self.phase != ClientPhase::Attached
            || self.early_render_binding.as_ref() != Some(binding)
            || !self.early_render_vfs_admitted
            || !self.early_render_capture_committed
            || index.process_start_nonce != binding.process_start_nonce
            || index.effective_vfs_manifest_sha256 != binding.effective_vfs_manifest_sha256
            || index.ordered_rows_sha256 != binding.effective_vfs_index_sha256
            || index
                .ordered_rows
                .binary_search_by(|row| {
                    row.normalized_game_relative_path
                        .cmp(&winner.normalized_game_relative_path)
                })
                .ok()
                .and_then(|position| index.ordered_rows.get(position))
                != Some(winner)
        {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        let request_id = self.next_early_render_vfs_payload_request_id;
        self.next_early_render_vfs_payload_request_id = self
            .next_early_render_vfs_payload_request_id
            .checked_add(1)
            .ok_or(ClientError::EarlyRenderBinding)?;
        let request = EarlyRenderVfsPayloadRequestV1 {
            process_id: binding.process_id,
            process_start_nonce: binding.process_start_nonce,
            session_id: binding.session_id,
            launch_bind_sha256: binding.launch_bind_sha256,
            effective_vfs_manifest_sha256: binding.effective_vfs_manifest_sha256,
            effective_vfs_index_sha256: binding.effective_vfs_index_sha256,
            request_id,
            payload_offset,
            payload_count,
            expected_payload_range_sha256,
            winner: winner.clone(),
        };
        let payload = encode_early_render_vfs_payload_request(&request)?;
        self.send_frame(
            MessageKind::EarlyRenderVfsPayloadRequest,
            &payload,
            deadline,
        )?;

        let capacity =
            usize::try_from(payload_count).map_err(|_| ClientError::EarlyRenderBinding)?;
        let mut bytes = Vec::with_capacity(capacity);
        let mut expected_chunk_count = None;
        let mut expected_chunk_index = 0u32;
        loop {
            let frame = self.recv_frame(deadline)?.ok_or(ClientError::NotUsable)?;
            if frame.kind != MessageKind::EarlyRenderVfsPayloadChunk {
                return self.fail(ClientError::Unsupported(frame.kind));
            }
            let chunk = match decode_early_render_vfs_payload_chunk(&frame.payload) {
                Ok(value) => value,
                Err(error) => {
                    return self.fail(ClientError::EarlyRenderVfsPayloadCodec(error));
                }
            };
            let expected_offset = bytes.len() as u64;
            if chunk.process_id != binding.process_id
                || chunk.process_start_nonce != binding.process_start_nonce
                || chunk.session_id != binding.session_id
                || chunk.launch_bind_sha256 != binding.launch_bind_sha256
                || chunk.effective_vfs_manifest_sha256 != binding.effective_vfs_manifest_sha256
                || chunk.effective_vfs_index_sha256 != binding.effective_vfs_index_sha256
                || chunk.request_id != request_id
                || chunk.payload_offset != payload_offset
                || chunk.payload_count != payload_count
                || chunk.expected_payload_range_sha256 != expected_payload_range_sha256
                || chunk.chunk_index != expected_chunk_index
                || chunk.chunk_offset != expected_offset
                || expected_chunk_count.is_some_and(|count| count != chunk.chunk_count)
            {
                return self.fail(ClientError::EarlyRenderBinding);
            }
            expected_chunk_count = Some(chunk.chunk_count);
            expected_chunk_index += 1;
            bytes.extend_from_slice(&chunk.bytes);
            if expected_chunk_index == chunk.chunk_count {
                break;
            }
        }
        if bytes.len() != capacity {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        let frame = self.recv_frame(deadline)?.ok_or(ClientError::NotUsable)?;
        if frame.kind != MessageKind::EarlyRenderVfsPayloadCommit {
            return self.fail(ClientError::Unsupported(frame.kind));
        }
        let commit = match decode_early_render_vfs_payload_commit(&frame.payload) {
            Ok(value) => value,
            Err(error) => return self.fail(ClientError::EarlyRenderVfsPayloadCodec(error)),
        };
        let observed_sha256: [u8; 32] = Sha256::digest(&bytes).into();
        if commit.process_id != binding.process_id
            || commit.process_start_nonce != binding.process_start_nonce
            || commit.session_id != binding.session_id
            || commit.launch_bind_sha256 != binding.launch_bind_sha256
            || commit.effective_vfs_manifest_sha256 != binding.effective_vfs_manifest_sha256
            || commit.effective_vfs_index_sha256 != binding.effective_vfs_index_sha256
            || commit.request_id != request_id
            || commit.payload_offset != payload_offset
            || commit.payload_count != payload_count
            || commit.payload_range_sha256 != expected_payload_range_sha256
            || commit.payload_range_sha256 != observed_sha256
            || Some(commit.chunk_count) != expected_chunk_count
        {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        Ok(AdmittedEarlyRenderVfsPayload {
            winner: winner.clone(),
            payload_offset,
            payload_range_sha256: observed_sha256,
            bytes,
        })
    }

    pub fn send_early_render_ready(
        &mut self,
        proof: &EarlyHookReadyV1,
        deadline: MonotonicDeadline,
    ) -> Result<(), ClientError> {
        let binding = self
            .early_render_binding
            .as_ref()
            .ok_or(ClientError::EarlyRenderBinding)?;
        if !self.early_render_vfs_admitted
            || proof.effective_vfs_manifest_sha256 != binding.effective_vfs_manifest_sha256
            || proof.effective_vfs_index_sha256 != binding.effective_vfs_index_sha256
            || proof.effective_vfs_index_row_count != binding.effective_vfs_index_row_count
            || proof.effective_vfs_index_chunk_count != binding.effective_vfs_index_chunk_count
            || proof.content_manifest_bytes_sha256 != binding.content_manifest_bytes_sha256
            || proof.content_manifest_byte_count != binding.content_manifest_byte_count
            || proof.content_manifest_chunk_count != binding.content_manifest_chunk_count
        {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        self.validate_early_binding(
            proof.process_id,
            proof.process_start_nonce,
            proof.hook_epoch_nonce,
            Some(&proof.launch_manifest_sha256),
        )?;
        let payload = encode_early_hook_ready(proof)?;
        self.send_frame(MessageKind::EarlyRenderReady, &payload, deadline)
    }

    pub fn send_early_render_first_game_d3d(
        &mut self,
        proof: &EarlyFirstGameD3dV1,
        deadline: MonotonicDeadline,
    ) -> Result<(), ClientError> {
        if self.early_render_first_game_d3d_sent || self.early_render_runtime_dlc_sent {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        self.validate_early_binding(
            proof.process_id,
            proof.process_start_nonce,
            proof.hook_epoch_nonce,
            Some(&proof.launch_manifest_sha256),
        )?;
        let payload = encode_early_first_game_d3d(proof)?;
        self.send_frame(MessageKind::EarlyRenderFirstGameD3d, &payload, deadline)?;
        self.early_render_first_game_d3d_sent = true;
        self.early_render_diagnostic_stage_probe = proof.diagnostic_stage_probe;
        Ok(())
    }

    pub fn send_early_render_runtime_dlc_roster(
        &mut self,
        proof: &EarlyRenderRuntimeDlcRosterV1,
        deadline: MonotonicDeadline,
    ) -> Result<(), ClientError> {
        let binding = self
            .early_render_binding
            .as_ref()
            .ok_or(ClientError::EarlyRenderBinding)?;
        if !self.early_render_first_game_d3d_sent
            || self.early_render_runtime_dlc_sent
            || self.early_render_capture_committed
            || proof.launch_manifest_sha256 != binding.launch_manifest_sha256
            || proof.effective_vfs_manifest_sha256 != binding.effective_vfs_manifest_sha256
        {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        self.validate_early_binding(
            proof.process_id,
            proof.process_start_nonce,
            proof.hook_epoch_nonce,
            Some(&proof.launch_manifest_sha256),
        )?;
        let payload = encode_early_render_runtime_dlc_roster(proof)?;
        self.send_frame(MessageKind::EarlyRenderRuntimeDlcRoster, &payload, deadline)?;
        self.early_render_runtime_dlc_sent = true;
        Ok(())
    }

    pub fn receive_early_render_commit(
        &mut self,
        expected_hook_epoch_nonce: u64,
        deadline: MonotonicDeadline,
    ) -> Result<EarlyCommitCaptureReceiptV1, ClientError> {
        if !self.early_render_first_game_d3d_sent || !self.early_render_runtime_dlc_sent {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        if self.early_render_diagnostic_stage_probe {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        let frame = self.recv_frame(deadline)?.ok_or(ClientError::NotUsable)?;
        if frame.kind != MessageKind::EarlyRenderCommitReceipt {
            return self.fail(ClientError::Unsupported(frame.kind));
        }
        let commit = decode_early_commit_capture_receipt(&frame.payload)?;
        self.validate_early_binding(
            commit.process_id,
            commit.process_start_nonce,
            commit.hook_epoch_nonce,
            None,
        )?;
        if commit.hook_epoch_nonce != expected_hook_epoch_nonce {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        self.early_render_capture_committed = true;
        Ok(commit)
    }

    fn complete_authenticated_diagnostic_hold<F>(
        &mut self,
        binding: &EarlyRenderControlBinding,
        first: &EarlyFirstGameD3dV1,
        hold_started: MonotonicInstant,
        sample_readiness: F,
    ) -> Result<AuthenticatedEarlyRenderDiagnosticHoldProof, ClientError>
    where
        F: FnOnce() -> Result<DiagnosticReadinessSample, ClientError>,
    {
        if self.early_render_binding.as_ref() != Some(binding)
            || !self.early_render_first_game_d3d_sent
            || !self.early_render_runtime_dlc_sent
            || !self.early_render_diagnostic_stage_probe
            || self.early_render_capture_committed
            || !first.diagnostic_stage_probe
        {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        self.validate_early_binding(
            first.process_id,
            first.process_start_nonce,
            first.hook_epoch_nonce,
            Some(&first.launch_manifest_sha256),
        )?;

        let recorder_owned_end = MonotonicDeadline(MonotonicInstant(u128::MAX));
        let Some(request_frame) = self.recv_frame(recorder_owned_end)? else {
            return self.fail(ClientError::NotUsable);
        };
        if request_frame.kind != MessageKind::EarlyRenderDiagnosticLivenessRequest {
            return self.fail(ClientError::Unsupported(request_frame.kind));
        }
        let request = match decode_early_diagnostic_liveness_request(&request_frame.payload) {
            Ok(request) => request,
            Err(error) => return self.fail(ClientError::EarlyRenderCodec(error)),
        };
        let request_matches = request.session_id == binding.session_id
            && request.launch_manifest_sha256 == binding.launch_manifest_sha256
            && request.process_id == binding.process_id
            && request.process_start_nonce == binding.process_start_nonce
            && request.hook_epoch_nonce == first.hook_epoch_nonce
            && request.game_device_ordinal == first.game_device_ordinal
            && request.first_creation_sequence == first.first_creation_sequence
            && request.creation_qpc == first.creation_qpc
            && request.staged_hook_generation == first.staged_hook_generation
            && request.staged_hook_count == first.staged_hook_count
            && request.staged_hook_group_mask == first.staged_hook_group_mask
            && request.staged_hook_set_sha256 == first.staged_hook_set_sha256
            && request.game_device_ptr == first.game_device_ptr
            && request.immediate_context_ptr == first.immediate_context_ptr
            && request.required_hold_ms >= EARLY_DIAGNOSTIC_MIN_HOLD_MS;
        if !request_matches {
            return self.fail(ClientError::EarlyRenderBinding);
        }

        let hold_sampled = self.clock.now();
        let observed_hold_ms = hold_sampled
            .0
            .checked_sub(hold_started.0)
            .and_then(|elapsed_ns| u64::try_from(elapsed_ns / 1_000_000).ok())
            .ok_or(ClientError::EarlyRenderBinding)?;
        if observed_hold_ms < request.required_hold_ms {
            return self.fail(ClientError::EarlyRenderDiagnosticHoldTooShort {
                observed_ms: observed_hold_ms,
                required_ms: request.required_hold_ms,
            });
        }

        let sample = match sample_readiness() {
            Ok(sample) => sample,
            Err(error) => return self.fail(error),
        };
        let sample_matches = sample.process_id == request.process_id
            && sample.process_start_nonce == request.process_start_nonce
            && sample.hook_epoch_nonce == request.hook_epoch_nonce
            && sample.d3d_device_created_qpc == request.creation_qpc
            && sample.first_creation_sequence == request.first_creation_sequence
            && sample.qpc_frequency_hz != 0
            && sample.staged_hook_generation == request.staged_hook_generation
            && sample.staged_hook_count == request.staged_hook_count
            && sample.diagnostic_stage_probe
            && sample.staged_hook_group_mask == request.staged_hook_group_mask
            && sample.staged_hook_set_sha256 == request.staged_hook_set_sha256
            && sample.game_device_ptr == request.game_device_ptr
            && sample.immediate_context_ptr == request.immediate_context_ptr
            && sample.sampled_qpc >= request.creation_qpc;
        if !sample_matches {
            return self.fail(ClientError::EarlyRenderBinding);
        }

        let request_sha256 = Sha256::digest(&request_frame.payload).into();
        let mut response = EarlyDiagnosticLivenessResponseV1 {
            session_id: request.session_id,
            process_id: request.process_id,
            process_start_nonce: request.process_start_nonce,
            hook_epoch_nonce: request.hook_epoch_nonce,
            request_nonce: request.request_nonce,
            request_sha256,
            sampled_qpc: sample.sampled_qpc,
            qpc_frequency_hz: sample.qpc_frequency_hz,
            observed_hold_ms,
            staged_hook_generation: sample.staged_hook_generation,
            staged_hook_count: sample.staged_hook_count,
            staged_hook_group_mask: sample.staged_hook_group_mask,
            staged_hook_set_sha256: sample.staged_hook_set_sha256,
            game_device_ptr: sample.game_device_ptr,
            immediate_context_ptr: sample.immediate_context_ptr,
            response_sha256: [0; 32],
        };
        let response_binding =
            match canonical_early_diagnostic_liveness_response_binding_bytes(&response) {
                Ok(binding) => binding,
                Err(error) => return self.fail(ClientError::EarlyRenderCodec(error)),
            };
        response.response_sha256 = Sha256::digest(&response_binding).into();
        let response_payload = match encode_early_diagnostic_liveness_response(&response) {
            Ok(payload) => payload,
            Err(error) => return self.fail(ClientError::EarlyRenderCodec(error)),
        };
        self.send_frame(
            MessageKind::EarlyRenderDiagnosticLivenessResponse,
            &response_payload,
            recorder_owned_end,
        )?;

        let Some(end_frame) = self.recv_frame(recorder_owned_end)? else {
            return self.fail(ClientError::NotUsable);
        };
        if end_frame.kind != MessageKind::EarlyRenderDiagnosticEnd {
            return self.fail(ClientError::Unsupported(end_frame.kind));
        }
        let end = match decode_early_diagnostic_end(&end_frame.payload) {
            Ok(end) => end,
            Err(error) => return self.fail(ClientError::EarlyRenderCodec(error)),
        };
        if end.session_id != request.session_id
            || end.process_id != request.process_id
            || end.process_start_nonce != request.process_start_nonce
            || end.hook_epoch_nonce != request.hook_epoch_nonce
            || end.request_nonce != request.request_nonce
            || end.response_sha256 != response.response_sha256
            || end.disposition != EARLY_DIAGNOSTIC_END_RECORDER_CANCEL
        {
            return self.fail(ClientError::EarlyRenderBinding);
        }

        self.phase = ClientPhase::Stopped;
        Ok(AuthenticatedEarlyRenderDiagnosticHoldProof {
            process_id: binding.process_id,
            process_start_nonce: binding.process_start_nonce,
            session_id: binding.session_id,
            launch_manifest_sha256: binding.launch_manifest_sha256,
            hook_epoch_nonce: first.hook_epoch_nonce,
            staged_hook_generation: first.staged_hook_generation,
            staged_hook_group_mask: first.staged_hook_group_mask,
            staged_hook_set_sha256: first.staged_hook_set_sha256,
            observed_hold_ms,
            sampled_qpc: sample.sampled_qpc,
            request_sha256,
            response_sha256: response.response_sha256,
        })
    }

    fn validate_early_binding(
        &mut self,
        process_id: u32,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
        launch_manifest_sha256: Option<&[u8; 32]>,
    ) -> Result<(), ClientError> {
        let valid = self.early_render_binding.as_ref().is_some_and(|binding| {
            process_id == binding.process_id
                && process_start_nonce == binding.process_start_nonce
                && hook_epoch_nonce != 0
                && launch_manifest_sha256.is_none_or(|hash| hash == &binding.launch_manifest_sha256)
        });
        if !valid {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        Ok(())
    }

    #[cfg(windows)]
    pub fn complete_early_render_after_bind(
        &mut self,
        _permit: &EarlyRenderDuplexProductPermit,
        binding: &EarlyRenderControlBinding,
        first_game_d3d_deadline: MonotonicDeadline,
        commit_deadline: MonotonicDeadline,
    ) -> Result<EarlyCommitCaptureReceiptV1, ClientError> {
        if self.early_render_binding.as_ref() != Some(binding) {
            return self.fail(ClientError::EarlyRenderBinding);
        }
        let ready = crate::early_render_capture_control::ready_proof(binding)
            .map_err(|_| ClientError::EarlyRenderBinding)?;
        let hook_epoch_nonce = ready.hook_epoch_nonce;
        self.send_early_render_ready(&ready, first_game_d3d_deadline)?;
        let first = loop {
            if first_game_d3d_deadline.is_expired(self.clock.now()) {
                return self.fail(ClientError::Control(ControlError::Io(
                    io::ErrorKind::TimedOut,
                )));
            }
            match crate::early_render_capture_control::first_game_d3d_proof(
                binding,
                hook_epoch_nonce,
            ) {
                Ok(Some(first)) => break first,
                Ok(None) => thread::yield_now(),
                Err(error) => return self.fail(ClientError::EarlyRenderLocal(error)),
            }
        };
        self.send_early_render_first_game_d3d(&first, first_game_d3d_deadline)?;
        let runtime_dlc = loop {
            if commit_deadline.is_expired(self.clock.now()) {
                return self.fail(ClientError::Control(ControlError::Io(
                    io::ErrorKind::TimedOut,
                )));
            }
            match crate::early_render_capture_control::runtime_dlc_roster_proof(
                binding,
                hook_epoch_nonce,
                first.staged_hook_generation,
                first.game_device_ptr,
            ) {
                Ok(Some(proof)) => break proof,
                Ok(None) => thread::yield_now(),
                Err(_) => return self.fail(ClientError::EarlyRenderBinding),
            }
        };
        self.send_early_render_runtime_dlc_roster(&runtime_dlc, commit_deadline)?;
        if first.diagnostic_stage_probe {
            let hold_started = self.clock.now();
            let proof = self.complete_authenticated_diagnostic_hold(
                binding,
                &first,
                hold_started,
                || {
                    let live = production_render_submit_live_lane::a729_hook::early_capture_liveness_sample()
                        .map_err(|_| ClientError::EarlyRenderBinding)?;
                    let readiness = live.readiness;
                    Ok(DiagnosticReadinessSample {
                        process_id: readiness.process.process_id,
                        process_start_nonce: readiness.process.process_start_nonce,
                        hook_epoch_nonce: readiness.hook_epoch_nonce,
                        d3d_device_created_qpc: readiness
                            .d3d_device_created_qpc
                            .ok_or(ClientError::EarlyRenderBinding)?,
                        first_creation_sequence: readiness
                            .first_creation_sequence
                            .ok_or(ClientError::EarlyRenderBinding)?,
                        qpc_frequency_hz: readiness.qpc_frequency_hz,
                        staged_hook_generation: readiness.staged_hook_generation,
                        staged_hook_count: readiness.staged_hook_count,
                        diagnostic_stage_probe: readiness.diagnostic_stage_probe,
                        staged_hook_group_mask: readiness.staged_hook_group_mask,
                        staged_hook_set_sha256: decode_canonical_lower_sha256(
                            &readiness.staged_hook_set_sha256,
                        )?,
                        game_device_ptr: u64::try_from(readiness.game_device_ptr)
                            .map_err(|_| ClientError::EarlyRenderBinding)?,
                        immediate_context_ptr: u64::try_from(readiness.immediate_context_ptr)
                            .map_err(|_| ClientError::EarlyRenderBinding)?,
                        sampled_qpc: live.sampled_qpc,
                    })
                },
            )?;
            return Err(ClientError::EarlyRenderDiagnosticHoldCompleted(proof));
        }
        let commit = self.receive_early_render_commit(hook_epoch_nonce, commit_deadline)?;
        crate::early_render_capture_control::commit_capture_receipt(binding, &commit)
            .map_err(|_| ClientError::EarlyRenderBinding)?;
        Ok(commit)
    }

    pub fn report_sink_failure(
        &mut self,
        report: &SinkFailureReport,
        deadline: MonotonicDeadline,
    ) -> Result<(), ClientError> {
        let pre_hello = match self.phase {
            ClientPhase::Handshaking => true,
            ClientPhase::Attached
            | ClientPhase::Producing
            | ClientPhase::AwaitingRolloverAck
            | ClientPhase::AwaitingNewHeaderAck => false,
            ClientPhase::Stopped | ClientPhase::Failed => return Err(ClientError::NotUsable),
        };
        let payload = match encode_sink_failure(report) {
            Ok(payload) => payload,
            Err(err) => return self.fail(ClientError::SinkStatus(err)),
        };
        let kind = match report.kind {
            SinkFailureKind::Setup => MessageKind::SinkSetupFailure,
            SinkFailureKind::Terminal => MessageKind::SinkTerminalFailure,
        };
        self.send_frame(kind, &payload, deadline)?;
        if pre_hello {
            self.phase = ClientPhase::Failed;
        }
        Ok(())
    }

    pub fn report_identity_failure(
        &mut self,
        failure: &IdentityFailureV1,
        deadline: MonotonicDeadline,
    ) -> Result<(), ClientError> {
        if !matches!(self.phase, ClientPhase::Attached | ClientPhase::Producing) {
            return Err(ClientError::NotUsable);
        }
        let payload = match encode_identity_failure(failure) {
            Ok(payload) => payload,
            Err(err) => return self.fail(ClientError::IdentityCodec(err)),
        };
        self.send_frame(MessageKind::IdentityFailure, &payload, deadline)
    }

    pub fn start_rollover<S: rollover::CollectorRolloverSink>(
        &mut self,
        coordinator: &mut rollover::CollectorRolloverCoordinator,
        sink: &mut S,
        candidate: ValidatedCid1,
        boundary_game_hour_raw: i32,
        reason: RolloverReason,
        proposal_nonce: u64,
        deadline: MonotonicDeadline,
    ) -> Result<RolloverProposeV1, ClientError> {
        if self.phase != ClientPhase::Producing
            || self.ordinary_capabilities != ControlCapabilities::auto_attach_duplex_rollover_v1()
        {
            return Err(ClientError::NotUsable);
        }
        let proposal = match coordinator.prepare_proposal(
            sink,
            candidate,
            boundary_game_hour_raw,
            reason,
            proposal_nonce,
        ) {
            Ok(proposal) => proposal,
            Err(error) => return self.fail(ClientError::RolloverState(error)),
        };
        let payload = match encode_rollover_propose(&proposal) {
            Ok(payload) => payload,
            Err(error) => return self.fail(ClientError::RolloverCodec(error)),
        };
        self.reported_state = ProductionState::PausedForControl;
        self.send_frame(MessageKind::RolloverPropose, &payload, deadline)?;
        self.phase = ClientPhase::AwaitingRolloverAck;
        Ok(proposal)
    }

    pub fn receive_rollover_ack<S: rollover::CollectorRolloverSink>(
        &mut self,
        coordinator: &mut rollover::CollectorRolloverCoordinator,
        sink: &mut S,
        deadline: MonotonicDeadline,
    ) -> Result<RolloverAckV1, ClientError> {
        if self.phase != ClientPhase::AwaitingRolloverAck {
            return Err(ClientError::NotUsable);
        }
        let frame = match self.recv_frame(deadline)? {
            Some(frame) => frame,
            None => return self.fail(ClientError::NotUsable),
        };
        if frame.kind != MessageKind::RolloverAck {
            return self.fail(ClientError::Unsupported(frame.kind));
        }
        let acknowledgement = match decode_rollover_ack(&frame.payload) {
            Ok(value) => value,
            Err(error) => return self.fail(ClientError::RolloverCodec(error)),
        };
        if let Err(error) = coordinator.accept_rollover_ack(sink, acknowledgement) {
            return self.fail(ClientError::RolloverState(error));
        }
        self.first_frame_acked = false;
        self.last_h4cr_sequence = 0;
        self.phase = ClientPhase::AwaitingNewHeaderAck;
        Ok(acknowledgement)
    }

    /// then release ordinary world production.
    pub fn receive_rollover_header_ack<S: rollover::CollectorRolloverSink>(
        &mut self,
        coordinator: &mut rollover::CollectorRolloverCoordinator,
        sink: &mut S,
        deadline: MonotonicDeadline,
    ) -> Result<compact_contract::first_compact_ack::FirstCompactFrameAckV1, ClientError> {
        if self.phase != ClientPhase::AwaitingNewHeaderAck {
            return Err(ClientError::NotUsable);
        }
        let frame = match self.recv_frame(deadline)? {
            Some(frame) => frame,
            None => return self.fail(ClientError::NotUsable),
        };
        if frame.kind != MessageKind::FirstCompactFrameAck {
            return self.fail(ClientError::Unsupported(frame.kind));
        }
        let acknowledgement = match decode_canonical_first_compact_frame_ack(
            &frame.payload,
            compact_contract::stream_validator::StreamIdentityMode::IdentityV1,
        ) {
            Ok(value) => value,
            Err(error) => return self.fail(ClientError::FirstCompactAckCodec(error)),
        };
        if let Err(error) = coordinator.accept_new_header_ack(sink, acknowledgement) {
            return self.fail(ClientError::RolloverState(error));
        }
        self.first_frame_acked = true;
        self.reported_state = ProductionState::Producing;
        self.phase = ClientPhase::Producing;
        Ok(acknowledgement)
    }

    pub fn serve_next(&mut self, deadline: MonotonicDeadline) -> Result<ClientEvent, ClientError> {
        if !matches!(self.phase, ClientPhase::Attached | ClientPhase::Producing) {
            return Err(ClientError::NotUsable);
        }
        let frame = match self.recv_frame(deadline)? {
            Some(frame) => frame,
            None => {
                self.phase = ClientPhase::Stopped;
                return Ok(ClientEvent::GracefulStop);
            }
        };
        match frame.kind {
            MessageKind::HeartbeatChallenge => {
                let challenge = match decode_heartbeat_challenge(&frame.payload) {
                    Ok(challenge) => challenge,
                    Err(err) => return self.fail(ClientError::Payload(err)),
                };
                let proof = encode_heartbeat_proof(&HeartbeatProofV1 {
                    challenge: challenge.challenge,
                    last_h4cr_sequence: self.last_h4cr_sequence,
                    state: self.reported_state,
                });
                self.send_frame(MessageKind::HeartbeatProof, &proof, deadline)?;
                Ok(ClientEvent::HeartbeatServed {
                    host_challenge_counter: challenge.host_challenge_counter,
                    reported_state: self.reported_state.code(),
                })
            }
            MessageKind::FirstCompactFrameAck => {
                let ack = match decode_canonical_first_compact_frame_ack(
                    &frame.payload,
                    compact_contract::stream_validator::StreamIdentityMode::IdentityV1,
                ) {
                    Ok(ack) => ack,
                    Err(err) => return self.fail(ClientError::FirstCompactAckCodec(err)),
                };
                self.first_frame_acked = true;
                self.phase = ClientPhase::Producing;
                Ok(ClientEvent::FirstFrameAcked(ack))
            }
            MessageKind::IdentityFailureAck => {
                let ack = match decode_identity_failure_ack(&frame.payload) {
                    Ok(ack) => ack,
                    Err(err) => return self.fail(ClientError::IdentityCodec(err)),
                };
                Ok(ClientEvent::IdentityFailureAcked(ack))
            }
            other => self.fail(ClientError::Unsupported(other)),
        }
    }

    pub fn run(&mut self, stop_flag: &AtomicBool) -> Result<ClientOutcome, ClientError> {
        loop {
            if stop_flag.load(Ordering::Acquire) {
                self.phase = ClientPhase::Stopped;
                return Ok(ClientOutcome::StoppedByRequest);
            }
            let deadline = MonotonicDeadline::from_now(&self.clock, self.policy.heartbeat_liveness);
            if let ClientEvent::GracefulStop = self.serve_next(deadline)? {
                return Ok(ClientOutcome::StoppedByPeer);
            }
        }
    }

    #[cfg(test)]
    fn backend(&self) -> &T {
        &self.channel.io().inner
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DuplexConnectError {
    DeadlineExpired,
    Fatal(&'static str),
}

pub enum CreateFileOutcome<H> {
    Connected(H),
    Busy,
    NotFound,
    Fatal(&'static str),
}

pub enum WaitOutcome {
    Signaled,
    TimedOut,
    Fatal(&'static str),
}

pub trait PipeConnector {
    type Handle;
    fn create_file(&mut self) -> CreateFileOutcome<Self::Handle>;
    fn wait_named_pipe(&mut self, timeout_ms: u32) -> WaitOutcome;
    fn discard(&mut self, handle: Self::Handle);
}

pub fn open_client_control_pipe<P: PipeConnector, C: MonotonicClock>(
    _permit: &DuplexClientProductPermit,
    connector: &mut P,
    clock: &C,
    deadline: MonotonicDeadline,
) -> Result<P::Handle, DuplexConnectError> {
    open_control_pipe_inner(connector, clock, deadline)
}

pub fn open_early_render_control_pipe<P: PipeConnector, C: MonotonicClock>(
    _permit: &EarlyRenderDuplexProductPermit,
    connector: &mut P,
    clock: &C,
    deadline: MonotonicDeadline,
) -> Result<P::Handle, DuplexConnectError> {
    open_control_pipe_inner(connector, clock, deadline)
}

fn open_control_pipe_inner<P: PipeConnector, C: MonotonicClock>(
    connector: &mut P,
    clock: &C,
    deadline: MonotonicDeadline,
) -> Result<P::Handle, DuplexConnectError> {
    loop {
        if deadline.is_expired(clock.now()) {
            return Err(DuplexConnectError::DeadlineExpired);
        }
        match connector.create_file() {
            CreateFileOutcome::Connected(handle) => {
                if deadline.is_expired(clock.now()) {
                    connector.discard(handle);
                    return Err(DuplexConnectError::DeadlineExpired);
                }
                return Ok(handle);
            }
            CreateFileOutcome::Fatal(reason) => return Err(DuplexConnectError::Fatal(reason)),
            CreateFileOutcome::Busy | CreateFileOutcome::NotFound => {}
        }
        let Some(timeout_ms) = deadline.wait_named_pipe_timeout_ms(clock.now()) else {
            return Err(DuplexConnectError::DeadlineExpired);
        };
        match connector.wait_named_pipe(timeout_ms) {
            WaitOutcome::Signaled | WaitOutcome::TimedOut => {}
            WaitOutcome::Fatal(reason) => return Err(DuplexConnectError::Fatal(reason)),
        }
    }
}

pub fn control_pipe_name(pid: u32, token: &str) -> String {
    format!(r"\\.\pipe\battle-plan_control_{pid}_{token}")
}

pub enum BoundedWorkerOutcome<R> {
    Completed(R),
    /// moved to process-lifetime terminal quarantine and never joined here.
    Quarantined,
}

/// Process-lifetime terminal quarantine for setup/connector workers that did not
fn worker_quarantine() -> &'static Mutex<Vec<JoinHandle<()>>> {
    static QUARANTINE: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();
    QUARANTINE.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn quarantined_worker_count() -> usize {
    worker_quarantine().lock().map(|q| q.len()).unwrap_or(0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReapOutcome {
    Completed,
    LeakAbandoned,
}

fn bounded_reap<F: FnMut() -> bool>(attempts: u32, mut poll_signaled: F) -> ReapOutcome {
    for _ in 0..attempts {
        if poll_signaled() {
            return ReapOutcome::Completed;
        }
    }
    ReapOutcome::LeakAbandoned
}

const WORKER_POLL: Duration = Duration::from_millis(1);

pub fn run_bounded_worker<R, C, F>(
    _permit: &DuplexClientProductPermit,
    clock: &C,
    join_deadline: MonotonicDeadline,
    work: F,
) -> BoundedWorkerOutcome<R>
where
    R: Send + 'static,
    C: MonotonicClock,
    F: FnOnce() -> R + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let _ = tx.send(work());
    });

    loop {
        match rx.try_recv() {
            Ok(result) => {
                let _ = handle.join();
                return BoundedWorkerOutcome::Completed(result);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let _ = handle.join();
                return BoundedWorkerOutcome::Quarantined;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        if join_deadline.is_expired(clock.now()) {
            match rx.try_recv() {
                Ok(result) => {
                    let _ = handle.join();
                    return BoundedWorkerOutcome::Completed(result);
                }
                Err(_) => {
                    if let Ok(mut quarantine) = worker_quarantine().lock() {
                        quarantine.push(handle);
                    }
                    return BoundedWorkerOutcome::Quarantined;
                }
            }
        }
        thread::sleep(WORKER_POLL);
    }
}

#[cfg(windows)]
mod win32 {

    use super::{
        bounded_reap, CreateFileOutcome, DeadlineControlIo, DuplexClientProductPermit,
        DuplexIoError, EarlyRenderDuplexProductPermit, MonotonicClock, MonotonicDeadline,
        PipeConnector, ReapOutcome, WaitOutcome,
    };
    use std::os::raw::c_void;
    use std::ptr;

    const CANCEL_REAP_ATTEMPTS: u32 = 100;
    const CANCEL_REAP_SLICE_MS: Dword = 10;

    type Handle = *mut c_void;
    type Dword = u32;
    type Bool = i32;

    const GENERIC_READ: Dword = 0x8000_0000;
    const GENERIC_WRITE: Dword = 0x4000_0000;
    const OPEN_EXISTING: Dword = 3;
    const FILE_ATTRIBUTE_NORMAL: Dword = 0x80;
    const FILE_FLAG_OVERLAPPED: Dword = 0x4000_0000;
    const INVALID_HANDLE_VALUE: isize = -1;

    const ERROR_FILE_NOT_FOUND: Dword = 2;
    const ERROR_BROKEN_PIPE: Dword = 109;
    const ERROR_NO_DATA: Dword = 232;
    const ERROR_OPERATION_ABORTED: Dword = 995;
    const ERROR_PIPE_BUSY: Dword = 231;
    const ERROR_IO_PENDING: Dword = 997;
    const ERROR_IO_INCOMPLETE: Dword = 996;

    const WAIT_OBJECT_0: Dword = 0;
    const WAIT_TIMEOUT: Dword = 0x102;

    #[repr(C)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: Dword,
        offset_high: Dword,
        h_event: Handle,
    }

    extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: Dword,
            share: Dword,
            security: *mut c_void,
            creation: Dword,
            flags: Dword,
            template: Handle,
        ) -> Handle;
        fn WaitNamedPipeW(name: *const u16, timeout_ms: Dword) -> Bool;
        fn CloseHandle(handle: Handle) -> Bool;
        fn GetLastError() -> Dword;
        fn CreateEventW(
            security: *mut c_void,
            manual_reset: Bool,
            initial_state: Bool,
            name: *const u16,
        ) -> Handle;
        fn ReadFile(
            handle: Handle,
            buffer: *mut c_void,
            len: Dword,
            read: *mut Dword,
            overlapped: *mut Overlapped,
        ) -> Bool;
        fn WriteFile(
            handle: Handle,
            buffer: *const c_void,
            len: Dword,
            written: *mut Dword,
            overlapped: *mut c_void,
        ) -> Bool;
        fn GetOverlappedResult(
            handle: Handle,
            overlapped: *mut Overlapped,
            transferred: *mut Dword,
            wait: Bool,
        ) -> Bool;
        fn CancelIoEx(handle: Handle, overlapped: *mut Overlapped) -> Bool;
        fn PeekNamedPipe(
            handle: Handle,
            buffer: *mut c_void,
            buffer_size: Dword,
            bytes_read: *mut Dword,
            total_bytes_available: *mut Dword,
            bytes_left_this_message: *mut Dword,
        ) -> Bool;
        #[cfg(test)]
        fn SetEvent(event: Handle) -> Bool;
        fn WaitForSingleObject(handle: Handle, timeout_ms: Dword) -> Dword;
    }

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// across a thread boundary. Ownership is single: whoever holds it closes it.
    pub struct SendHandle(Handle);
    // SAFETY: the handle is only ever used by one thread at a time — the
    // connector worker owns it during the open, then transfers ownership back to
    unsafe impl Send for SendHandle {}

    impl Drop for SendHandle {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 as isize != INVALID_HANDLE_VALUE {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
    }

    pub struct Win32PipeConnector {
        name: Vec<u16>,
    }

    impl Win32PipeConnector {
        pub fn new(_permit: &DuplexClientProductPermit, pipe_name: &str) -> Self {
            Self {
                name: to_wide(pipe_name),
            }
        }

        pub fn new_early_render(_permit: &EarlyRenderDuplexProductPermit, pipe_name: &str) -> Self {
            Self {
                name: to_wide(pipe_name),
            }
        }
    }

    impl PipeConnector for Win32PipeConnector {
        type Handle = SendHandle;

        fn create_file(&mut self) -> CreateFileOutcome<SendHandle> {
            let handle = unsafe {
                CreateFileW(
                    self.name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    ptr::null_mut(),
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                    ptr::null_mut(),
                )
            };
            if handle as isize != INVALID_HANDLE_VALUE {
                return CreateFileOutcome::Connected(SendHandle(handle));
            }
            match unsafe { GetLastError() } {
                ERROR_FILE_NOT_FOUND => CreateFileOutcome::NotFound,
                ERROR_PIPE_BUSY => CreateFileOutcome::Busy,
                _ => CreateFileOutcome::Fatal("CreateFileW failed"),
            }
        }

        fn wait_named_pipe(&mut self, timeout_ms: u32) -> WaitOutcome {
            if unsafe { WaitNamedPipeW(self.name.as_ptr(), timeout_ms) } != 0 {
                WaitOutcome::Signaled
            } else {
                WaitOutcome::TimedOut
            }
        }

        fn discard(&mut self, handle: SendHandle) {
            drop(handle);
        }
    }

    pub struct Win32OverlappedControlIo {
        handle: SendHandle,
    }

    impl Win32OverlappedControlIo {
        pub fn new(_permit: &DuplexClientProductPermit, handle: SendHandle) -> Self {
            Self { handle }
        }

        pub fn new_early_render(
            _permit: &EarlyRenderDuplexProductPermit,
            handle: SendHandle,
        ) -> Self {
            Self { handle }
        }

        /// Ownership/UB invariant: the `OVERLAPPED` and its data buffer live in
        fn wait_and_reap<C: MonotonicClock>(
            &self,
            mut pending: Box<PendingIo>,
            clock: &C,
            deadline: MonotonicDeadline,
        ) -> Result<(u32, Box<PendingIo>), DuplexIoError> {
            loop {
                let Some(ms) = deadline.wait_named_pipe_timeout_ms(clock.now()) else {
                    break;
                };
                match unsafe { WaitForSingleObject(pending.event.0, ms) } {
                    WAIT_OBJECT_0 => {
                        let mut transferred: Dword = 0;
                        let ok = unsafe {
                            GetOverlappedResult(
                                self.handle.0,
                                &mut pending.overlapped,
                                &mut transferred,
                                0,
                            )
                        };
                        return if ok != 0 {
                            Ok((transferred, pending))
                        } else {
                            match unsafe { GetLastError() } {
                                ERROR_BROKEN_PIPE | ERROR_NO_DATA | ERROR_OPERATION_ABORTED => {
                                    Err(DuplexIoError::PeerClosed)
                                }
                                _ => Err(DuplexIoError::Disconnected),
                            }
                        };
                    }
                    WAIT_TIMEOUT if !deadline.is_expired(clock.now()) => continue,
                    _ => break,
                }
            }

            unsafe {
                let _ = CancelIoEx(self.handle.0, &mut pending.overlapped);
            }
            let event = pending.event.0;
            match bounded_reap(CANCEL_REAP_ATTEMPTS, || unsafe {
                WaitForSingleObject(event, CANCEL_REAP_SLICE_MS) == WAIT_OBJECT_0
            }) {
                ReapOutcome::Completed => {
                    let mut transferred: Dword = 0;
                    let _ = unsafe {
                        GetOverlappedResult(
                            self.handle.0,
                            &mut pending.overlapped,
                            &mut transferred,
                            0,
                        )
                    };
                    Err(DuplexIoError::TimedOut)
                }
                ReapOutcome::LeakAbandoned => {
                    Box::leak(pending);
                    Err(DuplexIoError::TimedOut)
                }
            }
        }
    }

    pub struct CaptureReadyControlSession {
        io: Win32OverlappedControlIo,
    }

    impl CaptureReadyControlSession {
        pub fn stop_requested(&self) -> Result<bool, DuplexIoError> {
            let mut available = 0u32;
            let ok = unsafe {
                PeekNamedPipe(
                    self.io.handle.0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    &mut available,
                    ptr::null_mut(),
                )
            };
            if ok != 0 {
                return if available == 0 {
                    Ok(false)
                } else {
                    Err(DuplexIoError::Disconnected)
                };
            }
            match unsafe { GetLastError() } {
                ERROR_BROKEN_PIPE | ERROR_NO_DATA => Ok(true),
                _ => Err(DuplexIoError::Disconnected),
            }
        }
    }

    pub struct CaptureReadyExchange {
        pub ack: Vec<u8>,
        pub session: CaptureReadyControlSession,
    }

    pub fn exchange_capture_ready_pipe(
        pipe_name: &str,
        ready: &[u8],
        budget: std::time::Duration,
    ) -> Result<CaptureReadyExchange, DuplexIoError> {
        const MAX_DOCUMENT: usize = 16 * 1024;
        if ready.is_empty()
            || ready.len() > MAX_DOCUMENT
            || !pipe_name.starts_with(r"\\.\pipe\battle-plan_control_")
            || pipe_name.contains('\0')
        {
            return Err(DuplexIoError::Disconnected);
        }
        let clock = super::SystemClock::new();
        let deadline = MonotonicDeadline::from_now(&clock, budget);
        let name = to_wide(pipe_name);
        let handle = loop {
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    ptr::null_mut(),
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                    ptr::null_mut(),
                )
            };
            if handle as isize != INVALID_HANDLE_VALUE {
                break SendHandle(handle);
            }
            match unsafe { GetLastError() } {
                ERROR_FILE_NOT_FOUND | ERROR_PIPE_BUSY => {
                    let timeout_ms = deadline
                        .wait_named_pipe_timeout_ms(clock.now())
                        .ok_or(DuplexIoError::TimedOut)?;
                    unsafe {
                        let _ = WaitNamedPipeW(name.as_ptr(), timeout_ms);
                    }
                }
                _ => return Err(DuplexIoError::Disconnected),
            }
        };
        let mut io = Win32OverlappedControlIo { handle };
        let mut frame = Vec::with_capacity(4 + ready.len());
        frame.extend_from_slice(&(ready.len() as u32).to_le_bytes());
        frame.extend_from_slice(ready);
        io.send_all(&frame, deadline)?;
        let mut ack_len = [0u8; 4];
        io.recv_exact(&mut ack_len, deadline)?;
        let ack_len = u32::from_le_bytes(ack_len) as usize;
        if ack_len == 0 || ack_len > MAX_DOCUMENT {
            return Err(DuplexIoError::Disconnected);
        }
        let mut ack = vec![0u8; ack_len];
        io.recv_exact(&mut ack, deadline)?;
        Ok(CaptureReadyExchange {
            ack,
            session: CaptureReadyControlSession { io },
        })
    }

    /// Heap ownership is what makes the pathological-driver leak sound — a leaked
    /// `Box<PendingIo>` keeps both alive for the process lifetime.
    struct PendingIo {
        overlapped: Overlapped,
        event: SendHandle,
        buffer: Vec<u8>,
    }

    impl PendingIo {
        fn new(buffer: Vec<u8>) -> Result<Box<Self>, DuplexIoError> {
            let event = unsafe { CreateEventW(ptr::null_mut(), 1, 0, ptr::null()) };
            if event.is_null() {
                return Err(DuplexIoError::Disconnected);
            }
            let event = SendHandle(event);
            Ok(Box::new(Self {
                overlapped: Overlapped {
                    internal: 0,
                    internal_high: 0,
                    offset: 0,
                    offset_high: 0,
                    h_event: event.0,
                },
                event,
                buffer,
            }))
        }
    }

    impl DeadlineControlIo for Win32OverlappedControlIo {
        fn send_all(
            &mut self,
            bytes: &[u8],
            deadline: MonotonicDeadline,
        ) -> Result<(), DuplexIoError> {
            let clock = super::SystemClock::new();
            let mut sent = 0usize;
            while sent < bytes.len() {
                let chunk_len = (bytes.len() - sent).min(Dword::MAX as usize);
                let mut pending = PendingIo::new(bytes[sent..sent + chunk_len].to_vec())?;
                let mut written: Dword = 0;
                let ok = unsafe {
                    WriteFile(
                        self.handle.0,
                        pending.buffer.as_ptr() as *const c_void,
                        pending.buffer.len() as Dword,
                        &mut written,
                        (&mut pending.overlapped as *mut Overlapped).cast(),
                    )
                };
                let transferred = if ok != 0 {
                    written
                } else if unsafe { GetLastError() } == ERROR_IO_PENDING {
                    let (transferred, _pending) = self.wait_and_reap(pending, &clock, deadline)?;
                    transferred
                } else {
                    return Err(DuplexIoError::Disconnected);
                };
                if transferred == 0 {
                    return Err(DuplexIoError::Disconnected);
                }
                sent += transferred as usize;
            }
            Ok(())
        }

        fn recv_exact(
            &mut self,
            bytes: &mut [u8],
            deadline: MonotonicDeadline,
        ) -> Result<(), DuplexIoError> {
            let clock = super::SystemClock::new();
            let mut received = 0usize;
            while received < bytes.len() {
                let remaining = (bytes.len() - received).min(Dword::MAX as usize);
                let mut pending = PendingIo::new(vec![0u8; remaining])?;
                let mut read: Dword = 0;
                let ok = unsafe {
                    ReadFile(
                        self.handle.0,
                        pending.buffer.as_mut_ptr() as *mut c_void,
                        remaining as Dword,
                        &mut read,
                        &mut pending.overlapped,
                    )
                };
                let (transferred, pending) = if ok != 0 {
                    (read, pending)
                } else {
                    match unsafe { GetLastError() } {
                        ERROR_IO_PENDING => self.wait_and_reap(pending, &clock, deadline)?,
                        _ => {
                            return Err(if received == 0 {
                                DuplexIoError::PeerClosed
                            } else {
                                DuplexIoError::Truncated
                            });
                        }
                    }
                };
                if transferred == 0 {
                    return Err(if received == 0 {
                        DuplexIoError::PeerClosed
                    } else {
                        DuplexIoError::Truncated
                    });
                }
                let count = transferred as usize;
                bytes[received..received + count].copy_from_slice(&pending.buffer[..count]);
                received += count;
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn each_pending_transfer_owns_a_fresh_unsignalled_event() {
            let first = PendingIo::new(vec![0x48; 48]).expect("first pending I/O");
            let second = PendingIo::new(vec![0x72; 72]).expect("second pending I/O");

            assert_ne!(first.event.0, second.event.0);
            assert_eq!(first.overlapped.h_event, first.event.0);
            assert_eq!(second.overlapped.h_event, second.event.0);
            assert_ne!(unsafe { SetEvent(first.event.0) }, 0);
            assert_eq!(
                unsafe { WaitForSingleObject(first.event.0, 0) },
                WAIT_OBJECT_0
            );
            assert_eq!(
                unsafe { WaitForSingleObject(second.event.0, 0) },
                WAIT_TIMEOUT
            );
        }
    }

    const _: Dword = ERROR_IO_INCOMPLETE;
}

#[cfg(windows)]
pub use win32::{
    exchange_capture_ready_pipe, CaptureReadyControlSession, CaptureReadyExchange, SendHandle,
    Win32OverlappedControlIo, Win32PipeConnector,
};

#[cfg(test)]
mod tests;
