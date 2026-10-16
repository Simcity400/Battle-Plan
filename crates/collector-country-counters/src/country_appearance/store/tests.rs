//! Carried CountryAppearance rows over the synthetic country census
//! ([`super::super::bench::World`]).
use super::super::bench::{ARRAYS, World, rows as published};
use super::*;
use crate::country_input_changes::Domain;
use engine::{
    ccountry, cdiplomacy_status, cgame_state, csubject_relation::Name, memory::SliceMemory,
    msvc_string,
};
use replay::rows::{CountryAppearanceRow, Recorded};
use std::cell::RefCell;

const COUNTRIES: usize = 48;
/// Hour 5's slice: countries 5 and 29 (`id % ROTATION == 5`).
const SLICE: Check = Check::Slice { hour: 5 };

/// Records every guarded read's address.
struct Traced<'a> {
    memory: SliceMemory<'a>,
    reads: RefCell<Vec<usize>>,
}

impl engine::Memory for Traced<'_> {
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

/// Country `id`'s object; the fixture's canonical slot is its id.
fn country(world: &World, id: u32) -> usize {
    word(world, world.origin + ARRAYS + id as usize * 8)
}

/// Countries 2, 10 and 30 name link 3's tag `XYZ` as their original tag, so
/// carried rows place original tag bytes between fresh ones.
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

/// The arenas a table is published from: each row's original tag span, the
/// naming bytes, the subject payloads and each row's payload start.
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

/// Today's full read of the same memory.
fn full(world: &World) -> (Vec<CountryAppearanceRow>, Layout) {
    let mut captured = Captured::with_capacities(&world.capacities());
    let rows = world.read(&mut captured);
    (rows, layout(&captured))
}

/// One hour's read with the store: the captured pack and the countries whose
/// appearance was read (their colour, the reader's first appearance field).
fn hour(
    world: &World,
    captured: &mut Captured,
    store: &mut AppearanceStore,
    domain: &Domain,
    check: Check,
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
            check,
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

/// A quiet hour reads no country's appearance but its slice's, and publishes
/// the rows, arenas and payloads a full read of the same memory does.
#[test]
fn a_quiet_hour_reads_only_its_slice_and_publishes_a_full_read() {
    let world = world();
    let domain = Domain::enabled(64);
    let mut store = AppearanceStore::default();
    let mut captured = Captured::with_capacities(&world.capacities());
    // Nothing held yet: every country is read.
    assert_eq!(
        hour(&world, &mut captured, &mut store, &domain, SLICE),
        every()
    );
    assert_eq!((published(&captured), layout(&captured)), full(&world));
    for _ in 0..2 {
        assert_eq!(
            hour(&world, &mut captured, &mut store, &domain, SLICE),
            [5, 29]
        );
        let rows = published(&captured);
        assert_eq!((rows.clone(), layout(&captured)), full(&world));
        // Carried rows keep their payload and original tag bytes.
        assert_eq!(rows[1].subjects, Recorded::Value(vec![2, -1, 2]));
        assert_eq!(rows[1].original_tag, Recorded::Value(Some(b"XYZ".to_vec())));
    }
    assert!(!domain.poisoned(Table::Appearance));
}

/// A marked country is read within the hour; a change written without a mark
/// is found by its slice, which publishes the fresh row and poisons the set,
/// so later hours read every country.
#[test]
fn a_marked_country_is_read_and_an_unmarked_change_poisons_the_set() {
    let mut world = world();
    let domain = Domain::enabled(64);
    let mut store = AppearanceStore::default();
    let mut captured = Captured::with_capacities(&world.capacities());
    hour(&world, &mut captured, &mut store, &domain, SLICE);
    let colour = country(&world, 3) + ccountry::CURRENT_PRIMARY_COLOR;
    put(&mut world, colour, &0x3f00_0000_u32.to_le_bytes());
    domain.write(Table::Appearance, 3);
    assert_eq!(
        hour(&world, &mut captured, &mut store, &domain, SLICE),
        [3, 5, 29]
    );
    let rows = published(&captured);
    assert_eq!(rows[2].red, 0x3f00_0000);
    assert_eq!((rows, layout(&captured)), full(&world));
    assert!(!domain.poisoned(Table::Appearance));

    // Country 29's subject roster changes with no mark: its slice reads it.
    let diplomacy = word(&world, country(&world, 29) + ccountry::DIPLOMACY_STATUS);
    let payload = word(&world, diplomacy + cdiplomacy_status::SUBJECTS);
    put(&mut world, payload, &77_i32.to_le_bytes());
    assert_eq!(
        hour(&world, &mut captured, &mut store, &domain, SLICE),
        [5, 29]
    );
    let rows = published(&captured);
    assert_eq!(rows[28].subjects, Recorded::Value(vec![77, -1, 29]));
    assert_eq!((rows, layout(&captured)), full(&world));
    assert!(domain.poisoned(Table::Appearance));
    assert_eq!(
        hour(&world, &mut captured, &mut store, &domain, SLICE),
        every()
    );
}

/// A failed read holds nothing, and a take whose epoch moved certifies
/// nothing: the next hour reads every country, and holds again.
#[test]
fn a_failed_read_or_an_uncertified_take_reads_every_country() {
    let mut world = world();
    let domain = Domain::enabled(64);
    let mut store = AppearanceStore::default();
    let mut captured = Captured::with_capacities(&world.capacities());
    hour(&world, &mut captured, &mut store, &domain, SLICE);
    // Country 7's diplomacy status fails its read: the hour is refused.
    let diplomacy = word(&world, country(&world, 7) + ccountry::DIPLOMACY_STATUS);
    let vtable = world.bytes[diplomacy - world.origin..][..8].to_vec();
    put(&mut world, diplomacy, &0_u64.to_le_bytes());
    domain.write(Table::Appearance, 7);
    hour(&world, &mut captured, &mut store, &domain, SLICE);
    assert!(matches!(
        captured.outcomes()[0],
        Outcome::Unavailable { .. }
    ));
    put(&mut world, diplomacy, &vtable);
    // The take is certified, but nothing is held.
    assert_eq!(
        hour(&world, &mut captured, &mut store, &domain, SLICE),
        every()
    );
    assert_eq!(
        hour(&world, &mut captured, &mut store, &domain, SLICE),
        [5, 29]
    );
    // A writer that names no country moves the epoch.
    domain.write_every(Table::Appearance);
    assert_eq!(
        hour(&world, &mut captured, &mut store, &domain, SLICE),
        every()
    );
    assert_eq!((published(&captured), layout(&captured)), full(&world));
    assert_eq!(
        hour(&world, &mut captured, &mut store, &domain, SLICE),
        [5, 29]
    );
    assert!(!domain.poisoned(Table::Appearance));
}
