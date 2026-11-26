use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use crate::scripted_text::EventScope;
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

fn fixture() -> Fixture {
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            seed: replay_data::rows::Recorded::Value(None),
            scope_kind: 1,
            scope_id: 100,
            entries: vec![ScriptVariableEntry {
                name: b"x".to_vec(),
                value_raw: 300_000,
            }],
        }]));
    *fixture.frame.tables[TableId::TrainInventory.index()] =
        TableView::Complete(TableRows::TrainInventory(vec![TrainInventoryRow {
            country: 100,
            ..Default::default()
        }]));
    fixture
}

#[test]
fn persistent_arithmetic_keeps_unchanged_prefix_without_rng_seed() {
    game::enter();
    let mut fixture = fixture();
    for seed in [
        Recorded::Value(None),
        Recorded::Value(Some(37)),
        Recorded::NotRecorded,
    ] {
        let TableView::Complete(TableRows::ScriptVariables(rows)) =
            &mut *fixture.frame.tables[TableId::ScriptVariables.index()]
        else {
            panic!("fixture has a complete script variables table");
        };
        rows[0].seed = seed;
        let out = fixture.render(
            "set_variable={var=x value=2 tooltip=VARIABLE_TIP} fixture_points=x",
            &mut fresh(),
            Content::default(),
        );
        assert_eq!(out.text, "AAA:3/2\nXP3\n", "{seed:?}");
        assert!(out.refusals.is_empty(), "{seed:?}: {:?}", out.refusals);
    }
}

#[test]
fn persistent_arithmetic_only_describes_authored_tooltips() {
    game::enter();
    let fixture = fixture();
    for operation in [
        "set_variable",
        "add_to_variable",
        "subtract_from_variable",
        "multiply_variable",
        "divide_variable",
    ] {
        let out = fixture.render(&format!(
            "{operation}={{var=x value={{value=2 add=2}} tooltip=VARIABLE_TIP}} fixture_points=x"
        ), &mut fresh(), Content::default());
        assert_eq!(out.text, "AAA:3/4\nXP3\n", "{operation}");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        let out = fixture.render(
            &format!("{operation}={{var=x value=random_seed tooltip=\"\"}}"),
            &mut fresh(),
            Content::default(),
        );
        assert_eq!(out, Description::default());
    }
}

#[test]
fn tooltip_evaluates_right_before_left_and_repeats_temp_rhs() {
    game::enter();
    let mut fixture = fixture();
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    else {
        panic!("fixture has a complete country appearance table");
    };
    let country = rows.iter_mut().find(|row| row.country == 100).unwrap();
    country.original_tag_reference = Recorded::Value(Some(0));
    country.original_tag = Recorded::Value(Some(Vec::new()));
    *fixture.frame.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "unused alias country lists not sampled".into(),
    };
    let mut names = content_reader_script::key_names::KeyNames::default();
    names.countries = [b"AAA".to_vec()].into();
    names.aliases = [b"ali".to_vec()].into();
    fixture.aliases = Arc::new(content_reader_key_names::tag_aliases::parse(
        &[(
            "common/country_tag_aliases/synthetic.txt",
            b"ALI={original_tag=AAA set_temp_variable={n=9}}".as_slice(),
        )],
        &names,
    ));
    let out = fixture.render(
        "set_temp_variable={n=2} set_variable={var=ALI.x value=n tooltip=VARIABLE_TIP}
         fixture_points=n set_temp_variable={n=ALI.x tooltip=VARIABLE_TIP} fixture_points=n",
        &mut fresh(),
        Content::default(),
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(out.text, "AAA:3/2\nXP9\nAAA:9/3\nXP9\n");
}

#[test]
fn temporary_scalars_execute_before_tooltip_in_one_session() {
    game::enter();
    let fixture = fixture();
    let out = fixture.render(
        "set_temp_variable={n={value=3 add=1} tooltip=VARIABLE_TIP}
         add_to_temp_variable={n=2 tooltip=VARIABLE_TIP}
         subtract_from_temp_variable={n=1 tooltip=VARIABLE_TIP}
         multiply_temp_variable={n=2 tooltip=VARIABLE_TIP}
         divide_temp_variable={n=2 tooltip=VARIABLE_TIP}
         BBB={fixture_points=n}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "AAA:4/4\nAAA:6/2\nAAA:5/1\nAAA:10/2\nAAA:5/2\n\u{12}BBBBeta XP5\n"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render(
        "fixture_points=temp_var:n",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "XP0\n");
}

#[test]
fn effect_division_reads_if_zero_from_tls_even_for_nonzero_divisors() {
    game::enter();
    let fixture = fixture();
    let out = fixture.render(
        "set_temp_variable={n=8} set_temp_variable={fallback=3}
         divide_temp_variable={n=0 if_zero=fallback tooltip=VARIABLE_TIP}
         divide_temp_variable={n=0 if_zero=fallback if_zero=9 tooltip=VARIABLE_TIP}
         divide_temp_variable={n=0 if_zero=9 tooltip=VARIABLE_TIP}
         set_temp_variable={n=8} divide_temp_variable={n=2 if_zero=fallback}
         fixture_points=n",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "AAA:3/0\nAAA:3/0\nAAA:0/0\nXP4\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render(
        "set_temp_variable={n=8} divide_temp_variable={n=2 if_zero=random_seed}",
        &mut fresh(),
        Content::default(),
    );
    assert!(
        out.refusals.keys().any(|why| why.contains("random_seed")),
        "{:?}",
        out.refusals
    );
}

#[test]
fn clamp_and_modulo_execute_but_empty_setters_do_not() {
    game::enter();
    let fixture = fixture();
    let out = fixture.render(
        "set_temp_variable={n=7.9} clamp_temp_variable={var=n min=5 max=2}
         modulo_temp_variable={n=2} fixture_ignored_assignment=n
         fixture_ignored_arguments={var=n min=9 max=9} fixture_ignored_arguments={array=n value=9}
         fixture_points=n",
        &mut fresh(), Content::default(),
    );
    assert_eq!(out.text, "XP1\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render(
        "fixture_ignored_arguments={} fixture_ignored_assignment=x fixture_points=x",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "XP3\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn global_tooltip_target_replaces_in_order_and_cleans_up() {
    game::enter();
    let fixture = fixture();
    let mut graph = fresh();
    let out = fixture.render(
        "fixture_global_target=selected BBB={fixture_global_target=selected}
         event_target:selected={fixture_points=2}",
        &mut graph,
        Content::default(),
    );
    assert_eq!(out.text, "\u{12}BBBBeta XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert!(graph.global_tooltip_targets.is_empty());
    let mut graph = EventScopes::new(vec![EventScope::of(EventObject::CountryState {
        country: 1,
        state: 2,
    })]);
    fixture.with_renderer(&mut graph, Content::default(), |renderer| {
        let evaluator = renderer.evaluator().unwrap();
        let scopes = evaluator.event_scopes(0).unwrap();
        evaluator
            .save_global_tooltip_target(b"selected", scopes)
            .unwrap();
        assert_eq!(
            renderer.graph.borrow().global_tooltip_targets,
            vec![(b"selected".to_vec(), EventObject::State(2))]
        );
    });
}

#[test]
fn known_noops_do_not_need_rng_or_supply_totals() {
    game::enter();
    let fixture = fixture();
    let out = fixture.render(
        "set_temp_variable={n=5} set_temp_variable_to_random={var=n min=3 max=2}
         get_supply_vehicles_temp={var=vehicles type=unknown}
         get_supply_vehicles_temp={var=vehicles type=\"truck\"}
         fixture_points=n fixture_points=vehicles",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "XP5\nXP0\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    for (script, expected) in [
        (
            "set_temp_variable_to_random={var=n}",
            "seed of script variables",
        ),
        (
            "get_supply_vehicles_temp={var=n type=truck need=yes}",
            "train inventory, required trucks raw",
        ),
        (
            "get_supply_vehicles_temp={var=n type=truck}",
            "train inventory, allocated trucks",
        ),
        (
            "get_supply_vehicles_temp={var=n type=train need=yes}",
            "train inventory, required trains",
        ),
        ("fixture_unported={var=n}", "description is unsupported"),
    ] {
        let out = fixture.render(script, &mut fresh(), Content::default());
        assert_eq!(
            out.refusals.values().sum::<usize>(),
            1,
            "{script}: {:?}",
            out.refusals
        );
        assert!(
            out.refusals.keys().any(|why| why.contains(expected)),
            "{script}: {:?}",
            out.refusals
        );
        assert!(
            out.refusals.keys().all(|why| !why.contains("unrecorded")),
            "{script}: {:?}",
            out.refusals,
        );
    }
    let out = fixture.render(
        "set_temp_variable_to_random={var=n} fixture_points=n",
        &mut fresh(),
        Content::default(),
    );
    assert!(out.text.is_empty());
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("temp write is not reproduced"))
    );
}

#[test]
fn format98_random_temp_uses_store_seed_and_keeps_one_tls_session() {
    game::enter();
    let mut fixture = fixture();
    let TableView::Complete(TableRows::ScriptVariables(rows)) =
        &mut *fixture.frame.tables[TableId::ScriptVariables.index()]
    else {
        panic!("complete variables");
    };
    rows[0].seed = Recorded::Value(Some(37));
    let mut graph = fresh();
    fixture.with_renderer(&mut graph, Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();

        assert_eq!(
            renderer
                .evaluator()
                .unwrap()
                .scoped_value(b"random_seed", scopes),
            Ok(967)
        );
        let body =
            content_reader_parser::parser::parse(b"set_temp_variable_to_random={var=n min=2 max=4}")
                .unwrap();
        assert_eq!(renderer.group(&body.items, scopes, 0), "");
        assert_eq!(
            renderer.evaluator().unwrap().scoped_value(b"n", scopes),
            Ok(330_754)
        );
        assert_eq!(
            renderer
                .evaluator()
                .unwrap()
                .scoped_value(b"random_seed", scopes),
            Ok(1_514_732_982_084_589_112)
        );
        let selected = renderer
            .evaluator()
            .unwrap()
            .with_this(scopes, Scope::Country(2))
            .unwrap();
        renderer.evaluator().unwrap().begin_description_in(selected);
        assert_eq!(renderer.group(&body.items, selected, 0), "");
        renderer
            .evaluator()
            .unwrap()
            .end_description(selected)
            .unwrap();
        assert_eq!(
            renderer.evaluator().unwrap().scoped_value(b"n", scopes),
            Ok(318_646)
        );
        assert_eq!(
            renderer
                .evaluator()
                .unwrap()
                .scoped_value(b"random_seed", scopes),
            Ok(8_040_804_592_834_738_808)
        );
        assert!(
            renderer.out.refusals.is_empty(),
            "{:?}",
            renderer.out.refusals
        );
        renderer
            .evaluator()
            .unwrap()
            .refuse_description_writer("synthetic unknown seed write".into());
        renderer.group(&body.items, scopes, 0);
        assert!(
            renderer
                .out
                .refusals
                .keys()
                .any(|why| why.contains("synthetic unknown seed write")),
            "{:?}",
            renderer.out.refusals
        );
    });
    assert_eq!(graph.scopes[0].random_seed, 1);
    let out = fixture.render(
        "set_temp_variable_to_random={var=n min=2 max=4} fixture_points=n",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "XP3\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn format98_random_seed_uses_immediate_from_and_integer_flooring() {
    game::enter();
    let mut fixture = fixture();
    let TableView::Complete(TableRows::ScriptVariables(rows)) =
        &mut *fixture.frame.tables[TableId::ScriptVariables.index()]
    else {
        panic!("complete variables");
    };
    rows[0].seed = Recorded::Value(Some(37));
    let mut graph = EventScopes::new(vec![
        EventScope {
            root: Link::Scope(2),
            from: Link::Scope(1),
            ..EventScope::of(EventObject::Country(1))
        },
        EventScope::of(EventObject::Country(2)),
        EventScope::of(EventObject::State(5)),
    ]);
    fixture.with_renderer(&mut graph, Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        assert_eq!(
            renderer
                .evaluator()
                .unwrap()
                .scoped_value(b"random_seed", scopes),
            Ok(1494)
        );
        let body = content_reader_parser::parser::parse(
            b"set_temp_variable_to_random={var=n min=-2.25 max=3.75 integer=yes}",
        )
        .unwrap();
        renderer.group(&body.items, scopes, 0);
        assert_eq!(
            renderer.evaluator().unwrap().scoped_value(b"n", scopes),
            Ok(200_000)
        );
        assert!(
            renderer.out.refusals.is_empty(),
            "{:?}",
            renderer.out.refusals
        );
    });
}

#[test]
fn format98_variable_rng_selects_state_character_and_global_stores() {
    game::enter();
    let mut fixture = fixture();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(
            [(0, 0, 5), (1, 100, 37), (2, 3, 7), (3, 9, 11)]
                .map(|(scope_kind, scope_id, seed)| ScriptVariablesRow {
                    scope_kind,
                    scope_id,
                    seed: Recorded::Value(Some(seed)),
                    ..Default::default()
                })
                .to_vec(),
        ));
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 9,
            country: 100,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 4,
            character_id: 9,
            ..Default::default()
        }]));
    for (object, expected) in [
        (
            EventObject::CountryState {
                country: 1,
                state: 3,
            },
            967,
        ),
        (EventObject::State(3), 5_317),
        (EventObject::Character(9), 884_937_927),
        (EventObject::UnitLeader(4 << 32), 884_937_927),
        (EventObject::State(0), 35),
    ] {
        let mut graph = EventScopes::new(vec![EventScope::of(object)]);
        fixture.with_renderer(&mut graph, Content::default(), |renderer| {
            let evaluator = renderer.evaluator().unwrap();
            let scopes = evaluator.event_scopes(0).unwrap();
            assert_eq!(evaluator.scoped_value(b"random_seed", scopes), Ok(expected));
        });
    }
}

#[test]
fn format98_random_seed_unavailable_or_old_records_refuse_without_losing_prefix() {
    game::enter();
    let mut fixture = fixture();
    for seed in [Recorded::NotRecorded, Recorded::Value(None)] {
        let TableView::Complete(TableRows::ScriptVariables(rows)) =
            &mut *fixture.frame.tables[TableId::ScriptVariables.index()]
        else {
            panic!("complete variables");
        };
        rows[0].seed = seed;
        let out = fixture.render(
            "fixture_points=2 set_temp_variable_to_random={var=n} fixture_points=n",
            &mut fresh(),
            Content::default(),
        );
        assert_eq!(out.text, "XP2\n");
        assert!(
            out.refusals
                .keys()
                .any(|why| why.contains("seed of script variables")),
            "{:?}",
            out.refusals
        );
        assert!(
            out.refusals
                .keys()
                .any(|why| why.contains("temp write is not reproduced")),
            "{:?}",
            out.refusals
        );
    }
}

#[test]
fn format98_supply_temps_use_stored_scalars_ceiling_and_content_minimum() {
    game::enter();
    let mut fixture = fixture();
    for (kind, need, raw, expected) in [
        ("truck", false, -4, -400_000),
        ("truck", true, 200_001, 300_000),
        ("truck", true, -200_001, -200_000),
        ("truck", true, 0, 0),
        ("truck", true, i64::MAX, 70_231_305_300_000),
        ("truck", true, i64::MIN, 70_231_305_300_000),
        ("train", true, 1, 0),
        ("train", true, 2, 0),
        ("train", true, 3, 300_000),
    ] {
        let TableView::Complete(TableRows::TrainInventory(rows)) =
            &mut *fixture.frame.tables[TableId::TrainInventory.index()]
        else {
            panic!("complete train inventory");
        };
        rows[0].allocated_trucks = Recorded::Value(Some(raw as i32));
        rows[0].required_trucks_raw = Recorded::Value(Some(raw));
        rows[0].required_trains = Recorded::Value(Some(raw as i32));
        let script = format!(
            "get_supply_vehicles_temp={{var=n type={kind} need={}}}",
            if need { "yes" } else { "no" }
        );
        fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
            let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
            let body = content_reader_parser::parser::parse(script.as_bytes()).unwrap();
            assert_eq!(renderer.group(&body.items, scopes, 0), "");
            assert_eq!(
                renderer.evaluator().unwrap().scoped_value(b"n", scopes),
                Ok(expected),
                "{script}/{raw}"
            );
            assert!(
                renderer.out.refusals.is_empty(),
                "{:?}",
                renderer.out.refusals
            );
        });
    }
    let out = fixture.render(
        "get_supply_vehicles_temp={var=n type=train} fixture_points=n",
        &mut fresh(),
        Content::default(),
    );
    assert!(out.text.is_empty());
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("actual train count is not recorded")),
        "{:?}",
        out.refusals
    );
}

#[test]
fn format98_supply_unavailable_and_old_scalars_keep_named_refusals() {
    game::enter();
    let mut fixture = fixture();
    for unavailable in [false, true] {
        let TableView::Complete(TableRows::TrainInventory(rows)) =
            &mut *fixture.frame.tables[TableId::TrainInventory.index()]
        else {
            panic!("complete train inventory");
        };
        rows[0].allocated_trucks = if unavailable {
            Recorded::Value(None)
        } else {
            Recorded::NotRecorded
        };
        rows[0].required_trucks_raw = if unavailable {
            Recorded::Value(None)
        } else {
            Recorded::NotRecorded
        };
        rows[0].required_trains = if unavailable {
            Recorded::Value(None)
        } else {
            Recorded::NotRecorded
        };
        for (kind, need, field) in [
            ("truck", "no", "allocated_trucks"),
            ("truck", "yes", "required_trucks_raw"),
            ("train", "yes", "required_trains"),
        ] {
            let out = fixture.render(
                &format!(
                    "get_supply_vehicles_temp={{var=n type={kind} need={need}}} fixture_points=n"
                ),
                &mut fresh(),
                Content::default(),
            );
            assert!(out.text.is_empty());
            let input = format!("train inventory, {}", field.replace('_', " "));
            assert!(
                out.refusals.keys().any(|why| why.contains(&input)),
                "{:?}",
                out.refusals
            );
            assert!(
                out.refusals
                    .keys()
                    .any(|why| why.contains("temp write is not reproduced")),
                "{:?}",
                out.refusals
            );
        }
    }
}

#[test]
fn withheld_required_trucks_refuses_only_truck_need_while_other_vehicle_temps_work() {
    game::enter();
    let mut fixture = fixture();
    let TableView::Complete(TableRows::TrainInventory(rows)) =
        &mut *fixture.frame.tables[TableId::TrainInventory.index()]
    else {
        panic!("complete train inventory");
    };
    rows[0].allocated_trucks = Recorded::Value(Some(45));
    rows[0].required_trucks_raw = Recorded::Value(None);
    rows[0].required_trains = Recorded::Value(Some(6));

    let refused = fixture.render(
        "get_supply_vehicles_temp={var=n type=truck need=yes} fixture_points=n",
        &mut fresh(),
        Content::default(),
    );
    assert!(refused.text.is_empty());
    assert!(
        refused
            .refusals
            .keys()
            .any(|why| why.contains("train inventory, required trucks raw")),
        "{:?}",
        refused.refusals
    );

    for (script, expected) in [
        ("get_supply_vehicles_temp={var=n type=truck need=no}", 4_500_000),
        ("get_supply_vehicles_temp={var=n type=train need=yes}", 600_000),
    ] {
        fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
            let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
            let body = content_reader_parser::parser::parse(script.as_bytes()).unwrap();
            assert_eq!(renderer.group(&body.items, scopes, 0), "");
            assert_eq!(
                renderer.evaluator().unwrap().scoped_value(b"n", scopes),
                Ok(expected),
                "{script}"
            );
            assert!(renderer.out.refusals.is_empty(), "{:?}", renderer.out.refusals);
        });
    }

    let TableView::Complete(TableRows::TrainInventory(rows)) =
        &mut *fixture.frame.tables[TableId::TrainInventory.index()]
    else {
        panic!("complete train inventory");
    };
    rows[0].required_trucks_raw = Recorded::Value(Some(9_000_000));
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        let body = content_reader_parser::parser::parse(
            b"get_supply_vehicles_temp={var=n type=truck need=yes}",
        )
        .unwrap();
        assert_eq!(renderer.group(&body.items, scopes, 0), "");
        assert_eq!(
            renderer.evaluator().unwrap().scoped_value(b"n", scopes),
            Ok(9_000_000)
        );
        assert!(renderer.out.refusals.is_empty(), "{:?}", renderer.out.refusals);
    });
}
