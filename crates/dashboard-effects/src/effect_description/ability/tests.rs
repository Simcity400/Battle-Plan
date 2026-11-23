use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use content_reader_script::modifiers;
use content_reader_parser::parser;
use content_reader_key_names::unit_leader::Adjuster;

fn fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.catalogue.localisation = Arc::new(
        fixture
            .catalogue
            .localisation
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .chain(
                [
                    ("ABILITY_ON_HOURLY", "Hourly"),
                    ("ABILITY_ON_DAILY", "Daily"),
                    ("RAW", "Raw [Root.GetTag]"),
                    ("synthetic", "Synthetic"),
                    ("STAT_ADJUSTER_MOVEMENT", "Move"),
                    ("STAT_ADJUSTER_MOVEMENT_DIFF", "=$VALUE|%0$"),
                    ("STAT_ADJUSTER_ATTACK", "Attack"),
                    ("STAT_ADJUSTER_ATTACK_DIFF", "=$VALUE|%0$"),
                    ("STAT_ADJUSTER_DEFENCE", "Defence"),
                    ("STAT_ADJUSTER_DEFENCE_DIFF", "=$VALUE|%0$"),
                ]
                .into_iter()
                .map(|(key, value)| (key.into(), value.into())),
            )
            .collect(),
    );
    std::sync::Arc::make_mut(&mut fixture.catalogue.localisation).language =
        fixture.catalogue.recorded_language.clone();
    fixture
}

fn items(body: &str) -> Vec<Item> {
    parser::parse(body.as_bytes()).unwrap().items
}

fn render(fixture: &Fixture, definition: &Definition) -> Description {
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        let mut scopes = renderer.graph.borrow().clone();
        ability_description(definition, renderer.context, &mut scopes)
    })
}

#[test]
fn ability_body_order_and_raw_modifier_text_match_calls() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut definition = Definition::default();
    definition.modifiers[0] = modifiers::values(&items("custom_modifier_tooltip=RAW"));
    definition.modifiers[1] = modifiers::values(&items("custom_modifier_tooltip=TIP"));
    definition.effects = std::array::from_fn(|_| items("custom_effect_tooltip=TIP"));
    let result = render(&fixture(), &definition);
    assert_eq!(
        result.text,
        "\nRaw [Root.GetTag]\ntip\ntipHourly\ntipDaily\ntip"
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
}

#[test]
fn ability_empty_body_has_no_leading_newline_or_effect_headings() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let definition = Definition {
        adjusters: vec![Adjuster {
            key: "synthetic".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(render(&fixture(), &definition), Description::default());
}

#[test]
fn ability_adjuster_is_unindented_in_component_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let definition = Definition {
        adjusters: vec![Adjuster {
            key: "synthetic".into(),
            attack: 10_000,
            defence: -20_000,
            movement: 30_000,
        }],
        ..Default::default()
    };
    let result = render(&fixture(), &definition);
    assert_eq!(
        result.text,
        "\n\u{11}HSynthetic\u{11}!\nMove=30%\nAttack=10%\nDefence=-20%\n"
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
}

#[test]
fn ability_failed_accumulator_retains_other_fragments() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut definition = Definition::default();
    definition.modifiers[0] = modifiers::values(&items("political_power_gain=0.1"));
    definition.modifiers[1] = modifiers::values(&items("custom_modifier_tooltip=TIP"));
    definition.effects[2] = items("custom_effect_tooltip=TIP");
    let result = render(&fixture(), &definition);
    assert_eq!(result.text, "\ntip\nDaily\ntip");
    assert_eq!(result.refusals.values().sum::<usize>(), 1);
    assert!(
        result
            .refusals
            .keys()
            .next()
            .unwrap()
            .contains("ability country modifiers")
    );
}
