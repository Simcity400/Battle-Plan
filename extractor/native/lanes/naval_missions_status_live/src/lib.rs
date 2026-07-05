#[path = "domain56_compact.rs"]
pub mod compact;

use std::collections::BTreeMap;

pub use compact::{
    decode_payload, decode_status_payload, encode_payload, DecodedNavalMissionStatusPayload,
    Domain56GateSummary, NavalMissionStatusHeader, NavalMissionStatusRow, NavalSupremacyRow,
    DOMAIN_ID, DOMAIN_NAME,
};

pub const EXPECTED_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

pub const CGAMESTATE_COUNTRY_TABLE: usize = 0x310;
pub const CGAMESTATE_COUNTRY_COUNT: usize = 0x31C;

pub const COUNTRY_VTABLE_RVA: usize = 0x27D0E90;
pub const OBJ_TAG_DB_ID: usize = 0x08;
pub const COUNTRY_FLEET_VECTOR_DATA: usize = 0x278;
pub const COUNTRY_FLEET_VECTOR_CAPACITY: usize = 0x280;
pub const COUNTRY_FLEET_VECTOR_COUNT: usize = 0x284;

pub const FLEET_VTABLE_RVA: usize = 0x294BF28;
pub const TASK_FORCE_SUBOBJECT_VTABLE_RVA: usize = 0x294C480;
pub const TASK_FORCE_VTABLE_RVA: usize = 0x294C660;
pub const OOB_TYPE_FLEET: u32 = 61;

pub const CSHIP_VTABLE_RVA: usize = 0x2941748;
pub const CSHIP_TYPE: u32 = 51;
pub const CSHIP_OFF_HANDLE: usize = 0x08;
pub const CSHIP_OFF_SUNK_CONVOYS: usize = 0x3C;
pub const CSHIP_OFF_REF100: usize = 0x710;
pub const CSHIP_REF100_RAW: u32 = 10_000_000;

pub const TASK_FORCE_OFF_LOCATION_PROVINCE: usize = 0x18;
pub const TASK_FORCE_OFF_FLEET_PARENT: usize = 0x330;
pub const TASK_FORCE_OFF_SHIP_BEGIN: usize = 0x338;
pub const TASK_FORCE_OFF_SHIP_COUNT: usize = 0x344;

// Wave-4 naval enrichment (validated live 870ebf, paused D04, same-tick melt
// oracle; runs/attemptcrack_naval_missions/EVIDENCE.md — each offset was the
// UNIQUE full-population diagonal in its scanned span):
//   - mission enum: CTaskForce+0x364 i32, 192/192 (values 0,1,2,3,4,8,9).
//   - current-location pointer form CTaskForce+0x1E0 == the already-validated
//     +0x18 province id (not separately emitted; +0x18 is the direct read).
//   - repair_target: CTaskForce+0x4B0 CProvince* (nullable; 3/3 set, 189/189
//     null) -> province id at CProvince+0xA4.
//   - home_base: CFleet+0xD8 CProvince* (119/119) -> province id at +0xA4.
pub const TASK_FORCE_OFF_MISSION_ENUM: usize = 0x364;
pub const TASK_FORCE_OFF_REPAIR_TARGET: usize = 0x4B0;
pub const FLEET_OFF_HOME_BASE: usize = 0xD8;
pub const CPROVINCE_VTABLE_RVA: usize = 0x295AF68;
pub const CPROVINCE_OFF_ID: usize = 0xA4;
// E10 / R6 naval supremacy (validated live 870ebf, paused D04, same-tick melt
// oracle): scan CStrategicRegion objects, walk the per-region
// CNavalRegionDominance container, and emit CDominanceValues rows. The
// 870ebf field order is target@+0x30 and base_target@+0x28; the older python
// prototype had those two names reversed for this build.
pub const STRATEGIC_REGION_VTABLE_RVA: usize = 0x29569A8;
pub const STRATEGIC_REGION_SECONDARY_VTABLE_RVA: usize = 0x29569F8;
pub const STRATEGIC_REGION_OFF_SECONDARY_VTABLE: usize = 0x08;
pub const STRATEGIC_REGION_OFF_ID: usize = 0x58;
pub const STRATEGIC_REGION_OFF_NAVAL_DOMINANCE: usize = 0xE8;
pub const NAVAL_REGION_DOMINANCE_VTABLE_RVA: usize = 0x296E010;
pub const NAVAL_REGION_DOMINANCE_OFF_BEGIN: usize = 0x40;
pub const NAVAL_REGION_DOMINANCE_OFF_COUNT: usize = 0x48;
pub const DOMINANCE_VALUES_VTABLE_RVA: usize = 0x296DFC0;
pub const DOMINANCE_VALUES_STRIDE: usize = 0x50;
pub const DOMINANCE_VALUES_OFF_TAG: usize = 0x00;
pub const DOMINANCE_VALUES_OFF_VTABLE: usize = 0x08;
pub const DOMINANCE_VALUES_OFF_TAG_DUP: usize = 0x10;
pub const DOMINANCE_VALUES_OFF_CURRENT: usize = 0x20;
pub const DOMINANCE_VALUES_OFF_BASE_TARGET: usize = 0x28;
pub const DOMINANCE_VALUES_OFF_TARGET: usize = 0x30;
pub const DOMINANCE_VALUES_OFF_INDIVIDUAL_RATIO: usize = 0x38;
pub const DOMINANCE_VALUES_OFF_PREVIOUS: usize = 0x40;
pub const DOMINANCE_VALUES_OFF_DECLINE_FROM: usize = 0x48;
pub const MAX_STRATEGIC_REGIONS: usize = 4096;
pub const MAX_NAVAL_DOMINANCE_ENTRIES_PER_REGION: u32 = 1024;
/// The collector emits a mission enum only when the raw value is within the
/// PROVEN naval-mission range. D04 exercised {0,1,2,3,4,8,9} (max 9); the
/// naval mission enum is a small contiguous game enum, so this is the
/// fail-closed acceptance bound — a value beyond it (or a negative-i32
/// sentinel read as a huge u32) is badged ABSENT rather than emitted as a
/// plausible-but-unproven mission. The wire/gate cap (MAX_MISSION_ENUM=64) is
/// deliberately looser headroom; the collector is the strict layer.
pub const MAX_OBSERVED_MISSION_ENUM: u32 = 9;

pub const COMPACT_FLEET_SCAN_SPAN: usize = 0x1800;
pub const TASK_FORCE_SUBOBJECT_TO_BODY_DELTAS: [i64; 4] = [-0x780, 0x10, 0x7A0, 0xF30];

pub const HEAP_LO: u64 = 0x10000000000;
pub const HEAP_HI: u64 = 0x7FF000000000;
pub const MAX_COUNTRY_TABLE_SLOTS: u32 = 4096;
pub const MAX_COUNTRY_FLEET_ROWS: u32 = 4096;
pub const MAX_FLEET_POINTER_VECTOR_ROWS: u32 = 20_000;
pub const MAX_FLEET_POINTER_VECTOR_CAPACITY: u32 = 40_000;
pub const MAX_OOB_TASK_FORCE_SHIPS: u32 = 20_000;
pub const MAX_OOB_ID: u32 = 4_000_000;
pub const MAX_SHIP_INSTANCE_ID: u32 = 4_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtractError {
    BuildMismatch {
        expected: &'static str,
        actual: String,
    },
    MemoryRead {
        address: u64,
        len: usize,
    },
    InvalidRoot(&'static str),
    InvalidCount {
        name: &'static str,
        value: u32,
    },
}

pub trait NavalMissionMemory {
    fn module_base(&self) -> u64;
    fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError>;

    fn province_to_region(&self, _province_id: u32) -> Option<u32> {
        None
    }

    fn scan_qword_hits(&self, _needle: u64) -> Result<Vec<u64>, ExtractError> {
        Err(ExtractError::InvalidRoot("scan_qword_hits"))
    }

    fn read_u64(&self, address: u64) -> Result<u64, ExtractError> {
        let bytes = self.read_bytes(address, 8)?;
        Ok(u64_from(&bytes, 0))
    }

    fn read_u32(&self, address: u64) -> Result<u32, ExtractError> {
        let bytes = self.read_bytes(address, 4)?;
        Ok(u32_from(&bytes, 0))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FleetRoot {
    pub fleet_id: u32,
    pub body_ptr: u64,
    pub owner_tag_db_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskForceRoot {
    pub task_force_id: u32,
    pub body_ptr: u64,
    pub fleet_id: u32,
    pub fleet_ptr: u64,
    pub owner_tag_db_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalMissionRoots {
    pub game_state: u64,
    pub country_table: u64,
    pub country_count: u32,
    pub fleets: Vec<FleetRoot>,
    pub task_forces: Vec<TaskForceRoot>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavalRootCounters {
    pub game_state_valid: u32,
    pub country_count: u32,
    pub valid_country_count: u32,
    pub readable_fleet_vector_count: u32,
    pub fleet_count: u32,
    pub task_force_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedRows {
    pub rows: Vec<compact::NavalMissionStatusRow>,
    pub naval_supremacy_rows: Vec<compact::NavalSupremacyRow>,
    pub observed_task_forces: u32,
    pub invalid_task_force_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedFrame {
    pub payload: Vec<u8>,
    pub header: compact::NavalMissionStatusHeader,
    pub rows: Vec<compact::NavalMissionStatusRow>,
    pub naval_supremacy_rows: Vec<compact::NavalSupremacyRow>,
}

#[derive(Clone, Debug, Default)]
pub struct CompactNavalMissionStatusTracker {
    last_rows: BTreeMap<u32, compact::NavalMissionStatusRow>,
}

impl CompactNavalMissionStatusTracker {
    pub fn encode_frame(
        &mut self,
        elapsed_wall_ms: u64,
        status_flags: u32,
        observed_task_forces: u32,
        rows: &[compact::NavalMissionStatusRow],
    ) -> EncodedFrame {
        self.encode_frame_with_naval_supremacy(
            elapsed_wall_ms,
            status_flags,
            observed_task_forces,
            rows,
            &[],
        )
    }

    pub fn encode_frame_with_naval_supremacy(
        &mut self,
        elapsed_wall_ms: u64,
        status_flags: u32,
        observed_task_forces: u32,
        rows: &[compact::NavalMissionStatusRow],
        naval_supremacy_rows: &[compact::NavalSupremacyRow],
    ) -> EncodedFrame {
        let changed_rows = rows
            .iter()
            .filter(|row| self.last_rows.get(&row.task_force_id) != Some(row))
            .count()
            .min(u32::MAX as usize) as u32;
        self.last_rows = rows
            .iter()
            .map(|row| (row.task_force_id, row.clone()))
            .collect();
        let header = build_header(
            elapsed_wall_ms,
            status_flags,
            changed_rows,
            observed_task_forces,
            rows,
        );
        let payload =
            compact::encode_payload_with_naval_supremacy(&header, rows, naval_supremacy_rows);
        EncodedFrame {
            payload,
            header,
            rows: rows.to_vec(),
            naval_supremacy_rows: naval_supremacy_rows.to_vec(),
        }
    }
}

pub fn validate_build(sha256: &str) -> Result<(), ExtractError> {
    if sha256 == EXPECTED_SHA256 {
        Ok(())
    } else {
        Err(ExtractError::BuildMismatch {
            expected: EXPECTED_SHA256,
            actual: sha256.to_string(),
        })
    }
}

pub fn resolve_naval_roots<M: NavalMissionMemory>(
    mem: &M,
) -> Result<NavalMissionRoots, ExtractError> {
    let (result, _) = resolve_naval_roots_with_counters(mem);
    result
}

pub fn resolve_naval_roots_with_counters<M: NavalMissionMemory>(
    mem: &M,
) -> (Result<NavalMissionRoots, ExtractError>, NavalRootCounters) {
    let mut counters = NavalRootCounters::default();
    let module_base = mem.module_base();
    let game_state = match mem
        .read_u64(module_base + compact_contract::build_profile::CG_STATE_DATE_OFFSET as u64)
    {
        Ok(game_state) => game_state,
        Err(err) => return (Err(err), counters),
    };
    if !is_heap_aligned(game_state) {
        return (Err(ExtractError::InvalidRoot("CGameState")), counters);
    }
    counters.game_state_valid = 1;
    let country_table = match mem.read_u64(game_state + CGAMESTATE_COUNTRY_TABLE as u64) {
        Ok(country_table) => country_table,
        Err(err) => return (Err(err), counters),
    };
    let country_count = match mem.read_u32(game_state + CGAMESTATE_COUNTRY_COUNT as u64) {
        Ok(country_count) => country_count,
        Err(err) => return (Err(err), counters),
    };
    counters.country_count = country_count;
    if !is_heap_aligned(country_table)
        || country_count == 0
        || country_count > MAX_COUNTRY_TABLE_SLOTS
    {
        return (
            Err(ExtractError::InvalidCount {
                name: "country_count",
                value: country_count,
            }),
            counters,
        );
    }
    let table_raw = match mem.read_bytes(country_table, country_count as usize * 8) {
        Ok(table_raw) => table_raw,
        Err(err) => return (Err(err), counters),
    };
    let mut fleets_by_ptr = BTreeMap::new();
    for tag_db_id in 1..country_count {
        let country_ptr = u64_from(&table_raw, tag_db_id as usize * 8);
        let Some(owner) = validate_country(mem, module_base, country_ptr, tag_db_id) else {
            continue;
        };
        counters.valid_country_count = counters.valid_country_count.saturating_add(1);
        match collect_country_fleet_roots(mem, module_base, country_ptr, owner, &mut fleets_by_ptr)
        {
            Ok(readable_vectors) => {
                counters.readable_fleet_vector_count = counters
                    .readable_fleet_vector_count
                    .saturating_add(readable_vectors);
            }
            Err(err) => return (Err(err), counters),
        }
    }
    counters.fleet_count = fleets_by_ptr.len().min(u32::MAX as usize) as u32;

    let mut task_forces_by_ptr = BTreeMap::new();
    for fleet in fleets_by_ptr.values() {
        collect_task_force_roots_from_fleet(
            mem,
            module_base,
            fleet.body_ptr,
            &fleets_by_ptr,
            &mut task_forces_by_ptr,
        );
    }
    counters.task_force_count = task_forces_by_ptr.len().min(u32::MAX as usize) as u32;

    (
        Ok(NavalMissionRoots {
            game_state,
            country_table,
            country_count,
            fleets: fleets_by_ptr.values().cloned().collect(),
            task_forces: task_forces_by_ptr.values().cloned().collect(),
        }),
        counters,
    )
}

pub fn extract_rows_from_roots<M: NavalMissionMemory>(
    mem: &M,
    roots: &NavalMissionRoots,
) -> Result<ExtractedRows, ExtractError> {
    let mut rows = Vec::with_capacity(roots.task_forces.len());
    let mut invalid_task_force_rows = 0u32;
    for task_force in &roots.task_forces {
        match read_task_force_row(mem, task_force) {
            Ok(row) => rows.push(row),
            Err(_) => invalid_task_force_rows = invalid_task_force_rows.saturating_add(1),
        }
    }
    let naval_supremacy_rows = read_naval_supremacy_rows(mem)?.unwrap_or_default();
    Ok(ExtractedRows {
        rows,
        naval_supremacy_rows,
        observed_task_forces: roots.task_forces.len().min(u32::MAX as usize) as u32,
        invalid_task_force_rows,
    })
}

pub fn encode_rows(
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_task_forces: u32,
    rows: &[compact::NavalMissionStatusRow],
) -> EncodedFrame {
    encode_rows_with_naval_supremacy(
        elapsed_wall_ms,
        status_flags,
        observed_task_forces,
        rows,
        &[],
    )
}

pub fn encode_rows_with_naval_supremacy(
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_task_forces: u32,
    rows: &[compact::NavalMissionStatusRow],
    naval_supremacy_rows: &[compact::NavalSupremacyRow],
) -> EncodedFrame {
    let header = build_header(
        elapsed_wall_ms,
        status_flags,
        rows.len().min(u32::MAX as usize) as u32,
        observed_task_forces,
        rows,
    );
    let payload = compact::encode_payload_with_naval_supremacy(&header, rows, naval_supremacy_rows);
    EncodedFrame {
        payload,
        header,
        rows: rows.to_vec(),
        naval_supremacy_rows: naval_supremacy_rows.to_vec(),
    }
}

pub fn build_header(
    elapsed_wall_ms: u64,
    status_flags: u32,
    changed_rows: u32,
    observed_task_forces: u32,
    rows: &[compact::NavalMissionStatusRow],
) -> compact::NavalMissionStatusHeader {
    compact::NavalMissionStatusHeader {
        elapsed_wall_ms,
        changed_rows,
        status_flags,
        observed_task_forces,
        validated_location_rows: count_rows(rows, compact::FLAG_HAS_LOCATION),
        derived_region_rows: count_rows(rows, compact::FLAG_HAS_REGION),
        fixture_required_rows: rows
            .iter()
            .filter(|row| {
                row.flags
                    & (compact::FLAG_FIXTURE_REQUIRED
                        | compact::FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED)
                    != 0
            })
            .count()
            .min(u32::MAX as usize) as u32,
        convoy_sunk_count_rows: count_rows(rows, compact::FLAG_HAS_SUNK_CONVOY_COUNT),
    }
}

fn read_task_force_row<M: NavalMissionMemory>(
    mem: &M,
    task_force: &TaskForceRoot,
) -> Result<compact::NavalMissionStatusRow, ExtractError> {
    if task_force.owner_tag_db_id > compact::MAX_OWNER_TAG_DB_ID {
        return Err(ExtractError::InvalidCount {
            name: "owner_tag_db_id",
            value: task_force.owner_tag_db_id,
        });
    }
    let module_base = mem.module_base();
    // Wave-4: mission enum, home base, and repair target are now directly
    // read (validated), so the mission/home-base/repair fixture-required
    // badges are dropped. Retreat has no field in the save schema (documented
    // absence, not a fixture gap). Convoy-COMBAT context remains fixture-
    // blocked (needs live combat) and is set below when sunk convoys appear.
    let mut flags = compact::FLAG_VALIDATED_DIRECT;
    let mut source_status = 0u32;
    let mut location_province_id = 0u32;
    let location = mem.read_u32(task_force.body_ptr + TASK_FORCE_OFF_LOCATION_PROVINCE as u64)?;
    if (1..=compact::MAX_PROVINCE_ID).contains(&location) {
        flags |= compact::FLAG_HAS_LOCATION;
        location_province_id = location;
    }
    let mut strategic_region_id = 0u32;
    if location_province_id != 0 {
        if let Some(region_id) = mem.province_to_region(location_province_id) {
            if (1..=compact::MAX_REGION_ID).contains(&region_id) {
                flags |= compact::FLAG_HAS_REGION | compact::FLAG_DERIVED_REGION;
                strategic_region_id = region_id;
            }
        }
    }
    // Mission enum (CTaskForce+0x364 i32). Only the PROVEN range is emitted
    // (<= MAX_OBSERVED_MISSION_ENUM); an unproven/out-of-range value or a
    // negative-i32 sentinel (read as a huge u32) is badged absent (absent != 0).
    let mut mission_enum = 0u32;
    let mission_raw = mem.read_u32(task_force.body_ptr + TASK_FORCE_OFF_MISSION_ENUM as u64)?;
    if mission_raw <= MAX_OBSERVED_MISSION_ENUM {
        flags |= compact::FLAG_HAS_MISSION;
        mission_enum = mission_raw;
    }
    // Home base (CFleet+0xD8 -> CProvince* -> province id at +0xA4). Absent
    // when the pointer is null/invalid.
    let mut home_base_province_id = 0u32;
    if let Some(province_id) = read_cprovince_id(
        mem,
        module_base,
        task_force.fleet_ptr + FLEET_OFF_HOME_BASE as u64,
    )? {
        flags |= compact::FLAG_HAS_HOME_BASE;
        home_base_province_id = province_id;
    }
    // Repair target (CTaskForce+0x4B0 -> CProvince*). Nullable (only set while
    // repairing at a specific base); absent otherwise.
    let mut repair_base_province_id = 0u32;
    if let Some(province_id) = read_cprovince_id(
        mem,
        module_base,
        task_force.body_ptr + TASK_FORCE_OFF_REPAIR_TARGET as u64,
    )? {
        flags |= compact::FLAG_HAS_REPAIR_BASE;
        repair_base_province_id = province_id;
    }
    let sunk_convoys = aggregate_sunk_convoys(mem, task_force.body_ptr)?;
    if sunk_convoys > 0 {
        flags |= compact::FLAG_HAS_SUNK_CONVOY_COUNT | compact::FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED;
        source_status |= compact::STATUS_CONVOY_CONTEXT_FIXTURE_REQUIRED;
    }
    Ok(compact::NavalMissionStatusRow {
        task_force_id: task_force.task_force_id,
        fleet_id: task_force.fleet_id,
        owner_tag_db_id: task_force.owner_tag_db_id,
        flags,
        mission_enum,
        location_province_id,
        strategic_region_id,
        home_base_province_id,
        repair_base_province_id,
        sunk_convoys,
        source_status,
        ..compact::NavalMissionStatusRow::default()
    })
}

fn read_naval_supremacy_rows<M: NavalMissionMemory>(
    mem: &M,
) -> Result<Option<Vec<compact::NavalSupremacyRow>>, ExtractError> {
    let module_base = mem.module_base();
    let region_vtable = module_base + STRATEGIC_REGION_VTABLE_RVA as u64;
    let region_secondary_vtable = module_base + STRATEGIC_REGION_SECONDARY_VTABLE_RVA as u64;
    let container_vtable = module_base + NAVAL_REGION_DOMINANCE_VTABLE_RVA as u64;
    let entry_vtable = module_base + DOMINANCE_VALUES_VTABLE_RVA as u64;
    let hits = match mem.scan_qword_hits(region_vtable) {
        Ok(hits) => hits,
        Err(_) => return Ok(None),
    };
    if hits.is_empty() || hits.len() > MAX_STRATEGIC_REGIONS {
        return Ok(None);
    }

    let mut regions_by_id = BTreeMap::new();
    for region_ptr in hits {
        if mem.read_u64(region_ptr + STRATEGIC_REGION_OFF_SECONDARY_VTABLE as u64)
            != Ok(region_secondary_vtable)
        {
            continue;
        }
        let Ok(region_id) = mem.read_u32(region_ptr + STRATEGIC_REGION_OFF_ID as u64) else {
            return Ok(None);
        };
        if !(1..=compact::MAX_REGION_ID).contains(&region_id) {
            continue;
        }
        regions_by_id.entry(region_id).or_insert(region_ptr);
    }
    if regions_by_id.is_empty() || regions_by_id.len() > MAX_STRATEGIC_REGIONS {
        return Ok(None);
    }

    let mut rows = Vec::new();
    for (region_id, region_ptr) in regions_by_id {
        let container_ptr =
            match mem.read_u64(region_ptr + STRATEGIC_REGION_OFF_NAVAL_DOMINANCE as u64) {
                Ok(container_ptr) => container_ptr,
                Err(_) => return Ok(None),
            };
        if !is_heap_aligned(container_ptr) {
            continue;
        }
        if mem.read_u64(container_ptr)? != container_vtable {
            return Ok(None);
        }
        let begin = mem.read_u64(container_ptr + NAVAL_REGION_DOMINANCE_OFF_BEGIN as u64)?;
        let count = mem.read_u32(container_ptr + NAVAL_REGION_DOMINANCE_OFF_COUNT as u64)?;
        if count == 0 {
            continue;
        }
        if count > MAX_NAVAL_DOMINANCE_ENTRIES_PER_REGION || !is_heap_aligned(begin) {
            return Ok(None);
        }
        let byte_len = (count as usize)
            .checked_mul(DOMINANCE_VALUES_STRIDE)
            .ok_or(ExtractError::InvalidCount {
                name: "naval_dominance_entry_bytes",
                value: count,
            })?;
        let raw = match mem.read_bytes(begin, byte_len) {
            Ok(raw) if raw.len() == byte_len => raw,
            _ => return Ok(None),
        };
        let mut staged = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            let cursor = index * DOMINANCE_VALUES_STRIDE;
            if u64_from(&raw, cursor + DOMINANCE_VALUES_OFF_VTABLE) != entry_vtable {
                return Ok(None);
            }
            let controller_tag_db_id = u32_from(&raw, cursor + DOMINANCE_VALUES_OFF_TAG);
            let controller_tag_db_id_dup = u32_from(&raw, cursor + DOMINANCE_VALUES_OFF_TAG_DUP);
            if controller_tag_db_id == 0
                || controller_tag_db_id > compact::MAX_OWNER_TAG_DB_ID
                || controller_tag_db_id != controller_tag_db_id_dup
            {
                return Ok(None);
            }
            let row = compact::NavalSupremacyRow {
                region_id,
                controller_tag_db_id,
                current_raw: u32_from(&raw, cursor + DOMINANCE_VALUES_OFF_CURRENT),
                target_raw: u32_from(&raw, cursor + DOMINANCE_VALUES_OFF_TARGET),
                base_target_raw: u32_from(&raw, cursor + DOMINANCE_VALUES_OFF_BASE_TARGET),
                previous_raw: u32_from(&raw, cursor + DOMINANCE_VALUES_OFF_PREVIOUS),
                decline_from_raw: u32_from(&raw, cursor + DOMINANCE_VALUES_OFF_DECLINE_FROM),
                individual_ratio_raw: u32_from(
                    &raw,
                    cursor + DOMINANCE_VALUES_OFF_INDIVIDUAL_RATIO,
                ),
            };
            if row.individual_ratio_raw > compact::NAVAL_SUPREMACY_FIXED_SCALE_RAW {
                return Ok(None);
            }
            staged.push(row);
        }
        if mem.read_u64(container_ptr + NAVAL_REGION_DOMINANCE_OFF_BEGIN as u64)? != begin
            || mem.read_u32(container_ptr + NAVAL_REGION_DOMINANCE_OFF_COUNT as u64)? != count
        {
            return Ok(None);
        }
        rows.extend(staged);
    }
    rows.sort_by_key(|row| (row.region_id, row.controller_tag_db_id));
    Ok(Some(rows))
}

/// Read a CProvince* at `pointer_addr`, vtable-gate it as a CProvince, and
/// return its province id (CProvince+0xA4) when valid. Returns `Ok(None)` for
/// a null/invalid pointer (nullable field). A memory-read failure on a
/// present pointer propagates as `Err` so the whole row is dropped fail-closed.
///
/// Hardening (Codex DEFECT B): the pointer + id are read with a STABLE double
/// read. On an unpaused game a stale/being-freed pointer can flap; if the two
/// reads of the pointer OR the id disagree the field is badged ABSENT (not
/// emitted as a plausible-but-wrong province). A stably-reused address that
/// still carries the CProvince vtable and an in-range id remains a documented
/// residual shared by every CProvince read in the codebase (the established
/// vtable+id gate); the paused capture is the current proof.
fn read_cprovince_id<M: NavalMissionMemory>(
    mem: &M,
    module_base: u64,
    pointer_addr: u64,
) -> Result<Option<u32>, ExtractError> {
    let province_ptr = mem.read_u64(pointer_addr)?;
    if province_ptr != mem.read_u64(pointer_addr)? {
        // The pointer flapped mid-read: torn, badge absent.
        return Ok(None);
    }
    if province_ptr == 0 || !is_heap_aligned(province_ptr) {
        return Ok(None);
    }
    if mem.read_u64(province_ptr)? != module_base + CPROVINCE_VTABLE_RVA as u64 {
        return Ok(None);
    }
    let province_id = mem.read_u32(province_ptr + CPROVINCE_OFF_ID as u64)?;
    if province_id != mem.read_u32(province_ptr + CPROVINCE_OFF_ID as u64)? {
        // The id flapped mid-read: torn, badge absent.
        return Ok(None);
    }
    if (1..=compact::MAX_PROVINCE_ID).contains(&province_id) {
        Ok(Some(province_id))
    } else {
        Ok(None)
    }
}

fn aggregate_sunk_convoys<M: NavalMissionMemory>(
    mem: &M,
    task_force_body: u64,
) -> Result<u32, ExtractError> {
    let count = mem.read_u32(task_force_body + TASK_FORCE_OFF_SHIP_COUNT as u64)?;
    if count == 0 || count > MAX_OOB_TASK_FORCE_SHIPS {
        return Ok(0);
    }
    let begin = mem.read_u64(task_force_body + TASK_FORCE_OFF_SHIP_BEGIN as u64)?;
    if !is_heap_aligned(begin) {
        return Ok(0);
    }
    let raw = mem.read_bytes(begin, count as usize * 8)?;
    let mut total = 0u32;
    let module_base = mem.module_base();
    for index in 0..count as usize {
        let ship_ptr = u64_from(&raw, index * 8);
        if read_cship_instance_id(mem, module_base, ship_ptr).is_some() {
            let sunk = mem
                .read_u32(ship_ptr + CSHIP_OFF_SUNK_CONVOYS as u64)
                .unwrap_or(0);
            total = total.saturating_add(sunk);
        }
    }
    Ok(total)
}

fn validate_country<M: NavalMissionMemory>(
    mem: &M,
    module_base: u64,
    country_ptr: u64,
    slot_id: u32,
) -> Option<u32> {
    if !is_heap_aligned(country_ptr) {
        return None;
    }
    if mem.read_u64(country_ptr).ok()? != module_base + COUNTRY_VTABLE_RVA as u64 {
        return None;
    }
    let tag_db_id = mem.read_u32(country_ptr + OBJ_TAG_DB_ID as u64).ok()?;
    if tag_db_id == 0 || tag_db_id != slot_id {
        return None;
    }
    Some(tag_db_id)
}

fn collect_country_fleet_roots<M: NavalMissionMemory>(
    mem: &M,
    module_base: u64,
    country_ptr: u64,
    owner_tag_db_id: u32,
    fleets_by_ptr: &mut BTreeMap<u64, FleetRoot>,
) -> Result<u32, ExtractError> {
    let vector_data = mem.read_u64(country_ptr + COUNTRY_FLEET_VECTOR_DATA as u64)?;
    let capacity = mem.read_u32(country_ptr + COUNTRY_FLEET_VECTOR_CAPACITY as u64)?;
    let count = mem.read_u32(country_ptr + COUNTRY_FLEET_VECTOR_COUNT as u64)?;
    if count == 0
        || count > capacity
        || count > MAX_COUNTRY_FLEET_ROWS
        || !is_heap_aligned(vector_data)
    {
        return Ok(0);
    }
    let raw = mem.read_bytes(vector_data, count as usize * 8)?;
    for index in 0..count as usize {
        let fleet_ptr = u64_from(&raw, index * 8);
        let Some(fleet_id) = validate_oob_body(
            mem,
            module_base,
            fleet_ptr,
            FLEET_VTABLE_RVA,
            OOB_TYPE_FLEET,
        ) else {
            continue;
        };
        fleets_by_ptr.entry(fleet_ptr).or_insert(FleetRoot {
            fleet_id,
            body_ptr: fleet_ptr,
            owner_tag_db_id,
        });
    }
    Ok(1)
}

fn collect_task_force_roots_from_fleet<M: NavalMissionMemory>(
    mem: &M,
    module_base: u64,
    fleet_ptr: u64,
    fleets_by_ptr: &BTreeMap<u64, FleetRoot>,
    task_forces_by_ptr: &mut BTreeMap<u64, TaskForceRoot>,
) {
    let Ok(blob) = mem.read_bytes(fleet_ptr, COMPACT_FLEET_SCAN_SPAN) else {
        return;
    };
    // 2026-07-03 live probe provenance: 436 countries, 154 fleets, 176 task
    // forces; it validated CTaskForce ship vectors at +0x338/+0x344 through
    // the OOB path. Fleet-body vector candidates are speculative, so unreadable
    // candidates must not abort valid sibling task-force roots in this lane.
    for offset in (0..blob.len().saturating_sub(8)).step_by(8) {
        let candidate = u64_from(&blob, offset);
        maybe_insert_task_force(
            mem,
            module_base,
            candidate,
            fleets_by_ptr,
            task_forces_by_ptr,
        );
    }
    for offset in (0..blob.len().saturating_sub(16)).step_by(8) {
        let vector_data = u64_from(&blob, offset);
        let capacity = u32_from(&blob, offset + 8);
        let count = u32_from(&blob, offset + 12);
        if count == 0
            || count > capacity
            || count > MAX_FLEET_POINTER_VECTOR_ROWS
            || capacity > MAX_FLEET_POINTER_VECTOR_CAPACITY
            || !is_heap_aligned(vector_data)
        {
            continue;
        }
        let Ok(raw) = mem.read_bytes(vector_data, count as usize * 8) else {
            continue;
        };
        for index in 0..count as usize {
            let candidate = u64_from(&raw, index * 8);
            if maybe_insert_task_force(
                mem,
                module_base,
                candidate,
                fleets_by_ptr,
                task_forces_by_ptr,
            ) {
                continue;
            }
            if !is_heap_aligned(candidate)
                || mem.read_u64(candidate).ok()
                    != Some(module_base + TASK_FORCE_SUBOBJECT_VTABLE_RVA as u64)
            {
                continue;
            }
            for delta in TASK_FORCE_SUBOBJECT_TO_BODY_DELTAS {
                let body_ptr = if delta < 0 {
                    candidate.checked_sub(delta.unsigned_abs())
                } else {
                    candidate.checked_add(delta as u64)
                };
                let Some(body_ptr) = body_ptr else {
                    continue;
                };
                if maybe_insert_task_force(
                    mem,
                    module_base,
                    body_ptr,
                    fleets_by_ptr,
                    task_forces_by_ptr,
                ) {
                    break;
                }
            }
        }
    }
}

fn maybe_insert_task_force<M: NavalMissionMemory>(
    mem: &M,
    module_base: u64,
    body_ptr: u64,
    fleets_by_ptr: &BTreeMap<u64, FleetRoot>,
    task_forces_by_ptr: &mut BTreeMap<u64, TaskForceRoot>,
) -> bool {
    let Some((task_force_id, fleet_root)) =
        validate_task_force_body(mem, module_base, body_ptr, fleets_by_ptr)
    else {
        return false;
    };
    task_forces_by_ptr.entry(body_ptr).or_insert(TaskForceRoot {
        task_force_id,
        body_ptr,
        fleet_id: fleet_root.fleet_id,
        fleet_ptr: fleet_root.body_ptr,
        owner_tag_db_id: fleet_root.owner_tag_db_id,
    });
    true
}

fn validate_task_force_body<'a, M: NavalMissionMemory>(
    mem: &M,
    module_base: u64,
    body_ptr: u64,
    fleets_by_ptr: &'a BTreeMap<u64, FleetRoot>,
) -> Option<(u32, &'a FleetRoot)> {
    let task_force_id = validate_oob_body(
        mem,
        module_base,
        body_ptr,
        TASK_FORCE_VTABLE_RVA,
        OOB_TYPE_FLEET,
    )?;
    let parent_ptr = mem
        .read_u64(body_ptr + TASK_FORCE_OFF_FLEET_PARENT as u64)
        .ok()?;
    let fleet_root = fleets_by_ptr.get(&parent_ptr)?;
    let count = mem
        .read_u32(body_ptr + TASK_FORCE_OFF_SHIP_COUNT as u64)
        .ok()?;
    if count > MAX_OOB_TASK_FORCE_SHIPS {
        return None;
    }
    if count > 0 {
        let begin = mem
            .read_u64(body_ptr + TASK_FORCE_OFF_SHIP_BEGIN as u64)
            .ok()?;
        if !is_heap_aligned(begin) || !validate_task_force_ship_vector(mem, module_base, body_ptr) {
            return None;
        }
    }
    Some((task_force_id, fleet_root))
}

fn validate_task_force_ship_vector<M: NavalMissionMemory>(
    mem: &M,
    module_base: u64,
    task_force_body: u64,
) -> bool {
    let Ok(count) = mem.read_u32(task_force_body + TASK_FORCE_OFF_SHIP_COUNT as u64) else {
        return false;
    };
    if count == 0 {
        return true;
    }
    if count > MAX_OOB_TASK_FORCE_SHIPS {
        return false;
    }
    let Ok(begin) = mem.read_u64(task_force_body + TASK_FORCE_OFF_SHIP_BEGIN as u64) else {
        return false;
    };
    if !is_heap_aligned(begin) {
        return false;
    }
    let Ok(raw) = mem.read_bytes(begin, count as usize * 8) else {
        return false;
    };
    for index in 0..count as usize {
        let ship_ptr = u64_from(&raw, index * 8);
        if read_cship_instance_id(mem, module_base, ship_ptr).is_none() {
            return false;
        }
    }
    true
}

fn read_cship_instance_id<M: NavalMissionMemory>(
    mem: &M,
    module_base: u64,
    ship_ptr: u64,
) -> Option<u32> {
    if !is_heap_aligned(ship_ptr) {
        return None;
    }
    if mem.read_u64(ship_ptr).ok()? != module_base + CSHIP_VTABLE_RVA as u64 {
        return None;
    }
    let handle = mem.read_u64(ship_ptr + CSHIP_OFF_HANDLE as u64).ok()?;
    if (handle & 0xFFFF_FFFF) as u32 != CSHIP_TYPE {
        return None;
    }
    let instance_id = ((handle >> 32) & 0xFFFF_FFFF) as u32;
    if instance_id == 0 || instance_id > MAX_SHIP_INSTANCE_ID {
        return None;
    }
    if mem.read_u32(ship_ptr + CSHIP_OFF_REF100 as u64).ok()? != CSHIP_REF100_RAW {
        return None;
    }
    Some(instance_id)
}

fn validate_oob_body<M: NavalMissionMemory>(
    mem: &M,
    module_base: u64,
    body: u64,
    vtable_rva: usize,
    type_tag: u32,
) -> Option<u32> {
    if !is_heap_aligned(body) {
        return None;
    }
    if mem.read_u64(body).ok()? != module_base + vtable_rva as u64 {
        return None;
    }
    let handle = mem.read_u64(body + 0x08).ok()?;
    if (handle & 0xFFFF_FFFF) as u32 != type_tag {
        return None;
    }
    let object_id = ((handle >> 32) & 0xFFFF_FFFF) as u32;
    if object_id == 0 || object_id > MAX_OOB_ID {
        return None;
    }
    Some(object_id)
}

fn count_rows(rows: &[compact::NavalMissionStatusRow], flag: u32) -> u32 {
    rows.iter()
        .filter(|row| row.flags & flag != 0)
        .count()
        .min(u32::MAX as usize) as u32
}

fn is_heap_aligned(value: u64) -> bool {
    (HEAP_LO..HEAP_HI).contains(&value) && value % 8 == 0
}

fn u64_from(data: &[u8], offset: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(bytes)
}

fn u32_from(data: &[u8], offset: usize) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&data[offset..offset + 4]);
    u32::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const MODULE_BASE: u64 = 0x7FF6AAFF0000;
    const GAME_STATE: u64 = 0x22000001000;
    const COUNTRY_TABLE: u64 = 0x22000002000;
    const COUNTRY_1: u64 = 0x22000003000;
    const COUNTRY_2049: u64 = 0x22000003400;
    const FLEET_VECTOR: u64 = 0x22000004000;
    const FLEET_615: u64 = 0x22000005000;
    const TASK_FORCE_617: u64 = 0x22000007000;
    const SHIP_VECTOR: u64 = 0x22000009000;
    const SHIP_9001: u64 = 0x2200000A000;
    const PROVINCE_HOME_BASE: u64 = 0x2200000B000;
    const PROVINCE_REPAIR: u64 = 0x2200000C000;
    const REGION_16: u64 = 0x2200000D000;
    const NAVAL_DOMINANCE_16: u64 = 0x2200000E000;
    const DOMINANCE_VALUES_16: u64 = 0x2200000F000;

    #[derive(Clone, Debug, Default)]
    struct FakeMemory {
        writes: BTreeMap<u64, u8>,
        fail_reads: BTreeSet<u64>,
        scan_hits: BTreeMap<u64, Vec<u64>>,
    }

    impl FakeMemory {
        fn write_bytes(&mut self, address: u64, bytes: &[u8]) {
            for (index, byte) in bytes.iter().enumerate() {
                self.writes.insert(address + index as u64, *byte);
            }
        }

        fn write_u64(&mut self, address: u64, value: u64) {
            self.write_bytes(address, &value.to_le_bytes());
        }

        fn write_u32(&mut self, address: u64, value: u32) {
            self.write_bytes(address, &value.to_le_bytes());
        }

        fn fail_read(&mut self, address: u64) {
            self.fail_reads.insert(address);
        }

        fn add_scan_hit(&mut self, needle: u64, address: u64) {
            self.scan_hits.entry(needle).or_default().push(address);
        }
    }

    impl NavalMissionMemory for FakeMemory {
        fn module_base(&self) -> u64 {
            MODULE_BASE
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            if self.fail_reads.contains(&address) {
                return Err(ExtractError::MemoryRead { address, len });
            }
            Ok((0..len)
                .map(|index| {
                    self.writes
                        .get(&(address + index as u64))
                        .copied()
                        .unwrap_or(0)
                })
                .collect())
        }

        fn province_to_region(&self, province_id: u32) -> Option<u32> {
            (province_id == 12389).then_some(31)
        }

        fn scan_qword_hits(&self, needle: u64) -> Result<Vec<u64>, ExtractError> {
            Ok(self.scan_hits.get(&needle).cloned().unwrap_or_default())
        }
    }

    fn fake_memory() -> FakeMemory {
        let mut mem = FakeMemory::default();
        mem.write_u64(
            MODULE_BASE + compact_contract::build_profile::CG_STATE_DATE_OFFSET as u64,
            GAME_STATE,
        );
        mem.write_u64(GAME_STATE + CGAMESTATE_COUNTRY_TABLE as u64, COUNTRY_TABLE);
        mem.write_u32(GAME_STATE + CGAMESTATE_COUNTRY_COUNT as u64, 2);
        mem.write_u64(COUNTRY_TABLE + 8, COUNTRY_1);

        mem.write_u64(COUNTRY_1, MODULE_BASE + COUNTRY_VTABLE_RVA as u64);
        mem.write_u32(COUNTRY_1 + OBJ_TAG_DB_ID as u64, 1);
        mem.write_u64(COUNTRY_1 + COUNTRY_FLEET_VECTOR_DATA as u64, FLEET_VECTOR);
        mem.write_u32(COUNTRY_1 + COUNTRY_FLEET_VECTOR_CAPACITY as u64, 1);
        mem.write_u32(COUNTRY_1 + COUNTRY_FLEET_VECTOR_COUNT as u64, 1);
        mem.write_u64(FLEET_VECTOR, FLEET_615);

        mem.write_u64(FLEET_615, MODULE_BASE + FLEET_VTABLE_RVA as u64);
        mem.write_u64(FLEET_615 + 0x08, (615u64 << 32) | OOB_TYPE_FLEET as u64);
        mem.write_u64(FLEET_615 + 0x40, TASK_FORCE_617);
        // Wave-4: home base CProvince* on the fleet.
        mem.write_u64(FLEET_615 + FLEET_OFF_HOME_BASE as u64, PROVINCE_HOME_BASE);

        // CProvince objects (vtable-gated, id at +0xA4).
        mem.write_u64(
            PROVINCE_HOME_BASE,
            MODULE_BASE + CPROVINCE_VTABLE_RVA as u64,
        );
        mem.write_u32(PROVINCE_HOME_BASE + CPROVINCE_OFF_ID as u64, 4021);
        mem.write_u64(PROVINCE_REPAIR, MODULE_BASE + CPROVINCE_VTABLE_RVA as u64);
        mem.write_u32(PROVINCE_REPAIR + CPROVINCE_OFF_ID as u64, 5088);

        mem.write_u64(TASK_FORCE_617, MODULE_BASE + TASK_FORCE_VTABLE_RVA as u64);
        mem.write_u64(
            TASK_FORCE_617 + 0x08,
            (617u64 << 32) | OOB_TYPE_FLEET as u64,
        );
        mem.write_u32(
            TASK_FORCE_617 + TASK_FORCE_OFF_LOCATION_PROVINCE as u64,
            12389,
        );
        // Wave-4: mission enum + repair target CProvince*.
        mem.write_u32(TASK_FORCE_617 + TASK_FORCE_OFF_MISSION_ENUM as u64, 3);
        mem.write_u64(
            TASK_FORCE_617 + TASK_FORCE_OFF_REPAIR_TARGET as u64,
            PROVINCE_REPAIR,
        );
        mem.write_u64(
            TASK_FORCE_617 + TASK_FORCE_OFF_FLEET_PARENT as u64,
            FLEET_615,
        );
        mem.write_u64(
            TASK_FORCE_617 + TASK_FORCE_OFF_SHIP_BEGIN as u64,
            SHIP_VECTOR,
        );
        mem.write_u32(TASK_FORCE_617 + TASK_FORCE_OFF_SHIP_COUNT as u64, 1);
        mem.write_u64(SHIP_VECTOR, SHIP_9001);

        mem.write_u64(SHIP_9001, MODULE_BASE + CSHIP_VTABLE_RVA as u64);
        mem.write_u64(
            SHIP_9001 + CSHIP_OFF_HANDLE as u64,
            (9001u64 << 32) | CSHIP_TYPE as u64,
        );
        mem.write_u32(SHIP_9001 + CSHIP_OFF_REF100 as u64, CSHIP_REF100_RAW);
        mem.write_u32(SHIP_9001 + CSHIP_OFF_SUNK_CONVOYS as u64, 2);

        mem.write_u64(REGION_16, MODULE_BASE + STRATEGIC_REGION_VTABLE_RVA as u64);
        mem.write_u64(
            REGION_16 + STRATEGIC_REGION_OFF_SECONDARY_VTABLE as u64,
            MODULE_BASE + STRATEGIC_REGION_SECONDARY_VTABLE_RVA as u64,
        );
        mem.write_u32(REGION_16 + STRATEGIC_REGION_OFF_ID as u64, 16);
        mem.write_u64(
            REGION_16 + STRATEGIC_REGION_OFF_NAVAL_DOMINANCE as u64,
            NAVAL_DOMINANCE_16,
        );
        mem.write_u64(
            NAVAL_DOMINANCE_16,
            MODULE_BASE + NAVAL_REGION_DOMINANCE_VTABLE_RVA as u64,
        );
        mem.write_u64(
            NAVAL_DOMINANCE_16 + NAVAL_REGION_DOMINANCE_OFF_BEGIN as u64,
            DOMINANCE_VALUES_16,
        );
        mem.write_u32(
            NAVAL_DOMINANCE_16 + NAVAL_REGION_DOMINANCE_OFF_COUNT as u64,
            1,
        );
        mem.write_u32(DOMINANCE_VALUES_16 + DOMINANCE_VALUES_OFF_TAG as u64, 1);
        mem.write_u64(
            DOMINANCE_VALUES_16 + DOMINANCE_VALUES_OFF_VTABLE as u64,
            MODULE_BASE + DOMINANCE_VALUES_VTABLE_RVA as u64,
        );
        mem.write_u32(DOMINANCE_VALUES_16 + DOMINANCE_VALUES_OFF_TAG_DUP as u64, 1);
        mem.write_u32(
            DOMINANCE_VALUES_16 + DOMINANCE_VALUES_OFF_CURRENT as u64,
            67_962_800,
        );
        mem.write_u32(
            DOMINANCE_VALUES_16 + DOMINANCE_VALUES_OFF_TARGET as u64,
            67_962_800,
        );
        mem.write_u32(
            DOMINANCE_VALUES_16 + DOMINANCE_VALUES_OFF_BASE_TARGET as u64,
            67_962_800,
        );
        mem.write_u32(
            DOMINANCE_VALUES_16 + DOMINANCE_VALUES_OFF_PREVIOUS as u64,
            67_962_800,
        );
        mem.write_u32(
            DOMINANCE_VALUES_16 + DOMINANCE_VALUES_OFF_DECLINE_FROM as u64,
            67_962_800,
        );
        mem.write_u32(
            DOMINANCE_VALUES_16 + DOMINANCE_VALUES_OFF_INDIVIDUAL_RATIO as u64,
            87_173,
        );
        mem.add_scan_hit(MODULE_BASE + STRATEGIC_REGION_VTABLE_RVA as u64, REGION_16);
        mem
    }

    fn fake_memory_with_owner_slot(owner_tag_db_id: u32) -> FakeMemory {
        let mut mem = fake_memory();
        let country_ptr = if owner_tag_db_id == 1 {
            COUNTRY_1
        } else {
            COUNTRY_2049
        };
        let country_count = owner_tag_db_id.saturating_add(1);
        mem.write_u32(GAME_STATE + CGAMESTATE_COUNTRY_COUNT as u64, country_count);
        if owner_tag_db_id != 1 {
            mem.write_u64(COUNTRY_TABLE + 8, 0);
        }
        mem.write_u64(COUNTRY_TABLE + owner_tag_db_id as u64 * 8, country_ptr);
        mem.write_u64(country_ptr, MODULE_BASE + COUNTRY_VTABLE_RVA as u64);
        mem.write_u32(country_ptr + OBJ_TAG_DB_ID as u64, owner_tag_db_id);
        mem.write_u64(country_ptr + COUNTRY_FLEET_VECTOR_DATA as u64, FLEET_VECTOR);
        mem.write_u32(country_ptr + COUNTRY_FLEET_VECTOR_CAPACITY as u64, 1);
        mem.write_u32(country_ptr + COUNTRY_FLEET_VECTOR_COUNT as u64, 1);
        mem
    }

    #[test]
    fn extracts_validated_task_force_status_rows() {
        let mem = fake_memory();
        let roots = resolve_naval_roots(&mem).unwrap();
        assert_eq!(roots.fleets.len(), 1);
        assert_eq!(roots.task_forces.len(), 1);

        let extracted = extract_rows_from_roots(&mem, &roots).unwrap();
        assert_eq!(extracted.rows.len(), 1);
        assert_eq!(extracted.naval_supremacy_rows.len(), 1);
        assert_eq!(
            extracted.naval_supremacy_rows[0],
            compact::NavalSupremacyRow {
                region_id: 16,
                controller_tag_db_id: 1,
                current_raw: 67_962_800,
                target_raw: 67_962_800,
                base_target_raw: 67_962_800,
                previous_raw: 67_962_800,
                decline_from_raw: 67_962_800,
                individual_ratio_raw: 87_173,
            }
        );
        let row = &extracted.rows[0];
        assert_eq!(row.task_force_id, 617);
        assert_eq!(row.fleet_id, 615);
        assert_eq!(row.owner_tag_db_id, 1);
        assert_eq!(row.location_province_id, 12389);
        assert_eq!(row.strategic_region_id, 31);
        assert_eq!(row.sunk_convoys, 2);
        assert!(row.flags & compact::FLAG_HAS_LOCATION != 0);
        assert!(row.flags & compact::FLAG_HAS_REGION != 0);
        assert!(row.flags & compact::FLAG_HAS_SUNK_CONVOY_COUNT != 0);
        // Wave-4 enrichment: mission enum, home base, repair target.
        assert_eq!(row.mission_enum, 3);
        assert!(row.flags & compact::FLAG_HAS_MISSION != 0);
        assert_eq!(row.home_base_province_id, 4021);
        assert!(row.flags & compact::FLAG_HAS_HOME_BASE != 0);
        assert_eq!(row.repair_base_province_id, 5088);
        assert!(row.flags & compact::FLAG_HAS_REPAIR_BASE != 0);
        // Mission/home-base/repair fixture-required badges are gone.
        assert!(row.flags & compact::FLAG_FIXTURE_REQUIRED == 0);
        assert!(row.source_status & compact::STATUS_MISSION_FIXTURE_REQUIRED == 0);
        assert!(row.source_status & compact::STATUS_HOME_BASE_FIXTURE_REQUIRED == 0);
    }

    #[test]
    fn encodes_extracted_naval_supremacy_trailer() {
        let mem = fake_memory();
        let roots = resolve_naval_roots(&mem).unwrap();
        let extracted = extract_rows_from_roots(&mem, &roots).unwrap();

        let frame = encode_rows_with_naval_supremacy(
            1,
            0,
            extracted.observed_task_forces,
            &extracted.rows,
            &extracted.naval_supremacy_rows,
        );
        let decoded = compact::decode_status_payload(0, &frame.payload).unwrap();
        assert_eq!(decoded.rows.len(), 1);
        assert_eq!(decoded.naval_supremacy_rows, extracted.naval_supremacy_rows);
        assert_eq!(frame.naval_supremacy_rows.len(), 1);
    }

    #[test]
    fn mission_enum_zero_is_a_valid_present_value_not_a_panic() {
        // Mission enum 0 is the "hold" mission; it must encode as present
        // (FLAG_HAS_MISSION set, value 0) and survive the encode validator,
        // which previously rejected 0-when-present and panicked the worker.
        let mut mem = fake_memory();
        mem.write_u32(TASK_FORCE_617 + TASK_FORCE_OFF_MISSION_ENUM as u64, 0);

        let roots = resolve_naval_roots(&mem).unwrap();
        let extracted = extract_rows_from_roots(&mem, &roots).unwrap();
        let row = &extracted.rows[0];
        assert_eq!(row.mission_enum, 0);
        assert!(row.flags & compact::FLAG_HAS_MISSION != 0);

        // Full encode + decode round-trip must not panic or error.
        let frame = encode_rows(1, 0, extracted.observed_task_forces, &extracted.rows);
        assert!(compact::decode_status_payload(0, &frame.payload).is_ok());
    }

    #[test]
    fn out_of_range_mission_enum_badges_absent() {
        // A negative i32 sentinel (read as a huge u32) or an out-of-cap value
        // badges the mission absent rather than emitting garbage.
        let mut mem = fake_memory();
        mem.write_u32(
            TASK_FORCE_617 + TASK_FORCE_OFF_MISSION_ENUM as u64,
            0xFFFF_FFFF,
        );

        let roots = resolve_naval_roots(&mem).unwrap();
        let extracted = extract_rows_from_roots(&mem, &roots).unwrap();
        let row = &extracted.rows[0];
        assert_eq!(row.flags & compact::FLAG_HAS_MISSION, 0);
        assert_eq!(row.mission_enum, 0);
    }

    #[test]
    fn unproven_mission_enum_above_observed_max_badges_absent() {
        // A value within the loose wire cap (64) but above the PROVEN max (9)
        // must be badged absent, not emitted as a plausible-but-unproven
        // mission (Codex DEFECT A).
        let mut mem = fake_memory();
        mem.write_u32(
            TASK_FORCE_617 + TASK_FORCE_OFF_MISSION_ENUM as u64,
            MAX_OBSERVED_MISSION_ENUM + 1,
        );

        let roots = resolve_naval_roots(&mem).unwrap();
        let extracted = extract_rows_from_roots(&mem, &roots).unwrap();
        let row = &extracted.rows[0];
        assert_eq!(row.flags & compact::FLAG_HAS_MISSION, 0);
        assert_eq!(row.mission_enum, 0);
    }

    #[test]
    fn nullable_repair_target_and_home_base_badge_absent_not_zero() {
        let mut mem = fake_memory();
        // Null out the repair target pointer and the fleet home base pointer.
        mem.write_u64(TASK_FORCE_617 + TASK_FORCE_OFF_REPAIR_TARGET as u64, 0);
        mem.write_u64(FLEET_615 + FLEET_OFF_HOME_BASE as u64, 0);

        let roots = resolve_naval_roots(&mem).unwrap();
        let extracted = extract_rows_from_roots(&mem, &roots).unwrap();
        let row = &extracted.rows[0];
        // Absent, not zero-emitted: the flags are clear and the fields stay 0.
        assert_eq!(row.flags & compact::FLAG_HAS_REPAIR_BASE, 0);
        assert_eq!(row.flags & compact::FLAG_HAS_HOME_BASE, 0);
        assert_eq!(row.repair_base_province_id, 0);
        assert_eq!(row.home_base_province_id, 0);
        // Mission still present.
        assert!(row.flags & compact::FLAG_HAS_MISSION != 0);
    }

    #[test]
    fn resolver_counters_report_country_validation_drop_stage() {
        let mut mem = fake_memory();
        mem.write_u32(COUNTRY_1 + OBJ_TAG_DB_ID as u64, 99);

        let (roots, counters) = resolve_naval_roots_with_counters(&mem);
        let roots = roots.unwrap();

        assert_eq!(roots.fleets.len(), 0);
        assert_eq!(roots.task_forces.len(), 0);
        assert_eq!(counters.game_state_valid, 1);
        assert_eq!(counters.country_count, 2);
        assert_eq!(counters.valid_country_count, 0);
        assert_eq!(counters.readable_fleet_vector_count, 0);
        assert_eq!(counters.fleet_count, 0);
        assert_eq!(counters.task_force_count, 0);
    }

    #[test]
    fn unreadable_speculative_fleet_vector_does_not_drop_valid_task_force_siblings() {
        let mut mem = fake_memory();
        let bogus_vector = 0x2200000B000;
        mem.write_u64(FLEET_615 + 0x100, bogus_vector);
        mem.write_u32(FLEET_615 + 0x108, 1);
        mem.write_u32(FLEET_615 + 0x10C, 1);
        mem.fail_read(bogus_vector);

        let (roots, counters) = resolve_naval_roots_with_counters(&mem);
        let roots = roots.unwrap();

        assert_eq!(roots.fleets.len(), 1);
        assert_eq!(roots.task_forces.len(), 1);
        assert_eq!(counters.fleet_count, 1);
        assert_eq!(counters.task_force_count, 1);
    }

    #[test]
    fn row_read_failures_are_counted_without_returning_partial_rows() {
        let mut mem = fake_memory();
        mem.fail_read(TASK_FORCE_617 + TASK_FORCE_OFF_LOCATION_PROVINCE as u64);

        let roots = resolve_naval_roots(&mem).unwrap();
        let extracted = extract_rows_from_roots(&mem, &roots).unwrap();

        assert_eq!(extracted.observed_task_forces, 1);
        assert_eq!(extracted.invalid_task_force_rows, 1);
        assert!(extracted.rows.is_empty());
    }

    #[test]
    fn over_cap_owner_row_is_dropped_and_counted_before_encoding() {
        let mem = fake_memory_with_owner_slot(compact::MAX_OWNER_TAG_DB_ID + 1);

        let roots = resolve_naval_roots(&mem).unwrap();
        assert_eq!(roots.task_forces.len(), 1);

        let extracted = extract_rows_from_roots(&mem, &roots).unwrap();
        assert_eq!(extracted.observed_task_forces, 1);
        assert_eq!(extracted.invalid_task_force_rows, 1);
        assert!(extracted.rows.is_empty());

        let frame = encode_rows(
            1,
            compact::STATUS_PARTIAL_DATA,
            extracted.observed_task_forces,
            &extracted.rows,
        );
        assert_eq!(
            frame.header.status_flags & compact::STATUS_PARTIAL_DATA,
            compact::STATUS_PARTIAL_DATA
        );
        assert!(compact::decode_status_payload(0, &frame.payload).is_ok());
    }

    #[test]
    fn tracker_emits_full_replace_payloads_with_changed_row_counts() {
        let mem = fake_memory();
        let roots = resolve_naval_roots(&mem).unwrap();
        let extracted = extract_rows_from_roots(&mem, &roots).unwrap();
        let mut tracker = CompactNavalMissionStatusTracker::default();

        let first = tracker.encode_frame(10, 0, extracted.observed_task_forces, &extracted.rows);
        assert_eq!(first.header.changed_rows, 1);
        assert_eq!(first.header.status_flags & compact::STATUS_DELTA_DATA, 0);
        assert!(compact::decode_status_payload(0, &first.payload).is_ok());

        let second = tracker.encode_frame(11, 0, extracted.observed_task_forces, &extracted.rows);
        assert_eq!(second.header.changed_rows, 0);
        assert_eq!(second.rows.len(), 1);
        assert_eq!(second.header.status_flags & compact::STATUS_DELTA_DATA, 0);
    }
}
