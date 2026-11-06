//! Persistent espionage-operation object.
//!
//! Completion retains the selected effect's localized description and the bonus
//! and detection flags. These are exact client bytes and outcome flags, rather
//! than reconstructed effects or a universal success state. Serialization writes
//! true bonus/detection flags and writes the description only for terminal
//! operations outside checksum mode.
//!
//! The map icons' preparation display reads the selected phases, the
//! requirement records and the inline equipment pool the preparation checks
//! fill ([`Instance::parts`]).

use engine_sites::Rva;
use crate::{
    Memory, ccharacter::Literal, cequipment_type, cequipment_variant_pool, coperation,
    cpdx_array::Array, cprovince, csubject_relation::Name, read,
};

/// Primary vtable at subobject offset zero.
pub const VTABLE_RVA: Rva = Rva(0x02a3_8fc0);
/// Packed full reference, u64: low word type and high word instance ID.
pub const HANDLE: usize = 0x08;
/// Known operation-instance reference type. Admission uses the dynamic full
/// handle; this value is not an admission whitelist.
pub const REFERENCE_TYPE: u32 = 4_713;
/// COperationInstance signed i64 fixed-point preparation.
pub const PREPARATION_CURRENT: usize = 0x30;
/// COperationInstance signed i64 fixed-point preparation requirement.
#[cfg(any(test, feature = "test-support"))]
pub const PREPARATION_REQUIRED: usize = 0x38;
/// COperationInstance auto-repeat flag.
#[cfg(any(test, feature = "test-support"))]
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
/// Selected-phase array data containing the chosen definition pointers, with
/// its capacity and count after it as a CPdxArray header.
pub const PHASE_DATA: usize = 0xf8;
/// Signed phase-array count; definition pointers are serialized in this order.
pub const PHASE_COUNT: usize = 0x104;
/// Phase definition pointer width.
pub const PHASE_STRIDE: usize = 8;
/// A phase definition's u32 lexer token, saved as its `phases` spelling.
pub const PHASE_TOKEN: usize = 0x08;
/// Inline equipment pool (`cequipment_variant_pool`) the preparation checks
/// fill: its variant entries are saved as `equipment` and its per-archetype
/// totals are what the requirements are checked against.
pub const EQUIPMENT_POOL: usize = 0x110;
/// Requirement-record array pointer, with its capacity and signed count after
/// it as a CPdxArray header. Preparation checks read it with 0x20-byte records.
pub const REQUIREMENT_DATA: usize = 0x158;
/// Requirement-record stride.
pub const REQUIREMENT_STRIDE: usize = 0x20;
/// Requirement amount, signed i64 fixed5.
pub const REQUIREMENT_AMOUNT: usize = 0x00;
/// Requirement preparation days, i32.
pub const REQUIREMENT_DAYS: usize = 0x08;
/// Requirement equipment-type pointer; null for the civilian-factory
/// requirement. The type's archetype (`cequipment_type::ARCHETYPE`) names it.
pub const REQUIREMENT_EQUIPMENT: usize = 0x10;
/// Requirement byte: nonzero when the requirement uses preparation time.
pub const REQUIREMENT_USES_TIME: usize = 0x18;
/// Operation-instance serializer.
#[cfg(test)]
pub const SAVE_RVA: usize = 0x014a_0a70;
/// Operation-instance loader.
#[cfg(test)]
pub const LOADER_RVA: usize = 0x0149_e700;

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
    /// The operation's parts in the caller's part arena from `parts_start`:
    /// `phase_count` phases, then `requirement_count` requirements,
    /// `equipment_count` pool variant entries and `equipment_total_count`
    /// pool archetype totals, each in native order with repeats.
    pub parts_start: usize,
    pub requirement_count: u32,
    pub equipment_count: u32,
    pub equipment_total_count: u32,
    /// The equipment pool's raw allow-zero byte (`allow_zero_entries`).
    pub equipment_allow_zero: u8,
}

impl Row {
    /// The parts this row holds in the part arena.
    pub fn part_count(&self) -> usize {
        self.phase_count as usize
            + self.requirement_count as usize
            + self.equipment_count as usize
            + self.equipment_total_count as usize
    }
}

/// One requirement record. The civilian-factory requirement has no
/// equipment type: `equipment` and `archetype` are zero.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Requirement {
    pub amount: i64,
    pub days: i32,
    /// The equipment type's archetype lexer token.
    pub archetype: u32,
    /// 1 for an equipment requirement, 0 for the civilian-factory one.
    pub equipment: u8,
    /// The raw uses-preparation-time byte.
    pub uses_time: u8,
}

/// One equipment pool variant entry: its full variant reference, the lexer
/// token of its equipment type's archetype, and its amount.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Equipment {
    pub variant: u64,
    pub archetype: u32,
    pub amount: i64,
}

/// One equipment pool archetype total: the archetype's lexer token and the
/// stored total the requirements are checked against.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EquipmentTotal {
    pub archetype: u32,
    pub amount: i64,
}

/// One entry of an operation's parts ([`Row::parts_start`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    #[default]
    Empty,
    /// A selected phase definition's lexer token.
    Phase(u32),
    Requirement(Requirement),
    Equipment(Equipment),
    EquipmentTotal(EquipmentTotal),
}

pub struct Instance {
    pub row: Row,
    pub owner_pointer: usize,
    pub assigned: Array,
    pub effect_description: Literal,
    pub phases: Array,
    pub requirements: Array,
    /// The pool's variant entries and its archetype totals.
    pub equipment: Array,
    pub equipment_totals: Array,
}

impl Instance {
    /// Every part in arena order, each read and authenticated: phase tokens,
    /// requirement records (an equipment requirement's type and archetype
    /// authenticated), pool variant entries and pool archetype totals.
    pub fn parts(
        &self,
        memory: &impl Memory,
        base: usize,
        mut each: impl FnMut(Part),
    ) -> Result<(), &'static str> {
        for index in 0..self.phases.count {
            let definition = self
                .phases
                .element(index, PHASE_STRIDE)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|&pointer| pointer != 0)
                .ok_or("operation phase definition unreadable or null")?;
            let token = definition
                .checked_add(PHASE_TOKEN)
                .and_then(|address| read::<u32>(memory, address))
                .ok_or("operation phase token unreadable")?;
            each(Part::Phase(token));
        }
        for index in 0..self.requirements.count {
            let record = self
                .requirements
                .element(index, REQUIREMENT_STRIDE)
                .and_then(|address| read::<[u8; REQUIREMENT_STRIDE]>(memory, address))
                .ok_or("operation requirement unreadable")?;
            let equipment = usize::try_from(u64::from_le_bytes(
                record[REQUIREMENT_EQUIPMENT..REQUIREMENT_EQUIPMENT + 8]
                    .try_into()
                    .unwrap(),
            ))
            .map_err(|_| "operation requirement equipment pointer overflow")?;
            let archetype = if equipment == 0 {
                None
            } else {
                authenticate_type(memory, base, equipment)?;
                let archetype = equipment
                    .checked_add(cequipment_type::ARCHETYPE)
                    .and_then(|address| read::<u64>(memory, address))
                    .and_then(|pointer| usize::try_from(pointer).ok())
                    .ok_or("operation requirement archetype unreadable")?;
                Some(archetype_token(memory, base, archetype)?)
            };
            each(Part::Requirement(Requirement {
                amount: i64::from_le_bytes(
                    record[REQUIREMENT_AMOUNT..REQUIREMENT_AMOUNT + 8]
                        .try_into()
                        .unwrap(),
                ),
                days: i32::from_le_bytes(
                    record[REQUIREMENT_DAYS..REQUIREMENT_DAYS + 4]
                        .try_into()
                        .unwrap(),
                ),
                archetype: archetype.unwrap_or(0),
                equipment: u8::from(archetype.is_some()),
                uses_time: record[REQUIREMENT_USES_TIME],
            }));
        }
        for ordinal in 0..self.equipment.count {
            let address = self
                .equipment
                .element(ordinal, crate::equipment_entry::STRIDE)
                .ok_or("operation equipment entry address overflow")?;
            let entry = crate::equipment_entry::read(memory, base, address, ordinal)?;
            let equipment = entry
                .variant_pointer
                .checked_add(crate::cequipment_variant::DEFINITION)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|&pointer| pointer != 0)
                .ok_or("operation equipment variant definition unreadable or null")?;
            authenticate_type(memory, base, equipment)?;
            let archetype = equipment
                .checked_add(cequipment_type::ARCHETYPE)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or("operation equipment archetype unreadable")?;
            each(Part::Equipment(Equipment {
                variant: entry.variant_handle,
                archetype: archetype_token(memory, base, archetype)?,
                amount: entry.amount_raw,
            }));
        }
        for ordinal in 0..self.equipment_totals.count {
            let address = self
                .equipment_totals
                .element(ordinal, crate::allocation_equipment_row::STRIDE)
                .ok_or("operation equipment total address overflow")?;
            let total = crate::allocation_equipment_row::read(memory, base, address, ordinal)?;
            each(Part::EquipmentTotal(EquipmentTotal {
                archetype: archetype_token(memory, base, total.archetype_pointer)?,
                amount: total.amount_raw,
            }));
        }
        Ok(())
    }
}

/// An equipment type, archetype or null placeholder: each shares the
/// complete type layout.
fn authenticate_type(
    memory: &impl Memory,
    base: usize,
    equipment: usize,
) -> Result<(), &'static str> {
    let vtable = |key: Rva| {
        engine_sites::at(base, key)
            .map(|address| address as u64)
            .ok_or("operation equipment type vtable overflow")
    };
    let actual = read::<u64>(memory, equipment);
    if actual != Some(vtable(cequipment_type::VTABLE_RVA)?)
        && actual != Some(vtable(cequipment_type::NULL_VTABLE_RVA)?)
    {
        return Err("operation equipment type vtable mismatch");
    }
    Ok(())
}

/// An authenticated archetype's lexer token.
fn archetype_token(
    memory: &impl Memory,
    base: usize,
    archetype: usize,
) -> Result<u32, &'static str> {
    if archetype == 0 {
        return Err("operation equipment archetype null");
    }
    authenticate_type(memory, base, archetype)?;
    archetype
        .checked_add(cequipment_type::TOKEN)
        .and_then(|address| read::<u32>(memory, address))
        .ok_or("operation equipment archetype token unreadable")
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
    let expected = engine_sites::at(base, VTABLE_RVA)
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
        let expected = engine_sites::at(base, cprovince::VTABLE_RVA)
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
    let phases = Array::read(memory, at(PHASE_DATA)?, PHASE_STRIDE)
        .ok_or("operation phases unreadable or invalid")?;
    if phases.count != phase_count as u32 {
        return Err("operation phase count changed during read");
    }
    let requirements = Array::read(memory, at(REQUIREMENT_DATA)?, REQUIREMENT_STRIDE)
        .ok_or("operation requirements unreadable or invalid")?;
    let pool = at(EQUIPMENT_POOL)?;
    let equipment = cequipment_variant_pool::entries(memory, base, pool)?;
    let equipment_totals = cequipment_variant_pool::allocation_entries(memory, base, pool)?;
    let equipment_allow_zero = pool
        .checked_add(cequipment_variant_pool::ALLOW_ZERO)
        .and_then(|address| read::<u8>(memory, address))
        .ok_or("operation equipment allow-zero byte unreadable")?;
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
            requirement_count: requirements.count,
            equipment_count: equipment.count,
            equipment_total_count: equipment_totals.count,
            equipment_allow_zero,
            ..Row::default()
        },
        owner_pointer,
        assigned,
        effect_description,
        phases,
        requirements,
        equipment,
        equipment_totals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializer_rvas_match_supported_operation_layout() {
        assert_eq!(SAVE_RVA, 0x14A0A70);
        assert_eq!(LOADER_RVA, 0x149e700);
    }
}
