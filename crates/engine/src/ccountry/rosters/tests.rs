use super::*;
use crate::ccountry::{CounterInputsRow, synthetic::World, walk_counter_inputs_with_rosters};
use crate::read;
use crate::walk::Outcome;

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
        world.u64(division, (world.module + crate::carmy::VTABLE_RVA) as u64);
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
            (world.module + ccountry_characters::VTABLE_RVA) as u64,
        );
        world.u64(characters + ccountry_characters::OWNER, owner as u64);
        let leader = world.alloc(0x10);
        world.u64(leader, (world.module + cunit_leader::VTABLE_RVA) as u64);
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
fn five_native_order_rosters_scale_to_5000_countries_without_fabricating_handles() {
    let (world, _) = fixture(5_000);
    assert_eq!(
        counter_roster_demand(&world.memory(), world.module, world.game_state()),
        Ok(RosterDemand {
            states: 45_000,
            leaders: 30_000,
        })
    );
    let mut rows = vec![CounterInputsRow::default(); 5_000];
    let mut states = vec![0; 45_000];
    let mut leaders = vec![0; 30_000];
    let mut rosters = CounterRosters {
        states: &mut states,
        leaders: &mut leaders,
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
        Outcome::Rows { len: 5_000 }
    );
    assert_eq!(
        rosters.required,
        RosterDemand {
            states: 45_000,
            leaders: 30_000,
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
fn a_short_arena_and_invalid_leaders_preserve_country_scalars() {
    let (mut world, leader_objects) = fixture(2);
    let mut rows = [CounterInputsRow::default(); 2];
    let mut states = [0; 17];
    let mut leaders = [0; 12];
    let mut rosters = CounterRosters {
        states: &mut states,
        leaders: &mut leaders,
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
    assert_eq!(rows[1].state_rosters, None);
    assert_eq!(rows[1].leader_rosters, None);
    assert_eq!(
        rows[1].roster_reason,
        Some("country state roster arena capacity exceeded")
    );
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
fn absent_arenas_preserve_all_scalar_rows_at_5000_countries() {
    let (world, _) = fixture(5_000);
    let mut rows = vec![CounterInputsRow::default(); 5_000];
    let mut rosters = CounterRosters {
        states: &mut [],
        leaders: &mut [],
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
        Outcome::Rows { len: 5_000 }
    );
    assert_eq!(
        rosters.required,
        RosterDemand {
            states: 45_000,
            leaders: 30_000,
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
    let mut rows = [CounterInputsRow::default()];
    let mut rosters = CounterRosters {
        states: &mut [],
        leaders: &mut [],
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
        memory: crate::memory::SliceMemory<'a>,
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
    let mut rows = [CounterInputsRow::default()];
    let mut rosters = CounterRosters {
        states: &mut [0; 9],
        leaders: &mut [0; 6],
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
