use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::nproject_cprogram as program;
use engine_characters::{cscientist, scientist_skill_entry};
use engine_native_parts::{
    cbuilding,
    cbuilding_template,
    creference_registry,
    modifier_accumulator,
};
use engine_native::{
    cprovince::{buildings as cbuilding_status, self},
    ccharacter,
    cgame_state,
    cstate,
};
use engine_sites::Rva;

pub const BYTES: usize = 36;
pub const BUILDING: u32 = 2;
pub const SUPPLY: u32 = 3;
pub const RESOURCES: u32 = 4;
pub const SKILL: u32 = 5;
pub const COUNTRY: u32 = 6;
pub const STATE: u32 = 7;
pub const TRAIT: u32 = 8;
pub const INJURY: u32 = 9;
pub const BASIC: u32 = 10;
pub const BREAKTHROUGH: u32 = 11;
pub const TECHNOLOGY: u32 = 12;
pub const EXPERIENCE: u32 = 13;

const CONFIGURATION: usize = 0x48;
const SPECIALIZATIONS: usize = 0x298;
const SUPPLY_RATIO: usize = 0xf8;
const BASIC_FACTOR: usize = 0x180;
const PROJECT_DEFINITION: usize = 0x20;
const PROJECT_RESOURCE_RATIO: usize = 0x130;
const PROJECT_SPEED_KEYS: usize = 0x3c8;
const SCIENTIST_MODIFIERS: usize = 0x58;
const TRAIT_MODIFIERS: usize = 0x58;
const BREAKTHROUGH_MODIFIER: u32 = 0x27b;
const TECHNOLOGY_MODIFIER: u32 = 0x27a;
const MINIMUM_SUPPLY_RVA: Rva = Rva(0x33e_46e8);
const MINIMUM_RESOURCES_RVA: Rva = Rva(0x33e_4c18);
const SUPPORT_FRACTION_RVA: Rva = Rva(0x33d_f328);
const INJURY_FACTOR_RVA: Rva = Rva(0x33e_55b8);
const BREAKTHROUGH_SKILL_GAIN_RVA: Rva = Rva(0x33e_525c);
const TECHNOLOGY_BONUS_RVA: Rva = Rva(0x33e_5328);
const BASIC_EXPERIENCE_RVA: Rva = Rva(0x33e_5cd0);
const SKILL_SPEED_ARRAY_RVA: Rva = Rva(0x33e_73d0);
const ONE: i64 = 100_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Operand {
    pub kind: u32,
    pub scientist: u64,
    pub token: u32,
    pub key: u32,
    pub ordinal: u32,
    pub assignment: u32,
    pub raw: i64,
}
impl Operand {
    pub fn encode(self, bytes: &mut [u8]) {
        bytes[..4].copy_from_slice(&self.kind.to_le_bytes());
        bytes[4..12].copy_from_slice(&self.scientist.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.token.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.key.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.ordinal.to_le_bytes());
        bytes[24..28].copy_from_slice(&self.assignment.to_le_bytes());
        bytes[28..36].copy_from_slice(&self.raw.to_le_bytes());
    }
    pub fn decode(bytes: &[u8]) -> Self {
        Self {
            kind: u32::from_le_bytes(bytes[..4].try_into().unwrap()),
            scientist: u64::from_le_bytes(bytes[4..12].try_into().unwrap()),
            token: u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
            key: u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            ordinal: u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
            assignment: u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            raw: i64::from_le_bytes(bytes[28..36].try_into().unwrap()),
        }
    }
}
fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("facility detail address overflow")
}
fn word<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("facility detail operand unreadable")
}
fn pointer(memory: &impl Memory, address: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(word::<u64>(memory, address, offset)?)
        .ok()
        .filter(|p| *p != 0)
        .ok_or("facility detail pointer is null or invalid")
}
fn global<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    read(memory, site(base, key)?).ok_or("facility detail operand unreadable")
}
fn site(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("facility detail address overflow")
}
fn identity(
    memory: &impl Memory,
    base: usize,
    object: usize,
    expected: Rva,
) -> Result<(), &'static str> {
    if word::<u64>(memory, object, 0)? != site(base, expected)? as u64 {
        return Err("facility detail object identity mismatch");
    }
    Ok(())
}

fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

pub fn health(level: i16, healthy: i16, partial: i64) -> i64 {
    if level == 0 {
        return 0;
    }
    if level == healthy {
        return ONE;
    }
    let valid = |n: i64| (n.wrapping_add(i64::MAX - 1) as u64) < u64::MAX - 2;
    let fraction = if valid(partial) {
        partial.wrapping_mul(ONE) / 10_000_000
    } else {
        0xffff_ffff
    };
    let total = fraction.wrapping_add(i64::from(healthy) * ONE);
    if valid(total) {
        total.wrapping_mul(ONE) / (i64::from(level) * ONE)
    } else {
        0xffff_ffff
    }
}

fn skill_level(
    memory: &impl Memory,
    skills: Array,
    specialization: u32,
) -> Result<i32, &'static str> {
    for index in 0..skills.count {
        let entry = skills
            .element(index, scientist_skill_entry::STRIDE)
            .ok_or("facility skill address invalid")?;
        if word::<u32>(memory, entry, scientist_skill_entry::TOKEN)? == specialization {
            return word(memory, entry, scientist_skill_entry::LEVEL);
        }
    }
    Ok(0)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the program pointer and row identify one program, and the owner, assignment and emit callbacks belong to the caller"
)]
pub fn observe(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    program_pointer: usize,
    row: &program::Row,
    active_owner: impl FnOnce() -> Result<(usize, usize), &'static str>,
    assignment_count: usize,
    assignment_reference: impl Fn(usize) -> u64,
    mut emit: impl FnMut(Operand) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut output = |kind, scientist, token, key, ordinal, assignment, raw| {
        emit(Operand {
            kind,
            scientist,
            token,
            key,
            ordinal,
            assignment,
            raw,
        })
    };
    let configuration = pointer(memory, program_pointer, CONFIGURATION)?;
    identity(memory, base, configuration, cbuilding_template::VTABLE_RVA)?;
    let provinces = Array::read(memory, at(game_state, cgame_state::PROVINCES)?, 8)
        .ok_or("facility province roster invalid")?;
    let province = pointer(
        memory,
        provinces
            .element(row.province, 8)
            .ok_or("facility province outside roster")?,
        0,
    )?;
    identity(memory, base, province, cprovince::VTABLE_RVA)?;
    if word::<u32>(memory, province, cprovince::ID)? != row.province {
        return Err("facility province identity mismatch");
    }
    let state = pointer(memory, province, cprovince::STATE)?;
    identity(memory, base, state, cstate::VTABLE_RVA)?;
    let provincial = word::<i32>(memory, configuration, cbuilding_template::SCOPE)? > 0;
    let status = if provincial {
        at(province, 0x190)?
    } else {
        at(state, cstate::BUILDING_STATUS)?
    };
    identity(memory, base, status, cbuilding_status::VTABLE_RVA)?;
    let mut building_factor = 0;
    if word::<u8>(
        memory,
        configuration,
        cbuilding_template::LOOKUP_ENABLED_CANDIDATE,
    )? != 0
    {
        let indices = Array::read(memory, at(status, cbuilding_status::BUILDING_INDICES)?, 4)
            .ok_or("facility building indices invalid")?;
        if indices.count != 0 {
            let ordinal = word::<u32>(memory, configuration, cbuilding_template::DATABASE_ORDINAL)?;
            let index = word::<i32>(
                memory,
                indices
                    .element(ordinal, 4)
                    .ok_or("facility building ordinal outside indices")?,
                0,
            )?;
            if index != -1 {
                let buildings = Array::read(memory, at(status, cbuilding_status::BUILDINGS)?, 8)
                    .ok_or("facility buildings invalid")?;
                let building = pointer(
                    memory,
                    buildings
                        .element(
                            u32::try_from(index).map_err(|_| "facility building index negative")?,
                            8,
                        )
                        .ok_or("facility building index outside roster")?,
                    0,
                )?;
                identity(memory, base, building, cbuilding::VTABLE_RVA)?;
                if pointer(memory, building, cbuilding::TEMPLATE)? != configuration {
                    return Err("facility building template mismatch");
                }
                let level: i16 = word(memory, building, cbuilding::LEVEL)?;
                building_factor = if level == 0 {
                    0
                } else {
                    let healthy = word(memory, building, cbuilding::HEALTHY_LEVEL)?;
                    if level == healthy {
                        ONE
                    } else {
                        health(
                            level,
                            healthy,
                            word(memory, building, cbuilding::PARTIAL_HEALTH)?,
                        )
                        .clamp(-ONE, ONE)
                    }
                };
            }
        }
    }
    output(BUILDING, 0, 0, 0, 0, 0, building_factor)?;
    let supply = word::<i64>(memory, program_pointer, SUPPLY_RATIO)?;
    let minimum = global::<i64>(memory, base, MINIMUM_SUPPLY_RVA)?;
    let supply = if supply < minimum {
        minimum
    } else {
        supply.min(ONE)
    };
    output(SUPPLY, 0, 0, 0, 0, 0, supply)?;
    let basic = word::<i64>(memory, program_pointer, BASIC_FACTOR)?;
    output(BASIC, 0, 0, 0, 0, 0, basic)?;
    let active = if row.active_project.is_some() {
        Some(active_owner()?)
    } else {
        None
    };
    let owner = active.map(|(country, _)| country);
    let project = if let Some(token) = row.active_project {
        crate::nproject_cproject_pool::active_pointer(memory, base, active.unwrap().1, token)?
    } else {
        None
    };
    let definition = project
        .map(|p| pointer(memory, p, PROJECT_DEFINITION))
        .transpose()?;
    let specialization = definition.map(|p| word::<u32>(memory, p, 0)).transpose()?;
    let keys = definition
        .map(|p| -> Result<_, &'static str> {
            Array::read(memory, at(p, PROJECT_SPEED_KEYS)?, 4).ok_or("project speed keys invalid")
        })
        .transpose()?;
    let resources = if let Some(project) = project {
        let minimum = global::<i64>(memory, base, MINIMUM_RESOURCES_RVA)?;
        mul(
            ONE.wrapping_sub(minimum),
            word(memory, project, PROJECT_RESOURCE_RATIO)?,
        )
        .wrapping_add(minimum)
    } else {
        ONE
    };
    output(RESOURCES, 0, 0, 0, 0, 0, resources)?;
    let specializations = if (0..assignment_count).any(|index| assignment_reference(index) != 0) {
        Array::read(memory, at(configuration, SPECIALIZATIONS)?, 4)
            .ok_or("facility specializations invalid")?
    } else {
        Array { data: 0, count: 0 }
    };
    let state_accumulator = if keys.is_some_and(|keys| keys.count != 0) {
        Some(cstate::country_modifier_accumulator(
            memory, game_state, state, row.owner,
        )?)
    } else {
        None
    };
    if let Some(keys) = keys {
        for ordinal in 0..keys.count {
            let key = word::<u32>(
                memory,
                keys.element(ordinal, 4)
                    .ok_or("project speed key address invalid")?,
                0,
            )?;
            let country = modifier_accumulator::requested_effective_at(
                memory,
                base,
                at(owner.unwrap(), engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?,
                key,
            )?
            .wrapping_add(ONE);
            let state = modifier_accumulator::requested_effective_at(
                memory,
                base,
                state_accumulator.unwrap(),
                key,
            )?
            .wrapping_add(ONE);
            output(COUNTRY, 0, 0, key, ordinal, 0, country)?;
            output(STATE, 0, 0, key, ordinal, 0, state)?;
        }
    }
    for ordinal in 0..assignment_count {
        if specialization.is_none() && specializations.count == 0 {
            continue;
        }
        let reference = assignment_reference(ordinal);
        let character = creference_registry::resolve(memory, base, reference)?;
        let Some(character) = character else {
            if project.is_some() {
                output(
                    SKILL,
                    reference,
                    specialization.unwrap(),
                    0,
                    0,
                    ordinal as u32,
                    0,
                )?;
            }
            continue;
        };
        let (id, _) = ccharacter::identity(memory, base, character)?;
        if reference as u32 != ccharacter::REFERENCE_TYPE || (reference >> 32) as u32 != id {
            return Err("facility scientist reference mismatch");
        }
        let scientist = pointer(memory, character, ccharacter::SCIENTIST)?;
        identity(memory, base, scientist, cscientist::VTABLE_RVA)?;
        let skills = Array::read(
            memory,
            at(scientist, cscientist::SKILLS)?,
            scientist_skill_entry::STRIDE,
        )
        .ok_or("facility scientist skills invalid")?;
        let support = ordinal != 0;
        let fraction = if support {
            global(memory, base, SUPPORT_FRACTION_RVA)?
        } else {
            ONE
        };
        if let Some(specialization) = specialization {
            let level = skill_level(memory, skills, specialization)?;
            let levels = Array::read(memory, site(base, SKILL_SPEED_ARRAY_RVA)?, 8)
                .ok_or("scientist skill speed array invalid")?;
            let bonus = word::<i64>(
                memory,
                levels
                    .element(
                        u32::try_from(level).map_err(|_| "scientist skill speed level negative")?,
                        8,
                    )
                    .ok_or("scientist skill speed level outside defines")?,
                0,
            )?;
            let factor = if support {
                mul(fraction, bonus).wrapping_add(ONE).max(ONE)
            } else {
                bonus.wrapping_add(ONE)
            };
            output(
                SKILL,
                reference,
                specialization,
                0,
                0,
                ordinal as u32,
                factor,
            )?;
        }
        for index in 0..specializations.count {
            let spec = word(
                memory,
                specializations
                    .element(index, 4)
                    .ok_or("facility specialization address invalid")?,
                0,
            )?;
            let level = skill_level(memory, skills, spec)?;
            let breakthrough = if level > 0 {
                let gain: i32 = global(memory, base, BREAKTHROUGH_SKILL_GAIN_RVA)?;
                let modifier = modifier_accumulator::requested_effective_at(
                    memory,
                    base,
                    at(scientist, SCIENTIST_MODIFIERS)?,
                    BREAKTHROUGH_MODIFIER,
                )?;
                mul(
                    mul(
                        mul(i64::from(level.wrapping_mul(gain)).wrapping_mul(basic), ONE),
                        modifier.wrapping_add(ONE),
                    ),
                    fraction,
                )
            } else {
                ONE
            };
            let technology_define: i64 = global(memory, base, TECHNOLOGY_BONUS_RVA)?;
            let modifier = modifier_accumulator::requested_effective_at(
                memory,
                base,
                at(scientist, SCIENTIST_MODIFIERS)?,
                TECHNOLOGY_MODIFIER,
            )?;
            let technology = mul(
                mul(
                    mul(
                        mul(i64::from(level).wrapping_mul(ONE), technology_define),
                        basic,
                    ),
                    modifier.wrapping_add(ONE),
                ),
                fraction,
            );
            let experience = mul(global(memory, base, BASIC_EXPERIENCE_RVA)?, fraction);
            output(
                BREAKTHROUGH,
                reference,
                spec,
                0,
                index,
                ordinal as u32,
                breakthrough,
            )?;
            output(
                TECHNOLOGY,
                reference,
                spec,
                0,
                index,
                ordinal as u32,
                technology,
            )?;
            output(
                EXPERIENCE,
                reference,
                spec,
                0,
                index,
                ordinal as u32,
                experience,
            )?;
        }
        if let Some(keys) = keys.filter(|keys| keys.count != 0) {
            let injury = if word::<i32>(memory, scientist, cscientist::INJURY_DAYS)? > 0 {
                global(memory, base, INJURY_FACTOR_RVA)?
            } else {
                ONE
            };
            let traits = Array::read(memory, at(scientist, cscientist::TRAIT_DATA)?, 8)
                .ok_or("facility scientist traits invalid")?;
            for index in 0..keys.count {
                let key = word(
                    memory,
                    keys.element(index, 4)
                        .ok_or("project modifier key address invalid")?,
                    0,
                )?;
                output(INJURY, reference, 0, key, index, ordinal as u32, injury)?;
                for trait_index in 0..traits.count {
                    let trait_pointer = pointer(
                        memory,
                        traits
                            .element(trait_index, 8)
                            .ok_or("scientist trait address invalid")?,
                        0,
                    )?;
                    identity(memory, base, trait_pointer, cscientist::TRAIT_VTABLE_RVA)?;
                    let token = word(memory, trait_pointer, cscientist::TRAIT_TOKEN)?;
                    let amount = modifier_accumulator::requested_effective_at(
                        memory,
                        base,
                        at(trait_pointer, TRAIT_MODIFIERS)?,
                        key,
                    )?;
                    let value = mul(fraction, amount).wrapping_add(ONE);
                    output(TRAIT, reference, token, key, index, ordinal as u32, value)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use engine_native::ccountry;
    use super::*;
    struct Idle {
        bytes: Vec<u8>,
    }
    impl Memory for Idle {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == MINIMUM_SUPPLY_RVA.0 && output.len() == 8 {
                output.copy_from_slice(&25_000i64.to_le_bytes());
                return true;
            }
            if let Some(bytes) = self
                .bytes
                .get(address..address.saturating_add(output.len()))
            {
                output.copy_from_slice(bytes);
                true
            } else {
                false
            }
        }
    }
    #[test]
    fn idle_facility_reads_no_country_project_or_scientist_roster_and_uses_native_supply_floor() {
        let mut memory = Idle {
            bytes: vec![0; 0x3000],
        };
        let put = |memory: &mut Idle, at: usize, bytes: &[u8]| {
            memory.bytes[at..at + bytes.len()].copy_from_slice(bytes)
        };
        let config = 0x1200usize;
        let province = 0x1800usize;
        let state = 0x2000usize;
        let program = 0x800usize;
        put(
            &mut memory,
            program + CONFIGURATION,
            &(config as u64).to_le_bytes(),
        );
        put(
            &mut memory,
            config,
            &(cbuilding_template::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut memory,
            cgame_state::PROVINCES,
            &0x1000u64.to_le_bytes(),
        );
        put(&mut memory, cgame_state::PROVINCES + 8, &2u32.to_le_bytes());
        put(
            &mut memory,
            cgame_state::PROVINCES + 12,
            &2i32.to_le_bytes(),
        );
        put(&mut memory, 0x1008, &(province as u64).to_le_bytes());
        put(
            &mut memory,
            province,
            &(cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut memory, province + cprovince::ID, &1u32.to_le_bytes());
        put(
            &mut memory,
            province + cprovince::STATE,
            &(state as u64).to_le_bytes(),
        );
        put(
            &mut memory,
            state,
            &(cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut memory,
            state + cstate::BUILDING_STATUS,
            &(cbuilding_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut memory, program + SUPPLY_RATIO, &(-1i64).to_le_bytes());
        put(&mut memory, program + BASIC_FACTOR, &12345i64.to_le_bytes());
        let mut operands = Vec::new();
        observe(
            &memory,
            0,
            0,
            program,
            &program::Row {
                province: 1,
                ..Default::default()
            },
            || panic!("idle facility must not request an active owner"),
            1,
            |_| 0,
            |operand| {
                operands.push(operand);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            operands
                .iter()
                .map(|operand| (operand.kind, operand.raw))
                .collect::<Vec<_>>(),
            [
                (BUILDING, 0),
                (SUPPLY, 25000),
                (BASIC, 12345),
                (RESOURCES, ONE)
            ]
        );
        for operand in operands {
            let mut bytes = [0; BYTES];
            operand.encode(&mut bytes);
            assert_eq!(Operand::decode(&bytes), operand);
        }
    }
    #[test]
    fn average_health_keeps_partial_rounding_signed_levels_and_native_sentinels() {
        assert_eq!(health(0, 0, 0), 0);
        assert_eq!(health(2, 2, -100), ONE);
        assert_eq!(health(2, 1, 5_000_000), 75_000);
        assert_eq!(health(-2, -1, 0), 50_000);
        assert_eq!(health(1, 0, i64::MAX), 0xffff_ffff);
    }

    struct Active {
        local: Vec<u8>,
        globals: std::collections::BTreeMap<usize, u8>,
    }
    impl Active {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            if let Some(output) = self.local.get_mut(address..address + bytes.len()) {
                output.copy_from_slice(bytes);
            } else {
                for (index, byte) in bytes.iter().enumerate() {
                    self.globals.insert(address + index, *byte);
                }
            }
        }
        fn u64(&mut self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        fn u32(&mut self, address: usize, value: u32) {
            self.put(address, &value.to_le_bytes());
        }
        fn array(&mut self, address: usize, data: usize, count: u32) {
            self.u64(address, data as u64);
            self.u32(address + 8, count);
            self.u32(address + 12, count);
        }
    }
    impl Memory for Active {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if let Some(bytes) = self
                .local
                .get(address..address.saturating_add(output.len()))
            {
                output.copy_from_slice(bytes);
                return true;
            }
            for (index, byte) in output.iter_mut().enumerate() {
                let Some(value) = self.globals.get(&(address + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }
    #[test]
    fn active_facility_keeps_partial_health_acquired_resources_duplicate_tag_keys_and_scientist_contributions()
     {
        use crate::{
            nproject_cprogram_status as status, nproject_cproject as project,
            nproject_cproject_pool as pool,
        };
        let mut memory = Active {
            local: vec![0; 0x20000],
            globals: Default::default(),
        };
        let program = 0x800;
        let config = 0x1200;
        let province = 0x1800;
        let state = 0x2000;
        memory.u64(program + CONFIGURATION, config as u64);
        memory.u64(config, cbuilding_template::VTABLE_RVA.0 as u64);
        memory.array(cgame_state::PROVINCES, 0x19000, 2);
        memory.u64(0x19008, province as u64);
        memory.u64(province, cprovince::VTABLE_RVA.0 as u64);
        memory.u32(province + cprovince::ID, 1);
        memory.u64(province + cprovince::STATE, state as u64);
        memory.u64(state, cstate::VTABLE_RVA.0 as u64);
        memory.u64(
            state + cstate::BUILDING_STATUS,
            cbuilding_status::VTABLE_RVA.0 as u64,
        );
        memory.put(config + cbuilding_template::LOOKUP_ENABLED_CANDIDATE, &[1]);
        memory.array(
            state + cstate::BUILDING_STATUS + cbuilding_status::BUILDING_INDICES,
            0x17000,
            1,
        );
        memory.array(
            state + cstate::BUILDING_STATUS + cbuilding_status::BUILDINGS,
            0x17100,
            1,
        );
        memory.u64(0x17100, 0x17200);
        memory.u64(0x17200, cbuilding::VTABLE_RVA.0 as u64);
        memory.put(0x17200 + cbuilding::LEVEL, &2i16.to_le_bytes());
        memory.put(0x17200 + cbuilding::HEALTHY_LEVEL, &1i16.to_le_bytes());
        memory.u64(0x17200 + cbuilding::PARTIAL_HEALTH, 5_000_000);
        memory.u64(0x17200 + cbuilding::TEMPLATE, config as u64);
        memory.u64(program + SUPPLY_RATIO, 80_000);
        memory.u64(program + BASIC_FACTOR, 100_000);
        memory.array(cgame_state::COUNTRIES, 0x5000, 2);
        memory.u64(0x5008, 0x6000);
        memory.array(cgame_state::COUNTRY_LINKS, 0x5100, 2);
        memory.u32(0x5104, 1);
        memory.u64(0x6000, ccountry::VTABLE_RVA.0 as u64);
        memory.u32(0x6000 + ccountry::ID, 1);
        memory.u64(0x6000 + ccountry::PROGRAM_STATUS, 0x8000);
        memory.u64(0x8000, status::VTABLE_RVA.0 as u64);
        memory.u64(0x8008, status::SECONDARY_VTABLE_RVA.0 as u64);
        memory.u32(0x8010, 1);
        memory.u64(0x8000 + status::PROJECT_POOL, 0x9000);
        memory.u64(0x9000, pool::VTABLE_RVA.0 as u64);
        memory.array(0x9000 + pool::PROJECTS, 0xa000, 1);
        memory.u64(0x9030, 0x9400);
        memory.put(0x9400, &[1]);
        memory.u64(0xa000, project::VTABLE_RVA.0 as u64);
        memory.u64(0xa000 + PROJECT_DEFINITION, 0xb018);
        memory.u64(0xa000 + PROJECT_RESOURCE_RATIO, 50_000);
        memory.u32(0xb018, 501);
        memory.array(0xb018 + PROJECT_SPEED_KEYS, 0x13000, 2);
        memory.u32(0x13000, 600);
        memory.u32(0x13004, 600);
        memory.array(config + SPECIALIZATIONS, 0xc000, 1);
        memory.u32(0xc000, 501);
        memory.u64(state + cstate::COUNTRY_MODIFIERS, 0x17500);
        let reference = (7u64 << 32) | u64::from(ccharacter::REFERENCE_TYPE);
        memory.u64(
            creference_registry::LOW_ROOTS_RVA.0 + ccharacter::REFERENCE_TYPE as usize * 8,
            0xf000,
        );
        memory.u64(0xf008, 0xf100);
        memory.u32(0xf010, 1);
        memory.u32(0xf014, 7);
        memory.put(0xf018, &[5]);
        memory.put(0xf100 + 13 * 24 + 4, &[255]);
        let hash = reference
            .rotate_left(32)
            .wrapping_mul(creference_registry::HASH_MULTIPLIER);
        let bucket = 0xf100 + (((hash >> 32) as u32 ^ hash as u32) & 7) as usize * 24;
        memory.put(bucket + 4, &[1]);
        memory.u64(bucket + 8, reference);
        memory.u64(bucket + 16, 0xe000);
        memory.u64(0xe000, ccharacter::VTABLE_RVA.0 as u64);
        memory.u32(0xe000 + ccharacter::TYPE, ccharacter::REFERENCE_TYPE);
        memory.u32(0xe000 + ccharacter::ID, 7);
        memory.u64(0xe000 + ccharacter::SCIENTIST, 0xd000);
        memory.u64(0xd000, cscientist::VTABLE_RVA.0 as u64);
        memory.array(0xd000 + cscientist::SKILLS, 0xc100, 1);
        memory.u32(0xc100, 501);
        memory.u32(0xc100 + scientist_skill_entry::LEVEL, 2);
        memory.array(0xd000 + cscientist::TRAIT_DATA, 0xc200, 1);
        memory.u64(0xc200, 0xc300);
        memory.u64(0xc300, cscientist::TRAIT_VTABLE_RVA.0 as u64);
        memory.u32(0xc300 + cscientist::TRAIT_TOKEN, 701);
        memory.u32(0xd000 + cscientist::INJURY_DAYS, 8);
        for (address, value) in [
            (MINIMUM_SUPPLY_RVA, 25000),
            (MINIMUM_RESOURCES_RVA, 20000),
            (INJURY_FACTOR_RVA, 40000),
            (TECHNOLOGY_BONUS_RVA, 1000),
            (BASIC_EXPERIENCE_RVA, 10000),
        ] {
            memory.u64(address.0, value);
        }
        memory.u32(BREAKTHROUGH_SKILL_GAIN_RVA.0, 2);
        memory.array(SKILL_SPEED_ARRAY_RVA.0, 0x11000, 3);
        memory.u64(0x11010, 20000);
        let mut values = Vec::with_capacity(32);
        observe(
            &memory,
            0,
            0,
            program,
            &program::Row {
                owner: 1,
                province: 1,
                scientist: reference,
                active_project: Some(0),
                ..Default::default()
            },
            || {
                let countries = ccountry::Reader::new(&memory, 0, 0)?;
                let owner = countries.resolve(1)?;
                Ok((
                    owner.pointer,
                    status::from_country(&memory, 0, owner, &countries)?,
                ))
            },
            1,
            |_| reference,
            |value| {
                values.push(value);
                Ok(())
            },
        )
        .unwrap();
        let raw = |kind| values.iter().find(|value| value.kind == kind).unwrap().raw;
        assert_eq!(
            (raw(BUILDING), raw(SUPPLY), raw(RESOURCES), raw(SKILL)),
            (75000, 80000, 60000, 120000)
        );
        assert_eq!(
            (raw(BREAKTHROUGH), raw(TECHNOLOGY), raw(EXPERIENCE)),
            (400000, 2000, 10000)
        );
        assert_eq!(
            values
                .iter()
                .filter(|value| value.kind == COUNTRY)
                .map(|value| (value.key, value.ordinal))
                .collect::<Vec<_>>(),
            [(600, 0), (600, 1)]
        );
        assert_eq!(
            values
                .iter()
                .filter(|value| value.kind == INJURY)
                .map(|value| value.raw)
                .collect::<Vec<_>>(),
            [40000, 40000]
        );
        memory.array(0xd000 + cscientist::SKILLS, 0xc100, 0);
        let mut missing = Vec::new();
        observe(
            &memory,
            0,
            0,
            program,
            &program::Row {
                owner: 1,
                province: 1,
                scientist: reference,
                active_project: Some(0),
                ..Default::default()
            },
            || Ok((0x6000, 0x8000)),
            1,
            |_| reference,
            |value| {
                missing.push(value);
                Ok(())
            },
        )
        .unwrap();
        let raw = |kind| missing.iter().find(|value| value.kind == kind).unwrap().raw;
        assert_eq!(
            (raw(BUILDING), raw(SUPPLY), raw(RESOURCES), raw(SKILL)),
            (75000, 80000, 60000, 100000)
        );
        assert_eq!(
            (raw(BREAKTHROUGH), raw(TECHNOLOGY), raw(EXPERIENCE)),
            (100000, 0, 10000)
        );
    }
}
