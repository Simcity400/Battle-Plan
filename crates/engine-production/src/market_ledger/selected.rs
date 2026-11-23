use engine_native_memory::{Memory, read};
use engine_native::{ccountry, cequipment_variant::{self as variant, pool as pool}};
use engine_native_parts::{cequipment_type as definition, equipment_entry};

const GROUP_STRIDE: usize = 24;
const GROUP_FIRST: usize = 8;
const GROUP_COUNT: usize = 12;

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    at: usize,
    offset: usize,
) -> Result<T, &'static str> {
    at.checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("market selected pool operand unreadable")
}
fn key<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    at: usize,
) -> Result<(u8, i32, u32, u32), &'static str> {
    engine_native_parts::equipment_variant_identity::identity(memory, base, at)?;
    let model: u64 = field(memory, at, variant::DEFINITION)?;
    let canonical = |offset| -> Result<u32, &'static str> {
        let raw: u32 = field(memory, at, offset)?;
        if raw == 0 {
            Ok(0)
        } else {
            countries
                .link(raw)?
                .ok_or("market selected variant country unlinked")
        }
    };
    Ok((
        field(memory, at, variant::VERSION)?,
        field(memory, model as usize, definition::ARCHETYPE_ID)?,
        canonical(variant::CREATOR_LINK)?,
        canonical(variant::ORIGIN_LINK)?,
    ))
}

pub fn item(
    memory: &impl Memory,
    base: usize,
    destination: usize,
    source: usize,
) -> Result<equipment_entry::Row, &'static str> {
    let game: u64 = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|p| read(memory, p))
        .ok_or("market selected pool operand unreadable")?;
    let countries = ccountry::Reader::new(memory, base, game as usize)?;
    let wanted = key(memory, base, &countries, source)?;
    let model: u64 = field(memory, source, variant::DEFINITION)?;
    let archetype: u64 = field(memory, model as usize, definition::ARCHETYPE)?;
    let group_id: i32 = field(memory, archetype as usize, definition::ARCHETYPE_ID)?;
    let groups = pool::allocation_entries(memory, base, destination)?;
    let entries = pool::entries(memory, base, destination)?;
    let mut low = 0;
    let mut high = groups.count;
    while low < high {
        let mid = low + (high - low) / 2;
        let at = groups
            .element(mid, GROUP_STRIDE)
            .ok_or("market selected group address overflow")?;
        let pointer: u64 = field(memory, at, 0)?;
        let id: i32 = field(memory, pointer as usize, definition::ARCHETYPE_ID)?;
        if id < group_id {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    let at = groups
        .element(low, GROUP_STRIDE)
        .ok_or("market selected group absent")?;
    let pointer: u64 = field(memory, at, 0)?;
    if field::<i32>(memory, pointer as usize, definition::ARCHETYPE_ID)? != group_id {
        return Err("market selected group differs");
    }
    let first: u32 = field(memory, at, GROUP_FIRST)?;
    let count: u32 = field(memory, at, GROUP_COUNT)?;
    let end = first
        .checked_add(count)
        .filter(|end| *end <= entries.count)
        .ok_or("market selected variant range invalid")?;
    low = first;
    high = end;
    while low < high {
        let mid = low + (high - low) / 2;
        let at = entries
            .element(mid, equipment_entry::STRIDE)
            .ok_or("market selected entry address overflow")?;
        let pointer: u64 = field(memory, at, equipment_entry::VARIANT)?;
        if key(memory, base, &countries, pointer as usize)? < wanted {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    if low == end {
        return Err("market selected destination variant absent");
    }
    let item = equipment_entry::read(
        memory,
        base,
        entries
            .element(low, equipment_entry::STRIDE)
            .ok_or("market selected entry absent")?,
        low,
    )?;
    if key(memory, base, &countries, item.variant_pointer)? != wanted {
        return Err("market selected destination variant differs");
    }
    Ok(item)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Memory<'a> {
        world: &'a ccountry::synthetic::World,
        reads: std::cell::Cell<usize>,
    }
    impl engine_native_memory::Memory for Memory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if address == engine_native::cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
                output.copy_from_slice(&(self.world.game_state() as u64).to_le_bytes());
                true
            } else {
                self.world.memory().copy(address, output)
            }
        }
    }
    #[test]
    fn market_selected_item_uses_sorted_ranges_and_actual_destination_identity() {
        let mut world = ccountry::synthetic::World::new(0, 5000, 0x20);
        let destination = world.alloc(0x40);
        let groups = world.alloc(GROUP_STRIDE);
        let entries = world.alloc(5000 * equipment_entry::STRIDE);
        let archetype = world.alloc(0x500);
        world.u32(archetype + definition::ARCHETYPE_ID, 17);
        world.u64(destination, pool::VTABLE_RVA.0 as u64);
        world.array(destination + pool::ALLOCATIONS, groups, 1);
        world.array(destination + pool::ITEMS, entries, 5000);
        world.u64(groups, archetype as u64);
        world.u32(groups + GROUP_FIRST, 0);
        world.u32(groups + GROUP_COUNT, 5000);
        let mut source = 0;
        for index in 0..5000 {
            let model = world.alloc(definition::ARCHETYPE + 8);
            world.u64(model + definition::ARCHETYPE, archetype as u64);
            world.u32(model + definition::ARCHETYPE_ID, index as u32);
            let at = world.alloc(0x500);
            world.u64(at, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
            world.u64(
                at + 8,
                ((index as u64 + 1) << 32) | variant::REFERENCE_TYPE as u64,
            );
            world.u64(at + variant::DEFINITION, model as u64);
            world.u32(at + variant::CREATOR_LINK, 5000);
            world.u32(at + variant::ORIGIN_LINK, 4999);
            world.u64(entries + index * 16, at as u64);
            world.u64(entries + index * 16 + 8, index as u64);
            if index == 2500 {
                source = world.alloc(0x500);
                let bytes = world.bytes
                    [at - ccountry::synthetic::START..at - ccountry::synthetic::START + 0x500]
                    .to_vec();
                world.put(source, &bytes);
                world.u64(
                    source + 8,
                    (9999_u64 << 32) | variant::REFERENCE_TYPE as u64,
                );
            }
        }
        let memory = Memory {
            world: &world,
            reads: std::cell::Cell::new(0),
        };
        let found = item(&memory, 0, destination, source).unwrap();
        assert_eq!((found.source_ordinal, found.amount_raw), (2500, 2500));
        assert_eq!(found.variant_handle >> 32, 2501);
        assert!(
            memory.reads.get() < 300,
            "{} reads for one changed entry",
            memory.reads.get()
        );
    }
}
