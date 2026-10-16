use super::*;
use crate::{country_input_changes::Domain, reserved_visibility};
use engine::{
    ccountry_characters, cdiplomacy_status as diplomacy, cgame_state, cname_group_member,
    creference_registry, crelation_status, cship, memory::SliceMemory, reference_bucket as bucket,
};
use replay::frame::TableView;
use std::cell::RefCell;

const STATE: usize = 0x1000;
const COUNTRY_DATA: usize = 0x2000;
const LINK_DATA: usize = 0x10000;
const BLOCKS: usize = 0x20000;
/// One country per block: the object (its civil-war header at `+0x12F0`),
/// then its diplomacy, its relation matrix (slot zero only), its status to
/// zero, its neighbour data and characters object, and last the reserved
/// country's status to it.
const BLOCK: usize = 0x2000;
const DIPLOMACY: usize = 0x1300;
const MATRIX: usize = 0x1660;
const FORWARD: usize = 0x1680;
const NEIGHBOR_DATA: usize = 0x19a0;
const CHARACTERS: usize = 0x1a00;
const REVERSE: usize = 0x1b00;
const PRIDE_HANDLE: u64 = (91_u64 << 32) | cship::HANDLE_TYPE as u64;

fn block(slot: usize) -> usize {
    BLOCKS + slot * BLOCK
}

/// `countries` positive countries after the reserved one at slot zero, each
/// with a readable counter row and relation suffix, odd ones co-belligerent
/// with zero and each with `slot % 4` neighbours.
struct World {
    bytes: Vec<u8>,
    pride: usize,
}

impl World {
    fn new(countries: usize) -> Self {
        let reverse_matrix = block(countries + 1);
        let pride = (reverse_matrix + 8 * (countries + 1) + 0xfff) & !0xfff;
        let mut world = Self {
            bytes: vec![0; pride + 0x5000],
            pride,
        };
        let slots = countries as u32 + 1;
        world.header(STATE + cgame_state::COUNTRIES, COUNTRY_DATA, slots);
        world.header(STATE + cgame_state::COUNTRY_LINKS, LINK_DATA, slots);
        for slot in 0..=countries {
            let country = block(slot);
            let root = country + DIPLOMACY;
            world.put64(COUNTRY_DATA + slot * 8, country as u64);
            world.put32(LINK_DATA + slot * 4, slot as u32);
            world.put64(country, ccountry::VTABLE_RVA as u64);
            world.put32(country + ccountry::ID, slot as u32);
            world.put64(country + ccountry::DIPLOMACY_STATUS, root as u64);
            world.put64(root, diplomacy::VTABLE_RVA as u64);
            world.put64(root + diplomacy::COUNTRY, country as u64);
            let (matrix, cells) = if slot == 0 {
                (reverse_matrix, countries + 1)
            } else {
                (country + MATRIX, 1)
            };
            world.header(root + diplomacy::RELATIONS, matrix, cells as u32);
            for target in 0..cells {
                let status = if slot == 0 {
                    block(target) + REVERSE
                } else {
                    country + FORWARD
                };
                world.put64(matrix + target * 8, status as u64);
                world.put64(status, crelation_status::VTABLE_RVA as u64);
                world.put64(status + crelation_status::DIPLOMACY_STATUS, root as u64);
                world.put32(status + crelation_status::TARGET_COUNTRY, target as u32);
            }
            if slot != 0 {
                world.bytes[country + FORWARD + crelation_status::CO_BELLIGERENT] =
                    (slot % 2) as u8;
                let neighbors: Vec<u32> = (0..slot % 4).map(|n| (slot * 10 + n) as u32).collect();
                world.neighbors(slot, &neighbors);
            }
        }
        world
    }

    fn put32(&mut self, at: usize, value: u32) {
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn header(&mut self, at: usize, data: usize, count: u32) {
        self.put64(at, data as u64);
        self.put32(at + 8, count);
        self.put32(at + 12, count);
    }

    /// Rewrites the country's neighbour roster (the neighbour rebuild's).
    fn neighbors(&mut self, slot: usize, values: &[u32]) {
        let data = block(slot) + NEIGHBOR_DATA;
        self.header(block(slot) + ccountry::NEIGHBORS, data, values.len() as u32);
        for (index, value) in values.iter().enumerate() {
            self.put32(data + index * 4, *value);
        }
    }

    /// Installs a CCountryCharacters object (no leaders) at `+0xFF0`.
    fn characters(&mut self, slot: usize) {
        let object = block(slot) + CHARACTERS;
        self.put64(block(slot) + ccountry::CHARACTERS, object as u64);
        self.put64(object, ccountry_characters::VTABLE_RVA as u64);
        self.put64(object + ccountry_characters::OWNER, block(slot) as u64);
    }

    /// The country's pride of the fleet: a registered ship named `text`.
    fn pride(&mut self, slot: usize, text: &[u8]) {
        let ship = self.pride;
        let member = ship + 0x1000;
        let table = ship + 0x2000;
        let entries = ship + 0x3000;
        let heap = ship + 0x4000;
        self.put64(block(slot) + ccountry::PRIDE_OF_THE_FLEET, PRIDE_HANDLE);
        self.put64(ship, cship::VTABLE_RVA as u64);
        self.put64(ship + cship::HANDLE, PRIDE_HANDLE);
        self.put64(ship + cship::NAME_MEMBER, member as u64);
        self.put64(member, cname_group_member::VTABLE_RVA as u64);
        let string = member + cname_group_member::CACHED_NAME;
        self.put64(string + 16, text.len() as u64);
        if text.len() <= 15 {
            self.bytes[string..string + text.len()].copy_from_slice(text);
            self.put64(string + 24, 15);
        } else {
            self.put64(string, heap as u64);
            self.put64(string + 24, text.len() as u64);
            self.bytes[heap..heap + text.len()].copy_from_slice(text);
        }
        self.put64(table + creference_registry::ENTRIES, entries as u64);
        self.put32(table + creference_registry::COUNT, 1);
        self.put32(table + creference_registry::MASK, 7);
        self.bytes[table + creference_registry::OVERFLOW_DISTANCE] = 5;
        self.bytes[entries + 13 * bucket::STRIDE + bucket::DISTANCE] = 255;
        let hash = PRIDE_HANDLE
            .rotate_left(32)
            .wrapping_mul(creference_registry::HASH_MULTIPLIER);
        let first = ((hash >> 32) as u32 ^ hash as u32) & 7;
        let at = entries + first as usize * bucket::STRIDE;
        self.bytes[at + bucket::DISTANCE] = 1;
        self.put64(at + bucket::TYPE, PRIDE_HANDLE);
        self.put64(at + bucket::OBJECT, ship as u64);
    }

    fn memory(&self, fault: Option<usize>) -> Probe<'_> {
        Probe {
            bytes: &self.bytes,
            pride_table: (self.pride + 0x2000) as u64,
            fault,
            reads: RefCell::default(),
        }
    }
}

/// Records every read; the reference registry's ship root names the pride
/// table, and the reserved country is authenticated as the callback's.
struct Probe<'a> {
    bytes: &'a [u8],
    pride_table: u64,
    fault: Option<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
}

impl Memory for Probe<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if self
            .fault
            .is_some_and(|at| address <= at && at < address.saturating_add(destination.len()))
        {
            return false;
        }
        let root = creference_registry::LOW_ROOTS_RVA + cship::HANDLE_TYPE as usize * 8;
        if address == root && destination.len() == 8 {
            destination.copy_from_slice(&self.pride_table.to_le_bytes());
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<ccountry::Country, &'static str>> {
        (base == 0 && state == STATE).then_some(Ok(ccountry::Country {
            pointer: block(0),
            canonical_slot: 0,
            id: 0,
        }))
    }
}

fn domain(ids: usize) -> &'static Domain {
    Box::leak(Box::new(Domain::enabled(ids)))
}

/// One hour's counter pass with the reserved country's reverse visibility.
struct Hour {
    captured: Captured,
    reverse: Option<replay::rows::ReservedReverseVisibilityInputs>,
    /// The counter pass's reads, after the naming observation.
    reads: Vec<(usize, usize)>,
}

fn hour(world: &World, store: Option<&mut CounterStore>, at: i32, fault: Option<usize>) -> Hour {
    let memory = world.memory(fault);
    let mut naming = ccountry::read_reserved_naming(&memory, 0, STATE, &mut []);
    let observed = naming
        .visibility
        .take()
        .expect("completed controls survive naming refusal");
    naming.launch_states = None;
    memory.reads.borrow_mut().clear();
    let mut captured = Captured::vacant();
    let mut reverse = reserved_visibility::Captured::new(0);
    captured.capture_carried(
        &memory,
        0,
        STATE,
        Some(reserved_visibility::Request {
            hour: Some(at),
            observation: Some(observed),
            prefix_ready: true,
            capture: &mut reverse,
        }),
        store,
    );
    Hour {
        reverse: reverse.projection().unwrap(),
        captured,
        reads: memory.reads.into_inner(),
    }
}

type Arenas = (
    Vec<u32>,
    Vec<u64>,
    Vec<u32>,
    Vec<u32>,
    Vec<u32>,
    Vec<u8>,
    [usize; 7],
);

fn arenas(captured: &Captured) -> Arenas {
    (
        captured.states.first[..captured.states.len].to_vec(),
        captured.leaders.first[..captured.leaders.len].to_vec(),
        captured.neighbors.first[..captured.neighbors.len].to_vec(),
        captured.policies.first[..captured.policies.len].to_vec(),
        captured.co_belligerents.first[..captured.co_belligerents.len].to_vec(),
        captured.pride_names.first[..captured.pride_names.len].to_vec(),
        [
            captured.states.required,
            captured.leaders.required,
            captured.neighbors.required,
            captured.policies.required,
            captured.co_belligerents.required,
            captured.pride_names.required,
            captured.relations.len,
        ],
    )
}

/// The same published tables, reverse cells and arenas.
fn assert_same(carried: &Hour, full: &Hour) {
    assert_eq!(carried.captured.tables(), full.captured.tables());
    assert_eq!(carried.reverse, full.reverse);
    assert_eq!(arenas(&carried.captured), arenas(&full.captured));
}

/// The slots whose own fields, relation status to zero or roster data the
/// hour read beyond the identity checks, the carry's pointer words and the
/// civil-war header.
fn read_countries(reads: &[(usize, usize)], countries: usize) -> Vec<usize> {
    // CountryCivilWars still reads every country's civil-war header.
    let allowed = [
        (0, 8),
        (ccountry::ID, 4),
        (WORDS, WORDS_LEN),
        (ccountry::CIVIL_WAR_TARGETS, 16),
    ];
    let mut slots: Vec<usize> = reads
        .iter()
        .filter_map(|&(address, len)| {
            let offset = address.checked_sub(block(1))?;
            let (slot, within) = (offset / BLOCK + 1, offset % BLOCK);
            (slot <= countries && within < REVERSE && !allowed.contains(&(within, len)))
                .then_some(slot)
        })
        .collect();
    slots.sort_unstable();
    slots.dedup();
    slots
}

fn quiet() -> Counts {
    Counts::default()
}

/// A quiet hour reads no country's counter fields, rosters or relation
/// suffix, only each one's identity and carry words, its census resolution,
/// its reverse cell and the pride ship name, and publishes what a full read
/// of the same memory publishes.
#[test]
fn a_quiet_hour_reads_no_carried_country_and_equals_a_full_read() {
    let mut world = World::new(6);
    world.pride(2, b"Pride of a fleet with a long name");
    world.characters(3);
    let domain = domain(64);
    let mut store = CounterStore::new(domain).with_check(Check::Nothing);
    let first = hour(&world, Some(&mut store), 10, None);
    assert_eq!(
        store.counts(),
        Counts {
            read: 6,
            full: true,
            ..quiet()
        }
    );
    assert_eq!(read_countries(&first.reads, 6), [1, 2, 3, 4, 5, 6]);
    domain.boundary(false);
    let carried = hour(&world, Some(&mut store), 11, None);
    let full = hour(&world, None, 11, None);
    assert_same(&carried, &full);
    assert_same(&first, &full);
    assert_eq!(store.counts(), quiet());
    assert!(read_countries(&carried.reads, 6).is_empty());
    assert_eq!(read_countries(&full.reads, 6), [1, 2, 3, 4, 5, 6]);
    // The pride ship name is read every hour, carried or not.
    assert!(
        carried
            .reads
            .iter()
            .any(|&(address, _)| (world.pride..world.pride + 0x5000).contains(&address))
    );
    let TableView::Complete(replay::rows::TableRows::CountryCounterInputs(rows)) =
        &carried.captured.tables()[0]
    else {
        panic!("carried table refused");
    };
    assert_eq!(
        rows[1].pride_name,
        replay::rows::Recorded::Value(Some(b"Pride of a fleet with a long name".to_vec()))
    );
    assert_eq!(
        rows[4].neighbors,
        replay::rows::Recorded::Value(Some(vec![50]))
    );
    assert_eq!(
        rows[4].co_belligerents,
        replay::rows::Recorded::Value(Some(vec![0]))
    );
    assert!(full.reverse.is_some());
}

/// A marked country is read afresh within the hour; a change no writer
/// marked is carried until the hour's slice reads it, which poisons the set.
#[test]
fn marked_countries_are_read_and_the_slice_finds_an_unmarked_change() {
    let mut world = World::new(4);
    let domain = domain(64);
    let mut store = CounterStore::new(domain).with_check(Check::Nothing);
    hour(&world, Some(&mut store), 10, None);
    world.neighbors(2, &[7, 8, 9]);
    domain.write(Table::Counters, 2);
    domain.boundary(false);
    let marked = hour(&world, Some(&mut store), 11, None);
    assert_same(&marked, &hour(&world, None, 11, None));
    assert_eq!(read_countries(&marked.reads, 4), [2]);
    assert_eq!(
        store.counts(),
        Counts {
            marked: 1,
            read: 1,
            ..quiet()
        }
    );
    // The slice reads country 1 afresh and finds its held row current.
    store.check = Some(Check::Slice { hour: 1 });
    domain.boundary(false);
    let sliced = hour(&world, Some(&mut store), 12, None);
    assert_eq!(read_countries(&sliced.reads, 4), [1]);
    assert_eq!(
        store.counts(),
        Counts {
            read: 1,
            verified: 1,
            ..quiet()
        }
    );
    assert!(!domain.poisoned(Table::Counters));
    // An unmarked write is carried until the slice covers the country.
    world.neighbors(3, &[5]);
    store.check = Some(Check::Nothing);
    domain.boundary(false);
    let stale = hour(&world, Some(&mut store), 13, None);
    assert_ne!(
        stale.captured.tables(),
        hour(&world, None, 13, None).captured.tables()
    );
    store.check = Some(Check::Slice { hour: 3 });
    domain.boundary(false);
    let found = hour(&world, Some(&mut store), 14, None);
    assert_same(&found, &hour(&world, None, 14, None));
    assert_eq!(
        store.counts(),
        Counts {
            read: 1,
            verified: 1,
            mismatches: 1,
            ..quiet()
        }
    );
    assert!(domain.poisoned(Table::Counters));
    let mismatch = domain.mismatches(Table::Counters).first.unwrap();
    assert_eq!((mismatch.country, mismatch.field), (3, "neighbors"));
    // A poisoned set certifies nothing: every country is read.
    domain.boundary(false);
    let after = hour(&world, Some(&mut store), 15, None);
    assert_eq!(read_countries(&after.reads, 4), [1, 2, 3, 4]);
    assert!(store.counts().full);
}

/// A country whose CCountryCharacters pointer moved is read, though no
/// writer marked it.
#[test]
fn a_moved_characters_pointer_reads_the_country() {
    let mut world = World::new(4);
    let domain = domain(64);
    let mut store = CounterStore::new(domain).with_check(Check::Nothing);
    hour(&world, Some(&mut store), 10, None);
    world.characters(2);
    domain.boundary(false);
    let moved = hour(&world, Some(&mut store), 11, None);
    assert_same(&moved, &hour(&world, None, 11, None));
    assert_eq!(read_countries(&moved.reads, 4), [2]);
    assert_eq!(store.counts(), Counts { read: 1, ..quiet() });
    // Held again with the new pointer, it is carried.
    domain.boundary(false);
    let held = hour(&world, Some(&mut store), 12, None);
    assert!(read_countries(&held.reads, 4).is_empty());
}

/// A failed walk holds nothing, and a take that certifies nothing (a new
/// epoch, or a writer naming no country) reads every country.
#[test]
fn a_failed_read_holds_nothing_and_an_uncertified_take_reads_every_country() {
    let world = World::new(4);
    let domain = domain(64);
    let mut store = CounterStore::new(domain).with_check(Check::Nothing);
    let failed = hour(
        &world,
        Some(&mut store),
        10,
        Some(block(2) + ccountry::PRIDE_OF_THE_FLEET),
    );
    assert!(matches!(
        failed.captured.tables()[0],
        TableView::Unavailable { .. }
    ));
    assert_eq!(store.held.epoch, None);
    domain.boundary(false);
    let next = hour(&world, Some(&mut store), 11, None);
    assert_eq!(read_countries(&next.reads, 4), [1, 2, 3, 4]);
    assert!(store.counts().full);
    domain.boundary(true);
    let invalidated = hour(&world, Some(&mut store), 12, None);
    assert_eq!(read_countries(&invalidated.reads, 4), [1, 2, 3, 4]);
    assert!(store.counts().full);
    domain.boundary(false);
    assert!(read_countries(&hour(&world, Some(&mut store), 13, None).reads, 4).is_empty());
    domain.write_every(Table::Counters);
    domain.boundary(false);
    let every = hour(&world, Some(&mut store), 14, None);
    assert_eq!(read_countries(&every.reads, 4), [1, 2, 3, 4]);
    // A forgotten store (a failed job) reads every country too.
    domain.boundary(false);
    assert!(read_countries(&hour(&world, Some(&mut store), 15, None).reads, 4).is_empty());
    store.forget();
    domain.boundary(false);
    let forgotten = hour(&world, Some(&mut store), 16, None);
    assert_eq!(read_countries(&forgotten.reads, 4), [1, 2, 3, 4]);
}

/// 5,000 countries: the quiet hour reads only identities and carry words,
/// and publishes the full read's table and arenas.
#[test]
fn five_thousand_country_quiet_hour_carries_every_row() {
    const COUNTRIES: usize = 5_000;
    let world = World::new(COUNTRIES);
    let domain = domain(COUNTRIES + 1);
    let mut store = CounterStore::new(domain).with_check(Check::Nothing);
    hour(&world, Some(&mut store), 10, None);
    domain.boundary(false);
    let carried = hour(&world, Some(&mut store), 11, None);
    assert_eq!(store.counts(), quiet());
    assert!(read_countries(&carried.reads, COUNTRIES).is_empty());
    assert_same(&carried, &hour(&world, None, 11, None));
}
