//! CountryLeaderRoles carried per character ([`super::leaders::LeaderCache`])
//! over the census test world, against the typed roster walk.

use super::census_tests::{Counting, GAME_STATE, MODULE, World, capacities, context};
use super::leaders::LeaderCache;
use super::*;
use crate::character_changes::{Changes, Counting as Lookups, Watches};
use crate::sampled_verify::Check;

/// The roster job's admitted census of this world.
fn census_of(world: &World) -> CharactersJob {
    let mut job = CharactersJob::new_part(0);
    for _ in 0..16 {
        let report =
            copy_characters_growing(world, context(PART_MASKS[0]), &mut job, Roles::Gather);
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
    check: Check,
) -> Captured {
    let owner = census_of(world);
    let loan = owner.role_census().unwrap();
    cache.capture(
        memory,
        MODULE,
        GAME_STATE,
        loan.admitted(MODULE, GAME_STATE).unwrap(),
        changes,
        check,
    );
    let mut captured = Captured::with_capacities(&capacities());
    captured.fail_character_tables(
        PART_MASKS[1].map(|active| !active),
        replay::cadence::SKIPPED_REASON,
    );
    cache.decode(&mut captured);
    captured
}

/// The typed roster walk's CountryLeaderRoles.
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
    let world = small();
    let characters = world.characters as u64;
    let watches = Watches::enabled(world.characters);
    let mut cache = LeaderCache::default();
    let first = hour(&mut cache, &world, &world, &watches, Check::Nothing);
    assert_eq!((cache.counts.read, cache.counts.reused), (characters, 0));
    assert_eq!(first.tables()[1], expected(&world));
    let counting = Counting::new(&world);
    let second = hour(&mut cache, &counting, &world, &watches, Check::Nothing);
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
    let mut world = small();
    let characters = world.characters as u64;
    let watches = Watches::enabled(world.characters);
    let mut cache = LeaderCache::default();
    hour(&mut cache, &world, &world, &watches, Check::Nothing);
    // A hooked writer touched character 10's role; its current ideology now
    // fails to read.
    let leader = World::character(10);
    world.patch(
        leader_role(10) + ccountry_leader::CURRENT_IDEOLOGY,
        &0x10_u64.to_le_bytes(),
    );
    drop(watches.notify(Some(leader), 0));
    // Character 20's country changed: the census supplies it.
    world.patch(
        World::character(20) + ccharacter::COUNTRY,
        &99_u32.to_le_bytes(),
    );
    let captured = hour(&mut cache, &world, &world, &watches, Check::Nothing);
    assert_eq!(
        (cache.counts.read, cache.counts.reused),
        (1, characters - 1)
    );
    assert_eq!(captured.tables()[1], expected(&world));
}

#[test]
fn set_ideology_and_invalidation_read_every_leader() {
    let world = small();
    let characters = world.characters as u64;
    let watches = Watches::enabled(world.characters);
    let mut cache = LeaderCache::default();
    hour(&mut cache, &world, &world, &watches, Check::Nothing);
    // set_country_leader_ideology names no character.
    drop(watches.notify(
        None,
        engine::character_role_mutations::COUNTRY_LEADER_IDEOLOGY,
    ));
    hour(&mut cache, &world, &world, &watches, Check::Nothing);
    assert_eq!((cache.counts.read, cache.counts.reused), (characters, 0));
    watches.boundary(true);
    hour(&mut cache, &world, &world, &watches, Check::Nothing);
    assert_eq!((cache.counts.read, cache.counts.reused), (characters, 0));
    cache.forget();
    hour(&mut cache, &world, &world, &watches, Check::Nothing);
    assert_eq!(cache.counts.reused, 0);
    hour(&mut cache, &world, &world, &watches, Check::Nothing);
    assert_eq!(cache.counts.read, 0);
}

#[test]
fn verify_mode_catches_an_unobserved_ideology_write_and_poisons_reuse() {
    let mut world = small();
    let characters = world.characters as u64;
    let watches = Watches::enabled(world.characters);
    let mut cache = LeaderCache::default();
    hour(&mut cache, &world, &world, &watches, Check::Every);
    hour(&mut cache, &world, &world, &watches, Check::Every);
    assert_eq!(
        (
            cache.counts.verified,
            cache.counts.mismatches,
            cache.counts.reused
        ),
        (characters, 0, 0)
    );
    world.patch(
        leader_role(15) + ccountry_leader::CURRENT_IDEOLOGY,
        &0x10_u64.to_le_bytes(),
    );
    let captured = hour(&mut cache, &world, &world, &watches, Check::Every);
    assert_eq!(cache.counts.mismatches, 1);
    assert!(watches.poisoned());
    assert_eq!(captured.tables()[1], expected(&world));
    hour(&mut cache, &world, &world, &watches, Check::Nothing);
    assert_eq!((cache.counts.read, cache.counts.reused), (characters, 0));
}

#[test]
fn sampled_hours_cover_every_leader_and_catch_a_missed_write_within_one_rotation() {
    let mut world = small();
    let characters = world.characters as u64;
    let watches = Watches::enabled(world.characters);
    let mut cache = LeaderCache::default();
    hour(&mut cache, &world, &world, &watches, Check::Nothing);
    // Any 24 consecutive hours read each carried character once.
    let mut compared = 0;
    for at in 0..24 {
        let captured = hour(
            &mut cache,
            &world,
            &world,
            &watches,
            Check::Slice { hour: at },
        );
        assert_eq!(cache.counts.mismatches, 0);
        assert_eq!(captured.tables()[1], expected(&world));
        compared += cache.counts.verified;
    }
    assert_eq!(compared, characters);
    // A write no hook observed.
    world.patch(
        leader_role(15) + ccountry_leader::CURRENT_IDEOLOGY,
        &0x10_u64.to_le_bytes(),
    );
    let mut id = [0; 4];
    assert!(world.copy(World::character(15) + ccharacter::ID, &mut id));
    let id = u32::from_le_bytes(id);
    for at in 0..24 {
        let captured = hour(
            &mut cache,
            &world,
            &world,
            &watches,
            Check::Slice { hour: at },
        );
        if at as u32 == id % 24 {
            assert_eq!(cache.counts.mismatches, 1);
            assert_eq!(captured.tables()[1], expected(&world), "fresh rows");
            break;
        }
        assert_eq!(cache.counts.mismatches, 0);
        assert_ne!(captured.tables()[1], expected(&world), "hour {at} carries");
    }
    assert!(watches.poisoned());
}

fn leader_role(index: usize) -> usize {
    assert!(World::leader(index));
    World::leader_role(index)
}

#[test]
fn quiet_hours_look_up_no_character_and_carry_the_whole_table() {
    let mut world = small();
    let characters = world.characters as u64;
    let domain = Lookups::new();
    domain.watches.enable(world.characters);
    let mut cache = LeaderCache::default();
    hour(&mut cache, &world, &world, &domain, Check::Nothing);
    assert_eq!(domain.lookups(), characters);
    // Nothing ran since: no lookup, no read, the same rows.
    let counting = Counting::new(&world);
    let quiet = hour(&mut cache, &counting, &world, &domain, Check::Nothing);
    assert_eq!(domain.lookups(), 0, "a quiet hour looks up no character");
    assert_eq!(
        counting.calls.get(),
        0,
        "a quiet hour reads no leader memory"
    );
    assert!(cache.counts.quiet);
    assert_eq!((cache.counts.read, cache.counts.reused), (0, characters));
    assert_eq!(quiet.tables()[1], expected(&world));
    // A sampled quiet hour reads only its slice.
    let counting = Counting::new(&world);
    let sampled = hour(
        &mut cache,
        &counting,
        &world,
        &domain,
        Check::Slice { hour: 5 },
    );
    let slice = (0..world.characters)
        .filter(|&index| {
            let mut id = [0; 4];
            assert!(world.copy(World::character(index) + ccharacter::ID, &mut id));
            u32::from_le_bytes(id) % 24 == 5
        })
        .count() as u64;
    assert!(slice > 0 && slice < characters);
    assert_eq!(domain.lookups(), 0);
    assert!(counting.calls.get() > 0);
    assert_eq!(
        (
            cache.counts.read,
            cache.counts.verified,
            cache.counts.reused
        ),
        (slice, slice, characters - slice)
    );
    assert_eq!(sampled.tables()[1], expected(&world));
    // A hooked writer: the hour looks up every character, reads the one.
    drop(domain.watches.notify(Some(World::character(10)), 0));
    hour(&mut cache, &world, &world, &domain, Check::Nothing);
    assert_eq!(domain.lookups(), characters);
    assert!(!cache.counts.quiet);
    assert_eq!(cache.counts.read, 1);
    hour(&mut cache, &world, &world, &domain, Check::Nothing);
    assert_eq!(domain.lookups(), 0);
    // A changed census (a country the census supplies): every character is
    // certified again, and the table follows the census.
    world.patch(
        World::character(20) + ccharacter::COUNTRY,
        &99_u32.to_le_bytes(),
    );
    let moved = hour(&mut cache, &world, &world, &domain, Check::Nothing);
    assert_eq!(domain.lookups(), characters);
    assert_eq!(cache.counts.reused, characters);
    assert_eq!(moved.tables()[1], expected(&world));
    hour(&mut cache, &world, &world, &domain, Check::Nothing);
    assert_eq!(domain.lookups(), 0);
    // A load, gap or recovery reads every character.
    domain.watches.boundary(true);
    hour(&mut cache, &world, &world, &domain, Check::Nothing);
    assert_eq!(domain.lookups(), characters);
    assert_eq!(cache.counts.read, characters);
    // A failed or censusless job forgets the held rows.
    cache.forget();
    hour(&mut cache, &world, &world, &domain, Check::Nothing);
    assert_eq!(
        (domain.lookups(), cache.counts.read),
        (characters, characters)
    );
}

#[test]
fn a_quiet_hours_slice_catches_an_unobserved_write_and_reads_afresh() {
    let mut world = small();
    let characters = world.characters as u64;
    let domain = Lookups::new();
    domain.watches.enable(world.characters);
    let mut cache = LeaderCache::default();
    hour(&mut cache, &world, &world, &domain, Check::Nothing);
    // A write no hook observed.
    world.patch(
        leader_role(15) + ccountry_leader::CURRENT_IDEOLOGY,
        &0x10_u64.to_le_bytes(),
    );
    let mut id = [0; 4];
    assert!(world.copy(World::character(15) + ccharacter::ID, &mut id));
    let at = (u32::from_le_bytes(id) % 24) as i32;
    let caught = hour(
        &mut cache,
        &world,
        &world,
        &domain,
        Check::Slice { hour: at },
    );
    assert_eq!(cache.counts.mismatches, 1);
    assert!(domain.watches.poisoned());
    assert!(!cache.counts.quiet);
    assert_eq!(cache.counts.read, characters, "the hour reads afresh");
    assert_eq!(caught.tables()[1], expected(&world));
}

/// A leader role job that carried its rows from the census copied no raw
/// log; recycling keeps it, and a pool miss starts one with its cache.
#[test]
fn recycling_keeps_a_leader_job_that_carried_from_the_census() {
    let world = small();
    let owner = census_of(&world);
    let loan = owner.role_census().unwrap();
    let mut job = CharactersJob::new_part(1);
    let report = copy_characters_growing(
        &world,
        context(PART_MASKS[1]),
        &mut job,
        Roles::Borrow(&loan),
    );
    assert!(report.raw_ok && job.leader_cached);
    job.clear();
    assert!(job.has_storage(), "the held rows are the job's storage");
    assert!(CharactersJob::vacant(1).leader_cache.is_some());
}
