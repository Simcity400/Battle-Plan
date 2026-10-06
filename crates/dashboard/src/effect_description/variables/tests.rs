use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use crate::scripted_text::EventScope;
use replay::{frame::TableView, rows::*};

fn fixture() -> Fixture {
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            entries: vec![ScriptVariableEntry { name: b"x".to_vec(), value_raw: 300_000 }],
        }]));
    fixture
}

#[test]
fn persistent_arithmetic_only_describes_authored_tooltips() {
    let fixture = fixture();
    for operation in ["set_variable", "add_to_variable", "subtract_from_variable", "multiply_variable", "divide_variable"] {
        let out = fixture.render(&format!(
            "{operation}={{var=x value={{value=2 add=2}} tooltip=VARIABLE_TIP}} army_experience=x"
        ), &mut fresh(), Content::default());
        assert_eq!(out.text, "AAA:3/4\nXP3\n", "{operation}");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        // 14039A7D0 never evaluates the otherwise unsupported RHS without a tooltip.
        let out = fixture.render(&format!("{operation}={{var=x value=random_seed tooltip=\"\"}}"), &mut fresh(), Content::default());
        assert_eq!(out, Description::default());
    }
}

#[test]
fn tooltip_evaluates_right_before_left_and_repeats_temp_rhs() {
    let mut fixture = fixture();
    let mut names = installed_content::key_names::KeyNames::default();
    names.countries = [b"AAA".to_vec()].into();
    names.aliases = [b"ali".to_vec()].into();
    fixture.aliases = Arc::new(installed_content::tag_aliases::parse(&[(
        "common/country_tag_aliases/synthetic.txt",
        b"ALI={original_tag=AAA set_temp_variable={n=9}}".as_slice(),
    )], &names));
    let out = fixture.render(
        "set_temp_variable={n=2} set_variable={var=ALI.x value=n tooltip=VARIABLE_TIP}
         army_experience=n set_temp_variable={n=ALI.x tooltip=VARIABLE_TIP} army_experience=n",
        &mut fresh(), Content::default(),
    );
    assert_eq!(out.text, "AAA:3/2\nXP9\nAAA:9/3\nXP9\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn temporary_scalars_execute_before_tooltip_in_one_session() {
    let fixture = fixture();
    let out = fixture.render(
        "set_temp_variable={n={value=3 add=1} tooltip=VARIABLE_TIP}
         add_to_temp_variable={n=2 tooltip=VARIABLE_TIP}
         subtract_from_temp_variable={n=1 tooltip=VARIABLE_TIP}
         multiply_temp_variable={n=2 tooltip=VARIABLE_TIP}
         divide_temp_variable={n=2 tooltip=VARIABLE_TIP}
         BBB={army_experience=n}",
        &mut fresh(), Content::default(),
    );
    assert_eq!(out.text, "AAA:4/4\nAAA:6/2\nAAA:5/1\nAAA:10/2\nAAA:5/2\n\u{12}BBBBeta XP5\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render("army_experience=temp_var:n", &mut fresh(), Content::default());
    assert_eq!(out.text, "XP0\n"); // Next outer description opens fresh TLS.
}

#[test]
fn effect_division_reads_if_zero_from_tls_even_for_nonzero_divisors() {
    let out = fixture().render(
        "set_temp_variable={n=8} set_temp_variable={fallback=3}
         divide_temp_variable={n=0 if_zero=fallback tooltip=VARIABLE_TIP}
         divide_temp_variable={n=0 if_zero=fallback if_zero=9 tooltip=VARIABLE_TIP}
         divide_temp_variable={n=0 if_zero=9 tooltip=VARIABLE_TIP}
         set_temp_variable={n=8} divide_temp_variable={n=2 if_zero=fallback}
         army_experience=n",
        &mut fresh(), Content::default(),
    );
    // Literal `if_zero=9` is an empty TLS key, not Evaluate's literal branch.
    assert_eq!(out.text, "AAA:3/0\nAAA:3/0\nAAA:0/0\nXP4\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture().render(
        "set_temp_variable={n=8} divide_temp_variable={n=2 if_zero=random_seed}",
        &mut fresh(), Content::default(),
    );
    assert!(out.refusals.keys().any(|why| why.contains("random_seed")), "{:?}", out.refusals);
}

#[test]
fn clamp_and_modulo_execute_but_empty_setters_do_not() {
    let fixture = fixture();
    let out = fixture.render(
        "set_temp_variable={n=7.9} clamp_temp_variable={var=n min=5 max=2}
         modulo_temp_variable={n=2} round_temp_variable=n randomize_temp_variable={var=n min=9 max=9}
         clear_variable=n clear_temp_array=n add_to_temp_array={array=n value=9}
         remove_from_temp_array={array=n value=1} resize_temp_array={array=n value=2}
         find_highest_in_array={array=n value=n} find_lowest_in_array={array=n value=n}
         army_experience=n",
        &mut fresh(), Content::default(),
    );
    assert_eq!(out.text, "XP1\n"); // max then min -> 5; modulo -> 1.
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render(
        "add_to_array={} remove_from_array={} resize_array={} clear_array={}
         clamp_variable={} round_variable=x randomize_variable={}
         get_sorted_scored_countries={} get_highest_scored_country={}
         modulo_variable={} set_variable_to_random={} army_experience=x",
        &mut fresh(), Content::default(),
    );
    assert_eq!(out.text, "XP3\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn global_tooltip_target_replaces_in_order_and_cleans_up() {
    let fixture = fixture();
    let mut graph = fresh();
    let out = fixture.render(
        "save_global_event_target_as=selected BBB={save_global_event_target_as=selected}
         event_target:selected={army_experience=2}",
        &mut graph, Content::default(),
    );
    assert_eq!(out.text, "\u{12}BBBBeta XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert!(graph.global_tooltip_targets.is_empty());
    let mut graph = EventScopes::new(vec![EventScope::of(EventObject::CountryState { country: 1, state: 2 })]);
    fixture.with_renderer(&mut graph, Content::default(), |renderer| {
        let evaluator = renderer.evaluator().unwrap();
        let scopes = evaluator.event_scopes(0).unwrap();
        evaluator.save_global_tooltip_target(b"selected", scopes).unwrap();
        assert_eq!(renderer.graph.borrow().global_tooltip_targets,
            vec![(b"selected".to_vec(), EventObject::State(2))]);
    });
}

#[test]
fn known_noops_do_not_need_rng_or_supply_totals() {
    let fixture = fixture();
    let out = fixture.render(
        "set_temp_variable={n=5} set_temp_variable_to_random={var=n min=3 max=2}
         get_supply_vehicles_temp={var=vehicles type=unknown}
         get_supply_vehicles_temp={var=vehicles type=\"truck\"}
         army_experience=n army_experience=vehicles",
        &mut fresh(), Content::default(),
    );
    assert_eq!(out.text, "XP5\nXP0\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    for (script, expected) in [
        ("set_temp_variable_to_random={var=n}", "CVariables+8"),
        ("get_supply_vehicles_temp={var=n type=truck need=yes}", "+0x130 i64"),
        ("get_supply_vehicles_temp={var=n type=truck}", "+0x138 i32"),
        ("get_supply_vehicles_temp={var=n type=train need=yes}", "+0x140 i32"),
        ("get_supply_vehicles={var=n}", "description is not ported"),
    ] {
        let out = fixture.render(script, &mut fresh(), Content::default());
        assert_eq!(out.refusals.values().sum::<usize>(), 1, "{script}: {:?}", out.refusals);
        assert!(out.refusals.keys().any(|why| why.contains(expected)), "{script}: {:?}", out.refusals);
    }
    let out = fixture.render("set_temp_variable_to_random={var=n} army_experience=n", &mut fresh(), Content::default());
    assert!(out.text.is_empty());
    assert!(out.refusals.keys().any(|why| why.contains("temp write is not reproduced")));
}
