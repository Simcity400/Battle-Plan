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
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={\"synthetic_slot\"}}}",
        ),
        (
            "common/ideologies/test.txt",
            "ideologies={ synthetic_group={types={synthetic_ideology={}}} }",
        ),
        (
            "common/country_leader/test.txt",
            "leader_traits={synthetic_trait={} synthetic_numeric={political_power_gain=0.1}}",
        ),
        (
            "common/characters/test.txt",
            "characters={synthetic_character={name=synthetic_name country_leader={ideology=synthetic_ideology traits={synthetic_trait}} advisor={slot=synthetic_slot idea_token=synthetic_advisor traits={synthetic_trait}}} synthetic_unrecruited={name=unrecruited_name country_leader={ideology=synthetic_ideology}} synthetic_conditional={instance={allowed={always=no} name=conditional_name country_leader={ideology=synthetic_ideology}} instance={allowed={always=no} name=conditional_name country_leader={ideology=synthetic_ideology}}} synthetic_split_name={instance={allowed={always=yes} name=first country_leader={ideology=synthetic_ideology}} instance={allowed={always=yes} name=second country_leader={ideology=synthetic_ideology}}}}",
        ),
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
                " POWER_WITH_ARG:0 \"Power $ARG$\"\n ARG:0 \"Argument fallback\"\n",
                " synthetic_faction_name:0 \"Template [Root.GetTag]\"\n synthetic_faction_root:0 \"Root $PART$\"\n synthetic_faction_nested:0 \"Nested $WORD$\"\n synthetic_word:0 \"Word\"\n",
                " AAA_legacy_name:0 \"Localized legacy\"\n synthetic_name:0 \"Content name\"\n unrecruited_name:0 \"Unrecruited [Root.GetTag]\"\n",
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
            ..Default::default()
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

// common/decisions/ENG.txt:3198, authored legacy name/ideology/traits block.
#[test]
fn create_country_leader() {
    text(
        "create_country_leader={name=\"Legacy Name\" desc=synthetic_desc picture=GFX_synthetic expire=1965.1.1 ideology=synthetic_ideology traits={}}",
        "Promote Localized legacy/Group [Root.GetTag]\n",
    );
}

// common/decisions/AST.txt:1534, role with explicit character and ideology.
#[test]
fn add_country_leader_role() {
    text(
        "add_country_leader_role={character=synthetic_character country_leader={ideology=synthetic_ideology expire=1989.1.1.1}}",
        "Role Current name/Group [Root.GetTag]\n",
    );
}

#[test]
fn promoted_role_preserves_traits_and_no_scoped_expansion() {
    text(
        "add_country_leader_role={character=synthetic_character promote_leader=yes country_leader={ideology=synthetic_ideology traits={synthetic_trait}}}",
        "Promote Current name/Group [Root.GetTag]\n    \u{11}HTrait\u{11}!\n",
    );
}

#[test]
fn country_role_refusal_keeps_core_children_and_own_projection() {
    let (fixture, _folder) = fixture();
    let out = render(
        &fixture,
        "add_country_leader_role={character=synthetic_character country_leader={ideology=synthetic_ideology traits={synthetic_numeric}} army_experience=2}",
        &mut country_scopes(),
    );
    assert_eq!(out.text, "XP2\n"); // 1403AEC00 own fields never enter the child array.
    assert_eq!(out.refusals, BTreeMap::from([(
        "synthetic effect CAddCountryLeaderRoleEffect: key add_country_leader_role: native plain modifier description requires core plain/mask/separator/show-zero parameters (14054E050/14054F6D0)".into(),
        1,
    )]));
}

// common/bop/ITA.txt:158, explicit character/ideology block.
#[test]
fn promote_character() {
    text(
        "promote_character={character=synthetic_character ideology=synthetic_ideology}",
        "Promote Current name/Group [Root.GetTag]\n",
    );
}

// 1403AA3C0 scope-kind 8: scalar is ideology, exact yes is empty,
// and a block's character key is rejected instead of replacing THIS.
#[test]
fn promote_character_uses_character_scope_reader_fields() {
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
        });
    }
    for (body, expected) in [
        ("promote_character=missing_ideology", ""),
        ("promote_character=yes", "Promote Current name/Group [Root.GetTag]\n"),
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
fn promote_scalar_and_template_fallback_keep_native_single_role_gate() {
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
            },
            CountryLeaderRoleRow {
                character: 7,
                country: 100,
                ideology: 102,
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

// 140536D20/1406B0920 select the tag-map winner, not a runtime holder's
// template. 140F8B8A0 can copy an older instance into a later character ID.
#[test]
fn template_promote_names_the_unrecorded_tag_map_winner() {
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
            "synthetic effect CPromoteCharacterToCountryLeader: key promote_character: character template synthetic_split_name selected identity/presence (CCharacterTemplateDatabase+0x40/+0x60 tag entry+0x10, 1406B0920/1406ACC90) is unrecorded".into(),
            1,
        )])
    );
}

// 1406ACC90 clears the maps and only registers startup-allowed templates.
#[test]
fn agreeing_conditional_templates_do_not_prove_tag_presence() {
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
            "synthetic effect CPromoteCharacterToCountryLeader: key promote_character: character template synthetic_conditional selected identity/presence (CCharacterTemplateDatabase+0x40/+0x60 tag entry+0x10, 1406B0920/1406ACC90) is unrecorded".into(),
            1,
        )])
    );
}

// common/decisions/BUL.txt:4079, scalar character selector.
#[test]
fn retire_character() {
    text(
        "retire_character=synthetic_character",
        "Retire Current name/Alpha AAA\n",
    );
}

// common/decisions/AST.txt:1903, authored yes is execution-only.
#[test]
fn retire_country_leader() {
    text(
        "retire_country_leader=yes",
        "Retire leader Current name AAA\n",
    );
}

// common/decisions/INS.txt:3500.
#[test]
fn kill_country_leader() {
    text("kill_country_leader=yes", "Kill leader Current name AAA\n");
}

// common/decisions/CZE.txt:967, subgroup ideology maps to group.
#[test]
fn remove_country_leader_role() {
    text(
        "remove_country_leader_role={character=synthetic_character ideology=synthetic_ideology}",
        "Remove role Current name/Group AAA",
    );
}

// common/decisions/BUL.txt:10020, ideology selector is the group.
#[test]
fn retire_ideology_leader() {
    text(
        "retire_ideology_leader=synthetic_group",
        "Retire leader Current name AAA\n",
    );
}

// events/BFTB_Bulgaria.txt:9067.
#[test]
fn kill_ideology_leader() {
    text(
        "kill_ideology_leader=synthetic_group",
        "Kill leader Current name AAA\n",
    );
}

// common/national_focus/afghanistan.txt:9354, authored role and activate=no.
#[test]
fn add_advisor_role() {
    text(
        "add_advisor_role={character=synthetic_character advisor={slot=synthetic_slot cost=150 can_be_fired=yes idea_token=synthetic_advisor traits={synthetic_trait}} activate=no}",
        "Add advisor Current name/Slot AAA/ AAA",
    );
}

#[test]
fn advisor_role_plain_description_requires_core_options() {
    let (fixture, _folder) = fixture();
    let out = render(
        &fixture,
        "add_advisor_role={character=synthetic_character advisor={slot=synthetic_slot traits={synthetic_numeric} modifier={political_power_gain=0.2}} activate=yes}",
        &mut country_scopes(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([(
        "synthetic effect CAddAdvisorRoleEffect: key add_advisor_role: native plain modifier description requires core plain/mask/separator/show-zero parameters (14054E050/14054F6D0)".into(),
        1,
    )]));
}

// common/decisions/GER.txt:16992, no explicit character in character scope.
#[test]
fn remove_advisor_role() {
    let (fixture, _folder) = fixture();
    let out = render(
        &fixture,
        "remove_advisor_role={slot=synthetic_slot}",
        &mut character_scopes(),
    );
    assert_eq!(out.text, "Remove advisor Current name/Slot AAA AAA");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// common/national_focus/austria.txt:10513, explicit character, no slot.
#[test]
fn set_can_be_fired_in_advisor_role() {
    text(
        "set_can_be_fired_in_advisor_role={character=synthetic_character value=yes}",
        "Fire Current name /AAA",
    );
}

#[test]
fn current_character_omission_and_slot_parentheses_match_inverted_native_guard() {
    let (fixture, _folder) = fixture();
    let out = render(
        &fixture,
        "set_can_be_fired_in_advisor_role={value=no slot=synthetic_slot}",
        &mut character_scopes(),
    );
    assert_eq!(out.text, "Keep /(Slot AAA) AAA");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

fn advisor_gap(source: &str, class: &str) {
    let (fixture, _folder) = fixture();
    let out = render(&fixture, source, &mut country_scopes());
    assert_eq!(out.text, "");
    assert_eq!(
        out.refusals,
        BTreeMap::from([(
            format!(
                "synthetic effect {class}: key {}: advisor lookup requires the unrecorded available-advisor roster (CCountryCharacters+0x40/+0x4C, 1410D69C0) and cached modifiers (CAdvisor+0x1A0, 14036F000/14037E3B0)",
                source.split('=').next().unwrap()
            ),
            1
        )])
    );
}

// common/decisions/FIN.txt:2180, scalar idea token. Persistent roles
// cannot prove the available lookup roster or its cached modifier.
#[test]
fn activate_advisor() {
    advisor_gap(
        "activate_advisor=synthetic_advisor",
        "CActivateAdvisorEffect",
    );
}

// common/national_focus/czechoslovakia_mu.txt:5818.
#[test]
fn deactivate_advisor() {
    advisor_gap(
        "deactivate_advisor=synthetic_advisor",
        "CDeactivateAdvisorEffect",
    );
}

#[test]
fn absent_persistent_advisor_cannot_prove_absent_native_available_roster() {
    let (fixture, _folder) = fixture();
    let out = render(
        &fixture,
        "activate_advisor=absent_advisor deactivate_advisor=absent_advisor",
        &mut country_scopes(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([
        ("synthetic effect CActivateAdvisorEffect: key activate_advisor: advisor lookup requires the unrecorded available-advisor roster (CCountryCharacters+0x40/+0x4C, 1410D69C0) and cached modifiers (CAdvisor+0x1A0, 14036F000/14037E3B0)".into(), 1),
        ("synthetic effect CDeactivateAdvisorEffect: key deactivate_advisor: advisor lookup requires the unrecorded available-advisor roster (CCountryCharacters+0x40/+0x4C, 1410D69C0) and cached modifiers (CAdvisor+0x1A0, 14036F000/14037E3B0)".into(), 1),
    ]));
}

// Mod 3350890356 events/TFR_events_IRQ.txt:45, global token lookup.
#[test]
fn recruit_character() {
    text(
        "recruit_character=synthetic_character",
        "Recruit Current name AAA\n",
    );
}

// common/national_focus/austria.txt:12758, literal name/surname/callsign.
#[test]
fn add_ace() {
    text(
        "add_ace={name=First surname=Last callsign=Nick type=synthetic_ace}",
        "Ace First/Last/Nick AAA",
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

// common/national_focus/australia_taog.txt:2269.
#[test]
fn set_faction_leader() {
    let (fixture, _folder) = faction_fixture(b"Recorded faction", None);
    let out = render(&fixture, "set_faction_leader=yes", &mut country_scopes());
    assert_eq!(out.text, "Lead Recorded faction AAA\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn empty_faction_name_uses_bound_template_with_empty_supplied_arguments() {
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
fn advisor_refusal_does_not_join_five_thousand_country_rosters() {
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::AdvisorRoles.index()] =
        TableView::Complete(TableRows::AdvisorRoles((0..5_000).map(|id| AdvisorRoleRow {
            character: id,
            country: id,
            idea: b"synthetic_advisor".to_vec(),
            ..Default::default()
        }).collect()));
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters((0..5_000).map(|id| CountryCharacterRow {
            country: id,
            ordinal: 0,
            character: Some(id),
        }).collect()));
    let out = render(&fixture, "activate_advisor=synthetic_advisor deactivate_advisor=absent_advisor", &mut country_scopes());
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([
        ("synthetic effect CActivateAdvisorEffect: key activate_advisor: advisor lookup requires the unrecorded available-advisor roster (CCountryCharacters+0x40/+0x4C, 1410D69C0) and cached modifiers (CAdvisor+0x1A0, 14036F000/14037E3B0)".into(), 1),
        ("synthetic effect CDeactivateAdvisorEffect: key deactivate_advisor: advisor lookup requires the unrecorded available-advisor roster (CCountryCharacters+0x40/+0x4C, 1410D69C0) and cached modifiers (CAdvisor+0x1A0, 14036F000/14037E3B0)".into(), 1),
    ]));
}
