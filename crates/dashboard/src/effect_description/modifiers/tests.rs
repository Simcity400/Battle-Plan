use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use replay::{frame::TableView, rows::*};

fn fixture() -> Fixture {
    let mut fixture = Fixture::new();
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
