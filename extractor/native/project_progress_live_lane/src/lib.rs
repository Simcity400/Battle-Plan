#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

pub const LANE_COMPACT_MAGIC: [u8; 4] = *b"PPL1";
pub const LANE_COMPACT_VERSION: u16 = 2;
pub const LANE_HEADER_LEN: usize = 32;
pub const PROJECT_STATE_ROW_LEN: usize = 48;
pub const PROJECT_BREAKTHROUGH_ROW_LEN: usize = 16;
pub const LANE_EVENT_MAGIC: [u8; 4] = *b"PLE1";
pub const LANE_EVENT_VERSION: u16 = 1;
pub const LANE_EVENT_HEADER_LEN: usize = 16;
pub const PROJECT_LIFECYCLE_EVENT_ROW_LEN: usize = 32;
pub const PROJECT_EVENT_KIND_COMPLETION_CHANGED: u16 = 1;
pub const PROJECT_EVENT_KIND_BREAKTHROUGH_DELTA: u16 = 2;

pub const PROJECT_STATE_MACHINE_VTABLE_RVA: usize = 0x2A19358;
pub const PROJECT_PROTOTYPE_STATE_VTABLE_RVA: usize = 0x2A19300;
pub const PROJECT_SIMPLE_STATE_VTABLE_RVA: usize = 0x2A192A8;
pub const PROJECT_BREAKTHROUGH_PROGRESS_VTABLE_RVA: usize = 0x2A152E0;
pub const PROJECT_PROJECT_OFF_STATE_MACHINE_PTR: usize = 0x28;
pub const PROJECT_PROJECT_OFF_TYPE: usize = 0x08;
pub const PROJECT_PROJECT_OFF_LOCAL_ID: usize = 0x0C;
pub const PROJECT_PROJECT_TYPE: u32 = 86;
pub const PROJECT_PROGRAM_STATUS_OFF_BREAKTHROUGH: usize = 0x50;
pub const PROJECT_PROJECT_OFF_HISTORY_DATA: usize = 0x68;
pub const PROJECT_PROJECT_OFF_HISTORY_CAPACITY: usize = 0x70;
pub const PROJECT_PROJECT_OFF_HISTORY_COUNT: usize = 0x74;
pub const PROJECT_PROJECT_OFF_START: usize = 0x1BC;
pub const PROJECT_PROJECT_OFF_COMPLETED: usize = 0x1BD;
pub const PROJECT_HISTORY_ROW_STRIDE: usize = 0x58;
pub const PROJECT_HISTORY_VTABLE_RVA: usize = 0x296C220;
pub const PROJECT_HISTORY_CONTEXT_VTABLE_RVA: usize = 0x296C1D0;
pub const PROJECT_HISTORY_TOKEN_PROVINCE: u32 = 0x2840;
pub const PROJECT_HISTORY_TOKEN_PROGRESS: u32 = 0x2B05;
pub const PROJECT_HISTORY_TOKEN_COUNTRY: u32 = 0x289A;
pub const PROJECT_HISTORY_TOKEN_SCIENTIST: u32 = 0x4005;
pub const PROJECT_HISTORY_TOKEN_STATE: u32 = 0x01B7;
pub const PROJECT_HISTORY_TOKEN_CHARACTER: u32 = 0x4C16;
pub const PROJECT_HISTORY_TOKEN_CONTEXT: u32 = 0x4029;
pub const PROJECT_HISTORY_TOKEN_OPTION: u32 = 0x2966;
pub const PROJECT_HISTORY_TOKEN_PROTOTYPE_REWARD: u32 = 0x2732;
pub const PROJECT_HISTORY_TOKEN_DATE: u32 = 0x284A;
pub const PROJECT_HISTORY_TOKEN_HISTORY: u32 = 0x2835;
pub const PROJECT_TOKEN_COMPLETED: u32 = 0x3127;
pub const PROJECT_TOKEN_START: u32 = 0x0069;
pub const PROJECT_HISTORY_UNDEFINED_HANDLE: u32 = 0x4C17;
pub const PROJECT_STATE_MACHINE_OFF_CURRENT_STATE_PTR: usize = 0x08;
pub const PROJECT_STATE_MACHINE_OFF_NOT_COMPLETED_FLAG: usize = 0x10;
pub const PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE: usize = 0x18;
pub const PROJECT_STATE_MACHINE_OFF_SIMPLE_1: usize = 0xE8;
pub const PROJECT_STATE_MACHINE_OFF_SIMPLE_2: usize = 0x168;
pub const PROJECT_STATE_MACHINE_OFF_SIMPLE_3: usize = 0x1E8;
pub const PROJECT_STATE_OFF_CURRENT_STATE_ENUM: usize = 0x78;
pub const PROJECT_PROTOTYPE_STATE_OFF_PHASE_PROGRESS_RAW: usize = 0x48;
pub const PROJECT_PROTOTYPE_STATE_OFF_PHASE_TARGET: usize = 0x50;
pub const PROJECT_PROTOTYPE_STATE_OFF_ITERATIONS: usize = 0x8C;

pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 0;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 1;
pub const STATUS_WORKER_PANIC: u32 = 1 << 2;
pub const STATUS_SOURCE_FAILURE: u32 = 1 << 3;
pub const PROJECT_STATE_FLAG_COMPLETED: u32 = 1 << 0;

const MAX_PROJECT_ID: u32 = 10_000_000;
const MAX_PROJECT_HANDLE: u32 = 10_000_000;
const MAX_PROJECT_ITERATIONS: u32 = 1_000_000;
const MAX_PROJECT_PROGRESS_RAW: u32 = 1_000_000_000;
const MAX_PROJECT_PHASE_TARGET_RAW: u32 = 1_000_000;
const MAX_PROJECT_STATE_ENUM: u32 = 64;
const MAX_PROJECT_BREAKTHROUGH_POINTS: u32 = 1_000_000_000;
const MAX_PROJECT_BREAKTHROUGH_ROWS: usize = 16_384;
const MAX_PROJECT_REWARD_HISTORY_ROWS: u32 = 4_096;

pub trait ProjectMemory {
    fn read_u8(&self, addr: usize) -> Option<u8>;
    fn read_u32(&self, addr: usize) -> Option<u32>;
    fn read_u64(&self, addr: usize) -> Option<u64>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectLayout {
    pub base: usize,
    pub project_vtable: usize,
    pub state_machine_vtable: usize,
    pub prototype_state_vtable: usize,
    pub simple_state_vtable: usize,
    pub breakthrough_vtable: usize,
}

impl ProjectLayout {
    pub fn from_module_base(base: usize, project_vtable_rva: usize) -> Self {
        Self {
            base,
            project_vtable: base.saturating_add(project_vtable_rva),
            state_machine_vtable: base.saturating_add(PROJECT_STATE_MACHINE_VTABLE_RVA),
            prototype_state_vtable: base.saturating_add(PROJECT_PROTOTYPE_STATE_VTABLE_RVA),
            simple_state_vtable: base.saturating_add(PROJECT_SIMPLE_STATE_VTABLE_RVA),
            breakthrough_vtable: base.saturating_add(PROJECT_BREAKTHROUGH_PROGRESS_VTABLE_RVA),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectPin {
    pub addr: usize,
    pub id: u32,
    pub static_def_handle: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnedProjectPin {
    pub addr: usize,
    pub tag_db_id: u32,
    pub local_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectRewardHistoryLayout {
    pub project_vtable: usize,
    pub history_vtable: usize,
    pub context_vtable: usize,
}

impl ProjectRewardHistoryLayout {
    pub fn from_module_base(base: usize, project_vtable_rva: usize) -> Self {
        Self {
            project_vtable: base.saturating_add(project_vtable_rva),
            history_vtable: base.saturating_add(PROJECT_HISTORY_VTABLE_RVA),
            context_vtable: base.saturating_add(PROJECT_HISTORY_CONTEXT_VTABLE_RVA),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRewardHistoryRow {
    pub tag_db_id: u32,
    pub local_id: u32,
    pub history_index: u32,
    pub prototype_reward_handle: u32,
    pub option_handle: u32,
    pub province: i32,
    pub progress_raw: u32,
    pub character_ref_type: u32,
    pub character_ref_id: u32,
    pub country_ref: u32,
    pub scientist_ref: u32,
    pub state_ref: u32,
    pub date_raw: i32,
    pub country_present: bool,
    pub scientist_present: bool,
    pub state_present: bool,
}

pub fn read_project_lifecycle<M: ProjectMemory>(
    mem: &M,
    pin: OwnedProjectPin,
    layout: ProjectRewardHistoryLayout,
) -> Result<(bool, bool), String> {
    validate_owned_project(mem, pin, layout)?;
    let start = read_bool_byte(mem, pin.addr + PROJECT_PROJECT_OFF_START, "start")?;
    let completed = read_bool_byte(
        mem,
        pin.addr + PROJECT_PROJECT_OFF_COMPLETED,
        "completed",
    )?;
    Ok((start, completed))
}

pub fn read_project_reward_history_rows<M: ProjectMemory>(
    mem: &M,
    pin: OwnedProjectPin,
    layout: ProjectRewardHistoryLayout,
) -> Result<Vec<ProjectRewardHistoryRow>, String> {
    validate_owned_project(mem, pin, layout)?;
    let data = mem
        .read_u64(pin.addr + PROJECT_PROJECT_OFF_HISTORY_DATA)
        .map(|value| value as usize)
        .ok_or_else(|| "project history data pointer unreadable".to_string())?;
    let capacity = mem
        .read_u32(pin.addr + PROJECT_PROJECT_OFF_HISTORY_CAPACITY)
        .ok_or_else(|| "project history capacity unreadable".to_string())?;
    let count = mem
        .read_u32(pin.addr + PROJECT_PROJECT_OFF_HISTORY_COUNT)
        .ok_or_else(|| "project history count unreadable".to_string())?;
    if count > capacity || count > MAX_PROJECT_REWARD_HISTORY_ROWS {
        return Err("project history vector bounds are invalid".into());
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    if !is_live_heap_ptr(data) {
        return Err("project history data pointer is invalid".into());
    }
    let mut rows = Vec::with_capacity(count as usize);
    for history_index in 0..count {
        let row_offset = (history_index as usize)
            .checked_mul(PROJECT_HISTORY_ROW_STRIDE)
            .ok_or_else(|| "project history row offset overflow".to_string())?;
        let row = data
            .checked_add(row_offset)
            .ok_or_else(|| "project history row address overflow".to_string())?;
        if mem.read_u64(row).map(|value| value as usize) != Some(layout.history_vtable)
            || mem
                .read_u64(row + 0x10)
                .map(|value| value as usize)
                != Some(layout.context_vtable)
        {
            return Err(format!("project history row {history_index} vtable mismatch"));
        }
        let country_present = read_bool_byte(mem, row + 0x2C, "country presence")?;
        let scientist_present = read_bool_byte(mem, row + 0x34, "scientist presence")?;
        let state_present = read_bool_byte(mem, row + 0x3C, "state presence")?;
        let country_ref = required_u32(mem, row + 0x28, "country ref")?;
        let scientist_ref = required_u32(mem, row + 0x30, "scientist ref")?;
        let state_ref = required_u32(mem, row + 0x38, "state ref")?;
        for (label, present, raw) in [
            ("country", country_present, country_ref),
            ("scientist", scientist_present, scientist_ref),
            ("state", state_present, state_ref),
        ] {
            if !present && raw != 0 {
                return Err(format!(
                    "project history row {history_index} has {label} ref without presence"
                ));
            }
        }
        let character_ref = mem
            .read_u64(row + 0x20)
            .ok_or_else(|| format!("project history row {history_index} character ref unreadable"))?;
        rows.push(ProjectRewardHistoryRow {
            tag_db_id: pin.tag_db_id,
            local_id: pin.local_id,
            history_index,
            prototype_reward_handle: required_u32(mem, row + 0x08, "prototype reward")?,
            option_handle: required_u32(mem, row + 0x0C, "option")?,
            province: required_u32(mem, row + 0x18, "province")? as i32,
            progress_raw: required_u32(mem, row + 0x1C, "progress")?,
            character_ref_type: character_ref as u32,
            character_ref_id: (character_ref >> 32) as u32,
            country_ref,
            scientist_ref,
            state_ref,
            date_raw: required_u32(mem, row + 0x48, "date")? as i32,
            country_present,
            scientist_present,
            state_present,
        });
    }
    Ok(rows)
}

fn validate_owned_project<M: ProjectMemory>(
    mem: &M,
    pin: OwnedProjectPin,
    layout: ProjectRewardHistoryLayout,
) -> Result<(), String> {
    if pin.tag_db_id == 0
        || pin.local_id == 0
        || pin.local_id > MAX_PROJECT_ID
        || !is_live_heap_ptr(pin.addr)
    {
        return Err("owned project key/address is invalid".into());
    }
    if mem.read_u64(pin.addr).map(|value| value as usize) != Some(layout.project_vtable) {
        return Err("owned project vtable mismatch".into());
    }
    if mem.read_u32(pin.addr + PROJECT_PROJECT_OFF_TYPE) != Some(PROJECT_PROJECT_TYPE)
        || mem.read_u32(pin.addr + PROJECT_PROJECT_OFF_LOCAL_ID) != Some(pin.local_id)
    {
        return Err("owned project type/local-id mismatch".into());
    }
    Ok(())
}

fn read_bool_byte<M: ProjectMemory>(mem: &M, addr: usize, label: &str) -> Result<bool, String> {
    match mem.read_u8(addr) {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        Some(value) => Err(format!("{label} byte is outside the boolean domain: {value}")),
        None => Err(format!("{label} byte unreadable")),
    }
}

fn required_u32<M: ProjectMemory>(mem: &M, addr: usize, label: &str) -> Result<u32, String> {
    mem.read_u32(addr)
        .ok_or_else(|| format!("{label} unreadable"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectStateRow {
    pub id: u32,
    pub static_def_handle: u32,
    pub current_state_offset: u32,
    pub current_state_enum: u32,
    pub prototype_state_enum: u32,
    pub simple_state_1_enum: u32,
    pub simple_state_2_enum: u32,
    pub simple_state_3_enum: u32,
    pub prototype_iterations: u32,
    pub phase_progress_raw: u32,
    pub phase_target_raw: u32,
    pub state_flags: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectBreakthroughRow {
    pub tag_db_id: u32,
    pub specialization_handle: u32,
    pub points_raw: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneFrame {
    pub game_hour_raw: i32,
    pub status_flags: u32,
    pub state_rows: Vec<ProjectStateRow>,
    pub breakthrough_rows: Vec<ProjectBreakthroughRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateReport {
    pub ok: bool,
    pub state_rows: usize,
    pub breakthrough_rows: usize,
    pub status_flags: u32,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectLifecycleEvent {
    pub kind: u16,
    pub key_a: u32,
    pub key_b: u32,
    pub before: u32,
    pub after: u32,
    pub delta: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventGateReport {
    pub ok: bool,
    pub events: usize,
    pub errors: Vec<String>,
}

pub fn read_project_state_row<M: ProjectMemory>(
    mem: &M,
    pin: ProjectPin,
    layout: ProjectLayout,
) -> Option<ProjectStateRow> {
    if pin.id == 0 || pin.id > MAX_PROJECT_ID || pin.static_def_handle == 0 {
        return None;
    }
    if mem.read_u64(pin.addr)? as usize != layout.project_vtable {
        return None;
    }
    let state_machine_ptr =
        mem.read_u64(pin.addr + PROJECT_PROJECT_OFF_STATE_MACHINE_PTR)? as usize;
    if !is_live_heap_ptr(state_machine_ptr)
        || mem.read_u64(state_machine_ptr)? as usize != layout.state_machine_vtable
    {
        return None;
    }
    let not_completed_flag =
        mem.read_u8(state_machine_ptr + PROJECT_STATE_MACHINE_OFF_NOT_COMPLETED_FLAG)?;
    if not_completed_flag > 1 {
        return None;
    }

    let prototype_addr = state_machine_ptr + PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE;
    let simple_1_addr = state_machine_ptr + PROJECT_STATE_MACHINE_OFF_SIMPLE_1;
    let simple_2_addr = state_machine_ptr + PROJECT_STATE_MACHINE_OFF_SIMPLE_2;
    let simple_3_addr = state_machine_ptr + PROJECT_STATE_MACHINE_OFF_SIMPLE_3;
    if mem.read_u64(prototype_addr)? as usize != layout.prototype_state_vtable
        || mem.read_u64(simple_1_addr)? as usize != layout.simple_state_vtable
        || mem.read_u64(simple_2_addr)? as usize != layout.simple_state_vtable
        || mem.read_u64(simple_3_addr)? as usize != layout.simple_state_vtable
    {
        return None;
    }

    let current_state_ptr =
        mem.read_u64(state_machine_ptr + PROJECT_STATE_MACHINE_OFF_CURRENT_STATE_PTR)? as usize;
    let current_state_offset = current_state_ptr.checked_sub(state_machine_ptr)?;
    let expected_vtable = match current_state_offset {
        PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE => layout.prototype_state_vtable,
        PROJECT_STATE_MACHINE_OFF_SIMPLE_1
        | PROJECT_STATE_MACHINE_OFF_SIMPLE_2
        | PROJECT_STATE_MACHINE_OFF_SIMPLE_3 => layout.simple_state_vtable,
        _ => return None,
    };
    if mem.read_u64(current_state_ptr)? as usize != expected_vtable {
        return None;
    }

    Some(ProjectStateRow {
        id: pin.id,
        static_def_handle: pin.static_def_handle,
        current_state_offset: current_state_offset as u32,
        current_state_enum: read_state_enum(mem, current_state_ptr)?,
        prototype_state_enum: read_state_enum(mem, prototype_addr)?,
        simple_state_1_enum: read_state_enum(mem, simple_1_addr)?,
        simple_state_2_enum: read_state_enum(mem, simple_2_addr)?,
        simple_state_3_enum: read_state_enum(mem, simple_3_addr)?,
        prototype_iterations: read_bounded_u32(
            mem,
            prototype_addr + PROJECT_PROTOTYPE_STATE_OFF_ITERATIONS,
            MAX_PROJECT_ITERATIONS,
        )?,
        phase_progress_raw: read_bounded_u32(
            mem,
            prototype_addr + PROJECT_PROTOTYPE_STATE_OFF_PHASE_PROGRESS_RAW,
            MAX_PROJECT_PROGRESS_RAW,
        )?,
        phase_target_raw: read_nonzero_bounded_u32(
            mem,
            prototype_addr + PROJECT_PROTOTYPE_STATE_OFF_PHASE_TARGET,
            MAX_PROJECT_PHASE_TARGET_RAW,
        )?,
        state_flags: if not_completed_flag == 0 {
            PROJECT_STATE_FLAG_COMPLETED
        } else {
            0
        },
    })
}

pub fn read_breakthrough_rows<M: ProjectMemory>(
    mem: &M,
    breakthrough_addr: usize,
    breakthrough_vtable: usize,
    tag_db_id: u32,
) -> Result<Vec<ProjectBreakthroughRow>, String> {
    if tag_db_id == 0 {
        return Err("tag_db_id is zero".to_string());
    }
    if !is_live_heap_ptr(breakthrough_addr)
        || mem.read_u64(breakthrough_addr).map(|v| v as usize) != Some(breakthrough_vtable)
    {
        return Err("breakthrough vtable mismatch".to_string());
    }
    let header = mem
        .read_u64(breakthrough_addr + 0x08)
        .map(|v| v as usize)
        .ok_or_else(|| "breakthrough header unreadable".to_string())?;
    let size = mem
        .read_u64(breakthrough_addr + 0x10)
        .ok_or_else(|| "breakthrough size unreadable".to_string())? as usize;
    if size == 0 {
        return Ok(Vec::new());
    }
    if !is_live_heap_ptr(header) || size > MAX_PROJECT_BREAKTHROUGH_ROWS {
        return Err("breakthrough header/size invalid".to_string());
    }
    let root = mem
        .read_u64(header + 0x08)
        .map(|v| v as usize)
        .ok_or_else(|| "breakthrough root unreadable".to_string())?;
    if !is_live_heap_ptr(root) {
        return Err("breakthrough root invalid".to_string());
    }

    let mut stack = vec![root];
    let mut seen_nodes = BTreeSet::new();
    let mut rows = Vec::new();
    while let Some(node) = stack.pop() {
        if !is_live_heap_ptr(node) || !seen_nodes.insert(node) {
            return Err("breakthrough tree cycle or invalid node".to_string());
        }
        if mem.read_u8(node + 0x19) == Some(1) {
            continue;
        }
        let specialization_handle = read_nonzero_bounded_u32(mem, node + 0x1C, MAX_PROJECT_HANDLE)
            .ok_or_else(|| "breakthrough specialization invalid".to_string())?;
        let points_raw = read_bounded_u32(mem, node + 0x20, MAX_PROJECT_BREAKTHROUGH_POINTS)
            .ok_or_else(|| "breakthrough points invalid".to_string())?;
        rows.push(ProjectBreakthroughRow {
            tag_db_id,
            specialization_handle,
            points_raw,
        });
        if rows.len() > size || rows.len() > MAX_PROJECT_BREAKTHROUGH_ROWS {
            return Err("breakthrough row count overflow".to_string());
        }
        for child_off in [0x10usize, 0usize] {
            if let Some(child) = mem.read_u64(node + child_off).map(|v| v as usize) {
                if is_live_heap_ptr(child) {
                    stack.push(child);
                }
            }
        }
    }
    if rows.len() != size {
        return Err(format!(
            "breakthrough tree yielded {} rows, expected {size}",
            rows.len()
        ));
    }
    rows.sort_by_key(|row| (row.tag_db_id, row.specialization_handle));
    Ok(rows)
}

pub fn encode_lane_frame(frame: &LaneFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        LANE_HEADER_LEN
            + frame.state_rows.len() * PROJECT_STATE_ROW_LEN
            + frame.breakthrough_rows.len() * PROJECT_BREAKTHROUGH_ROW_LEN,
    );
    out.extend_from_slice(&LANE_COMPACT_MAGIC);
    out.extend_from_slice(&LANE_COMPACT_VERSION.to_le_bytes());
    out.extend_from_slice(&(LANE_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&frame.status_flags.to_le_bytes());
    out.extend_from_slice(&frame.game_hour_raw.to_le_bytes());
    out.extend_from_slice(&(frame.state_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(
        &(frame.breakthrough_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    out.extend_from_slice(&(PROJECT_STATE_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&(PROJECT_BREAKTHROUGH_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for row in &frame.state_rows {
        write_state_row(&mut out, row);
    }
    for row in &frame.breakthrough_rows {
        write_breakthrough_row(&mut out, row);
    }
    out
}

pub fn decode_lane_frame(bytes: &[u8]) -> Result<LaneFrame, String> {
    if bytes.len() < LANE_HEADER_LEN {
        return Err(format!("frame too short: {}", bytes.len()));
    }
    if bytes[0..4] != LANE_COMPACT_MAGIC {
        return Err("magic mismatch".to_string());
    }
    if read_u16_le(bytes, 4) != LANE_COMPACT_VERSION {
        return Err("version mismatch".to_string());
    }
    let header_len = read_u16_le(bytes, 6) as usize;
    if header_len != LANE_HEADER_LEN {
        return Err(format!("unexpected header_len {header_len}"));
    }
    let status_flags = read_u32_le(bytes, 8);
    let game_hour_raw = read_i32_le(bytes, 12);
    let state_count = read_u32_le(bytes, 16) as usize;
    let breakthrough_count = read_u32_le(bytes, 20) as usize;
    let state_row_len = read_u16_le(bytes, 24) as usize;
    let breakthrough_row_len = read_u16_le(bytes, 26) as usize;
    if state_row_len != PROJECT_STATE_ROW_LEN
        || breakthrough_row_len != PROJECT_BREAKTHROUGH_ROW_LEN
    {
        return Err("row length mismatch".to_string());
    }
    if read_u32_le(bytes, 28) != 0 {
        return Err("reserved header field is nonzero".to_string());
    }
    let expected_len = LANE_HEADER_LEN
        .saturating_add(state_count.saturating_mul(PROJECT_STATE_ROW_LEN))
        .saturating_add(breakthrough_count.saturating_mul(PROJECT_BREAKTHROUGH_ROW_LEN));
    if bytes.len() != expected_len {
        return Err(format!(
            "frame length {} != expected {expected_len}",
            bytes.len()
        ));
    }

    let mut cursor = LANE_HEADER_LEN;
    let mut state_rows = Vec::with_capacity(state_count);
    let mut seen_states = BTreeSet::new();
    for idx in 0..state_count {
        let row = read_state_row(bytes, cursor);
        validate_state_row(idx, &row)?;
        if !seen_states.insert((row.id, row.static_def_handle)) {
            return Err(format!("state row {idx} duplicate id/static_def_handle"));
        }
        cursor += PROJECT_STATE_ROW_LEN;
        state_rows.push(row);
    }
    let mut breakthrough_rows = Vec::with_capacity(breakthrough_count);
    let mut seen_breakthrough = BTreeSet::new();
    for idx in 0..breakthrough_count {
        let row = read_breakthrough_row(bytes, cursor);
        validate_breakthrough_row(idx, &row)?;
        if !seen_breakthrough.insert((row.tag_db_id, row.specialization_handle)) {
            return Err(format!(
                "breakthrough row {idx} duplicate tag/specialization"
            ));
        }
        cursor += PROJECT_BREAKTHROUGH_ROW_LEN;
        breakthrough_rows.push(row);
    }
    Ok(LaneFrame {
        game_hour_raw,
        status_flags,
        state_rows,
        breakthrough_rows,
    })
}

pub fn gate_lane_frame(bytes: &[u8]) -> GateReport {
    match decode_lane_frame(bytes) {
        Ok(frame) => {
            let mut errors = Vec::new();
            let blocking_flags = STATUS_SOURCE_UNAVAILABLE
                | STATUS_PARTIAL_DATA
                | STATUS_WORKER_PANIC
                | STATUS_SOURCE_FAILURE;
            if frame.status_flags & blocking_flags != 0 {
                errors.push(format!(
                    "blocking status_flags set: {:#x}",
                    frame.status_flags
                ));
            }
            if frame.state_rows.is_empty() {
                errors.push("no project state rows".to_string());
            }
            if frame.breakthrough_rows.is_empty() {
                errors.push("no breakthrough rows".to_string());
            }
            GateReport {
                ok: errors.is_empty(),
                state_rows: frame.state_rows.len(),
                breakthrough_rows: frame.breakthrough_rows.len(),
                status_flags: frame.status_flags,
                errors,
            }
        }
        Err(err) => GateReport {
            ok: false,
            state_rows: 0,
            breakthrough_rows: 0,
            status_flags: 0,
            errors: vec![err],
        },
    }
}

pub fn derive_lifecycle_events(
    previous: &LaneFrame,
    next: &LaneFrame,
) -> Vec<ProjectLifecycleEvent> {
    let mut events = Vec::new();
    let previous_states: BTreeMap<(u32, u32), u32> = previous
        .state_rows
        .iter()
        .map(|row| {
            (
                (row.id, row.static_def_handle),
                completion_flag_value(row.state_flags),
            )
        })
        .collect();
    for row in &next.state_rows {
        let key = (row.id, row.static_def_handle);
        if let Some(before) = previous_states.get(&key).copied() {
            let after = completion_flag_value(row.state_flags);
            if before != after {
                events.push(ProjectLifecycleEvent {
                    kind: PROJECT_EVENT_KIND_COMPLETION_CHANGED,
                    key_a: row.id,
                    key_b: row.static_def_handle,
                    before,
                    after,
                    delta: after as i64 - before as i64,
                });
            }
        }
    }

    let previous_breakthrough: BTreeMap<(u32, u32), u32> = previous
        .breakthrough_rows
        .iter()
        .map(|row| ((row.tag_db_id, row.specialization_handle), row.points_raw))
        .collect();
    for row in &next.breakthrough_rows {
        let key = (row.tag_db_id, row.specialization_handle);
        if let Some(before) = previous_breakthrough.get(&key).copied() {
            if before != row.points_raw {
                events.push(ProjectLifecycleEvent {
                    kind: PROJECT_EVENT_KIND_BREAKTHROUGH_DELTA,
                    key_a: row.tag_db_id,
                    key_b: row.specialization_handle,
                    before,
                    after: row.points_raw,
                    delta: row.points_raw as i64 - before as i64,
                });
            }
        }
    }
    events.sort_by_key(|event| (event.kind, event.key_a, event.key_b));
    events
}

pub fn encode_lifecycle_events(events: &[ProjectLifecycleEvent]) -> Vec<u8> {
    let mut out =
        Vec::with_capacity(LANE_EVENT_HEADER_LEN + events.len() * PROJECT_LIFECYCLE_EVENT_ROW_LEN);
    out.extend_from_slice(&LANE_EVENT_MAGIC);
    out.extend_from_slice(&LANE_EVENT_VERSION.to_le_bytes());
    out.extend_from_slice(&(LANE_EVENT_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&(events.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&(PROJECT_LIFECYCLE_EVENT_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    for event in events {
        out.extend_from_slice(&event.kind.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&event.key_a.to_le_bytes());
        out.extend_from_slice(&event.key_b.to_le_bytes());
        out.extend_from_slice(&event.before.to_le_bytes());
        out.extend_from_slice(&event.after.to_le_bytes());
        out.extend_from_slice(&event.delta.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out
}

pub fn decode_lifecycle_events(bytes: &[u8]) -> Result<Vec<ProjectLifecycleEvent>, String> {
    if bytes.len() < LANE_EVENT_HEADER_LEN {
        return Err(format!("event frame too short: {}", bytes.len()));
    }
    if bytes[0..4] != LANE_EVENT_MAGIC {
        return Err("event magic mismatch".to_string());
    }
    if read_u16_le(bytes, 4) != LANE_EVENT_VERSION {
        return Err("event version mismatch".to_string());
    }
    let header_len = read_u16_le(bytes, 6) as usize;
    if header_len != LANE_EVENT_HEADER_LEN {
        return Err(format!("unexpected event header_len {header_len}"));
    }
    let event_count = read_u32_le(bytes, 8) as usize;
    let row_len = read_u16_le(bytes, 12) as usize;
    if row_len != PROJECT_LIFECYCLE_EVENT_ROW_LEN {
        return Err("event row length mismatch".to_string());
    }
    if read_u16_le(bytes, 14) != 0 {
        return Err("event reserved header field is nonzero".to_string());
    }
    let expected_len = LANE_EVENT_HEADER_LEN.saturating_add(event_count.saturating_mul(row_len));
    if bytes.len() != expected_len {
        return Err(format!(
            "event frame length {} != expected {expected_len}",
            bytes.len()
        ));
    }
    let mut events = Vec::with_capacity(event_count);
    let mut cursor = LANE_EVENT_HEADER_LEN;
    for idx in 0..event_count {
        let kind = read_u16_le(bytes, cursor);
        if read_u16_le(bytes, cursor + 2) != 0 {
            return Err(format!("event row {idx} reserved field is nonzero"));
        }
        let event = ProjectLifecycleEvent {
            kind,
            key_a: read_u32_le(bytes, cursor + 4),
            key_b: read_u32_le(bytes, cursor + 8),
            before: read_u32_le(bytes, cursor + 12),
            after: read_u32_le(bytes, cursor + 16),
            delta: read_i64_le(bytes, cursor + 20),
        };
        if read_u32_le(bytes, cursor + 28) != 0 {
            return Err(format!(
                "event row {idx} trailing reserved field is nonzero"
            ));
        }
        validate_lifecycle_event(idx, &event)?;
        events.push(event);
        cursor += row_len;
    }
    Ok(events)
}

pub fn gate_lifecycle_events(bytes: &[u8]) -> EventGateReport {
    match decode_lifecycle_events(bytes) {
        Ok(events) => EventGateReport {
            ok: true,
            events: events.len(),
            errors: Vec::new(),
        },
        Err(err) => EventGateReport {
            ok: false,
            events: 0,
            errors: vec![err],
        },
    }
}

fn read_state_enum<M: ProjectMemory>(mem: &M, state_addr: usize) -> Option<u32> {
    read_bounded_u32(
        mem,
        state_addr + PROJECT_STATE_OFF_CURRENT_STATE_ENUM,
        MAX_PROJECT_STATE_ENUM,
    )
}

fn read_bounded_u32<M: ProjectMemory>(mem: &M, addr: usize, max: u32) -> Option<u32> {
    mem.read_u32(addr).filter(|value| *value <= max)
}

fn read_nonzero_bounded_u32<M: ProjectMemory>(mem: &M, addr: usize, max: u32) -> Option<u32> {
    mem.read_u32(addr)
        .filter(|value| *value > 0 && *value <= max)
}

fn is_live_heap_ptr(value: usize) -> bool {
    (0x10000..=0x0000_7fff_ffff_ffff).contains(&value)
}

fn write_state_row(out: &mut Vec<u8>, row: &ProjectStateRow) {
    out.extend_from_slice(&row.id.to_le_bytes());
    out.extend_from_slice(&row.static_def_handle.to_le_bytes());
    out.extend_from_slice(&row.current_state_offset.to_le_bytes());
    out.extend_from_slice(&row.current_state_enum.to_le_bytes());
    out.extend_from_slice(&row.prototype_state_enum.to_le_bytes());
    out.extend_from_slice(&row.simple_state_1_enum.to_le_bytes());
    out.extend_from_slice(&row.simple_state_2_enum.to_le_bytes());
    out.extend_from_slice(&row.simple_state_3_enum.to_le_bytes());
    out.extend_from_slice(&row.prototype_iterations.to_le_bytes());
    out.extend_from_slice(&row.phase_progress_raw.to_le_bytes());
    out.extend_from_slice(&row.phase_target_raw.to_le_bytes());
    out.extend_from_slice(&row.state_flags.to_le_bytes());
}

fn write_breakthrough_row(out: &mut Vec<u8>, row: &ProjectBreakthroughRow) {
    out.extend_from_slice(&row.tag_db_id.to_le_bytes());
    out.extend_from_slice(&row.specialization_handle.to_le_bytes());
    out.extend_from_slice(&row.points_raw.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

fn read_state_row(bytes: &[u8], offset: usize) -> ProjectStateRow {
    ProjectStateRow {
        id: read_u32_le(bytes, offset),
        static_def_handle: read_u32_le(bytes, offset + 4),
        current_state_offset: read_u32_le(bytes, offset + 8),
        current_state_enum: read_u32_le(bytes, offset + 12),
        prototype_state_enum: read_u32_le(bytes, offset + 16),
        simple_state_1_enum: read_u32_le(bytes, offset + 20),
        simple_state_2_enum: read_u32_le(bytes, offset + 24),
        simple_state_3_enum: read_u32_le(bytes, offset + 28),
        prototype_iterations: read_u32_le(bytes, offset + 32),
        phase_progress_raw: read_u32_le(bytes, offset + 36),
        phase_target_raw: read_u32_le(bytes, offset + 40),
        state_flags: read_u32_le(bytes, offset + 44),
    }
}

fn read_breakthrough_row(bytes: &[u8], offset: usize) -> ProjectBreakthroughRow {
    ProjectBreakthroughRow {
        tag_db_id: read_u32_le(bytes, offset),
        specialization_handle: read_u32_le(bytes, offset + 4),
        points_raw: read_u32_le(bytes, offset + 8),
    }
}

fn validate_state_row(idx: usize, row: &ProjectStateRow) -> Result<(), String> {
    if row.id == 0 || row.id > MAX_PROJECT_ID {
        return Err(format!("state row {idx} invalid id {}", row.id));
    }
    if row.static_def_handle == 0 || row.static_def_handle > MAX_PROJECT_HANDLE {
        return Err(format!(
            "state row {idx} invalid static_def_handle {}",
            row.static_def_handle
        ));
    }
    if !matches!(row.current_state_offset, 0x18 | 0xE8 | 0x168 | 0x1E8) {
        return Err(format!(
            "state row {idx} invalid current_state_offset {:#x}",
            row.current_state_offset
        ));
    }
    for (name, value) in [
        ("current_state_enum", row.current_state_enum),
        ("prototype_state_enum", row.prototype_state_enum),
        ("simple_state_1_enum", row.simple_state_1_enum),
        ("simple_state_2_enum", row.simple_state_2_enum),
        ("simple_state_3_enum", row.simple_state_3_enum),
    ] {
        if value > MAX_PROJECT_STATE_ENUM {
            return Err(format!("state row {idx} invalid {name} {value}"));
        }
    }
    if row.prototype_iterations > MAX_PROJECT_ITERATIONS {
        return Err(format!(
            "state row {idx} invalid prototype_iterations {}",
            row.prototype_iterations
        ));
    }
    if row.phase_progress_raw > MAX_PROJECT_PROGRESS_RAW {
        return Err(format!(
            "state row {idx} invalid phase_progress_raw {}",
            row.phase_progress_raw
        ));
    }
    if row.phase_target_raw == 0 || row.phase_target_raw > MAX_PROJECT_PHASE_TARGET_RAW {
        return Err(format!(
            "state row {idx} invalid phase_target_raw {}",
            row.phase_target_raw
        ));
    }
    if row.state_flags & !PROJECT_STATE_FLAG_COMPLETED != 0 {
        return Err(format!(
            "state row {idx} invalid state_flags {:#x}",
            row.state_flags
        ));
    }
    Ok(())
}

fn validate_breakthrough_row(idx: usize, row: &ProjectBreakthroughRow) -> Result<(), String> {
    if row.tag_db_id == 0 {
        return Err(format!("breakthrough row {idx} invalid tag_db_id"));
    }
    if row.specialization_handle == 0 || row.specialization_handle > MAX_PROJECT_HANDLE {
        return Err(format!(
            "breakthrough row {idx} invalid specialization_handle {}",
            row.specialization_handle
        ));
    }
    if row.points_raw > MAX_PROJECT_BREAKTHROUGH_POINTS {
        return Err(format!(
            "breakthrough row {idx} invalid points_raw {}",
            row.points_raw
        ));
    }
    Ok(())
}

fn completion_flag_value(state_flags: u32) -> u32 {
    if state_flags & PROJECT_STATE_FLAG_COMPLETED != 0 {
        1
    } else {
        0
    }
}

fn validate_lifecycle_event(idx: usize, event: &ProjectLifecycleEvent) -> Result<(), String> {
    if event.before == event.after {
        return Err(format!("event row {idx} has no value change"));
    }
    let expected_delta = event.after as i64 - event.before as i64;
    if event.delta != expected_delta {
        return Err(format!(
            "event row {idx} delta {} != expected {expected_delta}",
            event.delta
        ));
    }
    match event.kind {
        PROJECT_EVENT_KIND_COMPLETION_CHANGED => {
            if event.key_a == 0 || event.key_a > MAX_PROJECT_ID {
                return Err(format!(
                    "event row {idx} invalid project id {}",
                    event.key_a
                ));
            }
            if event.key_b == 0 || event.key_b > MAX_PROJECT_HANDLE {
                return Err(format!(
                    "event row {idx} invalid static_def_handle {}",
                    event.key_b
                ));
            }
            if event.before > 1 || event.after > 1 {
                return Err(format!("event row {idx} invalid completion transition"));
            }
        }
        PROJECT_EVENT_KIND_BREAKTHROUGH_DELTA => {
            if event.key_a == 0 {
                return Err(format!("event row {idx} invalid tag_db_id"));
            }
            if event.key_b == 0 || event.key_b > MAX_PROJECT_HANDLE {
                return Err(format!(
                    "event row {idx} invalid specialization_handle {}",
                    event.key_b
                ));
            }
            if event.before > MAX_PROJECT_BREAKTHROUGH_POINTS
                || event.after > MAX_PROJECT_BREAKTHROUGH_POINTS
            {
                return Err(format!("event row {idx} invalid breakthrough points"));
            }
        }
        other => {
            return Err(format!("event row {idx} unknown kind {other}"));
        }
    }
    Ok(())
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_i64_le(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
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
        fn write_u8(&mut self, addr: usize, value: u8) {
            self.bytes.insert(addr, value);
        }

        fn write_u32(&mut self, addr: usize, value: u32) {
            for (idx, byte) in value.to_le_bytes().iter().copied().enumerate() {
                self.bytes.insert(addr + idx, byte);
            }
        }

        fn write_u64(&mut self, addr: usize, value: u64) {
            for (idx, byte) in value.to_le_bytes().iter().copied().enumerate() {
                self.bytes.insert(addr + idx, byte);
            }
        }
    }

    impl ProjectMemory for FakeMemory {
        fn read_u8(&self, addr: usize) -> Option<u8> {
            self.bytes.get(&addr).copied()
        }

        fn read_u32(&self, addr: usize) -> Option<u32> {
            let mut out = [0u8; 4];
            for (idx, byte) in out.iter_mut().enumerate() {
                *byte = self.bytes.get(&(addr + idx)).copied()?;
            }
            Some(u32::from_le_bytes(out))
        }

        fn read_u64(&self, addr: usize) -> Option<u64> {
            let mut out = [0u8; 8];
            for (idx, byte) in out.iter_mut().enumerate() {
                *byte = self.bytes.get(&(addr + idx)).copied()?;
            }
            Some(u64::from_le_bytes(out))
        }
    }

    #[test]
    fn extractor_reads_project_state_from_validated_state_machine() {
        let base = 0x1400_0000usize;
        let project_vtable = base + 0x2A1_9300;
        let layout = ProjectLayout::from_module_base(base, 0x2A1_9300);
        let project = 0x2000_0000usize;
        let sm = 0x2100_0000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(project, project_vtable as u64);
        mem.write_u64(project + PROJECT_PROJECT_OFF_STATE_MACHINE_PTR, sm as u64);
        mem.write_u64(sm, layout.state_machine_vtable as u64);
        mem.write_u8(sm + PROJECT_STATE_MACHINE_OFF_NOT_COMPLETED_FLAG, 0);
        mem.write_u64(
            sm + PROJECT_STATE_MACHINE_OFF_CURRENT_STATE_PTR,
            (sm + PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE) as u64,
        );
        mem.write_u64(
            sm + PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE,
            layout.prototype_state_vtable as u64,
        );
        mem.write_u64(
            sm + PROJECT_STATE_MACHINE_OFF_SIMPLE_1,
            layout.simple_state_vtable as u64,
        );
        mem.write_u64(
            sm + PROJECT_STATE_MACHINE_OFF_SIMPLE_2,
            layout.simple_state_vtable as u64,
        );
        mem.write_u64(
            sm + PROJECT_STATE_MACHINE_OFF_SIMPLE_3,
            layout.simple_state_vtable as u64,
        );
        mem.write_u32(
            sm + PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE + PROJECT_STATE_OFF_CURRENT_STATE_ENUM,
            0,
        );
        mem.write_u32(
            sm + PROJECT_STATE_MACHINE_OFF_SIMPLE_1 + PROJECT_STATE_OFF_CURRENT_STATE_ENUM,
            1,
        );
        mem.write_u32(
            sm + PROJECT_STATE_MACHINE_OFF_SIMPLE_2 + PROJECT_STATE_OFF_CURRENT_STATE_ENUM,
            2,
        );
        mem.write_u32(
            sm + PROJECT_STATE_MACHINE_OFF_SIMPLE_3 + PROJECT_STATE_OFF_CURRENT_STATE_ENUM,
            3,
        );
        mem.write_u32(
            sm + PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE + PROJECT_PROTOTYPE_STATE_OFF_ITERATIONS,
            7,
        );
        mem.write_u32(
            sm + PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE
                + PROJECT_PROTOTYPE_STATE_OFF_PHASE_PROGRESS_RAW,
            3_000_000,
        );
        mem.write_u32(
            sm + PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE
                + PROJECT_PROTOTYPE_STATE_OFF_PHASE_TARGET,
            60,
        );

        let row = read_project_state_row(
            &mem,
            ProjectPin {
                addr: project,
                id: 91_234,
                static_def_handle: 6_655,
            },
            layout,
        )
        .unwrap();

        assert_eq!(row.current_state_offset, 0x18);
        assert_eq!(row.simple_state_3_enum, 3);
        assert_eq!(row.phase_progress_raw, 3_000_000);
        assert_eq!(row.phase_target_raw, 60);
        assert_eq!(row.state_flags, PROJECT_STATE_FLAG_COMPLETED);
    }

    #[test]
    fn extractor_reads_embedded_breakthrough_progress_tree() {
        let base = 0x1400_0000usize;
        let layout = ProjectLayout::from_module_base(base, 0x2A1_9300);
        let breakthrough = 0x2200_0000usize;
        let header = 0x2300_0000usize;
        let root = 0x2400_0000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(breakthrough, layout.breakthrough_vtable as u64);
        mem.write_u64(breakthrough + 0x08, header as u64);
        mem.write_u64(breakthrough + 0x10, 1);
        mem.write_u64(header + 0x08, root as u64);
        mem.write_u64(root, 0);
        mem.write_u64(root + 0x10, 0);
        mem.write_u8(root + 0x19, 0);
        mem.write_u32(root + 0x1C, 22_214);
        mem.write_u32(root + 0x20, 12_345);

        let rows =
            read_breakthrough_rows(&mem, breakthrough, layout.breakthrough_vtable, 47).unwrap();

        assert_eq!(
            rows,
            vec![ProjectBreakthroughRow {
                tag_db_id: 47,
                specialization_handle: 22_214,
                points_raw: 12_345,
            }]
        );
    }

    #[test]
    fn compact_frame_round_trips_and_gates() {
        let frame = LaneFrame {
            game_hour_raw: 123,
            status_flags: 0,
            state_rows: vec![ProjectStateRow {
                id: 91_234,
                static_def_handle: 6_655,
                current_state_offset: 0x18,
                current_state_enum: 0,
                prototype_state_enum: 0,
                simple_state_1_enum: 1,
                simple_state_2_enum: 2,
                simple_state_3_enum: 3,
                prototype_iterations: 7,
                phase_progress_raw: 3_000_000,
                phase_target_raw: 60,
                state_flags: PROJECT_STATE_FLAG_COMPLETED,
            }],
            breakthrough_rows: vec![ProjectBreakthroughRow {
                tag_db_id: 47,
                specialization_handle: 22_214,
                points_raw: 12_345,
            }],
        };
        let encoded = encode_lane_frame(&frame);
        assert_eq!(decode_lane_frame(&encoded).unwrap(), frame);
        let gate = gate_lane_frame(&encoded);
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.state_rows, 1);
        assert_eq!(gate.breakthrough_rows, 1);
    }

    #[test]
    fn compact_gate_rejects_fabricated_state_offset() {
        let frame = LaneFrame {
            game_hour_raw: 123,
            status_flags: 0,
            state_rows: vec![ProjectStateRow {
                id: 91_234,
                static_def_handle: 6_655,
                current_state_offset: 0x170,
                current_state_enum: 0,
                prototype_state_enum: 0,
                simple_state_1_enum: 1,
                simple_state_2_enum: 2,
                simple_state_3_enum: 3,
                prototype_iterations: 7,
                phase_progress_raw: 1,
                phase_target_raw: 60,
                state_flags: 0,
            }],
            breakthrough_rows: vec![ProjectBreakthroughRow {
                tag_db_id: 47,
                specialization_handle: 22_214,
                points_raw: 12_345,
            }],
        };
        let encoded = encode_lane_frame(&frame);
        let gate = gate_lane_frame(&encoded);
        assert!(!gate.ok);
        assert!(gate.errors[0].contains("current_state_offset"));
    }

    #[test]
    fn compact_gate_rejects_unknown_state_flags() {
        let frame = LaneFrame {
            game_hour_raw: 123,
            status_flags: 0,
            state_rows: vec![ProjectStateRow {
                id: 91_234,
                static_def_handle: 6_655,
                current_state_offset: 0x18,
                current_state_enum: 0,
                prototype_state_enum: 0,
                simple_state_1_enum: 1,
                simple_state_2_enum: 2,
                simple_state_3_enum: 3,
                prototype_iterations: 7,
                phase_progress_raw: 1,
                phase_target_raw: 60,
                state_flags: 0x8000_0000,
            }],
            breakthrough_rows: vec![ProjectBreakthroughRow {
                tag_db_id: 47,
                specialization_handle: 22_214,
                points_raw: 12_345,
            }],
        };
        let encoded = encode_lane_frame(&frame);
        let gate = gate_lane_frame(&encoded);
        assert!(!gate.ok);
        assert!(gate.errors[0].contains("state_flags"));
    }

    #[test]
    fn extractor_rejects_unknown_completed_flag_encoding() {
        let base = 0x1400_0000usize;
        let project_vtable = base + 0x2A1_9300;
        let layout = ProjectLayout::from_module_base(base, 0x2A1_9300);
        let project = 0x2000_0000usize;
        let sm = 0x2100_0000usize;
        let mut mem = FakeMemory::default();
        mem.write_u64(project, project_vtable as u64);
        mem.write_u64(project + PROJECT_PROJECT_OFF_STATE_MACHINE_PTR, sm as u64);
        mem.write_u64(sm, layout.state_machine_vtable as u64);
        mem.write_u8(sm + PROJECT_STATE_MACHINE_OFF_NOT_COMPLETED_FLAG, 2);

        let row = read_project_state_row(
            &mem,
            ProjectPin {
                addr: project,
                id: 91_234,
                static_def_handle: 6_655,
            },
            layout,
        );

        assert_eq!(row, None);
    }

    #[test]
    fn lifecycle_delta_events_round_trip_and_gate() {
        let previous = LaneFrame {
            game_hour_raw: 123,
            status_flags: 0,
            state_rows: vec![ProjectStateRow {
                id: 91_234,
                static_def_handle: 6_655,
                current_state_offset: 0x18,
                current_state_enum: 0,
                prototype_state_enum: 0,
                simple_state_1_enum: 1,
                simple_state_2_enum: 2,
                simple_state_3_enum: 3,
                prototype_iterations: 7,
                phase_progress_raw: 1,
                phase_target_raw: 60,
                state_flags: 0,
            }],
            breakthrough_rows: vec![ProjectBreakthroughRow {
                tag_db_id: 47,
                specialization_handle: 22_214,
                points_raw: 12_345,
            }],
        };
        let mut next = previous.clone();
        next.game_hour_raw = 124;
        next.state_rows[0].state_flags = PROJECT_STATE_FLAG_COMPLETED;
        next.breakthrough_rows[0].points_raw = 12_405;

        let events = derive_lifecycle_events(&previous, &next);
        assert_eq!(
            events,
            vec![
                ProjectLifecycleEvent {
                    kind: PROJECT_EVENT_KIND_COMPLETION_CHANGED,
                    key_a: 91_234,
                    key_b: 6_655,
                    before: 0,
                    after: 1,
                    delta: 1,
                },
                ProjectLifecycleEvent {
                    kind: PROJECT_EVENT_KIND_BREAKTHROUGH_DELTA,
                    key_a: 47,
                    key_b: 22_214,
                    before: 12_345,
                    after: 12_405,
                    delta: 60,
                },
            ]
        );

        let encoded = encode_lifecycle_events(&events);
        assert_eq!(decode_lifecycle_events(&encoded).unwrap(), events);
        let gate = gate_lifecycle_events(&encoded);
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.events, 2);
    }

    #[test]
    fn lifecycle_gate_rejects_malformed_delta() {
        let event = ProjectLifecycleEvent {
            kind: PROJECT_EVENT_KIND_BREAKTHROUGH_DELTA,
            key_a: 47,
            key_b: 22_214,
            before: 12_345,
            after: 12_405,
            delta: 1,
        };
        let encoded = encode_lifecycle_events(&[event]);
        let gate = gate_lifecycle_events(&encoded);
        assert!(!gate.ok);
        assert!(gate.errors[0].contains("delta"));
    }
}
