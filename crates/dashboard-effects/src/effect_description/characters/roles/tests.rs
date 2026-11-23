use super::*;
use crate::fixtures::{Folder, folder_catalogue};
use crate::effect_description::tests::Fixture;
use crate::scripted_text::{EventObject, EventScope, Link};
use replay_data::rows::*;
use replay_frame::frame::TableView;

fn fixture() -> (Fixture, Folder) {
    fixture_with_advisor_modifier("")
}

fn fixture_with_advisor_modifier(modifier: &str) -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let characters = [
        "characters={synthetic_character={name=synthetic_name country_leader={ideology=synthetic_ideology traits={synthetic_trait}} advisor={slot=synthetic_slot idea_token=synthetic_advisor traits={synthetic_trait} modifier={",
        modifier,
        "}}} synthetic_unrecruited={name=unrecruited_name country_leader={ideology=synthetic_ideology}} synthetic_conditional={instance={allowed={always=no} name=conditional_name country_leader={ideology=synthetic_ideology}} instance={allowed={always=no} name=conditional_name country_leader={ideology=synthetic_ideology}}} synthetic_split_name={instance={allowed={always=yes} name=first country_leader={ideology=synthetic_ideology}} instance={allowed={always=yes} name=second country_leader={ideology=synthetic_ideology}}}}",
    ]
    .concat();
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={\"synthetic_slot\"}}}",
        ),
        (
            "common/ideologies/test.txt",
            "ideologies={ synthetic_group={types={synthetic_ideology={}}} }",
        ),
        (
            "common/country_leader/test.txt",
            "leader_traits={synthetic_trait={} synthetic_numeric={political_power_gain=0.1} synthetic_tip={custom_modifier_tooltip=SYN_TIP} synthetic_two_tips={custom_modifier_tooltip=SYN_TIP custom_modifier_tooltip=SYN_OTHER_TIP}}",
        ),
        ("common/characters/test.txt", characters.as_str()),
        (
            "common/factions/templates/test.txt",
            "synthetic_faction={name=synthetic_faction_name} synthetic_bound={name={localization_key=synthetic_faction_root PART={localization_key=synthetic_faction_nested WORD=synthetic_word}}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n AAA_DEF:0 \"alpha\"\n BBB_DEF:0 \"beta\"\n",
                " synthetic_group:0 \"Group\"\n synthetic_trait:0 \"Trait\"\n synthetic_slot:0 \"Slot [Root.GetTag]\"\n",
                " POWER:0 \"Power\"\n synthetic_numeric:0 \"Numeric\"\n",
                " SYN_TIP:0 \"Tip [Root.GetTag]\"\n",
                " POWER_WITH_ARG:0 \"Power $ARG$\"\n ARG:0 \"Argument fallback\"\n",
                " synthetic_faction_name:0 \"Template [Root.GetTag]\"\n synthetic_faction_root:0 \"Root $PART$\"\n synthetic_faction_nested:0 \"Nested $WORD$\"\n synthetic_word:0 \"Word\"\n",
                " AAA_archived_label:0 \"Catalogued predecessor\"\n synthetic_name:0 \"Content name\"\n unrecruited_name:0 \"Unrecruited [Root.GetTag]\"\n",
                " EFFECT_ADD_COUNTRY_LEADER_ROLE_PROMOTE:0 \"Promote $NAME$/$IDEOLOGY$ [Root.GetTag]\"\n",
                " EFFECT_ADD_COUNTRY_LEADER_ROLE:0 \"Role $NAME$/$IDEOLOGY$ [Root.GetTag]\"\n",
                " EFFECT_ADD_ARMY_EXPERIENCE:0 \"XP$VAL|0$\"\n",
                " EFFECT_RETIRE_CHARACTER:0 \"Retire $NAME$/$COUNTRY$ [Root.GetTag]\"\n",
                " EFFECT_RETIRE_COUNTRY_LEADER:0 \"Retire leader $NAME$ [Root.GetTag]\"\n",
                " EFFECT_KILL_COUNTRY_LEADER:0 \"Kill leader $NAME$ [Root.GetTag]\"\n",
                " EFFECT_REMOVE_COUNTRY_LEADER_ROLE:0 \"Remove role $NAME$/$IDEOLOGY$ [Root.GetTag]\"\n",
                " EFFECT_REMOVE_ADVISOR_ROLE:0 \"Remove advisor $NAME$/$SLOT$ [Root.GetTag]\"\n",
                " EFFECT_ADD_ADVISOR_ROLE:0 \"Add advisor $NAME$/$SLOT$/$MODIFIER$ [Root.GetTag]\"\n",
                " EFFECT_ADD_ADVISOR_ROLE_APPOINT:0 \"Appoint $NAME$/$SLOT$/$MODIFIER$ [Root.GetTag]\"\n",
                " EFFECT_ASSIGN_ADVISOR:0 \"Activate $NAME$/$DESC$ [Root.GetTag]\"\n",
                " EFFECT_DEACTIVATE_ADVISOR:0 \"Deactivate $NAME$/$DESC$ [Root.GetTag]\"\n",
                " CACHE_A:0 \"First [Root.GetTag]\"\n CACHE_B:0 \"Second\"\n",
                " EFFECT_SET_CAN_BE_FIRED:0 \"Fire $CHARACTER$/$SLOT$[Root.GetTag]\"\n",
                " EFFECT_SET_CANNOT_BE_FIRED:0 \"Keep $CHARACTER$/$SLOT$[Root.GetTag]\"\n",
                " EFFECT_RECRUIT_CHARACTER:0 \"Recruit $NAME$ [Root.GetTag]\"\n",
                " EFFECT_ADD_ACE:0 \"Ace $NAME$/$SURNAME$/$CALLSIGN$ [Root.GetTag]\"\n",
                " EFFECT_SET_FACTION_LEADER:0 \"Lead $NAME$ [Root.GetTag]\"\n",
            ),
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 10,
                literal: b"synthetic_character".to_vec(),
            },
            LexicalTokenRow {
                token: 11,
                literal: b"synthetic_unrecruited".to_vec(),
            },
            LexicalTokenRow {
                token: 31,
                literal: b"political_power_gain".to_vec(),
            },
            LexicalTokenRow {
                token: 101,
                literal: b"synthetic_ideology".to_vec(),
            },
            LexicalTokenRow {
                token: 201,
                literal: b"synthetic_group".to_vec(),
            },
            LexicalTokenRow {
                token: 601,
                literal: b"synthetic_faction".to_vec(),
            },
            LexicalTokenRow {
                token: 602,
                literal: b"synthetic_bound".to_vec(),
            },
        ]));
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![
            ModifierDefinitionRow {
                index: 0,
                token: 31,
                category_mask: 1,
                flags: 2,
                precision: 2,
                name_key: b"POWER".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 7,
            country: 100,
            name_token_id: 10,
            name: b"Current name".to_vec(),
            token: b"synthetic_name".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(vec![CountryCharacterRow {
            country: 100,
            ordinal: 0,
            character: Some(7),
        }]));
    *fixture.frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(vec![CountryLeaderRoleRow {
            character: 7,
            country: 100,
            ideology: 101,
            current_ideology: Recorded::Value(Some(Some(101))),
        }]));
    *fixture.frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            id: 201,
            token: b"synthetic_group".to_vec(),
            is_ruling: 1,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::PartyLeaders.index()] =
        TableView::Complete(TableRows::PartyLeaders(vec![PartyLeaderRow {
            country: 100,
            party: 201,
            ordinal: 0,
            character: Some(7),
            role: Some(101),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::AdvisorRoles.index()] =
        TableView::Complete(TableRows::AdvisorRoles(vec![AdvisorRoleRow {
            character: 7,
            country: 100,
            ordinal: 0,
            idea: b"synthetic_advisor".to_vec(),
            slot: b"synthetic_slot".to_vec(),
            tooltip_keys: Recorded::NotRecorded,
            modifier_cache: Recorded::Value(Some(AdvisorModifierCache {
                entries: vec![CachedModifier {
                    definition: 0,
                    value: 20_000,
                }],
            })),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::AvailableAdvisors.index()] =
        TableView::Complete(TableRows::AvailableAdvisors(vec![AvailableAdvisorRow {
            country: 100,
            ordinal: 0,
            character: 7,
            idea: b"synthetic_advisor".to_vec(),
        }]));
    crate::effect_description::characters::tests::empty_variable_stores(&mut fixture);
    (fixture, folder)
}

fn country_scopes() -> EventScopes {
    EventScopes::new(vec![EventScope::of(EventObject::Country(1))])
}

fn character_scopes() -> EventScopes {
    let mut character = EventScope::of(EventObject::Character(7));
    character.root = Link::Scope(1);
    EventScopes::new(vec![character, EventScope::of(EventObject::Country(1))])
}

fn render(fixture: &Fixture, source: &str, scopes: &mut EventScopes) -> Description {
    let cache = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    fixture.render(source, scopes, cache.view(None))
}

fn text(source: &str, expected: &str) {
    let (fixture, _folder) = fixture();
    let out = render(&fixture, source, &mut country_scopes());
    assert_eq!(out.text, expected);
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn create_country_leader() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "create_country_leader={traits={} expire=2037.6.23 name=\"Archived Label\" ideology=synthetic_ideology picture=GFX_synthetic desc=synthetic_desc}",
        "Promote Catalogued predecessor/Group [Root.GetTag]\n",
    );
}

#[test]
fn add_country_leader_role() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "add_country_leader_role={character=synthetic_character country_leader={expire=2043.11.9.5 ideology=synthetic_ideology}}",
        "Role Current name/Group [Root.GetTag]\n",
    );
}

#[test]
fn promoted_role_preserves_traits_and_no_scoped_expansion() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "add_country_leader_role={character=synthetic_character promote_leader=yes country_leader={ideology=synthetic_ideology traits={synthetic_trait}}}",
        "Promote Current name/Group [Root.GetTag]\n    \u{11}HTrait\u{11}!\n",
    );
}

#[test]
fn country_role_description_keeps_trait_lines_and_core_children() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let out = render(
        &fixture,
        "add_country_leader_role={character=synthetic_character country_leader={ideology=synthetic_ideology traits={synthetic_numeric}} army_experience=2}",
        &mut country_scopes(),
    );
    assert_eq!(
        out.text,
        "Role Current name/Group [Root.GetTag]\n    \u{11}HNumeric\u{11}!\n      - Power: \u{11}G+0.10\u{11}!\n\nXP2\n",
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn promote_character() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "promote_character={ideology=synthetic_ideology character=synthetic_character}",
        "Promote Current name/Group [Root.GetTag]\n",
    );
}

#[test]
fn promote_character_uses_character_scope_reader_fields() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    if let TableView::Complete(TableRows::Characters(rows)) =
        &mut *fixture.frame.tables[TableId::Characters.index()]
    {
        rows.push(CharacterRow {
            id: 8,
            country: 100,
            name_token_id: 11,
            name: b"Other character".to_vec(),
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::CountryCharacters(rows)) =
        &mut *fixture.frame.tables[TableId::CountryCharacters.index()]
    {
        rows.push(CountryCharacterRow {
            country: 100,
            ordinal: 1,
            character: Some(8),
        });
    }
    if let TableView::Complete(TableRows::CountryLeaderRoles(rows)) =
        &mut *fixture.frame.tables[TableId::CountryLeaderRoles.index()]
    {
        rows.push(CountryLeaderRoleRow {
            character: 8,
            country: 100,
            ideology: 101,
            current_ideology: Recorded::Value(Some(Some(101))),
        });
    }
    for (body, expected) in [
        ("promote_character=missing_ideology", ""),
        (
            "promote_character=yes",
            "Promote Current name/Group [Root.GetTag]\n",
        ),
        (
            "promote_character={character=synthetic_unrecruited ideology=synthetic_ideology}",
            "Promote Current name/Group [Root.GetTag]\n",
        ),
    ] {
        let out = render(&fixture, body, &mut character_scopes());
        assert_eq!(out.text, expected, "{body}");
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn promote_scalar_and_template_fallback_keep_single_role_gate() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "promote_character=synthetic_character",
        "Promote Current name/Group [Root.GetTag]\n",
    );
    text(
        "promote_character={character=synthetic_unrecruited ideology=synthetic_ideology}",
        "Promote Unrecruited [Root.GetTag]/Group [Root.GetTag]\n",
    );
    text(
        "promote_character=synthetic_unrecruited",
        "Promote Unrecruited [Root.GetTag]/ [Root.GetTag]\n",
    );
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::CountryLeaderRoles.index()] =
        TableView::Complete(TableRows::CountryLeaderRoles(vec![
            CountryLeaderRoleRow {
                character: 7,
                country: 100,
                ideology: 101,
                current_ideology: Recorded::Value(Some(Some(101))),
            },
            CountryLeaderRoleRow {
                character: 7,
                country: 100,
                ideology: 102,
                current_ideology: Recorded::Value(Some(Some(102))),
            },
        ]));
    let out = render(
        &fixture,
        "promote_character=synthetic_character",
        &mut country_scopes(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn template_promote_names_the_unrecorded_tag_map_winner() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let out = render(
        &fixture,
        "promote_character={character=synthetic_split_name ideology=synthetic_ideology}",
        &mut country_scopes(),
    );
    assert_eq!(out.text, "");
    assert_eq!(
        out.refusals,
        BTreeMap::from([(
            "synthetic effect promote_character: key promote_character: character template synthetic_split_name selected identity/presence is unrecorded".into(),
            1,
        )])
    );
}

#[test]
fn agreeing_conditional_templates_do_not_prove_tag_presence() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let out = render(
        &fixture,
        "promote_character={character=synthetic_conditional ideology=synthetic_ideology}",
        &mut country_scopes(),
    );
    assert_eq!(out.text, "");
    assert_eq!(
        out.refusals,
        BTreeMap::from([(
            "synthetic effect promote_character: key promote_character: character template synthetic_conditional selected identity/presence is unrecorded".into(),
            1,
        )])
    );
}

#[test]
fn retire_character() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "retire_character=synthetic_character",
        "Retire Current name/Alpha AAA\n",
    );
}

#[test]
fn retire_country_leader() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "retire_country_leader=yes",
        "Retire leader Current name AAA\n",
    );
}

#[test]
fn kill_country_leader() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text("kill_country_leader=yes", "Kill leader Current name AAA\n");
}

#[test]
fn remove_country_leader_role() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "remove_country_leader_role={ideology=synthetic_ideology character=synthetic_character}",
        "Remove role Current name/Group AAA",
    );
}

#[test]
fn retire_ideology_leader() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "retire_ideology_leader=synthetic_group",
        "Retire leader Current name AAA\n",
    );
}

#[test]
fn kill_ideology_leader() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "kill_ideology_leader=synthetic_group",
        "Kill leader Current name AAA\n",
    );
}

#[test]
fn add_advisor_role() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "add_advisor_role={activate=no advisor={idea_token=synthetic_advisor traits={synthetic_trait} cost=83 can_be_fired=yes slot=synthetic_slot} character=synthetic_character}",
        "Add advisor Current name/Slot AAA/ AAA",
    );
}

#[test]
fn advisor_role_sums_trait_and_authored_modifiers_before_formatting() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "add_advisor_role={character=synthetic_character advisor={slot=synthetic_slot traits={synthetic_numeric} modifier={political_power_gain=0.2}} activate=yes}",
        "Appoint Current name/Slot AAA/Power: \u{11}G+0.30\u{11}! AAA",
    );
}

#[test]
fn advisor_role_includes_the_high_category_bit() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
        &mut *fixture.frame.tables[TableId::ModifierDefinitions.index()]
    else {
        unreachable!()
    };
    definitions[0].category_mask = 1 << 31;
    let out = render(
        &fixture,
        "add_advisor_role={character=synthetic_character advisor={slot=synthetic_slot modifier={political_power_gain=0.1}}}",
        &mut country_scopes(),
    );
    assert_eq!(
        out.text,
        "Add advisor Current name/Slot AAA/Power: \u{11}G+0.10\u{11}! AAA",
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn advisor_role_expands_modifiers_only_after_wrapper_substitution() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    let mut localisation = fixture
        .catalogue
        .localisation
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain(
            [
                ("ADVISOR_OPEN_SCOPE", "[Root.GetTag"),
                (
                    "EFFECT_ADD_ADVISOR_ROLE",
                    "Add advisor $NAME$/$SLOT$/$MODIFIER$] [Root.GetTag]",
                ),
            ]
            .map(|(key, value)| (key.into(), value.into())),
        )
        .collect::<content_reader_localisation::localisation::Localisation>();
    localisation.language = fixture.catalogue.localisation.language.clone();
    fixture.catalogue.localisation = localisation.into();
    let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
        &mut *fixture.frame.tables[TableId::ModifierDefinitions.index()]
    else {
        unreachable!()
    };
    definitions[0].postfix_key = b"ADVISOR_OPEN_SCOPE".to_vec();
    let out = render(
        &fixture,
        "add_advisor_role={character=synthetic_character advisor={slot=synthetic_slot modifier={political_power_gain=0.2}}}",
        &mut country_scopes(),
    );
    assert_eq!(
        out.text,
        "Add advisor Current name/Slot AAA/Power: \u{11}G+0.20\u{11}! AAA AAA",
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn remove_advisor_role() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let out = render(
        &fixture,
        "remove_advisor_role={slot=synthetic_slot}",
        &mut character_scopes(),
    );
    assert_eq!(out.text, "Remove advisor Current name/Slot AAA AAA");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn set_can_be_fired_in_advisor_role() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "set_can_be_fired_in_advisor_role={value=yes character=synthetic_character}",
        "Fire Current name /AAA",
    );
}

#[test]
fn current_character_omission_and_slot_parentheses_match_inverted_guard() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let out = render(
        &fixture,
        "set_can_be_fired_in_advisor_role={value=no slot=synthetic_slot}",
        &mut character_scopes(),
    );
    assert_eq!(out.text, "Keep /(Slot AAA) AAA");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn advisor_own_block_tooltips_refuse_both_lines_without_guessing_refresh_history() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (modifier, refuses) in [
        ("", false),
        ("political_power_gain=0.47", false),
        ("custom_modifier_tooltip=SYN_TIP", true),
        ("added_modifier={custom_modifier_tooltip=SYN_TIP}", true),
    ] {
        let (fixture, _folder) = fixture_with_advisor_modifier(modifier);
        for (key, effect_key, expected) in [
            (
                "activate_advisor",
                "activate_advisor",
                "Activate Current name/Power: \u{11}G+0.20\u{11}! AAA",
            ),
            (
                "deactivate_advisor",
                "deactivate_advisor",
                "Deactivate Current name/Power: \u{11}G+0.20\u{11}! AAA",
            ),
        ] {
            let out = render(
                &fixture,
                &format!("{key}=synthetic_advisor"),
                &mut country_scopes(),
            );
            if refuses {
                assert_eq!(out.text, "", "{key}/{modifier}");
                assert_eq!(
                    out.refusals,
                    BTreeMap::from([(
                        format!(
                            "synthetic effect {effect_key}: key {key}: advisor own-block custom tooltip presence depends on refresh history"
                        ),
                        1,
                    )])
                );
            } else {
                assert_eq!(out.text, expected, "{key}/{modifier}");
                assert!(
                    out.refusals.is_empty(),
                    "{key}/{modifier}: {:?}",
                    out.refusals
                );
            }
        }
    }
}

#[test]
fn advisor_cache_not_recorded_refuses_each_class() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    let TableView::Complete(TableRows::AdvisorRoles(rows)) =
        &mut *fixture.frame.tables[TableId::AdvisorRoles.index()]
    else {
        unreachable!()
    };
    rows[0].modifier_cache = Recorded::NotRecorded;
    for key in ["activate_advisor", "deactivate_advisor"] {
        let out = render(
            &fixture,
            &format!("{key}=synthetic_advisor"),
            &mut country_scopes(),
        );
        assert_eq!(out.text, "", "{key}");
        assert_eq!(
            out.refusals,
            BTreeMap::from([(
                format!("synthetic effect {key}: key {key}: advisor modifier cache not recorded"),
                1,
            )]),
            "{key}",
        );
    }
}

#[test]
fn complete_rosters_prove_absent_advisor_is_empty() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let out = render(
        &fixture,
        "activate_advisor=absent_advisor deactivate_advisor=absent_advisor",
        &mut country_scopes(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn recruit_character() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "recruit_character=synthetic_character",
        "Recruit Current name AAA\n",
    );
}

#[test]
fn add_ace() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    text(
        "add_ace={callsign=Firefly name=Mira type=synthetic_ace surname=Vale}",
        "Ace Mira/Vale/Firefly AAA",
    );
}

fn faction_fixture(name: &[u8], template_token: Option<u32>) -> (Fixture, Folder) {
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 5,
            name: name.to_vec(),
            template_token,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 200,
                faction: 5,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 100,
                faction: 5,
                ordinal: 1,
            },
        ]));
    (fixture, _folder)
}

#[test]
fn set_faction_leader() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = faction_fixture(b"Recorded faction", None);
    let out = render(&fixture, "set_faction_leader=yes", &mut country_scopes());
    assert_eq!(out.text, "Lead Recorded faction AAA\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn empty_faction_name_uses_bound_template_with_empty_supplied_arguments() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (token, expected) in [
        (Some(601), "Lead Template AAA AAA\n"),
        (Some(602), "Lead Root Nested Word AAA\n"),
        (None, "Lead  AAA\n"),
    ] {
        let (fixture, _folder) = faction_fixture(b"", token);
        let out = render(&fixture, "set_faction_leader=yes", &mut country_scopes());
        assert_eq!(out.text, expected);
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn empty_display_name_uses_recorded_original_key() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 7,
            country: 100,
            name_token_id: 10,
            token: b"synthetic_name".to_vec(),
            ..Default::default()
        }]));
    let out = render(&fixture, "retire_country_leader=yes", &mut country_scopes());
    assert_eq!(out.text, "Retire leader Content name AAA\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn advisor_lookup_without_character_roster_join() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *fixture.frame.tables[TableId::CountryLinks.index()]
    {
        rows.extend((0..24).map(|id| CountryLinkRow {
            id: 10_000 + id,
            canonical_slot: 10 + id,
        }));
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *fixture.frame.tables[TableId::Countries.index()]
    {
        rows.extend((0..24).map(|id| CountryRow {
            id: 10_000 + id,
            canonical_slot: 10 + id,
            tag: format!("S{id}").into_bytes(),
            ..Default::default()
        }));
    }
    let TableView::Complete(TableRows::AdvisorRoles(existing)) =
        &*fixture.frame.tables[TableId::AdvisorRoles.index()]
    else {
        unreachable!()
    };
    let selected = existing[0].clone();
    *fixture.frame.tables[TableId::AdvisorRoles.index()] =
        TableView::Complete(TableRows::AdvisorRoles(
            std::iter::once(selected)
                .chain((0..24).map(|id| AdvisorRoleRow {
                    character: 10_000 + id,
                    country: 10_000 + id,
                    idea: b"synthetic_advisor".to_vec(),
                    ..Default::default()
                }))
                .collect(),
        ));
    *fixture.frame.tables[TableId::AvailableAdvisors.index()] =
        TableView::Complete(TableRows::AvailableAdvisors(
            std::iter::once(AvailableAdvisorRow {
                country: 100,
                ordinal: 0,
                character: 7,
                idea: b"synthetic_advisor".to_vec(),
            })
            .chain((0..24).map(|id| AvailableAdvisorRow {
                country: 10_000 + id,
                ordinal: 0,
                character: 10_000 + id,
                idea: b"synthetic_advisor".to_vec(),
            }))
            .collect(),
        ));
    *fixture.frame.tables[TableId::CountryCharacters.index()] = TableView::Unavailable {
        reason: "unused primary character roster".into(),
    };
    let out = render(
        &fixture,
        "activate_advisor=synthetic_advisor deactivate_advisor=absent_advisor",
        &mut country_scopes(),
    );
    assert_eq!(
        out.text,
        "Activate Current name/Power: \u{11}G+0.20\u{11}! AAA"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn advisor_roster_and_cache_capture_failures_are_named() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    if let TableView::Complete(TableRows::AdvisorRoles(rows)) =
        &mut *fixture.frame.tables[TableId::AdvisorRoles.index()]
    {
        rows[0].modifier_cache = Recorded::Value(None);
    }
    let out = render(
        &fixture,
        "activate_advisor=synthetic_advisor",
        &mut country_scopes(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect activate_advisor: key activate_advisor: advisor modifier cache capture unavailable".into(), 1)]));
    *fixture.frame.tables[TableId::AvailableAdvisors.index()] = TableView::Unavailable {
        reason: "format 97 lacks available advisors".into(),
    };
    let out = render(
        &fixture,
        "activate_advisor=synthetic_advisor",
        &mut country_scopes(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect activate_advisor: key activate_advisor: hour 1 AvailableAdvisors is unavailable: format 97 lacks available advisors".into(), 1)]));
}

#[test]
fn advisor_primary_order_precedes_retired_case_folded_lookup() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    if let TableView::Complete(TableRows::Characters(rows)) =
        &mut *fixture.frame.tables[TableId::Characters.index()]
    {
        rows.push(CharacterRow {
            id: 8,
            name: b"Retired name".to_vec(),
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::AdvisorRoles(rows)) =
        &mut *fixture.frame.tables[TableId::AdvisorRoles.index()]
    {
        let mut retired = rows[0].clone();
        retired.character = 8;
        rows.push(retired);
    }
    *fixture.frame.tables[TableId::AvailableAdvisors.index()] =
        TableView::Complete(TableRows::AvailableAdvisors(vec![
            AvailableAdvisorRow {
                country: 100,
                ordinal: 0,
                character: 7,
                idea: b"synthetic_advisor".to_vec(),
            },
            AvailableAdvisorRow {
                country: 100,
                ordinal: 9,
                character: 8,
                idea: b"synthetic_advisor".to_vec(),
            },
        ]));
    *fixture.frame.tables[TableId::CountryRetiredCharacters.index()] =
        TableView::Complete(TableRows::CountryRetiredCharacters(vec![
            CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(8),
            },
        ]));
    if let TableView::Complete(TableRows::ModifierDefinitions(rows)) =
        &mut *fixture.frame.tables[TableId::ModifierDefinitions.index()]
    {
        rows[0].category_mask = 1 << 31;
    }
    for (token, name) in [
        ("synthetic_advisor", "Current name"),
        ("SYNTHETIC_ADVISOR", "Retired name"),
    ] {
        let out = render(
            &fixture,
            &format!("activate_advisor={token}"),
            &mut country_scopes(),
        );
        assert_eq!(
            out.text,
            format!("Activate {name}/Power: \u{11}G+0.20\u{11}! AAA")
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn advisor_cache_role_requires_unambiguous_pointer_identity() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    for second_idea in ["synthetic_advisor", "SYNTHETIC_ADVISOR"] {
        let (mut fixture, _folder) = fixture();
        if let TableView::Complete(TableRows::AdvisorRoles(rows)) =
            &mut *fixture.frame.tables[TableId::AdvisorRoles.index()]
        {
            let mut second = rows[0].clone();
            second.ordinal = 4;
            second.idea = second_idea.as_bytes().to_vec();
            second.slot = b"synthetic_other_slot".to_vec();
            second.modifier_cache = Recorded::Value(Some(AdvisorModifierCache {
                entries: vec![CachedModifier {
                    definition: 0,
                    value: 90_000,
                }],
            }));
            rows.insert(0, second);
        }
        *fixture.frame.tables[TableId::CountryRetiredCharacters.index()] =
            TableView::Complete(TableRows::CountryRetiredCharacters(vec![
                CountryCharacterRow {
                    country: 100,
                    ordinal: 0,
                    character: Some(7),
                },
            ]));
        for (key, effect_key, expected) in [
            (
                "activate_advisor",
                "activate_advisor",
                "Activate Current name/Power: \u{11}G+0.20\u{11}! AAA",
            ),
            (
                "deactivate_advisor",
                "deactivate_advisor",
                "Deactivate Current name/Power: \u{11}G+0.20\u{11}! AAA",
            ),
        ] {
            let out = render(
                &fixture,
                &format!("{key}={second_idea}"),
                &mut country_scopes(),
            );
            assert_eq!(out.text, "");
            assert_eq!(
                out.refusals,
                BTreeMap::from([(
                    format!(
                        "synthetic effect {effect_key}: key {key}: advisor 7 / {second_idea} has ambiguous role identity after character-only lookup"
                    ),
                    1,
                )])
            );
            let out = render(
                &fixture,
                &format!("{key}=SyNtHeTiC_AdViSoR"),
                &mut country_scopes(),
            );
            assert_eq!(out.text, expected);
            assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        }
    }
}

#[test]
fn advisor_single_tooltip_is_derivable_but_several_need_recorded_slot_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    if let TableView::Complete(TableRows::ModifierDefinitions(rows)) =
        &mut *fixture.frame.tables[TableId::ModifierDefinitions.index()]
    {
        rows.push(ModifierDefinitionRow {
            index: 1,
            token: 32,
            ..rows[0].clone()
        });
    }
    if let TableView::Complete(TableRows::AdvisorRoles(rows)) =
        &mut *fixture.frame.tables[TableId::AdvisorRoles.index()]
    {
        rows[0].modifier_cache = Recorded::Value(Some(AdvisorModifierCache {
            entries: vec![
                CachedModifier {
                    definition: 1,
                    value: 30_000,
                },
                CachedModifier {
                    definition: 0,
                    value: 20_000,
                },
            ],
        }));
    }
    let traits = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .environment()
        .unwrap()
        .leader_traits()
        .unwrap();
    let single_tooltip = traits.first_ordinal("synthetic_tip").unwrap().unwrap();
    let several_tooltips = traits.first_ordinal("synthetic_two_tips").unwrap().unwrap();
    let definition = single_tooltip;
    *fixture.frame.tables[TableId::CharacterRoleTraits.index()] =
        TableView::Complete(TableRows::CharacterRoleTraits(vec![
            CharacterRoleTraitRow {
                character: 7,
                kind: CharacterRoleTraitRow::ADVISOR,
                role: 0,
                tokens: vec![999],
                definitions: vec![definition],
            },
        ]));
    for (key, effect_key) in [
        ("activate_advisor", "activate_advisor"),
        ("deactivate_advisor", "deactivate_advisor"),
    ] {
        let out = render(
            &fixture,
            &format!("{key}=synthetic_advisor"),
            &mut country_scopes(),
        );
        assert_eq!(
            out.text,
            format!(
                "{} Current name/Power: \u{11}G+0.30\u{11}!\nPower: \u{11}G+0.20\u{11}!\nTip AAA AAA",
                if key == "activate_advisor" {
                    "Activate"
                } else {
                    "Deactivate"
                }
            )
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        let definition = several_tooltips;
        if let TableView::Complete(TableRows::CharacterRoleTraits(rows)) =
            &mut *fixture.frame.tables[TableId::CharacterRoleTraits.index()]
        {
            rows[0].definitions[0] = definition;
        }
        let out = render(
            &fixture,
            &format!("{key}=synthetic_advisor"),
            &mut country_scopes(),
        );
        assert_eq!(out.text, "");
        assert_eq!(
            out.refusals,
            BTreeMap::from([(
                format!(
                    "synthetic effect {effect_key}: key {key}: advisor cached tooltip keys/slot order not recorded"
                ),
                1
            )])
        );
        let definition = single_tooltip;
        if let TableView::Complete(TableRows::CharacterRoleTraits(rows)) =
            &mut *fixture.frame.tables[TableId::CharacterRoleTraits.index()]
        {
            rows[0].definitions[0] = definition;
        }
    }
}

#[test]
fn format99_advisor_cache_keeps_stored_tooltip_order_and_old_format_fallback() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    let TableView::Complete(TableRows::AdvisorRoles(rows)) =
        &mut *fixture.frame.tables[TableId::AdvisorRoles.index()]
    else {
        unreachable!()
    };
    rows[0].modifier_cache = Recorded::Value(Some(AdvisorModifierCache {
        entries: vec![CachedModifier {
            definition: 0,
            value: 10_000,
        }],
    }));
    rows[0].tooltip_keys = Recorded::Value(Some(vec![b"CACHE_B".to_vec(), b"CACHE_A".to_vec()]));
    for (effect, expected) in [
        ("activate_advisor", "Activate"),
        ("deactivate_advisor", "Deactivate"),
    ] {
        let out = render(
            &fixture,
            &format!("{effect}=synthetic_advisor"),
            &mut country_scopes(),
        );
        assert_eq!(
            out.text,
            format!("{expected} Current name/Power: \u{11}G+0.10\u{11}!\nSecond\nFirst AAA AAA")
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
    let TableView::Complete(TableRows::AdvisorRoles(rows)) =
        &mut *fixture.frame.tables[TableId::AdvisorRoles.index()]
    else {
        unreachable!()
    };
    rows[0].tooltip_keys = Recorded::NotRecorded;
    let out = render(
        &fixture,
        "activate_advisor=synthetic_advisor",
        &mut country_scopes(),
    );
    assert_eq!(
        out.text,
        "Activate Current name/Power: \u{11}G+0.10\u{11}! AAA"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn recorded_advisor_tooltip_keys_override_authored_blocks_and_failed_captures_refuse() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture_with_advisor_modifier("custom_modifier_tooltip=SYN_TIP");
    *fixture.frame.tables[TableId::CharacterRoleTraits.index()] = TableView::Unavailable {
        reason: "unused by recorded tooltip cache".into(),
    };
    for (keys, expected) in [
        (
            Recorded::Value(Some(Vec::new())),
            Some("Power: \u{11}G+0.20\u{11}!"),
        ),
        (
            Recorded::Value(Some(vec![b"CACHE_A\0ignored".to_vec()])),
            Some("Power: \u{11}G+0.20\u{11}!\nFirst AAA"),
        ),
        (Recorded::Value(None), None),
    ] {
        let TableView::Complete(TableRows::AdvisorRoles(rows)) =
            &mut *fixture.frame.tables[TableId::AdvisorRoles.index()]
        else {
            unreachable!()
        };
        rows[0].tooltip_keys = keys;
        for (key, effect_key, verb) in [
            ("activate_advisor", "activate_advisor", "Activate"),
            ("deactivate_advisor", "deactivate_advisor", "Deactivate"),
        ] {
            let out = render(
                &fixture,
                &format!("{key}=synthetic_advisor"),
                &mut country_scopes(),
            );
            if let Some(description) = expected {
                assert_eq!(out.text, format!("{verb} Current name/{description} AAA"));
                assert!(out.refusals.is_empty(), "{:?}", out.refusals);
            } else {
                assert!(out.text.is_empty());
                assert_eq!(
                    out.refusals,
                    BTreeMap::from([(
                        format!(
                            "synthetic effect {effect_key}: key {key}: tooltip keys of advisor roles capture unavailable"
                        ),
                        1,
                    )])
                );
            }
        }
    }
}
