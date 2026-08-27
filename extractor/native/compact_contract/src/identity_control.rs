use std::fmt;

use crate::campaign_identity::Sha256;
use crate::control_transport::MessageKind;

pub const IDENTITY_FAILURE_MAGIC: &[u8; 4] = b"CIF1";
pub const IDENTITY_FAILURE_FIXED_LEN: usize = 116;
pub const IDENTITY_FAILURE_ACK_MAGIC: &[u8; 4] = b"CIA1";
pub const IDENTITY_FAILURE_ACK_LEN: usize = 32;
const WIRE_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityControlCodecError {
    InvalidLength { expected: usize, actual: usize },
    BadMagic,
    UnsupportedVersion(u16),
    DeclaredLength { expected: usize, actual: usize },
    NonZeroReserved,
    InvalidField(&'static str),
    UnknownReason(u16),
    UnknownDisposition(u16),
    UnknownFlags(u16),
    SuffixHashMismatch,
}

impl fmt::Display for IdentityControlCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(f, "payload length {actual} is not {expected}")
            }
            Self::BadMagic => write!(f, "payload magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(f, "payload version {version} is unsupported")
            }
            Self::DeclaredLength { expected, actual } => {
                write!(f, "declared payload length {actual} is not {expected}")
            }
            Self::NonZeroReserved => write!(f, "payload reserved field is nonzero"),
            Self::InvalidField(field) => write!(f, "payload field {field} is invalid"),
            Self::UnknownReason(reason) => write!(f, "identity failure reason {reason} is unknown"),
            Self::UnknownDisposition(disposition) => {
                write!(
                    f,
                    "identity failure ack disposition {disposition} is unknown"
                )
            }
            Self::UnknownFlags(flags) => {
                write!(
                    f,
                    "identity failure flags contain unknown bits {flags:#06x}"
                )
            }
            Self::SuffixHashMismatch => write!(f, "identity failure suffix hash mismatched"),
        }
    }
}

impl std::error::Error for IdentityControlCodecError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum IdentityFailureReason {
    UnreadableRoot = 1,
    TornRead = 2,
    UuidMissing = 3,
    SeedMissing = 4,
    UuidMalformed = 5,
    SeedMalformed = 6,
    InvalidHour = 7,
    InvalidProfile = 8,
    StabilityIncompleteOrMismatch = 9,
    ComponentTransitionConflict = 10,
    Cid1EncodeOrHashFailure = 11,
}

crate::wire_coded_codes! {
    IdentityFailureReason: u16,
    code = pub code,
    Ok = from_wire(value) -> Result<Self, IdentityControlCodecError> { Err(IdentityControlCodecError::UnknownReason(value)) };

    UnreadableRoot = 1,
    TornRead = 2,
    UuidMissing = 3,
    SeedMissing = 4,
    UuidMalformed = 5,
    SeedMalformed = 6,
    InvalidHour = 7,
    InvalidProfile = 8,
    StabilityIncompleteOrMismatch = 9,
    ComponentTransitionConflict = 10,
    Cid1EncodeOrHashFailure = 11,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityFailureV1 {
    pub reason: IdentityFailureReason,
    pub current_logical_id: Option<u64>,
    pub attempt_id: u64,
    pub attempted_hour: Option<i32>,
    pub uuid_raw: Option<Vec<u8>>,
    pub seed_raw: Option<Vec<u8>>,
    pub profile_raw: Option<Vec<u8>>,
    pub first_read_complete: bool,
    pub second_read_complete: bool,
    pub reads_equal: bool,
    pub last_admitted_sequence: Option<u64>,
    pub current_cid1_hash: Option<[u8; 32]>,
}

impl IdentityFailureV1 {
    pub const MESSAGE_KIND: MessageKind = MessageKind::IdentityFailure;

    fn validate(&self) -> Result<(), IdentityControlCodecError> {
        for (name, value) in [
            ("uuid_raw", self.uuid_raw.as_deref()),
            ("seed_raw", self.seed_raw.as_deref()),
            ("profile_raw", self.profile_raw.as_deref()),
        ] {
            if let Some(bytes) = value {
                if bytes.is_empty() || bytes.len() > u16::MAX as usize {
                    return Err(IdentityControlCodecError::InvalidField(name));
                }
            }
        }
        if self.current_logical_id == Some(u64::MAX) {
            return Err(IdentityControlCodecError::InvalidField(
                "current_logical_id",
            ));
        }
        if self.last_admitted_sequence == Some(u64::MAX) {
            return Err(IdentityControlCodecError::InvalidField(
                "last_admitted_sequence",
            ));
        }
        if self.current_logical_id.is_none()
            && (self.last_admitted_sequence.is_some() || self.current_cid1_hash.is_some())
        {
            return Err(IdentityControlCodecError::InvalidField(
                "unbound_current_stream",
            ));
        }
        if self.current_logical_id.is_some() != self.current_cid1_hash.is_some() {
            return Err(IdentityControlCodecError::InvalidField("current_cid1_hash"));
        }
        if self.current_cid1_hash == Some([0; 32]) {
            return Err(IdentityControlCodecError::InvalidField("current_cid1_hash"));
        }
        if let Some(hour) = self.attempted_hour {
            if crate::calendar::hour_index_from_raw(hour).is_none() {
                return Err(IdentityControlCodecError::InvalidField("attempted_hour"));
            }
        }
        if self.reads_equal && (!self.first_read_complete || !self.second_read_complete) {
            return Err(IdentityControlCodecError::InvalidField("reads_equal"));
        }
        if self.reason == IdentityFailureReason::ComponentTransitionConflict
            && (!self.first_read_complete
                || !self.second_read_complete
                || !self.reads_equal
                || self.uuid_raw.is_none()
                || self.seed_raw.is_none()
                || self.profile_raw.is_none()
                || self.attempted_hour.is_none()
                || self.current_cid1_hash.is_none())
        {
            return Err(IdentityControlCodecError::InvalidField(
                "component_transition_conflict_evidence",
            ));
        }
        Ok(())
    }

    fn flags(&self) -> u16 {
        (u16::from(self.uuid_raw.is_some()) << 0)
            | (u16::from(self.seed_raw.is_some()) << 1)
            | (u16::from(self.profile_raw.is_some()) << 2)
            | (u16::from(self.attempted_hour.is_some()) << 3)
            | (u16::from(self.first_read_complete) << 4)
            | (u16::from(self.second_read_complete) << 5)
            | (u16::from(self.reads_equal) << 6)
    }
}

pub fn encode_identity_failure(
    value: &IdentityFailureV1,
) -> Result<Vec<u8>, IdentityControlCodecError> {
    value.validate()?;
    let uuid = value.uuid_raw.as_deref().unwrap_or_default();
    let seed = value.seed_raw.as_deref().unwrap_or_default();
    let profile = value.profile_raw.as_deref().unwrap_or_default();
    let suffix_len = uuid
        .len()
        .checked_add(seed.len())
        .and_then(|len| len.checked_add(profile.len()))
        .ok_or(IdentityControlCodecError::InvalidField("suffix_length"))?;
    let total_len = IDENTITY_FAILURE_FIXED_LEN
        .checked_add(suffix_len)
        .ok_or(IdentityControlCodecError::InvalidField("total_length"))?;
    let mut out = vec![0u8; total_len];
    out[0..4].copy_from_slice(IDENTITY_FAILURE_MAGIC);
    out[4..6].copy_from_slice(&WIRE_VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&(IDENTITY_FAILURE_FIXED_LEN as u16).to_le_bytes());
    out[8..12].copy_from_slice(&(total_len as u32).to_le_bytes());
    out[12..14].copy_from_slice(&value.reason.code().to_le_bytes());
    out[14..16].copy_from_slice(&value.flags().to_le_bytes());
    out[16..24].copy_from_slice(&value.current_logical_id.unwrap_or(u64::MAX).to_le_bytes());
    out[24..32].copy_from_slice(&value.attempt_id.to_le_bytes());
    out[32..36].copy_from_slice(&value.attempted_hour.unwrap_or(0).to_le_bytes());
    out[36..38].copy_from_slice(&(uuid.len() as u16).to_le_bytes());
    out[38..40].copy_from_slice(&(seed.len() as u16).to_le_bytes());
    out[40..42].copy_from_slice(&(profile.len() as u16).to_le_bytes());
    out[44..52].copy_from_slice(
        &value
            .last_admitted_sequence
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    out[52..84].copy_from_slice(&value.current_cid1_hash.unwrap_or([0; 32]));
    let mut cursor = IDENTITY_FAILURE_FIXED_LEN;
    for bytes in [uuid, seed, profile] {
        out[cursor..cursor + bytes.len()].copy_from_slice(bytes);
        cursor += bytes.len();
    }
    let suffix_hash = Sha256::digest(&out[IDENTITY_FAILURE_FIXED_LEN..]);
    out[84..116].copy_from_slice(&suffix_hash);
    Ok(out)
}

pub fn decode_identity_failure(
    payload: &[u8],
) -> Result<IdentityFailureV1, IdentityControlCodecError> {
    if payload.len() < IDENTITY_FAILURE_FIXED_LEN {
        return Err(IdentityControlCodecError::InvalidLength {
            expected: IDENTITY_FAILURE_FIXED_LEN,
            actual: payload.len(),
        });
    }
    validate_variable_prefix(payload, IDENTITY_FAILURE_MAGIC, IDENTITY_FAILURE_FIXED_LEN)?;
    let total_len = read_u32(payload, 8) as usize;
    if total_len != payload.len() {
        return Err(IdentityControlCodecError::DeclaredLength {
            expected: payload.len(),
            actual: total_len,
        });
    }
    let flags = read_u16(payload, 14);
    const KNOWN_FLAGS: u16 = (1 << 7) - 1;
    if flags & !KNOWN_FLAGS != 0 {
        return Err(IdentityControlCodecError::UnknownFlags(
            flags & !KNOWN_FLAGS,
        ));
    }
    if read_u16(payload, 42) != 0 {
        return Err(IdentityControlCodecError::NonZeroReserved);
    }
    let lengths = [
        read_u16(payload, 36) as usize,
        read_u16(payload, 38) as usize,
        read_u16(payload, 40) as usize,
    ];
    let suffix_len = lengths
        .into_iter()
        .try_fold(0usize, |sum, len| sum.checked_add(len))
        .ok_or(IdentityControlCodecError::InvalidField("suffix_length"))?;
    if IDENTITY_FAILURE_FIXED_LEN.checked_add(suffix_len) != Some(payload.len()) {
        return Err(IdentityControlCodecError::InvalidField("suffix_length"));
    }
    if payload[84..116] != Sha256::digest(&payload[IDENTITY_FAILURE_FIXED_LEN..]) {
        return Err(IdentityControlCodecError::SuffixHashMismatch);
    }
    let mut cursor = IDENTITY_FAILURE_FIXED_LEN;
    let mut take = |len: usize| {
        let bytes = payload[cursor..cursor + len].to_vec();
        cursor += len;
        bytes
    };
    let raw = [take(lengths[0]), take(lengths[1]), take(lengths[2])];
    let optional_raw = |bit: u16,
                        bytes: Vec<u8>,
                        field: &'static str|
     -> Result<Option<Vec<u8>>, IdentityControlCodecError> {
        let present = flags & bit != 0;
        if present != !bytes.is_empty() {
            return Err(IdentityControlCodecError::InvalidField(field));
        }
        Ok(present.then_some(bytes))
    };
    let logical_raw = read_u64(payload, 16);
    let last_raw = read_u64(payload, 44);
    let hash: [u8; 32] = payload[52..84].try_into().expect("fixed hash slice");
    let hour_valid = flags & (1 << 3) != 0;
    let hour_raw = read_i32(payload, 32);
    if hour_valid != (hour_raw != 0) {
        return Err(IdentityControlCodecError::InvalidField("attempted_hour"));
    }
    let value = IdentityFailureV1 {
        reason: IdentityFailureReason::from_wire(read_u16(payload, 12))?,
        current_logical_id: (logical_raw != u64::MAX).then_some(logical_raw),
        attempt_id: read_u64(payload, 24),
        attempted_hour: hour_valid.then_some(hour_raw),
        uuid_raw: optional_raw(1 << 0, raw[0].clone(), "uuid_raw")?,
        seed_raw: optional_raw(1 << 1, raw[1].clone(), "seed_raw")?,
        profile_raw: optional_raw(1 << 2, raw[2].clone(), "profile_raw")?,
        first_read_complete: flags & (1 << 4) != 0,
        second_read_complete: flags & (1 << 5) != 0,
        reads_equal: flags & (1 << 6) != 0,
        last_admitted_sequence: (last_raw != u64::MAX).then_some(last_raw),
        current_cid1_hash: (hash != [0; 32]).then_some(hash),
    };
    value.validate()?;
    Ok(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum IdentityFailureAckDisposition {
    DurablyRecorded = 1,
    Rejected = 2,
}

crate::wire_coded_codes! {
    IdentityFailureAckDisposition: u16,
    code = pub code,
    Ok = from_wire(value) -> Result<Self, IdentityControlCodecError> { Err(IdentityControlCodecError::UnknownDisposition(value)) };

    DurablyRecorded = 1,
    Rejected = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityFailureAckV1 {
    pub attempt_id: u64,
    pub logical_id: Option<u64>,
    pub disposition: IdentityFailureAckDisposition,
    pub error: u16,
}

impl IdentityFailureAckV1 {
    pub const MESSAGE_KIND: MessageKind = MessageKind::IdentityFailureAck;

    fn validate(self) -> Result<Self, IdentityControlCodecError> {
        if self.logical_id == Some(u64::MAX) {
            return Err(IdentityControlCodecError::InvalidField("logical_id"));
        }
        let coherent = match self.disposition {
            IdentityFailureAckDisposition::DurablyRecorded => self.error == 0,
            IdentityFailureAckDisposition::Rejected => self.error != 0,
        };
        if !coherent {
            return Err(IdentityControlCodecError::InvalidField("disposition_error"));
        }
        Ok(self)
    }
}

pub fn encode_identity_failure_ack(
    value: IdentityFailureAckV1,
) -> Result<[u8; IDENTITY_FAILURE_ACK_LEN], IdentityControlCodecError> {
    let value = value.validate()?;
    let mut out = [0u8; IDENTITY_FAILURE_ACK_LEN];
    out[0..4].copy_from_slice(IDENTITY_FAILURE_ACK_MAGIC);
    out[4..6].copy_from_slice(&WIRE_VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&(IDENTITY_FAILURE_ACK_LEN as u16).to_le_bytes());
    out[8..16].copy_from_slice(&value.attempt_id.to_le_bytes());
    out[16..24].copy_from_slice(&value.logical_id.unwrap_or(u64::MAX).to_le_bytes());
    out[24..26].copy_from_slice(&value.disposition.code().to_le_bytes());
    out[26..28].copy_from_slice(&value.error.to_le_bytes());
    Ok(out)
}

pub fn decode_identity_failure_ack(
    payload: &[u8],
) -> Result<IdentityFailureAckV1, IdentityControlCodecError> {
    validate_fixed_prefix(
        payload,
        IDENTITY_FAILURE_ACK_MAGIC,
        IDENTITY_FAILURE_ACK_LEN,
    )?;
    if read_u32(payload, 28) != 0 {
        return Err(IdentityControlCodecError::NonZeroReserved);
    }
    let logical_id = read_u64(payload, 16);
    IdentityFailureAckV1 {
        attempt_id: read_u64(payload, 8),
        logical_id: (logical_id != u64::MAX).then_some(logical_id),
        disposition: IdentityFailureAckDisposition::from_wire(read_u16(payload, 24))?,
        error: read_u16(payload, 26),
    }
    .validate()
}
fn validate_fixed_prefix(
    payload: &[u8],
    magic: &[u8; 4],
    expected_len: usize,
) -> Result<(), IdentityControlCodecError> {
    if payload.len() != expected_len {
        return Err(IdentityControlCodecError::InvalidLength {
            expected: expected_len,
            actual: payload.len(),
        });
    }
    validate_variable_prefix(payload, magic, expected_len)
}

fn validate_variable_prefix(
    payload: &[u8],
    magic: &[u8; 4],
    expected_header_len: usize,
) -> Result<(), IdentityControlCodecError> {
    if &payload[0..4] != magic {
        return Err(IdentityControlCodecError::BadMagic);
    }
    let version = read_u16(payload, 4);
    if version != WIRE_VERSION {
        return Err(IdentityControlCodecError::UnsupportedVersion(version));
    }
    let actual = read_u16(payload, 6) as usize;
    if actual != expected_header_len {
        return Err(IdentityControlCodecError::DeclaredLength {
            expected: expected_header_len,
            actual,
        });
    }
    Ok(())
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

    fn sample_failure() -> IdentityFailureV1 {
        IdentityFailureV1 {
            reason: IdentityFailureReason::UnreadableRoot,
            current_logical_id: Some(7),
            attempt_id: 42,
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
    fn identity_failure_round_trips_and_rejects_corruption() {
        let value = sample_failure();
        let wire = encode_identity_failure(&value).unwrap();
        assert_eq!(decode_identity_failure(&wire).unwrap(), value);
        assert_eq!(&wire[0..4], IDENTITY_FAILURE_MAGIC);

        let mut corrupt = wire.clone();
        corrupt[84] ^= 0xff;
        assert_eq!(
            decode_identity_failure(&corrupt),
            Err(IdentityControlCodecError::SuffixHashMismatch)
        );

        assert!(matches!(
            decode_identity_failure(&wire[..IDENTITY_FAILURE_FIXED_LEN - 1]),
            Err(IdentityControlCodecError::InvalidLength { .. })
        ));
    }

    #[test]
    fn identity_failure_ack_round_trips_and_enforces_coherence() {
        let ack = IdentityFailureAckV1 {
            attempt_id: 42,
            logical_id: Some(7),
            disposition: IdentityFailureAckDisposition::DurablyRecorded,
            error: 0,
        };
        let wire = encode_identity_failure_ack(ack).unwrap();
        assert_eq!(decode_identity_failure_ack(&wire).unwrap(), ack);
        assert_eq!(&wire[0..4], IDENTITY_FAILURE_ACK_MAGIC);

        let incoherent = IdentityFailureAckV1 { error: 5, ..ack };
        assert!(matches!(
            encode_identity_failure_ack(incoherent),
            Err(IdentityControlCodecError::InvalidField(_))
        ));
    }
}
