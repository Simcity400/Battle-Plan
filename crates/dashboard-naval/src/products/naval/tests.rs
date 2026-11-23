use dashboard_base::products::Country;
use dashboard_query::identity::Holders;
use super::{
    battles::{BattleCells, NavalBattleCountryFilter, battle_id, live_run, parse_id, result_history, sides_at, sides_at_truncated},
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
    assert_eq!((sides[0].ships, sides[0].convoys, sides[0].unknown, sides[0].counts_unknown), (1, 1, false, false));
    assert_eq!(sides[0].countries.iter().collect::<Vec<_>>(), ["ENG"]);
    assert_eq!((sides[1].ships, sides[1].convoys, sides[1].unknown, sides[1].counts_unknown), (2, 0, true, false), "an owner no tag names leaves the side unknown");

    let partial = [
        (combat, Some(vec![Cell::Number(11), Cell::Unrecorded, Cell::Number(0)])),
        (combat | 1, Some(vec![Cell::Number(0), Cell::Unrecorded, Cell::Number(0)])),
        (combat | 1 << 32, Some(vec![Cell::Unrecorded, Cell::Number(2), Cell::Number(0)])),
    ];
    let mixed = sides_at(partial.iter().map(|(key, cells)| (*key, cells.as_deref())), &tag);
    assert_eq!((mixed[0].ships, mixed[0].convoys, mixed[0].unknown, mixed[0].counts_unknown), (1, 1, true, false), "an unread owner still counts known ships and convoys");
    assert_eq!((mixed[1].ships, mixed[1].convoys, mixed[1].unknown, mixed[1].counts_unknown), (0, 0, false, true), "an unread ship type marks counts unknown while keeping the owner");
    assert_eq!(mixed[1].countries.iter().collect::<Vec<_>>(), ["GER"]);

    let truncated = sides_at_truncated(partial.iter().map(|(key, cells)| (*key, cells.as_deref())), &tag);
    assert_eq!((truncated[0].ships, truncated[0].convoys, truncated[0].unknown, truncated[0].counts_unknown), (0, 0, true, false), "truncated snapshot keeps main's skip when owner is unread");
    assert_eq!((truncated[1].ships, truncated[1].convoys, truncated[1].unknown, truncated[1].counts_unknown), (0, 0, true, false), "truncated snapshot keeps main's skip when ship is unread");
    assert!(truncated[1].countries.is_empty());
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
    let run = live_run("combat:1:100", Some(500), Some(100), &cells, None, 120, 130);
    assert_eq!(run.province_id, Some(500));
    assert_eq!(run.row.place.hour, 100);
    assert_eq!(run.row.place.named_at, None);
    let moved = live_run("combat:1:100", Some(600), Some(120), &cells, None, 120, 130);
    assert_eq!(moved.row.place.hour, 100);
    assert_eq!(moved.row.place.named_at, Some(120));
    let measures = serde_json::to_value(&run.measures).unwrap();
    assert_eq!(measures[0]["value"], 3.0);
    assert_eq!(measures[2]["value"], 30.0);
    let side = serde_json::to_value(&run.row.sides[0]).unwrap();
    assert_eq!(side["counts"][0]["unknown"], "Participants not recorded");
    assert_eq!(side["losses"][1]["unknown"], "Participants not recorded", "a floor not recorded is not zero");
    assert_eq!(side["countriesUnknown"], "Countries not recorded");

    let mut partial_sides: [super::battles::SideAt; 2] = Default::default();
    partial_sides[0].ships = 2;
    partial_sides[0].counts_unknown = true;
    partial_sides[0].countries.insert("ENG".into());
    let partial_run = live_run("combat:1:100", None, Some(120), &cells, Some(&partial_sides), 120, 130);
    assert_eq!(partial_run.province_id, None);
    assert_eq!(partial_run.row.place.named_at, None);
    let partial_side = serde_json::to_value(&partial_run.row.sides[0]).unwrap();
    assert_eq!(partial_side["counts"][0]["value"], 2.0, "confirmed ships report as a lower bound when another participant has an unread ship type");
    assert_eq!(partial_side["counts"][0]["atLeast"], true);
    assert_eq!(partial_side["counts"][1]["unknown"], "Participants not recorded", "a zero count with an unread ship type has no positive lower bound");
    assert_eq!(partial_side["countriesUnknown"], serde_json::Value::Null, "owner remains known when only ship type is unread");
}

#[test]
fn the_country_filter_keeps_sides_not_known_whole() {
    let cells = BattleCells::of(&[Cell::Number(1), Cell::Number(2), Cell::Number(1), Cell::Number(0), Cell::Number(0), Cell::Number(0), Cell::Number(0)]).unwrap();
    let mut sides: [super::battles::SideAt; 2] = Default::default();
    sides[0].countries.insert("ENG".into());
    sides[1].countries.insert("GER".into());
    let run = live_run("combat:1:1", Some(500), None, &cells, Some(&sides), 5, 5);
    let tags = |tags: &[&str]| NavalBattleCountryFilter { countries: Some(tags.iter().map(|tag| Country::from(*tag)).collect()), membership: None, unobserved: Vec::new() };
    assert!(tags(&["GER"]).matches(&Holders::default(), &run.row.sides, 5));
    assert!(!tags(&["ITA"]).matches(&Holders::default(), &run.row.sides, 5));
    let member = NavalBattleCountryFilter { countries: None, membership: Some(vec![MembershipRun { tag: "ENG".into(), first_hour: 6, last_hour: 9 }]), unobserved: Vec::new() };
    assert!(!member.matches(&Holders::default(), &run.row.sides, 5));
    assert!(member.matches(&Holders::default(), &run.row.sides, 7));
    sides[1].unknown = true;
    let unknown = live_run("combat:1:1", Some(500), None, &cells, Some(&sides), 5, 5);
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

    #[test]
    fn unread_participants_with_historical_peak_stay_unknown_while_truncated_keeps_main() {
        use super::super::battles::{NavalBattleCountryFilter, NavalBattles, NavalBattlesParams, NavalBattlesRanking, NavalBattlesRankingParams};
        use replay_data::rows::{CountryLinkRow, CountryRow, NavalCombatParticipantRow, NavalCombatRow, TableId, TableRows};

        let frames = (100..=105)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
                countries.push(CountryRow { id: 44, canonical_slot: 2, tag: b"ENG".to_vec(), ..CountryRow::default() });
                countries.push(CountryRow { id: 45, canonical_slot: 3, tag: b"GER".to_vec(), ..CountryRow::default() });
                countries.sort_by_key(|row| row.id);
                set::<tables::Countries>(&mut frame, countries);
                let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
                links.push(CountryLinkRow { id: 44, canonical_slot: 2 });
                links.push(CountryLinkRow { id: 45, canonical_slot: 3 });
                links.sort_by_key(|row| row.id);
                set::<tables::CountryLinks>(&mut frame, links);
                let moved_province = if hour < 105 { 500 } else { 600 };
                set::<tables::NavalCombats>(&mut frame, vec![
                    NavalCombatRow { id: 7, province: 500, ..NavalCombatRow::default() },
                    NavalCombatRow { id: 8, province: moved_province, ..NavalCombatRow::default() },
                ]);
                if hour == 104 {
                    *frame.tables[TableId::NavalCombatParticipants.index()] = TableView::Truncated {
                        rows: TableRows::NavalCombatParticipants(vec![
                            NavalCombatParticipantRow { combat: 7, side: 0, battle_slot: 0, ship: 11, owner: 44, state: 0 },
                            NavalCombatParticipantRow { combat: 7, side: 0, battle_slot: 1, ship: 12, owner: 44, state: 0 },
                        ]),
                        missing: 1,
                    };
                    frame.metrics.table_ns[TableId::NavalCombatParticipants.index()] = 1;
                } else if hour == 105 {
                    *frame.tables[TableId::NavalCombatParticipants.index()] = TableView::Unavailable { reason: "participants unreadable".into() };
                } else {
                    set::<tables::NavalCombatParticipants>(&mut frame, vec![
                        NavalCombatParticipantRow { combat: 7, side: 0, battle_slot: 0, ship: 11, owner: 44, state: 0 },
                        NavalCombatParticipantRow { combat: 7, side: 0, battle_slot: 1, ship: 12, owner: 44, state: 0 },
                        NavalCombatParticipantRow { combat: 7, side: 1, battle_slot: 0, ship: 21, owner: 45, state: 0 },
                    ]);
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Source::from_replay(dashboard_base::SourceKey { source_id: "naval-runs".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();

        let window = NavalBattles::load(&source, &NavalBattlesParams { branch_id: "1".into(), window: 0 }, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();
        let battle = window.rows.iter().find(|row| row.id == "combat:7:100").expect("live battle present");
        let early = battle.runs.iter().find(|run| run.first_hour <= 103 && run.last_hour >= 103).expect("early run");
        let truncated = battle.runs.iter().find(|run| run.first_hour <= 104 && run.last_hour >= 104).expect("truncated run");
        let late = battle.runs.iter().find(|run| run.first_hour <= 105 && run.last_hour >= 105).expect("late run");
        assert_eq!(early.province_id, Some(500));
        assert_eq!(truncated.province_id, Some(500));
        assert_eq!(late.province_id, Some(500));
        let early_side = serde_json::to_value(&early.row.sides[0]).unwrap();
        assert_eq!(early_side["counts"][0]["value"], 2.0);
        let early_defender = serde_json::to_value(&early.row.sides[1]).unwrap();
        assert_eq!(early_defender["counts"][0]["value"], 1.0);
        assert_eq!(early_defender["countries"][0]["tag"], "GER");
        let truncated_attacker = serde_json::to_value(&truncated.row.sides[0]).unwrap();
        assert_eq!(truncated_attacker["counts"][0]["value"], 2.0, "truncated roster keeps main's recorded attacking ships");
        assert_eq!(truncated_attacker["countries"][0]["tag"], "ENG");
        let truncated_defender = serde_json::to_value(&truncated.row.sides[1]).unwrap();
        assert_eq!(truncated_defender["counts"][0]["value"], 0.0, "truncated roster reflects the omitted defender rather than reusing hour 103");
        assert!(truncated_defender["countries"].as_array().unwrap().is_empty());
        let late_side = serde_json::to_value(&late.row.sides[0]).unwrap();
        assert_eq!(late_side["counts"][0]["unknown"], "Participants not recorded", "historical peak never makes an unread participant hour read zero");

        let moved_early = window.rows.iter().find(|row| row.id == "combat:8:100").expect("first province battle");
        let moved_late = window.rows.iter().find(|row| row.id == "combat:8:105").expect("new province battle");
        assert_eq!(moved_early.runs[0].province_id, Some(500));
        assert_eq!(moved_late.runs[0].province_id, Some(600));

        let truncated_ranking = NavalBattlesRanking::load(
            &source,
            &NavalBattlesRankingParams { branch_id: "1".into(), first_hour: 100, last_hour: 104, top_n: 5.0, measure: "Peak ships".into(), countries: NavalBattleCountryFilter::default() },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        let ranked_truncated = truncated_ranking.rows.iter().find(|row| row.id == "combat:7:100").expect("ranked battle at 104");
        assert_eq!(ranked_truncated.runs[0].province_id, Some(500));
        let ranked_attacker = serde_json::to_value(&ranked_truncated.runs[0].row.sides[0]).unwrap();
        assert_eq!(ranked_attacker["counts"][0]["value"], 2.0, "ranking keeps main's truncated participant count");
        assert_eq!(ranked_attacker["countries"][0]["tag"], "ENG");
        let ranked_defender = serde_json::to_value(&ranked_truncated.runs[0].row.sides[1]).unwrap();
        assert_eq!(ranked_defender["counts"][0]["value"], 0.0, "ranking drops the omitted defender at truncated hour 104");
        assert!(ranked_defender["countries"].as_array().unwrap().is_empty());

        let unread_ranking = NavalBattlesRanking::load(
            &source,
            &NavalBattlesRankingParams { branch_id: "1".into(), first_hour: 100, last_hour: 105, top_n: 5.0, measure: "Peak ships".into(), countries: NavalBattleCountryFilter::default() },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        let ranked_unread = unread_ranking.rows.iter().find(|row| row.id == "combat:7:100").expect("ranked battle at 105");
        assert_eq!(ranked_unread.runs[0].province_id, Some(500));
        let ranked_side = serde_json::to_value(&ranked_unread.runs[0].row.sides[0]).unwrap();
        assert_eq!(ranked_side["counts"][0]["unknown"], "Participants not recorded", "ranking keeps unread participant counts unknown at 105");
    }

    #[test]
    fn complete_first_hour_without_peak_and_province_changes_under_one_battle_id() {
        use super::super::battles::{NavalBattleCountryFilter, NavalBattles, NavalBattlesParams, NavalBattlesRanking, NavalBattlesRankingParams};
        use replay_data::rows::{CountryLinkRow, CountryRow, NavalBattleRow, NavalCombatParticipantRow, NavalCombatRow, Recorded, TableId};

        let current: Vec<usize> = replay_recording::schema::schema().unwrap().iter().map(Vec::len).collect();
        let mut columns = current.clone();
        columns[TableId::NavalBattles.index()] = 0;

        let frames = (100..=105)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
                countries.push(CountryRow { id: 44, canonical_slot: 2, tag: b"ENG".to_vec(), ..CountryRow::default() });
                countries.push(CountryRow { id: 45, canonical_slot: 3, tag: b"GER".to_vec(), ..CountryRow::default() });
                countries.sort_by_key(|row| row.id);
                set::<tables::Countries>(&mut frame, countries);
                let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
                links.push(CountryLinkRow { id: 44, canonical_slot: 2 });
                links.push(CountryLinkRow { id: 45, canonical_slot: 3 });
                links.sort_by_key(|row| row.id);
                set::<tables::CountryLinks>(&mut frame, links);
                let mut combats = vec![NavalCombatRow { id: 10, province: 500, ..NavalCombatRow::default() }];
                if (102..=103).contains(&hour) {
                    combats.push(NavalCombatRow { id: 9, province: 500, ..NavalCombatRow::default() });
                } else if hour == 104 {
                    combats.push(NavalCombatRow { id: 9, province: 600, ..NavalCombatRow::default() });
                }
                combats.sort_by_key(|row| row.id);
                set::<tables::NavalCombats>(&mut frame, combats);
                set::<tables::NavalBattles>(&mut frame, vec![
                    NavalBattleRow {
                        combat: 9,
                        first_hour: 100,
                        peak_ships: Recorded::Value(2),
                        peak_hour: Recorded::Value(100),
                        attacker_sunk: Recorded::Value(0),
                        defender_sunk: Recorded::Value(0),
                        attacker_convoys_sunk: Recorded::Value(0),
                        defender_convoys_sunk: Recorded::Value(0),
                    },
                    NavalBattleRow {
                        combat: 10,
                        first_hour: 100,
                        peak_ships: Recorded::NotRecorded,
                        peak_hour: Recorded::NotRecorded,
                        attacker_sunk: Recorded::Value(0),
                        defender_sunk: Recorded::NotRecorded,
                        attacker_convoys_sunk: Recorded::Value(0),
                        defender_convoys_sunk: Recorded::NotRecorded,
                    },
                ]);
                set::<tables::NavalCombatParticipants>(&mut frame, vec![
                    NavalCombatParticipantRow { combat: 9, side: 0, battle_slot: 0, ship: 11, owner: 44, state: 0 },
                    NavalCombatParticipantRow { combat: 9, side: 1, battle_slot: 0, ship: 21, owner: 45, state: 0 },
                    NavalCombatParticipantRow { combat: 10, side: 0, battle_slot: 0, ship: 11, owner: 44, state: 0 },
                    NavalCombatParticipantRow { combat: 10, side: 1, battle_slot: 0, ship: 21, owner: 45, state: 0 },
                ]);
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers_within(&columns, &current), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let source = Source::from_replay(dashboard_base::SourceKey { source_id: "naval-peakless".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();

        let window = NavalBattles::load(&source, &NavalBattlesParams { branch_id: "1".into(), window: 0 }, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();
        let moving = window.rows.iter().find(|row| row.id == "combat:9:100").expect("single derived battle id");
        let spans: Vec<_> = moving.runs.iter().map(|run| (run.first_hour, run.last_hour, run.province_id, run.row.place.hour, run.row.place.named_at)).collect();
        assert_eq!(spans, vec![(100, 101, None, 100, None), (102, 103, Some(500), 100, Some(102)), (104, 104, Some(600), 100, Some(104)), (105, 105, None, 100, None)]);

        let peakless = window.rows.iter().find(|row| row.id == "combat:10:100").expect("peakless complete battle");
        let attacker = serde_json::to_value(&peakless.runs[0].row.sides[0]).unwrap();
        let defender = serde_json::to_value(&peakless.runs[0].row.sides[1]).unwrap();
        assert_eq!(attacker["countries"][0]["tag"], "ENG", "complete roster resolves attacker even without a derived peak");
        assert_eq!(attacker["countriesUnknown"], serde_json::Value::Null);
        assert_eq!(defender["countries"][0]["tag"], "GER", "complete roster resolves defender even without a derived peak");
        assert_eq!(defender["countriesUnknown"], serde_json::Value::Null);

        let ranking = NavalBattlesRanking::load(
            &source,
            &NavalBattlesRankingParams { branch_id: "1".into(), first_hour: 100, last_hour: 105, top_n: 5.0, measure: "Length".into(), countries: NavalBattleCountryFilter::default() },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        let ranked = ranking.rows.iter().find(|row| row.id == "combat:10:100").expect("ranked peakless battle");
        let ranked_attacker = serde_json::to_value(&ranked.runs[0].row.sides[0]).unwrap();
        let ranked_defender = serde_json::to_value(&ranked.runs[0].row.sides[1]).unwrap();
        assert_eq!(ranked_attacker["countries"][0]["tag"], "ENG");
        assert_eq!(ranked_attacker["countriesUnknown"], serde_json::Value::Null);
        assert_eq!(ranked_defender["countries"][0]["tag"], "GER");
        assert_eq!(ranked_defender["countriesUnknown"], serde_json::Value::Null);
    }

    #[test]
    fn truncated_status_edge_keeps_mains_tag_lookup_hour_and_coalesces_missing_counts() {
        use super::super::battles::{NavalBattleCountryFilter, NavalBattles, NavalBattlesParams, NavalBattlesRanking, NavalBattlesRankingParams};
        use dashboard_base::products::Country;
        use replay_data::rows::{CountryLinkRow, CountryRow, NavalCombatParticipantRow, NavalCombatRow, TableId, TableRows};

        let frames = (100..=105)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
                countries.push(CountryRow { id: 44, canonical_slot: 2, tag: b"ENG".to_vec(), ..CountryRow::default() });
                countries.push(CountryRow { id: 45, canonical_slot: 3, tag: b"FRA".to_vec(), ..CountryRow::default() });
                countries.sort_by_key(|row| row.id);
                set::<tables::Countries>(&mut frame, countries);
                let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
                links.push(CountryLinkRow { id: 44, canonical_slot: if hour == 104 { 3 } else { 2 } });
                links.push(CountryLinkRow { id: 45, canonical_slot: 3 });
                links.sort_by_key(|row| row.id);
                set::<tables::CountryLinks>(&mut frame, links);
                set::<tables::NavalCombats>(&mut frame, vec![NavalCombatRow { id: 7, province: 500, ..NavalCombatRow::default() }]);
                let rows = vec![NavalCombatParticipantRow { combat: 7, side: 0, battle_slot: 0, ship: 11, owner: 44, state: 0 }];
                if hour >= 104 {
                    *frame.tables[TableId::NavalCombatParticipants.index()] = TableView::Truncated {
                        rows: TableRows::NavalCombatParticipants(rows),
                        missing: if hour == 104 { 1 } else { 2 },
                    };
                    frame.metrics.table_ns[TableId::NavalCombatParticipants.index()] = 1;
                } else {
                    set::<tables::NavalCombatParticipants>(&mut frame, rows);
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Source::from_replay(dashboard_base::SourceKey { source_id: "naval-trunc-tag".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();

        let window = NavalBattles::load(&source, &NavalBattlesParams { branch_id: "1".into(), window: 0 }, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();
        let battle = window.rows.iter().find(|row| row.id == "combat:7:100").expect("live battle present");
        assert_eq!(battle.runs.len(), 1, "unchanged truncated rows coalesce into one run");
        assert_eq!((battle.runs[0].first_hour, battle.runs[0].last_hour), (100, 105));
        let listed_tags: Vec<&str> = battle.runs[0].row.sides[0].countries.iter().map(|country| country.tag.as_str()).collect();
        assert_eq!(listed_tags, vec!["ENG"]);

        let ranking = NavalBattlesRanking::load(
            &source,
            &NavalBattlesRankingParams {
                branch_id: "1".into(),
                first_hour: 100,
                last_hour: 105,
                top_n: 5.0,
                measure: "Peak ships".into(),
                countries: NavalBattleCountryFilter { countries: Some(vec![Country::from("ENG")]), membership: None, unobserved: Vec::new() },
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        let ranked = ranking.rows.iter().find(|row| row.id == "combat:7:100").expect("ranking resolves ENG at 105");
        let ranked_tags: Vec<&str> = ranked.runs[0].row.sides[0].countries.iter().map(|country| country.tag.as_str()).collect();
        assert_eq!(ranked_tags, listed_tags);
    }
}
