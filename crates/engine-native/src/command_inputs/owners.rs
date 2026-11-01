use crate::{
    Memory, ccountry, cgame_state, cpdx_array::Array, creference_registry::HeaderResolver, read,
};
use engine_executable::executable_tables::commands::{Object, Source, Target};
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
        use crate::{cbuilding_database as database, cbuilding_template as template};
        let result = (|| {
            let building_database = field::<u64>(memory, base, database::SINGLETON_RVA)? as usize;
            let building_header =
                field::<[u8; 16]>(memory, building_database, database::TEMPLATES)?;
            let array =
                Array::from_header(building_header, 8).ok_or("command building catalog invalid")?;
            let empty = array
                .element(0, 8)
                .and_then(|at| read::<u64>(memory, at))
                .ok_or("command building sentinel missing")? as usize;
            if field::<u64>(memory, empty, 0)? != (base + template::NULL_VTABLE_RVA) as u64 {
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
                if field::<u64>(memory, pointer, 0)? != (base + template::VTABLE_RVA) as u64 {
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

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
    state: usize,
    inputs: &'a Inputs,
    registry: HeaderResolver,
}

fn field<T: crate::memory::Raw>(
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

impl<M: Memory> Reader<'_, M> {
    fn object(&self, pointer: usize, expected: Object) -> Result<u32, &'static str> {
        let vtable = field::<u64>(self.memory, pointer, 0)?;
        let relative = vtable
            .checked_sub(self.base as u64)
            .and_then(|rva| u32::try_from(rva).ok())
            .ok_or("command owner identity outside module")?;
        if !engine_executable::executable_tables::get()
            .commands
            .object(relative, expected)
        {
            return Err("command owner object type unsupported");
        }
        Ok(relative)
    }

    fn reference(&mut self, handle: u64, expected: Object) -> Result<Option<usize>, &'static str> {
        if handle == 0 {
            return Ok(None);
        }
        let reference = self
            .registry
            .resolve(self.memory, self.base, handle)?
            .ok_or("command owner reference absent")?;
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
        if field::<u64>(self.memory, pointer, 0)? != (self.base + ccountry::VTABLE_RVA) as u64 {
            return Err("command country pointer identity mismatch");
        }
        field(self.memory, pointer, 8)
    }

    fn owner(&mut self, pointer: usize, object: Object) -> Result<Option<u32>, &'static str> {
        use Object::*;
        let value = match object {
            Unit => field(self.memory, pointer, crate::cunit::ROSTER_OWNER)?,
            Group => field(self.memory, pointer, crate::corders_group::COUNTRY)?,
            Theatre => field(self.memory, pointer, 0x48)?,
            Leader => field(self.memory, pointer, crate::cunit_leader::OWNER)?,
            IndustrialOrganisation => field(self.memory, pointer, 0x120)?,
            Character => field(self.memory, pointer, crate::ccharacter::COUNTRY)?,
            CountryReference => field(self.memory, pointer, 0x18)?,
            CountryToken => field(self.memory, pointer, 0x1c)?,
            DiplomaticAction | MarketAccessRequest => field(self.memory, pointer, 0x14)?,
            Front | TheatreGroup => {
                let theatre = field::<u64>(self.memory, pointer, 0x18)? as usize;
                self.object(theatre, Theatre)?;
                field(self.memory, theatre, 0x48)?
            }
            Fleet => self.country_pointer(
                field::<u64>(self.memory, pointer, crate::cfleet::OWNER)? as usize,
            )?,
            Ace => {
                self.country_pointer(
                    field::<u64>(self.memory, pointer, crate::cace::OWNER_COUNTRY)? as usize,
                )?
            }
            NavalTheatreGroup => {
                self.country_pointer(field::<u64>(self.memory, pointer, 0x18)? as usize)?
            }
            Raid => self.country_pointer(field::<u64>(
                self.memory,
                pointer,
                crate::craid_instance::OWNER_OBJECT,
            )? as usize)?,
            ProductionLine => {
                let status = field::<u64>(
                    self.memory,
                    pointer,
                    crate::cproduction_line::PRODUCTION_STATUS,
                )? as usize;
                self.country_pointer(field::<u64>(
                    self.memory,
                    status,
                    crate::cproduction_status::COUNTRY,
                )? as usize)?
            }
            DivisionTemplate => field(
                self.memory,
                pointer,
                crate::creferenced_division_template::COUNTRY_OPERAND_289A,
            )?,
            Conveyor => {
                let template = field::<u64>(self.memory, pointer, 0x20)? as usize;
                self.object(template, DivisionTemplate)?;
                field(
                    self.memory,
                    template,
                    crate::creferenced_division_template::COUNTRY_OPERAND_289A,
                )?
            }
            ConveyorLine => {
                let conveyor = field::<u64>(self.memory, pointer, 0x38)? as usize;
                self.object(conveyor, Conveyor)?;
                return self.owner(conveyor, Conveyor);
            }
            Ship => {
                let unit = field::<u64>(self.memory, pointer, crate::cship::TASK_FORCE)? as usize;
                self.object(unit, Unit)?;
                field(self.memory, unit, crate::cunit::ROSTER_OWNER)?
            }
            Wing => {
                let direct =
                    field::<i32>(self.memory, pointer, crate::cair_wing::COUNTRY_OVERRIDE)?;
                if direct > 0 {
                    direct as u32
                } else {
                    let pool = field::<u64>(self.memory, pointer, crate::cair_wing::POOL)? as usize;
                    self.object(pool, WingPool)?;
                    let strategic =
                        field::<u64>(self.memory, pool, crate::cair_wing_pool::OWNER)? as usize;
                    if field::<u64>(self.memory, strategic, 0)?
                        != (self.base + crate::cstrategic_air::VTABLE_RVA) as u64
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
                let handle = field(self.memory, pointer, crate::cair_group::THEATRE_HANDLE)?;
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
            if let Some(country) = self.referenced_owner(handle, object)? {
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
        use crate::{cprovince, cstate};
        let (array, vtable, identity) = if province {
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
        if field::<u64>(self.memory, pointer, 0)? != (self.base + vtable) as u64
            || field::<u32>(self.memory, pointer, identity)? != id
        {
            return Err("command location identity mismatch");
        }
        Ok(Some(pointer))
    }

    fn building(&self, index: u32, state: u32) -> Result<Option<u32>, &'static str> {
        use crate::{cbuilding, cbuilding_database as database, cbuilding_status as status};
        if field::<u64>(self.memory, self.base, database::SINGLETON_RVA)? as usize
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
            .checked_add(crate::cstate::BUILDING_STATUS)
            .ok_or("command building status overflow")?;
        if field::<u64>(self.memory, status, 0)? != (self.base + status::VTABLE_RVA) as u64 {
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
        if field::<u64>(self.memory, building, 0)? != (self.base + cbuilding::VTABLE_RVA) as u64
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
                crate::cprovince::CONTROLLER
            } else {
                crate::cstate::CONTROLLER
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
        crate::cnaval_combatant::side(self.memory, self.base, side, combat, false)?;
        let countries = ccountry::Reader::new(self.memory, self.base, self.state)?;
        let slots = field::<u32>(self.memory, self.state, cgame_state::COUNTRIES_COUNT)?;
        let count = field::<u32>(
            self.memory,
            side,
            crate::naval_combat_summary_inputs::COUNTRY_COUNT,
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
            crate::naval_combat_summary_inputs::COUNTRIES,
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
        // The host's human and AI retreat requests have the same replicated payload.
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
                        && engine_executable::executable_tables::get()
                            .commands
                            .object(identity, Object::MarketAccessRequest)
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
                    .map(|province| field(self.memory, province, crate::cprovince::CONTROLLER))
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
                // Scripted window effects originate in the UI; global scopes have no country operand.
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
    };
    let source = match target {
        Target::Unknown => return Ok(Resolution::Unknown),
        Target::Country(offset) => Source::Country(offset),
        Target::Group(offset) => Source::Reference(offset, Object::Group),
        Target::Unit(offset) => Source::Reference(offset, Object::Unit),
        Target::Units(offset) => Source::References(offset, Object::Unit),
        Target::Wings(offset) => Source::References(offset, Object::Wing),
        Target::Sources(sources) => {
            for &source in sources {
                let result = reader.source(command, source, player, actor_country)?;
                if result != Resolution::Unknown {
                    return Ok(result);
                }
            }
            return Ok(Resolution::Unknown);
        }
    };
    reader.source(command, source, player, actor_country)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

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

    #[test]
    fn building_owners_use_definition_slots_and_their_independent_native_ordinals() {
        use crate::{cbuilding_database as database, cbuilding_template as template};
        let base = 0x1_0000_0000usize;
        let mut memory = Bytes::default();
        memory.put(base + database::SINGLETON_RVA, &0x1000u64.to_le_bytes());
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
            &((base + template::NULL_VTABLE_RVA) as u64).to_le_bytes(),
        );
        for (pointer, token, ordinal, enabled) in
            [(0x4000, u32::MAX, 73u32, 0u8), (0x5000, 37, 5, 0x19)]
        {
            memory.put(
                pointer,
                &((base + template::VTABLE_RVA) as u64).to_le_bytes(),
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
            &((base + crate::cstate::VTABLE_RVA) as u64).to_le_bytes(),
        );
        memory.put(0x8000 + crate::cstate::ID, &8u32.to_le_bytes());
        let status = 0x8000 + crate::cstate::BUILDING_STATUS;
        memory.put(
            status,
            &((base + crate::cbuilding_status::VTABLE_RVA) as u64).to_le_bytes(),
        );
        header.clear();
        header.extend_from_slice(&0x9000u64.to_le_bytes());
        header.extend_from_slice(&6u32.to_le_bytes());
        header.extend_from_slice(&6u32.to_le_bytes());
        memory.put(status + crate::cbuilding_status::BUILDING_INDICES, &header);
        memory.put(0x9000 + 5 * 4, &0i32.to_le_bytes());
        header.clear();
        header.extend_from_slice(&0xa000u64.to_le_bytes());
        header.extend_from_slice(&1u32.to_le_bytes());
        header.extend_from_slice(&1u32.to_le_bytes());
        memory.put(status + crate::cbuilding_status::BUILDINGS, &header);
        memory.put(0xa000, &0xb000u64.to_le_bytes());
        memory.put(
            0xb000,
            &((base + crate::cbuilding::VTABLE_RVA) as u64).to_le_bytes(),
        );
        memory.put(
            0xb000 + crate::cbuilding::BUILDING_STATUS,
            &(status as u64).to_le_bytes(),
        );
        memory.put(
            0xb000 + crate::cbuilding::TEMPLATE,
            &0x5000u64.to_le_bytes(),
        );
        memory.put(
            status + crate::cbuilding_status::COUNTRY,
            &7u32.to_le_bytes(),
        );
        let reader = Reader {
            memory: &memory,
            base,
            state: 0x6000,
            inputs: &inputs,
            registry: HeaderResolver::default(),
        };
        assert_eq!(reader.building(2, 8).unwrap(), Some(7));
        assert_eq!(reader.building(1, 8).unwrap(), None);
        assert!(reader.building(37, 8).is_err());
        memory.put(0x3000, &0u64.to_le_bytes());
        assert!(Inputs::new(&memory, base).is_err());
    }
}
