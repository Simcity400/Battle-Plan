use dashboard_base::products::Country;
use dashboard_query::identity::Holders;
use super::{
    battles::{BattleCells, NavalBattleCountryFilter, battle_id, live_run, parse_id, result_history, sides_at},
    losses::{Counts, NavalPeriod, month_bases, month_start, row_key, run_key, score},
    sunk::{ShipSinkingCause, cause, known_at, pruned},
};
use dashboard_world::products::groups::MembershipRun;
use game_model::calendar::HOUR_EPOCH;
use replay_lanes::lanes::Cell;
use replay_data::rows::{NavalLossTotalRow as Row, NavalResultObservationRow as Observation, NavalResultRow, Recorded, SunkShipEntry};
use std::collections::HashMap;

#[test]
fn a_month_starts_at_its_first_day_whatever_the_year() {
    let january = HOUR_EPOCH + 3 * 365 * 24;
    assert_eq!(month_start(january), january);
    assert_eq!(month_start(january + 30 * 24 + 23), january);
    let february = january + 31 * 24;
    assert_eq!(month_start(february), february);
    assert_eq!(month_start(february + 27 * 24 + 5), february);
    let december = january + (365 - 31) * 24;
    assert_eq!(month_start(january + 364 * 24), december);
    assert_eq!(month_start(january + 365 * 24), january + 365 * 24);
}

#[test]
fn counts_are_known_only_after_the_baseline_while_read_whole_and_a_missing_row_counted_none() {
    let owner = 7u128 << 32 | 9;
    let ships = row_key(owner, Row::LOST, Row::SHIPS);
    let run = run_key(Row::SHIPS);
    let start = month_start(HOUR_EPOCH + 4 * 365 * 24 + 40 * 24);
    let known = HashMap::from([(run, vec![(start, start + 99, Some(1)), (start + 100, start + 110, Some(0)), (start + 111, start + 2000, Some(1))])]);
    let counted = HashMap::from([(run, vec![(start, start + 2000, Some(i64::from(start + 10)))])]);
    let totals = HashMap::from([(ships, vec![(start + 50, start + 899, Some(3)), (start + 900, start + 2000, Some(5))])]);
    let counts = Counts::of(known, counted, totals);
    assert_eq!(counts.count(Row::SHIPS, ships, start + 5, NavalPeriod::Running), None, "before the baseline nothing is counted");
    assert_eq!(counts.count(Row::SHIPS, ships, start + 20, NavalPeriod::Running), Some(0), "after the baseline a missing row counted none");
    assert_eq!(counts.count(Row::SHIPS, ships, start + 60, NavalPeriod::Running), Some(3));
    assert_eq!(counts.count(Row::SHIPS, ships, start + 105, NavalPeriod::Running), None, "an hour not read whole is not known");
    assert_eq!(counts.count(Row::SHIPS, ships, start + 60, NavalPeriod::Month), Some(3), "a month begun before the baseline counts from it");
    let next = month_start(start + 900);
    assert!(next > start + 10 && next <= start + 900);
    assert_eq!(counts.count(Row::SHIPS, ships, start + 950, NavalPeriod::Month), Some(5 - counts.running(Row::SHIPS, ships, next - 1).unwrap()));
    assert_eq!(counts.count(Row::CONVOYS, row_key(owner, Row::LOST, Row::CONVOYS), start + 60, NavalPeriod::Running), None, "a kind without a run row is not known");
}

#[test]
fn a_months_ranking_subtracts_each_kind_at_its_own_baseline_as_its_counts_do() {
    let h = month_start(HOUR_EPOCH + 4 * 365 * 24 + 40 * 24) + 48;
    let (a, b) = (7u128 << 32 | 1, 8u128 << 32 | 2);
    let kinds = [Row::SHIPS, Row::CONVOYS];
    let known = HashMap::from([(run_key(Row::SHIPS), vec![(h, h + 20, Some(1))]), (run_key(Row::CONVOYS), vec![(h, h + 20, Some(1))])]);
    let counted = HashMap::from([(run_key(Row::SHIPS), vec![(h, h + 20, Some(i64::from(h)))]), (run_key(Row::CONVOYS), vec![(h, h + 20, Some(i64::from(h + 10)))])]);
    let (a_ships, b_ships) = (row_key(a, Row::LOST, Row::SHIPS), row_key(b, Row::LOST, Row::SHIPS));
    let totals = HashMap::from([(a_ships, vec![(h + 5, h + 20, Some(5))]), (b_ships, vec![(h + 15, h + 20, Some(1))])]);
    let counts = Counts::of(known, counted, totals);
    let at = h + 20;
    let bases = month_bases(&counts, &kinds, at, NavalPeriod::Month);
    assert_eq!(bases, [Some(h), Some(h + 10)]);
    let total = |key: u128, hour: i32| counts.running(Row::SHIPS, key, hour).unwrap();
    let snapshot = |hour: i32| HashMap::from([(a_ships, total(a_ships, hour)), (b_ships, total(b_ships, hour))]);
    let now = snapshot(at);
    let before: HashMap<i32, HashMap<u128, i64>> = bases.iter().flatten().map(|&base| (base, snapshot(base))).collect();
    for (owner, key, lost) in [(a, a_ships, 5), (b, b_ships, 1)] {
        assert_eq!(counts.count(Row::SHIPS, key, at, NavalPeriod::Month), Some(lost));
        assert_eq!(score(owner, Row::LOST, &kinds, &bases, at, &now, &before), lost, "the ranked count is the count shown");
    }
}

#[test]
fn an_entry_is_known_at_the_first_read_at_or_after_its_sinking() {
    let reads = [(100, 100), (124, 124), (148, 200)];
    assert_eq!(known_at(&reads, 90), Some(100));
    assert_eq!(known_at(&reads, 100), Some(100));
    assert_eq!(known_at(&reads, 101), Some(124));
    assert_eq!(known_at(&reads, 160), Some(160));
    assert_eq!(known_at(&reads, 201), None);
}

#[test]
fn a_window_keeps_the_sinkings_its_first_read_listed_that_the_game_pruned_before_its_end() {
    let entry = |sink_hour, ship: &[u8]| SunkShipEntry { sink_hour, ship_name: ship.to_vec(), ..Default::default() };
    let reads = [(5, 5), (100, 100), (124, 124), (148, 148)];
    let earlier = [entry(3, b"Z"), entry(10, b"A"), entry(10, b"A"), entry(20, b"B"), entry(30, b"C"), entry(99, b"D")];
    let later = [entry(10, b"A"), entry(30, b"C"), entry(99, b"D"), entry(130, b"E")];
    let gone: Vec<&SunkShipEntry> = pruned(&earlier, &later, 100, &reads);
    assert_eq!(gone, [&earlier[2], &earlier[3]], "each pruned entry once per copy missing; none still listed, none an earlier window's");
}

#[test]
fn a_cause_record_comes_before_an_assist() {
    let entry = |cause_record, assisted_kill| SunkShipEntry { cause_record, assisted_kill, ..Default::default() };
    assert_eq!(cause(&entry(0, 0)), ShipSinkingCause::Hostile);
    assert_eq!(cause(&entry(0, 1)), ShipSinkingCause::Assisted);
    assert_eq!(cause(&entry(1, 1)), ShipSinkingCause::Cause);
}

#[test]
fn sides_count_ships_and_convoys_and_name_their_owners() {
    let combat = 5u128 << 33;
    let rows = [
        (combat, Some(vec![Cell::Number(11), Cell::Number(1), Cell::Number(0)])),
        (combat | 1, Some(vec![Cell::Number(0), Cell::Number(1), Cell::Number(0)])),
        (combat | 1 << 32, Some(vec![Cell::Number(12), Cell::Number(2), Cell::Number(0x20)])),
        (combat | 1 << 32 | 1, Some(vec![Cell::Number(13), Cell::Number(3), Cell::Number(0)])),
        (combat | 1 << 32 | 2, None),
    ];
    let tag = |raw: u32| match raw {
        1 => Some("ENG".to_owned()),
        2 => Some("GER".to_owned()),
        _ => None,
    };
    let sides = sides_at(rows.iter().map(|(key, cells)| (*key, cells.as_deref())), &tag);
    assert_eq!((sides[0].ships, sides[0].convoys, sides[0].unknown), (1, 1, false));
    assert_eq!(sides[0].countries.iter().collect::<Vec<_>>(), ["ENG"]);
    assert_eq!((sides[1].ships, sides[1].convoys, sides[1].unknown), (2, 0, true), "an owner no tag names leaves the side unknown");
}

#[test]
fn a_battle_is_named_by_its_combat_and_first_hour() {
    assert_eq!(parse_id(&battle_id(25447681233513, 60768781)), Some((25447681233513, 60768781)));
    assert_eq!(parse_id("result:4"), None);
    assert_eq!(parse_id("combat:1:2:3"), None);
}

#[test]
fn a_live_run_measures_floors_and_length_and_leaves_unread_sides_unknown() {
    let cells = BattleCells::of(&[Cell::Number(100), Cell::Number(9), Cell::Number(103), Cell::Number(2), Cell::Number(1), Cell::Unrecorded, Cell::Number(0)]).unwrap();
    assert_eq!(cells.ships_sunk(), Some(3));
    let run = live_run("combat:1:100", &cells, None, 120, 130);
    let measures = serde_json::to_value(&run.measures).unwrap();
    assert_eq!(measures[0]["value"], 3.0);
    assert_eq!(measures[2]["value"], 30.0);
    let side = serde_json::to_value(&run.row.sides[0]).unwrap();
    assert_eq!(side["counts"][0]["unknown"], "Participants not recorded");
    assert_eq!(side["losses"][1]["unknown"], "Participants not recorded", "a floor not recorded is not zero");
    assert_eq!(side["countriesUnknown"], "Countries not recorded");
}

#[test]
fn the_country_filter_keeps_sides_not_known_whole() {
    let cells = BattleCells::of(&[Cell::Number(1), Cell::Number(2), Cell::Number(1), Cell::Number(0), Cell::Number(0), Cell::Number(0), Cell::Number(0)]).unwrap();
    let mut sides: [super::battles::SideAt; 2] = Default::default();
    sides[0].countries.insert("ENG".into());
    sides[1].countries.insert("GER".into());
    let run = live_run("combat:1:1", &cells, Some(&sides), 5, 5);
    let tags = |tags: &[&str]| NavalBattleCountryFilter { countries: Some(tags.iter().map(|tag| Country::from(*tag)).collect()), membership: None, unobserved: Vec::new() };
    assert!(tags(&["GER"]).matches(&Holders::default(), &run.row.sides, 5));
    assert!(!tags(&["ITA"]).matches(&Holders::default(), &run.row.sides, 5));
    let member = NavalBattleCountryFilter { countries: None, membership: Some(vec![MembershipRun { tag: "ENG".into(), first_hour: 6, last_hour: 9 }]), unobserved: Vec::new() };
    assert!(!member.matches(&Holders::default(), &run.row.sides, 5));
    assert!(member.matches(&Holders::default(), &run.row.sides, 7));
    sides[1].unknown = true;
    let unknown = live_run("combat:1:1", &cells, Some(&sides), 5, 5);
    assert!(tags(&["ITA"]).matches(&Holders::default(), &unknown.row.sides, 5));
}

#[test]
fn a_result_dates_its_countries_at_its_runs_last_hour_as_every_run_does() {
    let row = NavalResultRow { id: 7, province: 1200, result_hour: 100, winning_side: 2, ..Default::default() };
    let observed = Observation {
        result: 7,
        refusal: Observation::OBSERVED,
        first_hour: Recorded::Value(90),
        last_hour: Recorded::Value(99),
        peak_ships: Recorded::Value(12),
        peak_hour: Recorded::Value(95),
        ..Default::default()
    };
    let history = result_history(&row, Some(&observed), [Some(vec![Some("ENG".into())]), Some(vec![Some("GER".into()), None])], 130);
    assert_eq!(history.id, "result:7");
    let run = &history.runs[0];
    assert_eq!((run.first_hour, run.last_hour), (100, 130));
    assert!(run.row.sides.iter().flat_map(|side| &side.countries).all(|country| country.hour == run.last_hour), "a country is dated at the run's last hour, which the list moves to its playhead");
    assert_eq!(run.row.sides.iter().map(|side| side.countries.len()).collect::<Vec<_>>(), [1, 1]);
    assert_eq!(run.row.sides[0].countries_unknown, None);
    assert_eq!(run.row.sides[1].countries_unknown.as_deref(), Some("Countries not recorded"), "an id no country held");
    assert_eq!(run.peak_hour, Some(95));
    let measures = serde_json::to_value(&run.measures).unwrap();
    assert_eq!(measures[1]["value"], 12.0);
    assert_eq!(measures[2]["value"], 9.0);
    let unread = result_history(&row, None, [None, None], 130);
    assert!(unread.runs[0].row.sides.iter().all(|side| side.countries.is_empty() && side.countries_unknown.is_some()), "lists not read are not known empty");
    assert_eq!(unread.runs[0].peak_hour, None);
}

mod details {
    use super::super::battles::{NavalBattleDetails, NavalBattleDetailsParams, NavalBattleDetailRequest};
    use dashboard_base::{Source, products::{LoadContext, Product}, test_fixtures::{CAMPAIGN, records, source}};
    use replay::campaign::Replay;
    use replay_data::rows::{NavalCombatDetailsRow, SunkShipEntry, SunkShipsRow, TableSchema, tables};
    use replay_frame::frame::{DecodedFrame, TableView};
    use std::sync::{Arc, atomic::AtomicBool};

    fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
        *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
        frame.metrics.table_ns[T::ID.index()] = 1;
    }

    fn recording() -> Source {
        let frames = (100..=110)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let entries = [(7, 101), (8, 103), (9, 106)]
                    .into_iter()
                    .filter(|&(_, sunk)| sunk <= hour)
                    .map(|(battle, sink_hour)| SunkShipEntry { battle, sink_hour, ship_name: format!("ship {battle}").into_bytes(), ..Default::default() })
                    .collect();
                set::<tables::SunkShips>(&mut frame, vec![SunkShipsRow { entries }]);
                set::<tables::NavalCombatDetails>(&mut frame, [7, 8].into_iter().map(|combat| NavalCombatDetailsRow { combat, custom_sea: Some(Vec::new()), ..Default::default() }).collect());
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(dashboard_base::SourceKey { source_id: "naval".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }

    fn details(source: &Source, battles: &[(u64, i32)]) -> Vec<serde_json::Value> {
        let params = NavalBattleDetailsParams {
            branch_id: "1".into(),
            battles: battles.iter().map(|&(combat, hour)| NavalBattleDetailRequest { id: super::battle_id(combat, 100), hour }).collect(),
        };
        let reply = NavalBattleDetails::load(source, &params, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();
        reply.rows.iter().map(|row| serde_json::to_value(row).unwrap()).collect()
    }

    #[test]
    fn battles_asked_together_read_as_each_asked_alone_however_many_share_an_hour() {
        let source = recording();
        let asked = [(7, 104), (8, 104), (9, 108), (7, 102), (8, 104), (5, 400)];
        let together = details(&source, &asked);
        let alone: Vec<_> = asked.iter().flat_map(|&battle| details(&source, &[battle])).collect();
        assert_eq!(together, alone);
        let ships = |row: &serde_json::Value| row["namedShips"].as_array().unwrap().iter().map(|ship| ship["sinkHour"].as_i64().unwrap()).collect::<Vec<_>>();
        assert_eq!(together.iter().take(5).map(ships).collect::<Vec<_>>(), [vec![101], vec![103], vec![106], vec![101], vec![103]], "each battle its own sinkings, as listed by its hour");
        assert_eq!(together[0]["shipRead"], 104, "the list as read at the hour asked");
        assert_eq!(together[5]["unavailable"], "Details are kept for live battles at a recorded hour", "an hour outside the recording");
    }
}
