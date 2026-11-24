use super::{PoliticsRun, civil_wars, history, state};
use dashboard_base::{Source, test_fixtures::{CAMPAIGN, snapshot_records, source}};
use dashboard_follow::live::Cells;
use replay::campaign::Replay;
use replay_data::rows::{
    CountryCivilWarRow, CountryCounterInputRow, CountryLinkRow, CountryRow, PoliticalPartyRow, PoliticsRow, PowerBalanceMemberRow, PowerBalanceRow, Recorded, TableId, TableRows,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::{Arc, atomic::AtomicBool};

const FIRST: i32 = 100;
const LAST: i32 = 130;
const GER: u32 = 9001;
const ENG: u32 = 9002;

fn last_election() -> i32 {
    game_model::calendar::HOUR_EPOCH + 1936 * 365 * 24
}

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    frame.tables[table.index()] = TableView::Complete(rows).into();
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1_000));
    let countries = vec![
        CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() },
        CountryRow { id: ENG, canonical_slot: 8, tag: b"ENG".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() },
    ];
    set(&mut frame, TableId::Countries, TableRows::Countries(countries));
    set(
        &mut frame,
        TableId::CountryLinks,
        TableRows::CountryLinks(vec![CountryLinkRow { id: 11, canonical_slot: 7 }, CountryLinkRow { id: GER, canonical_slot: 7 }, CountryLinkRow { id: ENG, canonical_slot: 8 }]),
    );
    set(
        &mut frame,
        TableId::Politics,
        TableRows::Politics(vec![
            PoliticsRow { country: GER, last_election: last_election(), has_elections: true.into(), election_interval: 48.into(), next_election_cache: None.into(), ..Default::default() },
            PoliticsRow { country: ENG, last_election: last_election(), has_elections: Recorded::NotRecorded, election_interval: 60.into(), next_election_cache: None.into(), ..Default::default() },
        ]),
    );
    set(
        &mut frame,
        TableId::CountryCounterInputs,
        TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow { country: GER, owned_state_count: if hour <= 115 { 5 } else { 0 }, ..Default::default() },
            CountryCounterInputRow { country: ENG, owned_state_count: if hour <= 112 { 3 } else { 0 }, ..Default::default() },
        ]),
    );
    let mut records = vec![PowerBalanceRow {
        ordinal: 0,
        value_raw: if hour < 110 { 5_000 } else { 6_000 },
        balance: b"GER_bop".to_vec(),
        active_range: if hour < 115 { b"GER_low".to_vec() } else { b"GER_high".to_vec() },
        trending: if hour < 110 { Vec::new() } else { b"GER_right".to_vec() },
        ..Default::default()
    }];
    let mut members = vec![PowerBalanceMemberRow { record: 0, ordinal: 0, country: GER, ..Default::default() }];
    if hour >= 120 {
        members.push(PowerBalanceMemberRow { record: 0, ordinal: 1, country: ENG, ..Default::default() });
    }
    if hour >= 125 {
        records.push(PowerBalanceRow { ordinal: 1, value_raw: 1_000, balance: b"ENG_bop".to_vec(), ..Default::default() });
        members.push(PowerBalanceMemberRow { record: 1, ordinal: 0, country: ENG, ..Default::default() });
    }
    set(&mut frame, TableId::PowerBalances, TableRows::PowerBalances(records));
    set(&mut frame, TableId::PowerBalanceMembers, TableRows::PowerBalanceMembers(members));
    let wars = if (105..=120).contains(&hour) { vec![CountryCivilWarRow { country: GER, ordinal: 0, target: ENG }] } else { Vec::new() };
    set(&mut frame, TableId::CountryCivilWars, TableRows::CountryCivilWars(wars));
    frame
}

fn recording() -> Source {
    let frames = (FIRST..=LAST).map(frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[snapshot_records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "politics".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn run(first: i32, last: i32, value: impl ToString) -> PoliticsRun {
    PoliticsRun { first_hour: first, last_hour: last, value: value.to_string() }
}

#[test]
fn an_election_schedule_is_held_by_an_owner_of_states_and_an_unread_eligibility_is_refused_not_denied() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let params = history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Elections, country: None, hour: None };
    let reply = history::answer(&query, &params, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.known.len(), 1);
    assert_eq!((reply.known[0].first_hour, reply.known[0].last_hour), (FIRST, LAST));
    let of = |tag: &str| reply.elections.iter().find(|entry| entry.tags.iter().any(|held| held.tag == tag)).unwrap();
    let ger = of("GER");
    assert_eq!(ger.held, [run(FIRST, 115, 1), run(116, LAST, 0)], "no elections once it owns no state");
    assert!(ger.refused.is_empty());
    assert_eq!(ger.interval, [run(FIRST, LAST, 48)]);
    let next = crate::products::country_people::election_date(last_election(), 48);
    assert_eq!(ger.next, [run(FIRST, LAST, next)]);
    assert_eq!(ger.last, [run(FIRST, LAST, last_election())]);
    let eng = of("ENG");
    assert!(eng.held.is_empty(), "an unrecorded eligibility is never shown as not held");
    assert_eq!(eng.refused, [run(FIRST, LAST, 1)]);
    assert_eq!(eng.interval, [run(FIRST, LAST, 60)]);
}

#[test]
fn a_holders_balances_follow_the_records_listing_it_with_their_ranges_and_leanings_and_the_game_shows_the_first() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let country = |tag: &str| {
        let params = history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Country, country: Some(tag.into()), hour: None };
        let reply = history::answer(&query, &params, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.holder_balances.len(), 1, "{tag} has one holder");
        reply.holder_balances[0].clone()
    };
    let ger = country("GER");
    assert_eq!(ger.holder, "7");
    assert_eq!(ger.balances.len(), 1);
    assert_eq!(ger.balances[0].token, "GER_bop");
    assert_eq!(ger.balances[0].values, [run(FIRST, 109, 5_000), run(110, LAST, 6_000)]);
    assert_eq!(ger.balances[0].record, [run(FIRST, LAST, 0)]);
    assert_eq!(ger.shown, [run(FIRST, LAST, "GER_bop")]);
    assert_eq!(ger.balances[0].range, [run(FIRST, 114, "GER_low"), run(115, LAST, "GER_high")]);
    assert_eq!(ger.balances[0].trending, [run(110, LAST, "GER_right")], "no side named is no trending run");
    let eng = country("ENG");
    assert_eq!(eng.holder, "8");
    let balance = |token: &str| eng.balances.iter().find(|balance| balance.token == token).unwrap();
    assert_eq!(balance("GER_bop").values, [run(120, LAST, 6_000)]);
    assert_eq!(balance("ENG_bop").values, [run(125, LAST, 1_000)]);
    assert_eq!(balance("ENG_bop").record, [run(125, LAST, 1)]);
    assert_eq!(balance("GER_bop").range, [run(120, LAST, "GER_high")], "a member's range runs only while it is listed");
    assert!(balance("ENG_bop").range.is_empty() && balance("ENG_bop").trending.is_empty());
    assert_eq!(eng.shown, [run(120, LAST, "GER_bop")], "the first record listing it, as the game shows it");
}

#[test]
fn memberships_list_every_holders_balances_by_the_records_listing_it_without_reading_their_values() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let params = history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Memberships, country: None, hour: None };
    let reply = history::answer(&query, &params, None, &AtomicBool::new(false)).unwrap();
    let holders: Vec<&str> = reply.holder_balances.iter().map(|holder| holder.holder.as_str()).collect();
    assert_eq!(holders, ["7", "8"], "every holder listed by a record");
    let eng = &reply.holder_balances[1];
    let balance = |token: &str| eng.balances.iter().find(|balance| balance.token == token).unwrap();
    assert_eq!(balance("GER_bop").record, [run(120, LAST, 0)]);
    assert_eq!(balance("ENG_bop").record, [run(125, LAST, 1)]);
    assert!(eng.balances.iter().all(|balance| balance.values.is_empty() && balance.range.is_empty() && balance.trending.is_empty()));
    assert_eq!(eng.shown, [run(120, LAST, "GER_bop")]);
}

#[test]
fn a_civil_war_is_an_episode_of_listings_ongoing_while_a_listed_country_exists() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let reply = civil_wars::answer(&query, &civil_wars::CivilWarsParams { branch_id: "1".into() }, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.wars.len(), 1);
    let war = &reply.wars[0];
    assert_eq!((war.first_hour, war.last_hour), (105, 120));
    assert_eq!(war.sides.len(), 2);
    assert_eq!(war.sides[0].tags[0].tag, "GER");
    assert!(war.sides.iter().all(|side| side.existed_before && side.parent.is_none() && side.exit.is_none()));
    assert_eq!(war.at_war.iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>(), [(105, 115)], "neither side exists after hour 115");
    assert!(war.outcome.is_none(), "no outcome without an annexation in the feed");
    assert!(reply.warnings.is_empty(), "the feed's annexations are read: {:?}", reply.warnings);
}

#[test]
fn listings_apart_in_time_are_two_episodes_and_a_third_side_breaks_from_the_one_listed_first() {
    let tags = std::collections::BTreeMap::new();
    let exists = std::collections::BTreeMap::new();
    let listing = |a, b, first, last| civil_wars::Listing { a, b, first, last };
    let wars = civil_wars::episodes(vec![listing(1, 2, 10, 20), listing(2, 3, 15, 30), listing(1, 2, 40, 50)], &tags, &exists, &[]);
    assert_eq!(wars.len(), 2);
    assert_eq!(wars[0].sides.len(), 3);
    assert_eq!(wars[0].sides[2].parent, Some(1), "side 3 broke from side 2, listed before it");
    assert!(wars[0].at_war.is_empty(), "no existence recorded: the war is not said to be ongoing");
    assert_eq!((wars[1].first_hour, wars[1].last_hour), (40, 50));
}

#[test]
fn a_holders_schedule_across_its_raw_ids_is_in_hour_order() {
    let id = |hour: i32| 20_000 + ((hour - FIRST) / 4) as u32;
    let frames = (FIRST..=LAST)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1_000));
            let country = id(hour);
            set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: country, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
            set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: country, canonical_slot: 7 }]));
            set(
                &mut frame,
                TableId::Politics,
                TableRows::Politics(vec![PoliticsRow { country, last_election: last_election(), has_elections: true.into(), election_interval: 48.into(), next_election_cache: None.into(), ..Default::default() }]),
            );
            set(&mut frame, TableId::CountryCounterInputs, TableRows::CountryCounterInputs(vec![CountryCounterInputRow { country, owned_state_count: 5, ..Default::default() }]));
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[snapshot_records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "politics-ids".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let params = history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Elections, country: None, hour: None };
    let reply = history::answer(&query, &params, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.elections.len(), 1);
    let ger = &reply.elections[0];
    assert_eq!(ger.tags.iter().map(|held| (held.hour, held.tag.as_str())).collect::<Vec<_>>(), [(FIRST, "GER")]);
    assert_eq!(ger.held, [run(FIRST, LAST, 1)]);
    assert_eq!(ger.interval, [run(FIRST, LAST, 48)]);
    assert_eq!(ger.last, [run(FIRST, LAST, last_election())]);
}

#[test]
fn next_elections_at_hour_is_bounded_by_asked_countries_and_independent_of_stretches() {
    let make = |stretches: i32| {
        let mut records = Vec::new();
        for s in 0..stretches {
            let hour = FIRST + s * 10;
            records.push(snapshot_records(s as u64 + 1, vec![frame(hour)]));
        }
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &records).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(dashboard_base::SourceKey { source_id: format!("elections-{stretches}"), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    };
    let one = make(1);
    let many = make(40);
    let _ = dashboard_base::queries::query(&one, 1).unwrap();
    let _ = dashboard_base::queries::query(&many, 1).unwrap();
    let load = dashboard_base::products::LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let ask = |src: &Source, params: serde_json::Value| {
        let mut req = params;
        req.as_object_mut().unwrap().insert("hour".into(), FIRST.into());
        let (result, reads) = replay_recording::cells::count_reads(|| (crate::registrations::POLITICS_ELECTIONS_NEXTELECTIONS.registered.load)(src, "lease", req, &load, &cancelled));
        (serde_json::from_str::<serde_json::Value>(&result.unwrap()).unwrap()["data"].clone(), reads)
    };
    let (all_one, reads_one) = ask(&one, serde_json::json!({ "branchId": "1" }));
    let (all_many, reads_many) = ask(&many, serde_json::json!({ "branchId": "1" }));
    assert_eq!(all_one, all_many, "reply at an hour does not grow with stretch count");
    assert_eq!(reads_one.get(&TableId::Politics), reads_many.get(&TableId::Politics), "Politics storage reads at an hour do not grow with stretch count");
    assert_eq!(reads_one.get(&TableId::CountryCounterInputs), reads_many.get(&TableId::CountryCounterInputs), "CountryCounterInputs storage reads at an hour do not grow with stretch count");
    assert_eq!(all_one["recorded"], true);
    assert_eq!(all_one["rows"].as_array().unwrap().len(), 1);
    assert_eq!(all_one["rows"][0]["tag"], "GER");
    assert_eq!(all_one["rows"][0]["interval"], 48);
    assert_eq!(all_one["rows"][0]["next"], crate::products::country_people::election_date(last_election(), 48));
    assert_eq!(all_one["rows"][0]["last"], last_election());
    assert_eq!(all_one["unknown"], 1, "ENG has unrecorded eligibility");

    let (only_ger, _) = ask(&many, serde_json::json!({ "branchId": "1", "countries": ["GER"] }));
    assert_eq!(only_ger["rows"].as_array().unwrap().len(), 1);
    assert_eq!(only_ger["rows"][0]["tag"], "GER");
    assert_eq!(only_ger["unknown"], 0, "unasked ENG is never read or counted");

    let (only_eng, _) = ask(&many, serde_json::json!({ "branchId": "1", "countries": ["ENG"] }));
    assert!(only_eng["rows"].as_array().unwrap().is_empty());
    assert_eq!(only_eng["unknown"], 1);

    let after_loss = recording();
    let lost = (crate::registrations::POLITICS_ELECTIONS_NEXTELECTIONS.registered.load)(
        &after_loss,
        "lease",
        serde_json::json!({ "branchId": "1", "hour": 116, "countries": ["GER"] }),
        &load,
        &cancelled,
    );
    let lost: serde_json::Value = serde_json::from_str::<serde_json::Value>(&lost.unwrap()).unwrap()["data"].clone();
    assert!(lost["rows"].as_array().unwrap().is_empty(), "GER owns no states at hour 116");
    assert_eq!(lost["unknown"], 0);
}

#[test]
fn balance_scripted_names_respect_scopes_causality_staged_reads_and_named_refusals() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        ("common/country_tags/synthetic.txt", "GER=\"countries/ger.txt\" ENG=\"countries/eng.txt\""),
        ("common/defines/test.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }"),
        ("common/units/synthetic.txt", "sub_units={syn_unit={}}"),
        ("common/ideologies/synthetic.txt", "ideologies = { democratic = {} fascism = {} }"),
        (
            "common/bop/synthetic.txt",
            concat!(
                "GER_bop = {\n",
                "  initial_value = 0.0\n",
                "  left_side = GER_left\n",
                "  right_side = GER_right\n",
                "  side = { id = GER_left range = { id = GER_low min = -1.0 max = 0.0 } }\n",
                "  side = { id = GER_right range = { id = GER_high min = 0.0 max = 1.0 } }\n",
                "}\n",
            ),
        ),
        (
            "common/ideologies/00_ideologies.txt",
            "ideologies = { democratic = { types = { liberalism = {} } } fascism = { types = { fascism_ideology = {} } } }\n",
        ),
        (
            "common/scripted_localisation/synthetic.txt",
            concat!(
                "defined_text = {\n",
                "  name = GetCouncil\n",
                "  text = { trigger = { ENG = { has_government = democratic } } localization_key = COUNCIL_DEM }\n",
                "  text = { localization_key = COUNCIL_ALT }\n",
                "}\n",
            ),
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " GER_bop:0 \"Power Struggle\"\n",
                " GER_left:0 \"[GetCouncil]\"\n",
                " GER_right:0 \"[ROOT.GetTag]\"\n",
                " GER_low:0 \"[ROOT.GetTag] low\"\n",
                " GER_high:0 \"[ROOT.GetTag] high\"\n",
                " COUNCIL_DEM:0 \"Democratic Council\"\n",
                " COUNCIL_ALT:0 \"Fascist Council\"\n",
            ),
        ),
    ]);
    let catalogue = Arc::new(dashboard_text::catalogue::tests::with_effect_describer(
        catalogue,
        dashboard_effects::effect_describer,
    ));
    let make_frame = |hour: i32| {
        let mut f = frame(hour);
        let ruling_eng = if hour < 115 { 1 } else { 0 };
        set(
            &mut f,
            TableId::CountryCounterInputs,
            TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow { country: GER, owned_state_count: 5, ..Default::default() },
                CountryCounterInputRow { country: ENG, owned_state_count: 3, ..Default::default() },
            ]),
        );
        set(
            &mut f,
            TableId::Politics,
            TableRows::Politics(vec![
                PoliticsRow { country: GER, ruling_party: Some(1), ..Default::default() },
                PoliticsRow { country: ENG, ruling_party: Some(ruling_eng), ..Default::default() },
            ]),
        );
        set(
            &mut f,
            TableId::PoliticalParties,
            TableRows::PoliticalParties(vec![
                PoliticalPartyRow { country: ENG, id: 0, token: b"democratic".to_vec(), is_ruling: u8::from(ruling_eng == 0), ..Default::default() },
                PoliticalPartyRow { country: ENG, id: 1, token: b"fascism".to_vec(), is_ruling: u8::from(ruling_eng == 1), ..Default::default() },
            ]),
        );
        set(
            &mut f,
            TableId::PowerBalances,
            TableRows::PowerBalances(vec![PowerBalanceRow {
                ordinal: 0,
                value_raw: if hour < 115 { -20_000 } else { 40_000 },
                balance: b"GER_bop".to_vec(),
                left: b"GER_left".to_vec(),
                right: b"GER_right".to_vec(),
                trending: b"MISSING_SIDE".to_vec(),
                active_range: if hour < 115 { b"GER_low".to_vec() } else { b"GER_high".to_vec() },
            }]),
        );
        f
    };
    let frames = (FIRST..=LAST).map(make_frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[snapshot_records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(
        dashboard_base::SourceKey { source_id: "bop-scripted".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let query = dashboard_base::queries::query(&src, 1).unwrap();
    let cancelled = AtomicBool::new(false);

    let without_hour = history::answer(
        &query,
        &history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Country, country: Some("GER".into()), hour: None },
        Some(Ok(Arc::clone(&catalogue))),
        &cancelled,
    )
    .unwrap();
    assert_eq!(without_hour.names.get("GER_bop").map(String::as_str), Some("Power Struggle"));
    assert!(!without_hour.names.contains_key("GER_left"), "scripted side names must never enter plain token cache");
    assert!(!without_hour.names.contains_key("GER_low"), "scripted range names must never enter plain token cache");
    let ger_no_hour = &without_hour.holder_balances[0].balances[0];
    assert_eq!(ger_no_hour.left.as_ref().unwrap().name, None);
    assert!(ger_no_hour.left.as_ref().unwrap().reason.is_some());

    let (early, early_reads) = replay_recording::cells::count_reads(|| {
        history::answer(
            &query,
            &history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Country, country: Some("GER".into()), hour: Some(105) },
            Some(Ok(Arc::clone(&catalogue))),
            &cancelled,
        )
        .unwrap()
    });
    assert!(!early_reads.contains_key(&TableId::Provinces), "staged reads must never touch Provinces");
    assert!(!early_reads.contains_key(&TableId::States), "staged reads must never touch States");
    assert!(!early.names.contains_key("GER_left"));
    let ger_early = &early.holder_balances[0].balances[0];
    assert_eq!(ger_early.left.as_ref().unwrap().name.as_deref(), Some("Fascist Council"), "left={:?}", ger_early.left);
    assert_eq!(ger_early.right.as_ref().unwrap().name.as_deref(), Some(""), "right={:?}", ger_early.right);
    assert_eq!(ger_early.ranges.iter().find(|r| r.token == "GER_low").unwrap().name.as_deref(), Some("GER low"), "ranges={:?}", ger_early.ranges);

    let late = history::answer(
        &query,
        &history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Country, country: Some("GER".into()), hour: Some(125) },
        Some(Ok(Arc::clone(&catalogue))),
        &cancelled,
    )
    .unwrap();
    let ger_late = &late.holder_balances[0].balances[0];
    assert_eq!(ger_late.left.as_ref().unwrap().name.as_deref(), Some("Democratic Council"));
    assert_eq!(ger_late.ranges.iter().find(|r| r.token == "GER_high").unwrap().name.as_deref(), Some("GER high"));

    let shared_frame = make_frame(125);
    let shared_reply = state::read(
        &shared_frame,
        &state::PoliticsStateParams { branch_id: "1".into(), view: state::PoliticsView::Balances, country: None },
        Some(Ok(catalogue.as_ref())),
        &Cells::default(),
    )
    .unwrap();
    let shared = &shared_reply.balances[0];
    assert_eq!(shared.left.name.as_deref(), Some("Democratic Council"));
    assert_eq!(shared.right.name.as_deref(), Some(""));
    assert_eq!(shared.trending.as_ref().unwrap().name, None);
    assert_eq!(shared.trending.as_ref().unwrap().reason.as_deref(), Some("no localisation in the recorded content"));
    assert_eq!(shared.range.as_ref().unwrap().name, None, "shared multi-member record without chosen country refuses scripted active range");
    assert!(shared.range.as_ref().unwrap().reason.as_deref().unwrap().contains("country event scope"));
}
