#[path = "domain52_compact.rs"]
pub mod compact;

use std::collections::{BTreeMap, BTreeSet};

pub const EXPECTED_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

pub const CG_STATE_DATE_OFFSET: usize = 0x3316E90;
pub const CGAMESTATE_CPROVINCE_ARRAY: usize = 0x2B0;
pub const CGAMESTATE_CPROVINCE_COUNT: usize = 0x2BC;
pub const CGAMESTATE_COUNTRY_TABLE: usize = 0x310;
pub const CGAMESTATE_COUNTRY_COUNT: usize = 0x31C;

pub const STATE_VTABLE_RVA: usize = 0x29206C0;
pub const STATE_OFF_SID: usize = 0x58;
pub const STATE_OFF_OWNER: usize = 0xC8;
pub const STATE_OFF_CTRL: usize = 0xCC;
pub const STATE_OFF_RESISTANCE: usize = 0x278;
pub const STATE_OFF_RESISTANCE_SPEED: usize = 0x280;
pub const STATE_OFF_COMPLIANCE: usize = 0x2A8;
pub const STATE_OFF_COMPLIANCE_SPEED: usize = 0x2B0;

pub const CPROVINCE_VTABLE_RVA: usize = 0x295AF68;
pub const CPROVINCE_OFF_ID: usize = 0xA4;
pub const CPROVINCE_OFF_STATE: usize = 0xC0;

pub const COUNTRY_VTABLE_RVA: usize = 0x27D0E90;
pub const COUNTRY_OFF_TAG_DB_ID: usize = 0x08;
pub const COUNTRY_OFF_OCCUPATION_STATUS: usize = 0xFD0;
pub const COUNTRY_OFF_COLLABORATION_STATUS: usize = 0xFD8;
pub const COUNTRY_OCCUPATION_STATUS_VTABLE_RVA: usize = 0x296DD50;
pub const COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_RVA: usize = 0x296DD88;
pub const COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_OFF: usize = 0x18;
pub const COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF: usize = 0x118;
pub const COUNTRY_OCCUPATION_STATUS_GARRISON_TEMPLATE_OFF: usize = 0x78;
pub const COUNTRY_COLLABORATION_STATUS_VTABLE_RVA: usize = 0x29A5D58;
pub const OCCUPATION_LAW_VTABLE_RVA: usize = 0x2928218;
pub const OCCUPATION_LAW_NAME_OFF: usize = 0x10;
pub const DIVISION_TEMPLATE_VTABLE_RVA: usize = 0x2938F70;
pub const DIVISION_TEMPLATE_TYPE: u32 = 52;
pub const DIVISION_TEMPLATE_OFF_TYPE: usize = 0x08;
pub const DIVISION_TEMPLATE_OFF_ID: usize = 0x0C;

// Garrison scalar chain (wave-3 crack, validated on the paused D04 save vs the
// same-tick save oracle: 303 CStateGarrisonData rows, clean diagonal, 0 stale;
// build 870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad).
// Chain: CCountryOccupationStatus+0x48 -> occupation-data container ->
// CCountryOccupationData (+0x08 parent back-ref, +0x10 occupied tag_db_id,
// +0xD8 garrison-list container) -> CStateGarrisonData (+0x50 -> CState,
// +0x58 -> CCountryOccupationData back-ref).
pub const COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF: usize = 0x48;
pub const COUNTRY_OCCUPATION_DATA_VTABLE_RVA: usize = 0x296DD00;
pub const COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF: usize = 0x08;
pub const COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF: usize = 0x10;
pub const COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF: usize = 0xD8;
pub const STATE_GARRISON_DATA_VTABLE_RVA: usize = 0x296DCB0;
pub const STATE_GARRISON_DATA_STATE_OFF: usize = 0x50;
pub const STATE_GARRISON_DATA_OCCUPATION_DATA_OFF: usize = 0x58;
// Validated scalars: +0x88 i32 army_manpower_need (18/18 discriminating rows,
// 280/280 all), +0xF0 i32 fixed-x100000 strength_ratio (288/288), +0xF8 i32
// fixed-x100000 garrison_required (282/282).
pub const STATE_GARRISON_DATA_ARMY_MANPOWER_NEED_OFF: usize = 0x88;
pub const STATE_GARRISON_DATA_STRENGTH_RATIO_OFF: usize = 0xF0;
pub const STATE_GARRISON_DATA_GARRISON_REQUIRED_OFF: usize = 0xF8;
// The occupation-data/garrison containers are walked with the same bounded
// qword pointer scan the validation probe used (each candidate is accepted
// only after exact vtable + parent back-ref + validated-state join checks, so
// stale or foreign pointers fail closed).
pub const GARRISON_CONTAINER_SCAN_LEN: usize = 0x300;
pub const STATE_GARRISON_READ_LEN: usize = 0x100;

pub const HEAP_LO: u64 = 0x10000000000;
pub const HEAP_HI: u64 = 0x7FF000000000;
pub const MIN_USABLE_STATE_ROWS: usize = 1000;
pub const MIN_USABLE_PROVINCE_ROWS: usize = 10000;
pub const MAX_PROVINCE_COUNT: u32 = 65536;
pub const MAX_TAG_DB_ID: u32 = 1024;
pub const MAX_ABS_FIXED_RAW: i32 = 100_000_000;

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
    ConflictingStateRoot {
        state_id: u32,
        old: u64,
        new: u64,
    },
}

pub trait OccupationMemory {
    fn module_base(&self) -> u64;
    fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError>;

    fn read_u64(&self, address: u64) -> Result<u64, ExtractError> {
        let bytes = self.read_bytes(address, 8)?;
        Ok(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
    }

    fn read_u32(&self, address: u64) -> Result<u32, ExtractError> {
        let bytes = self.read_bytes(address, 4)?;
        Ok(u32::from_le_bytes(bytes[..4].try_into().unwrap()))
    }

    fn read_i32(&self, address: u64) -> Result<i32, ExtractError> {
        let bytes = self.read_bytes(address, 4)?;
        Ok(i32::from_le_bytes(bytes[..4].try_into().unwrap()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateRoots {
    pub game_state: u64,
    pub province_array: u64,
    pub province_count: u32,
    pub province_rows: u32,
    pub invalid_province_rows: u32,
    pub invalid_state_links: u32,
    pub state_roots: BTreeMap<u32, u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryOccupationStatusSummary {
    pub country_count: u32,
    pub valid_country_roots: u32,
    pub valid_occupation_status_roots: u32,
    pub valid_default_law_roots: u32,
    pub valid_garrison_template_roots: u32,
    pub valid_collaboration_status_roots: u32,
    pub invalid_status_roots: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryOccupationStatus {
    pub country_ptr: u64,
    pub occupation_status_ptr: u64,
    pub collaboration_status_ptr: u64,
    pub default_law_ptr: u64,
    pub default_law_token_id: u32,
    pub garrison_template_ptr: u64,
    pub garrison_template_id: u32,
}

/// One validated CStateGarrisonData row, keyed by the state it garrisons.
/// Rows are only accepted after vtable + parent back-ref + validated-state
/// join checks; conflicting duplicates for one state are dropped fail-closed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateGarrisonScalars {
    pub controller_tag_db_id: u32,
    pub occupied_tag_db_id: u32,
    pub army_manpower_need: i32,
    pub strength_ratio_raw: i32,
    pub garrison_required_raw: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GarrisonScalarWalk {
    pub by_state: BTreeMap<u32, StateGarrisonScalars>,
    pub anomaly_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedRows {
    pub rows: Vec<compact::OccupationStateRow>,
    pub observed_state_rows: u32,
    pub invalid_state_rows: u32,
    pub garrison_anomaly_rows: u32,
    pub status_summary: CountryOccupationStatusSummary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedFrame {
    pub payload: Vec<u8>,
    pub header: compact::OccupationHeader,
    pub rows: Vec<compact::OccupationStateRow>,
}

#[derive(Clone, Debug, Default)]
pub struct CompactOccupationResistanceComplianceTracker {
    last_rows: BTreeMap<u32, compact::OccupationStateRow>,
    emitted_baseline: bool,
}

impl CompactOccupationResistanceComplianceTracker {
    pub fn encode_frame(
        &mut self,
        elapsed_wall_ms: u64,
        status_flags: u32,
        observed_state_rows: u32,
        garrison_anomaly_rows: u32,
        rows: &[compact::OccupationStateRow],
    ) -> EncodedFrame {
        let had_baseline = self.emitted_baseline;
        let delta_rows = self.delta_rows(rows);
        let mut final_status_flags = status_flags;
        if had_baseline {
            final_status_flags |= compact::STATUS_DELTA_DATA;
        }
        let header = build_header(
            elapsed_wall_ms,
            final_status_flags,
            observed_state_rows,
            garrison_anomaly_rows,
            &delta_rows,
        );
        let payload = compact::encode_payload(&header, &delta_rows);
        EncodedFrame {
            payload,
            header,
            rows: delta_rows,
        }
    }

    fn delta_rows(
        &mut self,
        rows: &[compact::OccupationStateRow],
    ) -> Vec<compact::OccupationStateRow> {
        let mut changed = Vec::new();
        for row in rows {
            let old = self.last_rows.insert(row.state_id, row.clone());
            if !self.emitted_baseline || old.as_ref() != Some(row) {
                changed.push(row.clone());
            }
        }
        self.emitted_baseline = true;
        changed
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

pub fn resolve_state_roots<M: OccupationMemory>(mem: &M) -> Result<StateRoots, ExtractError> {
    let module_base = mem.module_base();
    let game_state = mem.read_u64(module_base + CG_STATE_DATE_OFFSET as u64)?;
    if !is_heap_ptr(game_state) {
        return Err(ExtractError::InvalidRoot("CGameState"));
    }
    let province_array = mem.read_u64(game_state + CGAMESTATE_CPROVINCE_ARRAY as u64)?;
    let province_count = mem.read_u32(game_state + CGAMESTATE_CPROVINCE_COUNT as u64)?;
    if !is_heap_ptr(province_array) {
        return Err(ExtractError::InvalidRoot("CProvince array"));
    }
    if province_count <= MIN_USABLE_PROVINCE_ROWS as u32 || province_count > MAX_PROVINCE_COUNT {
        return Err(ExtractError::InvalidCount {
            name: "province_count",
            value: province_count,
        });
    }

    let ptr_bytes = mem.read_bytes(province_array, province_count as usize * 8)?;
    let cprovince_vtable = module_base + CPROVINCE_VTABLE_RVA as u64;
    let state_vtable = module_base + STATE_VTABLE_RVA as u64;
    let mut state_roots = BTreeMap::new();
    let mut province_rows = 0u32;
    let mut invalid_province_rows = 0u32;
    let mut invalid_state_links = 0u32;

    for province_id in 1..province_count {
        let province_ptr = u64_from(&ptr_bytes, province_id as usize * 8);
        if province_ptr == 0 {
            continue;
        }
        if !is_heap_ptr(province_ptr) {
            invalid_province_rows = invalid_province_rows.saturating_add(1);
            continue;
        }
        let province_ok = mem.read_u64(province_ptr).ok() == Some(cprovince_vtable)
            && mem.read_u32(province_ptr + CPROVINCE_OFF_ID as u64).ok() == Some(province_id);
        if !province_ok {
            invalid_province_rows = invalid_province_rows.saturating_add(1);
            continue;
        }
        province_rows = province_rows.saturating_add(1);
        let state_ptr = match mem.read_u64(province_ptr + CPROVINCE_OFF_STATE as u64) {
            Ok(value) => value,
            Err(_) => {
                invalid_state_links = invalid_state_links.saturating_add(1);
                continue;
            }
        };
        if state_ptr == 0 {
            continue;
        }
        if !is_heap_ptr(state_ptr) {
            invalid_state_links = invalid_state_links.saturating_add(1);
            continue;
        }
        let state_ok = mem.read_u64(state_ptr).ok() == Some(state_vtable);
        let state_id = mem.read_u32(state_ptr + STATE_OFF_SID as u64).unwrap_or(0);
        if !state_ok || !(1..=u16::MAX as u32).contains(&state_id) {
            invalid_state_links = invalid_state_links.saturating_add(1);
            continue;
        }
        if let Some(old) = state_roots.insert(state_id, state_ptr) {
            if old != state_ptr {
                return Err(ExtractError::ConflictingStateRoot {
                    state_id,
                    old,
                    new: state_ptr,
                });
            }
        }
    }

    if province_rows < MIN_USABLE_PROVINCE_ROWS as u32 {
        return Err(ExtractError::InvalidCount {
            name: "validated_province_rows",
            value: province_rows,
        });
    }
    if state_roots.len() < MIN_USABLE_STATE_ROWS {
        return Err(ExtractError::InvalidCount {
            name: "validated_state_roots",
            value: state_roots.len().min(u32::MAX as usize) as u32,
        });
    }

    Ok(StateRoots {
        game_state,
        province_array,
        province_count,
        province_rows,
        invalid_province_rows,
        invalid_state_links,
        state_roots,
    })
}

pub fn read_country_occupation_status<M: OccupationMemory>(
    mem: &M,
    game_state: u64,
) -> Result<
    (
        BTreeMap<u32, CountryOccupationStatus>,
        CountryOccupationStatusSummary,
    ),
    ExtractError,
> {
    let country_table = mem.read_u64(game_state + CGAMESTATE_COUNTRY_TABLE as u64)?;
    let country_count = mem.read_u32(game_state + CGAMESTATE_COUNTRY_COUNT as u64)?;
    let mut summary = CountryOccupationStatusSummary {
        country_count,
        ..CountryOccupationStatusSummary::default()
    };
    if !is_heap_ptr(country_table) || country_count == 0 || country_count > MAX_TAG_DB_ID {
        return Ok((BTreeMap::new(), summary));
    }

    let ptrs = mem.read_bytes(country_table, country_count as usize * 8)?;
    let country_vtable = mem.module_base() + COUNTRY_VTABLE_RVA as u64;
    let status_vtable = mem.module_base() + COUNTRY_OCCUPATION_STATUS_VTABLE_RVA as u64;
    let status_secondary_vtable =
        mem.module_base() + COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_RVA as u64;
    let collab_vtable = mem.module_base() + COUNTRY_COLLABORATION_STATUS_VTABLE_RVA as u64;
    let law_vtable = mem.module_base() + OCCUPATION_LAW_VTABLE_RVA as u64;
    let template_vtable = mem.module_base() + DIVISION_TEMPLATE_VTABLE_RVA as u64;
    let mut by_tag = BTreeMap::new();

    for tag_id in 1..country_count {
        let country_ptr = u64_from(&ptrs, tag_id as usize * 8);
        if !is_heap_ptr(country_ptr) {
            continue;
        }
        let country_ok = mem.read_u64(country_ptr).ok() == Some(country_vtable)
            && mem
                .read_u32(country_ptr + COUNTRY_OFF_TAG_DB_ID as u64)
                .ok()
                == Some(tag_id);
        if !country_ok {
            continue;
        }
        summary.valid_country_roots = summary.valid_country_roots.saturating_add(1);
        let status_ptr = mem
            .read_u64(country_ptr + COUNTRY_OFF_OCCUPATION_STATUS as u64)
            .unwrap_or(0);
        let collab_ptr = mem
            .read_u64(country_ptr + COUNTRY_OFF_COLLABORATION_STATUS as u64)
            .unwrap_or(0);
        let collab_valid =
            is_heap_ptr(collab_ptr) && mem.read_u64(collab_ptr).ok() == Some(collab_vtable);
        if collab_valid {
            summary.valid_collaboration_status_roots =
                summary.valid_collaboration_status_roots.saturating_add(1);
        }

        let status_valid = is_heap_ptr(status_ptr)
            && mem.read_u64(status_ptr).ok() == Some(status_vtable)
            && mem
                .read_u64(status_ptr + COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_OFF as u64)
                .ok()
                == Some(status_secondary_vtable);
        if !status_valid {
            summary.invalid_status_roots = summary.invalid_status_roots.saturating_add(1);
            continue;
        }
        summary.valid_occupation_status_roots =
            summary.valid_occupation_status_roots.saturating_add(1);

        let mut rec = CountryOccupationStatus {
            country_ptr,
            occupation_status_ptr: status_ptr,
            collaboration_status_ptr: if collab_valid { collab_ptr } else { 0 },
            ..CountryOccupationStatus::default()
        };

        let law_ptr = mem
            .read_u64(status_ptr + COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF as u64)
            .unwrap_or(0);
        if is_heap_ptr(law_ptr) && mem.read_u64(law_ptr).ok() == Some(law_vtable) {
            if let Some(name) = read_pdx_string(mem, law_ptr + OCCUPATION_LAW_NAME_OFF as u64) {
                rec.default_law_ptr = law_ptr;
                rec.default_law_token_id = fnv1a32(&name);
                summary.valid_default_law_roots = summary.valid_default_law_roots.saturating_add(1);
            }
        }

        let template_ptr = mem
            .read_u64(status_ptr + COUNTRY_OCCUPATION_STATUS_GARRISON_TEMPLATE_OFF as u64)
            .unwrap_or(0);
        if is_heap_ptr(template_ptr)
            && mem.read_u64(template_ptr).ok() == Some(template_vtable)
            && mem
                .read_u32(template_ptr + DIVISION_TEMPLATE_OFF_TYPE as u64)
                .ok()
                == Some(DIVISION_TEMPLATE_TYPE)
        {
            let template_id = mem
                .read_u32(template_ptr + DIVISION_TEMPLATE_OFF_ID as u64)
                .unwrap_or(0);
            if template_id != 0 {
                rec.garrison_template_ptr = template_ptr;
                rec.garrison_template_id = template_id;
                summary.valid_garrison_template_roots =
                    summary.valid_garrison_template_roots.saturating_add(1);
            }
        }

        by_tag.insert(tag_id, rec);
    }

    Ok((by_tag, summary))
}

pub fn extract_rows_from_state_roots<M: OccupationMemory>(
    mem: &M,
    game_state: u64,
    state_roots: &BTreeMap<u32, u64>,
) -> Result<ExtractedRows, ExtractError> {
    let (occupation_status, status_summary) = read_country_occupation_status(mem, game_state)?;
    let garrison_walk = read_state_garrison_scalars(mem, &occupation_status, state_roots);
    let mut garrison_anomaly_rows = garrison_walk.anomaly_rows;
    let mut rows = Vec::with_capacity(state_roots.len());
    let mut invalid_state_rows = 0u32;
    let state_vtable = mem.module_base() + STATE_VTABLE_RVA as u64;

    for (&state_id, &state_ptr) in state_roots {
        let mut row =
            match read_state_row(mem, state_vtable, state_id, state_ptr, &occupation_status) {
                Ok(Some(row)) => row,
                Ok(None) => {
                    invalid_state_rows = invalid_state_rows.saturating_add(1);
                    continue;
                }
                Err(_) => {
                    invalid_state_rows = invalid_state_rows.saturating_add(1);
                    continue;
                }
            };
        if let Some(garrison) = garrison_walk.by_state.get(&state_id) {
            // The garrison row must corroborate the state row it enriches: the
            // controller whose occupation-status root the garrison row was
            // discovered under must be this state's current controller (a live
            // 303/303 invariant on the validation fixture). A mismatch means
            // the garrison object is stale relative to this frame's state row,
            // so the scalars stay badged unavailable (fail closed, absent !=
            // 0). The garrison chain's occupied tag (+0x10) is deliberately
            // NOT compared against the state owner: it is the occupation
            // GROUPING country (original/core owner, e.g. colonial garrisons
            // grouped under the never-independent core tag), which routinely
            // differs from the current owner, and garrison rows also exist
            // for states whose owner equals their controller.
            let join_ok = row.controller_tag_db_id != 0
                && garrison.controller_tag_db_id == row.controller_tag_db_id;
            if join_ok {
                row.garrison_manpower_need = garrison.army_manpower_need;
                row.garrison_strength_ratio_raw = garrison.strength_ratio_raw;
                row.garrison_required_raw = garrison.garrison_required_raw;
                row.flags = (row.flags & !compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE)
                    | compact::ROW_FLAG_GARRISON_SCALARS;
            } else {
                garrison_anomaly_rows = garrison_anomaly_rows.saturating_add(1);
            }
        }
        rows.push(row);
    }

    Ok(ExtractedRows {
        rows,
        observed_state_rows: state_roots.len().min(u32::MAX as usize) as u32,
        invalid_state_rows,
        garrison_anomaly_rows,
        status_summary,
    })
}

/// Walk the validated occupation-status roots to the per-state
/// CStateGarrisonData rows and read the three validated garrison scalars.
///
/// Every accepted row passed, from a single blob read: exact vtable match,
/// CStateGarrisonData+0x58 back-ref to the exact CCountryOccupationData it was
/// found under (which itself back-refs its CCountryOccupationStatus at +0x08),
/// and a +0x50 state pointer join against this frame's validated state roots.
/// Conflicting duplicate rows for one state are removed again (fail closed)
/// and counted as anomalies; benign re-discoveries of the identical row dedup
/// silently.
pub fn read_state_garrison_scalars<M: OccupationMemory>(
    mem: &M,
    occupation_status: &BTreeMap<u32, CountryOccupationStatus>,
    state_roots: &BTreeMap<u32, u64>,
) -> GarrisonScalarWalk {
    let state_id_by_ptr: BTreeMap<u64, u32> = state_roots
        .iter()
        .map(|(&state_id, &state_ptr)| (state_ptr, state_id))
        .collect();
    let occupation_data_vtable = mem.module_base() + COUNTRY_OCCUPATION_DATA_VTABLE_RVA as u64;
    let garrison_vtable = mem.module_base() + STATE_GARRISON_DATA_VTABLE_RVA as u64;
    let mut by_state: BTreeMap<u32, StateGarrisonScalars> = BTreeMap::new();
    let mut conflicted: BTreeSet<u32> = BTreeSet::new();
    let mut anomaly_rows = 0u32;

    for (&controller_tag_db_id, status) in occupation_status {
        let container = match mem.read_u64(
            status.occupation_status_ptr
                + COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF as u64,
        ) {
            Ok(value) => value,
            Err(_) => {
                anomaly_rows = anomaly_rows.saturating_add(1);
                continue;
            }
        };
        if !is_heap_ptr(container) {
            // No occupation data for this country: normal absence.
            continue;
        }
        let Ok(container_blob) = mem.read_bytes(container, GARRISON_CONTAINER_SCAN_LEN) else {
            anomaly_rows = anomaly_rows.saturating_add(1);
            continue;
        };
        let mut seen_occupation_data = BTreeSet::new();
        for occupation_data_ptr in iter_qword_heap_ptrs(&container_blob) {
            if mem.read_u64(occupation_data_ptr).ok() != Some(occupation_data_vtable) {
                continue;
            }
            if !seen_occupation_data.insert(occupation_data_ptr) {
                continue;
            }
            // Parent back-ref: this occupation-data record must belong to the
            // exact status root we reached it from; foreign records are not
            // anomalies, they are simply not ours.
            if mem
                .read_u64(occupation_data_ptr + COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF as u64)
                .ok()
                != Some(status.occupation_status_ptr)
            {
                continue;
            }
            let occupied_tag_db_id = match mem
                .read_u32(occupation_data_ptr + COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF as u64)
            {
                Ok(value) if value != 0 && value <= MAX_TAG_DB_ID => value,
                _ => {
                    anomaly_rows = anomaly_rows.saturating_add(1);
                    continue;
                }
            };
            let garrison_container = match mem.read_u64(
                occupation_data_ptr + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            ) {
                Ok(value) => value,
                Err(_) => {
                    anomaly_rows = anomaly_rows.saturating_add(1);
                    continue;
                }
            };
            if !is_heap_ptr(garrison_container) {
                continue;
            }
            let Ok(garrison_blob) = mem.read_bytes(garrison_container, GARRISON_CONTAINER_SCAN_LEN)
            else {
                anomaly_rows = anomaly_rows.saturating_add(1);
                continue;
            };
            let mut seen_garrison = BTreeSet::new();
            for garrison_ptr in iter_qword_heap_ptrs(&garrison_blob) {
                if !seen_garrison.insert(garrison_ptr) {
                    continue;
                }
                let Ok(blob) = mem.read_bytes(garrison_ptr, STATE_GARRISON_READ_LEN) else {
                    continue;
                };
                if u64_from(&blob, 0) != garrison_vtable {
                    continue;
                }
                // Back-ref + state join and the scalars all come from the same
                // single blob read, so a row can never mix data from two
                // different object generations.
                if u64_from(&blob, STATE_GARRISON_DATA_OCCUPATION_DATA_OFF) != occupation_data_ptr {
                    continue;
                }
                let state_ptr = u64_from(&blob, STATE_GARRISON_DATA_STATE_OFF);
                let Some(&state_id) = state_id_by_ptr.get(&state_ptr) else {
                    anomaly_rows = anomaly_rows.saturating_add(1);
                    continue;
                };
                let army_manpower_need =
                    i32_from(&blob, STATE_GARRISON_DATA_ARMY_MANPOWER_NEED_OFF);
                let strength_ratio_raw = i32_from(&blob, STATE_GARRISON_DATA_STRENGTH_RATIO_OFF);
                let garrison_required_raw =
                    i32_from(&blob, STATE_GARRISON_DATA_GARRISON_REQUIRED_OFF);
                if army_manpower_need < 0
                    || army_manpower_need > MAX_ABS_FIXED_RAW
                    || strength_ratio_raw.abs() > MAX_ABS_FIXED_RAW
                    || garrison_required_raw.abs() > MAX_ABS_FIXED_RAW
                {
                    anomaly_rows = anomaly_rows.saturating_add(1);
                    continue;
                }
                let record = StateGarrisonScalars {
                    controller_tag_db_id,
                    occupied_tag_db_id,
                    army_manpower_need,
                    strength_ratio_raw,
                    garrison_required_raw,
                };
                match by_state.get(&state_id) {
                    None => {
                        by_state.insert(state_id, record);
                    }
                    Some(existing) if *existing == record => {}
                    Some(_) => {
                        // Two live garrison rows disagree about one state:
                        // frame-scope identity violation, drop both.
                        conflicted.insert(state_id);
                        anomaly_rows = anomaly_rows.saturating_add(1);
                    }
                }
            }
        }
    }

    for state_id in &conflicted {
        by_state.remove(state_id);
        // The originally inserted row is also dropped by the conflict, so it
        // counts as a failed-closed row too (the conflicting row itself was
        // already counted when the conflict was detected).
        anomaly_rows = anomaly_rows.saturating_add(1);
    }
    GarrisonScalarWalk {
        by_state,
        anomaly_rows,
    }
}

fn iter_qword_heap_ptrs(blob: &[u8]) -> impl Iterator<Item = u64> + '_ {
    (0..blob.len().saturating_sub(7))
        .step_by(8)
        .map(move |offset| u64_from(blob, offset))
        .filter(|&value| is_heap_ptr(value))
}

pub fn build_header(
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_state_rows: u32,
    garrison_anomaly_rows: u32,
    rows: &[compact::OccupationStateRow],
) -> compact::OccupationHeader {
    compact::OccupationHeader {
        elapsed_wall_ms,
        status_flags,
        changed_rows: rows.len().min(u32::MAX as usize) as u32,
        observed_state_rows,
        occupied_state_rows: count_rows(rows, compact::ROW_FLAG_OCCUPIED_STATE),
        wall_badge_rows: rows
            .iter()
            .filter(|row| {
                row.flags
                    & (compact::ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE
                        | compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
                        | compact::ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE)
                    != 0
            })
            .count()
            .min(u32::MAX as usize) as u32,
        law_token_rows: count_rows(rows, compact::ROW_FLAG_OCCUPATION_LAW_TOKEN),
        garrison_template_rows: count_rows(rows, compact::ROW_FLAG_GARRISON_TEMPLATE),
        garrison_scalar_rows: count_rows(rows, compact::ROW_FLAG_GARRISON_SCALARS),
        garrison_anomaly_rows,
    }
}

pub fn encode_rows(
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_state_rows: u32,
    garrison_anomaly_rows: u32,
    rows: &[compact::OccupationStateRow],
) -> EncodedFrame {
    let header = build_header(
        elapsed_wall_ms,
        status_flags,
        observed_state_rows,
        garrison_anomaly_rows,
        rows,
    );
    let payload = compact::encode_payload(&header, rows);
    EncodedFrame {
        payload,
        header,
        rows: rows.to_vec(),
    }
}

fn read_state_row<M: OccupationMemory>(
    mem: &M,
    state_vtable: u64,
    state_id: u32,
    state_ptr: u64,
    occupation_status: &BTreeMap<u32, CountryOccupationStatus>,
) -> Result<Option<compact::OccupationStateRow>, ExtractError> {
    let blob = mem.read_bytes(state_ptr, STATE_OFF_COMPLIANCE_SPEED + 4)?;
    if u64_from(&blob, 0) != state_vtable || u32_from(&blob, STATE_OFF_SID) != state_id {
        return Ok(None);
    }
    let owner = u32_from(&blob, STATE_OFF_OWNER);
    let controller = u32_from(&blob, STATE_OFF_CTRL);
    let resistance_raw = i32_from(&blob, STATE_OFF_RESISTANCE);
    let resistance_speed_raw = i32_from(&blob, STATE_OFF_RESISTANCE_SPEED);
    let compliance_raw = i32_from(&blob, STATE_OFF_COMPLIANCE);
    let compliance_speed_raw = i32_from(&blob, STATE_OFF_COMPLIANCE_SPEED);
    if owner > MAX_TAG_DB_ID || controller > MAX_TAG_DB_ID {
        return Ok(None);
    }
    if [
        resistance_raw,
        resistance_speed_raw,
        compliance_raw,
        compliance_speed_raw,
    ]
    .into_iter()
    .any(|value| value.abs() > MAX_ABS_FIXED_RAW)
    {
        return Ok(None);
    }

    let mut flags = compact::ROW_FLAG_RESISTANCE
        | compact::ROW_FLAG_COMPLIANCE
        | compact::ROW_FLAG_RESISTANCE_SPEED
        | compact::ROW_FLAG_COMPLIANCE_SPEED;
    if owner != 0 {
        flags |= compact::ROW_FLAG_OWNER;
    }
    if controller != 0 {
        flags |= compact::ROW_FLAG_CONTROLLER;
    }
    let mut occupied_country = 0u32;
    if owner != 0 && controller != 0 && owner != controller {
        flags |= compact::ROW_FLAG_OCCUPIED_STATE | compact::ROW_FLAG_OCCUPIED_TAG_DERIVED;
        occupied_country = owner;
    }
    if resistance_raw != 0 || compliance_raw != 0 {
        flags |= compact::ROW_FLAG_OCCUPIED_STATE;
    }
    if flags & compact::ROW_FLAG_OCCUPIED_STATE != 0 {
        flags |= compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
            | compact::ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE;
    }

    let mut occupation_law_token_id = 0u32;
    let mut garrison_template_id = 0u32;
    if let Some(status) = occupation_status.get(&controller) {
        flags |= compact::ROW_FLAG_OCCUPATION_STATUS_ROOT;
        occupation_law_token_id = status.default_law_token_id;
        if occupation_law_token_id != 0 {
            flags |= compact::ROW_FLAG_OCCUPATION_LAW_TOKEN
                | compact::ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN;
        }
        if flags & compact::ROW_FLAG_OCCUPIED_STATE != 0 {
            garrison_template_id = status.garrison_template_id;
            if garrison_template_id != 0 {
                flags |= compact::ROW_FLAG_GARRISON_TEMPLATE;
            }
        }
    }

    Ok(Some(compact::OccupationStateRow {
        state_id,
        owner_tag_db_id: owner,
        controller_tag_db_id: controller,
        occupied_country_tag_db_id: occupied_country,
        resistance_raw,
        compliance_raw,
        resistance_speed_raw,
        compliance_speed_raw,
        garrison_manpower_need: 0,
        garrison_strength_ratio_raw: 0,
        garrison_required_raw: 0,
        occupation_law_token_id,
        garrison_template_id,
        flags,
    }))
}

fn read_pdx_string<M: OccupationMemory>(mem: &M, address: u64) -> Option<String> {
    let blob = mem.read_bytes(address, 32).ok()?;
    let size = u64_from(&blob, 0x10) as usize;
    let cap = u64_from(&blob, 0x18);
    if size == 0 || size > 255 {
        return None;
    }
    let raw = if cap == 15 {
        blob.get(0..size.min(16))?.to_vec()
    } else {
        let ptr = u64_from(&blob, 0);
        if !is_heap_address(ptr) {
            return None;
        }
        mem.read_bytes(ptr, size).ok()?
    };
    if !raw.iter().all(|byte| (32..127).contains(byte)) {
        return None;
    }
    String::from_utf8(raw).ok()
}

fn fnv1a32(text: &str) -> u32 {
    let mut value = 0x811C9DC5u32;
    for byte in text.as_bytes() {
        value ^= u32::from(*byte);
        value = value.wrapping_mul(0x01000193);
    }
    if value == 0 {
        1
    } else {
        value
    }
}

fn count_rows(rows: &[compact::OccupationStateRow], flag: u32) -> u32 {
    rows.iter()
        .filter(|row| row.flags & flag != 0)
        .count()
        .min(u32::MAX as usize) as u32
}

fn is_heap_ptr(value: u64) -> bool {
    (HEAP_LO..HEAP_HI).contains(&value) && value % 8 == 0
}

fn is_heap_address(value: u64) -> bool {
    (HEAP_LO..HEAP_HI).contains(&value)
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

fn i32_from(data: &[u8], offset: usize) -> i32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&data[offset..offset + 4]);
    i32::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULE_BASE: u64 = 0x7FF6AAFF0000;
    const GAME_STATE: u64 = 0x22000001000;
    const COUNTRY_TABLE: u64 = 0x22000002000;
    const COUNTRY_2: u64 = 0x22000003000;
    const STATUS_2: u64 = 0x22000005000;
    const LAW_2: u64 = 0x22000006000;
    const TEMPLATE_2: u64 = 0x22000007000;
    const COLLAB_2: u64 = 0x22000008000;
    const STATE_10: u64 = 0x22000009000;

    #[derive(Clone, Debug, Default)]
    struct FakeMemory {
        writes: BTreeMap<u64, u8>,
    }

    impl FakeMemory {
        fn write_bytes(&mut self, address: u64, bytes: &[u8]) {
            for (index, byte) in bytes.iter().enumerate() {
                self.writes.insert(address + index as u64, *byte);
            }
        }

        fn write_zeroes(&mut self, address: u64, len: usize) {
            self.write_bytes(address, &vec![0u8; len]);
        }

        fn write_u64(&mut self, address: u64, value: u64) {
            self.write_bytes(address, &value.to_le_bytes());
        }

        fn write_u32(&mut self, address: u64, value: u32) {
            self.write_bytes(address, &value.to_le_bytes());
        }

        fn write_i32(&mut self, address: u64, value: i32) {
            self.write_bytes(address, &value.to_le_bytes());
        }

        fn write_inline_pdx_string(&mut self, address: u64, value: &str) {
            let mut bytes = [0u8; 32];
            bytes[..value.len()].copy_from_slice(value.as_bytes());
            bytes[0x10..0x18].copy_from_slice(&(value.len() as u64).to_le_bytes());
            bytes[0x18..0x20].copy_from_slice(&15u64.to_le_bytes());
            self.write_bytes(address, &bytes);
        }
    }

    impl OccupationMemory for FakeMemory {
        fn module_base(&self) -> u64 {
            MODULE_BASE
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            let mut out = Vec::with_capacity(len);
            for index in 0..len {
                let addr = address + index as u64;
                let Some(byte) = self.writes.get(&addr) else {
                    return Err(ExtractError::MemoryRead { address, len });
                };
                out.push(*byte);
            }
            Ok(out)
        }
    }

    fn fake_memory(template_type: u32) -> FakeMemory {
        let mut mem = FakeMemory::default();
        mem.write_zeroes(GAME_STATE, 0x400);
        mem.write_zeroes(COUNTRY_TABLE, 4 * 8);
        mem.write_zeroes(COUNTRY_2, COUNTRY_OFF_COLLABORATION_STATUS + 8);
        mem.write_zeroes(STATUS_2, COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF + 8);
        mem.write_zeroes(LAW_2, OCCUPATION_LAW_NAME_OFF + 32);
        mem.write_zeroes(TEMPLATE_2, 0x20);
        mem.write_zeroes(COLLAB_2, 8);
        mem.write_zeroes(STATE_10, STATE_OFF_COMPLIANCE_SPEED + 4);

        mem.write_u64(GAME_STATE + CGAMESTATE_COUNTRY_TABLE as u64, COUNTRY_TABLE);
        mem.write_u32(GAME_STATE + CGAMESTATE_COUNTRY_COUNT as u64, 4);
        mem.write_u64(COUNTRY_TABLE + 2 * 8, COUNTRY_2);

        mem.write_u64(COUNTRY_2, MODULE_BASE + COUNTRY_VTABLE_RVA as u64);
        mem.write_u32(COUNTRY_2 + COUNTRY_OFF_TAG_DB_ID as u64, 2);
        mem.write_u64(COUNTRY_2 + COUNTRY_OFF_OCCUPATION_STATUS as u64, STATUS_2);
        mem.write_u64(
            COUNTRY_2 + COUNTRY_OFF_COLLABORATION_STATUS as u64,
            COLLAB_2,
        );
        mem.write_u64(
            COLLAB_2,
            MODULE_BASE + COUNTRY_COLLABORATION_STATUS_VTABLE_RVA as u64,
        );

        mem.write_u64(
            STATUS_2,
            MODULE_BASE + COUNTRY_OCCUPATION_STATUS_VTABLE_RVA as u64,
        );
        mem.write_u64(
            STATUS_2 + COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_OFF as u64,
            MODULE_BASE + COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_RVA as u64,
        );
        mem.write_u64(
            STATUS_2 + COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF as u64,
            LAW_2,
        );
        mem.write_u64(
            STATUS_2 + COUNTRY_OCCUPATION_STATUS_GARRISON_TEMPLATE_OFF as u64,
            TEMPLATE_2,
        );

        mem.write_u64(LAW_2, MODULE_BASE + OCCUPATION_LAW_VTABLE_RVA as u64);
        mem.write_inline_pdx_string(
            LAW_2 + OCCUPATION_LAW_NAME_OFF as u64,
            "military_governor_occupation",
        );

        mem.write_u64(
            TEMPLATE_2,
            MODULE_BASE + DIVISION_TEMPLATE_VTABLE_RVA as u64,
        );
        mem.write_u32(
            TEMPLATE_2 + DIVISION_TEMPLATE_OFF_TYPE as u64,
            template_type,
        );
        mem.write_u32(TEMPLATE_2 + DIVISION_TEMPLATE_OFF_ID as u64, 4183);

        mem.write_u64(STATE_10, MODULE_BASE + STATE_VTABLE_RVA as u64);
        mem.write_u32(STATE_10 + STATE_OFF_SID as u64, 10);
        mem.write_u32(STATE_10 + STATE_OFF_OWNER as u64, 1);
        mem.write_u32(STATE_10 + STATE_OFF_CTRL as u64, 2);
        mem.write_i32(STATE_10 + STATE_OFF_RESISTANCE as u64, 125_000);
        mem.write_i32(STATE_10 + STATE_OFF_RESISTANCE_SPEED as u64, 700);
        mem.write_i32(STATE_10 + STATE_OFF_COMPLIANCE as u64, 2_500_000);
        mem.write_i32(STATE_10 + STATE_OFF_COMPLIANCE_SPEED as u64, -300);
        mem
    }

    #[test]
    fn extracts_validated_rows_and_encodes_domain52_payload() {
        let mem = fake_memory(DIVISION_TEMPLATE_TYPE);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.status_summary.valid_occupation_status_roots, 1);
        assert_eq!(extracted.status_summary.valid_garrison_template_roots, 1);
        assert_eq!(extracted.status_summary.valid_collaboration_status_roots, 1);
        assert_eq!(extracted.invalid_state_rows, 0);
        assert_eq!(extracted.rows.len(), 1);

        let row = &extracted.rows[0];
        assert_eq!(row.state_id, 10);
        assert_eq!(row.occupied_country_tag_db_id, 1);
        assert_eq!(row.garrison_template_id, 4183);
        assert_eq!(row.garrison_manpower_need, 0);
        assert_eq!(row.garrison_strength_ratio_raw, 0);
        assert_eq!(row.garrison_required_raw, 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_TEMPLATE != 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert!(row.flags & compact::ROW_FLAG_OCCUPATION_LAW_TOKEN != 0);
        assert_eq!(extracted.garrison_anomaly_rows, 0);

        let encoded = encode_rows(25, 0, extracted.observed_state_rows, 0, &extracted.rows);
        assert_eq!(compact::DOMAIN_ID, 52);
        let summary = compact::decode_payload(0, &encoded.payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.data_rows_total, 1);
        assert_eq!(
            summary
                .data_units_total
                .get("occupation_garrison_template_rows"),
            Some(&1)
        );
    }

    #[test]
    fn invalid_garrison_template_type_fails_closed() {
        let mem = fake_memory(99);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.status_summary.valid_garrison_template_roots, 0);
        let row = &extracted.rows[0];
        assert_eq!(row.garrison_template_id, 0);
        assert_eq!(row.flags & compact::ROW_FLAG_GARRISON_TEMPLATE, 0);
    }

    #[test]
    fn tracker_emits_baseline_then_state_delta() {
        let mem = fake_memory(DIVISION_TEMPLATE_TYPE);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let mut extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        let mut tracker = CompactOccupationResistanceComplianceTracker::default();

        let first = tracker.encode_frame(1, 0, extracted.observed_state_rows, 0, &extracted.rows);
        assert_eq!(first.rows.len(), 1);
        assert_eq!(first.header.status_flags & compact::STATUS_DELTA_DATA, 0);

        let second = tracker.encode_frame(2, 0, extracted.observed_state_rows, 0, &extracted.rows);
        assert!(second.rows.is_empty());
        assert!(second.header.status_flags & compact::STATUS_DELTA_DATA != 0);
        assert!(compact::decode_payload(0, &second.payload)
            .parse_errors
            .is_empty());

        extracted.rows[0].resistance_raw += 1000;
        let third = tracker.encode_frame(3, 0, extracted.observed_state_rows, 0, &extracted.rows);
        assert_eq!(third.rows.len(), 1);
        assert!(third.header.status_flags & compact::STATUS_DELTA_DATA != 0);
    }

    #[test]
    fn build_gate_rejects_wrong_sha() {
        assert!(validate_build(EXPECTED_SHA256).is_ok());
        assert!(matches!(
            validate_build("bad"),
            Err(ExtractError::BuildMismatch { .. })
        ));
    }

    const OCC_DATA_CONTAINER: u64 = 0x2200000A000;
    const OCC_DATA: u64 = 0x2200000B000;
    const GARRISON_CONTAINER: u64 = 0x2200000C000;
    const GARRISON_ROW: u64 = 0x2200000D000;
    const GARRISON_ROW_B: u64 = 0x2200000E000;

    fn write_garrison_row(
        mem: &mut FakeMemory,
        row_ptr: u64,
        occupation_data_ptr: u64,
        state_ptr: u64,
        manpower_need: i32,
        strength_ratio_raw: i32,
        garrison_required_raw: i32,
    ) {
        mem.write_zeroes(row_ptr, STATE_GARRISON_READ_LEN);
        mem.write_u64(row_ptr, MODULE_BASE + STATE_GARRISON_DATA_VTABLE_RVA as u64);
        mem.write_u64(row_ptr + STATE_GARRISON_DATA_STATE_OFF as u64, state_ptr);
        mem.write_u64(
            row_ptr + STATE_GARRISON_DATA_OCCUPATION_DATA_OFF as u64,
            occupation_data_ptr,
        );
        mem.write_i32(
            row_ptr + STATE_GARRISON_DATA_ARMY_MANPOWER_NEED_OFF as u64,
            manpower_need,
        );
        mem.write_i32(
            row_ptr + STATE_GARRISON_DATA_STRENGTH_RATIO_OFF as u64,
            strength_ratio_raw,
        );
        mem.write_i32(
            row_ptr + STATE_GARRISON_DATA_GARRISON_REQUIRED_OFF as u64,
            garrison_required_raw,
        );
    }

    fn fake_memory_with_garrison(occupied_tag: u32) -> FakeMemory {
        let mut mem = fake_memory(DIVISION_TEMPLATE_TYPE);
        mem.write_u64(
            STATUS_2 + COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF as u64,
            OCC_DATA_CONTAINER,
        );
        mem.write_zeroes(OCC_DATA_CONTAINER, GARRISON_CONTAINER_SCAN_LEN);
        mem.write_u64(OCC_DATA_CONTAINER + 0x10, OCC_DATA);
        mem.write_zeroes(
            OCC_DATA,
            COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF + 8,
        );
        mem.write_u64(
            OCC_DATA,
            MODULE_BASE + COUNTRY_OCCUPATION_DATA_VTABLE_RVA as u64,
        );
        mem.write_u64(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF as u64,
            STATUS_2,
        );
        mem.write_u32(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF as u64,
            occupied_tag,
        );
        mem.write_u64(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER,
        );
        mem.write_zeroes(GARRISON_CONTAINER, GARRISON_CONTAINER_SCAN_LEN);
        mem.write_u64(GARRISON_CONTAINER + 0x08, GARRISON_ROW);
        write_garrison_row(
            &mut mem,
            GARRISON_ROW,
            OCC_DATA,
            STATE_10,
            14_000,
            87_500,
            250_000,
        );
        mem
    }

    #[test]
    fn attaches_validated_garrison_scalars_to_occupied_state_row() {
        let mem = fake_memory_with_garrison(1);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.garrison_anomaly_rows, 0);
        let row = &extracted.rows[0];
        assert_eq!(row.garrison_manpower_need, 14_000);
        assert_eq!(row.garrison_strength_ratio_raw, 87_500);
        assert_eq!(row.garrison_required_raw, 250_000);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS != 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE == 0);

        let encoded = encode_rows(
            25,
            0,
            extracted.observed_state_rows,
            extracted.garrison_anomaly_rows,
            &extracted.rows,
        );
        assert_eq!(encoded.header.garrison_scalar_rows, 1);
        assert_eq!(encoded.header.garrison_anomaly_rows, 0);
        let summary = compact::decode_payload(0, &encoded.payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("occupation_garrison_scalar_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn garrison_occupied_grouping_tag_differs_from_owner_still_attaches() {
        // The occupation chain groups garrisons by original/core tag (e.g.
        // colonial garrisons under a never-independent core country), which
        // routinely differs from the state's current owner. The join must not
        // require owner equality (live invariant is controller equality).
        let mem = fake_memory_with_garrison(3);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.garrison_anomaly_rows, 0);
        let row = &extracted.rows[0];
        assert_eq!(row.garrison_manpower_need, 14_000);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS != 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE == 0);
    }

    #[test]
    fn stale_garrison_controller_mismatch_fails_closed() {
        // The garrison row was discovered under controller 2's occupation
        // status, but the state row says controller 3 now controls the state:
        // the stale garrison row must not attach.
        let mut mem = fake_memory_with_garrison(1);
        mem.write_u32(STATE_10 + STATE_OFF_CTRL as u64, 3);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.garrison_anomaly_rows, 1);
        let row = &extracted.rows[0];
        assert_eq!(row.garrison_manpower_need, 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);
    }

    #[test]
    fn conflicting_garrison_rows_for_one_state_fail_closed() {
        let mut mem = fake_memory_with_garrison(1);
        mem.write_u64(GARRISON_CONTAINER + 0x18, GARRISON_ROW_B);
        write_garrison_row(&mut mem, GARRISON_ROW_B, OCC_DATA, STATE_10, 9_999, 1, 2);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        // Both conflicting rows failed closed: one counted at conflict
        // detection, one counted when the kept row is dropped again.
        assert_eq!(extracted.garrison_anomaly_rows, 2);
        let row = &extracted.rows[0];
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);
        assert_eq!(row.garrison_manpower_need, 0);
    }

    #[test]
    fn garrison_row_with_foreign_backref_is_ignored() {
        let mut mem = fake_memory_with_garrison(1);
        // Re-point the back-ref at a different (valid heap) address: the row
        // no longer belongs to this occupation-data record and is skipped
        // without attaching and without an anomaly.
        mem.write_u64(
            GARRISON_ROW + STATE_GARRISON_DATA_OCCUPATION_DATA_OFF as u64,
            GARRISON_ROW_B,
        );
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.garrison_anomaly_rows, 0);
        let row = &extracted.rows[0];
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);
    }

    #[test]
    fn duplicate_identical_garrison_row_discovery_dedups_silently() {
        let mut mem = fake_memory_with_garrison(1);
        // The same garrison object reachable through two container slots is a
        // benign re-discovery, not a conflict.
        mem.write_u64(GARRISON_CONTAINER + 0x40, GARRISON_ROW);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.garrison_anomaly_rows, 0);
        let row = &extracted.rows[0];
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS != 0);
        assert_eq!(row.garrison_manpower_need, 14_000);
    }
}
