use super::*;
use crate::catalogue::tests::script_game;

const NOW: i32 = game_model::calendar::HOUR_EPOCH + 1003 * 24 + 1;
const SET: i32 = game_model::calendar::HOUR_EPOCH + 1000 * 24 + 5;

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

#[test]
fn country_flags_compare_value_date_and_days() {
    script_game::enter();
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
        (
            1,
            "has_country_flag = { flag = x date > 2.9.28 }",
            Some(true),
        ),
        (
            1,
            "has_country_flag = { flag = x date > 2.9.29 }",
            Some(false),
        ),
        (
            1,
            "has_country_flag = { flag = x date < 2.9.29 }",
            Some(true),
        ),
        (
            1,
            "has_country_flag = { flag = x date = 1.1.1 }",
            Some(true),
        ),
        (1, "has_country_flag = { flag = x days > 2 }", Some(true)),
        (1, "has_country_flag = { flag = x days > 3 }", Some(false)),
        (1, "has_country_flag = { flag = x days < 3 }", Some(false)),
        (1, "has_country_flag = { flag = x days < 4 }", Some(true)),
        (1, "has_country_flag = { flag = x days = 100 }", Some(true)),
        (1, "has_country_flag = { flag = x date = 2.13.1 }", None),
        (1, "has_country_flag = { flag = x@ROOT }", Some(false)),
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

#[test]
fn state_global_and_character_flags_share_the_test() {
    script_game::enter();
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
        cases(
            "has_character_flag = x",
            Scope::Character(9),
            Some(Scope::Character(10)),
        );
        cases("has_global_flag = x", Scope::Country(1), None);
        cases("has_global_flag = x", Scope::State(8), None);
    });
}

#[test]
fn dynamic_names_use_selector_getters_and_keep_invalid_selectors_literal() {
    use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};
    use std::{cell::RefCell, rc::Rc};
    let mut f = flagged();
    let TableView::Complete(TableRows::States(states)) = &mut *f.tables[TableId::States.index()]
    else {
        panic!()
    };
    states.push(replay_data::rows::StateRow {
        id: 9,
        ..Default::default()
    });
    *f.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay_data::rows::PoliticsRow {
            country: 101,
            capital: 7,
            ..Default::default()
        }]));
    let TableView::Complete(TableRows::Politics(politics)) =
        &mut *f.tables[TableId::Politics.index()]
    else {
        panic!()
    };
    politics.insert(
        0,
        replay_data::rows::PoliticsRow {
            country: 100,
            capital: 8,
            ..Default::default()
        },
    );
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *f.tables[TableId::CountryLinks.index()]
    else {
        panic!()
    };
    links.push(CountryLinkRow {
        id: 200,
        canonical_slot: 2,
    });
    let TableView::Complete(TableRows::States(states)) = &mut *f.tables[TableId::States.index()]
    else {
        panic!()
    };
    states.push(replay_data::rows::StateRow {
        id: 10,
        owner: 200,
        ..Default::default()
    });
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let scopes = Scopes {
            from: Some(Scope::Country(2)),
            ..Scopes::of(Scope::Country(1))
        };
        for (name, expected) in [
            ("sample@THIS", "sampleD01"),
            ("sample@root", "sampleD01"),
            ("sample@FROM", "sampleD02"),
            ("sample@PREV", "none"),
            ("sample@capital", "sample7"),
            ("sample@random", "none"),
            ("sample@GER", "sample@GER"),
            ("sample@ROOT.FROM", "sample@ROOT.FROM"),
            ("sample@ROOT@FROM", "sample@ROOT@FROM"),
            ("@ROOT", "@ROOT"),
        ] {
            assert_eq!(
                evaluator
                    .flag_name(name.as_bytes(), scopes)
                    .unwrap()
                    .as_ref(),
                expected.as_bytes(),
                "{name}"
            );
        }
        for (state, name, expected) in [
            (7, "sample@THIS", "sampleGER"),
            (7, "sample@OWNER", "sampleGER"),
            (7, "sample@controller", "sampleGER"),
            (9, "sample@THIS", "sample9"),
            (9, "sample@owner", "none"),
        ] {
            assert_eq!(
                evaluator
                    .flag_name(name.as_bytes(), Scopes::of(Scope::State(state)))
                    .unwrap()
                    .as_ref(),
                expected.as_bytes()
            );
        }
        let unknown = Scopes {
            unknown: UnknownLinks {
                from: true,
                ..Default::default()
            },
            ..scopes
        };
        assert!(evaluator.flag_name(b"sample@FROM", unknown).is_err());
        assert!(
            evaluator
                .flag_name(b"sample@occupied", Scopes::of(Scope::State(7)))
                .is_err()
        );
        assert!(
            evaluator
                .flag_name(b"sample@OWNER", Scopes::of(Scope::State(10)))
                .unwrap_err()
                .to_string()
                .contains("raw country 200")
        );
        let mut graph = EventScopes::new(vec![EventScope::of(EventObject::CountryState {
            country: 1,
            state: 9,
        })]);
        graph.scopes[0].from = Link::Scope(0);
        graph.saved_targets.insert(
            0,
            vec![
                (b"state".to_vec(), EventObject::State(7)),
                (
                    b"combined".to_vec(),
                    EventObject::CountryState {
                        country: 2,
                        state: 9,
                    },
                ),
            ],
        );
        graph.scopes.push(EventScope::of(EventObject::CountryState {
            country: 1,
            state: 7,
        }));
        let evaluator = Evaluator::new(world, None).with_event_scopes(Rc::new(RefCell::new(graph)));
        assert_eq!(
            evaluator
                .flag_name(b"sample@capital", evaluator.event_scopes(1).unwrap())
                .unwrap()
                .as_ref(),
            b"sample8"
        );
        let scopes = evaluator.event_scopes(0).unwrap();
        for (name, expected) in [
            ("sample@THIS", "sampleD01"),
            ("sample@owner", "none"),
            ("sample@event_target:state", "sample7"),
            ("sample@event_target:combined", "sampleD02"),
            ("sample@event_target:missing", "none"),
        ] {
            assert_eq!(
                evaluator
                    .flag_name(name.as_bytes(), scopes)
                    .unwrap()
                    .as_ref(),
                expected.as_bytes(),
                "{name}"
            );
        }
    });
}
