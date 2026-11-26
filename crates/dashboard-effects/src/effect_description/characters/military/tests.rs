use super::*;
use crate::fixtures::{Folder, folder_catalogue};
use crate::effect_description::tests::Fixture;
use crate::scripted_text::{EventObject, EventScope, Link};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

fn fixture(category: i32) -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23}}",
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
                " fm_leader:0 \"Strategic commander\"\n cc_leader:0 \"Ground commander\"\n navy_leader:0 \"Fleet commander\"\n",
                " operative_skill_3:0 \"Expert Agent\"\n OPERATIVE:0 \"Agent\"\n",
                " synthetic_operative:0 \"Authored agent\"\n",
                " ADD_DIVISIONAL_COMMANDER_XP:0 \"Commander XP $VAL|2$ [Root.GetTag]\"\n",
                " EFFECT_ADD_HISTORY_ENTRY_UNIT_EFFECT:0 \"History $UNIT$ [Root.GetTag]\"\n",
                " ADD_UNIT_MEDAL_DIV_EFFECT:0 \"Medal $DIV$ [Root.GetTag]\"\n",
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

#[test]
fn legacy_empty_name_uses_recorded_name_token_then_exact_stored_token() {
    game::enter();
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

#[test]
fn create_operative_leader() {
    game::enter();
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "fixture_new_agent={bypass_recruitment=yes}",
        "Operative  AAA",
        false,
    );
    rendered(
        &fixture,
        "fixture_new_agent={traits={} bypass_recruitment=yes name=synthetic_operative}",
        "Operative Authored agent AAA",
        false,
    );
}

#[test]
fn show_unit_leaders_tooltip() {
    game::enter();
    let (mut fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "fixture_list_commanders=synthetic_character",
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
        "fixture_list_commanders=synthetic_character",
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
        "fixture_list_commanders=synthetic_character",
        "",
        false,
    );
}

#[test]
fn promote_and_demote_leader_category_gates() {
    game::enter();
    let (mut fixture, _folder) = fixture(1);
    for (category, body, expected) in [
        (1, "fixture_raise_rank=yes", "Promote [Root.GetTag]\n"),
        (1, "fixture_lower_rank=yes", ""),
        (0, "fixture_lower_rank=yes", "Demote [Root.GetTag]\n"),
        (0, "fixture_raise_rank=yes", ""),
    ] {
        let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
        else {
            unreachable!()
        };
        rows[0].category = category;
        let out = fixture.render(body, &mut character_scopes(), Content::default());
        assert_eq!(out.text, expected, "{category}/{body}");
        assert!(
            out.refusals.is_empty(),
            "{category}/{body}: {:?}",
            out.refusals
        );
    }
}

#[test]
fn remove_unit_leader() {
    game::enter();
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "fixture_retire_commander=7",
        "Remove corps Leader name [Root.GetTag]",
        false,
    );
    rendered(
        &fixture,
        "fixture_retire_commander=yes",
        "Remove corps Leader name [Root.GetTag]",
        true,
    );
    rendered(&fixture, "fixture_retire_commander=201", "", false);
}

#[test]
fn remove_operative_refuses_codename_by_class_and_field() {
    game::enter();
    let (fixture, _folder) = fixture(3);
    let out = fixture.render(
        "fixture_retire_commander=yes",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([(
        "synthetic effect fixture_retire_commander: key fixture_retire_commander: operative codename not recorded".to_owned(), 1
    )]));
}

#[test]
fn remove_operative_uses_format98_codename() {
    game::enter();
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
        "fixture_retire_commander=yes",
        &mut character_scopes(),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "Remove operative Leader name/Nightjar [Root.GetTag]"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn show_unit_leaders_uses_format98_referenced_name() {
    game::enter();
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
        "fixture_list_commanders=synthetic_character",
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
        "fixture_list_commanders=synthetic_character",
        &mut EventScopes::new(vec![EventScope::of(EventObject::Country(1))]),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect fixture_list_commanders: key fixture_list_commanders: character 7 referenced unit leader name not recorded".into(), 1)]));
}

#[test]
fn remove_unit_leader_role() {
    game::enter();
    let (fixture, _folder) = fixture(1);
    rendered(
        &fixture,
        "fixture_remove_commander_role=yes",
        "Remove role Character name/Ground commander AAA",
        true,
    );
    rendered(
        &fixture,
        "fixture_remove_commander_role={character=synthetic_character}",
        "Remove role Character name/Ground commander AAA",
        false,
    );
}

#[test]
fn remove_military_role_respects_its_scope_dependent_reader() {
    game::enter();
    let (fixture, _folder) = fixture(0);
    rendered(
        &fixture,
        "fixture_remove_commander_role=synthetic_character",
        "Remove role Character name/Strategic commander AAA",
        false,
    );
    rendered(
        &fixture,
        "fixture_remove_commander_role={character=synthetic_other}",
        "Remove role Character name/Strategic commander AAA",
        true,
    );
}

#[test]
fn remove_operative_role_uses_skill_specific_title_then_fallback() {
    game::enter();
    let (mut fixture, _folder) = fixture(3);
    rendered(
        &fixture,
        "fixture_remove_commander_role=yes",
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
        "fixture_remove_commander_role=yes",
        "Remove role Character name/Agent AAA",
        true,
    );
}

#[test]
fn add_divisional_commander_xp() {
    game::enter();
    let (mut fixture, _folder) = fixture(1);
    let reference = 44 | 2 << 32;
    *fixture.frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: reference,
            ..Default::default()
        }]));
    let out = fixture.render(
        "fixture_division_experience=137.5",
        &mut EventScopes::new(vec![EventScope::of(EventObject::Unit(reference))]),
        Content::default(),
    );
    assert_eq!(out.text, "Commander XP 137.50 [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    for object in [EventObject::Unit(0), EventObject::Country(1)] {
        let out = fixture.render(
            "fixture_division_experience=137.5",
            &mut EventScopes::new(vec![EventScope::of(object)]),
            Content::default(),
        );
        assert_eq!(out.text, "");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn add_unit_medal_is_empty_without_army() {
    game::enter();
    let (fixture, _folder) = fixture(1);
    let out = fixture.render(
        "fixture_award_medal={unit_medals=synthetic_medal}",
        &mut EventScopes::new(vec![EventScope::of(EventObject::Unit(0))]),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn army_name_consumers_share_names_and_field_level_compatibility() {
    game::enter();
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
    for naming in [
        Recorded::NotRecorded,
        Recorded::Value(None),
        Recorded::Value(Some(NameOperands {
            custom: b"Recorded army".to_vec(),
            ..Default::default()
        })),
    ] {
        let recorded = matches!(naming, Recorded::Value(Some(_)));
        let unavailable = matches!(naming, Recorded::Value(None));
        *fixture.frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: unit,
                name: Recorded::NotRecorded,
                naming,
                ..Default::default()
            }]));
        for (source, effect_key, scope, expected) in [
            (
                "fixture_division_history={key=synthetic_history subject=\"Training record\" allow=yes}",
                "fixture_division_history",
                EventObject::Unit(unit),
                "History Recorded army [Root.GetTag]",
            ),
            (
                "fixture_award_medal={unit_medals=synthetic_medal}",
                "fixture_award_medal",
                EventObject::Unit(unit),
                "Medal Recorded army [Root.GetTag]",
            ),
            (
                "fixture_raid_history=yes",
                "fixture_raid_history",
                EventObject::Raid(raid),
                "Raid history Recorded army [Root.GetTag]\n",
            ),
        ] {
            let out = fixture.render(
                source,
                &mut EventScopes::new(vec![EventScope::of(scope)]),
                Content::default(),
            );
            if recorded {
                assert_eq!(out.text, expected);
                assert!(out.refusals.is_empty(), "{:?}", out.refusals);
                continue;
            }
            assert!(out.text.is_empty());
            let reason = if unavailable {
                format!(
                    "naming of division status unavailable for UNIT {unit:#x}: guarded read or arena shortage"
                )
            } else {
                "naming of division status not recorded".to_owned()
            };
            assert_eq!(out.refusals.len(), 1);
            let (why, count) = out.refusals.iter().next().unwrap();
            assert_eq!(*count, 1);
            let expected = format!(
                "synthetic effect {effect_key}: key {}: {reason}",
                source.split('=').next().unwrap()
            );
            if unavailable {
                assert!(why.starts_with(expected.as_str()), "{why}");
            } else {
                assert_eq!(why, &expected);
            }
        }
    }
}

#[test]
fn raid_history_without_assigned_army_or_wing_keeps_empty_name_line() {
    game::enter();
    let (mut fixture, _folder) = fixture(1);
    let reference = 9 | 3 << 32;
    *fixture.frame.tables[TableId::Raids.index()] =
        TableView::Complete(TableRows::Raids(vec![RaidRow {
            id: 3,
            reference_type: 9,
            ..Default::default()
        }]));
    let out = fixture.render(
        "fixture_raid_history=yes",
        &mut EventScopes::new(vec![EventScope::of(EventObject::Raid(reference))]),
        Content::default(),
    );
    assert_eq!(out.text, "Raid history  [Root.GetTag]\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn raid_history_wing_falls_back_from_stale_army_and_preserves_language_refusal() {
    game::enter();
    use engine_native_air::raid_tooltip_operands::{
        ARMY_RESOLVED,
        NAME,
        REFERENCES,
        WING_NAME,
        WING_RESOLVED,
    };
    let (mut fixture, _folder) = fixture(1);
    let reference = 9 | 3 << 32;
    let wing = 12 | 4 << 32;
    let name = b"Recorded wing";
    *fixture.frame.tables[TableId::Raids.index()] =
        TableView::Complete(TableRows::Raids(vec![RaidRow {
            id: 3,
            reference_type: 9,
            air_wing: wing,
            phase: 4,
            ..Default::default()
        }]));
    let values = [
        (REFERENCES, 0, 0),
        (REFERENCES, 1, wing as i64),
        (NAME, ARMY_RESOLVED, 0),
        (NAME, WING_RESOLVED, 1),
        (WING_NAME, u32::MAX, name.len() as i64),
    ];
    let chunks = name.chunks(8).enumerate().map(|(index, bytes)| {
        let mut packed = [0; 8];
        packed[..bytes.len()].copy_from_slice(bytes);
        (WING_NAME, index as u32, i64::from_le_bytes(packed))
    });
    *fixture.frame.tables[TableId::RaidTooltipOperands.index()] =
        TableView::Complete(TableRows::RaidTooltipOperands(
            values
                .into_iter()
                .chain(chunks)
                .enumerate()
                .map(|(ordinal, (kind, key, raw))| RaidTooltipOperandRow {
                    raid: reference,
                    ordinal: ordinal as u32,
                    kind,
                    key,
                    raw,
                })
                .collect(),
        ));
    let body = "fixture_raid_history=yes";
    let out = fixture.render(
        body,
        &mut EventScopes::new(vec![EventScope::of(EventObject::Raid(reference))]),
        Content::default(),
    );
    assert_eq!(out.text, "Raid history Recorded wing [Root.GetTag]\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let stale_army = 44 | 2 << 32;
    if let TableView::Complete(TableRows::Raids(rows)) =
        &mut *fixture.frame.tables[TableId::Raids.index()]
    {
        rows[0].unit = stale_army;
    }
    if let TableView::Complete(TableRows::RaidTooltipOperands(rows)) =
        &mut *fixture.frame.tables[TableId::RaidTooltipOperands.index()]
    {
        rows.iter_mut()
            .find(|row| row.kind == REFERENCES && row.key == 0)
            .unwrap()
            .raw = stale_army as i64;
    }
    for census in [
        TableView::Complete(TableRows::Divisions(Vec::new())),
        TableView::Unavailable {
            reason: "synthetic Army census unavailable".into(),
        },
    ] {
        *fixture.frame.tables[TableId::Divisions.index()] = census;
        let out = fixture.render(
            body,
            &mut EventScopes::new(vec![EventScope::of(EventObject::Raid(reference))]),
            Content::default(),
        );
        assert_eq!(out.text, "Raid history Recorded wing [Root.GetTag]\n");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
    let operands = fixture.frame.tables[TableId::RaidTooltipOperands.index()].clone();
    *fixture.frame.tables[TableId::RaidTooltipOperands.index()] = TableView::Unavailable {
        reason: "synthetic raid witness unavailable".into(),
    };
    let out = fixture.render(
        body,
        &mut EventScopes::new(vec![EventScope::of(EventObject::Raid(reference))]),
        Content::default(),
    );
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.len(), 1);
    assert!(
        out.refusals
            .keys()
            .next()
            .unwrap()
            .contains("synthetic Army census unavailable")
    );
    fixture.frame.tables[TableId::RaidTooltipOperands.index()] = operands;
    std::sync::Arc::make_mut(&mut fixture.catalogue.localisation).language = "french".into();
    let out = fixture.render(
        body,
        &mut EventScopes::new(vec![EventScope::of(EventObject::Raid(reference))]),
        Content::default(),
    );
    assert!(out.text.is_empty());
    assert_eq!(out.refusals, BTreeMap::from([(
        "synthetic effect fixture_raid_history: key fixture_raid_history: raid wing name is text the game stored in english without its key, and the viewer's language is french".into(), 1
    )]));
}

#[test]
fn add_random_valid_trait_from_unit() {
    game::enter();
    let (fixture, _folder) = fixture(1);
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Character(7))]);
    rendered(
        &fixture,
        "fixture_choose_unit_award={character=THIS}",
        "Random trait for Character name",
        true,
    );
    let out = fixture.render(
        "fixture_choose_unit_award={character=ROOT}",
        &mut scopes,
        Content::default(),
    );
    assert_eq!(out.text, "Random trait for Character name");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn absent_characters_and_missing_roles_keep_empty_descriptions() {
    game::enter();
    let (mut fixture, _folder) = fixture(1);
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![]));
    for body in [
        "fixture_raise_rank=yes",
        "fixture_lower_rank=yes",
        "fixture_retire_commander=yes",
        "fixture_remove_commander_role=yes",
    ] {
        rendered(&fixture, body, "", true);
    }
    rendered(
        &fixture,
        "fixture_list_commanders=synthetic_character",
        "",
        false,
    );
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![]));
    rendered(&fixture, "fixture_assign_commander={}", "", true);
    rendered(
        &fixture,
        "fixture_choose_unit_award={character=THIS}",
        "",
        true,
    );
}
