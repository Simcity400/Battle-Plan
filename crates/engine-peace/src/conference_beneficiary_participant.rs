
use engine_native_memory::{Memory, cpdx_array::Array, read};

pub const COUNTRY: usize = 0x08;
pub const STATES: usize = 0x18;

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
