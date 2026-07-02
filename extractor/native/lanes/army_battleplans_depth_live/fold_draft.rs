//! Standalone fold draft for domain 54.
//!
//! This file is deliberately not wired into any build. It uses only `std` so a
//! serial worker can compile it in isolation with `rustc --test fold_draft.rs`
//! or move the logic into `recorder_host/src/world_fold.rs` and convert the
//! snapshot structs to `serde_json::Value`.

use std::collections::BTreeMap;
use std::convert::TryInto;

const HEADER_LEN: usize = 52;
const ROW_LEN: usize = 64;
const CHILD_ROW_LEN: usize = 8;

const FLAG_DELETED: u32 = 1;
const FLAG_TARGET_STATES: u32 = 1 << 1;
const FLAG_TARGET_STATE_COUNT: u32 = 1 << 2;
const FLAG_PATH_GEOMETRY: u32 = 1 << 5;

const STATUS_EXACT: u32 = 1;
const STATUS_UNKNOWN: u32 = 2;
const STATUS_UNSUPPORTED: u32 = 3;
const STATUS_FIXTURE_REQUIRED: u32 = 6;

const COMPACT_STATUS_DELTA_DATA: u32 = 1 << 10;
const COMPACT_STATUS_FAILURE_MASK: u32 = (1 << 0)
    | (1 << 1)
    | (1 << 2)
    | (1 << 3)
    | (1 << 6)
    | (1 << 7)
    | (1 << 8)
    | (1 << 9)
    | (1 << 12);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BattleplanOrderKey {
    pub army_id: u32,
    pub generation: u64,
    pub slot_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattleplanOrder {
    pub key: BattleplanOrderKey,
    pub order_type: i32,
    pub target_states: Vec<u32>,
    pub path_provinces: Vec<u32>,
    pub target_declared_count: Option<u32>,
    pub target_status: &'static str,
    pub geometry_status: &'static str,
    pub execution_status: &'static str,
    pub assigned_units_status: &'static str,
    pub order_linkage_status: &'static str,
    pub stance_status: &'static str,
    pub aggressiveness_status: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedDepthPayload {
    pub status_flags: u32,
    pub is_delta: bool,
    pub rows: Vec<DecodedDepthRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedDepthRow {
    pub deleted: bool,
    pub order: BattleplanOrder,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BattleplansDepthFold {
    baseline_seen: bool,
    unknown_this_hour: bool,
    orders: BTreeMap<BattleplanOrderKey, BattleplanOrder>,
    notes_this_hour: Vec<String>,
    domain_problems_this_hour: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattleplansDepthExtraSnapshot {
    pub known: bool,
    pub baseline_seen: bool,
    pub orders: Vec<BattleplanOrder>,
    pub notes: Vec<String>,
    pub domain_problems: Vec<String>,
}

impl BattleplansDepthFold {
    pub fn apply_domain54_payload(&mut self, payload: &[u8]) -> Result<(), String> {
        let decoded = decode_domain54_payload(payload)?;
        if compact_status_is_failure(decoded.status_flags) {
            self.unknown_this_hour = true;
            self.notes_this_hour.push(format!(
                "army_battleplans_depth_live failure status {:#x}; battleplan depth state unknown this hour",
                decoded.status_flags & COMPACT_STATUS_FAILURE_MASK
            ));
            return Ok(());
        }
        if decoded.is_delta && !self.baseline_seen {
            self.unknown_this_hour = true;
            self.domain_problems_this_hour
                .push("delta_before_baseline".to_string());
            self.notes_this_hour.push(
                "army_battleplans_depth_live delta arrived before a full baseline; rows ignored this hour"
                    .to_string(),
            );
            return Ok(());
        }
        if !decoded.is_delta {
            self.orders.clear();
            self.baseline_seen = true;
        }
        for row in decoded.rows {
            if row.deleted {
                self.orders.remove(&row.order.key);
            } else {
                self.orders.insert(row.order.key.clone(), row.order);
            }
        }
        self.unknown_this_hour = false;
        Ok(())
    }

    pub fn extra_snapshot(&self) -> BattleplansDepthExtraSnapshot {
        BattleplansDepthExtraSnapshot {
            known: !self.unknown_this_hour && self.baseline_seen,
            baseline_seen: self.baseline_seen,
            orders: self.orders.values().cloned().collect(),
            notes: self.notes_this_hour.clone(),
            domain_problems: self.domain_problems_this_hour.clone(),
        }
    }

    pub fn reset_hour_flags(&mut self) {
        self.unknown_this_hour = false;
        self.notes_this_hour.clear();
        self.domain_problems_this_hour.clear();
    }
}

pub fn decode_domain54_payload(payload: &[u8]) -> Result<DecodedDepthPayload, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!("domain 54 payload shorter than {HEADER_LEN} bytes"));
    }
    let row_count = read_u32(payload, 8) as usize;
    let status_flags = read_u32(payload, 16);
    let target_count = read_u32(payload, 28) as usize;
    let direct_root_count = read_u32(payload, 40) as usize;
    let direct_member_count = read_u32(payload, 44) as usize;
    let path_count = read_u32(payload, 48) as usize;
    if direct_root_count != 0 || direct_member_count != 0 {
        return Err("domain 54 direct-root rows are outside this validated fold slice".to_string());
    }
    let expected = HEADER_LEN
        .saturating_add(row_count.saturating_mul(ROW_LEN))
        .saturating_add(target_count.saturating_mul(CHILD_ROW_LEN))
        .saturating_add(path_count.saturating_mul(CHILD_ROW_LEN));
    if payload.len() != expected {
        return Err(format!("domain 54 payload len {} != {expected}", payload.len()));
    }

    let target_table_start = HEADER_LEN + row_count * ROW_LEN;
    let path_table_start = target_table_start + target_count * CHILD_ROW_LEN;
    let mut rows = Vec::with_capacity(row_count);
    let mut declared_targets = 0usize;
    let mut declared_paths = 0usize;
    let mut cursor = HEADER_LEN;
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
        let has_targets = flags & FLAG_TARGET_STATES != 0;
        let has_path = flags & FLAG_PATH_GEOMETRY != 0;
        if has_targets && has_path {
            return Err(format!("domain 54 row {row_index} has both target and path children"));
        }
        if !deleted && assigned_units_status != STATUS_EXACT {
            return Err(format!("domain 54 row {row_index} lacks exact assigned-unit status"));
        }
        if !deleted && order_linkage_status != STATUS_EXACT {
            return Err(format!("domain 54 row {row_index} lacks exact order-linkage status"));
        }
        let target_states = if has_targets {
            let end = child_first.saturating_add(child_count);
            if end > target_count {
                return Err(format!("domain 54 row {row_index} target range exceeds child table"));
            }
            declared_targets = declared_targets.saturating_add(child_count);
            read_child_table(payload, target_table_start, child_first, child_count)
        } else {
            Vec::new()
        };
        let path_provinces = if has_path {
            let end = child_first.saturating_add(child_count);
            if end > path_count {
                return Err(format!("domain 54 row {row_index} path range exceeds child table"));
            }
            if child_count == 0 || geometry_status != STATUS_EXACT {
                return Err(format!("domain 54 row {row_index} has invalid path geometry"));
            }
            declared_paths = declared_paths.saturating_add(child_count);
            read_child_table(payload, path_table_start, child_first, child_count)
        } else {
            Vec::new()
        };
        let target_declared_count =
            (flags & FLAG_TARGET_STATE_COUNT != 0).then_some(target_declared);
        rows.push(DecodedDepthRow {
            deleted,
            order: BattleplanOrder {
                key: BattleplanOrderKey {
                    army_id,
                    generation,
                    slot_index,
                },
                order_type,
                target_states,
                path_provinces,
                target_declared_count,
                target_status: status_name(target_status),
                geometry_status: status_name(geometry_status),
                execution_status: status_name(execution_status),
                assigned_units_status: status_name(assigned_units_status),
                order_linkage_status: status_name(order_linkage_status),
                stance_status: status_name(stance_status),
                aggressiveness_status: status_name(aggressiveness_status),
            },
        });
    }
    if declared_targets != target_count {
        return Err("domain 54 target child count mismatch".to_string());
    }
    if declared_paths != path_count {
        return Err("domain 54 path child count mismatch".to_string());
    }
    Ok(DecodedDepthPayload {
        status_flags,
        is_delta: status_flags & COMPACT_STATUS_DELTA_DATA != 0,
        rows,
    })
}

fn read_child_table(payload: &[u8], table_start: usize, first: usize, count: usize) -> Vec<u32> {
    (0..count)
        .map(|index| read_u32(payload, table_start + (first + index) * CHILD_ROW_LEN))
        .collect()
}

fn compact_status_is_failure(status_flags: u32) -> bool {
    status_flags & COMPACT_STATUS_FAILURE_MASK != 0
}

fn status_name(status: u32) -> &'static str {
    match status {
        STATUS_EXACT => "exact",
        STATUS_UNKNOWN => "unknown",
        STATUS_UNSUPPORTED => "unsupported",
        STATUS_FIXTURE_REQUIRED => "fixture_required",
        _ => "none",
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(rows: &[TestRow], status_flags: u32) -> Vec<u8> {
        let target_count: usize = rows.iter().map(|row| row.targets.len()).sum();
        let path_count: usize = rows.iter().map(|row| row.path.len()).sum();
        let mut out = Vec::new();
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&status_flags.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(target_count as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(path_count as u32).to_le_bytes());
        let mut target_cursor = 0u32;
        let mut path_cursor = 0u32;
        for row in rows {
            let mut flags = 0u32;
            if row.deleted {
                flags |= FLAG_DELETED;
            }
            if !row.targets.is_empty() {
                flags |= FLAG_TARGET_STATES | FLAG_TARGET_STATE_COUNT;
            }
            if !row.path.is_empty() {
                flags |= FLAG_PATH_GEOMETRY;
            }
            let child_first = if !row.targets.is_empty() {
                target_cursor
            } else if !row.path.is_empty() {
                path_cursor
            } else {
                0
            };
            let child_count = if !row.targets.is_empty() {
                row.targets.len() as u32
            } else {
                row.path.len() as u32
            };
            out.extend_from_slice(&row.army_id.to_le_bytes());
            out.extend_from_slice(&row.generation.to_le_bytes());
            for value in [
                flags,
                row.slot_index,
                row.order_type as u32,
                child_first,
                child_count,
                row.targets.len() as u32,
                if row.order_type == 5 { STATUS_EXACT } else { STATUS_UNSUPPORTED },
                STATUS_EXACT,
                STATUS_FIXTURE_REQUIRED,
                STATUS_EXACT,
                STATUS_EXACT,
                STATUS_UNKNOWN,
                STATUS_UNKNOWN,
            ] {
                out.extend_from_slice(&value.to_le_bytes());
            }
            target_cursor += row.targets.len() as u32;
            path_cursor += row.path.len() as u32;
        }
        for row in rows {
            for value in &row.targets {
                out.extend_from_slice(&value.to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        for row in rows {
            for value in &row.path {
                out.extend_from_slice(&value.to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        out
    }

    struct TestRow {
        army_id: u32,
        generation: u64,
        slot_index: u32,
        order_type: i32,
        targets: Vec<u32>,
        path: Vec<u32>,
        deleted: bool,
    }

    #[test]
    fn baseline_then_delta_updates_extra_snapshot() {
        let baseline = payload(
            &[
                TestRow {
                    army_id: 10,
                    generation: 1,
                    slot_index: 0,
                    order_type: 5,
                    targets: vec![44, 45],
                    path: Vec::new(),
                    deleted: false,
                },
                TestRow {
                    army_id: 10,
                    generation: 2,
                    slot_index: 1,
                    order_type: 3,
                    targets: Vec::new(),
                    path: vec![101, 102],
                    deleted: false,
                },
            ],
            0,
        );
        let mut fold = BattleplansDepthFold::default();
        fold.apply_domain54_payload(&baseline).unwrap();
        assert_eq!(fold.extra_snapshot().orders.len(), 2);

        let delta = payload(
            &[TestRow {
                army_id: 10,
                generation: 2,
                slot_index: 1,
                order_type: 3,
                targets: Vec::new(),
                path: Vec::new(),
                deleted: true,
            }],
            COMPACT_STATUS_DELTA_DATA,
        );
        fold.reset_hour_flags();
        fold.apply_domain54_payload(&delta).unwrap();
        let snapshot = fold.extra_snapshot();
        assert!(snapshot.known);
        assert_eq!(snapshot.orders.len(), 1);
        assert_eq!(snapshot.orders[0].target_states, vec![44, 45]);
    }

    #[test]
    fn delta_before_baseline_is_machine_visible_and_does_not_apply() {
        let delta = payload(
            &[TestRow {
                army_id: 10,
                generation: 1,
                slot_index: 0,
                order_type: 5,
                targets: vec![44],
                path: Vec::new(),
                deleted: false,
            }],
            COMPACT_STATUS_DELTA_DATA,
        );
        let mut fold = BattleplansDepthFold::default();
        fold.apply_domain54_payload(&delta).unwrap();
        let snapshot = fold.extra_snapshot();
        assert!(!snapshot.known);
        assert!(snapshot.orders.is_empty());
        assert_eq!(snapshot.domain_problems, vec!["delta_before_baseline"]);
    }

    #[test]
    fn failure_status_retains_previous_accumulator() {
        let baseline = payload(
            &[TestRow {
                army_id: 10,
                generation: 1,
                slot_index: 0,
                order_type: 5,
                targets: vec![44],
                path: Vec::new(),
                deleted: false,
            }],
            0,
        );
        let failure = payload(&[], 1 << 8);
        let mut fold = BattleplansDepthFold::default();
        fold.apply_domain54_payload(&baseline).unwrap();
        fold.reset_hour_flags();
        fold.apply_domain54_payload(&failure).unwrap();
        let snapshot = fold.extra_snapshot();
        assert!(!snapshot.known);
        assert_eq!(snapshot.orders.len(), 1);
        assert!(snapshot.notes[0].contains("failure status"));
    }
}
