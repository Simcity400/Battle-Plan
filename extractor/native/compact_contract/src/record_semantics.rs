//! Dependency-free canonical semantics for fully framed H4CR record envelopes.

pub const COMPACT_RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const COMPACT_RECORD_TYPE_FRAME_END: u16 = 2;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;
/// Ordinary control record reporting one acquisition outcome with no data.
pub const COMPACT_RECORD_TYPE_DOMAIN_GAP: u16 = 5;
/// Frame-level record: this hour was assembled and REFUSED, naming the
/// domains that forced it. Not a per-domain outcome -- a refused frame is one
/// event with a list of offenders, which is why it is not a DOMAIN_GAP. Type 4
/// is STREAM_HEADER (campaign_identity.rs), hence 6.
pub const COMPACT_RECORD_TYPE_FRAME_REFUSED: u16 = 6;
pub const COMPACT_DOMAIN_FRAME: u16 = 1000;

pub const COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2: u32 = 1 << 31;
pub const COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER: u32 = 1 << 30;

pub const COMPACT_DOMAIN_PERF_BASE_LEN: usize = 40;
pub const COMPACT_FRAME_END_PAYLOAD_LEN: usize = 32;
pub const COMPACT_DOMAIN_GAP_PAYLOAD_LEN: usize = 72;
pub const COMPACT_DOMAIN_GAP_MAGIC: &[u8; 4] = b"H4DG";
pub const COMPACT_DOMAIN_GAP_VERSION: u16 = 1;
pub const DOMAIN_GAP_REASON_TORN: u16 = 1;
pub const DOMAIN_GAP_REASON_OVERRUN: u16 = 2;
pub const DOMAIN_GAP_REASON_BACKPRESSURE: u16 = 3;
pub const DOMAIN_GAP_REASON_CLOCK_UNREADABLE: u16 = 4;
pub const DOMAIN_GAP_REASON_WORKER_PANIC: u16 = 5;
pub const DOMAIN_GAP_PART_STATE: u16 = 0;
pub const DOMAIN_GAP_PART_EVENT: u16 = 1;
pub const DOMAIN_GAP_PART_BOTH: u16 = 2;
pub const COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC: &[u8; 4] = b"PDU1";
pub const COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION: u16 = 1;

/// Whether one ordinary record is the terminal outcome for a scheduled domain.
///
/// Exactly one PERF or DOMAIN_GAP exhausts each scheduled domain. DATA records
/// are subordinate payloads and never occupy a second outcome slot.
pub const fn compact_record_counts_toward_terminal_outcome(record_type: u16) -> bool {
    matches!(
        record_type,
        COMPACT_RECORD_TYPE_DOMAIN_PERF | COMPACT_RECORD_TYPE_DOMAIN_GAP
    )
}

/// Whether one ordinary record occupies the exact per-domain PERF cardinality.
pub const fn compact_record_counts_toward_perf_cardinality(record_type: u16) -> bool {
    record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF
}

/// Canonical record-header flag mask for one record envelope.
pub const fn compact_record_allowed_flags(domain_id: u16, record_type: u16) -> u32 {
    let mut allowed = 0;
    if record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF {
        allowed |= COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER;
    }
    if record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA && matches!(domain_id, 45 | 67) {
        allowed |= COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2;
    }
    allowed
}

/// Header flag bits that are unknown or invalid for this domain/record type.
pub const fn compact_record_rejected_flags(
    domain_id: u16,
    record_type: u16,
    record_flags: u32,
) -> u32 {
    record_flags & !compact_record_allowed_flags(domain_id, record_type)
}

/// Validate canonical semantics for a fully framed compact record payload.
///
/// The caller owns H4CR framing and must pass the payload slice selected by the
/// header. This validates record flags for every type, the exact DOMAIN_PERF
/// base/PDU1 trailer grammar, and the exact FRAME_END domain/payload contract.
pub fn validate_compact_record_semantics(
    domain_id: u16,
    record_type: u16,
    record_flags: u32,
    payload: &[u8],
) -> Result<(), String> {
    let rejected_flags = compact_record_rejected_flags(domain_id, record_type, record_flags);
    if rejected_flags != 0 {
        return Err(format!(
            "record flags {record_flags:#010x} contain rejected bits {rejected_flags:#010x} for domain {domain_id} record type {record_type}"
        ));
    }

    match record_type {
        COMPACT_RECORD_TYPE_DOMAIN_PERF => validate_domain_perf(record_flags, payload),
        COMPACT_RECORD_TYPE_FRAME_END => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "FRAME_END domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            if payload.len() != COMPACT_FRAME_END_PAYLOAD_LEN {
                return Err(format!(
                    "FRAME_END payload length {} is not {COMPACT_FRAME_END_PAYLOAD_LEN}",
                    payload.len()
                ));
            }
            Ok(())
        }
        COMPACT_RECORD_TYPE_DOMAIN_DATA => Ok(()),
        COMPACT_RECORD_TYPE_FRAME_REFUSED => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "FRAME_REFUSED domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            crate::frame_refusal::validate_frame_refusal_payload(payload)
        }
        COMPACT_RECORD_TYPE_DOMAIN_GAP => {
            if domain_id == 0 || domain_id == COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "DOMAIN_GAP envelope affected domain {domain_id} is invalid"
                ));
            }
            validate_domain_gap_payload(payload)
        }
        other => Err(format!("unknown compact record type {other}")),
    }
}

/// Validate the dependency-free, authenticated H4DG payload grammar.
///
/// This proves the complete payload layout and witness semantics. Authentication
/// belongs to the enclosing retained H4CR sequence/campaign/session digest,
/// which also binds the affected domain carried by the H4CR header.
pub fn validate_domain_gap_payload(payload: &[u8]) -> Result<(), String> {
    if payload.len() != COMPACT_DOMAIN_GAP_PAYLOAD_LEN {
        return Err(format!(
            "DOMAIN_GAP payload length {} is not {COMPACT_DOMAIN_GAP_PAYLOAD_LEN}",
            payload.len()
        ));
    }
    if &payload[..4] != COMPACT_DOMAIN_GAP_MAGIC {
        return Err("DOMAIN_GAP payload magic is invalid".to_string());
    }
    let version = u16::from_le_bytes([payload[4], payload[5]]);
    if version != COMPACT_DOMAIN_GAP_VERSION {
        return Err(format!(
            "DOMAIN_GAP payload version {version} is not {COMPACT_DOMAIN_GAP_VERSION}"
        ));
    }
    let declared_len = usize::from(u16::from_le_bytes([payload[6], payload[7]]));
    if declared_len != COMPACT_DOMAIN_GAP_PAYLOAD_LEN {
        return Err(format!(
            "DOMAIN_GAP declared length {declared_len} is not {COMPACT_DOMAIN_GAP_PAYLOAD_LEN}"
        ));
    }
    let reason = u16::from_le_bytes([payload[8], payload[9]]);
    if !matches!(
        reason,
        DOMAIN_GAP_REASON_TORN
            | DOMAIN_GAP_REASON_OVERRUN
            | DOMAIN_GAP_REASON_BACKPRESSURE
            | DOMAIN_GAP_REASON_CLOCK_UNREADABLE
            | DOMAIN_GAP_REASON_WORKER_PANIC
    ) {
        return Err(format!("DOMAIN_GAP reason {reason} is unknown"));
    }
    let part = u16::from_le_bytes([payload[10], payload[11]]);
    if !matches!(
        part,
        DOMAIN_GAP_PART_STATE | DOMAIN_GAP_PART_EVENT | DOMAIN_GAP_PART_BOTH
    ) {
        return Err(format!("DOMAIN_GAP part {part} is unknown"));
    }
    if u32::from_le_bytes(payload[12..16].try_into().expect("fixed reserved slice")) != 0
        || u32::from_le_bytes(payload[28..32].try_into().expect("fixed reserved slice")) != 0
    {
        return Err("DOMAIN_GAP reserved field is nonzero".to_string());
    }
    let target_hour = i32::from_le_bytes(payload[16..20].try_into().expect("fixed target slice"));
    let range_start =
        i32::from_le_bytes(payload[20..24].try_into().expect("fixed range-start slice"));
    let range_end = i32::from_le_bytes(payload[24..28].try_into().expect("fixed range-end slice"));
    for (name, hour) in [
        ("target", target_hour),
        ("range_start", range_start),
        ("range_end", range_end),
    ] {
        if crate::calendar::hour_index_from_raw(hour).is_none() {
            return Err(format!("DOMAIN_GAP {name} hour {hour} is invalid"));
        }
    }
    if range_start > range_end || target_hour < range_start || target_hour > range_end {
        return Err(format!(
            "DOMAIN_GAP target/range witness {target_hour} not within {range_start}..={range_end}"
        ));
    }
    let inclusive_range_width = u64::try_from(
        i64::from(range_end)
            .checked_sub(i64::from(range_start))
            .and_then(|width| width.checked_add(1))
            .ok_or_else(|| "DOMAIN_GAP inclusive range width overflows".to_string())?,
    )
    .map_err(|_| "DOMAIN_GAP inclusive range width is negative".to_string())?;
    let tick_before =
        u64::from_le_bytes(payload[32..40].try_into().expect("fixed tick-before slice"));
    let tick_after =
        u64::from_le_bytes(payload[40..48].try_into().expect("fixed tick-after slice"));
    let queue_capacity = u32::from_le_bytes(
        payload[48..52]
            .try_into()
            .expect("fixed queue-capacity slice"),
    );
    let queue_depth =
        u32::from_le_bytes(payload[52..56].try_into().expect("fixed queue-depth slice"));
    let queue_dropped = u64::from_le_bytes(
        payload[56..64]
            .try_into()
            .expect("fixed queue-dropped slice"),
    );
    let outcome_ordinal = u64::from_le_bytes(
        payload[64..72]
            .try_into()
            .expect("fixed outcome-ordinal slice"),
    );
    if outcome_ordinal == u64::MAX {
        return Err("DOMAIN_GAP outcome ordinal is exhausted".to_string());
    }
    match reason {
        DOMAIN_GAP_REASON_TORN
            if range_start == range_end
                && target_hour == range_start
                && tick_before != tick_after
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        DOMAIN_GAP_REASON_OVERRUN
            if target_hour == range_end
                && range_start < range_end
                && tick_after.checked_sub(tick_before) == Some(inclusive_range_width)
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        DOMAIN_GAP_REASON_BACKPRESSURE
            if target_hour == range_end
                && tick_before == tick_after
                && queue_capacity > 0
                && queue_depth == queue_capacity
                && queue_dropped == inclusive_range_width => {}
        DOMAIN_GAP_REASON_CLOCK_UNREADABLE
            if range_start == range_end
                && target_hour == range_start
                && tick_before == 0
                && tick_after == 0
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        DOMAIN_GAP_REASON_WORKER_PANIC
            if range_start == range_end
                && target_hour == range_start
                && tick_before == tick_after
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        _ => {
            return Err(format!(
                "DOMAIN_GAP witnesses do not prove closed reason {reason}"
            ))
        }
    }
    Ok(())
}

/// Read the target-hour witness after validating the complete H4DG grammar.
pub fn domain_gap_target_hour_raw(payload: &[u8]) -> Result<i32, String> {
    validate_domain_gap_payload(payload)?;
    Ok(i32::from_le_bytes(
        payload[16..20]
            .try_into()
            .expect("validated DOMAIN_GAP target slice"),
    ))
}

fn validate_domain_perf(record_flags: u32, payload: &[u8]) -> Result<(), String> {
    if payload.len() < COMPACT_DOMAIN_PERF_BASE_LEN {
        return Err(format!(
            "DOMAIN_PERF payload length {} is shorter than {COMPACT_DOMAIN_PERF_BASE_LEN}",
            payload.len()
        ));
    }
    if u32::from_le_bytes(payload[36..40].try_into().expect("fixed PERF base slice")) != 0 {
        return Err("DOMAIN_PERF reserved base word is nonzero".to_string());
    }

    let has_trailer = record_flags & COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER != 0;
    if !has_trailer {
        return if payload.len() == COMPACT_DOMAIN_PERF_BASE_LEN {
            Ok(())
        } else {
            Err(format!(
                "DOMAIN_PERF payload has {} unannounced trailer byte(s)",
                payload.len() - COMPACT_DOMAIN_PERF_BASE_LEN
            ))
        };
    }

    const TRAILER_HEADER_LEN: usize = 8;
    const UNIT_HEADER_LEN: usize = 12;
    let trailer_start = COMPACT_DOMAIN_PERF_BASE_LEN;
    if payload.len() < trailer_start + TRAILER_HEADER_LEN {
        return Err("DOMAIN_PERF diagnostic trailer header is truncated".to_string());
    }
    if &payload[trailer_start..trailer_start + 4] != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC {
        return Err("DOMAIN_PERF diagnostic trailer magic is invalid".to_string());
    }
    let version = u16::from_le_bytes(
        payload[trailer_start + 4..trailer_start + 6]
            .try_into()
            .expect("fixed PERF trailer version slice"),
    );
    if version != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION {
        return Err(format!(
            "DOMAIN_PERF diagnostic trailer version {version} is not {COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION}"
        ));
    }
    let unit_count = u16::from_le_bytes(
        payload[trailer_start + 6..trailer_start + 8]
            .try_into()
            .expect("fixed PERF trailer count slice"),
    ) as usize;
    let mut cursor = trailer_start + TRAILER_HEADER_LEN;
    for unit_index in 0..unit_count {
        if payload.len().saturating_sub(cursor) < UNIT_HEADER_LEN {
            return Err(format!(
                "DOMAIN_PERF diagnostic unit {unit_index} header is truncated"
            ));
        }
        let name_len = u16::from_le_bytes(
            payload[cursor..cursor + 2]
                .try_into()
                .expect("fixed PERF unit name-length slice"),
        ) as usize;
        let reserved = u16::from_le_bytes(
            payload[cursor + 2..cursor + 4]
                .try_into()
                .expect("fixed PERF unit reserved slice"),
        );
        if reserved != 0 {
            return Err(format!(
                "DOMAIN_PERF diagnostic unit {unit_index} reserved field is nonzero"
            ));
        }
        cursor += UNIT_HEADER_LEN;
        if payload.len().saturating_sub(cursor) < name_len {
            return Err(format!(
                "DOMAIN_PERF diagnostic unit {unit_index} name is truncated"
            ));
        }
        let name = std::str::from_utf8(&payload[cursor..cursor + name_len])
            .map_err(|_| format!("DOMAIN_PERF diagnostic unit {unit_index} name is not UTF-8"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(format!(
                "DOMAIN_PERF diagnostic unit {unit_index} name {name:?} is invalid"
            ));
        }
        cursor += name_len;
    }
    if cursor != payload.len() {
        return Err(format!(
            "DOMAIN_PERF diagnostic trailer has {} trailing byte(s)",
            payload.len() - cursor
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::DATE_EPOCH_CONST;

    fn base_perf() -> Vec<u8> {
        vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN]
    }

    fn perf_with_trailer(name: Option<&[u8]>, reserved: u16) -> Vec<u8> {
        let mut payload = base_perf();
        payload.extend_from_slice(COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
        payload.extend_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes());
        payload.extend_from_slice(&(name.is_some() as u16).to_le_bytes());
        if let Some(name) = name {
            payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
            payload.extend_from_slice(&reserved.to_le_bytes());
            payload.extend_from_slice(&7u64.to_le_bytes());
            payload.extend_from_slice(name);
        }
        payload
    }

    fn perf_result(flags: u32, payload: &[u8]) -> Result<(), String> {
        validate_compact_record_semantics(18, COMPACT_RECORD_TYPE_DOMAIN_PERF, flags, payload)
    }

    fn gap_payload(reason: u16) -> Vec<u8> {
        let mut payload = vec![0u8; COMPACT_DOMAIN_GAP_PAYLOAD_LEN];
        payload[..4].copy_from_slice(COMPACT_DOMAIN_GAP_MAGIC);
        payload[4..6].copy_from_slice(&COMPACT_DOMAIN_GAP_VERSION.to_le_bytes());
        payload[6..8].copy_from_slice(&(COMPACT_DOMAIN_GAP_PAYLOAD_LEN as u16).to_le_bytes());
        payload[8..10].copy_from_slice(&reason.to_le_bytes());
        let hour = DATE_EPOCH_CONST + 1936 * 365 * 24;
        payload[16..20].copy_from_slice(&hour.to_le_bytes());
        payload[20..24].copy_from_slice(&hour.to_le_bytes());
        payload[24..28].copy_from_slice(&hour.to_le_bytes());
        payload[32..40].copy_from_slice(&10u64.to_le_bytes());
        payload[40..48].copy_from_slice(&11u64.to_le_bytes());
        payload
    }

    #[test]
    fn record_flag_policy_is_domain_and_type_specific() {
        assert_eq!(
            compact_record_allowed_flags(45, COMPACT_RECORD_TYPE_DOMAIN_DATA),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
        );
        assert_eq!(
            compact_record_allowed_flags(67, COMPACT_RECORD_TYPE_DOMAIN_DATA),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
        );
        assert_eq!(
            compact_record_allowed_flags(18, COMPACT_RECORD_TYPE_DOMAIN_PERF),
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER
        );
        assert_eq!(
            compact_record_allowed_flags(18, COMPACT_RECORD_TYPE_DOMAIN_DATA),
            0
        );
        assert_eq!(
            compact_record_rejected_flags(
                67,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            ),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
        );
        assert_ne!(
            compact_record_rejected_flags(
                18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            ),
            0
        );
    }

    #[test]
    fn domain_gap_is_terminal_outcome_but_not_perf_or_data_cardinality() {
        assert!(compact_record_counts_toward_terminal_outcome(
            COMPACT_RECORD_TYPE_DOMAIN_GAP
        ));
        assert!(!compact_record_counts_toward_perf_cardinality(
            COMPACT_RECORD_TYPE_DOMAIN_GAP
        ));
        assert!(compact_record_counts_toward_terminal_outcome(
            COMPACT_RECORD_TYPE_DOMAIN_PERF
        ));
        assert!(compact_record_counts_toward_perf_cardinality(
            COMPACT_RECORD_TYPE_DOMAIN_PERF
        ));
        assert!(!compact_record_counts_toward_terminal_outcome(
            COMPACT_RECORD_TYPE_DOMAIN_DATA
        ));
    }

    #[test]
    fn domain_gap_header_owns_affected_domain_and_witnesses_are_closed() {
        let torn = gap_payload(DOMAIN_GAP_REASON_TORN);
        validate_compact_record_semantics(81, COMPACT_RECORD_TYPE_DOMAIN_GAP, 0, &torn).unwrap();
        for invalid_domain in [0, COMPACT_DOMAIN_FRAME] {
            assert!(validate_compact_record_semantics(
                invalid_domain,
                COMPACT_RECORD_TYPE_DOMAIN_GAP,
                0,
                &torn
            )
            .is_err());
        }
        assert!(
            validate_compact_record_semantics(81, COMPACT_RECORD_TYPE_DOMAIN_GAP, 1, &torn)
                .is_err()
        );

        let mut queue_on_torn = torn.clone();
        queue_on_torn[48..52].copy_from_slice(&1u32.to_le_bytes());
        assert!(validate_compact_record_semantics(
            81,
            COMPACT_RECORD_TYPE_DOMAIN_GAP,
            0,
            &queue_on_torn
        )
        .is_err());

        let mut backpressure = gap_payload(DOMAIN_GAP_REASON_BACKPRESSURE);
        backpressure[40..48].copy_from_slice(&10u64.to_le_bytes());
        backpressure[48..52].copy_from_slice(&8u32.to_le_bytes());
        backpressure[52..56].copy_from_slice(&8u32.to_le_bytes());
        backpressure[56..64].copy_from_slice(&1u64.to_le_bytes());
        validate_compact_record_semantics(81, COMPACT_RECORD_TYPE_DOMAIN_GAP, 0, &backpressure)
            .unwrap();
    }

    #[test]
    fn accepts_exact_base_perf_without_trailer() {
        perf_result(0, &base_perf()).expect("exact base PERF must be valid");
    }

    #[test]
    fn announced_perf_trailer_must_be_present() {
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &base_perf())
            .expect_err("bit 30 without a trailer must fail");
        assert!(err.contains("trailer header is truncated"), "{err}");
    }

    #[test]
    fn perf_trailer_must_be_announced() {
        let payload = perf_with_trailer(None, 0);
        let err = perf_result(0, &payload).expect_err("unannounced trailer must fail");
        assert!(err.contains("unannounced trailer"), "{err}");
    }

    #[test]
    fn accepts_empty_and_nonempty_announced_pdu1_trailers() {
        for payload in [
            perf_with_trailer(None, 0),
            perf_with_trailer(Some(b"read_us"), 0),
        ] {
            perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &payload)
                .expect("canonical PDU1 trailer must be valid");
        }
    }

    #[test]
    fn rejects_bad_pdu1_magic_and_version() {
        let mut bad_magic = perf_with_trailer(None, 0);
        bad_magic[COMPACT_DOMAIN_PERF_BASE_LEN] = b'X';
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &bad_magic)
            .expect_err("bad PDU1 magic must fail");
        assert!(err.contains("magic is invalid"), "{err}");

        let mut bad_version = perf_with_trailer(None, 0);
        bad_version[COMPACT_DOMAIN_PERF_BASE_LEN + 4..COMPACT_DOMAIN_PERF_BASE_LEN + 6]
            .copy_from_slice(&2u16.to_le_bytes());
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &bad_version)
            .expect_err("bad PDU1 version must fail");
        assert!(err.contains("version 2 is not 1"), "{err}");
    }

    #[test]
    fn rejects_reserved_perf_words() {
        let mut bad_base = base_perf();
        bad_base[36..40].copy_from_slice(&1u32.to_le_bytes());
        let err = perf_result(0, &bad_base).expect_err("nonzero base reserved word must fail");
        assert!(err.contains("reserved base word"), "{err}");

        let bad_unit = perf_with_trailer(Some(b"read_us"), 1);
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &bad_unit)
            .expect_err("nonzero unit reserved word must fail");
        assert!(err.contains("reserved field"), "{err}");
    }

    #[test]
    fn rejects_invalid_perf_unit_names() {
        for name in [b"ReadUs".as_slice(), b"".as_slice(), &[0xff][..]] {
            let payload = perf_with_trailer(Some(name), 0);
            assert!(
                perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &payload).is_err(),
                "invalid diagnostic name {name:?} was accepted"
            );
        }
    }

    #[test]
    fn rejects_trailing_pdu1_bytes() {
        let mut payload = perf_with_trailer(Some(b"read_us"), 0);
        payload.push(0);
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &payload)
            .expect_err("trailing PDU1 bytes must fail");
        assert!(err.contains("trailing byte"), "{err}");
    }

    #[test]
    fn frame_end_domain_payload_and_flags_are_exact() {
        validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        )
        .expect("canonical FRAME_END must be valid");

        let wrong_domain = validate_compact_record_semantics(
            41,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        )
        .expect_err("wrong FRAME_END domain must fail");
        assert!(wrong_domain.contains("FRAME_END domain"), "{wrong_domain}");

        let wrong_len = validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; COMPACT_FRAME_END_PAYLOAD_LEN - 1],
        )
        .expect_err("wrong FRAME_END length must fail");
        assert!(wrong_len.contains("payload length"), "{wrong_len}");

        let wrong_flags = validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &[0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        )
        .expect_err("FRAME_END flags must fail");
        assert!(wrong_flags.contains("rejected bits"), "{wrong_flags}");
    }

    #[test]
    fn explicitly_accepts_data_and_rejects_unknown_record_types() {
        validate_compact_record_semantics(18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &[])
            .expect("DOMAIN_DATA envelope semantics must be accepted");
        let misplaced = validate_compact_record_semantics(
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &[],
        )
        .expect_err("misplaced PERF flag on DOMAIN_DATA must fail");
        assert!(misplaced.contains("rejected bits"), "{misplaced}");
        let err = validate_compact_record_semantics(18, 99, 0, &[])
            .expect_err("unknown record type must fail");
        assert!(err.contains("unknown compact record type 99"), "{err}");
    }
}
