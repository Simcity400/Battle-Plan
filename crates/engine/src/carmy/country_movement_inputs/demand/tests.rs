use super::*;
use crate::{
    ccountry::synthetic::World, creference_registry,
    creference_registry::tests::Fixture as Registry,
};
use std::cell::Cell;

struct Fixture {
    world: World,
    registry: Registry,
}

impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.world.memory().copy(address, output) || self.registry.copy(address, output)
    }
}

impl Fixture {
    fn new() -> Self {
        let mut world = World::new(0, 3, 0x400);
        let state = world.game_state();
        let [countries, links] = ccountry::Reader::new(&world.memory(), 0, state)
            .unwrap()
            .arrays();
        // Raw 3 is the current id of canonical country 2. Its alias roster
        // must never cause a second scan of the same native unit population.
        world.u64(countries.element(3, 8).unwrap(), world.object(2) as u64);
        world.u32(links.element(3, 4).unwrap(), 2);
        world.u32(world.object(2) + ccountry::ID, 3);
        // Only this map header is a sizing input: its payload is absent.
        world.array(state + cgame_state::STRATEGIC_REGIONS, 0x0900_0000, 3);
        Self {
            world,
            registry: Registry::new(creference_registry::MID_TYPE_LIMIT),
        }
    }

    fn division(&mut self, roster: u32, logical: u32, moving: bool) -> usize {
        let unit = self.world.alloc(0x220);
        self.world.u64(unit, crate::carmy::VTABLE_RVA as u64);
        self.world.u32(unit + cunit::CLASS, 0);
        self.world
            .u64(unit + cunit::HANDLE, (unit as u64) << 32 | 4713);
        self.world.u32(unit + cunit::ROSTER_OWNER, roster);
        self.world.u32(unit + cunit::LOGICAL_OWNER, logical);
        self.world.array(
            unit + cunit::ORDINARY_ROUTE_DATA,
            if moving { 0x0a00_0000 } else { 0 },
            u32::from(moving),
        );
        // Leave secondary identity zero: division ETA does not require it.
        unit
    }

    fn gun(&mut self, roster: u32, logical: u32) -> usize {
        let unit = self.world.alloc(0x220);
        let handle = (42_u64 << 32) | u64::from(creference_registry::MID_TYPE_LIMIT);
        self.world.u64(unit, crailway_gun::VTABLE_RVA as u64);
        self.world.u32(unit + cunit::CLASS, 13);
        self.world.u64(
            unit + crate::carmy::SECONDARY_BASE,
            crailway_gun::SECONDARY_VTABLE_RVA as u64,
        );
        self.world.u64(unit + cunit::HANDLE, handle);
        self.world.u32(unit + cunit::ROSTER_OWNER, roster);
        self.world.u32(unit + cunit::LOGICAL_OWNER, logical);
        self.world
            .array(unit + cunit::ORDINARY_ROUTE_DATA, 0x0b00_0000, 1);
        self.registry
            .entry(Registry::index(handle), 1, handle, (unit + 0x10) as u64);
        self.registry
            .put(0x1000 + creference_registry::COUNT, &1_u32.to_le_bytes());
        unit
    }

    fn roster(&mut self, country: u32, offset: usize, units: &[usize]) -> usize {
        let data = self.world.alloc(units.len() * 8);
        for (index, &unit) in units.iter().enumerate() {
            self.world.u64(data + index * 8, unit as u64);
        }
        self.world.array(
            self.world.object(country) + offset,
            data,
            units.len() as u32,
        );
        data
    }

    fn observe(&self) -> LiveDemand {
        observe_live(self, 0, self.world.game_state())
    }
}

struct Fault<'a> {
    fixture: &'a Fixture,
    address: usize,
    calls: Cell<usize>,
    once: bool,
}

impl Memory for Fault<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address <= self.address && self.address - address < output.len() {
            let calls = self.calls.get();
            self.calls.set(calls + 1);
            if !self.once || calls == 0 {
                return false;
            }
        }
        self.fixture.copy(address, output)
    }
}

#[test]
fn aliases_and_repeated_movers_count_distinct_owners_without_payload_reads() {
    let mut fixture = Fixture::new();
    let first = fixture.division(1, 3, true);
    let second = fixture.division(3, 2, true);
    let gun = fixture.gun(3, 1);
    fixture.roster(1, ccountry::DIVISIONS, &[0, first, first]);
    fixture.roster(2, ccountry::DIVISIONS, &[second]);
    fixture.roster(2, ccountry::RAILWAY_UNITS, &[gun]);
    let no_secondary = Fault {
        fixture: &fixture,
        address: first + crate::carmy::SECONDARY_BASE,
        calls: Cell::new(0),
        once: false,
    };
    assert_eq!(
        observe_live(&no_secondary, 0, fixture.world.game_state()),
        LiveDemand {
            country_slots: Ok(4),
            raw_links: Ok(4),
            region_bound: Ok(3),
            movers: Ok(MovingOwners { logical: 2 }),
        }
    );
    assert_eq!(no_secondary.calls.get(), 0);
    // The alias is authenticated, but the canonical country's rosters are
    // visited once, including when the native current id is the alias.
    let header = fixture.world.object(2) + ccountry::DIVISIONS;
    struct Count<'a>(&'a Fixture, usize, Cell<usize>);
    impl Memory for Count<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == self.1 {
                self.2.set(self.2.get() + 1);
            }
            self.0.copy(address, output)
        }
    }
    let counter = Count(&fixture, header, Cell::new(0));
    assert!(
        observe_live(&counter, 0, fixture.world.game_state())
            .movers
            .is_ok()
    );
    assert_eq!(counter.2.get(), 1);
}

#[test]
fn idle_divisions_and_railways_demand_owners_without_route_payload_reads() {
    let mut fixture = Fixture::new();
    let first = fixture.division(1, 3, false);
    let idle = fixture.division(3, 3, false);
    let gun = fixture.gun(3, 1);
    // Every receiver has a valid zero-count header and its logical owner
    // remains demanded. The first division's retained empty-vector pointer
    // cannot cause a route payload read.
    fixture
        .world
        .array(first + cunit::ORDINARY_ROUTE_DATA, 0x0a00_0000, 0);
    fixture.world.array(gun + cunit::ORDINARY_ROUTE_DATA, 0, 0);
    fixture.roster(1, ccountry::DIVISIONS, &[first]);
    fixture.roster(2, ccountry::DIVISIONS, &[idle, idle]);
    fixture.roster(2, ccountry::RAILWAY_UNITS, &[gun]);
    assert_eq!(
        read::<u32>(&fixture, idle + cunit::ORDINARY_ROUTE_COUNT),
        Some(0)
    );
    assert_eq!(
        read::<u32>(&fixture, gun + cunit::ORDINARY_ROUTE_COUNT),
        Some(0)
    );
    let no_payload = Fault {
        fixture: &fixture,
        address: 0x0a00_0000,
        calls: Cell::new(0),
        once: false,
    };
    assert_eq!(
        observe_live(&no_payload, 0, fixture.world.game_state()).movers,
        Ok(MovingOwners { logical: 2 })
    );
    assert_eq!(no_payload.calls.get(), 0);
    assert!(LiveDemand::default().movers.is_err());
}

#[test]
fn invalid_idle_route_header_refuses_before_demanded_owner_reads() {
    let mut fixture = Fixture::new();
    let idle = fixture.division(1, 3, false);
    fixture.roster(1, ccountry::DIVISIONS, &[idle]);
    // Positive count with null data cannot be reinterpreted as a valid idle
    // route, even though no route payload is required by the demand reader.
    fixture.world.array(idle + cunit::ORDINARY_ROUTE_DATA, 0, 1);
    let owner = Fault {
        fixture: &fixture,
        address: idle + cunit::LOGICAL_OWNER,
        calls: Cell::new(0),
        once: false,
    };
    assert_eq!(
        observe_live(&owner, 0, fixture.world.game_state()).movers,
        Err("country movement demand array unreadable or invalid")
    );
    assert_eq!(owner.calls.get(), 0);
}

#[test]
fn failed_later_idle_owner_withholds_partial_counts_but_retains_every_header_floor() {
    let mut fixture = Fixture::new();
    let first = fixture.division(1, 1, true);
    let later = fixture.division(3, 3, false);
    fixture.roster(1, ccountry::DIVISIONS, &[first]);
    fixture.roster(2, ccountry::DIVISIONS, &[later]);
    let memory = Fault {
        fixture: &fixture,
        address: later + cunit::LOGICAL_OWNER,
        calls: Cell::new(0),
        once: true,
    };
    let observed = observe_live(&memory, 0, fixture.world.game_state());
    assert!(observed.movers.is_err());
    assert_eq!(
        (
            observed.country_slots,
            observed.raw_links,
            observed.region_bound
        ),
        (Ok(4), Ok(4), Ok(3))
    );
    assert_eq!(memory.calls.get(), 1);
    assert_eq!(
        observe_live(&memory, 0, fixture.world.game_state()).movers,
        Ok(MovingOwners { logical: 2 })
    );
}

#[test]
fn transient_country_header_failure_is_not_retried_and_other_headers_survive() {
    let fixture = Fixture::new();
    let memory = Fault {
        fixture: &fixture,
        address: fixture.world.game_state() + cgame_state::COUNTRIES,
        calls: Cell::new(0),
        once: true,
    };
    let observed = observe_live(&memory, 0, fixture.world.game_state());
    assert!(observed.country_slots.is_err());
    assert!(observed.movers.is_err());
    assert_eq!((observed.raw_links, observed.region_bound), (Ok(4), Ok(3)));
    assert_eq!(memory.calls.get(), 1);
}

#[test]
fn independent_map_failure_does_not_erase_the_complete_mover_census() {
    let mut fixture = Fixture::new();
    let unit = fixture.division(1, 3, true);
    fixture.roster(1, ccountry::DIVISIONS, &[unit]);
    let memory = Fault {
        fixture: &fixture,
        address: fixture.world.game_state() + cgame_state::STRATEGIC_REGIONS,
        calls: Cell::new(0),
        once: false,
    };
    let observed = observe_live(&memory, 0, fixture.world.game_state());
    assert!(observed.region_bound.is_err());
    assert_eq!(observed.movers, Ok(MovingOwners { logical: 1 }));
}

#[test]
fn invalid_railway_secondary_and_null_counted_slots_refuse_the_census() {
    let mut fixture = Fixture::new();
    let gun = fixture.gun(3, 1);
    fixture.roster(2, ccountry::RAILWAY_UNITS, &[gun]);
    fixture.world.u64(gun + crate::carmy::SECONDARY_BASE, 0);
    assert!(fixture.observe().movers.is_err());
    fixture.roster(2, ccountry::RAILWAY_UNITS, &[0]);
    assert!(fixture.observe().movers.is_err());
}
