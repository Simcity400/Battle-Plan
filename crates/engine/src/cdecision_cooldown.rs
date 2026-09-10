//! CDecisionCooldown state in CDecisionStatus's re-enable and removal arrays.

use crate::{Memory, read};

/// CDecisionCooldown primary vtable; build_profile.rs:810, confirmed by pinned-exe RTTI and read_decision_cooldowns.
pub const VTABLE_RVA: usize = 0x027d_4f80;
/// CDecisionCooldown CDecision pointer; build_profile.rs:811, read_decision_cooldowns at attach_log.rs:171268.
pub const DEFINITION: usize = 0x08;
/// CDecisionCooldown i32 remaining days; build_profile.rs:812, read_decision_cooldowns at attach_log.rs:171275 preserves signed values.
pub const DAYS: usize = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Record {
    pub definition: usize,
    pub days: i32,
}

pub(crate) fn record(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Record, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    let bytes = read::<[u8; 20]>(memory, pointer).ok_or("decision cooldown unreadable")?;
    if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected as u64 {
        return Err("decision cooldown vtable mismatch");
    }
    let definition = usize::try_from(u64::from_le_bytes(bytes[8..16].try_into().unwrap()))
        .map_err(|_| "decision definition pointer overflow")?;
    Ok(Record {
        definition,
        days: i32::from_le_bytes(bytes[16..20].try_into().unwrap()),
    })
}
