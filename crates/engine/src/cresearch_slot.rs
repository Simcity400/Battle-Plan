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
/// `CResearchSlot::SetTechnology@140ECE4F0` stores this lexer `empty` token
/// to slot+8 at 140ECE9BC when slot+0x18 has no technology; `fixed_layout`
/// checks its spelling and static range.
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
    let mut raw = Raw::default();
    let completed = copy_raw(memory, pointer, &mut raw)?;
    if completed != 4 {
        return Err("research slot fields unreadable");
    }
    let technology_u64 = u64::from_le_bytes(raw.owner[8..].try_into().unwrap());
    let linked = if technology_u64 == 0 {
        None
    } else {
        let technology =
            usize::try_from(technology_u64).map_err(|_| "research technology address overflow")?;
        let mut bytes = [0; crate::ctechnology::INSTANCE_BYTES];
        if crate::ctechnology::copy_one(memory, technology, &mut bytes)
            != crate::ctechnology::FIELDS.len()
        {
            return Err("technology fields unreadable");
        }
        Some(bytes)
    };
    decode_raw(base, status, country, &raw, linked.as_ref())
}

/// Game-thread raw copy with no validation beyond address bounds.
/// Returns completed scattered fields; failed field may be partial.
pub fn copy_raw(
    memory: &impl crate::Memory,
    pointer: usize,
    out: &mut Raw,
) -> Result<usize, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("research slot address overflow")
    };
    let mut header: &mut [u8] = &mut out.header;
    let mut owner: &mut [u8] = &mut out.owner;
    let mut speed_index: &mut [u8] = &mut out.speed_index;
    let mut days: &mut [u8] = &mut out.days;
    Ok(memory.copy_scattered(
        [pointer, at(STATUS)?, at(RESEARCH_SPEED)?, at(BANKED_DAYS)?],
        &mut [&mut header, &mut owner, &mut speed_index, &mut days],
    ))
}

/// Decode-side interpretation of already-copied slot bytes with identical faults.
pub fn decode_raw(
    base: usize,
    status: usize,
    country: u32,
    raw: &Raw,
    linked: Option<&[u8; crate::ctechnology::INSTANCE_BYTES]>,
) -> Result<Row, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    if u64::from_le_bytes(raw.header[..8].try_into().unwrap()) != expected {
        return Err("research slot vtable mismatch");
    }
    if u64::from_le_bytes(raw.owner[..8].try_into().unwrap()) != status as u64 {
        return Err("research slot status backlink mismatch");
    }
    let technology_handle = u32::from_le_bytes(raw.header[8..].try_into().unwrap());
    let technology = usize::try_from(u64::from_le_bytes(raw.owner[8..].try_into().unwrap()))
        .map_err(|_| "research technology address overflow")?;
    if technology == 0 {
        if technology_handle != IDLE_HANDLE {
            return Err("idle research slot handle mismatch");
        }
    } else {
        let Some(bytes) = linked else {
            return Err("technology fields unreadable");
        };
        let instance = crate::ctechnology::decode_one(base, status, country, bytes)?;
        if instance.row.technology_handle != technology_handle {
            return Err("research slot technology handle mismatch");
        }
    }
    Ok(Row {
        country,
        index: u32::from_le_bytes(raw.speed_index[8..].try_into().unwrap()),
        technology_handle,
        technology_present: u8::from(technology != 0),
        research_speed_raw: i64::from_le_bytes(raw.speed_index[..8].try_into().unwrap()),
        banked_days_raw: i64::from_le_bytes(raw.days[..8].try_into().unwrap()),
        applied_days_raw: i64::from_le_bytes(raw.days[8..].try_into().unwrap()),
    })
}

/// Flat raw spans for one slot plus its linked-technology presence.
/// Game thread fills header/owner/speed/days via guarded copies only.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Raw {
    pub header: [u8; 12],
    pub owner: [u8; 16],
    pub speed_index: [u8; 12],
    pub days: [u8; 16],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_slot_requires_the_shared_empty_handle() {
        let mut raw = Raw::default();
        raw.header[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
        raw.header[8..].copy_from_slice(&IDLE_HANDLE.to_le_bytes());
        let idle = decode_raw(0, 0, 7, &raw, None).unwrap();
        assert_eq!(idle.technology_handle, IDLE_HANDLE);
        assert_eq!(idle.technology_present, 0);
        for handle in [0, IDLE_HANDLE - 1, IDLE_HANDLE + 1] {
            raw.header[8..].copy_from_slice(&handle.to_le_bytes());
            assert_eq!(
                decode_raw(0, 0, 7, &raw, None),
                Err("idle research slot handle mismatch")
            );
        }
    }
}
