//! Collector-side H4CT duplex-control CLIENT (injected DLL endpoint).
//!
//! This is the injected-DLL counterpart of the recorder's control server
//! described in `extractor/documents/DUPLEX_CONTROL_TRANSPORT.md`. It opens the
//! per-attach control pipe, speaks FIRST by sending the 72-byte capabilities
//! `CollectorHello` (the recorder-side admission,
//! `DuplexWorkerSession::receive_collector_hello`, begins by waiting for it; no
//! endpoint ever sends a `RecorderHello` frame — the recorder's expectation and
//! the control challenge travel out-of-band in the attach payload), echoes
//! heartbeat challenges, participates in the first-compact-frame ack, and shuts
//! its worker down on a graceful stop. The H4CR data pipe (`open_pipe`) is
//! unchanged; this is the second, per-attach control pipe.
//!
//! # Shared codec
//!
//! The v1 wire format (framing, CRC-32, per-direction sequence, direction and
//! poison rules) and every typed payload live once in
//! [`compact_contract::control_transport`]; the recorder host consumes the exact
//! same code. This module never re-implements the frame or payload bytes: it
//! drives the shared [`ControlChannel`] state machine through a deadline-bearing
//! transport seam, so a wire drift is a shared-crate test failure, not two
//! copies.
//!
//! # Deadlines
//!
//! The proof-of-concept [`ControlChannel`] is deadline-free. Per the design (§4
//! "Timeouts and liveness", §5), every production send/receive must carry an
//! explicit absolute monotonic deadline. This module supplies that on top of the
//! shared codec: [`DeadlineControlIo`] is the deadline-bearing backend, and
//! [`DeadlineBridge`] adapts it to the shared `ControlIo` by injecting the
//! current operation's deadline. The transport is the party that enforces the
//! deadline and reports `TimedOut`; the channel then poisons through its normal
//! terminal path, so a timeout can never be read as "probably alive".
//!
//! # Activation freeze
//!
//! Everything here is DORMANT, and dormancy is ENFORCED, not merely described:
//! every production entry point that can open a pipe or spawn a connector
//! worker requires a move-only [`DuplexClientProductPermit`], and the only
//! production constructor ([`duplex_client_product_permit`]) fails while
//! [`DUPLEX_CONTROL_CLIENT_ENABLED`] is `false` — the same private-permit
//! pattern as the recorder-side `AutoAttachDuplexProductPermit`. Activation is
//! owner-gated on live validation. The module compiles, is fully unit-tested
//! against fake in-memory transports, and is reachable only through the permit.

#![allow(dead_code)]

use std::cell::Cell;
use std::io;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use compact_contract::control_transport::payloads::{
    decode_first_compact_frame_ack, decode_heartbeat_challenge,
    encode_collector_hello_with_capabilities, encode_heartbeat_proof, CollectorHelloV1,
    FirstCompactFrameAckV1, HeartbeatProofV1, PayloadError, HEARTBEAT_STATE_PAUSED_FOR_CONTROL,
    HEARTBEAT_STATE_PRODUCING, HEARTBEAT_STATE_WAITING_FOR_WORLD,
};
use compact_contract::identity_control::{
    decode_identity_failure_ack, encode_identity_failure, IdentityControlCodecError,
    IdentityFailureAckV1, IdentityFailureV1,
};
use compact_contract::sink_failure::{encode_sink_failure, SinkFailureKind, SinkFailureReport, SinkStatusError};
use compact_contract::control_transport::{
    ControlCapabilities, ControlChannel, ControlError, ControlIo, EndpointRole, MessageKind,
    SessionCapability, CONTROL_VERSION,
};

/// ACTIVATION FREEZE: the duplex control client is built and unit-tested but
/// must NOT be wired into the live attach path yet. The live integration is
/// gated on this constant (kept `false` until owner sign-off on live
/// validation), exactly like the recorder-side duplex activation seam.
pub const DUPLEX_CONTROL_CLIENT_ENABLED: bool = false;

/// Move-only proof that the activation gate was consulted (mirrors the
/// recorder-side `AutoAttachDuplexProductPermit`). Every production entry point
/// that can start pipe I/O or spawn a worker thread demands one, so a caller
/// that ignores [`DUPLEX_CONTROL_CLIENT_ENABLED`] simply cannot compile a call:
/// dormancy is enforced by construction.
pub struct DuplexClientProductPermit {
    _private: (),
}

/// The ONLY production constructor for [`DuplexClientProductPermit`]: fails
/// closed while the activation constant is `false`.
pub fn duplex_client_product_permit() -> Result<DuplexClientProductPermit, ClientError> {
    if DUPLEX_CONTROL_CLIENT_ENABLED {
        Ok(DuplexClientProductPermit { _private: () })
    } else {
        Err(ClientError::ActivationDisabled)
    }
}

/// Test-only permit so the offline suites can exercise the connector/worker
/// logic while the activation constant stays `false`. Never available to a
/// production build.
#[cfg(any(test, feature = "test-support"))]
pub fn test_duplex_client_permit() -> DuplexClientProductPermit {
    DuplexClientProductPermit { _private: () }
}

// ---------------------------------------------------------------------------
// Monotonic clock and absolute deadlines
// ---------------------------------------------------------------------------

/// A point on an abstract monotonic timeline, in nanoseconds from an arbitrary
/// origin. Never derived from wall-clock time.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct MonotonicInstant(pub u128);

/// Source of monotonic time. Injected so tests drive a deterministic fake clock.
pub trait MonotonicClock {
    fn now(&self) -> MonotonicInstant;
}

/// One checked absolute deadline. Computed once from a nonzero budget and used
/// without reset or extension across the operations it governs (§4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MonotonicDeadline(MonotonicInstant);

/// `MAXDWORD - 1`: the largest legal finite `WaitNamedPipeW` timeout. `MAXDWORD`
/// itself is `NMPWAIT_WAIT_FOREVER`; `0` selects the server default. Both are
/// forbidden by the design, so the clamp lands strictly inside `1..=MAXDWORD-1`.
const MAX_WAIT_NAMED_PIPE_MS: u128 = (u32::MAX as u128) - 1;

impl MonotonicDeadline {
    /// Absolute deadline `budget` after `clock.now()`. A zero budget yields an
    /// already-reached deadline (the design forbids deadline-free operation, not
    /// an immediately-expired one).
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

    /// Remaining budget, or `None` once reached. Never negative, never wraps.
    pub fn remaining(&self, now: MonotonicInstant) -> Option<Duration> {
        if now >= self.0 {
            None
        } else {
            Some(Duration::from_nanos((self.0 .0 - now.0).min(u64::MAX as u128) as u64))
        }
    }

    /// The `WaitNamedPipeW` timeout argument at `now`: the remaining budget
    /// rounded UP to a whole millisecond and clamped to `1..=MAXDWORD-1`. `None`
    /// once no positive budget remains — the caller must then not call
    /// `WaitNamedPipeW` at all, because `0` means "server default" and `MAXDWORD`
    /// means "wait forever" (§5).
    pub fn wait_named_pipe_timeout_ms(&self, now: MonotonicInstant) -> Option<u32> {
        if now >= self.0 {
            return None;
        }
        let remaining_ns = self.0 .0 - now.0;
        // Round a positive fractional millisecond up so we never pass 0.
        let ms = remaining_ns.div_ceil(1_000_000);
        if ms == 0 {
            return None;
        }
        Some(ms.min(MAX_WAIT_NAMED_PIPE_MS) as u32)
    }
}

/// Production monotonic clock backed by `std::time::Instant`.
///
/// All instances read from ONE process-global origin so every `MonotonicDeadline`
/// lives on a single monotonic timeline. This matters because a deadline computed
/// by the client must remain comparable inside a separately-constructed backend
/// (e.g. the Win32 overlapped adapter); a per-instance origin would put them on
/// divergent timelines and corrupt the remaining-budget math.
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

// ---------------------------------------------------------------------------
// Deadline-bearing transport seam
// ---------------------------------------------------------------------------

/// Terminal outcomes a deadline-bearing control backend can report. All map to
/// the shared channel's terminal poison path; `PeerClosed` is the one that the
/// client re-interprets as a graceful stop rather than a corruption failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DuplexIoError {
    /// The operation's absolute deadline was reached before completion.
    TimedOut,
    /// The peer closed the pipe cleanly at a frame boundary (zero bytes pending
    /// for a fresh read). Interpreted as a graceful stop receipt.
    PeerClosed,
    /// EOF/short transfer in the middle of a frame: ambiguous, terminal.
    Truncated,
    /// Broken pipe or any other terminal I/O failure.
    Disconnected,
}

impl DuplexIoError {
    /// Map to an `io::Error` so the shared `ControlChannel` poisons through its
    /// existing terminal path. The `ErrorKind`s are chosen so the channel's
    /// `map_io_error` classifies truncation as `Truncated` while the client can
    /// still recognise a clean peer close (`ConnectionAborted`).
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

/// The production control backend seam: every transfer carries an explicit
/// absolute monotonic deadline and either completes fully or returns a terminal
/// [`DuplexIoError`]. A partial transfer followed by an error is still an error.
pub trait DeadlineControlIo {
    fn send_all(&mut self, bytes: &[u8], deadline: MonotonicDeadline) -> Result<(), DuplexIoError>;
    fn recv_exact(
        &mut self,
        bytes: &mut [u8],
        deadline: MonotonicDeadline,
    ) -> Result<(), DuplexIoError>;
}

/// Adapts a [`DeadlineControlIo`] to the shared, deadline-free [`ControlIo`] the
/// codec is generic over, by injecting the current operation's deadline from a
/// shared cell the owning client sets immediately before each channel call. The
/// channel is single-threaded after connection, so `Rc<Cell<_>>` is sufficient
/// and avoids widening the shared trait's signature.
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

// ---------------------------------------------------------------------------
// Client policy and identity
// ---------------------------------------------------------------------------

/// The collector's own attach facts, echoed exactly in `CollectorHello`. The
/// identity fields are observed at attach time (real PID, process creation
/// FILETIME, the H4CR record version, and the directly-verified loaded-DLL
/// SHA-256), never fabricated; the challenge is the per-attach CSPRNG value
/// delivered out-of-band with the attach payload (the recorder hands it to the
/// injected instance the same way it hands over the pipe token — no
/// `RecorderHello` frame ever carries it). Exact byte equality of the resulting
/// hello with the recorder's expectation is what proves the connected injected
/// instance answered (§3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CollectorIdentity {
    // PRIVATE fields: a `CollectorIdentity` can ONLY be built through
    // [`CollectorIdentity::from_attach`], which requires the per-attach control
    // challenge as a value. There is no way to synthesize one without a
    // challenge, so the "no challenge => no duplex client" invariant is enforced
    // at the type boundary, not merely by convention.
    pid: u32,
    creation_filetime: u64,
    h4cr_version: u16,
    dll_sha256: [u8; 32],
    challenge: [u8; 16],
}

impl CollectorIdentity {
    /// The ONLY constructor. Takes the runtime-observed attach facts plus the
    /// per-attach control `challenge` (parsed from the injection control block);
    /// requiring the challenge by value is what makes a challenge-less identity
    /// impossible to build.
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

/// Nonzero, versioned product-policy durations. The handshake budget is the one
/// 30-second initial-handshake budget (§4); heartbeat and first-frame budgets
/// are separate explicit policies that never reset or extend the handshake or
/// each other.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DuplexClientPolicy {
    pub handshake_budget: Duration,
    pub heartbeat_cycle: Duration,
    pub first_frame_budget: Duration,
}

impl DuplexClientPolicy {
    pub const DEFAULT: Self = Self {
        handshake_budget: Duration::from_secs(30),
        heartbeat_cycle: Duration::from_secs(5),
        first_frame_budget: Duration::from_secs(60),
    };
}

impl Default for DuplexClientPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The collector's reported production state for `HeartbeatProof.state` (§3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductionState {
    WaitingForWorld,
    Producing,
    PausedForControl,
}

impl ProductionState {
    fn wire(self) -> u16 {
        match self {
            Self::WaitingForWorld => HEARTBEAT_STATE_WAITING_FOR_WORLD,
            Self::Producing => HEARTBEAT_STATE_PRODUCING,
            Self::PausedForControl => HEARTBEAT_STATE_PAUSED_FOR_CONTROL,
        }
    }
}

// ---------------------------------------------------------------------------
// Client state machine
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientPhase {
    /// Before the collector-first `CollectorHello` was sent.
    Handshaking,
    /// Hello sent; serving recorder-driven frames (heartbeats, first-frame ack).
    Attached,
    /// First-compact-frame ack accepted.
    Producing,
    /// Cleanly stopped (graceful stop by request or peer close).
    Stopped,
    /// Terminally failed/poisoned. No reconnect, no reinjection (§4).
    Failed,
}

/// Events surfaced by [`CollectorControlClient::serve_next`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientEvent {
    HeartbeatServed { host_challenge_counter: u64, reported_state: u16 },
    FirstFrameAcked(FirstCompactFrameAckV1),
    /// The recorder durably recorded (or rejected) a previously-sent
    /// `IdentityFailure` and returned its `IdentityFailureAck` (CAMPAIGN_IDENTITY).
    IdentityFailureAcked(IdentityFailureAckV1),
    GracefulStop,
}

/// Terminal outcome of the steady-state [`CollectorControlClient::run`] loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientOutcome {
    /// Cooperative stop flag observed at a cycle boundary.
    StoppedByRequest,
    /// Recorder closed the control pipe cleanly (graceful stop receipt).
    StoppedByPeer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientError {
    /// Terminal transport/codec failure from the shared channel.
    Control(ControlError),
    /// A typed payload failed exact-length/deny-by-default decoding.
    Payload(PayloadError),
    /// A sink-failure report failed the shared TRANSPORT_STATUS codec's
    /// validation (never sent) or an inbound one failed to decode.
    SinkStatus(SinkStatusError),
    /// A campaign-identity CIF1/CIA1 payload failed the shared codec.
    IdentityCodec(IdentityControlCodecError),
    /// A recorder->collector kind valid on the wire but not implemented by the
    /// v1 client (rollover/Stage-B belong to their own lanes), or a kind no
    /// production endpoint ever sends (e.g. `RecorderHello`): fail closed.
    Unsupported(MessageKind),
    /// A production pipe-I/O entry point was consulted while the duplex client
    /// is frozen dormant ([`DUPLEX_CONTROL_CLIENT_ENABLED`] is `false`).
    ActivationDisabled,
    /// A method was called after the session already stopped or failed.
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

/// The collector's H4CT control-pipe client over the shared codec.
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
}

impl<T: DeadlineControlIo, C: MonotonicClock> CollectorControlClient<T, C> {
    /// Build a client over a connected, deadline-bearing backend. `session` is
    /// the move-only, already-claimed capability for this attach's CSPRNG id.
    ///
    /// Requires a [`DuplexClientProductPermit`]: this is the single construction
    /// gate for the whole client. Because the only production permit constructor
    /// fails closed while [`DUPLEX_CONTROL_CLIENT_ENABLED`] is `false`, no
    /// production code can build a client — and therefore cannot handshake,
    /// report failures, or drive any pipe I/O — while the activation freeze holds.
    pub fn new(
        _permit: &DuplexClientProductPermit,
        io: T,
        clock: C,
        session: SessionCapability,
        identity: CollectorIdentity,
        policy: DuplexClientPolicy,
    ) -> Self {
        // A placeholder deadline; every real operation overwrites the cell
        // immediately before it drives the channel.
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
        }
    }

    pub fn phase(&self) -> ClientPhase {
        self.phase
    }

    pub fn first_frame_acked(&self) -> bool {
        self.first_frame_acked
    }

    /// Update the production progress the client reports in the next
    /// `HeartbeatProof`. Fed by the H4CR data-pipe writer, never fabricated.
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

    /// Receive one control frame under `deadline`, returning `Ok(None)` for a
    /// clean peer close (graceful stop). Any other error is terminal.
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

    /// (b) Open the exchange under the one initial-handshake deadline. The
    /// COLLECTOR speaks first: the recorder-side admission
    /// (`DuplexWorkerSession::receive_collector_hello`) begins by waiting for
    /// `CollectorHello`, and no production endpoint ever sends a `RecorderHello`
    /// frame — the recorder's expectation and the control challenge travel
    /// out-of-band in the attach payload and are construction inputs here
    /// ([`CollectorIdentity`]). The hello is the exact 72-byte
    /// product-admission form: the identity facts plus the closed v1 capability
    /// claim ([`ControlCapabilities::auto_attach_duplex_v1`]), which admission
    /// requires verbatim.
    ///
    /// There is no dedicated hello-ack kind in the closed v1 registry: a
    /// successful admission is observed as the first recorder-driven frame
    /// (awaited via [`Self::serve_next`]/[`Self::run`] under their own
    /// deadlines), and a rejection is observed as the recorder poisoning or
    /// closing the pipe.
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
            ControlCapabilities::auto_attach_duplex_v1().bits(),
        );
        self.send_frame(MessageKind::CollectorHello, &payload, handshake_deadline)?;
        self.phase = ClientPhase::Attached;
        Ok(())
    }

    /// TRANSPORT_STATUS capability: report an authoritative sink transport
    /// failure to the recorder. The collector detects that its own H4CR data
    /// sink (file/pipe) failed; this frames the shared `encode_sink_failure`
    /// payload under the matching envelope kind (Setup -> `SinkSetupFailure`,
    /// Terminal -> `SinkTerminalFailure`, mirroring the recorder's
    /// `SinkFailureMessageKind`). The payload bytes are the ONE shared codec,
    /// never re-implemented here.
    ///
    /// # Pre-hello vs post-hello (mirrors the host's first-frame contract)
    ///
    /// The host's `recv_handshake_until`
    /// (`compact_contract::control_transport`) opens a closed pre-hello window
    /// that accepts a sink-failure frame as the collector's very FIRST control
    /// frame, so a sink that fails during setup — before the collector can even
    /// answer with a hello — still delivers a *structured* failure instead of a
    /// bare timeout. This method mirrors that:
    ///
    /// - **Handshaking (pre-hello):** either `SinkFailureKind::Setup` or
    ///   `SinkFailureKind::Terminal` may be the very first frame — this mirrors the
    ///   host's first-frame contract exactly. `recv_handshake_until` /
    ///   `is_pre_hello_receivable_sink_failure` admit BOTH sink-failure kinds, and
    ///   the shared sink codec permits a `Terminal` on sequence 0 with no prior
    ///   completion (both sinks can open, then the first sequence-0 write fails
    ///   before the hello — the only valid report is then `Terminal`). It is sent
    ///   as frame 0 (sequence 0) and the client then terminates: no hello follows,
    ///   the session is over. (This method only accepts a `SinkFailureReport`, so
    ///   `report.kind` is already one of the two sink kinds; no other kind can be
    ///   reported here at all.)
    /// - **Attached / Producing (post-hello):** either kind may be sent; the
    ///   recorder consumes it as terminal and fails the session.
    pub fn report_sink_failure(
        &mut self,
        report: &SinkFailureReport,
        deadline: MonotonicDeadline,
    ) -> Result<(), ClientError> {
        let pre_hello = match self.phase {
            // Pre-hello: BOTH sink-failure kinds are legitimate first frames,
            // matching the host's pre-hello window (Setup or Terminal on seq 0).
            ClientPhase::Handshaking => true,
            ClientPhase::Attached | ClientPhase::Producing => false,
            ClientPhase::Stopped | ClientPhase::Failed => return Err(ClientError::NotUsable),
        };
        let payload = match encode_sink_failure(report) {
            Ok(payload) => payload,
            Err(err) => return self.fail(ClientError::SinkStatus(err)),
        };
        // Envelope-kind selection (not a wire format): the two authoritative
        // sink-failure kinds, identical to recorder_host's SinkFailureMessageKind.
        let kind = match report.kind {
            SinkFailureKind::Setup => MessageKind::SinkSetupFailure,
            SinkFailureKind::Terminal => MessageKind::SinkTerminalFailure,
        };
        self.send_frame(kind, &payload, deadline)?;
        if pre_hello {
            // A structured pre-hello sink failure IS the whole session: no hello
            // will follow it (the recorder fails the worker on receipt), so the
            // client transitions terminally and refuses any further use.
            self.phase = ClientPhase::Failed;
        }
        Ok(())
    }

    /// CAMPAIGN_IDENTITY capability: report a campaign-identity read failure
    /// (CIF1) to the recorder, which durably records it and returns an
    /// `IdentityFailureAck` (surfaced by the next [`Self::serve_next`]). The
    /// payload is the ONE shared `encode_identity_failure` codec.
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

    /// (c)/(d) Receive one steady-state frame under `deadline` and act:
    /// `HeartbeatChallenge` -> reply `HeartbeatProof` echoing that challenge and
    /// reporting the last written H4CR sequence; `FirstCompactFrameAck` -> record
    /// the ack; `IdentityFailureAck` -> surface the CAMPAIGN_IDENTITY ack; a clean
    /// peer close -> `GracefulStop`.
    ///
    /// # Advertised-capability coverage (all four in 0x33 are genuine)
    ///
    /// - HEARTBEAT: receive `HeartbeatChallenge`, send `HeartbeatProof` (here).
    /// - FIRST_COMPACT_ACK: receive `FirstCompactFrameAck` (here).
    /// - TRANSPORT_STATUS: send `SinkSetupFailure`/`SinkTerminalFailure` via
    ///   [`Self::report_sink_failure`].
    /// - CAMPAIGN_IDENTITY: send `IdentityFailure` via
    ///   [`Self::report_identity_failure`], receive `IdentityFailureAck` (here).
    ///
    /// Kinds OUTSIDE the advertised set are NOT advertised and fail closed:
    /// rollover (`RolloverPropose/Ack`), Stage-B epoch/declaration/seal, and any
    /// hello frame are deferred future lanes and return
    /// [`ClientError::Unsupported`] — never silently accepted.
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
                    state: self.reported_state.wire(),
                });
                self.send_frame(MessageKind::HeartbeatProof, &proof, deadline)?;
                Ok(ClientEvent::HeartbeatServed {
                    host_challenge_counter: challenge.host_challenge_counter,
                    reported_state: self.reported_state.wire(),
                })
            }
            MessageKind::FirstCompactFrameAck => {
                let ack = match decode_first_compact_frame_ack(&frame.payload) {
                    Ok(ack) => ack,
                    Err(err) => return self.fail(ClientError::Payload(err)),
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

    /// (c)/(e) Steady-state loop: serve control frames under a fresh
    /// heartbeat-cycle deadline each iteration, checking the cooperative stop
    /// flag at each boundary. Returns when the stop flag is set
    /// (`StoppedByRequest`) or the recorder closes the pipe (`StoppedByPeer`).
    pub fn run(&mut self, stop_flag: &AtomicBool) -> Result<ClientOutcome, ClientError> {
        loop {
            if stop_flag.load(Ordering::Acquire) {
                self.phase = ClientPhase::Stopped;
                return Ok(ClientOutcome::StoppedByRequest);
            }
            let deadline = MonotonicDeadline::from_now(&self.clock, self.policy.heartbeat_cycle);
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

// ---------------------------------------------------------------------------
// Bounded client-side pipe connector (retry loop + isolated worker)
// ---------------------------------------------------------------------------

/// Terminal error from the bounded client connector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DuplexConnectError {
    /// The initial-handshake deadline was reached before a validated connection.
    DeadlineExpired,
    /// A non-retryable Win32 failure (e.g. `ERROR_ACCESS_DENIED`).
    Fatal(&'static str),
}

/// Result of one `CreateFileW` attempt.
pub enum CreateFileOutcome<H> {
    /// Connected; the handle is not yet validated against the deadline.
    Connected(H),
    /// `ERROR_PIPE_BUSY`: an instance exists but is busy; retryable.
    Busy,
    /// `ERROR_FILE_NOT_FOUND`: the server has not created the pipe yet; retryable.
    NotFound,
    /// Non-retryable failure.
    Fatal(&'static str),
}

/// Result of one finite `WaitNamedPipeW`.
pub enum WaitOutcome {
    /// An instance became available; retry `CreateFileW`.
    Signaled,
    /// The finite wait elapsed; the driver rechecks the absolute deadline.
    TimedOut,
    /// Non-retryable failure.
    Fatal(&'static str),
}

/// One bounded `CreateFileW`/`WaitNamedPipeW` attempt strategy. The real Win32
/// implementation opens `\\.\pipe\battle-plan_control_<pid>_<token>` with
/// `GENERIC_READ | GENERIC_WRITE | FILE_FLAG_OVERLAPPED`; tests supply a
/// scripted fake so the retry/clamp/deadline logic is deterministic.
pub trait PipeConnector {
    type Handle;
    fn create_file(&mut self) -> CreateFileOutcome<Self::Handle>;
    fn wait_named_pipe(&mut self, timeout_ms: u32) -> WaitOutcome;
    /// Close a handle that was opened but observed at/after the deadline, so a
    /// late candidate can never be published.
    fn discard(&mut self, handle: Self::Handle);
}

/// Drive the bounded client open loop (§5 "Each client open runs its local
/// `CreateFileW`/`WaitNamedPipeW` retry loop"). Every `CreateFileW`,
/// `WaitNamedPipeW`, and retry consumes the one unchanged `deadline`; a handle
/// first observed at/after it is discarded and never published.
///
/// Requires a [`DuplexClientProductPermit`]: this is a pipe-I/O entry point and
/// stays unreachable while the activation freeze holds.
pub fn open_client_control_pipe<P: PipeConnector, C: MonotonicClock>(
    _permit: &DuplexClientProductPermit,
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
                // Every successful CreateFileW is followed by a monotonic
                // recheck; a handle first observed at/after the deadline is
                // closed and returns DeadlineExpired, never a connection.
                if deadline.is_expired(clock.now()) {
                    connector.discard(handle);
                    return Err(DuplexConnectError::DeadlineExpired);
                }
                return Ok(handle);
            }
            CreateFileOutcome::Fatal(reason) => return Err(DuplexConnectError::Fatal(reason)),
            // ERROR_PIPE_BUSY / ERROR_FILE_NOT_FOUND: wait, then retry under the
            // SAME absolute deadline.
            CreateFileOutcome::Busy | CreateFileOutcome::NotFound => {}
        }
        // Clip the wait to the checked remaining budget, rounded up, never 0 and
        // never MAXDWORD. No positive budget => expire without calling wait.
        let Some(timeout_ms) = deadline.wait_named_pipe_timeout_ms(clock.now()) else {
            return Err(DuplexConnectError::DeadlineExpired);
        };
        match connector.wait_named_pipe(timeout_ms) {
            // A successful wait is only a retry hint; the next CreateFileW may
            // still race ERROR_PIPE_BUSY and retry under the same deadline.
            WaitOutcome::Signaled | WaitOutcome::TimedOut => {}
            WaitOutcome::Fatal(reason) => return Err(DuplexConnectError::Fatal(reason)),
        }
    }
}

/// The client control-pipe name for one attach (§1). The example in the design
/// is components, not a literal path; `token` is the per-attempt CSPRNG token
/// delivered in the collector's control block (same channel as the H4CR data
/// pipe token).
pub fn control_pipe_name(pid: u32, token: &str) -> String {
    format!(r"\\.\pipe\battle-plan_control_{pid}_{token}")
}

// ---------------------------------------------------------------------------
// Isolated worker with finite wait and zero-time join-or-quarantine
// ---------------------------------------------------------------------------

/// Outcome of [`run_bounded_worker`].
pub enum BoundedWorkerOutcome<R> {
    /// The worker finished strictly before the deadline and was joined.
    Completed(R),
    /// The deadline was reached first; the worker (and anything it owns) was
    /// moved to process-lifetime terminal quarantine and never joined here.
    Quarantined,
}

/// Process-lifetime terminal quarantine for setup/connector workers that did not
/// complete before their governing deadline. Their thread handles are retained
/// forever (never `TerminateThread`d); an optional janitor may later reap them.
fn worker_quarantine() -> &'static Mutex<Vec<JoinHandle<()>>> {
    static QUARANTINE: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();
    QUARANTINE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Number of workers currently quarantined (diagnostic/testing only).
pub fn quarantined_worker_count() -> usize {
    worker_quarantine().lock().map(|q| q.len()).unwrap_or(0)
}

/// Outcome of the bounded post-cancel reap of an overlapped operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReapOutcome {
    /// The operation completed (with data or `ERROR_OPERATION_ABORTED`) within
    /// the bounded reap window: it is no longer pending, so its heap state may
    /// be reclaimed.
    Completed,
    /// A pathological driver never honored the cancel within the bound: the
    /// operation may still be pending against the heap `OVERLAPPED`/buffer, so
    /// that allocation must be deliberately leaked rather than freed (freeing it
    /// while the kernel can still write into it is undefined behavior).
    LeakAbandoned,
}

/// Bounded reap loop shared by the live Win32 backend and unit-tested platform-
/// independently. `poll_signaled` performs ONE bounded wait slice and returns
/// `true` iff the operation's completion event became signaled. After at most
/// `attempts` slices with no completion, the caller must leak the operation's
/// heap state instead of blocking forever or freeing kernel-owned memory.
fn bounded_reap<F: FnMut() -> bool>(attempts: u32, mut poll_signaled: F) -> ReapOutcome {
    for _ in 0..attempts {
        if poll_signaled() {
            return ReapOutcome::Completed;
        }
    }
    ReapOutcome::LeakAbandoned
}

const WORKER_POLL: Duration = Duration::from_millis(1);

/// Run `work` on a dedicated worker thread and wait for it only until
/// `join_deadline`. This is the isolated-worker + finite-wait +
/// zero-time-join-or-quarantine contract (§1, §5): the driver never blocks past
/// the deadline, never calls `TerminateThread`, and transfers a still-running
/// worker to terminal quarantine rather than waiting again.
///
/// The wait is a bounded poll keyed on the injected clock so it is deterministic
/// under a fake clock; the live Win32 connector variant uses
/// `WaitForSingleObject(remaining)` on the thread handle for the same contract.
///
/// Requires a [`DuplexClientProductPermit`]: this spawns a worker thread on the
/// duplex path and stays unreachable while the activation freeze holds.
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
                // Joined strictly before the deadline.
                let _ = handle.join();
                return BoundedWorkerOutcome::Completed(result);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Worker exited without a result (panicked): terminal.
                let _ = handle.join();
                return BoundedWorkerOutcome::Quarantined;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        if join_deadline.is_expired(clock.now()) {
            // Zero-time completion check, then join-or-quarantine.
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

// ---------------------------------------------------------------------------
// Live Win32 adapters (dormant; owner-gated on live validation, not unit-tested)
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod win32 {
    //! Real named-pipe connector and overlapped control I/O. Per the design's
    //! residual-risk section these Win32 behaviors (DACL, overlapped
    //! cancellation, weaker-PC/AV environments) cannot be proven by offline fake
    //! tests; they are validated live and remain dormant behind the activation
    //! freeze. The bounded retry, ms-clamp, deadline, and state-machine logic
    //! that governs them is exercised by the offline tests above through the
    //! platform-independent traits.

    use super::{
        bounded_reap, CreateFileOutcome, DeadlineControlIo, DuplexClientProductPermit,
        DuplexIoError, MonotonicClock, MonotonicDeadline, PipeConnector, ReapOutcome, WaitOutcome,
    };
    use std::os::raw::c_void;
    use std::ptr;

    /// Bound on the post-cancel reap: at most `CANCEL_REAP_ATTEMPTS` waits of
    /// `CANCEL_REAP_SLICE_MS` each (~1s total) before a pathological, cancel-
    /// ignoring driver forces the deliberate leak. Teardown of a cancelled op is
    /// normally immediate; this bound only guards the pathological case.
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
            overlapped: *mut Overlapped,
        ) -> Bool;
        fn GetOverlappedResult(
            handle: Handle,
            overlapped: *mut Overlapped,
            transferred: *mut Dword,
            wait: Bool,
        ) -> Bool;
        fn CancelIoEx(handle: Handle, overlapped: *mut Overlapped) -> Bool;
        fn WaitForSingleObject(handle: Handle, timeout_ms: Dword) -> Dword;
    }

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A raw pipe handle wrapped so an isolated connector worker can return it
    /// across a thread boundary. Ownership is single: whoever holds it closes it.
    ///
    /// The inner handle is PRIVATE: a `SendHandle` can only be minted inside this
    /// module (by the permit-gated `Win32PipeConnector::create_file` or the
    /// completion-event constructor), so external production code cannot wrap an
    /// arbitrary raw handle and bypass the connector's activation permit.
    pub struct SendHandle(Handle);
    // SAFETY: the handle is only ever used by one thread at a time — the
    // connector worker owns it during the open, then transfers ownership back to
    // the driver through the worker's single result value.
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

    /// Real client connector for the H4CT control pipe.
    pub struct Win32PipeConnector {
        name: Vec<u16>,
    }

    impl Win32PipeConnector {
        /// Requires a [`DuplexClientProductPermit`]: this connector's
        /// `create_file` is a direct `CreateFileW` pipe open, so its
        /// construction stays unreachable while the activation freeze holds.
        pub fn new(_permit: &DuplexClientProductPermit, pipe_name: &str) -> Self {
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
            // timeout_ms is pre-clamped to 1..=MAXDWORD-1 by the driver.
            if unsafe { WaitNamedPipeW(self.name.as_ptr(), timeout_ms) } != 0 {
                WaitOutcome::Signaled
            } else {
                // ERROR_FILE_NOT_FOUND (no instance yet) or the finite wait
                // elapsed: the driver rechecks the absolute deadline and retries.
                WaitOutcome::TimedOut
            }
        }

        fn discard(&mut self, handle: SendHandle) {
            drop(handle);
        }
    }

    /// Overlapped, deadline-bounded control I/O over a connected control pipe
    /// handle opened with `FILE_FLAG_OVERLAPPED`.
    pub struct Win32OverlappedControlIo {
        handle: SendHandle,
        event: SendHandle,
    }

    impl Win32OverlappedControlIo {
        /// Wrap a connected overlapped handle for live control I/O. Returns
        /// `None` if the manual-reset completion event cannot be created.
        ///
        /// Requires a [`DuplexClientProductPermit`]: this begins live overlapped
        /// pipe I/O, so it stays unreachable while the activation freeze holds.
        pub fn new(_permit: &DuplexClientProductPermit, handle: SendHandle) -> Option<Self> {
            let event = unsafe { CreateEventW(ptr::null_mut(), 1, 0, ptr::null()) };
            if event.is_null() {
                return None;
            }
            Some(Self {
                handle,
                event: SendHandle(event),
            })
        }

        fn make_pending(&self, buffer: Vec<u8>) -> Box<PendingIo> {
            Box::new(PendingIo {
                overlapped: Overlapped {
                    internal: 0,
                    internal_high: 0,
                    offset: 0,
                    offset_high: 0,
                    h_event: self.event.0,
                },
                buffer,
            })
        }

        /// Wait for one overlapped op to complete within the deadline; on expiry
        /// or wait failure, cancel and reap under a BOUNDED loop.
        ///
        /// Ownership/UB invariant: the `OVERLAPPED` and its data buffer live in
        /// `pending`, a heap `Box`. On completion the reclaimed `Box` is returned
        /// (the caller reads its buffer, then drops it). On the pathological path
        /// — a driver that never honors `CancelIoEx` within the bound — the op may
        /// still be pending against the heap allocation, so the `Box` is
        /// deliberately LEAKED (a documented, bounded one-buffer leak) rather than
        /// freed: freeing memory the kernel can still write into is undefined
        /// behavior, and blocking on `GetOverlappedResult(.., TRUE)` forever is
        /// the unbounded stall this replaces.
        fn wait_and_reap<C: MonotonicClock>(
            &self,
            mut pending: Box<PendingIo>,
            clock: &C,
            deadline: MonotonicDeadline,
        ) -> Result<(u32, Box<PendingIo>), DuplexIoError> {
            let signaled = match deadline.wait_named_pipe_timeout_ms(clock.now()) {
                Some(ms) => (unsafe { WaitForSingleObject(self.event.0, ms) }) == WAIT_OBJECT_0,
                // No budget left: skip the wait, go straight to cancel + reap.
                None => false,
            };
            if signaled {
                let mut transferred: Dword = 0;
                let ok = unsafe {
                    GetOverlappedResult(self.handle.0, &mut pending.overlapped, &mut transferred, 0)
                };
                return if ok != 0 {
                    Ok((transferred, pending))
                } else {
                    // Completed with an error: no longer pending, reclaim + drop.
                    Err(DuplexIoError::Disconnected)
                };
            }

            // Deadline reached or the wait failed: cancel this exact op, then reap
            // under a bounded slice loop instead of an unbounded blocking wait.
            unsafe {
                let _ = CancelIoEx(self.handle.0, &mut pending.overlapped);
            }
            let event = self.event.0;
            match bounded_reap(CANCEL_REAP_ATTEMPTS, || unsafe {
                WaitForSingleObject(event, CANCEL_REAP_SLICE_MS) == WAIT_OBJECT_0
            }) {
                ReapOutcome::Completed => {
                    // Whether it delivered data or ERROR_OPERATION_ABORTED, the op
                    // is no longer pending: reclaim the box (dropped here) and
                    // surface the deadline error.
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
                    // The cancel was never honored within the bound; the kernel
                    // may still write into pending.buffer/overlapped. Leak the
                    // heap allocation deliberately (bounded: one buffer per
                    // abandoned op) — never free it, never block further.
                    Box::leak(pending);
                    Err(DuplexIoError::TimedOut)
                }
            }
        }
    }

    /// Heap-owned state for one in-flight overlapped operation: the `OVERLAPPED`
    /// and the data buffer the kernel reads from (write) or writes into (read).
    /// Heap ownership is what makes the pathological-driver leak sound — a leaked
    /// `Box<PendingIo>` keeps both alive for the process lifetime.
    struct PendingIo {
        overlapped: Overlapped,
        buffer: Vec<u8>,
    }

    impl DeadlineControlIo for Win32OverlappedControlIo {
        fn send_all(
            &mut self,
            bytes: &[u8],
            deadline: MonotonicDeadline,
        ) -> Result<(), DuplexIoError> {
            // Overlapped writes are the responsibility of the live integration's
            // clock; this dormant adapter uses a SystemClock so its deadline math
            // matches the connected handle's real wall progress.
            let clock = super::SystemClock::new();
            let mut sent = 0usize;
            while sent < bytes.len() {
                let chunk_len = (bytes.len() - sent).min(Dword::MAX as usize);
                // Own a heap copy of the chunk: the kernel reads from THIS buffer,
                // so a leaked pending write never dangles into the caller's slice.
                let mut pending = self.make_pending(bytes[sent..sent + chunk_len].to_vec());
                let mut written: Dword = 0;
                let ok = unsafe {
                    WriteFile(
                        self.handle.0,
                        pending.buffer.as_ptr() as *const c_void,
                        pending.buffer.len() as Dword,
                        &mut written,
                        &mut pending.overlapped,
                    )
                };
                let transferred = if ok != 0 {
                    // Completed synchronously: not pending, `pending` drops safely.
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
            // NOTE (live-owed): `received` is local to this single exact read.
            // The shared channel calls `recv_exact` once for the 48-byte header
            // and once for the payload, so a clean zero-byte EOF observed here is
            // a true frame boundary only on the header read. Distinguishing a
            // mid-frame (header-consumed, payload-pending) close from a boundary
            // close for the real handle is part of the live-validation work; the
            // offline fake transport models the boundary rule exactly.
            let clock = super::SystemClock::new();
            let mut received = 0usize;
            while received < bytes.len() {
                let remaining = (bytes.len() - received).min(Dword::MAX as usize);
                // The kernel writes into THIS heap buffer; on success we copy out
                // to the caller's slice, so a leaked pending read never dangles
                // into the caller's memory.
                let mut pending = self.make_pending(vec![0u8; remaining]);
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
                        // A synchronous zero-byte broken pipe: clean close only if
                        // nothing has been read for this frame yet, else truncation.
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
                    // Clean EOF: peer close at a boundary if nothing pending for
                    // this frame, otherwise a mid-frame truncation.
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

    // Keep the ERROR_IO_INCOMPLETE constant referenced so a future janitor reap
    // can distinguish it without a fresh magic number.
    const _: Dword = ERROR_IO_INCOMPLETE;
}

#[cfg(windows)]
pub use win32::{SendHandle, Win32OverlappedControlIo, Win32PipeConnector};

#[cfg(test)]
mod tests;
