use super::*;
use engine_native::{cgame_state, memory::SliceMemory};
use std::cell::RefCell;

const GAME: usize = 0x1000;
const COUNTRIES: usize = 0x4000;
const LINKS: usize = 0x10000;
const COUNTRY: usize = 0x20000;
const BLOCK: usize = 0x800;
const SPEED_KEY: u32 = 17;

struct World {
    bytes: Vec<u8>,
    reads: RefCell<Vec<usize>>,
    fault: Option<usize>,
}
impl World {
    fn new(countries: usize) -> Self {
        let mut world = Self {
            bytes: vec![
                0;
                COUNTRY
                    + countries * BLOCK
                    + BLOCK.max(ccountry::DIPLOMACY_STATUS + size_of::<u64>())
            ],
            reads: RefCell::default(),
            fault: None,
        };
        world.header(GAME + cgame_state::COUNTRIES, COUNTRIES, countries + 1);
        world.header(GAME + cgame_state::COUNTRY_LINKS, LINKS, countries + 1);
        for owner in 1..=countries {
            let country = COUNTRY + owner * BLOCK;
            world.u64(COUNTRIES + owner * 8, country as u64);
            world.u32(LINKS + owner * 4, owner as u32);
            world.u64(country, ccountry::VTABLE_RVA.0 as u64);
            world.u32(country + ccountry::ID, owner as u32);
            world.header(country + ccountry::MODIFIER_ACCUMULATOR, country + 0x700, 1);
            world.u32(country + 0x700, SPEED_KEY);
            world.u64(country + 0x708, 10_000);
        }
        world
    }
    fn u32(&mut self, at: usize, n: u32) {
        self.bytes[at..at + 4].copy_from_slice(&n.to_le_bytes());
    }
    fn u64(&mut self, at: usize, n: u64) {
        self.bytes[at..at + 8].copy_from_slice(&n.to_le_bytes());
    }
    fn header(&mut self, at: usize, data: usize, count: usize) {
        self.u64(at, data as u64);
        self.u32(at + 8, count as u32);
        self.u32(at + 12, count as u32);
    }
    fn value(&mut self, owner: usize, raw: i64) {
        self.u64(COUNTRY + owner * BLOCK + 0x708, raw as u64);
    }
}
impl Memory for World {
    fn core_supply_access(
        &self,
        _: usize,
        _: usize,
        _: u32,
        area: usize,
    ) -> Option<Result<bool, &'static str>> {
        self.reads.borrow_mut().push(area);
        Some(Ok(true))
    }
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(at);
        if self
            .fault
            .is_some_and(|fault| at <= fault && fault < at.saturating_add(out.len()))
        {
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(at, out)
    }
}
struct ChangesForTest;
impl Changes for ChangesForTest {
    fn take(&self, _: Table, _: &mut dyn FnMut(usize)) -> Option<u64> {
        Some(1)
    }
}
fn changes() -> &'static ChangesForTest {
    &ChangesForTest
}
fn movers(capture: &mut SpeedTerms, countries: usize) {
    capture.keys = vec![SPEED_KEY, SPEED_KEY, SPEED_KEY + 1];
    capture.templates = vec![(1, Ok((0, 3)))];
    capture.movers = (1..=countries)
        .map(|owner| Mover {
            division: owner as u64,
            attacking: Ok(false),
            land: Ok(true),
            strategic: Some(0),
            roster: Some(owner as u32),
            owner: Some(owner as u32),
            logical: Some(owner as u32),
            location: Ok(1),
            template: Some(1),
            noncore: Some(false),
            transfer: Some(false),
            route: Ok((0, 0)),
        })
        .collect();
}
fn entries(capture: &mut SpeedTerms, world: &World, marked: Vec<u32>, epoch: u64) {
    capture.entries.clear();
    capture.counts = [[0; 3]; FAMILIES];
    let reader = ccountry::Reader::new(world, 0, GAME);
    assert!(reader.is_ok());
    let outcome = capture.capture_entries(
        world,
        &reader,
        Take {
            epoch: Some(epoch),
            complete: true,
            marked,
        },
    );
    assert_eq!(
        outcome,
        Outcome::Rows {
            len: capture.movers.len() * 2
        }
    );
}

#[test]
fn shared_unit_speed_history_follows_hourly_marks_across_pooled_frames() {
    let mut world = World::new(2);
    let changes = changes();
    let store = Mutex::new(None);
    let mut a = SpeedTerms {
        changes,
        ..SpeedTerms::default()
    };
    let mut b = SpeedTerms {
        changes,
        ..SpeedTerms::default()
    };
    movers(&mut a, 2);
    movers(&mut b, 2);
    a.capture_holding(&store, |c| entries(c, &world, vec![], 1));
    assert_eq!(a.counts[2], [4, 0, 0]);
    assert_eq!(a.entries[0].entry, Some(Some(10_000)));
    assert_eq!(a.entries[1].entry, Some(None));
    world.value(1, -20_000);
    b.capture_holding(&store, |c| entries(c, &world, vec![1], 1));
    assert_eq!(b.counts[2], [2, 2, 0]);
    a.capture_holding(&store, |c| entries(c, &world, vec![], 1));
    assert_eq!(a.counts[2], [0, 4, 0]);
    assert_eq!(a.entries[0].entry, Some(Some(-20_000)));
    assert!(a.held.epoch.is_none());
    world.value(1, 30_000);
    b.capture_holding(&store, |c| entries(c, &world, vec![], 2));
    assert_eq!(b.counts[2], [4, 0, 0]);
    assert_eq!(b.entries[0].entry, Some(Some(30_000)));
}

#[test]
fn deferred_unit_speed_entries_keep_owned_values_and_refusals() {
    for countries in [0, 1, 7, 512] {
        let mut world = World::new(countries.max(1));
        let changes = changes();
        let mut capture = SpeedTerms { changes, ..SpeedTerms::default() };
        movers(&mut capture, countries);
        entries(&mut capture, &world, vec![], 1);
        if countries != 0 {
            world.value(1, -70_000);
        }
        if countries > 1 {
            world.fault = Some(COUNTRY + 2 * BLOCK + 0x708);
        }
        capture.entries.clear();
        capture.counts = [[0; 3]; FAMILIES];
        {
            let reader = ccountry::Reader::new(&world, 0, GAME);
            assert_eq!(capture.acquire_entries(&world, &reader, Take {
                epoch: Some(1), complete: true, marked: (1..=countries as u32).collect(),
            }), Outcome::Rows { len: countries * 2 });
        }
        assert!(capture.entries.is_empty(), "acquisition assembles no unit-speed rows");
        drop(world);
        assert_eq!(capture.settle_entries(), Outcome::Rows { len: countries * 2 });
        assert_eq!(capture.counts[2], [(countries * 2 - usize::from(countries > 1)) as u64, 0, u64::from(countries > 1)]);
        for owner in 1..=countries {
            assert_eq!(capture.entries[(owner - 1) * 2], CountryUnitSpeedEntryRow {
                country: owner as u32,
                key: SPEED_KEY,
                entry: if owner == 2 { None } else { Some(Some(if owner == 1 { -70_000 } else { 10_000 })) },
            });
            assert_eq!(capture.entries[(owner - 1) * 2 + 1].entry, Some(None));
        }
    }
}

#[test]
fn deferred_unit_speed_loan_forgets_dropped_unwound_and_invalidated_baselines() {
    for complete in [false, true] {
        let mut loan = owned::Loan::take().unwrap();
        loan.epoch = Some(7);
        if complete {
            loan.return_complete();
        }
        drop(loan);
        let next = owned::Loan::take().unwrap();
        assert_eq!(next.epoch, complete.then_some(7));
        drop(next);
    }
    let result = std::panic::catch_unwind(|| {
        let mut loan = owned::Loan::take().unwrap();
        loan.epoch = Some(8);
        panic!("synthetic unit speed decode unwind");
    });
    assert!(result.is_err());
    let mut loan = owned::Loan::take().unwrap();
    assert_eq!(loan.epoch, None);
    loan.epoch = Some(9);
    owned::forget();
    loan.return_complete();
    assert_eq!(owned::Loan::take().unwrap().epoch, None);
    let mut capture = SpeedTerms {
        held: owned::Loan::take().unwrap(),
        ..Default::default()
    };
    capture.held.epoch = Some(10);
    capture.settle_pending = false;
    capture.reset();
    assert_eq!(owned::Loan::take().unwrap().epoch, None);
}

#[test]
fn discarded_frame_without_army_store_returns_unit_speed_loan() {
    struct Owned;
    impl Memory for Owned {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("unavailable owned speed census must not read native memory")
        }
    }
    let sources = || super::super::Sources {
        rows: &[],
        outcome: Outcome::Unavailable {
            reason: "discarded frame fixture",
        },
    };
    let mut frame = super::super::Captured::vacant();
    let mut pass = frame.take_speed_terms();
    pass.capture(&Owned, 0, 0, sources());
    frame.restore_speed_terms(pass);
    assert!(frame.take_settled_store().is_none());
    let mut next = super::super::Captured::vacant();
    let mut pass = next.take_speed_terms();
    pass.capture(&Owned, 0, 0, sources());
}

#[test]
fn core_supply_access_is_read_once_per_owner_area_and_not_for_bypassed_movers() {
    let world = World::new(2);
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 2);
    for (index, mover) in capture.movers.iter_mut().enumerate() {
        mover.owner = Some(1);
        mover.location = Ok(index as u32 + 1);
        mover.noncore = Some(true);
    }
    let bypassed = Mover {
        location: Ok(3),
        noncore: Some(false),
        ..capture.movers[0]
    };
    capture.movers.push(bypassed);
    capture.provinces = (1..=3)
        .map(|id| {
            (
                id,
                Ok(cprovince::MovementProvince {
                    state: Ok(0),
                    controller: Ok(1),
                    controller_area: Ok(0x8000),
                    strategic_region: Ok(0),
                    terrain_flags: Ok(engine_native::province_definition::LAND_FLAG),
                }),
            )
        })
        .collect();
    assert_eq!(
        capture.capture_access(&world, 0, GAME),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(*world.reads.borrow(), [0x8000]);
    assert!(capture.access.iter().all(|r| r.access == Some(true)));
}

#[test]
fn combat_movement_uses_the_first_attacking_combat_and_shares_its_read() {
    use engine_native::{cland_combat as combat, cland_combatant as side};
    const COMBAT: usize = 0x22000;
    const SIDE: usize = 0x22400;
    const LIST: usize = 0x22800;
    const ATTACKERS: usize = 0x22900;
    const DIVISION: usize = 0x22a00;
    let mut world = World::new(8);
    world.u64(COMBAT, combat::VTABLE_RVA.0 as u64);
    world.u64(COMBAT + combat::MOVEMENT_MODIFIER, (-13_000_i64) as u64);
    world.u64(COMBAT + combat::ATTACKER, SIDE as u64);
    world.u64(SIDE, side::VTABLE_RVA.0 as u64);
    world.u64(SIDE + side::COMBAT, COMBAT as u64);
    world.header(SIDE + side::UNITS, ATTACKERS, 1);
    world.u64(ATTACKERS, DIVISION as u64);
    world.u64(LIST, COMBAT as u64);
    let mut header = [0; 16];
    header[..8].copy_from_slice(&(LIST as u64).to_le_bytes());
    header[8..12].copy_from_slice(&1_u32.to_le_bytes());
    header[12..].copy_from_slice(&1_u32.to_le_bytes());
    let seed = |division, pointer| DivisionSeed {
        division,
        pointer,
        combat: Some(header),
        withdrawal: Some(0),
        ..DivisionSeed::default()
    };
    let seeds = [seed(1, DIVISION), seed(2, DIVISION), seed(3, DIVISION + 1)];
    let mut capture = SpeedTerms {
        combat_demand: seeds.iter().map(|seed| (seed.division, Ok(true))).collect(),
        ..Default::default()
    };
    capture.capture_combat(&world, 0, &seeds).unwrap();
    assert_eq!(
        capture.combat,
        [
            (1, Some(Some(-13_000))),
            (2, Some(Some(-13_000))),
            (3, Some(None))
        ]
    );
    assert_eq!(capture.combats.len(), 1);
    assert_eq!(
        world
            .reads
            .borrow()
            .iter()
            .filter(|&&at| at == COMBAT + combat::MOVEMENT_MODIFIER)
            .count(),
        1
    );
}

#[test]
fn inactive_and_early_return_movers_read_no_combat_lists() {
    let world = World::new(1);
    let seeds: Vec<_> = (1..=5_000)
        .map(|division| DivisionSeed {
            division,
            ..DivisionSeed::default()
        })
        .collect();
    let mut capture = SpeedTerms {
        combat_demand: vec![(2, Ok(false)), (3, Ok(false))],
        ..Default::default()
    };
    capture.capture_combat(&world, 0, &seeds).unwrap();
    assert!(world.reads.borrow().is_empty());
    assert!(capture.combat.iter().all(|(_, value)| *value == Some(None)));
    assert_eq!(capture.counts[COMBAT], [0, 0, 0]);
}

#[test]
fn transfer_demands_no_land_families_but_shared_land_keys_remain() {
    let world = World::new(2);
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 2);
    capture.movers[1].transfer = Some(true);
    capture.movers[1].land = Ok(false);
    let countries = ccountry::Reader::new(&world, 0, GAME);
    world.reads.borrow_mut().clear();
    assert_eq!(
        capture.capture_states(&world, 0, GAME),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(capture.counts[0][2], 1);
    assert_eq!(
        capture.capture_access(&world, 0, GAME),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(
        capture.capture_entries(
            &world,
            &countries,
            Take {
                epoch: None,
                complete: false,
                marked: vec![],
            }
        ),
        Outcome::Rows { len: 2 }
    );
    assert!(capture.entries.iter().all(|row| row.country == 1));
    assert!(
        !world
            .reads
            .borrow()
            .contains(&(COUNTRY + 2 * BLOCK + ccountry::MODIFIER_ACCUMULATOR))
    );
    capture.movers[0].land = Ok(false);
    capture.counts = [[0; 3]; FAMILIES];
    world.reads.borrow_mut().clear();
    assert_eq!(
        capture.capture_states(&world, 0, GAME),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(
        capture.capture_access(&world, 0, GAME),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(
        capture.capture_rivers_using(&world, 0, GAME, &countries, &Mutex::new(None)),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(capture.counts[..4], [[0; 3]; 4]);
    assert!(world.reads.borrow().is_empty());
}

#[test]
fn land_only_transport_demand_never_allocates_and_later_refusals_keep_sources() {
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 5_000);
    capture.route = vec![2].into_iter().map(Ok).collect();
    capture.provinces = (1..=3)
        .map(|id| {
            (
                id,
                Ok(cprovince::MovementProvince {
                    state: Ok(0),
                    controller: Ok(1),
                    controller_area: Ok(0),
                    strategic_region: Ok(id as usize * 16),
                    terrain_flags: Ok(if id == 3 {
                        0
                    } else {
                        engine_native::province_definition::LAND_FLAG
                    }),
                }),
            )
        })
        .collect();
    let mut demand = Vec::new();
    for mover in &mut capture.movers {
        mover.route = Ok((0, 1));
    }
    let growths = engine_allocation::growths();
    let result = collector_test_support::with_allocations_denied(|| {
        for mover in &capture.movers {
            append_transport_demand(&mut demand, &capture.route, &capture.provinces, mover)?;
        }
        Ok::<(), &'static str>(())
    });
    assert_eq!(result, Ok(()));
    assert_eq!(engine_allocation::growths(), growths);
    assert_eq!(demand.capacity(), 0);
    let mover = capture.movers[0];
    capture.route = vec![Ok(3), Err("later route position refused")];
    let mut mover = Mover {
        route: Ok((0, 2)),
        ..mover
    };
    demand.push((7, 42));
    assert_eq!(
        append_transport_demand(&mut demand, &capture.route, &capture.provinces, &mover),
        Ok(1)
    );
    assert_eq!(demand, [(7, 42), (1, 16), (1, 48)]);
    mover.logical = None;
    let established = demand.clone();
    assert_eq!(
        append_transport_demand(&mut demand, &capture.route, &capture.provinces, &mover),
        Ok(2)
    );
    assert_eq!(demand, established);
}

#[test]
fn transfer_branch_prepares_without_any_readable_land_state_or_template() {
    const PATH: usize = 0x18000;
    let mut world = World::new(1);
    world.u32(PATH, 2);
    let mut header = [0; 16];
    header[..8].copy_from_slice(&(PATH as u64).to_le_bytes());
    header[8..12].copy_from_slice(&1_u32.to_le_bytes());
    header[12..].copy_from_slice(&1_u32.to_le_bytes());
    let seed = DivisionSeed {
        division: 1,
        ordinary_route: Some(header),
        roster_owner: Some(1),
        expeditionary_owner: Some(0),
        logical_owner: Some(1),
        naval_transfer_membership: Some(1),
        standard_template: Some(usize::MAX),
        ..DivisionSeed::default()
    };
    let mut capture = SpeedTerms::default();
    capture.prepare(&world, 0, GAME, &[seed]).unwrap();
    assert_eq!(capture.movers.len(), 1);
    assert_eq!(capture.movers[0].land, Ok(false));
    assert_eq!(capture.combat_demand, [(1, Ok(false))]);
    assert!(capture.templates.is_empty());
    assert!(capture.state_facts.is_empty());
    assert_eq!(
        capture.capture_states(&world, 0, GAME),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(
        capture.capture_access(&world, 0, GAME),
        Outcome::Rows { len: 0 }
    );
    let countries = ccountry::Reader::new(&world, 0, GAME);
    assert_eq!(
        capture.capture_entries(
            &world,
            &countries,
            Take {
                epoch: None,
                complete: false,
                marked: vec![]
            }
        ),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(
        capture.capture_rivers_using(&world, 0, GAME, &countries, &Mutex::new(None)),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(capture.counts[..4], [[0; 3]; 4]);
}

fn attacking_mover(world: &mut World) -> DivisionSeed {
    use engine_native::{cland_combat as combat, cland_combatant as side};
    const COMBAT: usize = 0x22000;
    const SIDE: usize = 0x22400;
    const LIST: usize = 0x22800;
    const ATTACKERS: usize = 0x22900;
    const DIVISION: usize = 0x22a00;
    world.u64(COMBAT, combat::VTABLE_RVA.0 as u64);
    world.u64(COMBAT + combat::MOVEMENT_MODIFIER, 20_000);
    world.u64(COMBAT + combat::ATTACKER, SIDE as u64);
    world.u64(SIDE, side::VTABLE_RVA.0 as u64);
    world.u64(SIDE + side::COMBAT, COMBAT as u64);
    world.header(SIDE + side::UNITS, ATTACKERS, 1);
    world.u64(ATTACKERS, DIVISION as u64);
    world.u64(LIST, COMBAT as u64);
    let mut header = [0; 16];
    header[..8].copy_from_slice(&(LIST as u64).to_le_bytes());
    header[8..12].copy_from_slice(&1_u32.to_le_bytes());
    header[12..].copy_from_slice(&1_u32.to_le_bytes());
    DivisionSeed {
        division: 1,
        pointer: DIVISION,
        combat: Some(header),
        withdrawal: Some(0),
        ..Default::default()
    }
}

fn route_facts(sea: bool) -> Result<cprovince::MovementProvince, &'static str> {
    Ok(cprovince::MovementProvince {
        controller: Ok(1),
        controller_area: Ok(0x8000),
        terrain_flags: Ok(if sea {
            0
        } else {
            engine_native::province_definition::LAND_FLAG
        }),
        ..Default::default()
    })
}

#[test]
fn later_province_refusal_preserves_established_land_and_combat_demand() {
    let mut world = World::new(8);
    let seed = attacking_mover(&mut world);
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 1);
    capture.route = vec![2, 3].into_iter().map(Ok).collect();
    capture.movers[0].route = Ok((0, 2));
    capture.movers[0].noncore = Some(true);
    capture.provinces = vec![
        (1, route_facts(false)),
        (2, route_facts(false)),
        (3, Err("later province refused")),
    ];
    capture.prepare_land_demand(&world, 0, GAME).unwrap();
    assert_eq!(capture.movers[0].land, Ok(true));
    assert_eq!(capture.combat_demand, [(1, Ok(true))]);
    assert_eq!(capture.counts[..4], [[0; 3]; 4]);
    assert_eq!(
        capture.capture_states(&world, 0, GAME),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(capture.counts[0][2], 0);
    assert_eq!(
        capture.capture_access(&world, 0, GAME),
        Outcome::Rows { len: 1 }
    );
    let countries = ccountry::Reader::new(&world, 0, GAME);
    assert_eq!(
        capture.capture_entries(
            &world,
            &countries,
            Take {
                epoch: None,
                complete: false,
                marked: vec![]
            }
        ),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        capture.capture_rivers_with_crossings(&world, 0, &countries, |_, _| Ok(Some(2))),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(capture.rivers[0].controller, 1);
    assert!(capture.counts[3][2] >= 1);
    capture.capture_combat(&world, 0, &[seed]).unwrap();
    assert_eq!(capture.combat, [(1, Some(Some(20_000)))]);
}

#[test]
fn mixed_itinerary_captures_combat_for_a_later_land_edge() {
    use replay_derive::eta_derive::{
        self as eta, CommonModifiers, Defines, EdgeTerm, SpeedTerms as Terms,
    };
    let mut world = World::new(8);
    let seed = attacking_mover(&mut world);
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 1);
    capture.route = vec![2, 3, 4].into_iter().map(Ok).collect();
    capture.movers[0].route = Ok((0, 3));
    capture.provinces = vec![
        (1, route_facts(false)),
        (2, route_facts(true)),
        (3, route_facts(false)),
        (4, route_facts(false)),
    ];
    capture.prepare_land_demand(&world, 0, GAME).unwrap();
    assert_eq!(capture.movers[0].land, Ok(true));
    assert_eq!(capture.combat_demand, [(1, Ok(true))]);
    capture.capture_combat(&world, 0, &[seed]).unwrap();
    let entry = capture.combat[0].1.expect("captured combat branch");
    assert_eq!(entry, Some(20_000));
    let modifier = eta::common_modifier(
        &CommonModifiers {
            combat_base: Some(eta::combat_base(entry, 0)),
            ..Default::default()
        },
        true,
        100_000,
        0,
        0,
        0,
    );
    let edge = |_, _| Ok(EdgeTerm::Normal(Default::default()));
    let terms = Terms {
        modifier,
        edge: &edge,
        organisation: None,
        fuel: None,
        base_speed: None,
    };
    let defines = Defines {
        outer_scale: 100_000,
        ..Default::default()
    };
    assert_eq!(
        eta::edge_speed(700_000, 3, 4, &terms, &defines),
        Ok(140_000)
    );
}

#[test]
fn transfer_partial_route_keeps_readable_prefix_and_suffix_regions_without_roster_keys() {
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 1);
    let mover = Mover {
        roster: None,
        owner: None,
        transfer: Some(true),
        route: Ok((0, 4)),
        ..capture.movers[0]
    };
    let path = [Ok(2), Err("route position refused"), Ok(3), Ok(4)];
    let provinces: Vec<_> = (1..=4)
        .map(|id| {
            (
                id,
                Ok(cprovince::MovementProvince {
                    strategic_region: Ok(id as usize * 16),
                    ..Default::default()
                }),
            )
        })
        .collect();
    let mut demand = vec![(7, 77)];
    assert_eq!(
        append_transport_demand(&mut demand, &path, &provinces, &mover),
        Ok(2)
    );
    assert_eq!(demand, [(7, 77), (1, 16), (1, 32), (1, 48)]);
    capture.movers = vec![mover];
    capture.route = path.to_vec();
    capture.provinces = provinces;
    let world = World::new(1);
    capture.prepare_land_demand(&world, 0, GAME).unwrap();
    assert_eq!(capture.movers[0].land, Ok(false));
    capture.movers[0].route = Ok((0, 2));
    capture.route = vec![Ok(2), Ok(3)];
    capture.provinces[2].1 = Err("later province facts refused");
    assert_eq!(
        capture.capture_transport(&world, 0, GAME),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(capture.transport_demand, [(1, 16), (1, 32)]);
}

#[test]
fn river_demand_is_only_inside_native_crossing_kinds_and_survives_later_refusal() {
    for kind in 0..=4 {
        let world = World::new(2);
        let mut capture = SpeedTerms::default();
        movers(&mut capture, 2);
        capture.route = vec![Ok(2), Err("later route position refused")];
        for mover in &mut capture.movers {
            mover.owner = Some(1);
            mover.route = Ok((0, 2));
        }
        capture.provinces = vec![(1, route_facts(false)), (2, route_facts(false))];
        let countries = ccountry::Reader::new(&world, 0, GAME);
        let result = capture.capture_rivers_with_crossings(&world, 0, &countries, |from, to| {
            assert_eq!((from, to), (1, 2));
            Ok(Some(kind))
        });
        let needed = usize::from((1..=3).contains(&kind));
        assert_eq!(result, Outcome::Rows { len: needed });
        assert_eq!(capture.rivers.len(), needed);
        if needed != 0 {
            assert_eq!(capture.rivers[0].entry, Some(None));
        }
        assert_eq!(capture.counts[3][2], 2);
    }
}

#[test]
fn river_crossing_answers_follow_work_across_threads() {
    use engine_native::{province_definition, province_neighbour};

    let slots = 0x12000;
    let province = 0x13000;
    let definition = 0x14000;
    let neighbour = 0x15000;
    let crossing = neighbour + province_neighbour::CROSSING_KIND;
    let (from, to) = (1, 12_345);
    let mut world = World::new(2);
    world.header(GAME + cgame_state::PROVINCES, slots, 2);
    world.u64(slots + 8, province as u64);
    world.u64(province, cprovince::VTABLE_RVA.0 as u64);
    world.u32(province + cprovince::ID, from);
    world.u64(province + cprovince::DEFINITION, definition as u64);
    world.u32(definition + province_definition::ID, from);
    world.header(definition + province_definition::NEIGHBOURS, neighbour, 1);
    world.u32(neighbour + province_neighbour::PROVINCE_ID, to);
    world.bytes[crossing] = 3;

    let capture = move |world: World| {
        let mut capture = SpeedTerms::default();
        movers(&mut capture, 1);
        capture.route = vec![Ok(to)];
        capture.movers[0].route = Ok((0, 1));
        capture.provinces = vec![(from, route_facts(false)), (to, route_facts(false))];
        let countries = ccountry::Reader::new(&world, 0, GAME);
        assert_eq!(
            capture.capture_rivers(&world, 0, GAME, &countries),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(capture.rivers[0].entry, Some(None));
        world
    };
    let mut world = std::thread::spawn(move || capture(world)).join().unwrap();
    assert!(world.reads.get_mut().contains(&crossing));
    world.reads.get_mut().clear();
    world.fault = Some(crossing);
    let mut world = std::thread::spawn(move || capture(world)).join().unwrap();
    assert!(!world.reads.get_mut().contains(&crossing));
}

#[test]
fn river_and_combat_skip_unselected_branches() {
    let mut world = World::new(8);
    let seed = attacking_mover(&mut world);
    let list = u64::from_le_bytes(seed.combat.unwrap()[..8].try_into().unwrap()) as usize;
    world.u64(list + 8, u64::MAX);
    let mut seed = seed;
    let mut header = seed.combat.unwrap();
    header[8..12].copy_from_slice(&2_u32.to_le_bytes());
    header[12..].copy_from_slice(&2_u32.to_le_bytes());
    seed.combat = Some(header);
    let mut capture = SpeedTerms {
        combat_demand: vec![(1, Ok(true))],
        ..Default::default()
    };
    capture.capture_combat(&world, 0, &[seed]).unwrap();
    assert_eq!(capture.combat, [(1, Some(Some(20_000)))]);
    assert!(!world.reads.borrow().contains(&(list + 8)));
    world.reads.borrow_mut().clear();
    seed.pointer += 1;
    header[8..12].copy_from_slice(&1_u32.to_le_bytes());
    header[12..].copy_from_slice(&1_u32.to_le_bytes());
    seed.combat = Some(header);
    capture.combat.clear();
    capture.capture_combat(&world, 0, &[seed]).unwrap();
    assert_eq!(capture.combat, [(1, Some(None))]);
    assert!(
        !world
            .reads
            .borrow()
            .contains(&(0x22000 + engine_native::cland_combat::MOVEMENT_MODIFIER))
    );
    for branch in [
        StateMovementTerms::FRIENDLY,
        StateMovementTerms::ENEMY,
        StateMovementTerms::NEITHER,
    ] {
        let (keys, len) = state_keys(branch);
        let selected = &keys[..len];
        assert!(selected.contains(&STATE_KEYS[0]));
        assert!(!selected.contains(&STATE_KEYS[3]));
        assert_eq!(
            selected.contains(&STATE_KEYS[1]),
            branch == StateMovementTerms::FRIENDLY
        );
        assert_eq!(
            selected.contains(&STATE_KEYS[2]),
            branch == StateMovementTerms::ENEMY
        );
    }
    assert_eq!(noncore_demand(Some(1), None), Some(false));
    assert_eq!(noncore_demand(None, Some(1)), Some(false));
    assert_eq!(noncore_demand(Some(0), Some(0)), Some(true));
    assert_eq!(noncore_demand(Some(0), None), None);
}

#[test]
fn refused_bulk_route_copy_retains_individually_readable_positions() {
    struct Partial;
    impl Memory for Partial {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            if out.len() != 4 || at == 8 {
                return false;
            }
            out.copy_from_slice(&((at / 4) as u32).to_le_bytes());
            true
        }
    }
    let path = Array { data: 4, count: 4 };
    let mut arena = vec![Ok(99)];
    let mut values = Vec::new();
    assert_eq!(
        copy_route(&Partial, path, &mut arena, &mut values),
        Ok((1, 4))
    );
    assert_eq!(
        arena,
        [
            Ok(99),
            Ok(1),
            Err("division route position unreadable"),
            Ok(3),
            Ok(4)
        ]
    );
}

#[test]
fn known_sea_transport_demand_does_not_require_transfer_membership() {
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 1);
    let mover = Mover {
        transfer: None,
        route: Ok((0, 2)),
        ..capture.movers[0]
    };
    let path = [Ok(2), Ok(3)];
    let provinces: Vec<_> = (1..=3)
        .map(|id| {
            (
                id,
                Ok(cprovince::MovementProvince {
                    terrain_flags: Ok(if id == 1 {
                        0
                    } else {
                        engine_native::province_definition::LAND_FLAG
                    }),
                    strategic_region: Ok(id as usize * 16),
                    ..Default::default()
                }),
            )
        })
        .collect();
    let mut demand = vec![];
    assert_eq!(
        append_transport_demand(&mut demand, &path, &provinces, &mover),
        Ok(1)
    );
    assert_eq!(demand, [(1, 16)]);
    capture.movers = vec![Mover {
        route: Ok((0, 1)),
        ..mover
    }];
    capture.route = path.to_vec();
    capture.provinces = provinces;
    let world = World::new(1);
    capture.prepare_land_demand(&world, 0, GAME).unwrap();
    assert_eq!(capture.movers[0].land, Ok(false));
}

#[test]
fn no_combat_and_withdrawal_do_not_require_unknown_land_demand() {
    let world = World::new(1);
    let mut capture = SpeedTerms {
        combat_demand: vec![(1, Err("route refused")), (2, Err("route refused"))],
        ..Default::default()
    };
    let seeds = [
        DivisionSeed {
            division: 1,
            combat: Some([0; 16]),
            ..Default::default()
        },
        DivisionSeed {
            division: 2,
            withdrawal: Some(1),
            ..Default::default()
        },
    ];
    capture.capture_combat(&world, 0, &seeds).unwrap();
    assert_eq!(capture.combat, [(1, Some(None)), (2, Some(None))]);
    assert!(world.reads.borrow().is_empty());
}

#[test]
fn first_self_edge_keeps_land_and_transfer_speed_demand() {
    let world = World::new(1);
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 1);
    capture.route = vec![Ok(1), Ok(2)];
    capture.movers[0].route = Ok((0, 2));
    capture.provinces = vec![(1, route_facts(false)), (2, route_facts(true))];
    capture.prepare_land_demand(&world, 0, GAME).unwrap();
    assert_eq!(capture.movers[0].land, Ok(true));
    capture.route = vec![Ok(1)];
    capture.movers[0].route = Ok((0, 1));
    capture.movers[0].transfer = Some(true);
    let from = cprovince::MovementProvince {
        controller_area: Err("unused controller area refused"),
        terrain_flags: Err("unused transfer terrain refused"),
        strategic_region: Ok(77),
        ..Default::default()
    };
    capture.provinces = vec![(1, Ok(from))];
    let mut demand = vec![];
    assert_eq!(
        append_transport_demand(
            &mut demand,
            &capture.route,
            &capture.provinces,
            &capture.movers[0]
        ),
        Ok(0)
    );
    assert_eq!(demand, [(1, 77)]);
}

#[test]
fn combat_prefix_match_survives_a_later_unreadable_member_for_shared_movers() {
    let mut world = World::new(8);
    let first = attacking_mover(&mut world);
    let combat = u64::from_le_bytes(world.bytes[0x22800..0x22808].try_into().unwrap()) as usize;
    let side = u64::from_le_bytes(
        world.bytes[combat + engine_native::cland_combat::ATTACKER
            ..combat + engine_native::cland_combat::ATTACKER + 8]
            .try_into()
            .unwrap(),
    ) as usize;
    world.header(side + engine_native::cland_combatant::UNITS, 0x22900, 2);
    world.fault = Some(0x22908);
    let mut capture = SpeedTerms {
        combat_demand: vec![(1, Ok(true)), (2, Ok(true)), (3, Ok(true))],
        ..Default::default()
    };
    let absent = DivisionSeed {
        division: 2,
        pointer: first.pointer + 1,
        ..first
    };
    let shared = DivisionSeed {
        division: 3,
        ..first
    };
    capture
        .capture_combat(&world, 0, &[first, absent, shared])
        .unwrap();
    assert_eq!(
        capture.combat,
        [(1, Some(Some(20_000))), (2, None), (3, Some(Some(20_000)))]
    );
    assert_eq!(
        world
            .reads
            .borrow()
            .iter()
            .filter(|&&at| at == 0x22908)
            .count(),
        1
    );
}

#[test]
fn peaceful_state_demand_skips_attack_and_shares_its_local_refusal_with_hostile_movers() {
    const STATE: usize = 0x18000;
    const BUCKET: usize = 0x19000;
    const ENTRIES: usize = 0x1a000;
    let mut world = World::new(2);
    world.u64(
        STATE + engine_native::cstate::COUNTRY_MODIFIERS,
        BUCKET as u64,
    );
    world.header(STATE + engine_native::cstate::BASE_MODIFIERS, ENTRIES, 4);
    for (index, key) in STATE_KEYS.iter().enumerate() {
        world.u32(ENTRIES + index * 16, *key);
    }
    world.fault = Some(ENTRIES + 3 * 16 + 8);
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 2);
    for mover in &mut capture.movers {
        mover.roster = Some(1);
        mover.owner = Some(1);
    }
    capture.provinces = vec![(
        1,
        Ok(cprovince::MovementProvince {
            state: Ok(STATE),
            ..Default::default()
        }),
    )];
    capture.state_facts = vec![(STATE, Ok((9, 1)))];
    assert_eq!(
        capture.capture_states(&world, 0, GAME),
        Outcome::Rows { len: 1 }
    );
    let terms = capture.states[0].terms.unwrap();
    assert_eq!(terms.branch, StateMovementTerms::FRIENDLY);
    assert!(!world.reads.borrow().contains(&world.fault.unwrap()));
    capture.states.clear();
    capture.movers[1].attacking = Ok(true);
    assert_eq!(
        capture.capture_states(&world, 0, GAME),
        Outcome::Rows { len: 1 }
    );
    let terms = capture.states[0].terms.unwrap();
    assert_eq!(terms.army_speed, Some(0));
    assert_eq!(
        terms.branch,
        StateMovementTerms::FRIENDLY | StateMovementTerms::ATTACK_REFUSED
    );
}

#[test]
fn river_refusals_have_one_owner_across_preparation_and_capture() {
    let world = World::new(1);
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 1);
    capture.route = vec![Ok(2), Ok(3), Ok(4)];
    capture.movers[0].route = Ok((0, 3));
    capture.provinces = vec![
        (1, route_facts(false)),
        (2, Err("province refused")),
        (3, route_facts(false)),
        (4, route_facts(false)),
    ];
    capture.prepare_land_demand(&world, 0, GAME).unwrap();
    assert_eq!(capture.movers[0].land, Ok(true));
    assert_eq!(capture.counts[3][2], 0);
    let countries = ccountry::Reader::new(&world, 0, GAME);
    assert_eq!(
        capture.capture_rivers_with_crossings(&world, 0, &countries, |from, to| {
            assert_eq!((from, to), (3, 4));
            Ok(Some(2))
        }),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(capture.rivers[0].entry, Some(None));
    assert_eq!(capture.counts[3][2], 2);
    capture.counts[3][2] = 0;
    capture.movers[0].location = Err("location refused");
    capture.prepare_land_demand(&world, 0, GAME).unwrap();
    assert_eq!(capture.counts[3][2], 1);
    assert_eq!(
        capture.capture_rivers_using(&world, 0, GAME, &countries, &Mutex::new(None)),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(capture.counts[3][2], 1);
}

#[test]
fn country_entry_scratch_is_reused_and_one_failed_key_keeps_healthy_entries() {
    let mut world = World::new(64);
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 64);
    let owners: Vec<_> = (1..=64).collect();
    entries(&mut capture, &world, owners.clone(), 1);
    let keys = capture.observed_entries.wanted.as_ptr();
    let values = capture.observed_entries.values.as_ptr();
    entries(&mut capture, &world, owners, 1);
    assert_eq!(capture.observed_entries.wanted.as_ptr(), keys);
    assert_eq!(capture.observed_entries.values.as_ptr(), values);
    world.fault = Some(COUNTRY + BLOCK + 0x708);
    entries(&mut capture, &world, vec![1], 1);
    assert_eq!(capture.entries[0].entry, None);
    assert_eq!(capture.entries[1].entry, Some(None));
    assert!(
        capture
            .entries
            .iter()
            .skip(2)
            .all(|row| row.entry.is_some())
    );
}

#[test]
fn speed_term_projection_keeps_frame_bytes_for_quiet_changes_and_refusals() {
    use collector_projection::projection::{Projection, ProjectionScratch};
    use replay_frame::frame::{FrameEncoder, FrameInput, Metrics};

    let input = |hour| FrameInput {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| {
            collector_projection::table_conversion::unavailable("fixture table").into()
        }),
        metrics: Metrics::default(),
    };
    for force in [false, true] {
        let mut capture = SpeedTerms::default();
        for owner in 1..=5_000 {
            capture.states.push(MovementStateTermRow {
                state: owner,
                roster: owner,
                owner,
                terms: Some(StateMovementTerms::default()),
            });
            capture.access.push(CoreSupplyAccessRow {
                owner,
                province: 1,
                access: Some(true),
            });
            capture.entries.push(CountryUnitSpeedEntryRow {
                country: owner,
                key: SPEED_KEY,
                entry: Some(Some(123)),
            });
            capture.rivers.push(RiverRelationRow {
                owner,
                controller: 1,
                entry: Some(None),
            });
            capture.transport.push(OwnerTransportSpeedRow {
                owner,
                region: 1,
                speed: Some(456),
            });
        }
        capture.outcomes = [Outcome::Rows { len: 5_000 }; N];
        let scratch = ProjectionScratch::default();
        let mut optimized = FrameEncoder::new();
        let mut reference = FrameEncoder::new();
        for hour in 0..=8 {
            match hour {
                2 => {
                    capture.states.reverse();
                    capture.access.reverse();
                    capture.entries.reverse();
                    capture.rivers.reverse();
                    capture.transport.reverse();
                }
                3 => {
                    capture.states[36].terms = None;
                    capture.access[36].access = None;
                    capture.entries[36].entry = None;
                    capture.rivers[36].entry = Some(Some(-123));
                    capture.transport[36].speed = None;
                }
                4 => {
                    capture.states.pop();
                    capture.access.pop();
                    capture.entries.pop();
                    capture.rivers.pop();
                    capture.transport.pop();
                    capture.outcomes = [Outcome::Rows { len: 4_999 }; N];
                }
                5 => {
                    capture.states[1] = capture.states[0];
                    capture.access[1] = capture.access[0];
                    capture.entries[1] = capture.entries[0];
                    capture.rivers[1] = capture.rivers[0];
                    capture.transport[1] = capture.transport[0];
                }
                6 => {
                    capture.outcomes = [Outcome::Unavailable {
                        reason: "fixture refusal",
                    }; N];
                    capture.outcomes[2] = Outcome::Truncated {
                        len: 1,
                        unread_rows: 4_998,
                    };
                }
                7 => {
                    capture.outcomes = [Outcome::Truncated {
                        len: 2,
                        unread_rows: 0,
                    }; N];
                }
                8 => capture.outcomes = [Outcome::Rows { len: 5_000 }; N],
                _ => {}
            }
            let mut projection = Projection::new(&optimized, 1, hour, force, &scratch);
            let actual = if !force && hour == 1 {
                collector_test_support::with_allocations_denied(|| {
                    capture.table_inputs(Some(&mut projection))
                })
            } else {
                capture.table_inputs(Some(&mut projection))
            };
            if !force && matches!(hour, 1 | 2) {
                assert!(actual.iter().all(|input| matches!(input, TableInput::ComparedEqual(_))));
            }
            let mut next = input(hour);
            let mut control = input(hour);
            for (table, (actual, expected)) in TABLES
                .into_iter()
                .zip(actual.into_iter().zip(capture.tables()))
            {
                next.tables[table.index()] = actual;
                control.tables[table.index()] = expected.into();
            }
            assert_eq!(
                optimized.encode_input(next, force).unwrap(),
                reference.encode_input(control, force).unwrap(),
                "hour {hour}, force {force}"
            );
        }
    }
}

const HOSTILITY_ARRAY: usize = 0x30000;
const HOSTILITY_PROVINCE: usize = 0x31000;
const HOSTILITY_ARMY: usize = 0x40000;
const HOSTILITY_IDS: u32 = 6;
const HOSTILITY_ARMIES: [(&[u32], u32); 9] = [
    (&[1, 3], 2),
    (&[4], 1),
    (&[2], 1),
    (&[5], 3),
    (&[3, 1], 3),
    (&[1], 2),
    (&[7], 1),
    (&[4], 2),
    (&[4], 1),
];

fn hostility_world() -> World {
    use engine_native::{
        cdiplomacy_status, crelation, crelation_status, cunit, cwar_relation, province_definition,
    };
    let mut world = World::new(3);
    world.bytes.resize(0x60000, 0);
    world.header(
        GAME + cgame_state::PROVINCES,
        HOSTILITY_ARRAY,
        HOSTILITY_IDS as usize,
    );
    for id in 1..HOSTILITY_IDS {
        if id == 2 {
            continue;
        }
        let province = HOSTILITY_PROVINCE + id as usize * 0x400;
        let definition = province + 0x200;
        world.u64(HOSTILITY_ARRAY + id as usize * 8, province as u64);
        world.u64(province, cprovince::VTABLE_RVA.0 as u64);
        world.u32(province + cprovince::ID, if id == 5 { 9 } else { id });
        world.u64(province + cprovince::DEFINITION, definition as u64);
        world.u32(definition + province_definition::ID, id);
        world.u32(
            province + cprovince::CONTROLLER,
            [0, 1, 0, 0, 2, 3][id as usize],
        );
    }
    for (index, &(route, logical)) in HOSTILITY_ARMIES.iter().enumerate() {
        let army = HOSTILITY_ARMY + index * 0x400;
        world.header(army + cunit::ORDINARY_ROUTE_DATA, army + 0x300, route.len());
        for (at, &id) in route.iter().enumerate() {
            world.u32(army + 0x300 + at * 4, id);
        }
        world.u32(army + cunit::LOGICAL_OWNER, logical);
    }
    for (index, (owner, target, war)) in [(1, 2, false), (1, 3, true), (2, 1, true)]
        .into_iter()
        .enumerate()
    {
        let country = COUNTRY + owner * BLOCK;
        let diplomacy = 0x50000 + owner * 0x1000;
        let relations = diplomacy + 0x800;
        let tuple = 0x54000 + index * 0x400;
        world.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        world.u64(diplomacy, cdiplomacy_status::VTABLE_RVA.0 as u64);
        world.u64(diplomacy + cdiplomacy_status::COUNTRY, country as u64);
        world.header(diplomacy + cdiplomacy_status::RELATIONS, relations, 4);
        world.u64(relations + target * 8, tuple as u64);
        world.u64(tuple, crelation_status::VTABLE_RVA.0 as u64);
        world.u64(tuple + crelation_status::DIPLOMACY_STATUS, diplomacy as u64);
        world.u32(tuple + crelation_status::TARGET_COUNTRY, target as u32);
        if war {
            let relation = 0x58000 + index * 0x100;
            world.u64(tuple + crelation_status::WAR_RELATION, relation as u64);
            world.u64(relation, cwar_relation::VTABLE_RVA.0 as u64);
            world.u32(relation + crelation::TYPE_ID, cwar_relation::TYPE);
        }
    }
    world
}

fn shared_hostility(
    memory: &impl Memory,
    hostility: &mut Hostility,
) -> Vec<Result<bool, &'static str>> {
    (0..HOSTILITY_ARMIES.len())
        .map(|index| {
            let army = HOSTILITY_ARMY + index * 0x400;
            let (mut arena, mut values) = (Vec::new(), Vec::new());
            let last = Array::read(memory, army + engine_native::cunit::ORDINARY_ROUTE_DATA, 4)
                .ok_or("division route unreadable")
                .and_then(|path| copy_route(memory, path, &mut arena, &mut values))
                .and_then(|(start, len)| arena[start + len - 1]);
            let logical =
                engine_native::read::<u32>(memory, army + engine_native::cunit::LOGICAL_OWNER);
            hostility.attacking(memory, 0, GAME, last, logical)
        })
        .collect()
}

#[test]
fn shared_hostility_matches_each_movers_own_attacking_read() {
    let mut world = hostility_world();
    let alone = |world: &World| -> Vec<_> {
        (0..HOSTILITY_ARMIES.len())
            .map(|index| {
                engine_native::carmy::movement_attacking(
                    world,
                    0,
                    GAME,
                    HOSTILITY_ARMY + index * 0x400,
                )
            })
            .collect()
    };
    let mut hostility = Hostility::default();
    let shared = shared_hostility(&world, &mut hostility);
    assert_eq!(
        shared,
        [
            Ok(false),
            Ok(true),
            Ok(false),
            Err("movement province definition mismatch"),
            Ok(true),
            Ok(false),
            Err("movement route province outside roster"),
            Ok(false),
            Ok(true),
        ]
    );
    assert_eq!(shared, alone(&world));
    assert_eq!((hostility.destinations.len(), hostility.pairs.len()), (4, 4));
    let touched: std::collections::BTreeSet<usize> = world.reads.take().into_iter().collect();
    assert!(touched.len() > HOSTILITY_ARMIES.len() * 4);
    for at in touched {
        world.fault = Some(at);
        let shared = shared_hostility(&world, &mut Hostility::default());
        let alone = alone(&world);
        for index in 0..HOSTILITY_ARMIES.len() {
            assert_eq!(
                shared[index] == Ok(false),
                alone[index] == Ok(false),
                "army {index}, fault at {at:#x}"
            );
        }
    }
}

struct OneShot<'a> {
    world: &'a World,
    fault: usize,
    spent: std::cell::Cell<bool>,
}

impl Memory for OneShot<'_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        if !self.spent.get() && at <= self.fault && self.fault < at.saturating_add(out.len()) {
            self.spent.set(true);
            return false;
        }
        self.world.copy(at, out)
    }
}

#[test]
fn a_one_shot_fault_changes_only_the_mover_that_met_it() {
    let world = hostility_world();
    let clean = shared_hostility(&world, &mut Hostility::default());
    let touched: std::collections::BTreeSet<usize> = world.reads.take().into_iter().collect();
    let mut met = 0;
    for at in touched {
        let memory = OneShot {
            world: &world,
            fault: at,
            spent: Default::default(),
        };
        let faulted = shared_hostility(&memory, &mut Hostility::default());
        let changed = (0..clean.len())
            .filter(|&index| faulted[index] != clean[index])
            .count();
        assert!(changed <= 1, "fault at {at:#x}: {faulted:?}");
        met += changed;
    }
    assert!(met > 0);
}
