//! Lane-local compact schema for `army_battleplans_depth_live`.
//!
//! This crate is intentionally self-contained and is not registered in the
//! shared native workspace by this packet. The serial integration worker can
//! move these types behind the collector/recorder registration hooks.

pub const DOMAIN_ID: u16 = 54;
pub const DOMAIN_NAME: &str = "army_battleplans_depth_live";

pub const ARMY_VTABLE_RVA: u64 = compact_contract::build_profile::DOMAIN54_ARMY_VTABLE_RVA as u64;
pub const ORDER_VTABLE_RVA: u64 = compact_contract::build_profile::DOMAIN54_ORDER_VTABLE_RVA as u64;
pub const ARMY_OFF_ORDERS_VEC: u64 =
    compact_contract::build_profile::DOMAIN54_ARMY_OFF_ORDERS_VEC as u64;
pub const ORDER_OFF_TYPE: u64 = compact_contract::build_profile::DOMAIN54_ORDER_OFF_TYPE as u64;
pub const ORDER_OFF_ARMY_BACKREF: u64 =
    compact_contract::build_profile::DOMAIN54_ORDER_OFF_ARMY_BACKREF as u64;
pub const ORDER_OFF_PATH_BEGIN: u64 =
    compact_contract::build_profile::DOMAIN54_ORDER_OFF_PATH_BEGIN as u64;
pub const ORDER_OFF_PATH_SIZE: u64 =
    compact_contract::build_profile::DOMAIN54_ORDER_OFF_PATH_SIZE as u64;
pub const ORDER_OFF_TYPE5_STATES_BEGIN: u64 =
    compact_contract::build_profile::DOMAIN54_ORDER_OFF_TYPE5_STATES_BEGIN as u64;
pub const ORDER_OFF_TYPE5_STATES_SIZE: u64 =
    compact_contract::build_profile::DOMAIN54_ORDER_OFF_TYPE5_STATES_SIZE as u64;

pub const HEADER_LEN: usize = 52;
pub const ROW_LEN: usize = 64;
pub const TARGET_ROW_LEN: usize = 8;
pub const PATH_GEOMETRY_ROW_LEN: usize = 8;

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

pub const COMPACT_STATUS_DELTA_DATA: u32 = compact_contract::COMPACT_STATUS_DELTA_DATA;
pub const COMPACT_STATUS_PARTIAL_DATA: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
pub const COMPACT_STATUS_FAILURE_MASK: u32 = compact_contract::COMPACT_STATUS_FAILURE_MASK;

const ORDER_TYPE5_OFFENSIVE: i32 = 5;
const MAX_CHILD_ROWS: u32 = 5_000;
const MAX_INLINE_CHILD_ROWS: u32 = 256;
const MAX_STATE_ID: u32 = 65_535;
const MAX_PROVINCE_ID: u32 = 65_535;
const MAX_ARMY_ID: u32 = i32::MAX as u32;
const MAX_SLOT_INDEX: u32 = 64;

#[derive(Default)]
pub struct BattleplansDepthTracker {
    pub baseline_seen: bool,
}

pub trait BattleplansDepthMemory {
    fn module_base(&self) -> u64;
    fn read_u64(&self, address: u64) -> Option<u64>;
    fn read_u32(&self, address: u64) -> Option<u32>;
    fn read_i32(&self, address: u64) -> Option<i32>;
    fn read_bytes(&self, address: u64, len: usize) -> Option<Vec<u8>>;
    fn is_heap_pointer(&self, address: u64) -> bool;
}

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DepthExtraction {
    pub rows: Vec<BattleplanDepthRow>,
    pub type5_exact_rows: u32,
    pub type5_non_exact_omitted_rows: u32,
    pub assignment_or_linkage_omitted_rows: u32,
    pub path_geometry_omitted_rows: u32,
    pub non_type5_exact_path_rows: u32,
    pub path_geometry_rows: u32,
    pub non_type5_fixture_blocked_rows: u32,
    pub execution_fixture_blocked_rows: u32,
    pub assigned_units_exact_rows: u32,
    pub order_linkage_exact_rows: u32,
    pub stance_unknown_rows: u32,
    pub aggressiveness_unknown_rows: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EncodedDepthRecord {
    pub domain_id: u16,
    pub payload: Vec<u8>,
    pub emitted_rows: u32,
    pub extraction: DepthExtraction,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecodeTotals {
    pub data_records: u64,
    pub rows: u64,
    pub changed_rows: u64,
    pub target_rows: u64,
    pub path_geometry_rows: u64,
    pub non_type5_exact_path_rows: u64,
    pub unsupported_geometry_rows: u64,
    pub execution_fixture_required_rows: u64,
    pub assigned_units_exact_rows: u64,
    pub order_linkage_exact_rows: u64,
    pub stance_unknown_rows: u64,
    pub aggressiveness_unknown_rows: u64,
    pub omitted_non_exact_rows: u64,
    pub path_geometry_omitted_rows: u64,
    pub status_flags_or: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecodeSummary {
    pub totals: DecodeTotals,
    pub parse_errors: Vec<String>,
}

pub trait CompactDomainSink {
    fn emit_domain_data(&mut self, domain_id: u16, payload: &[u8]);
}

pub fn compact_status_is_failure(status_flags: u32) -> bool {
    compact_contract::compact_status_is_failure(status_flags)
}

pub fn status_code_from_name(name: &str) -> u32 {
    match name {
        "exact" => STATUS_EXACT,
        "unknown" => STATUS_UNKNOWN,
        "unsupported" => STATUS_UNSUPPORTED,
        "overflow" => STATUS_OVERFLOW,
        "validation_failed" => STATUS_VALIDATION_FAILED,
        "fixture_required" => STATUS_FIXTURE_REQUIRED,
        "cold_preflight_only" => STATUS_COLD_PREFLIGHT_ONLY,
        _ => STATUS_NONE,
    }
}

pub fn validated_order_observation<M: BattleplansDepthMemory>(
    mem: &M,
    army_id: u32,
    generation: u64,
    army_body: u64,
    order_ptr: u64,
    slot_index: u32,
    assigned_units_status: u32,
    order_linkage_status: u32,
) -> Option<RawOrderObservation> {
    let order_vtable = mem.module_base().checked_add(ORDER_VTABLE_RVA)?;
    if !mem.is_heap_pointer(order_ptr) || mem.read_u64(order_ptr)? != order_vtable {
        return None;
    }
    if mem.read_u64(order_ptr.checked_add(ORDER_OFF_ARMY_BACKREF)?)? != army_body {
        return None;
    }
    let order_type = mem.read_i32(order_ptr.checked_add(ORDER_OFF_TYPE)?)?;
    let (target_states, target_declared_count, target_status) =
        read_type5_target_states(mem, order_ptr, order_type);
    let (path_provinces, geometry_status) =
        read_non_type5_path_provinces(mem, order_ptr, order_type);
    Some(RawOrderObservation {
        army_id,
        generation,
        deleted: false,
        slot_index,
        order_type,
        target_states,
        path_provinces,
        target_declared_count,
        target_status,
        geometry_status,
        assigned_units_status,
        order_linkage_status,
        stance_status: STATUS_UNKNOWN,
        aggressiveness_status: STATUS_UNKNOWN,
    })
}

pub fn read_type5_target_states<M: BattleplansDepthMemory>(
    mem: &M,
    order_ptr: u64,
    order_type: i32,
) -> (Vec<u32>, Option<u32>, u32) {
    if order_type != ORDER_TYPE5_OFFENSIVE {
        return (Vec::new(), None, STATUS_UNSUPPORTED);
    }
    read_u32_vector(
        mem,
        order_ptr,
        ORDER_OFF_TYPE5_STATES_BEGIN,
        ORDER_OFF_TYPE5_STATES_SIZE,
        MAX_CHILD_ROWS,
        MAX_INLINE_CHILD_ROWS,
        MAX_STATE_ID,
    )
    .map(|(values, count)| (values, Some(count), STATUS_EXACT))
    .unwrap_or_else(|status| (Vec::new(), None, status))
}

pub fn read_non_type5_path_provinces<M: BattleplansDepthMemory>(
    mem: &M,
    order_ptr: u64,
    order_type: i32,
) -> (Vec<u32>, u32) {
    if order_type == ORDER_TYPE5_OFFENSIVE {
        return (Vec::new(), STATUS_EXACT);
    }
    read_u32_vector(
        mem,
        order_ptr,
        ORDER_OFF_PATH_BEGIN,
        ORDER_OFF_PATH_SIZE,
        MAX_CHILD_ROWS,
        MAX_INLINE_CHILD_ROWS,
        MAX_PROVINCE_ID,
    )
    .map(|(values, _)| (values, STATUS_EXACT))
    .unwrap_or_else(|status| (Vec::new(), status))
}

pub fn extract_validated_depth_subset(source: &[RawOrderObservation]) -> DepthExtraction {
    let mut out = DepthExtraction::default();
    for row in source {
        if row.deleted {
            out.rows.push(BattleplanDepthRow {
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
        if row.assigned_units_status != STATUS_EXACT || row.order_linkage_status != STATUS_EXACT {
            out.assignment_or_linkage_omitted_rows =
                out.assignment_or_linkage_omitted_rows.saturating_add(1);
            continue;
        }
        if row.order_type == ORDER_TYPE5_OFFENSIVE && row.target_status != STATUS_EXACT {
            out.type5_non_exact_omitted_rows = out.type5_non_exact_omitted_rows.saturating_add(1);
            continue;
        }
        let (target_status, geometry_status, path_provinces) =
            if row.order_type == ORDER_TYPE5_OFFENSIVE {
                out.type5_exact_rows = out.type5_exact_rows.saturating_add(1);
                (STATUS_EXACT, STATUS_EXACT, Vec::new())
            } else if row.geometry_status == STATUS_EXACT && !row.path_provinces.is_empty() {
                out.non_type5_exact_path_rows = out.non_type5_exact_path_rows.saturating_add(1);
                out.path_geometry_rows = out
                    .path_geometry_rows
                    .saturating_add(row.path_provinces.len().min(u32::MAX as usize) as u32);
                (STATUS_UNSUPPORTED, STATUS_EXACT, row.path_provinces.clone())
            } else if row.geometry_status == STATUS_OVERFLOW
                || row.geometry_status == STATUS_VALIDATION_FAILED
            {
                out.path_geometry_omitted_rows = out.path_geometry_omitted_rows.saturating_add(1);
                continue;
            } else {
                out.non_type5_fixture_blocked_rows =
                    out.non_type5_fixture_blocked_rows.saturating_add(1);
                (STATUS_UNSUPPORTED, STATUS_FIXTURE_REQUIRED, Vec::new())
            };
        out.execution_fixture_blocked_rows = out.execution_fixture_blocked_rows.saturating_add(1);
        out.assigned_units_exact_rows = out.assigned_units_exact_rows.saturating_add(1);
        out.order_linkage_exact_rows = out.order_linkage_exact_rows.saturating_add(1);
        if row.stance_status == STATUS_UNKNOWN {
            out.stance_unknown_rows = out.stance_unknown_rows.saturating_add(1);
        }
        if row.aggressiveness_status == STATUS_UNKNOWN {
            out.aggressiveness_unknown_rows = out.aggressiveness_unknown_rows.saturating_add(1);
        }
        out.rows.push(BattleplanDepthRow {
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

pub fn collect_and_encode_depth_record(
    observations: &[RawOrderObservation],
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_army_rows: u32,
    observed_order_rows: u32,
) -> EncodedDepthRecord {
    let extraction = extract_validated_depth_subset(observations);
    let emitted_rows = extraction.rows.len().min(u32::MAX as usize) as u32;
    let omitted_non_exact_rows = extraction
        .type5_non_exact_omitted_rows
        .saturating_add(extraction.assignment_or_linkage_omitted_rows);
    let payload = encode_depth_payload(
        &extraction.rows,
        elapsed_wall_ms,
        status_flags,
        emitted_rows,
        observed_army_rows,
        observed_order_rows,
        omitted_non_exact_rows,
        extraction.path_geometry_omitted_rows,
    );
    EncodedDepthRecord {
        domain_id: DOMAIN_ID,
        payload,
        emitted_rows,
        extraction,
    }
}

pub fn collect_and_emit_depth_record(
    observations: &[RawOrderObservation],
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_army_rows: u32,
    observed_order_rows: u32,
    sink: &mut dyn CompactDomainSink,
) -> EncodedDepthRecord {
    let record = collect_and_encode_depth_record(
        observations,
        elapsed_wall_ms,
        status_flags,
        observed_army_rows,
        observed_order_rows,
    );
    sink.emit_domain_data(record.domain_id, &record.payload);
    record
}

pub fn encode_depth_payload(
    rows: &[BattleplanDepthRow],
    elapsed_wall_ms: u64,
    status_flags: u32,
    changed_rows: u32,
    observed_army_rows: u32,
    observed_order_rows: u32,
    omitted_non_exact_rows: u32,
    path_geometry_omitted_rows: u32,
) -> Vec<u8> {
    let target_count = rows
        .iter()
        .map(|row| row.target_states.len())
        .sum::<usize>();
    let path_count = rows
        .iter()
        .map(|row| row.path_provinces.len())
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
            + path_count * PATH_GEOMETRY_ROW_LEN,
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
    out.extend_from_slice(&omitted_non_exact_rows.to_le_bytes());
    out.extend_from_slice(&path_geometry_omitted_rows.to_le_bytes());
    out.extend_from_slice(&(path_count.min(u32::MAX as usize) as u32).to_le_bytes());

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
        debug_assert!(row.target_states.is_empty() || row.path_provinces.is_empty());
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
    out
}

pub fn decode_depth_payload(offset: usize, payload: &[u8]) -> DecodeSummary {
    let mut summary = DecodeSummary::default();
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: domain 54 payload shorter than {HEADER_LEN} bytes"
        ));
        return summary;
    }
    let row_count = read_u32(payload, 8) as usize;
    let changed_rows = read_u32(payload, 12) as usize;
    let status_flags = read_u32(payload, 16);
    let observed_army_rows = read_u32(payload, 20) as usize;
    let observed_order_rows = read_u32(payload, 24) as usize;
    let target_count = read_u32(payload, 28) as usize;
    let unsupported_geometry_rows = read_u32(payload, 32) as usize;
    let execution_unknown_rows = read_u32(payload, 36) as usize;
    let omitted_non_exact_rows = read_u32(payload, 40) as usize;
    let path_geometry_omitted_rows = read_u32(payload, 44) as usize;
    let path_count = read_u32(payload, 48) as usize;
    let expected = HEADER_LEN
        .saturating_add(row_count.saturating_mul(ROW_LEN))
        .saturating_add(target_count.saturating_mul(TARGET_ROW_LEN))
        .saturating_add(path_count.saturating_mul(PATH_GEOMETRY_ROW_LEN));
    if payload.len() != expected {
        summary.parse_errors.push(format!(
            "offset {offset}: domain 54 payload length {} != {expected}",
            payload.len()
        ));
        return summary;
    }
    if changed_rows > row_count {
        summary
            .parse_errors
            .push(format!("offset {offset}: changed rows exceed row count"));
    }
    if observed_order_rows < row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: observed order rows below emitted rows"
        ));
    }
    if observed_army_rows == 0 && row_count != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: emitted rows with zero observed armies"
        ));
    }
    summary.totals.data_records = 1;
    summary.totals.rows = row_count as u64;
    summary.totals.changed_rows = changed_rows as u64;
    summary.totals.target_rows = target_count as u64;
    summary.totals.path_geometry_rows = path_count as u64;
    summary.totals.unsupported_geometry_rows = unsupported_geometry_rows as u64;
    summary.totals.execution_fixture_required_rows = execution_unknown_rows as u64;
    summary.totals.omitted_non_exact_rows = omitted_non_exact_rows as u64;
    summary.totals.path_geometry_omitted_rows = path_geometry_omitted_rows as u64;
    summary.totals.status_flags_or = status_flags;

    let mut cursor = HEADER_LEN;
    let mut declared_targets = 0usize;
    let mut declared_paths = 0usize;
    for row_index in 0..row_count {
        let army_id = read_u32(payload, cursor);
        let _generation = read_u64(payload, cursor + 4);
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
        if army_id == 0 || army_id > MAX_ARMY_ID {
            summary
                .parse_errors
                .push(format!("offset {offset}: row {row_index} has bad army id"));
        }
        if slot_index >= MAX_SLOT_INDEX {
            summary
                .parse_errors
                .push(format!("offset {offset}: row {row_index} has bad slot"));
        }
        if flags
            & !(FLAG_DELETED
                | FLAG_TARGET_STATES
                | FLAG_TARGET_STATE_COUNT
                | FLAG_NON_TYPE5_GEOMETRY_BLOCKED
                | FLAG_EXECUTION_BLOCKED
                | FLAG_PATH_GEOMETRY)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has unknown flags"
            ));
        }
        if has_targets && has_path {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has both target and path children"
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
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has bad status code"
            ));
        }
        if deleted && (child_first != 0 || child_count != 0 || target_declared != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: deleted row {row_index} has child fields"
            ));
        }
        if !deleted && assigned_units_status != STATUS_EXACT {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} lacks exact assigned-unit status"
            ));
        }
        if !deleted && order_linkage_status != STATUS_EXACT {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} lacks exact order-linkage status"
            ));
        }
        if !deleted && order_type == ORDER_TYPE5_OFFENSIVE && target_status != STATUS_EXACT {
            summary.parse_errors.push(format!(
                "offset {offset}: type-5 row {row_index} is not exact"
            ));
        }
        if has_targets {
            if order_type != ORDER_TYPE5_OFFENSIVE {
                summary.parse_errors.push(format!(
                    "offset {offset}: non-type-5 row {row_index} has target children"
                ));
            }
            if child_first.saturating_add(child_count) > target_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: target range for row {row_index} exceeds child table"
                ));
            }
            declared_targets = declared_targets.saturating_add(child_count);
        }
        if has_path {
            if order_type == ORDER_TYPE5_OFFENSIVE {
                summary.parse_errors.push(format!(
                    "offset {offset}: type-5 row {row_index} has path geometry"
                ));
            }
            if child_count == 0 || child_first.saturating_add(child_count) > path_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: path range for row {row_index} is invalid"
                ));
            }
            if geometry_status != STATUS_EXACT {
                summary.parse_errors.push(format!(
                    "offset {offset}: row {row_index} has path geometry without exact geometry"
                ));
            }
            declared_paths = declared_paths.saturating_add(child_count);
            summary.totals.non_type5_exact_path_rows =
                summary.totals.non_type5_exact_path_rows.saturating_add(1);
        }
        if flags & FLAG_TARGET_STATE_COUNT == 0 && target_declared != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has target count without flag"
            ));
        }
        if execution_status == STATUS_FIXTURE_REQUIRED && !deleted {
            summary.totals.execution_fixture_required_rows = summary
                .totals
                .execution_fixture_required_rows
                .saturating_add(0);
        }
        if assigned_units_status == STATUS_EXACT && !deleted {
            summary.totals.assigned_units_exact_rows =
                summary.totals.assigned_units_exact_rows.saturating_add(1);
        }
        if order_linkage_status == STATUS_EXACT && !deleted {
            summary.totals.order_linkage_exact_rows =
                summary.totals.order_linkage_exact_rows.saturating_add(1);
        }
        if stance_status == STATUS_UNKNOWN && !deleted {
            summary.totals.stance_unknown_rows =
                summary.totals.stance_unknown_rows.saturating_add(1);
        }
        if aggressiveness_status == STATUS_UNKNOWN && !deleted {
            summary.totals.aggressiveness_unknown_rows =
                summary.totals.aggressiveness_unknown_rows.saturating_add(1);
        }
    }
    if declared_targets != target_count {
        summary
            .parse_errors
            .push(format!("offset {offset}: target child count mismatch"));
    }
    if declared_paths != path_count {
        summary
            .parse_errors
            .push(format!("offset {offset}: path child count mismatch"));
    }
    for child_index in 0..target_count {
        let state_id = read_u32(payload, cursor);
        let reserved = read_u32(payload, cursor + 4);
        cursor += TARGET_ROW_LEN;
        if state_id == 0 || state_id > MAX_STATE_ID || reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: target child row {child_index} is invalid"
            ));
        }
    }
    for child_index in 0..path_count {
        let province_id = read_u32(payload, cursor);
        let reserved = read_u32(payload, cursor + 4);
        cursor += PATH_GEOMETRY_ROW_LEN;
        if province_id == 0 || province_id > MAX_PROVINCE_ID || reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: path child row {child_index} is invalid"
            ));
        }
    }
    summary
}

fn read_u32_vector<M: BattleplansDepthMemory>(
    mem: &M,
    root: u64,
    begin_offset: u64,
    size_offset: u64,
    max_count: u32,
    max_inline: u32,
    max_value: u32,
) -> Result<(Vec<u32>, u32), u32> {
    let begin = mem
        .read_u64(
            root.checked_add(begin_offset)
                .ok_or(STATUS_VALIDATION_FAILED)?,
        )
        .ok_or(STATUS_VALIDATION_FAILED)?;
    let count = mem
        .read_u32(
            root.checked_add(size_offset)
                .ok_or(STATUS_VALIDATION_FAILED)?,
        )
        .ok_or(STATUS_VALIDATION_FAILED)?;
    if count == 0 {
        return Ok((Vec::new(), 0));
    }
    if count > max_count {
        return Err(STATUS_VALIDATION_FAILED);
    }
    if count > max_inline {
        return Err(STATUS_OVERFLOW);
    }
    if !mem.is_heap_pointer(begin) {
        return Err(STATUS_VALIDATION_FAILED);
    }
    let byte_len = (count as usize)
        .checked_mul(4)
        .ok_or(STATUS_VALIDATION_FAILED)?;
    let bytes = mem
        .read_bytes(begin, byte_len)
        .ok_or(STATUS_VALIDATION_FAILED)?;
    if bytes.len() != byte_len {
        return Err(STATUS_VALIDATION_FAILED);
    }
    let mut out = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        let start = index * 4;
        let value = u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap());
        if value == 0 || value > max_value {
            return Err(STATUS_VALIDATION_FAILED);
        }
        out.push(value);
    }
    Ok((out, count))
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
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct TestSink {
        domain_id: Option<u16>,
        payload: Vec<u8>,
    }

    impl CompactDomainSink for TestSink {
        fn emit_domain_data(&mut self, domain_id: u16, payload: &[u8]) {
            self.domain_id = Some(domain_id);
            self.payload = payload.to_vec();
        }
    }

    #[derive(Default)]
    struct TestMemory {
        module_base: u64,
        u64s: BTreeMap<u64, u64>,
        u32s: BTreeMap<u64, u32>,
        i32s: BTreeMap<u64, i32>,
        bytes: BTreeMap<u64, Vec<u8>>,
    }

    impl BattleplansDepthMemory for TestMemory {
        fn module_base(&self) -> u64 {
            self.module_base
        }

        fn read_u64(&self, address: u64) -> Option<u64> {
            self.u64s.get(&address).copied()
        }

        fn read_u32(&self, address: u64) -> Option<u32> {
            self.u32s.get(&address).copied()
        }

        fn read_i32(&self, address: u64) -> Option<i32> {
            self.i32s.get(&address).copied()
        }

        fn read_bytes(&self, address: u64, len: usize) -> Option<Vec<u8>> {
            let bytes = self.bytes.get(&address)?;
            let _ = len;
            Some(bytes.clone())
        }

        fn is_heap_pointer(&self, address: u64) -> bool {
            address >= 0x1000
        }
    }

    fn exact_order(
        army_id: u32,
        slot_index: u32,
        order_type: i32,
        target_states: Vec<u32>,
        path_provinces: Vec<u32>,
    ) -> RawOrderObservation {
        RawOrderObservation {
            army_id,
            generation: 1,
            deleted: false,
            slot_index,
            order_type,
            target_declared_count: (!target_states.is_empty())
                .then_some(target_states.len() as u32),
            target_states,
            path_provinces,
            target_status: if order_type == ORDER_TYPE5_OFFENSIVE {
                STATUS_EXACT
            } else {
                STATUS_UNSUPPORTED
            },
            geometry_status: STATUS_EXACT,
            assigned_units_status: STATUS_EXACT,
            order_linkage_status: STATUS_EXACT,
            stance_status: STATUS_UNKNOWN,
            aggressiveness_status: STATUS_UNKNOWN,
        }
    }

    #[test]
    fn encodes_and_decodes_validated_type5_and_path_geometry_rows() {
        let input = vec![
            exact_order(10, 0, 5, vec![44, 45], Vec::new()),
            exact_order(11, 1, 3, Vec::new(), vec![101, 102, 103]),
        ];
        let record = collect_and_encode_depth_record(&input, 77, 0, 2, 2);
        assert_eq!(record.domain_id, 54);
        assert_eq!(record.emitted_rows, 2);
        assert_eq!(record.extraction.type5_exact_rows, 1);
        assert_eq!(record.extraction.non_type5_exact_path_rows, 1);
        assert_eq!(record.extraction.path_geometry_rows, 3);

        let decoded = decode_depth_payload(0, &record.payload);
        assert_eq!(decoded.parse_errors, Vec::<String>::new());
        assert_eq!(decoded.totals.rows, 2);
        assert_eq!(decoded.totals.target_rows, 2);
        assert_eq!(decoded.totals.path_geometry_rows, 3);
        assert_eq!(decoded.totals.non_type5_exact_path_rows, 1);
        assert_eq!(decoded.totals.assigned_units_exact_rows, 2);
        assert_eq!(decoded.totals.order_linkage_exact_rows, 2);
    }

    #[test]
    fn omits_non_exact_type5_and_rows_without_exact_assignment() {
        let mut non_exact = exact_order(10, 0, 5, Vec::new(), Vec::new());
        non_exact.target_status = STATUS_UNSUPPORTED;
        let mut no_assignment = exact_order(11, 0, 3, Vec::new(), vec![101]);
        no_assignment.assigned_units_status = STATUS_FIXTURE_REQUIRED;
        let record = collect_and_encode_depth_record(&[non_exact, no_assignment], 77, 0, 2, 2);
        assert_eq!(record.emitted_rows, 0);
        assert_eq!(record.extraction.type5_non_exact_omitted_rows, 1);
        assert_eq!(record.extraction.assignment_or_linkage_omitted_rows, 1);
        let decoded = decode_depth_payload(0, &record.payload);
        assert_eq!(decoded.parse_errors, Vec::<String>::new());
        assert_eq!(decoded.totals.rows, 0);
        assert_eq!(decoded.totals.omitted_non_exact_rows, 2);
    }

    #[test]
    fn keeps_exact_rows_when_a_sibling_row_is_non_exact() {
        let mut input = (0..50)
            .map(|index| exact_order(100 + index, index, 3, Vec::new(), vec![1_000 + index]))
            .collect::<Vec<_>>();
        let mut no_assignment = exact_order(999, 50, 3, Vec::new(), vec![2_000]);
        no_assignment.assigned_units_status = STATUS_FIXTURE_REQUIRED;
        input.push(no_assignment);

        let record = collect_and_encode_depth_record(&input, 77, 0, 51, 51);
        assert_eq!(record.emitted_rows, 50);
        assert_eq!(record.extraction.assignment_or_linkage_omitted_rows, 1);
        let decoded = decode_depth_payload(0, &record.payload);
        assert_eq!(decoded.parse_errors, Vec::<String>::new());
        assert_eq!(decoded.totals.rows, 50);
        assert_eq!(decoded.totals.omitted_non_exact_rows, 1);
        assert_eq!(decoded.totals.path_geometry_rows, 50);
    }

    #[test]
    fn drops_invalid_path_rows_but_keeps_fixture_blocked_rows() {
        let exact = exact_order(10, 0, 3, Vec::new(), vec![101, 102]);
        let mut invalid = exact_order(11, 1, 3, Vec::new(), Vec::new());
        invalid.geometry_status = STATUS_VALIDATION_FAILED;
        let mut fixture = exact_order(12, 2, 3, Vec::new(), Vec::new());
        fixture.geometry_status = STATUS_FIXTURE_REQUIRED;

        let record = collect_and_encode_depth_record(&[exact, invalid, fixture], 77, 0, 3, 3);
        assert_eq!(record.emitted_rows, 2);
        assert_eq!(record.extraction.path_geometry_omitted_rows, 1);
        assert_eq!(record.extraction.non_type5_fixture_blocked_rows, 1);
        let decoded = decode_depth_payload(0, &record.payload);
        assert_eq!(decoded.parse_errors, Vec::<String>::new());
        assert_eq!(decoded.totals.rows, 2);
        assert_eq!(decoded.totals.path_geometry_rows, 2);
        assert_eq!(decoded.totals.path_geometry_omitted_rows, 1);
        assert_eq!(decoded.totals.unsupported_geometry_rows, 1);
    }

    #[test]
    fn sink_receives_domain54_payload() {
        let input = vec![exact_order(10, 0, 5, vec![44], Vec::new())];
        let mut sink = TestSink::default();
        let record =
            collect_and_emit_depth_record(&input, 10, COMPACT_STATUS_DELTA_DATA, 1, 1, &mut sink);
        assert_eq!(record.domain_id, 54);
        assert_eq!(sink.domain_id, Some(54));
        let decoded = decode_depth_payload(0, &sink.payload);
        assert_eq!(
            decoded.totals.status_flags_or & COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA
        );
    }

    #[test]
    fn memory_helper_reads_validated_path_vector() {
        let mut mem = TestMemory {
            module_base: 0x1000_0000,
            ..Default::default()
        };
        let army_body = 0x2000;
        let order_ptr = 0x3000;
        let path_begin = 0x4000;
        mem.u64s
            .insert(order_ptr, mem.module_base + ORDER_VTABLE_RVA);
        mem.u64s
            .insert(order_ptr + ORDER_OFF_ARMY_BACKREF, army_body);
        mem.i32s.insert(order_ptr + ORDER_OFF_TYPE, 3);
        mem.u64s
            .insert(order_ptr + ORDER_OFF_PATH_BEGIN, path_begin);
        mem.u32s.insert(order_ptr + ORDER_OFF_PATH_SIZE, 3);
        mem.bytes.insert(
            path_begin,
            [101u32, 102, 103]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect(),
        );
        let row = validated_order_observation(
            &mem,
            17,
            9,
            army_body,
            order_ptr,
            2,
            STATUS_EXACT,
            STATUS_EXACT,
        )
        .expect("validated order");
        assert_eq!(row.path_provinces, vec![101, 102, 103]);
        assert_eq!(row.geometry_status, STATUS_EXACT);
        assert_eq!(row.target_status, STATUS_UNSUPPORTED);
    }

    fn path_vector_test_memory(begin: u64, size: u32, bytes: Option<Vec<u8>>) -> TestMemory {
        let mut mem = TestMemory {
            module_base: 0x1000_0000,
            ..Default::default()
        };
        let army_body = 0x2000;
        let order_ptr = 0x3000;
        mem.u64s
            .insert(order_ptr, mem.module_base + ORDER_VTABLE_RVA);
        mem.u64s
            .insert(order_ptr + ORDER_OFF_ARMY_BACKREF, army_body);
        mem.i32s.insert(order_ptr + ORDER_OFF_TYPE, 3);
        mem.u64s.insert(order_ptr + ORDER_OFF_PATH_BEGIN, begin);
        mem.u32s.insert(order_ptr + ORDER_OFF_PATH_SIZE, size);
        if let Some(bytes) = bytes {
            mem.bytes.insert(begin, bytes);
        }
        mem
    }

    fn assert_path_vector_is_omitted(mem: &TestMemory) {
        let row =
            validated_order_observation(mem, 17, 9, 0x2000, 0x3000, 2, STATUS_EXACT, STATUS_EXACT)
                .expect("validated order");
        assert_eq!(row.geometry_status, STATUS_VALIDATION_FAILED);

        let record = collect_and_encode_depth_record(&[row], 10, 0, 1, 1);
        assert_eq!(record.extraction.path_geometry_omitted_rows, 1);
        let decoded = decode_depth_payload(0, &record.payload);
        assert_eq!(decoded.parse_errors, Vec::<String>::new());
        assert_eq!(decoded.totals.rows, 0);
        assert_eq!(decoded.totals.path_geometry_omitted_rows, 1);
    }

    #[test]
    fn memory_helper_non_heap_path_vector_is_validation_failed_and_omitted() {
        let mem = path_vector_test_memory(0, 2, None);

        assert_path_vector_is_omitted(&mem);
    }

    #[test]
    fn memory_helper_short_path_vector_is_validation_failed_and_omitted() {
        let path_begin = 0x4000;
        let mem = path_vector_test_memory(path_begin, 2, Some(101u32.to_le_bytes().to_vec()));

        assert_path_vector_is_omitted(&mem);
    }
}
