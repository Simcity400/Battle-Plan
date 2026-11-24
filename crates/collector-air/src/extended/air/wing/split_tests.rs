use super::*;
use crate::unit_fixture as fixture;
use collector_observers::wing_changes::Watches;
use engine_native_air::{air_mission, cstrategic_air, cstrategic_air_manager};
use engine_native_parts::cair_wing_pool;
use engine_native::cgame_state;
use engine_native_memory::cpdx_array::Array;

const COUNTRIES: u32 = 5_000;
const WINGS: usize = 12_000;

fn world() -> fixture::World {
    fixture::build(fixture::Shape {
        countries: COUNTRIES as usize,
        divisions: 0,
        wings: WINGS,
        planes: WINGS,
        ships: 0,
    })
}

#[derive(Clone, Copy)]
struct Config {
    hour: i32,
    force_full: bool,
    game_score: bool,
}

const FULL: Config = Config {
    hour: 0,
    force_full: true,
    game_score: true,
};

fn quiet() -> Config {
    let hour = (0..24)
        .find(|&hour| {
            !replay_data::cadence::should_walk(TableId::WingCumulative, hour, false)
                && !replay_data::cadence::should_walk(TableId::AirTheatreMembers, hour, false)
        })
        .unwrap();
    Config {
        hour,
        force_full: false,
        game_score: false,
    }
}

struct Jobs {
    census: AirWingJob,
    parts: Vec<AirWingJob>,
    watches: &'static Watches,
}

impl Jobs {
    fn new() -> Self {
        let watches: &'static Watches = Box::leak(Box::new(Watches::enabled(1 << 15)));
        let job = || AirWingJob {
            carry_changes: CarryChanges(watches),
            ..AirWingJob::part()
        };
        Self {
            census: job(),
            parts: (1..WING_PARTS).map(|_| job()).collect(),
            watches,
        }
    }

    fn hour(&mut self, world: &fixture::World, config: Config, starts: &[u32]) -> &AirWingJob {
        self.walk(world, config, starts);
        if self.census.split {
            self.census
                .join_parts(&mut self.parts[..starts.len()])
                .unwrap();
        }
        self.watches.boundary(false);
        &self.census
    }

    fn walk(&mut self, world: &fixture::World, config: Config, starts: &[u32]) {
        let ctx = AirWingCopyContext {
            base: world.base,
            game_state: world.state,
            hour: config.hour,
            force_full: config.force_full,
            limits: [0; 3],
            game_score: config.game_score,
        };
        let end = |index: usize| starts.get(index).copied().unwrap_or(u32::MAX);
        self.census.slots = 0..end(0);
        self.census.split = !starts.is_empty();
        let active = &mut self.parts[..starts.len()];
        for (index, part) in active.iter_mut().enumerate() {
            part.slots = starts[index]..end(index + 1);
            part.split = true;
        }
        std::thread::scope(|scope| {
            for job in std::iter::once(&mut self.census).chain(active.iter_mut()) {
                scope.spawn(move || {
                    assert!(copy_air_refresh(&world.live(), ctx, job).raw_ok);
                });
            }
        });
        for job in std::iter::once(&self.census).chain(&self.parts[..starts.len()]) {
            assert_ne!(job.identity_outcome, NO_WING_SOURCE);
            assert_ne!(job.naval_membership_outcome, NO_WING_SOURCE);
        }
    }
}

fn certified(job: &AirWingJob) -> u64 {
    job.carry.0.reused
}

fn walk_serial(world: &fixture::World, config: Config) -> AirWingJob {
    let mut jobs = Jobs::new();
    jobs.hour(world, config, &[]);
    jobs.census
}

fn assert_same(split: &AirWingJob, serial: &AirWingJob) {
    let outcomes = |job: &AirWingJob| {
        [
            job.map_outcome,
            job.source_outcome,
            job.naval_outcome,
            job.equipment_outcome,
            job.cumulative_outcome,
            job.identity_outcome,
            job.naval_membership_outcome,
            job.census.score,
        ]
    };
    assert_eq!(outcomes(split), outcomes(serial));
    let map = serial.map_outcome.len();
    assert_eq!(split.map_rows[..map], serial.map_rows[..map]);
    assert_eq!(
        split.names[..split.names_len],
        serial.names[..serial.names_len]
    );
    assert_eq!(
        (split.names_required, split.name_failures),
        (serial.names_required, serial.name_failures)
    );
    assert_eq!(split.name_failure_reasons, serial.name_failure_reasons);
    let identities = serial.identity_outcome.len();
    assert_eq!(
        split.identities[..identities],
        serial.identities[..identities]
    );
    let equipment = serial.equipment_outcome.len();
    assert_eq!(split.equipment[..equipment], serial.equipment[..equipment]);
    let cumulative = serial.cumulative_outcome.len();
    assert_eq!(
        split.cumulative[..cumulative],
        serial.cumulative[..cumulative]
    );
    let naval = serial.naval_membership_outcome.len();
    assert_eq!(
        split.naval_memberships[..naval],
        serial.naval_memberships[..naval]
    );
    assert_eq!(split.transport.failures, serial.transport.failures);
    assert_eq!(
        (split.required, split.required_memberships),
        (serial.required, serial.required_memberships)
    );
    assert_eq!(split.game_score.witnessed(), serial.game_score.witnessed());
}

fn pointer(memory: &impl Memory, address: usize) -> usize {
    engine_native_memory::read::<u64>(memory, address).unwrap() as usize
}

fn array(memory: &impl Memory, header: usize) -> Array {
    Array::read(memory, header, 8).unwrap()
}

fn element(memory: &impl Memory, header: usize, index: u32) -> usize {
    pointer(memory, array(memory, header).element(index, 8).unwrap())
}

fn manager(world: &fixture::World) -> usize {
    pointer(
        &world.live(),
        world.state + cgame_state::STRATEGIC_AIR_MANAGER,
    )
}

fn owner(world: &fixture::World, slot: u32) -> usize {
    element(
        &world.live(),
        manager(world) + cstrategic_air_manager::COUNTRY_AIR,
        slot,
    )
}

fn pool(world: &fixture::World, slot: u32) -> usize {
    element(&world.live(), owner(world, slot) + cstrategic_air::POOLS, 0)
}

fn put(world: &mut fixture::World, address: usize, value: u64) -> u64 {
    let at = address - world.base;
    let old = u64::from_le_bytes(world.bytes[at..at + 8].try_into().unwrap());
    world.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    old
}

fn first_sites(world: &mut fixture::World, slots: &[u32]) {
    let bases = array(
        &world.live(),
        manager(world) + cstrategic_air_manager::STATELESS_AIR_BASES,
    );
    for &slot in slots {
        assert!(slot < bases.count);
        let header = owner(world, slot) + cstrategic_air::FIRST_AIR_SITES;
        let data = bases.element(slot - 1, 8).unwrap();
        put(world, header, data as u64);
        put(world, header + 8, 2 << 32 | 2);
    }
}

#[test]
fn parts_kept_across_hours_carry_and_join_whatever_the_split() {
    let world = world();
    let full = walk_serial(&world, FULL);
    let ordinary = walk_serial(&world, quiet());
    let mut jobs = Jobs::new();
    let wings = WINGS as u64;
    let quarters = [1_250, 2_500, 3_750];
    let some = |certified: u64| certified > 0 && certified < wings;
    for (config, starts, reference, expected) in [
        (FULL, &quarters[..], &full, Some(0)),
        (quiet(), &quarters[..], &ordinary, Some(wings)),
        (FULL, &[][..], &full, None),
        (FULL, &quarters[..], &full, Some(wings)),
        (quiet(), &[2_000][..], &ordinary, None),
    ] {
        let joined = jobs.hour(&world, config, starts);
        assert_same(joined, reference);
        if cfg!(feature = "wing_changes") {
            let certified = certified(joined);
            match expected {
                Some(expected) => assert_eq!(certified, expected, "{starts:?}"),
                None => assert!(some(certified), "{starts:?} {certified}"),
            }
        }
    }
}

#[test]
fn a_join_refused_after_partial_growth_forgets_every_store() {
    use collector_primitives::raw_memory::{Charge, Tracker};
    static LOCAL: Tracker = Tracker::new();
    let world = world();
    let quarters = [1_250, 2_500, 3_750];
    let mut jobs = Jobs::new();
    jobs.hour(&world, FULL, &quarters);
    jobs.census.capacity_charge = Charge::empty_on(&LOCAL);
    jobs.census.names = Vec::new();
    jobs.census.map_rows = Vec::new();
    jobs.walk(&world, FULL, &quarters);
    JOIN_GROWTHS.with(|left| left.set(Some(1)));
    assert_eq!(
        jobs.census.join_parts(&mut jobs.parts[..quarters.len()]),
        Err("capture buffer allocation failed")
    );
    JOIN_GROWTHS.with(|left| left.set(None));
    let charged = LOCAL.snapshot().current;
    jobs.census.update_capacity_charge();
    assert_eq!(LOCAL.snapshot().current, charged);
    let held = retained_store_bytes(
        jobs.census.wing_store.as_deref_mut(),
        &mut jobs.parts,
        false,
    );
    let retained =
        retained_store_bytes(jobs.census.wing_store.as_deref_mut(), &mut jobs.parts, true);
    assert_eq!(retained, held);
    jobs.watches.boundary(false);
    let joined = jobs.hour(&world, FULL, &quarters);
    assert_same(joined, &walk_serial(&world, FULL));
    if cfg!(feature = "wing_changes") {
        assert!(held > 0);
        assert_eq!(certified(joined), 0);
    }
}

#[test]
fn the_join_charges_the_storage_it_grew() {
    use collector_primitives::raw_memory::{Charge, Tracker};
    static LOCAL: Tracker = Tracker::new();
    let world = world();
    let mut jobs = Jobs::new();
    jobs.census.capacity_charge = Charge::empty_on(&LOCAL);
    jobs.hour(&world, FULL, &[2_500]);
    let joined = LOCAL.snapshot().current;
    jobs.census.update_capacity_charge();
    assert_eq!(LOCAL.snapshot().current, joined);
    assert!(jobs.census.map_rows.len() >= WINGS);
}

#[test]
fn a_fault_in_any_part_refuses_what_the_serial_walk_refuses() {
    let mut world = world();
    first_sites(&mut world, &[1, 1_249, 1_250, 2_500, 4_998]);
    let starts = [1_250, 2_500, 3_750];
    for config in [FULL, quiet()] {
        for slot in [1, 1_250, 2_600, COUNTRIES] {
            let pool = pool(&world, slot);
            let vtable = put(&mut world, pool, 0);
            let serial = walk_serial(&world, config);
            assert_eq!(
                serial.map_outcome,
                Outcome::Unavailable {
                    reason: "air wing pool vtable mismatch"
                }
            );
            assert_same(Jobs::new().hour(&world, config, &starts), &serial);
            put(&mut world, pool, vtable);
            let wing = element(&world.live(), pool + cair_wing_pool::WINGS, 0);
            let region = wing + cair_wing::MISSION + air_mission::REGION;
            let unset = put(&mut world, region, wing as u64);
            let serial = walk_serial(&world, config);
            assert_eq!(
                serial.map_outcome,
                Outcome::Unavailable {
                    reason: "air wing mission region vtable mismatch"
                }
            );
            assert_eq!(serial.census.ended, !config.game_score);
            assert_same(Jobs::new().hour(&world, config, &starts), &serial);
            put(&mut world, region, unset);
        }
    }
    let wing = |world: &fixture::World, slot| {
        element(&world.live(), pool(world, slot) + cair_wing_pool::WINGS, 0)
    };
    let equipment = wing(&world, 1_300) + cair_wing::EQUIPMENT_POOL;
    let name = wing(&world, 2_600) + cair_wing::NAME + engine_native_memory::msvc_string::LENGTH;
    let region = wing(&world, 1_300) + cair_wing::MISSION + air_mission::REGION;
    let later_pool = pool(&world, 3_800);
    for faults in [
        &[(equipment, 0)][..],
        &[(name, 16)][..],
        &[(equipment, 0), (name, 16)][..],
        &[(region, region as u64), (later_pool, 0)][..],
    ] {
        let kept: Vec<_> = faults
            .iter()
            .map(|&(address, value)| (address, put(&mut world, address, value)))
            .collect();
        for config in [FULL, quiet()] {
            let serial = walk_serial(&world, config);
            assert_same(Jobs::new().hour(&world, config, &starts), &serial);
        }
        for (address, value) in kept {
            put(&mut world, address, value);
        }
    }
    let serial = walk_serial(&world, FULL);
    assert_eq!(serial.map_outcome, Outcome::Rows { len: WINGS });
    put(&mut world, equipment, 0);
    put(&mut world, name, 16);
    let serial = walk_serial(&world, FULL);
    assert!(matches!(
        serial.equipment_outcome,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(serial.name_failures, 1);
    assert_eq!(serial.map_outcome, Outcome::Rows { len: WINGS });
    let owners = manager(&world) + cstrategic_air_manager::COUNTRY_AIR + 8;
    let header = put(&mut world, owners, (u64::from(COUNTRIES) + 1) << 32 | 1);
    let serial = walk_serial(&world, FULL);
    assert_eq!(
        serial.map_outcome,
        Outcome::Unavailable {
            reason: "air wing roster unreadable or invalid"
        }
    );
    assert_same(Jobs::new().hour(&world, FULL, &starts), &serial);
    put(&mut world, owners, header);
    let manager = world.state + cgame_state::STRATEGIC_AIR_MANAGER;
    put(&mut world, manager, 0);
    let serial = walk_serial(&world, FULL);
    assert_same(Jobs::new().hour(&world, FULL, &starts), &serial);
}

#[test]
fn parts_start_at_the_owners_holding_each_share_and_never_split_an_owner() {
    let mut census = AirWingJob {
        map_outcome: Outcome::Rows { len: 997 },
        owner_costs: vec![(1, 0), (2, 1), (3, 900)],
        cost: 1_000,
        ..Default::default()
    };
    let starts = |census: &AirWingJob, parts: &[AirWingJob]| {
        part_starts(census, parts).map(|(starts, split)| starts[..split].to_vec())
    };
    assert_eq!(starts(&census, &[]), Some(vec![2]));
    let part = AirWingJob {
        owner_costs: vec![(4, 0), (5, 500)],
        cost: 1_000,
        ..Default::default()
    };
    assert_eq!(starts(&census, &[part]), Some(vec![2, 4, 5]));
    census.cost = 300;
    assert_eq!(starts(&census, &[]), Some(vec![]));
    census.map_outcome = NO_WING_SOURCE;
    assert_eq!(starts(&census, &[]), None);
}
