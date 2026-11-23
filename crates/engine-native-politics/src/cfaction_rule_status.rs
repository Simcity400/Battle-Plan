
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_allocation as allocation;

pub const STATUS: usize = 0x570;
pub const VTABLE_RVA: Rva = Rva(0x02a9_bb58);
pub const OWNER: usize = 0x08;
pub const RULES: usize = 0x58;
pub const DEFINITION_VTABLE_RVA: Rva = Rva(0x0279_7ee8);
pub const DEFINITION_TOKEN: usize = 0x08;

pub fn read_into(
    memory: &impl Memory,
    base: usize,
    faction: usize,
    output: &mut Vec<u32>,
) -> Result<(), &'static str> {
    let status = faction
        .checked_add(STATUS)
        .ok_or("faction rule status address overflow")?;
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("faction rule vtable address overflow")?;
    if read::<u64>(memory, status) != Some(expected as u64) {
        return Err("faction rule status vtable mismatch");
    }
    if status
        .checked_add(OWNER)
        .and_then(|address| read::<u64>(memory, address))
        != Some(faction as u64)
    {
        return Err("faction rule status owner mismatch");
    }
    let array = status
        .checked_add(RULES)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("faction selected rule array unreadable or invalid")?;
    allocation::resize_default(output, array.count as usize)
        .map_err(|_| "faction selected rule storage allocation failed")?;
    let definition_vtable =
        engine_sites::at(base, DEFINITION_VTABLE_RVA)
            .ok_or("faction rule definition vtable address overflow")? as u64;
    for (ordinal, value) in output.iter_mut().enumerate() {
        let pointer = array
            .element(ordinal as u32, 8)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("faction selected rule pointer unreadable or null")?;
        if read::<u64>(memory, pointer) != Some(definition_vtable) {
            return Err("faction rule definition vtable mismatch");
        }
        *value = pointer
            .checked_add(DEFINITION_TOKEN)
            .and_then(|address| read(memory, address))
            .ok_or("faction rule definition token unreadable")?;
    }
    Ok(())
}
