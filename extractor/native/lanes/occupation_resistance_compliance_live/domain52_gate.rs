#[path = "../../compact_contract/src/record_header.rs"]
#[allow(dead_code)]
mod compact_record_header;
#[path = "../../compact_contract/src/record_semantics.rs"]
#[allow(dead_code)]
mod compact_record_semantics;
#[path = "../../compact_contract/src/status_flags_base.rs"]
#[allow(dead_code)]
mod compact_status_flags;
#[path = "domain52_compact.rs"]
#[allow(dead_code)]
mod domain52_compact;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process;

use compact_record_header::{
    parse_compact_record_header, RecordFramingError, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_VERSION,
};
use compact_record_semantics::{
    validate_compact_record_semantics, COMPACT_DOMAIN_FRAME, COMPACT_DOMAIN_PERF_BASE_LEN,
    COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC, COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION,
    COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END,
};
use compact_status_flags::{
    compact_status_unknown_flags_v1, COMPACT_STATUS_BRIDGE_UNAVAILABLE,
    COMPACT_STATUS_CONTROL_INVALID, COMPACT_STATUS_NO_CLOCK, COMPACT_STATUS_NO_SCALAR_SOURCE,
    COMPACT_STATUS_PARTIAL_DATA, COMPACT_STATUS_PROFILE_MISMATCH,
    COMPACT_STATUS_SOURCE_UNAVAILABLE, COMPACT_STATUS_STATE_UNAVAILABLE,
    COMPACT_STATUS_WORKER_PANIC,
};

const COMPACT_STATUS_FAILURE_MASK: u32 = COMPACT_STATUS_STATE_UNAVAILABLE
    | COMPACT_STATUS_PROFILE_MISMATCH
    | COMPACT_STATUS_NO_CLOCK
    | COMPACT_STATUS_NO_SCALAR_SOURCE
    | COMPACT_STATUS_WORKER_PANIC
    | COMPACT_STATUS_BRIDGE_UNAVAILABLE
    | COMPACT_STATUS_SOURCE_UNAVAILABLE
    | COMPACT_STATUS_PARTIAL_DATA
    | COMPACT_STATUS_CONTROL_INVALID;

fn main() {
    let mut stream_mode = false;
    let mut input_path = None;
    for arg in env::args().skip(1) {
        if arg == "--stream" {
            stream_mode = true;
        } else if input_path.is_none() {
            input_path = Some(arg);
        } else {
            eprintln!("usage: domain52_gate [--stream] <payload-or-stream-path>");
            process::exit(64);
        }
    }

    let Some(input_path) = input_path else {
        eprintln!("usage: domain52_gate [--stream] <payload-or-stream-path>");
        process::exit(64);
    };

    let bytes = match fs::read(&input_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("failed to read {input_path}: {err}");
            process::exit(66);
        }
    };

    let result = if stream_mode {
        decode_stream(&bytes)
    } else {
        let summary = domain52_compact::decode_payload(0, &bytes);
        let mut errors = summary.parse_errors;
        validate_known_status_flags(&mut errors, "DOMAIN_DATA", 0, summary.status_flags_or);
        GateResult {
            records_seen: 1,
            domain_perf_records: 0,
            domain_data_records: 1,
            rows: summary.data_rows_total,
            changed_rows: summary.data_changed_rows_total,
            status_flags_or: summary.status_flags_or,
            units: summary.data_units_total,
            errors: errors_with_fatal_status(errors, summary.status_flags_or),
        }
    };

    println!("expected_domain_id={}", domain52_compact::DOMAIN_ID);
    println!("records_seen={}", result.records_seen);
    println!("domain_perf_records={}", result.domain_perf_records);
    println!("domain_data_records={}", result.domain_data_records);
    println!("rows={}", result.rows);
    println!("changed_rows={}", result.changed_rows);
    println!("status_flags_or=0x{:08X}", result.status_flags_or);
    for (key, value) in &result.units {
        println!("{key}={value}");
    }

    if !result.errors.is_empty() {
        for error in result.errors {
            eprintln!("{error}");
        }
        process::exit(1);
    }
}

struct GateResult {
    records_seen: u64,
    domain_perf_records: u64,
    domain_data_records: u64,
    rows: u64,
    changed_rows: u64,
    status_flags_or: u32,
    units: BTreeMap<&'static str, u64>,
    errors: Vec<String>,
}

#[derive(Default)]
struct OwnedFrameAuthority {
    emitted_perf_records: u64,
    perf_records: u64,
    data_records: u64,
    perf_status_flags_or: u32,
    data_status_flags_or: u32,
}

fn decode_stream(bytes: &[u8]) -> GateResult {
    let mut cursor = 0usize;
    let mut records_seen = 0u64;
    let mut domain_perf_records = 0u64;
    let mut domain_data_records = 0u64;
    let mut rows = 0u64;
    let mut changed_rows = 0u64;
    let mut status_flags_or = 0u32;
    let mut units = BTreeMap::new();
    let mut errors = Vec::new();
    if let Err(error) = compact_contract::stream_validator::validate_freeze_census_stream_placement(bytes) {
        errors.push(format!("stateful stream placement: {error}"));
    }
    let mut previous_sequence: Option<u64> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut open_frame_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut final_record_is_clean_frame_end = false;
    let mut release_epilogue_seen = false;
    let mut owned_frame = OwnedFrameAuthority::default();
    let mut repeat_baselines = compact_contract::DomainRepeatBaselines::new();

    while cursor < bytes.len() {
        let record_offset = cursor;
        let header = match parse_compact_record_header(&bytes[cursor..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(_)) => {
                errors.push(format!(
                    "offset {record_offset}: compact record has truncated header"
                ));
                break;
            }
            Err(RecordFramingError::BadMagic) => {
                errors.push(format!(
                    "offset {record_offset}: compact record magic mismatch"
                ));
                break;
            }
            Err(RecordFramingError::UnsupportedVersion(version)) => {
                errors.push(format!(
                    "offset {record_offset}: compact record version {version} != {COMPACT_RECORD_VERSION}"
                ));
                break;
            }
            Err(RecordFramingError::UnsupportedHeaderLen(header_len)) => {
                errors.push(format!(
                    "offset {record_offset}: compact record header_len {header_len} != {COMPACT_RECORD_HEADER_LEN}"
                ));
                break;
            }
            Err(err @ RecordFramingError::PayloadTooLarge(_)) => {
                errors.push(format!("offset {record_offset}: {err}"));
                break;
            }
        };
        match previous_sequence {
            None if header.sequence != 1 => errors.push(format!(
                "offset {record_offset}: sequence {} does not start at 1 (stream prefix is missing)",
                header.sequence
            )),
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if header.sequence != expected => errors.push(format!(
                    "offset {record_offset}: sequence discontinuity {previous} -> {} (expected {expected})",
                    header.sequence
                )),
                None => errors.push(format!(
                    "offset {record_offset}: sequence space exhausted after {previous}; got {} (wrapping is not allowed)",
                    header.sequence
                )),
                _ => {}
            },
            _ => {}
        }
        previous_sequence = Some(header.sequence);
        let previous_record_closed_world = final_record_is_clean_frame_end;
        final_record_is_clean_frame_end = false;
        cursor += COMPACT_RECORD_HEADER_LEN;
        let Some(payload_end) = cursor.checked_add(header.payload_len) else {
            errors.push(format!(
                "offset {record_offset}: compact record payload length overflows"
            ));
            break;
        };
        if payload_end > bytes.len() {
            errors.push(format!(
                "offset {record_offset}: compact record payload is truncated"
            ));
            break;
        }
        records_seen = records_seen.saturating_add(1);
        let payload = &bytes[cursor..payload_end];
        let mut repeat_reconstruction: Option<Vec<u8>> = None;
        let record_semantics_ok = match validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        ) {
            Ok(()) => {
                match repeat_baselines.observe(header.domain_id, header.record_type, payload) {
                    Ok(reconstructed) => repeat_reconstruction = reconstructed,
                    Err(error) => {
                        errors.push(format!("offset {record_offset}: {error}"));
                    }
                }
                true
            },
            Err(error) => {
                errors.push(format!("offset {record_offset}: {error}"));
                false
            }
        };
        let known_record_type =
            match compact_contract::KnownRecordType::try_from(header.record_type) {
                Ok(record_type) => record_type,
                Err(unknown) => {
                    errors.push(format!(
                        "offset {record_offset}: unsupported compact record type {}",
                        unknown.0
                    ));
                    cursor = payload_end;
                    continue;
                }
            };
        match known_record_type {
            compact_contract::KnownRecordType::DomainPerf => {
                if record_semantics_ok {
                    owned_frame.emitted_perf_records =
                        owned_frame.emitted_perf_records.saturating_add(1);
                    observe_perf_hour(
                        &mut errors,
                        &mut pending_perf_hour,
                        open_frame_hour,
                        last_closed_hour,
                        header.game_hour_raw,
                        record_offset,
                    );
                    if header.domain_id == domain52_compact::DOMAIN_ID {
                        let status_flags = read_u32_le(payload, 32);
                        validate_known_status_flags(
                            &mut errors,
                            "DOMAIN_PERF",
                            record_offset,
                            status_flags,
                        );
                        domain_perf_records = domain_perf_records.saturating_add(1);
                        owned_frame.perf_records = owned_frame.perf_records.saturating_add(1);
                        owned_frame.perf_status_flags_or |= status_flags;
                        status_flags_or |= status_flags;
                    }
                }
            }
            compact_contract::KnownRecordType::DomainData => {
                if record_semantics_ok {
                    observe_data_hour(
                        &mut errors,
                        pending_perf_hour,
                        &mut open_frame_hour,
                        last_closed_hour,
                        header.game_hour_raw,
                        record_offset,
                    );
                    if header.domain_id == domain52_compact::DOMAIN_ID {
                        let summary = domain52_compact::decode_payload(cursor, payload);
                        validate_known_status_flags(
                            &mut errors,
                            "DOMAIN_DATA",
                            record_offset,
                            summary.status_flags_or,
                        );
                        domain_data_records = domain_data_records.saturating_add(1);
                        owned_frame.data_records = owned_frame.data_records.saturating_add(1);
                        owned_frame.data_status_flags_or |= summary.status_flags_or;
                        status_flags_or |= summary.status_flags_or;
                        rows = rows.saturating_add(summary.data_rows_total);
                        changed_rows = changed_rows.saturating_add(summary.data_changed_rows_total);
                        for (key, value) in summary.data_units_total {
                            *units.entry(key).or_default() += value;
                        }
                        errors.extend(summary.parse_errors);
                    }
                }
            }
            // A repeat covers its hour like DATA and satisfies the same
            // frame authority; the d52 grammar runs on the reconstruction
            // verified above, so status/row totals stay exact.
            compact_contract::KnownRecordType::DomainDataRepeat => {
                if record_semantics_ok {
                    observe_data_hour(
                        &mut errors,
                        pending_perf_hour,
                        &mut open_frame_hour,
                        last_closed_hour,
                        header.game_hour_raw,
                        record_offset,
                    );
                    if header.domain_id == domain52_compact::DOMAIN_ID {
                        let Some(reconstructed) = repeat_reconstruction.as_ref() else {
                            errors.push(format!(
                                "offset {record_offset}: DOMAIN_DATA_REPEAT resolved to no payload"
                            ));
                            continue;
                        };
                        let summary = domain52_compact::decode_payload(cursor, reconstructed);
                        validate_known_status_flags(
                            &mut errors,
                            "DOMAIN_DATA_REPEAT",
                            record_offset,
                            summary.status_flags_or,
                        );
                        domain_data_records = domain_data_records.saturating_add(1);
                        owned_frame.data_records = owned_frame.data_records.saturating_add(1);
                        owned_frame.data_status_flags_or |= summary.status_flags_or;
                        status_flags_or |= summary.status_flags_or;
                        rows = rows.saturating_add(summary.data_rows_total);
                        changed_rows = changed_rows.saturating_add(summary.data_changed_rows_total);
                        for (key, value) in summary.data_units_total {
                            *units.entry(key).or_default() += value;
                        }
                        errors.extend(summary.parse_errors);
                    }
                }
            }
            compact_contract::KnownRecordType::FrameEnd => {
                let mut clean = record_semantics_ok;
                if record_semantics_ok {
                    let frame_status_flags = read_u32_le(payload, 24);
                    status_flags_or |= frame_status_flags;
                    clean &= validate_frame_end_authority(
                        &mut errors,
                        record_offset,
                        payload,
                        owned_frame.emitted_perf_records,
                    );
                    validate_owned_frame_authority(
                        &mut errors,
                        header.game_hour_raw,
                        record_offset,
                        &owned_frame,
                    );
                }
                match open_frame_hour {
                    Some(open) if open == header.game_hour_raw => {}
                    Some(open) => {
                        clean = false;
                        errors.push(format!(
                            "offset {record_offset}: FRAME_END hour {} does not close open DATA frame hour {open}",
                            header.game_hour_raw
                        ));
                    }
                    None => {
                        clean = false;
                        errors.push(format!(
                            "offset {record_offset}: FRAME_END hour {} has no open DATA frame",
                            header.game_hour_raw
                        ));
                    }
                }
                if let Some(closed) = last_closed_hour {
                    match closed.checked_add(1) {
                        Some(expected) if header.game_hour_raw == expected => {}
                        Some(expected) => {
                            clean = false;
                            errors.push(format!(
                                "offset {record_offset}: FRAME_END hour {} follows closed frame hour {closed}; expected {expected}",
                                header.game_hour_raw
                            ));
                        }
                        None => {
                            clean = false;
                            errors.push(format!(
                                "offset {record_offset}: closed frame hour {closed} exhausted; another frame is forbidden"
                            ));
                        }
                    }
                }
                if clean {
                    open_frame_hour = None;
                    pending_perf_hour = None;
                    last_closed_hour = Some(header.game_hour_raw);
                    final_record_is_clean_frame_end = true;
                    owned_frame = OwnedFrameAuthority::default();
                }
            }
            compact_contract::KnownRecordType::FreezeCensus => {
                if let Ok(census) = compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload) {
                    final_record_is_clean_frame_end = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(previous_record_closed_world, census.phase(), release_epilogue_seen);
                    release_epilogue_seen |= census.phase() == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                }
            }
            compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StateTopologyCensus
            | compact_contract::KnownRecordType::StreamHeader => {}
            compact_contract::KnownRecordType::DomainGap
            | compact_contract::KnownRecordType::FrameRefused
            | compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => errors.push(format!(
                "offset {record_offset}: unsupported compact record type {}",
                known_record_type.canonical_name()
            )),
        }
        cursor = payload_end;
    }

    if let Some(open) = open_frame_hour {
        errors.push(format!("stream ended with open DATA frame hour {open}"));
    }
    if !final_record_is_clean_frame_end {
        errors.push("stream is not closed by a final canonical FRAME_END record".to_string());
    }

    if domain_data_records == 0 {
        errors.push(format!(
            "compact stream did not contain domain {} data records",
            domain52_compact::DOMAIN_ID
        ));
    }
    if domain_perf_records == 0 {
        errors.push(format!(
            "compact stream did not contain domain {} perf records",
            domain52_compact::DOMAIN_ID
        ));
    }
    errors = errors_with_fatal_status(errors, status_flags_or);

    GateResult {
        records_seen,
        domain_perf_records,
        domain_data_records,
        rows,
        changed_rows,
        status_flags_or,
        units,
        errors,
    }
}

fn validate_frame_end_authority(
    errors: &mut Vec<String>,
    offset: usize,
    payload: &[u8],
    observed_perf_records: u64,
) -> bool {
    let emitted_domains = read_u32_le(payload, 16);
    let scheduled_domains = read_u32_le(payload, 20);
    let status_flags = read_u32_le(payload, 24);
    let reserved = read_u32_le(payload, 28);
    let mut valid = true;

    if u64::from(emitted_domains) != observed_perf_records {
        valid = false;
        errors.push(format!(
            "offset {offset}: FRAME_END emitted_domains {emitted_domains} != independently observed DOMAIN_PERF count {observed_perf_records}"
        ));
    }
    if scheduled_domains != emitted_domains {
        valid = false;
        errors.push(format!(
            "offset {offset}: FRAME_END scheduled_domains {scheduled_domains} != emitted_domains {emitted_domains}"
        ));
    }
    if reserved != 0 {
        valid = false;
        errors.push(format!(
            "offset {offset}: FRAME_END reserved word is nonzero ({reserved:#010x})"
        ));
    }
    if !validate_known_status_flags(errors, "FRAME_END", offset, status_flags) {
        valid = false;
    }
    let fatal = status_flags & COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        valid = false;
        errors.push(format!(
            "offset {offset}: FRAME_END status flags {status_flags:#010x} contain fatal bits {fatal:#010x}"
        ));
    }
    valid
}

fn validate_known_status_flags(
    errors: &mut Vec<String>,
    record_label: &str,
    offset: usize,
    status_flags: u32,
) -> bool {
    let unknown = compact_status_unknown_flags_v1(status_flags);
    if unknown == 0 {
        return true;
    }
    errors.push(format!(
        "offset {offset}: {record_label} status flags {status_flags:#010x} contain unknown bits {unknown:#010x}"
    ));
    false
}

fn validate_owned_frame_authority(
    errors: &mut Vec<String>,
    game_hour_raw: i32,
    offset: usize,
    authority: &OwnedFrameAuthority,
) {
    if authority.perf_records != 1 {
        errors.push(format!(
            "offset {offset}: game_hour_raw {game_hour_raw} domain {} PERF record count {} != 1",
            domain52_compact::DOMAIN_ID,
            authority.perf_records
        ));
    }
    if authority.data_records != 1 {
        errors.push(format!(
            "offset {offset}: game_hour_raw {game_hour_raw} domain {} DATA record count {} != 1",
            domain52_compact::DOMAIN_ID,
            authority.data_records
        ));
    }
    if authority.perf_records == 1
        && authority.data_records == 1
        && authority.perf_status_flags_or != authority.data_status_flags_or
    {
        errors.push(format!(
            "offset {offset}: game_hour_raw {game_hour_raw} domain {} PERF status {:#010x} != DATA status {:#010x}",
            domain52_compact::DOMAIN_ID,
            authority.perf_status_flags_or,
            authority.data_status_flags_or
        ));
    }
}

fn errors_with_fatal_status(mut errors: Vec<String>, status_flags: u32) -> Vec<String> {
    let failure_flags = status_flags & COMPACT_STATUS_FAILURE_MASK;
    if failure_flags != 0 {
        errors.push(format!(
            "domain {} normalized status flags contain fatal bits {failure_flags:#010x}",
            domain52_compact::DOMAIN_ID
        ));
    }
    errors
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed-width u32 slice"),
    )
}

fn validate_next_hour(
    errors: &mut Vec<String>,
    last_closed_hour: Option<i32>,
    game_hour_raw: i32,
    record_label: &str,
    offset: usize,
) {
    let Some(closed) = last_closed_hour else {
        return;
    };
    match closed.checked_add(1) {
        Some(expected) if game_hour_raw == expected => {}
        Some(expected) => errors.push(format!(
            "offset {offset}: {record_label} hour {game_hour_raw} follows closed frame hour {closed}; expected {expected}"
        )),
        None => errors.push(format!(
            "offset {offset}: closed frame hour {closed} exhausted; another record is forbidden"
        )),
    }
}

fn observe_perf_hour(
    errors: &mut Vec<String>,
    pending_perf_hour: &mut Option<i32>,
    open_frame_hour: Option<i32>,
    last_closed_hour: Option<i32>,
    game_hour_raw: i32,
    offset: usize,
) {
    match open_frame_hour {
        Some(open) if open != game_hour_raw => errors.push(format!(
            "offset {offset}: DOMAIN_PERF hour {game_hour_raw} does not match open DATA frame hour {open}"
        )),
        Some(_) => {}
        None => {
            validate_next_hour(
                errors,
                last_closed_hour,
                game_hour_raw,
                "DOMAIN_PERF",
                offset,
            );
            match *pending_perf_hour {
                Some(pending) if pending != game_hour_raw => errors.push(format!(
                    "offset {offset}: DOMAIN_PERF hour {game_hour_raw} conflicts with pending frame hour {pending}"
                )),
                Some(_) => {}
                None => *pending_perf_hour = Some(game_hour_raw),
            }
        }
    }
}

fn observe_data_hour(
    errors: &mut Vec<String>,
    pending_perf_hour: Option<i32>,
    open_frame_hour: &mut Option<i32>,
    last_closed_hour: Option<i32>,
    game_hour_raw: i32,
    offset: usize,
) {
    match *open_frame_hour {
        Some(open) if open != game_hour_raw => errors.push(format!(
            "offset {offset}: DOMAIN_DATA hour {game_hour_raw} encountered before FRAME_END closed open frame hour {open}"
        )),
        Some(_) => {}
        None => {
            validate_next_hour(
                errors,
                last_closed_hour,
                game_hour_raw,
                "DOMAIN_DATA",
                offset,
            );
            if let Some(pending) = pending_perf_hour {
                if pending != game_hour_raw {
                    errors.push(format!(
                        "offset {offset}: DOMAIN_DATA hour {game_hour_raw} conflicts with pending PERF hour {pending}"
                    ));
                }
            }
            *open_frame_hour = Some(game_hour_raw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_gate_enforces_canonical_record_flag_placement() {
        let mut legal = valid_stream(0);
        replace_record_payload(&mut legal, 0, &perf_payload_with_empty_diag());
        legal[28..32].copy_from_slice(&COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER.to_le_bytes());
        assert!(decode_stream(&legal).errors.is_empty());

        let mut illegal = valid_stream(0);
        let data_offset = COMPACT_RECORD_HEADER_LEN + COMPACT_DOMAIN_PERF_BASE_LEN;
        illegal[data_offset + 28..data_offset + 32]
            .copy_from_slice(&COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER.to_le_bytes());
        assert!(decode_stream(&illegal)
            .errors
            .iter()
            .any(|error| error.contains("contain rejected bits")));
    }

    #[test]
    fn stream_gate_binds_frame_end_to_observed_perf_and_status_authority() {
        let cases = [
            (
                frame_payload_with_authority(2, 2, 0, 0),
                "independently observed DOMAIN_PERF count 1",
            ),
            (
                frame_payload_with_authority(1, 2, 0, 0),
                "scheduled_domains 2 != emitted_domains 1",
            ),
            (
                frame_payload_with_authority(1, 1, 0, 7),
                "FRAME_END reserved word is nonzero",
            ),
            (
                frame_payload_with_authority(1, 1, 1 << 31, 0),
                "FRAME_END status flags 0x80000000 contain unknown bits",
            ),
            (
                frame_payload_with_authority(1, 1, COMPACT_STATUS_SOURCE_UNAVAILABLE, 0),
                "FRAME_END status flags 0x00000100 contain fatal bits",
            ),
        ];
        for (frame_payload, expected_error) in cases {
            let mut stream = valid_stream(0);
            let frame_offset = record_offsets(&stream)[2];
            replace_record_payload(&mut stream, frame_offset, &frame_payload);
            let errors = decode_stream(&stream).errors;
            assert!(
                errors.iter().any(|error| error.contains(expected_error)),
                "expected {expected_error:?} in {errors:?}"
            );
        }

        let informational = compact_status_flags::COMPACT_STATUS_EXTRACTION_PERF_GATE;
        let mut stream = valid_stream(0);
        let frame_offset = record_offsets(&stream)[2];
        replace_record_payload(
            &mut stream,
            frame_offset,
            &frame_payload_with_authority(1, 1, informational, 0),
        );
        let result = decode_stream(&stream);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.status_flags_or, informational);
    }

    #[test]
    fn stream_gate_rejects_unknown_owned_perf_and_data_status_bits() {
        let unknown = 1u32 << 31;

        let mut perf = valid_stream_with_status(0);
        let perf_offset = record_offsets(&perf)[0] + COMPACT_RECORD_HEADER_LEN;
        perf[perf_offset + 32..perf_offset + 36].copy_from_slice(&unknown.to_le_bytes());
        assert!(decode_stream(&perf)
            .errors
            .iter()
            .any(|error| error.contains("DOMAIN_PERF status flags")
                && error.contains("unknown bits")));

        let mut data = valid_stream_with_status(0);
        let data_offset = record_offsets(&data)[1];
        replace_record_payload(&mut data, data_offset, &valid_payload_with_status(unknown));
        assert!(decode_stream(&data)
            .errors
            .iter()
            .any(|error| error.contains("DOMAIN_DATA status flags")
                && error.contains("unknown bits")));
    }

    #[test]
    fn stream_gate_rejects_missing_gapped_and_wrapping_sequences() {
        let mut missing = valid_stream(0);
        missing[8..16].copy_from_slice(&2u64.to_le_bytes());
        assert!(decode_stream(&missing)
            .errors
            .iter()
            .any(|error| error.contains("does not start at 1")));

        let mut gap = valid_stream(0);
        let data_offset = COMPACT_RECORD_HEADER_LEN + COMPACT_DOMAIN_PERF_BASE_LEN;
        gap[data_offset + 8..data_offset + 16].copy_from_slice(&3u64.to_le_bytes());
        assert!(decode_stream(&gap)
            .errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 1 -> 3")));

        let mut wrapping = valid_stream(0);
        wrapping[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        wrapping[data_offset + 8..data_offset + 16].copy_from_slice(&0u64.to_le_bytes());
        assert!(decode_stream(&wrapping)
            .errors
            .iter()
            .any(|error| error.contains("wrapping is not allowed")));
    }

    #[test]
    fn stream_gate_rejects_missing_and_unannounced_perf_trailers_and_bad_frame_size() {
        let mut long_perf = valid_stream(0);
        replace_record_payload(&mut long_perf, 0, &[0; COMPACT_DOMAIN_PERF_BASE_LEN + 1]);
        assert!(decode_stream(&long_perf)
            .errors
            .iter()
            .any(|error| error.contains("unannounced trailer byte")));

        let announced_without_trailer = valid_stream(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER);
        assert!(decode_stream(&announced_without_trailer)
            .errors
            .iter()
            .any(|error| error.contains("diagnostic trailer header is truncated")));

        let mut short_frame = valid_stream(0);
        let frame_offset = record_offsets(&short_frame)[2];
        replace_record_payload(
            &mut short_frame,
            frame_offset,
            &[0; COMPACT_FRAME_END_PAYLOAD_LEN - 1],
        );
        assert!(decode_stream(&short_frame)
            .errors
            .iter()
            .any(|error| error.contains("FRAME_END payload length 31 is not 32")));

        let mut mismatched = valid_stream(0);
        let data_offset = record_offsets(&mismatched)[1];
        mismatched[data_offset + 16..data_offset + 20].copy_from_slice(&101i32.to_le_bytes());
        assert!(decode_stream(&mismatched)
            .errors
            .iter()
            .any(|error| error.contains("conflicts with pending PERF hour 100")));
    }

    #[test]
    fn stream_gate_rejects_trailing_closed_hour_and_malformed_data() {
        let mut trailing = valid_stream(0);
        append_record(
            &mut trailing,
            4,
            100,
            domain52_compact::DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &[0; COMPACT_DOMAIN_PERF_BASE_LEN],
        );
        assert!(decode_stream(&trailing).errors.iter().any(|error| {
            error.contains("follows closed frame hour 100")
                || error.contains("final canonical FRAME_END")
        }));

        let mut malformed = valid_stream(0);
        let data_offset = record_offsets(&malformed)[1];
        replace_record_payload(&mut malformed, data_offset, &[]);
        assert!(decode_stream(&malformed)
            .errors
            .iter()
            .any(|error| error.contains("expected at least")));
    }

    #[test]
    fn stream_gate_requires_exactly_one_owned_perf_and_data_per_closed_hour() {
        let mut missing_perf = Vec::new();
        append_record(
            &mut missing_perf,
            1,
            100,
            domain52_compact::DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &valid_payload_with_status(0),
        );
        append_frame_end(&mut missing_perf, 2, 100);
        assert!(decode_stream(&missing_perf)
            .errors
            .iter()
            .any(|error| error.contains("PERF record count 0 != 1")));

        let mut duplicate_perf = Vec::new();
        append_perf(&mut duplicate_perf, 1, 100, domain52_compact::DOMAIN_ID, 0);
        append_perf(&mut duplicate_perf, 2, 100, domain52_compact::DOMAIN_ID, 0);
        append_record(
            &mut duplicate_perf,
            3,
            100,
            domain52_compact::DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &valid_payload_with_status(0),
        );
        append_frame_end(&mut duplicate_perf, 4, 100);
        assert!(decode_stream(&duplicate_perf)
            .errors
            .iter()
            .any(|error| error.contains("PERF record count 2 != 1")));

        let mut duplicate_data = valid_stream(0);
        let frame_offset = record_offsets(&duplicate_data)[2];
        let mut duplicate = Vec::new();
        append_record(
            &mut duplicate,
            3,
            100,
            domain52_compact::DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &valid_payload_with_status(0),
        );
        duplicate_data.splice(frame_offset..frame_offset, duplicate);
        let new_frame_offset = record_offsets(&duplicate_data)[3];
        duplicate_data[new_frame_offset + 8..new_frame_offset + 16]
            .copy_from_slice(&4u64.to_le_bytes());
        assert!(decode_stream(&duplicate_data)
            .errors
            .iter()
            .any(|error| error.contains("DATA record count 2 != 1")));
    }

    #[test]
    fn stream_gate_rejects_bare_unrelated_and_perf_only_frames() {
        let mut bare = Vec::new();
        append_frame_end(&mut bare, 1, 100);
        assert!(decode_stream(&bare)
            .errors
            .iter()
            .any(|error| error.contains("has no open DATA frame")));

        let mut unrelated = Vec::new();
        append_perf(&mut unrelated, 1, 100, 18, 0);
        append_record(
            &mut unrelated,
            2,
            100,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &[],
        );
        append_frame_end(&mut unrelated, 3, 100);
        let unrelated_errors = decode_stream(&unrelated).errors;
        assert!(unrelated_errors
            .iter()
            .any(|error| error.contains("PERF record count 0 != 1")));
        assert!(unrelated_errors
            .iter()
            .any(|error| error.contains("DATA record count 0 != 1")));

        let mut perf_only = Vec::new();
        append_perf(&mut perf_only, 1, 100, domain52_compact::DOMAIN_ID, 0);
        append_frame_end(&mut perf_only, 2, 100);
        assert!(decode_stream(&perf_only)
            .errors
            .iter()
            .any(|error| error.contains("has no open DATA frame")));
    }

    #[test]
    fn stream_gate_rejects_hour_gaps_regressions_wrong_hour_perf_and_fatal_status() {
        let mut gap = Vec::new();
        append_owned_frame(&mut gap, 1, 100, 0);
        append_owned_frame(&mut gap, 4, 102, 0);
        assert!(decode_stream(&gap)
            .errors
            .iter()
            .any(|error| error.contains("expected 101")));

        let mut regression = Vec::new();
        append_owned_frame(&mut regression, 1, 100, 0);
        append_owned_frame(&mut regression, 4, 99, 0);
        assert!(decode_stream(&regression)
            .errors
            .iter()
            .any(|error| error.contains("expected 101")));

        let mut wrong_perf_hour = Vec::new();
        append_record(
            &mut wrong_perf_hour,
            1,
            100,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &[],
        );
        append_perf(&mut wrong_perf_hour, 2, 101, domain52_compact::DOMAIN_ID, 0);
        append_record(
            &mut wrong_perf_hour,
            3,
            100,
            domain52_compact::DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &valid_payload_with_status(0),
        );
        append_frame_end(&mut wrong_perf_hour, 4, 100);
        assert!(decode_stream(&wrong_perf_hour)
            .errors
            .iter()
            .any(|error| error
                .contains("DOMAIN_PERF hour 101 does not match open DATA frame hour 100")));

        let fatal = COMPACT_STATUS_SOURCE_UNAVAILABLE;
        let fatal_result = decode_stream(&valid_stream_with_status(fatal));
        assert_eq!(fatal_result.status_flags_or, fatal);
        assert!(fatal_result
            .errors
            .iter()
            .any(|error| error.contains("normalized status flags contain fatal bits")));

        let nonfatal = compact_status_flags::COMPACT_STATUS_DELTA_DATA;
        assert!(decode_stream(&valid_stream_with_status(nonfatal))
            .errors
            .is_empty());

        let mut mismatch = valid_stream_with_status(0);
        let perf_offset = record_offsets(&mismatch)[0];
        mismatch[perf_offset + COMPACT_RECORD_HEADER_LEN + 32
            ..perf_offset + COMPACT_RECORD_HEADER_LEN + 36]
            .copy_from_slice(&compact_status_flags::COMPACT_STATUS_DELTA_DATA.to_le_bytes());
        assert!(decode_stream(&mismatch)
            .errors
            .iter()
            .any(|error| error.contains("PERF status") && error.contains("!= DATA status")));
    }

    fn valid_stream(perf_flags: u32) -> Vec<u8> {
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            1,
            100,
            domain52_compact::DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            perf_flags,
            &[0; COMPACT_DOMAIN_PERF_BASE_LEN],
        );
        append_record(
            &mut stream,
            2,
            100,
            domain52_compact::DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &valid_payload(),
        );
        append_record(
            &mut stream,
            3,
            100,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_payload_with_authority(1, 1, 0, 0),
        );
        stream
    }

    fn valid_stream_with_status(status_flags: u32) -> Vec<u8> {
        let mut stream = Vec::new();
        append_owned_frame(&mut stream, 1, 100, status_flags);
        stream
    }

    fn append_owned_frame(
        out: &mut Vec<u8>,
        first_sequence: u64,
        game_hour_raw: i32,
        status_flags: u32,
    ) {
        append_perf(
            out,
            first_sequence,
            game_hour_raw,
            domain52_compact::DOMAIN_ID,
            status_flags,
        );
        append_record(
            out,
            first_sequence + 1,
            game_hour_raw,
            domain52_compact::DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &valid_payload_with_status(status_flags),
        );
        append_frame_end(out, first_sequence + 2, game_hour_raw);
    }

    fn append_perf(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        status_flags: u32,
    ) {
        let mut payload = vec![0; COMPACT_DOMAIN_PERF_BASE_LEN];
        payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
        append_record(
            out,
            sequence,
            game_hour_raw,
            domain_id,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &payload,
        );
    }

    fn append_frame_end(out: &mut Vec<u8>, sequence: u64, game_hour_raw: i32) {
        append_record(
            out,
            sequence,
            game_hour_raw,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_payload_with_authority(1, 1, 0, 0),
        );
    }

    fn frame_payload_with_authority(
        emitted_domains: u32,
        scheduled_domains: u32,
        status_flags: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = vec![0; COMPACT_FRAME_END_PAYLOAD_LEN];
        payload[16..20].copy_from_slice(&emitted_domains.to_le_bytes());
        payload[20..24].copy_from_slice(&scheduled_domains.to_le_bytes());
        payload[24..28].copy_from_slice(&status_flags.to_le_bytes());
        payload[28..32].copy_from_slice(&reserved.to_le_bytes());
        payload
    }

    fn valid_payload() -> Vec<u8> {
        valid_payload_with_status(0)
    }

    fn valid_payload_with_status(status_flags: u32) -> Vec<u8> {
        domain52_compact::encode_payload(
            &domain52_compact::OccupationHeader {
                elapsed_wall_ms: 0,
                status_flags,
                changed_rows: 0,
                observed_state_rows: 0,
                occupied_state_rows: 0,
                wall_badge_rows: 0,
                law_token_rows: 0,
                garrison_template_rows: 0,
                garrison_scalar_rows: 0,
                garrison_anomaly_rows: 0,
                invalid_state_rows: 0,
            },
            &[],
        )
    }

    fn perf_payload_with_empty_diag() -> Vec<u8> {
        let mut payload = vec![0; COMPACT_DOMAIN_PERF_BASE_LEN];
        payload.extend_from_slice(COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
        payload.extend_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }

    fn append_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) {
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&record_flags.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn record_offsets(raw: &[u8]) -> Vec<usize> {
        let mut offsets = Vec::new();
        let mut offset = 0usize;
        while offset < raw.len() {
            offsets.push(offset);
            let header = parse_compact_record_header(&raw[offset..]).expect("test stream header");
            offset += header.record_len();
        }
        offsets
    }

    fn replace_record_payload(raw: &mut Vec<u8>, record_offset: usize, payload: &[u8]) {
        let header =
            parse_compact_record_header(&raw[record_offset..]).expect("test stream header");
        let payload_start = record_offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + header.payload_len;
        raw.splice(payload_start..payload_end, payload.iter().copied());
        raw[record_offset + 24..record_offset + 28]
            .copy_from_slice(&(payload.len() as u32).to_le_bytes());
    }
}
