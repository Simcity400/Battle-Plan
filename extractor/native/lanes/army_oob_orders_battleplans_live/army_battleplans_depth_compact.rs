//! Lane-local compact encoder/decoder/gate for army_battleplans_depth_live.
//!
//! Registry assignment: domain 54 = army_battleplans_depth_live. This module is
//! intentionally not wired into shared collector/recorder entrypoints.

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
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const DOMAIN_ARMY_BATTLEPLANS_DEPTH: u16 = 54;
pub const DOMAIN_FRAME: u16 = 1000;

pub const HEADER_LEN: usize = 52;
pub const ROW_LEN: usize = 64;
pub const TARGET_ROW_LEN: usize = 8;
pub const PATH_GEOMETRY_ROW_LEN: usize = 8;
pub const DIRECT_ROOT_ROW_LEN: usize = 64;
pub const DIRECT_ROOT_MEMBER_ROW_LEN: usize = 8;

pub const FLAG_DELETED: u32 = 1;
pub const FLAG_TARGET_STATES: u32 = 1 << 1;
pub const FLAG_TARGET_STATE_COUNT: u32 = 1 << 2;
pub const FLAG_NON_TYPE5_GEOMETRY_BLOCKED: u32 = 1 << 3;
pub const FLAG_EXECUTION_BLOCKED: u32 = 1 << 4;
pub const FLAG_PATH_GEOMETRY: u32 = 1 << 5;

pub const STATUS_NONE: u32 = 0;
pub const STATUS_EXACT: u32 = 1;
pub const STATUS_UNKNOWN: u32 = 2;
pub const STATUS_UNSUPPORTED: u32 = 3;
pub const STATUS_OVERFLOW: u32 = 4;
pub const STATUS_VALIDATION_FAILED: u32 = 5;
pub const STATUS_FIXTURE_REQUIRED: u32 = 6;
pub const STATUS_COLD_PREFLIGHT_ONLY: u32 = 7;

pub const DIRECT_KIND_ARMY: u32 = 1;
pub const DIRECT_KIND_ARMY_GROUP: u32 = 2;
pub const DIRECT_KIND_THEATRE: u32 = 5;

pub const DIRECT_FLAG_DELETED: u32 = 1;
pub const DIRECT_FLAG_PARENT: u32 = 1 << 1;
pub const DIRECT_FLAG_OWNER: u32 = 1 << 2;
pub const DIRECT_FLAG_ENGINE_MEMBER_COUNT: u32 = 1 << 3;
pub const DIRECT_FLAG_MISSING_MEMBER_COUNT: u32 = 1 << 4;
pub const DIRECT_FLAG_MEMBER_IDS: u32 = 1 << 5;
pub const DIRECT_FLAG_ORDER_COUNTS: u32 = 1 << 6;

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

const MAX_ARMY_ID: u32 = i32::MAX as u32;
const MAX_SLOT_INDEX: u32 = 64;
const MAX_STATE_ID: u32 = 65_535;
const MAX_PROVINCE_ID: u32 = 65_535;
const MAX_DIRECT_ID: u32 = 4_000_000;
const MAX_DIRECT_MEMBER_ROWS: usize = 250_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattleplanDepthRow {
    pub army_id: u32,
    pub generation: u64,
    pub deleted: bool,
    pub slot_index: u32,
    pub order_type: i32,
    pub target_states: Vec<u32>,
    pub path_provinces: Vec<u32>,
    pub target_declared_count: Option<u32>,
    pub target_status: u32,
    pub geometry_status: u32,
    pub execution_status: u32,
    pub assigned_units_status: u32,
    pub order_linkage_status: u32,
    pub stance_status: u32,
    pub aggressiveness_status: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectRootRow {
    pub kind: u32,
    pub object_id: u32,
    pub deleted: bool,
    pub parent_id: Option<u32>,
    pub owner_tag_db_id: Option<u32>,
    pub member_ids: Vec<u32>,
    pub engine_member_count: Option<u32>,
    pub missing_member_count: Option<u32>,
    pub order_count: Option<u32>,
    pub non_type5_order_count: Option<u32>,
    pub parent_status: u32,
    pub member_status: u32,
    pub root_status: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainTotals {
    pub data_records: u64,
    pub rows: u64,
    pub changed_rows: u64,
    pub target_rows: u64,
    pub path_geometry_rows: u64,
    pub non_type5_exact_path_rows: u64,
    pub unsupported_geometry_rows: u64,
    pub execution_unknown_rows: u64,
    pub assigned_units_exact_rows: u64,
    pub order_linkage_exact_rows: u64,
    pub stance_unknown_rows: u64,
    pub aggressiveness_unknown_rows: u64,
    pub direct_root_rows: u64,
    pub direct_root_member_rows: u64,
    pub direct_army_rows: u64,
    pub direct_army_group_rows: u64,
    pub direct_theatre_rows: u64,
    pub direct_cold_preflight_rows: u64,
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
    direct_roots: &[DirectRootRow],
    elapsed_wall_ms: u64,
    status_flags: u32,
    changed_rows: u32,
    observed_army_rows: u32,
    observed_order_rows: u32,
) -> Vec<u8> {
    let target_count = rows
        .iter()
        .map(|row| row.target_states.len())
        .sum::<usize>();
    let path_geometry_count = rows
        .iter()
        .map(|row| row.path_provinces.len())
        .sum::<usize>();
    let direct_member_count = direct_roots
        .iter()
        .map(|row| row.member_ids.len())
        .sum::<usize>();
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
    let mut out = Vec::with_capacity(
        HEADER_LEN
            + rows.len() * ROW_LEN
            + target_count * TARGET_ROW_LEN
            + path_geometry_count * PATH_GEOMETRY_ROW_LEN
            + direct_roots.len() * DIRECT_ROOT_ROW_LEN
            + direct_member_count * DIRECT_ROOT_MEMBER_ROW_LEN,
    );
    out.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    out.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&changed_rows.to_le_bytes());
    out.extend_from_slice(&status_flags.to_le_bytes());
    out.extend_from_slice(&observed_army_rows.to_le_bytes());
    out.extend_from_slice(&observed_order_rows.to_le_bytes());
    out.extend_from_slice(&(target_count.min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&unsupported_geometry_rows.to_le_bytes());
    out.extend_from_slice(&execution_unknown_rows.to_le_bytes());
    out.extend_from_slice(&(direct_roots.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&(direct_member_count.min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&(path_geometry_count.min(u32::MAX as usize) as u32).to_le_bytes());

    let mut target_cursor = 0u32;
    let mut path_cursor = 0u32;
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
        if !row.path_provinces.is_empty() {
            flags |= FLAG_PATH_GEOMETRY;
        }
        debug_assert!(
            row.target_states.is_empty() || row.path_provinces.is_empty(),
            "domain-54 row can encode one child range only"
        );
        let child_first = if !row.target_states.is_empty() {
            target_cursor
        } else if !row.path_provinces.is_empty() {
            path_cursor
        } else {
            0
        };
        let child_count = if !row.target_states.is_empty() {
            row.target_states.len().min(u32::MAX as usize) as u32
        } else if !row.path_provinces.is_empty() {
            row.path_provinces.len().min(u32::MAX as usize) as u32
        } else {
            0
        };
        out.extend_from_slice(&row.army_id.to_le_bytes());
        out.extend_from_slice(&row.generation.to_le_bytes());
        for value in [
            flags,
            row.slot_index,
            row.order_type as u32,
            child_first,
            child_count,
            row.target_declared_count.unwrap_or(0),
            row.target_status,
            row.geometry_status,
            row.execution_status,
            row.assigned_units_status,
            row.order_linkage_status,
            row.stance_status,
            row.aggressiveness_status,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        target_cursor =
            target_cursor.saturating_add(row.target_states.len().min(u32::MAX as usize) as u32);
        path_cursor =
            path_cursor.saturating_add(row.path_provinces.len().min(u32::MAX as usize) as u32);
    }
    for row in rows {
        for state_id in &row.target_states {
            out.extend_from_slice(&state_id.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
    }
    for row in rows {
        for province_id in &row.path_provinces {
            out.extend_from_slice(&province_id.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
    }
    let mut member_cursor = 0u32;
    for row in direct_roots {
        let mut flags = 0u32;
        if row.deleted {
            flags |= DIRECT_FLAG_DELETED;
        }
        if row.parent_id.is_some() {
            flags |= DIRECT_FLAG_PARENT;
        }
        if row.owner_tag_db_id.is_some() {
            flags |= DIRECT_FLAG_OWNER;
        }
        if row.engine_member_count.is_some() {
            flags |= DIRECT_FLAG_ENGINE_MEMBER_COUNT;
        }
        if row.missing_member_count.is_some() {
            flags |= DIRECT_FLAG_MISSING_MEMBER_COUNT;
        }
        if !row.member_ids.is_empty() {
            flags |= DIRECT_FLAG_MEMBER_IDS;
        }
        if row.order_count.is_some() || row.non_type5_order_count.is_some() {
            flags |= DIRECT_FLAG_ORDER_COUNTS;
        }
        let member_count = row.member_ids.len().min(u32::MAX as usize) as u32;
        let member_first = if member_count == 0 { 0 } else { member_cursor };
        for value in [
            row.kind,
            row.object_id,
            flags,
            row.parent_id.unwrap_or(0),
            row.owner_tag_db_id.unwrap_or(0),
            member_first,
            member_count,
            row.engine_member_count.unwrap_or(0),
            row.missing_member_count.unwrap_or(0),
            row.order_count.unwrap_or(0),
            row.non_type5_order_count.unwrap_or(0),
            row.parent_status,
            row.member_status,
            row.root_status,
            0,
            0,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        member_cursor = member_cursor.saturating_add(member_count);
    }
    for row in direct_roots {
        for member_id in &row.member_ids {
            out.extend_from_slice(&member_id.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
    }
    out
}

pub fn decode_depth_payload(offset: usize, payload: &[u8], summary: &mut GateSummary) {
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: depth payload shorter than {HEADER_LEN} bytes"
        ));
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
    let direct_root_count = read_u32(payload, 40) as usize;
    let direct_member_count = read_u32(payload, 44) as usize;
    let path_geometry_count = read_u32(payload, 48) as usize;
    let expected_len = HEADER_LEN
        .saturating_add(row_count.saturating_mul(ROW_LEN))
        .saturating_add(target_count.saturating_mul(TARGET_ROW_LEN))
        .saturating_add(path_geometry_count.saturating_mul(PATH_GEOMETRY_ROW_LEN))
        .saturating_add(direct_root_count.saturating_mul(DIRECT_ROOT_ROW_LEN))
        .saturating_add(direct_member_count.saturating_mul(DIRECT_ROOT_MEMBER_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: depth payload length {} != {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary
            .parse_errors
            .push(format!("offset {offset}: changed rows exceed row count"));
    }
    // Delta rows include tombstones and generation replacements, so emitted
    // rows may exceed the current live census. Only a full baseline must be
    // bounded by observed_order_rows.
    if status_flags & COMPACT_STATUS_DELTA_DATA == 0 && observed_order_rows < row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: observed order rows below emitted rows"
        ));
    }
    if observed_army_rows == 0 && row_count != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: emitted rows with zero observed armies"
        ));
    }
    if direct_member_count > MAX_DIRECT_MEMBER_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: direct root member count exceeds limit"
        ));
    }

    summary.totals.data_records = summary.totals.data_records.saturating_add(1);
    summary.totals.rows = summary.totals.rows.saturating_add(row_count as u64);
    summary.totals.changed_rows = summary
        .totals
        .changed_rows
        .saturating_add(changed_rows as u64);
    summary.totals.target_rows = summary
        .totals
        .target_rows
        .saturating_add(target_count as u64);
    summary.totals.path_geometry_rows = summary
        .totals
        .path_geometry_rows
        .saturating_add(path_geometry_count as u64);
    summary.totals.unsupported_geometry_rows = summary
        .totals
        .unsupported_geometry_rows
        .saturating_add(unsupported_geometry_rows as u64);
    summary.totals.execution_unknown_rows = summary
        .totals
        .execution_unknown_rows
        .saturating_add(execution_unknown_rows as u64);
    let status_flags =
        status_for_global_interpretation(DOMAIN_ARMY_BATTLEPLANS_DEPTH, status_flags);
    summary.totals.status_flags_or |= status_flags;
    summary.status_flags_or |= status_flags;

    let mut cursor = HEADER_LEN;
    let mut declared_targets = 0usize;
    let mut declared_paths = 0usize;
    let mut seen = BTreeSet::new();
    let mut measured_unsupported_geometry_rows = 0usize;
    let mut measured_execution_unknown_rows = 0usize;
    let mut measured_non_type5_exact_path_rows = 0usize;
    for row_index in 0..row_count {
        let army_id = read_u32(payload, cursor);
        let generation = read_u64(payload, cursor + 4);
        let flags = read_u32(payload, cursor + 12);
        let slot_index = read_u32(payload, cursor + 16);
        let order_type = read_i32(payload, cursor + 20);
        let child_first = read_u32(payload, cursor + 24) as usize;
        let child_count = read_u32(payload, cursor + 28) as usize;
        let target_declared = read_u32(payload, cursor + 32);
        let target_status = read_u32(payload, cursor + 36);
        let geometry_status = read_u32(payload, cursor + 40);
        let execution_status = read_u32(payload, cursor + 44);
        let assigned_units_status = read_u32(payload, cursor + 48);
        let order_linkage_status = read_u32(payload, cursor + 52);
        let stance_status = read_u32(payload, cursor + 56);
        let aggressiveness_status = read_u32(payload, cursor + 60);
        cursor += ROW_LEN;

        let deleted = flags & FLAG_DELETED != 0;
        if army_id == 0 || army_id > MAX_ARMY_ID {
            summary
                .parse_errors
                .push(format!("offset {offset}: row {row_index} has bad army id"));
        }
        if !seen.insert((army_id, generation, slot_index, deleted)) {
            summary
                .parse_errors
                .push(format!("offset {offset}: duplicate row {row_index}"));
        }
        if slot_index >= MAX_SLOT_INDEX {
            summary
                .parse_errors
                .push(format!("offset {offset}: row {row_index} has bad slot"));
        }
        let known = FLAG_DELETED
            | FLAG_TARGET_STATES
            | FLAG_TARGET_STATE_COUNT
            | FLAG_NON_TYPE5_GEOMETRY_BLOCKED
            | FLAG_EXECUTION_BLOCKED
            | FLAG_PATH_GEOMETRY;
        if flags & !known != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has unknown flags"
            ));
        }
        if target_status > STATUS_FIXTURE_REQUIRED
            || geometry_status > STATUS_FIXTURE_REQUIRED
            || execution_status > STATUS_FIXTURE_REQUIRED
            || assigned_units_status > STATUS_COLD_PREFLIGHT_ONLY
            || order_linkage_status > STATUS_COLD_PREFLIGHT_ONLY
            || stance_status > STATUS_COLD_PREFLIGHT_ONLY
            || aggressiveness_status > STATUS_COLD_PREFLIGHT_ONLY
        {
            summary
                .parse_errors
                .push(format!("offset {offset}: row {row_index} has bad status"));
        }
        if deleted && (child_first != 0 || child_count != 0 || target_declared != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: deleted row {row_index} has target fields"
            ));
        }
        let has_target = flags & FLAG_TARGET_STATES != 0;
        let has_path = flags & FLAG_PATH_GEOMETRY != 0;
        if has_target && has_path {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has both target-state and path-geometry flags"
            ));
        }
        if has_target {
            let target_end = child_first.saturating_add(child_count);
            if target_end > target_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: row {row_index} target range exceeds child table"
                ));
            }
            declared_targets = declared_targets.saturating_add(child_count);
            if order_type != 5 {
                summary.parse_errors.push(format!(
                    "offset {offset}: non-type-5 row {row_index} has target-state child rows"
                ));
            }
        }
        if has_path {
            let path_end = child_first.saturating_add(child_count);
            if path_end > path_geometry_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: row {row_index} path range exceeds child table"
                ));
            }
            if child_count == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: row {row_index} has empty path geometry range"
                ));
            }
            declared_paths = declared_paths.saturating_add(child_count);
            if order_type == 5 {
                summary.parse_errors.push(format!(
                    "offset {offset}: type-5 row {row_index} has path geometry"
                ));
            }
            if geometry_status != STATUS_EXACT {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: row {row_index} has path geometry without exact geometry status"));
            }
        } else if !has_target && (child_first != 0 || child_count != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has child range without target/path flag"
            ));
        }
        if flags & FLAG_TARGET_STATE_COUNT == 0 && target_declared != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has target count without flag"
            ));
        }
        if !deleted && order_type == 5 && target_status != STATUS_EXACT {
            summary.parse_errors.push(format!(
                "offset {offset}: type-5 row {row_index} is not exact"
            ));
        }
        if !deleted && order_type != 5 {
            if geometry_status == STATUS_FIXTURE_REQUIRED {
                measured_unsupported_geometry_rows += 1;
            } else if geometry_status == STATUS_EXACT {
                if has_path {
                    measured_non_type5_exact_path_rows += 1;
                    summary.totals.non_type5_exact_path_rows =
                        summary.totals.non_type5_exact_path_rows.saturating_add(1);
                } else {
                    summary.parse_errors.push(format!(
                        "offset {offset}: exact non-type-5 row {row_index} lacks path geometry"
                    ));
                }
            }
        }
        if !deleted && execution_status == STATUS_FIXTURE_REQUIRED {
            measured_execution_unknown_rows += 1;
        }
        if !deleted {
            if assigned_units_status == STATUS_EXACT {
                summary.totals.assigned_units_exact_rows =
                    summary.totals.assigned_units_exact_rows.saturating_add(1);
            } else {
                summary.parse_errors.push(format!(
                    "offset {offset}: row {row_index} lacks exact assigned-unit status"
                ));
            }
            if order_linkage_status == STATUS_EXACT {
                summary.totals.order_linkage_exact_rows =
                    summary.totals.order_linkage_exact_rows.saturating_add(1);
            } else {
                summary.parse_errors.push(format!(
                    "offset {offset}: row {row_index} lacks exact order-linkage status"
                ));
            }
            if stance_status == STATUS_UNKNOWN {
                summary.totals.stance_unknown_rows =
                    summary.totals.stance_unknown_rows.saturating_add(1);
            }
            if aggressiveness_status == STATUS_UNKNOWN {
                summary.totals.aggressiveness_unknown_rows =
                    summary.totals.aggressiveness_unknown_rows.saturating_add(1);
            }
        } else if assigned_units_status != STATUS_NONE
            || order_linkage_status != STATUS_NONE
            || stance_status != STATUS_NONE
            || aggressiveness_status != STATUS_NONE
        {
            summary.parse_errors.push(format!(
                "offset {offset}: deleted row {row_index} has live residual statuses"
            ));
        }
    }
    if declared_targets != target_count {
        summary
            .parse_errors
            .push(format!("offset {offset}: target child count mismatch"));
    }
    if declared_paths != path_geometry_count {
        summary.parse_errors.push(format!(
            "offset {offset}: path geometry child count mismatch"
        ));
    }
    if measured_unsupported_geometry_rows != unsupported_geometry_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: unsupported geometry row count mismatch"
        ));
    }
    if measured_non_type5_exact_path_rows > path_geometry_count {
        summary.parse_errors.push(format!(
            "offset {offset}: exact path row count exceeds path child rows"
        ));
    }
    if measured_execution_unknown_rows != execution_unknown_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: execution unknown row count mismatch"
        ));
    }
    for row_index in 0..target_count {
        let state_id = read_u32(payload, cursor);
        let reserved = read_u32(payload, cursor + 4);
        cursor += TARGET_ROW_LEN;
        if state_id == 0 || state_id > MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: target row {row_index} has bad state id"
            ));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: target row {row_index} has nonzero reserved"
            ));
        }
    }
    for row_index in 0..path_geometry_count {
        let province_id = read_u32(payload, cursor);
        let reserved = read_u32(payload, cursor + 4);
        cursor += PATH_GEOMETRY_ROW_LEN;
        if province_id == 0 || province_id > MAX_PROVINCE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: path geometry row {row_index} has bad province id"
            ));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: path geometry row {row_index} has nonzero reserved"
            ));
        }
    }
    decode_direct_root_rows(
        offset,
        payload,
        &mut cursor,
        direct_root_count,
        direct_member_count,
        summary,
    );
}

fn decode_direct_root_rows(
    offset: usize,
    payload: &[u8],
    cursor: &mut usize,
    direct_root_count: usize,
    direct_member_count: usize,
    summary: &mut GateSummary,
) {
    summary.totals.direct_root_rows = summary
        .totals
        .direct_root_rows
        .saturating_add(direct_root_count as u64);
    summary.totals.direct_root_member_rows = summary
        .totals
        .direct_root_member_rows
        .saturating_add(direct_member_count as u64);

    let mut declared_members = 0usize;
    let mut seen = BTreeSet::new();
    for row_index in 0..direct_root_count {
        let kind = read_u32(payload, *cursor);
        let object_id = read_u32(payload, *cursor + 4);
        let flags = read_u32(payload, *cursor + 8);
        let parent_id = read_u32(payload, *cursor + 12);
        let owner_tag_db_id = read_u32(payload, *cursor + 16);
        let member_first = read_u32(payload, *cursor + 20) as usize;
        let member_count = read_u32(payload, *cursor + 24) as usize;
        let engine_member_count = read_u32(payload, *cursor + 28);
        let missing_member_count = read_u32(payload, *cursor + 32);
        let order_count = read_u32(payload, *cursor + 36);
        let non_type5_order_count = read_u32(payload, *cursor + 40);
        let parent_status = read_u32(payload, *cursor + 44);
        let member_status = read_u32(payload, *cursor + 48);
        let root_status = read_u32(payload, *cursor + 52);
        let reserved0 = read_u32(payload, *cursor + 56);
        let reserved1 = read_u32(payload, *cursor + 60);
        *cursor += DIRECT_ROOT_ROW_LEN;

        match kind {
            DIRECT_KIND_ARMY => {
                summary.totals.direct_army_rows = summary.totals.direct_army_rows.saturating_add(1)
            }
            DIRECT_KIND_ARMY_GROUP => {
                summary.totals.direct_army_group_rows =
                    summary.totals.direct_army_group_rows.saturating_add(1)
            }
            DIRECT_KIND_THEATRE => {
                summary.totals.direct_theatre_rows =
                    summary.totals.direct_theatre_rows.saturating_add(1)
            }
            _ => summary.parse_errors.push(format!(
                "offset {offset}: direct root row {row_index} has bad kind {kind}"
            )),
        }
        if object_id == 0 || object_id > MAX_DIRECT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: direct root row {row_index} has bad object id"
            ));
        }
        if !seen.insert((kind, object_id, flags & DIRECT_FLAG_DELETED != 0)) {
            summary.parse_errors.push(format!(
                "offset {offset}: duplicate direct root row {row_index}"
            ));
        }
        let known_flags = DIRECT_FLAG_DELETED
            | DIRECT_FLAG_PARENT
            | DIRECT_FLAG_OWNER
            | DIRECT_FLAG_ENGINE_MEMBER_COUNT
            | DIRECT_FLAG_MISSING_MEMBER_COUNT
            | DIRECT_FLAG_MEMBER_IDS
            | DIRECT_FLAG_ORDER_COUNTS;
        if flags & !known_flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: direct root row {row_index} has unknown flags"
            ));
        }
        validate_direct_optional_ref(
            summary,
            offset,
            row_index,
            flags & DIRECT_FLAG_PARENT != 0,
            parent_id,
            "parent_id",
        );
        validate_direct_optional_ref(
            summary,
            offset,
            row_index,
            flags & DIRECT_FLAG_OWNER != 0,
            owner_tag_db_id,
            "owner_tag_db_id",
        );
        validate_direct_optional_count(
            summary,
            offset,
            row_index,
            flags & DIRECT_FLAG_ENGINE_MEMBER_COUNT != 0,
            engine_member_count,
            "engine_member_count",
        );
        validate_direct_optional_count(
            summary,
            offset,
            row_index,
            flags & DIRECT_FLAG_MISSING_MEMBER_COUNT != 0,
            missing_member_count,
            "missing_member_count",
        );
        if flags & DIRECT_FLAG_ORDER_COUNTS == 0 && (order_count != 0 || non_type5_order_count != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: direct root row {row_index} has order counts without flag"
            ));
        }
        if non_type5_order_count > order_count {
            summary.parse_errors.push(format!(
                "offset {offset}: direct root row {row_index} non-type5 count exceeds order count"
            ));
        }
        for (name, status) in [
            ("parent", parent_status),
            ("member", member_status),
            ("root", root_status),
        ] {
            if status > STATUS_COLD_PREFLIGHT_ONLY {
                summary.parse_errors.push(format!(
                    "offset {offset}: direct root row {row_index} has bad {name} status"
                ));
            }
        }
        if root_status == STATUS_COLD_PREFLIGHT_ONLY {
            summary.totals.direct_cold_preflight_rows =
                summary.totals.direct_cold_preflight_rows.saturating_add(1);
        }
        if flags & DIRECT_FLAG_MEMBER_IDS != 0 {
            let member_end = member_first.saturating_add(member_count);
            if member_end > direct_member_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: direct root row {row_index} member range exceeds child table"
                ));
            }
            declared_members = declared_members.saturating_add(member_count);
        } else if member_first != 0 || member_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: direct root row {row_index} has member fields without flag"
            ));
        }
        if reserved0 != 0 || reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: direct root row {row_index} has nonzero reserved field"
            ));
        }
    }
    if declared_members != direct_member_count {
        summary.parse_errors.push(format!(
            "offset {offset}: direct root member child count mismatch"
        ));
    }
    for row_index in 0..direct_member_count {
        let member_id = read_u32(payload, *cursor);
        let reserved = read_u32(payload, *cursor + 4);
        *cursor += DIRECT_ROOT_MEMBER_ROW_LEN;
        if member_id == 0 || member_id > MAX_DIRECT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: direct root member row {row_index} has bad id"
            ));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: direct root member row {row_index} has nonzero reserved"
            ));
        }
    }
}

fn validate_direct_optional_ref(
    summary: &mut GateSummary,
    offset: usize,
    row_index: usize,
    present: bool,
    value: u32,
    field: &str,
) {
    if present {
        if value == 0 || value > MAX_DIRECT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: direct root row {row_index} has bad {field}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: direct root row {row_index} has {field} without flag"
        ));
    }
}

fn validate_direct_optional_count(
    summary: &mut GateSummary,
    offset: usize,
    row_index: usize,
    present: bool,
    value: u32,
    field: &str,
) {
    if present {
        if value > MAX_DIRECT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: direct root row {row_index} has bad {field}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: direct root row {row_index} has {field} without flag"
        ));
    }
}

pub fn decode_compact_stream(bytes: &[u8]) -> GateSummary {
    let mut summary = GateSummary::default();
    let mut offset = 0usize;
    let mut last_sequence = None::<u64>;
    let mut open_hour = None::<i32>;
    let mut pending_perf_hour = None::<i32>;
    let mut last_closed_hour = None::<i32>;
    let mut saw_frame_end = false;
    let mut last_record_was_clean_frame_end = false;
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
        let payload = &bytes[offset + COMPACT_RECORD_HEADER_LEN..offset + record_len];
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
        last_record_was_clean_frame_end = false;
        match record_type {
            RECORD_TYPE_FRAME_END if semantics_valid => {
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
            RECORD_TYPE_DOMAIN_DATA if semantics_valid => {
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
                if domain_id == DOMAIN_ARMY_BATTLEPLANS_DEPTH {
                    *summary
                        .per_hour_depth_records
                        .entry(game_hour_raw)
                        .or_default() += 1;
                    decode_depth_payload(offset, payload, &mut summary);
                }
            }
            RECORD_TYPE_DOMAIN_PERF if semantics_valid => {
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
                if domain_id == DOMAIN_ARMY_BATTLEPLANS_DEPTH {
                    summary.status_flags_or |=
                        status_for_global_interpretation(domain_id, read_u32(payload, 32));
                }
            }
            RECORD_TYPE_FRAME_END | RECORD_TYPE_DOMAIN_DATA | RECORD_TYPE_DOMAIN_PERF => {}
            other => summary
                .parse_errors
                .push(format!("offset {offset}: unsupported record type {other}")),
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

pub fn gate_depth_stream(bytes: &[u8], min_game_hours: i32) -> Vec<String> {
    let summary = decode_compact_stream(bytes);
    let mut errors = Vec::new();
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
        if summary
            .per_hour_depth_records
            .get(hour)
            .copied()
            .unwrap_or(0)
            != 1
        {
            errors.push(format!(
                "game_hour_raw {hour}: expected one depth data record"
            ));
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
    errors.extend(summary.parse_errors);
    errors.extend(summary.ordering_errors);
    errors
}

#[cfg(not(test))]
fn main() {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let json_output = args.iter().any(|arg| arg == "--json");
    args.retain(|arg| arg != "--json");
    let Some(stream_path) = args.first() else {
        eprintln!(
            "usage: army_battleplans_depth_compact <depth_stream.bin> [min_game_hours] [--json]"
        );
        std::process::exit(2);
    };
    let min_game_hours = args
        .get(1)
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(5);
    let bytes = match std::fs::read(&stream_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("read failed for {stream_path}: {err}");
            std::process::exit(2);
        }
    };
    let summary = decode_compact_stream(&bytes);
    let errors = gate_depth_stream(&bytes, min_game_hours);
    if json_output {
        print_json_gate_summary(&summary, &errors, min_game_hours);
        if !errors.is_empty() {
            std::process::exit(1);
        }
        return;
    }
    println!(
        "status={} records={} frames={} hours={:?}..{:?} rows={} target_rows={} path_geometry_rows={} non_type5_exact_path_rows={} unsupported_geometry_rows={} execution_unknown_rows={} status_flags=0x{:X}",
        if errors.is_empty() { "PASS" } else { "FAIL" },
        summary.record_count,
        summary.frame_end_count,
        summary.first_hour,
        summary.last_hour,
        summary.totals.rows,
        summary.totals.target_rows,
        summary.totals.path_geometry_rows,
        summary.totals.non_type5_exact_path_rows,
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

#[cfg(not(test))]
fn print_json_gate_summary(summary: &GateSummary, errors: &[String], min_game_hours: i32) {
    let advanced = match (summary.first_hour, summary.last_hour) {
        (Some(first), Some(last)) => Some(last.saturating_sub(first)),
        _ => None,
    };
    let expected_hours = advanced.map(|value| value.saturating_add(1).max(0) as usize);
    let zero_skipped_hours = expected_hours
        .map(|expected| summary.unique_frame_hours.len() == expected)
        .unwrap_or(false);
    let one_frame_per_hour = summary.unique_frame_hours.iter().all(|hour| {
        summary
            .per_hour_frame_records
            .get(hour)
            .copied()
            .unwrap_or(0)
            == 1
    });
    let one_depth_record_per_hour = summary.unique_frame_hours.iter().all(|hour| {
        summary
            .per_hour_depth_records
            .get(hour)
            .copied()
            .unwrap_or(0)
            == 1
    });
    let failure_status_flags = summary.status_flags_or & FAIL_STATUS_MASK;
    let ordered = summary.ordering_errors.is_empty();
    let decodable = summary.parse_errors.is_empty();
    let hourly_or_lower = zero_skipped_hours && one_frame_per_hour && one_depth_record_per_hour;
    println!(
        concat!(
            "{{\n",
            "  \"status\": \"{}\",\n",
            "  \"errors\": {},\n",
            "  \"domain_id\": {},\n",
            "  \"domain_name\": \"army_battleplans_depth_live\",\n",
            "  \"min_game_hours\": {},\n",
            "  \"records\": {},\n",
            "  \"frame_end_records\": {},\n",
            "  \"depth_data_records\": {},\n",
            "  \"first_hour\": {},\n",
            "  \"last_hour\": {},\n",
            "  \"game_hours_advanced\": {},\n",
            "  \"unique_frame_hours\": {},\n",
            "  \"zero_skipped_hours\": {},\n",
            "  \"one_frame_per_hour\": {},\n",
            "  \"one_depth_record_per_hour\": {},\n",
            "  \"hourly_or_lower\": {},\n",
            "  \"ordered\": {},\n",
            "  \"decodable\": {},\n",
            "  \"rows\": {},\n",
            "  \"changed_rows\": {},\n",
            "  \"target_rows\": {},\n",
            "  \"path_geometry_rows\": {},\n",
            "  \"non_type5_exact_path_rows\": {},\n",
            "  \"unsupported_geometry_rows\": {},\n",
            "  \"execution_unknown_rows\": {},\n",
            "  \"assigned_units_exact_rows\": {},\n",
            "  \"order_linkage_exact_rows\": {},\n",
            "  \"stance_unknown_rows\": {},\n",
            "  \"aggressiveness_unknown_rows\": {},\n",
            "  \"direct_root_rows\": {},\n",
            "  \"direct_root_member_rows\": {},\n",
            "  \"direct_army_rows\": {},\n",
            "  \"direct_army_group_rows\": {},\n",
            "  \"direct_theatre_rows\": {},\n",
            "  \"direct_cold_preflight_rows\": {},\n",
            "  \"status_flags_or\": {},\n",
            "  \"failure_status_flags\": {}\n",
            "}}\n"
        ),
        if errors.is_empty() { "PASS" } else { "FAIL" },
        json_string_array(errors),
        DOMAIN_ARMY_BATTLEPLANS_DEPTH,
        min_game_hours,
        summary.record_count,
        summary.frame_end_count,
        summary.totals.data_records,
        json_opt_i32(summary.first_hour),
        json_opt_i32(summary.last_hour),
        json_opt_i32(advanced),
        summary.unique_frame_hours.len(),
        zero_skipped_hours,
        one_frame_per_hour,
        one_depth_record_per_hour,
        hourly_or_lower,
        ordered,
        decodable,
        summary.totals.rows,
        summary.totals.changed_rows,
        summary.totals.target_rows,
        summary.totals.path_geometry_rows,
        summary.totals.non_type5_exact_path_rows,
        summary.totals.unsupported_geometry_rows,
        summary.totals.execution_unknown_rows,
        summary.totals.assigned_units_exact_rows,
        summary.totals.order_linkage_exact_rows,
        summary.totals.stance_unknown_rows,
        summary.totals.aggressiveness_unknown_rows,
        summary.totals.direct_root_rows,
        summary.totals.direct_root_member_rows,
        summary.totals.direct_army_rows,
        summary.totals.direct_army_group_rows,
        summary.totals.direct_theatre_rows,
        summary.totals.direct_cold_preflight_rows,
        summary.status_flags_or,
        failure_status_flags,
    );
}

#[cfg(not(test))]
fn json_opt_i32(value: Option<i32>) -> String {
    value
        .map(|inner| inner.to_string())
        .unwrap_or_else(|| "null".to_string())
}

#[cfg(not(test))]
fn json_string_array(values: &[String]) -> String {
    let parts = values
        .iter()
        .map(|value| format!("\"{}\"", json_escape(value)))
        .collect::<Vec<_>>();
    format!("[{}]", parts.join(", "))
}

#[cfg(not(test))]
fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04X}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
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
                    path_provinces: Vec::new(),
                    target_declared_count: Some(2),
                    target_status: STATUS_EXACT,
                    geometry_status: STATUS_EXACT,
                    execution_status: STATUS_FIXTURE_REQUIRED,
                    assigned_units_status: STATUS_EXACT,
                    order_linkage_status: STATUS_EXACT,
                    stance_status: STATUS_UNKNOWN,
                    aggressiveness_status: STATUS_UNKNOWN,
                },
                BattleplanDepthRow {
                    army_id: 18,
                    generation: 10,
                    deleted: false,
                    slot_index: 1,
                    order_type: 2,
                    target_states: Vec::new(),
                    path_provinces: Vec::new(),
                    target_declared_count: None,
                    target_status: STATUS_UNSUPPORTED,
                    geometry_status: STATUS_FIXTURE_REQUIRED,
                    execution_status: STATUS_FIXTURE_REQUIRED,
                    assigned_units_status: STATUS_EXACT,
                    order_linkage_status: STATUS_EXACT,
                    stance_status: STATUS_UNKNOWN,
                    aggressiveness_status: STATUS_UNKNOWN,
                },
            ],
            &[DirectRootRow {
                kind: DIRECT_KIND_ARMY,
                object_id: 17,
                deleted: false,
                parent_id: Some(301),
                owner_tag_db_id: Some(47),
                member_ids: vec![101, 102],
                engine_member_count: Some(3),
                missing_member_count: Some(1),
                order_count: Some(2),
                non_type5_order_count: Some(1),
                parent_status: STATUS_EXACT,
                member_status: STATUS_EXACT,
                root_status: STATUS_COLD_PREFLIGHT_ONLY,
            }],
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
        assert_eq!(summary.totals.path_geometry_rows, 0);
        assert_eq!(summary.totals.non_type5_exact_path_rows, 0);
        assert_eq!(summary.totals.unsupported_geometry_rows, 1);
        assert_eq!(summary.totals.execution_unknown_rows, 2);
        assert_eq!(summary.totals.assigned_units_exact_rows, 2);
        assert_eq!(summary.totals.order_linkage_exact_rows, 2);
        assert_eq!(summary.totals.stance_unknown_rows, 2);
        assert_eq!(summary.totals.aggressiveness_unknown_rows, 2);
        assert_eq!(summary.totals.direct_root_rows, 1);
        assert_eq!(summary.totals.direct_root_member_rows, 2);
        assert_eq!(summary.totals.direct_army_rows, 1);
        assert_eq!(summary.totals.direct_cold_preflight_rows, 1);
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
                path_provinces: Vec::new(),
                target_declared_count: Some(0),
                target_status: STATUS_UNSUPPORTED,
                geometry_status: STATUS_UNKNOWN,
                execution_status: STATUS_FIXTURE_REQUIRED,
                assigned_units_status: STATUS_EXACT,
                order_linkage_status: STATUS_EXACT,
                stance_status: STATUS_UNKNOWN,
                aggressiveness_status: STATUS_UNKNOWN,
            }],
            &[],
            11,
            0,
            1,
            1,
            1,
        );
        let mut summary = GateSummary::default();
        decode_depth_payload(0, &payload, &mut summary);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("type-5 row 0 is not exact")));
    }

    #[test]
    fn encodes_and_decodes_non_type5_exact_path_geometry_rows() {
        let payload = encode_depth_payload(
            &[BattleplanDepthRow {
                army_id: 19,
                generation: 11,
                deleted: false,
                slot_index: 2,
                order_type: 3,
                target_states: Vec::new(),
                path_provinces: vec![1001, 1002, 1003],
                target_declared_count: None,
                target_status: STATUS_UNSUPPORTED,
                geometry_status: STATUS_EXACT,
                execution_status: STATUS_FIXTURE_REQUIRED,
                assigned_units_status: STATUS_EXACT,
                order_linkage_status: STATUS_EXACT,
                stance_status: STATUS_UNKNOWN,
                aggressiveness_status: STATUS_UNKNOWN,
            }],
            &[],
            11,
            0,
            1,
            1,
            1,
        );
        let mut summary = GateSummary::default();
        decode_depth_payload(0, &payload, &mut summary);
        assert_eq!(summary.parse_errors, Vec::<String>::new());
        assert_eq!(summary.totals.rows, 1);
        assert_eq!(summary.totals.target_rows, 0);
        assert_eq!(summary.totals.path_geometry_rows, 3);
        assert_eq!(summary.totals.non_type5_exact_path_rows, 1);
        assert_eq!(summary.totals.unsupported_geometry_rows, 0);
    }

    #[test]
    fn stream_rejects_noncanonical_sequence_flags_and_lengths() {
        let payload = encode_depth_payload(&[], &[], 0, 0, 0, 0, 0);
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            2,
            100,
            DOMAIN_ARMY_BATTLEPLANS_DEPTH,
            RECORD_TYPE_DOMAIN_DATA,
            1 << 31,
            &payload,
        );
        append_record(
            &mut stream,
            3,
            100,
            DOMAIN_ARMY_BATTLEPLANS_DEPTH,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &[0; 39],
        );
        append_record(
            &mut stream,
            4,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &[0; 31],
        );

        let summary = decode_compact_stream(&stream);
        assert!(summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("expected 1")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("record flags")));
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
    fn stream_accepts_unowned_records_and_perf_before_data() {
        let payload = encode_depth_payload(&[], &[], 0, 0, 0, 0, 0);
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            1,
            100,
            51,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &[0; 40],
        );
        append_record(
            &mut stream,
            2,
            100,
            51,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            &[0xA5; 7],
        );
        append_record(
            &mut stream,
            3,
            100,
            DOMAIN_ARMY_BATTLEPLANS_DEPTH,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &[0; 40],
        );
        append_record(
            &mut stream,
            4,
            100,
            DOMAIN_ARMY_BATTLEPLANS_DEPTH,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_record(
            &mut stream,
            5,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &[0; FRAME_END_PAYLOAD_LEN],
        );

        let errors = gate_depth_stream(&stream, 0);
        assert_eq!(errors, Vec::<String>::new(), "{errors:?}");
    }

    #[test]
    fn stream_rejects_nonconsecutive_physical_closures() {
        let payload = encode_depth_payload(&[], &[], 0, 0, 0, 0, 0);
        let mut stream = Vec::new();
        let mut sequence = 1u64;
        for hour in [100i32, 99, 101] {
            append_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_ARMY_BATTLEPLANS_DEPTH,
                RECORD_TYPE_DOMAIN_DATA,
                0,
                &payload,
            );
            sequence += 1;
            append_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                0,
                &[0; FRAME_END_PAYLOAD_LEN],
            );
            sequence += 1;
        }

        let errors = gate_depth_stream(&stream, 0);
        assert!(errors
            .iter()
            .any(|error| error.contains("closed hour discontinuity: expected 101, got 99")));
    }

    #[test]
    fn stream_uses_full_canonical_failure_mask_after_normalization() {
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
            status_for_global_interpretation(DOMAIN_ARMY_BATTLEPLANS_DEPTH, 1 << 12),
            1 << 12
        );

        let payload = encode_depth_payload(&[], &[], 0, fatal, 0, 0, 0);
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            1,
            100,
            DOMAIN_ARMY_BATTLEPLANS_DEPTH,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_record(
            &mut stream,
            2,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &[0; FRAME_END_PAYLOAD_LEN],
        );
        let errors = gate_depth_stream(&stream, 0);
        assert!(errors
            .iter()
            .any(|error| error.contains("failure status flags")));
    }

    #[test]
    fn stream_validates_announced_perf_trailer_envelopes() {
        fn stream_with_perf(flags: u32, perf: &[u8]) -> GateSummary {
            let payload = encode_depth_payload(&[], &[], 0, 0, 0, 0, 0);
            let mut stream = Vec::new();
            append_record(
                &mut stream,
                1,
                100,
                DOMAIN_ARMY_BATTLEPLANS_DEPTH,
                RECORD_TYPE_DOMAIN_DATA,
                0,
                &payload,
            );
            append_record(
                &mut stream,
                2,
                100,
                DOMAIN_ARMY_BATTLEPLANS_DEPTH,
                RECORD_TYPE_DOMAIN_PERF,
                flags,
                perf,
            );
            append_record(
                &mut stream,
                3,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                0,
                &[0; FRAME_END_PAYLOAD_LEN],
            );
            decode_compact_stream(&stream)
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
    fn stream_rejects_hour_mismatch_and_unclean_tail() {
        let payload = encode_depth_payload(&[], &[], 0, 0, 0, 0, 0);
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            1,
            100,
            DOMAIN_ARMY_BATTLEPLANS_DEPTH,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        append_record(
            &mut stream,
            2,
            101,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &[0; FRAME_END_PAYLOAD_LEN],
        );
        append_record(
            &mut stream,
            3,
            101,
            DOMAIN_ARMY_BATTLEPLANS_DEPTH,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &[0; 40],
        );

        let summary = decode_compact_stream(&stream);
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

    fn append_record(
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

    fn empty_pdu1_perf_payload() -> Vec<u8> {
        let mut payload = vec![0; 40];
        payload.extend_from_slice(b"PDU1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }
}
