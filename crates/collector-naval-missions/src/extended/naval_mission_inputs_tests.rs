use super::*;
#[cfg(test)]
use engine_native_memory::memory::{CountedMemory, LiveMemory};
use engine_native::{
    ccharacter,
    cfleet,
    cgame_state,
    cprovince,
    ctask_force,
    cunit,
    cweather_manager::region_weather as sweather_per_region,
};
use engine_native_parts::{
    cnaval_mission,
    cnavy_leader,
    cstrategic_region,
    reference_bucket as bucket,
};
use engine_native_memory::memory::SliceMemory;
use engine_native_naval::{cstrategic_navy, naval_mission_inputs as native};

const GAME_STATE: usize = 0x10000;
const HEAP: usize = 0x0350_0000;
const REGIONS: u32 = 24;
const TYPE: u64 = 4713;
const KINDS: [u32; 5] = [2, 3, 4, 9, 0];
const RAIDER: usize = 1;
const ESCORTING: usize = 2;
const STRIKE: usize = 0;

pub struct World {
    bytes: Vec<u8>,
    pub base: usize,
    pub game_state: usize,
    #[cfg(test)]
    fleets: Vec<usize>,
    #[cfg(test)]
    tasks: Vec<[usize; 5]>,
    #[cfg(test)]
    ledgers: Vec<usize>,
}

use collector_extended_support::bench_world::Heap;

pub fn use_fixture_certificates(captured: &mut Captured) {
    captured.sequence = |_| Some((1, 0));
    captured.ledger_sequence = |_| None;
}

#[cfg(test)]
impl Captured {
    fn fixture(mut self) -> Self {
        use_fixture_certificates(&mut self);
        self
    }
}

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

#[cfg(test)]
fn build(countries: u32) -> World {
    build_world(countries, false)
}

#[cfg(test)]
fn fixture_capacities(world: &World) -> Capacities {
    let mut capacity = Capacities::default();
    capacity
        .presize(&world.slice(), world.base, world.game_state)
        .expect("synthetic naval mission storage demand");
    capacity
}

pub fn build_world(countries: u32, hubs: bool) -> World {
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
            country + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
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
        heap.u64(
            navy + engine_naval_history::country_naval_movement_inputs::MANAGER,
            heap.ptr(manager),
        );
        heap.u32(navy + cstrategic_navy::OWNER, slot);
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
        let navy_leader = heap.alloc(0x500);
        heap.vtable(navy_leader, cnavy_leader::VTABLE_RVA);
        heap.accumulator(navy_leader + engine_native::ctask_force::mission_layout::LEADER_MODIFIERS, &[(0xcc, 2_500)]);
        handles.push((leader(slot), navy_leader));
        let person = heap.alloc(0x100);
        heap.vtable(person, ccharacter::VTABLE_RVA);
        let unit_leader = heap.alloc(0x1000);
        heap.u64(
            person + engine_native::ctask_force::mission_layout::CHARACTER_UNIT_LEADER,
            heap.ptr(unit_leader),
        );
        heap.u32(unit_leader + engine_native::ctask_force::mission_layout::UNIT_LEADER_KIND, 2);
        for (index, source) in engine_native::ctask_force::mission_layout::UNIT_LEADER_SOURCES.into_iter().enumerate() {
            let object = heap.alloc(0x100);
            heap.u64(unit_leader + source, heap.ptr(object));
            heap.accumulator(
                object + engine_native::ctask_force::mission_layout::SOURCE_MODIFIERS,
                &[(0x19, index as i64 + 1)],
            );
        }
        handles.push((character(slot), person));
        let commander = heap.alloc(0x20);
        heap.u64(commander + engine_native::ctask_force::mission_layout::COMMANDER_CHARACTER, character(slot));
        let headquarters = heap.alloc(0x80);
        heap.u64(
            headquarters + engine_native::ctask_force::mission_layout::HEADQUARTERS_COMMANDER,
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
            heap.accumulator(task + engine_native::ctask_force::mission_layout::LOCAL_MODIFIERS, &[(0xcc, -1_000)]);
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
        heap.pointers(
            navy + cstrategic_navy::REGISTERED_MISSIONS,
            &[mission(ESCORTING), mission(RAIDER), mission(STRIKE)],
        );
        fleets.push(fleet);
        tasks.push(own);
    }
    heap.pointers(manager + cstrategic_navy_manager::NAVIES, &navies);
    let capacity = (handles.len() * 2).next_power_of_two().max(8);
    let exponent = capacity.trailing_zeros();
    let overflow = exponent + 2;
    let registry = heap.alloc(creference_registry::SIZE);
    let buckets = heap.alloc((capacity + overflow as usize + 1) * bucket::STRIDE);
    heap.u64(creference_registry::HIGH_ROOT_RVA.0, heap.ptr(registry));
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
        #[cfg(test)]
        fleets,
        #[cfg(test)]
        tasks,
        #[cfg(test)]
        ledgers,
    }
}

impl World {
    pub fn slice(&self) -> SliceMemory<'_> {
        SliceMemory {
            base: self.base,
            bytes: &self.bytes,
        }
    }
    #[cfg(test)]
    fn put(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    #[cfg(test)]
    fn pointer(&self, at: usize) -> usize {
        u64::from_le_bytes(self.bytes[at..at + 8].try_into().unwrap()) as usize
    }
}

#[cfg(test)]
struct TransportMemory<'a> {
    world: &'a World,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    fail_once: Option<usize>,
    failed: std::cell::Cell<bool>,
}

#[cfg(test)]
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
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
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

#[cfg(test)]
fn navy_of(world: &World, slot: u32) -> usize {
    let manager = world.pointer(GAME_STATE + cgame_state::STRATEGIC_NAVY_MANAGER) - world.base;
    let navies = world.pointer(manager + cstrategic_navy_manager::NAVIES) - world.base;
    world.pointer(navies + slot as usize * 8) - world.base
}

#[cfg(test)]
fn region_headers(world: &World, slot: u32) -> usize {
    world.pointer(navy_of(world, slot) + cstrategic_navy::REGION_MISSIONS)
}

#[cfg(test)]
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

#[cfg(test)]
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
    let mut captured = Captured::with_capacities(&fixture_capacities(world)).fixture();
    captured.capture_skipping(&memory, world.base, world.game_state, 71, skip);
    let views = captured.tables();
    let ledgers = rows::<tables::NavalMissionLedgers>(&views[1]).to_vec();
    (ledgers, captured.ledger_counts(), memory.reads.into_inner())
}

#[test]
fn a_navy_without_registered_missions_reads_no_region_header() {
    let mut world = build(3);
    empty_navy(&mut world, 2, true);
    let headers = region_headers(&world, 2);
    let reads_headers = |reads: &[(usize, usize)]| reads.iter().any(|&(at, _)| at == headers);
    let (all, counts, reads) = ledger_read(&world, LedgerSkip::NEVER);
    assert!(reads_headers(&reads));
    assert_eq!(counts.read, 3);
    assert!(all.iter().all(|row| row.owner != 2) && !all.is_empty());
    let (ledgers, counts, reads) = ledger_read(&world, LedgerSkip { enabled: true });
    assert_eq!(ledgers, all);
    assert!(!reads_headers(&reads));
    assert_eq!(
        counts,
        LedgerCounts {
            skipped: 1,
            read: 2,
            carried: 0,
        }
    );
}

#[test]
fn a_fleet_leader_reference_resolves_again_only_after_its_certificate_moves() {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    static MOVED: AtomicUsize = AtomicUsize::new(0);
    fn sequence(leader: usize) -> Option<(u64, u64)> {
        Some((1, u64::from(MOVED.load(Relaxed) == leader)))
    }
    let offset = std::cell::Cell::new(0x1000_usize);
    let resolve = |reference: u64| -> Result<Option<usize>, &'static str> {
        Ok(Some(reference as usize + offset.get()))
    };
    let authenticated = std::cell::Cell::new(0);
    let mut leaders = Leaders::default();
    let take = |leaders: &mut Leaders, reference: u64| {
        leaders.leader(reference, sequence, &resolve, |_| {
            authenticated.set(authenticated.get() + 1);
            Ok(())
        })
    };
    assert_eq!(take(&mut leaders, 7), Some(Some((0x1007, (1, 0)))));
    assert_eq!(take(&mut leaders, 9), Some(Some((0x1009, (1, 0)))));
    assert_eq!(leaders.finish(), [0, 2]);
    offset.set(0x2000);
    assert_eq!(take(&mut leaders, 7), Some(Some((0x1007, (1, 0)))));
    assert_eq!(take(&mut leaders, 9), Some(Some((0x1009, (1, 0)))));
    assert_eq!(leaders.finish(), [2, 0]);
    MOVED.store(0x1007, Relaxed);
    assert_eq!(take(&mut leaders, 7), Some(Some((0x2007, (1, 0)))));
    assert_eq!(take(&mut leaders, 9), Some(Some((0x1009, (1, 0)))));
    assert_eq!(leaders.finish(), [1, 1]);
    assert_eq!(authenticated.get(), 3);
}

#[test]
fn a_certified_navy_republishes_its_held_ledgers_and_reads_only_presence() {
    const TEST: &str = "extended::naval_mission_inputs::tests::a_certified_navy_republishes_its_held_ledgers_and_reads_only_presence";
    const CHILD: &str = "BATTLEPLAN_LIVE_LEDGER_CERTIFICATE_TEST";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    fn read(
        captured: &mut Captured,
        world: &World,
        hour: i32,
        force_full: bool,
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
        captured.capture_with_own_navies(
            &memory,
            world.base,
            world.game_state,
            hour,
            force_full,
            None,
            LedgerSkip { enabled: true },
        );
        let views = captured.tables();
        let ledgers = rows::<tables::NavalMissionLedgers>(&views[1]).to_vec();
        (ledgers, captured.ledger_counts(), memory.reads.into_inner())
    }
    let mut world = build(3);
    let ledger_changed = collector_missions::naval_map_changes::ledger_observation_fixture(3);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    captured.sequence = |_| Some((1, 0));
    let (all, counts, _) = read(&mut captured, &world, 0, true);
    assert_eq!((counts.read, counts.carried), (3, 0));
    let headers = region_headers(&world, 2);
    let first = escort_regions(2)[0] as usize;
    let count = headers - world.base + first * cstrategic_navy::REGION_MISSIONS_STRIDE + 12;
    world.bytes[count..count + 4].fill(0);
    let (ledgers, counts, reads) = read(&mut captured, &world, 1, false);
    assert_eq!((counts.read, counts.carried), (0, 3));
    assert_eq!(ledgers, all);
    assert!(reads.iter().all(|&(at, _)| at != headers));
    ledger_changed(world.base + navy_of(&world, 2));
    let (ledgers, counts, _) = read(&mut captured, &world, 2, false);
    assert_eq!((counts.read, counts.carried), (1, 2));
    assert_eq!(ledgers.len(), all.len() - 1);
    let (_, counts, _) = read(&mut captured, &world, 3, true);
    assert_eq!((counts.read, counts.carried), (3, 0));
}

#[test]
fn an_empty_manager_does_not_heal_a_missing_strict_country_navy() {
    let mut world = build(1);
    let capacity = fixture_capacities(&world);
    let manager = world.pointer(GAME_STATE + cgame_state::STRATEGIC_NAVY_MANAGER) - world.base;
    world.put(manager + cstrategic_navy_manager::NAVIES, 0);
    world.put(manager + cstrategic_navy_manager::NAVIES + 8, 0);
    let mut captured = Captured::with_capacities(&capacity).fixture();
    captured.capture(&world.slice(), world.base, world.game_state, 71, true, None);
    assert!(matches!(
        captured.ledgers.outcome,
        Outcome::Unavailable {
            reason: "navy manager country outside array"
        }
    ));
}

#[cfg(test)]
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

pub fn rows<S: replay_data::rows::TableSchema>(view: &TableView) -> &[S::Row] {
    let TableView::Complete(rows) = view else {
        panic!("incomplete {view:?}");
    };
    rows.rows::<S>().expect("table rows")
}

#[test]
fn a_full_hour_records_every_raiding_and_escort_task_force_ledger_and_country() {
    let world = build(3);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
    let mut fingerprints = SliceFingerprints::new();
    let views = capture(&mut captured, &world, 0, true, &mut fingerprints);
    let task_forces = rows::<tables::NavalMissionTaskForces>(&views[0]);
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
            weather_coordination: Some(-2_000),
            weather_raiding: None,
            leader: leader(1),
            headquarters: vec![character(1)].into(),
            range_fleet: (1 << 32) | 61,
            range_begin: 0,
            range_end: 3,
            convoys: 12,
            ..Default::default()
        }
    );
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
            escort_tree: replay_data::rows::Recorded::Value(None),
            ..Default::default()
        }
    );
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
fn foreign_navy_keeps_its_ledgers_under_its_slots_country() {
    let mut world = build(3);
    let owner = navy_of(&world, 1) + cstrategic_navy::OWNER;
    world.bytes[owner..owner + 4].copy_from_slice(&2_u32.to_le_bytes());
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
    let views = capture(
        &mut captured,
        &world,
        0,
        true,
        &mut SliceFingerprints::new(),
    );
    let ledgers = rows::<tables::NavalMissionLedgers>(&views[1]);
    assert_eq!(ledgers.len(), 9);
    assert_eq!(ledgers.iter().filter(|row| row.owner == 1).count(), 3);
}

#[test]
fn an_unlisted_escort_still_has_its_region_presence_input() {
    let mut world = build(1);
    let raider = world.tasks[1][RAIDER] + ctask_force::NAVAL_MISSION;
    let data = world.ledgers[1];
    world.put(data, (world.base + raider) as u64);
    world.put(data + 8, (world.base + raider) as u64);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
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
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
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

#[cfg(test)]
fn without_escort_demand(world: &mut World) {
    let owner = ctask_force::NAVAL_MISSION + native::MISSION_OWNER;
    for tasks in world.tasks.iter().skip(1) {
        let at = tasks[ESCORTING] + owner;
        world.bytes[at..at + 4].copy_from_slice(&999_999_u32.to_le_bytes());
    }
}

#[test]
fn a_quiet_hour_reads_only_countries_whose_commanders_changed() {
    let mut world = build(30);
    without_escort_demand(&mut world);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
    let mut fingerprints = SliceFingerprints::new();
    capture(&mut captured, &world, 0, true, &mut fingerprints);
    let owners = |views: &[TableView; N]| {
        rows::<tables::NavalMissionCountries>(&views[2])
            .iter()
            .map(|row| row.country)
            .collect::<Vec<_>>()
    };
    let views = capture(&mut captured, &world, 5, false, &mut fingerprints);
    assert!(owners(&views).is_empty());
    assert!(!captured.selection().contains(5) && !captured.selection().contains(29));
    let fleet = world.fleets[7];
    world.put(fleet + native::FLEET_LEADER, leader(99));
    let views = capture(&mut captured, &world, 6, false, &mut fingerprints);
    assert_eq!(owners(&views), [7]);
    assert_eq!(captured.selection().early_count, 1);
    assert_eq!(rows::<tables::NavalMissionTaskForces>(&views[0]).len(), 120);
    assert_eq!(rows::<tables::NavalMissionLedgers>(&views[1]).len(), 90);
}

#[test]
fn default_mission_capture_requires_a_live_escort_certificate() {
    let world = build(3);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world));
    let memory = world.slice();
    let countries = ccountry::Reader::new(&memory, world.base, world.game_state).unwrap();
    let country = countries.at(1).unwrap().unwrap();
    assert_eq!((captured.sequence)(country.pointer), None);
    captured.capture(&memory, world.base, world.game_state, 0, true, None);
    assert!(!captured.escort_owners.is_empty());
    assert_eq!(captured.escort_retries.len(), captured.escort_owners.len());
}

#[test]
fn a_refused_escort_tree_is_read_again_at_the_next_hour() {
    let world = build(3);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
    let mut fingerprints = SliceFingerprints::new();
    let read = |fingerprints: &mut SliceFingerprints, hour| {
        let mut selection = SliceSelection::new();
        selection.reset_hooked(hour, false);
        for country in 1..=3 {
            selection.scan_marking(fingerprints, country, Some(u64::from(country)));
        }
        (1..=3_u32)
            .filter(|&country| selection.contains(country))
            .collect::<Vec<_>>()
    };
    assert_eq!(read(&mut fingerprints, 1), [1, 2, 3]);
    assert!(read(&mut fingerprints, 2).is_empty());
    captured.retry_escort_tree(2).unwrap();
    captured.retry_refused_escort_trees(&mut fingerprints);
    assert_eq!(read(&mut fingerprints, 3), [2]);
    assert!(read(&mut fingerprints, 4).is_empty());
    captured.escort_retries.clear();
    for country in 0..100 {
        captured.retry_escort_tree(country).unwrap();
    }
    assert_eq!(captured.escort_retries.len(), 100);
    assert!(captured.escort_retries.capacity() < 1_000);
}

#[test]
fn a_moved_certificate_is_read_within_the_hour() {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut world = build(30);
    without_escort_demand(&mut world);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
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
    assert!(owners(&views).is_empty());
    SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let views = capture(&mut captured, &world, 6, false, &mut fingerprints);
    assert_eq!(owners(&views), (1..=30).collect::<Vec<_>>());
    let views = capture(&mut captured, &world, 7, false, &mut fingerprints);
    assert!(owners(&views).is_empty());
    captured.sequence = |_| None;
    let views = capture(&mut captured, &world, 8, false, &mut fingerprints);
    assert_eq!(owners(&views), (1..=30).collect::<Vec<_>>());
}

#[test]
fn without_a_store_the_hour_reads_every_country() {
    let world = build(30);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
    captured.capture(&world.slice(), world.base, world.game_state, 5, false, None);
    let views = captured.tables();
    assert_eq!(
        rows::<tables::NavalMissionCountries>(&views[2])
            .iter()
            .map(|row| row.country)
            .collect::<Vec<_>>(),
        (1..=30).collect::<Vec<_>>()
    );
}

#[test]
fn a_short_task_force_buffer_grows_to_the_whole_census_in_the_pass() {
    let world = build(5);
    let capacity = Capacities {
        task_forces: 3,
        ..fixture_capacities(&world)
    };
    let mut captured = Captured::with_capacities(&capacity).fixture();
    let mut fingerprints = SliceFingerprints::new();
    let views = capture(&mut captured, &world, 0, true, &mut fingerprints);
    assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 20 });
    assert_eq!(rows::<tables::NavalMissionLedgers>(&views[1]).len(), 15);
    assert_eq!(rows::<tables::NavalMissionTaskForces>(&views[0]).len(), 20);
    assert_eq!(rows::<tables::NavalMissionCountries>(&views[2]).len(), 5);
}

#[test]
fn a_bad_mission_read_refuses_dependents_and_preserves_independent_map_witnesses() {
    let mut world = build(2);
    let task = world.tasks[2][1];
    world.put(task + ctask_force::NAVAL_MISSION + 8, 1);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
    let views = capture(
        &mut captured,
        &world,
        0,
        true,
        &mut SliceFingerprints::new(),
    );
    for index in [0, 2, 3] {
        assert!(
            matches!(&views[index],TableView::Unavailable{reason}if reason=="naval mission identity mismatch")
        );
    }
    assert!(
        matches!(&views[1],TableView::Unavailable{reason}if reason=="naval mission ledger census truncated")
    );
    assert!(captured.map_roster_complete);
    for index in [4, 5] {
        if let TableView::Unavailable { reason } = &views[index] {
            assert_ne!(
                reason, "naval mission identity mismatch",
                "independent map failures keep their own reason"
            );
        }
    }
}

#[test]
#[ignore = "timing bench"]
fn naval_mission_bench() {
    for countries in [1_500, 5_000] {
        if std::env::var("BATTLEPLAN_NAVAL_COUNTRIES").or_else(|_| std::env::var("HOI4_NAVAL_COUNTRIES"))
            .ok()
            .is_some_and(|selected| selected != countries.to_string())
        {
            continue;
        }
        let world = build_world(countries, true);
        let mut capacity = fixture_capacities(&world);
        capacity.raise_for_countries(countries as usize + 1);
        let mut captured = Captured::with_capacities(&capacity).fixture();
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
        let live = CountedMemory::new(unsafe { LiveMemory::new() });
        let counts = collector_memory::read_accounting::Capture::new();
        let mut hour = 1;
        let mut tables = [0_u64; N];
        let (median, p95) = collector_extended_support::bench_world::time(48, || {
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
                .take(4)
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
                collector_extended_support::bench_world::work(counts[table.index()])
            );
        }
    }
}

#[test]
fn ledger_discovery_does_not_depend_on_any_accessible_region_list() {
    let mut world = build(1);
    for task in world.tasks[1] {
        world.put(
            task + ctask_force::NAVAL_MISSION + cnaval_mission::ACCESSIBLE_REGIONS + 8,
            0,
        );
    }
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
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
fn hubs_keep_order_and_repeats_and_grow_to_the_live_demand() {
    let world = build_world(1, true);
    let capacity = Capacities {
        hubs: 1,
        ..fixture_capacities(&world)
    };
    let mut captured = Captured::with_capacities(&capacity).fixture();
    let mut fingerprints = SliceFingerprints::new();
    let views = capture(&mut captured, &world, 0, true, &mut fingerprints);
    assert_eq!(captured.hubs.required, 3);
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
    world.put(stats + engine_native::ctask_force::mission_layout::STAT_RANGE, 123_456);
    world.put(task + engine_native::ctask_force::mission_layout::FUEL, (-17_i64) as u64);
    world.put(task + engine_native::ctask_force::mission_layout::FUEL + 8, 73);
    world.put(task + engine_native::ctask_force::mission_layout::UNDERWAY, 7);
    world.put(
        task + engine_native::ctask_force::mission_layout::REPLENISHMENT,
        u64::from((-2_i32) as u32) | (5_u64 << 32),
    );
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
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
        let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
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
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
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
        let mut fixed = Captured::with_capacities(&capacity).fixture();
        let mut growing = Captured::vacant().fixture();
        fixed.capture(&fixed_memory, world.base, world.game_state, 0, true, None);
        growing.capture(&growing_memory, world.base, world.game_state, 0, true, None);
        assert_eq!(
            &growing.outcomes()[..4],
            &fixed.outcomes()[..4],
            "fault {failure:?}"
        );
        let additional = growing_memory.reads.borrow();
        let original = fixed_memory.reads.borrow();
        let mut remaining = additional.iter();
        for read in original.iter() {
            assert!(
                remaining.any(|candidate| candidate == read),
                "original read {read:?}, fault {failure:?}"
            );
        }
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

#[test]
fn dominance_ranges_read_only_omitted_siblings_of_registered_strike_fleets() {
    let world = build(2);
    let memory = TransportMemory {
        world: &world,
        reads: Default::default(),
        fail_once: None,
        failed: Default::default(),
    };
    let mut captured = Captured::vacant().fixture();
    captured.capture(&memory, world.base, world.game_state, 0, true, None);
    let views = captured.tables();
    let ranges = rows::<tables::NavalDominanceTaskForceRanges>(&views[6]);
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0].task_force, task_handle(1, 4));
    assert_eq!(ranges[1].task_force, task_handle(2, 4));
    assert_eq!(rows::<tables::NavalMissionTaskForces>(&views[0]).len(), 8);
    let mut world = build(2);
    empty_navy(&mut world, 1, true);
    let memory = TransportMemory {
        world: &world,
        reads: Default::default(),
        fail_once: None,
        failed: Default::default(),
    };
    let mut captured = Captured::vacant().fixture();
    captured.capture(&memory, world.base, world.game_state, 0, true, None);
    let views = captured.tables();
    let ranges = rows::<tables::NavalDominanceTaskForceRanges>(&views[6]);
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].task_force, task_handle(2, 4));
    let idle_header = world.base + world.tasks[1][4] + ctask_force::NAVAL_MISSION;
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == idle_header)
    );
}

#[test]
fn failed_optional_sibling_does_not_refuse_existing_mission_tables() {
    let world = build(2);
    let memory = TransportMemory {
        world: &world,
        reads: Default::default(),
        fail_once: Some(world.base + world.tasks[1][4] + ctask_force::NAVAL_MISSION),
        failed: Default::default(),
    };
    let mut captured = Captured::vacant().fixture();
    captured.capture(&memory, world.base, world.game_state, 0, true, None);
    assert!(memory.failed.get());
    let views = captured.tables();
    assert_eq!(rows::<tables::NavalMissionTaskForces>(&views[0]).len(), 8);
    assert!(matches!(
        captured.ranges_outcome,
        Outcome::Unavailable { .. }
    ));
    assert!(matches!(captured.countries.outcome, Outcome::Rows { .. }));
    assert!(matches!(captured.commanders.outcome, Outcome::Rows { .. }));
}

#[test]
fn mission_weather_failure_preserves_the_independent_physical_map_census() {
    let mut world = build(1);
    let capacity = fixture_capacities(&world);
    world.put(GAME_STATE + cgame_state::WEATHER_MANAGER, 0);
    let mut captured = Captured::with_capacities(&capacity).fixture();
    captured.capture(&world.slice(), world.base, world.game_state, 71, true, None);
    let Outcome::Unavailable {
        reason: mission_reason,
    } = captured.task_forces.outcome
    else {
        panic!("missing weather must refuse mission operands");
    };
    assert!(
        captured.map_roster_complete,
        "map membership has no mission-weather dependency"
    );
    assert_eq!(
        captured.ledgers.outcome,
        Outcome::Rows { len: 3 },
        "a complete authenticated roster joins ledgers despite independent modifier failure"
    );
    for outcome in [captured.mines_outcome, captured.accidents_outcome] {
        if let Outcome::Unavailable { reason } = outcome {
            assert_ne!(
                reason, mission_reason,
                "map refusal must name its own missing witness"
            );
            assert_ne!(reason, "naval mission ledger census truncated");
        }
    }
}

#[test]
fn unavailable_map_certificates_preserve_the_original_mission_products() {
    let world = build(1);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
    let views = capture(
        &mut captured,
        &world,
        71,
        true,
        &mut SliceFingerprints::new(),
    );
    assert!(captured.map_roster_complete);
    for view in &views[..4] {
        assert!(matches!(view, TableView::Complete(_)), "{view:?}");
    }
    assert!(matches!(
        captured.mines_outcome,
        Outcome::Unavailable { .. }
    ));
    assert!(matches!(
        captured.accidents_outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn an_empty_mission_navy_still_observes_its_map_certificates() {
    let mut world = build(1);
    empty_navy(&mut world, 1, true);
    let mut captured = Captured::with_capacities(&fixture_capacities(&world)).fixture();
    captured.capture(&world.slice(), world.base, world.game_state, 71, true, None);
    assert!(captured.map_roster_complete);
    assert_eq!(captured.ledgers.outcome, Outcome::Rows { len: 0 });
    assert!(matches!(
        captured.mines_outcome,
        Outcome::Unavailable { .. }
    ));
    assert!(matches!(
        captured.accidents_outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn patrol_members_reuse_original_stats_and_do_not_duplicate_mission_context() {
    let world = build(1);
    let memory = TransportMemory {
        world: &world,
        reads: Default::default(),
        fail_once: None,
        failed: Default::default(),
    };
    let mut captured = Captured::vacant().fixture();
    captured.capture(&memory, world.base, world.game_state, 0, true, None);
    let views = captured.tables();
    let sources = rows::<tables::NavalPatrolMissionInputs>(&views[7]);
    assert_eq!(
        sources.len(),
        3,
        "only distinct registered identities are demanded"
    );
    assert!(sources.iter().all(|source| !source.local_detection_known
        && !source.weather_detection_known
        && !source.headquarters_known
        && source.headquarters.is_empty()
        && source.leader == 0));
    assert!(captured.patrol_headquarters.is_empty());
    let reads = memory.reads.borrow();
    for task in &world.tasks[1][..4] {
        assert_eq!(
            reads
                .iter()
                .filter(|&&(at, _)| at == world.base + task + cunit::STAT_BLOCK)
                .count(),
            1,
            "a patrol source must reuse the strict row's original selector"
        );
    }
    let header = world.base + world.tasks[1][ESCORTING] + ctask_force::NAVAL_MISSION;
    assert_eq!(
        reads.iter().filter(|&&(at, _)| at == header).count(),
        1,
        "repeated escort registrations must reuse one original mission header"
    );
    drop(reads);
    let first = rows::<tables::NavalMissionTaskForces>(&views[0])
        .iter()
        .find(|row| row.task_force == task_handle(1, ESCORTING))
        .unwrap()
        .headquarters
        .clone();
    captured.capture(&memory, world.base, world.game_state, 1, true, None);
    let after = captured.tables();
    let second = &rows::<tables::NavalMissionTaskForces>(&after[0])
        .iter()
        .find(|row| row.task_force == task_handle(1, ESCORTING))
        .unwrap()
        .headquarters;
    assert!(
        Arc::ptr_eq(&first, second),
        "existing mission lists also share quiet frames"
    );
}

#[test]
fn omitted_patrol_context_shares_headquarters_until_references_change() {
    let mut world = build(1);
    for index in [STRIKE, ESCORTING] {
        let at = world.tasks[1][index] + ctask_force::MISSION;
        world.bytes[at..at + 4].copy_from_slice(&1_u32.to_le_bytes());
    }
    let mut captured = Captured::vacant().fixture();
    captured.capture(&world.slice(), world.base, world.game_state, 0, true, None);
    let id = task_handle(1, ESCORTING);
    let source = |captured: &Captured| {
        let views = captured.tables();
        rows::<tables::NavalPatrolMissionInputs>(&views[7])
            .iter()
            .find(|row| row.task_force == id)
            .unwrap()
            .clone()
    };
    let first = source(&captured);
    assert!(
        first.headquarters_known && first.local_detection_known && first.weather_detection_known
    );
    assert_eq!(first.headquarters.as_slice(), &[character(1)]);
    captured.capture(&world.slice(), world.base, world.game_state, 1, true, None);
    let second = source(&captured);
    assert!(
        Arc::ptr_eq(&first.headquarters, &second.headquarters),
        "unchanged source lists allocate once"
    );
    let header = world.tasks[1][ESCORTING] + ctask_force::ATTACHED_HEADQUARTERS;
    let references = world.pointer(header) - world.base;
    let headquarters = world.pointer(references) - world.base;
    let commander = world.pointer(headquarters + engine_native::ctask_force::mission_layout::HEADQUARTERS_COMMANDER) - world.base;
    world.put(commander + engine_native::ctask_force::mission_layout::COMMANDER_CHARACTER, 0);
    captured.capture(&world.slice(), world.base, world.game_state, 2, true, None);
    let third = source(&captured);
    assert_eq!(third.headquarters.as_slice(), &[0]);
    assert!(!Arc::ptr_eq(&second.headquarters, &third.headquarters));
    assert_eq!(
        first.headquarters.as_slice(),
        &[character(1)],
        "published source lists stay immutable"
    );
}

#[test]
fn patrol_leaf_failure_preserves_existing_missions_ranges_and_map_outcomes() {
    let world = build(1);
    let mut baseline = Captured::vacant().fixture();
    baseline.capture(&world.slice(), world.base, world.game_state, 0, true, None);
    let task = world.tasks[1][ESCORTING];
    let stats = world.pointer(task + cunit::STAT_BLOCK);
    let memory = TransportMemory {
        world: &world,
        reads: Default::default(),
        fail_once: Some(stats + engine_native_naval::naval_patrol_inputs::SURFACE_DETECTION),
        failed: Default::default(),
    };
    let mut failed = Captured::vacant().fixture();
    failed.capture(&memory, world.base, world.game_state, 0, true, None);
    assert!(memory.failed.get());
    let before = baseline.tables();
    let after = failed.tables();
    assert_eq!(
        rows::<tables::NavalMissionTaskForces>(&before[0]),
        rows::<tables::NavalMissionTaskForces>(&after[0])
    );
    assert_eq!(
        rows::<tables::NavalDominanceTaskForceRanges>(&before[6]),
        rows::<tables::NavalDominanceTaskForceRanges>(&after[6])
    );
    assert_eq!(baseline.mines_outcome, failed.mines_outcome);
    assert_eq!(baseline.accidents_outcome, failed.accidents_outcome);
    let source = rows::<tables::NavalPatrolMissionInputs>(&after[7])
        .iter()
        .find(|row| row.task_force == task_handle(1, ESCORTING))
        .unwrap();
    assert_eq!(source.surface_detection, None);
    assert_eq!(
        source.carrier_detection,
        Some(0),
        "independent source leaves survive a failed word"
    );
}

#[test]
fn incomplete_patrol_witness_refuses_only_its_optional_table() {
    let world = build(1);
    let mut captured = Captured::vacant().fixture();
    captured.capture(&world.slice(), world.base, world.game_state, 0, true, None);
    let before = captured.tables();
    let outcomes = captured.outcomes();
    let member = captured.census.first[..captured.census.len]
        .iter_mut()
        .find(|member| member.id == task_handle(1, STRIKE))
        .unwrap();
    member.visit = None;
    captured.patrol.clear();
    let memory = world.slice();
    let countries = ccountry::Reader::new(&memory, world.base, world.game_state).unwrap();
    let reason = captured
        .read_patrol_members(&memory, world.base, world.game_state, &countries)
        .unwrap_err();
    captured.patrol_outcome = Outcome::Unavailable { reason };
    let after = captured.tables();
    assert_eq!(
        rows::<tables::NavalMissionTaskForces>(&before[0]),
        rows::<tables::NavalMissionTaskForces>(&after[0])
    );
    assert_eq!(
        rows::<tables::NavalDominanceTaskForceRanges>(&before[6]),
        rows::<tables::NavalDominanceTaskForceRanges>(&after[6])
    );
    assert_eq!(&captured.outcomes()[..7], &outcomes[..7]);
    assert!(matches!(after[7], TableView::Unavailable { .. }));
}

#[test]
fn patrol_never_retries_an_optional_range_source_failed_selector() {
    let mut world = build(1);
    let source = world.tasks[1][ESCORTING];
    let kind = source + ctask_force::MISSION;
    world.bytes[kind..kind + 4].copy_from_slice(&1_u32.to_le_bytes());
    let selector = world.base + source + cunit::STAT_BLOCK;
    let memory = TransportMemory {
        world: &world,
        reads: Default::default(),
        fail_once: Some(selector),
        failed: Default::default(),
    };
    let mut captured = Captured::vacant().fixture();
    captured.capture(&memory, world.base, world.game_state, 0, true, None);
    assert!(memory.failed.get());
    assert!(matches!(
        captured.ranges_outcome,
        Outcome::Unavailable { .. }
    ));
    let views = captured.tables();
    assert_eq!(rows::<tables::NavalMissionTaskForces>(&views[0]).len(), 3);
    let packet = rows::<tables::NavalPatrolMissionInputs>(&views[7])
        .iter()
        .find(|row| row.task_force == task_handle(1, ESCORTING))
        .unwrap();
    assert_eq!(packet.surface_detection, None);
    assert_eq!(packet.carrier_detection, None);
    assert!(
        !packet.local_detection_known
            && !packet.weather_detection_known
            && !packet.headquarters_known,
        "failed original context is never replaced by a second source observation"
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == selector)
            .count(),
        1
    );
}

#[test]
fn owned_mission_tail_keeps_reordered_weather_and_shared_headquarters_exact() {
    for count in [0, 1, 1_500, 5_000] {
        let mut captured = Captured::vacant().fixture();
        captured.weather = (1..=7)
            .map(|region| {
                (
                    region,
                    [Value {
                        raw: i64::from(region),
                        present: true,
                    }; 9],
                )
            })
            .collect();
        captured.weather_len = captured.weather.len();
        let mut expected = Vec::new();
        for index in 0..count {
            let region = (index % 8) as u32;
            let additional_range = index % 5 == 0;
            let patrol_context = index % 11 == 0;
            let weather = [Value {
                raw: -17,
                present: true,
            }; 9];
            captured.task_forces.rows.push(TaskForce {
                row: Operands {
                    id: index as u64 + 1,
                    region,
                    ..Default::default()
                },
                weather,
                additional_range,
                patrol_context,
                ..Default::default()
            });
            let weather = if !additional_range && !patrol_context && region != 0 {
                [Value {
                    raw: i64::from(region),
                    present: true,
                }; 9]
            } else {
                weather
            };
            expected.push(weather.map(|value| (value.raw, value.present)));
            if !patrol_context {
                captured.task_index.push((index as u64 + 1, 0));
            }
            captured
                .mission_headquarters
                .push((index as u64 + 1, Arc::new(vec![index as u64])));
        }
        captured
            .mission_headquarters
            .push((count as u64 + 1, Arc::new(vec![99])));
        let expected_heads = captured
            .task_index
            .iter()
            .map(|&(id, _)| {
                (
                    id,
                    Arc::clone(&captured.mission_headquarters[id as usize - 1].1),
                )
            })
            .collect::<Vec<_>>();
        captured.task_forces.rows.reverse();
        expected.reverse();
        captured.task_index.reverse();
        captured.task_forces.outcome = Outcome::Rows { len: count };
        captured.weather_pending = true;
        captured.headquarters_pending = true;
        captured.decode_owned();
        assert_eq!(
            captured.task_forces.rows.iter()
                .map(|row| row.weather.map(|value| (value.raw, value.present)))
                .collect::<Vec<_>>(),
            expected,
        );
        assert_eq!(captured.mission_headquarters.len(), expected_heads.len());
        for ((id, actual), (expected_id, expected)) in
            captured.mission_headquarters.iter().zip(&expected_heads)
        {
            assert_eq!(id, expected_id);
            assert!(Arc::ptr_eq(actual, expected));
        }
        captured.decode_owned();
        captured.headquarters_pending = true;
        captured.reset();
        assert!(captured.mission_headquarters.is_empty());
    }
}

#[test]
fn refused_mission_weather_or_headquarters_admission_does_not_settle_partial_state() {
    let mut captured = Captured::vacant().fixture();
    captured.weather = vec![(
        1,
        [Value {
            raw: 123,
            present: true,
        }; 9],
    )];
    captured.weather_len = 1;
    captured.task_forces.rows.push(TaskForce {
        row: Operands {
            id: 1,
            region: 1,
            ..Default::default()
        },
        ..Default::default()
    });
    captured.task_forces.outcome = Outcome::Rows { len: 1 };
    let held = Arc::new(vec![7]);
    captured.mission_headquarters.push((2, Arc::clone(&held)));
    captured.decode_owned();
    assert_eq!(captured.task_forces.rows[0].weather[0].raw, 0);
    assert!(Arc::ptr_eq(&captured.mission_headquarters[0].1, &held));
}
