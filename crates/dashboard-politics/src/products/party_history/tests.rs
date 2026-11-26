use super::{Election, IdeologyInfo, LeaderRun, PartOwnership, PartySeries, RulingRun, read, stack_order};
use dashboard_base::{products::Product, test_fixtures::{source, source_with_mods, visual_mods::Machine}};
use replay_data::rows::{CountryAppearanceRow, PoliticalPartyRow, PoliticsRow, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::atomic::AtomicBool;

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
    set(&mut frame, TableId::Characters, TableRows::Characters(vec![replay_data::rows::CharacterRow {
        id: 701, name_token_id: -1, token: b"TEST_LEADER_NAME".to_vec(), ..Default::default()
    }]));
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

#[test]
fn a_countrys_parties_ruling_party_leader_and_elections_are_runs_that_never_bridge_a_gap() {
    let machine = Machine::new("ruling-party-marker-names");
    machine.write("common/defines/base.lua", b"NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}");
    machine.write_mod("probe", "localisation/test_l_english.yml", "\u{feff}l_english:\n TEST_LEADER_NAME:0 \"A\u{a4}B\"\n".as_bytes());
    let (source, load) = source_with_mods((FIRST..=LAST).map(frame).collect(), &machine.root, &["mod/probe.mod"]);
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let parties = read(&query, &"GER".into(), super::PartyRead::Full, &AtomicBool::new(false)).unwrap();
    let ruling_only = read(&query, &"GER".into(), super::PartyRead::Ruling, &AtomicBool::new(false)).unwrap();
    assert_eq!(parties.ruling.runs, ruling_only.ruling.runs);
    assert_eq!(parties.ruling.name_runs, ruling_only.ruling.name_runs);
    assert_eq!(parties.unobserved, ruling_only.unobserved);
    assert!(ruling_only.leaders.is_empty() && ruling_only.people.is_empty() && ruling_only.elections.is_empty());
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
    let politics = super::politics_content(&source, load.content().unwrap()).unwrap();
    let params = dashboard_base::products::CountryParams { branch_id: "1".into(), country: "GER".into() };
    let narrow = super::present_ruling(Ok(&politics), "GER", &ruling_only);
    let reply = super::present(Ok(&politics), &params, "GER", &parties, &AtomicBool::new(false)).unwrap();
    let leader_warning = "1 texts are not shown: localisation TEST_LEADER_NAME holds the control byte 0x14 (an authored `\u{a4}`), which plain text cannot show; TEST_LEADER_NAME";
    let (leader, marker): (Vec<_>, Vec<_>) = reply.warnings.iter().map(String::as_str).partition(|warning| *warning == leader_warning);
    assert_eq!(leader, [leader_warning]);
    assert_eq!(narrow.warnings, marker);
    assert_eq!(narrow.runs, reply.ruling);
    assert_eq!(narrow.unobserved, reply.unobserved);
    assert_eq!(narrow.ideologies, reply.ideologies);
    assert!(narrow.warnings.iter().any(|warning| warning == "party name: original tag reference of country appearance capture refused"));
    let marker_params = crate::products::stability_markers::StabilityMarkersParams { branch_id: "1".into(), country: "GER".into(), hour: LAST, focus: false, government: true, wars: false, factions: false };
    let markers = {
        let mut own = dashboard_text::catalogue::OwnRefusals::default();
        dashboard_text::catalogue::own_refusals(&mut own, || crate::products::stability_markers::StabilityMarkers::load(&source, &marker_params, &load, &AtomicBool::new(false))).unwrap()
    };
    assert!(markers.markers.iter().any(|marker| marker.id.starts_with("party:")));
    assert!(markers.warnings.iter().any(|warning| warning == "party name: original tag reference of country appearance capture refused"));
    assert!(!markers.warnings.iter().any(|warning| warning == leader_warning));
}

#[test]
fn parties_stack_in_the_contents_ideology_order_whatever_rules() {
    let series = |token: &str| (token.to_owned(), PartySeries { token: token.into(), parts: Vec::new() });
    let held = [series("communism"), series("democratic"), series("fascism"), series("unlisted")].into_iter().collect();
    let ideology = |token: &str| IdeologyInfo { token: token.into(), name: None, color: None };
    let order = stack_order(&held, &[ideology("democratic"), ideology("fascism"), ideology("communism")]);
    assert_eq!(order.iter().map(|party| party.token.as_str()).collect::<Vec<_>>(), ["democratic", "fascism", "communism", "unlisted"]);
}
