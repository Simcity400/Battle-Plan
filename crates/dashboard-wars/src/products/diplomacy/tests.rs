use super::*;
use replay_data::rows::*;
use dashboard_follow::live::{CellValue, Cells, GroupState};
use replay_frame::frame::{Metrics, TableView, not_selected};

fn frame(opinions: Vec<RelationOpinionRow>, reads: Vec<SliceReadRow>) -> DecodedFrame {
    let mut frame = DecodedFrame { branch: 1, hour: 500, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
    *frame.tables[TableId::RelationOpinions.index()] = TableView::Complete(TableRows::RelationOpinions(opinions));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
    frame
}

#[test]
fn opinions_among_the_chosen_name_unread_and_unheld_countries_as_unknown_never_as_zero() {
    let opinion = |owner_country, target_country, opinion| RelationOpinionRow { owner_country, target_country, opinion };
    let table = TableId::RelationOpinions.index() as u32;
    let frame = frame(
        vec![opinion(1, 2, -20), opinion(1, 9, 50), opinion(2, 1, 10), opinion(9, 1, 5)],
        vec![SliceReadRow { table, owner: 1, sample_hour: 480 }, SliceReadRow { table, owner: 3, sample_hour: 470 }, SliceReadRow { table: table + 1, owner: 4, sample_hour: 490 }],
    );
    let cells = Cells::default();
    let owners = opinions(&frame, &[("AAA", Some((1, 1))), ("BBB", Some((2, 2))), ("CCC", Some((3, 3))), ("DDD", Some((4, 4))), ("EEE", None)], &cells).unwrap();
    let shown: Vec<_> = owners.iter().map(|owner| (owner.tag.as_str(), owner.known)).collect();
    assert_eq!(shown, vec![("AAA", true), ("BBB", true), ("CCC", true), ("DDD", false), ("EEE", false)]);
    assert!(owners[3].reason.is_some() && owners[4].reason.is_some());
    let live = cells.finish();
    let values: Vec<CellValue> = live.values(&frame).into_iter().flatten().collect();
    let json = live.json(&live.values(&frame).into_iter().map(GroupState::Values).collect::<Vec<_>>());
    assert_eq!(json["rows"], serde_json::json!(["AAA", "BBB", "CCC"]), "unknown owners name no cell");
    let opinions_group = json["groups"].as_array().unwrap().iter().find(|group| group["table"] == TableId::RelationOpinions.index()).unwrap();
    assert!(opinions_group["keys"].as_array().unwrap().contains(&serde_json::json!((1u128 << 32 | 2).to_string())));
    assert!(!opinions_group["keys"].as_array().unwrap().contains(&serde_json::json!((1u128 << 32 | 9).to_string())), "a country not chosen is no column");
    assert!(values.contains(&CellValue::Number(-20)) && values.contains(&CellValue::Number(480)) && values.contains(&CellValue::Number(470)));
}

#[test]
fn an_opinion_listed_under_a_countrys_other_raw_ids_is_its_pair() {
    let table = TableId::RelationOpinions.index() as u32;
    let mut frame = frame(
        vec![RelationOpinionRow { owner_country: 7, target_country: 8, opinion: 35 }],
        vec![SliceReadRow { table, owner: 7, sample_hour: 490 }],
    );
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![link(1, 10), link(2, 20), link(7, 10), link(8, 20)]));
    let cells = Cells::default();
    let owners = opinions(&frame, &[("AAA", Some((10, 1))), ("BBB", Some((20, 2)))], &cells).unwrap();
    assert_eq!(owners.iter().map(|owner| (owner.tag.as_str(), owner.known, owner.country_id)).collect::<Vec<_>>(), vec![("AAA", true, Some(1)), ("BBB", false, Some(2))]);
    let live = cells.finish();
    let values: Vec<CellValue> = live.values(&frame).into_iter().flatten().collect();
    assert!(values.contains(&CellValue::Number(35)), "the pair's row under the alias ids is followed");
    assert!(values.contains(&CellValue::Number(490)), "the owner's read under its alias dates its opinions");
}

#[test]
fn opinions_matrix_computes_exact_cells_and_displayed_only_domain() {
    let table = TableId::RelationOpinions.index() as u32;
    let frame = frame(
        vec![
            RelationOpinionRow { owner_country: 1, target_country: 2, opinion: -25 },
            RelationOpinionRow { owner_country: 2, target_country: 1, opinion: 40 },
            RelationOpinionRow { owner_country: 1, target_country: 9, opinion: 200 },
        ],
        vec![
            SliceReadRow { table, owner: 1, sample_hour: 480 },
            SliceReadRow { table, owner: 2, sample_hour: 485 },
        ],
    );
    let (owners, cells, domain) = opinions_matrix(&frame, &[("AAA", Some((1, 1))), ("BBB", Some((2, 2))), ("CCC", Some((3, 3)))], &Cells::default()).unwrap();
    assert_eq!(domain, [-25, 40], "off-matrix country 9 opinion +200 never pollutes displayed domain");
    assert_eq!(owners[0].read_hour, Some(480));
    assert_eq!(cells[0], OpinionMatrixCell::Diagonal);
    assert_eq!(cells[1], OpinionMatrixCell::Value { opinion: -25 });
    assert_eq!(cells[2], OpinionMatrixCell::Value { opinion: 0 }, "known owner without pair row is exact zero");
    assert_eq!(cells[3], OpinionMatrixCell::Value { opinion: 40 });
    assert!(matches!(cells[6], OpinionMatrixCell::Missing { .. }), "unread owner CCC cells are missing, never zero");
}

#[test]
fn a_relations_runs_are_cut_where_its_endpoints_or_dates_change() {
    let step = |hour, cells: Option<[Cell; 4]>| lanes::Step { hour, cells: cells.map(|cells| Box::from(cells)) };
    let n = Cell::Number;
    let steps = vec![
        step(100, Some([n(1), n(2), n(90), n(0)])),
        step(130, Some([n(1), n(2), n(90), n(400)])),
        step(150, None),
        step(160, Some([n(1), n(2), n(160), n(0)])),
        step(170, Some([n(1), n(2), n(160), n(0)])),
        step(180, Some([n(1), n(2), Cell::Unrecorded, n(0)])),
        step(190, Some([n(1), n(2), n(160), n(0)])),
    ];
    assert_eq!(
        pact_runs(&steps, 199),
        vec![
            PactRun { first: 100, last: 129, endpoints: (1, 2), dates: (90, 0) },
            PactRun { first: 130, last: 149, endpoints: (1, 2), dates: (90, 400) },
            PactRun { first: 160, last: 179, endpoints: (1, 2), dates: (160, 0) },
            PactRun { first: 190, last: 199, endpoints: (1, 2), dates: (160, 0) },
        ]
    );
}

#[test]
fn a_countrys_pacts_end_when_its_row_goes_and_are_unknown_past_a_gap_and_join_across_a_load() {
    let key = |owner: u32, target: u32, kind: u32| u128::from(owner) << 80 | u128::from(target) << 48 | u128::from(kind) << 16;
    let run = |first, last, endpoints, dates| PactRun { first, last, endpoints, dates };
    let first = RunPacts {
        rows: vec![
            (key(1, 2, 7), vec![run(100, 149, (1, 2), (50, 0))]),
            (key(1, 2, 8), vec![run(120, 199, (1, 2), (120, 900))]),
            (key(1, 3, 9), vec![run(100, 199, (3, 1), (60, 0))]),
            (key(2, 1, 7), vec![run(100, 149, (1, 2), (50, 0))]),
        ],
        kinds: HashMap::from([(7, "guarantee".to_owned()), (8, "non_aggression_pact".to_owned()), (9, "military_access".to_owned())]),
        unproven: Vec::new(),
    };
    let second = RunPacts { rows: vec![(key(1, 2, 8), vec![run(200, 299, (1, 2), (120, 900))])], kinds: HashMap::from([(8, "non_aggression_pact".to_owned())]), unproven: Vec::new() };
    let links = |first: i32, last: i32| RunLinks { slots: (1..=3).map(|raw| (raw, vec![(first, last, Some(i64::from(raw) * 10))])).collect() };
    let held = HashMap::from([(0, (Arc::new(first), Arc::new(links(100, 199)))), (1, (Arc::new(second), Arc::new(links(200, 299))))]);
    let names = Names { slots: HashMap::from([(10, vec![("AAA".to_owned(), 100, 299)]), (20, vec![("BBB".to_owned(), 100, 299)]), (30, vec![("CCC".to_owned(), 100, 299)])]) };
    let stretches = [Stretch { run: 0, first: 100, last: 199, join: Join::Start }, Stretch { run: 1, first: 200, last: 299, join: Join::Load { parent: 199 } }];
    let (pacts, counts) = pacts(&stretches, &held, &names, &[(1, 100, 299)], 299);
    let shown: Vec<_> = pacts
        .iter()
        .map(|pact| (pact.kind.as_str(), pact.first.as_deref(), pact.second.as_deref(), pact.start_hour, pact.end_hour, pact.first_seen, pact.last_seen, pact.gone_by, pact.held, pact.ended_by))
        .collect();
    assert_eq!(
        shown,
        vec![
            ("guarantee", Some("AAA"), Some("BBB"), Some(50), None, 100, 149, Some(150), false, false),
            ("military_access", Some("CCC"), Some("AAA"), Some(60), None, 100, 199, None, false, false),
            ("non_aggression_pact", Some("AAA"), Some("BBB"), Some(120), Some(900), 120, 299, None, true, false),
        ]
    );
    assert_eq!(counts.values().sum::<u32>(), 3);
    let broken = [Stretch { run: 0, first: 100, last: 199, join: Join::Start }, Stretch { run: 1, first: 250, last: 299, join: Join::Break }];
    let (pacts, _) = super::pacts(&broken, &held, &names, &[(1, 100, 299)], 299);
    assert!(pacts.iter().any(|pact| pact.kind == "military_access" && pact.ended_by && pact.gone_by.is_none()));
}

#[test]
fn a_reply_asked_at_an_hour_holds_no_pact_first_seen_after_it_and_names_and_ends_pacts_only_from_what_it_saw() {
    let key = |owner: u32, target: u32, kind: u32| u128::from(owner) << 80 | u128::from(target) << 48 | u128::from(kind) << 16;
    let run = |first, last, endpoints, dates| PactRun { first, last, endpoints, dates };
    let rows = RunPacts {
        rows: vec![(key(1, 2, 7), vec![run(100, 149, (1, 2), (50, 0))]), (key(1, 2, 8), vec![run(120, 199, (1, 2), (120, 900))])],
        kinds: HashMap::from([(7, "guarantee".to_owned()), (8, "non_aggression_pact".to_owned())]),
        unproven: Vec::new(),
    };
    let links = RunLinks { slots: (1..=2).map(|raw| (raw, vec![(100, 199, Some(i64::from(raw) * 10))])).collect() };
    let held = HashMap::from([(0, (Arc::new(rows), Arc::new(links)))]);
    let names = Names { slots: HashMap::from([(10, vec![("AAA".to_owned(), 100, 199)]), (20, vec![("BBB".to_owned(), 100, 159), ("NEW".to_owned(), 160, 199)])]) };
    let stretches = [Stretch { run: 0, first: 100, last: 199, join: Join::Start }];
    let at = |hour: i32| {
        let (pacts, counts) = pacts(&stretches, &held, &names, &[(1, 100, 199)], hour);
        let shown: Vec<_> = pacts.iter().map(|pact| (pact.kind.clone(), pact.second.clone(), pact.last_seen, pact.gone_by, pact.held)).collect();
        (shown, counts.values().sum::<u32>())
    };
    let pair = |kind: &str, second: &str, last: i32, gone: Option<i32>, held: bool| (kind.to_owned(), Some(second.to_owned()), last, gone, held);
    assert_eq!(at(119), (vec![pair("guarantee", "BBB", 119, None, true)], 1), "a pact first seen at 120 is not in the reply asked at 119, nor in its counts");
    assert_eq!(at(149), (vec![pair("guarantee", "BBB", 149, None, true), pair("non_aggression_pact", "BBB", 149, None, true)], 2), "held at the asked hour: its end is not known yet");
    assert_eq!(
        at(170),
        (vec![pair("guarantee", "BBB", 149, Some(150), false), pair("non_aggression_pact", "NEW", 170, None, true)], 2),
        "an endpoint is named by its tag at the last hour the reply saw, never a later one"
    );
    assert_eq!(at(155).0[1].1.as_deref(), Some("BBB"));
}

#[test]
fn a_row_hidden_by_an_unavailable_relation_table_is_not_gone_and_its_hours_are_unobserved() {
    let key = |owner: u32, target: u32, kind: u32| u128::from(owner) << 80 | u128::from(target) << 48 | u128::from(kind) << 16;
    let rows = RunPacts {
        rows: vec![(key(1, 2, 7), vec![PactRun { first: 100, last: 109, endpoints: (1, 2), dates: (50, 0) }])],
        kinds: HashMap::from([(7, "guarantee".to_owned())]),
        unproven: vec![(110, 120), (150, 159)],
    };
    let links = RunLinks { slots: (1..=2).map(|raw| (raw, vec![(100, 199, Some(i64::from(raw) * 10))])).collect() };
    let held = HashMap::from([(0, (Arc::new(rows), Arc::new(links)))]);
    let names = Names { slots: HashMap::from([(10, vec![("AAA".to_owned(), 100, 199)]), (20, vec![("BBB".to_owned(), 100, 199)])]) };
    let stretches = [Stretch { run: 0, first: 100, last: 155, join: Join::Start }];
    let (pacts, _) = pacts(&stretches, &held, &names, &[(1, 100, 199)], 155);
    assert_eq!(pacts.iter().map(|pact| (pact.last_seen, pact.gone_by)).collect::<Vec<_>>(), vec![(109, None)]);
    assert_eq!(unproven(&stretches, &held), vec![HourSpan { first_hour: 110, last_hour: 120 }, HourSpan { first_hour: 150, last_hour: 155 }]);
}

#[test]
fn a_relation_kind_the_content_cannot_title_keeps_its_id_with_why_and_its_count() {
    let counts = BTreeMap::from([("modded_relation".to_owned(), 3)]);
    let tokens = HashMap::from([("modded_relation", 987_654)]);
    let (kinds, warnings) = kind_titles(counts, &tokens, Ok(None));
    assert_eq!(kinds.len(), 1);
    let kind = &kinds[0];
    assert_eq!((kind.kind.as_str(), kind.count, kind.name.as_deref()), ("modded_relation", 3, None));
    assert!(kind.reason.as_deref().is_some_and(|reason| reason.contains("modded_relation") && reason.contains("count: 3")));
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("3 occurrences"));
}

#[test]
fn a_countrys_pacts_include_those_other_countries_rows_name_it_in_once_each() {
    let key = |owner: u32, target: u32, kind: u32| u128::from(owner) << 80 | u128::from(target) << 48 | u128::from(kind) << 16;
    let run = |first, last, endpoints, dates| PactRun { first, last, endpoints, dates };
    let rows = RunPacts {
        rows: vec![
            (key(2, 1, 5), vec![run(100, 199, (2, 1), (40, 0))]),
            (key(3, 4, 5), vec![run(100, 199, (3, 4), (40, 0))]),
            (key(1, 2, 8), vec![run(100, 199, (1, 2), (60, 0))]),
            (key(2, 1, 8), vec![run(100, 199, (1, 2), (60, 0))]),
        ],
        kinds: HashMap::from([(5, "market_access_rights".to_owned()), (8, "non_aggression_pact".to_owned())]),
        unproven: Vec::new(),
    };
    let links = RunLinks { slots: (1..=4).map(|raw| (raw, vec![(100, 199, Some(i64::from(raw) * 10))])).collect() };
    let held = HashMap::from([(0, (Arc::new(rows), Arc::new(links)))]);
    let names = Names { slots: (1..=4).map(|raw| (raw * 10, vec![(format!("T{raw:02}"), 100, 199)])).collect() };
    let stretches = [Stretch { run: 0, first: 100, last: 199, join: Join::Start }];
    let (pacts, _) = pacts(&stretches, &held, &names, &[(1, 100, 199)], 199);
    let shown: Vec<_> = pacts.iter().map(|pact| (pact.kind.as_str(), pact.first.as_deref(), pact.second.as_deref())).collect();
    assert_eq!(shown, vec![("market_access_rights", Some("T02"), Some("T01")), ("non_aggression_pact", Some("T01"), Some("T02"))]);
}

#[test]
fn a_country_chosen_by_slot_is_named_by_the_tag_that_holds_it() {
    use dashboard_query::identity::{Holder, HolderRun};
    let holders = dashboard_query::identity::Holders::new(
        [("GER", 1), ("ENG", 2)].iter().map(|(tag, id)| ((*tag).to_owned(), vec![HolderRun { first: 0, last: 1000, holder: Holder::Found { slot: *id, id: *id }, starts_segment: true }])).collect(),
        vec![],
    );
    let ids = holder_ids(&holders, &[Country::Slot(2)], 500);
    assert_eq!(ids[0].0, "ENG", "a country chosen by slot is named by the tag that holds it");
}

#[test]
fn the_largest_countries_are_named_only_from_a_whole_factories_read() {
    use dashboard_query::identity::{Holder, HolderRun};
    let holders = dashboard_query::identity::Holders::new(
        [("GER", 1), ("ENG", 2)].iter().map(|(tag, id)| ((*tag).to_owned(), vec![HolderRun { first: 0, last: 1000, holder: Holder::Found { slot: *id, id: *id }, starts_segment: true }])).collect(),
        vec![],
    );
    let factories = |rows: Vec<CountryFactoriesRow>, whole: bool| {
        let mut held = frame(Vec::new(), Vec::new());
        let rows = TableRows::CountryFactories(rows);
        *held.tables[TableId::CountryFactories.index()] = if whole { TableView::Complete(rows) } else { TableView::Truncated { rows, missing: 1 } };
        held
    };
    let row = |country: u32, military_levels: i32| CountryFactoriesRow { country, military_levels, ..CountryFactoriesRow::default() };
    assert_eq!(top_countries_at(&factories(vec![row(1, 5), row(2, 9)], true), &holders, 500, 1), Ok(vec![Country::from("ENG")]));
    assert!(top_countries_at(&factories(vec![row(1, 5)], false), &holders, 500, 1).is_err(), "a truncated read never ranks the rest at zero");
    assert!(top_countries_at(&frame(Vec::new(), Vec::new()), &holders, 500, 1).is_err(), "an unread table never picks countries alphabetically");
    assert!(top_countries_at(&factories(vec![row(1, 5), row(7, 99)], true), &holders, 500, 1).is_err(), "a row of an unknown country could be the largest");
}
