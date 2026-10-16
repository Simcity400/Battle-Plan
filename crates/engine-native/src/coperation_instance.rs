//! Persistent espionage-operation object.
//!
//! Completion retains the selected effect's localized description and the bonus
//! and detection flags. These are exact client bytes and outcome flags, rather
//! than reconstructed effects or a universal success state. Serialization writes
//! true bonus/detection flags and writes the description only for terminal
//! operations outside checksum mode.

use crate::{
    Memory, ccharacter::Literal, coperation, cpdx_array::Array, cprovince, csubject_relation::Name,
    read,
};

/// Primary vtable at subobject offset zero.
pub const VTABLE_RVA: usize = 0x029a_76c0;
/// Packed full reference, u64: low word type and high word instance ID.
pub const HANDLE: usize = 0x08;
/// Known operation-instance reference type. Admission uses the dynamic full
/// handle; this value is not an admission whitelist.
pub const REFERENCE_TYPE: u32 = 4_713;
/// COperationInstance signed i64 fixed-point preparation.
pub const PREPARATION_CURRENT: usize = 0x30;
/// COperationInstance signed i64 fixed-point preparation requirement.
pub const PREPARATION_REQUIRED: usize = 0x38;
/// COperationInstance boolean flag.
pub const FLAG_40: usize = 0x40;
/// COperationInstance auto-repeat flag.
pub const AUTO_REPEAT: usize = 0x41;
/// Completion sets this byte after applying the selected effect.
pub const TERMINAL: usize = 0x42;
/// Operation-definition pointer; authenticate the pointee's vtable.
pub const DEFINITION: usize = 0x48;
/// Owning country pointer, joined through its raw country reference.
pub const OWNER_COUNTRY: usize = 0x50;
/// Target-country raw reference, u32, joined to the country table.
pub const TARGET_COUNTRY_ID: usize = 0x58;
/// Optional target province pointer; authenticate the selected province
/// before following its state pointer.
pub const TARGET_PROVINCE: usize = 0x60;
/// Signed i32 start hour, copied from the game date on start and saved only
/// when duration is nonzero.
pub const START_HOUR: usize = 0x70;
/// COperationInstance signed i32 duration days.
pub const DURATION_DAYS: usize = 0x80;
/// Completion moves the localized effect text here and trims trailing whitespace.
/// The tooltip reads this retained string.
pub const EFFECT_DESCRIPTION: usize = 0xa0;
/// String construction and assignment refuse lengths above the game's maximum;
/// no smaller text cap is imposed.
pub const DESCRIPTION_MAX_LENGTH: u64 = 0x7fff_ffff_ffff_ffff;
/// Bonus trial result; a nonzero value selects the bonus effect definition.
/// Serialized under its bonus token.
pub const BONUS: usize = 0xc0;
/// Detection result set after the risk trial and action dispatch, read by
/// the completion icon and serialized under its detection token.
pub const DETECTED: usize = 0xc1;
/// Assigned-operative record-array data, with 0x38-byte records.
pub const ASSIGNED_OPERATIVE_DATA: usize = 0xe0;
/// Signed assigned-slot capacity for storage of 0x38-byte records.
pub const ASSIGNED_OPERATIVE_CAPACITY: usize = 0xe8;
/// Signed assigned-slot count, including slots with unassigned handles.
pub const ASSIGNED_OPERATIVE_COUNT: usize = 0xec;
/// Selected-phase array data containing the chosen definition pointers.
pub const PHASE_DATA: usize = 0xf8;
/// Signed phase-array count; definition pointers are serialized in this order.
pub const PHASE_COUNT: usize = 0x104;
/// Inline operation resource map rooted at +0x110, used by preparation
/// checks.
pub const RESOURCE_MAP: usize = 0x110;
/// Requirement-record array pointer. Preparation checks read +0x158
/// with 0x20-byte records.
pub const REQUIREMENT_DATA: usize = 0x158;
/// Requirement-record count, u32. Preparation checks read +0x164
/// before walking optional and required resources.
pub const REQUIREMENT_COUNT: usize = 0x164;
/// Operation-instance serializer.
pub const SAVE_RVA: usize = 0x013e_f710;
/// Operation-instance loader.
pub const LOADER_RVA: usize = 0x013e_d380;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub owner: u32,
    pub target: u32,
    pub target_province: Option<u32>,
    pub definition_token: u32,
    pub preparation_current: i64,
    pub preparation_required: i64,
    pub flag_40: u8,
    pub auto_repeat: u8,
    pub terminal: u8,
    pub bonus: u8,
    pub detected: u8,
    /// Exact retained client-localized bytes in the caller-owned arena.
    pub effect_description: Name,
    pub start_hour: i32,
    pub duration_days: i32,
    pub phase_count: u32,
    pub assigned_slots: u32,
}

pub struct Instance {
    pub row: Row,
    pub owner_pointer: usize,
    pub assigned: Array,
    pub effect_description: Literal,
}

pub fn instance(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Instance, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("operation address overflow")
    };
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("operation module address overflow")?;
    let identity = read::<[u8; 16]>(memory, pointer).ok_or("operation identity unreadable")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64 {
        return Err("operation instance vtable mismatch");
    }
    let id = u64::from_le_bytes(identity[8..].try_into().unwrap());
    if id == 0 {
        return Err("operation instance has null handle");
    }
    let mut preparation = [0; 19];
    let mut definition_pointer = [0; 8];
    let mut owner_pointer = [0; 8];
    let mut target = [0; 4];
    let mut province_pointer = [0; 8];
    let mut start_hour = [0; 4];
    let mut duration_days = [0; 4];
    let mut phase_count = [0; 4];
    let mut outcome = [0; 34];
    if memory.copy_scattered(
        [
            at(PREPARATION_CURRENT)?,
            at(DEFINITION)?,
            at(OWNER_COUNTRY)?,
            at(TARGET_COUNTRY_ID)?,
            at(TARGET_PROVINCE)?,
            at(START_HOUR)?,
            at(DURATION_DAYS)?,
            at(PHASE_COUNT)?,
            at(EFFECT_DESCRIPTION)?,
        ],
        &mut [
            &mut preparation,
            &mut definition_pointer,
            &mut owner_pointer,
            &mut target,
            &mut province_pointer,
            &mut start_hour,
            &mut duration_days,
            &mut phase_count,
            &mut outcome,
        ],
    ) != 9
    {
        return Err("operation instance fields unreadable");
    }
    if preparation[16..]
        .iter()
        .chain(&outcome[32..])
        .any(|&flag| flag > 1)
    {
        return Err("operation boolean flag invalid");
    }
    let description_header: [u8; 32] = outcome[..32].try_into().unwrap();
    if u64::from_le_bytes(description_header[16..24].try_into().unwrap()) > DESCRIPTION_MAX_LENGTH
        || u64::from_le_bytes(description_header[24..32].try_into().unwrap())
            > DESCRIPTION_MAX_LENGTH
    {
        return Err("operation effect description exceeds native string maximum");
    }
    let effect_description = Literal::from_header(at(EFFECT_DESCRIPTION)?, description_header)
        .map_err(|_| "operation effect description header invalid")?;
    Literal::validate(memory, [&effect_description])
        .map_err(|_| "operation effect description terminator unreadable or invalid")?;
    let phase_count = i32::from_le_bytes(phase_count);
    if phase_count < 0 {
        return Err("operation phase count negative");
    }
    let owner_pointer = usize::try_from(u64::from_le_bytes(owner_pointer))
        .map_err(|_| "operation owner pointer overflow")?;
    let definition_pointer = usize::try_from(u64::from_le_bytes(definition_pointer))
        .map_err(|_| "operation definition pointer overflow")?;
    let definition = coperation::definition(memory, base, definition_pointer)?;
    let province_pointer = usize::try_from(u64::from_le_bytes(province_pointer))
        .map_err(|_| "operation target province pointer overflow")?;
    let target_province = if province_pointer == 0 {
        None
    } else {
        let expected = base
            .checked_add(cprovince::VTABLE_RVA)
            .ok_or("province module address overflow")?;
        if read::<u64>(memory, province_pointer) != Some(expected as u64) {
            return Err("operation target province vtable mismatch");
        }
        Some(
            province_pointer
                .checked_add(cprovince::ID)
                .and_then(|p| read::<u32>(memory, p))
                .ok_or("operation target province id unreadable")?,
        )
    };
    let assigned = Array::read(
        memory,
        at(ASSIGNED_OPERATIVE_DATA)?,
        crate::operation_assigned_operative::STRIDE,
    )
    .ok_or("operation assigned slots unreadable or invalid")?;
    Ok(Instance {
        row: Row {
            id,
            target: u32::from_le_bytes(target),
            target_province,
            definition_token: definition,
            preparation_current: i64::from_le_bytes(preparation[..8].try_into().unwrap()),
            preparation_required: i64::from_le_bytes(preparation[8..16].try_into().unwrap()),
            flag_40: preparation[16],
            auto_repeat: preparation[17],
            terminal: preparation[18],
            bonus: outcome[32],
            detected: outcome[33],
            start_hour: i32::from_le_bytes(start_hour),
            duration_days: i32::from_le_bytes(duration_days),
            phase_count: phase_count as u32,
            assigned_slots: assigned.count,
            ..Row::default()
        },
        owner_pointer,
        assigned,
        effect_description,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializer_rvas_match_supported_operation_layout() {
        assert_eq!(SAVE_RVA, 0x13EF710);
        assert_eq!(LOADER_RVA, 0x13ED380);
    }
}
