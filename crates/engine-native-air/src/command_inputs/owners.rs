use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{ccountry, cgame_state};
use engine_native_parts::creference_registry::HeaderResolver;
use engine_executable::executable_tables::commands::{Object, Source, Target};
use engine_sites::Rva;
use std::io;

struct Building {
    index: u32,
    ordinal: u32,
    pointer: usize,
    enabled: bool,
}

#[derive(Default)]
pub struct Inputs {
    building_database: usize,
    building_header: [u8; 16],
    buildings: Vec<Building>,
}

impl Inputs {
    pub fn new(memory: &impl Memory, base: usize) -> io::Result<Self> {
        use engine_native_parts::{
            cbuilding_database as database,
            cbuilding_template as template,
        };
        let result = (|| {
            let building_database =
                field::<u64>(memory, global(base, database::SINGLETON_RVA)?, 0)? as usize;
            let building_header =
                field::<[u8; 16]>(memory, building_database, database::TEMPLATES)?;
            let array =
                Array::from_header(building_header, 8).ok_or("command building catalog invalid")?;
            let empty = array
                .element(0, 8)
                .and_then(|at| read::<u64>(memory, at))
                .ok_or("command building sentinel missing")? as usize;
            if Some(field::<u64>(memory, empty, 0)?) != vtable(base, template::NULL_VTABLE_RVA) {
                return Err("command building sentinel identity mismatch");
            }
            let mut buildings = Vec::new();
            buildings
                .try_reserve_exact(array.count as usize)
                .map_err(|_| "command building catalog allocation failed")?;
            for index in 1..array.count {
                let pointer = read::<u64>(
                    memory,
                    array
                        .element(index, 8)
                        .ok_or("command building slot overflow")?,
                )
                .ok_or("command building slot unreadable")? as usize;
                if pointer == 0 {
                    continue;
                }
                if Some(field::<u64>(memory, pointer, 0)?) != vtable(base, template::VTABLE_RVA) {
                    return Err("command building definition identity mismatch");
                }
                buildings.push(Building {
                    index,
                    ordinal: field(memory, pointer, template::DATABASE_ORDINAL)?,
                    pointer,
                    enabled: field::<u8>(memory, pointer, template::LOOKUP_ENABLED_CANDIDATE)? != 0,
                });
            }
            Ok(Self {
                building_database,
                building_header,
                buildings,
            })
        })();
        result.map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Country(u32),
    Automatic,
    Unknown,
}

struct Reader<'a, M, F> {
    memory: &'a M,
    base: usize,
    state: usize,
    inputs: &'a Inputs,
    registry: HeaderResolver,
    object_identity: F,
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .and_then(|at| read(memory, at))
        .ok_or("command owner field unreadable or overflowing")
}

fn global(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("command owner field unreadable or overflowing")
}

fn vtable(base: usize, key: Rva) -> Option<u64> {
    engine_sites::at(base, key).map(|address| address as u64)
}

impl<M: Memory, F: Fn(u32, Object) -> bool> Reader<'_, M, F> {
    fn object(&self, pointer: usize, expected: Object) -> Result<u32, &'static str> {
        let vtable = field::<u64>(self.memory, pointer, 0)?;
        let relative = vtable
            .checked_sub(self.base as u64)
            .and_then(|rva| u32::try_from(rva).ok())
            .ok_or("command owner identity outside module")?;
        if !(self.object_identity)(relative, expected) {
            return Err("command owner object type unsupported");
        }
        Ok(relative)
    }

    fn reference(&mut self, handle: u64, expected: Object) -> Result<Option<usize>, &'static str> {
        let pointer = self.resolve_reference(handle, expected)?;
        if pointer.is_none() && handle != 0 {
            Err("command owner reference absent")
        } else {
            Ok(pointer)
        }
    }

    fn resolve_reference(
        &mut self,
        handle: u64,
        expected: Object,
    ) -> Result<Option<usize>, &'static str> {
        let Some(reference) = self.registry.resolve(self.memory, self.base, handle)? else {
            return Ok(None);
        };
        let pointer = reference
            .checked_sub(expected.secondary())
            .ok_or("command owner base underflow")?;
        self.object(pointer, expected)?;
        if field::<u64>(self.memory, reference, 8)? != handle {
            return Err("command owner reference identity mismatch");
        }
        Ok(Some(pointer))
    }

    fn country_pointer(&self, pointer: usize) -> Result<u32, &'static str> {
        if Some(field::<u64>(self.memory, pointer, 0)?) != vtable(self.base, ccountry::VTABLE_RVA) {
            return Err("command country pointer identity mismatch");
        }
        field(self.memory, pointer, 8)
    }

    fn owner(&mut self, pointer: usize, object: Object) -> Result<Option<u32>, &'static str> {
        use Object::*;
        let value = match object {
            Unit => field(self.memory, pointer, engine_native::cunit::ROSTER_OWNER)?,
            Group => field(self.memory, pointer, engine_native::corders_group::COUNTRY)?,
            Theatre => field(self.memory, pointer, 0x48)?,
            Leader => field(self.memory, pointer, engine_native::cunit_leader::OWNER)?,
            IndustrialOrganisation => field(self.memory, pointer, 0x120)?,
            Character => field(self.memory, pointer, engine_native::ccharacter::COUNTRY)?,
            CountryReference => field(self.memory, pointer, 0x18)?,
            CountryToken => field(self.memory, pointer, 0x1c)?,
            DiplomaticAction | MarketAccessRequest => field(self.memory, pointer, 0x14)?,
            Front | TheatreGroup => {
                let theatre = field::<u64>(self.memory, pointer, 0x18)? as usize;
                self.object(theatre, Theatre)?;
                field(self.memory, theatre, 0x48)?
            }
            Fleet => self.country_pointer(
                field::<u64>(self.memory, pointer, engine_native::cfleet::OWNER)? as usize,
            )?,
            Ace => {
                self.country_pointer(
                    field::<u64>(self.memory, pointer, crate::cace::OWNER_COUNTRY)? as usize,
                )?
            }
            NavalTheatreGroup => {
                self.country_pointer(field::<u64>(self.memory, pointer, 0x18)? as usize)?
            }
            Raid => {
                use crate::ccountry_raid_status as status;
                let owner = field::<u64>(self.memory, pointer, crate::craid_instance::OWNER_OBJECT)?
                    as usize;
                if Some(field::<u64>(self.memory, owner, 0)?)
                    != vtable(self.base, status::VTABLE_RVA)
                {
                    return Err("command raid status identity mismatch");
                }
                let country = field::<i32>(self.memory, owner, status::COUNTRY)?;
                return Ok((country > 0).then_some(country as u32));
            }
            ProductionLine => {
                let status = field::<u64>(
                    self.memory,
                    pointer,
                    engine_native_parts::cproduction_line::PRODUCTION_STATUS,
                )? as usize;
                self.country_pointer(field::<u64>(
                    self.memory,
                    status,
                    engine_native::cproduction_status::COUNTRY,
                )? as usize)?
            }
            DivisionTemplate => field(
                self.memory,
                pointer,
                engine_native::creferenced_division_template::COUNTRY_OPERAND_289A,
            )?,
            Conveyor => {
                let template = field::<u64>(self.memory, pointer, 0x20)? as usize;
                self.object(template, DivisionTemplate)?;
                field(
                    self.memory,
                    template,
                    engine_native::creferenced_division_template::COUNTRY_OPERAND_289A,
                )?
            }
            ConveyorLine => {
                let conveyor = field::<u64>(self.memory, pointer, 0x38)? as usize;
                self.object(conveyor, Conveyor)?;
                return self.owner(conveyor, Conveyor);
            }
            Ship => {
                let unit = field::<u64>(self.memory, pointer, engine_native::cship::TASK_FORCE)? as usize;
                self.object(unit, Unit)?;
                field(self.memory, unit, engine_native::cunit::ROSTER_OWNER)?
            }
            Wing => {
                let direct =
                    field::<i32>(self.memory, pointer, crate::cair_wing::COUNTRY_OVERRIDE)?;
                if direct > 0 {
                    direct as u32
                } else {
                    let pool = field::<u64>(self.memory, pointer, engine_native_parts::cair_wing_layout::POOL)? as usize;
                    self.object(pool, WingPool)?;
                    let strategic =
                        field::<u64>(self.memory, pool, engine_native_parts::cair_wing_pool::OWNER)? as usize;
                    if Some(field::<u64>(self.memory, strategic, 0)?)
                        != vtable(self.base, crate::cstrategic_air::VTABLE_RVA)
                    {
                        return Err("command air country identity mismatch");
                    }
                    field(self.memory, strategic, crate::cstrategic_air::COUNTRY)?
                }
            }
            WingPool => {
                if field::<u64>(self.memory, pointer, 0x68)? != 0 {
                    return Ok(None);
                }
                let handle = field(self.memory, pointer, 0x60)?;
                let Some(ship) = self.reference(handle, Ship)? else {
                    return Ok(None);
                };
                return self.owner(ship, Ship);
            }
            AirGroup => {
                let handle = field(self.memory, pointer, engine_native_parts::cair_group::THEATRE_HANDLE)?;
                let Some(theatre) = self.reference(handle, CountryReference)? else {
                    return Ok(None);
                };
                return self.owner(theatre, CountryReference);
            }
            NavalCombat => return Err("command combat needs an acting side"),
        };
        Ok((value != 0).then_some(value))
    }

    fn referenced_owner(
        &mut self,
        handle: u64,
        object: Object,
    ) -> Result<Option<u32>, &'static str> {
        match self.reference(handle, object)? {
            Some(pointer) => self.owner(pointer, object),
            None => Ok(None),
        }
    }

    fn references(&mut self, at: usize, object: Object) -> Result<Option<u32>, &'static str> {
        let values = Array::read(self.memory, at, 8).ok_or("command owner list invalid")?;
        let countries = ccountry::Reader::new(self.memory, self.base, self.state)?;
        let mut owner = None;
        let mut canonical = None;
        for index in 0..values.count {
            let handle = read(
                self.memory,
                values
                    .element(index, 8)
                    .ok_or("command owner list overflow")?,
            )
            .ok_or("command owner handle unreadable")?;
            let Some(pointer) = self.resolve_reference(handle, object)? else {
                continue;
            };
            if let Some(country) = self.owner(pointer, object)? {
                if owner == Some(country) {
                    continue;
                }
                let slot = countries.resolve(country)?.canonical_slot;
                if canonical.is_some_and(|previous| previous != slot) {
                    return Err("command has different acting countries");
                }
                owner = Some(country);
                canonical = Some(slot);
            }
        }
        Ok(owner)
    }

    fn location(&self, id: u32, province: bool) -> Result<Option<usize>, &'static str> {
        use engine_native::{cprovince, cstate};
        let (array, class, identity) = if province {
            (cgame_state::PROVINCES, cprovince::VTABLE_RVA, cprovince::ID)
        } else {
            (cgame_state::STATES, cstate::VTABLE_RVA, cstate::ID)
        };
        let locations = Array::read(self.memory, self.state + array, 8)
            .ok_or("command location table invalid")?;
        let Some(slot) = locations.element(id, 8) else {
            return Ok(None);
        };
        let pointer = read::<u64>(self.memory, slot).ok_or("command location unreadable")? as usize;
        if pointer == 0 {
            return Ok(None);
        }
        if Some(field::<u64>(self.memory, pointer, 0)?) != vtable(self.base, class)
            || field::<u32>(self.memory, pointer, identity)? != id
        {
            return Err("command location identity mismatch");
        }
        Ok(Some(pointer))
    }

    fn building(&self, index: u32, state: u32) -> Result<Option<u32>, &'static str> {
        use engine_native_parts::{cbuilding, cbuilding_database as database};
        use engine_native::cprovince::buildings as status;
        if field::<u64>(self.memory, global(self.base, database::SINGLETON_RVA)?, 0)? as usize
            != self.inputs.building_database
            || field::<[u8; 16]>(
                self.memory,
                self.inputs.building_database,
                database::TEMPLATES,
            )? != self.inputs.building_header
        {
            return Err("command building definitions changed");
        }
        let index = self
            .inputs
            .buildings
            .binary_search_by_key(&index, |building| building.index)
            .map_err(|_| "command building definition unsupported")?;
        let definition = &self.inputs.buildings[index];
        if !definition.enabled {
            return Ok(None);
        }
        let Some(location) = self.location(state, false)? else {
            return Ok(None);
        };
        let status = location
            .checked_add(engine_native::cstate::BUILDING_STATUS)
            .ok_or("command building status overflow")?;
        if Some(field::<u64>(self.memory, status, 0)?) != vtable(self.base, status::VTABLE_RVA) {
            return Err("command building status identity mismatch");
        }
        let indices = Array::read(self.memory, status + status::BUILDING_INDICES, 4)
            .ok_or("command building indices invalid")?;
        let Some(slot) = indices.element(definition.ordinal, 4) else {
            return Ok(None);
        };
        let instance = read::<i32>(self.memory, slot).ok_or("command building index unreadable")?;
        if instance < 0 {
            return Ok(None);
        }
        let buildings = Array::read(self.memory, status + status::BUILDINGS, 8)
            .ok_or("command building instances invalid")?;
        let slot = buildings
            .element(instance as u32, 8)
            .ok_or("command building index outside instances")?;
        let building =
            read::<u64>(self.memory, slot).ok_or("command building instance unreadable")? as usize;
        if Some(field::<u64>(self.memory, building, 0)?) != vtable(self.base, cbuilding::VTABLE_RVA)
            || field::<u64>(self.memory, building, cbuilding::BUILDING_STATUS)? != status as u64
            || field::<u64>(self.memory, building, cbuilding::TEMPLATE)?
                != definition.pointer as u64
        {
            return Err("command building instance identity mismatch");
        }
        let country = field::<u32>(self.memory, status, status::COUNTRY)?;
        if country != 0 {
            return Ok(Some(country));
        }
        let id = field(self.memory, status, status::ID)?;
        let province =
            field::<u32>(self.memory, status, status::FLAGS)? & status::PROVINCE_FLAG != 0;
        let Some(location) = self.location(id, province)? else {
            return Ok(None);
        };
        field(
            self.memory,
            location,
            if province {
                engine_native::cprovince::CONTROLLER
            } else {
                engine_native::cstate::CONTROLLER
            },
        )
        .map(Some)
    }

    fn naval_side(
        &mut self,
        handle: u64,
        attacker: bool,
        player: i32,
        actor_country: u32,
    ) -> Result<Resolution, &'static str> {
        if actor_country == 0 || player < 0 {
            return Ok(Resolution::Unknown);
        }
        let Some(combat) = self.reference(handle, Object::NavalCombat)? else {
            return Ok(Resolution::Unknown);
        };
        let side = field::<u64>(self.memory, combat, if attacker { 0x98 } else { 0xa0 })? as usize;
        engine_native::cnaval_combatant::side(self.memory, self.base, side, combat, false)?;
        let countries = ccountry::Reader::new(self.memory, self.base, self.state)?;
        let slots = field::<u32>(self.memory, self.state, cgame_state::COUNTRIES_COUNT)?;
        let count = field::<u32>(
            self.memory,
            side,
            engine_native_parts::naval_side_layout::COUNTRY_COUNT,
        )?;
        if count > slots {
            return Err("command combat country count invalid");
        }
        let actor_slot = (actor_country != 0)
            .then(|| countries.resolve(actor_country))
            .transpose()?
            .map(|country| country.canonical_slot);
        let mut belongs = false;
        let mut automatic = false;
        let mut node = field::<u64>(
            self.memory,
            side,
            engine_native_parts::naval_side_layout::COUNTRIES,
        )? as usize;
        for _ in 0..count {
            let country = field::<u32>(self.memory, node, 0)?;
            let slot = countries.resolve(country)?.canonical_slot;
            belongs |= actor_slot == Some(slot);
            automatic |= super::autonomous(self.memory, self.base, self.state, country)?;
            node = field::<u64>(self.memory, node, 16)? as usize;
        }
        if node != 0 {
            return Err("command combat countries exceed native count");
        }
        Ok(if !belongs {
            Resolution::Automatic
        } else if player == 1 && automatic {
            Resolution::Unknown
        } else {
            Resolution::Country(actor_country)
        })
    }

    fn source(
        &mut self,
        command: usize,
        source: Source,
        player: i32,
        actor_country: u32,
    ) -> Result<Resolution, &'static str> {
        let country = match source {
            Source::Country(offset) => Some(field(self.memory, command, offset as usize)?),
            Source::Reference(offset, object) => {
                self.referenced_owner(field(self.memory, command, offset as usize)?, object)?
            }
            Source::References(offset, object) => self.references(
                command
                    .checked_add(offset as usize)
                    .ok_or("command owner list overflow")?,
                object,
            )?,
            Source::Pointer(offset, object) => {
                let pointer = field::<u64>(self.memory, command, offset as usize)? as usize;
                if pointer == 0 {
                    None
                } else {
                    let identity = self.object(pointer, object)?;
                    if object == Object::DiplomaticAction
                        && (self.object_identity)(identity, Object::MarketAccessRequest)
                        && field::<u8>(self.memory, pointer, 0x78)? == 0
                    {
                        return Ok(Resolution::Automatic);
                    }
                    self.owner(pointer, object)?
                }
            }
            Source::Province(offset) => {
                let id = field(self.memory, command, offset as usize)?;
                self.location(id, true)?
                    .map(|province| field(self.memory, province, engine_native::cprovince::CONTROLLER))
                    .transpose()?
            }
            Source::Building { definition, state } => self.building(
                field(self.memory, command, definition as usize)?,
                field(self.memory, command, state as usize)?,
            )?,
            Source::NavalSide { combat, side } => {
                return self.naval_side(
                    field(self.memory, command, combat as usize)?,
                    field::<u8>(self.memory, command, side as usize)? != 0,
                    player,
                    actor_country,
                );
            }
            Source::ScriptCountry { pointer, length } => {
                let count = field::<i32>(self.memory, command, length as usize)?;
                if count < 0 {
                    return Err("command script country list invalid");
                }
                if count == 0 {
                    Some(actor_country)
                } else {
                    let country = field(
                        self.memory,
                        field::<u64>(self.memory, command, pointer as usize)? as usize,
                        0,
                    )?;
                    Some(if country == 0 { actor_country } else { country })
                }
            }
        };
        Ok(country
            .filter(|&country| country != 0)
            .map_or(Resolution::Unknown, Resolution::Country))
    }

    fn target(
        &mut self,
        command: usize,
        target: Target,
        player: i32,
        actor_country: u32,
    ) -> Result<Resolution, &'static str> {
        let source = match target {
            Target::Unknown => return Ok(Resolution::Unknown),
            Target::Country(offset) => Source::Country(offset),
            Target::Group(offset) => Source::Reference(offset, Object::Group),
            Target::Unit(offset) => Source::Reference(offset, Object::Unit),
            Target::Units(offset) => Source::References(offset, Object::Unit),
            Target::Wings(offset) => Source::References(offset, Object::Wing),
            Target::Sources(sources) => {
                for &source in sources {
                    let result = self.source(command, source, player, actor_country)?;
                    if result != Resolution::Unknown {
                        return Ok(result);
                    }
                }
                return Ok(Resolution::Unknown);
            }
        };
        self.source(command, source, player, actor_country)
    }
}

pub fn target(
    memory: &impl Memory,
    base: usize,
    state: usize,
    inputs: &Inputs,
    command: usize,
    target: Target,
    player: i32,
    actor_country: u32,
) -> Result<Resolution, &'static str> {
    let mut reader = Reader {
        memory,
        base,
        state,
        inputs,
        registry: HeaderResolver::default(),
        object_identity: |identity, object| {
            engine_executable::executable_tables::commands()
                .is_ok_and(|commands| commands.object(identity, object))
        },
    };
    reader.target(command, target, player, actor_country)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::{corders_group, ctheatre};
    use crate::craid_instance;
    use engine_native_parts::ctheater_group;
    use std::collections::BTreeMap;

    const BASE: usize = 0x1_0000_0000;
    const STATE: usize = 0x1000;
    const COMMAND: usize = 0x2000;
    const PRODUCTION_LINE_IDENTITY: usize = 0x100;

    #[derive(Default)]
    struct Bytes(BTreeMap<usize, u8>);

    impl Bytes {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.0.extend(
                bytes
                    .iter()
                    .enumerate()
                    .map(|(offset, &byte)| (at + offset, byte)),
            );
        }

        fn u32(&mut self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }

        fn u64(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }

        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.u64(at, data as u64);
            self.u32(at + 8, count);
            self.u32(at + 12, count);
        }

        fn references(&mut self, table: usize, references: &[(u64, usize)]) {
            use engine_native_parts::{
                creference_registry as registry,
                reference_bucket as bucket,
            };
            let kind = references[0].0 as u32;
            assert!(kind < registry::LOW_ROOT_COUNT);
            let entries = table + 0x100;
            self.u64(
                BASE + registry::LOW_ROOTS_RVA.0 + kind as usize * 8,
                table as u64,
            );
            self.u64(table + registry::ENTRIES, entries as u64);
            self.u32(table + registry::COUNT, references.len() as u32);
            self.u32(table + registry::MASK, 7);
            self.put(table + registry::OVERFLOW_DISTANCE, &[5]);
            self.put(entries, &[0; 14 * bucket::STRIDE]);
            self.put(entries + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
            for &(handle, pointer) in references {
                assert_eq!(handle as u32, kind);
                let hash = handle
                    .rotate_left(32)
                    .wrapping_mul(registry::HASH_MULTIPLIER);
                let index = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
                let at = entries + index * bucket::STRIDE;
                assert_eq!(self.0[&(at + bucket::DISTANCE)], 0);
                self.put(at + bucket::DISTANCE, &[1]);
                self.u64(at + bucket::TYPE, handle);
                self.u64(at + bucket::OBJECT, pointer as u64);
            }
        }
    }

    impl Memory for Bytes {
        fn copy(&self, at: usize, destination: &mut [u8]) -> bool {
            for (offset, byte) in destination.iter_mut().enumerate() {
                let Some(value) = self.0.get(&(at + offset)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    fn reader<'a>(
        memory: &'a Bytes,
        inputs: &'a Inputs,
    ) -> Reader<'a, Bytes, impl Fn(u32, Object) -> bool> {
        Reader {
            memory,
            base: BASE,
            state: STATE,
            inputs,
            registry: HeaderResolver::default(),
            object_identity: |identity, object| {
                let expected = match object {
                    Object::Group => corders_group::VTABLE_RVA.0,
                    Object::Theatre => ctheatre::VTABLE_RVA.0,
                    Object::TheatreGroup => ctheater_group::VTABLE_RVA.0,
                    Object::Raid => craid_instance::VTABLE_RVA.0,
                    Object::ProductionLine => PRODUCTION_LINE_IDENTITY,
                    _ => return false,
                };
                identity as usize == expected
            },
        }
    }

    #[test]
    fn null_theatre_groups_fall_back_to_orders_groups_and_present_groups_take_precedence() {
        let inputs = Inputs::default();
        let target = Target::Sources(&[
            Source::Reference(0x28, Object::TheatreGroup),
            Source::References(0x30, Object::Group),
        ]);
        let resolve = |memory: &Bytes| reader(memory, &inputs).target(COMMAND, target, 1, 1);
        let mut memory = Bytes::default();
        let groups = [
            (1u64 << 32) | u64::from(corders_group::REFERENCE_TYPE),
            (2u64 << 32) | u64::from(corders_group::REFERENCE_TYPE),
        ];
        memory.u64(COMMAND + 0x28, 0);
        memory.array(COMMAND + 0x30, 0x3000, 2);
        for (index, handle) in groups.into_iter().enumerate() {
            let pointer = 0x4000 + index * 0x1000;
            memory.u64(0x3000 + index * 8, handle);
            memory.u64(pointer, (BASE + corders_group::VTABLE_RVA.0) as u64);
            memory.u64(pointer + corders_group::HANDLE, handle);
            memory.u32(pointer + corders_group::COUNTRY, 1);
        }
        memory.references(0x1_0000, &[(groups[0], 0x4000), (groups[1], 0x5000)]);
        memory.array(STATE + cgame_state::COUNTRIES, 0x8000, 4);
        memory.array(STATE + cgame_state::COUNTRY_LINKS, 0x9000, 4);
        memory.u64(0x8000 + 8, 0xa000);
        memory.u64(0x8000 + 24, 0xb000);
        for (raw, slot) in [(1, 1), (2, 1), (3, 3)] {
            memory.u32(0x9000 + raw * 4, slot);
        }
        for (pointer, country) in [(0xa000, 1), (0xb000, 3)] {
            memory.u64(pointer, (BASE + ccountry::VTABLE_RVA.0) as u64);
            memory.u32(pointer + ccountry::ID, country);
        }
        assert_eq!(resolve(&memory), Ok(Resolution::Country(1)));

        memory.u32(0x5000 + corders_group::COUNTRY, 2);
        assert_eq!(resolve(&memory), Ok(Resolution::Country(2)));
        memory.u32(0x5000 + corders_group::COUNTRY, 3);
        assert_eq!(
            resolve(&memory),
            Err("command has different acting countries")
        );

        let theatre_group = (1u64 << 32) | u64::from(ctheater_group::REFERENCE_TYPE);
        memory.references(0x1_1000, &[(theatre_group, 0x6000)]);
        memory.u64(COMMAND + 0x28, theatre_group);
        memory.u64(0x6000, (BASE + ctheater_group::VTABLE_RVA.0) as u64);
        memory.u64(0x6000 + ctheater_group::HANDLE, theatre_group);
        memory.u64(0x6000 + ctheater_group::THEATRE, 0x7000);
        memory.u64(0x7000, (BASE + ctheatre::VTABLE_RVA.0) as u64);
        memory.u32(0x7000 + ctheatre::OWNER, 3);
        assert_eq!(resolve(&memory), Ok(Resolution::Country(3)));

        memory.u64(COMMAND + 0x28, 0);
        memory.array(COMMAND + 0x30, 0, 0);
        assert_eq!(resolve(&memory), Ok(Resolution::Unknown));
    }

    #[test]
    fn production_line_lists_skip_absent_references_and_validate_present_objects() {
        use engine_native_parts::cproduction_line as line;
        use engine_native::cproduction_status as status;
        let inputs = Inputs::default();
        let target = Target::Sources(&[Source::References(0x28, Object::ProductionLine)]);
        let resolve = |memory: &Bytes| reader(memory, &inputs).target(COMMAND, target, 1, 1);
        let mut memory = Bytes::default();
        let handles = [(1u64 << 32) | 1, (2u64 << 32) | 1, (3u64 << 32) | 1];
        memory.references(0x1_0000, &[(handles[1], 0x4000)]);
        memory.array(COMMAND + 0x28, 0x3000, 2);
        memory.u64(0x3000, handles[0]);
        memory.u64(0x3008, handles[1]);
        memory.u64(0x4000, (BASE + PRODUCTION_LINE_IDENTITY) as u64);
        memory.u64(0x4000 + line::REFERENCE_TYPE, handles[1]);
        memory.u64(0x4000 + line::PRODUCTION_STATUS, 0x5000);
        memory.u64(0x5000 + status::COUNTRY, 0x6000);
        memory.u64(0x6000, (BASE + ccountry::VTABLE_RVA.0) as u64);
        memory.u32(0x6000 + ccountry::ID, 1);
        memory.array(STATE + cgame_state::COUNTRIES, 0x8000, 2);
        memory.array(STATE + cgame_state::COUNTRY_LINKS, 0x9000, 2);
        memory.u64(0x8008, 0x6000);
        memory.u32(0x9004, 1);
        assert_eq!(resolve(&memory), Ok(Resolution::Country(1)));

        memory.u64(0x3008, handles[2]);
        assert_eq!(resolve(&memory), Ok(Resolution::Unknown));

        memory.u64(COMMAND + 0x40, handles[0]);
        assert_eq!(
            reader(&memory, &inputs).source(
                COMMAND,
                Source::Reference(0x40, Object::ProductionLine),
                1,
                1,
            ),
            Err("command owner reference absent")
        );

        memory.u64(0x3008, handles[1]);
        memory.u64(0x4000, (BASE + corders_group::VTABLE_RVA.0) as u64);
        assert_eq!(
            resolve(&memory),
            Err("command owner object type unsupported")
        );

        memory.u64(0x4000, (BASE + PRODUCTION_LINE_IDENTITY) as u64);
        memory.u64(0x4000 + line::REFERENCE_TYPE, handles[0]);
        assert_eq!(
            resolve(&memory),
            Err("command owner reference identity mismatch")
        );
    }

    #[test]
    fn raid_owners_require_a_raid_status_and_a_positive_signed_country() {
        use crate::ccountry_raid_status as status;
        let inputs = Inputs::default();
        let target = Target::Sources(&[Source::Reference(0x28, Object::Raid)]);
        let resolve = |memory: &Bytes| reader(memory, &inputs).target(COMMAND, target, 1, 1);
        let mut memory = Bytes::default();
        let handle = (1u64 << 32) | u64::from(craid_instance::REFERENCE_TYPE_VALUE);
        memory.references(0x1_0000, &[(handle, 0x4000)]);
        memory.u64(COMMAND + 0x28, handle);
        memory.u64(0x4000, (BASE + craid_instance::VTABLE_RVA.0) as u64);
        memory.u64(0x4000 + craid_instance::REFERENCE_TYPE, handle);
        memory.u64(0x4000 + craid_instance::OWNER_OBJECT, 0x5000);
        memory.u64(0x5000, (BASE + status::VTABLE_RVA.0) as u64);
        memory.u32(0x5000 + status::COUNTRY, 7);
        assert_eq!(resolve(&memory), Ok(Resolution::Country(7)));

        memory.u64(0x5000, (BASE + ccountry::VTABLE_RVA.0) as u64);
        assert_eq!(
            resolve(&memory),
            Err("command raid status identity mismatch")
        );
        memory.u64(0x5000, (BASE + status::VTABLE_RVA.0) as u64);
        for country in [0, -1, i32::MIN] {
            memory.put(0x5000 + status::COUNTRY, &country.to_le_bytes());
            assert_eq!(
                reader(&memory, &inputs).owner(0x4000, Object::Raid),
                Ok(None)
            );
            assert_eq!(resolve(&memory), Ok(Resolution::Unknown));
        }
    }

    #[test]
    fn building_owners_use_definition_slots_and_their_independent_native_ordinals() {
        use engine_native_parts::{
            cbuilding_database as database,
            cbuilding_template as template,
        };
        let base = 0x1_0000_0000usize;
        let mut memory = Bytes::default();
        memory.put(base + database::SINGLETON_RVA.0, &0x1000u64.to_le_bytes());
        let mut header = Vec::new();
        header.extend_from_slice(&0x2000u64.to_le_bytes());
        header.extend_from_slice(&3u32.to_le_bytes());
        header.extend_from_slice(&3u32.to_le_bytes());
        memory.put(0x1000 + database::TEMPLATES, &header);
        for (slot, pointer) in [(0, 0x3000u64), (1, 0x4000), (2, 0x5000)] {
            memory.put(0x2000 + slot * 8, &pointer.to_le_bytes());
        }
        memory.put(
            0x3000,
            &((base + template::NULL_VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        for (pointer, token, ordinal, enabled) in
            [(0x4000, u32::MAX, 73u32, 0u8), (0x5000, 37, 5, 0x19)]
        {
            memory.put(
                pointer,
                &((base + template::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            memory.put(pointer + template::TYPE_ID, &token.to_le_bytes());
            memory.put(pointer + template::DATABASE_ORDINAL, &ordinal.to_le_bytes());
            memory.put(pointer + template::LOOKUP_ENABLED_CANDIDATE, &[enabled]);
        }
        let inputs = Inputs::new(&memory, base).unwrap();
        assert_eq!(inputs.buildings.len(), 2);
        assert_eq!(
            (
                inputs.buildings[0].index,
                inputs.buildings[0].ordinal,
                inputs.buildings[0].pointer
            ),
            (1, 73, 0x4000)
        );
        assert!(!inputs.buildings[0].enabled);
        assert_eq!(inputs.buildings[1].index, 2);
        assert_eq!(inputs.buildings[1].ordinal, 5);
        assert!(inputs.buildings[1].enabled);
        let mut header = Vec::new();
        header.extend_from_slice(&0x7000u64.to_le_bytes());
        header.extend_from_slice(&9u32.to_le_bytes());
        header.extend_from_slice(&9u32.to_le_bytes());
        memory.put(0x6000 + cgame_state::STATES, &header);
        memory.put(0x7000 + 8 * 8, &0x8000u64.to_le_bytes());
        memory.put(
            0x8000,
            &((base + engine_native::cstate::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        memory.put(0x8000 + engine_native::cstate::ID, &8u32.to_le_bytes());
        let status = 0x8000 + engine_native::cstate::BUILDING_STATUS;
        memory.put(
            status,
            &((base + engine_native::cprovince::buildings::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        header.clear();
        header.extend_from_slice(&0x9000u64.to_le_bytes());
        header.extend_from_slice(&6u32.to_le_bytes());
        header.extend_from_slice(&6u32.to_le_bytes());
        memory.put(status + engine_native::cprovince::buildings::BUILDING_INDICES, &header);
        memory.put(0x9000 + 5 * 4, &0i32.to_le_bytes());
        header.clear();
        header.extend_from_slice(&0xa000u64.to_le_bytes());
        header.extend_from_slice(&1u32.to_le_bytes());
        header.extend_from_slice(&1u32.to_le_bytes());
        memory.put(status + engine_native::cprovince::buildings::BUILDINGS, &header);
        memory.put(0xa000, &0xb000u64.to_le_bytes());
        memory.put(
            0xb000,
            &((base + engine_native_parts::cbuilding::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        memory.put(
            0xb000 + engine_native_parts::cbuilding::BUILDING_STATUS,
            &(status as u64).to_le_bytes(),
        );
        memory.put(
            0xb000 + engine_native_parts::cbuilding::TEMPLATE,
            &0x5000u64.to_le_bytes(),
        );
        memory.put(
            status + engine_native::cprovince::buildings::COUNTRY,
            &7u32.to_le_bytes(),
        );
        let reader = Reader {
            memory: &memory,
            base,
            state: 0x6000,
            inputs: &inputs,
            registry: HeaderResolver::default(),
            object_identity: |_, _| false,
        };
        assert_eq!(reader.building(2, 8).unwrap(), Some(7));
        assert_eq!(reader.building(1, 8).unwrap(), None);
        assert!(reader.building(37, 8).is_err());
        memory.put(0x3000, &0u64.to_le_bytes());
        assert!(Inputs::new(&memory, base).is_err());
    }
}
