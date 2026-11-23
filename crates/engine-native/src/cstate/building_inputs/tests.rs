use super::*;
use crate::{
    ccountry::synthetic::World,
    cstate::{BASE_MODIFIERS, CONTROLLER, COUNTRY_MODIFIER_STRIDE},
};
use engine_native_parts::modifier_entry;

const MODULE: usize = 0x1_4000_0000;
const KEYS: [u32; 8] = [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17];

fn keys() -> [Key; 8] {
    KEYS.map(|key| Some((key, modifier_definition::Operands::default())))
}

fn accumulator(world: &mut World, at: usize, entries: &[(u32, i64)]) {
    let data = world.alloc(entries.len().max(1) * modifier_entry::STRIDE);
    for (index, &(key, value)) in entries.iter().enumerate() {
        let entry = data + index * modifier_entry::STRIDE;
        world.u32(entry + modifier_entry::KEY, key);
        world.put(entry + modifier_entry::VALUE, &value.to_le_bytes());
    }
    world.array(at, data, entries.len() as u32);
}

fn state(
    world: &mut World,
    id: u32,
    [owner, controller]: [u32; 2],
    extra: i32,
    base: &[(u32, i64)],
    overrides: Option<(u32, &[(u32, i64)])>,
) -> usize {
    let state = world.alloc(0x900);
    world.u64(state, (MODULE + VTABLE_RVA.0) as u64);
    world.u32(state + ID, id);
    world.u32(state + OWNER, owner);
    world.u32(state + CONTROLLER, controller);
    world.put(state + EXTRA_SHARED_SLOTS, &extra.to_le_bytes());
    accumulator(world, state + BASE_MODIFIERS, base);
    let buckets = world.alloc(2 * COUNTRY_MODIFIER_STRIDE);
    world.u64(state + COUNTRY_MODIFIERS, buckets as u64);
    world.u32(state + COUNTRY_MODIFIERS + 12, 0);
    world.put(state + COUNTRY_MODIFIERS + 16, &[1]);
    if let Some((raw, entries)) = overrides {
        world.put(buckets + 4, &[1]);
        world.u32(buckets + 8, raw);
        accumulator(world, buckets + 0x20, entries);
    }
    state
}

fn country(world: &mut World, slot: u32, entries: &[(u32, i64)], factor: i64) {
    let object = world.object(slot);
    accumulator(world, object + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, entries);
    let status = world.alloc(0x600);
    world.u64(status, (MODULE + cproduction_status::VTABLE_RVA.0) as u64);
    world.put(
        status + cproduction_status::FACTORY_ENERGY_FACTOR,
        &factor.to_le_bytes(),
    );
    world.u64(object + ccountry::PRODUCTION_STATUS, status as u64);
}

fn world() -> World {
    let mut world = World::new(MODULE, 2, 0x1000);
    country(
        &mut world,
        1,
        &[(0x12, 200_000), (0x13, 20_000), (0x17, -10_000)],
        7_500,
    );
    country(&mut world, 2, &[(0x17, 5_000)], 12_000);
    let home = state(
        &mut world,
        0,
        [1, 1],
        2,
        &[
            (0x10, 300_000),
            (0x11, 50_000),
            (0x14, -20_000),
            (0x15, 10_000),
            (0x16, 1_000),
        ],
        None,
    );
    let occupied = state(
        &mut world,
        2,
        [1, 2],
        0,
        &[(0x10, 100_000)],
        Some((2, &[(0x14, -50_000)])),
    );
    let roster = world.alloc(3 * 8);
    world.u64(roster, home as u64);
    world.u64(roster + 16, occupied as u64);
    let game_state = world.game_state();
    world.array(game_state + cgame_state::STATES, roster, 3);
    world
}

fn rows(world: &World, keys: &[Key; 8]) -> Result<Vec<Row>, &'static str> {
    let mut output = Vec::new();
    let mut cache = Vec::new();
    let memory = world.memory();
    match walk_keys(
        &memory,
        MODULE,
        world.game_state(),
        keys,
        &mut cache,
        Buffer::Growing(&mut output),
    )? {
        Outcome::Rows { len } => {
            output.truncate(len);
            Ok(output)
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn each_state_reads_its_owners_and_controllers_operands() {
    let world = world();
    assert_eq!(
        rows(&world, &keys()).unwrap(),
        [
            Row {
                state: 0,
                owner: 1,
                controller: 1,
                extra_shared_slots: 2,
                owner_state: Some([300_000, 50_000]),
                owner_country: Some([200_000, 20_000]),
                controller_state: Some([-20_000, 10_000, 1_000]),
                controller_country: Some(-10_000),
                energy_factor: Some(7_500),
            },
            Row {
                state: 2,
                owner: 1,
                controller: 2,
                extra_shared_slots: 0,
                owner_state: Some([100_000, 0]),
                owner_country: Some([200_000, 20_000]),
                controller_state: Some([-50_000, 0, 0]),
                controller_country: Some(5_000),
                energy_factor: Some(12_000),
            },
        ]
    );
}

#[test]
fn an_unresolved_key_or_status_refuses_only_its_group() {
    let mut world = world();
    let mut keys = keys();
    keys[4] = None;
    keys[3] = None;
    let status = u64::from_le_bytes(
        world.bytes
            [world.object(2) + ccountry::PRODUCTION_STATUS - crate::ccountry::synthetic::START..]
            [..8]
            .try_into()
            .unwrap(),
    ) as usize;
    world.u64(status, 0);
    let rows = rows(&world, &keys).unwrap();
    assert_eq!(rows[0].controller_state, None);
    assert_eq!(rows[0].owner_country, None);
    assert_eq!(rows[0].owner_state, Some([300_000, 50_000]));
    assert_eq!(rows[0].controller_country, Some(-10_000));
    assert_eq!(rows[0].energy_factor, Some(7_500));
    assert_eq!(rows[1].energy_factor, None);
}

#[test]
fn a_foreign_roster_entry_refuses_the_table() {
    let mut world = world();
    let roster = u64::from_le_bytes(
        world.bytes[world.game_state() + cgame_state::STATES - crate::ccountry::synthetic::START..]
            [..8]
            .try_into()
            .unwrap(),
    ) as usize;
    let occupied = u64::from_le_bytes(
        world.bytes[roster + 16 - crate::ccountry::synthetic::START..][..8]
            .try_into()
            .unwrap(),
    ) as usize;
    world.u32(occupied + ID, 1);
    assert_eq!(
        rows(&world, &keys()),
        Err("state building input identity invalid")
    );
}
