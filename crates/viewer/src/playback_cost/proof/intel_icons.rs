
use super::Compared;
use crate::{
    document::Document,
    layers::{
        Layer,
        intel_icons::{Evaluate, Icons},
    },
    playhead_host::{
        MapSide,
        mirror::Mirrors,
        views::{
            Pace,
            view::{HeadFailure, HeadFailureKind, head_failures},
        },
    },
};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_raids::map::intel_icons::{self, Entry, EntryKey, EntryKind, IntelIcons, StateIcon};

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

fn is_core_world_table(table: TableId) -> bool {
    matches!(
        table,
        TableId::Countries
            | TableId::CountryLinks
            | TableId::Provinces
            | TableId::States
            | TableId::Clock
    )
}

fn explains_built_for(failure: &HeadFailure) -> bool {
    matches!(
        failure.table,
        TableId::Countries | TableId::CountryLinks | TableId::LocalPlayer
    )
}

fn explains_anchor(failure: &HeadFailure) -> bool {
    is_core_world_table(failure.table)
}

fn kind_rank(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Shield => 0,
        EntryKind::Operative => 1,
        EntryKind::Operation => 2,
        EntryKind::More => 3,
    }
}

fn explains_entry(kind: EntryKind, failure: &HeadFailure) -> bool {
    if is_core_world_table(failure.table) {
        return true;
    }
    match kind {
        EntryKind::Shield => matches!(
            failure.table,
            TableId::Politics
                | TableId::FactionMembers
                | TableId::Subjects
                | TableId::CountryWars
                | TableId::WarRelations
                | TableId::RelationLinks
                | TableId::Autonomy
                | TableId::StrategicOperatives
                | TableId::RaidIntelInputs
                | TableId::CountryAppearance
                | TableId::PoliticalParties
                | TableId::ReservedCountryNaming
                | TableId::ModifierDefinitions
        ),
        EntryKind::Operative => matches!(
            failure.table,
            TableId::OperativeMissions
                | TableId::StrategicOperatives
                | TableId::CharacterOperatives
                | TableId::CharacterPortraits
                | TableId::Politics
                | TableId::IntelNetworks
                | TableId::IntelNetworkAuxiliaries
                | TableId::IntelligenceAgencies
                | TableId::RaidIntelInputs
                | TableId::CountryStabilityInputs
                | TableId::PoliticalParties
                | TableId::CountryAppearance
                | TableId::Autonomy
                | TableId::ReservedCountryNaming
                | TableId::ModifierDefinitions
        ),
        EntryKind::Operation => matches!(
            failure.table,
            TableId::LocalPlayer
                | TableId::AgencyOperationBuffers
                | TableId::Operations
                | TableId::CountryAppearance
                | TableId::Politics
                | TableId::PoliticalParties
                | TableId::Autonomy
                | TableId::ReservedCountryNaming
                | TableId::ModifierDefinitions
        ),
        EntryKind::More => matches!(
            failure.table,
            TableId::LocalPlayer | TableId::AgencyOperationBuffers
        ),
    }
}

fn explains_reason(reason: &str, failure: &HeadFailure) -> bool {
    let table_name = format!("{:?}", failure.table);
    if reason.contains(&table_name) {
        return true;
    }
    if reason.starts_with("the viewing country cannot be resolved") {
        return explains_built_for(failure);
    }
    if reason.starts_with("the core tables are incomplete")
        || reason.starts_with("a state's icon has no anchor")
    {
        return explains_anchor(failure);
    }
    if reason.starts_with("the related countries' shields")
        || reason.starts_with("whether a related country is at war")
        || reason.starts_with("a shield's capital is unknown")
        || reason.starts_with("a friendly country controls no capital state")
    {
        return explains_entry(EntryKind::Shield, failure);
    }
    if reason.starts_with("operative icons are not drawn")
        || reason.starts_with("an operative's state is unknown")
    {
        return explains_entry(EntryKind::Operative, failure);
    }
    if reason.starts_with("operation entries are withheld")
        || reason.starts_with("operation and offer entries are withheld")
        || reason.starts_with("an admitted operation entry is withheld")
        || reason.starts_with("offer entries are withheld")
    {
        return explains_entry(EntryKind::Operation, failure);
    }
    if reason.starts_with("an entry's order is undecided") {
        return matches!(
            failure.table,
            TableId::Countries
                | TableId::CountryLinks
                | TableId::CountryAppearance
                | TableId::Politics
                | TableId::PoliticalParties
                | TableId::Autonomy
                | TableId::ReservedCountryNaming
        );
    }
    false
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

fn compare_with_failures(
    wanted: &IntelIcons,
    icons: &IntelIcons,
    failures: &[HeadFailure],
    table_count: usize,
) -> Compared {
    let mut compared = Compared {
        entries: entries(wanted),
        ..Compared::default()
    };
    let notes = json!({
        "states": wanted.states.len(),
        "entries": wanted.report.entries,
        "reasons": wanted.report.reasons.len(),
        "tables": table_count,
    });
    let qualifying: Vec<&HeadFailure> = failures
        .iter()
        .filter(|failure| matches!(failure.kind, HeadFailureKind::BuildError(_)))
        .collect();
    let mut exceptions: Vec<Value> = Vec::new();
    let mut record = |description: String, excused_by: Option<&HeadFailure>| {
        if let Some(failure) = excused_by {
            exceptions.push(json!({
                "difference": description,
                "table": format!("{:?}", failure.table),
                "failure": failure_json(failure),
            }));
        } else {
            compared.differing += 1;
            if compared.first.is_none() {
                compared.first = Some(description);
            }
        }
    };

    if wanted.built_for != icons.built_for {
        record(
            format!("built for: today {:?}, the layer {:?}", wanted.built_for, icons.built_for),
            qualifying.iter().copied().find(|f| explains_built_for(f)),
        );
    }

    let wanted_states: BTreeMap<u32, &StateIcon> =
        wanted.states.iter().map(|state| (state.state, state)).collect();
    let actual_states: BTreeMap<u32, &StateIcon> =
        icons.states.iter().map(|state| (state.state, state)).collect();
    let state_ids: BTreeSet<u32> = wanted_states
        .keys()
        .chain(actual_states.keys())
        .copied()
        .collect();

    for state_id in state_ids {
        match (wanted_states.get(&state_id), actual_states.get(&state_id)) {
            (Some(a), Some(b)) => {
                if a.anchor != b.anchor {
                    record(
                        format!(
                            "state {state_id} anchor: today {:?}, the layer {:?}",
                            a.anchor, b.anchor
                        ),
                        qualifying.iter().copied().find(|f| explains_anchor(f)),
                    );
                }
                let a_entries: BTreeMap<(u8, &EntryKey), &Entry> = a
                    .entries
                    .iter()
                    .map(|entry| ((kind_rank(entry.kind), &entry.key), entry))
                    .collect();
                let b_entries: BTreeMap<(u8, &EntryKey), &Entry> = b
                    .entries
                    .iter()
                    .map(|entry| ((kind_rank(entry.kind), &entry.key), entry))
                    .collect();
                let entry_keys: BTreeSet<(u8, &EntryKey)> = a_entries
                    .keys()
                    .chain(b_entries.keys())
                    .copied()
                    .collect();
                for map_key in entry_keys {
                    match (a_entries.get(&map_key), b_entries.get(&map_key)) {
                        (Some(left), Some(right)) if left != right => {
                            let kind = left.kind;
                            record(
                                format!(
                                    "state {state_id} {kind:?} {:?}: today {left:?}, the layer {right:?}",
                                    left.key
                                ),
                                qualifying.iter().copied().find(|f| explains_entry(kind, f)),
                            );
                        }
                        (Some(left), None) => {
                            let kind = left.kind;
                            record(
                                format!("state {state_id} {kind:?} {:?}: missing in layer ({left:?})", left.key),
                                qualifying.iter().copied().find(|f| explains_entry(kind, f)),
                            );
                        }
                        (None, Some(right)) => {
                            let kind = right.kind;
                            record(
                                format!("state {state_id} {kind:?} {:?}: extra in layer ({right:?})", right.key),
                                qualifying.iter().copied().find(|f| explains_entry(kind, f)),
                            );
                        }
                        _ => {}
                    }
                }
                let a_common: Vec<(EntryKind, &EntryKey)> = a
                    .entries
                    .iter()
                    .filter(|entry| b_entries.contains_key(&(kind_rank(entry.kind), &entry.key)))
                    .map(|entry| (entry.kind, &entry.key))
                    .collect();
                let b_common: Vec<(EntryKind, &EntryKey)> = b
                    .entries
                    .iter()
                    .filter(|entry| a_entries.contains_key(&(kind_rank(entry.kind), &entry.key)))
                    .map(|entry| (entry.kind, &entry.key))
                    .collect();
                if a_common != b_common {
                    let excused = a_common
                        .iter()
                        .zip(&b_common)
                        .find(|(left, right)| left != right)
                        .and_then(|((left_kind, _), (right_kind, _))| {
                            qualifying.iter().copied().find(|f| {
                                explains_entry(*left_kind, f) || explains_entry(*right_kind, f)
                            })
                        });
                    record(
                        format!(
                            "state {state_id} entry order: today {a_common:?}, the layer {b_common:?}"
                        ),
                        excused,
                    );
                }
            }
            (Some(a), None) => {
                if a.entries.is_empty() {
                    record(
                        format!("state {state_id}: missing in layer"),
                        qualifying.iter().copied().find(|f| explains_anchor(f)),
                    );
                } else {
                    for entry in &a.entries {
                        record(
                            format!(
                                "state {state_id} {:?} {:?}: missing in layer ({entry:?})",
                                entry.kind, entry.key
                            ),
                            qualifying
                                .iter()
                                .copied()
                                .find(|f| explains_entry(entry.kind, f)),
                        );
                    }
                }
            }
            (None, Some(b)) => {
                if b.entries.is_empty() {
                    record(
                        format!("state {state_id}: extra in layer"),
                        qualifying.iter().copied().find(|f| explains_anchor(f)),
                    );
                } else {
                    for entry in &b.entries {
                        record(
                            format!(
                                "state {state_id} {:?} {:?}: extra in layer ({entry:?})",
                                entry.kind, entry.key
                            ),
                            qualifying
                                .iter()
                                .copied()
                                .find(|f| explains_entry(entry.kind, f)),
                        );
                    }
                }
            }
            (None, None) => {}
        }
    }

    let reason_keys: BTreeSet<&str> = wanted
        .report
        .reasons
        .keys()
        .chain(icons.report.reasons.keys())
        .map(String::as_str)
        .collect();
    for reason in reason_keys {
        let a_count = wanted.report.reasons.get(reason).copied().unwrap_or(0);
        let b_count = icons.report.reasons.get(reason).copied().unwrap_or(0);
        if a_count != b_count {
            record(
                format!("report reason '{reason}': today {a_count}, the layer {b_count}"),
                qualifying.iter().copied().find(|f| explains_reason(reason, f)),
            );
        }
    }

    if (wanted.report.states != icons.report.states || wanted.report.entries != icons.report.entries)
        && compared.differing == 0
        && exceptions.is_empty()
    {
        compared.differing += 1;
        compared.first = Some(format!(
            "report counts: today ({}, {}), the layer ({}, {})",
            wanted.report.states, wanted.report.entries, icons.report.states, icons.report.entries
        ));
    }

    compared.notes = if exceptions.is_empty() && failures.is_empty() {
        notes
    } else {
        json!({
            "known_exceptions": exceptions,
            "refused_at_the_head": failures.iter().map(failure_json).collect::<Vec<_>>(),
            "detail": notes,
        })
    };
    compared
}

fn compare_today(
    wanted: &IntelIcons,
    icons: &IntelIcons,
    rows: Option<&DecodedFrame>,
    table_count: usize,
) -> Compared {
    let failures = rows.map(head_failures).unwrap_or_default();
    compare_with_failures(wanted, icons, &failures, table_count)
}

pub(super) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = of(layer);
    let frame = document.frame()?;
    let Some((icons, hour)) = layer.icons() else {
        return Ok(Compared { notes: json!({"skipped": "no icons yet: their tables are being learned or loading", "tables": layer.tables().len()}), ..Compared::default() });
    };
    if *hour != frame.hour {
        return Ok(Compared { notes: json!({"skipped": format!("the icons are of hour {hour}")}), ..Compared::default() });
    }
    let choice = viewer_foundation::map::perspective::Choice::default();
    let ours = document.vision(&choice)?.ours();
    let content = &document.content;
    let wanted = intel_icons::icons(&content.icons.intel, &content.icons.raids, frame, ours, &content.names.localisation, &content.raid_hover.recorded_language);
    let newest = layer.newest_rows();
    let declared = layer.tables();
    let failures: Vec<HeadFailure> = newest
        .as_deref()
        .filter(|rows| rows.hour == *hour)
        .map(head_failures)
        .unwrap_or_default()
        .into_iter()
        .filter(|failure| {
            declared.contains(&failure.table)
                && *frame.tables[failure.table.index()] != *newest.as_ref().expect("rows present").tables[failure.table.index()]
        })
        .collect();
    Ok(compare_with_failures(&wanted, icons, &failures, declared.len()))
}

pub(super) fn fresh(layer: &dyn Layer, _mirrors: &Mirrors, _upstream: &crate::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = of(layer);
    let (Some((icons, hour)), Some(rows)) = (layer.icons(), layer.newest_rows()) else {
        return Ok(Compared { notes: json!({"skipped": "no icons or no rows yet"}), ..Compared::default() });
    };
    if rows.hour != *hour {
        return Ok(Compared { notes: json!({"skipped": format!("the rows are at {}, the icons of {hour}", rows.hour)}), ..Compared::default() });
    }
    let again = layer.evaluate(&rows);
    let differing = u64::from(again != **icons);
    Ok(Compared { entries: entries(&again), differing, first: (differing > 0).then(|| first_difference(&again, icons)), notes: Value::Null })
}

pub(super) fn summary(side: &MapSide) -> Value {
    crate::layers::intel_icons::summary(side)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{TABLE_COUNT, TableRows};
    use replay_frame::frame::{Metrics, SharedView, TableView};
    use viewer_foundation::tooltip_text::Tooltip;
    use viewer_raids::map::intel_icons::{Anchor, IntelReport, Write};

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

    fn frame_with_unavailable(entries: &[(TableId, String)]) -> DecodedFrame {
        let mut tables: [SharedView; TABLE_COUNT] = TableId::ALL
            .map(|table| SharedView::from(TableView::Complete(TableRows::empty(table))));
        for (table, reason) in entries {
            tables[table.index()] = SharedView::from(TableView::Unavailable {
                reason: reason.clone(),
            });
        }
        DecodedFrame {
            branch: 1,
            hour: 10,
            tables,
            metrics: Metrics::default(),
        }
    }

    #[test]
    fn unrelated_table_head_failure_does_not_excuse_intel_icon_mismatch() {
        let wanted = icons_of(
            vec![StateIcon {
                state: 10,
                anchor: Anchor::Province(100),
                entries: vec![entry(EntryKind::Shield, EntryKey::Shield(1), "5")],
            }],
            &[],
        );
        let actual = icons_of(
            vec![StateIcon {
                state: 10,
                anchor: Anchor::Province(100),
                entries: vec![entry(EntryKind::Shield, EntryKey::Shield(1), "9")],
            }],
            &[],
        );
        let rows = frame_with_unavailable(&[(
            TableId::Divisions,
            HeadFailure {
                table: TableId::Divisions,
                kind: HeadFailureKind::BuildError("Divisions is held without keys".into()),
            }
            .format(),
        )]);
        let compared = compare_today(&wanted, &actual, Some(&rows), 10);
        assert_eq!(compared.differing, 1, "unrelated table failure must not excuse shield mismatch: {compared:?}");
        assert!(compared.first.is_some());
    }

    #[test]
    fn missing_operation_buffer_excuses_operation_mismatch_while_retaining_unrelated_shield_mismatch() {
        let wanted = icons_of(
            vec![StateIcon {
                state: 10,
                anchor: Anchor::Province(100),
                entries: vec![
                    entry(EntryKind::Shield, EntryKey::Shield(1), "5"),
                    entry(EntryKind::Operation, EntryKey::Operation(42), "op"),
                ],
            }],
            &[],
        );
        let actual = icons_of(
            vec![StateIcon {
                state: 10,
                anchor: Anchor::Province(100),
                entries: vec![entry(EntryKind::Shield, EntryKey::Shield(1), "9")],
            }],
            &[(
                "operation entries are withheld: AgencyOperationBuffers are unavailable: AgencyOperationBuffers is not complete: table AgencyOperationBuffers is unavailable: AgencyOperationBuffers's rows at the head could not be made: AgencyOperationBuffers is held without keys",
                1,
            )],
        );
        let rows = frame_with_unavailable(&[(
            TableId::AgencyOperationBuffers,
            HeadFailure {
                table: TableId::AgencyOperationBuffers,
                kind: HeadFailureKind::BuildError(
                    "AgencyOperationBuffers is held without keys".into(),
                ),
            }
            .format(),
        )]);
        let compared = compare_today(&wanted, &actual, Some(&rows), 10);
        assert_eq!(compared.differing, 1, "only the unrelated shield mismatch should remain unexcused: {compared:?}");
        assert!(compared.first.as_deref().unwrap_or_default().contains("Shield"));
        assert_eq!(compared.notes["known_exceptions"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn panicked_head_failure_does_not_excuse_mismatch() {
        let wanted = icons_of(
            vec![StateIcon {
                state: 10,
                anchor: Anchor::Province(100),
                entries: vec![entry(EntryKind::Operation, EntryKey::Operation(42), "op")],
            }],
            &[],
        );
        let actual = icons_of(Vec::new(), &[]);
        let rows = frame_with_unavailable(&[(
            TableId::AgencyOperationBuffers,
            HeadFailure {
                table: TableId::AgencyOperationBuffers,
                kind: HeadFailureKind::Panicked("index out of bounds".into()),
            }
            .format(),
        )]);
        let compared = compare_today(&wanted, &actual, Some(&rows), 10);
        assert_eq!(compared.differing, 1, "a panic while building head rows must not be excused as a missing input: {compared:?}");
    }

    #[test]
    fn exact_missing_operation_buffer_excuses_only_affected_operation_and_reason() {
        let wanted = icons_of(
            vec![StateIcon {
                state: 10,
                anchor: Anchor::Province(100),
                entries: vec![
                    entry(EntryKind::Shield, EntryKey::Shield(1), "5"),
                    entry(EntryKind::Operation, EntryKey::Operation(42), "op"),
                ],
            }],
            &[],
        );
        let actual = icons_of(
            vec![StateIcon {
                state: 10,
                anchor: Anchor::Province(100),
                entries: vec![entry(EntryKind::Shield, EntryKey::Shield(1), "5")],
            }],
            &[(
                "operation entries are withheld: AgencyOperationBuffers are unavailable: AgencyOperationBuffers's rows at the head could not be made: AgencyOperationBuffers is held without keys",
                1,
            )],
        );
        let rows = frame_with_unavailable(&[(
            TableId::AgencyOperationBuffers,
            HeadFailure {
                table: TableId::AgencyOperationBuffers,
                kind: HeadFailureKind::BuildError(
                    "AgencyOperationBuffers is held without keys".into(),
                ),
            }
            .format(),
        )]);
        let compared = compare_today(&wanted, &actual, Some(&rows), 10);
        assert_eq!(compared.differing, 0, "exact affected operation mismatch and its reason should be excused: {compared:?}");
        assert!(compared.first.is_none());
        assert_eq!(compared.notes["known_exceptions"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn multiple_unaffected_mismatches_retain_independent_counts_beside_excused_difference() {
        let wanted = icons_of(
            vec![
                StateIcon {
                    state: 10,
                    anchor: Anchor::Province(100),
                    entries: vec![
                        entry(EntryKind::Shield, EntryKey::Shield(1), "5"),
                        entry(EntryKind::Operation, EntryKey::Operation(42), "op"),
                    ],
                },
                StateIcon {
                    state: 20,
                    anchor: Anchor::Province(200),
                    entries: vec![entry(EntryKind::Operative, EntryKey::Operative(1, 7), "spy")],
                },
            ],
            &[],
        );
        let actual = icons_of(
            vec![
                StateIcon {
                    state: 10,
                    anchor: Anchor::Province(100),
                    entries: vec![entry(EntryKind::Shield, EntryKey::Shield(1), "8")],
                },
                StateIcon {
                    state: 20,
                    anchor: Anchor::Province(200),
                    entries: vec![entry(EntryKind::Operative, EntryKey::Operative(1, 7), "other")],
                },
            ],
            &[],
        );
        let rows = frame_with_unavailable(&[(
            TableId::AgencyOperationBuffers,
            HeadFailure {
                table: TableId::AgencyOperationBuffers,
                kind: HeadFailureKind::BuildError(
                    "AgencyOperationBuffers is held without keys".into(),
                ),
            }
            .format(),
        )]);
        let compared = compare_today(&wanted, &actual, Some(&rows), 10);
        assert_eq!(compared.differing, 2, "both unaffected shield and operative mismatches must be counted independently: {compared:?}");
        assert_eq!(compared.notes["known_exceptions"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn other_unavailable_reason_and_mismatched_table_prefix_do_not_excuse_mismatch() {
        let wanted = icons_of(
            vec![StateIcon {
                state: 10,
                anchor: Anchor::Province(100),
                entries: vec![entry(EntryKind::Operation, EntryKey::Operation(42), "op")],
            }],
            &[],
        );
        let actual = icons_of(Vec::new(), &[]);
        for reason in [
            "collector capture refused".to_owned(),
            "Operations's rows at the head could not be made: mismatched slot".to_owned(),
        ] {
            let rows = frame_with_unavailable(&[(TableId::AgencyOperationBuffers, reason)]);
            let compared = compare_today(&wanted, &actual, Some(&rows), 10);
            assert_eq!(compared.differing, 1, "non-matching unavailability reason must not excuse mismatch: {compared:?}");
        }
    }
}
