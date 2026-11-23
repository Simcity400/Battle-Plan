pub use engine_native::cfleet::CensusVisit;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_sites::Rva;


pub const REPAIR_MODE: usize = 0x464;
pub const REPAIR_CHILDREN: usize = 0x498;
pub const MERGE_AFTER_REPAIR: usize = 0x4e0;
pub const REPAIR_SPLIT: usize = 0x4f4;
pub const AUTO_REINFORCEMENT: usize = 0x4f7;
pub const REFIT_VARIANT: usize = 0x4c8;
pub const REFIT_AFTER_REPAIR: usize = 0x4d0;
pub const COMPOSITION: usize = 0x530;
pub const REQUIREMENTS: usize = COMPOSITION + 8;
pub const FULFILLMENT: usize = COMPOSITION + 0x70;
pub const COMPOSITION_TOKEN: usize = COMPOSITION + 0xd8;
pub const NO_AI_COMPOSITION_TOKEN: u32 = 0x4c17;
pub const ENTRIES_VTABLE_RVA: Rva = Rva(0x029f_b6a8);
pub const COMPOSITION_ENTRIES: usize = 8;
pub const COMPOSITION_ENTRY_BYTES: usize = 16;
pub const AUTOMATED_HOMEBASE: usize = 0x104;
pub const MAX_FACTORIES_FOR_REPAIR: usize = 0x4c8;
pub const MAX_REPAIR_DOCKYARDS: usize = 0x1a0;
pub const USED_REPAIR_DOCKYARDS: usize = 0x1a4;
pub const BASE_REPAIR_DOCKYARDS: usize = 0x1c;
pub const BASE_REPAIR_PRIORITY: usize = 0x20;
pub const BASE_REPAIR_QUEUE: usize = 0x28;
pub const SHIP_REFIT_LINE: usize = 0x738;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskSettings {
    pub repair_mode: i32,
    pub repair_split: u8,
    pub repair_target: u32,
    pub auto_reinforcement: u8,
    pub refit_variant: u64,
    pub refit_after_repair: u64,
    pub composition_token: u32,
    pub merge_after_repair: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompositionEntry {
    pub sub_unit: i32,
    pub role: i32,
    pub definition: i32,
    pub amount: i32,
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("naval management field unreadable or address overflow")
}

fn variant(memory: &impl Memory, base: usize, pointer: usize) -> Result<u64, &'static str> {
    if pointer == 0 {
        return Ok(0);
    }
    if field::<u64>(memory, pointer, 0)?
        != engine_sites::at(base, engine_native::cequipment_variant::VTABLE_RVA)
            .ok_or("naval refit variant vtable withheld")? as u64
    {
        return Err("naval refit variant identity mismatch");
    }
    let handle = field(memory, pointer, engine_native::cequipment_variant::TYPE)?;
    if handle == 0 {
        return Err("naval refit variant has null handle");
    }
    Ok(handle)
}

pub fn task_settings(
    memory: &impl Memory,
    base: usize,
    state: usize,
    task: usize,
) -> Result<TaskSettings, &'static str> {
    let pointer = |offset| {
        field::<u64>(memory, task, offset)
            .and_then(|value| usize::try_from(value).map_err(|_| "naval repair pointer overflow"))
    };
    let target = pointer(engine_native::ctask_force::REPAIR_TARGET)?;
    let repair_target = if target == 0 {
        0
    } else {
        let provinces = state
            .checked_add(engine_native::cgame_state::PROVINCES)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("naval repair province array unreadable")?;
        engine_native::cfleet::province(memory, base, provinces, target)?
    };
    Ok(TaskSettings {
        repair_mode: field(memory, task, REPAIR_MODE)?,
        repair_split: field(memory, task, REPAIR_SPLIT)?,
        repair_target,
        merge_after_repair: field(memory, task, MERGE_AFTER_REPAIR)?,
        auto_reinforcement: field(memory, task, AUTO_REINFORCEMENT)?,
        refit_variant: variant(memory, base, pointer(REFIT_VARIANT)?)?,
        refit_after_repair: variant(memory, base, pointer(REFIT_AFTER_REPAIR)?)?,
        composition_token: field(memory, task, COMPOSITION_TOKEN)?,
    })
}

pub fn composition(
    memory: &impl Memory,
    base: usize,
    object: usize,
    entries: &mut Vec<CompositionEntry>,
) -> Result<(), &'static str> {
    if field::<u64>(memory, object, 0)?
        != engine_sites::at(base, ENTRIES_VTABLE_RVA).ok_or("naval composition vtable withheld")?
            as u64
    {
        return Err("naval composition identity mismatch");
    }
    let array = object
        .checked_add(COMPOSITION_ENTRIES)
        .and_then(|at| Array::read(memory, at, COMPOSITION_ENTRY_BYTES))
        .ok_or("naval composition array unreadable or invalid")?;
    entries.clear();
    engine_allocation::reserve(entries, array.count as usize)
        .map_err(|_| "naval composition allocation failed")?;
    for index in 0..array.count {
        let at = array
            .element(index, COMPOSITION_ENTRY_BYTES)
            .ok_or("naval composition entry address overflow")?;
        let bytes: [u8; COMPOSITION_ENTRY_BYTES] = field(memory, at, 0)?;
        let word = |offset| i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        entries.push(CompositionEntry {
            sub_unit: word(0),
            role: word(4),
            definition: word(8),
            amount: word(12),
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BaseSettings {
    pub priority: i32,
    pub dockyards: i32,
}

pub fn base_settings(
    memory: &impl Memory,
    base: usize,
    object: usize,
    queue: &mut Vec<u64>,
) -> Result<BaseSettings, &'static str> {
    if field::<u64>(memory, object, 0)?
        != engine_sites::at(base, crate::cnaval_base::VTABLE_RVA)
            .ok_or("naval repair base vtable withheld")? as u64
    {
        return Err("naval repair base identity mismatch");
    }
    base_settings_authenticated(memory, object, queue)
}

pub fn base_settings_authenticated(
    memory: &impl Memory,
    object: usize,
    queue: &mut Vec<u64>,
) -> Result<BaseSettings, &'static str> {
    let settings = BaseSettings {
        priority: field(memory, object, BASE_REPAIR_PRIORITY)?,
        dockyards: field(memory, object, BASE_REPAIR_DOCKYARDS)?,
    };
    let array = object
        .checked_add(BASE_REPAIR_QUEUE)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("naval repair queue unreadable or invalid")?;
    queue.clear();
    engine_allocation::reserve(queue, array.count as usize)
        .map_err(|_| "naval repair queue allocation failed")?;
    for index in 0..array.count {
        queue.push(field(
            memory,
            array
                .element(index, 8)
                .ok_or("naval repair queue address overflow")?,
            0,
        )?);
    }
    Ok(settings)
}

pub fn automated_homebase(memory: &impl Memory, fleet: usize) -> Result<u8, &'static str> {
    field(memory, fleet, AUTOMATED_HOMEBASE)
}

pub fn max_factories_for_repair(
    memory: &impl Memory,
    production: usize,
) -> Result<i32, &'static str> {
    field(memory, production, MAX_FACTORIES_FOR_REPAIR)
}

pub fn repair_dockyards(memory: &impl Memory, navy: usize) -> Result<(i32, i32), &'static str> {
    Ok((
        field(memory, navy, MAX_REPAIR_DOCKYARDS)?,
        field(memory, navy, USED_REPAIR_DOCKYARDS)?,
    ))
}

pub fn ship_refit_line(
    memory: &impl Memory,
    base: usize,
    ship: usize,
) -> Result<u64, &'static str> {
    let handle = field(memory, ship, SHIP_REFIT_LINE)?;
    let invalid = engine_sites::at(base, engine_native_parts::creference_registry::INVALID_REFERENCE_RVA)
        .and_then(|at| read::<u64>(memory, at))
        .ok_or("naval refit invalid reference unreadable")?;
    Ok(if handle == invalid { 0 } else { handle })
}

#[cfg(test)]
mod tests;

pub fn repair_children(
    memory: &impl Memory,
    object: usize,
    children: &mut Vec<u64>,
) -> Result<(), &'static str> {
    let array = Array::read(memory, object, 8)
        .ok_or("naval repair children array unreadable or invalid")?;
    children.clear();
    engine_allocation::reserve(children, array.count as usize)
        .map_err(|_| "naval repair children allocation failed")?;
    for index in 0..array.count {
        children.push(field(
            memory,
            array
                .element(index, 8)
                .ok_or("naval repair child address overflow")?,
            0,
        )?);
    }
    Ok(())
}
