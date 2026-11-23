use super::*;
use crate::{
    fixtures::{Folder, folder_catalogue},
    effect_description::tests::Fixture,
    scripted_text::{EventObject, EventScope, Link},
};
use replay_data::rows::*;
use replay_frame::frame::TableView;

fn fixture() -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NMilitary={GENERAL_PROXIMITY_DEFAULT=10,GENERAL_PROXIMITY_CLOSE=5,GENERAL_PROXIMITY_MEDIUM=15}}",
        ),
        (
            "common/country_leader/test.txt",
            "leader_traits={syn_country={custom_modifier_tooltip=COUNTRY_BODY} syn_old={political_power_gain=0.11} syn_new={political_power_gain=0.37} syn_hidden={hidden_modifier={political_power_gain=0.3} custom_modifier_tooltip=COUNTRY_BODY} syn_equipment={equipment_bonus={syn_gun={soft_attack=0.1 supply_consumption=-0.2}}} syn_targeted={targeted_modifier={tag=BBB political_power_gain=0.2}} syn_targeted_custom={targeted_modifier={tag=BBB custom_modifier_tooltip=TARGET_BODY}} syn_targeted_dynamic={targeted_modifier={tag=event_target:syn_target political_power_gain=0.1}} syn_targeted_multi={targeted_modifier={tag=BBB political_power_gain=0.2 political_power_cost=-0.1} targeted_modifier={tag=ROOT political_power_gain=0.3 custom_modifier_tooltip=TARGET_RAW}} syn_empty={}}",
        ),
        (
            "common/unit_leader/test.txt",
            "leader_traits={syn_unit={modifier={custom_modifier_tooltip=UNIT_BODY}} syn_held={} syn_second={} syn_full={modifier={political_power_gain=0.1} non_shared_modifier={custom_modifier_tooltip=NON_SHARED} attack_skill=1 defense_skill=1 logistics_skill=1 planning_skill=1 leader_default_proximity_offset=1 trait_xp_factor={syn_unit=0.2}} syn_override={override_effect_tooltip=OVERRIDE custom_effect_tooltip=IGNORED modifier={political_power_gain=0.2}} syn_numeric={modifier={political_power_gain=0.1 political_power_cost=-0.2 custom_modifier_tooltip=UNIT_BODY}} syn_sub={sub_unit_modifiers={syn_foot={} syn_foot={political_power_gain=0.1} syn_foot={political_power_gain=0.2}}} syn_corps_sub={corps_commander_modifier={political_power_gain=0.3} sub_unit_modifiers={syn_foot={political_power_gain=0.1}}} syn_sub_full={sub_unit_modifiers={syn_foot={political_power_gain=0.1 political_power_cost=-0.2 custom_modifier_tooltip=UNIT_BODY}}} syn_hidden_sub={sub_unit_modifiers={syn_foot={hidden_modifier={political_power_gain=0}} syn_foot={political_power_gain=0.2}}}}",
        ),
        ("common/units/test.txt", "sub_units={syn_foot={}}"),
        (
            "common/scientist_traits/test.txt",
            "syn_scientist={modifier={political_power_gain=0.1 political_power_cost=-0.2}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n AAA_DEF:0 \"alpha\"\n BBB_DEF:0 \"beta\"\n",
                " syn_country:0 \"Country trait\"\n syn_unit:0 \"Unit trait\"\n AAA_syn_unit:0 \"Viewing Unit trait\"\n syn_old:0 \"Old\"\n syn_new:0 \"New\"\n syn_full:0 \"Full\"\n syn_override:0 \"Overriding\"\n syn_numeric:0 \"Numeric\"\n syn_equipment:0 \"Equipment\"\n syn_targeted:0 \"Targeted\"\n syn_empty:0 \"Empty\"\n",
                " syn_sub:0 \"Sub\"\n syn_sub_full:0 \"Sub full\"\n syn_corps_sub:0 \"Corps sub\"\n syn_hidden_sub:0 \"Hidden sub\"\n syn_foot:0 \"Foot\"\n POWER_SCOPE:0 \"Power [Root.GetTag]\"\n POWER_ARGUMENT:0 \"Power $ARG$\"\n SCOPED_SUFFIX:0 \"[Root.GetTag]\"\n ARG:0 \"Literal argument\"\n",
                " syn_hidden:0 \"Hidden\"\n",
                " syn_held:0 \"Held\"\n",
                " syn_targeted_custom:0 \"Target custom\"\n TARGET_BODY:0 \"Target first\\n  - target second\\n\"\n POWER_LF:0 \"Power first\\nPower second\"\n POSTFIX_LF:0 \"Postfix first\\nPostfix second\\n\"\n YES:0 \"Yes first\\nYes second\"\n NO:0 \"Disabled\"\n",
                " syn_scientist:0 \"Synthetic\"\n SCIENTIST_ADD_TRAIT_EFFECT:0 \"scientist[$TRAIT_NAME$]\"\n POWER_SCOPED_LF:0 \"Power [Root.GetTag]\\nAgain\"\n",
                " TARGET_RAW:0 \"Raw [Root.GetTag]\"\n",
                " COUNTRY_BODY:0 \"Country body [Root.GetTag]\"\n UNIT_BODY:0 \"Unit body [Root.GetTag]\"\n NON_SHARED:0 \"Other\"\n OVERRIDE:0 \"Override [Root.GetTag]\"\n IGNORED:0 \"Must not appear\"\n",
                " EFFECT_ADD_TRAIT:0 \"add[$NAME$]/[$TRAITNAME$]|$TRAITMODIFIER$\"\n EFFECT_REMOVE_TRAIT:0 \"remove[$NAME$]/[$TRAITNAME$]|$TRAITMODIFIER$\"\n",
                " EFFECT_ADD_UNIT_LEADER_TRAIT:0 \"unit+[$TRAITNAME$]|$TRAITMODIFIER$\"\n EFFECT_REMOVE_UNIT_LEADER_TRAIT:0 \"unit-[$TRAITNAME$]|$TRAITMODIFIER$\"\n",
                " EFFECT_ADD_UNIT_LEADER_TRAIT_WITH_DURATION:0 \"timed[$TRAITNAME$]/$NUMDAYS$|$TRAITMODIFIER$\"\n",
                " EFFECT_ADD_COUNTRY_LEADER_TRAIT:0 \"country+<$LEADER$>$TRAIT$END\"\n EFFECT_REMOVE_COUNTRY_LEADER_TRAIT:0 \"country-<$LEADER$>$TRAIT$END\"\n",
                " EFFECT_SWAP_RULER_TRAITS_MODIFY:0 \"modify<$LEADER$>/$REMOVE$ [Root.GetTag]\"\n EFFECT_SWAP_RULER_TRAITS_REPLACE:0 \"swap<$LEADER$>/$REMOVE$/$ADD$ [Root.GetTag]\"\n EFFECT_ADD_MAX_TRAIT:0 \"maximum $NUM$ [Root.GetTag]\"\n",
                " EFFECT_CONTEXT_COUNTRY:0 \"@$TAG$/$COUNTRY$: \"\n POWER:0 \"Power\"\n COST:0 \"Cost\"\n trait_bonus_all:0 \"All=$VAL$\"\n trait_bonus_leader_default_proximity_offset:0 \"Distance=$NAME$\"\n LEADER_PROXIMITY_MEDIUM_TITLE:0 \"Medium\"\n trait_xp_factor:0 \"XP=$NAME$/$VAL$\"\n",
                " syn_gun:0 \"Gun\"\n STAT_ARMY_SOFT_ATTACK:0 \"Soft\"\n STAT_ARMY_SUPPLY_CONSUMPTION:0 \"Supply\"\n STAT_MODIFIER_DIFF:0 \"+$VALUE$\"\n STAT_MODIFIER_DIFF_NEG:0 \"-$VALUE$\"\n"
            ),
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 7,
            country: 100,
            name_token_id: 17,
            name: b"Current".to_vec(),
            token: b"original_name".to_vec(),
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
            ideology: 17,
            current_ideology: Recorded::Value(Some(Some(17))),
        }]));
    *fixture.frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            id: 3,
            token: b"syn_group".to_vec(),
            is_ruling: 1,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::PartyLeaders.index()] =
        TableView::Complete(TableRows::PartyLeaders(vec![PartyLeaderRow {
            country: 100,
            party: 3,
            ordinal: 0,
            character: Some(7),
            role: Some(17),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            reference_type: 55,
            id: 5,
            character_id: 7,
            category: 0,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::Scientists.index()] =
        TableView::Complete(TableRows::Scientists(vec![ScientistRow {
            country: 100,
            character: 7,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::UnitLeaderTraits.index()] =
        TableView::Complete(TableRows::UnitLeaderTraits(vec![UnitLeaderTraitRow {
            leader_reference: (5_u64 << 32) | 55,
            ordinal: 0,
            definition_ordinal: 2,
            token: b"syn_held".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 17,
                literal: b"syn_character".to_vec(),
            },
            LexicalTokenRow {
                token: 31,
                literal: b"political_power_gain".to_vec(),
            },
            LexicalTokenRow {
                token: 32,
                literal: b"political_power_cost".to_vec(),
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
            ModifierDefinitionRow {
                index: 1,
                token: 32,
                category_mask: 1,
                flags: 2,
                precision: 2,
                name_key: b"COST".to_vec(),
                ..Default::default()
            },
        ]));
    crate::effect_description::characters::tests::empty_variable_stores(&mut fixture);
    (fixture, folder)
}

fn character() -> EventScopes {
    let mut character = EventScope::of(EventObject::Character(7));
    character.root = Link::Scope(1);
    EventScopes::new(vec![character, EventScope::of(EventObject::Country(1))])
}

fn country() -> EventScopes {
    EventScopes::new(vec![EventScope::of(EventObject::Country(1))])
}

fn assert_render(body: &str, scopes: &mut EventScopes, expected: &str) {
    let (fixture, _folder) = fixture();
    let result = fixture.render(body, scopes, Content::default());
    assert_eq!(result.text, expected, "{body}");
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
}

#[test]
fn add_trait() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_trait={trait=syn_country}",
        &mut character(),
        "unit+[Country trait]|  -Country body [Root.GetTag]\n",
    );
}

#[test]
fn remove_trait() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "remove_trait={trait=syn_country}",
        &mut character(),
        "remove[Current]/[Country trait]|  -Country body [Root.GetTag]\n",
    );
}

#[test]
fn add_country_leader_trait() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_country_leader_trait=syn_country",
        &mut country(),
        "country+<Current>\n    \u{11}HCountry trait\u{11}!\n  - Country body AAA\n\nEND",
    );
}

#[test]
fn remove_country_leader_trait() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "remove_country_leader_trait=syn_country",
        &mut character(),
        "country-<Current>\n    \u{11}HCountry trait\u{11}!\n  - Country body AAA\n\nEND",
    );
}

#[test]
fn scalar_unit_leader_traits_use_viewing_country_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    for (body, expected) in [
        (
            "add_unit_leader_trait=syn_unit",
            "unit+[Viewing Unit trait]|      - Unit body [Root.GetTag]\n",
        ),
        (
            "remove_unit_leader_trait=syn_unit",
            "unit-[Viewing Unit trait]|      - Unit body [Root.GetTag]\n",
        ),
    ] {
        let result = fixture.render(body, &mut character(), Content::default());
        assert_eq!(result.text, expected, "{body}");
        assert!(result.refusals.is_empty(), "{body}: {:?}", result.refusals);
    }
}

#[test]
fn untimed_unit_traits_do_not_read_timed_own_fields() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    for key in ["add_unit_leader_trait", "remove_unit_leader_trait"] {
        assert_render(
            &format!("{key}={{trait=syn_held days=10}}"),
            &mut character(),
            "",
        );
    }
}

#[test]
fn swap_ruler_traits() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "swap_ruler_traits={add=syn_new remove=syn_old}",
        &mut country(),
        "swap<Current>/Old/New AAA\nPower: \u{11}G+0.26\u{11}!\n",
    );
}

#[test]
fn swap_country_leader_traits() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "swap_country_leader_traits={add=syn_new remove=syn_old}",
        &mut character(),
        "swap<Current>/Old/New [Root.GetTag]\nPower: \u{11}G+0.26\u{11}!\n",
    );
}

#[test]
fn add_timed_unit_leader_trait() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_timed_unit_leader_trait={days=017 trait=syn_unit}",
        &mut character(),
        "timed[Viewing Unit trait]/17|      - Unit body [Root.GetTag]\n",
    );
}

#[test]
fn add_random_trait() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_random_trait={syn_unit syn_held}",
        &mut character(),
        "unit+[Viewing Unit trait]|      - Unit body [Root.GetTag]\n",
    );
}

#[test]
fn add_max_trait() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_max_trait=6.5",
        &mut character(),
        "maximum 7 [Root.GetTag]\n",
    );
    assert_render(
        "add_max_trait=-6.5",
        &mut character(),
        "maximum -7 [Root.GetTag]\n",
    );
}

#[test]
fn named_character_trait_uses_raw_modifiers() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_trait={character=syn_character trait=syn_country}",
        &mut country(),
        "add[Current]/[Country trait]|  -Country body [Root.GetTag]\n",
    );
}

#[test]
fn empty_trait_gates_and_swap_error_literal() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_trait={trait=absent} remove_trait={trait=absent} add_country_leader_trait=absent remove_country_leader_trait=absent add_unit_leader_trait=absent remove_unit_leader_trait=absent",
        &mut character(),
        "",
    );
    assert_render(
        "swap_ruler_traits={remove=absent add=syn_new}",
        &mut country(),
        "ERROR: TRAITS ARE MISSING\n",
    );
    assert_render(
        "swap_country_leader_traits={remove=absent add=syn_new}",
        &mut character(),
        "",
    );
}

#[test]
fn numeric_trait_preserves_separator_and_raw_tooltip() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_unit_leader_trait=syn_numeric",
        &mut character(),
        "unit+[Numeric]|      - Power: \u{11}G+0.10\u{11}!\n      - Cost: \u{11}R-0.20\u{11}!\nUnit body [Root.GetTag]\n",
    );
}

#[test]
fn unit_trait_body_preserves_block_order_and_fixed5_scale() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_trait={trait=syn_full}",
        &mut character(),
        "unit+[Full]|  -Power: \u{11}G+0.10\u{11}!\nOther\nAll=1\nDistance=Medium\nXP=Viewing Unit trait/0.20\n\n",
    );
}

#[test]
fn unit_helper_override_suppresses_modifiers_and_separate_custom_lines() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_trait={trait=syn_override}",
        &mut character(),
        "unit+[Overriding]|  -Override [Root.GetTag]\n\n",
    );
}

#[test]
fn country_helper_equipment_uses_stat_labels_and_polarity() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_country_leader_trait=syn_equipment",
        &mut character(),
        "country+<Current>\n    \u{11}HEquipment\u{11}!\n  - \u{11}YGun\u{11}!: \n        Soft+0.10\n        Supply--0.20\nEND",
    );
}

#[test]
fn targeted_trait_uses_country_prefix_and_default_mask() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_country_leader_trait=syn_targeted",
        &mut character(),
        "country+<Current>\n    \u{11}HTargeted\u{11}!\n  - \u{12}BBB/Beta: Power: \u{11}G+0.20\u{11}!\n\nEND",
    );
}

#[test]
fn targeted_trait_keeps_raw_prefix_separator_and_empty_target_scope() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    fixture.with_renderer(&mut character(), Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        assert_eq!(
            renderer
                .char_country_trait_description_with_prefix(b"syn_targeted_multi", scopes, "|/|")
                .unwrap()
                .unwrap(),
            "\u{12}BBB/Beta: Power: \u{11}G+0.20\u{11}!|/|\u{12}BBB/Beta: Cost: \u{11}R-0.10\u{11}!|/|Power: \u{11}G+0.30\u{11}!\nRaw [Root.GetTag]\n",
        );
    });
}

#[test]
fn discarded_targeted_operand_refuses_instead_of_inventing_unprefixed_text() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (fixture, _folder) = fixture();
    let result = fixture.render(
        "add_country_leader_trait=syn_targeted_dynamic",
        &mut character(),
        Content::default(),
    );
    assert_eq!(result.text, "");
    assert_eq!(result.refusals, BTreeMap::from([(
        "synthetic effect add_country_leader_trait: key add_country_leader_trait: country trait syn_targeted_dynamic targeted-country operand is not retained by the shared catalogue".into(),
        1,
    )]));
}

#[test]
fn trait_masks_keep_high_categories_only_in_primary_blocks() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
        &mut *fixture.frame.tables[TableId::ModifierDefinitions.index()]
    else {
        panic!("fixture definitions are complete");
    };
    definitions[0].category_mask = 1 << 31;
    let result = fixture.render(
        "add_trait={trait=syn_full}",
        &mut character(),
        Content::default(),
    );
    assert_eq!(
        result.text,
        "unit+[Full]|  -Power: \u{11}G+0.10\u{11}!\nOther\nAll=1\nDistance=Medium\nXP=Viewing Unit trait/0.20\n\n",
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    let result = fixture.render(
        "add_country_leader_trait=syn_new",
        &mut character(),
        Content::default(),
    );
    assert_eq!(
        result.text,
        "country+<Current>\n    \u{11}HNew\u{11}!\n  - Power: \u{11}G+0.37\u{11}!\n\nEND",
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    fixture.with_renderer(&mut character(), Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        assert_eq!(
            renderer
                .char_country_trait_description_with_prefix(b"syn_targeted_multi", scopes, "|/|")
                .unwrap()
                .unwrap(),
            "\u{12}BBB/Beta: Cost: \u{11}R-0.10\u{11}!|/|Raw [Root.GetTag]\n",
        );
    });
}

#[test]
fn random_trait_reference_role_uses_recorded_assigned_definition_ordinals() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![]));
    *fixture.frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![
            CharacterOperativeRow {
                character: 7,
                reference: (6_u64 << 32) | 55,
                resolved: true,
                category: 3,
                trait_definitions: vec![2],
                ..Default::default()
            },
        ]));
    let result = fixture.render(
        "add_random_trait={syn_unit syn_held}",
        &mut character(),
        Content::default(),
    );
    assert_eq!(
        result.text,
        "unit+[Viewing Unit trait]|      - Unit body [Root.GetTag]\n"
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
}

#[test]
fn empty_country_trait_keeps_name_and_bullet() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_country_leader_trait=syn_empty",
        &mut character(),
        "country+<Current>\n    \u{11}HEmpty\u{11}!\n  - \nEND",
    );
}

#[test]
fn unknown_trait_modifier_has_named_definition_refusal() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![]));
    let result = fixture.render(
        "add_unit_leader_trait=syn_numeric",
        &mut character(),
        Content::default(),
    );
    assert_eq!(result.text, "");
    assert_eq!(result.refusals, BTreeMap::from([(
        "synthetic effect add_unit_leader_trait: key add_unit_leader_trait: idea modifier political_power_gain has no recorded definition".into(),
        1,
    )]));
}

#[test]
fn sub_unit_modifiers_keep_first_nonempty_definition() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_trait={trait=syn_sub}",
        &mut character(),
        "unit+[Sub]|  -\u{11}HFoot\u{11}!\n    Power: \u{11}G+0.10\u{11}!\n\n",
    );
}

#[test]
fn sub_unit_prefix_applies_to_every_numeric_row_and_no_custom_tooltip() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_trait={trait=syn_sub_full}",
        &mut character(),
        "unit+[Sub full]|  -\u{11}HFoot\u{11}!\n    Power: \u{11}G+0.10\u{11}!\n    Cost: \u{11}R-0.20\u{11}!\nUnit body [Root.GetTag]\n\n",
    );
}

#[test]
fn sub_unit_buffer_appends_directly_after_corps_modifiers() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_trait={trait=syn_corps_sub}",
        &mut character(),
        "unit+[Corps sub]|  -Power: \u{11}G+0.30\u{11}!\u{11}HFoot\u{11}!\n    Power: \u{11}G+0.10\u{11}!\n\n",
    );
}

#[test]
fn hidden_sub_unit_keeps_header_without_numeric_line() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_trait={trait=syn_hidden_sub}",
        &mut character(),
        "unit+[Hidden sub]|  -\u{11}HFoot\u{11}!\n\n",
    );
}

#[test]
fn trait_modifier_lookup_preserves_shared_token_ambiguity() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    fixture.with_renderer(&mut character(), Content::default(), |renderer| {
        assert_eq!(
            renderer
                .char_trait_modifier_definition("POLITICAL_POWER_COST")
                .unwrap()
                .token,
            32,
        );
    });
    let TableView::Complete(TableRows::LexicalTokens(tokens)) =
        &mut *fixture.frame.tables[TableId::LexicalTokens.index()]
    else {
        panic!("fixture tokens are complete");
    };
    tokens.push(LexicalTokenRow {
        token: 33,
        literal: b"POLITICAL_POWER_GAIN".to_vec(),
    });
    let result = fixture.render(
        "add_trait={trait=syn_sub}",
        &mut character(),
        Content::default(),
    );
    assert_eq!(result.text, "");
    assert_eq!(result.refusals, BTreeMap::from([(
        "synthetic effect add_trait: key add_trait: refused token: POLITICAL_POWER_GAIN names several lexer tokens".into(),
        1,
    )]));
}

#[test]
fn conditional_unit_catalogue_preserves_shared_token_ambiguity() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    let (catalogue, _conditional_folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/unit_leader/test.txt",
            "leader_traits={syn_conditional={modifier={syn_modifier={}}}}",
        ),
        ("common/terrain/test.txt", "categories={syn_plain={}}"),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 41,
                literal: b"syn_modifier".to_vec(),
            },
            LexicalTokenRow {
                token: 42,
                literal: b"SYN_MODIFIER".to_vec(),
            },
        ]));
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![
            ModifierDefinitionRow {
                token: 41,
                ..Default::default()
            },
        ]));
    fixture.with_renderer(&mut country(), Content::default(), |renderer| {
        assert!(
            renderer
                .character_content()
                .unwrap()
                .unit_leaders
                .as_ref()
                .unwrap()
                .conditional()
        );
        assert_eq!(
            renderer.char_unit_catalogue().err().unwrap(),
            "refused token: SYN_MODIFIER names several lexer tokens",
        );
    });
}

#[test]
fn empty_plain_trait_needs_no_recorded_modifier_tables() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    for table in [TableId::ModifierDefinitions, TableId::LexicalTokens] {
        *fixture.frame.tables[table.index()] = TableView::Unavailable {
            reason: "empty traits need no modifier registry".into(),
        };
    }
    for (effect, expected) in [
        ("add_unit_leader_trait=syn_held", "unit+[Held]|"),
        ("remove_unit_leader_trait=syn_held", "unit-[Held]|"),
    ] {
        let result = fixture.render(effect, &mut character(), Content::default());
        assert_eq!(result.text, expected);
        assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    }
}

#[test]
fn swap_root_delta_does_not_copy_custom_tooltips_or_hidden_sets() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "swap_ruler_traits={remove=syn_old add=syn_hidden}",
        &mut country(),
        "swap<Current>/Old/Hidden AAA\nPower: \u{11}G+0.19\u{11}!\n",
    );
    assert_render(
        "swap_country_leader_traits={remove=syn_country add=syn_country}",
        &mut character(),
        "modify<Current>/Country trait [Root.GetTag]\n\n",
    );
}

#[test]
fn targeted_custom_trait_keeps_embedded_separator_and_trailing_lf() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_render(
        "add_country_leader_trait=syn_targeted_custom",
        &mut character(),
        "country+<Current>\n    \u{11}HTarget custom\u{11}!\n  - Target first\n  - target second\n\nEND",
    );
}

#[test]
fn scoped_modifier_entry_point_allows_expressions_and_embedded_lf() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut fixture, _folder) = fixture();
    let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
        &mut *fixture.frame.tables[TableId::ModifierDefinitions.index()]
    else {
        panic!("fixture definitions are complete");
    };
    definitions[0].name_key = b"POWER_SCOPED_LF".to_vec();
    let result = fixture.render(
        "add_scientist_trait=syn_scientist",
        &mut character(),
        Content::default(),
    );
    assert_eq!(
        result.text,
        "scientist[Synthetic]\n  Power AAA\n  Again: \u{11}G+0.10\u{11}!\n  Cost: \u{11}R-0.20\u{11}!"
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
}
