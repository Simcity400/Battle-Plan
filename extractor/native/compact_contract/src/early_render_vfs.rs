use crate::control_transport::CONTROL_MAX_PAYLOAD_LEN;

const VERSION: u16 = 1;
const ROW_MAGIC: &[u8; 4] = b"EVRW";
const CHUNK_MAGIC: &[u8; 4] = b"EVCH";
const COMMIT_MAGIC: &[u8; 4] = b"EVCM";
const ROW_FIXED_LEN: usize = 139;
const CHUNK_FIXED_LEN: usize = 120;
const COMMIT_LEN: usize = 108;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum EarlyRenderVfsLayerKind {
    Base = 1,
    IntegratedDlc = 2,
    Dlc = 3,
    Mod = 4,
}

crate::wire_coded_codes! {
    EarlyRenderVfsLayerKind: u8,
    code = pub code,
    Ok = from_wire(value) -> Result<Self, EarlyRenderVfsCodecError> {
        Err(EarlyRenderVfsCodecError::InvalidField)
    };

    Base = 1,
    IntegratedDlc = 2,
    Dlc = 3,
    Mod = 4,
}

impl TryFrom<u8> for EarlyRenderVfsLayerKind {
    type Error = EarlyRenderVfsCodecError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::from_wire(value)
    }
}

impl EarlyRenderVfsLayerKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::IntegratedDlc => "integrated_dlc",
            Self::Dlc => "dlc",
            Self::Mod => "mod",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderVfsWinnerRowV1 {
    pub normalized_game_relative_path: String,
    pub winning_layer_ordinal: u32,
    pub layer_kind: EarlyRenderVfsLayerKind,
    pub layer_id: String,
    pub source_sha256: [u8; 32],
    pub payload_sha256: [u8; 32],
    pub payload_bytes: u64,
    pub source_container_sha256: Option<[u8; 32]>,
    pub source_byte_offset: u64,
    pub source_byte_count: u64,
    pub source_range_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderVfsIndexChunkV1 {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub session_id: [u8; 16],
    pub launch_bind_sha256: [u8; 32],
    pub index_sha256: [u8; 32],
    pub chunk_index: u32,
    pub chunk_count: u32,
    pub first_row_index: u32,
    pub total_row_count: u32,
    pub rows: Vec<EarlyRenderVfsWinnerRowV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderVfsIndexCommitV1 {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub session_id: [u8; 16],
    pub launch_bind_sha256: [u8; 32],
    pub index_sha256: [u8; 32],
    pub chunk_count: u32,
    pub total_row_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarlyRenderVfsCodecError {
    Length,
    Magic,
    Version,
    Reserved,
    InvalidField,
    Utf8,
    PayloadTooLarge,
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
) -> Result<(), EarlyRenderVfsCodecError> {
    if bytes.len() < minimum {
        return Err(EarlyRenderVfsCodecError::Length);
    }
    if bytes.get(..4) != Some(magic) {
        return Err(EarlyRenderVfsCodecError::Magic);
    }
    if u16::from_le_bytes(bytes[4..6].try_into().unwrap()) != VERSION {
        return Err(EarlyRenderVfsCodecError::Version);
    }
    if bytes[6..8] != [0, 0] {
        return Err(EarlyRenderVfsCodecError::Reserved);
    }
    Ok(())
}

fn put_string(out: &mut Vec<u8>, value: &str) -> Result<(), EarlyRenderVfsCodecError> {
    if value.is_empty() || value.len() > u16::MAX as usize || value.chars().any(char::is_control) {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn put_canonical_string(out: &mut Vec<u8>, value: &str) -> Result<(), EarlyRenderVfsCodecError> {
    if value.is_empty() || value.len() > u32::MAX as usize || value.chars().any(char::is_control) {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn lower_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn take_u16(bytes: &[u8], at: &mut usize) -> Result<u16, EarlyRenderVfsCodecError> {
    let value = u16::from_le_bytes(
        bytes
            .get(*at..*at + 2)
            .ok_or(EarlyRenderVfsCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at += 2;
    Ok(value)
}

fn take_u32(bytes: &[u8], at: &mut usize) -> Result<u32, EarlyRenderVfsCodecError> {
    let value = u32::from_le_bytes(
        bytes
            .get(*at..*at + 4)
            .ok_or(EarlyRenderVfsCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at += 4;
    Ok(value)
}

fn take_u64(bytes: &[u8], at: &mut usize) -> Result<u64, EarlyRenderVfsCodecError> {
    let value = u64::from_le_bytes(
        bytes
            .get(*at..*at + 8)
            .ok_or(EarlyRenderVfsCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at += 8;
    Ok(value)
}

fn take_array<const N: usize>(
    bytes: &[u8],
    at: &mut usize,
) -> Result<[u8; N], EarlyRenderVfsCodecError> {
    let value = bytes
        .get(*at..*at + N)
        .ok_or(EarlyRenderVfsCodecError::Length)?
        .try_into()
        .unwrap();
    *at += N;
    Ok(value)
}

fn take_string(bytes: &[u8], at: &mut usize) -> Result<String, EarlyRenderVfsCodecError> {
    let len = take_u16(bytes, at)? as usize;
    if len == 0 {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    let raw = bytes
        .get(*at..*at + len)
        .ok_or(EarlyRenderVfsCodecError::Length)?;
    *at += len;
    let value = std::str::from_utf8(raw)
        .map_err(|_| EarlyRenderVfsCodecError::Utf8)?
        .to_owned();
    if value.chars().any(char::is_control) {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    Ok(value)
}

pub fn encode_early_render_vfs_winner_row(
    row: &EarlyRenderVfsWinnerRowV1,
) -> Result<Vec<u8>, EarlyRenderVfsCodecError> {
    let source_shape_valid = match row.source_container_sha256 {
        Some(hash) => hash == row.source_sha256 && nonzero(&hash),
        None => {
            row.source_byte_offset == 0
                && row.source_byte_count == row.payload_bytes
                && row.source_range_sha256 == row.payload_sha256
                && row.source_sha256 == row.payload_sha256
        }
    };
    if !nonzero(&row.source_sha256)
        || !nonzero(&row.payload_sha256)
        || !nonzero(&row.source_range_sha256)
        || !source_shape_valid
        || row
            .source_byte_offset
            .checked_add(row.source_byte_count)
            .is_none()
    {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    let path_len = row.normalized_game_relative_path.len();
    let layer_len = row.layer_id.len();
    if path_len == 0
        || layer_len == 0
        || path_len > u16::MAX as usize
        || layer_len > u16::MAX as usize
    {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(ROW_FIXED_LEN + path_len + layer_len);
    put_header(&mut out, ROW_MAGIC);
    out.push(row.layer_kind.code());
    out.push(0);
    out.extend_from_slice(&row.winning_layer_ordinal.to_le_bytes());
    out.extend_from_slice(&row.payload_bytes.to_le_bytes());
    out.extend_from_slice(&row.source_sha256);
    out.extend_from_slice(&row.payload_sha256);
    match row.source_container_sha256 {
        Some(hash) => {
            out.push(1);
            out.extend_from_slice(&hash);
        }
        None => out.push(0),
    }
    out.extend_from_slice(&row.source_byte_offset.to_le_bytes());
    out.extend_from_slice(&row.source_byte_count.to_le_bytes());
    out.extend_from_slice(&row.source_range_sha256);
    put_string(&mut out, &row.normalized_game_relative_path)?;
    put_string(&mut out, &row.layer_id)?;
    Ok(out)
}

pub fn decode_early_render_vfs_winner_row(
    bytes: &[u8],
) -> Result<EarlyRenderVfsWinnerRowV1, EarlyRenderVfsCodecError> {
    validate_header(bytes, ROW_MAGIC, ROW_FIXED_LEN)?;
    let mut at = 8;
    let layer_kind =
        EarlyRenderVfsLayerKind::try_from(*bytes.get(at).ok_or(EarlyRenderVfsCodecError::Length)?)?;
    at += 1;
    if *bytes.get(at).ok_or(EarlyRenderVfsCodecError::Length)? != 0 {
        return Err(EarlyRenderVfsCodecError::Reserved);
    }
    at += 1;
    let winning_layer_ordinal = take_u32(bytes, &mut at)?;
    let payload_bytes = take_u64(bytes, &mut at)?;
    let source_sha256 = take_array(bytes, &mut at)?;
    let payload_sha256 = take_array(bytes, &mut at)?;
    let source_container_sha256 = match *bytes.get(at).ok_or(EarlyRenderVfsCodecError::Length)? {
        0 => None,
        1 => {
            at += 1;
            Some(take_array(bytes, &mut at)?)
        }
        _ => return Err(EarlyRenderVfsCodecError::InvalidField),
    };
    if source_container_sha256.is_none() {
        at += 1;
    }
    let source_byte_offset = take_u64(bytes, &mut at)?;
    let source_byte_count = take_u64(bytes, &mut at)?;
    let source_range_sha256 = take_array(bytes, &mut at)?;
    let row = EarlyRenderVfsWinnerRowV1 {
        normalized_game_relative_path: String::new(),
        winning_layer_ordinal,
        layer_kind,
        layer_id: String::new(),
        source_sha256,
        payload_sha256,
        payload_bytes,
        source_container_sha256,
        source_byte_offset,
        source_byte_count,
        source_range_sha256,
    };
    let row = EarlyRenderVfsWinnerRowV1 {
        normalized_game_relative_path: take_string(bytes, &mut at)?,
        layer_id: take_string(bytes, &mut at)?,
        ..row
    };
    if at != bytes.len() {
        return Err(EarlyRenderVfsCodecError::Length);
    }
    if encode_early_render_vfs_winner_row(&row)? != bytes {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    Ok(row)
}

pub fn canonical_early_render_vfs_index_bytes(
    rows: &[EarlyRenderVfsWinnerRowV1],
) -> Result<Vec<u8>, EarlyRenderVfsCodecError> {
    if rows.is_empty() || rows.len() > u32::MAX as usize {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    let mut out = b"hoi4-effective-vfs-winners-v1\0".to_vec();
    out.extend_from_slice(&(rows.len() as u64).to_le_bytes());
    let mut prior: Option<&str> = None;
    for row in rows {
        if prior.is_some_and(|value| value >= row.normalized_game_relative_path.as_str()) {
            return Err(EarlyRenderVfsCodecError::InvalidField);
        }
        prior = Some(&row.normalized_game_relative_path);
        encode_early_render_vfs_winner_row(row)?;
        put_canonical_string(&mut out, &row.normalized_game_relative_path)?;
        out.extend_from_slice(&row.winning_layer_ordinal.to_le_bytes());
        put_canonical_string(&mut out, row.layer_kind.as_str())?;
        put_canonical_string(&mut out, &row.layer_id)?;
        put_canonical_string(&mut out, &lower_hex(&row.source_sha256))?;
        put_canonical_string(&mut out, &lower_hex(&row.payload_sha256))?;
        out.extend_from_slice(&row.payload_bytes.to_le_bytes());
        match &row.source_container_sha256 {
            Some(hash) => {
                out.push(1);
                put_canonical_string(&mut out, &lower_hex(hash))?;
            }
            None => out.push(0),
        }
        out.extend_from_slice(&row.source_byte_offset.to_le_bytes());
        out.extend_from_slice(&row.source_byte_count.to_le_bytes());
        put_canonical_string(&mut out, &lower_hex(&row.source_range_sha256))?;
    }
    Ok(out)
}

pub fn encode_early_render_vfs_index_chunk(
    value: &EarlyRenderVfsIndexChunkV1,
) -> Result<Vec<u8>, EarlyRenderVfsCodecError> {
    let row_count =
        u32::try_from(value.rows.len()).map_err(|_| EarlyRenderVfsCodecError::InvalidField)?;
    if value.process_id == 0
        || value.process_start_nonce == 0
        || !nonzero(&value.session_id)
        || !nonzero(&value.launch_bind_sha256)
        || !nonzero(&value.index_sha256)
        || value.chunk_count == 0
        || value.total_row_count == 0
        || row_count == 0
        || value.chunk_index >= value.chunk_count
        || value
            .first_row_index
            .checked_add(row_count)
            .map_or(true, |end| end > value.total_row_count)
    {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    canonical_early_render_vfs_index_bytes(&value.rows)?;
    let mut out = Vec::new();
    put_header(&mut out, CHUNK_MAGIC);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.session_id);
    out.extend_from_slice(&value.launch_bind_sha256);
    out.extend_from_slice(&value.index_sha256);
    out.extend_from_slice(&value.chunk_index.to_le_bytes());
    out.extend_from_slice(&value.chunk_count.to_le_bytes());
    out.extend_from_slice(&value.first_row_index.to_le_bytes());
    out.extend_from_slice(&value.total_row_count.to_le_bytes());
    out.extend_from_slice(&row_count.to_le_bytes());
    for row in &value.rows {
        let encoded = encode_early_render_vfs_winner_row(row)?;
        out.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        out.extend_from_slice(&encoded);
    }
    if out.len() > CONTROL_MAX_PAYLOAD_LEN {
        return Err(EarlyRenderVfsCodecError::PayloadTooLarge);
    }
    Ok(out)
}

pub fn decode_early_render_vfs_index_chunk(
    bytes: &[u8],
) -> Result<EarlyRenderVfsIndexChunkV1, EarlyRenderVfsCodecError> {
    validate_header(bytes, CHUNK_MAGIC, CHUNK_FIXED_LEN)?;
    let mut at = 8;
    let process_id = take_u32(bytes, &mut at)?;
    let process_start_nonce = take_u64(bytes, &mut at)?;
    let session_id = take_array(bytes, &mut at)?;
    let launch_bind_sha256 = take_array(bytes, &mut at)?;
    let index_sha256 = take_array(bytes, &mut at)?;
    let chunk_index = take_u32(bytes, &mut at)?;
    let chunk_count = take_u32(bytes, &mut at)?;
    let first_row_index = take_u32(bytes, &mut at)?;
    let total_row_count = take_u32(bytes, &mut at)?;
    let row_count = take_u32(bytes, &mut at)?;
    let mut rows = Vec::with_capacity(row_count as usize);
    for _ in 0..row_count {
        let len = take_u32(bytes, &mut at)? as usize;
        let end = at
            .checked_add(len)
            .ok_or(EarlyRenderVfsCodecError::Length)?;
        rows.push(decode_early_render_vfs_winner_row(
            bytes.get(at..end).ok_or(EarlyRenderVfsCodecError::Length)?,
        )?);
        at = end;
    }
    if at != bytes.len() {
        return Err(EarlyRenderVfsCodecError::Length);
    }
    let value = EarlyRenderVfsIndexChunkV1 {
        process_id,
        process_start_nonce,
        session_id,
        launch_bind_sha256,
        index_sha256,
        chunk_index,
        chunk_count,
        first_row_index,
        total_row_count,
        rows,
    };
    if encode_early_render_vfs_index_chunk(&value)? != bytes {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    Ok(value)
}

pub fn encode_early_render_vfs_index_commit(
    value: &EarlyRenderVfsIndexCommitV1,
) -> Result<Vec<u8>, EarlyRenderVfsCodecError> {
    if value.process_id == 0
        || value.process_start_nonce == 0
        || !nonzero(&value.session_id)
        || !nonzero(&value.launch_bind_sha256)
        || !nonzero(&value.index_sha256)
        || value.chunk_count == 0
        || value.total_row_count == 0
        || value.chunk_count > value.total_row_count
    {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    let mut out = Vec::with_capacity(COMMIT_LEN);
    put_header(&mut out, COMMIT_MAGIC);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.session_id);
    out.extend_from_slice(&value.launch_bind_sha256);
    out.extend_from_slice(&value.index_sha256);
    out.extend_from_slice(&value.chunk_count.to_le_bytes());
    out.extend_from_slice(&value.total_row_count.to_le_bytes());
    Ok(out)
}

pub fn decode_early_render_vfs_index_commit(
    bytes: &[u8],
) -> Result<EarlyRenderVfsIndexCommitV1, EarlyRenderVfsCodecError> {
    validate_header(bytes, COMMIT_MAGIC, COMMIT_LEN)?;
    if bytes.len() != COMMIT_LEN {
        return Err(EarlyRenderVfsCodecError::Length);
    }
    let mut at = 8;
    let value = EarlyRenderVfsIndexCommitV1 {
        process_id: take_u32(bytes, &mut at)?,
        process_start_nonce: take_u64(bytes, &mut at)?,
        session_id: take_array(bytes, &mut at)?,
        launch_bind_sha256: take_array(bytes, &mut at)?,
        index_sha256: take_array(bytes, &mut at)?,
        chunk_count: take_u32(bytes, &mut at)?,
        total_row_count: take_u32(bytes, &mut at)?,
    };
    if encode_early_render_vfs_index_commit(&value)? != bytes {
        return Err(EarlyRenderVfsCodecError::InvalidField);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str, byte: u8) -> EarlyRenderVfsWinnerRowV1 {
        EarlyRenderVfsWinnerRowV1 {
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 3,
            layer_kind: EarlyRenderVfsLayerKind::Mod,
            layer_id: "mod/test.mod".into(),
            source_sha256: [byte.wrapping_add(1); 32],
            payload_sha256: [byte.wrapping_add(1); 32],
            payload_bytes: 7,
            source_container_sha256: None,
            source_byte_offset: 0,
            source_byte_count: 7,
            source_range_sha256: [byte.wrapping_add(1); 32],
        }
    }

    #[test]
    fn exact_chunk_commit_and_canonical_index_roundtrip() {
        let rows = vec![row("gfx/a.asset", 1), row("gfx/b.asset", 3)];
        let canonical = canonical_early_render_vfs_index_bytes(&rows).unwrap();
        assert!(canonical.starts_with(b"hoi4-effective-vfs-winners-v1\0"));
        let chunk = EarlyRenderVfsIndexChunkV1 {
            process_id: 7,
            process_start_nonce: 8,
            session_id: [9; 16],
            launch_bind_sha256: [10; 32],
            index_sha256: [11; 32],
            chunk_index: 0,
            chunk_count: 1,
            first_row_index: 0,
            total_row_count: 2,
            rows,
        };
        let encoded = encode_early_render_vfs_index_chunk(&chunk).unwrap();
        assert_eq!(
            decode_early_render_vfs_index_chunk(&encoded).unwrap(),
            chunk
        );
        let commit = EarlyRenderVfsIndexCommitV1 {
            process_id: 7,
            process_start_nonce: 8,
            session_id: [9; 16],
            launch_bind_sha256: [10; 32],
            index_sha256: [11; 32],
            chunk_count: 1,
            total_row_count: 2,
        };
        let encoded = encode_early_render_vfs_index_commit(&commit).unwrap();
        assert_eq!(
            decode_early_render_vfs_index_commit(&encoded).unwrap(),
            commit
        );
    }

    #[test]
    fn duplicate_order_foreign_reserved_and_oversized_data_fail_closed() {
        let duplicate = vec![row("gfx/a.asset", 1), row("gfx/a.asset", 3)];
        assert_eq!(
            canonical_early_render_vfs_index_bytes(&duplicate),
            Err(EarlyRenderVfsCodecError::InvalidField)
        );
        let mut invalid = row("gfx/a.asset", 1);
        invalid.layer_id = "x".repeat(u16::MAX as usize + 1);
        assert_eq!(
            encode_early_render_vfs_winner_row(&invalid),
            Err(EarlyRenderVfsCodecError::InvalidField)
        );
        let mut encoded = encode_early_render_vfs_winner_row(&row("gfx/a.asset", 1)).unwrap();
        encoded[9] = 1;
        assert_eq!(
            decode_early_render_vfs_winner_row(&encoded),
            Err(EarlyRenderVfsCodecError::Reserved)
        );
    }
}
