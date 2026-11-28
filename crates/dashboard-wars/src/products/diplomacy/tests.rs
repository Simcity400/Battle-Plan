use super::*;
use replay_data::rows::*;
use dashboard_follow::live::Cells;
use replay_frame::frame::{Metrics, TableView, not_selected};

fn frame(opinions: Vec<RelationOpinionRow>, reads: Vec<SliceReadRow>) -> DecodedFrame {
    let mut frame = DecodedFrame { branch: 1, hour: 500, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
    *frame.tables[TableId::RelationOpinions.index()] = TableView::Complete(TableRows::RelationOpinions(opinions));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
    frame
}

fn shown(pairs: &[OpinionPair]) -> Vec<(u32, u32, Option<i32>)> {
    pairs
        .iter()
        .map(|pair| match pair.cell {
            OpinionCell::Value { opinion } => (pair.row, pair.column, Some(opinion)),
            OpinionCell::Missing { .. } => (pair.row, pair.column, None),
        })
        .collect()
}

#[test]
fn opinions_among_the_chosen_name_unread_and_unheld_countries_as_unknown_never_as_zero() {
    let opinion = |owner_country, target_country, opinion| RelationOpinionRow { owner_country, target_country, opinion };
    let table = TableId::RelationOpinions.index() as u32;
    let frame = frame(
        vec![opinion(1, 2, -20), opinion(1, 9, 50), opinion(2, 1, 10), opinion(9, 1, 5)],
        vec![SliceReadRow { table, owner: 1, sample_hour: 480 }, SliceReadRow { table, owner: 3, sample_hour: 470 }, SliceReadRow { table: table + 1, owner: 4, sample_hour: 490 }],
    );
    let (owners, pairs, _) = opinions_matrix(&frame, &[("AAA", Some((1, 1))), ("BBB", Some((2, 2))), ("CCC", Some((3, 3))), ("DDD", Some((4, 4))), ("EEE", None)]);
    let known: Vec<_> = owners.iter().map(|owner| (owner.tag.as_str(), owner.known)).collect();
    assert_eq!(known, vec![("AAA", true), ("BBB", true), ("CCC", true), ("DDD", false), ("EEE", false)]);
    assert!(owners[3].reason.is_some() && owners[4].reason.is_some());
    assert_eq!((owners[0].read_hour, owners[2].read_hour), (Some(480), Some(470)));
    assert_eq!(
        shown(&pairs),
        vec![(0, 1, Some(-20)), (0, 4, None), (1, 0, Some(10)), (1, 4, None), (2, 4, None)],
        "only witnessed opinions and the unheld column are pairs; a country not chosen is no column"
    );
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
    let (owners, pairs, _) = opinions_matrix(&frame, &[("AAA", Some((10, 1))), ("BBB", Some((20, 2)))]);
    assert_eq!(owners.iter().map(|owner| (owner.tag.as_str(), owner.known, owner.country_id, owner.read_hour)).collect::<Vec<_>>(), vec![("AAA", true, Some(1), Some(490)), ("BBB", false, Some(2), None)]);
    assert_eq!(shown(&pairs), vec![(0, 1, Some(35))], "the pair's row under the alias ids is its opinion");
}

#[test]
fn opinions_matrix_lists_witnessed_pairs_only_and_a_displayed_only_domain() {
    let table = TableId::RelationOpinions.index() as u32;
    let frame = frame(
        vec![
            RelationOpinionRow { owner_country: 1, target_country: 2, opinion: -25 },
            RelationOpinionRow { owner_country: 2, target_country: 1, opinion: 40 },
            RelationOpinionRow { owner_country: 1, target_country: 9, opinion: 200 },
        ],
        vec![SliceReadRow { table, owner: 1, sample_hour: 480 }, SliceReadRow { table, owner: 2, sample_hour: 485 }],
    );
    let (owners, pairs, domain) = opinions_matrix(&frame, &[("AAA", Some((1, 1))), ("BBB", Some((2, 2))), ("CCC", Some((3, 3)))]);
    assert_eq!(domain, [-25, 40], "off-matrix country 9 opinion +200 never pollutes displayed domain");
    assert_eq!(shown(&pairs), vec![(0, 1, Some(-25)), (1, 0, Some(40))], "no diagonal and no cell per pair: AAA's opinion of CCC is its known row's zero");
    assert!(owners[0].known && !owners[2].known, "unread owner CCC's row is not recorded, never zero");
}

#[test]
fn a_truncated_opinions_read_keeps_each_witnessed_opinion_and_claims_no_zero() {
    let table = TableId::RelationOpinions.index() as u32;
    let mut frame = frame(Vec::new(), vec![SliceReadRow { table, owner: 1, sample_hour: 480 }]);
    *frame.tables[TableId::RelationOpinions.index()] = TableView::Truncated { rows: TableRows::RelationOpinions(vec![RelationOpinionRow { owner_country: 1, target_country: 2, opinion: 30 }]), missing: 2 };
    let (owners, pairs, _) = opinions_matrix(&frame, &[("AAA", Some((1, 1))), ("BBB", Some((2, 2)))]);
    assert_eq!(shown(&pairs), vec![(0, 1, Some(30))], "the witnessed opinion is shown, not refused with the whole matrix");
    assert!(owners.iter().all(|owner| !owner.known && owner.reason.is_some()), "an opinion not listed may be unread, so no row claims zero");
}

#[test]
fn a_group_is_its_members_at_the_asked_hour_and_unknown_membership_shows_no_owner() {
    use dashboard_query::{identity::HolderRun, series::ranked::{GroupKind, GroupMembership}};
    use dashboard_world::products::groups::{CountryGroupReply, MembershipRun, UnobservedHours};
    let table = TableId::RelationOpinions.index() as u32;
    let frame = frame(
        vec![RelationOpinionRow { owner_country: 1, target_country: 2, opinion: 30 }],
        vec![SliceReadRow { table, owner: 1, sample_hour: 480 }, SliceReadRow { table, owner: 2, sample_hour: 480 }],
    );
    let run = |tag: &str, first_hour, last_hour| MembershipRun { tag: tag.into(), first_hour, last_hour };
    let reply = CountryGroupReply {
        branch_id: "1".into(),
        humans: vec![run("AAA", 0, 1000), run("BBB", 600, 1000)],
        factions: Vec::new(),
        wars: Vec::new(),
        war_starts: Vec::new(),
        capitulations: Vec::new(),
        identity_changes: Vec::new(),
        country_identities: Vec::new(),
        unobserved: UnobservedHours { humans: vec![HourSpan { first_hour: 300, last_hour: 320 }], ..UnobservedHours::default() },
        warnings: Vec::new(),
    };
    let held = |slot: u32| vec![HolderRun { first: 0, last: 1000, holder: Holder::Found { slot, id: slot }, starts_segment: true }];
    let holders = Holders::new([("AAA".to_owned(), held(1)), ("BBB".to_owned(), held(2))].into_iter().collect(), Vec::new());
    let context = DiplomacyContext { looks: None, factions: Arc::new(Vec::new()), faction_edges: vec![650], warnings: Vec::new(), groups: GroupsAt::new(Arc::new(reply), 0) };
    let group = GroupSelection { kind: GroupKind::Humans, id: None, tag: None, membership: GroupMembership::Hour };
    let params = DiplomaticRelationsParams { branch_id: "1".into(), countries: Vec::new(), top: None, order: None, group: Some(group) };
    let live = Cells::default();
    let cancelled = AtomicBool::new(false);
    let shown = |hour: i32| {
        let at = At { owner: None, holders: &holders, hour, context: &context, cancelled: &cancelled, live: &live };
        let reply = DiplomaticRelations::reply(&frame, &params, &at).unwrap();
        (reply.owners.iter().map(|owner| owner.tag.clone()).collect::<Vec<_>>(), reply.chosen_missing)
    };
    let through = |hour| DiplomaticRelations::valid_through(&frame, &params, &At { owner: None, holders: &holders, hour, context: &context, cancelled: &cancelled, live: &live });
    assert_eq!(through(500), 599);
    assert_eq!(through(600), 649);
    assert_eq!(shown(500), (vec!["AAA".to_owned()], None), "a country joining at 600 is not shown at 500");
    assert_eq!(shown(700), (vec!["AAA".to_owned(), "BBB".to_owned()], None));
    let (none, refused) = shown(310);
    assert!(none.is_empty() && refused.is_some(), "unknown membership is never a guess or a union");
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
fn following_pacts_redates_active_rows_and_unread_spans_and_rereads_on_a_link_change() {
    use dashboard_base::test_fixtures::{CAMPAIGN, played, records, source};
    use replay_data::rows::{CountryLinkRow, CountryRow, LexicalTokenRow, RelationTupleRow, TableRows};

    let frames = (100..=108).map(|hour| {
        let mut frame = source(1, hour, Some(1));
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
            CountryRow { id: 9001, canonical_slot: 1, tag: b"AAA".to_vec(), ..Default::default() },
            CountryRow { id: 9002, canonical_slot: 2, tag: b"BBB".to_vec(), ..Default::default() },
            CountryRow { id: 9003, canonical_slot: 3, tag: b"CCC".to_vec(), ..Default::default() },
        ]));
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow { id: 9001, canonical_slot: 1 },
            CountryLinkRow { id: 9002, canonical_slot: 2 },
            CountryLinkRow { id: 9003, canonical_slot: 3 },
            CountryLinkRow { id: 9004, canonical_slot: if hour < 103 { 2 } else { 3 } },
        ]));
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow { token: 7, literal: b"guarantee".to_vec() },
        ]));
        *frame.tables[TableId::RelationTuples.index()] = if hour >= 106 {
            TableView::Unavailable { reason: "Agreements unread".into() }
        } else {
            TableView::Complete(TableRows::RelationTuples(vec![RelationTupleRow {
                owner_country: 9001, target_country: 9004, type_id: 7, first_country: 9001, second_country: 9004,
                start_hour: 90, ..Default::default()
            }]))
        };
        frame
    }).collect();
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = Arc::new(Source::from_replay(dashboard_base::SourceKey { source_id: "pact-steps".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &cancelled).unwrap());
    let open = crate::registrations::DIPLOMACY_DIPLOMATICPACTS.panel.expect("pacts follow playback").open;
    let panel = open(&source, &LoadContext { content: None }, &serde_json::json!({"branchId": "1", "country": "AAA"})).unwrap();
    let exact = panel.exact().unwrap();
    let steps = played(source.replay_ref(), exact, 100, 108);
    for step in &steps {
        let cold = (exact.cold)(step.hour, &cancelled).unwrap();
        assert_eq!(step.value.as_ref().as_ref().unwrap(), &cold, "pacts at {}", step.hour);
    }
    let at = |hour| steps.iter().find(|step| step.hour == hour).unwrap();
    assert_eq!(at(102).value.as_ref().as_ref().unwrap()["pacts"][0]["second"], "BBB");
    assert_eq!(at(103).value.as_ref().as_ref().unwrap()["pacts"][0]["second"], "CCC");
    assert_eq!(at(108).value.as_ref().as_ref().unwrap()["unobserved"][0]["lastHour"], 108);
    assert_eq!(steps.iter().filter(|step| step.hour > 100).map(|step| step.product_calls).sum::<u64>(), 2);
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
