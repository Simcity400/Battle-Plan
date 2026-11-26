use super::*;
use crate::fixtures::{Folder, folder_catalogue};
use crate::effect_description::tests::Fixture;
use crate::scripted_text::{EventObject, EventScope, Link};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

pub(super) fn empty_variable_stores(fixture: &mut Fixture) {
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(
            [(1, 100), (1, 200), (1, 300), (3, 7), (3, 8)]
                .into_iter()
                .map(|(scope_kind, scope_id)| ScriptVariablesRow {
                    scope_kind,
                    scope_id,
                    ..Default::default()
                })
                .collect(),
        ));
}

fn fixture() -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " AAA:0 \"Aurora\"\n BBB:0 \"Basin\"\n AAA_DEF:0 \"aurora\"\n BBB_DEF:0 \"basin\"\n",
                " EFFECT_SET_LEADER_NATIONALITY:0 \"Nationality $NAME$/$COUNTRYDEF$ [Root.GetTag]\"\n",
                " EFFECT_RETIRE_CHARACTER:0 \"Retire $NAME$/$COUNTRY$ [Root.GetTag]\"\n",
                " EFFECT_CAPTURE_GENERAL:0 \"Capture $NAME$/$COUNTRY_NAME_DEF$ [Root.GetTag]\"\n",
                " BBB_ADJ:0 \"Basin adjective\"\n",
                " EFFECT_ADD_NATIONALITY_TO_OPERATIVE:0 \"Nationality $NAME$/$CODENAME$/$NATIONALITY$ [Root.GetTag]\"\n",
            ),
        ),
        (
            "common/units/codenames_operatives/test.txt",
            "synthetic_codes={type=codename unique={\"Nightjar\"}}",
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
    empty_variable_stores(&mut fixture);
    (fixture, folder)
}

fn character_scopes() -> EventScopes {
    let mut character = EventScope::of(EventObject::Character(7));
    character.root = Link::Scope(1);
    EventScopes::new(vec![character, EventScope::of(EventObject::Country(1))])
}

#[test]
fn nationality_character_selector_uses_root_roster_then_current_fallback() {
    game::enter();
    let (fixture, _folder) = fixture();
    for (selector, name) in [
        ("synthetic_other", "Other name"),
        ("nonexistent_character", "Current name"),
        ("THIS", "Current name"),
    ] {
        let out = fixture.render(
            &format!("fixture_switch_allegiance={{character={selector} target_country=BBB}}"),
            &mut character_scopes(),
            Content::default(),
        );
        assert_eq!(
            out.text,
            format!("Nationality {name}/basin AAA\n"),
            "{:?}",
            out.refusals
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn retire() {
    game::enter();
    let (fixture, _folder) = fixture();
    let out = fixture.render("fixture_leave_service=yes", &mut character_scopes(), Content::default());
    assert_eq!(out.text, "Retire Current name/Aurora [Root.GetTag]\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn capture_by() {
    game::enter();
    let (fixture, _folder) = fixture();
    let out = fixture.render(
        "fixture_captured_leader=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Capture Army name/basin [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn capture_by_block_reads_country_without_target_leak() {
    game::enter();
    let (fixture, _folder) = fixture();
    for (body, expected) in [
        (
            "fixture_captured_leader={country=BBB province=29}",
            "Capture Army name/basin [Root.GetTag]",
        ),
        ("fixture_captured_leader={target=BBB province=29}", ""),
    ] {
        let out = fixture.render(body, &mut character_scopes(), Content::default());
        assert_eq!(out.text, expected, "{body}");
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn country_target_zero_and_state_owner_follow_character_gates() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    for category in [0, 3] {
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
        {
            rows[0].category = category;
        }
        let key = if category == 0 {
            "fixture_captured_leader"
        } else {
            "fixture_additional_allegiance"
        };
        for selector in ["capital", "random"] {
            let out = fixture.render(
                &format!("{key}={selector}"),
                &mut character_scopes(),
                Content::default(),
            );
            assert_eq!(out.text, "", "{key}={selector}");
            assert!(
                out.refusals.is_empty(),
                "{key}={selector}: {:?}",
                out.refusals
            );
        }
    }
    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
    {
        rows[0].category = 0;
    }
    let mut graph = EventScopes::new(vec![
        EventScope {
            root: Link::Scope(1),
            ..EventScope::of(EventObject::Character(7))
        },
        EventScope::of(EventObject::State(2)),
    ]);
    let out = fixture.render("fixture_captured_leader=ROOT", &mut graph, Content::default());
    assert_eq!(out.text, "Capture Army name/aurora [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn referenced_leader_uses_matching_recorded_handle_or_refuses_its_missing_name() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![
            CharacterOperativeRow {
                character: 7,
                reference: (5_u64 << 32) | 55,
                resolved: true,
                category: 0,
                name: Recorded::NotRecorded,
                ..Default::default()
            },
        ]));
    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
    {
        rows[0].character_id = 8;
    }
    let out = fixture.render(
        "fixture_captured_leader=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Capture Army name/basin [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);

    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
    {
        rows[0].id = 6;
    }
    let out = fixture.render(
        "fixture_captured_leader=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([(
        "synthetic effect fixture_captured_leader: key fixture_captured_leader: character 7 referenced unit leader name not recorded".to_owned(),
        1
    )]));
}

#[test]
fn unit_leader_scope_resolves_its_character_role() {
    game::enter();
    let (fixture, _folder) = fixture();
    let mut leader = EventScope::of(EventObject::UnitLeader((5_u64 << 32) | 55));
    leader.root = Link::Scope(1);
    let out = fixture.render(
        "fixture_leave_service=yes fixture_captured_leader=BBB",
        &mut EventScopes::new(vec![leader, EventScope::of(EventObject::Country(1))]),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "Retire Current name/Aurora [Root.GetTag]\nCapture Army name/basin [Root.GetTag]"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn add_nationality() {
    game::enter();
    let (fixture, _folder) = fixture();
    let out = fixture.render(
        "fixture_additional_allegiance=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn valid_operative_refuses_unrecorded_codename_by_class_and_field() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            reference_type: 55,
            id: 5,
            character_id: 7,
            category: 3,
            codename: Recorded::NotRecorded,
            ..Default::default()
        }]));
    let out = fixture.render(
        "fixture_additional_allegiance=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect fixture_additional_allegiance: key fixture_additional_allegiance: operative codename not recorded".to_owned(), 1)]));
    let out = fixture.render(
        "fixture_captured_leader=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn capture_by_uses_format98_referenced_name_and_capture_status() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![]));
    *fixture.frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![
            CharacterOperativeRow {
                character: 7,
                reference: (5_u64 << 32) | 55,
                resolved: true,
                category: 1,
                name: Recorded::Value(Some(b"Referenced general".to_vec())),
                ..Default::default()
            },
        ]));
    let out = fixture.render(
        "fixture_captured_leader=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Capture Referenced general/basin [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    if let TableView::Complete(TableRows::CharacterOperatives(rows)) =
        &mut *fixture.frame.tables[TableId::CharacterOperatives.index()]
    {
        rows[0].name = Recorded::Value(None);
    }
    let out = fixture.render(
        "fixture_captured_leader=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect fixture_captured_leader: key fixture_captured_leader: character 7 referenced unit leader name capture unavailable".into(), 1)]));
}

#[test]
fn add_nationality_uses_format98_selected_codename() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    for custom in [Vec::new(), b"Chosen name".to_vec()] {
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
        {
            rows[0].category = 3;
            rows[0].kind = 3;
            rows[0].codename = Recorded::Value(Some(NameOperands {
                kind: 2,
                order: 0,
                group: b"synthetic_codes".to_vec(),
                custom: custom.clone(),
                ..Default::default()
            }));
        }
        let out = fixture.render(
            "fixture_additional_allegiance=BBB",
            &mut character_scopes(),
            Content::default(),
        );
        assert_eq!(
            out.text,
            format!(
                "Nationality Army name/{}/Basin adjective AAA",
                if custom.is_empty() {
                    "Nightjar"
                } else {
                    "Chosen name"
                }
            )
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn history_entries_are_empty_without_their_reference() {
    game::enter();
    let (fixture, _folder) = fixture();
    for body in [
        "fixture_division_history={key=synthetic_history subject=\"Training record\" allow=yes}",
        "fixture_raid_history=yes",
    ] {
        let out = fixture.render(body, &mut character_scopes(), Content::default());
        assert_eq!(out.text, "", "{body}");
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn absent_character_is_empty_and_stored_name_does_not_use_saved_key() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let out = fixture.render(
        "fixture_leave_service=yes fixture_captured_leader=BBB fixture_additional_allegiance=BBB fixture_switch_allegiance=BBB",
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
        "fixture_switch_allegiance=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Nationality /basin AAA\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn character_and_leader_names_keep_recording_language_rule() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    std::sync::Arc::make_mut(&mut fixture.catalogue.localisation).language =
        "synthetic_other_language".into();
    for body in ["fixture_switch_allegiance=BBB", "fixture_leave_service=yes", "fixture_captured_leader=BBB"] {
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

#[test]
fn named_character_selector_uses_retired_roster_after_active() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(vec![]));
    *fixture.frame.tables[TableId::CountryRetiredCharacters.index()] =
        TableView::Complete(TableRows::CountryRetiredCharacters(vec![
            CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(8),
            },
        ]));
    let out = fixture.render(
        "fixture_switch_allegiance={character=synthetic_other target_country=BBB}",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Nationality Other name/basin AAA\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn missing_executable_localisation_name_is_named_and_counted() {
    game::enter();
    let fixture = Fixture::new();
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let out = fixture.render(
        "fixture_air_specialist={name=First surname=Last callsign=Nick type=synthetic_ace}",
        &mut scopes,
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(
        out.refusals,
        BTreeMap::from([(
            "synthetic effect fixture_air_specialist: key fixture_air_specialist: native character localisation key EFFECT_ADD_ACE is not defined".into(),
            1,
        )]),
    );
}

#[test]
fn character_lookup_keeps_a_late_country() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *fixture.frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    for slot in 4_u32..=24 {
        let tag_index = slot + 7_996;
        countries.push(CountryRow {
            id: slot + 1_000,
            canonical_slot: slot,
            tag: vec![
                b'A' + (tag_index / 676) as u8,
                b'A' + ((tag_index / 26) % 26) as u8,
                b'A' + (tag_index % 26) as u8,
            ],
            ..Default::default()
        });
    }
    let links = countries
        .iter()
        .map(|row| CountryLinkRow {
            id: row.id,
            canonical_slot: row.canonical_slot,
        })
        .collect();
    *fixture.frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links));
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(vec![CountryCharacterRow {
            country: 1_024,
            ordinal: 0,
            character: Some(8),
        }]));
    let TableView::Complete(TableRows::ScriptVariables(stores)) =
        &mut *fixture.frame.tables[TableId::ScriptVariables.index()]
    else {
        unreachable!()
    };
    stores.push(ScriptVariablesRow {
        scope_kind: 1,
        scope_id: 1_024,
        ..Default::default()
    });
    stores.sort_by_key(|row| (row.scope_kind, row.scope_id));
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(24))]);
    let out = fixture.render(
        "fixture_switch_allegiance={character=synthetic_other target_country=BBB}",
        &mut scopes,
        Content::default(),
    );
    assert_eq!(out.text, "Nationality Other name/basin LWM\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}
