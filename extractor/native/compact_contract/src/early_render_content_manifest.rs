//! Typed H4CT transfer for the canonical portable-content manifest used by
//! strict in-process d77/d80 admission. The manifest is transmitted only over
//! the authenticated early-render session; no install path or sidecar is used.

use crate::control_transport::CONTROL_MAX_PAYLOAD_LEN;

const VERSION: u16 = 1;
const CHUNK_MAGIC: &[u8; 4] = b"EMCH";
const COMMIT_MAGIC: &[u8; 4] = b"EMCM";
const CHUNK_FIXED_LEN: usize = 192;
const COMMIT_LEN: usize = 176;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderContentManifestChunkV1 {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub session_id: [u8; 16],
    pub launch_bind_sha256: [u8; 32],
    pub effective_vfs_manifest_sha256: [u8; 32],
    pub effective_vfs_index_sha256: [u8; 32],
    /// SHA-256 of the exact canonical JSON bytes, including the manifest's
    /// independently validated `manifest_digest_sha256` field.
    pub content_manifest_bytes_sha256: [u8; 32],
    pub chunk_index: u32,
    pub chunk_count: u32,
    pub byte_offset: u64,
    pub total_bytes: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderContentManifestCommitV1 {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub session_id: [u8; 16],
    pub launch_bind_sha256: [u8; 32],
    pub effective_vfs_manifest_sha256: [u8; 32],
    pub effective_vfs_index_sha256: [u8; 32],
    pub content_manifest_bytes_sha256: [u8; 32],
    pub chunk_count: u32,
    pub total_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarlyRenderContentManifestCodecError {
    Length,
    Magic,
    Version,
    Reserved,
    InvalidField,
    PayloadTooLarge,
}

fn nonzero(value: &[u8]) -> bool {
    value.iter().any(|byte| *byte != 0)
}

fn put_header(out: &mut Vec<u8>, magic: &[u8; 4]) {
    out.extend_from_slice(magic);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
}

fn validate_header(
    bytes: &[u8],
    magic: &[u8; 4],
    minimum: usize,
) -> Result<(), EarlyRenderContentManifestCodecError> {
    if bytes.len() < minimum {
        return Err(EarlyRenderContentManifestCodecError::Length);
    }
    if bytes.get(..4) != Some(magic) {
        return Err(EarlyRenderContentManifestCodecError::Magic);
    }
    if u16::from_le_bytes(bytes[4..6].try_into().unwrap()) != VERSION {
        return Err(EarlyRenderContentManifestCodecError::Version);
    }
    if bytes[6..8] != [0, 0] {
        return Err(EarlyRenderContentManifestCodecError::Reserved);
    }
    Ok(())
}

fn take_u32(bytes: &[u8], at: &mut usize) -> Result<u32, EarlyRenderContentManifestCodecError> {
    let value = u32::from_le_bytes(
        bytes
            .get(*at..*at + 4)
            .ok_or(EarlyRenderContentManifestCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at += 4;
    Ok(value)
}

fn take_u64(bytes: &[u8], at: &mut usize) -> Result<u64, EarlyRenderContentManifestCodecError> {
    let value = u64::from_le_bytes(
        bytes
            .get(*at..*at + 8)
            .ok_or(EarlyRenderContentManifestCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at += 8;
    Ok(value)
}

fn take_array<const N: usize>(
    bytes: &[u8],
    at: &mut usize,
) -> Result<[u8; N], EarlyRenderContentManifestCodecError> {
    let value = bytes
        .get(*at..*at + N)
        .ok_or(EarlyRenderContentManifestCodecError::Length)?
        .try_into()
        .unwrap();
    *at += N;
    Ok(value)
}

fn valid_binding(
    process_id: u32,
    process_start_nonce: u64,
    session_id: &[u8; 16],
    launch_bind_sha256: &[u8; 32],
    effective_vfs_manifest_sha256: &[u8; 32],
    effective_vfs_index_sha256: &[u8; 32],
    content_manifest_bytes_sha256: &[u8; 32],
) -> bool {
    process_id != 0
        && process_start_nonce != 0
        && nonzero(session_id)
        && nonzero(launch_bind_sha256)
        && nonzero(effective_vfs_manifest_sha256)
        && nonzero(effective_vfs_index_sha256)
        && nonzero(content_manifest_bytes_sha256)
}

pub fn encode_early_render_content_manifest_chunk(
    value: &EarlyRenderContentManifestChunkV1,
) -> Result<Vec<u8>, EarlyRenderContentManifestCodecError> {
    let byte_count = u32::try_from(value.bytes.len())
        .map_err(|_| EarlyRenderContentManifestCodecError::PayloadTooLarge)?;
    if !valid_binding(
        value.process_id,
        value.process_start_nonce,
        &value.session_id,
        &value.launch_bind_sha256,
        &value.effective_vfs_manifest_sha256,
        &value.effective_vfs_index_sha256,
        &value.content_manifest_bytes_sha256,
    ) || value.chunk_count == 0
        || value.chunk_index >= value.chunk_count
        || value.total_bytes == 0
        || value.bytes.is_empty()
        || value.byte_offset >= value.total_bytes
        || value
            .byte_offset
            .checked_add(value.bytes.len() as u64)
            .is_none_or(|end| end > value.total_bytes)
    {
        return Err(EarlyRenderContentManifestCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(CHUNK_FIXED_LEN + value.bytes.len());
    put_header(&mut out, CHUNK_MAGIC);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.session_id);
    out.extend_from_slice(&value.launch_bind_sha256);
    out.extend_from_slice(&value.effective_vfs_manifest_sha256);
    out.extend_from_slice(&value.effective_vfs_index_sha256);
    out.extend_from_slice(&value.content_manifest_bytes_sha256);
    out.extend_from_slice(&value.chunk_index.to_le_bytes());
    out.extend_from_slice(&value.chunk_count.to_le_bytes());
    out.extend_from_slice(&value.byte_offset.to_le_bytes());
    out.extend_from_slice(&value.total_bytes.to_le_bytes());
    out.extend_from_slice(&byte_count.to_le_bytes());
    out.extend_from_slice(&value.bytes);
    if out.len() > CONTROL_MAX_PAYLOAD_LEN {
        return Err(EarlyRenderContentManifestCodecError::PayloadTooLarge);
    }
    Ok(out)
}

pub fn decode_early_render_content_manifest_chunk(
    bytes: &[u8],
) -> Result<EarlyRenderContentManifestChunkV1, EarlyRenderContentManifestCodecError> {
    validate_header(bytes, CHUNK_MAGIC, CHUNK_FIXED_LEN)?;
    if bytes.len() > CONTROL_MAX_PAYLOAD_LEN {
        return Err(EarlyRenderContentManifestCodecError::PayloadTooLarge);
    }
    let mut at = 8;
    let process_id = take_u32(bytes, &mut at)?;
    let process_start_nonce = take_u64(bytes, &mut at)?;
    let session_id = take_array(bytes, &mut at)?;
    let launch_bind_sha256 = take_array(bytes, &mut at)?;
    let effective_vfs_manifest_sha256 = take_array(bytes, &mut at)?;
    let effective_vfs_index_sha256 = take_array(bytes, &mut at)?;
    let content_manifest_bytes_sha256 = take_array(bytes, &mut at)?;
    let chunk_index = take_u32(bytes, &mut at)?;
    let chunk_count = take_u32(bytes, &mut at)?;
    let byte_offset = take_u64(bytes, &mut at)?;
    let total_bytes = take_u64(bytes, &mut at)?;
    let byte_count = take_u32(bytes, &mut at)? as usize;
    let end = at
        .checked_add(byte_count)
        .ok_or(EarlyRenderContentManifestCodecError::Length)?;
    if end != bytes.len() {
        return Err(EarlyRenderContentManifestCodecError::Length);
    }
    let value = EarlyRenderContentManifestChunkV1 {
        process_id,
        process_start_nonce,
        session_id,
        launch_bind_sha256,
        effective_vfs_manifest_sha256,
        effective_vfs_index_sha256,
        content_manifest_bytes_sha256,
        chunk_index,
        chunk_count,
        byte_offset,
        total_bytes,
        bytes: bytes[at..end].to_vec(),
    };
    encode_early_render_content_manifest_chunk(&value)?;
    Ok(value)
}

pub fn encode_early_render_content_manifest_commit(
    value: &EarlyRenderContentManifestCommitV1,
) -> Result<Vec<u8>, EarlyRenderContentManifestCodecError> {
    if !valid_binding(
        value.process_id,
        value.process_start_nonce,
        &value.session_id,
        &value.launch_bind_sha256,
        &value.effective_vfs_manifest_sha256,
        &value.effective_vfs_index_sha256,
        &value.content_manifest_bytes_sha256,
    ) || value.chunk_count == 0
        || value.total_bytes == 0
    {
        return Err(EarlyRenderContentManifestCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(COMMIT_LEN);
    put_header(&mut out, COMMIT_MAGIC);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.session_id);
    out.extend_from_slice(&value.launch_bind_sha256);
    out.extend_from_slice(&value.effective_vfs_manifest_sha256);
    out.extend_from_slice(&value.effective_vfs_index_sha256);
    out.extend_from_slice(&value.content_manifest_bytes_sha256);
    out.extend_from_slice(&value.chunk_count.to_le_bytes());
    out.extend_from_slice(&value.total_bytes.to_le_bytes());
    debug_assert_eq!(out.len(), COMMIT_LEN);
    Ok(out)
}

pub fn decode_early_render_content_manifest_commit(
    bytes: &[u8],
) -> Result<EarlyRenderContentManifestCommitV1, EarlyRenderContentManifestCodecError> {
    validate_header(bytes, COMMIT_MAGIC, COMMIT_LEN)?;
    if bytes.len() != COMMIT_LEN {
        return Err(EarlyRenderContentManifestCodecError::Length);
    }
    let mut at = 8;
    let value = EarlyRenderContentManifestCommitV1 {
        process_id: take_u32(bytes, &mut at)?,
        process_start_nonce: take_u64(bytes, &mut at)?,
        session_id: take_array(bytes, &mut at)?,
        launch_bind_sha256: take_array(bytes, &mut at)?,
        effective_vfs_manifest_sha256: take_array(bytes, &mut at)?,
        effective_vfs_index_sha256: take_array(bytes, &mut at)?,
        content_manifest_bytes_sha256: take_array(bytes, &mut at)?,
        chunk_count: take_u32(bytes, &mut at)?,
        total_bytes: take_u64(bytes, &mut at)?,
    };
    encode_early_render_content_manifest_commit(&value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_chunk_and_commit_round_trip_and_reject_gaps() {
        let chunk = EarlyRenderContentManifestChunkV1 {
            process_id: 1,
            process_start_nonce: 2,
            session_id: [3; 16],
            launch_bind_sha256: [4; 32],
            effective_vfs_manifest_sha256: [5; 32],
            effective_vfs_index_sha256: [6; 32],
            content_manifest_bytes_sha256: [7; 32],
            chunk_index: 0,
            chunk_count: 1,
            byte_offset: 0,
            total_bytes: 2,
            bytes: b"{}".to_vec(),
        };
        assert_eq!(
            decode_early_render_content_manifest_chunk(
                &encode_early_render_content_manifest_chunk(&chunk).unwrap()
            )
            .unwrap(),
            chunk
        );
        let commit = EarlyRenderContentManifestCommitV1 {
            process_id: 1,
            process_start_nonce: 2,
            session_id: [3; 16],
            launch_bind_sha256: [4; 32],
            effective_vfs_manifest_sha256: [5; 32],
            effective_vfs_index_sha256: [6; 32],
            content_manifest_bytes_sha256: [7; 32],
            chunk_count: 1,
            total_bytes: 2,
        };
        assert_eq!(
            decode_early_render_content_manifest_commit(
                &encode_early_render_content_manifest_commit(&commit).unwrap()
            )
            .unwrap(),
            commit
        );
        let mut bad = chunk;
        bad.byte_offset = 1;
        assert_eq!(
            encode_early_render_content_manifest_chunk(&bad),
            Err(EarlyRenderContentManifestCodecError::InvalidField)
        );
    }
}
