use crate::{Memory, cbuilding, cbuilding_database, cbuilding_status as status, cbuilding_template as template, ccountry, cgame_state, cpdx_array::Array, cprovince, cstate, modifier_accumulator, read};
use std::collections::BTreeMap;

const INFRASTRUCTURE_DEFINITIONS: usize = 0x280;
const STATE_MAXIMUM: usize = 0x2b8;
const PROVINCE_MAXIMUM: usize = 0x2bc;
const SHARES_SLOTS: usize = 0x2c0;
const GROUP: usize = 0x2c4;
const NO_GROUP: u32 = 0x165;
const EXCLUSIVE: usize = 0x2c8;
const ENABLE_TECHNOLOGIES: usize = 0x1ec;
const FORT_WEIGHTS: [usize; 2] = [0x304, 0x308];
const DEFAULT_MAXIMUM: usize = 0x033e_5274;
const MAXIMUM_SHARED_SLOTS: usize = 0x033e_59cc;
const GAME_RULES: usize = 0x440;
const FORT_MAXIMUM: usize = 0x24;
const CATEGORY_OVERRIDES: usize = 0x128;
const CATEGORY_HAS_OVERRIDES: usize = 0x10;
const MAXIMUM_BONUS: usize = 0x1e8;
const PRIMARY_CONSTRUCTION: usize = 0x10;
const SECONDARY_CONSTRUCTION: usize = 0x28;
const CONSTRUCTION_LEVEL: usize = 0x48;
const CONVERSION: usize = 0x94;
const TECHNOLOGY_LIMITS: usize = 0x58;
const EXTRA_SLOTS: usize = 0x858;
const CATEGORY_MAP_DATA: usize = 0;
const CATEGORY_MAP_MASK: usize = 12;
const CATEGORY_MAP_OVERFLOW: usize = 16;
const CATEGORY_ENTRY_STRIDE: usize = 12;
const CATEGORY_ENTRY_PROBE: usize = 0;
const CATEGORY_ENTRY_BUILDING: usize = 4;
const CATEGORY_ENTRY_MAXIMUM: usize = 8;
const GLOBAL_BUILDING_SLOTS: u32 = 0xa1;
const GLOBAL_BUILDING_SLOTS_FACTOR: u32 = 0xa2;
const LOCAL_BUILDING_SLOTS: u32 = 0xa3;
const LOCAL_BUILDING_SLOTS_FACTOR: u32 = 0xa4;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Infrastructure {
    pub building_type: u32,
    pub level: i32,
    pub maximum: i32,
}

fn field<T: crate::memory::Raw>(memory: &impl Memory, object: usize, offset: usize) -> Result<T, &'static str> {
    object.checked_add(offset).and_then(|address| read(memory, address)).ok_or("infrastructure map operand unreadable")
}

fn array(memory: &impl Memory, object: usize, offset: usize, stride: usize) -> Result<Array, &'static str> {
    object.checked_add(offset).and_then(|address| Array::read(memory, address, stride)).ok_or("infrastructure map array invalid")
}

fn authenticate(memory: &impl Memory, base: usize, object: usize, vtable: usize) -> Result<(), &'static str> {
    if object == 0 || field::<u64>(memory, object, 0)? != base.checked_add(vtable).ok_or("infrastructure map vtable overflow")? as u64 {
        return Err("infrastructure map object identity invalid");
    }
    Ok(())
}

fn building(memory: &impl Memory, base: usize, status: usize, definition: usize) -> Result<Option<usize>, &'static str> {
    if field::<u8>(memory, definition, template::LOOKUP_ENABLED_CANDIDATE)? == 0 || field::<i32>(memory, status, status::BUILDING_INDEX_COUNT)? == 0 {
        return Ok(None);
    }
    let indices = array(memory, status, status::BUILDING_INDICES, 4)?;
    let ordinal = field::<u32>(memory, definition, template::DATABASE_ORDINAL)?;
    let index = indices.element(ordinal, 4).and_then(|address| read::<i32>(memory, address)).ok_or("infrastructure map building index invalid")?;
    if index == status::NO_BUILDING_INDEX { return Ok(None); }
    let buildings = array(memory, status, status::BUILDINGS, 8)?;
    let pointer = u32::try_from(index).ok().and_then(|index| buildings.element(index, 8)).and_then(|address| read::<u64>(memory, address)).ok_or("infrastructure map building pointer invalid")? as usize;
    if pointer == 0 { return Ok(None); }
    authenticate(memory, base, pointer, cbuilding::VTABLE_RVA)?;
    if field::<u64>(memory, pointer, cbuilding::TEMPLATE)? != definition as u64 { return Err("infrastructure map building template mismatch"); }
    Ok(Some(pointer))
}

fn construction(memory: &impl Memory, building: usize) -> Result<(i16, i16), &'static str> {
    let mut ordinary = 0i16;
    let mut conversion = 0i16;
    for (offset, sum) in [(PRIMARY_CONSTRUCTION, false), (SECONDARY_CONSTRUCTION, true)] {
        let entries = array(memory, building, offset, 8)?;
        let mut levels = 0i16;
        for index in 0..entries.count {
            let entry = entries.element(index, 8).and_then(|address| read::<u64>(memory, address)).filter(|&entry| entry != 0).ok_or("infrastructure map construction pointer invalid")? as usize;
            let level = field::<i16>(memory, entry, CONSTRUCTION_LEVEL)?;
            if field::<u8>(memory, entry, CONVERSION)? == 0 {
                levels = if sum { levels.wrapping_add(level) } else { levels.max(level) };
            } else if !sum {
                conversion = conversion.max(level);
            }
        }
        ordinary = ordinary.wrapping_add(levels);
    }
    Ok((ordinary, conversion))
}

fn category_limit(memory: &impl Memory, category: usize, token: u32, fallback: i32) -> Result<i32, &'static str> {
    if field::<u8>(memory, category, CATEGORY_HAS_OVERRIDES)? == 0 { return Ok(fallback); }
    let root = category.checked_add(CATEGORY_OVERRIDES).ok_or("infrastructure map category overflow")?;
    let data = field::<u64>(memory, root, CATEGORY_MAP_DATA)? as usize;
    let mask = field::<u32>(memory, root, CATEGORY_MAP_MASK)?;
    let overflow = field::<u8>(memory, root, CATEGORY_MAP_OVERFLOW)?;
    let buckets = mask.checked_add(1).filter(|count| count.is_power_of_two()).ok_or("infrastructure map category mask invalid")?;
    let extent = u64::from(buckets) + u64::from(overflow);
    let mut hash = (token >> 16 ^ token).wrapping_mul(0x45d9f3b);
    hash = (hash >> 16 ^ hash).wrapping_mul(0x45d9f3b);
    let mut index = u64::from((hash >> 16 ^ hash) & mask);
    let mut distance = 1u16;
    while index < extent && distance <= u16::from(u8::MAX) {
        let address = data.checked_add(index as usize * CATEGORY_ENTRY_STRIDE).ok_or("infrastructure map category entry overflow")?;
        let probe = field::<u8>(memory, address, CATEGORY_ENTRY_PROBE)?;
        if probe == u8::MAX || u16::from(probe) < distance { return Ok(fallback); }
        if field::<u32>(memory, address, CATEGORY_ENTRY_BUILDING)? == token { return field(memory, address, CATEGORY_ENTRY_MAXIMUM); }
        index += 1;
        distance += 1;
    }
    Err("infrastructure map category probe exceeds storage")
}

fn visit_buildings(memory: &impl Memory, base: usize, status: usize, mut visit: impl FnMut(usize, usize) -> Result<(), &'static str>) -> Result<(), &'static str> {
    let buildings = array(memory, status, status::BUILDINGS, 8)?;
    for index in 0..buildings.count {
        let building = buildings.element(index, 8).and_then(|address| read::<u64>(memory, address)).filter(|&pointer| pointer != 0).ok_or("infrastructure map grouped building invalid")? as usize;
        authenticate(memory, base, building, cbuilding::VTABLE_RVA)?;
        let definition = field::<u64>(memory, building, cbuilding::TEMPLATE)? as usize;
        authenticate(memory, base, definition, template::VTABLE_RVA)?;
        visit(building, definition)?;
    }
    Ok(())
}

fn grouped(memory: &impl Memory, base: usize, status: usize, definition: usize, group: u32) -> Result<i32, &'static str> {
    let mut total = 0i32;
    let mut add = |building| {
        let (queued, _) = construction(memory, building)?;
        total = total.wrapping_add(i32::from(field::<i16>(memory, building, cbuilding::LEVEL)?)).wrapping_add(i32::from(queued));
        Ok(())
    };
    if group == NO_GROUP {
        if let Some(building) = building(memory, base, status, definition)? { add(building)?; }
    } else {
        visit_buildings(memory, base, status, |building, other| {
            if field::<u32>(memory, other, GROUP)? == group { add(building)?; }
            Ok(())
        })?;
    }
    Ok(total)
}

fn free_slots(memory: &impl Memory, base: usize, game: usize, state: usize, countries: &ccountry::Reader<'_, impl Memory>) -> Result<i32, &'static str> {
    let owner = field::<u32>(memory, state, cstate::OWNER)?;
    let country = countries.resolve(owner)?;
    let state_accumulator = cstate::country_modifier_accumulator(memory, game, state, owner)?;
    let country_accumulator = country.pointer.checked_add(ccountry::MODIFIER_ACCUMULATOR).ok_or("infrastructure map country accumulator overflow")?;
    let modifier = |root, key| modifier_accumulator::lookup_effective_at(memory, base, root, key).map(|value| value.effective);
    let mut slots = modifier(state_accumulator, LOCAL_BUILDING_SLOTS)?.wrapping_add(modifier(country_accumulator, GLOBAL_BUILDING_SLOTS)?);
    if slots > 0 {
        let factor = 100_000i64.wrapping_add(modifier(state_accumulator, LOCAL_BUILDING_SLOTS_FACTOR)?).wrapping_add(modifier(country_accumulator, GLOBAL_BUILDING_SLOTS_FACTOR)?);
        slots = factor.wrapping_mul(slots) / 100_000;
    }
    let maximum = ((slots / 100_000) as i32).wrapping_add(field::<i32>(memory, state, EXTRA_SLOTS)?).max(0).min(field(memory, base, MAXIMUM_SHARED_SLOTS)?);
    let mut occupied = 0i32;
    visit_buildings(memory, base, state + cstate::BUILDING_STATUS, |building, definition| {
        if field::<u8>(memory, definition, SHARES_SLOTS)? != 0 {
            occupied = occupied.wrapping_add(i32::from(field::<i16>(memory, building, cbuilding::LEVEL)?)).wrapping_add(i32::from(construction(memory, building)?.0));
        }
        Ok(())
    })?;
    Ok(maximum.wrapping_sub(occupied).max(0))
}

struct Inputs {
    definition: usize,
    token: u32,
    maximum: i32,
    state_maximum: i32,
    group: u32,
    exclusive: Vec<u32>,
    shares_slots: bool,
    technology: bool,
    fort: bool,
}

impl Inputs {
    fn read(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let database = field::<u64>(memory, base, cbuilding_database::SINGLETON_RVA)? as usize;
        let definitions = array(memory, database, INFRASTRUCTURE_DEFINITIONS, 8)?;
        let definition = definitions.element(0, 8).and_then(|address| read::<u64>(memory, address)).ok_or("infrastructure map definition unavailable")? as usize;
        authenticate(memory, base, definition, template::VTABLE_RVA)?;
        let province_maximum = field::<i32>(memory, definition, PROVINCE_MAXIMUM)?;
        let state_maximum = field::<i32>(memory, definition, STATE_MAXIMUM)?;
        let maximum = if province_maximum > 0 { province_maximum } else if state_maximum > 0 { state_maximum } else { field(memory, base, DEFAULT_MAXIMUM)? };
        let entries = array(memory, definition, EXCLUSIVE, 4)?;
        let mut exclusive = Vec::new();
        crate::allocation::reserve(&mut exclusive, entries.count as usize).map_err(|_| "infrastructure map exclusivity allocation failed")?;
        for index in 0..entries.count {
            exclusive.push(entries.element(index, 4).and_then(|address| read(memory, address)).ok_or("infrastructure map exclusivity token unreadable")?);
        }
        Ok(Self { definition, token: field(memory, definition, template::TYPE_ID)?, maximum, state_maximum, group: field(memory, definition, GROUP)?, exclusive, shares_slots: field::<u8>(memory, definition, SHARES_SLOTS)? != 0, technology: field::<i32>(memory, definition, ENABLE_TECHNOLOGIES)? != 0, fort: field::<i32>(memory, definition, FORT_WEIGHTS[0])? > 0 || field::<i32>(memory, definition, FORT_WEIGHTS[1])? > 0 })
    }

    fn state(&self, memory: &impl Memory, base: usize, game: usize, state: usize, countries: &ccountry::Reader<'_, impl Memory>, technologies: &mut BTreeMap<u32, i32>) -> Result<Infrastructure, &'static str> {
        let status = state.checked_add(cstate::BUILDING_STATUS).ok_or("infrastructure map status overflow")?;
        let instance = building(memory, base, status, self.definition)?;
        let (level, aggregate, bonus) = match instance {
            Some(building) => {
                let level = field::<i16>(memory, building, cbuilding::LEVEL)?;
                let (queued, conversion) = construction(memory, building)?;
                (i32::from(level), i32::from(level.wrapping_add(queued).wrapping_add(conversion)), i32::from(field::<i16>(memory, building, MAXIMUM_BONUS)?))
            }
            None => (0, 0, 0),
        };
        let category = field::<u64>(memory, state, cstate::CATEGORY)? as usize;
        authenticate(memory, base, category, cstate::CATEGORY_VTABLE_RVA)?;
        let mut maximum = self.maximum.min(category_limit(memory, category, self.token, self.maximum)?.wrapping_add(bonus));
        if field::<i32>(memory, status, status::COUNTRY)? > 0 {
            maximum = i32::MAX;
        } else {
            if self.fort {
                let rules = field::<u64>(memory, game, GAME_RULES)? as usize;
                let fort = field::<i32>(memory, rules, FORT_MAXIMUM)?;
                if fort >= 0 { maximum = fort; }
            }
            if self.technology {
                let controller = countries.resolve(field::<u32>(memory, state, cstate::CONTROLLER)?)?;
                let limit = if let Some(&limit) = technologies.get(&controller.canonical_slot) { limit } else {
                    let status = field::<u64>(memory, controller.pointer, ccountry::TECHNOLOGY_STATUS)? as usize;
                    let limits = array(memory, status, TECHNOLOGY_LIMITS, 4)?;
                    let ordinal = field::<u32>(memory, self.definition, template::DATABASE_ORDINAL)?;
                    let limit = limits.element(ordinal, 4).and_then(|address| read::<i32>(memory, address)).ok_or("infrastructure map technology limit unavailable")?;
                    technologies.insert(controller.canonical_slot, limit);
                    limit
                };
                maximum = maximum.min(limit);
            }
        }
        if !self.exclusive.is_empty() {
            let mut blocked = false;
            let active = array(memory, status, status::ACTIVE_BUILDINGS, 8)?;
            for index in 0..active.count {
                let building = active.element(index, 8).and_then(|address| read::<u64>(memory, address)).ok_or("infrastructure map active building unreadable")? as usize;
                authenticate(memory, base, building, cbuilding::VTABLE_RVA)?;
                if self.exclusive.contains(&field(memory, building, cbuilding::TYPE_ID)?) { blocked = true; }
            }
            visit_buildings(memory, base, status, |candidate, definition| {
                if self.exclusive.contains(&field(memory, definition, template::TYPE_ID)?)
                    && building(memory, base, status, definition)? == Some(candidate)
                    && { let (ordinary, conversion) = construction(memory, candidate)?; ordinary.wrapping_add(conversion) > 0 }
                { blocked = true; }
                Ok(())
            })?;
            if blocked { return Ok(Infrastructure { building_type: self.token, level, maximum: 0 }); }
        }
        if self.state_maximum > 0 {
            let mut used = grouped(memory, base, status, self.definition, self.group)?;
            if self.group != NO_GROUP {
                let provinces = array(memory, state, cstate::PROVINCES, 8)?;
                for index in 0..provinces.count {
                    let province = provinces.element(index, 8).and_then(|address| read::<u64>(memory, address)).ok_or("infrastructure map province pointer unreadable")? as usize;
                    authenticate(memory, base, province, cprovince::VTABLE_RVA)?;
                    used = used.wrapping_add(grouped(memory, base, province + cprovince::BUILDING_STATUS, self.definition, self.group)?);
                }
            }
            maximum = maximum.min(aggregate.wrapping_add(self.state_maximum.wrapping_add(bonus).wrapping_sub(used).max(0)));
        }
        if self.shares_slots { maximum = maximum.min(aggregate.wrapping_add(free_slots(memory, base, game, state, countries)?)); }
        Ok(Infrastructure { building_type: self.token, level, maximum })
    }
}

pub fn snapshot(memory: &impl Memory, base: usize, game: usize, census: crate::walk::Outcome, rows: &mut [cstate::Row]) {
    let rows = &mut rows[..census.len()];
    for row in rows.iter_mut() { row.infrastructure = None; }
    let load = || -> Result<_, &'static str> { Ok((Inputs::read(memory, base)?, ccountry::Reader::new(memory, base, game)?, array(memory, game, cgame_state::STATES, 8)?)) };
    let Ok((inputs, countries, states)) = load() else { return; };
    let mut technologies = BTreeMap::new();
    for row in rows {
        row.infrastructure = (|| {
            let state = states.element(row.id, 8).and_then(|address| read::<u64>(memory, address)).ok_or("infrastructure map state pointer unreadable")? as usize;
            authenticate(memory, base, state, cstate::VTABLE_RVA)?;
            if field::<u32>(memory, state, cstate::ID)? != row.id { return Err("infrastructure map state identity mismatch"); }
            inputs.state(memory, base, game, state, &countries, &mut technologies)
        })().ok();
    }
}
