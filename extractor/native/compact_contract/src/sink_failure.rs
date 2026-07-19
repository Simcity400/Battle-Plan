//! Dependency-neutral H4CT sink-failure payload contract shared by collector and
//! recorder.
//!
//! The collector is the only producer of these terminal reports and the recorder
//! is the only consumer, exactly like the CFA1 payload in [`crate::first_compact_ack`].
//! Both endpoints therefore encode and decode this 64-byte payload through this
//! single module so a producer in the collector crate and a consumer in the
//! recorder-host crate can never drift into two silently divergent layouts.
//!
//! This module is deliberately MessageKind-neutral: the two control-transport
//! envelope kinds (`SinkSetupFailure` / `SinkTerminalFailure`) belong to the
//! recorder-host control transport, not to the payload. The intrinsic
//! [`SinkFailureKind`] (`Setup` / `Terminal`) selects the codec branch, and the
//! recorder-host `transport_status` shim maps its `MessageKind` onto it.
//!
//! These terminal reports are never appended to H4CR. When either authoritative
//! sink fails, both stop together; the independent control pipe carries the
//! requested/opened topology and final progress needed to explain the failure.

use core::fmt;

const PAYLOAD_SCHEMA: u16 = 1;
pub const SINK_FAILURE_PAYLOAD_LEN: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SinkTopology(u8);

impl SinkTopology {
    pub const FILE: u8 = 1 << 0;
    pub const PIPE: u8 = 1 << 1;
    pub const KNOWN_MASK: u8 = Self::FILE | Self::PIPE;

    pub fn from_bits(bits: u8) -> Result<Self, SinkStatusError> {
        if bits & !Self::KNOWN_MASK != 0 {
            Err(SinkStatusError::UnknownTopologyBits(bits))
        } else {
            Ok(Self(bits))
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum SinkErrorClass {
    Win32 = 1,
    AccessDenied = 2,
    BrokenPipe = 3,
    DiskFull = 4,
    ShortWrite = 5,
    Cancelled = 6,
    DeadlineExpired = 7,
    OtherIo = 8,
}

impl SinkErrorClass {
    fn from_wire(value: u16) -> Result<Self, SinkStatusError> {
        match value {
            1 => Ok(Self::Win32),
            2 => Ok(Self::AccessDenied),
            3 => Ok(Self::BrokenPipe),
            4 => Ok(Self::DiskFull),
            5 => Ok(Self::ShortWrite),
            6 => Ok(Self::Cancelled),
            7 => Ok(Self::DeadlineExpired),
            8 => Ok(Self::OtherIo),
            other => Err(SinkStatusError::UnknownErrorClass(other)),
        }
    }
}

/// Intrinsic failure branch. The recorder-host control transport maps its two
/// `MessageKind` envelope kinds onto these; this module never names `MessageKind`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SinkFailureKind {
    Setup,
    Terminal,
}

/// Both failure messages are terminal by construction. The explicit wire byte
/// prevents a future producer from silently treating the same schema as a
/// recoverable warning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SinkTerminality {
    AttemptTerminal = 1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SinkFailureReport {
    pub kind: SinkFailureKind,
    pub requested: SinkTopology,
    pub opened: SinkTopology,
    pub failed: SinkTopology,
    pub terminality: SinkTerminality,
    pub error_class: SinkErrorClass,
    pub win32_error: Option<u32>,
    pub last_complete_sequence: Option<u64>,
    pub failed_sequence: Option<u64>,
    pub file_bytes_committed: u64,
    pub pipe_bytes_committed: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SinkStatusError {
    /// A payload was delivered under a control envelope kind that is not one of
    /// the two authoritative sink-failure kinds. The wire kind is retained as a
    /// dependency-neutral `u16`; the recorder-host shim owns the `MessageKind`
    /// mapping that constructs this.
    WrongMessageKind(u16),
    InvalidLength {
        expected: usize,
        actual: usize,
    },
    UnsupportedSchema(u16),
    NonZeroReserved,
    UnknownTopologyBits(u8),
    EmptyRequestedTopology,
    EmptyFailedTopology,
    OpenedNotRequested,
    FailedNotRequested,
    SetupFailureWasOpened,
    TerminalFailureWasNotOpened,
    SetupHasProgress,
    TerminalMissingFailedSequence,
    InvalidSequenceProgress,
    BytesForUnopenedSink,
    UnknownErrorClass(u16),
    InvalidErrorPresence(u8),
    Win32ClassWithoutCode,
    InvalidTerminality(u8),
}

impl fmt::Display for SinkStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid sink status: {self:?}")
    }
}

impl std::error::Error for SinkStatusError {}

pub fn encode_sink_failure(
    report: &SinkFailureReport,
) -> Result<[u8; SINK_FAILURE_PAYLOAD_LEN], SinkStatusError> {
    validate_report(report)?;
    let mut payload = [0u8; SINK_FAILURE_PAYLOAD_LEN];
    payload[0..2].copy_from_slice(&PAYLOAD_SCHEMA.to_le_bytes());
    payload[4] = report.requested.bits();
    payload[5] = report.opened.bits();
    payload[6] = report.failed.bits();
    payload[7] = report.terminality as u8;
    payload[8..10].copy_from_slice(&(report.error_class as u16).to_le_bytes());
    if let Some(error) = report.win32_error {
        payload[10] = 1;
        payload[12..16].copy_from_slice(&error.to_le_bytes());
    }
    if let Some(sequence) = report.last_complete_sequence {
        payload[16] = 1;
        payload[24..32].copy_from_slice(&sequence.to_le_bytes());
    }
    if let Some(sequence) = report.failed_sequence {
        payload[32] = 1;
        payload[40..48].copy_from_slice(&sequence.to_le_bytes());
    }
    payload[48..56].copy_from_slice(&report.file_bytes_committed.to_le_bytes());
    payload[56..64].copy_from_slice(&report.pipe_bytes_committed.to_le_bytes());
    Ok(payload)
}

/// Decode a sink-failure payload for the explicitly expected intrinsic branch.
///
/// The expected [`SinkFailureKind`] is a codec argument (mirroring
/// [`crate::first_compact_ack::decode_first_compact_frame_ack`]'s explicit mode):
/// the wire is never sniffed to choose Setup vs Terminal.
pub fn decode_sink_failure(
    expected: SinkFailureKind,
    payload: &[u8],
) -> Result<SinkFailureReport, SinkStatusError> {
    if payload.len() != SINK_FAILURE_PAYLOAD_LEN {
        return Err(SinkStatusError::InvalidLength {
            expected: SINK_FAILURE_PAYLOAD_LEN,
            actual: payload.len(),
        });
    }
    let schema = read_u16(payload, 0);
    if schema != PAYLOAD_SCHEMA {
        return Err(SinkStatusError::UnsupportedSchema(schema));
    }
    if payload[2..4] != [0, 0]
        || payload[11] != 0
        || payload[17..24] != [0; 7]
        || payload[33..40] != [0; 7]
    {
        return Err(SinkStatusError::NonZeroReserved);
    }
    if payload[7] != SinkTerminality::AttemptTerminal as u8 {
        return Err(SinkStatusError::InvalidTerminality(payload[7]));
    }
    let requested = SinkTopology::from_bits(payload[4])?;
    let opened = SinkTopology::from_bits(payload[5])?;
    let failed = SinkTopology::from_bits(payload[6])?;
    let error_class = SinkErrorClass::from_wire(read_u16(payload, 8))?;
    let win32_error = decode_optional_u32(payload[10], read_u32(payload, 12))?;
    let last_complete_sequence = decode_optional_u64(payload[16], read_u64(payload, 24))?;
    let failed_sequence = decode_optional_u64(payload[32], read_u64(payload, 40))?;
    let report = SinkFailureReport {
        kind: expected,
        requested,
        opened,
        failed,
        terminality: SinkTerminality::AttemptTerminal,
        error_class,
        win32_error,
        last_complete_sequence,
        failed_sequence,
        file_bytes_committed: read_u64(payload, 48),
        pipe_bytes_committed: read_u64(payload, 56),
    };
    validate_report(&report)?;
    Ok(report)
}

fn validate_report(report: &SinkFailureReport) -> Result<(), SinkStatusError> {
    if report.requested.is_empty() {
        return Err(SinkStatusError::EmptyRequestedTopology);
    }
    if report.failed.is_empty() {
        return Err(SinkStatusError::EmptyFailedTopology);
    }
    if !report.requested.contains(report.opened) {
        return Err(SinkStatusError::OpenedNotRequested);
    }
    if !report.requested.contains(report.failed) {
        return Err(SinkStatusError::FailedNotRequested);
    }
    if report.error_class == SinkErrorClass::Win32
        && !report.win32_error.is_some_and(|code| code != 0)
    {
        return Err(SinkStatusError::Win32ClassWithoutCode);
    }
    if report.last_complete_sequence == Some(u64::MAX) || report.failed_sequence == Some(u64::MAX) {
        return Err(SinkStatusError::InvalidSequenceProgress);
    }
    match report.kind {
        SinkFailureKind::Setup => {
            if report.opened.intersects(report.failed) {
                return Err(SinkStatusError::SetupFailureWasOpened);
            }
            if report.last_complete_sequence.is_some()
                || report.failed_sequence.is_some()
                || report.file_bytes_committed != 0
                || report.pipe_bytes_committed != 0
            {
                return Err(SinkStatusError::SetupHasProgress);
            }
        }
        SinkFailureKind::Terminal => {
            if !report.opened.contains(report.failed) {
                return Err(SinkStatusError::TerminalFailureWasNotOpened);
            }
            let failed = report
                .failed_sequence
                .ok_or(SinkStatusError::TerminalMissingFailedSequence)?;
            match report.last_complete_sequence {
                Some(last) if last.checked_add(1) == Some(failed) => {}
                // IdentityV1 begins at sequence 0; the still-supported legacy
                // compact stream begins at sequence 1. A failure before the
                // first complete record therefore has exactly these two valid
                // origins and no prior-completion witness.
                None if failed <= 1 => {}
                _ => return Err(SinkStatusError::InvalidSequenceProgress),
            }
        }
    }
    if report.file_bytes_committed != 0
        && !report
            .opened
            .intersects(SinkTopology::from_bits(SinkTopology::FILE).unwrap())
    {
        return Err(SinkStatusError::BytesForUnopenedSink);
    }
    if report.pipe_bytes_committed != 0
        && !report
            .opened
            .intersects(SinkTopology::from_bits(SinkTopology::PIPE).unwrap())
    {
        return Err(SinkStatusError::BytesForUnopenedSink);
    }
    Ok(())
}

fn decode_optional_u32(presence: u8, value: u32) -> Result<Option<u32>, SinkStatusError> {
    match presence {
        0 if value == 0 => Ok(None),
        1 => Ok(Some(value)),
        other => Err(SinkStatusError::InvalidErrorPresence(other)),
    }
}

fn decode_optional_u64(presence: u8, value: u64) -> Result<Option<u64>, SinkStatusError> {
    match presence {
        0 if value == 0 => Ok(None),
        1 => Ok(Some(value)),
        _ => Err(SinkStatusError::InvalidSequenceProgress),
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topology(bits: u8) -> SinkTopology {
        SinkTopology::from_bits(bits).unwrap()
    }

    pub(super) fn setup_failure() -> SinkFailureReport {
        SinkFailureReport {
            kind: SinkFailureKind::Setup,
            requested: topology(SinkTopology::FILE | SinkTopology::PIPE),
            opened: topology(SinkTopology::FILE),
            failed: topology(SinkTopology::PIPE),
            terminality: SinkTerminality::AttemptTerminal,
            error_class: SinkErrorClass::Win32,
            win32_error: Some(5),
            last_complete_sequence: None,
            failed_sequence: None,
            file_bytes_committed: 0,
            pipe_bytes_committed: 0,
        }
    }

    pub(super) fn terminal_failure() -> SinkFailureReport {
        SinkFailureReport {
            kind: SinkFailureKind::Terminal,
            requested: topology(SinkTopology::FILE | SinkTopology::PIPE),
            opened: topology(SinkTopology::FILE | SinkTopology::PIPE),
            failed: topology(SinkTopology::PIPE),
            terminality: SinkTerminality::AttemptTerminal,
            error_class: SinkErrorClass::BrokenPipe,
            win32_error: Some(109),
            last_complete_sequence: Some(8),
            failed_sequence: Some(9),
            file_bytes_committed: 4_096,
            pipe_bytes_committed: 3_100,
        }
    }

    #[test]
    fn setup_and_terminal_reports_round_trip_exactly() {
        for report in [setup_failure(), terminal_failure()] {
            let payload = encode_sink_failure(&report).unwrap();
            assert_eq!(decode_sink_failure(report.kind, &payload).unwrap(), report);
        }
    }

    #[test]
    fn topology_sequence_and_terminality_mutants_are_rejected() {
        let report = terminal_failure();
        let base = encode_sink_failure(&report).unwrap();
        for mutate in [4usize, 5, 6, 7, 17, 33] {
            let mut payload = base;
            payload[mutate] ^= 0x80;
            assert!(decode_sink_failure(SinkFailureKind::Terminal, &payload).is_err());
        }
        let mut gap = base;
        gap[40..48].copy_from_slice(&11u64.to_le_bytes());
        assert!(matches!(
            decode_sink_failure(SinkFailureKind::Terminal, &gap),
            Err(SinkStatusError::InvalidSequenceProgress)
        ));
    }

    #[test]
    fn setup_cannot_claim_progress_and_terminal_failure_must_name_opened_sink() {
        let mut setup = setup_failure();
        setup.file_bytes_committed = 1;
        assert!(matches!(
            encode_sink_failure(&setup),
            Err(SinkStatusError::SetupHasProgress)
        ));

        let mut terminal = terminal_failure();
        terminal.opened = topology(SinkTopology::FILE);
        assert!(matches!(
            encode_sink_failure(&terminal),
            Err(SinkStatusError::TerminalFailureWasNotOpened)
        ));
    }

    #[test]
    fn terminal_first_write_accepts_both_declared_stream_origins_only() {
        let mut report = terminal_failure();
        report.last_complete_sequence = None;
        report.failed_sequence = Some(0);
        assert!(encode_sink_failure(&report).is_ok());
        report.failed_sequence = Some(1);
        assert!(encode_sink_failure(&report).is_ok());
        report.failed_sequence = Some(2);
        assert_eq!(
            encode_sink_failure(&report),
            Err(SinkStatusError::InvalidSequenceProgress)
        );
    }

    #[test]
    fn length_schema_reserved_and_error_presence_fail_closed() {
        let base = encode_sink_failure(&terminal_failure()).unwrap();
        assert!(matches!(
            decode_sink_failure(SinkFailureKind::Terminal, &base[..63]),
            Err(SinkStatusError::InvalidLength { .. })
        ));
        let mut schema = base;
        schema[0] ^= 1;
        assert!(matches!(
            decode_sink_failure(SinkFailureKind::Terminal, &schema),
            Err(SinkStatusError::UnsupportedSchema(_))
        ));
        let mut reserved = base;
        reserved[2] = 1;
        assert!(matches!(
            decode_sink_failure(SinkFailureKind::Terminal, &reserved),
            Err(SinkStatusError::NonZeroReserved)
        ));
        let mut presence = base;
        presence[10] = 2;
        assert!(matches!(
            decode_sink_failure(SinkFailureKind::Terminal, &presence),
            Err(SinkStatusError::InvalidErrorPresence(2))
        ));
    }

    #[test]
    fn win32_class_requires_a_nonzero_code() {
        let mut report = terminal_failure();
        report.error_class = SinkErrorClass::Win32;
        report.win32_error = None;
        assert!(matches!(
            encode_sink_failure(&report),
            Err(SinkStatusError::Win32ClassWithoutCode)
        ));
    }
}
