
use super::Compared;
use crate::{document::Document, playhead_host::{MapSide, mirror::Mirrors, views::{Pace, view::{HeadFailure, HeadFailureKind}}}};
use viewer_layer_core::layers::Layer;
use viewer_layers_forces::intel_icons::{Evaluate, FailedView, Icons};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use serde_json::{Value, json};
use std::{collections::BTreeSet, io, sync::Arc};
use viewer_raids::map::intel_icons::{self, IntelIcons};

pub(super) fn make(document: &Document) -> Box<dyn Layer> {
    let layer = Icons::new(Arc::clone(&document.content) as Arc<dyn Evaluate>, Pace::Ready);
    Box::new(layer)
}

fn of(layer: &dyn Layer) -> &Icons {
    layer.any().downcast_ref::<Icons>().expect("the intel icons line's layer")
}

pub(super) fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let stats = of(layer).stats();
    vec![("evaluations", stats.evaluations.values().sum()), ("rows skipped", stats.skipped), ("discarded learning", stats.learning)]
}

fn entries(icons: &IntelIcons) -> u64 {
    (icons.states.len() + icons.states.iter().map(|state| state.entries.len()).sum::<usize>() + icons.report.reasons.len() + 1) as u64
}

fn first_difference(today: &IntelIcons, ours: &IntelIcons) -> String {
    if today.built_for != ours.built_for {
        return format!("built for: today {:?}, the layer {:?}", today.built_for, ours.built_for);
    }
    if let Some((a, b)) = today.states.iter().zip(&ours.states).find(|(a, b)| a != b) {
        return format!("state {}: today {a:?}, the layer {b:?}", a.state);
    }
    if today.states.len() != ours.states.len() {
        return format!("states: today {}, the layer {}", today.states.len(), ours.states.len());
    }
    format!("report: today {:?}, the layer {:?}", today.report, ours.report)
}

fn failure_json(failure: &HeadFailure) -> Value {
    match &failure.kind {
        HeadFailureKind::BuildError(error) => json!({
            "table": format!("{:?}", failure.table),
            "kind": "build_error",
            "reason": error,
        }),
        HeadFailureKind::Panicked(detail) => json!({
            "table": format!("{:?}", failure.table),
            "kind": "panicked",
            "reason": detail,
        }),
    }
}

fn diff_intel_icons(expected: &IntelIcons, actual: &IntelIcons) -> Vec<String> {
    let mut diffs = Vec::new();
    if expected.built_for != actual.built_for {
        diffs.push(format!(
            "built for: expected {:?}, the layer {:?}",
            expected.built_for, actual.built_for
        ));
    }

    let state_slots = expected.states.len().max(actual.states.len());
    for slot in 0..state_slots {
        match (expected.states.get(slot), actual.states.get(slot)) {
            (Some(a), Some(b)) => {
                if a.state != b.state {
                    diffs.push(format!(
                        "state[{slot}]: expected state {}, the layer has state {}",
                        a.state, b.state
                    ));
                    continue;
                }
                if a.anchor != b.anchor {
                    diffs.push(format!(
                        "state {} anchor: expected {:?}, the layer {:?}",
                        a.state, a.anchor, b.anchor
                    ));
                }
                let entry_slots = a.entries.len().max(b.entries.len());
                for idx in 0..entry_slots {
                    match (a.entries.get(idx), b.entries.get(idx)) {
                        (Some(left), Some(right)) if left != right => {
                            diffs.push(format!(
                                "state {} entry[{idx}] {:?} {:?}: expected {left:?}, the layer {right:?}",
                                a.state, left.kind, left.key
                            ));
                        }
                        (Some(left), None) => {
                            diffs.push(format!(
                                "state {} entry[{idx}] {:?} {:?}: missing in layer ({left:?})",
                                a.state, left.kind, left.key
                            ));
                        }
                        (None, Some(right)) => {
                            diffs.push(format!(
                                "state {} entry[{idx}] {:?} {:?}: extra in layer ({right:?})",
                                a.state, right.kind, right.key
                            ));
                        }
                        _ => {}
                    }
                }
            }
            (Some(a), None) => {
                if a.entries.is_empty() {
                    diffs.push(format!("state[{slot}] (state {}): missing in layer", a.state));
                } else {
                    for (idx, entry) in a.entries.iter().enumerate() {
                        diffs.push(format!(
                            "state {} entry[{idx}] {:?} {:?}: missing in layer ({entry:?})",
                            a.state, entry.kind, entry.key
                        ));
                    }
                }
            }
            (None, Some(b)) => {
                if b.entries.is_empty() {
                    diffs.push(format!("state[{slot}] (state {}): extra in layer", b.state));
                } else {
                    for (idx, entry) in b.entries.iter().enumerate() {
                        diffs.push(format!(
                            "state {} entry[{idx}] {:?} {:?}: extra in layer ({entry:?})",
                            b.state, entry.kind, entry.key
                        ));
                    }
                }
            }
            (None, None) => {}
        }
    }

    let reason_keys: BTreeSet<&str> = expected
        .report
        .reasons
        .keys()
        .chain(actual.report.reasons.keys())
        .map(String::as_str)
        .collect();
    for reason in reason_keys {
        let a_count = expected.report.reasons.get(reason).copied().unwrap_or(0);
        let b_count = actual.report.reasons.get(reason).copied().unwrap_or(0);
        if a_count != b_count {
            diffs.push(format!(
                "report reason '{reason}': expected {a_count}, the layer {b_count}"
            ));
        }
    }

    diffs
}

fn report_invariant_error(label: &str, icons: &IntelIcons) -> Option<String> {
    let state_count = icons.states.len();
    let entry_count: usize = icons.states.iter().map(|state| state.entries.len()).sum();
    if icons.report.states != state_count || icons.report.entries != entry_count {
        Some(format!(
            "{label} report counts inconsistent: report holds ({}, {}), vectors hold ({}, {})",
            icons.report.states, icons.report.entries, state_count, entry_count
        ))
    } else {
        None
    }
}

fn substitute_eligible_failures(
    document_frame: &DecodedFrame,
    eligible: &[&FailedView],
) -> DecodedFrame {
    let mut substituted = document_frame.clone();
    for failed in eligible {
        let index = failed.failure.table.index();
        substituted.tables[index] = failed.view.clone();
    }
    substituted
}

fn compare_with_substituted(
    wanted: &IntelIcons,
    icons: &IntelIcons,
    document_frame: &DecodedFrame,
    installed_failures: &[FailedView],
    declared: &[TableId],
    evaluate_reference: impl FnOnce(&DecodedFrame) -> IntelIcons,
) -> Compared {
    let mut compared = Compared {
        entries: entries(wanted),
        ..Compared::default()
    };
    let notes = json!({
        "states": wanted.states.len(),
        "entries": wanted.report.entries,
        "reasons": wanted.report.reasons.len(),
        "tables": declared.len(),
    });

    let eligible: Vec<&FailedView> = installed_failures
        .iter()
        .filter(|failed| {
            matches!(failed.failure.kind, HeadFailureKind::BuildError(_))
                && declared.contains(&failed.failure.table)
                && *document_frame.tables[failed.failure.table.index()] != *failed.view
        })
        .collect();

    let substituted_expected = if !eligible.is_empty() {
        let substituted = substitute_eligible_failures(document_frame, &eligible);
        Some(evaluate_reference(&substituted))
    } else {
        None
    };

    let expected = substituted_expected.as_ref().unwrap_or(wanted);
    let mut mismatches = diff_intel_icons(expected, icons);
    if let Some(err) = report_invariant_error("layer", icons) {
        mismatches.push(err);
    }
    if let Some(err) = report_invariant_error("expected", wanted) {
        mismatches.push(err);
    }
    if let Some(substituted) = &substituted_expected
        && let Some(err) = report_invariant_error("substituted expected", substituted)
    {
        mismatches.push(err);
    }
    let exceptions = match &substituted_expected {
        Some(expected) => diff_intel_icons(wanted, expected),
        None => Vec::new(),
    };

    compared.differing = mismatches.len() as u64;
    compared.first = mismatches.into_iter().next();
    compared.notes = if exceptions.is_empty() && installed_failures.is_empty() {
        notes
    } else {
        json!({
            "known_exceptions": exceptions,
            "refused_at_the_head": installed_failures
                .iter()
                .map(|failed| failure_json(&failed.failure))
                .collect::<Vec<_>>(),
            "detail": notes,
        })
    };
    compared
}

fn reference_icons_from_ours(
    frame: &DecodedFrame,
    ours: Result<u32, String>,
    build_icons: impl FnOnce(&DecodedFrame, u32) -> IntelIcons,
) -> IntelIcons {
    match ours {
        Ok(ours) => build_icons(frame, ours),
        Err(why) => {
            let mut icons = IntelIcons {
                built_for: None,
                ..IntelIcons::default()
            };
            icons.report.reasons.insert(
                format!(
                    "the viewing country cannot be resolved at hour {}: {why}",
                    frame.hour
                ),
                1,
            );
            icons
        }
    }
}

fn compare_today(
    layer: &Icons,
    frame: &DecodedFrame,
    document_eval: impl Fn(&DecodedFrame) -> IntelIcons,
) -> Compared {
    let Some((icons, hour, installed_failures)) = layer.icons_with_failures() else {
        return Compared {
            notes: json!({"skipped": "no icons yet: their tables are being learned or loading", "tables": layer.tables().len()}),
            ..Compared::default()
        };
    };
    if hour != frame.hour {
        return Compared {
            notes: json!({"skipped": format!("the icons are of hour {hour}")}),
            ..Compared::default()
        };
    }
    let wanted = document_eval(frame);
    compare_with_substituted(
        &wanted,
        icons,
        frame,
        installed_failures,
        layer.tables(),
        &document_eval,
    )
}

fn today_with(
    layer: &Icons,
    frame: &DecodedFrame,
    initial_ours: impl FnOnce() -> io::Result<u32>,
    resolve_ours: impl Fn(&DecodedFrame) -> io::Result<u32>,
    build_icons: impl Fn(&DecodedFrame, u32) -> IntelIcons,
) -> Compared {
    let Some((_, hour, _)) = layer.icons_with_failures() else {
        return Compared {
            notes: json!({"skipped": "no icons yet: their tables are being learned or loading", "tables": layer.tables().len()}),
            ..Compared::default()
        };
    };
    if hour != frame.hour {
        return Compared {
            notes: json!({"skipped": format!("the icons are of hour {hour}")}),
            ..Compared::default()
        };
    }
    let initial_ours = initial_ours().map_err(|error| error.to_string());
    compare_today(layer, frame, |target| {
        let resolved = if std::ptr::eq(target, frame) {
            initial_ours.clone()
        } else {
            resolve_ours(target).map_err(|error| error.to_string())
        };
        reference_icons_from_ours(target, resolved, &build_icons)
    })
}

pub(super) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = of(layer);
    let frame = document.frame()?;
    let choice = viewer_foundation::perspective::Choice::default();
    let content = &document.content;
    Ok(today_with(
        layer,
        frame,
        || document.vision(&choice).map(|vision| vision.ours()),
        |target| {
            viewer_foundation::perspective::incremental::Fold::advance(
                &mut None,
                target,
                &choice,
                viewer_foundation::visibility::FogContent::loaded(&content.counters.fog),
            )
            .map(|vision| vision.ours())
        },
        |target, ours| {
            intel_icons::icons(
                &content.icons.intel,
                &content.icons.raids,
                target,
                ours,
                &content.names.localisation,
                &content.raid_hover.recorded_language,
            )
        },
    ))
}

pub(super) fn fresh(layer: &dyn Layer, _mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = of(layer);
    let (Some((icons, hour)), Some(rows)) = (layer.icons(), layer.newest_rows()) else {
        return Ok(Compared { notes: json!({"skipped": "no icons or no rows yet"}), ..Compared::default() });
    };
    if rows.hour != hour {
        return Ok(Compared { notes: json!({"skipped": format!("the rows are at {}, the icons of {hour}", rows.hour)}), ..Compared::default() });
    }
    let again = layer.evaluate(&rows);
    let differing = u64::from(again != **icons);
    Ok(Compared { entries: entries(&again), differing, first: (differing > 0).then(|| first_difference(&again, icons)), notes: Value::Null })
}

pub(super) fn summary(side: &MapSide) -> Value {
    viewer_layers_forces::intel_icons::summary(side)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;
    use replay_frame::frame::{Metrics, SharedView, TableView};
    use viewer_foundation::tooltip_text::Tooltip;
    use viewer_raids::map::intel_icons::{Anchor, Entry, EntryKey, EntryKind, IntelReport, StateIcon, Write};

    fn entry(kind: EntryKind, key: EntryKey, text: &str) -> Entry {
        Entry {
            kind,
            key,
            writes: vec![Write::Text("rating_txt", text.to_owned())],
            tooltip: Arc::new(Tooltip::default()),
        }
    }

    fn icons_of(states: Vec<StateIcon>, reasons: &[(&str, usize)]) -> IntelIcons {
        let report = IntelReport {
            states: states.len(),
            entries: states.iter().map(|s| s.entries.len()).sum(),
            reasons: reasons.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
        };
        IntelIcons {
            states,
            report,
            built_for: Some((1, 10, 1)),
        }
    }

    fn complete_frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 10,
            tables: TableId::ALL
                .map(|table| SharedView::from(TableView::Complete(TableRows::empty(table)))),
            metrics: Metrics::default(),
        }
    }

    fn frame_with_unavailable(entries: &[(TableId, String)]) -> DecodedFrame {
        let mut frame = complete_frame();
        for (table, reason) in entries {
            frame.tables[table.index()] = SharedView::from(TableView::Unavailable {
                reason: reason.clone(),
            });
        }
        frame
    }

    fn failed_view(failure: HeadFailure) -> FailedView {
        let view = SharedView::from(TableView::Unavailable {
            reason: failure.format(),
        });
        FailedView { failure, view }
    }

    fn synthetic_eval_for_country(frame: &DecodedFrame, country: u32) -> IntelIcons {
        let shield_text = if country == 1 { "5" } else { "9" };
        let mut entries_10 = vec![entry(EntryKind::Shield, EntryKey::Shield(1), shield_text)];
        let mut reasons: Vec<(&str, usize)> = Vec::new();
        match &*frame.tables[TableId::AgencyOperationBuffers.index()] {
            TableView::Complete(_) => {
                let text = match &*frame.tables[TableId::LexicalTokens.index()] {
                    TableView::Complete(_) => "op_token",
                    _ => "op_missing_token",
                };
                entries_10.push(entry(EntryKind::Operation, EntryKey::Operation(42), text));
            }
            TableView::Unavailable { .. } => {
                reasons.push(("operation entries are withheld: AgencyOperationBuffers unavailable", 1));
            }
            _ => {}
        }
        let mut icons = icons_of(
            vec![StateIcon {
                state: 10,
                anchor: Anchor::Province(100),
                entries: entries_10,
            }],
            &reasons,
        );
        icons.built_for = Some((frame.branch, frame.hour, country));
        icons
    }

    fn synthetic_eval(frame: &DecodedFrame) -> IntelIcons {
        synthetic_eval_for_country(frame, 1)
    }

    #[test]
    fn clock_or_unrelated_build_error_does_not_excuse_unrelated_shield_mismatch() {
        let doc = complete_frame();
        let wanted = synthetic_eval(&doc);
        let actual = icons_of(
            vec![StateIcon {
                state: 10,
                anchor: Anchor::Province(100),
                entries: vec![
                    entry(EntryKind::Shield, EntryKey::Shield(1), "9"),
                    entry(EntryKind::Operation, EntryKey::Operation(42), "op_token"),
                ],
            }],
            &[],
        );
        for table in [TableId::Clock, TableId::Divisions] {
            let failure = HeadFailure {
                table,
                kind: HeadFailureKind::BuildError(format!("{table:?} is held without keys")),
            };
            let compared = compare_with_substituted(
                &wanted,
                &actual,
                &doc,
                &[failed_view(failure)],
                &[TableId::Clock, TableId::AgencyOperationBuffers, TableId::LexicalTokens],
                synthetic_eval,
            );
            assert_eq!(compared.differing, 1, "table {table:?} must not excuse shield mismatch: {compared:?}");
            assert!(compared.first.as_deref().unwrap_or_default().contains("Shield"));
        }
    }

    #[test]
    fn unrelated_build_error_does_not_switch_reference_to_layer_evaluator() {
        struct WrongCountryLayer;
        impl Evaluate for WrongCountryLayer {
            fn icons(
                &self,
                frame: &DecodedFrame,
                _choice: &viewer_foundation::perspective::Choice,
            ) -> IntelIcons {
                synthetic_eval_for_country(frame, 2)
            }
        }

        let doc = complete_frame();
        let clock_failure = HeadFailure {
            table: TableId::Clock,
            kind: HeadFailureKind::BuildError("Clock is held without keys".into()),
        };
        let head = frame_with_unavailable(&[(TableId::Clock, clock_failure.format())]);
        let mut layer = Icons::new(Arc::new(WrongCountryLayer), Pace::Step);
        let layer_icons = layer.evaluate(&head);
        layer.install_for_test(layer_icons, doc.hour, vec![failed_view(clock_failure)]);

        let compared = compare_today(&layer, &doc, synthetic_eval);
        assert!(
            compared.differing >= 1,
            "layer-only wrong-country discrepancy must not be reproduced on the reference side under an unrelated Clock build error: {compared:?}"
        );
        assert!(
            compared.first.as_deref().unwrap_or_default().contains("built for"),
            "{compared:?}"
        );
        let exceptions = compared.notes["known_exceptions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            exceptions.is_empty(),
            "unrelated Clock build error should produce zero reference exceptions: {compared:?}"
        );
    }

    #[test]
    fn lexical_tokens_build_error_excuses_only_affected_entry_via_reevaluation() {
        let doc = complete_frame();
        let wanted = synthetic_eval(&doc);
        let failure = HeadFailure {
            table: TableId::LexicalTokens,
            kind: HeadFailureKind::BuildError("LexicalTokens is held without keys".into()),
        };
        let head = frame_with_unavailable(&[(TableId::LexicalTokens, failure.format())]);
        let actual = synthetic_eval(&head);
        let compared = compare_with_substituted(
            &wanted,
            &actual,
            &doc,
            &[failed_view(failure)],
            &[TableId::AgencyOperationBuffers, TableId::LexicalTokens],
            synthetic_eval,
        );
        assert_eq!(compared.differing, 0, "exact LexicalTokens difference should be excused: {compared:?}");
        assert_eq!(compared.notes["known_exceptions"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn missing_operation_buffer_excuses_operation_while_retaining_unrelated_shield_mismatch() {
        let doc = complete_frame();
        let wanted = synthetic_eval(&doc);
        let failure = HeadFailure {
            table: TableId::AgencyOperationBuffers,
            kind: HeadFailureKind::BuildError("AgencyOperationBuffers is held without keys".into()),
        };
        let head = frame_with_unavailable(&[(TableId::AgencyOperationBuffers, failure.format())]);
        let mut actual = synthetic_eval(&head);
        actual.states[0].entries[0] = entry(EntryKind::Shield, EntryKey::Shield(1), "9");
        let compared = compare_with_substituted(
            &wanted,
            &actual,
            &doc,
            &[failed_view(failure)],
            &[TableId::AgencyOperationBuffers, TableId::LexicalTokens],
            synthetic_eval,
        );
        assert_eq!(compared.differing, 1, "unrelated shield mismatch must remain unexcused: {compared:?}");
        assert!(compared.first.as_deref().unwrap_or_default().contains("Shield"));
        assert_eq!(compared.notes["known_exceptions"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn panicked_head_failure_and_recorded_unavailability_never_excuse_mismatch() {
        let doc = complete_frame();
        let wanted = synthetic_eval(&doc);
        let panic_failure = HeadFailure {
            table: TableId::AgencyOperationBuffers,
            kind: HeadFailureKind::Panicked("index out of bounds".into()),
        };
        let head = frame_with_unavailable(&[(TableId::AgencyOperationBuffers, panic_failure.format())]);
        let actual = synthetic_eval(&head);
        let panicked_cmp = compare_with_substituted(
            &wanted,
            &actual,
            &doc,
            &[failed_view(panic_failure)],
            &[TableId::AgencyOperationBuffers],
            synthetic_eval,
        );
        assert_eq!(panicked_cmp.differing, 2, "panicked head failure must not excuse operation or reason: {panicked_cmp:?}");

        let recorded_cmp = compare_with_substituted(
            &wanted,
            &actual,
            &doc,
            &[],
            &[TableId::AgencyOperationBuffers],
            synthetic_eval,
        );
        assert_eq!(recorded_cmp.differing, 2, "recorded unavailability with no typed HeadFailure must not excuse mismatch: {recorded_cmp:?}");
    }

    #[test]
    fn reversed_state_order_and_duplicate_entry_keys_are_reported_independently() {
        let doc = complete_frame();
        let wanted = icons_of(
            vec![
                StateIcon {
                    state: 10,
                    anchor: Anchor::Province(100),
                    entries: vec![
                        entry(EntryKind::Operation, EntryKey::Offer(1, 2), "first"),
                        entry(EntryKind::Operation, EntryKey::Offer(1, 2), "second"),
                    ],
                },
                StateIcon {
                    state: 20,
                    anchor: Anchor::Province(200),
                    entries: vec![entry(EntryKind::Shield, EntryKey::Shield(1), "5")],
                },
            ],
            &[],
        );
        let reversed = icons_of(
            vec![wanted.states[1].clone(), wanted.states[0].clone()],
            &[],
        );
        let reversed_cmp = compare_with_substituted(
            &wanted,
            &reversed,
            &doc,
            &[],
            &[],
            |_| unreachable!(),
        );
        assert_eq!(reversed_cmp.differing, 2, "reversed states must not be hidden: {reversed_cmp:?}");

        let mut modified_dup = wanted.clone();
        modified_dup.states[0].entries[0] =
            entry(EntryKind::Operation, EntryKey::Offer(1, 2), "changed_first");
        let dup_cmp = compare_with_substituted(
            &wanted,
            &modified_dup,
            &doc,
            &[],
            &[],
            |_| unreachable!(),
        );
        assert_eq!(dup_cmp.differing, 1, "modified first duplicate key must not be collapsed: {dup_cmp:?}");
    }

    #[test]
    fn bogus_report_counts_are_not_masked_by_an_excused_entry() {
        let doc = complete_frame();
        let wanted = synthetic_eval(&doc);
        let failure = HeadFailure {
            table: TableId::AgencyOperationBuffers,
            kind: HeadFailureKind::BuildError("AgencyOperationBuffers is held without keys".into()),
        };
        let head = frame_with_unavailable(&[(TableId::AgencyOperationBuffers, failure.format())]);
        let mut actual = synthetic_eval(&head);
        actual.report.entries = 99;
        let compared = compare_with_substituted(
            &wanted,
            &actual,
            &doc,
            &[failed_view(failure)],
            &[TableId::AgencyOperationBuffers],
            synthetic_eval,
        );
        assert_eq!(compared.differing, 1, "bogus report count must not be suppressed by an excused entry: {compared:?}");
        assert!(compared.first.as_deref().unwrap_or_default().contains("report counts inconsistent"));
    }

    #[test]
    fn malformed_original_reference_report_is_not_masked_by_substitution() {
        let doc = complete_frame();
        let mut wanted = synthetic_eval(&doc);
        wanted.report.entries = 99;
        let failure = HeadFailure {
            table: TableId::Clock,
            kind: HeadFailureKind::BuildError("Clock is held without keys".into()),
        };
        let head = frame_with_unavailable(&[(TableId::Clock, failure.format())]);
        let actual = synthetic_eval(&head);
        let compared = compare_with_substituted(
            &wanted,
            &actual,
            &doc,
            &[failed_view(failure)],
            &[TableId::Clock, TableId::AgencyOperationBuffers],
            synthetic_eval,
        );
        assert_eq!(
            compared.differing, 1,
            "malformed original reference report must remain a hard mismatch: {compared:?}"
        );
        assert!(
            compared
                .first
                .as_deref()
                .unwrap_or_default()
                .contains("expected report counts inconsistent"),
            "{compared:?}"
        );
        let exceptions = compared.notes["known_exceptions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            exceptions
                .iter()
                .all(|item| !item.as_str().unwrap_or_default().contains("report counts inconsistent")),
            "report invariant violation must never enter known_exceptions: {compared:?}"
        );
    }

    #[test]
    fn unavailable_original_country_links_compares_refusal_instead_of_exiting_early() {
        struct RefusingLayer;
        impl Evaluate for RefusingLayer {
            fn icons(
                &self,
                frame: &DecodedFrame,
                choice: &viewer_foundation::perspective::Choice,
            ) -> IntelIcons {
                let resolved = viewer_layers_forces::intel_icons::ours(frame, choice)
                    .map_err(|error| error.to_string());
                reference_icons_from_ours(frame, resolved, synthetic_eval_for_country)
            }
        }

        struct EmittingLayer;
        impl Evaluate for EmittingLayer {
            fn icons(
                &self,
                frame: &DecodedFrame,
                _choice: &viewer_foundation::perspective::Choice,
            ) -> IntelIcons {
                synthetic_eval_for_country(frame, 1)
            }
        }

        let choice = viewer_foundation::perspective::Choice::default();
        let doc = frame_with_unavailable(&[(
            TableId::CountryLinks,
            "recorded CountryLinks unavailable".into(),
        )]);
        let resolve = |target: &DecodedFrame| {
            viewer_foundation::perspective::incremental::Fold::advance(
                &mut None,
                target,
                &choice,
                Err("unloaded"),
            )
            .map(|vision| vision.ours())
        };

        let mut refusing = Icons::new(Arc::new(RefusingLayer), Pace::Step);
        let refusing_icons = refusing.evaluate(&doc);
        refusing.install_for_test(refusing_icons, doc.hour, Vec::new());
        let matching = today_with(
            &refusing,
            &doc,
            || resolve(&doc),
            resolve,
            synthetic_eval_for_country,
        );
        assert_eq!(
            matching.differing, 0,
            "layer that emits the matching viewing-country refusal should pass: {matching:?}"
        );

        let mut emitting = Icons::new(Arc::new(EmittingLayer), Pace::Step);
        let emitting_icons = emitting.evaluate(&doc);
        emitting.install_for_test(emitting_icons, doc.hour, Vec::new());
        let mismatch = today_with(
            &emitting,
            &doc,
            || resolve(&doc),
            resolve,
            synthetic_eval_for_country,
        );
        assert!(
            mismatch.differing >= 1,
            "layer that wrongly emits icons when CountryLinks is unavailable must fail proof: {mismatch:?}"
        );
        assert!(
            mismatch.first.as_deref().unwrap_or_default().contains("built for"),
            "{mismatch:?}"
        );
    }
}
