//! CTargetedDecision records, including those in CDecisionStatus's timed-targeted array.
//! The old normal/timed arrays assigned different names to the same lifecycle codes;
//! preserve raw codes until engine semantics are rechecked. Numeric targets were treated as state ids.

use crate::Memory;

/// CTargetedDecision primary vtable; build_profile.rs:797, pinned-exe RTTI corrects the old TARGETED_TIMED_DECISION name.
pub const VTABLE_RVA: usize = 0x027d_5038;
/// CTargetedDecision CDecision definition pointer; build_profile.rs:798, read_active_targeted_decisions at attach_log.rs:171371.
pub const DEFINITION: usize = 0x10;
/// CTargetedDecision u32 country target reference; build_profile.rs:799, read_active_targeted_decisions at attach_log.rs:171405.
pub const COUNTRY_TARGET: usize = 0x20;
/// CTargetedDecision u32 numeric target; build_profile.rs:800, old reader treated it as a state id at attach_log.rs:171416.
pub const NUMERIC_TARGET: usize = 0x24;
/// CTargetedDecision i32 lifecycle code; build_profile.rs:801, read_active_targeted_decisions at attach_log.rs:171439.
pub const STATE: usize = 0x28;
/// CTargetedDecision ignore byte; build_profile.rs:802, read_active_targeted_decisions at attach_log.rs:171454 accepts zero or one.
pub const IGNORE: usize = 0x2c;
/// CTargetedDecision i32 remaining days; build_profile.rs:803, read_active_targeted_decisions at attach_log.rs:171467 preserves negative cooldown days.
pub const DAYS: usize = 0x30;

/// This is a decoded discriminator, not a stored native enum. Numeric targets
/// remain numeric until a definition-specific type is proved.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TargetKind {
    #[default]
    None = 0,
    Country = 1,
    Numeric = 2,
}

impl TargetKind {
    pub fn from_country_reference(country: u32) -> Self {
        if (country as i32) > 0 {
            Self::Country
        } else {
            Self::Numeric
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Record {
    pub definition: usize,
    pub country_target: u32,
    pub numeric_target: u32,
    pub lifecycle: i32,
    pub ignore: u8,
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
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("targeted decision address overflow")
    };
    let mut vtable = [0; 8];
    let mut definition = [0; 8];
    let mut target_and_state = [0; 13];
    let mut days = [0; 4];
    if memory.copy_scattered(
        [pointer, at(DEFINITION)?, at(COUNTRY_TARGET)?, at(DAYS)?],
        &mut [
            &mut vtable,
            &mut definition,
            &mut target_and_state,
            &mut days,
        ],
    ) != 4
    {
        return Err("targeted decision unreadable");
    }
    if u64::from_le_bytes(vtable) != expected as u64 {
        return Err("targeted decision vtable mismatch");
    }
    let ignore = target_and_state[12];
    if ignore > 1 {
        return Err("targeted decision ignore flag invalid");
    }
    Ok(Record {
        definition: usize::try_from(u64::from_le_bytes(definition))
            .map_err(|_| "decision definition pointer overflow")?,
        country_target: u32::from_le_bytes(target_and_state[..4].try_into().unwrap()),
        numeric_target: u32::from_le_bytes(target_and_state[4..8].try_into().unwrap()),
        lifecycle: i32::from_le_bytes(target_and_state[8..12].try_into().unwrap()),
        ignore,
        days: i32::from_le_bytes(days),
    })
}
