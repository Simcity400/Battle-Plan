use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use compact_contract::control_transport::payloads::{
    decode_collector_hello_with_capabilities, decode_heartbeat_proof, encode_heartbeat_challenge,
    encode_recorder_hello, HeartbeatChallengeV1, RecorderHelloV1,
    HeartbeatState, COLLECTOR_HELLO_WITH_CAPABILITIES_LEN,
};
use compact_contract::control_transport::{
    claim_session_capability, decode_frame, encode_frame, ControlCapabilities, ControlError,
    MessageKind, SessionCapability, SessionId, CONTROL_HEADER_LEN, CONTROL_VERSION,
};
use compact_contract::early_render_content_manifest::{
    encode_early_render_content_manifest_chunk, encode_early_render_content_manifest_commit,
    EarlyRenderContentManifestChunkV1, EarlyRenderContentManifestCommitV1,
};
use compact_contract::early_render_control::{
    canonical_early_diagnostic_liveness_response_binding_bytes,
    decode_early_diagnostic_liveness_response, decode_early_first_game_d3d,
    encode_early_commit_capture_receipt, encode_early_diagnostic_end,
    encode_early_diagnostic_liveness_request, encode_early_render_launch_bind_v2,
    EarlyCommitCaptureReceiptV1, EarlyDiagnosticEndV1, EarlyDiagnosticLivenessRequestV1,
    EarlyDiagnosticLivenessResponseV1, EarlyFirstGameD3dV1, EarlyHookReadyV1,
    EarlyRenderLaunchBindV2, EARLY_DIAGNOSTIC_END_RECORDER_CANCEL, EARLY_DIAGNOSTIC_MIN_HOLD_MS,
    EARLY_HOOK_COMPLETENESS_MASK, EARLY_STAGED_HOOK_COUNT, EARLY_STAGED_HOOK_GENERATION,
};
use compact_contract::early_render_runtime_dlc::{
    canonical_runtime_dlc_roster_sha256, decode_early_render_runtime_dlc_roster,
    encode_early_render_runtime_dlc_roster, EarlyRenderRuntimeDlcRosterV1,
    A729_ACTIVE_DLC_ENUMERATOR_RVA, A729_DLC_MANAGER_GETTER_RVA, A729_STRING_VECTOR_TEARDOWN_RVA,
};
use compact_contract::early_render_startup_failure::{
    decode_early_render_startup_failure, EarlyRenderStartupPhase,
};
use compact_contract::early_render_vfs::{
    canonical_early_render_vfs_index_bytes, encode_early_render_vfs_index_chunk,
    encode_early_render_vfs_index_commit, EarlyRenderVfsIndexChunkV2, EarlyRenderVfsIndexCommitV2,
    EarlyRenderVfsLayerKind, EarlyRenderVfsWinnerRowV2,
};
use compact_contract::early_render_vfs_payload::{
    decode_early_render_vfs_payload_request, encode_early_render_vfs_payload_chunk,
    encode_early_render_vfs_payload_commit, EarlyRenderVfsPayloadChunkV1,
    EarlyRenderVfsPayloadCommitV1,
};
use compact_contract::first_compact_ack::{encode_first_compact_frame_ack, FirstCompactFrameAckV1};
use compact_contract::identity_control::{
    decode_identity_failure, encode_identity_failure_ack, IdentityFailureAckDisposition,
    IdentityFailureAckV1, IdentityFailureReason, IdentityFailureV1,
};
use compact_contract::sink_failure::{
    decode_sink_failure, SinkErrorClass, SinkFailureKind, SinkFailureReport, SinkTerminality,
    SinkTopology,
};
use compact_contract::stream_validator::StreamIdentityMode;
use portable_content_manifest::{
    ContentLayerKind, ContentManifest, LayerStorageKind, RecordedContentLayer, SourceGameIdentity,
    SourceProfileIdentity,
};
use sha2::{Digest, Sha256};

use super::*;

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
            return Err(if self.inbound.is_empty() {
                if self.peer_closed {
                    DuplexIoError::PeerClosed
                } else {
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

const TEST_PID: u32 = 0x0000_1234;
const TEST_CHALLENGE: [u8; 16] = [0x11; 16];
const TEST_DIAGNOSTIC_COM_GROUP_MASK: u64 = 0x01;

fn identity() -> CollectorIdentity {
    CollectorIdentity::from_attach(
        TEST_PID,
        0x0102_0304_0506_0708,
        7,
        [0xab; 32],
        TEST_CHALLENGE,
    )
}

fn permit() -> DuplexClientProductPermit {
    test_duplex_client_permit()
}

const HELLO_WIRE_LEN: usize = CONTROL_HEADER_LEN + COLLECTOR_HELLO_WITH_CAPABILITIES_LEN;

fn fresh_session() -> (SessionCapability, SessionId) {
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut bytes = [0x3c; 16];
    bytes[8..].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
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

fn vfs_row(path: &str) -> EarlyRenderVfsWinnerRowV2 {
    let payload_sha256: [u8; 32] = Sha256::digest(b"data").into();
    EarlyRenderVfsWinnerRowV2 {
        stored_game_relative_path: path.into(),
        normalized_game_relative_path: path.into(),
        winning_layer_ordinal: 1,
        layer_kind: EarlyRenderVfsLayerKind::Mod,
        layer_id: "mod/test.mod".into(),
        source_sha256: payload_sha256,
        payload_sha256,
        payload_bytes: 4,
        source_container_sha256: None,
        source_byte_offset: 0,
        source_byte_count: 4,
        source_range_sha256: payload_sha256,
    }
}

fn lower_sha256(value: &[u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn manifest_envelope_bytes(effective_vfs_manifest_sha256: [u8; 32]) -> Vec<u8> {
    let mut manifest = ContentManifest {
        witnessed_absent_content: Vec::new(),
        schema: portable_content_manifest::SCHEMA.into(),
        product_modset_sha256: "09".repeat(32),
        source_game: SourceGameIdentity {
            version: "1.19.2".into(),
            build: "a729".into(),
            executable_sha256: "03".repeat(32),
            build_profile: "hoi4-1.19.2-a729".into(),
        },
        source_profile: SourceProfileIdentity {
            profile_id: "test".into(),
            playset_id: "test".into(),
            selection_sha256: "0".repeat(64),
        },
        effective_vfs_manifest_sha256: lower_sha256(&effective_vfs_manifest_sha256),
        ordered_layers: vec![
            RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: false,
            },
            RecordedContentLayer {
                ordinal: 1,
                layer_kind: ContentLayerKind::Mod,
                layer_id: "mod/test.mod".into(),
                display_name: "test".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: false,
            },
        ],
        dlc_selection: Vec::new(),
        localisation_language: "english".into(),
        required_settings: BTreeMap::new(),
        recorder_local_settings: BTreeMap::new(),
        required_content: Vec::new(),
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest
        .with_computed_digest()
        .unwrap()
        .encode_json()
        .unwrap()
}

#[test]
fn early_launch_bind_rejects_foreign_process_nonce_before_ready() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let wire = EarlyRenderLaunchBindV2 {
        process_id: TEST_PID,
        process_start_nonce: identity().creation_filetime.wrapping_add(1),
        timeout_ms: 30_000,
        launch_manifest_sha256: [1; 32],
        effective_vfs_manifest_sha256: [2; 32],
        effective_vfs_index_sha256: [5; 32],
        effective_vfs_index_row_count: 1,
        effective_vfs_index_chunk_count: 1,
        content_manifest_bytes_sha256: [6; 32],
        content_manifest_byte_count: 2,
        content_manifest_chunk_count: 1,
        executable_sha256: [3; 32],
        collector_sha256: [0xab; 32],
    };
    let inbound = recorder_frame(
        session_id,
        0,
        MessageKind::EarlyRenderLaunchBind,
        &encode_early_render_launch_bind_v2(&wire).unwrap(),
    );
    let permit = early_render_duplex_product_permit();
    let transport = FakeTransport::new(clock.clone(), inbound);
    let mut client = CollectorControlClient::new_early_render(
        &permit,
        transport,
        clock.clone(),
        session,
        identity(),
        DuplexClientPolicy::DEFAULT,
    );
    client
        .perform_early_render_handshake(&permit, far_deadline(&clock))
        .unwrap();
    assert!(matches!(
        client.receive_early_render_launch_bind(far_deadline(&clock)),
        Err(ClientError::EarlyRenderBinding)
    ));
    assert_eq!(client.phase(), ClientPhase::Failed);
}

#[test]
fn startup_failure_reports_exact_post_bind_error_after_local_client_failure() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let test_identity = CollectorIdentity::from_attach(
        TEST_PID,
        0x0102_0304_0506_0708,
        7,
        [0xab; 32],
        *session_id.as_bytes(),
    );
    let bind = EarlyRenderLaunchBindV2 {
        process_id: TEST_PID,
        process_start_nonce: test_identity.creation_filetime,
        timeout_ms: 30_000,
        launch_manifest_sha256: [1; 32],
        effective_vfs_manifest_sha256: [2; 32],
        effective_vfs_index_sha256: [5; 32],
        effective_vfs_index_row_count: 1,
        effective_vfs_index_chunk_count: 1,
        content_manifest_bytes_sha256: [6; 32],
        content_manifest_byte_count: 2,
        content_manifest_chunk_count: 1,
        executable_sha256: [3; 32],
        collector_sha256: [0xab; 32],
    };
    let bind_payload = encode_early_render_launch_bind_v2(&bind).unwrap();
    let expected_bind_sha256: [u8; 32] = Sha256::digest(&bind_payload).into();
    let inbound = recorder_frame(
        session_id,
        0,
        MessageKind::EarlyRenderLaunchBind,
        &bind_payload,
    );
    let permit = early_render_duplex_product_permit();
    let transport = FakeTransport::new(clock.clone(), inbound);
    let mut client = CollectorControlClient::new_early_render(
        &permit,
        transport,
        clock.clone(),
        session,
        test_identity,
        DuplexClientPolicy::DEFAULT,
    );
    let deadline = far_deadline(&clock);
    client
        .perform_early_render_handshake(&permit, deadline)
        .unwrap();
    client.receive_early_render_launch_bind(deadline).unwrap();
    let _: Result<(), ClientError> = client.fail(ClientError::EarlyRenderBinding);

    let exact_error = "bind A25 common-GUI effective-VFS definition failed: exact test failure";
    client
        .report_early_render_startup_failure(&permit, exact_error, deadline)
        .unwrap();

    let outbound = &client.backend().outbound;
    let frame = decode_frame(&outbound[HELLO_WIRE_LEN..], session_id, 1).unwrap();
    assert_eq!(frame.kind, MessageKind::EarlyRenderStartupFailure);
    let report = decode_early_render_startup_failure(&frame.payload).unwrap();
    assert_eq!(report.process_id, TEST_PID);
    assert_eq!(report.process_start_nonce, test_identity.creation_filetime);
    assert_eq!(report.session_id, *session_id.as_bytes());
    assert_eq!(report.phase, EarlyRenderStartupPhase::PostBindPreFirst);
    assert_eq!(report.launch_bind_sha256, Some(expected_bind_sha256));
    assert_eq!(report.detail, exact_error);
    let expected_detail_sha256: [u8; 32] = Sha256::digest(exact_error.as_bytes()).into();
    assert_eq!(report.detail_sha256, expected_detail_sha256);
    assert_eq!(client.phase(), ClientPhase::Failed);
}

#[test]
fn early_vfs_index_rejects_foreign_bind_digest_before_ready() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let rows = vec![vfs_row("gfx/test.asset")];
    let index_sha256: [u8; 32] =
        Sha256::digest(canonical_early_render_vfs_index_bytes(&rows).unwrap()).into();
    let manifest_bytes = manifest_envelope_bytes([2; 32]);
    let manifest_sha256: [u8; 32] = Sha256::digest(&manifest_bytes).into();
    let bind = EarlyRenderLaunchBindV2 {
        process_id: TEST_PID,
        process_start_nonce: identity().creation_filetime,
        timeout_ms: 30_000,
        launch_manifest_sha256: [1; 32],
        effective_vfs_manifest_sha256: [2; 32],
        effective_vfs_index_sha256: index_sha256,
        effective_vfs_index_row_count: 1,
        effective_vfs_index_chunk_count: 1,
        content_manifest_bytes_sha256: manifest_sha256,
        content_manifest_byte_count: manifest_bytes.len() as u64,
        content_manifest_chunk_count: 1,
        executable_sha256: [3; 32],
        collector_sha256: [0xab; 32],
    };
    let bind_payload = encode_early_render_launch_bind_v2(&bind).unwrap();
    let launch_bind_sha256: [u8; 32] = Sha256::digest(&bind_payload).into();
    let manifest_chunk = EarlyRenderContentManifestChunkV1 {
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        session_id: *session_id.as_bytes(),
        launch_bind_sha256,
        effective_vfs_manifest_sha256: bind.effective_vfs_manifest_sha256,
        effective_vfs_index_sha256: bind.effective_vfs_index_sha256,
        content_manifest_bytes_sha256: manifest_sha256,
        chunk_index: 0,
        chunk_count: 1,
        byte_offset: 0,
        total_bytes: manifest_bytes.len() as u64,
        bytes: manifest_bytes,
    };
    let manifest_commit = EarlyRenderContentManifestCommitV1 {
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        session_id: *session_id.as_bytes(),
        launch_bind_sha256,
        effective_vfs_manifest_sha256: bind.effective_vfs_manifest_sha256,
        effective_vfs_index_sha256: bind.effective_vfs_index_sha256,
        content_manifest_bytes_sha256: manifest_sha256,
        chunk_count: 1,
        total_bytes: manifest_chunk.total_bytes,
    };
    let chunk = EarlyRenderVfsIndexChunkV2 {
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        session_id: *session_id.as_bytes(),
        launch_bind_sha256: [0x44; 32],
        index_sha256,
        chunk_index: 0,
        chunk_count: 1,
        first_row_index: 0,
        total_row_count: 1,
        rows,
    };
    let mut inbound = recorder_frame(
        session_id,
        0,
        MessageKind::EarlyRenderLaunchBind,
        &bind_payload,
    );
    inbound.extend(recorder_frame(
        session_id,
        1,
        MessageKind::EarlyRenderContentManifestChunk,
        &encode_early_render_content_manifest_chunk(&manifest_chunk).unwrap(),
    ));
    inbound.extend(recorder_frame(
        session_id,
        2,
        MessageKind::EarlyRenderContentManifestCommit,
        &encode_early_render_content_manifest_commit(&manifest_commit).unwrap(),
    ));
    inbound.extend(recorder_frame(
        session_id,
        3,
        MessageKind::EarlyRenderVfsIndexChunk,
        &encode_early_render_vfs_index_chunk(&chunk).unwrap(),
    ));
    let permit = early_render_duplex_product_permit();
    let transport = FakeTransport::new(clock.clone(), inbound);
    let mut client = CollectorControlClient::new_early_render(
        &permit,
        transport,
        clock.clone(),
        session,
        identity(),
        DuplexClientPolicy::DEFAULT,
    );
    let deadline = far_deadline(&clock);
    client
        .perform_early_render_handshake(&permit, deadline)
        .unwrap();
    let observed = client.receive_early_render_launch_bind(deadline).unwrap();
    client
        .receive_early_render_content_manifest(&permit, &observed, deadline)
        .unwrap();
    assert!(matches!(
        client.receive_early_render_vfs_index(&permit, &observed, deadline),
        Err(ClientError::EarlyRenderBinding)
    ));
    assert_eq!(client.phase(), ClientPhase::Failed);
}

#[test]
fn early_transaction_admits_commit_after_the_authenticated_bind_sequence() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let rows = vec![vfs_row("gfx/test.asset")];
    let index_sha256: [u8; 32] =
        Sha256::digest(canonical_early_render_vfs_index_bytes(&rows).unwrap()).into();
    let manifest_bytes = manifest_envelope_bytes([2; 32]);
    let manifest_sha256: [u8; 32] = Sha256::digest(&manifest_bytes).into();
    let bind = EarlyRenderLaunchBindV2 {
        process_id: TEST_PID,
        process_start_nonce: identity().creation_filetime,
        timeout_ms: 17_000,
        launch_manifest_sha256: [1; 32],
        effective_vfs_manifest_sha256: [2; 32],
        effective_vfs_index_sha256: index_sha256,
        effective_vfs_index_row_count: 1,
        effective_vfs_index_chunk_count: 1,
        content_manifest_bytes_sha256: manifest_sha256,
        content_manifest_byte_count: manifest_bytes.len() as u64,
        content_manifest_chunk_count: 1,
        executable_sha256: [3; 32],
        collector_sha256: [0xab; 32],
    };
    let commit = EarlyCommitCaptureReceiptV1 {
        process_id: TEST_PID,
        process_start_nonce: identity().creation_filetime,
        hook_epoch_nonce: 9,
        receipt_sha256: [6; 32],
    };
    let bind_payload = encode_early_render_launch_bind_v2(&bind).unwrap();
    let launch_bind_sha256: [u8; 32] = Sha256::digest(&bind_payload).into();
    let manifest_chunk = EarlyRenderContentManifestChunkV1 {
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        session_id: *session_id.as_bytes(),
        launch_bind_sha256,
        effective_vfs_manifest_sha256: bind.effective_vfs_manifest_sha256,
        effective_vfs_index_sha256: bind.effective_vfs_index_sha256,
        content_manifest_bytes_sha256: manifest_sha256,
        chunk_index: 0,
        chunk_count: 1,
        byte_offset: 0,
        total_bytes: manifest_bytes.len() as u64,
        bytes: manifest_bytes,
    };
    let manifest_commit = EarlyRenderContentManifestCommitV1 {
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        session_id: *session_id.as_bytes(),
        launch_bind_sha256,
        effective_vfs_manifest_sha256: bind.effective_vfs_manifest_sha256,
        effective_vfs_index_sha256: bind.effective_vfs_index_sha256,
        content_manifest_bytes_sha256: manifest_sha256,
        chunk_count: 1,
        total_bytes: manifest_chunk.total_bytes,
    };
    let chunk = EarlyRenderVfsIndexChunkV2 {
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        session_id: *session_id.as_bytes(),
        launch_bind_sha256,
        index_sha256,
        chunk_index: 0,
        chunk_count: 1,
        first_row_index: 0,
        total_row_count: 1,
        rows: rows.clone(),
    };
    let index_commit = EarlyRenderVfsIndexCommitV2 {
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        session_id: *session_id.as_bytes(),
        launch_bind_sha256,
        index_sha256,
        chunk_count: 1,
        total_row_count: 1,
    };
    let payload_response = EarlyRenderVfsPayloadChunkV1 {
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        session_id: *session_id.as_bytes(),
        launch_bind_sha256,
        effective_vfs_manifest_sha256: bind.effective_vfs_manifest_sha256,
        effective_vfs_index_sha256: bind.effective_vfs_index_sha256,
        request_id: 1,
        payload_offset: 0,
        payload_count: 4,
        expected_payload_range_sha256: rows[0].payload_sha256,
        chunk_index: 0,
        chunk_count: 1,
        chunk_offset: 0,
        bytes: b"data".to_vec(),
    };
    let payload_commit = EarlyRenderVfsPayloadCommitV1 {
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        session_id: *session_id.as_bytes(),
        launch_bind_sha256,
        effective_vfs_manifest_sha256: bind.effective_vfs_manifest_sha256,
        effective_vfs_index_sha256: bind.effective_vfs_index_sha256,
        request_id: 1,
        payload_offset: 0,
        payload_count: 4,
        payload_range_sha256: rows[0].payload_sha256,
        chunk_count: 1,
    };
    let mut inbound = recorder_frame(
        session_id,
        0,
        MessageKind::EarlyRenderLaunchBind,
        &bind_payload,
    );
    inbound.extend(recorder_frame(
        session_id,
        1,
        MessageKind::EarlyRenderContentManifestChunk,
        &encode_early_render_content_manifest_chunk(&manifest_chunk).unwrap(),
    ));
    inbound.extend(recorder_frame(
        session_id,
        2,
        MessageKind::EarlyRenderContentManifestCommit,
        &encode_early_render_content_manifest_commit(&manifest_commit).unwrap(),
    ));
    inbound.extend(recorder_frame(
        session_id,
        3,
        MessageKind::EarlyRenderVfsIndexChunk,
        &encode_early_render_vfs_index_chunk(&chunk).unwrap(),
    ));
    inbound.extend(recorder_frame(
        session_id,
        4,
        MessageKind::EarlyRenderVfsIndexCommit,
        &encode_early_render_vfs_index_commit(&index_commit).unwrap(),
    ));
    inbound.extend(recorder_frame(
        session_id,
        5,
        MessageKind::EarlyRenderCommitReceipt,
        &encode_early_commit_capture_receipt(&commit).unwrap(),
    ));
    inbound.extend(recorder_frame(
        session_id,
        6,
        MessageKind::EarlyRenderVfsPayloadChunk,
        &encode_early_render_vfs_payload_chunk(&payload_response).unwrap(),
    ));
    inbound.extend(recorder_frame(
        session_id,
        7,
        MessageKind::EarlyRenderVfsPayloadCommit,
        &encode_early_render_vfs_payload_commit(&payload_commit).unwrap(),
    ));
    let permit = early_render_duplex_product_permit();
    let transport = FakeTransport::new(clock.clone(), inbound);
    let mut client = CollectorControlClient::new_early_render(
        &permit,
        transport,
        clock.clone(),
        session,
        identity(),
        DuplexClientPolicy::DEFAULT,
    );
    let deadline =
        MonotonicDeadline::from_now(&clock, Duration::from_millis(bind.timeout_ms.into()));
    client
        .perform_early_render_handshake(&permit, deadline)
        .unwrap();
    let observed = client.receive_early_render_launch_bind(deadline).unwrap();
    assert_eq!(observed.timeout_ms, bind.timeout_ms);
    let manifest = client
        .receive_early_render_content_manifest(&permit, &observed, deadline)
        .unwrap();
    assert!(manifest.manifest.required_content.is_empty());
    let admitted = client
        .receive_early_render_vfs_index(&permit, &observed, deadline)
        .unwrap();
    assert_eq!(admitted.ordered_rows.len(), 1);
    let ready = EarlyHookReadyV1 {
        launch_manifest_sha256: bind.launch_manifest_sha256,
        effective_vfs_manifest_sha256: bind.effective_vfs_manifest_sha256,
        effective_vfs_index_sha256: bind.effective_vfs_index_sha256,
        effective_vfs_index_row_count: bind.effective_vfs_index_row_count,
        effective_vfs_index_chunk_count: bind.effective_vfs_index_chunk_count,
        content_manifest_bytes_sha256: bind.content_manifest_bytes_sha256,
        content_manifest_byte_count: bind.content_manifest_byte_count,
        content_manifest_chunk_count: bind.content_manifest_chunk_count,
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        executable_sha256: bind.executable_sha256,
        collector_sha256: bind.collector_sha256,
        hook_epoch_nonce: 9,
        hook_set_sha256: [5; 32],
        hook_ready_sequence: 10,
        hook_ready_qpc: 11,
        game_d3d_creation_count_at_hook_ready: 0,
        preexisting_game_d3d_object_count: 0,
        completeness_bits: EARLY_HOOK_COMPLETENESS_MASK,
    };
    client.send_early_render_ready(&ready, deadline).unwrap();
    let first = EarlyFirstGameD3dV1 {
        launch_manifest_sha256: bind.launch_manifest_sha256,
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        hook_epoch_nonce: ready.hook_epoch_nonce,
        game_device_ordinal: 1,
        first_creation_sequence: 13,
        creation_qpc: 12,
        staged_hook_generation: EARLY_STAGED_HOOK_GENERATION,
        staged_hook_count: EARLY_STAGED_HOOK_COUNT,
        diagnostic_stage_probe: false,
        staged_hook_group_mask:
            compact_contract::early_render_control::EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK,
        staged_hook_set_sha256: [8; 32],
        game_device_ptr: 0x1000,
        immediate_context_ptr: 0x2000,
        collector_null_probe_excluded: true,
    };
    client
        .send_early_render_first_game_d3d(&first, deadline)
        .unwrap();
    let raw_names = vec!["Together for Victory".to_string()];
    let runtime_dlc = EarlyRenderRuntimeDlcRosterV1 {
        launch_manifest_sha256: bind.launch_manifest_sha256,
        effective_vfs_manifest_sha256: bind.effective_vfs_manifest_sha256,
        process_id: bind.process_id,
        process_start_nonce: bind.process_start_nonce,
        hook_epoch_nonce: ready.hook_epoch_nonce,
        staged_hook_generation: first.staged_hook_generation,
        game_device_ptr: first.game_device_ptr,
        manager_getter_rva: A729_DLC_MANAGER_GETTER_RVA,
        active_dlc_enumerator_rva: A729_ACTIVE_DLC_ENUMERATOR_RVA,
        string_vector_teardown_rva: A729_STRING_VECTOR_TEARDOWN_RVA,
        sample_count: 2,
        double_stable: true,
        roster_count: raw_names.len() as u32,
        raw_name_roster_sha256: canonical_runtime_dlc_roster_sha256(&raw_names),
        raw_names,
    };
    client
        .send_early_render_runtime_dlc_roster(&runtime_dlc, deadline)
        .unwrap();
    assert_eq!(
        client
            .receive_early_render_commit(ready.hook_epoch_nonce, deadline)
            .unwrap(),
        commit
    );
    let payload = client
        .request_early_render_vfs_payload_range(
            &permit,
            &observed,
            &admitted,
            &admitted.ordered_rows[0],
            0,
            4,
            admitted.ordered_rows[0].payload_sha256,
            deadline,
        )
        .unwrap();
    assert_eq!(payload.bytes, b"data");
    let outbound = &client.backend().outbound;
    let hello = decode_frame(&outbound[..HELLO_WIRE_LEN], session_id, 0).unwrap();
    assert_eq!(hello.kind, MessageKind::CollectorHello);
    let ready_end = HELLO_WIRE_LEN + CONTROL_HEADER_LEN + 304;
    let ready_frame = decode_frame(&outbound[HELLO_WIRE_LEN..ready_end], session_id, 1).unwrap();
    assert_eq!(ready_frame.kind, MessageKind::EarlyRenderReady);
    let first_payload_len = encode_early_first_game_d3d(&first).unwrap().len();
    let first_end = ready_end + CONTROL_HEADER_LEN + first_payload_len;
    let first_frame = decode_frame(&outbound[ready_end..first_end], session_id, 2).unwrap();
    assert_eq!(first_frame.kind, MessageKind::EarlyRenderFirstGameD3d);
    assert_eq!(
        decode_early_first_game_d3d(&first_frame.payload)
            .unwrap()
            .first_creation_sequence,
        13
    );
    let roster_payload_len = encode_early_render_runtime_dlc_roster(&runtime_dlc)
        .unwrap()
        .len();
    let roster_end = first_end + CONTROL_HEADER_LEN + roster_payload_len;
    let roster_frame = decode_frame(&outbound[first_end..roster_end], session_id, 3).unwrap();
    assert_eq!(roster_frame.kind, MessageKind::EarlyRenderRuntimeDlcRoster);
    assert_eq!(
        decode_early_render_runtime_dlc_roster(&roster_frame.payload).unwrap(),
        runtime_dlc
    );
    let request_frame = decode_frame(&outbound[roster_end..], session_id, 4).unwrap();
    assert_eq!(
        request_frame.kind,
        MessageKind::EarlyRenderVfsPayloadRequest
    );
    assert_eq!(
        decode_early_render_vfs_payload_request(&request_frame.payload)
            .unwrap()
            .winner,
        admitted.ordered_rows[0]
    );
}

fn diagnostic_binding(session_id: SessionId) -> EarlyRenderControlBinding {
    EarlyRenderControlBinding {
        process_id: TEST_PID,
        process_start_nonce: identity().creation_filetime,
        timeout_ms: 120_000,
        launch_manifest_sha256: [1; 32],
        effective_vfs_manifest_sha256: [2; 32],
        effective_vfs_index_sha256: [3; 32],
        effective_vfs_index_row_count: 1,
        effective_vfs_index_chunk_count: 1,
        content_manifest_bytes_sha256: [4; 32],
        content_manifest_byte_count: 1,
        content_manifest_chunk_count: 1,
        session_id: *session_id.as_bytes(),
        launch_bind_sha256: [5; 32],
        executable_sha256: [6; 32],
        collector_sha256: [0xab; 32],
    }
}

fn diagnostic_first(binding: &EarlyRenderControlBinding) -> EarlyFirstGameD3dV1 {
    EarlyFirstGameD3dV1 {
        launch_manifest_sha256: binding.launch_manifest_sha256,
        process_id: binding.process_id,
        process_start_nonce: binding.process_start_nonce,
        hook_epoch_nonce: 9,
        game_device_ordinal: 1,
        first_creation_sequence: 13,
        creation_qpc: 12,
        staged_hook_generation: EARLY_STAGED_HOOK_GENERATION,
        staged_hook_count: 19,
        diagnostic_stage_probe: true,
        staged_hook_group_mask: TEST_DIAGNOSTIC_COM_GROUP_MASK,
        staged_hook_set_sha256: [8; 32],
        game_device_ptr: 0x1000,
        immediate_context_ptr: 0x2000,
        collector_null_probe_excluded: true,
    }
}

fn admitted_diagnostic_client(
    clock: FakeClock,
    session: SessionCapability,
    binding: EarlyRenderControlBinding,
    transport: FakeTransport,
) -> CollectorControlClient<FakeTransport, FakeClock> {
    let mut client = CollectorControlClient::new_inner(
        transport,
        clock,
        session,
        identity(),
        DuplexClientPolicy::DEFAULT,
        Some(binding),
        ControlCapabilities::early_render_duplex_v1(),
    );
    client.phase = ClientPhase::Attached;
    client.early_render_first_game_d3d_sent = true;
    client.early_render_diagnostic_stage_probe = true;
    client.early_render_runtime_dlc_sent = true;
    client
}

fn diagnostic_request(
    binding: &EarlyRenderControlBinding,
    first: &EarlyFirstGameD3dV1,
) -> EarlyDiagnosticLivenessRequestV1 {
    EarlyDiagnosticLivenessRequestV1 {
        session_id: binding.session_id,
        launch_manifest_sha256: binding.launch_manifest_sha256,
        process_id: binding.process_id,
        process_start_nonce: binding.process_start_nonce,
        hook_epoch_nonce: first.hook_epoch_nonce,
        game_device_ordinal: first.game_device_ordinal,
        first_creation_sequence: first.first_creation_sequence,
        creation_qpc: first.creation_qpc,
        staged_hook_generation: first.staged_hook_generation,
        staged_hook_count: first.staged_hook_count,
        staged_hook_group_mask: first.staged_hook_group_mask,
        staged_hook_set_sha256: first.staged_hook_set_sha256,
        game_device_ptr: first.game_device_ptr,
        immediate_context_ptr: first.immediate_context_ptr,
        required_hold_ms: EARLY_DIAGNOSTIC_MIN_HOLD_MS,
        request_nonce: [10; 16],
    }
}

fn diagnostic_sample(first: &EarlyFirstGameD3dV1) -> DiagnosticReadinessSample {
    DiagnosticReadinessSample {
        process_id: first.process_id,
        process_start_nonce: first.process_start_nonce,
        hook_epoch_nonce: first.hook_epoch_nonce,
        d3d_device_created_qpc: first.creation_qpc,
        first_creation_sequence: first.first_creation_sequence,
        qpc_frequency_hz: 10_000_000,
        staged_hook_generation: first.staged_hook_generation,
        staged_hook_count: first.staged_hook_count,
        diagnostic_stage_probe: true,
        staged_hook_group_mask: first.staged_hook_group_mask,
        staged_hook_set_sha256: first.staged_hook_set_sha256,
        game_device_ptr: first.game_device_ptr,
        immediate_context_ptr: first.immediate_context_ptr,
        sampled_qpc: first.creation_qpc + 2_100_000_000,
    }
}

fn diagnostic_response(
    request: &EarlyDiagnosticLivenessRequestV1,
    request_payload: &[u8],
    sample: &DiagnosticReadinessSample,
) -> EarlyDiagnosticLivenessResponseV1 {
    let mut response = EarlyDiagnosticLivenessResponseV1 {
        session_id: request.session_id,
        process_id: request.process_id,
        process_start_nonce: request.process_start_nonce,
        hook_epoch_nonce: request.hook_epoch_nonce,
        request_nonce: request.request_nonce,
        request_sha256: Sha256::digest(request_payload).into(),
        sampled_qpc: sample.sampled_qpc,
        qpc_frequency_hz: sample.qpc_frequency_hz,
        observed_hold_ms: EARLY_DIAGNOSTIC_MIN_HOLD_MS,
        staged_hook_generation: sample.staged_hook_generation,
        staged_hook_count: sample.staged_hook_count,
        staged_hook_group_mask: sample.staged_hook_group_mask,
        staged_hook_set_sha256: sample.staged_hook_set_sha256,
        game_device_ptr: sample.game_device_ptr,
        immediate_context_ptr: sample.immediate_context_ptr,
        response_sha256: [0; 32],
    };
    let binding = canonical_early_diagnostic_liveness_response_binding_bytes(&response).unwrap();
    response.response_sha256 = Sha256::digest(&binding).into();
    response
}

#[test]
fn diagnostic_hold_requires_request_fresh_sample_response_and_bound_cancel() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let binding = diagnostic_binding(session_id);
    let first = diagnostic_first(&binding);
    let request = diagnostic_request(&binding, &first);
    let request_payload = encode_early_diagnostic_liveness_request(&request).unwrap();
    let sample = diagnostic_sample(&first);
    let response = diagnostic_response(&request, &request_payload, &sample);
    let end = EarlyDiagnosticEndV1 {
        session_id: request.session_id,
        process_id: request.process_id,
        process_start_nonce: request.process_start_nonce,
        hook_epoch_nonce: request.hook_epoch_nonce,
        request_nonce: request.request_nonce,
        response_sha256: response.response_sha256,
        disposition: EARLY_DIAGNOSTIC_END_RECORDER_CANCEL,
    };
    let mut inbound = recorder_frame(
        session_id,
        0,
        MessageKind::EarlyRenderDiagnosticLivenessRequest,
        &request_payload,
    );
    inbound.extend_from_slice(&recorder_frame(
        session_id,
        1,
        MessageKind::EarlyRenderDiagnosticEnd,
        &encode_early_diagnostic_end(&end).unwrap(),
    ));
    let transport = FakeTransport::new(clock.clone(), inbound);
    let mut client = admitted_diagnostic_client(clock.clone(), session, binding.clone(), transport);
    let hold_started = clock.now();
    clock.advance(u128::from(EARLY_DIAGNOSTIC_MIN_HOLD_MS) * 1_000_000);
    let proof = client
        .complete_authenticated_diagnostic_hold(&binding, &first, hold_started, || {
            Ok(sample.clone())
        })
        .unwrap();
    assert_eq!(proof.observed_hold_ms, EARLY_DIAGNOSTIC_MIN_HOLD_MS);
    assert_eq!(proof.session_id, binding.session_id);
    assert_eq!(proof.staged_hook_set_sha256, first.staged_hook_set_sha256);
    assert_eq!(proof.request_sha256, response.request_sha256);
    assert_eq!(proof.response_sha256, response.response_sha256);
    let response_frame = decode_frame(&client.backend().outbound, session_id, 0).unwrap();
    assert_eq!(
        response_frame.kind,
        MessageKind::EarlyRenderDiagnosticLivenessResponse
    );
    assert_eq!(
        decode_early_diagnostic_liveness_response(&response_frame.payload).unwrap(),
        response
    );
    assert_eq!(client.phase(), ClientPhase::Stopped);
    assert!(!client.early_render_capture_committed);
}

#[test]
fn diagnostic_hold_rejects_commit_frame_without_opening_replay() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let binding = diagnostic_binding(session_id);
    let first = diagnostic_first(&binding);
    let inbound = recorder_frame(session_id, 0, MessageKind::EarlyRenderCommitReceipt, &[]);
    let transport = FakeTransport::new(clock.clone(), inbound);
    let mut client = admitted_diagnostic_client(clock.clone(), session, binding.clone(), transport);
    let hold_started = clock.now();
    clock.advance(u128::from(EARLY_DIAGNOSTIC_MIN_HOLD_MS) * 1_000_000);
    assert!(matches!(
        client.complete_authenticated_diagnostic_hold(&binding, &first, hold_started, || {
            panic!("COMMIT must be rejected before sampling readiness")
        }),
        Err(ClientError::Unsupported(
            MessageKind::EarlyRenderCommitReceipt
        ))
    ));
    assert_eq!(client.phase(), ClientPhase::Failed);
    assert!(!client.early_render_capture_committed);
}

#[test]
fn diagnostic_hold_rejects_premature_liveness_request() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let binding = diagnostic_binding(session_id);
    let first = diagnostic_first(&binding);
    let request = diagnostic_request(&binding, &first);
    let inbound = recorder_frame(
        session_id,
        0,
        MessageKind::EarlyRenderDiagnosticLivenessRequest,
        &encode_early_diagnostic_liveness_request(&request).unwrap(),
    );
    let transport = FakeTransport::new(clock.clone(), inbound);
    let mut client = admitted_diagnostic_client(clock.clone(), session, binding.clone(), transport);
    let hold_started = clock.now();
    clock.advance((u128::from(EARLY_DIAGNOSTIC_MIN_HOLD_MS) - 1) * 1_000_000);
    assert!(matches!(
        client.complete_authenticated_diagnostic_hold(&binding, &first, hold_started, || {
            panic!("premature request must be rejected before sampling readiness")
        }),
        Err(ClientError::EarlyRenderDiagnosticHoldTooShort {
            observed_ms: 209_999,
            required_ms: EARLY_DIAGNOSTIC_MIN_HOLD_MS,
        })
    ));
    assert_eq!(client.phase(), ClientPhase::Failed);
    assert!(!client.early_render_capture_committed);
}

#[test]
fn diagnostic_hold_rejects_eof_as_proof() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let binding = diagnostic_binding(session_id);
    let first = diagnostic_first(&binding);
    let mut transport = FakeTransport::new(clock.clone(), Vec::new());
    transport.peer_closed = true;
    let mut client = admitted_diagnostic_client(clock.clone(), session, binding.clone(), transport);
    let hold_started = clock.now();
    clock.advance(u128::from(EARLY_DIAGNOSTIC_MIN_HOLD_MS) * 1_000_000);
    assert_eq!(
        client.complete_authenticated_diagnostic_hold(&binding, &first, hold_started, || {
            panic!("EOF must be rejected before sampling readiness")
        }),
        Err(ClientError::NotUsable)
    );
    assert_eq!(client.phase(), ClientPhase::Failed);
    assert!(!client.early_render_capture_committed);
}

#[test]
fn production_activation_mints_only_the_typed_local_identity_permit() {
    assert!(DUPLEX_CONTROL_CLIENT_ENABLED);
    assert!(compact_contract::build_profile::DOMAIN_CAMPAIGN_ID_LIVE_READER_ENABLED);
    assert!(product_rollover_enabled());
    assert!(duplex_client_product_permit().is_ok());

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
    let inbound_len = inbound.len();
    let client = client(clock, session, inbound);

    assert_eq!(client.phase(), ClientPhase::Handshaking);
    assert!(!client.first_frame_acked());
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

#[test]
fn handshake_heartbeat_and_first_frame_happy_path() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let heartbeat_challenge = [0x22; 16];

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
        &encode_first_compact_frame_ack(FirstCompactFrameAckV1 {
            mode: StreamIdentityMode::IdentityV1,
            logical_stream_id: 0,
            h4cr_sequence: 0,
            game_hour_raw: compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24,
            domain_id: compact_contract::campaign_identity::COMPACT_DOMAIN_STREAM_HEADER,
            record_type: compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER,
            complete_record_len: 4096,
            complete_record_sha256: [0x77; 32],
            payload_sha256: [0x66; 32],
        })
        .unwrap(),
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
            reported_state: HeartbeatState::Producing.code(),
        }
    );

    let event = client.serve_next(deadline).unwrap();
    match event {
        ClientEvent::FirstFrameAcked(ack) => assert_eq!(ack.h4cr_sequence, 0),
        other => panic!("expected first-frame ack, got {other:?}"),
    }
    assert_eq!(client.phase(), ClientPhase::Producing);
    assert!(client.first_frame_acked());

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
    assert_eq!(proof.state, HeartbeatState::Producing);
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
    assert_eq!(proof.state, HeartbeatState::WaitingForWorld);
}

#[test]
fn deadline_expiry_during_handshake_send_poisons_the_session() {
    let clock = FakeClock::at(0);
    let (session, _session_id) = fresh_session();
    let mut client = client(clock.clone(), session, Vec::new());
    let deadline = MonotonicDeadline::from_now(&clock, Duration::from_secs(30));
    clock.set(30_000_000_000);

    let err = client.perform_handshake(&permit(), deadline).unwrap_err();
    assert_eq!(
        err,
        ClientError::Control(ControlError::Io(std::io::ErrorKind::TimedOut))
    );
    assert_eq!(client.phase(), ClientPhase::Failed);
    assert_eq!(
        client.perform_handshake(&permit(), deadline),
        Err(ClientError::NotUsable)
    );
}

#[test]
fn wrong_direction_frame_is_rejected_and_poisons() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let inbound = recorder_frame(session_id, 0, MessageKind::HeartbeatProof, &vec![0u8; 32]);

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
    let inbound = recorder_frame(session_id, 0, MessageKind::RolloverAck, &[]);
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
    transport.peer_closed = true;
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

    let stop_flag = AtomicBool::new(true);
    let outcome = client.run(&stop_flag).unwrap();
    assert_eq!(outcome, ClientOutcome::StoppedByRequest);
    assert_eq!(client.phase(), ClientPhase::Stopped);
}

#[test]
fn terminal_transport_error_on_hello_send_poisons_the_session() {
    let clock = FakeClock::at(0);
    let (session, _session_id) = fresh_session();
    let mut transport = FakeTransport::new(clock.clone(), Vec::new());
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

#[test]
fn wait_named_pipe_timeout_rounds_up_and_clamps() {
    let clock = FakeClock::at(0);
    let deadline = MonotonicDeadline(MonotonicInstant(400_000));
    assert_eq!(deadline.wait_named_pipe_timeout_ms(clock.now()), Some(1));

    let deadline = MonotonicDeadline(MonotonicInstant(1));
    assert_eq!(deadline.wait_named_pipe_timeout_ms(clock.now()), Some(1));

    let deadline = MonotonicDeadline(MonotonicInstant(0));
    assert_eq!(deadline.wait_named_pipe_timeout_ms(clock.now()), None);
    let deadline = MonotonicDeadline(MonotonicInstant(0));
    assert_eq!(
        deadline.wait_named_pipe_timeout_ms(MonotonicInstant(5)),
        None
    );

    let deadline = MonotonicDeadline(MonotonicInstant(u128::from(u64::MAX)));
    let ms = deadline.wait_named_pipe_timeout_ms(clock.now()).unwrap();
    assert_eq!(ms, u32::MAX - 1);
}

enum Scripted {
    Create {
        advance_ns: u128,
        outcome: CreateFileOutcome<u64>,
    },
    Wait {
        advance_ns: u128,
        outcome: WaitOutcome,
    },
}

struct ScriptedConnector {
    clock: FakeClock,
    steps: VecDeque<Scripted>,
    wait_timeouts: Vec<(u128, u32)>,
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
            Scripted::Create {
                advance_ns: 0,
                outcome: CreateFileOutcome::NotFound,
            },
            Scripted::Wait {
                advance_ns: 100_000_000,
                outcome: WaitOutcome::Signaled,
            },
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

    let handle = open_client_control_pipe(
        &test_duplex_client_permit(),
        &mut connector,
        &clock,
        deadline,
    )
    .unwrap();
    assert_eq!(handle, 1);
    assert_eq!(connector.discarded, 0);
    assert_eq!(
        connector.wait_timeouts,
        vec![(0, 30_000), (100_000_000, 29_900)]
    );
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
                advance_ns: 20_000_000_000,
                outcome: WaitOutcome::TimedOut,
            },
            Scripted::Create {
                advance_ns: 0,
                outcome: CreateFileOutcome::Busy,
            },
            Scripted::Wait {
                advance_ns: 20_000_000_000,
                outcome: WaitOutcome::TimedOut,
            },
        ],
    );

    let err = open_client_control_pipe(
        &test_duplex_client_permit(),
        &mut connector,
        &clock,
        deadline,
    )
    .unwrap_err();
    assert_eq!(err, DuplexConnectError::DeadlineExpired);
    assert_eq!(connector.discarded, 0);
    assert!(connector
        .create_instants
        .iter()
        .all(|&t| t < 30_000_000_000));
    assert!(connector
        .wait_timeouts
        .iter()
        .all(|&(t, _)| t < 30_000_000_000));
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
                advance_ns: 29_900_000_000,
                outcome: WaitOutcome::Signaled,
            },
            Scripted::Create {
                advance_ns: 200_000_000,
                outcome: CreateFileOutcome::Connected(0),
            },
        ],
    );
    let err = open_client_control_pipe(
        &test_duplex_client_permit(),
        &mut connector,
        &clock,
        deadline,
    )
    .unwrap_err();
    assert_eq!(err, DuplexConnectError::DeadlineExpired);
    assert_eq!(connector.discarded, 1);
}

#[test]
fn connector_expires_before_calling_wait_when_no_positive_budget_remains() {
    let clock = FakeClock::at(0);
    let deadline = MonotonicDeadline::from_now(&clock, Duration::from_secs(30));
    let mut connector = ScriptedConnector::new(
        clock.clone(),
        vec![Scripted::Create {
            advance_ns: 30_000_000_000,
            outcome: CreateFileOutcome::Busy,
        }],
    );
    let err = open_client_control_pipe(
        &test_duplex_client_permit(),
        &mut connector,
        &clock,
        deadline,
    )
    .unwrap_err();
    assert_eq!(err, DuplexConnectError::DeadlineExpired);
    assert!(connector.wait_timeouts.is_empty());
    assert_eq!(connector.create_instants, vec![0]);
}

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
    let deadline = MonotonicDeadline(MonotonicInstant(0));
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let outcome = run_bounded_worker(&test_duplex_client_permit(), &clock, deadline, move || {
        // Block until the test releases us; the driver must not wait for this.
        let _ = release_rx.recv();
        0u32
    });
    assert!(matches!(outcome, BoundedWorkerOutcome::Quarantined));
    assert!(quarantined_worker_count() >= before + 1);
    drop(release_tx);
}

#[test]
fn control_pipe_name_is_per_pid_and_token() {
    assert_eq!(
        control_pipe_name(4321, "tok-abc"),
        r"\\.\pipe\battle-plan_control_4321_tok-abc"
    );
}

#[test]
fn bounded_reap_reclaims_when_completion_is_observed_within_the_bound() {
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
    let mut polls = 0;
    let outcome = bounded_reap(5, || {
        polls += 1;
        false
    });
    assert_eq!(outcome, ReapOutcome::LeakAbandoned);
    assert_eq!(polls, 5, "the reap is bounded to exactly `attempts` slices");
}

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

    let backend = client.backend();
    let frame = decode_frame(&backend.outbound[HELLO_WIRE_LEN..], session_id, 1).unwrap();
    assert_eq!(frame.kind, MessageKind::SinkTerminalFailure);
    let decoded = decode_sink_failure(SinkFailureKind::Terminal, &frame.payload).unwrap();
    assert_eq!(decoded, report);

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

    {
        let backend = client.backend();
        let frame = decode_frame(&backend.outbound[HELLO_WIRE_LEN..], session_id, 1).unwrap();
        assert_eq!(frame.kind, MessageKind::IdentityFailure);
        assert_eq!(decode_identity_failure(&frame.payload).unwrap(), failure);
    }

    let event = client.serve_next(deadline).unwrap();
    assert_eq!(event, ClientEvent::IdentityFailureAcked(ack));
}

#[test]
fn pre_hello_sink_setup_failure_is_sent_as_frame_zero() {
    let clock = FakeClock::at(0);
    let (session, session_id) = fresh_session();
    let mut client = client(clock.clone(), session, Vec::new());
    let deadline = far_deadline(&clock);
    let report = sample_sink_report(SinkFailureKind::Setup);
    client.report_sink_failure(&report, deadline).unwrap();

    let backend = client.backend();
    let frame = decode_frame(&backend.outbound, session_id, 0).unwrap();
    assert_eq!(frame.kind, MessageKind::SinkSetupFailure);
    assert_eq!(
        decode_sink_failure(SinkFailureKind::Setup, &frame.payload).unwrap(),
        report
    );

    assert_eq!(client.phase(), ClientPhase::Failed);
    assert_eq!(
        client.perform_handshake(&permit(), deadline),
        Err(ClientError::NotUsable)
    );
}

#[test]
fn pre_hello_terminal_sink_failure_is_sent_as_frame_zero() {
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
    assert_eq!(
        client.report_identity_failure(&sample_identity_failure(), deadline),
        Err(ClientError::NotUsable)
    );
    assert!(client.backend().outbound.is_empty());
    assert_eq!(client.phase(), ClientPhase::Handshaking);
}
