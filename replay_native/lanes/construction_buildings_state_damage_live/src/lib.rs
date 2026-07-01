#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

pub mod collector_module;
pub mod decoder_gate;

pub const DOMAIN_ID: u16 = 48;
pub const DOMAIN_NAME: &str = "construction_buildings_state_damage_live";
pub const COMPACT_MAGIC: [u8; 4] = *b"CBSD";
pub const COMPACT_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 64;
pub const CONSTRUCTION_ROW_LEN: usize = 40;
pub const BUILDING_ROW_LEN: usize = 32;
pub const RESOURCE_ROW_LEN: usize = 16;

pub const STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

pub const SOURCE_FLAG_DLL_NATIVE: u32 = 1;
pub const SOURCE_FLAG_SHARED_COUNTRY_ROOTS: u32 = 1 << 1;
pub const SOURCE_FLAG_SHARED_MAP_TOPOLOGY: u32 = 1 << 2;
pub const SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE: u32 = 1 << 3;
pub const SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE: u32 = 1 << 4;

pub const BUILD_PROFILE: &str = "870ebf";
pub const HEAP_LO: usize = 0x10000000000;
pub const HEAP_HI: usize = 0x7FF000000000;
pub const POINTER_WIDTH: usize = 8;

pub const STATE_VTABLE_RVA: usize = 0x29206C0;
pub const STATE_OFF_SID: usize = 0x58;
pub const STATE_OFF_DEF: usize = 0x30;
pub const STATE_OFF_BUILDINGS_VEC: usize = 0x158;
pub const STATE_DEF_OFF_RESOURCES_PTR: usize = 0xF8;
pub const STATE_DEF_OFF_RESOURCES_SIZE: usize = 0x100;
pub const STATE_DEF_OFF_RESOURCES_CAP: usize = 0x104;
pub const STATE_RESOURCE_STRIDE: usize = 0x10;
pub const STATE_RESOURCE_NO_ID: u32 = u32::MAX;
pub const STATE_RESOURCE_MAX_ROWS: u32 = 16;
pub const STATE_RESOURCE_MAX_ABS_RAW: i32 = 100_000_000;

pub const CPROVINCE_VTABLE_RVA: usize = 0x295AF68;
pub const CPROVINCE_OFF_ID: usize = 0xA4;
pub const CPROVINCE_OFF_STATE: usize = 0xC0;
pub const CPROVINCE_OFF_BUILDINGS_VEC: usize = 0x1C8;

pub const CBUILDING_VTABLE_RVA: usize = 0x2973B88;
pub const CBUILDING_OFF_TYPE: usize = 0x08;
pub const CBUILDING_OFF_LEVEL: usize = 0x40;
pub const CBUILDING_OFF_HEALTHY_LEVEL: usize = 0x42;
pub const BUILDING_VECTOR_MAX: u32 = 64;
pub const BUILDING_LEVEL_SANE_MAX: u32 = 1_000;

pub const OBJ_PROD_MANAGER: usize = 0x0F68;
pub const CONSTRUCTION_BEGIN: usize = 0x70;
pub const CONSTRUCTION_CAP: usize = 0x78;
pub const CONSTRUCTION_SIZE: usize = 0x7C;
pub const CONSTRUCTION_STRIDE: usize = 0x08;
pub const LINE_TYPE: usize = 0x08;
pub const LINE_ID: usize = 0x0C;
pub const LINE_ACTIVE_FACTORIES: usize = 0x18;
pub const LINE_COST: usize = 0x28;
pub const LINE_PRODUCED: usize = 0x38;
pub const LINE_CONSTRUCTION_SUB: usize = 0x70;
pub const LINE_TYPE_CONSTRUCTION: u32 = 59;
pub const CONSTRUCTION_TEMPLATE_ID: usize = 0x08;
pub const MAX_CONSTRUCTION_TEMPLATE_ID: u32 = 10_000_000;
pub const MAX_PRODUCTION_VECTOR_ENTRIES: u32 = 4096;
pub const MAX_PRODUCTION_LINE_FIXED_RAW: i32 = 1_000_000_000;
pub const MAX_ACTIVE_FACTORIES: i32 = 100_000;

pub const SCOPE_STATE: u32 = 1;
pub const SCOPE_PROVINCE: u32 = 2;
pub const MAX_STATE_ID: u32 = 65_535;
pub const MAX_PROVINCE_ID: u32 = 65_535;

pub const CONSTRUCTION_FLAG_TEMPLATE_PRESENT: u32 = 1;
pub const CONSTRUCTION_FLAG_TARGET_STATE_PRESENT: u32 = 1 << 1;
pub const BUILDING_FLAG_HEALTHY_PRESENT: u32 = 1;
pub const BUILDING_FLAG_DAMAGED: u32 = 1 << 1;

pub trait LaneMemory {
    fn read_u8(&self, addr: usize) -> Option<u8>;

    fn read_bytes(&self, addr: usize, len: usize) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        for offset in 0..len {
            out.push(self.read_u8(addr.checked_add(offset)?)?);
        }
        Some(out)
    }

    fn read_u16(&self, addr: usize) -> Option<u16> {
        let bytes = self.read_bytes(addr, 2)?;
        Some(u16::from_le_bytes(bytes.try_into().ok()?))
    }

    fn read_u32(&self, addr: usize) -> Option<u32> {
        let bytes = self.read_bytes(addr, 4)?;
        Some(u32::from_le_bytes(bytes.try_into().ok()?))
    }

    fn read_i32(&self, addr: usize) -> Option<i32> {
        let bytes = self.read_bytes(addr, 4)?;
        Some(i32::from_le_bytes(bytes.try_into().ok()?))
    }

    fn read_u64(&self, addr: usize) -> Option<u64> {
        let bytes = self.read_bytes(addr, 8)?;
        Some(u64::from_le_bytes(bytes.try_into().ok()?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryConstructionRoot {
    pub tag_db_id: u32,
    pub country_ptr: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateRoot {
    pub state_id: u32,
    pub state_ptr: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProvinceRoot {
    pub province_id: u32,
    pub province_ptr: usize,
    pub state_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructionQueueRow {
    pub tag_db_id: u32,
    pub line_id: u32,
    pub template_id: Option<u32>,
    pub active_factories: i32,
    pub cost_raw: i32,
    pub produced_raw: i32,
    pub priority: u32,
    pub target_state_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildingRow {
    pub scope: u32,
    pub location_id: u32,
    pub state_id: u32,
    pub type_id: u32,
    pub level: u32,
    pub healthy_level: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateResourceRow {
    pub state_id: u32,
    pub resource_id: u32,
    pub amount_raw: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneFrame {
    pub game_hour_raw: i32,
    pub status_flags: u32,
    pub source_flags: u32,
    pub observed_country_roots: u32,
    pub observed_state_roots: u32,
    pub observed_province_roots: u32,
    pub construction_rows: Vec<ConstructionQueueRow>,
    pub building_rows: Vec<BuildingRow>,
    pub resource_rows: Vec<StateResourceRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateReport {
    pub ok: bool,
    pub status_flags: u32,
    pub source_flags: u32,
    pub construction_rows: usize,
    pub construction_rows_with_target_state: usize,
    pub building_rows: usize,
    pub resource_rows: usize,
    pub damaged_building_rows: usize,
    pub errors: Vec<String>,
}

pub fn production_manager_from_country<M: LaneMemory>(
    mem: &M,
    country_ptr: usize,
) -> Option<usize> {
    let manager = mem.read_u64(country_ptr.checked_add(OBJ_PROD_MANAGER)?)? as usize;
    is_heap_ptr(manager).then_some(manager)
}

pub fn read_country_construction_queue<M: LaneMemory>(
    mem: &M,
    root: CountryConstructionRoot,
) -> Option<Vec<ConstructionQueueRow>> {
    let manager = production_manager_from_country(mem, root.country_ptr)?;
    read_construction_queue_from_manager(mem, manager, root.tag_db_id)
}

pub fn read_construction_queue_from_manager<M: LaneMemory>(
    mem: &M,
    manager: usize,
    tag_db_id: u32,
) -> Option<Vec<ConstructionQueueRow>> {
    if tag_db_id == 0 || !is_heap_ptr(manager) {
        return None;
    }
    let begin = mem.read_u64(manager.checked_add(CONSTRUCTION_BEGIN)?)? as usize;
    let cap = mem.read_u32(manager.checked_add(CONSTRUCTION_CAP)?)?;
    let size = mem.read_u32(manager.checked_add(CONSTRUCTION_SIZE)?)?;
    if size > cap || cap > MAX_PRODUCTION_VECTOR_ENTRIES {
        return None;
    }
    if size == 0 {
        return Some(Vec::new());
    }
    if !is_heap_ptr(begin) {
        return None;
    }
    let byte_len = (size as usize).checked_mul(CONSTRUCTION_STRIDE)?;
    let raw = mem.read_bytes(begin, byte_len)?;
    if raw.len() != byte_len {
        return None;
    }

    let mut rows = Vec::new();
    for offset in (0..byte_len).step_by(CONSTRUCTION_STRIDE) {
        let line_ptr = read_u64_from_slice(&raw, offset)? as usize;
        if !is_heap_ptr(line_ptr) {
            continue;
        }
        if mem.read_u32(line_ptr.checked_add(LINE_TYPE)?)? != LINE_TYPE_CONSTRUCTION {
            continue;
        }
        let active_factories = mem.read_i32(line_ptr.checked_add(LINE_ACTIVE_FACTORIES)?)?;
        if !(0..=MAX_ACTIVE_FACTORIES).contains(&active_factories) {
            continue;
        }
        let cost_raw = read_production_fixed_raw(mem, line_ptr.checked_add(LINE_COST)?)?;
        let produced_raw = read_production_fixed_raw(mem, line_ptr.checked_add(LINE_PRODUCED)?)?;
        if cost_raw < 0 || produced_raw < 0 {
            continue;
        }
        let sub_ptr = mem.read_u64(line_ptr.checked_add(LINE_CONSTRUCTION_SUB)?)? as usize;
        let template_id = if is_heap_ptr(sub_ptr) {
            mem.read_u32(sub_ptr.checked_add(CONSTRUCTION_TEMPLATE_ID)?)
                .filter(|id| *id > 0 && *id <= MAX_CONSTRUCTION_TEMPLATE_ID)
        } else {
            None
        };
        rows.push(ConstructionQueueRow {
            tag_db_id,
            line_id: mem.read_u32(line_ptr.checked_add(LINE_ID)?)?,
            template_id,
            active_factories,
            cost_raw,
            produced_raw,
            priority: (offset / CONSTRUCTION_STRIDE) as u32,
            target_state_id: None,
        });
    }
    Some(rows)
}

pub fn read_state_building_rows<M: LaneMemory>(
    mem: &M,
    module_base: usize,
    root: StateRoot,
) -> Option<Vec<BuildingRow>> {
    read_building_rows_for_location(
        mem,
        module_base,
        SCOPE_STATE,
        root.state_id,
        root.state_id,
        root.state_ptr,
        STATE_OFF_BUILDINGS_VEC,
    )
}

pub fn read_province_building_rows<M: LaneMemory>(
    mem: &M,
    module_base: usize,
    root: ProvinceRoot,
) -> Option<Vec<BuildingRow>> {
    read_building_rows_for_location(
        mem,
        module_base,
        SCOPE_PROVINCE,
        root.province_id,
        root.state_id,
        root.province_ptr,
        CPROVINCE_OFF_BUILDINGS_VEC,
    )
}

pub fn read_building_rows_for_location<M: LaneMemory>(
    mem: &M,
    module_base: usize,
    scope: u32,
    location_id: u32,
    state_id: u32,
    obj_base: usize,
    vec_off: usize,
) -> Option<Vec<BuildingRow>> {
    if !is_heap_ptr(obj_base) {
        return None;
    }
    if scope == SCOPE_STATE && location_id != state_id {
        return None;
    }
    let header = mem.read_bytes(obj_base.checked_add(vec_off)?, 16)?;
    if header.len() != 16 {
        return None;
    }
    let begin = read_u64_from_slice(&header, 0)? as usize;
    let packed = read_u64_from_slice(&header, 8)?;
    let logical = (packed >> 32) as u32;
    if logical > BUILDING_VECTOR_MAX {
        return None;
    }
    if logical == 0 {
        return Some(Vec::new());
    }
    if !is_heap_ptr(begin) {
        return None;
    }
    let byte_len = (logical as usize).checked_mul(8)?;
    let ptrs = mem.read_bytes(begin, byte_len)?;
    if ptrs.len() != byte_len {
        return None;
    }
    let cbuilding_vtable = module_base.checked_add(CBUILDING_VTABLE_RVA)?;
    let mut by_type: BTreeMap<u32, BuildingAccumulator> = BTreeMap::new();
    for index in 0..logical {
        let offset = (index as usize).checked_mul(8)?;
        let building_ptr = read_u64_from_slice(&ptrs, offset)? as usize;
        if !is_heap_ptr(building_ptr) {
            continue;
        }
        if mem.read_u64(building_ptr)? as usize != cbuilding_vtable {
            continue;
        }
        let type_id = mem.read_u32(building_ptr.checked_add(CBUILDING_OFF_TYPE)?)?;
        let level = mem.read_u16(building_ptr.checked_add(CBUILDING_OFF_LEVEL)?)? as u32;
        if type_id == 0 || level > BUILDING_LEVEL_SANE_MAX {
            continue;
        }
        let healthy_level = mem
            .read_u16(building_ptr.checked_add(CBUILDING_OFF_HEALTHY_LEVEL)?)
            .map(u32::from)
            .filter(|healthy| *healthy <= level && *healthy <= BUILDING_LEVEL_SANE_MAX);
        by_type
            .entry(type_id)
            .or_default()
            .add(level, healthy_level);
    }
    Some(
        by_type
            .into_iter()
            .map(|(type_id, acc)| BuildingRow {
                scope,
                location_id,
                state_id,
                type_id,
                level: acc.level,
                healthy_level: acc.healthy_level(),
            })
            .collect(),
    )
}

pub fn read_state_resource_rows<M: LaneMemory>(
    mem: &M,
    root: StateRoot,
) -> Option<Vec<StateResourceRow>> {
    if !is_heap_ptr(root.state_ptr) {
        return None;
    }
    let state_def = mem.read_u64(root.state_ptr.checked_add(STATE_OFF_DEF)?)? as usize;
    if !is_heap_ptr(state_def) {
        return None;
    }
    let data = mem.read_u64(state_def.checked_add(STATE_DEF_OFF_RESOURCES_PTR)?)? as usize;
    let size = mem.read_u32(state_def.checked_add(STATE_DEF_OFF_RESOURCES_SIZE)?)?;
    let cap = mem.read_u32(state_def.checked_add(STATE_DEF_OFF_RESOURCES_CAP)?)?;
    let logical_rows = state_resource_logical_rows(size, cap)?;
    if logical_rows == 0 {
        return Some(Vec::new());
    }
    if !is_heap_ptr(data) {
        return None;
    }
    let byte_len = (logical_rows as usize).checked_mul(STATE_RESOURCE_STRIDE)?;
    let rows = mem.read_bytes(data, byte_len)?;
    if rows.len() != byte_len {
        return None;
    }
    let mut resources = BTreeMap::new();
    for index in 0..logical_rows {
        let offset = (index as usize).checked_mul(STATE_RESOURCE_STRIDE)?;
        let resource_id = read_u32_from_slice(&rows, offset.checked_add(8)?)?;
        if resource_id == STATE_RESOURCE_NO_ID {
            continue;
        }
        let amount_raw = read_i32_from_slice(&rows, offset)?;
        if !(0..=STATE_RESOURCE_MAX_ABS_RAW).contains(&amount_raw) {
            return None;
        }
        resources.insert(resource_id, amount_raw);
    }
    Some(
        resources
            .into_iter()
            .map(|(resource_id, amount_raw)| StateResourceRow {
                state_id: root.state_id,
                resource_id,
                amount_raw,
            })
            .collect(),
    )
}

pub fn encode_lane_frame(frame: &LaneFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        HEADER_LEN
            + frame.construction_rows.len() * CONSTRUCTION_ROW_LEN
            + frame.building_rows.len() * BUILDING_ROW_LEN
            + frame.resource_rows.len() * RESOURCE_ROW_LEN,
    );
    out.extend_from_slice(&COMPACT_MAGIC);
    out.extend_from_slice(&COMPACT_VERSION.to_le_bytes());
    out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&DOMAIN_ID.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&frame.status_flags.to_le_bytes());
    out.extend_from_slice(&frame.game_hour_raw.to_le_bytes());
    out.extend_from_slice(
        &(frame.construction_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    out.extend_from_slice(&(frame.building_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&(frame.resource_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&(CONSTRUCTION_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&(BUILDING_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&(RESOURCE_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&frame.observed_country_roots.to_le_bytes());
    out.extend_from_slice(&frame.observed_state_roots.to_le_bytes());
    out.extend_from_slice(&frame.observed_province_roots.to_le_bytes());
    out.extend_from_slice(&frame.source_flags.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for row in &frame.construction_rows {
        write_construction_row(&mut out, row);
    }
    for row in &frame.building_rows {
        write_building_row(&mut out, row);
    }
    for row in &frame.resource_rows {
        write_resource_row(&mut out, row);
    }
    out
}

pub fn decode_lane_frame(bytes: &[u8]) -> Result<LaneFrame, String> {
    if bytes.len() < HEADER_LEN {
        return Err(format!("payload too short: {}", bytes.len()));
    }
    if &bytes[0..4] != COMPACT_MAGIC.as_slice() {
        return Err("magic mismatch".to_string());
    }
    if read_u16_le(bytes, 4) != COMPACT_VERSION {
        return Err("version mismatch".to_string());
    }
    let header_len = read_u16_le(bytes, 6) as usize;
    if header_len != HEADER_LEN {
        return Err(format!("unexpected header_len {header_len}"));
    }
    if read_u16_le(bytes, 8) != DOMAIN_ID {
        return Err("domain id mismatch".to_string());
    }
    if read_u16_le(bytes, 10) != 0 {
        return Err("schema reserved field is nonzero".to_string());
    }
    let status_flags = read_u32_le(bytes, 12);
    let game_hour_raw = read_i32_le(bytes, 16);
    let construction_count = read_u32_le(bytes, 20) as usize;
    let building_count = read_u32_le(bytes, 24) as usize;
    let resource_count = read_u32_le(bytes, 28) as usize;
    if read_u16_le(bytes, 32) as usize != CONSTRUCTION_ROW_LEN
        || read_u16_le(bytes, 34) as usize != BUILDING_ROW_LEN
        || read_u16_le(bytes, 36) as usize != RESOURCE_ROW_LEN
    {
        return Err("row length mismatch".to_string());
    }
    if read_u16_le(bytes, 38) != 0 {
        return Err("row reserved field is nonzero".to_string());
    }
    let observed_country_roots = read_u32_le(bytes, 40);
    let observed_state_roots = read_u32_le(bytes, 44);
    let observed_province_roots = read_u32_le(bytes, 48);
    let source_flags = read_u32_le(bytes, 52);
    if read_u32_le(bytes, 56) != 0 || read_u32_le(bytes, 60) != 0 {
        return Err("header reserved tail is nonzero".to_string());
    }
    let expected_len = HEADER_LEN
        .saturating_add(construction_count.saturating_mul(CONSTRUCTION_ROW_LEN))
        .saturating_add(building_count.saturating_mul(BUILDING_ROW_LEN))
        .saturating_add(resource_count.saturating_mul(RESOURCE_ROW_LEN));
    if bytes.len() != expected_len {
        return Err(format!(
            "payload length {} != expected {expected_len}",
            bytes.len()
        ));
    }

    let mut cursor = HEADER_LEN;
    let mut construction_rows = Vec::with_capacity(construction_count);
    for index in 0..construction_count {
        let row = read_construction_row(bytes, cursor);
        validate_construction_row(index, &row)?;
        construction_rows.push(row);
        cursor += CONSTRUCTION_ROW_LEN;
    }
    let mut building_rows = Vec::with_capacity(building_count);
    for index in 0..building_count {
        let row = read_building_row(bytes, cursor);
        validate_building_row(index, &row)?;
        building_rows.push(row);
        cursor += BUILDING_ROW_LEN;
    }
    let mut resource_rows = Vec::with_capacity(resource_count);
    for index in 0..resource_count {
        let row = read_resource_row(bytes, cursor);
        validate_resource_row(index, &row)?;
        resource_rows.push(row);
        cursor += RESOURCE_ROW_LEN;
    }

    Ok(LaneFrame {
        game_hour_raw,
        status_flags,
        source_flags,
        observed_country_roots,
        observed_state_roots,
        observed_province_roots,
        construction_rows,
        building_rows,
        resource_rows,
    })
}

pub fn gate_lane_frame(bytes: &[u8]) -> GateReport {
    match decode_lane_frame(bytes) {
        Ok(frame) => gate_decoded_frame(&frame),
        Err(error) => GateReport {
            ok: false,
            status_flags: 0,
            source_flags: 0,
            construction_rows: 0,
            construction_rows_with_target_state: 0,
            building_rows: 0,
            resource_rows: 0,
            damaged_building_rows: 0,
            errors: vec![error],
        },
    }
}

pub fn gate_decoded_frame(frame: &LaneFrame) -> GateReport {
    let mut errors = Vec::new();
    let blocking_flags = STATUS_WORKER_PANIC | STATUS_SOURCE_UNAVAILABLE | STATUS_PARTIAL_DATA;
    if frame.status_flags & blocking_flags != 0 {
        errors.push(format!(
            "blocking status_flags set: {:#x}",
            frame.status_flags
        ));
    }
    if frame.source_flags & SOURCE_FLAG_DLL_NATIVE == 0 {
        errors.push("source_flags missing DLL-native bit".to_string());
    }
    if frame.building_rows.is_empty() {
        errors.push("no building rows emitted".to_string());
    }
    if frame.resource_rows.is_empty() {
        errors.push("no state resource rows emitted".to_string());
    }
    let construction_rows_with_target_state = frame
        .construction_rows
        .iter()
        .filter(|row| row.target_state_id.is_some())
        .count();
    let target_state_unavailable =
        frame.source_flags & SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE != 0;
    if !frame.construction_rows.is_empty() {
        if construction_rows_with_target_state == 0 && !target_state_unavailable {
            errors.push(
                "construction target_state_id absent without target-state-unavailable source flag"
                    .to_string(),
            );
        }
        if target_state_unavailable && construction_rows_with_target_state > 0 {
            errors.push(
                "construction target_state_id emitted while target-state-unavailable source flag is set"
                    .to_string(),
            );
        }
        if construction_rows_with_target_state > 0
            && construction_rows_with_target_state != frame.construction_rows.len()
        {
            errors.push(
                "construction target_state_id partially emitted; fill every construction row or badge unavailable"
                    .to_string(),
            );
        }
    }
    let mut seen_construction = BTreeSet::new();
    for (index, row) in frame.construction_rows.iter().enumerate() {
        if !seen_construction.insert((row.tag_db_id, row.line_id, row.priority)) {
            errors.push(format!(
                "construction row {index} duplicates tag/line/priority"
            ));
        }
    }
    let mut seen_buildings = BTreeSet::new();
    let mut damaged_building_rows = 0usize;
    for (index, row) in frame.building_rows.iter().enumerate() {
        if !seen_buildings.insert((row.scope, row.location_id, row.type_id)) {
            errors.push(format!(
                "building row {index} duplicates scope/location/type"
            ));
        }
        if row.healthy_level.is_some_and(|healthy| healthy < row.level) {
            damaged_building_rows = damaged_building_rows.saturating_add(1);
        }
    }
    let mut seen_resources = BTreeSet::new();
    for (index, row) in frame.resource_rows.iter().enumerate() {
        if !seen_resources.insert((row.state_id, row.resource_id)) {
            errors.push(format!("resource row {index} duplicates state/resource"));
        }
    }
    GateReport {
        ok: errors.is_empty(),
        status_flags: frame.status_flags,
        source_flags: frame.source_flags,
        construction_rows: frame.construction_rows.len(),
        construction_rows_with_target_state,
        building_rows: frame.building_rows.len(),
        resource_rows: frame.resource_rows.len(),
        damaged_building_rows,
        errors,
    }
}

fn read_production_fixed_raw<M: LaneMemory>(mem: &M, addr: usize) -> Option<i32> {
    mem.read_i32(addr)
        .filter(|value| value.abs() <= MAX_PRODUCTION_LINE_FIXED_RAW)
}

fn state_resource_logical_rows(size: u32, cap: u32) -> Option<u32> {
    if size == 0 {
        return Some(0);
    }
    if size > STATE_RESOURCE_MAX_ROWS {
        return None;
    }
    let logical_rows = if cap != 0 && cap < size { cap } else { size };
    (logical_rows <= STATE_RESOURCE_MAX_ROWS).then_some(logical_rows)
}

fn write_construction_row(out: &mut Vec<u8>, row: &ConstructionQueueRow) {
    let mut flags = 0u32;
    if row.template_id.is_some() {
        flags |= CONSTRUCTION_FLAG_TEMPLATE_PRESENT;
    }
    if row.target_state_id.is_some() {
        flags |= CONSTRUCTION_FLAG_TARGET_STATE_PRESENT;
    }
    out.extend_from_slice(&row.tag_db_id.to_le_bytes());
    out.extend_from_slice(&row.line_id.to_le_bytes());
    out.extend_from_slice(&row.template_id.unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&row.active_factories.to_le_bytes());
    out.extend_from_slice(&row.cost_raw.to_le_bytes());
    out.extend_from_slice(&row.produced_raw.to_le_bytes());
    out.extend_from_slice(&row.priority.to_le_bytes());
    out.extend_from_slice(&row.target_state_id.unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

fn write_building_row(out: &mut Vec<u8>, row: &BuildingRow) {
    let mut flags = 0u32;
    if let Some(healthy) = row.healthy_level {
        flags |= BUILDING_FLAG_HEALTHY_PRESENT;
        if healthy < row.level {
            flags |= BUILDING_FLAG_DAMAGED;
        }
    }
    out.extend_from_slice(&row.scope.to_le_bytes());
    out.extend_from_slice(&row.location_id.to_le_bytes());
    out.extend_from_slice(&row.state_id.to_le_bytes());
    out.extend_from_slice(&row.type_id.to_le_bytes());
    out.extend_from_slice(&row.level.to_le_bytes());
    out.extend_from_slice(&row.healthy_level.unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

fn write_resource_row(out: &mut Vec<u8>, row: &StateResourceRow) {
    out.extend_from_slice(&row.state_id.to_le_bytes());
    out.extend_from_slice(&row.resource_id.to_le_bytes());
    out.extend_from_slice(&row.amount_raw.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

fn read_construction_row(bytes: &[u8], offset: usize) -> ConstructionQueueRow {
    let template_raw = read_u32_le(bytes, offset + 8);
    let target_state_raw = read_u32_le(bytes, offset + 28);
    let flags = read_u32_le(bytes, offset + 32);
    ConstructionQueueRow {
        tag_db_id: read_u32_le(bytes, offset),
        line_id: read_u32_le(bytes, offset + 4),
        template_id: (flags & CONSTRUCTION_FLAG_TEMPLATE_PRESENT != 0).then_some(template_raw),
        active_factories: read_i32_le(bytes, offset + 12),
        cost_raw: read_i32_le(bytes, offset + 16),
        produced_raw: read_i32_le(bytes, offset + 20),
        priority: read_u32_le(bytes, offset + 24),
        target_state_id: (flags & CONSTRUCTION_FLAG_TARGET_STATE_PRESENT != 0)
            .then_some(target_state_raw),
    }
}

fn read_building_row(bytes: &[u8], offset: usize) -> BuildingRow {
    let healthy_raw = read_u32_le(bytes, offset + 20);
    let flags = read_u32_le(bytes, offset + 24);
    BuildingRow {
        scope: read_u32_le(bytes, offset),
        location_id: read_u32_le(bytes, offset + 4),
        state_id: read_u32_le(bytes, offset + 8),
        type_id: read_u32_le(bytes, offset + 12),
        level: read_u32_le(bytes, offset + 16),
        healthy_level: (flags & BUILDING_FLAG_HEALTHY_PRESENT != 0).then_some(healthy_raw),
    }
}

fn read_resource_row(bytes: &[u8], offset: usize) -> StateResourceRow {
    StateResourceRow {
        state_id: read_u32_le(bytes, offset),
        resource_id: read_u32_le(bytes, offset + 4),
        amount_raw: read_i32_le(bytes, offset + 8),
    }
}

fn validate_construction_row(index: usize, row: &ConstructionQueueRow) -> Result<(), String> {
    if row.tag_db_id == 0 {
        return Err(format!("construction row {index} has zero tag_db_id"));
    }
    if row.template_id == Some(0) {
        return Err(format!("construction row {index} has zero template_id"));
    }
    if let Some(template_id) = row.template_id {
        if template_id > MAX_CONSTRUCTION_TEMPLATE_ID {
            return Err(format!(
                "construction row {index} invalid template_id {template_id}"
            ));
        }
    }
    if !(0..=MAX_ACTIVE_FACTORIES).contains(&row.active_factories) {
        return Err(format!(
            "construction row {index} invalid active_factories {}",
            row.active_factories
        ));
    }
    if row.cost_raw < 0 || row.produced_raw < 0 {
        return Err(format!(
            "construction row {index} has negative progress fields"
        ));
    }
    if let Some(state_id) = row.target_state_id {
        if state_id == 0 || state_id > MAX_STATE_ID {
            return Err(format!(
                "construction row {index} invalid target_state_id {state_id}"
            ));
        }
    }
    Ok(())
}

fn validate_building_row(index: usize, row: &BuildingRow) -> Result<(), String> {
    if row.scope != SCOPE_STATE && row.scope != SCOPE_PROVINCE {
        return Err(format!("building row {index} invalid scope {}", row.scope));
    }
    if row.location_id == 0 {
        return Err(format!("building row {index} has zero location_id"));
    }
    if row.scope == SCOPE_STATE && row.location_id != row.state_id {
        return Err(format!(
            "building row {index} state-scope location/state mismatch"
        ));
    }
    if row.scope == SCOPE_PROVINCE && row.location_id > MAX_PROVINCE_ID {
        return Err(format!(
            "building row {index} invalid province_id {}",
            row.location_id
        ));
    }
    if row.state_id == 0 || row.state_id > MAX_STATE_ID {
        return Err(format!(
            "building row {index} invalid state_id {}",
            row.state_id
        ));
    }
    if row.type_id == 0 {
        return Err(format!("building row {index} has zero type_id"));
    }
    if row.level > BUILDING_LEVEL_SANE_MAX {
        return Err(format!("building row {index} invalid level {}", row.level));
    }
    if let Some(healthy) = row.healthy_level {
        if healthy > row.level {
            return Err(format!(
                "building row {index} healthy_level {healthy} > level {}",
                row.level
            ));
        }
    }
    Ok(())
}

fn validate_resource_row(index: usize, row: &StateResourceRow) -> Result<(), String> {
    if row.state_id == 0 || row.state_id > MAX_STATE_ID {
        return Err(format!(
            "resource row {index} invalid state_id {}",
            row.state_id
        ));
    }
    if row.resource_id == 0 || row.resource_id == STATE_RESOURCE_NO_ID {
        return Err(format!(
            "resource row {index} invalid resource_id {}",
            row.resource_id
        ));
    }
    if !(0..=STATE_RESOURCE_MAX_ABS_RAW).contains(&row.amount_raw) {
        return Err(format!(
            "resource row {index} invalid amount_raw {}",
            row.amount_raw
        ));
    }
    Ok(())
}

fn is_heap_ptr(value: usize) -> bool {
    value >= HEAP_LO && value < HEAP_HI && value % POINTER_WIDTH == 0
}

#[derive(Default)]
struct BuildingAccumulator {
    level: u32,
    healthy: u32,
    healthy_complete: bool,
    seen: bool,
}

impl BuildingAccumulator {
    fn add(&mut self, level: u32, healthy_level: Option<u32>) {
        self.level = self.level.saturating_add(level);
        match healthy_level {
            Some(healthy) if !self.seen || self.healthy_complete => {
                self.healthy = self.healthy.saturating_add(healthy);
                self.healthy_complete = true;
            }
            _ => {
                self.healthy_complete = false;
            }
        }
        self.seen = true;
    }

    fn healthy_level(&self) -> Option<u32> {
        self.healthy_complete.then_some(self.healthy)
    }
}

fn read_u64_from_slice(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
    ))
}

fn read_u32_from_slice(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn read_i32_from_slice(bytes: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("u16 range"))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("i32 range"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct FakeMemory {
        bytes: BTreeMap<usize, u8>,
    }

    impl FakeMemory {
        fn write_u16(&mut self, addr: usize, value: u16) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_u32(&mut self, addr: usize, value: u32) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_i32(&mut self, addr: usize, value: i32) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_u64(&mut self, addr: usize, value: u64) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_bytes(&mut self, addr: usize, bytes: &[u8]) {
            for (index, byte) in bytes.iter().copied().enumerate() {
                self.bytes.insert(addr + index, byte);
            }
        }
    }

    impl LaneMemory for FakeMemory {
        fn read_u8(&self, addr: usize) -> Option<u8> {
            self.bytes.get(&addr).copied()
        }
    }

    #[test]
    fn slot_ready_domain_id_avoids_current_shared_domain_ids() {
        let current_shared_domain_ids = [
            10u16, 11, 12, 13, 14, 15, 16, 17, 20, 21, 22, 23, 24, 26, 27, 28, 29, 30, 31, 32, 33,
            34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
        ];

        assert!(
            !current_shared_domain_ids.contains(&DOMAIN_ID),
            "lane-local proposed domain id {DOMAIN_ID} collides with current shared compact domains"
        );
    }

    #[test]
    fn extractor_reads_construction_queue_and_skips_invalid_rows() {
        let manager = 0x1000_0000_0000usize;
        let vector = 0x1000_0000_1000usize;
        let good_line = 0x1000_0000_2000usize;
        let bad_line = 0x1000_0000_2100usize;
        let sub = 0x1000_0000_3000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(manager + CONSTRUCTION_BEGIN, vector as u64);
        mem.write_u32(manager + CONSTRUCTION_CAP, 2);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 2);
        mem.write_u64(vector, good_line as u64);
        mem.write_u64(vector + 8, bad_line as u64);
        mem.write_u32(good_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(good_line + LINE_ID, 77);
        mem.write_i32(good_line + LINE_ACTIVE_FACTORIES, 15);
        mem.write_i32(good_line + LINE_COST, 10_000_000);
        mem.write_i32(good_line + LINE_PRODUCED, 5_000_000);
        mem.write_u64(good_line + LINE_CONSTRUCTION_SUB, sub as u64);
        mem.write_u32(sub + CONSTRUCTION_TEMPLATE_ID, 20_013);
        mem.write_u32(bad_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_i32(bad_line + LINE_ACTIVE_FACTORIES, -1);

        let rows = read_construction_queue_from_manager(&mem, manager, 123).unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tag_db_id, 123);
        assert_eq!(rows[0].line_id, 77);
        assert_eq!(rows[0].template_id, Some(20_013));
        assert_eq!(rows[0].active_factories, 15);
        assert_eq!(rows[0].priority, 0);
    }

    #[test]
    fn extractor_reads_building_rows_with_damage_signal() {
        let base = 0x1400_0000usize;
        let state = 0x1000_0001_0000usize;
        let vector = 0x1000_0002_0000usize;
        let building = 0x1000_0003_0000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(state + STATE_OFF_BUILDINGS_VEC, vector as u64);
        mem.write_u64(state + STATE_OFF_BUILDINGS_VEC + 8, 1u64 << 32);
        mem.write_u64(vector, building as u64);
        mem.write_u64(building, (base + CBUILDING_VTABLE_RVA) as u64);
        mem.write_u32(building + CBUILDING_OFF_TYPE, 0x2FAB);
        mem.write_u16(building + CBUILDING_OFF_LEVEL, 5);
        mem.write_u16(building + CBUILDING_OFF_HEALTHY_LEVEL, 4);

        let rows = read_state_building_rows(
            &mem,
            base,
            StateRoot {
                state_id: 42,
                state_ptr: state,
            },
        )
        .unwrap();

        assert_eq!(
            rows,
            vec![BuildingRow {
                scope: SCOPE_STATE,
                location_id: 42,
                state_id: 42,
                type_id: 0x2FAB,
                level: 5,
                healthy_level: Some(4),
            }]
        );
    }

    #[test]
    fn extractor_reads_state_resources() {
        let state = 0x1000_0010_0000usize;
        let def = 0x1000_0011_0000usize;
        let rows_ptr = 0x1000_0012_0000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(state + STATE_OFF_DEF, def as u64);
        mem.write_u64(def + STATE_DEF_OFF_RESOURCES_PTR, rows_ptr as u64);
        mem.write_u32(def + STATE_DEF_OFF_RESOURCES_SIZE, 1);
        mem.write_u32(def + STATE_DEF_OFF_RESOURCES_CAP, 1);
        mem.write_bytes(rows_ptr, &[0; STATE_RESOURCE_STRIDE]);
        mem.write_i32(rows_ptr, 2_500_000);
        mem.write_u32(rows_ptr + 8, 0x3E03);

        let rows = read_state_resource_rows(
            &mem,
            StateRoot {
                state_id: 64,
                state_ptr: state,
            },
        )
        .unwrap();

        assert_eq!(
            rows,
            vec![StateResourceRow {
                state_id: 64,
                resource_id: 0x3E03,
                amount_raw: 2_500_000,
            }]
        );
    }

    #[test]
    fn compact_frame_round_trips_and_gates() {
        let frame = sample_frame();
        let encoded = encode_lane_frame(&frame);
        let decoded = decode_lane_frame(&encoded).unwrap();
        assert_eq!(decoded, frame);
        let gate = gate_lane_frame(&encoded);
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.construction_rows, 1);
        assert_eq!(gate.construction_rows_with_target_state, 0);
        assert_eq!(gate.damaged_building_rows, 1);
    }

    #[test]
    fn compact_gate_rejects_invalid_construction_payload() {
        let mut frame = sample_frame();
        frame.construction_rows[0].active_factories = -1;
        let encoded = encode_lane_frame(&frame);
        let gate = gate_lane_frame(&encoded);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("active_factories")));
    }

    #[test]
    fn compact_frame_round_trips_target_state_when_present() {
        let mut frame = sample_frame();
        frame.source_flags &= !SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        frame.construction_rows[0].target_state_id = Some(419);

        let encoded = encode_lane_frame(&frame);
        let decoded = decode_lane_frame(&encoded).unwrap();

        assert_eq!(decoded.construction_rows[0].target_state_id, Some(419));
        let gate = gate_lane_frame(&encoded);
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.construction_rows_with_target_state, 1);
    }

    #[test]
    fn compact_gate_rejects_invalid_target_state_payload() {
        let mut frame = sample_frame();
        frame.source_flags &= !SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        frame.construction_rows[0].target_state_id = Some(0);

        let encoded = encode_lane_frame(&frame);
        let gate = gate_lane_frame(&encoded);

        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("target_state_id")));
    }

    #[test]
    fn compact_gate_requires_target_unavailable_badge_when_target_absent() {
        let mut frame = sample_frame();
        frame.source_flags &= !SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;

        let encoded = encode_lane_frame(&frame);
        let gate = gate_lane_frame(&encoded);

        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("target-state-unavailable")));
    }

    #[test]
    fn compact_gate_rejects_target_state_when_unavailable_badged() {
        let mut frame = sample_frame();
        frame.construction_rows[0].target_state_id = Some(419);

        let encoded = encode_lane_frame(&frame);
        let gate = gate_lane_frame(&encoded);

        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("target_state_id emitted")));
    }

    #[test]
    fn compact_gate_rejects_partial_target_state_payload() {
        let mut frame = sample_frame();
        frame.source_flags &= !SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        frame.construction_rows[0].target_state_id = Some(419);
        frame.construction_rows.push(ConstructionQueueRow {
            tag_db_id: 369,
            line_id: 8,
            template_id: Some(20_002),
            active_factories: 1,
            cost_raw: 7_200_000,
            produced_raw: 100_000,
            priority: 1,
            target_state_id: None,
        });

        let encoded = encode_lane_frame(&frame);
        let gate = gate_lane_frame(&encoded);

        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("partially emitted")));
    }

    #[test]
    fn compact_gate_rejects_incoherent_damage_signal() {
        let mut frame = sample_frame();
        frame.building_rows[0].healthy_level = Some(6);
        let encoded = encode_lane_frame(&frame);
        let gate = gate_lane_frame(&encoded);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("healthy_level")));
    }

    fn sample_frame() -> LaneFrame {
        LaneFrame {
            game_hour_raw: 123,
            status_flags: 0,
            source_flags: SOURCE_FLAG_DLL_NATIVE
                | SOURCE_FLAG_SHARED_COUNTRY_ROOTS
                | SOURCE_FLAG_SHARED_MAP_TOPOLOGY
                | SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE
                | SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
            observed_country_roots: 87,
            observed_state_roots: 1081,
            observed_province_roots: 13275,
            construction_rows: vec![ConstructionQueueRow {
                tag_db_id: 369,
                line_id: 7,
                template_id: Some(20_013),
                active_factories: 15,
                cost_raw: 10_000_000,
                produced_raw: 5_000_000,
                priority: 0,
                target_state_id: None,
            }],
            building_rows: vec![
                BuildingRow {
                    scope: SCOPE_STATE,
                    location_id: 42,
                    state_id: 42,
                    type_id: 0x2FAB,
                    level: 5,
                    healthy_level: Some(4),
                },
                BuildingRow {
                    scope: SCOPE_PROVINCE,
                    location_id: 2068,
                    state_id: 42,
                    type_id: 0x4CC1,
                    level: 2,
                    healthy_level: Some(2),
                },
            ],
            resource_rows: vec![StateResourceRow {
                state_id: 42,
                resource_id: 0x3E03,
                amount_raw: 2_500_000,
            }],
        }
    }
}
