
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_allocation as allocation;

pub const STATUS: usize = 0x7c8;
pub const VTABLE_RVA: Rva = Rva(0x029e_2b08);
pub const PROGRAMS: usize = 0x08;

pub fn read_into(
    memory: &impl Memory,
    base: usize,
    faction: usize,
    output: &mut Vec<u64>,
) -> Result<(), &'static str> {
    let status = faction
        .checked_add(STATUS)
        .ok_or("faction programme status address overflow")?;
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("faction programme vtable address overflow")?;
    if read::<u64>(memory, status) != Some(expected as u64) {
        return Err("faction programme status vtable mismatch");
    }
    let array = status
        .checked_add(PROGRAMS)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("faction programme array unreadable or invalid")?;
    allocation::resize_default(output, array.count as usize)
        .map_err(|_| "faction programme storage allocation failed")?;
    for (ordinal, value) in output.iter_mut().enumerate() {
        *value = array
            .element(ordinal as u32, 8)
            .and_then(|address| read(memory, address))
            .ok_or("faction programme reference unreadable")?;
    }
    Ok(())
}
