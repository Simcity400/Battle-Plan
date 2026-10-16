//! Retained reduced territory outcomes, consumed before finalization.

use crate::{Memory, cpdx_array::Array, read};

/// Raw country reference used when applying state ownership.
pub const COUNTRY: usize = 0x08;
/// Ordered state pointers, appended and applied without clearing.
pub const STATES: usize = 0x18;

/// Caller authenticates the concrete participant before reading its base.
pub fn copy(memory: &impl Memory, pointer: usize) -> Result<(u32, Array), &'static str> {
    let country = pointer
        .checked_add(COUNTRY)
        .and_then(|at| read(memory, at))
        .ok_or("peace beneficiary country unreadable")?;
    let states = pointer
        .checked_add(STATES)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("peace beneficiary state array unreadable or invalid")?;
    Ok((country, states))
}
