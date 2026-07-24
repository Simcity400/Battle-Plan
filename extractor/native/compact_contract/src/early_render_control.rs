//! Fixed H4CT payloads for exact early d77/d80 capture.
//!
//! All hashes are raw 32-byte SHA-256 values. The enclosing H4CT frame binds
//! session, direction, sequence, length, and CRC; these payloads additionally
//! bind the suspended process epoch and hook epoch.

const VERSION: u16 = 1;
const READY_MAGIC: &[u8; 4] = b"ERDY";
const FIRST_MAGIC: &[u8; 4] = b"EFD3";
const COMMIT_MAGIC: &[u8; 4] = b"ECMT";
const LAUNCH_BIND_MAGIC: &[u8; 4] = b"ELBD";
const LAUNCH_BIND_V2_MAGIC: &[u8; 4] = b"ELB2";
const DIAGNOSTIC_LIVENESS_REQUEST_MAGIC: &[u8; 4] = b"EDRQ";
const DIAGNOSTIC_LIVENESS_RESPONSE_MAGIC: &[u8; 4] = b"EDRP";
const DIAGNOSTIC_END_MAGIC: &[u8; 4] = b"EDND";
const G2_CAPTURE_ARM_MAGIC: &[u8; 4] = b"EG2A";
pub const EARLY_HOOK_COMPLETENESS_MASK: u16 = 0x03ff;
pub const EARLY_STAGED_HOOK_GENERATION: u64 = 1;
pub const EARLY_STAGED_HOOK_GROUP_COUNTS: [u32; 10] = [19, 2, 3, 8, 17, 7, 7, 3, 2, 3];
pub const EARLY_STAGED_HOOK_COUNT: u32 = 19 + 2 + 3 + 8 + 17 + 7 + 7 + 3 + 2 + 3;
pub const EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK: u64 = 0x3ff;
pub const EARLY_DIAGNOSTIC_MIN_HOLD_MS: u64 = 210_000;
pub const EARLY_DIAGNOSTIC_END_RECORDER_CANCEL: u16 = 1;
pub const MAX_EARLY_RENDER_TIMEOUT_MS: u32 = 1_800_000;

pub fn staged_hook_count_for_group_mask(group_mask: u64) -> Option<u32> {
    if group_mask & !EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK != 0 || group_mask & 1 == 0 {
        return None;
    }
    Some(
        EARLY_STAGED_HOOK_GROUP_COUNTS
            .into_iter()
            .enumerate()
            .filter_map(|(bit, count)| (group_mask & (1u64 << bit) != 0).then_some(count))
            .sum(),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyHookReadyV1 {
    pub launch_manifest_sha256: [u8; 32],
    pub effective_vfs_manifest_sha256: [u8; 32],
    pub effective_vfs_index_sha256: [u8; 32],
    pub effective_vfs_index_row_count: u32,
    pub effective_vfs_index_chunk_count: u32,
    pub content_manifest_bytes_sha256: [u8; 32],
    pub content_manifest_byte_count: u64,
    pub content_manifest_chunk_count: u32,
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub executable_sha256: [u8; 32],
    pub collector_sha256: [u8; 32],
    pub hook_epoch_nonce: u64,
    pub hook_set_sha256: [u8; 32],
    pub hook_ready_sequence: u64,
    pub hook_ready_qpc: u64,
    pub game_d3d_creation_count_at_hook_ready: u64,
    pub preexisting_game_d3d_object_count: u64,
    pub completeness_bits: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyFirstGameD3dV1 {
    pub launch_manifest_sha256: [u8; 32],
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub game_device_ordinal: u64,
    pub first_creation_sequence: u64,
    pub creation_qpc: u64,
    pub staged_hook_generation: u64,
    pub staged_hook_count: u32,
    pub diagnostic_stage_probe: bool,
    pub staged_hook_group_mask: u64,
    pub staged_hook_set_sha256: [u8; 32],
    pub game_device_ptr: u64,
    pub immediate_context_ptr: u64,
    pub collector_null_probe_excluded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyCommitCaptureReceiptV1 {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub receipt_sha256: [u8; 32],
}

/// Recorder-authored authority transmitted only after the collector hello has
/// been admitted. This is the first point at which the injected collector can
/// learn the host's in-memory launch and effective-VFS identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderLaunchBindV1 {
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
    pub executable_sha256: [u8; 32],
    pub collector_sha256: [u8; 32],
}

/// Versioned successor to [`EarlyRenderLaunchBindV1`]. The recorder knows
/// whether G2 was requested while the primary thread is still suspended; this
/// bit lets the collector configure exact pre-arm monitoring before hooks are
/// installed instead of taxing every ordinary production capture forever.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderLaunchBindV2 {
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
    pub executable_sha256: [u8; 32],
    pub collector_sha256: [u8; 32],
    pub g2_capture_expected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyDiagnosticLivenessRequestV1 {
    pub session_id: [u8; 16],
    pub launch_manifest_sha256: [u8; 32],
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub game_device_ordinal: u64,
    pub first_creation_sequence: u64,
    pub creation_qpc: u64,
    pub staged_hook_generation: u64,
    pub staged_hook_count: u32,
    pub staged_hook_group_mask: u64,
    pub staged_hook_set_sha256: [u8; 32],
    pub game_device_ptr: u64,
    pub immediate_context_ptr: u64,
    pub required_hold_ms: u64,
    pub request_nonce: [u8; 16],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyDiagnosticLivenessResponseV1 {
    pub session_id: [u8; 16],
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub request_nonce: [u8; 16],
    pub request_sha256: [u8; 32],
    pub sampled_qpc: u64,
    pub qpc_frequency_hz: u64,
    pub observed_hold_ms: u64,
    pub staged_hook_generation: u64,
    pub staged_hook_count: u32,
    pub staged_hook_group_mask: u64,
    pub staged_hook_set_sha256: [u8; 32],
    pub game_device_ptr: u64,
    pub immediate_context_ptr: u64,
    pub response_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyDiagnosticEndV1 {
    pub session_id: [u8; 16],
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub request_nonce: [u8; 16],
    pub response_sha256: [u8; 32],
    pub disposition: u16,
}

/// Recorder-authored, H4CT-authenticated request for the existing collector
/// D3D hook owner to retain one pre-first-UI G2 backbuffer witness. Local paths
/// are transport-only; every portable identity is an exact digest or token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyG2CaptureArmV1 {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub launch_manifest_sha256: [u8; 32],
    pub launch_epoch_sha256: [u8; 32],
    pub capture_receipt_sha256: [u8; 32],
    pub authority_exe_sha256: [u8; 32],
    pub effective_vfs_manifest_sha256: [u8; 32],
    pub content_manifest_digest_sha256: [u8; 32],
    pub settings_file_sha256: [u8; 32],
    pub camera_payload_sha256: [u8; 32],
    pub capture_context_sha256: [u8; 32],
    pub expected_selected_hour: i64,
    pub observation_id: String,
    pub localisation_language: String,
    pub pov_country_or_global: String,
    pub capture_context_path: String,
    pub output_directory: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EarlyRenderCodecError {
    Length,
    Magic,
    Version,
    Reserved,
    InvalidField,
}

fn header(out: &mut Vec<u8>, magic: &[u8; 4]) {
    out.extend_from_slice(magic);
    out.extend_from_slice(&VERSION.to_le_bytes());
}
fn u16_at(bytes: &[u8], at: &mut usize) -> Result<u16, EarlyRenderCodecError> {
    let value = u16::from_le_bytes(
        bytes
            .get(*at..*at + 2)
            .ok_or(EarlyRenderCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at += 2;
    Ok(value)
}
fn u32_at(bytes: &[u8], at: &mut usize) -> Result<u32, EarlyRenderCodecError> {
    let value = u32::from_le_bytes(
        bytes
            .get(*at..*at + 4)
            .ok_or(EarlyRenderCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at += 4;
    Ok(value)
}
fn u64_at(bytes: &[u8], at: &mut usize) -> Result<u64, EarlyRenderCodecError> {
    let value = u64::from_le_bytes(
        bytes
            .get(*at..*at + 8)
            .ok_or(EarlyRenderCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at += 8;
    Ok(value)
}
fn hash_at(bytes: &[u8], at: &mut usize) -> Result<[u8; 32], EarlyRenderCodecError> {
    let value = bytes
        .get(*at..*at + 32)
        .ok_or(EarlyRenderCodecError::Length)?
        .try_into()
        .unwrap();
    *at += 32;
    Ok(value)
}
fn bytes16_at(bytes: &[u8], at: &mut usize) -> Result<[u8; 16], EarlyRenderCodecError> {
    let value = bytes
        .get(*at..*at + 16)
        .ok_or(EarlyRenderCodecError::Length)?
        .try_into()
        .unwrap();
    *at += 16;
    Ok(value)
}
fn validate_header(
    bytes: &[u8],
    magic: &[u8; 4],
    expected: usize,
) -> Result<usize, EarlyRenderCodecError> {
    if bytes.len() != expected {
        return Err(EarlyRenderCodecError::Length);
    }
    if bytes.get(..4) != Some(magic) {
        return Err(EarlyRenderCodecError::Magic);
    }
    let mut at = 4;
    if u16_at(bytes, &mut at)? != VERSION {
        return Err(EarlyRenderCodecError::Version);
    }
    Ok(at)
}
fn nonzero_hash(value: &[u8; 32]) -> bool {
    value.iter().any(|byte| *byte != 0)
}

const G2_MAX_TOKEN_BYTES: usize = 512;
const G2_MAX_PATH_BYTES: usize = 32 * 1024;

fn put_string(
    out: &mut Vec<u8>,
    value: &str,
    max_bytes: usize,
) -> Result<(), EarlyRenderCodecError> {
    if value.is_empty()
        || value.len() > max_bytes
        || value.as_bytes().contains(&0)
        || value.chars().any(char::is_control)
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let length = u32::try_from(value.len()).map_err(|_| EarlyRenderCodecError::Length)?;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn string_at(
    bytes: &[u8],
    at: &mut usize,
    max_bytes: usize,
) -> Result<String, EarlyRenderCodecError> {
    let length = usize::try_from(u32_at(bytes, at)?).map_err(|_| EarlyRenderCodecError::Length)?;
    if length == 0 || length > max_bytes {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let end = (*at)
        .checked_add(length)
        .ok_or(EarlyRenderCodecError::Length)?;
    let value = bytes
        .get(*at..end)
        .ok_or(EarlyRenderCodecError::Length)?;
    *at += length;
    let value = std::str::from_utf8(value).map_err(|_| EarlyRenderCodecError::InvalidField)?;
    if value.as_bytes().contains(&0) || value.chars().any(char::is_control) {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    Ok(value.to_owned())
}

fn validate_g2_arm(value: &EarlyG2CaptureArmV1) -> Result<(), EarlyRenderCodecError> {
    if value.process_id == 0
        || value.process_start_nonce == 0
        || value.hook_epoch_nonce == 0
        || value.expected_selected_hour < 0
        || [
            &value.launch_manifest_sha256,
            &value.launch_epoch_sha256,
            &value.capture_receipt_sha256,
            &value.authority_exe_sha256,
            &value.effective_vfs_manifest_sha256,
            &value.content_manifest_digest_sha256,
            &value.settings_file_sha256,
            &value.camera_payload_sha256,
            &value.capture_context_sha256,
        ]
        .into_iter()
        .any(|hash| !nonzero_hash(hash))
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    for (value, max) in [
        (value.observation_id.as_str(), G2_MAX_TOKEN_BYTES),
        (value.localisation_language.as_str(), G2_MAX_TOKEN_BYTES),
        (value.pov_country_or_global.as_str(), G2_MAX_TOKEN_BYTES),
        (value.capture_context_path.as_str(), G2_MAX_PATH_BYTES),
        (value.output_directory.as_str(), G2_MAX_PATH_BYTES),
    ] {
        let mut sink = Vec::new();
        put_string(&mut sink, value, max)?;
    }
    Ok(())
}

pub fn encode_early_g2_capture_arm(
    value: &EarlyG2CaptureArmV1,
) -> Result<Vec<u8>, EarlyRenderCodecError> {
    validate_g2_arm(value)?;
    let mut out = Vec::new();
    header(&mut out, G2_CAPTURE_ARM_MAGIC);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.hook_epoch_nonce.to_le_bytes());
    for hash in [
        &value.launch_manifest_sha256,
        &value.launch_epoch_sha256,
        &value.capture_receipt_sha256,
        &value.authority_exe_sha256,
        &value.effective_vfs_manifest_sha256,
        &value.content_manifest_digest_sha256,
        &value.settings_file_sha256,
        &value.camera_payload_sha256,
        &value.capture_context_sha256,
    ] {
        out.extend_from_slice(hash);
    }
    out.extend_from_slice(&value.expected_selected_hour.to_le_bytes());
    put_string(&mut out, &value.observation_id, G2_MAX_TOKEN_BYTES)?;
    put_string(
        &mut out,
        &value.localisation_language,
        G2_MAX_TOKEN_BYTES,
    )?;
    put_string(
        &mut out,
        &value.pov_country_or_global,
        G2_MAX_TOKEN_BYTES,
    )?;
    put_string(&mut out, &value.capture_context_path, G2_MAX_PATH_BYTES)?;
    put_string(&mut out, &value.output_directory, G2_MAX_PATH_BYTES)?;
    Ok(out)
}

pub fn decode_early_g2_capture_arm(
    bytes: &[u8],
) -> Result<EarlyG2CaptureArmV1, EarlyRenderCodecError> {
    if bytes.len() < 6 || bytes.get(..4) != Some(G2_CAPTURE_ARM_MAGIC) {
        return Err(if bytes.len() < 6 {
            EarlyRenderCodecError::Length
        } else {
            EarlyRenderCodecError::Magic
        });
    }
    let mut at = 4;
    if u16_at(bytes, &mut at)? != VERSION {
        return Err(EarlyRenderCodecError::Version);
    }
    let value = EarlyG2CaptureArmV1 {
        process_id: u32_at(bytes, &mut at)?,
        process_start_nonce: u64_at(bytes, &mut at)?,
        hook_epoch_nonce: u64_at(bytes, &mut at)?,
        launch_manifest_sha256: hash_at(bytes, &mut at)?,
        launch_epoch_sha256: hash_at(bytes, &mut at)?,
        capture_receipt_sha256: hash_at(bytes, &mut at)?,
        authority_exe_sha256: hash_at(bytes, &mut at)?,
        effective_vfs_manifest_sha256: hash_at(bytes, &mut at)?,
        content_manifest_digest_sha256: hash_at(bytes, &mut at)?,
        settings_file_sha256: hash_at(bytes, &mut at)?,
        camera_payload_sha256: hash_at(bytes, &mut at)?,
        capture_context_sha256: hash_at(bytes, &mut at)?,
        expected_selected_hour: i64::from_le_bytes(
            bytes
                .get(at..at + 8)
                .ok_or(EarlyRenderCodecError::Length)?
                .try_into()
                .unwrap(),
        ),
        observation_id: {
            at += 8;
            string_at(bytes, &mut at, G2_MAX_TOKEN_BYTES)?
        },
        localisation_language: string_at(bytes, &mut at, G2_MAX_TOKEN_BYTES)?,
        pov_country_or_global: string_at(bytes, &mut at, G2_MAX_TOKEN_BYTES)?,
        capture_context_path: string_at(bytes, &mut at, G2_MAX_PATH_BYTES)?,
        output_directory: string_at(bytes, &mut at, G2_MAX_PATH_BYTES)?,
    };
    if at != bytes.len() {
        return Err(EarlyRenderCodecError::Length);
    }
    validate_g2_arm(&value)?;
    Ok(value)
}

pub fn encode_early_hook_ready(value: &EarlyHookReadyV1) -> Result<Vec<u8>, EarlyRenderCodecError> {
    if value.process_id == 0
        || value.process_start_nonce == 0
        || value.hook_epoch_nonce == 0
        || value.hook_ready_sequence == 0
        || value.hook_ready_qpc == 0
        || value.effective_vfs_index_row_count == 0
        || value.effective_vfs_index_chunk_count == 0
        || value.effective_vfs_index_chunk_count > value.effective_vfs_index_row_count
        || value.content_manifest_byte_count == 0
        || value.content_manifest_chunk_count == 0
        || value.game_d3d_creation_count_at_hook_ready != 0
        || value.preexisting_game_d3d_object_count != 0
        || value.completeness_bits != EARLY_HOOK_COMPLETENESS_MASK
        || ![
            &value.launch_manifest_sha256,
            &value.effective_vfs_manifest_sha256,
            &value.effective_vfs_index_sha256,
            &value.content_manifest_bytes_sha256,
            &value.executable_sha256,
            &value.collector_sha256,
            &value.hook_set_sha256,
        ]
        .into_iter()
        .all(|hash| nonzero_hash(hash))
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(304);
    header(&mut out, READY_MAGIC);
    out.extend_from_slice(&value.launch_manifest_sha256);
    out.extend_from_slice(&value.effective_vfs_manifest_sha256);
    out.extend_from_slice(&value.effective_vfs_index_sha256);
    out.extend_from_slice(&value.effective_vfs_index_row_count.to_le_bytes());
    out.extend_from_slice(&value.effective_vfs_index_chunk_count.to_le_bytes());
    out.extend_from_slice(&value.content_manifest_bytes_sha256);
    out.extend_from_slice(&value.content_manifest_byte_count.to_le_bytes());
    out.extend_from_slice(&value.content_manifest_chunk_count.to_le_bytes());
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.executable_sha256);
    out.extend_from_slice(&value.collector_sha256);
    out.extend_from_slice(&value.hook_epoch_nonce.to_le_bytes());
    out.extend_from_slice(&value.hook_set_sha256);
    out.extend_from_slice(&value.hook_ready_sequence.to_le_bytes());
    out.extend_from_slice(&value.hook_ready_qpc.to_le_bytes());
    out.extend_from_slice(&value.game_d3d_creation_count_at_hook_ready.to_le_bytes());
    out.extend_from_slice(&value.preexisting_game_d3d_object_count.to_le_bytes());
    out.extend_from_slice(&value.completeness_bits.to_le_bytes());
    Ok(out)
}

pub fn decode_early_hook_ready(bytes: &[u8]) -> Result<EarlyHookReadyV1, EarlyRenderCodecError> {
    let mut at = validate_header(bytes, READY_MAGIC, 304)?;
    let value = EarlyHookReadyV1 {
        launch_manifest_sha256: hash_at(bytes, &mut at)?,
        effective_vfs_manifest_sha256: hash_at(bytes, &mut at)?,
        effective_vfs_index_sha256: hash_at(bytes, &mut at)?,
        effective_vfs_index_row_count: u32_at(bytes, &mut at)?,
        effective_vfs_index_chunk_count: u32_at(bytes, &mut at)?,
        content_manifest_bytes_sha256: hash_at(bytes, &mut at)?,
        content_manifest_byte_count: u64_at(bytes, &mut at)?,
        content_manifest_chunk_count: u32_at(bytes, &mut at)?,
        process_id: u32_at(bytes, &mut at)?,
        process_start_nonce: u64_at(bytes, &mut at)?,
        executable_sha256: hash_at(bytes, &mut at)?,
        collector_sha256: hash_at(bytes, &mut at)?,
        hook_epoch_nonce: u64_at(bytes, &mut at)?,
        hook_set_sha256: hash_at(bytes, &mut at)?,
        hook_ready_sequence: u64_at(bytes, &mut at)?,
        hook_ready_qpc: u64_at(bytes, &mut at)?,
        game_d3d_creation_count_at_hook_ready: u64_at(bytes, &mut at)?,
        preexisting_game_d3d_object_count: u64_at(bytes, &mut at)?,
        completeness_bits: u16_at(bytes, &mut at)?,
    };
    encode_early_hook_ready(&value)?;
    Ok(value)
}

pub fn encode_early_first_game_d3d(
    value: &EarlyFirstGameD3dV1,
) -> Result<Vec<u8>, EarlyRenderCodecError> {
    if value.process_id == 0
        || value.process_start_nonce == 0
        || value.hook_epoch_nonce == 0
        || value.game_device_ordinal != 1
        || value.first_creation_sequence == 0
        || value.creation_qpc == 0
        || value.staged_hook_generation != EARLY_STAGED_HOOK_GENERATION
        || staged_hook_count_for_group_mask(value.staged_hook_group_mask)
            != Some(value.staged_hook_count)
        || (!value.diagnostic_stage_probe
            && (value.staged_hook_group_mask != EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK
                || value.staged_hook_count != EARLY_STAGED_HOOK_COUNT))
        || (value.diagnostic_stage_probe
            && value.staged_hook_group_mask == EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK)
        || !nonzero_hash(&value.staged_hook_set_sha256)
        || value.game_device_ptr == 0
        || value.immediate_context_ptr == 0
        || !value.collector_null_probe_excluded
        || !nonzero_hash(&value.launch_manifest_sha256)
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(152);
    header(&mut out, FIRST_MAGIC);
    out.extend_from_slice(&value.launch_manifest_sha256);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.hook_epoch_nonce.to_le_bytes());
    out.extend_from_slice(&value.game_device_ordinal.to_le_bytes());
    out.extend_from_slice(&value.first_creation_sequence.to_le_bytes());
    out.extend_from_slice(&value.creation_qpc.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_generation.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_count.to_le_bytes());
    out.push(u8::from(value.diagnostic_stage_probe));
    out.extend_from_slice(&value.staged_hook_group_mask.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_set_sha256);
    out.extend_from_slice(&value.game_device_ptr.to_le_bytes());
    out.extend_from_slice(&value.immediate_context_ptr.to_le_bytes());
    out.push(u8::from(value.collector_null_probe_excluded));
    Ok(out)
}

pub fn decode_early_first_game_d3d(
    bytes: &[u8],
) -> Result<EarlyFirstGameD3dV1, EarlyRenderCodecError> {
    let mut at = validate_header(bytes, FIRST_MAGIC, 152)?;
    let value = EarlyFirstGameD3dV1 {
        launch_manifest_sha256: hash_at(bytes, &mut at)?,
        process_id: u32_at(bytes, &mut at)?,
        process_start_nonce: u64_at(bytes, &mut at)?,
        hook_epoch_nonce: u64_at(bytes, &mut at)?,
        game_device_ordinal: u64_at(bytes, &mut at)?,
        first_creation_sequence: u64_at(bytes, &mut at)?,
        creation_qpc: u64_at(bytes, &mut at)?,
        staged_hook_generation: u64_at(bytes, &mut at)?,
        staged_hook_count: u32_at(bytes, &mut at)?,
        diagnostic_stage_probe: match bytes[at] {
            0 => false,
            1 => true,
            _ => return Err(EarlyRenderCodecError::Reserved),
        },
        staged_hook_group_mask: {
            at += 1;
            u64_at(bytes, &mut at)?
        },
        staged_hook_set_sha256: hash_at(bytes, &mut at)?,
        game_device_ptr: u64_at(bytes, &mut at)?,
        immediate_context_ptr: u64_at(bytes, &mut at)?,
        collector_null_probe_excluded: match bytes[at] {
            0 => false,
            1 => true,
            _ => return Err(EarlyRenderCodecError::Reserved),
        },
    };
    encode_early_first_game_d3d(&value)?;
    Ok(value)
}

pub fn encode_early_commit_capture_receipt(
    value: &EarlyCommitCaptureReceiptV1,
) -> Result<Vec<u8>, EarlyRenderCodecError> {
    if value.process_id == 0
        || value.process_start_nonce == 0
        || value.hook_epoch_nonce == 0
        || !nonzero_hash(&value.receipt_sha256)
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(58);
    header(&mut out, COMMIT_MAGIC);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.hook_epoch_nonce.to_le_bytes());
    out.extend_from_slice(&value.receipt_sha256);
    Ok(out)
}

pub fn decode_early_commit_capture_receipt(
    bytes: &[u8],
) -> Result<EarlyCommitCaptureReceiptV1, EarlyRenderCodecError> {
    let mut at = validate_header(bytes, COMMIT_MAGIC, 58)?;
    let value = EarlyCommitCaptureReceiptV1 {
        process_id: u32_at(bytes, &mut at)?,
        process_start_nonce: u64_at(bytes, &mut at)?,
        hook_epoch_nonce: u64_at(bytes, &mut at)?,
        receipt_sha256: hash_at(bytes, &mut at)?,
    };
    encode_early_commit_capture_receipt(&value)?;
    Ok(value)
}

pub fn encode_early_diagnostic_liveness_request(
    value: &EarlyDiagnosticLivenessRequestV1,
) -> Result<Vec<u8>, EarlyRenderCodecError> {
    if value.session_id == [0; 16]
        || value.process_id == 0
        || value.process_start_nonce == 0
        || value.hook_epoch_nonce == 0
        || value.game_device_ordinal != 1
        || value.first_creation_sequence == 0
        || value.creation_qpc == 0
        || value.staged_hook_generation != EARLY_STAGED_HOOK_GENERATION
        || staged_hook_count_for_group_mask(value.staged_hook_group_mask)
            != Some(value.staged_hook_count)
        || value.staged_hook_group_mask == EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK
        || !nonzero_hash(&value.staged_hook_set_sha256)
        || value.game_device_ptr == 0
        || value.immediate_context_ptr == 0
        || value.required_hold_ms < EARLY_DIAGNOSTIC_MIN_HOLD_MS
        || value.request_nonce == [0; 16]
        || !nonzero_hash(&value.launch_manifest_sha256)
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(190);
    header(&mut out, DIAGNOSTIC_LIVENESS_REQUEST_MAGIC);
    out.extend_from_slice(&value.session_id);
    out.extend_from_slice(&value.launch_manifest_sha256);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.hook_epoch_nonce.to_le_bytes());
    out.extend_from_slice(&value.game_device_ordinal.to_le_bytes());
    out.extend_from_slice(&value.first_creation_sequence.to_le_bytes());
    out.extend_from_slice(&value.creation_qpc.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_generation.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_count.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_group_mask.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_set_sha256);
    out.extend_from_slice(&value.game_device_ptr.to_le_bytes());
    out.extend_from_slice(&value.immediate_context_ptr.to_le_bytes());
    out.extend_from_slice(&value.required_hold_ms.to_le_bytes());
    out.extend_from_slice(&value.request_nonce);
    Ok(out)
}

pub fn decode_early_diagnostic_liveness_request(
    bytes: &[u8],
) -> Result<EarlyDiagnosticLivenessRequestV1, EarlyRenderCodecError> {
    let mut at = validate_header(bytes, DIAGNOSTIC_LIVENESS_REQUEST_MAGIC, 190)?;
    let value = EarlyDiagnosticLivenessRequestV1 {
        session_id: bytes16_at(bytes, &mut at)?,
        launch_manifest_sha256: hash_at(bytes, &mut at)?,
        process_id: u32_at(bytes, &mut at)?,
        process_start_nonce: u64_at(bytes, &mut at)?,
        hook_epoch_nonce: u64_at(bytes, &mut at)?,
        game_device_ordinal: u64_at(bytes, &mut at)?,
        first_creation_sequence: u64_at(bytes, &mut at)?,
        creation_qpc: u64_at(bytes, &mut at)?,
        staged_hook_generation: u64_at(bytes, &mut at)?,
        staged_hook_count: u32_at(bytes, &mut at)?,
        staged_hook_group_mask: u64_at(bytes, &mut at)?,
        staged_hook_set_sha256: hash_at(bytes, &mut at)?,
        game_device_ptr: u64_at(bytes, &mut at)?,
        immediate_context_ptr: u64_at(bytes, &mut at)?,
        required_hold_ms: u64_at(bytes, &mut at)?,
        request_nonce: bytes16_at(bytes, &mut at)?,
    };
    encode_early_diagnostic_liveness_request(&value)?;
    Ok(value)
}

/// Canonical bytes authenticated by `EarlyDiagnosticLivenessResponseV1::response_sha256`.
/// This is exactly bytes 0..182 of the 214-byte EDRP payload: the complete
/// encoded response through `immediate_context_ptr`, excluding only the final
/// 32-byte `response_sha256`. The digest is SHA-256 of these bytes directly;
/// there is no domain prefix and it is not a 214-byte zero-filled encoding.
pub fn canonical_early_diagnostic_liveness_response_binding_bytes(
    value: &EarlyDiagnosticLivenessResponseV1,
) -> Result<Vec<u8>, EarlyRenderCodecError> {
    if value.session_id == [0; 16]
        || value.process_id == 0
        || value.process_start_nonce == 0
        || value.hook_epoch_nonce == 0
        || value.request_nonce == [0; 16]
        || !nonzero_hash(&value.request_sha256)
        || value.sampled_qpc == 0
        || value.qpc_frequency_hz == 0
        || value.observed_hold_ms < EARLY_DIAGNOSTIC_MIN_HOLD_MS
        || value.staged_hook_generation != EARLY_STAGED_HOOK_GENERATION
        || staged_hook_count_for_group_mask(value.staged_hook_group_mask)
            != Some(value.staged_hook_count)
        || value.staged_hook_group_mask == EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK
        || !nonzero_hash(&value.staged_hook_set_sha256)
        || value.game_device_ptr == 0
        || value.immediate_context_ptr == 0
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(182);
    header(&mut out, DIAGNOSTIC_LIVENESS_RESPONSE_MAGIC);
    out.extend_from_slice(&value.session_id);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.hook_epoch_nonce.to_le_bytes());
    out.extend_from_slice(&value.request_nonce);
    out.extend_from_slice(&value.request_sha256);
    out.extend_from_slice(&value.sampled_qpc.to_le_bytes());
    out.extend_from_slice(&value.qpc_frequency_hz.to_le_bytes());
    out.extend_from_slice(&value.observed_hold_ms.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_generation.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_count.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_group_mask.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_set_sha256);
    out.extend_from_slice(&value.game_device_ptr.to_le_bytes());
    out.extend_from_slice(&value.immediate_context_ptr.to_le_bytes());
    debug_assert_eq!(out.len(), 182);
    Ok(out)
}

pub fn encode_early_diagnostic_liveness_response(
    value: &EarlyDiagnosticLivenessResponseV1,
) -> Result<Vec<u8>, EarlyRenderCodecError> {
    if !nonzero_hash(&value.response_sha256) {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let mut out = canonical_early_diagnostic_liveness_response_binding_bytes(value)?;
    out.reserve_exact(32);
    out.extend_from_slice(&value.response_sha256);
    debug_assert_eq!(out.len(), 214);
    Ok(out)
}

pub fn decode_early_diagnostic_liveness_response(
    bytes: &[u8],
) -> Result<EarlyDiagnosticLivenessResponseV1, EarlyRenderCodecError> {
    let mut at = validate_header(bytes, DIAGNOSTIC_LIVENESS_RESPONSE_MAGIC, 214)?;
    let value = EarlyDiagnosticLivenessResponseV1 {
        session_id: bytes16_at(bytes, &mut at)?,
        process_id: u32_at(bytes, &mut at)?,
        process_start_nonce: u64_at(bytes, &mut at)?,
        hook_epoch_nonce: u64_at(bytes, &mut at)?,
        request_nonce: bytes16_at(bytes, &mut at)?,
        request_sha256: hash_at(bytes, &mut at)?,
        sampled_qpc: u64_at(bytes, &mut at)?,
        qpc_frequency_hz: u64_at(bytes, &mut at)?,
        observed_hold_ms: u64_at(bytes, &mut at)?,
        staged_hook_generation: u64_at(bytes, &mut at)?,
        staged_hook_count: u32_at(bytes, &mut at)?,
        staged_hook_group_mask: u64_at(bytes, &mut at)?,
        staged_hook_set_sha256: hash_at(bytes, &mut at)?,
        game_device_ptr: u64_at(bytes, &mut at)?,
        immediate_context_ptr: u64_at(bytes, &mut at)?,
        response_sha256: hash_at(bytes, &mut at)?,
    };
    encode_early_diagnostic_liveness_response(&value)?;
    Ok(value)
}

pub fn encode_early_diagnostic_end(
    value: &EarlyDiagnosticEndV1,
) -> Result<Vec<u8>, EarlyRenderCodecError> {
    if value.session_id == [0; 16]
        || value.process_id == 0
        || value.process_start_nonce == 0
        || value.hook_epoch_nonce == 0
        || value.request_nonce == [0; 16]
        || !nonzero_hash(&value.response_sha256)
        || value.disposition != EARLY_DIAGNOSTIC_END_RECORDER_CANCEL
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(92);
    header(&mut out, DIAGNOSTIC_END_MAGIC);
    out.extend_from_slice(&value.session_id);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.hook_epoch_nonce.to_le_bytes());
    out.extend_from_slice(&value.request_nonce);
    out.extend_from_slice(&value.response_sha256);
    out.extend_from_slice(&value.disposition.to_le_bytes());
    Ok(out)
}

pub fn decode_early_diagnostic_end(
    bytes: &[u8],
) -> Result<EarlyDiagnosticEndV1, EarlyRenderCodecError> {
    let mut at = validate_header(bytes, DIAGNOSTIC_END_MAGIC, 92)?;
    let value = EarlyDiagnosticEndV1 {
        session_id: bytes16_at(bytes, &mut at)?,
        process_id: u32_at(bytes, &mut at)?,
        process_start_nonce: u64_at(bytes, &mut at)?,
        hook_epoch_nonce: u64_at(bytes, &mut at)?,
        request_nonce: bytes16_at(bytes, &mut at)?,
        response_sha256: hash_at(bytes, &mut at)?,
        disposition: u16_at(bytes, &mut at)?,
    };
    encode_early_diagnostic_end(&value)?;
    Ok(value)
}

pub fn encode_early_render_launch_bind(
    value: &EarlyRenderLaunchBindV1,
) -> Result<Vec<u8>, EarlyRenderCodecError> {
    if value.process_id == 0
        || value.process_start_nonce == 0
        || !(1..=MAX_EARLY_RENDER_TIMEOUT_MS).contains(&value.timeout_ms)
        || value.effective_vfs_index_row_count == 0
        || value.effective_vfs_index_chunk_count == 0
        || value.effective_vfs_index_chunk_count > value.effective_vfs_index_row_count
        || value.content_manifest_byte_count == 0
        || value.content_manifest_chunk_count == 0
        || [
            &value.launch_manifest_sha256,
            &value.effective_vfs_manifest_sha256,
            &value.effective_vfs_index_sha256,
            &value.content_manifest_bytes_sha256,
            &value.executable_sha256,
            &value.collector_sha256,
        ]
        .into_iter()
        .any(|hash| !nonzero_hash(hash))
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(234);
    header(&mut out, LAUNCH_BIND_MAGIC);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.timeout_ms.to_le_bytes());
    out.extend_from_slice(&value.launch_manifest_sha256);
    out.extend_from_slice(&value.effective_vfs_manifest_sha256);
    out.extend_from_slice(&value.effective_vfs_index_sha256);
    out.extend_from_slice(&value.effective_vfs_index_row_count.to_le_bytes());
    out.extend_from_slice(&value.effective_vfs_index_chunk_count.to_le_bytes());
    out.extend_from_slice(&value.content_manifest_bytes_sha256);
    out.extend_from_slice(&value.content_manifest_byte_count.to_le_bytes());
    out.extend_from_slice(&value.content_manifest_chunk_count.to_le_bytes());
    out.extend_from_slice(&value.executable_sha256);
    out.extend_from_slice(&value.collector_sha256);
    Ok(out)
}

pub fn decode_early_render_launch_bind(
    bytes: &[u8],
) -> Result<EarlyRenderLaunchBindV1, EarlyRenderCodecError> {
    let mut at = validate_header(bytes, LAUNCH_BIND_MAGIC, 234)?;
    let value = EarlyRenderLaunchBindV1 {
        process_id: u32_at(bytes, &mut at)?,
        process_start_nonce: u64_at(bytes, &mut at)?,
        timeout_ms: u32_at(bytes, &mut at)?,
        launch_manifest_sha256: hash_at(bytes, &mut at)?,
        effective_vfs_manifest_sha256: hash_at(bytes, &mut at)?,
        effective_vfs_index_sha256: hash_at(bytes, &mut at)?,
        effective_vfs_index_row_count: u32_at(bytes, &mut at)?,
        effective_vfs_index_chunk_count: u32_at(bytes, &mut at)?,
        content_manifest_bytes_sha256: hash_at(bytes, &mut at)?,
        content_manifest_byte_count: u64_at(bytes, &mut at)?,
        content_manifest_chunk_count: u32_at(bytes, &mut at)?,
        executable_sha256: hash_at(bytes, &mut at)?,
        collector_sha256: hash_at(bytes, &mut at)?,
    };
    encode_early_render_launch_bind(&value)?;
    Ok(value)
}

pub fn encode_early_render_launch_bind_v2(
    value: &EarlyRenderLaunchBindV2,
) -> Result<Vec<u8>, EarlyRenderCodecError> {
    if value.process_id == 0
        || value.process_start_nonce == 0
        || !(1..=MAX_EARLY_RENDER_TIMEOUT_MS).contains(&value.timeout_ms)
        || value.effective_vfs_index_row_count == 0
        || value.effective_vfs_index_chunk_count == 0
        || value.effective_vfs_index_chunk_count > value.effective_vfs_index_row_count
        || value.content_manifest_byte_count == 0
        || value.content_manifest_chunk_count == 0
        || [
            &value.launch_manifest_sha256,
            &value.effective_vfs_manifest_sha256,
            &value.effective_vfs_index_sha256,
            &value.content_manifest_bytes_sha256,
            &value.executable_sha256,
            &value.collector_sha256,
        ]
        .into_iter()
        .any(|hash| !nonzero_hash(hash))
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(235);
    header(&mut out, LAUNCH_BIND_V2_MAGIC);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.timeout_ms.to_le_bytes());
    out.extend_from_slice(&value.launch_manifest_sha256);
    out.extend_from_slice(&value.effective_vfs_manifest_sha256);
    out.extend_from_slice(&value.effective_vfs_index_sha256);
    out.extend_from_slice(&value.effective_vfs_index_row_count.to_le_bytes());
    out.extend_from_slice(&value.effective_vfs_index_chunk_count.to_le_bytes());
    out.extend_from_slice(&value.content_manifest_bytes_sha256);
    out.extend_from_slice(&value.content_manifest_byte_count.to_le_bytes());
    out.extend_from_slice(&value.content_manifest_chunk_count.to_le_bytes());
    out.extend_from_slice(&value.executable_sha256);
    out.extend_from_slice(&value.collector_sha256);
    out.push(u8::from(value.g2_capture_expected));
    Ok(out)
}

pub fn decode_early_render_launch_bind_v2(
    bytes: &[u8],
) -> Result<EarlyRenderLaunchBindV2, EarlyRenderCodecError> {
    let mut at = validate_header(bytes, LAUNCH_BIND_V2_MAGIC, 235)?;
    let process_id = u32_at(bytes, &mut at)?;
    let process_start_nonce = u64_at(bytes, &mut at)?;
    let timeout_ms = u32_at(bytes, &mut at)?;
    let launch_manifest_sha256 = hash_at(bytes, &mut at)?;
    let effective_vfs_manifest_sha256 = hash_at(bytes, &mut at)?;
    let effective_vfs_index_sha256 = hash_at(bytes, &mut at)?;
    let effective_vfs_index_row_count = u32_at(bytes, &mut at)?;
    let effective_vfs_index_chunk_count = u32_at(bytes, &mut at)?;
    let content_manifest_bytes_sha256 = hash_at(bytes, &mut at)?;
    let content_manifest_byte_count = u64_at(bytes, &mut at)?;
    let content_manifest_chunk_count = u32_at(bytes, &mut at)?;
    let executable_sha256 = hash_at(bytes, &mut at)?;
    let collector_sha256 = hash_at(bytes, &mut at)?;
    let g2_capture_expected = match bytes.get(at).copied() {
        Some(0) => false,
        Some(1) => true,
        Some(_) => return Err(EarlyRenderCodecError::InvalidField),
        None => return Err(EarlyRenderCodecError::Length),
    };
    let value = EarlyRenderLaunchBindV2 {
        process_id,
        process_start_nonce,
        timeout_ms,
        launch_manifest_sha256,
        effective_vfs_manifest_sha256,
        effective_vfs_index_sha256,
        effective_vfs_index_row_count,
        effective_vfs_index_chunk_count,
        content_manifest_bytes_sha256,
        content_manifest_byte_count,
        content_manifest_chunk_count,
        executable_sha256,
        collector_sha256,
        g2_capture_expected,
    };
    encode_early_render_launch_bind_v2(&value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn production_sink_verifier_count_matches_wire_contract() {
        let verifier = include_str!("../../scripts/verify_sink_reconciliation.py");
        assert!(verifier.contains(&format!(
            "EARLY_STAGED_HOOK_COUNT = {EARLY_STAGED_HOOK_COUNT}"
        )));
        assert!(verifier.contains(
            "receipt.get(\"staged_hook_count\") == EARLY_STAGED_HOOK_COUNT"
        ));
    }

    #[test]
    fn exact_round_trips_and_tamper_rejects() {
        let ready = EarlyHookReadyV1 {
            launch_manifest_sha256: [1; 32],
            effective_vfs_manifest_sha256: [2; 32],
            effective_vfs_index_sha256: [6; 32],
            effective_vfs_index_row_count: 3,
            effective_vfs_index_chunk_count: 2,
            content_manifest_bytes_sha256: [7; 32],
            content_manifest_byte_count: 123,
            content_manifest_chunk_count: 1,
            process_id: 7,
            process_start_nonce: 8,
            executable_sha256: [3; 32],
            collector_sha256: [4; 32],
            hook_epoch_nonce: 9,
            hook_set_sha256: [5; 32],
            hook_ready_sequence: 10,
            hook_ready_qpc: 11,
            game_d3d_creation_count_at_hook_ready: 0,
            preexisting_game_d3d_object_count: 0,
            completeness_bits: EARLY_HOOK_COMPLETENESS_MASK,
        };
        let bytes = encode_early_hook_ready(&ready).unwrap();
        assert_eq!(decode_early_hook_ready(&bytes).unwrap(), ready);
        let mut tampered = bytes;
        let completeness_offset = tampered.len() - 2;
        tampered[completeness_offset] = 0;
        assert!(decode_early_hook_ready(&tampered).is_err());
        let first = EarlyFirstGameD3dV1 {
            launch_manifest_sha256: [1; 32],
            process_id: 7,
            process_start_nonce: 8,
            hook_epoch_nonce: 9,
            game_device_ordinal: 1,
            first_creation_sequence: 13,
            creation_qpc: 12,
            staged_hook_generation: EARLY_STAGED_HOOK_GENERATION,
            staged_hook_count: EARLY_STAGED_HOOK_COUNT,
            diagnostic_stage_probe: false,
            staged_hook_group_mask: EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK,
            staged_hook_set_sha256: [8; 32],
            game_device_ptr: 0x1000,
            immediate_context_ptr: 0x2000,
            collector_null_probe_excluded: true,
        };
        assert_eq!(
            decode_early_first_game_d3d(&encode_early_first_game_d3d(&first).unwrap()).unwrap(),
            first
        );
        let mut diagnostic_first = first.clone();
        diagnostic_first.diagnostic_stage_probe = true;
        diagnostic_first.staged_hook_group_mask = 0x01;
        diagnostic_first.staged_hook_count = 19;
        assert_eq!(
            decode_early_first_game_d3d(&encode_early_first_game_d3d(&diagnostic_first).unwrap())
                .unwrap(),
            diagnostic_first
        );
        diagnostic_first.staged_hook_group_mask = 0x02;
        assert_eq!(
            encode_early_first_game_d3d(&diagnostic_first),
            Err(EarlyRenderCodecError::InvalidField)
        );
        diagnostic_first.staged_hook_group_mask = 0x01;
        let request = EarlyDiagnosticLivenessRequestV1 {
            session_id: [9; 16],
            launch_manifest_sha256: diagnostic_first.launch_manifest_sha256,
            process_id: diagnostic_first.process_id,
            process_start_nonce: diagnostic_first.process_start_nonce,
            hook_epoch_nonce: diagnostic_first.hook_epoch_nonce,
            game_device_ordinal: diagnostic_first.game_device_ordinal,
            first_creation_sequence: diagnostic_first.first_creation_sequence,
            creation_qpc: diagnostic_first.creation_qpc,
            staged_hook_generation: diagnostic_first.staged_hook_generation,
            staged_hook_count: diagnostic_first.staged_hook_count,
            staged_hook_group_mask: diagnostic_first.staged_hook_group_mask,
            staged_hook_set_sha256: diagnostic_first.staged_hook_set_sha256,
            game_device_ptr: diagnostic_first.game_device_ptr,
            immediate_context_ptr: diagnostic_first.immediate_context_ptr,
            required_hold_ms: EARLY_DIAGNOSTIC_MIN_HOLD_MS,
            request_nonce: [10; 16],
        };
        assert_eq!(
            decode_early_diagnostic_liveness_request(
                &encode_early_diagnostic_liveness_request(&request).unwrap()
            )
            .unwrap(),
            request
        );
        let mut response = EarlyDiagnosticLivenessResponseV1 {
            session_id: request.session_id,
            process_id: request.process_id,
            process_start_nonce: request.process_start_nonce,
            hook_epoch_nonce: request.hook_epoch_nonce,
            request_nonce: request.request_nonce,
            request_sha256: [11; 32],
            sampled_qpc: 500,
            qpc_frequency_hz: 10_000_000,
            observed_hold_ms: request.required_hold_ms,
            staged_hook_generation: request.staged_hook_generation,
            staged_hook_count: request.staged_hook_count,
            staged_hook_group_mask: request.staged_hook_group_mask,
            staged_hook_set_sha256: request.staged_hook_set_sha256,
            game_device_ptr: request.game_device_ptr,
            immediate_context_ptr: request.immediate_context_ptr,
            response_sha256: [0; 32],
        };
        let response_binding =
            canonical_early_diagnostic_liveness_response_binding_bytes(&response).unwrap();
        assert_eq!(response_binding.len(), 182);
        response.response_sha256 = Sha256::digest(&response_binding).into();
        let encoded_response = encode_early_diagnostic_liveness_response(&response).unwrap();
        assert_eq!(&encoded_response[..182], response_binding.as_slice());
        assert_eq!(&encoded_response[182..], &response.response_sha256);
        assert_eq!(
            decode_early_diagnostic_liveness_response(&encoded_response).unwrap(),
            response
        );
        let end = EarlyDiagnosticEndV1 {
            session_id: request.session_id,
            process_id: request.process_id,
            process_start_nonce: request.process_start_nonce,
            hook_epoch_nonce: request.hook_epoch_nonce,
            request_nonce: request.request_nonce,
            response_sha256: response.response_sha256,
            disposition: EARLY_DIAGNOSTIC_END_RECORDER_CANCEL,
        };
        assert_eq!(
            decode_early_diagnostic_end(&encode_early_diagnostic_end(&end).unwrap()).unwrap(),
            end
        );
        let mut premature = request.clone();
        premature.required_hold_ms = EARLY_DIAGNOSTIC_MIN_HOLD_MS - 1;
        assert_eq!(
            encode_early_diagnostic_liveness_request(&premature),
            Err(EarlyRenderCodecError::InvalidField)
        );
        let mut invalid_first = first.clone();
        invalid_first.first_creation_sequence = 0;
        assert_eq!(
            encode_early_first_game_d3d(&invalid_first),
            Err(EarlyRenderCodecError::InvalidField)
        );
        let commit = EarlyCommitCaptureReceiptV1 {
            process_id: 7,
            process_start_nonce: 8,
            hook_epoch_nonce: 9,
            receipt_sha256: [6; 32],
        };
        assert_eq!(
            decode_early_commit_capture_receipt(
                &encode_early_commit_capture_receipt(&commit).unwrap()
            )
            .unwrap(),
            commit
        );
        let g2 = EarlyG2CaptureArmV1 {
            process_id: 7,
            process_start_nonce: 8,
            hook_epoch_nonce: 9,
            launch_manifest_sha256: [1; 32],
            launch_epoch_sha256: [2; 32],
            capture_receipt_sha256: commit.receipt_sha256,
            authority_exe_sha256: [3; 32],
            effective_vfs_manifest_sha256: [4; 32],
            content_manifest_digest_sha256: [5; 32],
            settings_file_sha256: [6; 32],
            camera_payload_sha256: [7; 32],
            capture_context_sha256: [8; 32],
            expected_selected_hour: 12,
            observation_id: "g2-fixture".into(),
            localisation_language: "english".into(),
            pov_country_or_global: "global".into(),
            capture_context_path: r"C:\capture\context.json".into(),
            output_directory: r"C:\capture\g2".into(),
        };
        let encoded_g2 = encode_early_g2_capture_arm(&g2).unwrap();
        assert_eq!(decode_early_g2_capture_arm(&encoded_g2).unwrap(), g2);
        let mut trailing = encoded_g2;
        trailing.push(0);
        assert_eq!(
            decode_early_g2_capture_arm(&trailing),
            Err(EarlyRenderCodecError::Length)
        );
        let bind = EarlyRenderLaunchBindV1 {
            process_id: 7,
            process_start_nonce: 8,
            timeout_ms: 30_000,
            launch_manifest_sha256: [1; 32],
            effective_vfs_manifest_sha256: [2; 32],
            effective_vfs_index_sha256: [5; 32],
            effective_vfs_index_row_count: 3,
            effective_vfs_index_chunk_count: 2,
            content_manifest_bytes_sha256: [6; 32],
            content_manifest_byte_count: 123,
            content_manifest_chunk_count: 1,
            executable_sha256: [3; 32],
            collector_sha256: [4; 32],
        };
        assert_eq!(
            decode_early_render_launch_bind(&encode_early_render_launch_bind(&bind).unwrap())
                .unwrap(),
            bind
        );
        let bind_v2 = EarlyRenderLaunchBindV2 {
            process_id: bind.process_id,
            process_start_nonce: bind.process_start_nonce,
            timeout_ms: bind.timeout_ms,
            launch_manifest_sha256: bind.launch_manifest_sha256,
            effective_vfs_manifest_sha256: bind.effective_vfs_manifest_sha256,
            effective_vfs_index_sha256: bind.effective_vfs_index_sha256,
            effective_vfs_index_row_count: bind.effective_vfs_index_row_count,
            effective_vfs_index_chunk_count: bind.effective_vfs_index_chunk_count,
            content_manifest_bytes_sha256: bind.content_manifest_bytes_sha256,
            content_manifest_byte_count: bind.content_manifest_byte_count,
            content_manifest_chunk_count: bind.content_manifest_chunk_count,
            executable_sha256: bind.executable_sha256,
            collector_sha256: bind.collector_sha256,
            g2_capture_expected: true,
        };
        let bind_v1_bytes = encode_early_render_launch_bind(&bind).unwrap();
        let bind_v2_bytes = encode_early_render_launch_bind_v2(&bind_v2).unwrap();
        assert_eq!(
            decode_early_render_launch_bind_v2(&bind_v2_bytes).unwrap(),
            bind_v2
        );
        assert_eq!(
            decode_early_render_launch_bind_v2(&bind_v1_bytes),
            Err(EarlyRenderCodecError::Length)
        );
        assert_eq!(
            decode_early_render_launch_bind(&bind_v2_bytes),
            Err(EarlyRenderCodecError::Length)
        );
        let mut noncanonical_bool = bind_v2_bytes;
        *noncanonical_bool.last_mut().unwrap() = 2;
        assert_eq!(
            decode_early_render_launch_bind_v2(&noncanonical_bool),
            Err(EarlyRenderCodecError::InvalidField)
        );
        let mut invalid = bind.clone();
        invalid.timeout_ms = 0;
        assert_eq!(
            encode_early_render_launch_bind(&invalid),
            Err(EarlyRenderCodecError::InvalidField)
        );
        invalid.timeout_ms = MAX_EARLY_RENDER_TIMEOUT_MS + 1;
        assert_eq!(
            encode_early_render_launch_bind(&invalid),
            Err(EarlyRenderCodecError::InvalidField)
        );
    }
}
