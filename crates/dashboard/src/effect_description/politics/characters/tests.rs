use super::*;
use crate::catalogue::tests::{Folder, folder_catalogue};
use crate::effect_description::tests::Fixture;
use crate::scripted_text::{EventObject, EventScope, Link};
use replay::{frame::TableView, rows::*};

fn fixture() -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n AAA_DEF:0 \"alpha\"\n BBB_DEF:0 \"beta\"\n",
                " EFFECT_SET_LEADER_NATIONALITY:0 \"Nationality $NAME$/$COUNTRYDEF$ [Root.GetTag]\"\n",
                " EFFECT_RETIRE_CHARACTER:0 \"Retire $NAME$/$COUNTRY$ [Root.GetTag]\"\n",
                " EFFECT_CAPTURE_GENERAL:0 \"Capture $NAME$/$COUNTRY_NAME_DEF$ [Root.GetTag]\"\n",
            ),
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![
            CharacterRow {
                id: 7,
                country: 100,
                name_token_id: 10,
                name: b"Current name".to_vec(),
                token: b"unused_key".to_vec(),
                ..Default::default()
            },
            CharacterRow {
                id: 8,
                country: 100,
                name_token_id: 11,
                name: b"Other name".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            reference_type: 55,
            id: 5,
            character_id: 7,
            category: 0,
            inherited_text_b: b"Army name".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 10,
                literal: b"synthetic_current".to_vec(),
            },
            LexicalTokenRow {
                token: 11,
                literal: b"synthetic_other".to_vec(),
            },
        ]));
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(vec![
            CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(7),
            },
            CountryCharacterRow {
                country: 100,
                ordinal: 1,
                character: Some(8),
            },
        ]));
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 3,
            scope_id: 7,
            ..Default::default()
        }]));
    (fixture, folder)
}

fn character_scopes() -> EventScopes {
    let mut character = EventScope::of(EventObject::Character(7));
    character.root = Link::Scope(1);
    EventScopes::new(vec![character, EventScope::of(EventObject::Country(1))])
}

// Scalar witness: common/decisions/_debug_decisions.txt:665.
#[test]
fn set_nationality() {
    let (fixture, _folder) = fixture();
    let out = fixture.render(
        "set_nationality=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Nationality Current name/beta AAA\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn nationality_character_selector_uses_root_roster_then_current_fallback() {
    let (fixture, _folder) = fixture();
    for (selector, name) in [
        ("synthetic_other", "Other name"),
        ("nonexistent_character", "Current name"),
        ("THIS", "Current name"),
    ] {
        let out = fixture.render(
            &format!("set_nationality={{character={selector} target_country=BBB}}"),
            &mut character_scopes(),
            Content::default(),
        );
        assert_eq!(
            out.text,
            format!("Nationality {name}/beta AAA\n"),
            "{:?}",
            out.refusals
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

// common/decisions/bul.txt:2278. Authored boolean is execution-only.
#[test]
fn retire() {
    let (fixture, _folder) = fixture();
    let out = fixture.render("retire=yes", &mut character_scopes(), Content::default());
    assert_eq!(out.text, "Retire Current name/Alpha [Root.GetTag]\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// common/decisions/_debug_decisions.txt:22.
#[test]
fn capture_by() {
    let (fixture, _folder) = fixture();
    let out = fixture.render(
        "capture_by=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Capture Army name/beta [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn referenced_leader_uses_matching_recorded_handle_or_refuses_its_missing_name() {
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![
            CharacterOperativeRow {
                character: 7,
                reference: (5_u64 << 32) | 55,
                resolved: true,
                category: 0,
                ..Default::default()
            },
        ]));
    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
    {
        // This character has only the reference role. Another direct role
        // independently acquired the same native leader, including its name.
        rows[0].character_id = 8;
    }
    let out = fixture.render(
        "capture_by=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Capture Army name/beta [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);

    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
    {
        rows[0].id = 6;
    }
    let out = fixture.render(
        "capture_by=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([(
        "synthetic effect CCaptureGeneralEffect: key capture_by: character 7 referenced army leader name (+0x40) is unrecorded".to_owned(),
        1
    )]));
}

#[test]
fn unit_leader_scope_resolves_its_character_role() {
    let (fixture, _folder) = fixture();
    let mut leader = EventScope::of(EventObject::UnitLeader((5_u64 << 32) | 55));
    leader.root = Link::Scope(1);
    let out = fixture.render(
        "retire=yes capture_by=BBB",
        &mut EventScopes::new(vec![leader, EventScope::of(EventObject::Country(1))]),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "Retire Current name/Alpha [Root.GetTag]\nCapture Army name/beta [Root.GetTag]"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn add_nationality() {
    let (fixture, _folder) = fixture();
    // Native early gate: an army leader is not an operative.
    let out = fixture.render(
        "add_nationality=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn valid_operative_refuses_unrecorded_codename_by_class_and_field() {
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            reference_type: 55,
            id: 5,
            character_id: 7,
            category: 3,
            ..Default::default()
        }]));
    let out = fixture.render(
        "add_nationality=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect CAddNationalityToOperativeEffect: key add_nationality: operative codename (COperativeLeader+0x1030, 140C06040) is unrecorded".to_owned(), 1)]));
    let out = fixture.render(
        "capture_by=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// common/decisions/_debug_decisions.txt:511. Scope gate only: the graph
// cannot yet express the authored division scope's UNIT_REFERENCE (+0x70/+0x74),
// or distinguish a known-absent carried reference from a dropped one.
#[test]
fn add_history_entry_refuses_missing_graph_reference() {
    let (fixture, _folder) = fixture();
    let out = fixture.render(
        "add_history_entry={key=synthetic_history subject=\"Custom String\" allow=yes}",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([(
        "synthetic effect CAddHistoryEntryEffect: key add_history_entry: CEventScope UNIT_REFERENCE (+0x70/+0x74) is not represented by the event graph (140529260); Divisions records the handles".to_owned(),
        1
    )]));
}

// common/raids/land_infiltration_custom.txt:218. Missing graph fields are refused; the
// compatible raid graph (+0x88/+0x8C) and full native line remain unimplemented.
#[test]
fn add_raid_history_entry_refuses_missing_graph_reference() {
    let (fixture, _folder) = fixture();
    let out = fixture.render(
        "add_raid_history_entry=yes",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([(
        "synthetic effect CAddRaidHistoryEntryEffect@?A0x8a0aef5d@NRaids: key add_raid_history_entry: CEventScope RAID_INSTANCE_REFERENCE (+0x88/+0x8C) is not represented by the event graph (140529460); Raids records the handles".to_owned(),
        1
    )]));
}

#[test]
fn absent_character_is_empty_and_stored_name_does_not_use_saved_key() {
    let (mut fixture, _folder) = fixture();
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let out = fixture.render(
        "retire=yes capture_by=BBB add_nationality=BBB set_nationality=BBB",
        &mut scopes,
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 7,
            country: 100,
            token: b"unused_key".to_vec(),
            ..Default::default()
        }]));
    let out = fixture.render(
        "set_nationality=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Nationality /beta AAA\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn character_and_leader_names_keep_recording_language_rule() {
    let (mut fixture, _folder) = fixture();
    std::sync::Arc::make_mut(&mut fixture.catalogue.localisation).language =
        "synthetic_other_language".into();
    for body in ["set_nationality=BBB", "retire=yes", "capture_by=BBB"] {
        let out = fixture.render(body, &mut character_scopes(), Content::default());
        assert_eq!(out.text, "");
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals
                .keys()
                .any(|why| why.contains("is text the game stored in english without its key")),
            "{body}: {:?}",
            out.refusals
        );
    }
}
