use super::*;
use crate::catalogue::tests::{Folder, folder_catalogue};
use crate::effect_description::tests::Fixture;
use crate::scripted_text::{EventObject, EventScope, Link};
use replay::{frame::TableView, rows::*};

fn fixture(category: i32) -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n AAA:0 \"Alpha\"\n",
                " EFFECT_CREATE_CC_LEADER:0 \"Corps $NAME$ [Root.GetTag]\"\n",
                " EFFECT_CREATE_FM_LEADER:0 \"Marshal $NAME$ [Root.GetTag]\"\n",
                " EFFECT_CREATE_NAVY_LEADER:0 \"Navy $NAME$ [Root.GetTag]\"\n",
                " EFFECT_CREATE_OPERATIVE_LEADER:0 \"Operative $NAME$ [Root.GetTag]\"\n",
                " EFFECT_PROMOTE_LEADER:0 \"Promote [Root.GetTag]\"\n",
                " EFFECT_DEMOTE_LEADER:0 \"Demote [Root.GetTag]\"\n",
                " EFFECT_REMOVE_CC_LEADER:0 \"Remove corps $NAME$ [Root.GetTag]\"\n",
                " EFFECT_REMOVE_FM_LEADER:0 \"Remove marshal $NAME$ [Root.GetTag]\"\n",
                " EFFECT_REMOVE_NAVY_LEADER:0 \"Remove navy $NAME$ [Root.GetTag]\"\n",
                " EFFECT_REMOVE_OPERATIVE_LEADER:0 \"Remove operative $NAME$/$CODENAME$ [Root.GetTag]\"\n",
                " EFFECT_REMOVE_UNIT_LEADER_ROLE:0 \"Remove role $NAME$/$TITLE$ [Root.GetTag]\"\n",
                " EFFECT_ADD_RANDOM_UNIT_TRAIT:0 \"Random trait for $NAME$\"\n",
                " fm_leader:0 \"Field Marshal\"\n cc_leader:0 \"General\"\n navy_leader:0 \"Admiral\"\n",
                " operative_skill_3:0 \"Expert Agent\"\n OPERATIVE:0 \"Agent\"\n",
                " synthetic_operative:0 \"Authored agent\"\n",
                " ADD_DIVISIONAL_COMMANDER_XP:0 \"Commander XP $VAL|2$ [Root.GetTag]\"\n",
                " EFFECT_ADD_RAID_HISTORY_ENTRY_UNIT_EFFECT:0 \"Raid history $NAME$ [Root.GetTag]\"\n",
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
                name: b"Character name".to_vec(),
                token: b"synthetic_character".to_vec(),
                ..Default::default()
            },
            CharacterRow {
                id: 8,
                country: 100,
                name_token_id: 11,
                name: b"Other character".to_vec(),
                token: b"AAA_".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 10,
                literal: b"synthetic_character".to_vec(),
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
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 5,
            reference_type: 55,
            character_id: 7,
            codename: Recorded::NotRecorded,
            category,
            inherited_text_b: b"Leader name".to_vec(),
            owner: 100,
            skill_status: 2,
            skill_valid: 1,
            skill_level: 3,
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

fn rendered(fixture: &Fixture, body: &str, expected: &str, character: bool) {
    let scopes = &mut if character {
        character_scopes()
    } else {
        country_scopes()
    };
    let out = fixture.render(body, scopes, Content::default());
    assert_eq!(out.text, expected, "{body}: {:?}", out.refusals);
    assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
}

// common/decisions/chi_decisions.txt:2435, optional character plus template fields.
#[test]
fn add_corps_commander_role() {
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "add_corps_commander_role={character=synthetic_character traits={} skill=3 attack_skill=3 defense_skill=3 planning_skill=2 logistics_skill=2}",
        "Corps Character name AAA",
        false,
    );
}

// common/decisions/tur.txt:26.
#[test]
fn add_field_marshal_role() {
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "add_field_marshal_role={character=synthetic_character traits={} skill=4 attack_skill=4 defense_skill=3 planning_skill=3 logistics_skill=3 legacy_id=59}",
        "Marshal Character name AAA",
        false,
    );
}

// common/national_focus/australia_taog.txt:1054, no explicit character.
#[test]
fn add_naval_commander_role() {
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "add_naval_commander_role={traits={} skill=4 attack_skill=2 defense_skill=3 coordination_skill=6 maneuvering_skill=2 legacy_id=-1}",
        "Navy Character name AAA",
        true,
    );
}

// common/decisions/hol.txt:2081. Authored names stay literal, even defined keys.
#[test]
fn create_corps_commander() {
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "create_corps_commander={name=synthetic_operative gfx=GFX_synthetic_portrait traits={} id=982 skill=2 attack_skill=2 defense_skill=1 planning_skill=1 logistics_skill=2}",
        "Corps synthetic_operative AAA",
        false,
    );
}

// common/national_focus/spain.txt:5705.
#[test]
fn create_field_marshal() {
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "create_field_marshal={name=\"Synthetic Marshal\" gfx=GFX_synthetic_portrait traits={synthetic_trait} skill=3 attack_skill=2 defense_skill=3 planning_skill=3 logistics_skill=2}",
        "Marshal Synthetic Marshal AAA",
        false,
    );
}

// common/national_focus/free_france.txt:505.
#[test]
fn create_navy_leader() {
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "create_navy_leader={name=\"Synthetic Admiral\" gfx=GFX_synthetic_portrait skill=3 traits={} attack_skill=1 defense_skill=2 maneuvering_skill=3 coordination_skill=4}",
        "Navy Synthetic Admiral AAA",
        false,
    );
}

#[test]
fn legacy_empty_name_uses_recorded_name_token_then_exact_stored_token() {
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "create_corps_commander={id=10 name=\"\"}",
        "Corps Character name AAA",
        false,
    );
    rendered(
        &fixture,
        "create_corps_commander={id=12}",
        "Corps Other character AAA",
        false,
    );
}

// common/decisions/_debug_decisions.txt:544.
#[test]
fn create_operative_leader() {
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "create_operative_leader={bypass_recruitment=yes}",
        "Operative  AAA",
        false,
    );
    rendered(
        &fixture,
        "create_operative_leader={name=synthetic_operative bypass_recruitment=yes traits={}}",
        "Operative Authored agent AAA",
        false,
    );
}

// common/decisions/cze.txt:1023, held scalar token and plain outer localisation.
#[test]
fn show_unit_leaders_tooltip() {
    let (mut fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "show_unit_leaders_tooltip=synthetic_character",
        "Corps Leader name [Root.GetTag]\n",
        false,
    );
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(vec![]));
    *fixture.frame.tables[TableId::CountryRetiredCharacters.index()] =
        TableView::Complete(TableRows::CountryRetiredCharacters(vec![
            CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(7),
            },
        ]));
    rendered(
        &fixture,
        "show_unit_leaders_tooltip=synthetic_character",
        "Corps Leader name [Root.GetTag]\n",
        false,
    );
    if let TableView::Complete(TableRows::Characters(rows)) =
        &mut *fixture.frame.tables[TableId::Characters.index()]
    {
        rows[0].token.clear();
    }
    rendered(
        &fixture,
        "show_unit_leaders_tooltip=synthetic_character",
        "",
        false,
    );
}

// common/decisions/ins.txt:1541. Boolean is execution-only.
#[test]
fn promote_leader() {
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "promote_leader=yes",
        "Promote [Root.GetTag]\n",
        true,
    );
    rendered(&fixture, "demote_leader=yes", "", true);
}

// events/aat_finland.txt:750.
#[test]
fn demote_leader() {
    let (fixture, _folder) = fixture(0);
    rendered(
        &fixture,
        "demote_leader=yes",
        "Demote [Root.GetTag]\n",
        true,
    );
    rendered(&fixture, "promote_leader=yes", "", true);
}

// Workshop 1137372539 common/national_focus/hungary.txt:7240; runtime character id.
#[test]
fn remove_unit_leader() {
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "remove_unit_leader=7",
        "Remove corps Leader name [Root.GetTag]",
        false,
    );
    rendered(
        &fixture,
        "remove_unit_leader=yes",
        "Remove corps Leader name [Root.GetTag]",
        true,
    );
    rendered(&fixture, "remove_unit_leader=201", "", false);
}

#[test]
fn remove_operative_refuses_codename_by_class_and_field() {
    let (fixture, _folder) = fixture(3);
    let out = fixture.render(
        "remove_unit_leader=yes",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([(
        "synthetic effect CRemoveUnitLeaderEffect: key remove_unit_leader: operative codename (COperativeLeader+0x1030, 140C06040) not recorded".to_owned(), 1
    )]));
}

// 1402ECF90 uses both stored names, without scope expansion or LF.
#[test]
fn remove_operative_uses_format98_codename() {
    let (mut fixture, _folder) = fixture(3);
    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
    {
        rows[0].kind = 3;
        rows[0].codename = Recorded::Value(Some(NameOperands {
            kind: 2,
            custom: b"Nightjar".to_vec(),
            ..Default::default()
        }));
    }
    let out = fixture.render(
        "remove_unit_leader=yes",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "Remove operative Leader name/Nightjar [Root.GetTag]"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// 140395390 takes NAME from the referenced leader +0x40 and appends LF.
#[test]
fn show_unit_leaders_uses_format98_referenced_name() {
    let (mut fixture, _folder) = fixture(3);
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![]));
    *fixture.frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![
            CharacterOperativeRow {
                character: 7,
                reference: (5_u64 << 32) | 55,
                resolved: true,
                category: 3,
                name: Recorded::Value(Some(b"Referenced operative".to_vec())),
                ..Default::default()
            },
        ]));
    let out = fixture.render(
        "show_unit_leaders_tooltip=synthetic_character",
        &mut EventScopes::new(vec![EventScope::of(EventObject::Country(1))]),
        Content::default(),
    );
    assert_eq!(out.text, "Operative Referenced operative [Root.GetTag]\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    if let TableView::Complete(TableRows::CharacterOperatives(rows)) =
        &mut *fixture.frame.tables[TableId::CharacterOperatives.index()]
    {
        rows[0].name = Recorded::NotRecorded;
    }
    let out = fixture.render(
        "show_unit_leaders_tooltip=synthetic_character",
        &mut EventScopes::new(vec![EventScope::of(EventObject::Country(1))]),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect CShowUnitLeadersTooltipEffect: key show_unit_leaders_tooltip: character 7 referenced unit leader name (+0x40, 140C16FB0) not recorded".into(), 1)]));
}

// common/decisions/ger.txt:16991, character-scope exact yes.
#[test]
fn remove_unit_leader_role() {
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "remove_unit_leader_role=yes",
        "Remove role Character name/General AAA",
        true,
    );
    rendered(
        &fixture,
        "remove_unit_leader_role={character=synthetic_character}",
        "Remove role Character name/General AAA",
        false,
    );
}

// 1403AAC00 reads a character selector only outside scope-kind 8.
#[test]
fn remove_military_role_respects_its_scope_dependent_reader() {
    let (fixture, _folder) = fixture(0);
    rendered(
        &fixture,
        "remove_unit_leader_role=synthetic_character",
        "Remove role Character name/Field Marshal AAA",
        false,
    );
    rendered(
        &fixture,
        "remove_unit_leader_role={character=synthetic_other}",
        "Remove role Character name/Field Marshal AAA",
        true,
    );
}

#[test]
fn remove_operative_role_uses_skill_specific_title_then_native_fallback() {
    let (mut fixture, _folder) = fixture(3);
    rendered(
        &fixture,
        "remove_unit_leader_role=yes",
        "Remove role Character name/Expert Agent AAA",
        true,
    );
    if let TableView::Complete(TableRows::UnitLeaders(rows)) =
        &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
    {
        rows[0].skill_level = 4;
    }
    rendered(
        &fixture,
        "remove_unit_leader_role=yes",
        "Remove role Character name/Agent AAA",
        true,
    );
}

// common/decisions/phi.txt:1481. The describer gates only on Army existence.
#[test]
fn add_divisional_commander_xp() {
    let (mut fixture, _folder) = fixture(1);
    let reference = 44 | 2 << 32;
    *fixture.frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: reference,
            ..Default::default()
        }]));
    let out = fixture.render(
        "add_divisional_commander_xp=200",
        &mut EventScopes::new(vec![EventScope::of(EventObject::Unit(reference))]),
        Content::default(),
    );
    assert_eq!(out.text, "Commander XP 200.00 [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    for object in [EventObject::Unit(0), EventObject::Country(1)] {
        let out = fixture.render(
            "add_divisional_commander_xp=200",
            &mut EventScopes::new(vec![EventScope::of(object)]),
            Content::default(),
        );
        assert_eq!(out.text, "");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

// Workshop 1137372539 common/national_focus/soviet.txt:12453.
#[test]
fn add_unit_medal_to_latest_entry() {
    let (mut fixture, _folder) = fixture(1);
    let reference = 44 | 2 << 32;
    *fixture.frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: reference,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: reference,
            naming: Recorded::Value(Some(NameOperands::default())),
            ..Default::default()
        }]));
    let out = fixture.render(
        "add_unit_medal_to_latest_entry={unit_medals=synthetic_medal}",
        &mut EventScopes::new(vec![EventScope::of(EventObject::Unit(reference))]),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([(
        "synthetic effect CAddUnitMedalEffect: key add_unit_medal_to_latest_entry: shared UNIT name formatter (140C72F00) awaits lead D2's format-98 follow-up PR to use DivisionStatus.naming".to_owned(), 1
    )]));
    let out = fixture.render(
        "add_unit_medal_to_latest_entry={unit_medals=synthetic_medal}",
        &mut EventScopes::new(vec![EventScope::of(EventObject::Unit(0))]),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// All three Army consumers distinguish old field absence from the still
// pending lead D2 format-98 formatter. Their caller keys/arguments are 14045D2E0,
// 14045D8B0 and 1404A2FB0 -> 141454480.
#[test]
fn army_name_consumers_keep_field_level_compatibility_and_name_missing_core_hook() {
    let (mut fixture, _folder) = fixture(1);
    let unit = 44 | 2 << 32;
    let raid = 9 | 3 << 32;
    *fixture.frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: unit,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::Raids.index()] =
        TableView::Complete(TableRows::Raids(vec![RaidRow {
            id: 3,
            reference_type: 9,
            unit,
            ..Default::default()
        }]));
    for recorded in [false, true] {
        *fixture.frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: unit,
                naming: if recorded {
                    Recorded::Value(Some(NameOperands::default()))
                } else {
                    Recorded::NotRecorded
                },
                ..Default::default()
            }]));
        for (source, class, scope) in [
            (
                "add_history_entry={key=synthetic_history subject=\"Custom String\" allow=yes}",
                "CAddHistoryEntryEffect",
                EventObject::Unit(unit),
            ),
            (
                "add_unit_medal_to_latest_entry={unit_medals=synthetic_medal}",
                "CAddUnitMedalEffect",
                EventObject::Unit(unit),
            ),
            (
                "add_raid_history_entry=yes",
                "CAddRaidHistoryEntryEffect@?A0x8a0aef5d@NRaids",
                EventObject::Raid(raid),
            ),
        ] {
            let out = fixture.render(
                source,
                &mut EventScopes::new(vec![EventScope::of(scope)]),
                Content::default(),
            );
            assert_eq!(out.text, "");
            let reason = if recorded {
                "shared UNIT name formatter (140C72F00) awaits lead D2's format-98 follow-up PR to use DivisionStatus.naming"
            } else {
                "Army name (Army+0x340 -> CNameGroupMember+0x60, 140C72F00) not recorded"
            };
            assert_eq!(
                out.refusals,
                BTreeMap::from([(
                    format!(
                        "synthetic effect {class}: key {}: {reason}",
                        source.split('=').next().unwrap()
                    ),
                    1
                )])
            );
        }
    }
}

#[test]
fn raid_history_without_assigned_army_or_wing_keeps_native_empty_name_line() {
    let (mut fixture, _folder) = fixture(1);
    let reference = 9 | 3 << 32;
    *fixture.frame.tables[TableId::Raids.index()] =
        TableView::Complete(TableRows::Raids(vec![RaidRow {
            id: 3,
            reference_type: 9,
            ..Default::default()
        }]));
    let out = fixture.render(
        "add_raid_history_entry=yes",
        &mut EventScopes::new(vec![EventScope::of(EventObject::Raid(reference))]),
        Content::default(),
    );
    assert_eq!(out.text, "Raid history  [Root.GetTag]\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// Workshop 1521695605 common/on_actions/00_on_actions_global.txt:3104.
#[test]
fn add_random_valid_trait_from_unit() {
    let (fixture, _folder) = fixture(1);
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Character(7))]);
    rendered(
        &fixture,
        "add_random_valid_trait_from_unit={character=THIS}",
        "Random trait for Character name",
        true,
    );
    let out = fixture.render(
        "add_random_valid_trait_from_unit={character=ROOT}",
        &mut scopes,
        Content::default(),
    );
    assert_eq!(out.text, "Random trait for Character name");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn absent_characters_and_missing_roles_keep_native_empty_descriptions() {
    let (mut fixture, _folder) = fixture(1);
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![]));
    for body in [
        "promote_leader=yes",
        "demote_leader=yes",
        "remove_unit_leader=yes",
        "remove_unit_leader_role=yes",
    ] {
        rendered(&fixture, body, "", true);
    }
    rendered(
        &fixture,
        "show_unit_leaders_tooltip=synthetic_character",
        "",
        false,
    );
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![]));
    rendered(&fixture, "add_corps_commander_role={}", "", true);
    rendered(
        &fixture,
        "add_random_valid_trait_from_unit={character=THIS}",
        "",
        true,
    );
}
