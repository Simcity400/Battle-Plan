use crate::calendar::hour_index_from_raw;
use crate::record_header::CompactRecordHeader;

pub const CID1_MAGIC: &[u8; 4] = b"CID1";
pub const CID1_VERSION: u16 = 1;
pub const CID1_FIXED_HEADER_LEN: usize = 96;
pub const CID1_UUID_LEN: usize = 36;
pub const CID1_MAX_BUILD_PROFILE_LEN: usize = 128;
pub const CID1_MAX_SCENARIO_LEN: usize = 256;
pub const CID1_MAX_ROSTER_TAGS: usize = 64;
pub const CID1_ROSTER_TAG_LEN: usize = 3;
pub const CID1_MAX_ROSTER_BYTES_LEN: usize = CID1_MAX_ROSTER_TAGS * CID1_ROSTER_TAG_LEN;
pub const CID1_MAX_MODSET_IDENTITY_LEN: usize = 256;
pub const CID1_MAX_PAYLOAD_LEN: usize = CID1_FIXED_HEADER_LEN
    + CID1_UUID_LEN
    + CID1_MAX_BUILD_PROFILE_LEN
    + CID1_MAX_SCENARIO_LEN
    + CID1_MAX_ROSTER_BYTES_LEN
    + CID1_MAX_MODSET_IDENTITY_LEN;

pub use crate::record_semantics::COMPACT_RECORD_TYPE_STREAM_HEADER;
pub const COMPACT_DOMAIN_STREAM_HEADER: u16 = 0;
pub const CID1_SOURCE_LIVE_MEMORY: u16 = 1;

const FLAG_START_DATE_PRESENT: u32 = 1 << 0;
const FLAG_SESSION_PRESENT: u32 = 1 << 1;
const FLAG_SCENARIO_PRESENT: u32 = 1 << 2;
const FLAG_ROSTER_PRESENT: u32 = 1 << 3;
const FLAG_MODSET_PRESENT: u32 = 1 << 4;
const FLAG_FIRST_OBSERVATION_COMPLETE: u32 = 1 << 5;
const FLAG_SECOND_OBSERVATION_COMPLETE: u32 = 1 << 6;
const FLAG_OBSERVATIONS_EQUAL: u32 = 1 << 7;
const REQUIRED_PROOF_FLAGS: u32 =
    FLAG_FIRST_OBSERVATION_COMPLETE | FLAG_SECOND_OBSERVATION_COMPLETE | FLAG_OBSERVATIONS_EQUAL;
const KNOWN_FLAGS: u32 = FLAG_START_DATE_PRESENT
    | FLAG_SESSION_PRESENT
    | FLAG_SCENARIO_PRESENT
    | FLAG_ROSTER_PRESENT
    | FLAG_MODSET_PRESENT
    | REQUIRED_PROOF_FLAGS;

const CID_PREIMAGE_PREFIX: &[u8] = b"hoi4-campaign-id-v1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cid1Observation {
    pub game_unique_id: String,
    pub game_unique_seed: i32,
    pub game_hour_raw: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cid1Witnesses {
    pub start_date_raw: Option<u32>,
    pub session: Option<u32>,
    pub scenario: Option<String>,
    pub initial_human_country_tags: Option<Vec<String>>,
    pub modset_identity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cid1Fields {
    game_unique_id: String,
    game_unique_seed: i32,
    game_hour_raw: i32,
    source_build_profile: String,
    witnesses: Cid1Witnesses,
}

impl Cid1Fields {
    pub fn from_stable_observations(
        first: Cid1Observation,
        second: Cid1Observation,
        source_build_profile: String,
        witnesses: Cid1Witnesses,
    ) -> Result<Self, Cid1Error> {
        validate_canonical_uuid(&first.game_unique_id)?;
        validate_canonical_uuid(&second.game_unique_id)?;
        validate_game_hour(first.game_hour_raw)?;
        validate_game_hour(second.game_hour_raw)?;
        validate_build_profile(&source_build_profile)?;
        validate_witnesses(&witnesses)?;

        if first.game_unique_id != second.game_unique_id
            || first.game_unique_seed != second.game_unique_seed
        {
            return Err(Cid1Error::ObservationMismatch);
        }

        Ok(Self {
            game_unique_id: first.game_unique_id,
            game_unique_seed: first.game_unique_seed,
            game_hour_raw: second.game_hour_raw,
            source_build_profile,
            witnesses,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCid1 {
    payload: Vec<u8>,
    payload_sha256: [u8; 32],
    campaign_id: [u8; 32],
    game_unique_id: String,
    game_unique_seed: i32,
    game_hour_raw: i32,
    source_build_profile: String,
    witnesses: Cid1Witnesses,
}

impl ParsedCid1 {
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn payload_sha256(&self) -> [u8; 32] {
        self.payload_sha256
    }

    pub fn campaign_id(&self) -> [u8; 32] {
        self.campaign_id
    }

    pub fn game_unique_id(&self) -> &str {
        &self.game_unique_id
    }

    pub fn game_unique_seed(&self) -> i32 {
        self.game_unique_seed
    }

    pub fn game_hour_raw(&self) -> i32 {
        self.game_hour_raw
    }

    pub fn source_build_profile(&self) -> &str {
        &self.source_build_profile
    }

    pub fn witnesses(&self) -> &Cid1Witnesses {
        &self.witnesses
    }

    pub fn campaign_id_hex(&self) -> String {
        lower_hex(&self.campaign_id)
    }

    pub fn payload_sha256_hex(&self) -> String {
        lower_hex(&self.payload_sha256)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedCid1 {
    parsed: ParsedCid1,
}

impl ValidatedCid1 {
    pub fn payload(&self) -> &[u8] {
        self.parsed.payload()
    }

    pub fn payload_sha256(&self) -> [u8; 32] {
        self.parsed.payload_sha256()
    }

    pub fn campaign_id(&self) -> [u8; 32] {
        self.parsed.campaign_id()
    }

    pub fn game_unique_id(&self) -> &str {
        self.parsed.game_unique_id()
    }

    pub fn game_unique_seed(&self) -> i32 {
        self.parsed.game_unique_seed()
    }

    pub fn game_hour_raw(&self) -> i32 {
        self.parsed.game_hour_raw()
    }

    pub fn source_build_profile(&self) -> &str {
        self.parsed.source_build_profile()
    }

    pub fn witnesses(&self) -> &Cid1Witnesses {
        self.parsed.witnesses()
    }

    pub fn campaign_id_hex(&self) -> String {
        self.parsed.campaign_id_hex()
    }

    pub fn payload_sha256_hex(&self) -> String {
        self.parsed.payload_sha256_hex()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cid1Error {
    TooShort(usize),
    BadMagic,
    UnsupportedVersion(u16),
    UnsupportedFixedHeaderLen(usize),
    TotalLengthMismatch { declared: usize, actual: usize },
    PayloadTooLarge(usize),
    UnknownFlags(u32),
    MissingStableReadProof,
    UnsupportedSource(u16),
    ReservedNonZero,
    InvalidLength(&'static str),
    InvalidUtf8(&'static str),
    InvalidCanonicalUuid,
    InvalidGameHour(i32),
    InvalidBuildProfile,
    InvalidWitness(&'static str),
    ObservationMismatch,
    CampaignDigestMismatch,
    WrongRecordSequence(u64),
    WrongRecordDomain(u16),
    WrongRecordType(u16),
    RecordFlagsNonZero(u32),
    RecordHourMismatch { record: i32, payload: i32 },
    RecordPayloadLengthMismatch { declared: usize, actual: usize },
}

impl core::fmt::Display for Cid1Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort(actual) => write!(
                f,
                "CID1 payload is {actual} byte(s), shorter than fixed header {CID1_FIXED_HEADER_LEN}"
            ),
            Self::BadMagic => write!(f, "invalid CID1 magic"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported CID1 version {v}"),
            Self::UnsupportedFixedHeaderLen(v) => {
                write!(f, "unsupported CID1 fixed-header length {v}")
            }
            Self::TotalLengthMismatch { declared, actual } => write!(
                f,
                "CID1 total length {declared} does not equal actual length {actual}"
            ),
            Self::PayloadTooLarge(v) => {
                write!(f, "CID1 payload length {v} exceeds {CID1_MAX_PAYLOAD_LEN}")
            }
            Self::UnknownFlags(bits) => write!(f, "CID1 unknown flag bits 0x{bits:08x}"),
            Self::MissingStableReadProof => write!(f, "CID1 stable-read proof flags are incomplete"),
            Self::UnsupportedSource(v) => write!(f, "CID1 unsupported source enum {v}"),
            Self::ReservedNonZero => write!(f, "CID1 reserved bytes are nonzero"),
            Self::InvalidLength(field) => write!(f, "CID1 invalid {field} length"),
            Self::InvalidUtf8(field) => write!(f, "CID1 {field} is not valid UTF-8"),
            Self::InvalidCanonicalUuid => write!(f, "CID1 UUID is not canonical lowercase ASCII"),
            Self::InvalidGameHour(hour) => write!(f, "CID1 game hour {hour} is outside the calendar window"),
            Self::InvalidBuildProfile => write!(f, "CID1 build profile violates its closed grammar"),
            Self::InvalidWitness(field) => write!(f, "CID1 witness {field} violates its closed grammar"),
            Self::ObservationMismatch => {
                write!(f, "CID1 stable identity observations are not identical")
            }
            Self::CampaignDigestMismatch => write!(f, "CID1 campaign digest does not recompute"),
            Self::WrongRecordSequence(v) => write!(f, "CID1 record sequence is {v}, expected 0"),
            Self::WrongRecordDomain(v) => write!(f, "CID1 record domain is {v}, expected 0"),
            Self::WrongRecordType(v) => write!(f, "CID1 record type is {v}, expected 4"),
            Self::RecordFlagsNonZero(v) => write!(f, "CID1 H4CR record flags are 0x{v:08x}, expected zero"),
            Self::RecordHourMismatch { record, payload } => write!(
                f,
                "CID1 H4CR hour {record} does not equal payload bracketing hour {payload}"
            ),
            Self::RecordPayloadLengthMismatch { declared, actual } => write!(
                f,
                "CID1 H4CR payload length {declared} does not equal supplied length {actual}"
            ),
        }
    }
}

pub fn encode_cid1(fields: &Cid1Fields) -> Result<Vec<u8>, Cid1Error> {
    validate_canonical_uuid(&fields.game_unique_id)?;
    validate_game_hour(fields.game_hour_raw)?;
    validate_build_profile(&fields.source_build_profile)?;
    validate_witnesses(&fields.witnesses)?;

    let scenario = fields.witnesses.scenario.as_deref().unwrap_or("");
    let roster = fields
        .witnesses
        .initial_human_country_tags
        .as_deref()
        .unwrap_or(&[]);
    let modset = fields.witnesses.modset_identity.as_deref().unwrap_or("");
    let roster_len = roster.len() * CID1_ROSTER_TAG_LEN;
    let total_len = CID1_FIXED_HEADER_LEN
        + CID1_UUID_LEN
        + fields.source_build_profile.len()
        + scenario.len()
        + roster_len
        + modset.len();
    if total_len > CID1_MAX_PAYLOAD_LEN {
        return Err(Cid1Error::PayloadTooLarge(total_len));
    }

    let mut flags = REQUIRED_PROOF_FLAGS;
    if fields.witnesses.start_date_raw.is_some() {
        flags |= FLAG_START_DATE_PRESENT;
    }
    if fields.witnesses.session.is_some() {
        flags |= FLAG_SESSION_PRESENT;
    }
    if fields.witnesses.scenario.is_some() {
        flags |= FLAG_SCENARIO_PRESENT;
    }
    if fields.witnesses.initial_human_country_tags.is_some() {
        flags |= FLAG_ROSTER_PRESENT;
    }
    if fields.witnesses.modset_identity.is_some() {
        flags |= FLAG_MODSET_PRESENT;
    }

    let campaign_id = campaign_id_digest(&fields.game_unique_id, fields.game_unique_seed);
    let mut out = vec![0u8; CID1_FIXED_HEADER_LEN];
    out[0..4].copy_from_slice(CID1_MAGIC);
    put_u16(&mut out, 4, CID1_VERSION);
    put_u16(&mut out, 6, CID1_FIXED_HEADER_LEN as u16);
    put_u32(&mut out, 8, total_len as u32);
    put_u32(&mut out, 12, flags);
    put_u16(&mut out, 16, CID1_SOURCE_LIVE_MEMORY);
    put_i32(&mut out, 20, fields.game_unique_seed);
    put_i32(&mut out, 24, fields.game_hour_raw);
    put_u16(&mut out, 28, CID1_UUID_LEN as u16);
    put_u16(&mut out, 30, fields.source_build_profile.len() as u16);
    put_u16(&mut out, 32, scenario.len() as u16);
    put_u16(&mut out, 34, roster.len() as u16);
    put_u16(&mut out, 36, roster_len as u16);
    put_u16(&mut out, 38, modset.len() as u16);
    put_u32(&mut out, 40, fields.witnesses.start_date_raw.unwrap_or(0));
    put_u32(&mut out, 44, fields.witnesses.session.unwrap_or(0));
    out[48..80].copy_from_slice(&campaign_id);

    out.extend_from_slice(fields.game_unique_id.as_bytes());
    out.extend_from_slice(fields.source_build_profile.as_bytes());
    out.extend_from_slice(scenario.as_bytes());
    for tag in roster {
        out.extend_from_slice(tag.as_bytes());
    }
    out.extend_from_slice(modset.as_bytes());

    parse_cid1(&out)?;
    Ok(out)
}

pub fn parse_cid1(payload: &[u8]) -> Result<ParsedCid1, Cid1Error> {
    if payload.len() < CID1_FIXED_HEADER_LEN {
        return Err(Cid1Error::TooShort(payload.len()));
    }
    if &payload[0..4] != CID1_MAGIC {
        return Err(Cid1Error::BadMagic);
    }
    let version = get_u16(payload, 4);
    if version != CID1_VERSION {
        return Err(Cid1Error::UnsupportedVersion(version));
    }
    let fixed_len = get_u16(payload, 6) as usize;
    if fixed_len != CID1_FIXED_HEADER_LEN {
        return Err(Cid1Error::UnsupportedFixedHeaderLen(fixed_len));
    }
    let total_len = get_u32(payload, 8) as usize;
    if total_len != payload.len() {
        return Err(Cid1Error::TotalLengthMismatch {
            declared: total_len,
            actual: payload.len(),
        });
    }
    if total_len > CID1_MAX_PAYLOAD_LEN {
        return Err(Cid1Error::PayloadTooLarge(total_len));
    }

    let flags = get_u32(payload, 12);
    let unknown = flags & !KNOWN_FLAGS;
    if unknown != 0 {
        return Err(Cid1Error::UnknownFlags(unknown));
    }
    if flags & REQUIRED_PROOF_FLAGS != REQUIRED_PROOF_FLAGS {
        return Err(Cid1Error::MissingStableReadProof);
    }
    let source = get_u16(payload, 16);
    if source != CID1_SOURCE_LIVE_MEMORY {
        return Err(Cid1Error::UnsupportedSource(source));
    }
    if payload[18..20].iter().any(|&byte| byte != 0)
        || payload[80..CID1_FIXED_HEADER_LEN]
            .iter()
            .any(|&byte| byte != 0)
    {
        return Err(Cid1Error::ReservedNonZero);
    }

    let seed = get_i32(payload, 20);
    let game_hour_raw = get_i32(payload, 24);
    validate_game_hour(game_hour_raw)?;
    let uuid_len = get_u16(payload, 28) as usize;
    let profile_len = get_u16(payload, 30) as usize;
    let scenario_len = get_u16(payload, 32) as usize;
    let roster_count = get_u16(payload, 34) as usize;
    let roster_len = get_u16(payload, 36) as usize;
    let modset_len = get_u16(payload, 38) as usize;
    if uuid_len != CID1_UUID_LEN {
        return Err(Cid1Error::InvalidLength("UUID"));
    }
    if !(1..=CID1_MAX_BUILD_PROFILE_LEN).contains(&profile_len) {
        return Err(Cid1Error::InvalidLength("build profile"));
    }
    if scenario_len > CID1_MAX_SCENARIO_LEN {
        return Err(Cid1Error::InvalidLength("scenario"));
    }
    if roster_count > CID1_MAX_ROSTER_TAGS || roster_len != roster_count * CID1_ROSTER_TAG_LEN {
        return Err(Cid1Error::InvalidLength("roster"));
    }
    if modset_len > CID1_MAX_MODSET_IDENTITY_LEN {
        return Err(Cid1Error::InvalidLength("modset identity"));
    }
    check_presence(
        flags,
        FLAG_START_DATE_PRESENT,
        get_u32(payload, 40) != 0,
        "start date",
    )?;
    check_presence(
        flags,
        FLAG_SESSION_PRESENT,
        get_u32(payload, 44) != 0,
        "session",
    )?;
    check_presence(flags, FLAG_SCENARIO_PRESENT, scenario_len != 0, "scenario")?;
    check_presence(flags, FLAG_ROSTER_PRESENT, roster_count != 0, "roster")?;
    check_presence(
        flags,
        FLAG_MODSET_PRESENT,
        modset_len != 0,
        "modset identity",
    )?;

    let expected_total = CID1_FIXED_HEADER_LEN
        .checked_add(uuid_len)
        .and_then(|v| v.checked_add(profile_len))
        .and_then(|v| v.checked_add(scenario_len))
        .and_then(|v| v.checked_add(roster_len))
        .and_then(|v| v.checked_add(modset_len))
        .ok_or(Cid1Error::PayloadTooLarge(total_len))?;
    if expected_total != total_len {
        return Err(Cid1Error::TotalLengthMismatch {
            declared: expected_total,
            actual: total_len,
        });
    }

    let mut cursor = CID1_FIXED_HEADER_LEN;
    let uuid_bytes = take(payload, &mut cursor, uuid_len);
    let profile_bytes = take(payload, &mut cursor, profile_len);
    let scenario_bytes = take(payload, &mut cursor, scenario_len);
    let roster_bytes = take(payload, &mut cursor, roster_len);
    let modset_bytes = take(payload, &mut cursor, modset_len);
    debug_assert_eq!(cursor, payload.len());

    let uuid = core::str::from_utf8(uuid_bytes)
        .map_err(|_| Cid1Error::InvalidUtf8("UUID"))?
        .to_owned();
    validate_canonical_uuid(&uuid)?;
    let profile = core::str::from_utf8(profile_bytes)
        .map_err(|_| Cid1Error::InvalidUtf8("build profile"))?
        .to_owned();
    validate_build_profile(&profile)?;
    let scenario = optional_text(flags, FLAG_SCENARIO_PRESENT, scenario_bytes, "scenario")?;
    let modset = optional_text(flags, FLAG_MODSET_PRESENT, modset_bytes, "modset identity")?;
    let roster = parse_roster(flags, roster_count, roster_bytes)?;

    let campaign_id = campaign_id_digest(&uuid, seed);
    if payload[48..80] != campaign_id {
        return Err(Cid1Error::CampaignDigestMismatch);
    }

    Ok(ParsedCid1 {
        payload: payload.to_vec(),
        payload_sha256: Sha256::digest(payload),
        campaign_id,
        game_unique_id: uuid,
        game_unique_seed: seed,
        game_hour_raw,
        source_build_profile: profile,
        witnesses: Cid1Witnesses {
            start_date_raw: (flags & FLAG_START_DATE_PRESENT != 0).then(|| get_u32(payload, 40)),
            session: (flags & FLAG_SESSION_PRESENT != 0).then(|| get_u32(payload, 44)),
            scenario,
            initial_human_country_tags: roster,
            modset_identity: modset,
        },
    })
}

pub fn parse_cid1_record(
    header: &CompactRecordHeader,
    payload: &[u8],
) -> Result<ValidatedCid1, Cid1Error> {
    if header.sequence != 0 {
        return Err(Cid1Error::WrongRecordSequence(header.sequence));
    }
    if header.domain_id != COMPACT_DOMAIN_STREAM_HEADER {
        return Err(Cid1Error::WrongRecordDomain(header.domain_id));
    }
    if header.record_type != COMPACT_RECORD_TYPE_STREAM_HEADER {
        return Err(Cid1Error::WrongRecordType(header.record_type));
    }
    if header.record_flags != 0 {
        return Err(Cid1Error::RecordFlagsNonZero(header.record_flags));
    }
    if header.payload_len != payload.len() {
        return Err(Cid1Error::RecordPayloadLengthMismatch {
            declared: header.payload_len,
            actual: payload.len(),
        });
    }
    let validated = parse_cid1(payload)?;
    if header.game_hour_raw != validated.game_hour_raw {
        return Err(Cid1Error::RecordHourMismatch {
            record: header.game_hour_raw,
            payload: validated.game_hour_raw,
        });
    }
    Ok(ValidatedCid1 { parsed: validated })
}

pub fn campaign_id_digest(canonical_uuid: &str, seed: i32) -> [u8; 32] {
    let seed = seed.to_string();
    let mut hasher = Sha256::new();
    hasher.update(CID_PREIMAGE_PREFIX);
    hasher.update(canonical_uuid.as_bytes());
    hasher.update(&[0]);
    hasher.update(seed.as_bytes());
    hasher.finalize()
}

pub fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn validate_canonical_uuid(value: &str) -> Result<(), Cid1Error> {
    let bytes = value.as_bytes();
    if bytes.len() != CID1_UUID_LEN {
        return Err(Cid1Error::InvalidCanonicalUuid);
    }
    for (index, &byte) in bytes.iter().enumerate() {
        let hyphen = matches!(index, 8 | 13 | 18 | 23);
        if (hyphen && byte != b'-') || (!hyphen && !matches!(byte, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(Cid1Error::InvalidCanonicalUuid);
        }
    }
    Ok(())
}

fn validate_game_hour(value: i32) -> Result<(), Cid1Error> {
    hour_index_from_raw(value)
        .map(|_| ())
        .ok_or(Cid1Error::InvalidGameHour(value))
}

fn validate_build_profile(value: &str) -> Result<(), Cid1Error> {
    if value.is_empty()
        || value.len() > CID1_MAX_BUILD_PROFILE_LEN
        || !value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
    {
        return Err(Cid1Error::InvalidBuildProfile);
    }
    Ok(())
}

fn validate_optional_text(
    value: &Option<String>,
    max: usize,
    field: &'static str,
) -> Result<(), Cid1Error> {
    if let Some(value) = value {
        if value.is_empty() || value.len() > max || value.chars().any(|ch| ch.is_control()) {
            return Err(Cid1Error::InvalidWitness(field));
        }
    }
    Ok(())
}

fn validate_witnesses(value: &Cid1Witnesses) -> Result<(), Cid1Error> {
    if value.start_date_raw == Some(0) {
        return Err(Cid1Error::InvalidWitness("start date"));
    }
    if value.session == Some(0) {
        return Err(Cid1Error::InvalidWitness("session"));
    }
    validate_optional_text(&value.scenario, CID1_MAX_SCENARIO_LEN, "scenario")?;
    validate_optional_text(
        &value.modset_identity,
        CID1_MAX_MODSET_IDENTITY_LEN,
        "modset identity",
    )?;
    if let Some(tags) = &value.initial_human_country_tags {
        if tags.is_empty() || tags.len() > CID1_MAX_ROSTER_TAGS {
            return Err(Cid1Error::InvalidWitness("roster"));
        }
        let mut previous: Option<&str> = None;
        for tag in tags {
            // and a released or civil-war faction is an ordinary one. See
            if !crate::country_tag::is_engine_country_tag(tag)
                || previous.is_some_and(|prior| prior >= tag.as_str())
            {
                return Err(Cid1Error::InvalidWitness("roster"));
            }
            previous = Some(tag);
        }
    }
    Ok(())
}

fn check_presence(
    flags: u32,
    bit: u32,
    has_nonempty_encoding: bool,
    field: &'static str,
) -> Result<(), Cid1Error> {
    if (flags & bit != 0) != has_nonempty_encoding {
        return Err(Cid1Error::InvalidWitness(field));
    }
    Ok(())
}

fn optional_text(
    flags: u32,
    bit: u32,
    bytes: &[u8],
    field: &'static str,
) -> Result<Option<String>, Cid1Error> {
    if flags & bit == 0 {
        return Ok(None);
    }
    let value = core::str::from_utf8(bytes)
        .map_err(|_| Cid1Error::InvalidUtf8(field))?
        .to_owned();
    if value.chars().any(|ch| ch.is_control()) {
        return Err(Cid1Error::InvalidWitness(field));
    }
    Ok(Some(value))
}

fn parse_roster(flags: u32, count: usize, bytes: &[u8]) -> Result<Option<Vec<String>>, Cid1Error> {
    if flags & FLAG_ROSTER_PRESENT == 0 {
        return Ok(None);
    }
    let mut tags = Vec::with_capacity(count);
    for chunk in bytes.chunks_exact(CID1_ROSTER_TAG_LEN) {
        let tag = core::str::from_utf8(chunk)
            .map_err(|_| Cid1Error::InvalidUtf8("roster"))?
            .to_owned();
        tags.push(tag);
    }
    let witnesses = Cid1Witnesses {
        initial_human_country_tags: Some(tags.clone()),
        ..Cid1Witnesses::default()
    };
    validate_witnesses(&witnesses)?;
    Ok(Some(tags))
}

fn take<'a>(payload: &'a [u8], cursor: &mut usize, len: usize) -> &'a [u8] {
    let start = *cursor;
    *cursor += len;
    &payload[start..*cursor]
}

fn get_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn get_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

const SHA256_H0: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];
const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

pub(crate) struct Sha256 {
    h: [u32; 8],
    len: u64,
    buffer: [u8; 64],
    buffer_len: usize,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            h: SHA256_H0,
            len: 0,
            buffer: [0; 64],
            buffer_len: 0,
        }
    }

    pub(crate) fn digest(bytes: &[u8]) -> [u8; 32] {
        let mut hasher = Self::new();
        hasher.update(bytes);
        hasher.finalize()
    }

    fn update(&mut self, mut bytes: &[u8]) {
        self.len = self.len.wrapping_add(bytes.len() as u64);
        if self.buffer_len != 0 {
            let take = (64 - self.buffer_len).min(bytes.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&bytes[..take]);
            self.buffer_len += take;
            bytes = &bytes[take..];
            if self.buffer_len == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffer_len = 0;
            }
        }
        while bytes.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&bytes[..64]);
            self.compress(&block);
            bytes = &bytes[64..];
        }
        if !bytes.is_empty() {
            self.buffer[..bytes.len()].copy_from_slice(bytes);
            self.buffer_len = bytes.len();
        }
    }

    fn update_without_length(&mut self, mut bytes: &[u8]) {
        if self.buffer_len != 0 {
            let take = (64 - self.buffer_len).min(bytes.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&bytes[..take]);
            self.buffer_len += take;
            bytes = &bytes[take..];
            if self.buffer_len == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffer_len = 0;
            }
        }
        while bytes.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&bytes[..64]);
            self.compress(&block);
            bytes = &bytes[64..];
        }
        if !bytes.is_empty() {
            self.buffer[..bytes.len()].copy_from_slice(bytes);
            self.buffer_len = bytes.len();
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.len.wrapping_mul(8);
        let mut padding = [0u8; 72];
        padding[0] = 0x80;
        let remainder = (self.buffer_len + 1) % 64;
        let zero_count = if remainder <= 56 {
            56 - remainder
        } else {
            120 - remainder
        };
        self.update_without_length(&padding[..1 + zero_count]);
        self.update_without_length(&bit_len.to_be_bytes());
        let mut output = [0u8; 32];
        for (index, word) in self.h.iter().enumerate() {
            output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        output
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut words = [0u32; 64];
        for index in 0..16 {
            words[index] = u32::from_be_bytes([
                block[index * 4],
                block[index * 4 + 1],
                block[index * 4 + 2],
                block[index * 4 + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let mut a = self.h[0];
        let mut b = self.h[1];
        let mut c = self.h[2];
        let mut d = self.h[3];
        let mut e = self.h[4];
        let mut f = self.h[5];
        let mut g = self.h[6];
        let mut h = self.h[7];
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(SHA256_K[index])
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        self.h[0] = self.h[0].wrapping_add(a);
        self.h[1] = self.h[1].wrapping_add(b);
        self.h[2] = self.h[2].wrapping_add(c);
        self.h[3] = self.h[3].wrapping_add(d);
        self.h[4] = self.h[4].wrapping_add(e);
        self.h[5] = self.h[5].wrapping_add(f);
        self.h[6] = self.h[6].wrapping_add(g);
        self.h[7] = self.h[7].wrapping_add(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::DATE_EPOCH_CONST;

    const UUID: &str = "a4b17fed-ed12-4361-9064-fb6576638110";
    const SEED: i32 = 1_026_046_527;

    fn hour() -> i32 {
        DATE_EPOCH_CONST + 1944 * 365 * 24
    }

    fn observation() -> Cid1Observation {
        Cid1Observation {
            game_unique_id: UUID.to_owned(),
            game_unique_seed: SEED,
            game_hour_raw: hour(),
        }
    }

    fn fields() -> Cid1Fields {
        Cid1Fields::from_stable_observations(
            observation(),
            observation(),
            "vanilla_latest_2026_06_30".to_owned(),
            Cid1Witnesses {
                start_date_raw: Some(123),
                session: Some(7),
                scenario: Some("1936".to_owned()),
                initial_human_country_tags: Some(vec!["ENG".to_owned(), "GER".to_owned()]),
                modset_identity: Some("sha256:abc".to_owned()),
            },
        )
        .unwrap()
    }

    #[test]
    fn d04_digest_matches_design() {
        assert_eq!(
            lower_hex(&campaign_id_digest(UUID, SEED)),
            "b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f"
        );
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            lower_hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn exact_layout_roundtrips_and_recomputes_hashes() {
        let payload = encode_cid1(&fields()).unwrap();
        let parsed = parse_cid1(&payload).unwrap();
        assert_eq!(&payload[0..4], CID1_MAGIC);
        assert_eq!(get_u16(&payload, 6) as usize, CID1_FIXED_HEADER_LEN);
        assert_eq!(get_u32(&payload, 8) as usize, payload.len());
        assert_eq!(get_i32(&payload, 20), SEED);
        assert_eq!(get_u16(&payload, 28) as usize, CID1_UUID_LEN);
        assert_eq!(parsed.game_unique_id(), UUID);
        assert_eq!(parsed.game_unique_seed(), SEED);
        assert_eq!(parsed.game_hour_raw(), hour());
        assert_eq!(parsed.campaign_id_hex(), lower_hex(&payload[48..80]));
        assert_eq!(parsed.payload_sha256(), Sha256::digest(&payload));
        assert_eq!(parsed.witnesses(), &fields().witnesses);
    }

    #[test]
    fn signed_i32_extremes_roundtrip() {
        for seed in [i32::MIN, -1, 0, i32::MAX] {
            let mut first = observation();
            first.game_unique_seed = seed;
            let fields = Cid1Fields::from_stable_observations(
                first.clone(),
                first,
                "profile".to_owned(),
                Cid1Witnesses::default(),
            )
            .unwrap();
            let parsed = parse_cid1(&encode_cid1(&fields).unwrap()).unwrap();
            assert_eq!(parsed.game_unique_seed(), seed);
        }
    }

    #[test]
    fn encoder_requires_two_equal_complete_identity_observations() {
        let mut second = observation();
        second.game_unique_seed += 1;
        assert_eq!(
            Cid1Fields::from_stable_observations(
                observation(),
                second,
                "profile".to_owned(),
                Cid1Witnesses::default(),
            ),
            Err(Cid1Error::ObservationMismatch)
        );
        let mut second = observation();
        second.game_hour_raw += 1;
        let fields = Cid1Fields::from_stable_observations(
            observation(),
            second.clone(),
            "profile".to_owned(),
            Cid1Witnesses::default(),
        )
        .unwrap();
        let parsed = parse_cid1(&encode_cid1(&fields).unwrap()).unwrap();
        assert_eq!(parsed.game_hour_raw(), second.game_hour_raw);
    }

    #[test]
    fn canonical_uuid_and_profile_are_closed() {
        let mut uppercase = observation();
        uppercase.game_unique_id = UUID.to_ascii_uppercase();
        assert!(matches!(
            Cid1Fields::from_stable_observations(
                uppercase.clone(),
                uppercase,
                "profile".to_owned(),
                Cid1Witnesses::default(),
            ),
            Err(Cid1Error::InvalidCanonicalUuid)
        ));
        assert!(matches!(
            Cid1Fields::from_stable_observations(
                observation(),
                observation(),
                "bad profile".to_owned(),
                Cid1Witnesses::default(),
            ),
            Err(Cid1Error::InvalidBuildProfile)
        ));
    }

    #[test]
    fn parser_rejects_digest_unknown_flags_reserved_and_trailing_bytes() {
        let base = encode_cid1(&fields()).unwrap();
        let mut digest = base.clone();
        digest[48] ^= 1;
        assert_eq!(parse_cid1(&digest), Err(Cid1Error::CampaignDigestMismatch));

        let mut flags = base.clone();
        put_u32(&mut flags, 12, get_u32(&base, 12) | (1 << 31));
        assert_eq!(parse_cid1(&flags), Err(Cid1Error::UnknownFlags(1 << 31)));

        let mut reserved = base.clone();
        reserved[80] = 1;
        assert_eq!(parse_cid1(&reserved), Err(Cid1Error::ReservedNonZero));

        let mut trailing = base;
        trailing.push(0);
        assert!(matches!(
            parse_cid1(&trailing),
            Err(Cid1Error::TotalLengthMismatch { .. })
        ));
    }

    #[test]
    fn parser_rejects_missing_proof_and_presence_mismatch() {
        let base = encode_cid1(&fields()).unwrap();
        let mut proof = base.clone();
        put_u32(
            &mut proof,
            12,
            get_u32(&base, 12) & !FLAG_SECOND_OBSERVATION_COMPLETE,
        );
        assert_eq!(parse_cid1(&proof), Err(Cid1Error::MissingStableReadProof));

        let mut presence = base;
        let presence_flags = get_u32(&presence, 12) & !FLAG_SCENARIO_PRESENT;
        put_u32(&mut presence, 12, presence_flags);
        assert_eq!(
            parse_cid1(&presence),
            Err(Cid1Error::InvalidWitness("scenario"))
        );
    }

    #[test]
    fn roster_is_sorted_unique_uppercase() {
        for tags in [
            vec!["GER".to_owned(), "ENG".to_owned()],
            vec!["GER".to_owned(), "GER".to_owned()],
            vec!["ger".to_owned()],
        ] {
            let result = Cid1Fields::from_stable_observations(
                observation(),
                observation(),
                "profile".to_owned(),
                Cid1Witnesses {
                    initial_human_country_tags: Some(tags),
                    ..Cid1Witnesses::default()
                },
            );
            assert_eq!(result, Err(Cid1Error::InvalidWitness("roster")));
        }
    }

    #[test]
    fn h4cr_context_is_part_of_validation() {
        let payload = encode_cid1(&fields()).unwrap();
        let header = CompactRecordHeader {
            sequence: 0,
            game_hour_raw: hour(),
            domain_id: COMPACT_DOMAIN_STREAM_HEADER,
            record_type: COMPACT_RECORD_TYPE_STREAM_HEADER,
            payload_len: payload.len(),
            record_flags: 0,
        };
        parse_cid1_record(&header, &payload).unwrap();
        let mut wrong = header;
        wrong.sequence = 1;
        assert_eq!(
            parse_cid1_record(&wrong, &payload),
            Err(Cid1Error::WrongRecordSequence(1))
        );
        let mut wrong = header;
        wrong.game_hour_raw += 1;
        assert!(matches!(
            parse_cid1_record(&wrong, &payload),
            Err(Cid1Error::RecordHourMismatch { .. })
        ));
    }
}
