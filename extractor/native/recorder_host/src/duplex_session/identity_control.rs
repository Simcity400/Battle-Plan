//! Closed, activation-disabled codecs for campaign-identity H4CT control.
//!
//! The byte codecs remain activation-neutral.  The dormant recorder dispatcher
//! below adds only durable-before-ack barriers; it does not create pipes,
//! authorize live activation, or mint a stateful CID1 proof.

use std::fmt;

use snapshot::sha256::Sha256;

pub use compact_contract::first_compact_ack::{
    decode_first_compact_frame_ack, encode_first_compact_frame_ack, FirstCompactAckCodecError,
    FirstCompactFrameAckV1, FIRST_COMPACT_FRAME_ACK_LEN, FIRST_COMPACT_FRAME_ACK_MAGIC,
};
use compact_contract::stream_validator::{
    AcceptedRecordKind, StatefulStreamValidator, StreamIdentityMode, StreamValidationError,
};

use crate::control_transport::{
    ControlCancellation, ControlDeadline, ControlFrame, DeadlineControlChannel, DeadlineControlIo,
    MessageKind,
};

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

impl IdentityFailureReason {
    fn from_wire(value: u16) -> Result<Self, IdentityControlCodecError> {
        match value {
            1 => Ok(Self::UnreadableRoot),
            2 => Ok(Self::TornRead),
            3 => Ok(Self::UuidMissing),
            4 => Ok(Self::SeedMissing),
            5 => Ok(Self::UuidMalformed),
            6 => Ok(Self::SeedMalformed),
            7 => Ok(Self::InvalidHour),
            8 => Ok(Self::InvalidProfile),
            9 => Ok(Self::StabilityIncompleteOrMismatch),
            10 => Ok(Self::ComponentTransitionConflict),
            11 => Ok(Self::Cid1EncodeOrHashFailure),
            other => Err(IdentityControlCodecError::UnknownReason(other)),
        }
    }
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
            if compact_contract::calendar::hour_index_from_raw(hour).is_none() {
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
    out[12..14].copy_from_slice(&(value.reason as u16).to_le_bytes());
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

impl IdentityFailureAckDisposition {
    fn from_wire(value: u16) -> Result<Self, IdentityControlCodecError> {
        match value {
            1 => Ok(Self::DurablyRecorded),
            2 => Ok(Self::Rejected),
            other => Err(IdentityControlCodecError::UnknownDisposition(other)),
        }
    }
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
    out[24..26].copy_from_slice(&(value.disposition as u16).to_le_bytes());
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidatedFirstCompactRecordError {
    Framing(String),
    LengthMismatch {
        declared: usize,
        actual: usize,
    },
    Stream(StreamValidationError),
    WrongAcceptedKind {
        expected: AcceptedRecordKind,
        actual: AcceptedRecordKind,
    },
    MissingValidatedCid1,
    Cfa1(FirstCompactAckCodecError),
}

impl fmt::Display for ValidatedFirstCompactRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Framing(error) => write!(f, "first H4CR framing failed: {error}"),
            Self::LengthMismatch { declared, actual } => write!(
                f,
                "first H4CR declared length {declared} does not equal exact bytes {actual}"
            ),
            Self::Stream(error) => write!(f, "first H4CR stream validation failed: {error}"),
            Self::WrongAcceptedKind { expected, actual } => write!(
                f,
                "first H4CR kind {actual:?} did not equal explicit mode kind {expected:?}"
            ),
            Self::MissingValidatedCid1 => write!(f, "IdentityV1 first record produced no CID1"),
            Self::Cfa1(error) => write!(f, "first H4CR could not form CFA1: {error}"),
        }
    }
}

/// Exact first-record authority.  IdentityV1 and Legacy are selected only by
/// separate constructors; neither constructor sniffs or falls back to the
/// other protocol. Exact complete bytes are retained for durable admission.
#[derive(Clone, Debug)]
pub struct ValidatedFirstCompactRecord {
    mode: StreamIdentityMode,
    complete_record: Vec<u8>,
    validated_cid1: Option<compact_contract::campaign_identity::ValidatedCid1>,
    acknowledgement: FirstCompactFrameAckV1,
}

impl ValidatedFirstCompactRecord {
    pub fn identity_v1(
        logical_stream_id: u64,
        complete_record: &[u8],
    ) -> Result<Self, ValidatedFirstCompactRecordError> {
        Self::validate_explicit(
            StreamIdentityMode::IdentityV1,
            logical_stream_id,
            complete_record,
        )
    }

    pub fn legacy(
        logical_stream_id: u64,
        complete_record: &[u8],
    ) -> Result<Self, ValidatedFirstCompactRecordError> {
        Self::validate_explicit(
            StreamIdentityMode::Legacy,
            logical_stream_id,
            complete_record,
        )
    }

    fn validate_explicit(
        mode: StreamIdentityMode,
        logical_stream_id: u64,
        complete_record: &[u8],
    ) -> Result<Self, ValidatedFirstCompactRecordError> {
        let header = compact_contract::parse_compact_record_header(complete_record)
            .map_err(|error| ValidatedFirstCompactRecordError::Framing(error.to_string()))?;
        if header.record_len() != complete_record.len() {
            return Err(ValidatedFirstCompactRecordError::LengthMismatch {
                declared: header.record_len(),
                actual: complete_record.len(),
            });
        }
        let payload = &complete_record[compact_contract::COMPACT_RECORD_HEADER_LEN..];
        let mut validator = match mode {
            StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
            StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
        };
        let actual = validator
            .validate_record(&header, payload)
            .map_err(ValidatedFirstCompactRecordError::Stream)?;
        let expected = match mode {
            StreamIdentityMode::IdentityV1 => AcceptedRecordKind::CampaignIdentity,
            StreamIdentityMode::Legacy => AcceptedRecordKind::Ordinary,
        };
        if actual != expected {
            return Err(ValidatedFirstCompactRecordError::WrongAcceptedKind { expected, actual });
        }
        let validated_cid1 = validator.identity().cloned();
        if mode == StreamIdentityMode::IdentityV1 && validated_cid1.is_none() {
            return Err(ValidatedFirstCompactRecordError::MissingValidatedCid1);
        }
        if mode == StreamIdentityMode::Legacy && validated_cid1.is_some() {
            return Err(ValidatedFirstCompactRecordError::WrongAcceptedKind { expected, actual });
        }
        let complete_record_len = u32::try_from(complete_record.len()).map_err(|_| {
            ValidatedFirstCompactRecordError::LengthMismatch {
                declared: header.record_len(),
                actual: complete_record.len(),
            }
        })?;
        let acknowledgement = FirstCompactFrameAckV1 {
            mode,
            logical_stream_id,
            h4cr_sequence: header.sequence,
            game_hour_raw: header.game_hour_raw,
            domain_id: header.domain_id,
            record_type: header.record_type,
            complete_record_len,
            complete_record_sha256: Sha256::digest(complete_record),
            payload_sha256: Sha256::digest(payload),
        }
        .validate()
        .map_err(ValidatedFirstCompactRecordError::Cfa1)?;
        Ok(Self {
            mode,
            complete_record: complete_record.to_vec(),
            validated_cid1,
            acknowledgement,
        })
    }

    pub fn mode(&self) -> StreamIdentityMode {
        self.mode
    }

    pub fn complete_record(&self) -> &[u8] {
        &self.complete_record
    }

    pub fn validated_cid1(&self) -> Option<&compact_contract::campaign_identity::ValidatedCid1> {
        self.validated_cid1.as_ref()
    }

    pub fn acknowledgement(&self) -> FirstCompactFrameAckV1 {
        self.acknowledgement
    }
}

/// Candidate writer implemented by product storage.  It cannot mint a durable
/// receipt: the adapter below performs write -> flush -> sync under one exact
/// deadline/cancellation authority and mints the receipt only afterwards.
pub trait IdentityDurableCandidateSink {
    type FirstRecordCandidate;
    type IdentityFailureCandidate;
    type IdentityFailureGapLedgerCandidate;

    fn write_first_record_candidate(
        &mut self,
        first_record: &ValidatedFirstCompactRecord,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<Self::FirstRecordCandidate, String>;

    fn flush_first_record_candidate(
        &mut self,
        candidate: &mut Self::FirstRecordCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String>;

    fn sync_first_record_candidate(
        &mut self,
        candidate: &mut Self::FirstRecordCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String>;

    fn write_identity_failure_candidate(
        &mut self,
        failure: &IdentityFailureV1,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<Self::IdentityFailureCandidate, String>;

    fn flush_identity_failure_candidate(
        &mut self,
        candidate: &mut Self::IdentityFailureCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String>;

    fn sync_identity_failure_candidate(
        &mut self,
        candidate: &mut Self::IdentityFailureCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String>;

    /// Materialize the capture failure into the durable manifest/gap ledger.
    /// The exact CIF1 WAL candidate above is forensic evidence, but it cannot
    /// authorize CIA1 disposition 1 by itself.
    fn write_identity_failure_gap_ledger_candidate(
        &mut self,
        failure: &IdentityFailureV1,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<Self::IdentityFailureGapLedgerCandidate, String>;

    fn flush_identity_failure_gap_ledger_candidate(
        &mut self,
        candidate: &mut Self::IdentityFailureGapLedgerCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String>;

    fn sync_identity_failure_gap_ledger_candidate(
        &mut self,
        candidate: &mut Self::IdentityFailureGapLedgerCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String>;
}

#[derive(Clone, Debug)]
pub struct DurableFirstRecordReceipt {
    acknowledgement: FirstCompactFrameAckV1,
}

impl DurableFirstRecordReceipt {
    pub fn acknowledgement(&self) -> FirstCompactFrameAckV1 {
        self.acknowledgement
    }
}

#[derive(Clone, Debug)]
pub struct DurableIdentityFailureReceipt {
    attempt_id: u64,
    logical_stream_id: Option<u64>,
    gap_ledger: DurableIdentityFailureGapLedgerReceipt,
}

/// Opaque proof minted by `IdentityControlJournal` only after the sink's
/// manifest/gap-ledger candidate completed write -> flush -> sync.
#[derive(Clone, Debug)]
pub struct DurableIdentityFailureGapLedgerReceipt {
    attempt_id: u64,
}

impl DurableIdentityFailureReceipt {
    pub fn attempt_id(&self) -> u64 {
        self.attempt_id
    }

    pub fn logical_stream_id(&self) -> Option<u64> {
        self.logical_stream_id
    }

    pub fn gap_ledger_receipt(&self) -> &DurableIdentityFailureGapLedgerReceipt {
        &self.gap_ledger
    }
}

impl DurableIdentityFailureGapLedgerReceipt {
    pub fn attempt_id(&self) -> u64 {
        self.attempt_id
    }
}

pub struct IdentityControlJournal<S> {
    sink: S,
    poisoned: bool,
}

impl<S: IdentityDurableCandidateSink> IdentityControlJournal<S> {
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            poisoned: false,
        }
    }

    pub fn durable_first_record_admission(
        &mut self,
        first_record: &ValidatedFirstCompactRecord,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<DurableFirstRecordReceipt, String> {
        self.run_preflight(deadline, cancellation)?;
        let mut candidate =
            match self
                .sink
                .write_first_record_candidate(first_record, deadline, cancellation)
            {
                Ok(candidate) => candidate,
                Err(error) => return Err(self.fail(error)),
            };
        self.run_preflight(deadline, cancellation)?;
        if let Err(error) =
            self.sink
                .flush_first_record_candidate(&mut candidate, deadline, cancellation)
        {
            return Err(self.fail(error));
        }
        self.run_preflight(deadline, cancellation)?;
        if let Err(error) =
            self.sink
                .sync_first_record_candidate(&mut candidate, deadline, cancellation)
        {
            return Err(self.fail(error));
        }
        self.run_preflight(deadline, cancellation)?;
        Ok(DurableFirstRecordReceipt {
            acknowledgement: first_record.acknowledgement(),
        })
    }

    pub fn durable_identity_failure(
        &mut self,
        failure: &IdentityFailureV1,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<DurableIdentityFailureReceipt, String> {
        self.run_preflight(deadline, cancellation)?;
        let mut candidate =
            match self
                .sink
                .write_identity_failure_candidate(failure, deadline, cancellation)
            {
                Ok(candidate) => candidate,
                Err(error) => return Err(self.fail(error)),
            };
        self.run_preflight(deadline, cancellation)?;
        if let Err(error) =
            self.sink
                .flush_identity_failure_candidate(&mut candidate, deadline, cancellation)
        {
            return Err(self.fail(error));
        }
        self.run_preflight(deadline, cancellation)?;
        if let Err(error) =
            self.sink
                .sync_identity_failure_candidate(&mut candidate, deadline, cancellation)
        {
            return Err(self.fail(error));
        }
        self.run_preflight(deadline, cancellation)?;
        let mut gap_ledger = match self.sink.write_identity_failure_gap_ledger_candidate(
            failure,
            deadline,
            cancellation,
        ) {
            Ok(candidate) => candidate,
            Err(error) => return Err(self.fail(error)),
        };
        self.run_preflight(deadline, cancellation)?;
        if let Err(error) = self.sink.flush_identity_failure_gap_ledger_candidate(
            &mut gap_ledger,
            deadline,
            cancellation,
        ) {
            return Err(self.fail(error));
        }
        self.run_preflight(deadline, cancellation)?;
        if let Err(error) = self.sink.sync_identity_failure_gap_ledger_candidate(
            &mut gap_ledger,
            deadline,
            cancellation,
        ) {
            return Err(self.fail(error));
        }
        self.run_preflight(deadline, cancellation)?;
        Ok(DurableIdentityFailureReceipt {
            attempt_id: failure.attempt_id,
            logical_stream_id: failure.current_logical_id,
            gap_ledger: DurableIdentityFailureGapLedgerReceipt {
                attempt_id: failure.attempt_id,
            },
        })
    }

    fn run_preflight(
        &mut self,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String> {
        if self.poisoned {
            Err("identity control journal is poisoned".to_string())
        } else if cancellation.is_cancelled() {
            Err(self.fail("identity control journal cancelled".to_string()))
        } else if deadline.is_expired() {
            Err(self.fail("identity control journal deadline expired".to_string()))
        } else {
            Ok(())
        }
    }

    fn fail(&mut self, error: String) -> String {
        self.poisoned = true;
        error
    }
}

pub struct AcknowledgedFirstCompactRecord {
    first_record: ValidatedFirstCompactRecord,
    durable_receipt: DurableFirstRecordReceipt,
}

impl AcknowledgedFirstCompactRecord {
    pub fn first_record(&self) -> &ValidatedFirstCompactRecord {
        &self.first_record
    }

    pub fn durable_receipt(&self) -> &DurableFirstRecordReceipt {
        &self.durable_receipt
    }
}

pub struct AcknowledgedIdentityFailure {
    failure: IdentityFailureV1,
    durable_receipt: DurableIdentityFailureReceipt,
}

impl AcknowledgedIdentityFailure {
    pub fn failure(&self) -> &IdentityFailureV1 {
        &self.failure
    }

    pub fn durable_receipt(&self) -> &DurableIdentityFailureReceipt {
        &self.durable_receipt
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityDispatchError {
    Poisoned,
    Codec(IdentityControlCodecError),
    Cfa1(FirstCompactAckCodecError),
    UnexpectedKind(MessageKind),
    FirstRecordAlreadyAcknowledged,
    FailureTupleCrosswire,
    FailureAttemptNotIncreasing,
    Journal(String),
    Transport(String),
}

impl fmt::Display for IdentityDispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Poisoned => write!(f, "identity control dispatcher is poisoned"),
            Self::Codec(error) => write!(f, "identity control payload invalid: {error}"),
            Self::Cfa1(error) => write!(f, "first-record acknowledgement invalid: {error}"),
            Self::UnexpectedKind(kind) => {
                write!(f, "identity control dispatcher received {kind:?}")
            }
            Self::FirstRecordAlreadyAcknowledged => {
                write!(f, "first compact record was already acknowledged")
            }
            Self::FailureTupleCrosswire => {
                write!(f, "identity failure did not match the bound logical stream")
            }
            Self::FailureAttemptNotIncreasing => {
                write!(f, "identity failure attempt id did not strictly increase")
            }
            Self::Journal(error) => write!(f, "identity control journal failed: {error}"),
            Self::Transport(error) => write!(f, "identity control transport failed: {error}"),
        }
    }
}

impl std::error::Error for IdentityDispatchError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoundLogicalStream {
    logical_stream_id: u64,
    cid1_payload_sha256: Option<[u8; 32]>,
}

/// Recorder-side CFA1/CIF1/CIA1 dispatch barrier.  It binds all messages to one
/// logical-stream tuple and permanently poisons itself together with H4CT on
/// any codec, journal, crosswire, capability, or I/O failure.
pub struct RecorderIdentityControlDispatcher<S> {
    journal: IdentityControlJournal<S>,
    bound: Option<BoundLogicalStream>,
    last_failure_attempt: Option<u64>,
    poisoned: bool,
}

impl<S: IdentityDurableCandidateSink> RecorderIdentityControlDispatcher<S> {
    pub fn new(sink: S) -> Self {
        Self {
            journal: IdentityControlJournal::new(sink),
            bound: None,
            last_failure_attempt: None,
            poisoned: false,
        }
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    pub fn bound_logical_stream_id(&self) -> Option<u64> {
        self.bound.map(|bound| bound.logical_stream_id)
    }

    /// Journal the exact first-record tuple before emitting its canonical CFA1.
    /// A journal-success/write-failure ambiguity is terminal and never retried.
    pub fn acknowledge_first_record<I: DeadlineControlIo>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        first_record: ValidatedFirstCompactRecord,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<AcknowledgedFirstCompactRecord, IdentityDispatchError> {
        self.preflight(channel)?;
        self.preflight_operation(channel, deadline, cancellation)?;
        if let Err(error) = channel.assert_capability(MessageKind::FirstCompactFrameAck) {
            self.poison(channel);
            return Err(IdentityDispatchError::Transport(error.to_string()));
        }
        let acknowledgement = first_record.acknowledgement();
        if self.bound.is_some() {
            self.poison(channel);
            return Err(IdentityDispatchError::FirstRecordAlreadyAcknowledged);
        }
        let durable_receipt =
            match self
                .journal
                .durable_first_record_admission(&first_record, deadline, cancellation)
            {
                Ok(receipt) => receipt,
                Err(error) => {
                    self.poison(channel);
                    return Err(IdentityDispatchError::Journal(error));
                }
            };
        let payload = encode_first_compact_frame_ack(acknowledgement).map_err(|error| {
            self.poison(channel);
            IdentityDispatchError::Cfa1(error)
        })?;
        if let Err(error) = channel.send_until(
            MessageKind::FirstCompactFrameAck,
            &payload,
            deadline,
            cancellation,
        ) {
            self.poison(channel);
            return Err(IdentityDispatchError::Transport(error.to_string()));
        }
        self.bound = Some(BoundLogicalStream {
            logical_stream_id: acknowledgement.logical_stream_id,
            cid1_payload_sha256: (acknowledgement.mode == StreamIdentityMode::IdentityV1)
                .then_some(acknowledgement.payload_sha256),
        });
        Ok(AcknowledgedFirstCompactRecord {
            first_record,
            durable_receipt,
        })
    }

    /// Decode an exact collector-originated CIF1, durably append its raw
    /// evidence, then and only then emit a matching CIA1 disposition 1.
    pub(crate) fn receive_and_dispatch_identity_failure<I: DeadlineControlIo>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<AcknowledgedIdentityFailure, IdentityDispatchError> {
        self.preflight(channel)?;
        self.preflight_operation(channel, deadline, cancellation)?;
        let frame = match channel.recv_until(deadline, cancellation) {
            Ok(frame) => frame,
            Err(error) => {
                self.poison(channel);
                return Err(IdentityDispatchError::Transport(error.to_string()));
            }
        };
        self.dispatch_identity_failure_frame(channel, frame, deadline, cancellation)
    }

    /// Dispatch a CIF1 frame already received by a central H4CT router. This
    /// prevents a heartbeat receive loop from consuming an asynchronous
    /// identity failure as a wrong-kind terminal error while preserving the
    /// same channel sequence, capability, durability, deadline, and poison
    /// authority as the direct receive path.
    pub(crate) fn dispatch_identity_failure_frame<I: DeadlineControlIo>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        frame: ControlFrame,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<AcknowledgedIdentityFailure, IdentityDispatchError> {
        self.preflight(channel)?;
        self.preflight_operation(channel, deadline, cancellation)?;
        if let Err(error) = channel.assert_capability(MessageKind::IdentityFailure) {
            self.poison(channel);
            return Err(IdentityDispatchError::Transport(error.to_string()));
        }
        if frame.kind != MessageKind::IdentityFailure {
            self.poison(channel);
            return Err(IdentityDispatchError::UnexpectedKind(frame.kind));
        }
        let failure = decode_identity_failure(&frame.payload).map_err(|error| {
            self.poison(channel);
            IdentityDispatchError::Codec(error)
        })?;
        if self
            .last_failure_attempt
            .is_some_and(|last| failure.attempt_id <= last)
        {
            self.poison(channel);
            return Err(IdentityDispatchError::FailureAttemptNotIncreasing);
        }
        let tuple_matches = match self.bound {
            None => failure.current_logical_id.is_none() && failure.current_cid1_hash.is_none(),
            Some(bound) => {
                failure.current_logical_id == Some(bound.logical_stream_id)
                    && failure.current_cid1_hash == bound.cid1_payload_sha256
            }
        };
        if !tuple_matches {
            self.poison(channel);
            return Err(IdentityDispatchError::FailureTupleCrosswire);
        }
        let durable_receipt =
            match self
                .journal
                .durable_identity_failure(&failure, deadline, cancellation)
            {
                Ok(receipt) => receipt,
                Err(error) => {
                    self.poison(channel);
                    return Err(IdentityDispatchError::Journal(error));
                }
            };
        let acknowledgement = IdentityFailureAckV1 {
            attempt_id: failure.attempt_id,
            logical_id: failure.current_logical_id,
            disposition: IdentityFailureAckDisposition::DurablyRecorded,
            error: 0,
        };
        let payload = encode_identity_failure_ack(acknowledgement).map_err(|error| {
            self.poison(channel);
            IdentityDispatchError::Codec(error)
        })?;
        if let Err(error) = channel.send_until(
            MessageKind::IdentityFailureAck,
            &payload,
            deadline,
            cancellation,
        ) {
            self.poison(channel);
            return Err(IdentityDispatchError::Transport(error.to_string()));
        }
        self.last_failure_attempt = Some(failure.attempt_id);
        Ok(AcknowledgedIdentityFailure {
            failure,
            durable_receipt,
        })
    }

    fn preflight<I: DeadlineControlIo>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
    ) -> Result<(), IdentityDispatchError> {
        if self.poisoned || channel.is_poisoned() {
            self.poison(channel);
            Err(IdentityDispatchError::Poisoned)
        } else {
            Ok(())
        }
    }

    fn preflight_operation<I: DeadlineControlIo>(
        &mut self,
        channel: &mut DeadlineControlChannel<I>,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), IdentityDispatchError> {
        if cancellation.is_cancelled() {
            self.poison(channel);
            Err(IdentityDispatchError::Transport(
                "identity control operation cancelled".to_string(),
            ))
        } else if deadline.is_expired() {
            self.poison(channel);
            Err(IdentityDispatchError::Transport(
                "identity control operation deadline expired".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    fn poison<I: DeadlineControlIo>(&mut self, channel: &mut DeadlineControlChannel<I>) {
        self.poisoned = true;
        channel.poison();
    }
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
    use std::cell::RefCell;
    use std::io;
    use std::rc::Rc;

    use crate::control_transport::{
        claim_session_capability, decode_frame, ControlCapabilities, ControlPeerIdentity,
        EndpointRole, SessionId, ValidatedSessionCapability,
    };

    #[derive(Default)]
    struct DispatchIo {
        events: Rc<RefCell<Vec<&'static str>>>,
        inbound: std::collections::VecDeque<u8>,
        outbound: Vec<u8>,
    }

    impl DeadlineControlIo for DispatchIo {
        fn send_all_until(
            &mut self,
            bytes: &[u8],
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            self.events.borrow_mut().push("send");
            self.outbound.extend_from_slice(bytes);
            Ok(())
        }

        fn recv_exact_until(
            &mut self,
            bytes: &mut [u8],
            _deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            if cancellation.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            for byte in bytes {
                *byte = self
                    .inbound
                    .pop_front()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "mock EOF"))?;
            }
            Ok(())
        }
    }

    struct DispatchSink {
        events: Rc<RefCell<Vec<&'static str>>>,
        fail: bool,
    }

    impl IdentityDurableCandidateSink for DispatchSink {
        type FirstRecordCandidate = ();
        type IdentityFailureCandidate = ();
        type IdentityFailureGapLedgerCandidate = ();

        fn write_first_record_candidate(
            &mut self,
            _first_record: &ValidatedFirstCompactRecord,
            _deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> Result<Self::FirstRecordCandidate, String> {
            if cancellation.is_cancelled() {
                return Err("cancelled before first journal".to_string());
            }
            self.events.borrow_mut().push("write-first");
            if self.fail {
                Err("first journal fault".to_string())
            } else {
                Ok(())
            }
        }

        fn flush_first_record_candidate(
            &mut self,
            _candidate: &mut Self::FirstRecordCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            self.events.borrow_mut().push("flush-first");
            Ok(())
        }

        fn sync_first_record_candidate(
            &mut self,
            _candidate: &mut Self::FirstRecordCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            self.events.borrow_mut().push("sync-first");
            Ok(())
        }

        fn write_identity_failure_candidate(
            &mut self,
            _failure: &IdentityFailureV1,
            _deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> Result<Self::IdentityFailureCandidate, String> {
            if cancellation.is_cancelled() {
                return Err("cancelled before failure journal".to_string());
            }
            self.events.borrow_mut().push("write-failure");
            if self.fail {
                Err("failure journal fault".to_string())
            } else {
                Ok(())
            }
        }

        fn flush_identity_failure_candidate(
            &mut self,
            _candidate: &mut Self::IdentityFailureCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            self.events.borrow_mut().push("flush-failure");
            Ok(())
        }

        fn sync_identity_failure_candidate(
            &mut self,
            _candidate: &mut Self::IdentityFailureCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            self.events.borrow_mut().push("sync-failure");
            Ok(())
        }

        fn write_identity_failure_gap_ledger_candidate(
            &mut self,
            _failure: &IdentityFailureV1,
            _deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> Result<Self::IdentityFailureGapLedgerCandidate, String> {
            if cancellation.is_cancelled() {
                return Err("cancelled before failure gap ledger".to_string());
            }
            self.events.borrow_mut().push("write-gap-ledger");
            Ok(())
        }

        fn flush_identity_failure_gap_ledger_candidate(
            &mut self,
            _candidate: &mut Self::IdentityFailureGapLedgerCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            self.events.borrow_mut().push("flush-gap-ledger");
            Ok(())
        }

        fn sync_identity_failure_gap_ledger_candidate(
            &mut self,
            _candidate: &mut Self::IdentityFailureGapLedgerCandidate,
            _deadline: ControlDeadline,
            _cancellation: &ControlCancellation,
        ) -> Result<(), String> {
            self.events.borrow_mut().push("sync-gap-ledger");
            Ok(())
        }
    }

    fn dispatch_channel(
        byte: u8,
        capabilities: u32,
        events: Rc<RefCell<Vec<&'static str>>>,
        inbound: Vec<u8>,
    ) -> (DeadlineControlChannel<DispatchIo>, SessionId) {
        let bytes = [byte; 16];
        let session_id = SessionId::from_bytes(bytes).unwrap();
        let capability = claim_session_capability(bytes).unwrap();
        let peer = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9001,
        };
        let capabilities = ControlCapabilities::from_bits(capabilities).unwrap();
        let admission =
            ValidatedSessionCapability::admit(capability, peer, peer, capabilities, capabilities)
                .unwrap();
        (
            DeadlineControlChannel::new(
                DispatchIo {
                    events,
                    inbound: inbound.into(),
                    outbound: Vec::new(),
                },
                EndpointRole::Recorder,
                admission,
            ),
            session_id,
        )
    }

    fn hour() -> i32 {
        compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24
    }

    fn legacy_ack() -> FirstCompactFrameAckV1 {
        FirstCompactFrameAckV1 {
            mode: StreamIdentityMode::Legacy,
            logical_stream_id: 0,
            h4cr_sequence: 1,
            game_hour_raw: hour(),
            domain_id: 10,
            record_type: 1,
            complete_record_len: 72,
            complete_record_sha256: [1; 32],
            payload_sha256: [2; 32],
        }
    }

    fn complete_h4cr(
        sequence: u64,
        domain_id: u16,
        record_type: u16,
        game_hour_raw: i32,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut record = vec![0u8; compact_contract::COMPACT_RECORD_HEADER_LEN];
        record[0..4].copy_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
        record[4..6].copy_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        record[6..8]
            .copy_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        record[8..16].copy_from_slice(&sequence.to_le_bytes());
        record[16..20].copy_from_slice(&game_hour_raw.to_le_bytes());
        record[20..22].copy_from_slice(&domain_id.to_le_bytes());
        record[22..24].copy_from_slice(&record_type.to_le_bytes());
        record[24..28].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        record.extend_from_slice(payload);
        record
    }

    fn validated_legacy(logical_stream_id: u64) -> ValidatedFirstCompactRecord {
        let payload = [0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
        ValidatedFirstCompactRecord::legacy(
            logical_stream_id,
            &complete_h4cr(
                1,
                10,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                hour(),
                &payload,
            ),
        )
        .unwrap()
    }

    fn validated_identity(logical_stream_id: u64) -> ValidatedFirstCompactRecord {
        use compact_contract::campaign_identity::{
            encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses,
        };
        let observation = Cid1Observation {
            game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".to_string(),
            game_unique_seed: -1234,
            game_hour_raw: hour(),
        };
        let fields = Cid1Fields::from_stable_observations(
            observation.clone(),
            observation,
            "profile-x".to_string(),
            Cid1Witnesses::default(),
        )
        .unwrap();
        let payload = encode_cid1(&fields).unwrap();
        ValidatedFirstCompactRecord::identity_v1(
            logical_stream_id,
            &complete_h4cr(
                0,
                0,
                compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER,
                hour(),
                &payload,
            ),
        )
        .unwrap()
    }

    #[test]
    fn canonical_ack_roundtrips_legacy_and_identity_first_records() {
        let legacy = legacy_ack();
        assert_eq!(
            decode_first_compact_frame_ack(
                &encode_first_compact_frame_ack(legacy).unwrap(),
                StreamIdentityMode::Legacy,
            )
            .unwrap(),
            legacy
        );
        let identity = validated_identity(4).acknowledgement();
        assert_eq!(
            decode_first_compact_frame_ack(
                &encode_first_compact_frame_ack(identity).unwrap(),
                StreamIdentityMode::IdentityV1,
            )
            .unwrap(),
            identity
        );
        assert!(decode_first_compact_frame_ack(
            &encode_first_compact_frame_ack(identity).unwrap(),
            StreamIdentityMode::Legacy,
        )
        .is_err());
    }

    #[test]
    fn ack_rejects_every_wrong_first_record_mode_and_fixed_field() {
        for value in [
            FirstCompactFrameAckV1 {
                mode: StreamIdentityMode::Legacy,
                h4cr_sequence: 0,
                domain_id: 10,
                record_type: 1,
                ..legacy_ack()
            },
            FirstCompactFrameAckV1 {
                mode: StreamIdentityMode::IdentityV1,
                h4cr_sequence: 1,
                domain_id: 0,
                record_type: compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER,
                ..legacy_ack()
            },
            FirstCompactFrameAckV1 {
                h4cr_sequence: 2,
                ..legacy_ack()
            },
            FirstCompactFrameAckV1 {
                record_type: 99,
                ..legacy_ack()
            },
        ] {
            assert!(encode_first_compact_frame_ack(value).is_err());
        }
        let mut wire = encode_first_compact_frame_ack(legacy_ack()).unwrap();
        wire[36] = 1;
        assert_eq!(
            decode_first_compact_frame_ack(&wire, StreamIdentityMode::Legacy),
            Err(FirstCompactAckCodecError::NonZeroReserved)
        );
        assert!(decode_first_compact_frame_ack(
            &wire[..wire.len() - 1],
            StreamIdentityMode::Legacy,
        )
        .is_err());
    }

    fn failure() -> IdentityFailureV1 {
        IdentityFailureV1 {
            reason: IdentityFailureReason::StabilityIncompleteOrMismatch,
            current_logical_id: Some(2),
            attempt_id: 7,
            attempted_hour: Some(hour()),
            uuid_raw: Some(b"bad-uuid".to_vec()),
            seed_raw: Some(b"seed".to_vec()),
            profile_raw: Some(b"profile-x".to_vec()),
            first_read_complete: true,
            second_read_complete: false,
            reads_equal: false,
            last_admitted_sequence: Some(91),
            current_cid1_hash: Some([3; 32]),
        }
    }

    #[test]
    fn identity_failure_roundtrips_exact_raw_suffix_and_hash() {
        let value = failure();
        let wire = encode_identity_failure(&value).unwrap();
        assert_eq!(read_u32(&wire, 8) as usize, wire.len());
        assert_eq!(decode_identity_failure(&wire).unwrap(), value);
    }

    #[test]
    fn identity_failure_rejects_flag_length_hash_and_evidence_incoherence() {
        let wire = encode_identity_failure(&failure()).unwrap();
        let mut unknown = wire.clone();
        unknown[14..16].copy_from_slice(&(1u16 << 15).to_le_bytes());
        assert!(matches!(
            decode_identity_failure(&unknown),
            Err(IdentityControlCodecError::UnknownFlags(_))
        ));
        let mut bad_hash = wire.clone();
        *bad_hash.last_mut().unwrap() ^= 1;
        assert_eq!(
            decode_identity_failure(&bad_hash),
            Err(IdentityControlCodecError::SuffixHashMismatch)
        );
        let mut empty_present = failure();
        empty_present.uuid_raw = Some(Vec::new());
        assert!(encode_identity_failure(&empty_present).is_err());
        let mut impossible_equal = failure();
        impossible_equal.reads_equal = true;
        assert!(encode_identity_failure(&impossible_equal).is_err());
    }

    #[test]
    fn identity_failure_ack_roundtrip_and_disposition_error_coherence() {
        let recorded = IdentityFailureAckV1 {
            attempt_id: 9,
            logical_id: None,
            disposition: IdentityFailureAckDisposition::DurablyRecorded,
            error: 0,
        };
        assert_eq!(
            decode_identity_failure_ack(&encode_identity_failure_ack(recorded).unwrap()).unwrap(),
            recorded
        );
        for invalid in [
            IdentityFailureAckV1 {
                error: 1,
                ..recorded
            },
            IdentityFailureAckV1 {
                disposition: IdentityFailureAckDisposition::Rejected,
                error: 0,
                ..recorded
            },
        ] {
            assert!(encode_identity_failure_ack(invalid).is_err());
        }
    }

    #[test]
    fn arbitrary_short_payloads_never_panic() {
        for len in 0..FIRST_COMPACT_FRAME_ACK_LEN {
            assert!(decode_first_compact_frame_ack(
                &vec![0xA5; len],
                StreamIdentityMode::IdentityV1,
            )
            .is_err());
        }
        for len in 0..IDENTITY_FAILURE_FIXED_LEN {
            assert!(decode_identity_failure(&vec![0xA5; len]).is_err());
        }
        for len in 0..IDENTITY_FAILURE_ACK_LEN {
            assert!(decode_identity_failure_ack(&vec![0xA5; len]).is_err());
        }
    }

    #[test]
    fn cfa1_is_derived_from_exact_h4cr_and_journalled_before_wire_ack() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let (mut channel, session_id) = dispatch_channel(
            0xA1,
            ControlCapabilities::FIRST_COMPACT_ACK | ControlCapabilities::CAMPAIGN_IDENTITY,
            events.clone(),
            Vec::new(),
        );
        let mut dispatcher = RecorderIdentityControlDispatcher::new(DispatchSink {
            events: events.clone(),
            fail: false,
        });
        let proof = validated_identity(0);
        let expected = proof.acknowledgement();
        let acknowledged = dispatcher
            .acknowledge_first_record(
                &mut channel,
                proof,
                ControlDeadline::after(std::time::Duration::from_secs(1)).unwrap(),
                &ControlCancellation::new(),
            )
            .unwrap();
        assert_eq!(
            acknowledged.first_record().mode(),
            StreamIdentityMode::IdentityV1
        );
        assert_eq!(
            acknowledged
                .first_record()
                .validated_cid1()
                .unwrap()
                .payload(),
            &acknowledged.first_record().complete_record()
                [compact_contract::COMPACT_RECORD_HEADER_LEN..]
        );
        assert_eq!(acknowledged.durable_receipt().acknowledgement(), expected);
        assert_eq!(
            &*events.borrow(),
            &["write-first", "flush-first", "sync-first", "send"]
        );
        let frame = decode_frame(channel.io().outbound.as_slice(), session_id, 0).unwrap();
        assert_eq!(frame.kind, MessageKind::FirstCompactFrameAck);
        assert_eq!(
            decode_first_compact_frame_ack(&frame.payload, StreamIdentityMode::IdentityV1).unwrap(),
            expected
        );
        assert_eq!(dispatcher.bound_logical_stream_id(), Some(0));

        let mut forged = complete_h4cr(1, 10, 1, hour(), b"legacy");
        forged[24..28].copy_from_slice(&99u32.to_le_bytes());
        assert!(ValidatedFirstCompactRecord::legacy(0, &forged).is_err());
        let legacy_bytes = complete_h4cr(1, 10, 1, hour(), b"legacy");
        assert!(ValidatedFirstCompactRecord::identity_v1(0, &legacy_bytes).is_err());
        let identity_bytes = validated_identity(0).complete_record().to_vec();
        assert!(ValidatedFirstCompactRecord::legacy(0, &identity_bytes).is_err());
    }

    #[test]
    fn preflight_journal_or_capability_failure_emits_no_cfa1() {
        for (byte, capabilities, fail, cancelled, expired) in [
            (
                0xA2,
                ControlCapabilities::FIRST_COMPACT_ACK,
                true,
                false,
                false,
            ),
            (
                0xA3,
                ControlCapabilities::CAMPAIGN_IDENTITY,
                false,
                false,
                false,
            ),
            (
                0xA7,
                ControlCapabilities::FIRST_COMPACT_ACK,
                false,
                true,
                false,
            ),
            (
                0xA8,
                ControlCapabilities::FIRST_COMPACT_ACK,
                false,
                false,
                true,
            ),
        ] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let (mut channel, _) = dispatch_channel(byte, capabilities, events.clone(), Vec::new());
            let mut dispatcher = RecorderIdentityControlDispatcher::new(DispatchSink {
                events: events.clone(),
                fail,
            });
            let cancellation = ControlCancellation::new();
            if cancelled {
                cancellation.cancel();
            }
            let deadline = if expired {
                ControlDeadline::at(std::time::Instant::now())
            } else {
                ControlDeadline::after(std::time::Duration::from_secs(1)).unwrap()
            };
            assert!(dispatcher
                .acknowledge_first_record(
                    &mut channel,
                    validated_legacy(0),
                    deadline,
                    &cancellation,
                )
                .is_err());
            assert!(channel.io().outbound.is_empty());
            assert!(channel.is_poisoned());
            if fail {
                assert_eq!(&*events.borrow(), &["write-first"]);
            } else {
                assert!(events.borrow().is_empty());
            }
        }
    }

    #[test]
    fn cif1_is_channel_admitted_tuple_bound_and_journalled_before_cia1() {
        let identity = validated_identity(2);
        let identity_ack = identity.acknowledgement();
        let session_bytes = [0xA4; 16];
        let session_id = SessionId::from_bytes(session_bytes).unwrap();
        let mut value = failure();
        value.current_logical_id = Some(2);
        value.current_cid1_hash = Some(identity_ack.payload_sha256);
        let inbound = crate::control_transport::encode_frame(
            session_id,
            0,
            MessageKind::IdentityFailure,
            &encode_identity_failure(&value).unwrap(),
        )
        .unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        let (mut channel, _) = dispatch_channel(
            0xA4,
            ControlCapabilities::FIRST_COMPACT_ACK | ControlCapabilities::CAMPAIGN_IDENTITY,
            events.clone(),
            inbound,
        );
        let mut dispatcher = RecorderIdentityControlDispatcher::new(DispatchSink {
            events: events.clone(),
            fail: false,
        });
        let deadline = ControlDeadline::after(std::time::Duration::from_secs(1)).unwrap();
        dispatcher
            .acknowledge_first_record(
                &mut channel,
                identity,
                deadline,
                &ControlCancellation::new(),
            )
            .unwrap();
        let acknowledged_failure = dispatcher
            .receive_and_dispatch_identity_failure(
                &mut channel,
                deadline,
                &ControlCancellation::new(),
            )
            .unwrap();
        assert_eq!(acknowledged_failure.failure(), &value);
        assert_eq!(
            acknowledged_failure.durable_receipt().attempt_id(),
            value.attempt_id
        );
        assert_eq!(
            acknowledged_failure
                .durable_receipt()
                .gap_ledger_receipt()
                .attempt_id(),
            value.attempt_id
        );
        assert_eq!(
            &*events.borrow(),
            &[
                "write-first",
                "flush-first",
                "sync-first",
                "send",
                "write-failure",
                "flush-failure",
                "sync-failure",
                "write-gap-ledger",
                "flush-gap-ledger",
                "sync-gap-ledger",
                "send",
            ]
        );
        let first_wire_len =
            crate::control_transport::CONTROL_HEADER_LEN + FIRST_COMPACT_FRAME_ACK_LEN;
        let ack = decode_frame(&channel.io().outbound[first_wire_len..], session_id, 1).unwrap();
        assert_eq!(ack.kind, MessageKind::IdentityFailureAck);
        assert_eq!(
            decode_identity_failure_ack(&ack.payload).unwrap(),
            IdentityFailureAckV1 {
                attempt_id: value.attempt_id,
                logical_id: Some(2),
                disposition: IdentityFailureAckDisposition::DurablyRecorded,
                error: 0,
            }
        );
    }

    #[test]
    fn wrong_session_or_sequence_never_reaches_identity_journal() {
        for (byte, wire_session, wire_sequence) in [
            (0xA5, SessionId::from_bytes([0xE5; 16]).unwrap(), 0),
            (0xA6, SessionId::from_bytes([0xA6; 16]).unwrap(), 1),
        ] {
            let identity = validated_identity(2);
            let mut value = failure();
            value.current_logical_id = Some(2);
            value.current_cid1_hash = Some(identity.acknowledgement().payload_sha256);
            let inbound = crate::control_transport::encode_frame(
                wire_session,
                wire_sequence,
                MessageKind::IdentityFailure,
                &encode_identity_failure(&value).unwrap(),
            )
            .unwrap();
            let events = Rc::new(RefCell::new(Vec::new()));
            let (mut channel, _) = dispatch_channel(
                byte,
                ControlCapabilities::FIRST_COMPACT_ACK | ControlCapabilities::CAMPAIGN_IDENTITY,
                events.clone(),
                inbound,
            );
            let mut dispatcher = RecorderIdentityControlDispatcher::new(DispatchSink {
                events: events.clone(),
                fail: false,
            });
            let deadline = ControlDeadline::after(std::time::Duration::from_secs(1)).unwrap();
            dispatcher
                .acknowledge_first_record(
                    &mut channel,
                    identity,
                    deadline,
                    &ControlCancellation::new(),
                )
                .unwrap();
            assert!(dispatcher
                .receive_and_dispatch_identity_failure(
                    &mut channel,
                    deadline,
                    &ControlCancellation::new(),
                )
                .is_err());
            assert_eq!(
                events
                    .borrow()
                    .iter()
                    .filter(|event| **event == "write-failure")
                    .count(),
                0
            );
        }
    }
}
