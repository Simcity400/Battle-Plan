use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use crate::fixtures::game;

#[test]
fn random_headers_weights_modifiers_and_member_seed_reset() {
    game::enter();
    let fixture = Fixture::new();
    let out = fixture.render("fixture_chance_branch={chance=49.6 fixture_points=2} fixture_chance_branch={chance=0 fixture_points=3 add_political_power=4}", &mut fresh(), Content::default());
    assert_eq!(out.text, "50%: XP2\n0%: \n   XP3\n   PP4\n");
    let mut graph = fresh();
    let out = fixture.render("fixture_weighted_choices={seed=99 1={modifier={always=yes factor=2 add=1} fixture_pick_state={fixture_points=2}} 3={fixture_pick_state={fixture_points=2}} 0={}}", &mut graph, Content::default());
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert!(out.text.starts_with("3 options:\n50%: "), "{}", out.text);
    assert_eq!(out.text.matches("50%: ").count(), 2);
    assert!(out.text.ends_with("0%: No effect\n"), "{}", out.text);
    assert_eq!(graph.scopes[0].random_seed, 1);
    assert_eq!(
        graph.scopes[0].random_counter,
        fresh().scopes[0].random_counter
    );
}

#[test]
fn scripted_templates_ignore_call_boolean_and_refuse_recursive_cycles() {
    game::enter();
    let fixture = Fixture::new();
    let mut names = content_reader_script::key_names::KeyNames::default();
    names.scripted_effects = [
        b"outer".to_vec(),
        b"inner".to_vec(),
        b"cycle".to_vec(),
        b"guarded".to_vec(),
        b"scoped".to_vec(),
    ]
    .into();
    names.countries = [b"AAA".to_vec(), b"BBB".to_vec()].into();
    let mut database = content_reader_key_names::scripted_effects::ScriptedEffects::default();
    database.read_file(b"outer={if={limit={always=yes} inner=no}} inner={fixture_points=2} cycle={cycle=yes} guarded={if={limit={always=no} inner=yes}} scoped={fixture_points=2 if={limit={tag=AAA} BBB={scoped=yes}}}", &names);
    let content = Content {
        key_names: Some(&names),
        scripted_effects: Some(&database),
        ..Default::default()
    };
    let out = fixture.render("outer=no", &mut fresh(), content);
    assert_eq!(out.text, "XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render("guarded=yes", &mut fresh(), content);
    assert!(out.text.is_empty() && out.refusals.is_empty());
    let out = fixture.render("scoped=yes", &mut fresh(), content);
    assert_eq!(out.text.matches("XP2").count(), 2);
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render("cycle=yes", &mut fresh(), content);
    assert!(out.text.is_empty());
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("effect cycle:") && why.contains("recursively"))
    );
}

#[test]
fn meta_uses_native_reader_and_case_sensitive_parameter_substitution() {
    game::enter();
    let mut fixture = Fixture::new();
    *fixture.frame.tables[replay_data::rows::TableId::ScriptVariables.index()] =
        replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::ScriptVariables(
            vec![replay_data::rows::ScriptVariablesRow {
                seed: replay_data::rows::Recorded::Value(None),
                scope_kind: 1,
                scope_id: 100,
                entries: Vec::new(),
            }],
        ));
    let names = content_reader_script::key_names::KeyNames::default();
    let out = fixture.render_native(
        "fixture_expand_text={X=2.60000 text={fixture_points=[X] add_political_power=1}}",
        &mut fresh(),
        Content {
            key_names: Some(&names),
            ..Default::default()
        },
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(out.text, "XP2\nPP1\n");
    let out = fixture.render_native(
        "fixture_expand_text={X=2 text={fixture_points=[x]}}",
        &mut fresh(),
        Content {
            key_names: Some(&names),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "XP0\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render_native(
        "fixture_expand_text={}",
        &mut fresh(),
        Content {
            key_names: Some(&names),
            ..Default::default()
        },
    );
    assert!(out.text.is_empty() && out.refusals.is_empty());
}

#[test]
fn array_loop_without_tooltip_does_not_read_and_override_keeps_children() {
    game::enter();
    let fixture = Fixture::new();
    let out = fixture.render("fixture_loop_scopes={array=unread break=yes fixture_points=9} fixture_override_text={tooltip=TIP fixture_points=2} fixture_override_text={tooltip=\"\" fixture_points=3}", &mut fresh(), Content::default());
    assert_eq!(out.text, "tip\nXP2\nXP3\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn array_selectors_decode_in_order_and_loop_keeps_links_with_default_seed() {
    game::enter();
    use crate::scripted_text::{EventObject, EventScope, Link};
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            seed: replay_data::rows::Recorded::Value(None),
            scope_kind: 1,
            scope_id: 100,
            entries: vec![
                ScriptVariableEntry {
                    name: b"roster^num".to_vec(),
                    value_raw: 200_000,
                },
                ScriptVariableEntry {
                    name: b"roster^0".to_vec(),
                    value_raw: 0,
                },
                ScriptVariableEntry {
                    name: b"roster^1".to_vec(),
                    value_raw: 500_000,
                },
            ],
        }]));
    let mut graph = fresh();
    graph.scopes[0].random_seed = 77;
    graph.scopes[0].from = Link::Scope(1);
    graph.scopes.push(EventScope::of(EventObject::Country(2)));
    let out = fixture.render("fixture_loop_scopes={array=roster tooltip=TIP if={limit={ROOT={tag=AAA} FROM={tag=BBB} PREV={tag=AAA}} fixture_points=2}}", &mut graph, Content::default());
    assert_eq!(out.text, "tip\n   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(graph.scopes[2].this, EventObject::State(5));
    assert_eq!(
        (
            graph.scopes[2].root,
            graph.scopes[2].from,
            graph.scopes[2].prev
        ),
        (Link::Scope(0), Link::Scope(1), Link::Scope(0))
    );
    assert_eq!(graph.scopes[2].random_seed, 1);
    let mut graph = fresh();
    let out = fixture.render(
        "fixture_array_member={array=roster fixture_points=2}",
        &mut graph,
        Content::default(),
    );
    assert_eq!(out.text, "\u{11}HFive:\n\u{11}!   XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(graph.scopes[0].random_seed, 2);
}

#[test]
fn random_list_maximum_weight_uses_native_invalid_chance_sentinel() {
    game::enter();
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            seed: replay_data::rows::Recorded::Value(None),
            scope_kind: 1,
            scope_id: 100,
            entries: vec![
                ScriptVariableEntry {
                    name: b"high".to_vec(),
                    value_raw: i64::MAX,
                },
                ScriptVariableEntry {
                    name: b"low".to_vec(),
                    value_raw: 1 - i64::MAX,
                },
            ],
        }]));
    let out = fixture.render(
        "fixture_weighted_choices={high={fixture_points=2} low={}}",
        &mut fresh(),
        Content::default(),
    );
    let sentinel = fixture
        .catalogue
        .localisation
        .localise_with(
            b"RANDOM_EFFECT",
            &[("CHANCE", Argument::Fixed5(0xffff_ffff))],
        )
        .unwrap();
    assert!(
        out.text
            .starts_with(&format!("2 options:\n{sentinel}XP2\n")),
        "{}",
        out.text
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn scripted_depth_is_bounded_when_this_keeps_changing() {
    game::enter();
    use crate::scripted_text::{EventObject, EventScope};
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (1..=65)
            .map(|id| StateRow {
                id,
                custom_name: id.to_string().into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    fixture.neighbors = Some((1..65).map(|id| (id, vec![id + 1])).collect());
    let mut names = content_reader_script::key_names::KeyNames::default();
    names.scripted_effects.insert(b"walk".to_vec());
    let mut database = content_reader_key_names::scripted_effects::ScriptedEffects::default();
    database.read_file(b"walk={fixture_list_neighbor_state={walk=yes}}", &names);
    let mut graph = EventScopes::new(vec![EventScope::of(EventObject::State(1))]);
    let out = fixture.render(
        "walk=yes",
        &mut graph,
        Content {
            key_names: Some(&names),
            scripted_effects: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(
        out.refusals.values().sum::<usize>(),
        1,
        "{:?}",
        out.refusals
    );
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("effect walk:") && why.contains("depth"))
    );
}

#[test]
fn own_like_scripted_keys_are_not_removed_from_unrelated_bodies() {
    game::enter();
    let fixture = Fixture::new();
    let mut names = content_reader_script::key_names::KeyNames::default();
    names.scripted_effects = [
        b"array".to_vec(),
        b"original_tag".to_vec(),
        b"prioritize".to_vec(),
    ]
    .into();
    let mut database = content_reader_key_names::scripted_effects::ScriptedEffects::default();
    database.read_file(b"array={fixture_points=2} original_tag={fixture_points=3} prioritize={fixture_points=4}", &names);
    let out = fixture.render(
        "if={limit={always=yes} array=yes} effect_tooltip={original_tag=yes} prioritize=yes",
        &mut fresh(),
        Content {
            key_names: Some(&names),
            scripted_effects: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "XP2\nXP3\nXP4\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn random_array_probe_recurses_through_its_children() {
    game::enter();
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            seed: replay_data::rows::Recorded::Value(None),
            scope_kind: 1,
            scope_id: 200,
            entries: vec![
                ScriptVariableEntry {
                    name: b"roster^num".to_vec(),
                    value_raw: 100_000,
                },
                ScriptVariableEntry {
                    name: b"roster^0".to_vec(),
                    value_raw: 100_000,
                },
            ],
        }]));
    let out = fixture.render(
        "BBB={fixture_array_member={array=roster fixture_points=2 add_political_power=3}}",
        &mut fresh(),
        Content::default(),
    );
    assert!(out.text.starts_with("\u{12}BBBBeta:\n"), "{}", out.text);
    assert!(out.text.contains("XP2") && out.text.contains("PP3"));
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn empty_overrides_keep_inherited_multiple_probe() {
    game::enter();
    let fixture = Fixture::new();
    fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
        let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
        for name in ["fixture_ordered_loop", "fixture_counted_loop", "fixture_conditional_loop"] {
            let item = content_reader_parser::parser::parse(
                format!("{name}={{fixture_points=2 add_political_power=3}}").as_bytes(),
            )
            .unwrap()
            .items
            .remove(0);
            assert!(renderer.empty_build_override(&item));
            assert!(renderer.multiple_child(&item, &[], scopes), "{name}");
        }
        assert!(renderer.out.refusals.is_empty());
    });
}
