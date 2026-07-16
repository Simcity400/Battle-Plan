//! Offline tests for the collector H4CT duplex-control client. Every test drives
//! the real shared state machine through a fake, deadline-bearing in-memory
//! transport and a deterministic fake monotonic clock. No live game, no real
//! pipes.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use compact_contract::control_transport::payloads::{
    decode_collector_hello_with_capabilities, decode_heartbeat_proof, encode_first_compact_frame_ack,
    encode_heartbeat_challenge, encode_recorder_hello, FirstCompactFrameAckV1, HeartbeatChallengeV1,
    RecorderHelloV1, COLLECTOR_HELLO_WITH_CAPABILITIES_LEN, HEARTBEAT_STATE_PRODUCING,
};
use compact_contract::control_transport::{
    claim_session_capability, decode_frame, encode_frame, ControlCapabilities, ControlError,
    MessageKind, SessionCapability, SessionId, CONTROL_HEADER_LEN, CONTROL_VERSION,
};
use compact_contract::identity_control::{
    decode_identity_failure, encode_identity_failure_ack, IdentityFailureAckDisposition,
    IdentityFailureAckV1, IdentityFailureReason, IdentityFailureV1,
};
use compact_contract::sink_failure::{
    decode_sink_failure, SinkErrorClass, SinkFailureKind, SinkFailureReport, SinkTerminality,
    SinkTopology,
};

use super::*;

// --- fake clock -------------------------------------------------------------

#[derive(Clone)]
struct FakeClock(Rc<Cell<u128>>);

impl FakeClock {
    fn at(ns: u128) -> Self {
        FakeClock(Rc::new(Cell::new(ns)))
    }
    fn set(&self, ns: u128) {
        self.0.set(ns);
    }
    fn advance(&self, ns: u128) {
        self.0.set(self.0.get() + ns);
    }
}

impl MonotonicClock for FakeClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant(self.0.get())
    }
}

// --- fake transport ---------------------------------------------------------

struct FakeTransport {
    clock: FakeClock,
    inbound: VecDeque<u8>,
    outbound: Vec<u8>,
    peer_closed: bool,
    fail_next_send: Option<DuplexIoError>,
    fail_next_recv: Option<DuplexIoError>,
}

impl FakeTransport {
    fn new(clock: FakeClock, inbound: Vec<u8>) -> Self {
        Self {
            clock,
            inbound: inbound.into(),
            outbound: Vec::new(),
            peer_closed: false,
            fail_next_send: None,
            fail_next_recv: None,
        }
    }
}

impl DeadlineControlIo for FakeTransport {
    fn send_all(&mut self, bytes: &[u8], deadline: MonotonicDeadline) -> Result<(), DuplexIoError> {
        if deadline.is_expired(self.clock.now()) {
            return Err(DuplexIoError::TimedOut);
        }
        if let Some(err) = self.fail_next_send.take() {
            return Err(err);
        }
        self.outbound.extend_from_slice(bytes);
        Ok(())
    }

    fn recv_exact(
        &mut self,
        bytes: &mut [u8],
        deadline: MonotonicDeadline,
    ) -> Result<(), DuplexIoError> {
        if deadline.is_expired(self.clock.now()) {
            return Err(DuplexIoError::TimedOut);
        }
        if let Some(err) = self.fail_next_recv.take() {
            return Err(err);
        }
        if self.inbound.len() < bytes.len() {
            // Not enough bytes for this exact read.
            return Err(if self.inbound.is_empty() {
                if self.peer_closed {
                    DuplexIoError::PeerClosed
                } else {
                    // Nothing scheduled and no close => treat as a stall.
                    DuplexIoError::TimedOut
                }
            } else {
                DuplexIoError::Truncated
            });
        }
        for slot in bytes.iter_mut() {
            *slot = self.inbound.pop_front().expect("length checked");
        }
        Ok(())
    }
}

// --- helpers ----------------------------------------------------------------

const TEST_PID: u32 = 0x0000_1234;
const TEST_CHALLENGE: [u8; 16] = [0x11; 16];

fn identity() -> CollectorIdentity {
    CollectorIdentity::from_attach(TEST_PID, 0x0102_0304_0506_0708, 7, [0xab; 32], TEST_CHALLENGE)
}

/// The shared test-support activation permit, used everywhere the production
/// code demands a `&DuplexClientProductPermit`. In production this permit is
/// unobtainable while the activation const is false.
fn permit() -> DuplexClientProductPermit {
    test_duplex_client_permit()
}

/// Wire length of the framed collector-first hello the client sends as its
/// first bytes: header + the 72-byte capabilities hello payload.
const HELLO_WIRE_LEN: usize = CONTROL_HEADER_LEN + COLLECTOR_HELLO_WITH_CAPABILITIES_LEN;

fn fresh_session() -> (SessionCapability, SessionId) {
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut bytes = [0x3c; 16];
    bytes[8..].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    // Route through the bounded-retry claim helper (enabled by the crate's
    // `test-support` dev feature) so the process-global session-id registry does
    // not spuriously report `Busy` under the concurrent `try_lock` load of the
    // parallel test run. The ids are already unique per call.
    let cap = claim_session_capability(bytes).unwrap();
    let id = cap.session_id();
    (cap, id)
}

fn recorder_frame(session: SessionId, seq: u64, kind: MessageKind, payload: &[u8]) -> Vec<u8> {
    encode_frame(session, seq, kind, payload).unwrap()
}

fn client(
    clock: FakeClock,
    session: SessionCapability,
    inbound: Vec<u8>,
) -> CollectorControlClient<FakeTransport, FakeClock> {
    let transport = FakeTransport::new(clock.clone(), inbound);
    CollectorControlClient::new(
        &permit(),
        transport,
        clock,
        session,
        identity(),
        DuplexClientPolicy::DEFAULT,
    )
}

fn far_deadline(clock: &FakeClock) -> MonotonicDeadline {
    MonotonicDeadline::from_now(clock, Duration::from_secs(30))
}

// --- dormancy / activation-freeze ------------------------------------------

/// ACTIVATION FREEZE proof. The live duplex-control client must stay dormant
/// until owner sign-off on live validation, and the dormancy is ENFORCED at the
/// type boundary:
///   1. the gate constant is `false`, and
///   2. the ONLY production constructor of [`DuplexClientProductPermit`] fails
///      while it is `false`. EVERY public entry point that can construct a
///      usable client, start pipe I/O, or spawn a connector worker now takes a
///      `&DuplexClientProductPermit` by type — `CollectorControlClient::new`,
///      `CollectorControlClient::perform_handshake`, `open_client_control_pipe`,
///      `run_bounded_worker`, `Win32PipeConnector::new`, and
///      `Win32OverlappedControlIo::new` — so a caller that ignores the constant
///      cannot even compile a call. This test fails the moment the permit
///      becomes obtainable while the constant is still claimed dormant; the type
///      system covers the rest. `SendHandle`'s inner handle and
///      `CollectorIdentity`'s fields are private, so neither the raw pipe handle
///      nor a challenge-less identity can be forged to sidestep the gate.
///   3. merely constructing the state machine (with a test permit) performs no
///      transport I/O — no `CollectorHello` is written and nothing is read until
///      a caller explicitly drives `perform_handshake`/`serve_next`.
#[test]
fn activation_freeze_holds_and_no_public_entry_can_start_pipe_io() {
    assert!(
        !DUPLEX_CONTROL_CLIENT_ENABLED,
        "activation freeze: the duplex-control client must stay dormant"
    );
    // ENFORCED dormancy: the sole production permit constructor fails closed.
    // Because `CollectorControlClient::new`/`perform_handshake` and every live
    // I/O entry point require this permit by type, a fail-closed permit makes a
    // usable client unconstructible and all pipe I/O unreachable in production.
    // SCOPE: the actual compile-time guarantee is the `&DuplexClientProductPermit`
    // requirement on those signatures (a permitless public I/O entry simply cannot
    // be written without the permit). This runtime test does not itself PROVE the
    // absence of a hypothetical future permitless entry point — it asserts the
    // permit fails closed and that mere construction is inert; the type boundary is
    // what enforces the rest, backed by review.
    assert!(
        matches!(
            duplex_client_product_permit(),
            Err(ClientError::ActivationDisabled)
        ),
        "no production caller may obtain a duplex-client permit while frozen"
    );

    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    // A recorder frame is already waiting on the fake transport, but the client
    // is only constructed (via the TEST-support permit), never driven.
    let inbound = recorder_frame(
        session_id,
        0,
        MessageKind::HeartbeatChallenge,
        &encode_heartbeat_challenge(&HeartbeatChallengeV1 {
            challenge: [0x22; 16],
            host_challenge_counter: 1,
        }),
    );
    let inbound_len = inbound.len();
    let client = client(clock, session, inbound);

    // No handshake auto-ran: the client sits in the pre-hello phase.
    assert_eq!(client.phase(), ClientPhase::Handshaking);
    assert!(!client.first_frame_acked());
    // No byte was written to the transport and nothing was read from the
    // waiting inbound frame: construction is inert.
    assert!(
        client.backend().outbound.is_empty(),
        "dormant client must not write any control bytes on construction"
    );
    assert_eq!(
        client.backend().inbound.len(),
        inbound_len,
        "dormant client must not consume any inbound bytes on construction"
    );
}

// --- state-machine tests ----------------------------------------------------

#[test]
fn handshake_heartbeat_and_first_frame_happy_path() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let heartbeat_challenge = [0x22; 16];

    // The COLLECTOR speaks first; the recorder's traffic begins at ITS
    // per-direction sequence 0 with the first heartbeat challenge.
    let mut inbound = recorder_frame(
        session_id,
        0,
        MessageKind::HeartbeatChallenge,
        &encode_heartbeat_challenge(&HeartbeatChallengeV1 {
            challenge: heartbeat_challenge,
            host_challenge_counter: 99,
        }),
    );
    inbound.extend_from_slice(&recorder_frame(
        session_id,
        1,
        MessageKind::FirstCompactFrameAck,
        &encode_first_compact_frame_ack(&FirstCompactFrameAckV1 {
            h4cr_sequence: 12,
            game_hour: 3,
            record_type: 5,
            total_record_len: 4096,
            record_sha256: [0x77; 32],
        }),
    ));

    let mut client = client(clock.clone(), session, inbound);
    client.set_production_progress(41, ProductionState::Producing);

    let deadline = far_deadline(&clock);
    client.perform_handshake(&permit(), deadline).unwrap();
    assert_eq!(client.phase(), ClientPhase::Attached);

    let event = client.serve_next(deadline).unwrap();
    assert_eq!(
        event,
        ClientEvent::HeartbeatServed {
            host_challenge_counter: 99,
            reported_state: HEARTBEAT_STATE_PRODUCING,
        }
    );

    let event = client.serve_next(deadline).unwrap();
    match event {
        ClientEvent::FirstFrameAcked(ack) => assert_eq!(ack.h4cr_sequence, 12),
        other => panic!("expected first-frame ack, got {other:?}"),
    }
    assert_eq!(client.phase(), ClientPhase::Producing);
    assert!(client.first_frame_acked());

    // Outbound: the collector-first 72-byte capabilities CollectorHello@0
    // carrying the attach challenge and exactly the required capability word,
    // then HeartbeatProof@1 echoing the heartbeat challenge with our H4CR
    // sequence.
    let backend = client.backend();
    let hello_frame = decode_frame(&backend.outbound[..HELLO_WIRE_LEN], session_id, 0).unwrap();
    assert_eq!(hello_frame.kind, MessageKind::CollectorHello);
    let (hello, capability_bits) =
        decode_collector_hello_with_capabilities(&hello_frame.payload).unwrap();
    assert_eq!(hello.pid, TEST_PID);
    assert_eq!(hello.control_version, CONTROL_VERSION);
    assert_eq!(hello.challenge, TEST_CHALLENGE);
    assert_eq!(
        capability_bits,
        ControlCapabilities::auto_attach_duplex_v1().bits(),
        "the hello must advertise exactly the capability word recorder admission requires"
    );

    let proof_frame = decode_frame(&backend.outbound[HELLO_WIRE_LEN..], session_id, 1).unwrap();
    assert_eq!(proof_frame.kind, MessageKind::HeartbeatProof);
    let proof = decode_heartbeat_proof(&proof_frame.payload).unwrap();
    assert_eq!(proof.challenge, heartbeat_challenge);
    assert_eq!(proof.last_h4cr_sequence, 41);
    assert_eq!(proof.state, HEARTBEAT_STATE_PRODUCING);
}

#[test]
fn heartbeat_proof_reports_updated_production_progress() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let inbound = recorder_frame(
        session_id,
        0,
        MessageKind::HeartbeatChallenge,
        &encode_heartbeat_challenge(&HeartbeatChallengeV1 {
            challenge: [0x22; 16],
            host_challenge_counter: 1,
        }),
    );

    let mut client = client(clock.clone(), session, inbound);
    let deadline = far_deadline(&clock);
    client.perform_handshake(&permit(), deadline).unwrap();
    client.set_production_progress(500, ProductionState::WaitingForWorld);
    client.serve_next(deadline).unwrap();

    let backend = client.backend();
    let proof_frame = decode_frame(&backend.outbound[HELLO_WIRE_LEN..], session_id, 1).unwrap();
    let proof = decode_heartbeat_proof(&proof_frame.payload).unwrap();
    assert_eq!(proof.last_h4cr_sequence, 500);
    assert_eq!(
        proof.state,
        compact_contract::control_transport::payloads::HEARTBEAT_STATE_WAITING_FOR_WORLD
    );
}

#[test]
fn deadline_expiry_during_handshake_send_poisons_the_session() {
    let clock = FakeClock::at(0);
    let (session, _session_id) = fresh_session();
    // The clock crosses the handshake deadline before the collector-first hello
    // send: a timeout must poison, never be read as "probably alive".
    let mut client = client(clock.clone(), session, Vec::new());
    let deadline = MonotonicDeadline::from_now(&clock, Duration::from_secs(30));
    clock.set(30_000_000_000); // exactly at the deadline

    let err = client.perform_handshake(&permit(), deadline).unwrap_err();
    assert_eq!(
        err,
        ClientError::Control(ControlError::Io(std::io::ErrorKind::TimedOut))
    );
    assert_eq!(client.phase(), ClientPhase::Failed);
    // The channel is permanently poisoned: no retry, no reconnect.
    assert_eq!(
        client.perform_handshake(&permit(), deadline),
        Err(ClientError::NotUsable)
    );
}

#[test]
fn wrong_direction_frame_is_rejected_and_poisons() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    // A collector-originated kind arriving on the receive side is impossible on
    // a correct wire; the shared channel rejects it as wrong-direction.
    let inbound = recorder_frame(
        session_id,
        0,
        MessageKind::HeartbeatProof,
        &vec![0u8; 32],
    );

    let mut client = client(clock.clone(), session, inbound);
    let deadline = far_deadline(&clock);
    client.perform_handshake(&permit(), deadline).unwrap();

    let err = client.serve_next(deadline).unwrap_err();
    assert!(
        matches!(
            err,
            ClientError::Control(ControlError::WrongDirection { .. })
        ),
        "got {err:?}"
    );
    assert_eq!(client.phase(), ClientPhase::Failed);
}

#[test]
fn out_of_order_sequence_is_rejected_and_poisons() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    // Expected first recorder-direction sequence is 0; feed 5.
    let inbound = recorder_frame(
        session_id,
        5,
        MessageKind::HeartbeatChallenge,
        &encode_heartbeat_challenge(&HeartbeatChallengeV1 {
            challenge: [0x22; 16],
            host_challenge_counter: 1,
        }),
    );

    let mut client = client(clock.clone(), session, inbound);
    let deadline = far_deadline(&clock);
    client.perform_handshake(&permit(), deadline).unwrap();

    let err = client.serve_next(deadline).unwrap_err();
    assert!(
        matches!(
            err,
            ClientError::Control(ControlError::SequenceMismatch {
                expected: 0,
                actual: 5
            })
        ),
        "got {err:?}"
    );
    assert_eq!(client.phase(), ClientPhase::Failed);
}

#[test]
fn unexpected_recorder_hello_frame_fails_closed() {
    // No production endpoint ever sends a RecorderHello FRAME: the recorder's
    // expectation and the challenge travel out-of-band in the attach payload,
    // and the recorder-side admission WAITS for the collector-first hello. A
    // client observing one on the wire fails closed rather than interpreting
    // it (it is a wire-valid recorder->collector kind, so it passes the shared
    // direction check and must be refused by the typed consumer).
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let hello = RecorderHelloV1 {
        expected_pid: TEST_PID,
        expected_creation_filetime: 0x0102_0304_0506_0708,
        h4cr_version: 7,
        expected_dll_sha256: [0xab; 32],
        challenge: [0x11; 16],
    };
    let inbound = recorder_frame(
        session_id,
        0,
        MessageKind::RecorderHello,
        &encode_recorder_hello(&hello),
    );
    let mut client = client(clock.clone(), session, inbound);
    let deadline = far_deadline(&clock);
    client.perform_handshake(&permit(), deadline).unwrap();
    assert_eq!(
        client.serve_next(deadline),
        Err(ClientError::Unsupported(MessageKind::RecorderHello))
    );
    assert_eq!(client.phase(), ClientPhase::Failed);
}

#[test]
fn serve_before_handshake_is_not_usable() {
    // The collector speaks first: no frame may be served before the hello went
    // out, and refusing is stateless (no I/O, no poison).
    let clock = FakeClock::at(0);
    let (session, _session_id) = fresh_session();
    let mut client = client(clock.clone(), session, Vec::new());
    let deadline = far_deadline(&clock);
    assert_eq!(client.serve_next(deadline), Err(ClientError::NotUsable));
    assert_eq!(client.phase(), ClientPhase::Handshaking);
    assert!(client.backend().outbound.is_empty());
}

#[test]
fn out_of_scope_recorder_kind_fails_closed() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    // RolloverAck is a valid recorder->collector kind but belongs to the [O]
    // rollover lane; the v1 client fails closed rather than half-handling it.
    let inbound = recorder_frame(
        session_id,
        0,
        MessageKind::RolloverAck,
        &[],
    );
    let mut client = client(clock.clone(), session, inbound);
    let deadline = far_deadline(&clock);
    client.perform_handshake(&permit(), deadline).unwrap();
    assert_eq!(
        client.serve_next(deadline),
        Err(ClientError::Unsupported(MessageKind::RolloverAck))
    );
    assert_eq!(client.phase(), ClientPhase::Failed);
}

#[test]
fn graceful_stop_by_peer_close_is_not_a_failure() {
    let clock = FakeClock::at(0);
    let (session, _session_id) = fresh_session();
    let mut transport = FakeTransport::new(clock.clone(), Vec::new());
    transport.peer_closed = true; // recorder closes cleanly after the hello
    let mut client = CollectorControlClient::new(
        &permit(),
        transport,
        clock.clone(),
        session,
        identity(),
        DuplexClientPolicy::DEFAULT,
    );

    let deadline = far_deadline(&clock);
    client.perform_handshake(&permit(), deadline).unwrap();

    // The peer close arrives at the next receive: a graceful stop, not poison.
    let stop_flag = AtomicBool::new(false);
    let outcome = client.run(&stop_flag).unwrap();
    assert_eq!(outcome, ClientOutcome::StoppedByPeer);
    assert_eq!(client.phase(), ClientPhase::Stopped);
}

#[test]
fn graceful_stop_by_request_stops_cleanly() {
    let clock = FakeClock::at(0);
    let (session, _session_id) = fresh_session();
    let mut client = client(clock.clone(), session, Vec::new());
    let deadline = far_deadline(&clock);
    client.perform_handshake(&permit(), deadline).unwrap();

    let stop_flag = AtomicBool::new(true); // host requested stop
    let outcome = client.run(&stop_flag).unwrap();
    assert_eq!(outcome, ClientOutcome::StoppedByRequest);
    assert_eq!(client.phase(), ClientPhase::Stopped);
}

#[test]
fn terminal_transport_error_on_hello_send_poisons_the_session() {
    let clock = FakeClock::at(0);
    let (session, _session_id) = fresh_session();
    let mut transport = FakeTransport::new(clock.clone(), Vec::new());
    // A broken pipe on the collector-first hello send is terminal (no
    // reconnect, no second hello).
    transport.fail_next_send = Some(DuplexIoError::Disconnected);
    let mut client = CollectorControlClient::new(
        &permit(),
        transport,
        clock.clone(),
        session,
        identity(),
        DuplexClientPolicy::DEFAULT,
    );
    let deadline = far_deadline(&clock);
    let err = client.perform_handshake(&permit(), deadline).unwrap_err();
    assert_eq!(
        err,
        ClientError::Control(ControlError::Io(std::io::ErrorKind::BrokenPipe))
    );
    assert_eq!(client.phase(), ClientPhase::Failed);
    assert_eq!(
        client.perform_handshake(&permit(), deadline),
        Err(ClientError::NotUsable)
    );
}

#[test]
fn terminal_transport_error_during_serve_poisons_the_session() {
    let clock = FakeClock::at(0);
    let (session, _session_id) = fresh_session();
    let mut transport = FakeTransport::new(clock.clone(), Vec::new());
    // A broken pipe on the first post-hello receive is terminal (no reconnect).
    transport.fail_next_recv = Some(DuplexIoError::Disconnected);
    let mut client = CollectorControlClient::new(
        &permit(),
        transport,
        clock.clone(),
        session,
        identity(),
        DuplexClientPolicy::DEFAULT,
    );
    let deadline = far_deadline(&clock);
    client.perform_handshake(&permit(), deadline).unwrap();
    let err = client.serve_next(deadline).unwrap_err();
    assert_eq!(
        err,
        ClientError::Control(ControlError::Io(std::io::ErrorKind::BrokenPipe))
    );
    assert_eq!(client.phase(), ClientPhase::Failed);
}

// --- deadline / clamp unit tests --------------------------------------------

#[test]
fn wait_named_pipe_timeout_rounds_up_and_clamps() {
    let clock = FakeClock::at(0);
    // 0.4 ms remaining rounds UP to 1 ms (never 0, which means wait-forever
    // server default).
    let deadline = MonotonicDeadline(MonotonicInstant(400_000));
    assert_eq!(deadline.wait_named_pipe_timeout_ms(clock.now()), Some(1));

    // 1 ns remaining still yields at least 1 ms.
    let deadline = MonotonicDeadline(MonotonicInstant(1));
    assert_eq!(deadline.wait_named_pipe_timeout_ms(clock.now()), Some(1));

    // Exactly reached / passed => None (do not call WaitNamedPipeW at all).
    let deadline = MonotonicDeadline(MonotonicInstant(0));
    assert_eq!(deadline.wait_named_pipe_timeout_ms(clock.now()), None);
    let deadline = MonotonicDeadline(MonotonicInstant(0));
    assert_eq!(
        deadline.wait_named_pipe_timeout_ms(MonotonicInstant(5)),
        None
    );

    // Huge remaining clamps to MAXDWORD-1, never MAXDWORD (wait-forever).
    let deadline = MonotonicDeadline(MonotonicInstant(u128::from(u64::MAX)));
    let ms = deadline.wait_named_pipe_timeout_ms(clock.now()).unwrap();
    assert_eq!(ms, u32::MAX - 1);
}

// --- connector retry-loop tests ---------------------------------------------

enum Scripted {
    // Advance the fake clock by `advance_ns` to model elapsed time DURING the
    // CreateFileW call, then return `outcome`. This lets a test cross the
    // deadline inside a successful open so the post-open recheck must discard it.
    Create {
        advance_ns: u128,
        outcome: CreateFileOutcome<u64>,
    },
    // Advance the fake clock by `advance_ns` to model elapsed wait time, then
    // return the given wait outcome.
    Wait {
        advance_ns: u128,
        outcome: WaitOutcome,
    },
}

struct ScriptedConnector {
    clock: FakeClock,
    steps: VecDeque<Scripted>,
    wait_timeouts: Vec<(u128, u32)>, // (call instant, timeout arg)
    create_instants: Vec<u128>,
    discarded: usize,
    next_handle: u64,
}

impl ScriptedConnector {
    fn new(clock: FakeClock, steps: Vec<Scripted>) -> Self {
        Self {
            clock,
            steps: steps.into(),
            wait_timeouts: Vec::new(),
            create_instants: Vec::new(),
            discarded: 0,
            next_handle: 1,
        }
    }
}

impl PipeConnector for ScriptedConnector {
    type Handle = u64;

    fn create_file(&mut self) -> CreateFileOutcome<u64> {
        self.create_instants.push(self.clock.now().0);
        match self.steps.pop_front() {
            Some(Scripted::Create {
                advance_ns,
                outcome,
            }) => {
                self.clock.advance(advance_ns);
                match outcome {
                    CreateFileOutcome::Connected(_) => {
                        let handle = self.next_handle;
                        self.next_handle += 1;
                        CreateFileOutcome::Connected(handle)
                    }
                    other => other,
                }
            }
            _ => panic!("expected a scripted create_file"),
        }
    }

    fn wait_named_pipe(&mut self, timeout_ms: u32) -> WaitOutcome {
        self.wait_timeouts.push((self.clock.now().0, timeout_ms));
        match self.steps.pop_front() {
            Some(Scripted::Wait {
                advance_ns,
                outcome,
            }) => {
                self.clock.advance(advance_ns);
                outcome
            }
            _ => panic!("expected a scripted wait_named_pipe"),
        }
    }

    fn discard(&mut self, _handle: u64) {
        self.discarded += 1;
    }
}

#[test]
fn connector_retries_busy_under_the_same_absolute_deadline() {
    let clock = FakeClock::at(0);
    let deadline = MonotonicDeadline::from_now(&clock, Duration::from_secs(30));
    let mut connector = ScriptedConnector::new(
        clock.clone(),
        vec![
            // Server pipe not there yet.
            Scripted::Create {
                advance_ns: 0,
                outcome: CreateFileOutcome::NotFound,
            },
            Scripted::Wait {
                advance_ns: 100_000_000, // 100 ms elapsed while waiting
                outcome: WaitOutcome::Signaled,
            },
            // Instance appeared but is busy (documented race after a good wait).
            Scripted::Create {
                advance_ns: 0,
                outcome: CreateFileOutcome::Busy,
            },
            Scripted::Wait {
                advance_ns: 100_000_000,
                outcome: WaitOutcome::Signaled,
            },
            Scripted::Create {
                advance_ns: 0,
                outcome: CreateFileOutcome::Connected(0),
            },
        ],
    );

    let handle = open_client_control_pipe(&test_duplex_client_permit(), &mut connector, &clock, deadline).unwrap();
    assert_eq!(handle, 1); // exactly one handle published
    assert_eq!(connector.discarded, 0);
    // Each WaitNamedPipeW timeout equals the checked remaining budget at its call
    // instant: 30000 ms at t=0, then 29900 ms at t=100 ms. A mutant that reset
    // the budget after connecting would record a larger second value.
    assert_eq!(connector.wait_timeouts, vec![(0, 30_000), (100_000_000, 29_900)]);
}

#[test]
fn connector_expires_at_the_original_deadline_with_no_post_deadline_calls() {
    let clock = FakeClock::at(0);
    let deadline = MonotonicDeadline::from_now(&clock, Duration::from_secs(30));
    let mut connector = ScriptedConnector::new(
        clock.clone(),
        vec![
            Scripted::Create {
                advance_ns: 0,
                outcome: CreateFileOutcome::Busy,
            },
            Scripted::Wait {
                advance_ns: 20_000_000_000, // 20 s
                outcome: WaitOutcome::TimedOut,
            },
            Scripted::Create {
                advance_ns: 0,
                outcome: CreateFileOutcome::Busy,
            },
            Scripted::Wait {
                advance_ns: 20_000_000_000, // crosses 30 s -> now 40 s
                outcome: WaitOutcome::TimedOut,
            },
        ],
    );

    let err = open_client_control_pipe(&test_duplex_client_permit(), &mut connector, &clock, deadline).unwrap_err();
    assert_eq!(err, DuplexConnectError::DeadlineExpired);
    // No handle was ever published.
    assert_eq!(connector.discarded, 0);
    // Every recorded call happened strictly before the 30 s deadline.
    assert!(connector.create_instants.iter().all(|&t| t < 30_000_000_000));
    assert!(connector
        .wait_timeouts
        .iter()
        .all(|&(t, _)| t < 30_000_000_000));
    // The second wait was clipped to the remaining 10 s, never the full budget.
    assert_eq!(connector.wait_timeouts[1], (20_000_000_000, 10_000));
}

#[test]
fn connector_discards_a_handle_first_observed_after_the_deadline() {
    let clock = FakeClock::at(0);
    let deadline = MonotonicDeadline::from_now(&clock, Duration::from_secs(30));
    let mut connector = ScriptedConnector::new(
        clock.clone(),
        vec![
            Scripted::Create {
                advance_ns: 0,
                outcome: CreateFileOutcome::NotFound,
            },
            Scripted::Wait {
                advance_ns: 29_900_000_000, // wait ends just before the deadline
                outcome: WaitOutcome::Signaled,
            },
            // CreateFileW succeeds but the clock crosses the deadline DURING the
            // open; the post-open recheck must discard the late candidate rather
            // than publish it.
            Scripted::Create {
                advance_ns: 200_000_000, // -> 30.1 s, past the 30 s deadline
                outcome: CreateFileOutcome::Connected(0),
            },
        ],
    );
    let err = open_client_control_pipe(&test_duplex_client_permit(), &mut connector, &clock, deadline).unwrap_err();
    assert_eq!(err, DuplexConnectError::DeadlineExpired);
    assert_eq!(connector.discarded, 1);
}

#[test]
fn connector_expires_before_calling_wait_when_no_positive_budget_remains() {
    let clock = FakeClock::at(0);
    let deadline = MonotonicDeadline::from_now(&clock, Duration::from_secs(30));
    let mut connector = ScriptedConnector::new(
        clock.clone(),
        vec![
            // The open itself consumes the whole budget: CreateFileW returns Busy
            // exactly at the deadline. The driver must NOT call WaitNamedPipeW
            // (0 would select the server default, MAXDWORD would wait forever).
            Scripted::Create {
                advance_ns: 30_000_000_000,
                outcome: CreateFileOutcome::Busy,
            },
        ],
    );
    let err = open_client_control_pipe(&test_duplex_client_permit(), &mut connector, &clock, deadline).unwrap_err();
    assert_eq!(err, DuplexConnectError::DeadlineExpired);
    assert!(connector.wait_timeouts.is_empty());
    // CreateFileW was called exactly once, strictly before the deadline.
    assert_eq!(connector.create_instants, vec![0]);
}

// --- bounded worker join-or-quarantine --------------------------------------

#[test]
fn bounded_worker_joins_a_fast_completion() {
    let clock = FakeClock::at(0);
    let deadline = MonotonicDeadline::from_now(&clock, Duration::from_secs(30));
    let outcome = run_bounded_worker(&test_duplex_client_permit(), &clock, deadline, || 7u32);
    match outcome {
        BoundedWorkerOutcome::Completed(value) => assert_eq!(value, 7),
        BoundedWorkerOutcome::Quarantined => panic!("fast worker must be joined"),
    }
}

#[test]
fn bounded_worker_quarantines_a_worker_that_misses_the_deadline() {
    let before = quarantined_worker_count();
    let clock = FakeClock::at(0);
    // Deadline already reached at entry.
    let deadline = MonotonicDeadline(MonotonicInstant(0));
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let outcome = run_bounded_worker(&test_duplex_client_permit(), &clock, deadline, move || {
        // Block until the test releases us; the driver must not wait for this.
        let _ = release_rx.recv();
        0u32
    });
    assert!(matches!(outcome, BoundedWorkerOutcome::Quarantined));
    assert!(quarantined_worker_count() >= before + 1);
    // Let the quarantined worker exit cleanly (it is already off the driver's
    // critical path and was never joined here).
    drop(release_tx);
}

// --- pipe name --------------------------------------------------------------

#[test]
fn control_pipe_name_is_per_pid_and_token() {
    assert_eq!(
        control_pipe_name(4321, "tok-abc"),
        r"\\.\pipe\battle-plan_control_4321_tok-abc"
    );
}

// --- bounded post-cancel reap (HIGH 3 leak-vs-reclaim decision) --------------

#[test]
fn bounded_reap_reclaims_when_completion_is_observed_within_the_bound() {
    // The completion event becomes signaled on the 3rd poll: reclaim, no leak.
    let mut polls = 0;
    let outcome = bounded_reap(100, || {
        polls += 1;
        polls == 3
    });
    assert_eq!(outcome, ReapOutcome::Completed);
    assert_eq!(polls, 3);
}

#[test]
fn bounded_reap_leaks_when_a_pathological_driver_never_completes() {
    // A driver that never honors the cancel: after exactly `attempts` bounded
    // slices with no completion, the caller must leak (never block forever).
    let mut polls = 0;
    let outcome = bounded_reap(5, || {
        polls += 1;
        false
    });
    assert_eq!(outcome, ReapOutcome::LeakAbandoned);
    assert_eq!(polls, 5, "the reap is bounded to exactly `attempts` slices");
}

// --- TRANSPORT_STATUS / CAMPAIGN_IDENTITY client paths ----------------------

fn sample_sink_report(kind: SinkFailureKind) -> SinkFailureReport {
    let terminal = matches!(kind, SinkFailureKind::Terminal);
    SinkFailureReport {
        kind,
        requested: SinkTopology::from_bits(SinkTopology::FILE | SinkTopology::PIPE).unwrap(),
        opened: if terminal {
            SinkTopology::from_bits(SinkTopology::FILE | SinkTopology::PIPE).unwrap()
        } else {
            SinkTopology::from_bits(SinkTopology::FILE).unwrap()
        },
        failed: SinkTopology::from_bits(SinkTopology::PIPE).unwrap(),
        terminality: SinkTerminality::AttemptTerminal,
        error_class: SinkErrorClass::Win32,
        win32_error: Some(109),
        last_complete_sequence: terminal.then_some(8),
        failed_sequence: terminal.then_some(9),
        file_bytes_committed: if terminal { 4096 } else { 0 },
        pipe_bytes_committed: if terminal { 3100 } else { 0 },
    }
}

#[test]
fn report_sink_failure_frames_the_shared_codec_under_the_matching_kind() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let mut client = client(clock.clone(), session, Vec::new());
    let deadline = far_deadline(&clock);
    client.perform_handshake(&permit(), deadline).unwrap();

    let report = sample_sink_report(SinkFailureKind::Terminal);
    client.report_sink_failure(&report, deadline).unwrap();

    // The frame after the hello is a SinkTerminalFailure carrying the ONE shared
    // encode_sink_failure payload (decoded here through the same shared codec).
    let backend = client.backend();
    let frame = decode_frame(&backend.outbound[HELLO_WIRE_LEN..], session_id, 1).unwrap();
    assert_eq!(frame.kind, MessageKind::SinkTerminalFailure);
    let decoded = decode_sink_failure(SinkFailureKind::Terminal, &frame.payload).unwrap();
    assert_eq!(decoded, report);

    // Setup failures map to the SinkSetupFailure envelope kind.
    let setup = sample_sink_report(SinkFailureKind::Setup);
    client.report_sink_failure(&setup, deadline).unwrap();
    let backend = client.backend();
    let base = HELLO_WIRE_LEN + CONTROL_HEADER_LEN + 64;
    let frame = decode_frame(&backend.outbound[base..], session_id, 2).unwrap();
    assert_eq!(frame.kind, MessageKind::SinkSetupFailure);
}

fn sample_identity_failure() -> IdentityFailureV1 {
    IdentityFailureV1 {
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
    }
}

#[test]
fn identity_failure_send_and_ack_receive_use_the_shared_codec() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();

    // The recorder acks a previously-sent IdentityFailure at its sequence 0.
    let ack = IdentityFailureAckV1 {
        attempt_id: 77,
        logical_id: Some(5),
        disposition: IdentityFailureAckDisposition::DurablyRecorded,
        error: 0,
    };
    let inbound = recorder_frame(
        session_id,
        0,
        MessageKind::IdentityFailureAck,
        &encode_identity_failure_ack(ack).unwrap(),
    );
    let mut client = client(clock.clone(), session, inbound);
    let deadline = far_deadline(&clock);
    client.perform_handshake(&permit(), deadline).unwrap();

    let failure = sample_identity_failure();
    client.report_identity_failure(&failure, deadline).unwrap();

    // The sent frame is an IdentityFailure carrying the shared CIF1 payload.
    {
        let backend = client.backend();
        let frame = decode_frame(&backend.outbound[HELLO_WIRE_LEN..], session_id, 1).unwrap();
        assert_eq!(frame.kind, MessageKind::IdentityFailure);
        assert_eq!(decode_identity_failure(&frame.payload).unwrap(), failure);
    }

    // The client surfaces the recorder's CIA1 ack via serve_next.
    let event = client.serve_next(deadline).unwrap();
    assert_eq!(event, ClientEvent::IdentityFailureAcked(ack));
}

#[test]
fn pre_hello_sink_setup_failure_is_sent_as_frame_zero() {
    // A sink SETUP failure during setup (before the hello can be sent) is a
    // valid FIRST frame, mirroring the host's recv_handshake_until pre-hello
    // window. The client emits it at sequence 0 and then terminates.
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let mut client = client(clock.clone(), session, Vec::new());
    let deadline = far_deadline(&clock);
    let report = sample_sink_report(SinkFailureKind::Setup);
    client.report_sink_failure(&report, deadline).unwrap();

    // Frame 0 (no hello precedes it) is a SinkSetupFailure carrying the shared
    // codec payload.
    let backend = client.backend();
    let frame = decode_frame(&backend.outbound, session_id, 0).unwrap();
    assert_eq!(frame.kind, MessageKind::SinkSetupFailure);
    assert_eq!(
        decode_sink_failure(SinkFailureKind::Setup, &frame.payload).unwrap(),
        report
    );

    // The session is over: the client is terminal and refuses any further use,
    // including a hello or a second report.
    assert_eq!(client.phase(), ClientPhase::Failed);
    assert_eq!(
        client.perform_handshake(&permit(), deadline),
        Err(ClientError::NotUsable)
    );
}

#[test]
fn pre_hello_terminal_sink_failure_is_sent_as_frame_zero() {
    // A sink TERMINAL failure is ALSO a valid first frame: both sinks can open,
    // then the first sequence-0 write fails before the hello — the only valid
    // report is then Terminal. The host's pre-hello window
    // (is_pre_hello_receivable_sink_failure) admits both sink kinds, and the sink
    // codec permits Terminal on sequence 0 with no prior completion. The client
    // emits it at sequence 0 and then terminates.
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let mut client = client(clock.clone(), session, Vec::new());
    let deadline = far_deadline(&clock);
    let report = sample_sink_report(SinkFailureKind::Terminal);
    client.report_sink_failure(&report, deadline).unwrap();

    let backend = client.backend();
    let frame = decode_frame(&backend.outbound, session_id, 0).unwrap();
    assert_eq!(frame.kind, MessageKind::SinkTerminalFailure);
    assert_eq!(
        decode_sink_failure(SinkFailureKind::Terminal, &frame.payload).unwrap(),
        report
    );
    assert_eq!(client.phase(), ClientPhase::Failed);
}

#[test]
fn pre_hello_identity_failure_refuses_before_handshake() {
    let clock = FakeClock::at(0);
    let (session, _session_id) = fresh_session();
    let mut client = client(clock.clone(), session, Vec::new());
    let deadline = far_deadline(&clock);
    // CAMPAIGN_IDENTITY has no pre-hello window (it requires an admitted
    // capability), so an identity failure before the hello is refused, nothing
    // sent, and the client stays in Handshaking (unlike a sink failure, which is
    // a valid terminal first frame).
    assert_eq!(
        client.report_identity_failure(&sample_identity_failure(), deadline),
        Err(ClientError::NotUsable)
    );
    assert!(client.backend().outbound.is_empty());
    assert_eq!(client.phase(), ClientPhase::Handshaking);
}
