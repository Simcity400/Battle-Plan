use super::{PoliticsRun, civil_wars, history, state};
use dashboard_base::{Source, test_fixtures::{CAMPAIGN, snapshot_records, source}};
use replay::campaign::Replay;
use replay_data::rows::{
    CountryCivilWarRow, CountryCounterInputRow, CountryLinkRow, CountryRow, PoliticsRow, PowerBalanceMemberRow, PowerBalanceRow, Recorded, TableId, TableRows,
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
    let params = history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Elections, country: None };
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
        let params = history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Country, country: Some(tag.into()) };
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
    let params = history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Memberships, country: None };
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
    let params = history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Elections, country: None };
    let reply = history::answer(&query, &params, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.elections.len(), 1);
    let ger = &reply.elections[0];
    assert_eq!(ger.tags.iter().map(|held| (held.hour, held.tag.as_str())).collect::<Vec<_>>(), [(FIRST, "GER")]);
    assert_eq!(ger.held, [run(FIRST, LAST, 1)]);
    assert_eq!(ger.interval, [run(FIRST, LAST, 48)]);
    assert_eq!(ger.last, [run(FIRST, LAST, last_election())]);
}

#[test]
fn scripted_balance_names_expand_for_the_member_country_in_state_and_history() {
    let (catalogue, _folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        ("common/country_tags/synthetic.txt", "GER=\"countries/synthetic_ger.txt\""),
        ("common/defines/test.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }"),
        (
            "common/scripted_localisation/test.txt",
            "defined_text = { name = GetTestSide text = { trigger = { tag = GER } localization_key = SIDE_GER } text = { localization_key = SIDE_OTHER } }",
        ),
        (
            "localisation/test_l_english.yml",
            "\u{feff}l_english:\n GER_bop:0 \"[GetTestSide]\"\n GER_left:0 \"[GetTestSide]\"\n GER_right:0 \"Plain right\"\n SIDE_GER:0 \"Grand Council\"\n SIDE_OTHER:0 \"Other Council\"\n",
        ),
    ]);
    let catalogue = Arc::new(catalogue);
    catalogue.scripted_content().unwrap().set_synthetic_environment(Default::default());

    let mut one = frame(FIRST);
    for &table in dashboard_text::scripted_text::TABLES {
        if !matches!(table, TableId::Countries | TableId::CountryLinks | TableId::Politics | TableId::CountryCounterInputs | TableId::PowerBalances | TableId::PowerBalanceMembers | TableId::CountryCivilWars) {
            set(&mut one, table, TableRows::empty(table));
        }
    }
    set(
        &mut one,
        TableId::PowerBalances,
        TableRows::PowerBalances(vec![PowerBalanceRow {
            ordinal: 0,
            value_raw: 5_000,
            balance: b"GER_bop".to_vec(),
            left: b"GER_left".to_vec(),
            right: b"GER_right".to_vec(),
            trending: b"GER_left".to_vec(),
            ..Default::default()
        }]),
    );
    let params = state::PoliticsStateParams { branch_id: "1".into(), view: state::PoliticsView::Balances };
    let live = dashboard_follow::live::Cells::default();
    let state_reply = state::read(&one, &params, Some(Ok(catalogue.as_ref())), &live).unwrap();
    let record = &state_reply.balances[0];
    assert_eq!(record.name.as_deref(), Some("Grand Council"));
    assert_eq!(record.left.name.as_deref(), Some("Grand Council"));
    assert_eq!(record.right.name.as_deref(), Some("Plain right"));
    assert_eq!(record.trending.as_ref().and_then(|side| side.name.as_deref()), Some("Grand Council"));
    assert!(!state_reply.unavailable.iter().any(|entry| entry.label == "balance side names"));

    let frames: Vec<DecodedFrame> = (FIRST..=FIRST + 2)
        .map(|hour| {
            let mut held = one.clone();
            held.hour = hour;
            held
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[snapshot_records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "politics-scripted".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let history_params = history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Country, country: Some("GER".into()) };
    let history_reply = history::answer(&query, &history_params, Some(Ok(Arc::clone(&catalogue))), &AtomicBool::new(false)).unwrap();
    assert_eq!(history_reply.holder_balances[0].balances[0].name.as_deref(), Some("Grand Council"));
    assert_eq!(history_reply.names.get("GER_bop").map(String::as_str), Some("Grand Council"));
    assert_eq!(history_reply.names.get("GER_left").map(String::as_str), Some("Grand Council"));
}
