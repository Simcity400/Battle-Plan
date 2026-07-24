//! Exact collector-to-recorder H4CT report for terminal early-render startup
//! failures.
//!
//! The report remains inside the already admitted H4CT session. It preserves
//! the collector's UTF-8 error text byte-for-byte and binds it to the observed
//! process epoch, session id, startup phase, and (once available) launch BIND.
//! A report is diagnostic terminal evidence; it can never authorize capture.

use crate::campaign_identity::Sha256;
use crate::control_transport::CONTROL_MAX_PAYLOAD_LEN;
use core::fmt;

const MAGIC: &[u8; 4] = b"ERSF";
const VERSION: u16 = 1;
const FLAG_LAUNCH_BIND_PRESENT: u16 = 1;
const KNOWN_FLAGS: u16 = FLAG_LAUNCH_BIND_PRESENT;

pub const EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN: usize = 112;
pub const MAX_EARLY_RENDER_STARTUP_FAILURE_DETAIL_LEN: usize =
    CONTROL_MAX_PAYLOAD_LEN - EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum EarlyRenderStartupPhase {
    PostHelloPreBind = 1,
    PostBindPreFirst = 2,
    PostFirstPreCommit = 3,
    PostCommitBootstrap = 4,
}

impl EarlyRenderStartupPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PostHelloPreBind => "post_hello_pre_bind",
            Self::PostBindPreFirst => "post_bind_pre_first",
            Self::PostFirstPreCommit => "post_first_pre_commit",
            Self::PostCommitBootstrap => "post_commit_bootstrap",
        }
    }

    fn from_wire(value: u16) -> Result<Self, EarlyRenderStartupFailureCodecError> {
        match value {
            1 => Ok(Self::PostHelloPreBind),
            2 => Ok(Self::PostBindPreFirst),
            3 => Ok(Self::PostFirstPreCommit),
            4 => Ok(Self::PostCommitBootstrap),
            other => Err(EarlyRenderStartupFailureCodecError::UnknownPhase(other)),
        }
    }

    const fn requires_launch_bind(self) -> bool {
        !matches!(self, Self::PostHelloPreBind)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EarlyRenderStartupFailureV1 {
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub session_id: [u8; 16],
    pub phase: EarlyRenderStartupPhase,
    pub launch_bind_sha256: Option<[u8; 32]>,
    pub detail_sha256: [u8; 32],
    pub detail: String,
}

impl EarlyRenderStartupFailureV1 {
    pub fn new(
        process_id: u32,
        process_start_nonce: u64,
        session_id: [u8; 16],
        phase: EarlyRenderStartupPhase,
        launch_bind_sha256: Option<[u8; 32]>,
        detail: impl Into<String>,
    ) -> Result<Self, EarlyRenderStartupFailureCodecError> {
        let detail = detail.into();
        let report = Self {
            process_id,
            process_start_nonce,
            session_id,
            phase,
            launch_bind_sha256,
            detail_sha256: Sha256::digest(detail.as_bytes()),
            detail,
        };
        validate(&report)?;
        Ok(report)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EarlyRenderStartupFailureCodecError {
    TooShort(usize),
    PayloadTooLarge(usize),
    BadMagic,
    UnsupportedVersion(u16),
    UnknownPhase(u16),
    UnknownFlags(u16),
    ReservedNonZero,
    ZeroProcessId,
    ZeroProcessStartNonce,
    ZeroSessionId,
    MissingLaunchBind,
    UnexpectedLaunchBind,
    ZeroLaunchBind,
    EmptyDetail,
    DetailTooLarge(usize),
    DetailLengthMismatch {
        declared: usize,
        actual: usize,
    },
    InvalidUtf8,
    DetailDigestMismatch,
}

impl fmt::Display for EarlyRenderStartupFailureCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort(actual) => write!(
                f,
                "early-render startup failure payload length {actual} is below {EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN}"
            ),
            Self::PayloadTooLarge(actual) => write!(
                f,
                "early-render startup failure payload length {actual} exceeds {CONTROL_MAX_PAYLOAD_LEN}"
            ),
            Self::BadMagic => write!(f, "early-render startup failure magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported early-render startup failure version {version}")
            }
            Self::UnknownPhase(phase) => {
                write!(f, "unknown early-render startup failure phase {phase}")
            }
            Self::UnknownFlags(flags) => {
                write!(f, "unknown early-render startup failure flags 0x{flags:04x}")
            }
            Self::ReservedNonZero => {
                write!(f, "early-render startup failure reserved bytes are nonzero")
            }
            Self::ZeroProcessId => write!(f, "early-render startup failure process id is zero"),
            Self::ZeroProcessStartNonce => {
                write!(f, "early-render startup failure process nonce is zero")
            }
            Self::ZeroSessionId => write!(f, "early-render startup failure session id is zero"),
            Self::MissingLaunchBind => {
                write!(f, "bound early-render startup phase has no launch BIND digest")
            }
            Self::UnexpectedLaunchBind => {
                write!(f, "pre-BIND early-render startup phase carries a launch BIND digest")
            }
            Self::ZeroLaunchBind => {
                write!(f, "early-render startup failure launch BIND digest is zero")
            }
            Self::EmptyDetail => write!(f, "early-render startup failure detail is empty"),
            Self::DetailTooLarge(actual) => write!(
                f,
                "early-render startup failure detail length {actual} exceeds {MAX_EARLY_RENDER_STARTUP_FAILURE_DETAIL_LEN}"
            ),
            Self::DetailLengthMismatch { declared, actual } => write!(
                f,
                "early-render startup failure detail length {declared} does not equal actual length {actual}"
            ),
            Self::InvalidUtf8 => write!(f, "early-render startup failure detail is not UTF-8"),
            Self::DetailDigestMismatch => {
                write!(f, "early-render startup failure detail digest does not recompute")
            }
        }
    }
}

impl std::error::Error for EarlyRenderStartupFailureCodecError {}

pub fn encode_early_render_startup_failure(
    report: &EarlyRenderStartupFailureV1,
) -> Result<Vec<u8>, EarlyRenderStartupFailureCodecError> {
    validate(report)?;
    let detail = report.detail.as_bytes();
    let total_len = EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN
        .checked_add(detail.len())
        .ok_or(EarlyRenderStartupFailureCodecError::PayloadTooLarge(
            usize::MAX,
        ))?;
    if total_len > CONTROL_MAX_PAYLOAD_LEN {
        return Err(EarlyRenderStartupFailureCodecError::PayloadTooLarge(
            total_len,
        ));
    }
    let detail_len = u32::try_from(detail.len()).map_err(|_| {
        EarlyRenderStartupFailureCodecError::DetailTooLarge(detail.len())
    })?;
    let flags = if report.launch_bind_sha256.is_some() {
        FLAG_LAUNCH_BIND_PRESENT
    } else {
        0
    };

    let mut payload = vec![0u8; EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN];
    payload[0..4].copy_from_slice(MAGIC);
    payload[4..6].copy_from_slice(&VERSION.to_le_bytes());
    payload[6..8].copy_from_slice(&(report.phase as u16).to_le_bytes());
    payload[8..12].copy_from_slice(&report.process_id.to_le_bytes());
    payload[12..14].copy_from_slice(&flags.to_le_bytes());
    payload[16..24].copy_from_slice(&report.process_start_nonce.to_le_bytes());
    payload[24..40].copy_from_slice(&report.session_id);
    if let Some(launch_bind_sha256) = report.launch_bind_sha256 {
        payload[40..72].copy_from_slice(&launch_bind_sha256);
    }
    payload[72..104].copy_from_slice(&report.detail_sha256);
    payload[104..108].copy_from_slice(&detail_len.to_le_bytes());
    payload.extend_from_slice(detail);
    debug_assert_eq!(payload.len(), total_len);
    decode_early_render_startup_failure(&payload)?;
    Ok(payload)
}

pub fn decode_early_render_startup_failure(
    payload: &[u8],
) -> Result<EarlyRenderStartupFailureV1, EarlyRenderStartupFailureCodecError> {
    if payload.len() < EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN {
        return Err(EarlyRenderStartupFailureCodecError::TooShort(payload.len()));
    }
    if payload.len() > CONTROL_MAX_PAYLOAD_LEN {
        return Err(EarlyRenderStartupFailureCodecError::PayloadTooLarge(
            payload.len(),
        ));
    }
    if &payload[0..4] != MAGIC {
        return Err(EarlyRenderStartupFailureCodecError::BadMagic);
    }
    let version = read_u16(payload, 4);
    if version != VERSION {
        return Err(EarlyRenderStartupFailureCodecError::UnsupportedVersion(
            version,
        ));
    }
    let phase = EarlyRenderStartupPhase::from_wire(read_u16(payload, 6))?;
    let process_id = read_u32(payload, 8);
    let flags = read_u16(payload, 12);
    if flags & !KNOWN_FLAGS != 0 {
        return Err(EarlyRenderStartupFailureCodecError::UnknownFlags(flags));
    }
    if payload[14..16].iter().any(|byte| *byte != 0)
        || payload[108..112].iter().any(|byte| *byte != 0)
    {
        return Err(EarlyRenderStartupFailureCodecError::ReservedNonZero);
    }
    let process_start_nonce = read_u64(payload, 16);
    let session_id = read_array::<16>(payload, 24);
    let launch_bind_wire = read_array::<32>(payload, 40);
    let launch_bind_sha256 = if flags & FLAG_LAUNCH_BIND_PRESENT != 0 {
        Some(launch_bind_wire)
    } else {
        if launch_bind_wire != [0; 32] {
            return Err(EarlyRenderStartupFailureCodecError::UnexpectedLaunchBind);
        }
        None
    };
    let detail_sha256 = read_array::<32>(payload, 72);
    let detail_len = read_u32(payload, 104) as usize;
    let actual_detail_len = payload.len() - EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN;
    if detail_len != actual_detail_len {
        return Err(
            EarlyRenderStartupFailureCodecError::DetailLengthMismatch {
                declared: detail_len,
                actual: actual_detail_len,
            },
        );
    }
    let detail_bytes = &payload[EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN..];
    let detail = std::str::from_utf8(detail_bytes)
        .map_err(|_| EarlyRenderStartupFailureCodecError::InvalidUtf8)?
        .to_owned();
    let report = EarlyRenderStartupFailureV1 {
        process_id,
        process_start_nonce,
        session_id,
        phase,
        launch_bind_sha256,
        detail_sha256,
        detail,
    };
    validate(&report)?;
    Ok(report)
}

fn validate(
    report: &EarlyRenderStartupFailureV1,
) -> Result<(), EarlyRenderStartupFailureCodecError> {
    if report.process_id == 0 {
        return Err(EarlyRenderStartupFailureCodecError::ZeroProcessId);
    }
    if report.process_start_nonce == 0 {
        return Err(EarlyRenderStartupFailureCodecError::ZeroProcessStartNonce);
    }
    if report.session_id == [0; 16] {
        return Err(EarlyRenderStartupFailureCodecError::ZeroSessionId);
    }
    match (report.phase.requires_launch_bind(), report.launch_bind_sha256) {
        (true, None) => return Err(EarlyRenderStartupFailureCodecError::MissingLaunchBind),
        (false, Some(_)) => {
            return Err(EarlyRenderStartupFailureCodecError::UnexpectedLaunchBind)
        }
        (_, Some(hash)) if hash == [0; 32] => {
            return Err(EarlyRenderStartupFailureCodecError::ZeroLaunchBind)
        }
        _ => {}
    }
    let detail = report.detail.as_bytes();
    if detail.is_empty() {
        return Err(EarlyRenderStartupFailureCodecError::EmptyDetail);
    }
    if detail.len() > MAX_EARLY_RENDER_STARTUP_FAILURE_DETAIL_LEN {
        return Err(EarlyRenderStartupFailureCodecError::DetailTooLarge(
            detail.len(),
        ));
    }
    if Sha256::digest(detail) != report.detail_sha256 {
        return Err(EarlyRenderStartupFailureCodecError::DetailDigestMismatch);
    }
    Ok(())
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

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> [u8; N] {
    bytes[offset..offset + N].try_into().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(phase: EarlyRenderStartupPhase) -> EarlyRenderStartupFailureV1 {
        EarlyRenderStartupFailureV1::new(
            42,
            0x1122_3344_5566_7788,
            [3; 16],
            phase,
            phase.requires_launch_bind().then_some([5; 32]),
            "bind A25 failed: quote=\"x\"\npath=gui/国家.txt",
        )
        .unwrap()
    }

    #[test]
    fn all_phases_round_trip_exact_utf8_detail() {
        for phase in [
            EarlyRenderStartupPhase::PostHelloPreBind,
            EarlyRenderStartupPhase::PostBindPreFirst,
            EarlyRenderStartupPhase::PostFirstPreCommit,
            EarlyRenderStartupPhase::PostCommitBootstrap,
        ] {
            let expected = report(phase);
            let payload = encode_early_render_startup_failure(&expected).unwrap();
            assert_eq!(
                payload.len(),
                EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN + expected.detail.len()
            );
            assert_eq!(
                decode_early_render_startup_failure(&payload).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn identity_phase_binding_and_digest_invariants_fail_closed() {
        let mut value = report(EarlyRenderStartupPhase::PostBindPreFirst);
        value.process_id = 0;
        assert_eq!(
            encode_early_render_startup_failure(&value),
            Err(EarlyRenderStartupFailureCodecError::ZeroProcessId)
        );

        let mut value = report(EarlyRenderStartupPhase::PostBindPreFirst);
        value.process_start_nonce = 0;
        assert_eq!(
            encode_early_render_startup_failure(&value),
            Err(EarlyRenderStartupFailureCodecError::ZeroProcessStartNonce)
        );

        let mut value = report(EarlyRenderStartupPhase::PostBindPreFirst);
        value.session_id = [0; 16];
        assert_eq!(
            encode_early_render_startup_failure(&value),
            Err(EarlyRenderStartupFailureCodecError::ZeroSessionId)
        );

        let mut value = report(EarlyRenderStartupPhase::PostBindPreFirst);
        value.launch_bind_sha256 = None;
        assert_eq!(
            encode_early_render_startup_failure(&value),
            Err(EarlyRenderStartupFailureCodecError::MissingLaunchBind)
        );

        let mut value = report(EarlyRenderStartupPhase::PostHelloPreBind);
        value.launch_bind_sha256 = Some([5; 32]);
        assert_eq!(
            encode_early_render_startup_failure(&value),
            Err(EarlyRenderStartupFailureCodecError::UnexpectedLaunchBind)
        );

        let mut value = report(EarlyRenderStartupPhase::PostBindPreFirst);
        value.detail.push('!');
        assert_eq!(
            encode_early_render_startup_failure(&value),
            Err(EarlyRenderStartupFailureCodecError::DetailDigestMismatch)
        );
    }

    #[test]
    fn malformed_wire_shapes_are_rejected_without_guessing() {
        let base =
            encode_early_render_startup_failure(&report(EarlyRenderStartupPhase::PostBindPreFirst))
                .unwrap();

        assert!(matches!(
            decode_early_render_startup_failure(
                &base[..EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN - 1]
            ),
            Err(EarlyRenderStartupFailureCodecError::TooShort(_))
        ));

        let mut bad_magic = base.clone();
        bad_magic[0] ^= 1;
        assert_eq!(
            decode_early_render_startup_failure(&bad_magic),
            Err(EarlyRenderStartupFailureCodecError::BadMagic)
        );

        let mut bad_version = base.clone();
        bad_version[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            decode_early_render_startup_failure(&bad_version),
            Err(EarlyRenderStartupFailureCodecError::UnsupportedVersion(2))
        );

        let mut bad_phase = base.clone();
        bad_phase[6..8].copy_from_slice(&99u16.to_le_bytes());
        assert_eq!(
            decode_early_render_startup_failure(&bad_phase),
            Err(EarlyRenderStartupFailureCodecError::UnknownPhase(99))
        );

        let mut bad_flags = base.clone();
        bad_flags[12..14].copy_from_slice(&0x8001u16.to_le_bytes());
        assert_eq!(
            decode_early_render_startup_failure(&bad_flags),
            Err(EarlyRenderStartupFailureCodecError::UnknownFlags(0x8001))
        );

        let mut bad_reserved = base.clone();
        bad_reserved[108] = 1;
        assert_eq!(
            decode_early_render_startup_failure(&bad_reserved),
            Err(EarlyRenderStartupFailureCodecError::ReservedNonZero)
        );

        let mut bad_length = base.clone();
        bad_length[104..108].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            decode_early_render_startup_failure(&bad_length),
            Err(EarlyRenderStartupFailureCodecError::DetailLengthMismatch { .. })
        ));

        let mut bad_utf8 = base.clone();
        bad_utf8[EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN] = 0xff;
        assert_eq!(
            decode_early_render_startup_failure(&bad_utf8),
            Err(EarlyRenderStartupFailureCodecError::InvalidUtf8)
        );

        let mut bad_digest = base;
        bad_digest[EARLY_RENDER_STARTUP_FAILURE_FIXED_LEN] ^= 1;
        assert_eq!(
            decode_early_render_startup_failure(&bad_digest),
            Err(EarlyRenderStartupFailureCodecError::DetailDigestMismatch)
        );
    }

    #[test]
    fn empty_and_overlong_details_are_rejected() {
        assert_eq!(
            EarlyRenderStartupFailureV1::new(
                42,
                7,
                [3; 16],
                EarlyRenderStartupPhase::PostHelloPreBind,
                None,
                ""
            ),
            Err(EarlyRenderStartupFailureCodecError::EmptyDetail)
        );
        let detail = "x".repeat(MAX_EARLY_RENDER_STARTUP_FAILURE_DETAIL_LEN + 1);
        assert_eq!(
            EarlyRenderStartupFailureV1::new(
                42,
                7,
                [3; 16],
                EarlyRenderStartupPhase::PostHelloPreBind,
                None,
                detail
            ),
            Err(EarlyRenderStartupFailureCodecError::DetailTooLarge(
                MAX_EARLY_RENDER_STARTUP_FAILURE_DETAIL_LEN + 1
            ))
        );
    }
}
