use super::{Election, GovernmentCauseKind, GovernmentChange, IdeologyInfo, LeaderRun, PartOwnership, PartySeries, RulingRun, read, stack_order};
use dashboard_base::{Source, products::Product, test_fixtures::{CAMPAIGN, records, source, source_with_mods, visual_mods::Machine}};
use replay::campaign::Replay;
use replay_data::rows::{
    CharacterRow, CountryAppearanceRow, CountryLinkRow, CountryRow, GovernmentCause, GovernmentChangeRow, LexicalTokenRow, PoliticalPartyRow, PoliticsCaptureCoverageRow, PoliticsRow, Recorded,
    ReservedCountryNamingRow, TableId, TableRows,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::{Arc, atomic::AtomicBool};

const FIRST: i32 = 100;
const LAST: i32 = 130;
const GER: u32 = 9001;

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    frame.tables[table.index()] = TableView::Complete(rows).into();
    frame.metrics.table_ns[table.index()] = 1;
}

fn unread(frame: &mut DecodedFrame, table: TableId) {
    frame.tables[table.index()] = TableView::Unavailable { reason: "not read".into() }.into();
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1_000));
    if hour == 115 {
        set(&mut frame, TableId::Countries, TableRows::Countries(Vec::new()));
    }
    let character = CharacterRow { id: 701, country: GER, token: if hour >= 120 { b"TEST_LEADER_RENAMED".to_vec() } else { b"TEST_LEADER_NAME".to_vec() }, ..Default::default() };
    set(&mut frame, TableId::Characters, TableRows::Characters(vec![character]));
    let words = [(51, "democratic"), (52, "fascism")].map(|(token, literal)| LexicalTokenRow { token, literal: literal.as_bytes().to_vec() });
    set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(words.to_vec()));
    let coup = GovernmentChangeRow {
        country: GER,
        sequence: 1,
        epoch: 1,
        hour: 110,
        before_ideology: Some(51),
        after_ideology: Some(52),
        cause: GovernmentCause::CoupCompletion,
        ..Default::default()
    };
    if (124..=125).contains(&hour) {
        unread(&mut frame, TableId::GovernmentChanges);
    } else {
        set(&mut frame, TableId::GovernmentChanges, TableRows::GovernmentChanges(if hour == 110 { vec![coup] } else { Vec::new() }));
    }
    let streams = if hour == FIRST || hour == 118 { 0 } else { PoliticsCaptureCoverageRow::GOVERNMENT };
    let witness = PoliticsCaptureCoverageRow { country: GER, epoch: 1, since_hour: (hour - 1).max(FIRST), through_hour: hour, streams, ..Default::default() };
    set(&mut frame, TableId::PoliticsCaptureCoverage, TableRows::PoliticsCaptureCoverage(vec![witness]));
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

fn recording(frames: Vec<DecodedFrame>) -> Source {
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "parties".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

#[test]
fn a_countrys_parties_ruling_party_leader_and_elections_are_runs_that_never_bridge_a_gap() {
    let machine = Machine::new("ruling-party-marker-names");
    machine.write("common/defines/base.lua", b"NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}");
    machine.write_mod("probe", "localisation/test_l_english.yml", "\u{feff}l_english:\n TEST_LEADER_NAME:0 \"A\u{a4}B\"\n TEST_LEADER_RENAMED:0 \"C\"\n".as_bytes());
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
    assert_eq!(
        parties.leaders,
        [leader(None, 100, 104, true), leader(Some(0), 105, 114, false), leader(Some(0), 116, 119, true), leader(Some(1), 120, LAST, false)],
        "an unread party table does not discard the leader read beside it, and a renamed leader is named anew from the hour it was read"
    );
    assert_eq!(parties.people.iter().map(|person| (person.character.id, person.character.name.as_str())).collect::<Vec<_>>(), [(701, "TEST_LEADER_NAME"), (701, "TEST_LEADER_RENAMED")]);
    assert_eq!(
        parties.government_changes,
        [GovernmentChange { hour: 110, witnessed: 110, before: Some("democratic".into()), after: Some("fascism".into()), cause: Some(GovernmentCauseKind::Coup) }]
    );
    assert_eq!(
        parties.government_unobserved.iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>(),
        [(100, 100), (115, 115), (118, 118), (124, 125)],
        "government changes are unrecorded before the first witness, outside the holder, where the stream was not witnessed and where the table was unread"
    );
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
    let reply = dashboard_text::catalogue::own_refusals(&mut Default::default(), || {
        super::present(Ok(&politics), &params, "GER", &parties, &AtomicBool::new(false))
    }).unwrap();
    let narrow = dashboard_text::catalogue::own_refusals(&mut Default::default(), || super::present_ruling(Ok(&politics), "GER", &ruling_only));
    let leader_warning = "1 texts are not shown: localisation TEST_LEADER_NAME holds the control byte 0x14 (an authored `\u{a4}`), which plain text cannot show; TEST_LEADER_NAME";
    let (leader, marker): (Vec<_>, Vec<_>) = reply.warnings.iter().map(String::as_str).partition(|warning| *warning == leader_warning);
    assert_eq!(leader, [leader_warning]);
    assert_eq!(narrow.warnings, marker);
    assert_eq!(narrow.runs, reply.ruling);
    assert_eq!(narrow.unobserved, reply.unobserved);
    assert_eq!(narrow.ideologies, reply.ideologies);
    assert!(narrow.warnings.iter().any(|warning| warning == "party name: original tag reference of country appearance capture refused"));
    let marker_params = crate::products::stability_markers::StabilityMarkersParams { branch_id: "1".into(), country: "GER".into(), hour: LAST, focus: false, government: true, wars: false, factions: false };
    let markers = dashboard_text::catalogue::own_refusals(&mut Default::default(), || {
        crate::products::stability_markers::StabilityMarkers::load(&source, &marker_params, &load, &AtomicBool::new(false))
    }).unwrap();
    assert!(markers.markers.iter().any(|marker| marker.id.starts_with("party:")));
    assert!(markers.warnings.iter().any(|warning| warning == "party name: original tag reference of country appearance capture refused"));
    assert!(!markers.warnings.iter().any(|warning| warning == leader_warning));
}

const NAMED: u32 = 4401;

fn naming_frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1_000));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow { id, canonical_slot, tag: tag.to_vec(), border_land_doctrine_level: Recorded::Value(None), ..Default::default() };
    let other = if (210..214).contains(&hour) { b"BBB" } else { b"AAA" };
    set(&mut frame, TableId::Countries, TableRows::Countries(vec![country(NAMED, 20, other), country(GER, 7, b"GER")]));
    let links = [(11, 7), (NAMED, 20), (GER, 7)].map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot });
    set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(links.to_vec()));
    let party = PoliticalPartyRow { country: GER, id: 1, popularity: 10_000_000, is_ruling: 1, token: b"democratic".to_vec(), tag: if hour >= 214 { 0 } else { NAMED }, name_default: 1, ..Default::default() };
    set(&mut frame, TableId::PoliticalParties, TableRows::PoliticalParties(vec![party]));
    if (205..=206).contains(&hour) {
        unread(&mut frame, TableId::CountryAppearance);
    } else {
        let look = |country| CountryAppearanceRow { country, original_tag_reference: Recorded::Value(Some(0)), ..Default::default() };
        set(&mut frame, TableId::CountryAppearance, TableRows::CountryAppearance(vec![look(NAMED), look(GER)]));
    }
    let reserved = ReservedCountryNamingRow { tag: if hour >= 217 { b"DDD".to_vec() } else { b"CCC".to_vec() }, original_tag_reference: Recorded::Value(Some(0)), ..Default::default() };
    set(&mut frame, TableId::ReservedCountryNaming, TableRows::ReservedCountryNaming(vec![reserved]));
    frame
}

#[test]
fn a_party_named_for_another_country_follows_that_countrys_naming_and_never_bridges_its_gaps() {
    let source = recording((200..=220).map(naming_frame).collect());
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let parties = read(&query, &"GER".into(), super::PartyRead::Full, &AtomicBool::new(false)).unwrap();
    let named = |run: &super::PartyNameRun| {
        let (naming, selected) = &parties.ruling.namings[run.name as usize];
        (run.first_hour, run.last_hour, naming.tag.clone(), selected.clone().ok())
    };
    let tag = |tag: &str| Some(tag.to_owned());
    assert_eq!(
        parties.ruling.name_runs.iter().map(named).collect::<Vec<_>>(),
        [
            (200, 204, tag("AAA"), tag("AAA")),
            (205, 206, tag("AAA"), None),
            (207, 209, tag("AAA"), tag("AAA")),
            (210, 213, tag("BBB"), tag("BBB")),
            (214, 216, tag("CCC"), tag("CCC")),
            (217, 220, tag("DDD"), tag("DDD")),
        ],
        "the other country's unread appearance, its new tag and the reserved country's new tag each name the party anew"
    );
}

#[test]
fn parties_stack_in_the_contents_ideology_order_whatever_rules() {
    let series = |token: &str| (token.to_owned(), PartySeries { token: token.into(), parts: Vec::new() });
    let held = [series("communism"), series("democratic"), series("fascism"), series("unlisted")].into_iter().collect();
    let ideology = |token: &str| IdeologyInfo { token: token.into(), name: None, color: None };
    let order = stack_order(&held, &[ideology("democratic"), ideology("fascism"), ideology("communism")]);
    assert_eq!(order.iter().map(|party| party.token.as_str()).collect::<Vec<_>>(), ["democratic", "fascism", "communism", "unlisted"]);
}
