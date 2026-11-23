
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};

pub const VTABLE_RVA: Rva = Rva(0x029e_aec0);
pub const COUNTRY: usize = 0x08;
pub const ACTIONS: usize = 0x10;

pub fn copy(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(u32, Array), &'static str> {
    let identity =
        read::<[u8; 12]>(memory, pointer).ok_or("peace bidding turn identity unreadable")?;
    if engine_sites::at(base, VTABLE_RVA).map(|value| value as u64)
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
