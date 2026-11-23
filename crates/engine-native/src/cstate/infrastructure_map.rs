use engine_sites::Rva;
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{
        RecordField,
        RecordLayout,
        RecordSources,
        gather_records,
        packed_fields,
    },
    read,
};
use engine_native_parts::{
    cbuilding,
    cbuilding_database,
    cbuilding_template as template,
    modifier_accumulator,
};
use crate::{cprovince::{buildings as status, self}, ccountry, cgame_state, cstate};

const INFRASTRUCTURE_DEFINITIONS: usize = 0x280;
const STATE_MAXIMUM: usize = 0x2b8;
const PROVINCE_MAXIMUM: usize = 0x2bc;
const SHARES_SLOTS: usize = 0x2c0;
const GROUP: usize = 0x2c4;
const NO_GROUP: u32 = 0x165;
const EXCLUSIVE: usize = 0x2c8;
const ENABLE_TECHNOLOGIES: usize = 0x1ec;
const FORT_WEIGHTS: [usize; 2] = [0x304, 0x308];
const DEFAULT_MAXIMUM: Rva = Rva(0x033e_5274);
const MAXIMUM_SHARED_SLOTS: Rva = Rva(0x033e_59cc);
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
const EXTRA_SLOTS: usize = cstate::EXTRA_SHARED_SLOTS;
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
const QUEUE_FIELDS: [RecordField; 3] = packed_fields([(PRIMARY_CONSTRUCTION, 16), (SECONDARY_CONSTRUCTION, 16), (MAXIMUM_BONUS, 2)]);
pub const QUEUE_BYTES: usize = 34;
const QUEUE_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&QUEUE_FIELDS, QUEUE_BYTES) {
    Some(layout) => layout,
    None => panic!("valid infrastructure queue layout"),
};
pub type Queue = [u8; QUEUE_BYTES];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Infrastructure {
    pub building_type: u32,
    pub level: i32,
    pub maximum: i32,
}

fn field<T: engine_native_memory::memory::Raw>(memory: &impl Memory, object: usize, offset: usize) -> Result<T, &'static str> {
    object.checked_add(offset).and_then(|address| read(memory, address)).ok_or("infrastructure map operand unreadable")
}

fn array(memory: &impl Memory, object: usize, offset: usize, stride: usize) -> Result<Array, &'static str> {
    object.checked_add(offset).and_then(|address| Array::read(memory, address, stride)).ok_or("infrastructure map array invalid")
}

fn global<T: engine_native_memory::memory::Raw>(memory: &impl Memory, base: usize, key: Rva) -> Result<T, &'static str> {
    engine_sites::at(base, key).and_then(|address| read(memory, address)).ok_or("infrastructure map operand unreadable")
}

fn authenticate(memory: &impl Memory, base: usize, object: usize, vtable: Rva) -> Result<(), &'static str> {
    if object == 0 || field::<u64>(memory, object, 0)? != engine_sites::at(base, vtable).ok_or("infrastructure map vtable overflow")? as u64 {
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
    let headers = [PRIMARY_CONSTRUCTION, SECONDARY_CONSTRUCTION].map(|offset| array(memory, building, offset, 8));
    construction_of(memory, [headers[0]?, headers[1]?])
}

fn queue(memory: &impl Memory, building: usize) -> Result<Queue, &'static str> {
    let mut bytes = [0; QUEUE_BYTES];
    if memory.copy_records(RecordSources::Pointers(&[building]), &QUEUE_LAYOUT, &mut bytes) != QUEUE_FIELDS.len() {
        return Err("infrastructure map construction queue unreadable");
    }
    Ok(bytes)
}

fn queue_headers(queue: &Queue) -> Result<[Array; 2], &'static str> {
    let header = |at: usize| Array::from_header(queue[at..at + 16].try_into().unwrap(), 8).ok_or("infrastructure map array invalid");
    Ok([header(0)?, header(16)?])
}

fn queue_bonus(queue: &Queue) -> i16 {
    i16::from_le_bytes([queue[32], queue[33]])
}

pub fn gather_queues(memory: &impl Memory, instances: &[usize], bytes: &mut [u8], complete: &mut [bool]) {
    gather_records(memory, instances, &QUEUE_LAYOUT, &mut bytes[..instances.len() * QUEUE_BYTES], &mut complete[..instances.len()]);
}

fn construction_of(memory: &impl Memory, headers: [Array; 2]) -> Result<(i16, i16), &'static str> {
    let mut ordinary = 0i16;
    let mut conversion = 0i16;
    for (entries, sum) in [(headers[0], false), (headers[1], true)] {
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
    let country_accumulator = country.pointer.checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR).ok_or("infrastructure map country accumulator overflow")?;
    let modifier = |root, key| modifier_accumulator::lookup_effective_at(memory, base, root, key).map(|value| value.effective);
    let mut slots = modifier(state_accumulator, LOCAL_BUILDING_SLOTS)?.wrapping_add(modifier(country_accumulator, GLOBAL_BUILDING_SLOTS)?);
    if slots > 0 {
        let factor = 100_000i64.wrapping_add(modifier(state_accumulator, LOCAL_BUILDING_SLOTS_FACTOR)?).wrapping_add(modifier(country_accumulator, GLOBAL_BUILDING_SLOTS_FACTOR)?);
        slots = factor.wrapping_mul(slots) / 100_000;
    }
    let maximum = ((slots / 100_000) as i32).wrapping_add(field::<i32>(memory, state, EXTRA_SLOTS)?).max(0).min(global(memory, base, MAXIMUM_SHARED_SLOTS)?);
    let mut occupied = 0i32;
    visit_buildings(memory, base, state + cstate::BUILDING_STATUS, |building, definition| {
        if field::<u8>(memory, definition, SHARES_SLOTS)? != 0 {
            occupied = occupied.wrapping_add(i32::from(field::<i16>(memory, building, cbuilding::LEVEL)?)).wrapping_add(i32::from(construction(memory, building)?.0));
        }
        Ok(())
    })?;
    Ok(maximum.wrapping_sub(occupied).max(0))
}

pub struct Plan {
    database: usize,
    definition: usize,
    ordinal: u32,
    token: u32,
    maximum: i32,
    state_maximum: i32,
    group: u32,
    exclusive: Vec<u32>,
    shares_slots: bool,
    technology: bool,
    fort: bool,
}

impl Plan {
    fn locate(memory: &impl Memory, base: usize) -> Result<(usize, usize), &'static str> {
        let database = global::<u64>(memory, base, cbuilding_database::SINGLETON_RVA)? as usize;
        let definitions = array(memory, database, INFRASTRUCTURE_DEFINITIONS, 8)?;
        let definition = definitions.element(0, 8).and_then(|address| read::<u64>(memory, address)).ok_or("infrastructure map definition unavailable")? as usize;
        Ok((database, definition))
    }

    fn read(memory: &impl Memory, base: usize, (database, definition): (usize, usize)) -> Result<Self, &'static str> {
        authenticate(memory, base, definition, template::VTABLE_RVA)?;
        if field::<i32>(memory, definition, PROVINCE_MAXIMUM)? > 0 { return Err("infrastructure map province-capped definition unsupported"); }
        let state_maximum = field::<i32>(memory, definition, STATE_MAXIMUM)?;
        let maximum = if state_maximum > 0 { state_maximum } else { global(memory, base, DEFAULT_MAXIMUM)? };
        let entries = array(memory, definition, EXCLUSIVE, 4)?;
        let mut exclusive = Vec::new();
        engine_allocation::reserve(&mut exclusive, entries.count as usize).map_err(|_| "infrastructure map exclusivity allocation failed")?;
        for index in 0..entries.count {
            exclusive.push(entries.element(index, 4).and_then(|address| read(memory, address)).ok_or("infrastructure map exclusivity token unreadable")?);
        }
        Ok(Self {
            database,
            definition,
            ordinal: field(memory, definition, template::DATABASE_ORDINAL)?,
            token: field(memory, definition, template::TYPE_ID)?,
            maximum,
            state_maximum,
            group: field(memory, definition, GROUP)?,
            exclusive,
            shares_slots: field::<u8>(memory, definition, SHARES_SLOTS)? != 0,
            technology: field::<i32>(memory, definition, ENABLE_TECHNOLOGIES)? != 0,
            fort: field::<i32>(memory, definition, FORT_WEIGHTS[0])? > 0 || field::<i32>(memory, definition, FORT_WEIGHTS[1])? > 0,
        })
    }

    fn allocated_bytes(&self) -> usize {
        self.exclusive.capacity() * size_of::<u32>()
    }
}

fn exclusive_queued(memory: &impl Memory, plan: &Plan, status: usize) -> Result<bool, &'static str> {
    let templates = array(memory, plan.database, cbuilding_database::TEMPLATES, cbuilding_database::TEMPLATE_STRIDE)?;
    let indices = array(memory, status, status::BUILDING_INDICES, 4)?;
    let buildings = array(memory, status, status::BUILDINGS, 8)?;
    for at in 1..templates.count {
        let definition = templates.element(at, cbuilding_database::TEMPLATE_STRIDE).and_then(|address| read::<u64>(memory, address)).ok_or("infrastructure map building template unreadable")? as usize;
        if !plan.exclusive.contains(&field(memory, definition, template::TYPE_ID)?) { continue; }
        let ordinal = field::<u32>(memory, definition, template::DATABASE_ORDINAL)?;
        let index = indices.element(ordinal, 4).and_then(|address| read::<i32>(memory, address)).ok_or("infrastructure map building index invalid")?;
        if index == status::NO_BUILDING_INDEX { continue; }
        let candidate = u32::try_from(index).ok().and_then(|index| buildings.element(index, 8)).and_then(|address| read::<u64>(memory, address)).ok_or("infrastructure map building pointer invalid")? as usize;
        if candidate == 0 { continue; }
        let (ordinary, conversion) = construction(memory, candidate)?;
        if ordinary.wrapping_add(conversion) > 0 { return Ok(true); }
    }
    Ok(false)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    instance: Option<usize>,
    level: i16,
    country: bool,
    controller: u32,
}

impl Held {
    pub fn instance(&self) -> Option<usize> {
        self.instance
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Proof {
    queue: Option<Queue>,
    category: u64,
    fort: Option<i32>,
    technology: Option<i32>,
    value: Infrastructure,
}

impl Proof {
    pub fn value(&self) -> Infrastructure {
        self.value
    }
}

#[derive(Default)]
pub struct Cache {
    plan: Option<Plan>,
    generation: u64,
    technologies: Vec<(u64, i32)>,
    categories: Vec<(usize, i32)>,
    controllers: Vec<(u32, engine_native_memory::country_identity::Country)>,
}

impl Cache {
    pub fn forget(&mut self) {
        self.plan = None;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.plan.as_ref().map_or(0, Plan::allocated_bytes)
            + self.technologies.capacity() * size_of::<(u64, i32)>()
            + self.categories.capacity() * size_of::<(usize, i32)>()
            + self.controllers.capacity() * size_of::<(u32, engine_native_memory::country_identity::Country)>()
    }
}

pub struct Hour<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    game: usize,
    plan: &'a Plan,
    generation: u64,
    technologies: &'a mut Vec<(u64, i32)>,
    categories: &'a mut Vec<(usize, i32)>,
    controllers: &'a mut Vec<(u32, engine_native_memory::country_identity::Country)>,
    countries: Option<Result<ccountry::Reader<'a, M>, &'static str>>,
    fort: Option<Result<i32, &'static str>>,
    states: Option<Result<Array, &'static str>>,
}

impl<'a, M: Memory> Hour<'a, M> {
    pub fn new(memory: &'a M, base: usize, game: usize, cache: &'a mut Cache, reuse: bool) -> Result<Self, &'static str> {
        let Cache { plan, generation, technologies, categories, controllers } = cache;
        let located = Plan::locate(memory, base);
        let held = reuse && matches!((&*plan, located), (Some(plan), Ok(at)) if (plan.database, plan.definition) == at);
        if !held {
            *plan = None;
            *plan = Some(Plan::read(memory, base, located?)?);
        }
        let plan: &'a Plan = match plan {
            Some(plan) => plan,
            None => return Err("infrastructure map definition unavailable"),
        };
        *generation = generation.wrapping_add(1);
        categories.clear();
        controllers.clear();
        Ok(Self { memory, base, game, plan, generation: *generation, technologies, categories, controllers, countries: None, fort: None, states: None })
    }

    pub fn proves(&self) -> bool {
        self.plan.exclusive.is_empty() && !self.plan.shares_slots && (self.plan.state_maximum <= 0 || self.plan.group == NO_GROUP)
    }

    fn countries(&mut self) -> Result<&ccountry::Reader<'a, M>, &'static str> {
        let (memory, base, game) = (self.memory, self.base, self.game);
        self.countries.get_or_insert_with(|| ccountry::Reader::new(memory, base, game)).as_ref().map_err(|reason| *reason)
    }

    fn fort(&mut self) -> Result<i32, &'static str> {
        let (memory, game) = (self.memory, self.game);
        *self.fort.get_or_insert_with(|| {
            let rules = field::<u64>(memory, game, GAME_RULES)? as usize;
            field::<i32>(memory, rules, FORT_MAXIMUM)
        })
    }

    fn category(&mut self, category: usize) -> Result<i32, &'static str> {
        if let Some(&(_, limit)) = self.categories.iter().find(|(held, _)| *held == category) {
            return Ok(limit);
        }
        authenticate(self.memory, self.base, category, cstate::CATEGORY_VTABLE_RVA)?;
        let limit = category_limit(self.memory, category, self.plan.token, self.plan.maximum)?;
        if engine_allocation::reserve(self.categories, 1).is_ok() {
            self.categories.push((category, limit));
        }
        Ok(limit)
    }

    fn controller(&mut self, raw: u32) -> Result<engine_native_memory::country_identity::Country, &'static str> {
        if let Some(&(_, country)) = self.controllers.iter().find(|(held, _)| *held == raw) {
            return Ok(country);
        }
        let country = self.countries()?.resolve(raw)?;
        if engine_allocation::reserve(self.controllers, 1).is_ok() {
            self.controllers.push((raw, country));
        }
        Ok(country)
    }

    fn technology(&mut self, raw: u32) -> Result<i32, &'static str> {
        let controller = self.controller(raw)?;
        let slot = controller.canonical_slot as usize;
        if let Some(&(generation, limit)) = self.technologies.get(slot)
            && generation == self.generation
        {
            return Ok(limit);
        }
        let status = field::<u64>(self.memory, controller.pointer, ccountry::TECHNOLOGY_STATUS)? as usize;
        let limits = array(self.memory, status, TECHNOLOGY_LIMITS, 4)?;
        let limit = limits.element(self.plan.ordinal, 4).and_then(|address| read::<i32>(self.memory, address)).ok_or("infrastructure map technology limit unavailable")?;
        if slot < self.technologies.len() || engine_allocation::resize_default(self.technologies, slot + 1).is_ok() {
            self.technologies[slot] = (self.generation, limit);
        }
        Ok(limit)
    }

    pub fn state_of(&mut self, id: u32) -> Result<usize, &'static str> {
        let (memory, game) = (self.memory, self.game);
        let states = (*self.states.get_or_insert_with(|| array(memory, game, cgame_state::STATES, 8)))?;
        let state = states.element(id, 8).and_then(|address| read::<u64>(memory, address)).ok_or("infrastructure map state pointer unreadable")? as usize;
        authenticate(memory, self.base, state, cstate::VTABLE_RVA)?;
        if field::<u32>(memory, state, cstate::ID)? != id { return Err("infrastructure map state identity mismatch"); }
        Ok(state)
    }

    pub fn fresh(&mut self, state: usize) -> Result<(Held, Proof), &'static str> {
        let status = state.checked_add(cstate::BUILDING_STATUS).ok_or("infrastructure map status overflow")?;
        let instance = building(self.memory, self.base, status, self.plan.definition)?;
        let level = match instance { Some(building) => field::<i16>(self.memory, building, cbuilding::LEVEL)?, None => 0 };
        let country = field::<i32>(self.memory, status, status::COUNTRY)? > 0;
        let controller = if self.plan.technology { field::<u32>(self.memory, state, cstate::CONTROLLER)? } else { 0 };
        let held = Held { instance, level, country, controller };
        Ok((held, self.carried(state, held)?))
    }

    pub fn quiet(&mut self, held: &Held, proof: &Proof, queue: Option<&Queue>, category: u64) -> bool {
        if !self.proves() || proof.category != category {
            return false;
        }
        match (held.instance, queue, &proof.queue) {
            (Some(_), Some(queue), Some(held_queue)) if queue == held_queue => {
                if !queue_headers(queue).is_ok_and(|headers| headers.iter().all(|entries| entries.count == 0)) {
                    return false;
                }
            }
            (None, None, None) => {}
            _ => return false,
        }
        if !held.country {
            if self.plan.fort && self.fort().ok() != proof.fort {
                return false;
            }
            if self.plan.technology && self.technology(held.controller).ok() != proof.technology {
                return false;
            }
        }
        true
    }

    pub fn carried(&mut self, state: usize, held: Held) -> Result<Proof, &'static str> {
        let (memory, base, plan) = (self.memory, self.base, self.plan);
        let status = state.checked_add(cstate::BUILDING_STATUS).ok_or("infrastructure map status overflow")?;
        let (queued_record, queued, conversion, bonus) = match held.instance {
            Some(building) => {
                let record = queue(memory, building)?;
                let (queued, conversion) = construction_of(memory, queue_headers(&record)?)?;
                (Some(record), queued, conversion, i32::from(queue_bonus(&record)))
            }
            None => (None, 0, 0, 0),
        };
        let level = i32::from(held.level);
        let aggregate = i32::from(held.level.wrapping_add(queued).wrapping_add(conversion));
        let category = field::<u64>(memory, state, cstate::CATEGORY)?;
        let mut maximum = plan.maximum.min(self.category(category as usize)?.wrapping_add(bonus));
        let (mut fort_read, mut technology_read) = (None, None);
        if held.country {
            maximum = i32::MAX;
        } else {
            if plan.fort {
                let fort = self.fort()?;
                fort_read = Some(fort);
                if fort >= 0 { maximum = fort; }
            }
            if plan.technology {
                let limit = self.technology(held.controller)?;
                technology_read = Some(limit);
                maximum = maximum.min(limit);
            }
        }
        let proof = |value| Proof { queue: queued_record, category, fort: fort_read, technology: technology_read, value };
        if !plan.exclusive.is_empty() {
            let mut blocked = false;
            let active = array(memory, status, status::ACTIVE_BUILDINGS, 8)?;
            for index in 0..active.count {
                let building = active.element(index, 8).and_then(|address| read::<u64>(memory, address)).ok_or("infrastructure map active building unreadable")? as usize;
                authenticate(memory, base, building, cbuilding::VTABLE_RVA)?;
                if plan.exclusive.contains(&field(memory, building, cbuilding::TYPE_ID)?) { blocked = true; }
            }
            if !blocked { blocked = exclusive_queued(memory, plan, status)?; }
            if blocked { return Ok(proof(Infrastructure { building_type: plan.token, level, maximum: 0 })); }
        }
        if plan.state_maximum > 0 {
            let used = if plan.group == NO_GROUP {
                level.wrapping_add(i32::from(queued))
            } else {
                let mut used = grouped(memory, base, status, plan.definition, plan.group)?;
                let provinces = array(memory, state, cstate::PROVINCES, 8)?;
                for index in 0..provinces.count {
                    let province = provinces.element(index, 8).and_then(|address| read::<u64>(memory, address)).ok_or("infrastructure map province pointer unreadable")? as usize;
                    authenticate(memory, base, province, cprovince::VTABLE_RVA)?;
                    used = used.wrapping_add(grouped(memory, base, province + cprovince::BUILDING_STATUS, plan.definition, plan.group)?);
                }
                used
            };
            maximum = maximum.min(aggregate.wrapping_add(plan.state_maximum.wrapping_add(bonus).wrapping_sub(used).max(0)));
        }
        if plan.shares_slots {
            let game = self.game;
            let slots = free_slots(memory, base, game, state, self.countries()?)?;
            maximum = maximum.min(aggregate.wrapping_add(slots));
        }
        Ok(proof(Infrastructure { building_type: plan.token, level, maximum }))
    }
}

pub fn snapshot(memory: &impl Memory, base: usize, game: usize, census: engine_native_memory::walk::Outcome, rows: &mut [cstate::Row]) {
    let rows = &mut rows[..census.len()];
    for row in rows.iter_mut() { row.infrastructure = None; }
    let mut cache = Cache::default();
    let Ok(mut hour) = Hour::new(memory, base, game, &mut cache, false) else { return; };
    for row in rows {
        row.infrastructure = hour.state_of(row.id).and_then(|state| hour.fresh(state)).map(|(_, proof)| proof.value).ok();
    }
}

#[cfg(test)]
mod tests;
