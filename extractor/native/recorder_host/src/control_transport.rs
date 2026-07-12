//! Fail-closed framing and byte-I/O seam for the recorder/collector control pipe.
//!
//! This module deliberately owns no pipe names, Windows handles, injection hooks,
//! reconnect policy, timers, or consumer state machines.  Live wiring supplies a
//! [`ControlIo`] implementation and an unpredictable, per-attach
//! [`SessionCapability`].
//! Consequently, no decoded control message can call or request DLL injection.

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::sync::{Mutex, OnceLock};

pub const CONTROL_MAGIC: &[u8; 4] = b"H4CT";
pub const CONTROL_VERSION: u16 = 1;
pub const CONTROL_HEADER_LEN: usize = 48;
pub const CONTROL_MAX_PAYLOAD_LEN: usize = 1 << 20;

const CHECKSUM_OFFSET: usize = 40;
const RESERVED_OFFSET: usize = 44;

/// Unpredictable identifier minted once for a single attach attempt.
///
/// The transport does not generate this value: the product integration must use
/// the operating-system CSPRNG and must never reuse an id after disconnect.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionId([u8; 16]);

impl SessionId {
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Move-only authority to construct one endpoint for a control session.
///
/// Claims are process-global and permanent: dropping a capability or its
/// channel never makes the session id claimable again. The two endpoints live
/// in separate processes, so each process independently claims the same
/// CSPRNG-minted id exactly once.
pub struct SessionCapability {
    session_id: SessionId,
}

impl SessionCapability {
    /// Permanently claims a CSPRNG-minted id for one channel in this process.
    pub fn claim(session_id: [u8; 16]) -> Result<Self, ControlError> {
        if session_id == [0; 16] {
            return Err(ControlError::InvalidSessionId);
        }

        let mut claimed = claimed_session_ids()
            .lock()
            .map_err(|_| ControlError::SessionClaimRegistryPoisoned)?;
        if !claimed.insert(session_id) {
            return Err(ControlError::SessionAlreadyClaimed(SessionId(session_id)));
        }

        Ok(Self {
            session_id: SessionId(session_id),
        })
    }

    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }
}

fn claimed_session_ids() -> &'static Mutex<BTreeSet<[u8; 16]>> {
    static CLAIMED: OnceLock<Mutex<BTreeSet<[u8; 16]>>> = OnceLock::new();
    CLAIMED.get_or_init(|| Mutex::new(BTreeSet::new()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointRole {
    Recorder,
    Collector,
}

impl EndpointRole {
    const fn opposite(self) -> Self {
        match self {
            Self::Recorder => Self::Collector,
            Self::Collector => Self::Recorder,
        }
    }
}

/// Closed v1 control-message registry. There is intentionally no Inject or
/// Attach variant; injection remains outside this transport's authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum MessageKind {
    RecorderHello = 1,
    CollectorHello = 2,
    HeartbeatChallenge = 3,
    HeartbeatProof = 4,
    FirstCompactFrameAck = 5,
    RolloverPropose = 6,
    RolloverAck = 7,
    EpochTokenOffer = 8,
    EpochTokenEcho = 9,
    DeclarationTableReady = 10,
    DeclarationTableAck = 11,
    EpochSealReady = 12,
    EpochSealAck = 13,
}

impl MessageKind {
    pub const fn sender(self) -> EndpointRole {
        match self {
            Self::RecorderHello
            | Self::HeartbeatChallenge
            | Self::FirstCompactFrameAck
            | Self::RolloverAck
            | Self::EpochTokenOffer
            | Self::DeclarationTableAck
            | Self::EpochSealAck => EndpointRole::Recorder,
            Self::CollectorHello
            | Self::HeartbeatProof
            | Self::RolloverPropose
            | Self::EpochTokenEcho
            | Self::DeclarationTableReady
            | Self::EpochSealReady => EndpointRole::Collector,
        }
    }

    fn from_wire(value: u16) -> Result<Self, ControlError> {
        match value {
            1 => Ok(Self::RecorderHello),
            2 => Ok(Self::CollectorHello),
            3 => Ok(Self::HeartbeatChallenge),
            4 => Ok(Self::HeartbeatProof),
            5 => Ok(Self::FirstCompactFrameAck),
            6 => Ok(Self::RolloverPropose),
            7 => Ok(Self::RolloverAck),
            8 => Ok(Self::EpochTokenOffer),
            9 => Ok(Self::EpochTokenEcho),
            10 => Ok(Self::DeclarationTableReady),
            11 => Ok(Self::DeclarationTableAck),
            12 => Ok(Self::EpochSealReady),
            13 => Ok(Self::EpochSealAck),
            other => Err(ControlError::UnknownMessageKind(other)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlFrame {
    pub session_id: SessionId,
    pub sequence: u64,
    pub kind: MessageKind,
    pub payload: Vec<u8>,
}

/// Minimal seam implemented later by a Windows named-pipe adapter and by tests.
///
/// `send_all` must either transfer the complete slice or return an error. A
/// partial transfer followed by an error is still an error: [`ControlChannel`]
/// poisons itself and permits no retry under the same session id.
pub trait ControlIo {
    fn send_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn recv_exact(&mut self, bytes: &mut [u8]) -> io::Result<()>;
}

#[derive(Debug)]
pub enum ControlError {
    Poisoned,
    Io(io::ErrorKind),
    Truncated,
    BadMagic,
    UnsupportedVersion(u16),
    UnsupportedHeaderLen(u16),
    UnknownMessageKind(u16),
    NonZeroFlags(u16),
    PayloadTooLarge(u32),
    InvalidSessionId,
    SessionAlreadyClaimed(SessionId),
    SessionClaimRegistryPoisoned,
    NonZeroReserved(u32),
    SessionMismatch {
        expected: SessionId,
        actual: SessionId,
    },
    SequenceMismatch {
        expected: u64,
        actual: u64,
    },
    SequenceExhausted,
    WrongDirection {
        role: EndpointRole,
        kind: MessageKind,
    },
    ChecksumMismatch {
        expected: u32,
        actual: u32,
    },
    FrameLengthMismatch {
        expected: usize,
        actual: usize,
    },
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Poisoned => write!(f, "control channel is poisoned"),
            Self::Io(kind) => write!(f, "control I/O failed: {kind:?}"),
            Self::Truncated => write!(f, "truncated control frame"),
            Self::BadMagic => write!(f, "invalid control magic"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported control version {v}"),
            Self::UnsupportedHeaderLen(len) => {
                write!(f, "unsupported control header length {len}")
            }
            Self::UnknownMessageKind(kind) => write!(f, "unknown control message kind {kind}"),
            Self::NonZeroFlags(flags) => write!(f, "unsupported control flags {flags:#06x}"),
            Self::PayloadTooLarge(len) => write!(
                f,
                "control payload length {len} exceeds {CONTROL_MAX_PAYLOAD_LEN}"
            ),
            Self::InvalidSessionId => write!(f, "all-zero control session id is invalid"),
            Self::SessionAlreadyClaimed(session_id) => {
                write!(f, "control session id {session_id:?} was already claimed")
            }
            Self::SessionClaimRegistryPoisoned => {
                write!(f, "control session claim registry is poisoned")
            }
            Self::NonZeroReserved(value) => {
                write!(f, "nonzero reserved control field {value:#010x}")
            }
            Self::SessionMismatch { expected, actual } => {
                write!(f, "control session mismatch: {actual:?} != {expected:?}")
            }
            Self::SequenceMismatch { expected, actual } => {
                write!(f, "control sequence mismatch: {actual} != {expected}")
            }
            Self::SequenceExhausted => write!(f, "control sequence exhausted"),
            Self::WrongDirection { role, kind } => {
                write!(
                    f,
                    "{role:?} cannot send or receive {kind:?} in this direction"
                )
            }
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "control checksum mismatch: {actual:08x} != {expected:08x}"
            ),
            Self::FrameLengthMismatch { expected, actual } => {
                write!(f, "control frame length {actual} != expected {expected}")
            }
        }
    }
}

impl std::error::Error for ControlError {}

/// Stateful, strictly ordered endpoint for one control-pipe connection.
pub struct ControlChannel<I> {
    io: I,
    role: EndpointRole,
    session_id: SessionId,
    next_send_sequence: u64,
    next_recv_sequence: u64,
    poisoned: bool,
}

impl<I: ControlIo> ControlChannel<I> {
    pub fn new(io: I, role: EndpointRole, session: SessionCapability) -> Self {
        Self {
            io,
            role,
            session_id: session.session_id,
            next_send_sequence: 0,
            next_recv_sequence: 0,
            poisoned: false,
        }
    }

    pub fn send(&mut self, kind: MessageKind, payload: &[u8]) -> Result<(), ControlError> {
        self.ensure_usable()?;
        if kind.sender() != self.role {
            return self.fail(ControlError::WrongDirection {
                role: self.role,
                kind,
            });
        }
        if self.next_send_sequence == u64::MAX {
            return self.fail(ControlError::SequenceExhausted);
        }
        let frame = match encode_frame(self.session_id, self.next_send_sequence, kind, payload) {
            Ok(frame) => frame,
            Err(err) => return self.fail(err),
        };
        if let Err(err) = self.io.send_all(&frame) {
            return self.fail(map_io_error(err));
        }
        self.next_send_sequence += 1;
        Ok(())
    }

    pub fn recv(&mut self) -> Result<ControlFrame, ControlError> {
        self.ensure_usable()?;
        if self.next_recv_sequence == u64::MAX {
            return self.fail(ControlError::SequenceExhausted);
        }

        let mut header = [0u8; CONTROL_HEADER_LEN];
        if let Err(err) = self.io.recv_exact(&mut header) {
            return self.fail(map_io_error(err));
        }
        let payload_len = match parse_header(&header) {
            Ok(parsed) => parsed.payload_len,
            Err(err) => return self.fail(err),
        };
        let mut wire = Vec::with_capacity(CONTROL_HEADER_LEN + payload_len);
        wire.extend_from_slice(&header);
        wire.resize(CONTROL_HEADER_LEN + payload_len, 0);
        if let Err(err) = self.io.recv_exact(&mut wire[CONTROL_HEADER_LEN..]) {
            return self.fail(map_io_error(err));
        }
        let frame = match decode_frame(&wire, self.session_id, self.next_recv_sequence) {
            Ok(frame) => frame,
            Err(err) => return self.fail(err),
        };
        if frame.kind.sender() != self.role.opposite() {
            return self.fail(ControlError::WrongDirection {
                role: self.role,
                kind: frame.kind,
            });
        }
        self.next_recv_sequence += 1;
        Ok(frame)
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    #[cfg(test)]
    fn io(&self) -> &I {
        &self.io
    }

    fn ensure_usable(&self) -> Result<(), ControlError> {
        if self.poisoned {
            Err(ControlError::Poisoned)
        } else {
            Ok(())
        }
    }

    fn fail<T>(&mut self, error: ControlError) -> Result<T, ControlError> {
        self.poisoned = true;
        Err(error)
    }
}

struct ParsedHeader {
    kind: MessageKind,
    payload_len: usize,
    session_id: SessionId,
    sequence: u64,
    checksum: u32,
}

pub fn encode_frame(
    session_id: SessionId,
    sequence: u64,
    kind: MessageKind,
    payload: &[u8],
) -> Result<Vec<u8>, ControlError> {
    if session_id.0 == [0; 16] {
        return Err(ControlError::InvalidSessionId);
    }
    if sequence == u64::MAX {
        return Err(ControlError::SequenceExhausted);
    }
    if payload.len() > CONTROL_MAX_PAYLOAD_LEN {
        return Err(ControlError::PayloadTooLarge(
            payload.len().min(u32::MAX as usize) as u32,
        ));
    }

    let mut wire = vec![0u8; CONTROL_HEADER_LEN + payload.len()];
    wire[0..4].copy_from_slice(CONTROL_MAGIC);
    wire[4..6].copy_from_slice(&CONTROL_VERSION.to_le_bytes());
    wire[6..8].copy_from_slice(&(CONTROL_HEADER_LEN as u16).to_le_bytes());
    wire[8..10].copy_from_slice(&(kind as u16).to_le_bytes());
    // bytes 10..12 are flags; v1 requires zero.
    wire[12..16].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    wire[16..32].copy_from_slice(&session_id.0);
    wire[32..40].copy_from_slice(&sequence.to_le_bytes());
    // bytes 40..44 are zero while calculating the checksum.
    // bytes 44..48 are reserved and zero in v1.
    wire[CONTROL_HEADER_LEN..].copy_from_slice(payload);
    let checksum = crc32(&wire);
    wire[CHECKSUM_OFFSET..RESERVED_OFFSET].copy_from_slice(&checksum.to_le_bytes());
    Ok(wire)
}

pub fn decode_frame(
    wire: &[u8],
    expected_session_id: SessionId,
    expected_sequence: u64,
) -> Result<ControlFrame, ControlError> {
    if expected_session_id.0 == [0; 16] {
        return Err(ControlError::InvalidSessionId);
    }
    if wire.len() < CONTROL_HEADER_LEN {
        return Err(ControlError::Truncated);
    }
    let parsed = parse_header(&wire[..CONTROL_HEADER_LEN])?;
    let expected_len = CONTROL_HEADER_LEN + parsed.payload_len;
    if wire.len() != expected_len {
        return Err(ControlError::FrameLengthMismatch {
            expected: expected_len,
            actual: wire.len(),
        });
    }
    if parsed.session_id != expected_session_id {
        return Err(ControlError::SessionMismatch {
            expected: expected_session_id,
            actual: parsed.session_id,
        });
    }
    if parsed.sequence != expected_sequence {
        return Err(ControlError::SequenceMismatch {
            expected: expected_sequence,
            actual: parsed.sequence,
        });
    }

    let mut checksummed = wire.to_vec();
    checksummed[CHECKSUM_OFFSET..RESERVED_OFFSET].fill(0);
    let actual = crc32(&checksummed);
    if actual != parsed.checksum {
        return Err(ControlError::ChecksumMismatch {
            expected: parsed.checksum,
            actual,
        });
    }

    Ok(ControlFrame {
        session_id: parsed.session_id,
        sequence: parsed.sequence,
        kind: parsed.kind,
        payload: wire[CONTROL_HEADER_LEN..].to_vec(),
    })
}

fn parse_header(header: &[u8]) -> Result<ParsedHeader, ControlError> {
    if header.len() < CONTROL_HEADER_LEN {
        return Err(ControlError::Truncated);
    }
    if &header[0..4] != CONTROL_MAGIC {
        return Err(ControlError::BadMagic);
    }
    let version = read_u16(header, 4);
    if version != CONTROL_VERSION {
        return Err(ControlError::UnsupportedVersion(version));
    }
    let header_len = read_u16(header, 6);
    if header_len as usize != CONTROL_HEADER_LEN {
        return Err(ControlError::UnsupportedHeaderLen(header_len));
    }
    let kind = MessageKind::from_wire(read_u16(header, 8))?;
    let flags = read_u16(header, 10);
    if flags != 0 {
        return Err(ControlError::NonZeroFlags(flags));
    }
    let payload_len = read_u32(header, 12);
    if payload_len as usize > CONTROL_MAX_PAYLOAD_LEN {
        return Err(ControlError::PayloadTooLarge(payload_len));
    }
    let mut session_id = [0u8; 16];
    session_id.copy_from_slice(&header[16..32]);
    if session_id == [0; 16] {
        return Err(ControlError::InvalidSessionId);
    }
    let sequence = read_u64(header, 32);
    if sequence == u64::MAX {
        return Err(ControlError::SequenceExhausted);
    }
    let reserved = read_u32(header, RESERVED_OFFSET);
    if reserved != 0 {
        return Err(ControlError::NonZeroReserved(reserved));
    }
    Ok(ParsedHeader {
        kind,
        payload_len: payload_len as usize,
        session_id: SessionId(session_id),
        sequence,
        checksum: read_u32(header, CHECKSUM_OFFSET),
    })
}

fn map_io_error(error: io::Error) -> ControlError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        ControlError::Truncated
    } else {
        ControlError::Io(error.kind())
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

/// IEEE CRC-32, used as an integrity checksum rather than an authenticator.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Default)]
    struct MemoryIo {
        inbound: VecDeque<u8>,
        outbound: Vec<u8>,
        fail_send: bool,
    }

    impl MemoryIo {
        fn with_inbound(bytes: Vec<u8>) -> Self {
            Self {
                inbound: bytes.into(),
                ..Self::default()
            }
        }
    }

    impl ControlIo for MemoryIo {
        fn send_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            if self.fail_send {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "fake disconnect"));
            }
            self.outbound.extend_from_slice(bytes);
            Ok(())
        }

        fn recv_exact(&mut self, bytes: &mut [u8]) -> io::Result<()> {
            for slot in bytes {
                *slot = self.inbound.pop_front().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "fake truncated input")
                })?;
            }
            Ok(())
        }
    }

    const SESSION: SessionId = SessionId([0x5a; 16]);

    fn fresh_session_bytes() -> [u8; 16] {
        static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
        let mut bytes = [0x5a; 16];
        bytes[8..].copy_from_slice(&NEXT_SESSION.fetch_add(1, Ordering::Relaxed).to_le_bytes());
        bytes
    }

    fn fresh_session() -> (SessionCapability, SessionId) {
        let capability = SessionCapability::claim(fresh_session_bytes()).unwrap();
        let session_id = capability.session_id();
        (capability, session_id)
    }

    #[test]
    fn frame_round_trip_preserves_every_field() {
        let wire = encode_frame(SESSION, 7, MessageKind::HeartbeatProof, b"proof").unwrap();
        let frame = decode_frame(&wire, SESSION, 7).unwrap();
        assert_eq!(frame.session_id, SESSION);
        assert_eq!(frame.sequence, 7);
        assert_eq!(frame.kind, MessageKind::HeartbeatProof);
        assert_eq!(frame.payload, b"proof");
    }

    #[test]
    fn ieee_crc32_matches_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn checksum_rejects_header_and_payload_corruption() {
        let original = encode_frame(SESSION, 0, MessageKind::CollectorHello, b"collector").unwrap();
        for offset in [16, CONTROL_HEADER_LEN] {
            let mut corrupt = original.clone();
            corrupt[offset] ^= 0x80;
            let error = decode_frame(&corrupt, SESSION, 0).unwrap_err();
            assert!(
                matches!(
                    error,
                    ControlError::SessionMismatch { .. } | ControlError::ChecksumMismatch { .. }
                ),
                "unexpected corruption error: {error}"
            );
        }
    }

    #[test]
    fn truncation_at_header_and_payload_fails_closed_and_poisons() {
        for truncate_header in [true, false] {
            let (session, session_id) = fresh_session();
            let wire =
                encode_frame(session_id, 0, MessageKind::CollectorHello, b"collector").unwrap();
            let truncated_len = if truncate_header {
                CONTROL_HEADER_LEN - 1
            } else {
                wire.len() - 1
            };
            let mut channel = ControlChannel::new(
                MemoryIo::with_inbound(wire[..truncated_len].to_vec()),
                EndpointRole::Recorder,
                session,
            );
            assert!(matches!(channel.recv(), Err(ControlError::Truncated)));
            assert!(channel.is_poisoned());
            assert!(matches!(channel.recv(), Err(ControlError::Poisoned)));
        }
    }

    #[test]
    fn duplicate_sequence_is_rejected_and_channel_stays_poisoned() {
        let (session, session_id) = fresh_session();
        let first = encode_frame(session_id, 0, MessageKind::CollectorHello, b"").unwrap();
        let mut input = first.clone();
        input.extend_from_slice(&first);
        let mut channel = ControlChannel::new(
            MemoryIo::with_inbound(input),
            EndpointRole::Recorder,
            session,
        );
        assert_eq!(channel.recv().unwrap().sequence, 0);
        assert!(matches!(
            channel.recv(),
            Err(ControlError::SequenceMismatch {
                expected: 1,
                actual: 0
            })
        ));
        assert!(matches!(channel.recv(), Err(ControlError::Poisoned)));
    }

    #[test]
    fn reordered_or_gapped_sequence_is_rejected() {
        let (session, session_id) = fresh_session();
        let wire = encode_frame(session_id, 1, MessageKind::CollectorHello, b"").unwrap();
        let mut channel = ControlChannel::new(
            MemoryIo::with_inbound(wire),
            EndpointRole::Recorder,
            session,
        );
        assert!(matches!(
            channel.recv(),
            Err(ControlError::SequenceMismatch {
                expected: 0,
                actual: 1
            })
        ));
        assert!(channel.is_poisoned());
    }

    #[test]
    fn stale_session_replay_is_rejected() {
        let stale =
            encode_frame(SessionId([0x11; 16]), 0, MessageKind::CollectorHello, b"").unwrap();
        assert!(matches!(
            decode_frame(&stale, SESSION, 0),
            Err(ControlError::SessionMismatch { .. })
        ));
    }

    #[test]
    fn role_rejects_messages_from_the_wrong_direction() {
        let (sender_session, _) = fresh_session();
        let mut sender =
            ControlChannel::new(MemoryIo::default(), EndpointRole::Recorder, sender_session);
        assert!(matches!(
            sender.send(MessageKind::CollectorHello, b""),
            Err(ControlError::WrongDirection { .. })
        ));
        assert!(sender.is_poisoned());

        let (receiver_session, receiver_session_id) = fresh_session();
        let wire = encode_frame(receiver_session_id, 0, MessageKind::RecorderHello, b"").unwrap();
        let mut receiver = ControlChannel::new(
            MemoryIo::with_inbound(wire),
            EndpointRole::Recorder,
            receiver_session,
        );
        assert!(matches!(
            receiver.recv(),
            Err(ControlError::WrongDirection { .. })
        ));
        assert!(receiver.is_poisoned());
    }

    #[test]
    fn send_sequence_advances_only_after_complete_fake_write() {
        let (session, session_id) = fresh_session();
        let mut sender = ControlChannel::new(MemoryIo::default(), EndpointRole::Recorder, session);
        sender.send(MessageKind::RecorderHello, b"one").unwrap();
        sender
            .send(MessageKind::HeartbeatChallenge, b"two")
            .unwrap();
        let output = &sender.io().outbound;

        let first_len = CONTROL_HEADER_LEN + 3;
        assert_eq!(
            decode_frame(&output[..first_len], session_id, 0)
                .unwrap()
                .sequence,
            0
        );
        assert_eq!(
            decode_frame(&output[first_len..], session_id, 1)
                .unwrap()
                .sequence,
            1
        );
    }

    #[test]
    fn send_failure_is_terminal_and_never_retried() {
        let io = MemoryIo {
            fail_send: true,
            ..MemoryIo::default()
        };
        let (session, _) = fresh_session();
        let mut sender = ControlChannel::new(io, EndpointRole::Recorder, session);
        assert!(matches!(
            sender.send(MessageKind::RecorderHello, b""),
            Err(ControlError::Io(io::ErrorKind::BrokenPipe))
        ));
        assert!(matches!(
            sender.send(MessageKind::RecorderHello, b""),
            Err(ControlError::Poisoned)
        ));
    }

    #[test]
    fn unknown_version_flags_kind_reserved_and_oversize_reject() {
        let base = encode_frame(SESSION, 0, MessageKind::CollectorHello, b"").unwrap();
        let mutations: &[(usize, &[u8], fn(&ControlError) -> bool)] = &[
            (4, &2u16.to_le_bytes(), |e| {
                matches!(e, ControlError::UnsupportedVersion(2))
            }),
            (8, &99u16.to_le_bytes(), |e| {
                matches!(e, ControlError::UnknownMessageKind(99))
            }),
            (10, &1u16.to_le_bytes(), |e| {
                matches!(e, ControlError::NonZeroFlags(1))
            }),
            (44, &1u32.to_le_bytes(), |e| {
                matches!(e, ControlError::NonZeroReserved(1))
            }),
            (
                12,
                &((CONTROL_MAX_PAYLOAD_LEN as u32) + 1).to_le_bytes(),
                |e| matches!(e, ControlError::PayloadTooLarge(_)),
            ),
        ];
        for (offset, bytes, predicate) in mutations {
            let mut wire = base.clone();
            wire[*offset..*offset + bytes.len()].copy_from_slice(bytes);
            let error = decode_frame(&wire, SESSION, 0).unwrap_err();
            assert!(predicate(&error), "unexpected error: {error}");
        }
    }

    #[test]
    fn zero_session_and_max_sequence_are_never_admitted() {
        assert!(matches!(
            SessionCapability::claim([0; 16]),
            Err(ControlError::InvalidSessionId)
        ));
        assert!(matches!(
            encode_frame(SESSION, u64::MAX, MessageKind::RecorderHello, b""),
            Err(ControlError::SequenceExhausted)
        ));
    }

    #[test]
    fn accepted_sequence_cannot_be_replayed_by_reconstructing_same_session() {
        let session_bytes = fresh_session_bytes();
        let session = SessionCapability::claim(session_bytes).unwrap();
        let session_id = session.session_id();
        let replay = encode_frame(session_id, 0, MessageKind::CollectorHello, b"").unwrap();

        let mut channel = ControlChannel::new(
            MemoryIo::with_inbound(replay.clone()),
            EndpointRole::Recorder,
            session,
        );
        assert_eq!(channel.recv().unwrap().sequence, 0);
        drop(channel);

        assert!(matches!(
            SessionCapability::claim(session_bytes),
            Err(ControlError::SessionAlreadyClaimed(claimed)) if claimed == session_id
        ));
    }
}
