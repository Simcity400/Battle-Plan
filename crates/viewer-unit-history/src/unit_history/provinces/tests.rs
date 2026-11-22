use super::super::{
    Budget, History, Kind, NavalSources, Target, same_history,
    test_support::{GATES, target},
};
use super::*;
use viewer_runtime::{unit_class::CounterClass, unit_handle::Unit};

fn xorshift(mut state: u32) -> impl FnMut() -> u32 {
    move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    }
}

fn candidate(province: u32, identity: u64, ordinal: u32) -> naval::Candidate {
    naval::Candidate {
        province,
        identity,
        ordinal,
        slot: ordinal % 7,
        priority: 3,
        ours: identity.is_multiple_of(2),
        class: if identity.is_multiple_of(3) {
            CounterClass::Division
        } else {
            CounterClass::TaskForce
        },
        land_eligible: true,
        sample_count: 8,
        next_arc: 0.5,
        route: vec![province, province.wrapping_add(17)],
        speed: 0.25,
        min_arc: 0.0,
        arrival_allowed: true,
        day: ordinal as i32,
    }
}

fn formation(candidate: &naval::Candidate, mesh: usize) -> naval::Formation {
    naval::Formation {
        identity: candidate.identity,
        province: candidate.province,
        slot: candidate.slot,
        roster: vec![naval::Hull {
            flags: 0x400040,
            graphics: b"synthetic_hull".to_vec(),
            targets: [
                Some(Target {
                    identity: candidate.identity,
                    mesh,
                    ..target(candidate.province, Kind::Ship)
                }),
                None,
                None,
                None,
            ],
        }],
    }
}

fn assert_same_join(pair: &[Provinces; 2], targets: &Targets, naval: &naval::History) {
    let [merged, looked_up] = pair;
    assert_eq!(merged.indices, looked_up.indices);
    assert!(merged.populated.iter().eq(looked_up.populated.iter()));
    assert_eq!(merged.records.len(), looked_up.records.len());
    for (a, b) in merged.records.iter().zip(&looked_up.records) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.province, b.province, "province {}", a.id);
        assert_eq!(a.has_candidates, b.has_candidates, "province {}", a.id);
        assert_eq!(
            a.has_candidates,
            naval.has_candidates(a.id),
            "province {}",
            a.id
        );
        match (&a.targets, &b.targets, targets.0.provinces.get(&a.id)) {
            (Some(joined), Some(reference), Some(prepared)) => {
                assert!(Arc::ptr_eq(joined, prepared), "province {}", a.id);
                assert!(Arc::ptr_eq(reference, prepared), "province {}", a.id);
            }
            (None, None, None) => {}
            _ => panic!("province {} joined different targets", a.id),
        }
    }
    for (id, prepared) in &targets.0.provinces {
        let holders = 1 + 2 * usize::from(merged.indices.contains_key(id));
        assert_eq!(Arc::strong_count(prepared), holders, "province {id} clones");
    }
}

#[test]
fn merge_join_matches_point_lookups_record_for_record() {
    let mut next = xorshift(0x5eed_1e57);
    let universe: Vec<u32> = [
        u32::MAX,
        5,
        0,
        77_000_003,
        u32::MAX - 1,
        9206,
        1,
        4_000_000_000,
    ]
    .into_iter()
    .chain((0..64u32).map(|index| index.wrapping_mul(2_654_435_761) | 1))
    .collect();
    let mut pair = [false, true].map(|lookup| Provinces {
        lookup_join: Some(lookup),
        sparse_install: Some(false),
        ..Default::default()
    });
    let mut naval = naval::History::default();
    let mut previous: Option<Targets> = None;
    let mut saw = [false; 4];
    for step in 0..96u32 {
        if step % 3 != 2 {
            let mut added: Vec<_> = (0..next() % 9)
                .map(|_| universe[next() as usize % universe.len()])
                .collect();
            if step == 17 {
                added.extend([u32::MAX, 0, u32::MAX - 1]);
            }
            for provinces in &mut pair {
                provinces.ensure(added.iter().copied());
            }
        }
        let placed: Vec<_> = if step % 11 == 7 {
            Vec::new()
        } else {
            (0..next() % 24)
                .map(|_| {
                    let province = universe[next() as usize % universe.len()];
                    let kind = if next().is_multiple_of(3) {
                        Kind::Ship
                    } else {
                        Kind::Land
                    };
                    Target {
                        identity: u64::from(next() % 5),
                        part: next() % 2,
                        order: next() % 38,
                        mesh: (next() % 6) as usize,
                        ..target(province, kind)
                    }
                })
                .collect()
        };
        let targets = Targets::from(placed);
        let candidates = (0..next() % 12)
            .map(|ordinal| {
                let province = universe[next() as usize % universe.len()];
                candidate(province, u64::from(next() % 7), ordinal)
            })
            .collect();
        naval.install(NavalSources {
            candidates,
            ..Default::default()
        });
        if step % 5 == 1 {
            naval.empty_candidates_for_test(universe[next() as usize % universe.len()]);
        }
        let joins = 1 + usize::from(step % 4 == 3);
        for _ in 0..joins {
            for provinces in &mut pair {
                provinces.prepare_inputs(&targets, &naval);
            }
            assert_same_join(&pair, &targets, &naval);
        }
        if let Some(previous) = previous.replace(targets.clone()) {
            for prepared in previous.0.provinces.values() {
                assert_eq!(Arc::strong_count(prepared), 1);
            }
        }
        let records = &pair[0].records;
        saw[0] |= records.iter().any(|record| record.targets.is_some());
        saw[1] |= records.iter().any(|record| record.has_candidates);
        saw[2] |= records
            .iter()
            .any(|record| record.targets.is_none() && !record.has_candidates);
        saw[3] |= records.first().is_some_and(|record| record.id == 0)
            && records.last().is_some_and(|record| record.id == u32::MAX);
    }
    assert_eq!(saw, [true; 4]);
}

#[test]
fn sparse_join_matches_the_full_walk_record_for_record() {
    let mut next = xorshift(0x51a7_e5ed);
    let half = |extremes: [u32; 4], range: std::ops::Range<u32>| -> Vec<u32> {
        extremes
            .into_iter()
            .chain(range.map(|index| index.wrapping_mul(2_654_435_761) | 1))
            .collect()
    };
    let halves = [
        half([u32::MAX, 5, 0, 77_000_003], 0..32),
        half([u32::MAX - 1, 9206, 1, 4_000_000_000], 32..64),
    ];
    let mut pair = [true, false].map(|sparse| Provinces {
        sparse_install: Some(sparse),
        lookup_join: Some(!sparse),
        ..Default::default()
    });
    let mut naval = naval::History::default();
    let mut previous: Option<Targets> = None;
    let mut saw = [false; 4];
    for step in 0..200u32 {
        let universe = &halves[(step / 23) as usize % 2];
        if step % 3 != 2 {
            let added: Vec<_> = (0..next() % 9)
                .map(|_| universe[next() as usize % universe.len()])
                .collect();
            for provinces in &mut pair {
                provinces.ensure(added.iter().copied());
            }
        }
        let empty = step % 37 == 36;
        let placed: Vec<_> = if empty {
            Vec::new()
        } else {
            (0..next() % 24)
                .map(|_| {
                    let province = universe[next() as usize % universe.len()];
                    Target {
                        identity: u64::from(next() % 5),
                        part: next() % 2,
                        order: next() % 38,
                        mesh: (next() % 6) as usize,
                        ..target(province, Kind::Land)
                    }
                })
                .collect()
        };
        let targets = Targets::from(placed);
        let candidates: Vec<_> = if empty {
            Vec::new()
        } else {
            (0..next() % 12)
                .map(|ordinal| {
                    let province = universe[next() as usize % universe.len()];
                    candidate(province, u64::from(next() % 7), ordinal)
                })
                .collect()
        };
        let named: Vec<_> = targets
            .0
            .provinces
            .keys()
            .copied()
            .chain(candidates.iter().map(|candidate| candidate.province))
            .collect();
        let unrecorded = step % 41 == 40;
        for provinces in &mut pair {
            provinces.ensure(named.iter().copied().skip(usize::from(unrecorded)));
        }
        naval.install(NavalSources {
            candidates,
            ..Default::default()
        });
        if step % 5 == 1 {
            naval.empty_candidates_for_test(universe[next() as usize % universe.len()]);
        }
        let joins = 1 + usize::from(step % 4 == 3);
        for _ in 0..joins {
            for provinces in &mut pair {
                provinces.prepare_inputs(&targets, &naval);
            }
            assert_same_join(&pair, &targets, &naval);
            assert!(
                pair[0]
                    .joined
                    .as_ref()
                    .is_some_and(|(joined, _)| Arc::ptr_eq(&joined.0, &targets.0))
            );
        }
        if let Some(previous) = previous.replace(targets.clone()) {
            for prepared in previous.0.provinces.values() {
                assert_eq!(Arc::strong_count(prepared), 1);
            }
        }
        let records = &pair[0].records;
        saw[0] |= records.iter().any(|record| record.targets.is_some());
        saw[1] |= records.iter().any(|record| record.has_candidates);
        saw[2] |= empty;
        saw[3] |= unrecorded && !named.is_empty();
    }
    assert_eq!(saw, [true; 4]);
}

fn assert_same_records(a: &History, b: &History) {
    assert_eq!(a.provinces.records.len(), b.provinces.records.len());
    for (a, b) in a.provinces.records.iter().zip(&b.provinces.records) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.has_candidates, b.has_candidates, "province {}", a.id);
        match (&a.targets, &b.targets) {
            (Some(joined), Some(reference)) => {
                assert!(Arc::ptr_eq(joined, reference), "province {}", a.id);
            }
            (None, None) => {}
            _ => panic!("province {} joined different targets", a.id),
        }
    }
}

fn assert_lookups(history: &History) {
    for record in &history.provinces.records {
        match (&record.targets, history.targets.0.provinces.get(&record.id)) {
            (Some(joined), Some(prepared)) => assert!(Arc::ptr_eq(joined, prepared)),
            (None, None) => {}
            _ => panic!("stale target join for province {}", record.id),
        }
        assert_eq!(
            record.has_candidates,
            history.naval.has_candidates(record.id),
            "province {}",
            record.id
        );
    }
}

fn hour_inputs(ids: &[u32], hour: u32) -> (Targets, NavalSources) {
    let mut placed = Vec::new();
    for (index, &id) in ids.iter().enumerate() {
        if hour % 9 == 8 || (index as u32 + hour) % 7 > 1 {
            continue;
        }
        for slot in 0..3u64 {
            let stack = Target {
                identity: u64::from(index as u32 + hour) * 4 + slot,
                order: (index as u32 + slot as u32) % 38,
                occupant: u64::from(id) * 4 + slot,
                mesh: (hour as usize + slot as usize) % 5,
                instance: ModelInstance {
                    position: [index as f32 * 3.0, 10.0, slot as f32],
                    scale: if slot == 1 { 0.3 } else { 1.0 },
                    animation_seed: id,
                    ..Default::default()
                },
                ..target(id, if slot < 2 { Kind::Land } else { Kind::Ship })
            };
            placed.push(stack);
            if index % 3 == 0 {
                placed.push(Target {
                    mesh: stack.mesh + 7,
                    ..stack
                });
            }
        }
    }
    let mut sources = NavalSources::default();
    if hour % 6 != 5 {
        for (index, &id) in ids.iter().enumerate() {
            if !(index as u32 + hour).is_multiple_of(19) {
                continue;
            }
            let mut occupant = candidate(id, index as u64 * 8 + 1, index as u32);
            sources
                .formations
                .push(formation(&occupant, hour as usize % 3));
            sources.candidates.push(occupant.clone());
            occupant.province = ids[(index + 1) % ids.len()];
            occupant.ordinal += 1_000;
            sources.candidates.push(occupant);
        }
        sources.candidates.push(candidate(
            u32::MAX - 10_000 - hour,
            9_000 + u64::from(hour),
            5_000,
        ));
    }
    (placed.into(), sources)
}

fn boxes(ids: &[u32], count: usize, shift: f32) -> Arc<BTreeMap<u32, [f32; 4]>> {
    Arc::new(
        ids.iter()
            .take(count)
            .enumerate()
            .map(|(index, &id)| {
                let x = (index % 16) as f32 * 96.0 + shift;
                let z = (index / 16) as f32 * 64.0;
                (id, [x, z, x + 48.0, z + 48.0])
            })
            .collect(),
    )
}

fn install(
    arms: &mut [History],
    targets: &Targets,
    sources: &NavalSources,
    bounds: &Arc<BTreeMap<u32, [f32; 4]>>,
    hour: u32,
) {
    for history in arms.iter_mut() {
        match hour % 3 {
            0 => {
                history.set_naval(sources.clone());
                history.set_prepared(targets.clone(), Arc::clone(bounds), false);
            }
            1 => {
                history.set_prepared(targets.clone(), Arc::clone(bounds), false);
                history.set_naval(sources.clone());
            }
            _ => history.set_prepared_with_naval(
                targets.clone(),
                Arc::clone(bounds),
                false,
                sources.clone(),
            ),
        }
        assert_lookups(history);
        if hour % 5 == 2 {
            let mut only_naval = sources.clone();
            only_naval
                .candidates
                .push(candidate(u32::MAX - 20_000 - hour, 7, 6_000));
            history.set_naval(only_naval);
            assert_lookups(history);
            history.set_naval(sources.clone());
            assert_lookups(history);
        }
    }
    for other in &arms[1..] {
        assert_same_records(&arms[0], other);
    }
}

fn step_all(
    arms: &mut [History],
    number: u32,
    roll: u32,
    time: f32,
    ids: &[u32],
) -> Vec<(usize, ModelInstance)> {
    let steady = number.is_multiple_of(79);
    let shown = [number % 37 < 25, number % 31 < 21];
    let arrows = number % 19 < 10;
    let limit = [-7, 0, 1, 2, 5, 17, 100][roll as usize % 7];
    let mut outputs = Vec::new();
    let mut traces = Vec::new();
    for history in arms.iter_mut() {
        history.steady = steady;
        if number.is_multiple_of(173) {
            history.reset(Some(&BTreeSet::from([ids[number as usize % ids.len()]])));
        }
        if number.is_multiple_of(227) {
            history.restart_live();
            history.steady = steady;
        }
        history.select_provinces(if number % 11 < 5 {
            BTreeSet::from([ids[0], ids[(number as usize / 3) % ids.len()]])
        } else {
            BTreeSet::new()
        });
        history.set_selection(&BTreeSet::from([Unit::TaskForce(
            u64::from(number % 150) * 8 + 1,
        )]));
        let mut budget = Budget::new(limit, steady);
        let mut trace = Vec::new();
        outputs.push(history.step_with(
            time,
            GATES,
            shown,
            arrows,
            |id| id == 0 || (id ^ roll).is_multiple_of(3),
            &mut budget,
            |id, budget| {
                if (id ^ roll).is_multiple_of(7) {
                    budget.take();
                }
                trace.push((id, budget.remaining()));
            },
        ));
        trace.push((u32::MAX, budget.remaining()));
        traces.push(trace);
    }
    for arm in 1..arms.len() {
        assert_eq!(
            traces[0], traces[arm],
            "frame {number} arm {arm} building and budget order"
        );
        same_history::assert_instances(&outputs[0], &outputs[arm]);
        same_history::assert_history(&arms[0], &arms[arm]);
        assert_same_records(&arms[0], &arms[arm]);
    }
    outputs.swap_remove(0)
}

fn has_formations(history: &History) -> bool {
    history.provinces.records.iter().any(|record| {
        record.has_candidates
            && record
                .province
                .entities
                .keys()
                .any(|&(kind, identity)| kind == Kind::Ship && identity % 8 == 1)
    })
}

#[test]
fn merge_join_matches_point_lookups_across_installs_views_and_frames() {
    let ids: Vec<u32> = [0, 1, 2]
        .into_iter()
        .chain((0..150u32).map(|index| index * 28_657 + 3))
        .chain([4_000_000_011, u32::MAX - 1, u32::MAX])
        .collect();
    let mut views: [[History; 2]; 2] = std::array::from_fn(|_| {
        [false, true].map(|lookup| {
            let mut history = History::default();
            history.provinces.lookup_join = Some(lookup);
            history.provinces.sparse_install = Some(false);
            history
        })
    });
    let mut bounds = boxes(&ids, 120, 0.0);
    let mut next = xorshift(0x0b5e_7a11);
    let mut time = 0.0;
    let mut saw = [false; 6];
    for number in 0..1_200u32 {
        let roll = next();
        let hour = number / 13;
        if number.is_multiple_of(13) {
            if hour.is_multiple_of(4) {
                bounds = boxes(&ids, 120 + hour as usize % 37, (hour % 3) as f32);
            }
            let (targets, sources) = hour_inputs(&ids, hour);
            for (view, pair) in views.iter_mut().enumerate() {
                let installed = if view == 1 || hour % 4 == 2 {
                    Arc::default()
                } else {
                    Arc::clone(&bounds)
                };
                install(pair, &targets, &sources, &installed, hour);
            }
            for (id, prepared) in &targets.0.provinces {
                assert_eq!(Arc::strong_count(prepared), 5, "province {id}");
            }
            let wide = &views[0][0].provinces.records;
            saw[0] |= targets.0.provinces.is_empty() && !wide.is_empty();
            saw[1] |= views[1][0].provinces.records.len() < wide.len();
            saw[2] |= wide.first().is_some_and(|record| record.id == 0)
                && wide.last().is_some_and(|record| record.id == u32::MAX);
            saw[3] |= wide
                .iter()
                .any(|record| record.has_candidates && record.id > u32::MAX - 30_000);
        }
        if number == 617 {
            time = 0.0;
        }
        time += [0.0, 1.0 / 240.0, 0.125, 0.5][(roll as usize >> 3) % 4];
        for pair in &mut views {
            let placed = step_all(pair, number, roll, time, &ids);
            saw[4] |= !placed.is_empty();
            saw[5] |= has_formations(&pair[0]);
            for history in pair.iter() {
                let bound = 18 * (history.provinces.records.len() + 1) + 36;
                let slots = history.schedule.lock().unwrap().storage_slots();
                assert!(
                    slots <= bound,
                    "frame {number}: schedule storage {slots} exceeded bound {bound}"
                );
            }
        }
    }
    assert_eq!(saw, [true; 6]);
}

#[test]
fn sparse_install_matches_the_full_walk_across_seeks_branches_views_and_frames() {
    let branches: [Vec<u32>; 2] = [
        [0, 1, 2]
            .into_iter()
            .chain((0..150u32).map(|index| index * 28_657 + 3))
            .chain([4_000_000_011, u32::MAX - 1, u32::MAX])
            .collect(),
        (0..140u32)
            .map(|index| index * 31_337 + 11)
            .chain([u32::MAX])
            .collect(),
    ];
    let arms = || {
        [(true, false), (false, false), (false, true)].map(|(sparse, lookup)| {
            let mut history = History::default();
            history.provinces.sparse_install = Some(sparse);
            history.provinces.lookup_join = Some(lookup);
            history
        })
    };
    let mut views = [arms(), arms()];
    let mut bounds = boxes(&branches[0], 120, 0.0);
    let mut next = xorshift(0x5eed_b4a7);
    let mut time = 0.0;
    let mut saw = [false; 6];
    for number in 0..1_200u32 {
        let roll = next();
        let hour = number / 13;
        let ids = &branches[(hour / 9) as usize % 2];
        if number.is_multiple_of(13) {
            let branch_change = hour > 0 && hour.is_multiple_of(9);
            let seek = !branch_change && hour % 5 == 3;
            if branch_change || hour.is_multiple_of(4) {
                bounds = boxes(ids, ids.len() - hour as usize % 23, (hour % 3) as f32);
            }
            let (targets, sources) = hour_inputs(ids, if seek { hour + 97 } else { hour });
            for (view, arms) in views.iter_mut().enumerate() {
                if branch_change || seek {
                    for history in arms.iter_mut() {
                        history.reset(None);
                    }
                }
                let installed = if view == 1 || hour % 4 == 2 {
                    Arc::default()
                } else {
                    Arc::clone(&bounds)
                };
                install(arms, &targets, &sources, &installed, hour);
            }
            for (id, prepared) in &targets.0.provinces {
                for arms in &views {
                    for history in arms {
                        let at = history.provinces.indices[id];
                        let held = history.provinces.records[at].targets.as_ref();
                        assert!(
                            held.is_some_and(|held| Arc::ptr_eq(held, prepared)
                                || **held == **prepared),
                            "province {id}"
                        );
                    }
                }
            }
            saw[0] |= branch_change;
            saw[1] |= seek;
            saw[2] |= views[0][0]
                .provinces
                .joined
                .as_ref()
                .is_some_and(|(joined, _)| Arc::ptr_eq(&joined.0, &targets.0));
            saw[3] |= branch_change
                && [&branches[0], &branches[1]]
                    .iter()
                    .all(|branch| views[0][0].provinces.indices.contains_key(&branch[5]));
            if branch_change || seek {
                time = 0.0;
            }
        }
        time += [0.0, 1.0 / 240.0, 0.125, 0.5][(roll as usize >> 3) % 4];
        for arms in &mut views {
            let placed = step_all(arms, number, roll, time, ids);
            saw[4] |= !placed.is_empty();
            saw[5] |= has_formations(&arms[0]);
            for history in arms.iter() {
                let bound = 18 * (history.provinces.records.len() + 1) + 36;
                let slots = history.schedule.lock().unwrap().storage_slots();
                assert!(
                    slots <= bound,
                    "frame {number}: schedule storage {slots} exceeded bound {bound}"
                );
            }
        }
    }
    assert_eq!(saw, [true; 6]);
}

#[test]
fn deferred_land_allowance_keeps_six_then_nine_at_one_time_under_every_join() {
    for (sparse, lookup) in [(true, false), (false, false), (false, true)] {
        let targets = [1, 2, 3]
            .into_iter()
            .flat_map(|province| {
                (0..3).map(move |order| Target {
                    order,
                    identity: u64::from(province * 10 + order),
                    mesh: (province * 10 + order) as usize,
                    ..target(province, Kind::Land)
                })
            })
            .collect();
        let mut history = History::default();
        history.provinces.lookup_join = Some(lookup);
        history.provinces.sparse_install = Some(sparse);
        history.set(targets, Arc::default(), false);
        let mut limited = || {
            history.step_with(
                0.0,
                GATES,
                [true; 2],
                true,
                |_| true,
                &mut Budget::new(2, false),
                |_, _| {},
            )
        };
        assert_eq!(
            limited().iter().map(|placed| placed.0).collect::<Vec<_>>(),
            [10, 11, 20, 21, 30, 31],
            "sparse {sparse} lookup {lookup}"
        );
        assert_eq!(limited().len(), 9, "same timestamp is still a new frame");
    }
}
