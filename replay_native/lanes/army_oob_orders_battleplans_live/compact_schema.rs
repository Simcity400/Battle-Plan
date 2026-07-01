//! Lane-local compact schema and gate for army orders plus land OOB command rows.
//!
//! This mirrors the currently emitted compact domains without importing the
//! shared recorder host. It is intended for isolated review before any shared
//! integration patch.

use std::collections::{BTreeMap, BTreeSet};

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
pub const RECORD_TYPE_FRAME_END: u16 = 2;

pub const DOMAIN_ARMY_ORDERS: u16 = 27;
pub const DOMAIN_OOB_COMMAND: u16 = 28;
pub const DOMAIN_FRAME: u16 = 1000;

const STATUS_WORKER_PANIC: u32 = 1 << 6;
const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
const STATUS_PARTIAL_DATA: u32 = 1 << 9;
const FAIL_STATUS_MASK: u32 = STATUS_WORKER_PANIC
    | STATUS_BRIDGE_UNAVAILABLE
    | STATUS_SOURCE_UNAVAILABLE
    | STATUS_PARTIAL_DATA;

const ARMY_ORDER_ROW_LEN: usize = 48;
const ARMY_ORDER_TARGET_ROW_LEN: usize = 8;
const ARMY_ORDER_FLAG_DELETED: u32 = 1;
const ARMY_ORDER_FLAG_TARGET_STATES: u32 = 1 << 1;
const ARMY_ORDER_FLAG_TARGET_STATE_COUNT: u32 = 1 << 2;
const MAX_ARMY_ID: u32 = i32::MAX as u32;
const MAX_ARMY_ORDER_SLOT_INDEX: u32 = 64;
const MAX_STATE_ID: u32 = 65_535;

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
const MAX_OOB_LOCATION_PROVINCE_ID: u32 = 1_000_000;
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
            if summary.per_hour_frame_records.get(hour).copied().unwrap_or(0) != 1 {
                errors.push(format!("game_hour_raw {hour}: expected one frame-end record"));
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
    let mut offset = 0usize;
    let mut last_sequence = None::<u64>;

    while offset < bytes.len() {
        if bytes.len().saturating_sub(offset) < COMPACT_RECORD_HEADER_LEN {
            summary
                .parse_errors
                .push(format!("offset {offset}: truncated compact record header"));
            break;
        }
        let header = &bytes[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            summary
                .parse_errors
                .push(format!("offset {offset}: bad compact record magic"));
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
            summary
                .parse_errors
                .push(format!("offset {offset}: unsupported compact version {version}"));
        }
        if header_len != COMPACT_RECORD_HEADER_LEN {
            summary
                .parse_errors
                .push(format!("offset {offset}: bad header length {header_len}"));
        }
        if let Some(last) = last_sequence {
            if sequence <= last {
                summary.ordering_errors.push(format!(
                    "offset {offset}: sequence {sequence} is not greater than {last}"
                ));
            }
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
        summary.record_count = summary.record_count.saturating_add(1);

        match record_type {
            RECORD_TYPE_FRAME_END => {
                summary.frame_end_count = summary.frame_end_count.saturating_add(1);
                summary.first_hour = Some(summary.first_hour.map_or(game_hour_raw, |v| v.min(game_hour_raw)));
                summary.last_hour = Some(summary.last_hour.map_or(game_hour_raw, |v| v.max(game_hour_raw)));
                summary.unique_frame_hours.insert(game_hour_raw);
                *summary.per_hour_frame_records.entry(game_hour_raw).or_default() += 1;
                if domain_id != DOMAIN_FRAME {
                    summary.parse_errors.push(format!(
                        "offset {offset}: frame-end record used domain {domain_id}"
                    ));
                }
            }
            RECORD_TYPE_DOMAIN_DATA => {
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
            _ => {}
        }

        offset += record_len;
    }

    summary
}

pub fn decode_army_orders_payload(offset: usize, payload: &[u8], summary: &mut CompactGateSummary) {
    if payload.len() < 40 {
        summary
            .parse_errors
            .push(format!("offset {offset}: army_orders payload shorter than 40 bytes"));
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
        summary
            .parse_errors
            .push(format!("offset {offset}: army_orders changed rows exceed row count"));
    }
    if observed_army_rows > root_rows {
        summary
            .parse_errors
            .push(format!("offset {offset}: observed army rows exceed root rows"));
    }
    if deleted_rows > row_count {
        summary
            .parse_errors
            .push(format!("offset {offset}: army_orders deleted rows exceed row count"));
    }

    let domain = summary.per_domain.entry(DOMAIN_ARMY_ORDERS).or_default();
    domain.data_records = domain.data_records.saturating_add(1);
    domain.rows = domain.rows.saturating_add(row_count as u64);
    domain.changed_rows = domain.changed_rows.saturating_add(changed_rows as u64);
    domain.child_rows = domain.child_rows.saturating_add(target_state_count as u64);
    domain.deleted_rows = domain.deleted_rows.saturating_add(deleted_rows as u64);
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
        let reserved0 = read_u32(payload, cursor + 40);
        let reserved1 = read_u32(payload, cursor + 44);
        cursor += ARMY_ORDER_ROW_LEN;

        let deleted = flags & ARMY_ORDER_FLAG_DELETED != 0;
        let has_target_states = flags & ARMY_ORDER_FLAG_TARGET_STATES != 0;
        let has_target_count = flags & ARMY_ORDER_FLAG_TARGET_STATE_COUNT != 0;
        if army_id == 0 || army_id > MAX_ARMY_ID {
            summary
                .parse_errors
                .push(format!("offset {offset}: army order row {row_index} has bad army id"));
        }
        if !seen.insert((army_id, generation, slot_index, deleted)) {
            summary
                .parse_errors
                .push(format!("offset {offset}: duplicate army order row {row_index}"));
        }
        if flags & !(ARMY_ORDER_FLAG_DELETED | ARMY_ORDER_FLAG_TARGET_STATES | ARMY_ORDER_FLAG_TARGET_STATE_COUNT) != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: army order row {row_index} has unknown flags"));
        }
        if slot_index >= MAX_ARMY_ORDER_SLOT_INDEX {
            summary
                .parse_errors
                .push(format!("offset {offset}: army order row {row_index} has bad slot"));
        }
        if !(-1000..=1000).contains(&order_type) {
            summary
                .parse_errors
                .push(format!("offset {offset}: army order row {row_index} has bad order type"));
        }
        if deleted && (target_first != 0 || target_count != 0 || target_declared != 0 || target_status != 0) {
            summary
                .parse_errors
                .push(format!("offset {offset}: deleted army order row {row_index} has live target fields"));
        }
        if !deleted && target_status > 4 {
            summary
                .parse_errors
                .push(format!("offset {offset}: army order row {row_index} has bad target status"));
        }
        if !has_target_states && target_count != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: army order row {row_index} has target rows without flag"));
        }
        if has_target_states {
            let target_end = target_first.saturating_add(target_count);
            if target_end > target_state_count {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: army order row {row_index} target range exceeds child table"));
            }
            declared_targets = declared_targets.saturating_add(target_count);
        } else if target_first != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: army order row {row_index} has target_first without flag"));
        }
        if !has_target_count && target_declared != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: army order row {row_index} has target count without flag"));
        }
        if reserved0 != 0 || reserved1 != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: army order row {row_index} has nonzero reserved field"));
        }
    }
    if declared_targets != target_state_count {
        summary
            .parse_errors
            .push(format!("offset {offset}: army_orders target child count mismatch"));
    }
    for row_index in 0..target_state_count {
        let state_id = read_u32(payload, cursor);
        let reserved = read_u32(payload, cursor + 4);
        cursor += ARMY_ORDER_TARGET_ROW_LEN;
        if state_id == 0 || state_id > MAX_STATE_ID {
            summary
                .parse_errors
                .push(format!("offset {offset}: target row {row_index} has bad state id"));
        }
        if reserved != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: target row {row_index} has nonzero reserved field"));
        }
    }
    let _ = observed_order_rows;
}

pub fn decode_oob_command_payload(offset: usize, payload: &[u8], summary: &mut CompactGateSummary) {
    if payload.len() < 48 {
        summary
            .parse_errors
            .push(format!("offset {offset}: oob_command payload shorter than 48 bytes"));
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
        summary
            .parse_errors
            .push(format!("offset {offset}: oob changed rows exceed row count"));
    }
    if observed_army_rows > root_rows {
        summary
            .parse_errors
            .push(format!("offset {offset}: oob observed army rows exceed root rows"));
    }
    if deleted_rows > row_count {
        summary
            .parse_errors
            .push(format!("offset {offset}: oob deleted rows exceed row count"));
    }

    let domain = summary.per_domain.entry(DOMAIN_OOB_COMMAND).or_default();
    domain.data_records = domain.data_records.saturating_add(1);
    domain.rows = domain.rows.saturating_add(row_count as u64);
    domain.changed_rows = domain.changed_rows.saturating_add(changed_rows as u64);
    domain.child_rows = domain.child_rows.saturating_add(member_count as u64);
    domain.deleted_rows = domain.deleted_rows.saturating_add(deleted_rows as u64);
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
        if !matches!(kind, OOB_KIND_ARMY | OOB_KIND_ARMY_GROUP | OOB_KIND_FLEET | OOB_KIND_TASK_FORCE) {
            summary
                .parse_errors
                .push(format!("offset {offset}: oob row {row_index} has bad kind"));
        }
        if object_id == 0 || object_id > MAX_OOB_ID {
            summary
                .parse_errors
                .push(format!("offset {offset}: oob row {row_index} has bad object id"));
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
            summary
                .parse_errors
                .push(format!("offset {offset}: oob row {row_index} has unknown flags"));
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
            summary
                .parse_errors
                .push(format!("offset {offset}: deleted oob row {row_index} has live fields"));
        }
        validate_optional_id(summary, offset, row_index, flags & OOB_FLAG_PARENT != 0, parent_id, "parent");
        validate_optional_id(summary, offset, row_index, flags & OOB_FLAG_OWNER != 0, owner_tag_db_id, "owner");
        validate_optional_id(summary, offset, row_index, flags & OOB_FLAG_LEADER != 0, leader_id, "leader");
        if flags & OOB_FLAG_LEADER_SKILL != 0 {
            if leader_skill > MAX_OOB_LEADER_SKILL {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: oob row {row_index} has bad leader skill"));
            }
        } else if leader_skill != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: oob row {row_index} has leader skill without flag"));
        }
        if flags & OOB_FLAG_LOCATION != 0 {
            if location_province_id == 0 || location_province_id > MAX_OOB_LOCATION_PROVINCE_ID {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: oob row {row_index} has bad location"));
            }
        } else if location_province_id != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: oob row {row_index} has location without flag"));
        }
        if flags & OOB_FLAG_MEMBER_COUNT_ENGINE == 0 && member_count_engine != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: oob row {row_index} has engine count without flag"));
        }
        if flags & OOB_FLAG_MISSING_MEMBER_COUNT == 0 && missing_member_count != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: oob row {row_index} has missing count without flag"));
        }
        for (name, status) in [
            ("parent", parent_status),
            ("member", member_status),
            ("owner", owner_status),
            ("leader", leader_status),
            ("leader_trait", leader_trait_status),
        ] {
            if status > 5 {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: oob row {row_index} has bad {name} status"));
            }
        }
        if row_member_count > 0 {
            let member_end = member_first.saturating_add(row_member_count);
            if member_end > member_count {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: oob row {row_index} member range exceeds child table"));
            }
            declared_members = declared_members.saturating_add(row_member_count);
        } else if member_first != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: oob row {row_index} has member_first without members"));
        }
        if logical_member_count < row_member_count {
            summary
                .parse_errors
                .push(format!("offset {offset}: oob row {row_index} logical member count is too small"));
        }
        if reserved != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: oob row {row_index} has nonzero reserved field"));
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
            summary
                .parse_errors
                .push(format!("offset {offset}: oob member row {row_index} has bad id"));
        }
        if reserved != 0 {
            summary
                .parse_errors
                .push(format!("offset {offset}: oob member row {row_index} has nonzero reserved field"));
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
    fn army_orders_payload_rejects_bad_target_reserved_field() {
        let mut payload = test_army_orders_payload(0);
        let target_reserved_offset = 40 + 2 * ARMY_ORDER_ROW_LEN + 4;
        payload[target_reserved_offset..target_reserved_offset + 4].copy_from_slice(&1u32.to_le_bytes());

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
            append_record(&mut raw, sequence, hour, DOMAIN_FRAME, RECORD_TYPE_FRAME_END, &[]);
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
        append_record(&mut raw, 3, 100, DOMAIN_FRAME, RECORD_TYPE_FRAME_END, &[]);

        let result = gate_lane_stream(&raw, 0);

        assert!(result
            .errors
            .iter()
            .any(|err| err.contains("failure status flags")));
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
        out.extend_from_slice(COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
    }
}
