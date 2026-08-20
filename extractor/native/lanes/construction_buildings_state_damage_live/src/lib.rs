#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

pub mod collector_module;
pub mod decoder_gate;

/// The D48V coverage trailer's byte grammar, shared with `wire_format`'s
/// independent d48 port so the two cannot drift (owner ruling 2026-08-15, ONE
/// shared decoder). This lane keeps its semantics, its gate and its
/// diagnostics; only the byte boundaries and refusals are shared.
///
/// MANDATORY on every frame since `COMPACT_VERSION` 3 — a tail without a D48V
/// is a decode error, because a frame that cannot say what it scanned cannot be
/// trusted to say what exists.
pub use d48_grammar::{
    decode_coverage_trailer, encode_coverage_trailer, railway_trailer_extent, split_frame_tail,
    CoverageCensus, CENSUS_FLAG_PROVINCE_FAILURES_AUTHORITATIVE,
    CENSUS_FLAG_STATE_ROOTS_AUTHORITATIVE, CENSUS_KNOWN_FLAGS, COVERAGE_TRAILER_HEADER_LEN,
    COVERAGE_TRAILER_MAGIC, COVERAGE_TRAILER_VERSION,
};

pub const DOMAIN_ID: u16 = 48;
pub const DOMAIN_NAME: &str = "construction_buildings_state_damage_live";
pub const COMPACT_MAGIC: [u8; 4] = *b"CBSD";
/// The payload version, from the shared grammar so this producer and
/// `wire_format`'s independent port cannot declare different numbers.
pub const COMPACT_VERSION: u16 = d48_grammar::COMPACT_VERSION;
pub const HEADER_LEN: usize = 64;
pub const CONSTRUCTION_ROW_LEN: usize = 52;
pub const BUILDING_ROW_LEN: usize = 40;
pub const RESOURCE_ROW_LEN: usize = 16;
pub const RAILWAY_TRAILER_MAGIC: [u8; 4] = *b"RLC1";
pub const RAILWAY_TRAILER_VERSION: u16 = 1;
pub const RAILWAY_TRAILER_HEADER_LEN: usize = 24;
pub const RAILWAY_TRAILER_ROW_LEN: usize = 64;

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
pub const CBUILDING_OFF_PARTIAL_HEALTH_RAW: usize =
    compact_contract::build_profile::CBUILDING_OFF_PARTIAL_HEALTH_RAW;
pub const CBUILDING_PARTIAL_HEALTH_RAW_PER_LEVEL: u64 =
    compact_contract::build_profile::CBUILDING_PARTIAL_HEALTH_RAW_PER_LEVEL;
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
pub const LINE_CREATED_DATE: usize = compact_contract::build_profile::LINE_CREATED_DATE;
pub const LINE_REPAIR_LEVELS_REMAINING: usize =
    compact_contract::build_profile::LINE_REPAIR_LEVELS_REMAINING;
pub const MAX_CONSTRUCTION_CREATED_DATE_RAW: u32 = 100_000_000;
pub const LINE_TYPE_CONSTRUCTION: u32 = compact_contract::build_profile::LINE_TYPE_CONSTRUCTION;
pub const LINE_TYPE_RAILWAY_CONSTRUCTION: u32 = 4_713;
pub const RAILWAY_LINE_VTABLE_RVA: usize = 0x2A15130;
pub const RAILWAY_LINE_CREATED_DATE: usize = 0x58;
pub const RAILWAY_LINE_BASE_LEVEL: usize = 0x70;
pub const RAILWAY_LINE_TARGET_LEVEL: usize = 0x74;
pub const RAILWAY_LINE_COMPLETION_FLAG: usize = 0x78;
pub const RAILWAY_LINE_ROUTE_PTR: usize = 0x80;
pub const RAILWAY_LINE_ROUTE_CAP: usize = 0x88;
pub const RAILWAY_LINE_ROUTE_SIZE: usize = 0x8C;
pub const RAILWAY_LINE_CURRENT_SEGMENT: usize = 0x98;
pub const RAILWAY_LINE_REMAINING_HOURS: usize = 0xA0;
pub const RAILWAY_LINE_WINDOW_LEN: usize = 0xA8;
pub const MAX_RAILWAY_ROUTE_PROVINCES: u32 = 4_096;
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
pub const CONSTRUCTION_FLAG_CREATED_DATE_PRESENT: u32 = 1 << 2;
pub const CONSTRUCTION_FLAG_TARGET_LOCATION_PRESENT: u32 = 1 << 3;
pub const CONSTRUCTION_FLAG_TARGET_SCOPE_PRESENT: u32 = 1 << 4;
pub const CONSTRUCTION_FLAG_REPAIR_LEVELS_PRESENT: u32 = 1 << 5;
pub const CONSTRUCTION_ROW_KNOWN_FLAGS: u32 = CONSTRUCTION_FLAG_TEMPLATE_PRESENT
    | CONSTRUCTION_FLAG_TARGET_STATE_PRESENT
    | CONSTRUCTION_FLAG_CREATED_DATE_PRESENT
    | CONSTRUCTION_FLAG_TARGET_LOCATION_PRESENT
    | CONSTRUCTION_FLAG_TARGET_SCOPE_PRESENT
    | CONSTRUCTION_FLAG_REPAIR_LEVELS_PRESENT;
pub const BUILDING_FLAG_HEALTHY_PRESENT: u32 = 1;
pub const BUILDING_FLAG_DAMAGED: u32 = 1 << 1;
pub const BUILDING_FLAG_PARTIAL_HEALTH_PRESENT: u32 = 1 << 2;
pub const BUILDING_ROW_KNOWN_FLAGS: u32 =
    BUILDING_FLAG_HEALTHY_PRESENT | BUILDING_FLAG_DAMAGED | BUILDING_FLAG_PARTIAL_HEALTH_PRESENT;
pub const ROW_ID_DELETED: u32 = 1 << 31;

pub const PROVINCE_CONSTRUCTION_TEMPLATE_IDS: [u32; 4] = [12_790, 19_649, 22_353, 22_354];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum BuildingDamageReason {
    None = 0,
    Bombed = 1,
    Sabotaged = 2,
    Collateral = 3,
    Repaired = 4,
    Scorched = 5,
}

impl BuildingDamageReason {
    pub fn from_raw(raw: u32) -> Option<Self> {
        Some(match raw {
            0 => Self::None,
            1 => Self::Bombed,
            2 => Self::Sabotaged,
            3 => Self::Collateral,
            4 => Self::Repaired,
            5 => Self::Scorched,
            _ => return None,
        })
    }

    pub const fn token(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bombed => "bombed",
            Self::Sabotaged => "sabotaged",
            Self::Collateral => "collateral",
            Self::Repaired => "repaired",
            Self::Scorched => "scorched",
        }
    }
}

pub const CBUILDING_WINDOW_LEN: usize = CBUILDING_OFF_PARTIAL_HEALTH_RAW + 8;

pub const LINE_WINDOW_LEN: usize = LINE_REPAIR_LEVELS_REMAINING + 4;

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

    fn read_i64(&self, addr: usize) -> Option<i64> {
        let bytes = self.read_bytes(addr, 8)?;
        Some(i64::from_le_bytes(bytes.try_into().ok()?))
    }

    fn read_u64(&self, addr: usize) -> Option<u64> {
        let bytes = self.read_bytes(addr, 8)?;
        Some(u64::from_le_bytes(bytes.try_into().ok()?))
    }

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
    pub target_location_id: Option<u32>,
    pub target_scope: Option<u32>,
    pub repair_levels_remaining: Option<u32>,
    pub created_date_raw: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailwayConstructionRow {
    pub tag_db_id: u32,
    pub line_id: u32,
    pub queue_index: u32,
    pub active_factories: i32,
    pub cost_raw: i32,
    pub produced_raw: i32,
    pub created_date_raw: u32,
    pub base_level: u32,
    pub target_level: u32,
    pub completion_flag: bool,
    pub current_segment_index: i32,
    pub remaining_hours_raw: i64,
    pub route_province_ids: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildingRow {
    pub scope: u32,
    pub location_id: u32,
    pub state_id: u32,
    pub type_id: u32,
    pub level: u32,
    pub healthy_level: Option<u32>,
    pub partial_health_raw: Option<u64>,
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
    pub railway_construction_rows: Option<Vec<RailwayConstructionRow>>,
    pub building_rows: Vec<BuildingRow>,
    pub resource_rows: Vec<StateResourceRow>,
    /// The walk's coverage census. Describes THIS frame's scan even on delta
    /// frames, whose rows are compressed but whose walk was complete.
    pub coverage: CoverageCensus,
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
    pub first_invalid_progress: Option<InvalidConstructionProgress>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidConstructionProgress {
    pub line_address: usize,
    pub tag_db_id: u32,
    pub queue_index: u32,
    pub line_type: u32,
    pub line_id: u32,
    pub cost_raw: i32,
    pub produced_raw: i32,
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

    fn skip_invalid_progress(&mut self, failure: InvalidConstructionProgress) {
        self.skip_construction(ConstructionSkipReason::InvalidProgressFields);
        if self
            .construction_skip_stats
            .first_invalid_progress
            .is_none()
        {
            self.construction_skip_stats.first_invalid_progress = Some(failure);
        }
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
        let window = mem.read_object_window::<LINE_WINDOW_LEN>(line_ptr);
        let win = window.as_ref().map(|w| w.as_slice());
        let line_type = win_u32(win, mem, line_ptr, LINE_TYPE)?;
        if line_type == LINE_TYPE_RAILWAY_CONSTRUCTION {
            continue;
        }
        if line_type != LINE_TYPE_CONSTRUCTION {
            read.skip_construction(ConstructionSkipReason::NonConstructionLine);
            continue;
        }
        let active_factories = win_i32(win, mem, line_ptr, LINE_ACTIVE_FACTORIES)?;
        if !(0..=MAX_ACTIVE_FACTORIES).contains(&active_factories) {
            read.skip_construction(ConstructionSkipReason::InvalidActiveFactories);
            continue;
        }
        let line_id = win_u32(win, mem, line_ptr, LINE_ID)?;
        let cost_raw = win_production_fixed_raw(win, mem, line_ptr, LINE_COST)?;
        let produced_raw = win_production_fixed_raw(win, mem, line_ptr, LINE_PRODUCED)?;
        if cost_raw < 0 || produced_raw < 0 {
            read.skip_invalid_progress(InvalidConstructionProgress {
                line_address: line_ptr,
                tag_db_id,
                queue_index: (offset / CONSTRUCTION_STRIDE) as u32,
                line_type,
                line_id,
                cost_raw,
                produced_raw,
            });
            continue;
        }
        let sub_ptr = win_u64(win, mem, line_ptr, LINE_CONSTRUCTION_SUB)? as usize;
        let template_id = if is_heap_ptr(sub_ptr) {
            mem.read_u32(sub_ptr.checked_add(CONSTRUCTION_TEMPLATE_ID)?)
                .filter(|id| *id > 0 && *id <= MAX_CONSTRUCTION_TEMPLATE_ID)
        } else {
            None
        };
        let target_location_id = read_construction_target_location_id(mem, sub_ptr);
        let target = target_lookup.and_then(|lookup| {
            target_location_id.and_then(|location_id| lookup.target(template_id, location_id))
        });
        let target_location_id =
            match (target_location_id, construction_template_scope(template_id)) {
                (Some(location_id), Some(scope)) => {
                    let (space_max, space_unit) = if scope == SCOPE_STATE {
                        (
                            MAX_STATE_ID,
                            compact_contract::bound_trips::BOUND_TRIP_MAP_STATE_ID,
                        )
                    } else {
                        (
                            MAX_PROVINCE_ID,
                            compact_contract::bound_trips::BOUND_TRIP_MAP_PROVINCE_ID,
                        )
                    };
                    if compact_contract::bound_trips::bound_exceeded_u32(
                        location_id,
                        space_max,
                        space_unit,
                    ) {
                        None
                    } else {
                        Some(location_id)
                    }
                }
                (Some(location_id), None) => {
                    (location_id <= MAX_PROVINCE_ID.max(MAX_STATE_ID)).then_some(location_id)
                }
                (None, _) => None,
            };
        let repair_levels_remaining = win_i32(win, mem, line_ptr, LINE_REPAIR_LEVELS_REMAINING)
            .filter(|raw| *raw >= 0 && (*raw as u32) <= BUILDING_LEVEL_SANE_MAX)
            .map(|raw| raw as u32);
        let created_date_raw = win_u32(win, mem, line_ptr, LINE_CREATED_DATE)
            .filter(|raw| *raw != 0 && *raw <= MAX_CONSTRUCTION_CREATED_DATE_RAW);
        read.push(ConstructionQueueRow {
            tag_db_id,
            line_id,
            template_id,
            active_factories,
            cost_raw,
            produced_raw,
            priority: (offset / CONSTRUCTION_STRIDE) as u32,
            target_state_id: target.map(|target| target.state_id),
            target_location_id,
            target_scope: target.map(|target| target.scope),
            repair_levels_remaining,
            created_date_raw,
        });
    }
    Some(read)
}

pub fn read_country_railway_construction_queue<M: LaneMemory>(
    mem: &M,
    module_base: usize,
    root: CountryConstructionRoot,
) -> Option<RowRead<RailwayConstructionRow>> {
    let manager = production_manager_from_country(mem, root.country_ptr)?;
    read_railway_construction_queue_from_manager(mem, module_base, manager, root.tag_db_id)
}

pub fn read_railway_construction_queue_from_manager<M: LaneMemory>(
    mem: &M,
    module_base: usize,
    manager: usize,
    tag_db_id: u32,
) -> Option<RowRead<RailwayConstructionRow>> {
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
    let exact_vtable = module_base.checked_add(RAILWAY_LINE_VTABLE_RVA)?;
    let mut read = RowRead::empty();
    for (queue_index, offset) in (0..byte_len).step_by(CONSTRUCTION_STRIDE).enumerate() {
        let line_ptr = read_u64_from_slice(&raw, offset)? as usize;
        if !is_heap_ptr(line_ptr) {
            continue;
        }
        let line_type = mem.read_u32(line_ptr.checked_add(LINE_TYPE)?)?;
        if line_type != LINE_TYPE_RAILWAY_CONSTRUCTION {
            continue;
        }
        let window = mem.read_object_window::<RAILWAY_LINE_WINDOW_LEN>(line_ptr);
        let win = window.as_ref().map(|w| w.as_slice());
        if win_u64(win, mem, line_ptr, 0)? as usize != exact_vtable {
            read.skip();
            continue;
        }
        let active_factories = win_i32(win, mem, line_ptr, LINE_ACTIVE_FACTORIES)?;
        let cost_raw = win_production_fixed_raw(win, mem, line_ptr, LINE_COST)?;
        let produced_raw = win_production_fixed_raw(win, mem, line_ptr, LINE_PRODUCED)?;
        let created_date_raw = win_u32(win, mem, line_ptr, RAILWAY_LINE_CREATED_DATE)?;
        let base_level = win_u32(win, mem, line_ptr, RAILWAY_LINE_BASE_LEVEL)?;
        let target_level = win_u32(win, mem, line_ptr, RAILWAY_LINE_TARGET_LEVEL)?;
        let completion_raw = win_u8(win, mem, line_ptr, RAILWAY_LINE_COMPLETION_FLAG)?;
        let route_ptr = win_u64(win, mem, line_ptr, RAILWAY_LINE_ROUTE_PTR)? as usize;
        let route_cap = win_u32(win, mem, line_ptr, RAILWAY_LINE_ROUTE_CAP)?;
        let route_size = win_u32(win, mem, line_ptr, RAILWAY_LINE_ROUTE_SIZE)?;
        let current_segment_index = win_i32(win, mem, line_ptr, RAILWAY_LINE_CURRENT_SEGMENT)?;
        let remaining_hours_raw = win_i64(win, mem, line_ptr, RAILWAY_LINE_REMAINING_HOURS)?;
        if !(0..=MAX_ACTIVE_FACTORIES).contains(&active_factories)
            || cost_raw < 0
            || produced_raw < 0
            || created_date_raw == 0
            || created_date_raw > MAX_CONSTRUCTION_CREATED_DATE_RAW
            || base_level > BUILDING_LEVEL_SANE_MAX
            || target_level > BUILDING_LEVEL_SANE_MAX
            || target_level < base_level
            || completion_raw > 1
            || route_size < 2
            || route_size > route_cap
            || route_size > MAX_RAILWAY_ROUTE_PROVINCES
            || !is_heap_ptr(route_ptr)
            || current_segment_index < -1
            || current_segment_index >= route_size as i32 - 1
            || remaining_hours_raw < 0
        {
            read.skip();
            continue;
        }
        let route_bytes = mem.read_bytes(route_ptr, (route_size as usize).checked_mul(4)?)?;
        if route_bytes.len() != route_size as usize * 4 {
            return None;
        }
        let mut route_province_ids = Vec::with_capacity(route_size as usize);
        let mut route_valid = true;
        for index in 0..route_size as usize {
            let province_id = read_u32_from_slice(&route_bytes, index * 4)?;
            if province_id == 0 {
                route_valid = false;
                break;
            }
            if compact_contract::bound_trips::bound_exceeded_u32(
                province_id,
                MAX_PROVINCE_ID,
                compact_contract::bound_trips::BOUND_TRIP_MAP_PROVINCE_ID,
            ) {
                route_valid = false;
                break;
            }
            route_province_ids.push(province_id);
        }
        if !route_valid {
            read.skip();
            continue;
        }
        let line_id = win_u32(win, mem, line_ptr, LINE_ID)?;
        if line_id == 0 {
            read.skip();
            continue;
        }
        read.push(RailwayConstructionRow {
            tag_db_id,
            line_id,
            queue_index: queue_index as u32,
            active_factories,
            cost_raw,
            produced_raw,
            created_date_raw,
            base_level,
            target_level,
            completion_flag: completion_raw != 0,
            current_segment_index,
            remaining_hours_raw,
            route_province_ids,
        });
    }
    Some(read)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetStateLookup {
    state_ids: BTreeSet<u32>,
    province_to_state: BTreeMap<u32, u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConstructionTarget {
    pub scope: u32,
    pub location_id: u32,
    pub state_id: u32,
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
        self.target(template_id, location_id)
            .map(|target| target.state_id)
    }

    pub fn target(&self, template_id: Option<u32>, location_id: u32) -> Option<ConstructionTarget> {
        if is_province_construction_template(template_id) {
            return Some(ConstructionTarget {
                scope: SCOPE_PROVINCE,
                location_id,
                state_id: self.province_to_state.get(&location_id).copied()?,
            });
        }
        self.state_ids
            .contains(&location_id)
            .then_some(ConstructionTarget {
                scope: SCOPE_STATE,
                location_id,
                state_id: location_id,
            })
    }
}

/// the bound check needs.
pub fn construction_template_scope(template_id: Option<u32>) -> Option<u32> {
    template_id.map(|id| {
        if PROVINCE_CONSTRUCTION_TEMPLATE_IDS.contains(&id) {
            SCOPE_PROVINCE
        } else {
            SCOPE_STATE
        }
    })
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
    if location_id == 0 {
        return None;
    }
    Some(location_id)
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
        let partial_health_raw = win_i64(win, mem, building_ptr, CBUILDING_OFF_PARTIAL_HEALTH_RAW)
            .filter(|raw| *raw >= 0 && (*raw as u64) <= CBUILDING_PARTIAL_HEALTH_RAW_PER_LEVEL)
            .map(|raw| raw as u64)
            .filter(|raw| healthy_level.is_some_and(|healthy| healthy < level || *raw == 0));
        by_type
            .entry(type_id)
            .or_default()
            .add(level, healthy_level, partial_health_raw);
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
                partial_health_raw: acc.partial_health_raw(),
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
    let railway_route_ids = frame
        .railway_construction_rows
        .as_ref()
        .map(|rows| {
            rows.iter()
                .map(|row| row.route_province_ids.len())
                .sum::<usize>()
        })
        .unwrap_or(0);
    let mut out = Vec::with_capacity(
        HEADER_LEN
            + frame.construction_rows.len() * CONSTRUCTION_ROW_LEN
            + frame.building_rows.len() * BUILDING_ROW_LEN
            + frame.resource_rows.len() * RESOURCE_ROW_LEN
            + frame.railway_construction_rows.as_ref().map_or(0, |rows| {
                RAILWAY_TRAILER_HEADER_LEN
                    + rows.len() * RAILWAY_TRAILER_ROW_LEN
                    + railway_route_ids * 4
            }),
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
    if let Some(rows) = &frame.railway_construction_rows {
        write_railway_trailer(&mut out, rows);
    }
    write_coverage_trailer(&mut out, &frame.coverage);
    out
}

/// Append the D48V trailer, or — if this census is one the grammar refuses —
/// append a trailer that is DELIBERATELY UNDECODABLE.
///
/// `encode_lane_frame` is infallible by design: its callers are the collector's
/// hot path and forty test sites, and threading a `Result` through them would
/// buy nothing, because the only way to reach the error is a producer bug (the
/// census is built from `BTreeSet`s, and each incomplete set only ever receives
/// an id already admitted to `scanned_states`).
///
/// But "cannot happen" is not a reason to emit something wrong if it does. So
/// the refusal turns into a bare header with a zero-length payload: the first
/// `decode_set_block` then runs past the end and the frame is REFUSED by the
/// gate, which excludes and names the hour. A producer bug therefore costs one
/// visibly excluded hour rather than a frame whose census quietly disagrees
/// with its own rows. A test pins exactly that.
fn write_coverage_trailer(out: &mut Vec<u8>, coverage: &CoverageCensus) {
    if encode_coverage_trailer(out, coverage).is_err() {
        out.extend_from_slice(&COVERAGE_TRAILER_MAGIC);
        out.extend_from_slice(&COVERAGE_TRAILER_VERSION.to_le_bytes());
        out.extend_from_slice(&(COVERAGE_TRAILER_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }
}

fn write_railway_trailer(out: &mut Vec<u8>, rows: &[RailwayConstructionRow]) {
    let route_count = rows
        .iter()
        .map(|row| row.route_province_ids.len())
        .sum::<usize>();
    out.extend_from_slice(&RAILWAY_TRAILER_MAGIC);
    out.extend_from_slice(&RAILWAY_TRAILER_VERSION.to_le_bytes());
    out.extend_from_slice(&(RAILWAY_TRAILER_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&(route_count.min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&(RAILWAY_TRAILER_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    let mut route_start = 0u32;
    for row in rows {
        out.extend_from_slice(&row.tag_db_id.to_le_bytes());
        out.extend_from_slice(&row.line_id.to_le_bytes());
        out.extend_from_slice(&row.queue_index.to_le_bytes());
        out.extend_from_slice(&row.active_factories.to_le_bytes());
        out.extend_from_slice(&row.cost_raw.to_le_bytes());
        out.extend_from_slice(&row.produced_raw.to_le_bytes());
        out.extend_from_slice(&row.created_date_raw.to_le_bytes());
        out.extend_from_slice(&row.base_level.to_le_bytes());
        out.extend_from_slice(&row.target_level.to_le_bytes());
        out.extend_from_slice(&row.current_segment_index.to_le_bytes());
        out.extend_from_slice(&row.remaining_hours_raw.to_le_bytes());
        out.extend_from_slice(&route_start.to_le_bytes());
        out.extend_from_slice(&(row.route_province_ids.len() as u32).to_le_bytes());
        out.extend_from_slice(&u32::from(row.completion_flag).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        route_start = route_start.saturating_add(row.route_province_ids.len() as u32);
    }
    for row in rows {
        for province_id in &row.route_province_ids {
            out.extend_from_slice(&province_id.to_le_bytes());
        }
    }
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
    let base_len = HEADER_LEN
        .saturating_add(construction_count.saturating_mul(CONSTRUCTION_ROW_LEN))
        .saturating_add(building_count.saturating_mul(BUILDING_ROW_LEN))
        .saturating_add(resource_count.saturating_mul(RESOURCE_ROW_LEN));
    if bytes.len() < base_len {
        return Err(format!(
            "payload length {} shorter than base {base_len}",
            bytes.len()
        ));
    }

    let mut cursor = HEADER_LEN;
    let mut construction_rows = Vec::with_capacity(construction_count);
    for index in 0..construction_count {
        let flags = read_u32_le(bytes, cursor + 32);
        if flags & !CONSTRUCTION_ROW_KNOWN_FLAGS != 0 {
            return Err(format!(
                "construction row {index} has unknown flags {flags:#x}"
            ));
        }
        if deleted_identity_id(read_u32_le(bytes, cursor + 4)).is_some()
            && (read_u32_le(bytes, cursor + 8) != u32::MAX
                || read_u32_le(bytes, cursor + 28) != 0
                || read_u32_le(bytes, cursor + 36) != 0
                || read_u32_le(bytes, cursor + 40) != 0
                || read_u32_le(bytes, cursor + 44) != 0
                || read_u32_le(bytes, cursor + 48) != 0)
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
        for (flag, field_offset, field_name) in [
            (
                CONSTRUCTION_FLAG_TARGET_LOCATION_PRESENT,
                40,
                "target_location",
            ),
            (CONSTRUCTION_FLAG_TARGET_SCOPE_PRESENT, 44, "target_scope"),
            (CONSTRUCTION_FLAG_REPAIR_LEVELS_PRESENT, 48, "repair_levels"),
        ] {
            if flags & flag == 0 && read_u32_le(bytes, cursor + field_offset) != 0 {
                return Err(format!(
                    "construction row {index} {field_name} reserved field is nonzero"
                ));
            }
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
        if flags & BUILDING_FLAG_PARTIAL_HEALTH_PRESENT == 0 && read_u64_le(bytes, cursor + 32) != 0
        {
            return Err(format!(
                "building row {index} partial_health reserved field is nonzero"
            ));
        }
        if deleted_identity_id(read_u32_le(bytes, cursor + 12)).is_some()
            && (read_u32_le(bytes, cursor + 20) != u32::MAX || read_u64_le(bytes, cursor + 32) != 0)
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
    debug_assert_eq!(cursor, base_len);
    let (railway_construction_rows, coverage) = decode_frame_tail(
        &bytes[cursor..],
        observed_state_roots,
        observed_province_roots,
    )?;

    Ok(LaneFrame {
        game_hour_raw,
        status_flags,
        source_flags,
        observed_country_roots,
        observed_state_roots,
        observed_province_roots,
        construction_rows,
        railway_construction_rows,
        building_rows,
        resource_rows,
        coverage,
    })
}

/// The tail after the three row tables: an optional `RLC1` trailer followed by
/// the MANDATORY `D48V` coverage trailer.
///
/// The split, the RLC1 extent arithmetic and every D48V refusal live in
/// `d48_grammar`, shared byte-for-byte with `wire_format`'s independent d48
/// port. What stays here is this lane's own railway ROW grammar, which the
/// wire port deliberately reads more strictly than the producer writes it.
pub fn decode_frame_tail(
    tail: &[u8],
    observed_state_roots: u32,
    observed_province_roots: u32,
) -> Result<(Option<Vec<RailwayConstructionRow>>, CoverageCensus), String> {
    let (railway_bytes, coverage_bytes) = split_frame_tail(tail)?;
    let railway_construction_rows = match railway_bytes {
        Some(bytes) => Some(decode_railway_trailer_rows(bytes)?),
        None => None,
    };
    let coverage = decode_coverage_trailer(
        coverage_bytes,
        observed_state_roots,
        observed_province_roots,
        MAX_STATE_ID,
        MAX_PROVINCE_ID,
    )?;
    Ok((railway_construction_rows, coverage))
}


/// Decode the ROWS of an `RLC1` trailer whose extent `d48_grammar` has already
/// computed, so `trailer` is exactly the railway trailer and nothing else.
///
/// The exact-length rule therefore stays — it just moved. It used to mean "the
/// RLC1 trailer is the whole rest of the payload", which is what made appending
/// D48V impossible; it now means "the bytes the shared tail split assigned to
/// RLC1 are entirely consumed by RLC1", and D48V enforces the same rule on its
/// own slice. Nothing can hide between the two.
pub fn decode_railway_trailer_rows(
    trailer: &[u8],
) -> Result<Vec<RailwayConstructionRow>, String> {
    if trailer.len() < RAILWAY_TRAILER_HEADER_LEN {
        return Err("RLC1 trailer truncated".to_string());
    }
    if &trailer[0..4] != RAILWAY_TRAILER_MAGIC.as_slice() {
        return Err("unknown D48 trailer magic".to_string());
    }
    if read_u16_le(trailer, 4) != RAILWAY_TRAILER_VERSION
        || read_u16_le(trailer, 6) as usize != RAILWAY_TRAILER_HEADER_LEN
        || read_u16_le(trailer, 16) as usize != RAILWAY_TRAILER_ROW_LEN
        || read_u16_le(trailer, 18) != 0
        || read_u32_le(trailer, 20) != 0
    {
        return Err("RLC1 header mismatch".to_string());
    }
    let row_count = read_u32_le(trailer, 8) as usize;
    let route_count = read_u32_le(trailer, 12) as usize;
    if route_count > row_count.saturating_mul(MAX_RAILWAY_ROUTE_PROVINCES as usize) {
        return Err("RLC1 route-count ceiling exceeded".to_string());
    }
    let rows_len = row_count
        .checked_mul(RAILWAY_TRAILER_ROW_LEN)
        .ok_or_else(|| "RLC1 row length overflow".to_string())?;
    let routes_len = route_count
        .checked_mul(4)
        .ok_or_else(|| "RLC1 route length overflow".to_string())?;
    let expected = RAILWAY_TRAILER_HEADER_LEN
        .checked_add(rows_len)
        .and_then(|value| value.checked_add(routes_len))
        .ok_or_else(|| "RLC1 total length overflow".to_string())?;
    if trailer.len() != expected {
        return Err(format!(
            "RLC1 length {} != expected {expected}",
            trailer.len()
        ));
    }
    let route_base = RAILWAY_TRAILER_HEADER_LEN + rows_len;
    let mut rows = Vec::with_capacity(row_count);
    let mut expected_route_start = 0usize;
    let mut previous_key = None;
    for index in 0..row_count {
        let offset = RAILWAY_TRAILER_HEADER_LEN + index * RAILWAY_TRAILER_ROW_LEN;
        let tag_db_id = read_u32_le(trailer, offset);
        let line_id = read_u32_le(trailer, offset + 4);
        let queue_index = read_u32_le(trailer, offset + 8);
        let active_factories = read_i32_le(trailer, offset + 12);
        let cost_raw = read_i32_le(trailer, offset + 16);
        let produced_raw = read_i32_le(trailer, offset + 20);
        let created_date_raw = read_u32_le(trailer, offset + 24);
        let base_level = read_u32_le(trailer, offset + 28);
        let target_level = read_u32_le(trailer, offset + 32);
        let current_segment_index = read_i32_le(trailer, offset + 36);
        let remaining_hours_raw = read_i64_le(trailer, offset + 40);
        let route_start = read_u32_le(trailer, offset + 48) as usize;
        let route_len = read_u32_le(trailer, offset + 52) as usize;
        let flags = read_u32_le(trailer, offset + 56);
        if read_u32_le(trailer, offset + 60) != 0 || flags & !1 != 0 {
            return Err(format!("RLC1 row {index} flags/reserved invalid"));
        }
        if tag_db_id == 0
            || line_id == 0
            || !(0..=MAX_ACTIVE_FACTORIES).contains(&active_factories)
            || cost_raw < 0
            || produced_raw < 0
            || created_date_raw == 0
            || created_date_raw > MAX_CONSTRUCTION_CREATED_DATE_RAW
            || base_level > BUILDING_LEVEL_SANE_MAX
            || target_level < base_level
            || target_level > BUILDING_LEVEL_SANE_MAX
            || !(2..=MAX_RAILWAY_ROUTE_PROVINCES as usize).contains(&route_len)
            || route_start != expected_route_start
            || route_start.saturating_add(route_len) > route_count
            || current_segment_index < -1
            || current_segment_index >= route_len as i32 - 1
            || remaining_hours_raw < 0
        {
            return Err(format!("RLC1 row {index} invariant failed"));
        }
        let key = (tag_db_id, queue_index, line_id);
        if previous_key.is_some_and(|previous| previous >= key) {
            return Err(format!("RLC1 row {index} is not canonically ordered"));
        }
        previous_key = Some(key);
        let mut route_province_ids = Vec::with_capacity(route_len);
        for route_index in 0..route_len {
            let province_id = read_u32_le(trailer, route_base + (route_start + route_index) * 4);
            if province_id == 0 || province_id > MAX_PROVINCE_ID {
                return Err(format!("RLC1 row {index} route province invalid"));
            }
            route_province_ids.push(province_id);
        }
        expected_route_start += route_len;
        rows.push(RailwayConstructionRow {
            tag_db_id,
            line_id,
            queue_index,
            active_factories,
            cost_raw,
            produced_raw,
            created_date_raw,
            base_level,
            target_level,
            completion_flag: flags & 1 != 0,
            current_segment_index,
            remaining_hours_raw,
            route_province_ids,
        });
    }
    if expected_route_start != route_count {
        return Err("RLC1 unreferenced route tail".to_string());
    }
    Ok(rows)
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
    // Every row must name a state the census says was scanned — tombstones
    // included, because a state-census change resets the delta baseline and
    // re-emits a full frame, so a tombstone can only ever name a state that is
    // still in the scanned set. `scanned_states` is sorted by construction.
    let scanned = &frame.coverage.scanned_states;
    for (index, row) in frame.building_rows.iter().enumerate() {
        if scanned.binary_search(&row.state_id).is_err() {
            errors.push(format!(
                "building row {index} names a state outside scanned_states"
            ));
        }
    }
    for (index, row) in frame.resource_rows.iter().enumerate() {
        if scanned.binary_search(&row.state_id).is_err() {
            errors.push(format!(
                "resource row {index} names a state outside scanned_states"
            ));
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
    if row.target_location_id.is_some() {
        flags |= CONSTRUCTION_FLAG_TARGET_LOCATION_PRESENT;
    }
    if row.target_scope.is_some() {
        flags |= CONSTRUCTION_FLAG_TARGET_SCOPE_PRESENT;
    }
    if row.repair_levels_remaining.is_some() {
        flags |= CONSTRUCTION_FLAG_REPAIR_LEVELS_PRESENT;
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
    out.extend_from_slice(&row.created_date_raw.unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&row.target_location_id.unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&row.target_scope.unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&row.repair_levels_remaining.unwrap_or(0).to_le_bytes());
}

fn write_building_row(out: &mut Vec<u8>, row: &BuildingRow) {
    let mut flags = 0u32;
    if let Some(healthy) = row.healthy_level {
        flags |= BUILDING_FLAG_HEALTHY_PRESENT;
        if healthy < row.level {
            flags |= BUILDING_FLAG_DAMAGED;
        }
    }
    if row.partial_health_raw.is_some() {
        flags |= BUILDING_FLAG_PARTIAL_HEALTH_PRESENT;
    }
    out.extend_from_slice(&row.scope.to_le_bytes());
    out.extend_from_slice(&row.location_id.to_le_bytes());
    out.extend_from_slice(&row.state_id.to_le_bytes());
    out.extend_from_slice(&row.type_id.to_le_bytes());
    out.extend_from_slice(&row.level.to_le_bytes());
    out.extend_from_slice(&row.healthy_level.unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&row.partial_health_raw.unwrap_or(0).to_le_bytes());
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
    let target_location_raw = read_u32_le(bytes, offset + 40);
    let target_scope_raw = read_u32_le(bytes, offset + 44);
    let repair_levels_raw = read_u32_le(bytes, offset + 48);
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
        target_location_id: (flags & CONSTRUCTION_FLAG_TARGET_LOCATION_PRESENT != 0)
            .then_some(target_location_raw),
        target_scope: (flags & CONSTRUCTION_FLAG_TARGET_SCOPE_PRESENT != 0)
            .then_some(target_scope_raw),
        repair_levels_remaining: (flags & CONSTRUCTION_FLAG_REPAIR_LEVELS_PRESENT != 0)
            .then_some(repair_levels_raw),
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
        partial_health_raw: (flags & BUILDING_FLAG_PARTIAL_HEALTH_PRESENT != 0)
            .then_some(read_u64_le(bytes, offset + 32)),
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
            || row.target_location_id.is_some()
            || row.target_scope.is_some()
            || row.repair_levels_remaining.is_some()
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
    if row.target_location_id.is_some() != row.target_scope.is_some() {
        return Err(format!(
            "construction row {index} target_location_id/target_scope presence mismatch"
        ));
    }
    if let (Some(location_id), Some(scope)) = (row.target_location_id, row.target_scope) {
        if scope != SCOPE_STATE && scope != SCOPE_PROVINCE {
            return Err(format!(
                "construction row {index} invalid target_scope {scope}"
            ));
        }
        let (space_max, space_unit) = if scope == SCOPE_STATE {
            (
                MAX_STATE_ID,
                compact_contract::bound_trips::BOUND_TRIP_MAP_STATE_ID,
            )
        } else {
            (
                MAX_PROVINCE_ID,
                compact_contract::bound_trips::BOUND_TRIP_MAP_PROVINCE_ID,
            )
        };
        if location_id == 0
            || compact_contract::bound_trips::bound_exceeded_u32(location_id, space_max, space_unit)
        {
            return Err(format!(
                "construction row {index} invalid target_location_id {location_id}"
            ));
        }
        if scope == SCOPE_STATE && row.target_state_id != Some(location_id) {
            return Err(format!(
                "construction row {index} state target location/state mismatch"
            ));
        }
        if scope == SCOPE_PROVINCE && row.target_state_id.is_none() {
            return Err(format!(
                "construction row {index} province target lacks target_state_id"
            ));
        }
    }
    if let Some(repair_levels) = row.repair_levels_remaining {
        if repair_levels > BUILDING_LEVEL_SANE_MAX {
            return Err(format!(
                "construction row {index} invalid repair_levels_remaining {repair_levels}"
            ));
        }
    }
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
        if row.level != 0 || row.healthy_level.is_some() || row.partial_health_raw.is_some() {
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
    if let Some(partial) = row.partial_health_raw {
        let max = u64::from(row.level).saturating_mul(CBUILDING_PARTIAL_HEALTH_RAW_PER_LEVEL);
        if partial > max {
            return Err(format!(
                "building row {index} partial_health_raw {partial} > maximum {max}"
            ));
        }
        if row.healthy_level == Some(row.level) && partial != 0 {
            return Err(format!(
                "building row {index} undamaged building has nonzero partial_health_raw"
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
    partial_health_raw: u64,
    partial_health_complete: bool,
    seen: bool,
}

impl BuildingAccumulator {
    fn add(&mut self, level: u32, healthy_level: Option<u32>, partial_health_raw: Option<u64>) {
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
        match partial_health_raw {
            Some(partial) if !self.seen || self.partial_health_complete => {
                self.partial_health_raw = self.partial_health_raw.saturating_add(partial);
                self.partial_health_complete = true;
            }
            _ => self.partial_health_complete = false,
        }
        self.seen = true;
    }

    fn healthy_level(&self) -> Option<u32> {
        self.healthy_complete.then_some(self.healthy)
    }

    fn partial_health_raw(&self) -> Option<u64> {
        self.partial_health_complete
            .then_some(self.partial_health_raw)
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

#[inline]
fn win_u64<M: LaneMemory>(win: Option<&[u8]>, mem: &M, addr: usize, off: usize) -> Option<u64> {
    match win {
        Some(buf) => read_u64_from_slice(buf, off),
        None => mem.read_u64(addr.checked_add(off)?),
    }
}

#[inline]
fn win_u8<M: LaneMemory>(win: Option<&[u8]>, mem: &M, addr: usize, off: usize) -> Option<u8> {
    match win {
        Some(buf) => buf.get(off).copied(),
        None => mem.read_u8(addr.checked_add(off)?),
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

#[inline]
fn win_i64<M: LaneMemory>(win: Option<&[u8]>, mem: &M, addr: usize, off: usize) -> Option<i64> {
    match win {
        Some(buf) => Some(i64::from_le_bytes(
            buf.get(off..off.checked_add(8)?)?.try_into().ok()?,
        )),
        None => mem.read_i64(addr.checked_add(off)?),
    }
}

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

fn read_i64_le(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("i64 range"))
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("u64 range"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // kills: letting a census the grammar refuses reach the wire as something
    // that DECODES. `encode_lane_frame` is infallible on purpose, so the only
    // question is what it writes when the census is malformed - and the answer
    // has to be bytes the gate refuses, never bytes whose census quietly
    // disagrees with the frame's own rows. Make `write_coverage_trailer` fall
    // back to writing the census anyway and this test decodes a frame whose
    // failure list names a state that was never scanned.
    #[test]
    fn a_census_the_grammar_refuses_produces_a_frame_the_decoder_refuses() {
        let mut frame = sample_frame();
        // A failure list naming a state outside the scanned set: exactly the
        // shape the encoder refuses and the decoder must never accept.
        frame.coverage.building_incomplete_states = vec![999_999];
        let bytes = encode_lane_frame(&frame);
        let error = decode_lane_frame(&bytes)
            .expect_err("a refused census must not produce a decodable frame");
        assert!(
            error.contains("D48V"),
            "the refusal must name the coverage trailer, got: {error}"
        );

        // And the ordinary census still round-trips, so the fallback is not
        // firing on the healthy path.
        let healthy = sample_frame();
        assert_eq!(
            decode_lane_frame(&encode_lane_frame(&healthy)).expect("decodes"),
            healthy
        );
    }

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

        fn write_i64(&mut self, addr: usize, value: i64) {
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
    fn extractor_retains_exact_first_invalid_progress_evidence() {
        let manager = 0x1000_0000_0000usize;
        let vector = 0x1000_0000_1000usize;
        let first_line = 0x1000_0000_2000usize;
        let second_line = 0x1000_0000_2100usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(manager + CONSTRUCTION_BEGIN, vector as u64);
        mem.write_u32(manager + CONSTRUCTION_CAP, 2);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 2);
        mem.write_u64(vector, first_line as u64);
        mem.write_u64(vector + CONSTRUCTION_STRIDE, second_line as u64);

        mem.write_u32(first_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(first_line + LINE_ID, 77);
        mem.write_i32(first_line + LINE_ACTIVE_FACTORIES, 15);
        mem.write_i32(first_line + LINE_COST, -73_728);
        mem.write_i32(first_line + LINE_PRODUCED, 5_000_000);

        mem.write_u32(second_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(second_line + LINE_ID, 78);
        mem.write_i32(second_line + LINE_ACTIVE_FACTORIES, 10);
        mem.write_i32(second_line + LINE_COST, 700_000_000);
        mem.write_i32(second_line + LINE_PRODUCED, -1);

        let read = read_construction_queue_from_manager(&mem, manager, 123).unwrap();

        assert!(read.rows.is_empty());
        assert_eq!(read.skipped_rows, 2);
        assert_eq!(read.construction_skip_stats.invalid_progress_rows, 2);
        assert_eq!(
            read.construction_skip_stats.first_invalid_progress,
            Some(InvalidConstructionProgress {
                line_address: first_line,
                tag_db_id: 123,
                queue_index: 0,
                line_type: LINE_TYPE_CONSTRUCTION,
                line_id: 77,
                cost_raw: -73_728,
                produced_raw: 5_000_000,
            })
        );
    }

    #[test]
    fn extractor_counts_non_construction_type_as_uncertain_skip() {
        let manager = 0x1000_0000_0000usize;
        let vector = 0x1000_0000_1000usize;
        let construction_line = 0x1000_0000_2000usize;
        let other_line = 0x1000_0000_2100usize;
        let sub = 0x1000_0000_3000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(manager + CONSTRUCTION_BEGIN, vector as u64);
        mem.write_u32(manager + CONSTRUCTION_CAP, 2);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 2);
        mem.write_u64(vector, construction_line as u64);
        mem.write_u64(vector + 8, other_line as u64);
        mem.write_u32(construction_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(construction_line + LINE_ID, 77);
        mem.write_i32(construction_line + LINE_ACTIVE_FACTORIES, 15);
        mem.write_i32(construction_line + LINE_COST, 1_450_000_000);
        mem.write_i32(construction_line + LINE_PRODUCED, 5_000_000);
        mem.write_u64(construction_line + LINE_CONSTRUCTION_SUB, sub as u64);
        mem.write_u32(sub + CONSTRUCTION_TEMPLATE_ID, 20_013);
        mem.write_u32(other_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION - 1);

        let read = read_construction_queue_from_manager(&mem, manager, 123).unwrap();

        assert_eq!(read.rows.len(), 1);
        assert_eq!(read.rows[0].line_id, 77);
        assert_eq!(read.skipped_rows, 1);
        assert_eq!(read.construction_skip_stats.total(), 1);
        assert_eq!(read.construction_skip_stats.non_construction_line_rows, 1);
    }

    #[test]
    fn ordinary_reader_routes_exact_railway_subtype_without_partial_skip() {
        let manager = 0x1000_0000_0000usize;
        let vector = 0x1000_0000_1000usize;
        let railway_line = 0x1000_0000_2000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(manager + CONSTRUCTION_BEGIN, vector as u64);
        mem.write_u32(manager + CONSTRUCTION_CAP, 1);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 1);
        mem.write_u64(vector, railway_line as u64);
        mem.write_u32(railway_line + LINE_TYPE, LINE_TYPE_RAILWAY_CONSTRUCTION);

        let read = read_construction_queue_from_manager(&mem, manager, 123).unwrap();

        assert!(read.rows.is_empty());
        assert_eq!(read.skipped_rows, 0);
        assert_eq!(read.construction_skip_stats.total(), 0);
    }

    #[test]
    fn exact_railway_reader_matches_d04_save_anchor_and_rlc1_round_trips() {
        let module_base = 0x1400_0000usize;
        let manager = 0x1000_0000_0000usize;
        let vector = 0x1000_0000_1000usize;
        let line = 0x1000_0000_2000usize;
        let route_ptr = 0x1000_0000_3000usize;
        let route = [
            9_422u32, 11_408, 9_564, 3_539, 11_485, 9_436, 3_582, 9_504, 3_529,
        ];
        let mut mem = FakeMemory::default();
        mem.write_u64(manager + CONSTRUCTION_BEGIN, vector as u64);
        mem.write_u32(manager + CONSTRUCTION_CAP, 1);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 1);
        mem.write_u64(vector, line as u64);
        mem.write_u64(line, (module_base + RAILWAY_LINE_VTABLE_RVA) as u64);
        mem.write_u32(line + LINE_TYPE, LINE_TYPE_RAILWAY_CONSTRUCTION);
        mem.write_u32(line + LINE_ID, 135_764);
        mem.write_i32(line + LINE_ACTIVE_FACTORIES, 15);
        mem.write_i32(line + LINE_COST, 448_000_000);
        mem.write_i32(line + LINE_PRODUCED, 231_350_000);
        mem.write_u32(line + RAILWAY_LINE_CREATED_DATE, 60_827_637);
        mem.write_u32(line + RAILWAY_LINE_BASE_LEVEL, 2);
        mem.write_u32(line + RAILWAY_LINE_TARGET_LEVEL, 3);
        mem.write_bytes(line + RAILWAY_LINE_COMPLETION_FLAG, &[0]);
        mem.write_u64(line + RAILWAY_LINE_ROUTE_PTR, route_ptr as u64);
        mem.write_u32(line + RAILWAY_LINE_ROUTE_CAP, route.len() as u32);
        mem.write_u32(line + RAILWAY_LINE_ROUTE_SIZE, route.len() as u32);
        mem.write_i32(line + RAILWAY_LINE_CURRENT_SEGMENT, 3);
        mem.write_i64(line + RAILWAY_LINE_REMAINING_HOURS, 216_650_000);
        for (index, province_id) in route.into_iter().enumerate() {
            mem.write_u32(route_ptr + index * 4, province_id);
        }

        let read =
            read_railway_construction_queue_from_manager(&mem, module_base, manager, 369).unwrap();
        assert_eq!(read.skipped_rows, 0);
        assert_eq!(read.rows.len(), 1);
        let row = &read.rows[0];
        assert_eq!(row.line_id, 135_764);
        assert_eq!(row.route_province_ids, route);
        assert_eq!(row.current_segment_index, 3);
        assert_eq!(row.remaining_hours_raw, 216_650_000);

        let mut frame = sample_frame();
        frame.railway_construction_rows = Some(read.rows);
        let encoded = encode_lane_frame(&frame);
        let decoded = decode_lane_frame(&encoded).unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn collector_non_construction_type_sets_partial() {
        let country = 0x1000_0000_0000usize;
        let manager = 0x1000_0000_1000usize;
        let vector = 0x1000_0000_2000usize;
        let construction_line = 0x1000_0000_2800usize;
        let other_line = 0x1000_0000_3000usize;
        let sub = 0x1000_0000_4000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(country + OBJ_PROD_MANAGER, manager as u64);
        mem.write_u64(manager + CONSTRUCTION_BEGIN, vector as u64);
        mem.write_u32(manager + CONSTRUCTION_CAP, 2);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 2);
        mem.write_u64(vector, construction_line as u64);
        mem.write_u64(vector + 8, other_line as u64);
        mem.write_u32(construction_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(construction_line + LINE_ID, 77);
        mem.write_i32(construction_line + LINE_ACTIVE_FACTORIES, 15);
        mem.write_i32(construction_line + LINE_COST, 1_450_000_000);
        mem.write_i32(construction_line + LINE_PRODUCED, 5_000_000);
        mem.write_u64(construction_line + LINE_CONSTRUCTION_SUB, sub as u64);
        mem.write_u32(sub + CONSTRUCTION_TEMPLATE_ID, 20_013);
        mem.write_u32(other_line + LINE_TYPE, LINE_TYPE_CONSTRUCTION - 1);

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
            output.stats.skipped_construction_non_construction_line_rows,
            1
        );
        assert_ne!(output.frame.status_flags & STATUS_PARTIAL_DATA, 0);
        assert_eq!(output.frame.construction_rows.len(), 1);
        assert_eq!(output.frame.construction_rows[0].line_id, 77);
    }

    #[test]
    fn uncertain_type_mismatch_after_baseline_stays_partial_without_tombstone() {
        let country = 0x1000_0000_0000usize;
        let manager = 0x1000_0000_1000usize;
        let vector = 0x1000_0000_2000usize;
        let line = 0x1000_0000_3000usize;
        let sub = 0x1000_0000_4000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(country + OBJ_PROD_MANAGER, manager as u64);
        mem.write_u64(manager + CONSTRUCTION_BEGIN, vector as u64);
        mem.write_u32(manager + CONSTRUCTION_CAP, 1);
        mem.write_u32(manager + CONSTRUCTION_SIZE, 1);
        mem.write_u64(vector, line as u64);
        mem.write_u32(line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
        mem.write_u32(line + LINE_ID, 77);
        mem.write_i32(line + LINE_ACTIVE_FACTORIES, 15);
        mem.write_i32(line + LINE_COST, 1_450_000_000);
        mem.write_i32(line + LINE_PRODUCED, 5_000_000);
        mem.write_u64(line + LINE_CONSTRUCTION_SUB, sub as u64);
        mem.write_u32(sub + CONSTRUCTION_TEMPLATE_ID, 20_013);

        let roots = |game_hour_raw| collector_module::CollectorRoots {
            module_base: 0x1400_0000,
            game_hour_raw,
            country_roots: vec![CountryConstructionRoot {
                tag_db_id: 123,
                country_ptr: country,
            }],
            state_roots: Vec::new(),
            province_roots: Vec::new(),
        };

        let baseline = collector_module::collect_domain48_frame(&mem, roots(320));
        assert_eq!(baseline.frame.status_flags & STATUS_PARTIAL_DATA, 0);
        assert_eq!(baseline.frame.construction_rows.len(), 1);
        assert_eq!(baseline.frame.construction_rows[0].line_id, 77);

        mem.write_u32(line + LINE_TYPE, LINE_TYPE_CONSTRUCTION - 1);
        let uncertain = collector_module::collect_domain48_frame(&mem, roots(321));

        assert_eq!(uncertain.stats.skipped_construction_rows, 1);
        assert_eq!(
            uncertain
                .stats
                .skipped_construction_non_construction_line_rows,
            1
        );
        assert_ne!(
            uncertain.frame.status_flags & STATUS_PARTIAL_DATA,
            0,
            "uncertain type mismatch must keep the frame partial"
        );
        assert!(
            uncertain
                .frame
                .construction_rows
                .iter()
                .all(|row| construction_row_deleted_id(row).is_none()),
            "uncertain omission must not emit a deletion tombstone for baseline row K"
        );
        assert!(uncertain.frame.construction_rows.is_empty());
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
    fn an_over_width_construction_target_reaches_its_own_id_spaces_bound() {
        use compact_contract::bound_trips as bt;

        for (template_id, over_width, expected_unit, other_unit) in [
            (
                20_013u32,
                MAX_STATE_ID as u64 + 1,
                bt::BOUND_TRIP_MAP_STATE_ID,
                bt::BOUND_TRIP_MAP_PROVINCE_ID,
            ),
            (
                12_790u32,
                MAX_PROVINCE_ID as u64 + 1,
                bt::BOUND_TRIP_MAP_PROVINCE_ID,
                bt::BOUND_TRIP_MAP_STATE_ID,
            ),
        ] {
            let _registry = bt::lock_registry_for_tests();
            let _ = bt::drain_frame_trips();

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
            mem.write_u32(line + LINE_TYPE, LINE_TYPE_CONSTRUCTION);
            mem.write_u32(line + LINE_ID, 91);
            mem.write_i32(line + LINE_ACTIVE_FACTORIES, 1);
            mem.write_i32(line + LINE_COST, 1);
            mem.write_i32(line + LINE_PRODUCED, 0);
            mem.write_u64(line + LINE_CONSTRUCTION_SUB, sub as u64);
            mem.write_u32(sub + CONSTRUCTION_TEMPLATE_ID, template_id);
            mem.write_u64(sub + CONSTRUCTION_TARGET_LOCATION_PTR, target as u64);
            mem.write_u32(target + TARGET_LOCATION_ID, over_width as u32);
            mem.write_i32(line + LINE_REPAIR_LEVELS_REMAINING, 0);

            let lookup = TargetStateLookup::new(&[], &[]);
            let read = read_construction_queue_from_manager_with_targets(
                &mem,
                manager,
                123,
                Some(&lookup),
            )
            .expect("the queue still reads");

            assert_eq!(read.rows.len(), 1, "the row itself is still emitted");
            assert_eq!(
                read.rows[0].target_location_id, None,
                "an out-of-space target is refused, not published"
            );

            let drained = bt::drain_frame_trips();
            assert_eq!(
                drained[expected_unit],
                (1, over_width),
                "the refusal must reach the id space the TEMPLATE names, with \
                 the offending identity as the raw sample"
            );
            assert_eq!(
                drained[other_unit].0, 0,
                "and must not reach the other map id space's unit"
            );
            assert_eq!(
                drained[bt::BOUND_TRIP_SUPPLY_PROVINCE_ID].0,
                0,
                "nor the supply family's province unit, which bounds a \
                 different thing against a live ceiling"
            );
        }
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
        mem.write_i32(state_line + LINE_REPAIR_LEVELS_REMAINING, 0);

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
        mem.write_i32(province_line + LINE_REPAIR_LEVELS_REMAINING, 2);

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
        assert_eq!(read.rows[0].target_location_id, Some(42));
        assert_eq!(read.rows[0].target_scope, Some(SCOPE_STATE));
        assert_eq!(read.rows[0].repair_levels_remaining, Some(0));
        assert_eq!(read.rows[1].target_location_id, Some(2_068));
        assert_eq!(read.rows[1].target_scope, Some(SCOPE_PROVINCE));
        assert_eq!(read.rows[1].repair_levels_remaining, Some(2));
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
        mem.write_u64(building + CBUILDING_OFF_PARTIAL_HEALTH_RAW, 8_184_501);

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
                partial_health_raw: Some(8_184_501),
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
        let mut frame = sample_frame();
        frame.construction_rows[0].created_date_raw = Some(60_827_043);
        let encoded = encode_lane_frame(&frame);
        assert_eq!(CONSTRUCTION_ROW_LEN, 52);
        let decoded = decode_lane_frame(&encoded).unwrap();
        assert_eq!(
            decoded.construction_rows[0].created_date_raw,
            Some(60_827_043)
        );
        assert!(gate_lane_frame(&encoded).ok);
    }

    #[test]
    fn compact_frame_round_trips_absent_created_date() {
        let mut frame = sample_frame();
        frame.construction_rows[0].created_date_raw = None;
        let encoded = encode_lane_frame(&frame);
        let decoded = decode_lane_frame(&encoded).unwrap();
        assert_eq!(decoded.construction_rows[0].created_date_raw, None);
        let row_start = HEADER_LEN;
        assert_eq!(read_u32_le(&encoded, row_start + 36), 0);
        assert!(gate_lane_frame(&encoded).ok);
    }

    #[test]
    fn a14_damage_reason_enum_is_exact_and_closed() {
        let expected = [
            BuildingDamageReason::None,
            BuildingDamageReason::Bombed,
            BuildingDamageReason::Sabotaged,
            BuildingDamageReason::Collateral,
            BuildingDamageReason::Repaired,
            BuildingDamageReason::Scorched,
        ];
        for (raw, reason) in expected.into_iter().enumerate() {
            assert_eq!(BuildingDamageReason::from_raw(raw as u32), Some(reason));
        }
        assert_eq!(BuildingDamageReason::Repaired.token(), "repaired");
        assert_eq!(BuildingDamageReason::from_raw(6), None);
        assert_eq!(BuildingDamageReason::from_raw(u32::MAX), None);
    }

    #[test]
    fn a14_compact_round_trips_exact_location_and_repair_state() {
        let mut frame = sample_frame();
        frame.source_flags &= !SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
        let row = &mut frame.construction_rows[0];
        row.target_state_id = Some(42);
        row.target_location_id = Some(2_068);
        row.target_scope = Some(SCOPE_PROVINCE);
        row.repair_levels_remaining = Some(1);
        frame.building_rows[0].partial_health_raw = Some(8_184_501);

        let encoded = encode_lane_frame(&frame);
        let decoded = decode_lane_frame(&encoded).unwrap();
        assert_eq!(decoded, frame);
        assert!(gate_lane_frame(&encoded).ok);
    }

    #[test]
    fn a14_compact_decode_rejects_unflagged_appended_payloads() {
        for (field_offset, field_name) in [
            (40usize, "target_location"),
            (44, "target_scope"),
            (48, "repair_levels"),
        ] {
            let mut encoded = encode_lane_frame(&sample_frame());
            let field = HEADER_LEN + field_offset;
            encoded[field..field + 4].copy_from_slice(&1u32.to_le_bytes());
            let err = decode_lane_frame(&encoded).unwrap_err();
            assert!(err.contains(field_name), "{field_name}: {err}");
        }

        let mut frame = sample_frame();
        frame.building_rows[0].partial_health_raw = None;
        let mut encoded = encode_lane_frame(&frame);
        let field = HEADER_LEN + CONSTRUCTION_ROW_LEN + 32;
        encoded[field..field + 8].copy_from_slice(&1u64.to_le_bytes());
        let err = decode_lane_frame(&encoded).unwrap_err();
        assert!(err.contains("partial_health"), "{err}");
    }

    #[test]
    fn compact_gate_rejects_zero_created_date() {
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
        let frame = sample_frame();
        let mut encoded = encode_lane_frame(&frame);
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
            target_location_id: None,
            target_scope: None,
            repair_levels_remaining: None,
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
        assert_eq!(bulk.skipped_rows, 1);
        assert_eq!(bulk.rows.len(), 2);
        assert_eq!(bulk.rows[0].healthy_level, Some(4));
    }

    #[test]
    fn bulk_window_construction_rows_match_per_field_path() {
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
                target_location_id: None,
                target_scope: None,
                repair_levels_remaining: None,
                created_date_raw: Some(60_827_043),
            }],
            railway_construction_rows: None,
            building_rows: vec![
                BuildingRow {
                    scope: SCOPE_STATE,
                    location_id: 42,
                    state_id: 42,
                    type_id: 0x2FAB,
                    level: 5,
                    healthy_level: Some(4),
                    partial_health_raw: Some(8_184_501),
                },
                BuildingRow {
                    scope: SCOPE_PROVINCE,
                    location_id: 2068,
                    state_id: 42,
                    type_id: 0x4CC1,
                    level: 2,
                    healthy_level: Some(2),
                    partial_health_raw: Some(0),
                },
            ],
            resource_rows: vec![StateResourceRow {
                state_id: 42,
                resource_id: 0x3E03,
                amount_raw: 2_500_000,
            }],
            coverage: CoverageCensus {
                census_flags: CENSUS_FLAG_STATE_ROOTS_AUTHORITATIVE
                    | CENSUS_FLAG_PROVINCE_FAILURES_AUTHORITATIVE,
                // Exactly `observed_state_roots` unique ids, containing every
                // state the rows above name.
                scanned_states: (1..=1081).collect(),
                building_incomplete_states: Vec::new(),
                resource_incomplete_states: Vec::new(),
                building_incomplete_provinces: Vec::new(),
            },
        }
    }
}
