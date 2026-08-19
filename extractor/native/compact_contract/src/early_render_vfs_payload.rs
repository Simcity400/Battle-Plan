use crate::control_transport::CONTROL_MAX_PAYLOAD_LEN;
use crate::early_render_vfs::{
    decode_early_render_vfs_winner_row, encode_early_render_vfs_winner_row,
    EarlyRenderVfsWinnerRowV1,
};

pub const EARLY_RENDER_MAX_VFS_PAYLOAD_RANGE_LEN: u64 = 1 << 28;
const VERSION: u16 = 1;
const REQUEST_MAGIC: &[u8; 4] = b"EVPR";
const CHUNK_MAGIC: &[u8; 4] = b"EVPC";
const COMMIT_MAGIC: &[u8; 4] = b"EVPM";
const REQUEST_FIXED_LEN: usize = 192;
const CHUNK_FIXED_LEN: usize = 208;
const COMMIT_LEN: usize = 192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderVfsPayloadRequestV1 {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub session_id: [u8; 16],
    pub launch_bind_sha256: [u8; 32],
    pub effective_vfs_manifest_sha256: [u8; 32],
    pub effective_vfs_index_sha256: [u8; 32],
    pub request_id: u64,
    pub payload_offset: u64,
    pub payload_count: u64,
    pub expected_payload_range_sha256: [u8; 32],
    pub winner: EarlyRenderVfsWinnerRowV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderVfsPayloadChunkV1 {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub session_id: [u8; 16],
    pub launch_bind_sha256: [u8; 32],
    pub effective_vfs_manifest_sha256: [u8; 32],
    pub effective_vfs_index_sha256: [u8; 32],
    pub request_id: u64,
    pub payload_offset: u64,
    pub payload_count: u64,
    pub expected_payload_range_sha256: [u8; 32],
    pub chunk_index: u32,
    pub chunk_count: u32,
    pub chunk_offset: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderVfsPayloadCommitV1 {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub session_id: [u8; 16],
    pub launch_bind_sha256: [u8; 32],
    pub effective_vfs_manifest_sha256: [u8; 32],
    pub effective_vfs_index_sha256: [u8; 32],
    pub request_id: u64,
    pub payload_offset: u64,
    pub payload_count: u64,
    pub payload_range_sha256: [u8; 32],
    pub chunk_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarlyRenderVfsPayloadCodecError {
    Length,
    Magic,
    Version,
    Reserved,
    InvalidField,
    PayloadTooLarge,
    Winner,
}

fn nonzero(bytes: &[u8]) -> bool {
    bytes.iter().any(|byte| *byte != 0)
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
) -> Result<(), EarlyRenderVfsPayloadCodecError> {
    if bytes.len() < minimum {
        return Err(EarlyRenderVfsPayloadCodecError::Length);
    }
    if bytes.get(..4) != Some(magic) {
        return Err(EarlyRenderVfsPayloadCodecError::Magic);
    }
    if u16::from_le_bytes(bytes[4..6].try_into().unwrap()) != VERSION {
        return Err(EarlyRenderVfsPayloadCodecError::Version);
    }
    if bytes[6..8] != [0, 0] {
        return Err(EarlyRenderVfsPayloadCodecError::Reserved);
    }
    Ok(())
}

fn take_u32(bytes: &[u8], at: &mut usize) -> Result<u32, EarlyRenderVfsPayloadCodecError> {
    let value = u32::from_le_bytes(
        bytes
            .get(*at..*at + 4)
            .ok_or(EarlyRenderVfsPayloadCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at += 4;
    Ok(value)
}

fn take_u64(bytes: &[u8], at: &mut usize) -> Result<u64, EarlyRenderVfsPayloadCodecError> {
    let value = u64::from_le_bytes(
        bytes
            .get(*at..*at + 8)
            .ok_or(EarlyRenderVfsPayloadCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at += 8;
    Ok(value)
}

fn take_array<const N: usize>(
    bytes: &[u8],
    at: &mut usize,
) -> Result<[u8; N], EarlyRenderVfsPayloadCodecError> {
    let value = bytes
        .get(*at..*at + N)
        .ok_or(EarlyRenderVfsPayloadCodecError::Length)?
        .try_into()
        .unwrap();
    *at += N;
    Ok(value)
}

fn valid_request(value: &EarlyRenderVfsPayloadRequestV1) -> bool {
    value.process_id != 0
        && value.process_start_nonce != 0
        && value.request_id != 0
        && nonzero(&value.session_id)
        && nonzero(&value.launch_bind_sha256)
        && nonzero(&value.effective_vfs_manifest_sha256)
        && nonzero(&value.effective_vfs_index_sha256)
        && nonzero(&value.expected_payload_range_sha256)
        && value.payload_count != 0
        && value.payload_count <= EARLY_RENDER_MAX_VFS_PAYLOAD_RANGE_LEN
        && value
            .payload_offset
            .checked_add(value.payload_count)
            .is_some_and(|end| end <= value.winner.payload_bytes)
        && !((value.payload_offset == 0 && value.payload_count == value.winner.payload_bytes)
            && value.expected_payload_range_sha256 != value.winner.payload_sha256)
}

pub fn encode_early_render_vfs_payload_request(
    value: &EarlyRenderVfsPayloadRequestV1,
) -> Result<Vec<u8>, EarlyRenderVfsPayloadCodecError> {
    if !valid_request(value) {
        return Err(EarlyRenderVfsPayloadCodecError::InvalidField);
    }
    let winner = encode_early_render_vfs_winner_row(&value.winner)
        .map_err(|_| EarlyRenderVfsPayloadCodecError::Winner)?;
    let mut out = Vec::with_capacity(REQUEST_FIXED_LEN + winner.len());
    put_header(&mut out, REQUEST_MAGIC);
    put_binding(
        &mut out,
        value.process_id,
        value.process_start_nonce,
        &value.session_id,
        &value.launch_bind_sha256,
        &value.effective_vfs_manifest_sha256,
        &value.effective_vfs_index_sha256,
        value.request_id,
        value.payload_offset,
        value.payload_count,
        &value.expected_payload_range_sha256,
    );
    out.extend_from_slice(&(winner.len() as u32).to_le_bytes());
    out.extend_from_slice(&winner);
    if out.len() > CONTROL_MAX_PAYLOAD_LEN {
        return Err(EarlyRenderVfsPayloadCodecError::PayloadTooLarge);
    }
    Ok(out)
}

pub fn decode_early_render_vfs_payload_request(
    bytes: &[u8],
) -> Result<EarlyRenderVfsPayloadRequestV1, EarlyRenderVfsPayloadCodecError> {
    validate_header(bytes, REQUEST_MAGIC, REQUEST_FIXED_LEN)?;
    let mut at = 8;
    let (
        process_id,
        process_start_nonce,
        session_id,
        launch_bind_sha256,
        effective_vfs_manifest_sha256,
        effective_vfs_index_sha256,
        request_id,
        payload_offset,
        payload_count,
        expected_payload_range_sha256,
    ) = take_binding(bytes, &mut at)?;
    let winner_len = take_u32(bytes, &mut at)? as usize;
    let end = at
        .checked_add(winner_len)
        .ok_or(EarlyRenderVfsPayloadCodecError::Length)?;
    let winner = decode_early_render_vfs_winner_row(
        bytes
            .get(at..end)
            .ok_or(EarlyRenderVfsPayloadCodecError::Length)?,
    )
    .map_err(|_| EarlyRenderVfsPayloadCodecError::Winner)?;
    if end != bytes.len() {
        return Err(EarlyRenderVfsPayloadCodecError::Length);
    }
    let value = EarlyRenderVfsPayloadRequestV1 {
        process_id,
        process_start_nonce,
        session_id,
        launch_bind_sha256,
        effective_vfs_manifest_sha256,
        effective_vfs_index_sha256,
        request_id,
        payload_offset,
        payload_count,
        expected_payload_range_sha256,
        winner,
    };
    if encode_early_render_vfs_payload_request(&value)? != bytes {
        return Err(EarlyRenderVfsPayloadCodecError::InvalidField);
    }
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn put_binding(
    out: &mut Vec<u8>,
    process_id: u32,
    nonce: u64,
    session: &[u8; 16],
    bind: &[u8; 32],
    manifest: &[u8; 32],
    index: &[u8; 32],
    request_id: u64,
    offset: u64,
    count: u64,
    range_sha: &[u8; 32],
) {
    out.extend_from_slice(&process_id.to_le_bytes());
    out.extend_from_slice(&nonce.to_le_bytes());
    out.extend_from_slice(session);
    out.extend_from_slice(bind);
    out.extend_from_slice(manifest);
    out.extend_from_slice(index);
    out.extend_from_slice(&request_id.to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(range_sha);
}

type BindingTuple = (
    u32,
    u64,
    [u8; 16],
    [u8; 32],
    [u8; 32],
    [u8; 32],
    u64,
    u64,
    u64,
    [u8; 32],
);

fn take_binding(
    bytes: &[u8],
    at: &mut usize,
) -> Result<BindingTuple, EarlyRenderVfsPayloadCodecError> {
    Ok((
        take_u32(bytes, at)?,
        take_u64(bytes, at)?,
        take_array(bytes, at)?,
        take_array(bytes, at)?,
        take_array(bytes, at)?,
        take_array(bytes, at)?,
        take_u64(bytes, at)?,
        take_u64(bytes, at)?,
        take_u64(bytes, at)?,
        take_array(bytes, at)?,
    ))
}

pub fn encode_early_render_vfs_payload_chunk(
    value: &EarlyRenderVfsPayloadChunkV1,
) -> Result<Vec<u8>, EarlyRenderVfsPayloadCodecError> {
    let byte_count = u32::try_from(value.bytes.len())
        .map_err(|_| EarlyRenderVfsPayloadCodecError::PayloadTooLarge)?;
    if value.process_id == 0
        || value.process_start_nonce == 0
        || value.request_id == 0
        || !nonzero(&value.session_id)
        || !nonzero(&value.launch_bind_sha256)
        || !nonzero(&value.effective_vfs_manifest_sha256)
        || !nonzero(&value.effective_vfs_index_sha256)
        || !nonzero(&value.expected_payload_range_sha256)
        || value.payload_count == 0
        || value.payload_count > EARLY_RENDER_MAX_VFS_PAYLOAD_RANGE_LEN
        || value.chunk_count == 0
        || value.chunk_index >= value.chunk_count
        || byte_count == 0
        || value
            .chunk_offset
            .checked_add(byte_count as u64)
            .map_or(true, |end| end > value.payload_count)
    {
        return Err(EarlyRenderVfsPayloadCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(CHUNK_FIXED_LEN + value.bytes.len());
    put_header(&mut out, CHUNK_MAGIC);
    put_binding(
        &mut out,
        value.process_id,
        value.process_start_nonce,
        &value.session_id,
        &value.launch_bind_sha256,
        &value.effective_vfs_manifest_sha256,
        &value.effective_vfs_index_sha256,
        value.request_id,
        value.payload_offset,
        value.payload_count,
        &value.expected_payload_range_sha256,
    );
    out.extend_from_slice(&value.chunk_index.to_le_bytes());
    out.extend_from_slice(&value.chunk_count.to_le_bytes());
    out.extend_from_slice(&value.chunk_offset.to_le_bytes());
    out.extend_from_slice(&byte_count.to_le_bytes());
    out.extend_from_slice(&value.bytes);
    if out.len() > CONTROL_MAX_PAYLOAD_LEN {
        return Err(EarlyRenderVfsPayloadCodecError::PayloadTooLarge);
    }
    Ok(out)
}

pub fn decode_early_render_vfs_payload_chunk(
    bytes: &[u8],
) -> Result<EarlyRenderVfsPayloadChunkV1, EarlyRenderVfsPayloadCodecError> {
    validate_header(bytes, CHUNK_MAGIC, CHUNK_FIXED_LEN)?;
    let mut at = 8;
    let (
        process_id,
        process_start_nonce,
        session_id,
        launch_bind_sha256,
        effective_vfs_manifest_sha256,
        effective_vfs_index_sha256,
        request_id,
        payload_offset,
        payload_count,
        expected_payload_range_sha256,
    ) = take_binding(bytes, &mut at)?;
    let chunk_index = take_u32(bytes, &mut at)?;
    let chunk_count = take_u32(bytes, &mut at)?;
    let chunk_offset = take_u64(bytes, &mut at)?;
    let byte_count = take_u32(bytes, &mut at)? as usize;
    let end = at
        .checked_add(byte_count)
        .ok_or(EarlyRenderVfsPayloadCodecError::Length)?;
    let payload = bytes
        .get(at..end)
        .ok_or(EarlyRenderVfsPayloadCodecError::Length)?
        .to_vec();
    if end != bytes.len() {
        return Err(EarlyRenderVfsPayloadCodecError::Length);
    }
    let value = EarlyRenderVfsPayloadChunkV1 {
        process_id,
        process_start_nonce,
        session_id,
        launch_bind_sha256,
        effective_vfs_manifest_sha256,
        effective_vfs_index_sha256,
        request_id,
        payload_offset,
        payload_count,
        expected_payload_range_sha256,
        chunk_index,
        chunk_count,
        chunk_offset,
        bytes: payload,
    };
    if encode_early_render_vfs_payload_chunk(&value)? != bytes {
        return Err(EarlyRenderVfsPayloadCodecError::InvalidField);
    }
    Ok(value)
}

pub fn encode_early_render_vfs_payload_commit(
    value: &EarlyRenderVfsPayloadCommitV1,
) -> Result<Vec<u8>, EarlyRenderVfsPayloadCodecError> {
    if value.process_id == 0
        || value.process_start_nonce == 0
        || value.request_id == 0
        || !nonzero(&value.session_id)
        || !nonzero(&value.launch_bind_sha256)
        || !nonzero(&value.effective_vfs_manifest_sha256)
        || !nonzero(&value.effective_vfs_index_sha256)
        || !nonzero(&value.payload_range_sha256)
        || value.payload_count == 0
        || value.payload_count > EARLY_RENDER_MAX_VFS_PAYLOAD_RANGE_LEN
        || value.chunk_count == 0
    {
        return Err(EarlyRenderVfsPayloadCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(COMMIT_LEN);
    put_header(&mut out, COMMIT_MAGIC);
    put_binding(
        &mut out,
        value.process_id,
        value.process_start_nonce,
        &value.session_id,
        &value.launch_bind_sha256,
        &value.effective_vfs_manifest_sha256,
        &value.effective_vfs_index_sha256,
        value.request_id,
        value.payload_offset,
        value.payload_count,
        &value.payload_range_sha256,
    );
    out.extend_from_slice(&value.chunk_count.to_le_bytes());
    Ok(out)
}

pub fn decode_early_render_vfs_payload_commit(
    bytes: &[u8],
) -> Result<EarlyRenderVfsPayloadCommitV1, EarlyRenderVfsPayloadCodecError> {
    validate_header(bytes, COMMIT_MAGIC, COMMIT_LEN)?;
    if bytes.len() != COMMIT_LEN {
        return Err(EarlyRenderVfsPayloadCodecError::Length);
    }
    let mut at = 8;
    let (
        process_id,
        process_start_nonce,
        session_id,
        launch_bind_sha256,
        effective_vfs_manifest_sha256,
        effective_vfs_index_sha256,
        request_id,
        payload_offset,
        payload_count,
        payload_range_sha256,
    ) = take_binding(bytes, &mut at)?;
    let value = EarlyRenderVfsPayloadCommitV1 {
        process_id,
        process_start_nonce,
        session_id,
        launch_bind_sha256,
        effective_vfs_manifest_sha256,
        effective_vfs_index_sha256,
        request_id,
        payload_offset,
        payload_count,
        payload_range_sha256,
        chunk_count: take_u32(bytes, &mut at)?,
    };
    if at != bytes.len() || encode_early_render_vfs_payload_commit(&value)? != bytes {
        return Err(EarlyRenderVfsPayloadCodecError::InvalidField);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::early_render_vfs::EarlyRenderVfsLayerKind;

    fn winner() -> EarlyRenderVfsWinnerRowV1 {
        EarlyRenderVfsWinnerRowV1 {
            normalized_game_relative_path: "gfx/test.asset".into(),
            winning_layer_ordinal: 0,
            layer_kind: EarlyRenderVfsLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: [7; 32],
            payload_sha256: [7; 32],
            payload_bytes: 4,
            source_container_sha256: None,
            source_byte_offset: 0,
            source_byte_count: 4,
            source_range_sha256: [7; 32],
        }
    }

    #[test]
    fn exact_request_chunk_commit_roundtrip_and_tamper_rejection() {
        let request = EarlyRenderVfsPayloadRequestV1 {
            process_id: 1,
            process_start_nonce: 2,
            session_id: [3; 16],
            launch_bind_sha256: [4; 32],
            effective_vfs_manifest_sha256: [5; 32],
            effective_vfs_index_sha256: [6; 32],
            request_id: 1,
            payload_offset: 0,
            payload_count: 4,
            expected_payload_range_sha256: [7; 32],
            winner: winner(),
        };
        let encoded = encode_early_render_vfs_payload_request(&request).unwrap();
        assert_eq!(
            decode_early_render_vfs_payload_request(&encoded).unwrap(),
            request
        );
        let chunk = EarlyRenderVfsPayloadChunkV1 {
            process_id: 1,
            process_start_nonce: 2,
            session_id: [3; 16],
            launch_bind_sha256: [4; 32],
            effective_vfs_manifest_sha256: [5; 32],
            effective_vfs_index_sha256: [6; 32],
            request_id: 1,
            payload_offset: 0,
            payload_count: 4,
            expected_payload_range_sha256: [7; 32],
            chunk_index: 0,
            chunk_count: 1,
            chunk_offset: 0,
            bytes: vec![1, 2, 3, 4],
        };
        let encoded = encode_early_render_vfs_payload_chunk(&chunk).unwrap();
        assert_eq!(
            decode_early_render_vfs_payload_chunk(&encoded).unwrap(),
            chunk
        );
        let commit = EarlyRenderVfsPayloadCommitV1 {
            process_id: 1,
            process_start_nonce: 2,
            session_id: [3; 16],
            launch_bind_sha256: [4; 32],
            effective_vfs_manifest_sha256: [5; 32],
            effective_vfs_index_sha256: [6; 32],
            request_id: 1,
            payload_offset: 0,
            payload_count: 4,
            payload_range_sha256: [7; 32],
            chunk_count: 1,
        };
        let encoded = encode_early_render_vfs_payload_commit(&commit).unwrap();
        assert_eq!(
            decode_early_render_vfs_payload_commit(&encoded).unwrap(),
            commit
        );
        let mut bad = request;
        bad.payload_count = EARLY_RENDER_MAX_VFS_PAYLOAD_RANGE_LEN + 1;
        assert_eq!(
            encode_early_render_vfs_payload_request(&bad),
            Err(EarlyRenderVfsPayloadCodecError::InvalidField)
        );
    }
}
