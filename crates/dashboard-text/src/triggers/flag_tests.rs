//! The shared flag test, as `has_country_flag` reads it.
use super::*;

/// The frame's hour: three days and an hour past the day the flags were set.
const NOW: i32 = engine_native::cdate::HOUR_EPOCH + 1003 * 24 + 1;
/// The hour the flags were set: day 1000 (year 2, 28 September), hour 5.
const SET: i32 = engine_native::cdate::HOUR_EPOCH + 1000 * 24 + 5;

/// Flag `x`, set at [`SET`], at hour [`NOW`]: of value 2 in the global store, on D01 (slot 1),
/// state 7 and character 9; of value 0 on D02 (slot 2), state 8 and character 10.
fn flagged() -> DecodedFrame {
    let mut f = frame(&countries(), &[]);
    f.hour = NOW;
    *f.tables[TableId::ScriptFlags.index()] = TableView::Complete(TableRows::ScriptFlags(
        [
            (0, 0, 2),
            (1, 101, 2),
            (1, 102, 0),
            (2, 7, 2),
            (2, 8, 0),
            (world::CHARACTER_FLAGS, 9, 2),
            (world::CHARACTER_FLAGS, 10, 0),
        ]
        .into_iter()
        .map(|(scope_kind, scope_id, value)| ScriptFlagRow {
            scope_kind,
            scope_id,
            token: 900,
            set_hour: SET,
            value,
            remaining_days: 0,
        })
        .collect(),
    ));
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        [7, 8]
            .into_iter()
            .map(|id| replay_data::rows::StateRow {
                id,
                owner: 100,
                controller: 100,
                ..Default::default()
            })
            .collect(),
    ));
    f
}

/// Without `value` a flag holds only above zero; `value` compares by its operator; `date`
/// compares the setting hour strictly; `days` compares day starts strictly; `date =` and
/// `days =` add no test; a quoted name is the same flag; a flag named through a country or
/// state, and a reader's error path, are refused.
#[test]
fn country_flags_compare_value_date_and_days() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = flagged();
    for (slot, trigger, expected) in [
        (1, "has_country_flag = x", Some(true)),
        (1, "has_country_flag = \"X\"", Some(true)),
        (2, "has_country_flag = x", Some(false)),
        (0, "has_country_flag = x", Some(false)),
        (2, "has_country_flag = { flag = x value = 0 }", Some(true)),
        (1, "has_country_flag = { flag = x value > 1 }", Some(true)),
        (1, "has_country_flag = { flag = x value > 2 }", Some(false)),
        (1, "has_country_flag = { flag = x value < 2 }", Some(false)),
        (1, "has_country_flag = { flag = x date > 2.9.28 }", Some(true)),
        (1, "has_country_flag = { flag = x date > 2.9.29 }", Some(false)),
        (1, "has_country_flag = { flag = x date < 2.9.29 }", Some(true)),
        (1, "has_country_flag = { flag = x date = 1.1.1 }", Some(true)),
        (1, "has_country_flag = { flag = x days > 2 }", Some(true)),
        (1, "has_country_flag = { flag = x days > 3 }", Some(false)),
        (1, "has_country_flag = { flag = x days < 3 }", Some(false)),
        (1, "has_country_flag = { flag = x days < 4 }", Some(true)),
        (1, "has_country_flag = { flag = x days = 100 }", Some(true)),
        (1, "has_country_flag = { flag = x date = 2.13.1 }", None),
        (1, "has_country_flag = { flag = x@ROOT }", None),
        (1, "has_country_flag = { flag = x unknown = 1 }", None),
        (1, "has_country_flag = { flag = x flag = x }", None),
    ] {
        let value = check(&f, "", slot, trigger);
        match expected {
            Some(expected) => assert_eq!(value, Eval::from_bool(expected), "{trigger}"),
            None => assert!(matches!(value, Eval::Unknown(_)), "{trigger}: {value:?}"),
        }
    }
}

/// State, global and character flags share the country flag's test, each on its own scope's
/// record: without `value` the value must be above zero, and `value`, `date` and `days`
/// compare as they do for countries.
#[test]
fn state_global_and_character_flags_share_the_test() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = flagged();
    with_world(&f, "", Err("none"), |world| {
        let cases = |trigger: &str, positive: Scope, zero: Option<Scope>| {
            let at = |scope: Scope, text: &str| world.evaluate(&items(text), Scopes::of(scope));
            let name = trigger.split(' ').next().unwrap();
            assert_eq!(at(positive, trigger), Eval::True, "{trigger}");
            if let Some(zero) = zero {
                assert_eq!(at(zero, trigger), Eval::False, "{trigger} at zero");
                let explicit = format!("{name} = {{ flag = x value = 0 }}");
                assert_eq!(at(zero, &explicit), Eval::True, "{explicit}");
            }
            for (block, expected) in [
                ("value > 2", Eval::False),
                ("value > 1", Eval::True),
                ("date > 2.9.28", Eval::True),
                ("days > 2", Eval::True),
                ("days > 3", Eval::False),
            ] {
                let text = format!("{name} = {{ flag = x {block} }}");
                assert_eq!(at(positive, &text), expected, "{text}");
            }
        };
        cases("has_state_flag = x", Scope::State(7), Some(Scope::State(8)));
        cases("has_character_flag = x", Scope::Character(9), Some(Scope::Character(10)));
        // The global store is one, whatever scope asks.
        cases("has_global_flag = x", Scope::Country(1), None);
        cases("has_global_flag = x", Scope::State(8), None);
    });
}
