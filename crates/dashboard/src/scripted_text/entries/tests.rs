use super::*;

#[test]
fn description_append_preserves_active_cursor_graph_and_guard_state() {
    let mut graph = EventScopes::new(vec![
        EventScope {
            root: Link::Scope(1),
            from: Link::Scope(2),
            ..EventScope::of(EventObject::Country(7))
        },
        EventScope::of(EventObject::State(11)),
        EventScope::of(EventObject::Country(9)),
    ]);
    graph.variables.target_state = Some(11);
    graph
        .tooltip_targets
        .insert(1, vec![(b"parent".to_vec(), EventObject::State(11))]);
    let mut expanded = ExpansionScopes::new(&graph);
    expanded.on_large_stack = true;
    expanded.fresh_depth.set(9);
    expanded.active.borrow_mut().push(ActiveEntry {
        name: "synthetic".into(),
        at: 0,
        scopes_len: 3,
        bare: false,
        variables: graph.variables,
    });
    {
        let appended = expanded.append_description(&graph);
        assert_eq!(appended.offset, 3);
        let active = expanded.active.borrow();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].at, 0);
        assert_eq!(active[0].scopes_len, 3);
        assert!(active[0].matches("synthetic", &expanded, appended.offset, false));
        assert_eq!(expanded.scope(3).root, Link::Scope(4));
        assert_eq!(expanded.scope(3).from, Link::Scope(5));
        assert_eq!(expanded.fresh_depth.get(), 9);
        assert!(expanded.on_large_stack);
        assert_eq!(
            expanded.targets.borrow().tooltip_targets[&4],
            graph.tooltip_targets[&1]
        );
    }
    assert_eq!(*expanded.scopes.borrow(), graph.scopes);
    assert_eq!(expanded.prev_distance.borrow().len(), 3);
    assert_eq!(
        expanded.targets.borrow().tooltip_targets,
        graph.tooltip_targets
    );
    assert_eq!(expanded.variables.get(), graph.variables);
    assert_eq!(expanded.active.borrow()[0].at, 0);
    graph.scopes[0].prev = Link::Scope(99);
    {
        let _appended = expanded.append_description(&graph);
        assert!(!expanded.valid_links.get());
    }
    assert!(expanded.valid_links.get());
}
