use super::*;
use dashboard_query::identity::Holders;

#[test]
fn a_battle_id_names_its_branch_combat_and_start_and_a_roster_counts_each_unit_once() {
    assert_eq!(parse_id("1:156968169766974:60864620", 1), Some((156_968_169_766_974, 60_864_620, None)));
    assert_eq!(parse_id("1:5:7:8", 1), Some((5, 7, Some(8))), "a result-only row names the hour it was written");
    assert_eq!(parse_id("2:5:7", 1), None);
    assert_eq!(parse_id("1:5", 1), None);
    assert_eq!(parse_id("1:5:7:x", 1), None);
    assert_eq!(parse_id("1:5:7:8:9", 1), None);
    let roster: Vec<u8> = [7u64, 9, 7].iter().flat_map(|handle| handle.to_le_bytes()).collect();
    assert_eq!(distinct(&roster), Some(2));
    assert_eq!(distinct(&roster[..5]), None);
}

#[test]
fn an_unread_roster_keeps_the_other_sides_count_and_splits_known_zero_from_unknown() {
    let battle = Battle { combat: 7, start: 10, exact: true, live: vec![(10, 14)], ..Battle::default() };
    let data = Combats {
        from: 10,
        census: HashMap::from([(7, vec![
            (10, Some(CensusRead { province: Some(3), counts: [Some(2), None] })),
            (11, Some(CensusRead { province: Some(3), counts: [None, Some(1)] })),
            (12, Some(CensusRead { province: Some(3), counts: [Some(0), None] })),
            (13, Some(CensusRead::default())),
            (14, None),
        ])]),
        durations: HashMap::from([(7, (10..=14).map(|hour| (hour, Some(hour - 10))).collect())]),
        ..Combats::default()
    };
    let names = Names::new(&Holders::default());
    let links = dashboard_base::queries::country_ids::RunLinks { slots: HashMap::new() };
    let runs: Vec<_> = shape(&battle, &data, &names, &links, 1, 14).battle.runs.into_iter()
        .map(|run| (run.first_hour, run.last_hour, run.counts, run.counts_unknown)).collect();
    assert_eq!(runs, [
        (10, 10, [2, 0], [false, true]),
        (11, 11, [0, 1], [true, false]),
        (12, 12, [0, 0], [false, true]),
        (13, 14, [0, 0], [true, true]),
    ]);
}

#[test]
fn every_written_end_row_is_an_end_of_its_own() {
    let row = |id: i64| Some(vec![Cell::Number(i64::from(LandBattleEndRow::RESULT_ONLY)), Cell::Number(10), Cell::Number(id)].into_boxed_slice());
    let battle = 7_u128 << 33 | 10;
    let alone = battle | 1 << 32;
    let step = |hour, cells| lanes::Step { hour, cells };
    let ended = lanes::History {
        columns: vec![0, 1, 18],
        to: 13,
        texts: Vec::new(),
        keys: vec![(battle, vec![step(12, row(59))]), (alone, vec![step(10, row(60)), step(11, row(61)), step(12, None)])],
        ..Default::default()
    };
    let found: Vec<(Key, i32, Option<i64>)> = written_ends(&ended).map(|(key, _, hour, cells)| (key, hour, match cells[2] { Cell::Number(id) => Some(id), _ => None })).collect();
    assert_eq!(found, [((7, 10, None), 12, Some(59)), ((7, 10, Some(10)), 10, Some(60)), ((7, 10, Some(11)), 11, Some(61))]);
}

#[test]
fn unread_end_kind_hour_side_lists_and_lengths_stay_unknown_in_the_course() {
    let end_cells = |set: &[(usize, Cell)]| -> Box<[Cell]> {
        let mut cells = vec![Cell::Missing; 27];
        for &(column, value) in set {
            cells[column] = value;
        }
        cells.into()
    };
    let battle_key = |combat: u64, start: u32, result_only: bool| -> u128 {
        u128::from(combat) << 33 | if result_only { 1_u128 << 32 } else { 0 } | u128::from(start)
    };
    let ended = lanes::History {
        columns: (0..27).collect(),
        to: 30,
        texts: Vec::new(),
        keys: vec![
            (
                battle_key(8, 10, false),
                vec![lanes::Step {
                    hour: 25,
                    cells: Some(end_cells(&[
                        (0, Cell::Unrecorded),
                        (1, Cell::Number(20)),
                        (18, Cell::Number(1)),
                        (20, Cell::Number(1)),
                        (26, Cell::Number(0)),
                    ])),
                }],
            ),
            (
                battle_key(9, 10, false),
                vec![lanes::Step {
                    hour: 25,
                    cells: Some(end_cells(&[
                        (0, Cell::Number(i64::from(LandBattleEndRow::FINALIZED))),
                        (1, Cell::Unrecorded),
                        (18, Cell::Number(2)),
                        (20, Cell::Number(2)),
                        (26, Cell::Number(0)),
                    ])),
                }],
            ),
            (
                battle_key(10, 10, false),
                vec![lanes::Step {
                    hour: 25,
                    cells: Some(end_cells(&[(0, Cell::Number(i64::from(LandBattleEndRow::UNKNOWN))), (1, Cell::Unrecorded)])),
                }],
            ),
            (
                battle_key(11, 10, false),
                vec![lanes::Step {
                    hour: 25,
                    cells: Some(end_cells(&[(0, Cell::Number(i64::from(LandBattleEndRow::UNKNOWN))), (1, Cell::Number(22))])),
                }],
            ),
            (
                battle_key(7, 10, true),
                vec![lanes::Step {
                    hour: 25,
                    cells: Some(end_cells(&[
                        (0, Cell::Number(i64::from(LandBattleEndRow::RESULT_ONLY))),
                        (1, Cell::Unrecorded),
                        (4, Cell::Unrecorded),
                        (18, Cell::Number(77)),
                        (20, Cell::Number(1)),
                    ])),
                }],
            ),
        ],
        ..Default::default()
    };
    let mut decoded = BTreeMap::new();
    decode_ended_rows(&ended, &mut decoded);
    let mut census = HashMap::new();
    census.insert(7, vec![(10, Some(CensusRead { province: Some(7), counts: [Some(2), Some(2)] }))]);
    census.insert(12, vec![(10, Some(CensusRead { province: Some(7), counts: [Some(2), Some(2)] }))]);
    let mut durations = HashMap::new();
    durations.insert(12, vec![(10, Some(1)), (11, Some(2)), (12, None), (14, Some(5)), (15, Some(6)), (16, Some(9)), (18, Some(10)), (19, Some(11)), (20, Some(12)), (22, None)]);
    let data = Combats { from: 10, census, sides: HashMap::new(), leads: HashMap::new(), losses: HashMap::new(), durations };
    let names = Names::new(&Holders::default());
    let links = dashboard_base::queries::country_ids::RunLinks { slots: HashMap::new() };

    let unread_kind = &decoded[&(8, 10, None)];
    assert_eq!(unread_kind.end.as_ref().unwrap().kind, None, "production decoder keeps unread end kind as None");
    let shaped = shape(unread_kind, &data, &names, &links, 1, 30).battle;
    assert_eq!(shaped.end, Some(BattleEnd::Unknown { unknown: "Ending kind not recorded".into() }));
    assert_eq!(shaped.end_unknown_hour, Some(25));
    let result = shaped.result.as_ref().unwrap();
    assert_eq!(result.countries_unknown, [true, true], "unread side lists keep both sides unknown even when the bitmask is zero");

    let mut unread_hour = decoded[&(9, 10, None)].clone();
    assert_eq!(unread_hour.end.as_ref().unwrap().end_hour, None, "production decoder keeps unread end hour as None");
    unread_hour.end.as_mut().unwrap().result_sides = [Some(vec![1]), None];
    let shaped = shape(&unread_hour, &data, &names, &links, 1, 30).battle;
    assert_eq!(shaped.end, Some(BattleEnd::Unknown { unknown: "Ending hour not recorded".into() }));
    assert_eq!(shaped.end_unknown_hour, Some(25));
    let result = shaped.result.as_ref().unwrap();
    assert_eq!(result.hour, None, "an unread result hour never copies the settlement hour");
    assert_eq!(result.duration, None, "an unread result length is not zero");
    assert_eq!(result.winner, BattleWinner::Side { side: 1 });
    assert_eq!(result.countries_unknown[1], true, "an unread defender side list stays unknown");

    let unknown_unread_hour = shape(&decoded[&(10, 10, None)], &data, &names, &links, 1, 30).battle;
    assert_eq!(unknown_unread_hour.end, Some(BattleEnd::Unknown { unknown: "Ending hour not recorded".into() }));
    assert_eq!(unknown_unread_hour.end_unknown_hour, Some(25));

    let unknown_dated = shape(&decoded[&(11, 10, None)], &data, &names, &links, 1, 30).battle;
    assert_eq!(unknown_dated.end, Some(BattleEnd::Unknown { unknown: "Ending not recorded".into() }));
    assert_eq!(unknown_dated.end_unknown_hour, Some(22));

    let result_only = shape(&decoded[&(7, 10, Some(25))], &data, &names, &links, 1, 30).battle;
    assert_eq!(result_only.province_id, None, "a result-only key never inherits a live battle's census province");
    assert_eq!(result_only.end, None, "a result-only key keeps no battle end");

    let lengths = Battle { combat: 12, start: 10, exact: true, live: vec![(10, 22)], ..Battle::default() };
    let runs: Vec<(i32, i32, Option<i32>)> = shape(&lengths, &data, &names, &links, 1, 30).battle.runs.iter().map(|run| (run.first_hour, run.last_hour, run.duration)).collect();
    assert_eq!(runs, [(10, 11, Some(1)), (12, 13, None), (14, 15, Some(5)), (16, 16, Some(9)), (17, 20, Some(9)), (21, 21, Some(12)), (22, 22, None)], "a length that turns unread at an unchanged census is missing for those hours, never carried on, and a jump or a hold starts a run of its own");

    let split = Battle { combat: 12, start: 10, exact: true, live: vec![(10, 12), (16, 18)], ..Battle::default() };
    let spans: Vec<(i32, i32)> = shape(&split, &data, &names, &links, 1, 30).battle.runs.iter().map(|run| (run.first_hour, run.last_hour)).collect();
    assert_eq!(spans.first().map(|span| span.0), Some(10));
    assert_eq!(spans.last().map(|span| span.1), Some(18));
    assert!(spans.iter().all(|&(first, last)| last < 13 || first > 15), "the hours between a battle's live stretches are no run, so they show as not recorded: {spans:?}");

    let result_only_runs = &result_only.runs;
    assert!(result_only_runs.iter().all(|run| run.counts_unknown == [true, true] && run.counts == [0, 0] && run.countries_unknown == [true, true]), "a result-only key never borrows a live battle's divisions or countries: {result_only_runs:?}");

    let mut sides = HashMap::new();
    sides.insert(13, vec![(10, Some([Some(vec![7]), Some(vec![8])]))]);
    let mut leads = HashMap::new();
    leads.insert((10, 13), [Some(7), Some(8)]);
    let relinked = Combats { from: 10, census: HashMap::new(), sides, leads, losses: HashMap::new(), durations: HashMap::new() };
    let links = dashboard_base::queries::country_ids::RunLinks { slots: HashMap::from([(7, vec![(10, 14, Some(3)), (15, 30, Some(4))]), (8, vec![(10, 30, Some(5))])]) };
    let names = Names { slots: HashMap::from([(3, vec![("AAA".to_owned(), 10, 30)]), (4, vec![("BBB".to_owned(), 10, 30)]), (5, vec![("CCC".to_owned(), 10, 17), ("DDD".to_owned(), 18, 30)])]) };
    let battle = Battle { combat: 13, start: 10, exact: true, live: vec![(10, 20)], ..Battle::default() };
    let runs: Vec<(i32, i32, Option<u32>, [Vec<String>; 2])> = shape(&battle, &relinked, &names, &links, 1, 30).battle.runs.into_iter().map(|run| (run.first_hour, run.last_hour, run.leads[0].as_ref().map(|lead| lead.slot), run.countries)).collect();
    assert_eq!(runs, [
        (10, 14, Some(3), [vec!["AAA".to_owned()], vec!["CCC".to_owned()]]),
        (15, 17, Some(4), [vec!["BBB".to_owned()], vec!["CCC".to_owned()]]),
        (18, 20, Some(4), [vec!["BBB".to_owned()], vec!["DDD".to_owned()]]),
    ], "a country's slot or tag changing in the battle starts a run, so its lead and countries are read at their own hour");

}

#[test]
fn a_battle_course_carries_its_opening_roster_and_each_exit_and_names_what_was_not_recorded() {
    use replay_data::rows::LandCombatEdgesRow as Edge;
    let edge = |id, kind, hour, division: Option<u64>, side, values: [Option<i32>; 3], withheld| Edge {
        id, kind, hour, combat: 9, division, side, manpower: values[0], casualties_before: values[1], casualties_after: values[2], withheld, pending_strength_raw: None,
    };
    let rows = [
        edge(1, Edge::OPENED, 10, Some(5), 0, [Some(9000), Some(300), None], 0),
        edge(2, Edge::OPENED, 10, None, 1, [None, None, None], Edge::DIVISION_UNREADABLE),
        edge(3, Edge::EXITED, 12, Some(5), 0, [Some(8500), Some(300), Some(812)], 0),
        edge(4, Edge::EXITED, 13, Some(6), 1, [Some(100), Some(40), None], 0),
    ];
    let battle = Battle { combat: 9, start: 11, exact: true, ..Battle::default() };
    let edges = shape_edges(&rows, &[(10, 12)], &[(13, false), (14, true)], &battle, 10, 14);
    let opening = edges.opening.expect("an opening");
    assert_eq!(opening.hour, 10);
    assert_eq!(opening.members[0], LandEdgeMember { division: Some("5".into()), side: 0, manpower: Some(9000), casualties: Some(300), missing: None });
    assert!(opening.members[1].missing.as_deref().is_some_and(|missing| missing.starts_with("The division could not be read")));
    assert_eq!(edges.opening_missing, None);
    assert_eq!(edges.exits[0].losses, Some(512), "losses are the casualties the exit realised");
    assert_eq!(edges.exits[1].losses, None);
    assert_eq!(edges.exits[1].missing.as_deref(), Some(CASUALTIES_UNREAD));
    let partial = Edge { manpower: None, withheld: Edge::HOUR_UNREADABLE, ..rows[2].clone() };
    let reasons = shape_edges(&[partial], &[(10, 14)], &[], &battle, 10, 14).exits[0].missing.clone().unwrap();
    assert!(reasons.contains("hour") && reasons.contains(MANPOWER_UNREAD), "every reason is kept: {reasons}");
    assert_eq!(
        edges.unrecorded,
        [
            LandEdgeGap { first_hour: 13, last_hour: 14, reason: NOT_RECORDED.into() },
            LandEdgeGap { first_hour: 13, last_hour: 13, reason: UNATTRIBUTED.into() },
            LandEdgeGap { first_hour: 14, last_hour: 14, reason: UNATTRIBUTED_DRAINED.into() },
        ]
    );

    let running = Battle { combat: 9, start: 10, exact: false, ..Battle::default() };
    let before = shape_edges(&rows[2..], &[(11, 14)], &[], &running, 10, 14);
    assert_eq!(before.opening_missing.as_deref(), Some(OPENED_EARLIER), "the first drain after attach is never complete");
    let reused = [rows.to_vec(), vec![edge(5, Edge::OPENED, 13, Some(7), 0, [Some(1), Some(0), None], 0), edge(6, Edge::EXITED, 14, Some(7), 0, [None, Some(0), Some(3)], 0)]].concat();
    let first = shape_edges(&reused, &[(10, 14)], &[], &battle, 10, 14);
    assert_eq!(first.opening.unwrap().members.len(), 2, "a later occurrence of the handle keeps its own opening");
    assert_eq!(first.exits.len(), 2, "and its own exits");
    let replaced = [rows[2..3].to_vec(), vec![edge(5, Edge::OPENED, 13, Some(7), 0, [Some(1), Some(0), None], 0)]].concat();
    let earlier = shape_edges(&replaced, &[(10, 14)], &[], &running, 10, 14);
    assert!(earlier.opening.is_none(), "an opening after the battle's start is the replacement's");
    assert_eq!(earlier.exits.len(), 1, "the running battle keeps its exit before the replacement opened");
    let older = shape_edges(&[], &[], &[], &battle, 10, 14);
    assert_eq!(older.opening_missing.as_deref(), Some(NOT_RECORDED), "a recording without edges says so, never an empty roster");
    assert_eq!(older.unrecorded, [LandEdgeGap { first_hour: 10, last_hour: 14, reason: NOT_RECORDED.into() }]);
}

#[test]
fn the_battle_log_shows_only_observed_losses_and_keeps_final_results_and_gaps() {
    use dashboard_base::{SourceKey, test_fixtures::{CAMPAIGN, records, source}};
    use replay_data::rows::{LandCombatProgressRow, LandCombatResultsRow, LandCombatRow, LandEquipmentLoss, LandCombatSideRow, LandEventCoverageRow, LandResultSide, LandSideLosses, Recorded, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::Arc;
    use tail::{LandBattleStatus, LandBattlesTail};

    let frames = (100..=105).map(|hour| {
        let mut frame = source(1, hour, Some(1));
        let active = (101..104).contains(&hour);
        for (table, rows) in [
            (TableId::LandCombats, TableRows::LandCombats(active.then(|| LandCombatRow {
                id: 19, province: 3, duration: hour - 101, attackers: vec![11, 12], defenders: vec![13], ..Default::default()
            }).into_iter().collect())),
            (TableId::LandCombatSides, TableRows::LandCombatSides(active.then(|| LandCombatSideRow {
                combat: 19, attacker_primary: vec![9001], defender_primary: vec![11], ..Default::default()
            }).into_iter().collect())),
            (TableId::LandCombatProgress, TableRows::LandCombatProgress(active.then(|| LandCombatProgressRow {
                combat: 19, attacker_manpower_loss: Recorded::Value(if hour < 103 { 2 } else { 7 }), defender_manpower_loss: Recorded::Value(0),
                attacker_equipment_losses: vec![LandEquipmentLoss { variant: 20, amount_raw: 100_000 }].into(),
                defender_equipment_losses: vec![].into(),
                ..Default::default()
            }).into_iter().collect())),
            (TableId::LandCombatResults, TableRows::LandCombatResults(if hour == 104 {
                let disputed = [(8, 31, 12, 5, 11), (9, 31, 12, 6, 12), (10, 32, 3, 4, 11), (11, 32, 5, 6, 11)].map(|(id, combat, attacker, lost, defender)| LandCombatResultsRow {
                    id, combat, province: 4, duration: 2, end_hour: 103, winners: 1,
                    sides: [LandResultSide { countries: vec![9001], ..Default::default() }, LandResultSide { countries: vec![defender], ..Default::default() }],
                    losses: [LandSideLosses { manpower: attacker, equipment: vec![] }, LandSideLosses { manpower: lost, equipment: vec![] }], ..Default::default()
                });
                [vec![LandCombatResultsRow {
                    id: 7, combat: 19, province: 3, duration: 3, end_hour: 104, winners: 2,
                    sides: [LandResultSide { countries: vec![9001], ..Default::default() }, LandResultSide::default()],
                    losses: [LandSideLosses { manpower: 10, equipment: vec![] }, LandSideLosses { manpower: 0, equipment: vec![] }], ..Default::default()
                }], disputed.into()].concat()
            } else {
                Vec::new()
            })),
            (TableId::LandEventCoverage, TableRows::LandEventCoverage(vec![LandEventCoverageRow {
                id: hour as u32, epoch: 1, from_hour: hour - 1, to_hour: hour, complete: true, lost_events: 0
            }])),
        ] {
            frame.metrics.table_ns[table.index()] = 1;
            *frame.tables[table.index()] = if hour == 105 && table == TableId::LandCombats { TableView::Unavailable { reason: "census gap".into() } }
                else { TableView::Complete(rows) };
        }
        frame
    }).collect();
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap()
        .into_timelines(&Default::default()).unwrap();
    let cancel = AtomicBool::new(false);
    let source = Source::from_replay(SourceKey { source_id: "battles".into(), revision: "1".into() }, "fixture.battleplan".into(), Arc::new(replay), &cancel).unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let (places, reads) = replay_recording::cells::count_reads(|| tail::battles(&query, &cancel).unwrap());
    let first = places.iter().find(|battle| battle.combat == 19).unwrap();
    assert_eq!((first.start, first.observed, first.province), (101, 101, Some(3)));
    assert!(places.iter().any(|battle| (battle.combat, battle.written, battle.province) == (31, Some(104), Some(4))));
    for table in [TableId::LandCombatProgress, TableId::LandCombatSides, TableId::VariantStats, TableId::Provinces] {
        assert_eq!(reads.get(&table).map_or(0, |read| read.opened), 0, "title opened {table:?}");
    }
    assert_eq!(reads[&TableId::LandCombats].opened, 2);
    let ask = |hour, countries: Option<Vec<u32>>| {
        let params = serde_json::from_value(serde_json::json!({ "branchId": "1", "limit": 64, "hour": hour, "countries": countries })).unwrap();
        LandBattlesTail::load(&source, &params, &LoadContext { content: None }, &cancel).unwrap()
    };
    assert!(ask(100, None).rows.is_empty());
    let first = ask(101, Some(vec![7]));
    assert_eq!(first.rows.len(), 1);
    assert_eq!(first.rows[0].result, LandBattleStatus::Ongoing);
    assert_eq!(first.rows[0].casualties, [Some(2), Some(0)]);
    assert_eq!(first.rows[0].divisions, [Some(2), Some(1)]);
    assert_eq!(first.rows[0].ic_lost, [None, Some(0.0)]);
    assert!(first.rows[0].losses_so_far, "a live battle's losses are the game's tally of divisions that have left it");
    assert_eq!(first.next_change, Some(103));
    assert_eq!(ask(103, None).rows[0].casualties, [Some(7), Some(0)]);
    let ended = ask(104, None);
    let ended = ended.rows.iter().find(|row| row.key == "1:19:101").unwrap();
    assert!(!ended.losses_so_far, "an ended battle's losses are its final ones");
    assert_eq!(ended.result, LandBattleStatus::DefenderWon);
    assert_eq!(ended.countries[1], [7], "a result missing its defending country retains the recorded census participant");
    assert_eq!(ended.casualties, [Some(10), Some(0)]);
    assert!(!ask(105, None).missing.is_empty());
    let disputed = ask(104, Some(vec![7]));
    let alone = disputed.rows.iter().find(|row| row.key == "1:31:103:104").expect("result-only rows that disagree on the result id stay under the attacker's filter");
    assert_eq!((alone.result, alone.casualties), (LandBattleStatus::AttackerWon, [Some(12), None]), "the agreed fields stand and the disputed side is unknown");
    let course = LandBattleCourses::load(&source, &serde_json::from_value(serde_json::json!({ "branchId": "1", "id": "1:31:103:104" })).unwrap(), &LoadContext { content: None }, &cancel).unwrap();
    let result = course.battle.result.expect("the agreed result");
    assert_eq!((result.winner, result.countries_unknown[1]), (BattleWinner::Side { side: 0 }, true));
    let terminal = course.battle.losses.last().expect("the result's losses");
    assert!(terminal.terminal);
    let LandLossSides::Read([attacker, defender]) = &terminal.sides else { panic!("read losses: {:?}", terminal.sides) };
    assert!(matches!(attacker, LandLossSide::Variants { manpower: LandManpower::Known(12), .. }), "{attacker:?}");
    assert!(matches!(defender, LandLossSide::Unknown { manpower: LandManpower::Unknown(_), .. } | LandLossSide::Variants { manpower: LandManpower::Unknown(_), .. }), "{defender:?}");
    let course = LandBattleCourses::load(&source, &serde_json::from_value(serde_json::json!({ "branchId": "1", "id": "1:32:103:104" })).unwrap(), &LoadContext { content: None }, &cancel).unwrap();
    let terminal = course.battle.losses.last().expect("agreed equipment losses are a result's losses though both manpower totals disagree");
    assert!(terminal.terminal && matches!(&terminal.sides, LandLossSides::Read([LandLossSide::Variants { manpower: LandManpower::Unknown(_), .. }, _])), "{terminal:?}");
}

#[test]
fn a_sides_men_are_its_divisions_manpower_and_one_unread_division_leaves_them_unrecorded() {
    let members = vec![(10, Some([Some(vec![1, 2, 1]), Some(vec![3])])), (12, Some([Some(vec![1]), Some(vec![3, 4])])), (14, None), (15, Some([Some(vec![1]), None]))];
    let manpower = HashMap::from([
        (1, vec![(10, Some(9000)), (11, Some(8700))]),
        (2, vec![(10, Some(500))]),
        (3, vec![(10, Some(4000))]),
        (4, vec![(13, Some(100))]),
    ]);
    assert_eq!(
        men_reads(&members, &manpower, 10, 16),
        [
            LandMenRead { hour: 10, sides: [Some(9500), Some(4000)] },
            LandMenRead { hour: 11, sides: [Some(9200), Some(4000)] },
            LandMenRead { hour: 12, sides: [Some(8700), None] },
            LandMenRead { hour: 13, sides: [Some(8700), Some(4100)] },
            LandMenRead { hour: 14, sides: [None, None] },
            LandMenRead { hour: 15, sides: [Some(8700), None] },
        ],
        "an unread side list leaves only that side's men unrecorded"
    );
    assert_eq!(men_reads(&[], &HashMap::new(), 10, 12), [LandMenRead { hour: 10, sides: [None, None] }], "no roster read is no men, never zero");
    assert_eq!(
        men_reads(&[(10, Some([Some(Vec::new()), Some(vec![3])]))], &HashMap::new(), 10, 10),
        [LandMenRead { hour: 10, sides: [Some(0), None] }],
        "a side with no divisions has no men without any manpower read"
    );
}

#[test]
fn ic_lost_follows_a_build_cost_recorded_after_the_loss_without_a_new_loss_read() {
    let step = |hour, price: Option<i64>| lanes::Step { hour, cells: price.map(|price| vec![Cell::Number(price)].into_boxed_slice()) };
    let costs = lanes::History {
        keys: vec![(7, vec![step(10, None), step(14, Some(300_000))])],
        statuses: vec![lanes::StatusRun { first: 10, last: 20, status: replay_columnar::manifest::Status::Complete }],
        ..lanes::History::default()
    };
    let lost = Losses { manpower: [Some(5), Some(0)], equipment: [Some(vec![LandEquipmentLoss { variant: 7, amount_raw: 200_000 }]), Some(Vec::new())] };
    let course = vec![(10, false, Some(lost)), (18, false, None)];
    assert_eq!(
        ic_reads(&course, &costs),
        [
            LandIcRead { hour: 10, sides: [None, Some(0.0)] },
            LandIcRead { hour: 14, sides: [Some(6.0), Some(0.0)] },
            LandIcRead { hour: 18, sides: [None, None] },
        ]
    );
}
