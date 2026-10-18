//! The AirWing census walked in owner-slot parts and joined as the callback
//! joins them, against the serial walk of the same world.
use super::*;
use crate::{unit_scale_tests::fixture, wing_changes::Watches};
use engine_native::{
    air_mission, cair_wing_pool, cgame_state, cpdx_array::Array, cstrategic_air,
    cstrategic_air_manager,
};

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

/// Every table walked, with the score inputs.
const FULL: Config = Config {
    hour: 0,
    force_full: true,
    game_score: true,
};

/// An ordinary hour that walks neither WingCumulative nor the theatre
/// memberships, without the score inputs.
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

/// The first job and its parts, kept across hours as the callback keeps them,
/// with a watch domain of their own that certifies their stores' carry.
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

    /// One hour's census in parts from each of `starts`, lent as the
    /// callback lends them (later parts grow their own storage), each on its
    /// own thread as the pool's workers run them, and joined.
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

    /// [`Self::hour`]'s parts, before the join.
    fn walk(&mut self, world: &fixture::World, config: Config, starts: &[u32]) {
        let ctx = AirWingCopyContext {
            base: world.base,
            game_state: world.state,
            hour: config.hour,
            force_full: config.force_full,
            limits: [0; 3],
            naval_limit: 0,
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
                    assert!(copy_air_refresh_growing(&world.live(), ctx, job).raw_ok);
                });
            }
        });
        // Each part built its own identities and memberships on its worker.
        for job in std::iter::once(&self.census).chain(&self.parts[..starts.len()]) {
            assert_ne!(job.identity_outcome, NO_WING_SOURCE);
            assert_ne!(job.naval_membership_outcome, NO_WING_SOURCE);
        }
    }
}

/// Wings whose held rows a certificate covered: carried, or read and
/// compared.
fn certified(job: &AirWingJob) -> u64 {
    assert_eq!(job.carry.0.mismatches, 0);
    job.carry.0.reused + job.carry.0.verified
}

/// One serial census of `world`.
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
    engine_native::read::<u64>(memory, address).unwrap() as usize
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

/// `slot`'s strategic-air owner.
fn owner(world: &fixture::World, slot: u32) -> usize {
    element(
        &world.live(),
        manager(world) + cstrategic_air_manager::COUNTRY_AIR,
        slot,
    )
}

/// The only pool of `slot`'s owner.
fn pool(world: &fixture::World, slot: u32) -> usize {
    element(&world.live(), owner(world, slot) + cstrategic_air::POOLS, 0)
}

/// Writes `value` at `address` in the world, returning what was there.
fn put(world: &mut fixture::World, address: usize, value: u64) -> u64 {
    let at = address - world.base;
    let old = u64::from_le_bytes(world.bytes[at..at + 8].try_into().unwrap());
    world.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    old
}

/// Gives each of `slots` two first air sites, the stateless bases of its
/// own and the next slot, so owners on both sides of a part boundary index
/// nonempty runs of the joined site arena.
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
fn owner_slot_parts_join_to_the_serial_census_at_5000_countries() {
    let mut world = world();
    first_sites(&mut world, &[1, 1_249, 1_250, 2_500, 4_998]);
    for config in [FULL, quiet()] {
        let serial = walk_serial(&world, config);
        let complete = Outcome::Rows { len: WINGS };
        assert_eq!(
            [serial.map_outcome, serial.equipment_outcome],
            [complete; 2]
        );
        for parts in 2..=4 {
            let starts: Vec<_> = (1..parts).map(|part| COUNTRIES * part / parts).collect();
            let mut jobs = Jobs::new();
            assert_same(jobs.hour(&world, config, &starts), &serial);
            assert_eq!(jobs.census.carry, serial.carry);
            assert_eq!(
                part_starts(&jobs.census, &jobs.parts[..starts.len()]),
                part_starts(&serial, &[])
            );
        }
    }
    // The score inputs index each owner's own first sites in the joined arena.
    let mut jobs = Jobs::new();
    let witnessed = jobs.hour(&world, FULL, &[1_250, 2_500]);
    let witnessed = witnessed.game_score.witnessed();
    for slot in [1_249_u64, 1_250, 2_500] {
        let (read, sites) = witnessed.countries[slot as usize];
        assert_eq!(read.len, 2, "slot {slot}");
        assert_eq!(
            sites.iter().map(|site| site.base).collect::<Vec<_>>(),
            [slot, slot + 1].map(|id| id << 32 | engine_native::cair_base::TYPE as u64),
            "slot {slot}"
        );
    }
    // The next hour's parts follow this census's wings and owners, within
    // the pool's workers.
    let serial = walk_serial(&world, FULL);
    let (starts, split) = part_starts(&serial, &[]).unwrap();
    assert_eq!(split, WING_PARTS - 1);
    assert!(starts[..split].windows(2).all(|pair| pair[0] < pair[1]));
    assert_same(Jobs::new().hour(&world, FULL, &starts[..split]), &serial);
}

#[test]
fn parts_kept_across_hours_carry_and_join_whatever_the_split() {
    let world = world();
    let full = walk_serial(&world, FULL);
    let ordinary = walk_serial(&world, quiet());
    let mut jobs = Jobs::new();
    let wings = WINGS as u64;
    let quarters = [1_250, 2_500, 3_750];
    // The split repeats, shrinks to none, leaves parts idle and grows again,
    // then moves; every part reuses the storage an earlier, different range
    // left in it. Its stores certify every wing whose part held it.
    let some = |certified: u64| certified > 0 && certified < wings;
    for (config, starts, reference, expected) in [
        (FULL, &quarters[..], &full, Some(0)),
        (quiet(), &quarters[..], &ordinary, Some(wings)),
        // The first part's store held only its own quarter.
        (FULL, &[][..], &full, None),
        // The idle parts' stores still hold their quarters.
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
fn idle_parts_keep_their_stores_in_the_retained_bytes() {
    let world = world();
    let mut jobs = Jobs::new();
    jobs.hour(&world, FULL, &[1_250, 2_500, 3_750]);
    // The split shrinks to none: the parts go idle with their held rows.
    jobs.hour(&world, FULL, &[]);
    let census = retained_store_bytes(jobs.census.wing_store.as_deref_mut(), &mut [], false);
    let retained = retained_store_bytes(
        jobs.census.wing_store.as_deref_mut(),
        &mut jobs.parts,
        false,
    );
    if !cfg!(feature = "wing_changes") {
        return;
    }
    assert!(retained > census, "{retained} {census}");
    // A failed acquisition forgets every store, idle ones too, which keep
    // their allocations; the next census certifies nothing.
    assert_eq!(
        retained_store_bytes(jobs.census.wing_store.as_deref_mut(), &mut jobs.parts, true),
        retained
    );
    assert_eq!(
        certified(jobs.hour(&world, FULL, &[1_250, 2_500, 3_750])),
        0
    );
}

/// A join refused after it grew part of the census, as a refused
/// allocation is: it refuses with the growth's own reason, as the serial
/// walk's growth would; its growth stays charged; the callback forgets every
/// store, which keeps its bytes; and the next hour certifies nothing.
#[test]
fn a_join_refused_after_partial_growth_forgets_every_store() {
    use crate::raw_memory::{Charge, Tracker};
    static LOCAL: Tracker = Tracker::new();
    let world = world();
    let quarters = [1_250, 2_500, 3_750];
    let mut jobs = Jobs::new();
    jobs.hour(&world, FULL, &quarters);
    // A first job whose storage holds only its own share, as a cold pooled
    // job's does: the join grows its names, then its rows.
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
    use crate::raw_memory::{Charge, Tracker};
    static LOCAL: Tracker = Tracker::new();
    let world = world();
    let mut jobs = Jobs::new();
    jobs.census.capacity_charge = Charge::empty_on(&LOCAL);
    // The first part's worker charged only its own share; the join grew
    // its vectors to the whole census.
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
            // A roster fault ends the walk at its owner: nothing after it
            // joins, and the score inputs witness no later owner.
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
            // A wing's mission region that does not authenticate refuses
            // the census rows. With the score inputs the serial walk goes on
            // with their leaves alone; without them it ends there.
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
    // Faults of the dependent tables and the names leave the census rows,
    // and the first of two census faults in slot order leads.
    let wing = |world: &fixture::World, slot| {
        element(&world.live(), pool(world, slot) + cair_wing_pool::WINGS, 0)
    };
    let equipment = wing(&world, 1_300) + cair_wing::EQUIPMENT_POOL;
    let name = wing(&world, 2_600) + cair_wing::NAME + engine_native::msvc_string::LENGTH;
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
    // A roster header that does not read ends every part at once.
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
    // Without a manager only the first part witnesses the owners, every one
    // of them as the serial walk does.
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
    // Both shares' first units fall in the second owner's wings.
    assert_eq!(starts(&census, &[]), Some(vec![2]));
    // A later part's owners follow this one's units.
    let part = AirWingJob {
        owner_costs: vec![(4, 0), (5, 500)],
        cost: 1_000,
        ..Default::default()
    };
    assert_eq!(starts(&census, &[part]), Some(vec![2, 4, 5]));
    // Too little for two parts walks in one.
    census.cost = 300;
    assert_eq!(starts(&census, &[]), Some(vec![]));
    // An incomplete census keeps the current split.
    census.map_outcome = NO_WING_SOURCE;
    assert_eq!(starts(&census, &[]), None);
}
