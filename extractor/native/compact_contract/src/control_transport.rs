//! Fail-closed framing and byte-I/O seam for the recorder/collector control pipe.
//!
//! This module deliberately owns no pipe names, Windows handles, injection hooks,
//! reconnect policy, timer scheduling, or consumer state machines. Existing
//! offline wiring can supply [`ControlIo`]; live wiring must supply
//! [`DeadlineControlIo`], absolute deadlines, cancellation, and an unpredictable,
//! per-attach [`SessionCapability`].
//! Consequently, no decoded control message can call or request DLL injection.
//!
//! This codec is the single, shared H4CT v1 implementation consumed by both the
//! recorder host (`recorder_host::control_transport` re-exports it transparently)
//! and the injected collector DLL (`collector::duplex_client`). Any drift between
//! the two consumers is a shared-crate compile/test failure, not two diverging
//! local copies. The typed payload codecs live in the [`payloads`] submodule.

pub mod payloads;

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant};

pub const CONTROL_MAGIC: &[u8; 4] = b"H4CT";
pub const CONTROL_VERSION: u16 = 1;
pub const CONTROL_HEADER_LEN: usize = 48;
pub const CONTROL_MAX_PAYLOAD_LEN: usize = 1 << 20;

const CHECKSUM_OFFSET: usize = 40;
const RESERVED_OFFSET: usize = 44;

/// Unpredictable identifier minted once for a single attach attempt.
///
/// The transport does not generate this value: the product integration must use
/// the operating-system CSPRNG and must never reuse an id after disconnect.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionId([u8; 16]);

impl SessionId {
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, ControlError> {
        if bytes == [0; 16] {
            Err(ControlError::InvalidSessionId)
        } else {
            Ok(Self(bytes))
        }
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Move-only authority to construct one endpoint for a control session.
///
/// Claims are process-global and permanent: dropping a capability or its
/// channel never makes the session id claimable again. The two endpoints live
/// in separate processes, so each process independently claims the same
/// CSPRNG-minted id exactly once.
pub struct SessionCapability {
    session_id: SessionId,
}

/// Absolute monotonic deadline shared by every partial transfer in one control
/// operation.  A backend must not replace this with a fresh relative timeout.
#[derive(Clone, Copy, Debug)]
pub struct ControlDeadline(Instant);

impl ControlDeadline {
    pub fn at(instant: Instant) -> Self {
        Self(instant)
    }

    pub fn after(duration: Duration) -> Result<Self, ControlError> {
        Instant::now()
            .checked_add(duration)
            .map(Self)
            .ok_or(ControlError::DeadlineOverflow)
    }

    pub fn instant(self) -> Instant {
        self.0
    }

    pub fn is_expired(self) -> bool {
        Instant::now() >= self.0
    }

    pub fn remaining(self) -> Option<Duration> {
        self.0.checked_duration_since(Instant::now())
    }
}

/// Cooperative cancellation shared by the session driver and the exact
/// operation adapter.  A Windows backend is expected to translate this into
/// `CancelIoEx(handle, &overlapped)` and reap or quarantine that operation.
#[derive(Clone, Debug, Default)]
pub struct ControlCancellation(Arc<AtomicBool>);

impl ControlCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Closed capability set admitted for H4CT v1 consumers.  These bits are a
/// local handshake/admission fact; they do not change the v1 envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlCapabilities(u32);

impl ControlCapabilities {
    pub const HEARTBEAT: u32 = 1 << 0;
    pub const FIRST_COMPACT_ACK: u32 = 1 << 1;
    pub const ROLLOVER: u32 = 1 << 2;
    pub const STAGE_B: u32 = 1 << 3;
    pub const TRANSPORT_STATUS: u32 = 1 << 4;
    pub const CAMPAIGN_IDENTITY: u32 = 1 << 5;
    pub const EARLY_RENDER: u32 = 1 << 6;
    pub const CAPTURE_READY: u32 = 1 << 7;
    pub const KNOWN_MASK: u32 = Self::HEARTBEAT
        | Self::FIRST_COMPACT_ACK
        | Self::ROLLOVER
        | Self::STAGE_B
        | Self::TRANSPORT_STATUS
        | Self::CAMPAIGN_IDENTITY
        | Self::EARLY_RENDER
        | Self::CAPTURE_READY;

    pub fn from_bits(bits: u32) -> Result<Self, ControlError> {
        let unknown = bits & !Self::KNOWN_MASK;
        if unknown != 0 {
            return Err(ControlError::UnknownCapabilities(unknown));
        }
        Ok(Self(bits))
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn permits(self, kind: MessageKind) -> bool {
        let required = kind.required_capability_bits();
        self.0 & required == required
    }

    /// The closed capability word of the v1 auto-attach duplex product:
    /// heartbeat, first-compact-frame ack, transport status, and campaign
    /// identity. Defined once in the shared codec so the recorder host's
    /// admission requirement (`required_auto_attach_duplex_capabilities`) and
    /// the collector client's advertised hello word can never drift.
    pub const AUTO_ATTACH_DUPLEX_V1: u32 = Self::HEARTBEAT
        | Self::FIRST_COMPACT_ACK
        | Self::TRANSPORT_STATUS
        | Self::CAMPAIGN_IDENTITY
        | Self::CAPTURE_READY;

    pub fn auto_attach_duplex_v1() -> Self {
        Self::from_bits(Self::AUTO_ATTACH_DUPLEX_V1).expect("closed auto-attach capability set")
    }

    /// Dormant next-step capability word used only by rollover-focused offline
    /// composition. The active auto-attach client/host continue to advertise
    /// [`Self::AUTO_ATTACH_DUPLEX_V1`] until the live drain/durable-close/new-CID1
    /// lifecycle is proven.
    pub const AUTO_ATTACH_DUPLEX_ROLLOVER_V1: u32 = Self::AUTO_ATTACH_DUPLEX_V1 | Self::ROLLOVER;

    pub fn auto_attach_duplex_rollover_v1() -> Self {
        Self::from_bits(Self::AUTO_ATTACH_DUPLEX_ROLLOVER_V1)
            .expect("closed dormant rollover capability set")
    }

    pub const EARLY_RENDER_DUPLEX_V1: u32 = Self::AUTO_ATTACH_DUPLEX_V1 | Self::EARLY_RENDER;

    pub fn early_render_duplex_v1() -> Self {
        Self::from_bits(Self::EARLY_RENDER_DUPLEX_V1)
            .expect("closed early-render duplex capability set")
    }
}

/// Process fact returned by the live pipe peer-validation adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlPeerIdentity {
    pub pid: u32,
    pub creation_filetime: u64,
}

/// Move-only admission token proving that process identity and the closed
/// capability set were validated before channel construction.
pub struct ValidatedSessionCapability {
    session: SessionCapability,
    peer: ControlPeerIdentity,
    capabilities: ControlCapabilities,
}

impl ValidatedSessionCapability {
    pub fn admit(
        session: SessionCapability,
        expected_peer: ControlPeerIdentity,
        observed_peer: ControlPeerIdentity,
        required: ControlCapabilities,
        observed: ControlCapabilities,
    ) -> Result<Self, ControlError> {
        if expected_peer.pid == 0 || expected_peer.creation_filetime == 0 {
            return Err(ControlError::InvalidPeerIdentity);
        }
        if observed_peer.pid != expected_peer.pid {
            return Err(ControlError::PeerPidMismatch {
                expected: expected_peer.pid,
                actual: observed_peer.pid,
            });
        }
        if observed_peer.creation_filetime != expected_peer.creation_filetime {
            return Err(ControlError::PeerCreationTimeMismatch {
                expected: expected_peer.creation_filetime,
                actual: observed_peer.creation_filetime,
            });
        }
        if !observed.contains(required) {
            return Err(ControlError::MissingCapabilities {
                required: required.bits(),
                observed: observed.bits(),
            });
        }
        Ok(Self {
            session,
            peer: observed_peer,
            capabilities: observed,
        })
    }

    pub const fn peer(&self) -> ControlPeerIdentity {
        self.peer
    }

    pub const fn capabilities(&self) -> ControlCapabilities {
        self.capabilities
    }
}

impl SessionCapability {
    /// Permanently claims a CSPRNG-minted id for one channel in this process.
    pub fn claim(session_id: [u8; 16]) -> Result<Self, ControlError> {
        if session_id == [0; 16] {
            return Err(ControlError::InvalidSessionId);
        }

        let mut claimed = match claimed_session_ids().try_lock() {
            Ok(claimed) => claimed,
            Err(TryLockError::WouldBlock) => return Err(ControlError::SessionClaimRegistryBusy),
            Err(TryLockError::Poisoned(_)) => {
                return Err(ControlError::SessionClaimRegistryPoisoned)
            }
        };
        if !claimed.insert(session_id) {
            return Err(ControlError::SessionAlreadyClaimed(SessionId(session_id)));
        }

        Ok(Self {
            session_id: SessionId(session_id),
        })
    }

    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
}

/// Internal claim seam used by recorder-host composition.
///
/// Production preserves the public nonblocking `try_lock` behavior exactly.
/// Unit tests share one process-global registry while running concurrently, so
/// their composition paths receive a bounded retry instead of randomly
/// interpreting another test's short critical section as an attach failure.
// Widened from `pub(crate)` to `pub` when this codec moved into the shared
// `compact_contract` crate: the recorder host consumes it through the
// transparent re-export in `recorder_host::control_transport`, which a
// glob re-export cannot carry across the crate boundary at `pub(crate)`.
pub fn claim_session_capability(session_id: [u8; 16]) -> Result<SessionCapability, ControlError> {
    #[cfg(not(any(test, feature = "test-support")))]
    {
        SessionCapability::claim(session_id)
    }
    #[cfg(any(test, feature = "test-support"))]
    {
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(5))
            .ok_or(ControlError::DeadlineOverflow)?;
        loop {
            match SessionCapability::claim(session_id) {
                Err(ControlError::SessionClaimRegistryBusy) if Instant::now() < deadline => {
                    std::thread::yield_now();
                }
                result => return result,
            }
        }
    }
}

fn claimed_session_ids() -> &'static Mutex<BTreeSet<[u8; 16]>> {
    static CLAIMED: OnceLock<Mutex<BTreeSet<[u8; 16]>>> = OnceLock::new();
    CLAIMED.get_or_init(|| Mutex::new(BTreeSet::new()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointRole {
    Recorder,
    Collector,
}

impl EndpointRole {
    const fn opposite(self) -> Self {
        match self {
            Self::Recorder => Self::Collector,
            Self::Collector => Self::Recorder,
        }
    }
}

/// Closed v1 control-message registry. There is intentionally no Inject or
/// Attach variant; injection remains outside this transport's authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum MessageKind {
    RecorderHello = 1,
    CollectorHello = 2,
    HeartbeatChallenge = 3,
    HeartbeatProof = 4,
    FirstCompactFrameAck = 5,
    RolloverPropose = 6,
    RolloverAck = 7,
    EpochTokenOffer = 8,
    EpochTokenEcho = 9,
    DeclarationTableReady = 10,
    DeclarationTableAck = 11,
    EpochSealReady = 12,
    EpochSealAck = 13,
    SinkSetupFailure = 14,
    SinkTerminalFailure = 15,
    IdentityFailure = 16,
    IdentityFailureAck = 17,
    EarlyRenderReady = 18,
    EarlyRenderFirstGameD3d = 19,
    EarlyRenderCommitReceipt = 20,
    EarlyRenderLaunchBind = 21,
    EarlyRenderVfsIndexChunk = 22,
    EarlyRenderVfsIndexCommit = 23,
    EarlyRenderVfsPayloadRequest = 24,
    EarlyRenderVfsPayloadChunk = 25,
    EarlyRenderVfsPayloadCommit = 26,
    EarlyRenderContentManifestChunk = 27,
    EarlyRenderContentManifestCommit = 28,
    EarlyRenderRuntimeDlcRoster = 29,
    EarlyRenderDiagnosticLivenessRequest = 30,
    EarlyRenderDiagnosticLivenessResponse = 31,
    EarlyRenderDiagnosticEnd = 32,
    EarlyRenderG2CaptureArm = 33,
    EarlyRenderStartupFailure = 34,
    CaptureReady = 35,
    CaptureReadyAck = 36,
}

impl MessageKind {
    pub const fn sender(self) -> EndpointRole {
        match self {
            Self::RecorderHello
            | Self::HeartbeatChallenge
            | Self::FirstCompactFrameAck
            | Self::RolloverAck
            | Self::EpochTokenOffer
            | Self::DeclarationTableAck
            | Self::EpochSealAck
            | Self::IdentityFailureAck
            | Self::EarlyRenderCommitReceipt
            | Self::EarlyRenderLaunchBind
            | Self::EarlyRenderVfsIndexChunk
            | Self::EarlyRenderVfsIndexCommit
            | Self::EarlyRenderVfsPayloadChunk
            | Self::EarlyRenderVfsPayloadCommit
            | Self::EarlyRenderContentManifestChunk
            | Self::EarlyRenderContentManifestCommit
            | Self::EarlyRenderDiagnosticLivenessRequest
            | Self::EarlyRenderDiagnosticEnd
            | Self::EarlyRenderG2CaptureArm => EndpointRole::Recorder,
            Self::CollectorHello
            | Self::HeartbeatProof
            | Self::RolloverPropose
            | Self::EpochTokenEcho
            | Self::DeclarationTableReady
            | Self::EpochSealReady
            | Self::SinkSetupFailure
            | Self::SinkTerminalFailure
            | Self::IdentityFailure
            | Self::EarlyRenderReady
            | Self::EarlyRenderFirstGameD3d
            | Self::EarlyRenderRuntimeDlcRoster
            | Self::EarlyRenderDiagnosticLivenessResponse
            | Self::EarlyRenderVfsPayloadRequest
            | Self::EarlyRenderStartupFailure
            | Self::CaptureReady => EndpointRole::Collector,
            Self::CaptureReadyAck => EndpointRole::Recorder,
        }
    }

    /// Capability required before this message may cross the typed transport
    /// boundary. Hello is the admission exchange itself; every consumer kind
    /// is closed over exactly one capability.
    pub const fn required_capability_bits(self) -> u32 {
        match self {
            Self::RecorderHello | Self::CollectorHello => 0,
            Self::HeartbeatChallenge | Self::HeartbeatProof => ControlCapabilities::HEARTBEAT,
            Self::FirstCompactFrameAck => ControlCapabilities::FIRST_COMPACT_ACK,
            Self::RolloverPropose | Self::RolloverAck => ControlCapabilities::ROLLOVER,
            Self::EpochTokenOffer
            | Self::EpochTokenEcho
            | Self::DeclarationTableReady
            | Self::DeclarationTableAck
            | Self::EpochSealReady
            | Self::EpochSealAck => ControlCapabilities::STAGE_B,
            Self::SinkSetupFailure | Self::SinkTerminalFailure => {
                ControlCapabilities::TRANSPORT_STATUS
            }
            Self::IdentityFailure | Self::IdentityFailureAck => {
                ControlCapabilities::CAMPAIGN_IDENTITY
            }
            Self::EarlyRenderReady
            | Self::EarlyRenderFirstGameD3d
            | Self::EarlyRenderCommitReceipt
            | Self::EarlyRenderLaunchBind
            | Self::EarlyRenderVfsIndexChunk
            | Self::EarlyRenderVfsIndexCommit
            | Self::EarlyRenderVfsPayloadRequest
            | Self::EarlyRenderVfsPayloadChunk
            | Self::EarlyRenderVfsPayloadCommit
            | Self::EarlyRenderContentManifestChunk
            | Self::EarlyRenderContentManifestCommit
            | Self::EarlyRenderRuntimeDlcRoster
            | Self::EarlyRenderDiagnosticLivenessRequest
            | Self::EarlyRenderDiagnosticLivenessResponse
            | Self::EarlyRenderDiagnosticEnd
            | Self::EarlyRenderG2CaptureArm
            | Self::EarlyRenderStartupFailure => ControlCapabilities::EARLY_RENDER,
            Self::CaptureReady | Self::CaptureReadyAck => ControlCapabilities::CAPTURE_READY,
        }
    }

    fn from_wire(value: u16) -> Result<Self, ControlError> {
        match value {
            1 => Ok(Self::RecorderHello),
            2 => Ok(Self::CollectorHello),
            3 => Ok(Self::HeartbeatChallenge),
            4 => Ok(Self::HeartbeatProof),
            5 => Ok(Self::FirstCompactFrameAck),
            6 => Ok(Self::RolloverPropose),
            7 => Ok(Self::RolloverAck),
            8 => Ok(Self::EpochTokenOffer),
            9 => Ok(Self::EpochTokenEcho),
            10 => Ok(Self::DeclarationTableReady),
            11 => Ok(Self::DeclarationTableAck),
            12 => Ok(Self::EpochSealReady),
            13 => Ok(Self::EpochSealAck),
            14 => Ok(Self::SinkSetupFailure),
            15 => Ok(Self::SinkTerminalFailure),
            16 => Ok(Self::IdentityFailure),
            17 => Ok(Self::IdentityFailureAck),
            18 => Ok(Self::EarlyRenderReady),
            19 => Ok(Self::EarlyRenderFirstGameD3d),
            20 => Ok(Self::EarlyRenderCommitReceipt),
            21 => Ok(Self::EarlyRenderLaunchBind),
            22 => Ok(Self::EarlyRenderVfsIndexChunk),
            23 => Ok(Self::EarlyRenderVfsIndexCommit),
            24 => Ok(Self::EarlyRenderVfsPayloadRequest),
            25 => Ok(Self::EarlyRenderVfsPayloadChunk),
            26 => Ok(Self::EarlyRenderVfsPayloadCommit),
            27 => Ok(Self::EarlyRenderContentManifestChunk),
            28 => Ok(Self::EarlyRenderContentManifestCommit),
            29 => Ok(Self::EarlyRenderRuntimeDlcRoster),
            30 => Ok(Self::EarlyRenderDiagnosticLivenessRequest),
            31 => Ok(Self::EarlyRenderDiagnosticLivenessResponse),
            32 => Ok(Self::EarlyRenderDiagnosticEnd),
            33 => Ok(Self::EarlyRenderG2CaptureArm),
            34 => Ok(Self::EarlyRenderStartupFailure),
            35 => Ok(Self::CaptureReady),
            36 => Ok(Self::CaptureReadyAck),
            other => Err(ControlError::UnknownMessageKind(other)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlFrame {
    pub session_id: SessionId,
    pub sequence: u64,
    pub kind: MessageKind,
    pub payload: Vec<u8>,
}

/// Minimal seam implemented later by a Windows named-pipe adapter and by tests.
///
/// `send_all` must either transfer the complete slice or return an error. A
/// partial transfer followed by an error is still an error: [`ControlChannel`]
/// poisons itself and permits no retry under the same session id.
pub trait ControlIo {
    fn send_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn recv_exact(&mut self, bytes: &mut [u8]) -> io::Result<()>;
}

/// Production-facing byte seam.  Implementations must use the same absolute
/// deadline for every partial transfer and must give cancellation ownership of
/// the exact in-flight operation.  Returning `Ok` after either condition has
/// become observable is still rejected by [`DeadlineControlChannel`].
pub trait DeadlineControlIo {
    fn send_all_until(
        &mut self,
        bytes: &[u8],
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> io::Result<()>;

    fn recv_exact_until(
        &mut self,
        bytes: &mut [u8],
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> io::Result<()>;
}

/// Read-only readiness seam for transports that can inspect their receive
/// queue without consuming framing bytes.  Startup arbitration uses this to
/// choose between H4CR and H4CT before beginning either exact read; it must not
/// advance the transport, alter sequence state, or manufacture readiness.
pub trait DeadlineControlInputReadiness {
    fn input_bytes_available(&self) -> io::Result<usize>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlError {
    Poisoned,
    Io(io::ErrorKind),
    Truncated,
    BadMagic,
    UnsupportedVersion(u16),
    UnsupportedHeaderLen(u16),
    UnknownMessageKind(u16),
    NonZeroFlags(u16),
    PayloadTooLarge(u32),
    InvalidSessionId,
    SessionAlreadyClaimed(SessionId),
    SessionClaimRegistryBusy,
    SessionClaimRegistryPoisoned,
    DeadlineOverflow,
    DeadlineExpired,
    Cancelled,
    InvalidPeerIdentity,
    PeerPidMismatch {
        expected: u32,
        actual: u32,
    },
    PeerCreationTimeMismatch {
        expected: u64,
        actual: u64,
    },
    UnknownCapabilities(u32),
    MissingCapabilities {
        required: u32,
        observed: u32,
    },
    UnexpectedCapabilities {
        required: u32,
        observed: u32,
    },
    InvalidCapabilityAdmissionState,
    CapabilityDenied {
        kind: MessageKind,
        required: u32,
        admitted: u32,
    },
    NonZeroReserved(u32),
    SessionMismatch {
        expected: SessionId,
        actual: SessionId,
    },
    SequenceMismatch {
        expected: u64,
        actual: u64,
    },
    SequenceExhausted,
    WrongDirection {
        role: EndpointRole,
        kind: MessageKind,
    },
    ChecksumMismatch {
        expected: u32,
        actual: u32,
    },
    FrameLengthMismatch {
        expected: usize,
        actual: usize,
    },
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Poisoned => write!(f, "control channel is poisoned"),
            Self::Io(kind) => write!(f, "control I/O failed: {kind:?}"),
            Self::Truncated => write!(f, "truncated control frame"),
            Self::BadMagic => write!(f, "invalid control magic"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported control version {v}"),
            Self::UnsupportedHeaderLen(len) => {
                write!(f, "unsupported control header length {len}")
            }
            Self::UnknownMessageKind(kind) => write!(f, "unknown control message kind {kind}"),
            Self::NonZeroFlags(flags) => write!(f, "unsupported control flags {flags:#06x}"),
            Self::PayloadTooLarge(len) => write!(
                f,
                "control payload length {len} exceeds {CONTROL_MAX_PAYLOAD_LEN}"
            ),
            Self::InvalidSessionId => write!(f, "all-zero control session id is invalid"),
            Self::SessionAlreadyClaimed(session_id) => {
                write!(f, "control session id {session_id:?} was already claimed")
            }
            Self::SessionClaimRegistryBusy => {
                write!(f, "control session claim registry is busy")
            }
            Self::SessionClaimRegistryPoisoned => {
                write!(f, "control session claim registry is poisoned")
            }
            Self::DeadlineOverflow => write!(f, "control deadline overflow"),
            Self::DeadlineExpired => write!(f, "control operation deadline expired"),
            Self::Cancelled => write!(f, "control operation was cancelled"),
            Self::InvalidPeerIdentity => write!(f, "invalid expected control peer identity"),
            Self::PeerPidMismatch { expected, actual } => {
                write!(f, "control peer PID mismatch: {actual} != {expected}")
            }
            Self::PeerCreationTimeMismatch { expected, actual } => write!(
                f,
                "control peer creation FILETIME mismatch: {actual} != {expected}"
            ),
            Self::UnknownCapabilities(bits) => {
                write!(f, "unknown control capabilities {bits:#010x}")
            }
            Self::MissingCapabilities { required, observed } => write!(
                f,
                "missing control capabilities: required {required:#010x}, observed {observed:#010x}"
            ),
            Self::UnexpectedCapabilities { required, observed } => write!(
                f,
                "control hello capabilities must equal {required:#010x}, observed {observed:#010x}"
            ),
            Self::InvalidCapabilityAdmissionState => write!(
                f,
                "decoded hello capabilities were admitted outside the first-hello boundary"
            ),
            Self::CapabilityDenied {
                kind,
                required,
                admitted,
            } => write!(
                f,
                "control message {kind:?} requires capability {required:#010x}, admitted {admitted:#010x}"
            ),
            Self::NonZeroReserved(value) => {
                write!(f, "nonzero reserved control field {value:#010x}")
            }
            Self::SessionMismatch { expected, actual } => {
                write!(f, "control session mismatch: {actual:?} != {expected:?}")
            }
            Self::SequenceMismatch { expected, actual } => {
                write!(f, "control sequence mismatch: {actual} != {expected}")
            }
            Self::SequenceExhausted => write!(f, "control sequence exhausted"),
            Self::WrongDirection { role, kind } => {
                write!(
                    f,
                    "{role:?} cannot send or receive {kind:?} in this direction"
                )
            }
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "control checksum mismatch: {actual:08x} != {expected:08x}"
            ),
            Self::FrameLengthMismatch { expected, actual } => {
                write!(f, "control frame length {actual} != expected {expected}")
            }
        }
    }
}

impl std::error::Error for ControlError {}

/// Stateful, strictly ordered endpoint for one control-pipe connection.
pub struct ControlChannel<I> {
    io: I,
    role: EndpointRole,
    session_id: SessionId,
    next_send_sequence: u64,
    next_recv_sequence: u64,
    poisoned: bool,
}

impl<I: ControlIo> ControlChannel<I> {
    pub fn new(io: I, role: EndpointRole, session: SessionCapability) -> Self {
        Self {
            io,
            role,
            session_id: session.session_id,
            next_send_sequence: 0,
            next_recv_sequence: 0,
            poisoned: false,
        }
    }

    pub fn send(&mut self, kind: MessageKind, payload: &[u8]) -> Result<(), ControlError> {
        self.ensure_usable()?;
        if kind.sender() != self.role {
            return self.fail(ControlError::WrongDirection {
                role: self.role,
                kind,
            });
        }
        if self.next_send_sequence == u64::MAX {
            return self.fail(ControlError::SequenceExhausted);
        }
        let frame = match encode_frame(self.session_id, self.next_send_sequence, kind, payload) {
            Ok(frame) => frame,
            Err(err) => return self.fail(err),
        };
        if let Err(err) = self.io.send_all(&frame) {
            return self.fail(map_io_error(err));
        }
        self.next_send_sequence += 1;
        Ok(())
    }

    pub fn recv(&mut self) -> Result<ControlFrame, ControlError> {
        self.ensure_usable()?;
        if self.next_recv_sequence == u64::MAX {
            return self.fail(ControlError::SequenceExhausted);
        }

        let mut header = [0u8; CONTROL_HEADER_LEN];
        if let Err(err) = self.io.recv_exact(&mut header) {
            return self.fail(map_io_error(err));
        }
        let payload_len = match parse_header(&header) {
            Ok(parsed) => parsed.payload_len,
            Err(err) => return self.fail(err),
        };
        let mut wire = Vec::with_capacity(CONTROL_HEADER_LEN + payload_len);
        wire.extend_from_slice(&header);
        wire.resize(CONTROL_HEADER_LEN + payload_len, 0);
        if let Err(err) = self.io.recv_exact(&mut wire[CONTROL_HEADER_LEN..]) {
            return self.fail(map_io_error(err));
        }
        let frame = match decode_frame(&wire, self.session_id, self.next_recv_sequence) {
            Ok(frame) => frame,
            Err(err) => return self.fail(err),
        };
        if frame.kind.sender() != self.role.opposite() {
            return self.fail(ControlError::WrongDirection {
                role: self.role,
                kind: frame.kind,
            });
        }
        self.next_recv_sequence += 1;
        Ok(frame)
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Immutable accessor for the underlying byte transport.
    ///
    /// The channel owns its `ControlIo`, so a test that needs to inspect
    /// transport-local state (the collector client reads its deadline bridge;
    /// recorder-host tests read the fake pipe's written bytes) reaches it here.
    /// Read-only: it cannot mutate framing, sequence, or poison state. Gated
    /// behind `test-support` so it never widens the production API surface.
    #[cfg(any(test, feature = "test-support"))]
    pub fn io(&self) -> &I {
        &self.io
    }

    fn ensure_usable(&self) -> Result<(), ControlError> {
        if self.poisoned {
            Err(ControlError::Poisoned)
        } else {
            Ok(())
        }
    }

    fn fail<T>(&mut self, error: ControlError) -> Result<T, ControlError> {
        self.poisoned = true;
        Err(error)
    }
}

/// Deadline-bearing H4CT endpoint for live adapters. Construction requires a
/// move-only peer-admission token, so a raw pipe connection cannot dispatch a
/// control frame before PID, creation time, and capabilities are checked.
pub struct DeadlineControlChannel<I> {
    io: I,
    role: EndpointRole,
    session_id: SessionId,
    peer: ControlPeerIdentity,
    capabilities: ControlCapabilities,
    next_send_sequence: u64,
    next_recv_sequence: u64,
    poisoned: bool,
    hello_capabilities_admitted: bool,
}

impl<I: DeadlineControlIo> DeadlineControlChannel<I> {
    pub fn new(io: I, role: EndpointRole, admission: ValidatedSessionCapability) -> Self {
        Self {
            io,
            role,
            session_id: admission.session.session_id,
            peer: admission.peer,
            capabilities: admission.capabilities,
            next_send_sequence: 0,
            next_recv_sequence: 0,
            poisoned: false,
            hello_capabilities_admitted: admission.capabilities.bits() != 0,
        }
    }

    pub fn peer(&self) -> ControlPeerIdentity {
        self.peer
    }

    pub fn capabilities(&self) -> ControlCapabilities {
        self.capabilities
    }

    /// Inspect queued receive bytes without consuming a partial control frame.
    /// The same fixed deadline and cancellation authority used by the eventual
    /// exact receive also bound this readiness probe. Any transport failure is
    /// terminal for the channel.
    pub fn input_bytes_available_until(
        &mut self,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<usize, ControlError>
    where
        I: DeadlineControlInputReadiness,
    {
        self.preflight(deadline, cancellation)?;
        let available = match self.io.input_bytes_available() {
            Ok(available) => available,
            Err(error) => return self.fail(map_io_error(error)),
        };
        self.preflight(deadline, cancellation)?;
        Ok(available)
    }

    /// Complete the capability half of peer admission from a decoded hello.
    ///
    /// The live H4CT accept path can prove PID and creation time before it reads
    /// any bytes, but the collector's capability claim is inside its first
    /// typed hello.  Such a channel is constructed with an empty capability
    /// set, may receive only `CollectorHello`, and calls this method exactly
    /// once after the hello codec has validated the complete payload.  No raw
    /// or caller-invented capability tuple can be substituted later.
    pub fn admit_decoded_hello_capabilities(
        &mut self,
        required: ControlCapabilities,
        observed: ControlCapabilities,
    ) -> Result<(), ControlError> {
        if self.poisoned {
            return Err(ControlError::Poisoned);
        }
        if self.hello_capabilities_admitted
            || self.capabilities.bits() != 0
            || self.next_recv_sequence != 1
            || self.next_send_sequence != 0
        {
            return self.fail(ControlError::InvalidCapabilityAdmissionState);
        }
        if observed.bits() != required.bits() {
            return self.fail(ControlError::UnexpectedCapabilities {
                required: required.bits(),
                observed: observed.bits(),
            });
        }
        self.capabilities = required;
        self.hello_capabilities_admitted = true;
        Ok(())
    }

    pub fn send_until(
        &mut self,
        kind: MessageKind,
        payload: &[u8],
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), ControlError> {
        self.preflight(deadline, cancellation)?;
        self.assert_capability(kind)?;
        if kind.sender() != self.role {
            return self.fail(ControlError::WrongDirection {
                role: self.role,
                kind,
            });
        }
        if self.next_send_sequence == u64::MAX {
            return self.fail(ControlError::SequenceExhausted);
        }
        let frame = match encode_frame(self.session_id, self.next_send_sequence, kind, payload) {
            Ok(frame) => frame,
            Err(error) => return self.fail(error),
        };
        let result = self.io.send_all_until(&frame, deadline, cancellation);
        self.finish_io(result, deadline, cancellation)?;
        self.next_send_sequence += 1;
        Ok(())
    }

    pub fn recv_until(
        &mut self,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<ControlFrame, ControlError> {
        self.recv_frame_until(deadline, cancellation, false)
    }

    /// Receive the collector's first handshake frame, permitting exactly the two
    /// authoritative sink-failure kinds in addition to the capability-free
    /// `CollectorHello`.
    ///
    /// A collector whose sink setup fails *before* it can answer with a hello
    /// legitimately sends `SinkSetupFailure`/`SinkTerminalFailure` as its very
    /// first control frame. Those two kinds require `TRANSPORT_STATUS`, which is
    /// not admitted until after a valid hello, so the ordinary [`Self::recv_until`]
    /// capability gate would poison the channel and discard the structured
    /// failure report (a capability catch-22). This method opens a closed
    /// pre-hello window for exactly those two kinds and only while the channel is
    /// still in its empty-capability handshake state. Every other
    /// capability-gated kind is still denied and permanently poisons, and the
    /// window closes the instant any capability is admitted. The handshake still
    /// fails terminally on such a frame; the fix is that the failure is
    /// *structured and surfaced* rather than lost.
    pub fn recv_handshake_until(
        &mut self,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<ControlFrame, ControlError> {
        self.recv_frame_until(deadline, cancellation, true)
    }

    fn recv_frame_until(
        &mut self,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
        allow_pre_hello_sink_failure: bool,
    ) -> Result<ControlFrame, ControlError> {
        self.preflight(deadline, cancellation)?;
        if self.next_recv_sequence == u64::MAX {
            return self.fail(ControlError::SequenceExhausted);
        }

        let mut header = [0u8; CONTROL_HEADER_LEN];
        let result = self
            .io
            .recv_exact_until(&mut header, deadline, cancellation);
        self.finish_io(result, deadline, cancellation)?;
        let payload_len = match parse_header(&header) {
            Ok(parsed) => parsed.payload_len,
            Err(error) => return self.fail(error),
        };
        let mut wire = Vec::with_capacity(CONTROL_HEADER_LEN + payload_len);
        wire.extend_from_slice(&header);
        wire.resize(CONTROL_HEADER_LEN + payload_len, 0);
        let result =
            self.io
                .recv_exact_until(&mut wire[CONTROL_HEADER_LEN..], deadline, cancellation);
        self.finish_io(result, deadline, cancellation)?;
        let frame = match decode_frame(&wire, self.session_id, self.next_recv_sequence) {
            Ok(frame) => frame,
            Err(error) => return self.fail(error),
        };
        if !(allow_pre_hello_sink_failure && self.is_pre_hello_receivable_sink_failure(frame.kind))
        {
            self.assert_capability(frame.kind)?;
        }
        if frame.kind.sender() != self.role.opposite() {
            return self.fail(ControlError::WrongDirection {
                role: self.role,
                kind: frame.kind,
            });
        }
        self.next_recv_sequence += 1;
        Ok(frame)
    }

    /// The closed set of kinds receivable before any capability is admitted:
    /// exactly the two authoritative sink-failure kinds, and only while the
    /// channel is still in its empty-capability pre-hello handshake window.
    fn is_pre_hello_receivable_sink_failure(&self, kind: MessageKind) -> bool {
        !self.hello_capabilities_admitted
            && self.capabilities.bits() == 0
            && matches!(
                kind,
                MessageKind::SinkSetupFailure | MessageKind::SinkTerminalFailure
            )
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Consumer-level codec/state failures share the same terminal session
    /// result as transport failures. There is no transparent reconnect.
    pub fn poison(&mut self) {
        self.poisoned = true;
    }

    /// Assert the closed message-to-capability mapping before state-machine
    /// mutation or wire dispatch. Failure permanently poisons the session.
    pub fn assert_capability(&mut self, kind: MessageKind) -> Result<(), ControlError> {
        let required = kind.required_capability_bits();
        if self.capabilities.permits(kind) {
            Ok(())
        } else {
            self.fail(ControlError::CapabilityDenied {
                kind,
                required,
                admitted: self.capabilities.bits(),
            })
        }
    }

    /// Immutable accessor for the underlying deadline-bearing transport; see
    /// the matching note on `ControlChannel::io`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn io(&self) -> &I {
        &self.io
    }

    fn preflight(
        &mut self,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), ControlError> {
        if self.poisoned {
            return Err(ControlError::Poisoned);
        }
        if cancellation.is_cancelled() {
            return self.fail(ControlError::Cancelled);
        }
        if deadline.is_expired() {
            return self.fail(ControlError::DeadlineExpired);
        }
        Ok(())
    }

    fn finish_io(
        &mut self,
        result: io::Result<()>,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), ControlError> {
        if cancellation.is_cancelled() {
            return self.fail(ControlError::Cancelled);
        }
        if deadline.is_expired() {
            return self.fail(ControlError::DeadlineExpired);
        }
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                self.fail(ControlError::DeadlineExpired)
            }
            Err(error) => self.fail(map_io_error(error)),
        }
    }

    fn fail<T>(&mut self, error: ControlError) -> Result<T, ControlError> {
        self.poisoned = true;
        Err(error)
    }
}

struct ParsedHeader {
    kind: MessageKind,
    payload_len: usize,
    session_id: SessionId,
    sequence: u64,
    checksum: u32,
}

pub fn encode_frame(
    session_id: SessionId,
    sequence: u64,
    kind: MessageKind,
    payload: &[u8],
) -> Result<Vec<u8>, ControlError> {
    if session_id.0 == [0; 16] {
        return Err(ControlError::InvalidSessionId);
    }
    if sequence == u64::MAX {
        return Err(ControlError::SequenceExhausted);
    }
    if payload.len() > CONTROL_MAX_PAYLOAD_LEN {
        return Err(ControlError::PayloadTooLarge(
            payload.len().min(u32::MAX as usize) as u32,
        ));
    }

    let mut wire = vec![0u8; CONTROL_HEADER_LEN + payload.len()];
    wire[0..4].copy_from_slice(CONTROL_MAGIC);
    wire[4..6].copy_from_slice(&CONTROL_VERSION.to_le_bytes());
    wire[6..8].copy_from_slice(&(CONTROL_HEADER_LEN as u16).to_le_bytes());
    wire[8..10].copy_from_slice(&(kind as u16).to_le_bytes());
    // bytes 10..12 are flags; v1 requires zero.
    wire[12..16].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    wire[16..32].copy_from_slice(&session_id.0);
    wire[32..40].copy_from_slice(&sequence.to_le_bytes());
    // bytes 40..44 are zero while calculating the checksum.
    // bytes 44..48 are reserved and zero in v1.
    wire[CONTROL_HEADER_LEN..].copy_from_slice(payload);
    let checksum = crc32(&wire);
    wire[CHECKSUM_OFFSET..RESERVED_OFFSET].copy_from_slice(&checksum.to_le_bytes());
    Ok(wire)
}

pub fn decode_frame(
    wire: &[u8],
    expected_session_id: SessionId,
    expected_sequence: u64,
) -> Result<ControlFrame, ControlError> {
    if expected_session_id.0 == [0; 16] {
        return Err(ControlError::InvalidSessionId);
    }
    if wire.len() < CONTROL_HEADER_LEN {
        return Err(ControlError::Truncated);
    }
    let parsed = parse_header(&wire[..CONTROL_HEADER_LEN])?;
    let expected_len = CONTROL_HEADER_LEN + parsed.payload_len;
    if wire.len() != expected_len {
        return Err(ControlError::FrameLengthMismatch {
            expected: expected_len,
            actual: wire.len(),
        });
    }
    if parsed.session_id != expected_session_id {
        return Err(ControlError::SessionMismatch {
            expected: expected_session_id,
            actual: parsed.session_id,
        });
    }
    if parsed.sequence != expected_sequence {
        return Err(ControlError::SequenceMismatch {
            expected: expected_sequence,
            actual: parsed.sequence,
        });
    }

    let mut checksummed = wire.to_vec();
    checksummed[CHECKSUM_OFFSET..RESERVED_OFFSET].fill(0);
    let actual = crc32(&checksummed);
    if actual != parsed.checksum {
        return Err(ControlError::ChecksumMismatch {
            expected: parsed.checksum,
            actual,
        });
    }

    Ok(ControlFrame {
        session_id: parsed.session_id,
        sequence: parsed.sequence,
        kind: parsed.kind,
        payload: wire[CONTROL_HEADER_LEN..].to_vec(),
    })
}

fn parse_header(header: &[u8]) -> Result<ParsedHeader, ControlError> {
    if header.len() < CONTROL_HEADER_LEN {
        return Err(ControlError::Truncated);
    }
    if &header[0..4] != CONTROL_MAGIC {
        return Err(ControlError::BadMagic);
    }
    let version = read_u16(header, 4);
    if version != CONTROL_VERSION {
        return Err(ControlError::UnsupportedVersion(version));
    }
    let header_len = read_u16(header, 6);
    if header_len as usize != CONTROL_HEADER_LEN {
        return Err(ControlError::UnsupportedHeaderLen(header_len));
    }
    let kind = MessageKind::from_wire(read_u16(header, 8))?;
    let flags = read_u16(header, 10);
    if flags != 0 {
        return Err(ControlError::NonZeroFlags(flags));
    }
    let payload_len = read_u32(header, 12);
    if payload_len as usize > CONTROL_MAX_PAYLOAD_LEN {
        return Err(ControlError::PayloadTooLarge(payload_len));
    }
    let mut session_id = [0u8; 16];
    session_id.copy_from_slice(&header[16..32]);
    if session_id == [0; 16] {
        return Err(ControlError::InvalidSessionId);
    }
    let sequence = read_u64(header, 32);
    if sequence == u64::MAX {
        return Err(ControlError::SequenceExhausted);
    }
    let reserved = read_u32(header, RESERVED_OFFSET);
    if reserved != 0 {
        return Err(ControlError::NonZeroReserved(reserved));
    }
    Ok(ParsedHeader {
        kind,
        payload_len: payload_len as usize,
        session_id: SessionId(session_id),
        sequence,
        checksum: read_u32(header, CHECKSUM_OFFSET),
    })
}

fn map_io_error(error: io::Error) -> ControlError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        ControlError::Truncated
    } else {
        ControlError::Io(error.kind())
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

/// IEEE CRC-32, used as an integrity checksum rather than an authenticator.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Default)]
    struct MemoryIo {
        inbound: VecDeque<u8>,
        outbound: Vec<u8>,
        fail_send: bool,
    }

    impl MemoryIo {
        fn with_inbound(bytes: Vec<u8>) -> Self {
            Self {
                inbound: bytes.into(),
                ..Self::default()
            }
        }
    }

    impl ControlIo for MemoryIo {
        fn send_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            if self.fail_send {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "fake disconnect"));
            }
            self.outbound.extend_from_slice(bytes);
            Ok(())
        }

        fn recv_exact(&mut self, bytes: &mut [u8]) -> io::Result<()> {
            for slot in bytes {
                *slot = self.inbound.pop_front().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "fake truncated input")
                })?;
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct DeadlineMemoryIo {
        inbound: VecDeque<u8>,
        outbound: Vec<u8>,
        cancel_during_send: Option<ControlCancellation>,
        delay_during_send: Option<Duration>,
    }

    impl DeadlineControlIo for DeadlineMemoryIo {
        fn send_all_until(
            &mut self,
            bytes: &[u8],
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            if let Some(delay) = self.delay_during_send {
                std::thread::sleep(delay);
            }
            self.outbound.extend_from_slice(bytes);
            if let Some(cancellation) = &self.cancel_during_send {
                cancellation.cancel();
            }
            Ok(())
        }

        fn recv_exact_until(
            &mut self,
            bytes: &mut [u8],
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            for slot in bytes {
                *slot = self.inbound.pop_front().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "fake truncated input")
                })?;
            }
            Ok(())
        }
    }

    const SESSION: SessionId = SessionId([0x5a; 16]);

    fn fresh_session_bytes() -> [u8; 16] {
        static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
        let mut bytes = [0x5a; 16];
        bytes[8..].copy_from_slice(&NEXT_SESSION.fetch_add(1, Ordering::Relaxed).to_le_bytes());
        bytes
    }

    fn fresh_session() -> (SessionCapability, SessionId) {
        let capability = claim_session_for_test(fresh_session_bytes()).unwrap();
        let session_id = capability.session_id();
        (capability, session_id)
    }

    fn claim_session_for_test(bytes: [u8; 16]) -> Result<SessionCapability, ControlError> {
        claim_session_capability(bytes)
    }

    fn admitted_session(
        required_bits: u32,
        observed_bits: u32,
    ) -> (ValidatedSessionCapability, SessionId) {
        let (session, session_id) = fresh_session();
        let identity = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9_001,
        };
        let admission = ValidatedSessionCapability::admit(
            session,
            identity,
            identity,
            ControlCapabilities::from_bits(required_bits).unwrap(),
            ControlCapabilities::from_bits(observed_bits).unwrap(),
        )
        .unwrap();
        (admission, session_id)
    }

    #[test]
    fn frame_round_trip_preserves_every_field() {
        let wire = encode_frame(SESSION, 7, MessageKind::HeartbeatProof, b"proof").unwrap();
        let frame = decode_frame(&wire, SESSION, 7).unwrap();
        assert_eq!(frame.session_id, SESSION);
        assert_eq!(frame.sequence, 7);
        assert_eq!(frame.kind, MessageKind::HeartbeatProof);
        assert_eq!(frame.payload, b"proof");
    }

    #[test]
    fn ieee_crc32_matches_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn checksum_rejects_header_and_payload_corruption() {
        let original = encode_frame(SESSION, 0, MessageKind::CollectorHello, b"collector").unwrap();
        for offset in [16, CONTROL_HEADER_LEN] {
            let mut corrupt = original.clone();
            corrupt[offset] ^= 0x80;
            let error = decode_frame(&corrupt, SESSION, 0).unwrap_err();
            assert!(
                matches!(
                    error,
                    ControlError::SessionMismatch { .. } | ControlError::ChecksumMismatch { .. }
                ),
                "unexpected corruption error: {error}"
            );
        }
    }

    #[test]
    fn truncation_at_header_and_payload_fails_closed_and_poisons() {
        for truncate_header in [true, false] {
            let (session, session_id) = fresh_session();
            let wire =
                encode_frame(session_id, 0, MessageKind::CollectorHello, b"collector").unwrap();
            let truncated_len = if truncate_header {
                CONTROL_HEADER_LEN - 1
            } else {
                wire.len() - 1
            };
            let mut channel = ControlChannel::new(
                MemoryIo::with_inbound(wire[..truncated_len].to_vec()),
                EndpointRole::Recorder,
                session,
            );
            assert!(matches!(channel.recv(), Err(ControlError::Truncated)));
            assert!(channel.is_poisoned());
            assert!(matches!(channel.recv(), Err(ControlError::Poisoned)));
        }
    }

    #[test]
    fn duplicate_sequence_is_rejected_and_channel_stays_poisoned() {
        let (session, session_id) = fresh_session();
        let first = encode_frame(session_id, 0, MessageKind::CollectorHello, b"").unwrap();
        let mut input = first.clone();
        input.extend_from_slice(&first);
        let mut channel = ControlChannel::new(
            MemoryIo::with_inbound(input),
            EndpointRole::Recorder,
            session,
        );
        assert_eq!(channel.recv().unwrap().sequence, 0);
        assert!(matches!(
            channel.recv(),
            Err(ControlError::SequenceMismatch {
                expected: 1,
                actual: 0
            })
        ));
        assert!(matches!(channel.recv(), Err(ControlError::Poisoned)));
    }

    #[test]
    fn reordered_or_gapped_sequence_is_rejected() {
        let (session, session_id) = fresh_session();
        let wire = encode_frame(session_id, 1, MessageKind::CollectorHello, b"").unwrap();
        let mut channel = ControlChannel::new(
            MemoryIo::with_inbound(wire),
            EndpointRole::Recorder,
            session,
        );
        assert!(matches!(
            channel.recv(),
            Err(ControlError::SequenceMismatch {
                expected: 0,
                actual: 1
            })
        ));
        assert!(channel.is_poisoned());
    }

    #[test]
    fn stale_session_replay_is_rejected() {
        let stale =
            encode_frame(SessionId([0x11; 16]), 0, MessageKind::CollectorHello, b"").unwrap();
        assert!(matches!(
            decode_frame(&stale, SESSION, 0),
            Err(ControlError::SessionMismatch { .. })
        ));
    }

    #[test]
    fn role_rejects_messages_from_the_wrong_direction() {
        let (sender_session, _) = fresh_session();
        let mut sender =
            ControlChannel::new(MemoryIo::default(), EndpointRole::Recorder, sender_session);
        assert!(matches!(
            sender.send(MessageKind::CollectorHello, b""),
            Err(ControlError::WrongDirection { .. })
        ));
        assert!(sender.is_poisoned());

        let (receiver_session, receiver_session_id) = fresh_session();
        let wire = encode_frame(receiver_session_id, 0, MessageKind::RecorderHello, b"").unwrap();
        let mut receiver = ControlChannel::new(
            MemoryIo::with_inbound(wire),
            EndpointRole::Recorder,
            receiver_session,
        );
        assert!(matches!(
            receiver.recv(),
            Err(ControlError::WrongDirection { .. })
        ));
        assert!(receiver.is_poisoned());
    }

    #[test]
    fn send_sequence_advances_only_after_complete_fake_write() {
        let (session, session_id) = fresh_session();
        let mut sender = ControlChannel::new(MemoryIo::default(), EndpointRole::Recorder, session);
        sender.send(MessageKind::RecorderHello, b"one").unwrap();
        sender
            .send(MessageKind::HeartbeatChallenge, b"two")
            .unwrap();
        let output = &sender.io().outbound;

        let first_len = CONTROL_HEADER_LEN + 3;
        assert_eq!(
            decode_frame(&output[..first_len], session_id, 0)
                .unwrap()
                .sequence,
            0
        );
        assert_eq!(
            decode_frame(&output[first_len..], session_id, 1)
                .unwrap()
                .sequence,
            1
        );
    }

    #[test]
    fn send_failure_is_terminal_and_never_retried() {
        let io = MemoryIo {
            fail_send: true,
            ..MemoryIo::default()
        };
        let (session, _) = fresh_session();
        let mut sender = ControlChannel::new(io, EndpointRole::Recorder, session);
        assert!(matches!(
            sender.send(MessageKind::RecorderHello, b""),
            Err(ControlError::Io(io::ErrorKind::BrokenPipe))
        ));
        assert!(matches!(
            sender.send(MessageKind::RecorderHello, b""),
            Err(ControlError::Poisoned)
        ));
    }

    #[test]
    fn unknown_version_flags_kind_reserved_and_oversize_reject() {
        let base = encode_frame(SESSION, 0, MessageKind::CollectorHello, b"").unwrap();
        let mutations: &[(usize, &[u8], fn(&ControlError) -> bool)] = &[
            (4, &2u16.to_le_bytes(), |e| {
                matches!(e, ControlError::UnsupportedVersion(2))
            }),
            (8, &99u16.to_le_bytes(), |e| {
                matches!(e, ControlError::UnknownMessageKind(99))
            }),
            (10, &1u16.to_le_bytes(), |e| {
                matches!(e, ControlError::NonZeroFlags(1))
            }),
            (44, &1u32.to_le_bytes(), |e| {
                matches!(e, ControlError::NonZeroReserved(1))
            }),
            (
                12,
                &((CONTROL_MAX_PAYLOAD_LEN as u32) + 1).to_le_bytes(),
                |e| matches!(e, ControlError::PayloadTooLarge(_)),
            ),
        ];
        for (offset, bytes, predicate) in mutations {
            let mut wire = base.clone();
            wire[*offset..*offset + bytes.len()].copy_from_slice(bytes);
            let error = decode_frame(&wire, SESSION, 0).unwrap_err();
            assert!(predicate(&error), "unexpected error: {error}");
        }
    }

    #[test]
    fn zero_session_and_max_sequence_are_never_admitted() {
        assert!(matches!(
            SessionCapability::claim([0; 16]),
            Err(ControlError::InvalidSessionId)
        ));
        assert!(matches!(
            encode_frame(SESSION, u64::MAX, MessageKind::RecorderHello, b""),
            Err(ControlError::SequenceExhausted)
        ));
    }

    #[test]
    fn accepted_sequence_cannot_be_replayed_by_reconstructing_same_session() {
        let session_bytes = fresh_session_bytes();
        let session = claim_session_for_test(session_bytes).unwrap();
        let session_id = session.session_id();
        let replay = encode_frame(session_id, 0, MessageKind::CollectorHello, b"").unwrap();

        let mut channel = ControlChannel::new(
            MemoryIo::with_inbound(replay.clone()),
            EndpointRole::Recorder,
            session,
        );
        assert_eq!(channel.recv().unwrap().sequence, 0);
        drop(channel);

        assert!(matches!(
            claim_session_for_test(session_bytes),
            Err(ControlError::SessionAlreadyClaimed(claimed)) if claimed == session_id
        ));
    }

    #[test]
    fn session_claim_never_blocks_a_deadline_path_on_registry_contention() {
        let guard = claimed_session_ids().lock().unwrap();
        let result = SessionCapability::claim(fresh_session_bytes());
        assert!(matches!(
            result,
            Err(ControlError::SessionClaimRegistryBusy)
        ));
        drop(guard);
    }

    #[test]
    fn peer_admission_rejects_pid_creation_time_and_capability_faults() {
        let expected = ControlPeerIdentity {
            pid: 100,
            creation_filetime: 200,
        };
        let heartbeat = ControlCapabilities::from_bits(ControlCapabilities::HEARTBEAT).unwrap();

        let (session, _) = fresh_session();
        assert!(matches!(
            ValidatedSessionCapability::admit(
                session,
                expected,
                ControlPeerIdentity {
                    pid: 101,
                    ..expected
                },
                heartbeat,
                heartbeat,
            ),
            Err(ControlError::PeerPidMismatch { .. })
        ));

        let (session, _) = fresh_session();
        assert!(matches!(
            ValidatedSessionCapability::admit(
                session,
                expected,
                ControlPeerIdentity {
                    creation_filetime: 201,
                    ..expected
                },
                heartbeat,
                heartbeat,
            ),
            Err(ControlError::PeerCreationTimeMismatch { .. })
        ));

        let (session, _) = fresh_session();
        let none = ControlCapabilities::from_bits(0).unwrap();
        assert!(matches!(
            ValidatedSessionCapability::admit(session, expected, expected, heartbeat, none,),
            Err(ControlError::MissingCapabilities { .. })
        ));
        assert!(matches!(
            ControlCapabilities::from_bits(1 << 31),
            Err(ControlError::UnknownCapabilities(_))
        ));
    }

    #[test]
    fn deadline_channel_rejects_pre_cancel_without_touching_io() {
        let (admission, _) = admitted_session(
            ControlCapabilities::HEARTBEAT,
            ControlCapabilities::HEARTBEAT,
        );
        let mut channel = DeadlineControlChannel::new(
            DeadlineMemoryIo::default(),
            EndpointRole::Recorder,
            admission,
        );
        let cancellation = ControlCancellation::new();
        cancellation.cancel();
        let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            channel.send_until(
                MessageKind::HeartbeatChallenge,
                b"x",
                deadline,
                &cancellation
            ),
            Err(ControlError::Cancelled)
        ));
        assert!(channel.io().outbound.is_empty());
        assert!(channel.is_poisoned());
    }

    #[test]
    fn cancellation_observed_after_ambiguous_write_is_terminal() {
        let cancellation = ControlCancellation::new();
        let io = DeadlineMemoryIo {
            cancel_during_send: Some(cancellation.clone()),
            ..DeadlineMemoryIo::default()
        };
        let (admission, _) = admitted_session(
            ControlCapabilities::HEARTBEAT,
            ControlCapabilities::HEARTBEAT,
        );
        let mut channel = DeadlineControlChannel::new(io, EndpointRole::Recorder, admission);
        let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            channel.send_until(
                MessageKind::HeartbeatChallenge,
                b"x",
                deadline,
                &cancellation
            ),
            Err(ControlError::Cancelled)
        ));
        assert!(!channel.io().outbound.is_empty());
        assert!(channel.is_poisoned());
    }

    #[test]
    fn completion_first_observed_after_absolute_deadline_is_rejected() {
        let io = DeadlineMemoryIo {
            delay_during_send: Some(Duration::from_millis(10)),
            ..DeadlineMemoryIo::default()
        };
        let (admission, _) = admitted_session(
            ControlCapabilities::HEARTBEAT,
            ControlCapabilities::HEARTBEAT,
        );
        let mut channel = DeadlineControlChannel::new(io, EndpointRole::Recorder, admission);
        let cancellation = ControlCancellation::new();
        let deadline = ControlDeadline::after(Duration::from_millis(1)).unwrap();
        assert!(matches!(
            channel.send_until(
                MessageKind::HeartbeatChallenge,
                b"x",
                deadline,
                &cancellation
            ),
            Err(ControlError::DeadlineExpired)
        ));
        assert!(channel.is_poisoned());
    }

    fn capability_kinds() -> [(MessageKind, EndpointRole, u32); 34] {
        [
            (
                MessageKind::HeartbeatChallenge,
                EndpointRole::Recorder,
                ControlCapabilities::HEARTBEAT,
            ),
            (
                MessageKind::HeartbeatProof,
                EndpointRole::Collector,
                ControlCapabilities::HEARTBEAT,
            ),
            (
                MessageKind::FirstCompactFrameAck,
                EndpointRole::Recorder,
                ControlCapabilities::FIRST_COMPACT_ACK,
            ),
            (
                MessageKind::RolloverPropose,
                EndpointRole::Collector,
                ControlCapabilities::ROLLOVER,
            ),
            (
                MessageKind::RolloverAck,
                EndpointRole::Recorder,
                ControlCapabilities::ROLLOVER,
            ),
            (
                MessageKind::EpochTokenOffer,
                EndpointRole::Recorder,
                ControlCapabilities::STAGE_B,
            ),
            (
                MessageKind::EpochTokenEcho,
                EndpointRole::Collector,
                ControlCapabilities::STAGE_B,
            ),
            (
                MessageKind::DeclarationTableReady,
                EndpointRole::Collector,
                ControlCapabilities::STAGE_B,
            ),
            (
                MessageKind::DeclarationTableAck,
                EndpointRole::Recorder,
                ControlCapabilities::STAGE_B,
            ),
            (
                MessageKind::EpochSealReady,
                EndpointRole::Collector,
                ControlCapabilities::STAGE_B,
            ),
            (
                MessageKind::EpochSealAck,
                EndpointRole::Recorder,
                ControlCapabilities::STAGE_B,
            ),
            (
                MessageKind::SinkSetupFailure,
                EndpointRole::Collector,
                ControlCapabilities::TRANSPORT_STATUS,
            ),
            (
                MessageKind::SinkTerminalFailure,
                EndpointRole::Collector,
                ControlCapabilities::TRANSPORT_STATUS,
            ),
            (
                MessageKind::IdentityFailure,
                EndpointRole::Collector,
                ControlCapabilities::CAMPAIGN_IDENTITY,
            ),
            (
                MessageKind::IdentityFailureAck,
                EndpointRole::Recorder,
                ControlCapabilities::CAMPAIGN_IDENTITY,
            ),
            (
                MessageKind::EarlyRenderReady,
                EndpointRole::Collector,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderFirstGameD3d,
                EndpointRole::Collector,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderCommitReceipt,
                EndpointRole::Recorder,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderLaunchBind,
                EndpointRole::Recorder,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderVfsIndexChunk,
                EndpointRole::Recorder,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderVfsIndexCommit,
                EndpointRole::Recorder,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderVfsPayloadRequest,
                EndpointRole::Collector,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderVfsPayloadChunk,
                EndpointRole::Recorder,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderVfsPayloadCommit,
                EndpointRole::Recorder,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderContentManifestChunk,
                EndpointRole::Recorder,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderContentManifestCommit,
                EndpointRole::Recorder,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderRuntimeDlcRoster,
                EndpointRole::Collector,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderDiagnosticLivenessRequest,
                EndpointRole::Recorder,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderDiagnosticLivenessResponse,
                EndpointRole::Collector,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderDiagnosticEnd,
                EndpointRole::Recorder,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderG2CaptureArm,
                EndpointRole::Recorder,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::EarlyRenderStartupFailure,
                EndpointRole::Collector,
                ControlCapabilities::EARLY_RENDER,
            ),
            (
                MessageKind::CaptureReady,
                EndpointRole::Collector,
                ControlCapabilities::CAPTURE_READY,
            ),
            (
                MessageKind::CaptureReadyAck,
                EndpointRole::Recorder,
                ControlCapabilities::CAPTURE_READY,
            ),
        ]
    }

    #[test]
    fn v1_message_registry_is_contiguous_and_identity_ids_do_not_collide() {
        let registry = [
            MessageKind::RecorderHello,
            MessageKind::CollectorHello,
            MessageKind::HeartbeatChallenge,
            MessageKind::HeartbeatProof,
            MessageKind::FirstCompactFrameAck,
            MessageKind::RolloverPropose,
            MessageKind::RolloverAck,
            MessageKind::EpochTokenOffer,
            MessageKind::EpochTokenEcho,
            MessageKind::DeclarationTableReady,
            MessageKind::DeclarationTableAck,
            MessageKind::EpochSealReady,
            MessageKind::EpochSealAck,
            MessageKind::SinkSetupFailure,
            MessageKind::SinkTerminalFailure,
            MessageKind::IdentityFailure,
            MessageKind::IdentityFailureAck,
            MessageKind::EarlyRenderReady,
            MessageKind::EarlyRenderFirstGameD3d,
            MessageKind::EarlyRenderCommitReceipt,
            MessageKind::EarlyRenderLaunchBind,
            MessageKind::EarlyRenderVfsIndexChunk,
            MessageKind::EarlyRenderVfsIndexCommit,
            MessageKind::EarlyRenderVfsPayloadRequest,
            MessageKind::EarlyRenderVfsPayloadChunk,
            MessageKind::EarlyRenderVfsPayloadCommit,
            MessageKind::EarlyRenderContentManifestChunk,
            MessageKind::EarlyRenderContentManifestCommit,
            MessageKind::EarlyRenderRuntimeDlcRoster,
            MessageKind::EarlyRenderDiagnosticLivenessRequest,
            MessageKind::EarlyRenderDiagnosticLivenessResponse,
            MessageKind::EarlyRenderDiagnosticEnd,
            MessageKind::EarlyRenderG2CaptureArm,
            MessageKind::EarlyRenderStartupFailure,
            MessageKind::CaptureReady,
            MessageKind::CaptureReadyAck,
        ];
        for (wire, kind) in (1u16..=36).zip(registry) {
            assert_eq!(kind as u16, wire);
            assert_eq!(MessageKind::from_wire(wire).unwrap(), kind);
        }
        assert_eq!(MessageKind::SinkSetupFailure as u16, 14);
        assert_eq!(MessageKind::SinkTerminalFailure as u16, 15);
        assert_eq!(MessageKind::IdentityFailure as u16, 16);
        assert_eq!(MessageKind::IdentityFailureAck as u16, 17);
        assert_eq!(MessageKind::EarlyRenderReady as u16, 18);
        assert_eq!(MessageKind::EarlyRenderFirstGameD3d as u16, 19);
        assert_eq!(MessageKind::EarlyRenderCommitReceipt as u16, 20);
        assert_eq!(MessageKind::EarlyRenderLaunchBind as u16, 21);
        assert_eq!(MessageKind::EarlyRenderVfsIndexChunk as u16, 22);
        assert_eq!(MessageKind::EarlyRenderVfsIndexCommit as u16, 23);
        assert_eq!(MessageKind::EarlyRenderVfsPayloadRequest as u16, 24);
        assert_eq!(MessageKind::EarlyRenderVfsPayloadChunk as u16, 25);
        assert_eq!(MessageKind::EarlyRenderVfsPayloadCommit as u16, 26);
        assert_eq!(MessageKind::EarlyRenderContentManifestChunk as u16, 27);
        assert_eq!(MessageKind::EarlyRenderContentManifestCommit as u16, 28);
        assert_eq!(MessageKind::EarlyRenderRuntimeDlcRoster as u16, 29);
        assert_eq!(MessageKind::EarlyRenderDiagnosticLivenessRequest as u16, 30);
        assert_eq!(
            MessageKind::EarlyRenderDiagnosticLivenessResponse as u16,
            31
        );
        assert_eq!(MessageKind::EarlyRenderDiagnosticEnd as u16, 32);
        assert_eq!(MessageKind::EarlyRenderG2CaptureArm as u16, 33);
        assert_eq!(MessageKind::EarlyRenderStartupFailure as u16, 34);
        assert_eq!(MessageKind::CaptureReady as u16, 35);
        assert_eq!(MessageKind::CaptureReadyAck as u16, 36);
    }

    fn wrong_capability(required: u32) -> u32 {
        match required {
            ControlCapabilities::HEARTBEAT => ControlCapabilities::ROLLOVER,
            ControlCapabilities::FIRST_COMPACT_ACK => ControlCapabilities::HEARTBEAT,
            ControlCapabilities::ROLLOVER => ControlCapabilities::STAGE_B,
            ControlCapabilities::STAGE_B => ControlCapabilities::FIRST_COMPACT_ACK,
            ControlCapabilities::TRANSPORT_STATUS => ControlCapabilities::HEARTBEAT,
            ControlCapabilities::CAMPAIGN_IDENTITY => ControlCapabilities::TRANSPORT_STATUS,
            ControlCapabilities::EARLY_RENDER => ControlCapabilities::CAMPAIGN_IDENTITY,
            ControlCapabilities::CAPTURE_READY => ControlCapabilities::EARLY_RENDER,
            _ => unreachable!(),
        }
    }

    #[test]
    fn every_consumer_kind_denies_zero_and_wrong_capability_on_send() {
        let cancellation = ControlCancellation::new();
        for (kind, sender, required) in capability_kinds() {
            for admitted in [0, wrong_capability(required)] {
                let (admission, _) = admitted_session(0, admitted);
                let mut channel =
                    DeadlineControlChannel::new(DeadlineMemoryIo::default(), sender, admission);
                let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
                assert!(matches!(
                    channel.send_until(kind, b"", deadline, &cancellation),
                    Err(ControlError::CapabilityDenied {
                        kind: denied,
                        required: denied_required,
                        admitted: denied_admitted,
                    }) if denied == kind
                        && denied_required == required
                        && denied_admitted == admitted
                ));
                assert!(channel.io().outbound.is_empty());
                assert!(channel.is_poisoned());
            }
        }
    }

    #[test]
    fn every_consumer_kind_denies_zero_and_wrong_capability_on_receive() {
        let cancellation = ControlCancellation::new();
        for (kind, sender, required) in capability_kinds() {
            for admitted in [0, wrong_capability(required)] {
                let (session, session_id) = fresh_session();
                let wire = encode_frame(session_id, 0, kind, b"").unwrap();
                let peer = ControlPeerIdentity {
                    pid: 42,
                    creation_filetime: 9_001,
                };
                let admission = ValidatedSessionCapability::admit(
                    session,
                    peer,
                    peer,
                    ControlCapabilities::from_bits(0).unwrap(),
                    ControlCapabilities::from_bits(admitted).unwrap(),
                )
                .unwrap();
                let mut channel = DeadlineControlChannel::new(
                    DeadlineMemoryIo {
                        inbound: wire.into(),
                        ..DeadlineMemoryIo::default()
                    },
                    sender.opposite(),
                    admission,
                );
                let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
                assert!(matches!(
                    channel.recv_until(deadline, &cancellation),
                    Err(ControlError::CapabilityDenied {
                        kind: denied,
                        required: denied_required,
                        admitted: denied_admitted,
                    }) if denied == kind
                        && denied_required == required
                        && denied_admitted == admitted
                ));
                assert!(channel.is_poisoned());
            }
        }
    }

    #[test]
    fn handshake_receive_admits_only_sink_failure_kinds_before_capabilities() {
        let cancellation = ControlCancellation::new();

        // Both authoritative sink-failure kinds are receivable on an
        // empty-capability channel through the handshake receive, even though
        // they require TRANSPORT_STATUS. CollectorHello (capability-free) also
        // passes. Every other capability-gated collector kind is still denied
        // and permanently poisons.
        for (kind, permitted) in [
            (MessageKind::SinkSetupFailure, true),
            (MessageKind::SinkTerminalFailure, true),
            (MessageKind::CollectorHello, true),
            (MessageKind::HeartbeatProof, false),
            (MessageKind::RolloverPropose, false),
            (MessageKind::IdentityFailure, false),
        ] {
            let (session, session_id) = fresh_session();
            let peer = ControlPeerIdentity {
                pid: 42,
                creation_filetime: 9_001,
            };
            let admission = ValidatedSessionCapability::admit(
                session,
                peer,
                peer,
                ControlCapabilities::from_bits(0).unwrap(),
                ControlCapabilities::from_bits(0).unwrap(),
            )
            .unwrap();
            let wire = encode_frame(session_id, 0, kind, b"").unwrap();
            let mut channel = DeadlineControlChannel::new(
                DeadlineMemoryIo {
                    inbound: wire.into(),
                    ..DeadlineMemoryIo::default()
                },
                EndpointRole::Recorder,
                admission,
            );
            let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
            let result = channel.recv_handshake_until(deadline, &cancellation);
            if permitted {
                assert_eq!(result.unwrap().kind, kind);
                assert!(!channel.is_poisoned());
            } else {
                assert!(matches!(
                    result,
                    Err(ControlError::CapabilityDenied { kind: denied, .. }) if denied == kind
                ));
                assert!(channel.is_poisoned());
            }
        }
    }

    #[test]
    fn ordinary_receive_still_denies_sink_failure_without_transport_status() {
        // The pre-hello allowance lives only in the handshake receive; the
        // ordinary recv_until path must keep gating the sink-failure kinds.
        let cancellation = ControlCancellation::new();
        let (session, session_id) = fresh_session();
        let peer = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9_001,
        };
        let admission = ValidatedSessionCapability::admit(
            session,
            peer,
            peer,
            ControlCapabilities::from_bits(0).unwrap(),
            ControlCapabilities::from_bits(0).unwrap(),
        )
        .unwrap();
        let wire = encode_frame(session_id, 0, MessageKind::SinkSetupFailure, b"").unwrap();
        let mut channel = DeadlineControlChannel::new(
            DeadlineMemoryIo {
                inbound: wire.into(),
                ..DeadlineMemoryIo::default()
            },
            EndpointRole::Recorder,
            admission,
        );
        let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            channel.recv_until(deadline, &cancellation),
            Err(ControlError::CapabilityDenied {
                kind: MessageKind::SinkSetupFailure,
                ..
            })
        ));
        assert!(channel.is_poisoned());
    }

    #[test]
    fn exact_capability_allows_each_kind_and_hello_needs_none() {
        let cancellation = ControlCancellation::new();
        for (kind, sender, required) in capability_kinds() {
            let (admission, _) = admitted_session(required, required);
            let mut channel =
                DeadlineControlChannel::new(DeadlineMemoryIo::default(), sender, admission);
            let deadline = ControlDeadline::after(Duration::from_secs(1)).unwrap();
            channel
                .send_until(kind, b"", deadline, &cancellation)
                .unwrap();
            assert!(!channel.is_poisoned());
        }

        let (admission, _) = admitted_session(0, 0);
        let mut channel = DeadlineControlChannel::new(
            DeadlineMemoryIo::default(),
            EndpointRole::Recorder,
            admission,
        );
        channel
            .send_until(
                MessageKind::RecorderHello,
                b"",
                ControlDeadline::after(Duration::from_secs(1)).unwrap(),
                &cancellation,
            )
            .unwrap();
    }

    #[test]
    fn rollover_foundation_does_not_expand_the_active_auto_attach_word() {
        let active = ControlCapabilities::auto_attach_duplex_v1();
        assert!(!active.permits(MessageKind::RolloverPropose));
        assert!(!active.permits(MessageKind::RolloverAck));

        let dormant = ControlCapabilities::auto_attach_duplex_rollover_v1();
        assert!(dormant.contains(active));
        assert!(dormant.permits(MessageKind::RolloverPropose));
        assert!(dormant.permits(MessageKind::RolloverAck));
    }
}
