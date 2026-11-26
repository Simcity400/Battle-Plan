use super::*;
use crate::catalogue::tests::script_game;
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
    script_game::enter();
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
            let parsed = content_reader_script::scope::parse(key, false, world.recorded());
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
                    &content_reader_script::scope::Scope::Keyword(content_reader_script::scope::Keyword::Root),
                    caller
                )
                .unwrap_err()
                .contains("outside the caller graph")
        );
    });
}

#[test]
fn graph_cursors_preserve_nested_from_and_restore_root_prev() {
    script_game::enter();
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
        let parsed = content_reader_script::scope::parse("FROM.FROM", false, world.recorded());
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
                &content_reader_script::scope::Scope::Keyword(content_reader_script::scope::Keyword::Root),
                caller,
            )
            .unwrap()
            .unwrap();
        assert_eq!(evaluator.scoped_country(b"FROM", root), Ok(Some(0)));
        let prev = evaluator
            .enter_scope(
                &content_reader_script::scope::Scope::Keyword(content_reader_script::scope::Keyword::Prev),
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
    script_game::enter();
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
                &content_reader_script::scope::Scope::Keyword(content_reader_script::scope::Keyword::Root),
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
                &content_reader_script::scope::Scope::Keyword(content_reader_script::scope::Keyword::Root),
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
    script_game::enter();
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
    script_game::enter();
    use replay_data::{rows::ProvinceRow, rows::StateRow};
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
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 100)
            .unwrap()
            .owned_states = Recorded::NotRecorded;
        rows.iter_mut()
            .find(|row| row.country == 103)
            .unwrap()
            .owned_states = Recorded::Value(Some(vec![7, 8]));
    }
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
    script_game::enter();
    use replay_data::{rows::DivisionTemplateRow, rows::Recorded};
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
            .contains("name key of division templates not recorded")
    );
}

#[test]
fn templates_compare_explicit_display_names_exactly() {
    script_game::enter();
    use replay_data::rows::DivisionTemplateRow;
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
    script_game::enter();
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

#[test]
fn ability_country_leader_copies_keep_both_masks_and_getter_priorities() {
    script_game::enter();
    use replay_data::{rows::CharacterRow, rows::Recorded, rows::UnitLeaderRow};
    let leader = 9_u64 << 32 | 3;
    let mut f = with_variables(
        frame(&original_countries(), &[103]),
        &[(1, 103, &[("x", ONE)]), (3, 44, &[("x", 9 * ONE)])],
    );
    *f.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 9,
            reference_type: 3,
            owner: 100,
            character_id: 44,
            character_type: engine_native::ccharacter::REFERENCE_TYPE,
            ..Default::default()
        }]));
    *f.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 44,
            country: 103,
            ..Default::default()
        }]));
    let TableView::Complete(TableRows::ScriptFlags(flags)) =
        &mut *f.tables[TableId::ScriptFlags.index()]
    else {
        panic!("flags");
    };
    flags.push(ScriptFlagRow {
        scope_kind: 3,
        scope_id: 44,
        token: 900,
        value: 1,
        ..Default::default()
    });
    let TableView::Complete(TableRows::ScriptVariables(stores)) =
        &mut *f.tables[TableId::ScriptVariables.index()]
    else {
        panic!("variables");
    };
    for row in stores {
        row.seed = Recorded::Value(Some(if row.scope_kind == 1 { 37 } else { 999 }));
    }
    with_world(&f, "", Err("none"), |world| {
        let object = EventObject::CountryUnitLeader { country: 4, leader };
        let graph = Rc::new(RefCell::new(EventScopes::new(vec![EventScope::of(object)])));
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph.clone());
        let caller = evaluator.event_scopes(0).unwrap();
        let combined = Scope::CountryLeader { country: 4, leader };
        assert_eq!(
            (caller.this, caller.root, caller.from, caller.prev),
            (combined, combined, Some(combined), Some(combined))
        );
        for prefix in ["", "ROOT", "FROM", "PREV"] {
            let body = "tag=GER has_country_flag=x has_character_flag=x check_variable={x=1}";
            let script = if prefix.is_empty() {
                body.to_owned()
            } else {
                format!("{prefix}={{{body}}}")
            };
            assert_eq!(
                evaluator.evaluate(&items(&script), caller),
                Eval::True,
                "{script}"
            );
        }
        let encoded = i64::from(u32::from_le_bytes(*b"SOV\0") | 0x4000_0000);
        assert_eq!(evaluator.scoped_value(b"THIS", caller), Ok(encoded));
        assert_eq!(evaluator.scoped_country(b"ROOT", caller), Ok(Some(4)));
        assert_eq!(
            evaluator.scoped_scope(b"THIS", caller),
            Ok(Some(Scope::Country(4)))
        );
        assert_eq!(
            evaluator.evaluate(&items("var:THIS={has_character_flag=x}"), caller),
            Eval::False
        );
        evaluator.begin_description_in(caller);
        assert_eq!(
            evaluator.scoped_value(b"random_seed", caller),
            Ok(37 + 4 * 31 * (13 + 17))
        );
        evaluator.end_description(caller).unwrap();
        evaluator.save_tooltip_target(b"kept", caller).unwrap();
        evaluator
            .save_global_tooltip_target(b"global", caller)
            .unwrap();
        assert_eq!(
            graph.borrow().tooltip_targets[&0][0].1,
            EventObject::Country(4)
        );
        assert_eq!(
            graph.borrow().global_tooltip_targets[0].1,
            EventObject::Country(4)
        );
        let entered = evaluator
            .enter_scope(
                &content_reader_script::scope::parse("SOV", false, world.recorded()),
                caller,
            )
            .unwrap()
            .unwrap();
        assert_eq!(entered.this, Scope::Country(4));
        assert!(entered.bare);
        assert_eq!(
            evaluator.evaluate(&items("has_character_flag=x"), entered),
            Eval::False
        );
        assert_eq!(
            evaluator.evaluate(&items("THIS={has_character_flag=x}"), entered),
            Eval::False
        );
        assert_eq!(
            evaluator.evaluate(&items("var:THIS={THIS={has_character_flag=x}}"), caller),
            Eval::False
        );
        for object in [
            EventObject::Country(4),
            EventObject::State(7),
            EventObject::Combat,
        ] {
            let cleared = {
                let mut graph = graph.borrow_mut();
                let at = graph.scopes.len();
                graph.scopes.push(EventScope {
                    prev: Link::Scope(0),
                    ..EventScope::of(object)
                });
                at
            };
            let cleared = evaluator.event_scopes(cleared).unwrap();
            assert!(cleared.bare);
            for prefix in ["THIS", "ROOT"] {
                assert_eq!(
                    evaluator.evaluate(
                        &items(&format!("{prefix}={{has_character_flag=x}}")),
                        cleared
                    ),
                    Eval::False,
                    "{object:?}.{prefix}"
                );
            }
        }
        assert_eq!(
            evaluator.evaluate(&items("has_country_flag=x"), Scopes::combat()),
            Eval::False
        );
        assert_eq!(
            evaluator.evaluate(&items("has_character_flag=x"), Scopes::combat()),
            Eval::False
        );
        assert_eq!(graph.borrow().scopes[0].this, object);
        assert_eq!(
            (
                graph.borrow().scopes[0].random_seed,
                graph.borrow().scopes[0].random_counter
            ),
            (1, 0x5ea6_ba9e)
        );
    });
    let mut incomplete = f.clone();
    *incomplete.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "country mapping unavailable".into(),
    };
    with_world(&incomplete, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let scopes = Scopes::of(Scope::CountryLeader { country: 4, leader });
        for script in [
            "always=yes",
            "has_character_flag=x",
            "hidden_trigger={has_character_flag=x}",
        ] {
            assert_eq!(
                evaluator.evaluate(&items(script), scopes),
                Eval::True,
                "{script}"
            );
        }
        assert!(
            evaluator
                .evaluate(&items("has_country_flag=x"), scopes)
                .decided()
                .is_err()
        );
    });
    let TableView::Complete(TableRows::UnitLeaders(leaders)) =
        &mut *f.tables[TableId::UnitLeaders.index()]
    else {
        panic!("leaders");
    };
    leaders.push(UnitLeaderRow {
        id: 10,
        reference_type: 3,
        owner: 103,
        character_id: 44,
        character_type: engine_native::ccharacter::REFERENCE_TYPE,
        ..Default::default()
    });
    with_world(&f, "", Err("none"), |world| {
        for this in [
            Scope::Leader(leader),
            Scope::CountryLeader { country: 4, leader },
        ] {
            let why = Evaluator::new(world, None)
                .evaluate(&items("tag=GER"), Scopes::of(this))
                .decided()
                .unwrap_err();
            assert!(why.contains("more than one unit leaders row"), "{why}");
        }
    });
}

#[test]
fn controlled_state_roster_keeps_stored_order_and_wrapping_count() {
    script_game::enter();
    use replay_data::{rows::ProvinceRow, rows::Recorded, rows::StateRow};
    let mut f = frame(&countries(), &[]);
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 7,
            owner: 100,
            controller: 103,
            ..Default::default()
        },
        StateRow {
            id: 8,
            owner: 100,
            controller: 103,
            ..Default::default()
        },
        StateRow {
            id: 9,
            owner: 100,
            controller: 103,
            ..Default::default()
        },
    ]));
    *f.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
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
    let TableView::Complete(TableRows::CountryCounterInputs(counters)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    else {
        unreachable!()
    };
    counters
        .iter_mut()
        .find(|row| row.country == 103)
        .unwrap()
        .controlled_states = Recorded::Value(Some(vec![8, 7]));
    counters
        .iter_mut()
        .find(|row| row.country == 100)
        .unwrap()
        .controlled_states = Recorded::Value(Some(vec![]));
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph());
        let caller = evaluator.event_scopes(0).unwrap();
        for (body, expected) in [
            ("state=8", true),
            ("state=9", false),
            ("count=2 always=yes", true),
            ("count=4294967296 always=no", true),
            ("count=-2147483648 always=yes", false),
            ("count=-2147483648 always=no", true),
        ] {
            assert_eq!(
                evaluator
                    .evaluate(
                        &items(&format!(
                            "FROM.FROM={{ any_controlled_state={{ {body} }} }}"
                        )),
                        caller
                    )
                    .decided(),
                Ok(expected)
            );
        }
        assert_eq!(
            evaluator.evaluate(&items("any_controlled_state={count=0 always=yes}"), caller),
            Eval::False
        );
        assert_eq!(
            evaluator.evaluate(
                &items("FROM.FROM={any_controlled_state={set_temp_variable={seen=THIS}}}"),
                caller
            ),
            Eval::True
        );
        assert_eq!(evaluator.scoped_state(b"seen", caller), Ok(8));
        assert!(evaluator.evaluate(&items("FROM.FROM={any_controlled_state={count=-2147483648 is_on_continent=mod_continent}}"), caller).decided().is_err());
    });
}

#[test]
fn scope_keys_precede_an_unavailable_scripted_catalogue() {
    script_game::enter();
    let f = frame(&countries(), &[]);
    let dlcs = BTreeSet::new();
    let names = TriggerNames::default();
    let aliases = Arc::new(TagAliases::default());
    let world = World::read(
        &f,
        ContentFacts {
            triggers: Err("scripted catalogue failed"),
            ..ContentFacts::content_only(&dlcs, &names, &aliases)
        },
    );
    let evaluator = Evaluator::new(&world, None).with_event_scopes(graph());
    let caller = evaluator.event_scopes(0).unwrap();
    for body in [
        "always=yes",
        "FROM={always=yes}",
        "FROM.FROM={always=yes}",
        "SOV={always=yes}",
    ] {
        assert_eq!(
            evaluator.evaluate(&items(body), caller),
            Eval::True,
            "{body}"
        );
    }
    assert!(
        evaluator
            .evaluate(&items("missing_script=yes"), caller)
            .decided()
            .unwrap_err()
            .contains("scripted catalogue failed")
    );
}

#[test]
fn state_roster_distinguishes_older_fields_from_failed_capture() {
    script_game::enter();
    use replay_data::{rows::ProvinceRow, rows::StateRow};
    let mut f = frame(&countries(), &[]);
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow {
        id: 7,
        owner: 100,
        controller: 103,
        ..Default::default()
    }]));
    *f.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
            id: 1,
            state: 7,
            ..Default::default()
        }]));
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 103)
            .unwrap()
            .controlled_states = Recorded::NotRecorded;
    }
    assert_eq!(
        check(&f, "", 3, "any_controlled_state={always=yes}"),
        Eval::True
    );
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    else {
        unreachable!()
    };
    rows.iter_mut()
        .find(|row| row.country == 103)
        .unwrap()
        .controlled_states = Recorded::Value(None);
    assert!(
        check(&f, "", 3, "any_controlled_state={always=yes}")
            .decided()
            .unwrap_err()
            .contains("controlled states of country counter inputs capture unavailable")
    );
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    else {
        unreachable!()
    };
    rows.iter_mut()
        .find(|row| row.country == 103)
        .unwrap()
        .controlled_states = Recorded::Value(Some(vec![7, 7]));
    assert_eq!(
        check(&f, "", 3, "any_controlled_state={count=2 always=yes}"),
        Eval::True
    );
    with_world(&f, "", Err("none"), |world| {
        let scopes = Scopes::of(Scope::CountryLeader {
            country: 3,
            leader: 0,
        });
        assert_eq!(
            world.evaluate(&items("any_controlled_state={count=2 always=yes}"), scopes),
            Eval::True
        );
    });
}

#[test]
fn controlled_state_rosters_admit_every_country_without_map_reads() {
    script_game::enter();
    use replay_data::rows::StateRow;
    let tags = (1..=24)
        .map(|id| format!("Synthetic{id}"))
        .collect::<Vec<_>>();
    let countries = tags
        .iter()
        .enumerate()
        .map(|(at, tag)| {
            let id = at as u32 + 1;
            (id, id, tag.as_str(), 1, "", "")
        })
        .collect::<Vec<_>>();
    let mut f = frame(&countries, &[]);
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (1..=24)
            .map(|id| StateRow {
                id,
                owner: id,
                controller: id,
                ..Default::default()
            })
            .collect(),
    ));
    *f.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "native roster body needs no province read".into(),
    };
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    else {
        unreachable!()
    };
    for row in rows {
        row.controlled_state_count = 1;
        row.controlled_states = Recorded::Value(Some(vec![row.country]));
    }
    with_world(&f, "", Err("none"), |world| {
        let body = items("any_controlled_state={always=yes}");
        for slot in 1..=24 {
            assert_eq!(
                world.evaluate(&body, Scopes::of(Scope::Country(slot))),
                Eval::True
            );
        }
    });
}
