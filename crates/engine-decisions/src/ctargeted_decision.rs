//! Targeted-decision records, including timed-targeted entries. Preserve raw
//! lifecycle codes and numeric targets until definition-specific meaning is
//! established.

use crate::Memory;

/// Targeted-decision primary vtable.
pub const VTABLE_RVA: usize = 0x0286_86a8;
/// Decision-definition pointer.
pub const DEFINITION: usize = 0x10;
/// Country-target reference, u32.
pub const COUNTRY_TARGET: usize = 0x20;
/// Numeric target, u32, requiring definition-specific interpretation.
pub const NUMERIC_TARGET: usize = 0x24;
/// Instance-relative selector view passed to authored effect execution.
pub const SELECTOR_VIEW: usize = 0x18;
/// Target-selector view passed to authored effect functions.
pub const SELECTOR_COUNTRY: usize = 0x08;
pub const SELECTOR_NUMERIC: usize = 0x0c;
/// Lifecycle code, i32.
pub const STATE: usize = 0x28;
/// Ignore byte accepting zero or one.
pub const IGNORE: usize = 0x2c;
/// Remaining days, i32, preserving negative cooldown values.
pub const DAYS: usize = 0x30;

/// Scope decoded from the signed country-reference slot. This discriminator
/// is not stored as an enum; numeric targets remain numeric until their
/// definition-specific type is established.
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
