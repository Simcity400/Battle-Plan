
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native_parts::{carmy_manpower_values, equipment_entry};
use engine_native::cequipment_variant::pool as cequipment_variant_pool;
use crate::manpower_value_entry;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x29da928);
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x29da960);
pub const SECONDARY_VTABLE: usize = 0x18;
pub const DISTRIBUTABLE_PRIORITY: usize = 0x08;
pub const EQUIPMENT_POOL: usize = 0x20;
pub const TRAINING: usize = 0x60;
pub const MAX_TRAINING: usize = 0x68;
pub const MANPOWER_HAVE: usize = 0x70;
pub const MANPOWER_NEED: usize = 0x90;
pub const MAX_MANPOWER: usize = 0xb0;
pub const OWNING_LINE: usize = 0xb8;
pub const HALTING_REASON: usize = 0xc0;
pub const HALTING_REASON_PRESENT: usize = 0xc4;
pub const FIXED_POINT_SCALE: i64 = 100_000;
pub const CONSTRUCT_RVA: Rva = Rva(0x00d4_6d40);

pub const SIDE_HAVE: u8 = 0;
pub const SIDE_NEED: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fields {
    pub training_raw: i64,
    pub max_training_raw: i64,
    pub max_manpower: i32,
    pub allow_zero_entries: u8,
    pub equipment_allocations: i32,
    pub distributable_priority: i32,
    pub halting_reason_present: u8,
    pub halting_reason: i32,
    pub equipment: Array,
    pub manpower: [Array; 2],
}

pub(crate) fn read_fields(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    line: usize,
) -> Result<Fields, &'static str> {
    let module = |key: Rva| {
        engine_sites::at(base, key)
            .map(|address| address as u64)
            .ok_or("military deployment module address overflow")
    };
    let at = |offset: usize| {
        pointer
            .checked_add(offset)
            .ok_or("military deployment address overflow")
    };
    if read::<u64>(memory, pointer) != Some(module(VTABLE_RVA)?) {
        return Err("military deployment vtable mismatch");
    }
    if read::<u64>(memory, at(SECONDARY_VTABLE)?) != Some(module(SECONDARY_VTABLE_RVA)?) {
        return Err("military deployment secondary vtable mismatch");
    }
    let pool = at(EQUIPMENT_POOL)?;
    let equipment = cequipment_variant_pool::entries(memory, base, pool)?;
    let equipment_allocations = cequipment_variant_pool::allocation_entries(memory, base, pool)?
        .count
        .try_into()
        .map_err(|_| "military deployment allocation count exceeds i32")?;
    let allow_zero_entries = pool
        .checked_add(cequipment_variant_pool::ALLOW_ZERO)
        .and_then(|address| read::<u8>(memory, address))
        .ok_or("military deployment allow-zero byte unreadable")?;
    if read::<u64>(memory, at(OWNING_LINE)?) != Some(line as u64) {
        return Err("military deployment owning-line back pointer mismatch");
    }
    Ok(Fields {
        training_raw: read::<i64>(memory, at(TRAINING)?)
            .ok_or("military deployment training unreadable")?,
        max_training_raw: read::<i64>(memory, at(MAX_TRAINING)?)
            .ok_or("military deployment max training unreadable")?,
        max_manpower: read::<i32>(memory, at(MAX_MANPOWER)?)
            .ok_or("military deployment max manpower unreadable")?,
        allow_zero_entries,
        equipment_allocations,
        distributable_priority: read::<i32>(memory, at(DISTRIBUTABLE_PRIORITY)?)
            .ok_or("military deployment distributable priority unreadable")?,
        halting_reason_present: read::<u8>(memory, at(HALTING_REASON_PRESENT)?)
            .ok_or("military deployment halting-reason flag unreadable")?,
        halting_reason: read::<i32>(memory, at(HALTING_REASON)?)
            .ok_or("military deployment halting reason unreadable")?,
        equipment,
        manpower: [
            manpower_array(memory, base, at(MANPOWER_HAVE)?)?,
            manpower_array(memory, base, at(MANPOWER_NEED)?)?,
        ],
    })
}

fn manpower_array(
    memory: &impl Memory,
    base: usize,
    container: usize,
) -> Result<Array, &'static str> {
    let expected = engine_sites::at(base, carmy_manpower_values::VTABLE_RVA)
        .ok_or("military deployment module address overflow")?;
    if read::<u64>(memory, container) != Some(expected as u64) {
        return Err("military deployment manpower container vtable mismatch");
    }
    container
        .checked_add(carmy_manpower_values::DATA)
        .and_then(|address| Array::read(memory, address, manpower_value_entry::STRIDE))
        .ok_or("military deployment manpower array invalid")
}

pub(crate) fn equipment_entry(
    memory: &impl Memory,
    base: usize,
    array: Array,
    ordinal: u32,
) -> Result<equipment_entry::Row, &'static str> {
    let address = array
        .element(ordinal, equipment_entry::STRIDE)
        .ok_or("military deployment equipment address overflow")?;
    equipment_entry::read(memory, base, address, ordinal)
}

pub(crate) fn manpower_entry(
    memory: &impl Memory,
    array: Array,
    ordinal: u32,
) -> Result<(u32, i32), &'static str> {
    let value = array
        .element(ordinal, manpower_value_entry::STRIDE)
        .and_then(|address| read::<[u8; 8]>(memory, address))
        .ok_or("military deployment manpower entry unreadable")?;
    Ok((
        u32::from_le_bytes(value[..4].try_into().unwrap()),
        i32::from_le_bytes(value[manpower_value_entry::COUNT..].try_into().unwrap()),
    ))
}
