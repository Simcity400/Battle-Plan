//! Timed-decision lifecycle records.

use crate::Memory;

/// CTimedDecision primary vtable.
pub const VTABLE_RVA: usize = 0x027e_b4a0;
/// CTimedDecision CDecision definition pointer.
pub const DEFINITION: usize = 0x10;
/// CTimedDecision i32 remaining days.
pub const DAYS: usize = 0x18;
/// CTimedDecision i32 lifecycle code.
pub const STATE: usize = 0x1c;
/// Active lifecycle code; zero selects an active record.
pub const STATE_ACTIVE: i32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Record {
    pub definition: usize,
    pub days: i32,
    pub lifecycle: i32,
}

pub(crate) fn record(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Record, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    let fields = pointer
        .checked_add(DEFINITION)
        .ok_or("timed decision address overflow")?;
    let mut vtable = [0; 8];
    let mut bytes = [0; 16];
    if memory.copy_scattered([pointer, fields], &mut [&mut vtable, &mut bytes]) != 2 {
        return Err("timed decision unreadable");
    }
    if u64::from_le_bytes(vtable) != expected as u64 {
        return Err("timed decision vtable mismatch");
    }
    Ok(Record {
        definition: usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
            .map_err(|_| "decision definition pointer overflow")?,
        days: i32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        lifecycle: i32::from_le_bytes(bytes[12..16].try_into().unwrap()),
    })
}
