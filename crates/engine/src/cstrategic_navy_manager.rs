//! CStrategicNavyManager canonical-country-indexed cache.
use crate::{Memory, ccountry, cgame_state, cpdx_array::Array, cstrategic_navy, read};

/// CStrategicNavyManager primary vtable; constructor 140E88580 installs 14295C6C0.
pub const VTABLE_RVA: usize = 0x0295_c6c0;
/// CStrategicNavyManager CPdxArray<CStrategicNavy*>; constructor E88580 and getter6ECBF0.
pub const NAVIES: usize = 8;

fn with_navy<T>(
    memory: &impl Memory,
    base: usize,
    state: usize,
    country: ccountry::Country,
    consume: impl FnOnce(usize) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let root = state
        .checked_add(cgame_state::STRATEGIC_NAVY_MANAGER)
        .ok_or("navy manager root overflow")?;
    let manager = read::<u64>(memory, root)
        .filter(|&p| p != 0)
        .ok_or("navy manager absent or unreadable")? as usize;
    let vtable = base
        .checked_add(VTABLE_RVA)
        .ok_or("navy manager vtable overflow")? as u64;
    if read::<u64>(memory, manager) != Some(vtable) {
        return Err("navy manager identity mismatch");
    }
    let header = manager
        .checked_add(NAVIES)
        .ok_or("navy manager array overflow")?;
    let array = Array::read(memory, header, 8).ok_or("navy manager array invalid")?;
    let slot = array
        .element(country.canonical_slot, 8)
        .ok_or("navy manager country outside array")?;
    let navy = read::<u64>(memory, slot)
        .filter(|&p| p != 0)
        .ok_or("country navy absent or unreadable")?;
    consume(navy as usize)
}

pub(crate) fn authenticate_owner(
    memory: &impl Memory,
    base: usize,
    state: usize,
    country: ccountry::Country,
) -> Result<(), &'static str> {
    with_navy(memory, base, state, country, |navy| {
        cstrategic_navy::authenticate_owner(memory, base, state, navy, country)
    })
}

pub(crate) fn regional_operand(
    memory: &impl Memory,
    base: usize,
    state: usize,
    country: ccountry::Country,
    region: u32,
) -> Result<i64, &'static str> {
    with_navy(memory, base, state, country, |navy| {
        cstrategic_navy::regional_operand(memory, base, state, navy, country, region)
    })
}

pub(crate) fn regional_inputs(
    memory: &impl Memory,
    base: usize,
    state: usize,
    country: ccountry::Country,
) -> Result<Array, &'static str> {
    with_navy(memory, base, state, country, |navy| {
        cstrategic_navy::regional_inputs(memory, base, state, navy, country)
    })
}
