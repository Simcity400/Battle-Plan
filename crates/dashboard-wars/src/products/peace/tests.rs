use super::*;
use replay_data::rows::{SettledPeaceAction, SettledPeaceTransfer};

const A: Identity = (1, 11);
const B: Identity = (2, 22);
const C: Identity = (3, 33);

fn joins() -> Joins {
    let mut by: HashMap<Identity, Vec<(String, i32, i32)>> = HashMap::new();
    by.insert(A, vec![("AAA".into(), 0, 999)]);
    by.insert(B, vec![("BBB".into(), 0, 999)]);
    by.insert(C, vec![("CCC".into(), 150, 999)]);
    Joins {
        holders: HolderTags::from_runs(by),
        raw: HashMap::from([((11, 100), (A, "AAA".to_owned())), ((22, 100), (B, "BBB".to_owned()))]),
        tokens: HashMap::from([(7, "take_states".to_owned())]),
        action_key: Ok("take_states".into()),
        ..Joins::default()
    }
}

fn outcome(after: i32, before: i32, observation: u8, conference: Option<u64>, change: u8, state: u32, from: Identity, to: Identity) -> Outcome {
    Outcome { after, ordinal: state, observation, conference, before, change, group: 0, state, from, to, victory_points: Some(i64::from(state) * 10) }
}

fn span(conference: u64, compared: Option<i32>) -> PeaceConferenceSpanRow {
    PeaceConferenceSpanRow {
        conference,
        hour: 95,
        first_observed_hour: 80,
        last_observed_hour: 95,
        current_turn: 3,
        finalized: 1,
        first_finalized_hour: Some(95),
        first_absent_hour: Some(96),
        comparison: if compared.is_some() { PeaceConferenceSpanRow::COMPARED } else { PeaceConferenceSpanRow::WAITING },
        before_hour: compared.map(|_| 90),
        compared_hour: compared,
    }
}

#[test]
fn a_settlement_overrides_its_compared_conference_and_an_unsampled_change_is_a_card_of_its_own() {
    let outcomes = [
        outcome(100, 90, PeaceOutcomeRow::RECORDED, Some(7), PeaceOutcomeRow::TRANSFER, 5, A, B),
        outcome(200, 199, PeaceOutcomeRow::NOT_RECORDED, None, PeaceOutcomeRow::TRANSFER, 9, B, A),
    ];
    let settled = [Settled {
        publication_hour: 100,
        row: PeaceSettlementRow {
            sequence: 4,
            conference: Some(7),
            hour: Some(100),
            winners: vec![22],
            losers: vec![11],
            actions: vec![SettledPeaceAction { country: 22, token: 7, taker: 22, giver: 11, negotiator: 22, state: Some(5), ..Default::default() }],
            transfers: vec![SettledPeaceTransfer { state: 5, taker: 22, automatic: false }],
            ..Default::default()
        },
    }];
    let (deals, graph, warnings) = fold(&[span(7, Some(100))], &outcomes, &settled, &joins());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(deals.iter().map(|deal| deal.id.as_str()).collect::<Vec<_>>(), ["outcome:200:0", "conference:7:80"], "newest first");
    let unsampled = &deals[0];
    assert_eq!((unsampled.observation, unsampled.derived, unsampled.end_hour, unsampled.end_is_by), (PeaceObservation::ConferenceNotRecorded, true, Some(200), true));
    assert_eq!((unsampled.winners.as_slice(), unsampled.losers.as_slice()), (&["AAA".to_owned()][..], &["BBB".to_owned()][..]));
    let conference = &deals[1];
    assert!(!conference.derived && conference.captured.is_some());
    assert_eq!((conference.end_hour, conference.end_is_by, conference.winners.clone()), (Some(100), false, vec!["BBB".to_owned()]));
    assert_eq!(conference.items.len(), 1, "the capture replaces the derived transfer it witnessed");
    let item = &conference.items[0];
    assert!(!item.derived);
    match &item.change {
        PeaceChange::StateTransfer { state, from, to, victory_points, .. } => assert_eq!((*state, from.as_str(), to.as_str(), victory_points.as_deref()), (5, "AAA", "BBB", Some("50"))),
        other => panic!("{other:?}"),
    }
    let captured = conference.captured.as_ref().unwrap();
    assert_eq!((captured.actions[0].token_name.as_deref(), captured.actions[0].giver_tag.as_deref()), (Some("take_states"), Some("AAA")));
    assert_eq!(graph.links.iter().map(|link| link.state_count).sum::<u32>(), 2, "the campaign graph holds each transfer once");
}

#[test]
fn a_release_is_one_item_per_country_and_a_country_without_a_tag_is_left_out_by_name() {
    let outcomes = [
        Outcome { group: 3, ..outcome(160, 159, PeaceOutcomeRow::UNKNOWN, None, PeaceOutcomeRow::RELEASED, 4, A, C) },
        Outcome { group: 3, ..outcome(160, 159, PeaceOutcomeRow::UNKNOWN, None, PeaceOutcomeRow::RELEASED, 2, B, C) },
        Outcome { group: 3, ..outcome(160, 159, PeaceOutcomeRow::UNKNOWN, None, PeaceOutcomeRow::TRANSFER, 8, A, (9, 99)) },
    ];
    let (deals, graph, _) = fold(&[], &outcomes, &[], &joins());
    assert_eq!(deals.len(), 1);
    let deal = &deals[0];
    assert_eq!(deal.observation, PeaceObservation::ConferenceUnknown);
    assert_eq!(deal.items.len(), 1);
    match &deal.items[0].change {
        PeaceChange::CountryReleased { country, from, states, .. } => {
            assert_eq!((country.as_str(), from.as_slice(), states.as_slice()), ("CCC", &["AAA".to_owned(), "BBB".to_owned()][..], &[2, 4][..]));
        }
        other => panic!("{other:?}"),
    }
    assert!(deal.limitations.iter().any(|limitation| limitation.contains("{8}")), "the transfer to a holder without a tag is named, not dropped silently: {:?}", deal.limitations);
    assert_eq!(graph.links.iter().map(|link| link.state_count).sum::<u32>(), 2);
}

#[test]
fn a_conference_ends_at_its_last_read_when_the_next_read_misses_it_and_waits_with_its_reason() {
    let (deals, _, _) = fold(&[span(9, None)], &[], &[], &joins());
    let deal = &deals[0];
    assert_eq!((deal.end_hour, deal.end_is_by, deal.first_absent_hour), (Some(95), false, Some(96)));
    assert!(deal.limitations.iter().any(|limitation| limitation.starts_with("No complete ownership and war read after settlement")));
    let mut late = span(9, None);
    late.first_absent_hour = Some(120);
    let (deals, _, _) = fold(&[late], &[], &[], &joins());
    assert_eq!((deals[0].end_hour, deals[0].end_is_by), (Some(120), true), "an absence after unread hours bounds the end");
}

#[test]
fn a_conference_span_is_its_log_rows_cells_and_a_cell_not_recorded_leaves_it_out() {
    let columns = SPAN_COLUMNS.map(|name| dashboard_declare::field(TableId::PeaceConferenceSpans, name).expect("a span column"));
    let width = columns.iter().max().unwrap() + 1;
    let mut cells = vec![log::Cell::Missing; width];
    for (index, value) in [(0, 100), (1, 130), (2, 4), (3, 1), (4, 125), (6, 1), (8, 128)] {
        cells[columns[index]] = log::Cell::Number(value);
    }
    let row = log::Row { hour: 130, key: 77u128 << 32 | 130, facet: 0, cells };
    let span = conference_span(&row, &columns).expect("a whole row");
    assert_eq!((span.conference, span.hour, span.first_observed_hour, span.last_observed_hour), (77, 130, 100, 130));
    assert_eq!((span.first_finalized_hour, span.first_absent_hour, span.before_hour, span.compared_hour), (Some(125), None, None, Some(128)), "an optional cell without a value is none");
    let mut unrecorded = row;
    unrecorded.cells[columns[5]] = log::Cell::Unrecorded;
    assert_eq!(conference_span(&unrecorded, &columns), None, "a cell not recorded is never read as none");
}

#[test]
fn a_conference_listed_again_is_a_second_card_and_its_finalizer_joins_the_listing_it_ends() {
    let first = span(7, None);
    let moved = PeaceConferenceSpanRow { hour: 90, last_observed_hour: 90, ..span(7, None) };
    let again = PeaceConferenceSpanRow { hour: 310, first_observed_hour: 300, last_observed_hour: 310, first_finalized_hour: Some(310), first_absent_hour: Some(311), ..span(7, None) };
    let spans = listings(vec![moved, again.clone(), first.clone()]);
    assert_eq!(spans, vec![first, again], "the newest row of each listing, one per first observed hour");
    let settled = [Settled {
        publication_hour: 311,
        row: PeaceSettlementRow { sequence: 5, conference: Some(7), hour: Some(310), winners: vec![22], losers: vec![11], ..Default::default() },
    }];
    let (deals, _, warnings) = fold(&spans, &[], &settled, &joins());
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut ids: Vec<&str> = deals.iter().map(|deal| deal.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["conference:7:300", "conference:7:80"]);
    let ended = |id: &str| deals.iter().find(|deal| deal.id == id).map(|deal| deal.derived).unwrap();
    assert!(!ended("conference:7:300"), "the finalizer at hour 310 ends the second listing");
    assert!(ended("conference:7:80"), "the first listing keeps its own derived card");
}

#[test]
fn a_finalizer_without_a_conference_identity_warns_from_its_publication_not_from_its_own_hour() {
    let settled = [Settled {
        publication_hour: 311,
        row: PeaceSettlementRow { sequence: 5, conference: None, hour: Some(300), failure: Some(b"unreadable".to_vec()), ..Default::default() },
    }];
    let (_, _, warnings) = fold(&[], &[], &settled, &joins());
    assert_eq!(warnings.iter().map(|warning| warning.hour).collect::<Vec<_>>(), [Some(311)], "{warnings:?}");
}

#[test]
fn a_settlement_is_its_log_rows_cells_and_a_cell_not_recorded_leaves_it_out() {
    use replay_data::{rows::SettledPeaceSubject, value::Value as _};
    let columns = SETTLEMENT_COLUMNS.map(|name| dashboard_declare::field(TableId::PeaceSettlements, name).expect("a settlement column"));
    let expected = PeaceSettlementRow {
        sequence: 9,
        conference: Some(7),
        hour: Some(100),
        winners: vec![22, 23],
        losers: vec![11],
        actions: vec![SettledPeaceAction { country: 22, token: 7, taker: 22, giver: 11, negotiator: 22, state: Some(5), ..Default::default() }],
        transfers: vec![SettledPeaceTransfer { state: 5, taker: 22, automatic: false }],
        subjects: vec![SettledPeaceSubject { overlord: 22, subject: 30 }],
        failure: None,
        omitted: 0,
    };
    let encoded = |value: &dyn Fn(&mut Vec<u8>) -> std::io::Result<()>| {
        let mut bytes = Vec::new();
        value(&mut bytes).expect("encodes");
        log::Cell::List(bytes)
    };
    let list = |items: &[u32]| log::Cell::List(items.iter().flat_map(|item| item.to_le_bytes()).collect());
    let mut cells = vec![log::Cell::Missing; columns.iter().max().unwrap() + 1];
    cells[columns[0]] = log::Cell::Number(7);
    cells[columns[1]] = log::Cell::Number(100);
    cells[columns[2]] = list(&[22, 23]);
    cells[columns[3]] = list(&[11]);
    cells[columns[4]] = encoded(&|out| expected.actions.encode(out));
    cells[columns[5]] = encoded(&|out| expected.transfers.encode(out));
    cells[columns[6]] = encoded(&|out| expected.failure.encode(out));
    cells[columns[7]] = log::Cell::Number(0);
    cells[columns[8]] = encoded(&|out| expected.subjects.encode(out));
    let row = log::Row { hour: 100, key: 9, facet: 0, cells };
    assert_eq!(settlement(&row, &columns), Some(expected));
    let mut empty = row.clone();
    empty.cells[columns[4]] = log::Cell::List(Vec::new());
    empty.cells[columns[5]] = log::Cell::Missing;
    let emptied = settlement(&empty, &columns).expect("a settlement with empty lists");
    assert!(emptied.actions.is_empty() && emptied.transfers.is_empty(), "an empty list stored as no bytes is the game's empty list");
    let mut unrecorded = row;
    unrecorded.cells[columns[4]] = log::Cell::Unrecorded;
    assert_eq!(settlement(&unrecorded, &columns), None, "a cell not recorded is never read as an empty list");
}

#[test]
fn a_native_peace_action_is_named_by_the_games_demand_label_never_by_its_keyword() {
    let (cat, _folder) = dashboard_base::catalogue::tests::folder_catalogue(&[(
        "localisation/peace_l_english.yml",
        "\u{feff}l_english:\n PEACE_TAKE_STATES_DEMAND:0 \"Take State\"\n PEACE_LIBERATE_LABEL:0 \"Liberate\"\n PEACE_FORCE_GOVERNMENT_LABEL:0 \"Change government\"\n PEACE_PUPPET_COUNTRY_LABEL:0 \"Puppet $COUNTRY$\"\n",
    )]);
    let action = |token: u32, keyword: &str| PeaceCapturedAction {
        token_name: Some(keyword.to_owned()),
        cached_name: Some(keyword.as_bytes().to_vec()),
        ..captured_action(&SettledPeaceAction { country: 22, token, taker: 22, giver: 11, negotiator: 22, ..Default::default() }, &joins(), 100)
    };
    let puppet = |tag: Option<&str>| PeaceCapturedAction { country_operand: Some(33), country_operand_tag: tag.map(str::to_owned), ..action(10, "puppet") };
    let mut deal = card("deal".into(), PeaceObservation::ConferenceUnknown, 100);
    deal.captured = Some(PeaceCaptured {
        hour: 100,
        publication_hour: 100,
        winners: Vec::new(),
        losers: Vec::new(),
        actions: vec![action(7, "take_states"), action(8, "force_government"), action(9, "liberate"), puppet(Some("SLO")), puppet(None)],
        transfers: Vec::new(),
        subjects: Vec::new(),
    });
    let mut reply = PeaceDealsReply {
        branch_id: "1".into(),
        derived: false,
        deals: vec![deal],
        transfers: PeaceTransferGraph::default(),
        unobserved: PeaceUnobserved::default(),
        warnings: Vec::new(),
    };
    name_actions(&mut reply, &cat.localisation, &cat.recorded_language);
    let actions = &reply.deals[0].captured.as_ref().unwrap().actions;
    let names: Vec<(Option<&str>, Option<&str>)> = actions.iter().map(|action| (action.name.as_deref(), action.name_country.as_deref())).collect();
    assert_eq!(
        names,
        [(Some("Take State"), None), (Some("Change government"), None), (Some("Liberate"), None), (Some("Puppet {COUNTRY}"), Some("SLO")), (None, None)]
    );
    assert!(
        reply.warnings.iter().any(|warning| warning.contains("PEACE_PUPPET_COUNTRY_LABEL") && warning.contains("the country it puppets is not recorded")),
        "a puppet demand whose country is not recorded is refused by name: {:?}",
        reply.warnings
    );
}
