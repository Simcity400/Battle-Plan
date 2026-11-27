use super::{
    battles::{battle_id, parse_id},
    losses::{Counts, NavalPeriod, month_bases, month_start, score},
    sunk::known_at,
};
use game_model::calendar::HOUR_EPOCH;
use replay_data::rows::NavalLossTotalRow as Row;
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
    let ships = Row::holder_key(owner, Row::LOST, Row::SHIPS);
    let run = Row::run_key(Row::SHIPS);
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
    assert_eq!(counts.count(Row::CONVOYS, Row::holder_key(owner, Row::LOST, Row::CONVOYS), start + 60, NavalPeriod::Running), None, "a kind without a run row is not known");
}

#[test]
fn a_months_ranking_subtracts_each_kind_at_its_own_baseline_as_its_counts_do() {
    let h = month_start(HOUR_EPOCH + 4 * 365 * 24 + 40 * 24) + 48;
    let (a, b) = (7u128 << 32 | 1, 8u128 << 32 | 2);
    let kinds = [Row::SHIPS, Row::CONVOYS];
    let known = HashMap::from([(Row::run_key(Row::SHIPS), vec![(h, h + 20, Some(1))]), (Row::run_key(Row::CONVOYS), vec![(h, h + 20, Some(1))])]);
    let counted = HashMap::from([(Row::run_key(Row::SHIPS), vec![(h, h + 20, Some(i64::from(h)))]), (Row::run_key(Row::CONVOYS), vec![(h, h + 20, Some(i64::from(h + 10)))])]);
    let (a_ships, b_ships) = (Row::holder_key(a, Row::LOST, Row::SHIPS), Row::holder_key(b, Row::LOST, Row::SHIPS));
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
fn a_battle_is_named_by_its_combat_and_first_hour() {
    assert_eq!(parse_id(&battle_id(25447681233513, 60768781)), Some((25447681233513, 60768781)));
    assert_eq!(parse_id("result:4"), None);
    assert_eq!(parse_id("combat:1:2:3"), None);
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
                set::<tables::NavalCombats>(
                    &mut frame,
                    [7, 8, 9].into_iter().map(|id| replay_data::rows::NavalCombatRow { id, province: 500, ..Default::default() }).collect(),
                );
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
        assert_eq!(together[5]["unavailable"], "Details are kept for battles at a recorded hour", "an hour outside the recording");
    }

    fn battle_tail_recording() -> Source {
        use replay_data::rows::{CountryLinkRow, CountryRow, NavalCombatParticipantRow, NavalCombatRow, NavalResultCountryRow, NavalResultRow, NavalResultSourceRow, TableId, TableRows};

        let frames = (100..=108)
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
                if hour == 108 {
                    *frame.tables[TableId::NavalResults.index()] = TableView::Unavailable { reason: "results unread".into() };
                } else if hour >= 107 {
                    set::<tables::NavalResults>(&mut frame, vec![
                        NavalResultRow { id: 61, province: 500, result_hour: 106, retire_hour: 180, winning_side: 2, ..Default::default() },
                        NavalResultRow { id: 62, province: 600, result_hour: 106, retire_hour: 180, winning_side: 2, ..Default::default() },
                    ]);
                    set::<tables::NavalResultSources>(&mut frame, vec![NavalResultSourceRow { id: 1, game_hour: 106, result: 61, combat: 7, combat_created_hour: Some(100), ..Default::default() }]);
                    set::<tables::NavalResultCountries>(&mut frame, vec![
                        NavalResultCountryRow { result: 62, list: 0, country: 44, ordinal: 0 },
                        NavalResultCountryRow { result: 62, list: 1, country: 99, ordinal: 0 },
                    ]);
                } else if hour == 106 {
                    set::<tables::NavalResults>(&mut frame, vec![NavalResultRow { id: 62, province: 600, result_hour: 106, retire_hour: 180, winning_side: 2, ..Default::default() }]);
                    *frame.tables[TableId::NavalResultCountries.index()] = TableView::Truncated {
                        rows: TableRows::NavalResultCountries(vec![
                            NavalResultCountryRow { result: 62, list: 0, country: 44, ordinal: 0 },
                            NavalResultCountryRow { result: 62, list: 1, country: 99, ordinal: 0 },
                        ]),
                        missing: 1,
                    };
                    frame.metrics.table_ns[TableId::NavalResultCountries.index()] = 1;
                } else {
                    set::<tables::NavalResults>(&mut frame, vec![]);
                }
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
                    let mut participants = vec![
                        NavalCombatParticipantRow { combat: 7, side: 0, battle_slot: 0, ship: 11, owner: 44, state: 0 },
                        NavalCombatParticipantRow { combat: 7, side: 0, battle_slot: 1, ship: 12, owner: 44, state: 0 },
                        NavalCombatParticipantRow { combat: 7, side: 1, battle_slot: 0, ship: 21, owner: 45, state: 0 },
                    ];
                    if hour == 106 {
                        participants.push(NavalCombatParticipantRow { combat: 8, side: 1, battle_slot: 0, ship: 31, owner: 99, state: 0 });
                    }
                    set::<tables::NavalCombatParticipants>(&mut frame, participants);
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(dashboard_base::SourceKey { source_id: "naval-runs".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn followed_battles_keep_recorded_flags_and_withhold_incomplete_side_totals() {
        use super::super::battles::tail::NavalBattlesTail;
        let source = battle_tail_recording();

        let page = |hour| NavalBattlesTail::load(&source, &serde_json::from_value(serde_json::json!({"branchId":"1","hour":hour,"limit":64})).unwrap(),
            &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();
        let row = |hour| page(hour).rows.into_iter().find(|row| row.key == "combat:7:100").unwrap();
        let coverage = |row| serde_json::to_value(row).unwrap()["countriesMissing"].clone();
        let truncated = "Country roster is truncated";
        let unread = "Country roster was not recorded";
        assert_eq!(row(103).ships, [Some(2), Some(1)]);
        let empty = page(103).rows.into_iter().find(|row| row.key == "combat:8:100").unwrap();
        assert!(empty.countries.iter().all(Vec::is_empty));
        assert_eq!(coverage(empty), serde_json::json!([null, null]));
        assert_eq!(page(103).next_change, Some(104));
        assert_eq!(row(104).ships, [None, None]);
        assert_eq!(row(104).countries, [vec![2], vec![]]);
        assert_eq!(coverage(row(104)), serde_json::json!([truncated, truncated]));
        assert_eq!(row(105).ships, [None, None]);
        assert!(row(105).countries.iter().all(Vec::is_empty));
        assert_eq!(coverage(row(105)), serde_json::json!([unread, unread]));
        assert!(page(105).rows.iter().any(|row| row.key == "combat:8:105"));
        assert_eq!(row(106).detail_id, "combat:7:100");
        let unlinked = page(106).rows.into_iter().find(|row| row.key == "combat:8:105").unwrap();
        assert_eq!(coverage(unlinked), serde_json::json!([null, "A recorded owner has no country link"]));
        let settled = row(107);
        assert_eq!(settled.detail_id, "result:61");
        assert_eq!(settled.result, super::super::battles::tail::NavalBattleStatus::DefenderWon);
        assert_eq!(settled.countries, [vec![2], vec![3]]);
        assert_eq!(settled.detail_hour, Some(107));
        assert_eq!(coverage(settled), serde_json::json!([null, null]));
        assert_eq!(page(107).total, page(106).total);
        assert!(page(108).missing.iter().any(|gap| gap.first_hour == 108));
        let filtered = NavalBattlesTail::load(&source, &serde_json::from_value(serde_json::json!({
            "branchId":"1","hour":107,"limit":1,"countries":[2],"sort":{"column":"ships","direction":"descending"}
        })).unwrap(), &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();
        assert_eq!(filtered.rows[0].key, "combat:7:100");
        use super::super::battles::tail::NavalBattleCourse;
        let course = |key: &str| NavalBattleCourse::load(&source, &serde_json::from_value(serde_json::json!({"branchId":"1","key":key})).unwrap(), &LoadContext { content: None }, &AtomicBool::new(false));
        let reads = course("combat:7:100").unwrap().reads;
        let at = |hour: i32| reads.iter().rev().find(|read| read.hour <= hour).unwrap();
        assert_eq!((at(103).ships, at(104).ships), ([Some(2), Some(1)], [None, None]), "the course holds the same readings as the battle's row at each hour");
        assert_eq!((at(107).hour, at(107).result), (107, super::super::battles::tail::NavalBattleStatus::DefenderWon));
        assert!(course("combat:9:100").is_err(), "a key with no battle in the branch is refused");
        assert!(course("combat:7:100").unwrap().row.is_none(), "a course asked without an hour carries no row");
        let at = |hour: i32| NavalBattleCourse::load(&source, &serde_json::from_value(serde_json::json!({"branchId":"1","key":"combat:7:100","hour":hour})).unwrap(), &LoadContext { content: None }, &AtomicBool::new(false)).unwrap().row.unwrap();
        let (shown, listed) = (at(107), row(107));
        assert_eq!((shown.detail_id, shown.result, shown.countries, shown.read_hour), (listed.detail_id, listed.result, listed.countries, listed.read_hour), "the course's row at an hour is the table's row then");
        assert_eq!(at(103).ships, [Some(2), Some(1)]);
        assert_eq!(coverage(at(104)), coverage(row(104)));
        assert!(NavalBattleCourse::validate(&serde_json::from_value(serde_json::json!({"branchId":"1","key":"province:7"})).unwrap()).is_err());
    }

    #[test]
    fn result_rosters_keep_witnessed_countries_and_explain_missing_ones() {
        use super::super::battles::tail::NavalBattlesTail;
        let source = battle_tail_recording();
        let result = |hour| {
            let params = serde_json::from_value(serde_json::json!({"branchId":"1","hour":hour,"limit":64})).unwrap();
            let page = NavalBattlesTail::load(&source, &params, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap();
            serde_json::to_value(page.rows.into_iter().find(|row| row.key == "result:62").unwrap()).unwrap()
        };
        assert_eq!(result(106)["countries"], serde_json::json!([[2], []]));
        assert_eq!(result(106)["countriesMissing"], serde_json::json!(["Country roster is truncated", "Country roster is truncated and a recorded owner has no country link"]));
        assert_eq!(result(107)["countriesMissing"], serde_json::json!([null, "A recorded owner has no country link"]));
    }

    #[test]
    fn a_battle_restarted_in_place_keeps_each_battles_sinkings_with_every_hour_read() {
        use super::super::{
            sinking_details::ShipSinkingBattle,
            sunk::tail::ShipsSunkTail,
        };
        use replay_data::rows::{NavalCombatRow, TableId};
        let entry = |sink_hour: i32, ship: &[u8]| SunkShipEntry {
            ship_name: ship.to_vec(),
            attribution_name: b"Killer".to_vec(),
            victim_country: 9001,
            attacker_country: 9001,
            sink_hour,
            class_token: sink_hour as u32,
            battle: 20,
            ..SunkShipEntry::default()
        };
        let frames = (100..=106)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                set::<tables::NavalCombats>(&mut frame, vec![NavalCombatRow { id: 20, province: if hour < 103 { 600 } else { 601 }, ..NavalCombatRow::default() }]);
                set::<tables::NavalCombatParticipants>(&mut frame, Vec::new());
                if hour == 102 {
                    *frame.tables[TableId::SunkShips.index()] = TableView::Unavailable { reason: "unread".into() };
                } else {
                    let entries = match hour {
                        101 => vec![
                            entry(101, b"Before"), entry(101, b"First escort"),
                            SunkShipEntry { battle: 0, ..entry(101, b"Unlinked") },
                        ],
                        105.. => vec![entry(104, b"After"), entry(104, b"Second escort")],
                        _ => Vec::new(),
                    };
                    set::<tables::SunkShips>(&mut frame, vec![SunkShipsRow { entries }]);
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Source::from_replay(dashboard_base::SourceKey { source_id: "restarted".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
        let page = |params| {
            let params = serde_json::from_value(params).unwrap();
            ShipsSunkTail::load(&source, &params, &LoadContext { content: None }, &AtomicBool::new(false)).unwrap()
        };
        let (sunk, reads) = replay_recording::cells::count_reads(|| page(serde_json::json!({"branchId":"1","limit":64})));
        assert!(!reads.contains_key(&TableId::NavalCombats), "sinking pages do not read a separate battle province history");
        let filtered = |hour| page(serde_json::json!({
            "branchId":"1", "hour":hour, "limit":64, "classes":[104]
        }));
        let early = filtered(101);
        assert!(early.rows.is_empty());
        assert_eq!(early.classes.iter().map(|chip| chip.class.token).collect::<Vec<_>>(), [101]);
        assert_eq!(early.next_change, Some(102));
        let gap = filtered(104);
        assert_eq!(gap.missing.iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>(), [(102, 102)]);
        assert_eq!(gap.next_change, Some(105));
        assert!(gap.rows.is_empty());
        let first_seen = filtered(105);
        assert_eq!((first_seen.rows[0].hour, first_seen.rows[0].observed), (104, 105));
        assert!(matches!(&first_seen.rows[0].battle, ShipSinkingBattle::Linked { id, .. } if id == "combat:20:103"));
        assert_eq!(first_seen.classes.len(), 2);
        let battle = |name: &str| sunk.rows.iter().find(|row| row.name.as_deref() == Some(name)).map(|row| row.battle.clone());
        let live = |first: i32| ShipSinkingBattle::Linked {
            id: format!("combat:20:{first}"),
            live_id: Some(format!("combat:20:{first}")),
            result_id: None,
            first_hour: Some(first),
            result_hour: None,
        };
        assert_eq!(battle("Before"), Some(live(100)), "a sinking before the restart is the first battle's");
        assert_eq!(battle("After"), Some(live(103)), "a sinking after the restart is the second battle's");
    }

    #[test]
    fn sinking_cause_and_battle_link_resolve_independently_causally_and_within_branch() {
        use super::super::{
            battles::{NavalBattleDetailRequest, NavalBattleDetails, NavalBattleDetailsParams},
            sinking_details::{SINKING_BATTLE_NOT_RECORDED, SINKING_CAUSE_NOT_RECORDED, ShipSinkingAttribution, ShipSinkingBattle, resolve_attribution},
            sunk::tail::ShipsSunkTail,
        };
        use replay_data::rows::{
            LexicalTokenRow, NavalCombatParticipantRow, NavalCombatRow, NavalResultRow, NavalResultSourceRow, SunkShipEntry, SunkShipsRow,
            TableId,
        };

        let make_entry = |sink_hour: i32, ship: &[u8], attr: &[u8], cause_record: u8, assisted_kill: u8, air_ref: u64, attr_class_token: u32, attr_class_name: &[u8], battle: u64| SunkShipEntry {
            ship_name: ship.to_vec(),
            attribution_name: attr.to_vec(),
            class_name: b"mod_hull".to_vec(),
            attribution_class_name: attr_class_name.to_vec(),
            victim_country: 9001,
            attacker_country: 9001,
            sink_hour,
            class_token: 101,
            attribution_class_token: attr_class_token,
            attribution_class_present: u8::from(attr_class_token != 0 || !attr_class_name.is_empty()),
            damage_source_reference: air_ref,
            battle,
            cause_record,
            assisted_kill,
            ..SunkShipEntry::default()
        };

        let mine = make_entry(100, b"S1", b"mod sea mine", 1, 1, 0, 0, b"", 0);
        assert_eq!(
            resolve_attribution(&mine, Some("mod sea mine".into()), None, None),
            ShipSinkingAttribution::Cause { text: "mod sea mine".into(), assisted: true }
        );
        let empty_cause = make_entry(100, b"S2", b"", 1, 0, 0, 0, b"", 9);
        assert_eq!(
            resolve_attribution(&empty_cause, None, None, None),
            ShipSinkingAttribution::NotRecorded { reason: SINKING_CAUSE_NOT_RECORDED.to_owned() }
        );
        let empty_hostile = make_entry(100, b"S3", b"", 0, 0, 0, 0, b"", 9);
        assert_eq!(
            resolve_attribution(&empty_hostile, None, None, None),
            ShipSinkingAttribution::NotRecorded { reason: SINKING_CAUSE_NOT_RECORDED.to_owned() }
        );
        let air_land = make_entry(100, b"S4", b"", 0, 0, 777, 0, b"", 0);
        assert_eq!(
            resolve_attribution(&air_land, None, None, None),
            ShipSinkingAttribution::AirWing { carrier_name: None, carrier_class_key: None, carrier_class_name: None, assisted: false }
        );
        let air_carrier = make_entry(100, b"S5", b"CV Mod", 0, 0, 888, 202, b"mod_carrier", 9);
        assert_eq!(
            resolve_attribution(&air_carrier, Some("CV Mod".into()), Some("mod_carrier".into()), Some("Mod Carrier".into())),
            ShipSinkingAttribution::AirWing {
                carrier_name: Some("CV Mod".into()),
                carrier_class_key: Some("mod_carrier".into()),
                carrier_class_name: Some("Mod Carrier".into()),
                assisted: false,
            }
        );

        let e_live_a = make_entry(101, b"Ship A", b"Killer Ship", 0, 0, 0, 201, b"mod_dd", 9);
        let e_live_dup = make_entry(101, b"Ship A", b"Killer Ship", 0, 0, 0, 201, b"mod_dd", 9);
        let e_del = make_entry(102, b"Ship Del", b"", 0, 0, 555, 0, b"", 9);
        let e_no_battle = make_entry(101, b"Ship Mine", b"mod minefield", 1, 0, 0, 0, b"", 0);
        let e_no_cause = make_entry(101, b"Ship Blank", b"", 0, 0, 0, 0, b"", 9);
        let e_reused = make_entry(105, b"Ship Reused", b"Later Killer", 0, 1, 0, 201, b"mod_dd", 9);
        let e_merged = make_entry(103, b"Ship Merged", b"CV Mod", 0, 0, 888, 202, b"mod_carrier", 12);
        let e_transient = make_entry(104, b"Ship Transient", b"Fast Ship", 0, 0, 0, 201, b"mod_dd", 13);
        let e_gap_del = make_entry(103, b"Ship Gap", b"Gap Killer", 0, 0, 0, 201, b"mod_dd", 14);
        let e_restarted = make_entry(101, b"Ship Restarted", b"First Killer", 0, 0, 0, 201, b"mod_dd", 20);

        let b1_frames: Vec<_> = (100..=106)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                set::<tables::LexicalTokens>(
                    &mut frame,
                    vec![
                        LexicalTokenRow { token: 101, literal: b"mod_hull".to_vec() },
                        LexicalTokenRow { token: 201, literal: b"mod_dd".to_vec() },
                        LexicalTokenRow { token: 202, literal: b"mod_carrier".to_vec() },
                    ],
                );
                let mut combats = Vec::new();
                let mut parts = Vec::new();
                if (100..=101).contains(&hour) {
                    combats.push(NavalCombatRow { id: 9, province: 500, ..NavalCombatRow::default() });
                    parts.push(NavalCombatParticipantRow { combat: 9, side: 0, battle_slot: 0, ship: 11, owner: 9001, state: 0 });
                }
                if (105..=106).contains(&hour) {
                    combats.push(NavalCombatRow { id: 9, province: 500, ..NavalCombatRow::default() });
                    parts.push(NavalCombatParticipantRow { combat: 9, side: 0, battle_slot: 0, ship: 12, owner: 9001, state: 0 });
                }
                combats.push(NavalCombatRow { id: 20, province: if hour <= 102 { 600 } else { 601 }, ..NavalCombatRow::default() });
                if (101..=102).contains(&hour) {
                    combats.push(NavalCombatRow { id: 14, province: 501, ..NavalCombatRow::default() });
                    parts.push(NavalCombatParticipantRow { combat: 14, side: 0, battle_slot: 0, ship: 14, owner: 9001, state: 0 });
                }
                combats.sort_by_key(|row| row.id);
                parts.sort_by_key(|row| (row.combat, row.side, row.battle_slot));
                if hour == 103 {
                    *frame.tables[TableId::NavalCombats.index()] = TableView::Unavailable { reason: "unread gap".into() };
                } else {
                    set::<tables::NavalCombats>(&mut frame, combats);
                }
                set::<tables::NavalCombatParticipants>(&mut frame, parts);
                if hour >= 104 {
                    set::<tables::NavalResults>(
                        &mut frame,
                        vec![NavalResultRow { id: 50, province: 500, result_hour: 104, retire_hour: 200, winning_side: 1, ..NavalResultRow::default() }],
                    );
                    set::<tables::NavalResultSources>(
                        &mut frame,
                        vec![
                            NavalResultSourceRow { id: 1, game_hour: 104, result: 50, combat: 9, merged: false, transient: false, combat_created_hour: Some(100) },
                            NavalResultSourceRow { id: 2, game_hour: 104, result: 50, combat: 12, merged: true, transient: false, combat_created_hour: Some(103) },
                            NavalResultSourceRow { id: 3, game_hour: 104, result: 50, combat: 13, merged: false, transient: true, combat_created_hour: Some(104) },
                        ],
                    );
                }
                let mut entries = vec![e_live_a.clone(), e_live_dup.clone(), e_no_battle.clone(), e_no_cause.clone()];
                if hour >= 101 {
                    entries.push(e_restarted.clone());
                }
                if hour >= 102 {
                    entries.push(e_del.clone());
                }
                if hour >= 103 {
                    entries.push(e_merged.clone());
                    entries.push(e_gap_del.clone());
                }
                if hour >= 104 {
                    entries.push(e_transient.clone());
                }
                if hour >= 105 {
                    entries.push(e_reused.clone());
                }
                set::<tables::SunkShips>(&mut frame, vec![SunkShipsRow { entries }]);
                frame
            })
            .collect();

        let b2_frames: Vec<_> = (100..=102)
            .map(|hour| {
                let mut frame = source(2, hour, Some(1));
                set::<tables::LexicalTokens>(&mut frame, vec![LexicalTokenRow { token: 101, literal: b"mod_hull".to_vec() }, LexicalTokenRow { token: 201, literal: b"mod_dd".to_vec() }]);
                set::<tables::SunkShips>(&mut frame, vec![SunkShipsRow { entries: vec![e_live_a.clone()] }]);
                frame
            })
            .collect();

        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, b1_frames), records(2, b2_frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let replay_source = Source::from_replay(
            dashboard_base::SourceKey { source_id: "sunk-cause-battle".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let ctx = LoadContext { content: None };
        let cancel = AtomicBool::new(false);

        let page_at_103 = |offset: &str| ShipsSunkTail::load(&replay_source, &serde_json::from_value(serde_json::json!({
            "branchId":"1", "hour":103, "to":103, "limit":64, "offset":offset
        })).unwrap(), &ctx, &cancel).unwrap();
        let before_result = page_at_103("0");
        let merged = before_result.rows.iter().find(|row| row.name.as_deref() == Some("Ship Merged")).unwrap();
        assert!(matches!(merged.battle, ShipSinkingBattle::NotRecorded { .. }));
        assert_eq!(before_result.next_change, Some(104));
        assert_eq!(page_at_103("64").next_change, None, "the result only changes pages showing the sinking");
        let b1 = ShipsSunkTail::load(&replay_source, &serde_json::from_value(serde_json::json!({"branchId":"1","limit":64})).unwrap(), &ctx, &cancel).unwrap();
        let dups: Vec<_> = b1.rows.iter().filter(|row| row.name.as_deref() == Some("Ship A")).collect();
        assert_eq!(dups.len(), 2, "same-hour duplicate sinkings stay distinct");
        assert_eq!(
            dups[0].sinking_cause,
            ShipSinkingAttribution::Ship { name: "Killer Ship".into(), class_key: Some("mod_dd".into()), class_name: Some("mod_dd".into()), assisted: false }
        );
        assert_eq!(
            dups[0].battle,
            ShipSinkingBattle::Linked {
                id: "combat:9:100".into(),
                live_id: Some("combat:9:100".into()),
                result_id: Some("result:50".into()),
                first_hour: Some(100),
                result_hour: Some(104),
            }
        );

        let del = b1.rows.iter().find(|row| row.name.as_deref() == Some("Ship Del")).unwrap();
        assert_eq!(
            del.sinking_cause,
            ShipSinkingAttribution::AirWing { carrier_name: None, carrier_class_key: None, carrier_class_name: None, assisted: false }
        );
        assert_eq!(
            del.battle,
            ShipSinkingBattle::Linked {
                id: "combat:9:100".into(),
                live_id: Some("combat:9:100".into()),
                result_id: Some("result:50".into()),
                first_hour: Some(100),
                result_hour: Some(104),
            }
        );

        let no_battle = b1.rows.iter().find(|row| row.name.as_deref() == Some("Ship Mine")).unwrap();
        assert_eq!(no_battle.sinking_cause, ShipSinkingAttribution::Cause { text: "mod minefield".into(), assisted: false });
        assert_eq!(no_battle.battle, ShipSinkingBattle::NotRecorded { reason: SINKING_BATTLE_NOT_RECORDED.to_owned() });

        let no_cause = b1.rows.iter().find(|row| row.name.as_deref() == Some("Ship Blank")).unwrap();
        assert_eq!(no_cause.sinking_cause, ShipSinkingAttribution::NotRecorded { reason: SINKING_CAUSE_NOT_RECORDED.to_owned() });
        assert!(matches!(no_cause.battle, ShipSinkingBattle::Linked { ref id, .. } if id == "combat:9:100"), "a sinking in a live battle is that battle's, its later result attached");

        let restarted = b1.rows.iter().find(|row| row.name.as_deref() == Some("Ship Restarted")).unwrap();
        assert_eq!(
            restarted.battle,
            ShipSinkingBattle::Linked {
                id: "combat:20:100".into(),
                live_id: Some("combat:20:100".into()),
                result_id: None,
                first_hour: Some(100),
                result_hour: None,
            },
            "a battle restarted under the same handle without deletion keeps its first battle's sinkings"
        );

        let reused = b1.rows.iter().find(|row| row.name.as_deref() == Some("Ship Reused")).unwrap();
        assert_eq!(
            reused.battle,
            ShipSinkingBattle::Linked {
                id: "combat:9:105".into(),
                live_id: Some("combat:9:105".into()),
                result_id: None,
                first_hour: Some(105),
                result_hour: None,
            },
            "reused combat handle resolves to its own incarnation only"
        );

        let merged = b1.rows.iter().find(|row| row.name.as_deref() == Some("Ship Merged")).unwrap();
        assert_eq!(
            merged.sinking_cause,
            ShipSinkingAttribution::AirWing {
                carrier_name: Some("CV Mod".into()),
                carrier_class_key: Some("mod_carrier".into()),
                carrier_class_name: Some("mod_carrier".into()),
                assisted: false,
            }
        );
        assert_eq!(
            merged.battle,
            ShipSinkingBattle::Linked { id: "result:50".into(), live_id: None, result_id: Some("result:50".into()), first_hour: None, result_hour: Some(104) }
        );

        let transient = b1.rows.iter().find(|row| row.name.as_deref() == Some("Ship Transient")).unwrap();
        assert_eq!(
            transient.battle,
            ShipSinkingBattle::Linked { id: "result:50".into(), live_id: None, result_id: Some("result:50".into()), first_hour: None, result_hour: Some(104) }
        );

        let gap_del = b1.rows.iter().find(|row| row.name.as_deref() == Some("Ship Gap")).unwrap();
        assert_eq!(
            gap_del.battle,
            ShipSinkingBattle::NotRecorded { reason: SINKING_BATTLE_NOT_RECORDED.to_owned() },
            "deletion hour is refused when NavalCombats is unavailable at gone + 1"
        );

        let b2 = ShipsSunkTail::load(&replay_source, &serde_json::from_value(serde_json::json!({"branchId":"2","limit":64})).unwrap(), &ctx, &cancel).unwrap();
        assert_eq!(b2.rows.len(), 1);
        assert_eq!(
            b2.rows[0].battle,
            ShipSinkingBattle::NotRecorded { reason: SINKING_BATTLE_NOT_RECORDED.to_owned() },
            "branch 2 must never join branch 1 battle handles"
        );

        let details = NavalBattleDetails::load(
            &replay_source,
            &NavalBattleDetailsParams {
                branch_id: "1".into(),
                battles: vec![
                    NavalBattleDetailRequest { id: "result:50".into(), hour: 102 },
                    NavalBattleDetailRequest { id: "combat:9:100".into(), hour: 102 },
                    NavalBattleDetailRequest { id: "result:50".into(), hour: 106 },
                    NavalBattleDetailRequest { id: "combat:9:105".into(), hour: 106 },
                ],
            },
            &ctx,
            &cancel,
        )
        .unwrap();
        assert!(details.rows[0].unavailable.is_some(), "result:50 does not exist before hour 104");
        assert!(details.rows[0].named_ships.is_empty());

        let live_102: Vec<_> = details.rows[1].named_ships.iter().filter_map(|s| s.name.as_deref()).collect();
        assert_eq!(live_102, vec!["Ship A", "Ship A", "Ship Blank", "Ship Del"]);

        assert_eq!(details.rows[2].unavailable, None);
        let res_106: Vec<_> = details.rows[2].named_ships.iter().filter_map(|s| s.name.as_deref()).collect();
        assert_eq!(res_106, vec!["Ship A", "Ship A", "Ship Blank", "Ship Del", "Ship Merged", "Ship Transient"]);
        assert_eq!(details.rows[2].named_ships[2].sinking_cause, ShipSinkingAttribution::NotRecorded { reason: SINKING_CAUSE_NOT_RECORDED.to_owned() });

        let live_105: Vec<_> = details.rows[3].named_ships.iter().filter_map(|s| s.name.as_deref()).collect();
        assert_eq!(live_105, vec!["Ship Reused"], "incarnation filtering keeps combat:9:105 separate from combat:9:100");
    }
}
