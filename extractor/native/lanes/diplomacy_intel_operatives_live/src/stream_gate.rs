use crate::{compact, DOMAIN_ID};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, parse_compact_record_header,
    validate_compact_record_semantics, COMPACT_DOMAIN_FRAME, COMPACT_DOMAIN_PERF_BASE_LEN,
    COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Authority {
    rows: u64,
    changed_rows: u64,
    units: u64,
    changed_units: u64,
    status: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain53StreamGateSummary {
    pub record_count: u64,
    pub domain_perf_record_count: u64,
    pub domain_data_record_count: u64,
    pub frame_end_record_count: u64,
    pub max_record_len: u64,
    pub closed_game_hours: Vec<i32>,
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub parse_errors: Vec<String>,
}

#[derive(Default)]
struct OwnedFrameAuthority {
    perf_count: u64,
    data_count: u64,
    perf: Option<Authority>,
    data: Option<Authority>,
}

#[derive(Default)]
struct FrameRecordCounts {
    perf: u64,
    data: u64,
}

/// Fail-closed certification for a complete d53 collector stream.
pub fn verify_domain53_stream(bytes: &[u8]) -> Domain53StreamGateSummary {
    let mut out = Domain53StreamGateSummary::default();
    if let Err(error) = compact_contract::stream_validator::validate_freeze_census_stream_placement(bytes) {
        out.parse_errors.push(format!("stateful stream placement: {error}"));
    }
    let mut cursor = 0usize;
    let mut previous_sequence = None;
    let mut pending_hour = None;
    let mut open_hour = None;
    let mut last_closed_hour = None;
    let mut final_record_is_clean_frame_end = false;
    let mut release_epilogue_seen = false;
    let mut owned = OwnedFrameAuthority::default();
    let mut frame_records = FrameRecordCounts::default();

    while cursor < bytes.len() {
        let offset = cursor;
        let header = match parse_compact_record_header(&bytes[cursor..]) {
            Ok(header) => header,
            Err(error) => {
                out.parse_errors.push(format!("offset {offset}: {error}"));
                break;
            }
        };
        validate_sequence(
            &mut out.parse_errors,
            previous_sequence,
            header.sequence,
            offset,
        );
        previous_sequence = Some(header.sequence);
        let previous_record_closed_world = final_record_is_clean_frame_end;
        final_record_is_clean_frame_end = false;
        let Some(end) = cursor.checked_add(header.record_len()) else {
            out.parse_errors
                .push(format!("offset {offset}: compact record length overflows"));
            break;
        };
        if end > bytes.len() {
            out.parse_errors.push(format!(
                "offset {offset}: compact record payload is truncated"
            ));
            break;
        }
        let payload = &bytes[cursor + COMPACT_RECORD_HEADER_LEN..end];
        out.record_count = out.record_count.saturating_add(1);
        out.max_record_len = out.max_record_len.max(header.record_len() as u64);
        let semantics_ok = match validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        ) {
            Ok(()) => true,
            Err(error) => {
                out.parse_errors.push(format!("offset {offset}: {error}"));
                false
            }
        };
        // The single release epilogue the owner ruling admits after a closed-hour
        // boundary. It uses the SHARED policy so this gate cannot drift into a
        // laxer rule than the stateful validator run above.
        if semantics_ok && header.record_type == compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS {
            if let Ok(census) =
                compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload)
            {
                final_record_is_clean_frame_end =
                    compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(
                        previous_record_closed_world,
                        census.phase(),
                        release_epilogue_seen,
                    );
                release_epilogue_seen |= census.phase()
                    == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
            }
        }

        match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF if semantics_ok => {
                frame_records.perf = frame_records.perf.saturating_add(1);
                observe_perf_hour(
                    &mut out.parse_errors,
                    &mut pending_hour,
                    open_hour,
                    last_closed_hour,
                    header.game_hour_raw,
                    offset,
                );
                if header.domain_id == DOMAIN_ID {
                    out.domain_perf_record_count = out.domain_perf_record_count.saturating_add(1);
                    owned.perf_count = owned.perf_count.saturating_add(1);
                    let authority = decode_perf_authority(payload);
                    out.status_flags_or |= authority.status;
                    if owned.perf.is_none() {
                        owned.perf = Some(authority);
                    }
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA if semantics_ok => {
                frame_records.data = frame_records.data.saturating_add(1);
                observe_data_hour(
                    &mut out.parse_errors,
                    pending_hour,
                    &mut open_hour,
                    last_closed_hour,
                    header.game_hour_raw,
                    offset,
                );
                if header.domain_id == DOMAIN_ID {
                    out.domain_data_record_count = out.domain_data_record_count.saturating_add(1);
                    owned.data_count = owned.data_count.saturating_add(1);
                    let decoded =
                        compact::decode_payload(offset + COMPACT_RECORD_HEADER_LEN, payload);
                    let authority = Authority {
                        rows: decoded.data_rows_total,
                        changed_rows: decoded.data_changed_rows_total,
                        // Production d53 PERF names this `unit_count`, but the
                        // collector sets it to emitted_rows. The DATA unit map
                        // contains both a total and per-family breakdown, so
                        // summing that map would double count.
                        units: decoded.data_rows_total,
                        changed_units: decoded.data_changed_rows_total,
                        status: decoded.status_flags_or,
                    };
                    out.status_flags_or |= authority.status;
                    out.data_rows_total = out.data_rows_total.saturating_add(authority.rows);
                    out.data_changed_rows_total = out
                        .data_changed_rows_total
                        .saturating_add(authority.changed_rows);
                    out.parse_errors.extend(decoded.parse_errors);
                    if owned.data.is_none() {
                        owned.data = Some(authority);
                    }
                }
            }
            COMPACT_RECORD_TYPE_FRAME_END => {
                let mut clean = validate_frame_end(
                    &mut out.parse_errors,
                    semantics_ok,
                    open_hour,
                    last_closed_hour,
                    header.game_hour_raw,
                    offset,
                );
                if semantics_ok
                    && !validate_frame_end_payload(
                        &mut out.parse_errors,
                        payload,
                        offset,
                        &frame_records,
                    )
                {
                    clean = false;
                }
                if semantics_ok {
                    validate_owned_authority(
                        &mut out.parse_errors,
                        header.game_hour_raw,
                        offset,
                        &owned,
                    );
                }
                if clean {
                    out.frame_end_record_count = out.frame_end_record_count.saturating_add(1);
                    out.closed_game_hours.push(header.game_hour_raw);
                    open_hour = None;
                    pending_hour = None;
                    last_closed_hour = Some(header.game_hour_raw);
                    final_record_is_clean_frame_end = true;
                    owned = OwnedFrameAuthority::default();
                    frame_records = FrameRecordCounts::default();
                }
            }
            _ => {}
        }
        cursor = end;
    }

    if let Some(hour) = pending_hour.filter(|_| open_hour.is_none()) {
        out.parse_errors.push(format!(
            "stream ended with pending PERF frame hour {hour} without DATA/FRAME_END"
        ));
    }
    if let Some(hour) = open_hour {
        out.parse_errors
            .push(format!("stream ended with open DATA frame hour {hour}"));
    }
    if !final_record_is_clean_frame_end {
        out.parse_errors
            .push("stream is not closed by a final canonical FRAME_END record".to_string());
    }
    if out.frame_end_record_count == 0 {
        out.parse_errors
            .push("compact stream contains no closed game hours".to_string());
    }
    let fatal = compact_status_failure_flags_after_expected_partial_tolerance(
        DOMAIN_ID,
        out.status_flags_or,
    );
    if fatal != 0 {
        out.parse_errors.push(format!(
            "domain {DOMAIN_ID} status contains fatal bits {fatal:#010x}"
        ));
    }
    let allowed_status = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK
        | compact::STATUS_OPERATIONS_DETAIL_RESISTED
        | compact::STATUS_INTEL_MAP_RESISTED
        | compact::STATUS_WAR_ROOT_UNVALIDATED
        | compact::STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED
        | compact::STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED
        | compact::STATUS_FACTION_GROUP_UNVALIDATED
        | compact::STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED;
    let unknown = out.status_flags_or & !allowed_status;
    if unknown != 0 {
        out.parse_errors.push(format!(
            "domain {DOMAIN_ID} status contains unknown bits {unknown:#010x}"
        ));
    }
    out
}

/// Produce a synthetic, explicitly lane-local one-hour proof stream.
pub fn encode_domain53_proof_stream(game_hour_raw: i32, payload: &[u8]) -> Result<Vec<u8>, String> {
    let decoded = compact::decode_payload(0, payload);
    if !decoded.parse_errors.is_empty() {
        return Err(format!(
            "domain53 payload is invalid: {:?}",
            decoded.parse_errors
        ));
    }
    let rows = u32::try_from(decoded.data_rows_total)
        .map_err(|_| "domain53 row count exceeds u32 PERF authority".to_string())?;
    let changed = u32::try_from(decoded.data_changed_rows_total)
        .map_err(|_| "domain53 changed-row count exceeds u32 PERF authority".to_string())?;
    let mut perf = vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN];
    perf[16..20].copy_from_slice(&rows.to_le_bytes());
    perf[20..24].copy_from_slice(&changed.to_le_bytes());
    perf[24..28].copy_from_slice(&rows.to_le_bytes());
    perf[28..32].copy_from_slice(&changed.to_le_bytes());
    perf[32..36].copy_from_slice(&decoded.status_flags_or.to_le_bytes());

    let mut out = Vec::new();
    let mut frame_end = [0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
    frame_end[16..20].copy_from_slice(&1u32.to_le_bytes());
    frame_end[20..24].copy_from_slice(&1u32.to_le_bytes());
    append_record(
        &mut out,
        1,
        game_hour_raw,
        DOMAIN_ID,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &perf,
    );
    append_record(
        &mut out,
        2,
        game_hour_raw,
        DOMAIN_ID,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        payload,
    );
    append_record(
        &mut out,
        3,
        game_hour_raw,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end,
    );
    Ok(out)
}

fn append_record(
    out: &mut Vec<u8>,
    sequence: u64,
    hour: i32,
    domain: u16,
    record_type: u16,
    payload: &[u8],
) {
    out.extend_from_slice(b"H4CR");
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&hour.to_le_bytes());
    out.extend_from_slice(&domain.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
}

fn decode_perf_authority(payload: &[u8]) -> Authority {
    Authority {
        rows: read_u32(payload, 16) as u64,
        changed_rows: read_u32(payload, 20) as u64,
        units: read_u32(payload, 24) as u64,
        changed_units: read_u32(payload, 28) as u64,
        status: read_u32(payload, 32),
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated PERF"),
    )
}

fn validate_sequence(errors: &mut Vec<String>, previous: Option<u64>, current: u64, offset: usize) {
    match previous {
        None if current != 1 => errors.push(format!(
            "offset {offset}: sequence {current} does not start at 1"
        )),
        Some(previous) => match previous.checked_add(1) {
            Some(expected) if current != expected => errors.push(format!(
                "offset {offset}: sequence discontinuity {previous} -> {current} (expected {expected})"
            )),
            None => errors.push(format!(
                "offset {offset}: sequence space exhausted after {previous}; wrapping is forbidden"
            )),
            _ => {}
        },
        _ => {}
    }
}

fn validate_next_hour(
    errors: &mut Vec<String>,
    last_closed: Option<i32>,
    hour: i32,
    label: &str,
    offset: usize,
) {
    let Some(last) = last_closed else { return };
    match last.checked_add(1) {
        Some(expected) if hour == expected => {}
        Some(expected) => errors.push(format!(
            "offset {offset}: {label} hour {hour} follows closed hour {last}; expected {expected}"
        )),
        None => errors.push(format!(
            "offset {offset}: closed hour {last} exhausted; another record is forbidden"
        )),
    }
}

fn observe_perf_hour(
    errors: &mut Vec<String>,
    pending: &mut Option<i32>,
    open: Option<i32>,
    last_closed: Option<i32>,
    hour: i32,
    offset: usize,
) {
    if let Some(open) = open {
        if open != hour {
            errors.push(format!(
                "offset {offset}: DOMAIN_PERF hour {hour} does not match open DATA frame hour {open}"
            ));
        }
        return;
    }
    validate_next_hour(errors, last_closed, hour, "DOMAIN_PERF", offset);
    match *pending {
        Some(existing) if existing != hour => errors.push(format!(
            "offset {offset}: DOMAIN_PERF hour {hour} conflicts with pending frame hour {existing}"
        )),
        None => *pending = Some(hour),
        _ => {}
    }
}

fn observe_data_hour(
    errors: &mut Vec<String>,
    pending: Option<i32>,
    open: &mut Option<i32>,
    last_closed: Option<i32>,
    hour: i32,
    offset: usize,
) {
    match *open {
        Some(existing) if existing != hour => errors.push(format!(
            "offset {offset}: DOMAIN_DATA hour {hour} encountered before FRAME_END closed hour {existing}"
        )),
        Some(_) => {}
        None => {
            validate_next_hour(errors, last_closed, hour, "DOMAIN_DATA", offset);
            if let Some(pending) = pending {
                if pending != hour {
                    errors.push(format!(
                        "offset {offset}: DOMAIN_DATA hour {hour} conflicts with pending PERF hour {pending}"
                    ));
                }
            }
            *open = Some(hour);
        }
    }
}

fn validate_frame_end(
    errors: &mut Vec<String>,
    semantics_ok: bool,
    open: Option<i32>,
    last_closed: Option<i32>,
    hour: i32,
    offset: usize,
) -> bool {
    let mut clean = semantics_ok;
    match open {
        Some(existing) if existing == hour => {}
        Some(existing) => {
            clean = false;
            errors.push(format!(
                "offset {offset}: FRAME_END hour {hour} does not close open DATA frame hour {existing}"
            ));
        }
        None => {
            clean = false;
            errors.push(format!(
                "offset {offset}: FRAME_END hour {hour} has no open DATA frame"
            ));
        }
    }
    if let Some(last) = last_closed {
        if last.checked_add(1) != Some(hour) {
            clean = false;
        }
    }
    clean
}

fn validate_frame_end_payload(
    errors: &mut Vec<String>,
    payload: &[u8],
    offset: usize,
    frame_records: &FrameRecordCounts,
) -> bool {
    let emitted = read_u32(payload, 16);
    let scheduled = read_u32(payload, 20);
    let status = read_u32(payload, 24);
    let reserved = read_u32(payload, 28);
    let mut clean = true;
    if emitted != scheduled || emitted == 0 {
        clean = false;
        errors.push(format!(
            "offset {offset}: FRAME_END emitted/scheduled counts {emitted}/{scheduled} are not equal and nonzero"
        ));
    }
    if u64::from(emitted) != frame_records.data || u64::from(scheduled) != frame_records.perf {
        clean = false;
        errors.push(format!(
            "offset {offset}: FRAME_END counts {emitted}/{scheduled} do not bind observed DATA/PERF counts {}/{}",
            frame_records.data, frame_records.perf
        ));
    }
    if reserved != 0 {
        clean = false;
        errors.push(format!(
            "offset {offset}: FRAME_END reserved word is nonzero"
        ));
    }
    let unknown = compact_contract::compact_status_unknown_flags_v1(status);
    if unknown != 0 {
        clean = false;
        errors.push(format!(
            "offset {offset}: FRAME_END status contains unknown v1 bits {unknown:#010x}"
        ));
    }
    let fatal =
        compact_status_failure_flags_after_expected_partial_tolerance(COMPACT_DOMAIN_FRAME, status);
    if fatal != 0 {
        clean = false;
        errors.push(format!(
            "offset {offset}: FRAME_END status contains fatal bits {fatal:#010x}"
        ));
    }
    clean
}

fn validate_owned_authority(
    errors: &mut Vec<String>,
    hour: i32,
    offset: usize,
    owned: &OwnedFrameAuthority,
) {
    if owned.perf_count != 1 {
        errors.push(format!(
            "offset {offset}: hour {hour} domain {DOMAIN_ID} PERF count {} != 1",
            owned.perf_count
        ));
    }
    if owned.data_count != 1 {
        errors.push(format!(
            "offset {offset}: hour {hour} domain {DOMAIN_ID} DATA count {} != 1",
            owned.data_count
        ));
    }
    if owned.perf_count == 1 && owned.data_count == 1 {
        if let (Some(perf), Some(data)) = (owned.perf, owned.data) {
            // D53's first two PERF counters are observed-work telemetry,
            // while unit_count/changed_unit_count are the emitted DATA
            // authority. Do not invent equality between observed work and
            // emitted rows (delta/partial modes legitimately diverge).
            let matches = perf.units == data.rows
                && perf.changed_units == data.changed_rows
                && perf.status == data.status;
            if !matches {
                errors.push(format!(
                    "offset {offset}: hour {hour} domain {DOMAIN_ID} PERF/DATA cardinality or status mismatch: perf={perf:?} data={data:?}"
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::{
        build_domain53_payload, proof_fixture_rows, CURRENT_VALIDATED_SOURCE_PROOFS,
    };

    fn stream() -> Vec<u8> {
        let extraction =
            build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        assert!(extraction.ok(), "{:?}", extraction.errors);
        encode_domain53_proof_stream(100, &extraction.payload).unwrap()
    }

    fn offsets(bytes: &[u8]) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cursor = 0;
        while cursor < bytes.len() {
            out.push(cursor);
            cursor += parse_compact_record_header(&bytes[cursor..])
                .unwrap()
                .record_len();
        }
        out
    }

    #[test]
    fn accepts_one_canonical_closed_hour() {
        let gate = verify_domain53_stream(&stream());
        assert!(gate.parse_errors.is_empty(), "{:?}", gate.parse_errors);
        assert_eq!(gate.record_count, 3);
        assert_eq!(gate.domain_perf_record_count, 1);
        assert_eq!(gate.domain_data_record_count, 1);
        assert_eq!(gate.frame_end_record_count, 1);
        assert_eq!(gate.closed_game_hours, vec![100]);
    }

    #[test]
    fn rejects_bad_flags_sequence_and_unclosed_tail() {
        let mut flags = stream();
        let data = offsets(&flags)[1];
        flags[data + 28..data + 32].copy_from_slice(&1u32.to_le_bytes());
        assert!(verify_domain53_stream(&flags)
            .parse_errors
            .iter()
            .any(|error| error.contains("rejected bits")));

        let mut sequence = stream();
        sequence[data + 8..data + 16].copy_from_slice(&4u64.to_le_bytes());
        assert!(verify_domain53_stream(&sequence)
            .parse_errors
            .iter()
            .any(|error| error.contains("sequence discontinuity")));

        let valid = stream();
        let frame = offsets(&valid)[2];
        assert!(verify_domain53_stream(&valid[..frame])
            .parse_errors
            .iter()
            .any(|error| error.contains("final canonical FRAME_END")));
    }

    #[test]
    fn rejects_missing_and_mismatched_authority() {
        let valid = stream();
        let record_offsets = offsets(&valid);
        let no_perf = valid[record_offsets[1]..].to_vec();
        assert!(verify_domain53_stream(&no_perf)
            .parse_errors
            .iter()
            .any(|error| error.contains("PERF count 0 != 1")));

        let mut rows = valid.clone();
        rows[COMPACT_RECORD_HEADER_LEN + 24..COMPACT_RECORD_HEADER_LEN + 28]
            .copy_from_slice(&0u32.to_le_bytes());
        assert!(verify_domain53_stream(&rows)
            .parse_errors
            .iter()
            .any(|error| error.contains("cardinality or status mismatch")));

        let mut status = valid;
        status[COMPACT_RECORD_HEADER_LEN + 32..COMPACT_RECORD_HEADER_LEN + 36]
            .copy_from_slice(&compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE.to_le_bytes());
        let errors = verify_domain53_stream(&status).parse_errors;
        assert!(errors
            .iter()
            .any(|error| error.contains("cardinality or status mismatch")));
        assert!(errors.iter().any(|error| error.contains("fatal bits")));
    }

    #[test]
    fn accepts_observed_work_above_emitted_rows_but_rejects_frame_authority_defects() {
        let mut observed_work = stream();
        observed_work[COMPACT_RECORD_HEADER_LEN + 16..COMPACT_RECORD_HEADER_LEN + 20]
            .copy_from_slice(&999u32.to_le_bytes());
        observed_work[COMPACT_RECORD_HEADER_LEN + 20..COMPACT_RECORD_HEADER_LEN + 24]
            .copy_from_slice(&998u32.to_le_bytes());
        let gate = verify_domain53_stream(&observed_work);
        assert!(gate.parse_errors.is_empty(), "{:?}", gate.parse_errors);

        let mut frame = stream();
        let frame_offset = offsets(&frame)[2] + COMPACT_RECORD_HEADER_LEN;
        frame[frame_offset + 28..frame_offset + 32].copy_from_slice(&1u32.to_le_bytes());
        assert!(verify_domain53_stream(&frame)
            .parse_errors
            .iter()
            .any(|error| error.contains("reserved word")));

        let mut forged_equal_counts = stream();
        let frame_offset = offsets(&forged_equal_counts)[2] + COMPACT_RECORD_HEADER_LEN;
        forged_equal_counts[frame_offset + 16..frame_offset + 20]
            .copy_from_slice(&2u32.to_le_bytes());
        forged_equal_counts[frame_offset + 20..frame_offset + 24]
            .copy_from_slice(&2u32.to_le_bytes());
        assert!(verify_domain53_stream(&forged_equal_counts)
            .parse_errors
            .iter()
            .any(|error| error.contains("do not bind observed DATA/PERF counts")));
    }

    #[test]
    fn rejects_noncanonical_record_type_even_with_valid_payload_bytes() {
        let mut forged = stream();
        let data = offsets(&forged)[1];
        forged[data + 22..data + 24].copy_from_slice(&99u16.to_le_bytes());
        let errors = verify_domain53_stream(&forged).parse_errors;
        assert!(!errors.is_empty());
        assert!(errors
            .iter()
            .any(|error| error.contains("record type") || error.contains("FRAME_END")));
    }

    #[test]
    fn rejects_nonconsecutive_hours_and_unknown_status() {
        let mut first = stream();
        let extraction =
            build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        let mut second = encode_domain53_proof_stream(102, &extraction.payload).unwrap();
        for (index, offset) in offsets(&second).into_iter().enumerate() {
            second[offset + 8..offset + 16].copy_from_slice(&(4 + index as u64).to_le_bytes());
        }
        first.extend_from_slice(&second);
        assert!(verify_domain53_stream(&first)
            .parse_errors
            .iter()
            .any(|error| error.contains("expected 101")));

        let mut unknown = stream();
        unknown[COMPACT_RECORD_HEADER_LEN + 32..COMPACT_RECORD_HEADER_LEN + 36]
            .copy_from_slice(&(1u32 << 31).to_le_bytes());
        assert!(verify_domain53_stream(&unknown)
            .parse_errors
            .iter()
            .any(|error| error.contains("unknown bits")));
    }
}
