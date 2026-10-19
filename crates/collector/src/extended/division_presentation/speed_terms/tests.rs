use super::*;
use engine_native::{cgame_state, memory::SliceMemory};
use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, Ordering},
};

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
            // River lookup reads the final country's nullable diplomacy pointer,
            // which lies beyond this fixture's tightly packed counter blocks.
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
            world.u64(country, ccountry::VTABLE_RVA as u64);
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
struct ChangesForTest {
    every: AtomicBool,
    poisoned: AtomicBool,
}
impl Changes for ChangesForTest {
    fn take(&self, _: Table, _: &mut dyn FnMut(usize)) -> Option<u64> {
        Some(1)
    }
    fn mismatch(&self, _: Table, _: Mismatch) {
        self.poisoned.store(true, Ordering::Relaxed);
    }
    fn every(&self, _: Table) -> bool {
        self.every.load(Ordering::Relaxed)
    }
}
fn changes() -> &'static ChangesForTest {
    Box::leak(Box::new(ChangesForTest {
        every: AtomicBool::new(false),
        poisoned: AtomicBool::new(false),
    }))
}
fn movers(capture: &mut SpeedTerms, countries: usize) {
    capture.keys = vec![SPEED_KEY, SPEED_KEY, SPEED_KEY + 1];
    capture.templates = vec![(1, Ok((0, 3)))];
    capture.movers = (1..=countries)
        .map(|owner| Mover {
            division: owner as u64,
            pointer: 0,
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
    // A changed epoch never certifies the old read.
    world.value(1, 30_000);
    b.capture_holding(&store, |c| entries(c, &world, vec![], 2));
    assert_eq!(b.counts[2], [4, 0, 0]);
    assert_eq!(b.entries[0].entry, Some(Some(30_000)));
}

#[test]
fn unit_speed_verification_publishes_fresh_entries_and_poison_on_a_missed_write() {
    let mut world = World::new(2);
    let changes = changes();
    let mut capture = SpeedTerms {
        changes,
        ..SpeedTerms::default()
    };
    movers(&mut capture, 2);
    entries(&mut capture, &world, vec![], 1);
    world.value(1, -70_000);
    changes.every.store(true, Ordering::Relaxed);
    entries(&mut capture, &world, vec![], 1);
    assert!(changes.poisoned.load(Ordering::Relaxed));
    assert_eq!(capture.entries[0].entry, Some(Some(-70_000)));
    assert_eq!(capture.counts[2], [4, 0, 0]);
}

#[test]
fn unit_speed_entries_are_shared_by_key_for_five_thousand_countries() {
    let world = World::new(5_000);
    let mut capture = SpeedTerms {
        changes: changes(),
        ..SpeedTerms::default()
    };
    movers(&mut capture, 5_000);
    entries(&mut capture, &world, vec![], 1);
    assert_eq!(capture.entries.len(), 10_000);
    entries(&mut capture, &world, vec![], 1);
    assert_eq!(capture.counts[2], [0, 10_000, 0]);
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
    world.u64(COMBAT, combat::VTABLE_RVA as u64);
    world.u64(COMBAT + combat::MOVEMENT_MODIFIER, (-13_000_i64) as u64);
    world.u64(COMBAT + combat::ATTACKER, SIDE as u64);
    world.u64(SIDE, side::VTABLE_RVA as u64);
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
            // Even unreadable combat fields must not be read outside the branch.
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
    assert_eq!(capture.counts[0][2], 1); // Only the land mover's missing province.
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
        capture.capture_rivers(&world, 0, GAME, &countries),
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
    let result = crate::test_support::with_allocations_denied(|| {
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
        capture.capture_rivers(&world, 0, GAME, &countries),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(capture.counts[..4], [[0; 3]; 4]);
}

/// One synthetic attacking combat with a 0.2 movement word.
fn attacking_mover(world: &mut World) -> DivisionSeed {
    use engine_native::{cland_combat as combat, cland_combatant as side};
    const COMBAT: usize = 0x22000;
    const SIDE: usize = 0x22400;
    const LIST: usize = 0x22800;
    const ATTACKERS: usize = 0x22900;
    const DIVISION: usize = 0x22a00;
    world.u64(COMBAT, combat::VTABLE_RVA as u64);
    world.u64(COMBAT + combat::MOVEMENT_MODIFIER, 20_000);
    world.u64(COMBAT + combat::ATTACKER, SIDE as u64);
    world.u64(SIDE, side::VTABLE_RVA as u64);
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
    ); // No location state.
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
    assert!(capture.counts[3][2] >= 1); // The later edge is counted independently.
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
    // The reported transfer A -> B -> C case: C's facts refuse, while A
    // and B already establish two source-region demands. Exercise the caller too.
    capture.movers[0].route = Ok((0, 2));
    capture.route = vec![Ok(2), Ok(3)];
    capture.provinces[2].1 = Err("later province facts refused");
    assert_eq!(
        capture.capture_transport(&world, 0, GAME),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(capture.transport_demand, [(1, 16), (1, 32)]);
    // These synthetic region pointers have no authenticated region identity.
    // Their local refusals cannot remove the established source demand.
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
        assert_eq!(capture.rivers.len(), needed); // Shared owner/controller is not duplicated.
        if needed != 0 {
            assert_eq!(capture.rivers[0].entry, Some(None));
        }
        assert_eq!(capture.counts[3][2], 2); // One later-edge refusal per mover.
    }
}

#[test]
fn river_and_combat_skip_unselected_branches() {
    let mut world = World::new(8);
    let seed = attacking_mover(&mut world);
    // A second, unreadable combat must never be demanded after the first match.
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
    // A defending division reads side membership, but not the combat movement word.
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
    assert_eq!(demand, [(1, 16)]); // Later land/land edge still refuses its unknown branch.
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
    assert_eq!(capture.rivers[0].entry, Some(None)); // Readable null diplomacy, not another refusal.
    assert_eq!(capture.counts[3][2], 2);
    // A failed common gate is counted during preparation, without another
    // mover census or an additional charge when river acquisition skips it.
    capture.counts[3][2] = 0;
    capture.movers[0].location = Err("location refused");
    capture.prepare_land_demand(&world, 0, GAME).unwrap();
    assert_eq!(capture.counts[3][2], 1);
    assert_eq!(
        capture.capture_rivers(&world, 0, GAME, &countries),
        Outcome::Rows { len: 0 }
    );
    assert_eq!(capture.counts[3][2], 1);
}

#[test]
fn country_entry_scratch_is_reused_and_one_failed_key_keeps_healthy_entries() {
    let mut world = World::new(5_000);
    let mut capture = SpeedTerms::default();
    movers(&mut capture, 5_000);
    let owners: Vec<_> = (1..=5_000).collect();
    entries(&mut capture, &world, owners.clone(), 1);
    let keys = capture.entry_keys.as_ptr();
    let values = capture.entry_values.as_ptr();
    entries(&mut capture, &world, owners, 1);
    assert_eq!(capture.entry_keys.as_ptr(), keys);
    assert_eq!(capture.entry_values.as_ptr(), values);
    // The selected key's value fails; another key's known absence stays usable.
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
