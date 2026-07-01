//! Lane-local compact encoder/decoder/gate for army_battleplans_depth_live.
//!
//! Registry assignment: domain 54 = army_battleplans_depth_live. This module is
//! intentionally not wired into shared collector/recorder entrypoints.

use std::collections::{BTreeMap, BTreeSet};

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
pub const DOMAIN_ARMY_BATTLEPLANS_DEPTH: u16 = 54;
pub const DOMAIN_FRAME: u16 = 1000;

pub const HEADER_LEN: usize = 40;
pub const ROW_LEN: usize = 64;
pub const TARGET_ROW_LEN: usize = 8;

pub const FLAG_DELETED: u32 = 1;
pub const FLAG_TARGET_STATES: u32 = 1 << 1;
pub const FLAG_TARGET_STATE_COUNT: u32 = 1 << 2;
pub const FLAG_NON_TYPE5_GEOMETRY_BLOCKED: u32 = 1 << 3;
pub const FLAG_EXECUTION_BLOCKED: u32 = 1 << 4;

pub const STATUS_NONE: u32 = 0;
pub const STATUS_EXACT: u32 = 1;
pub const STATUS_UNKNOWN: u32 = 2;
pub const STATUS_UNSUPPORTED: u32 = 3;
pub const STATUS_OVERFLOW: u32 = 4;
pub const STATUS_VALIDATION_FAILED: u32 = 5;
pub const STATUS_FIXTURE_REQUIRED: u32 = 6;

const STATUS_WORKER_PANIC: u32 = 1 << 6;
const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
const STATUS_PARTIAL_DATA: u32 = 1 << 9;
const FAIL_STATUS_MASK: u32 =
    STATUS_WORKER_PANIC | STATUS_BRIDGE_UNAVAILABLE | STATUS_SOURCE_UNAVAILABLE | STATUS_PARTIAL_DATA;

const MAX_ARMY_ID: u32 = i32::MAX as u32;
const MAX_SLOT_INDEX: u32 = 64;
const MAX_STATE_ID: u32 = 65_535;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattleplanDepthRow {
    pub army_id: u32,
    pub generation: u64,
    pub deleted: bool,
    pub slot_index: u32,
    pub order_type: i32,
    pub target_states: Vec<u32>,
    pub target_declared_count: Option<u32>,
    pub target_status: u32,
    pub geometry_status: u32,
    pub execution_status: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainTotals {
    pub data_records: u64,
    pub rows: u64,
    pub changed_rows: u64,
    pub target_rows: u64,
    pub unsupported_geometry_rows: u64,
    pub execution_unknown_rows: u64,
    pub status_flags_or: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GateSummary {
    pub record_count: u64,
    pub frame_end_count: u64,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    pub unique_frame_hours: BTreeSet<i32>,
    pub per_hour_depth_records: BTreeMap<i32, u32>,
    pub per_hour_frame_records: BTreeMap<i32, u32>,
    pub totals: DomainTotals,
    pub status_flags_or: u32,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
}

pub fn encode_depth_payload(
    rows: &[BattleplanDepthRow],
    elapsed_wall_ms: u64,
    status_flags: u32,
    changed_rows: u32,
    observed_army_rows: u32,
    observed_order_rows: u32,
) -> Vec<u8> {
    let target_count = rows.iter().map(|row| row.target_states.len()).sum::<usize>();
    let unsupported_geometry_rows = rows
        .iter()
        .filter(|row| !row.deleted && row.geometry_status == STATUS_FIXTURE_REQUIRED)
        .count()
        .min(u32::MAX as usize) as u32;
    let execution_unknown_rows = rows
        .iter()
        .filter(|row| !row.deleted && row.execution_status == STATUS_FIXTURE_REQUIRED)
        .count()
        .min(u32::MAX as usize) as u32;

    let mut out = Vec::with_capacity(HEADER_LEN + rows.len() * ROW_LEN + target_count * TARGET_ROW_LEN);
    out.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    out.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&changed_rows.to_le_bytes());
    out.extend_from_slice(&status_flags.to_le_bytes());
    out.extend_from_slice(&observed_army_rows.to_le_bytes());
    out.extend_from_slice(&observed_order_rows.to_le_bytes());
    out.extend_from_slice(&(target_count.min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&unsupported_geometry_rows.to_le_bytes());
    out.extend_from_slice(&execution_unknown_rows.to_le_bytes());

    let mut target_cursor = 0u32;
    for row in rows {
        let mut flags = 0u32;
        if row.deleted {
            flags |= FLAG_DELETED;
        }
        if !row.target_states.is_empty() {
            flags |= FLAG_TARGET_STATES;
        }
        if row.target_declared_count.is_some() {
            flags |= FLAG_TARGET_STATE_COUNT;
        }
        if row.geometry_status == STATUS_FIXTURE_REQUIRED {
            flags |= FLAG_NON_TYPE5_GEOMETRY_BLOCKED;
        }
        if row.execution_status == STATUS_FIXTURE_REQUIRED {
            flags |= FLAG_EXECUTION_BLOCKED;
        }
        let target_first = if row.target_states.is_empty() { 0 } else { target_cursor };
        out.extend_from_slice(&row.army_id.to_le_bytes());
        out.extend_from_slice(&row.generation.to_le_bytes());
        for value in [
            flags,
            row.slot_index,
            row.order_type as u32,
            target_first,
            row.target_states.len().min(u32::MAX as usize) as u32,
            row.target_declared_count.unwrap_or(0),
            row.target_status,
            row.geometry_status,
            row.execution_status,
            0,
            0,
            0,
            0,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        target_cursor = target_cursor.saturating_add(row.target_states.len().min(u32::MAX as usize) as u32);
    }
    for row in rows {
        for state_id in &row.target_states {
            out.extend_from_slice(&state_id.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
    }
    out
}

pub fn decode_depth_payload(offset: usize, payload: &[u8], summary: &mut GateSummary) {
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!("offset {offset}: depth payload shorter than 40 bytes"));
        return;
    }
    let row_count = read_u32(payload, 8) as usize;
    let changed_rows = read_u32(payload, 12) as usize;
    let status_flags = read_u32(payload, 16);
    let observed_army_rows = read_u32(payload, 20) as usize;
    let observed_order_rows = read_u32(payload, 24) as usize;
    let target_count = read_u32(payload, 28) as usize;
    let unsupported_geometry_rows = read_u32(payload, 32) as usize;
    let execution_unknown_rows = read_u32(payload, 36) as usize;
    let expected_len = HEADER_LEN
        .saturating_add(row_count.saturating_mul(ROW_LEN))
        .saturating_add(target_count.saturating_mul(TARGET_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: depth payload length {} != {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!("offset {offset}: changed rows exceed row count"));
    }
    if observed_order_rows < row_count {
        summary.parse_errors.push(format!("offset {offset}: observed order rows below emitted rows"));
    }
    if observed_army_rows == 0 && row_count != 0 {
        summary.parse_errors.push(format!("offset {offset}: emitted rows with zero observed armies"));
    }

    summary.totals.data_records = summary.totals.data_records.saturating_add(1);
    summary.totals.rows = summary.totals.rows.saturating_add(row_count as u64);
    summary.totals.changed_rows = summary.totals.changed_rows.saturating_add(changed_rows as u64);
    summary.totals.target_rows = summary.totals.target_rows.saturating_add(target_count as u64);
    summary.totals.unsupported_geometry_rows = summary
        .totals
        .unsupported_geometry_rows
        .saturating_add(unsupported_geometry_rows as u64);
    summary.totals.execution_unknown_rows = summary
        .totals
        .execution_unknown_rows
        .saturating_add(execution_unknown_rows as u64);
    summary.totals.status_flags_or |= status_flags;
    summary.status_flags_or |= status_flags;

    let mut cursor = HEADER_LEN;
    let mut declared_targets = 0usize;
    let mut seen = BTreeSet::new();
    let mut measured_unsupported_geometry_rows = 0usize;
    let mut measured_execution_unknown_rows = 0usize;
    for row_index in 0..row_count {
        let army_id = read_u32(payload, cursor);
        let generation = read_u64(payload, cursor + 4);
        let flags = read_u32(payload, cursor + 12);
        let slot_index = read_u32(payload, cursor + 16);
        let order_type = read_i32(payload, cursor + 20);
        let target_first = read_u32(payload, cursor + 24) as usize;
        let row_target_count = read_u32(payload, cursor + 28) as usize;
        let target_declared = read_u32(payload, cursor + 32);
        let target_status = read_u32(payload, cursor + 36);
        let geometry_status = read_u32(payload, cursor + 40);
        let execution_status = read_u32(payload, cursor + 44);
        let reserved0 = read_u32(payload, cursor + 48);
        let reserved1 = read_u32(payload, cursor + 52);
        let reserved2 = read_u32(payload, cursor + 56);
        let reserved3 = read_u32(payload, cursor + 60);
        cursor += ROW_LEN;

        let deleted = flags & FLAG_DELETED != 0;
        if army_id == 0 || army_id > MAX_ARMY_ID {
            summary.parse_errors.push(format!("offset {offset}: row {row_index} has bad army id"));
        }
        if !seen.insert((army_id, generation, slot_index, deleted)) {
            summary.parse_errors.push(format!("offset {offset}: duplicate row {row_index}"));
        }
        if slot_index >= MAX_SLOT_INDEX {
            summary.parse_errors.push(format!("offset {offset}: row {row_index} has bad slot"));
        }
        let known = FLAG_DELETED
            | FLAG_TARGET_STATES
            | FLAG_TARGET_STATE_COUNT
            | FLAG_NON_TYPE5_GEOMETRY_BLOCKED
            | FLAG_EXECUTION_BLOCKED;
        if flags & !known != 0 {
            summary.parse_errors.push(format!("offset {offset}: row {row_index} has unknown flags"));
        }
        if target_status > STATUS_FIXTURE_REQUIRED
            || geometry_status > STATUS_FIXTURE_REQUIRED
            || execution_status > STATUS_FIXTURE_REQUIRED
        {
            summary.parse_errors.push(format!("offset {offset}: row {row_index} has bad status"));
        }
        if deleted && (target_first != 0 || row_target_count != 0 || target_declared != 0) {
            summary.parse_errors.push(format!("offset {offset}: deleted row {row_index} has target fields"));
        }
        if flags & FLAG_TARGET_STATES != 0 {
            let target_end = target_first.saturating_add(row_target_count);
            if target_end > target_count {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: row {row_index} target range exceeds child table"));
            }
            declared_targets = declared_targets.saturating_add(row_target_count);
        } else if target_first != 0 || row_target_count != 0 {
            summary.parse_errors.push(format!("offset {offset}: row {row_index} has target range without flag"));
        }
        if flags & FLAG_TARGET_STATE_COUNT == 0 && target_declared != 0 {
            summary.parse_errors.push(format!("offset {offset}: row {row_index} has target count without flag"));
        }
        if !deleted && order_type == 5 && target_status != STATUS_EXACT {
            summary.parse_errors.push(format!("offset {offset}: type-5 row {row_index} is not exact"));
        }
        if !deleted && order_type != 5 && geometry_status == STATUS_FIXTURE_REQUIRED {
            measured_unsupported_geometry_rows += 1;
        }
        if !deleted && execution_status == STATUS_FIXTURE_REQUIRED {
            measured_execution_unknown_rows += 1;
        }
        if reserved0 != 0 || reserved1 != 0 || reserved2 != 0 || reserved3 != 0 {
            summary.parse_errors.push(format!("offset {offset}: row {row_index} has nonzero reserved field"));
        }
    }
    if declared_targets != target_count {
        summary.parse_errors.push(format!("offset {offset}: target child count mismatch"));
    }
    if measured_unsupported_geometry_rows != unsupported_geometry_rows {
        summary.parse_errors.push(format!("offset {offset}: unsupported geometry row count mismatch"));
    }
    if measured_execution_unknown_rows != execution_unknown_rows {
        summary.parse_errors.push(format!("offset {offset}: execution unknown row count mismatch"));
    }
    for row_index in 0..target_count {
        let state_id = read_u32(payload, cursor);
        let reserved = read_u32(payload, cursor + 4);
        cursor += TARGET_ROW_LEN;
        if state_id == 0 || state_id > MAX_STATE_ID {
            summary.parse_errors.push(format!("offset {offset}: target row {row_index} has bad state id"));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!("offset {offset}: target row {row_index} has nonzero reserved"));
        }
    }
}

pub fn decode_compact_stream(bytes: &[u8]) -> GateSummary {
    let mut summary = GateSummary::default();
    let mut offset = 0usize;
    let mut last_sequence = None::<u64>;
    while offset < bytes.len() {
        if bytes.len().saturating_sub(offset) < COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!("offset {offset}: truncated compact record header"));
            break;
        }
        let header = &bytes[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            summary.parse_errors.push(format!("offset {offset}: bad compact record magic"));
            break;
        }
        let version = read_u16(header, 4);
        let header_len = read_u16(header, 6) as usize;
        let sequence = read_u64(header, 8);
        let game_hour_raw = read_i32(header, 16);
        let domain_id = read_u16(header, 20);
        let record_type = read_u16(header, 22);
        let payload_len = read_u32(header, 24) as usize;
        let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
        if version != COMPACT_RECORD_VERSION {
            summary.parse_errors.push(format!("offset {offset}: unsupported compact version {version}"));
        }
        if header_len != COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!("offset {offset}: bad header length {header_len}"));
        }
        if let Some(last) = last_sequence {
            if sequence <= last {
                summary
                    .ordering_errors
                    .push(format!("offset {offset}: sequence {sequence} is not greater than {last}"));
            }
        }
        last_sequence = Some(sequence);
        if bytes.len().saturating_sub(offset) < record_len {
            summary.parse_errors.push(format!("offset {offset}: payload length exceeds stream"));
            break;
        }
        let payload = &bytes[offset + COMPACT_RECORD_HEADER_LEN..offset + record_len];
        summary.record_count = summary.record_count.saturating_add(1);
        match record_type {
            RECORD_TYPE_FRAME_END => {
                summary.frame_end_count = summary.frame_end_count.saturating_add(1);
                summary.first_hour = Some(summary.first_hour.map_or(game_hour_raw, |v| v.min(game_hour_raw)));
                summary.last_hour = Some(summary.last_hour.map_or(game_hour_raw, |v| v.max(game_hour_raw)));
                summary.unique_frame_hours.insert(game_hour_raw);
                *summary.per_hour_frame_records.entry(game_hour_raw).or_default() += 1;
                if domain_id != DOMAIN_FRAME {
                    summary.parse_errors.push(format!("offset {offset}: frame-end record used domain {domain_id}"));
                }
            }
            RECORD_TYPE_DOMAIN_DATA => {
                if domain_id == DOMAIN_ARMY_BATTLEPLANS_DEPTH {
                    *summary.per_hour_depth_records.entry(game_hour_raw).or_default() += 1;
                    decode_depth_payload(offset, payload, &mut summary);
                }
            }
            _ => {}
        }
        offset += record_len;
    }
    summary
}

pub fn gate_depth_stream(bytes: &[u8], min_game_hours: i32) -> Vec<String> {
    let summary = decode_compact_stream(bytes);
    let mut errors = Vec::new();
    for hour in &summary.unique_frame_hours {
        if summary.per_hour_frame_records.get(hour).copied().unwrap_or(0) != 1 {
            errors.push(format!("game_hour_raw {hour}: expected one frame-end record"));
        }
        if summary.per_hour_depth_records.get(hour).copied().unwrap_or(0) != 1 {
            errors.push(format!("game_hour_raw {hour}: expected one depth data record"));
        }
    }
    if let (Some(first), Some(last)) = (summary.first_hour, summary.last_hour) {
        let advanced = last.saturating_sub(first);
        if advanced < min_game_hours {
            errors.push(format!("advanced {advanced} game hours, expected at least {min_game_hours}"));
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
        errors.push(format!("failure status flags present: 0x{:X}", summary.status_flags_or & FAIL_STATUS_MASK));
    }
    errors.extend(summary.parse_errors);
    errors.extend(summary.ordering_errors);
    errors
}

#[cfg(not(test))]
fn main() {
    let mut args = std::env::args().skip(1);
    let Some(stream_path) = args.next() else {
        eprintln!("usage: army_battleplans_depth_compact <depth_stream.bin> [min_game_hours]");
        std::process::exit(2);
    };
    let min_game_hours = args.next().and_then(|value| value.parse::<i32>().ok()).unwrap_or(5);
    let bytes = match std::fs::read(&stream_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("read failed for {stream_path}: {err}");
            std::process::exit(2);
        }
    };
    let summary = decode_compact_stream(&bytes);
    let errors = gate_depth_stream(&bytes, min_game_hours);
    println!(
        "status={} records={} frames={} hours={:?}..{:?} rows={} target_rows={} unsupported_geometry_rows={} execution_unknown_rows={} status_flags=0x{:X}",
        if errors.is_empty() { "PASS" } else { "FAIL" },
        summary.record_count,
        summary.frame_end_count,
        summary.first_hour,
        summary.last_hour,
        summary.totals.rows,
        summary.totals.target_rows,
        summary.totals.unsupported_geometry_rows,
        summary.totals.execution_unknown_rows,
        summary.status_flags_or
    );
    if !errors.is_empty() {
        for error in errors {
            eprintln!("{error}");
        }
        std::process::exit(1);
    }
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
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
    fn encodes_and_decodes_type5_exact_and_non_type5_blocked_rows() {
        let payload = encode_depth_payload(
            &[
                BattleplanDepthRow {
                    army_id: 17,
                    generation: 9,
                    deleted: false,
                    slot_index: 0,
                    order_type: 5,
                    target_states: vec![123, 456],
                    target_declared_count: Some(2),
                    target_status: STATUS_EXACT,
                    geometry_status: STATUS_EXACT,
                    execution_status: STATUS_FIXTURE_REQUIRED,
                },
                BattleplanDepthRow {
                    army_id: 18,
                    generation: 10,
                    deleted: false,
                    slot_index: 1,
                    order_type: 2,
                    target_states: Vec::new(),
                    target_declared_count: None,
                    target_status: STATUS_UNSUPPORTED,
                    geometry_status: STATUS_FIXTURE_REQUIRED,
                    execution_status: STATUS_FIXTURE_REQUIRED,
                },
            ],
            11,
            0,
            2,
            2,
            2,
        );
        let mut summary = GateSummary::default();
        decode_depth_payload(0, &payload, &mut summary);
        assert_eq!(summary.parse_errors, Vec::<String>::new());
        assert_eq!(summary.totals.rows, 2);
        assert_eq!(summary.totals.target_rows, 2);
        assert_eq!(summary.totals.unsupported_geometry_rows, 1);
        assert_eq!(summary.totals.execution_unknown_rows, 2);
    }

    #[test]
    fn rejects_type5_without_exact_target_status() {
        let payload = encode_depth_payload(
            &[BattleplanDepthRow {
                army_id: 17,
                generation: 9,
                deleted: false,
                slot_index: 0,
                order_type: 5,
                target_states: Vec::new(),
                target_declared_count: Some(0),
                target_status: STATUS_UNSUPPORTED,
                geometry_status: STATUS_UNKNOWN,
                execution_status: STATUS_FIXTURE_REQUIRED,
            }],
            11,
            0,
            1,
            1,
            1,
        );
        let mut summary = GateSummary::default();
        decode_depth_payload(0, &payload, &mut summary);
        assert!(summary.parse_errors.iter().any(|err| err.contains("type-5 row 0 is not exact")));
    }
}
