use super::*;
use replay::{
    frame::TableView,
    rows::{CountryLinkRow, CountryRow, LandCombatRow, LandCombatSideRow, TableRows},
};

fn frame(hour: i32, countries: u32, combats: u32) -> DecodedFrame {
    let mut frame = crate::test_fixtures::source(1, hour, Some(0));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=countries)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("mod country {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=countries)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    set_combats(&mut frame, combats, 0);
    frame
}
fn set_combats(frame: &mut DecodedFrame, count: u32, generation: u64) {
    *frame.tables[TableId::LandCombats.index()] = TableView::Complete(TableRows::LandCombats(
        (1..=count)
            .map(|id| LandCombatRow {
                id: generation << 32 | u64::from(id),
                province: id,
                day: 7,
                duration: 12,
                attackers: vec![u64::from(id) << 32 | 99, u64::from(id) << 32 | 100],
                defenders: vec![u64::from(id) << 32 | 101],
            })
            .collect(),
    ));
    *frame.tables[TableId::LandCombatSides.index()] =
        TableView::Complete(TableRows::LandCombatSides(
            (1..=count)
                .map(|id| LandCombatSideRow {
                    combat: generation << 32 | u64::from(id),
                    attacker_primary: vec![id],
                    attacker_secondary: vec![2],
                    defender_primary: vec![id + 1],
                    ..Default::default()
                })
                .collect(),
        ));
}
fn params(window: Option<usize>) -> LandBattlesParams {
    LandBattlesParams {
        branch_id: "1".into(),
        window: window.unwrap_or(0),
    }
}
fn windows(first: i32, last: i32) -> Vec<BattleWindow> {
    vec![BattleWindow {
        first_hour: first,
        last_hour: last,
    }]
}
#[test]
fn land_battles_exact_adjacent_boundaries_distinct_rosters_and_native_duration() {
    let mut census = Census::default();
    census.observe(&frame(99, 5000, 0)).unwrap();
    let mut sample = frame(100, 5000, 1);
    let mut combats = sample.complete::<tables::LandCombats>().unwrap().to_vec();
    let repeated = combats[0].attackers[0];
    combats[0].attackers.push(repeated);
    *sample.tables[TableId::LandCombats.index()] =
        TableView::Complete(TableRows::LandCombats(combats));
    census.observe(&sample).unwrap();
    sample.hour = 101;
    census.observe(&sample).unwrap();
    sample.hour = 102;
    set_combats(&mut sample, 0, 0);
    census.observe(&sample).unwrap();
    let reply = census.reply(&params(Some(0)), &windows(99, 102)).unwrap();
    let row = &reply.rows[0];
    assert_eq!(row.start.hour, 100);
    assert_eq!(row.start.bound, BattleBound::Exact);
    assert!(
        matches!(&row.end, Some(BattleEnd::Moment(end)) if end.hour == 102 && end.bound == BattleBound::Exact)
    );
    assert_eq!(row.runs[0].counts, [2, 1]);
    assert_eq!(row.runs.len(), 2);
    assert_eq!(row.runs[0].duration, 12);
    assert_eq!(row.counter_base, 0);
    assert_eq!(row.runs[0].countries[0], ["mod country 1", "mod country 2"]);
}
#[test]
fn land_battles_gaps_split_reused_handles_and_keep_missing_endings() {
    let mut census = Census::default();
    census.observe(&frame(100, 5000, 1)).unwrap();
    let mut missing = frame(101, 5000, 1);
    *missing.tables[TableId::LandCombats.index()] = TableView::Unavailable {
        reason: "missing".into(),
    };
    census.observe(&missing).unwrap();
    census.observe(&frame(105, 5000, 1)).unwrap();
    let reply = census.reply(&params(Some(0)), &windows(100, 105)).unwrap();
    assert_eq!(reply.rows.len(), 2);
    assert_ne!(reply.rows[0].id, reply.rows[1].id);
    assert!(
        matches!(&reply.rows[0].end, Some(BattleEnd::Unknown { unknown }) if unknown == "Ending not recorded")
    );
    assert_eq!(reply.rows[0].end_unknown_hour, Some(101));
    assert_eq!(reply.rows[1].start.bound, BattleBound::By);
    assert_eq!(reply.rows[1].counter_base, 12);
    assert!(reply.rows[1].end.is_none());
    assert_eq!(reply.rows[0].runs[0].last_hour, 100);
}
#[test]
fn land_battles_windows_clip_reads_retain_prior_peak_and_scope_warnings() {
    let mut census = Census::default();
    for hour in 100..=105 {
        census.observe(&frame(hour, 5000, 1)).unwrap();
    }
    census.warn(101, "earlier warning");
    census.warn(105, "later warning");
    census.episodes[0].runs[0].counts = [4, 1];
    let windows = [
        BattleWindow {
            first_hour: 100,
            last_hour: 102,
        },
        BattleWindow {
            first_hour: 103,
            last_hour: 105,
        },
    ];
    let reply = census.reply(&params(Some(1)), &windows).unwrap();
    assert_eq!(reply.rows[0].runs[0].first_hour, 103);
    assert_eq!(reply.rows[0].runs.len(), 3);
    assert_eq!(reply.rows[0].runs[0].duration, 12);
    assert_eq!(reply.rows[0].prior_peak.as_ref().unwrap().hour, 100);
    assert_eq!(reply.rows[0].prior_peak.as_ref().unwrap().value, 5);
    assert_eq!(reply.observed, [(103, 105)]);
    assert_eq!(reply.warnings.len(), 1);
    assert_eq!(reply.warnings[0].hour, 105);
    assert!(census.reply(&params(Some(2)), &windows).is_err());
}
#[test]
fn land_battles_window_boundary_end_carries_only_the_final_read() {
    let mut census = Census::default();
    for hour in 100..=999 {
        census.observe(&frame(hour, 5, 1)).unwrap();
    }
    census.observe(&frame(1000, 5, 0)).unwrap();
    let reply = census
        .reply(&params(Some(0)), &windows(1000, 1000))
        .unwrap();
    assert_eq!(reply.rows.len(), 1);
    let row = &reply.rows[0];
    assert_eq!(row.runs[0].first_hour, 999);
    assert_eq!(row.runs[0].last_hour, 999);
    assert_eq!(row.runs[0].duration, 12);
    assert_eq!(row.prior_peak.as_ref().unwrap().hour, 100);
    assert!(matches!(&row.end, Some(BattleEnd::Moment(end)) if end.hour == 1000));
}
#[test]
fn land_battles_no_chosen_row_or_country_limits_and_only_reply_budget_refuses() {
    let mut census = Census::default();
    census.observe(&frame(100, 5000, 3000)).unwrap();
    let reply = census.reply(&params(Some(0)), &windows(100, 100)).unwrap();
    assert_eq!(reply.rows.len(), 3000);
    assert!(LandBattles::validate(&params(None)).is_ok());
    assert!(
        serde_json::from_value::<LandBattlesParams>(
            serde_json::json!({"branchId":"1","window":null,"tags":["tag"]})
        )
        .is_err()
    );
    let length = MAX_PRODUCT_REPLY_BYTES / census.episodes.len() + 1;
    for episode in &mut census.episodes {
        episode.runs[0].countries[0][0] = "x".repeat(length);
    }
    assert!(
        census
            .reply(&params(Some(0)), &windows(100, 100))
            .unwrap_err()
            .to_string()
            .contains("MAX_PRODUCT_REPLY_BYTES")
    );
}
#[test]
fn land_battles_shared_wire_types_and_missing_side_data() {
    use super::battles::{BattleSide, BattleValue};
    assert!(BattleValue::decl().contains("value: null"));
    assert!(BattleValue::decl().contains("atLeast?: boolean"));
    assert!(BattleSide::decl().contains("countriesUnknown?:"));
    assert!(!BattleSide::decl().contains("units"));
    let mut census = Census::default();
    let mut sample = frame(100, 5000, 1);
    *sample.tables[TableId::LandCombatSides.index()] = TableView::Unavailable {
        reason: "old recording".into(),
    };
    census.observe(&sample).unwrap();
    let reply = census.reply(&params(Some(0)), &windows(100, 100)).unwrap();
    assert_eq!(reply.rows.len(), 1);
    assert_eq!(reply.rows[0].runs[0].counts, [2, 1]);
    assert!(reply.rows[0].runs[0].countries.iter().all(Vec::is_empty));
    assert!(!reply.warnings.is_empty());
    assert_eq!(reply.rows[0].runs[0].countries_unknown, [true, true]);
}
#[test]
fn land_battles_wire_scan_keeps_branch_tail_and_absent_read_end() {
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(
            1,
            vec![
                frame(100, 5000, 1),
                frame(101, 5000, 0),
                frame(102, 5000, 1),
            ],
        )],
    )
    .unwrap();
    let census = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let reply = census.reply(&params(Some(0)), &windows(100, 102)).unwrap();
    assert_eq!(reply.rows.len(), 2);
    assert!(
        matches!(&reply.rows[0].end, Some(BattleEnd::Moment(end)) if end.hour == 101 && end.bound == BattleBound::Exact)
    );
    assert!(reply.rows[1].end.is_none());
    assert!(scan(&replay, 1, &AtomicBool::new(true)).is_err());
}
#[test]
#[ignore = "nine-year synthetic scan measurement; run explicitly"]
fn land_battles_nine_years_5000_countries_cost_and_reply_sizes() {
    let provinces = std::env::var("LAND_BATTLES_BENCH_PROVINCES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20_000);
    measure_land_battles(provinces);
}
fn measure_land_battles(provinces: u32) {
    // One active front per 100 content provinces, three divisions, ten-day episodes.
    let concurrent = provinces.div_ceil(100);
    let mut sample = frame(100, 5000, concurrent);
    let original_countries = sample.tables[TableId::Countries.index()].clone();
    let mut changed = sample.complete::<tables::Countries>().unwrap().to_vec();
    changed[0].political_power_raw = Some(1);
    let changed_countries: replay::frame::SharedView =
        TableView::Complete(TableRows::Countries(changed)).into();
    let mut census = Census::default();
    let mut nanos = Vec::with_capacity(9 * 365 * 24);
    for elapsed in 0..9 * 365 * 24 {
        sample.hour = 100 + elapsed;
        sample.tables[TableId::Countries.index()] = if elapsed % 2 == 0 {
            original_countries.clone()
        } else {
            changed_countries.clone()
        };
        if elapsed % 240 == 0 {
            set_combats(&mut sample, concurrent, (elapsed / 240) as u64);
        }
        let mut combats = sample.complete::<tables::LandCombats>().unwrap().to_vec();
        for combat in &mut combats {
            combat.duration = elapsed % 240 + 1;
        }
        *sample.tables[TableId::LandCombats.index()] =
            TableView::Complete(TableRows::LandCombats(combats));
        let start = std::time::Instant::now();
        census.observe(&sample).unwrap();
        nanos.push(start.elapsed().as_nanos());
    }
    nanos.sort_unstable();
    let step = replay::KEYFRAME_INTERVAL as i32;
    let windows: Vec<_> = (100..=sample.hour)
        .step_by(step as usize)
        .map(|first| BattleWindow {
            first_hour: first,
            last_hour: (first + step - 1).min(sample.hour),
        })
        .collect();
    let manifest = LandBattlesManifestReply {
        windows: windows.clone(),
        available: true,
        unavailable: None,
        content_warnings: Vec::new(),
    };
    let first = census.reply(&params(Some(0)), &windows).unwrap();
    let last = census
        .reply(&params(Some(windows.len() - 1)), &windows)
        .unwrap();
    eprintln!(
        "land_battles: provinces={provinces}, concurrent={concurrent}, countries=5000, hours={}, episodes={}, decoded p50={:.3}ms p95={:.3}ms, manifest={} bytes, first window={} bytes, final window={} bytes ({} hours)",
        nanos.len(),
        census.episodes.len(),
        nanos[nanos.len() / 2] as f64 / 1e6,
        nanos[nanos.len() * 95 / 100] as f64 / 1e6,
        json_len(&manifest, MAX_PRODUCT_REPLY_BYTES).unwrap(),
        json_len(&first, MAX_PRODUCT_REPLY_BYTES).unwrap(),
        json_len(&last, MAX_PRODUCT_REPLY_BYTES).unwrap(),
        step
    );

    let ranking = census
        .ranked(
            &ranking_params(100, sample.hour, 30, "Peak size"),
            &AtomicBool::new(false),
        )
        .unwrap();
    eprintln!(
        "land_battles: top30={} bytes, census counter runs={}",
        json_len(&ranking, MAX_PRODUCT_REPLY_BYTES).unwrap(),
        census
            .episodes
            .iter()
            .map(|episode| episode.runs.len())
            .sum::<usize>()
    );
    let previous = census
        .reply(&params(Some(windows.len() - 2)), &windows)
        .unwrap();
    let before_previous = census
        .reply(&params(Some(windows.len() - 3)), &windows)
        .unwrap();
    let census_bytes = census.episodes.capacity() * std::mem::size_of::<Episode>()
        + census
            .episodes
            .iter()
            .map(|episode| {
                episode.id.capacity()
                    + episode.runs.capacity() * std::mem::size_of::<LandBattleRun>()
                    + episode
                        .runs
                        .iter()
                        .map(|run| {
                            run.countries
                                .iter()
                                .map(|side| {
                                    side.capacity() * std::mem::size_of::<String>()
                                        + side.iter().map(String::capacity).sum::<usize>()
                                })
                                .sum::<usize>()
                        })
                        .sum::<usize>()
            })
            .sum::<usize>();
    eprintln!(
        "land_battles: episode storage={} bytes (allocated payload/capacities, excluding allocator and shared LinkTags), Campaign two windows={} bytes, Live three windows={} bytes",
        census_bytes,
        json_len(&previous, MAX_PRODUCT_REPLY_BYTES).unwrap()
            + json_len(&last, MAX_PRODUCT_REPLY_BYTES).unwrap(),
        json_len(&before_previous, MAX_PRODUCT_REPLY_BYTES).unwrap()
            + json_len(&previous, MAX_PRODUCT_REPLY_BYTES).unwrap()
            + json_len(&last, MAX_PRODUCT_REPLY_BYTES).unwrap()
    );
    // Reuse a synthetic wire cycle over nine years; handles reappear only after
    // recorded absence. Encoding stays outside timing. Block parsing and
    // decompression are amortized across its built hours, as in a cold scan.
    let frames = (0..replay::KEYFRAME_INTERVAL as i32)
        .map(|elapsed| {
            sample.hour = 100 + elapsed;
            sample.tables[TableId::Countries.index()] = if elapsed % 2 == 0 {
                original_countries.clone()
            } else {
                changed_countries.clone()
            };
            if elapsed % 240 == 0 {
                set_combats(&mut sample, concurrent, (elapsed / 240) as u64);
            }
            let mut combats = sample.complete::<tables::LandCombats>().unwrap().to_vec();
            for combat in &mut combats {
                combat.duration = elapsed % 240 + 1;
            }
            *sample.tables[TableId::LandCombats.index()] =
                TableView::Complete(TableRows::LandCombats(combats));
            sample.clone()
        })
        .collect();
    let wire = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(1, frames)],
    )
    .unwrap();
    let mut decoded_census = Census::default();
    let mut decoded_nanos = Vec::with_capacity(nanos.len());
    while decoded_nanos.len() < nanos.len() {
        let offset = decoded_nanos.len() as i32;
        for block in &wire.index {
            let start = std::time::Instant::now();
            let frames = wire.read_block_selected(block, &CENSUS_TABLES).unwrap();
            let parse_per_hour = start.elapsed().as_nanos() / frames.len() as u128;
            let mut decoder = FrameDecoder::for_tables(&CENSUS_TABLES);
            for mut frame in frames {
                if decoded_nanos.len() == nanos.len() {
                    break;
                }
                frame.hour += offset;
                for update in &mut frame.tables {
                    match update {
                        replay::frame::TableUpdate::Delta { baseline, .. }
                        | replay::frame::TableUpdate::Unchanged { baseline } => *baseline += offset,
                        _ => {}
                    }
                }
                let start = std::time::Instant::now();
                let decoded = decoder.advance(&frame).unwrap();
                validate_campaign(decoded, &wire).unwrap();
                decoded_census.observe(decoded).unwrap();
                decoded_nanos.push(parse_per_hour + start.elapsed().as_nanos());
            }
        }
    }
    assert!(!decoded_census.episodes.is_empty());
    decoded_nanos.sort_unstable();
    eprintln!(
        "land_battles: nine-year wire scan including selected parsing/decompression/reconstruction/census, block-read cost amortized per hour, p50={:.3}ms p95={:.3}ms",
        decoded_nanos[decoded_nanos.len() / 2] as f64 / 1e6,
        decoded_nanos[decoded_nanos.len() * 95 / 100] as f64 / 1e6,
    );
}

fn ranking_params(
    first_hour: i32,
    last_hour: i32,
    top_n: usize,
    measure: &str,
) -> LandBattlesRankingParams {
    LandBattlesRankingParams {
        branch_id: "1".into(),
        first_hour,
        last_hour,
        top_n: top_n as f64,
        measure: measure.into(),
        countries: LandBattleCountryFilter {
            tags: None,
            membership: None,
            unobserved: Vec::new(),
        },
    }
}
#[test]
fn land_battles_counter_runs_are_exact_and_split_non_unit_steps() {
    let mut census = Census::default();
    for (hour, duration) in [(100, 50), (101, 51), (102, 53), (103, 53), (104, 54)] {
        let mut sample = frame(hour, 5, 1);
        if let TableView::Complete(TableRows::LandCombats(rows)) =
            &mut *sample.tables[TableId::LandCombats.index()]
        {
            rows[0].duration = duration;
        }
        census.observe(&sample).unwrap();
    }
    let episode = &census.episodes[0];
    assert_eq!(episode.counter_base, 50);
    assert_eq!(episode.runs.len(), 3);
    assert_eq!(episode.runs[0].duration_at(101), 51);
    assert_eq!(episode.runs[1].duration_at(102), 53);
    assert_eq!(episode.runs[2].duration_at(104), 54);
    let reply = census.reply(&params(Some(0)), &windows(101, 104)).unwrap();
    assert_eq!(reply.rows[0].runs[0].duration, 51);
    assert!(!serde_json::to_string(&reply).unwrap().contains("durations"));
}
#[test]
fn land_battles_cache_retains_two_most_recent_branches() {
    let mut cache = Cache::default();
    for id in [1, 2] {
        cache.insert(id, Arc::new(Census::default()));
    }
    assert!(cache.get(1).is_some());
    cache.insert(3, Arc::new(Census::default()));
    assert!(cache.get(2).is_none());
    assert_eq!(cache.branches.len(), 2);
}
#[test]
fn land_battles_ranking_filters_before_top_n_and_keeps_unknown_participants() {
    let mut census = Census::default();
    let mut sample = frame(100, 5, 3);
    if let TableView::Complete(TableRows::LandCombats(rows)) =
        &mut *sample.tables[TableId::LandCombats.index()]
    {
        rows[0].attackers.extend([200, 201]);
    }
    if let TableView::Complete(TableRows::LandCombatSides(rows)) =
        &mut *sample.tables[TableId::LandCombatSides.index()]
    {
        rows[2].attacker_primary = vec![999];
        rows[2].attacker_secondary.clear();
    }
    census.observe(&sample).unwrap();
    let mut params = ranking_params(100, 100, 1, "Casualties");
    params.countries.tags = Some(vec!["mod country 4".into()]);
    let reply = census.ranked(&params, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.effective_measure, "Peak size");
    assert_eq!(reply.rows.len(), 1);
    assert_eq!(reply.rows[0].province_id, 3);
    assert!(reply.rows[0].runs[0].countries_unknown[0]);
    params.top_n = f64::MAX;
    assert!(LandBattlesRanking::validate(&params).is_ok());
    params.countries.tags = None;
    assert_eq!(
        census
            .ranked(&params, &AtomicBool::new(false))
            .unwrap()
            .rows
            .len(),
        3
    );
    assert!(census.episodes[0].end.is_none());
}
#[test]
fn land_battles_availability_stops_before_later_invalid_campaign_read() {
    let mut invalid = frame(101, 5, 1);
    *invalid.tables[TableId::Campaign.index()] = TableView::Unavailable {
        reason: "later bad read".into(),
    };
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(
            1,
            vec![frame(100, 5, 0), invalid],
        )],
    )
    .unwrap();
    assert!(recording_available(&replay, 1, &AtomicBool::new(false)).unwrap());
    assert!(recording_available(&replay, 1, &AtomicBool::new(true)).is_err());
}

#[test]
fn land_battles_generated_shared_contract_keeps_fractional_values_without_units() {
    let value = battles::BattleValue::Known {
        label: "Fractional caller measure".into(),
        value: 0.125,
        at_least: None,
    };
    assert_eq!(serde_json::to_value(value).unwrap()["value"], 0.125);
    assert!(battles::BattleRow::decl().contains("BattleSide"));
    assert!(!battles::BattleSide::decl().contains("BattleUnit"));
}

#[test]
fn land_battles_top_n_has_no_chosen_upper_limit_and_requires_a_positive_integer() {
    let mut params = ranking_params(100, 100, 1, "Peak size");
    for value in [1e100, f64::MAX] {
        params.top_n = value;
        assert!(LandBattlesRanking::validate(&params).is_ok());
    }
    for value in [0.0, -1.0, 1.5, f64::NAN, f64::INFINITY] {
        params.top_n = value;
        assert!(LandBattlesRanking::validate(&params).is_err());
    }
}

fn loss_read(hour: i32, terminal: bool, amounts: [f64; 2]) -> LandBattleLossRead {
    LandBattleLossRead {
        hour,
        terminal,
        sides: amounts.map(|value| {
            vec![
                battles::BattleValue::Known {
                    label: "Manpower lost".into(),
                    value,
                    at_least: None,
                },
                battles::BattleValue::Known {
                    label: "Equipment lost".into(),
                    value: 0.125,
                    at_least: None,
                },
            ]
        }),
    }
}

#[test]
fn land_battles_casualties_window_carries_one_read_and_keeps_terminal_dated() {
    let mut census = Census::default();
    for hour in 100..=103 {
        census.observe(&frame(hour, 5, 1)).unwrap();
    }
    census.observe(&frame(104, 5, 0)).unwrap();
    census.episodes[0].losses = vec![
        loss_read(100, false, [1.0, 2.0]),
        loss_read(101, false, [3.0, 4.0]),
        loss_read(103, true, [7.0, 8.0]),
    ];
    let before_terminal = census.reply(&params(None), &windows(102, 102)).unwrap();
    assert_eq!(before_terminal.rows[0].losses.len(), 1);
    assert_eq!(before_terminal.rows[0].losses[0].hour, 101);
    assert!(!before_terminal.rows[0].losses[0].terminal);
    let ended = census.reply(&params(None), &windows(104, 104)).unwrap();
    assert_eq!(ended.rows[0].losses.len(), 1);
    assert_eq!(ended.rows[0].losses[0].hour, 103);
    assert!(ended.rows[0].losses[0].terminal);
    assert_eq!(census.episodes[0].casualties_at(102), Some(7.0));
    assert_eq!(census.episodes[0].casualties_at(103), Some(15.0));
}

#[test]
fn land_battles_casualties_rank_so_far_and_terminal_manpower_before_top_n() {
    let mut census = Census::default();
    for hour in 100..=102 {
        census.observe(&frame(hour, 5, 3)).unwrap();
    }
    census.episodes[0].runs[0].counts = [10, 10];
    census.episodes[0].losses = vec![
        loss_read(101, false, [1.0, 2.0]),
        loss_read(102, true, [100.0, 200.0]),
    ];
    census.episodes[1].losses = vec![loss_read(101, false, [10.0, 20.0])];
    let reply = census
        .ranked(
            &ranking_params(100, 101, 1, "Casualties"),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(reply.effective_measure, "Casualties");
    assert_eq!(reply.rows[0].province_id, 2);
    assert!(!reply.rows[0].losses[0].terminal);
    let reply = census
        .ranked(
            &ranking_params(100, 102, 3, "Casualties"),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(
        reply
            .rows
            .iter()
            .map(|row| row.province_id)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(reply.rows[0].losses.last().unwrap().terminal);
    assert!(reply.rows[2].losses.is_empty());
    let earlier = census
        .ranked(
            &ranking_params(100, 100, 1, "Casualties"),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(earlier.effective_measure, "Peak size");
    assert_eq!(earlier.rows[0].province_id, 1);
}

#[test]
fn land_battles_casualties_unknown_side_is_not_zero_and_recorded_zero_ranks() {
    let mut census = Census::default();
    census.observe(&frame(100, 5, 1)).unwrap();
    census.episodes[0].losses = vec![loss_read(100, false, [0.0, 0.0])];
    let params = ranking_params(100, 100, 1, "Casualties");
    assert_eq!(
        census
            .ranked(&params, &AtomicBool::new(false))
            .unwrap()
            .effective_measure,
        "Casualties"
    );
    census.episodes[0].losses[0].sides[1][0] = battles::BattleValue::Unknown {
        label: "Manpower lost".into(),
        value: (),
        unknown: "Loss read unavailable".into(),
    };
    assert_eq!(census.episodes[0].casualties_at(100), None);
    assert_eq!(
        census
            .ranked(&params, &AtomicBool::new(false))
            .unwrap()
            .effective_measure,
        "Peak size"
    );
}

#[test]
fn land_battles_shared_window_and_warning_contract() {
    assert!(battles::types().contains(&BattleWindow::decl()));
    assert!(battles::types().contains(&BattleWarning::decl()));
    assert_eq!(
        serde_json::to_value(BattleWindow {
            first_hour: 100,
            last_hour: 101
        })
        .unwrap(),
        serde_json::json!({ "firstHour": 100, "lastHour": 101 })
    );
    assert_eq!(
        serde_json::to_value(BattleWarning {
            hour: 100,
            reason: "Missing read".into(),
            count: 1
        })
        .unwrap(),
        serde_json::json!({ "hour": 100, "reason": "Missing read", "count": 1 })
    );
}

#[test]
fn land_battles_content_diagnostics_preserve_dated_battle_warnings() {
    let mut census = Census::default();
    census.observe(&frame(100, 5, 1)).unwrap();
    census.warn(100, "Battle read incomplete");
    let mut window = census.reply(&params(Some(0)), &windows(100, 100)).unwrap();
    let mut ranking = census
        .ranked(
            &ranking_params(100, 100, 30, "Peak size"),
            &AtomicBool::new(false),
        )
        .unwrap();
    let mut manifest = LandBattlesManifestReply {
        windows: windows(100, 100),
        available: true,
        unavailable: None,
        content_warnings: Vec::new(),
    };
    let mut places = super::super::place_names::PlaceNamesReply {
        places: Vec::new(),
        warnings: Vec::new(),
    };
    let replies: [&mut dyn ReplyWarnings; 4] =
        [&mut manifest, &mut window, &mut ranking, &mut places];
    for reply in replies {
        reply.warnings_mut().push("Content file unavailable".into());
    }
    let window = serde_json::to_value(window).unwrap();
    let ranking = serde_json::to_value(ranking).unwrap();
    for reply in [&window, &ranking, &serde_json::to_value(manifest).unwrap()] {
        assert_eq!(
            reply["contentWarnings"],
            serde_json::json!(["Content file unavailable"])
        );
    }
    for reply in [&window, &ranking] {
        assert_eq!(
            reply["warnings"],
            serde_json::json!([{"hour": 100, "reason": "Battle read incomplete", "count": 1}])
        );
    }
    assert_eq!(places.warnings, ["Content file unavailable"]);
}
