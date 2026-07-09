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

pub const STATUS_WORKER_PANIC: u32 = compact_contract::COMPACT_STATUS_WORKER_PANIC;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
pub const STATUS_PARTIAL_DATA: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
pub const STATUS_DELTA_DATA: u32 = compact_contract::COMPACT_STATUS_DELTA_DATA;

pub const SOURCE_FLAG_DLL_NATIVE: u32 = 1;
pub const SOURCE_FLAG_SHARED_COUNTRY_ROOTS: u32 = 1 << 1;
pub const SOURCE_FLAG_SHARED_MAP_TOPOLOGY: u32 = 1 << 2;
pub const SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE: u32 = 1 << 3;
pub const SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE: u32 = 1 << 4;

pub const BUILD_PROFILE: &str = compact_contract::build_profile::DOMAIN48_BUILD_PROFILE;
pub const HEAP_LO: usize = compact_contract::build_profile::DOMAIN48_HEAP_LO;
pub const HEAP_HI: usize = compact_contract::build_profile::DOMAIN48_HEAP_HI;
pub const POINTER_WIDTH: usize = compact_contract::build_profile::DOMAIN48_POINTER_WIDTH;

pub const STATE_VTABLE_RVA: usize = compact_contract::build_profile::STATE_VTABLE_RVA;
pub const STATE_OFF_SID: usize = compact_contract::build_profile::STATE_OFF_SID;
pub const STATE_OFF_DEF: usize = compact_contract::build_profile::STATE_OFF_DEF;
pub const STATE_OFF_BUILDINGS_VEC: usize = compact_contract::build_profile::STATE_OFF_BUILDINGS_VEC;
pub const STATE_DEF_OFF_RESOURCES_PTR: usize =
    compact_contract::build_profile::STATE_DEF_OFF_RESOURCES_PTR;
pub const STATE_DEF_OFF_RESOURCES_SIZE: usize =
    compact_contract::build_profile::STATE_DEF_OFF_RESOURCES_SIZE;
pub const STATE_DEF_OFF_RESOURCES_CAP: usize =
    compact_contract::build_profile::STATE_DEF_OFF_RESOURCES_CAP;
pub const STATE_RESOURCE_STRIDE: usize = compact_contract::build_profile::STATE_RESOURCE_STRIDE;
pub const STATE_RESOURCE_NO_ID: u32 = compact_contract::build_profile::STATE_RESOURCE_NO_ID;
pub const STATE_RESOURCE_MAX_ROWS: u32 = compact_contract::build_profile::STATE_RESOURCE_MAX_ROWS;
pub const STATE_RESOURCE_MAX_ABS_RAW: i32 =
    compact_contract::build_profile::STATE_RESOURCE_MAX_ABS_RAW;

pub const CPROVINCE_VTABLE_RVA: usize = compact_contract::build_profile::CPROVINCE_VTABLE_RVA;
pub const CPROVINCE_OFF_ID: usize = compact_contract::build_profile::CPROVINCE_OFF_ID;
pub const CPROVINCE_OFF_STATE: usize = compact_contract::build_profile::CPROVINCE_OFF_STATE;
pub const CPROVINCE_OFF_BUILDINGS_VEC: usize =
    compact_contract::build_profile::CPROVINCE_OFF_BUILDINGS_VEC;

pub const CBUILDING_VTABLE_RVA: usize = compact_contract::build_profile::CBUILDING_VTABLE_RVA;
pub const CBUILDING_OFF_TYPE: usize = compact_contract::build_profile::CBUILDING_OFF_TYPE;
pub const CBUILDING_OFF_LEVEL: usize = compact_contract::build_profile::CBUILDING_OFF_LEVEL;
pub const CBUILDING_OFF_HEALTHY_LEVEL: usize =
    compact_contract::build_profile::CBUILDING_OFF_HEALTHY_LEVEL;
pub const BUILDING_VECTOR_MAX: u32 = compact_contract::build_profile::BUILDING_VECTOR_MAX;
pub const BUILDING_LEVEL_SANE_MAX: u32 = compact_contract::build_profile::BUILDING_LEVEL_SANE_MAX;

pub const OBJ_PROD_MANAGER: usize = compact_contract::build_profile::OBJ_PROD_MANAGER;
pub const CONSTRUCTION_BEGIN: usize = compact_contract::build_profile::CONSTRUCTION_BEGIN;
pub const CONSTRUCTION_CAP: usize = compact_contract::build_profile::CONSTRUCTION_CAP;
pub const CONSTRUCTION_SIZE: usize = compact_contract::build_profile::CONSTRUCTION_SIZE;
pub const CONSTRUCTION_STRIDE: usize = compact_contract::build_profile::CONSTRUCTION_STRIDE;
pub const LINE_TYPE: usize = compact_contract::build_profile::LINE_TYPE;
pub const LINE_ID: usize = compact_contract::build_profile::LINE_ID;
pub const LINE_ACTIVE_FACTORIES: usize = compact_contract::build_profile::LINE_ACTIVE_FACTORIES;
pub const LINE_COST: usize = compact_contract::build_profile::LINE_COST;
pub const LINE_PRODUCED: usize = compact_contract::build_profile::LINE_PRODUCED;
pub const LINE_CONSTRUCTION_SUB: usize = compact_contract::build_profile::LINE_CONSTRUCTION_SUB;
// W4: CBuildingProductionLine+0x58 u32 created_date (native hour-count, same
// epoch as the CGameState date word). D5-validated 105/105 lines.
pub const LINE_CREATED_DATE: usize = compact_contract::build_profile::LINE_CREATED_DATE;
// Fail-closed ceiling on a created_date raw hour-count. The date epoch offset is
// 43799999 (raw = calendar_hours + 43799999), so the smallest plausible raw is
// well above zero and no live line can approach u32::MAX; a raw at/above this
// bound (or exactly 0) is a torn/uninitialised read and the field is badged
// absent rather than emitted. 100_000_000 comfortably exceeds every D04 date
// (gate 1944.2.11.18 == 60830441) with headroom for late-game saves.
pub const MAX_CONSTRUCTION_CREATED_DATE_RAW: u32 = 100_000_000;
pub const LINE_TYPE_CONSTRUCTION: u32 = compact_contract::build_profile::LINE_TYPE_CONSTRUCTION;
pub const CONSTRUCTION_TEMPLATE_ID: usize =
    compact_contract::build_profile::CONSTRUCTION_TEMPLATE_ID;
pub const CONSTRUCTION_TARGET_LOCATION_PTR: usize =
    compact_contract::build_profile::CONSTRUCTION_TARGET_LOCATION_PTR;
pub const TARGET_LOCATION_ID: usize = compact_contract::build_profile::TARGET_LOCATION_ID;
pub const MAX_CONSTRUCTION_TEMPLATE_ID: u32 =
    compact_contract::build_profile::MAX_CONSTRUCTION_TEMPLATE_ID;
pub const MAX_PRODUCTION_VECTOR_ENTRIES: u32 =
    compact_contract::build_profile::MAX_PRODUCTION_VECTOR_ENTRIES;
pub const MAX_PRODUCTION_LINE_FIXED_RAW: i32 =
    compact_contract::build_profile::MAX_PRODUCTION_LINE_FIXED_RAW;
pub const MAX_ACTIVE_FACTORIES: i32 = compact_contract::build_profile::MAX_ACTIVE_FACTORIES;

pub const SCOPE_STATE: u32 = compact_contract::build_profile::SCOPE_STATE;
pub const SCOPE_PROVINCE: u32 = compact_contract::build_profile::SCOPE_PROVINCE;
pub const MAX_STATE_ID: u32 = compact_contract::build_profile::MAX_STATE_ID;
pub const MAX_PROVINCE_ID: u32 = compact_contract::build_profile::MAX_PROVINCE_ID;

pub const CONSTRUCTION_FLAG_TEMPLATE_PRESENT: u32 = 1;
pub const CONSTRUCTION_FLAG_TARGET_STATE_PRESENT: u32 = 1 << 1;
// W4: created_date_raw is carried in the previously-reserved trailing row u32
// (row byte offset 36), gated by this presence bit. Repurposing the reserved
// slot keeps CONSTRUCTION_ROW_LEN == 40 (no wire-size change): the decoder's
// row-length header field and total-length check are unaffected, so this is
// strictly safer than a 40->44 row growth (Codex-reviewed).
pub const CONSTRUCTION_FLAG_CREATED_DATE_PRESENT: u32 = 1 << 2;
// Every construction-row flag bit that is defined. An unknown bit in a decoded
// row is a malformed/forward-incompatible payload and fails the gate closed.
pub const CONSTRUCTION_ROW_KNOWN_FLAGS: u32 = CONSTRUCTION_FLAG_TEMPLATE_PRESENT
    | CONSTRUCTION_FLAG_TARGET_STATE_PRESENT
    | CONSTRUCTION_FLAG_CREATED_DATE_PRESENT;
pub const BUILDING_FLAG_HEALTHY_PRESENT: u32 = 1;
pub const BUILDING_FLAG_DAMAGED: u32 = 1 << 1;
pub const BUILDING_ROW_KNOWN_FLAGS: u32 = BUILDING_FLAG_HEALTHY_PRESENT | BUILDING_FLAG_DAMAGED;
pub const ROW_ID_DELETED: u32 = 1 << 31;

pub const PROVINCE_CONSTRUCTION_TEMPLATE_IDS: [u32; 4] = [12_790, 19_649, 22_353, 22_354];

/// Bulk-read window covering every CBuilding field the building walk reads:
/// vtable `[0x00,0x08)`, type `[0x08,0x0C)`, level `[0x40,0x42)`, healthy_level
/// `[0x42,0x44)`. `N = CBUILDING_OFF_HEALTHY_LEVEL + 2` = the far field's end, so
/// the window's region gate is identical to a scalar read of the last field.
pub const CBUILDING_WINDOW_LEN: usize = CBUILDING_OFF_HEALTHY_LEVEL + 2;

/// Bulk-read window covering every CBuildingProductionLine field the queue walk
/// reads on the line object itself: type `[0x08,0x0C)`, id `[0x0C,0x10)`,
/// active_factories `[0x18,0x1C)`, cost `[0x28,0x2C)`, produced `[0x38,0x3C)`,
/// created_date `[0x58,0x5C)`, construction_sub ptr `[0x70,0x78)`. `N =
/// LINE_CONSTRUCTION_SUB + 8` = the far (mandatory) field's end. The sub-object
/// (template/target) is chased separately and stays a per-field read.
pub const LINE_WINDOW_LEN: usize = LINE_CONSTRUCTION_SUB + 8;

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

    /// Bulk-read a contiguous object window `[addr, addr + N)` in a single
    /// guarded read, returning the captured bytes, or `None` when the whole
    /// window is not readable as one region.
    ///
    /// The default returns `None`: generic/test callers fall back to per-field
    /// reads, so the byte-for-byte accept/reject decision is identical to the
    /// pre-collapse path (every existing `FakeMemory` fixture exercises this
    /// fallback unchanged). The in-process collector impl overrides this with a
    /// single region-gated copy, collapsing the several per-field guarded reads
    /// of one heap object into one region check + copy.
    ///
    /// FAIL-CLOSED CONTRACT (byte-identical adoption rule): the window gates
    /// `[addr, addr+N)` through the exact same region check a scalar read of the
    /// field at the far end (`N = last_offset + last_size`) would use, so for a
    /// real heap object (one allocation = one committed region) the window
    /// succeeds iff every field is readable and fails iff the far field is
    /// unreadable — identical to the per-field `?`/`continue` sequence. On
    /// `None` the caller MUST read each field individually (which then
    /// fail-close identically), so a failed bulk read is never worse than the
    /// original per-field walk.
    fn read_object_window<const N: usize>(&self, _addr: usize) -> Option<[u8; N]> {
        None
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
    /// W4: CBuildingProductionLine+0x58 native hour-count of the line's
    /// created_date (absent when the read fails/torn or is an implausible 0/
    /// out-of-range value). Carried in the reserved trailing row u32 (offset
    /// 36), flag-gated by CONSTRUCTION_FLAG_CREATED_DATE_PRESENT.
    pub created_date_raw: Option<u32>,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConstructionSkipStats {
    pub invalid_line_ptr_rows: u32,
    pub non_construction_line_rows: u32,
    pub invalid_active_factories_rows: u32,
    pub invalid_progress_rows: u32,
}

impl ConstructionSkipStats {
    pub fn total(self) -> u32 {
        self.invalid_line_ptr_rows
            .saturating_add(self.non_construction_line_rows)
            .saturating_add(self.invalid_active_factories_rows)
            .saturating_add(self.invalid_progress_rows)
    }

    fn increment(&mut self, reason: ConstructionSkipReason) {
        match reason {
            ConstructionSkipReason::InvalidLinePtr => {
                self.invalid_line_ptr_rows = self.invalid_line_ptr_rows.saturating_add(1);
            }
            ConstructionSkipReason::NonConstructionLine => {
                self.non_construction_line_rows = self.non_construction_line_rows.saturating_add(1);
            }
            ConstructionSkipReason::InvalidActiveFactories => {
                self.invalid_active_factories_rows =
                    self.invalid_active_factories_rows.saturating_add(1);
            }
            ConstructionSkipReason::InvalidProgressFields => {
                self.invalid_progress_rows = self.invalid_progress_rows.saturating_add(1);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConstructionSkipReason {
    InvalidLinePtr,
    NonConstructionLine,
    InvalidActiveFactories,
    InvalidProgressFields,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowRead<T> {
    pub rows: Vec<T>,
    pub skipped_rows: u32,
    pub construction_skip_stats: ConstructionSkipStats,
}

impl<T> RowRead<T> {
    fn empty() -> Self {
        Self {
            rows: Vec::new(),
            skipped_rows: 0,
            construction_skip_stats: ConstructionSkipStats::default(),
        }
    }

    fn push(&mut self, row: T) {
        self.rows.push(row);
    }

    fn skip(&mut self) {
        self.skipped_rows = self.skipped_rows.saturating_add(1);
    }
}

impl RowRead<ConstructionQueueRow> {
    fn skip_construction(&mut self, reason: ConstructionSkipReason) {
        self.skip();
        self.construction_skip_stats.increment(reason);
    }
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
) -> Option<RowRead<ConstructionQueueRow>> {
    let manager = production_manager_from_country(mem, root.country_ptr)?;
    read_construction_queue_from_manager_with_targets(mem, manager, root.tag_db_id, None)
}

pub fn read_construction_queue_from_manager<M: LaneMemory>(
    mem: &M,
    manager: usize,
    tag_db_id: u32,
) -> Option<RowRead<ConstructionQueueRow>> {
    read_construction_queue_from_manager_with_targets(mem, manager, tag_db_id, None)
}

pub fn read_country_construction_queue_with_target_states<M: LaneMemory>(
    mem: &M,
    root: CountryConstructionRoot,
    state_roots: &[StateRoot],
    province_roots: &[ProvinceRoot],
) -> Option<RowRead<ConstructionQueueRow>> {
    let lookup = TargetStateLookup::new(state_roots, province_roots);
    read_country_construction_queue_with_target_lookup(mem, root, &lookup)
}

pub fn read_country_construction_queue_with_target_lookup<M: LaneMemory>(
    mem: &M,
    root: CountryConstructionRoot,
    target_lookup: &TargetStateLookup,
) -> Option<RowRead<ConstructionQueueRow>> {
    let manager = production_manager_from_country(mem, root.country_ptr)?;
    read_construction_queue_from_manager_with_targets(
        mem,
        manager,
        root.tag_db_id,
        Some(target_lookup),
    )
}

fn read_construction_queue_from_manager_with_targets<M: LaneMemory>(
    mem: &M,
    manager: usize,
    tag_db_id: u32,
    target_lookup: Option<&TargetStateLookup>,
) -> Option<RowRead<ConstructionQueueRow>> {
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
        return Some(RowRead::empty());
    }
    if !is_heap_ptr(begin) {
        return None;
    }
    let byte_len = (size as usize).checked_mul(CONSTRUCTION_STRIDE)?;
    let raw = mem.read_bytes(begin, byte_len)?;
    if raw.len() != byte_len {
        return None;
    }

    let mut read = RowRead::empty();
    for offset in (0..byte_len).step_by(CONSTRUCTION_STRIDE) {
        let line_ptr = read_u64_from_slice(&raw, offset)? as usize;
        if !is_heap_ptr(line_ptr) {
            read.skip_construction(ConstructionSkipReason::InvalidLinePtr);
            continue;
        }
        // Collapse the six per-field guarded reads of one CBuildingProductionLine
        // (type/id/active_factories/cost/produced/created_date/sub-ptr) into one
        // bulk window read. The generic default returns None so each `win_*`
        // falls back to its original per-field guarded read; the window is
        // `Some` only when the whole [0,0x78) span is one readable region, so
        // every served field is byte-identical to its individual read. The
        // sub-object (template/target) behind `sub_ptr` stays a per-field chase.
        let window = mem.read_object_window::<LINE_WINDOW_LEN>(line_ptr);
        let win = window.as_ref().map(|w| w.as_slice());
        if win_u32(win, mem, line_ptr, LINE_TYPE)? != LINE_TYPE_CONSTRUCTION {
            read.skip_construction(ConstructionSkipReason::NonConstructionLine);
            continue;
        }
        let active_factories = win_i32(win, mem, line_ptr, LINE_ACTIVE_FACTORIES)?;
        if !(0..=MAX_ACTIVE_FACTORIES).contains(&active_factories) {
            read.skip_construction(ConstructionSkipReason::InvalidActiveFactories);
            continue;
        }
        let cost_raw = win_production_fixed_raw(win, mem, line_ptr, LINE_COST)?;
        let produced_raw = win_production_fixed_raw(win, mem, line_ptr, LINE_PRODUCED)?;
        if cost_raw < 0 || produced_raw < 0 {
            read.skip_construction(ConstructionSkipReason::InvalidProgressFields);
            continue;
        }
        let sub_ptr = win_u64(win, mem, line_ptr, LINE_CONSTRUCTION_SUB)? as usize;
        let template_id = if is_heap_ptr(sub_ptr) {
            mem.read_u32(sub_ptr.checked_add(CONSTRUCTION_TEMPLATE_ID)?)
                .filter(|id| *id > 0 && *id <= MAX_CONSTRUCTION_TEMPLATE_ID)
        } else {
            None
        };
        let target_state_id = target_lookup.and_then(|lookup| {
            read_construction_target_location_id(mem, sub_ptr)
                .and_then(|location_id| lookup.target_state_id(template_id, location_id))
        });
        // W4: created_date at CBuildingProductionLine+0x58. Fail-closed — a read
        // failure or an implausible raw (0, or beyond the sane ceiling) is
        // badged absent (the row is still emitted with its validated fields).
        let created_date_raw = win_u32(win, mem, line_ptr, LINE_CREATED_DATE)
            .filter(|raw| *raw != 0 && *raw <= MAX_CONSTRUCTION_CREATED_DATE_RAW);
        read.push(ConstructionQueueRow {
            tag_db_id,
            line_id: win_u32(win, mem, line_ptr, LINE_ID)?,
            template_id,
            active_factories,
            cost_raw,
            produced_raw,
            priority: (offset / CONSTRUCTION_STRIDE) as u32,
            target_state_id,
            created_date_raw,
        });
    }
    Some(read)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetStateLookup {
    state_ids: BTreeSet<u32>,
    province_to_state: BTreeMap<u32, u32>,
}

impl TargetStateLookup {
    pub fn new(state_roots: &[StateRoot], province_roots: &[ProvinceRoot]) -> Self {
        let state_ids = state_roots.iter().map(|root| root.state_id).collect();
        let province_to_state = province_roots
            .iter()
            .map(|root| (root.province_id, root.state_id))
            .collect();
        Self {
            state_ids,
            province_to_state,
        }
    }

    pub fn target_state_id(&self, template_id: Option<u32>, location_id: u32) -> Option<u32> {
        if is_province_construction_template(template_id) {
            return self.province_to_state.get(&location_id).copied();
        }
        if self.state_ids.contains(&location_id) {
            return Some(location_id);
        }
        if !self.province_to_state.contains_key(&location_id) {
            return None;
        }
        self.province_to_state.get(&location_id).copied()
    }
}

pub fn is_province_construction_template(template_id: Option<u32>) -> bool {
    template_id
        .map(|id| PROVINCE_CONSTRUCTION_TEMPLATE_IDS.contains(&id))
        .unwrap_or(false)
}

pub fn read_construction_target_location_id<M: LaneMemory>(mem: &M, sub_ptr: usize) -> Option<u32> {
    if !is_heap_ptr(sub_ptr) {
        return None;
    }
    let target_ptr = mem.read_u64(sub_ptr.checked_add(CONSTRUCTION_TARGET_LOCATION_PTR)?)? as usize;
    if !is_heap_ptr(target_ptr) {
        return None;
    }
    let location_id = mem.read_u32(target_ptr.checked_add(TARGET_LOCATION_ID)?)?;
    (location_id > 0 && location_id <= MAX_PROVINCE_ID).then_some(location_id)
}

pub fn read_state_building_rows<M: LaneMemory>(
    mem: &M,
    module_base: usize,
    root: StateRoot,
) -> Option<RowRead<BuildingRow>> {
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
) -> Option<RowRead<BuildingRow>> {
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
) -> Option<RowRead<BuildingRow>> {
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
        return Some(RowRead::empty());
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
    let mut skipped_rows = 0u32;
    for index in 0..logical {
        let offset = (index as usize).checked_mul(8)?;
        let building_ptr = read_u64_from_slice(&ptrs, offset)? as usize;
        if !is_heap_ptr(building_ptr) {
            skipped_rows = skipped_rows.saturating_add(1);
            continue;
        }
        // Collapse the four per-field guarded reads of one CBuilding
        // (vtable/type/level/healthy_level) into one bulk window read. The
        // collector impl serves the whole object from a single region-gated
        // copy; the generic default returns None so `win_*` fall back to the
        // original per-field reads. Because the window is only `Some` when the
        // whole `[0,0x44)` span is readable as one region, every field served
        // from it is byte-identical to what its individual guarded read would
        // have returned (and on `None` we take the original path verbatim).
        let window = mem.read_object_window::<CBUILDING_WINDOW_LEN>(building_ptr);
        let win = window.as_ref().map(|w| w.as_slice());
        if win_u64(win, mem, building_ptr, 0)? as usize != cbuilding_vtable {
            skipped_rows = skipped_rows.saturating_add(1);
            continue;
        }
        let type_id = win_u32(win, mem, building_ptr, CBUILDING_OFF_TYPE)?;
        let level = win_u16(win, mem, building_ptr, CBUILDING_OFF_LEVEL)? as u32;
        if type_id == 0 || level > BUILDING_LEVEL_SANE_MAX {
            skipped_rows = skipped_rows.saturating_add(1);
            continue;
        }
        let healthy_level = win_u16(win, mem, building_ptr, CBUILDING_OFF_HEALTHY_LEVEL)
            .map(u32::from)
            .filter(|healthy| *healthy <= level && *healthy <= BUILDING_LEVEL_SANE_MAX);
        by_type
            .entry(type_id)
            .or_default()
            .add(level, healthy_level);
    }
    Some(RowRead {
        rows: by_type
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
        skipped_rows,
        construction_skip_stats: ConstructionSkipStats::default(),
    })
}

pub fn read_state_resource_rows<M: LaneMemory>(
    mem: &M,
    root: StateRoot,
) -> Option<RowRead<StateResourceRow>> {
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
        return Some(RowRead::empty());
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
    Some(RowRead {
        rows: resources
            .into_iter()
            .map(|(resource_id, amount_raw)| StateResourceRow {
                state_id: root.state_id,
                resource_id,
                amount_raw,
            })
            .collect(),
        skipped_rows: 0,
        construction_skip_stats: ConstructionSkipStats::default(),
    })
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
        // W4: reject any row carrying an unknown flag bit (forward-incompatible
        // payload) — fail closed rather than silently drop the unknown field.
        let flags = read_u32_le(bytes, cursor + 32);
        if flags & !CONSTRUCTION_ROW_KNOWN_FLAGS != 0 {
            return Err(format!(
                "construction row {index} has unknown flags {flags:#x}"
            ));
        }
        if deleted_identity_id(read_u32_le(bytes, cursor + 4)).is_some()
            && (read_u32_le(bytes, cursor + 8) != u32::MAX
                || read_u32_le(bytes, cursor + 28) != 0
                || read_u32_le(bytes, cursor + 36) != 0)
        {
            return Err(format!(
                "construction row {index} tombstone has live fields"
            ));
        }
        if flags & CONSTRUCTION_FLAG_CREATED_DATE_PRESENT == 0
            && read_u32_le(bytes, cursor + 36) != 0
        {
            return Err(format!(
                "construction row {index} created_date reserved field is nonzero"
            ));
        }
        let row = read_construction_row(bytes, cursor);
        validate_construction_row(index, &row)?;
        construction_rows.push(row);
        cursor += CONSTRUCTION_ROW_LEN;
    }
    let mut building_rows = Vec::with_capacity(building_count);
    for index in 0..building_count {
        let flags = read_u32_le(bytes, cursor + 24);
        if flags & !BUILDING_ROW_KNOWN_FLAGS != 0 {
            return Err(format!("building row {index} has unknown flags {flags:#x}"));
        }
        if flags & BUILDING_FLAG_DAMAGED != 0 && flags & BUILDING_FLAG_HEALTHY_PRESENT == 0 {
            return Err(format!(
                "building row {index} damaged flag set without healthy level"
            ));
        }
        if read_u32_le(bytes, cursor + 28) != 0 {
            return Err(format!("building row {index} reserved field is nonzero"));
        }
        if deleted_identity_id(read_u32_le(bytes, cursor + 12)).is_some()
            && read_u32_le(bytes, cursor + 20) != u32::MAX
        {
            return Err(format!("building row {index} tombstone has live fields"));
        }
        let row = read_building_row(bytes, cursor);
        validate_building_row(index, &row)?;
        building_rows.push(row);
        cursor += BUILDING_ROW_LEN;
    }
    let mut resource_rows = Vec::with_capacity(resource_count);
    for index in 0..resource_count {
        if read_u32_le(bytes, cursor + 12) != 0 {
            return Err(format!("resource row {index} reserved field is nonzero"));
        }
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
    let blocking_flags = STATUS_WORKER_PANIC | STATUS_SOURCE_UNAVAILABLE;
    if frame.status_flags & blocking_flags != 0 {
        errors.push(format!(
            "blocking status_flags set: {:#x}",
            frame.status_flags
        ));
    }
    if frame.source_flags & SOURCE_FLAG_DLL_NATIVE == 0 {
        errors.push("source_flags missing DLL-native bit".to_string());
    }
    let clean_full_frame = frame.status_flags & (STATUS_PARTIAL_DATA | STATUS_DELTA_DATA) == 0;
    if clean_full_frame && frame.building_rows.is_empty() {
        errors.push("no building rows emitted".to_string());
    }
    if clean_full_frame && frame.resource_rows.is_empty() {
        errors.push("no state resource rows emitted".to_string());
    }
    let live_construction_rows = frame
        .construction_rows
        .iter()
        .filter(|row| construction_row_deleted_id(row).is_none())
        .count();
    let construction_rows_with_target_state = frame
        .construction_rows
        .iter()
        .filter(|row| construction_row_deleted_id(row).is_none())
        .filter(|row| row.target_state_id.is_some())
        .count();
    let target_state_unavailable =
        frame.source_flags & SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE != 0;
    if live_construction_rows != 0 {
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
            && construction_rows_with_target_state != live_construction_rows
        {
            errors.push(
                "construction target_state_id partially emitted; fill every construction row or badge unavailable"
                    .to_string(),
            );
        }
    }
    let mut seen_construction = BTreeSet::new();
    for (index, row) in frame.construction_rows.iter().enumerate() {
        if !seen_construction.insert((row.tag_db_id, row.line_id & !ROW_ID_DELETED)) {
            errors.push(format!("construction row {index} duplicates tag/line"));
        }
    }
    let mut seen_buildings = BTreeSet::new();
    let mut damaged_building_rows = 0usize;
    for (index, row) in frame.building_rows.iter().enumerate() {
        if !seen_buildings.insert((row.scope, row.location_id, row.type_id & !ROW_ID_DELETED)) {
            errors.push(format!(
                "building row {index} duplicates scope/location/type"
            ));
        }
        if building_row_deleted_id(row).is_none()
            && row.healthy_level.is_some_and(|healthy| healthy < row.level)
        {
            damaged_building_rows = damaged_building_rows.saturating_add(1);
        }
    }
    let mut seen_resources = BTreeSet::new();
    for (index, row) in frame.resource_rows.iter().enumerate() {
        if !seen_resources.insert((row.state_id, row.resource_id & !ROW_ID_DELETED)) {
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
    if row.created_date_raw.is_some() {
        flags |= CONSTRUCTION_FLAG_CREATED_DATE_PRESENT;
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
    // W4: reserved trailing u32 now carries created_date_raw when present (0
    // when absent — identical to the prior always-zero behaviour).
    out.extend_from_slice(&row.created_date_raw.unwrap_or(0).to_le_bytes());
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
    let created_date_raw = read_u32_le(bytes, offset + 36);
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
        created_date_raw: (flags & CONSTRUCTION_FLAG_CREATED_DATE_PRESENT != 0)
            .then_some(created_date_raw),
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
    if row.tag_db_id > compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID {
        return Err(format!(
            "construction row {index} invalid tag_db_id {}",
            row.tag_db_id
        ));
    }
    if let Some(line_id) = construction_row_deleted_id(row) {
        if line_id == 0 {
            return Err(format!("construction row {index} has zero deleted line_id"));
        }
        if row.template_id.is_some()
            || row.active_factories != 0
            || row.cost_raw != 0
            || row.produced_raw != 0
            || row.priority != 0
            || row.target_state_id.is_some()
            || row.created_date_raw.is_some()
        {
            return Err(format!(
                "construction row {index} tombstone has live fields"
            ));
        }
        return Ok(());
    }
    if row.line_id == 0 {
        return Err(format!("construction row {index} has zero line_id"));
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
    // W4: a present created_date must be a plausible non-zero raw hour-count
    // (fail closed on a torn/uninitialised value).
    if let Some(created_date_raw) = row.created_date_raw {
        if created_date_raw == 0 || created_date_raw > MAX_CONSTRUCTION_CREATED_DATE_RAW {
            return Err(format!(
                "construction row {index} invalid created_date_raw {created_date_raw}"
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
    let type_id = row.type_id & !ROW_ID_DELETED;
    if type_id == 0 {
        return Err(format!("building row {index} has zero type_id"));
    }
    if let Some(type_id) = building_row_deleted_id(row) {
        if type_id == 0 {
            return Err(format!("building row {index} has zero deleted type_id"));
        }
        if row.level != 0 || row.healthy_level.is_some() {
            return Err(format!("building row {index} tombstone has live fields"));
        }
        return Ok(());
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
    let resource_id = row.resource_id & !ROW_ID_DELETED;
    if resource_id == 0 || resource_id == STATE_RESOURCE_NO_ID {
        return Err(format!(
            "resource row {index} invalid resource_id {}",
            resource_id
        ));
    }
    if let Some(resource_id) = resource_row_deleted_id(row) {
        if resource_id == 0 || resource_id == STATE_RESOURCE_NO_ID {
            return Err(format!(
                "resource row {index} invalid deleted resource_id {resource_id}"
            ));
        }
        if row.amount_raw != 0 {
            return Err(format!("resource row {index} tombstone has live fields"));
        }
        return Ok(());
    }
    if !(0..=STATE_RESOURCE_MAX_ABS_RAW).contains(&row.amount_raw) {
        return Err(format!(
            "resource row {index} invalid amount_raw {}",
            row.amount_raw
        ));
    }
    Ok(())
}

pub fn deleted_identity_id(value: u32) -> Option<u32> {
    (value & ROW_ID_DELETED != 0).then_some(value & !ROW_ID_DELETED)
}

pub fn construction_row_deleted_id(row: &ConstructionQueueRow) -> Option<u32> {
    deleted_identity_id(row.line_id)
}

pub fn building_row_deleted_id(row: &BuildingRow) -> Option<u32> {
    deleted_identity_id(row.type_id)
}

pub fn resource_row_deleted_id(row: &StateResourceRow) -> Option<u32> {
    deleted_identity_id(row.resource_id)
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

fn read_u16_from_slice(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

// Window-or-live field readers. When `win` holds an object's captured bytes
// (one guarded bulk read), the field is sliced from the buffer with no further
// guarded read; otherwise the exact original per-field guarded read at
// `addr + off` is performed. Both paths yield the identical value for a
// readable field and identical `None` (fail-closed) for an unreadable one, so
// swapping a per-field walk for `Some(window)` is byte-identical on any real
// (single-region) heap object; on `None` the caller degrades to the original
// per-field sequence.
#[inline]
fn win_u64<M: LaneMemory>(win: Option<&[u8]>, mem: &M, addr: usize, off: usize) -> Option<u64> {
    match win {
        Some(buf) => read_u64_from_slice(buf, off),
        None => mem.read_u64(addr.checked_add(off)?),
    }
}

#[inline]
fn win_u32<M: LaneMemory>(win: Option<&[u8]>, mem: &M, addr: usize, off: usize) -> Option<u32> {
    match win {
        Some(buf) => read_u32_from_slice(buf, off),
        None => mem.read_u32(addr.checked_add(off)?),
    }
}

#[inline]
fn win_u16<M: LaneMemory>(win: Option<&[u8]>, mem: &M, addr: usize, off: usize) -> Option<u16> {
    match win {
        Some(buf) => read_u16_from_slice(buf, off),
        None => mem.read_u16(addr.checked_add(off)?),
    }
}

#[inline]
fn win_i32<M: LaneMemory>(win: Option<&[u8]>, mem: &M, addr: usize, off: usize) -> Option<i32> {
    match win {
        Some(buf) => read_i32_from_slice(buf, off),
        None => mem.read_i32(addr.checked_add(off)?),
    }
}

/// Reads a production-line fixed-point raw either from the captured window or
/// via an individual guarded read, then applies the in-range filter (the
/// former `read_production_fixed_raw` guard). Byte-identical to the original.
#[inline]
fn win_production_fixed_raw<M: LaneMemory>(
    win: Option<&[u8]>,
    mem: &M,
    addr: usize,
    off: usize,
) -> Option<i32> {
    win_i32(win, mem, addr, off).filter(|value| {
        (-MAX_PRODUCTION_LINE_FIXED_RAW..=MAX_PRODUCTION_LINE_FIXED_RAW).contains(value)
    })
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

    /// Test double modelling the in-process collector's bulk-window read. It
    /// serves `read_object_window` from one contiguous copy — `Some` iff every
    /// byte of `[addr, addr + N)` is mapped, exactly like the collector's
    /// `read_into` succeeding only when the whole span is one readable region.
    /// Every other read delegates to the inner sparse store, so the per-field
    /// fallback path (over `FakeMemory`) and the bulk-window path (over this
    /// double) observe identical underlying bytes — any offset/slice mistake in
    /// the window helpers shows up as a row mismatch between the two.
    struct ContiguousFakeMemory {
        inner: FakeMemory,
    }

    impl LaneMemory for ContiguousFakeMemory {
        fn read_u8(&self, addr: usize) -> Option<u8> {
            self.inner.read_u8(addr)
        }

        fn read_object_window<const N: usize>(&self, addr: usize) -> Option<[u8; N]> {
            let mut buf = [0u8; N];
            for (index, slot) in buf.iter_mut().enumerate() {
                *slot = self.inner.read_u8(addr.checked_add(index)?)?;
            }
            Some(buf)
        }
    }

    /// Fully populate a CBuilding object (zero-filled window + fields) so the
    /// contiguous window read succeeds.
    fn write_full_building(
        mem: &mut FakeMemory,
        base: usize,
        ptr: usize,
        ty: u32,
        level: u16,
        healthy: u16,
    ) {
        mem.write_bytes(ptr, &[0u8; CBUILDING_WINDOW_LEN]);
        mem.write_u64(ptr, (base + CBUILDING_VTABLE_RVA) as u64);
        mem.write_u32(ptr + CBUILDING_OFF_TYPE, ty);
        mem.write_u16(ptr + CBUILDING_OFF_LEVEL, level);
        mem.write_u16(ptr + CBUILDING_OFF_HEALTHY_LEVEL, healthy);
    }

    #[test]
    fn domain_id_matches_shared_compact_descriptor() {
        let descriptor = compact_contract::assigned_descriptor_by_id(DOMAIN_ID)
            .expect("domain 48 descriptor is registered");
        assert_eq!(DOMAIN_ID, 48);
        assert_eq!(descriptor.canonical_name, DOMAIN_NAME);
        assert!(descriptor.schedulable);
        assert!(descriptor.requires_data_record);
    }

    #[test]
    fn extractor_counts_skipped_construction_rows() {
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
        mem.write_i32(good_line + LINE_COST, 1_450_000_000);
        mem.write_i32(good_line + LINE_PRODUCED, 5_000_000);
        mem.write_u64(good_line + LINE_CONSTRUCTION_SUB, sub as u64);
        mem.write_u32(sub + CONSTRUCTION_TEMPLATE_ID, 20_013);
        mem.write_u32(bad_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_i32(bad_line + LINE_ACTIVE_FACTORIES, -1);

        let read = read_construction_queue_from_manager(&mem, manager, 123).unwrap();

        assert_eq!(read.skipped_rows, 1);
        assert_eq!(
            read.construction_skip_stats.invalid_active_factories_rows,
            1
        );
        assert_eq!(read.construction_skip_stats.total(), 1);
        assert_eq!(read.rows.len(), 1);
        assert_eq!(read.rows[0].tag_db_id, 123);
        assert_eq!(read.rows[0].line_id, 77);
        assert_eq!(read.rows[0].template_id, Some(20_013));
        assert_eq!(read.rows[0].active_factories, 15);
        assert_eq!(read.rows[0].cost_raw, 1_450_000_000);
        assert_eq!(read.rows[0].priority, 0);
    }

    #[test]
    fn collector_skipped_construction_row_keeps_valid_sibling_rows_partial() {
        let country = 0x1000_0000_0000usize;
        let manager = 0x1000_0000_1000usize;
        let vector = 0x1000_0000_2000usize;
        let good_line = 0x1000_0000_2800usize;
        let bad_line = 0x1000_0000_3000usize;
        let sub = 0x1000_0000_4000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(country + OBJ_PROD_MANAGER, manager as u64);
        mem.write_u64(manager + CONSTRUCTION_BEGIN, vector as u64);
        mem.write_u32(manager + CONSTRUCTION_CAP, 2);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 2);
        mem.write_u64(vector, good_line as u64);
        mem.write_u64(vector + 8, bad_line as u64);
        mem.write_u32(good_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(good_line + LINE_ID, 77);
        mem.write_i32(good_line + LINE_ACTIVE_FACTORIES, 15);
        mem.write_i32(good_line + LINE_COST, 1_450_000_000);
        mem.write_i32(good_line + LINE_PRODUCED, 5_000_000);
        mem.write_u64(good_line + LINE_CONSTRUCTION_SUB, sub as u64);
        mem.write_u32(sub + CONSTRUCTION_TEMPLATE_ID, 20_013);
        mem.write_u32(bad_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_i32(bad_line + LINE_ACTIVE_FACTORIES, -1);

        let output = collector_module::collect_domain48_frame(
            &mem,
            collector_module::CollectorRoots {
                module_base: 0x1400_0000,
                game_hour_raw: 321,
                country_roots: vec![CountryConstructionRoot {
                    tag_db_id: 123,
                    country_ptr: country,
                }],
                state_roots: Vec::new(),
                province_roots: Vec::new(),
            },
        );

        assert_eq!(output.stats.skipped_construction_rows, 1);
        assert_eq!(
            output
                .stats
                .skipped_construction_invalid_active_factories_rows,
            1
        );
        assert_ne!(output.frame.status_flags & STATUS_PARTIAL_DATA, 0);
        assert_eq!(output.frame.construction_rows.len(), 1);
        assert_eq!(output.frame.construction_rows[0].line_id, 77);
        assert!(output.frame.building_rows.is_empty());
        assert!(output.frame.resource_rows.is_empty());
    }

    #[test]
    fn extractor_reads_construction_target_state_from_location_root() {
        let manager = 0x1000_0000_0000usize;
        let vector = 0x1000_0000_1000usize;
        let state_line = 0x1000_0000_2000usize;
        let province_line = 0x1000_0000_2100usize;
        let state_sub = 0x1000_0000_3000usize;
        let province_sub = 0x1000_0000_3100usize;
        let state_target = 0x1000_0000_4000usize;
        let province_target = 0x1000_0000_4100usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(manager + CONSTRUCTION_BEGIN, vector as u64);
        mem.write_u32(manager + CONSTRUCTION_CAP, 2);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 2);
        mem.write_u64(vector, state_line as u64);
        mem.write_u64(vector + 8, province_line as u64);

        mem.write_u32(state_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(state_line + LINE_ID, 77);
        mem.write_i32(state_line + LINE_ACTIVE_FACTORIES, 15);
        mem.write_i32(state_line + LINE_COST, 1_080_000_000);
        mem.write_i32(state_line + LINE_PRODUCED, 5_000_000);
        mem.write_u64(state_line + LINE_CONSTRUCTION_SUB, state_sub as u64);
        mem.write_u32(state_sub + CONSTRUCTION_TEMPLATE_ID, 20_013);
        mem.write_u64(
            state_sub + CONSTRUCTION_TARGET_LOCATION_PTR,
            state_target as u64,
        );
        mem.write_u32(state_target + TARGET_LOCATION_ID, 42);

        mem.write_u32(province_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(province_line + LINE_ID, 78);
        mem.write_i32(province_line + LINE_ACTIVE_FACTORIES, 10);
        mem.write_i32(province_line + LINE_COST, 600_000_000);
        mem.write_i32(province_line + LINE_PRODUCED, 4_000_000);
        mem.write_u64(province_line + LINE_CONSTRUCTION_SUB, province_sub as u64);
        mem.write_u32(province_sub + CONSTRUCTION_TEMPLATE_ID, 12_790);
        mem.write_u64(
            province_sub + CONSTRUCTION_TARGET_LOCATION_PTR,
            province_target as u64,
        );
        mem.write_u32(province_target + TARGET_LOCATION_ID, 2_068);

        let lookup = TargetStateLookup::new(
            &[StateRoot {
                state_id: 42,
                state_ptr: 0x1000_0001_0000,
            }],
            &[ProvinceRoot {
                province_id: 2_068,
                province_ptr: 0x1000_0002_0000,
                state_id: 42,
            }],
        );
        let read =
            read_construction_queue_from_manager_with_targets(&mem, manager, 123, Some(&lookup))
                .unwrap();

        assert_eq!(read.skipped_rows, 0);
        assert_eq!(read.rows.len(), 2);
        assert_eq!(read.rows[0].target_state_id, Some(42));
        assert_eq!(read.rows[1].target_state_id, Some(42));
    }

    #[test]
    fn extractor_reads_building_rows_with_damage_signal() {
        let base = 0x1400_0000usize;
        let state = 0x1000_0001_0000usize;
        let vector = 0x1000_0002_0000usize;
        let building = 0x1000_0003_0000usize;
        let bad_building = 0x2000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(state + STATE_OFF_BUILDINGS_VEC, vector as u64);
        mem.write_u64(state + STATE_OFF_BUILDINGS_VEC + 8, 2u64 << 32);
        mem.write_u64(vector, building as u64);
        mem.write_u64(vector + 8, bad_building as u64);
        mem.write_u64(building, (base + CBUILDING_VTABLE_RVA) as u64);
        mem.write_u32(building + CBUILDING_OFF_TYPE, 0x2FAB);
        mem.write_u16(building + CBUILDING_OFF_LEVEL, 5);
        mem.write_u16(building + CBUILDING_OFF_HEALTHY_LEVEL, 4);

        let read = read_state_building_rows(
            &mem,
            base,
            StateRoot {
                state_id: 42,
                state_ptr: state,
            },
        )
        .unwrap();

        assert_eq!(read.skipped_rows, 1);
        assert_eq!(
            read.rows,
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

        let read = read_state_resource_rows(
            &mem,
            StateRoot {
                state_id: 64,
                state_ptr: state,
            },
        )
        .unwrap();

        assert_eq!(read.skipped_rows, 0);
        assert_eq!(
            read.rows,
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
    fn compact_frame_round_trips_created_date_melt_anchored() {
        // W4 melt anchor (00_d04_1944_melted.txt line 966792): GER construction
        // line id 18354 created_date=1943.9.23.4 -> raw 60_827_043 (D5 live
        // +0x58). The reserved trailing row slot carries it; row length is
        // unchanged, so the decoder round-trips it exactly.
        let mut frame = sample_frame();
        frame.construction_rows[0].created_date_raw = Some(60_827_043);
        let encoded = encode_lane_frame(&frame);
        // Repurposing the reserved slot must NOT change the wire row size.
        assert_eq!(CONSTRUCTION_ROW_LEN, 40);
        let decoded = decode_lane_frame(&encoded).unwrap();
        assert_eq!(
            decoded.construction_rows[0].created_date_raw,
            Some(60_827_043)
        );
        assert!(gate_lane_frame(&encoded).ok);
    }

    #[test]
    fn compact_frame_round_trips_absent_created_date() {
        // Absent created_date: flag clear, reserved slot stays zero (byte-for-
        // byte the prior always-zero behaviour), decodes back to None.
        let mut frame = sample_frame();
        frame.construction_rows[0].created_date_raw = None;
        let encoded = encode_lane_frame(&frame);
        let decoded = decode_lane_frame(&encoded).unwrap();
        assert_eq!(decoded.construction_rows[0].created_date_raw, None);
        // The trailing row u32 (row offset 36) is zero when absent.
        let row_start = HEADER_LEN;
        assert_eq!(read_u32_le(&encoded, row_start + 36), 0);
        assert!(gate_lane_frame(&encoded).ok);
    }

    #[test]
    fn compact_gate_rejects_zero_created_date() {
        // A present-but-zero created_date is a torn read and fails the gate.
        let mut frame = sample_frame();
        frame.construction_rows[0].created_date_raw = Some(0);
        let encoded = encode_lane_frame(&frame);
        let gate = gate_lane_frame(&encoded);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("created_date_raw")));
    }

    #[test]
    fn compact_decode_rejects_unknown_row_flag_bit() {
        // Forward-incompatibility guard: an unknown flag bit in a construction
        // row is rejected (fail closed) rather than silently dropped.
        let frame = sample_frame();
        let mut encoded = encode_lane_frame(&frame);
        // Set an undefined high flag bit on the first construction row.
        let flags_off = HEADER_LEN + 32;
        let mut flags = read_u32_le(&encoded, flags_off);
        flags |= 1 << 15;
        encoded[flags_off..flags_off + 4].copy_from_slice(&flags.to_le_bytes());
        let err = decode_lane_frame(&encoded).unwrap_err();
        assert!(err.contains("unknown flags"), "{err}");
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
            created_date_raw: None,
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

    #[test]
    fn bulk_window_building_rows_match_per_field_path() {
        // Two buildings under one state; one damaged, one bad pointer. The bulk
        // window path (ContiguousFakeMemory) must yield rows byte-identical to
        // the per-field fallback path (FakeMemory), proving the window offsets
        // and slice readers reproduce the individual reads exactly.
        let base = 0x1400_0000usize;
        let state = 0x1000_0001_0000usize;
        let vector = 0x1000_0002_0000usize;
        let building_a = 0x1000_0003_0000usize;
        let building_b = 0x1000_0003_1000usize;
        let bad_building = 0x2000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(state + STATE_OFF_BUILDINGS_VEC, vector as u64);
        mem.write_u64(state + STATE_OFF_BUILDINGS_VEC + 8, 3u64 << 32);
        mem.write_u64(vector, building_a as u64);
        mem.write_u64(vector + 8, building_b as u64);
        mem.write_u64(vector + 16, bad_building as u64);
        write_full_building(&mut mem, base, building_a, 0x2FAB, 5, 4);
        write_full_building(&mut mem, base, building_b, 0x4CC1, 2, 2);

        let root = StateRoot {
            state_id: 42,
            state_ptr: state,
        };
        let per_field = read_state_building_rows(&mem, base, root).unwrap();
        let bulk =
            read_state_building_rows(&ContiguousFakeMemory { inner: mem }, base, root).unwrap();

        assert_eq!(per_field.rows, bulk.rows);
        assert_eq!(per_field.skipped_rows, bulk.skipped_rows);
        // Sanity: the fixture actually produced the expected rows (damage kept).
        assert_eq!(bulk.skipped_rows, 1);
        assert_eq!(bulk.rows.len(), 2);
        assert_eq!(bulk.rows[0].healthy_level, Some(4));
    }

    #[test]
    fn bulk_window_construction_rows_match_per_field_path() {
        // One country queue with a construction line whose window is fully
        // populated. The bulk window path must reproduce the per-field row
        // (incl. created_date, template chase, and target-state lookup) exactly.
        let manager = 0x1000_0000_0000usize;
        let vector = 0x1000_0000_1000usize;
        let line = 0x1000_0000_2000usize;
        let sub = 0x1000_0000_3000usize;
        let target = 0x1000_0000_4000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(manager + CONSTRUCTION_BEGIN, vector as u64);
        mem.write_u32(manager + CONSTRUCTION_CAP, 1);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 1);
        mem.write_u64(vector, line as u64);
        // Fully populate the line window so the contiguous reader can snapshot it.
        mem.write_bytes(line, &[0u8; LINE_WINDOW_LEN]);
        mem.write_u32(line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(line + LINE_ID, 77);
        mem.write_i32(line + LINE_ACTIVE_FACTORIES, 15);
        mem.write_i32(line + LINE_COST, 1_080_000_000);
        mem.write_i32(line + LINE_PRODUCED, 5_000_000);
        mem.write_u32(line + LINE_CREATED_DATE, 60_827_043);
        mem.write_u64(line + LINE_CONSTRUCTION_SUB, sub as u64);
        mem.write_u32(sub + CONSTRUCTION_TEMPLATE_ID, 20_013);
        mem.write_u64(sub + CONSTRUCTION_TARGET_LOCATION_PTR, target as u64);
        mem.write_u32(target + TARGET_LOCATION_ID, 42);

        let lookup = TargetStateLookup::new(
            &[StateRoot {
                state_id: 42,
                state_ptr: 0x1000_0001_0000,
            }],
            &[],
        );
        let per_field =
            read_construction_queue_from_manager_with_targets(&mem, manager, 123, Some(&lookup))
                .unwrap();
        let bulk = read_construction_queue_from_manager_with_targets(
            &ContiguousFakeMemory { inner: mem },
            manager,
            123,
            Some(&lookup),
        )
        .unwrap();

        assert_eq!(per_field.rows, bulk.rows);
        assert_eq!(per_field.skipped_rows, bulk.skipped_rows);
        // Sanity: the populated fixture produced the fully-decoded row.
        assert_eq!(bulk.rows.len(), 1);
        assert_eq!(bulk.rows[0].line_id, 77);
        assert_eq!(bulk.rows[0].template_id, Some(20_013));
        assert_eq!(bulk.rows[0].created_date_raw, Some(60_827_043));
        assert_eq!(bulk.rows[0].target_state_id, Some(42));
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
                // W4: melt-anchored created_date (GER line 18354 == raw
                // 60827043 -> renders 1943.9.23.4). Exercises the reserved-slot
                // round trip through encode/decode.
                created_date_raw: Some(60_827_043),
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
