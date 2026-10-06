use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use replay::{frame::TableView, rows::*};

fn fixture() -> Fixture {
    let mut fixture = Fixture::new();
    let added = [
        ("EFFECT_ADD_IDEA", "Add $NAME$"),
        ("EFFECT_REMOVE_IDEA", "Remove $NAME$"),
        ("EFFECT_SHOW_IDEA_TOOLTIP", "$NAME$"),
        ("EFFECT_SHOW_IDEA_TOOLTIP_DETAILED", "$NAME$:\n$DESC$"),
        ("EFFECT_REMOVE_IDEAS_WITH_TRAIT", "$COUNTRY$ loses $TRAIT$"),
        ("POLITICAL_RESEARCH_BONUS_ENTRY", "$CATEGORY$: $FACTOR|%0$"),
        ("industry_research", "Industry"),
        ("SECOND_IDEA", "Second"),
        ("TARGET_IDEA", "Targeted"),
        ("NO_TARGET_IDEA", "Untargeted"),
        ("SELF_TARGET_IDEA", "Empty selected scope"),
        ("SYN_OPINION", "Opinion"),
        ("SYN_RELATION", "Relation"),
        ("SYN_DYNAMIC", "Dynamic [THIS.GetTag]"),
        ("EFFECT_ADD_OPINION", "$TAG$/$TARGET$ $TRUST$ $NAME$"),
        (
            "EFFECT_REVERSE_ADD_OPINION",
            "Reverse $TARGET$ $TRUST$ $NAME$",
        ),
        ("EFFECT_REMOVE_OPINION", "Remove $TARGET$ $TRUST$ $NAME$"),
        ("EFFECT_ADD_TRADE_OPINION", "Trade $TARGET$ $TRUST$ $NAME$"),
        (
            "EFFECT_REVERSE_ADD_TRADE_OPINION",
            "Reverse trade $TARGET$ $TRUST$ $NAME$",
        ),
        (
            "EFFECT_REMOVE_TRADE_OPINION",
            "Remove trade $TARGET$ $TRUST$ $NAME$",
        ),
        (
            "EFFECT_ADD_RELATION_MODIFIER",
            "Relation $TARGET$ $NAME$ ($DESC$)",
        ),
        (
            "EFFECT_REMOVE_RELATION_MODIFIER",
            "Unrelation $TARGET$ $NAME$ ($DESC$)",
        ),
        ("EFFECT_ADD_DYMAMIC_MODIFIER", "Add $NAME$ ($DESC$)"),
        (
            "EFFECT_ADD_DYMAMIC_MODIFIER_WITH_DAYS",
            "Add $NAME$ for $DAYS|0$ ($DESC$)",
        ),
        ("EFFECT_REMOVE_DYMAMIC_MODIFIER", "Remove $NAME$ ($DESC$)"),
        ("EFFECT_ADD_STATE_MODIFIER", "State$MODIFIERS$"),
        ("EFFECT_ADD_PROVINCE_MODIFIER", "Province $MODIFIERLIST$"),
        (
            "EFFECT_REMOVE_PROVINCE_MODIFIER",
            "Unprovince $MODIFIERLIST$",
        ),
        ("EFFECT_PROVINCE_MODIFIER_ENTRY", "$NAME$:$MODIFIERS$"),
    ];
    fixture.catalogue.localisation = std::sync::Arc::new(
        fixture
            .catalogue
            .localisation
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .chain(
                added
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value.to_owned())),
            )
            .collect(),
    );
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(
            ["attack", "defense"]
                .iter()
                .enumerate()
                .map(|(at, literal)| LexicalTokenRow {
                    token: at as u32 + 1,
                    literal: literal.as_bytes().to_vec(),
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(
            ["ATTACK", "DEFENSE"]
                .iter()
                .enumerate()
                .map(|(at, key)| ModifierDefinitionRow {
                    index: at as u32,
                    token: at as u32 + 1,
                    name_key: key.as_bytes().to_vec(),
                    category_mask: 1 << at,
                    precision: 2,
                    flags: crate::modifier_text::POSITIVE_IS_GOOD,
                    ..Default::default()
                })
                .collect(),
        ));
    fixture
}

fn definitions(fixture: &mut Fixture) -> &mut [ModifierDefinitionRow] {
    let TableView::Complete(TableRows::ModifierDefinitions(rows)) =
        &mut *fixture.frame.tables[TableId::ModifierDefinitions.index()]
    else {
        unreachable!()
    };
    rows
}

fn describe(fixture: &Fixture, body: &str, options: ModifierListOptions<'_>) -> String {
    let modifiers =
        installed_content::modifiers::values(&installed_content::parser::parse(body.as_bytes()).unwrap().items);
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        renderer
            .modifier_list_with(&modifiers, ">", scopes, options)
            .unwrap()
    })
}

#[test]
fn category_mask_selects_any_matching_definition_bit() {
    let mut fixture = fixture();
    definitions(&mut fixture)[1].category_mask |= 1;
    assert_eq!(
        describe(
            &fixture,
            "defense=2 attack=1",
            ModifierListOptions {
                category_mask: 2,
                ..Default::default()
            },
        ),
        "> Defense: \u{11}G+2.00\u{11}!"
    );
    assert_eq!(
        describe(
            &fixture,
            "attack=1 custom_modifier_tooltip=TIP",
            ModifierListOptions {
                category_mask: 0,
                ..Default::default()
            },
        ),
        "tip"
    );
}

#[test]
fn separator_joins_numeric_lines_without_changing_custom_tooltip_lf() {
    let separator = String::from(" / \0ignored");
    assert_eq!(
        describe(
            &fixture(),
            "defense=2 attack=1 custom_modifier_tooltip=TIP",
            ModifierListOptions {
                separator: &separator,
                ..Default::default()
            },
        ),
        "> Attack: \u{11}G+1.00\u{11}! / > Defense: \u{11}G+2.00\u{11}!\ntip"
    );
}

#[test]
fn default_wrapper_uses_the_same_native_options() {
    let fixture = fixture();
    let modifiers = installed_content::modifiers::values(
        &installed_content::parser::parse(b"defense=2 attack=1 custom_modifier_tooltip=TIP")
            .unwrap()
            .items,
    );
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        let text = renderer.modifier_list(&modifiers, ">", scopes).unwrap();
        assert_eq!(
            text,
            "> Attack: \u{11}G+1.00\u{11}!\n> Defense: \u{11}G+2.00\u{11}!\ntip"
        );
        assert_eq!(
            text,
            renderer
                .modifier_list_with(&modifiers, ">", scopes, ModifierListOptions::default())
                .unwrap()
        );
    });
}

#[test]
fn precision_is_signed_and_capped_by_the_definition() {
    let fixture = fixture();
    for (precision, number) in [(1, "+1.2"), (4, "+1.23"), (-1, "+1.23456")] {
        assert_eq!(
            describe(
                &fixture,
                "attack=1.23456",
                ModifierListOptions {
                    precision,
                    ..Default::default()
                },
            ),
            format!("> Attack: \u{11}G{number}\u{11}!")
        );
    }
}

#[test]
fn outer_colour_wraps_even_empty_lists_and_precedes_the_first_separator() {
    let fixture = fixture();
    let options = ModifierListOptions {
        outer_colour: b'Y',
        separator: " / ",
        ..Default::default()
    };
    assert_eq!(describe(&fixture, "", options), "\u{11}Y\u{11}!");
    assert_eq!(
        describe(&fixture, "attack=1 custom_modifier_tooltip=TIP", options),
        "\u{11}Y / > Attack: \u{11}G+1.00\u{11}!\ntip\u{11}!"
    );
}

#[test]
fn excluded_categories_intersect_independently_of_the_include_mask() {
    let mut fixture = fixture();
    definitions(&mut fixture)[1].category_mask = 3;
    // Include bit 1 is excluded, but the definition also has unexcluded bit 2.
    let options = ModifierListOptions {
        category_mask: 1,
        excluded_category_mask: 1,
        ..Default::default()
    };
    assert_eq!(
        describe(
            &fixture,
            "attack=1 defense=2 custom_modifier_tooltip=TIP",
            options,
        ),
        "> Defense: \u{11}G+2.00\u{11}!\ntip"
    );
}

#[test]
fn scale_wraps_and_truncates_after_the_getter_clamp_before_zero_filtering() {
    let mut fixture = fixture();
    assert_eq!(
        describe(
            &fixture,
            "attack=0.33333 defense=-0.00003",
            ModifierListOptions {
                scale: 50_000,
                ..Default::default()
            },
        ),
        "> Attack: \u{11}G+0.16\u{11}!\n> Defense: \u{11}R-0.00\u{11}!"
    );
    assert_eq!(
        describe(
            &fixture,
            "attack=2",
            ModifierListOptions {
                scale: i64::MAX,
                precision: -1,
                ..Default::default()
            },
        ),
        "> Attack: \u{11}R-0.00002\u{11}!"
    );
    definitions(&mut fixture)[0].flags |= engine::modifier_definition::CLAMP_MAXIMUM;
    assert_eq!(
        describe(
            &fixture,
            "attack=2",
            ModifierListOptions {
                scale: 200_000,
                ..Default::default()
            },
        ),
        "> Attack: \u{11}G+2.00\u{11}!"
    );
    assert_eq!(
        describe(&fixture, "attack=-1", ModifierListOptions::default()),
        ""
    );
    definitions(&mut fixture)[1].flags |= 0x100;
    assert_eq!(
        describe(
            &fixture,
            "attack=0.00001 defense=-0.00001",
            ModifierListOptions {
                scale: 50_000,
                ..Default::default()
            },
        ),
        "> Defense: \u{11}Y0.00\u{11}!"
    );
}

#[test]
fn force_plus_overrides_no_plus_only_for_positive_numbers() {
    let mut fixture = fixture();
    definitions(&mut fixture)[0].flags |= crate::modifier_text::NO_PLUS;
    let options = ModifierListOptions {
        force_plus: true,
        ..Default::default()
    };
    assert_eq!(
        describe(&fixture, "attack=1", ModifierListOptions::default()),
        "> Attack: \u{11}G1.00\u{11}!"
    );
    assert_eq!(
        describe(&fixture, "attack=1 defense=-1", options),
        "> Attack: \u{11}G+1.00\u{11}!\n> Defense: \u{11}R-1.00\u{11}!"
    );
}

#[test]
fn invert_changes_good_bad_polarity_and_preserves_neutral() {
    let mut fixture = fixture();
    definitions(&mut fixture)[1].flags = 0;
    let options = ModifierListOptions {
        invert: true,
        ..Default::default()
    };
    assert_eq!(
        describe(&fixture, "attack=1 defense=-1", options),
        "> Attack: \u{11}R+1.00\u{11}!\n> Defense: \u{11}R-1.00\u{11}!"
    );
    definitions(&mut fixture)[1].flags = crate::modifier_text::NEUTRAL;
    assert_eq!(
        describe(&fixture, "defense=-1", options),
        "> Defense: \u{11}Y-1.00\u{11}!"
    );
}

#[test]
fn custom_tooltips_have_no_numeric_prefix_and_no_duplicate_lf() {
    let mut fixture = fixture();
    definitions(&mut fixture)[0].postfix_key = b"tail\n".to_vec();
    assert_eq!(
        describe(
            &fixture,
            "attack=1 custom_modifier_tooltip=TIP custom_modifier_tooltip=\"\"",
            ModifierListOptions::default(),
        ),
        "> Attack: \u{11}G+1.00\u{11}! tail\ntip"
    );
}

#[test]
fn raw_description_leaves_scoped_expansion_to_the_caller() {
    let fixture = fixture();
    let modifiers =
        installed_content::modifiers::values(&installed_content::parser::parse(b"attack=1").unwrap().items);
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        let options = ModifierListOptions::default();
        assert_eq!(
            renderer
                .modifier_description_with(&modifiers, "[THIS.GetTag]", options)
                .unwrap(),
            "[THIS.GetTag] Attack: \u{11}G+1.00\u{11}!"
        );
        assert_eq!(
            renderer
                .modifier_list_with(&modifiers, "[THIS.GetTag]", scopes, options)
                .unwrap(),
            "AAA Attack: \u{11}G+1.00\u{11}!"
        );
    });
}

fn ideas() -> installed_content::ideas::IdeaContent {
    let file = installed_content::ideas::parse_ideas(
        b"ideas={country={SYN_IDEA={research_bonus={industry=0.1} modifier={attack=1}} SECOND_IDEA={}}}",
        &[], &Default::default());
    installed_content::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas)
}

#[test]
fn ideas_single_details_and_multi_names_use_the_native_wrappers() {
    let fixture = fixture();
    let ideas = ideas();
    let names = installed_content::names::NameLists::default();
    let content = Content {
        ideas: Some(&ideas),
        name_lists: Some(&names),
        ..Default::default()
    };
    for (body, expected) in [
        (
            "add_ideas=token:SYN_IDEA",
            "Add Spirit:\nIndustry: 10%\n Attack: \u{11}G+1.00\u{11}!\n",
        ),
        (
            "remove_ideas={SYN_IDEA SECOND_IDEA}",
            "Remove Spirit, Second",
        ),
        ("show_ideas_tooltip=SECOND_IDEA", "Second"),
        (
            "show_ideas_tooltip={SYN_IDEA SECOND_IDEA}",
            "SpiritSpirit, SecondSecond",
        ),
        ("remove_ideas_with_trait=TIP", "Alpha loses tip"),
    ] {
        let description = fixture.render(body, &mut fresh(), content);
        assert!(
            description.refusals.is_empty(),
            "{:?}",
            description.refusals
        );
        assert_eq!(description.text, expected, "{body}");
    }
}

#[test]
fn idea_targeted_operands_keep_case_and_resolve_in_the_native_empty_scope() {
    let mut fixture = fixture();
    *fixture.frame.tables[TableId::GlobalSavedEventTargets.index()] =
        TableView::Complete(TableRows::GlobalSavedEventTargets(vec![
            GlobalSavedEventTargetRow {
                name: b"MixedCase".to_vec(),
                country: 200,
                ..Default::default()
            },
        ]));
    let file = installed_content::ideas::parse_ideas(
        b"ideas={country={
        TARGET_IDEA={targeted_modifier={tag=event_target:MixedCase attack=1}}
        NO_TARGET_IDEA={targeted_modifier={attack=1}}
        SELF_TARGET_IDEA={targeted_modifier={tag=THIS attack=1}}
    }}",
        &[],
        &Default::default(),
    );
    let ideas = installed_content::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = installed_content::names::NameLists::default();
    let content = Content {
        ideas: Some(&ideas),
        name_lists: Some(&names),
        ..Default::default()
    };
    for (body, expected) in [
        (
            "show_ideas_tooltip=TARGET_IDEA",
            "Targeted:\n\u{12}BBBBeta Attack: \u{11}G+1.00\u{11}!\n",
        ),
        (
            "show_ideas_tooltip=NO_TARGET_IDEA",
            "Untargeted:\nAttack: \u{11}G+1.00\u{11}!\n",
        ),
        (
            "show_ideas_tooltip=SELF_TARGET_IDEA",
            "Empty selected scope:\nAttack: \u{11}G+1.00\u{11}!\n",
        ),
    ] {
        let description = fixture.render(body, &mut fresh(), content);
        assert!(
            description.refusals.is_empty(),
            "{:?}",
            description.refusals
        );
        assert_eq!(description.text, expected);
    }
}

#[test]
fn timed_idea_reuses_research_details_and_missing_duration_is_named() {
    let fixture = fixture();
    let ideas = ideas();
    let names = installed_content::names::NameLists::default();
    let content = Content {
        ideas: Some(&ideas),
        name_lists: Some(&names),
        ..Default::default()
    };
    let description = fixture.render(
        "add_timed_idea={idea=SYN_IDEA days=2}",
        &mut fresh(),
        content,
    );
    assert!(
        description.refusals.is_empty(),
        "{:?}",
        description.refusals
    );
    assert_eq!(
        description.text,
        "Timed Spirit for 2 days (Industry: 10%,  Attack: \u{11}G+1.00\u{11}!).\n"
    );
    for body in [
        "modify_timed_idea={idea=SYN_IDEA days=2}",
        "swap_ideas={remove_idea=SYN_IDEA add_idea=SECOND_IDEA}",
    ] {
        let description = fixture.render(body, &mut fresh(), content);
        assert!(description.text.is_empty());
        assert_eq!(description.refusals.values().sum::<usize>(), 1);
        assert!(
            description
                .refusals
                .keys()
                .next()
                .unwrap()
                .contains("CPoliticalStatus timed ideas")
        );
    }
}

#[test]
fn opinion_trade_reverse_and_removal_keep_native_sign_colours() {
    let fixture = fixture();
    let mut opinions = installed_content::opinion_modifiers::Database::default();
    opinions.definitions = installed_content::opinion_modifiers::parse(
        b"opinion_modifiers={SYN_OPINION={value=12} TRADE={value=-3 trade=yes}}",
        [-100, 100],
    )
    .unwrap();
    let content = Content {
        opinion_modifiers: Some(&opinions),
        ..Default::default()
    };
    for (body, expected) in [
        (
            "add_opinion_modifier={target=BBB modifier=SYN_OPINION days=999}",
            "BBB/Beta \u{11}G+12\u{11}! Opinion\n",
        ),
        (
            "reverse_add_opinion_modifier={target=BBB modifier=SYN_OPINION}",
            "Reverse Beta \u{11}G+12\u{11}! Opinion\n",
        ),
        (
            "remove_opinion_modifier={target=BBB modifier=SYN_OPINION}",
            "Remove Beta \u{11}R-12\u{11}! Opinion\n",
        ),
        (
            "add_opinion_modifier={target=BBB modifier=TRADE}",
            "Trade Beta \u{11}R-3\u{11}! TRADE\n",
        ),
    ] {
        let description = fixture.render(body, &mut fresh(), content);
        assert!(
            description.refusals.is_empty(),
            "{:?}",
            description.refusals
        );
        assert_eq!(description.text, expected);
    }
}

#[test]
fn dynamic_scope_values_and_rounded_duration_share_the_evaluator() {
    let fixture = fixture();
    let mut dynamic = installed_content::dynamic_modifiers::Database::default();
    dynamic.definitions.insert(
        "SYN_DYNAMIC".into(),
        installed_content::dynamic_modifiers::Dynamic {
            name: "SYN_DYNAMIC".into(),
            scoped_modifiers: vec![
                ("attack".into(), b"1".to_vec()),
                ("defense".into(), b"2".to_vec()),
            ],
            ..Default::default()
        },
    );
    let content = Content {
        dynamic_modifiers: Some(&dynamic),
        ..Default::default()
    };
    for (body, expected) in [
        (
            "add_dynamic_modifier={modifier=SYN_DYNAMIC scope=BBB days=2.5}",
            "Add Dynamic BBB (Beta) for 3 ( Attack: \u{11}G+1.00\u{11}!\n Defense: \u{11}G+2.00\u{11}!)",
        ),
        (
            "remove_dynamic_modifier={modifier=SYN_DYNAMIC days=constant:not_recorded}",
            "Remove Dynamic AAA ( Attack: \u{11}G+1.00\u{11}!\n Defense: \u{11}G+2.00\u{11}!)",
        ),
    ] {
        let description = fixture.render(body, &mut fresh(), content);
        assert!(
            description.refusals.is_empty(),
            "{:?}",
            description.refusals
        );
        assert_eq!(description.text, expected);
    }
}

#[test]
fn static_relation_and_province_descriptions_preserve_indentation() {
    let fixture = fixture();
    let modifiers =
        installed_content::modifiers::values(&installed_content::parser::parse(b"attack=1").unwrap().items);
    let static_modifiers = installed_content::static_modifiers::Descriptions::from_blocks([
        ("SYN_RELATION".to_owned(), modifiers.clone()),
        ("SECOND_IDEA".to_owned(), Modifiers::default()),
    ]);
    let content = Content {
        static_modifiers: Some(&static_modifiers),
        ..Default::default()
    };
    for (body, expected) in [
        (
            "add_relation_modifier={target=BBB modifier=SYN_RELATION}",
            "Relation Beta Relation ( Attack: \u{11}G+1.00\u{11}!)",
        ),
        (
            "remove_relation_modifier={target=BBB modifier=SYN_RELATION}",
            "Unrelation Beta Relation ( Attack: \u{11}G+1.00\u{11}!)",
        ),
        (
            "add_province_modifier={static_modifiers={SYN_RELATION SECOND_IDEA} days=9}",
            "\nProvince \nRelation:\n     Attack: \u{11}G+1.00\u{11}!\nSecond:\n    \n",
        ),
    ] {
        let description = fixture.render(body, &mut fresh(), content);
        assert!(
            description.refusals.is_empty(),
            "{:?}",
            description.refusals
        );
        assert_eq!(description.text, expected);
    }
}

#[test]
fn state_modifier_uses_breakdown_colour_and_keeps_zero() {
    let fixture = fixture();
    let description = fixture.render(
        "add_state_modifier={modifier={attack=0 defense=2}}",
        &mut fresh(),
        Content::default(),
    );
    assert!(
        description.refusals.is_empty(),
        "{:?}",
        description.refusals
    );
    assert_eq!(
        description.text,
        "State\n\u{11}\u{1} Attack: \u{11}Y0.00\u{11}!\u{11}!\n\u{11}\u{1} Defense: \u{11}G+2.00\u{11}!\u{11}!"
    );
}
