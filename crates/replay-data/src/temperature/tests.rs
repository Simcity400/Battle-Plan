use super::*;

/// A game date: `days` after 1 January of `year`, at `hour`.
fn date(year: i32, days: i32, hour: i32) -> i32 {
    EPOCH + year * HOURS_PER_YEAR + days * 24 + hour
}

#[test]
fn the_roll_matches_independent_hash_expectations() {
    // Expected hash values calculated independently of the implementation.
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
fn period_bounds_follow_the_native_date_setters() {
    let year_one = PERIOD_DEFAULT_DATE;
    assert_eq!(normalize_to_year_one(year_one), year_one);
    assert_eq!(apply_bound(year_one, 0, 0), Ok(year_one));
    assert_eq!(apply_bound(year_one, 30, 11), Ok(year_one + 364 * 24));
    // The zero-based day setter does not clamp: 30 February is 3 March.
    assert_eq!(apply_bound(year_one, 30, 1), Ok(year_one + 61 * 24));
    // A bound already set moves within year one; the month keeps the day offset.
    let june = apply_bound(year_one, 14, 5).unwrap();
    assert_eq!(june, year_one + (151 + 14) * 24);
    assert_eq!(set_month_zero_based(june, 0), Ok(year_one + 14 * 24));
    assert_eq!(
        set_month_zero_based(june, 12),
        Ok(year_one + (365 + 14) * 24)
    );
    assert!(set_month_zero_based(june, 13).is_err());
}

#[test]
fn the_period_date_is_the_day_of_year_in_year_one() {
    let year_one = PERIOD_DEFAULT_DATE;
    assert_eq!(period_date(date(1936, 0, 0)), year_one);
    assert_eq!(period_date(date(1936, 0, 23)), year_one);
    assert_eq!(period_date(date(1943, 59, 5)), year_one + 59 * 24);
    assert_eq!(period_date(date(1940, 364, 12)), year_one + 364 * 24);
    let periods = [
        Period {
            start: year_one,
            end: year_one + 30 * 24,
            minimum: 1,
            maximum: 2,
        },
        Period {
            start: year_one + 20 * 24,
            end: year_one + 364 * 24,
            minimum: 3,
            maximum: 4,
        },
    ];
    // Inclusive bounds; the first period in template order wins.
    assert_eq!(find_period(&periods, date(1936, 30, 6)).unwrap().minimum, 1);
    assert_eq!(find_period(&periods, date(1936, 31, 0)).unwrap().minimum, 3);
    assert!(find_period(&periods[..1], date(1936, 31, 0)).is_none());
}

#[test]
fn the_window_wraps_stops_on_its_start_and_visits_at_least_one_slot() {
    assert_eq!(window(3, 3, 5), Ok((vec![3, 4, 0], 1)));
    assert_eq!(window(4, 2, 5), Ok((vec![4, 0], 1)));
    // A window wider than the array visits each slot once and returns to the start.
    assert_eq!(window(2, 9, 5), Ok((vec![2, 3, 4, 0, 1], 2)));
    assert_eq!(window(1, 0, 5), Ok((vec![1], 2)));
    assert_eq!(window(1, -4, 5), Ok((vec![1], 2)));
    // A cursor past the count visits nothing and stays.
    assert_eq!(window(5, 3, 5), Ok((vec![], 5)));
    assert!(window(-1, 3, 5).is_err());
}

fn content() -> Content {
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
            // No region: the pass smooths its stale staged temperature.
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
    // Independently calculated expectations for the synthetic weather inputs.
    let seed = 0x9e37_79b9_u32;
    simulation.advance(&content, Some(seed)).unwrap();
    assert_eq!(simulation.hour, hour);
    assert_eq!(simulation.cursor, 4);
    assert_eq!(
        simulation.states,
        [
            state(0, 0, 0, 0),
            // Slot 1 reads slot 2's committed words of the hour before.
            state(1_157_465, 120_000, 1_157_465, 120_000),
            state(618_397, 212_500, 618_397, 212_500),
            // No active neighbour: staged and working move, nothing commits.
            state(-50_000, 600_000, 87_991, 0),
            state(2_000_000, 10, 1_500_000, -40_000),
        ]
    );
    // A suspended hour changes nothing; the next update wraps to slot 0.
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
    assert_eq!(simulation.checksum(), 0xc02c_c1f6_239a_6700);
}

#[test]
fn unsettled_content_refuses_only_where_the_pass_reads_it() {
    let mut content = content();
    let states = vec![State::default(); 5];
    // Slot 4 has no region, so its unknown terrain is never read.
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
fn the_checksum_is_fnv_over_little_endian_temperatures() {
    assert_eq!(checksum([]), 0xcbf2_9ce4_8422_2325);
    assert_ne!(checksum([1, 0]), checksum([0, 1]));
}

fn rows(states: &[State], pass: Option<crate::rows::TemperaturePass>) -> Vec<WeatherProvinceRow> {
    states
        .iter()
        .enumerate()
        .map(|(slot, &state)| WeatherProvinceRow {
            id: slot as u32,
            temperature: crate::rows::ProvinceTemperature::anchored(state),
            temperature_pass: Recorded::Value(if slot == 0 { pass.clone() } else { None }),
            ..Default::default()
        })
        .collect()
}

#[test]
fn frames_derive_from_their_anchor_and_must_match_the_recorded_checksum() {
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
    let pass = |hours: Vec<Option<u32>>, checksum| crate::rows::TemperaturePass {
        anchor: hour - 1,
        cursor: 1,
        anchor_seed: None,
        hours,
        checksum,
    };
    let mut memo = Memo::default();
    let frame = rows(
        &anchored,
        Some(pass(vec![Some(seed)], super::checksum(derived))),
    );
    let states = memo.states(1, hour, &frame, Ok(&content)).unwrap();
    assert_eq!(
        states.iter().map(|s| s.temperature).collect::<Vec<_>>(),
        derived
    );
    // The next hour of the same anchor steps once from the kept derivation; a suspended
    // hour changes nothing.
    let frame = rows(
        &anchored,
        Some(pass(vec![Some(seed), None], super::checksum(derived))),
    );
    memo.states(1, hour + 1, &frame, Ok(&content)).unwrap();
    assert_eq!(memo.simulation().unwrap().hour, hour + 1);
    // A wrong checksum, a pass that does not reach the frame, a refused pass and missing
    // content each refuse every temperature of the frame.
    let wrong = rows(
        &anchored,
        Some(pass(vec![Some(seed ^ 1)], super::checksum(derived))),
    );
    assert!(
        Memo::default()
            .states(1, hour, &wrong, Ok(&content))
            .unwrap_err()
            .contains("checksum")
    );
    let short = rows(&anchored, Some(pass(vec![], super::checksum(derived))));
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
        Some(pass(vec![Some(seed)], super::checksum(derived))),
    );
    assert!(
        Memo::default()
            .states(1, hour, &frame, Err("unread"))
            .is_err()
    );
    // At its anchor a frame's temperatures are its rows' own.
    let at_anchor = rows(&anchored, Some(pass(vec![], 0)));
    let mut anchor_pass = pass(vec![], super::checksum(anchored.map(|s| s.temperature)));
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
fn hourly_temperatures_of_older_formats_need_no_content() {
    let frame: Vec<WeatherProvinceRow> = [5, -6]
        .into_iter()
        .enumerate()
        .map(|(slot, temperature)| WeatherProvinceRow {
            id: slot as u32,
            temperature: crate::rows::ProvinceTemperature::historical(temperature),
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
            for width in [-3, 0, 1, 2, 4, 5, 13, 40] {
                let listed =
                    window(cursor, width, count).map(|(slots, next)| (slots.len() as i32, next));
                assert_eq!(window_span(cursor, width, count), listed);
            }
        }
    }
}
