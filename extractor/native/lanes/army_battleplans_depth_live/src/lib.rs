pub mod orders_group_live;

use std::collections::BTreeSet;

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
pub const ORDER_OFF_PARADROP_TARGET: u64 =
    compact_contract::build_profile::DOMAIN54_ORDER_OFF_PARADROP_TARGET as u64;
pub const ORDER_OFF_TYPE5_STATES_BEGIN: u64 =
    compact_contract::build_profile::DOMAIN54_ORDER_OFF_TYPE5_STATES_BEGIN as u64;
pub const ORDER_OFF_TYPE5_STATES_SIZE: u64 =
    compact_contract::build_profile::DOMAIN54_ORDER_OFF_TYPE5_STATES_SIZE as u64;
pub const ORDER_OFF_ASSIGNED_UNITS_BEGIN: u64 =
    compact_contract::build_profile::DOMAIN54_ORDER_OFF_ASSIGNED_UNITS_BEGIN as u64;
pub const ORDER_OFF_ASSIGNED_UNITS_SIZE: u64 =
    compact_contract::build_profile::DOMAIN54_ORDER_OFF_ASSIGNED_UNITS_SIZE as u64;
pub const ORDER_OFF_INSTANCE_ID: u64 =
    compact_contract::build_profile::DOMAIN54_ORDER_OFF_INSTANCE_ID as u64;
pub const ORDER_OFF_ACTIVE: u64 = compact_contract::build_profile::DOMAIN54_ORDER_OFF_ACTIVE as u64;

pub const HEADER_LEN: usize = 60;
pub const ROW_LEN: usize = 88;
pub const TARGET_ROW_LEN: usize = 8;
pub const PATH_GEOMETRY_ROW_LEN: usize = 8;
pub const ASSIGNED_DIVISION_ROW_LEN: usize = 8;

pub const FLAG_DELETED: u32 = 1;
pub const FLAG_TARGET_STATES: u32 = 1 << 1;
pub const FLAG_TARGET_STATE_COUNT: u32 = 1 << 2;
pub const FLAG_NON_TYPE5_GEOMETRY_BLOCKED: u32 = 1 << 3;
pub const FLAG_EXECUTION_BLOCKED: u32 = 1 << 4;
pub const FLAG_PATH_GEOMETRY: u32 = 1 << 5;
pub const FLAG_ASSIGNED_DIVISIONS: u32 = 1 << 6;
pub const FLAG_ACTIVE_VALUE: u32 = 1 << 7;

pub const STATUS_NONE: u32 = 0;
pub const STATUS_EXACT: u32 = 1;
pub const STATUS_UNKNOWN: u32 = 2;
pub const STATUS_UNSUPPORTED: u32 = 3;
pub const STATUS_OVERFLOW: u32 = 4;
pub const STATUS_VALIDATION_FAILED: u32 = 5;
pub const STATUS_FIXTURE_REQUIRED: u32 = 6;
pub const STATUS_COLD_PREFLIGHT_ONLY: u32 = 7;
pub const ORDER_OWNER_KIND_ARMY: u32 = orders_group_live::ORDER_OWNER_KIND_ARMY;
pub const ORDER_OWNER_KIND_ARMY_GROUP: u32 = orders_group_live::ORDER_OWNER_KIND_ARMY_GROUP;

pub const COMPACT_STATUS_DELTA_DATA: u32 = compact_contract::COMPACT_STATUS_DELTA_DATA;
pub const COMPACT_STATUS_PARTIAL_DATA: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
pub const COMPACT_STATUS_FAILURE_MASK: u32 = compact_contract::COMPACT_STATUS_FAILURE_MASK;

pub const ORDER_TYPE_MOVE: i32 = 1;
pub const ORDER_TYPE_LINE: i32 = 2;
pub const ORDER_TYPE_INVASION: i32 = 3;
pub const ORDER_TYPE_PARADROP: i32 = 4;
pub const ORDER_TYPE_AREA_DEFENSE: i32 = 5;
const MAX_CHILD_ROWS: u32 = 5_000;
const MAX_INLINE_CHILD_ROWS: u32 = 256;
const MAX_STATE_ID: u32 = (i32::MAX as u32) - 1;
const MAX_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
const MAX_ARMY_ID: u32 = i32::MAX as u32;
const MAX_SLOT_INDEX: u32 = 64;
const MAX_ORDER_INSTANCE_ID: u32 = 100_000_000;
const MAX_DIVISION_ID: u32 = i32::MAX as u32;
const DIVISION_VTABLE_RVA: u64 = 0x2943CB0;
const DIVISION_TYPE: u32 = 4713;
const DIVISION_ALT_TYPE: u32 = 51;

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
    pub owner_kind: u32,
    pub army_id: u32,
    pub generation: u64,
    pub engine_order_instance_id: u32,
    pub deleted: bool,
    pub slot_index: u32,
    pub order_type: i32,
    pub target_states: Vec<u32>,
    pub path_provinces: Vec<u32>,
    pub target_declared_count: Option<u32>,
    pub target_status: u32,
    pub geometry_status: u32,
    pub assigned_division_ids: Vec<u32>,
    pub assigned_units_status: u32,
    pub order_linkage_status: u32,
    pub active: bool,
    pub active_status: u32,
    pub stance_status: u32,
    pub aggressiveness_status: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattleplanDepthRow {
    pub owner_kind: u32,
    pub army_id: u32,
    pub generation: u64,
    pub engine_order_instance_id: u32,
    pub deleted: bool,
    pub slot_index: u32,
    pub order_type: i32,
    pub target_states: Vec<u32>,
    pub path_provinces: Vec<u32>,
    pub target_declared_count: Option<u32>,
    pub target_status: u32,
    pub geometry_status: u32,
    pub execution_status: u32,
    pub assigned_division_ids: Vec<u32>,
    pub assigned_units_status: u32,
    pub order_linkage_status: u32,
    pub active: bool,
    pub active_status: u32,
    pub stance_status: u32,
    pub aggressiveness_status: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DepthExtraction {
    pub rows: Vec<BattleplanDepthRow>,
    pub type5_exact_rows: u32,
    pub type5_non_exact_omitted_rows: u32,
    pub assignment_or_linkage_omitted_rows: u32,
    pub identity_or_active_omitted_rows: u32,
    pub path_geometry_omitted_rows: u32,
    pub non_type5_exact_path_rows: u32,
    pub path_geometry_rows: u32,
    pub non_type5_fixture_blocked_rows: u32,
    pub execution_fixture_blocked_rows: u32,
    pub assigned_units_exact_rows: u32,
    pub assigned_division_rows: u32,
    pub identity_exact_rows: u32,
    pub active_exact_rows: u32,
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
    pub assigned_division_rows: u64,
    pub identity_exact_rows: u64,
    pub active_exact_rows: u64,
    pub order_linkage_exact_rows: u64,
    pub stance_unknown_rows: u64,
    pub aggressiveness_unknown_rows: u64,
    pub omitted_non_exact_rows: u64,
    pub path_geometry_omitted_rows: u64,
    pub identity_or_active_omitted_rows: u64,
    pub status_flags_or: u32,
    pub orders_group_extension_records: u64,
    pub complete_army_rows: u64,
    pub registered_orders_group_rows: u64,
    pub registered_army_group_rows: u64,
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
    let engine_order_instance_id = mem.read_u32(order_ptr.checked_add(ORDER_OFF_INSTANCE_ID)?)?;
    if engine_order_instance_id == 0 || engine_order_instance_id > MAX_ORDER_INSTANCE_ID {
        return None;
    }
    let active_bytes = mem.read_bytes(order_ptr.checked_add(ORDER_OFF_ACTIVE)?, 1)?;
    let active = match active_bytes.first().copied()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let assigned_division_ids = read_assigned_division_ids(mem, order_ptr).ok()?;
    let (target_states, target_declared_count, target_status) =
        read_type5_target_states(mem, order_ptr, order_type);
    let (path_provinces, geometry_status) =
        read_non_type5_path_provinces(mem, order_ptr, order_type);
    Some(RawOrderObservation {
        owner_kind: ORDER_OWNER_KIND_ARMY,
        army_id,
        generation,
        engine_order_instance_id,
        deleted: false,
        slot_index,
        order_type,
        target_states,
        path_provinces,
        target_declared_count,
        target_status,
        geometry_status,
        assigned_division_ids,
        assigned_units_status,
        order_linkage_status,
        active,
        active_status: STATUS_EXACT,
        stance_status: STATUS_UNKNOWN,
        aggressiveness_status: STATUS_UNKNOWN,
    })
}

fn read_assigned_division_ids<M: BattleplansDepthMemory>(
    mem: &M,
    order_ptr: u64,
) -> Result<Vec<u32>, u32> {
    let begin = mem
        .read_u64(
            order_ptr
                .checked_add(ORDER_OFF_ASSIGNED_UNITS_BEGIN)
                .ok_or(STATUS_VALIDATION_FAILED)?,
        )
        .ok_or(STATUS_VALIDATION_FAILED)?;
    let count = mem
        .read_u32(
            order_ptr
                .checked_add(ORDER_OFF_ASSIGNED_UNITS_SIZE)
                .ok_or(STATUS_VALIDATION_FAILED)?,
        )
        .ok_or(STATUS_VALIDATION_FAILED)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    if count > MAX_CHILD_ROWS || count > MAX_INLINE_CHILD_ROWS || !mem.is_heap_pointer(begin) {
        return Err(if count > MAX_INLINE_CHILD_ROWS {
            STATUS_OVERFLOW
        } else {
            STATUS_VALIDATION_FAILED
        });
    }
    let raw = mem
        .read_bytes(begin, count as usize * 8)
        .ok_or(STATUS_VALIDATION_FAILED)?;
    if raw.len() != count as usize * 8 {
        return Err(STATUS_VALIDATION_FAILED);
    }
    let division_vtable = mem
        .module_base()
        .checked_add(DIVISION_VTABLE_RVA)
        .ok_or(STATUS_VALIDATION_FAILED)?;
    let mut ids = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        let start = index * 8;
        let ptr = u64::from_le_bytes(raw[start..start + 8].try_into().unwrap());
        if !mem.is_heap_pointer(ptr) || mem.read_u64(ptr) != Some(division_vtable) {
            return Err(STATUS_VALIDATION_FAILED);
        }
        let handle = mem.read_u64(ptr + 8).ok_or(STATUS_VALIDATION_FAILED)?;
        let kind = handle as u32;
        let id = (handle >> 32) as u32;
        if !matches!(kind, DIVISION_TYPE | DIVISION_ALT_TYPE)
            || id == 0
            || id > MAX_DIVISION_ID
            || ids.contains(&id)
        {
            return Err(STATUS_VALIDATION_FAILED);
        }
        ids.push(id);
    }
    Ok(ids)
}

pub fn read_type5_target_states<M: BattleplansDepthMemory>(
    mem: &M,
    order_ptr: u64,
    order_type: i32,
) -> (Vec<u32>, Option<u32>, u32) {
    if order_type != ORDER_TYPE_AREA_DEFENSE {
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
        compact_contract::bound_trips::BOUND_TRIP_MAP_STATE_ID,
    )
    .map(|(values, count)| (values, Some(count), STATUS_EXACT))
    .unwrap_or_else(|status| (Vec::new(), None, status))
}

pub fn read_non_type5_path_provinces<M: BattleplansDepthMemory>(
    mem: &M,
    order_ptr: u64,
    order_type: i32,
) -> (Vec<u32>, u32) {
    if order_type == ORDER_TYPE_AREA_DEFENSE {
        return (Vec::new(), STATUS_EXACT);
    }
    if !matches!(
        order_type,
        ORDER_TYPE_MOVE | ORDER_TYPE_LINE | ORDER_TYPE_INVASION | ORDER_TYPE_PARADROP
    ) {
        return (Vec::new(), STATUS_UNSUPPORTED);
    }
    let path = match read_u32_vector(
        mem,
        order_ptr,
        ORDER_OFF_PATH_BEGIN,
        ORDER_OFF_PATH_SIZE,
        MAX_CHILD_ROWS,
        MAX_INLINE_CHILD_ROWS,
        MAX_PROVINCE_ID,
        compact_contract::bound_trips::BOUND_TRIP_MAP_PROVINCE_ID,
    ) {
        Ok((values, _)) if !values.is_empty() => values,
        Ok(_) => return (Vec::new(), STATUS_FIXTURE_REQUIRED),
        Err(status) => return (Vec::new(), status),
    };
    if order_type != ORDER_TYPE_PARADROP {
        return (path, STATUS_EXACT);
    }

    let source = path[0];
    let Some(target_address) = order_ptr.checked_add(ORDER_OFF_PARADROP_TARGET) else {
        return (Vec::new(), STATUS_VALIDATION_FAILED);
    };
    let Some(target) = mem.read_u32(target_address) else {
        return (Vec::new(), STATUS_VALIDATION_FAILED);
    };
    if target == 0
        || compact_contract::bound_trips::bound_exceeded_u32(
            target,
            MAX_PROVINCE_ID,
            compact_contract::bound_trips::BOUND_TRIP_MAP_PROVINCE_ID,
        )
        || target == source
    {
        return (Vec::new(), STATUS_VALIDATION_FAILED);
    }
    (vec![source, target], STATUS_EXACT)
}

pub fn extract_validated_depth_subset(source: &[RawOrderObservation]) -> DepthExtraction {
    let mut out = DepthExtraction::default();
    for row in source {
        if row.army_id == 0
            || row.army_id > MAX_ARMY_ID
            || !matches!(
                row.owner_kind,
                ORDER_OWNER_KIND_ARMY | ORDER_OWNER_KIND_ARMY_GROUP
            )
        {
            out.identity_or_active_omitted_rows =
                out.identity_or_active_omitted_rows.saturating_add(1);
            continue;
        }
        if row.deleted {
            out.rows.push(BattleplanDepthRow {
                owner_kind: row.owner_kind,
                army_id: row.army_id,
                generation: row.generation,
                engine_order_instance_id: row.engine_order_instance_id,
                deleted: true,
                slot_index: row.slot_index,
                order_type: row.order_type,
                target_states: Vec::new(),
                path_provinces: Vec::new(),
                target_declared_count: None,
                target_status: STATUS_NONE,
                geometry_status: STATUS_NONE,
                execution_status: STATUS_NONE,
                assigned_division_ids: Vec::new(),
                assigned_units_status: STATUS_NONE,
                order_linkage_status: STATUS_NONE,
                active: false,
                active_status: STATUS_NONE,
                stance_status: STATUS_NONE,
                aggressiveness_status: STATUS_NONE,
            });
            continue;
        }
        if row.engine_order_instance_id == 0
            || row.engine_order_instance_id > MAX_ORDER_INSTANCE_ID
            || row.active_status != STATUS_EXACT
        {
            out.identity_or_active_omitted_rows =
                out.identity_or_active_omitted_rows.saturating_add(1);
            continue;
        }
        if row.assigned_units_status != STATUS_EXACT || row.order_linkage_status != STATUS_EXACT {
            out.assignment_or_linkage_omitted_rows =
                out.assignment_or_linkage_omitted_rows.saturating_add(1);
            continue;
        }
        if row.order_type == ORDER_TYPE_AREA_DEFENSE && row.target_status != STATUS_EXACT {
            out.type5_non_exact_omitted_rows = out.type5_non_exact_omitted_rows.saturating_add(1);
            continue;
        }
        let (target_status, geometry_status, path_provinces) =
            if row.order_type == ORDER_TYPE_AREA_DEFENSE {
                out.type5_exact_rows = out.type5_exact_rows.saturating_add(1);
                (STATUS_EXACT, STATUS_EXACT, Vec::new())
            } else if matches!(
                row.order_type,
                ORDER_TYPE_MOVE | ORDER_TYPE_LINE | ORDER_TYPE_INVASION | ORDER_TYPE_PARADROP
            ) && row.geometry_status == STATUS_EXACT
                && !row.path_provinces.is_empty()
                && (row.order_type != ORDER_TYPE_PARADROP || row.path_provinces.len() == 2)
            {
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
        out.assigned_units_exact_rows = out.assigned_units_exact_rows.saturating_add(1);
        out.assigned_division_rows = out
            .assigned_division_rows
            .saturating_add(row.assigned_division_ids.len().min(u32::MAX as usize) as u32);
        out.identity_exact_rows = out.identity_exact_rows.saturating_add(1);
        out.active_exact_rows = out.active_exact_rows.saturating_add(1);
        out.order_linkage_exact_rows = out.order_linkage_exact_rows.saturating_add(1);
        if row.stance_status == STATUS_UNKNOWN {
            out.stance_unknown_rows = out.stance_unknown_rows.saturating_add(1);
        }
        if row.aggressiveness_status == STATUS_UNKNOWN {
            out.aggressiveness_unknown_rows = out.aggressiveness_unknown_rows.saturating_add(1);
        }
        out.rows.push(BattleplanDepthRow {
            owner_kind: row.owner_kind,
            army_id: row.army_id,
            generation: row.generation,
            engine_order_instance_id: row.engine_order_instance_id,
            deleted: false,
            slot_index: row.slot_index,
            order_type: row.order_type,
            target_states: row.target_states.clone(),
            path_provinces,
            target_declared_count: row.target_declared_count,
            target_status,
            geometry_status,
            execution_status: STATUS_EXACT,
            assigned_division_ids: row.assigned_division_ids.clone(),
            assigned_units_status: row.assigned_units_status,
            order_linkage_status: row.order_linkage_status,
            active: row.active,
            active_status: row.active_status,
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
        .saturating_add(extraction.assignment_or_linkage_omitted_rows)
        .saturating_add(extraction.identity_or_active_omitted_rows);
    let payload = encode_depth_payload(
        &extraction.rows,
        elapsed_wall_ms,
        status_flags,
        emitted_rows,
        observed_army_rows,
        observed_order_rows,
        omitted_non_exact_rows,
        extraction.path_geometry_omitted_rows,
        extraction.identity_or_active_omitted_rows,
    );
    EncodedDepthRecord {
        domain_id: DOMAIN_ID,
        payload,
        emitted_rows,
        extraction,
    }
}

pub fn collect_and_encode_depth_record_with_orders_groups(
    observations: &[RawOrderObservation],
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_order_rows: u32,
    orders_groups: &orders_group_live::CompleteArmyOrdersGroups,
) -> Result<EncodedDepthRecord, orders_group_live::OrdersGroupReadError> {
    let complete_army_ids = orders_groups
        .armies
        .iter()
        .map(|row| row.army_id)
        .collect::<BTreeSet<_>>();
    let complete_army_group_ids = orders_groups
        .army_group_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if let Some(orphan) = observations.iter().find(|row| {
        !row.deleted
            && match row.owner_kind {
                ORDER_OWNER_KIND_ARMY => !complete_army_ids.contains(&row.army_id),
                ORDER_OWNER_KIND_ARMY_GROUP => !complete_army_group_ids.contains(&row.army_id),
                _ => true,
            }
    }) {
        return Err(orders_group_live::OrdersGroupReadError::InvalidObject {
            army_id: orphan.army_id,
            reason: "typed order owner absent from complete manager",
        });
    }
    let observed_army_rows = orders_groups.armies.len().min(u32::MAX as usize) as u32;
    let mut record = collect_and_encode_depth_record(
        observations,
        elapsed_wall_ms,
        status_flags,
        observed_army_rows,
        observed_order_rows,
    );
    orders_group_live::append_orders_group_extension(&mut record.payload, orders_groups)?;
    Ok(record)
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
    identity_or_active_omitted_rows: u32,
) -> Vec<u8> {
    let target_count = rows
        .iter()
        .map(|row| row.target_states.len())
        .sum::<usize>();
    let path_count = rows
        .iter()
        .map(|row| row.path_provinces.len())
        .sum::<usize>();
    let assigned_division_count = rows
        .iter()
        .map(|row| row.assigned_division_ids.len())
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
            + path_count * PATH_GEOMETRY_ROW_LEN
            + assigned_division_count * ASSIGNED_DIVISION_ROW_LEN,
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
    out.extend_from_slice(&(assigned_division_count.min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&identity_or_active_omitted_rows.to_le_bytes());

    let mut target_cursor = 0u32;
    let mut path_cursor = 0u32;
    let mut assigned_cursor = 0u32;
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
        if !row.assigned_division_ids.is_empty() {
            flags |= FLAG_ASSIGNED_DIVISIONS;
        }
        if !row.deleted && row.active_status == STATUS_EXACT {
            flags |= FLAG_ACTIVE_VALUE;
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
        let assigned_first = assigned_cursor;
        let assigned_count = row.assigned_division_ids.len().min(u32::MAX as usize) as u32;

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
        out.extend_from_slice(&row.engine_order_instance_id.to_le_bytes());
        out.extend_from_slice(&assigned_first.to_le_bytes());
        out.extend_from_slice(&assigned_count.to_le_bytes());
        out.extend_from_slice(&(row.active as u32).to_le_bytes());
        out.extend_from_slice(&row.active_status.to_le_bytes());
        out.extend_from_slice(&row.owner_kind.to_le_bytes());
        target_cursor =
            target_cursor.saturating_add(row.target_states.len().min(u32::MAX as usize) as u32);
        path_cursor =
            path_cursor.saturating_add(row.path_provinces.len().min(u32::MAX as usize) as u32);
        assigned_cursor = assigned_cursor.saturating_add(assigned_count);
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
    for row in rows {
        for division_id in &row.assigned_division_ids {
            out.extend_from_slice(&division_id.to_le_bytes());
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
    let assigned_division_count = read_u32(payload, 52) as usize;
    let identity_or_active_omitted_rows = read_u32(payload, 56) as usize;
    let legacy_expected = HEADER_LEN
        .saturating_add(row_count.saturating_mul(ROW_LEN))
        .saturating_add(target_count.saturating_mul(TARGET_ROW_LEN))
        .saturating_add(path_count.saturating_mul(PATH_GEOMETRY_ROW_LEN))
        .saturating_add(assigned_division_count.saturating_mul(ASSIGNED_DIVISION_ROW_LEN));
    if payload.len() < legacy_expected {
        summary.parse_errors.push(format!(
            "offset {offset}: domain 54 payload length {} below {legacy_expected}",
            payload.len()
        ));
        return summary;
    }
    if payload.len() > legacy_expected {
        match orders_group_live::decode_orders_group_extension(&payload[legacy_expected..]) {
            Ok(extension) => {
                if extension.armies.len() != observed_army_rows {
                    summary.parse_errors.push(format!(
                        "offset {offset}: complete-manager army count differs from header"
                    ));
                    return summary;
                }
                summary.totals.orders_group_extension_records = 1;
                summary.totals.complete_army_rows = extension.armies.len() as u64;
                summary.totals.registered_orders_group_rows =
                    extension.registered_entry_count as u64;
                summary.totals.registered_army_group_rows = extension.army_group_count as u64;
            }
            Err(reason) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: invalid orders-group extension: {reason}"
                ));
                return summary;
            }
        }
    }
    if changed_rows > row_count {
        summary
            .parse_errors
            .push(format!("offset {offset}: changed rows exceed row count"));
    }
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
    summary.totals.data_records = 1;
    summary.totals.rows = row_count as u64;
    summary.totals.changed_rows = changed_rows as u64;
    summary.totals.target_rows = target_count as u64;
    summary.totals.path_geometry_rows = path_count as u64;
    summary.totals.assigned_division_rows = assigned_division_count as u64;
    summary.totals.unsupported_geometry_rows = unsupported_geometry_rows as u64;
    summary.totals.execution_fixture_required_rows = execution_unknown_rows as u64;
    summary.totals.omitted_non_exact_rows = omitted_non_exact_rows as u64;
    summary.totals.path_geometry_omitted_rows = path_geometry_omitted_rows as u64;
    summary.totals.identity_or_active_omitted_rows = identity_or_active_omitted_rows as u64;
    summary.totals.status_flags_or = status_flags;

    let mut cursor = HEADER_LEN;
    let mut declared_targets = 0usize;
    let mut declared_paths = 0usize;
    let mut declared_assigned_divisions = 0usize;
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
        let engine_order_instance_id = read_u32(payload, cursor + 64);
        let assigned_first = read_u32(payload, cursor + 68) as usize;
        let assigned_count = read_u32(payload, cursor + 72) as usize;
        let active_value = read_u32(payload, cursor + 76);
        let active_status = read_u32(payload, cursor + 80);
        let owner_kind = read_u32(payload, cursor + 84);
        cursor += ROW_LEN;

        let deleted = flags & FLAG_DELETED != 0;
        let has_targets = flags & FLAG_TARGET_STATES != 0;
        let has_path = flags & FLAG_PATH_GEOMETRY != 0;
        let has_assigned = flags & FLAG_ASSIGNED_DIVISIONS != 0;
        let has_active = flags & FLAG_ACTIVE_VALUE != 0;
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
                | FLAG_PATH_GEOMETRY
                | FLAG_ASSIGNED_DIVISIONS
                | FLAG_ACTIVE_VALUE)
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
            || active_status > STATUS_COLD_PREFLIGHT_ONLY
        {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has bad status code"
            ));
        }
        if deleted
            && (child_first != 0
                || child_count != 0
                || target_declared != 0
                || assigned_first != 0
                || assigned_count != 0
                || active_value != 0
                || active_status != STATUS_NONE)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: deleted row {row_index} has child fields"
            ));
        }
        if !matches!(
            owner_kind,
            ORDER_OWNER_KIND_ARMY | ORDER_OWNER_KIND_ARMY_GROUP
        ) {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has invalid owner kind"
            ));
        }
        if engine_order_instance_id == 0 || engine_order_instance_id > MAX_ORDER_INSTANCE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has bad engine order instance id"
            ));
        }
        if !deleted && (!has_active || active_status != STATUS_EXACT || active_value > 1) {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} lacks exact active state"
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
        if has_assigned {
            if assigned_count == 0
                || assigned_first.saturating_add(assigned_count) > assigned_division_count
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: assigned-division range for row {row_index} is invalid"
                ));
            }
        } else if assigned_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: row {row_index} has assigned count without flag"
            ));
        }
        declared_assigned_divisions = declared_assigned_divisions.saturating_add(assigned_count);
        if !deleted && order_type == ORDER_TYPE_AREA_DEFENSE && target_status != STATUS_EXACT {
            summary.parse_errors.push(format!(
                "offset {offset}: type-5 row {row_index} is not exact"
            ));
        }
        if has_targets {
            if order_type != ORDER_TYPE_AREA_DEFENSE {
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
            if !matches!(
                order_type,
                ORDER_TYPE_MOVE | ORDER_TYPE_LINE | ORDER_TYPE_INVASION | ORDER_TYPE_PARADROP
            ) {
                summary.parse_errors.push(format!(
                    "offset {offset}: unsupported order type {order_type} row {row_index} has path geometry"
                ));
            }
            if order_type == ORDER_TYPE_PARADROP && child_count != 2 {
                summary.parse_errors.push(format!(
                    "offset {offset}: paradrop row {row_index} does not carry exactly source and target"
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
        if engine_order_instance_id != 0 && !deleted {
            summary.totals.identity_exact_rows =
                summary.totals.identity_exact_rows.saturating_add(1);
        }
        if active_status == STATUS_EXACT && !deleted {
            summary.totals.active_exact_rows = summary.totals.active_exact_rows.saturating_add(1);
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
    if declared_assigned_divisions != assigned_division_count {
        summary.parse_errors.push(format!(
            "offset {offset}: assigned-division child count mismatch"
        ));
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
    for child_index in 0..assigned_division_count {
        let division_id = read_u32(payload, cursor);
        let reserved = read_u32(payload, cursor + 4);
        cursor += ASSIGNED_DIVISION_ROW_LEN;
        if division_id == 0 || division_id > MAX_DIVISION_ID || reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: assigned-division child row {child_index} is invalid"
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
    value_trip_unit: usize,
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
        if value == 0 {
            return Err(STATUS_VALIDATION_FAILED);
        }
        if compact_contract::bound_trips::bound_exceeded_u32(value, max_value, value_trip_unit) {
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
            owner_kind: ORDER_OWNER_KIND_ARMY,
            army_id,
            generation: 1,
            engine_order_instance_id: 10_000 + slot_index,
            deleted: false,
            slot_index,
            order_type,
            target_declared_count: (!target_states.is_empty())
                .then_some(target_states.len() as u32),
            target_states,
            path_provinces,
            target_status: if order_type == ORDER_TYPE_AREA_DEFENSE {
                STATUS_EXACT
            } else {
                STATUS_UNSUPPORTED
            },
            geometry_status: STATUS_EXACT,
            assigned_division_ids: vec![20_000 + slot_index],
            assigned_units_status: STATUS_EXACT,
            order_linkage_status: STATUS_EXACT,
            active: true,
            active_status: STATUS_EXACT,
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
    fn production_record_carries_complete_armies_and_exact_scalars() {
        let carrier = orders_group_live::CompleteArmyOrdersGroups {
            armies: vec![
                orders_group_live::ArmyOrdersGroupRow {
                    army_id: 10,
                    execution_type: 0,
                    cohesion_type: 1,
                    leader_proximity: 2,
                },
                orders_group_live::ArmyOrdersGroupRow {
                    army_id: 11,
                    execution_type: 2,
                    cohesion_type: 3,
                    leader_proximity: 4,
                },
            ],
            army_group_count: 1,
            army_group_ids: vec![20],
            registered_entry_count: 3,
        };
        let record = collect_and_encode_depth_record_with_orders_groups(
            &[exact_order(10, 0, ORDER_TYPE_LINE, Vec::new(), vec![101])],
            77,
            0,
            1,
            &carrier,
        )
        .unwrap();
        let decoded = decode_depth_payload(0, &record.payload);
        assert_eq!(decoded.parse_errors, Vec::<String>::new());
        assert_eq!(decoded.totals.rows, 1);
        assert_eq!(decoded.totals.orders_group_extension_records, 1);
        assert_eq!(decoded.totals.complete_army_rows, 2);
        assert_eq!(decoded.totals.registered_orders_group_rows, 3);
        assert_eq!(decoded.totals.registered_army_group_rows, 1);
    }

    #[test]
    fn complete_manager_rejects_header_army_count_disagreement() {
        let carrier = orders_group_live::CompleteArmyOrdersGroups {
            armies: vec![orders_group_live::ArmyOrdersGroupRow {
                army_id: 10,
                execution_type: 2,
                cohesion_type: 3,
                leader_proximity: 4,
            }],
            army_group_count: 0,
            army_group_ids: vec![],
            registered_entry_count: 1,
        };
        let mut record =
            collect_and_encode_depth_record_with_orders_groups(&[], 77, 0, 0, &carrier).unwrap();
        record.payload[20..24].copy_from_slice(&2u32.to_le_bytes());

        let decoded = decode_depth_payload(0, &record.payload);
        assert!(decoded
            .parse_errors
            .iter()
            .any(|error| error.contains("complete-manager army count differs from header")));
        assert_eq!(decoded.totals.orders_group_extension_records, 0);
    }

    #[test]
    fn complete_manager_rejects_live_order_for_unregistered_army() {
        let carrier = orders_group_live::CompleteArmyOrdersGroups {
            armies: vec![orders_group_live::ArmyOrdersGroupRow {
                army_id: 10,
                execution_type: 2,
                cohesion_type: 3,
                leader_proximity: 4,
            }],
            army_group_count: 0,
            army_group_ids: vec![],
            registered_entry_count: 1,
        };
        let result = collect_and_encode_depth_record_with_orders_groups(
            &[exact_order(11, 0, ORDER_TYPE_LINE, Vec::new(), vec![101])],
            77,
            0,
            1,
            &carrier,
        );
        assert_eq!(
            result,
            Err(orders_group_live::OrdersGroupReadError::InvalidObject {
                army_id: 11,
                reason: "typed order owner absent from complete manager",
            })
        );
    }

    #[test]
    fn complete_manager_encodes_exact_army_group_owner_in_reserved_row_word() {
        let carrier = orders_group_live::CompleteArmyOrdersGroups {
            armies: vec![orders_group_live::ArmyOrdersGroupRow {
                army_id: 10,
                execution_type: 2,
                cohesion_type: 3,
                leader_proximity: 4,
            }],
            army_group_count: 1,
            army_group_ids: vec![23],
            registered_entry_count: 2,
        };
        let mut group_order = exact_order(23, 0, ORDER_TYPE_LINE, Vec::new(), vec![101]);
        group_order.owner_kind = ORDER_OWNER_KIND_ARMY_GROUP;

        let record =
            collect_and_encode_depth_record_with_orders_groups(&[group_order], 77, 0, 1, &carrier)
                .unwrap();
        assert_eq!(
            read_u32(&record.payload, HEADER_LEN + 84),
            ORDER_OWNER_KIND_ARMY_GROUP
        );
        assert!(decode_depth_payload(0, &record.payload)
            .parse_errors
            .is_empty());
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
    fn delta_tombstone_may_exceed_current_observed_order_count() {
        let mut removed = exact_order(10, 0, 5, vec![44], Vec::new());
        removed.deleted = true;
        removed.target_states.clear();
        let record =
            collect_and_encode_depth_record(&[removed], 10, COMPACT_STATUS_DELTA_DATA, 1, 0);
        let decoded = decode_depth_payload(0, &record.payload);
        assert_eq!(decoded.parse_errors, Vec::<String>::new());
        assert_eq!(decoded.totals.rows, 1);
    }

    #[test]
    fn preserves_engine_identity_ordered_membership_inactive_state_and_tombstone() {
        let mut live = exact_order(17, 2, 3, Vec::new(), vec![101, 102]);
        live.engine_order_instance_id = 2_025;
        live.assigned_division_ids = vec![301, 205, 777];
        live.active = false;
        let full = collect_and_encode_depth_record(&[live.clone()], 5, 0, 1, 1);
        let decoded = decode_depth_payload(0, &full.payload);
        assert_eq!(decoded.parse_errors, Vec::<String>::new());
        assert_eq!(decoded.totals.identity_exact_rows, 1);
        assert_eq!(decoded.totals.active_exact_rows, 1);
        assert_eq!(decoded.totals.assigned_division_rows, 3);
        assert_eq!(read_u32(&full.payload, HEADER_LEN + 64), 2_025);
        assert_eq!(read_u32(&full.payload, HEADER_LEN + 76), 0);
        let assigned_start = HEADER_LEN + ROW_LEN + 2 * PATH_GEOMETRY_ROW_LEN;
        assert_eq!(read_u32(&full.payload, assigned_start), 301);
        assert_eq!(
            read_u32(&full.payload, assigned_start + ASSIGNED_DIVISION_ROW_LEN),
            205
        );
        assert_eq!(
            read_u32(
                &full.payload,
                assigned_start + 2 * ASSIGNED_DIVISION_ROW_LEN
            ),
            777
        );

        live.deleted = true;
        live.target_states.clear();
        live.path_provinces.clear();
        live.assigned_division_ids.clear();
        let tombstone =
            collect_and_encode_depth_record(&[live], 6, COMPACT_STATUS_DELTA_DATA, 1, 1);
        let decoded = decode_depth_payload(0, &tombstone.payload);
        assert_eq!(decoded.parse_errors, Vec::<String>::new());
        assert_eq!(read_u32(&tombstone.payload, HEADER_LEN + 64), 2_025);
        assert_eq!(
            read_u32(&tombstone.payload, HEADER_LEN + 12) & FLAG_DELETED,
            FLAG_DELETED
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
        mem.u32s.insert(order_ptr + ORDER_OFF_INSTANCE_ID, 1_234);
        mem.bytes.insert(order_ptr + ORDER_OFF_ACTIVE, vec![1]);
        mem.u64s
            .insert(order_ptr + ORDER_OFF_ASSIGNED_UNITS_BEGIN, 0);
        mem.u32s
            .insert(order_ptr + ORDER_OFF_ASSIGNED_UNITS_SIZE, 0);
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

    #[test]
    fn path_geometry_is_fail_closed_for_unknown_order_types() {
        let mem = TestMemory::default();
        let (path, status) = read_non_type5_path_provinces(&mem, 0x3000, 99);
        assert!(path.is_empty());
        assert_eq!(status, STATUS_UNSUPPORTED);
        let (path, status) = read_non_type5_path_provinces(&mem, 0x3000, ORDER_TYPE_AREA_DEFENSE);
        assert!(path.is_empty());
        assert_eq!(status, STATUS_EXACT);
    }

    #[test]
    fn prepared_type1_and_type4_geometry_is_exact() {
        let movement = exact_order(10, 0, ORDER_TYPE_MOVE, Vec::new(), vec![101, 102]);
        let paradrop = exact_order(10, 1, ORDER_TYPE_PARADROP, Vec::new(), vec![201, 202]);
        let record = collect_and_encode_depth_record(&[movement, paradrop], 10, 0, 1, 2);
        assert_eq!(record.emitted_rows, 2);
        assert_eq!(record.extraction.non_type5_exact_path_rows, 2);
        assert_eq!(record.extraction.path_geometry_rows, 4);
        let decoded = decode_depth_payload(0, &record.payload);
        assert_eq!(decoded.parse_errors, Vec::<String>::new());
    }

    #[test]
    fn prepared_unknown_order_geometry_cannot_bypass_type_dispatch() {
        let input = exact_order(10, 0, 99, Vec::new(), vec![101, 102]);
        let record = collect_and_encode_depth_record(&[input], 10, 0, 1, 1);
        assert_eq!(record.emitted_rows, 1);
        assert_eq!(record.extraction.non_type5_exact_path_rows, 0);
        assert_eq!(record.extraction.path_geometry_rows, 0);
        assert_eq!(record.extraction.non_type5_fixture_blocked_rows, 1);
    }

    #[test]
    fn decoder_rejects_path_geometry_for_unknown_order_type() {
        let input = exact_order(10, 0, ORDER_TYPE_INVASION, Vec::new(), vec![101, 102]);
        let mut record = collect_and_encode_depth_record(&[input], 10, 0, 1, 1);
        record.payload[HEADER_LEN + 20..HEADER_LEN + 24].copy_from_slice(&99u32.to_le_bytes());
        let decoded = decode_depth_payload(0, &record.payload);
        assert!(decoded
            .parse_errors
            .iter()
            .any(|error| error.contains("unsupported order type 99")));
    }

    #[test]
    fn memory_helper_reads_type1_route_and_type4_source_target() {
        let path_begin = 0x4000;
        let mut mem = path_vector_test_memory(
            path_begin,
            2,
            Some(
                [101u32, 102]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect(),
            ),
        );
        let (movement, movement_status) =
            read_non_type5_path_provinces(&mem, 0x3000, ORDER_TYPE_MOVE);
        assert_eq!(movement, vec![101, 102]);
        assert_eq!(movement_status, STATUS_EXACT);

        mem.u32s.insert(0x3000 + ORDER_OFF_PARADROP_TARGET, 303);
        let (paradrop, paradrop_status) =
            read_non_type5_path_provinces(&mem, 0x3000, ORDER_TYPE_PARADROP);
        assert_eq!(paradrop, vec![101, 303]);
        assert_eq!(paradrop_status, STATUS_EXACT);
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
        mem.u32s.insert(order_ptr + ORDER_OFF_INSTANCE_ID, 1_234);
        mem.bytes.insert(order_ptr + ORDER_OFF_ACTIVE, vec![1]);
        mem.u64s
            .insert(order_ptr + ORDER_OFF_ASSIGNED_UNITS_BEGIN, 0);
        mem.u32s
            .insert(order_ptr + ORDER_OFF_ASSIGNED_UNITS_SIZE, 0);
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
