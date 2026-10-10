use super::super::{
    Captured, CharactersCopyContext, CharactersJob, Roles, census, copy_characters_growing,
};
use super::*;
use engine::{cgame_state, cunit_leader_trait, memory::SliceMemory, walk::Outcome};
use std::cell::Cell;
use std::mem::size_of;

const BASE: usize = 0x1_4000_0000;
const STATE: usize = 0x1000;
const COUNT: usize = 5_000;
const TRAITS: usize = 5;
const EXPERIENCE: usize = 17;
const TOKEN: &[u8] = b"mod_trait_name";

struct World {
    bytes: Vec<u8>,
    units: Vec<usize>,
}

impl World {
    fn alloc(&mut self, len: usize) -> usize {
        let address = self.bytes.len();
        self.bytes.resize(address + len, 0);
        address
    }
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.bytes[address..address + bytes.len()].copy_from_slice(bytes);
    }
    fn u64(&mut self, address: usize, word: u64) {
        self.put(address, &word.to_le_bytes());
    }
    fn u32(&mut self, address: usize, word: u32) {
        self.put(address, &word.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: usize) {
        self.u64(address, data as u64);
        self.u32(address + 8, count as u32);
        self.u32(address + 12, count as u32);
    }
    fn new() -> Self {
        let mut world = Self {
            bytes: vec![0; 0x2000],
            units: Vec::new(),
        };
        let manager = world.alloc(0x40);
        world.u64(STATE + cgame_state::CHARACTER_MANAGER, manager as u64);
        world.u64(manager, (BASE + ccharacter_manager::VTABLE_RVA) as u64);
        let roster = world.alloc(COUNT * 8);
        world.array(
            manager + ccharacter_manager::HISTORICAL_CHARACTERS,
            roster,
            COUNT,
        );
        for id in 1..=COUNT {
            let character = world.alloc(0x120);
            let unit = world.alloc(0x1000);
            world.units.push(unit);
            world.u64(roster + (id - 1) * 8, character as u64);
            world.u64(character, (BASE + ccharacter::VTABLE_RVA) as u64);
            world.u32(character + ccharacter::TYPE, ccharacter::REFERENCE_TYPE);
            world.u32(character + ccharacter::ID, id as u32);
            world.u32(character + ccharacter::COUNTRY, id as u32);
            world.u64(character + ccharacter::UNIT_LEADER, unit as u64);
            world.u64(unit, (BASE + cunit_leader::VTABLE_RVA) as u64);
            for offset in [cunit_leader::NAME, cunit_leader::TOKEN] {
                world.u64(unit + offset + 24, 15);
            }

            world.u32(unit + cunit_leader::TYPE, cunit_leader::REFERENCE_TYPE);
            world.u32(unit + cunit_leader::ID, id as u32);
            world.u32(
                unit + cunit_leader::CHARACTER_TYPE,
                ccharacter::REFERENCE_TYPE,
            );
            world.u32(unit + cunit_leader::CHARACTER_ID, id as u32);
            let definition = world.alloc(cunit_leader_trait::SIZE);
            world.u64(definition, (BASE + cunit_leader_trait::VTABLE_RVA) as u64);
            world.u32(definition + cunit_leader_trait::TOKEN_ID, id as u32);
            world.u32(definition + cunit_leader_trait::ORDINAL, id as u32);
            world.put(definition + cunit_leader_trait::TOKEN, TOKEN);
            world.u64(
                definition + cunit_leader_trait::TOKEN + 16,
                TOKEN.len() as u64,
            );
            world.u64(definition + cunit_leader_trait::TOKEN + 24, 15);
            let traits = world.alloc(TRAITS * 8);
            world.array(unit + cunit_leader::TRAIT_DATA, traits, TRAITS);
            for ordinal in 0..TRAITS {
                world.u64(traits + ordinal * 8, definition as u64);
            }
            let experience = world.alloc(EXPERIENCE * 16);
            world.array(
                unit + cunit_leader::TRAIT_EXPERIENCE_DATA,
                experience,
                EXPERIENCE,
            );
            for ordinal in 0..EXPERIENCE {
                world.u64(experience + ordinal * 16, definition as u64);
                world.u64(
                    experience + ordinal * 16 + 8,
                    (id as u64) << 32 | ordinal as u64,
                );
            }
        }
        world
    }
    fn memory(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
    }
}

#[test]
fn live_trait_demand_presizes_complete_5000_country_roster_and_token_arenas() {
    let world = World::new();
    let mut capacity = Capacities::default();
    assert!(capacity.rows[5] < COUNT * EXPERIENCE);
    capacity.presize(&world.memory(), BASE, STATE).unwrap();
    assert!(capacity.rows[3] >= COUNT);
    assert!(capacity.rows[4] >= COUNT * TRAITS);
    assert!(capacity.bytes[4] >= COUNT * TRAITS * TOKEN.len());
    assert!(capacity.rows[5] >= COUNT * EXPERIENCE);
    assert!(capacity.bytes[5] >= COUNT * EXPERIENCE * TOKEN.len());
}

#[test]
fn attach_hints_seed_character_auxiliary_storage_from_5000_country_demand() {
    let world = World::new();
    let mut capacity = Capacities::default();
    capacity.presize(&world.memory(), BASE, STATE).unwrap();
    let roster = CharactersJob::with_storage(0, 0, 0, false);
    assert!(
        roster.portrait_census.as_ref().unwrap().allocated_bytes()
            >= COUNT * size_of::<ccharacter_manager::PortraitIdentity>()
    );
    assert!(
        roster.role_census.as_ref().unwrap().allocated_bytes()
            >= COUNT * (size_of::<usize>() + census::ROW)
    );
    assert!(roster.appended_roles.capacity() >= COUNT);
    let traits = CharactersJob::with_storage(3, 0, 0, true);
    assert!(traits.census_visited.capacity() >= COUNT);
    assert!(traits.source_bindings.as_ref().unwrap().sources.capacity() >= COUNT * 2);
    assert!(traits.traits.definitions.entries.capacity() >= COUNT * (TRAITS + EXPERIENCE));
    assert!(
        traits
            .traits
            .vectors
            .iter()
            .all(|rows| rows.capacity() >= COUNT)
    );
}

#[test]
fn failed_assigned_trait_hint_preserves_experience_and_other_occurrences() {
    let mut world = World::new();
    world.u32(world.units[0] + cunit_leader::TRAIT_COUNT, u32::MAX);
    let mut capacity = Capacities::default();
    assert!(capacity.presize(&world.memory(), BASE, STATE).is_err());
    assert!(capacity.rows[4] >= (COUNT - 1) * TRAITS);
    assert!(capacity.rows[5] >= COUNT * EXPERIENCE);
    assert!(capacity.bytes[5] >= COUNT * EXPERIENCE * TOKEN.len());
}

#[test]
fn final_input_fallback_grows_5000_country_traits_before_first_publication() {
    let world = World::new();
    let mut captured = Captured::vacant();
    assert_eq!(captured.allocated_bytes(), 0);
    assert!(captured.fallback_jobs.iter().all(Option::is_none));
    for _ in 0..2 {
        captured.set_character_activity([false, false, false, false, true, true]);
        // An independent raw-owned table does not authorize a duplicate read.
        captured.raw_active[0] = true;
        captured.capture_growing(&world.memory(), BASE, STATE, 0, false);
        assert_eq!(
            captured.traits.rows.outcome,
            Outcome::Rows {
                len: COUNT * TRAITS
            }
        );
        assert_eq!(
            captured.trait_experience.rows.outcome,
            Outcome::Rows {
                len: COUNT * EXPERIENCE
            }
        );
        assert_eq!(
            captured.trait_experience.bytes.len,
            COUNT * EXPERIENCE * TOKEN.len()
        );
        assert!(captured.fallback_jobs[..3].iter().all(Option::is_none));
        assert!(captured.fallback_jobs[3].is_some());
        assert!(captured.raw_active[0]);
    }
}

#[test]
fn worker_capture_grows_original_trait_observations_and_projection_at_5000_countries() {
    struct Traced<'a> {
        world: &'a World,
        first_xp_header_reads: Cell<usize>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == self.world.units[0] + cunit_leader::TRAIT_EXPERIENCE_DATA {
                self.first_xp_header_reads
                    .set(self.first_xp_header_reads.get() + 1);
            }
            self.world.memory().copy(address, output)
        }
    }
    let world = World::new();
    let memory = Traced {
        world: &world,
        first_xp_header_reads: Cell::new(0),
    };
    let mut job = CharactersJob::with_storage(3, 0, 0, true);
    let context = CharactersCopyContext {
        base: BASE,
        game_state: STATE,
        force_full: true,
        active: [false, false, false, false, true, true],
        trait_rows: [1; 2],
        trait_bytes: [1; 2],
    };
    assert!(copy_characters_growing(&memory, context, &mut job, Roles::Independent).raw_ok);
    assert_eq!(memory.first_xp_header_reads.get(), 1);
    let mut capacity = Capacities::default();
    let mut captured = Captured::with_capacities(&capacity);
    captured.decode_characters_payload(BASE, &job, &mut [0; 6]);
    assert_eq!(
        captured.traits.rows.outcome,
        Outcome::Rows {
            len: COUNT * TRAITS
        }
    );
    assert_eq!(
        captured.trait_experience.rows.outcome,
        Outcome::Rows {
            len: COUNT * EXPERIENCE
        }
    );
    assert_eq!(
        captured.trait_experience.bytes.len,
        COUNT * EXPERIENCE * TOKEN.len()
    );
    let last = captured.trait_experience.rows.rows[COUNT * EXPERIENCE - 1];
    assert_eq!(last.trait_row.leader_id as usize, COUNT);
    assert_eq!(last.trait_row.ordinal as usize, EXPERIENCE - 1);
    assert_eq!(
        last.experience as u64,
        (COUNT as u64) << 32 | (EXPERIENCE - 1) as u64
    );
    captured.reset();
    captured.grow(&mut capacity);
    let mut idle = Captured::with_capacities(&capacity);
    idle.decode_characters_payload(BASE, &job, &mut [0; 6]);
    assert_eq!(
        idle.trait_experience.rows.outcome,
        Outcome::Rows {
            len: COUNT * EXPERIENCE
        }
    );
}

#[test]
fn worker_grows_5000_country_leader_maps_and_retains_prepared_demand() {
    use engine::{ccountry_leader, character_leader_node, msvc_tree_node as node};
    let mut world = World::new();
    let mut job = CharactersJob::with_storage(1, 0, 0, false);
    let context = CharactersCopyContext {
        base: BASE,
        game_state: STATE,
        force_full: false,
        active: [false, true, false, false, false, false],
        trait_rows: [0; 2],
        trait_bytes: [0; 2],
    };
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [0; 6],
        bytes: [0; 6],
    });
    for count in [3_usize, 7] {
        for index in 0..COUNT {
            let character = world.units[index] - 0x120;
            let head = world.alloc(32);
            let nodes: Vec<_> = (0..count)
                .map(|_| world.alloc(character_leader_node::LEADER + 8))
                .collect();
            world.u64(character + ccharacter::LEADER_MAP_HEAD, head as u64);
            world.u64(character + ccharacter::LEADER_MAP_SIZE, count as u64);
            world.u64(head + node::LEFT, nodes[0] as u64);
            world.u64(head + node::PARENT, nodes[0] as u64);
            world.u64(head + node::RIGHT, nodes[count - 1] as u64);
            world.put(head + node::COLOR, &[1, 1]);
            for (ordinal, &entry) in nodes.iter().enumerate() {
                world.u64(entry + node::LEFT, head as u64);
                world.u64(
                    entry + node::PARENT,
                    if ordinal == 0 {
                        head
                    } else {
                        nodes[ordinal - 1]
                    } as u64,
                );
                world.u64(
                    entry + node::RIGHT,
                    nodes.get(ordinal + 1).copied().unwrap_or(head) as u64,
                );
                let role = world.alloc(0x150);
                world.u64(role, (BASE + ccountry_leader::VTABLE_RVA) as u64);
                world.u64(role + ccountry_leader::CHARACTER, character as u64);
                world.u32(entry + character_leader_node::IDEOLOGY, ordinal as u32);
                world.u64(entry + character_leader_node::LEADER, role as u64);
            }
        }
        for _ in 0..2 {
            assert!(
                copy_characters_growing(&world.memory(), context, &mut job, Roles::Independent)
                    .raw_ok
            );
            captured.decode_characters_payload(BASE, &job, &mut [0; 6]);
            assert_eq!(
                captured.leaders.rows.outcome,
                Outcome::Rows { len: COUNT * count }
            );
            let last = captured.leaders.rows.rows[COUNT * count - 1];
            assert_eq!(
                (last.character as usize, last.ideology as usize),
                (COUNT, count - 1)
            );
            captured.reset();
        }
    }
}

#[test]
fn worker_grows_character_unit_text_and_census_before_first_publication() {
    let mut world = World::new();
    for index in 0..COUNT {
        let unit = world.units[index];
        let character = unit - 0x120;
        for (address, value) in [
            (character + ccharacter::NAME, b"char_name".as_slice()),
            (character + ccharacter::TOKEN, b"char_token"),
            (unit + cunit_leader::NAME, b"unit_name"),
            (unit + cunit_leader::TOKEN, b"unit_token"),
        ] {
            world.put(address, value);
            world.u64(address + 16, value.len() as u64);
            world.u64(address + 24, 15);
        }
    }
    let mut job = CharactersJob::with_storage(0, 0, 0, false);
    job.portrait_census = Some(std::sync::Arc::new(
        engine::ccharacter_manager::PortraitCensus::try_new(0).unwrap(),
    ));
    job.role_census = Some(std::sync::Arc::new(super::super::census::RoleCensus::new(
        0,
    )));
    let context = CharactersCopyContext {
        base: BASE,
        game_state: STATE,
        force_full: false,
        active: [true, false, false, true, false, false],
        trait_rows: [0; 2],
        trait_bytes: [0; 2],
    };
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [0; 6],
        bytes: [0; 6],
    });
    for enlarged in [false, true] {
        if enlarged {
            let heap = world.alloc(4_097);
            world.put(heap, &vec![b'x'; 4_096]);
            for address in [
                world.units[0] - 0x120 + ccharacter::NAME,
                world.units[0] + cunit_leader::NAME,
            ] {
                world.u64(address, heap as u64);
                world.u64(address + 16, 4_096);
                world.u64(address + 24, 4_096);
            }
        }
        assert!(copy_characters_growing(&world.memory(), context, &mut job, Roles::Gather).raw_ok);
        assert_eq!(
            job.portrait_census()
                .unwrap()
                .rows(BASE, STATE)
                .unwrap()
                .len(),
            COUNT
        );
        assert_eq!(
            job.role_census()
                .unwrap()
                .admitted(BASE, STATE)
                .unwrap()
                .len(),
            COUNT
        );
        captured.decode_characters_payload(BASE, &job, &mut [0; 6]);
        assert_eq!(
            captured.characters.rows.outcome,
            Outcome::Rows { len: COUNT }
        );
        assert_eq!(captured.units.rows.outcome, Outcome::Rows { len: COUNT });
        assert_eq!(
            captured.characters.bytes.len,
            COUNT * 19 + if enlarged { 4_096 - 9 } else { 0 }
        );
        assert_eq!(
            captured.units.bytes.len,
            COUNT * 19 + if enlarged { 4_096 - 9 } else { 0 }
        );
        captured.reset();
    }
}
