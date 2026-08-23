use sha2::{Digest, Sha256};

pub const PAYLOAD_SCHEMA: u16 = 1;

const PREFIX_LEN: usize = 4;

pub const RECORDER_HELLO_LEN: usize = 68;
pub const COLLECTOR_HELLO_LEN: usize = 68;
pub const COLLECTOR_HELLO_WITH_CAPABILITIES_LEN: usize = 72;
pub const HEARTBEAT_CHALLENGE_LEN: usize = 28;
pub const HEARTBEAT_PROOF_LEN: usize = 32;
pub const FIRST_COMPACT_FRAME_ACK_LEN: usize = 56;
pub const CAPTURE_READY_MAGIC: &[u8; 4] = b"H4RD";
pub const CAPTURE_READY_ACK_MAGIC: &[u8; 4] = b"H4RA";
pub const CAPTURE_READY_VERSION: u16 = 2;
pub const CAPTURE_READY_LEN: usize = 512;
pub const CAPTURE_READY_ACK_LEN: usize = 168;

const CAPTURE_READY_AUTH_OFFSET: usize = CAPTURE_READY_LEN - 32;
const CAPTURE_READY_ACK_AUTH_OFFSET: usize = CAPTURE_READY_ACK_LEN - 32;

pub const HEARTBEAT_STATE_WAITING_FOR_WORLD: u16 = 1;
pub const HEARTBEAT_STATE_PRODUCING: u16 = 2;
pub const HEARTBEAT_STATE_PAUSED_FOR_CONTROL: u16 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadError {
    WrongLength { expected: usize, actual: usize },
    UnsupportedSchema(u16),
    NonZeroReserved,
    ZeroChallenge,
    UnknownHeartbeatState(u16),
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidDeclaredLength,
    UnknownStatusFlags(u32),
    UnknownCollectorFeatureBits(u32),
    UndeclaredCollectorFeatureBits(u32),
    MissingRequiredStatusFlags { required: u32, observed: u32 },
    ZeroRequiredField(&'static str),
    InvalidGameHour(i32),
    HookCountExceedsAllowlist { active: u16, allowed: u16 },
    UncountedInstallFreezes { active: u16, import_only: u16 },
    ProductionHookContractSkew { declared: u16, required: u16 },
    ExhaustedOrdinal(&'static str),
    EmptyAuthenticationToken,
    AuthenticationFailed,
    InvalidPinnedProductionContract,
}

impl core::fmt::Display for PayloadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongLength { expected, actual } => {
                write!(f, "H4CT payload length {actual} != expected {expected}")
            }
            Self::UnsupportedSchema(schema) => {
                write!(f, "unsupported H4CT payload schema {schema}")
            }
            Self::NonZeroReserved => write!(f, "nonzero reserved H4CT payload field"),
            Self::ZeroChallenge => write!(f, "all-zero H4CT challenge/id is invalid"),
            Self::UnknownHeartbeatState(state) => {
                write!(f, "unknown H4CT heartbeat state {state}")
            }
            Self::InvalidMagic => write!(f, "invalid capture-ready payload magic"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported capture-ready payload version {version}")
            }
            Self::InvalidDeclaredLength => {
                write!(f, "capture-ready payload declared a noncanonical length")
            }
            Self::UnknownStatusFlags(flags) => {
                write!(
                    f,
                    "capture-ready payload has unknown status flags {flags:#x}"
                )
            }
            Self::UnknownCollectorFeatureBits(bits) => write!(
                f,
                "capture-ready payload declares unknown collector feature bits {bits:#x}"
            ),
            Self::UndeclaredCollectorFeatureBits(bits) => write!(
                f,
                "capture-ready payload collector feature bits {bits:#x} omit the declaration bit"
            ),
            Self::MissingRequiredStatusFlags { required, observed } => write!(
                f,
                "capture-ready payload flags {observed:#x} omit required set {required:#x}"
            ),
            Self::ZeroRequiredField(field) => {
                write!(f, "capture-ready payload has zero required field {field}")
            }
            Self::InvalidGameHour(hour) => {
                write!(f, "capture-ready payload has invalid game hour {hour}")
            }
            Self::HookCountExceedsAllowlist { active, allowed } => write!(
                f,
                "capture-ready active hook count {active} exceeds allowlist count {allowed}"
            ),
            Self::UncountedInstallFreezes {
                active,
                import_only,
            } => write!(
                f,
                "capture-ready reports {active} active hooks with an install freeze count of \
                 zero; only the {import_only} bootstrap import slots can be patched without \
                 stopping the process, so at least one inline detour was installed under a \
                 freeze this receipt did not count -- the collector's freeze counter is not \
                 wired to its READY payload"
            ),
            Self::ProductionHookContractSkew { declared, required } => write!(
                f,
                "capture-ready collector declares a pinned production hook roster of {declared}, \
                 but this build's pinned contract has {required}; the loaded collector DLL was \
                 built from a different revision of PINNED_PRODUCTION_HOOK_SPECS_V1 -- rebuild \
                 collector.dll against this source tree (this is a build-freshness fault, not a \
                 hook install failure)"
            ),
            Self::ExhaustedOrdinal(field) => {
                write!(f, "capture-ready ordinal {field} is exhausted")
            }
            Self::EmptyAuthenticationToken => {
                write!(f, "capture-ready authentication token is empty")
            }
            Self::AuthenticationFailed => {
                write!(f, "capture-ready HMAC-SHA256 authentication failed")
            }
            Self::InvalidPinnedProductionContract => {
                write!(f, "capture-ready pinned production contract is invalid")
            }
        }
    }
}

impl std::error::Error for PayloadError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecorderHelloV1 {
    pub expected_pid: u32,
    pub expected_creation_filetime: u64,
    pub h4cr_version: u16,
    pub expected_dll_sha256: [u8; 32],
    pub challenge: [u8; 16],
}

pub fn encode_recorder_hello(fields: &RecorderHelloV1) -> Vec<u8> {
    let mut payload = Vec::with_capacity(RECORDER_HELLO_LEN);
    push_prefix(&mut payload);
    payload.extend_from_slice(&fields.expected_pid.to_le_bytes());
    payload.extend_from_slice(&fields.expected_creation_filetime.to_le_bytes());
    payload.extend_from_slice(&fields.h4cr_version.to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&fields.expected_dll_sha256);
    payload.extend_from_slice(&fields.challenge);
    debug_assert_eq!(payload.len(), RECORDER_HELLO_LEN);
    payload
}

pub fn decode_recorder_hello(payload: &[u8]) -> Result<RecorderHelloV1, PayloadError> {
    check_prefix(payload, RECORDER_HELLO_LEN)?;
    if read_u16(payload, PREFIX_LEN + 14) != 0 {
        return Err(PayloadError::NonZeroReserved);
    }
    let challenge = read_bytes16(payload, PREFIX_LEN + 48);
    if challenge == [0; 16] {
        return Err(PayloadError::ZeroChallenge);
    }
    Ok(RecorderHelloV1 {
        expected_pid: read_u32(payload, PREFIX_LEN),
        expected_creation_filetime: read_u64(payload, PREFIX_LEN + 4),
        h4cr_version: read_u16(payload, PREFIX_LEN + 12),
        expected_dll_sha256: read_bytes32(payload, PREFIX_LEN + 16),
        challenge,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CollectorHelloV1 {
    pub pid: u32,
    pub creation_filetime: u64,
    pub h4cr_version: u16,
    pub control_version: u16,
    pub dll_sha256: [u8; 32],
    pub challenge: [u8; 16],
}

pub fn encode_collector_hello(fields: &CollectorHelloV1) -> Vec<u8> {
    let mut payload = Vec::with_capacity(COLLECTOR_HELLO_LEN);
    push_prefix(&mut payload);
    payload.extend_from_slice(&fields.pid.to_le_bytes());
    payload.extend_from_slice(&fields.creation_filetime.to_le_bytes());
    payload.extend_from_slice(&fields.h4cr_version.to_le_bytes());
    payload.extend_from_slice(&fields.control_version.to_le_bytes());
    payload.extend_from_slice(&fields.dll_sha256);
    payload.extend_from_slice(&fields.challenge);
    debug_assert_eq!(payload.len(), COLLECTOR_HELLO_LEN);
    payload
}

pub fn decode_collector_hello(payload: &[u8]) -> Result<CollectorHelloV1, PayloadError> {
    check_prefix(payload, COLLECTOR_HELLO_LEN)?;
    let challenge = read_bytes16(payload, PREFIX_LEN + 48);
    if challenge == [0; 16] {
        return Err(PayloadError::ZeroChallenge);
    }
    Ok(CollectorHelloV1 {
        pid: read_u32(payload, PREFIX_LEN),
        creation_filetime: read_u64(payload, PREFIX_LEN + 4),
        h4cr_version: read_u16(payload, PREFIX_LEN + 12),
        control_version: read_u16(payload, PREFIX_LEN + 14),
        dll_sha256: read_bytes32(payload, PREFIX_LEN + 16),
        challenge,
    })
}

pub fn encode_collector_hello_with_capabilities(
    fields: &CollectorHelloV1,
    capability_bits: u32,
) -> Vec<u8> {
    let mut payload = encode_collector_hello(fields);
    payload.reserve_exact(COLLECTOR_HELLO_WITH_CAPABILITIES_LEN - payload.len());
    payload.extend_from_slice(&capability_bits.to_le_bytes());
    debug_assert_eq!(payload.len(), COLLECTOR_HELLO_WITH_CAPABILITIES_LEN);
    payload
}

pub fn decode_collector_hello_with_capabilities(
    payload: &[u8],
) -> Result<(CollectorHelloV1, u32), PayloadError> {
    if payload.len() != COLLECTOR_HELLO_WITH_CAPABILITIES_LEN {
        return Err(PayloadError::WrongLength {
            expected: COLLECTOR_HELLO_WITH_CAPABILITIES_LEN,
            actual: payload.len(),
        });
    }
    let hello = decode_collector_hello(&payload[..COLLECTOR_HELLO_LEN])?;
    let bits = read_u32(payload, COLLECTOR_HELLO_LEN);
    Ok((hello, bits))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeartbeatChallengeV1 {
    pub challenge: [u8; 16],
    pub host_challenge_counter: u64,
}

pub fn encode_heartbeat_challenge(fields: &HeartbeatChallengeV1) -> Vec<u8> {
    let mut payload = Vec::with_capacity(HEARTBEAT_CHALLENGE_LEN);
    push_prefix(&mut payload);
    payload.extend_from_slice(&fields.challenge);
    payload.extend_from_slice(&fields.host_challenge_counter.to_le_bytes());
    debug_assert_eq!(payload.len(), HEARTBEAT_CHALLENGE_LEN);
    payload
}

pub fn decode_heartbeat_challenge(payload: &[u8]) -> Result<HeartbeatChallengeV1, PayloadError> {
    check_prefix(payload, HEARTBEAT_CHALLENGE_LEN)?;
    let challenge = read_bytes16(payload, PREFIX_LEN);
    if challenge == [0; 16] {
        return Err(PayloadError::ZeroChallenge);
    }
    Ok(HeartbeatChallengeV1 {
        challenge,
        host_challenge_counter: read_u64(payload, PREFIX_LEN + 16),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeartbeatProofV1 {
    pub challenge: [u8; 16],
    pub last_h4cr_sequence: u64,
    pub state: u16,
}

pub fn encode_heartbeat_proof(fields: &HeartbeatProofV1) -> Vec<u8> {
    let mut payload = Vec::with_capacity(HEARTBEAT_PROOF_LEN);
    push_prefix(&mut payload);
    payload.extend_from_slice(&fields.challenge);
    payload.extend_from_slice(&fields.last_h4cr_sequence.to_le_bytes());
    payload.extend_from_slice(&fields.state.to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    debug_assert_eq!(payload.len(), HEARTBEAT_PROOF_LEN);
    payload
}

pub fn decode_heartbeat_proof(payload: &[u8]) -> Result<HeartbeatProofV1, PayloadError> {
    check_prefix(payload, HEARTBEAT_PROOF_LEN)?;
    if read_u16(payload, PREFIX_LEN + 26) != 0 {
        return Err(PayloadError::NonZeroReserved);
    }
    let challenge = read_bytes16(payload, PREFIX_LEN);
    if challenge == [0; 16] {
        return Err(PayloadError::ZeroChallenge);
    }
    let state = read_u16(payload, PREFIX_LEN + 24);
    if !matches!(
        state,
        HEARTBEAT_STATE_WAITING_FOR_WORLD
            | HEARTBEAT_STATE_PRODUCING
            | HEARTBEAT_STATE_PAUSED_FOR_CONTROL
    ) {
        return Err(PayloadError::UnknownHeartbeatState(state));
    }
    Ok(HeartbeatProofV1 {
        challenge,
        last_h4cr_sequence: read_u64(payload, PREFIX_LEN + 16),
        state,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirstCompactFrameAckV1 {
    pub h4cr_sequence: u64,
    pub game_hour: i32,
    pub record_type: u16,
    pub total_record_len: u32,
    pub record_sha256: [u8; 32],
}

pub fn encode_first_compact_frame_ack(fields: &FirstCompactFrameAckV1) -> Vec<u8> {
    let mut payload = Vec::with_capacity(FIRST_COMPACT_FRAME_ACK_LEN);
    push_prefix(&mut payload);
    payload.extend_from_slice(&fields.h4cr_sequence.to_le_bytes());
    payload.extend_from_slice(&fields.game_hour.to_le_bytes());
    payload.extend_from_slice(&fields.record_type.to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&fields.total_record_len.to_le_bytes());
    payload.extend_from_slice(&fields.record_sha256);
    debug_assert_eq!(payload.len(), FIRST_COMPACT_FRAME_ACK_LEN);
    payload
}

pub fn decode_first_compact_frame_ack(
    payload: &[u8],
) -> Result<FirstCompactFrameAckV1, PayloadError> {
    check_prefix(payload, FIRST_COMPACT_FRAME_ACK_LEN)?;
    if read_u16(payload, PREFIX_LEN + 14) != 0 {
        return Err(PayloadError::NonZeroReserved);
    }
    Ok(FirstCompactFrameAckV1 {
        h4cr_sequence: read_u64(payload, PREFIX_LEN),
        game_hour: read_i32(payload, PREFIX_LEN + 8),
        record_type: read_u16(payload, PREFIX_LEN + 12),
        total_record_len: read_u32(payload, PREFIX_LEN + 16),
        record_sha256: read_bytes32(payload, PREFIX_LEN + 20),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureReadyFlags(u32);

impl CaptureReadyFlags {
    pub const TARGET_IDENTITY_VALIDATED: u32 = 1 << 0;
    pub const DLL_IDENTITY_VALIDATED: u32 = 1 << 1;
    pub const HOOK_ALLOWLIST_VALIDATED: u32 = 1 << 2;
    pub const DOMAIN_ROSTER_VALIDATED: u32 = 1 << 3;
    pub const READ_ONLY_CLOCK_VALIDATED: u32 = 1 << 4;
    pub const BOUNDED_SCHEDULER_ALLOCATED: u32 = 1 << 5;
    pub const OUTPUT_PIPE_CONNECTED: u32 = 1 << 6;
    pub const ZERO_MUTATION_POLICY_ENFORCED: u32 = 1 << 7;
    pub const IDENTITY_V1_STREAM: u32 = 1 << 8;
    pub const REQUIRED_PASSIVE_CAPTURE: u32 = Self::TARGET_IDENTITY_VALIDATED
        | Self::DLL_IDENTITY_VALIDATED
        | Self::HOOK_ALLOWLIST_VALIDATED
        | Self::DOMAIN_ROSTER_VALIDATED
        | Self::READ_ONLY_CLOCK_VALIDATED
        | Self::BOUNDED_SCHEDULER_ALLOCATED
        | Self::OUTPUT_PIPE_CONNECTED
        | Self::ZERO_MUTATION_POLICY_ENFORCED;
    pub const KNOWN_MASK: u32 = Self::REQUIRED_PASSIVE_CAPTURE | Self::IDENTITY_V1_STREAM;

    pub fn from_bits(bits: u32) -> Result<Self, PayloadError> {
        let unknown = bits & !Self::KNOWN_MASK;
        if unknown != 0 {
            Err(PayloadError::UnknownStatusFlags(unknown))
        } else {
            Ok(Self(bits))
        }
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, required: u32) -> bool {
        self.0 & required == required
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CollectorFeatureBits(u32);

impl CollectorFeatureBits {
    pub const DECLARED: u32 = 1 << 0;
    pub const TEST_SUPPORT: u32 = 1 << 1;
    pub const RANGE_TRACER: u32 = 1 << 2;
    pub const KNOWN_MASK: u32 = Self::DECLARED | Self::TEST_SUPPORT | Self::RANGE_TRACER;

    pub const DIAGNOSTIC_MASK: u32 = Self::TEST_SUPPORT | Self::RANGE_TRACER;

    pub const FEATURE_NAMES: &'static [(u32, &'static str)] = &[
        (Self::TEST_SUPPORT, "test-support"),
        (Self::RANGE_TRACER, "range-tracer"),
    ];

    pub const fn undeclared() -> Self {
        Self(0)
    }

    pub fn from_bits(bits: u32) -> Result<Self, PayloadError> {
        if bits == 0 {
            return Ok(Self(0));
        }
        let unknown = bits & !Self::KNOWN_MASK;
        if unknown != 0 {
            return Err(PayloadError::UnknownCollectorFeatureBits(unknown));
        }
        if bits & Self::DECLARED == 0 {
            return Err(PayloadError::UndeclaredCollectorFeatureBits(bits));
        }
        Ok(Self(bits))
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn declared(self) -> bool {
        self.0 & Self::DECLARED != 0
    }

    pub const fn diagnostic_build(self) -> Option<bool> {
        if self.declared() {
            Some(self.0 & Self::DIAGNOSTIC_MASK != 0)
        } else {
            None
        }
    }

    pub fn feature_names(self) -> Option<Vec<&'static str>> {
        if !self.declared() {
            return None;
        }
        Some(
            Self::FEATURE_NAMES
                .iter()
                .filter(|(bit, _)| self.0 & bit != 0)
                .map(|(_, name)| *name)
                .collect(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureReadyV2 {
    pub flags: CaptureReadyFlags,
    pub pid: u32,
    pub collector_features: CollectorFeatureBits,
    pub process_creation_filetime_utc: u64,
    pub requested_duration_ms: u32,
    pub validated_domain_count: u32,
    pub scheduler_queue_capacity: u32,
    pub scheduler_worker_count: u16,
    pub allowed_hook_count: u16,
    pub active_hook_count: u16,
    pub install_freeze_count: u16,
    pub clock_hour_raw: i32,
    pub clock_observation_ordinal: u64,
    pub target_executable_sha256: [u8; 32],
    pub collector_dll_sha256: [u8; 32],
    pub hook_allowlist_sha256: [u8; 32],
    pub hook_outcomes_sha256: [u8; 32],
    pub validated_domain_roster_sha256: [u8; 32],
    pub clock_witness_sha256: [u8; 32],
    pub scheduler_config_sha256: [u8; 32],
    pub output_pipe_identity_sha256: [u8; 32],
    pub pipe_token_sha256: [u8; 32],
    pub zero_mutation_policy_sha256: [u8; 32],
    pub session_id_sha256: [u8; 32],
    pub capture_id_sha256: [u8; 32],
    pub control_sha256: [u8; 32],
}

impl CaptureReadyV2 {
    pub fn encode_authenticated(&self, token: &[u8]) -> Result<Vec<u8>, PayloadError> {
        validate_ready(self)?;
        if token.is_empty() {
            return Err(PayloadError::EmptyAuthenticationToken);
        }
        validate_ready_token_binding(self, token)?;
        let mut out = Vec::with_capacity(CAPTURE_READY_LEN);
        out.extend_from_slice(CAPTURE_READY_MAGIC);
        out.extend_from_slice(&CAPTURE_READY_VERSION.to_le_bytes());
        out.extend_from_slice(&(CAPTURE_READY_LEN as u16).to_le_bytes());
        out.extend_from_slice(&(CAPTURE_READY_LEN as u32).to_le_bytes());
        out.extend_from_slice(&self.flags.bits().to_le_bytes());
        out.extend_from_slice(&self.pid.to_le_bytes());
        out.extend_from_slice(&self.collector_features.bits().to_le_bytes());
        out.extend_from_slice(&self.process_creation_filetime_utc.to_le_bytes());
        out.extend_from_slice(&self.requested_duration_ms.to_le_bytes());
        out.extend_from_slice(&self.validated_domain_count.to_le_bytes());
        out.extend_from_slice(&self.scheduler_queue_capacity.to_le_bytes());
        out.extend_from_slice(&self.scheduler_worker_count.to_le_bytes());
        out.extend_from_slice(&self.allowed_hook_count.to_le_bytes());
        out.extend_from_slice(&self.active_hook_count.to_le_bytes());
        out.extend_from_slice(&self.install_freeze_count.to_le_bytes());
        out.extend_from_slice(&self.clock_hour_raw.to_le_bytes());
        out.extend_from_slice(&self.clock_observation_ordinal.to_le_bytes());
        out.extend_from_slice(&self.target_executable_sha256);
        out.extend_from_slice(&self.collector_dll_sha256);
        out.extend_from_slice(&self.hook_allowlist_sha256);
        out.extend_from_slice(&self.hook_outcomes_sha256);
        out.extend_from_slice(&self.validated_domain_roster_sha256);
        out.extend_from_slice(&self.clock_witness_sha256);
        out.extend_from_slice(&self.scheduler_config_sha256);
        out.extend_from_slice(&self.output_pipe_identity_sha256);
        out.extend_from_slice(&self.pipe_token_sha256);
        out.extend_from_slice(&self.zero_mutation_policy_sha256);
        out.extend_from_slice(&self.session_id_sha256);
        out.extend_from_slice(&self.capture_id_sha256);
        out.extend_from_slice(&self.control_sha256);
        debug_assert_eq!(out.len(), CAPTURE_READY_AUTH_OFFSET);
        let tag = hmac_sha256(token, &out);
        out.extend_from_slice(&tag);
        debug_assert_eq!(out.len(), CAPTURE_READY_LEN);
        Ok(out)
    }

    pub fn decode_authenticated(bytes: &[u8], token: &[u8]) -> Result<Self, PayloadError> {
        check_capture_length(bytes, CAPTURE_READY_LEN)?;
        if token.is_empty() {
            return Err(PayloadError::EmptyAuthenticationToken);
        }
        verify_hmac(
            token,
            &bytes[..CAPTURE_READY_AUTH_OFFSET],
            &bytes[CAPTURE_READY_AUTH_OFFSET..],
        )?;
        check_capture_prefix(bytes, CAPTURE_READY_MAGIC, CAPTURE_READY_LEN)?;
        let value = Self {
            flags: CaptureReadyFlags::from_bits(read_u32(bytes, 12))?,
            pid: read_u32(bytes, 16),
            collector_features: CollectorFeatureBits::from_bits(read_u32(bytes, 20))?,
            process_creation_filetime_utc: read_u64(bytes, 24),
            requested_duration_ms: read_u32(bytes, 32),
            validated_domain_count: read_u32(bytes, 36),
            scheduler_queue_capacity: read_u32(bytes, 40),
            scheduler_worker_count: read_u16(bytes, 44),
            allowed_hook_count: read_u16(bytes, 46),
            active_hook_count: read_u16(bytes, 48),
            install_freeze_count: read_u16(bytes, 50),
            clock_hour_raw: read_i32(bytes, 52),
            clock_observation_ordinal: read_u64(bytes, 56),
            target_executable_sha256: read_bytes32(bytes, 64),
            collector_dll_sha256: read_bytes32(bytes, 96),
            hook_allowlist_sha256: read_bytes32(bytes, 128),
            hook_outcomes_sha256: read_bytes32(bytes, 160),
            validated_domain_roster_sha256: read_bytes32(bytes, 192),
            clock_witness_sha256: read_bytes32(bytes, 224),
            scheduler_config_sha256: read_bytes32(bytes, 256),
            output_pipe_identity_sha256: read_bytes32(bytes, 288),
            pipe_token_sha256: read_bytes32(bytes, 320),
            zero_mutation_policy_sha256: read_bytes32(bytes, 352),
            session_id_sha256: read_bytes32(bytes, 384),
            capture_id_sha256: read_bytes32(bytes, 416),
            control_sha256: read_bytes32(bytes, 448),
        };
        validate_ready(&value)?;
        validate_ready_token_binding(&value, token)?;
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureReadyAckFlags(u32);

impl CaptureReadyAckFlags {
    pub const HOST_VALIDATED: u32 = 1 << 0;
    pub const KNOWN_MASK: u32 = Self::HOST_VALIDATED;

    pub fn from_bits(bits: u32) -> Result<Self, PayloadError> {
        let unknown = bits & !Self::KNOWN_MASK;
        if unknown != 0 {
            Err(PayloadError::UnknownStatusFlags(unknown))
        } else {
            Ok(Self(bits))
        }
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn host_validated(self) -> bool {
        self.0 & Self::HOST_VALIDATED != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureReadyAckV2 {
    pub flags: CaptureReadyAckFlags,
    pub pid: u32,
    pub process_creation_filetime_utc: u64,
    pub requested_duration_ms: u32,
    pub validated_domain_count: u32,
    pub ready_sha256: [u8; 32],
    pub public_receipt_core_sha256: [u8; 32],
    pub session_id_sha256: [u8; 32],
}

impl CaptureReadyAckV2 {
    pub fn encode_authenticated(&self, token: &[u8]) -> Result<Vec<u8>, PayloadError> {
        validate_ready_ack(self)?;
        if token.is_empty() {
            return Err(PayloadError::EmptyAuthenticationToken);
        }
        let mut out = Vec::with_capacity(CAPTURE_READY_ACK_LEN);
        out.extend_from_slice(CAPTURE_READY_ACK_MAGIC);
        out.extend_from_slice(&CAPTURE_READY_VERSION.to_le_bytes());
        out.extend_from_slice(&(CAPTURE_READY_ACK_LEN as u16).to_le_bytes());
        out.extend_from_slice(&(CAPTURE_READY_ACK_LEN as u32).to_le_bytes());
        out.extend_from_slice(&self.flags.bits().to_le_bytes());
        out.extend_from_slice(&self.pid.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&self.process_creation_filetime_utc.to_le_bytes());
        out.extend_from_slice(&self.requested_duration_ms.to_le_bytes());
        out.extend_from_slice(&self.validated_domain_count.to_le_bytes());
        out.extend_from_slice(&self.ready_sha256);
        out.extend_from_slice(&self.public_receipt_core_sha256);
        out.extend_from_slice(&self.session_id_sha256);
        debug_assert_eq!(out.len(), CAPTURE_READY_ACK_AUTH_OFFSET);
        let tag = hmac_sha256(token, &out);
        out.extend_from_slice(&tag);
        debug_assert_eq!(out.len(), CAPTURE_READY_ACK_LEN);
        Ok(out)
    }

    pub fn decode_authenticated(bytes: &[u8], token: &[u8]) -> Result<Self, PayloadError> {
        check_capture_length(bytes, CAPTURE_READY_ACK_LEN)?;
        if token.is_empty() {
            return Err(PayloadError::EmptyAuthenticationToken);
        }
        verify_hmac(
            token,
            &bytes[..CAPTURE_READY_ACK_AUTH_OFFSET],
            &bytes[CAPTURE_READY_ACK_AUTH_OFFSET..],
        )?;
        check_capture_prefix(bytes, CAPTURE_READY_ACK_MAGIC, CAPTURE_READY_ACK_LEN)?;
        if read_u32(bytes, 20) != 0 {
            return Err(PayloadError::NonZeroReserved);
        }
        let value = Self {
            flags: CaptureReadyAckFlags::from_bits(read_u32(bytes, 12))?,
            pid: read_u32(bytes, 16),
            process_creation_filetime_utc: read_u64(bytes, 24),
            requested_duration_ms: read_u32(bytes, 32),
            validated_domain_count: read_u32(bytes, 36),
            ready_sha256: read_bytes32(bytes, 40),
            public_receipt_core_sha256: read_bytes32(bytes, 72),
            session_id_sha256: read_bytes32(bytes, 104),
        };
        validate_ready_ack(&value)?;
        Ok(value)
    }
}

pub fn capture_ready_payload_sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn validate_ready(value: &CaptureReadyV2) -> Result<(), PayloadError> {
    CaptureReadyFlags::from_bits(value.flags.bits())?;
    CollectorFeatureBits::from_bits(value.collector_features.bits())?;
    if !value
        .flags
        .contains(CaptureReadyFlags::REQUIRED_PASSIVE_CAPTURE)
    {
        return Err(PayloadError::MissingRequiredStatusFlags {
            required: CaptureReadyFlags::REQUIRED_PASSIVE_CAPTURE,
            observed: value.flags.bits(),
        });
    }
    require_nonzero_u64("pid", u64::from(value.pid))?;
    require_nonzero_u64(
        "process_creation_filetime_utc",
        value.process_creation_filetime_utc,
    )?;
    require_nonzero_u64(
        "requested_duration_ms",
        u64::from(value.requested_duration_ms),
    )?;
    require_nonzero_u64(
        "validated_domain_count",
        u64::from(value.validated_domain_count),
    )?;
    require_nonzero_u64(
        "scheduler_queue_capacity",
        u64::from(value.scheduler_queue_capacity),
    )?;
    if value.scheduler_queue_capacity != crate::passive_ready::PRODUCTION_DOMAIN_QUEUE_CAPACITY_V1 {
        return Err(PayloadError::InvalidPinnedProductionContract);
    }
    require_nonzero_u64(
        "scheduler_worker_count",
        u64::from(value.scheduler_worker_count),
    )?;
    if value.allowed_hook_count != crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1 {
        return Err(PayloadError::ProductionHookContractSkew {
            declared: value.allowed_hook_count,
            required: crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1,
        });
    }
    if value.active_hook_count > value.allowed_hook_count {
        return Err(PayloadError::HookCountExceedsAllowlist {
            active: value.active_hook_count,
            allowed: value.allowed_hook_count,
        });
    }
    if value.install_freeze_count == 0
        && value.active_hook_count > crate::passive_ready::PINNED_PRODUCTION_IMPORT_HOOK_COUNT_V1
    {
        return Err(PayloadError::UncountedInstallFreezes {
            active: value.active_hook_count,
            import_only: crate::passive_ready::PINNED_PRODUCTION_IMPORT_HOOK_COUNT_V1,
        });
    }
    if crate::calendar::hour_index_from_raw(value.clock_hour_raw).is_none() {
        return Err(PayloadError::InvalidGameHour(value.clock_hour_raw));
    }
    if value.clock_observation_ordinal == u64::MAX {
        return Err(PayloadError::ExhaustedOrdinal("clock_observation_ordinal"));
    }
    require_nonzero_digest("target_executable_sha256", &value.target_executable_sha256)?;
    if value.target_executable_sha256 != crate::passive_ready::PINNED_A729_TARGET_EXECUTABLE_SHA256
    {
        return Err(PayloadError::InvalidPinnedProductionContract);
    }
    require_nonzero_digest("collector_dll_sha256", &value.collector_dll_sha256)?;
    require_nonzero_digest("hook_allowlist_sha256", &value.hook_allowlist_sha256)?;
    let expected_allowlist = crate::passive_ready::pinned_production_hook_allowlist_sha256_v1()
        .map_err(|_| PayloadError::InvalidPinnedProductionContract)?;
    if value.hook_allowlist_sha256 != expected_allowlist {
        return Err(PayloadError::InvalidPinnedProductionContract);
    }
    require_nonzero_digest("hook_outcomes_sha256", &value.hook_outcomes_sha256)?;
    require_nonzero_digest(
        "validated_domain_roster_sha256",
        &value.validated_domain_roster_sha256,
    )?;
    require_nonzero_digest("clock_witness_sha256", &value.clock_witness_sha256)?;
    require_nonzero_digest("scheduler_config_sha256", &value.scheduler_config_sha256)?;
    require_nonzero_digest(
        "output_pipe_identity_sha256",
        &value.output_pipe_identity_sha256,
    )?;
    require_nonzero_digest("pipe_token_sha256", &value.pipe_token_sha256)?;
    require_nonzero_digest(
        "zero_mutation_policy_sha256",
        &value.zero_mutation_policy_sha256,
    )?;
    if value.zero_mutation_policy_sha256 != crate::passive_ready::zero_mutation_policy_sha256() {
        return Err(PayloadError::InvalidPinnedProductionContract);
    }
    require_nonzero_digest("session_id_sha256", &value.session_id_sha256)?;
    require_nonzero_digest("control_sha256", &value.control_sha256)
}

fn validate_ready_token_binding(value: &CaptureReadyV2, token: &[u8]) -> Result<(), PayloadError> {
    let expected: [u8; 32] = Sha256::digest(token).into();
    if value.pipe_token_sha256 == expected {
        Ok(())
    } else {
        Err(PayloadError::InvalidPinnedProductionContract)
    }
}

fn validate_ready_ack(value: &CaptureReadyAckV2) -> Result<(), PayloadError> {
    CaptureReadyAckFlags::from_bits(value.flags.bits())?;
    if value.flags.bits() != CaptureReadyAckFlags::HOST_VALIDATED {
        return Err(PayloadError::MissingRequiredStatusFlags {
            required: CaptureReadyAckFlags::HOST_VALIDATED,
            observed: value.flags.bits(),
        });
    }
    require_nonzero_u64("pid", u64::from(value.pid))?;
    require_nonzero_u64(
        "process_creation_filetime_utc",
        value.process_creation_filetime_utc,
    )?;
    require_nonzero_u64(
        "requested_duration_ms",
        u64::from(value.requested_duration_ms),
    )?;
    require_nonzero_u64(
        "validated_domain_count",
        u64::from(value.validated_domain_count),
    )?;
    require_nonzero_digest("ready_sha256", &value.ready_sha256)?;
    require_nonzero_digest(
        "public_receipt_core_sha256",
        &value.public_receipt_core_sha256,
    )?;
    require_nonzero_digest("session_id_sha256", &value.session_id_sha256)
}

fn require_nonzero_u64(field: &'static str, value: u64) -> Result<(), PayloadError> {
    if value == 0 {
        Err(PayloadError::ZeroRequiredField(field))
    } else {
        Ok(())
    }
}

fn require_nonzero_digest(field: &'static str, value: &[u8; 32]) -> Result<(), PayloadError> {
    if value == &[0; 32] {
        Err(PayloadError::ZeroRequiredField(field))
    } else {
        Ok(())
    }
}

fn check_capture_prefix(
    bytes: &[u8],
    magic: &[u8; 4],
    expected_len: usize,
) -> Result<(), PayloadError> {
    if bytes.len() != expected_len {
        return Err(PayloadError::WrongLength {
            expected: expected_len,
            actual: bytes.len(),
        });
    }
    if &bytes[..4] != magic {
        return Err(PayloadError::InvalidMagic);
    }
    let version = read_u16(bytes, 4);
    if version != CAPTURE_READY_VERSION {
        return Err(PayloadError::UnsupportedVersion(version));
    }
    if usize::from(read_u16(bytes, 6)) != expected_len
        || read_u32(bytes, 8) as usize != expected_len
    {
        return Err(PayloadError::InvalidDeclaredLength);
    }
    Ok(())
}

fn check_capture_length(bytes: &[u8], expected_len: usize) -> Result<(), PayloadError> {
    if bytes.len() == expected_len {
        Ok(())
    } else {
        Err(PayloadError::WrongLength {
            expected: expected_len,
            actual: bytes.len(),
        })
    }
}

fn hmac_sha256(token: &[u8], body: &[u8]) -> [u8; 32] {
    const BLOCK_LEN: usize = 64;
    let mut key = [0u8; BLOCK_LEN];
    if token.len() > BLOCK_LEN {
        key[..32].copy_from_slice(&Sha256::digest(token));
    } else {
        key[..token.len()].copy_from_slice(token);
    }
    let mut inner_pad = [0x36u8; BLOCK_LEN];
    let mut outer_pad = [0x5cu8; BLOCK_LEN];
    for index in 0..BLOCK_LEN {
        inner_pad[index] ^= key[index];
        outer_pad[index] ^= key[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(body);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}

fn verify_hmac(token: &[u8], body: &[u8], supplied: &[u8]) -> Result<(), PayloadError> {
    let expected = hmac_sha256(token, body);
    let mut difference = 0u8;
    for (left, right) in expected.iter().zip(supplied) {
        difference |= left ^ right;
    }
    if supplied.len() == expected.len() && difference == 0 {
        Ok(())
    } else {
        Err(PayloadError::AuthenticationFailed)
    }
}

fn push_prefix(payload: &mut Vec<u8>) {
    payload.extend_from_slice(&PAYLOAD_SCHEMA.to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
}

fn check_prefix(payload: &[u8], expected_len: usize) -> Result<(), PayloadError> {
    if payload.len() != expected_len {
        return Err(PayloadError::WrongLength {
            expected: expected_len,
            actual: payload.len(),
        });
    }
    let schema = read_u16(payload, 0);
    if schema != PAYLOAD_SCHEMA {
        return Err(PayloadError::UnsupportedSchema(schema));
    }
    if read_u16(payload, 2) != 0 {
        return Err(PayloadError::NonZeroReserved);
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("bounds checked"),
    )
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounds checked"),
    )
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounds checked"),
    )
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("bounds checked"),
    )
}

fn read_bytes16(bytes: &[u8], offset: usize) -> [u8; 16] {
    bytes[offset..offset + 16]
        .try_into()
        .expect("bounds checked")
}

fn read_bytes32(bytes: &[u8], offset: usize) -> [u8; 32] {
    bytes[offset..offset + 32]
        .try_into()
        .expect("bounds checked")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_hello_round_trips_and_rejects_zero_challenge() {
        let fields = RecorderHelloV1 {
            expected_pid: 0x1122_3344,
            expected_creation_filetime: 0x0102_0304_0506_0708,
            h4cr_version: 7,
            expected_dll_sha256: [0xab; 32],
            challenge: [0x5c; 16],
        };
        let wire = encode_recorder_hello(&fields);
        assert_eq!(wire.len(), RECORDER_HELLO_LEN);
        assert_eq!(decode_recorder_hello(&wire).unwrap(), fields);

        let mut zero_challenge = fields;
        zero_challenge.challenge = [0; 16];
        assert_eq!(
            decode_recorder_hello(&encode_recorder_hello(&zero_challenge)),
            Err(PayloadError::ZeroChallenge)
        );
    }

    #[test]
    fn collector_hello_round_trips_and_pins_control_version_slot() {
        let fields = CollectorHelloV1 {
            pid: 42,
            creation_filetime: 9001,
            h4cr_version: 5,
            control_version: 1,
            dll_sha256: [3; 32],
            challenge: [4; 16],
        };
        let wire = encode_collector_hello(&fields);
        assert_eq!(wire.len(), COLLECTOR_HELLO_LEN);
        assert_eq!(read_u16(&wire, PREFIX_LEN + 14), 1);
        assert_eq!(decode_collector_hello(&wire).unwrap(), fields);
    }

    #[test]
    fn collector_hello_with_capabilities_round_trips_and_pins_layout() {
        let fields = CollectorHelloV1 {
            pid: 42,
            creation_filetime: 9001,
            h4cr_version: 5,
            control_version: 1,
            dll_sha256: [3; 32],
            challenge: [4; 16],
        };
        let bits: u32 = 0b11_0011;
        let wire = encode_collector_hello_with_capabilities(&fields, bits);
        assert_eq!(wire.len(), COLLECTOR_HELLO_WITH_CAPABILITIES_LEN);
        assert_eq!(wire[..COLLECTOR_HELLO_LEN], encode_collector_hello(&fields));
        assert_eq!(wire[COLLECTOR_HELLO_LEN..], bits.to_le_bytes());
        assert_eq!(
            decode_collector_hello_with_capabilities(&wire).unwrap(),
            (fields, bits)
        );

        assert_eq!(
            decode_collector_hello_with_capabilities(&encode_collector_hello(&fields)),
            Err(PayloadError::WrongLength {
                expected: COLLECTOR_HELLO_WITH_CAPABILITIES_LEN,
                actual: COLLECTOR_HELLO_LEN,
            })
        );
    }

    #[test]
    fn heartbeat_challenge_round_trips() {
        let fields = HeartbeatChallengeV1 {
            challenge: [9; 16],
            host_challenge_counter: 0xdead_beef_cafe_0001,
        };
        let wire = encode_heartbeat_challenge(&fields);
        assert_eq!(wire.len(), HEARTBEAT_CHALLENGE_LEN);
        assert_eq!(decode_heartbeat_challenge(&wire).unwrap(), fields);
    }

    #[test]
    fn heartbeat_proof_round_trips_and_rejects_unknown_state() {
        let fields = HeartbeatProofV1 {
            challenge: [1; 16],
            last_h4cr_sequence: 123,
            state: HEARTBEAT_STATE_PRODUCING,
        };
        let wire = encode_heartbeat_proof(&fields);
        assert_eq!(wire.len(), HEARTBEAT_PROOF_LEN);
        assert_eq!(decode_heartbeat_proof(&wire).unwrap(), fields);

        let mut bad = wire.clone();
        bad[PREFIX_LEN + 24..PREFIX_LEN + 26].copy_from_slice(&9u16.to_le_bytes());
        assert_eq!(
            decode_heartbeat_proof(&bad),
            Err(PayloadError::UnknownHeartbeatState(9))
        );

        let mut nonzero_reserved = wire;
        nonzero_reserved[PREFIX_LEN + 26..PREFIX_LEN + 28].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            decode_heartbeat_proof(&nonzero_reserved),
            Err(PayloadError::NonZeroReserved)
        );
    }

    #[test]
    fn first_compact_frame_ack_round_trips_and_rejects_reserved() {
        let fields = FirstCompactFrameAckV1 {
            h4cr_sequence: 12,
            game_hour: -1,
            record_type: 3,
            total_record_len: 4096,
            record_sha256: [0x77; 32],
        };
        let wire = encode_first_compact_frame_ack(&fields);
        assert_eq!(wire.len(), FIRST_COMPACT_FRAME_ACK_LEN);
        assert_eq!(decode_first_compact_frame_ack(&wire).unwrap(), fields);

        let mut bad = wire;
        bad[PREFIX_LEN + 14..PREFIX_LEN + 16].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            decode_first_compact_frame_ack(&bad),
            Err(PayloadError::NonZeroReserved)
        );
    }

    #[test]
    fn wrong_length_and_bad_schema_reject_before_field_access() {
        assert!(matches!(
            decode_collector_hello(&[0u8; 10]),
            Err(PayloadError::WrongLength { .. })
        ));
        let mut wire = encode_collector_hello(&CollectorHelloV1 {
            pid: 1,
            creation_filetime: 2,
            h4cr_version: 3,
            control_version: 1,
            dll_sha256: [0; 32],
            challenge: [1; 16],
        });
        wire[0..2].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            decode_collector_hello(&wire),
            Err(PayloadError::UnsupportedSchema(2))
        );
    }

    fn to_hex32(bytes: &[u8; 32]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn hex32(text: &str) -> [u8; 32] {
        let bytes: Vec<u8> = (0..32)
            .map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap())
            .collect();
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        out
    }

    fn t63_ready_fields(install_freeze_count: u16) -> CaptureReadyV2 {
        CaptureReadyV2 {
            flags: CaptureReadyFlags::from_bits(CaptureReadyFlags::REQUIRED_PASSIVE_CAPTURE)
                .unwrap(),
            pid: 34_900,
            collector_features: CollectorFeatureBits::from_bits(CollectorFeatureBits::DECLARED)
                .unwrap(),
            process_creation_filetime_utc: 134_313_300_665_890_026,
            requested_duration_ms: 120_000,
            validated_domain_count: 67,
            scheduler_queue_capacity: 1,
            scheduler_worker_count: 12,
            // 2026-08-24: the three gameplay.d57.* sites joined the pinned
            // roster (45 -> 48); 2026-08-25: d57 readmission kept the count
            // while nine sealed domains left, then lane RO's five
            // gameplay.d13.* change-gate sites joined (48 -> 53) and the
            // allowlist digest moved with them.
            allowed_hook_count: 53,
            active_hook_count: 53,
            install_freeze_count,
            clock_hour_raw: 60_831_264,
            clock_observation_ordinal: 12,
            target_executable_sha256: hex32(
                "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
            ),
            collector_dll_sha256: hex32(
                "18883f3ad61d796d13e406b28869e764efca61863928150fbe08cdcb23bb07b8",
            ),
            hook_allowlist_sha256: hex32(
                "46adf385cbd87c19d5715d11a0e88be1587ccfd86ecdcac37211af650a0538fd",
            ),
            hook_outcomes_sha256: hex32(
                "416db3019bcd56f982a9e11978bcc47a0b9dc06d23147935d56dbbf30907951b",
            ),
            validated_domain_roster_sha256: hex32(
                "3e74b2f4cdad7b9a4f6f5326cba28dd14bf0eb2f8475be8f87b39ef65bd4b1ca",
            ),
            clock_witness_sha256: hex32(
                "b9783a3e914bc1cb498e69ab9d6d2c5c7979854e6b5d4d981cad6b0539922b39",
            ),
            scheduler_config_sha256: hex32(
                "bccf0cf2af1a2d002316ce163e5c41504b1507dca3ebd901e5082866c56a9bc4",
            ),
            output_pipe_identity_sha256: hex32(
                "8d85b8126cb3811b7f49b002c865d543cf66cd8f0c3c3c854f520bb975499218",
            ),
            pipe_token_sha256: hex32(
                "468803408b348616f75c0e5fad146849d75c4b03af945baf14a1412e205afd3b",
            ),
            zero_mutation_policy_sha256: hex32(
                "bfd0f328539c2dc07982553870d218d8c6dd5b2ab2683ec082b3392cf465dcae",
            ),
            session_id_sha256: hex32(
                "33c6ece20b57f5ebaaa0fbef6f384c0bce421f511082c56a3581657e19d6b602",
            ),
            capture_id_sha256: hex32(
                "b751d03e68c6f9edc58e5a6924031a584a04d827e1cf44b198e185acaf22648d",
            ),
            control_sha256: hex32(
                "758746092eb8b83c15916bce1842ab8d1d50d99dd0c14cafe0d3403206bded36",
            ),
        }
    }

    #[test]
    fn passive_capture_ready_pins_the_harness_rebuild_vectors_at_offset_50() {
        let token = b"f743c8f124d2572ad18b99db033554ba";
        let thirteen = t63_ready_fields(13)
            .encode_authenticated(token)
            .expect("t63 fields are a valid CaptureReadyV2");
        assert_eq!(read_u16(&thirteen, 50), 13);
        assert_eq!(
            to_hex32(&capture_ready_payload_sha256(&thirteen)),
            "cc16c0ee1e931532cb0df751c28af5619ab9d8cc047684bd39fef990e1887eff"
        );
        let thousand = t63_ready_fields(1000)
            .encode_authenticated(token)
            .expect("t63 fields are a valid CaptureReadyV2");
        assert_eq!(read_u16(&thousand, 50), 1000);
        assert_eq!(
            to_hex32(&capture_ready_payload_sha256(&thousand)),
            "8052536857f29782d414b232d338c921d7aa365baeb89d0fc49e2003e8f7c61d"
        );
        assert_ne!(thirteen, thousand);
    }

    fn ready_fields() -> CaptureReadyV2 {
        CaptureReadyV2 {
            flags: CaptureReadyFlags::from_bits(CaptureReadyFlags::REQUIRED_PASSIVE_CAPTURE)
                .unwrap(),
            pid: 42,
            collector_features: CollectorFeatureBits::from_bits(
                CollectorFeatureBits::DECLARED | CollectorFeatureBits::RANGE_TRACER,
            )
            .unwrap(),
            process_creation_filetime_utc: 0x0102_0304_0506_0708,
            requested_duration_ms: 60_000,
            validated_domain_count: 76,
            scheduler_queue_capacity: crate::passive_ready::PRODUCTION_DOMAIN_QUEUE_CAPACITY_V1,
            scheduler_worker_count: 8,
            allowed_hook_count: crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1,
            active_hook_count: crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1,
            install_freeze_count: 13,
            clock_hour_raw: 60_800_276,
            clock_observation_ordinal: 9_876,
            target_executable_sha256: crate::passive_ready::PINNED_A729_TARGET_EXECUTABLE_SHA256,
            collector_dll_sha256: [2; 32],
            hook_allowlist_sha256:
                crate::passive_ready::pinned_production_hook_allowlist_sha256_v1().unwrap(),
            hook_outcomes_sha256: [4; 32],
            validated_domain_roster_sha256: [5; 32],
            clock_witness_sha256: [6; 32],
            scheduler_config_sha256: [7; 32],
            output_pipe_identity_sha256: [8; 32],
            pipe_token_sha256: Sha256::digest(b"per-attach CSPRNG token").into(),
            zero_mutation_policy_sha256: crate::passive_ready::zero_mutation_policy_sha256(),
            session_id_sha256: [11; 32],
            capture_id_sha256: [0; 32],
            control_sha256: [12; 32],
        }
    }

    #[test]
    fn capture_ready_identity_bit_is_optional_and_round_trips() {
        let token = b"per-attach CSPRNG token";

        let mut with_identity = ready_fields();
        with_identity.flags = CaptureReadyFlags::from_bits(
            CaptureReadyFlags::REQUIRED_PASSIVE_CAPTURE | CaptureReadyFlags::IDENTITY_V1_STREAM,
        )
        .unwrap();
        let wire = with_identity.encode_authenticated(token).unwrap();
        assert_eq!(wire.len(), CAPTURE_READY_LEN);
        let decoded = CaptureReadyV2::decode_authenticated(&wire, token).unwrap();
        assert_eq!(decoded, with_identity);
        assert!(decoded
            .flags
            .contains(CaptureReadyFlags::IDENTITY_V1_STREAM));

        let without_identity = ready_fields();
        let wire = without_identity.encode_authenticated(token).unwrap();
        let decoded = CaptureReadyV2::decode_authenticated(&wire, token).unwrap();
        assert_eq!(decoded, without_identity);
        assert!(!decoded
            .flags
            .contains(CaptureReadyFlags::IDENTITY_V1_STREAM));
    }

    #[test]
    fn capture_ready_still_refuses_a_missing_required_bit_and_an_unknown_bit() {
        let token = b"per-attach CSPRNG token";
        let mut short = ready_fields();
        short.flags = CaptureReadyFlags::from_bits(
            CaptureReadyFlags::REQUIRED_PASSIVE_CAPTURE
                & !CaptureReadyFlags::ZERO_MUTATION_POLICY_ENFORCED,
        )
        .unwrap();
        assert!(matches!(
            short.encode_authenticated(token),
            Err(PayloadError::MissingRequiredStatusFlags { .. })
        ));

        assert!(matches!(
            CaptureReadyFlags::from_bits(CaptureReadyFlags::KNOWN_MASK | (1 << 9)),
            Err(PayloadError::UnknownStatusFlags(_))
        ));
    }

    #[test]
    fn passive_capture_ready_fixed_layout_round_trips_with_optional_capture_id() {
        let token = b"per-attach CSPRNG token";
        let fields = ready_fields();
        let wire = fields.encode_authenticated(token).unwrap();
        assert_eq!(wire.len(), CAPTURE_READY_LEN);
        assert_eq!(&wire[0..4], CAPTURE_READY_MAGIC);
        assert_eq!(read_u16(&wire, 4), CAPTURE_READY_VERSION);
        assert_eq!(read_u16(&wire, 6) as usize, CAPTURE_READY_LEN);
        assert_eq!(read_u32(&wire, 8) as usize, CAPTURE_READY_LEN);
        assert_eq!(
            read_u32(&wire, 12),
            CaptureReadyFlags::REQUIRED_PASSIVE_CAPTURE
        );
        assert_eq!(read_u32(&wire, 16), fields.pid);
        assert_eq!(read_u32(&wire, 20), fields.collector_features.bits());
        assert_eq!(read_u64(&wire, 24), fields.process_creation_filetime_utc);
        assert_eq!(read_u32(&wire, 36), fields.validated_domain_count);
        assert_eq!(read_u32(&wire, 40), fields.scheduler_queue_capacity);
        assert_eq!(read_u16(&wire, 44), fields.scheduler_worker_count);
        assert_eq!(read_u16(&wire, 46), fields.allowed_hook_count);
        assert_eq!(read_u16(&wire, 48), fields.active_hook_count);
        assert_eq!(read_i32(&wire, 52), fields.clock_hour_raw);
        assert_eq!(read_u64(&wire, 56), fields.clock_observation_ordinal);
        assert_eq!(&wire[416..448], &[0; 32]);
        assert_eq!(
            CaptureReadyV2::decode_authenticated(&wire, token).unwrap(),
            fields
        );
        assert_eq!(
            CaptureReadyV2::decode_authenticated(&wire, b"other token"),
            Err(PayloadError::AuthenticationFailed)
        );

        let mut identified = fields;
        identified.capture_id_sha256 = [9; 32];
        let identified_wire = identified.encode_authenticated(token).unwrap();
        assert_eq!(
            CaptureReadyV2::decode_authenticated(&identified_wire, token).unwrap(),
            identified
        );
    }

    #[test]
    fn collector_feature_bits_separate_undeclared_from_clean_from_diagnostic() {
        let undeclared = CollectorFeatureBits::from_bits(0).unwrap();
        assert!(!undeclared.declared());
        assert_eq!(undeclared.diagnostic_build(), None);
        assert_eq!(undeclared.feature_names(), None);

        let clean = CollectorFeatureBits::from_bits(CollectorFeatureBits::DECLARED).unwrap();
        assert!(clean.declared());
        assert_eq!(clean.diagnostic_build(), Some(false));
        assert_eq!(clean.feature_names(), Some(Vec::new()));

        let traced = CollectorFeatureBits::from_bits(
            CollectorFeatureBits::DECLARED | CollectorFeatureBits::RANGE_TRACER,
        )
        .unwrap();
        assert_eq!(traced.diagnostic_build(), Some(true));
        assert_eq!(traced.feature_names(), Some(vec!["range-tracer"]));

        let named = CollectorFeatureBits::FEATURE_NAMES
            .iter()
            .fold(0u32, |acc, (bit, _)| acc | bit);
        assert_eq!(
            named,
            CollectorFeatureBits::KNOWN_MASK & !CollectorFeatureBits::DECLARED
        );
        let mut names: Vec<&str> = CollectorFeatureBits::FEATURE_NAMES
            .iter()
            .map(|(_, name)| *name)
            .collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
        assert_eq!(CollectorFeatureBits::DIAGNOSTIC_MASK, named);

        assert_eq!(
            CollectorFeatureBits::from_bits(1 << 31),
            Err(PayloadError::UnknownCollectorFeatureBits(1 << 31))
        );

        for retired in [1u32 << 3, 1u32 << 4, 1u32 << 5] {
            assert_eq!(
                CollectorFeatureBits::from_bits(CollectorFeatureBits::DECLARED | retired),
                Err(PayloadError::UnknownCollectorFeatureBits(retired)),
                "retired collector feature bit {retired:#x} must stay refused as unknown"
            );
        }
        assert_eq!(
            CollectorFeatureBits::from_bits(CollectorFeatureBits::TEST_SUPPORT),
            Err(PayloadError::UndeclaredCollectorFeatureBits(
                CollectorFeatureBits::TEST_SUPPORT
            ))
        );
    }

    #[test]
    fn passive_capture_ready_from_an_undeclaring_collector_still_decodes() {
        let token = b"per-attach CSPRNG token";
        let mut fields = ready_fields();
        fields.collector_features = CollectorFeatureBits::undeclared();
        let wire = fields.encode_authenticated(token).unwrap();
        assert_eq!(read_u32(&wire, 20), 0);
        let decoded = CaptureReadyV2::decode_authenticated(&wire, token).unwrap();
        assert_eq!(decoded, fields);
        assert_eq!(decoded.collector_features.diagnostic_build(), None);
    }

    #[test]
    fn passive_capture_ready_rejects_noncanonical_header_reserved_flags_and_tag() {
        let token = b"per-attach CSPRNG token";
        let wire = ready_fields().encode_authenticated(token).unwrap();
        let mutations: &[(usize, u8, PayloadError)] = &[
            (0, b'X', PayloadError::InvalidMagic),
            (4, 1, PayloadError::UnsupportedVersion(1)),
            (6, 1, PayloadError::InvalidDeclaredLength),
            (8, 1, PayloadError::InvalidDeclaredLength),
            (
                23,
                0x80,
                PayloadError::UnknownCollectorFeatureBits(0x8000_0000),
            ),
            (
                20,
                CollectorFeatureBits::RANGE_TRACER as u8,
                PayloadError::UndeclaredCollectorFeatureBits(CollectorFeatureBits::RANGE_TRACER),
            ),
            (
                50,
                0,
                PayloadError::UncountedInstallFreezes {
                    active: crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1,
                    import_only: crate::passive_ready::PINNED_PRODUCTION_IMPORT_HOOK_COUNT_V1,
                },
            ),
        ];
        for &(offset, replacement, expected) in mutations {
            let mut bad = wire.clone();
            bad[offset] = replacement;
            let tag = hmac_sha256(token, &bad[..CAPTURE_READY_AUTH_OFFSET]);
            bad[CAPTURE_READY_AUTH_OFFSET..].copy_from_slice(&tag);
            assert_eq!(
                CaptureReadyV2::decode_authenticated(&bad, token),
                Err(expected),
                "offset {offset}"
            );
        }
        let mut wide = ready_fields();
        wide.install_freeze_count = 0x0102;
        let wide_wire = wide.encode_authenticated(token).unwrap();
        assert_eq!(
            CaptureReadyV2::decode_authenticated(&wide_wire, token)
                .unwrap()
                .install_freeze_count,
            0x0102
        );
        assert_eq!(read_u16(&wide_wire, 50), 0x0102);
        let mut unknown_flag = wire.clone();
        unknown_flag[15] |= 0x80;
        let tag = hmac_sha256(token, &unknown_flag[..CAPTURE_READY_AUTH_OFFSET]);
        unknown_flag[CAPTURE_READY_AUTH_OFFSET..].copy_from_slice(&tag);
        assert_eq!(
            CaptureReadyV2::decode_authenticated(&unknown_flag, token),
            Err(PayloadError::UnknownStatusFlags(0x8000_0000))
        );
        let mut missing_flags = ready_fields();
        missing_flags.flags = CaptureReadyFlags::from_bits(0).unwrap();
        assert_eq!(
            missing_flags.encode_authenticated(token),
            Err(PayloadError::MissingRequiredStatusFlags {
                required: CaptureReadyFlags::REQUIRED_PASSIVE_CAPTURE,
                observed: 0,
            })
        );
        let mut bad_tag = wire.clone();
        bad_tag[CAPTURE_READY_LEN - 1] ^= 0x80;
        assert_eq!(
            CaptureReadyV2::decode_authenticated(&bad_tag, token),
            Err(PayloadError::AuthenticationFailed)
        );
        assert!(matches!(
            CaptureReadyV2::decode_authenticated(&wire[..wire.len() - 1], token),
            Err(PayloadError::WrongLength { .. })
        ));
        assert_eq!(
            ready_fields().encode_authenticated(b""),
            Err(PayloadError::EmptyAuthenticationToken)
        );
        let mut wrong_token_binding = ready_fields();
        wrong_token_binding.pipe_token_sha256 = [9; 32];
        assert_eq!(
            wrong_token_binding.encode_authenticated(token),
            Err(PayloadError::InvalidPinnedProductionContract)
        );
        let mut invalid_hour = ready_fields();
        invalid_hour.clock_hour_raw = 0;
        assert_eq!(
            invalid_hour.encode_authenticated(token),
            Err(PayloadError::InvalidGameHour(0))
        );

        let mut unauthenticated_reserved = wire;
        unauthenticated_reserved[20] = 1;
        assert_eq!(
            CaptureReadyV2::decode_authenticated(&unauthenticated_reserved, token),
            Err(PayloadError::AuthenticationFailed)
        );
    }

    #[test]
    fn capture_ready_ack_binds_ready_and_public_receipt_core_digests() {
        let token = b"per-attach CSPRNG token";
        let ready = ready_fields().encode_authenticated(token).unwrap();
        let ack = CaptureReadyAckV2 {
            flags: CaptureReadyAckFlags::from_bits(CaptureReadyAckFlags::HOST_VALIDATED).unwrap(),
            pid: 42,
            process_creation_filetime_utc: 0x0102_0304_0506_0708,
            requested_duration_ms: 60_000,
            validated_domain_count: 76,
            ready_sha256: capture_ready_payload_sha256(&ready),
            public_receipt_core_sha256: [8; 32],
            session_id_sha256: [11; 32],
        };
        let wire = ack.encode_authenticated(token).unwrap();
        assert_eq!(wire.len(), CAPTURE_READY_ACK_LEN);
        assert_eq!(&wire[0..4], CAPTURE_READY_ACK_MAGIC);
        assert_eq!(&wire[40..72], &capture_ready_payload_sha256(&ready));
        assert_eq!(&wire[72..104], &[8; 32]);
        assert_eq!(
            CaptureReadyAckV2::decode_authenticated(&wire, token).unwrap(),
            ack
        );

        let mut unknown_flag = wire.clone();
        unknown_flag[12..16].copy_from_slice(&2u32.to_le_bytes());
        let tag = hmac_sha256(token, &unknown_flag[..CAPTURE_READY_ACK_AUTH_OFFSET]);
        unknown_flag[CAPTURE_READY_ACK_AUTH_OFFSET..].copy_from_slice(&tag);
        assert_eq!(
            CaptureReadyAckV2::decode_authenticated(&unknown_flag, token),
            Err(PayloadError::UnknownStatusFlags(2))
        );
        let mut missing_flags = ack;
        missing_flags.flags = CaptureReadyAckFlags::from_bits(0).unwrap();
        assert_eq!(
            missing_flags.encode_authenticated(token),
            Err(PayloadError::MissingRequiredStatusFlags {
                required: CaptureReadyAckFlags::HOST_VALIDATED,
                observed: 0,
            })
        );

        let mut reserved = wire.clone();
        reserved[20] = 1;
        assert_eq!(
            CaptureReadyAckV2::decode_authenticated(&reserved, token),
            Err(PayloadError::AuthenticationFailed)
        );
        let tag = hmac_sha256(token, &reserved[..CAPTURE_READY_ACK_AUTH_OFFSET]);
        reserved[CAPTURE_READY_ACK_AUTH_OFFSET..].copy_from_slice(&tag);
        assert_eq!(
            CaptureReadyAckV2::decode_authenticated(&reserved, token),
            Err(PayloadError::NonZeroReserved)
        );
    }

    #[test]
    fn capture_ready_required_zero_fields_fail_closed() {
        let token = b"per-attach CSPRNG token";
        let ready_mutations: [(&str, fn(&mut CaptureReadyV2)); 18] = [
            ("pid", |value| value.pid = 0),
            ("process_creation_filetime_utc", |value| {
                value.process_creation_filetime_utc = 0
            }),
            ("requested_duration_ms", |value| {
                value.requested_duration_ms = 0
            }),
            ("validated_domain_count", |value| {
                value.validated_domain_count = 0
            }),
            ("scheduler_queue_capacity", |value| {
                value.scheduler_queue_capacity = 0
            }),
            ("scheduler_worker_count", |value| {
                value.scheduler_worker_count = 0
            }),
            ("target_executable_sha256", |value| {
                value.target_executable_sha256 = [0; 32]
            }),
            ("collector_dll_sha256", |value| {
                value.collector_dll_sha256 = [0; 32]
            }),
            ("hook_allowlist_sha256", |value| {
                value.hook_allowlist_sha256 = [0; 32]
            }),
            ("hook_outcomes_sha256", |value| {
                value.hook_outcomes_sha256 = [0; 32]
            }),
            ("validated_domain_roster_sha256", |value| {
                value.validated_domain_roster_sha256 = [0; 32]
            }),
            ("clock_witness_sha256", |value| {
                value.clock_witness_sha256 = [0; 32]
            }),
            ("scheduler_config_sha256", |value| {
                value.scheduler_config_sha256 = [0; 32]
            }),
            ("output_pipe_identity_sha256", |value| {
                value.output_pipe_identity_sha256 = [0; 32]
            }),
            ("pipe_token_sha256", |value| {
                value.pipe_token_sha256 = [0; 32]
            }),
            ("zero_mutation_policy_sha256", |value| {
                value.zero_mutation_policy_sha256 = [0; 32]
            }),
            ("session_id_sha256", |value| {
                value.session_id_sha256 = [0; 32]
            }),
            ("control_sha256", |value| value.control_sha256 = [0; 32]),
        ];
        for (field, mutate) in ready_mutations {
            let mut value = ready_fields();
            mutate(&mut value);
            assert_eq!(
                value.encode_authenticated(token),
                Err(PayloadError::ZeroRequiredField(field)),
                "{field}"
            );
        }

        let ready = ready_fields().encode_authenticated(token).unwrap();
        let ack_fields = || CaptureReadyAckV2 {
            flags: CaptureReadyAckFlags::from_bits(CaptureReadyAckFlags::HOST_VALIDATED).unwrap(),
            pid: 42,
            process_creation_filetime_utc: 0x0102_0304_0506_0708,
            requested_duration_ms: 60_000,
            validated_domain_count: 76,
            ready_sha256: capture_ready_payload_sha256(&ready),
            public_receipt_core_sha256: [8; 32],
            session_id_sha256: [11; 32],
        };
        let ack_mutations: [(&str, fn(&mut CaptureReadyAckV2)); 7] = [
            ("pid", |value| value.pid = 0),
            ("process_creation_filetime_utc", |value| {
                value.process_creation_filetime_utc = 0
            }),
            ("requested_duration_ms", |value| {
                value.requested_duration_ms = 0
            }),
            ("validated_domain_count", |value| {
                value.validated_domain_count = 0
            }),
            ("ready_sha256", |value| value.ready_sha256 = [0; 32]),
            ("public_receipt_core_sha256", |value| {
                value.public_receipt_core_sha256 = [0; 32]
            }),
            ("session_id_sha256", |value| {
                value.session_id_sha256 = [0; 32]
            }),
        ];
        for (field, mutate) in ack_mutations {
            let mut value = ack_fields();
            mutate(&mut value);
            assert_eq!(
                value.encode_authenticated(token),
                Err(PayloadError::ZeroRequiredField(field)),
                "{field}"
            );
        }
    }

    #[test]
    fn capture_ready_hook_counts_are_allowlist_bounded() {
        let token = b"per-attach CSPRNG token";
        let mut value = ready_fields();
        value.active_hook_count = value.allowed_hook_count + 1;
        assert_eq!(
            value.encode_authenticated(token),
            Err(PayloadError::HookCountExceedsAllowlist {
                active: crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1 + 1,
                allowed: crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1,
            })
        );
        value.active_hook_count = 0;
        value.allowed_hook_count = 0;
        assert_eq!(
            value.encode_authenticated(token),
            Err(PayloadError::ProductionHookContractSkew {
                declared: 0,
                required: crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1,
            })
        );
        let mut stale_collector = ready_fields();
        stale_collector.allowed_hook_count =
            crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1 - 14;
        stale_collector.active_hook_count = stale_collector.allowed_hook_count;
        let skew = stale_collector
            .encode_authenticated(token)
            .expect_err("a collector declaring a shorter pinned roster must be refused");
        assert_eq!(
            skew,
            PayloadError::ProductionHookContractSkew {
                declared: crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1 - 14,
                required: crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1,
            }
        );
        let rendered = skew.to_string();
        assert!(
            rendered.contains("rebuild") && rendered.contains("collector.dll"),
            "contract skew must tell the reader to rebuild the collector, got: {rendered}"
        );

        let mut degraded = ready_fields();
        degraded.active_hook_count = degraded.allowed_hook_count - 1;
        degraded
            .encode_authenticated(token)
            .expect("a capture that lost one non-mandatory producer must still be admissible");

        let mut exhausted_clock = ready_fields();
        exhausted_clock.clock_observation_ordinal = u64::MAX;
        assert_eq!(
            exhausted_clock.encode_authenticated(token),
            Err(PayloadError::ExhaustedOrdinal("clock_observation_ordinal"))
        );
    }

    #[test]
    fn hmac_sha256_matches_rfc4231_normal_and_long_key_vectors() {
        fn hex32(text: &str) -> [u8; 32] {
            let mut out = [0u8; 32];
            for (index, byte) in out.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap();
            }
            out
        }

        assert_eq!(
            hmac_sha256(&[0x0b; 20], b"Hi There"),
            hex32("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7")
        );
        assert_eq!(
            hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            ),
            hex32("60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54")
        );
    }
}
