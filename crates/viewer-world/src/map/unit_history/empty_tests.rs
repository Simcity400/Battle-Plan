use super::{test_support::GATES, test_support::target, *};
use viewer_foundation::{
    map::camera::{MapDefines, MapSize, Placement},
    unit_class::CounterClass,
};

fn histories(bounds: &Arc<BTreeMap<u32, [f32; 4]>>) -> [History; 2] {
    [true, false].map(|fast| {
        let mut history = History {
            empty_fast_path: Some(fast),
            ..Default::default()
        };
        history.set(Vec::new(), Arc::clone(bounds), false);
        history
    })
}

pub(super) fn assert_instances(a: &[(usize, ModelInstance)], b: &[(usize, ModelInstance)]) {
    assert_eq!(a.len(), b.len());
    for ((a_mesh, a), (b_mesh, b)) in a.iter().zip(b) {
        assert_eq!(a_mesh, b_mesh);
        assert_eq!(bytemuck::bytes_of(a), bytemuck::bytes_of(b));
    }
}

pub(super) fn assert_history(a: &History, b: &History) {
    assert_eq!(a.targets, b.targets);
    assert_eq!(a.bounds, b.bounds);
    assert_eq!(a.steady, b.steady);
    assert_instances(&a.raids, &b.raids);
    assert_eq!(a.provinces, b.provinces);
    assert_eq!(a.provinces.populated_ids(), b.provinces.populated_ids());
    assert_eq!(a.updates, b.updates);
    assert_eq!(a.queued, b.queued);
    assert_eq!(a.queue_trace, b.queue_trace);
    assert_eq!(a.epoch, b.epoch);
    assert_eq!(a.last.map(f32::to_bits), b.last.map(f32::to_bits));
    assert_eq!(a.raid_epoch, b.raid_epoch);
    assert_eq!(a.province_bounds, b.province_bounds);
    a.naval.assert_same_state(&b.naval);
    assert_eq!(a.combat_clocks(), b.combat_clocks());
    assert_eq!(a.combat_warnings(), b.combat_warnings());
    let a_schedule = a.schedule.lock().unwrap();
    let b_schedule = b.schedule.lock().unwrap();
    assert_eq!(
        a_schedule.presentation_state(),
        b_schedule.presentation_state()
    );
    assert_eq!(a_schedule.pending(), b_schedule.pending());
    for &id in a.provinces.keys() {
        assert_eq!(a_schedule.update(id), b_schedule.update(id));
        assert_eq!(a_schedule.dirty(id), b_schedule.dirty(id));
    }
}

#[test]
fn empty_visits_keep_budget_held_show_bits_for_later_arrivals() {
    let bounds = Arc::new(BTreeMap::from([
        (1, [0.0, 0.0, 1.0, 1.0]),
        (2, [2.0, 0.0, 3.0, 1.0]),
    ]));
    let mut pair = histories(&bounds);
    for history in &mut pair {
        let province = history.provinces.get_mut(&1).unwrap();
        province.land_cursor = 731;
        province.land_visits.insert((Kind::Land, 99), 17);
        let mut budget = Budget::new(1, false);
        assert!(
            history
                .step_with(
                    0.0,
                    GATES,
                    [true; 2],
                    true,
                    |_| true,
                    &mut budget,
                    |id, budget| {
                        if id == 2 {
                            budget.take();
                        }
                    },
                )
                .is_empty()
        );
        assert_eq!(budget.remaining(), 0);
        assert_eq!(history.provinces[&1].shown, [true; 2]);
        assert_eq!(history.provinces[&2].shown, [false; 2]);
        assert!(history.provinces[&1].arrows);
        assert!(history.provinces[&1].land_visits.is_empty());
        assert_eq!(history.provinces[&1].land_cursor, 731);
        history.set(
            vec![target(1, Kind::Land), target(2, Kind::Land)],
            Arc::clone(&bounds),
            false,
        );
    }
    let output = pair.each_mut().map(|history| {
        history.step_with(
            0.25,
            GATES,
            [true; 2],
            false,
            |_| true,
            &mut Budget::new(8, false),
            |_, _| {},
        )
    });
    assert_instances(&output[0], &output[1]);
    assert_eq!(
        output[0][0].1.scale, 1.0,
        "arrival into an already shown empty province"
    );
    assert_eq!(
        output[0][1].1.scale, 0.0,
        "spent budget kept the other province newly shown"
    );
    assert_history(&pair[0], &pair[1]);
}

#[test]
fn empty_fast_path_matches_full_walk_across_camera_motion_and_hour_installs() {
    let ids: Vec<_> = (0..320u32).map(|i| i * 17 + 1).collect();
    let bounds = Arc::new(
        ids.iter()
            .enumerate()
            .map(|(index, &id)| {
                let x = (index % 16) as f32 * 96.0 + 48.0;
                let z = (index / 16) as f32 * 64.0 + 32.0;
                (id, [x - 24.0, z - 24.0, x + 24.0, z + 24.0])
            })
            .collect(),
    );
    let mut pair = histories(&bounds);
    let mut time = 0.0;
    let mut in_view_visits = 0;
    let mut off_screen_visits = 0;
    let mut empty_visits = 0;
    for number in 0..720u32 {
        let hour = number / 13;
        if number % 13 == 0 {
            let targets: Vec<_> = ids
                .iter()
                .enumerate()
                .filter_map(|(index, &id)| {
                    let remainder = (index as u32 + hour) % 23;
                    if remainder >= 2 {
                        return None;
                    }
                    Some(Target {
                        identity: u64::from(id) * 10,
                        order: (index as u32 + hour) % 7,
                        kind: if remainder == 0 {
                            Kind::Land
                        } else {
                            Kind::Ship
                        },
                        mesh: hour as usize % 4,
                        instance: ModelInstance {
                            position: [bounds[&id][0], 10.0, bounds[&id][1]],
                            rotation: -0.0,
                            scale: 1.0,
                            animation_seed: id,
                            ..Default::default()
                        },
                        ..target(id, Kind::Land)
                    })
                })
                .collect();
            let mut naval = NavalSources::default();
            for (index, &id) in ids.iter().enumerate() {
                if !(index as u32 + hour).is_multiple_of(31) {
                    continue;
                }
                let class = if index % 2 == 0 {
                    CounterClass::TaskForce
                } else {
                    CounterClass::Division
                };
                let identity = u64::from(id) * 10 + 1;
                let slot = index as u32 % 7;
                naval.candidates.push(naval::Candidate {
                    province: id,
                    identity,
                    ordinal: index as u32,
                    slot,
                    priority: 3,
                    ours: true,
                    class,
                    land_eligible: true,
                    sample_count: 8,
                    next_arc: 0.5,
                    route: vec![id, id + 17],
                    speed: 0.25,
                    min_arc: 0.0,
                    arrival_allowed: true,
                    day: hour as i32,
                });
                if class == CounterClass::TaskForce {
                    naval.formations.push(naval::Formation {
                        identity,
                        province: id,
                        slot,
                        roster: vec![naval::Hull {
                            flags: 0x400040,
                            graphics: b"synthetic_hull".to_vec(),
                            targets: [
                                Some(Target {
                                    identity,
                                    mesh: hour as usize % 3,
                                    ..target(id, Kind::Ship)
                                }),
                                None,
                                None,
                                None,
                            ],
                        }],
                    });
                }
            }
            for history in &mut pair {
                history.set(targets.clone(), Arc::clone(&bounds), false);
                history.set_naval(naval.clone());
            }
        }
        if number == 360 {
            time = 0.0;
        }
        time += if number % 29 == 0 { 0.0 } else { 0.03125 };
        let camera = Placement::map(
            [
                ((number / 5) % 16) as f32 * 96.0 + 48.0,
                ((number / 23) % 20) as f32 * 64.0 + 32.0,
            ],
            [60.0, 150.0, 260.0, 350.0][(number as usize / 17) % 4],
        )
        .on(
            MapSize {
                width: 4000,
                height: 1800,
            },
            MapDefines::test(),
        );
        let viewport = if number % 2 == 0 {
            [1024, 768]
        } else {
            [1600, 900]
        };
        let admission = Admission::new(camera, viewport);
        let shown = [GATES.units(camera.height), GATES.ships(camera.height)];
        let arrows = camera.height < GATES.arrows;
        let steady = number % 79 == 0;
        let limit = [-7, 0, 1, 2, 5, 17, 100][number as usize % 7];
        let mut output = Vec::new();
        let mut traces = Vec::new();
        for history in &mut pair {
            history.steady = steady;
            if number % 173 == 0 {
                history.reset(Some(&BTreeSet::from([ids[number as usize % ids.len()]])));
            }
            if number % 227 == 0 {
                history.restart_live();
                history.steady = steady;
            }
            history.select_provinces(if number % 11 < 5 {
                BTreeSet::from([ids[(number as usize / 3) % ids.len()]])
            } else {
                BTreeSet::new()
            });
            history.set_selection(&BTreeSet::from([Unit::TaskForce(
                u64::from(ids[number as usize % ids.len()]) * 10 + 1,
            )]));
            if number % 3 == 0 {
                history.advance_schedule(camera, viewport);
            }
            let mut budget = Budget::new(limit, steady);
            let mut trace = Vec::new();
            output.push(history.step_with(
                time,
                GATES,
                shown,
                arrows,
                |id| admission.admits_province(&bounds, id),
                &mut budget,
                |id, budget| {
                    if id % 19 == number % 19 {
                        budget.take();
                    }
                    trace.push((id, budget.remaining()));
                },
            ));
            trace.push((u32::MAX, budget.remaining()));
            traces.push(trace);
        }
        assert_eq!(
            traces[0], traces[1],
            "frame {number} building and budget order"
        );
        assert_instances(&output[0], &output[1]);
        assert_history(&pair[0], &pair[1]);
        for &(id, update) in &pair[0].updates {
            match update {
                Update::InView => in_view_visits += 1,
                Update::OffScreen => off_screen_visits += 1,
            }
            if pair[0].provinces[&id].entities.is_empty() {
                empty_visits += 1;
            }
        }
    }
    assert!(in_view_visits > 100 && off_screen_visits > 100 && empty_visits > 100);
}
