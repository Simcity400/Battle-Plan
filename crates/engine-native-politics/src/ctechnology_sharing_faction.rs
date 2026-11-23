
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_allocation as allocation;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02ac_66b8);
pub const BASE: usize = 0x10;
pub const COUNTRIES: usize = 0x18;
pub const BONUSES: usize = 0x30;
pub const OWNER: usize = 0x48;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub base_raw: i64,
    pub countries: Vec<u32>,
    pub bonuses: Vec<i64>,
}

pub fn read_into(
    memory: &impl Memory,
    base: usize,
    faction: usize,
    status: usize,
    output: &mut Snapshot,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("faction research sharing vtable address overflow")? as u64;
    if read::<u64>(memory, status) != Some(expected) {
        return Err("faction research sharing status vtable mismatch");
    }
    if status
        .checked_add(OWNER)
        .and_then(|address| read::<u64>(memory, address))
        != Some(faction as u64)
    {
        return Err("faction research sharing owner mismatch");
    }
    output.base_raw = status
        .checked_add(BASE)
        .and_then(|address| read(memory, address))
        .ok_or("faction research sharing base unreadable")?;
    let countries = status
        .checked_add(COUNTRIES)
        .and_then(|address| Array::read(memory, address, 4))
        .ok_or("faction research sharing countries unreadable or invalid")?;
    allocation::resize_default(&mut output.countries, countries.count as usize)
        .map_err(|_| "faction sharing countries allocation failed")?;
    engine_native_memory::cpdx_array::copy_u32s(
        memory,
        countries,
        &mut output.countries,
        "faction sharing country reference unreadable",
    )?;
    let bonuses = status
        .checked_add(BONUSES)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("faction research sharing bonuses unreadable or invalid")?;
    allocation::resize_default(&mut output.bonuses, bonuses.count as usize)
        .map_err(|_| "faction sharing bonuses allocation failed")?;
    for (ordinal, value) in output.bonuses.iter_mut().enumerate() {
        *value = bonuses
            .element(ordinal as u32, 8)
            .and_then(|address| read(memory, address))
            .ok_or("faction research sharing bonus unreadable")?;
    }
    Ok(())
}
