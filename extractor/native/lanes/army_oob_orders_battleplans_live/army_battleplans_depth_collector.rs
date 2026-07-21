//! Lane-owned collector/extractor contract for army_battleplans_depth_live.
//!
//! This is the module shape to move behind the DLL collector registration hook
//! when an integration slot is granted. It contains no shared entrypoint edits.

pub const DOMAIN_ARMY_BATTLEPLANS_DEPTH: u16 = 54;

pub const ARMY_VTABLE_RVA: u64 = 0x293BC60;
pub const ORDER_VTABLE_RVA: u64 = 0x296FAE8;
pub const ARMY_OFF_ORDERS_VEC: u64 = 0x98;
pub const ORDER_OFF_TYPE: u64 = 0x30;
pub const ORDER_OFF_ARMY_BACKREF: u64 = 0x38;
pub const ORDER_OFF_PATH_BEGIN: u64 = 0x70;
pub const ORDER_OFF_PATH_SIZE: u64 = 0x7C;
pub const ORDER_OFF_TYPE5_STATES_BEGIN: u64 = 0xE0;
pub const ORDER_OFF_TYPE5_STATES_SIZE: u64 = 0xEC;

pub const STATUS_NONE: u32 = 0;
pub const STATUS_EXACT: u32 = 1;
pub const STATUS_UNKNOWN: u32 = 2;
pub const STATUS_UNSUPPORTED: u32 = 3;
pub const STATUS_FIXTURE_REQUIRED: u32 = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawOrderObservation {
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
    pub assigned_units_status: u32,
    pub order_linkage_status: u32,
    pub stance_status: u32,
    pub aggressiveness_status: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepthCollectorRow {
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DepthExtraction {
    pub rows: Vec<DepthCollectorRow>,
    pub type5_exact_rows: u32,
    pub type5_non_exact_omitted_rows: u32,
    pub non_type5_exact_path_rows: u32,
    pub path_geometry_rows: u32,
    pub non_type5_fixture_blocked_rows: u32,
    pub execution_fixture_blocked_rows: u32,
    pub assigned_units_exact_rows: u32,
    pub order_linkage_exact_rows: u32,
    pub stance_unknown_rows: u32,
    pub aggressiveness_unknown_rows: u32,
}

pub fn extract_validated_depth_subset(source: &[RawOrderObservation]) -> DepthExtraction {
    let mut out = DepthExtraction::default();
    for row in source {
        if row.deleted {
            out.rows.push(DepthCollectorRow {
                army_id: row.army_id,
                generation: row.generation,
                deleted: true,
                slot_index: row.slot_index,
                order_type: row.order_type,
                target_states: Vec::new(),
                path_provinces: Vec::new(),
                target_declared_count: None,
                target_status: STATUS_NONE,
                geometry_status: STATUS_NONE,
                execution_status: STATUS_NONE,
                assigned_units_status: STATUS_NONE,
                order_linkage_status: STATUS_NONE,
                stance_status: STATUS_NONE,
                aggressiveness_status: STATUS_NONE,
            });
            continue;
        }
        if row.order_type == 5 && row.target_status != STATUS_EXACT {
            out.type5_non_exact_omitted_rows = out.type5_non_exact_omitted_rows.saturating_add(1);
            continue;
        }
        let (target_status, geometry_status, path_provinces) = if row.order_type == 5 {
            out.type5_exact_rows = out.type5_exact_rows.saturating_add(1);
            (STATUS_EXACT, STATUS_EXACT, Vec::new())
        } else if row.geometry_status == STATUS_EXACT && !row.path_provinces.is_empty() {
            out.non_type5_exact_path_rows = out.non_type5_exact_path_rows.saturating_add(1);
            out.path_geometry_rows = out
                .path_geometry_rows
                .saturating_add(row.path_provinces.len().min(u32::MAX as usize) as u32);
            (STATUS_UNSUPPORTED, STATUS_EXACT, row.path_provinces.clone())
        } else {
            out.non_type5_fixture_blocked_rows = out.non_type5_fixture_blocked_rows.saturating_add(1);
            (STATUS_UNSUPPORTED, STATUS_FIXTURE_REQUIRED, Vec::new())
        };
        out.execution_fixture_blocked_rows = out.execution_fixture_blocked_rows.saturating_add(1);
        if row.assigned_units_status == STATUS_EXACT {
            out.assigned_units_exact_rows = out.assigned_units_exact_rows.saturating_add(1);
        }
        if row.order_linkage_status == STATUS_EXACT {
            out.order_linkage_exact_rows = out.order_linkage_exact_rows.saturating_add(1);
        }
        if row.stance_status == STATUS_UNKNOWN {
            out.stance_unknown_rows = out.stance_unknown_rows.saturating_add(1);
        }
        if row.aggressiveness_status == STATUS_UNKNOWN {
            out.aggressiveness_unknown_rows = out.aggressiveness_unknown_rows.saturating_add(1);
        }
        out.rows.push(DepthCollectorRow {
            army_id: row.army_id,
            generation: row.generation,
            deleted: false,
            slot_index: row.slot_index,
            order_type: row.order_type,
            target_states: row.target_states.clone(),
            path_provinces,
            target_declared_count: row.target_declared_count,
            target_status,
            geometry_status,
            execution_status: STATUS_FIXTURE_REQUIRED,
            assigned_units_status: row.assigned_units_status,
            order_linkage_status: row.order_linkage_status,
            stance_status: row.stance_status,
            aggressiveness_status: row.aggressiveness_status,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_exact_type5_and_blocks_non_type5() {
        let result = extract_validated_depth_subset(&[
            RawOrderObservation {
                army_id: 10,
                generation: 1,
                deleted: false,
                slot_index: 0,
                order_type: 5,
                target_states: vec![1, 2],
                path_provinces: Vec::new(),
                target_declared_count: Some(2),
                target_status: STATUS_EXACT,
                geometry_status: STATUS_EXACT,
                assigned_units_status: STATUS_EXACT,
                order_linkage_status: STATUS_EXACT,
                stance_status: STATUS_UNKNOWN,
                aggressiveness_status: STATUS_UNKNOWN,
            },
            RawOrderObservation {
                army_id: 11,
                generation: 2,
                deleted: false,
                slot_index: 0,
                order_type: 2,
                target_states: Vec::new(),
                path_provinces: Vec::new(),
                target_declared_count: None,
                target_status: STATUS_UNSUPPORTED,
                geometry_status: STATUS_FIXTURE_REQUIRED,
                assigned_units_status: STATUS_EXACT,
                order_linkage_status: STATUS_EXACT,
                stance_status: STATUS_UNKNOWN,
                aggressiveness_status: STATUS_UNKNOWN,
            },
        ]);
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.type5_exact_rows, 1);
        assert_eq!(result.non_type5_fixture_blocked_rows, 1);
        assert_eq!(result.execution_fixture_blocked_rows, 2);
        assert_eq!(result.assigned_units_exact_rows, 2);
        assert_eq!(result.order_linkage_exact_rows, 2);
        assert_eq!(result.stance_unknown_rows, 2);
        assert_eq!(result.aggressiveness_unknown_rows, 2);
        assert_eq!(result.rows[1].geometry_status, STATUS_FIXTURE_REQUIRED);
    }

    #[test]
    fn extracts_exact_non_type5_path_geometry() {
        let result = extract_validated_depth_subset(&[RawOrderObservation {
            army_id: 12,
            generation: 3,
            deleted: false,
            slot_index: 1,
            order_type: 3,
            target_states: Vec::new(),
            path_provinces: vec![101, 102, 103],
            target_declared_count: None,
            target_status: STATUS_UNSUPPORTED,
            geometry_status: STATUS_EXACT,
            assigned_units_status: STATUS_EXACT,
            order_linkage_status: STATUS_EXACT,
            stance_status: STATUS_UNKNOWN,
            aggressiveness_status: STATUS_UNKNOWN,
        }]);
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.non_type5_exact_path_rows, 1);
        assert_eq!(result.path_geometry_rows, 3);
        assert_eq!(result.non_type5_fixture_blocked_rows, 0);
        assert_eq!(result.rows[0].geometry_status, STATUS_EXACT);
        assert_eq!(result.rows[0].path_provinces, vec![101, 102, 103]);
    }

    #[test]
    fn omits_non_exact_type5_fail_closed() {
        let result = extract_validated_depth_subset(&[RawOrderObservation {
            army_id: 10,
            generation: 1,
            deleted: false,
            slot_index: 0,
            order_type: 5,
            target_states: Vec::new(),
            path_provinces: Vec::new(),
            target_declared_count: Some(0),
            target_status: STATUS_UNSUPPORTED,
            geometry_status: STATUS_FIXTURE_REQUIRED,
            assigned_units_status: STATUS_EXACT,
            order_linkage_status: STATUS_EXACT,
            stance_status: STATUS_UNKNOWN,
            aggressiveness_status: STATUS_UNKNOWN,
        }]);
        assert_eq!(result.rows.len(), 0);
        assert_eq!(result.type5_non_exact_omitted_rows, 1);
    }
}
