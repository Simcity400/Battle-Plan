//! Retained reduced territory outcomes, consumed by 141442C20 before finalization.

use crate::{Memory, cpdx_array::Array, read};

/// 141442C20 passes this raw country to CState::SetOwner.
pub const COUNTRY: usize = 0x08;
/// Ordered state pointers; 140E24170 appends, 141442C20 applies without clearing.
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
