use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use replay_data::rows::*;
use replay_frame::frame::TableView;

pub(super) fn fixture() -> Fixture {
    let mut fixture = Fixture::new();
    let added = [
        ("EFFECT_ADD_IDEA", "Add $NAME$"),
        ("EFFECT_REMOVE_IDEA", "Remove $NAME$"),
        ("EFFECT_SHOW_IDEA_TOOLTIP", "$NAME$"),
        ("EFFECT_SHOW_IDEA_TOOLTIP_DETAILED", "$NAME$:\n$DESC$"),
        ("EFFECT_SWAP_IDEAS_REPLACE", "Replace $REMOVE$/$ADD$"),
        (
            "EFFECT_SWAP_IDEAS_REPLACE_AND_SET_DURATION",
            "Replace set $REMOVE$/$ADD$/$DAYS$",
        ),
        (
            "EFFECT_SWAP_IDEAS_REPLACE_AND_CLEAR_DURATION",
            "Replace clear $REMOVE$/$ADD$",
        ),
        (
            "EFFECT_SWAP_IDEAS_REPLACE_AND_ADD_DURATION",
            "Replace duration $REMOVE$/$ADD$/$DAYS_MODIFIED$/$DAYS$",
        ),
        ("EFFECT_SWAP_IDEAS_MODIFY", "Modify $REMOVE$"),
        (
            "EFFECT_SWAP_IDEAS_MODIFY_AND_SET_DURATION",
            "Modify set $REMOVE$/$DAYS$",
        ),
        (
            "EFFECT_SWAP_IDEAS_MODIFY_AND_CLEAR_DURATION",
            "Modify clear $REMOVE$",
        ),
        (
            "EFFECT_SWAP_IDEAS_MODIFY_AND_ADD_DURATION",
            "Modify duration $REMOVE$/$DAYS_MODIFIED$/$DAYS$",
        ),
        (
            "EFFECT_SWAP_IDEAS_TARGETED_MODIFIERS",
            "$COUNTRY$:\n$MODIFIER$",
        ),
        ("TRIGGER_FULLFILLED_PREFIX", "yes"),
        ("TRIGGER_UNFULLFILLED_PREFIX", "no"),
        ("CAN_SEND_VOLUNTEERS", "Volunteers"),
        ("RULE_DESC", "Rule [$DESC$]"),
        ("synthetic_equipment", "Equipment"),
        ("STAT_ARMY_SOFT_ATTACK", "Soft"),
        ("STAT_ARMY_HARD_ATTACK", "Hard"),
        ("STAT_MODIFIER_DIFF", " ($VALUE|%0$)"),
        ("EFFECT_REMOVE_IDEAS_WITH_TRAIT", "$COUNTRY$ loses $TRAIT$"),
        (
            "EFFECT_MODIFY_TIMED_IDEA_INCREASE",
            "Increase $IDEA$/$TIME_DESC$/$DATE$/$DESC$",
        ),
        (
            "EFFECT_MODIFY_TIMED_IDEA_DECREASE",
            "Decrease $IDEA$/$TIME_DESC$/$DATE$/$DESC$",
        ),
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
    std::sync::Arc::make_mut(&mut fixture.catalogue.localisation).language =
        fixture.catalogue.recorded_language.clone();
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
    let modifiers = content_reader_script::modifiers::values(
        &content_reader_parser::parser::parse(body.as_bytes())
            .unwrap()
            .items,
    );
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        renderer
            .modifier_list_with(&modifiers, ">", scopes, options)
            .unwrap()
    })
}

#[test]
fn category_mask_selects_any_matching_definition_bit() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
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
fn default_options_format_modifier_order_and_tooltip() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_eq!(
        describe(
            &fixture(),
            "defense=2 attack=1 custom_modifier_tooltip=TIP",
            ModifierListOptions::default(),
        ),
        "> Attack: \u{11}G+1.00\u{11}!\n> Defense: \u{11}G+2.00\u{11}!\ntip"
    );
}

#[test]
fn precision_is_signed_and_capped_by_the_definition() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    definitions(&mut fixture)[1].category_mask = 3;
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    definitions(&mut fixture)[0].flags |= engine_native::modifier_definition::CLAMP_MAXIMUM;
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let modifiers = content_reader_script::modifiers::values(
        &content_reader_parser::parser::parse(b"attack=1").unwrap().items,
    );
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

fn ideas() -> content_reader_politics::ideas::IdeaContent {
    let file = content_reader_politics::ideas::parse_ideas(
        b"ideas={country={SYN_IDEA={research_bonus={industry=0.1} modifier={attack=1}} SECOND_IDEA={}}}",
        &[], &Default::default());
    content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas)
}

#[test]
fn ideas_single_details_and_multi_names_use_the_wrappers() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let ideas = ideas();
    let names = content_reader_politics::names::NameLists::default();
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
fn idea_targeted_operands_keep_case_and_resolve_in_the_empty_scope() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    *fixture.frame.tables[TableId::GlobalSavedEventTargets.index()] =
        TableView::Complete(TableRows::GlobalSavedEventTargets(vec![
            GlobalSavedEventTargetRow {
                name: b"MixedCase".to_vec(),
                country: 200,
                ..Default::default()
            },
        ]));
    let file = content_reader_politics::ideas::parse_ideas(
        b"ideas={country={
        TARGET_IDEA={targeted_modifier={tag=event_target:MixedCase attack=1}}
        NO_TARGET_IDEA={targeted_modifier={attack=1}}
        SELF_TARGET_IDEA={targeted_modifier={tag=THIS attack=1}}
    }}",
        &[],
        &Default::default(),
    );
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = content_reader_politics::names::NameLists::default();
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
fn bound_idea_names_use_fresh_country_carrier_and_details_keep_caller_scope() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};
    let mut fixture = fixture();
    fixture.catalogue.localisation = std::sync::Arc::new(
        fixture
            .catalogue
            .localisation
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .chain([
                ("NAME".into(), "Fresh [Root.GetTag]/[This.GetTag]".into()),
                (
                    "DETAIL".into(),
                    "Detail [Root.GetTag]/[This.GetName]".into(),
                ),
                ("ZZ0_SYN_IDEA".into(), "Zero spirit".into()),
            ])
            .collect(),
    );
    std::sync::Arc::make_mut(&mut fixture.catalogue.localisation).language =
        fixture.catalogue.recorded_language.clone();
    *fixture.frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"ZZ0".to_vec(),
                ..Default::default()
            },
        ]));
    let file = content_reader_politics::ideas::parse_ideas(
        b"ideas={country={OWN_IDEA={name=NAME modifier={custom_modifier_tooltip=DETAIL}} SYN_IDEA={modifier={custom_modifier_tooltip=DETAIL}}}}",&[],&Default::default());
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = content_reader_politics::names::NameLists::default();
    let content = Content {
        ideas: Some(&ideas),
        name_lists: Some(&names),
        ..Default::default()
    };
    for (this, token, expected) in [
        (
            EventObject::Country(1),
            "OWN_IDEA",
            "Fresh AAA/AAA:\nDetail BBB/Alpha\n",
        ),
        (
            EventObject::State(1),
            "OWN_IDEA",
            "Fresh ZZ0/ZZ0:\nDetail BBB/One\n",
        ),
        (
            EventObject::State(1),
            "SYN_IDEA",
            "Zero spirit:\nDetail BBB/One\n",
        ),
    ] {
        let mut selected = EventScope::of(this);
        selected.root = Link::Scope(1);
        let mut scopes = EventScopes::new(vec![selected, EventScope::of(EventObject::Country(2))]);
        let result = fixture.render(
            &format!("custom_effect_tooltip=idea_desc|{token}"),
            &mut scopes,
            content,
        );
        assert!(result.refusals.is_empty(), "{:?}", result.refusals);
        assert_eq!(result.text, expected);
    }
}

#[test]
fn timed_idea_reuses_research_details_and_missing_duration_is_named() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    *fixture.frame.tables[TableId::TimedIdeas.index()] = TableView::Unavailable {
        reason: "timed ideas not recorded".into(),
    };
    let ideas = ideas();
    let names = content_reader_politics::names::NameLists::default();
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
                .contains("TimedIdeas")
        );
    }
}

#[test]
fn format99_timed_idea_uses_first_stored_days_and_component_floors() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    fixture.frame.hour = game_model::calendar::HOUR_EPOCH + 1936 * 8760;
    fixture.catalogue.localisation = std::sync::Arc::new(
        fixture
            .catalogue
            .localisation
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .chain([("January".into(), "SyntheticJanuary".into())])
            .collect(),
    );
    std::sync::Arc::make_mut(&mut fixture.catalogue.localisation).language =
        fixture.catalogue.recorded_language.clone();
    *fixture.frame.tables[TableId::TimedIdeas.index()] =
        TableView::Complete(TableRows::TimedIdeas(vec![
            TimedIdeaRow {
                country: 100,
                ordinal: 2,
                token: b"SYN_IDEA".to_vec(),
                days: 99,
            },
            TimedIdeaRow {
                country: 100,
                ordinal: 0,
                token: b"SYN_IDEA".to_vec(),
                days: 5,
            },
        ]));
    let ideas = ideas();
    let names = content_reader_politics::names::NameLists::default();
    let content = Content {
        ideas: Some(&ideas),
        name_lists: Some(&names),
        ..Default::default()
    };
    for (body, prefix) in [
        (
            "modify_timed_idea={idea=SYN_IDEA days=2.9 years=0.9 months=0.9}",
            "Increase Spirit/2 days/8 SyntheticJanuary, 1936/",
        ),
        (
            "modify_timed_idea={idea=SYN_IDEA days=-1.2}",
            "Decrease Spirit//4 SyntheticJanuary, 1936/",
        ),
    ] {
        let description = fixture.render(body, &mut fresh(), content);
        assert!(
            description.refusals.is_empty(),
            "{:?}",
            description.refusals
        );
        assert_eq!(
            description.text,
            format!("{prefix}Industry: 10%,  Attack: \u{11}G+1.00\u{11}!")
        );
    }
    let description = fixture.render(
        "swap_ideas={remove_idea=SYN_IDEA add_idea=SECOND_IDEA add_days=-6}",
        &mut fresh(),
        content,
    );
    assert_eq!(description.text, "Remove Spirit\n");
    assert!(
        description.refusals.is_empty(),
        "{:?}",
        description.refusals
    );
    *fixture.frame.tables[TableId::TimedIdeas.index()] =
        TableView::Complete(TableRows::TimedIdeas(Vec::new()));
    let description = fixture.render(
        "modify_timed_idea={idea=SYN_IDEA days=2}",
        &mut fresh(),
        content,
    );
    assert!(description.text.is_empty() && description.refusals.is_empty());
}

#[test]
fn timed_components_keep_floor_intermediate_wrap() {
    for (value, expected) in [
        (-120_000, -2),
        (290_000, 2),
        (-100_000, -1),
        (i64::MIN, -702_313_053),
        (i64::MAX, -702_313_053),
    ] {
        assert_eq!(timed_component_integer(value), expected, "{value}");
    }
}

#[test]
fn modify_timed_idea_keeps_spaced_colon_comma_that_add_timed_cleans() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    fixture.frame.hour = game_model::calendar::HOUR_EPOCH + 1936 * 8760;
    fixture.catalogue.localisation = std::sync::Arc::new(
        fixture
            .catalogue
            .localisation
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .chain([
                ("COLON_TIP".into(), "HEADER: \nBODY".into()),
                ("January".into(), "January".into()),
            ])
            .collect(),
    );
    std::sync::Arc::make_mut(&mut fixture.catalogue.localisation).language =
        fixture.catalogue.recorded_language.clone();
    *fixture.frame.tables[TableId::TimedIdeas.index()] =
        TableView::Complete(TableRows::TimedIdeas(vec![TimedIdeaRow {
            country: 100,
            token: b"SYN_IDEA".to_vec(),
            days: 5,
            ..Default::default()
        }]));
    let file = content_reader_politics::ideas::parse_ideas(
        b"ideas={country={SYN_IDEA={modifier={custom_modifier_tooltip=COLON_TIP}}}}",
        &[],
        &Default::default(),
    );
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = content_reader_politics::names::NameLists::default();
    let content = Content {
        ideas: Some(&ideas),
        name_lists: Some(&names),
        ..Default::default()
    };
    for (body, expected) in [
        (
            "add_timed_idea={idea=SYN_IDEA days=2}",
            "Timed Spirit for 2 days (HEADER: BODY).\n",
        ),
        (
            "modify_timed_idea={idea=SYN_IDEA days=2}",
            "Increase Spirit/2 days/8 January, 1936/HEADER: , BODY",
        ),
    ] {
        let result = fixture.render(body, &mut fresh(), content);
        assert_eq!(result.text, expected);
        assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    }
}

#[test]
fn swap_ideas_uses_display_name_and_duration_branches() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    let names = content_reader_politics::names::NameLists::default();
    for (same, old, extra, expected) in [
        (false, -1, "", "Replace OLD/NEW"),
        (false, -1, "days=2", "Replace set OLD/NEW/2"),
        (false, 5, "days=-1", "Replace clear OLD/NEW"),
        (false, 5, "days=8", "Replace duration OLD/NEW/3/8"),
        (true, 5, "days=5", "Modify OLD"),
        (true, -1, "days=2", "Modify set OLD/2"),
        (true, 5, "days=-1", "Modify clear OLD"),
        (true, 5, "add_days=1.5 days=99", "Modify duration OLD/2/7"),
    ] {
        let source = format!(
            "ideas={{country={{A={{name=OLD}} B={{name={}}}}}}}",
            if same { "OLD" } else { "NEW" }
        );
        let file = content_reader_politics::ideas::parse_ideas(source.as_bytes(), &[], &Default::default());
        let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
        *fixture.frame.tables[TableId::TimedIdeas.index()] =
            TableView::Complete(TableRows::TimedIdeas(vec![TimedIdeaRow {
                country: 100,
                ordinal: 0,
                token: b"A".to_vec(),
                days: old,
            }]));
        let result = fixture.render(
            &format!("swap_ideas={{remove_idea=A add_idea=B {extra}}}"),
            &mut fresh(),
            Content {
                ideas: Some(&ideas),
                name_lists: Some(&names),
                ..Default::default()
            },
        );
        assert_eq!(result.text, format!("{expected}\n\n"));
        assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    }
}

#[test]
fn swap_ideas_differentials_keep_numeric_research_rules_and_equipment_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let source = b"ideas={country={A={name=OLD modifier={attack=1} research_bonus={industry=0.1} equipment_bonus={synthetic_equipment={soft_attack=0.05}}} B={name=NEW modifier={attack=2 defense=0.5} research_bonus={industry=0.3} rule={can_send_volunteers=yes override={7=RULE_DESC}} equipment_bonus={synthetic_equipment={soft_attack=0.1 hard_attack=0.2}}}}}";
    let file = content_reader_politics::ideas::parse_ideas(source, &[], &Default::default());
    assert!(file.refused.is_none(), "{:?}", file.refused);
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = content_reader_politics::names::NameLists::default();
    let result = fixture.render(
        "swap_ideas={remove_idea=A add_idea=B}",
        &mut fresh(),
        Content {
            ideas: Some(&ideas),
            name_lists: Some(&names),
            ..Default::default()
        },
    );
    assert_eq!(
        result.text,
        concat!(
            "Replace OLD/NEW\n Attack: \u{11}G+1.00\u{11}!\n Defense: \u{11}G+0.50\u{11}!\n",
            "Industry: 20%\n\nRule [yes Volunteers]\n\n",
            "\u{11}YEquipment\u{11}!: \n    Soft (5%)\n    Hard (20%)\n\n\n"
        )
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
}

#[test]
fn swap_ideas_targeted_blocks_use_first_match() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let source = b"ideas={country={A={name=OLD targeted_modifier={tag=BBB attack=0.2}} B={name=NEW modifier={defense=1} targeted_modifier={tag=BBB attack=0.7} targeted_modifier={tag=BBB attack=99}}}}";
    let file = content_reader_politics::ideas::parse_ideas(source, &[], &Default::default());
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = content_reader_politics::names::NameLists::default();
    let content = Content {
        ideas: Some(&ideas),
        name_lists: Some(&names),
        ..Default::default()
    };
    let result = fixture.render(
        "swap_ideas={remove_idea=A add_idea=B}",
        &mut fresh(),
        content,
    );
    assert_eq!(
        result.text,
        "Replace OLD/NEW\n Defense: \u{11}G+1.00\u{11}!\nBeta:\n     Attack: \u{11}G+0.50\u{11}!\n\n"
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
}

#[test]
fn swap_ideas_targeted_first_match_skips_unreached_variable_operands() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *fixture.frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    rows.retain(|row| row.canonical_slot <= 2);
    let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *fixture.frame.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    rows.retain(|row| row.canonical_slot <= 2);
    *fixture.frame.tables[TableId::ScriptVariables.index()] = TableView::Unavailable {
        reason: "unreached target variable".into(),
    };
    let source = b"ideas={country={A={name=OLD targeted_modifier={} targeted_modifier={tag=AAA} targeted_modifier={tag=BBB attack=0.2} targeted_modifier={tag=var:missing}} B={name=NEW targeted_modifier={} targeted_modifier={tag=AAA defense=0.1} targeted_modifier={tag=BBB attack=0.7} targeted_modifier={tag=var:missing}}}}";
    let file = content_reader_politics::ideas::parse_ideas(source, &[], &Default::default());
    assert!(file.refused.is_none(), "{:?}", file.refused);
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = content_reader_politics::names::NameLists::default();
    let result = fixture.render(
        "swap_ideas={remove_idea=A add_idea=B}",
        &mut fresh(),
        Content {
            ideas: Some(&ideas),
            name_lists: Some(&names),
            ..Default::default()
        },
    );
    assert_eq!(
        result.text,
        "Replace OLD/NEW\nAlpha:\n     Defense: \u{11}G+0.10\u{11}!\nBeta:\n     Attack: \u{11}G+0.50\u{11}!\n\n"
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
}

#[test]
fn swap_ideas_keeps_available_fragments_when_numeric_definitions_are_unavailable() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = fixture();
    let source = b"ideas={country={A={name=OLD} B={name=NEW modifier={defense=1} rule={can_send_volunteers=yes}}}}";
    let file = content_reader_politics::ideas::parse_ideas(source, &[], &Default::default());
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = content_reader_politics::names::NameLists::default();
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] = TableView::Unavailable {
        reason: "numeric definitions unavailable".into(),
    };
    let result = fixture.render(
        "swap_ideas={remove_idea=A add_idea=B}",
        &mut fresh(),
        Content {
            ideas: Some(&ideas),
            name_lists: Some(&names),
            ..Default::default()
        },
    );
    assert_eq!(result.text, "Replace OLD/NEW\nyes Volunteers\n\n\n");
    assert_eq!(result.refusals.values().sum::<usize>(), 1);
    assert!(
        result
            .refusals
            .keys()
            .next()
            .unwrap()
            .contains("numeric definitions unavailable")
    );
}

#[test]
fn swap_ideas_numeric_accumulator_drops_source_hidden_flags_children_and_tooltips() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let source = b"ideas={country={A={name=OLD modifier={attack=0.5}} B={name=NEW modifier={hidden_modifier={attack=1} added_modifier={defense=99} custom_modifier_tooltip=TIP}}}}";
    let file = content_reader_politics::ideas::parse_ideas(source, &[], &Default::default());
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = content_reader_politics::names::NameLists::default();
    let result = fixture.render(
        "swap_ideas={remove_idea=A add_idea=B}",
        &mut fresh(),
        Content {
            ideas: Some(&ideas),
            name_lists: Some(&names),
            ..Default::default()
        },
    );
    assert_eq!(
        result.text,
        "Replace OLD/NEW\n Attack: \u{11}G+0.50\u{11}!\n\n"
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
}

#[test]
fn swap_ideas_rule_reset_keeps_blank_lines() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let source = b"ideas={country={A={name=OLD rule={can_send_volunteers=yes}} B={name=NEW}}}";
    let file = content_reader_politics::ideas::parse_ideas(source, &[], &Default::default());
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = content_reader_politics::names::NameLists::default();
    let result = fixture.render(
        "swap_ideas={remove_idea=A add_idea=B}",
        &mut fresh(),
        Content {
            ideas: Some(&ideas),
            name_lists: Some(&names),
            ..Default::default()
        },
    );
    assert_eq!(result.text, "Replace OLD/NEW\n\n\n\n");
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
}

#[test]
fn opinion_trade_reverse_and_removal_keep_sign_colours() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let mut opinions = content_reader_politics::opinion_modifiers::Database::default();
    opinions.definitions = content_reader_politics::opinion_modifiers::parse(
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let mut dynamic = content_reader_politics::dynamic_modifiers::Database::default();
    dynamic.definitions.insert(
        "SYN_DYNAMIC".into(),
        content_reader_politics::dynamic_modifiers::Dynamic {
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let modifiers = content_reader_script::modifiers::values(
        &content_reader_parser::parser::parse(b"attack=1").unwrap().items,
    );
    let static_modifiers = content_reader_politics::static_modifiers::Descriptions::from_blocks([
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
fn scaled_static_modifiers_scale_each_value_and_prefix_each_line() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = fixture();
    let modifiers = content_reader_script::modifiers::values(
        &content_reader_parser::parser::parse(b"attack=1 defense=2")
            .unwrap()
            .items,
    );
    let static_modifiers = content_reader_politics::static_modifiers::Descriptions::from_blocks([(
        "SYN_NETWORK".to_owned(),
        modifiers,
    )]);
    let content = Content {
        static_modifiers: Some(&static_modifiers),
        ..Default::default()
    };
    let list = |scale| {
        fixture.with_context(content, |context| {
            crate::effect_description::scaled_static_modifiers(context, "SYN_NETWORK", "  ", scale)
        })
    };
    assert_eq!(
        list(25_000),
        Ok("   Attack: \u{11}G+0.25\u{11}!\n   Defense: \u{11}G+0.50\u{11}!".to_owned())
    );
    assert_eq!(
        list(100_000),
        Ok("   Attack: \u{11}G+1.00\u{11}!\n   Defense: \u{11}G+2.00\u{11}!".to_owned())
    );
}

#[test]
fn state_modifier_uses_breakdown_colour_and_keeps_zero() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
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
