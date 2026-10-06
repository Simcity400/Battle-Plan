use super::*;
use crate::{
    catalogue::tests::{Folder, folder_catalogue},
    effect_description::tests::Fixture,
    scripted_text::{EventObject, EventScope, Link},
};
use replay::{frame::TableView, rows::*};

fn fixture() -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NMilitary={GENERAL_PROXIMITY_DEFAULT=10,GENERAL_PROXIMITY_CLOSE=5,GENERAL_PROXIMITY_MEDIUM=15}}",
        ),
        (
            "common/country_leader/test.txt",
            "leader_traits={syn_country={custom_modifier_tooltip=COUNTRY_BODY} syn_old={political_power_gain=0.1} syn_new={political_power_gain=0.3} syn_hidden={hidden_modifier={political_power_gain=0.3} custom_modifier_tooltip=COUNTRY_BODY} syn_equipment={equipment_bonus={syn_gun={soft_attack=0.1 supply_consumption=-0.2}}} syn_targeted={targeted_modifier={tag=BBB political_power_gain=0.2}} syn_targeted_custom={targeted_modifier={tag=BBB custom_modifier_tooltip=TARGET_BODY}} syn_empty={}}",
        ),
        (
            "common/unit_leader/test.txt",
            "leader_traits={syn_unit={modifier={custom_modifier_tooltip=UNIT_BODY}} syn_held={} syn_second={} syn_full={modifier={political_power_gain=0.1} non_shared_modifier={custom_modifier_tooltip=NON_SHARED} attack_skill=1 defense_skill=1 logistics_skill=1 planning_skill=1 leader_default_proximity_offset=1 trait_xp_factor={syn_unit=0.2}} syn_override={override_effect_tooltip=OVERRIDE custom_effect_tooltip=IGNORED modifier={political_power_gain=0.2}} syn_numeric={modifier={political_power_gain=0.1 political_power_cost=-0.2 custom_modifier_tooltip=UNIT_BODY}} syn_sub={sub_unit_modifiers={syn_foot={} syn_foot={political_power_gain=0.1} syn_foot={political_power_gain=0.2}}} syn_hidden_sub={sub_unit_modifiers={syn_foot={hidden_modifier={political_power_gain=0}} syn_foot={political_power_gain=0.2}}}}",
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
                " syn_sub:0 \"Sub\"\n syn_hidden_sub:0 \"Hidden sub\"\n syn_foot:0 \"Foot\"\n POWER_SCOPE:0 \"Power [Root.GetTag]\"\n POWER_ARGUMENT:0 \"Power $ARG$\"\n SCOPED_SUFFIX:0 \"[Root.GetTag]\"\n ARG:0 \"Literal argument\"\n",
                " syn_hidden:0 \"Hidden\"\n",
                " syn_held:0 \"Held\"\n",
                " syn_targeted_custom:0 \"Target custom\"\n TARGET_BODY:0 \"Target first\\n  - target second\\n\"\n POWER_LF:0 \"Power first\\nPower second\"\n POSTFIX_LF:0 \"Postfix first\\nPostfix second\\n\"\n YES:0 \"Yes first\\nYes second\"\n NO:0 \"No\"\n",
                " syn_scientist:0 \"Synthetic\"\n SCIENTIST_ADD_TRAIT_EFFECT:0 \"scientist[$TRAIT_NAME$]\"\n POWER_SCOPED_LF:0 \"Power [Root.GetTag]\\nAgain\"\n",
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

fn assert_plain_refusal(body: &str, scopes: &mut EventScopes, class: &str) {
    let (fixture, _folder) = fixture();
    let result = fixture.render(body, scopes, Content::default());
    assert_eq!(result.text, "");
    assert_eq!(result.refusals, BTreeMap::from([(
        format!("synthetic effect {class}: key {}: native plain modifier description requires core plain/mask/separator/show-zero parameters (14054E050/14054F6D0)", body.split('=').next().unwrap()),
        1,
    )]));
}

// Authored block: common/characters/swe.txt:1731.
#[test]
fn add_trait() {
    assert_plain_refusal(
        "add_trait={trait=syn_country}",
        &mut character(),
        "CAddTraitEffect",
    );
}

// Authored block: common/characters/swe.txt:1740.
#[test]
fn remove_trait() {
    assert_plain_refusal(
        "remove_trait={trait=syn_country}",
        &mut character(),
        "CRemoveTraitEffect",
    );
}

// Authored scalar: common/decisions/ast.txt:1484.
#[test]
fn add_country_leader_trait() {
    assert_plain_refusal(
        "add_country_leader_trait=syn_country",
        &mut country(),
        "CAddCountryLeaderTraitEffect",
    );
}

// Authored scalar: common/decisions/ita.txt:2013.
#[test]
fn remove_country_leader_trait() {
    assert_plain_refusal(
        "remove_country_leader_trait=syn_country",
        &mut character(),
        "CRemoveCountryLeaderTraitEffect",
    );
}

// Authored scalar: common/decisions/bul.txt:5238.
#[test]
fn add_unit_leader_trait() {
    assert_plain_refusal(
        "add_unit_leader_trait=syn_unit",
        &mut character(),
        "CAddUnitLeaderTraitEffect",
    );
}

// Authored scalar: common/decisions/pol.txt:1011.
#[test]
fn remove_unit_leader_trait() {
    assert_plain_refusal(
        "remove_unit_leader_trait=syn_unit",
        &mut character(),
        "CRemoveUnitLeaderTraitEffect",
    );
}

// Untimed classes use ReadValue 141385570; only the timed key reader
// 1402F0FF0 consumes a trait/days block.
#[test]
fn untimed_unit_traits_do_not_read_timed_own_fields() {
    for key in ["add_unit_leader_trait", "remove_unit_leader_trait"] {
        assert_render(
            &format!("{key}={{trait=syn_held days=10}}"),
            &mut character(),
            "",
        );
    }
}

// Authored remove/add: common/decisions/formable_nation_decisions.txt:7565.
#[test]
fn swap_ruler_traits() {
    assert_plain_refusal(
        "swap_ruler_traits={remove=syn_old add=syn_new}",
        &mut country(),
        "CSwapRulerTraitsEffect",
    );
}

// Authored remove/add: common/decisions/swe.txt:1280.
#[test]
fn swap_country_leader_traits() {
    assert_plain_refusal(
        "swap_country_leader_traits={remove=syn_old add=syn_new}",
        &mut character(),
        "CSwapCountryLeaderTraitsEffect",
    );
}

// Authored days/trait: common/national_focus/siam.txt:403.
#[test]
fn add_timed_unit_leader_trait() {
    assert_plain_refusal(
        "add_timed_unit_leader_trait={trait=syn_unit days=010}",
        &mut character(),
        "CAddTimedUnitLeaderTraitEffect",
    );
}

// Authored list: common/national_focus/china_warlord.txt:1205.
#[test]
fn add_random_trait() {
    assert_plain_refusal(
        "add_random_trait={syn_held syn_unit}",
        &mut character(),
        "CAddRandomTraitEffect",
    );
}

// Authored scalar: common/decisions/_generic_decisions.txt:2549.
#[test]
fn add_max_trait() {
    assert_render(
        "add_max_trait=2.5",
        &mut character(),
        "maximum 3 [Root.GetTag]\n",
    );
    assert_render(
        "add_max_trait=-2.5",
        &mut character(),
        "maximum -3 [Root.GetTag]\n",
    );
}

#[test]
fn named_character_trait_preserves_named_formatter_refusal() {
    assert_plain_refusal(
        "add_trait={character=syn_character trait=syn_country}",
        &mut country(),
        "CAddTraitEffect",
    );
}

#[test]
fn native_empty_trait_gates_and_swap_error_literal() {
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
fn numeric_trait_separator_requires_core_options() {
    assert_plain_refusal(
        "add_unit_leader_trait=syn_numeric",
        &mut character(),
        "CAddUnitLeaderTraitEffect",
    );
}

#[test]
fn unit_trait_plain_block_requires_core_options() {
    assert_plain_refusal(
        "add_trait={trait=syn_full}",
        &mut character(),
        "CAddTraitEffect",
    );
}

#[test]
fn unit_helper_override_suppresses_modifiers_and_separate_custom_lines() {
    assert_render(
        "add_trait={trait=syn_override}",
        &mut character(),
        "unit+[Overriding]|  -Override [Root.GetTag]\n\n",
    );
}

#[test]
fn country_helper_equipment_uses_native_stat_labels_and_polarity() {
    assert_render(
        "add_country_leader_trait=syn_equipment",
        &mut character(),
        "country+<Current>\n    \u{11}HEquipment\u{11}!\n  - \u{11}YGun\u{11}!: \n        Soft+0.10\n        Supply--0.20\nEND",
    );
}

#[test]
fn targeted_trait_requires_core_mask_and_plain_options() {
    let (fixture, _folder) = fixture();
    let result = fixture.render("add_country_leader_trait=syn_targeted", &mut character(), Content::default());
    assert_eq!(result.text, "");
    assert_eq!(result.refusals, BTreeMap::from([(
        "synthetic effect CAddCountryLeaderTraitEffect: key add_country_leader_trait: native targeted trait modifiers require core category-mask/plain/separator/show-zero parameters (140712850/14054E050)".into(),
        1,
    )]));
}

#[test]
fn random_trait_reference_role_uses_recorded_assigned_definition_ordinals() {
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
        "add_random_trait={syn_held syn_unit}",
        &mut character(),
        Content::default(),
    );
    assert_eq!(result.text, "");
    assert_eq!(result.refusals, BTreeMap::from([(
        "synthetic effect CAddRandomTraitEffect: key add_random_trait: native plain modifier description requires core plain/mask/separator/show-zero parameters (14054E050/14054F6D0)".into(),
        1,
    )]));
}

#[test]
fn empty_country_trait_keeps_native_name_and_bullet() {
    assert_render(
        "add_country_leader_trait=syn_empty",
        &mut character(),
        "country+<Current>\n    \u{11}HEmpty\u{11}!\n  - \nEND",
    );
}

#[test]
fn plain_modifier_options_refuse_without_inspecting_definition_rules() {
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
        "synthetic effect CAddUnitLeaderTraitEffect: key add_unit_leader_trait: native plain modifier description requires core plain/mask/separator/show-zero parameters (14054E050/14054F6D0)".into(),
        1,
    )]));
}

#[test]
fn sub_unit_nonempty_plain_modifiers_require_core_options() {
    assert_plain_refusal(
        "add_trait={trait=syn_sub}",
        &mut character(),
        "CAddTraitEffect",
    );
}

#[test]
fn hidden_sub_unit_plain_modifiers_require_core_options() {
    assert_plain_refusal(
        "add_trait={trait=syn_hidden_sub}",
        &mut character(),
        "CAddTraitEffect",
    );
}

#[test]
fn trait_modifier_lookup_preserves_shared_token_ambiguity() {
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
        "synthetic effect CAddTraitEffect: key add_trait: refused token: POLITICAL_POWER_GAIN names several lexer tokens".into(),
        1,
    )]));
}

#[test]
fn conditional_unit_catalogue_preserves_shared_token_ambiguity() {
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
        TableView::Complete(TableRows::ModifierDefinitions(vec![ModifierDefinitionRow {
            token: 41,
            ..Default::default()
        }]));
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
    assert_plain_refusal(
        "swap_ruler_traits={remove=syn_old add=syn_hidden}",
        &mut country(),
        "CSwapRulerTraitsEffect",
    );
    assert_render(
        "swap_country_leader_traits={remove=syn_country add=syn_country}",
        &mut character(),
        "modify<Current>/Country trait [Root.GetTag]\n\n",
    );
}


#[test]
fn targeted_custom_trait_requires_core_options() {
    let (fixture, _folder) = fixture();
    let result = fixture.render("add_country_leader_trait=syn_targeted_custom", &mut character(), Content::default());
    assert_eq!(result.text, "");
    assert_eq!(result.refusals, BTreeMap::from([(
        "synthetic effect CAddCountryLeaderTraitEffect: key add_country_leader_trait: native targeted trait modifiers require core category-mask/plain/separator/show-zero parameters (140712850/14054E050)".into(),
        1,
    )]));
}

#[test]
fn scoped_modifier_entry_point_allows_expressions_and_embedded_lf() {
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
