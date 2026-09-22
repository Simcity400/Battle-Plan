//! CStrategicNavy cached regional operands, consumed by transport movement.
use crate::{Memory, ccountry, cpdx_array::Array, read};

/// CStrategicNavy primary vtable; constructor 140E88190 installs 14295C640.
pub const VTABLE_RVA: usize = 0x0295_c640;
/// CStrategicNavy secondary vtable; constructor 140E88190 installs 14295C670 at +8.
pub const SECONDARY_VTABLE_RVA: usize = 0x0295_c670;
/// CStrategicNavy secondary subobject displacement; constructor 140E88190.
pub const SECONDARY: usize = 8;
/// CStrategicNavy owner raw country tag; 140E88190 stores +10, E90910 resolves it.
pub const OWNER: usize = 0x10;
/// CStrategicNavy CPdxArray<i64>; 140E88190 constructs +C8, E90CD0 indexes by region id.
pub const REGIONAL_OPERANDS: usize = 0xc8;

fn with_identity<T>(
    memory: &impl Memory,
    base: usize,
    state: usize,
    navy: usize,
    country: ccountry::Country,
    consume: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let primary = base.checked_add(VTABLE_RVA).ok_or("navy vtable overflow")? as u64;
    let secondary = base
        .checked_add(SECONDARY_VTABLE_RVA)
        .ok_or("navy vtable overflow")? as u64;
    let secondary_at = navy
        .checked_add(SECONDARY)
        .ok_or("navy identity overflow")?;
    let owner_at = navy.checked_add(OWNER).ok_or("navy owner overflow")?;
    let owner = read::<u32>(memory, owner_at).ok_or("navy owner unreadable")?;
    if navy == 0
        || read::<u64>(memory, navy) != Some(primary)
        || read::<u64>(memory, secondary_at) != Some(secondary)
        || ccountry::Reader::new(memory, base, state)?
            .resolve(owner)?
            .pointer
            != country.pointer
    {
        return Err("navy identity or owner mismatch");
    }
    consume()
}

/// E90910 resolves the owning navy's +10 tag without reading its regional operand.
pub(crate) fn authenticate_owner(
    memory: &impl Memory,
    base: usize,
    state: usize,
    navy: usize,
    country: ccountry::Country,
) -> Result<(), &'static str> {
    with_identity(memory, base, state, navy, country, || Ok(()))
}

pub(crate) fn regional_operand(
    memory: &impl Memory,
    base: usize,
    state: usize,
    navy: usize,
    country: ccountry::Country,
    region: u32,
) -> Result<i64, &'static str> {
    let array = regional_inputs(memory, base, state, navy, country)?;
    let at = array
        .element(region, 8)
        .ok_or("navy regional operand outside array")?;
    read::<i64>(memory, at).ok_or("navy regional operand unreadable")
}

/// Current authenticated array geometry; callers may share it only inside the
/// same frozen acquisition, and must still read each requested region's value.
pub(crate) fn regional_inputs(
    memory: &impl Memory,
    base: usize,
    state: usize,
    navy: usize,
    country: ccountry::Country,
) -> Result<Array, &'static str> {
    with_identity(memory, base, state, navy, country, || {
        let root = navy
            .checked_add(REGIONAL_OPERANDS)
            .ok_or("navy regional root overflow")?;
        Array::read(memory, root, 8).ok_or("navy regional array invalid")
    })
}
