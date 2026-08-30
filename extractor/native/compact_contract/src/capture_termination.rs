pub const CAPTURE_TERMINATION_MAGIC: &[u8; 4] = b"H4AT";
pub const CAPTURE_TERMINATION_VERSION: u16 = 1;
pub const CAPTURE_TERMINATION_HEADER_LEN: usize = 24;
pub const CAPTURE_TERMINATION_MAX_DETAIL_LEN: usize = 2_048;

crate::cause_tables::bit_flag_causes! {
    repr = u16,
    mask = pub CAPTURE_TERMINATION_REASON_KNOWN_MASK,
    names = pub(crate) CAPTURE_TERMINATION_REASON_NAMES,
    bit_prefix = "CAPTURE_TERMINATION_REASON_";

    CAPTURE_TERMINATION_REASON_CAPTURE_LOOP_ABORTED = 0 => capture_loop_aborted,
    CAPTURE_TERMINATION_REASON_CLOCK_OBSERVER_PANICKED = 1 => clock_observer_panicked,
    CAPTURE_TERMINATION_REASON_CLOCK_OBSERVER_EXITED_EARLY = 2 => clock_observer_exited_early,
}

const CAPTURE_TERMINATION_FLAG_DETAIL_TRUNCATED: u16 = 1 << 0;
const CAPTURE_TERMINATION_FLAG_KNOWN_MASK: u16 = CAPTURE_TERMINATION_FLAG_DETAIL_TRUNCATED;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureTerminationReasons(u16);

impl CaptureTerminationReasons {
    pub fn from_bits(bits: u16) -> Result<Self, String> {
        if bits == 0 {
            return Err("capture termination reason mask is empty".to_string());
        }
        let unknown = bits & !CAPTURE_TERMINATION_REASON_KNOWN_MASK;
        if unknown != 0 {
            return Err(format!(
                "capture termination reason mask contains unknown bits {unknown:#06x}"
            ));
        }
        Ok(Self(bits))
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, reason: u16) -> bool {
        self.0 & reason != 0
    }

    pub fn names(self) -> Vec<&'static str> {
        CAPTURE_TERMINATION_REASON_NAMES
            .iter()
            .filter(|(reason, _)| self.contains(*reason))
            .map(|(_, name)| *name)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureTerminationRecordV1 {
    pub reasons: CaptureTerminationReasons,
    pub elapsed_wall_ms: u64,
    pub detail: String,
    pub detail_truncated: bool,
}

impl CaptureTerminationRecordV1 {
    pub fn encode_bounded(
        reasons: CaptureTerminationReasons,
        elapsed_wall_ms: u64,
        detail: &str,
    ) -> Vec<u8> {
        let original_detail_len = detail.len();
        let mut detail_len = original_detail_len.min(CAPTURE_TERMINATION_MAX_DETAIL_LEN);
        while detail_len > 0 && !detail.is_char_boundary(detail_len) {
            detail_len -= 1;
        }
        let detail = &detail.as_bytes()[..detail_len];
        let mut out = Vec::with_capacity(CAPTURE_TERMINATION_HEADER_LEN + detail_len);
        out.extend_from_slice(CAPTURE_TERMINATION_MAGIC);
        out.extend_from_slice(&CAPTURE_TERMINATION_VERSION.to_le_bytes());
        out.extend_from_slice(&(CAPTURE_TERMINATION_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&reasons.bits().to_le_bytes());
        let flags = if detail_len < original_detail_len {
            CAPTURE_TERMINATION_FLAG_DETAIL_TRUNCATED
        } else {
            0
        };
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&(detail_len as u32).to_le_bytes());
        out.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
        out.extend_from_slice(detail);
        out
    }

    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        if payload.len() < CAPTURE_TERMINATION_HEADER_LEN {
            return Err(format!(
                "capture termination payload has {} byte(s), fewer than header {CAPTURE_TERMINATION_HEADER_LEN}",
                payload.len()
            ));
        }
        if &payload[..4] != CAPTURE_TERMINATION_MAGIC {
            return Err("capture termination payload magic is invalid".to_string());
        }
        let version = u16::from_le_bytes([payload[4], payload[5]]);
        if version != CAPTURE_TERMINATION_VERSION {
            return Err(format!(
                "capture termination payload version {version} is not {CAPTURE_TERMINATION_VERSION}"
            ));
        }
        let header_len = usize::from(u16::from_le_bytes([payload[6], payload[7]]));
        if header_len != CAPTURE_TERMINATION_HEADER_LEN {
            return Err(format!(
                "capture termination header length {header_len} is not {CAPTURE_TERMINATION_HEADER_LEN}"
            ));
        }
        let reasons =
            CaptureTerminationReasons::from_bits(u16::from_le_bytes([payload[8], payload[9]]))?;
        let flags = u16::from_le_bytes([payload[10], payload[11]]);
        let unknown_flags = flags & !CAPTURE_TERMINATION_FLAG_KNOWN_MASK;
        if unknown_flags != 0 {
            return Err(format!(
                "capture termination flags contain unknown bits {unknown_flags:#06x}"
            ));
        }
        let detail_len =
            u32::from_le_bytes([payload[12], payload[13], payload[14], payload[15]]) as usize;
        if detail_len > CAPTURE_TERMINATION_MAX_DETAIL_LEN {
            return Err(format!(
                "capture termination detail length {detail_len} exceeds {CAPTURE_TERMINATION_MAX_DETAIL_LEN}"
            ));
        }
        let expected_len = CAPTURE_TERMINATION_HEADER_LEN
            .checked_add(detail_len)
            .ok_or_else(|| "capture termination payload length overflows".to_string())?;
        if payload.len() != expected_len {
            return Err(format!(
                "capture termination payload length {} is not declared {expected_len}",
                payload.len()
            ));
        }
        let elapsed_wall_ms = u64::from_le_bytes(
            payload[16..24]
                .try_into()
                .expect("validated eight-byte elapsed time"),
        );
        let detail = std::str::from_utf8(&payload[24..])
            .map_err(|_| "capture termination detail is not UTF-8".to_string())?
            .to_string();
        Ok(Self {
            reasons,
            elapsed_wall_ms,
            detail,
            detail_truncated: flags & CAPTURE_TERMINATION_FLAG_DETAIL_TRUNCATED != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_encoder_preserves_typed_reasons_and_never_rejects_long_utf8_detail() {
        let reasons = CaptureTerminationReasons::from_bits(
            CAPTURE_TERMINATION_REASON_CAPTURE_LOOP_ABORTED
                | CAPTURE_TERMINATION_REASON_CLOCK_OBSERVER_PANICKED,
        )
        .unwrap();
        let detail = "failure-\u{1f4a5}".repeat(600);
        let decoded = CaptureTerminationRecordV1::decode(
            &CaptureTerminationRecordV1::encode_bounded(reasons, 1234, &detail),
        )
        .unwrap();
        assert_eq!(decoded.reasons, reasons);
        assert_eq!(decoded.elapsed_wall_ms, 1234);
        assert!(decoded.detail_truncated);
        assert!(decoded.detail.len() <= CAPTURE_TERMINATION_MAX_DETAIL_LEN);
        assert!(detail.starts_with(&decoded.detail));
    }
}
