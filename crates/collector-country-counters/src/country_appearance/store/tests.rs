use super::super::bench::{ARRAYS, World, rows as published};
use super::*;
use crate::country_input_changes::Domain;
use engine_native::{ccountry, cgame_state, crelation::subject::Name};
use engine_native_memory::{memory::SliceMemory, msvc_string};
use replay_data::{rows::CountryAppearanceRow, rows::Recorded};
use std::cell::RefCell;

const COUNTRIES: usize = 48;

struct Traced<'a> {
    memory: SliceMemory<'a>,
    reads: RefCell<Vec<usize>>,
}

impl engine_native_memory::Memory for Traced<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        self.memory.copy(address, destination)
    }
}

fn put(world: &mut World, address: usize, value: &[u8]) {
    let at = address - world.origin;
    world.bytes[at..at + value.len()].copy_from_slice(value);
}

fn word(world: &World, address: usize) -> usize {
    let at = address - world.origin;
    u64::from_le_bytes(world.bytes[at..at + 8].try_into().unwrap()) as usize
}

fn country(world: &World, id: u32) -> usize {
    word(world, world.origin + ARRAYS + id as usize * 8)
}

fn world() -> World {
    let mut world = World::new(COUNTRIES, true);
    let tags = word(&world, world.state() + cgame_state::COUNTRY_TAGS);
    let tag = tags + 3 * msvc_string::STRIDE;
    put(&mut world, tag, b"XYZ");
    put(&mut world, tag + msvc_string::LENGTH, &3_u64.to_le_bytes());
    for id in [2, 10, 30] {
        let at = country(&world, id) + ccountry::ORIGINAL_TAG;
        put(&mut world, at, &3_i32.to_le_bytes());
    }
    world
}

type Layout = (Vec<Option<Name>>, Vec<u8>, Vec<u32>, Vec<usize>);

fn layout(captured: &Captured) -> Layout {
    let len = captured.rows.outcome.len();
    (
        captured.rows.rows[..len]
            .iter()
            .map(|row| row.original_tag)
            .collect(),
        captured.naming.first[..captured.naming.len].to_vec(),
        captured.subjects.first[..captured.subjects.len].to_vec(),
        captured.subject_starts[..len].to_vec(),
    )
}

fn full(world: &World) -> (Vec<CountryAppearanceRow>, Layout) {
    let mut captured = Captured::with_capacities(&world.capacities());
    let rows = world.read(&mut captured);
    (rows, layout(&captured))
}

fn hour(
    world: &World,
    captured: &mut Captured,
    store: &mut AppearanceStore,
    domain: &Domain,
) -> Vec<u32> {
    let memory = Traced {
        memory: SliceMemory {
            base: world.origin,
            bytes: &world.bytes,
        },
        reads: RefCell::default(),
    };
    captured.reset();
    captured.capture_preacquired(
        &memory,
        world.module,
        world.state(),
        0,
        Some(Carry {
            store,
            changes: domain,
        }),
    );
    let reads = memory.reads.borrow();
    (1..=COUNTRIES as u32)
        .filter(|&id| reads.contains(&(country(world, id) + ccountry::CURRENT_PRIMARY_COLOR)))
        .collect()
}

fn every() -> Vec<u32> {
    (1..=COUNTRIES as u32).collect()
}

#[test]
fn a_quiet_hour_reads_no_country_and_publishes_a_full_read() {
    let world = world();
    let domain = Domain::enabled(64);
    let mut store = AppearanceStore::default();
    let mut captured = Captured::with_capacities(&world.capacities());
    assert_eq!(
        hour(&world, &mut captured, &mut store, &domain),
        every()
    );
    assert_eq!((published(&captured), layout(&captured)), full(&world));
    for _ in 0..2 {
        assert!(hour(&world, &mut captured, &mut store, &domain).is_empty());
        let rows = published(&captured);
        assert_eq!((rows.clone(), layout(&captured)), full(&world));
        assert_eq!(rows[1].subjects, Recorded::Value(vec![2, -1, 2]));
        assert_eq!(rows[1].original_tag, Recorded::Value(Some(b"XYZ".to_vec())));
    }
}

#[test]
fn a_marked_country_is_read_within_the_hour() {
    let mut world = world();
    let domain = Domain::enabled(64);
    let mut store = AppearanceStore::default();
    let mut captured = Captured::with_capacities(&world.capacities());
    hour(&world, &mut captured, &mut store, &domain);
    let colour = country(&world, 3) + ccountry::CURRENT_PRIMARY_COLOR;
    put(&mut world, colour, &0x3f00_0000_u32.to_le_bytes());
    domain.write(Table::Appearance, 3);
    assert_eq!(hour(&world, &mut captured, &mut store, &domain), [3]);
    let rows = published(&captured);
    assert_eq!(rows[2].red, 0x3f00_0000);
    assert_eq!((rows, layout(&captured)), full(&world));
    assert!(hour(&world, &mut captured, &mut store, &domain).is_empty());
    let rows = published(&captured);
    assert_eq!(rows[2].red, 0x3f00_0000);
    assert_eq!((rows, layout(&captured)), full(&world));
}

#[test]
fn a_failed_read_or_an_uncertified_take_reads_every_country() {
    let mut world = world();
    let domain = Domain::enabled(64);
    let mut store = AppearanceStore::default();
    let mut captured = Captured::with_capacities(&world.capacities());
    hour(&world, &mut captured, &mut store, &domain);
    let diplomacy = word(&world, country(&world, 7) + ccountry::DIPLOMACY_STATUS);
    let vtable = world.bytes[diplomacy - world.origin..][..8].to_vec();
    put(&mut world, diplomacy, &0_u64.to_le_bytes());
    domain.write(Table::Appearance, 7);
    hour(&world, &mut captured, &mut store, &domain);
    assert!(matches!(
        captured.outcomes()[0],
        Outcome::Unavailable { .. }
    ));
    put(&mut world, diplomacy, &vtable);
    assert_eq!(hour(&world, &mut captured, &mut store, &domain), every());
    assert!(hour(&world, &mut captured, &mut store, &domain).is_empty());
    domain.write_every(Table::Appearance);
    assert_eq!(hour(&world, &mut captured, &mut store, &domain), every());
    assert_eq!((published(&captured), layout(&captured)), full(&world));
    assert!(hour(&world, &mut captured, &mut store, &domain).is_empty());
}
