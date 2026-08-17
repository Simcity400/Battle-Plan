//! Fixed H4CT payloads for exact early d77/d80 capture.
//!
//! All hashes are raw 32-byte SHA-256 values. The enclosing H4CT frame binds
//! session, direction, sequence, length, and CRC; these payloads additionally
//! bind the suspended process epoch and hook epoch.

const VERSION_V1: u16 = 1;
const VERSION_V2: u16 = 2;
const READY_MAGIC: &[u8; 4] = b"ERDY";
const FIRST_MAGIC: &[u8; 4] = b"EFD3";
const COMMIT_MAGIC: &[u8; 4] = b"ECMT";
const LAUNCH_BIND_V2_MAGIC: &[u8; 4] = b"ELB2";
const DIAGNOSTIC_LIVENESS_REQUEST_MAGIC: &[u8; 4] = b"EDRQ";
const DIAGNOSTIC_LIVENESS_RESPONSE_MAGIC: &[u8; 4] = b"EDRP";
const DIAGNOSTIC_END_MAGIC: &[u8; 4] = b"EDND";
pub const EARLY_HOOK_COMPLETENESS_MASK: u16 = 0x03ff;
pub const EARLY_STAGED_HOOK_GENERATION: u64 = 1;
pub const EARLY_STAGED_HOOK_GROUP_COUNTS: [u32; 10] = [19, 2, 3, 8, 17, 7, 7, 3, 2, 3];
pub const EARLY_STAGED_HOOK_COUNT: u32 = 19 + 2 + 3 + 8 + 17 + 7 + 7 + 3 + 2 + 3;
pub const EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK: u64 = 0x3ff;
pub const EARLY_DIAGNOSTIC_COM_HOOK_COUNT: u32 = 19;
pub const EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT: u32 = 32;
pub const EARLY_DIAGNOSTIC_COM_SELECTOR_VALUE_MASK: u64 =
    (1u64 << EARLY_DIAGNOSTIC_COM_HOOK_COUNT) - 1;
pub const EARLY_DIAGNOSTIC_COM_SELECTOR_WIRE_MASK: u64 =
    EARLY_DIAGNOSTIC_COM_SELECTOR_VALUE_MASK << EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT;
pub const EARLY_DIAGNOSTIC_MIN_HOLD_MS: u64 = 210_000;
pub const EARLY_DIAGNOSTIC_END_RECORDER_CANCEL: u16 = 1;
pub const MAX_EARLY_RENDER_TIMEOUT_MS: u32 = 1_800_000;
/// Process-environment key the recorder writes into a diagnostic child and the
/// collector reads back to cross-check its control-block staged-hook
/// selection. Both ends previously spelled this key as their own literal; it
/// is a two-party agreement, so it lives with the rest of the early-render
/// control contract.
pub const DIAGNOSTIC_STAGE_MASK_ENV: &str = "BATTLE_PLAN_XN15L_DIAGNOSTIC_STAGE_MASK";

/// Typed staged-hook inventory selected for one early-render run.
///
/// `Empty` is represented only by wire mask zero and is admitted only in v2
/// staged-hook frames. Production retains its sole historical wire mask
/// (`0x3ff`); an explicit all-COM selector remains noncanonical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarlyStagedHookSelection {
    Production,
    Diagnostic { group_mask: u64, com_selector: u64 },
    Empty,
}

impl EarlyStagedHookSelection {
    pub fn from_wire_mask(wire_mask: u64) -> Option<Self> {
        if wire_mask == 0 {
            return Some(Self::Empty);
        }
        let allowed =
            EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK | EARLY_DIAGNOSTIC_COM_SELECTOR_WIRE_MASK;
        if wire_mask & !allowed != 0 {
            return None;
        }
        let group_mask = wire_mask & EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK;
        let com_selector = (wire_mask >> EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT)
            & EARLY_DIAGNOSTIC_COM_SELECTOR_VALUE_MASK;
        if group_mask & 1 == 0 || com_selector == EARLY_DIAGNOSTIC_COM_SELECTOR_VALUE_MASK {
            return None;
        }
        if group_mask == EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK && com_selector == 0 {
            Some(Self::Production)
        } else {
            Some(Self::Diagnostic {
                group_mask,
                com_selector,
            })
        }
    }

    pub fn to_wire_mask(self) -> Option<u64> {
        match self {
            Self::Production => Some(EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK),
            Self::Empty => Some(0),
            Self::Diagnostic {
                group_mask,
                com_selector,
            } => {
                if group_mask & !EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK != 0
                    || group_mask & 1 == 0
                    || com_selector & !EARLY_DIAGNOSTIC_COM_SELECTOR_VALUE_MASK != 0
                    || com_selector == EARLY_DIAGNOSTIC_COM_SELECTOR_VALUE_MASK
                    || (group_mask == EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK && com_selector == 0)
                {
                    return None;
                }
                Some(group_mask | (com_selector << EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT))
            }
        }
    }

    pub fn group_mask(self) -> u64 {
        match self {
            Self::Production => EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK,
            Self::Diagnostic { group_mask, .. } => group_mask,
            Self::Empty => 0,
        }
    }

    pub fn com_selector(self) -> u64 {
        match self {
            Self::Diagnostic { com_selector, .. } => com_selector,
            Self::Production | Self::Empty => 0,
        }
    }

    pub fn staged_hook_count(self) -> u32 {
        match self {
            Self::Empty => 0,
            Self::Production => EARLY_STAGED_HOOK_COUNT,
            Self::Diagnostic {
                group_mask,
                com_selector,
            } => {
                let com_count = if com_selector == 0 {
                    EARLY_DIAGNOSTIC_COM_HOOK_COUNT
                } else {
                    com_selector.count_ones()
                };
                com_count
                    + EARLY_STAGED_HOOK_GROUP_COUNTS
                        .into_iter()
                        .enumerate()
                        .skip(1)
                        .filter_map(|(bit, count)| {
                            (group_mask & (1u64 << bit) != 0).then_some(count)
                        })
                        .sum::<u32>()
            }
        }
    }

    pub fn is_diagnostic(self) -> bool {
        !matches!(self, Self::Production)
    }

    pub fn is_empty(self) -> bool {
        matches!(self, Self::Empty)
    }
}

/// Splits the packed staged-hook selection carried in the historical
/// `staged_hook_group_mask` field. Empty is the unique `(0, 0)` selection.
pub fn staged_hook_selection_masks(wire_mask: u64) -> Option<(u64, u64)> {
    let selection = EarlyStagedHookSelection::from_wire_mask(wire_mask)?;
    Some((selection.group_mask(), selection.com_selector()))
}

pub fn staged_hook_count_for_group_mask(wire_mask: u64) -> Option<u32> {
    Some(EarlyStagedHookSelection::from_wire_mask(wire_mask)?.staged_hook_count())
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
/// The authenticated recorder-to-collector launch binding.
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EarlyRenderCodecError {
    Length,
    Magic,
    Version,
    Reserved,
    InvalidField,
}

fn header(out: &mut Vec<u8>, magic: &[u8; 4]) {
    header_version(out, magic, VERSION_V1);
}

fn header_version(out: &mut Vec<u8>, magic: &[u8; 4], version: u16) {
    out.extend_from_slice(magic);
    out.extend_from_slice(&version.to_le_bytes());
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
    if u16_at(bytes, &mut at)? != VERSION_V1 {
        return Err(EarlyRenderCodecError::Version);
    }
    Ok(at)
}

fn validate_staged_header(
    bytes: &[u8],
    magic: &[u8; 4],
    expected: usize,
) -> Result<(usize, u16), EarlyRenderCodecError> {
    if bytes.len() != expected {
        return Err(EarlyRenderCodecError::Length);
    }
    if bytes.get(..4) != Some(magic) {
        return Err(EarlyRenderCodecError::Magic);
    }
    let mut at = 4;
    let version = u16_at(bytes, &mut at)?;
    if !matches!(version, VERSION_V1 | VERSION_V2) {
        return Err(EarlyRenderCodecError::Version);
    }
    Ok((at, version))
}

fn staged_selection_for_fields(
    group_mask: u64,
    count: u32,
) -> Result<EarlyStagedHookSelection, EarlyRenderCodecError> {
    let selection = EarlyStagedHookSelection::from_wire_mask(group_mask)
        .ok_or(EarlyRenderCodecError::InvalidField)?;
    if selection.staged_hook_count() != count {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    Ok(selection)
}

fn staged_wire_version(selection: EarlyStagedHookSelection) -> u16 {
    if selection == EarlyStagedHookSelection::Empty {
        VERSION_V2
    } else {
        VERSION_V1
    }
}

fn validate_staged_wire_version(
    version: u16,
    selection: EarlyStagedHookSelection,
) -> Result<(), EarlyRenderCodecError> {
    if version != staged_wire_version(selection) {
        return Err(EarlyRenderCodecError::Version);
    }
    Ok(())
}
fn nonzero_hash(value: &[u8; 32]) -> bool {
    value.iter().any(|byte| *byte != 0)
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
    let selection =
        staged_selection_for_fields(value.staged_hook_group_mask, value.staged_hook_count)?;
    if value.process_id == 0
        || value.process_start_nonce == 0
        || value.hook_epoch_nonce == 0
        || value.game_device_ordinal != 1
        || value.first_creation_sequence == 0
        || value.creation_qpc == 0
        || value.staged_hook_generation != EARLY_STAGED_HOOK_GENERATION
        || (!value.diagnostic_stage_probe
            && (selection != EarlyStagedHookSelection::Production
                || value.staged_hook_count != EARLY_STAGED_HOOK_COUNT))
        || (value.diagnostic_stage_probe && selection == EarlyStagedHookSelection::Production)
        || !nonzero_hash(&value.staged_hook_set_sha256)
        || value.game_device_ptr == 0
        || value.immediate_context_ptr == 0
        || !value.collector_null_probe_excluded
        || !nonzero_hash(&value.launch_manifest_sha256)
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(152);
    header_version(&mut out, FIRST_MAGIC, staged_wire_version(selection));
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
    let (mut at, version) = validate_staged_header(bytes, FIRST_MAGIC, 152)?;
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
    let selection =
        staged_selection_for_fields(value.staged_hook_group_mask, value.staged_hook_count)?;
    validate_staged_wire_version(version, selection)?;
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
    let selection =
        staged_selection_for_fields(value.staged_hook_group_mask, value.staged_hook_count)?;
    if value.session_id == [0; 16]
        || value.process_id == 0
        || value.process_start_nonce == 0
        || value.hook_epoch_nonce == 0
        || value.game_device_ordinal != 1
        || value.first_creation_sequence == 0
        || value.creation_qpc == 0
        || value.staged_hook_generation != EARLY_STAGED_HOOK_GENERATION
        || selection == EarlyStagedHookSelection::Production
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
    header_version(
        &mut out,
        DIAGNOSTIC_LIVENESS_REQUEST_MAGIC,
        staged_wire_version(selection),
    );
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
    let (mut at, version) = validate_staged_header(bytes, DIAGNOSTIC_LIVENESS_REQUEST_MAGIC, 190)?;
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
    let selection =
        staged_selection_for_fields(value.staged_hook_group_mask, value.staged_hook_count)?;
    validate_staged_wire_version(version, selection)?;
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
    let selection =
        staged_selection_for_fields(value.staged_hook_group_mask, value.staged_hook_count)?;
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
        || selection == EarlyStagedHookSelection::Production
        || !nonzero_hash(&value.staged_hook_set_sha256)
        || value.game_device_ptr == 0
        || value.immediate_context_ptr == 0
    {
        return Err(EarlyRenderCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(182);
    header_version(
        &mut out,
        DIAGNOSTIC_LIVENESS_RESPONSE_MAGIC,
        staged_wire_version(selection),
    );
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
    let (mut at, version) = validate_staged_header(bytes, DIAGNOSTIC_LIVENESS_RESPONSE_MAGIC, 214)?;
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
    let selection =
        staged_selection_for_fields(value.staged_hook_group_mask, value.staged_hook_count)?;
    validate_staged_wire_version(version, selection)?;
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
    let mut out = Vec::with_capacity(234);
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
    Ok(out)
}

pub fn decode_early_render_launch_bind_v2(
    bytes: &[u8],
) -> Result<EarlyRenderLaunchBindV2, EarlyRenderCodecError> {
    let mut at = validate_header(bytes, LAUNCH_BIND_V2_MAGIC, 234)?;
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
    };
    encode_early_render_launch_bind_v2(&value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    /// The out-of-process production sink verifier and this wire constant must
    /// never disagree about the staged-hook inventory, and there are exactly two
    /// non-disagreeing arrangements:
    ///
    ///  * the verifier grades an early-render capture receipt, in which case it
    ///    has to pin THIS value, or
    ///  * the verifier refuses the early-render surface outright, in which case
    ///    no production run it grades can carry a staged-hook count at all and
    ///    the constant is enforced only by the Rust receipt validators.
    ///
    /// The passive product took the second arrangement: `--early-launch-exe` is
    /// a forbidden recorder argument and `early_render_capture_receipt.json` is
    /// a forbidden run-directory artifact. Asserting only the first arrangement
    /// -- which is what this test did while the verifier still graded the
    /// receipt -- would now demand grading logic for a file the same script
    /// requires to be absent.
    ///
    /// What makes this fail: bumping `EARLY_STAGED_HOOK_COUNT` while the
    /// verifier still pins the old number, or letting the verifier readmit the
    /// early-render surface without pinning the count.
    #[test]
    fn production_sink_verifier_count_matches_wire_contract() {
        let verifier = include_str!("../../scripts/verify_sink_reconciliation.py");
        let grades_receipt =
            verifier.contains("receipt.get(\"staged_hook_count\") == EARLY_STAGED_HOOK_COUNT");
        if grades_receipt {
            assert!(
                verifier.contains(&format!(
                    "EARLY_STAGED_HOOK_COUNT = {EARLY_STAGED_HOOK_COUNT}"
                )),
                "the verifier grades staged_hook_count but pins a value other than \
                 EARLY_STAGED_HOOK_COUNT = {EARLY_STAGED_HOOK_COUNT}"
            );
        } else {
            // These two have to be read POSITIONALLY, inside the block that
            // makes them forbidden. A bare `contains` passes just as happily
            // when the same literal is moved into an allowed/tolerated set,
            // which is the readmission this branch exists to refuse.
            let forbidden_arguments = verifier
                .split_once("FORBIDDEN_RECORDER_ARGUMENTS = {")
                .and_then(|(_, rest)| rest.split_once('}'))
                .map(|(block, _)| block)
                .expect("the verifier's forbidden recorder-argument set");
            assert!(
                forbidden_arguments.contains("\"--early-launch-exe\","),
                "a verifier that does not grade staged_hook_count must list \
                 --early-launch-exe in FORBIDDEN_RECORDER_ARGUMENTS, or a production \
                 run can stage hooks nothing counts"
            );
            // Membership in the set is worth nothing if the check that reads
            // the set exempts the member. Pin the enforcement itself, so
            // neutering it (`forbidden not in arguments or forbidden == ...`)
            // is a failure rather than a silent readmission.
            //
            // The limit of this test, stated so nobody reads more into it:
            // it is a LEXICAL guard from Rust over a Python file. It pins the
            // set, the artifact loop, and the enforcement expression, but it
            // cannot prove the verifier enforces anything at RUNTIME. Filtering
            // the flag out of `arguments` before this loop, or returning early
            // above it, readmits the surface with every assertion here still
            // green. Runtime enforcement is the Python verifier's own to test.
            assert!(
                verifier.contains(
                    "require(forbidden not in arguments, \
                     f\"forbidden_recorder_argument:{forbidden}\", problems)"
                ),
                "FORBIDDEN_RECORDER_ARGUMENTS must be enforced unconditionally for every \
                 member; listing --early-launch-exe means nothing if the check exempts it"
            );
            let forbidden_artifacts = verifier
                .split_once("forbidden_early_launch_artifact")
                .map(|(before, _)| before)
                .and_then(|before| before.rsplit_once("for name in ("))
                .map(|(_, block)| block)
                .expect("the verifier's forbidden early-launch artifact loop");
            assert!(
                forbidden_artifacts.contains("\"early_render_capture_receipt.json\","),
                "a verifier that does not grade staged_hook_count must forbid the \
                 early_render_capture_receipt.json artifact in the forbidden-artifact loop"
            );
            assert!(
                verifier.contains("forbidden_early_capture_receipt_present"),
                "the forbidden early-render capture receipt needs a named problem code"
            );
        }
    }

    #[test]
    fn diagnostic_com_selector_is_canonical_and_counts_exact_ordinals() {
        let one_com = 0x01 | (1u64 << EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT);
        let two_com = 0x01 | (0b101u64 << EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT);
        let two_com_plus_vfs = 0x03 | (0b101u64 << EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT);
        assert_eq!(staged_hook_selection_masks(0x01), Some((0x01, 0)));
        assert_eq!(staged_hook_selection_masks(one_com), Some((0x01, 0b001)));
        assert_eq!(staged_hook_count_for_group_mask(one_com), Some(1));
        assert_eq!(staged_hook_count_for_group_mask(two_com), Some(2));
        assert_eq!(staged_hook_count_for_group_mask(two_com_plus_vfs), Some(4));
        assert_eq!(
            staged_hook_count_for_group_mask(EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK),
            Some(EARLY_STAGED_HOOK_COUNT)
        );
        assert_eq!(
            staged_hook_count_for_group_mask(
                0x01 | (EARLY_DIAGNOSTIC_COM_SELECTOR_VALUE_MASK
                    << EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT)
            ),
            None,
            "the explicit all-COM selector must not alias the canonical zero selector"
        );
        assert_eq!(
            staged_hook_count_for_group_mask(1u64 << EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT),
            None,
            "a COM selector without the COM group bit is invalid"
        );
        assert_eq!(
            staged_hook_count_for_group_mask(
                1u64 << (EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT + EARLY_DIAGNOSTIC_COM_HOOK_COUNT)
            ),
            None,
            "selector bits outside the nineteen-hook inventory are invalid"
        );
        assert_eq!(staged_hook_selection_masks(0), Some((0, 0)));
        assert_eq!(staged_hook_count_for_group_mask(0), Some(0));
        for wire_mask in [
            0,
            EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK,
            0x01,
            0x1ff,
            one_com,
            two_com_plus_vfs,
        ] {
            let selection = EarlyStagedHookSelection::from_wire_mask(wire_mask).unwrap();
            assert_eq!(selection.to_wire_mask(), Some(wire_mask));
        }
        assert_eq!(
            EarlyStagedHookSelection::from_wire_mask(EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK),
            Some(EarlyStagedHookSelection::Production)
        );
        assert_eq!(
            EarlyStagedHookSelection::Diagnostic {
                group_mask: EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK,
                com_selector: 0,
            }
            .to_wire_mask(),
            None,
            "production must not acquire a diagnostic alias"
        );
        assert_eq!(
            EarlyStagedHookSelection::Diagnostic {
                group_mask: 0x01,
                com_selector: EARLY_DIAGNOSTIC_COM_SELECTOR_VALUE_MASK,
            }
            .to_wire_mask(),
            None,
            "the explicit all-COM selector remains noncanonical"
        );
    }

    fn head_v1_first_bytes(value: &EarlyFirstGameD3dV1) -> Vec<u8> {
        let mut out = Vec::with_capacity(152);
        out.extend_from_slice(FIRST_MAGIC);
        out.extend_from_slice(&VERSION_V1.to_le_bytes());
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
        out
    }

    fn head_v1_diagnostic_request_bytes(value: &EarlyDiagnosticLivenessRequestV1) -> Vec<u8> {
        let mut out = Vec::with_capacity(190);
        out.extend_from_slice(DIAGNOSTIC_LIVENESS_REQUEST_MAGIC);
        out.extend_from_slice(&VERSION_V1.to_le_bytes());
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
        out
    }

    fn head_v1_diagnostic_response_bytes(value: &EarlyDiagnosticLivenessResponseV1) -> Vec<u8> {
        let mut out = Vec::with_capacity(214);
        out.extend_from_slice(DIAGNOSTIC_LIVENESS_RESPONSE_MAGIC);
        out.extend_from_slice(&VERSION_V1.to_le_bytes());
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
        out.extend_from_slice(&value.response_sha256);
        out
    }

    fn representative_first() -> EarlyFirstGameD3dV1 {
        EarlyFirstGameD3dV1 {
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
        }
    }

    #[test]
    fn nonempty_first_encodings_are_byte_identical_to_head_v1() {
        let mut cases = vec![representative_first()];
        for (mask, count) in [
            (0x01, 19),
            (0x1ff, 68),
            (0x01 | (0b101u64 << EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT), 2),
        ] {
            let mut diagnostic = representative_first();
            diagnostic.diagnostic_stage_probe = true;
            diagnostic.staged_hook_group_mask = mask;
            diagnostic.staged_hook_count = count;
            cases.push(diagnostic);
        }
        for value in cases {
            assert_eq!(
                encode_early_first_game_d3d(&value).unwrap(),
                head_v1_first_bytes(&value)
            );
        }
    }

    #[test]
    fn empty_first_is_v2_only_and_round_trips_with_zero_staged_hooks() {
        let mut empty = representative_first();
        empty.diagnostic_stage_probe = true;
        empty.staged_hook_group_mask = 0;
        empty.staged_hook_count = 0;
        let bytes = encode_early_first_game_d3d(&empty).unwrap();
        assert_eq!(&bytes[..4], FIRST_MAGIC);
        assert_eq!(
            u16::from_le_bytes(bytes[4..6].try_into().unwrap()),
            VERSION_V2
        );
        assert_eq!(decode_early_first_game_d3d(&bytes).unwrap(), empty);
        assert_eq!(staged_hook_count_for_group_mask(0), Some(0));
        assert_eq!(
            validate_header(&bytes, FIRST_MAGIC, 152),
            Err(EarlyRenderCodecError::Version),
            "the exact v1 decoder header gate must refuse a v2 empty frame"
        );

        let mut v1_empty = bytes.clone();
        v1_empty[4..6].copy_from_slice(&VERSION_V1.to_le_bytes());
        assert_eq!(
            decode_early_first_game_d3d(&v1_empty),
            Err(EarlyRenderCodecError::Version),
            "empty has no legacy v1 representation"
        );

        let mut v2_production = head_v1_first_bytes(&representative_first());
        v2_production[4..6].copy_from_slice(&VERSION_V2.to_le_bytes());
        assert_eq!(
            decode_early_first_game_d3d(&v2_production),
            Err(EarlyRenderCodecError::Version),
            "production has no v2 alias"
        );
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
        let first = representative_first();
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
        diagnostic_first.staged_hook_group_mask =
            0x01 | (0b101u64 << EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT);
        diagnostic_first.staged_hook_count = 2;
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
        diagnostic_first.staged_hook_count = EARLY_DIAGNOSTIC_COM_HOOK_COUNT;
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
        let encoded_request = encode_early_diagnostic_liveness_request(&request).unwrap();
        assert_eq!(
            encoded_request,
            head_v1_diagnostic_request_bytes(&request),
            "nonempty diagnostic request encoding must remain byte-identical to v1"
        );
        assert_eq!(
            decode_early_diagnostic_liveness_request(&encoded_request).unwrap(),
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
        assert_eq!(
            encoded_response,
            head_v1_diagnostic_response_bytes(&response),
            "nonempty diagnostic response encoding must remain byte-identical to v1"
        );
        assert_eq!(&encoded_response[..182], response_binding.as_slice());
        assert_eq!(&encoded_response[182..], &response.response_sha256);
        assert_eq!(
            decode_early_diagnostic_liveness_response(&encoded_response).unwrap(),
            response
        );
        let mut empty_request = request.clone();
        empty_request.staged_hook_group_mask = 0;
        empty_request.staged_hook_count = 0;
        let encoded_empty_request =
            encode_early_diagnostic_liveness_request(&empty_request).unwrap();
        assert_eq!(
            u16::from_le_bytes(encoded_empty_request[4..6].try_into().unwrap()),
            VERSION_V2
        );
        assert_eq!(
            decode_early_diagnostic_liveness_request(&encoded_empty_request).unwrap(),
            empty_request
        );
        let mut empty_response = response.clone();
        empty_response.staged_hook_group_mask = 0;
        empty_response.staged_hook_count = 0;
        empty_response.response_sha256 = [0; 32];
        let empty_response_binding =
            canonical_early_diagnostic_liveness_response_binding_bytes(&empty_response).unwrap();
        empty_response.response_sha256 = Sha256::digest(&empty_response_binding).into();
        let encoded_empty_response =
            encode_early_diagnostic_liveness_response(&empty_response).unwrap();
        assert_eq!(
            u16::from_le_bytes(encoded_empty_response[4..6].try_into().unwrap()),
            VERSION_V2
        );
        assert_eq!(
            decode_early_diagnostic_liveness_response(&encoded_empty_response).unwrap(),
            empty_response
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
        let bind_v2 = EarlyRenderLaunchBindV2 {
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
        let bind_v2_bytes = encode_early_render_launch_bind_v2(&bind_v2).unwrap();
        assert_eq!(
            decode_early_render_launch_bind_v2(&bind_v2_bytes).unwrap(),
            bind_v2
        );
        let mut trailing_bind = bind_v2_bytes.clone();
        trailing_bind.push(0);
        assert_eq!(
            decode_early_render_launch_bind_v2(&trailing_bind),
            Err(EarlyRenderCodecError::Length)
        );
        let mut foreign_magic = bind_v2_bytes;
        foreign_magic[..4].copy_from_slice(b"ELBD");
        assert_eq!(
            decode_early_render_launch_bind_v2(&foreign_magic),
            Err(EarlyRenderCodecError::Magic)
        );
        let mut invalid = bind_v2.clone();
        invalid.timeout_ms = 0;
        assert_eq!(
            encode_early_render_launch_bind_v2(&invalid),
            Err(EarlyRenderCodecError::InvalidField)
        );
        invalid.timeout_ms = MAX_EARLY_RENDER_TIMEOUT_MS + 1;
        assert_eq!(
            encode_early_render_launch_bind_v2(&invalid),
            Err(EarlyRenderCodecError::InvalidField)
        );
    }
}
