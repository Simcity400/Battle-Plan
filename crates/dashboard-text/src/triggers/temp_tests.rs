use super::*;
use crate::catalogue::tests::script_game;

fn temp_effects(text: &str) -> content_reader_script::native::Read {
    use content_reader_script::native::{self, Shape};
    native::read(
        text.as_bytes(),
        native::Mode::KeepMark,
        native::Top::File(()),
        Some(&names()),
        &mut |(), _| Shape::Effect,
    )
}

#[test]
fn temp_colour_lifetime_depends_on_the_outer_variable_context() {
    script_game::enter();
    with_world(&variables_frame(), "", Err("none"), |world| {
        let scopes = Scopes::of(Scope::Country(0));
        let evaluator = Evaluator::new(world, None);
        let scripts = [
            "set_temp_variable={alpha=1} set_temp_variable={red=1}",
            "set_temp_variable={alpha=1} set_temp_variable={red=0} set_temp_variable={green=0} set_temp_variable={blue=1}",
            "set_temp_variable={alpha=1} set_temp_variable={green=1}",
            "set_temp_variable={alpha=1} set_temp_variable={red=1}",
        ].map(items);
        let run = || {
            scripts
                .iter()
                .map(|script| {
                    assert_eq!(evaluator.evaluate_colour(script, scopes, &[]), Eval::True);
                    [b"red".as_slice(), b"green", b"blue"]
                        .map(|key| evaluator.temp_value(key).unwrap_or(Ok(0)).unwrap())
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), [[ONE, 0, 0], [0, 0, ONE], [0, ONE, 0], [ONE, 0, 0]]);
        assert_eq!(
            evaluator.with_variable_context(scopes, run),
            [[ONE, 0, 0], [0, 0, ONE], [0, ONE, ONE], [ONE, ONE, ONE]]
        );
        assert_eq!(
            run()[0],
            [ONE, 0, 0],
            "the next standalone check clears TLS"
        );
    });
}

#[test]
fn temp_inherited_context_requires_proved_reads_even_inside_nested_checks() {
    script_game::enter();
    with_world(&variables_frame(), "", Err("none"), |world| {
        let scopes = Scopes::of(Scope::Country(0));
        let evaluator = Evaluator::new(world, None).with_temps(Temps::Inherited);
        for _ in 0..2 {
            assert_eq!(
                evaluator.evaluate_colour(&items("set_temp_variable={red=1}"), scopes, &[]),
                Eval::True
            );
            assert_eq!(evaluator.temp_value(b"red"), Some(Ok(ONE)));
            assert!(
                evaluator
                    .temp_value(b"blue")
                    .unwrap()
                    .unwrap_err()
                    .contains("caller temporary")
            );
            for script in [
                "set_temp_variable={red=missing?0}",
                "OR={check_variable={missing=0} always=yes}",
                "set_temp_variable_to_random={var=red min=0 max=1}",
                "divide_temp_variable={var=red?1 value=0} check_variable={red=0}",
                "modulo_temp_variable={var=red?1 value=0} check_variable={red=0}",
                "add_to_temp_variable={red=1}",
            ] {
                let Eval::Unknown(failures) =
                    evaluator.evaluate_colour(&items(script), scopes, &[])
                else {
                    panic!("inherited input must refuse: {script}");
                };
                assert_eq!(failures.len(), 1, "{failures:?}");
                assert!(
                    failures.iter().next().unwrap().contains("caller temporary"),
                    "{failures:?}"
                );
            }
            assert_eq!(evaluator.evaluate_colour(&items("set_temp_variable={red=0.25} set_temp_variable={green=0.5} set_temp_variable={blue=0.75} set_temp_variable={alpha=1} set_temp_variable={highlighted=0}"), scopes, &[]), Eval::True);
            assert_eq!(evaluator.temp_value(b"blue"), Some(Ok(75_000)));
        }
    });
}

#[test]
fn temp_modulo_uses_integer_operands_and_guards() {
    script_game::enter();
    let frame = with_variables(variables_frame(), &[(1, 100, &[("bottom", i64::MIN)])]);
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let scopes = Scopes::of(Scope::Country(0));
        for (lhs, rhs, expected) in [
            ("8.9", "2.5", 2 * ONE),
            ("8.9", "2.49999", 0),
            ("7.9", "0.49999", 790_000),
            ("7.9", "-2", 790_000),
            ("-2.5", "stability", -250_000),
            ("2147483648", "stability", 214_748_364_800_000),
            ("ROOT.bottom", "stability", i64::MIN),
            ("4294967303.9", "3", ONE),
            ("7.9", "4294967299", ONE),
        ] {
            let script = format!(
                "set_temp_variable = {{ n = {lhs} }} modulo_temp_variable = {{ n = {rhs} }}"
            );
            assert_eq!(
                evaluator.evaluate(&items(&script), scopes).decided(),
                Ok(true),
                "{script}"
            );
            assert_eq!(
                evaluator.scoped_value(b"n", scopes),
                Ok(expected),
                "{script}"
            );
        }
    });
}

#[test]
fn temp_modulo_consumes_the_effect_reader_form() {
    script_game::enter();
    let read = temp_effects(
        "body = { set_temp_variable = { n = 8.9 }
                  modulo_temp_variable = { var = n value = 2.5 }
                  set_temp_variable = { var = tooltip value = 8.9 }
                  modulo_temp_variable = { tooltip = 3 } }",
    );
    assert!(
        read.misreads.is_empty() && read.unsettled.is_none(),
        "{read:?}"
    );
    with_world(&variables_frame(), "", Err("none"), |world| {
        let scopes = Scopes::of(Scope::Country(0));
        let evaluator = Evaluator::new(world, None);
        assert_eq!(
            evaluator
                .evaluate(read.items[0].value.items().unwrap(), scopes)
                .decided(),
            Ok(true)
        );
        assert_eq!(evaluator.scoped_value(b"n", scopes), Ok(2 * ONE));
        assert_eq!(evaluator.scoped_value(b"tooltip", scopes), Ok(2 * ONE));
    });
}

#[test]
fn temp_random_bare_scalar_keeps_the_counted_reader_refusal() {
    script_game::enter();
    let read = temp_effects("body = { set_temp_variable_to_random = runtime_name }");
    let refusals: Vec<_> = read
        .misreads
        .iter()
        .filter(|note| {
            note.what
                .starts_with("Deferred effect `set_temp_variable_to_random`")
        })
        .collect();
    assert_eq!(refusals.len(), 1, "{read:?}");
    assert!(
        refusals[0].what.contains("its value reader is unsupported")
            && refusals[0].what.contains("by-key reading retained"),
        "{read:?}"
    );
}

#[test]
fn temp_modulo_caches_the_target_before_rhs_alias_mutations() {
    script_game::enter();
    with_world(
        &positive_variables_frame(),
        "ALI = { original_tag = GER set_temp_variable = { n = 9 } }",
        Err("none"),
        |world| {
            assert_eq!(
                world
                    .evaluate(
                        &items(
                            "set_temp_variable = { n = 7.9 }
                         modulo_temp_variable = { n = ALI.x }
                         check_variable = { n = 2 }
                         set_temp_variable = { n = 7.9 }
                         modulo_temp_variable = { n = ALI.missing }
                         check_variable = { n = 9 }",
                        ),
                        Scopes::of(Scope::Country(1)),
                    )
                    .decided(),
                Ok(true)
            );
        },
    );
}

#[test]
fn temp_variables_follow_order_across_scripted_triggers_and_scopes() {
    script_game::enter();
    use content_reader_script::scripted_trigger_grammar::Template;

    let frame = positive_variables_frame();
    let mut triggers = TriggerNames::default();
    triggers.scripted.templates.insert(
        b"nested".to_vec(),
        Template {
            name: b"nested".to_vec(),
            body: Ok(items(
                "add_to_temp_variable = { var = x value = 2.25 }
                 multiply_temp_variable = { x = 1.5 }
                 subtract_from_temp_variable = { x = 0.375 }
                 divide_temp_variable = { x = 2 }
                 clamp_temp_variable = { var = x min = 0.5 max = 1.25 }
                 round_temp_variable = x
                 64 = { add_to_temp_variable = { x = 2 } }",
            )),
        },
    );
    let aliases = Arc::new(TagAliases::default());
    let dlcs = BTreeSet::new();
    let world = World::read(
        &frame,
        ContentFacts::content_only(&dlcs, &triggers, &aliases),
    );
    let script = items(
        "set_temp_variable = { x = x }
         check_variable = { temp_var:x = 0 }
         nested = yes
         check_variable = { x = 3 }
         if = { limit = { always = no } set_temp_variable = { x = 90 } }
         check_variable = { x = 3 }
         set_temp_variable = { y@ROOT^1 = 6 }
         GER = { add_to_temp_variable = { y@ROOT^1 = 2 } }
         check_variable = { y@ROOT^1 = 8 }
         check_variable = { ROOT.x = 5 }",
    );
    assert_eq!(
        world
            .evaluate(&script, Scopes::of(Scope::Country(1)))
            .decided(),
        Ok(true)
    );
}

#[test]
fn temp_variables_use_fixed5_arithmetic() {
    script_game::enter();
    let frame = with_variables(variables_frame(), &[(1, 100, &[("huge", i64::MAX)])]);
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let scopes = Scopes::of(Scope::Country(0));
        for (script, expected) in [
            (
                "set_temp_variable = { n = -2.5 } round_temp_variable = n",
                -3 * ONE,
            ),
            (
                "set_temp_variable = { n = 2.5 } round_temp_variable = n",
                3 * ONE,
            ),
            (
                "set_temp_variable = { n = -7 } divide_temp_variable = { n = 2 }",
                -350_000,
            ),
            (
                "set_temp_variable = { n = -2.5 } divide_temp_variable = { n = 0 }",
                -250_000,
            ),
            (
                "set_temp_variable = { n = 5 } clamp_temp_variable = { var = n min = 9 max = 2 }",
                9 * ONE,
            ),
            (
                "set_temp_variable = { n = ROOT.huge } multiply_temp_variable = { n = 2 }",
                -2,
            ),
            (
                "set_temp_variable = { n = ROOT.huge } divide_temp_variable = { n = 2 }",
                0xffff_ffff,
            ),
            ("add_to_temp_variable = { n?4 = 1 }", 5 * ONE),
            (
                "set_temp_variable = { n = 3 } set_temp_variable = { var = n }",
                0,
            ),
            (
                "set_temp_variable = { n = { value = 2 add = { value = 3 multiply = 2 } } }",
                8 * ONE,
            ),
        ] {
            assert_eq!(
                evaluator.evaluate(&items(script), scopes).decided(),
                Ok(true),
                "{script}"
            );
            assert_eq!(
                evaluator.scoped_value(b"n", scopes),
                Ok(expected),
                "{script}"
            );
        }
    });
}

#[test]
fn temp_variables_read_the_destination_after_rhs_alias_conditions() {
    script_game::enter();
    with_world(
        &positive_variables_frame(),
        "ALI = { original_tag = GER set_temp_variable = { n = 7 }
            add_to_temp_variable = { hits = 1 } }",
        Err("none"),
        |world| {
            assert_eq!(
                world.evaluate(
                    &items(
                        "set_temp_variable = { n = 2 }
                     add_to_temp_variable = { n = ALI.x } check_variable = { n = 12 }
                     set_temp_variable = { n = 2 }
                     clamp_temp_variable = { var = n min = ALI.x max = 9 }
                     check_variable = { n = 7 }
                     set_temp_variable = { hits = 0 }
                     set_temp_variable = { k@var:ALI.x = 1 }
                     check_variable = { hits = 2 }
                     check_variable = { temp_var:k5 = 1 }"
                    ),
                    Scopes::of(Scope::Country(1))
                ),
                Eval::True
            );
        },
    );
}

#[test]
fn temp_variables_override_caller_temps_and_reset_for_the_next_context() {
    script_game::enter();
    let frame = variables_frame();
    let scopes = Scopes {
        target_state: Some(64),
        target_province: Some(42),
        target_leader: Some(1),
        ..Scopes::of(Scope::Country(0))
    };
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        assert_eq!(
            evaluator
                .evaluate(
                    &items(
                        "var:target_state = { always = yes }
                     set_temp_variable = { target_state = 1 }
                     set_temp_variable = { target_province = 99 }
                     set_temp_variable = { target_leader = 2 }
                     set_temp_variable = { t = 7 }
                     NOT = { var:target_state = { always = yes } }",
                    ),
                    scopes,
                )
                .decided(),
            Ok(true)
        );
        for (key, value) in [
            (b"target_state".as_slice(), ONE),
            (b"target_province".as_slice(), 99 * ONE),
            (b"target_leader".as_slice(), 2 * ONE),
            (b"t".as_slice(), 7 * ONE),
        ] {
            assert_eq!(evaluator.scoped_value(key, scopes), Ok(value));
        }
        assert_eq!(
            evaluator
                .evaluate(&items("check_variable = { temp_var:t = 0 }"), scopes)
                .decided(),
            Ok(true)
        );
        assert_eq!(
            evaluator.scoped_value(b"target_province", scopes),
            Ok(42 * ONE)
        );
    });
}

#[test]
fn temp_variables_keep_pending_writers_expressions_and_residuals_named() {
    script_game::enter();
    let frame = variables_frame();
    with_world(&frame, "", Err("none"), |world| {
        let scopes = Scopes::of(Scope::Country(0));
        let evaluator = Evaluator::new(world, None);
        let script = "set_temp_variable={t=1} set_temp_variable_to_random={var=t min=0 max=1} check_variable={t=1}";
        let Eval::Unknown(reasons) = evaluator.evaluate(&items(script), scopes) else {
            panic!("{script}");
        };
        assert!(
            reasons
                .iter()
                .any(|why| why.contains("seed of script variables")),
            "{reasons:?}"
        );
        assert!(
            evaluator
                .scoped_value(b"t", scopes)
                .unwrap_err()
                .contains("set_temp_variable_to_random")
        );
        let why = world
            .evaluate(
                &items("set_temp_variable = { var = t value = { value = 1 pow = 2 } }"),
                scopes,
            )
            .decided()
            .unwrap_err();
        assert!(
            why.contains(REFUSED) && why.contains("expression") && why.contains("pow"),
            "{why}"
        );
        let why = world
            .evaluate(
                &items(
                    "OR = { check_variable = { stability > 0 } set_temp_variable = { t = 9 } }
             check_variable = { t = 9 }",
                ),
                scopes,
            )
            .decided()
            .unwrap_err();
        assert!(why.contains("unknown earlier condition"), "{why}");
        let stale = Evaluator::new(world, None).with_temps(Temps::Stale);
        let why = stale.scoped_value(b"t", scopes).unwrap_err();
        assert!(
            why.contains("t") && why.contains("outside a script context"),
            "{why}"
        );
        assert_eq!(stale.scoped_value(b"ROOT.x", scopes), Ok(5 * ONE));
    });
}

#[test]
fn format98_random_temp_trigger_and_mtth_share_the_seed_context() {
    script_game::enter();
    let mut frame = variables_frame();
    let TableView::Complete(TableRows::ScriptVariables(rows)) =
        &mut *frame.tables[TableId::ScriptVariables.index()]
    else {
        panic!("complete variables");
    };
    for row in rows
        .iter_mut()
        .filter(|row| row.scope_kind == 1 && matches!(row.scope_id, 100 | 101))
    {
        row.seed = replay_data::rows::Recorded::Value(Some(37));
    }
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        assert_eq!(
            evaluator
                .evaluate(
                    &items("check_variable={random_seed=0.00037}"),
                    Scopes::of(Scope::Country(0))
                )
                .decided(),
            Ok(true)
        );
        let scopes = Scopes::of(Scope::Country(1));
        assert_eq!(
            evaluator
                .evaluate(
                    &items(
                        "set_temp_variable_to_random={var=n min=2 max=4} check_variable={n=3.30754}"
                    ),
                    scopes
                )
                .decided(),
            Ok(true)
        );
        let fields = content_reader_parser::parser::parse(
            b"base=1 modifier={always=yes factor=random} modifier={always=yes factor=random}",
        )
        .unwrap()
        .items;
        assert_eq!(
            evaluator.mean_time_to_happen("test", &fields, scopes),
            Ok(38_783)
        );
    });
}
