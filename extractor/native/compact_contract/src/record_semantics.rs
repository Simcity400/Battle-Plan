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
/// Frame-level record: this hour WAS published, and these named domains are
/// UNKNOWN in it. The counterpart to `FRAME_REFUSED` -- that one closes an hour
/// which published nothing, this one annotates an hour which published while
/// some of its domains failed. It is deliberately NOT a boundary record: it
/// rides inside the frame it describes, before that frame's `FRAME_END`, so a
/// consumer reads the degradation as part of the same closed hour.
pub const COMPACT_RECORD_TYPE_FRAME_DEGRADED: u16 = 7;
/// Frame-independent producer-abort notice. Unlike FRAME_REFUSED and
/// FRAME_DEGRADED, this closes or qualifies no game hour and claims no domain
/// coverage. Its H4CR envelope uses domain 1000 and game-hour sentinel zero.
pub const COMPACT_RECORD_TYPE_CAPTURE_TERMINATED: u16 = 8;
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
/// The reader ran to completion inside its clock bracket and could not produce
/// a payload: a root, header, or object invariant it depends on was not
/// available this hour. The clock was read fine, which is exactly why this is
/// not `CLOCK_UNREADABLE`; the per-domain PERF diagnostics carry the typed
/// stage and reason.
pub const DOMAIN_GAP_REASON_SOURCE_UNAVAILABLE: u16 = 6;
/// The domain was scheduled and the frame sealed without it emitting either a
/// DATA record or a gap naming why there is none. This reason asserts only what
/// is actually known -- that no outcome was recorded -- and exists so that such
/// an hour is a NAMED loss rather than an absence no ledger can see.
pub const DOMAIN_GAP_REASON_READER_SILENT: u16 = 7;
/// The reader's clock bracket neither tore nor observed the frame's own hour:
/// it opened and closed in some OTHER hour entirely.
///
/// This is the equal-but-wrong case, and it is invisible to `TORN`. A bracket
/// proves the hour did not CHANGE across the read; it does not prove the read
/// happened in the hour the frame is labelled with. A sweep that begins after
/// the label's hour has already elapsed reads hour H+1 from end to end, so
/// `tick_before == tick_after` and every tear check passes -- while the frame
/// still publishes under label H.
///
/// The witnesses make the claim self-proving: `tick_before == tick_after`
/// (nothing tore) and both differ from `target_hour_raw` (and so name the hour
/// actually observed). A consumer never has to take the reason's word for it.
pub const DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH: u16 = 8;
pub const DOMAIN_GAP_PART_STATE: u16 = 0;
pub const DOMAIN_GAP_PART_EVENT: u16 = 1;
pub const DOMAIN_GAP_PART_BOTH: u16 = 2;
pub const COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC: &[u8; 4] = b"PDU1";
pub const COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION: u16 = 1;
/// Trailer version written by the current producer.
///
/// Identical layout to version 1 with ONE change: the two bytes that version 1
/// reserved and required to be zero now carry the unit's
/// [`crate::diagnostic_channel::UnitRole`] and
/// [`crate::diagnostic_channel::UnitPresence`], both non-zero. Version 1 stays
/// legal on the wire and still requires those bytes to be zero, so retained
/// telemetry from earlier recorder builds is unaffected and a version-1 unit
/// lifts to `Undeclared`/`Undeclared` rather than to a fabricated declaration.
pub const COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2: u16 = 2;

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
            // Both refusal versions are legal on the wire: version 2 is what
            // the current producer writes, version 1 is what retained
            // telemetry from earlier recorder builds holds. Validating only
            // one would report every record of the other version as corrupt.
            crate::frame_refusal::validate_frame_refusal_payload_any_version(payload)
        }
        COMPACT_RECORD_TYPE_FRAME_DEGRADED => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "FRAME_DEGRADED domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            crate::frame_degraded::validate_frame_degraded_payload(payload)
        }
        COMPACT_RECORD_TYPE_CAPTURE_TERMINATED => {
            if domain_id != COMPACT_DOMAIN_FRAME {
                return Err(format!(
                    "CAPTURE_TERMINATED domain {domain_id} is not {COMPACT_DOMAIN_FRAME}"
                ));
            }
            crate::capture_termination::CaptureTerminationRecordV1::decode(payload).map(|_| ())
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
            | DOMAIN_GAP_REASON_SOURCE_UNAVAILABLE
            | DOMAIN_GAP_REASON_READER_SILENT
            | DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH
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
        // The reader held a bracket, so the two tick witnesses are a real,
        // non-advancing clock reading for the hour it failed in. One hour, no
        // queue: nothing was dropped, the source simply was not there.
        DOMAIN_GAP_REASON_SOURCE_UNAVAILABLE
            if range_start == range_end
                && target_hour == range_start
                && tick_before == tick_after
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        // No per-domain witness of any kind exists -- that absence IS the
        // claim -- so both ticks are required to be zero rather than carrying a
        // borrowed reading that would assert more than is known.
        DOMAIN_GAP_REASON_READER_SILENT
            if range_start == range_end
                && target_hour == range_start
                && tick_before == 0
                && tick_after == 0
                && queue_capacity == 0
                && queue_depth == 0
                && queue_dropped == 0 => {}
        // SELF-PROVING. The other clock reasons are believed on the producer's
        // word -- `TORN` says the ticks differ and the grammar checks only
        // that. This one is checkable end to end from the payload alone:
        //
        //   tick_before == tick_after      nothing tore, so the read is not a
        //                                  straddle wearing a different name;
        //   tick_before != target_hour     and the hour it DID observe is not
        //                                  the hour the frame published under.
        //
        // Both conjuncts are required. Drop the first and a torn bracket
        // encodes as a mismatch; drop the second and any clean bracket does,
        // which would let this reason swallow the whole capture.
        //
        // A zero tick is rejected with the rest: zero is the wire's "no
        // witness" value (see `READER_SILENT` above), and a reason whose entire
        // content is "the observed hour was X" cannot be built on an absent X.
        DOMAIN_GAP_REASON_HOUR_LABEL_MISMATCH
            if range_start == range_end
                && target_hour == range_start
                && tick_before == tick_after
                && tick_before != 0
                && u64::try_from(target_hour).map(|label| label != tick_before) == Ok(true)
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
    if version != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION
        && version != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2
    {
        return Err(format!(
            "DOMAIN_PERF diagnostic trailer version {version} is neither \
             {COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION} nor \
             {COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2}"
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
        if version == COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION {
            if reserved != 0 {
                return Err(format!(
                    "DOMAIN_PERF diagnostic unit {unit_index} reserved field is nonzero"
                ));
            }
        } else {
            // Version 2 spends those two bytes on the unit's declaration. Both
            // halves must be DEFINED codes: a zero here would be a unit that
            // says nothing about itself while claiming to be self-describing,
            // which is the shape the version bump exists to eliminate.
            let role = (reserved & 0x00FF) as u8;
            let presence = (reserved >> 8) as u8;
            if crate::diagnostic_channel::UnitRole::from_code(role).is_none() {
                return Err(format!(
                    "DOMAIN_PERF diagnostic unit {unit_index} has undefined role code {role}"
                ));
            }
            if crate::diagnostic_channel::UnitPresence::from_code(presence).is_none() {
                return Err(format!(
                    "DOMAIN_PERF diagnostic unit {unit_index} has undefined presence code \
                     {presence}"
                ));
            }
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

    /// This test used to assert that trailer version 2 is refused. Version 2
    /// is now a DEFINED version -- same layout, with the two bytes version 1
    /// reserved carrying the unit's role and presence -- so that assertion
    /// was asserting the absence of a contract that now exists. The test keeps
    /// its whole job: an undefined version is still refused loudly, and a
    /// silently-skipped unknown version is still impossible. What changed is
    /// which number is undefined.
    #[test]
    fn rejects_bad_pdu1_magic_and_undefined_version() {
        let mut bad_magic = perf_with_trailer(None, 0);
        bad_magic[COMPACT_DOMAIN_PERF_BASE_LEN] = b'X';
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &bad_magic)
            .expect_err("bad PDU magic must fail");
        assert!(err.contains("magic is invalid"), "{err}");

        for version in [0u16, 3, 9, u16::MAX] {
            let mut bad_version = perf_with_trailer(None, 0);
            bad_version[COMPACT_DOMAIN_PERF_BASE_LEN + 4..COMPACT_DOMAIN_PERF_BASE_LEN + 6]
                .copy_from_slice(&version.to_le_bytes());
            let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &bad_version)
                .expect_err("an undefined trailer version must fail");
            assert!(
                err.contains(&format!("version {version} is neither")),
                "{err}"
            );
        }

        // Both DEFINED versions are accepted through the same dispatch.
        for version in [
            COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION,
            COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2,
        ] {
            let mut good = perf_with_trailer(None, 0);
            good[COMPACT_DOMAIN_PERF_BASE_LEN + 4..COMPACT_DOMAIN_PERF_BASE_LEN + 6]
                .copy_from_slice(&version.to_le_bytes());
            perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &good)
                .unwrap_or_else(|error| panic!("version {version} must validate: {error}"));
        }
    }

    /// Version 2 spends version 1's reserved word on the unit's declaration,
    /// and both halves must be DEFINED codes. A unit that claims to be
    /// self-describing while describing nothing is the exact shape the version
    /// bump exists to remove, so it fails closed.
    #[test]
    fn a_version_two_unit_must_declare_a_defined_role_and_presence() {
        use crate::diagnostic_channel::{UnitPresence, UnitRole};
        let declaration =
            |role: u8, presence: u8| -> u16 { u16::from(role) | (u16::from(presence) << 8) };
        let mut good = perf_with_trailer(
            Some(b"d44_country_supply_failure_stage"),
            declaration(UnitRole::Deciding.code(), UnitPresence::Observed.code()),
        );
        good[COMPACT_DOMAIN_PERF_BASE_LEN + 4..COMPACT_DOMAIN_PERF_BASE_LEN + 6]
            .copy_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2.to_le_bytes());
        perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &good)
            .expect("a fully declared version 2 unit must validate");

        for (role, presence, expected) in [
            (0u8, UnitPresence::Observed.code(), "undefined role code 0"),
            (99, UnitPresence::Observed.code(), "undefined role code 99"),
            (UnitRole::Deciding.code(), 0, "undefined presence code 0"),
            (UnitRole::Deciding.code(), 99, "undefined presence code 99"),
        ] {
            let mut bad = perf_with_trailer(Some(b"x"), declaration(role, presence));
            bad[COMPACT_DOMAIN_PERF_BASE_LEN + 4..COMPACT_DOMAIN_PERF_BASE_LEN + 6]
                .copy_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2.to_le_bytes());
            let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &bad)
                .expect_err("an undeclared version 2 unit must fail");
            assert!(err.contains(expected), "{err}");
        }

        // And version 1 still requires those bytes to be zero, so retained
        // telemetry is validated exactly as it was.
        let mut legacy = perf_with_trailer(Some(b"x"), 1);
        legacy[COMPACT_DOMAIN_PERF_BASE_LEN + 4..COMPACT_DOMAIN_PERF_BASE_LEN + 6]
            .copy_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes());
        let err = perf_result(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &legacy)
            .expect_err("version 1 reserved bytes must stay zero");
        assert!(err.contains("reserved field is nonzero"), "{err}");
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
