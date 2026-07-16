//! Dependency-neutral CFA1 control payload contract shared by collector and recorder.
//!
//! The protocol mode is selected by the surrounding state machine and is an
//! explicit codec argument.  The wire is never sniffed to choose IdentityV1 or
//! Legacy, and neither mode falls back to the other.

use crate::calendar::hour_index_from_raw;
use crate::campaign_identity::{COMPACT_DOMAIN_STREAM_HEADER, COMPACT_RECORD_TYPE_STREAM_HEADER};
use crate::stream_validator::StreamIdentityMode;
use crate::{
    COMPACT_MAX_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END,
};

pub const FIRST_COMPACT_FRAME_ACK_MAGIC: &[u8; 4] = b"CFA1";
pub const FIRST_COMPACT_FRAME_ACK_LEN: usize = 104;
const WIRE_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirstCompactFrameAckV1 {
    pub mode: StreamIdentityMode,
    pub logical_stream_id: u64,
    pub h4cr_sequence: u64,
    pub game_hour_raw: i32,
    pub domain_id: u16,
    pub record_type: u16,
    pub complete_record_len: u32,
    pub complete_record_sha256: [u8; 32],
    pub payload_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirstCompactAckCodecError {
    InvalidLength { expected: usize, actual: usize },
    BadMagic,
    UnsupportedVersion(u16),
    DeclaredLength { expected: usize, actual: usize },
    NonZeroReserved,
    InvalidField(&'static str),
}

impl core::fmt::Display for FirstCompactAckCodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(f, "CFA1 payload length {actual} is not {expected}")
            }
            Self::BadMagic => write!(f, "CFA1 payload magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(f, "CFA1 payload version {version} is unsupported")
            }
            Self::DeclaredLength { expected, actual } => {
                write!(f, "CFA1 declared length {actual} is not {expected}")
            }
            Self::NonZeroReserved => write!(f, "CFA1 reserved field is nonzero"),
            Self::InvalidField(field) => write!(f, "CFA1 field {field} is invalid"),
        }
    }
}

impl FirstCompactFrameAckV1 {
    pub fn validate(self) -> Result<Self, FirstCompactAckCodecError> {
        if self.logical_stream_id == u64::MAX {
            return Err(FirstCompactAckCodecError::InvalidField("logical_stream_id"));
        }
        if hour_index_from_raw(self.game_hour_raw).is_none() {
            return Err(FirstCompactAckCodecError::InvalidField("game_hour_raw"));
        }
        let envelope_matches_mode = match self.mode {
            StreamIdentityMode::IdentityV1 => {
                self.h4cr_sequence == 0
                    && self.domain_id == COMPACT_DOMAIN_STREAM_HEADER
                    && self.record_type == COMPACT_RECORD_TYPE_STREAM_HEADER
            }
            StreamIdentityMode::Legacy => {
                self.h4cr_sequence == 1
                    && self.domain_id != COMPACT_DOMAIN_STREAM_HEADER
                    && matches!(
                        self.record_type,
                        COMPACT_RECORD_TYPE_DOMAIN_PERF
                            | COMPACT_RECORD_TYPE_FRAME_END
                            | COMPACT_RECORD_TYPE_DOMAIN_DATA
                    )
            }
        };
        if !envelope_matches_mode {
            return Err(FirstCompactAckCodecError::InvalidField(
                "first_record_envelope",
            ));
        }
        let min = COMPACT_RECORD_HEADER_LEN as u32;
        let max = min
            .checked_add(COMPACT_MAX_PAYLOAD_LEN)
            .expect("bounded compact record length");
        if !(min..=max).contains(&self.complete_record_len) {
            return Err(FirstCompactAckCodecError::InvalidField(
                "complete_record_len",
            ));
        }
        Ok(self)
    }
}

pub fn encode_first_compact_frame_ack(
    value: FirstCompactFrameAckV1,
) -> Result<[u8; FIRST_COMPACT_FRAME_ACK_LEN], FirstCompactAckCodecError> {
    let value = value.validate()?;
    let mut out = [0u8; FIRST_COMPACT_FRAME_ACK_LEN];
    out[0..4].copy_from_slice(FIRST_COMPACT_FRAME_ACK_MAGIC);
    out[4..6].copy_from_slice(&WIRE_VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&(FIRST_COMPACT_FRAME_ACK_LEN as u16).to_le_bytes());
    out[8..16].copy_from_slice(&value.logical_stream_id.to_le_bytes());
    out[16..24].copy_from_slice(&value.h4cr_sequence.to_le_bytes());
    out[24..28].copy_from_slice(&value.game_hour_raw.to_le_bytes());
    out[28..30].copy_from_slice(&value.domain_id.to_le_bytes());
    out[30..32].copy_from_slice(&value.record_type.to_le_bytes());
    out[32..36].copy_from_slice(&value.complete_record_len.to_le_bytes());
    out[40..72].copy_from_slice(&value.complete_record_sha256);
    out[72..104].copy_from_slice(&value.payload_sha256);
    Ok(out)
}

pub fn decode_first_compact_frame_ack(
    payload: &[u8],
    expected_mode: StreamIdentityMode,
) -> Result<FirstCompactFrameAckV1, FirstCompactAckCodecError> {
    if payload.len() != FIRST_COMPACT_FRAME_ACK_LEN {
        return Err(FirstCompactAckCodecError::InvalidLength {
            expected: FIRST_COMPACT_FRAME_ACK_LEN,
            actual: payload.len(),
        });
    }
    if &payload[0..4] != FIRST_COMPACT_FRAME_ACK_MAGIC {
        return Err(FirstCompactAckCodecError::BadMagic);
    }
    let version = read_u16(payload, 4);
    if version != WIRE_VERSION {
        return Err(FirstCompactAckCodecError::UnsupportedVersion(version));
    }
    let declared = read_u16(payload, 6) as usize;
    if declared != FIRST_COMPACT_FRAME_ACK_LEN {
        return Err(FirstCompactAckCodecError::DeclaredLength {
            expected: FIRST_COMPACT_FRAME_ACK_LEN,
            actual: declared,
        });
    }
    if read_u32(payload, 36) != 0 {
        return Err(FirstCompactAckCodecError::NonZeroReserved);
    }
    FirstCompactFrameAckV1 {
        mode: expected_mode,
        logical_stream_id: read_u64(payload, 8),
        h4cr_sequence: read_u64(payload, 16),
        game_hour_raw: read_i32(payload, 24),
        domain_id: read_u16(payload, 28),
        record_type: read_u16(payload, 30),
        complete_record_len: read_u32(payload, 32),
        complete_record_sha256: payload[40..72].try_into().expect("fixed hash slice"),
        payload_sha256: payload[72..104].try_into().expect("fixed hash slice"),
    }
    .validate()
}

fn read_u16(payload: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        payload[offset..offset + 2]
            .try_into()
            .expect("checked slice"),
    )
}

fn read_u32(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        payload[offset..offset + 4]
            .try_into()
            .expect("checked slice"),
    )
}

fn read_i32(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        payload[offset..offset + 4]
            .try_into()
            .expect("checked slice"),
    )
}

fn read_u64(payload: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        payload[offset..offset + 8]
            .try_into()
            .expect("checked slice"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::DATE_EPOCH_CONST;

    fn hour() -> i32 {
        DATE_EPOCH_CONST + 1936 * 365 * 24
    }

    fn value(mode: StreamIdentityMode) -> FirstCompactFrameAckV1 {
        let (h4cr_sequence, domain_id, record_type) = match mode {
            StreamIdentityMode::IdentityV1 => (
                0,
                COMPACT_DOMAIN_STREAM_HEADER,
                COMPACT_RECORD_TYPE_STREAM_HEADER,
            ),
            StreamIdentityMode::Legacy => (1, 10, COMPACT_RECORD_TYPE_DOMAIN_PERF),
        };
        FirstCompactFrameAckV1 {
            mode,
            logical_stream_id: 0,
            h4cr_sequence,
            game_hour_raw: hour(),
            domain_id,
            record_type,
            complete_record_len: 72,
            complete_record_sha256: [1; 32],
            payload_sha256: [2; 32],
        }
    }

    #[test]
    fn both_explicit_modes_roundtrip_without_sniffing() {
        for mode in [StreamIdentityMode::IdentityV1, StreamIdentityMode::Legacy] {
            let expected = value(mode);
            let wire = encode_first_compact_frame_ack(expected).unwrap();
            assert_eq!(
                decode_first_compact_frame_ack(&wire, mode).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn mode_crosswire_is_rejected() {
        let identity =
            encode_first_compact_frame_ack(value(StreamIdentityMode::IdentityV1)).unwrap();
        assert!(decode_first_compact_frame_ack(&identity, StreamIdentityMode::Legacy).is_err());
        let legacy = encode_first_compact_frame_ack(value(StreamIdentityMode::Legacy)).unwrap();
        assert!(decode_first_compact_frame_ack(&legacy, StreamIdentityMode::IdentityV1).is_err());
    }

    #[test]
    fn reserved_length_and_short_payloads_fail_closed() {
        let wire = encode_first_compact_frame_ack(value(StreamIdentityMode::IdentityV1)).unwrap();
        for len in 0..FIRST_COMPACT_FRAME_ACK_LEN {
            assert!(
                decode_first_compact_frame_ack(&wire[..len], StreamIdentityMode::IdentityV1)
                    .is_err()
            );
        }
        let mut reserved = wire;
        reserved[36] = 1;
        assert_eq!(
            decode_first_compact_frame_ack(&reserved, StreamIdentityMode::IdentityV1),
            Err(FirstCompactAckCodecError::NonZeroReserved)
        );
    }
}
