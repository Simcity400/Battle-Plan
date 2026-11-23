use super::*;
use crate::ccountry::{CounterInputsRow, synthetic::World};

fn walk_counter_inputs_with_rosters(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<CounterInputsRow>,
    rosters: &mut CounterRosters<'_>,
) -> Outcome {
    crate::ccountry::walk_counter_inputs_inner(
        memory,
        base,
        game_state,
        Buffer::Growing(output),
        Some(rosters),
        None,
        None,
        None,
    )
}
use engine_native_memory::read;
use engine_native_memory::walk::Outcome;

fn fixture(countries: u32) -> (World, Vec<usize>) {
    let mut world = World::new(0x4000_0000, countries, 0x1100);
    let mut leader_objects = Vec::new();
    for country in 1..=countries {
        let owner = world.object(country);
        world.u64(
            owner + crate::ccountry::PRIDE_OF_THE_FLEET,
            (u64::from(country) << 32) | 51,
        );
        let division = world.alloc(crate::cunit::HANDLE + 8);
        let divisions = world.alloc(8);
        world.u64(division, (world.module + crate::carmy::VTABLE_RVA.0) as u64);
        world.u64(
            division + crate::cunit::HANDLE,
            (u64::from(country) << 32) | 4,
        );
        world.u64(divisions, division as u64);
        world.array(owner + crate::ccountry::DIVISIONS, divisions, 1);
        let first = world.alloc(cstate::ID + 4);
        let second = world.alloc(cstate::ID + 4);
        world.u32(first + cstate::ID, country + 10_000);
        world.u32(second + cstate::ID, country);
        let state_list = world.alloc(24);
        world.u64(state_list, first as u64);
        world.u64(state_list + 8, 0);
        world.u64(state_list + 16, second as u64);
        for offset in [OWNED_STATES, CONTROLLED_STATES, CORE_STATES] {
            world.array(owner + offset, state_list, 3);
        }
        let characters = world.alloc(0xa0);
        world.u64(owner + crate::ccountry::CHARACTERS, characters as u64);
        world.u64(
            characters,
            (world.module + VTABLE_RVA.0) as u64,
        );
        world.u64(characters + OWNER, owner as u64);
        let leader = world.alloc(0x10);
        world.u64(leader, (world.module + cunit_leader::VTABLE_RVA.0) as u64);
        world.u64(
            leader + cunit_leader::TYPE,
            (u64::from(country) << 32) | 4713,
        );
        let leaders = world.alloc(24);
        world.u64(leaders, leader as u64);
        world.u64(leaders + 8, 0);
        world.u64(leaders + 16, leader as u64);
        for offset in LEADERS {
            world.array(characters + offset, leaders, 3);
        }
        leader_objects.push(leader);
    }
    (world, leader_objects)
}

#[test]
fn effect_rosters_preserve_order_and_fail_independently() {
    let (mut world, _) = fixture(4);
    for country in 1..=4 {
        let owner = world.object(country);
        let neighbors = world.alloc(12);
        world.array(owner + NEIGHBORS, neighbors, 3);
        for (index, raw) in [u32::MAX, country, country].into_iter().enumerate() {
            world.u32(neighbors + index * 4, raw);
        }
        let status = world.alloc(cproduction_status::COUNTRY + 8);
        let roster = world.alloc(16);
        let policy = world.alloc(12);
        world.u64(owner + crate::ccountry::PRODUCTION_STATUS, status as u64);
        world.u64(
            status,
            (world.module + cproduction_status::VTABLE_RVA.0) as u64,
        );
        world.u64(status + cproduction_status::COUNTRY, owner as u64);
        world.array(status + cproduction_status::POLICIES, roster, 2);
        world.u64(roster, policy as u64);
        world.u64(roster + 8, policy as u64);
        world.u64(
            policy,
            (world.module + cproduction_status::POLICY_VTABLE_RVA.0) as u64,
        );
        world.u32(policy + cproduction_status::POLICY_TOKEN, country + 80_000);
    }
    assert_eq!(
        counter_effect_roster_demand(&world.memory(), world.module, world.game_state()),
        Ok(EffectRosterDemand {
            neighbors: 12,
            policies: 8
        })
    );
    let mut rows = vec![CounterInputsRow::default(); 4];
    let mut states = vec![0; 36];
    let mut leaders = vec![0; 24];
    let mut neighbors = vec![0; 12];
    let mut policies = vec![0; 8];
    let mut rosters = CounterRosters {
        capture_sent_expeditionaries: true,

        states: Buffer::Growing(&mut states),
        leaders: Buffer::Growing(&mut leaders),
        state_len: 0,
        leader_len: 0,
        required: Default::default(),
    };
    let mut effects = EffectRosters {
        neighbors: Buffer::Growing(&mut neighbors),
        policies: Buffer::Growing(&mut policies),
        neighbor_len: 0,
        policy_len: 0,
        required: Default::default(),
    };
    assert_eq!(
        crate::ccountry::walk_counter_inputs_inner(&world.memory(), world.module, world.game_state(), engine_native_memory::walk::Buffer::Growing(&mut rows), Some(&mut rosters), Some(&mut effects), None, None),
        Outcome::Rows { len: 4 }
    );
    for row in &rows {
        let span = row.neighbors.unwrap();
        assert_eq!(
            &effects.neighbors[span.start..span.start + span.count],
            &[u32::MAX, row.country, row.country]
        );
        let span = row.mio_policies.unwrap();
        assert_eq!(
            &effects.policies[span.start..span.start + span.count],
            &[row.country + 80_000; 2]
        );
    }
    effects.neighbors = Buffer::Count;
    effects.neighbor_len = 0;
    effects.policy_len = 0;
    effects.required = Default::default();
    rosters.state_len = 0;
    rosters.leader_len = 0;
    assert_eq!(
        crate::ccountry::walk_counter_inputs_inner(&world.memory(), world.module, world.game_state(), engine_native_memory::walk::Buffer::Growing(&mut rows), Some(&mut rosters), Some(&mut effects), None, None),
        Outcome::Rows { len: 4 }
    );
    assert_eq!(effects.required.neighbors, 12);
    assert!(rows.iter().all(|row| row.neighbors.is_none()
        && row.neighbor_reason == Some("country neighbor roster arena capacity exceeded")
        && row.mio_policies.is_some()
        && row.state_rosters.is_some()));
    world.u64(world.object(1) + crate::ccountry::PRODUCTION_STATUS, 0);
    effects.policy_len = 0;
    rosters.state_len = 0;
    rosters.leader_len = 0;
    assert_eq!(
        crate::ccountry::walk_counter_inputs_inner(&world.memory(), world.module, world.game_state(), engine_native_memory::walk::Buffer::Growing(&mut rows), Some(&mut rosters), Some(&mut effects), None, None),
        Outcome::Rows { len: 4 }
    );
    assert_eq!(
        rows[0].policy_reason,
        Some("MIO policy production status is null")
    );
    assert!(rows[0].mio_policies.is_none());
    assert!(rows[0].state_rosters.is_some());
    assert!(rows[1..].iter().all(|row| row.mio_policies.is_some()));
}

#[test]
fn five_native_order_rosters_cover_every_country_without_fabricating_handles() {
    let (world, _) = fixture(12);
    assert_eq!(
        counter_roster_demand(&world.memory(), world.module, world.game_state()),
        Ok(RosterDemand {
            states: 108,
            leaders: 72,
        })
    );
    let mut rows = vec![CounterInputsRow::default(); 12];
    let mut states = vec![0; 108];
    let mut leaders = vec![0; 72];
    let mut rosters = CounterRosters {
        capture_sent_expeditionaries: true,

        states: Buffer::Growing(&mut states),
        leaders: Buffer::Growing(&mut leaders),
        state_len: 0,
        leader_len: 0,
        required: Default::default(),
    };
    assert_eq!(
        walk_counter_inputs_with_rosters(
            &world.memory(),
            world.module,
            world.game_state(),
            &mut rows,
            &mut rosters,
        ),
        Outcome::Rows { len: 12 }
    );
    assert_eq!(
        rosters.required,
        RosterDemand {
            states: 108,
            leaders: 72,
        }
    );
    for row in &rows {
        assert_eq!((row.owned_state_count, row.controlled_state_count), (3, 3));
        for span in row.state_rosters.unwrap() {
            assert_eq!(
                &rosters.states[span.start..span.start + span.count],
                &[row.country + 10_000, 0, row.country]
            );
        }
        let handle = (u64::from(row.country) << 32) | 4713;
        for span in row.leader_rosters.unwrap() {
            assert_eq!(
                &rosters.leaders[span.start..span.start + span.count],
                &[handle, 0, handle]
            );
        }
    }
}

#[test]
fn a_short_arena_grows_and_invalid_leaders_preserve_country_scalars() {
    let (mut world, leader_objects) = fixture(2);
    let mut rows = vec![CounterInputsRow::default(); 2];
    let mut states = vec![0; 17];
    let mut leaders = vec![0; 12];
    let mut rosters = CounterRosters {
        capture_sent_expeditionaries: true,

        states: Buffer::Growing(&mut states),
        leaders: Buffer::Growing(&mut leaders),
        state_len: 0,
        leader_len: 0,
        required: Default::default(),
    };
    assert_eq!(
        walk_counter_inputs_with_rosters(
            &world.memory(),
            world.module,
            world.game_state(),
            &mut rows,
            &mut rosters,
        ),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        rosters.required,
        RosterDemand {
            states: 18,
            leaders: 12,
        }
    );
    assert!(rows[0].state_rosters.is_some());
    assert!(rows[1].state_rosters.is_some());
    assert!(rows[1].leader_rosters.is_some());
    assert_eq!(rows[1].roster_reason, None);
    assert_eq!(
        (rows[1].owned_state_count, rows[1].controlled_state_count),
        (3, 3)
    );
    assert_eq!(rows[1].pride_of_the_fleet, (2 << 32) | 51);
    assert_eq!(rows[1].first_division, (2 << 32) | 4);
    world.u32(leader_objects[0] + cunit_leader::TYPE, 73);
    rosters.state_len = 0;
    rosters.leader_len = 0;
    rosters.required = Default::default();
    assert_eq!(
        walk_counter_inputs_with_rosters(
            &world.memory(),
            world.module,
            world.game_state(),
            &mut rows,
            &mut rosters,
        ),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(rows[0].state_rosters, None);
    assert_eq!(
        rows[0].roster_reason,
        Some("unit leader reference type mismatch")
    );
    assert!(rows[1].state_rosters.is_some());
    assert_eq!((rosters.state_len, rosters.leader_len), (9, 6));
    world.u32(leader_objects[0] + cunit_leader::TYPE, 4713);
    world.u64(leader_objects[0], 0);
    rosters.state_len = 0;
    rosters.leader_len = 0;
    assert_eq!(
        walk_counter_inputs_with_rosters(
            &world.memory(),
            world.module,
            world.game_state(),
            &mut rows,
            &mut rosters,
        ),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(rows[0].roster_reason, Some("unit leader vtable mismatch"));
}

#[test]
fn absent_arenas_preserve_all_scalar_rows() {
    let (world, _) = fixture(4);
    let mut rows = vec![CounterInputsRow::default(); 4];
    let mut rosters = CounterRosters {
        capture_sent_expeditionaries: true,

        states: Buffer::Count,
        leaders: Buffer::Count,
        state_len: 0,
        leader_len: 0,
        required: Default::default(),
    };
    assert_eq!(
        walk_counter_inputs_with_rosters(
            &world.memory(),
            world.module,
            world.game_state(),
            &mut rows,
            &mut rosters,
        ),
        Outcome::Rows { len: 4 }
    );
    assert_eq!(
        rosters.required,
        RosterDemand {
            states: 36,
            leaders: 24,
        }
    );
    for row in rows {
        assert_eq!(row.pride_of_the_fleet, (u64::from(row.country) << 32) | 51);
        assert_eq!(row.first_division, (u64::from(row.country) << 32) | 4);
        assert_eq!((row.owned_state_count, row.controlled_state_count), (3, 3));
        assert_eq!(row.state_rosters, None);
        assert_eq!(row.leader_rosters, None);
        assert_eq!(
            row.roster_reason,
            Some("country state roster arena capacity exceeded")
        );
    }
}

#[test]
fn observed_empty_and_null_character_rosters_are_present_empty_spans() {
    let world = World::new(0x4000_0000, 1, 0x1100);
    let mut rows = vec![CounterInputsRow::default(); 1];
    let mut rosters = CounterRosters {
        capture_sent_expeditionaries: true,

        states: Buffer::Count,
        leaders: Buffer::Count,
        state_len: 0,
        leader_len: 0,
        required: Default::default(),
    };
    assert_eq!(
        walk_counter_inputs_with_rosters(
            &world.memory(),
            world.module,
            world.game_state(),
            &mut rows,
            &mut rosters,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].state_rosters, Some([RosterSpan::default(); 3]));
    assert_eq!(rows[0].leader_rosters, Some([RosterSpan::default(); 2]));
}

#[test]
fn a_short_member_copy_cannot_publish_empty_rosters_and_demand_never_follows_members() {
    struct Fault<'a> {
        memory: engine_native_memory::memory::SliceMemory<'a>,
        address: usize,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            !(address..address + output.len()).contains(&self.address)
                && self.memory.copy(address, output)
        }
    }
    let (world, _) = fixture(1);
    let list = read::<u64>(&world.memory(), world.object(1) + OWNED_STATES).unwrap() as usize;
    let memory = Fault {
        memory: world.memory(),
        address: list + 8,
    };
    assert_eq!(
        counter_roster_demand(&memory, world.module, world.game_state()),
        Ok(RosterDemand {
            states: 9,
            leaders: 6,
        })
    );
    let mut rows = vec![CounterInputsRow::default(); 1];
    let mut rosters = CounterRosters {
        capture_sent_expeditionaries: true,

        states: Buffer::Growing(&mut vec![0; 9]),
        leaders: Buffer::Growing(&mut vec![0; 6]),
        state_len: 0,
        leader_len: 0,
        required: Default::default(),
    };
    assert_eq!(
        walk_counter_inputs_with_rosters(
            &memory,
            world.module,
            world.game_state(),
            &mut rows,
            &mut rosters,
        ),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].state_rosters, None);
    assert_eq!(
        rows[0].roster_reason,
        Some("country state roster pointer unreadable")
    );
    assert_eq!(
        (rows[0].owned_state_count, rows[0].controlled_state_count),
        (3, 3)
    );
    assert_eq!(rows[0].pride_of_the_fleet, (1 << 32) | 51);
    assert_eq!(rows[0].first_division, (1 << 32) | 4);
    assert_eq!((rosters.state_len, rosters.leader_len), (0, 0));
}

#[test]
fn growing_civil_wars_capture_every_native_roster_and_same_hour_expansion() {
    let mut world = World::new(0x4000_0000, 12, 0x1300);
    let targets = world.alloc(1_024 * 4);
    for index in 0..1_024 {
        world.u32(targets + index * 4, index as u32);
    }
    for slot in 1..=12 {
        world.array(
            world.object(slot) + crate::ccountry::CIVIL_WAR_TARGETS,
            targets,
            64,
        );
    }
    let mut rows = Vec::new();
    assert_eq!(
        crate::ccountry::walk_civil_wars(
            &world.memory(),
            world.module,
            world.game_state(),
            &mut rows
        ),
        Outcome::Rows { len: 12 * 64 }
    );
    world.array(
        world.object(1) + crate::ccountry::CIVIL_WAR_TARGETS,
        targets,
        1_024,
    );
    assert_eq!(
        crate::ccountry::walk_civil_wars(
            &world.memory(),
            world.module,
            world.game_state(),
            &mut rows
        ),
        Outcome::Rows { len: 12 * 64 + 960 }
    );
    assert_eq!(
        (rows[1_023].country, rows[1_023].ordinal, rows[1_023].target),
        (1, 1_023, 1_023)
    );
}

struct Hinted<'a> {
    memory: engine_native_memory::memory::SliceMemory<'a>,
    hints: std::cell::RefCell<Vec<usize>>,
}

impl Memory for Hinted<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.memory.copy(address, destination)
    }

    fn prefetch(&self, address: usize, _len: usize) {
        self.hints.borrow_mut().push(address);
    }
}

fn counter_walk(memory: &impl Memory, world: &World) -> (Outcome, Vec<CounterInputsRow>) {
    let mut rows = vec![CounterInputsRow::default(); 40];
    let (mut states, mut leaders) = (vec![0; 4_000], vec![0; 4_000]);
    let (mut neighbors, mut policies) = (vec![0; 4_000], vec![0; 4_000]);
    let mut rosters = CounterRosters {
        capture_sent_expeditionaries: true,

        states: Buffer::Growing(&mut states),
        leaders: Buffer::Growing(&mut leaders),
        state_len: 0,
        leader_len: 0,
        required: Default::default(),
    };
    let mut effects = EffectRosters {
        neighbors: Buffer::Growing(&mut neighbors),
        policies: Buffer::Growing(&mut policies),
        neighbor_len: 0,
        policy_len: 0,
        required: Default::default(),
    };
    let outcome = crate::ccountry::walk_counter_inputs_inner(memory, world.module, world.game_state(), engine_native_memory::walk::Buffer::Growing(&mut rows), Some(&mut rosters), Some(&mut effects), None, None);
    (outcome, rows)
}

#[test]
fn the_counter_walk_hints_each_countrys_fields_before_reading_them() {
    let (world, _) = fixture(40);
    let hinted = Hinted {
        memory: world.memory(),
        hints: Default::default(),
    };
    assert_eq!(
        counter_walk(&hinted, &world),
        counter_walk(&world.memory(), &world)
    );
    let hints = hinted.hints.into_inner();
    for country in 1..=40 {
        let object = world.object(country);
        for offset in [
            crate::ccountry::PRIDE_OF_THE_FLEET,
            OWNED_STATES,
            CONTROLLED_STATES,
            crate::ccountry::DIVISIONS,
            CORE_STATES,
            crate::ccountry::CHARACTERS,
            NEIGHBORS,
            crate::ccountry::PRODUCTION_STATUS,
        ] {
            assert!(hints.contains(&(object + offset)), "{country} +{offset:#x}");
        }
    }
}

struct Guarded<'a> {
    memory: engine_native_memory::memory::SliceMemory<'a>,
    guards: std::cell::Cell<usize>,
}

impl Memory for Guarded<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.guards.set(self.guards.get() + 1);
        self.memory.copy(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.guards.set(self.guards.get() + 1);
        for (index, (address, destination)) in addresses.into_iter().zip(destinations).enumerate() {
            if !destination.is_empty() && (address == 0 || !self.memory.copy(address, destination))
            {
                return index;
            }
        }
        N
    }
}

#[test]
fn a_page_of_leaders_reads_their_identities_in_one_guarded_batch() {
    let (mut world, leader_objects) = fixture(1);
    let characters = world.object(1) + crate::ccountry::CHARACTERS;
    let characters = read::<u64>(&world.memory(), characters).unwrap() as usize;
    let list = world.alloc(60 * 8);
    for slot in 0..60 {
        let pointer = if slot % 7 == 3 { 0 } else { leader_objects[0] };
        world.u64(list + slot * 8, pointer as u64);
    }
    world.array(characters + LEADERS[0], list, 60);
    let counted = Guarded {
        memory: world.memory(),
        guards: Default::default(),
    };
    let mut rows = vec![CounterInputsRow::default(); 1];
    let (mut states, mut leaders) = (vec![0; 16], vec![0; 70]);
    let mut rosters = CounterRosters {
        capture_sent_expeditionaries: true,

        states: Buffer::Growing(&mut states),
        leaders: Buffer::Growing(&mut leaders),
        state_len: 0,
        leader_len: 0,
        required: Default::default(),
    };
    assert_eq!(
        walk_counter_inputs_with_rosters(
            &counted,
            world.module,
            world.game_state(),
            &mut rows,
            &mut rosters,
        ),
        Outcome::Rows { len: 1 }
    );
    let handle = (1_u64 << 32) | 4713;
    let army = rows[0].leader_rosters.unwrap()[0];
    let expected: Vec<u64> = (0..60)
        .map(|slot| if slot % 7 == 3 { 0 } else { handle })
        .collect();
    assert_eq!(
        &rosters.leaders[army.start..army.start + army.count],
        expected
    );
    let guards = counted.guards.get();
    assert!(guards < 60, "{guards} guarded calls");
}

#[test]
fn manpower_sent_expeditionaries_keep_raw_reference_order() {
    let (mut world, _) = fixture(4);
    for country in 1..=4 {
        let entries = world.alloc(32);
        let refs = [u64::MAX, 0, (u64::from(country) << 32) | 4713, u64::MAX];
        for (index, reference) in refs.into_iter().enumerate() {
            world.u64(entries + index * 8, reference);
        }
        world.array(world.object(country) + SENT_EXPEDITIONARIES, entries, 4);
    }
    let mut rows = vec![CounterInputsRow::default(); 4];
    let mut states = vec![0; 36];
    let mut leaders = vec![0; 40];
    let mut rosters = CounterRosters {
        capture_sent_expeditionaries: true,

        states: Buffer::Growing(&mut states),
        leaders: Buffer::Growing(&mut leaders),
        state_len: 0,
        leader_len: 0,
        required: Default::default(),
    };
    assert_eq!(
        walk_counter_inputs_with_rosters(
            &world.memory(),
            world.module,
            world.game_state(),
            &mut rows,
            &mut rosters
        ),
        Outcome::Rows { len: 4 }
    );
    for row in rows {
        let span = row.sent_expeditionaries.unwrap();
        assert_eq!(
            &rosters.leaders[span.start..span.start + span.count],
            &[u64::MAX, 0, (u64::from(row.country) << 32) | 4713, u64::MAX]
        );
    }
}

#[test]
fn manpower_unreadable_expeditionary_references_keep_state_and_leader_rosters() {
    let (mut world, _) = fixture(2);
    world.array(world.object(1) + SENT_EXPEDITIONARIES, 0xdead_beef, 1);
    let mut rows = vec![CounterInputsRow::default(); 2];
    let mut states = vec![0; 18];
    let mut leaders = vec![0; 13];
    let mut rosters = CounterRosters {
        capture_sent_expeditionaries: true,
        states: Buffer::Growing(&mut states),
        leaders: Buffer::Growing(&mut leaders),
        state_len: 0,
        leader_len: 0,
        required: Default::default(),
    };
    assert_eq!(
        walk_counter_inputs_with_rosters(
            &world.memory(),
            world.module,
            world.game_state(),
            &mut rows,
            &mut rosters
        ),
        Outcome::Rows { len: 2 }
    );
    assert!(rows[0].state_rosters.is_some());
    assert!(rows[0].leader_rosters.is_some());
    assert_eq!(rows[0].sent_expeditionaries, None);
    assert_eq!(rows[0].roster_reason, None);
    assert_eq!(rows[1].sent_expeditionaries.unwrap().count, 0);
}

#[test]
fn manpower_without_a_certificate_reads_no_sent_reference_header() {
    let (world, _) = fixture(2);
    struct NoSent<'a> {
        memory: engine_native_memory::memory::SliceMemory<'a>,
        forbidden: [usize; 2],
    }
    impl Memory for NoSent<'_> {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            assert!(
                !self.forbidden.contains(&address),
                "uncertified sent-reference read"
            );
            self.memory.copy(address, bytes)
        }
    }
    let memory = NoSent {
        memory: world.memory(),
        forbidden: [
            world.object(1) + SENT_EXPEDITIONARIES,
            world.object(2) + SENT_EXPEDITIONARIES,
        ],
    };
    let mut rows = vec![CounterInputsRow::default(); 2];
    let mut states = vec![0; 18];
    let mut leaders = vec![0; 12];
    let mut rosters = CounterRosters {
        capture_sent_expeditionaries: false,
        states: Buffer::Growing(&mut states),
        leaders: Buffer::Growing(&mut leaders),
        state_len: 0,
        leader_len: 0,
        required: Default::default(),
    };
    assert_eq!(
        walk_counter_inputs_with_rosters(
            &memory,
            world.module,
            world.game_state(),
            &mut rows,
            &mut rosters
        ),
        Outcome::Rows { len: 2 }
    );
    assert!(rows.iter().all(|row| row.sent_expeditionaries.is_none()
        && row.state_rosters.is_some()
        && row.leader_rosters.is_some()));
}
