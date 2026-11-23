
use engine_sites::Rva;
use crate::{
    Memory,
    ccharacter::Literal,
    cequipment_type,
    cequipment_variant::pool as cequipment_variant_pool,
    coperation,
    cpdx_array::Array,
    cprovince,
    crelation::subject::Name,
    read,
};

pub const VTABLE_RVA: Rva = Rva(0x02a3_8fc0);
pub const HANDLE: usize = 0x08;
pub const REFERENCE_TYPE: u32 = 4_713;
pub const PREPARATION_CURRENT: usize = 0x30;
#[cfg(any(test, feature = "test-support"))]
pub const PREPARATION_REQUIRED: usize = 0x38;
#[cfg(any(test, feature = "test-support"))]
pub const AUTO_REPEAT: usize = 0x41;
pub const TERMINAL: usize = 0x42;
pub const DEFINITION: usize = 0x48;
pub const OWNER_COUNTRY: usize = 0x50;
pub const TARGET_COUNTRY_ID: usize = 0x58;
pub const TARGET_PROVINCE: usize = 0x60;
pub const START_HOUR: usize = 0x70;
pub const DURATION_DAYS: usize = 0x80;
pub const EFFECT_DESCRIPTION: usize = 0xa0;
pub const DESCRIPTION_MAX_LENGTH: u64 = 0x7fff_ffff_ffff_ffff;
pub const BONUS: usize = 0xc0;
pub const DETECTED: usize = 0xc1;
pub const ASSIGNED_OPERATIVE_DATA: usize = 0xe0;
pub const PHASE_DATA: usize = 0xf8;
pub const PHASE_COUNT: usize = 0x104;
pub const PHASE_STRIDE: usize = 8;
pub const PHASE_TOKEN: usize = 0x08;
pub const EQUIPMENT_POOL: usize = 0x110;
pub const REQUIREMENT_DATA: usize = 0x158;
pub const REQUIREMENT_STRIDE: usize = 0x20;
pub const REQUIREMENT_AMOUNT: usize = 0x00;
pub const REQUIREMENT_DAYS: usize = 0x08;
pub const REQUIREMENT_EQUIPMENT: usize = 0x10;
pub const REQUIREMENT_USES_TIME: usize = 0x18;
#[cfg(test)]
pub const SAVE_RVA: usize = 0x014a_0a70;
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
    pub effect_description: Name,
    pub start_hour: i32,
    pub duration_days: i32,
    pub phase_count: u32,
    pub assigned_slots: u32,
    pub parts_start: usize,
    pub requirement_count: u32,
    pub equipment_count: u32,
    pub equipment_total_count: u32,
    pub equipment_allow_zero: u8,
}

impl Row {
    pub fn part_count(&self) -> usize {
        self.phase_count as usize
            + self.requirement_count as usize
            + self.equipment_count as usize
            + self.equipment_total_count as usize
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Requirement {
    pub amount: i64,
    pub days: i32,
    pub archetype: u32,
    pub equipment: u8,
    pub uses_time: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Equipment {
    pub variant: u64,
    pub archetype: u32,
    pub amount: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EquipmentTotal {
    pub archetype: u32,
    pub amount: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    #[default]
    Empty,
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
    pub equipment: Array,
    pub equipment_totals: Array,
}

impl Instance {
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
