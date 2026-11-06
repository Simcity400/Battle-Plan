//! CConferenceWinnerParticipant automatic civil-war outcomes.

use crate::{Memory, cpdx_array::Array, read};

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02a3_c848;
/// Raw winner country reference used when applying state ownership.
pub const COUNTRY: usize = 8;
/// Ordered signed state IDs, appended and then consumed during settlement.
pub const AUTOMATIC_STATES: usize = 0x148;

pub fn copy(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(u32, Array), &'static str> {
    let bytes =
        read::<[u8; 12]>(memory, pointer).ok_or("peace winner participant identity unreadable")?;
    if base.checked_add(VTABLE_RVA).map(|value| value as u64)
        != Some(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
    {
        return Err("peace winner participant vtable mismatch");
    }
    let country = u32::from_le_bytes(bytes[COUNTRY..].try_into().unwrap());
    let states = pointer
        .checked_add(AUTOMATIC_STATES)
        .and_then(|at| Array::read(memory, at, 4))
        .ok_or("peace automatic transfer array unreadable or invalid")?;
    Ok((country, states))
}
