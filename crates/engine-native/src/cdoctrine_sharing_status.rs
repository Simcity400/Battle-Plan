//! Ordered doctrine folders retained by faction doctrine sharing.

use crate::{Memory, allocation, cdoctrine_folder_template as folder, cpdx_array::Array, read};

pub const VTABLE_RVA: usize = 0x029a_6e20;
pub const FOLDERS: usize = 0x08;

/// Read the companion reached through an already authenticated upgrade status.
pub fn read_into(
    memory: &impl Memory,
    base: usize,
    status: usize,
    output: &mut Vec<u32>,
) -> Result<(), &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
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
    let expected = base
        .checked_add(folder::VTABLE_RVA)
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
