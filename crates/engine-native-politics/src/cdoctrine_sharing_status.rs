
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_allocation as allocation;
use crate::cdoctrine_folder_template as folder;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a3_8780);
pub const FOLDERS: usize = 0x08;

pub fn read_into(
    memory: &impl Memory,
    base: usize,
    status: usize,
    output: &mut Vec<u32>,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("faction doctrine status vtable address overflow")? as u64;
    if read::<u64>(memory, status) != Some(expected) {
        return Err("faction doctrine sharing status vtable mismatch");
    }
    let source = status
        .checked_add(FOLDERS)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("faction doctrine sharing folders unreadable or invalid")?;
    allocation::resize_default(output, source.count as usize)
        .map_err(|_| "faction doctrine sharing allocation failed")?;
    let expected = engine_sites::at(base, folder::VTABLE_RVA)
        .ok_or("faction doctrine folder vtable address overflow")? as u64;
    for (ordinal, value) in output.iter_mut().enumerate() {
        let pointer = source
            .element(ordinal as u32, 8)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value != 0)
            .ok_or("faction doctrine sharing folder unreadable or null")?;
        if read::<u64>(memory, pointer) != Some(expected) {
            return Err("faction doctrine folder vtable mismatch");
        }
        *value = pointer
            .checked_add(folder::TOKEN)
            .and_then(|address| read(memory, address))
            .ok_or("faction doctrine folder token unreadable")?;
    }
    Ok(())
}
