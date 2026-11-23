#![cfg(feature = "visibility_changes")]
use super::*;
use crate::country_input_changes::Domain;
use engine_native::{ccountry, cgame_state};
use engine_native_parts::modifier_definition as definition;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

const STATE: usize = 0x1000;
const COUNTRIES: usize = 0x10_0000;
const LINKS: usize = 0x20_0000;
const OBJECTS: usize = 0x30_0000;
const ENTRIES: usize = 0x800_0000;
const DEFINITIONS: usize = 0x1000_0000;
fn object(slot: usize) -> usize {
    OBJECTS + slot * 0x1000
}
fn metadata() -> usize {
    DEFINITIONS + native::UNIT_MODIFIER as usize * definition::STRIDE
}

struct World {
    spans: BTreeMap<usize, Vec<u8>>,
}
impl World {
    fn new(countries: usize) -> Self {
        let mut world = Self {
            spans: BTreeMap::new(),
        };
        world.header(
            STATE + cgame_state::COUNTRIES,
            COUNTRIES,
            countries as u32 + 1,
        );
        world.header(
            STATE + cgame_state::COUNTRY_LINKS,
            LINKS,
            countries as u32 + 1,
        );
        let mut pointers = vec![0; (countries + 1) * 8];
        let mut links = vec![0; (countries + 1) * 4];
        for slot in 1..=countries {
            pointers[slot * 8..slot * 8 + 8].copy_from_slice(&(object(slot) as u64).to_le_bytes());
            links[slot * 4..slot * 4 + 4].copy_from_slice(&(slot as u32).to_le_bytes());
            world.put(object(slot), &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
            world.put(object(slot) + ccountry::ID, &(slot as u32).to_le_bytes());
            world.raw(
                slot,
                if slot % 3 == 0 {
                    None
                } else {
                    Some(slot as i64 * 1000)
                },
            );
        }
        world.spans.insert(COUNTRIES, pointers);
        world.spans.insert(LINKS, links);
        world.put(definition::TABLE_RVA.0, &(DEFINITIONS as u64).to_le_bytes());
        world.spans.insert(metadata(), vec![0; definition::STRIDE]);
        world
    }
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.spans.insert(address, bytes.to_vec());
    }
    fn header(&mut self, address: usize, data: usize, count: u32) {
        let mut bytes = (data as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        self.spans.insert(address, bytes);
    }
    fn raw(&mut self, slot: usize, raw: Option<i64>) {
        let data = ENTRIES + slot * 0x100;
        self.header(
            object(slot) + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
            data,
            u32::from(raw.is_some()),
        );
        if let Some(raw) = raw {
            let mut entry = vec![0; 16];
            entry[..4].copy_from_slice(&native::UNIT_MODIFIER.to_le_bytes());
            entry[8..].copy_from_slice(&raw.to_le_bytes());
            self.spans.insert(data, entry);
        }
    }
    fn flags(&mut self, flags: u32, selector: u32) {
        let row = self.spans.get_mut(&metadata()).unwrap();
        row[definition::FLAGS..definition::FLAGS + 4].copy_from_slice(&flags.to_le_bytes());
        row[definition::FEATURE_SELECTOR..definition::FEATURE_SELECTOR + 4]
            .copy_from_slice(&selector.to_le_bytes());
    }
}
impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let Some((&start, bytes)) = self.spans.range(..=address).next_back() else {
            return false;
        };
        let offset = address - start;
        let Some(end) = offset.checked_add(output.len()) else {
            return false;
        };
        let Some(bytes) = bytes.get(offset..end) else {
            return false;
        };
        output.copy_from_slice(bytes);
        true
    }
}

struct Counted<'a> {
    world: &'a World,
    reads: Cell<u64>,
    trace: RefCell<Vec<usize>>,
    fault: Cell<Option<usize>>,
}
impl<'a> Counted<'a> {
    fn new(world: &'a World) -> Self {
        Self {
            world,
            reads: Cell::new(0),
            trace: RefCell::new(Vec::new()),
            fault: Cell::new(None),
        }
    }
}
impl Memory for Counted<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.trace.borrow_mut().push(address);
        if output.len() == 16
            && address
                .checked_sub(OBJECTS + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
                .is_some_and(|offset| offset.is_multiple_of(0x1000))
        {
            self.reads.set(self.reads.get() + 1);
        }
        if self.fault.get() == Some(address) {
            self.fault.set(None);
            return false;
        }
        self.world.copy(address, output)
    }
}

fn capture(
    store: &mut Store,
    world: &World,
    domain: &Domain,
) -> (Vec<native::CountryRow>, Counts, u64, u64) {
    let memory = Counted::new(world);
    let mut rows = Vec::new();
    let ((outcome, _), counts, carried) = store.capture(
        &memory,
        0,
        STATE,
        Buffer::Growing(&mut rows),
        domain,
        false,
    );
    assert!(matches!(outcome, Outcome::Rows { .. }), "{outcome:?}");
    rows.truncate(outcome.len());
    (rows, counts, carried, memory.reads.get())
}
fn full(world: &World) -> Vec<native::CountryRow> {
    let mut rows = Vec::new();
    let (outcome, _) = native::countries(world, 0, STATE, &mut rows);
    assert!(matches!(outcome, Outcome::Rows { .. }), "{outcome:?}");
    rows.truncate(outcome.len());
    rows
}

#[test]
fn quiet_certified_hour_reads_no_accumulator_and_equals_full_read() {
    let world = World::new(5);
    let domain = Domain::enabled(6);
    let mut store = Store::new();
    assert_eq!(capture(&mut store, &world, &domain).3, 5);
    domain.boundary(false);
    let (rows, counts, carried, reads) = capture(&mut store, &world, &domain);
    assert_eq!(rows, full(&world));
    assert_eq!(reads, 0);
    assert_eq!(counts.read, 0);
    assert!(!counts.full);
    assert_eq!(carried, 5);
}

#[test]
fn a_marked_country_reads_its_accumulator_and_publishes_its_new_value() {
    let mut world = World::new(3);
    let domain = Domain::enabled(4);
    let mut store = Store::new();
    capture(&mut store, &world, &domain);
    world.raw(2, Some(-700));
    domain.write(Table::Visibility, 2);
    domain.boundary(false);
    let (rows, counts, carried, reads) = capture(&mut store, &world, &domain);
    assert_eq!((reads, counts.read, counts.marked, carried), (1, 1, 1, 2));
    assert_eq!(rows, full(&world));
}

#[test]
fn moved_epoch_and_uncertified_takes_read_every_country() {
    let world = World::new(3);
    let domain = Domain::enabled(4);
    let mut store = Store::new();
    capture(&mut store, &world, &domain);
    domain.boundary(true);
    let (_, counts, carried, reads) = capture(&mut store, &world, &domain);
    assert_eq!((counts.full, carried, reads), (true, 0, 3));
    domain.write_every(Table::Visibility);
    domain.boundary(false);
    let (rows, counts, carried, reads) = capture(&mut store, &world, &domain);
    assert_eq!(rows, full(&world));
    assert_eq!((counts.full, carried, reads), (true, 0, 3));
    domain.boundary(false);
    let (rows, counts, carried, reads) = capture(&mut store, &world, &domain);
    assert_eq!(rows, full(&world));
    assert_eq!((counts.full, carried, reads), (false, 3, 0));
}

#[test]
fn effective_definition_changes_apply_without_an_accumulator_mark_or_read() {
    let mut world = World::new(1);
    world.raw(1, Some(200_000));
    let domain = Domain::enabled(2);
    let mut store = Store::new();
    capture(&mut store, &world, &domain);
    for (flags, selector, expected) in [
        (definition::CLAMP_MAXIMUM, 0, Some(100_000)),
        (0, 0, Some(200_000)),
        (0, 1, None),
    ] {
        world.flags(flags, selector);
        world.put(definition::ELIGIBILITY_CALLBACK_RVA.0, &0_u64.to_le_bytes());
        domain.boundary(false);
        let (rows, counts, carried, reads) = capture(&mut store, &world, &domain);
        assert_eq!(rows, full(&world));
        assert_eq!((reads, carried, counts.read), (0, 1, 0));
        assert_eq!(rows.first().map(|row| row.unit_modifier), expected);
    }
}

#[test]
fn failed_definition_and_incomplete_output_forget_every_held_country() {
    let world = World::new(3);
    let domain = Domain::enabled(4);
    let mut store = Store::new();
    capture(&mut store, &world, &domain);
    domain.boundary(false);
    let memory = Counted::new(&world);
    memory
        .fault
        .set(Some(metadata() + definition::FEATURE_SELECTOR));
    let mut rows = Vec::new();
    let ((outcome, _), _, _) = store.capture(
        &memory,
        0,
        STATE,
        Buffer::Growing(&mut rows),
        &domain,
        false,
    );
    assert!(matches!(outcome, Outcome::Unavailable { .. }));
    assert!(store.epoch.is_none());
    domain.boundary(false);
    assert_eq!(capture(&mut store, &world, &domain).3, 3);
    domain.boundary(false);
    let ((outcome, _), _, _) = store.capture(
        &world,
        0,
        STATE,
        Buffer::Count,
        &domain,
        false,
    );
    assert!(matches!(outcome, Outcome::Truncated { .. }));
    assert!(store.epoch.is_none());
    domain.boundary(false);
    assert_eq!(capture(&mut store, &world, &domain).3, 3);
}

#[test]
fn a_replaced_census_object_and_forced_refresh_read_again() {
    let mut world = World::new(1);
    let domain = Domain::enabled(2);
    let mut store = Store::new();
    capture(&mut store, &world, &domain);
    let replaced = object(20);
    for offset in [0, ccountry::ID, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR] {
        let bytes = world.spans.get(&(object(1) + offset)).unwrap().clone();
        world.spans.insert(replaced + offset, bytes);
    }
    world.spans.get_mut(&COUNTRIES).unwrap()[8..16]
        .copy_from_slice(&(replaced as u64).to_le_bytes());
    domain.boundary(false);
    let (rows, counts, carried, reads) = capture(&mut store, &world, &domain);
    assert_eq!(rows, full(&world));
    assert_eq!((counts.read, carried), (1, 0));
    assert_eq!(reads, 1);
    domain.boundary(false);
    let mut rows = Vec::new();
    let (_, counts, carried) = store.capture(
        &world,
        0,
        STATE,
        Buffer::Growing(&mut rows),
        &domain,
        true,
    );
    assert_eq!((counts.full, counts.read, carried), (true, 1, 0));
}

#[test]
fn a_full_read_keeps_metadata_before_the_next_country_and_does_not_retry_a_one_shot_fault() {
    let world = World::new(2);
    let domain = Domain::enabled(3);
    let mut store = Store::new();
    let memory = Counted::new(&world);
    let mut rows = Vec::new();
    store.capture(
        &memory,
        0,
        STATE,
        Buffer::Growing(&mut rows),
        &domain,
        false,
    );
    let trace = memory.trace.borrow();
    let first = trace
        .iter()
        .position(|&address| address == object(1) + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
        .unwrap();
    let definition = trace
        .iter()
        .position(|&address| address == metadata() + definition::FEATURE_SELECTOR)
        .unwrap();
    let second = trace
        .iter()
        .position(|&address| address == object(2) + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
        .unwrap();
    assert!(first < definition && definition < second);
    drop(trace);
    domain.boundary(true);
    let failed = object(1) + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
    memory.fault.set(Some(failed));
    memory.trace.borrow_mut().clear();
    let ((outcome, _), _, _) = store.capture(
        &memory,
        0,
        STATE,
        Buffer::Growing(&mut rows),
        &domain,
        false,
    );
    let (expected, _) = native::countries(
        &{
            let same_fault = Counted::new(&world);
            same_fault.fault.set(Some(failed));
            same_fault
        },
        0,
        STATE,
        &mut Vec::new(),
    );
    assert_eq!(outcome, expected);
    assert_eq!(
        memory
            .trace
            .borrow()
            .iter()
            .filter(|&&address| address == failed)
            .count(),
        1
    );
    assert!(
        !memory
            .trace
            .borrow()
            .contains(&(object(2) + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR))
    );
    assert!(store.epoch.is_none());
}

#[test]
fn advisory_store_allocation_failure_publishes_exact_rows_and_forgets_consumed_marks() {
    let world = World::new(3);
    let domain = Domain::enabled(4);
    let mut store = Store::new();
    capture(&mut store, &world, &domain);
    store.fresh = Vec::new();
    domain.write(Table::Visibility, 2);
    domain.boundary(false);
    let mut rows = vec![native::CountryRow::default(); 8];
    let ((outcome, _), _, _) = crate::test_support::with_allocations_denied(|| {
        store.capture(
            &world,
            0,
            STATE,
            Buffer::Growing(&mut rows),
            &domain,
            false,
        )
    });
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(rows[..2], full(&world));
    assert!(store.epoch.is_none());
    domain.boundary(false);
    assert_eq!(capture(&mut store, &world, &domain).3, 3);
}
