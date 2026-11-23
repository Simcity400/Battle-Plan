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
    assert_eq!(entries[0].id, "1:3:10");
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
    let run = |first: i32, last: i32, duration: i32| LandBattleRun { first_hour: first, last_hour: last, counts: [2, 1], roster_unknown: false, countries: [vec!["GER".into()], vec!["POL".into()]], countries_unknown: [false, false], duration };
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
    };
    let found: Vec<(Key, i32, Option<i64>)> = written_ends(&ended).map(|(key, _, hour, cells)| (key, hour, match cells[2] { Cell::Number(id) => Some(id), _ => None })).collect();
    assert_eq!(found, [((7, 10, None), 12, Some(59)), ((7, 10, Some(10)), 10, Some(60)), ((7, 10, Some(11)), 11, Some(61))]);
}

#[test]
fn a_result_without_runs_is_listed_from_the_hour_it_was_written() {
    let history = |runs: Vec<LandBattleRun>, settled: Option<i32>| LandBattleHistory {
        id: "1:7:10:11".into(),
        province_id: 3,
        start: BattleMoment { hour: 10, bound: BattleBound::Exact },
        end: None,
        end_unknown_hour: None,
        counter_base: 0,
        prior_peak: None,
        prior_side_peaks: [None, None],
        runs,
        losses: Vec::new(),
        result: Some(LandBattleResultRead { hour: 10, duration: 2, winner: BattleWinner::Side { side: 0 }, countries: [Vec::new(), Vec::new()], countries_unknown: [true, true] }),
        settled_hour: settled,
    };
    let shown = |history: Option<LandBattleHistory>| history.map(|history| history.runs.iter().map(|run| (run.first_hour, run.last_hour, run.roster_unknown)).collect::<Vec<_>>());
    assert_eq!(shown(listed(history(Vec::new(), Some(11)), 5)), Some(vec![(11, 11, true)]), "a window holding both hours lists it at the hour written");
    assert_eq!(shown(listed(history(Vec::new(), Some(11)), 11)), Some(vec![(11, 11, true)]), "the window beginning at the hour written lists it");
    assert_eq!(shown(listed(history(Vec::new(), Some(11)), 12)), None, "a later window does not");
    assert_eq!(shown(listed(history(Vec::new(), None), 5)), None, "nothing settled is nothing listed");
    let run = LandBattleRun { first_hour: 8, last_hour: 9, counts: [1, 1], roster_unknown: false, countries: [Vec::new(), Vec::new()], countries_unknown: [true, true], duration: 3 };
    assert_eq!(shown(listed(history(vec![run], Some(11)), 5)), Some(vec![(8, 9, false)]), "a battle with runs keeps them");
}
