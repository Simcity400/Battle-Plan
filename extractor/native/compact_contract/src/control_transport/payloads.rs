//! Shared, exact-byte H4CT v1 typed payload codecs.
//!
//! These are the single implementation of every H4CT payload the recorder host
//! and the injected collector exchange, so neither side can drift. Each payload
//! begins with the common `payload_schema: u16 = 1, reserved: u16 = 0` prefix;
//! decoding is deny-by-default and exact-length: unknown schema, wrong length,
//! nonzero reserved fields, zero IDs, and unknown enums all reject before a
//! typed consumer sees the value. There are no sentinel values and no
//! length-guessed schemas.

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

/// `HeartbeatProof.state` closed enum.
pub const HEARTBEAT_STATE_WAITING_FOR_WORLD: u16 = 1;
pub const HEARTBEAT_STATE_PRODUCING: u16 = 2;
pub const HEARTBEAT_STATE_PAUSED_FOR_CONTROL: u16 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadError {
    /// Total payload length did not equal the exact fixed size for its kind.
    WrongLength { expected: usize, actual: usize },
    /// The common prefix schema word was not the supported `1`.
    UnsupportedSchema(u16),
    /// A reserved field (prefix or body) carried nonzero bytes.
    NonZeroReserved,
    /// A 16-byte CSPRNG id/challenge decoded as all-zero.
    ZeroChallenge,
    /// `HeartbeatProof.state` was outside the closed enum.
    UnknownHeartbeatState(u16),
    /// A fixed READY/ACK payload carried a different four-byte magic.
    InvalidMagic,
    /// A fixed READY/ACK payload carried an unsupported version.
    UnsupportedVersion(u16),
    /// A fixed READY/ACK payload's encoded header/total length was not exact.
    InvalidDeclaredLength,
    /// A READY/ACK status word carried an unknown bit.
    UnknownStatusFlags(u32),
    /// READY declared a collector feature bit this build has no name for. A
    /// consumer cannot tell whether an unnamed feature is diagnostic, so the
    /// payload is refused rather than decoded with the bit dropped.
    UnknownCollectorFeatureBits(u32),
    /// READY declared collector features WITHOUT
    /// [`CollectorFeatureBits::DECLARED`]. A producer that knows the word sets
    /// that bit; a value that names features without it cannot be told apart
    /// from a corrupted reserved field.
    UndeclaredCollectorFeatureBits(u32),
    /// A READY/ACK status word omitted a required readiness fact.
    MissingRequiredStatusFlags { required: u32, observed: u32 },
    /// A field that must identify real production state was zero.
    ZeroRequiredField(&'static str),
    /// A witnessed game hour was outside the canonical the game calendar range.
    InvalidGameHour(i32),
    /// The active hook count exceeded the authenticated allowlist count.
    HookCountExceedsAllowlist { active: u16, allowed: u16 },
    /// READY reported live INLINE DETOURS with an install freeze count of zero.
    ///
    /// Patching live code requires stopping the process, so this is never a
    /// property of the capture: it is the producer's freeze counter not being
    /// wired to its receipt. Refused rather than decoded, because a zero here
    /// is indistinguishable from the months in which the counter existed and
    /// nothing read it -- and a census built on it would report a capture that
    /// stopped the game a dozen times as one that never did.
    UncountedInstallFreezes { active: u16, import_only: u16 },
    /// READY declared a pinned production roster of a different SIZE than this
    /// build's [`crate::passive_ready::PINNED_PRODUCTION_HOOK_SPECS_V1`].
    ///
    /// `allowed_hook_count` is restated by the producer straight from its own
    /// compiled `PINNED_PRODUCTION_HOOK_COUNT_V1`, so a disagreement here is
    /// never an install outcome: the two sides were built from different
    /// revisions of the pinned contract. This is called out separately from
    /// [`Self::HookCountExceedsAllowlist`] because the remedy is completely
    /// different -- rebuild the collector, do not investigate hooks.
    ProductionHookContractSkew { declared: u16, required: u16 },
    /// A zero-based ordinal reached the reserved terminal value.
    ExhaustedOrdinal(&'static str),
    /// HMAC-SHA256 requires a nonempty, unpredictable per-attach token.
    EmptyAuthenticationToken,
    /// The trailing HMAC-SHA256 did not authenticate the exact fixed body.
    AuthenticationFailed,
    /// The compiled exact-production contract could not construct its pinned
    /// hook authority.
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

// --- RecorderHello ----------------------------------------------------------

/// `RecorderHello` (recorder -> collector): the recorder's expectation of the
/// injected instance plus a fresh challenge id the collector must echo.
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
    payload.extend_from_slice(&0u16.to_le_bytes()); // reserved
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

// --- CollectorHello ---------------------------------------------------------

/// `CollectorHello` (collector -> recorder): the injected instance's exact
/// identity, its control version, its loaded-DLL hash, and the echoed challenge.
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

/// `CollectorHello` product-admission form: the exact 68-byte base hello
/// followed by the collector's closed H4CT capability claim as a little-endian
/// `u32` word (72 bytes total). This is the ONLY hello form the combined
/// H4CR/H4CT product admission accepts; the historical 68-byte form remains for
/// the stdout proof adapter. The capability bits are returned raw: semantic
/// validation (unknown bits, required-set equality) belongs to
/// `ControlCapabilities::from_bits` and the admission boundary, exactly as on
/// the recorder's hand-rolled predecessor.
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

// --- HeartbeatChallenge -----------------------------------------------------

/// `HeartbeatChallenge` (recorder -> collector).
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

// --- HeartbeatProof ---------------------------------------------------------

/// `HeartbeatProof` (collector -> recorder).
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
    payload.extend_from_slice(&0u16.to_le_bytes()); // reserved
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

// --- FirstCompactFrameAck ---------------------------------------------------

/// `FirstCompactFrameAck` (recorder -> collector).
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
    payload.extend_from_slice(&0u16.to_le_bytes()); // reserved
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

// --- CaptureReady / CaptureReadyAck ----------------------------------------

/// Closed passive-acquisition readiness flags in collector-authored `H4RD`.
///
/// The codec rejects unknown bits but does not manufacture readiness. The
/// producer sets only facts it has established; product admission requires the
/// complete passive-capture set.
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
    /// The stream this collector has already begun is an IdentityV1 stream: a
    /// CID1 campaign-identity record is its sequence-zero record and every
    /// ordinary record follows from sequence one.
    ///
    /// OPTIONAL, deliberately, and therefore NOT part of
    /// [`Self::REQUIRED_PASSIVE_CAPTURE`]. The campaign identity is read from
    /// live engine memory and can legitimately fail to validate -- an
    /// unrecognised build, a torn read, a world that has not finished loading.
    /// When it does, the collector clears this bit and produces a Legacy
    /// stream, and the recording still seals with its identity named
    /// unavailable. Making the bit mandatory would convert "we could not tell
    /// which campaign this was" into "there is no recording", which is the one
    /// trade this product never makes.
    ///
    /// It is a CLAIM ABOUT BYTES ALREADY WRITTEN, not a request: the collector
    /// writes CID1 into every configured sink BEFORE it sends this payload, so
    /// a set bit means the record is already in the stream. The recorder uses
    /// it only to select the reader mode; the identity itself is decoded from
    /// the stream bytes and nothing here can substitute for that.
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

/// WHICH COMPILE-TIME CARGO FEATURES the collector that produced a capture was
/// built with.
///
/// # Why a capture has to declare its own build
///
/// The collector DLL's sha256 has always been on this payload, and a sha names
/// a build exactly — but only to somebody who already has the build. It answers
/// "was this the same binary as that one"; it cannot answer "was this binary
/// compiled with the read tracer", which is the question that decides whether a
/// capture's TIMINGS are evidence at all.
///
/// Runs `t34` and `t35` (2026-08-07) both carried the `range-tracer` feature
/// DLL. Nothing in either manifest said so. Their frame timings were compared
/// against an untraced run's and a 12.7 ms shift was investigated as a
/// production regression for most of a day before the feature build was
/// noticed, from a sha alone. This word is what makes that a read rather than a
/// deduction.
///
/// # Bit 0 is the declaration itself
///
/// This word occupies what was a must-be-zero reserved `u32`, so an all-zero
/// value is exactly what every collector built before this contract sends. That
/// is why [`Self::DECLARED`] exists and why every declaring build sets it: `0`
/// means "this producer predates the declaration" — ABSENT — and it must never
/// be read as "this producer declared no features". Those are different facts
/// and only one of them licenses a timing comparison.
///
/// # Stability
///
/// New bits are appended. An unassigned bit is refused
/// ([`PayloadError::UnknownCollectorFeatureBits`]) rather than ignored: a
/// consumer that does not know a feature cannot know whether it is diagnostic,
/// and silently dropping it is how a diagnostic build passes for a production
/// one.
///
/// A bit is UNASSIGNED when the cargo feature it stood for is deleted, and
/// that is a wire-format break by construction — every stream that declared it
/// stops decoding. Pre-production that is the intended outcome (old captures
/// are disposable) and it is why the refusal above must never be relaxed into
/// a mask-and-ignore. What an unassignment must not do is free the bit for
/// reuse: a number handed to a different feature would make an old diagnostic
/// capture decode as a new clean one, which is the exact confusion this word
/// exists to prevent. Retired numbers stay retired and are named at the
/// assignment site.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CollectorFeatureBits(u32);

impl CollectorFeatureBits {
    /// This producer knows about this word. Set by every declaring build,
    /// INCLUDING one compiled with no optional feature at all.
    pub const DECLARED: u32 = 1 << 0;
    /// `collector/Cargo.toml` `test-support`.
    pub const TEST_SUPPORT: u32 = 1 << 1;
    /// `collector/Cargo.toml` `range-tracer` — the capture-floor read tracer.
    pub const RANGE_TRACER: u32 = 1 << 2;
    // Bits 3, 4 and 5 were `unshipped-a25-runtime-text-evidence`,
    // `unshipped-a39-character-presentation-evidence` and
    // `unshipped-a31-focus-presentation-evidence`. The fixture hooks those
    // features gated are deleted and the features with them, so the bits are
    // UNASSIGNED again and fall back under the refuse-unknown rule below. This
    // is a wire-format break, taken deliberately while there is no backwards
    // compatibility to keep: a stream that declares bit 3, 4 or 5 no longer
    // decodes, which is correct, because no producer that can be built today
    // has a feature to declare there. A bit added later takes the next free
    // one; do not reuse 3..=5 for an unrelated feature, or an old diagnostic
    // capture would decode as a new clean one.
    pub const KNOWN_MASK: u32 = Self::DECLARED | Self::TEST_SUPPORT | Self::RANGE_TRACER;

    /// The features whose presence makes a capture inadmissible as TIMING
    /// evidence, or makes it interact with the engine in a way ordinary
    /// production does not.
    ///
    /// `range-tracer` records every guarded read and is the reason this word
    /// exists. `test-support` compiles test-only lifecycle helpers into the
    /// lane crates. Neither belongs in a capture whose numbers are compared
    /// against production.
    pub const DIAGNOSTIC_MASK: u32 = Self::TEST_SUPPORT | Self::RANGE_TRACER;

    /// Each assigned bit spelled as the CARGO FEATURE NAME it stands for, so a
    /// manifest and an operator's `--features` line use the same words.
    /// Ascending by bit; `DECLARED` is not a feature and is not listed.
    pub const FEATURE_NAMES: &'static [(u32, &'static str)] = &[
        (Self::TEST_SUPPORT, "test-support"),
        (Self::RANGE_TRACER, "range-tracer"),
    ];

    /// The undeclared value: what a producer that predates this contract sends.
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

    /// Whether this producer declared its build at all. `false` is ABSENT, not
    /// "no features".
    pub const fn declared(self) -> bool {
        self.0 & Self::DECLARED != 0
    }

    /// `Some(true)` when a timing-affecting feature is compiled in,
    /// `Some(false)` when the producer declared a clean build, `None` when it
    /// declared nothing. The three are different and a caller that flattens
    /// `None` into `false` reintroduces the t35 defect.
    pub const fn diagnostic_build(self) -> Option<bool> {
        if self.declared() {
            Some(self.0 & Self::DIAGNOSTIC_MASK != 0)
        } else {
            None
        }
    }

    /// The compiled features by cargo name, ascending, or `None` when this
    /// producer declared nothing. An empty vector is a real observation: a
    /// declaring build with no optional feature.
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

/// Collector-authored fixed passive-acquisition startup receipt.
///
/// The HMAC binds the exact body to the per-attach token. It is not a
/// substitute for the live H4CT/named-pipe peer check: the recorder must still
/// authenticate the peer PID and process-creation FILETIME before trusting this
/// payload. `capture_id_sha256` is the sole optional digest and is all-zero when
/// no capture id was assigned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureReadyV2 {
    pub flags: CaptureReadyFlags,
    pub pid: u32,
    /// The producer's own compile-time feature set. Occupies the `u32` at
    /// offset 20 that was previously reserved-must-be-zero, so the payload
    /// LENGTH and every offset after it are unchanged and an undeclaring
    /// collector still encodes byte-identically. See [`CollectorFeatureBits`].
    pub collector_features: CollectorFeatureBits,
    pub process_creation_filetime_utc: u64,
    pub requested_duration_ms: u32,
    pub validated_domain_count: u32,
    pub scheduler_queue_capacity: u32,
    pub scheduler_worker_count: u16,
    pub allowed_hook_count: u16,
    pub active_hook_count: u16,
    /// How many freezes this collector ATTEMPTED to install its detours.
    ///
    /// Counted as a delta CLOSED WHEN THE INSTALL FINISHED, not read when READY
    /// is built: a value read here would also absorb anything that froze between
    /// the two. An upper bound on the number of times it stopped the game -- see
    /// the paragraph below.
    ///
    /// Occupies the `u16` at offset 50 that was previously
    /// reserved-must-be-zero, exactly as `collector_features` took the reserved
    /// `u32` at offset 20: the payload LENGTH and every offset after it are
    /// unchanged.
    ///
    /// WHY IT IS ON THE WIRE AT ALL. `inline_detour::freeze_attempts()` has
    /// existed for months with ZERO consumers in `collector/` or
    /// `recorder_host/`, so no receipt and no recording carried it, and the
    /// question "how many times does a real attach freeze the process" could not
    /// be answered from a capture -- which is the question lane JU's
    /// recommendation to spend a lane on `MinHook::apply_queued` turns on. Every
    /// per-attach figure JU could have published would have been this count,
    /// resolved to actual stopped windows by the census, multiplied by
    /// a per-window measurement, and it refused to publish one because the count
    /// was unverified. This is the count.
    ///
    /// IT IS ATTEMPTS, NOT STOPPED WINDOWS, and the difference is real rather
    /// than pedantic. `freeze_attempts` counts every entry to the freeze,
    /// refusals included, and a refusal comes in two shapes: one that never
    /// reached its first `SuspendThread` and stopped nothing, and one that
    /// suspended threads and then gave up, which DID stop the game. So this is
    /// an upper bound on the number of times the game was stopped to install,
    /// and the per-window rows are what resolve it exactly. An earlier version
    /// of this comment asserted that a refusal suspends nothing; it was wrong,
    /// and the census exists partly because nothing could have caught that.
    pub install_freeze_count: u16,
    pub clock_hour_raw: i32,
    /// Collector-owned ordinal of the stable read-only clock observation.
    /// This is not an engine tick.
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

/// Recorder-authored fixed acknowledgement. `ready_sha256` hashes the entire
/// authenticated 512-byte `H4RD` payload, so the ACK transitively binds all
/// collector claims. `public_receipt_core_sha256` binds the canonical READY
/// receipt core, which excludes `ready_ack_sha256`. The published JSON can then
/// include both this core digest and the exact ACK digest without a hash cycle.
/// This field is not a digest of that final JSON envelope.
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

/// SHA-256 helper for binding the exact authenticated `H4RD` bytes into ACK.
pub fn capture_ready_payload_sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

// --- helpers ----------------------------------------------------------------

fn validate_ready(value: &CaptureReadyV2) -> Result<(), PayloadError> {
    CaptureReadyFlags::from_bits(value.flags.bits())?;
    // Re-run on the way OUT as well as in. `CollectorFeatureBits` can only be
    // built through `from_bits` outside this module, but the encoder is the
    // last place an in-process construction can be caught before an unnameable
    // build claim is signed and shipped.
    CollectorFeatureBits::from_bits(value.collector_features.bits())?;
    // Every REQUIRED bit must be present; the optional identity bit may be
    // present or absent. `from_bits` above has already refused anything outside
    // KNOWN_MASK, so this cannot become a hole for an unnamed bit.
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
    // Two DIFFERENT questions, which used to share one `||` and one message.
    //
    // `allowed_hook_count` is the producer restating its own compiled
    // `PINNED_PRODUCTION_HOOK_COUNT_V1`, so it can only disagree with ours if
    // the two binaries were built from different revisions of the pinned
    // contract. That is a build fault and it names itself.
    if value.allowed_hook_count != crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1 {
        return Err(PayloadError::ProductionHookContractSkew {
            declared: value.allowed_hook_count,
            required: crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1,
        });
    }
    // `active_hook_count` is counted from the install outcomes, so under
    // install isolation it is legitimately LESS than the allowlist when a
    // non-mandatory gameplay producer failed to patch. Whether that voids the
    // capture is answered in exactly one place -- `HookAllowlistEntryV1::
    // mandatory`, enforced by `hook_install_outcomes_preimage`, which refuses
    // any outcome roster whose mandatory entries are not `Installed`. Refusing
    // `active < allowed` here was a second, undeclared answer to that same
    // question, and it answered it the other way: it aborted all 73 domains
    // over one domain's producer. What remains refused is a count ABOVE the
    // allowlist, which no isolation story can explain.
    if value.active_hook_count > value.allowed_hook_count {
        return Err(PayloadError::HookCountExceedsAllowlist {
            active: value.active_hook_count,
            allowed: value.allowed_hook_count,
        });
    }
    // A LIVE INLINE DETOUR MEANS THE GAME WAS STOPPED, and this is where the
    // freeze count stops being a field anybody can forget to fill.
    //
    // The pinned roster's `bootstrap.*` entries are import-slot swaps: one
    // aligned pointer store into hoi4.exe's IAT, no code rewritten, no thread
    // suspended. Every OTHER entry is an inline detour, and `inline_detour` is
    // the only way one is patched -- enforced by that crate's own
    // `no_direct_minhook_enable_outside_this_crate` -- which takes a process
    // freeze before it calls MinHook, on every path including the failure path.
    // So an `active_hook_count` above the import count with a freeze count of
    // zero is not a degraded capture, it is a producer whose count is not
    // wired to its freezes.
    //
    // THIS IS THE CHECK THAT MAKES THE FIELD NON-VACUOUS. Without it a
    // regression that stopped stamping the counter would publish zeroes, and a
    // zero is exactly what the field looked like for the whole time the counter
    // had no consumers -- the defect this closes would have re-opened silently
    // and every t55-style census would have read "no freezes" off a capture
    // that stopped the game a dozen times.
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
        // The control-version u16 occupies the RecorderHello reserved slot.
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
        // Layout: the exact 68-byte base hello, then the capability word LE.
        assert_eq!(wire[..COLLECTOR_HELLO_LEN], encode_collector_hello(&fields));
        assert_eq!(wire[COLLECTOR_HELLO_LEN..], bits.to_le_bytes());
        assert_eq!(
            decode_collector_hello_with_capabilities(&wire).unwrap(),
            (fields, bits)
        );

        // The base 68-byte form is NOT this payload: exact-length deny.
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

    /// The exact receipt the harness-side rebuild is pinned against.
    ///
    /// These are run t63's own fields (`runs/lp_refusal_volume/evidence/
    /// t63_CAPTURE_READY_V2.json`), with one substitution: `pipe_token_sha256`
    /// is rebound to the receipt-dialect fixture's token, because that token is
    /// the only one still known and `validate_ready_token_binding` requires the
    /// two to agree. Nothing else is touched.
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
            allowed_hook_count: 45,
            active_hook_count: 45,
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
                "54348f3d32277a114474c0a2b667ddf32cab6388adfd2a42500cac79ea8eaa79",
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

    /// THE PRODUCER SIDE OF THE FIXTURE THE HARNESS REBUILD IS PINNED AGAINST.
    ///
    /// `PassiveCaptureReady.ReceiptDialects.fixture.v1.json` carries two cases
    /// derived from run t63 (`t63_install_freeze_13`, `t63_install_freeze_1000`)
    /// and `Test-PassiveCapturePolicy.ps1` requires the PowerShell rebuild to
    /// reproduce their digests. Those digests are only worth pinning if they are
    /// what THIS encoder emits, so the same two vectors are asserted here, off
    /// `encode_authenticated` itself.
    ///
    /// WHY IT EXISTS. `install_freeze_count` took CaptureReadyV2 offset 50 from
    /// a reserved zero; the harness rebuild kept writing that zero, and because
    /// installing an inline detour freezes the process, every capture that
    /// installs one was refused on `ready_payload_sha256` for days (runs t57,
    /// t58, t59 and t60 on C:, t62 and t63 on L:, all reporting 13).
    /// The whole test suite stayed green because every pinned fixture receipt
    /// predated offset 50 and genuinely had a zero there. Changing offset 50's
    /// encoding, or dropping the field back to a constant on either side, now
    /// fails here and in the PowerShell policy suite together.
    #[test]
    fn passive_capture_ready_pins_the_harness_rebuild_vectors_at_offset_50() {
        let token = b"f743c8f124d2572ad18b99db033554ba";
        let thirteen = t63_ready_fields(13)
            .encode_authenticated(token)
            .expect("t63 fields are a valid CaptureReadyV2");
        assert_eq!(read_u16(&thirteen, 50), 13);
        assert_eq!(
            to_hex32(&capture_ready_payload_sha256(&thirteen)),
            "e8b08ebca73f6b9d4821931b3c53f74dc55e7c48bc203b0f6a69f4b596ad16b8"
        );
        // A SECOND, DIFFERENT COUNT. One vector is satisfied by an encoder that
        // hardcodes 13 exactly as the old one hardcoded 0, and 1000 does not fit
        // in a byte either.
        let thousand = t63_ready_fields(1000)
            .encode_authenticated(token)
            .expect("t63 fields are a valid CaptureReadyV2");
        assert_eq!(read_u16(&thousand, 50), 1000);
        assert_eq!(
            to_hex32(&capture_ready_payload_sha256(&thousand)),
            "5089b85d8f0fa835790baa90d45984c8c1b64036e88da2f7070ac616aff27bfe"
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

    /// The campaign identity is optional on the wire and mandatory nowhere:
    /// a collector that established it says so, a collector that could not
    /// still produces an admissible READY, and both round-trip.
    ///
    /// kills: folding IDENTITY_V1_STREAM into REQUIRED_PASSIVE_CAPTURE makes
    /// the second half fail, which is the exact defect that would turn "we
    /// could not tell which campaign this was" into a refused capture. Leaving
    /// the bit out of KNOWN_MASK makes the first half fail at `from_bits`.
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

    /// Optional does not mean lax: a READY missing any required readiness bit
    /// is still refused, and an unnamed bit is still refused.
    ///
    /// kills: relaxing the required check to "any known bits" admits a
    /// collector that never validated the target, the DLL, or the hook
    /// allowlist.
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
        // REQUIRED, not KNOWN_MASK: the optional IDENTITY_V1_STREAM bit is
        // part of KNOWN_MASK and this fixture does not claim it.
        assert_eq!(
            read_u32(&wire, 12),
            CaptureReadyFlags::REQUIRED_PASSIVE_CAPTURE
        );
        assert_eq!(read_u32(&wire, 16), fields.pid);
        // The build declaration sits in what used to be reserved-must-be-zero,
        // at the same offset and the same width, so nothing after it moved.
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
        // ABSENT, never `false`. A consumer that reads this as "clean build"
        // is the t35 defect.
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

        // Every assigned bit has a name, and no name is duplicated: the
        // manifest and an operator's `--features` line use the same words.
        let named = CollectorFeatureBits::FEATURE_NAMES
            .iter()
            .fold(0u32, |acc, (bit, _)| acc | bit);
        assert_eq!(named, CollectorFeatureBits::KNOWN_MASK & !CollectorFeatureBits::DECLARED);
        let mut names: Vec<&str> = CollectorFeatureBits::FEATURE_NAMES
            .iter()
            .map(|(_, name)| *name)
            .collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
        // Every named feature is diagnostic; the declaration bit is not.
        assert_eq!(CollectorFeatureBits::DIAGNOSTIC_MASK, named);

        assert_eq!(
            CollectorFeatureBits::from_bits(1 << 31),
            Err(PayloadError::UnknownCollectorFeatureBits(1 << 31))
        );

        // Bits 3, 4 and 5 are RETIRED, not merely unassigned: they carried the
        // three `unshipped-*` evidence features. Every other assertion in this
        // test derives from the masks, so handing one of those numbers to a new
        // feature would make it known and leave the whole file green. This is
        // the one place that would turn red, which is the point -- an old
        // diagnostic capture declaring bit 3, 4 or 5 must never decode as a new
        // clean one. A genuinely new feature takes the next free bit instead.
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
        // A collector built before the declaration existed sends the old
        // all-zero reserved word. That capture is still admissible; what it is
        // not is a capture whose build is KNOWN.
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
        // A declared-length mutation has to actually change the encoded value.
        // CAPTURE_READY_LEN is 512, so the low bytes of its little-endian u16
        // at offset 6 and u32 at offset 8 are already 0x00: overwriting them
        // with 0 left the wire byte-identical, and the decoder was right to
        // return the canonical payload. Writing 1 declares 513 in both fields.
        let mutations: &[(usize, u8, PayloadError)] = &[
            (0, b'X', PayloadError::InvalidMagic),
            (4, 1, PayloadError::UnsupportedVersion(1)),
            (6, 1, PayloadError::InvalidDeclaredLength),
            (8, 1, PayloadError::InvalidDeclaredLength),
            // Offset 20 is no longer reserved: it is the collector's build
            // declaration. An unassigned bit and a feature set that omits the
            // declaration bit are its two refusals.
            (
                23,
                0x80,
                PayloadError::UnknownCollectorFeatureBits(0x8000_0000),
            ),
            (
                20,
                CollectorFeatureBits::RANGE_TRACER as u8,
                PayloadError::UndeclaredCollectorFeatureBits(
                    CollectorFeatureBits::RANGE_TRACER,
                ),
            ),
            // Offset 50 is no longer reserved either: it is the install freeze
            // count. Its refusal is not "nonzero" -- a count IS nonzero on
            // every real capture -- it is ZERO beside live inline detours, so
            // the mutation that has to be caught here is the one that erases
            // it. Written as a whole-u16 clear because a single-byte write
            // could not reach a two-byte field's high half.
            (50, 0, PayloadError::UncountedInstallFreezes {
                active: crate::passive_ready::PINNED_PRODUCTION_HOOK_COUNT_V1,
                import_only: crate::passive_ready::PINNED_PRODUCTION_IMPORT_HOOK_COUNT_V1,
            }),
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
        // AND THE FIELD IS A FULL `u16`, not a byte with a spare beside it. A
        // decoder that read only the low byte would report 1 for a capture that
        // froze 257 times and would pass every assertion above, because the
        // fixture's count fits in one byte. This is the only assertion that
        // fails on that mistake.
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
        // Recompute a valid tag to prove closed-flag validation, not HMAC
        // failure, rejects the altered semantic word.
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
        // A collector built from a different revision of the pinned contract
        // restates ITS OWN roster size here. This is the exact shape live run
        // t16 hit (a collector.dll built before fourteen specs were pinned
        // reported allowed=41 against a required 55), and it must name the
        // build fault rather than read as a hook install failure.
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

        // Install isolation: a non-mandatory gameplay producer that failed to
        // patch lowers the active count, and that is a capture worth having.
        // `HookAllowlistEntryV1::mandatory` is the one place that decides
        // whether a hook failing voids the capture, and it is enforced by
        // `hook_install_outcomes_preimage`, not by this count.
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
