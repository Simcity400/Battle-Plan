use super::*;

#[test]
fn temp_variables_follow_order_across_scripted_triggers_and_scopes() {
    use installed_content::scripted_triggers::Template;

    let frame = variables_frame();
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
            .evaluate(&script, Scopes::of(Scope::Country(0)))
            .decided(),
        Ok(true)
    );
}

#[test]
fn temp_variables_use_native_fixed5_arithmetic() {
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
    with_world(
        &variables_frame(),
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
                    Scopes::of(Scope::Country(0))
                ),
                Eval::True
            );
        },
    );
}

#[test]
fn temp_variables_override_caller_temps_and_reset_for_the_next_context() {
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
    let frame = variables_frame();
    with_world(&frame, "", Err("none"), |world| {
        let scopes = Scopes::of(Scope::Country(0));
        for (name, form) in [
            ("modulo_temp_variable", "{ t = 2 }"),
            ("set_temp_variable_to_random", "{ var = t min = 0 max = 1 }"),
        ] {
            let evaluator = Evaluator::new(world, None);
            let script = format!(
                "set_temp_variable = {{ t = 1 }} {name} = {form} check_variable = {{ t = 1 }}"
            );
            let Eval::Unknown(reasons) = evaluator.evaluate(&items(&script), scopes) else {
                panic!("{script}");
            };
            assert!(reasons.len() >= 2, "{reasons:?}");
            assert!(
                reasons
                    .iter()
                    .any(|why| why.contains(REFUSED) && why.contains(name)),
                "{reasons:?}"
            );
            assert!(
                evaluator
                    .scoped_value(b"t", scopes)
                    .unwrap_err()
                    .contains(name)
            );
        }
        let why = world
            .evaluate(
                &items("set_temp_variable = { var = t value = { value = 1 pow = 2 } }"),
                scopes,
            )
            .decided()
            .unwrap_err();
        assert!(
            why.contains(REFUSED) && why.contains("CExpression") && why.contains("pow"),
            "{why}"
        );
        // OR is true, but the first condition may skip the setter.
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
