use super::*;
use crate::catalogue::tests::{Folder, folder_catalogue};
use crate::effect_description::tests::Fixture;
use crate::scripted_text::{EventObject, EventScope, Link};
use replay::{frame::TableView, rows::*};

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
                " BBB_ADJ:0 \"Beta adjective\"\n",
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

// CCaptureGeneralEffect::ReadEffect 1402F8640 consumes country/province;
// the province is execution-only, and target is an unexpected native key.
#[test]
fn capture_by_block_reads_country_without_target_leak() {
    let (fixture, _folder) = fixture();
    for (body, expected) in [
        (
            "capture_by={country=BBB province=17}",
            "Capture Army name/beta [Root.GetTag]",
        ),
        ("capture_by={target=BBB province=17}", ""),
    ] {
        let out = fixture.render(body, &mut character_scopes(), Content::default());
        assert_eq!(out.text, expected, "{body}");
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

// 1402F8130 and 140374C60 require a signed positive capturer/nationality.
// Selectors capital/random keep country zero (1405336C0); linked states
// instead supply their owner through 14052B6A0, state +0xC8.
#[test]
fn country_target_zero_and_state_owner_follow_native_character_gates() {
    let (mut fixture, _folder) = fixture();
    for category in [0, 3] {
        if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
        {
            rows[0].category = category;
        }
        let key = if category == 0 {
            "capture_by"
        } else {
            "add_nationality"
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
    let out = fixture.render("capture_by=ROOT", &mut graph, Content::default());
    assert_eq!(out.text, "Capture Army name/alpha [Root.GetTag]");
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
                name: Recorded::NotRecorded,
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
        "synthetic effect CCaptureGeneralEffect: key capture_by: character 7 referenced unit leader name (+0x40, 140C16FB0) not recorded".to_owned(),
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
            codename: Recorded::NotRecorded,
            ..Default::default()
        }]));
    let out = fixture.render(
        "add_nationality=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect CAddNationalityToOperativeEffect: key add_nationality: operative codename (COperativeLeader+0x1030, 140C06040) not recorded".to_owned(), 1)]));
    let out = fixture.render(
        "capture_by=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// Format-98 registry-only +0x40 is the actual selected role, not another
// character's display name (140F8A840 -> 1402F8130).
#[test]
fn capture_by_uses_format98_referenced_name_and_capture_status() {
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
        "capture_by=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Capture Referenced general/beta [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    if let TableView::Complete(TableRows::CharacterOperatives(rows)) =
        &mut *fixture.frame.tables[TableId::CharacterOperatives.index()]
    {
        rows[0].name = Recorded::Value(None);
    }
    let out = fixture.render(
        "capture_by=BBB",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect CCaptureGeneralEffect: key capture_by: character 7 referenced unit leader name (+0x40, 140C16FB0) capture unavailable".into(), 1)]));
}

// common/decisions/_debug_decisions.txt:724, scalar country;
// NAME/CODENAME/NATIONALITY@140374C60, expanded with no appended LF.
#[test]
fn add_nationality_uses_format98_selected_codename() {
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
            "add_nationality=BBB",
            &mut character_scopes(),
            Content::default(),
        );
        assert_eq!(
            out.text,
            format!(
                "Nationality Army name/{}/Beta adjective AAA",
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

// common/decisions/_debug_decisions.txt:511. ResolveArmy@140529260
// is null outside UNIT; the carrier no longer drops this reference.
#[test]
fn add_history_entry_is_empty_without_unit_reference() {
    let (fixture, _folder) = fixture();
    let out = fixture.render(
        "add_history_entry={key=synthetic_history subject=\"Custom String\" allow=yes}",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// common/raids/land_infiltration_custom.txt:218. ResolveRaidInstance
// 140529460 is null outside RAID; 1404A2FB0 emits only a diagnostic.
#[test]
fn add_raid_history_entry_is_empty_without_raid_reference() {
    let (fixture, _folder) = fixture();
    let out = fixture.render(
        "add_raid_history_entry=yes",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
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

#[test]
fn named_character_selector_uses_retired_roster_after_active() {
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
        "set_nationality={character=synthetic_other target_country=BBB}",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Nationality Other name/beta AAA\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn missing_executable_localisation_name_is_named_and_counted() {
    let fixture = Fixture::new();
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let out = fixture.render(
        "add_ace={name=First surname=Last callsign=Nick type=synthetic_ace}",
        &mut scopes,
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(
        out.refusals,
        BTreeMap::from([(
            "synthetic effect CAddAceEffect: key add_ace: native character localisation key EFFECT_ADD_ACE is not defined".into(),
            1,
        )]),
    );
}

#[test]
fn character_lookup_keeps_country_five_thousand() {
    let (mut fixture, _folder) = fixture();
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *fixture.frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    for slot in 4_u32..=5_000 {
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
            country: 6_000,
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
        scope_id: 6_000,
        ..Default::default()
    });
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(5_000))]);
    let out = fixture.render(
        "set_nationality={character=synthetic_other target_country=BBB}",
        &mut scopes,
        Content::default(),
    );
    assert_eq!(out.text, "Nationality Other name/beta TFW\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}
