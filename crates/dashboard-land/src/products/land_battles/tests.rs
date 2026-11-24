use dashboard_query::identity::Holders;
use super::*;

fn entry(id: &str, casualties: Option<f64>, length: u64, peak: Option<u64>) -> Ranked {
    Ranked { casualties, length, peak, id: id.into() }
}

#[test]
fn a_ranking_orders_by_its_measure_and_falls_back_to_peak_size_only_where_no_casualties_were_recorded() {
    let mut entries = vec![entry("1:2:10", Some(500.0), 30, Some(4)), entry("1:3:10", None, 90, Some(9)), entry("1:1:10", Some(500.0), 10, None)];
    assert_eq!(rank(&mut entries, "Casualties"), "Casualties");
    assert_eq!(entries.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(), ["1:1:10", "1:2:10", "1:3:10"], "ties break by id; an unknown is last");
    assert_eq!(rank(&mut entries, "Length"), "Length");
    assert_eq!(entries.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(), ["1:3:10", "1:2:10", "1:1:10"]);
    let mut unrecorded = vec![entry("1:2:10", None, 30, Some(4)), entry("1:3:10", None, 90, Some(9))];
    assert_eq!(rank(&mut unrecorded, "Casualties"), "Peak size");
    assert_eq!(unrecorded[0].id, "1:3:10");
}

#[test]
fn a_battle_is_chosen_by_a_country_on_either_side_at_its_read_and_never_left_out_while_a_side_is_unknown() {
    let ger = ["GER".to_owned()];
    let sov = ["SOV".to_owned()];
    let all = BattleCountryFilter::default();
    assert!(all.matches(&Holders::default(), [(&ger, false), (&sov, false)], 10));
    let tags = BattleCountryFilter { countries: Some(vec!["SOV".into()]), ..BattleCountryFilter::default() };
    assert!(tags.matches(&Holders::default(), [(&ger, false), (&sov, false)], 10));
    assert!(!tags.matches(&Holders::default(), [(&ger, false), (&ger, false)], 10));
    assert!(tags.matches(&Holders::default(), [(&ger, false), (&[], true)], 10));
    let runs = BattleCountryFilter {
        membership: Some(vec![dashboard_world::products::groups::MembershipRun { tag: "GER".into(), first_hour: 100, last_hour: 200 }]),
        unobserved: vec![dashboard_base::span::HourSpan { first_hour: 300, last_hour: 310 }],
        ..BattleCountryFilter::default()
    };
    assert!(runs.matches(&Holders::default(), [(&ger, false), (&sov, false)], 150));
    assert!(!runs.matches(&Holders::default(), [(&ger, false), (&sov, false)], 250));
    assert!(runs.matches(&Holders::default(), [(&ger, false), (&sov, false)], 305), "membership not observed at the hour is not known to leave a battle out");
}

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
fn a_window_cuts_runs_to_itself_and_carries_the_loss_read_standing_at_its_first_hour() {
    let run = |first: i32, last: i32, duration: i32| LandBattleRun {
        first_hour: first,
        last_hour: last,
        counts: [2, 1],
        roster_unknown: false,
        countries: [vec!["GER".into()], vec!["POL".into()]],
        countries_unknown: [false, false],
        duration,
    };
    let spans = |runs: Vec<LandBattleRun>| runs.iter().map(|run| (run.first_hour, run.last_hour, run.duration)).collect::<Vec<_>>();
    let runs = [run(90, 99, 5), run(100, 130, 20)];
    assert_eq!(spans(window_runs(&runs, 100, 120, false)), [(100, 120, 20)]);
    assert_eq!(spans(window_runs(&runs, 95, 120, false)), [(95, 99, 10), (100, 120, 20)]);
    assert!(window_runs(&runs, 131, 140, false).is_empty());
    assert_eq!(spans(window_runs(&runs, 131, 140, true)), [(130, 130, 50)], "the end in the window shows the last run at its last hour");
    let hours = [60, 95, 100, 125, 131];
    assert_eq!(carried(&hours, 100, 120), 2..3, "a read at the first hour is the one standing");
    assert_eq!(carried(&hours, 99, 120), 1..3, "the read before the first hour, at its own hour, then those after");
    assert_eq!(carried(&hours, 130, 131), 3..5);
    assert_eq!(carried(&hours, 10, 50), 0..0);
    assert_eq!(carried(&[], 10, 50), 0..0);
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
fn a_result_without_runs_is_listed_from_the_hour_it_was_written() {
    let history = |runs: Vec<LandBattleRun>, settled: Option<i32>| LandBattleHistory {
        id: "1:7:10:11".into(),
        province_id: Some(3),
        start: BattleMoment { hour: 10, bound: BattleBound::Exact },
        end: None,
        end_unknown_hour: None,
        counter_base: 0,
        prior_peak: None,
        prior_side_peaks: [None, None],
        runs,
        losses: Vec::new(),
        result: Some(LandBattleResultRead { hour: Some(10), duration: 2, winner: BattleWinner::Side { side: 0 }, countries: [Vec::new(), Vec::new()], countries_unknown: [true, true] }),
        settled_hour: settled,
    };
    let shown = |history: Option<LandBattleHistory>| history.map(|history| history.runs.iter().map(|run| (run.first_hour, run.last_hour, run.roster_unknown, run.duration)).collect::<Vec<_>>());
    assert_eq!(shown(listed(history(Vec::new(), Some(11)), 5)), Some(vec![(11, 11, true, 0)]), "a window holding both hours lists it at the hour written");
    assert_eq!(shown(listed(history(Vec::new(), Some(11)), 11)), Some(vec![(11, 11, true, 0)]), "the window beginning at the hour written lists it");
    assert_eq!(shown(listed(history(Vec::new(), Some(11)), 12)), None, "a later window does not");
    assert_eq!(shown(listed(history(Vec::new(), None), 5)), None, "nothing settled is nothing listed");
    let run = LandBattleRun {
        first_hour: 8,
        last_hour: 9,
        counts: [1, 1],
        roster_unknown: false,
        countries: [Vec::new(), Vec::new()],
        countries_unknown: [true, true],
        duration: 3,
    };
    assert_eq!(shown(listed(history(vec![run], Some(11)), 5)), Some(vec![(8, 9, false, 3)]), "a battle with runs keeps them");
}

#[test]
fn unread_end_kind_hour_and_side_lists_stay_unknown_and_survive_country_filters() {
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
    let data = Combats { from: 10, census, sides: HashMap::new(), losses: HashMap::new(), since: HashMap::new() };
    let names = Names::new(&Holders::default());
    let links = dashboard_base::queries::country_ids::RunLinks { slots: HashMap::new() };

    let unread_kind = &decoded[&(8, 10, None)];
    assert_eq!(unread_kind.end.as_ref().unwrap().kind, None, "production decoder keeps unread end kind as None");
    let shaped = shape(unread_kind, &data, &names, &links, 1, 10, 30, 30).history;
    assert_eq!(shaped.end, Some(BattleEnd::Unknown { unknown: "Ending kind not recorded".into() }));
    assert_eq!(shaped.end_unknown_hour, Some(25));
    assert_eq!(shaped.settled_hour, Some(25));
    let result = shaped.result.as_ref().unwrap();
    assert_eq!(result.countries_unknown, [true, true], "unread side lists keep both sides unknown even when the bitmask is zero");

    let mut unread_hour = decoded[&(9, 10, None)].clone();
    assert_eq!(unread_hour.end.as_ref().unwrap().end_hour, None, "production decoder keeps unread end hour as None");
    unread_hour.end.as_mut().unwrap().result_sides = [Some(vec![1]), None];
    let shaped = shape(&unread_hour, &data, &names, &links, 1, 10, 30, 30).history;
    assert_eq!(shaped.end, Some(BattleEnd::Unknown { unknown: "Ending hour not recorded".into() }));
    assert_eq!(shaped.end_unknown_hour, Some(25));
    assert_eq!(shaped.settled_hour, Some(25));
    let result = shaped.result.as_ref().unwrap();
    assert_eq!(result.hour, None, "an unread result hour never copies the settlement hour");
    assert_eq!(result.winner, BattleWinner::Side { side: 1 });
    assert_eq!(ranked_hour(&shaped, 12), 25, "ranking length and country lookup keep main's settlement-hour fallback when result hour is unread");
    assert_eq!(result.countries_unknown[1], true, "an unread defender side list stays unknown");
    let filter = BattleCountryFilter { countries: Some(vec!["GER".into()]), ..BattleCountryFilter::default() };
    assert!(filter.matches(&Holders::default(), [(&result.countries[0], result.countries_unknown[0]), (&result.countries[1], result.countries_unknown[1])], 25));

    let unknown_unread_hour = shape(&decoded[&(10, 10, None)], &data, &names, &links, 1, 10, 30, 30).history;
    assert_eq!(unknown_unread_hour.end, Some(BattleEnd::Unknown { unknown: "Ending hour not recorded".into() }));
    assert_eq!(unknown_unread_hour.end_unknown_hour, Some(25));

    let unknown_dated = shape(&decoded[&(11, 10, None)], &data, &names, &links, 1, 10, 30, 30).history;
    assert_eq!(unknown_dated.end, Some(BattleEnd::Unknown { unknown: "Ending not recorded".into() }));
    assert_eq!(unknown_dated.end_unknown_hour, Some(22));

    let result_only = shape(&decoded[&(7, 10, Some(25))], &data, &names, &links, 1, 10, 30, 30).history;
    assert_eq!(result_only.province_id, None, "a result-only key never inherits a live battle's census province");
    assert_eq!(result_only.end, None, "a result-only key keeps no battle end");
    assert_eq!(result_only.settled_hour, Some(25));

    let mut known_run_unknown_result = shaped;
    known_run_unknown_result.runs = vec![LandBattleRun {
        first_hour: 10,
        last_hour: 15,
        counts: [1, 1],
        roster_unknown: false,
        countries: [vec!["GER".into()], vec!["POL".into()]],
        countries_unknown: [false, false],
        duration: 5,
    }];
    known_run_unknown_result.result = Some(LandBattleResultRead {
        hour: Some(16),
        duration: 6,
        winner: BattleWinner::Side { side: 0 },
        countries: [vec!["GER".into()], Vec::new()],
        countries_unknown: [false, true],
    });
    assert_eq!(battle_side(&known_run_unknown_result, 1), (&[][..], true), "unread result side stays unknown and never inherits earlier run certainty");
    let fra_filter = BattleCountryFilter { countries: Some(vec!["FRA".into()]), ..BattleCountryFilter::default() };
    assert!(fra_filter.matches(&Holders::default(), [battle_side(&known_run_unknown_result, 0), battle_side(&known_run_unknown_result, 1)], 16));
}

#[test]
fn a_battle_course_carries_its_opening_roster_and_each_exit_and_names_what_was_not_recorded() {
    use replay_data::rows::LandCombatEdgesRow as Edge;
    let edge = |id, kind, hour, division: Option<u64>, side, values: [Option<i32>; 3], withheld| Edge {
        id, kind, hour, combat: 9, division, side, manpower: values[0], casualties_before: values[1], casualties_after: values[2], withheld,
    };
    let rows = [
        edge(1, Edge::OPENED, 10, Some(5), 0, [Some(9000), Some(300), None], 0),
        edge(2, Edge::OPENED, 10, None, 1, [None, None, None], Edge::DIVISION_UNREADABLE),
        edge(3, Edge::EXITED, 12, Some(5), 0, [Some(8500), Some(300), Some(812)], 0),
        edge(4, Edge::EXITED, 13, Some(6), 1, [Some(100), Some(40), None], 0),
    ];
    let battle = Battle { combat: 9, start: 11, exact: true, ..Battle::default() };
    let edges = shape_edges(&rows, &[(10, 12)], &battle, 10, 14);
    let opening = edges.opening.expect("an opening");
    assert_eq!(opening.hour, 10);
    assert_eq!(opening.members[0], LandEdgeMember { division: Some("5".into()), side: 0, manpower: Some(9000), casualties: Some(300), missing: None });
    assert_eq!(opening.members[1].missing.as_deref(), Some("The division could not be read"));
    assert_eq!(edges.opening_missing, None);
    assert_eq!(edges.exits[0].losses, Some(512), "losses are the casualties the exit realised");
    assert_eq!(edges.exits[1].losses, None);
    assert_eq!(edges.exits[1].missing.as_deref(), Some(CASUALTIES_UNREAD));
    assert_eq!(edges.unrecorded, [LandEdgeGap { first_hour: 13, last_hour: 14, reason: NOT_RECORDED.into() }]);

    let running = Battle { combat: 9, start: 10, exact: false, ..Battle::default() };
    let before = shape_edges(&rows[2..], &[(10, 14)], &running, 10, 14);
    assert_eq!(before.opening_missing.as_deref(), Some(OPENED_EARLIER));
    let older = shape_edges(&[], &[], &battle, 10, 14);
    assert_eq!(older.opening_missing.as_deref(), Some(NOT_RECORDED), "a recording without edges says so, never an empty roster");
    assert_eq!(older.unrecorded, [LandEdgeGap { first_hour: 10, last_hour: 14, reason: NOT_RECORDED.into() }]);
}
