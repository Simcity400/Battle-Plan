//! CResearchSlot fields from the archived a729 active-research walk.
//!
//! The vtable's MSVC RTTI names CResearchSlot in the pinned executable.
//! The slot holds a research-speed multiplier, not technology progress.
//! Sources at `archive/pre-rewrite-2026-09-09`: collector
//! `src/attach_log/offsets.rs:330-336` and
//! `src/attach_log.rs::read_country_active_research_slots`.

/// CResearchSlot vtable; offsets.rs:330, old active-slot identity check, and a729 RTTI agree.
pub const VTABLE_RVA: usize = 0x0295_e328;
/// CResearchSlot technology definition handle u32; old active-slot reader compares it to CTechnology+0x08.
pub const TECHNOLOGY_HANDLE: usize = 0x08;
/// CResearchSlot CTechnologyStatus* owner; old active-slot reader validates the parent pointer.
pub const STATUS: usize = 0x10;
/// CResearchSlot CTechnology*; offsets.rs:333 and old active-slot reader confirm a pointer at +0x18.
pub const TECHNOLOGY: usize = 0x18;
/// CResearchSlot banked idle days signed i64 fixed5; EC49E0 increments this qword by 100000 each idle day.
pub const BANKED_DAYS: usize = 0x20;
/// CResearchSlot applied saved days signed i64 fixed5; ECE4F0 moves the banked amount here on assignment.
pub const APPLIED_DAYS: usize = 0x28;
/// CResearchSlot signed i64 fixed5 research-speed multiplier; ECE4F0 writes EC1810's qword result and EBDB60 initializes 100000.
pub const RESEARCH_SPEED: usize = 0x38;
/// CResearchSlot native i32 ordinal; EBDB60 stores int, and the walker rejects negative/out-of-range bits against the positive array count.
pub const INDEX: usize = 0x40;
/// CResearchSlot idle handle; offsets.rs:336 and old active-slot reader skip this value (build-specific observation).
pub const IDLE_HANDLE: u32 = 0x2a4e;
/// CResearchSlot multiplier scale; ECE4F0 resets to 100000 and divides the days product by 100000.
pub const RESEARCH_SPEED_SCALE: i64 = 100_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    /// Nonnegative native i32 ordinal; all bits preserved and checked against the array count.
    pub index: u32,
    pub technology_handle: u32,
    /// Native technology pointer presence; the pointer itself is not portable identity.
    pub technology_present: u8,
    pub research_speed_raw: i64,
    pub banked_days_raw: i64,
    pub applied_days_raw: i64,
}

pub(crate) fn row(
    memory: &impl crate::Memory,
    base: usize,
    pointer: usize,
    status: usize,
    country: u32,
) -> Result<Row, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("research slot address overflow")
    };
    let mut header = [0; 12];
    let mut owner = [0; 16];
    let mut speed_index = [0; 12];
    let mut days = [0; 16];
    if memory.copy_scattered(
        [pointer, at(STATUS)?, at(RESEARCH_SPEED)?, at(BANKED_DAYS)?],
        &mut [&mut header, &mut owner, &mut speed_index, &mut days],
    ) != 4
    {
        return Err("research slot fields unreadable");
    }
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected {
        return Err("research slot vtable mismatch");
    }
    if u64::from_le_bytes(owner[..8].try_into().unwrap()) != status as u64 {
        return Err("research slot status backlink mismatch");
    }
    let technology_handle = u32::from_le_bytes(header[8..].try_into().unwrap());
    let technology = usize::try_from(u64::from_le_bytes(owner[8..].try_into().unwrap()))
        .map_err(|_| "research technology address overflow")?;
    if technology == 0 {
        if technology_handle != IDLE_HANDLE {
            return Err("idle research slot handle mismatch");
        }
    } else {
        let instance = crate::ctechnology::instance(memory, base, technology, status, country)?;
        if instance.row.technology_handle != technology_handle {
            return Err("research slot technology handle mismatch");
        }
    }
    Ok(Row {
        country,
        index: u32::from_le_bytes(speed_index[8..].try_into().unwrap()),
        technology_handle,
        technology_present: u8::from(technology != 0),
        research_speed_raw: i64::from_le_bytes(speed_index[..8].try_into().unwrap()),
        banked_days_raw: i64::from_le_bytes(days[..8].try_into().unwrap()),
        applied_days_raw: i64::from_le_bytes(days[8..].try_into().unwrap()),
    })
}
