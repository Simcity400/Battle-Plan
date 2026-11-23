use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::{ccountry, cstate};
use engine_native_parts::{creference_registry, cwar_goal_type};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a5_9250);
pub const HANDLE: usize = 8;
pub const ACTOR: usize = 0x38;
pub const RECIPIENT: usize = 0x3c;
pub const TYPE: usize = 0x40;
pub const STATES: usize = 0x48;
pub const COUNTRIES: usize = 0x60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub owner: u32,
    pub relation_ordinal: u32,
    pub side: u8,
    pub ordinal: u32,
    pub handle: u64,
    pub resolved: u8,
    pub type_token: u32,
    pub actor: u32,
    pub recipient: u32,
    pub state_count: u32,
    pub first_state: u32,
    pub country_count: u32,
    pub first_country: u32,
}

pub fn read_goal(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    mut row: Row,
) -> Result<Row, &'static str> {
    let Some(pointer) = creference_registry::resolve(memory, base, row.handle)? else {
        return Ok(row);
    };
    if read::<u64>(memory, pointer) != engine_sites::at(base, VTABLE_RVA).map(|p| p as u64) {
        return Err("war goal vtable mismatch");
    }
    let field = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("war goal address overflow")
    };
    if read::<u64>(memory, field(HANDLE)?) != Some(row.handle) {
        return Err("war goal handle mismatch");
    }
    row.actor = read(memory, field(ACTOR)?).ok_or("war goal actor unreadable")?;
    row.recipient = read(memory, field(RECIPIENT)?).ok_or("war goal recipient unreadable")?;
    countries.resolve(row.actor)?;
    countries.resolve(row.recipient)?;
    let definition =
        read::<u64>(memory, field(TYPE)?).ok_or("war goal type pointer unreadable")? as usize;
    row.type_token = cwar_goal_type::token(memory, base, definition)?;
    let states = Array::read(memory, field(STATES)?, 8)
        .ok_or("war goal state array invalid or unreadable")?;
    row.state_count = states.count;
    if states.count != 0 {
        let state = read::<u64>(memory, states.data)
            .ok_or("war goal first state pointer unreadable")? as usize;
        if state == 0
            || read::<u64>(memory, state)
                != engine_sites::at(base, cstate::VTABLE_RVA).map(|p| p as u64)
        {
            return Err("war goal state vtable mismatch");
        }
        row.first_state = state
            .checked_add(cstate::ID)
            .and_then(|p| read(memory, p))
            .ok_or("war goal first state id unreadable")?;
    }
    let targets = Array::read(memory, field(COUNTRIES)?, 4)
        .ok_or("war goal country array invalid or unreadable")?;
    row.country_count = targets.count;
    if targets.count != 0 {
        row.first_country =
            read(memory, targets.data).ok_or("war goal first country unreadable")?;
        countries.resolve(row.first_country)?;
    }
    row.resolved = 1;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccountry::synthetic::World;

    #[test]
    fn null_goal_is_not_an_empty_resolved_goal() {
        let fixture = World::new(0, 3, 0x1600);
        let memory = fixture.memory();
        let countries = ccountry::Reader::new(&memory, 0, fixture.game_state()).unwrap();
        let row = Row {
            ordinal: 7,
            ..Default::default()
        };
        assert_eq!(read_goal(&memory, 0, &countries, row).unwrap(), row);
    }

    #[test]
    fn resolved_goal_authenticates_registry_type_state_and_country_operands() {
        use engine_native_parts::{
            creference_registry as registry,
            reference_bucket as bucket,
        };
        let mut world = World::new(0, 2, 0x1600);
        let goal = world.alloc(0x90);
        let definition = world.alloc(0x10);
        let state = world.alloc(0x100);
        let state_list = world.alloc(8);
        let country_list = world.alloc(4);
        let table = world.alloc(registry::SIZE);
        let entries = world.alloc(14 * bucket::STRIDE);
        let handle = (41_u64 << 32) | 12_345;
        world.u64(goal, VTABLE_RVA.0 as u64);
        world.u64(goal + HANDLE, handle);
        world.u32(goal + ACTOR, 1);
        world.u32(goal + RECIPIENT, 2);
        world.u64(goal + TYPE, definition as u64);
        world.u64(definition, cwar_goal_type::VTABLE_RVA.0 as u64);
        world.u32(definition + cwar_goal_type::TOKEN, 42);
        world.u64(state, cstate::VTABLE_RVA.0 as u64);
        world.u32(state + cstate::ID, 91);
        world.u64(state_list, state as u64);
        world.u32(country_list, 2);
        world.array(goal + STATES, state_list, 2);
        world.array(goal + COUNTRIES, country_list, 3);
        world.u64(table + registry::ENTRIES, entries as u64);
        world.u32(table + registry::COUNT, 1);
        world.u32(table + registry::MASK, 7);
        world.put(table + registry::OVERFLOW_DISTANCE, &[5]);
        world.put(entries + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let index = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let entry = entries + index * bucket::STRIDE;
        world.put(entry + bucket::DISTANCE, &[1]);
        world.u64(entry + bucket::TYPE, handle);
        world.u64(entry + bucket::OBJECT, goal as u64);
        struct MemoryWithRoot<'a> {
            world: &'a World,
            table: usize,
        }
        impl engine_native_memory::Memory for MemoryWithRoot<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address == registry::HIGH_ROOT_RVA.0 && destination.len() == 8 {
                    destination.copy_from_slice(&(self.table as u64).to_le_bytes());
                    true
                } else {
                    self.world.memory().copy(address, destination)
                }
            }
        }
        let read = |world: &World| {
            let memory = MemoryWithRoot { world, table };
            let countries = ccountry::Reader::new(&memory, 0, world.game_state()).unwrap();
            read_goal(
                &memory,
                0,
                &countries,
                Row {
                    handle,
                    ..Default::default()
                },
            )
        };
        let row = read(&world).unwrap();
        assert_eq!(
            (row.resolved, row.type_token, row.actor, row.recipient),
            (1, 42, 1, 2)
        );
        assert_eq!(
            (
                row.state_count,
                row.first_state,
                row.country_count,
                row.first_country
            ),
            (2, 91, 3, 2)
        );
        for (address, expected) in [
            (goal, VTABLE_RVA.0),
            (definition, cwar_goal_type::VTABLE_RVA.0),
            (state, cstate::VTABLE_RVA.0),
        ] {
            world.u64(address, 1);
            assert!(read(&world).is_err());
            world.u64(address, expected as u64);
        }
        world.u32(goal + RECIPIENT, 99);
        assert!(read(&world).is_err());
        world.u32(goal + RECIPIENT, 2);
        world.u64(goal + HANDLE, handle + 1);
        assert!(read(&world).is_err());
        world.u64(goal + HANDLE, handle);
        world.u64(entry + bucket::OBJECT, 0);
        assert_eq!(read(&world).unwrap().resolved, 0);
        world.u64(table + registry::ENTRIES, 0);
        assert!(read(&world).is_err());
    }
}
