//! CTimedDecision lifecycle record retained from the old decision walker.

use crate::Memory;

/// CTimedDecision primary vtable; offsets.rs:288, confirmed by pinned-exe RTTI and read_timed_decision_record.
pub const VTABLE_RVA: usize = 0x027d_4fd0;
/// CTimedDecision CDecision definition pointer; offsets.rs:289, read_timed_decision_record at attach_log.rs:171521.
pub const DEFINITION: usize = 0x10;
/// CTimedDecision i32 remaining days; offsets.rs:290, read_timed_decision_record at attach_log.rs:171526.
pub const DAYS: usize = 0x18;
/// CTimedDecision i32 lifecycle code; offsets.rs:291, read_timed_decision_record at attach_log.rs:171530.
pub const STATE: usize = 0x1c;
/// CTimedDecision active lifecycle code; offsets.rs:293 and read_active_timed_decisions at attach_log.rs:171235 filter zero.
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
