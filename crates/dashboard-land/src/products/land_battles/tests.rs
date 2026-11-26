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
fn unread_end_kind_hour_and_side_lists_stay_unknown_in_the_course() {
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
    census.insert(7, vec![(10, Some(CensusRead { province: Some(7), duration: Some(1), counts: Some([2, 2]) }))]);
    let data = Combats { from: 10, census, sides: HashMap::new(), losses: HashMap::new() };
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
            (TableId::LandCombatResults, TableRows::LandCombatResults((hour == 104).then(|| LandCombatResultsRow {
                id: 7, combat: 19, province: 3, duration: 3, end_hour: 104, winners: 2,
                sides: [LandResultSide { countries: vec![9001], ..Default::default() }, LandResultSide::default()],
                losses: [LandSideLosses { manpower: 10, equipment: vec![] }, LandSideLosses { manpower: 0, equipment: vec![] }], ..Default::default()
            }).into_iter().collect())),
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
    assert!(!ended.rows[0].losses_so_far, "an ended battle's losses are its final ones");
    assert_eq!(ended.rows[0].result, LandBattleStatus::DefenderWon);
    assert_eq!(ended.rows[0].countries[1], [7], "a result missing its defending country retains the recorded census participant");
    assert_eq!(ended.rows[0].casualties, [Some(10), Some(0)]);
    assert!(!ask(105, None).missing.is_empty());
}
