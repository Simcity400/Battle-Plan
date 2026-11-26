use super::{Election, IdeologyInfo, LeaderRun, PartOwnership, PartySeries, RulingRun, read, stack_order};
use dashboard_base::{Source, test_fixtures::{CAMPAIGN, records, source}};
use replay::campaign::Replay;
use replay_data::rows::{CountryAppearanceRow, PoliticalPartyRow, PoliticsRow, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::{Arc, atomic::AtomicBool};

const FIRST: i32 = 100;
const LAST: i32 = 130;
const GER: u32 = 9001;

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    frame.tables[table.index()] = TableView::Complete(rows).into();
    frame.metrics.table_ns[table.index()] = 1;
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1_000));
    if hour == 115 {
        set(&mut frame, TableId::Countries, TableRows::Countries(Vec::new()));
    }
    let fascist = hour >= 110;
    let reused = hour >= 128;
    let party = |id: u32, token: &str, popularity: i64, ruling: bool| PoliticalPartyRow {
        country: GER,
        id,
        popularity,
        is_ruling: u8::from(ruling),
        token: token.as_bytes().to_vec(),
        tag: GER,
        name_default: 1,
        ..Default::default()
    };
    let parties = vec![party(1, if reused { "communism" } else { "democratic" }, if fascist { 3_000_000 } else { 6_000_000 }, !fascist), party(2, "fascism", if fascist { 7_000_000 } else { 4_000_000 }, fascist)];
    if (124..=125).contains(&hour) {
        frame.tables[TableId::PoliticalParties.index()] = TableView::Unavailable { reason: "not read".into() }.into();
    } else {
        set(&mut frame, TableId::PoliticalParties, TableRows::PoliticalParties(parties));
    }
    let look = CountryAppearanceRow { country: GER, leader_ideology: b"liberalism".to_vec(), leader_character: (hour >= 105).then_some(701), ..Default::default() };
    set(&mut frame, TableId::CountryAppearance, TableRows::CountryAppearance(vec![look]));
    let date = match hour {
        ..105 => 50,
        105..120 => 104,
        120..125 => 118,
        _ => 124,
    };
    set(&mut frame, TableId::Politics, TableRows::Politics(vec![PoliticsRow { country: GER, last_election: date, ..Default::default() }]));
    frame
}

fn recording() -> Source {
    let frames = (FIRST..=LAST).map(frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "parties".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

#[test]
fn a_countrys_parties_ruling_party_leader_and_elections_are_runs_that_never_bridge_a_gap() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let parties = read(&query, &"GER".into(), &AtomicBool::new(false)).unwrap();
    let owned = |part, first_hour, last_hour| PartOwnership { part, first_hour, last_hour };
    assert_eq!(parties.parties["democratic"].parts, [owned(1, 100, 114), owned(1, 116, 123), owned(1, 126, 127)], "a part is owned only over the hours it was read");
    assert_eq!(parties.parties["communism"].parts, [owned(1, 128, LAST)], "a reused part id belongs to its new token only from its first reading");
    assert_eq!(parties.parties["fascism"].parts, [owned(2, 100, 114), owned(2, 116, 123), owned(2, 126, LAST)]);
    let ruling = |token: &str, first, last, starts_segment| RulingRun { token: token.into(), first_hour: first, last_hour: last, starts_segment };
    assert_eq!(parties.ruling.runs, [ruling("democratic", 100, 109, true), ruling("fascism", 110, 114, false), ruling("fascism", 116, 123, true), ruling("fascism", 126, LAST, true)], "a run after a gap is no change of government");
    let leader = |person, first, last, starts_segment| LeaderRun { ideology: "liberalism".into(), person, first_hour: first, last_hour: last, starts_segment };
    assert_eq!(parties.leaders, [leader(None, 100, 104, true), leader(Some(0), 105, 114, false), leader(Some(0), 116, LAST, true)], "an unread party table does not discard the leader read beside it");
    assert_eq!(parties.people.len(), 1);
    assert_eq!(parties.people[0].character.id, 701);
    let election = |hour, witnessed| Election { hour, witnessed };
    assert_eq!(
        parties.elections,
        [election(104, 105), election(118, 120), election(124, 125)],
        "an election is dated from its recorded date and revealed when read, also while the party table is unread; the first date read is no election"
    );
    assert_eq!(parties.unobserved.iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>(), [(115, 115), (124, 125)]);
    assert_eq!(parties.ruling.name_runs.iter().map(|run| (run.first_hour, run.last_hour)).collect::<Vec<_>>(), [(100, 109), (110, 114), (116, 123), (126, LAST)]);
    assert!(parties.ruling.namings.iter().all(|(naming, _)| naming.key.is_none() && naming.tag.as_deref() == Some("GER")));
    let (_stack, recorded, load) = dashboard_base::test_fixtures::recorded_mod_defines(
        "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        "",
        &[],
    );
    let politics = super::politics_content(&recorded, load.content().unwrap()).unwrap();
    let params = dashboard_base::products::CountryParams { branch_id: "1".into(), country: "GER".into() };
    let reply = super::present(Ok(&politics), &params, "GER", &parties, &AtomicBool::new(false)).unwrap();
    assert!(reply.warnings.iter().any(|warning| warning == "party name: original tag reference of country appearance capture refused"));
}

#[test]
fn parties_stack_in_the_contents_ideology_order_whatever_rules() {
    let series = |token: &str| (token.to_owned(), PartySeries { token: token.into(), parts: Vec::new() });
    let held = [series("communism"), series("democratic"), series("fascism"), series("unlisted")].into_iter().collect();
    let ideology = |token: &str| IdeologyInfo { token: token.into(), name: None, color: None };
    let order = stack_order(&held, &[ideology("democratic"), ideology("fascism"), ideology("communism")]);
    assert_eq!(order.iter().map(|party| party.token.as_str()).collect::<Vec<_>>(), ["democratic", "fascism", "communism", "unlisted"]);
}
