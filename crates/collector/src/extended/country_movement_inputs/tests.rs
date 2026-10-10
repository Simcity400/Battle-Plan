use super::*;
use engine::{
    cdiplomacy_status, cgame_state, cnaval_unit_transfer, cprovince, crelation_status,
    cstrategic_region, cweather_manager, memory::SliceMemory, modifier_definition,
};
use std::{cell::RefCell, collections::BTreeMap};

const STATE: usize = 0x1000;

struct World {
    bytes: Vec<u8>,
    globals: BTreeMap<usize, Vec<u8>>,
}

impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if let Some((&start, bytes)) = self.globals.range(..=address).next_back()
            && let Some(bytes) = bytes.get(address - start..address - start + output.len())
        {
            output.copy_from_slice(bytes);
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}

impl World {
    fn put(&mut self, address: usize, value: &[u8]) {
        self.bytes[address..address + value.len()].copy_from_slice(value);
    }
    fn u64(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
    fn u32(&mut self, address: usize, value: u32) {
        self.put(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.u64(address, data as u64);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }
    fn new() -> Self {
        let mut world = Self {
            bytes: vec![0; 0x30000],
            globals: BTreeMap::from([
                (
                    cnaval_unit_transfer::BASE_SPEED_RVA,
                    200_000_i64.to_le_bytes().to_vec(),
                ),
                (
                    modifier_definition::TRADE_FEATURE_FLAGS_RVA,
                    0_u32.to_le_bytes().to_vec(),
                ),
            ]),
        };
        world.array(STATE + cgame_state::COUNTRIES, 0x2000, 4);
        world.array(STATE + cgame_state::COUNTRY_LINKS, 0x2100, 4);
        for (slot, country, raw, diplomacy, matrix, statuses) in [
            (1, 0x3000, 1, 0xa000, 0xc000, 0x18000),
            (2, 0x5000, 3, 0xb000, 0xc100, 0x18800),
        ] {
            world.u64(0x2000 + slot * 8, country as u64);
            world.u32(0x2100 + slot * 4, slot as u32);
            world.u64(country, ccountry::VTABLE_RVA as u64);
            world.u32(country + ccountry::ID, raw);
            world.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
            world.u64(diplomacy, cdiplomacy_status::VTABLE_RVA as u64);
            world.u64(diplomacy + cdiplomacy_status::COUNTRY, country as u64);
            world.array(diplomacy + cdiplomacy_status::RELATIONS, matrix, 4);
            for (index, target, target_raw) in [(0, 0, 0), (1, 2, 3)] {
                let status = statuses + index * 0x400;
                world.u64(matrix + target * 8, status as u64);
                world.u64(status, crelation_status::VTABLE_RVA as u64);
                world.u64(
                    status + crelation_status::DIPLOMACY_STATUS,
                    diplomacy as u64,
                );
                world.u32(status + crelation_status::TARGET_COUNTRY, target_raw);
                world.array(status + 0x140, 0, 0);
            }
        }
        world.u64(0x2018, 0x5000);
        world.u32(0x210c, 2);
        world.array(STATE + cgame_state::PROVINCES, 0xf000, 4);
        for (slot, controller) in [(1, 3), (2, 0), (3, 3)] {
            let province = 0x11000 + (slot - 1) * 0x200;
            world.u64(0xf000 + slot * 8, province as u64);
            world.u64(province, cprovince::VTABLE_RVA as u64);
            world.u32(province + cprovince::ID, slot as u32);
            world.u32(province + cprovince::CONTROLLER, controller);
        }
        world.array(STATE + cgame_state::STRATEGIC_REGIONS, 0xc200, 2);
        for (slot, id) in [(0, 0), (1, 37)] {
            let region = 0xd000 + slot * 0x200;
            world.u64(0xc200 + slot * 8, region as u64);
            world.u64(region, cstrategic_region::VTABLE_RVA as u64);
            world.u32(region + cstrategic_region::ID, id);
            world.u32(region + cstrategic_region::RUNTIME_SLOT, slot as u32);
        }
        world.u64(STATE + cgame_state::WEATHER_MANAGER, 0xe000);
        world.u64(0xe000, cweather_manager::VTABLE_RVA as u64);
        world.array(0xe000 + cweather_manager::REGIONS, 0, 0);
        world
    }
}

struct Observed<'a> {
    world: &'a World,
    fault: Option<usize>,
    reads: RefCell<Vec<usize>>,
}
impl Memory for Observed<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        if self
            .fault
            .is_some_and(|fault| address <= fault && fault < address + output.len())
        {
            return false;
        }
        self.world.copy(address, output)
    }
}

fn capacity() -> Capacities {
    Capacities {
        owners: 4,
        links: 4,
        controllers: 3,
        regions: 2,
        river: 2,
        transport: 2,
    }
}

fn demands() -> [OwnerDemand; 3] {
    [
        OwnerDemand {
            raw: 2,
            river: false,
            transport: true,
        },
        OwnerDemand {
            raw: 3,
            river: true,
            transport: false,
        },
        OwnerDemand {
            raw: 3,
            river: true,
            transport: true,
        },
    ]
}

fn moving_seed() -> carmy::DivisionSeed {
    let mut route = [0; 16];
    route[..8].copy_from_slice(&0x2000_u64.to_le_bytes());
    route[8..12].copy_from_slice(&2_u32.to_le_bytes());
    route[12..].copy_from_slice(&2_u32.to_le_bytes());
    carmy::DivisionSeed {
        pointer: 0x3000,
        division: 9,
        primary_vtable: Some(carmy::VTABLE_RVA as u64),
        handle: Some(9),
        class: Some(0),
        ordinary_route: Some(route),
        expeditionary_owner: Some(u32::MAX),
        roster_owner: Some(3),
        logical_owner: Some(2),
        ..Default::default()
    }
}

#[test]
fn moving_owner_classifier_uses_signed_land_owner_and_independent_logical_owner() {
    let first = moving_seed();
    let second = carmy::DivisionSeed {
        expeditionary_owner: Some(4),
        ..first
    };
    let other_class = carmy::DivisionSeed {
        class: Some(1),
        ..first
    };
    let seeds = [first, second, other_class];
    let guns = [railway_gun_counter_inputs::Row {
        gun: 8,
        logical_owner: 2,
        ordinary_len: 1,
        ..Default::default()
    }];
    let (demands, failure) = owner_demands(
        0,
        Sources {
            rows: &seeds,
            outcome: Outcome::Rows { len: 3 },
        },
        Sources {
            rows: &guns,
            outcome: Outcome::Rows { len: 1 },
        },
    );
    assert_eq!(failure, None);
    assert_eq!(
        demands
            .map(|demand| (demand.raw, demand.river, demand.transport))
            .collect::<Vec<_>>(),
        [
            (3, true, false),
            (2, false, true),
            (4, true, false),
            (2, false, true),
            (2, false, true)
        ]
    );
}

#[test]
fn idle_division_and_railway_demands_reach_the_shared_owner_capture() {
    let seeds = [carmy::DivisionSeed {
        // A complete empty header is an observed idle route, not missing data.
        ordinary_route: Some([0; 16]),
        ..moving_seed()
    }];
    let guns = [railway_gun_counter_inputs::Row {
        gun: 8,
        logical_owner: 2,
        ordinary_len: 0,
        ..Default::default()
    }];
    let (demands, failure) = owner_demands(
        0,
        Sources {
            rows: &seeds,
            outcome: Outcome::Rows { len: 1 },
        },
        Sources {
            rows: &guns,
            outcome: Outcome::Rows { len: 1 },
        },
    );
    assert_eq!(failure, None);
    let demands = demands.collect::<Vec<_>>();
    assert_eq!(
        demands
            .iter()
            .map(|demand| (demand.raw, demand.river, demand.transport))
            .collect::<Vec<_>>(),
        [(3, true, false), (2, false, true), (2, false, true)]
    );
    let world = World::new();
    let mut capture = Captured::with_capacities(capacity());
    capture.capture(&world, 0, STATE, demands, failure);
    // Raw owner 2 and native current id 3 resolve to one canonical owner. The
    // idle rail and division share the existing transport graph acquisition.
    assert_eq!((capture.land_owners, capture.logical_owners), (1, 1));
    assert_eq!(capture.owners.len, 1);
    assert_eq!((capture.controller_keys, capture.region_keys), (2, 2));
    assert_eq!(capture.source_failure, None);
    assert_eq!(capture.refusals().count(), 0);
}

#[test]
fn invalid_idle_header_is_not_a_known_empty_route_or_owner_demand() {
    let mut invalid = [0; 16];
    invalid[8..12].copy_from_slice(&1_u32.to_le_bytes());
    invalid[12..].copy_from_slice(&1_u32.to_le_bytes());
    let seeds = [carmy::DivisionSeed {
        ordinary_route: Some(invalid),
        ..moving_seed()
    }];
    let (demands, failure) = owner_demands(
        0,
        Sources {
            rows: &seeds,
            outcome: Outcome::Rows { len: 1 },
        },
        Sources {
            rows: &[],
            outcome: Outcome::Rows { len: 0 },
        },
    );
    assert_eq!(
        failure,
        Some("country movement division route unobserved or invalid")
    );
    assert_eq!(demands.count(), 0);
}

#[test]
fn missing_land_owner_preserves_logical_demand_and_source_refusal() {
    let seeds = [carmy::DivisionSeed {
        expeditionary_owner: None,
        ..moving_seed()
    }];
    let (demands, failure) = owner_demands(
        0,
        Sources {
            rows: &seeds,
            outcome: Outcome::Rows { len: 1 },
        },
        Sources {
            rows: &[],
            outcome: Outcome::Rows { len: 0 },
        },
    );
    assert_eq!(
        failure,
        Some("country movement division land owner unobserved")
    );
    assert_eq!(
        demands
            .map(|demand| (demand.raw, demand.river, demand.transport))
            .collect::<Vec<_>>(),
        [(2, false, true)]
    );
    let (demands, failure) = owner_demands(
        0,
        Sources {
            rows: &[],
            outcome: Outcome::Unavailable {
                reason: "native source refused",
            },
        },
        Sources {
            rows: &[],
            outcome: Outcome::Rows { len: 0 },
        },
    );
    assert_eq!(failure, Some("native source refused"));
    assert_eq!(demands.count(), 0);
}

#[test]
fn live_hint_growth_keeps_independent_floors_and_bounds_controller_products() {
    let mut capacity = Capacities::default();
    let mut hint = LiveDemand {
        country_slots: Ok(4),
        raw_links: Ok(7),
        controller_bound: Ok(1000),
        region_bound: Ok(11),
        movers: Err("moving roster refused"),
    };
    assert_eq!(
        capacity.raise_for_demand(hint),
        Err("moving roster refused")
    );
    assert!(
        capacity.owners >= 4
            && capacity.links >= 7
            && capacity.controllers >= 1000
            && capacity.regions >= 11
    );
    assert_eq!((capacity.river, capacity.transport), (0, 0));
    hint.movers = Ok(native::MovingOwners {
        land: 2,
        logical: 1,
    });
    capacity.raise_for_demand(hint).unwrap();
    assert_eq!(capacity.river, 8);
    assert!(capacity.transport >= 11);
    hint.controller_bound = Err("province census refused");
    hint.movers = Ok(native::MovingOwners {
        land: 3,
        logical: 3,
    });
    assert_eq!(
        capacity.raise_for_demand(hint),
        Err("province census refused")
    );
    assert_eq!(capacity.river, 8);
    assert!(capacity.transport >= 33);
}

#[test]
fn retained_scratch_capacity_does_not_admit_an_owner_outside_live_links() {
    let world = World::new();
    let memory = Observed {
        world: &world,
        fault: None,
        reads: RefCell::new(Vec::new()),
    };
    let mut capture = Captured::with_capacities(Capacities {
        links: 8,
        ..capacity()
    });
    capture.capture(
        &memory,
        0,
        STATE,
        [OwnerDemand {
            raw: 5,
            river: true,
            transport: true,
        }],
        None,
    );
    assert_eq!(capture.refused_raw_owners, 1);
    assert_eq!(
        capture.source_failure,
        Some("country movement demanded owner is zero or outside live links")
    );
    assert_eq!((capture.land_owners, capture.logical_owners), (0, 0));
    assert_eq!(capture.owners.len, 0);
    assert!(memory.reads.borrow().iter().all(|&address| {
        address != STATE + cgame_state::PROVINCES
            && address != STATE + cgame_state::STRATEGIC_REGIONS
    }));
    let mut work = Work::default();
    work.observe(&capture).unwrap();
    assert_eq!(
        serde_json::to_value(work.status()).unwrap()["refused_raw_owners"],
        1
    );
}

#[test]
fn status_retains_domain_hint_failure_when_owner_scratch_is_short() {
    let world = World::new();
    let memory = Observed {
        world: &world,
        fault: Some(STATE + cgame_state::PROVINCES),
        reads: RefCell::new(Vec::new()),
    };
    let mut capture = Captured::with_capacities(Capacities::default());
    capture.capture(&memory, 0, STATE, demands(), None);
    assert_eq!(capture.owners.len, 0);
    assert_eq!(capture.region_failure, None);
    let mut work = Work::default();
    work.observe(&capture).unwrap();
    assert_eq!(
        work.refusals,
        [
            ("country movement owner-demand arenas exhausted", 1),
            ("country movement province census unreadable or invalid", 1),
        ]
    );
    assert_eq!(work.required.regions, 2);
    assert_eq!(work.required.transport, 4);
}

#[test]
fn aliases_share_one_owner_capture_and_projection_joins_native_id() {
    let world = World::new();
    let memory = Observed {
        world: &world,
        fault: None,
        reads: RefCell::new(Vec::new()),
    };
    let mut capture = Captured::with_capacities(capacity());
    capture.capture(&memory, 0, STATE, demands(), None);
    assert_eq!((capture.land_owners, capture.logical_owners), (1, 1));
    assert_eq!((capture.controller_keys, capture.region_keys), (2, 2));
    assert_eq!(capture.owners.len, 1);
    let row = capture.for_country(3).unwrap();
    assert_eq!(
        row.river.unwrap(),
        vec![
            ControllerRiverSpeed {
                target_slot: 0,
                modifier: Some(0)
            },
            ControllerRiverSpeed {
                target_slot: 2,
                modifier: Some(0)
            },
        ]
    );
    assert_eq!(
        row.transport.unwrap(),
        vec![
            RegionTransportSpeed {
                region: 0,
                speed: Some(200_000)
            },
            RegionTransportSpeed {
                region: 37,
                speed: Some(200_000)
            },
        ]
    );
    assert_eq!(
        capture.for_country(2).unwrap(),
        CountryMovementInputs::default()
    );
    // The domain and transport acquisition each retain their own region header.
    // Both transport queries reuse that accepted header but check their own slot.
    for (root, expected_reads) in [
        (STATE + cgame_state::PROVINCES, 1),
        (STATE + cgame_state::STRATEGIC_REGIONS, 2),
    ] {
        assert_eq!(
            memory
                .reads
                .borrow()
                .iter()
                .filter(|&&address| address == root)
                .count(),
            expected_reads
        );
    }
    // Each domain and transport authentication checks every region's slot once.
    for region in [0xd000, 0xd200] {
        assert_eq!(
            memory
                .reads
                .borrow()
                .iter()
                .filter(|&&address| address == region + cstrategic_region::RUNTIME_SLOT)
                .count(),
            2
        );
    }
    assert!(capture.owners.first.iter().all(|row| row.owner.is_none()));
    assert!(
        capture
            .regions
            .first
            .iter()
            .all(|region| region.pointer == 0)
    );
}

#[test]
fn short_river_arena_preserves_transport_and_grows_from_full_known_demand() {
    let world = World::new();
    let mut capacities = capacity();
    capacities.river = 1;
    let mut capture = Captured::with_capacities(capacities);
    capture.capture(&world, 0, STATE, demands(), None);
    let row = capture.for_country(3).unwrap();
    assert_eq!(row.river, None);
    assert_eq!(row.transport.unwrap().len(), 2);
    assert_eq!(capture.river.required, 2);
    assert_eq!(
        capture.owners.first[0].river_failure,
        Some(native::RIVER_CAPACITY)
    );
    assert!(
        capture
            .regions
            .first
            .iter()
            .all(|region| region.pointer == 0)
    );
    capture.grow(&mut capacities);
    capture.capture(&world, 0, STATE, demands(), None);
    assert_eq!(capture.for_country(3).unwrap().river.unwrap().len(), 2);
    assert!(capture.owners.first.iter().all(|row| row.owner.is_none()));
}

#[test]
fn controller_domain_fault_preserves_transport_and_does_not_publish_partial_keys() {
    let world = World::new();
    let memory = Observed {
        world: &world,
        fault: Some(0x11200 + cprovince::CONTROLLER),
        reads: RefCell::new(Vec::new()),
    };
    let mut capture = Captured::with_capacities(capacity());
    capture.capture(&memory, 0, STATE, demands(), None);
    assert!(capture.controller_failure.is_some());
    assert_eq!(capture.controllers.required, 3);
    let row = capture.for_country(3).unwrap();
    assert_eq!(row.river, None);
    assert_eq!(row.transport.unwrap().len(), 2);
    assert!(
        capture
            .regions
            .first
            .iter()
            .all(|region| region.pointer == 0)
    );
}

#[test]
fn simultaneous_shortages_report_all_pressure_and_one_growth_recovers() {
    let world = World::new();
    let memory = Observed {
        world: &world,
        fault: None,
        reads: RefCell::new(Vec::new()),
    };
    let mut capacities = Capacities::default();
    let mut capture = Captured::with_capacities(capacities);
    capture.capture(&memory, 0, STATE, demands(), None);
    assert_eq!(
        (capture.owners.required, capture.raw_flags.required),
        (4, 4)
    );
    assert_eq!(
        (capture.controllers.required, capture.regions.required),
        (3, 2)
    );
    // Count-only bounds include repeated raw demands until dedup scratch fits.
    assert_eq!((capture.river.required, capture.transport.required), (6, 4));
    assert!(capture.source_failure.is_some());
    assert!(
        !memory
            .reads
            .borrow()
            .contains(&cnaval_unit_transfer::BASE_SPEED_RVA)
    );
    capture.grow(&mut capacities);
    capture.capture(&world, 0, STATE, demands(), None);
    let row = capture.for_country(3).unwrap();
    assert_eq!(row.river.unwrap().len(), 2);
    assert_eq!(row.transport.unwrap().len(), 2);
    assert_eq!((capture.land_owners, capture.logical_owners), (1, 1));
    assert_eq!(capture.source_failure, None);
    assert_eq!(capture.refusals().count(), 0);
    assert!(capture.owners.first.iter().all(|row| row.owner.is_none()));
    assert!(
        capture
            .regions
            .first
            .iter()
            .all(|region| region.pointer == 0)
    );
}

#[test]
fn short_domains_report_coefficient_pressure_before_growth() {
    let world = World::new();
    let mut capacities = capacity();
    capacities.controllers = 0;
    capacities.regions = 0;
    capacities.river = 0;
    capacities.transport = 0;
    let mut capture = Captured::with_capacities(capacities);
    capture.capture(&world, 0, STATE, demands(), None);
    assert_eq!(
        (capture.controllers.required, capture.regions.required),
        (3, 2)
    );
    assert_eq!((capture.river.required, capture.transport.required), (3, 2));
    assert_eq!(
        capture.for_country(3).unwrap(),
        CountryMovementInputs::default()
    );
    assert_eq!(capture.refusals().count(), 2);
    capture.grow(&mut capacities);
    capture.capture(&world, 0, STATE, demands(), None);
    let row = capture.for_country(3).unwrap();
    assert_eq!(row.river.unwrap().len(), 2);
    assert_eq!(row.transport.unwrap().len(), 2);
}

#[test]
fn next_empty_demand_clears_prior_rows_and_skips_map_domains() {
    let world = World::new();
    let mut capture = Captured::with_capacities(capacity());
    capture.capture(&world, 0, STATE, demands(), None);
    let memory = Observed {
        world: &world,
        fault: None,
        reads: RefCell::new(Vec::new()),
    };
    capture.capture(&memory, 0, STATE, [], None);
    assert_eq!(capture.owners.len, 0);
    assert_eq!(
        capture.for_country(3).unwrap(),
        CountryMovementInputs::default()
    );
    assert_eq!((capture.river.len, capture.transport.len), (0, 0));
    assert_eq!(capture.transport_graph_failure, None);
    assert!(!memory.reads.borrow().iter().any(|&address| {
        address == STATE + cgame_state::PROVINCES
            || address == STATE + cgame_state::STRATEGIC_REGIONS
    }));
    assert!(capture.owners.first.iter().all(|row| row.owner.is_none()));
    assert!(
        capture
            .regions
            .first
            .iter()
            .all(|region| region.pointer == 0)
    );
}

#[test]
fn transport_refusals_count_each_named_region_query() {
    let world = World::new();
    let memory = Observed {
        world: &world,
        fault: Some(cnaval_unit_transfer::BASE_SPEED_RVA),
        reads: RefCell::new(Vec::new()),
    };
    let mut capture = Captured::with_capacities(capacity());
    capture.capture(&memory, 0, STATE, demands(), None);
    let row = capture.for_country(3).unwrap();
    assert!(row.transport.unwrap().iter().all(|row| row.speed.is_none()));
    assert_eq!(
        capture.refusals().collect::<Vec<_>>(),
        vec![
            (3, "transport", "transport input unreadable"),
            (3, "transport", "transport input unreadable"),
        ]
    );
    let mut work = Work::default();
    work.observe(&capture).unwrap();
    assert_eq!(work.refusals, vec![("transport input unreadable", 2)]);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&at| at == cnaval_unit_transfer::BASE_SPEED_RVA)
            .count(),
        1
    );
}

#[test]
fn worker_owner_and_coefficient_arenas_grow_for_five_thousand_countries_on_first_pass() {
    let mut world = World::new();
    let count = 5001;
    let mut roster = vec![0; count * 8];
    let mut links = vec![0; count * 4];
    let mut provinces = vec![0; count * 8];
    for slot in 1..count {
        let country = 0x10_0000 + slot * 0x10000;
        roster[slot * 8..slot * 8 + 8].copy_from_slice(&(country as u64).to_le_bytes());
        links[slot * 4..slot * 4 + 4].copy_from_slice(&(slot as u32).to_le_bytes());
        let mut identity = vec![0; ccountry::ID + 4];
        identity[..8].copy_from_slice(&(ccountry::VTABLE_RVA as u64).to_le_bytes());
        identity[ccountry::ID..].copy_from_slice(&(slot as u32).to_le_bytes());
        world.globals.insert(country, identity);
        let province = 0x8000_0000 + slot * 0x1000;
        provinces[slot * 8..slot * 8 + 8].copy_from_slice(&(province as u64).to_le_bytes());
        let mut row = vec![0; cprovince::CONTROLLER + 4];
        row[..8].copy_from_slice(&(cprovince::VTABLE_RVA as u64).to_le_bytes());
        row[cprovince::ID..cprovince::ID + 4].copy_from_slice(&(slot as u32).to_le_bytes());
        row[cprovince::CONTROLLER..].copy_from_slice(&(slot as u32).to_le_bytes());
        world.globals.insert(province, row);
    }
    world.array(STATE + cgame_state::COUNTRIES, 0x40000, count as u32);
    world.array(STATE + cgame_state::COUNTRY_LINKS, 0x50000, count as u32);
    world.array(STATE + cgame_state::PROVINCES, 0x60000, count as u32);
    world.globals.insert(0x40000, roster);
    world.globals.insert(0x50000, links);
    world.globals.insert(0x60000, provinces);
    let mut fixed = Captured::with_capacities(Capacities::default());
    fixed.capture(
        &world,
        0,
        STATE,
        [OwnerDemand {
            raw: 1,
            river: true,
            transport: true,
        }],
        None,
    );
    assert_eq!(
        fixed.source_failure,
        Some("country movement owner-demand arenas exhausted")
    );
    assert_eq!(fixed.owners.len, 0);
    let mut capture = Captured::with_capacities(Capacities::default());
    for regions in [2, 3] {
        world.array(STATE + cgame_state::STRATEGIC_REGIONS, 0xc200, regions);
        if regions == 3 {
            world.u64(0xc210, 0xd400);
            world.u64(0xd400, cstrategic_region::VTABLE_RVA as u64);
            world.u32(0xd400 + cstrategic_region::ID, 91);
            world.u32(0xd400 + cstrategic_region::RUNTIME_SLOT, 2);
        }
        capture.capture_growing(
            &world,
            0,
            STATE,
            (1..count).map(|slot| OwnerDemand {
                raw: slot as u32,
                river: slot == 1,
                transport: true,
            }),
            None,
        );
        assert_eq!(capture.source_failure, None);
        assert_eq!(capture.controller_failure, None);
        assert_eq!(capture.region_failure, None);
        assert_eq!(capture.owners.len, count - 1);
        assert_eq!(capture.controller_keys, count - 1);
        assert_eq!(
            capture.for_country(1).unwrap().river.unwrap().len(),
            count - 1
        );
        for slot in 1..count {
            let queries = capture.for_country(slot as u32).unwrap().transport.unwrap();
            assert_eq!(queries.len(), regions as usize);
        }
        assert!(
            capture
                .refusals()
                .all(|(_, _, reason)| { reason != "transport refusal arena capacity exhausted" })
        );
    }
}
