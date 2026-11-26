use super::*;
use crate::script_test_game as game;
use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};
use std::{cell::RefCell, rc::Rc};

fn collection(text: &str) -> content_reader_politics::collections::Collection {
    let input = content_reader_parser::parser::parse(format!("input={{{text}}}").as_bytes())
        .unwrap()
        .items;
    content_reader_politics::collections::compile(&input[0], None).unwrap()
}

#[test]
fn collection_selection_uses_order_limits_and_fresh_root_environment() {
    game::enter();
    let mut f = frame(
        &[(100, 1, "AAA", 2, "", ""), (200, 2, "GER", 1, "", "")],
        &[],
    );
    *f.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 100,
                faction: 8,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 200,
                faction: 8,
                ordinal: 0,
            },
        ]));
    *f.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "subjects unread".into(),
    };
    with_world(&f, "", Err("none"), |world| {
        let graph = Rc::new(RefCell::new(EventScopes::new(vec![
            EventScope {
                root: Link::Scope(1),
                from: Link::Scope(2),
                random_seed: 77,
                ..EventScope::of(EventObject::Country(1))
            },
            EventScope {
                from: Link::Scope(2),
                ..EventScope::of(EventObject::Country(2))
            },
            EventScope::of(EventObject::State(9)),
        ])));
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph.clone());
        let caller = evaluator.event_scopes(0).unwrap();
        let spec = collection("input=game:scope operators={faction_members limit={tag=GER}} ");
        let selected = evaluator
            .collection_representative(&spec, caller)
            .unwrap()
            .unwrap();
        assert_eq!(
            (selected.this, selected.root, selected.from, selected.prev),
            (
                Scope::Country(2),
                Scope::Country(2),
                Some(Scope::State(9)),
                Some(Scope::Country(2))
            )
        );
        let selected = graph.borrow().scopes[selected.cursor.unwrap()];
        assert_eq!(
            (selected.random_seed, selected.random_counter),
            (1, 0x5ea6_ba9e)
        );
        let empty = collection("input=game:scope operators={faction_members limit={always=no}} ");
        assert!(
            evaluator
                .collection_representative(&empty, caller)
                .unwrap()
                .is_none()
        );
        let ordered = collection("input=game:scope operators={faction_members}");
        assert_eq!(
            evaluator
                .collection_representative(&ordered, caller)
                .unwrap()
                .unwrap()
                .this,
            Scope::Country(2)
        );
        let own_first = collection("input=game:scope operators={country_and_all_subjects}");
        assert_eq!(
            evaluator
                .collection_representative(&own_first, caller)
                .unwrap()
                .unwrap()
                .this,
            Scope::Country(1)
        );
    });
}

#[test]
fn unrecorded_subjects_refuse_country_and_all_subjects() {
    game::enter();
    let mut f = frame(
        &[(100, 1, "AAA", 2, "", ""), (200, 2, "GER", 1, "", "")],
        &[],
    );
    let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *f.tables[TableId::CountryAppearance.index()]
    else {
        panic!("the fixture records country appearance");
    };
    rows[0].subjects = replay_data::rows::Recorded::NotRecorded;
    with_world(&f, "", Err("none"), |world| {
        let graph = Rc::new(RefCell::new(EventScopes::new(vec![EventScope::of(
            EventObject::Country(1),
        )])));
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph);
        let caller = evaluator.event_scopes(0).unwrap();
        let spec =
            collection("input=game:scope operators={country_and_all_subjects limit={tag=GER}}");
        let Err(error) = evaluator.collection_representative(&spec, caller) else {
            panic!("unrecorded subjects refuse");
        };
        assert!(
            error.contains("subjects of country appearance not recorded"),
            "{error}"
        );
    });
}

#[test]
fn format98_collections_select_first_passing_owned_and_controlled_state() {
    game::enter();
    use replay_data::rows::StateRow;
    let mut f = frame(&[(100, 1, "AAA", 2, "", "")], &[]);
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 7,
            owner: 100,
            controller: 100,
            ..Default::default()
        },
        StateRow {
            id: 8,
            owner: 100,
            controller: 100,
            ..Default::default()
        },
    ]));
    *f.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        [7, 8]
            .map(|id| replay_data::rows::ProvinceRow {
                id,
                state: id,
                ..Default::default()
            })
            .to_vec(),
    ));
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    else {
        panic!("complete counters");
    };
    rows[0].owned_states = replay_data::rows::Recorded::Value(Some(vec![8, 7]));
    rows[0].controlled_states = replay_data::rows::Recorded::Value(Some(vec![7, 8]));
    with_world(&f, "", Err("none"), |world| {
        let graph = Rc::new(RefCell::new(EventScopes::new(vec![EventScope::of(
            EventObject::Country(1),
        )])));
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph);
        let caller = evaluator.event_scopes(0).unwrap();
        for (limit, state) in [("always=no", None), ("state=8", Some(Scope::State(8)))] {
            let spec = collection(&format!(
                "input=game:scope operators={{owned_states limit={{{limit}}}}}"
            ));
            assert_eq!(
                evaluator
                    .collection_representative(&spec, caller)
                    .unwrap()
                    .map(|scopes| scopes.this),
                state
            );
        }
        for (operator, expected) in [
            ("owned_states", Scope::State(8)),
            ("controlled_states", Scope::State(7)),
        ] {
            let spec = collection(&format!(
                "input=game:scope operators={{{operator} limit={{always=yes}}}}"
            ));
            assert_eq!(
                evaluator
                    .collection_representative(&spec, caller)
                    .unwrap()
                    .unwrap()
                    .this,
                expected
            );
        }
    });
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    else {
        panic!("complete counters");
    };
    rows[0].owned_states = replay_data::rows::Recorded::NotRecorded;
    with_world(&f, "", Err("none"), |world| {
        let graph = Rc::new(RefCell::new(EventScopes::new(vec![EventScope::of(
            EventObject::Country(1),
        )])));
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph);
        let caller = evaluator.event_scopes(0).unwrap();
        let spec = collection("input=game:scope operators={owned_states}");
        assert!(
            evaluator
                .collection_representative(&spec, caller)
                .unwrap_err()
                .contains("owned states of country counter inputs")
        );
    });
}

#[test]
fn collection_constant_arrays_keep_content_order_and_default_seeds() {
    game::enter();
    use replay_data::rows::StateRow;
    let mut f = frame(
        &[(100, 1, "AAA", 2, "", ""), (200, 2, "GER", 1, "", "")],
        &[],
    );
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 7,
            owner: 100,
            controller: 100,
            ..Default::default()
        },
        StateRow {
            id: 8,
            owner: 100,
            controller: 100,
            ..Default::default()
        },
    ]));
    let constants = content_reader_politics::script_constants::parse(&[("synthetic.txt", b"states={schema={any_key=yes array=state} group={8 7} invalid={0}} tags={schema={any_key=yes array=country} group={GER AAA}}")]);
    let none = BTreeSet::new();
    let names = TriggerNames::default();
    let aliases = Arc::default();
    let mut facts = ContentFacts::content_only(&none, &names, &aliases);
    facts.script_constants = Ok(&constants);
    let world = World::read(&f, facts);
    let graph = Rc::new(RefCell::new(EventScopes::new(vec![EventScope {
        random_seed: 77,
        ..EventScope::of(EventObject::Country(1))
    }])));
    let evaluator = Evaluator::new(&world, None).with_event_scopes(graph.clone());
    let caller = evaluator.event_scopes(0).unwrap();
    for (path, expected) in [
        ("states.group", Scope::State(8)),
        ("tags.group", Scope::Country(2)),
    ] {
        let spec = collection(&format!("input=constant:{path}"));
        let selected = evaluator
            .collection_representative(&spec, caller)
            .unwrap()
            .unwrap();
        assert_eq!(selected.this, expected);
        assert_eq!(
            graph.borrow().scopes[selected.cursor.unwrap()].random_seed,
            1
        );
    }
    let invalid = collection("input=constant:states.invalid");
    assert_eq!(
        evaluator
            .collection_representative(&invalid, caller)
            .unwrap_err(),
        "collection constant state 0 resolves to a null state pointer"
    );
    assert_eq!(graph.borrow().scopes[0].random_seed, 77);
}
