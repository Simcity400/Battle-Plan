use super::super::{Captured, CharactersCopyContext, CharactersJob, N, Roles, copy_characters};
use super::*;
use engine_native::cgame_state;
use engine_native_parts::cunit_leader_trait;
use engine_native_memory::{memory::SliceMemory, walk::Outcome};

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
        world.u64(manager, (BASE + ccharacter_manager::VTABLE_RVA.0) as u64);
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
            world.u64(character, (BASE + ccharacter::VTABLE_RVA.0) as u64);
            world.u32(character + ccharacter::TYPE, ccharacter::REFERENCE_TYPE);
            world.u32(character + ccharacter::ID, id as u32);
            world.u32(character + ccharacter::COUNTRY, id as u32);
            world.u64(character + ccharacter::UNIT_LEADER, unit as u64);
            world.u64(unit, (BASE + cunit_leader::VTABLE_RVA.0) as u64);
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
            world.u64(definition, (BASE + cunit_leader_trait::VTABLE_RVA.0) as u64);
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
fn failed_assigned_trait_hint_preserves_other_occurrences() {
    let _globals = crate::extended::characters::test_globals();
    let mut world = World::new();
    world.u32(world.units[0] + cunit_leader::TRAIT_COUNT, u32::MAX);
    let mut capacity = Capacities::default();
    assert!(capacity.presize(&world.memory(), BASE, STATE).is_err());
    assert!(capacity.rows[4] >= (COUNT - 1) * TRAITS);
    assert!(capacity.bytes[4] >= (COUNT - 1) * TRAITS * TOKEN.len());
}

#[test]
fn worker_grows_character_unit_text_and_census_before_first_publication() {
    let _globals = crate::extended::characters::test_globals();
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
    job.portrait_census = Some(
        engine_allocation::shared(
            engine_native::ccharacter::manager::PortraitCensus::try_new(0).unwrap(),
        )
        .unwrap(),
    );
    job.role_census =
        Some(engine_allocation::shared(super::super::census::RoleCensus::new(0)).unwrap());
    let context = CharactersCopyContext {
        base: BASE,
        game_state: STATE,
        force_full: false,
        active: [true, false, false, true, false, true],
    };
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [0; N],
        bytes: [0; N],
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
        assert!(copy_characters(&world.memory(), context, &mut job, Roles::Gather).raw_ok);
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
        captured.decode_characters_payload(BASE, &job, &mut [0; N]);
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
