use core::fmt;

pub const HOURLY_SERVICE_VERSION: u16 = 2;
pub const HOURLY_SERVICE_LIMIT_NS: u64 = 16_000_000;
pub const HOURLY_SERVICE_STAGE_COUNT: usize = 7;
pub const HOURLY_SERVICE_STAGE_MASK: u8 = (1 << HOURLY_SERVICE_STAGE_COUNT) - 1;
pub const HOURLY_SERVICE_KNOWN_FAULT_BITS: u32 = (1 << 17) - 1;
pub const HOURLY_SERVICE_MAX_BATCH_SAMPLES: usize = 16;
pub const HOURLY_SERVICE_SAMPLE_LEN: usize = 136;
pub const HOURLY_SERVICE_BATCH_HEADER_LEN: usize = 24;
pub const HOURLY_SERVICE_MAX_BATCH_LEN: usize =
    HOURLY_SERVICE_BATCH_HEADER_LEN + HOURLY_SERVICE_MAX_BATCH_SAMPLES * HOURLY_SERVICE_SAMPLE_LEN;
pub const HOURLY_SERVICE_CENSUS_LEN: usize = 192;
pub const HOURLY_SERVICE_ACK_LEN: usize = 24;
pub const HOURLY_SERVICE_BATCH_MAGIC: &[u8; 4] = b"H4SB";
pub const HOURLY_SERVICE_CENSUS_MAGIC: &[u8; 4] = b"H4SX";
pub const HOURLY_SERVICE_ACK_MAGIC: &[u8; 4] = b"H4SA";

const SAMPLE_PRESENCE_MASK: u16 = (1 << 11) - 1;
const CENSUS_SEALED: u32 = 1;
const CENSUS_MAXIMUM_PRESENT: u32 = 2;
const CENSUS_TERMINAL_LIFECYCLE_PRESENT: u32 = 4;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HourlyServiceOwnerV1 {
    pub logical_stream_id: u64,
    pub epoch: u64,
    pub game_hour_raw: i32,
    pub observation_ordinal: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HourlyServiceOutcomeV1 {
    Complete = 1,
    Partial = 2,
    Refused = 3,
    Aborted = 4,
    /// The request the capture teardown cut: closed by the terminal close,
    /// never by the loop; publishes nothing. A boundary hour.
    TeardownCut = 5,
}

impl HourlyServiceOutcomeV1 {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Result<Self, HourlyServiceCodecError> {
        match code {
            1 => Ok(Self::Complete),
            2 => Ok(Self::Partial),
            3 => Ok(Self::Refused),
            4 => Ok(Self::Aborted),
            5 => Ok(Self::TeardownCut),
            _ => Err(HourlyServiceCodecError::UnknownOutcome(code)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HourlyServiceSampleV1 {
    pub owner: HourlyServiceOwnerV1,
    pub outcome: HourlyServiceOutcomeV1,
    // The exclusive end belongs to this committed DATA frame, not its timing carrier.
    pub data_sequence: Option<(u64, u64)>,
    pub observed_service_ns: Option<u64>,
    pub stage_ns: [Option<u64>; HOURLY_SERVICE_STAGE_COUNT],
    pub observed_stages: u8,
    pub closed_stages: u8,
    pub not_applicable_stages: u8,
    pub edge_to_publication_ns: Option<u64>,
    pub edge_to_completion_ns: Option<u64>,
    pub fault_bits: u32,
}

impl HourlyServiceSampleV1 {
    pub fn validate(&self) -> Result<(), HourlyServiceCodecError> {
        check_fault_bits(self.fault_bits)?;
        for mask in [
            self.observed_stages,
            self.closed_stages,
            self.not_applicable_stages,
        ] {
            if mask & !HOURLY_SERVICE_STAGE_MASK != 0 {
                return Err(HourlyServiceCodecError::UnknownStageBits(mask));
            }
        }
        if self.fault_bits == 0
            && matches!(
                self.outcome,
                HourlyServiceOutcomeV1::Complete | HourlyServiceOutcomeV1::Partial
            )
            && self.not_applicable_stages & !(1 << 6) != 0
        {
            return Err(HourlyServiceCodecError::InvalidField(
                "published_nonterminal_not_applicable",
            ));
        }
        match (self.outcome, self.data_sequence) {
            (_, Some((first, after))) if first >= after => {
                Err(HourlyServiceCodecError::InvalidField("data_sequence"))
            }
            (HourlyServiceOutcomeV1::Complete | HourlyServiceOutcomeV1::Partial, None) => Err(
                HourlyServiceCodecError::InvalidField("published_outcome_without_data"),
            ),
            (HourlyServiceOutcomeV1::Refused, Some(_)) => Err(
                HourlyServiceCodecError::InvalidField("refused_outcome_with_data"),
            ),
            (HourlyServiceOutcomeV1::TeardownCut, Some(_)) => Err(
                HourlyServiceCodecError::InvalidField("teardown_cut_outcome_with_data"),
            ),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HourlyServiceBatchV1 {
    pub batch_id: u64,
    pub samples: Vec<HourlyServiceSampleV1>,
}

impl HourlyServiceBatchV1 {
    pub fn encode(&self) -> Result<Vec<u8>, HourlyServiceCodecError> {
        let len = batch_len(self.samples.len())?;
        if self.batch_id == 0 {
            return Err(HourlyServiceCodecError::InvalidField("batch_id"));
        }
        for sample in &self.samples {
            sample.validate()?;
        }
        let mut out = vec![0u8; len];
        write_prefix(&mut out, HOURLY_SERVICE_BATCH_MAGIC);
        put_u64(&mut out, 8, self.batch_id);
        put_u16(&mut out, 16, self.samples.len() as u16);
        put_u16(&mut out, 18, HOURLY_SERVICE_SAMPLE_LEN as u16);
        for (bytes, sample) in out[HOURLY_SERVICE_BATCH_HEADER_LEN..]
            .chunks_exact_mut(HOURLY_SERVICE_SAMPLE_LEN)
            .zip(&self.samples)
        {
            encode_sample(bytes, sample);
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, HourlyServiceCodecError> {
        check_prefix(
            bytes,
            HOURLY_SERVICE_BATCH_MAGIC,
            HOURLY_SERVICE_BATCH_HEADER_LEN,
        )?;
        let count = read_u16(bytes, 16) as usize;
        check_length(bytes, batch_len(count)?)?;
        if read_u16(bytes, 18) as usize != HOURLY_SERVICE_SAMPLE_LEN {
            return Err(HourlyServiceCodecError::InvalidField("sample_length"));
        }
        check_reserved(&bytes[20..24])?;
        let batch_id = read_u64(bytes, 8);
        if batch_id == 0 {
            return Err(HourlyServiceCodecError::InvalidField("batch_id"));
        }
        let mut samples = Vec::with_capacity(count);
        for sample in
            bytes[HOURLY_SERVICE_BATCH_HEADER_LEN..].chunks_exact(HOURLY_SERVICE_SAMPLE_LEN)
        {
            samples.push(decode_sample(sample)?);
        }
        Ok(Self { batch_id, samples })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HourlyServiceCensusV1 {
    pub last_batch_id: u64,
    pub offered_requests: u64,
    pub admitted_requests: u64,
    pub closed_requests: u64,
    pub queued_measurements: u64,
    pub lost_measurements: u64,
    pub complete_measurements: u64,
    pub incomplete_measurements: u64,
    pub partial_requests: u64,
    pub refused_requests: u64,
    pub aborted_requests: u64,
    pub unmeasured_hours: u64,
    pub boundary_excluded_hours: u64,
    pub over_limit_measurements: u64,
    pub total_service_ns: u64,
    pub maximum_service_ns: Option<u64>,
    pub active_requests: u64,
    pub pending_delivery: u64,
    pub unattributed_service_ns: u64,
    // Includes preceding timing batches and their receipts; excludes this census exchange.
    pub diagnostic_transport_ns: u64,
    pub terminal_lifecycle_ns: Option<u64>,
    pub fault_bits: u32,
    pub sealed: bool,
}

impl HourlyServiceCensusV1 {
    pub fn encode(&self) -> Result<Vec<u8>, HourlyServiceCodecError> {
        check_fault_bits(self.fault_bits)?;
        let mut out = vec![0u8; HOURLY_SERVICE_CENSUS_LEN];
        write_prefix(&mut out, HOURLY_SERVICE_CENSUS_MAGIC);
        put_u64(&mut out, 8, self.last_batch_id);
        let flags = if self.sealed { CENSUS_SEALED } else { 0 }
            | if self.maximum_service_ns.is_some() {
                CENSUS_MAXIMUM_PRESENT
            } else {
                0
            }
            | if self.terminal_lifecycle_ns.is_some() {
                CENSUS_TERMINAL_LIFECYCLE_PRESENT
            } else {
                0
            };
        put_u32(&mut out, 16, flags);
        put_u32(&mut out, 20, self.fault_bits);
        let fields = [
            self.offered_requests,
            self.admitted_requests,
            self.closed_requests,
            self.queued_measurements,
            self.lost_measurements,
            self.complete_measurements,
            self.incomplete_measurements,
            self.partial_requests,
            self.refused_requests,
            self.aborted_requests,
            self.unmeasured_hours,
            self.over_limit_measurements,
            self.total_service_ns,
            self.maximum_service_ns.unwrap_or(0),
            self.active_requests,
            self.pending_delivery,
            self.unattributed_service_ns,
            self.diagnostic_transport_ns,
            self.terminal_lifecycle_ns.unwrap_or(0),
        ];
        for (index, value) in fields.into_iter().enumerate() {
            put_u64(&mut out, 32 + index * 8, value);
        }
        put_u64(&mut out, 184, self.boundary_excluded_hours);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, HourlyServiceCodecError> {
        check_prefix(
            bytes,
            HOURLY_SERVICE_CENSUS_MAGIC,
            HOURLY_SERVICE_CENSUS_LEN,
        )?;
        check_length(bytes, HOURLY_SERVICE_CENSUS_LEN)?;
        let flags = read_u32(bytes, 16);
        if flags & !(CENSUS_SEALED | CENSUS_MAXIMUM_PRESENT | CENSUS_TERMINAL_LIFECYCLE_PRESENT)
            != 0
        {
            return Err(HourlyServiceCodecError::UnknownFlags(flags));
        }
        let fault_bits = read_u32(bytes, 20);
        check_fault_bits(fault_bits)?;
        check_reserved(&bytes[24..32])?;
        let maximum_service_ns = read_option_u64(bytes, 136, flags & CENSUS_MAXIMUM_PRESENT != 0)?;
        Ok(Self {
            last_batch_id: read_u64(bytes, 8),
            offered_requests: read_u64(bytes, 32),
            admitted_requests: read_u64(bytes, 40),
            closed_requests: read_u64(bytes, 48),
            queued_measurements: read_u64(bytes, 56),
            lost_measurements: read_u64(bytes, 64),
            complete_measurements: read_u64(bytes, 72),
            incomplete_measurements: read_u64(bytes, 80),
            partial_requests: read_u64(bytes, 88),
            refused_requests: read_u64(bytes, 96),
            aborted_requests: read_u64(bytes, 104),
            unmeasured_hours: read_u64(bytes, 112),
            boundary_excluded_hours: read_u64(bytes, 184),
            over_limit_measurements: read_u64(bytes, 120),
            total_service_ns: read_u64(bytes, 128),
            maximum_service_ns,
            active_requests: read_u64(bytes, 144),
            pending_delivery: read_u64(bytes, 152),
            unattributed_service_ns: read_u64(bytes, 160),
            diagnostic_transport_ns: read_u64(bytes, 168),
            terminal_lifecycle_ns: read_option_u64(
                bytes,
                176,
                flags & CENSUS_TERMINAL_LIFECYCLE_PRESENT != 0,
            )?,
            fault_bits,
            sealed: flags & CENSUS_SEALED != 0,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum HourlyServiceAckKindV1 {
    Batch = 1,
    Census = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HourlyServiceAckV1 {
    pub kind: HourlyServiceAckKindV1,
    pub batch_id: u64,
    pub accepted_count: u16,
}

impl HourlyServiceAckV1 {
    pub fn validate(&self) -> Result<(), HourlyServiceCodecError> {
        match self.kind {
            HourlyServiceAckKindV1::Batch => {
                if self.batch_id == 0 {
                    return Err(HourlyServiceCodecError::InvalidField("batch_id"));
                }
                if self.accepted_count as usize > HOURLY_SERVICE_MAX_BATCH_SAMPLES {
                    return Err(HourlyServiceCodecError::InvalidField("accepted_count"));
                }
            }
            HourlyServiceAckKindV1::Census if self.accepted_count > 1 => {
                return Err(HourlyServiceCodecError::InvalidField("accepted_count"))
            }
            HourlyServiceAckKindV1::Census => {}
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, HourlyServiceCodecError> {
        self.validate()?;
        let mut out = vec![0u8; HOURLY_SERVICE_ACK_LEN];
        write_prefix(&mut out, HOURLY_SERVICE_ACK_MAGIC);
        put_u64(&mut out, 8, self.batch_id);
        put_u16(&mut out, 16, self.kind as u16);
        put_u16(&mut out, 18, self.accepted_count);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, HourlyServiceCodecError> {
        check_prefix(bytes, HOURLY_SERVICE_ACK_MAGIC, HOURLY_SERVICE_ACK_LEN)?;
        check_length(bytes, HOURLY_SERVICE_ACK_LEN)?;
        check_reserved(&bytes[20..24])?;
        let kind = match read_u16(bytes, 16) {
            1 => HourlyServiceAckKindV1::Batch,
            2 => HourlyServiceAckKindV1::Census,
            code => return Err(HourlyServiceCodecError::UnknownAckKind(code)),
        };
        let value = Self {
            kind,
            batch_id: read_u64(bytes, 8),
            accepted_count: read_u16(bytes, 18),
        };
        value.validate()?;
        Ok(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HourlyServiceCodecError {
    InvalidLength { expected: usize, actual: usize },
    Truncated { minimum: usize, actual: usize },
    BadMagic,
    UnsupportedVersion(u16),
    DeclaredLength { declared: usize, actual: usize },
    InvalidBatchCount(usize),
    NonZeroReserved,
    NonCanonicalAbsentValue,
    UnknownOutcome(u8),
    UnknownAckKind(u16),
    UnknownStageBits(u8),
    UnknownFaultBits(u32),
    UnknownFlags(u32),
    InvalidField(&'static str),
}

impl fmt::Display for HourlyServiceCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "hourly service payload: {self:?}")
    }
}

impl std::error::Error for HourlyServiceCodecError {}

fn batch_len(count: usize) -> Result<usize, HourlyServiceCodecError> {
    if !(1..=HOURLY_SERVICE_MAX_BATCH_SAMPLES).contains(&count) {
        return Err(HourlyServiceCodecError::InvalidBatchCount(count));
    }
    count
        .checked_mul(HOURLY_SERVICE_SAMPLE_LEN)
        .and_then(|len| len.checked_add(HOURLY_SERVICE_BATCH_HEADER_LEN))
        .ok_or(HourlyServiceCodecError::InvalidBatchCount(count))
}

fn check_fault_bits(bits: u32) -> Result<(), HourlyServiceCodecError> {
    if bits & !HOURLY_SERVICE_KNOWN_FAULT_BITS != 0 {
        Err(HourlyServiceCodecError::UnknownFaultBits(bits))
    } else {
        Ok(())
    }
}

fn write_prefix(out: &mut [u8], magic: &[u8; 4]) {
    let len = u16::try_from(out.len()).expect("bounded hourly service payload");
    out[..4].copy_from_slice(magic);
    put_u16(out, 4, HOURLY_SERVICE_VERSION);
    put_u16(out, 6, len);
}

fn check_prefix(
    bytes: &[u8],
    magic: &[u8; 4],
    minimum: usize,
) -> Result<(), HourlyServiceCodecError> {
    if bytes.len() < minimum {
        return Err(HourlyServiceCodecError::Truncated {
            minimum,
            actual: bytes.len(),
        });
    }
    if &bytes[..4] != magic {
        return Err(HourlyServiceCodecError::BadMagic);
    }
    let version = read_u16(bytes, 4);
    if version != HOURLY_SERVICE_VERSION {
        return Err(HourlyServiceCodecError::UnsupportedVersion(version));
    }
    let declared = read_u16(bytes, 6) as usize;
    if declared != bytes.len() {
        return Err(HourlyServiceCodecError::DeclaredLength {
            declared,
            actual: bytes.len(),
        });
    }
    Ok(())
}

fn check_length(bytes: &[u8], expected: usize) -> Result<(), HourlyServiceCodecError> {
    if bytes.len() != expected {
        Err(HourlyServiceCodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        })
    } else {
        Ok(())
    }
}

fn check_reserved(bytes: &[u8]) -> Result<(), HourlyServiceCodecError> {
    if bytes.iter().any(|byte| *byte != 0) {
        Err(HourlyServiceCodecError::NonZeroReserved)
    } else {
        Ok(())
    }
}

fn encode_sample(out: &mut [u8], value: &HourlyServiceSampleV1) {
    put_u64(out, 0, value.owner.logical_stream_id);
    put_u64(out, 8, value.owner.epoch);
    out[16..20].copy_from_slice(&value.owner.game_hour_raw.to_le_bytes());
    out[20] = value.outcome.code();
    out[21] = value.observed_stages;
    out[22] = value.closed_stages;
    out[23] = value.not_applicable_stages;
    put_u64(out, 24, value.owner.observation_ordinal);
    put_u32(out, 32, value.fault_bits);
    let mut presence = 0u16;
    if let Some((first, after)) = value.data_sequence {
        presence |= 1;
        put_u64(out, 40, first);
        put_u64(out, 48, after);
    }
    put_option_u64(out, 56, value.observed_service_ns, 1, &mut presence);
    for (index, stage) in value.stage_ns.into_iter().enumerate() {
        put_option_u64(out, 64 + index * 8, stage, 2 + index, &mut presence);
    }
    put_option_u64(out, 120, value.edge_to_publication_ns, 9, &mut presence);
    put_option_u64(out, 128, value.edge_to_completion_ns, 10, &mut presence);
    put_u16(out, 36, presence);
}

fn decode_sample(bytes: &[u8]) -> Result<HourlyServiceSampleV1, HourlyServiceCodecError> {
    let presence = read_u16(bytes, 36);
    if presence & !SAMPLE_PRESENCE_MASK != 0 {
        return Err(HourlyServiceCodecError::UnknownFlags(presence as u32));
    }
    check_reserved(&bytes[38..40])?;
    let data_sequence = if presence & 1 != 0 {
        Some((read_u64(bytes, 40), read_u64(bytes, 48)))
    } else {
        read_option_u64(bytes, 40, false)?;
        read_option_u64(bytes, 48, false)?;
        None
    };
    let mut stage_ns = [None; HOURLY_SERVICE_STAGE_COUNT];
    for (index, stage) in stage_ns.iter_mut().enumerate() {
        *stage = read_option_u64(bytes, 64 + index * 8, presence & (1 << (2 + index)) != 0)?;
    }
    let value = HourlyServiceSampleV1 {
        owner: HourlyServiceOwnerV1 {
            logical_stream_id: read_u64(bytes, 0),
            epoch: read_u64(bytes, 8),
            game_hour_raw: i32::from_le_bytes(
                bytes[16..20].try_into().expect("checked sample length"),
            ),
            observation_ordinal: read_u64(bytes, 24),
        },
        outcome: HourlyServiceOutcomeV1::from_code(bytes[20])?,
        data_sequence,
        observed_service_ns: read_option_u64(bytes, 56, presence & (1 << 1) != 0)?,
        stage_ns,
        observed_stages: bytes[21],
        closed_stages: bytes[22],
        not_applicable_stages: bytes[23],
        edge_to_publication_ns: read_option_u64(bytes, 120, presence & (1 << 9) != 0)?,
        edge_to_completion_ns: read_option_u64(bytes, 128, presence & (1 << 10) != 0)?,
        fault_bits: read_u32(bytes, 32),
    };
    value.validate()?;
    Ok(value)
}

fn put_option_u64(
    out: &mut [u8],
    offset: usize,
    value: Option<u64>,
    bit: usize,
    presence: &mut u16,
) {
    if let Some(value) = value {
        *presence |= 1 << bit;
        put_u64(out, offset, value);
    }
}

fn read_option_u64(
    bytes: &[u8],
    offset: usize,
    present: bool,
) -> Result<Option<u64>, HourlyServiceCodecError> {
    let value = read_u64(bytes, offset);
    if present {
        Ok(Some(value))
    } else if value == 0 {
        Ok(None)
    } else {
        Err(HourlyServiceCodecError::NonCanonicalAbsentValue)
    }
}

fn put_u16(out: &mut [u8], offset: usize, value: u16) {
    out[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(out: &mut [u8], offset: usize, value: u32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut [u8], offset: usize, value: u64) {
    out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("checked payload length"),
    )
}
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("checked payload length"),
    )
}
fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("checked payload length"),
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HourlyServiceSampleV1 {
        HourlyServiceSampleV1 {
            owner: HourlyServiceOwnerV1 {
                logical_stream_id: 3,
                epoch: 4,
                game_hour_raw: 19_360_001,
                observation_ordinal: 5,
            },
            outcome: HourlyServiceOutcomeV1::Complete,
            data_sequence: Some((7, 11)),
            observed_service_ns: Some(HOURLY_SERVICE_LIMIT_NS),
            stage_ns: [
                Some(8_000_000),
                Some(2_000_000),
                Some(1_000_000),
                Some(1_000_000),
                Some(3_000_000),
                Some(1_000_000),
                None,
            ],
            observed_stages: 0x3f,
            closed_stages: HOURLY_SERVICE_STAGE_MASK,
            not_applicable_stages: 0x40,
            edge_to_publication_ns: Some(30_000_000),
            edge_to_completion_ns: Some(31_000_000),
            fault_bits: 0,
        }
    }

    fn batch() -> HourlyServiceBatchV1 {
        HourlyServiceBatchV1 {
            batch_id: 1,
            samples: vec![sample()],
        }
    }

    fn absent_sample() -> HourlyServiceSampleV1 {
        HourlyServiceSampleV1 {
            outcome: HourlyServiceOutcomeV1::Refused,
            data_sequence: None,
            observed_service_ns: None,
            stage_ns: [None; HOURLY_SERVICE_STAGE_COUNT],
            observed_stages: 0,
            closed_stages: 0,
            not_applicable_stages: 0,
            edge_to_publication_ns: None,
            edge_to_completion_ns: None,
            fault_bits: 1 << 9,
            ..sample()
        }
    }

    #[test]
    fn batches_preserve_owner_sequence_stages_and_single_nanosecond_excess() {
        let mut value = batch();
        let mut later = sample();
        later.owner.epoch += 1;
        later.owner.observation_ordinal = 1;
        later.data_sequence = Some((100, 500));
        later.observed_service_ns = Some(HOURLY_SERVICE_LIMIT_NS + 1);
        later.outcome = HourlyServiceOutcomeV1::Partial;
        value.samples.push(later);
        let wire = value.encode().unwrap();
        assert_eq!(
            wire.len(),
            HOURLY_SERVICE_BATCH_HEADER_LEN + 2 * HOURLY_SERVICE_SAMPLE_LEN
        );
        assert_eq!(HourlyServiceBatchV1::decode(&wire).unwrap(), value);
        assert_eq!(&wire[..8], b"H4SB\x02\x00\x28\x01");
    }

    #[test]
    fn absent_and_measured_zero_are_distinct_for_every_optional_duration() {
        let absent = absent_sample();
        let zero = HourlyServiceSampleV1 {
            observed_service_ns: Some(0),
            stage_ns: [Some(0); HOURLY_SERVICE_STAGE_COUNT],
            observed_stages: HOURLY_SERVICE_STAGE_MASK,
            closed_stages: HOURLY_SERVICE_STAGE_MASK,
            edge_to_publication_ns: Some(0),
            edge_to_completion_ns: Some(0),
            ..absent
        };
        let value = HourlyServiceBatchV1 {
            batch_id: 2,
            samples: vec![absent, zero],
        };
        let decoded = HourlyServiceBatchV1::decode(&value.encode().unwrap()).unwrap();
        assert_eq!(decoded, value);
        assert_ne!(decoded.samples[0], decoded.samples[1]);
    }

    #[test]
    fn bounded_batch_accepts_sixteen_and_refuses_zero_or_seventeen_before_allocation() {
        let value = HourlyServiceBatchV1 {
            batch_id: 1,
            samples: vec![sample(); 16],
        };
        let wire = value.encode().unwrap();
        assert_eq!(wire.len(), HOURLY_SERVICE_MAX_BATCH_LEN);
        assert!(wire.len() < crate::control_transport::CONTROL_MAX_PAYLOAD_LEN);
        assert_eq!(HourlyServiceBatchV1::decode(&wire).unwrap(), value);
        for count in [0, 17] {
            let bad = HourlyServiceBatchV1 {
                batch_id: 1,
                samples: vec![sample(); count],
            };
            assert!(matches!(
                bad.encode(),
                Err(HourlyServiceCodecError::InvalidBatchCount(_))
            ));
        }
        for count in [0, 17, u16::MAX] {
            let mut bad = wire.clone();
            put_u16(&mut bad, 16, count);
            assert!(matches!(
                HourlyServiceBatchV1::decode(&bad),
                Err(HourlyServiceCodecError::InvalidBatchCount(_))
            ));
        }
    }

    #[test]
    fn batch_prefix_count_width_and_all_truncations_fail_closed() {
        let wire = batch().encode().unwrap();
        for len in 0..wire.len() {
            assert!(HourlyServiceBatchV1::decode(&wire[..len]).is_err());
        }
        for (offset, value) in [(0, 0), (4, 0xff), (6, 0), (16, 2), (18, 0)] {
            let mut bad = wire.clone();
            bad[offset] = value;
            assert!(
                HourlyServiceBatchV1::decode(&bad).is_err(),
                "offset {offset}"
            );
        }
        let mut extra = wire.clone();
        extra.push(0);
        assert!(HourlyServiceBatchV1::decode(&extra).is_err());
        let declared = extra.len() as u16;
        put_u16(&mut extra, 6, declared);
        assert!(HourlyServiceBatchV1::decode(&extra).is_err());
        let mut zero_id = wire;
        put_u64(&mut zero_id, 8, 0);
        assert!(HourlyServiceBatchV1::decode(&zero_id).is_err());
    }

    #[test]
    fn every_batch_reserved_byte_and_unknown_bit_is_rejected() {
        let wire = batch().encode().unwrap();
        let start = HOURLY_SERVICE_BATCH_HEADER_LEN;
        for offset in (20..24).chain(start + 38..start + 40) {
            let mut bad = wire.clone();
            bad[offset] = 1;
            assert_eq!(
                HourlyServiceBatchV1::decode(&bad),
                Err(HourlyServiceCodecError::NonZeroReserved)
            );
        }
        for offset in [start + 21, start + 22, start + 23] {
            let mut bad = wire.clone();
            bad[offset] |= 0x80;
            assert!(matches!(
                HourlyServiceBatchV1::decode(&bad),
                Err(HourlyServiceCodecError::UnknownStageBits(_))
            ));
        }
        for bit in 17..32 {
            let mut bad = wire.clone();
            put_u32(&mut bad, start + 32, 1 << bit);
            assert!(matches!(
                HourlyServiceBatchV1::decode(&bad),
                Err(HourlyServiceCodecError::UnknownFaultBits(_))
            ));
        }
        for bit in 11..16 {
            let mut bad = wire.clone();
            let present = read_u16(&bad, start + 36);
            put_u16(&mut bad, start + 36, present | 1 << bit);
            assert!(matches!(
                HourlyServiceBatchV1::decode(&bad),
                Err(HourlyServiceCodecError::UnknownFlags(_))
            ));
        }
        for outcome in [0, 6, u8::MAX] {
            let mut bad = wire.clone();
            bad[start + 20] = outcome;
            assert_eq!(
                HourlyServiceBatchV1::decode(&bad),
                Err(HourlyServiceCodecError::UnknownOutcome(outcome))
            );
        }
    }

    #[test]
    fn absence_flags_cannot_hide_nonzero_duration_or_data_sequence_bytes() {
        let wire = HourlyServiceBatchV1 {
            batch_id: 1,
            samples: vec![absent_sample()],
        }
        .encode()
        .unwrap();
        for offset in HOURLY_SERVICE_BATCH_HEADER_LEN + 40..wire.len() {
            let mut bad = wire.clone();
            bad[offset] = 1;
            assert_eq!(
                HourlyServiceBatchV1::decode(&bad),
                Err(HourlyServiceCodecError::NonCanonicalAbsentValue),
                "offset {offset}"
            );
        }
    }

    #[test]
    fn data_sequence_pairs_are_exclusive_and_match_the_outcome() {
        for outcome in [
            HourlyServiceOutcomeV1::Complete,
            HourlyServiceOutcomeV1::Partial,
        ] {
            let bad = HourlyServiceSampleV1 {
                outcome,
                data_sequence: None,
                ..sample()
            };
            assert!(bad.validate().is_err());
        }
        let refused = HourlyServiceSampleV1 {
            outcome: HourlyServiceOutcomeV1::Refused,
            ..sample()
        };
        assert!(refused.validate().is_err());
        for sequence in [(8, 8), (9, 8)] {
            assert!(HourlyServiceSampleV1 {
                data_sequence: Some(sequence),
                ..sample()
            }
            .validate()
            .is_err());
        }
        for sequence in [None, Some((0, u64::MAX))] {
            let aborted = HourlyServiceSampleV1 {
                outcome: HourlyServiceOutcomeV1::Aborted,
                data_sequence: sequence,
                ..sample()
            };
            let value = HourlyServiceBatchV1 {
                batch_id: u64::MAX,
                samples: vec![aborted],
            };
            assert_eq!(
                HourlyServiceBatchV1::decode(&value.encode().unwrap()).unwrap(),
                value
            );
        }
    }

    #[test]
    fn only_failed_requests_may_omit_nonterminal_stages_without_a_timing_fault() {
        for outcome in [
            HourlyServiceOutcomeV1::Complete,
            HourlyServiceOutcomeV1::Partial,
        ] {
            let mut value = HourlyServiceSampleV1 {
                outcome,
                not_applicable_stages: 1 << 4,
                ..sample()
            };
            assert!(value.validate().is_err());
            value.fault_bits = 1 << 9;
            assert!(value.validate().is_ok());
        }
        for outcome in [
            HourlyServiceOutcomeV1::Refused,
            HourlyServiceOutcomeV1::Aborted,
            HourlyServiceOutcomeV1::TeardownCut,
        ] {
            let value = HourlyServiceSampleV1 {
                outcome,
                not_applicable_stages: HOURLY_SERVICE_STAGE_MASK,
                data_sequence: None,
                ..sample()
            };
            assert!(value.validate().is_ok());
        }
    }

    /// Lane PF: the teardown cut is its own outcome code so a reader can
    /// tell the hour the terminal close cut from an hour the loop lost, and
    /// it can never claim a published DATA span.
    #[test]
    fn the_teardown_cut_is_its_own_code_and_publishes_nothing() {
        assert_eq!(HourlyServiceOutcomeV1::TeardownCut.code(), 5);
        assert_eq!(
            HourlyServiceOutcomeV1::from_code(5).unwrap(),
            HourlyServiceOutcomeV1::TeardownCut
        );
        let cut = HourlyServiceSampleV1 {
            outcome: HourlyServiceOutcomeV1::TeardownCut,
            data_sequence: None,
            ..sample()
        };
        assert!(cut.validate().is_ok());
        let published = HourlyServiceSampleV1 {
            data_sequence: Some((1, 3)),
            ..cut
        };
        assert_eq!(
            published.validate().unwrap_err(),
            HourlyServiceCodecError::InvalidField("teardown_cut_outcome_with_data")
        );
    }

    #[test]
    fn collector_fault_diagnostics_survive_without_claiming_semantic_completeness() {
        let mut value = batch();
        value.samples[0].fault_bits = HOURLY_SERVICE_KNOWN_FAULT_BITS;
        value.samples[0].closed_stages = 0;
        value.samples[0].observed_service_ns = Some(u64::MAX);
        value.samples[0].edge_to_completion_ns = None;
        assert_eq!(
            HourlyServiceBatchV1::decode(&value.encode().unwrap()).unwrap(),
            value
        );
    }

    fn census() -> HourlyServiceCensusV1 {
        HourlyServiceCensusV1 {
            last_batch_id: 1,
            offered_requests: 2,
            admitted_requests: 3,
            closed_requests: 4,
            queued_measurements: 5,
            lost_measurements: 6,
            complete_measurements: 7,
            incomplete_measurements: 8,
            partial_requests: 9,
            refused_requests: 10,
            aborted_requests: 11,
            unmeasured_hours: 12,
            boundary_excluded_hours: 21,
            over_limit_measurements: 13,
            total_service_ns: 14,
            maximum_service_ns: Some(15),
            active_requests: 16,
            pending_delivery: 17,
            unattributed_service_ns: 18,
            diagnostic_transport_ns: 19,
            terminal_lifecycle_ns: Some(20),
            fault_bits: HOURLY_SERVICE_KNOWN_FAULT_BITS,
            sealed: true,
        }
    }

    #[test]
    fn census_preserves_failed_counts_and_distinct_queued_and_received_meanings() {
        let value = census();
        let wire = value.encode().unwrap();
        assert_eq!(HourlyServiceCensusV1::decode(&wire).unwrap(), value);
        assert_eq!(&wire[..8], b"H4SX\x02\x00\xc0\x00");
        for (index, expected) in (2u64..=20).enumerate() {
            assert_eq!(read_u64(&wire, 32 + index * 8), expected);
        }
        assert_eq!(read_u64(&wire, 184), 21);
        assert_eq!(read_u32(&wire, 16), 7);
        assert_eq!(read_u32(&wire, 20), HOURLY_SERVICE_KNOWN_FAULT_BITS);
    }

    #[test]
    fn census_optional_zero_is_distinct_from_missing_maximum_and_terminal_lifecycle() {
        for maximum_service_ns in [None, Some(0), Some(u64::MAX)] {
            for terminal_lifecycle_ns in [None, Some(0), Some(u64::MAX)] {
                let value = HourlyServiceCensusV1 {
                    maximum_service_ns,
                    terminal_lifecycle_ns,
                    ..HourlyServiceCensusV1::default()
                };
                assert_eq!(
                    HourlyServiceCensusV1::decode(&value.encode().unwrap()).unwrap(),
                    value
                );
            }
        }
        for offset in [136, 176] {
            let mut bad = HourlyServiceCensusV1::default().encode().unwrap();
            put_u64(&mut bad, offset, 1);
            assert_eq!(
                HourlyServiceCensusV1::decode(&bad),
                Err(HourlyServiceCodecError::NonCanonicalAbsentValue)
            );
        }
    }

    #[test]
    fn census_rejects_unknown_flags_faults_reserved_and_noncanonical_lengths() {
        let wire = census().encode().unwrap();
        for len in 0..wire.len() {
            assert!(HourlyServiceCensusV1::decode(&wire[..len]).is_err());
        }
        let mut missing_terminal_lifecycle = wire[..176].to_vec();
        put_u16(&mut missing_terminal_lifecycle, 6, 176);
        assert!(HourlyServiceCensusV1::decode(&missing_terminal_lifecycle).is_err());
        for offset in 24..32 {
            let mut bad = wire.clone();
            bad[offset] = 1;
            assert_eq!(
                HourlyServiceCensusV1::decode(&bad),
                Err(HourlyServiceCodecError::NonZeroReserved)
            );
        }
        for bit in 3..32 {
            let mut bad = wire.clone();
            put_u32(&mut bad, 16, 1 << bit);
            assert!(matches!(
                HourlyServiceCensusV1::decode(&bad),
                Err(HourlyServiceCodecError::UnknownFlags(_))
            ));
        }
        for bit in 17..32 {
            let mut bad = wire.clone();
            put_u32(&mut bad, 20, 1 << bit);
            assert!(matches!(
                HourlyServiceCensusV1::decode(&bad),
                Err(HourlyServiceCodecError::UnknownFaultBits(_))
            ));
        }
        for offset in [0, 4, 6] {
            let mut bad = wire.clone();
            bad[offset] = 0xff;
            assert!(HourlyServiceCensusV1::decode(&bad).is_err());
        }
        let mut extra = wire;
        extra.push(0);
        let declared = extra.len() as u16;
        put_u16(&mut extra, 6, declared);
        assert!(HourlyServiceCensusV1::decode(&extra).is_err());
    }

    #[test]
    fn receipts_bind_batch_identity_count_and_kind_including_negative_receipts() {
        for kind in [
            HourlyServiceAckKindV1::Batch,
            HourlyServiceAckKindV1::Census,
        ] {
            for accepted_count in [0, 1] {
                let value = HourlyServiceAckV1 {
                    kind,
                    batch_id: 7,
                    accepted_count,
                };
                assert_eq!(
                    HourlyServiceAckV1::decode(&value.encode().unwrap()).unwrap(),
                    value
                );
            }
        }
        let zero_census = HourlyServiceAckV1 {
            kind: HourlyServiceAckKindV1::Census,
            batch_id: 0,
            accepted_count: 1,
        };
        assert_eq!(
            HourlyServiceAckV1::decode(&zero_census.encode().unwrap()).unwrap(),
            zero_census
        );
        let last_batch = HourlyServiceAckV1 {
            kind: HourlyServiceAckKindV1::Batch,
            batch_id: u64::MAX,
            accepted_count: 16,
        };
        assert_eq!(
            HourlyServiceAckV1::decode(&last_batch.encode().unwrap()).unwrap(),
            last_batch
        );
    }

    #[test]
    fn malformed_receipts_cannot_be_decoded_as_acceptance() {
        let value = HourlyServiceAckV1 {
            kind: HourlyServiceAckKindV1::Batch,
            batch_id: 1,
            accepted_count: 1,
        };
        let wire = value.encode().unwrap();
        for len in 0..wire.len() {
            assert!(HourlyServiceAckV1::decode(&wire[..len]).is_err());
        }
        for offset in 20..24 {
            let mut bad = wire.clone();
            bad[offset] = 1;
            assert_eq!(
                HourlyServiceAckV1::decode(&bad),
                Err(HourlyServiceCodecError::NonZeroReserved)
            );
        }
        for code in [0, 3, u16::MAX] {
            let mut bad = wire.clone();
            put_u16(&mut bad, 16, code);
            assert_eq!(
                HourlyServiceAckV1::decode(&bad),
                Err(HourlyServiceCodecError::UnknownAckKind(code))
            );
        }
        for (kind, batch_id, accepted_count) in [
            (HourlyServiceAckKindV1::Batch, 0, 1),
            (HourlyServiceAckKindV1::Batch, 1, 17),
            (HourlyServiceAckKindV1::Census, 1, 2),
        ] {
            assert!(HourlyServiceAckV1 {
                kind,
                batch_id,
                accepted_count
            }
            .encode()
            .is_err());
            let mut bad = wire.clone();
            put_u64(&mut bad, 8, batch_id);
            put_u16(&mut bad, 16, kind as u16);
            put_u16(&mut bad, 18, accepted_count);
            assert!(HourlyServiceAckV1::decode(&bad).is_err());
        }
        for offset in [0, 4, 6] {
            let mut bad = wire.clone();
            bad[offset] = 0xff;
            assert!(HourlyServiceAckV1::decode(&bad).is_err());
        }
        let mut extra = wire;
        extra.push(0);
        let declared = extra.len() as u16;
        put_u16(&mut extra, 6, declared);
        assert!(HourlyServiceAckV1::decode(&extra).is_err());
    }

    #[test]
    fn payload_kinds_cannot_be_cross_decoded() {
        let batch = batch().encode().unwrap();
        let census = census().encode().unwrap();
        let ack = HourlyServiceAckV1 {
            kind: HourlyServiceAckKindV1::Batch,
            batch_id: 1,
            accepted_count: 1,
        }
        .encode()
        .unwrap();
        assert!(HourlyServiceBatchV1::decode(&census).is_err());
        assert!(HourlyServiceBatchV1::decode(&ack).is_err());
        assert!(HourlyServiceCensusV1::decode(&batch).is_err());
        assert!(HourlyServiceCensusV1::decode(&ack).is_err());
        assert!(HourlyServiceAckV1::decode(&batch).is_err());
        assert!(HourlyServiceAckV1::decode(&census).is_err());
    }
}
