use super::*;
use engine_native_memory::walk::Outcome;
use std::cell::RefCell;

const BASE: usize = 0x1000_0000;
const DATABASE: usize = 0x2000_0000;
const DEFINITIONS: usize = 0x2000_0800;
const DEFINITION: usize = 0x2100_0000;
const STATE: usize = 0x2200_0000;
const INDICES: usize = 0x2200_2000;
const BUILDINGS: usize = 0x2200_2100;
const BUILDING: usize = 0x2300_0000;
const ENTRIES: usize = 0x2300_1000;
const ENTRY: usize = 0x2400_0000;
const CATEGORY: usize = 0x2500_0000;
const GAME: usize = 0x2600_0000;
const STATES: usize = 0x2600_2000;
const TOKEN: u32 = 0x2fab;

struct World {
    regions: Vec<(usize, Vec<u8>)>,
    reads: RefCell<Vec<usize>>,
}

impl World {
    fn region(&mut self, at: usize, len: usize) {
        self.regions.push((at, vec![0; len]));
    }

    fn put<const N: usize>(&mut self, at: usize, value: [u8; N]) {
        let (start, bytes) = self
            .regions
            .iter_mut()
            .find(|(start, bytes)| at >= *start && at + N <= *start + bytes.len())
            .expect("write inside a region");
        let offset = at - *start;
        bytes[offset..offset + N].copy_from_slice(&value);
    }

    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.put(at, (data as u64).to_le_bytes());
        self.put(at + 8, count.to_le_bytes());
        self.put(at + 12, count.to_le_bytes());
    }

    fn reads(&self, address: usize) -> usize {
        self.reads.borrow().iter().filter(|&&at| at == address).count()
    }

    fn forget_reads(&self) {
        self.reads.borrow_mut().clear();
    }

    fn new() -> Self {
        let mut world = Self {
            regions: Vec::new(),
            reads: RefCell::default(),
        };
        world.region(BASE + 0x033d_d000, 0x9000);
        world.region(DATABASE, 0x1000);
        world.region(DEFINITION, template::SIZE);
        world.region(STATE, 0x1000);
        world.region(INDICES, 0x100);
        world.region(BUILDINGS, 0x100);
        world.region(BUILDING, 0x400);
        world.region(ENTRIES, 0x100);
        world.region(ENTRY, 0x100);
        world.region(CATEGORY, 0x200);
        world.region(GAME, 0x1000);
        world.region(STATES, 0x100);
        world.put(BASE + cbuilding_database::SINGLETON_RVA.0, (DATABASE as u64).to_le_bytes());
        world.array(DATABASE + INFRASTRUCTURE_DEFINITIONS, DEFINITIONS, 1);
        world.put(DEFINITIONS, (DEFINITION as u64).to_le_bytes());
        world.put(DEFINITION, ((BASE + template::VTABLE_RVA.0) as u64).to_le_bytes());
        world.put(DEFINITION + template::TYPE_ID, TOKEN.to_le_bytes());
        world.put(DEFINITION + template::LOOKUP_ENABLED_CANDIDATE, [1]);
        world.put(DEFINITION + template::DATABASE_ORDINAL, 2_u32.to_le_bytes());
        world.put(DEFINITION + STATE_MAXIMUM, 5_i32.to_le_bytes());
        world.put(DEFINITION + GROUP, NO_GROUP.to_le_bytes());
        world.put(STATE, ((BASE + cstate::VTABLE_RVA.0) as u64).to_le_bytes());
        world.put(STATE + cstate::ID, 1_u32.to_le_bytes());
        world.put(STATE + cstate::CATEGORY, (CATEGORY as u64).to_le_bytes());
        let status = STATE + cstate::BUILDING_STATUS;
        world.put(status + status::BUILDING_INDEX_COUNT, 3_i32.to_le_bytes());
        world.array(status + status::BUILDING_INDICES, INDICES, 3);
        for (index, value) in [-1_i32, -1, 0].into_iter().enumerate() {
            world.put(INDICES + index * 4, value.to_le_bytes());
        }
        world.array(status + status::BUILDINGS, BUILDINGS, 1);
        world.put(BUILDINGS, (BUILDING as u64).to_le_bytes());
        world.put(BUILDING, ((BASE + cbuilding::VTABLE_RVA.0) as u64).to_le_bytes());
        world.put(BUILDING + cbuilding::TEMPLATE, (DEFINITION as u64).to_le_bytes());
        world.put(BUILDING + cbuilding::LEVEL, 2_i16.to_le_bytes());
        world.array(BUILDING + PRIMARY_CONSTRUCTION, ENTRIES, 1);
        world.put(ENTRIES, (ENTRY as u64).to_le_bytes());
        world.put(ENTRY + CONSTRUCTION_LEVEL, 3_i16.to_le_bytes());
        world.put(CATEGORY, ((BASE + cstate::CATEGORY_VTABLE_RVA.0) as u64).to_le_bytes());
        world.array(GAME + cgame_state::STATES, STATES, 2);
        world.put(STATES + 8, (STATE as u64).to_le_bytes());
        world
    }
}

impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        let Some((start, bytes)) = self
            .regions
            .iter()
            .find(|(start, bytes)| address >= *start && address + output.len() <= *start + bytes.len())
        else {
            return false;
        };
        let offset = address - start;
        output.copy_from_slice(&bytes[offset..offset + output.len()]);
        true
    }
}

fn fresh(world: &World) -> (Infrastructure, Held, Proof) {
    let mut cache = Cache::default();
    let mut hour = Hour::new(world, BASE, GAME, &mut cache, false).unwrap();
    let state = hour.state_of(1).unwrap();
    let (held, proof) = hour.fresh(state).unwrap();
    (proof.value(), held, proof)
}

fn idle() -> World {
    let mut world = World::new();
    world.array(BUILDING + PRIMARY_CONSTRUCTION, 0, 0);
    world
}

fn gathered(world: &World) -> Option<Queue> {
    let mut bytes = [0; QUEUE_BYTES];
    let mut complete = [false];
    gather_queues(world, &[BUILDING], &mut bytes, &mut complete);
    complete[0].then_some(bytes)
}

#[test]
fn a_fresh_read_matches_the_snapshot() {
    let world = World::new();
    let (value, held, _) = fresh(&world);
    assert_eq!(value, Infrastructure { building_type: TOKEN, level: 2, maximum: 5 });
    assert_eq!(held, Held { instance: Some(BUILDING), level: 2, country: false, controller: 0 });
    let mut rows = [cstate::Row { id: 1, ..Default::default() }];
    snapshot(&world, BASE, GAME, Outcome::Rows { len: 1 }, &mut rows);
    assert_eq!(rows[0].infrastructure, Some(value));
}

#[test]
fn a_carried_state_reads_construction_bonus_and_category_but_not_its_status() {
    let world = World::new();
    let (_, held, _) = fresh(&world);
    let mut changed = World::new();
    changed.put(ENTRY + CONSTRUCTION_LEVEL, 1_i16.to_le_bytes());
    changed.put(BUILDING + MAXIMUM_BONUS, (-2_i16).to_le_bytes());
    let (expected, _, _) = fresh(&changed);
    assert_eq!(expected, Infrastructure { building_type: TOKEN, level: 2, maximum: 3 });
    changed.forget_reads();
    let mut cache = Cache::default();
    let mut hour = Hour::new(&changed, BASE, GAME, &mut cache, false).unwrap();
    assert_eq!(hour.carried(STATE, held).map(|proof| proof.value()), Ok(expected));
    let status = STATE + cstate::BUILDING_STATUS;
    for address in [
        status + status::BUILDING_INDEX_COUNT,
        status + status::BUILDING_INDICES,
        status + status::BUILDINGS,
        status + status::COUNTRY,
        BUILDING + cbuilding::LEVEL,
        BUILDING,
        STATE,
    ] {
        assert_eq!(changed.reads(address), 0, "{address:#x}");
    }
    assert_eq!(changed.reads(ENTRY + CONSTRUCTION_LEVEL), 1);
    assert_eq!(changed.reads(BUILDING + MAXIMUM_BONUS), 1);
    assert_eq!(changed.reads(STATE + cstate::CATEGORY), 1);
}

#[test]
fn an_hour_authenticates_each_category_once() {
    let world = World::new();
    let (_, held, _) = fresh(&world);
    world.forget_reads();
    let mut cache = Cache::default();
    let mut hour = Hour::new(&world, BASE, GAME, &mut cache, false).unwrap();
    let first = hour.carried(STATE, held);
    assert_eq!(hour.carried(STATE, held), first);
    assert_eq!(world.reads(CATEGORY), 1);
    drop(hour);
    let mut hour = Hour::new(&world, BASE, GAME, &mut cache, true).unwrap();
    assert_eq!(hour.carried(STATE, held), first);
    assert_eq!(world.reads(CATEGORY), 2);
}

#[test]
fn a_reused_plan_reads_only_its_pointers_until_they_move() {
    let world = World::new();
    let mut cache = Cache::default();
    drop(Hour::new(&world, BASE, GAME, &mut cache, true).unwrap());
    assert_eq!(world.reads(DEFINITION + STATE_MAXIMUM), 1);
    drop(Hour::new(&world, BASE, GAME, &mut cache, true).unwrap());
    assert_eq!(world.reads(DEFINITION + STATE_MAXIMUM), 1);
    drop(Hour::new(&world, BASE, GAME, &mut cache, false).unwrap());
    assert_eq!(world.reads(DEFINITION + STATE_MAXIMUM), 2);
    cache.forget();
    drop(Hour::new(&world, BASE, GAME, &mut cache, true).unwrap());
    assert_eq!(world.reads(DEFINITION + STATE_MAXIMUM), 3);
    let mut moved = World::new();
    moved.region(DEFINITION + 0x1000, template::SIZE);
    moved.put(DEFINITIONS, ((DEFINITION + 0x1000) as u64).to_le_bytes());
    moved.put(DEFINITION + 0x1000, ((BASE + template::VTABLE_RVA.0) as u64).to_le_bytes());
    assert!(Hour::new(&moved, BASE, GAME, &mut cache, true).is_ok());
    assert_eq!(moved.reads(DEFINITION + 0x1000 + STATE_MAXIMUM), 1);
}

#[test]
fn an_unreadable_definition_publishes_nothing() {
    let mut world = World::new();
    world.put(DEFINITION, 0_u64.to_le_bytes());
    let mut rows = [cstate::Row { id: 1, ..Default::default() }];
    snapshot(&world, BASE, GAME, Outcome::Rows { len: 1 }, &mut rows);
    assert_eq!(rows[0].infrastructure, None);
    let mut cache = Cache::default();
    assert!(Hour::new(&world, BASE, GAME, &mut cache, true).is_err());
}

#[test]
fn a_quiet_hour_is_proven_by_the_gathered_queue_and_category_alone() {
    let world = idle();
    let (value, held, proof) = fresh(&world);
    assert_eq!(value, Infrastructure { building_type: TOKEN, level: 2, maximum: 5 });
    world.forget_reads();
    let queue = gathered(&world).unwrap();
    let mut cache = Cache::default();
    drop(Hour::new(&world, BASE, GAME, &mut cache, false).unwrap());
    world.forget_reads();
    let mut hour = Hour::new(&world, BASE, GAME, &mut cache, true).unwrap();
    assert!(hour.proves());
    assert!(hour.quiet(&held, &proof, Some(&queue), CATEGORY as u64));
    for address in [STATE, STATE + cstate::CATEGORY, BUILDING, BUILDING + MAXIMUM_BONUS, CATEGORY] {
        assert_eq!(world.reads(address), 0, "{address:#x}");
    }
    assert!(!hour.quiet(&held, &proof, Some(&queue), CATEGORY as u64 + 8));
    assert!(!hour.quiet(&held, &proof, None, CATEGORY as u64));
}

#[test]
fn a_changed_or_queued_record_is_not_proven() {
    let world = idle();
    let (_, held, proof) = fresh(&world);
    let mut cache = Cache::default();
    let mut bonus = idle();
    bonus.put(BUILDING + MAXIMUM_BONUS, (-1_i16).to_le_bytes());
    let mut hour = Hour::new(&bonus, BASE, GAME, &mut cache, false).unwrap();
    assert!(!hour.quiet(&held, &proof, gathered(&bonus).as_ref(), CATEGORY as u64));
    assert_eq!(
        hour.carried(STATE, held).map(|proof| proof.value().maximum),
        Ok(4)
    );
    drop(hour);
    let queued = World::new();
    let (_, held, proof) = fresh(&queued);
    let mut hour = Hour::new(&queued, BASE, GAME, &mut cache, false).unwrap();
    assert!(!hour.quiet(&held, &proof, gathered(&queued).as_ref(), CATEGORY as u64));
}

#[test]
fn a_province_capped_definition_is_refused_by_name() {
    let mut world = World::new();
    world.put(DEFINITION + PROVINCE_MAXIMUM, 3_i32.to_le_bytes());
    let mut cache = Cache::default();
    assert_eq!(
        Hour::new(&world, BASE, GAME, &mut cache, false).err(),
        Some("infrastructure map province-capped definition unsupported")
    );
}
