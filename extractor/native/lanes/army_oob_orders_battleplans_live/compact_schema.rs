//! Lane-local compact schema and gate for army orders plus land OOB command rows.
//!
//! This mirrors the currently emitted compact domains without importing the
//! shared recorder host. It is intended for isolated review before any shared
//! integration patch.

use std::collections::{BTreeMap, BTreeSet};

#[path = "../../compact_contract/src/record_header.rs"]
mod compact_record_header;
#[path = "../../compact_contract/src/record_semantics.rs"]
mod compact_record_semantics;
#[allow(dead_code)]
#[path = "../../compact_contract/src/status_flags_base.rs"]
mod compact_status_flags;

use compact_record_header::{parse_compact_record_header, RecordFramingError};
pub use compact_record_header::{
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC, COMPACT_RECORD_VERSION,
};
use compact_record_semantics::validate_compact_record_semantics;
#[cfg(test)]
use compact_record_semantics::{
    COMPACT_FRAME_END_PAYLOAD_LEN as FRAME_END_PAYLOAD_LEN,
    COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER as PERF_DIAGNOSTIC_TRAILER_FLAG,
};
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;

pub const DOMAIN_ARMY_ORDERS: u16 = 27;
pub const DOMAIN_OOB_COMMAND: u16 = 28;
pub const DOMAIN_FRAME: u16 = 1000;

use compact_status_flags::{
    COMPACT_STATUS_BRIDGE_UNAVAILABLE as STATUS_BRIDGE_UNAVAILABLE,
    COMPACT_STATUS_CONTROL_INVALID as STATUS_CONTROL_INVALID,
    COMPACT_STATUS_NO_CLOCK as STATUS_NO_CLOCK,
    COMPACT_STATUS_NO_SCALAR_SOURCE as STATUS_NO_SCALAR_SOURCE,
    COMPACT_STATUS_PARTIAL_DATA as STATUS_PARTIAL_DATA,
    COMPACT_STATUS_PROFILE_MISMATCH as STATUS_PROFILE_MISMATCH,
    COMPACT_STATUS_SOURCE_UNAVAILABLE as STATUS_SOURCE_UNAVAILABLE,
    COMPACT_STATUS_STATE_UNAVAILABLE as STATUS_STATE_UNAVAILABLE,
    COMPACT_STATUS_WORKER_PANIC as STATUS_WORKER_PANIC,
};
const FAIL_STATUS_MASK: u32 = STATUS_STATE_UNAVAILABLE
    | STATUS_PROFILE_MISMATCH
    | STATUS_NO_CLOCK
    | STATUS_NO_SCALAR_SOURCE
    | STATUS_WORKER_PANIC
    | STATUS_BRIDGE_UNAVAILABLE
    | STATUS_SOURCE_UNAVAILABLE
    | STATUS_PARTIAL_DATA
    | STATUS_CONTROL_INVALID;

const fn lane_local_status_mask(domain_id: u16) -> u32 {
    match domain_id {
        49 => 1 << 11,
        51 => compact_status_flags::DOMAIN51_LANE_LOCAL_STATUS_MASK,
        _ => 0,
    }
}

const fn status_for_global_interpretation(domain_id: u16, status_flags: u32) -> u32 {
    status_flags & !lane_local_status_mask(domain_id)
}

const ARMY_ORDER_ROW_LEN: usize = 48;
const ARMY_ORDER_TARGET_ROW_LEN: usize = 8;
const ARMY_ORDER_FLAG_DELETED: u32 = 1;
const ARMY_ORDER_FLAG_TARGET_STATES: u32 = 1 << 1;
const ARMY_ORDER_FLAG_TARGET_STATE_COUNT: u32 = 1 << 2;
const ARMY_ORDER_OWNER_KIND_ARMY: u32 = 0;
const ARMY_ORDER_OWNER_KIND_ARMY_GROUP: u32 = 1;
const MAX_ARMY_ID: u32 = i32::MAX as u32;
const MAX_ARMY_ORDER_SLOT_INDEX: u32 = 64;
/// The state id space's STRUCTURAL width (lane IA). Was `65_535`, a guess.
/// Proven from `ai_theatre_graph.cpp:0x86` (`claim_39c989ce13504504353c`);
/// dossier `extractor/native/runs/ia_province_caps/SWEEP.md`. A separate
/// constant from the province one even though the numbers coincide.
const MAX_STATE_ID: u32 = (i32::MAX as u32) - 1;

const OOB_ROW_LEN: usize = 80;
const OOB_MEMBER_ROW_LEN: usize = 8;
const OOB_KIND_ARMY: u32 = 1;
const OOB_KIND_ARMY_GROUP: u32 = 2;
const OOB_KIND_FLEET: u32 = 3;
const OOB_KIND_TASK_FORCE: u32 = 4;
const OOB_FLAG_DELETED: u32 = 1;
const OOB_FLAG_PARENT: u32 = 1 << 1;
const OOB_FLAG_OWNER: u32 = 1 << 2;
const OOB_FLAG_LEADER: u32 = 1 << 3;
const OOB_FLAG_LEADER_SKILL: u32 = 1 << 4;
const OOB_FLAG_MEMBER_COUNT_ENGINE: u32 = 1 << 5;
const OOB_FLAG_MISSING_MEMBER_COUNT: u32 = 1 << 6;
const OOB_FLAG_LOCATION: u32 = 1 << 7;
const MAX_OOB_ID: u32 = 4_000_000;
const MAX_OOB_MEMBER_ROWS: usize = 250_000;
/// Province id space STRUCTURAL width (lane IA, `claim_347234c20a542ca79032`).
/// Was `1_000_000`, a guess; dossier `extractor/native/runs/ia_province_caps/SWEEP.md`.
const MAX_OOB_LOCATION_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
const MAX_OOB_LEADER_SKILL: u32 = 15;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainTotals {
    pub data_records: u64,
    pub rows: u64,
    pub changed_rows: u64,
    pub child_rows: u64,
    pub deleted_rows: u64,
    pub status_flags_or: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompactGateSummary {
    pub record_count: u64,
    pub frame_end_count: u64,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    pub unique_frame_hours: BTreeSet<i32>,
    pub per_hour_data_records: BTreeMap<i32, BTreeMap<u16, u32>>,
    pub per_hour_frame_records: BTreeMap<i32, u32>,
    pub per_domain: BTreeMap<u16, DomainTotals>,
    pub status_flags_or: u32,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateResult {
    pub summary: CompactGateSummary,
    pub errors: Vec<String>,
}

pub fn gate_lane_stream(bytes: &[u8], min_game_hours: i32) -> GateResult {
    let summary = decode_compact_stream(bytes);
    let mut errors = Vec::new();

    if summary.parse_errors.is_empty() {
        for hour in &summary.unique_frame_hours {
            if summary
                .per_hour_frame_records
                .get(hour)
                .copied()
                .unwrap_or(0)
                != 1
            {
                errors.push(format!(
                    "game_hour_raw {hour}: expected one frame-end record"
                ));
            }
            for domain in [DOMAIN_ARMY_ORDERS, DOMAIN_OOB_COMMAND] {
                let count = summary
                    .per_hour_data_records
                    .get(hour)
                    .and_then(|domains| domains.get(&domain))
                    .copied()
                    .unwrap_or(0);
                if count != 1 {
                    errors.push(format!(
                        "game_hour_raw {hour}: expected one data record for domain {domain}, got {count}"
                    ));
                }
            }
        }
    }

    if let (Some(first), Some(last)) = (summary.first_hour, summary.last_hour) {
        let advanced = last.saturating_sub(first);
        if advanced < min_game_hours {
            errors.push(format!(
                "advanced {advanced} game hours, expected at least {min_game_hours}"
            ));
        }
        let expected_hours = advanced.saturating_add(1) as usize;
        if summary.unique_frame_hours.len() != expected_hours {
            errors.push(format!(
                "frame-hour coverage has {} unique hours, expected {expected_hours}",
                summary.unique_frame_hours.len()
            ));
        }
    } else {
        errors.push("stream has no frame-end hours".to_string());
    }

    if summary.status_flags_or & FAIL_STATUS_MASK != 0 {
        errors.push(format!(
            "failure status flags present: 0x{:X}",
            summary.status_flags_or & FAIL_STATUS_MASK
        ));
    }
    errors.extend(summary.parse_errors.clone());
    errors.extend(summary.ordering_errors.clone());

    GateResult { summary, errors }
}

#[cfg(not(test))]
fn main() {
    let mut args = std::env::args().skip(1);
    let Some(stream_path) = args.next() else {
        eprintln!("usage: compact_schema <collector_stream.bin> [min_game_hours]");
        std::process::exit(2);
    };
    let min_game_hours = args
        .next()
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(5);
    let bytes = match std::fs::read(&stream_path) {
        Ok(bytes) => bytes,
        Err(exc) => {
            eprintln!("read failed for {stream_path}: {exc}");
            std::process::exit(2);
        }
    };
    let result = gate_lane_stream(&bytes, min_game_hours);
    let army = result
        .summary
        .per_domain
        .get(&DOMAIN_ARMY_ORDERS)
        .cloned()
        .unwrap_or_default();
    let oob = result
        .summary
        .per_domain
        .get(&DOMAIN_OOB_COMMAND)
        .cloned()
        .unwrap_or_default();
    println!(
        "status={} records={} frames={} hours={:?}..{:?} army_rows={} oob_rows={} child_rows={} status_flags=0x{:X}",
        if result.errors.is_empty() { "PASS" } else { "FAIL" },
        result.summary.record_count,
        result.summary.frame_end_count,
        result.summary.first_hour,
        result.summary.last_hour,
        army.rows,
        oob.rows,
        army.child_rows.saturating_add(oob.child_rows),
        result.summary.status_flags_or
    );
    if !result.errors.is_empty() {
        for error in &result.errors {
            eprintln!("{error}");
        }
        std::process::exit(1);
    }
}

pub fn decode_compact_stream(bytes: &[u8]) -> CompactGateSummary {
    let mut summary = CompactGateSummary::default();
    if let Err(error) = compact_contract::stream_validator::validate_freeze_census_stream_placement(bytes) {
        summary.parse_errors.push(format!("stateful stream placement: {error}"));
    }
    let mut offset = 0usize;
    let mut last_sequence = None::<u64>;
    let mut open_hour = None::<i32>;
    let mut pending_perf_hour = None::<i32>;
    let mut last_closed_hour = None::<i32>;
    let mut saw_frame_end = false;
    let mut last_record_was_clean_frame_end = false;
    let mut release_epilogue_seen = false;

    while offset < bytes.len() {
        let header = match parse_compact_record_header(&bytes[offset..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(_)) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: truncated compact record header"));
                break;
            }
            Err(RecordFramingError::BadMagic) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: bad compact record magic"));
                break;
            }
            Err(RecordFramingError::UnsupportedVersion(version)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: unsupported compact version {version}"
                ));
                break;
            }
            Err(RecordFramingError::UnsupportedHeaderLen(header_len)) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: bad header length {header_len}"));
                break;
            }
            Err(RecordFramingError::PayloadTooLarge(_)) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: payload length exceeds stream"));
                break;
            }
        };
        let sequence = header.sequence;
        let game_hour_raw = header.game_hour_raw;
        let domain_id = header.domain_id;
        let record_type = header.record_type;
        let record_flags = header.record_flags;
        let payload_len = header.payload_len;
        let record_len = header.record_len();

        match last_sequence {
            None if sequence != 1 => summary.ordering_errors.push(format!(
                "offset {offset}: first sequence is {sequence}, expected 1"
            )),
            Some(last) => match last.checked_add(1) {
                Some(expected) if sequence != expected => summary.ordering_errors.push(format!(
                    "offset {offset}: sequence discontinuity: expected {expected}, got {sequence}"
                )),
                None => summary.ordering_errors.push(format!(
                    "offset {offset}: sequence cannot advance after {last} without overflow"
                )),
                _ => {}
            },
            _ => {}
        }
        last_sequence = Some(sequence);
        if bytes.len().saturating_sub(offset) < record_len {
            summary
                .parse_errors
                .push(format!("offset {offset}: payload length exceeds stream"));
            break;
        }

        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload = &bytes[payload_start..payload_start + payload_len];
        let semantics_valid = match validate_compact_record_semantics(
            domain_id,
            record_type,
            record_flags,
            payload,
        ) {
            Ok(()) => true,
            Err(error) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: {error}"));
                false
            }
        };
        summary.record_count = summary.record_count.saturating_add(1);
        let previous_record_closed_world = last_record_was_clean_frame_end;
        last_record_was_clean_frame_end = false;
        let known_record_type = match compact_contract::KnownRecordType::try_from(record_type) {
            Ok(record_type) => record_type,
            Err(unknown) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: unsupported record type {}",
                    unknown.0
                ));
                offset += record_len;
                continue;
            }
        };

        match known_record_type {
            compact_contract::KnownRecordType::FrameEnd if semantics_valid => {
                let open_matches = match open_hour {
                    Some(open) if open == game_hour_raw => true,
                    Some(open) => {
                        summary.parse_errors.push(format!(
                            "offset {offset}: frame-end hour {game_hour_raw} does not close open hour {open}"
                        ));
                        false
                    }
                    None => {
                        summary.parse_errors.push(format!(
                            "offset {offset}: frame-end hour {game_hour_raw} has no open data hour"
                        ));
                        false
                    }
                };
                if let Some(pending) = pending_perf_hour {
                    summary.parse_errors.push(format!(
                        "offset {offset}: frame-end hour {game_hour_raw} encountered with pending perf-only hour {pending}"
                    ));
                }
                let successor_matches = closed_hour_is_successor(
                    offset,
                    last_closed_hour,
                    game_hour_raw,
                    &mut summary.ordering_errors,
                );
                if open_matches && pending_perf_hour.is_none() {
                    open_hour = None;
                    if successor_matches {
                        saw_frame_end = true;
                        last_closed_hour = Some(game_hour_raw);
                        last_record_was_clean_frame_end = true;
                        summary.frame_end_count = summary.frame_end_count.saturating_add(1);
                        summary.first_hour = Some(summary.first_hour.unwrap_or(game_hour_raw));
                        summary.last_hour = Some(game_hour_raw);
                        summary.unique_frame_hours.insert(game_hour_raw);
                        *summary
                            .per_hour_frame_records
                            .entry(game_hour_raw)
                            .or_default() += 1;
                        summary.status_flags_or |=
                            status_for_global_interpretation(DOMAIN_FRAME, read_u32(payload, 24));
                    }
                }
            }
            compact_contract::KnownRecordType::DomainData if semantics_valid => {
                match open_hour {
                    Some(open) if open != game_hour_raw => summary.parse_errors.push(format!(
                        "offset {offset}: data hour {game_hour_raw} encountered before frame-end closed hour {open}"
                    )),
                    None => match pending_perf_hour {
                        Some(pending) if pending != game_hour_raw => summary.parse_errors.push(
                            format!(
                                "offset {offset}: data hour {game_hour_raw} does not match pending perf hour {pending}"
                            ),
                        ),
                        _ => {
                            pending_perf_hour = None;
                            open_hour = Some(game_hour_raw);
                        }
                    },
                    _ => {}
                }
                *summary
                    .per_hour_data_records
                    .entry(game_hour_raw)
                    .or_default()
                    .entry(domain_id)
                    .or_default() += 1;
                match domain_id {
                    DOMAIN_ARMY_ORDERS => decode_army_orders_payload(offset, payload, &mut summary),
                    DOMAIN_OOB_COMMAND => decode_oob_command_payload(offset, payload, &mut summary),
                    _ => {}
                }
            }
            compact_contract::KnownRecordType::DomainPerf if semantics_valid => {
                match (open_hour, pending_perf_hour) {
                    (Some(open), _) if open == game_hour_raw => {}
                    (Some(open), _) => summary.parse_errors.push(format!(
                        "offset {offset}: domain perf hour {game_hour_raw} does not match open hour {open}"
                    )),
                    (None, Some(pending)) if pending == game_hour_raw => {}
                    (None, Some(pending)) => summary.parse_errors.push(format!(
                        "offset {offset}: domain perf hour {game_hour_raw} does not match pending perf hour {pending}"
                    )),
                    (None, None) => pending_perf_hour = Some(game_hour_raw),
                }
                if domain_id == DOMAIN_ARMY_ORDERS || domain_id == DOMAIN_OOB_COMMAND {
                    summary.status_flags_or |=
                        status_for_global_interpretation(domain_id, read_u32(payload, 32));
                }
            }
            compact_contract::KnownRecordType::FrameEnd
            | compact_contract::KnownRecordType::DomainData
            | compact_contract::KnownRecordType::DomainPerf => {}
            compact_contract::KnownRecordType::FreezeCensus => {
                if let Ok(census) = compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload) {
                    last_record_was_clean_frame_end = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(previous_record_closed_world, census.phase(), release_epilogue_seen);
                    release_epilogue_seen |= census.phase() == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                }
            }
            compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StreamHeader => {}
            compact_contract::KnownRecordType::DomainGap
            | compact_contract::KnownRecordType::FrameRefused
            | compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => {
                summary.parse_errors.push(format!(
                    "offset {offset}: unsupported record type {}",
                    known_record_type.canonical_name()
                ))
            }
        }

        offset += record_len;
    }

    if !bytes.is_empty() {
        if !saw_frame_end {
            summary
                .parse_errors
                .push("stream has no frame-end record".to_string());
        }
        if let Some(open) = open_hour {
            summary
                .parse_errors
                .push(format!("stream ended with open data hour {open}"));
        }
        if !last_record_was_clean_frame_end {
            summary
                .parse_errors
                .push("stream does not end at a clean frame-end boundary".to_string());
        }
    }

    summary
}

fn closed_hour_is_successor(
    offset: usize,
    previous: Option<i32>,
    current: i32,
    errors: &mut Vec<String>,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    match previous.checked_add(1) {
        Some(expected) if current == expected => true,
        Some(expected) => {
            errors.push(format!(
                "offset {offset}: closed hour discontinuity: expected {expected}, got {current}"
            ));
            false
        }
        None => {
            errors.push(format!(
                "offset {offset}: closed hour cannot advance after {previous} without overflow"
            ));
            false
        }
    }
}

pub fn decode_army_orders_payload(offset: usize, payload: &[u8], summary: &mut CompactGateSummary) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: army_orders payload shorter than 40 bytes"
        ));
        return;
    }
    let row_count = read_u32(payload, 8) as usize;
    let changed_rows = read_u32(payload, 12) as usize;
    let status_flags = read_u32(payload, 16);
    let root_rows = read_u32(payload, 20) as usize;
    let observed_army_rows = read_u32(payload, 24) as usize;
    let observed_order_rows = read_u32(payload, 28) as usize;
    let target_state_count = read_u32(payload, 32) as usize;
    let deleted_rows = read_u32(payload, 36) as usize;
    let expected_len = 40usize
        .saturating_add(row_count.saturating_mul(ARMY_ORDER_ROW_LEN))
        .saturating_add(target_state_count.saturating_mul(ARMY_ORDER_TARGET_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: army_orders payload length {} != {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: army_orders changed rows exceed row count"
        ));
    }
    if observed_army_rows > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: observed army rows exceed root rows"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: army_orders deleted rows exceed row count"
        ));
    }

    let domain = summary.per_domain.entry(DOMAIN_ARMY_ORDERS).or_default();
    domain.data_records = domain.data_records.saturating_add(1);
    domain.rows = domain.rows.saturating_add(row_count as u64);
    domain.changed_rows = domain.changed_rows.saturating_add(changed_rows as u64);
    domain.child_rows = domain.child_rows.saturating_add(target_state_count as u64);
    domain.deleted_rows = domain.deleted_rows.saturating_add(deleted_rows as u64);
    let status_flags = status_for_global_interpretation(DOMAIN_ARMY_ORDERS, status_flags);
    domain.status_flags_or |= status_flags;
    summary.status_flags_or |= status_flags;

    let mut cursor = 40usize;
    let mut declared_targets = 0usize;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let army_id = read_u32(payload, cursor);
        let generation = read_u64(payload, cursor + 4);
        let flags = read_u32(payload, cursor + 12);
        let slot_index = read_u32(payload, cursor + 16);
        let order_type = read_i32(payload, cursor + 20);
        let target_first = read_u32(payload, cursor + 24) as usize;
        let target_count = read_u32(payload, cursor + 28) as usize;
        let target_declared = read_u32(payload, cursor + 32);
        let target_status = read_u32(payload, cursor + 36);
        let owner_kind = read_u32(payload, cursor + 40);
        let reserved1 = read_u32(payload, cursor + 44);
        cursor += ARMY_ORDER_ROW_LEN;

        let deleted = flags & ARMY_ORDER_FLAG_DELETED != 0;
        let has_target_states = flags & ARMY_ORDER_FLAG_TARGET_STATES != 0;
        let has_target_count = flags & ARMY_ORDER_FLAG_TARGET_STATE_COUNT != 0;
        if army_id == 0 || army_id > MAX_ARMY_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has bad army id"
            ));
        }
        if !matches!(
            owner_kind,
            ARMY_ORDER_OWNER_KIND_ARMY | ARMY_ORDER_OWNER_KIND_ARMY_GROUP
        ) {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has bad owner kind"
            ));
        }
        if !seen.insert((owner_kind, army_id, generation, slot_index, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: duplicate army order row {row_index}"
            ));
        }
        if flags
            & !(ARMY_ORDER_FLAG_DELETED
                | ARMY_ORDER_FLAG_TARGET_STATES
                | ARMY_ORDER_FLAG_TARGET_STATE_COUNT)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has unknown flags"
            ));
        }
        if slot_index >= MAX_ARMY_ORDER_SLOT_INDEX {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has bad slot"
            ));
        }
        if !(-1000..=1000).contains(&order_type) {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has bad order type"
            ));
        }
        if deleted
            && (target_first != 0
                || target_count != 0
                || target_declared != 0
                || target_status != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: deleted army order row {row_index} has live target fields"
            ));
        }
        if !deleted && target_status > 4 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has bad target status"
            ));
        }
        if !has_target_states && target_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has target rows without flag"
            ));
        }
        if has_target_states {
            let target_end = target_first.saturating_add(target_count);
            if target_end > target_state_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: army order row {row_index} target range exceeds child table"
                ));
            }
            declared_targets = declared_targets.saturating_add(target_count);
        } else if target_first != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has target_first without flag"
            ));
        }
        if !has_target_count && target_declared != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has target count without flag"
            ));
        }
        if reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has nonzero reserved field"
            ));
        }
    }
    if declared_targets != target_state_count {
        summary.parse_errors.push(format!(
            "offset {offset}: army_orders target child count mismatch"
        ));
    }
    for row_index in 0..target_state_count {
        let state_id = read_u32(payload, cursor);
        let reserved = read_u32(payload, cursor + 4);
        cursor += ARMY_ORDER_TARGET_ROW_LEN;
        if state_id == 0 || state_id > MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: target row {row_index} has bad state id"
            ));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: target row {row_index} has nonzero reserved field"
            ));
        }
    }
    let _ = observed_order_rows;
}

pub fn decode_oob_command_payload(offset: usize, payload: &[u8], summary: &mut CompactGateSummary) {
    if payload.len() < 48 {
        summary.parse_errors.push(format!(
            "offset {offset}: oob_command payload shorter than 48 bytes"
        ));
        return;
    }
    let row_count = read_u32(payload, 8) as usize;
    let changed_rows = read_u32(payload, 12) as usize;
    let status_flags = read_u32(payload, 16);
    let root_rows = read_u32(payload, 20) as usize;
    let observed_army_rows = read_u32(payload, 24) as usize;
    let member_count = read_u32(payload, 32) as usize;
    let deleted_rows = read_u32(payload, 36) as usize;
    if member_count > MAX_OOB_MEMBER_ROWS {
        summary
            .parse_errors
            .push(format!("offset {offset}: oob member count exceeds limit"));
    }
    let expected_len = 48usize
        .saturating_add(row_count.saturating_mul(OOB_ROW_LEN))
        .saturating_add(member_count.saturating_mul(OOB_MEMBER_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: oob_command payload length {} != {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob changed rows exceed row count"
        ));
    }
    if observed_army_rows > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: oob observed army rows exceed root rows"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob deleted rows exceed row count"
        ));
    }

    let domain = summary.per_domain.entry(DOMAIN_OOB_COMMAND).or_default();
    domain.data_records = domain.data_records.saturating_add(1);
    domain.rows = domain.rows.saturating_add(row_count as u64);
    domain.changed_rows = domain.changed_rows.saturating_add(changed_rows as u64);
    domain.child_rows = domain.child_rows.saturating_add(member_count as u64);
    domain.deleted_rows = domain.deleted_rows.saturating_add(deleted_rows as u64);
    let status_flags = status_for_global_interpretation(DOMAIN_OOB_COMMAND, status_flags);
    domain.status_flags_or |= status_flags;
    summary.status_flags_or |= status_flags;

    let mut cursor = 48usize;
    let mut declared_members = 0usize;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let kind = read_u32(payload, cursor);
        let object_id = read_u32(payload, cursor + 4);
        let flags = read_u32(payload, cursor + 8);
        let parent_id = read_u32(payload, cursor + 12);
        let owner_tag_db_id = read_u32(payload, cursor + 16);
        let leader_id = read_u32(payload, cursor + 20);
        let leader_skill = read_u32(payload, cursor + 24);
        let member_first = read_u32(payload, cursor + 28) as usize;
        let row_member_count = read_u32(payload, cursor + 32) as usize;
        let logical_member_count = read_u32(payload, cursor + 36) as usize;
        let member_count_engine = read_u32(payload, cursor + 40);
        let missing_member_count = read_u32(payload, cursor + 44);
        let leader_trait_count = read_u32(payload, cursor + 48);
        let parent_status = read_u32(payload, cursor + 52);
        let member_status = read_u32(payload, cursor + 56);
        let owner_status = read_u32(payload, cursor + 60);
        let leader_status = read_u32(payload, cursor + 64);
        let leader_trait_status = read_u32(payload, cursor + 68);
        let location_province_id = read_u32(payload, cursor + 72);
        let reserved = read_u32(payload, cursor + 76);
        cursor += OOB_ROW_LEN;

        let deleted = flags & OOB_FLAG_DELETED != 0;
        if !matches!(
            kind,
            OOB_KIND_ARMY | OOB_KIND_ARMY_GROUP | OOB_KIND_FLEET | OOB_KIND_TASK_FORCE
        ) {
            summary
                .parse_errors
                .push(format!("offset {offset}: oob row {row_index} has bad kind"));
        }
        if object_id == 0 || object_id > MAX_OOB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: oob row {row_index} has bad object id"
            ));
        }
        if !seen.insert((kind, object_id, deleted)) {
            summary
                .parse_errors
                .push(format!("offset {offset}: duplicate oob row {row_index}"));
        }
        let known_flags = OOB_FLAG_DELETED
            | OOB_FLAG_PARENT
            | OOB_FLAG_OWNER
            | OOB_FLAG_LEADER
            | OOB_FLAG_LEADER_SKILL
            | OOB_FLAG_MEMBER_COUNT_ENGINE
            | OOB_FLAG_MISSING_MEMBER_COUNT
            | OOB_FLAG_LOCATION;
        if flags & !known_flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob row {row_index} has unknown flags"
            ));
        }
        if deleted
            && (parent_id != 0
                || owner_tag_db_id != 0
                || leader_id != 0
                || leader_skill != 0
                || row_member_count != 0
                || logical_member_count != 0
                || member_count_engine != 0
                || missing_member_count != 0
                || leader_trait_count != 0
                || location_province_id != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: deleted oob row {row_index} has live fields"
            ));
        }
        validate_optional_id(
            summary,
            offset,
            row_index,
            flags & OOB_FLAG_PARENT != 0,
            parent_id,
            "parent",
        );
        validate_optional_id(
            summary,
            offset,
            row_index,
            flags & OOB_FLAG_OWNER != 0,
            owner_tag_db_id,
            "owner",
        );
        validate_optional_id(
            summary,
            offset,
            row_index,
            flags & OOB_FLAG_LEADER != 0,
            leader_id,
            "leader",
        );
        if flags & OOB_FLAG_LEADER_SKILL != 0 {
            if leader_skill > MAX_OOB_LEADER_SKILL {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob row {row_index} has bad leader skill"
                ));
            }
        } else if leader_skill != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob row {row_index} has leader skill without flag"
            ));
        }
        if flags & OOB_FLAG_LOCATION != 0 {
            if location_province_id == 0 || location_province_id > MAX_OOB_LOCATION_PROVINCE_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob row {row_index} has bad location"
                ));
            }
        } else if location_province_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob row {row_index} has location without flag"
            ));
        }
        if flags & OOB_FLAG_MEMBER_COUNT_ENGINE == 0 && member_count_engine != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob row {row_index} has engine count without flag"
            ));
        }
        if flags & OOB_FLAG_MISSING_MEMBER_COUNT == 0 && missing_member_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob row {row_index} has missing count without flag"
            ));
        }
        for (name, status) in [
            ("parent", parent_status),
            ("member", member_status),
            ("owner", owner_status),
            ("leader", leader_status),
            ("leader_trait", leader_trait_status),
        ] {
            if status > 5 {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob row {row_index} has bad {name} status"
                ));
            }
        }
        if row_member_count > 0 {
            let member_end = member_first.saturating_add(row_member_count);
            if member_end > member_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob row {row_index} member range exceeds child table"
                ));
            }
            declared_members = declared_members.saturating_add(row_member_count);
        } else if member_first != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob row {row_index} has member_first without members"
            ));
        }
        if logical_member_count < row_member_count {
            summary.parse_errors.push(format!(
                "offset {offset}: oob row {row_index} logical member count is too small"
            ));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob row {row_index} has nonzero reserved field"
            ));
        }
    }
    if declared_members != member_count {
        summary
            .parse_errors
            .push(format!("offset {offset}: oob member child count mismatch"));
    }
    for row_index in 0..member_count {
        let member_id = read_u32(payload, cursor);
        let reserved = read_u32(payload, cursor + 4);
        cursor += OOB_MEMBER_ROW_LEN;
        if member_id == 0 || member_id > MAX_OOB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: oob member row {row_index} has bad id"
            ));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob member row {row_index} has nonzero reserved field"
            ));
        }
    }
}

fn validate_optional_id(
    summary: &mut CompactGateSummary,
    offset: usize,
    row_index: usize,
    present: bool,
    value: u32,
    field: &str,
) {
    if present {
        if value == 0 || value > MAX_OOB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: oob row {row_index} has bad {field} id"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: oob row {row_index} has {field} id without flag"
        ));
    }
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_i32(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn army_orders_payload_accepts_exact_target_and_delete() {
        let payload = test_army_orders_payload(0);
        let mut summary = CompactGateSummary::default();
        decode_army_orders_payload(0, &payload, &mut summary);

        assert_eq!(summary.parse_errors, Vec::<String>::new());
        let domain = summary.per_domain.get(&DOMAIN_ARMY_ORDERS).unwrap();
        assert_eq!(domain.rows, 2);
        assert_eq!(domain.child_rows, 2);
        assert_eq!(domain.deleted_rows, 1);
    }

    #[test]
    fn army_orders_payload_accepts_typed_army_group_and_rejects_unknown_kind() {
        let mut payload = test_army_orders_payload(0);
        payload[40 + 40..40 + 44].copy_from_slice(&ARMY_ORDER_OWNER_KIND_ARMY_GROUP.to_le_bytes());
        let mut summary = CompactGateSummary::default();
        decode_army_orders_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );

        payload[40 + 40..40 + 44].copy_from_slice(&2u32.to_le_bytes());
        let mut invalid = CompactGateSummary::default();
        decode_army_orders_payload(0, &payload, &mut invalid);
        assert!(invalid
            .parse_errors
            .iter()
            .any(|error| error.contains("bad owner kind")));
    }

    #[test]
    fn army_orders_payload_rejects_bad_target_reserved_field() {
        let mut payload = test_army_orders_payload(0);
        let target_reserved_offset = 40 + 2 * ARMY_ORDER_ROW_LEN + 4;
        payload[target_reserved_offset..target_reserved_offset + 4]
            .copy_from_slice(&1u32.to_le_bytes());

        let mut summary = CompactGateSummary::default();
        decode_army_orders_payload(77, &payload, &mut summary);

        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("target row 0 has nonzero reserved field")));
    }

    #[test]
    fn oob_payload_accepts_land_command_rows() {
        let payload = test_oob_payload(0);
        let mut summary = CompactGateSummary::default();
        decode_oob_command_payload(0, &payload, &mut summary);

        assert_eq!(summary.parse_errors, Vec::<String>::new());
        let domain = summary.per_domain.get(&DOMAIN_OOB_COMMAND).unwrap();
        assert_eq!(domain.rows, 2);
        assert_eq!(domain.child_rows, 3);
    }

    #[test]
    fn stream_gate_accepts_hourly_coverage() {
        let mut raw = Vec::new();
        let mut sequence = 1u64;
        for hour in [100i32, 101, 102] {
            append_record(
                &mut raw,
                sequence,
                hour,
                DOMAIN_ARMY_ORDERS,
                RECORD_TYPE_DOMAIN_DATA,
                &test_army_orders_payload(0),
            );
            sequence += 1;
            append_record(
                &mut raw,
                sequence,
                hour,
                DOMAIN_OOB_COMMAND,
                RECORD_TYPE_DOMAIN_DATA,
                &test_oob_payload(0),
            );
            sequence += 1;
            append_record(
                &mut raw,
                sequence,
                hour,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &[0; FRAME_END_PAYLOAD_LEN],
            );
            sequence += 1;
        }

        let result = gate_lane_stream(&raw, 2);

        assert_eq!(result.errors, Vec::<String>::new());
        assert_eq!(result.summary.frame_end_count, 3);
    }

    #[test]
    fn stream_gate_rejects_source_unavailable() {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            DOMAIN_ARMY_ORDERS,
            RECORD_TYPE_DOMAIN_DATA,
            &test_army_orders_payload(STATUS_SOURCE_UNAVAILABLE),
        );
        append_record(
            &mut raw,
            2,
            100,
            DOMAIN_OOB_COMMAND,
            RECORD_TYPE_DOMAIN_DATA,
            &test_oob_payload(0),
        );
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );

        let result = gate_lane_stream(&raw, 0);

        assert!(result
            .errors
            .iter()
            .any(|err| err.contains("failure status flags")));
    }

    #[test]
    fn stream_gate_accepts_unowned_records_and_perf_before_data() {
        let mut raw = Vec::new();
        append_record(&mut raw, 1, 100, 51, RECORD_TYPE_DOMAIN_PERF, &[0; 40]);
        append_record(&mut raw, 2, 100, 51, RECORD_TYPE_DOMAIN_DATA, &[0xA5; 7]);
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_ARMY_ORDERS,
            RECORD_TYPE_DOMAIN_PERF,
            &[0; 40],
        );
        append_record(
            &mut raw,
            4,
            100,
            DOMAIN_ARMY_ORDERS,
            RECORD_TYPE_DOMAIN_DATA,
            &test_army_orders_payload(0),
        );
        append_record(
            &mut raw,
            5,
            100,
            DOMAIN_OOB_COMMAND,
            RECORD_TYPE_DOMAIN_DATA,
            &test_oob_payload(0),
        );
        append_record(
            &mut raw,
            6,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );

        let result = gate_lane_stream(&raw, 0);
        assert_eq!(result.errors, Vec::<String>::new(), "{:?}", result.errors);
        assert!(!result.summary.per_domain.contains_key(&51));
    }

    #[test]
    fn stream_gate_rejects_nonconsecutive_physical_closures() {
        let mut raw = Vec::new();
        let mut sequence = 1u64;
        for hour in [100i32, 99, 101] {
            append_record(
                &mut raw,
                sequence,
                hour,
                DOMAIN_ARMY_ORDERS,
                RECORD_TYPE_DOMAIN_DATA,
                &test_army_orders_payload(0),
            );
            sequence += 1;
            append_record(
                &mut raw,
                sequence,
                hour,
                DOMAIN_OOB_COMMAND,
                RECORD_TYPE_DOMAIN_DATA,
                &test_oob_payload(0),
            );
            sequence += 1;
            append_record(
                &mut raw,
                sequence,
                hour,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &[0; FRAME_END_PAYLOAD_LEN],
            );
            sequence += 1;
        }

        let result = gate_lane_stream(&raw, 0);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("closed hour discontinuity: expected 101, got 99")));
    }

    #[test]
    fn stream_gate_uses_full_canonical_failure_mask_after_normalization() {
        let fatal = (1 << 0)
            | (1 << 1)
            | (1 << 2)
            | (1 << 3)
            | (1 << 6)
            | (1 << 7)
            | (1 << 8)
            | (1 << 9)
            | (1 << 12);
        assert_eq!(fatal, FAIL_STATUS_MASK);
        assert_eq!(status_for_global_interpretation(51, 1 << 12), 0);
        assert_eq!(
            status_for_global_interpretation(DOMAIN_ARMY_ORDERS, 1 << 12),
            1 << 12
        );

        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            DOMAIN_ARMY_ORDERS,
            RECORD_TYPE_DOMAIN_DATA,
            &test_army_orders_payload(fatal),
        );
        append_record(
            &mut raw,
            2,
            100,
            DOMAIN_OOB_COMMAND,
            RECORD_TYPE_DOMAIN_DATA,
            &test_oob_payload(0),
        );
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );
        let result = gate_lane_stream(&raw, 0);
        assert_eq!(result.summary.status_flags_or & FAIL_STATUS_MASK, fatal);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("failure status flags")));
    }

    #[test]
    fn stream_gate_rejects_sequence_gap_and_invalid_flags() {
        let mut raw = Vec::new();
        append_record_with_flags(
            &mut raw,
            1,
            100,
            DOMAIN_ARMY_ORDERS,
            RECORD_TYPE_DOMAIN_DATA,
            1 << 31,
            &test_army_orders_payload(0),
        );
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_OOB_COMMAND,
            RECORD_TYPE_DOMAIN_DATA,
            &test_oob_payload(0),
        );
        append_record(
            &mut raw,
            4,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );

        let summary = decode_compact_stream(&raw);
        assert!(summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("expected 2, got 3")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("record flags")));
    }

    #[test]
    fn stream_gate_rejects_bad_perf_and_frame_payload_lengths() {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            DOMAIN_ARMY_ORDERS,
            RECORD_TYPE_DOMAIN_DATA,
            &test_army_orders_payload(0),
        );
        append_record(
            &mut raw,
            2,
            100,
            DOMAIN_ARMY_ORDERS,
            RECORD_TYPE_DOMAIN_PERF,
            &[0; 39],
        );
        append_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; 31],
        );

        let summary = decode_compact_stream(&raw);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("shorter than 40")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("is not 32")));
    }

    #[test]
    fn stream_gate_validates_announced_perf_trailer_envelopes() {
        fn stream_with_perf(flags: u32, perf: &[u8]) -> CompactGateSummary {
            let mut raw = Vec::new();
            append_record(
                &mut raw,
                1,
                100,
                DOMAIN_ARMY_ORDERS,
                RECORD_TYPE_DOMAIN_DATA,
                &test_army_orders_payload(0),
            );
            append_record_with_flags(
                &mut raw,
                2,
                100,
                DOMAIN_ARMY_ORDERS,
                RECORD_TYPE_DOMAIN_PERF,
                flags,
                perf,
            );
            append_record(
                &mut raw,
                3,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &[0; FRAME_END_PAYLOAD_LEN],
            );
            decode_compact_stream(&raw)
        }

        let base = vec![0; 40];
        let trailer = empty_pdu1_perf_payload();
        assert!(stream_with_perf(PERF_DIAGNOSTIC_TRAILER_FLAG, &trailer)
            .parse_errors
            .is_empty());
        assert!(stream_with_perf(PERF_DIAGNOSTIC_TRAILER_FLAG, &base)
            .parse_errors
            .iter()
            .any(|error| error.contains("trailer header is truncated")));
        assert!(stream_with_perf(0, &trailer)
            .parse_errors
            .iter()
            .any(|error| error.contains("unannounced trailer")));
    }

    #[test]
    fn stream_gate_rejects_hour_mismatch_and_trailing_record() {
        let mut raw = Vec::new();
        append_record(
            &mut raw,
            1,
            100,
            DOMAIN_ARMY_ORDERS,
            RECORD_TYPE_DOMAIN_DATA,
            &test_army_orders_payload(0),
        );
        append_record(
            &mut raw,
            2,
            101,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );
        append_record(
            &mut raw,
            3,
            101,
            DOMAIN_ARMY_ORDERS,
            RECORD_TYPE_DOMAIN_PERF,
            &[0; 40],
        );

        let summary = decode_compact_stream(&raw);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("does not close open hour")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("does not match open hour")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("clean frame-end boundary")));
    }

    fn test_army_orders_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&80u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&20u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());

        for (army_id, generation, flags, slot, order_type, first, count, declared, status) in [
            (
                17u32,
                3u64,
                ARMY_ORDER_FLAG_TARGET_STATES | ARMY_ORDER_FLAG_TARGET_STATE_COUNT,
                2u32,
                5i32,
                0u32,
                2u32,
                2u32,
                1u32,
            ),
            (18, 4, ARMY_ORDER_FLAG_DELETED, 0, 2, 0, 0, 0, 0),
        ] {
            payload.extend_from_slice(&army_id.to_le_bytes());
            payload.extend_from_slice(&generation.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&slot.to_le_bytes());
            payload.extend_from_slice(&order_type.to_le_bytes());
            payload.extend_from_slice(&first.to_le_bytes());
            payload.extend_from_slice(&count.to_le_bytes());
            payload.extend_from_slice(&declared.to_le_bytes());
            payload.extend_from_slice(&status.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        for state_id in [118u32, 119u32] {
            payload.extend_from_slice(&state_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn empty_pdu1_perf_payload() -> Vec<u8> {
        let mut payload = vec![0; 40];
        payload.extend_from_slice(b"PDU1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }

    fn test_oob_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&90u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&20u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        append_oob_row(
            &mut payload,
            OOB_KIND_ARMY,
            17,
            OOB_FLAG_PARENT
                | OOB_FLAG_OWNER
                | OOB_FLAG_LEADER
                | OOB_FLAG_LEADER_SKILL
                | OOB_FLAG_MEMBER_COUNT_ENGINE
                | OOB_FLAG_MISSING_MEMBER_COUNT,
            301,
            47,
            9001,
            4,
            0,
            2,
            2,
            3,
            1,
            0,
            1,
            1,
            1,
            1,
            0,
        );
        append_oob_row(
            &mut payload,
            OOB_KIND_ARMY_GROUP,
            301,
            OOB_FLAG_OWNER,
            0,
            47,
            0,
            0,
            2,
            1,
            1,
            0,
            0,
            0,
            0,
            1,
            1,
            0,
            0,
        );
        for member_id in [101u32, 102, 17] {
            payload.extend_from_slice(&member_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    #[allow(clippy::too_many_arguments)]
    fn append_oob_row(
        out: &mut Vec<u8>,
        kind: u32,
        object_id: u32,
        flags: u32,
        parent_id: u32,
        owner_tag_db_id: u32,
        leader_id: u32,
        leader_skill: u32,
        member_first: u32,
        row_member_count: u32,
        logical_member_count: u32,
        member_count_engine: u32,
        missing_member_count: u32,
        leader_trait_count: u32,
        parent_status: u32,
        member_status: u32,
        owner_status: u32,
        leader_status: u32,
        location_province_id: u32,
    ) {
        for value in [
            kind,
            object_id,
            flags,
            parent_id,
            owner_tag_db_id,
            leader_id,
            leader_skill,
            member_first,
            row_member_count,
            logical_member_count,
            member_count_engine,
            missing_member_count,
            leader_trait_count,
            parent_status,
            member_status,
            owner_status,
            leader_status,
            0,
            location_province_id,
            0,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }

    fn append_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) {
        append_record_with_flags(
            out,
            sequence,
            game_hour_raw,
            domain_id,
            record_type,
            0,
            payload,
        );
    }

    fn append_record_with_flags(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) {
        out.extend_from_slice(COMPACT_RECORD_MAGIC);
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
}
