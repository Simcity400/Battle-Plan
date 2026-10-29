//! Peace bidding turn owned by one conference history node until destruction.

use crate::{Memory, cpdx_array::Array, read};

/// Bidding-turn primary vtable.
pub const VTABLE_RVA: usize = 0x029e_adb0;
/// Serialized raw country reference.
pub const COUNTRY: usize = 0x08;
/// Ordered owning action-pointer array, serialized before destruction.
pub const ACTIONS: usize = 0x10;

pub fn copy(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(u32, Array), &'static str> {
    let identity =
        read::<[u8; 12]>(memory, pointer).ok_or("peace bidding turn identity unreadable")?;
    if base.checked_add(VTABLE_RVA).map(|value| value as u64)
        != Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
    {
        return Err("peace bidding turn vtable mismatch");
    }
    let country = u32::from_le_bytes(identity[COUNTRY..].try_into().unwrap());
    let actions = pointer
        .checked_add(ACTIONS)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("peace bidding action array unreadable or invalid")?;
    Ok((country, actions))
}
