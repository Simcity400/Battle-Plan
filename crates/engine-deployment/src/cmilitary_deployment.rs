//! Military deployment layout uses the primary object base.
//!
//! It owns a secondary subobject, an equipment pool, two training qwords, the had and needed
//! manpower containers, a maximum-manpower word and an owning-line backlink. The queued template
//! belongs to the conveyor rather than this backlink.
//!
//! Serialization runs on the secondary base. Training, maximum training and
//! maximum manpower emit only when positive; both manpower wrappers always
//! emit. Equipment emits for a nonzero allocation, halting reason for its
//! set flag, and distributable priority when different from one.
//!
//! Training remains raw signed i64 fixed5 with denominator 100,000. Zero is
//! a stored value even when the save omits it.

use crate::{
    Memory, carmy_manpower_values, cequipment_variant_pool, cpdx_array::Array, equipment_entry,
    manpower_value_entry, read,
};

/// CMilitaryDeployment primary vtable.
pub const VTABLE_RVA: usize = 0x29da928;
/// CMilitaryDeployment secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x29da960;
/// CMilitaryDeployment secondary vptr offset.
pub const SECONDARY_VTABLE: usize = 0x18;
/// Distributable priority, signed i32, initially one; save omits one.
pub const DISTRIBUTABLE_PRIORITY: usize = 0x08;
/// CMilitaryDeployment embedded CEquipmentVariantPool.
pub const EQUIPMENT_POOL: usize = 0x20;
/// CMilitaryDeployment training, i64 fixed5.
pub const TRAINING: usize = 0x60;
/// CMilitaryDeployment maximum training, i64 fixed5.
pub const MAX_TRAINING: usize = 0x68;
/// CMilitaryDeployment available CArmyManpowerValues.
pub const MANPOWER_HAVE: usize = 0x70;
/// CMilitaryDeployment needed CArmyManpowerValues.
pub const MANPOWER_NEED: usize = 0x90;
/// CMilitaryDeployment maximum manpower, i32.
pub const MAX_MANPOWER: usize = 0xb0;
/// Owning deployment-line backlink, authenticated by equality with the
/// line pointer. The queued template belongs to the conveyor.
pub const OWNING_LINE: usize = 0xb8;
/// CMilitaryDeployment optional halting-reason value, i32.
pub const HALTING_REASON: usize = 0xc0;
/// CMilitaryDeployment halting-reason `_bIsSet` byte.
pub const HALTING_REASON_PRESENT: usize = 0xc4;
/// CMilitaryDeployment native training denominator.
pub const FIXED_POINT_SCALE: i64 = 100_000;
/// Deployment constructor.
pub const CONSTRUCT_RVA: usize = 0x00d4_6d40;

/// The manpower container's stored side. `Have` is the first container, `Need` the second.
pub const SIDE_HAVE: u8 = 0;
/// Needed manpower container; the save writes it as `army_manpower_need`.
pub const SIDE_NEED: u8 = 1;

/// The deployment object's own scalars. The queued template belongs to the
/// owning conveyor; this object only points back at its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fields {
    pub training_raw: i64,
    pub max_training_raw: i64,
    pub max_manpower: i32,
    /// Raw pool byte; the serializer emits zero-amount entries only when set.
    pub allow_zero_entries: u8,
    /// Native i32 count of the pool's allocation array, which gates whether the
    /// engine writes the equipment wrapper at all.
    pub equipment_allocations: i32,
    /// CEquipmentDistributable priority; one is the constructor's default.
    pub distributable_priority: i32,
    /// Raw `_bIsSet` byte of the optional halting reason.
    pub halting_reason_present: u8,
    /// Raw halting-reason word; meaningful only when the flag byte is set.
    pub halting_reason: i32,
    pub equipment: Array,
    pub manpower: [Array; 2],
}

/// Authenticates both subobject vtables, the owning-line back pointer, the
/// embedded pool and both manpower containers, then returns the object's
/// scalars and its three array headers.
pub(crate) fn read_fields(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    line: usize,
) -> Result<Fields, &'static str> {
    let module = |rva: usize| {
        base.checked_add(rva)
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
    let expected = base
        .checked_add(carmy_manpower_values::VTABLE_RVA)
        .ok_or("military deployment module address overflow")?;
    if read::<u64>(memory, container) != Some(expected as u64) {
        return Err("military deployment manpower container vtable mismatch");
    }
    container
        .checked_add(carmy_manpower_values::DATA)
        .and_then(|address| Array::read(memory, address, manpower_value_entry::STRIDE))
        .ok_or("military deployment manpower array invalid")
}

/// One in-transit equipment entry of the deployment's own pool.
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

/// One manpower entry: the raw source-country link and its signed count.
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
