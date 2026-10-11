use super::*;
use replay::{
    frame::TableView,
    rows::{
        CountryLinkRow, CountryRow, LandCombatProgressRow, LandCombatRow, LandCombatSideRow,
        LandEquipmentLoss, Recorded, TableRows,
    },
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
fn set_progress(frame: &mut DecodedFrame, rows: Vec<LandCombatProgressRow>) {
    *frame.tables[TableId::LandCombatProgress.index()] =
        TableView::Complete(TableRows::LandCombatProgress(rows));
}
fn loss_frame(hour: i32, manpower: i32) -> DecodedFrame {
    let mut sample = frame(hour, 5000, 1);
    set_progress(
        &mut sample,
        vec![LandCombatProgressRow {
            combat: 1,
            attacker_manpower_loss: manpower.into(),
            defender_manpower_loss: (manpower + 1).into(),
            attacker_equipment_losses: vec![LandEquipmentLoss {
                variant: 7,
                amount_raw: -100_000,
            }]
            .into(),
            ..Default::default()
        }],
    );
    sample
}
fn result_frame(hour: i32, winners: u8) -> DecodedFrame {
    let mut sample = frame(hour, 5000, 0);
    *sample.tables[TableId::LandCombatResults.index()] =
        TableView::Complete(TableRows::LandCombatResults(vec![
            replay::rows::LandCombatResultsRow {
                id: 1,
                combat: 1,
                province: 1,
                end_hour: hour,
                duration: 13,
                winners,
                sides: std::array::from_fn(|index| replay::rows::LandResultSide {
                    countries: vec![index as u32 + 1],
                    ..Default::default()
                }),
                losses: std::array::from_fn(|index| replay::rows::LandSideLosses {
                    manpower: 10 + index as i32,
                    equipment: Vec::new(),
                }),
                ..Default::default()
            },
        ]));
    *sample.tables[TableId::LandEventCoverage.index()] =
        TableView::Complete(TableRows::LandEventCoverage(vec![
            replay::rows::LandEventCoverageRow {
                from_hour: hour - 1,
                to_hour: hour,
                complete: true,
                ..Default::default()
            },
        ]));
    sample
}
#[test]
fn land_battles_wire_joins_hourly_losses_missing_reads_and_terminal_verdict() {
    let mut missing = loss_frame(101, 5);
    *missing.tables[TableId::LandCombatProgress.index()] = TableView::Unavailable {
        reason: "short loss arena".into(),
    };
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(
            1,
            vec![
                loss_frame(100, 4),
                missing,
                loss_frame(102, 6),
                result_frame(103, 2),
            ],
        )],
    )
    .unwrap();
    let census = scan(&replay, 1, &AtomicBool::new(false), None).unwrap();
    let row = &census.episodes[0];
    assert_eq!(census.episodes.len(), 1);
    assert_eq!(row.casualties_at(100), Some(9.0));
    assert_eq!(row.casualties_at(101), None);
    assert_eq!(row.casualties_at(102), Some(13.0));
    assert_eq!(row.casualties_at(103), Some(21.0));
    assert!(
        matches!(&row.losses[0].sides, Ok([attacker, _]) if attacker.equipment == Recorded::Value(vec![(0, -100_000)]))
    );
    let early = census.reply(&params(None), &windows(100, 102)).unwrap();
    assert!(early.rows[0].result.is_none());
    assert!(!early.rows[0].losses.last().unwrap().terminal);
    let ended = census.reply(&params(None), &windows(103, 103)).unwrap();
    assert!(matches!(
        ended.rows[0].result.as_ref().unwrap().winner,
        battles::BattleWinner::Side { side: 1 }
    ));
    assert!(ended.rows[0].losses.last().unwrap().terminal);
}
#[test]
fn land_battles_loss_fields_remain_independent_and_equipment_stays_exact() {
    let mut sample = loss_frame(100, 4);
    if let TableView::Complete(TableRows::LandCombatProgress(rows)) =
        &mut *sample.tables[TableId::LandCombatProgress.index()]
    {
        rows[0].attacker_equipment_losses = Recorded::NotRecorded;
        rows[0].defender_equipment_losses = vec![LandEquipmentLoss {
            variant: 8,
            amount_raw: -9_223_372_036_854_775_807,
        }]
        .into();
    }
    let mut census = Census::default();
    census.observe(&sample).unwrap();
    assert_eq!(census.episodes[0].casualties_at(100), Some(9.0));
    let reply = census.reply(&params(None), &windows(100, 100)).unwrap();
    assert_eq!(reply.labels, [label(8)]);
    let LandLossSides::Read([attacker, defender]) = &reply.rows[0].losses[0].sides else {
        panic!("recorded counters");
    };
    assert!(
        matches!(attacker, LandLossSide::Unknown { manpower: LandManpower::Known(4), unknown } if unknown == "Not in this recording")
    );
    assert!(
        matches!(defender, LandLossSide::Variants { manpower: LandManpower::Known(5), variants } if variants == &[(0, "-9223372036854775807".to_string())])
    );
}
#[test]
fn land_battles_terminal_only_rows_keep_unknown_rosters_and_do_not_join_across_gaps() {
    let mut census = Census::default();
    census.observe(&loss_frame(100, 4)).unwrap();
    let mut ended = result_frame(105, 3);
    if let TableView::Complete(TableRows::LandEventCoverage(rows)) =
        &mut *ended.tables[TableId::LandEventCoverage.index()]
    {
        rows[0].from_hour = 100;
    }
    census.observe(&ended).unwrap();
    assert_eq!(census.episodes.len(), 2);
    assert!(census.episodes[0].result.is_none());
    assert!(matches!(
        &census.episodes[0].end,
        Some((_, BattleEnd::Unknown { .. }))
    ));
    let terminal = &census.episodes[1];
    assert!(terminal.runs[0].roster_unknown);
    assert!(terminal.peak_before(105).is_none());
    assert_eq!(terminal.casualties_at(105), Some(21.0));
    assert!(
        matches!(&terminal.result.as_ref().unwrap().winner, battles::BattleWinner::Unknown { unknown } if unknown == "Both sides awarded a win")
    );
    let ranking = census
        .ranked(
            &ranking_params(100, 105, 2, "Peak size"),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(!ranking.rows[0].runs[0].roster_unknown);
    assert!(ranking.rows[1].runs[0].roster_unknown);
}
#[test]
fn land_battles_terminal_only_availability_requires_current_complete_coverage() {
    for complete in [false, true] {
        let mut sample = result_frame(100, 1);
        *sample.tables[TableId::LandCombats.index()] = TableView::Unavailable {
            reason: "census unread".into(),
        };
        if let TableView::Complete(TableRows::LandEventCoverage(rows)) =
            &mut *sample.tables[TableId::LandEventCoverage.index()]
        {
            rows[0].complete = complete;
        }
        let replay = Replay::from_records(
            crate::test_fixtures::CAMPAIGN.into(),
            &[crate::test_fixtures::records(1, vec![sample])],
        )
        .unwrap();
        assert_eq!(
            recording_available(&replay, 1, &AtomicBool::new(false)).unwrap(),
            complete
        );
        let census = scan(&replay, 1, &AtomicBool::new(false), None).unwrap();
        assert_eq!(census.episodes.len(), usize::from(complete));
    }
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
    assert_eq!(reply.reasons, ["later warning"]);
    assert_eq!(reply.warnings, [(105, 0, 1)]);
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
    let census = scan(&replay, 1, &AtomicBool::new(false), None).unwrap();
    let reply = census.reply(&params(Some(0)), &windows(100, 102)).unwrap();
    assert_eq!(reply.rows.len(), 2);
    assert!(
        matches!(&reply.rows[0].end, Some(BattleEnd::Moment(end)) if end.hour == 101 && end.bound == BattleBound::Exact)
    );
    assert!(reply.rows[1].end.is_none());
    assert!(scan(&replay, 1, &AtomicBool::new(true), None).is_err());
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
    assert!(cache.get(1, None).is_some());
    cache.insert(3, Arc::new(Census::default()));
    assert!(cache.get(2, None).is_none());
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

fn loss_read(hour: i32, terminal: bool, manpower: [i32; 2]) -> losses::LossRead {
    losses::LossRead {
        hour,
        terminal,
        sides: Ok(manpower.map(|value| losses::SideRead {
            manpower: Recorded::Value(value),
            equipment: Recorded::Value(Vec::new()),
        })),
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
        loss_read(100, false, [1, 2]),
        loss_read(101, false, [3, 4]),
        loss_read(103, true, [7, 8]),
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
        loss_read(101, false, [1, 2]),
        loss_read(102, true, [100, 200]),
    ];
    census.episodes[1].losses = vec![loss_read(101, false, [10, 20])];
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
    assert!(
        matches!(&reply.rows[2].losses[0].sides, LandLossSides::Unread(reason) if reason == "Hourly losses not recorded")
    );
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
    census.episodes[0].losses = vec![loss_read(100, false, [0, 0])];
    let params = ranking_params(100, 100, 1, "Casualties");
    assert_eq!(
        census
            .ranked(&params, &AtomicBool::new(false))
            .unwrap()
            .effective_measure,
        "Casualties"
    );
    census.episodes[0].losses[0].sides.as_mut().unwrap()[1].manpower = Recorded::NotRecorded;
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
        states: Vec::new(),
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
    assert_eq!(
        window["reasons"],
        serde_json::json!(["Battle read incomplete"])
    );
    assert_eq!(window["warnings"], serde_json::json!([[100, 0, 1]]));
    assert_eq!(
        ranking["warnings"],
        serde_json::json!([{"hour": 100, "reason": "Battle read incomplete", "count": 1}])
    );
    assert_eq!(places.warnings, ["Content file unavailable"]);
}

fn label(variant: u64) -> String {
    format!("Equipment lost · Name not recorded (variant {variant})")
}
/// One full side: manpower, then each variant's label and exact amount; reasons stay text.
type Side<'a> = (Result<i32, String>, Result<Vec<(&'a str, i64)>, String>);
/// Rebuilds every read's full sides, applying a side's changes to its previous read.
fn decode<'a>(
    labels: &'a [String],
    reads: &[LandBattleLossRead],
) -> Vec<(i32, Result<[Side<'a>; 2], String>)> {
    let mut previous: [Option<Vec<(&str, i64)>>; 2] = [None, None];
    reads
        .iter()
        .map(|read| {
            let sides = match &read.sides {
                LandLossSides::Unread(reason) => {
                    previous = [None, None];
                    Err(reason.clone())
                }
                LandLossSides::Read(sides) => Ok(std::array::from_fn(|index| {
                    let (manpower, equipment) = match &sides[index] {
                        LandLossSide::Variants { manpower, variants } => (
                            manpower,
                            Ok(variants
                                .iter()
                                .map(|(label, amount)| {
                                    (labels[*label].as_str(), amount.parse::<i64>().unwrap())
                                })
                                .collect()),
                        ),
                        LandLossSide::Changed { manpower, changed } => {
                            let mut list = previous[index].clone().expect("a listed read");
                            for (at, amount) in changed {
                                list[*at].1 = amount.parse().unwrap();
                            }
                            (manpower, Ok(list))
                        }
                        LandLossSide::Unknown { manpower, unknown } => {
                            (manpower, Err(unknown.clone()))
                        }
                    };
                    previous[index] = equipment.clone().ok();
                    let manpower = match manpower {
                        LandManpower::Known(value) => Ok(*value),
                        LandManpower::Unknown(reason) => Err(reason.clone()),
                    };
                    (manpower, equipment)
                })),
            };
            (read.hour, sides)
        })
        .collect()
}
/// The full sides a progress row supplies, labelled by `names[variant]`.
fn full_sides<'a>(names: &'a [String], row: &LandCombatProgressRow) -> [Side<'a>; 2] {
    [
        (row.attacker_manpower_loss, &row.attacker_equipment_losses),
        (row.defender_manpower_loss, &row.defender_equipment_losses),
    ]
    .map(|(manpower, equipment)| {
        let unknown = || "Not in this recording".to_string();
        (
            match manpower {
                Recorded::Value(value) => Ok(value),
                Recorded::NotRecorded => Err(unknown()),
            },
            match equipment {
                Recorded::Value(entries) => Ok(entries
                    .iter()
                    .map(|entry| (names[entry.variant as usize].as_str(), entry.amount_raw))
                    .collect()),
                Recorded::NotRecorded => Err(unknown()),
            },
        )
    })
}
#[test]
fn land_battles_compact_loss_reads_rebuild_every_full_read() {
    let row = |hour: i32, attacker: &[(u64, i64)], defender: Option<&[(u64, i64)]>| {
        let losses = |pairs: &[(u64, i64)]| -> Vec<LandEquipmentLoss> {
            pairs
                .iter()
                .map(|&(variant, amount_raw)| LandEquipmentLoss {
                    variant,
                    amount_raw,
                })
                .collect()
        };
        LandCombatProgressRow {
            combat: 1,
            attacker_manpower_loss: hour.into(),
            defender_manpower_loss: (hour + 1).into(),
            attacker_equipment_losses: losses(attacker).into(),
            defender_equipment_losses: defender
                .map_or(Recorded::NotRecorded, |pairs| losses(pairs).into()),
            ..Default::default()
        }
    };
    // Changed amounts, an unrecorded pool, reordering, removal and an unread hour.
    let rows = [
        Some(row(100, &[(7, -1), (8, -2)], Some(&[(9, -3)]))),
        Some(row(101, &[(7, -1), (8, -5)], None)),
        Some(row(102, &[(8, -5), (7, -1)], Some(&[(9, -4)]))),
        Some(row(103, &[(8, -6)], Some(&[(9, -4)]))),
        None,
        Some(row(105, &[(8, -7)], Some(&[(9, -4)]))),
    ];
    let mut census = Census::default();
    for (hour, row) in (100..).zip(&rows) {
        let mut sample = frame(hour, 5, 1);
        if let Some(row) = row {
            set_progress(&mut sample, vec![row.clone()]);
        }
        census.observe(&sample).unwrap();
    }
    let names: Vec<String> = (0..10).map(label).collect();
    let full: Vec<_> = (100..)
        .zip(&rows)
        .map(|(hour, row)| {
            let sides = row.as_ref().map(|row| full_sides(&names, row));
            (
                hour,
                sides.ok_or_else(|| "Hourly losses not recorded".to_string()),
            )
        })
        .collect();
    for first in [100, 101] {
        let reply = census.reply(&params(None), &windows(first, 105)).unwrap();
        assert_eq!(
            decode(&reply.labels, &reply.rows[0].losses),
            full[(first - 100) as usize..]
        );
    }
    let reply = census.reply(&params(None), &windows(100, 105)).unwrap();
    assert_eq!(reply.labels, [label(7), label(8), label(9)]);
    assert!(matches!(
        &reply.rows[0].losses[1].sides,
        LandLossSides::Read([LandLossSide::Changed { changed, .. }, LandLossSide::Unknown { .. }])
            if changed == &[(1, "-5".to_string())]
    ));
    assert!(matches!(
        &reply.rows[0].losses[3].sides,
        LandLossSides::Read([LandLossSide::Variants { .. }, LandLossSide::Changed { changed, .. }])
            if changed.is_empty()
    ));
}
/// Cumulative fixed5 losses: each side holds sixteen of 32 shared designs, and
/// half of its amounts change each hour.
fn busy_progress(combat: u32, elapsed: i32) -> LandCombatProgressRow {
    let side = |offset: u32| -> Recorded<Vec<LandEquipmentLoss>> {
        (0..16u32)
            .map(|k| LandEquipmentLoss {
                variant: u64::from(1 + (combat + offset + 2 * k) % 32),
                amount_raw: -i64::from(k + 1) * 12_345 * i64::from((elapsed as u32 + k) / 2 + 1),
            })
            .collect::<Vec<_>>()
            .into()
    };
    LandCombatProgressRow {
        combat: u64::from(combat),
        attacker_manpower_loss: (combat as i32 * 1000 + elapsed * 37).into(),
        defender_manpower_loss: (combat as i32 * 1000 + elapsed * 41).into(),
        attacker_equipment_losses: side(0),
        defender_equipment_losses: side(7),
        ..Default::default()
    }
}
#[test]
fn land_battles_busy_block_sends_changed_losses_far_below_their_full_reads() {
    let (battles, hours) = (30, replay::KEYFRAME_INTERVAL as i32);
    let mut sample = frame(100, 5000, battles);
    let mut census = Census::default();
    for elapsed in 0..hours {
        sample.hour = 100 + elapsed;
        let mut combats = sample.complete::<tables::LandCombats>().unwrap().to_vec();
        for combat in &mut combats {
            combat.duration = elapsed + 1;
        }
        *sample.tables[TableId::LandCombats.index()] =
            TableView::Complete(TableRows::LandCombats(combats));
        set_progress(
            &mut sample,
            (1..=battles)
                .map(|combat| busy_progress(combat, elapsed))
                .collect(),
        );
        census.observe(&sample).unwrap();
    }
    let reply = census
        .reply(&params(None), &windows(100, 99 + hours))
        .unwrap();
    let names: Vec<String> = (0..=32).map(label).collect();
    // Full reads repeated every variant's label and exact amount, beyond the budget.
    let mut full = 0;
    for row in &reply.rows {
        for (hour, sides) in decode(&reply.labels, &row.losses) {
            let sides = sides.unwrap();
            assert_eq!(
                sides,
                full_sides(&names, &busy_progress(row.province_id, hour - 100))
            );
            full += sides
                .iter()
                .flat_map(|(_, equipment)| equipment.as_ref().unwrap())
                .map(|(label, amount)| label.len() + amount.to_string().len())
                .sum::<usize>();
        }
    }
    let compact = json_len(&reply, MAX_PRODUCT_REPLY_BYTES).unwrap();
    assert!(full > MAX_PRODUCT_REPLY_BYTES, "{full}");
    assert!(compact * 4 < full, "{compact} bytes for {full}");
    assert!(compact * 2 < MAX_PRODUCT_REPLY_BYTES, "{compact}");
}
