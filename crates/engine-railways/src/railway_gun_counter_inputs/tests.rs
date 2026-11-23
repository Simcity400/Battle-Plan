use super::*;
use engine_native::{carmy, cland_combat, cland_combatant};
use crate::cnaval_unit_transfer;
use engine_native_memory::memory::SliceMemory;
use engine_native_parts::reference_bucket;

const STATE: usize = 0x1000;
const COUNTRIES: usize = 0x2000;
const LINKS: usize = 0x2100;
const COUNTRY: usize = 0x3000;
const ROSTER: usize = 0x4000;
const PROVINCES: usize = 0x5000;
const FROM: usize = 0x6000;
const TO: usize = 0x6100;
const GUN: usize = 0x7000;
const ORDINARY: usize = 0x8000;
const ALTERNATE: usize = 0x8100;
const COMBATS: usize = 0x8200;
const COMBAT: usize = 0x9000;
const SIDE: usize = 0x9100;
const SIDE_COUNTRY: usize = 0x9200;
const COMBAT_TWO: usize = 0x9300;
const SIDE_TWO: usize = 0x9400;
const SIDE_TWO_COUNTRY: usize = 0x9500;
const TABLE: usize = 0xa000;
const BUCKETS: usize = 0xa100;
const GROUP_TABLE: usize = 0xb000;
const GROUP_BUCKETS: usize = 0xb100;
const GROUP: usize = 0xc000;
const SUBUNIT: usize = 0xd000;
const TRANSFER: usize = 0xe000;
const RAIL_MANAGER: usize = 0xf000;
const RAIL_SLOTS: usize = 0xf100;
const RAIL_INFO: usize = 0xf200;
const RAIL_BUILDING: usize = 0xf300;
const DEFINITION: usize = 0xf600;
const GUN_HANDLE: u64 = (1_u64 << 32) | 4713;
const GROUP_HANDLE: u64 = (3_u64 << 32) | corders_group::REFERENCE_TYPE as u64;
const TRANSFER_HANDLE: u64 = (4_u64 << 32) | 4713;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address..address + value.len()].copy_from_slice(value);
}

fn put64(bytes: &mut [u8], address: usize, value: u64) {
    put(bytes, address, &value.to_le_bytes());
}

fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put64(bytes, at, data as u64);
    put(bytes, at + 8, &count.to_le_bytes());
    put(bytes, at + 12, &count.to_le_bytes());
}

fn bucket(handle: u64) -> usize {
    bucket_at(handle, BUCKETS)
}

fn bucket_at(handle: u64, data: usize) -> usize {
    let hash = handle
        .rotate_left(32)
        .wrapping_mul(creference_registry::HASH_MULTIPLIER);
    data + (((hash >> 32) as u32 ^ hash as u32) & 7) as usize * reference_bucket::STRIDE
}

fn registry(bytes: &mut [u8], table: usize, buckets: usize, entries: &[(u64, usize)]) {
    put64(bytes, table + creference_registry::ENTRIES, buckets as u64);
    put(
        bytes,
        table + creference_registry::COUNT,
        &(entries.len() as u32).to_le_bytes(),
    );
    put(
        bytes,
        table + creference_registry::MASK,
        &7_u32.to_le_bytes(),
    );
    bytes[table + creference_registry::OVERFLOW_DISTANCE] = 5;
    bytes[buckets + 13 * reference_bucket::STRIDE + reference_bucket::DISTANCE] = 255;
    for &(handle, object) in entries {
        let cell = bucket_at(handle, buckets);
        bytes[cell + reference_bucket::DISTANCE] = 1;
        put64(bytes, cell + reference_bucket::TYPE, handle);
        put64(bytes, cell + reference_bucket::OBJECT, object as u64);
    }
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x12000];
    array(&mut bytes, STATE + cgame_state::COUNTRIES, COUNTRIES, 2);
    array(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, LINKS, 2);
    put64(&mut bytes, COUNTRIES + 8, COUNTRY as u64);
    put(&mut bytes, LINKS + 4, &1_u32.to_le_bytes());
    put64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
    put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
    array(&mut bytes, COUNTRY + ccountry::RAILWAY_UNITS, ROSTER, 1);
    put64(&mut bytes, ROSTER, GUN as u64);

    array(&mut bytes, STATE + cgame_state::PROVINCES, PROVINCES, 3);
    for (id, pointer) in [(1_u32, FROM), (2, TO)] {
        put64(&mut bytes, PROVINCES + id as usize * 8, pointer as u64);
        put64(&mut bytes, pointer, cprovince::VTABLE_RVA.0 as u64);
        put(&mut bytes, pointer + cprovince::ID, &id.to_le_bytes());
    }
    put64(
        &mut bytes,
        STATE + cgame_state::RAIL_MANAGER,
        RAIL_MANAGER as u64,
    );
    put64(
        &mut bytes,
        RAIL_MANAGER,
        crailway_manager::VTABLE_RVA.0 as u64,
    );
    array(
        &mut bytes,
        RAIL_MANAGER + crailway_manager::PROVINCE_INFO_TABLE,
        RAIL_SLOTS,
        3,
    );
    put64(&mut bytes, RAIL_SLOTS + 8, RAIL_INFO as u64);
    put64(&mut bytes, FROM + cprovince::DEFINITION, DEFINITION as u64);
    put(
        &mut bytes,
        DEFINITION + province_definition::ID,
        &1_u32.to_le_bytes(),
    );
    put64(&mut bytes, RAIL_INFO, rail_info::VTABLE_RVA.0 as u64);
    put64(
        &mut bytes,
        RAIL_INFO + rail_info::PROVINCE_RECORD,
        DEFINITION as u64,
    );
    put64(
        &mut bytes,
        RAIL_INFO + rail_info::BUILDING_STATUS,
        (FROM + cprovince::BUILDING_STATUS) as u64,
    );
    put64(
        &mut bytes,
        RAIL_INFO + rail_info::BUILDING,
        RAIL_BUILDING as u64,
    );
    put64(&mut bytes, RAIL_BUILDING, cbuilding::VTABLE_RVA.0 as u64);
    put64(
        &mut bytes,
        RAIL_BUILDING + cbuilding::BUILDING_STATUS,
        (FROM + cprovince::BUILDING_STATUS) as u64,
    );
    put(
        &mut bytes,
        RAIL_BUILDING + cbuilding::LEVEL,
        &(-3_i16).to_le_bytes(),
    );

    put64(&mut bytes, GUN, crailway_gun::VTABLE_RVA.0 as u64);
    put64(
        &mut bytes,
        GUN + 0x10,
        crailway_gun::SECONDARY_VTABLE_RVA.0 as u64,
    );
    put(&mut bytes, GUN + cunit::CLASS, &13_u32.to_le_bytes());
    put64(&mut bytes, GUN + cunit::HANDLE, GUN_HANDLE);
    put(&mut bytes, GUN + cunit::ROSTER_OWNER, &1_u32.to_le_bytes());
    put(
        &mut bytes,
        GUN + cunit::EXPEDITIONARY_OWNER,
        &2_u32.to_le_bytes(),
    );
    put(&mut bytes, GUN + cunit::LOGICAL_OWNER, &1_u32.to_le_bytes());
    put64(
        &mut bytes,
        GUN + cunit::NAVAL_TRANSFER_MEMBERSHIP,
        TRANSFER as u64,
    );
    bytes[GUN + cunit::EXILED] = 1;
    bytes[GUN + cunit::RETREAT] = 1;
    bytes[GUN + cunit::STRATEGIC_REDEPLOYMENT] = 1;
    put(
        &mut bytes,
        GUN + crailway_gun::REFERENCE_ENTRY_COUNT,
        &1_i32.to_le_bytes(),
    );
    put64(
        &mut bytes,
        GUN + crailway_gun::ASSIGNEE,
        (GROUP + crailway_gun::ASSIGNEE_BASE) as u64,
    );
    put64(&mut bytes, GUN + cunit::STAT_BLOCK, SUBUNIT as u64);
    put(
        &mut bytes,
        GUN + crailway_gun::CURRENT_STRENGTH,
        &500_000_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        GUN + cunit::RECEIVED_SUPPLY_RATIO,
        &80_000_i64.to_le_bytes(),
    );
    put64(&mut bytes, SUBUNIT, csub_unit_definition::VTABLE_RVA.0 as u64);
    put(
        &mut bytes,
        SUBUNIT + csub_unit_definition::TOKEN,
        &17_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        SUBUNIT + csub_unit_definition::MAXIMUM_STRENGTH,
        &1_000_000_i64.to_le_bytes(),
    );
    put64(
        &mut bytes,
        TRANSFER,
        cnaval_unit_transfer::VTABLE_RVA.0 as u64,
    );
    put64(
        &mut bytes,
        TRANSFER + cnaval_unit_transfer::HANDLE,
        TRANSFER_HANDLE,
    );
    put(
        &mut bytes,
        TRANSFER + cnaval_unit_transfer::ACTION_STATUS,
        &1_i32.to_le_bytes(),
    );
    put64(&mut bytes, GUN + cunit::LOCATION, FROM as u64);
    put64(&mut bytes, GUN + cunit::MOVEMENT_TARGET, TO as u64);
    array(&mut bytes, GUN + cunit::ORDINARY_ROUTE_DATA, ORDINARY, 2);
    put(&mut bytes, ORDINARY, &2_u32.to_le_bytes());
    put(&mut bytes, ORDINARY + 4, &1_u32.to_le_bytes());
    array(&mut bytes, GUN + cunit::ALTERNATE_ROUTE_DATA, ALTERNATE, 1);
    put(&mut bytes, ALTERNATE, &1_u32.to_le_bytes());
    array(&mut bytes, GUN + cunit::COMBAT_REFERENCE_DATA, COMBATS, 2);
    put64(&mut bytes, COMBATS, COMBAT as u64);
    put64(&mut bytes, COMBATS + 8, u64::MAX);
    put64(&mut bytes, COMBAT, cland_combat::VTABLE_RVA.0 as u64);
    put64(&mut bytes, COMBAT + cland_combat::ATTACKER, SIDE as u64);
    put64(&mut bytes, COMBAT + cland_combat::PROVINCE, TO as u64);
    put64(&mut bytes, SIDE, cland_combatant::VTABLE_RVA.0 as u64);
    put64(&mut bytes, SIDE + cland_combatant::COMBAT, COMBAT as u64);
    put64(
        &mut bytes,
        SIDE + cland_combatant::COUNTRIES,
        SIDE_COUNTRY as u64,
    );
    put(&mut bytes, SIDE_COUNTRY, &1_u32.to_le_bytes());

    registry(
        &mut bytes,
        TABLE,
        BUCKETS,
        &[(GUN_HANDLE, GUN + 0x10), (TRANSFER_HANDLE, TRANSFER)],
    );
    put64(&mut bytes, GROUP, corders_group::VTABLE_RVA.0 as u64);
    put64(&mut bytes, GROUP + corders_group::HANDLE, GROUP_HANDLE);
    put64(
        &mut bytes,
        GROUP + crailway_gun::ASSIGNEE_BASE,
        crailway_gun::ORDERS_GROUP_ASSIGNEE_VTABLE_RVA.0 as u64,
    );
    registry(
        &mut bytes,
        GROUP_TABLE,
        GROUP_BUCKETS,
        &[(GROUP_HANDLE, GROUP)],
    );
    stored_name(&mut bytes, GUN, b"", 0);
    bytes
}

fn standing() -> Vec<u8> {
    let mut bytes = fixture();
    put64(&mut bytes, GUN + cunit::MOVEMENT_TARGET, 0);
    array(&mut bytes, GUN + cunit::ORDINARY_ROUTE_DATA, 0, 0);
    array(&mut bytes, GUN + cunit::ALTERNATE_ROUTE_DATA, 0, 0);
    array(&mut bytes, GUN + cunit::COMBAT_REFERENCE_DATA, 0, 0);
    put64(&mut bytes, GUN + cunit::NAVAL_TRANSFER_MEMBERSHIP, 0);
    bytes[GUN + cunit::EXILED] = 0;
    bytes[GUN + cunit::RETREAT] = 0;
    bytes[GUN + cunit::STRATEGIC_REDEPLOYMENT] = 0;
    put(
        &mut bytes,
        GUN + crailway_gun::REFERENCE_ENTRY_COUNT,
        &0_i32.to_le_bytes(),
    );
    put64(&mut bytes, GUN + crailway_gun::ASSIGNEE, 0);
    bytes
}

struct MemoryFixture {
    bytes: Vec<u8>,
    fault: Option<usize>,
}

impl Memory for MemoryFixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if self.fault == Some(address) {
            return false;
        }
        if address == creference_registry::HIGH_ROOT_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&(TABLE as u64).to_le_bytes());
            return true;
        }
        if address
            == creference_registry::LOW_ROOTS_RVA.0 + corders_group::REFERENCE_TYPE as usize * 8
            && output.len() == 8
        {
            output.copy_from_slice(&(GROUP_TABLE as u64).to_le_bytes());
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}

fn capture(bytes: Vec<u8>, fault: Option<usize>, rows: &mut Vec<Row>, paths: &mut Vec<u32>) -> Walk {
    walk_impl(&MemoryFixture { bytes, fault }, 0, STATE, engine_native_memory::walk::Buffer::Growing(rows), engine_native_memory::walk::Buffer::Growing(paths), &mut cunit::FirstCombatMemo::new(4), None, None, None)
}

fn outcome(bytes: Vec<u8>, fault: Option<usize>) -> Outcome {
    capture(bytes, fault, &mut vec![Row::default(); 1], &mut vec![0; 3]).outcome
}

fn stored_name(bytes: &mut [u8], gun: usize, text: &[u8], heap: usize) {
    let descriptor = gun + NAME;
    let capacity = if text.len() <= 15 { 15 } else { text.len() };
    let source = if capacity == 15 {
        descriptor
    } else {
        put64(bytes, descriptor, heap as u64);
        heap
    };
    put(bytes, source, text);
    bytes[source + text.len()] = 0;
    put64(bytes, descriptor + 16, text.len() as u64);
    put64(bytes, descriptor + 24, capacity as u64);
}

struct EtaFixture {
    memory: MemoryFixture,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
}

impl Memory for EtaFixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.memory.fault == Some(address) {
            return false;
        }
        if address == creference_registry::LOW_ROOTS_RVA.0 + 70 * 8 && output.len() == 8 {
            output.copy_from_slice(&0x11000_u64.to_le_bytes());
            return true;
        }
        if address == 0x033d_fbe8 && output.len() == 4 {
            output.copy_from_slice(&5_i32.to_le_bytes());
            return true;
        }
        for (define, value) in [
            (0x033e_0ae0, 400_000_i64),
            (0x033e_10c8, 200_000),
            (0x033e_36d0, -50_000),
        ] {
            if address == define && output.len() == 8 {
                output.copy_from_slice(&value.to_le_bytes());
                return true;
            }
        }
        self.memory.copy(address, output)
    }
}

fn eta_fixture() -> EtaFixture {
    const NEXT_DEFINITION: usize = DEFINITION + 0x100;
    const PAIRS: usize = 0xfa00;
    let mut bytes = standing();
    array(&mut bytes, GUN + cunit::ORDINARY_ROUTE_DATA, ORDINARY, 1);
    put(&mut bytes, ORDINARY, &2_u32.to_le_bytes());
    put64(
        &mut bytes,
        TO + cprovince::DEFINITION,
        NEXT_DEFINITION as u64,
    );
    put(
        &mut bytes,
        NEXT_DEFINITION + province_definition::ID,
        &2_u32.to_le_bytes(),
    );
    bytes[DEFINITION + province_definition::TERRAIN_FLAGS] = 1;
    bytes[NEXT_DEFINITION + province_definition::TERRAIN_FLAGS] = 1;
    array(&mut bytes, RAIL_INFO + rail_info::NEIGHBOR_PAIRS, PAIRS, 1);
    put(&mut bytes, PAIRS, &2_u32.to_le_bytes());
    put(&mut bytes, PAIRS + 4, &5_i32.to_le_bytes());
    put(
        &mut bytes,
        RAIL_BUILDING + cbuilding::LEVEL,
        &5_i16.to_le_bytes(),
    );
    put(
        &mut bytes,
        RAIL_BUILDING + cbuilding::HEALTHY_LEVEL,
        &5_i16.to_le_bytes(),
    );
    put64(&mut bytes, GUN + carmy::MOVEMENT_NUMERATOR, 12_345);
    put64(&mut bytes, GUN + 0x400, 100_000);
    put64(&mut bytes, GUN + 0x410, 50_000);
    stored_name(&mut bytes, GUN, &[0xff, 0, b'G'], 0);
    EtaFixture {
        memory: MemoryFixture { bytes, fault: None },
        reads: Default::default(),
    }
}

fn fresh_capture(image: &EtaFixture, hour: Option<i32>) -> Row {
    let mut rows = vec![Row::default(); 1];
    let mut paths = vec![0; 3];
    let mut roots = vec![(0, 0); 1];
    let mut observations = Vec::new();
    let mut name_bytes = vec![0; 3];
    let mut names = NameArena {
        bytes: engine_native_memory::walk::Buffer::Growing(&mut name_bytes),
        used: 0,
        required: 0,
        failures: 0,
    };
    let mut layout = None;
    let mut scratch = TooltipScratch {
        guns: engine_native_memory::walk::Buffer::Growing(&mut roots),
        fresh_observations: engine_native_memory::walk::Buffer::Growing(&mut observations),
        sample_hour: hour,
        named: crate::railway_gun_speed::NamedBonuses {
            pointers: engine_native_memory::walk::Buffer::Count,
            required: 0,
        },
        layout: &mut layout,
        layout_buffer: &mut [],
        supply_origins: engine_native_memory::walk::Buffer::Count,
        supply_origins_len: 0,
        required_supply_origins: 0,
    };
    let walk = walk_impl(image, 0, STATE, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut paths), &mut cunit::FirstCombatMemo::new(1), Some(&mut names), Some(&mut scratch), None);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(&*scratch.guns, &[(0, 0)]);
    assert!(
        scratch
            .fresh_observations
            .iter()
            .all(|row| *row == FreshEtaObservations::default())
    );
    rows[0]
}

#[test]
fn fresh_common_operands_reuse_success_and_failure_without_poisoning_other_fields() {
    for fault in [
        None,
        Some(GUN + carmy::MOVEMENT_NUMERATOR),
        Some(GUN + 0x338),
        Some(GUN + 0x400),
        Some(GUN + carmy::ORDERS_GROUP),
    ] {
        let mut image = eta_fixture();
        image.memory.fault = fault;
        let row = fresh_capture(&image, Some(-17));
        let fresh = row.fresh.expect("independent current-hour packet");
        assert_eq!(fresh.sample_hour, -17);
        assert_eq!(
            fresh.maximum_speed,
            if fault == Some(GUN + 0x338) {
                None
            } else {
                Some(0)
            }
        );
        assert_eq!(
            fresh.land_supply_factor,
            if fault == Some(GUN + 0x400) {
                None
            } else {
                Some(75_000)
            }
        );
        assert_eq!(
            fresh.orders_group,
            if fault == Some(GUN + carmy::ORDERS_GROUP) {
                None
            } else {
                Some(0)
            }
        );
        assert_eq!(
            row.eta.is_some(),
            fault.is_none() || fault == Some(GUN + carmy::ORDERS_GROUP)
        );
        for address in [GUN + 0x338, GUN + 0x400, GUN + carmy::ORDERS_GROUP] {
            assert_eq!(
                image
                    .reads
                    .borrow()
                    .iter()
                    .filter(|(at, _)| *at == address)
                    .count(),
                1
            );
        }
        assert_eq!(
            row.fresh_failures.map(|failure| failure.is_some()),
            [
                fault == Some(GUN + 0x338),
                fault == Some(GUN + 0x400),
                fault == Some(GUN + carmy::ORDERS_GROUP)
            ]
        );
        assert_eq!(row.name, Some((0, 3)));
        assert!(row.supply.is_some());
        assert_eq!(row.withdrawal, Some(0));
    }
}

#[test]
fn idle_ordinary_route_keeps_fresh_railway_domains_without_first_edge_reads() {
    for fault in [Some(ORDINARY), Some(GUN + 0x338)] {
        let mut image = eta_fixture();
        let header_at = GUN + cunit::ORDINARY_ROUTE_DATA;
        put(
            &mut image.memory.bytes,
            header_at + engine_native_memory::cpdx_array::COUNT,
            &0_u32.to_le_bytes(),
        );
        let header = image.memory.bytes[header_at..header_at + 16]
            .try_into()
            .unwrap();
        let route = engine_native_memory::cpdx_array::Array::from_header(header, 4)
            .expect("valid idle ordinary-route header");
        assert_eq!(route.count, 0);
        assert_eq!(route.data, ORDINARY);
        assert_eq!(route.element(0, 4), None);
        assert_eq!(
            u32::from_le_bytes(
                image.memory.bytes[header_at + 8..header_at + 12]
                    .try_into()
                    .unwrap()
            ),
            1
        );
        put64(
            &mut image.memory.bytes,
            GUN + carmy::ORDERS_GROUP,
            GROUP as u64,
        );
        image.memory.fault = fault;
        let row = fresh_capture(&image, Some(123));
        assert_eq!((row.ordinary_len, row.alternate_len), (0, 0));
        assert_eq!(row.eta, None);
        let profile = row.fresh.expect("idle railway profile remains acquired");
        assert_eq!(profile.sample_hour, 123);
        assert_eq!(
            profile.maximum_speed,
            if fault == Some(GUN + 0x338) {
                None
            } else {
                Some(0)
            }
        );
        assert_eq!(profile.land_supply_factor, Some(75_000));
        assert_eq!(profile.orders_group, Some(GROUP_HANDLE));
        assert_eq!(
            row.fresh_failures.map(|failure| failure.is_some()),
            [fault == Some(GUN + 0x338), false, false]
        );
        assert_eq!(row.fresh_failure, None);
        let reads = image.reads.borrow();
        assert!(
            !reads.iter().any(|&(address, len)| {
                address < ORDINARY + 4 && address.saturating_add(len) > ORDINARY
            }),
            "idle route payload must remain unread, even when it refuses"
        );
        for address in [
            GUN + carmy::MOVEMENT_NUMERATOR,
            TO + cprovince::DEFINITION,
            RAIL_INFO + rail_info::NEIGHBOR_PAIRS,
        ] {
            assert!(
                !reads.iter().any(|&(at, _)| at == address),
                "idle first-edge operand at{address:x} must remain unread"
            );
        }
        assert_eq!(
            reads.iter().filter(|&&(at, _)| at == GUN + 0x338).count(),
            1
        );
        drop(reads);
        let mut prefix = vec![Row::default(); 1];
        let baseline = capture(image.memory.bytes.clone(), fault, &mut prefix, &mut Vec::new());
        assert_eq!(
            baseline,
            Walk {
                outcome: Outcome::Rows { len: 1 },
                paths_len: 0,
                required_paths: 0,
            }
        );
        assert_eq!(
            prefix[0],
            Row {
                name: None,
                supply: None,
                supply_origins_start: 0,
                supply_failure: None,
                withdrawal: None,
                fresh: None,
                fresh_failure: None,
                fresh_failures: [None; 3],
                ..row
            }
        );
        assert_eq!(row.name, Some((0, 3)));
        assert!(row.supply.is_some());
        assert_eq!(row.withdrawal, Some(0));
    }
}

#[test]
fn fresh_genuine_orders_group_is_independent_of_the_railway_assignee() {
    let mut image = eta_fixture();
    put64(
        &mut image.memory.bytes,
        GUN + carmy::ORDERS_GROUP,
        GROUP as u64,
    );
    let row = fresh_capture(&image, Some(123));
    assert_eq!(row.assignee_group, 0);
    assert_eq!(row.fresh.unwrap().orders_group, Some(GROUP_HANDLE));

    put64(&mut image.memory.bytes, GROUP, 0);
    image.reads.borrow_mut().clear();
    let row = fresh_capture(&image, Some(124));
    assert_eq!(row.fresh.unwrap().orders_group, None);
    assert!(row.fresh_failures[2].is_some());
    assert!(row.eta.is_some());
}

#[test]
fn stored_first_sea_edge_does_not_withhold_fresh_land_operands() {
    let mut image = eta_fixture();
    image.memory.bytes[DEFINITION + province_definition::TERRAIN_FLAGS] = 0;
    let row = fresh_capture(&image, Some(15));
    assert!(row.eta.is_none());
    let fresh = row.fresh.unwrap();
    assert_eq!(fresh.maximum_speed, Some(0));
    assert_eq!(fresh.land_supply_factor, Some(75_000));
    assert_eq!(row.fresh_failures, [None; 3]);
}

#[test]
fn fresh_supply_preserves_native_lazy_branches_and_transfer_skips() {
    for (exiled, capacity, remaining, expected, capacity_reads, remaining_reads, penalty_reads) in [
        (true, 100_000, 50_000, 100_000, 0, 0, 0),
        (false, 0, 50_000, 50_000, 1, 0, 1),
        (false, 100_000, 100_000, 100_000, 1, 1, 0),
    ] {
        let mut image = eta_fixture();
        image.memory.bytes[GUN + cunit::EXILED] = u8::from(exiled);
        put64(&mut image.memory.bytes, GUN + 0x400, capacity);
        put64(&mut image.memory.bytes, GUN + 0x410, remaining);
        let row = fresh_capture(&image, Some(88));
        assert_eq!(row.fresh.unwrap().land_supply_factor, Some(expected));
        for (address, count) in [
            (GUN + 0x400, capacity_reads),
            (GUN + 0x410, remaining_reads),
            (0x033e_36d0, penalty_reads),
        ] {
            assert_eq!(
                image
                    .reads
                    .borrow()
                    .iter()
                    .filter(|(at, _)| *at == address)
                    .count(),
                count
            );
        }
    }

    let mut image = eta_fixture();
    put64(
        &mut image.memory.bytes,
        GUN + cunit::NAVAL_TRANSFER_MEMBERSHIP,
        TRANSFER as u64,
    );
    image.memory.fault = Some(GUN + carmy::MOVEMENT_NUMERATOR);
    let row = fresh_capture(&image, Some(89));
    let fresh = row.fresh.unwrap();
    assert_eq!(
        (fresh.maximum_speed, fresh.land_supply_factor),
        (None, None)
    );
    assert_eq!(fresh.orders_group, Some(0));
    assert_eq!(row.fresh_failures, [None; 3]);
    assert!(!image.reads.borrow().iter().any(|(address, _)| {
        [GUN + 0x338, GUN + 0x400, GUN + 0x410, 0x033e_36d0].contains(address)
    }));
}

#[test]
fn missing_callback_hour_preserves_legacy_eta() {
    let image = eta_fixture();
    let row = fresh_capture(&image, None);
    assert_eq!(row.fresh, None);
    assert!(row.fresh_failure.is_some());
    assert!(row.eta.is_some());
    assert!(row.supply.is_some());
}

#[test]
fn tooltip_suffixes_preserve_prefixes_and_independent_fragments() {
    use engine_native_parts::cunit_supply_inputs::{self as supply, Failure, Origin};
    const SECOND_GUN: usize = 0x10000;
    const FIRST_ORIGINS: usize = 0x10500;
    const SECOND_ORIGINS: usize = 0x10600;
    let mut image = eta_fixture();
    let second = image.memory.bytes[GUN..GUN + 0x450].to_vec();
    put(&mut image.memory.bytes, SECOND_GUN, &second);
    let second_handle = (7 << 32) | 4713;
    put64(
        &mut image.memory.bytes,
        SECOND_GUN + cunit::HANDLE,
        second_handle,
    );
    registry(
        &mut image.memory.bytes,
        TABLE,
        BUCKETS,
        &[(GUN_HANDLE, GUN + 0x10), (second_handle, SECOND_GUN + 0x10)],
    );
    array(
        &mut image.memory.bytes,
        COUNTRY + ccountry::RAILWAY_UNITS,
        ROSTER,
        2,
    );
    put64(&mut image.memory.bytes, ROSTER + 8, SECOND_GUN as u64);
    for (gun, origins, count) in [(GUN, FIRST_ORIGINS, 2), (SECOND_GUN, SECOND_ORIGINS, 1)] {
        put64(&mut image.memory.bytes, gun + supply::ASKED, 700_000);
        put64(&mut image.memory.bytes, gun + supply::RECEIVED, 300_000);
        array(
            &mut image.memory.bytes,
            gun + supply::ORIGINS,
            origins,
            count,
        );
    }
    for (address, province, requested, received) in [
        (FIRST_ORIGINS, 1_i32, 100_000_i64, 50_000_i64),
        (FIRST_ORIGINS + supply::ORIGIN_STRIDE, 1, -100, 0),
        (SECOND_ORIGINS, 2, 200_000, 75_000),
    ] {
        put(
            &mut image.memory.bytes,
            address + supply::ORIGIN_PROVINCE,
            &province.to_le_bytes(),
        );
        put64(
            &mut image.memory.bytes,
            address + supply::ORIGIN_REQUESTED,
            requested as u64,
        );
        put64(
            &mut image.memory.bytes,
            address + supply::ORIGIN_RECEIVED,
            received as u64,
        );
    }
    assert_eq!(name_demand(&image, 0, STATE).unwrap().supply_origins, 3);
    put64(&mut image.memory.bytes, GUN + 0x338, (8 << 32) | 70);
    image.memory.fault = Some(creference_registry::LOW_ROOTS_RVA.0 + 70 * 8);
    assert_eq!(
        name_demand(&image, 0, STATE).unwrap(),
        NameDemand {
            rows: 2,
            bytes: 6,
            paths: 2,
            named_bonuses: 0,
            supply_origins: 3,
            combat_references: 0,
        }
    );
    put64(&mut image.memory.bytes, GUN + 0x338, 0);
    image.memory.bytes[GUN + cunit::WITHDRAWAL] = 0xfe;
    image.memory.bytes[SECOND_GUN + cunit::WITHDRAWAL] = 2;
    let mut output = vec![Row::default(); 2];
    let mut paths = vec![0; 2];
    let mut name_storage = vec![0; 6];
    let mut guns = vec![(0, 0); 2];
    let mut named = vec![];
    let mut layout = None;
    let mut origins = Vec::new();
    for fault in [
        None,
        Some(GUN + carmy::MOVEMENT_NUMERATOR),
        Some(GUN + 0x410),
        Some(GUN + NAME + 16),
        Some(GUN + supply::ASKED),
        Some(GUN + cunit::WITHDRAWAL),
        Some(GUN + supply::ORIGINS),
        Some(FIRST_ORIGINS + supply::ORIGIN_REQUESTED),
    ] {
        image.memory.fault = fault;
        image.reads.borrow_mut().clear();
        let mut names = NameArena {
            bytes: engine_native_memory::walk::Buffer::Growing(&mut name_storage),
            used: 0,
            required: 0,
            failures: 0,
        };
        let mut scratch = TooltipScratch {
            guns: engine_native_memory::walk::Buffer::Growing(&mut guns),
            fresh_observations: engine_native_memory::walk::Buffer::Count,
            sample_hour: None,
            named: crate::railway_gun_speed::NamedBonuses {
                pointers: engine_native_memory::walk::Buffer::Growing(&mut named),
                required: 0,
            },
            layout: &mut layout,
            layout_buffer: &mut [],
            supply_origins: engine_native_memory::walk::Buffer::Growing(&mut origins),
            supply_origins_len: 0,
            required_supply_origins: 0,
        };
        let walked = walk_impl(&image, 0, STATE, engine_native_memory::walk::Buffer::Growing(&mut output), engine_native_memory::walk::Buffer::Growing(&mut paths), &mut cunit::FirstCombatMemo::new(2), Some(&mut names), Some(&mut scratch), None);
        assert_eq!(walked.outcome, Outcome::Rows { len: 2 });
        assert_eq!((output[0].gun, output[1].gun), (GUN_HANDLE, second_handle));
        assert_eq!(
            output[0].withdrawal,
            (fault != Some(GUN + cunit::WITHDRAWAL)).then_some(0xfe)
        );
        assert_eq!(output[1].withdrawal, Some(2));
        assert_eq!(
            output[1].eta,
            Some(EtaOperands {
                numerator: 12_345,
                current_speed: 600_000
            })
        );
        assert_eq!(
            output[0].eta,
            if matches!(fault, Some(address) if address == GUN + carmy::MOVEMENT_NUMERATOR || address == GUN + 0x410)
            {
                None
            } else {
                output[1].eta
            }
        );
        assert!(output[1].name.is_some());
        assert_eq!(output[0].name.is_none(), fault == Some(GUN + NAME + 16));
        assert_eq!(
            output[0].supply.is_none(),
            fault == Some(GUN + supply::ASKED)
        );
        if let Some(first) = output[0].supply {
            assert_eq!((first.asked, first.received), (700_000, 300_000));
            assert_eq!(
                first.origins_len,
                if fault == Some(GUN + supply::ORIGINS)
                    || fault == Some(FIRST_ORIGINS + supply::ORIGIN_REQUESTED)
                {
                    Err(Failure::Unreadable)
                } else {
                    Ok(2)
                }
            );
            assert_eq!(output[0].supply_failure, first.origins_len.err());
        }
        let second = output[1].supply.unwrap();
        assert_eq!(
            (second.asked, second.received, second.origins_len),
            (700_000, 300_000, Ok(1))
        );
        let start = output[1].supply_origins_start;
        assert_eq!(
            scratch.supply_origins[start],
            Origin {
                province: 2,
                requested: 200_000,
                received: 75_000
            }
        );
        let reads = image.reads.borrow();
        let last_name = reads
            .iter()
            .position(|row| *row == (SECOND_GUN + NAME + 16, 8))
            .unwrap();
        let eta_start = reads
            .iter()
            .position(|row| *row == (GUN + carmy::MOVEMENT_NUMERATOR, 8))
            .unwrap();
        assert!(last_name < eta_start);
        let last_eta = reads
            .iter()
            .position(|row| *row == (SECOND_GUN + 0x410, 8))
            .unwrap();
        let supply_start = reads
            .iter()
            .position(|row| *row == (GUN + supply::ASKED, 8))
            .unwrap();
        assert!(last_eta < supply_start);
        assert_eq!(&*scratch.guns, &[(0, 0); 2]);
    }
}

#[test]
fn initial_eta_named_bonus_sizing_uses_live_headers_without_reading_bonus_payloads() {
    const VARIANT: usize = 0x10000;
    const NAMED_HANDLES: usize = 0x11800;
    let mut image = eta_fixture();
    let handle = (8_u64 << 32) | engine_native::cequipment_variant::REFERENCE_TYPE as u64;
    put64(&mut image.memory.bytes, GUN + 0x338, handle);
    put64(
        &mut image.memory.bytes,
        VARIANT,
        engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64,
    );
    put64(
        &mut image.memory.bytes,
        VARIANT + engine_native::cequipment_variant::TYPE,
        handle,
    );
    array(&mut image.memory.bytes, VARIANT + 0xe8, NAMED_HANDLES, 129);
    array(
        &mut image.memory.bytes,
        GUN + engine_native_parts::cunit_supply_inputs::ORIGINS,
        0x10800,
        17,
    );
    registry(
        &mut image.memory.bytes,
        0x11000,
        0x11100,
        &[(handle, VARIANT)],
    );
    assert_eq!(
        name_demand(&image, 0, STATE).unwrap(),
        NameDemand {
            rows: 1,
            bytes: 3,
            paths: 1,
            named_bonuses: 129,
            supply_origins: 17,
            combat_references: 0,
        }
    );
    assert!(
        !image
            .reads
            .borrow()
            .iter()
            .any(|(address, _)| *address == NAMED_HANDLES)
    );
    assert!(
        !image
            .reads
            .borrow()
            .iter()
            .any(|(address, _)| *address == 0x10800 + engine_native_parts::cunit_supply_inputs::ORIGIN_REQUESTED)
    );
    image.memory.fault = Some(VARIANT + 0xe8);
    assert_eq!(
        name_demand(&image, 0, STATE).unwrap(),
        NameDemand {
            rows: 1,
            bytes: 3,
            paths: 1,
            named_bonuses: 0,
            supply_origins: 17,
            combat_references: 0,
        }
    );
    image.memory.fault = Some(GUN + engine_native_parts::cunit_supply_inputs::ORIGINS);
    assert_eq!(
        name_demand(&image, 0, STATE).unwrap(),
        NameDemand {
            rows: 1,
            bytes: 3,
            paths: 1,
            named_bonuses: 129,
            supply_origins: 0,
            combat_references: 0,
        }
    );
}

#[test]
fn stored_names_preserve_all_bytes_and_fail_independently_of_old_operands() {
    for text in [
        &b""[..],
        &[0xff, 0, b'G'][..],
        &b"a heap railway gun name"[..],
    ] {
        let mut bytes = fixture();
        stored_name(&mut bytes, GUN, text, 0x10000);
        let mut old = vec![Row::default(); 1];
        let mut old_paths = vec![0; 3];
        let old_walk = capture(bytes.clone(), None, &mut old, &mut old_paths);
        for (capacity, fault) in [(text.len(), None), (0, Some(GUN + NAME + 16)), (0, None)] {
            let mut storage = vec![0; capacity];
            let mut arena = NameArena {
                bytes: engine_native_memory::walk::Buffer::Growing(&mut storage),
                used: 0,
                required: 0,
                failures: 0,
            };
            let mut rows = vec![Row::default(); 1];
            let mut paths = vec![0; 3];
            let walked = walk_impl(&MemoryFixture {
                    bytes: bytes.clone(),
                    fault,
                }, 0, STATE, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut paths), &mut cunit::FirstCombatMemo::new(4), Some(&mut arena), None, None);
            assert_eq!(walked, old_walk);
            assert_eq!(paths, old_paths);
            let name = rows[0].name.take();
            assert_eq!(rows, old);
            if fault.is_some() {
                assert_eq!(name, None);
                assert_eq!(arena.failures, 1);
            } else {
                assert_eq!(name, Some((0, text.len())));
                assert_eq!(&arena.bytes[..arena.used], text);
                assert_eq!(arena.failures, 0);
            }
            assert_eq!(arena.required, if fault.is_some() { 0 } else { text.len() });
        }
        let memory = MemoryFixture { bytes, fault: None };
        assert_eq!(
            name_demand(&memory, 0, STATE).unwrap(),
            NameDemand {
                rows: 1,
                bytes: text.len(),
                paths: 3,
                named_bonuses: 0,
                supply_origins: 0,
                combat_references: 2,
            }
        );
    }
}

#[test]
fn names_count_full_demand_even_when_the_route_or_row_arena_is_short() {
    let mut bytes = fixture();
    stored_name(&mut bytes, GUN, b"full name demand", 0x10000);
    let memory = MemoryFixture { bytes, fault: None };
    let mut arena = NameArena {
        bytes: engine_native_memory::walk::Buffer::Count,
        used: 0,
        required: 0,
        failures: 0,
    };
    let walked = walk_impl(&memory, 0, STATE, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, &mut cunit::FirstCombatMemo::new(4), Some(&mut arena), None, None);
    assert!(matches!(
        walked.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    ));
    assert_eq!((arena.used, arena.required, arena.failures), (0, 16, 0));
}

#[test]
fn captures_the_roster_gun_motion_and_first_combat_without_map_entity_state() {
    let mut rows = vec![Row::default(); 1];
    let mut paths = vec![0; 3];
    assert_eq!(
        capture(fixture(), None, &mut rows, &mut paths),
        Walk {
            outcome: Outcome::Rows { len: 1 },
            paths_len: 3,
            required_paths: 3,
        }
    );
    assert_eq!(
        rows[0],
        Row {
            gun: GUN_HANDLE,
            roster_owner: 1,
            expeditionary_owner: 2,
            logical_owner: 1,
            province: 1,
            naval_transfer: true,
            transfer_action_status: true,
            exiled: true,
            retreat: true,
            strategic_redeployment: true,
            reference_entry_count: 1,
            assignee_group: GROUP_HANDLE,
            subunit_token: 17,
            current_strength: 500_000,
            maximum_strength: 1_000_000,
            received_supply_ratio: 80_000,
            railway_building_level: Some(-3),
            target: 2,
            combat_role: 1,
            combat_province: 2,
            combat_count: 2,
            combat_references: None,
            combat_references_failure: None,
            attacking: true,
            ordinary_start: 0,
            ordinary_len: 2,
            alternate_start: 2,
            alternate_len: 1,
            name: None,
            eta: None,
            supply: None,
            supply_origins_start: 0,
            supply_failure: None,
            fresh: None,
            fresh_failure: None,
            fresh_failures: [None; 3],
            withdrawal: None,
            theatre: Some(0),
            queued_actions: Some(0),
        }
    );
    assert_eq!(paths, [2, 1, 1]);
}

#[test]
fn theatre_and_queued_orders_are_field_local() {
    let mut bytes = fixture();
    bytes[GUN + cunit::THEATRE..GUN + cunit::THEATRE + 8].copy_from_slice(&0x7770_u64.to_le_bytes());
    bytes[GUN + cunit::QUEUED_ACTION_COUNT..GUN + cunit::QUEUED_ACTION_COUNT + 4]
        .copy_from_slice(&3_i32.to_le_bytes());
    let mut rows = vec![Row::default(); 1];
    let mut paths = vec![0; 3];
    assert_eq!(
        capture(bytes, None, &mut rows, &mut paths).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].theatre, rows[0].queued_actions), (None, Some(3)));
}

fn read_calls(mut bytes: Vec<u8>, guns: usize) -> u64 {
    use engine_native_memory::read_counts::{CountedMemory, ReadCounts};
    array(
        &mut bytes,
        COUNTRY + ccountry::RAILWAY_UNITS,
        ROSTER,
        guns as u32,
    );
    for index in 0..guns {
        put64(&mut bytes, ROSTER + index * 8, GUN as u64);
    }
    let mut rows = vec![Row::default(); guns];
    let mut paths = vec![0; 3 * guns];
    ReadCounts::take_current();
    let result = walk_impl(&CountedMemory::new(MemoryFixture { bytes, fault: None }), 0, STATE, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut paths), &mut cunit::FirstCombatMemo::new(4), None, None, None);
    let reads = ReadCounts::take_current();
    assert_eq!(result.outcome, Outcome::Rows { len: guns });
    reads.copy_calls + reads.scattered_calls + reads.record_calls + reads.plan_calls
}

#[test]
fn read_calls_per_hour_and_per_gun_are_pinned() {
    let cost = |bytes: fn() -> Vec<u8>| {
        let one = read_calls(bytes(), 1);
        (one, read_calls(bytes(), 2) - one)
    };
    let (standing_hour, standing_gun) = cost(standing);
    let (moving_hour, moving_gun) = cost(fixture);
    eprintln!(
        "railway gun inputs: standing {standing_hour} calls for one gun, {standing_gun} per \
         further gun; moving in combat {moving_hour}, {moving_gun} per further gun"
    );
    assert_eq!((standing_hour, standing_gun), (56, 41));
    assert_eq!((moving_hour, moving_gun), (92, 55));
}

#[test]
fn standing_gun_keeps_null_links_as_zero() {
    let mut rows = vec![Row::default(); 1];
    let walk = capture(standing(), None, &mut rows, &mut Vec::new());
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    let row = rows[0];
    assert_eq!(row.target, 0);
    assert_eq!((row.combat_role, row.combat_province), (0, 0));
    assert_eq!((row.combat_count, row.attacking), (0, false));
    assert_eq!((row.ordinary_len, row.alternate_len), (0, 0));
    assert!(!row.naval_transfer && !row.transfer_action_status);
    assert!(!row.exiled && !row.retreat && !row.strategic_redeployment);
    assert_eq!(row.reference_entry_count, 0);
    assert_eq!(row.assignee_group, 0);
    assert_eq!(row.railway_building_level, Some(-3));
}

#[test]
fn rail_level_preserves_null_links_and_signed_values() {
    for (at, value, expected) in [
        (RAIL_SLOTS + 8, 0_u64.to_le_bytes().to_vec(), None),
        (
            RAIL_INFO + rail_info::BUILDING,
            0_u64.to_le_bytes().to_vec(),
            None,
        ),
        (
            RAIL_BUILDING + cbuilding::LEVEL,
            0_i16.to_le_bytes().to_vec(),
            Some(0),
        ),
        (
            RAIL_BUILDING + cbuilding::LEVEL,
            5_i16.to_le_bytes().to_vec(),
            Some(5),
        ),
    ] {
        let mut bytes = standing();
        put(&mut bytes, at, &value);
        let mut rows = vec![Row::default(); 1];
        assert_eq!(
            capture(bytes, None, &mut rows, &mut Vec::new()).outcome,
            Outcome::Rows { len: 1 },
            "mutation at {at:#x}"
        );
        assert_eq!(rows[0].railway_building_level, expected);
    }
}

#[test]
fn rail_faults_and_bad_backlinks_refuse_the_hour() {
    assert_eq!(
        outcome(fixture(), Some(RAIL_BUILDING + cbuilding::LEVEL)),
        Outcome::Unavailable {
            reason: "railway gun rail building level unreadable",
        },
    );
    for (at, reason) in [
        (
            RAIL_BUILDING + cbuilding::BUILDING_STATUS,
            "railway gun rail building identity mismatch",
        ),
        (
            RAIL_INFO + rail_info::PROVINCE_RECORD,
            "railway gun rail-info province backlink mismatch",
        ),
        (RAIL_MANAGER, "railway gun rail manager vtable mismatch"),
    ] {
        let mut bytes = fixture();
        put64(&mut bytes, at, 0);
        assert_eq!(
            outcome(bytes, None),
            Outcome::Unavailable { reason },
            "{at:#x}"
        );
    }
}

#[test]
fn unlocated_gun_needs_no_rail_manager() {
    let mut bytes = standing();
    put64(&mut bytes, GUN + cunit::LOCATION, 0);
    put64(&mut bytes, RAIL_MANAGER, 0);
    let mut rows = vec![Row::default(); 1];
    assert_eq!(
        capture(bytes, None, &mut rows, &mut Vec::new()).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(
        (rows[0].province, rows[0].railway_building_level),
        (0, None)
    );
}

#[test]
fn null_roster_slot_refuses_the_dense_native_census() {
    let mut bytes = fixture();
    put64(&mut bytes, ROSTER, 0);
    assert_eq!(
        outcome(bytes, None),
        Outcome::Unavailable {
            reason: "railway gun roster has a null counted slot",
        },
    );
}

#[test]
fn malformed_identity_owner_and_location_fail_instead_of_relabelling() {
    for (at, value, reason) in [
        (
            GUN + cunit::CLASS,
            0_u32.to_le_bytes().to_vec(),
            "counter unit identity invalid",
        ),
        (
            GUN + 0x10,
            0_u64.to_le_bytes().to_vec(),
            "railway gun class or secondary identity mismatch",
        ),
        (
            GUN + cunit::ROSTER_OWNER,
            0_u32.to_le_bytes().to_vec(),
            "country id is zero",
        ),
        (
            PROVINCES + 8,
            0_u64.to_le_bytes().to_vec(),
            "unit province differs from its registered slot",
        ),
        (
            PROVINCES + 16,
            0_u64.to_le_bytes().to_vec(),
            "unit province differs from its registered slot",
        ),
        (
            bucket(GUN_HANDLE) + reference_bucket::OBJECT,
            0_u64.to_le_bytes().to_vec(),
            "counter unit registry identity mismatch",
        ),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, at, &value);
        assert_eq!(
            outcome(bytes, None),
            Outcome::Unavailable { reason },
            "mutation at {at:#x}",
        );
    }
}

#[test]
fn bad_routes_never_become_stationary() {
    assert_eq!(
        outcome(fixture(), Some(ORDINARY)),
        Outcome::Unavailable {
            reason: "unit ordinary route unreadable",
        },
    );
    let mut bytes = fixture();
    put(
        &mut bytes,
        GUN + cunit::ORDINARY_ROUTE_CAPACITY,
        &1_u32.to_le_bytes(),
    );
    assert_eq!(
        outcome(bytes, None),
        Outcome::Unavailable {
            reason: "unit route invalid or longer than the province roster",
        },
    );
    let mut bytes = fixture();
    array(&mut bytes, GUN + cunit::ORDINARY_ROUTE_DATA, ORDINARY, 4);
    assert_eq!(
        outcome(bytes, None),
        Outcome::Unavailable {
            reason: "unit route invalid or longer than the province roster",
        },
    );
}

#[test]
fn first_combat_side_follows_attacker_membership_and_needs_both_identities() {
    let mut bytes = fixture();
    put(&mut bytes, SIDE_COUNTRY, &0_u32.to_le_bytes());
    put(
        &mut bytes,
        GUN + cunit::COMBAT_REFERENCE_COUNT,
        &1_u32.to_le_bytes(),
    );
    let mut rows = vec![Row::default(); 1];
    assert_eq!(
        capture(bytes.clone(), None, &mut rows, &mut vec![0; 3]).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].combat_role, rows[0].combat_province), (2, 2));
    assert_eq!((rows[0].combat_count, rows[0].attacking), (1, false));
    put(
        &mut bytes,
        GUN + cunit::COMBAT_REFERENCE_COUNT,
        &2_u32.to_le_bytes(),
    );
    put64(&mut bytes, COMBATS + 8, COMBAT_TWO as u64);
    put64(&mut bytes, COMBAT_TWO, cland_combat::VTABLE_RVA.0 as u64);
    put64(
        &mut bytes,
        COMBAT_TWO + cland_combat::ATTACKER,
        SIDE_TWO as u64,
    );
    put64(&mut bytes, COMBAT_TWO + cland_combat::PROVINCE, FROM as u64);
    put64(&mut bytes, SIDE_TWO, cland_combatant::VTABLE_RVA.0 as u64);
    put64(
        &mut bytes,
        SIDE_TWO + cland_combatant::COMBAT,
        COMBAT_TWO as u64,
    );
    put64(
        &mut bytes,
        SIDE_TWO + cland_combatant::COUNTRIES,
        SIDE_TWO_COUNTRY as u64,
    );
    put(&mut bytes, SIDE_TWO_COUNTRY, &1_u32.to_le_bytes());
    assert_eq!(
        capture(bytes.clone(), None, &mut rows, &mut vec![0; 3]).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].combat_role, rows[0].combat_province), (2, 2));
    assert_eq!((rows[0].combat_count, rows[0].attacking), (2, true));
    put64(&mut bytes, COMBAT_TWO + cland_combat::PROVINCE, 0x10);
    assert_eq!(
        capture(bytes.clone(), None, &mut rows, &mut vec![0; 3]).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].combat_province, rows[0].attacking), (2, true));
    put64(&mut bytes, COMBAT_TWO, 0x10);
    assert_eq!(
        capture(bytes, None, &mut rows, &mut vec![0; 3]).outcome,
        Outcome::Unavailable {
            reason: "division first combat identity mismatch"
        }
    );
    for (at, reason) in [
        (COMBAT, "division first combat identity mismatch"),
        (SIDE, "first combat side identity mismatch"),
        (
            SIDE + cland_combatant::COMBAT,
            "first combat side identity mismatch",
        ),
        (
            COMBAT + cland_combat::PROVINCE,
            "unit province vtable mismatch",
        ),
    ] {
        let mut bytes = fixture();
        put64(&mut bytes, at, 0x10);
        assert_eq!(
            outcome(bytes, None),
            Outcome::Unavailable { reason },
            "{at:#x}"
        );
    }
}

#[test]
fn the_first_combat_answer_is_shared_through_the_capture_memo() {
    let mut memo = cunit::FirstCombatMemo::new(4);
    let memory = MemoryFixture {
        bytes: fixture(),
        fault: None,
    };
    let mut rows = vec![Row::default(); 1];
    let walk = walk_impl(&memory, 0, STATE, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 3]), &mut memo, None, None, None);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(memo.entries.len(), 1);
    let mut bytes = fixture();
    put64(&mut bytes, SIDE, 0);
    let memory = MemoryFixture { bytes, fault: None };
    assert_eq!(
        super::walk_impl(&memory, 0, STATE, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 3]), &mut memo, None, None, None).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!((rows[0].combat_role, rows[0].combat_province), (1, 2));
    let mut fresh = cunit::FirstCombatMemo::new(4);
    assert!(matches!(
        super::walk_impl(&memory, 0, STATE, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 3]), &mut fresh, None, None, None).outcome,
        Outcome::Unavailable { .. }
    ));
    assert!(fresh.entries.is_empty());
}

#[test]
fn assignee_must_be_a_registered_orders_group_subobject() {
    for (at, reason) in [
        (
            GROUP + crailway_gun::ASSIGNEE_BASE,
            "railway gun assignee secondary vtable mismatch",
        ),
        (
            GROUP + corders_group::HANDLE,
            "plan group reference type mismatch",
        ),
        (
            bucket_at(GROUP_HANDLE, GROUP_BUCKETS) + reference_bucket::OBJECT,
            "railway gun assignee registry identity mismatch",
        ),
    ] {
        let mut bytes = fixture();
        put64(&mut bytes, at, 0);
        assert_eq!(
            outcome(bytes, None),
            Outcome::Unavailable { reason },
            "{at:#x}"
        );
    }
}

#[test]
fn unreadable_operands_are_not_defaulted() {
    for (at, reason) in [
        (
            GUN + cunit::NAVAL_TRANSFER_MEMBERSHIP,
            "railway gun naval transfer unreadable",
        ),
        (
            TRANSFER + cnaval_unit_transfer::ACTION_STATUS,
            "railway gun naval transfer action unreadable",
        ),
        (GUN + cunit::EXILED, "railway gun exile byte unreadable"),
        (GUN + cunit::RETREAT, "railway gun retreat byte unreadable"),
        (
            GUN + cunit::STRATEGIC_REDEPLOYMENT,
            "railway gun strategic redeployment unreadable",
        ),
        (
            GUN + crailway_gun::REFERENCE_ENTRY_COUNT,
            "railway gun reference entry count unreadable",
        ),
        (
            GUN + crailway_gun::ASSIGNEE,
            "railway gun assignee pointer unreadable",
        ),
        (
            SUBUNIT + csub_unit_definition::MAXIMUM_STRENGTH,
            "railway gun maximum strength unreadable",
        ),
        (
            GUN + crailway_gun::CURRENT_STRENGTH,
            "railway gun current strength unreadable",
        ),
        (
            GUN + cunit::RECEIVED_SUPPLY_RATIO,
            "railway gun received supply ratio unreadable",
        ),
    ] {
        assert_eq!(
            outcome(fixture(), Some(at)),
            Outcome::Unavailable { reason },
            "fault at {at:#x}",
        );
    }
}

#[test]
fn naval_transfer_and_stat_block_need_their_identities() {
    for (at, reason) in [
        (TRANSFER, "railway gun naval transfer vtable mismatch"),
        (
            TRANSFER + cnaval_unit_transfer::HANDLE,
            "railway gun naval transfer handle invalid",
        ),
        (
            bucket(TRANSFER_HANDLE) + reference_bucket::OBJECT,
            "railway gun naval transfer registry mismatch",
        ),
        (SUBUNIT, "railway gun stat block vtable mismatch"),
    ] {
        let mut bytes = fixture();
        put64(&mut bytes, at, 0);
        assert_eq!(
            outcome(bytes, None),
            Outcome::Unavailable { reason },
            "{at:#x}"
        );
    }
}

mod unit_combat_reference_tests;
