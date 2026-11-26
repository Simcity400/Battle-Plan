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
    frame.metrics.table_ns[TableId::CountryCivilWars.index()] = 1;
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
        let params = history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Country, country: Some(tag.into()), hour: Some(LAST) };
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
fn a_countrys_balances_are_answered_at_the_hour_with_nothing_read_later_and_the_next_change() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let at = |tag: &str, hour: Option<i32>| {
        let params = history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Country, country: Some(tag.into()), hour };
        history::answer(&query, &params, None, &AtomicBool::new(false))
    };
    assert!(at("GER", None).is_err(), "a country's balance history names its hour");
    let ger = at("GER", Some(112)).unwrap();
    let balance = &ger.holder_balances[0].balances[0];
    assert_eq!(balance.values, [run(FIRST, 109, 5_000), run(110, 112, 6_000)], "nothing after the asked hour");
    assert_eq!(balance.range, [run(FIRST, 112, "GER_low")], "the range read at 115 is not known at 112");
    assert_eq!((ger.hour, ger.next_change), (Some(112), Some(115)));
    assert_eq!(ger.selected.as_deref(), Some("GER_bop"));
    assert_eq!(ger.known.iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>(), [(FIRST, 112)], "read coverage through the hour");
    assert_eq!(ger.unread, None);
    let eng = at("ENG", Some(119)).unwrap();
    assert!(eng.holder_balances.iter().all(|holder| holder.balances.is_empty()), "ENG is listed from 120");
    assert_eq!((eng.selected, eng.next_change), (None, Some(120)));
    assert_eq!(at("ENG", Some(126)).unwrap().selected.as_deref(), Some("GER_bop"), "the record listing it first, as the game shows it");
}

#[test]
fn a_records_change_is_the_games_own_arithmetic_over_every_member_and_a_missing_modifier_is_zero() {
    let mut frame = frame(125);
    set(
        &mut frame,
        TableId::PowerBalanceMembers,
        TableRows::PowerBalanceMembers(vec![
            PowerBalanceMemberRow { record: 0, ordinal: 0, country: GER, daily_raw: 1_000, daily_present: true, ..Default::default() },
            PowerBalanceMemberRow { record: 0, ordinal: 1, country: ENG, weekly_raw: -700, weekly_present: true, ..Default::default() },
        ]),
    );
    let reply = state::read(&frame, &state::PoliticsStateParams { branch_id: "1".into(), view: state::PoliticsView::Balances, country: None }, None, &Cells::default()).unwrap();
    let change = reply.balances[0].change.clone().unwrap();
    assert_eq!((change.daily.as_str(), change.weekly.as_str(), change.weekly_change.as_str()), ("1000", "-700", "6300"), "seven days of the daily drift plus the weekly");
    assert_eq!((change.shown, change.rate.as_str()), (state::ChangeShown::Daily, "900"), "the game shows the daily rate: daily plus a seventh of the weekly");
    set(&mut frame, TableId::PowerBalanceMembers, TableRows::PowerBalanceMembers(vec![PowerBalanceMemberRow { record: 0, ordinal: 0, country: GER, weekly_raw: 3, weekly_present: true, ..Default::default() }]));
    let reply = state::read(&frame, &state::PoliticsStateParams { branch_id: "1".into(), view: state::PoliticsView::Balances, country: None }, None, &Cells::default()).unwrap();
    let change = reply.balances[0].change.clone().unwrap();
    assert_eq!((change.shown, change.rate.as_str()), (state::ChangeShown::Weekly, "3"), "a daily rate the game rounds to zero shows the weekly change");
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
    let before = civil_wars::answer(&query, &civil_wars::CivilWarsParams { branch_id: "1".into(), hour: 104 }, &AtomicBool::new(false)).unwrap();
    assert_eq!((before.hour, before.next_change), (104, Some(105)));
    assert!(before.wars.is_empty(), "a war starting at 105 is not leaked at 104");
    assert_eq!(before.known.iter().map(|s| (s.first_hour, s.last_hour)).collect::<Vec<_>>(), [(FIRST, 104)]);

    let during = civil_wars::answer(&query, &civil_wars::CivilWarsParams { branch_id: "1".into(), hour: 110 }, &AtomicBool::new(false)).unwrap();
    assert_eq!((during.hour, during.next_change), (110, Some(116)), "next change is when the predicate stops at 116");
    assert_eq!(during.wars.len(), 1);
    assert_eq!((during.wars[0].first_hour, during.wars[0].last_hour), (105, 110), "cut at the asked hour");
    assert_eq!(during.wars[0].at_war.iter().map(|s| (s.first_hour, s.last_hour)).collect::<Vec<_>>(), [(105, 110)]);
    assert_eq!(during.wars[0].sides.iter().map(|s| (s.slot, s.last_hour)).collect::<Vec<_>>(), [(7, 110), (8, 110)]);

    let reply = civil_wars::answer(&query, &civil_wars::CivilWarsParams { branch_id: "1".into(), hour: LAST }, &AtomicBool::new(false)).unwrap();
    assert_eq!((reply.hour, reply.next_change), (LAST, None));
    assert_eq!(reply.wars.len(), 1);
    let war = &reply.wars[0];
    assert_eq!((war.first_hour, war.last_hour), (105, 120));
    assert_eq!(war.sides.len(), 2);
    assert_eq!(war.sides[0].slot, 7);
    assert_eq!(war.sides[0].tags[0].tag, "GER");
    assert!(war.sides.iter().all(|side| side.existed_before && side.parent.is_none() && side.exit.is_none()));
    assert_eq!(war.at_war.iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>(), [(105, 115)], "neither side exists after hour 115");
    assert!(war.outcome.is_none(), "no outcome without an annexation in the feed");
    assert!(reply.warnings.is_empty(), "the feed's annexations are read: {:?}", reply.warnings);
}

#[test]
fn two_civil_wars_join_only_from_the_hour_a_listing_connects_them() {
    let listing = |a, b, first, last| civil_wars::Listing { a, b, first, last };
    let evidence = civil_wars::Evidence {
        listings: vec![listing(1, 2, 100, 200), listing(3, 4, 100, 200), listing(2, 3, 110, 200)],
        known: vec![dashboard_base::span::HourSpan { first_hour: 100, last_hour: 200 }],
        changes: vec![100, 110, 201],
        ..Default::default()
    };
    let at = |hour| civil_wars::answer_at(&evidence, &civil_wars::CivilWarsParams { branch_id: "1".into(), hour }, 200);
    let before = at(105);
    assert_eq!(before.wars.len(), 2, "the listing that joins them is not read before 110");
    assert_eq!(before.next_change, Some(110));
    let after = at(110);
    assert_eq!(after.wars.len(), 1);
    assert_eq!(after.wars[0].sides.len(), 4);
}

#[test]
fn a_rebel_listed_with_its_parent_at_the_outbreak_keeps_the_parent_whatever_their_slot_order() {
    let mut tags = std::collections::BTreeMap::new();
    tags.insert(2, vec![super::HolderTag { hour: 100, tag: "REB".into() }]);
    tags.insert(9, vec![super::HolderTag { hour: 0, tag: "PAR".into() }]);
    let evidence = civil_wars::Evidence { listings: vec![civil_wars::Listing { a: 2, b: 9, first: 100, last: 200 }], tags, ..Default::default() };
    let reply = civil_wars::answer_at(&evidence, &civil_wars::CivilWarsParams { branch_id: "1".into(), hour: 150 }, 200);
    let sides = &reply.wars[0].sides;
    assert_eq!(sides.iter().map(|side| (side.slot, side.parent)).collect::<Vec<_>>(), [(2, Some(1)), (9, None)], "the rebel broke from the country that existed before");
}

#[test]
fn hours_the_civil_war_lists_were_not_read_are_not_known_and_say_why() {
    let frames = (FIRST..=LAST)
        .map(|hour| {
            let mut frame = frame(hour);
            if (122..=126).contains(&hour) {
                *frame.tables[TableId::CountryCivilWars.index()] = TableView::Unavailable { reason: "civil-war lists unreadable".into() };
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[snapshot_records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "politics-unread".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let reply = civil_wars::answer(&query, &civil_wars::CivilWarsParams { branch_id: "1".into(), hour: LAST }, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.known.iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>(), [(FIRST, 121), (127, LAST)]);
    assert!(reply.warnings.iter().any(|warning| warning.contains("civil-war lists unreadable")), "{:?}", reply.warnings);
    let early = civil_wars::answer(&query, &civil_wars::CivilWarsParams { branch_id: "1".into(), hour: 121 }, &AtomicBool::new(false)).unwrap();
    assert_eq!(early.next_change, Some(122), "the unread stretch is a change");
    assert!(early.warnings.is_empty(), "a reason dated later is not given earlier: {:?}", early.warnings);
}

fn source_of(id: &str, frames: Vec<DecodedFrame>) -> Source {
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[snapshot_records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: id.into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

#[test]
fn hours_a_complete_list_was_carried_but_not_sampled_are_not_known() {
    let frames = (FIRST..=LAST)
        .map(|hour| {
            let mut frame = frame(hour);
            if (123..=124).contains(&hour) {
                frame.metrics.table_ns[TableId::CountryCivilWars.index()] = 0;
            }
            frame
        })
        .collect();
    let source = source_of("politics-unsampled", frames);
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let reply = civil_wars::answer(&query, &civil_wars::CivilWarsParams { branch_id: "1".into(), hour: LAST }, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.known.iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>(), [(FIRST, 122), (125, LAST)]);
}

#[test]
fn a_listing_whose_target_is_named_only_later_keeps_its_refusal_from_the_hour_it_was_listed() {
    let frames = (FIRST..=LAST)
        .map(|hour| {
            let mut frame = frame(hour);
            if hour < 110 {
                set(&mut frame, TableId::Countries, TableRows::Countries(vec![CountryRow { id: GER, canonical_slot: 7, tag: b"GER".to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() }]));
                set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: 11, canonical_slot: 7 }, CountryLinkRow { id: GER, canonical_slot: 7 }]));
            }
            frame
        })
        .collect();
    let source = source_of("politics-late-target", frames);
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let early = civil_wars::answer(&query, &civil_wars::CivilWarsParams { branch_id: "1".into(), hour: 107 }, &AtomicBool::new(false)).unwrap();
    assert!(early.wars.is_empty());
    assert!(early.warnings.iter().any(|warning| warning.contains("refused civil-war listings")), "the listing at 105 names a country with no identity yet: {:?}", early.warnings);
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
        let (result, reads) = replay_recording::cells::count_reads(|| (crate::registrations::POLITICS_ELECTIONS_NEXTELECTIONS.load)(src, "lease", req, &load, &cancelled));
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
    let lost = (crate::registrations::POLITICS_ELECTIONS_NEXTELECTIONS.load)(
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
fn elections_follow_only_schedule_and_eligibility_changes_on_the_selected_branch() {
    use dashboard_base::products::at_hour::frame_panel;
    use dashboard_base::test_fixtures::{materialized_frame, played};
    use serde_json::json;

    let branches: Vec<_> = [1, 2].map(|branch| {
        let frames = (FIRST..=LAST).map(|hour| {
            let mut frame = frame(hour);
            frame.branch = branch;
            let mut politics = frame.complete::<replay_data::rows::tables::Politics>().unwrap().to_vec();
            politics[0].capital = hour;
            politics[0].election_interval = (48 + branch as i32 + i32::from(hour >= 108)).into();
            politics[1].ruling_party = Some(hour as u32);
            if (120..123).contains(&hour) {
                *frame.tables[TableId::Politics.index()] = TableView::Unavailable { reason: "damaged election schedule".into() };
            } else if (125..129).contains(&hour) {
                *frame.tables[TableId::Politics.index()] = TableView::Truncated { rows: TableRows::Politics(vec![politics.remove(0)]), missing: 1 };
            } else {
                set(&mut frame, TableId::Politics, TableRows::Politics(politics));
            }
            let mut counters = frame.complete::<replay_data::rows::tables::CountryCounterInputs>().unwrap().to_vec();
            if counters[0].owned_state_count > 0 { counters[0].owned_state_count += hour; }
            if matches!(hour, 125 | 126 | 128) { counters[0].owned_state_count = 1; }
            set(&mut frame, TableId::CountryCounterInputs, TableRows::CountryCounterInputs(counters));
            frame
        }).collect();
        let mut records = snapshot_records(branch, frames);
        for record in &mut records {
            if let replay_capture_spool::spool::Record::Frame(frame) = record && (125..129).contains(&frame.hour) { frame.keyframe = false; }
        }
        records
    }).into();
    let load = dashboard_base::products::LoadContext { content: None };
    for branch in [1, 2] {
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &branches[..branch as usize]).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Arc::new(Source::from_replay(dashboard_base::SourceKey { source_id: "elections".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &false.into()).unwrap());
        for mut params in [json!({}), json!({"countries":["GER"]})] {
            params["branchId"] = branch.to_string().into();
            let panel = (crate::registrations::POLITICS_ELECTIONS_NEXTELECTIONS.panel.unwrap().open)(&source, &load, &params).unwrap();
            let exact = panel.exact().unwrap();
            let mut whole = frame_panel::<super::elections::NextElections>(&source, &load, &params).unwrap();
            let fresh = |panel: &mut dyn dashboard_follow::panel::Panel, hour| materialized_frame(panel, source.replay_ref(), branch, hour);
            if branch == 1 {
                let mut previous = None;
                for step in played(source.replay_ref(), exact, FIRST, LAST) {
                    let expected = fresh(&mut *whole, step.hour);
                    assert_eq!(*step.value, expected, "hour {}", step.hour);
                    if let Some(prior) = &previous { assert_eq!(step.product_calls, u64::from(prior != &expected), "hour {}", step.hour); }
                    previous = Some(expected);
                }
            }
            for hour in [109, 103, 121] {
                let seek = (exact.advance)(hour, true, &false.into(), None);
                assert_eq!(*seek.value, fresh(&mut *whole, hour));
            }
        }
    }
}

#[test]
fn balance_scripted_names_respect_scopes_causality_staged_reads_and_named_refusals() {
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        ("common/country_tags/synthetic.txt", "GER=\"countries/ger.txt\" ENG=\"countries/eng.txt\""),
        ("common/defines/test.lua", "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 23 } }"),
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

    let history = history::answer(
        &query,
        &history::PoliticsHistoryParams { branch_id: "1".into(), mode: history::PoliticsHistoryMode::Country, country: Some("GER".into()), hour: Some(105) },
        Some(Ok(Arc::clone(&catalogue))),
        &cancelled,
    )
    .unwrap();
    assert_eq!(history.names.get("GER_bop").map(String::as_str), Some("Power Struggle"));
    assert!(!history.names.contains_key("GER_left"), "scripted side names must never enter plain token cache");
    assert!(!history.names.contains_key("GER_low"), "scripted range names must never enter plain token cache");
    let ger = &history.holder_balances[0].balances[0];
    assert_eq!(ger.left.as_ref().unwrap().name, None, "a scripted name is the at-hour state's, never a history's");
    assert!(ger.left.as_ref().unwrap().reason.is_some());

    let selected = |hour: i32| {
        let frame = make_frame(hour);
        let reply = state::read(&frame, &state::PoliticsStateParams { branch_id: "1".into(), view: state::PoliticsView::Balances, country: None }, Some(Ok(catalogue.as_ref())), &Cells::default()).unwrap();
        assert!(state::names_read_the_date(&frame, catalogue.as_ref()), "a scripted name bounds the reply at the next day");
        reply
    };
    let early = selected(105);
    assert_eq!(early.balances[0].left.name.as_deref(), Some("Fascist Council"));
    assert_eq!(early.balances[0].range.as_ref().unwrap().name.as_deref(), Some("GER low"));
    let late = selected(125);
    assert_eq!(late.balances[0].left.name.as_deref(), Some("Democratic Council"));

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

#[test]
fn next_elections_for_a_group_reads_only_its_members_at_the_hour_and_never_guesses_unknown_membership() {
    use super::elections::{NextElections, NextElectionsParams};
    use dashboard_base::{products::at_hour::{At, AtHour}, span::HourSpan};
    use dashboard_query::series::ranked::{GroupKind, GroupMembership, GroupSelection};
    use dashboard_world::products::{group_members::GroupsAt, groups::{CountryGroupReply, FactionGroup, MembershipRun, UnobservedHours}};
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let run = |tag: &str, first_hour, last_hour| MembershipRun { tag: tag.into(), first_hour, last_hour };
    let groups = GroupsAt::new(
        Arc::new(CountryGroupReply {
            branch_id: "1".into(),
            humans: Vec::new(),
            factions: vec![FactionGroup {
                id: "42".into(),
                name: None,
                name_reason: None,
                names: Vec::new(),
                recorded_name: String::new(),
                first_hour: FIRST,
                last_hour: LAST,
                members: vec![run("GER", FIRST, LAST), run("ENG", 108, LAST)],
                leaders: Vec::new(),
                incomplete: vec![HourSpan { first_hour: 125, last_hour: 126 }],
            }],
            wars: Vec::new(),
            war_starts: Vec::new(),
            capitulations: Vec::new(),
            identity_changes: Vec::new(),
            country_identities: Vec::new(),
            unobserved: UnobservedHours::default(),
            warnings: Vec::new(),
        }),
        FIRST,
    );
    let faction = GroupSelection { kind: GroupKind::Faction, id: Some("42".into()), tag: None, membership: GroupMembership::Hour };
    let params = NextElectionsParams { branch_id: "1".into(), countries: None, group: Some(faction) };
    assert!(NextElections::validate(&NextElectionsParams { countries: Some(Vec::new()), ..params.clone() }).is_err(), "a group is never sent with countries");
    let (live, cancelled) = (Cells::default(), AtomicBool::new(false));
    let at = |hour: i32| At { owner: None, holders: &query.holders, hour, context: &groups, cancelled: &cancelled, live: &live };
    let before = NextElections::reply(&frame(105), &params, &at(105)).unwrap();
    assert_eq!((before.rows.iter().map(|row| row.tag.as_str()).collect::<Vec<_>>(), before.unknown), (vec!["GER"], 0), "ENG's unread schedule is not counted before it joins");
    assert_eq!(NextElections::valid_through(&frame(105), &params, &at(105)), 107, "the reply holds until the hour before ENG joins");
    assert_eq!(NextElections::reply(&frame(110), &params, &at(110)).unwrap().unknown, 1, "a member's unread eligibility is counted once it is a member");
    let unknown = NextElections::reply(&frame(125), &params, &at(125)).unwrap();
    assert_eq!((unknown.rows.len(), unknown.refused.as_deref()), (0, Some("Membership of this group is not recorded at this hour")));
}
