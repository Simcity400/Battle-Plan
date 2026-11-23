use super::*;
use crate::local_subunit_changes::Watches;
use engine_deployment::cdeployment_status;
use engine_native::{ccountry, cgame_state};
use engine_native_parts::{cequipment_type, csub_unit_definition};

struct World(Vec<u8>);
const STATE: usize = 0x1000;
const DATABASE: usize = 0x9800;
const ARCHETYPE: usize = 0xa000;
const GLOBAL: usize = 0xb000;
impl engine_native_memory::Memory for World {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        if at == csub_unit_definition::DATABASE_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&(DATABASE as u64).to_le_bytes());
            return true;
        }
        engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &self.0,
        }
        .copy(at, output)
    }
}
impl World {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn word(&mut self, at: usize, value: usize) {
        self.put(at, &(value as u64).to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.word(at, data);
        self.put(at + 8, &count.to_le_bytes());
        self.put(at + 12, &count.to_le_bytes());
    }
    fn subunit(&mut self, at: usize, amount: i64) {
        self.word(at, csub_unit_definition::VTABLE_RVA.0);
        self.put(at + csub_unit_definition::TOKEN, &5_u32.to_le_bytes());
        self.0[at + csub_unit_definition::IS_REAL] = 1;
        self.put(
            at + csub_unit_definition::DATABASE_INDEX,
            &1_i32.to_le_bytes(),
        );
        self.array(
            at + csub_unit_definition::ARCHETYPE_REQUIREMENTS_DATA,
            at + 0x700,
            1,
        );
        self.word(at + 0x700, ARCHETYPE);
        self.put(at + 0x708, &amount.to_le_bytes());
    }
    fn country(id: usize) -> usize {
        0x4000 + (id - 1) * 0x2000
    }
    fn status(id: usize) -> usize {
        Self::country(id) + 0x1400
    }
    fn clone_of(id: usize) -> usize {
        0x10000 + id * 0x1000
    }
}

fn world() -> World {
    let mut world = World(vec![0; 0x14000]);
    world.array(STATE + cgame_state::COUNTRIES, 0x2000, 3);
    world.array(STATE + cgame_state::COUNTRY_LINKS, 0x2100, 3);
    for id in 1..=2 {
        let country = World::country(id);
        let status = World::status(id);
        world.word(0x2000 + id * 8, country);
        world.put(0x2100 + id * 4, &(id as u32).to_le_bytes());
        world.word(country, ccountry::VTABLE_RVA.0);
        world.put(country + ccountry::ID, &(id as u32).to_le_bytes());
        world.word(country + ccountry::DEPLOYMENT_MANAGER, status);
        world.word(status, cdeployment_status::VTABLE_RVA.0);
        world.word(status + cdeployment_status::OWNER, country);
        world.array(
            status + cdeployment_status::LOCAL_SUBUNITS,
            status + 0x100,
            1,
        );
        world.word(status + 0x100, World::clone_of(id));
        world.subunit(World::clone_of(id), 200_000 + id as i64);
    }
    world.subunit(GLOBAL, 100_000);
    world.array(DATABASE + csub_unit_definition::DATABASE_VECTOR, 0x9900, 2);
    world.word(0x9908, GLOBAL);
    world.word(ARCHETYPE, cequipment_type::VTABLE_RVA.0);
    world.put(ARCHETYPE + cequipment_type::TOKEN, &71_u32.to_le_bytes());
    world.0[ARCHETYPE + cequipment_type::ACTIVE] = 1;
    world
}

struct Hour {
    rows: Vec<LocalSubunitRow>,
    requirements: Vec<LocalRequirement>,
}

fn hour(world: &World, store: &mut LocalStore, watches: &Watches) -> Hour {
    let (mut rows, mut requirements) = (Vec::new(), Vec::new());
    let walk = capture(world, 0, STATE, store, watches, &mut rows, &mut requirements);
    rows.truncate(walk.outcome.len());
    requirements.truncate(walk.requirements_len);
    Hour { rows, requirements }
}

fn amounts(hour: &Hour) -> Vec<(u32, i64)> {
    hour.rows
        .iter()
        .map(|row| {
            (
                row.country,
                hour.requirements[row.requirements.start].amount_raw,
            )
        })
        .collect()
}

#[test]
fn quiet_hours_carry_and_a_written_country_is_read_again() {
    let mut world = world();
    let watches = Watches::enabled(16);
    let mut store = LocalStore::default();
    let first = hour(&world, &mut store, &watches);
    assert_eq!(amounts(&first), [(1, 200_001), (2, 200_002)]);
    assert!(store.quiet.is_some());
    world.word(World::country(1) + ccountry::DEPLOYMENT_MANAGER, 0);
    let quiet = hour(&world, &mut store, &watches);
    assert_eq!(amounts(&quiet), amounts(&first));
    world.word(
        World::country(1) + ccountry::DEPLOYMENT_MANAGER,
        World::status(1),
    );
    world.subunit(World::clone_of(2), 300_000);
    drop(watches.notify(Some(World::status(2)), 1));
    world.subunit(World::clone_of(1), 400_000);
    let written = hour(&world, &mut store, &watches);
    assert_eq!(amounts(&written), [(1, 200_001), (2, 300_000)]);
    world.array(
        World::status(1) + cdeployment_status::LOCAL_SUBUNITS,
        World::status(1) + 0x100,
        2,
    );
    world.word(World::status(1) + 0x108, World::clone_of(1) + 0x800);
    world.subunit(World::clone_of(1) + 0x800, 100_000);
    drop(watches.notify(Some(0xdead), 0));
    let appended = hour(&world, &mut store, &watches);
    assert_eq!(amounts(&appended), [(1, 400_000), (2, 300_000)]);
    assert!(!watches.poisoned());
}

#[test]
fn a_failed_walk_forgets_the_store() {
    let mut world = world();
    let watches = Watches::enabled(16);
    let mut store = LocalStore::default();
    hour(&world, &mut store, &watches);
    world.word(World::status(2), 0);
    drop(watches.notify(Some(World::status(2)), 2));
    let failed = hour(&world, &mut store, &watches);
    assert!(failed.rows.is_empty());
    assert!(store.quiet.is_none() && store.countries.is_empty());
}
