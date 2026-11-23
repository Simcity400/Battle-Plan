use super::*;
use collector_missions::mission_changes::{Counts, Watches};
use engine_native::{
    cair_regional_mission::FailureContext, cair_regional_mission::Row,
    cair_regional_mission::walk_carried, cair_regional_mission::walk_with_diagnostic,
    cgame_state as gs, cpdx_array, cstrategic_air, memory::SliceMemory,
};
use std::cell::RefCell;

const GS: usize = 0x1000;
const SLOTS: usize = 4;
const TABLE: usize = 0x2000;
const LINKS: usize = 0x2100;
const COUNTRY: usize = 0x3000;
const MANAGER: usize = 0x4000;
const ROSTER: usize = 0x4100;
const AIR: usize = 0x5000;
const REGIONAL: usize = 0x1_0000;
const WINDOW: usize = 0x1000;
const WINGS: usize = 0x10_0000;
const REGIONS: u32 = 8;

fn air(slot: u32) -> usize {
    AIR + slot as usize * 0x200
}

fn window(slot: u32) -> usize {
    REGIONAL + slot as usize * WINDOW
}

struct World {
    bytes: Vec<u8>,
    countries: Vec<Vec<(u32, Vec<usize>)>>,
    census: Vec<WingIdentity>,
}

impl World {
    fn put(&mut self, at: usize, value: &[u8]) {
        self.bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32, capacity: u32) {
        self.u64(at, data as u64);
        self.u32(at + cpdx_array::CAPACITY, capacity);
        self.u32(at + cpdx_array::COUNT, count);
    }

    fn new() -> Self {
        let mut world = Self {
            bytes: vec![0; REGIONAL + (SLOTS + 1) * WINDOW],
            countries: vec![Vec::new(); SLOTS + 1],
            census: Vec::new(),
        };
        let slots = SLOTS as u32 + 1;
        world.array(GS + gs::COUNTRIES, TABLE, slots, slots);
        world.array(GS + gs::COUNTRY_LINKS, LINKS, slots, slots);
        world.u64(GS + gs::STRATEGIC_AIR_MANAGER, MANAGER as u64);
        world.u64(MANAGER, cstrategic_air_manager::VTABLE_RVA.0 as u64);
        world.array(
            MANAGER + cstrategic_air_manager::COUNTRY_AIR,
            ROSTER,
            slots,
            slots,
        );
        for slot in 1..slots {
            let country = COUNTRY + slot as usize * 0x40;
            world.u64(country, ccountry::VTABLE_RVA.0 as u64);
            world.u32(country + ccountry::ID, slot);
            world.u64(TABLE + slot as usize * 8, country as u64);
            world.u32(LINKS + slot as usize * 4, slot);
            world.u64(ROSTER + slot as usize * 8, air(slot) as u64);
            world.u64(air(slot), cstrategic_air::VTABLE_RVA.0 as u64);
            world.u32(air(slot) + cstrategic_air::COUNTRY, slot);
            let regions: &[u32] = if slot % 2 == 1 { &[2, 5] } else { &[3] };
            for &region in regions {
                let wings = (0..1 + (slot + region) as usize % 3)
                    .map(|_| world.wing(slot, region))
                    .collect();
                world.countries[slot as usize].push((region, wings));
            }
            world.layout(slot);
        }
        world
    }

    fn wing(&mut self, slot: u32, region: u32) -> usize {
        let index = self.census.len();
        self.census.push(WingIdentity {
            mission_pointer: WINGS + index * 0x200 + cair_wing::MISSION,
            wing: 1_000 + index as u64 * 7,
            owner: slot,
            region: Some(region),
            ..Default::default()
        });
        index
    }

    fn layout(&mut self, slot: u32) {
        let base = window(slot);
        let (outer, ids) = (base, base + 0x100);
        let regions = self.countries[slot as usize].clone();
        for region in 0..REGIONS {
            self.array(outer + region as usize * cpdx_array::STRIDE, 0, 0, 0);
        }
        let mut all = 0;
        for (index, (region, wings)) in regions.iter().enumerate() {
            self.u32(ids + index * 4, *region);
            let vector = base + 0x200 + *region as usize * 0x80;
            for (ordinal, &wing) in wings.iter().enumerate() {
                let pointer = self.census[wing].mission_pointer as u64;
                self.u64(vector + ordinal * 8, pointer);
            }
            self.array(
                outer + *region as usize * cpdx_array::STRIDE,
                vector,
                wings.len() as u32,
                16,
            );
            all += wings.len() as u32;
        }
        let at = air(slot);
        self.array(
            at + cstrategic_air::REGIONAL_MISSIONS,
            outer,
            REGIONS,
            REGIONS,
        );
        self.array(
            at + cstrategic_air::ACTIVE_MISSION_REGIONS,
            ids,
            regions.len() as u32,
            16,
        );
        self.array(at + cstrategic_air::ALL_ACTIVE_MISSIONS, ids, all, 64);
    }

    fn sorted_census(&self) -> Vec<WingIdentity> {
        let mut census = self.census.clone();
        census.sort_unstable_by_key(|wing| wing.mission_pointer);
        census
    }

    fn rows(&self) -> Vec<Row> {
        self.countries
            .iter()
            .flatten()
            .flat_map(|(_, wings)| {
                wings.iter().enumerate().map(|(ordinal, &wing)| Row {
                    wing: self.census[wing].wing,
                    ordinal: ordinal as u32,
                })
            })
            .collect()
    }
}

struct Traced<'a> {
    bytes: &'a [u8],
    regional: RefCell<[usize; SLOTS + 1]>,
}
impl Memory for Traced<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if let Some(offset) = address.checked_sub(REGIONAL)
            && let Some(reads) = self.regional.borrow_mut().get_mut(offset / WINDOW)
        {
            *reads += 1;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(address, output)
    }
}

struct Hour {
    rows: Vec<Row>,
    outcome: Outcome,
    failure: FailureContext,
    counts: Counts,
    regional: [usize; SLOTS + 1],
}

impl Hour {
    fn read(&self) -> Vec<u32> {
        (1..=SLOTS as u32)
            .filter(|&slot| self.regional[slot as usize] != 0)
            .collect()
    }
}

fn walk_with(
    world: &World,
    census: &[WingIdentity],
    store: &mut MissionStore,
    watches: &Watches,
) -> Hour {
    let memory = Traced {
        bytes: &world.bytes,
        regional: Default::default(),
    };
    // SAFETY: Unit test owns its local `MissionStore` and `Watches` fixture.
    let mut reuse = unsafe { MissionReuse::begin(store, watches) };
    let mut rows = Vec::new();
    let mut failure = FailureContext::default();
    let outcome = walk_carried(
        &memory,
        0,
        GS,
        census,
        engine_native::walk::Buffer::Growing(&mut rows),
        &mut failure,
        &mut reuse,
    );
    // SAFETY: Unit test owns its local `MissionStore` and `Watches` fixture.
    unsafe { reuse.finish(matches!(outcome, Outcome::Rows { .. })) };
    rows.truncate(outcome.len());
    Hour {
        rows,
        outcome,
        failure,
        counts: store.counts(),
        regional: memory.regional.into_inner(),
    }
}

fn walk(world: &World, store: &mut MissionStore, watches: &Watches) -> Hour {
    walk_with(world, &world.sorted_census(), store, watches)
}

fn fresh(world: &World, census: &[WingIdentity]) -> (Outcome, Vec<Row>) {
    let mut rows = Vec::new();
    let outcome = engine_native::cair_regional_mission::walk_with_diagnostic(
        &SliceMemory {
            base: 0,
            bytes: &world.bytes,
        },
        0,
        GS,
        census,
        &mut rows,
        &mut Default::default(),
    );
    rows.truncate(outcome.len());
    (outcome, rows)
}

fn first_hour(world: &World, store: &mut MissionStore, watches: &Watches) -> Hour {
    let first = walk(world, store, watches);
    assert_eq!(first.rows, world.rows());
    assert_eq!(
        first.outcome,
        Outcome::Rows {
            len: first.rows.len()
        }
    );
    assert_eq!(
        (
            first.counts.read,
            first.counts.reused,
            first.counts.read_rows
        ),
        (SLOTS as u64, 0, first.rows.len() as u64)
    );
    assert_eq!(first.read(), [1, 2, 3, 4]);
    watches.boundary(false);
    first
}

fn register(world: &mut World, watches: &Watches, slot: u32, change: impl FnOnce(&mut World)) {
    let notification = watches.notify(air(slot), 0);
    change(world);
    world.layout(slot);
    drop(notification);
}

#[test]
fn quiet_hours_read_no_regional_vector_and_publish_a_fresh_walks_rows() {
    let world = World::new();
    let watches = Watches::enabled(16);
    let mut store = MissionStore::new();
    let first = first_hour(&world, &mut store, &watches);
    for _ in 0..3 {
        let quiet = walk(&world, &mut store, &watches);
        assert_eq!(quiet.rows, first.rows);
        assert_eq!(quiet.outcome, first.outcome);
        assert_eq!(
            (
                quiet.counts.read,
                quiet.counts.reused,
                quiet.counts.reused_rows
            ),
            (0, SLOTS as u64, first.rows.len() as u64)
        );
        assert!(quiet.counts.quiet);
        assert!(quiet.read().is_empty(), "{:?}", quiet.regional);
        assert_eq!(
            fresh(&world, &world.sorted_census()),
            (quiet.outcome, quiet.rows)
        );
        watches.boundary(false);
    }
}

#[test]
fn all_active_order_is_read_only_on_certified_changes_and_failures_are_carried() {
    fn active(world: &mut World, slot: u32, reverse: bool) {
        let mut indices: Vec<_> = world.countries[slot as usize]
            .iter()
            .flat_map(|(_, wings)| wings.iter().copied())
            .collect();
        if reverse {
            indices.reverse();
        }
        let data = window(slot) + 0x900;
        for (at, index) in indices.iter().enumerate() {
            let wing = &mut world.census[*index];
            wing.owner_slot = slot;
            let pointer = wing.mission_pointer;
            world.u64(data + at * 8, pointer as u64);
        }
        world.array(
            air(slot) + cstrategic_air::ALL_ACTIVE_MISSIONS,
            data,
            indices.len() as u32,
            64,
        );
    }
    fn ordered(
        world: &World,
        store: &mut MissionStore,
        watches: &Watches,
    ) -> (Vec<mission_store::OrderLabel>, [usize; SLOTS + 1]) {
        let memory = Traced {
            bytes: &world.bytes,
            regional: Default::default(),
        };
        let census = world.sorted_census();
        // SAFETY: Unit test owns its local `MissionStore` and `Watches` fixture.
        let mut reuse = unsafe { MissionReuse::begin(store, watches) };
        let mut rows = Vec::new();
        let result = walk_carried(
            &memory,
            0,
            GS,
            &census,
            engine_native::walk::Buffer::Growing(&mut rows),
            &mut Default::default(),
            &mut reuse,
        );
        assert!(matches!(result, Outcome::Rows { .. }));
        rows.truncate(result.len());
        assert_eq!(rows, world.rows());
        let mut labels = Vec::new();
        // SAFETY: Unit test owns its local `MissionStore` and `Watches` fixture.
        unsafe { reuse.finish_with_order(true, &memory, &census, &mut labels) };
        (labels, memory.regional.into_inner())
    }
    let mut world = World::new();
    for slot in 1..=SLOTS as u32 {
        active(&mut world, slot, true);
    }
    let watches = Watches::enabled(16);
    let mut store = MissionStore::new();
    let (first, _) = ordered(&world, &mut store, &watches);
    for slot in 1..=SLOTS as u32 {
        let mut expected: Vec<_> = world.countries[slot as usize]
            .iter()
            .flat_map(|(_, indices)| indices.iter().map(|&index| world.census[index].wing))
            .collect();
        expected.reverse();
        let mut observed: Vec<_> = first
            .iter()
            .filter(|row| expected.contains(&row.wing))
            .copied()
            .collect();
        observed.sort_unstable_by_key(|row| row.label);
        assert_eq!(
            observed.iter().map(|row| row.wing).collect::<Vec<_>>(),
            expected
        );
    }
    watches.boundary(false);
    let (quiet, reads) = ordered(&world, &mut store, &watches);
    assert_eq!(quiet, first);
    assert_eq!(reads, [0; SLOTS + 1]);
    let bad = window(1) + 0x900;
    world.u64(bad, 0);
    let ordinary = world.countries[1]
        .iter()
        .map(|(_, rows)| rows.len())
        .sum::<usize>();
    world.countries[1].push((7, Vec::new()));
    world.layout(1);
    world.array(
        air(1) + cstrategic_air::ALL_ACTIVE_MISSIONS,
        bad,
        ordinary as u32,
        64,
    );
    let (moved, reads) = ordered(&world, &mut store, &watches);
    assert_eq!(moved, first);
    assert!(reads[1] > 0);
    drop(watches.notify(air(1), 0));
    watches.boundary(false);
    let (failed, _) = ordered(&world, &mut store, &watches);
    assert!(failed.len() < first.len());
    active(&mut world, 1, false);
    watches.boundary(false);
    let (still_unknown, reads) = ordered(&world, &mut store, &watches);
    assert_eq!(still_unknown, failed);
    assert_eq!(reads, [0; SLOTS + 1]);
    drop(watches.notify(air(1), 0));
    watches.boundary(false);
    let (restored, reads) = ordered(&world, &mut store, &watches);
    assert_eq!(restored.len(), first.len());
    assert!(reads[1] > 0);
    assert!(reads[2..].iter().all(|&n| n == 0));
}

#[test]
fn a_hooked_registration_rereads_only_its_country() {
    let mut world = World::new();
    let watches = Watches::enabled(16);
    let mut store = MissionStore::new();
    first_hour(&world, &mut store, &watches);
    let wing = world.countries[2][0].1[0];
    world.census[wing].region = Some(4);
    register(&mut world, &watches, 2, |world| {
        world.countries[2][0].1.remove(0);
        world.countries[2].push((4, vec![wing]));
    });
    watches.boundary(false);
    let hour = walk(&world, &mut store, &watches);
    assert_eq!(hour.read(), [2]);
    assert_eq!((hour.counts.read, hour.counts.reused), (1, 3));
    assert!(!hour.counts.quiet);
    assert_eq!(hour.rows, world.rows());
    watches.boundary(false);
    let quiet = walk(&world, &mut store, &watches);
    assert!(quiet.read().is_empty());
    assert_eq!(quiet.rows, world.rows());
}

#[test]
fn a_writer_that_keeps_every_header_is_still_seen() {
    let mut world = World::new();
    let watches = Watches::enabled(16);
    let mut store = MissionStore::new();
    first_hour(&world, &mut store, &watches);
    let headers = |world: &World| {
        let at = air(3) + cstrategic_air::REGIONAL_MISSIONS;
        world.bytes[at..at + engine_native::cair_regional_mission::HEADERS].to_vec()
    };
    let before = headers(&world);
    let replacement = world.wing(3, 5);
    register(&mut world, &watches, 3, |world| {
        world.countries[3][1].1[0] = replacement;
    });
    assert_eq!(headers(&world), before);
    watches.boundary(false);
    let hour = walk(&world, &mut store, &watches);
    assert_eq!(hour.read(), [3]);
    assert_eq!(hour.rows, world.rows());
}

#[test]
fn an_invalidating_hour_or_a_forgotten_store_rereads_every_country() {
    let world = World::new();
    let watches = Watches::enabled(16);
    let mut store = MissionStore::new();
    let first = first_hour(&world, &mut store, &watches);
    watches.boundary(true);
    let hour = walk(&world, &mut store, &watches);
    assert_eq!((hour.counts.read, hour.counts.reused), (SLOTS as u64, 0));
    assert_eq!(hour.read(), [1, 2, 3, 4]);
    assert_eq!(hour.rows, first.rows);
    watches.boundary(false);
    store.forget();
    let forgotten = walk(&world, &mut store, &watches);
    assert_eq!(
        (forgotten.counts.read, forgotten.counts.reused),
        (SLOTS as u64, 0)
    );
}

#[test]
fn an_unhooked_carrier_region_moves_the_header_and_rereads() {
    let mut world = World::new();
    let watches = Watches::enabled(16);
    let mut store = MissionStore::new();
    first_hour(&world, &mut store, &watches);
    world.countries[4].insert(0, (1, Vec::new()));
    world.layout(4);
    let hour = walk(&world, &mut store, &watches);
    assert_eq!(hour.read(), [4]);
    assert_eq!(hour.rows, world.rows());
}

#[test]
fn a_census_disagreement_reads_the_country_afresh_with_the_fresh_outcome() {
    let world = World::new();
    let watches = Watches::enabled(16);
    let mut store = MissionStore::new();
    first_hour(&world, &mut store, &watches);
    let wing = world.countries[3][0].1[0];
    let mut foreign = world.census.clone();
    foreign[wing].owner = 1;
    let mut missing = world.census.clone();
    missing.remove(wing);
    for mut census in [foreign, missing] {
        census.sort_unstable_by_key(|wing| wing.mission_pointer);
        let hour = walk_with(&world, &census, &mut store, &watches);
        let mut failure = Default::default();
        let expected = walk_with_diagnostic(
            &SliceMemory {
                base: 0,
                bytes: &world.bytes,
            },
            0,
            GS,
            &census,
            &mut vec![Row::default(); 32],
            &mut failure,
        );
        assert!(
            matches!(expected, Outcome::Unavailable { .. }),
            "{expected:?}"
        );
        assert_eq!((hour.outcome, hour.failure), (expected, failure));
        assert_eq!(failure.country_slot, Some(3));
        assert!(hour.regional[3] != 0);
        assert_eq!(hour.counts.reused, 2);
        watches.boundary(false);
        let after = walk(&world, &mut store, &watches);
        assert_eq!((after.counts.read, after.counts.reused), (SLOTS as u64, 0));
        watches.boundary(false);
    }
}

#[test]
fn an_unobserved_write_is_carried_stale() {
    let mut world = World::new();
    let watches = Watches::enabled(16);
    let mut store = MissionStore::new();
    let first = first_hour(&world, &mut store, &watches);
    let replacement = world.wing(1, 5);
    world.countries[1][1].1[0] = replacement;
    world.layout(1);
    let stale = walk(&world, &mut store, &watches);
    assert_eq!(stale.counts.reused, SLOTS as u64);
    assert_eq!(stale.rows, first.rows);
}
