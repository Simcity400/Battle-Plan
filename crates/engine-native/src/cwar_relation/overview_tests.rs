#[path = "overview_cost_baseline.rs"]
mod baseline;
use super::*;
use crate::ccountry::synthetic::World;

#[test]
fn participation_sources_include_authenticated_countries_without_owned_wars() {
    let world = fixture(3);
    let mut cache = DiscoveryCache::new(4, 0);
    let (mut owned, mut goals, mut scores) = (Vec::new(), Vec::new(), Vec::new());
    let outcomes = walk_cached(
        &world.memory(),
        0,
        world.game_state(),
        &mut owned,
        &mut goals,
        &mut scores,
        &mut cache,
    );
    assert!(matches!(outcomes[0], Outcome::Rows { .. }));
    let sources: Vec<_> = cache.country_sources().collect();
    assert_eq!(
        sources.iter().map(|c| c.slot).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(sources.iter().all(|c| c.country != 0 && c.diplomacy != 0));
    cache.invalidate();
    assert_eq!(cache.country_sources().count(), 0);
}

fn fixture(count: u32) -> World {
    let mut world = World::new(0, count, 0x1600);
    let mut roots = Vec::new();
    for slot in 1..=count {
        let production = world.alloc(0x300);
        world.u64(production, crate::cproduction_status::VTABLE_RVA.0 as u64);
        world.u64(
            production + crate::cproduction_status::COUNTRY,
            world.object(slot) as u64,
        );
        world.u64(
            world.object(slot) + ccountry::PRODUCTION_STATUS,
            production as u64,
        );
        let diplomacy = world.alloc(0x300);
        roots.push(diplomacy);
        world.u64(
            world.object(slot) + ccountry::DIPLOMACY_STATUS,
            diplomacy as u64,
        );
        world.u64(diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64);
        world.u64(
            diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
            world.object(slot) as u64,
        );
        let offsite = world.alloc(0x80);
        world.u64(offsite, crate::cprovince::buildings::VTABLE_RVA.0 as u64);
        world.u64(
            world.object(slot) + ccountry::WAR_OVERVIEW_OFFSITE,
            offsite as u64,
        );
        world.put(world.object(slot) + ccountry::DYNAMIC_MAJOR, &[2]);
        world.array(world.object(slot) + ccountry::DIVISIONS, 0x8000_0000, slot);
    }
    for first in (1..count).step_by(2) {
        let relation = world.alloc(0x1b0);
        world.u64(relation, super::super::VTABLE_RVA.0 as u64);
        world.u32(relation + crelation::TYPE_ID, super::super::TYPE);
        world.u32(relation + crelation::FIRST_COUNTRY, first);
        world.u32(relation + crelation::SECOND_COUNTRY, first + 1);
        world.put(relation + crelation::RETIRED, &[3]);
        let handles = world.alloc(16);
        world.array(relation + super::super::GOALS[0], handles, 2);
        for owner in [first, first + 1] {
            let vector = world.alloc(8);
            world.u64(vector, relation as u64);
            world.array(
                roots[owner as usize - 1] + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
                vector,
                1,
            );
        }
    }
    world
}

#[test]
fn owned_order_retired_bytes_null_goals_and_scores_for_every_country() {
    for count in [4, 40] {
        let world = fixture(count);
        let memory = world.memory();
        assert_eq!(
            demand(&memory, 0, world.game_state()).unwrap(),
            Demand {
                owned: count as usize,
                goals: count as usize * 2,
                relations: count as usize,
            }
        );
        let mut owned = vec![OwnedRow::default(); count as usize];
        let mut goals = vec![cwar_goal::relation::Row::default(); count as usize * 2];
        let mut scores = vec![CountryRow::default(); count as usize];
        assert_eq!(
            walk(
                &memory,
                0,
                world.game_state(),
                &mut owned,
                &mut goals,
                &mut scores
            ),
            [
                Outcome::Rows {
                    len: count as usize
                },
                Outcome::Rows {
                    len: count as usize * 2
                },
                Outcome::Rows {
                    len: count as usize
                }
            ]
        );
        assert_eq!(
            (
                owned[0].retired,
                owned[0].first_goal_count,
                owned[0].second_goal_count
            ),
            (3, 2, 0)
        );
        assert_eq!(
            (goals[1].ordinal, goals[1].resolved, goals[1].handle),
            (1, 0, 0)
        );
        assert_eq!(
            (
                scores[count as usize - 1].major,
                scores[count as usize - 1].divisions,
                scores[0].factories
            ),
            (2, 0, 0)
        );
    }
}

#[test]
fn bad_owned_identity_and_short_goal_arrays_are_unavailable() {
    let mut world = fixture(2);
    let diplomacy = read::<u64>(
        &world.memory(),
        world.object(1) + ccountry::DIPLOMACY_STATUS,
    )
    .unwrap() as usize;
    let vector = read::<u64>(
        &world.memory(),
        diplomacy + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
    )
    .unwrap() as usize;
    let relation = read::<u64>(&world.memory(), vector).unwrap() as usize;
    world.u32(relation + crelation::SECOND_COUNTRY, 1);
    assert!(
        walk(
            &world.memory(),
            0,
            world.game_state(),
            &mut vec![OwnedRow::default(); 2],
            &mut vec![cwar_goal::relation::Row::default(); 4],
            &mut vec![CountryRow::default(); 2]
        )
        .iter()
        .all(|o| matches!(o, Outcome::Unavailable { .. }))
    );
    world.u32(relation + crelation::SECOND_COUNTRY, 2);
    world.u64(relation + super::super::GOALS[0], u64::MAX - 7);
    assert!(
        walk(
            &world.memory(),
            0,
            world.game_state(),
            &mut vec![OwnedRow::default(); 2],
            &mut vec![cwar_goal::relation::Row::default(); 4],
            &mut vec![CountryRow::default(); 2]
        )
        .iter()
        .all(|o| matches!(o, Outcome::Unavailable { .. }))
    );
}

fn cost_fixture(count: u32) -> World {
    use engine_native_parts::{cbuilding, cbuilding_template};
    use crate::{cprovince::buildings as cbuilding_status, cstate};
    let mut w = fixture(count);
    let template = w.alloc(cbuilding_template::SIZE);
    w.u64(template, cbuilding_template::VTABLE_RVA.0 as u64);
    w.u32(template + cbuilding_template::WAR_OVERVIEW_PRODUCTION[0], 1);
    for slot in 1..=count {
        let root =
            read::<u64>(&w.memory(), w.object(slot) + ccountry::DIPLOMACY_STATUS).unwrap() as usize;
        let old =
            read::<u64>(&w.memory(), root + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS).unwrap() as usize;
        let war = read::<u64>(&w.memory(), old).unwrap();
        w.put(war as usize + crelation::RETIRED, &[0]);
        let relations = w.alloc(25 * 8);
        w.u64(relations, war);
        for index in 1..25 {
            let relation = w.alloc(0x50);
            w.u32(relation + crelation::TYPE_ID, 99);
            w.u64(relations + index * 8, relation as u64);
        }
        w.array(root + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, relations, 25);
        let cores = w.alloc(4 * 8);
        for index in 0..4 {
            let state = w.alloc(0x200);
            w.u64(state, cstate::VTABLE_RVA.0 as u64);
            w.u32(state + cstate::CONTROLLER, slot);
            w.u64(cores + index * 8, state as u64);
            let buildings = w.alloc(8 * 8);
            for i in 0..8 {
                let building = w.alloc(0x200);
                w.u64(building, cbuilding::VTABLE_RVA.0 as u64);
                w.u64(building + cbuilding::TEMPLATE, template as u64);
                w.put(building + cbuilding::LEVEL, &3_i16.to_le_bytes());
                w.u64(buildings + i * 8, building as u64);
            }
            w.array(
                state + cstate::BUILDING_STATUS + cbuilding_status::ACTIVE_BUILDINGS,
                buildings,
                8,
            );
        }
        w.array(
            w.object(slot) + ccountry::WAR_OVERVIEW_CORE_STATES,
            cores,
            4,
        );
    }
    w.u32(w.game_state() + crate::cgame_state::LOCAL_PLAYER_COUNTRY, 1);
    let root = read::<u64>(&w.memory(), w.object(1) + ccountry::DIPLOMACY_STATUS).unwrap() as usize;
    let matrix = w.alloc((count as usize + 1) * 8);
    for target in 2..=count {
        let status = w.alloc(0x340);
        w.u64(status, crelation_status::VTABLE_RVA.0 as u64);
        w.u64(status + crelation_status::DIPLOMACY_STATUS, root as u64);
        w.u32(status + crelation_status::TARGET_COUNTRY, target);
        w.u64(matrix + target as usize * 8, status as u64);
    }
    w.array(root + engine_native_parts::cdiplomacy_status_layout::RELATIONS, matrix, count + 1);
    w
}

#[test]
#[ignore = "synthetic acquisition timing; run alone with --nocapture"]
fn hourly_cost_at_1500_and_5000_countries() {
    use engine_native_memory::memory::{CountedMemory, ReadCounts};
    for count in [1_500, 5_000] {
        let w = cost_fixture(count);
        let memory = CountedMemory::new(w.memory());
        let mut owned = vec![OwnedRow::default(); count as usize];
        let mut goals = vec![cwar_goal::relation::Row::default(); count as usize * 2];
        let mut scores = vec![CountryRow::default(); count as usize];
        let mut cache = DiscoveryCache::new(count as usize + 1, count as usize * 25);
        let mut census = engine_native_intelligence::country_memory::Cache::with_capacity(
            engine_native_intelligence::country_memory::Cache::cells_for_countries(count as usize + 1, 0),
        );
        census.prepare(&memory, 0, w.game_state());
        let memory = census.roster_view(&memory);
        for baseline in [true, false] {
            let mut best = std::time::Duration::MAX;
            let mut calls = ReadCounts::ZERO;
            for _ in 0..12 {
                ReadCounts::take_current();
                let start = std::time::Instant::now();
                let outcomes = if baseline {
                    baseline::walk(
                        &memory,
                        0,
                        w.game_state(),
                        &mut owned,
                        &mut goals,
                        &mut scores,
                    )
                } else {
                    walk_cached(
                        &memory,
                        0,
                        w.game_state(),
                        &mut owned,
                        &mut goals,
                        &mut scores,
                        &mut cache,
                    )
                };
                let elapsed = start.elapsed();
                let reads = ReadCounts::take_current();
                assert!(
                    outcomes.iter().all(|o| matches!(o, Outcome::Rows { .. })),
                    "{outcomes:?}"
                );
                if elapsed < best {
                    best = elapsed;
                    calls = reads;
                }
            }
            println!(
                "war hour baseline={baseline} countries={count} relations_per_owner=25 cores=4 buildings_per_core=8 guarded_calls={} wall_us={:.3} counts={calls:?}",
                calls.copy_calls + calls.record_calls + calls.scattered_calls + calls.plan_calls,
                best.as_secs_f64() * 1e6
            );
        }
    }
}

#[test]
fn shared_templates_keep_every_factory_score_native() {
    use engine_native_parts::{cbuilding, cbuilding_template};
    use crate::{cprovince::buildings as cbuilding_status, cstate};
    let mut w = cost_fixture(4);
    let words: [[u32; 2]; 4] = [[1, 0], [0, 0], [0, (-5_i32) as u32], [i32::MAX as u32, 1]];
    let templates = words.map(|[first, second]| {
        let template = w.alloc(cbuilding_template::SIZE);
        w.u64(template, cbuilding_template::VTABLE_RVA.0 as u64);
        w.u32(
            template + cbuilding_template::WAR_OVERVIEW_PRODUCTION[0],
            first,
        );
        w.u32(
            template + cbuilding_template::WAR_OVERVIEW_PRODUCTION[1],
            second,
        );
        template
    });
    for slot in 1..=4_u32 {
        let cores = read::<u64>(
            &w.memory(),
            w.object(slot) + ccountry::WAR_OVERVIEW_CORE_STATES,
        )
        .unwrap() as usize;
        for core in 0..4 {
            let state = read::<u64>(&w.memory(), cores + core * 8).unwrap() as usize;
            let buildings = read::<u64>(
                &w.memory(),
                state + cstate::BUILDING_STATUS + cbuilding_status::ACTIVE_BUILDINGS,
            )
            .unwrap() as usize;
            for index in 0..8 {
                let building = read::<u64>(&w.memory(), buildings + index * 8).unwrap() as usize;
                let pick = (slot as usize + core + index) % templates.len();
                w.u64(building + cbuilding::TEMPLATE, templates[pick] as u64);
                let level = (slot as usize * 3 + core + index) as i16;
                w.put(building + cbuilding::LEVEL, &level.to_le_bytes());
            }
        }
    }
    let mut expected = [CountryRow::default(); 4];
    let mut scores = vec![CountryRow::default(); 4];
    assert!(
        baseline::walk(
            &w.memory(),
            0,
            w.game_state(),
            &mut [OwnedRow::default(); 4],
            &mut [cwar_goal::relation::Row::default(); 8],
            &mut expected,
        )
        .iter()
        .all(|o| matches!(o, Outcome::Rows { .. }))
    );
    assert!(
        walk(
            &w.memory(),
            0,
            w.game_state(),
            &mut vec![OwnedRow::default(); 4],
            &mut vec![cwar_goal::relation::Row::default(); 8],
            &mut scores,
        )
        .iter()
        .all(|o| matches!(o, Outcome::Rows { .. }))
    );
    let scored: Vec<_> = scores
        .iter()
        .filter(|row| row.score_available == 1)
        .map(|row| (row.country, row.factories))
        .collect();
    assert_eq!(scored.len(), 2, "{scores:?}");
    assert_ne!(scored[0].1, scored[1].1);
    for (country, factories) in scored {
        let native = expected.iter().find(|row| row.country == country).unwrap();
        assert_eq!(factories, native.factories, "country {country}");
    }
}

#[test]
fn score_failures_keep_owned_and_goals_and_the_first_reason() {
    let mut w = cost_fixture(2);
    let production =
        read::<u64>(&w.memory(), w.object(1) + ccountry::PRODUCTION_STATUS).unwrap() as usize;
    w.u64(production, 0);
    let other =
        read::<u64>(&w.memory(), w.object(2) + ccountry::PRODUCTION_STATUS).unwrap() as usize;
    w.u64(other, 0);
    let outcomes = walk(
        &w.memory(),
        0,
        w.game_state(),
        &mut vec![OwnedRow::default(); 2],
        &mut vec![cwar_goal::relation::Row::default(); 4],
        &mut vec![CountryRow::default(); 2],
    );
    assert_eq!(
        outcomes,
        [
            Outcome::Rows { len: 2 },
            Outcome::Rows { len: 4 },
            Outcome::Unavailable {
                reason: "war factory vtable mismatch"
            }
        ]
    );
}

#[test]
fn unchanged_header_does_not_hide_retirement_goal_changes_or_replaced_pointers() {
    let mut w = fixture(2);
    let root = read::<u64>(&w.memory(), w.object(1) + ccountry::DIPLOMACY_STATUS).unwrap() as usize;
    let vector =
        read::<u64>(&w.memory(), root + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS).unwrap() as usize;
    let relation = read::<u64>(&w.memory(), vector).unwrap() as usize;
    let mut cache = DiscoveryCache::new(3, 2);
    let mut owned = vec![OwnedRow::default(); 2];
    let mut goals = vec![cwar_goal::relation::Row::default(); 4];
    let mut scores = vec![CountryRow::default(); 2];
    assert!(
        walk_cached(
            &w.memory(),
            0,
            w.game_state(),
            &mut owned,
            &mut goals,
            &mut scores,
            &mut cache
        )
        .iter()
        .all(|o| matches!(o, Outcome::Rows { .. }))
    );
    w.put(relation + crelation::RETIRED, &[0]);
    w.array(relation + super::super::GOALS[0], 0, 0);
    assert_eq!(
        walk_cached(
            &w.memory(),
            0,
            w.game_state(),
            &mut owned,
            &mut goals,
            &mut scores,
            &mut cache
        )[1],
        Outcome::Rows { len: 0 }
    );
    assert_eq!(owned[0].retired, 0);
    let not_war = w.alloc(0x50);
    w.u32(not_war + crelation::TYPE_ID, 99);
    w.u64(vector, not_war as u64);
    assert_eq!(
        walk_cached(
            &w.memory(),
            0,
            w.game_state(),
            &mut owned,
            &mut goals,
            &mut scores,
            &mut cache
        )[0],
        Outcome::Rows { len: 1 }
    );
}

#[test]
fn template_and_subject_failures_are_scores_only_and_unreachable_factories_are_not_read() {
    let mut w = cost_fixture(4);
    let unreachable =
        read::<u64>(&w.memory(), w.object(3) + ccountry::PRODUCTION_STATUS).unwrap() as usize;
    w.u64(unreachable, 0);
    let run = |w: &World| {
        let mut scores = vec![CountryRow::default(); 4];
        let outcomes = walk(
            &w.memory(),
            0,
            w.game_state(),
            &mut vec![OwnedRow::default(); 4],
            &mut vec![cwar_goal::relation::Row::default(); 8],
            &mut scores,
        );
        (outcomes, scores)
    };
    let (outcomes, scores) = run(&w);
    assert!(
        outcomes.iter().all(|o| matches!(o, Outcome::Rows { .. })),
        "{outcomes:?}"
    );
    assert_eq!(
        scores.iter().map(|r| r.score_available).collect::<Vec<_>>(),
        [1, 1, 0, 0]
    );

    let core = read::<u64>(
        &w.memory(),
        w.object(1) + ccountry::WAR_OVERVIEW_CORE_STATES,
    )
    .unwrap() as usize;
    let state = read::<u64>(&w.memory(), core).unwrap() as usize;
    let vector = read::<u64>(
        &w.memory(),
        state + crate::cstate::BUILDING_STATUS + crate::cprovince::buildings::ACTIVE_BUILDINGS,
    )
    .unwrap() as usize;
    let building = read::<u64>(&w.memory(), vector).unwrap() as usize;
    let template =
        read::<u64>(&w.memory(), building + engine_native_parts::cbuilding::TEMPLATE).unwrap() as usize;
    w.u64(template, 0);
    assert_eq!(
        run(&w).0,
        [
            Outcome::Rows { len: 4 },
            Outcome::Rows { len: 8 },
            Outcome::Unavailable {
                reason: "war factory vtable mismatch"
            }
        ]
    );
    w.u64(template, engine_native_parts::cbuilding_template::VTABLE_RVA.0 as u64);
    let root = read::<u64>(&w.memory(), w.object(1) + ccountry::DIPLOMACY_STATUS).unwrap() as usize;
    let matrix = read::<u64>(&w.memory(), root + engine_native_parts::cdiplomacy_status_layout::RELATIONS).unwrap() as usize;
    let status = read::<u64>(&w.memory(), matrix + 2 * 8).unwrap() as usize;
    w.u64(status, 0);
    assert_eq!(
        run(&w).0,
        [
            Outcome::Rows { len: 4 },
            Outcome::Rows { len: 8 },
            Outcome::Unavailable {
                reason: "war overview vtable mismatch"
            }
        ]
    );
}

#[test]
fn truncation_does_not_admit_an_empty_discovery_fingerprint() {
    let w = fixture(2);
    let mut cache = DiscoveryCache::new(3, 2);
    walk_cached(
        &w.memory(),
        0,
        w.game_state(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut Vec::new(),
        &mut cache,
    );
    assert_eq!(
        walk_cached(
            &w.memory(),
            0,
            w.game_state(),
            &mut vec![OwnedRow::default(); 2],
            &mut vec![cwar_goal::relation::Row::default(); 4],
            &mut vec![CountryRow::default(); 2],
            &mut cache
        )[0],
        Outcome::Rows { len: 2 }
    );
}

#[test]
fn batched_goals_preserve_owner_side_and_page_order_and_exact_truncation() {
    let mut w = fixture(4);
    let root = read::<u64>(&w.memory(), w.object(1) + ccountry::DIPLOMACY_STATUS).unwrap() as usize;
    let vector =
        read::<u64>(&w.memory(), root + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS).unwrap() as usize;
    let relation = read::<u64>(&w.memory(), vector).unwrap() as usize;
    for (offset, count) in [(super::super::GOALS[0], 65), (super::super::GOALS[1], 3)] {
        let handles = w.alloc(count * 8);
        w.array(relation + offset, handles, count as u32);
    }
    let mut cache = DiscoveryCache::new(5, 4);
    let mut owned = vec![OwnedRow::default(); 4];
    let mut scores = vec![CountryRow::default(); 4];
    let mut goals = vec![cwar_goal::relation::Row::default(); 140];
    for _ in 0..2 {
        assert_eq!(
            walk_cached(
                &w.memory(),
                0,
                w.game_state(),
                &mut owned,
                &mut goals,
                &mut scores,
                &mut cache
            )[1],
            Outcome::Rows { len: 140 }
        );
        let expected: Vec<_> = [
            (1, 0, 65),
            (1, 1, 3),
            (2, 0, 65),
            (2, 1, 3),
            (3, 0, 2),
            (4, 0, 2),
        ]
        .into_iter()
        .flat_map(|(owner, side, count)| (0..count).map(move |ordinal| (owner, side, ordinal)))
        .collect();
        assert_eq!(
            goals
                .iter()
                .map(|r| (r.owner, r.side, r.ordinal))
                .collect::<Vec<_>>(),
            expected
        );
    }
    w.u32(relation + crelation::TYPE_ID, 99);
    assert_eq!(
        walk_cached(
            &w.memory(),
            0,
            w.game_state(),
            &mut owned,
            &mut goals,
            &mut scores,
            &mut cache
        )[0],
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        owned[..2].iter().map(|r| r.owner).collect::<Vec<_>>(),
        [3, 4]
    );
}

#[test]
fn retired_viewpoint_counterparts_keep_scores_when_native_sides_can_reach_them() {
    let mut w = cost_fixture(4);
    for country in 1..=4 {
        w.put(w.object(country) + ccountry::DYNAMIC_MAJOR, &[0]);
    }
    let relation = w.alloc(0x1b0);
    w.u64(relation, super::super::VTABLE_RVA.0 as u64);
    w.u32(relation + crelation::TYPE_ID, super::super::TYPE);
    w.u32(relation + crelation::FIRST_COUNTRY, 1);
    w.u32(relation + crelation::SECOND_COUNTRY, 3);
    w.put(relation + crelation::RETIRED, &[1]);
    for owner in [1, 3] {
        let root = read::<u64>(&w.memory(), w.object(owner) + ccountry::DIPLOMACY_STATUS).unwrap()
            as usize;
        let old = Array::read(&w.memory(), root + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, 8).unwrap();
        let vector = w.alloc((old.count as usize + 1) * 8);
        for ordinal in 0..old.count {
            let pointer = read::<u64>(&w.memory(), old.element(ordinal, 8).unwrap()).unwrap();
            w.u64(vector + ordinal as usize * 8, pointer);
        }
        w.u64(vector + old.count as usize * 8, relation as u64);
        w.array(
            root + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
            vector,
            old.count + 1,
        );
    }
    let mut owned = vec![OwnedRow::default(); 6];
    let mut goals = vec![cwar_goal::relation::Row::default(); 8];
    let mut scores = vec![CountryRow::default(); 4];
    assert_eq!(
        walk(
            &w.memory(),
            0,
            w.game_state(),
            &mut owned,
            &mut goals,
            &mut scores
        ),
        [
            Outcome::Rows { len: 6 },
            Outcome::Rows { len: 8 },
            Outcome::Rows { len: 4 }
        ]
    );
    assert_eq!(
        scores.iter().map(|r| r.score_available).collect::<Vec<_>>(),
        [1, 1, 1, 0]
    );
    let production =
        read::<u64>(&w.memory(), w.object(3) + ccountry::PRODUCTION_STATUS).unwrap() as usize;
    w.u64(production, 0);
    assert_eq!(
        walk(
            &w.memory(),
            0,
            w.game_state(),
            &mut owned,
            &mut goals,
            &mut scores
        ),
        [
            Outcome::Rows { len: 6 },
            Outcome::Rows { len: 8 },
            Outcome::Unavailable {
                reason: "war factory vtable mismatch"
            }
        ]
    );
}

#[test]
fn a_flush_partway_through_a_country_keeps_its_later_pages_previous_rows() {
    fn war(w: &mut World) -> usize {
        let relation = w.alloc(0x1b0);
        w.u64(relation, super::super::VTABLE_RVA.0 as u64);
        w.u32(relation + crelation::TYPE_ID, super::super::TYPE);
        w.u32(relation + crelation::FIRST_COUNTRY, 1);
        w.u32(relation + crelation::SECOND_COUNTRY, 2);
        relation
    }
    let mut w = fixture(2);
    let root = read::<u64>(&w.memory(), w.object(1) + ccountry::DIPLOMACY_STATUS).unwrap() as usize;
    let vector = w.alloc(150 * 8);
    for ordinal in 0..150 {
        let relation = if !(63..128).contains(&ordinal) {
            war(&mut w)
        } else {
            let other = w.alloc(0x50);
            w.u32(other + crelation::TYPE_ID, 99);
            other
        };
        w.u64(vector + ordinal * 8, relation as u64);
    }
    w.array(root + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS, vector, 150);
    let mut cache = DiscoveryCache::new(3, 151);
    let mut owned = vec![OwnedRow::default(); 160];
    let mut goals = vec![cwar_goal::relation::Row::default(); 8];
    let mut scores = vec![CountryRow::default(); 2];
    let mut hour = |w: &World, owned: &mut Vec<OwnedRow>, cache: &mut DiscoveryCache| {
        walk_cached(
            &w.memory(),
            0,
            w.game_state(),
            owned,
            &mut goals,
            &mut scores,
            cache,
        )[0]
    };
    assert_eq!(hour(&w, &mut owned, &mut cache), Outcome::Rows { len: 86 });
    let replaced = war(&mut w);
    w.u64(vector + 63 * 8, replaced as u64);
    assert_eq!(hour(&w, &mut owned, &mut cache), Outcome::Rows { len: 87 });
    let mut fresh = vec![OwnedRow::default(); 160];
    walk(
        &w.memory(),
        0,
        w.game_state(),
        &mut fresh,
        &mut vec![cwar_goal::relation::Row::default(); 8],
        &mut vec![CountryRow::default(); 2],
    );
    assert_eq!(owned[..87], fresh[..87]);
}
#[test]
fn discovery_capacity_overflow_is_a_checked_error() {
    for (countries, relations) in [(usize::MAX, 0), (0, usize::MAX)] {
        assert!(matches!(
            DiscoveryCache::try_new(countries, relations),
            Err(error) if error.kind() == std::io::ErrorKind::OutOfMemory
        ));
    }
    let mut cache = DiscoveryCache::new(0, 0);
    assert_eq!(
        cache.try_grow(5, usize::MAX).unwrap_err().kind(),
        std::io::ErrorKind::OutOfMemory
    );
    assert_eq!(cache.allocated_bytes(), 0);
    cache.try_grow(1_500, 5_000).unwrap();
    assert_eq!(cache.headers.len(), 1_500);
    assert_eq!(cache.pointers.len(), 5_000);
    assert_eq!(cache.types.len(), 20_000);
}

type Hour = (
    [Outcome; 3],
    Vec<OwnedRow>,
    Vec<cwar_goal::relation::Row>,
    Vec<CountryRow>,
    Vec<u32>,
);

fn certified_hour(world: &World, cache: &mut DiscoveryCache, certified: bool) -> Hour {
    let (mut owned, mut goals, mut scores, mut visited) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let outcomes = walk_cached_with_owners(
        &world.memory(),
        0,
        world.game_state(),
        &mut owned,
        &mut goals,
        &mut scores,
        cache,
        &mut |country, _| visited.push(country.id),
        &mut || certified,
    );
    let [o, g, s] = outcomes.map(Outcome::len);
    owned.truncate(o);
    goals.truncate(g);
    scores.truncate(s);
    (outcomes, owned, goals, scores, visited)
}

#[test]
fn a_certified_hour_publishes_the_held_walk_without_reading_relations() {
    let mut world = fixture(4);
    let mut cache = DiscoveryCache::new(5, 0);
    let walked = certified_hour(&world, &mut cache, true);
    assert!(matches!(walked.0[0], Outcome::Rows { len: 4 }));
    assert!(!cache.carried());
    assert_eq!(cache.held_rows(), Some(&walked.1[..]));
    let carried = certified_hour(&world, &mut cache, true);
    assert!(cache.carried());
    assert_eq!(carried, walked);
    assert_eq!(cache.country_sources().count(), 4);
    world.u64(walked.1[0].source.relation, 0);
    assert_eq!(certified_hour(&world, &mut cache, true), walked);
    let refused = certified_hour(&world, &mut cache, false);
    assert!(!cache.carried());
    assert_eq!(
        refused.0[0],
        Outcome::Unavailable {
            reason: "war overview vtable mismatch"
        }
    );
    assert_eq!(cache.held_rows(), None);
}
