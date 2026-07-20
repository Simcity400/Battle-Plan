//! Activation-frozen auto-attach integration for the paired H4CR/H4CT transport.
//!
//! This module composes the listener-before-injection type state, admitted
//! deadline channel, recurring heartbeat, and [`WorkerHandle`] lifecycle. The
//! live spawner selects the combined product branch only when both explicit
//! activation authorities below are true; otherwise it remains byte-for-byte
//! on the legacy child path.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::control_transport::{
    ControlCancellation, ControlCapabilities, ControlDeadline, ControlFrame, ControlPeerIdentity,
    DeadlineControlChannel, DeadlineControlIo, EndpointRole, MessageKind,
};
use crate::duplex_session::identity_control::{
    AcknowledgedFirstCompactRecord, AcknowledgedIdentityFailure, IdentityDurableCandidateSink,
    RecorderIdentityControlDispatcher, ValidatedFirstCompactRecord,
};
use crate::duplex_session::rollover_control::{
    RecorderRolloverCoordinator, RecorderRolloverError, RecorderRolloverStore,
};
use crate::duplex_session::{HeartbeatError, HeartbeatProof, RecorderHeartbeat};
use crate::transport_status::{decode_sink_failure, SinkFailureReport, SinkStatusError};
use crate::windows_control_pipe::{
    prepare_control_listener, ControlPipeBackend, ControlPipeError, ListenerReadyProof,
    PreparedControlListener, RestrictiveSecurityPolicy,
};

use super::identity::ProcessKey;
use super::policy;
use super::proof::{
    decode_collector_hello_capabilities, decode_hex_16, decode_hex_32, encode_hex,
    ProofExpectation, ValidatedCompactRecordProof,
};
use super::sources::{WorkerHandle, WorkerSpec, WorkerStatus};

/// Explicit product kill switch. The reusable foundation is testable, but the
/// live spawner cannot select it until identity-reader activation is also true.
pub const AUTO_ATTACH_DUPLEX_ACTIVATION_ENABLED: bool = false;

/// Opaque product selector.  Offline preparation/tests do not mint it, and the
/// constructor fails while the activation constant remains false.
pub struct AutoAttachDuplexProductPermit {
    _private: (),
}

pub fn auto_attach_duplex_product_permit(
) -> Result<AutoAttachDuplexProductPermit, DuplexWorkerError> {
    if AUTO_ATTACH_DUPLEX_ACTIVATION_ENABLED
        && compact_contract::build_profile::DOMAIN_CAMPAIGN_ID_LIVE_READER_ENABLED
    {
        Ok(AutoAttachDuplexProductPermit { _private: () })
    } else {
        Err(DuplexWorkerError::ActivationDisabled)
    }
}

/// One fail-closed identity-mode selector shared by recorder-main consumers.
/// A duplex-only flag flip cannot silently reinterpret Legacy bytes as CID1.
pub const fn auto_attach_identity_v1_enabled() -> bool {
    AUTO_ATTACH_DUPLEX_ACTIVATION_ENABLED
        && compact_contract::build_profile::DOMAIN_CAMPAIGN_ID_LIVE_READER_ENABLED
}

pub fn required_auto_attach_duplex_capabilities() -> ControlCapabilities {
    // Single source of truth in the shared codec: the collector client's hello
    // advertises `ControlCapabilities::auto_attach_duplex_v1()` and admission
    // here requires exactly the same closed word, so they cannot drift.
    ControlCapabilities::auto_attach_duplex_v1()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DuplexWorkerState {
    Handshaking,
    WaitingForWorld,
    Recording,
    StopRequested,
    Finalizing,
    Complete,
    Failed,
}

#[derive(Debug)]
pub enum DuplexWorkerError {
    AttemptAuthority(String),
    Pipe(ControlPipeError),
    Heartbeat(HeartbeatError),
    Transport(String),
    CollectorHello(String),
    SinkStatus(SinkStatusError),
    SinkFailure(SinkFailureReport),
    IllegalTransition {
        from: DuplexWorkerState,
        action: &'static str,
    },
    StartupDeadlineExpired,
    StartupCancelled,
    FirstFrameDeadlineExpired,
    HeartbeatLivenessExpired,
    GracefulStopExpired,
    GracefulStopInterrupted,
    MissingReplayPath,
    DataPipe(String),
    TargetProbe(String),
    TargetExited,
    RecordingAdmission(String),
    IdentityControl(String),
    RolloverControl(String),
    ActivationDisabled,
}

impl fmt::Display for DuplexWorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AttemptAuthority(error) => {
                write!(f, "duplex attempt authority invalid: {error}")
            }
            Self::Pipe(error) => write!(f, "duplex worker pipe failed: {error}"),
            Self::Heartbeat(error) => write!(f, "duplex worker heartbeat failed: {error}"),
            Self::Transport(error) => write!(f, "duplex worker transport failed: {error}"),
            Self::CollectorHello(error) => write!(f, "duplex collector hello failed: {error}"),
            Self::SinkStatus(error) => write!(f, "duplex sink status invalid: {error}"),
            Self::SinkFailure(report) => write!(f, "duplex sink failure: {report:?}"),
            Self::IllegalTransition { from, action } => {
                write!(f, "duplex worker cannot {action} from {from:?}")
            }
            Self::StartupDeadlineExpired => {
                write!(f, "duplex startup authority deadline expired")
            }
            Self::StartupCancelled => write!(f, "duplex startup authority was cancelled"),
            Self::FirstFrameDeadlineExpired => {
                write!(f, "duplex first-world-frame deadline expired")
            }
            Self::HeartbeatLivenessExpired => write!(f, "duplex heartbeat liveness expired"),
            Self::GracefulStopExpired => write!(f, "duplex graceful-stop deadline expired"),
            Self::GracefulStopInterrupted => {
                write!(f, "duplex heartbeat interrupted by graceful stop")
            }
            Self::MissingReplayPath => write!(f, "duplex completion has no replay path"),
            Self::DataPipe(error) => write!(f, "duplex data pipe failed: {error}"),
            Self::TargetProbe(error) => write!(f, "duplex target probe failed: {error}"),
            Self::TargetExited => write!(f, "duplex target exited during startup"),
            Self::RecordingAdmission(error) => {
                write!(f, "duplex recording admission failed: {error}")
            }
            Self::IdentityControl(error) => write!(f, "duplex identity control failed: {error}"),
            Self::RolloverControl(error) => write!(f, "duplex rollover control failed: {error}"),
            Self::ActivationDisabled => write!(f, "auto-attach duplex activation is disabled"),
        }
    }
}

/// Move-only authority for the `WaitingForWorld -> Recording` transition.
/// Construction consumes both the durable-and-sent CFA1 result and the later
/// first-world proof, so neither independently callable seam can enter
/// `Recording`.  The placement-authoritative CID1 remains retained for the
/// lifetime of the worker session in IdentityV1 mode.
pub struct DuplexRecordingAdmission {
    logical_stream_id: u64,
    mode: compact_contract::stream_validator::StreamIdentityMode,
    validated_cid1: Option<compact_contract::campaign_identity::ValidatedCid1>,
    first_record_sequence: u64,
    first_world_sequence: u64,
    confirmed_hour: i64,
}

/// One recurring heartbeat result plus every asynchronous CIF1 that was
/// durably journalled and acknowledged before the matching proof arrived.
/// Keeping those outcomes together prevents a central runner from silently
/// dropping an identity failure while it waits for liveness proof.
pub struct RoutedHeartbeatCycle {
    proof: HeartbeatProof,
    acknowledged_identity_failures: Vec<AcknowledgedIdentityFailure>,
}

impl RoutedHeartbeatCycle {
    pub fn proof(&self) -> &HeartbeatProof {
        &self.proof
    }

    pub fn acknowledged_identity_failures(&self) -> &[AcknowledgedIdentityFailure] {
        &self.acknowledged_identity_failures
    }

    pub fn into_parts(self) -> (HeartbeatProof, Vec<AcknowledgedIdentityFailure>) {
        (self.proof, self.acknowledged_identity_failures)
    }
}

impl DuplexRecordingAdmission {
    pub fn from_acknowledged_first_record(
        acknowledged: AcknowledgedFirstCompactRecord,
        first_world: ValidatedCompactRecordProof,
    ) -> Result<Self, DuplexWorkerError> {
        let first_record = acknowledged.first_record();
        let acknowledgement = first_record.acknowledgement();
        if acknowledged.durable_receipt().acknowledgement() != acknowledgement {
            return Err(DuplexWorkerError::RecordingAdmission(
                "durable receipt does not match the acknowledged first record".to_string(),
            ));
        }
        if first_world.sequence() <= acknowledgement.h4cr_sequence {
            return Err(DuplexWorkerError::RecordingAdmission(format!(
                "first-world sequence {} does not follow acknowledged first sequence {}",
                first_world.sequence(),
                acknowledgement.h4cr_sequence
            )));
        }
        let first_hour =
            compact_contract::calendar::hour_index_from_raw(acknowledgement.game_hour_raw)
                .expect("validated CFA1 retains a calendar-valid hour");
        if first_world.confirmed_hour() < first_hour {
            return Err(DuplexWorkerError::RecordingAdmission(format!(
                "first-world hour {} regresses acknowledged first-record hour {}",
                first_world.confirmed_hour(),
                first_hour
            )));
        }
        let validated_cid1 = first_record.validated_cid1().cloned();
        match first_record.mode() {
            compact_contract::stream_validator::StreamIdentityMode::IdentityV1 => {
                let identity = validated_cid1.as_ref().ok_or_else(|| {
                    DuplexWorkerError::RecordingAdmission(
                        "IdentityV1 admission lost placement-authoritative CID1".to_string(),
                    )
                })?;
                if identity.payload_sha256() != acknowledgement.payload_sha256 {
                    return Err(DuplexWorkerError::RecordingAdmission(
                        "IdentityV1 CID1 hash does not match CFA1 payload hash".to_string(),
                    ));
                }
            }
            compact_contract::stream_validator::StreamIdentityMode::Legacy => {
                if validated_cid1.is_some() {
                    return Err(DuplexWorkerError::RecordingAdmission(
                        "legacy admission unexpectedly retained CID1".to_string(),
                    ));
                }
            }
        }
        Ok(Self {
            logical_stream_id: acknowledgement.logical_stream_id,
            mode: first_record.mode(),
            validated_cid1,
            first_record_sequence: acknowledgement.h4cr_sequence,
            first_world_sequence: first_world.sequence(),
            confirmed_hour: first_world.confirmed_hour(),
        })
    }

    pub fn logical_stream_id(&self) -> u64 {
        self.logical_stream_id
    }

    pub fn mode(&self) -> compact_contract::stream_validator::StreamIdentityMode {
        self.mode
    }

    pub fn validated_cid1(&self) -> Option<&compact_contract::campaign_identity::ValidatedCid1> {
        self.validated_cid1.as_ref()
    }

    pub fn first_record_sequence(&self) -> u64 {
        self.first_record_sequence
    }

    pub fn first_world_sequence(&self) -> u64 {
        self.first_world_sequence
    }

    pub fn confirmed_hour(&self) -> i64 {
        self.confirmed_hour
    }

    #[cfg(test)]
    fn for_test(game_hour_raw: i32) -> Self {
        Self {
            logical_stream_id: 0,
            mode: compact_contract::stream_validator::StreamIdentityMode::Legacy,
            validated_cid1: None,
            first_record_sequence: 0,
            first_world_sequence: 1,
            confirmed_hour: i64::from(game_hour_raw),
        }
    }
}

impl std::error::Error for DuplexWorkerError {}

impl From<ControlPipeError> for DuplexWorkerError {
    fn from(error: ControlPipeError) -> Self {
        Self::Pipe(error)
    }
}

impl From<HeartbeatError> for DuplexWorkerError {
    fn from(error: HeartbeatError) -> Self {
        Self::Heartbeat(error)
    }
}

/// Single derivation of all H4CT authority facts already minted by discovery
/// and staging. Future live wiring must not mint a second token, normalize an
/// alternate pipe name, or rebuild the collector proof expectation piecemeal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplexAttemptAuthority {
    attempt_id: String,
    process_key: ProcessKey,
    data_pipe_name: String,
    control_pipe_name: String,
    session_id: [u8; 16],
    expected_peer: ControlPeerIdentity,
    collector_hello: ProofExpectation,
}

impl DuplexAttemptAuthority {
    pub fn from_worker_spec(spec: &WorkerSpec) -> Result<Self, DuplexWorkerError> {
        let session_id = decode_hex_16(&spec.payload.pipe_token)
            .map_err(|error| authority_error(format!("invalid pipe token: {error}")))?;
        if session_id == [0; 16] {
            return Err(authority_error("pipe token must not be all zero"));
        }
        if encode_hex(&session_id) != spec.payload.pipe_token {
            return Err(authority_error(
                "pipe token must use canonical lowercase hexadecimal",
            ));
        }
        if spec.key.pid == 0 || spec.key.creation_filetime.0 == 0 {
            return Err(authority_error("process identity must be nonzero"));
        }
        if spec.attempt_id.is_empty() {
            return Err(authority_error("attempt id must not be empty"));
        }
        if spec.payload.dll_sha256 != spec.payload.source_sha256 {
            return Err(authority_error(
                "staged DLL hash does not match source DLL hash",
            ));
        }
        let dll_sha256 = decode_hex_32(&spec.payload.dll_sha256)
            .map_err(|error| authority_error(format!("invalid staged DLL hash: {error}")))?;
        if encode_hex(&dll_sha256) != spec.payload.dll_sha256 {
            return Err(authority_error(
                "staged DLL hash must use canonical lowercase hexadecimal",
            ));
        }

        Ok(Self {
            attempt_id: spec.attempt_id.clone(),
            process_key: spec.key,
            data_pipe_name: format!(
                r"\\.\pipe\battle-plan_collector_{}_{}",
                spec.key.pid, spec.payload.pipe_token
            ),
            control_pipe_name: format!(
                r"\\.\pipe\battle-plan_control_{}_{}",
                spec.key.pid, spec.payload.pipe_token
            ),
            session_id,
            expected_peer: ControlPeerIdentity {
                pid: spec.key.pid,
                creation_filetime: spec.key.creation_filetime.0,
            },
            collector_hello: ProofExpectation {
                pid: spec.key.pid,
                creation_filetime: spec.key.creation_filetime.0,
                h4cr_version: compact_contract::COMPACT_RECORD_VERSION,
                dll_sha256,
                challenge: spec.control_challenge,
            },
        })
    }

    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }

    pub const fn process_key(&self) -> ProcessKey {
        self.process_key
    }

    pub fn data_pipe_name(&self) -> &str {
        &self.data_pipe_name
    }

    pub fn control_pipe_name(&self) -> &str {
        &self.control_pipe_name
    }

    pub const fn session_id(&self) -> [u8; 16] {
        self.session_id
    }

    pub const fn expected_peer(&self) -> ControlPeerIdentity {
        self.expected_peer
    }

    pub fn collector_hello_expectation(&self) -> &ProofExpectation {
        &self.collector_hello
    }
}

fn authority_error(error: impl Into<String>) -> DuplexWorkerError {
    DuplexWorkerError::AttemptAuthority(error.into())
}

/// Dormant H4CR listener seam.  A later integration owner supplies the real
/// overlapped inbound-pipe adapter; tests use a deterministic in-memory fake.
/// All three operations consume the same absolute startup deadline and shared
/// cancellation token as H4CT.
pub trait DataPipeListenerBackend {
    type Listener;
    type Io;

    fn create_inbound_listener(
        &mut self,
        pipe_name: &str,
        security: RestrictiveSecurityPolicy,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<Self::Listener, String>;

    fn accept(
        &mut self,
        listener: Self::Listener,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<Self::Io, String>;

    fn peer_identity(
        &mut self,
        io: &Self::Io,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<ControlPeerIdentity, String>;
}

/// Read-only target-liveness seam retained inside combined listener type
/// state.  It observes only the already-authorized PID/creation-FILETIME pair;
/// it cannot discover or substitute another process.
pub trait ExpectedTargetMonitor {
    fn expected_target_alive(&mut self, expected: ProcessKey) -> Result<bool, String>;
}

fn ensure_target_live<T: ExpectedTargetMonitor>(
    monitor: &mut T,
    expected: ProcessKey,
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
) -> Result<(), DuplexWorkerError> {
    ensure_startup_live(deadline, cancellation)?;
    match monitor.expected_target_alive(expected) {
        Ok(true) => ensure_startup_live(deadline, cancellation),
        Ok(false) => {
            cancellation.cancel();
            Err(DuplexWorkerError::TargetExited)
        }
        Err(error) => {
            cancellation.cancel();
            Err(DuplexWorkerError::TargetProbe(error))
        }
    }
}

fn observe_target_live<T: ExpectedTargetMonitor>(
    monitor: &mut T,
    expected: ProcessKey,
    cancellation: &ControlCancellation,
) -> Result<(), DuplexWorkerError> {
    if cancellation.is_cancelled() {
        return Err(DuplexWorkerError::StartupCancelled);
    }
    match monitor.expected_target_alive(expected) {
        Ok(true) => Ok(()),
        Ok(false) => {
            cancellation.cancel();
            Err(DuplexWorkerError::TargetExited)
        }
        Err(error) => {
            cancellation.cancel();
            Err(DuplexWorkerError::TargetProbe(error))
        }
    }
}

/// Opaque proof that both per-attempt listeners were created from one retained
/// authority before the sole injection closure was invoked.
pub struct CombinedListenersReadyProof {
    _private: (),
    startup_deadline: ControlDeadline,
    cancellation: ControlCancellation,
}

impl CombinedListenersReadyProof {
    pub fn startup_deadline(&self) -> ControlDeadline {
        self.startup_deadline
    }

    pub fn cancellation(&self) -> &ControlCancellation {
        &self.cancellation
    }
}

/// Move-only, dormant combined H4CR+H4CT listener authority.  Data and control
/// names, expected peer, session id, startup deadline, cancellation, and target
/// monitor cannot be split into caller-owned tuples.
pub struct PreparedAutoAttachCombined<D, B, T>
where
    D: DataPipeListenerBackend,
    B: ControlPipeBackend,
    T: ExpectedTargetMonitor,
{
    authority: DuplexAttemptAuthority,
    data_backend: D,
    data_listener: D::Listener,
    control_prepared: PreparedControlListener<B>,
    target_monitor: T,
    startup_deadline: ControlDeadline,
    cancellation: ControlCancellation,
    ready: CombinedListenersReadyProof,
}

pub fn prepare_auto_attach_combined_for_worker_spec_offline<D, B, T>(
    mut data_backend: D,
    control_backend: B,
    target_monitor: T,
    spec: &WorkerSpec,
    security: RestrictiveSecurityPolicy,
    startup_deadline: ControlDeadline,
    cancellation: ControlCancellation,
) -> Result<PreparedAutoAttachCombined<D, B, T>, DuplexWorkerError>
where
    D: DataPipeListenerBackend,
    B: ControlPipeBackend,
    T: ExpectedTargetMonitor,
{
    let authority = DuplexAttemptAuthority::from_worker_spec(spec)?;
    let mut target_monitor = target_monitor;
    ensure_target_live(
        &mut target_monitor,
        authority.process_key(),
        startup_deadline,
        &cancellation,
    )?;
    let data_listener = data_backend
        .create_inbound_listener(
            authority.data_pipe_name(),
            security.clone(),
            startup_deadline,
            &cancellation,
        )
        .map_err(|error| {
            cancellation.cancel();
            DuplexWorkerError::DataPipe(error)
        })?;
    ensure_target_live(
        &mut target_monitor,
        authority.process_key(),
        startup_deadline,
        &cancellation,
    )?;
    let control_prepared = match prepare_control_listener(
        control_backend,
        authority.control_pipe_name(),
        security,
        startup_deadline,
        &cancellation,
    ) {
        Ok(prepared) => prepared,
        Err(error) => {
            cancellation.cancel();
            return Err(error.into());
        }
    };
    ensure_target_live(
        &mut target_monitor,
        authority.process_key(),
        startup_deadline,
        &cancellation,
    )?;
    Ok(PreparedAutoAttachCombined {
        authority,
        data_backend,
        data_listener,
        control_prepared,
        target_monitor,
        startup_deadline,
        cancellation: cancellation.clone(),
        ready: CombinedListenersReadyProof {
            _private: (),
            startup_deadline,
            cancellation,
        },
    })
}

impl<D, B, T> PreparedAutoAttachCombined<D, B, T>
where
    D: DataPipeListenerBackend,
    B: ControlPipeBackend,
    T: ExpectedTargetMonitor,
{
    pub fn authority(&self) -> &DuplexAttemptAuthority {
        &self.authority
    }

    pub fn listeners_ready_proof(&self) -> &CombinedListenersReadyProof {
        &self.ready
    }

    pub fn commit_single_injection<F>(
        mut self,
        inject_once: F,
    ) -> Result<CombinedInjectionCommitted<D, B, T>, DuplexWorkerError>
    where
        F: FnOnce(
            &CombinedListenersReadyProof,
            &DuplexAttemptAuthority,
        ) -> Result<(), DuplexWorkerError>,
    {
        ensure_target_live(
            &mut self.target_monitor,
            self.authority.process_key(),
            self.startup_deadline,
            &self.cancellation,
        )?;
        if let Err(error) = inject_once(&self.ready, &self.authority) {
            self.cancellation.cancel();
            return Err(error);
        }
        ensure_target_live(
            &mut self.target_monitor,
            self.authority.process_key(),
            self.startup_deadline,
            &self.cancellation,
        )?;
        Ok(CombinedInjectionCommitted {
            authority: self.authority,
            data_backend: self.data_backend,
            data_listener: self.data_listener,
            control_prepared: self.control_prepared,
            target_monitor: self.target_monitor,
            startup_deadline: self.startup_deadline,
            cancellation: self.cancellation,
        })
    }
}

pub struct CombinedInjectionCommitted<D, B, T>
where
    D: DataPipeListenerBackend,
    B: ControlPipeBackend,
    T: ExpectedTargetMonitor,
{
    authority: DuplexAttemptAuthority,
    data_backend: D,
    data_listener: D::Listener,
    control_prepared: PreparedControlListener<B>,
    target_monitor: T,
    startup_deadline: ControlDeadline,
    cancellation: ControlCancellation,
}

/// Accepted data/control endpoints remain bound to one immutable attempt.
/// There is deliberately no `into_parts`: later composition borrows H4CR I/O
/// through this owner while control state retains the same cancellation token.
pub struct CombinedWorkerSession<D, C, T>
where
    C: DeadlineControlIo,
    T: ExpectedTargetMonitor,
{
    authority: DuplexAttemptAuthority,
    data_io: D,
    control: DuplexWorkerSession<C>,
    target_monitor: T,
    startup_deadline: ControlDeadline,
    cancellation: ControlCancellation,
}

impl<D, C, T> CombinedWorkerSession<D, C, T>
where
    C: DeadlineControlIo,
    T: ExpectedTargetMonitor,
{
    pub fn authority(&self) -> &DuplexAttemptAuthority {
        &self.authority
    }

    pub fn data_io_mut(&mut self) -> &mut D {
        &mut self.data_io
    }

    pub fn control_mut(&mut self) -> &mut DuplexWorkerSession<C> {
        &mut self.control
    }

    pub fn startup_deadline(&self) -> ControlDeadline {
        self.startup_deadline
    }

    pub fn cancellation(&self) -> &ControlCancellation {
        &self.cancellation
    }

    /// Poll the exact retained PID/creation-time authority.  Exit or probe
    /// uncertainty cancels the one token shared by H4CR and H4CT and leaves the
    /// attempt terminal; it never selects another process or reinjects.
    pub fn check_expected_target_live(&mut self) -> Result<(), DuplexWorkerError> {
        observe_target_live(
            &mut self.target_monitor,
            self.authority.process_key(),
            &self.cancellation,
        )
    }
}

impl<D, B, T> CombinedInjectionCommitted<D, B, T>
where
    D: DataPipeListenerBackend,
    B: ControlPipeBackend,
    T: ExpectedTargetMonitor,
{
    pub(crate) fn accept_both_after_single_injection_offline(
        mut self,
    ) -> Result<CombinedWorkerSession<D::Io, B::Io, T>, DuplexWorkerError> {
        ensure_target_live(
            &mut self.target_monitor,
            self.authority.process_key(),
            self.startup_deadline,
            &self.cancellation,
        )?;
        let data_io = self
            .data_backend
            .accept(
                self.data_listener,
                self.startup_deadline,
                &self.cancellation,
            )
            .map_err(|error| {
                self.cancellation.cancel();
                DuplexWorkerError::DataPipe(error)
            })?;
        let data_peer = self
            .data_backend
            .peer_identity(&data_io, self.startup_deadline, &self.cancellation)
            .map_err(|error| {
                self.cancellation.cancel();
                DuplexWorkerError::DataPipe(error)
            })?;
        if data_peer != self.authority.expected_peer() {
            self.cancellation.cancel();
            return Err(DuplexWorkerError::DataPipe(format!(
                "H4CR peer {:?} did not match retained peer {:?}",
                data_peer,
                self.authority.expected_peer()
            )));
        }
        ensure_target_live(
            &mut self.target_monitor,
            self.authority.process_key(),
            self.startup_deadline,
            &self.cancellation,
        )?;
        let none = ControlCapabilities::from_bits(0).expect("empty capability set");
        let channel = self.control_prepared.accept_and_admit(
            self.authority.session_id(),
            self.authority.expected_peer(),
            none,
            none,
            EndpointRole::Recorder,
            self.startup_deadline,
            &self.cancellation,
        )?;
        ensure_target_live(
            &mut self.target_monitor,
            self.authority.process_key(),
            self.startup_deadline,
            &self.cancellation,
        )?;
        let control = DuplexWorkerSession::new(
            channel,
            self.authority.collector_hello_expectation().clone(),
            self.startup_deadline,
            self.cancellation.clone(),
        );
        Ok(CombinedWorkerSession {
            authority: self.authority,
            data_io,
            control,
            target_monitor: self.target_monitor,
            startup_deadline: self.startup_deadline,
            cancellation: self.cancellation,
        })
    }

    pub fn accept_both_for_product(
        self,
        _permit: AutoAttachDuplexProductPermit,
    ) -> Result<CombinedWorkerSession<D::Io, B::Io, T>, DuplexWorkerError> {
        self.accept_both_after_single_injection_offline()
    }
}

#[derive(Clone, Debug)]
pub struct GracefulStopToken {
    requested: Arc<AtomicBool>,
}

impl Default for GracefulStopToken {
    fn default() -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl GracefulStopToken {
    fn with_transport_cancellation(_transport_cancellation: ControlCancellation) -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn request(&self) {
        // Cooperative stop is deliberately distinct from transport failure.
        // Cancelling here left the H4CT handle open but unusable, so the
        // collector could not observe the protocol's clean peer-close receipt.
        // The owning finalizer transitions state, closes/drops H4CT, drains
        // H4CR, then uses transport cancellation only for timeout/failure.
        self.requested.store(true, Ordering::Release);
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}

/// H4CT listener exists and is secured. Future injection code must require the
/// ready proof before invoking the one existing one-shot injector.
pub struct PreparedAutoAttachDuplex<B: ControlPipeBackend> {
    authority: DuplexAttemptAuthority,
    prepared: PreparedControlListener<B>,
    startup_deadline: ControlDeadline,
    cancellation: ControlCancellation,
}

/// Discovery/staging-aware preparation seam for future activation. The
/// WorkerSpec-derived authority, unchanged absolute startup deadline, and one
/// shared cancellation token stay inside the move-only type state; callers
/// cannot cross-wire replacement session, peer, timeout, or cancellation facts
/// after listener creation.
pub fn prepare_auto_attach_duplex_for_worker_spec_offline<B: ControlPipeBackend>(
    backend: B,
    spec: &WorkerSpec,
    security: RestrictiveSecurityPolicy,
    startup_deadline: ControlDeadline,
    cancellation: ControlCancellation,
) -> Result<PreparedAutoAttachDuplex<B>, DuplexWorkerError> {
    let authority = DuplexAttemptAuthority::from_worker_spec(spec)?;
    ensure_startup_live(startup_deadline, &cancellation)?;
    let prepared = prepare_control_listener(
        backend,
        authority.control_pipe_name(),
        security,
        startup_deadline,
        &cancellation,
    )?;
    if let Err(error) = ensure_startup_live(startup_deadline, &cancellation) {
        cancellation.cancel();
        return Err(error);
    }
    Ok(PreparedAutoAttachDuplex {
        authority,
        prepared,
        startup_deadline,
        cancellation,
    })
}

impl<B: ControlPipeBackend> PreparedAutoAttachDuplex<B> {
    pub fn authority(&self) -> &DuplexAttemptAuthority {
        &self.authority
    }

    pub fn listener_ready_proof(&self) -> &ListenerReadyProof {
        self.prepared.listener_ready_proof()
    }

    /// Consume the prepared listener at the actual one-shot injection seam.
    /// Only a successful injector return can construct [`InjectionCommitted`],
    /// and the move prevents a second commit using this listener authority.
    pub fn commit_single_injection<F>(
        self,
        inject_once: F,
    ) -> Result<InjectionCommitted<B>, DuplexWorkerError>
    where
        F: FnOnce(&ListenerReadyProof, &DuplexAttemptAuthority) -> Result<(), DuplexWorkerError>,
    {
        ensure_startup_live(self.startup_deadline, &self.cancellation)?;
        if let Err(error) = inject_once(self.prepared.listener_ready_proof(), &self.authority) {
            self.cancellation.cancel();
            return Err(error);
        }
        if let Err(error) = ensure_startup_live(self.startup_deadline, &self.cancellation) {
            self.cancellation.cancel();
            return Err(error);
        }
        Ok(InjectionCommitted {
            authority: self.authority,
            prepared: self.prepared,
            startup_deadline: self.startup_deadline,
            cancellation: self.cancellation,
        })
    }
}

/// Opaque type-state proof that the one-shot injector returned success after
/// the secured H4CT listener was ready. There is intentionally no public
/// constructor and no route back to an uncommitted prepared listener.
pub struct InjectionCommitted<B: ControlPipeBackend> {
    authority: DuplexAttemptAuthority,
    prepared: PreparedControlListener<B>,
    startup_deadline: ControlDeadline,
    cancellation: ControlCancellation,
}

impl<B: ControlPipeBackend> InjectionCommitted<B> {
    /// Dormant fail-closed admission seam.  Peer PID/creation time is checked
    /// while accepting, but the channel starts with no consumer capabilities.
    /// The first collector-originated hello must decode and admit the exact
    /// capability word before any heartbeat/status/identity dispatch is legal.
    pub(crate) fn accept_after_single_injection_offline(
        self,
    ) -> Result<DuplexWorkerSession<B::Io>, DuplexWorkerError> {
        ensure_startup_live(self.startup_deadline, &self.cancellation)?;
        let none = ControlCapabilities::from_bits(0).expect("empty capability set");
        let channel = self.prepared.accept_and_admit(
            self.authority.session_id(),
            self.authority.expected_peer(),
            none,
            none,
            EndpointRole::Recorder,
            self.startup_deadline,
            &self.cancellation,
        )?;
        ensure_startup_live(self.startup_deadline, &self.cancellation)?;
        Ok(DuplexWorkerSession::new(
            channel,
            self.authority.collector_hello_expectation().clone(),
            self.startup_deadline,
            self.cancellation,
        ))
    }

    pub fn accept_for_product(
        self,
        _permit: AutoAttachDuplexProductPermit,
    ) -> Result<DuplexWorkerSession<B::Io>, DuplexWorkerError> {
        self.accept_after_single_injection_offline()
    }
}

fn ensure_startup_live(
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
) -> Result<(), DuplexWorkerError> {
    if cancellation.is_cancelled() {
        return Err(DuplexWorkerError::StartupCancelled);
    }
    if deadline.is_expired() {
        return Err(DuplexWorkerError::StartupDeadlineExpired);
    }
    Ok(())
}

/// Auto-attach worker control state. H4CT work is synchronous but bounded; a
/// future process worker runs heartbeat cycles on its existing control thread
/// and exposes this object through the nonblocking [`WorkerHandle`] poll seam.
pub struct DuplexWorkerSession<I: DeadlineControlIo> {
    channel: DeadlineControlChannel<I>,
    collector_hello_expectation: ProofExpectation,
    startup_deadline: ControlDeadline,
    heartbeat: RecorderHeartbeat,
    transport_cancellation: ControlCancellation,
    graceful_stop: GracefulStopToken,
    state: DuplexWorkerState,
    last_proven_alive: Option<Instant>,
    waiting_for_world_since: Option<Instant>,
    recording_admission: Option<DuplexRecordingAdmission>,
    last_confirmed_hour: Option<i64>,
    stop_requested_at: Option<Instant>,
    replay_path: Option<String>,
    failure: Option<String>,
    sink_failure: Option<SinkFailureReport>,
    required_capabilities: ControlCapabilities,
}

impl<I: DeadlineControlIo> DuplexWorkerSession<I> {
    pub(crate) fn new(
        channel: DeadlineControlChannel<I>,
        collector_hello_expectation: ProofExpectation,
        startup_deadline: ControlDeadline,
        transport_cancellation: ControlCancellation,
    ) -> Self {
        Self::new_with_capabilities(
            channel,
            collector_hello_expectation,
            startup_deadline,
            transport_cancellation,
            required_auto_attach_duplex_capabilities(),
        )
    }

    #[cfg(test)]
    pub(crate) fn new_rollover_offline(
        channel: DeadlineControlChannel<I>,
        collector_hello_expectation: ProofExpectation,
        startup_deadline: ControlDeadline,
        transport_cancellation: ControlCancellation,
    ) -> Self {
        Self::new_with_capabilities(
            channel,
            collector_hello_expectation,
            startup_deadline,
            transport_cancellation,
            ControlCapabilities::auto_attach_duplex_rollover_v1(),
        )
    }

    fn new_with_capabilities(
        channel: DeadlineControlChannel<I>,
        collector_hello_expectation: ProofExpectation,
        startup_deadline: ControlDeadline,
        transport_cancellation: ControlCancellation,
        required_capabilities: ControlCapabilities,
    ) -> Self {
        let graceful_stop =
            GracefulStopToken::with_transport_cancellation(transport_cancellation.clone());
        Self {
            channel,
            collector_hello_expectation,
            startup_deadline,
            heartbeat: RecorderHeartbeat::default(),
            transport_cancellation,
            graceful_stop,
            state: DuplexWorkerState::Handshaking,
            last_proven_alive: None,
            waiting_for_world_since: None,
            recording_admission: None,
            last_confirmed_hour: None,
            stop_requested_at: None,
            replay_path: None,
            failure: None,
            sink_failure: None,
            required_capabilities,
        }
    }

    pub fn state(&self) -> DuplexWorkerState {
        self.state
    }

    pub fn graceful_stop_token(&self) -> GracefulStopToken {
        self.graceful_stop.clone()
    }

    pub fn receive_collector_hello(
        &mut self,
        observed_at: Instant,
    ) -> Result<(), DuplexWorkerError> {
        self.require_state(DuplexWorkerState::Handshaking, "accept collector hello")?;
        if let Err(error) = ensure_startup_live(self.startup_deadline, &self.transport_cancellation)
        {
            self.fail(error.to_string());
            return Err(error);
        }
        let frame = match self
            .channel
            .recv_handshake_until(self.startup_deadline, &self.transport_cancellation)
        {
            Ok(frame) => frame,
            Err(error) => {
                if self.graceful_stop.is_requested() {
                    self.stop_requested_at.get_or_insert(observed_at);
                    self.state = DuplexWorkerState::StopRequested;
                    return Err(DuplexWorkerError::GracefulStopInterrupted);
                }
                self.fail(error.to_string());
                return Err(DuplexWorkerError::Transport(error.to_string()));
            }
        };
        if matches!(
            frame.kind,
            MessageKind::SinkSetupFailure | MessageKind::SinkTerminalFailure
        ) {
            return Err(self.consume_sink_failure(frame));
        }
        if frame.kind != MessageKind::CollectorHello {
            let error = format!("expected CollectorHello, got {:?}", frame.kind);
            self.fail(error.clone());
            return Err(DuplexWorkerError::CollectorHello(error));
        }
        let observed_capabilities = match decode_collector_hello_capabilities(
            &frame.payload,
            &self.collector_hello_expectation,
        ) {
            Ok(capabilities) => capabilities,
            Err(error) => {
                self.channel.poison();
                self.fail(error.clone());
                return Err(DuplexWorkerError::CollectorHello(error));
            }
        };
        if let Err(error) = self
            .channel
            .admit_decoded_hello_capabilities(self.required_capabilities, observed_capabilities)
        {
            let error = error.to_string();
            self.fail(error.clone());
            return Err(DuplexWorkerError::CollectorHello(error));
        }
        if let Err(error) = ensure_startup_live(self.startup_deadline, &self.transport_cancellation)
        {
            self.fail(error.to_string());
            return Err(error);
        }
        self.state = DuplexWorkerState::WaitingForWorld;
        self.last_proven_alive = Some(observed_at);
        self.waiting_for_world_since = Some(observed_at);
        Ok(())
    }

    pub(crate) fn heartbeat_cycle(
        &mut self,
        challenge: [u8; 16],
        deadline: ControlDeadline,
    ) -> Result<HeartbeatProof, DuplexWorkerError> {
        self.heartbeat_cycle_at(challenge, deadline, Instant::now())
    }

    /// Route the exact first H4CR record through the durable identity
    /// dispatcher using this session's private H4CT channel. This is the only
    /// public CFA1-send seam; callers cannot extract or cross-wire the channel.
    pub fn acknowledge_first_compact_record<S: IdentityDurableCandidateSink>(
        &mut self,
        identity: &mut RecorderIdentityControlDispatcher<S>,
        first_record: ValidatedFirstCompactRecord,
        deadline: ControlDeadline,
    ) -> Result<AcknowledgedFirstCompactRecord, DuplexWorkerError> {
        self.require_state(
            DuplexWorkerState::WaitingForWorld,
            "acknowledge first compact record",
        )?;
        match identity.acknowledge_first_record(
            &mut self.channel,
            first_record,
            deadline,
            &self.transport_cancellation,
        ) {
            Ok(acknowledged) => Ok(acknowledged),
            Err(error) => {
                let error = DuplexWorkerError::IdentityControl(error.to_string());
                self.fail(error.to_string());
                Err(error)
            }
        }
    }

    /// Rollover-specific second barrier.  It reuses the one durable CFA1
    /// dispatcher and then binds that exact acknowledgement to the accepted
    /// CRP1 before returning it to the recording-admission constructor.
    pub fn acknowledge_rollover_first_compact_record<S: IdentityDurableCandidateSink>(
        &mut self,
        rollover: &mut RecorderRolloverCoordinator,
        identity: &mut RecorderIdentityControlDispatcher<S>,
        first_record: ValidatedFirstCompactRecord,
        deadline: ControlDeadline,
    ) -> Result<AcknowledgedFirstCompactRecord, DuplexWorkerError> {
        let acknowledged =
            self.acknowledge_first_compact_record(identity, first_record, deadline)?;
        if let Err(error) = rollover.admit_new_header(acknowledged.first_record().acknowledgement())
        {
            let error = DuplexWorkerError::RolloverControl(error.to_string());
            self.fail(error.to_string());
            return Err(error);
        }
        Ok(acknowledged)
    }

    pub(crate) fn heartbeat_cycle_at(
        &mut self,
        challenge: [u8; 16],
        deadline: ControlDeadline,
        observed_at: Instant,
    ) -> Result<HeartbeatProof, DuplexWorkerError> {
        self.sync_external_stop(observed_at);
        if !matches!(
            self.state,
            DuplexWorkerState::WaitingForWorld | DuplexWorkerState::Recording
        ) {
            return Err(DuplexWorkerError::IllegalTransition {
                from: self.state,
                action: "run heartbeat",
            });
        }
        let result: Result<HeartbeatProof, DuplexWorkerError> = (|| {
            self.heartbeat
                .send_challenge(
                    &mut self.channel,
                    challenge,
                    deadline,
                    &self.transport_cancellation,
                )
                .map_err(DuplexWorkerError::Heartbeat)?;
            let frame = self
                .channel
                .recv_until(deadline, &self.transport_cancellation)
                .map_err(|error| DuplexWorkerError::Transport(error.to_string()))?;
            if matches!(
                frame.kind,
                MessageKind::SinkSetupFailure | MessageKind::SinkTerminalFailure
            ) {
                return Err(self.consume_sink_failure(frame));
            }
            self.heartbeat
                .accept_proof_frame(&mut self.channel, frame)
                .map_err(DuplexWorkerError::Heartbeat)
        })();
        match result {
            Ok(proof) => {
                self.last_proven_alive = Some(observed_at);
                Ok(proof)
            }
            Err(error) => {
                if self.graceful_stop.is_requested() {
                    self.stop_requested_at.get_or_insert(observed_at);
                    self.state = DuplexWorkerState::StopRequested;
                    return Err(DuplexWorkerError::GracefulStopInterrupted);
                }
                if self.state != DuplexWorkerState::Failed {
                    self.fail(error.to_string());
                }
                Err(error)
            }
        }
    }

    /// Production-facing recurring liveness cycle with one central H4CT
    /// receive router. Asynchronous CIF1 frames are consumed through the same
    /// channel by the durable identity dispatcher; the loop then continues
    /// under the unchanged heartbeat deadline until the matching proof.
    pub fn heartbeat_cycle_with_identity<S: IdentityDurableCandidateSink>(
        &mut self,
        identity: &mut RecorderIdentityControlDispatcher<S>,
        challenge: [u8; 16],
        deadline: ControlDeadline,
    ) -> Result<RoutedHeartbeatCycle, DuplexWorkerError> {
        self.heartbeat_cycle_with_identity_at(identity, challenge, deadline, Instant::now())
    }

    pub(crate) fn heartbeat_cycle_with_identity_at<S: IdentityDurableCandidateSink>(
        &mut self,
        identity: &mut RecorderIdentityControlDispatcher<S>,
        challenge: [u8; 16],
        deadline: ControlDeadline,
        observed_at: Instant,
    ) -> Result<RoutedHeartbeatCycle, DuplexWorkerError> {
        self.sync_external_stop(observed_at);
        if !matches!(
            self.state,
            DuplexWorkerState::WaitingForWorld | DuplexWorkerState::Recording
        ) {
            return Err(DuplexWorkerError::IllegalTransition {
                from: self.state,
                action: "run routed heartbeat",
            });
        }
        let result: Result<RoutedHeartbeatCycle, DuplexWorkerError> = (|| {
            self.heartbeat
                .send_challenge(
                    &mut self.channel,
                    challenge,
                    deadline,
                    &self.transport_cancellation,
                )
                .map_err(DuplexWorkerError::Heartbeat)?;
            let mut acknowledged_identity_failures = Vec::new();
            loop {
                let frame = self
                    .channel
                    .recv_until(deadline, &self.transport_cancellation)
                    .map_err(|error| DuplexWorkerError::Transport(error.to_string()))?;
                if matches!(
                    frame.kind,
                    MessageKind::SinkSetupFailure | MessageKind::SinkTerminalFailure
                ) {
                    return Err(self.consume_sink_failure(frame));
                }
                if frame.kind == MessageKind::IdentityFailure {
                    let acknowledged = identity
                        .dispatch_identity_failure_frame(
                            &mut self.channel,
                            frame,
                            deadline,
                            &self.transport_cancellation,
                        )
                        .map_err(|error| DuplexWorkerError::IdentityControl(error.to_string()))?;
                    acknowledged_identity_failures.push(acknowledged);
                    continue;
                }
                let proof = self
                    .heartbeat
                    .accept_proof_frame(&mut self.channel, frame)
                    .map_err(DuplexWorkerError::Heartbeat)?;
                return Ok(RoutedHeartbeatCycle {
                    proof,
                    acknowledged_identity_failures,
                });
            }
        })();
        match result {
            Ok(outcome) => {
                self.last_proven_alive = Some(observed_at);
                Ok(outcome)
            }
            Err(error) => {
                if self.graceful_stop.is_requested() {
                    self.stop_requested_at.get_or_insert(observed_at);
                    self.state = DuplexWorkerState::StopRequested;
                    return Err(DuplexWorkerError::GracefulStopInterrupted);
                }
                if self.state != DuplexWorkerState::Failed {
                    self.fail(error.to_string());
                }
                Err(error)
            }
        }
    }

    pub fn first_frame_valid(
        &mut self,
        admission: DuplexRecordingAdmission,
    ) -> Result<(), DuplexWorkerError> {
        self.require_state(DuplexWorkerState::WaitingForWorld, "accept first frame")?;
        self.last_confirmed_hour = Some(admission.confirmed_hour());
        self.recording_admission = Some(admission);
        self.waiting_for_world_since = None;
        self.state = DuplexWorkerState::Recording;
        Ok(())
    }

    pub fn recording_admission(&self) -> Option<&DuplexRecordingAdmission> {
        self.recording_admission.as_ref()
    }

    /// Dedicated CRP1 receive/drain/seal/open/CRA1 transaction.  Success resets
    /// recording admission and enters the existing first-record barrier for the
    /// new sequence-zero CID1.  The next call must durably acknowledge that CID1
    /// before [`Self::first_frame_valid`] can restore Recording.
    pub fn receive_rollover_and_prepare_new_header<S: RecorderRolloverStore>(
        &mut self,
        coordinator: &mut RecorderRolloverCoordinator,
        store: &mut S,
        deadline: ControlDeadline,
    ) -> Result<compact_contract::campaign_rollover::RolloverProposeV1, DuplexWorkerError> {
        self.require_state(DuplexWorkerState::Recording, "receive campaign rollover")?;
        if self.required_capabilities != ControlCapabilities::auto_attach_duplex_rollover_v1() {
            return Err(DuplexWorkerError::ActivationDisabled);
        }
        let proposal = match coordinator.receive_drain_seal_open_and_ack(
            &mut self.channel,
            store,
            deadline,
            &self.transport_cancellation,
        ) {
            Ok(proposal) => proposal,
            Err(error) => {
                let error: RecorderRolloverError = error;
                let error = DuplexWorkerError::RolloverControl(error.to_string());
                self.fail(error.to_string());
                return Err(error);
            }
        };
        self.recording_admission = None;
        self.last_confirmed_hour = None;
        self.waiting_for_world_since = Some(Instant::now());
        self.state = DuplexWorkerState::WaitingForWorld;
        Ok(proposal)
    }

    pub fn record_hour(&mut self, game_hour: i64) -> Result<(), DuplexWorkerError> {
        self.require_state(DuplexWorkerState::Recording, "record confirmed hour")?;
        if self
            .last_confirmed_hour
            .is_some_and(|previous| game_hour < previous)
        {
            let error = DuplexWorkerError::IllegalTransition {
                from: self.state,
                action: "regress confirmed hour",
            };
            self.fail(error.to_string());
            return Err(error);
        }
        self.last_confirmed_hour = Some(game_hour);
        Ok(())
    }

    pub fn check_liveness(&mut self) -> Result<(), DuplexWorkerError> {
        self.check_liveness_at(Instant::now())
    }

    pub(crate) fn check_liveness_at(&mut self, now: Instant) -> Result<(), DuplexWorkerError> {
        self.sync_external_stop(now);
        if matches!(
            self.state,
            DuplexWorkerState::WaitingForWorld | DuplexWorkerState::Recording
        ) {
            if self.state == DuplexWorkerState::WaitingForWorld
                && self.waiting_for_world_since.is_some_and(|started| {
                    now.saturating_duration_since(started)
                        >= Duration::from_millis(policy::FIRST_FRAME_DEADLINE_MS)
                })
            {
                self.fail("first-world-frame deadline expired".to_string());
                return Err(DuplexWorkerError::FirstFrameDeadlineExpired);
            }
            let expired = self.last_proven_alive.is_none_or(|proven| {
                now.saturating_duration_since(proven)
                    >= Duration::from_millis(policy::HEARTBEAT_LIVENESS_MS)
            });
            if expired {
                self.fail("heartbeat liveness expired".to_string());
                return Err(DuplexWorkerError::HeartbeatLivenessExpired);
            }
        }
        Ok(())
    }

    pub fn request_graceful_stop_at(&mut self, now: Instant) {
        if matches!(
            self.state,
            DuplexWorkerState::Complete | DuplexWorkerState::Failed
        ) {
            return;
        }
        self.graceful_stop.request();
        self.stop_requested_at.get_or_insert(now);
        self.state = DuplexWorkerState::StopRequested;
    }

    pub fn begin_finalizing(&mut self) -> Result<(), DuplexWorkerError> {
        if !matches!(
            self.state,
            DuplexWorkerState::StopRequested | DuplexWorkerState::Recording
        ) {
            return Err(DuplexWorkerError::IllegalTransition {
                from: self.state,
                action: "begin finalizing",
            });
        }
        self.state = DuplexWorkerState::Finalizing;
        Ok(())
    }

    pub fn check_graceful_stop_deadline_at(
        &mut self,
        now: Instant,
    ) -> Result<(), DuplexWorkerError> {
        if matches!(
            self.state,
            DuplexWorkerState::StopRequested | DuplexWorkerState::Finalizing
        ) && self.stop_requested_at.is_some_and(|started| {
            now.saturating_duration_since(started)
                >= Duration::from_millis(policy::GRACEFUL_STOP_DEADLINE_MS)
        }) {
            self.fail("graceful-stop deadline expired".to_string());
            return Err(DuplexWorkerError::GracefulStopExpired);
        }
        Ok(())
    }

    pub fn complete(&mut self, replay_path: String) -> Result<(), DuplexWorkerError> {
        self.require_state(DuplexWorkerState::Finalizing, "complete")?;
        if replay_path.is_empty() {
            self.fail("completion has no replay path".to_string());
            return Err(DuplexWorkerError::MissingReplayPath);
        }
        self.replay_path = Some(replay_path);
        self.state = DuplexWorkerState::Complete;
        Ok(())
    }

    pub fn cancel_transport(&self) {
        self.transport_cancellation.cancel();
    }

    /// Receive an independently framed terminal sink report outside a heartbeat
    /// exchange. The report is never read from or appended to H4CR.
    pub(crate) fn receive_terminal_sink_status(
        &mut self,
        deadline: ControlDeadline,
    ) -> Result<(), DuplexWorkerError> {
        let frame = match self
            .channel
            .recv_until(deadline, &self.transport_cancellation)
        {
            Ok(frame) => frame,
            Err(error) => {
                self.fail(error.to_string());
                return Err(DuplexWorkerError::Transport(error.to_string()));
            }
        };
        Err(self.consume_sink_failure(frame))
    }

    pub fn sink_failure(&self) -> Option<&SinkFailureReport> {
        self.sink_failure.as_ref()
    }

    fn consume_sink_failure(&mut self, frame: ControlFrame) -> DuplexWorkerError {
        match decode_sink_failure(frame.kind, &frame.payload) {
            Ok(report) => {
                self.sink_failure = Some(report.clone());
                self.fail(format!("authoritative sink transport failed: {report:?}"));
                DuplexWorkerError::SinkFailure(report)
            }
            Err(error) => {
                self.fail(format!("invalid sink failure status: {error}"));
                DuplexWorkerError::SinkStatus(error)
            }
        }
    }

    fn sync_external_stop(&mut self, now: Instant) {
        if self.graceful_stop.is_requested()
            && !matches!(
                self.state,
                DuplexWorkerState::StopRequested
                    | DuplexWorkerState::Finalizing
                    | DuplexWorkerState::Complete
                    | DuplexWorkerState::Failed
            )
        {
            self.stop_requested_at.get_or_insert(now);
            self.state = DuplexWorkerState::StopRequested;
        }
    }

    fn require_state(
        &self,
        expected: DuplexWorkerState,
        action: &'static str,
    ) -> Result<(), DuplexWorkerError> {
        if self.state == expected {
            Ok(())
        } else {
            Err(DuplexWorkerError::IllegalTransition {
                from: self.state,
                action,
            })
        }
    }

    fn fail(&mut self, reason: String) {
        self.transport_cancellation.cancel();
        self.channel.poison();
        self.failure = Some(reason);
        self.state = DuplexWorkerState::Failed;
    }

    fn worker_status(&self) -> WorkerStatus {
        match self.state {
            DuplexWorkerState::Handshaking => WorkerStatus::Handshaking,
            DuplexWorkerState::WaitingForWorld => WorkerStatus::Attached,
            DuplexWorkerState::Recording => WorkerStatus::Recording {
                last_confirmed_hour: self.last_confirmed_hour,
            },
            DuplexWorkerState::StopRequested | DuplexWorkerState::Finalizing => {
                WorkerStatus::Finalizing
            }
            DuplexWorkerState::Complete => WorkerStatus::Complete {
                replay_path: self.replay_path.clone().unwrap_or_default(),
            },
            DuplexWorkerState::Failed => WorkerStatus::Failed {
                reason: self
                    .failure
                    .clone()
                    .unwrap_or_else(|| "duplex worker failed".to_string()),
            },
        }
    }
}

impl<I: DeadlineControlIo + Send> WorkerHandle for DuplexWorkerSession<I> {
    fn poll(&mut self) -> WorkerStatus {
        self.sync_external_stop(Instant::now());
        self.worker_status()
    }

    fn request_stop(&mut self) {
        self.request_graceful_stop_at(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::io;
    use std::path::PathBuf;
    use std::rc::Rc;

    use super::super::identity::ProcessKey;
    use super::super::sources::StagedPayload;
    use crate::control_transport::{encode_frame, SessionId};
    use crate::duplex_session::{encode_heartbeat_proof, HeartbeatState};
    use crate::transport_status::{
        encode_sink_failure, SinkErrorClass, SinkFailureKind, SinkFailureMessageKind,
        SinkTerminality, SinkTopology,
    };

    #[derive(Default)]
    struct MockIo {
        inbound: VecDeque<u8>,
        outbound: Vec<u8>,
    }

    impl DeadlineControlIo for MockIo {
        fn send_all_until(
            &mut self,
            bytes: &[u8],
            _deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            if cancellation.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
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

    struct MockBackend {
        peer: ControlPeerIdentity,
        io: Option<MockIo>,
        created: bool,
    }

    impl ControlPipeBackend for MockBackend {
        type Listener = ();
        type Io = MockIo;

        fn create_restrictive_listener(
            &mut self,
            _pipe_name: &str,
            _security: RestrictiveSecurityPolicy,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<Self::Listener, ControlPipeError> {
            self.created = true;
            Ok(())
        }

        fn accept(
            &mut self,
            _listener: Self::Listener,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<Self::Io, ControlPipeError> {
            assert!(self.created);
            Ok(self.io.take().unwrap_or_default())
        }

        fn peer_identity(
            &mut self,
            _io: &Self::Io,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<ControlPeerIdentity, ControlPipeError> {
            Ok(self.peer)
        }
    }

    struct PairDataBackend {
        events: Rc<RefCell<Vec<String>>>,
        deadlines: Rc<RefCell<Vec<Instant>>>,
        peer: ControlPeerIdentity,
    }

    impl DataPipeListenerBackend for PairDataBackend {
        type Listener = String;
        type Io = String;

        fn create_inbound_listener(
            &mut self,
            pipe_name: &str,
            _security: RestrictiveSecurityPolicy,
            deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<Self::Listener, String> {
            self.events
                .borrow_mut()
                .push(format!("h4cr-create:{pipe_name}"));
            self.deadlines.borrow_mut().push(deadline.instant());
            Ok(pipe_name.to_string())
        }

        fn accept(
            &mut self,
            listener: Self::Listener,
            deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<Self::Io, String> {
            self.events.borrow_mut().push("h4cr-accept".to_string());
            self.deadlines.borrow_mut().push(deadline.instant());
            Ok(listener)
        }

        fn peer_identity(
            &mut self,
            _io: &Self::Io,
            deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<ControlPeerIdentity, String> {
            self.events.borrow_mut().push("h4cr-peer".to_string());
            self.deadlines.borrow_mut().push(deadline.instant());
            Ok(self.peer)
        }
    }

    struct PairControlBackend {
        events: Rc<RefCell<Vec<String>>>,
        deadlines: Rc<RefCell<Vec<Instant>>>,
        peer: ControlPeerIdentity,
        io: Option<MockIo>,
    }

    impl ControlPipeBackend for PairControlBackend {
        type Listener = String;
        type Io = MockIo;

        fn create_restrictive_listener(
            &mut self,
            pipe_name: &str,
            _security: RestrictiveSecurityPolicy,
            deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<Self::Listener, ControlPipeError> {
            self.events
                .borrow_mut()
                .push(format!("h4ct-create:{pipe_name}"));
            self.deadlines.borrow_mut().push(deadline.instant());
            Ok(pipe_name.to_string())
        }

        fn accept(
            &mut self,
            _listener: Self::Listener,
            deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<Self::Io, ControlPipeError> {
            self.events.borrow_mut().push("h4ct-accept".to_string());
            self.deadlines.borrow_mut().push(deadline.instant());
            Ok(self.io.take().unwrap_or_default())
        }

        fn peer_identity(
            &mut self,
            _io: &Self::Io,
            deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<ControlPeerIdentity, ControlPipeError> {
            self.events.borrow_mut().push("h4ct-peer".to_string());
            self.deadlines.borrow_mut().push(deadline.instant());
            Ok(self.peer)
        }
    }

    struct PairTargetMonitor {
        alive: VecDeque<bool>,
    }

    impl ExpectedTargetMonitor for PairTargetMonitor {
        fn expected_target_alive(&mut self, expected: ProcessKey) -> Result<bool, String> {
            assert_eq!(expected, ProcessKey::new(42, 9_001));
            Ok(self.alive.pop_front().unwrap_or(true))
        }
    }

    fn worker_capabilities() -> ControlCapabilities {
        required_auto_attach_duplex_capabilities()
    }

    fn expectation() -> ProofExpectation {
        ProofExpectation {
            pid: 42,
            creation_filetime: 9_001,
            h4cr_version: compact_contract::COMPACT_RECORD_VERSION,
            dll_sha256: [3; 32],
            challenge: [4; 16],
        }
    }

    fn worker_spec_with_session(session_id: [u8; 16]) -> WorkerSpec {
        WorkerSpec {
            key: ProcessKey::new(42, 9_001),
            attempt_id: "attempt-1".to_string(),
            payload: StagedPayload {
                staging_dir: PathBuf::from("staging"),
                dll_path: PathBuf::from("staging/collector.dll"),
                control_path: PathBuf::from("staging/control.json"),
                dll_sha256: "03".repeat(32),
                source_sha256: "03".repeat(32),
                pipe_token: encode_hex(&session_id),
                pipe_token_hash: "not-authority".to_string(),
            },
            out_dir: PathBuf::from("out"),
            session_id: "replay-session".to_string(),
            control_challenge: [4; 16],
        }
    }

    fn worker_spec() -> WorkerSpec {
        worker_spec_with_session([0x31; 16])
    }

    fn prepared_with_proofs(
        session_bytes: [u8; 16],
        proofs: &[([u8; 16], u64)],
    ) -> PreparedAutoAttachDuplex<MockBackend> {
        let session = SessionId::from_bytes(session_bytes).unwrap();
        let mut inbound = encode_frame(
            session,
            0,
            MessageKind::CollectorHello,
            &super::super::proof::collector_hello_with_capabilities_payload(
                &expectation(),
                worker_capabilities(),
            ),
        )
        .unwrap();
        for (index, (challenge, h4cr_sequence)) in proofs.iter().enumerate() {
            let payload = encode_heartbeat_proof(HeartbeatProof {
                challenge: *challenge,
                last_complete_h4cr_sequence: *h4cr_sequence,
                state: HeartbeatState::Producing,
            })
            .unwrap();
            inbound.extend_from_slice(
                &encode_frame(
                    session,
                    index as u64 + 1,
                    MessageKind::HeartbeatProof,
                    &payload,
                )
                .unwrap(),
            );
        }
        prepared_with_inbound(session_bytes, inbound)
    }

    fn prepared_with_inbound(
        session_id: [u8; 16],
        inbound: Vec<u8>,
    ) -> PreparedAutoAttachDuplex<MockBackend> {
        let backend = MockBackend {
            peer: ControlPeerIdentity {
                pid: 42,
                creation_filetime: 9_001,
            },
            io: Some(MockIo {
                inbound: inbound.into(),
                outbound: Vec::new(),
            }),
            created: false,
        };
        prepare_auto_attach_duplex_for_worker_spec_offline(
            backend,
            &worker_spec_with_session(session_id),
            RestrictiveSecurityPolicy::default(),
            ControlDeadline::after(Duration::from_secs(1)).unwrap(),
            ControlCancellation::new(),
        )
        .unwrap()
    }

    fn setup_sink_failure() -> SinkFailureReport {
        SinkFailureReport {
            kind: SinkFailureKind::Setup,
            requested: SinkTopology::from_bits(SinkTopology::FILE | SinkTopology::PIPE).unwrap(),
            opened: SinkTopology::from_bits(SinkTopology::FILE).unwrap(),
            failed: SinkTopology::from_bits(SinkTopology::PIPE).unwrap(),
            terminality: SinkTerminality::AttemptTerminal,
            error_class: SinkErrorClass::Win32,
            win32_error: Some(5),
            last_complete_sequence: None,
            failed_sequence: None,
            file_bytes_committed: 0,
            pipe_bytes_committed: 0,
        }
    }

    fn terminal_sink_failure() -> SinkFailureReport {
        SinkFailureReport {
            kind: SinkFailureKind::Terminal,
            requested: SinkTopology::from_bits(SinkTopology::FILE | SinkTopology::PIPE).unwrap(),
            opened: SinkTopology::from_bits(SinkTopology::FILE | SinkTopology::PIPE).unwrap(),
            failed: SinkTopology::from_bits(SinkTopology::PIPE).unwrap(),
            terminality: SinkTerminality::AttemptTerminal,
            error_class: SinkErrorClass::BrokenPipe,
            win32_error: Some(109),
            last_complete_sequence: Some(8),
            failed_sequence: Some(9),
            file_bytes_committed: 4_096,
            pipe_bytes_committed: 3_100,
        }
    }

    fn accept(prepared: PreparedAutoAttachDuplex<MockBackend>) -> DuplexWorkerSession<MockIo> {
        prepared
            .commit_single_injection(|_listener_ready, _authority| Ok(()))
            .unwrap()
            .accept_after_single_injection_offline()
            .unwrap()
    }

    #[test]
    fn combined_authority_creates_both_listeners_before_injection_and_never_splits_tuple() {
        let events = Rc::new(RefCell::new(Vec::<String>::new()));
        let deadlines = Rc::new(RefCell::new(Vec::<Instant>::new()));
        let peer = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9_001,
        };
        let session_bytes = [0xB1; 16];
        let session_id = SessionId::from_bytes(session_bytes).unwrap();
        let inbound = encode_frame(
            session_id,
            0,
            MessageKind::CollectorHello,
            &super::super::proof::collector_hello_with_capabilities_payload(
                &expectation(),
                worker_capabilities(),
            ),
        )
        .unwrap();
        let deadline = ControlDeadline::after(Duration::from_secs(2)).unwrap();
        let cancellation = ControlCancellation::new();
        let prepared = prepare_auto_attach_combined_for_worker_spec_offline(
            PairDataBackend {
                events: events.clone(),
                deadlines: deadlines.clone(),
                peer,
            },
            PairControlBackend {
                events: events.clone(),
                deadlines: deadlines.clone(),
                peer,
                io: Some(MockIo {
                    inbound: inbound.into(),
                    outbound: Vec::new(),
                }),
            },
            PairTargetMonitor {
                alive: VecDeque::new(),
            },
            &worker_spec_with_session(session_bytes),
            RestrictiveSecurityPolicy::default(),
            deadline,
            cancellation.clone(),
        )
        .unwrap();
        assert_eq!(
            events.borrow().as_slice(),
            [
                r"h4cr-create:\\.\pipe\battle-plan_collector_42_b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1",
                r"h4ct-create:\\.\pipe\battle-plan_control_42_b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1",
            ]
        );
        let committed = prepared
            .commit_single_injection(|ready, authority| {
                assert_eq!(ready.startup_deadline().instant(), deadline.instant());
                assert!(!ready.cancellation().is_cancelled());
                assert_eq!(authority.process_key(), ProcessKey::new(42, 9_001));
                events.borrow_mut().push("inject".to_string());
                Ok(())
            })
            .unwrap();
        let mut combined = committed
            .accept_both_after_single_injection_offline()
            .unwrap();
        combined
            .control_mut()
            .receive_collector_hello(Instant::now())
            .unwrap();
        assert_eq!(combined.authority().session_id(), session_bytes);
        assert_eq!(
            events.borrow().as_slice(),
            [
                r"h4cr-create:\\.\pipe\battle-plan_collector_42_b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1",
                r"h4ct-create:\\.\pipe\battle-plan_control_42_b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1",
                "inject",
                "h4cr-accept",
                "h4cr-peer",
                "h4ct-accept",
                "h4ct-peer",
            ]
        );
        assert!(deadlines
            .borrow()
            .iter()
            .all(|observed| *observed == deadline.instant()));
        assert!(!cancellation.is_cancelled());
    }

    #[test]
    fn target_exit_cancels_the_shared_startup_authority_before_injection() {
        let events = Rc::new(RefCell::new(Vec::<String>::new()));
        let deadlines = Rc::new(RefCell::new(Vec::<Instant>::new()));
        let peer = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9_001,
        };
        let cancellation = ControlCancellation::new();
        let prepared = prepare_auto_attach_combined_for_worker_spec_offline(
            PairDataBackend {
                events: events.clone(),
                deadlines: deadlines.clone(),
                peer,
            },
            PairControlBackend {
                events,
                deadlines,
                peer,
                io: Some(MockIo::default()),
            },
            PairTargetMonitor {
                alive: [true, true, true, false].into(),
            },
            &worker_spec_with_session([0xB2; 16]),
            RestrictiveSecurityPolicy::default(),
            ControlDeadline::after(Duration::from_secs(2)).unwrap(),
            cancellation.clone(),
        )
        .unwrap();
        let injection_calls = Rc::new(Cell::new(0));
        let observed = injection_calls.clone();
        assert!(matches!(
            prepared.commit_single_injection(move |_, _| {
                observed.set(observed.get() + 1);
                Ok(())
            }),
            Err(DuplexWorkerError::TargetExited)
        ));
        assert_eq!(injection_calls.get(), 0);
        assert!(cancellation.is_cancelled());
    }

    #[test]
    fn target_exit_after_both_accepts_cancels_bound_data_and_control_session() {
        let events = Rc::new(RefCell::new(Vec::<String>::new()));
        let deadlines = Rc::new(RefCell::new(Vec::<Instant>::new()));
        let peer = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9_001,
        };
        let cancellation = ControlCancellation::new();
        let prepared = prepare_auto_attach_combined_for_worker_spec_offline(
            PairDataBackend {
                events: events.clone(),
                deadlines: deadlines.clone(),
                peer,
            },
            PairControlBackend {
                events,
                deadlines,
                peer,
                io: Some(MockIo::default()),
            },
            PairTargetMonitor {
                alive: [true, true, true, true, true, true, true, true, false].into(),
            },
            &worker_spec_with_session([0xB3; 16]),
            RestrictiveSecurityPolicy::default(),
            ControlDeadline::after(Duration::from_secs(2)).unwrap(),
            cancellation.clone(),
        )
        .unwrap();
        let mut combined = prepared
            .commit_single_injection(|_, _| Ok(()))
            .unwrap()
            .accept_both_after_single_injection_offline()
            .unwrap();
        assert!(matches!(
            combined.check_expected_target_live(),
            Err(DuplexWorkerError::TargetExited)
        ));
        assert!(cancellation.is_cancelled());
        assert!(combined.cancellation().is_cancelled());
    }

    #[test]
    fn hello_capabilities_are_wire_derived_and_every_required_bit_is_fail_closed() {
        let required = required_auto_attach_duplex_capabilities();
        assert!(required.contains(
            ControlCapabilities::from_bits(ControlCapabilities::FIRST_COMPACT_ACK).unwrap()
        ));
        assert!(required.contains(
            ControlCapabilities::from_bits(ControlCapabilities::CAMPAIGN_IDENTITY).unwrap()
        ));
        let missing_bits = [
            ControlCapabilities::HEARTBEAT,
            ControlCapabilities::FIRST_COMPACT_ACK,
            ControlCapabilities::TRANSPORT_STATUS,
            ControlCapabilities::CAMPAIGN_IDENTITY,
        ];
        for (index, missing) in missing_bits.into_iter().enumerate() {
            let session_bytes = [0xC0 + index as u8; 16];
            let session_id = SessionId::from_bytes(session_bytes).unwrap();
            let advertised = ControlCapabilities::from_bits(required.bits() & !missing).unwrap();
            let wire = encode_frame(
                session_id,
                0,
                MessageKind::CollectorHello,
                &super::super::proof::collector_hello_with_capabilities_payload(
                    &expectation(),
                    advertised,
                ),
            )
            .unwrap();
            let mut session = accept(prepared_with_inbound(session_bytes, wire));
            assert!(matches!(
                session.receive_collector_hello(Instant::now()),
                Err(DuplexWorkerError::CollectorHello(_))
            ));
            assert_eq!(session.state(), DuplexWorkerState::Failed);
        }

        for (session_bytes, payload) in [
            (
                [0xC4; 16],
                super::super::proof::collector_hello_payload(&expectation()),
            ),
            ([0xC5; 16], {
                let mut payload = super::super::proof::collector_hello_with_capabilities_payload(
                    &expectation(),
                    required,
                );
                payload[68..72].copy_from_slice(&(1u32 << 31).to_le_bytes());
                payload
            }),
            ([0xC6; 16], {
                let advertised =
                    ControlCapabilities::from_bits(required.bits() | ControlCapabilities::ROLLOVER)
                        .unwrap();
                super::super::proof::collector_hello_with_capabilities_payload(
                    &expectation(),
                    advertised,
                )
            }),
            ([0xC7; 16], {
                let advertised =
                    ControlCapabilities::from_bits(required.bits() | ControlCapabilities::STAGE_B)
                        .unwrap();
                super::super::proof::collector_hello_with_capabilities_payload(
                    &expectation(),
                    advertised,
                )
            }),
        ] {
            let wire = encode_frame(
                SessionId::from_bytes(session_bytes).unwrap(),
                0,
                MessageKind::CollectorHello,
                &payload,
            )
            .unwrap();
            let mut session = accept(prepared_with_inbound(session_bytes, wire));
            assert!(session.receive_collector_hello(Instant::now()).is_err());
            assert_eq!(session.state(), DuplexWorkerState::Failed);
        }
    }

    #[test]
    fn activation_is_explicitly_disabled_and_listener_precedes_session() {
        assert!(!AUTO_ATTACH_DUPLEX_ACTIVATION_ENABLED);
        assert!(!auto_attach_identity_v1_enabled());
        assert!(matches!(
            auto_attach_duplex_product_permit(),
            Err(DuplexWorkerError::ActivationDisabled)
        ));
        let session_id = [0x31; 16];
        let prepared = prepared_with_proofs(session_id, &[]);
        let _proof_required_before_future_injection = prepared.listener_ready_proof();
        let session = accept(prepared);
        assert_eq!(session.state(), DuplexWorkerState::Handshaking);
    }

    #[test]
    fn worker_spec_is_the_single_source_of_duplex_attempt_authority() {
        let spec = worker_spec();
        let authority = DuplexAttemptAuthority::from_worker_spec(&spec).unwrap();

        assert_eq!(
            authority.control_pipe_name(),
            r"\\.\pipe\battle-plan_control_42_31313131313131313131313131313131"
        );
        assert_eq!(authority.session_id(), [0x31; 16]);
        assert_eq!(
            authority.expected_peer(),
            ControlPeerIdentity {
                pid: 42,
                creation_filetime: 9_001,
            }
        );
        assert_eq!(authority.collector_hello_expectation(), &expectation());
    }

    #[test]
    fn worker_spec_authority_rejects_aliases_and_staging_mismatch() {
        let mut uppercase = worker_spec();
        uppercase.payload.pipe_token = "ab".repeat(16);
        uppercase.payload.pipe_token.make_ascii_uppercase();
        assert!(matches!(
            DuplexAttemptAuthority::from_worker_spec(&uppercase),
            Err(DuplexWorkerError::AttemptAuthority(message))
                if message.contains("canonical lowercase")
        ));

        let mut zero = worker_spec();
        zero.payload.pipe_token = "00".repeat(16);
        assert!(matches!(
            DuplexAttemptAuthority::from_worker_spec(&zero),
            Err(DuplexWorkerError::AttemptAuthority(message))
                if message.contains("all zero")
        ));

        let mut mismatch = worker_spec();
        mismatch.payload.source_sha256 = "04".repeat(32);
        assert!(matches!(
            DuplexAttemptAuthority::from_worker_spec(&mismatch),
            Err(DuplexWorkerError::AttemptAuthority(message))
                if message.contains("does not match")
        ));
    }

    #[test]
    fn worker_spec_preparation_preserves_listener_before_injection_barrier() {
        let spec = worker_spec();
        let backend = MockBackend {
            peer: ControlPeerIdentity {
                pid: 42,
                creation_filetime: 9_001,
            },
            io: None,
            created: false,
        };
        let prepared = prepare_auto_attach_duplex_for_worker_spec_offline(
            backend,
            &spec,
            RestrictiveSecurityPolicy::default(),
            ControlDeadline::after(Duration::from_secs(1)).unwrap(),
            ControlCancellation::new(),
        )
        .unwrap();

        let _proof_required_before_future_injection = prepared.listener_ready_proof();
        assert_eq!(prepared.authority().session_id(), [0x31; 16]);
    }

    #[test]
    fn recurring_heartbeat_drives_liveness_without_skipping_first_frame_gate() {
        let session_id = [0x32; 16];
        let challenges = [[1; 16], [2; 16]];
        let prepared = prepared_with_proofs(session_id, &[(challenges[0], 7), (challenges[1], 9)]);
        let mut session = accept(prepared);
        let start = Instant::now();
        assert!(session
            .first_frame_valid(DuplexRecordingAdmission::for_test(10))
            .is_err());
        session.receive_collector_hello(start).unwrap();
        session
            .heartbeat_cycle_at(
                challenges[0],
                ControlDeadline::after(Duration::from_secs(1)).unwrap(),
                start + Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(session.state(), DuplexWorkerState::WaitingForWorld);
        session
            .first_frame_valid(DuplexRecordingAdmission::for_test(10))
            .unwrap();
        session
            .heartbeat_cycle_at(
                challenges[1],
                ControlDeadline::after(Duration::from_secs(1)).unwrap(),
                start + Duration::from_secs(2),
            )
            .unwrap();
        assert_eq!(
            session.worker_status(),
            WorkerStatus::Recording {
                last_confirmed_hour: Some(10)
            }
        );
        session
            .check_liveness_at(
                start
                    + Duration::from_millis(policy::HEARTBEAT_LIVENESS_MS)
                    + Duration::from_secs(3),
            )
            .unwrap_err();
        assert_eq!(session.state(), DuplexWorkerState::Failed);
    }

    #[test]
    fn recurring_heartbeat_does_not_extend_first_world_frame_deadline() {
        let session_id = [0x38; 16];
        let challenge = [8; 16];
        let prepared = prepared_with_proofs(session_id, &[(challenge, 7)]);
        let mut session = accept(prepared);
        let start = Instant::now();
        session.receive_collector_hello(start).unwrap();
        session
            .heartbeat_cycle_at(
                challenge,
                ControlDeadline::after(Duration::from_secs(1)).unwrap(),
                start + Duration::from_millis(policy::FIRST_FRAME_DEADLINE_MS - 1),
            )
            .unwrap();

        assert!(matches!(
            session
                .check_liveness_at(start + Duration::from_millis(policy::FIRST_FRAME_DEADLINE_MS)),
            Err(DuplexWorkerError::FirstFrameDeadlineExpired)
        ));
        assert_eq!(session.state(), DuplexWorkerState::Failed);
    }

    #[test]
    fn graceful_stop_is_observable_and_never_hard_kills_or_reinjects() {
        let session_id = [0x33; 16];
        let prepared = prepared_with_proofs(session_id, &[]);
        let mut session = accept(prepared);
        let start = Instant::now();
        session.receive_collector_hello(start).unwrap();
        session
            .first_frame_valid(DuplexRecordingAdmission::for_test(4))
            .unwrap();
        let token = session.graceful_stop_token();
        WorkerHandle::request_stop(&mut session);
        assert!(token.is_requested());
        assert_eq!(session.state(), DuplexWorkerState::StopRequested);
        assert_eq!(WorkerHandle::poll(&mut session), WorkerStatus::Finalizing);
        session.begin_finalizing().unwrap();
        session.complete("capture.battleplan".to_string()).unwrap();
        assert_eq!(
            WorkerHandle::poll(&mut session),
            WorkerStatus::Complete {
                replay_path: "capture.battleplan".to_string()
            }
        );
    }

    #[test]
    fn stale_heartbeat_and_graceful_stop_timeout_fail_terminally() {
        let session_id = [0x34; 16];
        let prepared = prepared_with_proofs(session_id, &[([9; 16], 1)]);
        let mut session = accept(prepared);
        let start = Instant::now();
        session.receive_collector_hello(start).unwrap();
        assert!(session
            .heartbeat_cycle_at(
                [8; 16],
                ControlDeadline::after(Duration::from_secs(1)).unwrap(),
                start,
            )
            .is_err());
        assert_eq!(session.state(), DuplexWorkerState::Failed);

        let session_id = [0x35; 16];
        let prepared = prepared_with_proofs(session_id, &[]);
        let mut session = accept(prepared);
        session.request_graceful_stop_at(start);
        assert!(matches!(
            session.check_graceful_stop_deadline_at(
                start + Duration::from_millis(policy::GRACEFUL_STOP_DEADLINE_MS)
            ),
            Err(DuplexWorkerError::GracefulStopExpired)
        ));
        assert_eq!(session.state(), DuplexWorkerState::Failed);
    }

    #[test]
    fn sink_failure_as_first_handshake_frame_is_terminal_but_structured_and_surfaced() {
        // Regression: the production control channel is admitted with an empty
        // capability set, and `SinkSetupFailure`/`SinkTerminalFailure` require
        // `TRANSPORT_STATUS`. Before this fix the ordinary capability gate poisoned
        // the channel on a pre-hello sink failure, so the branch that decodes the
        // structured report was unreachable, `sink_failure()` stayed `None`, and the
        // report's topology/error class was lost. The handshake must still FAIL, but
        // the failure must now be structured and reach `self.sink_failure`.
        for (session_bytes, report) in [
            ([0x36; 16], setup_sink_failure()),
            ([0x3E; 16], terminal_sink_failure()),
        ] {
            let session_id = SessionId::from_bytes(session_bytes).unwrap();
            let wire = encode_frame(
                session_id,
                0,
                report.kind.message_kind(),
                &encode_sink_failure(&report).unwrap(),
            )
            .unwrap();
            let prepared = prepared_with_inbound(session_bytes, wire);
            let mut session = accept(prepared);
            assert!(matches!(
                session.receive_collector_hello(Instant::now()),
                Err(DuplexWorkerError::SinkFailure(observed)) if observed == report
            ));
            assert_eq!(session.sink_failure(), Some(&report));
            assert_eq!(session.state(), DuplexWorkerState::Failed);
        }
    }

    #[test]
    fn non_sink_capability_gated_kind_before_hello_still_poisons_the_handshake() {
        // The pre-hello allowance is closed: it admits ONLY the two sink-failure
        // kinds. A different capability-gated kind (here `HeartbeatProof`, requiring
        // `HEARTBEAT`) arriving before the hello must still be denied and poison the
        // channel, so the allowance does not weaken the capability gate in general.
        let session_bytes = [0x3F; 16];
        let session_id = SessionId::from_bytes(session_bytes).unwrap();
        let payload = encode_heartbeat_proof(HeartbeatProof {
            challenge: [9; 16],
            last_complete_h4cr_sequence: 0,
            state: HeartbeatState::Producing,
        })
        .unwrap();
        let wire = encode_frame(session_id, 0, MessageKind::HeartbeatProof, &payload).unwrap();
        let prepared = prepared_with_inbound(session_bytes, wire);
        let mut session = accept(prepared);
        assert!(matches!(
            session.receive_collector_hello(Instant::now()),
            Err(DuplexWorkerError::Transport(_))
        ));
        assert_eq!(session.sink_failure(), None);
        assert_eq!(session.state(), DuplexWorkerState::Failed);
    }

    #[test]
    fn terminal_sink_failure_interrupts_heartbeat_without_h4cr_fallback() {
        let session_bytes = [0x37; 16];
        let session_id = SessionId::from_bytes(session_bytes).unwrap();
        let expected = expectation();
        let report = terminal_sink_failure();
        let mut inbound = encode_frame(
            session_id,
            0,
            MessageKind::CollectorHello,
            &super::super::proof::collector_hello_with_capabilities_payload(
                &expected,
                worker_capabilities(),
            ),
        )
        .unwrap();
        inbound.extend_from_slice(
            &encode_frame(
                session_id,
                1,
                report.kind.message_kind(),
                &encode_sink_failure(&report).unwrap(),
            )
            .unwrap(),
        );
        let prepared = prepared_with_inbound(session_bytes, inbound);
        let mut session = accept(prepared);
        let now = Instant::now();
        session.receive_collector_hello(now).unwrap();
        session
            .first_frame_valid(DuplexRecordingAdmission::for_test(10))
            .unwrap();
        assert!(matches!(
            session.heartbeat_cycle_at(
                [7; 16],
                ControlDeadline::after(Duration::from_secs(1)).unwrap(),
                now,
            ),
            Err(DuplexWorkerError::SinkFailure(observed)) if observed == report
        ));
        assert_eq!(session.sink_failure(), Some(&report));
        assert_eq!(session.state(), DuplexWorkerState::Failed);
    }

    // --- cross-crate loopback: REAL collector client vs REAL worker admission ---

    /// In-memory transport for the real collector client that captures the
    /// exact bytes it puts on the wire.
    struct ClientCaptureIo {
        outbound: Rc<RefCell<Vec<u8>>>,
    }

    impl collector::duplex_client::DeadlineControlIo for ClientCaptureIo {
        fn send_all(
            &mut self,
            bytes: &[u8],
            _deadline: collector::duplex_client::MonotonicDeadline,
        ) -> Result<(), collector::duplex_client::DuplexIoError> {
            self.outbound.borrow_mut().extend_from_slice(bytes);
            Ok(())
        }

        fn recv_exact(
            &mut self,
            _bytes: &mut [u8],
            _deadline: collector::duplex_client::MonotonicDeadline,
        ) -> Result<(), collector::duplex_client::DuplexIoError> {
            // The loopback only drives the collector-first hello send.
            Err(collector::duplex_client::DuplexIoError::PeerClosed)
        }
    }

    struct ClientZeroClock;

    impl collector::duplex_client::MonotonicClock for ClientZeroClock {
        fn now(&self) -> collector::duplex_client::MonotonicInstant {
            collector::duplex_client::MonotonicInstant(0)
        }
    }

    /// Drive the REAL collector client's `perform_handshake` over an in-memory
    /// transport and return the exact wire bytes it produced.
    fn real_collector_client_hello_bytes(client_session: [u8; 16]) -> Vec<u8> {
        let outbound = Rc::new(RefCell::new(Vec::new()));
        let session =
            compact_contract::control_transport::claim_session_capability(client_session).unwrap();
        let expected = expectation();
        let mut client = collector::duplex_client::CollectorControlClient::new(
            &collector::duplex_client::test_duplex_client_permit(),
            ClientCaptureIo {
                outbound: Rc::clone(&outbound),
            },
            ClientZeroClock,
            session,
            collector::duplex_client::CollectorIdentity::from_attach(
                expected.pid,
                expected.creation_filetime,
                expected.h4cr_version,
                expected.dll_sha256,
                expected.challenge,
            ),
            collector::duplex_client::DuplexClientPolicy::DEFAULT,
        );
        let deadline = collector::duplex_client::MonotonicDeadline::from_now(
            &ClientZeroClock,
            Duration::from_secs(30),
        );
        client
            .perform_handshake(
                &collector::duplex_client::test_duplex_client_permit(),
                deadline,
            )
            .unwrap();
        let bytes = outbound.borrow().clone();
        bytes
    }

    /// Compatibility loopback: the REAL collector client's collector-first hello
    /// is accepted by the REAL recorder worker-session admission.
    ///
    /// In production the two endpoints live in separate processes and each
    /// claims the per-attach session id once; a single test process cannot claim
    /// one id twice, so after the client's own framing is validated wire-exactly
    /// (same codec both sides: session id, sequence 0, kind), the byte-identical
    /// hello payload is re-framed under the worker's session id. The payload —
    /// the 72-byte capabilities form, the identity binding, and the capability
    /// word, i.e. everything admission judges — passes through untouched.
    #[test]
    fn real_collector_client_hello_is_admitted_by_the_worker_session() {
        let client_session = [0xD1; 16];
        let bytes = real_collector_client_hello_bytes(client_session);

        // The client framed exactly one CollectorHello at ITS sequence 0.
        let client_sid = SessionId::from_bytes(client_session).unwrap();
        let frame = crate::control_transport::decode_frame(&bytes, client_sid, 0).unwrap();
        assert_eq!(frame.kind, MessageKind::CollectorHello);
        assert_eq!(
            bytes.len(),
            crate::control_transport::CONTROL_HEADER_LEN + 72
        );

        // The payload decodes through the recorder's REAL admission decoder,
        // binds to the retained attempt facts, and claims exactly the required
        // capability word.
        let observed = super::super::proof::decode_collector_hello_capabilities(
            &frame.payload,
            &expectation(),
        )
        .unwrap();
        assert_eq!(observed, required_auto_attach_duplex_capabilities());

        // Full REAL worker-session admission over the identical payload.
        let worker_session = [0xD2; 16];
        let worker_sid = SessionId::from_bytes(worker_session).unwrap();
        let inbound =
            encode_frame(worker_sid, 0, MessageKind::CollectorHello, &frame.payload).unwrap();
        let mut session = accept(prepared_with_inbound(worker_session, inbound));
        session.receive_collector_hello(Instant::now()).unwrap();
        assert_eq!(session.state(), DuplexWorkerState::WaitingForWorld);
    }

    /// Negative loopback: the historical 68-byte hello — even byte-built by the
    /// shared codec with the exact expected identity — is rejected by product
    /// admission, which accepts only the 72-byte capabilities form.
    #[test]
    fn base_68_byte_collector_hello_is_rejected_by_worker_admission() {
        let expected = expectation();
        let payload = compact_contract::control_transport::payloads::encode_collector_hello(
            &compact_contract::control_transport::payloads::CollectorHelloV1 {
                pid: expected.pid,
                creation_filetime: expected.creation_filetime,
                h4cr_version: expected.h4cr_version,
                control_version: crate::control_transport::CONTROL_VERSION,
                dll_sha256: expected.dll_sha256,
                challenge: expected.challenge,
            },
        );
        assert_eq!(payload.len(), 68);

        let worker_session = [0xD3; 16];
        let worker_sid = SessionId::from_bytes(worker_session).unwrap();
        let inbound = encode_frame(worker_sid, 0, MessageKind::CollectorHello, &payload).unwrap();
        let mut session = accept(prepared_with_inbound(worker_session, inbound));
        let err = session.receive_collector_hello(Instant::now()).unwrap_err();
        assert!(
            matches!(err, DuplexWorkerError::CollectorHello(_)),
            "got {err:?}"
        );
        assert_eq!(session.state(), DuplexWorkerState::Failed);
    }

    // --- all four advertised capabilities, end-to-end against real host code ---

    /// In-memory duplex transport for the real client with BOTH a captured
    /// outbound stream and a scripted inbound stream (recorder->collector).
    struct ClientPairIo {
        outbound: Rc<RefCell<Vec<u8>>>,
        inbound: Rc<RefCell<std::collections::VecDeque<u8>>>,
    }

    impl collector::duplex_client::DeadlineControlIo for ClientPairIo {
        fn send_all(
            &mut self,
            bytes: &[u8],
            _deadline: collector::duplex_client::MonotonicDeadline,
        ) -> Result<(), collector::duplex_client::DuplexIoError> {
            self.outbound.borrow_mut().extend_from_slice(bytes);
            Ok(())
        }

        fn recv_exact(
            &mut self,
            bytes: &mut [u8],
            _deadline: collector::duplex_client::MonotonicDeadline,
        ) -> Result<(), collector::duplex_client::DuplexIoError> {
            let mut inbound = self.inbound.borrow_mut();
            if inbound.len() < bytes.len() {
                return Err(collector::duplex_client::DuplexIoError::PeerClosed);
            }
            for slot in bytes.iter_mut() {
                *slot = inbound.pop_front().expect("length checked");
            }
            Ok(())
        }
    }

    fn client_identity() -> collector::duplex_client::CollectorIdentity {
        let expected = expectation();
        collector::duplex_client::CollectorIdentity::from_attach(
            expected.pid,
            expected.creation_filetime,
            expected.h4cr_version,
            expected.dll_sha256,
            expected.challenge,
        )
    }

    fn far_client_deadline() -> collector::duplex_client::MonotonicDeadline {
        collector::duplex_client::MonotonicDeadline::from_now(
            &ClientZeroClock,
            Duration::from_secs(30),
        )
    }

    /// TRANSPORT_STATUS loopback: the REAL client's `report_sink_failure`
    /// produces a frame the REAL worker-session admission routes and surfaces
    /// as an authoritative terminal sink failure.
    #[test]
    fn real_client_sink_failure_is_routed_and_surfaced_by_the_worker() {
        let report = terminal_sink_failure();

        // Drive the real client: handshake, then report the sink failure.
        let outbound = Rc::new(RefCell::new(Vec::new()));
        let mut client = collector::duplex_client::CollectorControlClient::new(
            &collector::duplex_client::test_duplex_client_permit(),
            ClientPairIo {
                outbound: Rc::clone(&outbound),
                inbound: Rc::new(RefCell::new(std::collections::VecDeque::new())),
            },
            ClientZeroClock,
            compact_contract::control_transport::claim_session_capability([0xE1; 16]).unwrap(),
            client_identity(),
            collector::duplex_client::DuplexClientPolicy::DEFAULT,
        );
        let deadline = far_client_deadline();
        client
            .perform_handshake(
                &collector::duplex_client::test_duplex_client_permit(),
                deadline,
            )
            .unwrap();
        client.report_sink_failure(&report, deadline).unwrap();

        // The client's second frame (after the hello) is the sink failure. Decode
        // its framing with the shared codec, then re-frame the identical payload
        // at the worker's admission sequence 0 (a single test process cannot
        // claim one per-attach id twice).
        let bytes = outbound.borrow().clone();
        let hello_wire = crate::control_transport::CONTROL_HEADER_LEN + 72;
        let sink_frame = crate::control_transport::decode_frame(
            &bytes[hello_wire..],
            SessionId::from_bytes([0xE1; 16]).unwrap(),
            1,
        )
        .unwrap();
        assert_eq!(sink_frame.kind, MessageKind::SinkTerminalFailure);

        // The REAL worker session, waiting for the collector hello, routes a
        // sink-failure frame to its authoritative consumer and surfaces it.
        let worker_session = [0xE2; 16];
        let worker_sid = SessionId::from_bytes(worker_session).unwrap();
        let inbound = encode_frame(worker_sid, 0, sink_frame.kind, &sink_frame.payload).unwrap();
        let mut session = accept(prepared_with_inbound(worker_session, inbound));
        let err = session.receive_collector_hello(Instant::now()).unwrap_err();
        assert!(
            matches!(err, DuplexWorkerError::SinkFailure(_)),
            "got {err:?}"
        );
        assert_eq!(session.sink_failure(), Some(&report));
    }

    /// TRANSPORT_STATUS pre-hello loopback: the REAL client's PRE-HELLO sink
    /// SETUP failure (its very first frame, no hello) is accepted by the REAL
    /// worker session's first-frame window and surfaced as a structured failure
    /// rather than a bare timeout.
    #[test]
    fn real_client_pre_hello_sink_setup_failure_is_admitted_by_the_worker() {
        let report = setup_sink_failure();

        // Drive the real client: report the sink SETUP failure BEFORE any hello.
        let outbound = Rc::new(RefCell::new(Vec::new()));
        let mut client = collector::duplex_client::CollectorControlClient::new(
            &collector::duplex_client::test_duplex_client_permit(),
            ClientPairIo {
                outbound: Rc::clone(&outbound),
                inbound: Rc::new(RefCell::new(std::collections::VecDeque::new())),
            },
            ClientZeroClock,
            compact_contract::control_transport::claim_session_capability([0xE4; 16]).unwrap(),
            client_identity(),
            collector::duplex_client::DuplexClientPolicy::DEFAULT,
        );
        let deadline = far_client_deadline();
        client.report_sink_failure(&report, deadline).unwrap();

        // The client's FIRST and only frame is the sink setup failure at seq 0.
        let bytes = outbound.borrow().clone();
        let sink_frame = crate::control_transport::decode_frame(
            &bytes,
            SessionId::from_bytes([0xE4; 16]).unwrap(),
            0,
        )
        .unwrap();
        assert_eq!(sink_frame.kind, MessageKind::SinkSetupFailure);

        // The REAL worker session admits it through recv_handshake_until's
        // pre-hello window and surfaces the structured report.
        let worker_session = [0xE5; 16];
        let worker_sid = SessionId::from_bytes(worker_session).unwrap();
        let inbound = encode_frame(worker_sid, 0, sink_frame.kind, &sink_frame.payload).unwrap();
        let mut session = accept(prepared_with_inbound(worker_session, inbound));
        let err = session.receive_collector_hello(Instant::now()).unwrap_err();
        assert!(
            matches!(err, DuplexWorkerError::SinkFailure(_)),
            "got {err:?}"
        );
        assert_eq!(session.sink_failure(), Some(&report));
    }

    /// TRANSPORT_STATUS loopback (Terminal variant): a sink TERMINAL failure is
    /// ALSO a valid pre-hello first frame (both sinks opened, then the first
    /// sequence-0 write failed before the hello). The REAL client emits it at
    /// seq 0 and the REAL worker admits it through the same pre-hello window.
    #[test]
    fn real_client_pre_hello_sink_terminal_failure_is_admitted_by_the_worker() {
        let report = terminal_sink_failure();

        let outbound = Rc::new(RefCell::new(Vec::new()));
        let mut client = collector::duplex_client::CollectorControlClient::new(
            &collector::duplex_client::test_duplex_client_permit(),
            ClientPairIo {
                outbound: Rc::clone(&outbound),
                inbound: Rc::new(RefCell::new(std::collections::VecDeque::new())),
            },
            ClientZeroClock,
            compact_contract::control_transport::claim_session_capability([0xE6; 16]).unwrap(),
            client_identity(),
            collector::duplex_client::DuplexClientPolicy::DEFAULT,
        );
        let deadline = far_client_deadline();
        client.report_sink_failure(&report, deadline).unwrap();

        let bytes = outbound.borrow().clone();
        let sink_frame = crate::control_transport::decode_frame(
            &bytes,
            SessionId::from_bytes([0xE6; 16]).unwrap(),
            0,
        )
        .unwrap();
        assert_eq!(sink_frame.kind, MessageKind::SinkTerminalFailure);

        let worker_session = [0xE7; 16];
        let worker_sid = SessionId::from_bytes(worker_session).unwrap();
        let inbound = encode_frame(worker_sid, 0, sink_frame.kind, &sink_frame.payload).unwrap();
        let mut session = accept(prepared_with_inbound(worker_session, inbound));
        let err = session.receive_collector_hello(Instant::now()).unwrap_err();
        assert!(
            matches!(err, DuplexWorkerError::SinkFailure(_)),
            "got {err:?}"
        );
        assert_eq!(session.sink_failure(), Some(&report));
    }

    /// CAMPAIGN_IDENTITY loopback: the REAL client's `report_identity_failure`
    /// produces a CIF1 the REAL host decoder accepts, and the client surfaces
    /// the REAL host's CIA1 ack (both sides now share the one identity codec).
    #[test]
    fn real_client_identity_failure_is_decoded_and_acked_end_to_end() {
        use crate::duplex_session::identity_control::{
            decode_identity_failure, encode_identity_failure_ack, IdentityFailureAckDisposition,
            IdentityFailureAckV1, IdentityFailureReason, IdentityFailureV1,
        };

        let failure = IdentityFailureV1 {
            reason: IdentityFailureReason::UnreadableRoot,
            current_logical_id: Some(5),
            attempt_id: 77,
            attempted_hour: None,
            uuid_raw: None,
            seed_raw: None,
            profile_raw: None,
            first_read_complete: false,
            second_read_complete: false,
            reads_equal: false,
            last_admitted_sequence: Some(3),
            current_cid1_hash: Some([9; 32]),
        };

        // The REAL host durable-recorded ack it would return (as
        // dispatch_identity_failure_frame builds), scripted as the recorder's
        // sequence-0 inbound frame for the client to receive.
        let client_sid = SessionId::from_bytes([0xE3; 16]).unwrap();
        let ack = IdentityFailureAckV1 {
            attempt_id: failure.attempt_id,
            logical_id: failure.current_logical_id,
            disposition: IdentityFailureAckDisposition::DurablyRecorded,
            error: 0,
        };
        let ack_frame = encode_frame(
            client_sid,
            0,
            MessageKind::IdentityFailureAck,
            &encode_identity_failure_ack(ack).unwrap(),
        )
        .unwrap();

        let outbound = Rc::new(RefCell::new(Vec::new()));
        let inbound = Rc::new(RefCell::new(std::collections::VecDeque::from(ack_frame)));
        let mut client = collector::duplex_client::CollectorControlClient::new(
            &collector::duplex_client::test_duplex_client_permit(),
            ClientPairIo {
                outbound: Rc::clone(&outbound),
                inbound: Rc::clone(&inbound),
            },
            ClientZeroClock,
            compact_contract::control_transport::claim_session_capability([0xE3; 16]).unwrap(),
            client_identity(),
            collector::duplex_client::DuplexClientPolicy::DEFAULT,
        );
        let deadline = far_client_deadline();
        client
            .perform_handshake(
                &collector::duplex_client::test_duplex_client_permit(),
                deadline,
            )
            .unwrap();
        client.report_identity_failure(&failure, deadline).unwrap();

        // The client's CIF1 decodes through the REAL host codec, unchanged.
        let bytes = outbound.borrow().clone();
        let hello_wire = crate::control_transport::CONTROL_HEADER_LEN + 72;
        let cif1 =
            crate::control_transport::decode_frame(&bytes[hello_wire..], client_sid, 1).unwrap();
        assert_eq!(cif1.kind, MessageKind::IdentityFailure);
        assert_eq!(decode_identity_failure(&cif1.payload).unwrap(), failure);

        // The client surfaces the REAL host's CIA1 ack.
        let event = client.serve_next(deadline).unwrap();
        assert_eq!(
            event,
            collector::duplex_client::ClientEvent::IdentityFailureAcked(
                compact_contract::identity_control::IdentityFailureAckV1 {
                    attempt_id: 77,
                    logical_id: Some(5),
                    disposition:
                        compact_contract::identity_control::IdentityFailureAckDisposition::DurablyRecorded,
                    error: 0,
                }
            )
        );
    }
}
