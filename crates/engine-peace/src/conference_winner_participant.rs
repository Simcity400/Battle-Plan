
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};

pub const VTABLE_RVA: Rva = Rva(0x02a3_c848);
pub const COUNTRY: usize = 8;
pub const AUTOMATIC_STATES: usize = 0x148;

pub fn copy(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(u32, Array), &'static str> {
    let bytes =
        read::<[u8; 12]>(memory, pointer).ok_or("peace winner participant identity unreadable")?;
    if engine_sites::at(base, VTABLE_RVA).map(|value| value as u64)
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
