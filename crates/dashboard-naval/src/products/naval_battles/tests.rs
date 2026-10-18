use super::*;
mod newest;
mod places;
mod speed;
use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::NavalCombatParticipantRow,
    rows::NavalCombatResultDetailsRow, rows::NavalCombatRow, rows::NavalResultCountryRow,
    rows::NavalResultRow, rows::NavalResultShip, rows::NavalResultSourceRow, rows::TableRows,
};
use replay_frame::frame::TableView;

fn frame(hour: i32) -> DecodedFrame {
    let mut f = source(1, hour, Some(1));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *f.tables[TableId::Countries.index()]
    {
        rows.extend([(9002, 8, "mod defender"), (9003, 9, "mod viewer")].map(
            |(id, canonical_slot, tag)| CountryRow {
                id,
                canonical_slot,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            },
        ));
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *f.tables[TableId::CountryLinks.index()]
    {
        rows.extend(
            [(22, 8), (33, 9), (9002, 8), (9003, 9)]
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot }),
        );
        rows.sort_by_key(|row| row.id);
    }
    *f.tables[TableId::NavalCombats.index()] =
        TableView::Complete(TableRows::NavalCombats(vec![NavalCombatRow {
            id: 900,
            province: 45,
            ..Default::default()
        }]));
    *f.tables[TableId::NavalCombatParticipants.index()] =
        TableView::Complete(TableRows::NavalCombatParticipants(vec![
            NavalCombatParticipantRow {
                combat: 900,
                side: 0,
                battle_slot: 0,
                ship: 31,
                owner: 11,
                state: 0x22,
            },
            NavalCombatParticipantRow {
                combat: 900,
                side: 1,
                battle_slot: 0,
                ship: 0,
                owner: 22,
                state: 0x20,
            },
        ]));
    *f.tables[TableId::NavalResults.index()] =
        TableView::Complete(TableRows::NavalResults(vec![NavalResultRow {
            id: 800,
            province: 45,
            result_hour: hour - 1,
            winning_side: 2,
            losing_side: 1,
            flag_a: 1,
            ..Default::default()
        }]));
    *f.tables[TableId::NavalResultMembers.index()] =
        TableView::Complete(TableRows::NavalResultMembers(vec![
            NavalResultMemberRow {
                result: 800,
                member: 31,
                presence_value: 9,
                ordinal: 0,
                side: 0,
            },
            // Convoy references are nonzero in retained results.
            NavalResultMemberRow {
                result: 800,
                member: 32,
                presence_value: 0,
                ordinal: 1,
                side: 0,
            },
            NavalResultMemberRow {
                result: 800,
                member: 33,
                presence_value: 7,
                ordinal: 0,
                side: 1,
            },
        ]));
    *f.tables[TableId::NavalResultCountries.index()] =
        TableView::Complete(TableRows::NavalResultCountries(vec![
            NavalResultCountryRow {
                result: 800,
                country: 11,
                ordinal: 99,
                list: 0,
            },
            NavalResultCountryRow {
                result: 800,
                country: 22,
                ordinal: 0,
                list: 1,
            },
            NavalResultCountryRow {
                result: 800,
                country: 33,
                ordinal: 0,
                list: 2,
            },
        ]));
    let mut detail = NavalCombatResultDetailsRow {
        result: 800,
        ..Default::default()
    };
    detail.sides[0].ships = vec![
        NavalResultShip {
            member: 31,
            is_convoy: 0,
            ..Default::default()
        },
        NavalResultShip {
            member: 32,
            is_convoy: 1,
            ..Default::default()
        },
    ];
    detail.sides[1].ships = vec![NavalResultShip {
        member: 33,
        is_convoy: 0,
        ..Default::default()
    }];
    *f.tables[TableId::NavalCombatResultDetails.index()] =
        TableView::Complete(TableRows::NavalCombatResultDetails(vec![detail]));
    f
}

fn params(window: usize) -> NavalBattlesParams {
    NavalBattlesParams {
        branch_id: "1".into(),
        window,
    }
}
fn reply(scan: &Scan, hour: i32) -> NavalBattlesReply {
    scan.reply(
        &params(0),
        &[BattleWindow {
            first_hour: hour,
            last_hour: hour,
        }],
    )
    .unwrap()
}
fn result(reply: &NavalBattlesReply) -> &NavalBattleHistory {
    reply
        .rows
        .iter()
        .find(|r| r.id.starts_with("result:"))
        .unwrap()
}
fn combat(reply: &NavalBattlesReply) -> &NavalBattleHistory {
    reply
        .rows
        .iter()
        .find(|r| r.id.starts_with("combat:"))
        .unwrap()
}
fn last(history: &NavalBattleHistory) -> &NavalBattleRun {
    history.runs.last().unwrap()
}
fn value(value: &BattleValue) -> Option<f64> {
    match value {
        BattleValue::Known { value, .. } => Some(*value),
        BattleValue::Unknown { .. } => None,
    }
}
fn absent(f: &mut DecodedFrame) {
    *f.tables[TableId::NavalCombats.index()] = TableView::Complete(TableRows::NavalCombats(vec![]));
}
fn ranking(
    first_hour: i32,
    last_hour: i32,
    measure: &str,
    top_n: f64,
) -> NavalBattlesRankingParams {
    NavalBattlesRankingParams {
        branch_id: "1".into(),
        first_hour,
        last_hour,
        measure: measure.into(),
        top_n,
        countries: dashboard_land_battles::products::land_battles::LandBattleCountryFilter {
            tags: None,
            membership: None,
            unobserved: vec![],
        },
    }
}

fn result_source(f: &mut DecodedFrame, transient: bool) {
    *f.tables[TableId::NavalResultSources.index()] =
        TableView::Complete(TableRows::NavalResultSources(vec![NavalResultSourceRow {
            id: 1,
            game_hour: f.hour,
            result: 800,
            combat: if transient { u64::MAX } else { 900 },
            transient,
            combat_created_hour: transient.then_some(f.hour),
            ..Default::default()
        }]));
    *f.tables[TableId::NavalResults.index()] =
        TableView::Complete(TableRows::NavalResults(vec![NavalResultRow {
            id: 800,
            province: 45,
            result_hour: f.hour,
            winning_side: 2,
            losing_side: 1,
            ..Default::default()
        }]));
}

#[test]
fn naval_battles_native_link_replaces_live_row_with_observed_peak_and_length_once() {
    let mut f = frame(100);
    *f.tables[TableId::NavalResults.index()] = TableView::Complete(TableRows::NavalResults(vec![]));
    *f.tables[TableId::NavalResultSources.index()] =
        TableView::Complete(TableRows::NavalResultSources(vec![]));
    let mut scan = Scan::default();
    scan.observe(&f);
    f.hour = 101;
    result_source(&mut f, false);
    // A terminal combat can still be present in this census. The link, rather
    // than disappearance, replaces it with its one retained result.
    scan.observe(&f);
    let window = scan
        .reply(
            &params(0),
            &[BattleWindow {
                first_hour: 100,
                last_hour: 101,
            }],
        )
        .unwrap();
    let old = combat(&window);
    assert_eq!(last(old).last_hour, 100);
    let result = last(result(&window));
    assert_eq!(result.row.start.as_ref().unwrap().hour, 100);
    assert_eq!(result.row.place.hour, 100);
    assert_eq!(result.peak_hour, Some(100));
    assert_eq!(value(&result.measures[1]), Some(1.0));
    assert_eq!(value(&result.measures[2]), Some(1.0));
    assert_eq!(value(&result.row.sides[0].counts[0]), Some(1.0));
    assert_eq!(value(&result.row.sides[1].counts[0]), Some(0.0));
    assert!(matches!(result.row.winner, BattleWinner::Side { side: 1 }));
    for measure in ["Ships sunk", "Peak ships", "Length"] {
        let ranked = scan
            .ranked(&ranking(100, 101, measure, 30.0), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(ranked.rows.len(), 1);
        assert_eq!(ranked.rows[0].id, "result:800");
    }
    assert_eq!(
        scan.ranked(
            &ranking(100, 100, "Peak ships", 30.0),
            &AtomicBool::new(false)
        )
        .unwrap()
        .rows[0]
            .id,
        "combat:900:100"
    );
}

#[test]
fn naval_battles_transient_and_gap_links_never_guess_an_observed_episode() {
    for transient in [false, true] {
        let mut scan = Scan::default();
        let mut first = frame(100);
        *first.tables[TableId::NavalResults.index()] =
            TableView::Complete(TableRows::NavalResults(vec![]));
        scan.observe(&first);
        let mut ended = frame(if transient { 101 } else { 105 });
        absent(&mut ended);
        result_source(&mut ended, transient);
        scan.observe(&ended);
        let ended = reply(&scan, ended.hour);
        let run = last(result(&ended));
        assert!(run.row.start.is_none());
        assert_eq!(value(&run.measures[1]), None);
        assert_eq!(value(&run.measures[2]), None);
        for side in &run.row.sides {
            assert!(side.counts.iter().all(|count| value(count).is_none()));
        }
        assert_eq!(value(&run.measures[0]), Some(2.0));
        assert!(matches!(run.row.winner, BattleWinner::Side { side: 1 }));
    }
}

#[test]
fn naval_battles_windows_do_not_carry_retired_results_and_ranking_keeps_them() {
    let mut f = frame(100);
    let mut scan = Scan::default();
    scan.observe(&f);
    f.hour = 101;
    absent(&mut f);
    *f.tables[TableId::NavalResults.index()] = TableView::Complete(TableRows::NavalResults(vec![]));
    scan.observe(&f);
    assert!(reply(&scan, 101).rows.is_empty());
    let ranked = scan
        .ranked(
            &ranking(99, 101, "Ships sunk", 1.0),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(ranked.rows.len(), 1);
    assert_eq!(ranked.rows[0].id, "result:800");
    assert_eq!(last(&ranked.rows[0]).row.sides[0].countries[0].hour, 100);
    let mut p = ranking(99, 101, "Ships sunk", 5000.0);
    p.countries.tags = Some((0..5000).map(|i| format!("mod country {i}")).collect());
    assert!(NavalBattlesRanking::validate(&p).is_ok());
    assert!(
        scan.ranked(&p, &AtomicBool::new(false))
            .unwrap()
            .rows
            .is_empty()
    );
}

#[test]
fn naval_battles_ranking_filters_at_supplying_hour_and_keeps_unknown_after_zero() {
    let mut scan = Scan::default();
    let mut f = frame(100);
    scan.observe(&f);
    f.hour = 101;
    scan.observe(&f);
    let mut p = ranking(99, 101, "Ships sunk", 1.0);
    p.countries.tags = Some(vec!["other".into()]);
    let ranked = scan.ranked(&p, &AtomicBool::new(false)).unwrap();
    assert_eq!(ranked.rows.len(), 1);
    assert!(
        ranked.rows[0].id.starts_with("combat:"),
        "unknown participants pass selection"
    );
    p.countries.tags = None;
    let tag = last(result(&reply(&scan, 101))).row.sides[0].countries[0]
        .tag
        .clone();
    p.countries.membership = Some(vec![
        dashboard_base::products::country_groups::MembershipRun {
            tag,
            first_hour: 100,
            last_hour: 100,
        },
    ]);
    assert!(
        scan.ranked(&p, &AtomicBool::new(false)).unwrap().rows[0]
            .id
            .starts_with("combat:")
    );
    p.countries.membership.as_mut().unwrap()[0].last_hour = 101;
    assert_eq!(
        scan.ranked(&p, &AtomicBool::new(false)).unwrap().rows[0].id,
        "result:800"
    );
    p.countries.unobserved = vec![HourSpan {
        first_hour: 101,
        last_hour: 101,
    }];
    assert!(
        scan.ranked(&p, &AtomicBool::new(false)).unwrap().rows[0]
            .id
            .starts_with("combat:")
    );
    let at = scan.results[&800];
    for side in &mut scan.episodes[at].runs.last_mut().unwrap().sample.sides {
        side.sunk = Some(0);
    }
    p = ranking(99, 101, "Ships sunk", f64::MAX);
    let ranked = scan.ranked(&p, &AtomicBool::new(false)).unwrap();
    assert_eq!(ranked.rows.len(), 2);
    assert!(ranked.rows[0].id.starts_with("combat:"));
    assert_eq!(value(&last(&ranked.rows[1]).measures[0]), Some(0.0));
    let active = scan.combats[&900];
    scan.episodes[active].runs.last_mut().unwrap().sample.peak = Some(0);
    p.measure = "Peak ships".into();
    let ranked = scan.ranked(&p, &AtomicBool::new(false)).unwrap();
    assert_eq!(value(&last(&ranked.rows[0]).measures[1]), Some(0.0));
    assert_eq!(value(&last(&ranked.rows[1]).measures[1]), None);
    assert!(scan.ranked(&p, &AtomicBool::new(true)).is_err());
    for top_n in [0.0, -1.0, 1.5, f64::INFINITY] {
        p.top_n = top_n;
        assert!(NavalBattlesRanking::validate(&p).is_err());
    }
}

#[test]
fn naval_battles_results_in_recording_gaps_name_at_first_supplying_read() {
    let mut scan = Scan::default();
    let f = frame(100);
    scan.observe(&f);
    let mut f = frame(104);
    if let TableView::Complete(TableRows::NavalResults(rows)) =
        &mut *f.tables[TableId::NavalResults.index()]
    {
        rows[0].result_hour = 102;
    }
    scan.observe(&f);
    let r = reply(&scan, 104);
    let place = &last(result(&r)).row.place;
    assert_eq!(place.hour, 102);
    assert_eq!(place.named_at, Some(104));
    f.hour = 105;
    scan.observe(&f);
    assert_eq!(
        last(result(&reply(&scan, 105))).row.place.named_at,
        Some(104)
    );
    if let TableView::Complete(TableRows::NavalResults(rows)) =
        &mut *f.tables[TableId::NavalResults.index()]
    {
        rows[0].result_hour = 104;
    }
    f.hour = 106;
    scan.observe(&f);
    assert_eq!(last(result(&reply(&scan, 106))).row.place.named_at, None);
}

#[test]
fn naval_battles_ranking_ties_use_shared_id_order() {
    let mut f = frame(100);
    absent(&mut f);
    if let TableView::Complete(TableRows::NavalResults(rows)) =
        &mut *f.tables[TableId::NavalResults.index()]
    {
        let mut other = rows[0].clone();
        other.id = 801;
        // No child rows: unfiltered ranking must retain this result too.
        rows.push(other);
    }
    let mut scan = Scan::default();
    scan.observe(&f);
    assert_eq!(reply(&scan, 100).rows.len(), 2);
    let ranked = scan
        .ranked(
            &ranking(99, 100, "Peak ships", 30.0),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(
        ranked
            .rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["result:800", "result:801"]
    );
    assert!(
        ranked
            .rows
            .iter()
            .all(|row| value(&last(row).measures[1]).is_none())
    );
    let ranked = scan
        .ranked(
            &ranking(99, 100, "Ships sunk", 30.0),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(ranked.rows[0].id, "result:800");
    assert_eq!(value(&last(&ranked.rows[0]).measures[0]), Some(2.0));
    assert_eq!(ranked.rows[1].id, "result:801");
    assert_eq!(value(&last(&ranked.rows[1]).measures[0]), None);
    assert!(
        last(&ranked.rows[1])
            .row
            .sides
            .iter()
            .all(|side| side.countries.is_empty())
    );
}

#[test]
fn naval_battles_same_province_disappearance_and_new_result_prove_no_join() {
    let mut f = frame(100);
    *f.tables[TableId::NavalResults.index()] = TableView::Complete(TableRows::NavalResults(vec![]));
    let mut scan = Scan::default();
    scan.observe(&f);
    let mut f = frame(101);
    absent(&mut f);
    if let TableView::Complete(TableRows::NavalResults(rows)) =
        &mut *f.tables[TableId::NavalResults.index()]
    {
        rows[0].result_hour = 101;
    }
    scan.observe(&f);
    let r = reply(&scan, 101);
    assert_eq!(r.rows.len(), 1);
    let run = last(result(&r));
    assert!(run.row.start.is_none());
    assert!(
        matches!(&run.measures[1], BattleValue::Unknown { unknown, .. } if unknown == "Not recorded")
    );
    // A combat's result may be absent, and another engagement in the same
    // province can be aggregated within this hour.
    let ranked = scan
        .ranked(&ranking(100, 101, "Length", 30.0), &AtomicBool::new(false))
        .unwrap();
    assert!(
        last(&ranked.rows[0]).row.start.is_none(),
        "observed combat span must not be assigned by proximity"
    );
}

#[test]
fn naval_battles_window_contract_selectors_convoys_and_last_read_dates() {
    let mut scan = Scan::default();
    let mut f = frame(100);
    scan.observe(&f);
    f.hour = 101;
    scan.observe(&f);
    let windows = [BattleWindow {
        first_hour: 100,
        last_hour: 101,
    }];
    assert!(scan.reply(&params(1), &windows).is_err());
    let r = scan.reply(&params(0), &windows).unwrap();
    assert_eq!(r.observed, vec![(100, 101)]);
    assert_eq!(r.rows.len(), 2);
    let c = combat(&r);
    let cr = last(c);
    assert_eq!(cr.row.start.as_ref().unwrap().hour, 100);
    assert_eq!(cr.row.start.as_ref().unwrap().bound, BattleBound::By);
    assert_eq!(
        c.runs.len(),
        1,
        "Length must not prevent stable-run compression"
    );
    assert_eq!((cr.first_hour, cr.last_hour), (100, 101));
    assert!(cr.row.end.is_none());
    assert_eq!(cr.row.sides[0].label, "Attackers");
    assert_eq!(cr.row.sides[1].label, "Defenders");
    assert_eq!(cr.row.place.hour, 100);
    assert!(cr.row.place.name.is_none());
    assert_eq!(cr.row.sides[0].countries[0].hour, 101);
    assert_eq!(value(&cr.row.sides[1].counts[1]), Some(1.0));
    assert_eq!(value(&cr.row.sides[0].losses[0]), Some(1.0));
    assert_eq!(value(&cr.measures[0]), Some(1.0));
    assert_eq!(value(&cr.measures[1]), Some(1.0));
    assert!(
        matches!(&cr.measures[2], BattleValue::Known { label, value, .. }
        if label == "Length" && *value == 1.0)
    );
    assert!(
        cr.row.sides[0]
            .countries_unknown
            .as_ref()
            .is_some_and(|reason| !reason.is_empty())
    );
    assert_eq!(cr.row.sides[0].counts.len(), 2);
    let rr = last(result(&r));
    assert_eq!(
        rr.row.place.hour, 99,
        "result names are requested at the native result hour, not the later first read"
    );
    assert_eq!(rr.row.place.named_at, Some(100));
    assert!(rr.row.start.is_none());
    assert!(
        matches!(&rr.row.end, Some(BattleEnd::Moment(m)) if m.hour == 99 && m.bound == BattleBound::Exact)
    );
    assert!(matches!(rr.row.winner, BattleWinner::Side { side: 1 }));
    assert_eq!(rr.row.sides[0].label, "Side 1");
    assert_eq!(value(&rr.row.sides[0].counts[0]), None);
    assert_eq!(value(&rr.row.sides[0].counts[1]), None);
    assert_eq!(value(&rr.measures[0]), Some(2.0));
    assert_eq!(value(&rr.measures[1]), None);
    assert_eq!(
        rr.row.sides[0].countries.len(),
        1,
        "distinct viewer country must not enter either side"
    );
    assert_eq!(rr.row.sides[1].countries[0].tag, "mod defender");
    assert!(
        rr.row
            .sides
            .iter()
            .flat_map(|side| &side.countries)
            .all(|country| country.tag != "mod viewer")
    );
}

#[test]
fn naval_battles_only_live_combats_no_ended_end_consumer() {
    for gap in [false, true] {
        let mut scan = Scan::default();
        let mut f = frame(100);
        scan.observe(&f);
        f.hour = if gap { 103 } else { 101 };
        absent(&mut f);
        scan.observe(&f);
        assert_eq!(reply(&scan, f.hour).rows.len(), 1);
        assert_eq!(reply(&scan, 100).rows.len(), 2);
        assert_eq!(reply(&scan, 101).rows.len(), usize::from(!gap));
    }
    let mut scan = Scan::default();
    let mut f = frame(100);
    scan.observe(&f);
    assert!(
        last(combat(&reply(&scan, 100))).row.end.is_none(),
        "terminal present read is live"
    );
    f.hour = 101;
    *f.tables[TableId::NavalCombats.index()] = TableView::Unavailable {
        reason: "failed read".into(),
    };
    scan.observe(&f);
    assert_eq!(reply(&scan, 101).rows.len(), 1);
    assert!(reply(&scan, 105).rows.is_empty());
    f = frame(102);
    scan.observe(&f);
    assert_eq!(
        last(combat(&reply(&scan, 102)))
            .row
            .start
            .as_ref()
            .unwrap()
            .hour,
        102,
        "failed census splits episode"
    );
    let mut f = frame(200);
    *f.tables[TableId::NavalResults.index()] = TableView::Complete(TableRows::NavalResults(vec![]));
    let mut scan = Scan::default();
    scan.observe(&f);
    f.hour = 201;
    absent(&mut f);
    scan.observe(&f);
    assert!(
        reply(&scan, 201).rows.is_empty(),
        "accepted missing-result trade-off"
    );
}

#[test]
fn naval_battles_reused_handle_province_and_gaps_split_episodes() {
    let mut scan = Scan::default();
    let mut f = frame(100);
    scan.observe(&f);
    f.hour = 105;
    scan.observe(&f);
    assert_eq!(
        last(combat(&reply(&scan, 105)))
            .row
            .start
            .as_ref()
            .unwrap()
            .hour,
        105
    );
    f.hour = 106;
    if let TableView::Complete(TableRows::NavalCombats(rows)) =
        &mut *f.tables[TableId::NavalCombats.index()]
    {
        rows[0].province = 46;
    }
    scan.observe(&f);
    let r = reply(&scan, 106);
    assert_eq!(last(combat(&r)).row.start.as_ref().unwrap().hour, 106);
    assert_eq!(combat(&r).province_id, 46);
    assert_eq!(combat(&reply(&scan, 100)).province_id, 45);
}

#[test]
fn naval_battles_peak_and_sunk_floors_survive_missing_or_detached_participants() {
    let mut scan = Scan::default();
    let mut f = frame(100);
    scan.observe(&f);
    f.hour = 101;
    *f.tables[TableId::NavalCombatParticipants.index()] = TableView::Unavailable {
        reason: "failed".into(),
    };
    scan.observe(&f);
    let r = reply(&scan, 101);
    let c = last(combat(&r));
    assert_eq!(value(&c.row.sides[0].counts[0]), None);
    assert_eq!(value(&c.row.sides[0].losses[0]), Some(1.0));
    assert_eq!(value(&c.row.sides[1].losses[1]), Some(1.0));
    for v in [&c.measures[0], &c.measures[1], &c.row.sides[0].losses[0]] {
        assert!(matches!(
            v,
            BattleValue::Known {
                at_least: Some(true),
                ..
            }
        ));
    }
    f.hour = 102;
    *f.tables[TableId::NavalCombatParticipants.index()] =
        TableView::Complete(TableRows::NavalCombatParticipants(vec![]));
    scan.observe(&f);
    let r = reply(&scan, 102);
    let c = last(combat(&r));
    assert_eq!(value(&c.measures[0]), Some(1.0));
    assert_eq!(value(&c.measures[1]), Some(1.0));
    assert_eq!(value(&c.row.sides[0].counts[0]), Some(0.0));
    let mut scan = Scan::default();
    let mut f = frame(100);
    for (hour, side) in [(100, 0), (101, 1)] {
        f.hour = hour;
        *f.tables[TableId::NavalCombatParticipants.index()] =
            TableView::Complete(TableRows::NavalCombatParticipants(
                (0..2)
                    .map(|slot| NavalCombatParticipantRow {
                        combat: 900,
                        side,
                        battle_slot: slot,
                        ship: u64::from(slot) + 31,
                        owner: 11,
                        state: 1,
                    })
                    .collect(),
            ));
        scan.observe(&f);
    }
    assert_eq!(
        value(&last(combat(&reply(&scan, 101))).measures[1]),
        Some(2.0),
        "peak is simultaneous, not summed side maxima"
    );
}

#[test]
fn naval_battles_invalid_recorded_operands_make_only_the_affected_read_unknown() {
    let mut scan = Scan::default();
    let mut f = frame(100);
    scan.observe(&f);
    f.hour = 101;
    if let TableView::Complete(TableRows::NavalResults(rows)) =
        &mut *f.tables[TableId::NavalResults.index()]
    {
        rows[0].winning_side = 9;
    }
    scan.observe(&f);
    let r = reply(&scan, 101);
    assert_eq!(r.rows.len(), 1);
    assert!(r.observed.is_empty());
    assert!(
        r.warnings
            .iter()
            .any(|w| w.reason.contains("Invalid result selector") && w.count == 1)
    );
    f = frame(102);
    if let TableView::Complete(TableRows::NavalResultMembers(rows)) =
        &mut *f.tables[TableId::NavalResultMembers.index()]
    {
        rows[0].side = 9;
    }
    if let TableView::Complete(TableRows::NavalResultCountries(rows)) =
        &mut *f.tables[TableId::NavalResultCountries.index()]
    {
        rows[0].list = 9;
    }
    scan.observe(&f);
    let r = reply(&scan, 102);
    assert_eq!(value(&last(result(&r)).row.sides[0].counts[0]), None);
    assert!(last(result(&r)).row.sides[0].countries_unknown.is_some());
    f = frame(103);
    if let TableView::Complete(TableRows::NavalCombatResultDetails(rows)) =
        &mut *f.tables[TableId::NavalCombatResultDetails.index()]
    {
        rows[0].sides[0].ships[0].is_convoy = 2;
    }
    scan.observe(&f);
    assert!(reply(&scan, 103).warnings.iter().any(|w| {
        w.reason
            .contains("Invalid or duplicate result classification")
    }));
    f = frame(104);
    if let TableView::Complete(TableRows::NavalResults(rows)) =
        &mut *f.tables[TableId::NavalResults.index()]
    {
        rows[0].result_hour = 105;
    }
    scan.observe(&f);
    assert!(reply(&scan, 104).observed.is_empty());
}

#[test]
fn naval_battles_result_province_change_splits_only_that_episode() {
    let mut scan = Scan::default();
    let mut f = frame(100);
    scan.observe(&f);
    f.hour = 101;
    if let TableView::Complete(TableRows::NavalResults(rows)) =
        &mut *f.tables[TableId::NavalResults.index()]
    {
        rows[0].province = 99;
        let mut other = rows[0].clone();
        other.id = 801;
        other.province = 45;
        rows.push(other);
    }
    scan.observe(&f);
    f.hour = 102;
    scan.observe(&f);
    let r = reply(&scan, 102);
    assert_eq!(r.rows.len(), 3);
    assert!(
        r.rows
            .iter()
            .any(|h| h.id == "result:800:101" && h.province_id == 99)
    );
    assert!(r.rows.iter().any(|h| h.id == "result:801"));
    assert_eq!(r.observed, vec![(102, 102)]);
    assert_eq!(result(&reply(&scan, 100)).province_id, 45);
}

#[test]
fn naval_battles_result_details_bug_keeps_unknown_and_recorded_selector() {
    let mut scan = Scan::default();
    let mut f = frame(100);
    scan.observe(&f);
    f.hour = 101;
    *f.tables[TableId::NavalCombatResultDetails.index()] = TableView::Unavailable {
        reason: "live bug".into(),
    };
    scan.observe(&f);
    assert_eq!(value(&last(result(&reply(&scan, 101))).measures[0]), None);
    assert!(matches!(
        last(result(&reply(&scan, 101))).row.winner,
        BattleWinner::Side { side: 1 }
    ));
    assert_eq!(
        value(&last(result(&reply(&scan, 100))).measures[0]),
        Some(2.0)
    );
    f = frame(102);
    if let TableView::Complete(TableRows::NavalResults(rows)) =
        &mut *f.tables[TableId::NavalResults.index()]
    {
        rows[0].winning_side = 0;
    }
    scan.observe(&f);
    assert!(
        matches!(&last(result(&reply(&scan, 102))).row.winner, BattleWinner::Unknown { unknown } if unknown == "No winner recorded")
    );
}

#[test]
fn naval_battles_result_changes_cached_raw_inputs_and_identity_refresh_after_gap() {
    let mut scan = Scan::default();
    let mut f = frame(100);
    scan.observe(&f);
    f.hour = 101;
    scan.observe(&f);
    assert_eq!(scan.episodes[scan.results[&800]].runs.len(), 1);
    let retained = scan.result_inputs[&800].members.as_ref().unwrap().as_ptr();
    f.hour = 102;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *f.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(99);
    }
    scan.observe(&f);
    assert_eq!(
        scan.result_inputs[&800].members.as_ref().unwrap().as_ptr(),
        retained,
        "balance-only changes must not rebuild"
    );
    if let TableView::Complete(TableRows::NavalResults(rows)) =
        &mut *f.tables[TableId::NavalResults.index()]
    {
        let mut other = rows[0].clone();
        other.id = 801;
        rows.push(other);
    }
    f.hour = 103;
    scan.observe(&f);
    assert_eq!(
        scan.result_inputs[&800].members.as_ref().unwrap().as_ptr(),
        retained,
        "a different result changing must compare borrowed inputs without cloning this result"
    );
    if let TableView::Complete(TableRows::NavalResults(rows)) =
        &mut *f.tables[TableId::NavalResults.index()]
    {
        rows.pop();
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *f.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"renamed mod country".to_vec();
    }
    f.hour = 104;
    scan.observe(&f);
    assert_eq!(
        scan.result_inputs[&800].members.as_ref().unwrap().as_ptr(),
        retained,
        "tag identity changes rebuild labels without cloning unchanged raw rows"
    );
    assert_eq!(
        last(result(&reply(&scan, 104))).row.sides[0].countries[0].tag,
        "renamed mod country"
    );
    f.hour = 105;
    if let TableView::Complete(TableRows::NavalResults(rows)) =
        &mut *f.tables[TableId::NavalResults.index()]
    {
        rows[0].result_hour = 105;
        rows[0].winning_side = 1;
    }
    scan.observe(&f);
    assert!(matches!(
        last(result(&reply(&scan, 104))).row.winner,
        BattleWinner::Side { side: 1 }
    ));
    assert!(matches!(
        last(result(&reply(&scan, 105))).row.winner,
        BattleWinner::Side { side: 0 }
    ));
    f.hour = 106;
    let results = f.tables[TableId::NavalResults.index()].clone();
    *f.tables[TableId::NavalResults.index()] = TableView::Unavailable {
        reason: "census failed".into(),
    };
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *f.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"changed in census gap".to_vec();
    }
    scan.observe(&f);
    f.hour = 107;
    f.tables[TableId::NavalResults.index()] = results;
    scan.observe(&f);
    assert_eq!(
        last(result(&reply(&scan, 107))).row.sides[0].countries[0].tag,
        "changed in census gap"
    );
    f.hour = 108;
    *f.tables[TableId::NavalResults.index()] = TableView::Complete(TableRows::NavalResults(vec![]));
    absent(&mut f);
    scan.observe(&f);
    assert!(
        reply(&scan, 109).rows.is_empty(),
        "no retired inventory in later windows"
    );
    let r = scan
        .ranked(
            &ranking(100, 109, "Ships sunk", 30.0),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(last(&r.rows[0]).row.sides[0].countries[0].hour, 107);
    assert_eq!(last(&r.rows[0]).row.place.hour, 105);
}

#[test]
fn naval_battles_opaque_mod_country_fixture() {
    let mut f = frame(100);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *f.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"opaque mod country".to_vec();
    }
    let mut scan = Scan::default();
    scan.observe(&f);
    assert_eq!(
        last(result(&reply(&scan, 100))).row.sides[0].countries[0].tag,
        "opaque mod country"
    );
}

fn war_frame(countries: u32, generation: u32) -> DecodedFrame {
    let mut f = frame(100);
    *f.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=countries)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("mod country {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *f.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=countries)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    let mut combats = Vec::new();
    let mut participants = Vec::new();
    let mut results = Vec::new();
    let mut members = Vec::new();
    let mut result_countries = Vec::new();
    let mut details = Vec::new();
    let mut combat_details = Vec::new();
    for i in 0..50u32 {
        let ended = u64::from(generation * 50 + i + 1);
        let id = ended + 50;
        let result_id = ended + 1_000_000;
        combats.push(NavalCombatRow {
            id,
            province: i + 1,
            ..Default::default()
        });
        results.push(NavalResultRow {
            id: result_id,
            province: i + 1,
            result_hour: 100 + (generation * 10) as i32,
            winning_side: 1,
            losing_side: 2,
            ..Default::default()
        });
        let owner = generation * 50 % countries + i % countries + 1;
        let owner = (owner - 1) % countries + 1;
        let mut detail = NavalCombatResultDetailsRow {
            result: result_id,
            ..Default::default()
        };
        let mut active = replay_data::rows::NavalCombatDetailsRow {
            combat: id,
            province: i + 1,
            custom_sea: Some(vec![]),
            ..Default::default()
        };
        for side in 0..2u8 {
            let owner = (owner - 1 + u32::from(side)) % countries + 1;
            result_countries.push(NavalResultCountryRow {
                result: result_id,
                country: owner,
                list: side,
                ..Default::default()
            });
            active.sides[usize::from(side)].summary.countries = vec![owner];
            for slot in 0..13u32 {
                let member = ended * 100 + u64::from(side) * 20 + u64::from(slot);
                let live_ship = id * 100 + u64::from(side) * 20 + u64::from(slot);
                let sunk_by = if slot % 3 == 0 {
                    b"Fixture sinker".to_vec()
                } else {
                    vec![]
                };
                let convoy = slot != 0;
                participants.push(NavalCombatParticipantRow {
                    combat: id,
                    side,
                    battle_slot: slot,
                    ship: if convoy { 0 } else { live_ship },
                    owner,
                    state: if slot % 3 == 0 { 0x20 } else { 1 },
                });
                members.push(NavalResultMemberRow {
                    result: result_id,
                    member,
                    side,
                    ordinal: slot,
                    presence_value: sunk_by.len() as u64,
                });
                detail.sides[usize::from(side)].ships.push(NavalResultShip {
                    member,
                    is_convoy: u8::from(convoy),
                    sunk_by,
                    ..Default::default()
                });
            }
        }
        details.push(detail);
        combat_details.push(active);
    }
    *f.tables[TableId::NavalCombats.index()] =
        TableView::Complete(TableRows::NavalCombats(combats));
    *f.tables[TableId::NavalCombatParticipants.index()] =
        TableView::Complete(TableRows::NavalCombatParticipants(participants));
    *f.tables[TableId::NavalCombatDetails.index()] =
        TableView::Complete(TableRows::NavalCombatDetails(combat_details));
    *f.tables[TableId::NavalResults.index()] =
        TableView::Complete(TableRows::NavalResults(results));
    *f.tables[TableId::NavalResultMembers.index()] =
        TableView::Complete(TableRows::NavalResultMembers(members));
    *f.tables[TableId::NavalResultCountries.index()] =
        TableView::Complete(TableRows::NavalResultCountries(result_countries));
    *f.tables[TableId::NavalCombatResultDetails.index()] =
        TableView::Complete(TableRows::NavalCombatResultDetails(details));
    f
}

fn recording_windows(replay: &Replay) -> Vec<BattleWindow> {
    replay
        .index
        .iter()
        .filter(|b| b.branch == 1)
        .map(|b| BattleWindow {
            first_hour: b.first_hour,
            last_hour: b.last_hour,
        })
        .collect()
}
#[test]
#[ignore = "Long realistic fixture; lead prints reply sizes"]
fn naval_battles_long_convoy_war_1500_and_5000_countries_window_reply_sizes() {
    // A lightweight replay supplies actual writer block boundaries. Cold wire
    // coverage for all naval details is exercised by the other ignored fixture.
    let layout = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            (100..4100).map(|h| source(1, h, Some(0))).collect(),
        )],
    )
    .unwrap();
    let windows = recording_windows(&layout);
    for countries in [1500, 5000] {
        let mut scan = Scan::default();
        for generation in 0..400 {
            let mut f = war_frame(countries, generation);
            let original = f.tables[TableId::Countries.index()].clone();
            let mut changed = f.complete::<tables::Countries>().unwrap().to_vec();
            changed[0].political_power_raw = Some(1);
            let changed: SharedView = TableView::Complete(TableRows::Countries(changed)).into();
            for elapsed in 0..10 {
                f.hour = 100 + (generation * 10) as i32 + elapsed;
                f.tables[TableId::Countries.index()] = if elapsed % 2 == 0 {
                    original.clone()
                } else {
                    changed.clone()
                };
                scan.observe(&f);
            }
        }
        assert_eq!(scan.results.len(), 20_000);
        assert_eq!(
            scan.episodes
                .iter()
                .filter(|e| e.kind == Kind::Combat)
                .map(|e| e.runs.len())
                .sum::<usize>(),
            20_000
        );
        let r = scan.reply(&params(windows.len() - 1), &windows).unwrap();
        let final_window = windows.last().unwrap();
        let expected = (0..400)
            .filter(|g| 109 + g * 10 >= final_window.first_hour)
            .count()
            * 50;
        assert_eq!(
            r.rows
                .iter()
                .filter(|h| h.id.starts_with("result:"))
                .count(),
            expected
        );
        assert!(
            r.rows
                .iter()
                .flat_map(|h| &h.runs)
                .all(|run| run.first_hour >= final_window.first_hour)
        );
        assert_eq!(
            r.rows
                .iter()
                .filter(|h| h.id.starts_with("combat:"))
                .filter(|h| h
                    .runs
                    .iter()
                    .any(|run| run.first_hour <= 4099 && run.last_hour >= 4099))
                .count(),
            50
        );
        let ranked = scan
            .ranked(
                &ranking(100, 4099, "Ships sunk", 30.0),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(ranked.rows.len(), 30);
        eprintln!(
            "naval_battles countries={countries}, hours=4000, results=20000, active=50, 1300 participants/result members per read; final window={} bytes, top30={} bytes",
            json_len(&r, MAX_PRODUCT_REPLY_BYTES).unwrap(),
            json_len(&ranked, MAX_PRODUCT_REPLY_BYTES).unwrap()
        );
    }
}
#[test]
fn naval_battles_uncapped_members_countries_and_only_transport_refusal() {
    let mut f = war_frame(5000, 0);
    *f.tables[TableId::NavalResultCountries.index()] =
        TableView::Complete(TableRows::NavalResultCountries(
            (1..=5000)
                .map(|country| NavalResultCountryRow {
                    result: 1_000_001,
                    country,
                    list: 0,
                    ..Default::default()
                })
                .collect(),
        ));
    let mut scan = Scan::default();
    scan.observe(&f);
    let r = reply(&scan, 100);
    assert_eq!(
        last(r.rows.iter().find(|r| r.id == "result:1000001").unwrap())
            .row
            .sides[0]
            .countries
            .len(),
        5000
    );
    let result_index = scan.results[&1_000_001];
    scan.episodes[result_index].runs[0].sample.sides[0].countries =
        vec!["x".repeat(MAX_PRODUCT_REPLY_BYTES)];
    assert!(
        scan.reply(
            &params(0),
            &[BattleWindow {
                first_hour: 100,
                last_hour: 100
            }]
        )
        .unwrap_err()
        .to_string()
        .contains("MAX_PRODUCT_REPLY_BYTES")
    );
    assert!(reply(&Scan::default(), 100).rows.is_empty());
    let mut invalid = params(0);
    invalid.branch_id = "bad".into();
    assert!(NavalBattles::validate(&invalid).is_err());
}
#[test]
fn naval_battles_wire_windows_cache_and_no_ship_name_tables() {
    let mut first = frame(100);
    let mut second = first.clone();
    second.hour = 101;
    absent(&mut second);
    first.branch = 2;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(1, vec![frame(100), second]),
            records(2, vec![first]),
        ],
    )
    .unwrap();
    let windows = recording_windows(&replay);
    let scan = super::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scan.reply(&params(0), &windows).unwrap().rows.len(), 2);
    assert!(!CENSUS_TABLES.contains(&TableId::Ships));
    assert!(!CENSUS_TABLES.contains(&TableId::Fleets));
    assert!(!CENSUS_TABLES.contains(&TableId::TaskForces));
    assert!(super::scan(&replay, 1, &AtomicBool::new(true)).is_err());
    let src = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "naval fixture".into(),
            revision: "1".into(),
        },
        "naval-fixture.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let context = LoadContext { content: None };
    let manifest_params = NavalBattlesManifestParams {
        branch_id: "1".into(),
    };
    let manifest =
        NavalBattlesManifest::load(&src, &manifest_params, &context, &AtomicBool::new(false))
            .unwrap();
    assert_eq!(manifest.windows.len(), windows.len());
    assert!(manifest.available);
    let state = src.state::<Cache>();
    assert!(
        state.lock().unwrap().branches.is_empty(),
        "manifest must not build a census or hold the scan lock"
    );
    NavalBattles::load(&src, &params(0), &context, &AtomicBool::new(false)).unwrap();
    let first = state.lock().unwrap().get(1).unwrap();
    NavalBattles::load(&src, &params(0), &context, &AtomicBool::new(false)).unwrap();
    assert!(Arc::ptr_eq(&first, &state.lock().unwrap().get(1).unwrap()));
    let mut p = params(0);
    p.branch_id = "2".into();
    NavalBattles::load(&src, &p, &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(state.lock().unwrap().branches.len(), 2);
    assert!(Arc::ptr_eq(&first, &state.lock().unwrap().get(1).unwrap()));
    state.lock().unwrap().insert(3, Arc::new(Scan::default()));
    assert!(
        state.lock().unwrap().get(2).is_none(),
        "least recently used branch evicted, never rows"
    );
}

#[test]
#[ignore = "Full cold wire fixture; lead prints reply sizes"]
fn naval_battles_long_convoy_war_wire_sizes() {
    for countries in [1500, 5000] {
        let mut frames = Vec::new();
        for generation in 0..400 {
            let mut f = war_frame(countries, generation);
            for elapsed in 0..10 {
                f.hour = 100 + (generation * 10) as i32 + elapsed;
                frames.push(f.clone());
            }
        }
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let windows = recording_windows(&replay);
        let scan = super::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
        assert_eq!(scan.results.len(), 20_000);
        let r = scan.reply(&params(windows.len() - 1), &windows).unwrap();
        eprintln!(
            "naval_battles cold wire countries={countries}: {} final-window bytes",
            json_len(&r, MAX_PRODUCT_REPLY_BYTES).unwrap()
        );
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_naval_matches_oracle_across_commits_resume_append_and_large_census() {
    use dashboard_base::prepared::series::tests::Fixture;
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut inputs = Vec::new();
    for hour in 100..103 {
        let mut input = frame(hour);
        let scaled = dashboard_base::test_fixtures::scale_frame(1, hour, 5000);
        input.tables[TableId::Countries.index()] =
            scaled.tables[TableId::Countries.index()].clone();
        input.tables[TableId::CountryLinks.index()] =
            scaled.tables[TableId::CountryLinks.index()].clone();
        *input.tables[TableId::NavalCombats.index()] =
            TableView::Complete(TableRows::NavalCombats(
                (0..2048)
                    .map(|index| NavalCombatRow {
                        id: (1_u64 << 48) + index,
                        province: 45,
                        ..Default::default()
                    })
                    .collect(),
            ));
        inputs.push(input);
    }
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![inputs[0].clone()])]).unwrap();
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
            source_id: "prepared-naval".into(),
            revision: "1".into(),
        },
        "prepared-naval.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let fixture = Fixture::new();
    let mut prepared = Scan::default();
    let content = crate::products::ships_sunk::prepared_content(&source, &context);
    let mut changes = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    for (index, input) in inputs.iter().enumerate() {
        prepared.configure(&source, 1, &content).unwrap();
        let frame_changes = changes.observe(input, source.replay_ref().format);
        prepared.observe_prepared(input, &frame_changes);
        prepared
            .observe_details(&source, &context, input, &flag, &frame_changes)
            .unwrap();
        let mut writer = fixture.writer();
        prepared
            .publish_block(&source, &context, &mut writer, 1, index, &flag)
            .unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        prepared.resume(input);
        changes.invalidate();
    }
    let oracle = scan(source.replay_ref(), 1, &flag).unwrap();
    let windows = super::windows(source.replay_ref(), 1);
    for index in 0..windows.len() {
        let params = NavalBattlesParams {
            branch_id: "1".into(),
            window: index,
        };
        let actual = prepared::query(
            &source,
            "naval-battles",
            serde_json::to_value(&params).unwrap(),
            &context,
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(
            actual,
            serde_json::to_value(oracle.reply(&params, &windows).unwrap()).unwrap()
        );
    }
    for measure in ["Ships sunk", "Peak ships", "Length"] {
        for last_hour in 100..103 {
            let params = NavalBattlesRankingParams {
                branch_id: "1".into(),
                first_hour: 100,
                last_hour,
                top_n: 7.0,
                measure: measure.into(),
                countries:
                    dashboard_land_battles::products::land_battles::LandBattleCountryFilter {
                        tags: None,
                        membership: None,
                        unobserved: vec![],
                    },
            };
            let actual = prepared::query(
                &source,
                "naval-battles-ranking",
                serde_json::to_value(&params).unwrap(),
                &context,
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
    let params = details::NavalBattleDetailsParams {
        branch_id: "1".into(),
        battles: oracle
            .episodes
            .iter()
            .take(2)
            .map(|episode| details::NavalBattleDetailRequest {
                id: episode.id.clone(),
                hour: episode.runs[0].first,
            })
            .collect(),
    };
    let actual = prepared::query(
        &source,
        "naval-battle-details",
        serde_json::to_value(&params).unwrap(),
        &context,
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap();
    assert_eq!(
        actual,
        serde_json::to_value(NavalBattleDetails::load(&source, &params, &context, &flag).unwrap())
            .unwrap()
    );
}
