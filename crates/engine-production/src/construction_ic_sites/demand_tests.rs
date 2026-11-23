use super::*;
use engine_native::{ccountry::synthetic::World, cgame_state, cproduction_status};
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const BASE: usize = 0x140_000000;

struct Fixture {
    world: World,
    queue: usize,
    building: usize,
    railway: usize,
    route_data: usize,
    statuses: Vec<usize>,
}

impl Fixture {
    fn new(countries: u32, provinces: u32) -> Self {
        let mut world = World::new(BASE, countries, engine_native::ccountry::PRODUCTION_STATUS + 8);
        let building = world.alloc(8);
        world.u64(
            building,
            (BASE + cbuilding_production_line::VTABLE_RVA.0) as u64,
        );
        let railway = world.alloc(crailway_production_line::ROUTE_COUNT + 4);
        world.u64(
            railway,
            (BASE + crailway_production_line::VTABLE_RVA.0) as u64,
        );
        let route_data = world.alloc(4);
        world.array(
            railway + crailway_production_line::ROUTE,
            route_data,
            provinces,
        );
        let repair = world.alloc(8);
        world.u64(
            repair,
            (BASE + cbuilding_production_line::RAILWAY_GUN_REPAIR_VTABLE_RVA.0) as u64,
        );
        let queue = world.alloc(4 * 8);
        for (ordinal, line) in [building, building, railway, repair]
            .into_iter()
            .enumerate()
        {
            world.u64(queue + ordinal * 8, line as u64);
        }
        let mut statuses = Vec::new();
        for slot in 1..=countries {
            let country = world.object(slot);
            let status = world.alloc(cproduction_status::COUNTRY + 8);
            world.u64(country + engine_native::ccountry::PRODUCTION_STATUS, status as u64);
            world.u64(status, (BASE + cproduction_status::VTABLE_RVA.0) as u64);
            world.u64(status + cproduction_status::COUNTRY, country as u64);
            world.array(status + cproduction_status::CONSTRUCTION_LINES, queue, 4);
            statuses.push(status);
        }
        Self {
            world,
            queue,
            building,
            railway,
            route_data,
            statuses,
        }
    }

    fn demand(&self) -> Result<usize, &'static str> {
        events_per_pass(&self.world.memory(), BASE, self.world.game_state())
    }
}

struct TraceMemory<'a> {
    memory: SliceMemory<'a>,
    unreadable: Option<usize>,
    reads: RefCell<Vec<usize>>,
}
impl Memory for TraceMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        self.unreadable != Some(address) && self.memory.copy(address, output)
    }
}

#[test]
fn native_route_headers_size_empty_short_and_mod_sized_routes_without_payload_reads() {
    for provinces in [0, 1, 2, 3, 65_538] {
        let fixture = Fixture::new(1, provinces);
        let memory = TraceMemory {
            memory: fixture.world.memory(),
            unreadable: None,
            reads: RefCell::new(Vec::new()),
        };
        assert_eq!(
            events_per_pass(&memory, BASE, fixture.world.game_state()),
            Ok(6 + provinces.saturating_sub(1) as usize + 1),
        );
        assert!(!memory.reads.borrow().contains(&fixture.route_data));
    }
}

#[test]
fn unchanged_country_population_uses_the_new_native_route_count() {
    let mut fixture = Fixture::new(1, 2);
    assert_eq!(fixture.demand(), Ok(8));
    fixture.world.array(
        fixture.railway + crailway_production_line::ROUTE,
        fixture.route_data,
        100_003,
    );
    assert_eq!(fixture.demand(), Ok(100_009));
}

#[test]
fn sizing_preserves_the_first_fault_and_names_unknown_classes() {
    let mut fixture = Fixture::new(1, 3);
    fixture.world.u64(fixture.queue + 8, 0);
    let memory = TraceMemory {
        memory: fixture.world.memory(),
        unreadable: Some(fixture.building),
        reads: RefCell::new(Vec::new()),
    };
    assert_eq!(
        events_per_pass(&memory, BASE, fixture.world.game_state()),
        Err("construction output sizing line vtable unreadable"),
    );
    assert!(!memory.reads.borrow().contains(&(fixture.queue + 8)));
    fixture.world.u64(fixture.building, 0x1234);
    assert_eq!(
        fixture.demand(),
        Err("construction output sizing line vtable unsupported")
    );
}

#[test]
fn malformed_original_queue_and_owner_headers_are_not_zero_demand() {
    let mut fixture = Fixture::new(1, 3);
    fixture.world.u32(
        fixture.statuses[0] + cproduction_status::CONSTRUCTION_LINES_COUNT,
        5,
    );
    assert_eq!(
        fixture.demand(),
        Err("construction output sizing queue unreadable or invalid")
    );
    fixture
        .world
        .u64(fixture.statuses[0] + cproduction_status::COUNTRY, 0);
    assert_eq!(fixture.demand(), Err("production status country mismatch"));
    fixture
        .world
        .u32(fixture.world.game_state() + cgame_state::COUNTRIES + 12, 1);
    assert_eq!(fixture.demand(), Err("country table has no country slots"));
}
