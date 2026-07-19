//! Shared, exact-byte H4CT v1 typed payload codecs
//! (`extractor/documents/DUPLEX_CONTROL_TRANSPORT.md` §3).
//!
//! These are the single implementation of every H4CT payload the recorder host
//! and the injected collector exchange, so neither side can drift. Each payload
//! begins with the common `payload_schema: u16 = 1, reserved: u16 = 0` prefix;
//! decoding is deny-by-default and exact-length: unknown schema, wrong length,
//! nonzero reserved fields, zero IDs, and unknown enums all reject before a
//! typed consumer sees the value. There are no sentinel values and no
//! length-guessed schemas.

pub const PAYLOAD_SCHEMA: u16 = 1;

const PREFIX_LEN: usize = 4;

pub const RECORDER_HELLO_LEN: usize = 68;
pub const COLLECTOR_HELLO_LEN: usize = 68;
pub const COLLECTOR_HELLO_WITH_CAPABILITIES_LEN: usize = 72;
pub const HEARTBEAT_CHALLENGE_LEN: usize = 28;
pub const HEARTBEAT_PROOF_LEN: usize = 32;
pub const FIRST_COMPACT_FRAME_ACK_LEN: usize = 56;

/// `HeartbeatProof.state` closed enum (`§3`).
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

// --- helpers ----------------------------------------------------------------

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
}
