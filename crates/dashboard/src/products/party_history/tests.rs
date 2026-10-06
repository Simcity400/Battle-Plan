use super::{PartyNaming, RecordedPortrait, leader_portrait, party_key};
use crate::test_fixtures::{CAMPAIGN, ownership_frame, records};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{CharacterPortrait, CharacterPortraitRow, TableId, TableRows},
};
use std::sync::atomic::AtomicBool;

#[test]
fn a_party_is_named_as_the_game_names_it() {
    let loc = crate::catalogue::tests::loaded(concat!(
        "l_english:\n",
        " GER_fascism_party:0 \"NSDAP\"\n",
        " GER_fascism_party_long:0 \"Nationalsozialistische Deutsche Arbeiterpartei\"\n",
        " SOV_x_party:0 \"VKP(b)\"\n",
        " fascism:0 \"Fascism\"\n",
        " scoped_party:0 \"[ROOT.GetAdjective] Party\"\n",
        " gap_party:0 \"Front¤Populaire\"\n",
    ));
    let refused = std::cell::RefCell::new(crate::catalogue::Refusals::default());
    let name = |naming: &PartyNaming| {
        let key = party_key(naming, |key| loc.get(key).is_some())?;
        crate::catalogue::localised(&loc, &key, crate::catalogue::View::Plain)
            .inspect_err(|reason| refused.borrow_mut().note(reason.clone(), &key))
            .ok()
    };
    let naming =
        |token: &str, tag: Option<&str>, original: Option<&str>, key: Option<&str>| PartyNaming {
            token: token.to_owned(),
            tag: tag.map(str::to_owned),
            original_tag: original.map(str::to_owned),
            key: key.map(str::to_owned),
        };
    // The tag's key; the original tag is not needed when it exists.
    assert_eq!(
        name(&naming("fascism", Some("GER"), None, None)).as_deref(),
        Some("NSDAP")
    );
    // A dynamic country takes its original tag's key.
    assert_eq!(
        name(&naming("fascism", Some("D01"), Some("GER"), None)).as_deref(),
        Some("NSDAP")
    );
    // Neither: the ideology group's own name, or its token.
    assert_eq!(
        name(&naming("fascism", Some("ITA"), Some(""), None)).as_deref(),
        Some("Fascism")
    );
    assert_eq!(
        name(&naming("modism", Some("ITA"), Some(""), None)).as_deref(),
        Some("modism")
    );
    // Not recorded: not guessed.
    assert_eq!(name(&naming("fascism", Some("D01"), None, None)), None);
    assert_eq!(name(&naming("fascism", None, Some(""), None)), None);
    // set_party_name: the stored key, shown as itself when undefined.
    assert_eq!(
        name(&naming("x", Some("GER"), None, Some("SOV_x_party"))).as_deref(),
        Some("VKP(b)")
    );
    assert_eq!(
        name(&naming("x", Some("GER"), None, Some("MOD_key"))).as_deref(),
        Some("MOD_key")
    );
    // A literal name is its raw bytes: an authored `§` is not a stored colour control.
    assert_eq!(
        name(&naming("x", Some("GER"), None, Some("§YCustom Party§!"))).as_deref(),
        Some("§YCustom Party§!")
    );
    assert!(refused.borrow().lines("x").is_empty());
    // Party names reach their text boxes plain (14118F860): brackets
    // are glyphs.
    assert_eq!(
        name(&naming("x", Some("GER"), None, Some("scoped_party"))).as_deref(),
        Some("[ROOT.GetAdjective] Party")
    );
    // A refused name is not shown, and why is noted by key.
    assert_eq!(
        name(&naming("x", Some("GER"), None, Some("gap_party"))),
        None
    );
    assert_eq!(
        refused.borrow().lines("party names are"),
        [
            "1 party names are not shown: localisation gap_party holds the control byte 0x14 (an authored `¤`), which plain text cannot show; gap_party",
        ]
    );
}

#[test]
fn a_leader_shows_the_civilian_large_entry_or_the_first_large_one_when_set() {
    let entry = |category, size, is_set, name: &str| CharacterPortrait {
        category,
        size,
        is_set,
        kind: 0,
        identity: name.as_bytes().to_vec(),
    };
    let row = |character, portraits| CharacterPortraitRow {
        character,
        portraits,
    };
    let holders = [
        row(
            1,
            vec![entry(0, 0, 1, "civ_small"), entry(0, 1, 1, "civ_large")],
        ),
        // No civilian large: the first large entry, as the game's search returns it.
        row(
            2,
            vec![entry(0, 0, 1, "civ_small"), entry(1, 1, 1, "army_large")],
        ),
        // An unset entry is not tried further: the game generates one.
        row(
            3,
            vec![entry(0, 1, 0, "unset"), entry(1, 1, 1, "army_large")],
        ),
        row(4, vec![entry(0, 0, 1, "civ_small")]),
    ];
    let named = |name: &str| RecordedPortrait::Entry {
        kind: 0,
        identity: name.as_bytes().to_vec(),
    };
    assert_eq!(leader_portrait(Some(&holders), 1), named("civ_large"));
    assert_eq!(leader_portrait(Some(&holders), 2), named("army_large"));
    assert_eq!(
        leader_portrait(Some(&holders), 3),
        RecordedPortrait::Generated
    );
    assert_eq!(
        leader_portrait(Some(&holders), 4),
        RecordedPortrait::Generated
    );
    // A holder this hour did not record is not a generated portrait.
    assert_eq!(
        leader_portrait(Some(&holders), 5),
        RecordedPortrait::Unrecorded
    );
    assert_eq!(leader_portrait(None, 1), RecordedPortrait::Unrecorded);
}

/// GER (link 9001) with two parties at the given popularities (raw), one ruling,
/// and its last election hour.
fn party_frame(
    hour: i32,
    fascism: i64,
    democratic: i64,
    ruling: &str,
    election: i32,
) -> DecodedFrame {
    let mut frame = ownership_frame(1, hour, 44);
    let party = |id: u32, token: &str, popularity: i64| replay::rows::PoliticalPartyRow {
        country: 9001,
        id,
        popularity,
        is_ruling: u8::from(token == ruling),
        token: token.as_bytes().to_vec(),
        ..Default::default()
    };
    let mut parties = vec![
        party(1, "democratic", democratic),
        party(2, "fascism", fascism),
        // Another country's party is never GER's.
        replay::rows::PoliticalPartyRow {
            country: 44,
            id: 1,
            popularity: 1,
            is_ruling: 1,
            token: b"democratic".to_vec(),
            ..Default::default()
        },
    ];
    parties.sort_by_key(|row| (row.country, row.id));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(parties));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay::rows::PoliticsRow {
            country: 9001,
            last_election: election,
            ..Default::default()
        }]));
    frame
}

#[test]
fn party_history_follows_popularity_ruling_changes_and_elections() {
    use crate::products::party_history::{RulingRun, scan};
    let frames = vec![
        party_frame(100, 40_00000, 35_00000, "fascism", 10),
        party_frame(101, 40_00000, 35_00000, "fascism", 10),
        party_frame(102, 45_00000, 30_00000, "fascism", 10),
        // Hour 103 is missing; at 104 an election was held and the democrats rule.
        party_frame(104, 20_00000, 60_00000, "democratic", 104),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let fascism = &scanned.parties["fascism"];
    assert_eq!(
        fascism
            .runs
            .iter()
            .map(|run| (
                run.first_hour,
                run.last_hour,
                run.value_raw.as_str(),
                run.starts_segment
            ))
            .collect::<Vec<_>>(),
        [
            (100, 101, "4000000", true),
            (102, 102, "4500000", false),
            (104, 104, "2000000", true)
        ]
    );
    let ruling = |token: &str, first_hour, last_hour| RulingRun {
        token: token.into(),
        first_hour,
        last_hour,
        starts_segment: true,
    };
    assert_eq!(
        scanned.ruling,
        [ruling("fascism", 100, 102), ruling("democratic", 104, 104)]
    );

    assert_eq!(scanned.elections, [104]);
    assert_eq!(
        scanned.unobserved,
        [crate::products::country_groups::HourSpan {
            first_hour: 103,
            last_hour: 103
        }]
    );
}
