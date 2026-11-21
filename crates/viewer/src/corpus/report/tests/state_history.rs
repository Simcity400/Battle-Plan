use super::*;

fn states_run(completed: bool) -> ReaderRun {
    ReaderRun {
        reader: "states".into(),
        milliseconds: 0,
        completed,
        not_run: None,
        note: None,
    }
}

#[test]
fn state_history_lists_affected_states_and_all_returned_reader_evidence() {
    let vanilla = StackReport {
        stack: "vanilla".into(),
        name: "Vanilla and DLC".into(),
        readers: vec![states_run(true)],
        ..Default::default()
    };
    let mut fixture = StackReport {
        stack: "fixture".into(),
        name: "Synthetic mod".into(),
        readers: vec![states_run(true)],
        state_history_states: vec![
            StateHistoryState {
                id: 7,
                file: Some("history/states/test.txt".into()),
                reasons: vec![
                    "Producer reason | repeated (count: 3)".into(),
                    "Second reason".into(),
                ],
            },
            StateHistoryState {
                id: 99,
                file: Some("history/states/other.txt".into()),
                reasons: vec!["File-wide variable diagnostic".into()],
            },
        ],
        ..Default::default()
    };
    for _ in 0..2 {
        let diagnostic = Diagnostic::new(
            "states",
            Kind::Unsupported,
            "history/states/test.txt: line 3: Producer diagnostic | repeated (count: 3)".into(),
        );
        fixture.state_history.push((&diagnostic).into());
    }
    fixture.state_history.push(
        (&Diagnostic::new(
            "states",
            Kind::Misread,
            "history/states/test.txt: line 4: Arbitrary warning wording remains misread".into(),
        ))
            .into(),
    );
    fixture.state_history.push(
        (&Diagnostic::new("states", Kind::Warning,
            "state 2 is defined by both history/states/kept.txt and history/states/discarded.txt; the game keeps the first".into()))
            .into(),
    );
    fixture.diagnostics.push(Diagnostic::new(
        "states",
        Kind::Warning,
        "history block 1939.1.1 is skipped".into(),
    ));
    let missing = StackReport {
        stack: "missing".into(),
        readers: vec![ReaderRun {
            not_run: Some("needs map".into()),
            ..states_run(false)
        }],
        ..Default::default()
    };
    let failed = StackReport {
        stack: "failed".into(),
        readers: vec![states_run(false)],
        state_history: vec![
            (&Diagnostic::new("states", Kind::ReaderError, "I/O unavailable".into())).into(),
        ],
        ..Default::default()
    };
    let run = [vanilla, fixture, missing, failed]
        .into_iter()
        .map(|report| (report.stack.clone(), report))
        .collect();
    let text = state_history(&run);
    assert!(
        text.contains(
            "### vanilla (Vanilla and DLC)\n\nAffected states: 0.\n\nNo states reader diagnostics. Refused: no."
        ),
        "{text}"
    );
    assert!(
        text.contains("Notes per kind: misread: 1, unsupported: 2, warning: 1. Refused: no."),
        "{text}"
    );
    assert_eq!(
        text.matches("| fixture | Synthetic mod | history/states/test.txt | 3 | unsupported |")
            .count(),
        2
    );
    assert!(text.contains("Producer diagnostic \\| repeated"), "{text}");
    assert!(text.contains("Affected states: 2."), "{text}");
    assert!(text.contains("| fixture | Synthetic mod | 7 | history/states/test.txt | Producer reason \\| repeated (count: 3); Second reason |"), "{text}");
    assert!(text.contains("| fixture | Synthetic mod | 99 | history/states/other.txt | File-wide variable diagnostic |"), "{text}");
    assert!(text.contains("not attributable to a kept state"), "{text}");
    assert!(text.contains("state 2 is defined by both"), "{text}");
    assert!(text.contains("did not run (needs map)"), "{text}");
    assert!(
        text.contains("| failed |  | unknown | unknown | error | I/O unavailable |"),
        "{text}"
    );
    assert!(
        text.contains("Notes per kind: error: 1. Refused: yes."),
        "{text}"
    );
    assert!(text.contains("Affected states: unavailable."), "{text}");
    assert_eq!(
        text.matches("| stack | name | state id | file | reasons |")
            .count(),
        1
    );
    assert!(!text.contains("effect key |"), "{text}");
    assert!(!text.contains("history block 1939.1.1"), "{text}");
    assert!(summary(&run, 1).contains(&text));
    let encoded = serde_json::to_string(&run["fixture"]).unwrap();
    let decoded: StackReport = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        decoded.state_history_states,
        run["fixture"].state_history_states
    );
}
