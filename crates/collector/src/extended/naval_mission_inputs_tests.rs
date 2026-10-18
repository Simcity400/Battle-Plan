//! Synthetic-world tests of the naval mission operands pack.
use super::*;
use engine_native::{
    ccharacter, cfleet, cgame_state, cnaval_mission, cnavy_leader, cprovince, cstrategic_navy,
    cstrategic_region, ctask_force, cunit, memory::CountedMemory, memory::LiveMemory,
    memory::SliceMemory, naval_mission_inputs as native, reference_bucket as bucket,
    sweather_per_region,
};

const GAME_STATE: usize = 0x10000;
/// Above every module RVA the readers touch.
const HEAP: usize = 0x0350_0000;
const REGIONS: u32 = 24;
const TYPE: u64 = 4713;
/// Task-force kinds per fleet: strike force, raiding, escort, invasion support and none.
const KINDS: [u32; 5] = [2, 3, 4, 9, 0];
const RAIDER: usize = 1;
const ESCORTING: usize = 2;
const STRIKE: usize = 0;

/// A many-country world at its own buffer's address, so the capture can read it through the
/// production guarded copies. Each country has one fleet of five task forces (kinds 2, 3, 4,
/// 9 and 0), a fleet leader and one headquarters commander on the escort. The escort's
/// mission files three regions, the raider's the first of them and the strike force's the
/// second; each region's ledger stores the escort first, against the walk's order, and the
/// navy keeps a presence ring for each escort region.
struct World {
    bytes: Vec<u8>,
    base: usize,
    game_state: usize,
    /// Offsets to patch in tests.
    fleets: Vec<usize>,
    tasks: Vec<[usize; 5]>,
    /// Each country's ledger data for its first escort region.
    ledgers: Vec<usize>,
}

use super::super::bench_world::Heap;

fn leader(slot: u32) -> u64 {
    (u64::from(slot) << 32) | TYPE
}

fn character(slot: u32) -> u64 {
    ((u64::from(slot) + 1_000_000) << 32) | TYPE
}

fn task_handle(slot: u32, index: usize) -> u64 {
    ((u64::from(slot) * 8 + index as u64) << 32) | 61
}

fn escort_regions(slot: u32) -> [u32; 3] {
    [0, 1, 2].map(|step| (slot + step) % REGIONS + 1)
}

/// Robin Hood placement for both mission probes:
/// the (slot, one-based distance) of each item from its home slot.
fn robin_hood(homes: &[usize], slots: usize) -> Vec<(usize, u8)> {
    let mut table: Vec<Option<(u8, usize)>> = vec![None; slots];
    for (item, &home) in homes.iter().enumerate() {
        let (mut item, mut distance, mut index) = (item, 1_u8, home);
        loop {
            match table[index] {
                None => {
                    table[index] = Some((distance, item));
                    break;
                }
                Some((placed, other)) if placed < distance => {
                    table[index] = Some((distance, item));
                    (item, distance) = (other, placed);
                }
                Some(_) => {}
            }
            index += 1;
            distance += 1;
        }
    }
    let mut placed = vec![(0, 0); homes.len()];
    for (index, entry) in table.into_iter().enumerate() {
        if let Some((distance, item)) = entry {
            placed[item] = (index, distance);
        }
    }
    placed
}

fn build(countries: u32) -> World {
    build_world(countries, false)
}

fn fixture_capacities(world: &World) -> Capacities {
    let mut capacity = Capacities::default();
    capacity
        .presize(&world.slice(), world.base, world.game_state)
        .expect("synthetic naval mission storage demand");
    capacity
}

fn build_world(countries: u32, hubs: bool) -> World {
    let len = build_in(countries, Vec::new(), hubs).bytes.len();
    let world = build_in(countries, Vec::with_capacity(len), hubs);
    assert_eq!(world.bytes.len(), len);
    world
}

fn build_in(countries: u32, mut bytes: Vec<u8>, hubs: bool) -> World {
    let origin = bytes.as_ptr() as usize;
    bytes.resize(HEAP, 0);
    let mut heap = Heap { bytes, origin };
    let slots = countries as usize + 1;
    let table = heap.alloc(slots * 8);
    let links = heap.alloc(slots * 4);
    heap.array(GAME_STATE + cgame_state::COUNTRIES, table, slots as u32);
    heap.array(GAME_STATE + cgame_state::COUNTRY_LINKS, links, slots as u32);
    // Regions, one province each, and their weather records.
    let weather = heap.alloc(0x80);
    heap.u64(GAME_STATE + cgame_state::WEATHER_MANAGER, heap.ptr(weather));
    heap.vtable(weather, cweather_manager::VTABLE_RVA);
    let records = heap.alloc((REGIONS as usize + 1) * sweather_per_region::STRIDE);
    heap.array(weather + cweather_manager::REGIONS, records, REGIONS + 1);
    let mut regions = vec![0];
    let mut provinces = vec![0];
    for id in 1..=REGIONS {
        let record = records + id as usize * sweather_per_region::STRIDE;
        heap.vtable(record, sweather_per_region::VTABLE_RVA);
        heap.u32(record + sweather_per_region::ID, id);
        heap.accumulator(
            record + sweather_per_region::MODIFIERS,
            &[(0x19, -i64::from(id) * 1_000)],
        );
        let region = heap.alloc(0x100);
        heap.vtable(region, cstrategic_region::VTABLE_RVA);
        heap.u32(region + cstrategic_region::ID, id);
        heap.u32(region + cstrategic_region::RUNTIME_SLOT, id);
        let province = heap.alloc(0x100);
        heap.vtable(province, cprovince::VTABLE_RVA);
        heap.u64(province + cprovince::STRATEGIC_REGION, heap.ptr(region));
        regions.push(region);
        provinces.push(province);
    }
    let manager = heap.alloc(0x40);
    heap.u64(
        GAME_STATE + cgame_state::STRATEGIC_NAVY_MANAGER,
        heap.ptr(manager),
    );
    heap.vtable(manager, cstrategic_navy_manager::VTABLE_RVA);
    let mut navies = vec![0];
    let mut handles = Vec::new();
    let mut fleets = vec![0];
    let mut tasks = vec![[0; 5]];
    let mut ledgers = vec![0];
    for slot in 1..slots as u32 {
        let country = heap.alloc(0x600);
        heap.u64(table + slot as usize * 8, heap.ptr(country));
        heap.u32(links + slot as usize * 4, slot);
        heap.vtable(country, ccountry::VTABLE_RVA);
        heap.u32(country + ccountry::ID, slot);
        heap.accumulator(
            country + ccountry::MODIFIER_ACCUMULATOR,
            &[
                (0x19, 5_000),
                (0xcc, i64::from(slot)),
                (0x10e, 20_000),
                (0x248, -30_000),
            ],
        );
        let navy = heap.alloc(0x200);
        for (at, rva) in [
            (navy, cstrategic_navy::VTABLE_RVA),
            (
                navy + cstrategic_navy::SECONDARY,
                cstrategic_navy::SECONDARY_VTABLE_RVA,
            ),
        ] {
            heap.vtable(at, rva);
        }
        // The physical navy pass authenticates each navy's manager backlink.
        heap.u64(
            navy + engine_naval_history::country_naval_movement_inputs::MANAGER,
            heap.ptr(manager),
        );
        heap.u32(navy + cstrategic_navy::OWNER, slot);
        // Presence map: mask 31, probe 2, each escort region in its hashed slot.
        let entries = heap.alloc((32 + 2 + 2) * 0x28);
        heap.u64(navy + native::PRESENCE + 8, heap.ptr(entries));
        heap.u32(navy + native::PRESENCE + 0x14, 31);
        heap.put(navy + native::PRESENCE + 0x18, &[2]);
        let homes = escort_regions(slot).map(|region| {
            let hashed = (region >> 16 ^ region).wrapping_mul(0x045d_9f3b);
            let hashed = (hashed >> 16 ^ hashed).wrapping_mul(0x045d_9f3b);
            ((hashed >> 16 ^ hashed) & 31) as usize
        });
        for (step, (region, (index, distance))) in escort_regions(slot)
            .into_iter()
            .zip(robin_hood(&homes, 34))
            .enumerate()
        {
            assert!(distance <= 3, "presence probe too long");
            let entry = entries + index * 0x28;
            heap.put(entry + 4, &[distance]);
            heap.u32(entry + 8, region);
            let ring = heap.alloc(16);
            for (at, value) in [1_i32, 0, -1, step as i32].into_iter().enumerate() {
                heap.u32(ring + at * 4, value as u32);
            }
            heap.u64(entry + 0x10, heap.ptr(ring));
            for (offset, value) in [(0x18, 4), (0x1c, 0), (0x20, 3)] {
                heap.u32(entry + offset, value);
            }
        }
        let headers = heap.alloc((REGIONS as usize + 1) * cstrategic_navy::REGION_MISSIONS_STRIDE);
        heap.array(
            navy + cstrategic_navy::REGION_MISSIONS,
            headers,
            REGIONS + 1,
        );
        if hubs {
            heap.words(navy + native::HUBS, &[3, 1, 3]);
        }
        navies.push(navy);
        // The fleet's leader and the escort's headquarters commander.
        let navy_leader = heap.alloc(0x500);
        heap.vtable(navy_leader, cnavy_leader::VTABLE_RVA);
        heap.accumulator(navy_leader + native::LEADER_MODIFIERS, &[(0xcc, 2_500)]);
        handles.push((leader(slot), navy_leader));
        let person = heap.alloc(0x100);
        heap.vtable(person, ccharacter::VTABLE_RVA);
        let unit_leader = heap.alloc(0x1000);
        heap.u64(
            person + native::CHARACTER_UNIT_LEADER,
            heap.ptr(unit_leader),
        );
        heap.u32(unit_leader + native::UNIT_LEADER_KIND, 2);
        for (index, source) in native::UNIT_LEADER_SOURCES.into_iter().enumerate() {
            let object = heap.alloc(0x100);
            heap.u64(unit_leader + source, heap.ptr(object));
            heap.accumulator(
                object + native::SOURCE_MODIFIERS,
                &[(0x19, index as i64 + 1)],
            );
        }
        handles.push((character(slot), person));
        let commander = heap.alloc(0x20);
        heap.u64(commander + native::COMMANDER_CHARACTER, character(slot));
        let headquarters = heap.alloc(0x80);
        heap.u64(
            headquarters + native::HEADQUARTERS_COMMANDER,
            heap.ptr(commander),
        );
        let fleet = heap.alloc(0x100);
        heap.pointers(country + ccountry::FLEETS, &[fleet]);
        heap.vtable(fleet, cfleet::VTABLE_RVA);
        for (offset, value) in [
            (cfleet::HANDLE, (u64::from(slot) << 32) | 61),
            (cfleet::OWNER, heap.ptr(country)),
            (cfleet::LEADER, leader(slot)),
        ] {
            heap.u64(fleet + offset, value);
        }
        let mut own = [0; 5];
        for (index, kind) in KINDS.into_iter().enumerate() {
            let task = heap.alloc(0x800);
            own[index] = task;
            let mission = task + ctask_force::NAVAL_MISSION;
            let stats = heap.alloc(0x200);
            for (at, rva) in [
                (task, ctask_force::VTABLE_RVA),
                (
                    task + ctask_force::SECONDARY_BASE,
                    ctask_force::SECONDARY_VTABLE_RVA,
                ),
                (mission, cnaval_mission::VTABLE_RVA),
            ] {
                heap.vtable(at, rva);
            }
            for (at, value) in [
                (task + cunit::HANDLE, task_handle(slot, index)),
                (task + ctask_force::FLEET, heap.ptr(fleet)),
                (mission + 8, heap.ptr(task)),
                (mission + native::MISSION_RANGE_FLEET, heap.ptr(fleet)),
                (task + cunit::STAT_BLOCK, heap.ptr(stats)),
                (stats + native::STAT_RAIDING, 20_000),
                (
                    task + cunit::LOCATION,
                    heap.ptr(provinces[(slot % REGIONS + 1) as usize]),
                ),
            ] {
                heap.u64(at, value);
            }
            for (at, value) in [
                (task + ctask_force::MISSION, kind),
                (mission + native::MISSION_OWNER, slot),
                (mission + native::MISSION_CONVOYS, 10 + index as u32),
                (mission + native::MISSION_RANGE, 0),
                (mission + native::MISSION_RANGE + 4, 3),
            ] {
                heap.u32(at, value);
            }
            heap.accumulator(task + native::LOCAL_MODIFIERS, &[(0xcc, -1_000)]);
            heap.array(task + ctask_force::ATTACHED_HEADQUARTERS, 0, 0);
            let filed: &[u32] = match index {
                ESCORTING => &escort_regions(slot),
                RAIDER => &escort_regions(slot)[..1],
                STRIKE => &escort_regions(slot)[1..2],
                _ => &[],
            };
            let accessible: Vec<usize> = filed
                .iter()
                .map(|&region| regions[region as usize])
                .collect();
            heap.pointers(mission + cnaval_mission::ACCESSIBLE_REGIONS, &accessible);
            if index == ESCORTING {
                heap.pointers(task + ctask_force::ATTACHED_HEADQUARTERS, &[headquarters]);
            }
        }
        heap.pointers(fleet + cfleet::TASK_FORCES, &own);
        // Each region's ledger: the escort first, then the task force filed with it.
        let mission = |index: usize| own[index] + ctask_force::NAVAL_MISSION;
        let [first, second, third] = escort_regions(slot);
        for (region, filed) in [
            (first, vec![mission(ESCORTING), mission(RAIDER)]),
            (second, vec![mission(ESCORTING), mission(STRIKE)]),
            (third, vec![mission(ESCORTING)]),
        ] {
            let data = heap.pointers(
                headers + region as usize * cstrategic_navy::REGION_MISSIONS_STRIDE,
                &filed,
            );
            if region == first {
                ledgers.push(data);
            }
        }
        // Naval mission registration registers each filed mission once in the navy's set.
        heap.pointers(
            navy + cstrategic_navy::REGISTERED_MISSIONS,
            &[mission(ESCORTING), mission(RAIDER), mission(STRIKE)],
        );
        fleets.push(fleet);
        tasks.push(own);
    }
    heap.pointers(manager + cstrategic_navy_manager::NAVIES, &navies);
    // The high-type reference registry holds every leader and character.
    let capacity = (handles.len() * 2).next_power_of_two().max(8);
    let exponent = capacity.trailing_zeros();
    let overflow = exponent + 2;
    let registry = heap.alloc(creference_registry::SIZE);
    let buckets = heap.alloc((capacity + overflow as usize + 1) * bucket::STRIDE);
    heap.u64(creference_registry::HIGH_ROOT_RVA, heap.ptr(registry));
    heap.u64(registry + creference_registry::ENTRIES, heap.ptr(buckets));
    heap.u32(registry + creference_registry::COUNT, handles.len() as u32);
    heap.u32(registry + creference_registry::MASK, capacity as u32 - 1);
    heap.put(
        registry + creference_registry::OVERFLOW_DISTANCE,
        &[overflow as u8],
    );
    let homes: Vec<usize> = handles
        .iter()
        .map(|&(handle, _)| {
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(creference_registry::HASH_MULTIPLIER);
            (((hash >> 32) as u32 ^ hash as u32) & (capacity as u32 - 1)) as usize
        })
        .collect();
    for (&(handle, object), (index, distance)) in handles
        .iter()
        .zip(robin_hood(&homes, capacity + overflow as usize))
    {
        assert!(
            u32::from(distance) <= overflow + 1,
            "registry probe too long"
        );
        let entry = buckets + index * bucket::STRIDE;
        heap.put(entry + bucket::DISTANCE, &[distance]);
        heap.u64(entry + bucket::TYPE, handle);
        heap.u64(entry + bucket::OBJECT, heap.ptr(object));
    }
    heap.put(
        buckets + (capacity + overflow as usize) * bucket::STRIDE + bucket::DISTANCE,
        &[bucket::TERMINAL_DISTANCE],
    );
    World {
        base: origin,
        game_state: origin + GAME_STATE,
        bytes: heap.bytes,
        fleets,
        tasks,
        ledgers,
    }
}

impl World {
    fn slice(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: self.base,
            bytes: &self.bytes,
        }
    }
    fn put(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn pointer(&self, at: usize) -> usize {
        u64::from_le_bytes(self.bytes[at..at + 8].try_into().unwrap()) as usize
    }
}

struct TransportMemory<'a> {
    world: &'a World,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    fail_once: Option<usize>,
    failed: std::cell::Cell<bool>,
}

impl Memory for TransportMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.fail_once == Some(address) && !self.failed.replace(true) {
            return false;
        }
        self.world.slice().copy(address, output)
    }
}

#[test]
fn a_null_registration_refuses_the_strict_ledgers_after_one_read() {
    let mut world = build(1);
    let navy = {
        let manager = world.pointer(GAME_STATE + cgame_state::STRATEGIC_NAVY_MANAGER) - world.base;
        let navies = world.pointer(manager + cstrategic_navy_manager::NAVIES) - world.base;
        world.pointer(navies + 8) - world.base
    };
    let outer = world.pointer(navy + cstrategic_navy::REGION_MISSIONS) - world.base;
    for region in escort_regions(1) {
        let header = outer + region as usize * cstrategic_navy::REGION_MISSIONS_STRIDE;
        let data = world.pointer(header) - world.base;
        let count = u32::from_le_bytes(world.bytes[header + 12..header + 16].try_into().unwrap());
        for index in 0..count as usize {
            world.put(data + index * 8, 0);
        }
    }
    let memory = TransportMemory {
        world: &world,
        reads: Default::default(),
        fail_once: None,
        failed: Default::default(),
    };
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    captured.capture(&memory, world.base, world.game_state, 71, true, None);
    assert!(matches!(
        captured.ledgers.outcome,
        Outcome::Unavailable {
            reason: "navy ledger mission unreadable or null"
        }
    ));
    assert!(matches!(
        captured.task_forces.outcome,
        Outcome::Rows { len: 4 }
    ));
    let reads = memory.reads.into_inner();
    assert_eq!(
        reads
            .iter()
            .filter(|&&(at, _)| at == world.base + world.ledgers[1])
            .count(),
        1
    );
}

/// The offset of country `slot`'s navy.
fn navy_of(world: &World, slot: u32) -> usize {
    let manager = world.pointer(GAME_STATE + cgame_state::STRATEGIC_NAVY_MANAGER) - world.base;
    let navies = world.pointer(manager + cstrategic_navy_manager::NAVIES) - world.base;
    world.pointer(navies + slot as usize * 8) - world.base
}

/// The address of a navy's region ledger headers.
fn region_headers(world: &World, slot: u32) -> usize {
    world.pointer(navy_of(world, slot) + cstrategic_navy::REGION_MISSIONS)
}

/// Zeroes a navy's registered-mission count and, with `ledgers`, every
/// region ledger count, as the reset does.
fn empty_navy(world: &mut World, slot: u32, ledgers: bool) {
    let navy = navy_of(world, slot);
    let count = navy + cstrategic_navy::REGISTERED_MISSIONS + 12;
    world.bytes[count..count + 4].fill(0);
    if ledgers {
        let headers = region_headers(world, slot) - world.base;
        for region in 0..=REGIONS as usize {
            let count = headers + region * cstrategic_navy::REGION_MISSIONS_STRIDE + 12;
            world.bytes[count..count + 4].fill(0);
        }
    }
}

fn ledger_read(
    world: &World,
    skip: LedgerSkip,
) -> (
    Vec<NavalMissionLedgerRow>,
    LedgerCounts,
    Vec<(usize, usize)>,
) {
    let memory = TransportMemory {
        world,
        reads: Default::default(),
        fail_once: None,
        failed: Default::default(),
    };
    let mut captured = Captured::with_capacities(&fixture_capacities(world));
    captured.capture_skipping(&memory, world.base, world.game_state, 71, skip);
    let views = captured.tables();
    let ledgers = rows::<tables::NavalMissionLedgers>(&views[1]).to_vec();
    (ledgers, captured.ledger_counts(), memory.reads.into_inner())
}

/// A navy whose registered-mission set is empty has every region ledger
/// Empty  (append, remove and reset write both),
/// so outside its sampled slice its region headers are not read, and the
/// ledgers are those of a read of every navy.
#[test]
fn a_navy_without_registered_missions_reads_no_region_header_outside_its_slice() {
    use crate::sampled_verify::Check;
    let mut world = build(3);
    empty_navy(&mut world, 2, true);
    let headers = region_headers(&world, 2);
    let reads_headers = |reads: &[(usize, usize)]| reads.iter().any(|&(at, _)| at == headers);
    let (all, counts, reads) = ledger_read(&world, LedgerSkip::NEVER);
    assert!(reads_headers(&reads));
    assert_eq!(counts.read, 3);
    assert!(all.iter().all(|row| row.owner != 2) && !all.is_empty());
    // Hour 71 samples slot 23: navy 2 is skipped.
    let skip = LedgerSkip {
        enabled: true,
        check: Check::Slice { hour: 71 },
    };
    let (ledgers, counts, reads) = ledger_read(&world, skip);
    assert_eq!(ledgers, all);
    assert!(!reads_headers(&reads));
    assert_eq!(
        counts,
        LedgerCounts {
            skipped: 1,
            read: 2,
            verified: 0,
            mismatches: 0,
        }
    );
    // Its slice, and verify mode, read it and compare an empty ledger.
    for check in [Check::Slice { hour: 26 }, Check::Every] {
        let skip = LedgerSkip {
            enabled: true,
            check,
        };
        let (ledgers, counts, reads) = ledger_read(&world, skip);
        assert_eq!(ledgers, all);
        assert!(reads_headers(&reads));
        assert_eq!(
            counts,
            LedgerCounts {
                skipped: 0,
                read: 3,
                verified: 1,
                mismatches: 0,
            }
        );
    }
}

/// A skippable navy whose ledgers still file missions is a missed writer:
/// read in its slice, it publishes the fresh ledgers, counts a mismatch and
/// stops every later skip.
#[test]
fn a_filed_ledger_behind_an_empty_registered_set_is_a_published_mismatch() {
    use crate::sampled_verify::Check;
    let mut world = build(3);
    let (all, _, _) = ledger_read(&world, LedgerSkip::NEVER);
    assert!(all.iter().any(|row| row.owner == 3));
    empty_navy(&mut world, 3, false);
    for check in [Check::Slice { hour: 3 }, Check::Every] {
        let skip = LedgerSkip {
            enabled: true,
            check,
        };
        let (ledgers, counts, _) = ledger_read(&world, skip);
        assert_eq!(ledgers, all);
        assert_eq!(
            counts,
            LedgerCounts {
                skipped: 0,
                read: 3,
                verified: 1,
                mismatches: 1,
            }
        );
    }
    crate::navy_ledger_changes::record(LedgerCounts {
        skipped: 0,
        read: 3,
        verified: 1,
        mismatches: 1,
    });
    assert!(!LedgerSkip::current().enabled);
}

#[test]
fn an_empty_manager_does_not_heal_a_missing_strict_country_navy() {
    let mut world = build(1);
    let capacity = fixture_capacities(&world);
    let manager = world.pointer(GAME_STATE + cgame_state::STRATEGIC_NAVY_MANAGER) - world.base;
    world.put(manager + cstrategic_navy_manager::NAVIES, 0);
    world.put(manager + cstrategic_navy_manager::NAVIES + 8, 0);
    let mut captured = Captured::with_capacities(&capacity);
    captured.capture(&world.slice(), world.base, world.game_state, 71, true, None);
    assert!(matches!(
        captured.ledgers.outcome,
        Outcome::Unavailable {
            reason: "navy manager country outside array"
        }
    ));
}

fn capture(
    captured: &mut Captured,
    world: &World,
    hour: i32,
    force_full: bool,
    fingerprints: &mut SliceFingerprints,
) -> [TableView; N] {
    captured.capture(
        &world.slice(),
        world.base,
        world.game_state,
        hour,
        force_full,
        Some(fingerprints),
    );
    captured.tables()
}

fn rows<S: replay_data::rows::TableSchema>(view: &TableView) -> &[S::Row] {
    let TableView::Complete(rows) = view else {
        panic!("incomplete {view:?}");
    };
    rows.rows::<S>().expect("table rows")
}

#[test]
fn a_full_hour_records_every_raiding_and_escort_task_force_ledger_and_country() {
    let world = build(3);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    let mut fingerprints = SliceFingerprints::new();
    let views = capture(&mut captured, &world, 0, true, &mut fingerprints);
    let task_forces = rows::<tables::NavalMissionTaskForces>(&views[0]);
    // Strike, raid, escort and invasion-support operands, four per country.
    assert_eq!(task_forces.len(), 12);
    let escort = task_forces
        .iter()
        .find(|row| row.task_force == task_handle(1, ESCORTING))
        .unwrap();
    assert_eq!(
        *escort,
        NavalMissionTaskForceRow {
            task_force: task_handle(1, ESCORTING),
            mission_owner: 1,
            raiding_factor: 20_000,
            local_coordination: None,
            local_raiding: Some(-1_000),
            // Country 1's task forces sit in region 2.
            weather_coordination: Some(-2_000),
            weather_raiding: None,
            leader: leader(1),
            headquarters: vec![character(1)],
            range_fleet: (1 << 32) | 61,
            range_begin: 0,
            range_end: 3,
            convoys: 12,
            ..Default::default()
        }
    );
    // Each navy files its three escort regions. The ledger keeps its stored order, the
    // escort first, not the walk's; every region holds the escort, so its presence is read:
    // ring [1, 0, -1] (one above zero, two at least zero).
    let ledgers = rows::<tables::NavalMissionLedgers>(&views[1]);
    assert_eq!(ledgers.len(), 9);
    let [first, second, third] = escort_regions(1);
    let navy: Vec<_> = ledgers
        .iter()
        .filter(|row| row.owner == 1)
        .map(|row| (row.region, row.task_forces.clone()))
        .collect();
    let handle = |index| task_handle(1, index);
    let mut expected = vec![
        (first, vec![handle(ESCORTING), handle(RAIDER)]),
        (second, vec![handle(ESCORTING), handle(STRIKE)]),
        (third, vec![handle(ESCORTING)]),
    ];
    expected.sort();
    assert_eq!(navy, expected);
    assert!(
        ledgers
            .iter()
            .all(|row| (row.ring, row.positive, row.total) == (true, 1, 2))
    );
    let countries = rows::<tables::NavalMissionCountries>(&views[2]);
    assert_eq!(countries.len(), 3);
    assert_eq!(
        countries[1],
        NavalMissionCountryRow {
            country: 2,
            coordination: Some(5_000),
            raiding: Some(2),
            escort: Some(20_000),
            weather_reduction: Some(-30_000),
            ..Default::default()
        }
    );
    // Per country: the fleet leader (source 0) and the commander's four sources.
    let commanders = rows::<tables::NavalMissionCommanders>(&views[3]);
    assert_eq!(commanders.len(), 15);
    let first: Vec<_> = commanders
        .iter()
        .filter(|row| row.country == 1)
        .map(|row| (row.reference, row.source, row.coordination, row.raiding))
        .collect();
    assert_eq!(
        first,
        [
            (leader(1), 0, None, Some(2_500)),
            (character(1), 1, Some(1), None),
            (character(1), 2, Some(2), None),
            (character(1), 3, Some(3), None),
            (character(1), 4, Some(4), None),
        ]
    );
}

#[test]
fn an_unlisted_escort_still_has_its_region_presence_input() {
    let mut world = build(1);
    // The first escort region's ledger holds only the raider.
    let raider = world.tasks[1][RAIDER] + ctask_force::NAVAL_MISSION;
    let data = world.ledgers[1];
    world.put(data, (world.base + raider) as u64);
    world.put(data + 8, (world.base + raider) as u64);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    let views = capture(
        &mut captured,
        &world,
        0,
        true,
        &mut SliceFingerprints::new(),
    );
    let ledgers = rows::<tables::NavalMissionLedgers>(&views[1]);
    let first = ledgers
        .iter()
        .find(|row| row.region == escort_regions(1)[0])
        .unwrap();
    assert_eq!(
        (first.task_forces.clone(), first.ring),
        (vec![task_handle(1, RAIDER), task_handle(1, RAIDER)], true)
    );
}

#[test]
fn a_ledger_naming_a_task_force_outside_the_census_is_refused_alone() {
    let mut world = build(2);
    let data = world.ledgers[2];
    world.put(data + 8, (world.base + 0x40) as u64);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    let views = capture(
        &mut captured,
        &world,
        0,
        true,
        &mut SliceFingerprints::new(),
    );
    assert!(matches!(
        &views[1],
        TableView::Unavailable { reason }
            if reason == "navy ledger names a task force outside the fleet census"
    ));
    assert_eq!(rows::<tables::NavalMissionTaskForces>(&views[0]).len(), 8);
    assert_eq!(rows::<tables::NavalMissionCountries>(&views[2]).len(), 2);
}

#[test]
fn an_off_hour_reads_its_slice_and_countries_whose_commanders_changed() {
    let mut world = build(30);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    let mut fingerprints = SliceFingerprints::new();
    capture(&mut captured, &world, 0, true, &mut fingerprints);
    let owners = |views: &[TableView; N]| {
        rows::<tables::NavalMissionCountries>(&views[2])
            .iter()
            .map(|row| row.country)
            .collect::<Vec<_>>()
    };
    // Hour 5 reads slice 5 only: countries 5 and 29.
    let views = capture(&mut captured, &world, 5, false, &mut fingerprints);
    assert_eq!(owners(&views), [5, 29]);
    assert!(captured.selection().contains(29) && !captured.selection().contains(7));
    // Country 7's fleet takes another leader: it is read at the next hour, early.
    let fleet = world.fleets[7];
    world.put(fleet + native::FLEET_LEADER, leader(99));
    let views = capture(&mut captured, &world, 6, false, &mut fingerprints);
    assert_eq!(owners(&views), [6, 7, 30]);
    assert_eq!(captured.selection().early_count, 1);
    // The hourly tables are complete every hour.
    assert_eq!(rows::<tables::NavalMissionTaskForces>(&views[0]).len(), 120);
    assert_eq!(rows::<tables::NavalMissionLedgers>(&views[1]).len(), 90);
}

/// A modifier rebuild the naval tree family observed (its certificate moved)
/// is read within the hour, outside the country's slice; the daily slice
/// alone left it up to a day old. An unobservable certificate reads the
/// country every hour.
#[test]
fn a_moved_certificate_is_read_within_the_hour() {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let world = build(30);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    captured.sequence = |_| Some((1, SEQUENCE.load(std::sync::atomic::Ordering::Relaxed)));
    let mut fingerprints = SliceFingerprints::new();
    capture(&mut captured, &world, 0, true, &mut fingerprints);
    let owners = |views: &[TableView; N]| {
        rows::<tables::NavalMissionCountries>(&views[2])
            .iter()
            .map(|row| row.country)
            .collect::<Vec<_>>()
    };
    let views = capture(&mut captured, &world, 5, false, &mut fingerprints);
    assert_eq!(owners(&views), [5, 29]);
    // Every receiver's tree was rebuilt: every country is read at hour 6.
    SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let views = capture(&mut captured, &world, 6, false, &mut fingerprints);
    assert_eq!(owners(&views), (1..=30).collect::<Vec<_>>());
    let views = capture(&mut captured, &world, 7, false, &mut fingerprints);
    assert_eq!(owners(&views), [7]);
    // Unobserved: every country, every hour.
    captured.sequence = |_| None;
    let views = capture(&mut captured, &world, 8, false, &mut fingerprints);
    assert_eq!(owners(&views), (1..=30).collect::<Vec<_>>());
}

#[test]
fn without_a_store_the_hour_reads_only_its_slice() {
    let world = build(30);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    captured.capture(&world.slice(), world.base, world.game_state, 5, false, None);
    let views = captured.tables();
    assert_eq!(
        rows::<tables::NavalMissionCountries>(&views[2])
            .iter()
            .map(|row| row.country)
            .collect::<Vec<_>>(),
        [5, 29]
    );
}

#[test]
fn a_short_buffer_truncates_with_its_demand_and_never_publishes_the_family() {
    let world = build(5);
    let mut capacity = Capacities {
        task_forces: 3,
        ..fixture_capacities(&world)
    };
    let mut captured = Captured::with_capacities(&capacity);
    let mut fingerprints = SliceFingerprints::new();
    let views = capture(&mut captured, &world, 0, true, &mut fingerprints);
    assert_eq!(
        captured.outcomes()[0],
        Outcome::Truncated {
            len: 3,
            unread_rows: 17
        }
    );
    // The ledgers need the census, not the task-force rows.
    assert_eq!(rows::<tables::NavalMissionLedgers>(&views[1]).len(), 15);
    for index in [2, 3] {
        assert!(matches!(views[index], TableView::Unavailable { .. }));
    }
    captured.grow(&mut capacity);
    let views = capture(&mut captured, &world, 1, true, &mut fingerprints);
    assert_eq!(rows::<tables::NavalMissionTaskForces>(&views[0]).len(), 20);
    assert_eq!(rows::<tables::NavalMissionCountries>(&views[2]).len(), 5);
}

#[test]
fn a_bad_read_makes_every_table_unavailable() {
    let mut world = build(2);
    let task = world.tasks[2][1];
    world.put(task + ctask_force::NAVAL_MISSION + 8, 1);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    let views = capture(
        &mut captured,
        &world,
        0,
        true,
        &mut SliceFingerprints::new(),
    );
    assert!(views.iter().all(|view| matches!(
        view,
        TableView::Unavailable { reason } if reason == "naval mission identity mismatch"
    )));
}

#[test]
fn five_thousand_countries_capture_completely_after_country_sizing_and_growth() {
    let world = build(5_000);
    let mut capacity = Capacities::default();
    capacity.raise_for_countries(5_001);
    let mut captured = Captured::with_capacities(&capacity);
    let mut fingerprints = SliceFingerprints::new();
    capture(&mut captured, &world, 0, true, &mut fingerprints);
    // Empty fixed storage reports the original demand; ledgers wait for the
    // complete census, then report their own independent growth demand.
    assert!(matches!(
        captured.outcomes()[0],
        Outcome::Truncated {
            len: 0,
            unread_rows: 20_000
        }
    ));
    assert!(matches!(
        captured.outcomes()[1],
        Outcome::Unavailable { .. }
    ));
    captured.grow(&mut capacity);
    // The ledger rows are only counted once the census fits; their demand grows the next
    // frame.
    capture(&mut captured, &world, 0, true, &mut fingerprints);
    assert!(matches!(captured.outcomes()[1], Outcome::Truncated { .. }));
    captured.grow(&mut capacity);
    let views = capture(&mut captured, &world, 0, true, &mut fingerprints);
    assert!(
        captured
            .outcomes()
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
    );
    assert_eq!(
        rows::<tables::NavalMissionTaskForces>(&views[0]).len(),
        20_000
    );
    let ledgers = rows::<tables::NavalMissionLedgers>(&views[1]);
    assert_eq!(ledgers.len(), 15_000);
    // Every ring is found through the native probe.
    assert!(ledgers.iter().all(|row| row.ring));
    assert_eq!(
        rows::<tables::NavalMissionCountries>(&views[2]).len(),
        5_000
    );
    assert_eq!(
        rows::<tables::NavalMissionCommanders>(&views[3]).len(),
        25_000
    );
    // An off hour reads one slice of the family.
    let views = capture(&mut captured, &world, 7, false, &mut fingerprints);
    assert_eq!(
        rows::<tables::NavalMissionCountries>(&views[2]).len(),
        (1..=5_000).filter(|id| id % 24 == 7).count()
    );
}

/// Capture cost through the production guarded copies, one country slice per hour.
#[test]
#[ignore = "timing bench"]
fn naval_mission_bench() {
    for countries in [1_500, 5_000] {
        if std::env::var("BATTLE_PLAN_NAVAL_COUNTRIES").or_else(|_| std::env::var("HOI4_NAVAL_COUNTRIES"))
            .ok()
            .is_some_and(|selected| selected != countries.to_string())
        {
            continue;
        }
        let world = build_world(countries, true);
        let mut capacity = fixture_capacities(&world);
        capacity.raise_for_countries(countries as usize + 1);
        let mut captured = Captured::with_capacities(&capacity);
        let mut fingerprints = SliceFingerprints::new();
        let slice = world.slice();
        for _ in 0..3 {
            captured.capture(
                &slice,
                world.base,
                world.game_state,
                0,
                true,
                Some(&mut fingerprints),
            );
            captured.grow(&mut capacity);
        }
        // SAFETY: the world's bytes are owned, immutable and alive for every read.
        let live = CountedMemory::new(unsafe { LiveMemory::new() });
        let counts = crate::read_accounting::Capture::new();
        let mut hour = 1;
        let mut tables = [0_u64; N];
        let (median, p95) = crate::extended::bench_world::time(48, || {
            let ticks = captured.capture(
                &live,
                world.base,
                world.game_state,
                hour,
                false,
                Some(&mut fingerprints),
            );
            for (sum, tick) in tables.iter_mut().zip(ticks) {
                *sum += tick;
            }
            hour += 1;
        });
        let counts = counts.finish();
        assert!(
            captured
                .outcomes()
                .iter()
                .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
        );
        println!(
            "naval missions countries={countries} task forces={}: capture {median:.1} us (p95 {p95:.1}); qpc ticks per table over 51 hours {tables:?}",
            countries * 5,
        );
        println!(
            "  fixture bytes={} capture arena bytes={}",
            world.bytes.len(),
            captured.allocated_bytes()
        );
        for table in TABLES {
            println!(
                "  {table:?}: {}",
                crate::extended::bench_world::work(counts[table.index()])
            );
        }
    }
}

#[test]
fn ledger_discovery_does_not_depend_on_any_accessible_region_list() {
    let mut world = build(1);
    // Fleet regions can be registered while every mission's accessible-region list is empty.
    for task in world.tasks[1] {
        world.put(
            task + ctask_force::NAVAL_MISSION + cnaval_mission::ACCESSIBLE_REGIONS + 8,
            0,
        );
    }
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    let views = capture(
        &mut captured,
        &world,
        0,
        true,
        &mut SliceFingerprints::new(),
    );
    let ledgers = rows::<tables::NavalMissionLedgers>(&views[1]);
    assert_eq!(ledgers.len(), 3);
    assert_eq!(
        ledgers[0].task_forces,
        vec![task_handle(1, ESCORTING), task_handle(1, RAIDER)]
    );
    assert_eq!(rows::<tables::NavalMissionTaskForces>(&views[0]).len(), 4);
}

#[test]
fn hubs_keep_order_and_repeats_and_grow_from_the_live_demand() {
    let world = build_world(1, true);
    let mut capacity = Capacities {
        hubs: 1,
        ..fixture_capacities(&world)
    };
    let mut captured = Captured::with_capacities(&capacity);
    let mut fingerprints = SliceFingerprints::new();
    let views = capture(&mut captured, &world, 0, true, &mut fingerprints);
    assert!(matches!(views[2], TableView::Truncated { .. }));
    assert_eq!(captured.hubs.required, 3);
    captured.grow(&mut capacity);
    let views = capture(&mut captured, &world, 1, true, &mut fingerprints);
    assert_eq!(
        rows::<tables::NavalMissionCountries>(&views[2])[0].hubs,
        [3, 1, 3]
    );
}

#[test]
fn p2_range_operands_preserve_nonzero_signed_native_values() {
    let mut world = build(1);
    let task = world.tasks[1][STRIKE];
    let stats = u64::from_le_bytes(
        world.bytes[task + cunit::STAT_BLOCK..task + cunit::STAT_BLOCK + 8]
            .try_into()
            .unwrap(),
    ) as usize
        - world.base;
    world.put(stats + native::STAT_RANGE, 123_456);
    world.put(task + native::FUEL, (-17_i64) as u64);
    world.put(task + native::FUEL + 8, 73);
    world.put(task + native::UNDERWAY, 7);
    world.put(
        task + native::REPLENISHMENT,
        u64::from((-2_i32) as u32) | (5_u64 << 32),
    );
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    let views = capture(
        &mut captured,
        &world,
        0,
        true,
        &mut SliceFingerprints::new(),
    );
    let row = rows::<tables::NavalMissionTaskForces>(&views[0])
        .iter()
        .find(|row| row.task_force == task_handle(1, STRIKE))
        .unwrap();
    assert_eq!(
        (
            row.range_base,
            row.fuel,
            row.fuel_requested,
            row.underway,
            row.replenishment,
            row.replenishment_limit
        ),
        (123_456, -17, 73, 7, -2, 5)
    );
}

#[test]
fn bad_owner_keeps_raw_operand_row_and_linked_family_available() {
    for offset in [
        native::MISSION_OWNER + ctask_force::NAVAL_MISSION,
        cunit::EXPEDITIONARY_OWNER,
    ] {
        let mut world = build(2);
        let task = world.tasks[1][ESCORTING];
        world.bytes[task + offset..task + offset + 4].copy_from_slice(&999_999_u32.to_le_bytes());
        let mut captured = Captured::with_capacities(&fixture_capacities(&world));
        let views = capture(
            &mut captured,
            &world,
            0,
            true,
            &mut SliceFingerprints::new(),
        );
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 8 });
        let row = rows::<tables::NavalMissionTaskForces>(&views[0])
            .iter()
            .find(|row| row.task_force == task_handle(1, ESCORTING))
            .unwrap();
        assert_eq!(
            row.mission_owner,
            if offset == native::MISSION_OWNER + ctask_force::NAVAL_MISSION {
                999_999
            } else {
                1
            }
        );
        assert_eq!(rows::<tables::NavalMissionLedgers>(&views[1]).len(), 6);
        assert_eq!(rows::<tables::NavalMissionCountries>(&views[2]).len(), 2);
        assert!(!rows::<tables::NavalMissionCommanders>(&views[3]).is_empty());
    }
}

#[test]
fn stale_leader_has_an_explicit_zero_row() {
    let mut world = build(1);
    let stale = (99_999_u64 << 32) | TYPE;
    world.put(world.fleets[1] + native::FLEET_LEADER, stale);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    let views = capture(
        &mut captured,
        &world,
        0,
        true,
        &mut SliceFingerprints::new(),
    );
    let row = rows::<tables::NavalMissionCommanders>(&views[3])
        .iter()
        .find(|row| row.reference == stale)
        .unwrap();
    assert_eq!(
        (
            row.source,
            row.coordination,
            row.range,
            row.range_factor,
            row.raiding
        ),
        (0, None, None, None, None)
    );
}

#[test]
fn worker_capture_sizes_every_naval_mission_arena_before_first_publication_at_five_thousand_countries()
 {
    let world = build_world(5_000, true);
    let default = Captured::with_capacities(&Capacities::default());
    assert_eq!(default.allocated_bytes(), 0);
    let mut captured = Captured::vacant();
    assert_eq!(captured.allocated_bytes(), 0);
    let memory = world.slice();
    let mut fingerprints = SliceFingerprints::new();
    for sample in 0..2 {
        if sample == 1 {
            // Another pooled frame can have shorter storage with the same live roster.
            captured.task_forces.rows.truncate(1);
            captured.headquarters.first.truncate(1);
            captured.census.first.truncate(1);
            captured.ledgers.rows.truncate(1);
            captured.entries.first.truncate(1);
            captured.scratch.truncate(1);
            captured.hubs.first.truncate(1);
            captured.countries.rows.truncate(1);
            captured.commanders.rows.truncate(1);
        }
        captured.capture_growing(
            &memory,
            world.base,
            world.game_state,
            sample,
            true,
            Some(&mut fingerprints),
        );
        assert_eq!(
            captured.outcomes(),
            [
                Outcome::Rows { len: 20_000 },
                Outcome::Rows { len: 15_000 },
                Outcome::Rows { len: 5_000 },
                Outcome::Rows { len: 25_000 }
            ]
        );
        assert_eq!(captured.headquarters.len, 5_000);
        assert_eq!(captured.census.len, 25_000);
        assert_eq!(captured.hubs.len, 15_000);
        assert_eq!(captured.entries.len, 25_000);
    }
}

#[test]
fn ordinary_strict_missions_grow_on_first_and_later_publications() {
    use super::super::{Capacities, Captured, NavalMissionLoan, OrdinaryPart};

    // The navy part alone: the physical pass and the ledgers that borrow it.
    let mut owner = Captured::with_capacities(&Capacities::default());
    let countries =
        std::sync::Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let mut loan = NavalMissionLoan {
        force_full: true,
        fingerprints: Some(Box::new(SliceFingerprints::new())),
    };
    for (hour, count) in [5_000, 6_000].into_iter().enumerate() {
        let world = build_world(count, true);
        let (base, state, hour) = (world.base, world.game_state, hour as i32);
        let mut parts = owner.take_ordinary_parts(&countries, base, state, hour, Ok(()), loan);
        let request = owner.take_ordinary_request(&countries, base, state, hour);
        for part in &mut parts {
            if matches!(part.part, OrdinaryPart::Navies(_)) {
                part.part.capture(&world.slice(), base, state, hour, Ok(()));
            }
        }
        loan = owner.restore_ordinary_request(request, None, parts.map(|part| (part, None)));
        let count = count as usize;
        let captured = &owner.naval_mission_inputs;
        assert_eq!(
            captured.outcomes(),
            [
                Outcome::Rows { len: count * 4 },
                Outcome::Rows { len: count * 3 },
                Outcome::Rows { len: count },
                Outcome::Rows { len: count * 5 },
            ]
        );
        assert_eq!(captured.headquarters.len, count);
        assert_eq!(captured.census.len, count * 5);
        assert_eq!(captured.hubs.len, count * 3);
        assert_eq!(captured.entries.len, count * 5);
        let tables = captured.tables();
        assert_eq!(
            rows::<tables::NavalMissionTaskForces>(&tables[0]).len(),
            count * 4
        );
        assert_eq!(
            rows::<tables::NavalMissionLedgers>(&tables[1]).len(),
            count * 3
        );
        assert_eq!(
            rows::<tables::NavalMissionCountries>(&tables[2]).len(),
            count
        );
        assert_eq!(
            rows::<tables::NavalMissionCommanders>(&tables[3]).len(),
            count * 5
        );
    }
}

#[test]
fn growing_strict_missions_preserve_fixed_original_reads_and_first_faults() {
    let world = build_world(3, true);
    let capacity = fixture_capacities(&world);
    for failure in [
        None,
        Some(world.game_state + cgame_state::COUNTRIES),
        Some(world.base + world.fleets[1] + cfleet::TASK_FORCES),
        Some(world.base + world.tasks[1][ESCORTING] + ctask_force::ATTACHED_HEADQUARTERS),
        Some(world.base + world.ledgers[1]),
    ] {
        let fixed_memory = TransportMemory {
            world: &world,
            reads: Default::default(),
            fail_once: failure,
            failed: Default::default(),
        };
        let growing_memory = TransportMemory {
            world: &world,
            reads: Default::default(),
            fail_once: failure,
            failed: Default::default(),
        };
        let mut fixed = Captured::with_capacities(&capacity);
        let mut growing = Captured::vacant();
        fixed.capture(&fixed_memory, world.base, world.game_state, 0, true, None);
        growing.capture_growing(&growing_memory, world.base, world.game_state, 0, true, None);
        assert_eq!(growing.outcomes(), fixed.outcomes(), "fault {failure:?}");
        assert_eq!(
            growing_memory.reads.borrow().as_slice(),
            fixed_memory.reads.borrow().as_slice(),
            "fault {failure:?}"
        );
        if failure.is_some() {
            assert!(growing_memory.failed.get(), "fault reached {failure:?}");
            assert_eq!(
                growing_memory
                    .reads
                    .borrow()
                    .iter()
                    .filter(|(address, _)| Some(*address) == failure)
                    .count(),
                1,
                "original failure was not retried"
            );
        } else {
            let fixed = fixed.tables();
            let growing = growing.tables();
            assert_eq!(
                rows::<tables::NavalMissionTaskForces>(&growing[0]),
                rows::<tables::NavalMissionTaskForces>(&fixed[0])
            );
            assert_eq!(
                rows::<tables::NavalMissionLedgers>(&growing[1]),
                rows::<tables::NavalMissionLedgers>(&fixed[1])
            );
            assert_eq!(
                rows::<tables::NavalMissionCountries>(&growing[2]),
                rows::<tables::NavalMissionCountries>(&fixed[2])
            );
            assert_eq!(
                rows::<tables::NavalMissionCommanders>(&growing[3]),
                rows::<tables::NavalMissionCommanders>(&fixed[3])
            );
        }
    }
}
