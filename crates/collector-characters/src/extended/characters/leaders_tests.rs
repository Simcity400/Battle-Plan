use super::census_tests::{Counting, GAME_STATE, MODULE, World, capacities, context};
use super::leaders::LeaderCache;
use super::*;
use collector_observers::character_changes::{Changes, Counting as Lookups, Watches};

fn census_of(world: &World) -> CharactersJob {
    let mut job = CharactersJob::new_part(0);
    for _ in 0..16 {
        let report =
            copy_characters(world, context(PART_MASKS[0]), &mut job, Roles::Gather);
        if report.raw_ok && job.role_census().is_some() {
            return job;
        }
        if job.grow_for_next() {
            job.grow();
        }
        job.prepare_read_plans();
        job.clear();
    }
    panic!("no admitted role census");
}

fn hour(
    cache: &mut LeaderCache,
    memory: &impl Memory,
    world: &World,
    changes: &dyn Changes,
) -> Captured {
    let owner = census_of(world);
    let loan = owner.role_census().unwrap();
    cache.capture(
        memory,
        MODULE,
        GAME_STATE,
        loan.admitted(MODULE, GAME_STATE).unwrap(),
        changes,
    );
    let mut captured = Captured::with_capacities(&capacities());
    captured.fail_character_tables(
        PART_MASKS[1].map(|active| !active),
        replay_data::cadence::SKIPPED_REASON,
    );
    cache.decode(&mut captured);
    captured
}

fn expected(world: &World) -> TableView {
    let mut captured = Captured::with_capacities(&capacities());
    let results = walk_family(
        world,
        MODULE,
        GAME_STATE,
        captured.family().select(PART_MASKS[1]),
        None,
        None,
    );
    captured.install_family(PART_MASKS[1], results);
    captured.tables()[1].clone()
}

fn small() -> World {
    let mut world = World::new(300);
    world.dual = true;
    world
}

#[test]
fn unchanged_hours_read_no_leader_and_publish_the_roster_walk_rows() {
    let _globals = crate::extended::characters::test_globals();
    let world = small();
    let characters = world.characters as u64;
    let watches = Watches::enabled(world.characters);
    let mut cache = LeaderCache::default();
    let first = hour(&mut cache, &world, &world, &watches);
    assert_eq!((cache.counts.read, cache.counts.reused), (characters, 0));
    assert_eq!(first.tables()[1], expected(&world));
    let counting = Counting::new(&world);
    let second = hour(&mut cache, &counting, &world, &watches);
    assert_eq!((cache.counts.read, cache.counts.reused), (0, characters));
    assert_eq!(
        counting.calls.get(),
        0,
        "a quiet hour reads no leader memory"
    );
    assert_eq!(second.tables()[1], expected(&world));
}

#[test]
fn a_dirty_leader_alone_is_read_and_the_country_follows_the_census() {
    let _globals = crate::extended::characters::test_globals();
    let mut world = small();
    let characters = world.characters as u64;
    let watches = Watches::enabled(world.characters);
    let mut cache = LeaderCache::default();
    hour(&mut cache, &world, &world, &watches);
    let leader = World::character(10);
    world.patch(
        leader_role(10) + ccountry_leader::CURRENT_IDEOLOGY,
        &0x10_u64.to_le_bytes(),
    );
    drop(watches.notify(Some(leader), 0));
    world.patch(
        World::character(20) + ccharacter::COUNTRY,
        &99_u32.to_le_bytes(),
    );
    let captured = hour(&mut cache, &world, &world, &watches);
    assert_eq!(
        (cache.counts.read, cache.counts.reused),
        (1, characters - 1)
    );
    assert_eq!(captured.tables()[1], expected(&world));
}

fn leader_role(index: usize) -> usize {
    assert!(World::leader(index));
    World::leader_role(index)
}

#[test]
fn quiet_hours_look_up_no_character_and_carry_the_whole_table() {
    let _globals = crate::extended::characters::test_globals();
    let mut world = small();
    let characters = world.characters as u64;
    let domain = Lookups::new();
    domain.watches.enable(world.characters);
    let mut cache = LeaderCache::default();
    hour(&mut cache, &world, &world, &domain);
    assert_eq!(domain.lookups(), characters);
    let counting = Counting::new(&world);
    let quiet = hour(&mut cache, &counting, &world, &domain);
    assert_eq!(domain.lookups(), 0, "a quiet hour looks up no character");
    assert_eq!(
        counting.calls.get(),
        0,
        "a quiet hour reads no leader memory"
    );
    assert!(cache.counts.quiet);
    assert_eq!((cache.counts.read, cache.counts.reused), (0, characters));
    assert_eq!(quiet.tables()[1], expected(&world));
    drop(domain.watches.notify(Some(World::character(10)), 0));
    hour(&mut cache, &world, &world, &domain);
    assert_eq!(domain.lookups(), characters);
    assert!(!cache.counts.quiet);
    assert_eq!(cache.counts.read, 1);
    hour(&mut cache, &world, &world, &domain);
    assert_eq!(domain.lookups(), 0);
    world.patch(
        World::character(20) + ccharacter::COUNTRY,
        &99_u32.to_le_bytes(),
    );
    let moved = hour(&mut cache, &world, &world, &domain);
    assert_eq!(domain.lookups(), characters);
    assert_eq!(cache.counts.reused, characters);
    assert_eq!(moved.tables()[1], expected(&world));
    hour(&mut cache, &world, &world, &domain);
    assert_eq!(domain.lookups(), 0);
    domain.watches.boundary(true);
    hour(&mut cache, &world, &world, &domain);
    assert_eq!(domain.lookups(), characters);
    assert_eq!(cache.counts.read, characters);
    cache.forget();
    hour(&mut cache, &world, &world, &domain);
    assert_eq!(
        (domain.lookups(), cache.counts.read),
        (characters, characters)
    );
}

