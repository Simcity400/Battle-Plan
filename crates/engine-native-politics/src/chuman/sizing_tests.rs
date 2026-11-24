use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const MEMORY_BASE: usize = 0x1000;
const BASE: usize = 0x140000000;
const GS: usize = 0x1100;
const HUMANS: usize = 0x10000;

struct World {
    bytes: Vec<u8>,
    countries: usize,
    counts: usize,
}

impl World {
    fn new(country_count: u32, player_count: u32, user_len: usize) -> Self {
        let slots = country_count as usize + 1;
        let countries = HUMANS + player_count as usize * STRIDE;
        let links = countries + slots * 8;
        let counts = links + slots * 4;
        let objects = (counts + slots + 7) & !7;
        let heap = objects + slots * 16;
        let mut world = Self {
            bytes: vec![0; heap - MEMORY_BASE + player_count as usize * (user_len + 20)],
            countries,
            counts,
        };
        world.array(GS + cgame_state::PLAYER_RECORDS, HUMANS, player_count);
        world.array(GS + cgame_state::COUNTRIES, countries, country_count + 1);
        world.array(GS + cgame_state::COUNTRY_LINKS, links, country_count + 1);
        world.array(GS + cgame_state::HUMAN_STATUS, counts, country_count + 1);
        for slot in 1..=country_count {
            let pointer = objects + slot as usize * 16;
            world.put(
                countries + slot as usize * 8,
                &(pointer as u64).to_le_bytes(),
            );
            world.put(links + slot as usize * 4, &slot.to_le_bytes());
            world.put(
                pointer,
                &((BASE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            world.put(pointer + ccountry::ID, &slot.to_le_bytes());
            world.put(counts + slot as usize, &[slot as u8]);
        }
        for index in 0..player_count {
            let address = HUMANS + index as usize * STRIDE;
            let text = heap + index as usize * (user_len + 20);
            world.put(address, &((BASE + VTABLE_RVA.0) as u64).to_le_bytes());
            world.put(address + PLAYER_KEY, &(index as i32).to_le_bytes());
            world.put(
                address + COUNTRY_TAG,
                &(index % country_count + 1).to_le_bytes(),
            );
            world.put(address + FLAGS, &[index as u8]);
            world.string(address + FIRST_STRING, text, user_len, index as u8);
            world.string(address + NAME, text + user_len + 1, 18, (index + 1) as u8);
        }
        world
    }

    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: MEMORY_BASE,
            bytes: &self.bytes,
        }
    }

    fn put(&mut self, address: usize, value: &[u8]) {
        self.bytes[address - MEMORY_BASE..address - MEMORY_BASE + value.len()]
            .copy_from_slice(value);
    }

    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.put(address, &(data as u64).to_le_bytes());
        self.put(address + 8, &count.to_le_bytes());
        self.put(address + 12, &count.to_le_bytes());
    }

    fn string(&mut self, address: usize, source: usize, len: usize, value: u8) {
        assert!(len > 15);
        self.put(address, &(source as u64).to_le_bytes());
        self.put(address + 16, &(len as u64).to_le_bytes());
        self.put(address + 24, &(len as u64).to_le_bytes());
        self.bytes[source - MEMORY_BASE..source - MEMORY_BASE + len].fill(value);
    }
}

struct TraceMemory<'a> {
    memory: SliceMemory<'a>,
    reads: RefCell<Vec<(usize, usize)>>,
}

impl Memory for TraceMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        self.memory.copy(address, output)
    }
}

#[test]
fn growing_controllers_keep_original_count_bounds() {
    let mut world = World::new(50, 1, 29);
    let mut rows = Vec::new();
    assert_eq!(
        walk_controllers(&world.memory(), BASE, GS, &mut rows),
        Outcome::Rows { len: 50 }
    );
    world.put(GS + cgame_state::HUMAN_STATUS + 12, &50_u32.to_le_bytes());
    let memory = TraceMemory {
        memory: world.memory(),
        reads: RefCell::new(Vec::new()),
    };
    assert_eq!(
        walk_controllers(&memory, BASE, GS, &mut Vec::new()),
        Outcome::Unavailable {
            reason: "human controller count unreadable or outside array"
        }
    );
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(address, _)| address == world.counts + 50)
    );
    assert!(
        memory
            .reads
            .borrow()
            .iter()
            .any(|&(address, _)| address == world.countries + 50 * 8)
    );
}
