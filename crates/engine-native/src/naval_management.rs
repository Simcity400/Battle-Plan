//! Stored naval repair, refit and reinforcement controls. Callers supply objects already
//! authenticated by their existing census; these readers never enumerate the world or call a
//! game getter. Ordered variable data grows only when that object's writer invalidates its hold.
use crate::{Memory, cpdx_array::Array, read};

/// Existing core naval census observations; no observer owns these process pointers beyond the
/// capture unless its writer/lifetime domain certifies their continued identity.
#[derive(Clone, Copy, Debug)]
pub enum CensusVisit {
    Country(crate::ccountry::Country),
    Fleet { pointer: usize, handle: u64 },
    TaskForce { pointer: usize, handle: u64 },
}

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
/// The native empty AI-composition selection, omitted by the serializer.
pub const NO_AI_COMPOSITION_TOKEN: u32 = 0x4c17;
pub const ENTRIES_VTABLE_RVA: usize = 0x029f_b6a8;
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
    /// Raw rejoin reference, independent of whether its destination is still live.
    pub merge_after_repair: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompositionEntry {
    pub sub_unit: i32,
    pub role: i32,
    /// Runtime definition preference, independent of the serialized sub-unit and role.
    pub definition: i32,
    pub amount: i32,
}

fn field<T: crate::memory::Raw>(
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
        != base
            .checked_add(crate::cequipment_variant::VTABLE_RVA)
            .ok_or("naval refit variant vtable overflow")? as u64
    {
        return Err("naval refit variant identity mismatch");
    }
    let handle = field(memory, pointer, crate::cequipment_variant::TYPE)?;
    if handle == 0 {
        return Err("naval refit variant has null handle");
    }
    Ok(handle)
}

/// Reads settings after the selecting fleet census authenticated `task`. A non-null target must
/// authenticate as the province at its stored id. Null refit pointers retain known absence.
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
    let target = pointer(crate::ctask_force::REPAIR_TARGET)?;
    let repair_target = if target == 0 {
        0
    } else {
        let provinces = state
            .checked_add(crate::cgame_state::PROVINCES)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("naval repair province array unreadable")?;
        crate::cfleet::province(memory, base, provinces, target)?
    };
    Ok(TaskSettings {
        repair_mode: field(memory, task, REPAIR_MODE)?,
        repair_split: field(memory, task, REPAIR_SPLIT)?,
        repair_target,
        // Destination lifetime need not write this source. Consumers resolve the held
        // reference against the hour's task-force census, including after fleet transfer.
        merge_after_repair: field(memory, task, MERGE_AFTER_REPAIR)?,
        auto_reinforcement: field(memory, task, AUTO_REINFORCEMENT)?,
        refit_variant: variant(memory, base, pointer(REFIT_VARIANT)?)?,
        refit_after_repair: variant(memory, base, pointer(REFIT_AFTER_REPAIR)?)?,
        composition_token: field(memory, task, COMPOSITION_TOKEN)?,
    })
}

/// Copies one native requirements or fulfillment list. Entries, including zero or negative
/// amounts and repeated preferences, remain in native order. Capacity follows its live count.
pub fn composition(
    memory: &impl Memory,
    base: usize,
    object: usize,
    entries: &mut Vec<CompositionEntry>,
) -> Result<(), &'static str> {
    if field::<u64>(memory, object, 0)?
        != base
            .checked_add(ENTRIES_VTABLE_RVA)
            .ok_or("naval composition vtable overflow")? as u64
    {
        return Err("naval composition identity mismatch");
    }
    let array = object
        .checked_add(COMPOSITION_ENTRIES)
        .and_then(|at| Array::read(memory, at, COMPOSITION_ENTRY_BYTES))
        .ok_or("naval composition array unreadable or invalid")?;
    entries.clear();
    crate::allocation::reserve(entries, array.count as usize)
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
        != base
            .checked_add(crate::cnaval_base::VTABLE_RVA)
            .ok_or("naval repair base vtable overflow")? as u64
    {
        return Err("naval repair base identity mismatch");
    }
    base_settings_authenticated(memory, object, queue)
}

/// Reads one base already authenticated by the selecting base census.
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
    crate::allocation::reserve(queue, array.count as usize)
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

/// The ship's stored refit work-order reference; absence follows the native invalid-reference
/// sentinel, independently of whether an old reference still resolves.
pub fn ship_refit_line(
    memory: &impl Memory,
    base: usize,
    ship: usize,
) -> Result<u64, &'static str> {
    let handle = field(memory, ship, SHIP_REFIT_LINE)?;
    let invalid = base
        .checked_add(crate::creference_registry::INVALID_REFERENCE_RVA)
        .and_then(|at| read::<u64>(memory, at))
        .ok_or("naval refit invalid reference unreadable")?;
    Ok(if handle == invalid { 0 } else { handle })
}

#[cfg(test)]
mod tests;

/// The native ordered repair-detachment references, including unresolved entries.
pub fn repair_children(
    memory: &impl Memory,
    object: usize,
    children: &mut Vec<u64>,
) -> Result<(), &'static str> {
    let array = Array::read(memory, object, 8)
        .ok_or("naval repair children array unreadable or invalid")?;
    children.clear();
    crate::allocation::reserve(children, array.count as usize)
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
