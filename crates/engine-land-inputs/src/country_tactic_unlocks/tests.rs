use super::*;
use engine_native_intelligence::country_memory::Cache;
use engine_native_memory::memory::SliceMemory;
use std::{cell::RefCell, collections::BTreeMap};

const GAME: usize = 0x1000;
const COUNTRIES: usize = 0x2000;
const LINKS: usize = 0x2100;
const COUNTRY_A: usize = 0x3000;
const COUNTRY_B: usize = 0x4000;
const SYSTEM: usize = 0x5000;
const SYSTEM_MOVED: usize = 0x5800;
const DOCTRINE: usize = 0x6000;
const LIST: usize = 0x7000;
const TACTIC_A: usize = 0x8000;
const TACTIC_B: usize = 0x9000;
const DATABASE: usize = 0xa000;
const DATABASE_MOVED: usize = 0xa800;
const ENTRIES: usize = 0xb000;
const FOREIGN: usize = 0xc000;
const NULL_TACTIC: usize = 0xd000;

#[derive(Default)]
struct Fixture {
    bytes: BTreeMap<usize, u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    fail: Option<usize>,
    change: Option<(usize, usize, u64)>,
}

impl Fixture {
    fn put<const N: usize>(&mut self, at: usize, bytes: [u8; N]) {
        self.bytes
            .extend(bytes.into_iter().enumerate().map(|(i, b)| (at + i, b)));
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.put(at, (data as u64).to_le_bytes());
        self.put(at + 8, count.to_le_bytes());
        self.put(at + 12, count.to_le_bytes());
    }
    fn row(slot: usize) -> usize {
        DOCTRINE + slot * DOCTRINE_STRIDE
    }
    fn new() -> Self {
        let mut f = Self::default();
        f.array(GAME + cgame_state::COUNTRIES, COUNTRIES, 4);
        f.array(GAME + cgame_state::COUNTRY_LINKS, LINKS, 4);
        for (index, pointer) in [0, COUNTRY_A, COUNTRY_A, COUNTRY_B].into_iter().enumerate() {
            f.put(COUNTRIES + index * 8, (pointer as u64).to_le_bytes());
        }
        for (index, slot) in [0u32, 1, 1, 3].into_iter().enumerate() {
            f.put(LINKS + index * 4, slot.to_le_bytes());
        }
        for (pointer, id) in [(COUNTRY_A, 2u32), (COUNTRY_B, 3)] {
            f.put(pointer, (ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
            f.put(pointer + ccountry::ID, id.to_le_bytes());
        }
        f.put(GAME + DOCTRINE_SYSTEM, (SYSTEM as u64).to_le_bytes());
        f.array(SYSTEM + DOCTRINE_ROWS, DOCTRINE, 4);
        f.array(SYSTEM_MOVED + DOCTRINE_ROWS, DOCTRINE, 4);
        for slot in 0..4 {
            f.put(Self::row(slot) + ROW_TAG, [0; ROW_SPAN]);
        }
        f.put(Self::row(1) + ROW_TAG, 2u32.to_le_bytes());
        f.put(Self::row(3) + ROW_TAG, 3u32.to_le_bytes());
        f.array(Self::row(1) + TACTICS, LIST, 3);
        for (index, tactic) in [TACTIC_B, TACTIC_A, TACTIC_B].into_iter().enumerate() {
            f.put(LIST + index * 8, (tactic as u64).to_le_bytes());
        }
        f.put(
            ccombat_tactic::DATABASE_RVA.0,
            (DATABASE as u64).to_le_bytes(),
        );
        f.array(DATABASE + ccombat_tactic::DATABASE_ENTRIES, ENTRIES, 3);
        f.array(
            DATABASE_MOVED + ccombat_tactic::DATABASE_ENTRIES,
            ENTRIES,
            3,
        );
        for (index, tactic) in [NULL_TACTIC, TACTIC_A, TACTIC_B].into_iter().enumerate() {
            f.put(ENTRIES + index * 8, (tactic as u64).to_le_bytes());
        }
        f.put(
            NULL_TACTIC,
            (cnull_combat_tactic::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        f.put(NULL_TACTIC + ccombat_tactic::ID, 0i32.to_le_bytes());
        for (pointer, id) in [(TACTIC_A, 1i32), (TACTIC_B, 2), (FOREIGN, 3)] {
            f.put(pointer, (ccombat_tactic::VTABLE_RVA.0 as u64).to_le_bytes());
            f.put(pointer + ccombat_tactic::ID, id.to_le_bytes());
        }
        f
    }
}

impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let mut reads = self.reads.borrow_mut();
        let index = reads.len();
        let occurrence = reads.iter().filter(|(at, _)| *at == address).count();
        reads.push((address, output.len()));
        if self.fail == Some(index) {
            if let Some(first) = output.first_mut() {
                *first = 0xa5;
            }
            return false;
        }
        if let Some((at, count, replacement)) = self.change
            && address == at
            && occurrence == count
            && output.len() == 8
        {
            output.copy_from_slice(&replacement.to_le_bytes());
            return true;
        }
        for (i, byte) in output.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(address + i)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}

struct Owned {
    rows: Vec<Row>,
    unlocks: Vec<u32>,
    registry: Vec<RegistryEntry>,
    list: Vec<u8>,
}

impl Owned {
    fn new(rows: usize, unlocks: usize, registry: usize, list: usize) -> Self {
        Self {
            rows: vec![Row::default(); rows],
            unlocks: vec![0; unlocks],
            registry: vec![RegistryEntry::default(); registry],
            list: vec![0; list],
        }
    }
    fn fits() -> Self {
        Self::new(2, 3, 8, 24)
    }
    fn capture(&mut self, memory: &impl Memory, base: usize, game: usize) -> Capture {
        capture(
            memory,
            base,
            game,
            &mut self.rows,
            &mut self.unlocks,
            &mut self.registry,
            &mut self.list,
        )
    }
    fn fixture(&mut self, memory: &Fixture) -> Capture {
        self.capture(memory, 0, GAME)
    }
}

fn count(memory: &Fixture) -> Capture {
    use engine_native_memory::walk::Buffer;
    capture_buffer(memory, 0, GAME, Buffer::Count, Buffer::Count, Buffer::Count, Buffer::Count)
}

#[test]
fn census_records_saved_database_ids_in_native_order_with_one_copy_per_list() {
    let memory = Fixture::new();
    let mut owned = Owned::fits();
    let result = owned.fixture(&memory);
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        owned.rows[..2],
        [
            Row {
                country: 2,
                unlock_start: 0,
                unlock_count: 3
            },
            Row {
                country: 3,
                unlock_start: 3,
                unlock_count: 0
            },
        ]
    );
    assert_eq!(owned.unlocks, [2, 1, 2]);
    let reads = memory.reads.borrow();
    assert_eq!(
        reads
            .iter()
            .filter(|(at, _)| *at == LIST)
            .collect::<Vec<_>>(),
        [&(LIST, 24)]
    );
    assert_eq!(
        reads
            .iter()
            .filter(|(at, _)| *at == Fixture::row(1) + ROW_TAG)
            .collect::<Vec<_>>(),
        [&(Fixture::row(1) + ROW_TAG, ROW_SPAN)]
    );
    assert!(
        !reads
            .iter()
            .any(|(at, _)| (LIST + 8..LIST + 24).contains(at))
    );
    assert_eq!(reads.iter().filter(|(at, _)| *at == TACTIC_B).count(), 1);
}

#[test]
fn the_null_tactic_is_database_entry_zero_and_records_id_zero() {
    let mut memory = Fixture::new();
    memory.put(LIST + 8, (NULL_TACTIC as u64).to_le_bytes());
    let mut owned = Owned::fits();
    assert_eq!(owned.fixture(&memory).outcome, Outcome::Rows { len: 2 });
    assert_eq!(owned.unlocks, [2, 0, 2]);
    let invalid = Outcome::Unavailable {
        reason: "tactic database entry identity invalid",
    };
    let mut memory = Fixture::new();
    memory.put(
        NULL_TACTIC,
        (ccombat_tactic::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    assert_eq!(Owned::fits().fixture(&memory).outcome, invalid);
    let mut memory = Fixture::new();
    memory.put(NULL_TACTIC + ccombat_tactic::ID, 5i32.to_le_bytes());
    assert_eq!(Owned::fits().fixture(&memory).outcome, invalid);
}

#[test]
fn pressure_preserves_exact_demand_and_cannot_hide_an_unknown_tactic() {
    let memory = Fixture::new();
    let result = count(&memory);
    assert_eq!(
        result.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    assert_eq!(
        (
            result.required_rows,
            result.required_unlocks,
            result.required_registry,
            result.required_list
        ),
        (2, 3, 8, 24)
    );
    let mut memory = Fixture::new();
    memory.put(LIST + 8, (FOREIGN as u64).to_le_bytes());
    assert_eq!(
        Owned::fits().fixture(&memory).outcome,
        Outcome::Unavailable {
            reason: "country tactic unlock names no loaded tactic"
        }
    );
    let mut memory = Fixture::new();
    memory.put(LIST + 8, 0u64.to_le_bytes());
    assert_eq!(
        Owned::fits().fixture(&memory).outcome,
        Outcome::Unavailable {
            reason: "country tactic unlock names no loaded tactic"
        }
    );
}

#[test]
fn an_unfit_list_is_demand_only_once_its_ends_read() {
    let mut memory = Fixture::new();
    memory.array(Fixture::row(1) + TACTICS, 0x7fff_0000, i32::MAX as u32);
    let result = count(&memory);
    assert_eq!(
        result.outcome,
        Outcome::Unavailable {
            reason: "country tactic unlock list end unreadable"
        }
    );
    assert!(result.required_list < 1 << 20);
    let memory = Fixture::new();
    assert_eq!(count(&memory).required_list, 24);
}

#[test]
fn an_unreadable_list_is_refused_before_growing_capture_buffers() {
    let mut memory = Fixture::new();
    memory.array(Fixture::row(1) + TACTICS, 0x7fff_0000, 4096);
    let mut owned = Owned::new(0, 0, 0, 0);
    let result = owned.fixture(&memory);
    assert_eq!(
        result.outcome,
        Outcome::Unavailable {
            reason: "country tactic unlock list unreadable"
        }
    );
    assert_eq!(owned.list.len(), 24);
    assert!(owned.unlocks.is_empty());
}

#[test]
fn every_short_read_is_unavailable_and_stops_at_the_first_fault() {
    let memory = Fixture::new();
    let _ = Owned::fits().fixture(&memory);
    let count = memory.reads.borrow().len();
    for fault in 0..count {
        let mut memory = Fixture::new();
        memory.fail = Some(fault);
        let result = Owned::fits().fixture(&memory);
        assert!(
            matches!(result.outcome, Outcome::Unavailable { .. }),
            "fault {fault}"
        );
        assert_eq!(memory.reads.borrow().len(), fault + 1, "fault {fault}");
    }
}

#[test]
fn malformed_database_rows_and_changed_roots_never_become_empty_lists() {
    let invalid = Outcome::Unavailable {
        reason: "tactic database entry identity invalid",
    };
    let mut memory = Fixture::new();
    memory.put(TACTIC_A, 0u64.to_le_bytes());
    assert_eq!(Owned::fits().fixture(&memory).outcome, invalid);
    let mut memory = Fixture::new();
    memory.put(TACTIC_B + ccombat_tactic::ID, 7i32.to_le_bytes());
    assert_eq!(Owned::fits().fixture(&memory).outcome, invalid);
    let mut memory = Fixture::new();
    memory.put(ENTRIES + 16, (TACTIC_A as u64).to_le_bytes());
    assert_eq!(Owned::fits().fixture(&memory).outcome, invalid);
    let mut memory = Fixture::new();
    memory.put(Fixture::row(1) + ROW_TAG, 3u32.to_le_bytes());
    assert_eq!(
        Owned::fits().fixture(&memory).outcome,
        Outcome::Unavailable {
            reason: "country tactic unlock row names another country"
        }
    );
    let changed = Outcome::Unavailable {
        reason: "country tactic unlock census changed during capture",
    };
    for (address, moved) in [
        (GAME + DOCTRINE_SYSTEM, SYSTEM_MOVED),
        (ccombat_tactic::DATABASE_RVA.0, DATABASE_MOVED),
    ] {
        let mut memory = Fixture::new();
        memory.change = Some((address, 1, moved as u64));
        assert_eq!(Owned::fits().fixture(&memory).outcome, changed);
    }
}

const WORLD: usize = 0x1000_0000;

fn world(countries: usize, tactics: usize, unlocks: usize) -> (Vec<u8>, usize, usize) {
    let slots = countries + 1;
    let mut end = WORLD;
    let mut region = |size: usize| {
        let start = end;
        end += size.next_multiple_of(0x10);
        start
    };
    let game = region(0x1000);
    let table = region(slots * 8);
    let links = region(slots * 4);
    let objects = region(slots * 0x10);
    let system = region(0x20);
    let doctrine = region(slots * DOCTRINE_STRIDE);
    let database = region(0x20);
    let entries = region((tactics + 1) * 8);
    let objects_t = region((tactics + 1) * 0x100);
    let lists = region(countries * unlocks * 8);
    let base = region(8) - ccombat_tactic::DATABASE_RVA.0;
    let mut bytes = vec![0u8; end - WORLD];
    let mut put = |address: usize, value: &[u8]| {
        bytes[address - WORLD..address - WORLD + value.len()].copy_from_slice(value);
    };
    let header = |data: usize, count: usize| {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&(data as u64).to_le_bytes());
        out[8..12].copy_from_slice(&(count as u32).to_le_bytes());
        out[12..].copy_from_slice(&(count as u32).to_le_bytes());
        out
    };
    put(game + cgame_state::COUNTRIES, &header(table, slots));
    put(game + cgame_state::COUNTRY_LINKS, &header(links, slots));
    put(game + DOCTRINE_SYSTEM, &(system as u64).to_le_bytes());
    put(system + DOCTRINE_ROWS, &header(doctrine, slots));
    for slot in 1..slots {
        let object = objects + slot * 0x10;
        put(table + slot * 8, &(object as u64).to_le_bytes());
        put(links + slot * 4, &(slot as u32).to_le_bytes());
        put(
            object,
            &((base + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(object + ccountry::ID, &(slot as u32).to_le_bytes());
        let row = doctrine + slot * DOCTRINE_STRIDE;
        put(row + ROW_TAG, &(slot as u32).to_le_bytes());
        let list = lists + (slot - 1) * unlocks * 8;
        put(row + TACTICS, &header(list, unlocks));
        for ordinal in 0..unlocks {
            let index = 1 + (slot + ordinal) % tactics;
            let tactic = objects_t + index * 0x100;
            put(list + ordinal * 8, &(tactic as u64).to_le_bytes());
        }
    }
    put(
        base + ccombat_tactic::DATABASE_RVA.0,
        &(database as u64).to_le_bytes(),
    );
    put(
        database + ccombat_tactic::DATABASE_ENTRIES,
        &header(entries, tactics + 1),
    );
    for index in 0..=tactics {
        let tactic = objects_t + index * 0x100;
        let vtable = match index {
            0 => cnull_combat_tactic::VTABLE_RVA,
            _ => ccombat_tactic::VTABLE_RVA,
        };
        put(entries + index * 8, &(tactic as u64).to_le_bytes());
        put(tactic, &((base + vtable.0) as u64).to_le_bytes());
        put(tactic + ccombat_tactic::ID, &(index as i32).to_le_bytes());
    }
    (bytes, base, game)
}

struct Counted<'a, M> {
    memory: M,
    copies: std::cell::Cell<usize>,
    bytes: std::cell::Cell<usize>,
    _world: std::marker::PhantomData<&'a ()>,
}

impl<M: Memory> Memory for Counted<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.copies.set(self.copies.get() + 1);
        self.bytes.set(self.bytes.get() + output.len());
        self.memory.copy(address, output)
    }
    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine_native_memory::country_identity::AuthenticatedRoster<'_>> {
        self.memory.authenticated_countries(base, game_state)
    }
}

