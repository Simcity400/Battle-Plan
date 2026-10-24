use super::*;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::LandCombatProgressRow, rows::LandCombatRow,
    rows::LandCombatSideRow, rows::LandEquipmentLoss, rows::Recorded, rows::TableRows,
};
use replay_frame::frame::TableView;

mod forward;
mod speed;

fn frame(hour: i32, countries: u32, combats: u32) -> DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(0));
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
                attacker_classes: vec![0, 0],
                defender_classes: vec![0],
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
    let mut sample = frame(hour, 5, 1);
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
    let mut sample = frame(hour, 5, 0);
    *sample.tables[TableId::LandCombatResults.index()] =
        TableView::Complete(TableRows::LandCombatResults(vec![
            replay_data::rows::LandCombatResultsRow {
                id: 1,
                combat: 1,
                province: 1,
                end_hour: hour,
                duration: 13,
                winners,
                sides: std::array::from_fn(|index| replay_data::rows::LandResultSide {
                    countries: vec![index as u32 + 1],
                    ..Default::default()
                }),
                losses: std::array::from_fn(|index| replay_data::rows::LandSideLosses {
                    manpower: 10 + index as i32,
                    equipment: Vec::new(),
                }),
                ..Default::default()
            },
        ]));
    *sample.tables[TableId::LandEventCoverage.index()] =
        TableView::Complete(TableRows::LandEventCoverage(vec![
            replay_data::rows::LandEventCoverageRow {
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
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(
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
        matches!(&row.losses[0].sides, Ok([attacker, _]) if attacker.equipment == Some(vec![(0, -100_000)]))
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
    let LandLossTotals::Read([attacker, defender]) = &reply.rows[0].losses[0].sides else {
        panic!("recorded counters");
    };
    assert!(
        matches!(attacker, LandLossTotal::Unknown { manpower: LandManpower::Known(4), unknown } if unknown == "Not in this recording")
    );
    assert!(
        matches!(defender, LandLossTotal::Known { manpower: LandManpower::Known(5), equipment } if equipment == "-9223372036854775807")
    );
    let id = census.episodes[0].id.clone();
    let course = census.course(&id).unwrap();
    assert_eq!(course.designs, [design(8)]);
    let LandLossSides::Read([attacker, defender]) = &course.battle.losses[0].sides else {
        panic!("recorded counters");
    };
    assert!(matches!(attacker, LandLossSide::Unknown { .. }));
    assert!(
        matches!(defender, LandLossSide::Variants { manpower: LandManpower::Known(5), variants } if variants == &[(0, "-9223372036854775807".to_string())])
    );
}
#[test]
fn land_battles_census_rows_carry_exact_equipment_totals_without_designs() {
    let mut sample = loss_frame(100, 4);
    if let TableView::Complete(TableRows::LandCombatProgress(rows)) =
        &mut *sample.tables[TableId::LandCombatProgress.index()]
    {
        // Two amounts whose sum leaves i64.
        rows[0].defender_equipment_losses = vec![
            LandEquipmentLoss {
                variant: 8,
                amount_raw: i64::MIN + 1,
            },
            LandEquipmentLoss {
                variant: 9,
                amount_raw: i64::MIN + 1,
            },
        ]
        .into();
    }
    let mut census = Census::default();
    census.observe(&sample).unwrap();
    let reply = census.reply(&params(None), &windows(100, 100)).unwrap();
    let LandLossTotals::Read([attacker, defender]) = &reply.rows[0].losses[0].sides else {
        panic!("recorded counters");
    };
    assert!(matches!(attacker, LandLossTotal::Known { equipment, .. } if equipment == "-100000"));
    assert!(
        matches!(defender, LandLossTotal::Known { equipment, .. } if equipment == "-18446744073709551614")
    );
    let ranking = census
        .ranked(
            &ranking_params(100, 100, 1, "Casualties"),
            &AtomicBool::new(false),
        )
        .unwrap();
    for text in [
        serde_json::to_string(&reply).unwrap(),
        serde_json::to_string(&ranking).unwrap(),
    ] {
        for field in ["labels", "variants", "changed", "designs"] {
            assert!(!text.contains(field), "{field} in {text}");
        }
    }
}
#[test]
fn land_battles_course_names_designs_without_their_handles() {
    let mut census = Census::default();
    census.observe(&loss_frame(100, 4)).unwrap();
    assert!(
        census
            .course("1:1:absent")
            .unwrap_err()
            .to_string()
            .contains("not in this recording")
    );
    assert!(
        LandBattleCourses::validate(&LandBattleCourseParams {
            branch_id: "1".into(),
            id: String::new(),
        })
        .is_err()
    );
    let id = census.episodes[0].id.clone();
    let course = census.course(&id).unwrap();
    // Without content the name stays unknown; the handle is never part of it.
    assert_eq!(course.designs, [design(7)]);
    assert_eq!(course.battle.runs, census.episodes[0].runs);
    assert_eq!(
        serde_json::to_value(&course.designs).unwrap(),
        serde_json::json!([{ "name": null, "variant": "7" }])
    );
}
#[test]
fn land_battles_rows_carry_each_sides_most_divisions_at_once() {
    let mut census = Census::default();
    for hour in 100..=103 {
        census.observe(&frame(hour, 5, 1)).unwrap();
    }
    let run = census.episodes[0].runs[0].clone();
    census.episodes[0].runs = [(100, 100, [5, 1]), (101, 101, [2, 6]), (102, 103, [1, 1])]
        .map(|(first_hour, last_hour, counts)| LandBattleRun {
            first_hour,
            last_hour,
            counts,
            ..run.clone()
        })
        .to_vec();
    let windows = [
        BattleWindow {
            first_hour: 100,
            last_hour: 101,
        },
        BattleWindow {
            first_hour: 102,
            last_hour: 103,
        },
    ];
    let peaks = |row: &LandBattleHistory| {
        [
            &row.prior_peak,
            &row.prior_side_peaks[0],
            &row.prior_side_peaks[1],
        ]
        .map(|peak| peak.as_ref().map(|peak| (peak.value, peak.hour)))
    };
    let reply = census.reply(&params(Some(1)), &windows).unwrap();
    assert_eq!(
        peaks(&reply.rows[0]),
        [Some((8, 101)), Some((5, 100)), Some((6, 101))]
    );
    let first = census.reply(&params(Some(0)), &windows).unwrap();
    assert_eq!(peaks(&first.rows[0]), [None, None, None]);
    let ranking = census
        .ranked(
            &ranking_params(100, 100, 1, "Peak size"),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(
        peaks(&ranking.rows[0]),
        [Some((6, 100)), Some((5, 100)), Some((1, 100))]
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
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[dashboard_base::test_fixtures::records(1, vec![sample])],
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
    census.observe(&frame(99, 5, 0)).unwrap();
    let mut sample = frame(100, 5, 1);
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
    census.observe(&frame(100, 5, 1)).unwrap();
    let mut missing = frame(101, 5, 1);
    *missing.tables[TableId::LandCombats.index()] = TableView::Unavailable {
        reason: "missing".into(),
    };
    census.observe(&missing).unwrap();
    census.observe(&frame(105, 5, 1)).unwrap();
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
        census.observe(&frame(hour, 5, 1)).unwrap();
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
    census.observe(&frame(100, 5, 3000)).unwrap();
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
fn land_battles_missing_side_data_is_unknown_not_empty() {
    let mut census = Census::default();
    let mut sample = frame(100, 5, 1);
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
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(
            1,
            vec![
                frame(100, 5, 1),
                frame(101, 5, 0),
                frame(102, 5, 1),
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
fn land_battles_ranking_keeps_hourly_participants_for_a_result_side_without_countries() {
    let mut census = Census::default();
    let mut sample = frame(100, 5, 1);
    if let TableView::Complete(TableRows::LandCombatSides(rows)) =
        &mut *sample.tables[TableId::LandCombatSides.index()]
    {
        rows[0].defender_primary = vec![3];
    }
    census.observe(&sample).unwrap();
    let mut ended = result_frame(101, 1);
    if let TableView::Complete(TableRows::LandCombatResults(rows)) =
        &mut *ended.tables[TableId::LandCombatResults.index()]
    {
        rows[0].sides[1].countries.clear();
    }
    census.observe(&ended).unwrap();
    let result = census.episodes[0].result.as_ref().unwrap();
    assert_eq!(result.countries_unknown, [false, true]);
    let mut params = ranking_params(100, 101, 10, "Peak size");
    // The hourly read recorded the defender, so an unrelated country no longer matches.
    params.countries.tags = Some(vec!["mod country 4".into()]);
    let ranked = |params: &LandBattlesRankingParams| {
        census
            .ranked(params, &AtomicBool::new(false))
            .unwrap()
            .rows
            .len()
    };
    assert_eq!(ranked(&params), 0);
    params.countries.tags = Some(vec!["mod country 3".into()]);
    assert_eq!(ranked(&params), 1);
    // The defender matches at its own read, an hour before the result.
    params.countries.tags = None;
    params.countries.membership = Some(vec![
        dashboard_base::products::country_groups::MembershipRun {
            tag: "mod country 3".into(),
            first_hour: 100,
            last_hour: 100,
        },
    ]);
    assert_eq!(ranked(&params), 1);
    params.countries.unobserved = vec![dashboard_base::products::country_groups::HourSpan {
        first_hour: 100,
        last_hour: 100,
    }];
    assert_eq!(ranked(&params), 0);
    // A side unknown in both reads still survives every filter.
    let mut unknown = Census::default();
    let mut sample = frame(100, 5, 1);
    if let TableView::Complete(TableRows::LandCombatSides(rows)) =
        &mut *sample.tables[TableId::LandCombatSides.index()]
    {
        rows[0].defender_primary = vec![9999];
    }
    unknown.observe(&sample).unwrap();
    unknown.observe(&ended).unwrap();
    params.countries.membership = None;
    params.countries.unobserved.clear();
    params.countries.tags = Some(vec!["mod country 4".into()]);
    assert_eq!(
        unknown
            .ranked(&params, &AtomicBool::new(false))
            .unwrap()
            .rows
            .len(),
        1
    );
}
#[test]
fn land_battles_availability_stops_before_later_invalid_campaign_read() {
    let mut invalid = frame(101, 5, 1);
    *invalid.tables[TableId::Campaign.index()] = TableView::Unavailable {
        reason: "later bad read".into(),
    };
    let replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(
            1,
            vec![frame(100, 5, 0), invalid],
        )],
    )
    .unwrap();
    assert!(recording_available(&replay, 1, &AtomicBool::new(false)).unwrap());
    assert!(recording_available(&replay, 1, &AtomicBool::new(true)).is_err());
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
            manpower: Some(value),
            equipment: Some(Vec::new()),
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
        matches!(&reply.rows[2].losses[0].sides, LandLossTotals::Unread(reason) if reason == "Hourly losses not recorded")
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
    census.episodes[0].losses[0].sides.as_mut().unwrap()[1].manpower = None;
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
    let mut places = dashboard_base::products::place_names::PlaceNamesReply {
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
    format!("variant {variant}")
}
fn design(variant: u64) -> LandEquipmentDesign {
    LandEquipmentDesign {
        name: None,
        variant: variant.to_string(),
    }
}
/// Each design's test label, by its handle.
fn design_labels(designs: &[LandEquipmentDesign]) -> Vec<String> {
    designs
        .iter()
        .map(|design| label(design.variant.parse().unwrap()))
        .collect()
}
/// One full side: manpower, then each variant's label and exact amount; reasons stay text.
type Side<'a> = (Result<i32, String>, Result<Vec<(&'a str, i64)>, String>);
/// Rebuilds every read's full sides, applying a side's changes to its previous read.
fn decode<'a>(
    labels: &'a [String],
    reads: &[LandBattleCourseRead],
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
    let id = census.episodes[0].id.clone();
    let course = census.course(&id).unwrap();
    let labels = design_labels(&course.designs);
    assert_eq!(decode(&labels, &course.battle.losses), full);
    assert_eq!(course.designs, [design(7), design(8), design(9)]);
    assert!(matches!(
        &course.battle.losses[1].sides,
        LandLossSides::Read([LandLossSide::Changed { changed, .. }, LandLossSide::Unknown { .. }])
            if changed == &[(1, "-5".to_string())]
    ));
    assert!(matches!(
        &course.battle.losses[3].sides,
        LandLossSides::Read([LandLossSide::Variants { .. }, LandLossSide::Changed { changed, .. }])
            if changed.is_empty()
    ));
    // The census window keeps one prior read and each read's exact totals.
    for first in [100, 101] {
        let reply = census.reply(&params(None), &windows(first, 105)).unwrap();
        let totals: Vec<_> = reply.rows[0]
            .losses
            .iter()
            .map(|read| {
                let sides = match &read.sides {
                    LandLossTotals::Read(sides) => Ok(sides.each_ref().map(|side| match side {
                        LandLossTotal::Known { equipment, .. } => {
                            Ok(equipment.parse::<i128>().unwrap())
                        }
                        LandLossTotal::Unknown { unknown, .. } => Err(unknown.clone()),
                    })),
                    LandLossTotals::Unread(reason) => Err(reason.clone()),
                };
                (read.hour, sides)
            })
            .collect();
        let expected: Vec<_> = full[(first - 100) as usize..]
            .iter()
            .map(|(hour, sides)| {
                let sides = sides.clone().map(|sides| {
                    sides.map(|(_, equipment)| {
                        equipment.map(|list| {
                            list.iter()
                                .map(|(_, amount)| i128::from(*amount))
                                .sum::<i128>()
                        })
                    })
                });
                (*hour, sides)
            })
            .collect();
        assert_eq!(totals, expected);
    }
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
    let (battles, hours) = (30, replay_frame::KEYFRAME_INTERVAL as i32);
    let mut sample = frame(100, 5, battles);
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
    // The census sends each read's totals; per-design amounts stay in the course.
    let text = serde_json::to_string(&reply).unwrap();
    assert!(!text.contains("variants") && !text.contains("changed"));
    for row in &reply.rows {
        for read in &row.losses {
            let LandLossTotals::Read(sides) = &read.sides else {
                panic!("recorded counters");
            };
            let expected = full_sides(&names, &busy_progress(row.province_id, read.hour - 100));
            for (side, (_, equipment)) in sides.iter().zip(expected) {
                let sum: i128 = equipment
                    .unwrap()
                    .iter()
                    .map(|(_, amount)| i128::from(*amount))
                    .sum();
                assert!(
                    matches!(side, LandLossTotal::Known { equipment, .. } if *equipment == sum.to_string())
                );
            }
        }
    }
    let window = json_len(&reply, MAX_PRODUCT_REPLY_BYTES).unwrap();
    assert!(window * 4 < MAX_PRODUCT_REPLY_BYTES, "{window}");
    // One battle's course lists only changed amounts after its first read.
    let id = census.episodes[0].id.clone();
    let course = census.course(&id).unwrap();
    let labels = design_labels(&course.designs);
    let mut full = 0;
    for (hour, sides) in decode(&labels, &course.battle.losses) {
        let sides = sides.unwrap();
        assert_eq!(sides, full_sides(&names, &busy_progress(1, hour - 100)));
        full += sides
            .iter()
            .map(|(_, equipment)| equipment.as_ref().unwrap().len())
            .sum::<usize>();
    }
    let sent = course
        .battle
        .losses
        .iter()
        .flat_map(|read| match &read.sides {
            LandLossSides::Read(sides) => sides.iter().collect::<Vec<_>>(),
            LandLossSides::Unread(_) => Vec::new(),
        })
        .map(|side| match side {
            LandLossSide::Variants { variants, .. } => variants.len(),
            LandLossSide::Changed { changed, .. } => changed.len(),
            LandLossSide::Unknown { .. } => 0,
        })
        .sum::<usize>();
    assert!(sent * 3 < full * 2, "{sent} amounts for {full}");
}

#[test]
fn prepared_land_matches_oracle_across_commits_resume_append_and_census() {
    use dashboard_base::prepared::series::{self, tests::Fixture};
    let flag = AtomicBool::new(false);
    let mut inputs = Vec::new();
    for hour in 100..104 {
        let mut sample = frame(hour, 5, 40);
        if let TableView::Complete(TableRows::LandCombats(rows)) =
            &mut *sample.tables[TableId::LandCombats.index()]
        {
            for row in rows {
                row.duration = 12 + hour - 100;
            }
        }
        inputs.push(sample);
    }
    // A delayed result dates its loss read before the block that observes it.
    let mut result = result_frame(104, 3);
    if let TableView::Complete(TableRows::LandCombatResults(rows)) =
        &mut *result.tables[TableId::LandCombatResults.index()]
    {
        rows[0].end_hour = 103;
    }
    if let TableView::Complete(TableRows::LandEventCoverage(rows)) =
        &mut *result.tables[TableId::LandEventCoverage.index()]
    {
        rows[0].from_hour = 102;
    }
    inputs.push(result);
    let mut replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(
            1,
            vec![inputs[0].clone()],
        )],
    )
    .unwrap();
    for input in &inputs[1..] {
        replay = replay
            .append(&[dashboard_base::test_fixtures::continuation_records(
                &replay,
                1,
                vec![input.clone()],
            )])
            .unwrap();
    }
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "prepared-land".into(),
            revision: "1".into(),
        },
        "prepared-land.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let fixture = Fixture::new();
    let mut prepared = Census::new(source.replay_ref().format);
    for (index, input) in inputs.iter().enumerate() {
        prepared.configure(&source, &LoadContext { content: None });
        prepared.observe(input).unwrap();
        let mut writer = fixture.writer();
        prepared
            .publish_block(&mut writer, source.replay_ref(), 1, index, &flag)
            .unwrap();
        writer.commit().unwrap();
        // Every committed state can resume without observing its accepted prefix.
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    let mut oracle = scan(source.replay_ref(), 1, &flag, None).unwrap();
    oracle
        .content_warnings
        .push("Game content unavailable; land equipment names are not resolved.".into());
    let expected_windows = super::windows(source.replay_ref(), 1);
    for index in 0..expected_windows.len() {
        let params = params(Some(index));
        let actual = prepared::query(
            &source,
            "land-battles",
            serde_json::to_value(&params).unwrap(),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(
            actual,
            serde_json::to_value(oracle.reply(&params, &expected_windows).unwrap()).unwrap()
        );
    }
    for measure in ["Peak size", "Length", "Casualties"] {
        for last_hour in 100..=104 {
            let params = LandBattlesRankingParams {
                branch_id: "1".into(),
                first_hour: 101,
                last_hour: last_hour.max(101),
                top_n: 12.0,
                measure: measure.into(),
                countries: LandBattleCountryFilter {
                    tags: None,
                    membership: None,
                    unobserved: vec![],
                },
            };
            let actual = prepared::query(
                &source,
                "land-battles-ranking",
                serde_json::to_value(&params).unwrap(),
                &fixture.store,
                &fixture.key,
                &flag,
            )
            .unwrap();
            assert_eq!(
                actual,
                serde_json::to_value(oracle.ranked(&params, &flag).unwrap()).unwrap()
            );
        }
    }
    for episode in oracle.episodes.iter().take(3) {
        let params = LandBattleCourseParams {
            branch_id: "1".into(),
            id: episode.id.clone(),
        };
        let actual = prepared::query(
            &source,
            "land-battle-course",
            serde_json::to_value(params).unwrap(),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(
            actual,
            serde_json::to_value(oracle.course(&episode.id).unwrap()).unwrap()
        );
    }
    let (keys, ()) =
        series::read_catalogue::<()>(&fixture.store, &fixture.key, "land-battle-keys", 1)
            .unwrap()
            .unwrap();
    assert!(!keys.contains_key("missing"));
}

#[test]
fn prepared_land_ranking_keeps_gap_endings_and_census_reads_in_unrecorded_windows() {
    use dashboard_base::prepared::series::tests::Fixture;
    let flag = AtomicBool::new(false);
    let first = frame(100, 5, 1);
    let last = frame(104, 5, 0);
    let original = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(
            1,
            vec![first.clone()],
        )],
    )
    .unwrap();
    let replay = original
        .append(&[dashboard_base::test_fixtures::continuation_records(
            &original,
            1,
            vec![last.clone()],
        )])
        .unwrap();
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "land-gap".into(),
            revision: "1".into(),
        },
        "land-gap.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let fixture = Fixture::new();
    let context = LoadContext { content: None };
    let mut prepared = Census::new(source.replay_ref().format);
    for (index, input) in [&first, &last].into_iter().enumerate() {
        prepared.configure(&source, &context);
        prepared.observe(input).unwrap();
        let mut writer = fixture.writer();
        prepared
            .publish_block(&mut writer, source.replay_ref(), 1, index, &flag)
            .unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    let mut oracle = scan(source.replay_ref(), 1, &flag, None).unwrap();
    oracle
        .content_warnings
        .push("Game content unavailable; land equipment names are not resolved.".into());
    for (first_hour, last_hour) in [(101, 102), (102, 103), (101, 104), (105, 110)] {
        let params = LandBattlesRankingParams {
            branch_id: "1".into(),
            first_hour,
            last_hour,
            top_n: 5.0,
            measure: "Length".into(),
            countries: LandBattleCountryFilter {
                tags: None,
                membership: None,
                unobserved: vec![],
            },
        };
        let actual = prepared::query(
            &source,
            "land-battles-ranking",
            serde_json::to_value(&params).unwrap(),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(
            actual,
            serde_json::to_value(oracle.ranked(&params, &flag).unwrap()).unwrap()
        );
    }
}

/// Fresh censuses of one recording number equipment designs in combat order,
/// so their ranking pages and saved state match byte for byte.
#[test]
fn land_battles_fresh_censuses_number_designs_in_combat_order() {
    let combats = 64;
    let mut sample = frame(100, 70, combats);
    set_progress(
        &mut sample,
        (1..=u64::from(combats))
            .map(|combat| LandCombatProgressRow {
                combat,
                attacker_manpower_loss: 4.into(),
                defender_manpower_loss: 5.into(),
                attacker_equipment_losses: vec![LandEquipmentLoss {
                    variant: 1_000 + combat,
                    amount_raw: -100_000,
                }]
                .into(),
                ..Default::default()
            })
            .collect(),
    );
    let states: Vec<_> = (0..2)
        .map(|_| {
            let mut census = Census::default();
            census.observe(&sample).unwrap();
            assert_eq!(
                census
                    .designs
                    .values
                    .iter()
                    .map(|design| design.variant.clone())
                    .collect::<Vec<_>>(),
                (1..=u64::from(combats))
                    .map(|combat| (1_000 + combat).to_string())
                    .collect::<Vec<_>>()
            );
            serde_json::to_vec(&census).unwrap()
        })
        .collect();
    assert_eq!(states[0], states[1]);
}

#[test]
fn duration_deltas_reuse_roster_counts_and_match_full_observation_across_resume_and_gaps() {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, prepass};
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder};
    let format = replay_data::format::FormatVersion::CURRENT;
    let (mut sparse, mut whole) = (Census::new(format), Census::new(format));
    let mut encoder = FrameEncoder::default();
    let mut decoder = FrameDecoder::default();
    let mut tracker = FrameChangeTracker::new();
    let mut input = frame(100, 3, 2);
    for step in 0..8 {
        input.hour = 100 + step + i32::from(step >= 6);
        // These are hourly combat reads, including their recorded slot evidence.
        input.metrics.table_ns[TableId::LandCombats.index()] = 1;
        if let TableView::Complete(TableRows::LandCombats(rows)) =
            &mut *input.tables[TableId::LandCombats.index()]
        {
            for row in rows.iter_mut() {
                row.duration += 1;
            }
            if step == 3 {
                rows[0].attackers.extend([777, 777]);
            }
        }
        let raw = Arc::new(
            encoder
                .encode(input.clone(), [0, 4].contains(&step))
                .unwrap(),
        );
        let decoded = decoder.advance(&raw).unwrap();
        let changes = tracker.observe_updates(decoded, raw.clone(), format);
        let before = prepass::noted("land.rosters");
        sparse.observe_prepared(decoded, &changes).unwrap();
        if step == 1 || step == 2 || step == 5 || step == 7 {
            assert_eq!(
                prepass::noted("land.rosters"),
                before,
                "step {step} rebuilt unchanged rosters"
            );
        }
        whole.observe(decoded).unwrap();
        assert_eq!(
            serde_json::to_vec(&sparse).unwrap(),
            serde_json::to_vec(&whole).unwrap(),
            "step {step}"
        );
        assert_eq!(
            serde_json::to_vec(
                &sparse
                    .reply_window(0, &windows(100, input.hour)[0])
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_vec(&whole.reply_window(0, &windows(100, input.hour)[0]).unwrap())
                .unwrap(),
        );
        if step == 4 {
            let restore = |scan: &Census| {
                let mut restored: Census =
                    serde_json::from_slice(&serde_json::to_vec(scan).unwrap()).unwrap();
                restored.format = format;
                restored
            };
            sparse = restore(&sparse);
            whole = restore(&whole);
        }
    }
}
