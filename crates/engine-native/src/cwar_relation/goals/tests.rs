use engine_native_parts::cdiplomacy_status_layout as cdiplomacy_status;
use super::*;
use crate::ccountry::synthetic::World;

fn fixture(countries: u32) -> (World, Vec<usize>) {
    let mut world = World::new(0x4000_0000, countries, 0x1100);
    let definition = world.alloc(0x10);
    world.u32(definition + TYPE_TOKEN, 123);
    let mut goals = Vec::new();
    for country in 1..=countries {
        let owner = world.object(country);
        let diplomacy = world.alloc(0x300);
        let list = world.alloc(8);
        let goal = world.alloc(0x90);
        let state_list = world.alloc(32);
        let first = world.alloc(cstate::ID + 4);
        let second = world.alloc(cstate::ID + 4);
        world.u64(owner + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        world.u64(
            diplomacy,
            (world.module + cdiplomacy_status::VTABLE_RVA.0) as u64,
        );
        world.u64(diplomacy + cdiplomacy_status::COUNTRY, owner as u64);
        world.array(diplomacy + cdiplomacy_status::OWNED_WARGOALS, list, 1);
        world.u64(list, goal as u64);
        world.u64(goal, (world.module + VTABLE_RVA.0) as u64);
        world.u32(goal + RECIPIENT, 0x8000_0000 | country);
        world.u64(goal + GOAL_TYPE, definition as u64);
        world.array(goal + STATES, state_list, 4);
        world.u64(state_list, first as u64);
        world.u64(state_list + 8, 0);
        world.u64(state_list + 16, second as u64);
        world.u64(state_list + 24, first as u64);
        world.u32(first + cstate::ID, country);
        world.u32(second + cstate::ID, country + 10_000);
        goals.push(goal);
    }
    (world, goals)
}

fn capture(world: &World, rows: &mut Vec<Row>, states: &mut Vec<u32>) -> Walk {
    walk(
        &world.memory(),
        world.module,
        world.game_state(),
        rows,
        states,
    )
}

#[test]
fn owned_rosters_preserve_order_raw_targets_tokens_nulls_and_demand() {
    let (world, _) = fixture(4);
    let mut rows = vec![Row::default(); 4];
    let mut states = vec![0; 16];
    let captured = capture(&world, &mut rows, &mut states);
    assert_eq!(captured.outcome, Outcome::Rows { len: 4 });
    assert_eq!(
        captured.required,
        Demand {
            rows: 4,
            states: 16,
        }
    );
    for (index, row) in rows.iter().enumerate() {
        let country = index as u32 + 1;
        assert_eq!(
            (row.country, row.ordinal, row.recipient, row.goal_type),
            (country, 0, 0x8000_0000 | country, 123)
        );
        assert_eq!(
            &states[row.state_start..row.state_start + row.state_count],
            &[country, 0, country + 10_000, country]
        );
    }
    let demand = walk_buffer(
        &world.memory(),
        world.module,
        world.game_state(),
        crate::walk::Buffer::Count,
        crate::walk::Buffer::Count,
    );
    assert_eq!(demand.required, captured.required);
    assert_eq!(
        demand.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 4,
        }
    );
}

#[test]
fn an_observed_empty_roster_is_complete_and_missing_or_null_inputs_are_unavailable() {
    let (mut world, goals) = fixture(1);
    let owner = world.object(1);
    let diplomacy =
        read::<u64>(&world.memory(), owner + ccountry::DIPLOMACY_STATUS).unwrap() as usize;
    world.array(diplomacy + cdiplomacy_status::OWNED_WARGOALS, 0, 0);
    assert_eq!(
        capture(&world, &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Rows { len: 0 }
    );
    world.u64(owner + ccountry::DIPLOMACY_STATUS, 0);
    assert_eq!(
        capture(&world, &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Unavailable {
            reason: "owned wargoal diplomacy root is null",
        }
    );
    world.u64(owner + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
    let list = world.alloc(8);
    world.array(diplomacy + cdiplomacy_status::OWNED_WARGOALS, list, 1);
    world.u64(list, goals[0] as u64);
    world.u64(goals[0] + GOAL_TYPE, 0);
    assert_eq!(
        capture(&world, &mut vec![Row::default()], &mut vec![0; 4]).outcome,
        Outcome::Unavailable {
            reason: "owned wargoal type pointer is null or invalid",
        }
    );
    world.u64(list, 0);
    assert_eq!(
        capture(&world, &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Unavailable {
            reason: "owned wargoal pointer is null",
        }
    );
}

#[test]
fn native_goal_ordinal_and_first_guarded_failure_survive_reordered_members() {
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
    let (mut world, goals) = fixture(2);
    let diplomacy = |world: &World, country| {
        read::<u64>(
            &world.memory(),
            world.object(country) + ccountry::DIPLOMACY_STATUS,
        )
        .unwrap() as usize
    };
    let first = diplomacy(&world, 1);
    let second = diplomacy(&world, 2);
    let list = world.alloc(16);
    world.array(first + cdiplomacy_status::OWNED_WARGOALS, list, 2);
    world.array(second + cdiplomacy_status::OWNED_WARGOALS, 0, 0);
    world.u64(list, goals[1] as u64);
    world.u64(list + 8, goals[0] as u64);
    let mut rows = vec![Row::default(); 2];
    assert_eq!(
        capture(&world, &mut rows, &mut vec![0; 8]).outcome,
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        (rows[0].country, rows[0].ordinal, rows[0].recipient),
        (1, 0, 0x8000_0002)
    );
    assert_eq!(
        (rows[1].country, rows[1].ordinal, rows[1].recipient),
        (1, 1, 0x8000_0001)
    );
    let memory = Fault {
        memory: world.memory(),
        address: list + 8,
    };
    assert_eq!(
        walk(
            &memory,
            world.module,
            world.game_state(),
            &mut rows,
            &mut vec![0; 8],
        )
        .outcome,
        Outcome::Unavailable {
            reason: "owned wargoal pointer unreadable",
        }
    );
    let state_list = read::<u64>(&world.memory(), goals[1] + STATES).unwrap() as usize;
    let memory = Fault {
        memory: world.memory(),
        address: state_list + 8,
    };
    assert_eq!(
        walk_buffer(
            &memory,
            world.module,
            world.game_state(),
            crate::walk::Buffer::Count,
            crate::walk::Buffer::Count,
        )
        .required,
        Demand { rows: 2, states: 8 }
    );
    assert_eq!(
        walk(
            &memory,
            world.module,
            world.game_state(),
            &mut rows,
            &mut vec![0; 8],
        )
        .outcome,
        Outcome::Unavailable {
            reason: "owned wargoal state pointer unreadable",
        }
    );
}

#[test]
fn growing_goals_capture_every_country_and_expand_claims_in_the_same_hour() {
    let (mut world, goals) = fixture(12);
    let (mut rows, mut states) = (Vec::new(), Vec::new());
    let first = walk(
        &world.memory(),
        world.module,
        world.game_state(),
        &mut rows,
        &mut states,
    );
    assert_eq!(first.outcome, Outcome::Rows { len: 12 });
    assert_eq!(first.state_len, 48);
    let expanded = world.alloc(40 * 8);
    world.array(goals[0] + STATES, expanded, 40);
    let second = walk(
        &world.memory(),
        world.module,
        world.game_state(),
        &mut rows,
        &mut states,
    );
    assert_eq!(second.outcome, Outcome::Rows { len: 12 });
    assert_eq!(second.state_len, 84);
    assert_eq!(rows[0].state_count, 40);
    assert!(states[..40].iter().all(|&state| state == 0));
}
