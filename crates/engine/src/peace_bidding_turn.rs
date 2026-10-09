//! CPeaceBiddingTurn, owned by one conference history node until 140BAE7C0.

use crate::{Memory, cpdx_array::Array, read};

/// Primary vtable: a729 RTTI and constructors 1419E0B20/1419E0CD0/1419E0D30.
pub const VTABLE_RVA: usize = 0x0295_7900;
/// Raw country reference; 1419E1A20 serializes it.
pub const COUNTRY: usize = 0x08;
/// Ordered owning action-pointer array; 1419E1A20 serializes it, E23760 deletes it.
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
