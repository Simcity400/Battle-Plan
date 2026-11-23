use crate::{Memory, ccountry, cgame_state, cpdx_array::Array, cstrategic_navy, read};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029e_f9d0);
pub const NAVIES: usize = 8;

pub fn navies(memory: &impl Memory, base: usize, state: usize) -> Result<Array, &'static str> {
    let root = state
        .checked_add(cgame_state::STRATEGIC_NAVY_MANAGER)
        .ok_or("navy manager root overflow")?;
    let manager = read::<u64>(memory, root)
        .filter(|&p| p != 0)
        .ok_or("navy manager absent or unreadable")? as usize;
    let vtable = engine_sites::at(base, VTABLE_RVA).ok_or("navy manager vtable overflow")? as u64;
    if read::<u64>(memory, manager) != Some(vtable) {
        return Err("navy manager identity mismatch");
    }
    let header = manager
        .checked_add(NAVIES)
        .ok_or("navy manager array overflow")?;
    Array::read(memory, header, 8).ok_or("navy manager array invalid")
}

fn with_navy<T>(
    memory: &impl Memory,
    base: usize,
    state: usize,
    country: ccountry::Country,
    consume: impl FnOnce(usize) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let array = navies(memory, base, state)?;
    let slot = array
        .element(country.canonical_slot, 8)
        .ok_or("navy manager country outside array")?;
    let navy = read::<u64>(memory, slot)
        .filter(|&p| p != 0)
        .ok_or("country navy absent or unreadable")?;
    consume(navy as usize)
}

pub fn authenticate(
    memory: &impl Memory,
    base: usize,
    state: usize,
    country: ccountry::Country,
) -> Result<(), &'static str> {
    with_navy(memory, base, state, country, |navy| {
        cstrategic_navy::authenticate(memory, base, navy)
    })
}

pub fn navy(
    memory: &impl Memory,
    base: usize,
    state: usize,
    country: ccountry::Country,
) -> Result<usize, &'static str> {
    with_navy(memory, base, state, country, |navy| {
        cstrategic_navy::authenticate(memory, base, navy)?;
        Ok(navy)
    })
}

pub fn regional_operand(
    memory: &impl Memory,
    base: usize,
    state: usize,
    country: ccountry::Country,
    region: u32,
) -> Result<i64, &'static str> {
    with_navy(memory, base, state, country, |navy| {
        cstrategic_navy::regional_operand(memory, base, navy, region)
    })
}
