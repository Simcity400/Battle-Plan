//! Recorder-side H4CT transaction for suspended render capture.

#![cfg(windows)]

use crate::auto_attach::proof::{
    decode_collector_hello_capabilities, decode_hex_16, decode_hex_32, ProofExpectation,
};
use crate::auto_attach::staging::generate_secret_128;
use crate::control_transport::{
    ControlCancellation, ControlCapabilities, ControlDeadline, ControlError, ControlPeerIdentity,
    DeadlineControlChannel, EndpointRole, MessageKind,
};
use crate::early_render_launch::{
    EarlyDiagnosticLivenessProof, EarlyFirstGameD3dProof, EarlyHookCompleteness,
    EarlyHookReadyChannel, EarlyHookReadyProof, EarlyRenderCaptureReceipt,
    EarlyRenderLaunchManifest, EarlyRuntimeDlcRosterProof, EARLY_DIAGNOSTIC_LIVENESS_SCHEMA,
    EARLY_HOOK_READY_SCHEMA,
};
use crate::portable_content_manifest::PortableLaunchBinding;
use crate::runtime_dlc_roster::{
    validate_effective_runtime_dlc_roster, AuthenticatedDlcDescriptor, AuthenticatedDlcVfsRoster,
};
use crate::transport_status::{decode_sink_failure, SinkFailureReport};
use crate::windows_control_pipe::{
    prepare_control_listener, PreparedControlListener, RestrictiveSecurityPolicy, WindowsControlIo,
    WindowsPipeBackend,
};
use compact_contract::early_render_content_manifest::{
    encode_early_render_content_manifest_chunk, encode_early_render_content_manifest_commit,
    EarlyRenderContentManifestChunkV1, EarlyRenderContentManifestCommitV1,
};
use compact_contract::early_render_control::{
    decode_early_diagnostic_liveness_response, decode_early_first_game_d3d,
    decode_early_hook_ready, encode_early_commit_capture_receipt, encode_early_diagnostic_end,
    encode_early_diagnostic_liveness_request, encode_early_render_launch_bind,
    EarlyCommitCaptureReceiptV1, EarlyDiagnosticEndV1, EarlyDiagnosticLivenessRequestV1,
    EarlyRenderLaunchBindV1, EARLY_DIAGNOSTIC_END_RECORDER_CANCEL, EARLY_HOOK_COMPLETENESS_MASK,
};
use compact_contract::early_render_runtime_dlc::decode_early_render_runtime_dlc_roster;
use compact_contract::early_render_vfs::{
    canonical_early_render_vfs_index_bytes, encode_early_render_vfs_index_chunk,
    encode_early_render_vfs_index_commit, EarlyRenderVfsCodecError, EarlyRenderVfsIndexChunkV1,
    EarlyRenderVfsIndexCommitV1, EarlyRenderVfsWinnerRowV1,
};
use compact_contract::early_render_vfs_payload::{
    decode_early_render_vfs_payload_request, encode_early_render_vfs_payload_chunk,
    encode_early_render_vfs_payload_commit, EarlyRenderVfsPayloadChunkV1,
    EarlyRenderVfsPayloadCommitV1, EarlyRenderVfsPayloadRequestV1,
};
use portable_content_manifest::effective_vfs::EffectiveVfs;
use portable_content_manifest::ContentManifest;
use production_render_submit_live_lane::RawEffectiveVfsWinnerIndexBinding;
use snapshot::sha256::Sha256;
use std::ops::Range;
use std::time::Duration;

type Prepared = PreparedControlListener<WindowsPipeBackend>;

/// Terminal result of the post-COMMIT early-render H4CT service. A collector
/// sink failure is a typed authenticated outcome, not an unexpected VFS frame.
pub enum EarlyRenderServiceOutcome {
    CaptureTeardown,
    SinkFailure(SinkFailureReport),
}

pub fn early_render_content_manifest_metadata_envelope(
    source: &ContentManifest,
) -> Result<ContentManifest, String> {
    source.validate()?;
    let mut envelope = source.clone();
    envelope.required_content.clear();
    for layer in &mut envelope.ordered_layers {
        layer.required_for_playback = false;
    }
    for dlc in &mut envelope.dlc_selection {
        dlc.required_for_playback = false;
    }
    envelope.source_profile.selection_sha256 = envelope.computed_selection_sha256()?;
    envelope.manifest_digest_sha256 = None;
    envelope.with_computed_digest()
}

/// Canonical, path-redacted effective-VFS authority prepared while the game is
/// still suspended. Chunk boundaries are deterministic and every wire frame is
/// additionally bound to the admitted H4CT session and encoded BIND digest.
#[derive(Clone, Debug)]
pub struct PreparedEarlyRenderVfsIndex {
    rows: Vec<EarlyRenderVfsWinnerRowV1>,
    chunk_ranges: Vec<Range<usize>>,
    index_sha256: [u8; 32],
}

impl PreparedEarlyRenderVfsIndex {
    pub fn from_winner_binding(
        binding: &RawEffectiveVfsWinnerIndexBinding,
    ) -> Result<Self, String> {
        binding
            .validate()
            .map_err(|error| format!("admit effective-VFS winner binding: {error}"))?;
        let rows = binding
            .ordered_rows
            .iter()
            .map(|row| {
                let layer_kind = match row.content.layer_kind.as_str() {
                    "base" => compact_contract::early_render_vfs::EarlyRenderVfsLayerKind::Base,
                    "integrated_dlc" => {
                        compact_contract::early_render_vfs::EarlyRenderVfsLayerKind::IntegratedDlc
                    }
                    "dlc" => compact_contract::early_render_vfs::EarlyRenderVfsLayerKind::Dlc,
                    "mod" => compact_contract::early_render_vfs::EarlyRenderVfsLayerKind::Mod,
                    other => {
                        return Err(format!(
                            "effective-VFS winner has unsupported layer kind {other:?}"
                        ));
                    }
                };
                Ok(EarlyRenderVfsWinnerRowV1 {
                    normalized_game_relative_path: row.content.logical_path.clone(),
                    winning_layer_ordinal: row.content.winning_layer_ordinal,
                    layer_kind,
                    layer_id: row.content.layer_id.clone(),
                    source_sha256: decode_hex_32(&row.content.source_sha256)
                        .map_err(|error| format!("effective-VFS winner source digest: {error}"))?,
                    payload_sha256: decode_hex_32(&row.content.payload_sha256)
                        .map_err(|error| format!("effective-VFS winner payload digest: {error}"))?,
                    payload_bytes: row.content.payload_bytes,
                    source_container_sha256: row
                        .source_container_sha256
                        .as_deref()
                        .map(decode_hex_32)
                        .transpose()
                        .map_err(|error| {
                            format!("effective-VFS winner container digest: {error}")
                        })?,
                    source_byte_offset: row.source_byte_offset,
                    source_byte_count: row.source_byte_count,
                    source_range_sha256: decode_hex_32(&row.source_range_sha256)
                        .map_err(|error| format!("effective-VFS winner range digest: {error}"))?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Self::from_ordered_rows(rows)
    }

    pub fn from_ordered_rows(rows: Vec<EarlyRenderVfsWinnerRowV1>) -> Result<Self, String> {
        let total_row_count = u32::try_from(rows.len())
            .map_err(|_| "early effective-VFS winner count exceeds u32".to_string())?;
        let canonical = canonical_early_render_vfs_index_bytes(&rows)
            .map_err(|error| format!("canonicalize early effective-VFS index: {error:?}"))?;
        let index_sha256 = Sha256::digest(&canonical);
        let mut chunk_ranges = Vec::new();
        let mut first = 0usize;
        while first < rows.len() {
            let mut end = first;
            while end < rows.len() {
                let candidate_end = end + 1;
                let probe = EarlyRenderVfsIndexChunkV1 {
                    process_id: 1,
                    process_start_nonce: 1,
                    session_id: [1; 16],
                    launch_bind_sha256: [1; 32],
                    index_sha256,
                    chunk_index: 0,
                    chunk_count: 1,
                    first_row_index: first as u32,
                    total_row_count,
                    rows: rows[first..candidate_end].to_vec(),
                };
                match encode_early_render_vfs_index_chunk(&probe) {
                    Ok(_) => end = candidate_end,
                    Err(EarlyRenderVfsCodecError::PayloadTooLarge) if end > first => break,
                    Err(EarlyRenderVfsCodecError::PayloadTooLarge) => {
                        return Err(format!(
                            "early effective-VFS row {:?} exceeds the H4CT payload bound",
                            rows[first].normalized_game_relative_path
                        ));
                    }
                    Err(error) => {
                        return Err(format!(
                            "prepare early effective-VFS chunk at row {first}: {error:?}"
                        ));
                    }
                }
            }
            chunk_ranges.push(first..end);
            first = end;
        }
        if chunk_ranges.is_empty() || chunk_ranges.len() > rows.len() {
            return Err("early effective-VFS index produced an invalid chunk roster".into());
        }
        Ok(Self {
            rows,
            chunk_ranges,
            index_sha256,
        })
    }

    pub fn index_sha256(&self) -> [u8; 32] {
        self.index_sha256
    }

    pub fn row_count(&self) -> u32 {
        self.rows.len() as u32
    }

    pub fn chunk_count(&self) -> u32 {
        self.chunk_ranges.len() as u32
    }
}

pub struct PreparedEarlyRenderHostChannel {
    prepared: Option<Prepared>,
    channel: Option<DeadlineControlChannel<WindowsControlIo>>,
    cancellation: ControlCancellation,
    session_id: [u8; 16],
    expectation: ProofExpectation,
    manifest: EarlyRenderLaunchManifest,
    content_manifest_bytes: Vec<u8>,
    content_manifest_bytes_sha256: [u8; 32],
    content_manifest_chunk_count: u32,
    effective_vfs_index: PreparedEarlyRenderVfsIndex,
    effective_vfs: EffectiveVfs,
    runtime_dlc_launch: PortableLaunchBinding,
    runtime_dlc_descriptors: Vec<AuthenticatedDlcDescriptor>,
    launch_bind_sha256: Option<[u8; 32]>,
    timeout_ms: u32,
}

impl PreparedEarlyRenderHostChannel {
    const VFS_PAYLOAD_CHUNK_BYTES: usize = 1 << 19;
    const CONTENT_MANIFEST_CHUNK_BYTES: usize = 1 << 19;

    pub fn prepare(
        process_id: u32,
        process_start_nonce: u64,
        pipe_token: &str,
        manifest: EarlyRenderLaunchManifest,
        content_manifest: ContentManifest,
        effective_vfs_index: PreparedEarlyRenderVfsIndex,
        effective_vfs: EffectiveVfs,
        runtime_dlc_launch: PortableLaunchBinding,
        runtime_dlc_descriptors: Vec<AuthenticatedDlcDescriptor>,
        timeout_ms: u32,
    ) -> Result<Self, String> {
        if timeout_ms == 0 || timeout_ms > 120_000 {
            return Err("early H4CT timeout is outside 1..=120000 ms".into());
        }
        manifest.validate()?;
        content_manifest.validate()?;
        if !content_manifest.required_content.is_empty()
            || content_manifest
                .ordered_layers
                .iter()
                .any(|layer| layer.required_for_playback)
            || content_manifest
                .dlc_selection
                .iter()
                .any(|dlc| dlc.required_for_playback)
        {
            return Err(
                "early H4CT content manifest is not an observed-only metadata envelope".into(),
            );
        }
        if content_manifest.effective_vfs_manifest_sha256 != manifest.effective_vfs_manifest_sha256
        {
            return Err(
                "early content manifest differs from launch effective-VFS authority".into(),
            );
        }
        let content_manifest_bytes = content_manifest.encode_json()?;
        let content_manifest_bytes_sha256 = Sha256::digest(&content_manifest_bytes);
        let content_manifest_chunk_count = u32::try_from(
            content_manifest_bytes
                .len()
                .div_ceil(Self::CONTENT_MANIFEST_CHUNK_BYTES),
        )
        .map_err(|_| "early content manifest chunk count exceeds u32")?;
        if content_manifest_bytes.is_empty() || content_manifest_chunk_count == 0 {
            return Err("early content manifest encoded to no bytes".into());
        }
        let session_id =
            decode_hex_16(pipe_token).map_err(|error| format!("early H4CT pipe token: {error}"))?;
        let cancellation = ControlCancellation::new();
        let deadline = deadline(timeout_ms)?;
        let pipe_name = format!(r"\\.\pipe\battle-plan_control_{process_id}_{pipe_token}");
        let prepared = prepare_control_listener(
            WindowsPipeBackend,
            &pipe_name,
            RestrictiveSecurityPolicy::default(),
            deadline,
            &cancellation,
        )
        .map_err(|error| format!("prepare early H4CT listener: {error}"))?;
        Ok(Self {
            prepared: Some(prepared),
            channel: None,
            cancellation,
            session_id,
            expectation: ProofExpectation {
                pid: process_id,
                creation_filetime: process_start_nonce,
                h4cr_version: compact_contract::COMPACT_RECORD_VERSION,
                dll_sha256: decode_hex_32(&manifest.collector_sha256)
                    .map_err(|error| format!("early collector digest: {error}"))?,
                challenge: session_id,
            },
            manifest,
            content_manifest_bytes,
            content_manifest_bytes_sha256,
            content_manifest_chunk_count,
            effective_vfs_index,
            effective_vfs,
            runtime_dlc_launch,
            runtime_dlc_descriptors,
            launch_bind_sha256: None,
            timeout_ms,
        })
    }

    fn accept_and_handshake(&mut self, timeout_ms: u32) -> Result<(), String> {
        if timeout_ms != self.timeout_ms {
            return Err("early H4CT timeout differs from the launch policy".into());
        }
        if self.channel.is_some() {
            return Ok(());
        }
        let prepared = self
            .prepared
            .take()
            .ok_or("early H4CT listener was already consumed")?;
        let none = ControlCapabilities::from_bits(0).map_err(|error| error.to_string())?;
        let mut channel = prepared
            .accept_and_admit(
                self.session_id,
                ControlPeerIdentity {
                    pid: self.expectation.pid,
                    creation_filetime: self.expectation.creation_filetime,
                },
                none,
                none,
                EndpointRole::Recorder,
                deadline(timeout_ms)?,
                &self.cancellation,
            )
            .map_err(|error| format!("accept early H4CT peer: {error}"))?;
        let frame = channel
            .recv_handshake_until(deadline(timeout_ms)?, &self.cancellation)
            .map_err(|error| format!("receive early H4CT hello: {error}"))?;
        if frame.kind != MessageKind::CollectorHello {
            return Err(format!(
                "early H4CT expected CollectorHello, received {:?}",
                frame.kind
            ));
        }
        let observed = decode_collector_hello_capabilities(&frame.payload, &self.expectation)
            .map_err(|error| format!("decode early H4CT hello: {error}"))?;
        let required = ControlCapabilities::from_bits(
            ControlCapabilities::HEARTBEAT
                | ControlCapabilities::FIRST_COMPACT_ACK
                | ControlCapabilities::TRANSPORT_STATUS
                | ControlCapabilities::CAMPAIGN_IDENTITY
                | ControlCapabilities::EARLY_RENDER,
        )
        .map_err(|error| error.to_string())?;
        channel
            .admit_decoded_hello_capabilities(required, observed)
            .map_err(|error| format!("admit early H4CT capabilities: {error}"))?;
        let bind = encode_early_render_launch_bind(&EarlyRenderLaunchBindV1 {
            process_id: self.expectation.pid,
            process_start_nonce: self.expectation.creation_filetime,
            timeout_ms: self.timeout_ms,
            launch_manifest_sha256: decode_hex_32(&self.manifest.sha256()?)
                .map_err(|error| format!("early launch manifest digest: {error}"))?,
            effective_vfs_manifest_sha256: decode_hex_32(
                &self.manifest.effective_vfs_manifest_sha256,
            )
            .map_err(|error| format!("early effective VFS digest: {error}"))?,
            effective_vfs_index_sha256: self.effective_vfs_index.index_sha256(),
            effective_vfs_index_row_count: self.effective_vfs_index.row_count(),
            effective_vfs_index_chunk_count: self.effective_vfs_index.chunk_count(),
            content_manifest_bytes_sha256: self.content_manifest_bytes_sha256,
            content_manifest_byte_count: self.content_manifest_bytes.len() as u64,
            content_manifest_chunk_count: self.content_manifest_chunk_count,
            executable_sha256: decode_hex_32(&self.manifest.executable_sha256)
                .map_err(|error| format!("early executable digest: {error}"))?,
            collector_sha256: self.expectation.dll_sha256,
        })
        .map_err(|error| format!("encode early H4CT BIND: {error:?}"))?;
        channel
            .send_until(
                MessageKind::EarlyRenderLaunchBind,
                &bind,
                deadline(timeout_ms)?,
                &self.cancellation,
            )
            .map_err(|error| format!("send early H4CT BIND: {error}"))?;
        let launch_bind_sha256 = Sha256::digest(&bind);
        self.launch_bind_sha256 = Some(launch_bind_sha256);
        let effective_vfs_manifest_sha256 =
            decode_hex_32(&self.manifest.effective_vfs_manifest_sha256)
                .map_err(|error| format!("early effective VFS digest: {error}"))?;
        for (chunk_index, bytes) in self
            .content_manifest_bytes
            .chunks(Self::CONTENT_MANIFEST_CHUNK_BYTES)
            .enumerate()
        {
            let payload =
                encode_early_render_content_manifest_chunk(&EarlyRenderContentManifestChunkV1 {
                    process_id: self.expectation.pid,
                    process_start_nonce: self.expectation.creation_filetime,
                    session_id: self.session_id,
                    launch_bind_sha256,
                    effective_vfs_manifest_sha256,
                    effective_vfs_index_sha256: self.effective_vfs_index.index_sha256(),
                    content_manifest_bytes_sha256: self.content_manifest_bytes_sha256,
                    chunk_index: chunk_index as u32,
                    chunk_count: self.content_manifest_chunk_count,
                    byte_offset: (chunk_index * Self::CONTENT_MANIFEST_CHUNK_BYTES) as u64,
                    total_bytes: self.content_manifest_bytes.len() as u64,
                    bytes: bytes.to_vec(),
                })
                .map_err(|error| {
                    format!("encode early H4CT content manifest chunk {chunk_index}: {error:?}")
                })?;
            channel
                .send_until(
                    MessageKind::EarlyRenderContentManifestChunk,
                    &payload,
                    deadline(timeout_ms)?,
                    &self.cancellation,
                )
                .map_err(|error| {
                    format!("send early H4CT content manifest chunk {chunk_index}: {error}")
                })?;
        }
        let content_commit =
            encode_early_render_content_manifest_commit(&EarlyRenderContentManifestCommitV1 {
                process_id: self.expectation.pid,
                process_start_nonce: self.expectation.creation_filetime,
                session_id: self.session_id,
                launch_bind_sha256,
                effective_vfs_manifest_sha256,
                effective_vfs_index_sha256: self.effective_vfs_index.index_sha256(),
                content_manifest_bytes_sha256: self.content_manifest_bytes_sha256,
                chunk_count: self.content_manifest_chunk_count,
                total_bytes: self.content_manifest_bytes.len() as u64,
            })
            .map_err(|error| format!("encode early H4CT content manifest COMMIT: {error:?}"))?;
        channel
            .send_until(
                MessageKind::EarlyRenderContentManifestCommit,
                &content_commit,
                deadline(timeout_ms)?,
                &self.cancellation,
            )
            .map_err(|error| format!("send early H4CT content manifest COMMIT: {error}"))?;
        for (chunk_index, range) in self.effective_vfs_index.chunk_ranges.iter().enumerate() {
            let payload = encode_early_render_vfs_index_chunk(&EarlyRenderVfsIndexChunkV1 {
                process_id: self.expectation.pid,
                process_start_nonce: self.expectation.creation_filetime,
                session_id: self.session_id,
                launch_bind_sha256,
                index_sha256: self.effective_vfs_index.index_sha256(),
                chunk_index: chunk_index as u32,
                chunk_count: self.effective_vfs_index.chunk_count(),
                first_row_index: range.start as u32,
                total_row_count: self.effective_vfs_index.row_count(),
                rows: self.effective_vfs_index.rows[range.clone()].to_vec(),
            })
            .map_err(|error| format!("encode early H4CT VFS chunk {chunk_index}: {error:?}"))?;
            channel
                .send_until(
                    MessageKind::EarlyRenderVfsIndexChunk,
                    &payload,
                    deadline(timeout_ms)?,
                    &self.cancellation,
                )
                .map_err(|error| format!("send early H4CT VFS chunk {chunk_index}: {error}"))?;
        }
        let commit = encode_early_render_vfs_index_commit(&EarlyRenderVfsIndexCommitV1 {
            process_id: self.expectation.pid,
            process_start_nonce: self.expectation.creation_filetime,
            session_id: self.session_id,
            launch_bind_sha256,
            index_sha256: self.effective_vfs_index.index_sha256(),
            chunk_count: self.effective_vfs_index.chunk_count(),
            total_row_count: self.effective_vfs_index.row_count(),
        })
        .map_err(|error| format!("encode early H4CT VFS COMMIT: {error:?}"))?;
        channel
            .send_until(
                MessageKind::EarlyRenderVfsIndexCommit,
                &commit,
                deadline(timeout_ms)?,
                &self.cancellation,
            )
            .map_err(|error| format!("send early H4CT VFS COMMIT: {error}"))?;
        self.channel = Some(channel);
        Ok(())
    }

    pub fn cancellation_handle(&self) -> ControlCancellation {
        self.cancellation.clone()
    }

    /// Retains the authenticated H4CT session after COMMIT and serves only
    /// exact ranges of winners that were admitted in the pre-resume index.
    /// Cancellation is the normal capture-teardown terminator.
    pub fn serve_vfs_payload_requests(
        mut self,
        capture_teardown_timeout_ms: u64,
    ) -> Result<EarlyRenderServiceOutcome, String> {
        if capture_teardown_timeout_ms == 0 {
            return Err("early H4CT VFS payload service has no capture teardown bound".into());
        }
        let capture_teardown_deadline =
            ControlDeadline::after(Duration::from_millis(capture_teardown_timeout_ms))
                .map_err(|error| format!("early H4CT VFS service deadline: {error}"))?;
        loop {
            let frame = match self
                .channel
                .as_mut()
                .ok_or("early H4CT channel is not connected")?
                .recv_until(capture_teardown_deadline, &self.cancellation)
            {
                Ok(frame) => frame,
                Err(ControlError::Cancelled) => {
                    return Ok(EarlyRenderServiceOutcome::CaptureTeardown)
                }
                Err(error) => {
                    return Err(format!("receive early H4CT VFS payload request: {error}"));
                }
            };
            if matches!(
                frame.kind,
                MessageKind::SinkSetupFailure | MessageKind::SinkTerminalFailure
            ) {
                let report = decode_sink_failure(frame.kind, &frame.payload)
                    .map_err(|error| format!("decode authenticated sink failure: {error}"))?;
                return Ok(EarlyRenderServiceOutcome::SinkFailure(report));
            }
            if frame.kind != MessageKind::EarlyRenderVfsPayloadRequest {
                return Err(format!(
                    "early H4CT expected VFS payload request or terminal sink status, received {:?}",
                    frame.kind
                ));
            }
            let request = decode_early_render_vfs_payload_request(&frame.payload)
                .map_err(|error| format!("decode early H4CT VFS payload request: {error:?}"))?;
            self.respond_vfs_payload_request(request)?;
        }
    }

    fn respond_vfs_payload_request(
        &mut self,
        request: EarlyRenderVfsPayloadRequestV1,
    ) -> Result<(), String> {
        let range = self.resolve_vfs_payload_request(&request)?;
        let (chunks, commit) = Self::encode_vfs_payload_response(&request, &range)?;
        for payload in chunks {
            self.channel
                .as_mut()
                .ok_or("early H4CT channel is not connected")?
                .send_until(
                    MessageKind::EarlyRenderVfsPayloadChunk,
                    &payload,
                    deadline(self.timeout_ms)?,
                    &self.cancellation,
                )
                .map_err(|error| format!("send early H4CT VFS payload chunk: {error}"))?;
        }
        self.channel
            .as_mut()
            .ok_or("early H4CT channel is not connected")?
            .send_until(
                MessageKind::EarlyRenderVfsPayloadCommit,
                &commit,
                deadline(self.timeout_ms)?,
                &self.cancellation,
            )
            .map_err(|error| format!("send early H4CT VFS payload COMMIT: {error}"))
    }

    fn resolve_vfs_payload_request(
        &self,
        request: &EarlyRenderVfsPayloadRequestV1,
    ) -> Result<Vec<u8>, String> {
        let launch_bind_sha256 = self
            .launch_bind_sha256
            .ok_or("early H4CT VFS payload service has no admitted BIND")?;
        let effective_vfs_manifest_sha256 =
            decode_hex_32(&self.manifest.effective_vfs_manifest_sha256)
                .map_err(|error| format!("early effective VFS digest: {error}"))?;
        if request.process_id != self.expectation.pid
            || request.process_start_nonce != self.expectation.creation_filetime
            || request.session_id != self.session_id
            || request.launch_bind_sha256 != launch_bind_sha256
            || request.effective_vfs_manifest_sha256 != effective_vfs_manifest_sha256
            || request.effective_vfs_index_sha256 != self.effective_vfs_index.index_sha256()
        {
            return Err("early H4CT VFS payload request names foreign launch authority".into());
        }
        let winner_position = self
            .effective_vfs_index
            .rows
            .binary_search_by(|row| {
                row.normalized_game_relative_path
                    .cmp(&request.winner.normalized_game_relative_path)
            })
            .map_err(|_| "early H4CT VFS payload request names an unknown winner")?;
        if self.effective_vfs_index.rows[winner_position] != request.winner {
            return Err("early H4CT VFS payload request tampers with winner authority".into());
        }
        let resolved = self.effective_vfs.resolve_required(
            &request.winner.normalized_game_relative_path,
            "early H4CT payload winner",
        )?;
        let resolved_layer_kind = match resolved.content.layer_kind {
            portable_content_manifest::ContentLayerKind::Base => "base",
            portable_content_manifest::ContentLayerKind::IntegratedDlc => "integrated_dlc",
            portable_content_manifest::ContentLayerKind::Dlc => "dlc",
            portable_content_manifest::ContentLayerKind::Mod => "mod",
        };
        if resolved.content.effective_vfs_manifest_sha256
            != self.manifest.effective_vfs_manifest_sha256
            || resolved.content.normalized_game_relative_path
                != request.winner.normalized_game_relative_path
            || resolved.content.winning_layer_ordinal != request.winner.winning_layer_ordinal
            || resolved_layer_kind != request.winner.layer_kind.as_str()
            || resolved.content.layer_id != request.winner.layer_id
            || decode_hex_32(&resolved.content.source_sha256)
                .map_err(|error| format!("resolved VFS source digest: {error}"))?
                != request.winner.source_sha256
            || decode_hex_32(&resolved.content.payload_sha256)
                .map_err(|error| format!("resolved VFS payload digest: {error}"))?
                != request.winner.payload_sha256
            || resolved.content.payload_bytes != request.winner.payload_bytes
            || resolved.bytes.len() as u64 != request.winner.payload_bytes
        {
            return Err("early H4CT VFS payload winner drifted after pre-resume admission".into());
        }
        let range_start = usize::try_from(request.payload_offset)
            .map_err(|_| "early H4CT VFS payload range start exceeds usize")?;
        let range_end_u64 = request
            .payload_offset
            .checked_add(request.payload_count)
            .ok_or("early H4CT VFS payload range overflow")?;
        let range_end = usize::try_from(range_end_u64)
            .map_err(|_| "early H4CT VFS payload range end exceeds usize")?;
        let range = resolved
            .bytes
            .get(range_start..range_end)
            .ok_or("early H4CT VFS payload range exceeds the admitted winner")?;
        let range_sha256 = Sha256::digest(range);
        if range_sha256 != request.expected_payload_range_sha256 {
            return Err("early H4CT VFS payload range digest differs from request".into());
        }
        Ok(range.to_vec())
    }

    fn encode_vfs_payload_response(
        request: &EarlyRenderVfsPayloadRequestV1,
        range: &[u8],
    ) -> Result<(Vec<Vec<u8>>, Vec<u8>), String> {
        let range_sha256 = Sha256::digest(range);
        if range_sha256 != request.expected_payload_range_sha256
            || range.len() as u64 != request.payload_count
        {
            return Err("early H4CT VFS payload response range differs from request".into());
        }
        let chunk_count = u32::try_from(range.len().div_ceil(Self::VFS_PAYLOAD_CHUNK_BYTES))
            .map_err(|_| "early H4CT VFS payload chunk count exceeds u32")?;
        if chunk_count == 0 {
            return Err("early H4CT VFS payload request resolved an empty range".into());
        }
        let mut chunks = Vec::with_capacity(chunk_count as usize);
        for (chunk_index, bytes) in range.chunks(Self::VFS_PAYLOAD_CHUNK_BYTES).enumerate() {
            let chunk_offset = u64::try_from(chunk_index * Self::VFS_PAYLOAD_CHUNK_BYTES)
                .map_err(|_| "early H4CT VFS payload chunk offset overflow")?;
            let payload = encode_early_render_vfs_payload_chunk(&EarlyRenderVfsPayloadChunkV1 {
                process_id: request.process_id,
                process_start_nonce: request.process_start_nonce,
                session_id: request.session_id,
                launch_bind_sha256: request.launch_bind_sha256,
                effective_vfs_manifest_sha256: request.effective_vfs_manifest_sha256,
                effective_vfs_index_sha256: request.effective_vfs_index_sha256,
                request_id: request.request_id,
                payload_offset: request.payload_offset,
                payload_count: request.payload_count,
                expected_payload_range_sha256: request.expected_payload_range_sha256,
                chunk_index: chunk_index as u32,
                chunk_count,
                chunk_offset,
                bytes: bytes.to_vec(),
            })
            .map_err(|error| format!("encode early H4CT VFS payload chunk: {error:?}"))?;
            chunks.push(payload);
        }
        let commit = encode_early_render_vfs_payload_commit(&EarlyRenderVfsPayloadCommitV1 {
            process_id: request.process_id,
            process_start_nonce: request.process_start_nonce,
            session_id: request.session_id,
            launch_bind_sha256: request.launch_bind_sha256,
            effective_vfs_manifest_sha256: request.effective_vfs_manifest_sha256,
            effective_vfs_index_sha256: request.effective_vfs_index_sha256,
            request_id: request.request_id,
            payload_offset: request.payload_offset,
            payload_count: request.payload_count,
            payload_range_sha256: range_sha256,
            chunk_count,
        })
        .map_err(|error| format!("encode early H4CT VFS payload COMMIT: {error:?}"))?;
        Ok((chunks, commit))
    }

    fn recv(&mut self, expected: MessageKind, timeout_ms: u32) -> Result<Vec<u8>, String> {
        if timeout_ms != self.timeout_ms {
            return Err("early H4CT timeout differs from the launch policy".into());
        }
        let frame = self
            .channel
            .as_mut()
            .ok_or("early H4CT channel is not connected")?
            .recv_until(deadline(timeout_ms)?, &self.cancellation)
            .map_err(|error| format!("receive early H4CT {expected:?}: {error}"))?;
        if frame.kind != expected {
            return Err(format!(
                "early H4CT expected {expected:?}, received {:?}",
                frame.kind
            ));
        }
        Ok(frame.payload)
    }
}

impl Drop for PreparedEarlyRenderHostChannel {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl EarlyHookReadyChannel for PreparedEarlyRenderHostChannel {
    fn await_ready(
        &mut self,
        process_id: u32,
        process_start_nonce: u64,
        timeout_ms: u32,
    ) -> Result<EarlyHookReadyProof, String> {
        if process_id != self.expectation.pid
            || process_start_nonce != self.expectation.creation_filetime
        {
            return Err("early READY request names a foreign process epoch".into());
        }
        self.accept_and_handshake(timeout_ms)?;
        let wire = decode_early_hook_ready(&self.recv(MessageKind::EarlyRenderReady, timeout_ms)?)
            .map_err(|error| format!("decode early READY: {error:?}"))?;
        if wire.completeness_bits != EARLY_HOOK_COMPLETENESS_MASK {
            return Err("early READY completeness mask is incomplete".into());
        }
        if wire.effective_vfs_index_sha256 != self.effective_vfs_index.index_sha256()
            || wire.effective_vfs_index_row_count != self.effective_vfs_index.row_count()
            || wire.effective_vfs_index_chunk_count != self.effective_vfs_index.chunk_count()
            || wire.content_manifest_bytes_sha256 != self.content_manifest_bytes_sha256
            || wire.content_manifest_byte_count != self.content_manifest_bytes.len() as u64
            || wire.content_manifest_chunk_count != self.content_manifest_chunk_count
        {
            return Err(
                "early READY did not admit the exact transmitted effective-VFS index".into(),
            );
        }
        Ok(EarlyHookReadyProof {
            schema: EARLY_HOOK_READY_SCHEMA.into(),
            launch_manifest_sha256: hex(&wire.launch_manifest_sha256),
            effective_vfs_manifest_sha256: hex(&wire.effective_vfs_manifest_sha256),
            effective_vfs_index_sha256: hex(&wire.effective_vfs_index_sha256),
            effective_vfs_index_row_count: wire.effective_vfs_index_row_count,
            effective_vfs_index_chunk_count: wire.effective_vfs_index_chunk_count,
            content_manifest_bytes_sha256: hex(&wire.content_manifest_bytes_sha256),
            content_manifest_byte_count: wire.content_manifest_byte_count,
            content_manifest_chunk_count: wire.content_manifest_chunk_count,
            process_id: wire.process_id,
            process_start_nonce: wire.process_start_nonce,
            executable_sha256: hex(&wire.executable_sha256),
            collector_sha256: hex(&wire.collector_sha256),
            hook_epoch_nonce: wire.hook_epoch_nonce,
            hook_set_sha256: hex(&wire.hook_set_sha256),
            hook_ready_sequence: wire.hook_ready_sequence,
            hook_ready_qpc: wire.hook_ready_qpc,
            game_d3d_creation_count_at_hook_ready: wire.game_d3d_creation_count_at_hook_ready,
            preexisting_game_d3d_object_count: wire.preexisting_game_d3d_object_count,
            completeness: EarlyHookCompleteness {
                all_shader_create: true,
                input_layout_create: true,
                blend_state_create: true,
                depth_stencil_state_create: true,
                rasterizer_state_create: true,
                sampler_state_create: true,
                buffer_create_update: true,
                texture_create_update: true,
                resource_view_create: true,
                draw_state_readback: true,
            },
        })
    }

    fn await_first_game_d3d_creation(
        &mut self,
        process_id: u32,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
        timeout_ms: u32,
    ) -> Result<EarlyFirstGameD3dProof, String> {
        let wire = decode_early_first_game_d3d(
            &self.recv(MessageKind::EarlyRenderFirstGameD3d, timeout_ms)?,
        )
        .map_err(|error| format!("decode early FIRST: {error:?}"))?;
        if wire.process_id != process_id
            || wire.process_start_nonce != process_start_nonce
            || wire.hook_epoch_nonce != hook_epoch_nonce
        {
            return Err("early FIRST names a foreign process/hook epoch".into());
        }
        Ok(EarlyFirstGameD3dProof {
            schema: EARLY_HOOK_READY_SCHEMA.into(),
            launch_manifest_sha256: hex(&wire.launch_manifest_sha256),
            process_id: wire.process_id,
            process_start_nonce: wire.process_start_nonce,
            hook_epoch_nonce: wire.hook_epoch_nonce,
            game_device_ordinal: wire.game_device_ordinal,
            first_creation_sequence: wire.first_creation_sequence,
            creation_qpc: wire.creation_qpc,
            staged_hook_generation: wire.staged_hook_generation,
            staged_hook_count: wire.staged_hook_count,
            diagnostic_stage_probe: wire.diagnostic_stage_probe,
            staged_hook_group_mask: wire.staged_hook_group_mask,
            staged_hook_set_sha256: hex(&wire.staged_hook_set_sha256),
            game_device_ptr: wire.game_device_ptr,
            immediate_context_ptr: wire.immediate_context_ptr,
            collector_null_probe_excluded: wire.collector_null_probe_excluded,
        })
    }

    fn await_runtime_dlc_roster(
        &mut self,
        process_id: u32,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
        staged_hook_generation: u64,
        game_device_ptr: u64,
        timeout_ms: u32,
    ) -> Result<EarlyRuntimeDlcRosterProof, String> {
        let wire = decode_early_render_runtime_dlc_roster(
            &self.recv(MessageKind::EarlyRenderRuntimeDlcRoster, timeout_ms)?,
        )
        .map_err(|error| format!("decode early runtime DLC roster: {error:?}"))?;
        let expected_launch = decode_hex_32(&self.manifest.sha256()?)
            .map_err(|error| format!("early launch digest: {error}"))?;
        let expected_vfs = decode_hex_32(&self.manifest.effective_vfs_manifest_sha256)
            .map_err(|error| format!("early effective-VFS digest: {error}"))?;
        if wire.launch_manifest_sha256 != expected_launch
            || wire.effective_vfs_manifest_sha256 != expected_vfs
            || wire.process_id != process_id
            || wire.process_start_nonce != process_start_nonce
            || wire.hook_epoch_nonce != hook_epoch_nonce
            || wire.staged_hook_generation != staged_hook_generation
            || wire.game_device_ptr != game_device_ptr
        {
            return Err("early runtime DLC roster names a foreign launch/hook/device epoch".into());
        }
        let effective = validate_effective_runtime_dlc_roster(
            AuthenticatedDlcVfsRoster {
                launch: &self.runtime_dlc_launch,
                descriptors: &self.runtime_dlc_descriptors,
            },
            &wire.raw_names,
        )?;
        Ok(EarlyRuntimeDlcRosterProof {
            schema: crate::runtime_dlc_roster::EFFECTIVE_RUNTIME_DLC_SCHEMA.into(),
            launch_manifest_sha256: hex(&wire.launch_manifest_sha256),
            effective_vfs_manifest_sha256: hex(&wire.effective_vfs_manifest_sha256),
            process_id: wire.process_id,
            process_start_nonce: wire.process_start_nonce,
            hook_epoch_nonce: wire.hook_epoch_nonce,
            staged_hook_generation: wire.staged_hook_generation,
            game_device_ptr: wire.game_device_ptr,
            manager_getter_rva: wire.manager_getter_rva,
            active_dlc_enumerator_rva: wire.active_dlc_enumerator_rva,
            string_vector_teardown_rva: wire.string_vector_teardown_rva,
            sample_count: wire.sample_count,
            double_stable: wire.double_stable,
            roster_count: wire.roster_count,
            raw_name_roster_sha256: hex(&wire.raw_name_roster_sha256),
            raw_names: wire.raw_names,
            effective_runtime_dlc_ids: effective.enabled_dlc_ids.into_iter().collect(),
            display_name_to_layer_id: effective.display_name_to_layer_id,
        })
    }

    fn request_diagnostic_liveness(
        &mut self,
        receipt: &EarlyRenderCaptureReceipt,
        required_hold_ms: u64,
    ) -> Result<EarlyDiagnosticLivenessProof, String> {
        receipt.validate()?;
        let staged_hook_group_mask = receipt
            .diagnostic_staged_hook_mask
            .ok_or("refusing diagnostic liveness request for a production receipt")?;
        let request_nonce = generate_secret_128()?;
        let request = EarlyDiagnosticLivenessRequestV1 {
            session_id: self.session_id,
            launch_manifest_sha256: decode_hex_32(&receipt.launch_manifest_sha256)
                .map_err(|error| format!("diagnostic launch digest: {error}"))?,
            process_id: receipt.process_id,
            process_start_nonce: receipt.process_start_nonce,
            hook_epoch_nonce: receipt.hook_epoch_nonce,
            game_device_ordinal: 1,
            first_creation_sequence: receipt.first_creation_sequence,
            creation_qpc: receipt.first_game_d3d_creation_qpc,
            staged_hook_generation: receipt.staged_hook_generation,
            staged_hook_count: receipt.staged_hook_count,
            staged_hook_group_mask,
            staged_hook_set_sha256: decode_hex_32(&receipt.staged_hook_set_sha256)
                .map_err(|error| format!("diagnostic staged hook digest: {error}"))?,
            game_device_ptr: receipt.game_device_ptr,
            immediate_context_ptr: receipt.immediate_context_ptr,
            required_hold_ms,
            request_nonce,
        };
        let request_payload = encode_early_diagnostic_liveness_request(&request)
            .map_err(|error| format!("encode early diagnostic liveness request: {error:?}"))?;
        if request_payload.len() != 190 {
            return Err(
                "early diagnostic liveness request is not the fixed 190-byte v1 payload".into(),
            );
        }
        let request_sha256 = Sha256::digest(&request_payload);
        self.channel
            .as_mut()
            .ok_or("early H4CT channel is not connected")?
            .send_until(
                MessageKind::EarlyRenderDiagnosticLivenessRequest,
                &request_payload,
                deadline(self.timeout_ms)?,
                &self.cancellation,
            )
            .map_err(|error| format!("send early diagnostic liveness request: {error}"))?;

        let response_payload = self.recv(
            MessageKind::EarlyRenderDiagnosticLivenessResponse,
            self.timeout_ms,
        )?;
        validate_diagnostic_liveness_response(
            receipt,
            self.session_id,
            &request,
            request_sha256,
            &response_payload,
        )
    }

    fn send_diagnostic_end(
        &mut self,
        receipt: &EarlyRenderCaptureReceipt,
        proof: &EarlyDiagnosticLivenessProof,
    ) -> Result<(), String> {
        proof.validate_for(receipt, proof.required_hold_ms)?;
        if proof.session_id != hex_bytes(&self.session_id) {
            return Err("diagnostic END proof names a foreign H4CT session".into());
        }
        let payload = encode_early_diagnostic_end(&EarlyDiagnosticEndV1 {
            session_id: self.session_id,
            process_id: receipt.process_id,
            process_start_nonce: receipt.process_start_nonce,
            hook_epoch_nonce: receipt.hook_epoch_nonce,
            request_nonce: decode_hex_16(&proof.request_nonce)
                .map_err(|error| format!("diagnostic request nonce: {error}"))?,
            response_sha256: decode_hex_32(&proof.response_sha256)
                .map_err(|error| format!("diagnostic response digest: {error}"))?,
            disposition: EARLY_DIAGNOSTIC_END_RECORDER_CANCEL,
        })
        .map_err(|error| format!("encode early diagnostic END: {error:?}"))?;
        if payload.len() != 92 {
            return Err("early diagnostic END is not the fixed 92-byte v1 payload".into());
        }
        self.channel
            .as_mut()
            .ok_or("early H4CT channel is not connected")?
            .send_until(
                MessageKind::EarlyRenderDiagnosticEnd,
                &payload,
                deadline(self.timeout_ms)?,
                &self.cancellation,
            )
            .map_err(|error| format!("send early diagnostic END: {error}"))
    }

    fn commit_capture_receipt(
        &mut self,
        process_id: u32,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
        receipt_sha256: &str,
    ) -> Result<(), String> {
        let payload = encode_early_commit_capture_receipt(&EarlyCommitCaptureReceiptV1 {
            process_id,
            process_start_nonce,
            hook_epoch_nonce,
            receipt_sha256: decode_hex_32(receipt_sha256)
                .map_err(|error| format!("early receipt digest: {error}"))?,
        })
        .map_err(|error| format!("encode early COMMIT: {error:?}"))?;
        self.channel
            .as_mut()
            .ok_or("early H4CT channel is not connected")?
            .send_until(
                MessageKind::EarlyRenderCommitReceipt,
                &payload,
                deadline(self.timeout_ms)?,
                &self.cancellation,
            )
            .map_err(|error| format!("send early COMMIT: {error}"))
    }
}

fn validate_diagnostic_liveness_response(
    receipt: &EarlyRenderCaptureReceipt,
    session_id: [u8; 16],
    request: &EarlyDiagnosticLivenessRequestV1,
    request_sha256: [u8; 32],
    response_payload: &[u8],
) -> Result<EarlyDiagnosticLivenessProof, String> {
    if response_payload.len() != 214 {
        return Err(
            "early diagnostic liveness response is not the fixed 214-byte v1 payload".into(),
        );
    }
    let response = decode_early_diagnostic_liveness_response(response_payload)
        .map_err(|error| format!("decode early diagnostic liveness response: {error:?}"))?;
    let expected_response_sha256 = Sha256::digest(&response_payload[..182]);
    if response.session_id != session_id
        || response.process_id != receipt.process_id
        || response.process_start_nonce != receipt.process_start_nonce
        || response.hook_epoch_nonce != receipt.hook_epoch_nonce
        || response.request_nonce != request.request_nonce
        || response.request_sha256 != request_sha256
        || response.sampled_qpc <= receipt.first_game_d3d_creation_qpc
        || response.observed_hold_ms < request.required_hold_ms
        || response.staged_hook_generation != receipt.staged_hook_generation
        || response.staged_hook_count != receipt.staged_hook_count
        || response.staged_hook_group_mask != request.staged_hook_group_mask
        || response.staged_hook_set_sha256 != request.staged_hook_set_sha256
        || response.game_device_ptr != receipt.game_device_ptr
        || response.immediate_context_ptr != receipt.immediate_context_ptr
        || response.response_sha256 != expected_response_sha256
    {
        return Err(
            "early diagnostic liveness response differs from the request/FIRST transcript".into(),
        );
    }
    let qpc_elapsed_ms = response
        .sampled_qpc
        .checked_sub(receipt.first_game_d3d_creation_qpc)
        .and_then(|ticks| ticks.checked_mul(1_000))
        .map(|ticks_ms| ticks_ms / response.qpc_frequency_hz)
        .ok_or("early diagnostic liveness QPC interval is invalid")?;
    if qpc_elapsed_ms < request.required_hold_ms {
        return Err(format!(
            "collector diagnostic QPC interval is only {qpc_elapsed_ms} ms; required {} ms",
            request.required_hold_ms
        ));
    }

    let proof = EarlyDiagnosticLivenessProof {
        schema: EARLY_DIAGNOSTIC_LIVENESS_SCHEMA.into(),
        session_id: hex_bytes(&session_id),
        launch_manifest_sha256: receipt.launch_manifest_sha256.clone(),
        process_id: response.process_id,
        process_start_nonce: response.process_start_nonce,
        hook_epoch_nonce: response.hook_epoch_nonce,
        game_device_ordinal: request.game_device_ordinal,
        first_creation_sequence: request.first_creation_sequence,
        creation_qpc: request.creation_qpc,
        staged_hook_generation: response.staged_hook_generation,
        staged_hook_count: response.staged_hook_count,
        staged_hook_group_mask: response.staged_hook_group_mask,
        staged_hook_set_sha256: hex(&response.staged_hook_set_sha256),
        game_device_ptr: response.game_device_ptr,
        immediate_context_ptr: response.immediate_context_ptr,
        required_hold_ms: request.required_hold_ms,
        request_nonce: hex_bytes(&request.request_nonce),
        request_sha256: hex(&request_sha256),
        sampled_qpc: response.sampled_qpc,
        qpc_frequency_hz: response.qpc_frequency_hz,
        observed_hold_ms: response.observed_hold_ms,
        response_sha256: hex(&response.response_sha256),
        end_disposition: EARLY_DIAGNOSTIC_END_RECORDER_CANCEL,
    };
    proof.validate_for(receipt, request.required_hold_ms)?;
    Ok(proof)
}

fn deadline(timeout_ms: u32) -> Result<ControlDeadline, String> {
    ControlDeadline::after(Duration::from_millis(u64::from(timeout_ms)))
        .map_err(|error| format!("early H4CT deadline: {error}"))
}

fn hex(bytes: &[u8; 32]) -> String {
    hex_bytes(bytes)
}

fn hex_bytes(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::early_render_launch::{EarlyHookCompleteness, EffectiveRuntimeDlcReceipt};
    use compact_contract::early_render_control::{
        decode_early_diagnostic_end, encode_early_diagnostic_liveness_response,
        EarlyDiagnosticLivenessResponseV1, EARLY_DIAGNOSTIC_MIN_HOLD_MS,
        EARLY_STAGED_HOOK_GENERATION,
    };
    use portable_content_manifest::effective_vfs::EffectiveVfsLayerMount;
    use portable_content_manifest::{ContentLayerKind, LayerStorageKind};
    use production_render_submit_live_lane::{RawEffectiveVfsWinnerRow, RawVfsContentIdentity};
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn diagnostic_receipt_fixture() -> EarlyRenderCaptureReceipt {
        EarlyRenderCaptureReceipt {
            schema: crate::early_render_launch::EARLY_CAPTURE_RECEIPT_SCHEMA.into(),
            launch_manifest_sha256: "11".repeat(32),
            effective_vfs_manifest_sha256: "22".repeat(32),
            effective_vfs_index_sha256: "33".repeat(32),
            effective_vfs_index_row_count: 1,
            effective_vfs_index_chunk_count: 1,
            content_manifest_bytes_sha256: "44".repeat(32),
            content_manifest_byte_count: 1,
            content_manifest_chunk_count: 1,
            process_id: 77,
            process_start_nonce: 88,
            executable_sha256: "55".repeat(32),
            collector_sha256: "66".repeat(32),
            created_suspended: true,
            hook_epoch_nonce: 99,
            hook_set_sha256: "77".repeat(32),
            hook_ready_sequence: 1,
            resume_sequence: 2,
            hook_ready_qpc: 10,
            resume_qpc: 20,
            first_creation_sequence: 3,
            first_game_d3d_creation_qpc: 1_000_000,
            staged_hook_generation: EARLY_STAGED_HOOK_GENERATION,
            staged_hook_count: 19,
            staged_hook_set_sha256: "88".repeat(32),
            diagnostic_staged_hook_mask: Some(0x01),
            game_device_ptr: 0x1111,
            immediate_context_ptr: 0x2222,
            effective_runtime_dlc: EffectiveRuntimeDlcReceipt {
                schema: crate::runtime_dlc_roster::EFFECTIVE_RUNTIME_DLC_SCHEMA.into(),
                source: "engine_active_dlc_manager_v1".into(),
                manager_getter_rva: 1,
                active_dlc_enumerator_rva: 2,
                string_vector_teardown_rva: 3,
                sample_count: 2,
                double_stable: true,
                active_display_names_engine_order: Vec::new(),
                enabled_dlc_ids: Vec::new(),
                display_name_to_layer_id: Default::default(),
                engine_order_sha256: "99".repeat(32),
            },
            game_d3d_creation_count_at_hook_ready: 0,
            preexisting_game_d3d_object_count: 0,
            completeness: EarlyHookCompleteness {
                all_shader_create: true,
                input_layout_create: true,
                blend_state_create: true,
                depth_stencil_state_create: true,
                rasterizer_state_create: true,
                sampler_state_create: true,
                buffer_create_update: true,
                texture_create_update: true,
                resource_view_create: true,
                draw_state_readback: true,
            },
            transcript_sha256: "aa".repeat(32),
        }
    }

    fn diagnostic_wire_fixture() -> (
        EarlyRenderCaptureReceipt,
        [u8; 16],
        EarlyDiagnosticLivenessRequestV1,
        [u8; 32],
        EarlyDiagnosticLivenessResponseV1,
    ) {
        let receipt = diagnostic_receipt_fixture();
        let session_id = [0x31; 16];
        let request = EarlyDiagnosticLivenessRequestV1 {
            session_id,
            launch_manifest_sha256: [0x11; 32],
            process_id: receipt.process_id,
            process_start_nonce: receipt.process_start_nonce,
            hook_epoch_nonce: receipt.hook_epoch_nonce,
            game_device_ordinal: 1,
            first_creation_sequence: receipt.first_creation_sequence,
            creation_qpc: receipt.first_game_d3d_creation_qpc,
            staged_hook_generation: receipt.staged_hook_generation,
            staged_hook_count: receipt.staged_hook_count,
            staged_hook_group_mask: receipt.diagnostic_staged_hook_mask.unwrap(),
            staged_hook_set_sha256: [0x88; 32],
            game_device_ptr: receipt.game_device_ptr,
            immediate_context_ptr: receipt.immediate_context_ptr,
            required_hold_ms: EARLY_DIAGNOSTIC_MIN_HOLD_MS,
            request_nonce: [0x42; 16],
        };
        let request_payload = encode_early_diagnostic_liveness_request(&request).unwrap();
        assert_eq!(request_payload.len(), 190);
        let request_sha256 = Sha256::digest(&request_payload);
        let response = EarlyDiagnosticLivenessResponseV1 {
            session_id,
            process_id: receipt.process_id,
            process_start_nonce: receipt.process_start_nonce,
            hook_epoch_nonce: receipt.hook_epoch_nonce,
            request_nonce: request.request_nonce,
            request_sha256,
            sampled_qpc: receipt.first_game_d3d_creation_qpc + 210_000,
            qpc_frequency_hz: 1_000,
            observed_hold_ms: EARLY_DIAGNOSTIC_MIN_HOLD_MS,
            staged_hook_generation: receipt.staged_hook_generation,
            staged_hook_count: receipt.staged_hook_count,
            staged_hook_group_mask: receipt.diagnostic_staged_hook_mask.unwrap(),
            staged_hook_set_sha256: [0x88; 32],
            game_device_ptr: receipt.game_device_ptr,
            immediate_context_ptr: receipt.immediate_context_ptr,
            response_sha256: [1; 32],
        };
        (receipt, session_id, request, request_sha256, response)
    }

    fn seal_diagnostic_response(mut response: EarlyDiagnosticLivenessResponseV1) -> Vec<u8> {
        response.response_sha256 = [1; 32];
        let provisional = encode_early_diagnostic_liveness_response(&response).unwrap();
        assert_eq!(provisional.len(), 214);
        response.response_sha256 = Sha256::digest(&provisional[..182]);
        encode_early_diagnostic_liveness_response(&response).unwrap()
    }

    #[test]
    fn diagnostic_liveness_roundtrip_hashes_exact_payload_ranges_and_binds_end() {
        let (receipt, session_id, request, request_sha256, response) = diagnostic_wire_fixture();
        let payload = seal_diagnostic_response(response);
        let proof = validate_diagnostic_liveness_response(
            &receipt,
            session_id,
            &request,
            request_sha256,
            &payload,
        )
        .unwrap();
        assert_eq!(
            proof.request_sha256,
            hex(&Sha256::digest(
                &encode_early_diagnostic_liveness_request(&request).unwrap()
            ))
        );
        assert_eq!(proof.response_sha256, hex(&Sha256::digest(&payload[..182])));
        let end = EarlyDiagnosticEndV1 {
            session_id,
            process_id: receipt.process_id,
            process_start_nonce: receipt.process_start_nonce,
            hook_epoch_nonce: receipt.hook_epoch_nonce,
            request_nonce: request.request_nonce,
            response_sha256: decode_hex_32(&proof.response_sha256).unwrap(),
            disposition: EARLY_DIAGNOSTIC_END_RECORDER_CANCEL,
        };
        let end_payload = encode_early_diagnostic_end(&end).unwrap();
        assert_eq!(end_payload.len(), 92);
        assert_eq!(decode_early_diagnostic_end(&end_payload).unwrap(), end);
    }

    #[test]
    fn diagnostic_liveness_rejects_every_mutated_first_or_transcript_binding() {
        let (receipt, session_id, request, request_sha256, response) = diagnostic_wire_fixture();
        let reject = |mutated: EarlyDiagnosticLivenessResponseV1| {
            let payload = seal_diagnostic_response(mutated);
            assert!(validate_diagnostic_liveness_response(
                &receipt,
                session_id,
                &request,
                request_sha256,
                &payload,
            )
            .is_err());
        };

        let mut mutated = response.clone();
        mutated.request_sha256[0] ^= 1;
        reject(mutated);
        let mut mutated = response.clone();
        mutated.staged_hook_group_mask = 0x03;
        mutated.staged_hook_count = 21;
        reject(mutated);
        let mut mutated = response.clone();
        mutated.staged_hook_set_sha256[0] ^= 1;
        reject(mutated);
        let mut mutated = response.clone();
        mutated.game_device_ptr += 1;
        reject(mutated);
        let mut mutated = response.clone();
        mutated.immediate_context_ptr += 1;
        reject(mutated);
        let mut mutated = response;
        mutated.sampled_qpc = receipt.first_game_d3d_creation_qpc + 209_999;
        reject(mutated);

        let (_, _, _, _, valid_response) = diagnostic_wire_fixture();
        let mut payload = seal_diagnostic_response(valid_response);
        payload[213] ^= 1;
        assert!(validate_diagnostic_liveness_response(
            &receipt,
            session_id,
            &request,
            request_sha256,
            &payload,
        )
        .is_err());
    }

    #[test]
    fn prepared_index_preserves_loose_and_archive_physical_authority() {
        let rows = vec![
            RawEffectiveVfsWinnerRow {
                content: RawVfsContentIdentity {
                    logical_path: "gfx/a.asset".into(),
                    winning_layer_ordinal: 0,
                    layer_kind: "base".into(),
                    layer_id: "base".into(),
                    source_sha256: "1".repeat(64),
                    payload_sha256: "1".repeat(64),
                    payload_bytes: 7,
                },
                source_container_sha256: None,
                source_byte_offset: 0,
                source_byte_count: 7,
                source_range_sha256: "1".repeat(64),
            },
            RawEffectiveVfsWinnerRow {
                content: RawVfsContentIdentity {
                    logical_path: "gfx/b.asset".into(),
                    winning_layer_ordinal: 1,
                    layer_kind: "mod".into(),
                    layer_id: "mod/archive.mod".into(),
                    source_sha256: "2".repeat(64),
                    payload_sha256: "3".repeat(64),
                    payload_bytes: 19,
                },
                source_container_sha256: Some("2".repeat(64)),
                source_byte_offset: 41,
                source_byte_count: 11,
                source_range_sha256: "4".repeat(64),
            },
        ];
        let binding = RawEffectiveVfsWinnerIndexBinding {
            process_start_nonce: 7,
            authority_manifest_sha256: "a".repeat(64),
            declared_row_count: rows.len() as u64,
            ordered_rows_sha256: RawEffectiveVfsWinnerIndexBinding::canonical_rows_sha256(&rows)
                .unwrap(),
            ordered_rows: rows,
        };
        let prepared = PreparedEarlyRenderVfsIndex::from_winner_binding(&binding).unwrap();
        assert_eq!(prepared.row_count(), 2);
        assert_eq!(prepared.chunk_count(), 1);
        assert!(prepared.index_sha256().iter().any(|byte| *byte != 0));
        let archive = &prepared.rows[1];
        assert_eq!(archive.source_container_sha256, Some([0x22; 32]));
        assert_eq!(archive.source_byte_offset, 41);
        assert_eq!(archive.source_byte_count, 11);
        assert_eq!(archive.source_range_sha256, [0x44; 32]);
        assert_ne!(archive.source_byte_count, archive.payload_bytes);
        assert_ne!(archive.source_range_sha256, archive.payload_sha256);
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "early-h4ct-payload-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn responder_resolves_loose_and_archive_ranges_and_rejects_tamper() {
        let root = temp_root("exact");
        let base = root.join("base");
        fs::create_dir_all(base.join("gfx/replaced")).unwrap();
        fs::write(base.join("gfx/loose.asset"), b"loose-payload").unwrap();
        fs::write(base.join("gfx/replaced/old.asset"), b"must-not-resolve").unwrap();
        let archive_path = root.join("override.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&archive_path).unwrap());
        writer
            .start_file(
                "gfx/archive.asset",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"archive-payload").unwrap();
        writer.finish().unwrap();
        let archive_sha = Sha256::digest(&fs::read(&archive_path).unwrap());
        let authority = "a".repeat(64);
        let vfs = EffectiveVfs::new(
            authority.clone(),
            [
                EffectiveVfsLayerMount {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: base,
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                },
                EffectiveVfsLayerMount {
                    ordinal: 1,
                    layer_kind: ContentLayerKind::Mod,
                    layer_id: "mod/test.mod".into(),
                    storage_kind: LayerStorageKind::ZipArchive,
                    local_source: archive_path,
                    source_container_sha256: Some(hex(&archive_sha)),
                    replace_paths: vec!["gfx/replaced".into()],
                },
            ],
        )
        .unwrap();
        assert!(vfs.resolve("gfx/replaced/old.asset").unwrap().is_none());

        let archive_payload = b"archive-payload";
        let loose_payload = b"loose-payload";
        let archive_payload_sha = Sha256::digest(archive_payload);
        let loose_payload_sha = Sha256::digest(loose_payload);
        let raw_rows = vec![
            RawEffectiveVfsWinnerRow {
                content: RawVfsContentIdentity {
                    logical_path: "gfx/archive.asset".into(),
                    winning_layer_ordinal: 1,
                    layer_kind: "mod".into(),
                    layer_id: "mod/test.mod".into(),
                    source_sha256: hex(&archive_sha),
                    payload_sha256: hex(&archive_payload_sha),
                    payload_bytes: archive_payload.len() as u64,
                },
                source_container_sha256: Some(hex(&archive_sha)),
                source_byte_offset: 1,
                source_byte_count: 3,
                source_range_sha256: "b".repeat(64),
            },
            RawEffectiveVfsWinnerRow {
                content: RawVfsContentIdentity {
                    logical_path: "gfx/loose.asset".into(),
                    winning_layer_ordinal: 0,
                    layer_kind: "base".into(),
                    layer_id: "base".into(),
                    source_sha256: hex(&loose_payload_sha),
                    payload_sha256: hex(&loose_payload_sha),
                    payload_bytes: loose_payload.len() as u64,
                },
                source_container_sha256: None,
                source_byte_offset: 0,
                source_byte_count: loose_payload.len() as u64,
                source_range_sha256: hex(&loose_payload_sha),
            },
        ];
        let binding = RawEffectiveVfsWinnerIndexBinding {
            process_start_nonce: 8,
            authority_manifest_sha256: authority.clone(),
            declared_row_count: 2,
            ordered_rows_sha256: RawEffectiveVfsWinnerIndexBinding::canonical_rows_sha256(
                &raw_rows,
            )
            .unwrap(),
            ordered_rows: raw_rows,
        };
        let index = PreparedEarlyRenderVfsIndex::from_winner_binding(&binding).unwrap();
        let session_id = [3; 16];
        let launch_bind_sha256 = [4; 32];
        let manifest = EarlyRenderLaunchManifest {
            schema: "fixture".into(),
            executable_sha256: "1".repeat(64),
            collector_sha256: "2".repeat(64),
            effective_vfs_manifest_sha256: authority,
            ordered_content_layers: Vec::new(),
            requested_profile_selection:
                crate::early_render_launch::RequestedProfileSelectionReceipt {
                    schema: crate::runtime_dlc_roster::REQUESTED_PROFILE_SELECTION_SCHEMA.into(),
                    dlc_load_sha256: "6".repeat(64),
                    disabled_dlc_descriptor_ids: Vec::new(),
                    requested_enabled_dlc_ids: Vec::new(),
                },
            effective_runtime_dlc_candidate_ids: Vec::new(),
            localisation_language: "english".into(),
            required_settings: Default::default(),
            semantic_arguments: Vec::new(),
        };
        let host = PreparedEarlyRenderHostChannel {
            prepared: None,
            channel: None,
            cancellation: ControlCancellation::new(),
            session_id,
            expectation: ProofExpectation {
                pid: 7,
                creation_filetime: 8,
                h4cr_version: compact_contract::COMPACT_RECORD_VERSION,
                dll_sha256: [2; 32],
                challenge: session_id,
            },
            manifest,
            content_manifest_bytes: vec![1],
            content_manifest_bytes_sha256: [5; 32],
            content_manifest_chunk_count: 1,
            effective_vfs_index: index,
            effective_vfs: vfs,
            runtime_dlc_launch: PortableLaunchBinding {
                executable_sha256: "1".repeat(64),
                ordered_mods: Vec::new(),
                enabled_dlc_ids: Default::default(),
                enabled_dlc_layers: Default::default(),
                integrated_dlc_layers: Default::default(),
                localisation_language: "english".into(),
                required_settings: Default::default(),
            },
            runtime_dlc_descriptors: Vec::new(),
            launch_bind_sha256: Some(launch_bind_sha256),
            timeout_ms: 1000,
        };
        for (request_index, (winner_index, offset, count, expected)) in [
            (0usize, 2u64, 7u64, b"chive-p".as_slice()),
            (1, 0, 5, b"loose"),
        ]
        .into_iter()
        .enumerate()
        {
            let request = EarlyRenderVfsPayloadRequestV1 {
                process_id: 7,
                process_start_nonce: 8,
                session_id,
                launch_bind_sha256,
                effective_vfs_manifest_sha256: [0xaa; 32],
                effective_vfs_index_sha256: host.effective_vfs_index.index_sha256(),
                request_id: request_index as u64 + 1,
                payload_offset: offset,
                payload_count: count,
                expected_payload_range_sha256: Sha256::digest(expected),
                winner: host.effective_vfs_index.rows[winner_index].clone(),
            };
            let range = host.resolve_vfs_payload_request(&request).unwrap();
            assert_eq!(range, expected);
            let (chunks, commit) =
                PreparedEarlyRenderHostChannel::encode_vfs_payload_response(&request, &range)
                    .unwrap();
            assert_eq!(chunks.len(), 1);
            let decoded =
                compact_contract::early_render_vfs_payload::decode_early_render_vfs_payload_chunk(
                    &chunks[0],
                )
                .unwrap();
            assert_eq!(decoded.bytes, expected);
            let decoded_commit =
                compact_contract::early_render_vfs_payload::decode_early_render_vfs_payload_commit(
                    &commit,
                )
                .unwrap();
            assert_eq!(
                decoded_commit.payload_range_sha256,
                Sha256::digest(expected)
            );

            let mut tampered = request.clone();
            tampered.launch_bind_sha256[0] ^= 1;
            assert!(host.resolve_vfs_payload_request(&tampered).is_err());
            tampered = request;
            tampered.winner.payload_sha256[0] ^= 1;
            assert!(host.resolve_vfs_payload_request(&tampered).is_err());
        }
        let cancellation = host.cancellation_handle();
        cancellation.cancel();
        assert!(cancellation.is_cancelled());
        drop(host);
        fs::remove_dir_all(root).unwrap();
    }
}
