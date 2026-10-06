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

#[test]
fn scripted_text_cached_state_diagnostics_are_filtered_and_reported_once() {
    use installed_content::states::{Diagnostic, MisreadCause};
    let (cat, _folder) = crate::scripted_text::tests::catalogue("", "");
    let warnings = vec![
        Diagnostic {
            kind: installed_content::DiagnosticKind::Warning,
            message: "native template skip".into(),
            cause: None,
        },
        Diagnostic {
            kind: installed_content::DiagnosticKind::Misread,
            message: "proven state recovery".into(),
            cause: Some(MisreadCause::InvalidImpassable),
        },
        Diagnostic {
            kind: installed_content::DiagnosticKind::Misread,
            message: "unproven state recovery".into(),
            cause: None,
        },
        Diagnostic {
            kind: installed_content::DiagnosticKind::Unsupported,
            message: "unsupported state content".into(),
            cause: None,
        },
    ];
    assert!(
        cat.state_content
            .set(Ok(StateContent {
                defines: installed_content::defines::read(&cat.content.layers).unwrap(),
                states: Default::default(),
                names: StateNames::new(),
                warnings: warnings.clone(),
            }))
            .is_ok()
    );
    let expected = ["unproven state recovery", "unsupported state content"];
    assert_eq!(cat.reader_warning_lines(), expected);
    assert_eq!(cat.refusal_lines(), expected);
    let content = cat.scripted_content().unwrap();
    assert!(std::ptr::eq(
        cat.cached_states().unwrap(),
        content.cached_states().unwrap(),
    ));
    assert_eq!(cat.refusal_lines(), expected);
    let mut inputs = EnvironmentInputs::synthetic();
    inputs.warnings = live_state_warnings(&warnings).map(str::to_owned).collect();
    assert!(
        content
            .environment()
            .unwrap()
            .inputs
            .set(Ok(inputs))
            .is_ok()
    );
    assert_eq!(cat.reader_warning_lines(), expected);
    assert_eq!(cat.refusal_lines(), expected);
    cat.refuse("unrelated text failure", "synthetic text");
    assert_eq!(cat.reader_warning_lines(), expected);
    assert_eq!(cat.refusal_lines().len(), expected.len() + 1);
}

#[test]
fn scripted_text_state_display_shares_precedence_language_and_cached_failures() {
    let (mut cat, _folder) =
        crate::scripted_text::tests::catalogue("", " MOD_STATE:0 \"Mod state\"\n");
    let content = cat.scripted_content().unwrap();
    assert!(content.state_content.get().is_none());
    let first_error = cat.cached_states().err().unwrap();
    assert!(content.state_content.get().is_some());
    assert_eq!(cat.cached_states().err().unwrap(), first_error);
    let names = BTreeMap::from([(64, Ok("MOD_STATE".into()))]);
    let mut state = replay::rows::StateRow {
        id: 64,
        ..Default::default()
    };
    assert_eq!(
        state_display_name(&state, &cat, Ok(&names), StateNameUsage::Place).unwrap(),
        "Mod state"
    );
    state.owner_name = b"Owner state".to_vec();
    assert_eq!(
        state_display_name(&state, &cat, Err("unread content"), StateNameUsage::Place).unwrap(),
        "Owner state"
    );
    state.custom_name = b"Custom state".to_vec();
    assert_eq!(
        state_display_name(&state, &cat, Ok(&names), StateNameUsage::Place).unwrap(),
        "Custom state"
    );
    cat.recorded_language = "other-language".into();
    assert!(
        state_display_name(&state, &cat, Ok(&names), StateNameUsage::Place)
            .unwrap_err()
            .contains("other-language")
    );
}
