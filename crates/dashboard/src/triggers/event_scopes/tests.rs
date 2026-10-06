use super::*;
use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};
use std::{cell::RefCell, rc::Rc};

fn graph() -> Rc<RefCell<EventScopes>> {
    Rc::new(RefCell::new(EventScopes::new(vec![
        EventScope {
            root: Link::Scope(1),
            from: Link::Scope(2),
            ..EventScope::of(EventObject::Country(0))
        },
        EventScope::of(EventObject::Country(0)),
        EventScope {
            from: Link::Scope(3),
            ..EventScope::of(EventObject::State(7))
        },
        EventScope {
            from: Link::Scope(4),
            ..EventScope::of(EventObject::Country(3))
        },
        EventScope::of(EventObject::State(8)),
    ])))
}

#[test]
fn cursored_scope_without_a_bound_graph_refuses_instead_of_panicking() {
    let f = frame(&countries(), &[]);
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let caller = Scopes {
            cursor: Some(0),
            ..Scopes::of(Scope::Country(0))
        };
        for key in [
            "ROOT",
            "FROM",
            "PREV",
            "ROOT.FROM",
            "event_target:missing.FROM",
            "SOV",
        ] {
            let parsed = installed_content::scope::parse(key, false, world.recorded());
            assert!(
                evaluator
                    .enter_scope(&parsed, caller)
                    .unwrap_err()
                    .contains("scope graph is missing"),
                "{key}"
            );
        }
        let evaluator = evaluator.with_event_scopes(graph());
        let caller = Scopes {
            cursor: Some(99),
            ..caller
        };
        assert!(
            evaluator
                .enter_scope(
                    &installed_content::scope::Scope::Keyword(installed_content::scope::Keyword::Root),
                    caller
                )
                .unwrap_err()
                .contains("outside the caller graph")
        );
    });
}

#[test]
fn graph_cursors_preserve_nested_from_and_restore_root_prev() {
    let f = frame(&countries(), &[]);
    with_world(&f, "", Err("none"), |world| {
        let graph = graph();
        graph.borrow_mut().scopes[3].random_seed = 27;
        graph.borrow_mut().scopes[3].random_counter = 91;
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph.clone());
        let caller = evaluator.event_scopes(0).unwrap();
        assert_eq!(evaluator.scoped_state_id(b"999", caller), Ok(999));
        assert_eq!(evaluator.scoped_state_id(b"-1", caller), Ok(-1));
        assert_eq!(evaluator.scoped_country(b"FROM.FROM", caller), Ok(Some(3)));
        assert_eq!(
            evaluator.scoped_scope(b"FROM.FROM.FROM", caller),
            Ok(Some(Scope::State(8)))
        );
        assert_eq!(
            evaluator.evaluate(&items("FROM.FROM = { tag = SOV }"), caller),
            Eval::True
        );
        let parsed = installed_content::scope::parse("FROM.FROM", false, world.recorded());
        let entered = evaluator.enter_scope(&parsed, caller).unwrap().unwrap();
        assert_eq!(
            (entered.this, entered.root, entered.prev),
            (
                Scope::Country(3),
                Scope::Country(0),
                Some(Scope::Country(0))
            )
        );
        let root = evaluator
            .enter_scope(
                &installed_content::scope::Scope::Keyword(installed_content::scope::Keyword::Root),
                caller,
            )
            .unwrap()
            .unwrap();
        assert_eq!(evaluator.scoped_country(b"FROM", root), Ok(Some(0)));
        let prev = evaluator
            .enter_scope(
                &installed_content::scope::Scope::Keyword(installed_content::scope::Keyword::Prev),
                entered,
            )
            .unwrap()
            .unwrap();
        assert_eq!(evaluator.scoped_country(b"FROM", prev), Ok(Some(0)));
        let copied = graph.borrow().scopes[entered.cursor.unwrap()];
        assert_eq!((copied.random_seed, copied.random_counter), (27, 91));
    });
}

#[test]
fn description_cleanup_keeps_distinct_root_storage_and_predicate_saved_only() {
    let f = frame(&countries(), &[]);
    with_world(&f, "", Err("none"), |world| {
        let graph = graph();
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph.clone());
        let caller = evaluator.event_scopes(0).unwrap();
        evaluator.begin_description();
        evaluator.save_tooltip_target(b"our", caller).unwrap();
        assert_eq!(
            evaluator.scoped_country(b"event_target:our", caller),
            Ok(None)
        );
        assert_eq!(
            evaluator.evaluate(&items("has_event_target = our"), caller),
            Eval::False
        );
        let root = evaluator
            .enter_scope(
                &installed_content::scope::Scope::Keyword(installed_content::scope::Keyword::Root),
                caller,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            evaluator.scoped_country(b"event_target:our", root),
            Ok(Some(0))
        );
        evaluator.end_description(caller).unwrap();
        assert!(graph.borrow().tooltip_targets.contains_key(&1));
        evaluator.begin_description();
        let root = evaluator
            .enter_scope(
                &installed_content::scope::Scope::Keyword(installed_content::scope::Keyword::Root),
                caller,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            evaluator.scoped_country(b"event_target:our", root),
            Ok(Some(0))
        );
        evaluator.end_description(caller).unwrap();
    });
    with_world(&f, "", Err("none"), |world| {
        let graph = Rc::new(RefCell::new(EventScopes::new(vec![EventScope::of(
            EventObject::Country(0),
        )])));
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph.clone());
        let caller = evaluator.event_scopes(0).unwrap();
        evaluator.begin_description();
        evaluator.save_tooltip_target(b"our", caller).unwrap();
        assert_eq!(
            evaluator.scoped_country(b"event_target:our", caller),
            Ok(Some(0))
        );
        evaluator.end_description(caller).unwrap();
        assert!(!graph.borrow().tooltip_targets.contains_key(&0));
        assert_eq!(
            evaluator.scoped_country(b"event_target:our", caller),
            Ok(None)
        );
    });
}

#[test]
fn nested_description_checks_share_temps_and_outer_sessions_reset() {
    let f = frame(&countries(), &[]);
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let caller = Scopes::of(Scope::Country(0));
        evaluator.begin_description();
        assert_eq!(
            evaluator.evaluate(&items("set_temp_variable = { x = 7 }"), caller),
            Eval::True
        );
        assert_eq!(
            evaluator.evaluate(&items("check_variable = { x = 7 }"), caller),
            Eval::True
        );
        assert_eq!(evaluator.scoped_value(b"temp_var:x", caller), Ok(700_000));
        evaluator.end_description(caller).unwrap();
        evaluator.begin_description();
        assert_eq!(evaluator.scoped_value(b"temp_var:x", caller), Ok(0));
        evaluator.end_description(caller).unwrap();
    });
}

#[test]
fn owned_state_builtin_and_scoped_predicate_use_recorded_operands() {
    use replay::rows::{ProvinceRow, StateRow};
    let mut f = frame(&countries(), &[]);
    *f.tables[TableId::States.index()] =
        TableView::Complete(TableRows::from_rows::<tables::States>(vec![
            StateRow {
                id: 7,
                owner: 103,
                ..Default::default()
            },
            StateRow {
                id: 8,
                owner: 103,
                ..Default::default()
            },
            StateRow {
                id: 9,
                owner: 103,
                ..Default::default()
            },
        ]));
    *f.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::from_rows::<tables::Provinces>(vec![
            ProvinceRow {
                id: 1,
                state: 7,
                ..Default::default()
            },
            ProvinceRow {
                id: 2,
                state: 8,
                ..Default::default()
            },
        ]));
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph());
        let caller = evaluator.event_scopes(0).unwrap();
        assert_eq!(
            evaluator.evaluate(
                &items("check_variable = { FROM.FROM.num_owned_states > 1 }"),
                caller
            ),
            Eval::True
        );
        assert_eq!(evaluator.evaluate(&items("FROM.FROM = { any_owned_state = { NOT = { state = FROM.FROM.FROM } impassable = no } }"), caller), Eval::True);
        assert_eq!(
            evaluator.evaluate(
                &items("FROM.FROM = { any_owned_state = { state = 9 } }"),
                caller
            ),
            Eval::False
        );
        let refused = evaluator
            .evaluate(
                &items("any_owned_state = { set_temp_variable = { x = 1 } }"),
                caller,
            )
            .decided()
            .unwrap_err();
        assert!(refused.contains("roster order"), "{refused}");
    });
}

#[test]
fn compatibility_template_fallback_key_is_unknown_but_explicit_name_remains_usable() {
    use replay::rows::{DivisionTemplateRow, Recorded};
    let mut f = frame(&countries(), &[]);
    *f.tables[TableId::DivisionTemplates.index()] =
        TableView::Complete(TableRows::DivisionTemplates(vec![DivisionTemplateRow {
            template: 1,
            owner: 100,
            name: b"Intervention Force".to_vec(),
            name_key: Recorded::NotRecorded,
            ..Default::default()
        }]));
    assert_eq!(
        check(&f, "", 0, "has_template = \"Intervention Force\""),
        Eval::True
    );
    if let TableView::Complete(TableRows::DivisionTemplates(rows)) =
        &mut *f.tables[TableId::DivisionTemplates.index()]
    {
        rows[0].name.clear();
    }
    let result = check(&f, "", 0, "has_template = \"Intervention Force\"");
    assert!(
        result
            .decided()
            .unwrap_err()
            .contains("DivisionTemplates.name_key not recorded")
    );
}

#[test]
fn templates_compare_explicit_display_names_exactly() {
    use replay::rows::DivisionTemplateRow;
    let mut f = frame(&countries(), &[]);
    *f.tables[TableId::DivisionTemplates.index()] =
        TableView::Complete(TableRows::from_rows::<tables::DivisionTemplates>(vec![
            DivisionTemplateRow {
                template: 1,
                owner: 100,
                name: b"Intervention Force".to_vec(),
                ..Default::default()
            },
        ]));
    assert_eq!(
        check(&f, "", 0, "has_template = \"Intervention Force\""),
        Eval::True
    );
    assert_eq!(
        check(&f, "", 0, "has_template = \"intervention force\""),
        Eval::False
    );
    *f.tables[TableId::DivisionTemplates.index()] = TableView::Unavailable {
        reason: "not read".into(),
    };
    assert!(
        check(&f, "", 0, "has_template = \"Intervention Force\"")
            .decided()
            .is_err()
    );
}

#[test]
fn broken_graph_and_storage_cycles_refuse_only_reached_links() {
    let f = frame(&countries(), &[]);
    with_world(&f, "", Err("none"), |world| {
        let graph = graph();
        graph.borrow_mut().scopes[0].from = Link::Unknown;
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph.clone());
        let caller = evaluator.event_scopes(0).unwrap();
        assert_eq!(
            evaluator.evaluate(&items("always = yes"), caller),
            Eval::True
        );
        assert!(evaluator.scoped_value(b"FROM.FROM.x", caller).is_err());
        graph.borrow_mut().scopes[0].root = Link::Unknown;
        let caller = evaluator.event_scopes(0).unwrap();
        let entered = evaluator.with_this(caller, Scope::Country(3)).unwrap();
        assert_eq!(evaluator.evaluate(&items("tag = SOV"), entered), Eval::True);
        assert!(evaluator.scoped_country(b"ROOT", entered).is_err());
        graph.borrow_mut().scopes[0].root = Link::Scope(1);
        graph.borrow_mut().scopes[1].root = Link::Scope(0);
        assert!(
            evaluator
                .save_tooltip_target(b"x", caller)
                .unwrap_err()
                .contains("101-link")
        );
    });
}
