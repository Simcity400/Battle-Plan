//! Lane-local proposed compact schema for direct land-OOB roots and theatres.
//!
//! This is not wired into the shared collector. It defines the additive payload
//! shape needed once a bounded direct-root production slot is granted.

use std::collections::BTreeSet;

pub const DOMAIN_DIRECT_LAND_OOB: u16 = 44;
pub const HEADER_LEN: usize = 32;
pub const ROW_LEN: usize = 64;
pub const MEMBER_ROW_LEN: usize = 8;

pub const KIND_ARMY: u32 = 1;
pub const KIND_ARMY_GROUP: u32 = 2;
pub const KIND_THEATRE: u32 = 5;

pub const FLAG_DELETED: u32 = 1;
pub const FLAG_PARENT: u32 = 1 << 1;
pub const FLAG_OWNER: u32 = 1 << 2;
pub const FLAG_ENGINE_MEMBER_COUNT: u32 = 1 << 3;
pub const FLAG_MISSING_MEMBER_COUNT: u32 = 1 << 4;
pub const FLAG_MEMBER_IDS: u32 = 1 << 5;
pub const FLAG_ORDER_COUNTS: u32 = 1 << 6;

pub const STATUS_NONE: u32 = 0;
pub const STATUS_EXACT: u32 = 1;
pub const STATUS_UNKNOWN: u32 = 2;
pub const STATUS_OVERFLOW: u32 = 3;
pub const STATUS_VALIDATION_FAILED: u32 = 4;
pub const STATUS_COLD_PREFLIGHT_ONLY: u32 = 5;

const MAX_ID: u32 = 4_000_000;
const MAX_MEMBER_ROWS: usize = 250_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectLandOobRow {
    pub kind: u32,
    pub object_id: u32,
    pub deleted: bool,
    pub parent_id: Option<u32>,
    pub owner_tag_db_id: Option<u32>,
    pub member_ids: Option<Vec<u32>>,
    pub engine_member_count: Option<u32>,
    pub missing_member_count: Option<u32>,
    pub order_count: Option<u32>,
    pub non_type5_order_count: Option<u32>,
    pub parent_status: u32,
    pub member_status: u32,
    pub root_status: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeSummary {
    pub row_count: usize,
    pub changed_rows: usize,
    pub member_rows: usize,
    pub deleted_rows: usize,
    pub army_rows: usize,
    pub army_group_rows: usize,
    pub theatre_rows: usize,
    pub status_flags: u32,
    pub parse_errors: Vec<String>,
}

pub fn encode_direct_land_oob_payload(
    rows: &[DirectLandOobRow],
    elapsed_wall_ms: u64,
    status_flags: u32,
    changed_rows: u32,
) -> Vec<u8> {
    let member_count = rows
        .iter()
        .map(|row| row.member_ids.as_ref().map(Vec::len).unwrap_or(0))
        .sum::<usize>();
    let deleted_count = rows.iter().filter(|row| row.deleted).count().min(u32::MAX as usize) as u32;
    let mut out = Vec::with_capacity(HEADER_LEN + rows.len() * ROW_LEN + member_count * MEMBER_ROW_LEN);
    out.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    out.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&changed_rows.to_le_bytes());
    out.extend_from_slice(&status_flags.to_le_bytes());
    out.extend_from_slice(&(member_count.min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&deleted_count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    let mut member_cursor = 0u32;
    for row in rows {
        let row_member_count = row
            .member_ids
            .as_ref()
            .map(Vec::len)
            .unwrap_or(0)
            .min(u32::MAX as usize) as u32;
        let mut flags = 0u32;
        if row.deleted {
            flags |= FLAG_DELETED;
        }
        if row.parent_id.is_some() {
            flags |= FLAG_PARENT;
        }
        if row.owner_tag_db_id.is_some() {
            flags |= FLAG_OWNER;
        }
        if row.engine_member_count.is_some() {
            flags |= FLAG_ENGINE_MEMBER_COUNT;
        }
        if row.missing_member_count.is_some() {
            flags |= FLAG_MISSING_MEMBER_COUNT;
        }
        if row_member_count > 0 {
            flags |= FLAG_MEMBER_IDS;
        }
        if row.order_count.is_some() || row.non_type5_order_count.is_some() {
            flags |= FLAG_ORDER_COUNTS;
        }
        let member_first = if row_member_count == 0 { 0 } else { member_cursor };
        for value in [
            row.kind,
            row.object_id,
            flags,
            row.parent_id.unwrap_or(0),
            row.owner_tag_db_id.unwrap_or(0),
            member_first,
            row_member_count,
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
        member_cursor = member_cursor.saturating_add(row_member_count);
    }
    for row in rows {
        if let Some(member_ids) = row.member_ids.as_ref() {
            for member_id in member_ids {
                out.extend_from_slice(&member_id.to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
            }
        }
    }
    out
}

pub fn decode_direct_land_oob_payload(payload: &[u8]) -> DecodeSummary {
    let mut summary = DecodeSummary {
        row_count: 0,
        changed_rows: 0,
        member_rows: 0,
        deleted_rows: 0,
        army_rows: 0,
        army_group_rows: 0,
        theatre_rows: 0,
        status_flags: 0,
        parse_errors: Vec::new(),
    };
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push("payload shorter than 32-byte header".to_string());
        return summary;
    }
    let row_count = read_u32(payload, 8) as usize;
    let changed_rows = read_u32(payload, 12) as usize;
    let status_flags = read_u32(payload, 16);
    let member_count = read_u32(payload, 20) as usize;
    let deleted_rows = read_u32(payload, 24) as usize;
    let expected_len = HEADER_LEN
        .saturating_add(row_count.saturating_mul(ROW_LEN))
        .saturating_add(member_count.saturating_mul(MEMBER_ROW_LEN));
    summary.row_count = row_count;
    summary.changed_rows = changed_rows;
    summary.member_rows = member_count;
    summary.deleted_rows = deleted_rows;
    summary.status_flags = status_flags;
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "payload length {} != expected {expected_len}",
            payload.len()
        ));
        return summary;
    }
    if changed_rows > row_count {
        summary.parse_errors.push("changed rows exceed row count".to_string());
    }
    if deleted_rows > row_count {
        summary.parse_errors.push("deleted rows exceed row count".to_string());
    }
    if member_count > MAX_MEMBER_ROWS {
        summary.parse_errors.push("member rows exceed limit".to_string());
    }

    let mut cursor = HEADER_LEN;
    let mut declared_members = 0usize;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let kind = read_u32(payload, cursor);
        let object_id = read_u32(payload, cursor + 4);
        let flags = read_u32(payload, cursor + 8);
        let parent_id = read_u32(payload, cursor + 12);
        let owner_tag_db_id = read_u32(payload, cursor + 16);
        let member_first = read_u32(payload, cursor + 20) as usize;
        let row_member_count = read_u32(payload, cursor + 24) as usize;
        let engine_count = read_u32(payload, cursor + 28);
        let missing_count = read_u32(payload, cursor + 32);
        let order_count = read_u32(payload, cursor + 36);
        let non_type5_order_count = read_u32(payload, cursor + 40);
        let parent_status = read_u32(payload, cursor + 44);
        let member_status = read_u32(payload, cursor + 48);
        let root_status = read_u32(payload, cursor + 52);
        let reserved0 = read_u32(payload, cursor + 56);
        let reserved1 = read_u32(payload, cursor + 60);
        cursor += ROW_LEN;

        match kind {
            KIND_ARMY => summary.army_rows += 1,
            KIND_ARMY_GROUP => summary.army_group_rows += 1,
            KIND_THEATRE => summary.theatre_rows += 1,
            _ => summary.parse_errors.push(format!("row {row_index} has bad kind {kind}")),
        }
        if object_id == 0 || object_id > MAX_ID {
            summary.parse_errors.push(format!("row {row_index} has bad object id"));
        }
        if !seen.insert((kind, object_id, flags & FLAG_DELETED != 0)) {
            summary.parse_errors.push(format!("duplicate row {row_index}"));
        }
        let known = FLAG_DELETED
            | FLAG_PARENT
            | FLAG_OWNER
            | FLAG_ENGINE_MEMBER_COUNT
            | FLAG_MISSING_MEMBER_COUNT
            | FLAG_MEMBER_IDS
            | FLAG_ORDER_COUNTS;
        if flags & !known != 0 {
            summary.parse_errors.push(format!("row {row_index} has unknown flags"));
        }
        validate_optional(&mut summary.parse_errors, row_index, flags & FLAG_PARENT != 0, parent_id, "parent_id");
        validate_optional(&mut summary.parse_errors, row_index, flags & FLAG_OWNER != 0, owner_tag_db_id, "owner_tag_db_id");
        validate_optional(&mut summary.parse_errors, row_index, flags & FLAG_ENGINE_MEMBER_COUNT != 0, engine_count, "engine_count");
        validate_optional(&mut summary.parse_errors, row_index, flags & FLAG_MISSING_MEMBER_COUNT != 0, missing_count, "missing_count");
        if flags & FLAG_ORDER_COUNTS == 0 && (order_count != 0 || non_type5_order_count != 0) {
            summary.parse_errors.push(format!("row {row_index} has order counts without flag"));
        }
        if non_type5_order_count > order_count {
            summary.parse_errors.push(format!("row {row_index} non_type5 count exceeds order count"));
        }
        for (name, status) in [
            ("parent", parent_status),
            ("member", member_status),
            ("root", root_status),
        ] {
            if status > STATUS_COLD_PREFLIGHT_ONLY {
                summary.parse_errors.push(format!("row {row_index} has bad {name} status"));
            }
        }
        if flags & FLAG_MEMBER_IDS != 0 {
            let member_end = member_first.saturating_add(row_member_count);
            if member_end > member_count {
                summary.parse_errors.push(format!("row {row_index} member range exceeds child table"));
            }
            declared_members = declared_members.saturating_add(row_member_count);
        } else if member_first != 0 || row_member_count != 0 {
            summary.parse_errors.push(format!("row {row_index} has member fields without flag"));
        }
        if reserved0 != 0 || reserved1 != 0 {
            summary.parse_errors.push(format!("row {row_index} has nonzero reserved field"));
        }
    }
    if declared_members != member_count {
        summary.parse_errors.push("declared member count does not match child table".to_string());
    }
    for row_index in 0..member_count {
        let member_id = read_u32(payload, cursor);
        let reserved = read_u32(payload, cursor + 4);
        cursor += MEMBER_ROW_LEN;
        if member_id == 0 || member_id > MAX_ID {
            summary.parse_errors.push(format!("member row {row_index} has bad id"));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!("member row {row_index} has nonzero reserved"));
        }
    }
    summary
}

fn validate_optional(errors: &mut Vec<String>, row_index: usize, present: bool, value: u32, name: &str) {
    if present {
        if value == 0 || value > MAX_ID {
            errors.push(format!("row {row_index} has bad {name}"));
        }
    } else if value != 0 {
        errors.push(format!("row {row_index} has {name} without flag"));
    }
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

#[cfg(not(test))]
fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: direct_oob_compact <direct_land_oob_payload.bin>");
        std::process::exit(2);
    };
    let payload = match std::fs::read(&path) {
        Ok(payload) => payload,
        Err(err) => {
            eprintln!("status=FAIL read_error={err}");
            std::process::exit(2);
        }
    };
    let summary = decode_direct_land_oob_payload(&payload);
    if summary.parse_errors.is_empty() {
        println!(
            "status=PASS rows={} armies={} army_groups={} theatres={} member_rows={} status_flags=0x{:X}",
            summary.row_count,
            summary.army_rows,
            summary.army_group_rows,
            summary.theatre_rows,
            summary.member_rows,
            summary.status_flags,
        );
    } else {
        println!(
            "status=FAIL rows={} armies={} army_groups={} theatres={} member_rows={} errors={:?}",
            summary.row_count,
            summary.army_rows,
            summary.army_group_rows,
            summary.theatre_rows,
            summary.member_rows,
            summary.parse_errors,
        );
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_direct_army_group_and_theatre_rows() {
        let rows = vec![
            DirectLandOobRow {
                kind: KIND_ARMY,
                object_id: 17,
                deleted: false,
                parent_id: Some(300),
                owner_tag_db_id: Some(47),
                member_ids: Some(vec![101, 102]),
                engine_member_count: Some(3),
                missing_member_count: Some(1),
                order_count: Some(2),
                non_type5_order_count: Some(1),
                parent_status: STATUS_EXACT,
                member_status: STATUS_EXACT,
                root_status: STATUS_COLD_PREFLIGHT_ONLY,
            },
            DirectLandOobRow {
                kind: KIND_ARMY_GROUP,
                object_id: 300,
                deleted: false,
                parent_id: None,
                owner_tag_db_id: Some(47),
                member_ids: Some(vec![17]),
                engine_member_count: None,
                missing_member_count: None,
                order_count: None,
                non_type5_order_count: None,
                parent_status: STATUS_NONE,
                member_status: STATUS_EXACT,
                root_status: STATUS_COLD_PREFLIGHT_ONLY,
            },
            DirectLandOobRow {
                kind: KIND_THEATRE,
                object_id: 900,
                deleted: false,
                parent_id: None,
                owner_tag_db_id: None,
                member_ids: None,
                engine_member_count: None,
                missing_member_count: None,
                order_count: None,
                non_type5_order_count: None,
                parent_status: STATUS_NONE,
                member_status: STATUS_UNKNOWN,
                root_status: STATUS_COLD_PREFLIGHT_ONLY,
            },
        ];

        let payload = encode_direct_land_oob_payload(&rows, 10, 0, 3);
        let summary = decode_direct_land_oob_payload(&payload);

        assert_eq!(summary.parse_errors, Vec::<String>::new());
        assert_eq!(summary.row_count, 3);
        assert_eq!(summary.member_rows, 3);
        assert_eq!(summary.army_rows, 1);
        assert_eq!(summary.army_group_rows, 1);
        assert_eq!(summary.theatre_rows, 1);
    }

    #[test]
    fn rejects_bad_member_range() {
        let mut payload = encode_direct_land_oob_payload(
            &[DirectLandOobRow {
                kind: KIND_ARMY,
                object_id: 17,
                deleted: false,
                parent_id: None,
                owner_tag_db_id: None,
                member_ids: Some(vec![101]),
                engine_member_count: None,
                missing_member_count: None,
                order_count: None,
                non_type5_order_count: None,
                parent_status: STATUS_NONE,
                member_status: STATUS_EXACT,
                root_status: STATUS_COLD_PREFLIGHT_ONLY,
            }],
            10,
            0,
            1,
        );
        payload[HEADER_LEN + 20..HEADER_LEN + 24].copy_from_slice(&7u32.to_le_bytes());

        let summary = decode_direct_land_oob_payload(&payload);

        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("member range exceeds child table")));
    }
}
