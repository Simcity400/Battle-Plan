//! Dependency-free H4CR record-header contract.
//!
//! This module is deliberately self-contained so documented direct-`rustc`
//! validators can consume the canonical framing primitive through `#[path]`.
//! Keep it free of crate-local imports and external dependencies.

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_MAX_PAYLOAD_LEN: u32 = 1 << 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactRecordHeader {
    pub sequence: u64,
    pub game_hour_raw: i32,
    pub domain_id: u16,
    pub record_type: u16,
    pub payload_len: usize,
    pub record_flags: u32,
}

impl CompactRecordHeader {
    pub fn record_len(&self) -> usize {
        COMPACT_RECORD_HEADER_LEN.saturating_add(self.payload_len)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordFramingError {
    TruncatedHeader(usize),
    BadMagic,
    UnsupportedVersion(u16),
    UnsupportedHeaderLen(usize),
    PayloadTooLarge(u32),
}

impl core::fmt::Display for RecordFramingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RecordFramingError::TruncatedHeader(actual) => write!(
                f,
                "truncated record header: {actual} byte(s), expected {COMPACT_RECORD_HEADER_LEN}"
            ),
            RecordFramingError::BadMagic => write!(f, "invalid record magic"),
            RecordFramingError::UnsupportedVersion(v) => {
                write!(f, "unsupported record version {v}")
            }
            RecordFramingError::UnsupportedHeaderLen(l) => {
                write!(f, "unsupported header length {l}")
            }
            RecordFramingError::PayloadTooLarge(l) => write!(
                f,
                "payload length {l} exceeds the {COMPACT_MAX_PAYLOAD_LEN}-byte record bound"
            ),
        }
    }
}

/// Parse the fixed H4CR header. Total over arbitrary byte slices.
pub fn parse_compact_record_header(buf: &[u8]) -> Result<CompactRecordHeader, RecordFramingError> {
    if buf.len() < COMPACT_RECORD_HEADER_LEN {
        return Err(RecordFramingError::TruncatedHeader(buf.len()));
    }
    if &buf[0..4] != COMPACT_RECORD_MAGIC {
        return Err(RecordFramingError::BadMagic);
    }
    let version = u16::from_le_bytes([buf[4], buf[5]]);
    if version != COMPACT_RECORD_VERSION {
        return Err(RecordFramingError::UnsupportedVersion(version));
    }
    let header_len = u16::from_le_bytes([buf[6], buf[7]]) as usize;
    if header_len != COMPACT_RECORD_HEADER_LEN {
        return Err(RecordFramingError::UnsupportedHeaderLen(header_len));
    }
    let payload_len = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);
    if payload_len > COMPACT_MAX_PAYLOAD_LEN {
        return Err(RecordFramingError::PayloadTooLarge(payload_len));
    }
    Ok(CompactRecordHeader {
        sequence: u64::from_le_bytes([
            buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
        ]),
        game_hour_raw: i32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
        domain_id: u16::from_le_bytes([buf[20], buf[21]]),
        record_type: u16::from_le_bytes([buf[22], buf[23]]),
        payload_len: payload_len as usize,
        record_flags: u32::from_le_bytes([buf[28], buf[29], buf[30], buf[31]]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_short_record_header_length_is_safe_and_explicit() {
        for len in 0..COMPACT_RECORD_HEADER_LEN {
            let bytes = vec![0xA5; len];
            assert_eq!(
                parse_compact_record_header(&bytes),
                Err(RecordFramingError::TruncatedHeader(len)),
                "short length {len} did not return its exact truncation"
            );
        }
    }

    #[test]
    fn arbitrary_header_slices_never_panic() {
        let mut state = 0xD1B5_4A32_D192_ED03u64;
        for case in 0..16_384usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 513;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            let parsed = std::panic::catch_unwind(|| parse_compact_record_header(&bytes));
            assert!(
                parsed.is_ok(),
                "parser panicked for fuzz case {case}, len {len}"
            );
            if len < COMPACT_RECORD_HEADER_LEN {
                assert_eq!(
                    parsed.unwrap(),
                    Err(RecordFramingError::TruncatedHeader(len)),
                    "fuzz case {case} lost exact short length"
                );
            }
        }
    }
}
