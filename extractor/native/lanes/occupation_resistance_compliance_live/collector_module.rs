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
pub const STATE_ROOT_WINDOW_LEN: usize = STATE_OFF_SID + 4;

pub const CPROVINCE_VTABLE_RVA: usize = 0x295AF68;
pub const CPROVINCE_OFF_ID: usize = 0xA4;
pub const CPROVINCE_OFF_STATE: usize = 0xC0;
pub const CPROVINCE_WINDOW_LEN: usize = CPROVINCE_OFF_STATE + 8;

pub const COUNTRY_VTABLE_RVA: usize = 0x27D0E90;
pub const COUNTRY_OFF_TAG_DB_ID: usize = 0x08;
pub const COUNTRY_OFF_OCCUPATION_STATUS: usize = 0xFD0;
pub const COUNTRY_OFF_COLLABORATION_STATUS: usize = 0xFD8;
pub const COUNTRY_WINDOW_LEN: usize = COUNTRY_OFF_COLLABORATION_STATUS + 8;
pub const COUNTRY_OCCUPATION_STATUS_VTABLE_RVA: usize = 0x296DD50;
pub const COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_RVA: usize = 0x296DD88;
pub const COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_OFF: usize = 0x18;
pub const COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF: usize = 0x118;
pub const COUNTRY_OCCUPATION_STATUS_GARRISON_TEMPLATE_OFF: usize = 0x78;
pub const COUNTRY_OCCUPATION_STATUS_WINDOW_LEN: usize =
    COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF + 8;
pub const COUNTRY_COLLABORATION_STATUS_VTABLE_RVA: usize = 0x29A5D58;
pub const COUNTRY_COLLABORATION_DATA_VTABLE_RVA: usize = 0x29A5D08;
pub const COUNTRY_COLLABORATION_STATUS_ROWS_OFF: usize = 0x28;
pub const COUNTRY_COLLABORATION_STATUS_ROW_COUNT_OFF: usize = 0x34;
pub const COUNTRY_COLLABORATION_STATUS_OWNER_OFF: usize = 0x40;
pub const COUNTRY_COLLABORATION_STATUS_INCOMING_OFF: usize = 0x48;
pub const COUNTRY_COLLABORATION_STATUS_INCOMING_COUNT_OFF: usize = 0x54;
pub const COUNTRY_COLLABORATION_DATA_TARGET_OFF: usize = 0x08;
pub const COUNTRY_COLLABORATION_DATA_PROGRESS_OFF: usize = 0x10;
pub const COUNTRY_COLLABORATION_DATA_LEN: usize = 0x18;
pub const MAX_COLLABORATION_RELATIONS_PER_COUNTRY: u32 = 1024;
pub const OCCUPATION_LAW_VTABLE_RVA: usize = 0x2928218;
pub const OCCUPATION_LAW_NAME_OFF: usize = 0x10;
pub const PDX_STRING_WINDOW_LEN: usize = 0x20;
pub const OCCUPATION_LAW_WINDOW_LEN: usize = OCCUPATION_LAW_NAME_OFF + PDX_STRING_WINDOW_LEN;
pub const DIVISION_TEMPLATE_VTABLE_RVA: usize = 0x2938F70;
pub const DIVISION_TEMPLATE_TYPE: u32 = 52;
pub const DIVISION_TEMPLATE_OFF_TYPE: usize = 0x08;
pub const DIVISION_TEMPLATE_OFF_ID: usize = 0x0C;
pub const DIVISION_TEMPLATE_WINDOW_LEN: usize = DIVISION_TEMPLATE_OFF_ID + 4;

pub const COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF: usize = 0x48;
pub const COUNTRY_OCCUPATION_DATA_VTABLE_RVA: usize = 0x296DD00;
pub const COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF: usize = 0x08;
pub const COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF: usize = 0x10;
pub const COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF: usize = 0xD8;
pub const COUNTRY_OCCUPATION_DATA_WINDOW_LEN: usize =
    COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF + 8;
pub const STATE_GARRISON_DATA_VTABLE_RVA: usize = 0x296DCB0;
pub const STATE_GARRISON_DATA_STATE_OFF: usize = 0x50;
pub const STATE_GARRISON_DATA_OCCUPATION_DATA_OFF: usize = 0x58;
pub const STATE_GARRISON_DATA_ARMY_MANPOWER_NEED_OFF: usize = 0x88;
pub const STATE_GARRISON_DATA_STRENGTH_RATIO_OFF: usize = 0xF0;
pub const STATE_GARRISON_DATA_GARRISON_REQUIRED_OFF: usize = 0xF8;
pub const STATE_GARRISON_READ_LEN: usize = 0x100;

pub const HEAP_LO: u64 = 0x10000000000;
pub const HEAP_HI: u64 = 0x7FF000000000;
pub const MAX_PROVINCE_COUNT: u32 = 65536;

pub const MAX_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;
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

    fn read_object_window<const N: usize>(&self, _address: u64) -> Option<[u8; N]> {
        None
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

impl StateRoots {
    pub fn observed_state_rows_bound(&self) -> u32 {
        (self.state_roots.len().min(u32::MAX as usize) as u32)
            .saturating_add(self.invalid_state_links)
            .saturating_add(self.invalid_province_rows)
    }
}

pub fn observed_state_rows_after_currency_check(
    bound: u32,
    found: u32,
    province_array_still_current: bool,
) -> u32 {
    if province_array_still_current {
        bound
    } else {
        bound.saturating_add(found.max(1))
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryCollaborationRelations {
    pub extension: compact::CollaborationExtension,
}

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
    force_full_next_frame: bool,
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
        if compact_contract::compact_status_is_failure(status_flags) {
            let header = build_header(
                elapsed_wall_ms,
                status_flags,
                observed_state_rows,
                garrison_anomaly_rows,
                observed_state_rows,
                &[],
            );
            self.force_full_next_frame = true;
            let payload = compact::encode_payload(&header, &[]);
            return EncodedFrame {
                payload,
                header,
                rows: Vec::new(),
            };
        }
        let invalid_state_rows = invalid_state_rows_for_full_extraction(observed_state_rows, rows);
        let (delta_rows, is_full_frame) = self.delta_rows(rows, invalid_state_rows);
        let mut final_status_flags = status_flags;
        if had_baseline && !is_full_frame {
            final_status_flags |= compact::STATUS_DELTA_DATA;
        }
        let header = build_header(
            elapsed_wall_ms,
            final_status_flags,
            observed_state_rows,
            garrison_anomaly_rows,
            invalid_state_rows,
            &delta_rows,
        );
        let payload = compact::encode_payload(&header, &delta_rows);
        EncodedFrame {
            payload,
            header,
            rows: delta_rows,
        }
    }

    pub fn encode_frame_with_alert(
        &mut self,
        elapsed_wall_ms: u64,
        status_flags: u32,
        observed_state_rows: u32,
        garrison_anomaly_rows: u32,
        rows: &[compact::OccupationStateRow],
        alert: &compact::GarrisonAlertExtension,
    ) -> EncodedFrame {
        let mut frame = self.encode_frame(
            elapsed_wall_ms,
            status_flags,
            observed_state_rows,
            garrison_anomaly_rows,
            rows,
        );
        frame.payload = compact::encode_payload_with_alert(&frame.header, &frame.rows, alert);
        frame
    }

    pub fn encode_frame_with_alert_and_collaboration(
        &mut self,
        elapsed_wall_ms: u64,
        status_flags: u32,
        observed_state_rows: u32,
        garrison_anomaly_rows: u32,
        rows: &[compact::OccupationStateRow],
        alert: &compact::GarrisonAlertExtension,
        collaboration: &compact::CollaborationExtension,
    ) -> EncodedFrame {
        let mut frame = self.encode_frame(
            elapsed_wall_ms,
            status_flags,
            observed_state_rows,
            garrison_anomaly_rows,
            rows,
        );
        frame.payload = compact::encode_payload_with_alert_and_collaboration(
            &frame.header,
            &frame.rows,
            alert,
            collaboration,
        );
        frame
    }

    fn delta_rows(
        &mut self,
        rows: &[compact::OccupationStateRow],
        invalid_state_rows: u32,
    ) -> (Vec<compact::OccupationStateRow>, bool) {
        let present: BTreeSet<u32> = rows.iter().map(|row| row.state_id).collect();
        let root_set_changed = present.len() != self.last_rows.len()
            || self
                .last_rows
                .keys()
                .any(|state_id| !present.contains(state_id));
        if root_set_changed || self.force_full_next_frame || invalid_state_rows != 0 {
            self.last_rows = rows.iter().map(|row| (row.state_id, row.clone())).collect();
            self.emitted_baseline = true;
            self.force_full_next_frame = invalid_state_rows != 0;
            return (rows.to_vec(), true);
        }
        let mut changed = Vec::new();
        for row in rows {
            let old = self.last_rows.insert(row.state_id, row.clone());
            if !self.emitted_baseline || old.as_ref() != Some(row) {
                changed.push(row.clone());
            }
        }
        let was_full = !self.emitted_baseline;
        self.emitted_baseline = true;
        (changed, was_full)
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
    if province_count < 2 || province_count > MAX_PROVINCE_COUNT {
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
        let province_window = mem.read_object_window::<CPROVINCE_WINDOW_LEN>(province_ptr);
        let province_win = province_window.as_ref().map(|window| window.as_slice());
        let province_ok = win_u64(province_win, mem, province_ptr, 0).ok()
            == Some(cprovince_vtable)
            && win_u32(province_win, mem, province_ptr, CPROVINCE_OFF_ID).ok() == Some(province_id);
        if !province_ok {
            invalid_province_rows = invalid_province_rows.saturating_add(1);
            continue;
        }
        province_rows = province_rows.saturating_add(1);
        let state_ptr = match win_u64(province_win, mem, province_ptr, CPROVINCE_OFF_STATE) {
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
        let state_window = mem.read_object_window::<STATE_ROOT_WINDOW_LEN>(state_ptr);
        let state_win = state_window.as_ref().map(|window| window.as_slice());
        let state_ok = win_u64(state_win, mem, state_ptr, 0).ok() == Some(state_vtable);
        let state_id = win_u32(state_win, mem, state_ptr, STATE_OFF_SID).unwrap_or(0);
        if !state_ok || state_id == 0 {
            invalid_state_links = invalid_state_links.saturating_add(1);
            continue;
        }
        if compact_contract::bound_trips::bound_exceeded_u32(
            state_id,
            compact_contract::build_profile::MAX_STATE_ID,
            compact_contract::bound_trips::BOUND_TRIP_MAP_STATE_ID,
        ) {
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

    if province_rows == 0 {
        return Err(ExtractError::InvalidCount {
            name: "authenticated_province_rows",
            value: province_rows,
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
    let country_count_walkable = i32::try_from(country_count).is_ok_and(|signed| signed > 0);
    if !is_heap_ptr(country_table) || country_count == 0 || !country_count_walkable {
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
        let country_window = mem.read_object_window::<COUNTRY_WINDOW_LEN>(country_ptr);
        let country_win = country_window.as_ref().map(|window| window.as_slice());
        let country_ok = win_u64(country_win, mem, country_ptr, 0).ok() == Some(country_vtable)
            && win_u32(country_win, mem, country_ptr, COUNTRY_OFF_TAG_DB_ID).ok() == Some(tag_id);
        if !country_ok {
            continue;
        }
        summary.valid_country_roots = summary.valid_country_roots.saturating_add(1);
        let status_ptr =
            win_u64(country_win, mem, country_ptr, COUNTRY_OFF_OCCUPATION_STATUS).unwrap_or(0);
        let collab_ptr = win_u64(
            country_win,
            mem,
            country_ptr,
            COUNTRY_OFF_COLLABORATION_STATUS,
        )
        .unwrap_or(0);
        let collab_valid =
            is_heap_ptr(collab_ptr) && mem.read_u64(collab_ptr).ok() == Some(collab_vtable);
        if collab_valid {
            summary.valid_collaboration_status_roots =
                summary.valid_collaboration_status_roots.saturating_add(1);
        }

        let status_window = if is_heap_ptr(status_ptr) {
            mem.read_object_window::<COUNTRY_OCCUPATION_STATUS_WINDOW_LEN>(status_ptr)
        } else {
            None
        };
        let status_win = status_window.as_ref().map(|window| window.as_slice());
        let status_valid = is_heap_ptr(status_ptr)
            && win_u64(status_win, mem, status_ptr, 0).ok() == Some(status_vtable)
            && win_u64(
                status_win,
                mem,
                status_ptr,
                COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_OFF,
            )
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

        let law_ptr = win_u64(
            status_win,
            mem,
            status_ptr,
            COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF,
        )
        .unwrap_or(0);
        let law_window = if is_heap_ptr(law_ptr) {
            mem.read_object_window::<OCCUPATION_LAW_WINDOW_LEN>(law_ptr)
        } else {
            None
        };
        let law_win = law_window.as_ref().map(|window| window.as_slice());
        if is_heap_ptr(law_ptr) && win_u64(law_win, mem, law_ptr, 0).ok() == Some(law_vtable) {
            let name = match law_win {
                Some(window) => window
                    .get(OCCUPATION_LAW_NAME_OFF..OCCUPATION_LAW_WINDOW_LEN)
                    .and_then(|blob| read_pdx_string_from_blob(mem, blob)),
                None => read_pdx_string(mem, law_ptr + OCCUPATION_LAW_NAME_OFF as u64),
            };
            if let Some(name) = name {
                rec.default_law_ptr = law_ptr;
                rec.default_law_token_id = fnv1a32(&name);
                summary.valid_default_law_roots = summary.valid_default_law_roots.saturating_add(1);
            }
        }

        let template_ptr = win_u64(
            status_win,
            mem,
            status_ptr,
            COUNTRY_OCCUPATION_STATUS_GARRISON_TEMPLATE_OFF,
        )
        .unwrap_or(0);
        let template_window = if is_heap_ptr(template_ptr) {
            mem.read_object_window::<DIVISION_TEMPLATE_WINDOW_LEN>(template_ptr)
        } else {
            None
        };
        let template_win = template_window.as_ref().map(|window| window.as_slice());
        if is_heap_ptr(template_ptr)
            && win_u64(template_win, mem, template_ptr, 0).ok() == Some(template_vtable)
            && win_u32(template_win, mem, template_ptr, DIVISION_TEMPLATE_OFF_TYPE).ok()
                == Some(DIVISION_TEMPLATE_TYPE)
        {
            let template_id =
                win_u32(template_win, mem, template_ptr, DIVISION_TEMPLATE_OFF_ID).unwrap_or(0);
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

pub fn read_country_collaboration_relations<M: OccupationMemory>(
    mem: &M,
    countries: &BTreeMap<u32, CountryOccupationStatus>,
) -> CountryCollaborationRelations {
    let relation_vtable = mem.module_base() + COUNTRY_COLLABORATION_DATA_VTABLE_RVA as u64;
    let mut rows = Vec::new();
    let mut anomaly_rows = 0u32;
    let mut seen = BTreeSet::new();

    for (&owner_tag_db_id, country) in countries {
        let status_ptr = country.collaboration_status_ptr;
        if !is_heap_ptr(status_ptr) {
            anomaly_rows = anomaly_rows.saturating_add(1);
            continue;
        }
        let status = match mem.read_bytes(status_ptr, 0x58) {
            Ok(status) => status,
            Err(_) => {
                anomaly_rows = anomaly_rows.saturating_add(1);
                continue;
            }
        };
        let row_array = u64_from(&status, COUNTRY_COLLABORATION_STATUS_ROWS_OFF);
        let row_count = u32_from(&status, COUNTRY_COLLABORATION_STATUS_ROW_COUNT_OFF);
        let owner_ptr = u64_from(&status, COUNTRY_COLLABORATION_STATUS_OWNER_OFF);
        if owner_ptr != country.country_ptr
            || row_count > MAX_COLLABORATION_RELATIONS_PER_COUNTRY
            || (row_count != 0 && !is_heap_ptr(row_array))
        {
            anomaly_rows = anomaly_rows.saturating_add(1);
            continue;
        }
        let pointers = if row_count == 0 {
            Vec::new()
        } else {
            match mem.read_bytes(row_array, row_count as usize * 8) {
                Ok(pointers) => pointers,
                Err(_) => {
                    anomaly_rows = anomaly_rows.saturating_add(1);
                    continue;
                }
            }
        };
        for index in 0..row_count as usize {
            let relation_ptr = u64_from(&pointers, index * 8);
            let relation = if is_heap_ptr(relation_ptr) {
                mem.read_bytes(relation_ptr, COUNTRY_COLLABORATION_DATA_LEN)
                    .ok()
            } else {
                None
            };
            let Some(relation) = relation else {
                anomaly_rows = anomaly_rows.saturating_add(1);
                continue;
            };
            let target_tag_db_id = u32_from(&relation, COUNTRY_COLLABORATION_DATA_TARGET_OFF);
            let progress_raw = i64_from(&relation, COUNTRY_COLLABORATION_DATA_PROGRESS_OFF);
            let target = countries.get(&target_tag_db_id);
            let reciprocal = target.and_then(|target| {
                let target_status = mem.read_bytes(target.collaboration_status_ptr, 0x58).ok()?;
                let incoming = u64_from(&target_status, COUNTRY_COLLABORATION_STATUS_INCOMING_OFF);
                let incoming_count = u32_from(
                    &target_status,
                    COUNTRY_COLLABORATION_STATUS_INCOMING_COUNT_OFF,
                );
                if incoming_count > MAX_COLLABORATION_RELATIONS_PER_COUNTRY
                    || (incoming_count != 0 && !is_heap_ptr(incoming))
                {
                    return None;
                }
                let ids = if incoming_count == 0 {
                    Vec::new()
                } else {
                    mem.read_bytes(incoming, incoming_count as usize * 4).ok()?
                };
                Some(
                    (0..incoming_count as usize).any(|incoming_index| {
                        u32_from(&ids, incoming_index * 4) == owner_tag_db_id
                    }),
                )
            });
            if u64_from(&relation, 0) != relation_vtable
                || target_tag_db_id == 0
                || compact_contract::bound_trips::bound_exceeded_u32(
                    target_tag_db_id,
                    MAX_TAG_DB_ID,
                    compact_contract::bound_trips::BOUND_TRIP_COUNTRY_LINK_INDEX,
                )
                || target_tag_db_id == owner_tag_db_id
                || target.is_none()
                || reciprocal != Some(true)
                || !(0..=compact::FIXED_SCALE_RAW as i64).contains(&progress_raw)
                || !seen.insert((owner_tag_db_id, target_tag_db_id))
            {
                anomaly_rows = anomaly_rows.saturating_add(1);
                continue;
            }
            rows.push(compact::CollaborationRelationRow {
                owner_tag_db_id,
                target_tag_db_id,
                progress_raw,
            });
        }
    }

    rows.sort();
    let complete = !countries.is_empty() && anomaly_rows == 0;
    CountryCollaborationRelations {
        extension: compact::CollaborationExtension {
            flags: if complete {
                compact::COLLABORATION_FLAG_COMPLETE
            } else {
                compact::COLLABORATION_FLAG_SOURCE_UNAVAILABLE
            },
            observed_country_rows: if complete {
                countries.len().min(u32::MAX as usize) as u32
            } else {
                0
            },
            anomaly_rows,
            rows: if complete { rows } else { Vec::new() },
        },
    }
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

struct LaneRobinHoodMemory<'a, M: OccupationMemory>(&'a M);

impl<M: OccupationMemory> compact_contract::occupation_robin_hood::RobinHoodMemory
    for LaneRobinHoodMemory<'_, M>
{
    fn read_u8(&self, address: u64) -> Option<u8> {
        self.0.read_bytes(address, 1).ok()?.first().copied()
    }

    fn read_u32(&self, address: u64) -> Option<u32> {
        self.0.read_u32(address).ok()
    }

    fn read_u64(&self, address: u64) -> Option<u64> {
        self.0.read_u64(address).ok()
    }

    fn is_heap_pointer(&self, value: u64) -> bool {
        is_heap_ptr(value)
    }
}

pub fn read_state_garrison_scalars<M: OccupationMemory>(
    mem: &M,
    occupation_status: &BTreeMap<u32, CountryOccupationStatus>,
    state_roots: &BTreeMap<u32, u64>,
) -> GarrisonScalarWalk {
    use compact_contract::occupation_robin_hood as rh;

    let rh_mem = LaneRobinHoodMemory(mem);
    let state_id_by_ptr: BTreeMap<u64, u32> = state_roots
        .iter()
        .map(|(&state_id, &state_ptr)| (state_ptr, state_id))
        .collect();
    let occupation_data_vtable = mem.module_base() + COUNTRY_OCCUPATION_DATA_VTABLE_RVA as u64;
    let garrison_vtable = mem.module_base() + STATE_GARRISON_DATA_VTABLE_RVA as u64;
    let mut by_state: BTreeMap<u32, StateGarrisonScalars> = BTreeMap::new();
    let mut represented_rows: BTreeMap<u32, u32> = BTreeMap::new();
    let mut conflicted: BTreeSet<u32> = BTreeSet::new();
    let mut anomaly_rows = 0u32;

    for (&controller_tag_db_id, status) in occupation_status {
        let Some(container_header) = status
            .occupation_status_ptr
            .checked_add(COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF as u64)
        else {
            anomaly_rows = anomaly_rows.saturating_add(1);
            continue;
        };
        let occupation_entries = match rh::read_robin_hood_table(&rh_mem, container_header) {
            Ok(rh::RobinHoodTable::Empty) => continue,
            Ok(rh::RobinHoodTable::Entries(entries)) => entries,
            Err(_) => {
                anomaly_rows = anomaly_rows.saturating_add(1);
                continue;
            }
        };

        let mut country_staged: Vec<(u32, StateGarrisonScalars)> = Vec::new();
        let mut country_committed_buckets: Vec<rh::RobinHoodEntry> = Vec::new();
        let mut country_untrusted = false;
        let mut country_failed_rows = 0usize;
        let mut country_total_rows = 0usize;
        let mut seen_occupation_data = BTreeSet::new();

        for entry in &occupation_entries {
            let occupation_data_ptr = entry.value;
            if !is_heap_ptr(occupation_data_ptr) {
                country_untrusted = true;
                country_total_rows = country_total_rows.saturating_add(1);
                continue;
            }
            let occupation_data_window =
                mem.read_object_window::<COUNTRY_OCCUPATION_DATA_WINDOW_LEN>(occupation_data_ptr);
            let occupation_data_win = occupation_data_window
                .as_ref()
                .map(|window| window.as_slice());
            if win_u64(occupation_data_win, mem, occupation_data_ptr, 0).ok()
                != Some(occupation_data_vtable)
            {
                country_untrusted = true;
                country_total_rows = country_total_rows.saturating_add(1);
                continue;
            }
            if !seen_occupation_data.insert(occupation_data_ptr) {
                country_untrusted = true;
                continue;
            }
            if win_u64(
                occupation_data_win,
                mem,
                occupation_data_ptr,
                COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF,
            )
            .ok()
                != Some(status.occupation_status_ptr)
            {
                country_untrusted = true;
                country_total_rows = country_total_rows.saturating_add(1);
                continue;
            }

            let Some(garrison_header) = occupation_data_ptr
                .checked_add(COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64)
            else {
                country_failed_rows = country_failed_rows.saturating_add(1);
                country_total_rows = country_total_rows.saturating_add(1);
                continue;
            };
            let garrison_entries = match rh::read_robin_hood_table(&rh_mem, garrison_header) {
                Ok(rh::RobinHoodTable::Empty) => continue,
                Ok(rh::RobinHoodTable::Entries(entries)) => entries,
                Err(_) => {
                    country_failed_rows = country_failed_rows.saturating_add(1);
                    country_total_rows = country_total_rows.saturating_add(1);
                    continue;
                }
            };
            country_total_rows = country_total_rows.saturating_add(garrison_entries.len());

            let occupied_tag_db_id = match win_u32(
                occupation_data_win,
                mem,
                occupation_data_ptr,
                COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF,
            ) {
                Ok(value)
                    if value != 0
                        && compact_contract::bound_trips::gate_u32(
                            value,
                            MAX_TAG_DB_ID,
                            compact_contract::bound_trips::BOUND_TRIP_COUNTRY_LINK_INDEX,
                        )
                        .is_some() =>
                {
                    value
                }
                _ => {
                    country_failed_rows =
                        country_failed_rows.saturating_add(garrison_entries.len());
                    continue;
                }
            };
            let mut record_staged: Vec<(u32, StateGarrisonScalars)> = Vec::new();
            let mut record_untrusted = false;
            let mut record_failed_rows = 0usize;
            let mut seen_garrison = BTreeSet::new();

            for garrison_entry in &garrison_entries {
                let garrison_ptr = garrison_entry.value;
                if !is_heap_ptr(garrison_ptr) {
                    record_untrusted = true;
                    continue;
                }
                if !seen_garrison.insert(garrison_ptr) {
                    record_untrusted = true;
                    continue;
                }
                let Ok(blob) = mem.read_bytes(garrison_ptr, STATE_GARRISON_READ_LEN) else {
                    record_untrusted = true;
                    continue;
                };
                if u64_from(&blob, 0) != garrison_vtable {
                    record_untrusted = true;
                    continue;
                }
                if u64_from(&blob, STATE_GARRISON_DATA_OCCUPATION_DATA_OFF) != occupation_data_ptr {
                    record_untrusted = true;
                    continue;
                }

                let state_ptr = u64_from(&blob, STATE_GARRISON_DATA_STATE_OFF);
                let Some(&state_id) = state_id_by_ptr.get(&state_ptr) else {
                    record_failed_rows = record_failed_rows.saturating_add(1);
                    continue;
                };
                let army_manpower_need =
                    i32_from(&blob, STATE_GARRISON_DATA_ARMY_MANPOWER_NEED_OFF);
                let Ok(strength_ratio_raw) =
                    i32::try_from(i64_from(&blob, STATE_GARRISON_DATA_STRENGTH_RATIO_OFF))
                else {
                    record_failed_rows = record_failed_rows.saturating_add(1);
                    continue;
                };
                let Ok(garrison_required_raw) =
                    i32::try_from(i64_from(&blob, STATE_GARRISON_DATA_GARRISON_REQUIRED_OFF))
                else {
                    record_failed_rows = record_failed_rows.saturating_add(1);
                    continue;
                };
                if army_manpower_need < 0
                    || army_manpower_need > MAX_ABS_FIXED_RAW
                    || strength_ratio_raw < -MAX_ABS_FIXED_RAW
                    || strength_ratio_raw > MAX_ABS_FIXED_RAW
                    || garrison_required_raw < -MAX_ABS_FIXED_RAW
                    || garrison_required_raw > MAX_ABS_FIXED_RAW
                {
                    record_failed_rows = record_failed_rows.saturating_add(1);
                    continue;
                }
                record_staged.push((
                    state_id,
                    StateGarrisonScalars {
                        controller_tag_db_id,
                        occupied_tag_db_id,
                        army_manpower_need,
                        strength_ratio_raw,
                        garrison_required_raw,
                    },
                ));
            }

            if !record_untrusted
                && !garrison_entries
                    .iter()
                    .all(|bucket| rh::bucket_still_live(&rh_mem, bucket))
            {
                record_untrusted = true;
            }
            if record_untrusted {
                country_failed_rows = country_failed_rows.saturating_add(garrison_entries.len());
                continue;
            }
            country_failed_rows = country_failed_rows.saturating_add(record_failed_rows);
            country_staged.extend(record_staged);
            country_committed_buckets.extend(garrison_entries.iter().copied());
        }

        if !country_untrusted
            && !(occupation_entries
                .iter()
                .all(|bucket| rh::bucket_still_live(&rh_mem, bucket))
                && country_committed_buckets
                    .iter()
                    .all(|bucket| rh::bucket_still_live(&rh_mem, bucket)))
        {
            country_untrusted = true;
        }
        if country_untrusted {
            anomaly_rows =
                anomaly_rows.saturating_add(country_total_rows.min(u32::MAX as usize) as u32);
            continue;
        }
        anomaly_rows =
            anomaly_rows.saturating_add(country_failed_rows.min(u32::MAX as usize) as u32);

        for (state_id, record) in country_staged {
            match by_state.get(&state_id) {
                None => {
                    by_state.insert(state_id, record);
                    represented_rows.insert(state_id, 1);
                }
                Some(existing) if *existing == record => {
                    let represented = represented_rows.entry(state_id).or_insert(0);
                    *represented = represented.saturating_add(1);
                }
                Some(_) => {
                    conflicted.insert(state_id);
                    anomaly_rows = anomaly_rows.saturating_add(1);
                }
            }
        }
    }

    for state_id in &conflicted {
        by_state.remove(state_id);
        anomaly_rows =
            anomaly_rows.saturating_add(represented_rows.get(state_id).copied().unwrap_or(0));
    }
    GarrisonScalarWalk {
        by_state,
        anomaly_rows,
    }
}

pub fn build_header(
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_state_rows: u32,
    garrison_anomaly_rows: u32,
    invalid_state_rows: u32,
    rows: &[compact::OccupationStateRow],
) -> compact::OccupationHeader {
    compact::OccupationHeader {
        elapsed_wall_ms,
        status_flags,
        invalid_state_rows,
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

pub fn invalid_state_rows_for_full_extraction(
    observed_state_rows: u32,
    rows: &[compact::OccupationStateRow],
) -> u32 {
    observed_state_rows.saturating_sub(rows.len().min(u32::MAX as usize) as u32)
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
        invalid_state_rows_for_full_extraction(observed_state_rows, rows),
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
    if compact_contract::bound_trips::bound_exceeded_u32(
        owner,
        MAX_TAG_DB_ID,
        compact_contract::bound_trips::BOUND_TRIP_COUNTRY_LINK_INDEX,
    ) || compact_contract::bound_trips::bound_exceeded_u32(
        controller,
        MAX_TAG_DB_ID,
        compact_contract::bound_trips::BOUND_TRIP_COUNTRY_LINK_INDEX,
    ) {
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
    read_pdx_string_from_blob(mem, &blob)
}

fn read_pdx_string_from_blob<M: OccupationMemory>(mem: &M, blob: &[u8]) -> Option<String> {
    let size = u64_from(&blob, 0x10) as usize;
    let cap = u64_from(&blob, 0x18);
    if size == 0 || size > 255 {
        return None;
    }
    let borrowed;
    let owned;
    let raw = if cap == 15 {
        borrowed = blob.get(0..size.min(16))?;
        borrowed
    } else {
        let ptr = u64_from(&blob, 0);
        if !is_heap_address(ptr) {
            return None;
        }
        owned = mem.read_bytes(ptr, size).ok()?;
        &owned
    };
    if !raw.iter().all(|byte| (32..127).contains(byte)) {
        return None;
    }
    std::str::from_utf8(raw).ok().map(str::to_string)
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

pub fn encode_rows_with_alert(
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_state_rows: u32,
    garrison_anomaly_rows: u32,
    rows: &[compact::OccupationStateRow],
    alert: &compact::GarrisonAlertExtension,
) -> EncodedFrame {
    let mut frame = encode_rows(
        elapsed_wall_ms,
        status_flags,
        observed_state_rows,
        garrison_anomaly_rows,
        rows,
    );
    frame.payload = compact::encode_payload_with_alert(&frame.header, &frame.rows, alert);
    frame
}

fn i64_from(data: &[u8], offset: usize) -> i64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&data[offset..offset + 8]);
    i64::from_le_bytes(bytes)
}

fn win_u64<M: OccupationMemory>(
    win: Option<&[u8]>,
    mem: &M,
    addr: u64,
    off: usize,
) -> Result<u64, ExtractError> {
    match win {
        Some(buf) => Ok(u64_from(buf, off)),
        None => mem.read_u64(addr + off as u64),
    }
}

fn win_u32<M: OccupationMemory>(
    win: Option<&[u8]>,
    mem: &M,
    addr: u64,
    off: usize,
) -> Result<u32, ExtractError> {
    match win {
        Some(buf) => Ok(u32_from(buf, off)),
        None => mem.read_u32(addr + off as u64),
    }
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

        fn write_i64(&mut self, address: u64, value: i64) {
            self.write_bytes(address, &value.to_le_bytes());
        }

        fn write_u8(&mut self, address: u64, value: u8) {
            self.write_bytes(address, &[value]);
        }

        fn write_robin_hood_table(
            &mut self,
            header: u64,
            buckets: u64,
            count: u32,
            capacity: u32,
            max_distance: u8,
        ) {
            self.write_u64(header, buckets);
            self.write_u32(header + ROBIN_HOOD_SIZE_OFF, count);
            self.write_u32(header + ROBIN_HOOD_CAP_OFF, capacity);
            self.write_u8(header + ROBIN_HOOD_MAX_DISTANCE_OFF, max_distance);
            let ceiling = u64::from(capacity) + u64::from(max_distance) + 1;
            self.write_zeroes(buckets, ((ceiling + 1) * ROBIN_HOOD_STRIDE) as usize);
            self.write_u8(
                buckets + ceiling * ROBIN_HOOD_STRIDE + ROBIN_HOOD_DISTANCE_PLUS_1_OFF,
                ROBIN_HOOD_SENTINEL,
            );
        }

        fn write_robin_hood_bucket(&mut self, buckets: u64, index: u64, key: u32, value: u64) {
            let bucket = buckets + index * ROBIN_HOOD_STRIDE;
            self.write_u8(bucket + ROBIN_HOOD_DISTANCE_PLUS_1_OFF, 1);
            self.write_u32(bucket + ROBIN_HOOD_KEY_OFF, key);
            self.write_u64(bucket + ROBIN_HOOD_VALUE_OFF, value);
        }

        fn destroy_robin_hood_bucket(&mut self, buckets: u64, index: u64) {
            self.write_u8(
                buckets + index * ROBIN_HOOD_STRIDE + ROBIN_HOOD_DISTANCE_PLUS_1_OFF,
                0,
            );
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

    struct ContiguousFakeMemory {
        inner: FakeMemory,
        refused_windows: BTreeSet<u64>,
    }

    impl ContiguousFakeMemory {
        fn new(inner: FakeMemory) -> Self {
            Self {
                inner,
                refused_windows: BTreeSet::new(),
            }
        }

        fn refusing(mut self, address: u64) -> Self {
            self.refused_windows.insert(address);
            self
        }
    }

    impl OccupationMemory for ContiguousFakeMemory {
        fn module_base(&self) -> u64 {
            self.inner.module_base()
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            self.inner.read_bytes(address, len)
        }

        fn read_object_window<const N: usize>(&self, address: u64) -> Option<[u8; N]> {
            if self.refused_windows.contains(&address) {
                return None;
            }
            self.inner.read_bytes(address, N).ok()?.try_into().ok()
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
    fn a_country_table_past_the_old_ceiling_still_yields_its_occupation_rows() {
        let mut mem = fake_memory(DIVISION_TEMPLATE_TYPE);
        const BIG_TABLE: u64 = 0x2200000A000;
        const BIG_SLOT: u32 = 1_500;
        mem.write_zeroes(BIG_TABLE, 2_000 * 8);
        mem.write_u64(GAME_STATE + CGAMESTATE_COUNTRY_TABLE as u64, BIG_TABLE);
        mem.write_u32(GAME_STATE + CGAMESTATE_COUNTRY_COUNT as u64, 2_000);
        mem.write_u64(BIG_TABLE + u64::from(BIG_SLOT) * 8, COUNTRY_2);
        mem.write_u32(COUNTRY_2 + COUNTRY_OFF_TAG_DB_ID as u64, BIG_SLOT);

        let (status, summary) =
            read_country_occupation_status(&mem, GAME_STATE).expect("the walk runs");
        assert_eq!(summary.country_count, 2_000);
        assert!(
            status.contains_key(&BIG_SLOT),
            "the occupied country at slot {BIG_SLOT} is past the old 1024 ceiling \
             and must still be found"
        );
    }

    #[test]
    fn every_refused_tag_names_the_link_vector_not_the_country_database() {
        use compact_contract::bound_trips as bt;

        const OVER: u32 = MAX_TAG_DB_ID + 1;
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);

        let cases: Vec<(&str, Box<dyn Fn() -> ()>)> = vec![
            (
                "state owner (CState+0xC8)",
                Box::new(|| {
                    let mut mem = fake_memory(DIVISION_TEMPLATE_TYPE);
                    mem.write_u32(STATE_10 + STATE_OFF_OWNER as u64, OVER);
                    let extracted =
                        extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
                    assert!(extracted.rows.is_empty(), "the row is still discarded");
                }),
            ),
            (
                "state controller (CState+0xCC)",
                Box::new(|| {
                    let mut mem = fake_memory(DIVISION_TEMPLATE_TYPE);
                    mem.write_u32(STATE_10 + STATE_OFF_CTRL as u64, OVER);
                    let extracted =
                        extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
                    assert!(extracted.rows.is_empty(), "the row is still discarded");
                }),
            ),
            (
                "occupation-data occupied tag (CCountryOccupationData+0x10)",
                Box::new(|| {
                    let mem = fake_memory_with_garrison(OVER);
                    let extracted =
                        extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
                    assert!(
                        extracted.garrison_anomaly_rows > 0,
                        "the garrison row is still refused"
                    );
                }),
            ),
        ];

        for (label, run) in cases {
            bt::drain_frame_trips();
            run();
            let drained = bt::drain_frame_trips();
            assert_eq!(
                drained[bt::BOUND_TRIP_COUNTRY_LINK_INDEX],
                (1, u64::from(OVER)),
                "{label}: the link-vector bound must carry the trip and the value"
            );
            assert_eq!(
                drained[bt::BOUND_TRIP_TAG_DB_ID_1024].0,
                0,
                "{label}: a link-index refusal must never name the country database"
            );
        }

        {
            const OWNER: u64 = 0x22000021000;
            const TARGET: u64 = 0x22000022000;
            const OWNER_STATUS: u64 = 0x22000023000;
            const RELATION_POINTERS: u64 = 0x22000024000;
            const RELATION: u64 = 0x22000025000;

            let mut mem = FakeMemory::default();
            mem.write_zeroes(OWNER_STATUS, 0x58);
            mem.write_zeroes(RELATION_POINTERS, 8);
            mem.write_zeroes(RELATION, COUNTRY_COLLABORATION_DATA_LEN);
            mem.write_u64(
                OWNER_STATUS + COUNTRY_COLLABORATION_STATUS_ROWS_OFF as u64,
                RELATION_POINTERS,
            );
            mem.write_u32(
                OWNER_STATUS + COUNTRY_COLLABORATION_STATUS_ROW_COUNT_OFF as u64,
                1,
            );
            mem.write_u64(
                OWNER_STATUS + COUNTRY_COLLABORATION_STATUS_OWNER_OFF as u64,
                OWNER,
            );
            mem.write_u64(RELATION_POINTERS, RELATION);
            mem.write_u64(
                RELATION,
                MODULE_BASE + COUNTRY_COLLABORATION_DATA_VTABLE_RVA as u64,
            );
            mem.write_u32(
                RELATION + COUNTRY_COLLABORATION_DATA_TARGET_OFF as u64,
                OVER,
            );
            mem.write_i64(
                RELATION + COUNTRY_COLLABORATION_DATA_PROGRESS_OFF as u64,
                30_000,
            );

            let countries = BTreeMap::from([(
                24,
                CountryOccupationStatus {
                    country_ptr: OWNER,
                    collaboration_status_ptr: OWNER_STATUS,
                    ..CountryOccupationStatus::default()
                },
            )]);
            let _ = TARGET;

            bt::drain_frame_trips();
            let extension = read_country_collaboration_relations(&mem, &countries).extension;
            assert!(
                extension.rows.is_empty(),
                "the relation is still discarded: this change is attribution only"
            );
            assert_eq!(extension.anomaly_rows, 1);

            let drained = bt::drain_frame_trips();
            assert_eq!(
                drained[bt::BOUND_TRIP_COUNTRY_LINK_INDEX],
                (1, u64::from(OVER)),
                "collaboration target: the refusal used to leave no trace at all"
            );
            assert_eq!(drained[bt::BOUND_TRIP_TAG_DB_ID_1024].0, 0);
        }
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
    fn collaboration_reader_requires_reciprocal_owner_target_join() {
        const OWNER: u64 = 0x2200000A000;
        const TARGET: u64 = 0x2200000B000;
        const OWNER_STATUS: u64 = 0x2200000C000;
        const TARGET_STATUS: u64 = 0x2200000D000;
        const RELATION_POINTERS: u64 = 0x2200000E000;
        const RELATION: u64 = 0x2200000F000;
        const INCOMING_IDS: u64 = 0x22000010000;

        let mut mem = FakeMemory::default();
        mem.write_zeroes(OWNER_STATUS, 0x58);
        mem.write_zeroes(TARGET_STATUS, 0x58);
        mem.write_zeroes(RELATION_POINTERS, 8);
        mem.write_zeroes(RELATION, COUNTRY_COLLABORATION_DATA_LEN);
        mem.write_zeroes(INCOMING_IDS, 4);

        mem.write_u64(
            OWNER_STATUS + COUNTRY_COLLABORATION_STATUS_ROWS_OFF as u64,
            RELATION_POINTERS,
        );
        mem.write_u32(
            OWNER_STATUS + COUNTRY_COLLABORATION_STATUS_ROW_COUNT_OFF as u64,
            1,
        );
        mem.write_u64(
            OWNER_STATUS + COUNTRY_COLLABORATION_STATUS_OWNER_OFF as u64,
            OWNER,
        );
        mem.write_u64(RELATION_POINTERS, RELATION);
        mem.write_u64(
            RELATION,
            MODULE_BASE + COUNTRY_COLLABORATION_DATA_VTABLE_RVA as u64,
        );
        mem.write_u32(RELATION + COUNTRY_COLLABORATION_DATA_TARGET_OFF as u64, 1);
        mem.write_i64(
            RELATION + COUNTRY_COLLABORATION_DATA_PROGRESS_OFF as u64,
            30_000,
        );

        mem.write_u64(
            TARGET_STATUS + COUNTRY_COLLABORATION_STATUS_OWNER_OFF as u64,
            TARGET,
        );
        mem.write_u64(
            TARGET_STATUS + COUNTRY_COLLABORATION_STATUS_INCOMING_OFF as u64,
            INCOMING_IDS,
        );
        mem.write_u32(
            TARGET_STATUS + COUNTRY_COLLABORATION_STATUS_INCOMING_COUNT_OFF as u64,
            1,
        );
        mem.write_u32(INCOMING_IDS, 24);

        let countries = BTreeMap::from([
            (
                1,
                CountryOccupationStatus {
                    country_ptr: TARGET,
                    collaboration_status_ptr: TARGET_STATUS,
                    ..CountryOccupationStatus::default()
                },
            ),
            (
                24,
                CountryOccupationStatus {
                    country_ptr: OWNER,
                    collaboration_status_ptr: OWNER_STATUS,
                    ..CountryOccupationStatus::default()
                },
            ),
        ]);

        let complete = read_country_collaboration_relations(&mem, &countries).extension;
        assert_eq!(complete.flags, compact::COLLABORATION_FLAG_COMPLETE);
        assert_eq!(complete.observed_country_rows, 2);
        assert_eq!(complete.anomaly_rows, 0);
        assert_eq!(
            complete.rows,
            vec![compact::CollaborationRelationRow {
                owner_tag_db_id: 24,
                target_tag_db_id: 1,
                progress_raw: 30_000,
            }]
        );

        mem.write_u32(INCOMING_IDS, 0);
        let unavailable = read_country_collaboration_relations(&mem, &countries).extension;
        assert_eq!(
            unavailable.flags,
            compact::COLLABORATION_FLAG_SOURCE_UNAVAILABLE
        );
        assert_eq!(unavailable.observed_country_rows, 0);
        assert_eq!(unavailable.anomaly_rows, 1);
        assert!(unavailable.rows.is_empty());
    }

    #[test]
    fn bulk_windows_extract_rows_match_per_field_path() {
        let mem = fake_memory_with_garrison(1);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);

        let per_field = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        let bulk =
            extract_rows_from_state_roots(&ContiguousFakeMemory::new(mem), GAME_STATE, &roots)
                .unwrap();

        assert_eq!(bulk, per_field);
        let per_encoded = encode_rows(
            25,
            0,
            per_field.observed_state_rows,
            per_field.garrison_anomaly_rows,
            &per_field.rows,
        );
        let bulk_encoded = encode_rows(
            25,
            0,
            bulk.observed_state_rows,
            bulk.garrison_anomaly_rows,
            &bulk.rows,
        );
        assert_eq!(bulk_encoded.payload, per_encoded.payload);
    }

    #[test]
    fn refused_object_windows_fall_back_to_per_field_path() {
        let mem = fake_memory_with_garrison(1);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);

        let per_field = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        let refused = extract_rows_from_state_roots(
            &ContiguousFakeMemory::new(mem)
                .refusing(COUNTRY_2)
                .refusing(STATUS_2)
                .refusing(OCC_DATA),
            GAME_STATE,
            &roots,
        )
        .unwrap();

        assert_eq!(refused, per_field);
        let per_encoded = encode_rows(
            25,
            0,
            per_field.observed_state_rows,
            per_field.garrison_anomaly_rows,
            &per_field.rows,
        );
        let refused_encoded = encode_rows(
            25,
            0,
            refused.observed_state_rows,
            refused.garrison_anomaly_rows,
            &refused.rows,
        );
        assert_eq!(refused_encoded.payload, per_encoded.payload);
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

    fn census_row() -> compact::OccupationStateRow {
        compact::OccupationStateRow {
            state_id: 1,
            owner_tag_db_id: 1,
            controller_tag_db_id: 1,
            occupied_country_tag_db_id: 0,
            resistance_raw: 0,
            compliance_raw: 0,
            resistance_speed_raw: 0,
            compliance_speed_raw: 0,
            garrison_manpower_need: 0,
            garrison_strength_ratio_raw: 0,
            garrison_required_raw: 0,
            occupation_law_token_id: 0,
            garrison_template_id: 0,
            flags: 0,
        }
    }

    #[test]
    fn the_observed_root_bound_counts_the_states_discovery_may_have_missed() {
        let mut state_roots = BTreeMap::new();
        state_roots.insert(10u32, STATE_10);
        state_roots.insert(11u32, STATE_10);
        let roots = |links: u32, provinces: u32| StateRoots {
            game_state: GAME_STATE,
            province_array: 0,
            province_count: 0,
            province_rows: 0,
            invalid_province_rows: provinces,
            invalid_state_links: links,
            state_roots: state_roots.clone(),
        };
        assert_eq!(roots(0, 0).observed_state_rows_bound(), 2);
        assert_eq!(roots(1, 0).observed_state_rows_bound(), 3);
        assert_eq!(roots(0, 1).observed_state_rows_bound(), 3);
        assert_eq!(roots(2, 3).observed_state_rows_bound(), 7);
        let emitted = [census_row(), census_row()];
        assert_eq!(
            invalid_state_rows_for_full_extraction(
                roots(0, 0).observed_state_rows_bound(),
                &emitted
            ),
            0,
            "a clean walk drops nothing"
        );
        assert_eq!(
            invalid_state_rows_for_full_extraction(
                roots(1, 0).observed_state_rows_bound(),
                &emitted
            ),
            1,
            "one unfollowable state link is one dropped state"
        );
        assert_eq!(
            invalid_state_rows_for_full_extraction(
                roots(0, 1).observed_state_rows_bound(),
                &emitted
            ),
            1,
            "one unreadable province row is one possibly-dropped state"
        );
    }

    #[test]
    fn a_replaced_province_array_refuses_the_census_without_failing_the_record() {
        assert_eq!(
            observed_state_rows_after_currency_check(2, 2, true),
            2,
            "a current array publishes the bound unchanged"
        );
        let emitted = [census_row(), census_row()];
        assert_eq!(
            invalid_state_rows_for_full_extraction(
                observed_state_rows_after_currency_check(2, 2, true),
                &emitted
            ),
            0
        );
        assert_eq!(observed_state_rows_after_currency_check(2, 2, false), 4);
        assert_eq!(
            invalid_state_rows_for_full_extraction(
                observed_state_rows_after_currency_check(2, 2, false),
                &emitted
            ),
            2,
            "a replaced array refuses the census"
        );
        assert_eq!(
            invalid_state_rows_for_full_extraction(
                observed_state_rows_after_currency_check(0, 0, false),
                &[]
            ),
            1
        );
    }

    #[test]
    fn a_frame_that_lost_a_state_root_is_emitted_full_not_as_a_delta() {
        let row = |state_id: u32, resistance: i32| compact::OccupationStateRow {
            state_id,
            resistance_raw: resistance,
            ..census_row()
        };
        let mut tracker = CompactOccupationResistanceComplianceTracker::default();
        let baseline = tracker.encode_frame(1, 0, 2, 0, &[row(1, 10), row(2, 20)]);
        assert_eq!(baseline.header.status_flags & compact::STATUS_DELTA_DATA, 0);
        assert_eq!(baseline.rows.len(), 2);

        let delta = tracker.encode_frame(2, 0, 2, 0, &[row(1, 11), row(2, 20)]);
        assert_ne!(delta.header.status_flags & compact::STATUS_DELTA_DATA, 0);
        assert_eq!(delta.rows.len(), 1);

        let substituted = tracker.encode_frame(3, 0, 2, 0, &[row(1, 11), row(3, 30)]);
        assert_eq!(
            substituted.header.status_flags & compact::STATUS_DELTA_DATA,
            0,
            "a lost root cannot be expressed as a delta"
        );
        let ids: Vec<u32> = substituted.rows.iter().map(|r| r.state_id).collect();
        assert_eq!(ids, vec![1, 3]);
        assert_eq!(
            substituted.header.invalid_state_rows, 0,
            "the walk was clean"
        );

        let after = tracker.encode_frame(4, 0, 2, 0, &[row(1, 11), row(3, 30)]);
        assert_ne!(after.header.status_flags & compact::STATUS_DELTA_DATA, 0);
        assert!(after.rows.is_empty());
    }

    #[test]
    fn every_producer_visible_event_that_retires_the_census_claim_forces_a_full_frame() {
        let row = |state_id: u32, resistance: i32| compact::OccupationStateRow {
            state_id,
            resistance_raw: resistance,
            ..census_row()
        };
        let is_delta =
            |frame: &EncodedFrame| frame.header.status_flags & compact::STATUS_DELTA_DATA != 0;

        let mut tracker = CompactOccupationResistanceComplianceTracker::default();
        assert!(!is_delta(&tracker.encode_frame(
            1,
            0,
            2,
            0,
            &[row(1, 10), row(2, 20)]
        )));
        let failed = tracker.encode_frame(
            2,
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE
                | compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            0,
            0,
            &[],
        );
        assert!(failed.rows.is_empty());
        let recovered = tracker.encode_frame(3, 0, 2, 0, &[row(1, 10), row(2, 20)]);
        assert!(
            !is_delta(&recovered),
            "the walk after a failed record must restate the world"
        );
        assert_eq!(recovered.rows.len(), 2);
        assert!(is_delta(&tracker.encode_frame(
            4,
            0,
            2,
            0,
            &[row(1, 10), row(2, 20)]
        )));

        let mut tracker = CompactOccupationResistanceComplianceTracker::default();
        assert!(!is_delta(&tracker.encode_frame(
            1,
            0,
            2,
            0,
            &[row(1, 10), row(2, 20)]
        )));
        let grown = tracker.encode_frame(2, 0, 3, 0, &[row(1, 10), row(2, 20), row(3, 30)]);
        assert!(!is_delta(&grown), "a world that grew must restate itself");
        assert_eq!(grown.rows.len(), 3);
        assert!(is_delta(&tracker.encode_frame(
            3,
            0,
            3,
            0,
            &[row(1, 10), row(2, 20), row(3, 30)]
        )));

        let mut tracker = CompactOccupationResistanceComplianceTracker::default();
        assert!(!is_delta(&tracker.encode_frame(
            1,
            0,
            2,
            0,
            &[row(1, 10), row(2, 20)]
        )));
        let unclean = tracker.encode_frame(2, 0, 3, 0, &[row(1, 10), row(2, 20)]);
        assert_eq!(unclean.header.invalid_state_rows, 1);
        assert!(
            !is_delta(&unclean),
            "an incomplete walk must restate the world"
        );
        let still_unclean = tracker.encode_frame(3, 0, 3, 0, &[row(1, 10), row(2, 20)]);
        assert!(!is_delta(&still_unclean));
        let clean = tracker.encode_frame(4, 0, 2, 0, &[row(1, 10), row(2, 20)]);
        assert_eq!(clean.header.invalid_state_rows, 0);
        assert!(
            !is_delta(&clean),
            "the recovering frame is the new baseline"
        );
        assert!(is_delta(&tracker.encode_frame(
            5,
            0,
            2,
            0,
            &[row(1, 10), row(2, 20)]
        )));
    }

    #[test]
    fn a_delta_frames_drop_count_is_the_walks_not_the_deltas() {
        let mem = fake_memory(DIVISION_TEMPLATE_TYPE);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.invalid_state_rows, 0, "the fixture walk is clean");
        assert_eq!(extracted.rows.len(), 1);

        let mut tracker = CompactOccupationResistanceComplianceTracker::default();
        let baseline = tracker.encode_frame(1, 0, 1, 0, &extracted.rows);
        assert_eq!(baseline.header.invalid_state_rows, 0);
        assert_eq!(baseline.rows.len(), 1, "a first frame emits every row");

        let delta = tracker.encode_frame(2, 0, 1, 0, &extracted.rows);
        assert!(delta.rows.is_empty(), "an unchanged frame emits no rows");
        assert_eq!(
            delta.header.invalid_state_rows, 0,
            "the drop count describes the walk, not the changed subset"
        );
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
    fn a_failure_class_frame_does_not_latch_the_delta_baseline() {
        let mem = fake_memory(DIVISION_TEMPLATE_TYPE);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        let mut tracker = CompactOccupationResistanceComplianceTracker::default();

        let failed = tracker.encode_frame(
            1,
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE
                | compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            0,
            0,
            &[],
        );
        assert!(failed.rows.is_empty());
        assert_eq!(
            failed.header.status_flags & compact::STATUS_DELTA_DATA,
            0,
            "a failed frame is not a delta"
        );

        let recovered =
            tracker.encode_frame(2, 0, extracted.observed_state_rows, 0, &extracted.rows);
        assert_eq!(
            recovered.rows.len(),
            1,
            "the recovered frame must carry every row, not a delta"
        );
        assert_eq!(
            recovered.header.status_flags & compact::STATUS_DELTA_DATA,
            0,
            "the recovered frame must not be badged DELTA_DATA; the fold has no baseline to \
             patch and would reject it as delta_before_baseline"
        );
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
    const OCC_DATA_B: u64 = 0x2200000B800;
    const GARRISON_CONTAINER: u64 = 0x2200000C000;
    const GARRISON_CONTAINER_B: u64 = 0x2200000C800;
    const GARRISON_ROW: u64 = 0x2200000D000;
    const GARRISON_ROW_B: u64 = 0x2200000E000;
    const STATE_11: u64 = 0x2200000F000;

    fn write_second_state(mem: &mut FakeMemory) {
        mem.write_zeroes(STATE_11, STATE_OFF_COMPLIANCE_SPEED + 4);
        mem.write_u64(STATE_11, MODULE_BASE + STATE_VTABLE_RVA as u64);
        mem.write_u32(STATE_11 + STATE_OFF_SID as u64, 11);
        mem.write_u32(STATE_11 + STATE_OFF_OWNER as u64, 1);
        mem.write_u32(STATE_11 + STATE_OFF_CTRL as u64, 2);
        mem.write_i32(STATE_11 + STATE_OFF_RESISTANCE as u64, 90_000);
        mem.write_i32(STATE_11 + STATE_OFF_RESISTANCE_SPEED as u64, 400);
        mem.write_i32(STATE_11 + STATE_OFF_COMPLIANCE as u64, 1_000_000);
        mem.write_i32(STATE_11 + STATE_OFF_COMPLIANCE_SPEED as u64, -100);
    }

    const OCC_DATA_HEADER: u64 =
        STATUS_2 + COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF as u64;

    const ROBIN_HOOD_SIZE_OFF: u64 = 0x08;
    const ROBIN_HOOD_CAP_OFF: u64 = 0x0C;
    const ROBIN_HOOD_MAX_DISTANCE_OFF: u64 = 0x10;
    const ROBIN_HOOD_STRIDE: u64 = 0x18;
    const ROBIN_HOOD_DISTANCE_PLUS_1_OFF: u64 = 0x04;
    const ROBIN_HOOD_KEY_OFF: u64 = 0x08;
    const ROBIN_HOOD_VALUE_OFF: u64 = 0x10;
    const ROBIN_HOOD_SENTINEL: u8 = 0xFF;

    #[test]
    fn the_fixture_bucket_geometry_matches_the_production_geometry() {
        use compact_contract::build_profile as bp;
        assert_eq!(ROBIN_HOOD_SIZE_OFF, bp::DOMAIN66_ROBIN_HOOD_SIZE_OFF as u64);
        assert_eq!(ROBIN_HOOD_CAP_OFF, bp::DOMAIN66_ROBIN_HOOD_CAP_OFF as u64);
        assert_eq!(
            ROBIN_HOOD_MAX_DISTANCE_OFF,
            bp::DOMAIN66_ROBIN_HOOD_MAX_DISTANCE_OFF as u64
        );
        assert_eq!(
            ROBIN_HOOD_STRIDE,
            bp::DOMAIN66_ROBIN_HOOD_BUCKET_STRIDE as u64
        );
        assert_eq!(
            ROBIN_HOOD_DISTANCE_PLUS_1_OFF,
            bp::DOMAIN66_ROBIN_HOOD_DISTANCE_PLUS_1_OFF as u64
        );
        assert_eq!(ROBIN_HOOD_KEY_OFF, bp::DOMAIN66_ROBIN_HOOD_KEY_OFF as u64);
        assert_eq!(
            ROBIN_HOOD_VALUE_OFF,
            bp::DOMAIN66_ROBIN_HOOD_VALUE_OFF as u64
        );
        assert_eq!(
            ROBIN_HOOD_SENTINEL,
            bp::DOMAIN66_ROBIN_HOOD_ITERATOR_SENTINEL_DISTANCE
        );
    }

    fn write_garrison_row(
        mem: &mut FakeMemory,
        row_ptr: u64,
        occupation_data_ptr: u64,
        state_ptr: u64,
        manpower_need: i32,
        strength_ratio_raw: i64,
        garrison_required_raw: i64,
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
        mem.write_i64(
            row_ptr + STATE_GARRISON_DATA_STRENGTH_RATIO_OFF as u64,
            strength_ratio_raw,
        );
        mem.write_i64(
            row_ptr + STATE_GARRISON_DATA_GARRISON_REQUIRED_OFF as u64,
            garrison_required_raw,
        );
    }

    fn write_occupation_data_record(
        mem: &mut FakeMemory,
        record: u64,
        parent_status: u64,
        occupied_tag: u32,
        garrison_buckets: u64,
        garrison_capacity: u32,
    ) {
        mem.write_zeroes(
            record,
            COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF + 8,
        );
        mem.write_u64(
            record,
            MODULE_BASE + COUNTRY_OCCUPATION_DATA_VTABLE_RVA as u64,
        );
        mem.write_u64(
            record + COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF as u64,
            parent_status,
        );
        mem.write_u32(
            record + COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF as u64,
            occupied_tag,
        );
        mem.write_robin_hood_table(
            record + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            garrison_buckets,
            0,
            garrison_capacity,
            2,
        );
    }

    fn fake_memory_with_garrison(occupied_tag: u32) -> FakeMemory {
        let mut mem = fake_memory(DIVISION_TEMPLATE_TYPE);
        mem.write_robin_hood_table(OCC_DATA_HEADER, OCC_DATA_CONTAINER, 1, 8, 2);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 1, occupied_tag, OCC_DATA);
        write_occupation_data_record(
            &mut mem,
            OCC_DATA,
            STATUS_2,
            occupied_tag,
            GARRISON_CONTAINER,
            8,
        );
        mem.write_robin_hood_table(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER,
            1,
            8,
            2,
        );
        mem.write_robin_hood_bucket(GARRISON_CONTAINER, 1, 10, GARRISON_ROW);
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
    fn garrison_i64_scalar_outside_wire_range_fails_closed() {
        let mut mem = fake_memory_with_garrison(1);
        mem.write_i64(
            GARRISON_ROW + STATE_GARRISON_DATA_STRENGTH_RATIO_OFF as u64,
            i64::from(i32::MAX) + 1,
        );
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.garrison_anomaly_rows, 1);
        let row = &extracted.rows[0];
        assert_eq!(row.garrison_strength_ratio_raw, 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);
    }

    #[test]
    fn garrison_occupied_grouping_tag_differs_from_owner_still_attaches() {
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
        mem.write_robin_hood_table(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER,
            2,
            8,
            2,
        );
        mem.write_robin_hood_bucket(GARRISON_CONTAINER, 1, 10, GARRISON_ROW);
        mem.write_robin_hood_bucket(GARRISON_CONTAINER, 4, 11, GARRISON_ROW_B);
        write_garrison_row(&mut mem, GARRISON_ROW_B, OCC_DATA, STATE_10, 9_999, 1, 2);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.garrison_anomaly_rows, 2);
        let row = &extracted.rows[0];
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);
        assert_eq!(row.garrison_manpower_need, 0);
    }

    #[test]
    fn garrison_row_with_foreign_backref_fails_closed_and_is_counted() {
        let mut mem = fake_memory_with_garrison(1);
        mem.write_u64(
            GARRISON_ROW + STATE_GARRISON_DATA_OCCUPATION_DATA_OFF as u64,
            GARRISON_ROW_B,
        );
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.garrison_anomaly_rows, 1);
        let row = &extracted.rows[0];
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);
    }

    #[test]
    fn one_object_in_two_live_buckets_fails_closed() {
        let mut mem = fake_memory_with_garrison(1);
        mem.write_robin_hood_table(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER,
            2,
            8,
            2,
        );
        mem.write_robin_hood_bucket(GARRISON_CONTAINER, 1, 10, GARRISON_ROW);
        mem.write_robin_hood_bucket(GARRISON_CONTAINER, 6, 11, GARRISON_ROW);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(
            extracted.garrison_anomaly_rows, 2,
            "the duplicate itself, plus the row already staged from the same \
             self-contradicting table"
        );
        let row = &extracted.rows[0];
        assert!(
            row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0,
            "no row from a self-contradicting table may be published"
        );
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);
        assert_eq!(row.garrison_manpower_need, 0);
    }

    struct DestroyAfterFirstSample {
        inner: FakeMemory,
        occupancy_byte: u64,
        samples: std::cell::Cell<u32>,
    }

    impl OccupationMemory for DestroyAfterFirstSample {
        fn module_base(&self) -> u64 {
            self.inner.module_base()
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            if address == self.occupancy_byte && len == 1 {
                let seen = self.samples.get();
                self.samples.set(seen + 1);
                if seen > 0 {
                    return Ok(vec![0u8]);
                }
            }
            self.inner.read_bytes(address, len)
        }
    }

    #[test]
    fn a_garrison_bucket_destroyed_after_the_walk_sampled_it_publishes_nothing() {
        let mem = DestroyAfterFirstSample {
            inner: fake_memory_with_garrison(1),
            occupancy_byte: GARRISON_CONTAINER + ROBIN_HOOD_STRIDE + ROBIN_HOOD_DISTANCE_PLUS_1_OFF,
            samples: std::cell::Cell::new(0),
        };
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        let row = &extracted.rows[0];
        assert_eq!(
            row.garrison_manpower_need, 0,
            "the entry was destroyed while we were reading it; publishing its \
             scalars would attribute them to something that no longer exists"
        );
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);
        assert_eq!(extracted.garrison_anomaly_rows, 1);
    }

    #[test]
    fn an_occupation_destroyed_after_the_walk_sampled_it_publishes_nothing() {
        let mem = DestroyAfterFirstSample {
            inner: fake_memory_with_garrison(1),
            occupancy_byte: OCC_DATA_CONTAINER + ROBIN_HOOD_STRIDE + ROBIN_HOOD_DISTANCE_PLUS_1_OFF,
            samples: std::cell::Cell::new(0),
        };
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        let row = &extracted.rows[0];
        assert_eq!(row.garrison_manpower_need, 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert_eq!(extracted.garrison_anomaly_rows, 1);
    }

    #[test]
    fn a_destroyed_occupation_ahead_of_the_live_one_publishes_nothing() {
        let mut mem = fake_memory_with_garrison(1);
        write_second_state(&mut mem);
        write_occupation_data_record(&mut mem, OCC_DATA_B, STATUS_2, 2, GARRISON_CONTAINER_B, 8);
        mem.write_robin_hood_table(
            OCC_DATA_B + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER_B,
            1,
            8,
            2,
        );
        mem.write_robin_hood_bucket(GARRISON_CONTAINER_B, 0, 11, GARRISON_ROW_B);
        write_garrison_row(&mut mem, GARRISON_ROW_B, OCC_DATA_B, STATE_11, 7_777, 5, 6);

        mem.write_robin_hood_table(OCC_DATA_HEADER, OCC_DATA_CONTAINER, 2, 8, 2);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 0, 2, OCC_DATA_B);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 3, 1, OCC_DATA);

        mem.destroy_robin_hood_bucket(OCC_DATA_CONTAINER, 0);
        mem.write_u32(OCC_DATA_HEADER + ROBIN_HOOD_SIZE_OFF, 1);

        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        roots.insert(11, STATE_11);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();

        assert_eq!(
            mem.read_u64(OCC_DATA_CONTAINER + ROBIN_HOOD_VALUE_OFF)
                .unwrap(),
            OCC_DATA_B,
            "the fixture must actually leave the freed pointer behind, or this \
             test proves nothing"
        );
        assert_eq!(
            mem.read_u64(OCC_DATA_B).unwrap(),
            MODULE_BASE + COUNTRY_OCCUPATION_DATA_VTABLE_RVA as u64
        );
        assert_eq!(
            mem.read_u64(OCC_DATA_B + COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF as u64)
                .unwrap(),
            STATUS_2
        );

        let state_11 = extracted
            .rows
            .iter()
            .find(|row| row.state_id == 11)
            .expect("state 11 is in this frame");
        assert_eq!(
            state_11.garrison_manpower_need, 0,
            "state 11's garrison scalars belong to a DESTROYED occupation and \
             must not be published"
        );
        assert!(
            state_11.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0,
            "a row for a destroyed occupation must never be badged as carrying \
             validated garrison scalars"
        );
        assert!(state_11.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);

        let state_10 = extracted
            .rows
            .iter()
            .find(|row| row.state_id == 10)
            .expect("state 10 is in this frame");
        assert_eq!(state_10.garrison_manpower_need, 14_000);
        assert!(state_10.flags & compact::ROW_FLAG_GARRISON_SCALARS != 0);
        assert_eq!(
            extracted.garrison_anomaly_rows, 0,
            "a destroyed entry is not an anomaly, it is simply absent"
        );
    }

    #[test]
    fn a_destroyed_garrison_row_publishes_nothing() {
        let mut mem = fake_memory_with_garrison(1);
        write_second_state(&mut mem);
        mem.write_robin_hood_table(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER,
            1,
            8,
            2,
        );
        mem.write_robin_hood_bucket(GARRISON_CONTAINER, 0, 11, GARRISON_ROW_B);
        write_garrison_row(&mut mem, GARRISON_ROW_B, OCC_DATA, STATE_11, 7_777, 5, 6);
        mem.destroy_robin_hood_bucket(GARRISON_CONTAINER, 0);
        mem.write_robin_hood_bucket(GARRISON_CONTAINER, 3, 10, GARRISON_ROW);

        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        roots.insert(11, STATE_11);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();

        let state_11 = extracted
            .rows
            .iter()
            .find(|row| row.state_id == 11)
            .expect("state 11 is in this frame");
        assert_eq!(state_11.garrison_manpower_need, 0);
        assert!(state_11.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        let state_10 = extracted
            .rows
            .iter()
            .find(|row| row.state_id == 10)
            .expect("state 10 is in this frame");
        assert_eq!(state_10.garrison_manpower_need, 14_000);
        assert!(state_10.flags & compact::ROW_FLAG_GARRISON_SCALARS != 0);
        assert_eq!(extracted.garrison_anomaly_rows, 0);
    }

    #[test]
    fn an_entry_past_the_old_0x300_window_is_still_read() {
        const OLD_SCAN_LEN: u64 = 0x300;
        let far_index = 40u64;
        assert!(
            far_index * ROBIN_HOOD_STRIDE >= OLD_SCAN_LEN,
            "the fixture entry must sit outside the window being replaced"
        );

        let mut mem = fake_memory_with_garrison(1);
        mem.write_robin_hood_table(OCC_DATA_HEADER, OCC_DATA_CONTAINER, 1, 64, 2);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, far_index, 1, OCC_DATA);

        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        let row = &extracted.rows[0];
        assert_eq!(
            row.garrison_manpower_need, 14_000,
            "a live occupation at bucket {far_index} must be reached"
        );
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS != 0);
        assert_eq!(extracted.garrison_anomaly_rows, 0);
    }

    #[test]
    fn one_record_in_two_live_buckets_drops_the_whole_country() {
        let mut mem = fake_memory_with_garrison(1);
        write_second_state(&mut mem);
        write_occupation_data_record(&mut mem, OCC_DATA_B, STATUS_2, 2, GARRISON_CONTAINER_B, 8);
        mem.write_robin_hood_table(
            OCC_DATA_B + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER_B,
            1,
            8,
            2,
        );
        mem.write_robin_hood_bucket(GARRISON_CONTAINER_B, 0, 11, GARRISON_ROW_B);
        write_garrison_row(&mut mem, GARRISON_ROW_B, OCC_DATA_B, STATE_11, 7_777, 5, 6);

        mem.write_robin_hood_table(OCC_DATA_HEADER, OCC_DATA_CONTAINER, 3, 8, 2);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 1, 1, OCC_DATA);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 3, 2, OCC_DATA_B);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 5, 1, OCC_DATA);

        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        roots.insert(11, STATE_11);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        for row in &extracted.rows {
            assert!(
                row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0,
                "state {} was staged before the duplicate was seen and must not \
                 survive it",
                row.state_id
            );
            assert_eq!(row.garrison_manpower_need, 0);
        }
        assert_eq!(
            extracted.garrison_anomaly_rows, 2,
            "both records' garrison rows failed closed, counted once each"
        );
    }

    #[test]
    fn a_bucket_rejected_by_a_row_guard_is_still_a_witness_at_commit() {
        let mut base = fake_memory_with_garrison(1);
        base.write_robin_hood_table(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER,
            2,
            8,
            2,
        );
        base.write_robin_hood_bucket(GARRISON_CONTAINER, 1, 10, GARRISON_ROW);
        base.write_robin_hood_bucket(GARRISON_CONTAINER, 3, 11, GARRISON_ROW_B);
        write_garrison_row(&mut base, GARRISON_ROW_B, OCC_DATA, STATE_11, 7_777, 5, 6);

        let mem = DestroyAfterFirstSample {
            inner: base,
            occupancy_byte: GARRISON_CONTAINER
                + 3 * ROBIN_HOOD_STRIDE
                + ROBIN_HOOD_DISTANCE_PLUS_1_OFF,
            samples: std::cell::Cell::new(0),
        };
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        let row = &extracted.rows[0];
        assert_eq!(row.state_id, 10);
        assert!(
            row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0,
            "the sibling shared a table with a bucket that died mid-walk"
        );
        assert_eq!(row.garrison_manpower_need, 0);
        assert_eq!(
            extracted.garrison_anomaly_rows, 2,
            "both of the table's live buckets failed closed, counted once each \
             — the row-scope count is replaced, not added to"
        );
    }

    #[test]
    fn entries_behind_a_duplicate_are_counted_not_silently_suppressed() {
        let mut mem = fake_memory_with_garrison(1);
        mem.write_robin_hood_table(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER,
            3,
            8,
            2,
        );
        mem.write_robin_hood_bucket(GARRISON_CONTAINER, 1, 10, GARRISON_ROW);
        mem.write_robin_hood_bucket(GARRISON_CONTAINER, 3, 12, GARRISON_ROW);
        mem.write_robin_hood_bucket(GARRISON_CONTAINER, 5, 11, GARRISON_ROW_B);
        write_garrison_row(&mut mem, GARRISON_ROW_B, OCC_DATA, STATE_10, 7_777, 5, 6);

        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.garrison_anomaly_rows, 3);
        let row = &extracted.rows[0];
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert_eq!(row.garrison_manpower_need, 0);
    }

    fn garrison_table_with_unusable_occupied_tag(entries: u32) -> FakeMemory {
        let mut mem = fake_memory_with_garrison(1);
        mem.write_robin_hood_table(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER,
            entries,
            8,
            2,
        );
        for index in 0..u64::from(entries) {
            let row = GARRISON_ROW + index * 0x200;
            mem.write_robin_hood_bucket(GARRISON_CONTAINER, index * 2 + 1, 10, row);
            write_garrison_row(&mut mem, row, OCC_DATA, STATE_10, 14_000, 87_500, 250_000);
        }
        mem.write_u32(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF as u64,
            0,
        );
        mem
    }

    #[test]
    fn an_unusable_occupied_tag_costs_exactly_its_records_rows() {
        for entries in [1u32, 2, 4] {
            let mem = garrison_table_with_unusable_occupied_tag(entries);
            let mut roots = BTreeMap::new();
            roots.insert(10, STATE_10);
            let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
            assert_eq!(
                extracted.garrison_anomaly_rows, entries,
                "a {entries}-row record with an unusable occupied tag must \
                 report {entries}"
            );
            assert!(extracted.rows[0].flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        }
    }

    #[test]
    fn an_unusable_occupied_tag_on_an_empty_record_costs_nothing() {
        let mut mem = fake_memory_with_garrison(1);
        mem.write_robin_hood_table(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER,
            0,
            8,
            2,
        );
        mem.write_u32(
            OCC_DATA + COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF as u64,
            0,
        );
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.garrison_anomaly_rows, 0);
        assert!(extracted.rows[0].flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
    }

    fn two_records_claiming_one_state(second_manpower: i32) -> FakeMemory {
        let mut mem = fake_memory_with_garrison(1);
        write_occupation_data_record(&mut mem, OCC_DATA_B, STATUS_2, 1, GARRISON_CONTAINER_B, 8);
        mem.write_robin_hood_table(
            OCC_DATA_B + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER_B,
            1,
            8,
            2,
        );
        mem.write_robin_hood_bucket(GARRISON_CONTAINER_B, 0, 10, GARRISON_ROW_B);
        write_garrison_row(
            &mut mem,
            GARRISON_ROW_B,
            OCC_DATA_B,
            STATE_10,
            second_manpower,
            87_500,
            250_000,
        );
        mem.write_robin_hood_table(OCC_DATA_HEADER, OCC_DATA_CONTAINER, 2, 8, 2);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 1, 1, OCC_DATA);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 3, 2, OCC_DATA_B);
        mem
    }

    fn add_third_record(mem: &mut FakeMemory, manpower: i32) {
        const OCC_DATA_C: u64 = 0x22000010000;
        const GARRISON_CONTAINER_C: u64 = 0x22000010800;
        const GARRISON_ROW_C: u64 = 0x22000011000;
        write_occupation_data_record(mem, OCC_DATA_C, STATUS_2, 1, GARRISON_CONTAINER_C, 8);
        mem.write_robin_hood_table(
            OCC_DATA_C + COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF as u64,
            GARRISON_CONTAINER_C,
            1,
            8,
            2,
        );
        mem.write_robin_hood_bucket(GARRISON_CONTAINER_C, 0, 10, GARRISON_ROW_C);
        write_garrison_row(
            mem,
            GARRISON_ROW_C,
            OCC_DATA_C,
            STATE_10,
            manpower,
            87_500,
            250_000,
        );
        mem.write_robin_hood_table(OCC_DATA_HEADER, OCC_DATA_CONTAINER, 3, 8, 2);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 1, 1, OCC_DATA);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 3, 2, OCC_DATA_B);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 5, 3, OCC_DATA_C);
    }

    #[test]
    fn agreeing_rows_dropped_by_a_later_disagreement_are_each_counted() {
        let mut mem = two_records_claiming_one_state(14_000);
        add_third_record(&mut mem, 9_999);

        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        let row = &extracted.rows[0];
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert_eq!(row.garrison_manpower_need, 0);
        assert_eq!(
            extracted.garrison_anomaly_rows, 3,
            "A, A and B all failed closed"
        );
    }

    #[test]
    fn a_row_arriving_after_the_state_is_already_conflicted_is_counted() {
        let mut mem = two_records_claiming_one_state(9_999);
        add_third_record(&mut mem, 14_000);

        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        let row = &extracted.rows[0];
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert_eq!(extracted.garrison_anomaly_rows, 3);
    }

    #[test]
    fn a_country_scope_failure_with_nothing_else_to_count_is_still_visible() {
        let mut mem = fake_memory_with_garrison(1);
        mem.write_u64(OCC_DATA, MODULE_BASE + STATE_VTABLE_RVA as u64);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        let row = &extracted.rows[0];
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);
        assert_eq!(
            extracted.garrison_anomaly_rows, 1,
            "the unreachable record must be reported, not merely skipped"
        );
    }

    #[test]
    fn a_container_whose_count_disagrees_with_its_buckets_fails_closed() {
        let mut mem = fake_memory_with_garrison(1);
        mem.write_robin_hood_bucket(OCC_DATA_CONTAINER, 5, 2, OCC_DATA);
        let mut roots = BTreeMap::new();
        roots.insert(10, STATE_10);
        let extracted = extract_rows_from_state_roots(&mem, GAME_STATE, &roots).unwrap();
        assert_eq!(extracted.garrison_anomaly_rows, 1);
        let row = &extracted.rows[0];
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SCALARS == 0);
        assert!(row.flags & compact::ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0);
        assert_eq!(row.garrison_manpower_need, 0);
    }
}
