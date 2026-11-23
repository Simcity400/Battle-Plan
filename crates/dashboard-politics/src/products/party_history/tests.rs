use super::{LeaderRun, RulingRun, read};
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
    let parties = vec![party(1, "democratic", if fascist { 3_000_000 } else { 6_000_000 }, !fascist), party(2, "fascism", if fascist { 7_000_000 } else { 4_000_000 }, fascist)];
    set(&mut frame, TableId::PoliticalParties, TableRows::PoliticalParties(parties));
    let look = CountryAppearanceRow { country: GER, leader_ideology: b"liberalism".to_vec(), leader_character: (hour >= 105).then_some(701), ..Default::default() };
    set(&mut frame, TableId::CountryAppearance, TableRows::CountryAppearance(vec![look]));
    let date = match hour {
        ..105 => 1_000,
        105..120 => 2_000,
        _ => 3_000,
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
    assert_eq!(parties.parties["democratic"].parts, [1]);
    assert_eq!(parties.parties["fascism"].parts, [2]);
    let ruling = |token: &str, first, last, starts_segment| RulingRun { token: token.into(), first_hour: first, last_hour: last, starts_segment };
    assert_eq!(parties.ruling, [ruling("democratic", 100, 109, true), ruling("fascism", 110, 114, false), ruling("fascism", 116, LAST, true)], "a run after a gap is no change of government");
    let leader = |person, first, last, starts_segment| LeaderRun { ideology: "liberalism".into(), person, first_hour: first, last_hour: last, starts_segment };
    assert_eq!(parties.leaders, [leader(None, 100, 104, true), leader(Some(0), 105, 114, false), leader(Some(0), 116, LAST, true)]);
    assert_eq!(parties.people.len(), 1);
    assert_eq!(parties.people[0].character.id, 701);
    assert_eq!(parties.elections, [105, 120], "the first date read is the recording's start, not an election; a gap under the same holder still dates one");
    assert_eq!(parties.unobserved.iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>(), [(115, 115)]);
    assert_eq!(parties.party_name_runs.iter().map(|run| (run.first_hour, run.last_hour)).collect::<Vec<_>>(), [(100, 109), (110, 114), (116, LAST)]);
    assert!(parties.namings.iter().all(|(naming, _)| naming.key.is_none() && naming.tag.as_deref() == Some("GER")));
}
