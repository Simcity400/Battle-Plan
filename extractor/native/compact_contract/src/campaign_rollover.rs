use crate::calendar::hour_index_from_raw;

pub const ROLLOVER_PROPOSE_MAGIC: &[u8; 4] = b"CRP1";
pub const ROLLOVER_ACK_MAGIC: &[u8; 4] = b"CRA1";
pub const ROLLOVER_VERSION: u16 = 1;
pub const ROLLOVER_PROPOSE_LEN: usize = 168;
pub const ROLLOVER_ACK_LEN: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum RolloverReason {
    WholePairChanged = 1,
    SamePairTimeRollback = 2,
}

crate::wire_coded_codes! {
    RolloverReason: u16,
    code = pub code,
    Ok = decode(raw) -> Result<Self, RolloverCodecError> { Err(RolloverCodecError::UnknownReason(raw)) };

    WholePairChanged = 1,
    SamePairTimeRollback = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum RolloverDecision {
    Accept = 1,
    Reject = 2,
}

crate::wire_coded_codes! {
    RolloverDecision: u16,
    code = pub code,
    Ok = decode(raw) -> Result<Self, RolloverCodecError> { Err(RolloverCodecError::UnknownDecision(raw)) };

    Accept = 1,
    Reject = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RolloverProposeV1 {
    pub proposed_logical_stream_id: u64,
    pub last_admitted_h4cr_sequence: u64,
    pub boundary_game_hour_raw: i32,
    pub reason: RolloverReason,
    pub current_campaign_id: [u8; 32],
    pub proposed_campaign_id: [u8; 32],
    pub current_cid1_payload_sha256: [u8; 32],
    pub proposed_cid1_payload_sha256: [u8; 32],
    pub proposal_nonce: u64,
}

impl RolloverProposeV1 {
    pub fn validate(&self) -> Result<(), RolloverCodecError> {
        if self.proposed_logical_stream_id == 0 || self.proposed_logical_stream_id == u64::MAX {
            return Err(RolloverCodecError::InvalidField(
                "proposed_logical_stream_id",
            ));
        }
        if self.last_admitted_h4cr_sequence == u64::MAX {
            return Err(RolloverCodecError::InvalidField(
                "last_admitted_h4cr_sequence",
            ));
        }
        if hour_index_from_raw(self.boundary_game_hour_raw).is_none() {
            return Err(RolloverCodecError::InvalidField("boundary_game_hour_raw"));
        }
        if self.proposal_nonce == 0 || self.proposal_nonce == u64::MAX {
            return Err(RolloverCodecError::InvalidField("proposal_nonce"));
        }
        for (field, digest) in [
            ("current_campaign_id", self.current_campaign_id),
            ("proposed_campaign_id", self.proposed_campaign_id),
            (
                "current_cid1_payload_sha256",
                self.current_cid1_payload_sha256,
            ),
            (
                "proposed_cid1_payload_sha256",
                self.proposed_cid1_payload_sha256,
            ),
        ] {
            if digest == [0; 32] {
                return Err(RolloverCodecError::InvalidField(field));
            }
        }
        let campaign_ids_equal = self.current_campaign_id == self.proposed_campaign_id;
        match self.reason {
            RolloverReason::WholePairChanged if campaign_ids_equal => Err(
                RolloverCodecError::InvalidField("whole_pair_changed_campaign_ids"),
            ),
            RolloverReason::SamePairTimeRollback if !campaign_ids_equal => Err(
                RolloverCodecError::InvalidField("same_pair_rollback_campaign_ids"),
            ),
            _ if self.current_cid1_payload_sha256 == self.proposed_cid1_payload_sha256 => Err(
                RolloverCodecError::InvalidField("cid1_payload_hash_did_not_change"),
            ),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RolloverAckV1 {
    pub proposed_logical_stream_id: u64,
    pub proposal_nonce: u64,
    pub decision: RolloverDecision,
    pub error_code: u16,
}

impl RolloverAckV1 {
    pub fn validate(&self) -> Result<(), RolloverCodecError> {
        if self.proposed_logical_stream_id == 0 || self.proposed_logical_stream_id == u64::MAX {
            return Err(RolloverCodecError::InvalidField(
                "proposed_logical_stream_id",
            ));
        }
        if self.proposal_nonce == 0 || self.proposal_nonce == u64::MAX {
            return Err(RolloverCodecError::InvalidField("proposal_nonce"));
        }
        match (self.decision, self.error_code) {
            (RolloverDecision::Accept, 0) | (RolloverDecision::Reject, 1..=u16::MAX) => Ok(()),
            _ => Err(RolloverCodecError::InvalidField(
                "decision_error_consistency",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RolloverCodecError {
    WrongLength { expected: usize, actual: usize },
    BadMagic,
    UnsupportedVersion(u16),
    DeclaredLength { expected: u16, actual: u16 },
    NonZeroReserved,
    UnknownReason(u16),
    UnknownDecision(u16),
    InvalidField(&'static str),
}

impl core::fmt::Display for RolloverCodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongLength { expected, actual } => {
                write!(f, "rollover payload length {actual} is not {expected}")
            }
            Self::BadMagic => write!(f, "rollover payload magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(f, "rollover payload version {version} is unsupported")
            }
            Self::DeclaredLength { expected, actual } => {
                write!(f, "rollover declared length {actual} is not {expected}")
            }
            Self::NonZeroReserved => write!(f, "rollover reserved field is nonzero"),
            Self::UnknownReason(reason) => write!(f, "unknown rollover reason {reason}"),
            Self::UnknownDecision(decision) => {
                write!(f, "unknown rollover decision {decision}")
            }
            Self::InvalidField(field) => write!(f, "rollover field {field} is invalid"),
        }
    }
}

impl std::error::Error for RolloverCodecError {}

pub fn encode_rollover_propose(
    value: &RolloverProposeV1,
) -> Result<[u8; ROLLOVER_PROPOSE_LEN], RolloverCodecError> {
    value.validate()?;
    let mut out = [0u8; ROLLOVER_PROPOSE_LEN];
    out[0..4].copy_from_slice(ROLLOVER_PROPOSE_MAGIC);
    out[4..6].copy_from_slice(&ROLLOVER_VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&(ROLLOVER_PROPOSE_LEN as u16).to_le_bytes());
    out[8..16].copy_from_slice(&value.proposed_logical_stream_id.to_le_bytes());
    out[16..24].copy_from_slice(&value.last_admitted_h4cr_sequence.to_le_bytes());
    out[24..28].copy_from_slice(&value.boundary_game_hour_raw.to_le_bytes());
    out[28..30].copy_from_slice(&value.reason.code().to_le_bytes());
    out[32..64].copy_from_slice(&value.current_campaign_id);
    out[64..96].copy_from_slice(&value.proposed_campaign_id);
    out[96..128].copy_from_slice(&value.current_cid1_payload_sha256);
    out[128..160].copy_from_slice(&value.proposed_cid1_payload_sha256);
    out[160..168].copy_from_slice(&value.proposal_nonce.to_le_bytes());
    Ok(out)
}

pub fn decode_rollover_propose(payload: &[u8]) -> Result<RolloverProposeV1, RolloverCodecError> {
    check_header(payload, ROLLOVER_PROPOSE_MAGIC, ROLLOVER_PROPOSE_LEN)?;
    if read_u16(payload, 30) != 0 {
        return Err(RolloverCodecError::NonZeroReserved);
    }
    let value = RolloverProposeV1 {
        proposed_logical_stream_id: read_u64(payload, 8),
        last_admitted_h4cr_sequence: read_u64(payload, 16),
        boundary_game_hour_raw: read_i32(payload, 24),
        reason: RolloverReason::decode(read_u16(payload, 28))?,
        current_campaign_id: read_bytes32(payload, 32),
        proposed_campaign_id: read_bytes32(payload, 64),
        current_cid1_payload_sha256: read_bytes32(payload, 96),
        proposed_cid1_payload_sha256: read_bytes32(payload, 128),
        proposal_nonce: read_u64(payload, 160),
    };
    value.validate()?;
    Ok(value)
}

pub fn encode_rollover_ack(
    value: &RolloverAckV1,
) -> Result<[u8; ROLLOVER_ACK_LEN], RolloverCodecError> {
    value.validate()?;
    let mut out = [0u8; ROLLOVER_ACK_LEN];
    out[0..4].copy_from_slice(ROLLOVER_ACK_MAGIC);
    out[4..6].copy_from_slice(&ROLLOVER_VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&(ROLLOVER_ACK_LEN as u16).to_le_bytes());
    out[8..16].copy_from_slice(&value.proposed_logical_stream_id.to_le_bytes());
    out[16..24].copy_from_slice(&value.proposal_nonce.to_le_bytes());
    out[24..26].copy_from_slice(&value.decision.code().to_le_bytes());
    out[26..28].copy_from_slice(&value.error_code.to_le_bytes());
    Ok(out)
}

pub fn decode_rollover_ack(payload: &[u8]) -> Result<RolloverAckV1, RolloverCodecError> {
    check_header(payload, ROLLOVER_ACK_MAGIC, ROLLOVER_ACK_LEN)?;
    if read_u32(payload, 28) != 0 {
        return Err(RolloverCodecError::NonZeroReserved);
    }
    let value = RolloverAckV1 {
        proposed_logical_stream_id: read_u64(payload, 8),
        proposal_nonce: read_u64(payload, 16),
        decision: RolloverDecision::decode(read_u16(payload, 24))?,
        error_code: read_u16(payload, 26),
    };
    value.validate()?;
    Ok(value)
}

fn check_header(
    payload: &[u8],
    magic: &[u8; 4],
    expected_len: usize,
) -> Result<(), RolloverCodecError> {
    if payload.len() != expected_len {
        return Err(RolloverCodecError::WrongLength {
            expected: expected_len,
            actual: payload.len(),
        });
    }
    if &payload[0..4] != magic {
        return Err(RolloverCodecError::BadMagic);
    }
    let version = read_u16(payload, 4);
    if version != ROLLOVER_VERSION {
        return Err(RolloverCodecError::UnsupportedVersion(version));
    }
    let declared = read_u16(payload, 6);
    if usize::from(declared) != expected_len {
        return Err(RolloverCodecError::DeclaredLength {
            expected: expected_len as u16,
            actual: declared,
        });
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("length checked"),
    )
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("length checked"),
    )
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("length checked"),
    )
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("length checked"),
    )
}

fn read_bytes32(bytes: &[u8], offset: usize) -> [u8; 32] {
    bytes[offset..offset + 32]
        .try_into()
        .expect("length checked")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::DATE_EPOCH_CONST;

    fn hour() -> i32 {
        DATE_EPOCH_CONST + ((1936 * 365 + 10) * 24 + 5)
    }

    fn changed() -> RolloverProposeV1 {
        RolloverProposeV1 {
            proposed_logical_stream_id: 1,
            last_admitted_h4cr_sequence: 99,
            boundary_game_hour_raw: hour(),
            reason: RolloverReason::WholePairChanged,
            current_campaign_id: [1; 32],
            proposed_campaign_id: [2; 32],
            current_cid1_payload_sha256: [3; 32],
            proposed_cid1_payload_sha256: [4; 32],
            proposal_nonce: 7,
        }
    }

    #[test]
    fn proposal_round_trips_exact_crp1_layout() {
        let value = changed();
        let wire = encode_rollover_propose(&value).unwrap();
        assert_eq!(&wire[0..4], b"CRP1");
        assert_eq!(wire.len(), 168);
        assert_eq!(&wire[32..64], &value.current_campaign_id);
        assert_eq!(&wire[64..96], &value.proposed_campaign_id);
        assert_eq!(&wire[96..128], &value.current_cid1_payload_sha256);
        assert_eq!(&wire[128..160], &value.proposed_cid1_payload_sha256);
        assert_eq!(decode_rollover_propose(&wire).unwrap(), value);
    }

    #[test]
    fn reason_hash_relationship_and_reserved_bytes_fail_closed() {
        let mut same = changed();
        same.reason = RolloverReason::SamePairTimeRollback;
        assert!(encode_rollover_propose(&same).is_err());
        same.proposed_campaign_id = same.current_campaign_id;
        let mut wire = encode_rollover_propose(&same).unwrap();
        wire[30] = 1;
        assert_eq!(
            decode_rollover_propose(&wire),
            Err(RolloverCodecError::NonZeroReserved)
        );
    }

    #[test]
    fn same_pair_rollback_requires_equal_pair_but_distinct_full_cid1_hashes() {
        let mut same = changed();
        same.reason = RolloverReason::SamePairTimeRollback;
        same.proposed_campaign_id = same.current_campaign_id;
        assert!(encode_rollover_propose(&same).is_ok());

        same.proposed_cid1_payload_sha256 = same.current_cid1_payload_sha256;
        assert_eq!(
            same.validate(),
            Err(RolloverCodecError::InvalidField(
                "cid1_payload_hash_did_not_change"
            ))
        );
    }

    #[test]
    fn whole_pair_reason_is_independent_of_full_cid1_hash_equality() {
        let mut changed = changed();
        changed.proposed_campaign_id = changed.current_campaign_id;
        assert_eq!(
            changed.validate(),
            Err(RolloverCodecError::InvalidField(
                "whole_pair_changed_campaign_ids"
            ))
        );

        changed.proposed_campaign_id = [9; 32];
        changed.proposed_cid1_payload_sha256 = changed.current_cid1_payload_sha256;
        assert_eq!(
            changed.validate(),
            Err(RolloverCodecError::InvalidField(
                "cid1_payload_hash_did_not_change"
            ))
        );
    }

    #[test]
    fn ack_round_trips_and_decision_error_pair_is_closed() {
        let accepted = RolloverAckV1 {
            proposed_logical_stream_id: 1,
            proposal_nonce: 7,
            decision: RolloverDecision::Accept,
            error_code: 0,
        };
        let wire = encode_rollover_ack(&accepted).unwrap();
        assert_eq!(&wire[0..4], b"CRA1");
        assert_eq!(decode_rollover_ack(&wire).unwrap(), accepted);

        let mut invalid = accepted;
        invalid.error_code = 1;
        assert!(encode_rollover_ack(&invalid).is_err());
        invalid.decision = RolloverDecision::Reject;
        assert!(encode_rollover_ack(&invalid).is_ok());
    }

    #[test]
    fn wrong_length_version_enum_and_calendar_are_rejected() {
        assert!(matches!(
            decode_rollover_propose(&[0; 8]),
            Err(RolloverCodecError::WrongLength { .. })
        ));
        let mut wire = encode_rollover_propose(&changed()).unwrap();
        wire[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert!(matches!(
            decode_rollover_propose(&wire),
            Err(RolloverCodecError::UnsupportedVersion(2))
        ));
        let mut bad_hour = changed();
        bad_hour.boundary_game_hour_raw = 0;
        assert!(encode_rollover_propose(&bad_hour).is_err());
    }
}
