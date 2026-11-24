use super::*;

pub fn date(year: i32, days: i32, hour: i32) -> i32 {
    EPOCH + year * HOURS_PER_YEAR + days * 24 + hour
}

#[test]
fn the_roll_matches_independent_hash_expectations() {
    for (input, expected) in [
        (0, 42_299),
        (1, 96_854),
        (2, 13_016),
        (12_345, 50_529),
        (u32::MAX, 5_716),
        (0xdead_beef, 8_450),
    ] {
        assert_eq!(roll(input), expected, "{input:#x}");
    }
}

#[test]
fn the_window_wraps_stops_on_its_start_and_visits_at_least_one_slot() {
    assert_eq!(window(3, 3, 5), Ok((vec![3, 4, 0], 1)));
    assert_eq!(window(4, 2, 5), Ok((vec![4, 0], 1)));
    assert_eq!(window(2, 9, 5), Ok((vec![2, 3, 4, 0, 1], 2)));
    assert_eq!(window(1, 0, 5), Ok((vec![1], 2)));
    assert_eq!(window(1, -4, 5), Ok((vec![1], 2)));
    assert_eq!(window(5, 3, 5), Ok((vec![], 5)));
    assert!(window(-1, 3, 5).is_err());
}

pub fn content() -> Content {
    let year = |region| {
        (
            region,
            Ok(vec![Period {
                start: PERIOD_DEFAULT_DATE,
                end: PERIOD_DEFAULT_DATE + 364 * 24,
                minimum: match region {
                    10 => -1_000_000,
                    20 => 500_000,
                    _ => 0,
                },
                maximum: match region {
                    10 => 1_500_000,
                    20 => 900_000,
                    _ => 100,
                },
            }]),
        )
    };
    let province = |active, neighbours: &[u32], region, terrain| Province {
        active,
        neighbours: neighbours.to_vec(),
        region,
        terrain: Ok(terrain),
    };
    Content {
        provinces: vec![
            province(false, &[], None, None),
            province(true, &[2, 3, 0], Some(10), Some(5)),
            province(true, &[1, 3], Some(20), Some(2)),
            province(true, &[], Some(30), None),
            province(true, &[3, 1, 1], None, None),
        ],
        periods: [year(10), year(20), year(30)].into_iter().collect(),
        settings: Settings {
            variation: 200_000,
            max_change: 600_000,
            smoothing: 30_000,
            change_smoothing: 50_000,
            provinces_per_update: 3,
            terrain: vec![
                TerrainModifier {
                    terrain: 5,
                    value: -700_000,
                },
                TerrainModifier {
                    terrain: 2,
                    value: 300_000,
                },
            ],
        },
    }
}

fn state(temperature: i64, drift: i64, staged: i64, working: i64) -> State {
    State {
        temperature,
        drift,
        staged,
        working,
    }
}

#[test]
fn an_update_stages_every_window_province_before_committing_any() {
    let content = content();
    let hour = date(1936, 100, 7);
    let mut simulation = Simulation::new(
        hour - 1,
        1,
        vec![
            state(0, 0, 0, 0),
            state(1_234_567, 250_000, 1_234_567, 250_000),
            state(800_000, -120_000, 800_000, -120_000),
            state(-50_000, 600_000, 999, 7),
            state(2_000_000, 10, 1_500_000, -40_000),
        ],
    );
    let seed = 0x9e37_79b9_u32;
    simulation.advance(&content, Some(seed)).unwrap();
    assert_eq!(simulation.hour, hour);
    assert_eq!(simulation.cursor, 4);
    assert_eq!(
        simulation.states,
        [
            state(0, 0, 0, 0),
            state(1_157_465, 120_000, 1_157_465, 120_000),
            state(618_397, 212_500, 618_397, 212_500),
            state(-50_000, 600_000, 87_991, 0),
            state(2_000_000, 10, 1_500_000, -40_000),
        ]
    );
    let mut suspended = simulation.clone();
    suspended.advance(&content, None).unwrap();
    assert_eq!(suspended.states, simulation.states);
    assert_eq!(suspended.cursor, 4);
    simulation
        .update(&content, hour + 1, seed.wrapping_add(0x1234_5678))
        .unwrap();
    assert_eq!(simulation.cursor, 2);
    assert_eq!(
        simulation.states[1],
        state(864_797, 181_205, 864_797, 181_205)
    );
    assert_eq!(
        simulation.states[4],
        state(1_276_493, 120_000, 1_276_493, 120_000)
    );
}

#[test]
fn unsettled_content_refuses_only_where_the_pass_reads_it() {
    let mut content = content();
    let states = vec![State::default(); 5];
    content.provinces[4].terrain = Err("terrain unknown".into());
    let mut simulation = Simulation::new(0, 3, states.clone());
    simulation.update(&content, date(1936, 1, 0), 1).unwrap();
    content.provinces[1].terrain = Err("terrain unknown".into());
    let mut simulation = Simulation::new(0, 1, states.clone());
    assert!(
        simulation
            .update(&content, date(1936, 1, 0), 1)
            .unwrap_err()
            .contains("weather province 1")
    );
    content.provinces[1].terrain = Ok(None);
    content.periods.insert(20, Err("bound unported".into()));
    let mut simulation = Simulation::new(0, 1, states.clone());
    assert!(
        simulation
            .update(&content, date(1936, 1, 0), 1)
            .unwrap_err()
            .contains("region 20: bound unported")
    );
    content.provinces.pop();
    assert!(
        Simulation::new(0, 0, states)
            .update(&content, 0, 1)
            .is_err()
    );
}

#[test]
fn the_witness_sums_each_slots_term() {
    assert_eq!(witness([]), 0);
    assert_eq!(witness([0]), 0x4821_8226_ff3c_d4bf);
    assert_eq!(witness([0, 100_000, -1]), 0x6c78_0239_d499_3e98);
    assert_eq!(province_witness(13_412, -2_000_000), 0xd331_14c6_035a_7339);
    assert_ne!(witness([1, 0]), witness([0, 1]));
    let before = [5, -6, 7_000_000, 0];
    let mut after = before;
    after[2] = -7_000_000;
    assert_eq!(
        witness(after),
        witness(before)
            .wrapping_sub(province_witness(2, 7_000_000))
            .wrapping_add(province_witness(2, -7_000_000))
    );
}

fn rows(states: &[State], pass: Option<crate::TemperaturePass>) -> Vec<WeatherProvinceRow> {
    states
        .iter()
        .enumerate()
        .map(|(slot, &state)| WeatherProvinceRow {
            id: slot as u32,
            temperature: crate::ProvinceTemperature::anchored(state),
            temperature_pass: Recorded::Value(if slot == 0 { pass.clone() } else { None }),
            ..Default::default()
        })
        .collect()
}

#[test]
fn frames_derive_from_their_anchor_and_must_match_the_recorded_witness() {
    let content = content();
    let hour = date(1936, 100, 7);
    let anchored = [
        state(0, 0, 0, 0),
        state(1_234_567, 250_000, 1_234_567, 250_000),
        state(800_000, -120_000, 800_000, -120_000),
        state(-50_000, 600_000, 999, 7),
        state(2_000_000, 10, 1_500_000, -40_000),
    ];
    let seed = 0x9e37_79b9_u32;
    let derived = [0, 1_157_465, 618_397, -50_000, 2_000_000];
    let pass = |hours: Vec<Option<u32>>, witness| crate::TemperaturePass {
        anchor: hour - 1,
        cursor: 1,
        anchor_seed: None,
        hours,
        witness: crate::TemperatureWitness::new(witness),
    };
    let mut memo = Memo::default();
    let frame = rows(
        &anchored,
        Some(pass(vec![Some(seed)], super::witness(derived))),
    );
    let states = memo.states(1, hour, &frame, Ok(&content)).unwrap();
    assert_eq!(
        states.iter().map(|s| s.temperature).collect::<Vec<_>>(),
        derived
    );
    let frame = rows(
        &anchored,
        Some(pass(vec![Some(seed), None], super::witness(derived))),
    );
    memo.states(1, hour + 1, &frame, Ok(&content)).unwrap();
    assert_eq!(memo.simulation().unwrap().hour, hour + 1);
    let wrong = rows(
        &anchored,
        Some(pass(vec![Some(seed ^ 1)], super::witness(derived))),
    );
    assert!(
        Memo::default()
            .states(1, hour, &wrong, Ok(&content))
            .unwrap_err()
            .contains("differ from the recorded witness")
    );
    let short = rows(&anchored, Some(pass(vec![], super::witness(derived))));
    assert!(
        Memo::default()
            .states(1, hour, &short, Ok(&content))
            .is_err()
    );
    let refused = rows(&anchored, None);
    assert!(
        Memo::default()
            .states(1, hour, &refused, Ok(&content))
            .is_err()
    );
    let frame = rows(
        &anchored,
        Some(pass(vec![Some(seed)], super::witness(derived))),
    );
    assert!(
        Memo::default()
            .states(1, hour, &frame, Err("unread"))
            .is_err()
    );
    let at_anchor = rows(&anchored, Some(pass(vec![], 0)));
    let mut anchor_pass = pass(vec![], super::witness(anchored.map(|s| s.temperature)));
    anchor_pass.anchor = hour;
    let at_anchor = rows(&anchored, Some(anchor_pass))
        .into_iter()
        .chain(at_anchor.into_iter().skip(5))
        .collect::<Vec<_>>();
    assert_eq!(
        Memo::default()
            .states(1, hour, &at_anchor, Ok(&content))
            .unwrap(),
        anchored
    );
}

#[test]
fn passes_without_a_witness_refuse_by_name() {
    let content = content();
    let hour = date(1936, 100, 7);
    let anchored = [state(0, 0, 0, 0); 5];
    let frame = rows(
        &anchored,
        Some(crate::TemperaturePass {
            anchor: hour,
            cursor: 1,
            anchor_seed: None,
            hours: vec![],
            witness: crate::TemperatureWitness::default(),
        }),
    );
    let reason = Memo::default()
        .states(1, hour, &frame, Ok(&content))
        .unwrap_err();
    assert!(
        reason.contains("predates the province temperature witness"),
        "{reason}"
    );
}

#[test]
fn hourly_temperatures_of_older_formats_need_no_content() {
    let frame: Vec<WeatherProvinceRow> = [5, -6]
        .into_iter()
        .enumerate()
        .map(|(slot, temperature)| WeatherProvinceRow {
            id: slot as u32,
            temperature: crate::ProvinceTemperature::historical(temperature),
            temperature_pass: Recorded::NotRecorded,
            ..Default::default()
        })
        .collect();
    let states = Memo::default()
        .states(1, 0, &frame, Err("no content"))
        .unwrap()
        .to_vec();
    assert_eq!(
        states.iter().map(|s| s.temperature).collect::<Vec<_>>(),
        [5, -6]
    );
}

#[test]
fn the_window_span_counts_what_the_window_lists() {
    for count in [1, 2, 5, 13] {
        for cursor in -1..=count {
            for width in [i32::MIN, i32::MIN + 1, -3, 0, 1, 2, 4, 5, 13, 40, i32::MAX] {
                let listed =
                    window(cursor, width, count).map(|(slots, next)| (slots.len() as i32, next));
                assert_eq!(window_span(cursor, width, count), listed);
            }
        }
    }
}

#[test]
fn held_derivations_resume_exactly_in_any_order() {
    let content = content();
    let anchor = date(1936, 100, 7);
    let anchored = [
        state(0, 0, 0, 0),
        state(1_234_567, 250_000, 1_234_567, 250_000),
        state(800_000, -120_000, 800_000, -120_000),
        state(-50_000, 600_000, 999, 7),
        state(2_000_000, 10, 1_500_000, -40_000),
    ];
    let seeds: Vec<Option<u32>> = (0..12u32)
        .map(|at| (at % 5 != 3).then(|| 0x9e37_79b9_u32.wrapping_mul(at + 1)))
        .collect();
    let mut reference = vec![Simulation::new(anchor, 1, anchored.to_vec())];
    for &seed in &seeds {
        let mut next = reference.last().unwrap().clone();
        next.advance(&content, seed).unwrap();
        reference.push(next);
    }
    let frame = |steps: usize, witness: u64| {
        rows(
            &anchored,
            Some(crate::TemperaturePass {
                anchor,
                cursor: 1,
                anchor_seed: None,
                hours: seeds[..steps].to_vec(),
                witness: crate::TemperatureWitness::new(witness),
            }),
        )
    };
    let derive = |memo: &mut Memo, steps: usize, witness: u64| {
        memo.states(
            1,
            anchor + steps as i32,
            &frame(steps, witness),
            Ok(&content),
        )
        .map(<[State]>::to_vec)
    };
    let mut plain = Memo::default();
    for steps in (0..=12).chain([5, 12, 0]) {
        let right = reference[steps].witness();
        assert_eq!(
            derive(&mut plain, steps, right),
            Ok(reference[steps].states.clone())
        );
        assert_eq!((plain.held(), plain.held_bytes()), (0, 0));
    }
    let mut memo = Memo::holding();
    for steps in [3, 10, 7, 12, 0, 1, 11, 6, 6, 5, 9, 2] {
        let right = reference[steps].witness();
        let expected = Ok(reference[steps].states.clone());
        assert_eq!(derive(&mut memo, steps, right), expected, "{steps}");
        assert_eq!(derive(&mut Memo::default(), steps, right), expected);
        let wrong = right ^ 1;
        assert_eq!(
            derive(&mut memo, steps, wrong),
            derive(&mut Memo::default(), steps, wrong)
        );
    }
    memo.retain_held(|branch, hour| branch == 1 && (hour - anchor) % 2 == 0);
    assert!(memo.held.iter().all(|held| (held.hour - anchor) % 2 == 0));
    let window = 3;
    for steps in (0..=12).rev().chain(0..=12) {
        let right = reference[steps].witness();
        assert_eq!(
            derive(&mut memo, steps, right),
            Ok(reference[steps].states.clone())
        );
        let alive = |_: u64, hour: i32| (hour - anchor - steps as i32).abs() < window;
        memo.retain_held(alive);
        assert!(memo.held() < 2 * window as usize);
        assert_eq!(
            memo.held_bytes(),
            memo.held() * anchored.len() * std::mem::size_of::<State>()
        );
    }
    memo.retain_held(|_, _| false);
    assert_eq!((memo.held(), memo.held_bytes()), (0, 0));
    assert_eq!(memo.simulation(), reference.last());
    let right = reference[4].witness();
    let other = memo
        .states(2, anchor + 4, &frame(4, right), Ok(&content))
        .map(<[State]>::to_vec);
    assert_eq!(other, Ok(reference[4].states.clone()));
    assert_eq!(
        memo.states(2, anchor + 4, &frame(4, right), Err("unread"))
            .map(<[State]>::to_vec),
        Memo::default()
            .states(2, anchor + 4, &frame(4, right), Err("unread"))
            .map(<[State]>::to_vec)
    );
    let shared = std::sync::Mutex::new(Memo::holding());
    std::thread::scope(|scope| {
        for first in 0..4 {
            let (shared, reference, derive) = (&shared, &reference, &derive);
            scope.spawn(move || {
                for steps in (first..13).step_by(4).rev() {
                    let mut memo = shared.lock().unwrap();
                    let right = reference[steps].witness();
                    assert_eq!(
                        derive(&mut memo, steps, right),
                        Ok(reference[steps].states.clone())
                    );
                }
            });
        }
    });
}

#[test]
fn held_derivations_require_the_same_branch_anchor_cursor_words_and_seed_prefix() {
    let content = content();
    let anchor = date(1936, 100, 7);
    let seeds = [Some(10), None, Some(20), Some(30)];
    let different = [Some(10), Some(99), Some(20), Some(30)];
    let mut memo = Memo::holding();
    for (branch, shift, cursor, temperature, hours) in [
        (1, 0, 1, 100, &seeds[..2]),
        (1, 0, 1, 100, &seeds[..]),
        (1, 0, 1, 100, &different[..]),
        (1, 0, 1, 100, &seeds[..3]),
        (2, 0, 1, 100, &seeds[..]),
        (2, 3, 1, 100, &seeds[..]),
        (2, 3, 2, 100, &seeds[..]),
        (2, 3, 2, 900, &seeds[..]),
    ] {
        let anchored = vec![state(temperature, 0, temperature, 0); content.provinces.len()];
        let mut reference = Simulation::new(anchor + shift, cursor, anchored.clone());
        for &seed in hours {
            reference.advance(&content, seed).unwrap();
        }
        let frame = rows(
            &anchored,
            Some(crate::TemperaturePass {
                anchor: anchor + shift,
                cursor,
                anchor_seed: None,
                hours: hours.to_vec(),
                witness: crate::TemperatureWitness::new(reference.witness()),
            }),
        );
        assert_eq!(
            memo.states(branch, reference.hour, &frame, Ok(&content))
                .unwrap(),
            reference.states,
        );
        assert_eq!(memo.simulation(), Some(&reference));
        for held in &memo.held {
            let mut prefix = Simulation::new(anchor + shift, cursor, anchored.clone());
            for &seed in &hours[..(held.hour - anchor - shift) as usize] {
                prefix.advance(&content, seed).unwrap();
            }
            assert_eq!(held, &prefix);
        }
    }
}

#[test]
fn an_advance_refusal_clears_held_derivations_and_allows_a_fresh_retry() {
    let content = content();
    let anchor = date(1936, 100, 7);
    let anchored = vec![state(100, 0, 100, 0); content.provinces.len()];
    let mut memo = Memo::holding();
    let initial = rows(
        &anchored,
        Some(crate::TemperaturePass {
            anchor,
            cursor: 1,
            anchor_seed: None,
            hours: vec![],
            witness: crate::TemperatureWitness::new(witness(
                anchored.iter().map(|state| state.temperature),
            )),
        }),
    );
    memo.states(1, anchor, &initial, Ok(&content)).unwrap();
    assert_eq!(memo.held(), 1);
    let mut broken = self::content();
    broken.provinces[1].terrain = Err("synthetic unreadable terrain".into());
    let frame = rows(
        &anchored,
        Some(crate::TemperaturePass {
            anchor,
            cursor: 1,
            anchor_seed: None,
            hours: vec![Some(10)],
            witness: crate::TemperatureWitness::new(0),
        }),
    );
    let mut reference = Simulation::new(anchor, 1, anchored.clone());
    assert_eq!(
        memo.states(1, anchor + 1, &frame, Ok(&broken)).unwrap_err(),
        reference.advance(&broken, Some(10)).unwrap_err(),
    );
    assert!(memo.simulation().is_none());
    assert_eq!((memo.held(), memo.held_bytes()), (0, 0));
    assert_eq!(
        memo.states(1, anchor, &initial, Ok(&content)).unwrap(),
        anchored
    );
    assert_eq!(memo.held(), 1);
}

#[test]
fn certified_anchor_changes_match_full_validation_and_keep_witness_refusals() {
    let content = content();
    let anchor = date(1936, 100, 7);
    let original = vec![
        state(0, 0, 0, 0),
        state(1_234_567, 250_000, 1_234_567, 250_000),
        state(800_000, -120_000, 800_000, -120_000),
        state(-50_000, 600_000, 999, 7),
        state(2_000_000, 10, 1_500_000, -40_000),
    ];
    let seeds: Vec<_> = (0..8).map(|i| (i != 2).then_some(i * 171 + 7)).collect();
    let mut fast = Memo::default();
    let mut full = Memo::default();
    let mut before: Option<Vec<WeatherProvinceRow>> = None;
    let mut previous = None;
    let mut last_states: Option<Vec<State>> = None;
    for (branch, step, edit, corrupt) in [
        (1, 0, false, false),
        (1, 1, false, false),
        (1, 2, false, false),
        (1, 3, false, false),
        (1, 4, true, false),
        (1, 5, true, true),
        (1, 6, true, false),
        (1, 8, true, false),
        (2, 8, true, false),
        (2, 2, true, false),
    ] {
        let mut anchored = original.clone();
        if edit {
            anchored[3].staged += 31;
        }
        let mut expected = Simulation::new(anchor, 1, anchored.clone());
        for &seed in &seeds[..step] {
            expected.advance(&content, seed).unwrap();
        }
        let frame = rows(
            &anchored,
            Some(crate::TemperaturePass {
                anchor,
                cursor: 1,
                anchor_seed: None,
                hours: seeds[..step].to_vec(),
                witness: crate::TemperatureWitness::new(
                    expected.witness() ^ u64::from(corrupt),
                ),
            }),
        );
        let hour = anchor + step as i32;
        let keys: Vec<_> = frame
            .iter()
            .enumerate()
            .filter_map(|(at, row)| {
                (before.as_ref().and_then(|rows| rows.get(at)) != Some(row)).then_some(at as u128)
            })
            .collect();
        let consecutive = previous.is_some_and(|(b, h): (u64, i32)| b == branch && h + 1 == hour);
        let observed = fast
            .states_changed(
                branch,
                hour,
                frame.as_slice(),
                Ok(&content),
                consecutive.then_some(keys.as_slice()),
            )
            .map(<[State]>::to_vec);
        let oracle = full
            .states(branch, hour, &frame, Ok(&content))
            .map(<[State]>::to_vec);
        assert_eq!(observed, oracle, "{branch}/{hour}/{edit}/{corrupt}");
        if let Ok(now) = &observed {
            if let Some(changes) = fast.changes() {
                let old = last_states.as_ref().unwrap();
                let expected: Vec<_> = old
                    .iter()
                    .zip(now)
                    .enumerate()
                    .filter_map(|(id, (a, b))| {
                        (a.temperature != b.temperature).then_some((
                            id as u32,
                            a.temperature,
                            b.temperature,
                        ))
                    })
                    .collect();
                let mut actual = changes.to_vec();
                actual.sort_unstable_by_key(|edit| edit.0);
                assert_eq!(actual, expected);
            }
        } else {
            assert!(fast.changes().is_none());
        }
        previous = observed.is_ok().then_some((branch, hour));
        last_states = observed.ok();
        before = Some(frame);
    }
}

#[test]
fn minimum_signed_window_reserves_every_committed_edit_before_native_update() {
    let count = 9;
    let content = Content {
        provinces: (0..count)
            .map(|id| Province {
                active: true,
                neighbours: vec![id as u32],
                region: None,
                terrain: Ok(None),
            })
            .collect(),
        settings: Settings {
            provinces_per_update: i32::MIN,
            ..Default::default()
        },
        ..Default::default()
    };
    let states = vec![
        State {
            temperature: 10,
            staged: 20,
            ..Default::default()
        };
        count
    ];
    let mut memo = Memo::default();
    memo.reserve_changes(&content).unwrap();
    let capacity = memo.changed.capacity();
    assert_eq!(capacity, count);
    assert_eq!(
        window(3, i32::MIN, count as i32),
        Ok((vec![3, 4, 5, 6, 7, 8, 0, 1, 2], 3))
    );
    assert_eq!(
        window_span(3, i32::MIN, count as i32),
        Ok((count as i32, 3))
    );
    let mut simulation = Simulation::new(10, 3, states.clone());
    let mut untracked = Simulation::new(10, 3, states);
    simulation
        .update_changed(&content, 11, 7, Some(&mut memo.changed))
        .unwrap();
    untracked.update(&content, 11, 7).unwrap();
    assert_eq!(simulation, untracked);
    assert!(
        simulation
            .states
            .iter()
            .all(|state| state.temperature == 20)
    );
    assert_eq!(memo.changed.len(), count);
    assert_eq!(memo.changed.capacity(), capacity);
    for &(_, before, now) in &memo.changed {
        assert_eq!((before, now), (10, 20));
    }
}

#[test]
fn the_planned_pass_and_moved_witness_match_the_content_pass() {
    let base = content();
    let mut variants = vec![base.clone()];
    let mut refused_periods = base.clone();
    refused_periods
        .periods
        .insert(20, Err("synthetic periods".into()));
    variants.push(refused_periods);
    let mut no_template = base.clone();
    no_template.periods.remove(&30);
    variants.push(no_template);
    let mut refused_terrain = base.clone();
    refused_terrain.provinces[2].terrain = Err("synthetic terrain".into());
    variants.push(refused_terrain);
    let mut broken = base.clone();
    broken.provinces[4].neighbours.push(9);
    variants.push(broken);
    let mut wide = base;
    wide.settings.provinces_per_update = i32::MIN;
    variants.push(wide);
    let anchored = vec![
        state(0, 0, 0, 0),
        state(1_234_567, 250_000, 1_234_567, 250_000),
        state(800_000, -120_000, 800_000, -120_000),
        state(-50_000, 600_000, 999, 7),
        state(2_000_000, 10, 1_500_000, -40_000),
    ];
    let anchor = date(1936, 100, 7);
    let seeds: Vec<Option<u32>> = (0..9u32)
        .map(|at| (at % 3 != 1).then_some(at.wrapping_mul(0x9e37_79b9)))
        .collect();
    for (variant, content) in variants.iter().enumerate() {
        let plan = Plan::new(content);
        let planned = Planned::new(content, &plan);
        let mut whole = Simulation::new(anchor, 1, anchored.clone());
        let mut fast = whole.clone();
        let mut staged = Vec::new();
        for &seed in &seeds {
            let expected = whole.advance(content, seed);
            let actual = fast.advance_with(&planned, seed, &mut staged, None, &mut None);
            assert_eq!(actual, expected, "variant {variant}");
            assert_eq!(fast, whole, "variant {variant}");
            if expected.is_err() {
                break;
            }
        }
        let mut reference = Memo::default();
        let mut memo = Memo::default();
        let mut derivation = Simulation::new(anchor, 1, anchored.clone());
        for step in 0..=seeds.len() {
            let failed = step > 0 && derivation.advance(content, seeds[step - 1]).is_err();
            let frame = rows(
                &anchored,
                Some(crate::TemperaturePass {
                    anchor,
                    cursor: 1,
                    anchor_seed: None,
                    hours: seeds[..step].to_vec(),
                    witness: crate::TemperatureWitness::new(if failed {
                        0
                    } else {
                        derivation.witness()
                    }),
                }),
            );
            let hour = anchor + step as i32;
            let changed = (step > 0).then_some(&[0_u128][..]);
            let expected = reference
                .states_changed(1, hour, frame.as_slice(), Ok(content), changed)
                .map(<[State]>::to_vec);
            let actual = memo
                .states_changed(1, hour, frame.as_slice(), Ok(&planned), changed)
                .map(<[State]>::to_vec);
            assert_eq!(actual, expected, "variant {variant}, step {step}");
            assert_eq!(
                memo.changes(),
                reference.changes(),
                "variant {variant}, step {step}"
            );
            if failed {
                break;
            }
        }
    }
}

#[test]
fn reciprocal_averages_are_the_guarded_division() {
    let edge = (1_i64 << 31) - 1;
    let mut sums = vec![
        0,
        1,
        -1,
        7,
        -7,
        99_999,
        -100_001,
        edge,
        -edge,
        edge + 1,
        -edge - 1,
    ];
    sums.extend([i64::MAX, i64::MIN, i64::MIN + 1, 1 << 40, -(1 << 40)]);
    let mut x = 0x9e37_79b9_7f4a_7c15_u64;
    for _ in 0..4_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        sums.push((x as i64) >> (x % 40));
    }
    for count in (1..=64).chain([100, 1_000, 65_535, 1 << 20]) {
        let divisor = reciprocal(count);
        for &sum in &sums {
            assert_eq!(
                average_by(sum, count as i32, divisor),
                average(sum, count as i32),
                "{sum} over {count}"
            );
        }
    }
    assert_eq!(reciprocal(0), (0, 0));
}
